// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
  collections::HashMap,
  ffi::c_void,
  ptr,
  sync::{
    atomic::{AtomicPtr, AtomicU64, Ordering},
    Arc, LazyLock, Mutex, RwLock, Weak,
  },
};

use napi::{sys, Env, JsError, Status};

static NEXT_TSFN_ID: AtomicU64 = AtomicU64::new(1);
static ENV_LIFECYCLES: LazyLock<Mutex<HashMap<usize, Weak<TsfnLifecycle>>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));
#[cfg(feature = "test-hooks")]
static TEST_PAUSE_CALL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "test-hooks")]
static TEST_CALL_PAUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "test-hooks")]
static TEST_CLEANUP_WAITING: std::sync::atomic::AtomicBool =
  std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "test-hooks")]
static TEST_CLEANUP_ACQUIRED: std::sync::atomic::AtomicBool =
  std::sync::atomic::AtomicBool::new(false);

struct RegisteredHandle {
  id: u64,
  native: Arc<NativeTsfn>,
}

pub(crate) struct TsfnLifecycle {
  open: RwLock<bool>,
  handles: Mutex<Vec<RegisteredHandle>>,
}

impl TsfnLifecycle {
  fn get_or_create(env: Env) -> napi::Result<Arc<Self>> {
    let env_key = env.raw() as usize;
    let mut lifecycles = ENV_LIFECYCLES
      .lock()
      .map_err(|_| napi::Error::from_reason("TSFN environment registry is poisoned"))?;
    if let Some(lifecycle) = lifecycles.get(&env_key).and_then(Weak::upgrade) {
      return Ok(lifecycle);
    }

    let lifecycle = Arc::new(Self {
      open: RwLock::new(true),
      handles: Mutex::new(Vec::new()),
    });
    let cleanup_lifecycle = lifecycle.clone();
    let _hook =
      env.add_env_cleanup_hook((env_key, cleanup_lifecycle), |(env_key, lifecycle)| {
        lifecycle.close_and_abort();
        ENV_LIFECYCLES
          .lock()
          .unwrap_or_else(|error| error.into_inner())
          .remove(&env_key);
      })?;
    lifecycles.insert(env_key, Arc::downgrade(&lifecycle));
    Ok(lifecycle)
  }

  pub(crate) fn is_closing(&self) -> bool {
    !*self.open.read().unwrap_or_else(|error| error.into_inner())
  }

  fn with_open<R>(&self, closed: R, callback: impl FnOnce() -> R) -> R {
    let open = self.open.read().unwrap_or_else(|error| error.into_inner());
    if *open {
      callback()
    } else {
      closed
    }
  }

  fn register(&self, id: u64, native: Arc<NativeTsfn>) {
    let mut handles = self
      .handles
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    handles.push(RegisteredHandle { id, native });
  }

  fn unregister(&self, id: u64) -> Option<Arc<NativeTsfn>> {
    let mut handles = self
      .handles
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    handles
      .iter()
      .position(|handle| handle.id == id)
      .map(|index| handles.swap_remove(index).native)
  }

  fn close_and_abort(&self) {
    #[cfg(feature = "test-hooks")]
    TEST_CLEANUP_WAITING.store(true, Ordering::SeqCst);
    let mut open = self.open.write().unwrap_or_else(|error| error.into_inner());
    #[cfg(feature = "test-hooks")]
    TEST_CLEANUP_ACQUIRED.store(true, Ordering::SeqCst);
    if !*open {
      return;
    }
    *open = false;
    drop(open);

    let handles = self
      .handles
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .drain(..)
      .map(|handle| handle.native)
      .collect::<Vec<_>>();
    for native in handles {
      native.release(sys::ThreadsafeFunctionReleaseMode::abort);
    }
  }
}

type Mapper<T> = dyn Fn(T, sys::napi_env) -> napi::Result<Vec<sys::napi_value>> + Send + Sync;
pub(crate) type TsfnFinalizer = dyn FnOnce(sys::napi_env);

struct TsfnContext<T> {
  mapper: Box<Mapper<T>>,
  finalizer: Option<Box<TsfnFinalizer>>,
  fallback_env: sys::napi_env,
}

impl<T> TsfnContext<T> {
  fn finalize(&mut self, env: sys::napi_env) {
    if let Some(finalizer) = self.finalizer.take() {
      finalizer(env);
    }
  }
}

impl<T> Drop for TsfnContext<T> {
  fn drop(&mut self) {
    if let Some(finalizer) = self.finalizer.take() {
      finalizer(self.fallback_env);
    }
  }
}

struct NativeTsfn {
  raw: AtomicPtr<sys::napi_threadsafe_function__>,
}

unsafe impl Send for NativeTsfn {}
unsafe impl Sync for NativeTsfn {}

impl NativeTsfn {
  fn release(&self, mode: sys::napi_threadsafe_function_release_mode) {
    let raw = self.raw.swap(ptr::null_mut(), Ordering::AcqRel);
    if raw.is_null() {
      return;
    }
    let status = unsafe { sys::napi_release_threadsafe_function(raw, mode) };
    if status != sys::Status::napi_ok && status != sys::Status::napi_closing {
      eprintln!(
        "[dynwinrt] managed TSFN release failed: {}",
        Status::from(status)
      );
    }
  }
}

struct TsfnHandle {
  id: u64,
  native: Arc<NativeTsfn>,
  lifecycle: Arc<TsfnLifecycle>,
}

unsafe impl Send for TsfnHandle {}
unsafe impl Sync for TsfnHandle {}

impl Drop for TsfnHandle {
  fn drop(&mut self) {
    self.lifecycle.with_open((), || {
      if let Some(native) = self.lifecycle.unregister(self.id) {
        debug_assert!(Arc::ptr_eq(&native, &self.native));
        native.release(sys::ThreadsafeFunctionReleaseMode::release);
      }
    });
  }
}

pub(crate) struct ManagedTsfn<T: Send + 'static> {
  handle: Arc<TsfnHandle>,
  _payload: std::marker::PhantomData<fn(T)>,
}

unsafe impl<T: Send + 'static> Send for ManagedTsfn<T> {}
unsafe impl<T: Send + 'static> Sync for ManagedTsfn<T> {}

impl<T: Send + 'static> Clone for ManagedTsfn<T> {
  fn clone(&self) -> Self {
    Self {
      handle: self.handle.clone(),
      _payload: std::marker::PhantomData,
    }
  }
}

impl<T: Send + 'static> ManagedTsfn<T> {
  pub(crate) fn create(
    env: sys::napi_env,
    callback: sys::napi_value,
    max_queue_size: usize,
    weak: bool,
    mapper: impl Fn(T, sys::napi_env) -> napi::Result<Vec<sys::napi_value>> + Send + Sync + 'static,
    finalizer: Option<Box<TsfnFinalizer>>,
  ) -> napi::Result<Self> {
    let lifecycle = TsfnLifecycle::get_or_create(Env::from_raw(env))?;
    let open = lifecycle
      .open
      .read()
      .unwrap_or_else(|error| error.into_inner());
    if !*open {
      return Err(napi::Error::from_reason(
        "Cannot create a TSFN while the Node environment is closing",
      ));
    }

    let context = Box::new(TsfnContext {
      mapper: Box::new(mapper),
      finalizer,
      fallback_env: env,
    });
    let context_ptr = Box::into_raw(context);
    let native = Arc::new(NativeTsfn {
      raw: AtomicPtr::new(ptr::null_mut()),
    });
    let handle = Arc::new(TsfnHandle {
      id: NEXT_TSFN_ID.fetch_add(1, Ordering::Relaxed),
      native: native.clone(),
      lifecycle: lifecycle.clone(),
    });
    let finalize_native = Weak::into_raw(Arc::downgrade(&native));

    let mut resource_name = ptr::null_mut();
    let name = b"dynwinrt.managedTsfn";
    let name_status = unsafe {
      sys::napi_create_string_utf8(
        env,
        name.as_ptr().cast(),
        name.len() as isize,
        &mut resource_name,
      )
    };
    if name_status != sys::Status::napi_ok {
      unsafe {
        drop(Box::from_raw(context_ptr));
        drop(Weak::from_raw(finalize_native));
      }
      return Err(napi::Error::from_reason(format!(
        "Failed to create managed TSFN resource name: {}",
        Status::from(name_status),
      )));
    }

    let mut raw = ptr::null_mut();
    let create_status = unsafe {
      sys::napi_create_threadsafe_function(
        env,
        callback,
        ptr::null_mut(),
        resource_name,
        max_queue_size,
        1,
        finalize_native.cast_mut().cast(),
        Some(finalize_tsfn::<T>),
        context_ptr.cast(),
        Some(call_js::<T>),
        &mut raw,
      )
    };
    if create_status != sys::Status::napi_ok {
      unsafe {
        drop(Box::from_raw(context_ptr));
        drop(Weak::from_raw(finalize_native));
      }
      return Err(napi::Error::from_reason(format!(
        "Failed to create managed TSFN: {}",
        Status::from(create_status),
      )));
    }
    native.raw.store(raw, Ordering::Release);

    if weak {
      let status = unsafe { sys::napi_unref_threadsafe_function(env, raw) };
      if status != sys::Status::napi_ok {
        native.release(sys::ThreadsafeFunctionReleaseMode::abort);
        return Err(napi::Error::from_reason(format!(
          "Failed to unref managed TSFN: {}",
          Status::from(status),
        )));
      }
    }

    handle.lifecycle.register(handle.id, native);
    drop(open);
    Ok(Self {
      handle,
      _payload: std::marker::PhantomData,
    })
  }

  pub(crate) fn lifecycle(&self) -> Arc<TsfnLifecycle> {
    self.handle.lifecycle.clone()
  }

  pub(crate) fn call(&self, value: T) -> Status {
    self.handle.lifecycle.with_open(Status::Closing, || {
      #[cfg(feature = "test-hooks")]
      {
        if TEST_PAUSE_CALL.load(Ordering::SeqCst) {
          TEST_CALL_PAUSED.store(true, Ordering::SeqCst);
          while TEST_PAUSE_CALL.load(Ordering::SeqCst) {
            std::thread::yield_now();
          }
        }
      }
      let raw = self.handle.native.raw.load(Ordering::Acquire);
      if raw.is_null() {
        return Status::Closing;
      }

      let payload = Box::into_raw(Box::new(value));
      let status = unsafe {
        sys::napi_call_threadsafe_function(
          raw,
          payload.cast(),
          sys::ThreadsafeFunctionCallMode::nonblocking,
        )
      };
      if status != sys::Status::napi_ok {
        unsafe { drop(Box::from_raw(payload)) };
        if status == sys::Status::napi_closing {
          self
            .handle
            .native
            .raw
            .store(ptr::null_mut(), Ordering::Release);
        }
      }
      Status::from(status)
    })
  }
}

#[cfg(feature = "test-hooks")]
pub(crate) fn test_arm_call_pause() {
  TEST_CALL_PAUSED.store(false, Ordering::SeqCst);
  TEST_CLEANUP_WAITING.store(false, Ordering::SeqCst);
  TEST_CLEANUP_ACQUIRED.store(false, Ordering::SeqCst);
  TEST_PAUSE_CALL.store(true, Ordering::SeqCst);
}

#[cfg(feature = "test-hooks")]
pub(crate) fn test_call_paused() -> bool {
  TEST_CALL_PAUSED.load(Ordering::SeqCst)
}

#[cfg(feature = "test-hooks")]
pub(crate) fn test_cleanup_waiting() -> bool {
  TEST_CLEANUP_WAITING.load(Ordering::SeqCst)
}

#[cfg(feature = "test-hooks")]
pub(crate) fn test_cleanup_acquired() -> bool {
  TEST_CLEANUP_ACQUIRED.load(Ordering::SeqCst)
}

#[cfg(feature = "test-hooks")]
pub(crate) fn test_release_call_pause() {
  TEST_PAUSE_CALL.store(false, Ordering::SeqCst);
}

#[cfg(feature = "test-hooks")]
pub(crate) fn test_registered_handle_count(env: sys::napi_env) -> usize {
  ENV_LIFECYCLES
    .lock()
    .unwrap_or_else(|error| error.into_inner())
    .get(&(env as usize))
    .and_then(Weak::upgrade)
    .map(|lifecycle| {
      lifecycle
        .handles
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .len()
    })
    .unwrap_or(0)
}

unsafe extern "C" fn finalize_tsfn<T: Send + 'static>(
  env: sys::napi_env,
  finalize_data: *mut c_void,
  finalize_hint: *mut c_void,
) {
  let native = unsafe { Weak::<NativeTsfn>::from_raw(finalize_data.cast()) };
  if let Some(native) = native.upgrade() {
    native.raw.store(ptr::null_mut(), Ordering::Release);
  }
  let mut context = unsafe { Box::<TsfnContext<T>>::from_raw(finalize_hint.cast()) };
  context.finalize(env);
}

unsafe extern "C" fn call_js<T: Send + 'static>(
  env: sys::napi_env,
  callback: sys::napi_value,
  context: *mut c_void,
  data: *mut c_void,
) {
  if data.is_null() {
    return;
  }
  let value = unsafe { *Box::<T>::from_raw(data.cast()) };
  if env.is_null() || callback.is_null() {
    return;
  }

  let context = unsafe { &*context.cast::<TsfnContext<T>>() };
  let args = match (context.mapper)(value, env) {
    Ok(args) => args,
    Err(error) => {
      let error = unsafe { JsError::from(error).into_value(env) };
      unsafe {
        sys::napi_fatal_exception(env, error);
      }
      return;
    }
  };

  let mut receiver = ptr::null_mut();
  let receiver_status = unsafe { sys::napi_get_undefined(env, &mut receiver) };
  if receiver_status != sys::Status::napi_ok {
    report_callback_status(env, receiver_status);
    return;
  }

  let mut result = ptr::null_mut();
  let status = unsafe {
    sys::napi_call_function(
      env,
      receiver,
      callback,
      args.len(),
      args.as_ptr(),
      &mut result,
    )
  };
  report_callback_status(env, status);
}

fn report_callback_status(env: sys::napi_env, status: sys::napi_status) {
  if status == sys::Status::napi_ok {
    return;
  }
  if status == sys::Status::napi_pending_exception {
    let mut error = ptr::null_mut();
    let clear = unsafe { sys::napi_get_and_clear_last_exception(env, &mut error) };
    if clear == sys::Status::napi_ok {
      unsafe {
        sys::napi_fatal_exception(env, error);
      }
    }
    return;
  }
  eprintln!(
    "[dynwinrt] managed TSFN callback failed: {}",
    Status::from(status)
  );
}
