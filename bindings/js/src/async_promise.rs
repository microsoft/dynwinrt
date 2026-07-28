// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
  collections::HashMap,
  ffi::c_void,
  sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{sync_channel, Receiver, SyncSender, TrySendError},
    Arc, LazyLock, Mutex, Weak,
  },
  thread::JoinHandle,
};

use napi::{
  bindgen_prelude::{PromiseRaw, ToNapiValue},
  Env,
};
use napi_derive::napi;
use windows::{
  core::{Array, IUnknown, Interface},
  Win32::System::{
    Threading::GetCurrentThreadId,
    WinRT::{
      IAgileReference, RoGetAgileReference, RoInitialize, RoUninitialize, AGILEREFERENCE_DEFAULT,
      RO_INIT_MULTITHREADED,
    },
  },
};
use windows_future::IAsyncInfo;

use super::DynWinRTValue;

static ENV_DISPATCHERS: LazyLock<Mutex<HashMap<usize, Arc<EnvAsyncDispatcher>>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));

fn trace_async(message: impl AsRef<str>) {
  if std::env::var_os("DYNWINRT_ASYNC_TRACE").is_some() {
    eprintln!("[dynwinrt] async: {}", message.as_ref());
  }
}

fn async_error_message(error: dynwinrt::Error) -> String {
  match error {
    dynwinrt::Error::Canceled => "Async operation was canceled".to_string(),
    other => format!("Async operation failed: {}", other.message()),
  }
}

fn status_error(context: &str, status: napi::sys::napi_status) -> napi::Error {
  napi::Error::from_reason(format!("{context}: {status:?}"))
}

fn status_result(context: &str, status: napi::sys::napi_status) -> napi::Result<()> {
  if status == napi::sys::Status::napi_ok {
    Ok(())
  } else {
    Err(status_error(context, status))
  }
}

type AsyncResultJob = Box<dyn FnOnce(Result<(), String>) + Send + 'static>;

struct AsyncResultWorkerPool {
  sender: Mutex<Option<SyncSender<AsyncResultJob>>>,
  workers: Mutex<Vec<JoinHandle<()>>>,
}

impl AsyncResultWorkerPool {
  fn new() -> Result<Self, String> {
    let worker_count = std::thread::available_parallelism()
      .map(|count| count.get().min(4))
      .unwrap_or(2)
      .max(1);
    let (sender, receiver) = sync_channel::<AsyncResultJob>(256);
    let receiver = Arc::new(Mutex::new(receiver));
    let mut workers = Vec::with_capacity(worker_count);

    for index in 0..worker_count {
      let receiver = receiver.clone();
      match std::thread::Builder::new()
        .name(format!("dynwinrt-async-results-{index}"))
        .spawn(move || run_async_result_worker(receiver))
      {
        Ok(worker) => workers.push(worker),
        Err(error) => {
          drop(sender);
          for worker in workers {
            let _ = worker.join();
          }
          return Err(format!("Failed to start async result worker: {error}"));
        }
      }
    }

    Ok(Self {
      sender: Mutex::new(Some(sender)),
      workers: Mutex::new(workers),
    })
  }

  fn submit(&self, job: AsyncResultJob) -> Result<(), String> {
    let sender = self
      .sender
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    let Some(sender) = sender.as_ref() else {
      return Err("Async result workers are shutting down".to_string());
    };
    match sender.try_send(job) {
      Ok(()) => Ok(()),
      Err(TrySendError::Full(_)) => Err("Async result worker queue is full".to_string()),
      Err(TrySendError::Disconnected(_)) => Err("Async result workers are unavailable".to_string()),
    }
  }

  fn shutdown(&self) {
    self
      .sender
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .take();
    for worker in self
      .workers
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .drain(..)
    {
      let _ = worker.join();
    }
  }
}

fn run_async_result_worker(receiver: Arc<Mutex<Receiver<AsyncResultJob>>>) {
  let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
  let (initialization, _guard) = match initialized {
    Ok(()) => (Ok(()), Some(RoInitializeGuard)),
    Err(error) => (
      Err(format!(
        "Failed to initialize an async result worker: {error}"
      )),
      None,
    ),
  };

  loop {
    let job = receiver
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .recv();
    let Ok(job) = job else {
      break;
    };
    job(initialization.clone());
  }
}

struct AgileObject(IAgileReference);

unsafe impl Send for AgileObject {}
unsafe impl Sync for AgileObject {}

impl AgileObject {
  fn capture(object: &IUnknown) -> Result<Self, String> {
    unsafe {
      RoGetAgileReference(AGILEREFERENCE_DEFAULT, &IUnknown::IID, object)
        .map(Self)
        .map_err(|error| format!("Failed to marshal async COM result: {error}"))
    }
  }

  fn resolve(self) -> Result<IUnknown, String> {
    unsafe {
      self
        .0
        .Resolve::<IUnknown>()
        .map_err(|error| format!("Failed to resolve async COM result on the JS thread: {error}"))
    }
  }
}

// GetResults runs away from the JavaScript apartment. COM interfaces, including
// interfaces nested in arrays and structs, are carried back as agile references
// and resolved only on the JavaScript thread.
struct StoredStruct {
  data: dynwinrt::ValueTypeData,
  objects: Vec<(usize, AgileObject)>,
}

unsafe impl Send for StoredStruct {}
unsafe impl Sync for StoredStruct {}

impl StoredStruct {
  fn capture(mut data: dynwinrt::ValueTypeData) -> Result<Self, String> {
    let mut objects = Vec::new();
    let handle = data.type_handle().clone();
    unsafe {
      Self::capture_fields(&handle, data.as_mut_ptr(), 0, &mut objects)?;
    }
    Ok(Self { data, objects })
  }

  unsafe fn capture_fields(
    handle: &dynwinrt::TypeHandle,
    root: *mut u8,
    base_offset: usize,
    objects: &mut Vec<(usize, AgileObject)>,
  ) -> Result<(), String> {
    for index in 0..handle.field_count() {
      let field_handle = handle.field_type(index);
      let field_kind = field_handle.kind();
      let offset = base_offset + handle.field_offset(index);
      if field_kind.is_com_pointer() {
        let slot = unsafe { root.add(offset) as *mut *mut c_void };
        let raw = unsafe { slot.read() };
        if !raw.is_null() {
          let object = unsafe { IUnknown::from_raw(raw) };
          unsafe { slot.write(std::ptr::null_mut()) };
          let agile = AgileObject::capture(&object)?;
          objects.push((offset, agile));
        }
      } else if matches!(field_kind, dynwinrt::TypeKind::Struct(_)) {
        unsafe { Self::capture_fields(&field_handle, root, offset, objects)? };
      }
    }
    Ok(())
  }

  fn resolve(mut self) -> Result<dynwinrt::ValueTypeData, String> {
    for (offset, agile) in self.objects {
      let object = agile.resolve()?;
      unsafe {
        (self.data.as_mut_ptr().add(offset) as *mut *mut c_void).write(object.into_raw());
      }
    }
    Ok(self.data)
  }
}

enum StoredWinRTValue {
  Direct(dynwinrt::WinRTValue),
  Object(AgileObject),
  Async {
    object: AgileObject,
    async_type: dynwinrt::TypeHandle,
  },
  ArrayOfIUnknown(Vec<Option<AgileObject>>),
  Struct(StoredStruct),
  Array {
    element_type: dynwinrt::TypeHandle,
    values: Vec<StoredWinRTValue>,
  },
}

unsafe impl Send for StoredWinRTValue {}
unsafe impl Sync for StoredWinRTValue {}

impl StoredWinRTValue {
  fn capture(value: dynwinrt::WinRTValue) -> Result<Self, String> {
    match value {
      dynwinrt::WinRTValue::Object(object) => {
        AgileObject::capture(&object).map(StoredWinRTValue::Object)
      }
      dynwinrt::WinRTValue::Async(info) => {
        let object: IUnknown = info
          .info
          .cast()
          .map_err(|error| format!("Failed to access nested async result: {error}"))?;
        Ok(StoredWinRTValue::Async {
          object: AgileObject::capture(&object)?,
          async_type: info.async_type,
        })
      }
      dynwinrt::WinRTValue::ArrayOfIUnknown(data) => {
        let mut values = Vec::with_capacity(data.0.len());
        for object in data.0.iter() {
          values.push(object.as_ref().map(AgileObject::capture).transpose()?);
        }
        Ok(StoredWinRTValue::ArrayOfIUnknown(values))
      }
      dynwinrt::WinRTValue::Struct(data) => {
        StoredStruct::capture(data).map(StoredWinRTValue::Struct)
      }
      dynwinrt::WinRTValue::Array(data) => {
        let element_type = data.element_type.clone();
        let mut values = Vec::with_capacity(data.len());
        for index in 0..data.len() {
          values.push(Self::capture(data.get(index))?);
        }
        Ok(StoredWinRTValue::Array {
          element_type,
          values,
        })
      }
      dynwinrt::WinRTValue::RawPtr(_) | dynwinrt::WinRTValue::OutValue(_, _) => {
        Err("Async GetResults returned an unresolved ABI pointer".to_string())
      }
      direct => Ok(StoredWinRTValue::Direct(direct)),
    }
  }

  fn resolve(self) -> Result<dynwinrt::WinRTValue, String> {
    match self {
      StoredWinRTValue::Direct(value) => Ok(value),
      StoredWinRTValue::Object(object) => object.resolve().map(dynwinrt::WinRTValue::Object),
      StoredWinRTValue::Async { object, async_type } => {
        let object = object.resolve()?;
        let info: IAsyncInfo = object
          .cast()
          .map_err(|error| format!("Failed to restore nested async result: {error}"))?;
        Ok(dynwinrt::WinRTValue::Async(dynwinrt::AsyncInfo {
          info,
          async_type,
        }))
      }
      StoredWinRTValue::ArrayOfIUnknown(values) => {
        let mut array = Array::<IUnknown>::with_len(values.len());
        for (index, value) in values.into_iter().enumerate() {
          array[index] = value.map(AgileObject::resolve).transpose()?;
        }
        Ok(dynwinrt::WinRTValue::ArrayOfIUnknown(
          dynwinrt::ArrayOfIUnknownData(array),
        ))
      }
      StoredWinRTValue::Struct(value) => value.resolve().map(dynwinrt::WinRTValue::Struct),
      StoredWinRTValue::Array {
        element_type,
        values,
      } => {
        let values = values
          .into_iter()
          .map(StoredWinRTValue::resolve)
          .collect::<Result<Vec<_>, _>>()?;
        Ok(dynwinrt::WinRTValue::Array(
          dynwinrt::ArrayData::from_values(element_type, &values),
        ))
      }
    }
  }
}

type StoredCompletion = Result<StoredWinRTValue, String>;

struct AsyncPromiseState {
  id: u64,
  deferred: napi::sys::napi_deferred,
  async_context: napi::sys::napi_async_context,
  resource_ref: napi::sys::napi_ref,
  js_thread_id: u32,
  operation: Mutex<Option<dynwinrt::WinRTValue>>,
  result: Mutex<Option<StoredCompletion>>,
  settled: AtomicBool,
  fallback_queued: AtomicBool,
  dispatcher: Weak<EnvAsyncDispatcher>,
}

unsafe impl Send for AsyncPromiseState {}
unsafe impl Sync for AsyncPromiseState {}

impl AsyncPromiseState {
  fn complete(self: &Arc<Self>, result: StoredCompletion) {
    let Some(dispatcher) = self.dispatcher.upgrade() else {
      self.abandon();
      return;
    };
    if dispatcher.closing.load(Ordering::Acquire) {
      self.abandon();
      return;
    }

    let mut stored = self
      .result
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    if stored.is_some() || self.settled.load(Ordering::Acquire) {
      return;
    }
    *stored = Some(result);
    drop(stored);
    dispatcher.route_completion(self.clone());
  }

  fn abandon(&self) {
    if !self.settled.swap(true, Ordering::AcqRel) {
      self
        .operation
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
      self
        .result
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    }
  }

  fn take_operation(&self) -> Option<dynwinrt::WinRTValue> {
    self
      .operation
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .take()
  }

  fn settle_on_js(self: &Arc<Self>, env: napi::sys::napi_env) {
    let Some(dispatcher) = self.dispatcher.upgrade() else {
      self.abandon();
      return;
    };
    if dispatcher.closing.load(Ordering::Acquire) {
      self.abandon();
      return;
    }
    if self.settled.swap(true, Ordering::AcqRel) {
      return;
    }

    let result = self
      .result
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .take()
      .unwrap_or_else(|| Err("Async completion result was unavailable".to_string()));

    let mut resource = std::ptr::null_mut();
    let get_resource_status =
      unsafe { napi::sys::napi_get_reference_value(env, self.resource_ref, &mut resource) };
    let mut callback_scope = std::ptr::null_mut();
    let open_scope_status = if get_resource_status == napi::sys::Status::napi_ok {
      unsafe {
        napi::sys::napi_open_callback_scope(env, resource, self.async_context, &mut callback_scope)
      }
    } else {
      get_resource_status
    };
    if open_scope_status == napi::sys::Status::napi_ok {
      self.settle_deferred(env, result);
      let close_status = unsafe { napi::sys::napi_close_callback_scope(env, callback_scope) };
      if close_status != napi::sys::Status::napi_ok {
        eprintln!("[dynwinrt] async callback scope close failed: {close_status:?}");
      }
    } else {
      eprintln!("[dynwinrt] async callback scope open failed: {open_scope_status:?}");
      self.settle_deferred(
        env,
        Err(format!(
          "Failed to open async callback scope: {open_scope_status:?}"
        )),
      );
    }

    let destroy_status = unsafe { napi::sys::napi_async_destroy(env, self.async_context) };
    if destroy_status != napi::sys::Status::napi_ok {
      eprintln!("[dynwinrt] async context destroy failed: {destroy_status:?}");
    }
    let delete_status = unsafe { napi::sys::napi_delete_reference(env, self.resource_ref) };
    if delete_status != napi::sys::Status::napi_ok {
      eprintln!("[dynwinrt] async resource reference delete failed: {delete_status:?}");
    }
    dispatcher.finish_promise(self.id, env);
  }

  fn settle_deferred(&self, env: napi::sys::napi_env, result: StoredCompletion) {
    let result = result
      .and_then(StoredWinRTValue::resolve)
      .and_then(|value| {
        unsafe { DynWinRTValue::to_napi_value(env, DynWinRTValue(value)) }
          .map_err(|error| format!("Async result conversion failed: {error}"))
      });

    match result {
      Ok(value) => {
        let status = unsafe { napi::sys::napi_resolve_deferred(env, self.deferred, value) };
        if status != napi::sys::Status::napi_ok {
          eprintln!("[dynwinrt] async Promise resolution failed: {status:?}");
        }
      }
      Err(reason) => {
        let mut message = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let string_status = unsafe {
          napi::sys::napi_create_string_utf8(
            env,
            reason.as_ptr().cast(),
            reason.len() as isize,
            &mut message,
          )
        };
        let error_status = if string_status == napi::sys::Status::napi_ok {
          unsafe { napi::sys::napi_create_error(env, std::ptr::null_mut(), message, &mut error) }
        } else {
          string_status
        };
        if error_status == napi::sys::Status::napi_ok {
          let reject_status = unsafe { napi::sys::napi_reject_deferred(env, self.deferred, error) };
          if reject_status != napi::sys::Status::napi_ok {
            eprintln!("[dynwinrt] async Promise rejection failed: {reject_status:?}");
          }
        } else {
          eprintln!("[dynwinrt] async promise error creation failed: {error_status:?}");
        }
      }
    }
  }
}

struct QueueRegistration {
  owner: dynwinrt::SystemDispatcherQueue,
  handle: dynwinrt::SystemDispatcherQueueHandle,
  accepting: bool,
  pending: HashMap<u64, Arc<AsyncPromiseState>>,
}

struct EnvAsyncDispatcher {
  env: napi::sys::napi_env,
  js_thread_id: u32,
  tsfn: napi::sys::napi_threadsafe_function,
  closing: AtomicBool,
  next_id: AtomicU64,
  registry: Mutex<HashMap<u64, Arc<AsyncPromiseState>>>,
  tsfn_referenced: AtomicBool,
  queue: Mutex<Option<QueueRegistration>>,
  result_workers: Mutex<Option<AsyncResultWorkerPool>>,
}

unsafe impl Send for EnvAsyncDispatcher {}
unsafe impl Sync for EnvAsyncDispatcher {}

impl EnvAsyncDispatcher {
  fn create(env: Env) -> napi::Result<Arc<Self>> {
    let raw_env = env.raw();
    let mut resource_name = std::ptr::null_mut();
    let resource = b"dynwinrt.asyncPromise.dispatcher";
    status_result("Failed to create shared async dispatcher name", unsafe {
      napi::sys::napi_create_string_utf8(
        raw_env,
        resource.as_ptr().cast(),
        resource.len() as isize,
        &mut resource_name,
      )
    })?;

    let mut tsfn = std::ptr::null_mut();
    status_result("Failed to create shared async dispatcher", unsafe {
      napi::sys::napi_create_threadsafe_function(
        raw_env,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        resource_name,
        0,
        1,
        std::ptr::null_mut(),
        None,
        std::ptr::null_mut(),
        Some(shared_tsfn_callback),
        &mut tsfn,
      )
    })?;
    let unref_status = unsafe { napi::sys::napi_unref_threadsafe_function(raw_env, tsfn) };
    if unref_status != napi::sys::Status::napi_ok {
      unsafe {
        napi::sys::napi_release_threadsafe_function(
          tsfn,
          napi::sys::ThreadsafeFunctionReleaseMode::abort,
        );
      }
      return Err(status_error(
        "Failed to unref shared async dispatcher",
        unref_status,
      ));
    }

    let dispatcher = Arc::new(Self {
      env: raw_env,
      js_thread_id: unsafe { GetCurrentThreadId() },
      tsfn,
      closing: AtomicBool::new(false),
      next_id: AtomicU64::new(1),
      registry: Mutex::new(HashMap::new()),
      tsfn_referenced: AtomicBool::new(false),
      queue: Mutex::new(None),
      result_workers: Mutex::new(None),
    });
    let cleanup_dispatcher = dispatcher.clone();
    if let Err(error) = env.add_env_cleanup_hook(cleanup_dispatcher, |dispatcher| {
      dispatcher.cleanup_env();
    }) {
      unsafe {
        napi::sys::napi_release_threadsafe_function(
          tsfn,
          napi::sys::ThreadsafeFunctionReleaseMode::abort,
        );
      }
      return Err(error);
    }

    trace_async(format!(
      "created shared Node dispatcher for env {raw_env:p}"
    ));
    Ok(dispatcher)
  }

  fn get_or_create(env: Env) -> napi::Result<Arc<Self>> {
    let key = env.raw() as usize;
    let mut dispatchers = ENV_DISPATCHERS
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    if let Some(dispatcher) = dispatchers.get(&key) {
      return Ok(dispatcher.clone());
    }

    let dispatcher = Self::create(env)?;
    dispatchers.insert(key, dispatcher.clone());
    Ok(dispatcher)
  }

  fn existing(env: Env) -> Option<Arc<Self>> {
    ENV_DISPATCHERS
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .get(&(env.raw() as usize))
      .cloned()
  }

  fn register_promise(
    self: &Arc<Self>,
    state: Arc<AsyncPromiseState>,
    env: napi::sys::napi_env,
  ) -> napi::Result<()> {
    if self.closing.load(Ordering::Acquire) {
      return Err(napi::Error::from_reason(
        "Cannot create a Promise while the Node environment is closing",
      ));
    }

    let state_id = state.id;
    let should_ref = {
      let mut registry = self
        .registry
        .lock()
        .unwrap_or_else(|error| error.into_inner());
      let should_ref = registry.is_empty();
      registry.insert(state.id, state);
      should_ref
    };

    if should_ref {
      let status = unsafe { napi::sys::napi_ref_threadsafe_function(env, self.tsfn) };
      if status != napi::sys::Status::napi_ok {
        self
          .registry
          .lock()
          .unwrap_or_else(|error| error.into_inner())
          .remove(&state_id);
        return Err(status_error(
          "Failed to keep the shared async dispatcher alive",
          status,
        ));
      }
      self.tsfn_referenced.store(true, Ordering::Release);
    }
    Ok(())
  }

  fn finish_promise(&self, id: u64, env: napi::sys::napi_env) {
    let should_unref = {
      let mut registry = self
        .registry
        .lock()
        .unwrap_or_else(|error| error.into_inner());
      let removed = registry.remove(&id).is_some();
      removed && registry.is_empty() && self.tsfn_referenced.swap(false, Ordering::AcqRel)
    };
    if should_unref && !self.closing.load(Ordering::Acquire) {
      let status = unsafe { napi::sys::napi_unref_threadsafe_function(env, self.tsfn) };
      if status != napi::sys::Status::napi_ok {
        eprintln!("[dynwinrt] shared async dispatcher unref failed: {status:?}");
      }
    }
  }

  fn collect_result_on_worker(
    &self,
    state: Arc<AsyncPromiseState>,
    operation: dynwinrt::WinRTValue,
  ) {
    let worker_state = state.clone();
    let submit_result = {
      let mut workers = self
        .result_workers
        .lock()
        .unwrap_or_else(|error| error.into_inner());
      if workers.is_none() {
        match AsyncResultWorkerPool::new() {
          Ok(pool) => *workers = Some(pool),
          Err(error) => {
            state.complete(Err(error));
            return;
          }
        }
      }
      workers
        .as_ref()
        .expect("result workers initialized")
        .submit(Box::new(move |initialization| {
          if let Err(error) = initialization {
            worker_state.complete(Err(error));
            return;
          }
          let result = collect_async_result(&worker_state, &operation);
          worker_state.complete(result);
        }))
    };
    if let Err(error) = submit_result {
      state.complete(Err(error));
    }
  }

  fn route_completion(self: &Arc<Self>, state: Arc<AsyncPromiseState>) {
    if self.closing.load(Ordering::Acquire) {
      state.abandon();
      return;
    }

    let handle = {
      let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
      match queue.as_mut() {
        Some(queue) if queue.accepting => {
          queue.pending.insert(state.id, state.clone());
          Some(queue.handle.clone())
        }
        _ => None,
      }
    };

    let Some(handle) = handle else {
      self.queue_fallback(state);
      return;
    };

    let weak_dispatcher = Arc::downgrade(self);
    let queue_state = state.clone();
    match handle.try_enqueue(move || {
      if let Some(dispatcher) = weak_dispatcher.upgrade() {
        dispatcher.run_queue_callback(queue_state.clone());
      } else {
        queue_state.abandon();
      }
      Ok(())
    }) {
      Ok(true) => trace_async(format!("promise {} routed to DispatcherQueue", state.id)),
      Ok(false) => {
        self.remove_queue_pending(state.id);
        self.queue_fallback(state);
      }
      Err(error) => {
        self.remove_queue_pending(state.id);
        trace_async(format!(
          "promise {} DispatcherQueue route failed: {error}",
          state.id
        ));
        self.queue_fallback(state);
      }
    }
  }

  fn run_queue_callback(&self, state: Arc<AsyncPromiseState>) {
    self.remove_queue_pending(state.id);
    if self.closing.load(Ordering::Acquire) {
      state.abandon();
      return;
    }
    self.settle_queue_state_on_js(state);
  }

  fn settle_queue_state_on_js(&self, state: Arc<AsyncPromiseState>) {
    let mut scope = std::ptr::null_mut();
    let open_status = unsafe { napi::sys::napi_open_handle_scope(self.env, &mut scope) };
    if open_status != napi::sys::Status::napi_ok {
      eprintln!("[dynwinrt] DispatcherQueue async handle scope open failed: {open_status:?}");
      state.abandon();
      return;
    }
    state.settle_on_js(self.env);
    let close_status = unsafe { napi::sys::napi_close_handle_scope(self.env, scope) };
    if close_status != napi::sys::Status::napi_ok {
      eprintln!("[dynwinrt] DispatcherQueue async handle scope close failed: {close_status:?}");
    }
  }

  fn remove_queue_pending(&self, id: u64) {
    if let Some(queue) = self
      .queue
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .as_mut()
    {
      queue.pending.remove(&id);
    }
  }

  fn queue_fallback(&self, state: Arc<AsyncPromiseState>) {
    if self.closing.load(Ordering::Acquire) {
      state.abandon();
      return;
    }
    if state.fallback_queued.swap(true, Ordering::AcqRel) {
      return;
    }

    let data = Box::into_raw(Box::new(state.clone())).cast();
    let status = unsafe {
      napi::sys::napi_call_threadsafe_function(
        self.tsfn,
        data,
        napi::sys::ThreadsafeFunctionCallMode::nonblocking,
      )
    };
    if status != napi::sys::Status::napi_ok {
      unsafe { drop(Box::from_raw(data.cast::<Arc<AsyncPromiseState>>())) };
      if status == napi::sys::Status::napi_closing {
        state.abandon();
      } else {
        eprintln!(
          "[dynwinrt] shared async dispatcher call failed for promise {}: {status:?}",
          state.id
        );
      }
    } else {
      trace_async(format!(
        "promise {} routed to shared Node dispatcher",
        state.id
      ));
    }
  }

  fn register_queue(self: &Arc<Self>) -> napi::Result<()> {
    if self.closing.load(Ordering::Acquire) {
      return Err(napi::Error::from_reason(
        "Cannot register a DispatcherQueue while the Node environment is closing",
      ));
    }

    {
      let queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
      if queue.is_some() {
        return Ok(());
      }
    }

    let mut owner =
      dynwinrt::SystemDispatcherQueue::ensure_for_current_thread().map_err(map_win_error)?;
    if !owner.has_thread_access().map_err(map_win_error)? {
      return Err(napi::Error::from_reason(
        "The system DispatcherQueue was not captured on its owning thread",
      ));
    }
    let handle = owner.handle();
    let weak_dispatcher = Arc::downgrade(self);
    owner
      .observe_shutdown(
        move || {
          if let Some(dispatcher) = weak_dispatcher.upgrade() {
            dispatcher.settle_queue_pending_on_ui();
          }
        },
        || {},
      )
      .map_err(map_win_error)?;

    let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
    if queue.is_none() {
      *queue = Some(QueueRegistration {
        owner,
        handle,
        accepting: true,
        pending: HashMap::new(),
      });
      trace_async("registered WinUI system DispatcherQueue");
    }
    Ok(())
  }

  fn drain_queue_pending(&self) -> Vec<Arc<AsyncPromiseState>> {
    {
      let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
      let Some(queue) = queue.as_mut() else {
        return Vec::new();
      };
      queue.accepting = false;
      queue
        .pending
        .drain()
        .map(|(_, state)| state)
        .collect::<Vec<_>>()
    }
  }

  fn settle_queue_pending_on_ui(&self) {
    let pending = self.drain_queue_pending();
    if !pending.is_empty() {
      trace_async(format!(
        "DispatcherQueue shutdown claimed {} pending promise(s)",
        pending.len()
      ));
    }
    let can_settle_directly =
      !self.closing.load(Ordering::Acquire) && unsafe { GetCurrentThreadId() } == self.js_thread_id;
    for state in pending {
      if can_settle_directly {
        self.settle_queue_state_on_js(state);
      } else {
        self.queue_fallback(state);
      }
    }
  }

  fn unregister_queue(&self) -> napi::Result<()> {
    self.settle_queue_pending_on_ui();
    let owner = self
      .queue
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .take()
      .map(|queue| queue.owner);
    if let Some(mut owner) = owner {
      owner.request_shutdown().map_err(map_win_error)?;
    }
    Ok(())
  }

  fn cleanup_env(self: &Arc<Self>) {
    if self.closing.swap(true, Ordering::AcqRel) {
      return;
    }

    for state in self.drain_queue_pending() {
      state.abandon();
    }
    self
      .queue
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .take();
    let pending = self
      .registry
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .drain()
      .map(|(_, state)| state)
      .collect::<Vec<_>>();
    for state in pending {
      state.abandon();
    }
    if let Some(workers) = self
      .result_workers
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .take()
    {
      workers.shutdown();
    }

    ENV_DISPATCHERS
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .remove(&(self.env as usize));
    unsafe {
      napi::sys::napi_release_threadsafe_function(
        self.tsfn,
        napi::sys::ThreadsafeFunctionReleaseMode::abort,
      );
    }
    trace_async(format!(
      "cleaned shared Node dispatcher for env {:p}",
      self.env
    ));
  }
}

fn map_win_error(error: windows::core::Error) -> napi::Error {
  napi::Error::from_reason(error.message())
}

extern "C" fn shared_tsfn_callback(
  env: napi::sys::napi_env,
  _callback: napi::sys::napi_value,
  _context: *mut c_void,
  data: *mut c_void,
) {
  if data.is_null() {
    return;
  }
  let state = unsafe { Box::from_raw(data.cast::<Arc<AsyncPromiseState>>()) };
  if env.is_null() {
    state.abandon();
    return;
  }
  let Some(dispatcher) = state.dispatcher.upgrade() else {
    state.abandon();
    return;
  };
  if dispatcher.closing.load(Ordering::Acquire) {
    state.abandon();
    return;
  }
  state.settle_on_js(env);
}

struct RoInitializeGuard;

impl Drop for RoInitializeGuard {
  fn drop(&mut self) {
    unsafe { RoUninitialize() };
  }
}

fn collect_async_result(
  state: &Arc<AsyncPromiseState>,
  operation: &dynwinrt::WinRTValue,
) -> StoredCompletion {
  dynwinrt::get_async_results(operation)
    .map_err(async_error_message)
    .and_then(StoredWinRTValue::capture)
    .map_err(|error| {
      trace_async(format!("promise {} GetResults failed: {error}", state.id));
      error
    })
}

fn complete_async_operation(state: Arc<AsyncPromiseState>, operation: dynwinrt::WinRTValue) {
  let current_thread_id = unsafe { GetCurrentThreadId() };
  if current_thread_id != state.js_thread_id {
    let result = collect_async_result(&state, &operation);
    state.complete(result);
    return;
  }

  let Some(dispatcher) = state.dispatcher.upgrade() else {
    state.abandon();
    return;
  };
  dispatcher.collect_result_on_worker(state, operation);
}

fn create_async_promise<'env>(
  env: Env,
  operation: dynwinrt::WinRTValue,
) -> napi::Result<(
  PromiseRaw<'env, DynWinRTValue>,
  Arc<AsyncPromiseState>,
  Arc<EnvAsyncDispatcher>,
)> {
  let raw_env = env.raw();
  let dispatcher = EnvAsyncDispatcher::get_or_create(env)?;
  let mut deferred = std::ptr::null_mut();
  let mut promise = std::ptr::null_mut();
  status_result("Failed to create async WinRT Promise", unsafe {
    napi::sys::napi_create_promise(raw_env, &mut deferred, &mut promise)
  })?;

  let mut resource = std::ptr::null_mut();
  status_result("Failed to create async WinRT resource", unsafe {
    napi::sys::napi_create_object(raw_env, &mut resource)
  })?;
  let mut resource_ref = std::ptr::null_mut();
  status_result("Failed to retain async WinRT resource", unsafe {
    napi::sys::napi_create_reference(raw_env, resource, 1, &mut resource_ref)
  })?;

  let mut resource_name = std::ptr::null_mut();
  let resource_name_bytes = b"dynwinrt.asyncPromise";
  let name_status = unsafe {
    napi::sys::napi_create_string_utf8(
      raw_env,
      resource_name_bytes.as_ptr().cast(),
      resource_name_bytes.len() as isize,
      &mut resource_name,
    )
  };
  if name_status != napi::sys::Status::napi_ok {
    unsafe {
      napi::sys::napi_delete_reference(raw_env, resource_ref);
    }
    return Err(status_error(
      "Failed to create async WinRT resource name",
      name_status,
    ));
  }

  let mut async_context = std::ptr::null_mut();
  let async_status =
    unsafe { napi::sys::napi_async_init(raw_env, resource, resource_name, &mut async_context) };
  if async_status != napi::sys::Status::napi_ok {
    unsafe {
      napi::sys::napi_delete_reference(raw_env, resource_ref);
    }
    return Err(status_error(
      "Failed to initialize async WinRT context",
      async_status,
    ));
  }

  let id = dispatcher.next_id.fetch_add(1, Ordering::Relaxed);
  let state = Arc::new(AsyncPromiseState {
    id,
    deferred,
    async_context,
    resource_ref,
    js_thread_id: unsafe { GetCurrentThreadId() },
    operation: Mutex::new(Some(operation)),
    result: Mutex::new(None),
    settled: AtomicBool::new(false),
    fallback_queued: AtomicBool::new(false),
    dispatcher: Arc::downgrade(&dispatcher),
  });
  if let Err(error) = dispatcher.register_promise(state.clone(), raw_env) {
    unsafe {
      napi::sys::napi_async_destroy(raw_env, async_context);
      napi::sys::napi_delete_reference(raw_env, resource_ref);
    }
    return Err(error);
  }

  trace_async(format!("registered promise {id}"));
  Ok((PromiseRaw::new(raw_env, promise), state, dispatcher))
}

pub fn to_promise<'env>(
  env: Env,
  operation: dynwinrt::WinRTValue,
) -> napi::Result<PromiseRaw<'env, DynWinRTValue>> {
  let (promise, state, dispatcher) = create_async_promise(env, operation.clone())?;
  let completion_state = state.clone();
  let registration = dynwinrt::set_async_completed_handler(
    &operation,
    Box::new(move || {
      if let Some(completion_operation) = completion_state.take_operation() {
        complete_async_operation(completion_state.clone(), completion_operation);
      }
    }),
  );
  if let Err(error) = registration {
    state.take_operation();
    state.complete(Err(async_error_message(error)));
  }

  drop(dispatcher);
  Ok(promise)
}

#[napi]
/// Capture the current WinUI STA's system DispatcherQueue for Promise settlement.
pub fn register_winui_dispatcher_queue(env: Env) -> napi::Result<()> {
  EnvAsyncDispatcher::get_or_create(env)?.register_queue()
}

#[napi]
/// Stop routing Promise settlements to the captured WinUI DispatcherQueue.
pub fn unregister_winui_dispatcher_queue(env: Env) -> napi::Result<()> {
  if let Some(dispatcher) = EnvAsyncDispatcher::existing(env) {
    dispatcher.unregister_queue()
  } else {
    Ok(())
  }
}
