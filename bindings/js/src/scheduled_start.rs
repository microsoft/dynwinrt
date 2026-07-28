// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
  alloc::{alloc_zeroed, dealloc, Layout},
  cell::RefCell,
  collections::HashMap,
  sync::OnceLock,
};

use napi::{bindgen_prelude::PromiseRaw, Env};
use windows::{
  core::{Interface, PCSTR},
  Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress},
};

use super::{async_promise, set_winui_dispatcher_loop_active};

#[repr(C)]
struct UvAsyncHandle {
  _private: [u8; 0],
}

#[repr(C)]
struct UvHandle {
  _private: [u8; 0],
}

type UvAsyncCallback = unsafe extern "C" fn(*mut UvAsyncHandle);
type UvCloseCallback = unsafe extern "C" fn(*mut UvHandle);
type UvHandleSize = unsafe extern "C" fn(i32) -> usize;
type UvAsyncInit = unsafe extern "C" fn(
  *mut napi::sys::uv_loop_s,
  *mut UvAsyncHandle,
  Option<UvAsyncCallback>,
) -> i32;
type UvAsyncSend = unsafe extern "C" fn(*mut UvAsyncHandle) -> i32;
type UvClose = unsafe extern "C" fn(*mut UvHandle, Option<UvCloseCallback>);

const UV_ASYNC_HANDLE: i32 = 1;

struct UvFunctions {
  handle_size: UvHandleSize,
  async_init: UvAsyncInit,
  async_send: UvAsyncSend,
  close: UvClose,
}

static UV_FUNCTIONS: OnceLock<UvFunctions> = OnceLock::new();

fn uv_functions() -> napi::Result<&'static UvFunctions> {
  if let Some(functions) = UV_FUNCTIONS.get() {
    return Ok(functions);
  }

  let module = unsafe { GetModuleHandleW(None) }
    .map_err(|error| napi::Error::from_reason(format!("Failed to access node.exe: {error}")))?;
  // napi-rs links only Node-API symbols on Windows. Resolve libuv dynamically
  // and allocate handles through uv_handle_size so Node/Electron owns the ABI.
  let symbol = |name: &'static [u8]| {
    unsafe { GetProcAddress(module, PCSTR(name.as_ptr())) }.ok_or_else(|| {
      napi::Error::from_reason(format!(
        "node.exe does not export {}",
        String::from_utf8_lossy(&name[..name.len() - 1])
      ))
    })
  };
  let functions = UvFunctions {
    handle_size: unsafe { std::mem::transmute(symbol(b"uv_handle_size\0")?) },
    async_init: unsafe { std::mem::transmute(symbol(b"uv_async_init\0")?) },
    async_send: unsafe { std::mem::transmute(symbol(b"uv_async_send\0")?) },
    close: unsafe { std::mem::transmute(symbol(b"uv_close\0")?) },
  };
  let _ = UV_FUNCTIONS.set(functions);
  Ok(UV_FUNCTIONS.get().expect("libuv functions initialized"))
}

struct StartPromise {
  deferred: napi::sys::napi_deferred,
  async_context: napi::sys::napi_async_context,
  resource_ref: napi::sys::napi_ref,
}

struct ScheduledWinuiStart {
  env: napi::sys::napi_env,
  method: dynwinrt::MethodHandle,
  object: dynwinrt::WinRTValue,
  args: Vec<dynwinrt::WinRTValue>,
  promise: StartPromise,
}

struct ScheduledWinuiStartHandle {
  layout: Layout,
  invocation: Option<ScheduledWinuiStart>,
}

thread_local! {
  static SCHEDULED_STARTS: RefCell<HashMap<usize, ScheduledWinuiStartHandle>> =
    RefCell::new(HashMap::new());
}

fn create_start_promise(
  env: napi::sys::napi_env,
) -> napi::Result<(napi::sys::napi_value, StartPromise)> {
  let mut deferred = std::ptr::null_mut();
  let mut promise = std::ptr::null_mut();
  let promise_status = unsafe { napi::sys::napi_create_promise(env, &mut deferred, &mut promise) };
  if promise_status != napi::sys::Status::napi_ok {
    return Err(napi::Error::from_reason(format!(
      "Failed to create scheduled WinUI Start Promise: {promise_status:?}"
    )));
  }

  let mut resource = std::ptr::null_mut();
  let resource_status = unsafe { napi::sys::napi_create_object(env, &mut resource) };
  if resource_status != napi::sys::Status::napi_ok {
    return Err(napi::Error::from_reason(format!(
      "Failed to create scheduled WinUI Start resource: {resource_status:?}"
    )));
  }
  let mut resource_ref = std::ptr::null_mut();
  let reference_status =
    unsafe { napi::sys::napi_create_reference(env, resource, 1, &mut resource_ref) };
  if reference_status != napi::sys::Status::napi_ok {
    return Err(napi::Error::from_reason(format!(
      "Failed to retain scheduled WinUI Start resource: {reference_status:?}"
    )));
  }

  let name = b"dynwinrt.winuiApplicationStart";
  let mut resource_name = std::ptr::null_mut();
  let name_status = unsafe {
    napi::sys::napi_create_string_utf8(
      env,
      name.as_ptr().cast(),
      name.len() as isize,
      &mut resource_name,
    )
  };
  if name_status != napi::sys::Status::napi_ok {
    unsafe {
      napi::sys::napi_delete_reference(env, resource_ref);
    }
    return Err(napi::Error::from_reason(format!(
      "Failed to name scheduled WinUI Start resource: {name_status:?}"
    )));
  }

  let mut async_context = std::ptr::null_mut();
  let async_status =
    unsafe { napi::sys::napi_async_init(env, resource, resource_name, &mut async_context) };
  if async_status != napi::sys::Status::napi_ok {
    unsafe {
      napi::sys::napi_delete_reference(env, resource_ref);
    }
    return Err(napi::Error::from_reason(format!(
      "Failed to initialize scheduled WinUI Start context: {async_status:?}"
    )));
  }

  Ok((
    promise,
    StartPromise {
      deferred,
      async_context,
      resource_ref,
    },
  ))
}

fn cleanup_start_promise(env: napi::sys::napi_env, promise: &StartPromise) {
  unsafe {
    let _ = napi::sys::napi_async_destroy(env, promise.async_context);
    let _ = napi::sys::napi_delete_reference(env, promise.resource_ref);
  }
}

fn settle_start_promise(
  env: napi::sys::napi_env,
  promise: StartPromise,
  result: Result<(), String>,
) {
  let mut handle_scope = std::ptr::null_mut();
  let handle_status = unsafe { napi::sys::napi_open_handle_scope(env, &mut handle_scope) };
  if handle_status != napi::sys::Status::napi_ok {
    eprintln!("[dynwinrt] scheduled WinUI Start handle scope failed: {handle_status:?}");
    cleanup_start_promise(env, &promise);
    return;
  }

  let mut resource = std::ptr::null_mut();
  let resource_status =
    unsafe { napi::sys::napi_get_reference_value(env, promise.resource_ref, &mut resource) };
  let mut callback_scope = std::ptr::null_mut();
  let scope_status = if resource_status == napi::sys::Status::napi_ok {
    unsafe {
      napi::sys::napi_open_callback_scope(env, resource, promise.async_context, &mut callback_scope)
    }
  } else {
    resource_status
  };

  let settlement = if scope_status == napi::sys::Status::napi_ok {
    result
  } else {
    Err(format!(
      "Failed to open scheduled WinUI Start callback scope: {scope_status:?}"
    ))
  };
  match settlement {
    Ok(()) => {
      let mut undefined = std::ptr::null_mut();
      let value_status = unsafe { napi::sys::napi_get_undefined(env, &mut undefined) };
      let settle_status = if value_status == napi::sys::Status::napi_ok {
        unsafe { napi::sys::napi_resolve_deferred(env, promise.deferred, undefined) }
      } else {
        value_status
      };
      if settle_status != napi::sys::Status::napi_ok {
        eprintln!("[dynwinrt] scheduled WinUI Start resolution failed: {settle_status:?}");
      }
    }
    Err(reason) => {
      let mut message = std::ptr::null_mut();
      let mut error = std::ptr::null_mut();
      let message_status = unsafe {
        napi::sys::napi_create_string_utf8(
          env,
          reason.as_ptr().cast(),
          reason.len() as isize,
          &mut message,
        )
      };
      let error_status = if message_status == napi::sys::Status::napi_ok {
        unsafe { napi::sys::napi_create_error(env, std::ptr::null_mut(), message, &mut error) }
      } else {
        message_status
      };
      let settle_status = if error_status == napi::sys::Status::napi_ok {
        unsafe { napi::sys::napi_reject_deferred(env, promise.deferred, error) }
      } else {
        error_status
      };
      if settle_status != napi::sys::Status::napi_ok {
        eprintln!("[dynwinrt] scheduled WinUI Start rejection failed: {settle_status:?}");
      }
    }
  }
  if scope_status == napi::sys::Status::napi_ok {
    let close_status = unsafe { napi::sys::napi_close_callback_scope(env, callback_scope) };
    if close_status != napi::sys::Status::napi_ok {
      eprintln!("[dynwinrt] scheduled WinUI Start callback scope close failed: {close_status:?}");
    }
  }

  cleanup_start_promise(env, &promise);
  let close_status = unsafe { napi::sys::napi_close_handle_scope(env, handle_scope) };
  if close_status != napi::sys::Status::napi_ok {
    eprintln!("[dynwinrt] scheduled WinUI Start handle scope close failed: {close_status:?}");
  }
}

unsafe extern "C" fn run_scheduled_start(handle: *mut UvAsyncHandle) {
  let Some(functions) = UV_FUNCTIONS.get() else {
    eprintln!("[dynwinrt] scheduled WinUI Start lost its libuv function table");
    return;
  };
  unsafe { (functions.close)(handle.cast(), Some(close_scheduled_start)) };
  let invocation = SCHEDULED_STARTS.with(|scheduled| {
    scheduled
      .borrow_mut()
      .get_mut(&(handle as usize))
      .and_then(|state| state.invocation.take())
  });
  let Some(invocation) = invocation else {
    return;
  };

  set_winui_dispatcher_loop_active(true);
  let mut result = match &invocation.object {
    dynwinrt::WinRTValue::Object(object) => invocation
      .method
      .invoke(object.as_raw(), &invocation.args)
      .map(|_| ())
      .map_err(|error| error.message()),
    _ => Err("Scheduled WinUI Start requires an Object target".to_string()),
  };
  set_winui_dispatcher_loop_active(false);

  let env = Env::from_raw(invocation.env);
  if let Err(error) = async_promise::unregister_winui_dispatcher_queue(env) {
    if result.is_ok() {
      result = Err(format!(
        "Scheduled WinUI Start dispatcher cleanup failed: {error}"
      ));
    }
  }
  settle_start_promise(invocation.env, invocation.promise, result);
}

unsafe extern "C" fn close_scheduled_start(handle: *mut UvHandle) {
  let state = SCHEDULED_STARTS.with(|scheduled| scheduled.borrow_mut().remove(&(handle as usize)));
  if let Some(state) = state {
    unsafe { dealloc(handle.cast(), state.layout) };
  }
}

fn schedule_on_node_loop(env: Env, invocation: ScheduledWinuiStart) -> napi::Result<()> {
  let mut invocation = Some(invocation);
  let result = (|| {
    let event_loop = env.get_uv_event_loop()?;
    if event_loop.is_null() {
      return Err(napi::Error::from_reason(
        "Node returned a null libuv event loop",
      ));
    }

    let functions = uv_functions()?;
    let size = unsafe { (functions.handle_size)(UV_ASYNC_HANDLE) };
    let layout = Layout::from_size_align(size, std::mem::align_of::<u128>())
      .map_err(|error| napi::Error::from_reason(format!("Invalid libuv handle layout: {error}")))?;
    let handle = unsafe { alloc_zeroed(layout) }.cast::<UvAsyncHandle>();
    if handle.is_null() {
      return Err(napi::Error::from_reason(
        "Failed to allocate the scheduled WinUI Start handle",
      ));
    }

    let init_status =
      unsafe { (functions.async_init)(event_loop, handle, Some(run_scheduled_start)) };
    if init_status != 0 {
      unsafe { dealloc(handle.cast(), layout) };
      return Err(napi::Error::from_reason(format!(
        "uv_async_init failed while scheduling WinUI Start: {init_status}"
      )));
    }

    SCHEDULED_STARTS.with(|scheduled| {
      scheduled.borrow_mut().insert(
        handle as usize,
        ScheduledWinuiStartHandle {
          layout,
          invocation: invocation.take(),
        },
      );
    });
    let send_status = unsafe { (functions.async_send)(handle) };
    if send_status != 0 {
      let queued_invocation = SCHEDULED_STARTS.with(|scheduled| {
        scheduled
          .borrow_mut()
          .get_mut(&(handle as usize))
          .and_then(|state| state.invocation.take())
      });
      unsafe { (functions.close)(handle.cast(), Some(close_scheduled_start)) };
      if let Some(queued_invocation) = queued_invocation {
        cleanup_start_promise(queued_invocation.env, &queued_invocation.promise);
      }
      return Err(napi::Error::from_reason(format!(
        "uv_async_send failed while scheduling WinUI Start: {send_status}"
      )));
    }
    Ok(())
  })();

  if result.is_err() {
    if let Some(invocation) = invocation {
      cleanup_start_promise(invocation.env, &invocation.promise);
    }
  }
  result
}

pub fn schedule<'env>(
  env: Env,
  method: dynwinrt::MethodHandle,
  object: dynwinrt::WinRTValue,
  args: Vec<dynwinrt::WinRTValue>,
) -> napi::Result<PromiseRaw<'env, ()>> {
  async_promise::register_winui_dispatcher_queue(env)?;
  let raw_env = env.raw();
  let (promise, promise_state) = match create_start_promise(raw_env) {
    Ok(value) => value,
    Err(error) => {
      let _ = async_promise::unregister_winui_dispatcher_queue(env);
      return Err(error);
    }
  };
  let invocation = ScheduledWinuiStart {
    env: raw_env,
    method,
    object,
    args,
    promise: promise_state,
  };
  if let Err(error) = schedule_on_node_loop(env, invocation) {
    let _ = async_promise::unregister_winui_dispatcher_queue(env);
    return Err(error);
  }
  Ok(PromiseRaw::new(raw_env, promise))
}
