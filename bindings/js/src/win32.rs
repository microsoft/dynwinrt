// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::cell::UnsafeCell;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex};

use napi::bindgen_prelude::{BigInt, Buffer, FromNapiValue, Function, ToNapiValue, Unknown};
use napi::JsValue;
use napi_derive::napi;

use super::{com, managed_tsfn::ManagedTsfn, DynWinRTValue, WinGUID};

const ERROR_IO_PENDING: u32 = 997;
const ERROR_OPERATION_ABORTED: u32 = 995;
const ERROR_HANDLE_EOF: u32 = 38;
const ERROR_BROKEN_PIPE: u32 = 109;
const MAX_NATIVE_AGGREGATE_DESCRIPTOR_LENGTH: usize = 1024 * 1024;
const OVERLAPPED_WAITER_THREADS: usize = 8;

#[repr(C)]
struct NativeOverlapped {
  internal: usize,
  internal_high: usize,
  offset: u32,
  offset_high: u32,
  event: *mut std::ffi::c_void,
}

windows_link::link!("kernel32.dll" "system" "CreateEventW" fn create_event_w(
  event_attributes: *mut std::ffi::c_void,
  manual_reset: i32,
  initial_state: i32,
  name: *const u16,
) -> *mut std::ffi::c_void);
windows_link::link!("kernel32.dll" "system" "ReadFile" fn read_file_overlapped(
  file: *mut std::ffi::c_void,
  buffer: *mut std::ffi::c_void,
  bytes_to_read: u32,
  bytes_read: *mut u32,
  overlapped: *mut NativeOverlapped,
) -> i32);
windows_link::link!("kernel32.dll" "system" "WriteFile" fn write_file_overlapped(
  file: *mut std::ffi::c_void,
  buffer: *const std::ffi::c_void,
  bytes_to_write: u32,
  bytes_written: *mut u32,
  overlapped: *mut NativeOverlapped,
) -> i32);
windows_link::link!("kernel32.dll" "system" "GetOverlappedResult" fn get_overlapped_result(
  file: *mut std::ffi::c_void,
  overlapped: *mut NativeOverlapped,
  transferred: *mut u32,
  wait: i32,
) -> i32);
windows_link::link!("kernel32.dll" "system" "CancelIoEx" fn cancel_io_ex(
  file: *mut std::ffi::c_void,
  overlapped: *mut NativeOverlapped,
) -> i32);
windows_link::link!("kernel32.dll" "system" "CloseHandle" fn close_native_handle(
  handle: *mut std::ffi::c_void,
) -> i32);
windows_link::link!("kernel32.dll" "system" "GetLastError" fn get_last_error() -> u32);

#[napi(object)]
pub struct DynWin32ParameterSpec {
  #[napi(js_name = "type")]
  pub typ: String,
  pub direction: String,
  pub nullable: Option<bool>,
  pub cleanup: Option<String>,
  pub consumes_resource: Option<bool>,
  pub resource_cleanup: Option<String>,
  pub aggregate_descriptor: Option<String>,
}

#[napi(object)]
pub struct DynWin32FunctionSpec {
  pub dll: String,
  pub entry_point: String,
  pub parameters: Vec<DynWin32ParameterSpec>,
  pub return_type: Option<String>,
  pub return_cleanup: Option<String>,
  pub success_rule: Option<String>,
  pub capture_last_error: Option<bool>,
  pub calling_convention: Option<String>,
  pub return_aggregate_descriptor: Option<String>,
}

#[napi]
pub struct DynWin32Value {
  value: dynwinrt::win32::Value,
  pointer_owner: Option<Win32PointerOwner>,
}

enum Win32PointerOwner {
  Native(Arc<DynWinRTValue>),
  Aggregate(Arc<NativeAggregateStorage>),
  PointerSlot {
    inner: Arc<DynWinRTValue>,
    slot: Box<usize>,
  },
}

unsafe impl Send for DynWin32Value {}
unsafe impl Sync for DynWin32Value {}

impl DynWin32Value {
  fn new(value: dynwinrt::win32::Value) -> Self {
    Self {
      value,
      pointer_owner: None,
    }
  }

  fn with_pointer_owner(value: dynwinrt::win32::Value, pointer_owner: DynWinRTValue) -> Self {
    Self {
      value,
      pointer_owner: Some(Win32PointerOwner::Native(Arc::new(pointer_owner))),
    }
  }

  fn validate(&self) -> napi::Result<()> {
    if let Some(Win32PointerOwner::Native(owner)) = &self.pointer_owner {
      com::validate_pointer_owner(owner)?;
    }
    if let Some(Win32PointerOwner::Aggregate(owner)) = &self.pointer_owner {
      let _ = owner.byte_length;
    }
    if let Some(Win32PointerOwner::PointerSlot { inner, slot }) = &self.pointer_owner {
      com::validate_pointer_owner(inner)?;
      let _ = **slot;
    }
    Ok(())
  }
}

struct NativeAggregateStorage {
  state: std::sync::Mutex<NativeAggregateState>,
  byte_length: usize,
  contains_pointers: bool,
  owned_fields: Vec<OwnedNativeField>,
}

#[derive(Clone, Copy)]
struct OwnedNativeField {
  offset: usize,
  cleanup: dynwinrt::win32::Cleanup,
}

struct NativeAggregateState {
  words: Vec<u64>,
  owners: BTreeMap<usize, Arc<DynWinRTValue>>,
  call_succeeded: Option<bool>,
}

impl NativeAggregateStorage {
  fn new(
    byte_length: usize,
    bytes: Option<&[u8]>,
    contains_pointers: bool,
    owned_fields: Vec<OwnedNativeField>,
  ) -> napi::Result<Self> {
    if contains_pointers && bytes.is_some() {
      return Err(napi::Error::from_reason(
        "pointer-bearing native aggregates cannot be initialized from raw bytes",
      ));
    }
    if byte_length > dynwinrt::win32::MAX_NATIVE_AGGREGATE_SIZE {
      return Err(napi::Error::from_reason(format!(
        "native aggregate exceeds the {} byte safety limit",
        dynwinrt::win32::MAX_NATIVE_AGGREGATE_SIZE
      )));
    }
    let word_length = byte_length.div_ceil(std::mem::size_of::<u64>());
    let mut words = Vec::new();
    words.try_reserve_exact(word_length).map_err(|_| {
      napi::Error::from_reason("Unable to allocate flat Win32 native aggregate storage")
    })?;
    words.resize(word_length, 0);
    if let Some(bytes) = bytes {
      if bytes.len() != byte_length {
        return Err(napi::Error::from_reason(format!(
          "native aggregate requires exactly {byte_length} bytes, received {}",
          bytes.len()
        )));
      }
      unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), words.as_mut_ptr().cast::<u8>(), byte_length);
      }
    }
    Ok(Self {
      state: std::sync::Mutex::new(NativeAggregateState {
        words,
        owners: BTreeMap::new(),
        call_succeeded: None,
      }),
      byte_length,
      contains_pointers,
      owned_fields,
    })
  }

  fn pointer(&self) -> *mut std::ffi::c_void {
    self
      .state
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .words
      .as_mut_ptr()
      .cast()
  }

  fn bytes(&self) -> napi::Result<Vec<u8>> {
    if self.contains_pointers {
      return Err(napi::Error::from_reason(
        "raw bytes are unavailable for pointer-bearing native aggregates",
      ));
    }
    let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
    Ok(unsafe {
      std::slice::from_raw_parts(state.words.as_ptr().cast::<u8>(), self.byte_length).to_vec()
    })
  }

  fn write_field(
    &self,
    offset: usize,
    bytes: &[u8],
    owner: Option<Arc<DynWinRTValue>>,
  ) -> napi::Result<()> {
    let end = offset
      .checked_add(bytes.len())
      .filter(|end| *end <= self.byte_length)
      .ok_or_else(|| napi::Error::from_reason("native aggregate field exceeds its layout"))?;
    let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
    unsafe {
      std::ptr::copy_nonoverlapping(
        bytes.as_ptr(),
        state.words.as_mut_ptr().cast::<u8>().add(offset),
        end - offset,
      );
    }
    state.owners.remove(&offset);
    if let Some(owner) = owner {
      state.owners.insert(offset, owner);
    }
    Ok(())
  }

  fn read_field<const N: usize>(&self, offset: usize) -> napi::Result<[u8; N]> {
    let end = offset
      .checked_add(N)
      .filter(|end| *end <= self.byte_length)
      .ok_or_else(|| napi::Error::from_reason("native aggregate field exceeds its layout"))?;
    let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
    let mut bytes = [0u8; N];
    unsafe {
      std::ptr::copy_nonoverlapping(
        state.words.as_ptr().cast::<u8>().add(offset),
        bytes.as_mut_ptr(),
        end - offset,
      );
    }
    Ok(bytes)
  }

  fn take_usize(&self, offset: usize) -> napi::Result<usize> {
    let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
    let end = offset
      .checked_add(std::mem::size_of::<usize>())
      .filter(|end| *end <= self.byte_length)
      .ok_or_else(|| napi::Error::from_reason("native handle field exceeds its layout"))?;
    let mut bytes = [0u8; std::mem::size_of::<usize>()];
    unsafe {
      std::ptr::copy_nonoverlapping(
        state.words.as_ptr().cast::<u8>().add(offset),
        bytes.as_mut_ptr(),
        end - offset,
      );
      std::ptr::write_bytes(
        state.words.as_mut_ptr().cast::<u8>().add(offset),
        0,
        end - offset,
      );
    }
    Ok(usize::from_le_bytes(bytes))
  }

  fn mark_call_result(&self, succeeded: bool) {
    self
      .state
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .call_succeeded = Some(succeeded);
  }

  fn prepare_call(&self) -> napi::Result<()> {
    self.cleanup_owned_fields(true)?;
    self.mark_call_result(false);
    Ok(())
  }

  fn require_success(&self) -> napi::Result<()> {
    match self
      .state
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .call_succeeded
    {
      Some(true) => Ok(()),
      Some(false) => Err(napi::Error::from_reason(
        "native aggregate outputs are unavailable because the native call failed",
      )),
      None => Err(napi::Error::from_reason(
        "native aggregate outputs are unavailable before a successful native call",
      )),
    }
  }

  fn cleanup_owned_fields(&self, only_after_success: bool) -> napi::Result<()> {
    if only_after_success
      && self
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .call_succeeded
        != Some(true)
    {
      return Ok(());
    }
    for field in &self.owned_fields {
      let bits = {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        read_usize_from_words(&state.words, self.byte_length, field.offset)?
      };
      if bits == 0 {
        continue;
      }
      unsafe { dynwinrt::win32::cleanup_owned_resource(bits, field.cleanup) }
        .map_err(|error| napi::Error::from_reason(error.to_string()))?;
      let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
      if read_usize_from_words(&state.words, self.byte_length, field.offset)? == bits {
        write_usize_to_words(&mut state.words, self.byte_length, field.offset, 0)?;
      }
    }
    Ok(())
  }
}

impl Drop for NativeAggregateStorage {
  fn drop(&mut self) {
    let _ = self.cleanup_owned_fields(true);
  }
}

fn read_usize_from_words(words: &[u64], byte_length: usize, offset: usize) -> napi::Result<usize> {
  let end = offset
    .checked_add(std::mem::size_of::<usize>())
    .filter(|end| *end <= byte_length)
    .ok_or_else(|| napi::Error::from_reason("native handle field exceeds its layout"))?;
  let mut bytes = [0u8; std::mem::size_of::<usize>()];
  unsafe {
    std::ptr::copy_nonoverlapping(
      words.as_ptr().cast::<u8>().add(offset),
      bytes.as_mut_ptr(),
      end - offset,
    );
  }
  Ok(usize::from_le_bytes(bytes))
}

fn write_usize_to_words(
  words: &mut [u64],
  byte_length: usize,
  offset: usize,
  value: usize,
) -> napi::Result<()> {
  let end = offset
    .checked_add(std::mem::size_of::<usize>())
    .filter(|end| *end <= byte_length)
    .ok_or_else(|| napi::Error::from_reason("native handle field exceeds its layout"))?;
  unsafe {
    std::ptr::copy_nonoverlapping(
      value.to_le_bytes().as_ptr(),
      words.as_mut_ptr().cast::<u8>().add(offset),
      end - offset,
    );
  }
  Ok(())
}

#[napi]
pub struct DynWin32NativeStruct {
  descriptor: String,
  storage: Arc<NativeAggregateStorage>,
}

#[napi]
impl DynWin32NativeStruct {
  #[napi(getter)]
  pub fn bytes(&self) -> napi::Result<Buffer> {
    Ok(Buffer::from(self.storage.bytes()?))
  }

  #[napi(getter)]
  pub fn length(&self) -> u32 {
    self.storage.byte_length as u32
  }
}

#[napi]
pub struct DynWin32Resource(Arc<dynwinrt::win32::OwnedResource>);

#[napi]
impl DynWin32Resource {
  #[napi(getter)]
  pub fn value(&self) -> BigInt {
    BigInt::from(self.0.raw() as u64)
  }

  #[napi(getter)]
  pub fn closed(&self) -> bool {
    self.0.is_closed()
  }

  #[napi(getter)]
  pub fn busy(&self) -> bool {
    self.0.has_async_leases()
  }

  #[napi(getter)]
  pub fn active(&self) -> bool {
    self.0.has_active_async_io()
  }

  #[napi]
  pub fn close(&self) -> napi::Result<()> {
    self
      .0
      .close()
      .map_err(|error| napi::Error::from_reason(error.to_string()))
  }
}

#[napi]
pub struct DynWin32Function(Arc<dynwinrt::win32::CallPlan>);

#[napi]
impl DynWin32Function {
  #[napi(factory)]
  pub fn bind(spec: DynWin32FunctionSpec) -> napi::Result<Self> {
    bind_function(spec)
  }

  #[napi(getter)]
  pub fn dll(&self) -> String {
    self.0.dll().to_string()
  }

  #[napi(getter)]
  pub fn entry_point(&self) -> String {
    self.0.entry_point().to_string()
  }

  #[napi]
  pub fn invoke(&self, args: Vec<&DynWin32Value>) -> napi::Result<DynWin32CallResult> {
    for value in &args {
      value.validate()?;
    }
    let mut aggregates = args
      .iter()
      .filter_map(|value| match &value.pointer_owner {
        Some(Win32PointerOwner::Aggregate(owner)) => Some(owner),
        _ => None,
      })
      .collect::<Vec<_>>();
    aggregates.sort_by_key(|owner| Arc::as_ptr(owner) as usize);
    if aggregates
      .windows(2)
      .any(|pair| Arc::ptr_eq(pair[0], pair[1]))
    {
      return Err(napi::Error::from_reason(
        "the same native aggregate cannot occupy multiple parameters in one call",
      ));
    }
    let _aggregate_guards = aggregates
      .into_iter()
      .map(|owner| {
        owner
          .state
          .lock()
          .unwrap_or_else(|error| error.into_inner())
      })
      .collect::<Vec<_>>();
    for state in &_aggregate_guards {
      for owner in state.owners.values() {
        com::validate_pointer_owner(owner)?;
      }
    }
    let values = args
      .into_iter()
      .map(|value| value.value.clone())
      .collect::<Vec<_>>();
    let result = unsafe { self.0.invoke(&values) }.map_err(|error| {
      napi::Error::from_reason(format!(
        "DynWin32Function {}!{}: {}",
        self.0.dll(),
        self.0.entry_point(),
        error.message()
      ))
    })?;
    Ok(DynWin32CallResult {
      return_value: result.return_value.map(DynWin32Value::new),
      outputs: Some(result.outputs.into_iter().map(DynWin32Value::new).collect()),
      last_error: result.last_error,
      succeeded: result.succeeded,
    })
  }
}

#[napi]
pub struct DynWin32CallResult {
  return_value: Option<DynWin32Value>,
  outputs: Option<Vec<DynWin32Value>>,
  last_error: Option<u32>,
  succeeded: bool,
}

#[napi]
impl DynWin32CallResult {
  #[napi(getter)]
  pub fn return_value(&mut self) -> napi::Result<Option<DynWin32Value>> {
    Ok(self.return_value.take())
  }

  #[napi(getter)]
  pub fn outputs(&mut self) -> napi::Result<Vec<DynWin32Value>> {
    self
      .outputs
      .take()
      .ok_or_else(|| napi::Error::from_reason("Win32 outputs were already consumed"))
  }

  #[napi(getter)]
  pub fn last_error(&self) -> Option<u32> {
    self.last_error
  }

  #[napi(getter)]
  pub fn succeeded(&self) -> bool {
    self.succeeded
  }
}

#[derive(Clone, Copy)]
enum OverlappedIoKind {
  Read,
  Write,
}

struct OverlappedControl {
  active: bool,
  handle: usize,
}

struct OverlappedState {
  overlapped: UnsafeCell<NativeOverlapped>,
  control: Mutex<OverlappedControl>,
  cancelled: AtomicBool,
}

unsafe impl Send for OverlappedState {}
unsafe impl Sync for OverlappedState {}

impl OverlappedState {
  fn new(offset: u64) -> Arc<Self> {
    Arc::new(Self {
      overlapped: UnsafeCell::new(NativeOverlapped {
        internal: 0,
        internal_high: 0,
        offset: offset as u32,
        offset_high: (offset >> 32) as u32,
        event: std::ptr::null_mut(),
      }),
      control: Mutex::new(OverlappedControl {
        active: false,
        handle: 0,
      }),
      cancelled: AtomicBool::new(false),
    })
  }

  fn activate(&self, handle: usize, event: *mut std::ffi::c_void) {
    unsafe {
      (*self.overlapped.get()).event = event;
    }
    let mut control = self
      .control
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    control.handle = handle;
    control.active = true;
  }

  fn deactivate(&self) {
    let mut control = self
      .control
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    control.active = false;
    control.handle = 0;
  }

  fn cancel(&self) {
    self.cancelled.store(true, Ordering::Release);
    let control = self
      .control
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    if control.active {
      unsafe {
        cancel_io_ex(
          control.handle as *mut std::ffi::c_void,
          self.overlapped.get(),
        );
      }
    }
  }
}

struct NativeEvent(*mut std::ffi::c_void);

impl Drop for NativeEvent {
  fn drop(&mut self) {
    if !self.0.is_null() {
      unsafe {
        close_native_handle(self.0);
      }
    }
  }
}

pub struct OverlappedIoTask {
  kind: OverlappedIoKind,
  lease: dynwinrt::win32::OwnedResourceAsyncLease,
  buffer: Option<Buffer>,
  buffer_len: usize,
  native_buffer: Vec<u8>,
  state: Arc<OverlappedState>,
}

struct OverlappedCompletion {
  task: OverlappedIoTask,
  result: Result<u32, String>,
}

struct OverlappedWork {
  task: OverlappedIoTask,
  completion: ManagedTsfn<OverlappedCompletion>,
}

struct OverlappedWaiterQueue {
  work: Mutex<VecDeque<OverlappedWork>>,
  available: Condvar,
  in_flight: AtomicUsize,
}

struct OverlappedWaiterPool {
  queue: Arc<OverlappedWaiterQueue>,
}

struct OverlappedInFlight<'a>(&'a AtomicUsize);

impl Drop for OverlappedInFlight<'_> {
  fn drop(&mut self) {
    self.0.fetch_sub(1, Ordering::AcqRel);
  }
}

static OVERLAPPED_WAITER_POOL: LazyLock<Result<OverlappedWaiterPool, String>> =
  LazyLock::new(OverlappedWaiterPool::new);

impl OverlappedWaiterPool {
  fn new() -> Result<Self, String> {
    let queue = Arc::new(OverlappedWaiterQueue {
      work: Mutex::new(VecDeque::new()),
      available: Condvar::new(),
      in_flight: AtomicUsize::new(0),
    });
    for index in 0..OVERLAPPED_WAITER_THREADS {
      let worker_queue = Arc::clone(&queue);
      std::thread::Builder::new()
        .name(format!("dynwinrt-overlapped-waiter-{index}"))
        .spawn(move || overlapped_waiter_loop(&worker_queue))
        .map_err(|error| format!("Failed to create bounded OVERLAPPED waiter: {error}"))?;
    }
    Ok(Self { queue })
  }

  fn submit(&self, work: OverlappedWork) -> napi::Result<()> {
    self
      .queue
      .in_flight
      .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
        (count < OVERLAPPED_WAITER_THREADS).then_some(count + 1)
      })
      .map_err(|_| {
        napi::Error::from_reason(format!(
          "OVERLAPPED waiter capacity is full ({OVERLAPPED_WAITER_THREADS} active operations)"
        ))
      })?;
    let mut queue = self
      .queue
      .work
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    queue.push_back(work);
    self.queue.available.notify_one();
    Ok(())
  }
}

fn overlapped_waiter_loop(queue: &OverlappedWaiterQueue) {
  loop {
    let work = {
      let mut pending = queue.work.lock().unwrap_or_else(|error| error.into_inner());
      while pending.is_empty() {
        pending = queue
          .available
          .wait(pending)
          .unwrap_or_else(|error| error.into_inner());
      }
      pending.pop_front().expect("waiter queue is not empty")
    };
    let _in_flight = OverlappedInFlight(&queue.in_flight);
    let OverlappedWork {
      mut task,
      completion,
    } = work;
    let result = task.compute().map_err(|error| error.reason.clone());
    let _ = completion.call(OverlappedCompletion { task, result });
  }
}

#[napi]
pub struct DynWin32OverlappedOperation {
  task: Option<OverlappedIoTask>,
  state: Arc<OverlappedState>,
}

#[napi]
impl DynWin32OverlappedOperation {
  #[napi]
  pub fn cancel(&self) {
    self.state.cancel();
  }

  #[napi]
  pub fn start(
    &mut self,
    #[napi(ts_arg_type = "(error: Error | null, bytesTransferred?: number) => void")]
    callback: Function<'static, (), ()>,
  ) -> napi::Result<()> {
    let task = self
      .task
      .take()
      .ok_or_else(|| napi::Error::from_reason("OVERLAPPED operation was already started"))?;
    let env = callback.value().env;
    let raw_callback = napi::JsValue::raw(&callback);
    let completion = ManagedTsfn::create(
      env,
      raw_callback,
      1,
      false,
      |completion: OverlappedCompletion, env| completion.into_js_arguments(env),
      None,
    )?;
    OVERLAPPED_WAITER_POOL
      .as_ref()
      .map_err(|error| napi::Error::from_reason(error.clone()))?
      .submit(OverlappedWork { task, completion })
  }
}

impl OverlappedIoTask {
  fn compute(&mut self) -> napi::Result<u32> {
    let handle = self.lease.raw();
    perform_overlapped_io(
      self.kind,
      handle,
      &mut self.native_buffer,
      &self.state,
      &mut self.lease,
    )
  }

  fn resolve(mut self, env: napi::sys::napi_env, output: u32) -> napi::Result<u32> {
    if matches!(self.kind, OverlappedIoKind::Read) {
      let transferred = usize::try_from(output)
        .map_err(|_| napi::Error::from_reason("OVERLAPPED result exceeds usize"))?;
      if transferred > self.buffer_len {
        return Err(napi::Error::from_reason(
          "OVERLAPPED result exceeds the original Buffer length",
        ));
      }
      let buffer = self
        .buffer
        .take()
        .ok_or_else(|| napi::Error::from_reason("OVERLAPPED read Buffer is unavailable"))?;
      let raw = unsafe { Buffer::to_napi_value(env, buffer) }?;
      let mut is_buffer = false;
      napi::check_status!(
        unsafe { napi::sys::napi_is_buffer(env, raw, &mut is_buffer) },
        "Failed to revalidate OVERLAPPED read Buffer"
      )?;
      if !is_buffer {
        return Err(napi::Error::from_reason(
          "OVERLAPPED read Buffer is no longer a Node Buffer",
        ));
      }
      let mut pointer = std::ptr::null_mut();
      let mut length = 0usize;
      napi::check_status!(
        unsafe { napi::sys::napi_get_buffer_info(env, raw, &mut pointer, &mut length) },
        "Failed to revalidate OVERLAPPED read Buffer backing storage"
      )?;
      if length != self.buffer_len || (length != 0 && pointer.is_null()) {
        return Err(napi::Error::from_reason(
          "OVERLAPPED read Buffer backing ArrayBuffer was detached or changed",
        ));
      }
      if transferred != 0 {
        unsafe {
          std::ptr::copy_nonoverlapping(
            self.native_buffer.as_ptr(),
            pointer.cast::<u8>(),
            transferred,
          );
        }
      }
    }
    Ok(output)
  }
}

impl OverlappedCompletion {
  fn into_js_arguments(self, env: napi::sys::napi_env) -> napi::Result<Vec<napi::sys::napi_value>> {
    let result = self.result.and_then(|output| {
      self
        .task
        .resolve(env, output)
        .map_err(|error| error.reason.clone())
    });
    match result {
      Ok(output) => {
        let mut null = std::ptr::null_mut();
        napi::check_status!(
          unsafe { napi::sys::napi_get_null(env, &mut null) },
          "Failed to create OVERLAPPED completion null"
        )?;
        let output = unsafe { u32::to_napi_value(env, output) }?;
        Ok(vec![null, output])
      }
      Err(reason) => {
        let mut message = std::ptr::null_mut();
        napi::check_status!(
          unsafe {
            napi::sys::napi_create_string_utf8(
              env,
              reason.as_ptr().cast(),
              reason.len() as isize,
              &mut message,
            )
          },
          "Failed to create OVERLAPPED completion error message"
        )?;
        let mut error = std::ptr::null_mut();
        napi::check_status!(
          unsafe { napi::sys::napi_create_error(env, std::ptr::null_mut(), message, &mut error) },
          "Failed to create OVERLAPPED completion error"
        )?;
        Ok(vec![error])
      }
    }
  }
}

fn perform_overlapped_io(
  kind: OverlappedIoKind,
  handle: usize,
  buffer: &mut [u8],
  state: &Arc<OverlappedState>,
  lease: &mut dynwinrt::win32::OwnedResourceAsyncLease,
) -> napi::Result<u32> {
  if state.cancelled.load(Ordering::Acquire) {
    return Err(napi::Error::from_reason("OVERLAPPED operation was aborted"));
  }
  let event = unsafe { create_event_w(std::ptr::null_mut(), 1, 0, std::ptr::null()) };
  if event.is_null() {
    return Err(last_error("CreateEventW"));
  }
  let _event = NativeEvent(event);
  state.activate(handle, event);
  let length = u32::try_from(buffer.len())
    .map_err(|_| napi::Error::from_reason("OVERLAPPED buffer exceeds u32"))?;
  let started = unsafe {
    match kind {
      OverlappedIoKind::Read => read_file_overlapped(
        handle as *mut std::ffi::c_void,
        buffer.as_mut_ptr().cast(),
        length,
        std::ptr::null_mut(),
        state.overlapped.get(),
      ),
      OverlappedIoKind::Write => write_file_overlapped(
        handle as *mut std::ffi::c_void,
        buffer.as_ptr().cast(),
        length,
        std::ptr::null_mut(),
        state.overlapped.get(),
      ),
    }
  };
  if started == 0 {
    let error = unsafe { get_last_error() };
    if error != ERROR_IO_PENDING {
      state.deactivate();
      if is_read_eof(kind, error) {
        return Ok(0);
      }
      return Err(native_error(
        match kind {
          OverlappedIoKind::Read => "ReadFile",
          OverlappedIoKind::Write => "WriteFile",
        },
        error,
      ));
    }
    lease.mark_active();
  }
  if state.cancelled.load(Ordering::Acquire) {
    state.cancel();
  }
  let mut transferred = 0u32;
  let completed = unsafe {
    get_overlapped_result(
      handle as *mut std::ffi::c_void,
      state.overlapped.get(),
      &mut transferred,
      1,
    )
  };
  let error = (completed == 0).then(|| unsafe { get_last_error() });
  state.deactivate();
  lease.mark_inactive();
  if let Some(error) = error {
    if is_read_eof(kind, error) {
      return Ok(0);
    }
    return Err(native_error(
      if error == ERROR_OPERATION_ABORTED {
        "OVERLAPPED operation"
      } else {
        "GetOverlappedResult"
      },
      error,
    ));
  }
  Ok(transferred)
}

fn is_read_eof(kind: OverlappedIoKind, error: u32) -> bool {
  matches!(kind, OverlappedIoKind::Read) && matches!(error, ERROR_HANDLE_EOF | ERROR_BROKEN_PIPE)
}

fn last_error(function: &str) -> napi::Error {
  native_error(function, unsafe { get_last_error() })
}

fn native_error(function: &str, error: u32) -> napi::Error {
  napi::Error::from_reason(format!("{function} failed with Win32 error {error}"))
}

#[napi]
pub struct DynWin32;

#[napi]
impl DynWin32 {
  #[napi]
  pub fn bool8(value: bool) -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::U8(u8::from(value)))
  }

  #[napi]
  pub fn bool32(value: bool) -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::Bool(value))
  }

  #[napi]
  pub fn i8(value: i8) -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::I8(value))
  }

  #[napi]
  pub fn u8(value: u8) -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::U8(value))
  }

  #[napi]
  pub fn i16(value: i16) -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::I16(value))
  }

  #[napi]
  pub fn u16(value: u16) -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::U16(value))
  }

  #[napi]
  pub fn i32(value: i32) -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::I32(value))
  }

  #[napi]
  pub fn u32(value: u32) -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::U32(value))
  }

  #[napi]
  pub fn i64(value: BigInt) -> napi::Result<DynWin32Value> {
    let (value, lossless) = value.get_i64();
    if !lossless {
      return Err(napi::Error::from_reason(
        "DynWin32.i64(): value must fit a signed 64-bit integer",
      ));
    }
    Ok(DynWin32Value::new(dynwinrt::win32::Value::I64(value)))
  }

  #[napi]
  pub fn u64(value: BigInt) -> napi::Result<DynWin32Value> {
    let (negative, value, lossless) = value.get_u64();
    if negative || !lossless {
      return Err(napi::Error::from_reason(
        "DynWin32.u64(): value must fit an unsigned 64-bit integer",
      ));
    }
    Ok(DynWin32Value::new(dynwinrt::win32::Value::U64(value)))
  }

  #[napi]
  pub fn f32(value: f64) -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::F32(value as f32))
  }

  #[napi]
  pub fn f64(value: f64) -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::F64(value))
  }

  #[napi]
  pub fn handle(
    #[napi(ts_arg_type = "bigint | number | DynWin32Resource | null | undefined")] value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    handle_value(value, nullable.unwrap_or(false))
  }

  #[napi]
  pub fn resource(value: &DynWin32Resource, cleanup: String) -> napi::Result<DynWin32Value> {
    let cleanup = parse_cleanup(&cleanup)?;
    if cleanup == dynwinrt::win32::Cleanup::None || value.0.cleanup() != cleanup {
      return Err(napi::Error::from_reason(
        "Managed Win32 resource cleanup does not match the consuming API",
      ));
    }
    if value.0.is_closed() {
      return Err(napi::Error::from_reason(
        "Cannot consume a closed flat Win32 resource",
      ));
    }
    Ok(DynWin32Value::new(dynwinrt::win32::Value::Resource(
      Arc::clone(&value.0),
    )))
  }

  #[napi]
  pub fn com_object(
    #[napi(ts_arg_type = "DynWinRtValue | null | undefined")] value: Unknown,
    iid: String,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    use windows::core::Interface;

    let env = value.value().env;
    let raw = value.value().value;
    let mut value_type = napi::sys::ValueType::napi_undefined;
    unsafe { napi::sys::napi_typeof(env, raw, &mut value_type) };
    if matches!(
      value_type,
      napi::sys::ValueType::napi_null | napi::sys::ValueType::napi_undefined
    ) {
      return if nullable.unwrap_or(false) {
        Ok(DynWin32Value::new(dynwinrt::win32::Value::Null))
      } else {
        Err(napi::Error::from_reason(
          "DynWin32.comObject(): null requires an explicitly nullable interface",
        ))
      };
    }
    let value = unsafe { <&DynWinRTValue>::from_napi_value(env, raw) }?;
    let iid = WinGUID(
      windows::core::GUID::try_from(iid.as_str())
        .map_err(|_| napi::Error::from_reason("Invalid COM interface IID"))?,
    );
    let owner = com::try_cast(value, &iid)?.ok_or_else(|| {
      napi::Error::from_reason("Managed object does not implement the required Win32 interface")
    })?;
    let pointer = owner
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("Managed value is not a COM object"))?
      .as_raw();
    Ok(DynWin32Value {
      value: dynwinrt::win32::Value::Pointer(pointer),
      pointer_owner: Some(Win32PointerOwner::Native(Arc::new(owner))),
    })
  }

  #[napi]
  pub fn data_pointer(
    #[napi(ts_arg_type = "Buffer | Uint8Array | null | undefined")] value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    let nullable = nullable.unwrap_or(false);
    let value = pointer_value(com::safe_data_pointer(value, nullable)?)?;
    reject_required_null_pointer(&value, nullable)?;
    Ok(value)
  }

  #[napi]
  pub fn aligned_data_pointer(
    #[napi(ts_arg_type = "Buffer | Uint8Array | null | undefined")] value: Unknown,
    alignment: u32,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    if alignment == 0 || !alignment.is_power_of_two() || alignment > 8 {
      return Err(napi::Error::from_reason(
        "DynWin32.alignedDataPointer(): alignment must be 1, 2, 4, or 8",
      ));
    }
    let nullable = nullable.unwrap_or(false);
    let value = pointer_value(com::safe_data_pointer(value, nullable)?)?;
    reject_required_null_pointer(&value, nullable)?;
    if let dynwinrt::win32::Value::Pointer(pointer) = &value.value {
      if !pointer.is_null() && (*pointer as usize) % alignment as usize != 0 {
        return Err(napi::Error::from_reason(format!(
          "native buffer address is not aligned to {alignment} bytes"
        )));
      }
    }
    Ok(value)
  }

  #[napi]
  pub fn wide_string(
    #[napi(ts_arg_type = "string | Buffer | Uint8Array | null | undefined")] value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    pointer_value(com::safe_wide_string_pointer(
      value,
      nullable.unwrap_or(false),
    )?)
  }

  #[napi]
  pub fn ansi_string(
    #[napi(ts_arg_type = "string | Buffer | Uint8Array | null | undefined")] value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    pointer_value(com::safe_ansi_string_pointer(
      value,
      nullable.unwrap_or(false),
    )?)
  }

  #[napi]
  pub fn wide_multi_string(
    #[napi(ts_arg_type = "string | readonly string[] | Buffer | Uint8Array | null | undefined")]
    value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    pointer_value(com::safe_wide_multi_string_pointer(
      value,
      nullable.unwrap_or(false),
    )?)
  }

  #[napi]
  pub fn ansi_multi_string(
    #[napi(ts_arg_type = "string | readonly string[] | Buffer | Uint8Array | null | undefined")]
    value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    pointer_value(com::safe_ansi_multi_string_pointer(
      value,
      nullable.unwrap_or(false),
    )?)
  }

  #[napi]
  pub fn wide_string_pointer_pointer(
    #[napi(ts_arg_type = "string | Buffer | Uint8Array | null | undefined")] value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    string_pointer_pointer(value, nullable.unwrap_or(false), true)
  }

  #[napi]
  pub fn ansi_string_pointer_pointer(
    #[napi(ts_arg_type = "string | Buffer | Uint8Array | null | undefined")] value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    string_pointer_pointer(value, nullable.unwrap_or(false), false)
  }

  #[napi]
  pub fn null_pointer() -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::Null)
  }

  #[napi]
  pub fn begin_read_file(
    file: &DynWin32Resource,
    buffer: Buffer,
    offset: Option<BigInt>,
  ) -> napi::Result<DynWin32OverlappedOperation> {
    overlapped_io_task(OverlappedIoKind::Read, file, buffer, offset)
  }

  #[napi]
  pub fn begin_write_file(
    file: &DynWin32Resource,
    buffer: Buffer,
    offset: Option<BigInt>,
  ) -> napi::Result<DynWin32OverlappedOperation> {
    overlapped_io_task(OverlappedIoKind::Write, file, buffer, offset)
  }

  #[napi]
  pub fn create_native_struct(
    descriptor: String,
    bytes: Option<Buffer>,
  ) -> napi::Result<DynWin32NativeStruct> {
    let (_, size, alignment, contains_pointers, owned_fields) =
      native_aggregate_layout(&descriptor)?;
    if alignment > 8 {
      return Err(napi::Error::from_reason(
        "flat Win32 native aggregate alignment above 8 is unsupported",
      ));
    }
    Ok(DynWin32NativeStruct {
      descriptor,
      storage: Arc::new(NativeAggregateStorage::new(
        size,
        bytes.as_ref().map(|bytes| bytes.as_ref()),
        contains_pointers,
        owned_fields,
      )?),
    })
  }

  #[napi]
  pub fn set_native_struct_u32(
    value: &DynWin32NativeStruct,
    descriptor: String,
    field: String,
    input: u32,
  ) -> napi::Result<()> {
    validate_native_struct(value, &descriptor)?;
    let (offset, kind, _) = native_aggregate_field(&descriptor, &field)?;
    if kind != "u32" {
      return Err(napi::Error::from_reason(format!(
        "native field `{field}` is not u32"
      )));
    }
    value
      .storage
      .write_field(offset, &input.to_le_bytes(), None)
  }

  #[napi]
  pub fn set_native_struct_bool32(
    value: &DynWin32NativeStruct,
    descriptor: String,
    field: String,
    input: bool,
  ) -> napi::Result<()> {
    validate_native_struct(value, &descriptor)?;
    let (offset, kind, _) = native_aggregate_field(&descriptor, &field)?;
    if kind != "i32" {
      return Err(napi::Error::from_reason(format!(
        "native field `{field}` is not BOOL"
      )));
    }
    value
      .storage
      .write_field(offset, &i32::from(input).to_le_bytes(), None)
  }

  #[napi]
  pub fn set_native_struct_pointer(
    value: &DynWin32NativeStruct,
    descriptor: String,
    field: String,
    pointer: &DynWin32Value,
  ) -> napi::Result<()> {
    validate_native_struct(value, &descriptor)?;
    pointer.validate()?;
    let (offset, kind, _) = native_aggregate_field(&descriptor, &field)?;
    if kind != "pointer" {
      return Err(napi::Error::from_reason(format!(
        "native field `{field}` is not a data pointer"
      )));
    }
    let (bits, owner) = match (&pointer.value, &pointer.pointer_owner) {
      (dynwinrt::win32::Value::Null, _) => (0usize, None),
      (dynwinrt::win32::Value::Pointer(pointer), _) if pointer.is_null() => (0usize, None),
      (dynwinrt::win32::Value::Pointer(pointer), Some(Win32PointerOwner::Native(owner)))
        if com::has_native_pointer_owner(owner) =>
      {
        (*pointer as usize, Some(Arc::clone(owner)))
      }
      (dynwinrt::win32::Value::Pointer(_), _) => {
        return Err(napi::Error::from_reason(
          "native struct pointer fields require retained Buffer or string storage",
        ));
      }
      _ => {
        return Err(napi::Error::from_reason(
          "native struct pointer field value is not a data pointer",
        ));
      }
    };
    value
      .storage
      .write_field(offset, &bits.to_le_bytes(), owner)
  }

  #[napi]
  pub fn get_native_struct_u32(
    value: &DynWin32NativeStruct,
    descriptor: String,
    field: String,
  ) -> napi::Result<u32> {
    validate_native_struct(value, &descriptor)?;
    value.storage.require_success()?;
    let (offset, kind, _) = native_aggregate_field(&descriptor, &field)?;
    if kind != "u32" {
      return Err(napi::Error::from_reason(format!(
        "native field `{field}` is not u32"
      )));
    }
    Ok(u32::from_le_bytes(value.storage.read_field(offset)?))
  }

  #[napi]
  pub fn take_native_struct_resource(
    value: &DynWin32NativeStruct,
    descriptor: String,
    field: String,
    cleanup: String,
  ) -> napi::Result<Option<DynWin32Resource>> {
    validate_native_struct(value, &descriptor)?;
    value.storage.require_success()?;
    let (offset, kind, field_cleanup) = native_aggregate_field(&descriptor, &field)?;
    if kind != "handle" || field_cleanup.as_deref() != Some(cleanup.as_str()) {
      return Err(napi::Error::from_reason(format!(
        "native field `{field}` does not have cleanup `{cleanup}`"
      )));
    }
    let bits = value.storage.take_usize(offset)?;
    if bits == 0 {
      return Ok(None);
    }
    let cleanup = parse_cleanup(&cleanup)?;
    let resource = unsafe { dynwinrt::win32::OwnedResource::adopt(bits, cleanup) }
      .map_err(|error| napi::Error::from_reason(error.message()))?;
    Ok(Some(DynWin32Resource(resource)))
  }

  #[napi]
  pub fn mark_native_struct_call_result(
    value: &DynWin32NativeStruct,
    descriptor: String,
    succeeded: bool,
  ) -> napi::Result<()> {
    validate_native_struct(value, &descriptor)?;
    value.storage.mark_call_result(succeeded);
    Ok(())
  }

  #[napi]
  pub fn prepare_native_struct_call(
    value: &DynWin32NativeStruct,
    descriptor: String,
  ) -> napi::Result<()> {
    validate_native_struct(value, &descriptor)?;
    value.storage.prepare_call()
  }

  #[napi]
  pub fn native_struct(
    #[napi(ts_arg_type = "DynWin32NativeStruct | null | undefined")] value: Unknown,
    descriptor: String,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    let env = value.value().env;
    let raw = value.value().value;
    let mut value_type = napi::sys::ValueType::napi_undefined;
    unsafe { napi::sys::napi_typeof(env, raw, &mut value_type) };
    if matches!(
      value_type,
      napi::sys::ValueType::napi_null | napi::sys::ValueType::napi_undefined
    ) {
      return if nullable.unwrap_or(false) {
        Ok(DynWin32Value::new(dynwinrt::win32::Value::Null))
      } else {
        Err(napi::Error::from_reason(
          "DynWin32.nativeStruct(): null requires an explicitly nullable aggregate pointer",
        ))
      };
    }
    let value = unsafe { <&DynWin32NativeStruct>::from_napi_value(env, raw) }?;
    if value.descriptor != descriptor {
      return Err(napi::Error::from_reason(
        "DynWin32.nativeStruct(): native aggregate type mismatch",
      ));
    }
    let pointer = value.storage.pointer();
    Ok(DynWin32Value {
      value: dynwinrt::win32::Value::Pointer(pointer),
      pointer_owner: Some(Win32PointerOwner::Aggregate(Arc::clone(&value.storage))),
    })
  }

  #[napi]
  pub fn native_struct_value(
    value: &DynWin32NativeStruct,
    descriptor: String,
  ) -> napi::Result<DynWin32Value> {
    if value.descriptor != descriptor {
      return Err(napi::Error::from_reason(
        "DynWin32.nativeStructValue(): native aggregate type mismatch",
      ));
    }
    let layout = native_aggregate_call_layout(&descriptor)?;
    Ok(DynWin32Value {
      value: dynwinrt::win32::Value::Aggregate {
        layout,
        pointer: value.storage.pointer(),
      },
      pointer_owner: Some(Win32PointerOwner::Aggregate(Arc::clone(&value.storage))),
    })
  }

  #[napi]
  pub fn to_native_struct(
    value: &DynWin32Value,
    descriptor: String,
  ) -> napi::Result<DynWin32NativeStruct> {
    let dynwinrt::win32::Value::OwnedAggregate { layout, bytes } = &value.value else {
      return Err(napi::Error::from_reason(
        "Win32 value is not an owned native aggregate",
      ));
    };
    if layout.identity() != descriptor {
      return Err(napi::Error::from_reason(
        "native aggregate return identity mismatch",
      ));
    }
    Ok(DynWin32NativeStruct {
      descriptor,
      storage: Arc::new(NativeAggregateStorage::new(
        bytes.len(),
        Some(bytes),
        false,
        Vec::new(),
      )?),
    })
  }

  #[napi]
  pub fn to_number(value: &DynWin32Value) -> napi::Result<f64> {
    Ok(match &value.value {
      dynwinrt::win32::Value::Bool(value) => u8::from(*value) as f64,
      dynwinrt::win32::Value::I8(value) => *value as f64,
      dynwinrt::win32::Value::U8(value) => *value as f64,
      dynwinrt::win32::Value::I16(value) => *value as f64,
      dynwinrt::win32::Value::U16(value) => *value as f64,
      dynwinrt::win32::Value::I32(value) => *value as f64,
      dynwinrt::win32::Value::U32(value) => *value as f64,
      dynwinrt::win32::Value::F32(value) => *value as f64,
      dynwinrt::win32::Value::F64(value) => *value,
      _ => {
        return Err(napi::Error::from_reason(
          "Win32 value is not a JavaScript number",
        ));
      }
    })
  }

  #[napi]
  pub fn to_bigint(value: &DynWin32Value) -> napi::Result<BigInt> {
    match &value.value {
      dynwinrt::win32::Value::I64(value) => Ok(BigInt::from(*value)),
      dynwinrt::win32::Value::U64(value) => Ok(BigInt::from(*value)),
      dynwinrt::win32::Value::FunctionPointer(value) => Ok(BigInt::from(*value as u64)),
      dynwinrt::win32::Value::Handle(value) => Ok(BigInt::from(*value as u64)),
      dynwinrt::win32::Value::Resource(value) => Ok(BigInt::from(value.raw() as u64)),
      dynwinrt::win32::Value::Null => Ok(BigInt::from(0u64)),
      _ => Err(napi::Error::from_reason(
        "Win32 value is not a 64-bit integer, pointer, or handle",
      )),
    }
  }

  #[napi]
  pub fn to_boolean(value: &DynWin32Value) -> napi::Result<bool> {
    match value.value {
      dynwinrt::win32::Value::Bool(value) => Ok(value),
      dynwinrt::win32::Value::U8(value) => Ok(value != 0),
      _ => Err(napi::Error::from_reason("Win32 value is not BOOL")),
    }
  }

  #[napi]
  pub fn to_resource(value: &DynWin32Value) -> napi::Result<Option<DynWin32Resource>> {
    if matches!(
      &value.value,
      dynwinrt::win32::Value::Handle(0) | dynwinrt::win32::Value::Null
    ) {
      return Ok(None);
    }
    value
      .value
      .resource()
      .cloned()
      .map(DynWin32Resource)
      .map(Some)
      .ok_or_else(|| napi::Error::from_reason("Win32 value is not an owned resource"))
  }
}

fn reject_required_null_pointer(value: &DynWin32Value, nullable: bool) -> napi::Result<()> {
  if !nullable
    && matches!(
      &value.value,
      dynwinrt::win32::Value::Pointer(pointer) if pointer.is_null()
    )
  {
    return Err(napi::Error::from_reason(
      "non-nullable native pointer requires non-empty backing storage",
    ));
  }
  Ok(())
}

fn overlapped_io_task(
  kind: OverlappedIoKind,
  file: &DynWin32Resource,
  buffer: Buffer,
  offset: Option<BigInt>,
) -> napi::Result<DynWin32OverlappedOperation> {
  if file.0.cleanup() != dynwinrt::win32::Cleanup::CloseHandle {
    return Err(napi::Error::from_reason(
      "OVERLAPPED I/O requires a CloseHandle resource",
    ));
  }
  if file.0.is_closed() {
    return Err(napi::Error::from_reason(
      "OVERLAPPED I/O cannot use a closed Win32 resource",
    ));
  }
  let handle_bits = file.0.raw();
  if handle_bits == 0 || handle_bits == usize::MAX {
    return Err(napi::Error::from_reason(
      "OVERLAPPED I/O requires a valid file HANDLE",
    ));
  }
  let offset = match offset {
    Some(value) => {
      let (negative, value, lossless) = value.get_u64();
      if negative || !lossless {
        return Err(napi::Error::from_reason(
          "OVERLAPPED offset must fit an unsigned 64-bit integer",
        ));
      }
      value
    }
    None => 0,
  };
  u32::try_from(buffer.len())
    .map_err(|_| napi::Error::from_reason("OVERLAPPED buffer exceeds u32"))?;
  let lease = file
    .0
    .async_lease(dynwinrt::win32::Cleanup::CloseHandle)
    .map_err(|error| napi::Error::from_reason(error.message()))?;
  let state = OverlappedState::new(offset);
  Ok(DynWin32OverlappedOperation {
    task: Some(OverlappedIoTask {
      kind,
      lease,
      native_buffer: try_copy_io_buffer(kind, &buffer)?,
      buffer_len: buffer.len(),
      buffer: Some(buffer),
      state: Arc::clone(&state),
    }),
    state,
  })
}

fn validate_native_struct(value: &DynWin32NativeStruct, descriptor: &str) -> napi::Result<()> {
  if value.descriptor != descriptor {
    return Err(napi::Error::from_reason("native aggregate type mismatch"));
  }
  Ok(())
}

fn native_aggregate_field(
  descriptor: &str,
  field: &str,
) -> napi::Result<(usize, String, Option<String>)> {
  let root = parse_native_aggregate_descriptor(descriptor)?;
  #[cfg(target_arch = "x86")]
  let architecture = "x86";
  #[cfg(target_arch = "x86_64")]
  let architecture = "x64";
  #[cfg(target_arch = "aarch64")]
  let architecture = "arm64";
  #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
  return Err(napi::Error::from_reason(
    "flat Win32 native aggregates support only x86, x64, and ARM64",
  ));
  let fields = root
    .get(architecture)
    .and_then(|layout| layout.get("fields"))
    .and_then(serde_json::Value::as_array)
    .ok_or_else(|| napi::Error::from_reason("native aggregate descriptor has no fields"))?;
  let field = fields
    .iter()
    .find(|candidate| candidate.get("name").and_then(serde_json::Value::as_str) == Some(field))
    .ok_or_else(|| napi::Error::from_reason(format!("unknown native field `{field}`")))?;
  let offset = field
    .get("offset")
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .ok_or_else(|| napi::Error::from_reason("native field has invalid offset"))?;
  let kind = field
    .get("type")
    .and_then(|typ| typ.get("kind"))
    .and_then(serde_json::Value::as_str)
    .ok_or_else(|| napi::Error::from_reason("native field has invalid type"))?;
  let cleanup = field
    .get("type")
    .and_then(|typ| typ.get("cleanup"))
    .and_then(serde_json::Value::as_str)
    .map(str::to_string);
  Ok((offset, kind.to_string(), cleanup))
}

fn native_aggregate_layout(
  descriptor: &str,
) -> napi::Result<(String, usize, usize, bool, Vec<OwnedNativeField>)> {
  let root = parse_native_aggregate_descriptor(descriptor)?;
  let name = root
    .get("name")
    .and_then(serde_json::Value::as_str)
    .ok_or_else(|| napi::Error::from_reason("native aggregate descriptor is missing `name`"))?;
  #[cfg(target_arch = "x86")]
  let architecture = "x86";
  #[cfg(target_arch = "x86_64")]
  let architecture = "x64";
  #[cfg(target_arch = "aarch64")]
  let architecture = "arm64";
  #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
  return Err(napi::Error::from_reason(
    "flat Win32 native aggregates support only x86, x64, and ARM64",
  ));
  let layout = root.get(architecture).ok_or_else(|| {
    napi::Error::from_reason(format!(
      "native aggregate descriptor is missing `{architecture}`"
    ))
  })?;
  let size = layout
    .get("size")
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .filter(|value| *value > 0)
    .ok_or_else(|| napi::Error::from_reason("native aggregate has invalid `size`"))?;
  if size > dynwinrt::win32::MAX_NATIVE_AGGREGATE_SIZE {
    return Err(napi::Error::from_reason(format!(
      "native aggregate exceeds the {} byte safety limit",
      dynwinrt::win32::MAX_NATIVE_AGGREGATE_SIZE
    )));
  }
  let alignment = layout
    .get("alignment")
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .filter(|value| value.is_power_of_two() && size % *value == 0)
    .ok_or_else(|| napi::Error::from_reason("native aggregate has invalid `alignment`"))?;
  Ok((
    name.to_string(),
    size,
    alignment,
    native_layout_contains_pointers(layout),
    native_layout_owned_fields(layout)?,
  ))
}

fn native_layout_owned_fields(layout: &serde_json::Value) -> napi::Result<Vec<OwnedNativeField>> {
  layout
    .get("fields")
    .and_then(serde_json::Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(|field| {
      let typ = field.get("type")?;
      (typ.get("kind").and_then(serde_json::Value::as_str) == Some("handle"))
        .then_some((field, typ))
    })
    .map(|(field, typ)| {
      let offset = field
        .get("offset")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| napi::Error::from_reason("owned native field has invalid offset"))?;
      let cleanup = typ
        .get("cleanup")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| napi::Error::from_reason("owned native field has no cleanup"))
        .and_then(parse_cleanup)?;
      if cleanup == dynwinrt::win32::Cleanup::None {
        return Ok(None);
      }
      Ok(Some(OwnedNativeField { offset, cleanup }))
    })
    .filter_map(|result| result.transpose())
    .collect()
}

fn native_layout_contains_pointers(layout: &serde_json::Value) -> bool {
  layout
    .get("fields")
    .and_then(serde_json::Value::as_array)
    .is_some_and(|fields| {
      fields.iter().any(|field| {
        field
          .get("type")
          .and_then(|typ| {
            typ
              .get("kind")
              .and_then(serde_json::Value::as_str)
              .map(|kind| (typ, kind))
          })
          .is_some_and(|(typ, kind)| match kind {
            "pointer" | "handle" => true,
            "struct" | "union" => typ
              .get("layout")
              .is_some_and(native_layout_contains_pointers),
            _ => false,
          })
      })
    })
}

fn native_aggregate_call_layout(
  descriptor: &str,
) -> napi::Result<Arc<dynwinrt::win32::NativeAggregateLayout>> {
  let root = parse_native_aggregate_descriptor(descriptor)?;
  #[cfg(target_arch = "x86")]
  let architecture = "x86";
  #[cfg(target_arch = "x86_64")]
  let architecture = "x64";
  #[cfg(target_arch = "aarch64")]
  let architecture = "arm64";
  #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
  return Err(napi::Error::from_reason(
    "flat Win32 native aggregates support only x86, x64, and ARM64",
  ));
  let layout = root.get(architecture).ok_or_else(|| {
    napi::Error::from_reason(format!(
      "native aggregate descriptor is missing `{architecture}`"
    ))
  })?;
  let size = layout
    .get("size")
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .ok_or_else(|| napi::Error::from_reason("native aggregate has invalid `size`"))?;
  let alignment = layout
    .get("alignment")
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .ok_or_else(|| napi::Error::from_reason("native aggregate has invalid `alignment`"))?;
  let ffi_type = native_aggregate_ffi_type(layout)?;
  dynwinrt::win32::NativeAggregateLayout::new(descriptor, size, alignment, ffi_type)
    .map_err(|error| napi::Error::from_reason(error.message()))
}

fn parse_native_aggregate_descriptor(descriptor: &str) -> napi::Result<serde_json::Value> {
  if descriptor.len() > MAX_NATIVE_AGGREGATE_DESCRIPTOR_LENGTH {
    return Err(napi::Error::from_reason(format!(
      "flat Win32 native aggregate descriptor exceeds the {MAX_NATIVE_AGGREGATE_DESCRIPTOR_LENGTH} byte safety limit"
    )));
  }
  serde_json::from_str(descriptor).map_err(|error| {
    napi::Error::from_reason(format!(
      "Invalid flat Win32 native aggregate descriptor: {error}"
    ))
  })
}

fn try_copy_io_buffer(kind: OverlappedIoKind, buffer: &Buffer) -> napi::Result<Vec<u8>> {
  let mut native = Vec::new();
  native
    .try_reserve_exact(buffer.len())
    .map_err(|_| napi::Error::from_reason("Unable to allocate private OVERLAPPED I/O buffer"))?;
  match kind {
    OverlappedIoKind::Read => native.resize(buffer.len(), 0),
    OverlappedIoKind::Write => native.extend_from_slice(buffer),
  }
  Ok(native)
}

fn native_aggregate_ffi_type(layout: &serde_json::Value) -> napi::Result<libffi::middle::Type> {
  let size = layout
    .get("size")
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .ok_or_else(|| napi::Error::from_reason("native aggregate has invalid `size`"))?;
  let mut fields = layout
    .get("fields")
    .and_then(serde_json::Value::as_array)
    .ok_or_else(|| napi::Error::from_reason("native aggregate is missing `fields`"))?
    .iter()
    .collect::<Vec<_>>();
  fields.sort_by_key(|field| {
    field
      .get("offset")
      .and_then(serde_json::Value::as_u64)
      .unwrap_or(u64::MAX)
  });
  let mut elements = Vec::new();
  let mut cursor = 0usize;
  for field in fields {
    let offset = field
      .get("offset")
      .and_then(serde_json::Value::as_u64)
      .and_then(|value| usize::try_from(value).ok())
      .ok_or_else(|| napi::Error::from_reason("native field has invalid `offset`"))?;
    if offset < cursor {
      return Err(napi::Error::from_reason(
        "overlapping native fields cannot be passed by value",
      ));
    }
    elements.extend(std::iter::repeat_with(libffi::middle::Type::u8).take(offset - cursor));
    let count = field
      .get("count")
      .and_then(serde_json::Value::as_u64)
      .and_then(|value| usize::try_from(value).ok())
      .filter(|value| *value > 0)
      .ok_or_else(|| napi::Error::from_reason("native field has invalid `count`"))?;
    let typ = field
      .get("type")
      .ok_or_else(|| napi::Error::from_reason("native field is missing `type`"))?;
    let (field_type, field_size) = native_ffi_field_type(typ)?;
    for _ in 0..count {
      elements.push(field_type.clone());
    }
    cursor = offset
      .checked_add(
        field_size
          .checked_mul(count)
          .ok_or_else(|| napi::Error::from_reason("native aggregate field size overflow"))?,
      )
      .ok_or_else(|| napi::Error::from_reason("native aggregate field end overflow"))?;
  }
  if cursor > size {
    return Err(napi::Error::from_reason(
      "native aggregate fields exceed declared size",
    ));
  }
  elements.extend(std::iter::repeat_with(libffi::middle::Type::u8).take(size - cursor));
  Ok(libffi::middle::Type::structure(elements))
}

fn native_ffi_field_type(typ: &serde_json::Value) -> napi::Result<(libffi::middle::Type, usize)> {
  let kind = typ
    .get("kind")
    .and_then(serde_json::Value::as_str)
    .ok_or_else(|| napi::Error::from_reason("native field type is missing `kind`"))?;
  Ok(match kind {
    "i8" => (libffi::middle::Type::i8(), 1),
    "u8" => (libffi::middle::Type::u8(), 1),
    "i16" => (libffi::middle::Type::i16(), 2),
    "u16" => (libffi::middle::Type::u16(), 2),
    "i32" => (libffi::middle::Type::i32(), 4),
    "u32" => (libffi::middle::Type::u32(), 4),
    "i64" | "isize" => (libffi::middle::Type::i64(), 8),
    "u64" | "usize" => (libffi::middle::Type::u64(), 8),
    "f32" => (libffi::middle::Type::f32(), 4),
    "f64" => (libffi::middle::Type::f64(), 8),
    "pointer" => (
      libffi::middle::Type::pointer(),
      std::mem::size_of::<usize>(),
    ),
    "guid" => {
      let mut fields = vec![
        libffi::middle::Type::u32(),
        libffi::middle::Type::u16(),
        libffi::middle::Type::u16(),
      ];
      fields.extend(std::iter::repeat_with(libffi::middle::Type::u8).take(8));
      (libffi::middle::Type::structure(fields), 16)
    }
    "struct" => {
      let layout = typ
        .get("layout")
        .ok_or_else(|| napi::Error::from_reason("nested struct is missing `layout`"))?;
      let size = layout
        .get("size")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| napi::Error::from_reason("nested struct has invalid `size`"))?;
      (native_aggregate_ffi_type(layout)?, size)
    }
    "union" => {
      return Err(napi::Error::from_reason(
        "nested unions cannot be passed by value safely",
      ));
    }
    _ => {
      return Err(napi::Error::from_reason(format!(
        "unsupported native aggregate field kind `{kind}`"
      )));
    }
  })
}

#[napi]
pub struct DynWin32Unsafe;

#[napi]
impl DynWin32Unsafe {
  #[napi]
  pub fn bind(spec: DynWin32FunctionSpec) -> napi::Result<DynWin32Function> {
    bind_function(spec)
  }

  #[napi]
  pub fn pointer(
    #[napi(ts_arg_type = "bigint | number | Buffer | Uint8Array | null | undefined")]
    value: Unknown,
  ) -> napi::Result<DynWin32Value> {
    pointer_value(com::pointer(value)?)
  }

  #[napi]
  pub fn pointer_address(value: &DynWin32Value) -> napi::Result<BigInt> {
    value.validate()?;
    match value.value {
      dynwinrt::win32::Value::Pointer(pointer) => Ok(BigInt::from(pointer as usize as u64)),
      dynwinrt::win32::Value::Null => Ok(BigInt::from(0u64)),
      _ => Err(napi::Error::from_reason(
        "DynWin32Unsafe.pointerAddress(): value is not a data pointer",
      )),
    }
  }
}

fn bind_function(spec: DynWin32FunctionSpec) -> napi::Result<DynWin32Function> {
  let mut parameter_aggregates = Vec::with_capacity(spec.parameters.len());
  let parameters = spec
    .parameters
    .into_iter()
    .map(|parameter| {
      let aggregate = parameter
        .aggregate_descriptor
        .as_deref()
        .map(native_aggregate_call_layout)
        .transpose()?;
      let typ = if aggregate.is_some() {
        dynwinrt::win32::Type::Pointer
      } else {
        parse_type(&parameter.typ)?
      };
      parameter_aggregates.push(aggregate);
      Ok(dynwinrt::win32::Parameter {
        typ,
        direction: parse_direction(&parameter.direction)?,
        nullable: parameter.nullable.unwrap_or(false),
        cleanup: parse_cleanup(parameter.cleanup.as_deref().unwrap_or("none"))?,
        consumes_resource: parameter.consumes_resource.unwrap_or(false),
        resource_cleanup: parse_cleanup(parameter.resource_cleanup.as_deref().unwrap_or("none"))?,
      })
    })
    .collect::<napi::Result<Vec<_>>>()?;
  let return_aggregate = spec
    .return_aggregate_descriptor
    .as_deref()
    .map(native_aggregate_call_layout)
    .transpose()?;
  let return_type = spec
    .return_type
    .as_deref()
    .filter(|value| !value.eq_ignore_ascii_case("void"))
    .map(parse_type)
    .transpose()?
    .filter(|_| return_aggregate.is_none());
  let plan = unsafe {
    dynwinrt::win32::CallPlan::new(dynwinrt::win32::CallPlanSpec {
      dll: spec.dll,
      entry_point: spec.entry_point,
      parameters,
      return_type,
      return_cleanup: parse_cleanup(spec.return_cleanup.as_deref().unwrap_or("none"))?,
      success_rule: parse_success_rule(spec.success_rule.as_deref().unwrap_or("always"))?,
      capture_last_error: spec.capture_last_error.unwrap_or(false),
      calling_convention: parse_calling_convention(
        spec.calling_convention.as_deref().unwrap_or("system"),
      )?,
      parameter_aggregates,
      return_aggregate,
    })
  }
  .map_err(|error| napi::Error::from_reason(error.message()))?;
  Ok(DynWin32Function(plan))
}

fn parse_type(value: &str) -> napi::Result<dynwinrt::win32::Type> {
  use dynwinrt::win32::Type;
  match value.to_ascii_lowercase().as_str() {
    "bool32" | "bool" => Ok(Type::Bool32),
    "i8" => Ok(Type::I8),
    "u8" => Ok(Type::U8),
    "i16" => Ok(Type::I16),
    "u16" | "char16" => Ok(Type::U16),
    "i32" => Ok(Type::I32),
    "u32" => Ok(Type::U32),
    "i64" => Ok(Type::I64),
    "u64" => Ok(Type::U64),
    "f32" => Ok(Type::F32),
    "f64" => Ok(Type::F64),
    "pointer" | "ptr" => Ok(Type::Pointer),
    "functionpointer" | "function_pointer" => Ok(Type::FunctionPointer),
    "handle" => Ok(Type::Handle),
    _ => Err(napi::Error::from_reason(format!(
      "Unsupported flat Win32 ABI type `{value}`"
    ))),
  }
}

fn parse_direction(value: &str) -> napi::Result<dynwinrt::win32::Direction> {
  use dynwinrt::win32::Direction;
  match value.to_ascii_lowercase().as_str() {
    "in" => Ok(Direction::In),
    "out" => Ok(Direction::Out),
    "inout" | "in_out" | "in,out" => Ok(Direction::InOut),
    _ => Err(napi::Error::from_reason(format!(
      "Unsupported flat Win32 parameter direction `{value}`"
    ))),
  }
}

fn parse_cleanup(value: &str) -> napi::Result<dynwinrt::win32::Cleanup> {
  use dynwinrt::win32::Cleanup;
  match value.to_ascii_lowercase().as_str() {
    "none" => Ok(Cleanup::None),
    "closehandle" => Ok(Cleanup::CloseHandle),
    "regclosekey" => Ok(Cleanup::RegCloseKey),
    "localfree" => Ok(Cleanup::LocalFree),
    "globalfree" => Ok(Cleanup::GlobalFree),
    "freelibrary" => Ok(Cleanup::FreeLibrary),
    "closeservicehandle" => Ok(Cleanup::CloseServiceHandle),
    "cotaskmemfree" => Ok(Cleanup::CoTaskMemFree),
    "credfree" => Ok(Cleanup::CredFree),
    _ => Err(napi::Error::from_reason(format!(
      "Unsupported flat Win32 cleanup `{value}`"
    ))),
  }
}

fn parse_success_rule(value: &str) -> napi::Result<dynwinrt::win32::SuccessRule> {
  use dynwinrt::win32::SuccessRule;
  match value.to_ascii_lowercase().as_str() {
    "always" => Ok(SuccessRule::Always),
    "zero" | "returnzero" => Ok(SuccessRule::ReturnZero),
    "nonzero" | "returnnonzero" => Ok(SuccessRule::ReturnNonZero),
    "nonnull" | "returnnonnull" => Ok(SuccessRule::ReturnNonNull),
    "hresult" | "hresultsucceeded" => Ok(SuccessRule::HResultSucceeded),
    "signednonnegative" | "nonnegative" => Ok(SuccessRule::SignedNonNegative),
    "validhandle" | "returnvalidhandle" => Ok(SuccessRule::ReturnValidHandle),
    _ => Err(napi::Error::from_reason(format!(
      "Unsupported flat Win32 success rule `{value}`"
    ))),
  }
}

fn parse_calling_convention(value: &str) -> napi::Result<dynwinrt::win32::CallingConvention> {
  use dynwinrt::win32::CallingConvention;
  match value.to_ascii_lowercase().as_str() {
    "system" | "winapi" => Ok(CallingConvention::System),
    "cdecl" | "c" => Ok(CallingConvention::Cdecl),
    _ => Err(napi::Error::from_reason(format!(
      "Unsupported flat Win32 calling convention `{value}`"
    ))),
  }
}

fn pointer_value(owner: DynWinRTValue) -> napi::Result<DynWin32Value> {
  let value = match owner.0 {
    dynwinrt::WinRTValue::RawPtr(value) => dynwinrt::win32::Value::Pointer(value),
    dynwinrt::WinRTValue::Null => dynwinrt::win32::Value::Null,
    _ => {
      return Err(napi::Error::from_reason(
        "native pointer helper did not produce a pointer value",
      ));
    }
  };
  Ok(DynWin32Value::with_pointer_owner(value, owner))
}

fn string_pointer_pointer(
  value: Unknown,
  nullable: bool,
  wide: bool,
) -> napi::Result<DynWin32Value> {
  let env = value.value().env;
  let raw = value.value().value;
  let mut value_type = napi::sys::ValueType::napi_undefined;
  unsafe { napi::sys::napi_typeof(env, raw, &mut value_type) };
  if matches!(
    value_type,
    napi::sys::ValueType::napi_null | napi::sys::ValueType::napi_undefined
  ) {
    return if nullable {
      Ok(DynWin32Value::new(dynwinrt::win32::Value::Null))
    } else {
      Err(napi::Error::from_reason(
        "string pointer slot null requires an explicitly nullable parameter",
      ))
    };
  }
  let inner = if wide {
    com::safe_wide_string_pointer(value, false)?
  } else {
    com::safe_ansi_string_pointer(value, false)?
  };
  let pointer = match &inner.0 {
    dynwinrt::WinRTValue::RawPtr(pointer) => *pointer as usize,
    _ => {
      return Err(napi::Error::from_reason(
        "string pointer helper did not produce native storage",
      ));
    }
  };
  let mut slot = Box::new(pointer);
  let slot_pointer = (&mut *slot as *mut usize).cast();
  Ok(DynWin32Value {
    value: dynwinrt::win32::Value::Pointer(slot_pointer),
    pointer_owner: Some(Win32PointerOwner::PointerSlot {
      inner: Arc::new(inner),
      slot,
    }),
  })
}

fn handle_value(value: Unknown, nullable: bool) -> napi::Result<DynWin32Value> {
  use napi::sys;

  let env = value.value().env;
  let raw = value.value().value;
  if let Ok(resource) = unsafe { <&DynWin32Resource>::from_napi_value(env, raw) } {
    if resource.0.is_closed() {
      return Err(napi::Error::from_reason(
        "Cannot use a closed flat Win32 resource",
      ));
    }
    return Ok(DynWin32Value::new(dynwinrt::win32::Value::Resource(
      Arc::clone(&resource.0),
    )));
  }

  let mut value_type = sys::ValueType::napi_undefined;
  unsafe { sys::napi_typeof(env, raw, &mut value_type) };
  if matches!(
    value_type,
    sys::ValueType::napi_null | sys::ValueType::napi_undefined
  ) {
    return if nullable {
      Ok(DynWin32Value::new(dynwinrt::win32::Value::Null))
    } else {
      Err(napi::Error::from_reason(
        "DynWin32.handle(): null requires an explicitly nullable handle parameter",
      ))
    };
  }
  let bits = if value_type == sys::ValueType::napi_bigint {
    let value = unsafe { BigInt::from_napi_value(env, raw) }?;
    let (signed, signed_lossless) = value.get_i64();
    if signed_lossless {
      signed as u64
    } else {
      let (negative, unsigned, unsigned_lossless) = value.get_u64();
      if negative || !unsigned_lossless {
        return Err(napi::Error::from_reason(
          "DynWin32.handle(): bigint must fit in signed or unsigned pointer bits",
        ));
      }
      unsigned
    }
  } else if value_type == sys::ValueType::napi_number {
    let mut number = 0.0;
    unsafe { sys::napi_get_value_double(env, raw, &mut number) };
    if !number.is_finite() || number.fract() != 0.0 || number.abs() > 9_007_199_254_740_991.0 {
      return Err(napi::Error::from_reason(
        "DynWin32.handle(): number must be a safe integer",
      ));
    }
    (number as i64) as u64
  } else {
    return Err(napi::Error::from_reason(
      "DynWin32.handle(): expected bigint, number, or DynWin32Resource",
    ));
  };
  if bits as usize as u64 != bits {
    return Err(napi::Error::from_reason(
      "DynWin32.handle(): value does not fit this process pointer width",
    ));
  }
  Ok(DynWin32Value::new(dynwinrt::win32::Value::Handle(
    bits as usize,
  )))
}
