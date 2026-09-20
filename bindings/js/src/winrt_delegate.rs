// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::{direct_callback::*, managed_tsfn, DynWinRTType, DynWinRTValue, WinGUID};
use napi_derive::napi;
use std::sync::Arc;

native_class! {
  pub struct DynWinRtDelegate(pub(crate) dynwinrt::WinRTValue);
}

#[napi]
impl DynWinRtDelegate {
  /// Create a delegate COM object from a JS callback function.
  ///
  /// - `iid`: delegate interface IID
  /// - `param_types`: Invoke parameter types
  /// - `callback`: JS function called when WinRT fires the event
  #[napi(factory)]
  pub fn create(
    iid: &WinGUID,
    param_types: Vec<&DynWinRTType>,
    #[napi(ts_arg_type = "(...args: DynWinRTValue[]) => void")]
    callback: napi::bindgen_prelude::Function<'static, Vec<DynWinRTValue>, ()>,
  ) -> napi::Result<DynWinRtDelegate> {
    use napi::bindgen_prelude::ToNapiValue;
    use napi::JsValue;
    use windows::Win32::System::Threading::GetCurrentThreadId;

    // Track the thread we were registered on. WinRT delegate callbacks that
    // fire on this same thread are dispatched synchronously (see closure below),
    // which is required when the JS thread is running a WinUI/DispatcherQueue
    // message pump: in that state libuv is starved and the TSFN uv_async_send
    // path never wakes up. Any other thread falls back to the TSFN.
    //
    let register_tid = unsafe { GetCurrentThreadId() };
    let raw_env = callback.value().env;
    let raw_callback = napi::JsValue::raw(&callback);
    let (callback_ref, async_context, finalizer) =
      create_direct_callback_resources(raw_env, raw_callback, b"dynwinrt.delegate")?;
    let tsfn = managed_tsfn::ManagedTsfn::create(
      raw_env,
      raw_callback,
      1024,
      false,
      |values: Vec<DynWinRTValue>, env| {
        values
          .into_iter()
          .map(|value| unsafe { DynWinRTValue::to_napi_value(env, value) })
          .collect()
      },
      Some(finalizer),
    )?;
    let lifecycle = tsfn.lifecycle();
    let direct = Arc::new(DirectJsCallback {
      env: raw_env,
      callback_ref,
      async_context,
      lifecycle,
    });

    let type_handles: Vec<dynwinrt::TypeHandle> = param_types.iter().map(|t| t.0.clone()).collect();

    let delegate_callback: dynwinrt::delegate::DelegateCallback =
      Box::new(move |args: &[dynwinrt::WinRTValue]| {
        // Well-known HRESULTs used below to signal failure to the WinRT event
        // source (rather than silently returning S_OK, which would look like
        // the delegate ran).
        const E_FAIL: windows::core::HRESULT = windows::core::HRESULT(0x80004005u32 as i32);
        const E_UNEXPECTED: windows::core::HRESULT = windows::core::HRESULT(0x8000FFFFu32 as i32);

        let current_tid = unsafe { GetCurrentThreadId() };
        let js_args: Vec<DynWinRTValue> =
          args.iter().map(|a| DynWinRTValue::new(a.clone())).collect();

        if current_tid == register_tid {
          // Same-thread synchronous direct invocation. Bypass the TSFN because
          // libuv may be blocked (e.g. DispatcherQueue.runEventLoop), so
          // uv_async_send would queue the callback but never fire it.
          //
          // The entire body is wrapped in `catch_unwind`: this closure is
          // ultimately called by an `extern "system"` COM stub, and letting a
          // Rust panic unwind through the FFI boundary is UB. On panic we
          // convert to E_UNEXPECTED so the WinRT caller sees a clean failure.
          let unwind_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || -> windows::core::HRESULT {
              let call_result = invoke_direct_js_callback(
                &direct,
                |env| {
                  js_args
                    .into_iter()
                    .map(|value| unsafe { DynWinRTValue::to_napi_value(env, value) })
                    .collect()
                },
                |_env, _result| Ok(()),
              );
              if let Err(error) = call_result {
                eprintln!("[dynwinrt] delegate dispatch error: {error}");
                return E_FAIL;
              }
              windows::core::HRESULT(0)
            },
          ));
          return match unwind_result {
            Ok(hr) => hr,
            Err(_) => {
              eprintln!("[dynwinrt] delegate: panic caught at FFI boundary");
              E_UNEXPECTED
            }
          };
        }

        // Cross-thread fallback: schedule via the TSFN. This requires libuv to
        // be pumping on the JS thread, which is fine for classic Node.js work
        // but not for a JS thread stuck inside a foreign message pump.
        let status = tsfn.call(js_args);
        if status == napi::Status::Ok {
          windows::core::HRESULT(0)
        } else {
          if status != napi::Status::QueueFull {
            eprintln!("[dynwinrt] delegate callback queue failed: {status}");
          }
          E_FAIL
        }
      });

    let value =
      dynwinrt::delegate::try_create_delegate_value(iid.0, type_handles, delegate_callback)
        .map_err(|error| {
          napi::Error::from_reason(format!("DynWinRtDelegate.create: {}", error.message()))
        })?;
    Ok(DynWinRtDelegate(value))
  }

  /// Drop this owner's native reference without disconnecting native holders.
  /// Event registration code should release both this owner and its temporary
  /// toValue() result after the native event source has retained the delegate.
  #[napi]
  pub fn release(&mut self) {
    self.0 = dynwinrt::WinRTValue::Null;
  }

  /// Get an independently owned value for passing the delegate to WinRT.
  #[napi]
  pub fn to_value(&self) -> napi::Result<DynWinRTValue> {
    if matches!(self.0, dynwinrt::WinRTValue::Null) {
      return Err(napi::Error::from_reason(
        "WinRT delegate owner has been released",
      ));
    }
    Ok(DynWinRTValue::new(self.0.clone()))
  }
}
