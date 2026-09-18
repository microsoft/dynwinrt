// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::{DynWinRTValue, WinGUID};
use napi_derive::napi;
use std::sync::{Arc, Mutex};

type ElementFactoryGetFunction =
  napi::bindgen_prelude::Function<'static, DynWinRTValue, DynWinRTValue>;
type ElementFactoryRecycleFunction = napi::bindgen_prelude::Function<'static, DynWinRTValue, ()>;

struct ElementFactoryCallbackRefs {
  get_element: Option<Arc<napi::bindgen_prelude::FunctionRef<DynWinRTValue, DynWinRTValue>>>,
  recycle_element: Option<Arc<napi::bindgen_prelude::FunctionRef<DynWinRTValue, ()>>>,
}

#[napi]
pub struct DynWinRtElementFactory {
  value: dynwinrt::WinRTValue,
  callbacks: Arc<Mutex<ElementFactoryCallbackRefs>>,
}

unsafe fn take_pending_exception_message(env: napi::sys::napi_env) -> Option<String> {
  let mut pending = false;
  if napi::sys::napi_is_exception_pending(env, &mut pending) != napi::sys::Status::napi_ok
    || !pending
  {
    return None;
  }

  let mut exception = std::ptr::null_mut();
  if napi::sys::napi_get_and_clear_last_exception(env, &mut exception) != napi::sys::Status::napi_ok
  {
    return None;
  }
  let mut text = std::ptr::null_mut();
  if napi::sys::napi_coerce_to_string(env, exception, &mut text) != napi::sys::Status::napi_ok {
    return None;
  }

  let mut length = 0usize;
  if napi::sys::napi_get_value_string_utf8(env, text, std::ptr::null_mut(), 0, &mut length)
    != napi::sys::Status::napi_ok
  {
    return None;
  }
  let mut buffer = vec![0u8; length + 1];
  let mut written = 0usize;
  if napi::sys::napi_get_value_string_utf8(
    env,
    text,
    buffer.as_mut_ptr().cast(),
    buffer.len(),
    &mut written,
  ) != napi::sys::Status::napi_ok
  {
    return None;
  }
  Some(String::from_utf8_lossy(&buffer[..written]).into_owned())
}

#[napi]
impl DynWinRtElementFactory {
  #[napi(factory)]
  pub fn create(
    element_iid: &WinGUID,
    #[napi(ts_arg_type = "(args: DynWinRtValue) => DynWinRtValue")]
    get_element: ElementFactoryGetFunction,
    #[napi(ts_arg_type = "(args: DynWinRtValue) => void")]
    recycle_element: ElementFactoryRecycleFunction,
  ) -> napi::Result<DynWinRtElementFactory> {
    use napi::bindgen_prelude::{FromNapiValue, ToNapiValue};
    use napi::JsValue;
    use windows::Win32::System::Threading::GetCurrentThreadId;

    const E_FAIL: windows::core::HRESULT = windows::core::HRESULT(0x80004005u32 as i32);
    const E_UNEXPECTED: windows::core::HRESULT = windows::core::HRESULT(0x8000FFFFu32 as i32);
    const RPC_E_WRONG_THREAD: windows::core::HRESULT = windows::core::HRESULT(0x8001010Eu32 as i32);
    const RO_E_CLOSED: windows::core::HRESULT = windows::core::HRESULT(0x80000013u32 as i32);

    struct SendableEnv(napi::sys::napi_env);
    unsafe impl Send for SendableEnv {}
    unsafe impl Sync for SendableEnv {}

    let register_tid = unsafe { GetCurrentThreadId() };
    let element_iid = element_iid.0;
    let raw_env = Arc::new(SendableEnv(get_element.value().env));
    let callbacks = Arc::new(Mutex::new(ElementFactoryCallbackRefs {
      get_element: Some(Arc::new(get_element.create_ref()?)),
      recycle_element: Some(Arc::new(recycle_element.create_ref()?)),
    }));

    let get_env = raw_env.clone();
    let get_callbacks = callbacks.clone();
    let get_callback: dynwinrt::ElementFactoryGetCallback = Box::new(move |args| {
      if unsafe { GetCurrentThreadId() } != register_tid {
        return Err(RPC_E_WRONG_THREAD);
      }

      let get_ref = match get_callbacks.lock() {
        Ok(callbacks) => {
          let Some(callback) = callbacks.get_element.as_ref() else {
            return Err(RO_E_CLOSED);
          };
          callback.clone()
        }
        Err(_) => return Err(E_FAIL),
      };
      let raw_env = get_env.0;
      let js_arg = DynWinRTValue::new(args.clone());
      let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> napi::Result<dynwinrt::WinRTValue> {
          unsafe {
            let mut scope = std::ptr::null_mut();
            if napi::sys::napi_open_handle_scope(raw_env, &mut scope) != napi::sys::Status::napi_ok
            {
              return Err(napi::Error::from_reason("napi_open_handle_scope failed"));
            }

            let call_result = (|| -> napi::Result<dynwinrt::WinRTValue> {
              let env = napi::Env::from_raw(raw_env);
              let function = get_ref.borrow_back(&env)?;
              let function_value = napi::JsValue::raw(&function);
              let argument = DynWinRTValue::to_napi_value(raw_env, js_arg)?;
              let mut receiver = std::ptr::null_mut();
              napi::sys::napi_get_global(raw_env, &mut receiver);
              let mut raw_result = std::ptr::null_mut();
              let status = napi::sys::napi_make_callback(
                raw_env,
                std::ptr::null_mut(),
                receiver,
                function_value,
                1,
                &argument,
                &mut raw_result,
              );
              if status != napi::sys::Status::napi_ok {
                let detail = take_pending_exception_message(raw_env)
                  .unwrap_or_else(|| "unknown JavaScript exception".into());
                return Err(napi::Error::from_reason(format!(
                  "IElementFactory getElement callback failed: {detail}"
                )));
              }
              <&DynWinRTValue>::from_napi_value(raw_env, raw_result)?
                .winrt()
                .cast(&element_iid)
                .map_err(|error| napi::Error::from_reason(error.message()))
            })();
            let call_result =
              call_result.map_err(|error| match take_pending_exception_message(raw_env) {
                Some(detail) => napi::Error::from_reason(format!("{error}: {detail}")),
                None => error,
              });
            napi::sys::napi_close_handle_scope(raw_env, scope);
            call_result
          }
        },
      ));

      match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => {
          eprintln!("[dynwinrt] IElementFactory getElement dispatch error: {error}");
          Err(E_FAIL)
        }
        Err(_) => Err(E_UNEXPECTED),
      }
    });

    let recycle_env = raw_env.clone();
    let recycle_callbacks = callbacks.clone();
    let recycle_callback: dynwinrt::ElementFactoryRecycleCallback = Box::new(move |args| {
      if unsafe { GetCurrentThreadId() } != register_tid {
        return RPC_E_WRONG_THREAD;
      }

      let recycle_ref = match recycle_callbacks.lock() {
        Ok(callbacks) => {
          let Some(callback) = callbacks.recycle_element.as_ref() else {
            return RO_E_CLOSED;
          };
          callback.clone()
        }
        Err(_) => return E_FAIL,
      };
      let raw_env = recycle_env.0;
      let js_arg = DynWinRTValue::new(args.clone());
      let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> napi::Result<()> {
          unsafe {
            let mut scope = std::ptr::null_mut();
            if napi::sys::napi_open_handle_scope(raw_env, &mut scope) != napi::sys::Status::napi_ok
            {
              return Err(napi::Error::from_reason("napi_open_handle_scope failed"));
            }

            let call_result = (|| -> napi::Result<()> {
              let env = napi::Env::from_raw(raw_env);
              let function = recycle_ref.borrow_back(&env)?;
              let function_value = napi::JsValue::raw(&function);
              let argument = DynWinRTValue::to_napi_value(raw_env, js_arg)?;
              let mut receiver = std::ptr::null_mut();
              napi::sys::napi_get_global(raw_env, &mut receiver);
              let mut raw_result = std::ptr::null_mut();
              let status = napi::sys::napi_make_callback(
                raw_env,
                std::ptr::null_mut(),
                receiver,
                function_value,
                1,
                &argument,
                &mut raw_result,
              );
              if status != napi::sys::Status::napi_ok {
                let detail = take_pending_exception_message(raw_env)
                  .unwrap_or_else(|| "unknown JavaScript exception".into());
                return Err(napi::Error::from_reason(format!(
                  "IElementFactory recycleElement callback failed: {detail}"
                )));
              }
              Ok(())
            })();
            let call_result =
              call_result.map_err(|error| match take_pending_exception_message(raw_env) {
                Some(detail) => napi::Error::from_reason(format!("{error}: {detail}")),
                None => error,
              });
            napi::sys::napi_close_handle_scope(raw_env, scope);
            call_result
          }
        }));

      match result {
        Ok(Ok(())) => windows::core::HRESULT(0),
        Ok(Err(error)) => {
          eprintln!("[dynwinrt] IElementFactory recycleElement dispatch error: {error}");
          E_FAIL
        }
        Err(_) => E_UNEXPECTED,
      }
    });

    Ok(DynWinRtElementFactory {
      value: dynwinrt::create_element_factory_value(get_callback, recycle_callback),
      callbacks,
    })
  }

  #[napi]
  pub fn to_value(&self) -> DynWinRTValue {
    DynWinRTValue::new(self.value.clone())
  }

  #[napi]
  pub fn release_callbacks(&self) -> napi::Result<()> {
    let mut callbacks = self
      .callbacks
      .lock()
      .map_err(|_| napi::Error::from_reason("IElementFactory callback state is poisoned"))?;
    callbacks.get_element = None;
    callbacks.recycle_element = None;
    Ok(())
  }
}
