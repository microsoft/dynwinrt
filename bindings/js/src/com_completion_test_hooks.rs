// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! ABI-correct fake export. Cross-apartment delivery uses RoGetAgileReference
//! through windows-core's marshaling wrapper, never Send COM pointers.

use dynwinrt::com::completion::{CompletionError, CompletionSignal, OneShotContract, OneShotPlan};
use napi::Env;
use napi_derive::napi;
use std::{
  cell::RefCell,
  ffi::c_void,
  ptr,
  sync::{
    atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
    Arc, LazyLock, Mutex, Weak,
  },
  thread,
  time::{Duration, Instant},
};
use windows::{
  core::{AgileReference, IUnknown, IUnknown_Vtbl, Interface as _, BOOL, GUID, HRESULT},
  Win32::{
    Foundation::HANDLE,
    Media::Audio::{
      Endpoints::{IAudioEndpointVolume, IAudioEndpointVolume_Vtbl},
      IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceAsyncOperation_Vtbl,
      IActivateAudioInterfaceCompletionHandler, IAudioClient, IAudioClient_Vtbl, AUDCLNT_SHAREMODE,
      WAVEFORMATEX,
    },
    System::{
      Com::StructuredStorage::PROPVARIANT,
      Com::{CoCreateFreeThreadedMarshaler, CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
      Threading::GetCurrentThreadId,
    },
  },
};

const E_FAIL: HRESULT = HRESULT(0x80004005u32 as i32);
const E_POINTER: HRESULT = HRESULT(0x80004003u32 as i32);
const E_NOINTERFACE: HRESULT = HRESULT(0x80004002u32 as i32);
const E_NOTIMPL: HRESULT = HRESULT(0x80004001u32 as i32);
const E_ACCESSDENIED: HRESULT = HRESULT(0x80070005u32 as i32);
const E_WRONG_THREAD: HRESULT = HRESULT(0x8001010eu32 as i32);
const IID_AGILE: GUID = GUID::from_u128(0x94ea2b94_e9cc_49e0_c0ff_ee64ca8f5b90);
const IID_MARSHAL: GUID = GUID::from_u128(0x00000003_0000_0000_c000_000000000046);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
  Mta,
  Early,
  Duplicate,
  StartFailure,
  EarlyStartFailure,
  StartNull,
  OuterFailure,
  OuterFailureWritten,
  InnerFailure,
  InnerFailureWritten,
  NullResult,
  ProjectionFailure,
  Held,
  PauseInput,
}

#[derive(Default)]
struct Stats {
  start_calls: AtomicU32,
  validated_arguments: AtomicU32,
  signal_qi_checks: AtomicU32,
  operation_created: AtomicU32,
  operation_dropped: AtomicU32,
  result_calls: AtomicU32,
  result_created: AtomicU32,
  result_dropped: AtomicU32,
  wrong_thread: AtomicU32,
  callbacks: AtomicU32,
  mta_callbacks: AtomicU32,
  callback_errors: AtomicU32,
  callback_hresult: AtomicI32,
  marshaling_errors: AtomicU32,
  callback_entered: AtomicBool,
  release_input: AtomicBool,
  inside_callback: AtomicBool,
  signals: Mutex<Vec<Weak<CompletionSignal>>>,
}

thread_local! {
  static CONFIG: RefCell<Option<(Mode, Arc<Stats>)>> = const { RefCell::new(None) };
}
static LAST_STATS: LazyLock<Mutex<Arc<Stats>>> =
  LazyLock::new(|| Mutex::new(Arc::new(Stats::default())));
static HELD: LazyLock<Mutex<Vec<CallbackJob>>> = LazyLock::new(|| Mutex::new(Vec::new()));

pub(crate) fn prepare_plan(contract: OneShotContract) -> Result<OneShotPlan, CompletionError> {
  if CONFIG.with(|config| config.borrow().is_some()) {
    unsafe { OneShotPlan::with_test_export(contract, fake_start as *const () as *mut c_void) }
  } else {
    OneShotPlan::prepare(contract)
  }
}

pub(crate) fn observe_signal(signal: Weak<CompletionSignal>) {
  CONFIG.with(|config| {
    if let Some((_, stats)) = &*config.borrow() {
      stats.signals.lock().unwrap().push(signal);
    }
  });
}

#[napi]
pub fn com_completion_test_configure(mode: Option<String>) -> napi::Result<()> {
  let Some(mode) = mode else {
    CONFIG.with(|config| config.borrow_mut().take());
    return Ok(());
  };
  let mode = match mode.as_str() {
    "mta" => Mode::Mta,
    "early" => Mode::Early,
    "duplicate" => Mode::Duplicate,
    "start-failure" => Mode::StartFailure,
    "early-start-failure" => Mode::EarlyStartFailure,
    "start-null" => Mode::StartNull,
    "outer-failure" => Mode::OuterFailure,
    "outer-failure-written" => Mode::OuterFailureWritten,
    "inner-failure" => Mode::InnerFailure,
    "inner-failure-written" => Mode::InnerFailureWritten,
    "null-result" => Mode::NullResult,
    "projection-failure" => Mode::ProjectionFailure,
    "held" => Mode::Held,
    "pause-input" => Mode::PauseInput,
    _ => {
      return Err(napi::Error::from_reason(
        "Unknown native completion fake mode",
      ))
    }
  };
  let stats = Arc::new(Stats::default());
  *LAST_STATS.lock().unwrap() = stats.clone();
  CONFIG.with(|config| *config.borrow_mut() = Some((mode, stats)));
  Ok(())
}

#[napi(object)]
pub struct ComCompletionTestStats {
  pub start_calls: u32,
  pub validated_arguments: u32,
  pub signal_qi_checks: u32,
  pub operation_created: u32,
  pub operation_dropped: u32,
  pub result_calls: u32,
  pub result_created: u32,
  pub result_dropped: u32,
  pub wrong_thread: u32,
  pub callbacks: u32,
  pub mta_callbacks: u32,
  pub callback_errors: u32,
  pub callback_hresult: i32,
  pub marshaling_errors: u32,
  pub callback_entered: bool,
  pub signals_live: u32,
  pub pending: u32,
}

#[napi]
pub fn com_completion_test_stats(env: Env) -> ComCompletionTestStats {
  let stats = LAST_STATS.lock().unwrap().clone();
  let signals_live = stats
    .signals
    .lock()
    .unwrap()
    .iter()
    .filter(|signal| signal.strong_count() != 0)
    .count() as u32;
  ComCompletionTestStats {
    start_calls: stats.start_calls.load(Ordering::SeqCst),
    validated_arguments: stats.validated_arguments.load(Ordering::SeqCst),
    signal_qi_checks: stats.signal_qi_checks.load(Ordering::SeqCst),
    operation_created: stats.operation_created.load(Ordering::SeqCst),
    operation_dropped: stats.operation_dropped.load(Ordering::SeqCst),
    result_calls: stats.result_calls.load(Ordering::SeqCst),
    result_created: stats.result_created.load(Ordering::SeqCst),
    result_dropped: stats.result_dropped.load(Ordering::SeqCst),
    wrong_thread: stats.wrong_thread.load(Ordering::SeqCst),
    callbacks: stats.callbacks.load(Ordering::SeqCst),
    mta_callbacks: stats.mta_callbacks.load(Ordering::SeqCst),
    callback_errors: stats.callback_errors.load(Ordering::SeqCst),
    callback_hresult: stats.callback_hresult.load(Ordering::SeqCst),
    marshaling_errors: stats.marshaling_errors.load(Ordering::SeqCst),
    callback_entered: stats.callback_entered.load(Ordering::SeqCst),
    signals_live,
    pending: super::com_completion::pending_count(env) as u32,
  }
}

#[napi]
pub fn com_completion_test_release() {
  LAST_STATS
    .lock()
    .unwrap()
    .release_input
    .store(true, Ordering::SeqCst);
  let jobs = std::mem::take(&mut *HELD.lock().unwrap());
  if !jobs.is_empty() {
    spawn_jobs(jobs);
  }
}

fn query(object: &IUnknown, iid: &GUID) -> windows::core::Result<IUnknown> {
  let mut value = ptr::null_mut();
  let hr = unsafe { object.query(iid, &mut value) };
  if hr.is_err() {
    if !value.is_null() {
      drop(unsafe { IUnknown::from_raw(value) });
    }
    return Err(windows::core::Error::from_hresult(hr));
  }
  if value.is_null() {
    return Err(windows::core::Error::from_hresult(E_POINTER));
  }
  Ok(unsafe { IUnknown::from_raw(value) })
}

fn probe_signal(handler: &IUnknown, stats: &Stats) -> windows::core::Result<()> {
  let identity = query(handler, &IUnknown::IID)?;
  for iid in [
    IActivateAudioInterfaceCompletionHandler::IID,
    IID_AGILE,
    IID_MARSHAL,
  ] {
    let view = query(handler, &iid)?;
    let canonical = query(&view, &IUnknown::IID)?;
    if canonical.as_raw() != identity.as_raw() {
      return Err(windows::core::Error::from_hresult(E_FAIL));
    }
  }
  let mut unsupported = ptr::null_mut();
  let hr = unsafe { handler.query(&IAudioClient::IID, &mut unsupported) };
  if hr != E_NOINTERFACE || !unsupported.is_null() {
    return Err(windows::core::Error::from_hresult(E_FAIL));
  }
  let handler: IActivateAudioInterfaceCompletionHandler = handler.cast()?;
  let null_hr = unsafe { (handler.vtable().ActivateCompleted)(handler.as_raw(), ptr::null_mut()) };
  if null_hr != E_POINTER {
    return Err(windows::core::Error::from_hresult(E_FAIL));
  }
  let wrong_interface =
    unsafe { (handler.vtable().ActivateCompleted)(handler.as_raw(), handler.as_raw()) };
  if wrong_interface != E_NOINTERFACE {
    return Err(windows::core::Error::from_hresult(E_FAIL));
  }
  stats.signal_qi_checks.fetch_add(1, Ordering::SeqCst);
  Ok(())
}

#[repr(C)]
struct FakeOperation {
  vtable: *const IActivateAudioInterfaceAsyncOperation_Vtbl,
  references: AtomicU32,
  marshaler: Option<IUnknown>,
  _handler: AgileReference<IActivateAudioInterfaceCompletionHandler>,
  owner: u32,
  target: GUID,
  mode: Mode,
  stats: Arc<Stats>,
}

impl Drop for FakeOperation {
  fn drop(&mut self) {
    self.stats.operation_dropped.fetch_add(1, Ordering::SeqCst);
  }
}

unsafe extern "system" fn operation_query(
  this: *mut c_void,
  iid: *const GUID,
  output: *mut *mut c_void,
) -> HRESULT {
  if output.is_null() {
    return E_POINTER;
  }
  unsafe {
    *output = ptr::null_mut();
  }
  if iid.is_null() {
    return E_POINTER;
  }
  let operation = unsafe { &*this.cast::<FakeOperation>() };
  let iid = unsafe { *iid };
  if iid == IID_MARSHAL {
    return operation
      .marshaler
      .as_ref()
      .map_or(E_NOINTERFACE, |marshaler| unsafe {
        marshaler.query(&iid, output)
      });
  }
  if iid != IUnknown::IID && iid != IActivateAudioInterfaceAsyncOperation::IID && iid != IID_AGILE {
    return E_NOINTERFACE;
  }
  if iid == IActivateAudioInterfaceAsyncOperation::IID
    && operation.mode == Mode::PauseInput
    && unsafe { GetCurrentThreadId() } != operation.owner
    && operation.stats.inside_callback.load(Ordering::SeqCst)
  {
    operation
      .stats
      .callback_entered
      .store(true, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(30);
    while !operation.stats.release_input.load(Ordering::SeqCst) && Instant::now() < deadline {
      thread::sleep(Duration::from_millis(1));
    }
  }
  unsafe {
    *output = this;
    operation_add_ref(this);
  }
  HRESULT(0)
}

unsafe extern "system" fn operation_add_ref(this: *mut c_void) -> u32 {
  unsafe { &*this.cast::<FakeOperation>() }
    .references
    .fetch_add(1, Ordering::SeqCst)
    + 1
}

unsafe extern "system" fn operation_release(this: *mut c_void) -> u32 {
  let count = unsafe { &*this.cast::<FakeOperation>() }
    .references
    .fetch_sub(1, Ordering::SeqCst)
    - 1;
  if count == 0 {
    drop(unsafe { Box::from_raw(this.cast::<FakeOperation>()) });
  }
  count
}

unsafe extern "system" fn operation_result(
  this: *mut c_void,
  activation: *mut HRESULT,
  output: *mut *mut c_void,
) -> HRESULT {
  let operation = unsafe { &*this.cast::<FakeOperation>() };
  operation.stats.result_calls.fetch_add(1, Ordering::SeqCst);
  if unsafe { GetCurrentThreadId() } != operation.owner {
    operation.stats.wrong_thread.fetch_add(1, Ordering::SeqCst);
    return E_WRONG_THREAD;
  }
  if activation.is_null() || output.is_null() || unsafe { !(*output).is_null() } {
    return E_POINTER;
  }
  let inner_failure = matches!(
    operation.mode,
    Mode::InnerFailure | Mode::InnerFailureWritten
  );
  unsafe {
    *activation = if inner_failure {
      E_ACCESSDENIED
    } else {
      HRESULT(0)
    };
  }
  if !matches!(
    operation.mode,
    Mode::InnerFailure | Mode::OuterFailure | Mode::NullResult
  ) {
    // Results are deliberately non-agile and are CREATED HERE on the owner
    // apartment, not retained in the agile operation or MTA worker.
    let value = new_result(
      operation.target,
      operation.owner,
      operation.mode,
      operation.stats.clone(),
    );
    unsafe {
      *output = value.into_raw();
    }
  }
  if matches!(
    operation.mode,
    Mode::OuterFailure | Mode::OuterFailureWritten
  ) {
    E_FAIL
  } else {
    HRESULT(0)
  }
}

static OPERATION_VTABLE: IActivateAudioInterfaceAsyncOperation_Vtbl =
  IActivateAudioInterfaceAsyncOperation_Vtbl {
    base__: IUnknown_Vtbl {
      QueryInterface: operation_query,
      AddRef: operation_add_ref,
      Release: operation_release,
    },
    GetActivateResult: operation_result,
  };

fn new_operation(
  handler: &IUnknown,
  target: GUID,
  mode: Mode,
  stats: Arc<Stats>,
) -> windows::core::Result<IUnknown> {
  let handler: IActivateAudioInterfaceCompletionHandler = handler.cast()?;
  let handler = AgileReference::new(&handler)?;
  let object = Box::new(FakeOperation {
    vtable: &OPERATION_VTABLE,
    references: AtomicU32::new(1),
    marshaler: None,
    _handler: handler,
    owner: unsafe { GetCurrentThreadId() },
    target,
    mode,
    stats: stats.clone(),
  });
  stats.operation_created.fetch_add(1, Ordering::SeqCst);
  let raw = Box::into_raw(object);
  let identity = unsafe { IUnknown::from_raw(raw.cast()) };
  let marshaler = unsafe { CoCreateFreeThreadedMarshaler(&identity)? };
  unsafe {
    (*raw).marshaler = Some(marshaler);
  }
  Ok(identity)
}

struct CallbackJob {
  operation: AgileReference<IActivateAudioInterfaceAsyncOperation>,
  handler: AgileReference<IActivateAudioInterfaceCompletionHandler>,
  duplicate: bool,
  stats: Arc<Stats>,
}

fn deliver(
  handler: &IActivateAudioInterfaceCompletionHandler,
  operation: &IActivateAudioInterfaceAsyncOperation,
  stats: &Stats,
  mta: bool,
) {
  stats.callbacks.fetch_add(1, Ordering::SeqCst);
  if mta {
    stats.mta_callbacks.fetch_add(1, Ordering::SeqCst);
  }
  stats.inside_callback.store(true, Ordering::SeqCst);
  let hr = unsafe { (handler.vtable().ActivateCompleted)(handler.as_raw(), operation.as_raw()) };
  stats.inside_callback.store(false, Ordering::SeqCst);
  stats.callback_hresult.store(hr.0, Ordering::SeqCst);
  if hr.is_err() {
    stats.callback_errors.fetch_add(1, Ordering::SeqCst);
  }
}

fn spawn_jobs(jobs: Vec<CallbackJob>) {
  thread::spawn(move || {
    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if initialized.is_err() {
      for job in &jobs {
        job.stats.marshaling_errors.fetch_add(1, Ordering::SeqCst);
      }
      return;
    }
    for job in jobs {
      let result = (|| -> windows::core::Result<()> {
        let operation = job.operation.resolve()?;
        let handler = job.handler.resolve()?;
        deliver(&handler, &operation, &job.stats, true);
        if job.duplicate {
          deliver(&handler, &operation, &job.stats, true);
        }
        drop(handler);
        drop(operation);
        Ok(())
      })();
      if result.is_err() {
        job.stats.marshaling_errors.fetch_add(1, Ordering::SeqCst);
      }
    }
    unsafe {
      CoUninitialize();
    }
  });
}

unsafe extern "system" fn fake_start(
  path: *const u16,
  iid: *const GUID,
  activation_params: *const PROPVARIANT,
  handler: *mut c_void,
  operation: *mut *mut c_void,
) -> HRESULT {
  std::panic::catch_unwind(|| unsafe {
    fake_start_inner(path, iid, activation_params, handler, operation)
  })
  .unwrap_or(E_FAIL)
}

unsafe fn fake_start_inner(
  path: *const u16,
  iid: *const GUID,
  activation_params: *const PROPVARIANT,
  handler: *mut c_void,
  output: *mut *mut c_void,
) -> HRESULT {
  let Some((mode, stats)) = CONFIG.with(|config| config.borrow().clone()) else {
    return E_FAIL;
  };
  stats.start_calls.fetch_add(1, Ordering::SeqCst);
  if path.is_null()
    || iid.is_null()
    || !activation_params.is_null()
    || handler.is_null()
    || output.is_null()
    || unsafe { !(*output).is_null() }
  {
    return E_POINTER;
  }
  let expected = "dynwinrt-test-render\0".encode_utf16().collect::<Vec<_>>();
  for (index, expected) in expected.iter().enumerate() {
    if unsafe { *path.add(index) } != *expected {
      return E_FAIL;
    }
  }
  let target = unsafe { *iid };
  if target != IAudioClient::IID && target != IAudioEndpointVolume::IID {
    return E_NOINTERFACE;
  }
  stats.validated_arguments.fetch_add(1, Ordering::SeqCst);
  let handler = unsafe { IUnknown::from_raw_borrowed(&handler) }.unwrap();
  let started = (|| -> windows::core::Result<()> {
    probe_signal(handler, &stats)?;
    if mode == Mode::StartNull {
      return Ok(());
    }
    let operation = new_operation(handler, target, mode, stats.clone())?;
    unsafe {
      *output = operation.clone().into_raw();
    }
    let callback: IActivateAudioInterfaceCompletionHandler = handler.cast()?;
    let operation_view: IActivateAudioInterfaceAsyncOperation = operation.cast()?;
    if matches!(mode, Mode::Early | Mode::EarlyStartFailure) {
      deliver(&callback, &operation_view, &stats, false);
      deliver(&callback, &operation_view, &stats, false);
    } else if mode != Mode::StartFailure {
      let job = CallbackJob {
        operation: AgileReference::new(&operation_view)?,
        handler: AgileReference::new(&callback)?,
        duplicate: mode == Mode::Duplicate,
        stats: stats.clone(),
      };
      if mode == Mode::Held {
        HELD.lock().unwrap().push(job);
      } else {
        spawn_jobs(vec![job]);
      }
    }
    Ok(())
  })();
  if let Err(error) = started {
    stats.marshaling_errors.fetch_add(1, Ordering::SeqCst);
    return error.code();
  }
  if matches!(mode, Mode::StartFailure | Mode::EarlyStartFailure) {
    E_ACCESSDENIED
  } else {
    HRESULT(0)
  }
}

#[repr(C)]
struct FakeResult {
  vtable: *const c_void,
  references: AtomicU32,
  target: GUID,
  owner: u32,
  supports_target: bool,
  stats: Arc<Stats>,
}

fn result_owner(value: &FakeResult) -> bool {
  if unsafe { GetCurrentThreadId() } == value.owner {
    true
  } else {
    value.stats.wrong_thread.fetch_add(1, Ordering::SeqCst);
    false
  }
}

unsafe extern "system" fn result_query(
  this: *mut c_void,
  iid: *const GUID,
  output: *mut *mut c_void,
) -> HRESULT {
  if output.is_null() {
    return E_POINTER;
  }
  unsafe {
    *output = ptr::null_mut();
  }
  if iid.is_null() {
    return E_POINTER;
  }
  let value = unsafe { &*this.cast::<FakeResult>() };
  if !result_owner(value) {
    return E_WRONG_THREAD;
  }
  if unsafe { *iid } != IUnknown::IID && !(value.supports_target && unsafe { *iid } == value.target)
  {
    return E_NOINTERFACE;
  }
  unsafe {
    *output = this;
    result_add_ref(this);
  }
  HRESULT(0)
}

unsafe extern "system" fn result_add_ref(this: *mut c_void) -> u32 {
  let value = unsafe { &*this.cast::<FakeResult>() };
  result_owner(value);
  value.references.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn result_release(this: *mut c_void) -> u32 {
  let value = unsafe { &*this.cast::<FakeResult>() };
  result_owner(value);
  let count = value.references.fetch_sub(1, Ordering::SeqCst) - 1;
  if count == 0 {
    value.stats.result_dropped.fetch_add(1, Ordering::SeqCst);
    drop(unsafe { Box::from_raw(this.cast::<FakeResult>()) });
  }
  count
}

unsafe extern "system" fn result_u32(this: *mut c_void, output: *mut u32) -> HRESULT {
  let value = unsafe { &*this.cast::<FakeResult>() };
  if !result_owner(value) {
    return E_WRONG_THREAD;
  }
  if output.is_null() {
    return E_POINTER;
  }
  unsafe {
    *output = if value.target == IAudioClient::IID {
      384
    } else {
      2
    };
  }
  HRESULT(0)
}

macro_rules! stub {
  ($name:ident($($arg:ident: $typ:ty),*)) => {
    unsafe extern "system" fn $name(_this: *mut c_void, $($arg: $typ),*) -> HRESULT {
      E_NOTIMPL
    }
  };
}
stub!(audio_initialize(_mode: AUDCLNT_SHAREMODE, _flags: u32, _duration: i64, _period: i64, _format: *const WAVEFORMATEX, _session: *const GUID));
stub!(audio_i64(_value: *mut i64));
stub!(audio_format_supported(_mode: AUDCLNT_SHAREMODE, _format: *const WAVEFORMATEX, _closest: *mut *mut WAVEFORMATEX));
stub!(audio_mix_format(_format: *mut *mut WAVEFORMATEX));
stub!(audio_period(_default: *mut i64, _minimum: *mut i64));
stub!(audio_noargs());
stub!(audio_handle(_handle: HANDLE));
stub!(audio_service(_iid: *const GUID, _result: *mut *mut c_void));
stub!(endpoint_notify(_notify: *mut c_void));
stub!(endpoint_set_f32(_value: f32, _context: *const GUID));
stub!(endpoint_get_f32(_value: *mut f32));
stub!(endpoint_set_channel(_channel: u32, _value: f32, _context: *const GUID));
stub!(endpoint_get_channel(_channel: u32, _value: *mut f32));
stub!(endpoint_set_mute(_value: BOOL, _context: *const GUID));
stub!(endpoint_get_mute(_value: *mut BOOL));
stub!(endpoint_step_info(_step: *mut u32, _count: *mut u32));
stub!(endpoint_step(_context: *const GUID));
stub!(endpoint_range(_min: *mut f32, _max: *mut f32, _step: *mut f32));

const RESULT_IDENTITY: IUnknown_Vtbl = IUnknown_Vtbl {
  QueryInterface: result_query,
  AddRef: result_add_ref,
  Release: result_release,
};
static AUDIO_VTABLE: IAudioClient_Vtbl = IAudioClient_Vtbl {
  base__: RESULT_IDENTITY,
  Initialize: audio_initialize,
  GetBufferSize: result_u32,
  GetStreamLatency: audio_i64,
  GetCurrentPadding: result_u32,
  IsFormatSupported: audio_format_supported,
  GetMixFormat: audio_mix_format,
  GetDevicePeriod: audio_period,
  Start: audio_noargs,
  Stop: audio_noargs,
  Reset: audio_noargs,
  SetEventHandle: audio_handle,
  GetService: audio_service,
};
static ENDPOINT_VTABLE: IAudioEndpointVolume_Vtbl = IAudioEndpointVolume_Vtbl {
  base__: RESULT_IDENTITY,
  RegisterControlChangeNotify: endpoint_notify,
  UnregisterControlChangeNotify: endpoint_notify,
  GetChannelCount: result_u32,
  SetMasterVolumeLevel: endpoint_set_f32,
  SetMasterVolumeLevelScalar: endpoint_set_f32,
  GetMasterVolumeLevel: endpoint_get_f32,
  GetMasterVolumeLevelScalar: endpoint_get_f32,
  SetChannelVolumeLevel: endpoint_set_channel,
  SetChannelVolumeLevelScalar: endpoint_set_channel,
  GetChannelVolumeLevel: endpoint_get_channel,
  GetChannelVolumeLevelScalar: endpoint_get_channel,
  SetMute: endpoint_set_mute,
  GetMute: endpoint_get_mute,
  GetVolumeStepInfo: endpoint_step_info,
  VolumeStepUp: endpoint_step,
  VolumeStepDown: endpoint_step,
  QueryHardwareSupport: result_u32,
  GetVolumeRange: endpoint_range,
};

fn new_result(target: GUID, owner: u32, mode: Mode, stats: Arc<Stats>) -> IUnknown {
  stats.result_created.fetch_add(1, Ordering::SeqCst);
  let value = Box::new(FakeResult {
    vtable: if target == IAudioClient::IID {
      (&AUDIO_VTABLE as *const IAudioClient_Vtbl).cast()
    } else {
      (&ENDPOINT_VTABLE as *const IAudioEndpointVolume_Vtbl).cast()
    },
    references: AtomicU32::new(1),
    target,
    owner,
    supports_target: mode != Mode::ProjectionFailure,
    stats,
  });
  unsafe { IUnknown::from_raw(Box::into_raw(value).cast()) }
}
