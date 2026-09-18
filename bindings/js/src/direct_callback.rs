// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::managed_tsfn;
use std::sync::Arc;

pub(crate) struct DirectJsCallback {
  pub(crate) env: napi::sys::napi_env,
  pub(crate) callback_ref: napi::sys::napi_ref,
  pub(crate) async_context: napi::sys::napi_async_context,
  pub(crate) lifecycle: Arc<managed_tsfn::TsfnLifecycle>,
}

unsafe impl Send for DirectJsCallback {}
unsafe impl Sync for DirectJsCallback {}

pub(crate) fn napi_status(name: &str, status: napi::sys::napi_status) -> napi::Result<()> {
  if status == napi::sys::Status::napi_ok {
    Ok(())
  } else {
    Err(napi::Error::from_reason(format!(
      "{name} failed with status {}",
      napi::Status::from(status),
    )))
  }
}

pub(crate) fn create_direct_callback_resources(
  env: napi::sys::napi_env,
  callback: napi::sys::napi_value,
  resource_name_bytes: &[u8],
) -> napi::Result<(
  napi::sys::napi_ref,
  napi::sys::napi_async_context,
  Box<managed_tsfn::TsfnFinalizer>,
)> {
  let mut callback_ref = std::ptr::null_mut();
  napi_status("napi_create_reference(callback)", unsafe {
    napi::sys::napi_create_reference(env, callback, 1, &mut callback_ref)
  })?;

  let result = (|| {
    let mut resource = std::ptr::null_mut();
    napi_status("napi_create_object(callback resource)", unsafe {
      napi::sys::napi_create_object(env, &mut resource)
    })?;

    let mut resource_ref = std::ptr::null_mut();
    napi_status("napi_create_reference(callback resource)", unsafe {
      napi::sys::napi_create_reference(env, resource, 1, &mut resource_ref)
    })?;

    let async_context = (|| {
      let mut resource_name = std::ptr::null_mut();
      napi_status("napi_create_string_utf8(callback resource)", unsafe {
        napi::sys::napi_create_string_utf8(
          env,
          resource_name_bytes.as_ptr().cast(),
          resource_name_bytes.len() as isize,
          &mut resource_name,
        )
      })?;
      let mut async_context = std::ptr::null_mut();
      napi_status("napi_async_init(callback)", unsafe {
        napi::sys::napi_async_init(env, resource, resource_name, &mut async_context)
      })?;
      Ok::<_, napi::Error>(async_context)
    })();

    let async_context = match async_context {
      Ok(async_context) => async_context,
      Err(error) => {
        unsafe {
          napi::sys::napi_delete_reference(env, resource_ref);
        }
        return Err(error);
      }
    };

    let finalizer: Box<managed_tsfn::TsfnFinalizer> = Box::new(move |env| {
      if env.is_null() {
        return;
      }
      let async_status = unsafe { napi::sys::napi_async_destroy(env, async_context) };
      if async_status != napi::sys::Status::napi_ok {
        eprintln!(
          "[dynwinrt] callback async context cleanup failed: {}",
          napi::Status::from(async_status)
        );
      }
      for reference in [callback_ref, resource_ref] {
        let status = unsafe { napi::sys::napi_delete_reference(env, reference) };
        if status != napi::sys::Status::napi_ok {
          eprintln!(
            "[dynwinrt] callback reference cleanup failed: {}",
            napi::Status::from(status)
          );
        }
      }
    });
    Ok((callback_ref, async_context, finalizer))
  })();

  if result.is_err() {
    unsafe {
      napi::sys::napi_delete_reference(env, callback_ref);
    }
  }
  result
}

pub(crate) fn invoke_direct_js_callback<R>(
  direct: &DirectJsCallback,
  build_args: impl FnOnce(napi::sys::napi_env) -> napi::Result<Vec<napi::sys::napi_value>>,
  parse_result: impl FnOnce(napi::sys::napi_env, napi::sys::napi_value) -> napi::Result<R>,
) -> napi::Result<R> {
  if dynwinrt::com::borrowed::callbacks_suppressed() {
    return Err(napi::Error::from_reason(
      "JavaScript callbacks are forbidden during a native borrowed-copy transaction",
    ));
  }
  if direct.lifecycle.is_closing() {
    return Err(napi::Error::from_reason(
      "Cannot invoke a callback while the Node environment is closing",
    ));
  }

  let env = direct.env;
  unsafe {
    let mut scope: napi::sys::napi_handle_scope = std::ptr::null_mut();
    napi_status(
      "napi_open_handle_scope(callback)",
      napi::sys::napi_open_handle_scope(env, &mut scope),
    )?;

    let result = (|| {
      let mut function = std::ptr::null_mut();
      napi_status(
        "napi_get_reference_value(callback)",
        napi::sys::napi_get_reference_value(env, direct.callback_ref, &mut function),
      )?;
      let args = build_args(env)?;
      let mut receiver = std::ptr::null_mut();
      napi_status(
        "napi_get_global(callback)",
        napi::sys::napi_get_global(env, &mut receiver),
      )?;
      let mut result = std::ptr::null_mut();
      let status = napi::sys::napi_make_callback(
        env,
        direct.async_context,
        receiver,
        function,
        args.len(),
        args.as_ptr(),
        &mut result,
      );
      if status != napi::sys::Status::napi_ok {
        let mut is_pending = false;
        napi_status(
          "napi_is_exception_pending(callback)",
          napi::sys::napi_is_exception_pending(env, &mut is_pending),
        )?;
        if is_pending {
          let mut error = std::ptr::null_mut();
          napi_status(
            "napi_get_and_clear_last_exception(callback)",
            napi::sys::napi_get_and_clear_last_exception(env, &mut error),
          )?;
          napi_status(
            "napi_fatal_exception(callback)",
            napi::sys::napi_fatal_exception(env, error),
          )?;
        }
        return Err(napi::Error::from_reason(format!(
          "napi_make_callback failed with status {status}",
        )));
      }
      parse_result(env, result)
    })();

    napi_status(
      "napi_close_handle_scope(callback)",
      napi::sys::napi_close_handle_scope(env, scope),
    )?;
    result
  }
}
