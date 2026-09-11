// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};

use napi::bindgen_prelude::{BigInt, Buffer, FromNapiValue, Function, ToNapiValue, Unknown};
use napi::JsValue;
use napi_derive::napi;
use windows::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::IO::{
  CancelIoEx, CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED, OVERLAPPED_0_0,
};

use super::{
  com, managed_tsfn::ManagedTsfn, win32_subsystem, DynWin32SubsystemContext, DynWinRTValue, WinGUID,
};

#[path = "win32_boundary.rs"]
pub(super) mod boundary;
#[path = "win32_spec.rs"]
mod specification;
#[path = "win32_storage.rs"]
mod storage;

const ERROR_IO_PENDING: u32 = 997;
const ERROR_OPERATION_ABORTED: u32 = 995;
const ERROR_HANDLE_EOF: u32 = 38;
const ERROR_BROKEN_PIPE: u32 = 109;
const MAX_NATIVE_AGGREGATE_DESCRIPTOR_LENGTH: usize = 1024 * 1024;
const IOCP_COMPLETION_WORKERS_MAX: usize = 4;
const IOCP_MAX_PENDING_OPERATIONS: usize = 1024;
const IOCP_MAX_OPERATION_BUFFER_BYTES: usize = 64 * 1024 * 1024;
const IOCP_MAX_PENDING_BUFFER_BYTES: usize = 256 * 1024 * 1024;

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

pub struct DynWin32Value {
  value: dynwinrt::win32::Value,
  pointer_owner: Option<Win32PointerOwner>,
}
boundary::carrier!(DynWin32Value, 1, "DynWin32Value");

enum Win32PointerOwner {
  Native(Arc<RetainedNativePointer>),
  Aggregate(Arc<NativeAggregateStorage>),
  PointerSlot {
    inner: Arc<RetainedNativePointer>,
    slot: Box<usize>,
  },
}

struct RetainedNativePointer {
  value: DynWinRTValue,
  string: Option<(bool, bool)>,
}

impl RetainedNativePointer {
  fn validate(&self) -> napi::Result<()> {
    self.value.ensure_existing_com_apartment()?;
    self.value.check_com_input_state()?;
    com::validate_pointer_owner(&self.value)?;
    if let Some((wide, multi)) = self.string {
      storage::validate_string_owner(&self.value, wide, multi)?;
    }
    Ok(())
  }
}

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
      pointer_owner: Some(Win32PointerOwner::Native(Arc::new(RetainedNativePointer {
        value: pointer_owner,
        string: None,
      }))),
    }
  }

  fn validate(&self) -> napi::Result<()> {
    if let Some(Win32PointerOwner::Native(owner)) = &self.pointer_owner {
      owner.validate()?;
    }
    if let Some(Win32PointerOwner::Aggregate(owner)) = &self.pointer_owner {
      let _ = owner.byte_length;
    }
    if let Some(Win32PointerOwner::PointerSlot { inner, slot }) = &self.pointer_owner {
      inner.validate()?;
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
  owners: BTreeMap<usize, Arc<RetainedNativePointer>>,
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
    let mut bytes = storage::zeroed(self.byte_length)?;
    unsafe {
      std::ptr::copy_nonoverlapping(
        state.words.as_ptr().cast::<u8>(),
        bytes.as_mut_ptr(),
        self.byte_length,
      );
    }
    Ok(bytes)
  }

  fn write_field(
    &self,
    offset: usize,
    bytes: &[u8],
    owner: Option<Arc<RetainedNativePointer>>,
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
    let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
    if only_after_success && state.call_succeeded != Some(true) {
      return Ok(());
    }
    let mut first_error = None;
    for field in &self.owned_fields {
      let bits = read_usize_from_words(&state.words, self.byte_length, field.offset)?;
      if bits == 0 {
        continue;
      }
      if let Err(error) = unsafe { dynwinrt::win32::cleanup_owned_resource(bits, field.cleanup) } {
        first_error.get_or_insert_with(|| napi::Error::from_reason(error.to_string()));
        continue;
      }
      write_usize_to_words(&mut state.words, self.byte_length, field.offset, 0)?;
    }
    first_error.map_or(Ok(()), Err)
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

pub struct DynWin32NativeStruct {
  descriptor: String,
  storage: Arc<NativeAggregateStorage>,
}
boundary::carrier!(DynWin32NativeStruct, 2, "DynWin32NativeStruct");

impl DynWin32NativeStruct {
  pub fn bytes(&self) -> napi::Result<Buffer> {
    Ok(Buffer::from(self.storage.bytes()?))
  }

  pub fn length(&self) -> u32 {
    self.storage.byte_length as u32
  }
}

pub struct DynWin32Resource(Arc<dynwinrt::win32::OwnedResource>);
boundary::carrier!(DynWin32Resource, 3, "DynWin32Resource");

impl DynWin32Resource {
  pub fn value(&self) -> BigInt {
    BigInt::from(self.0.raw() as u64)
  }

  pub fn closed(&self) -> bool {
    self.0.is_closed()
  }

  pub fn busy(&self) -> bool {
    self.0.has_async_leases()
  }

  pub fn active(&self) -> bool {
    self.0.has_active_async_io()
  }

  pub fn close(&self) -> napi::Result<()> {
    self
      .0
      .close()
      .map_err(|error| napi::Error::from_reason(error.to_string()))
  }
}

pub struct DynWin32Function(Arc<dynwinrt::win32::CallPlan>);
boundary::carrier!(DynWin32Function, 4, "DynWin32Function");

impl DynWin32Function {
  pub fn bind(spec: DynWin32FunctionSpec) -> napi::Result<Self> {
    bind_function(spec)
  }

  pub fn dll(&self) -> String {
    self.0.dll().to_string()
  }

  pub fn entry_point(&self) -> String {
    self.0.entry_point().to_string()
  }

  pub fn invoke(&self, args: Vec<&DynWin32Value>) -> napi::Result<DynWin32CallResult> {
    self.invoke_impl(args)
  }

  pub fn invoke_with_subsystem(
    &self,
    context: &DynWin32SubsystemContext,
    subsystem: String,
    args: Vec<&DynWin32Value>,
  ) -> napi::Result<DynWin32CallResult> {
    let _subsystem_guard = win32_subsystem::call_guard(context, &subsystem)?;
    self.invoke_impl(args)
  }
}

impl DynWin32Function {
  fn invoke_impl(&self, args: Vec<&DynWin32Value>) -> napi::Result<DynWin32CallResult> {
    let owners = args
      .iter()
      .filter_map(|value| match &value.pointer_owner {
        Some(Win32PointerOwner::Native(owner)) => Some(&owner.value),
        Some(Win32PointerOwner::PointerSlot { inner, .. }) => Some(&inner.value),
        _ => None,
      })
      .collect::<Vec<_>>();
    com::with_win32_input_leases(&owners, |validate_inputs| {
      self.invoke_validated(&args, validate_inputs)
    })
  }

  fn invoke_validated(
    &self,
    args: &[&DynWin32Value],
    validate_inputs: &dyn Fn() -> napi::Result<()>,
  ) -> napi::Result<DynWin32CallResult> {
    for value in args {
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
        owner.validate()?;
      }
    }
    let values = args
      .iter()
      .map(|value| value.value.clone())
      .collect::<Vec<_>>();
    validate_inputs()?;
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

pub struct DynWin32CallResult {
  return_value: Option<DynWin32Value>,
  outputs: Option<Vec<DynWin32Value>>,
  last_error: Option<u32>,
  succeeded: bool,
}
boundary::carrier!(DynWin32CallResult, 5, "DynWin32CallResult");

impl DynWin32CallResult {
  pub fn return_value(&mut self) -> napi::Result<Option<DynWin32Value>> {
    Ok(self.return_value.take())
  }

  pub fn outputs(&mut self) -> napi::Result<Vec<DynWin32Value>> {
    self
      .outputs
      .take()
      .ok_or_else(|| napi::Error::from_reason("Win32 outputs were already consumed"))
  }

  pub fn last_error(&self) -> Option<u32> {
    self.last_error
  }

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
  overlapped: *const OVERLAPPED,
}

struct OverlappedState {
  control: Mutex<OverlappedControl>,
  cancelled: AtomicBool,
}

// Safety: the OVERLAPPED pointer is read only while protected by `control` and
// remains pinned in the IOCP registry until `deactivate` clears it.
unsafe impl Send for OverlappedState {}
unsafe impl Sync for OverlappedState {}

impl OverlappedState {
  fn new() -> Arc<Self> {
    Arc::new(Self {
      control: Mutex::new(OverlappedControl {
        active: false,
        handle: 0,
        overlapped: std::ptr::null(),
      }),
      cancelled: AtomicBool::new(false),
    })
  }

  fn activate(&self, handle: usize, overlapped: *const OVERLAPPED) {
    let mut control = self
      .control
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    control.handle = handle;
    control.overlapped = overlapped;
    control.active = true;
  }

  fn deactivate(&self) {
    let mut control = self
      .control
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    control.active = false;
    control.handle = 0;
    control.overlapped = std::ptr::null();
  }

  fn cancel(&self) {
    self.cancelled.store(true, Ordering::Release);
    let control = self
      .control
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    if control.active && !control.overlapped.is_null() {
      let _ = unsafe {
        CancelIoEx(
          HANDLE(control.handle as *mut std::ffi::c_void),
          Some(control.overlapped),
        )
      };
    }
  }
}

pub struct OverlappedIoTask {
  kind: OverlappedIoKind,
  resource: Arc<dynwinrt::win32::OwnedResource>,
  lease: dynwinrt::win32::OwnedResourceAsyncLease,
  buffer: Option<Buffer>,
  buffer_len: usize,
  buffer_pointer: usize,
  native_buffer: Vec<u8>,
  offset: u64,
  state: Arc<OverlappedState>,
  _reservation: Option<IocpReservation>,
}

struct OverlappedCompletion {
  task: OverlappedIoTask,
  result: Result<u32, String>,
}

struct IocpOperation {
  overlapped: OVERLAPPED,
  task: OverlappedIoTask,
  completion: ManagedTsfn<OverlappedCompletion>,
}

// Safety: the operation has exclusive ownership while it is moved into the
// mutex-protected IOCP registry and moved out exactly once on completion.
unsafe impl Send for IocpOperation {}

#[derive(Default)]
struct IocpRegistry {
  operations: HashMap<usize, Box<IocpOperation>>,
}

#[derive(Default)]
struct IocpCapacity {
  operations: usize,
  buffer_bytes: usize,
}

struct IocpReservation {
  capacity: Arc<Mutex<IocpCapacity>>,
  buffer_bytes: usize,
}

impl IocpReservation {
  fn acquire(capacity: &Arc<Mutex<IocpCapacity>>, buffer_bytes: usize) -> napi::Result<Self> {
    let mut state = capacity.lock().unwrap_or_else(|error| error.into_inner());
    validate_iocp_capacity(state.operations, state.buffer_bytes, buffer_bytes)?;
    state.operations += 1;
    state.buffer_bytes += buffer_bytes;
    drop(state);
    Ok(Self {
      capacity: Arc::clone(capacity),
      buffer_bytes,
    })
  }
}

impl Drop for IocpReservation {
  fn drop(&mut self) {
    let mut state = self
      .capacity
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    state.operations = state
      .operations
      .checked_sub(1)
      .expect("IOCP operation accounting remains balanced");
    state.buffer_bytes = state
      .buffer_bytes
      .checked_sub(self.buffer_bytes)
      .expect("IOCP Buffer accounting remains balanced");
  }
}

struct IocpRuntime {
  port: usize,
  associations: Mutex<HashMap<usize, Weak<dynwinrt::win32::OwnedResource>>>,
  registry: Mutex<IocpRegistry>,
  capacity: Arc<Mutex<IocpCapacity>>,
  shutting_down: AtomicBool,
}

static IOCP_RUNTIME: LazyLock<Result<Arc<IocpRuntime>, String>> = LazyLock::new(IocpRuntime::new);

impl IocpRuntime {
  fn new() -> Result<Arc<Self>, String> {
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::{
      GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
    };
    // Shared workers can outlive the last Node environment using the addon.
    let mut module = HMODULE::default();
    unsafe {
      GetModuleHandleExW(
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
        windows::core::PCWSTR(Self::new as *const () as *const u16),
        &mut module,
      )
    }
    .map_err(|error| format!("Cannot pin the IOCP worker module: {error}"))?;
    let worker_count = std::thread::available_parallelism()
      .map(|count| count.get().min(IOCP_COMPLETION_WORKERS_MAX))
      .unwrap_or(2)
      .max(1);
    let port = unsafe {
      CreateIoCompletionPort(
        INVALID_HANDLE_VALUE,
        None,
        0,
        u32::try_from(worker_count).expect("IOCP worker count fits u32"),
      )
    }
    .map_err(|error| format!("CreateIoCompletionPort failed: {error}"))?;
    let runtime = Arc::new(Self {
      port: port.0 as usize,
      associations: Mutex::new(HashMap::new()),
      registry: Mutex::new(IocpRegistry::default()),
      capacity: Arc::new(Mutex::new(IocpCapacity::default())),
      shutting_down: AtomicBool::new(false),
    });
    for index in 0..worker_count {
      let worker = Arc::clone(&runtime);
      if let Err(error) = std::thread::Builder::new()
        .name(format!("dynwinrt-iocp-completion-{index}"))
        .spawn(move || worker.completion_loop())
      {
        runtime.shutting_down.store(true, Ordering::Release);
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(port) };
        return Err(format!("Failed to create IOCP completion worker: {error}"));
      }
    }
    Ok(runtime)
  }

  fn port(&self) -> HANDLE {
    HANDLE(self.port as *mut std::ffi::c_void)
  }

  fn associate(
    &self,
    resource: &Arc<dynwinrt::win32::OwnedResource>,
    handle: usize,
  ) -> napi::Result<()> {
    let identity = Arc::as_ptr(resource) as usize;
    let mut associations = self
      .associations
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    associations.retain(|_, resource| resource.strong_count() != 0);
    if associations
      .get(&identity)
      .and_then(Weak::upgrade)
      .is_some_and(|existing| Arc::ptr_eq(&existing, resource))
    {
      return Ok(());
    }
    associations.remove(&identity);
    let associated = unsafe {
      CreateIoCompletionPort(
        HANDLE(handle as *mut std::ffi::c_void),
        Some(self.port()),
        0,
        0,
      )
    }
    .map_err(|error| {
      napi::Error::from_reason(format!(
        "Failed to associate Win32 resource with dynwinrt IOCP: {error}"
      ))
    })?;
    if associated != self.port() {
      return Err(napi::Error::from_reason(
        "Win32 resource was associated with an unexpected IOCP",
      ));
    }
    associations.insert(identity, Arc::downgrade(resource));
    Ok(())
  }

  fn submit(
    &self,
    task: OverlappedIoTask,
    completion: ManagedTsfn<OverlappedCompletion>,
  ) -> napi::Result<()> {
    if task.state.cancelled.load(Ordering::Acquire) {
      return Err(napi::Error::from_reason("OVERLAPPED operation was aborted"));
    }
    let handle = task.lease.raw();
    if task._reservation.is_none() {
      return Err(napi::Error::from_reason(
        "OVERLAPPED operation has no capacity reservation",
      ));
    }

    let mut overlapped = OVERLAPPED::default();
    overlapped.Anonymous.Anonymous = OVERLAPPED_0_0 {
      Offset: task.offset as u32,
      OffsetHigh: (task.offset >> 32) as u32,
    };
    let mut operation = Box::new(IocpOperation {
      overlapped,
      task,
      completion,
    });
    let overlapped_ptr = &mut operation.overlapped as *mut OVERLAPPED;
    let key = overlapped_ptr as usize;

    let mut registry = self
      .registry
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    self.associate(&operation.task.resource, handle)?;
    if registry.operations.contains_key(&key) {
      return Err(napi::Error::from_reason(
        "duplicate OVERLAPPED operation address",
      ));
    }
    operation.task.state.activate(handle, overlapped_ptr);
    operation.task.lease.mark_active();
    let previous = registry.operations.insert(key, operation);
    debug_assert!(previous.is_none());

    let result = {
      let operation = registry
        .operations
        .get_mut(&key)
        .expect("IOCP operation was just registered");
      unsafe {
        match operation.task.kind {
          OverlappedIoKind::Read => ReadFile(
            HANDLE(handle as *mut std::ffi::c_void),
            Some(operation.task.native_buffer.as_mut_slice()),
            None,
            Some(overlapped_ptr),
          ),
          OverlappedIoKind::Write => WriteFile(
            HANDLE(handle as *mut std::ffi::c_void),
            Some(operation.task.native_buffer.as_slice()),
            None,
            Some(overlapped_ptr),
          ),
        }
      }
    };
    let error = result.err().map(|error| win32_error_code(&error));
    if let Some(error) = error.filter(|error| *error != ERROR_IO_PENDING) {
      let mut operation =
        remove_iocp_operation(&mut registry, key).expect("failed IOCP operation was registered");
      drop(registry);
      operation.task.state.deactivate();
      operation.task.lease.mark_inactive();
      if is_read_eof(operation.task.kind, error) {
        let _ = operation.completion.call(OverlappedCompletion {
          task: operation.task,
          result: Ok(0),
        });
        return Ok(());
      }
      return Err(native_error(
        match operation.task.kind {
          OverlappedIoKind::Read => "ReadFile",
          OverlappedIoKind::Write => "WriteFile",
        },
        error,
      ));
    }
    if registry
      .operations
      .get(&key)
      .expect("submitted IOCP operation remains registered")
      .task
      .state
      .cancelled
      .load(Ordering::Acquire)
    {
      registry
        .operations
        .get(&key)
        .expect("submitted IOCP operation remains registered")
        .task
        .state
        .cancel();
    }
    Ok(())
  }

  fn completion_loop(self: &Arc<Self>) {
    loop {
      let mut transferred = 0u32;
      let mut completion_key = 0usize;
      let mut overlapped = std::ptr::null_mut();
      let result = unsafe {
        GetQueuedCompletionStatus(
          self.port(),
          &mut transferred,
          &mut completion_key,
          &mut overlapped,
          u32::MAX,
        )
      };
      if overlapped.is_null() {
        if self.shutting_down.load(Ordering::Acquire) {
          return;
        }
        if let Err(error) = result {
          eprintln!("[dynwinrt] IOCP completion wait failed: {error}");
        }
        continue;
      }
      let error = result.err().map(|error| win32_error_code(&error));
      self.complete(overlapped, transferred, error);
    }
  }

  fn complete(&self, overlapped: *mut OVERLAPPED, transferred: u32, error: Option<u32>) {
    let mut registry = self
      .registry
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    let Some(mut operation) = remove_iocp_operation(&mut registry, overlapped as usize) else {
      eprintln!(
        "[dynwinrt] ignored completion for unknown OVERLAPPED {:p}",
        overlapped
      );
      return;
    };
    drop(registry);

    operation.task.state.deactivate();
    operation.task.lease.mark_inactive();
    let result = match error {
      Some(error) if is_read_eof(operation.task.kind, error) => Ok(0),
      Some(error) => Err(format!(
        "{} failed with Win32 error {error}",
        if error == ERROR_OPERATION_ABORTED {
          "OVERLAPPED operation"
        } else {
          "IOCP completion"
        }
      )),
      None => Ok(transferred),
    };
    let _ = operation.completion.call(OverlappedCompletion {
      task: operation.task,
      result,
    });
  }
}

fn validate_iocp_capacity(
  operation_count: usize,
  buffer_bytes: usize,
  new_buffer_bytes: usize,
) -> napi::Result<()> {
  validate_iocp_operation_buffer(new_buffer_bytes)?;
  if operation_count >= IOCP_MAX_PENDING_OPERATIONS {
    return Err(napi::Error::from_reason(format!(
      "IOCP pending operation limit ({IOCP_MAX_PENDING_OPERATIONS}) was reached"
    )));
  }
  let total = buffer_bytes
    .checked_add(new_buffer_bytes)
    .ok_or_else(|| napi::Error::from_reason("IOCP pending Buffer accounting overflow"))?;
  if total > IOCP_MAX_PENDING_BUFFER_BYTES {
    return Err(napi::Error::from_reason(format!(
      "IOCP pending native Buffer limit ({IOCP_MAX_PENDING_BUFFER_BYTES} bytes) would be exceeded"
    )));
  }
  Ok(())
}

fn validate_iocp_operation_buffer(buffer_bytes: usize) -> napi::Result<()> {
  if buffer_bytes > IOCP_MAX_OPERATION_BUFFER_BYTES {
    return Err(napi::Error::from_reason(format!(
      "IOCP operation Buffer exceeds the {IOCP_MAX_OPERATION_BUFFER_BYTES} byte limit"
    )));
  }
  Ok(())
}

fn remove_iocp_operation(registry: &mut IocpRegistry, key: usize) -> Option<Box<IocpOperation>> {
  registry.operations.remove(&key)
}

pub struct DynWin32OverlappedOperation {
  task: Option<OverlappedIoTask>,
  state: Arc<OverlappedState>,
}
boundary::carrier!(
  DynWin32OverlappedOperation,
  7,
  "DynWin32OverlappedOperation"
);

impl DynWin32OverlappedOperation {
  pub fn cancel(&self) {
    self.state.cancel();
  }

  pub fn start(&mut self, callback: Function<'static, (), ()>) -> napi::Result<()> {
    let task = self
      .task
      .take()
      .ok_or_else(|| napi::Error::from_reason("OVERLAPPED operation was already started"))?;
    let env = callback.value().env;
    let raw_callback = napi::JsValue::raw(&callback);
    let closing_state = Arc::clone(&self.state);
    let completion = ManagedTsfn::create(
      env,
      raw_callback,
      1,
      false,
      |completion: OverlappedCompletion, env| completion.into_js_arguments(env),
      Some(Box::new(move |_| closing_state.cancel())),
    )?;
    IOCP_RUNTIME
      .as_ref()
      .map_err(|error| napi::Error::from_reason(error.clone()))?
      .submit(task, completion)
  }
}

impl OverlappedIoTask {
  fn resolve(mut self, env: napi::sys::napi_env, output: u32) -> napi::Result<u32> {
    let transferred = usize::try_from(output)
      .map_err(|_| napi::Error::from_reason("OVERLAPPED result exceeds usize"))?;
    if transferred > self.buffer_len {
      return Err(napi::Error::from_reason(
        "OVERLAPPED result exceeds the original Buffer length",
      ));
    }
    if matches!(self.kind, OverlappedIoKind::Read) {
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
      let info = storage::buffer_info(env, raw).map_err(|error| {
        napi::Error::new(
          error.status,
          format!(
            "OVERLAPPED read Buffer backing ArrayBuffer was detached or changed: {}",
            error.reason
          ),
        )
      })?;
      if info.length != self.buffer_len
        || (info.length != 0 && info.pointer as usize != self.buffer_pointer)
      {
        return Err(napi::Error::from_reason(
          "OVERLAPPED read Buffer backing ArrayBuffer was detached or changed",
        ));
      }
      if transferred != 0 {
        unsafe {
          std::ptr::copy_nonoverlapping(self.native_buffer.as_ptr(), info.pointer, transferred);
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

fn is_read_eof(kind: OverlappedIoKind, error: u32) -> bool {
  matches!(kind, OverlappedIoKind::Read) && matches!(error, ERROR_HANDLE_EOF | ERROR_BROKEN_PIPE)
}

fn win32_error_code(error: &windows::core::Error) -> u32 {
  let code = error.code().0 as u32;
  if code & 0xffff_0000 == 0x8007_0000 {
    code & 0xffff
  } else {
    code
  }
}

fn native_error(function: &str, error: u32) -> napi::Error {
  napi::Error::from_reason(format!("{function} failed with Win32 error {error}"))
}

#[napi]
pub struct DynWin32;

#[napi]
impl DynWin32 {
  #[napi]
  pub fn initialize_winsock() -> napi::Result<DynWin32SubsystemContext> {
    win32_subsystem::initialize("winsock")
  }

  #[napi]
  pub fn initialize_gdi_plus() -> napi::Result<DynWin32SubsystemContext> {
    win32_subsystem::initialize("gdiplus")
  }

  #[napi]
  pub fn initialize_media_foundation() -> napi::Result<DynWin32SubsystemContext> {
    win32_subsystem::initialize("mediaFoundation")
  }

  #[napi]
  pub fn initialize_mapi_utilities() -> napi::Result<DynWin32SubsystemContext> {
    win32_subsystem::initialize("mapiUtilities")
  }

  #[napi]
  pub fn require_subsystem(
    context: &DynWin32SubsystemContext,
    #[napi(ts_arg_type = "string")] subsystem: specification::Name,
  ) -> napi::Result<()> {
    win32_subsystem::require(context, &subsystem)
  }

  #[napi]
  pub fn bool8(value: bool) -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::U8(u8::from(value)))
  }

  #[napi]
  pub fn bool32(value: bool) -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::Bool(value))
  }

  #[napi]
  pub fn i8(value: f64) -> napi::Result<DynWin32Value> {
    check_integer(value, i8::MIN as f64, i8::MAX as f64, "i8")?;
    Ok(DynWin32Value::new(dynwinrt::win32::Value::I8(value as i8)))
  }

  #[napi]
  pub fn u8(value: f64) -> napi::Result<DynWin32Value> {
    check_integer(value, 0.0, u8::MAX as f64, "u8")?;
    Ok(DynWin32Value::new(dynwinrt::win32::Value::U8(value as u8)))
  }

  #[napi]
  pub fn i16(value: f64) -> napi::Result<DynWin32Value> {
    check_integer(value, i16::MIN as f64, i16::MAX as f64, "i16")?;
    Ok(DynWin32Value::new(dynwinrt::win32::Value::I16(
      value as i16,
    )))
  }

  #[napi]
  pub fn u16(value: f64) -> napi::Result<DynWin32Value> {
    check_integer(value, 0.0, u16::MAX as f64, "u16")?;
    Ok(DynWin32Value::new(dynwinrt::win32::Value::U16(
      value as u16,
    )))
  }

  #[napi]
  pub fn i32(value: f64) -> napi::Result<DynWin32Value> {
    check_integer(value, i32::MIN as f64, i32::MAX as f64, "i32")?;
    Ok(DynWin32Value::new(dynwinrt::win32::Value::I32(
      value as i32,
    )))
  }

  #[napi]
  pub fn u32(value: f64) -> napi::Result<DynWin32Value> {
    check_integer(value, 0.0, u32::MAX as f64, "u32")?;
    Ok(DynWin32Value::new(dynwinrt::win32::Value::U32(
      value as u32,
    )))
  }

  #[napi]
  pub fn i64(#[napi(ts_arg_type = "bigint")] value: Unknown) -> napi::Result<DynWin32Value> {
    Ok(DynWin32Value::new(dynwinrt::win32::Value::I64(
      storage::signed64(&value)?,
    )))
  }

  #[napi]
  pub fn u64(#[napi(ts_arg_type = "bigint")] value: Unknown) -> napi::Result<DynWin32Value> {
    Ok(DynWin32Value::new(dynwinrt::win32::Value::U64(
      storage::unsigned64(&value)?,
    )))
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
  pub fn resource(
    value: &DynWin32Resource,
    #[napi(ts_arg_type = "string")] cleanup: specification::Name,
  ) -> napi::Result<DynWin32Value> {
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
    #[napi(ts_arg_type = "string")] iid: specification::Name,
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
    if boundary::has_carrier(env, raw)? {
      return Err(napi::Error::from_reason(
        "Win32 handles and structs are not managed COM references",
      ));
    }
    let value = boundary::managed_com_value(&value)?;
    let iid = WinGUID(
      windows::core::GUID::try_from(iid.as_str())
        .map_err(|_| napi::Error::from_reason("Invalid COM interface IID"))?,
    );
    let owner = com::with_win32_input_leases(&[value], |validate_inputs| {
      let owner = com::try_cast(value, &iid)?.ok_or_else(|| {
        napi::Error::from_reason("Managed object does not implement the required Win32 interface")
      })?;
      validate_inputs()?;
      Ok(owner)
    })?;
    let pointer = owner
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("Managed value is not a COM object"))?
      .as_raw();
    Ok(DynWin32Value {
      value: dynwinrt::win32::Value::Pointer(pointer),
      pointer_owner: Some(Win32PointerOwner::Native(Arc::new(RetainedNativePointer {
        value: owner,
        string: None,
      }))),
    })
  }

  #[napi]
  pub fn data_pointer(
    #[napi(ts_arg_type = "Buffer | Uint8Array | null | undefined")] value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    let nullable = nullable.unwrap_or(false);
    let value = pointer_value(storage::data_pointer(value, nullable)?)?;
    reject_required_null_pointer(&value, nullable)?;
    Ok(value)
  }

  #[napi]
  pub fn buffer_length(
    #[napi(ts_arg_type = "Buffer | Uint8Array")] value: Unknown,
  ) -> napi::Result<u32> {
    Ok(storage::length(&value)? as u32)
  }

  #[napi]
  pub fn byte_length(
    #[napi(ts_arg_type = "Buffer | Uint8Array")] value: Unknown,
  ) -> napi::Result<u32> {
    Self::buffer_length(value)
  }

  #[napi]
  pub fn buffer_byte_length(
    #[napi(ts_arg_type = "Buffer | Uint8Array")] value: Unknown,
  ) -> napi::Result<u32> {
    Self::buffer_length(value)
  }

  #[napi]
  pub fn allocate_buffer(length: f64) -> napi::Result<Buffer> {
    Ok(Buffer::from(storage::zeroed(storage::checked_length(
      length,
    )?)?))
  }

  #[napi]
  pub fn copy_buffer(
    #[napi(ts_arg_type = "Buffer | Uint8Array")] value: Unknown,
    actual_length: Option<f64>,
  ) -> napi::Result<Buffer> {
    storage::copy(&value, actual_length)
  }

  #[napi]
  pub fn aligned_data_pointer(
    #[napi(ts_arg_type = "Buffer | Uint8Array | null | undefined")] value: Unknown,
    alignment: f64,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    check_integer(alignment, 1.0, 8.0, "alignedDataPointer alignment")?;
    let alignment = alignment as u32;
    if !alignment.is_power_of_two() {
      return Err(napi::Error::from_reason(
        "DynWin32.alignedDataPointer(): alignment must be 1, 2, 4, or 8",
      ));
    }
    let nullable = nullable.unwrap_or(false);
    let value = pointer_value(storage::data_pointer(value, nullable)?)?;
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
    string_value(
      storage::string_pointer(value, nullable.unwrap_or(false), true, false)?,
      true,
      false,
    )
  }

  #[napi]
  pub fn ansi_string(
    #[napi(ts_arg_type = "string | Buffer | Uint8Array | null | undefined")] value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    string_value(
      storage::string_pointer(value, nullable.unwrap_or(false), false, false)?,
      false,
      false,
    )
  }

  #[napi]
  pub fn wide_multi_string(
    #[napi(ts_arg_type = "string | readonly string[] | Buffer | Uint8Array | null | undefined")]
    value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    string_value(
      storage::string_pointer(value, nullable.unwrap_or(false), true, true)?,
      true,
      true,
    )
  }

  #[napi]
  pub fn ansi_multi_string(
    #[napi(ts_arg_type = "string | readonly string[] | Buffer | Uint8Array | null | undefined")]
    value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    string_value(
      storage::string_pointer(value, nullable.unwrap_or(false), false, true)?,
      false,
      true,
    )
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
    #[napi(ts_arg_type = "Buffer")] buffer: Unknown,
    #[napi(ts_arg_type = "bigint | null")] offset: Option<Unknown>,
  ) -> napi::Result<DynWin32OverlappedOperation> {
    overlapped_io_task(
      OverlappedIoKind::Read,
      file,
      storage::native_buffer(buffer)?,
      offset
        .as_ref()
        .map(storage::unsigned64)
        .transpose()?
        .unwrap_or(0),
    )
  }

  #[napi]
  pub fn begin_write_file(
    file: &DynWin32Resource,
    #[napi(ts_arg_type = "Buffer")] buffer: Unknown,
    #[napi(ts_arg_type = "bigint | null")] offset: Option<Unknown>,
  ) -> napi::Result<DynWin32OverlappedOperation> {
    overlapped_io_task(
      OverlappedIoKind::Write,
      file,
      storage::native_buffer(buffer)?,
      offset
        .as_ref()
        .map(storage::unsigned64)
        .transpose()?
        .unwrap_or(0),
    )
  }

  #[napi]
  pub fn create_native_struct(
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
    #[napi(ts_arg_type = "Buffer | Uint8Array | null")] bytes: Option<Unknown>,
  ) -> napi::Result<DynWin32NativeStruct> {
    let (_, size, alignment, contains_pointers, owned_fields) =
      native_aggregate_layout(&descriptor)?;
    if alignment > 8 {
      return Err(napi::Error::from_reason(
        "flat Win32 native aggregate alignment above 8 is unsupported",
      ));
    }
    Ok(DynWin32NativeStruct {
      descriptor: descriptor.into_string(),
      storage: Arc::new(NativeAggregateStorage::new(
        size,
        bytes
          .as_ref()
          .map(|bytes| storage::buffer_info(bytes.value().env, bytes.raw()))
          .transpose()?
          .as_ref()
          .map(|info| unsafe { info.bytes() }),
        contains_pointers,
        owned_fields,
      )?),
    })
  }

  #[napi]
  pub fn set_native_struct_u32(
    value: &DynWin32NativeStruct,
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
    #[napi(ts_arg_type = "string")] field: specification::Name,
    input: f64,
  ) -> napi::Result<()> {
    check_integer(input, 0.0, u32::MAX as f64, "native u32 field")?;
    validate_native_struct(value, &descriptor)?;
    let (offset, kind, _) = native_aggregate_field(&descriptor, &field)?;
    if kind != "u32" {
      return Err(napi::Error::from_reason(format!(
        "native field `{field}` is not u32"
      )));
    }
    value
      .storage
      .write_field(offset, &(input as u32).to_le_bytes(), None)
  }

  #[napi]
  pub fn set_native_struct_bool32(
    value: &DynWin32NativeStruct,
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
    #[napi(ts_arg_type = "string")] field: specification::Name,
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
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
    #[napi(ts_arg_type = "string")] field: specification::Name,
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
        if owner.value.1.is_some() =>
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
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
    #[napi(ts_arg_type = "string")] field: specification::Name,
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
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
    #[napi(ts_arg_type = "string")] field: specification::Name,
    #[napi(ts_arg_type = "string")] cleanup: specification::Name,
  ) -> napi::Result<Option<DynWin32Resource>> {
    validate_native_struct(value, &descriptor)?;
    value.storage.require_success()?;
    let (offset, kind, field_cleanup) = native_aggregate_field(&descriptor, &field)?;
    if kind != "handle" || field_cleanup.as_deref() != Some(cleanup.as_str()) {
      return Err(napi::Error::from_reason(format!(
        "native field `{field}` does not have cleanup `{cleanup}`"
      )));
    }
    let cleanup = parse_cleanup(&cleanup)?;
    if cleanup == dynwinrt::win32::Cleanup::None {
      return Err(napi::Error::from_reason(
        "Native resource fields require exact ownership cleanup",
      ));
    }
    let bits = value.storage.take_usize(offset)?;
    if bits == 0 {
      return Ok(None);
    }
    let resource = unsafe { dynwinrt::win32::OwnedResource::adopt(bits, cleanup) }
      .map_err(|error| napi::Error::from_reason(error.message()))?;
    Ok(Some(DynWin32Resource(resource)))
  }

  #[napi]
  pub fn mark_native_struct_call_result(
    value: &DynWin32NativeStruct,
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
    succeeded: bool,
  ) -> napi::Result<()> {
    validate_native_struct(value, &descriptor)?;
    value.storage.mark_call_result(succeeded);
    Ok(())
  }

  #[napi]
  pub fn prepare_native_struct_call(
    value: &DynWin32NativeStruct,
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
  ) -> napi::Result<()> {
    validate_native_struct(value, &descriptor)?;
    value.storage.prepare_call()
  }

  #[napi]
  pub fn native_struct(
    #[napi(ts_arg_type = "DynWin32NativeStruct | null | undefined")] value: Unknown,
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
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
    if value.descriptor != descriptor.as_str() {
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
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
  ) -> napi::Result<DynWin32Value> {
    if value.descriptor != descriptor.as_str() {
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
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
  ) -> napi::Result<DynWin32NativeStruct> {
    let dynwinrt::win32::Value::OwnedAggregate { layout, bytes } = &value.value else {
      return Err(napi::Error::from_reason(
        "Win32 value is not an owned native aggregate",
      ));
    };
    if layout.identity() != descriptor.as_str() {
      return Err(napi::Error::from_reason(
        "native aggregate return identity mismatch",
      ));
    }
    Ok(DynWin32NativeStruct {
      descriptor: descriptor.into_string(),
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
  offset: u64,
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
  u32::try_from(buffer.len())
    .map_err(|_| napi::Error::from_reason("OVERLAPPED buffer exceeds u32"))?;
  validate_iocp_operation_buffer(buffer.len())?;
  let runtime = IOCP_RUNTIME
    .as_ref()
    .map_err(|error| napi::Error::from_reason(error.clone()))?;
  let reservation = IocpReservation::acquire(&runtime.capacity, buffer.len())?;
  let lease = file
    .0
    .async_lease(dynwinrt::win32::Cleanup::CloseHandle)
    .map_err(|error| napi::Error::from_reason(error.message()))?;
  let state = OverlappedState::new();
  Ok(DynWin32OverlappedOperation {
    task: Some(OverlappedIoTask {
      kind,
      resource: Arc::clone(&file.0),
      lease,
      native_buffer: try_copy_io_buffer(kind, &buffer)?,
      buffer_len: buffer.len(),
      buffer_pointer: buffer.as_ptr() as usize,
      buffer: Some(buffer),
      offset,
      state: Arc::clone(&state),
      _reservation: Some(reservation),
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

fn native_layout_extent(layout: &serde_json::Value) -> napi::Result<(usize, usize)> {
  let size = layout
    .get("size")
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .filter(|value| *value > 0 && *value <= dynwinrt::win32::MAX_NATIVE_AGGREGATE_SIZE)
    .ok_or_else(|| napi::Error::from_reason("Native aggregate size exceeds its safety limit"))?;
  let alignment = layout
    .get("alignment")
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .filter(|value| value.is_power_of_two() && *value <= 8 && size % *value == 0)
    .ok_or_else(|| napi::Error::from_reason("Native aggregate alignment is invalid"))?;
  Ok((size, alignment))
}

fn validate_native_layout(layout: &serde_json::Value, union: bool) -> napi::Result<()> {
  let (size, _) = native_layout_extent(layout)?;
  let fields = layout
    .get("fields")
    .and_then(serde_json::Value::as_array)
    .ok_or_else(|| napi::Error::from_reason("Native aggregate is missing fields"))?;
  let mut ranges = Vec::new();
  ranges
    .try_reserve_exact(fields.len())
    .map_err(|_| napi::Error::from_reason("Unable to allocate native layout validation"))?;
  let mut names = std::collections::HashSet::new();
  names
    .try_reserve(fields.len())
    .map_err(|_| napi::Error::from_reason("Unable to allocate native field identities"))?;
  for field in fields {
    let name = field
      .get("name")
      .and_then(serde_json::Value::as_str)
      .filter(|name| !name.is_empty())
      .ok_or_else(|| napi::Error::from_reason("Native field requires an exact name"))?;
    if !names.insert(name) {
      return Err(napi::Error::from_reason("Duplicate native field name"));
    }
    let offset = field
      .get("offset")
      .and_then(serde_json::Value::as_u64)
      .and_then(|value| usize::try_from(value).ok())
      .ok_or_else(|| napi::Error::from_reason("Native field offset is invalid"))?;
    let count = field
      .get("count")
      .and_then(serde_json::Value::as_u64)
      .and_then(|value| usize::try_from(value).ok())
      .filter(|value| *value > 0)
      .ok_or_else(|| napi::Error::from_reason("Native field count is invalid"))?;
    let typ = field
      .get("type")
      .ok_or_else(|| napi::Error::from_reason("Native field type is missing"))?;
    let kind = typ
      .get("kind")
      .and_then(serde_json::Value::as_str)
      .ok_or_else(|| napi::Error::from_reason("Native field kind is missing"))?;
    let field_size = match kind {
      "i8" | "u8" => 1,
      "i16" | "u16" => 2,
      "i32" | "u32" | "f32" => 4,
      "i64" | "u64" | "f64" => 8,
      "isize" | "usize" => std::mem::size_of::<usize>(),
      "guid" => 16,
      "pointer" | "handle" => {
        if count != 1 || union {
          return Err(napi::Error::from_reason(
            "Native pointer/resource arrays and unions require ownership contracts",
          ));
        }
        if kind == "handle" {
          parse_cleanup(
            typ
              .get("cleanup")
              .and_then(serde_json::Value::as_str)
              .ok_or_else(|| {
                napi::Error::from_reason("Native handle field has no cleanup contract")
              })?,
          )?;
        }
        std::mem::size_of::<usize>()
      }
      "struct" | "union" => {
        let nested = typ
          .get("layout")
          .ok_or_else(|| napi::Error::from_reason("Nested native field layout is missing"))?;
        validate_native_layout(nested, kind == "union")?;
        if native_layout_contains_pointers(nested)? {
          return Err(napi::Error::from_reason(
            "Nested pointer ownership is not modeled",
          ));
        }
        native_layout_extent(nested)?.0
      }
      _ => {
        return Err(napi::Error::from_reason(format!(
          "Unsupported native field kind `{kind}`"
        )))
      }
    };
    let end = field_size
      .checked_mul(count)
      .and_then(|length| offset.checked_add(length))
      .filter(|end| *end <= size)
      .ok_or_else(|| napi::Error::from_reason("Native field exceeds aggregate size"))?;
    if union && offset != 0 {
      return Err(napi::Error::from_reason(
        "Native union fields must overlap at offset zero",
      ));
    }
    ranges.push((offset, end));
  }
  if !union {
    ranges.sort_unstable();
    if ranges.windows(2).any(|fields| fields[0].1 > fields[1].0) {
      return Err(napi::Error::from_reason("Native struct fields overlap"));
    }
  }
  Ok(())
}

fn reserve_ffi_elements(
  elements: &mut Vec<libffi::middle::Type>,
  count: usize,
) -> napi::Result<()> {
  const MAX_ELEMENTS: usize =
    dynwinrt::win32::MAX_NATIVE_AGGREGATE_SIZE / std::mem::size_of::<libffi::middle::Type>();
  if elements
    .len()
    .checked_add(count)
    .is_none_or(|length| length > MAX_ELEMENTS)
  {
    return Err(napi::Error::from_reason(
      "Native aggregate libffi type graph exceeds its safety limit",
    ));
  }
  elements
    .try_reserve_exact(count)
    .map_err(|_| napi::Error::from_reason("Unable to allocate native aggregate libffi fields"))
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
  let union = match root.get("kind").and_then(serde_json::Value::as_str) {
    Some("struct") => false,
    Some("union") => true,
    _ => {
      return Err(napi::Error::from_reason(
        "Native aggregate requires struct or union kind",
      ))
    }
  };
  validate_native_layout(layout, union)?;
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
    native_layout_contains_pointers(layout)?,
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

fn native_layout_contains_pointers(layout: &serde_json::Value) -> napi::Result<bool> {
  let mut contains = false;
  for field in layout
    .get("fields")
    .and_then(serde_json::Value::as_array)
    .ok_or_else(|| napi::Error::from_reason("Native aggregate is missing fields"))?
  {
    let typ = field
      .get("type")
      .ok_or_else(|| napi::Error::from_reason("Native field is missing type"))?;
    let kind = typ
      .get("kind")
      .and_then(serde_json::Value::as_str)
      .ok_or_else(|| napi::Error::from_reason("Native field is missing kind"))?;
    contains |= match kind {
      "pointer" | "handle" => true,
      "struct" | "union" => native_layout_contains_pointers(
        typ
          .get("layout")
          .ok_or_else(|| napi::Error::from_reason("Nested native layout is missing"))?,
      )?,
      "i8" | "u8" | "i16" | "u16" | "i32" | "u32" | "i64" | "u64" | "isize" | "usize" | "f32"
      | "f64" | "guid" => false,
      _ => {
        return Err(napi::Error::from_reason(format!(
          "Unsupported native field kind `{kind}`"
        )))
      }
    };
  }
  Ok(contains)
}

fn native_aggregate_call_layout(
  descriptor: &str,
) -> napi::Result<Arc<dynwinrt::win32::NativeAggregateLayout>> {
  let root = parse_native_aggregate_descriptor(descriptor)?;
  if root.get("kind").and_then(serde_json::Value::as_str) != Some("struct") {
    return Err(napi::Error::from_reason(
      "Only plain native structs can be passed by value",
    ));
  }
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
  validate_native_layout(layout, false)?;
  if native_layout_contains_pointers(layout)? {
    return Err(napi::Error::from_reason(
      "Pointer-bearing native aggregates cannot be passed by value",
    ));
  }
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
  validate_native_layout(layout, false)?;
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
    if offset > size {
      return Err(napi::Error::from_reason(
        "Native field offset exceeds aggregate size",
      ));
    }
    reserve_ffi_elements(&mut elements, offset - cursor)?;
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
    let end = offset
      .checked_add(
        field_size
          .checked_mul(count)
          .ok_or_else(|| napi::Error::from_reason("native aggregate field size overflow"))?,
      )
      .ok_or_else(|| napi::Error::from_reason("native aggregate field end overflow"))?;
    if end > size {
      return Err(napi::Error::from_reason(
        "Native field exceeds aggregate size",
      ));
    }
    reserve_ffi_elements(&mut elements, count)?;
    for _ in 0..count {
      elements.push(field_type.clone());
    }
    cursor = end;
  }
  if cursor > size {
    return Err(napi::Error::from_reason(
      "native aggregate fields exceed declared size",
    ));
  }
  if elements.is_empty() {
    return Err(napi::Error::from_reason(
      "By-value native aggregates require exact field ABI types",
    ));
  }
  reserve_ffi_elements(&mut elements, size - cursor)?;
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
    "i64" => (libffi::middle::Type::i64(), 8),
    "u64" => (libffi::middle::Type::u64(), 8),
    "isize" if std::mem::size_of::<usize>() == 4 => (libffi::middle::Type::i32(), 4),
    "usize" if std::mem::size_of::<usize>() == 4 => (libffi::middle::Type::u32(), 4),
    "isize" => (libffi::middle::Type::i64(), 8),
    "usize" => (libffi::middle::Type::u64(), 8),
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
  pub fn bind(
    #[napi(ts_arg_type = "DynWin32FunctionSpec")] spec: Unknown,
  ) -> napi::Result<DynWin32Function> {
    bind_function(specification::function_spec(spec)?)
  }

  #[napi]
  pub fn pointer(
    #[napi(ts_arg_type = "bigint | number | Buffer | Uint8Array | null | undefined")]
    value: Unknown,
  ) -> napi::Result<DynWin32Value> {
    pointer_value(storage::unsafe_pointer(value)?)
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

fn string_value(owner: DynWinRTValue, wide: bool, multi: bool) -> napi::Result<DynWin32Value> {
  let mut value = pointer_value(owner)?;
  if let Some(Win32PointerOwner::Native(owner)) = &mut value.pointer_owner {
    Arc::get_mut(owner)
      .expect("new string owner is unique")
      .string = Some((wide, multi));
  }
  Ok(value)
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
  let inner = storage::string_pointer(value, false, wide, false)?;
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
      inner: Arc::new(RetainedNativePointer {
        value: inner,
        string: Some((wide, false)),
      }),
      slot,
    }),
  })
}

fn check_integer(value: f64, minimum: f64, maximum: f64, name: &str) -> napi::Result<()> {
  if !value.is_finite() || value.fract() != 0.0 || !(minimum..=maximum).contains(&value) {
    return Err(napi::Error::from_reason(format!(
      "DynWin32.{name}(): value is outside the exact integer range"
    )));
  }
  Ok(())
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
    if let Ok(signed) = storage::signed64(&value) {
      if signed as isize as i64 != signed {
        return Err(napi::Error::from_reason(
          "DynWin32.handle(): value does not fit this process pointer width",
        ));
      }
      signed as isize as usize as u64
    } else {
      storage::unsigned64(&value)?
    }
  } else if value_type == sys::ValueType::napi_number {
    let mut number = 0.0;
    unsafe { sys::napi_get_value_double(env, raw, &mut number) };
    if !number.is_finite() || number.fract() != 0.0 || number.abs() > 9_007_199_254_740_991.0 {
      return Err(napi::Error::from_reason(
        "DynWin32.handle(): number must be a safe integer",
      ));
    }
    if number < 0.0 {
      let signed = number as i64;
      if signed as isize as i64 != signed {
        return Err(napi::Error::from_reason(
          "DynWin32.handle(): value does not fit this process pointer width",
        ));
      }
      signed as isize as usize as u64
    } else {
      number as u64
    }
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

#[cfg(test)]
#[path = "win32_outcome_tests.rs"]
mod outcome_tests;

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn iocp_capacity_bounds_operations_and_native_buffers() {
    validate_iocp_capacity(IOCP_MAX_PENDING_OPERATIONS - 1, 0, 1).unwrap();
    assert!(validate_iocp_capacity(IOCP_MAX_PENDING_OPERATIONS, 0, 1)
      .unwrap_err()
      .reason
      .contains("operation limit"));

    validate_iocp_capacity(0, IOCP_MAX_PENDING_BUFFER_BYTES - 1, 1).unwrap();
    assert!(validate_iocp_capacity(0, IOCP_MAX_PENDING_BUFFER_BYTES, 1)
      .unwrap_err()
      .reason
      .contains("Buffer limit"));
    assert!(
      validate_iocp_operation_buffer(IOCP_MAX_OPERATION_BUFFER_BYTES + 1)
        .unwrap_err()
        .reason
        .contains("operation Buffer")
    );
    assert!(validate_iocp_capacity(0, usize::MAX, 1)
      .unwrap_err()
      .reason
      .contains("accounting overflow"));

    let capacity = Arc::new(Mutex::new(IocpCapacity::default()));
    let reservation = IocpReservation::acquire(&capacity, 16).unwrap();
    {
      let state = capacity.lock().unwrap();
      assert_eq!((state.operations, state.buffer_bytes), (1, 16));
    }
    drop(reservation);
    let state = capacity.lock().unwrap();
    assert_eq!((state.operations, state.buffer_bytes), (0, 0));
  }

  #[test]
  fn hresult_from_win32_is_decoded_for_iocp_errors() {
    let error = windows::core::Error::from_hresult(windows::core::HRESULT(
      0x8007_0000u32.wrapping_add(ERROR_IO_PENDING) as i32,
    ));
    assert_eq!(win32_error_code(&error), ERROR_IO_PENDING);
  }
}
