// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
  ffi::c_void,
  sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
  },
  thread,
  time::{Duration, Instant},
};

use napi::{
  bindgen_prelude::{Function, ToNapiValue},
  Env, JsValue, Status,
};
use napi_derive::napi;
use windows::core::{IUnknown, Interface};

use crate::{
  managed_tsfn::{self, ManagedTsfn},
  DynWinRTValue, DynWinRtDelegate,
};

static PRODUCED: AtomicUsize = AtomicUsize::new(0);
static DROPPED: AtomicUsize = AtomicUsize::new(0);
static ACCEPTED: AtomicUsize = AtomicUsize::new(0);
static QUEUE_FULL: AtomicUsize = AtomicUsize::new(0);
static CLOSING: AtomicUsize = AtomicUsize::new(0);
static OTHER_FAILURE: AtomicUsize = AtomicUsize::new(0);

static HELD_STRONG: Mutex<Option<ManagedTsfn<TestPayload>>> = Mutex::new(None);
static HELD_WEAK: Mutex<Option<ManagedTsfn<TestPayload>>> = Mutex::new(None);
static RETAINED_DELEGATE: AtomicUsize = AtomicUsize::new(0);
static RETAINED_COM_SINK: AtomicUsize = AtomicUsize::new(0);
static DELEGATE_STRESS_DONE: AtomicUsize = AtomicUsize::new(0);
static DELEGATE_STRESS_SUCCEEDED: AtomicUsize = AtomicUsize::new(0);
static DELEGATE_STRESS_FAILED: AtomicUsize = AtomicUsize::new(0);

struct TestPayload {
  id: u32,
}

impl Drop for TestPayload {
  fn drop(&mut self) {
    DROPPED.fetch_add(1, Ordering::SeqCst);
  }
}

#[napi(object)]
pub struct TsfnTestStats {
  pub produced: u32,
  pub dropped: u32,
  pub accepted: u32,
  pub queue_full: u32,
  pub closing: u32,
  pub other_failure: u32,
}

#[napi(object)]
pub struct TsfnDelegateInvokeStats {
  pub succeeded: u32,
  pub failed: u32,
}

#[napi(object)]
pub struct TsfnComSinkInvokeResult {
  pub hresult: i32,
  pub output: i32,
}

#[napi(object)]
pub struct TsfnComAllocatedBufferResult {
  pub hresult: i32,
  pub count: u32,
  pub byte_sum: u32,
}

fn count(value: &AtomicUsize) -> u32 {
  value.load(Ordering::SeqCst).min(u32::MAX as usize) as u32
}

fn record_status(status: Status) {
  match status {
    Status::Ok => &ACCEPTED,
    Status::QueueFull => &QUEUE_FULL,
    Status::Closing => &CLOSING,
    _ => &OTHER_FAILURE,
  }
  .fetch_add(1, Ordering::SeqCst);
}

fn build_tsfn(callback: Function<'static, f64, ()>) -> napi::Result<ManagedTsfn<TestPayload>> {
  build_tsfn_with_options(callback, 0, false)
}

fn build_tsfn_with_options(
  callback: Function<'static, f64, ()>,
  max_queue_size: usize,
  weak: bool,
) -> napi::Result<ManagedTsfn<TestPayload>> {
  let env = callback.value().env;
  let raw = napi::JsValue::raw(&callback);
  ManagedTsfn::create(
    env,
    raw,
    max_queue_size,
    weak,
    |value: TestPayload, env| {
      unsafe { f64::to_napi_value(env, f64::from(value.id)) }.map(|value| vec![value])
    },
    None,
  )
}

fn spawn_producer(tsfn: ManagedTsfn<TestPayload>, count: u32, delay_ms: u32) {
  thread::spawn(move || {
    if delay_ms != 0 {
      thread::sleep(Duration::from_millis(u64::from(delay_ms)));
    }
    for id in 0..count {
      PRODUCED.fetch_add(1, Ordering::SeqCst);
      record_status(tsfn.call(TestPayload { id }));
    }
  });
}

#[napi]
pub fn tsfn_test_reset() {
  for counter in [
    &PRODUCED,
    &DROPPED,
    &ACCEPTED,
    &QUEUE_FULL,
    &CLOSING,
    &OTHER_FAILURE,
  ] {
    counter.store(0, Ordering::SeqCst);
  }
}

#[napi]
pub fn tsfn_test_stats() -> TsfnTestStats {
  TsfnTestStats {
    produced: count(&PRODUCED),
    dropped: count(&DROPPED),
    accepted: count(&ACCEPTED),
    queue_full: count(&QUEUE_FULL),
    closing: count(&CLOSING),
    other_failure: count(&OTHER_FAILURE),
  }
}

#[napi]
pub fn tsfn_test_start_unbounded(
  callback: Function<'static, f64, ()>,
  count: u32,
  delay_ms: u32,
) -> napi::Result<()> {
  spawn_producer(
    build_tsfn_with_options(callback, 0, false)?,
    count,
    delay_ms,
  );
  Ok(())
}

#[napi]
pub fn tsfn_test_start_bounded(
  callback: Function<'static, f64, ()>,
  count: u32,
  delay_ms: u32,
) -> napi::Result<()> {
  spawn_producer(
    build_tsfn_with_options(callback, 1, false)?,
    count,
    delay_ms,
  );
  Ok(())
}

#[napi]
pub fn tsfn_test_hold_strong(callback: Function<'static, f64, ()>) -> napi::Result<()> {
  *HELD_STRONG
    .lock()
    .map_err(|_| napi::Error::from_reason("strong TSFN test lock is poisoned"))? =
    Some(build_tsfn(callback)?);
  Ok(())
}

#[napi]
pub fn tsfn_test_hold_weak(callback: Function<'static, f64, ()>) -> napi::Result<()> {
  *HELD_WEAK
    .lock()
    .map_err(|_| napi::Error::from_reason("weak TSFN test lock is poisoned"))? =
    Some(build_tsfn_with_options(callback, 0, true)?);
  Ok(())
}

#[napi]
pub fn tsfn_test_release_held() {
  HELD_STRONG
    .lock()
    .unwrap_or_else(|error| error.into_inner())
    .take();
  HELD_WEAK
    .lock()
    .unwrap_or_else(|error| error.into_inner())
    .take();
}

#[napi]
pub fn tsfn_test_registered_handle_count(env: Env) -> u32 {
  managed_tsfn::test_registered_handle_count(env.raw()).min(u32::MAX as usize) as u32
}

#[napi]
pub fn tsfn_test_retain_delegate(delegate: &DynWinRtDelegate) -> napi::Result<()> {
  let object = delegate
    .0
    .as_object()
    .ok_or_else(|| napi::Error::from_reason("TSFN test delegate is not a COM object"))?
    .clone();
  let raw = object.into_raw() as usize;
  let previous = RETAINED_DELEGATE.swap(raw, Ordering::SeqCst);
  if previous != 0 {
    unsafe { drop(IUnknown::from_raw(previous as *mut c_void)) };
  }
  Ok(())
}

unsafe fn invoke_delegate(raw: *mut c_void) -> i32 {
  let vtable = unsafe { *(raw as *const *const *const c_void) };
  let invoke: unsafe extern "system" fn(*mut c_void) -> windows::core::HRESULT =
    unsafe { std::mem::transmute(*vtable.add(3)) };
  unsafe { invoke(raw) }.0
}

#[napi]
pub fn tsfn_test_invoke_retained_delegate() -> napi::Result<i32> {
  let raw = RETAINED_DELEGATE.load(Ordering::SeqCst) as *mut c_void;
  if raw.is_null() {
    return Err(napi::Error::from_reason(
      "No delegate is retained by the TSFN test harness",
    ));
  }
  Ok(unsafe { invoke_delegate(raw) })
}

#[napi]
pub fn tsfn_test_invoke_retained_delegate_on_thread() -> napi::Result<i32> {
  let raw = RETAINED_DELEGATE.load(Ordering::SeqCst);
  if raw == 0 {
    return Err(napi::Error::from_reason(
      "No delegate is retained by the TSFN test harness",
    ));
  }
  thread::spawn(move || unsafe { invoke_delegate(raw as *mut c_void) })
    .join()
    .map_err(|_| napi::Error::from_reason("TSFN delegate test thread panicked"))
}

#[napi]
pub fn tsfn_test_invoke_retained_delegate_on_thread_many(
  count: u32,
) -> napi::Result<TsfnDelegateInvokeStats> {
  let raw = RETAINED_DELEGATE.load(Ordering::SeqCst);
  if raw == 0 {
    return Err(napi::Error::from_reason(
      "No delegate is retained by the TSFN test harness",
    ));
  }
  thread::spawn(move || {
    let mut succeeded = 0;
    let mut failed = 0;
    for _ in 0..count {
      if unsafe { invoke_delegate(raw as *mut c_void) } == 0 {
        succeeded += 1;
      } else {
        failed += 1;
      }
    }
    TsfnDelegateInvokeStats { succeeded, failed }
  })
  .join()
  .map_err(|_| napi::Error::from_reason("TSFN delegate test thread panicked"))
}

#[napi]
pub fn tsfn_test_start_retained_delegate_stress(count: u32) -> napi::Result<()> {
  let raw = RETAINED_DELEGATE.load(Ordering::SeqCst);
  if raw == 0 {
    return Err(napi::Error::from_reason(
      "No delegate is retained by the TSFN test harness",
    ));
  }
  DELEGATE_STRESS_DONE.store(0, Ordering::SeqCst);
  DELEGATE_STRESS_SUCCEEDED.store(0, Ordering::SeqCst);
  DELEGATE_STRESS_FAILED.store(0, Ordering::SeqCst);
  thread::spawn(move || {
    for _ in 0..count {
      if unsafe { invoke_delegate(raw as *mut c_void) } == 0 {
        DELEGATE_STRESS_SUCCEEDED.fetch_add(1, Ordering::SeqCst);
      } else {
        DELEGATE_STRESS_FAILED.fetch_add(1, Ordering::SeqCst);
      }
      thread::yield_now();
    }
    DELEGATE_STRESS_DONE.store(1, Ordering::SeqCst);
  });
  Ok(())
}

#[napi]
pub fn tsfn_test_wait_retained_delegate_stress(
  timeout_ms: u32,
) -> napi::Result<TsfnDelegateInvokeStats> {
  let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
  while DELEGATE_STRESS_DONE.load(Ordering::SeqCst) == 0 {
    if Instant::now() >= deadline {
      return Err(napi::Error::from_reason(
        "Timed out waiting for the retained delegate stress thread",
      ));
    }
    thread::sleep(Duration::from_millis(1));
  }
  Ok(TsfnDelegateInvokeStats {
    succeeded: count(&DELEGATE_STRESS_SUCCEEDED),
    failed: count(&DELEGATE_STRESS_FAILED),
  })
}

#[napi]
pub fn tsfn_test_release_retained_delegate() {
  let raw = RETAINED_DELEGATE.swap(0, Ordering::SeqCst);
  if raw != 0 {
    unsafe { drop(IUnknown::from_raw(raw as *mut c_void)) };
  }
}

#[napi]
pub fn tsfn_test_retain_com_sink(value: &DynWinRTValue) -> napi::Result<()> {
  let object = value
    .0
    .as_object()
    .ok_or_else(|| napi::Error::from_reason("COM sink test value is not a COM object"))?
    .clone();
  let raw = object.into_raw() as usize;
  let previous = RETAINED_COM_SINK.swap(raw, Ordering::SeqCst);
  if previous != 0 {
    unsafe { drop(IUnknown::from_raw(previous as *mut c_void)) };
  }
  Ok(())
}

unsafe fn invoke_com_sink(raw: *mut c_void) -> i32 {
  let vtable = unsafe { *(raw as *const *const *const c_void) };
  let invoke: unsafe extern "system" fn(*mut c_void, *mut c_void) -> windows::core::HRESULT =
    unsafe { std::mem::transmute(*vtable.add(3)) };
  unsafe { invoke(raw, std::ptr::null_mut()) }.0
}

unsafe fn invoke_com_sink_out(raw: *mut c_void) -> TsfnComSinkInvokeResult {
  let vtable = unsafe { *(raw as *const *const *const c_void) };
  let invoke: unsafe extern "system" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut i32,
  ) -> windows::core::HRESULT = unsafe { std::mem::transmute(*vtable.add(3)) };
  let mut output = -1;
  let hresult = unsafe { invoke(raw, std::ptr::null_mut(), std::ptr::null_mut(), &mut output) };
  TsfnComSinkInvokeResult {
    hresult: hresult.0,
    output,
  }
}

unsafe fn invoke_com_sink_i32(raw: *mut c_void, value: i32) -> i32 {
  let vtable = unsafe { *(raw as *const *const *const c_void) };
  let invoke: unsafe extern "system" fn(*mut c_void, i32) -> windows::core::HRESULT =
    unsafe { std::mem::transmute(*vtable.add(3)) };
  unsafe { invoke(raw, value) }.0
}

unsafe fn invoke_com_sink_direct_i32(raw: *mut c_void, value: i32) -> i32 {
  let vtable = unsafe { *(raw as *const *const *const c_void) };
  let invoke: unsafe extern "system" fn(*mut c_void, i32) -> i32 =
    unsafe { std::mem::transmute(*vtable.add(3)) };
  unsafe { invoke(raw, value) }
}

unsafe fn invoke_com_sink_void_i32(raw: *mut c_void, value: i32) {
  let vtable = unsafe { *(raw as *const *const *const c_void) };
  let invoke: unsafe extern "system" fn(*mut c_void, i32) =
    unsafe { std::mem::transmute(*vtable.add(3)) };
  unsafe { invoke(raw, value) };
}

unsafe fn invoke_com_sink_guid(raw: *mut c_void, value: &windows::core::GUID) -> i32 {
  let vtable = unsafe { *(raw as *const *const *const c_void) };
  let invoke: unsafe extern "system" fn(
    *mut c_void,
    *const windows::core::GUID,
  ) -> windows::core::HRESULT = unsafe { std::mem::transmute(*vtable.add(3)) };
  unsafe { invoke(raw, value) }.0
}

unsafe fn invoke_com_sink_bstr(raw: *mut c_void) -> i32 {
  let vtable = unsafe { *(raw as *const *const *const c_void) };
  let invoke: unsafe extern "system" fn(*mut c_void, *const u16) -> windows::core::HRESULT =
    unsafe { std::mem::transmute(*vtable.add(3)) };
  let value = windows::core::BSTR::from("embedded\0callback");
  unsafe { invoke(raw, value.as_ptr()) }.0
}

unsafe fn invoke_com_sink_wide_string(raw: *mut c_void) -> i32 {
  let vtable = unsafe { *(raw as *const *const *const c_void) };
  let invoke: unsafe extern "system" fn(*mut c_void, *const u16) -> windows::core::HRESULT =
    unsafe { std::mem::transmute(*vtable.add(3)) };
  let value = "wide callback\0".encode_utf16().collect::<Vec<_>>();
  unsafe { invoke(raw, value.as_ptr()) }.0
}

unsafe fn invoke_com_sink_ansi_string(raw: *mut c_void) -> i32 {
  let vtable = unsafe { *(raw as *const *const *const c_void) };
  let invoke: unsafe extern "system" fn(*mut c_void, *const i8) -> windows::core::HRESULT =
    unsafe { std::mem::transmute(*vtable.add(3)) };
  unsafe { invoke(raw, c"ansi callback".as_ptr()) }.0
}

unsafe fn invoke_com_sink_allocated_buffer(raw: *mut c_void) -> TsfnComAllocatedBufferResult {
  let vtable = unsafe { *(raw as *const *const *const c_void) };
  let invoke: unsafe extern "system" fn(
    *mut c_void,
    *mut *mut u8,
    *mut u32,
  ) -> windows::core::HRESULT = unsafe { std::mem::transmute(*vtable.add(3)) };
  let mut bytes = std::ptr::null_mut();
  let mut count = u32::MAX;
  let hresult = unsafe { invoke(raw, &mut bytes, &mut count) };
  let byte_sum = if bytes.is_null() || count == 0 {
    0
  } else {
    unsafe { std::slice::from_raw_parts(bytes, count as usize) }
      .iter()
      .map(|value| u32::from(*value))
      .sum()
  };
  if !bytes.is_null() {
    unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(bytes.cast())) };
  }
  TsfnComAllocatedBufferResult {
    hresult: hresult.0,
    count,
    byte_sum,
  }
}

unsafe fn invoke_com_sink_caller_buffer(raw: *mut c_void) -> TsfnComAllocatedBufferResult {
  let vtable = unsafe { *(raw as *const *const *const c_void) };
  let invoke: unsafe extern "system" fn(
    *mut c_void,
    *mut u8,
    u32,
    *mut u32,
  ) -> windows::core::HRESULT = unsafe { std::mem::transmute(*vtable.add(3)) };
  let mut bytes = [0xff; 5];
  let mut count = u32::MAX;
  let hresult = unsafe { invoke(raw, bytes.as_mut_ptr(), bytes.len() as u32, &mut count) };
  TsfnComAllocatedBufferResult {
    hresult: hresult.0,
    count,
    byte_sum: bytes.iter().map(|value| u32::from(*value)).sum(),
  }
}

#[napi]
pub fn tsfn_test_invoke_retained_com_sink() -> napi::Result<i32> {
  let raw = RETAINED_COM_SINK.load(Ordering::SeqCst) as *mut c_void;
  if raw.is_null() {
    return Err(napi::Error::from_reason(
      "No COM sink is retained by the TSFN test harness",
    ));
  }
  Ok(unsafe { invoke_com_sink(raw) })
}

#[napi]
pub fn tsfn_test_invoke_retained_com_sink_on_thread() -> napi::Result<i32> {
  let raw = RETAINED_COM_SINK.load(Ordering::SeqCst);
  if raw == 0 {
    return Err(napi::Error::from_reason(
      "No COM sink is retained by the TSFN test harness",
    ));
  }
  thread::spawn(move || unsafe { invoke_com_sink(raw as *mut c_void) })
    .join()
    .map_err(|_| napi::Error::from_reason("COM sink test thread panicked"))
}

#[napi]
pub fn tsfn_test_invoke_retained_com_sink_out() -> napi::Result<TsfnComSinkInvokeResult> {
  let raw = RETAINED_COM_SINK.load(Ordering::SeqCst) as *mut c_void;
  if raw.is_null() {
    return Err(napi::Error::from_reason(
      "No COM sink is retained by the TSFN test harness",
    ));
  }
  Ok(unsafe { invoke_com_sink_out(raw) })
}

#[napi]
pub fn tsfn_test_invoke_retained_com_sink_i32(value: i32) -> napi::Result<i32> {
  let raw = RETAINED_COM_SINK.load(Ordering::SeqCst) as *mut c_void;
  if raw.is_null() {
    return Err(napi::Error::from_reason(
      "No COM sink is retained by the TSFN test harness",
    ));
  }
  Ok(unsafe { invoke_com_sink_i32(raw, value) })
}

#[napi]
pub fn tsfn_test_invoke_retained_com_sink_i32_on_thread(value: i32) -> napi::Result<i32> {
  let raw = RETAINED_COM_SINK.load(Ordering::SeqCst);
  if raw == 0 {
    return Err(napi::Error::from_reason(
      "No COM sink is retained by the TSFN test harness",
    ));
  }
  thread::spawn(move || unsafe { invoke_com_sink_i32(raw as *mut c_void, value) })
    .join()
    .map_err(|_| napi::Error::from_reason("COM sink test thread panicked"))
}

#[napi]
pub fn tsfn_test_invoke_retained_com_sink_direct_i32(value: i32) -> napi::Result<i32> {
  let raw = RETAINED_COM_SINK.load(Ordering::SeqCst) as *mut c_void;
  if raw.is_null() {
    return Err(napi::Error::from_reason(
      "No COM sink is retained by the TSFN test harness",
    ));
  }
  Ok(unsafe { invoke_com_sink_direct_i32(raw, value) })
}

#[napi]
pub fn tsfn_test_invoke_retained_com_sink_direct_i32_on_thread(value: i32) -> napi::Result<i32> {
  let raw = RETAINED_COM_SINK.load(Ordering::SeqCst);
  if raw == 0 {
    return Err(napi::Error::from_reason(
      "No COM sink is retained by the TSFN test harness",
    ));
  }
  thread::spawn(move || unsafe { invoke_com_sink_direct_i32(raw as *mut c_void, value) })
    .join()
    .map_err(|_| napi::Error::from_reason("COM sink test thread panicked"))
}

#[napi]
pub fn tsfn_test_invoke_retained_com_sink_void_i32(value: i32) -> napi::Result<()> {
  let raw = RETAINED_COM_SINK.load(Ordering::SeqCst) as *mut c_void;
  if raw.is_null() {
    return Err(napi::Error::from_reason(
      "No COM sink is retained by the TSFN test harness",
    ));
  }
  unsafe { invoke_com_sink_void_i32(raw, value) };
  Ok(())
}

#[napi]
pub fn tsfn_test_invoke_retained_com_sink_guid() -> napi::Result<i32> {
  let raw = RETAINED_COM_SINK.load(Ordering::SeqCst) as *mut c_void;
  if raw.is_null() {
    return Err(napi::Error::from_reason(
      "No COM sink is retained by the TSFN test harness",
    ));
  }
  let value = windows::core::GUID::from_u128(0x990c600e_60c7_4d28_af4c_bf148a92b11a);
  Ok(unsafe { invoke_com_sink_guid(raw, &value) })
}

#[napi]
pub fn tsfn_test_invoke_retained_com_sink_bstr() -> napi::Result<i32> {
  let raw = RETAINED_COM_SINK.load(Ordering::SeqCst) as *mut c_void;
  if raw.is_null() {
    return Err(napi::Error::from_reason(
      "No COM sink is retained by the TSFN test harness",
    ));
  }
  Ok(unsafe { invoke_com_sink_bstr(raw) })
}

#[napi]
pub fn tsfn_test_invoke_retained_com_sink_wide_string() -> napi::Result<i32> {
  let raw = RETAINED_COM_SINK.load(Ordering::SeqCst) as *mut c_void;
  if raw.is_null() {
    return Err(napi::Error::from_reason(
      "No COM sink is retained by the TSFN test harness",
    ));
  }
  Ok(unsafe { invoke_com_sink_wide_string(raw) })
}

#[napi]
pub fn tsfn_test_invoke_retained_com_sink_ansi_string() -> napi::Result<i32> {
  let raw = RETAINED_COM_SINK.load(Ordering::SeqCst) as *mut c_void;
  if raw.is_null() {
    return Err(napi::Error::from_reason(
      "No COM sink is retained by the TSFN test harness",
    ));
  }
  Ok(unsafe { invoke_com_sink_ansi_string(raw) })
}

#[napi]
pub fn tsfn_test_invoke_retained_com_sink_allocated_buffer(
) -> napi::Result<TsfnComAllocatedBufferResult> {
  let raw = RETAINED_COM_SINK.load(Ordering::SeqCst) as *mut c_void;
  if raw.is_null() {
    return Err(napi::Error::from_reason(
      "No COM sink is retained by the TSFN test harness",
    ));
  }
  Ok(unsafe { invoke_com_sink_allocated_buffer(raw) })
}

#[napi]
pub fn tsfn_test_invoke_retained_com_sink_caller_buffer(
) -> napi::Result<TsfnComAllocatedBufferResult> {
  let raw = RETAINED_COM_SINK.load(Ordering::SeqCst) as *mut c_void;
  if raw.is_null() {
    return Err(napi::Error::from_reason(
      "No COM sink is retained by the TSFN test harness",
    ));
  }
  Ok(unsafe { invoke_com_sink_caller_buffer(raw) })
}

#[napi]
pub fn tsfn_test_invoke_retained_com_object_i32(
  iid: &crate::WinGUID,
  value: i32,
) -> napi::Result<i32> {
  let raw = RETAINED_COM_SINK.load(Ordering::SeqCst) as *mut c_void;
  if raw.is_null() {
    return Err(napi::Error::from_reason(
      "No COM object is retained by the TSFN test harness",
    ));
  }
  let identity = unsafe { IUnknown::from_raw_borrowed(&raw) }
    .ok_or_else(|| napi::Error::from_reason("Retained COM identity is null"))?;
  let mut view = std::ptr::null_mut();
  unsafe { identity.query(&iid.0, &mut view) }
    .ok()
    .map_err(|error| napi::Error::from_reason(error.message()))?;
  if view.is_null() {
    return Err(napi::Error::from_reason(
      "Retained COM object did not expose the requested interface",
    ));
  }
  let view = unsafe { IUnknown::from_raw(view) };
  Ok(unsafe { invoke_com_sink_i32(view.as_raw(), value) })
}

#[napi]
pub fn tsfn_test_release_retained_com_sink() {
  let raw = RETAINED_COM_SINK.swap(0, Ordering::SeqCst);
  if raw != 0 {
    unsafe { drop(IUnknown::from_raw(raw as *mut c_void)) };
  }
}

#[napi]
pub fn tsfn_test_arm_call_pause() {
  managed_tsfn::test_arm_call_pause();
}

#[napi]
pub fn tsfn_test_wait_call_paused(timeout_ms: u32) -> bool {
  wait_until(timeout_ms, managed_tsfn::test_call_paused)
}

#[napi]
pub fn tsfn_test_wait_cleanup_waiting(timeout_ms: u32) -> bool {
  wait_until(timeout_ms, managed_tsfn::test_cleanup_waiting)
}

#[napi]
pub fn tsfn_test_cleanup_acquired() -> bool {
  managed_tsfn::test_cleanup_acquired()
}

#[napi]
pub fn tsfn_test_release_call_pause() {
  managed_tsfn::test_release_call_pause();
}

#[napi]
pub fn tsfn_test_wait_produced(expected: u32, timeout_ms: u32) -> bool {
  wait_until(timeout_ms, || count(&PRODUCED) >= expected)
}

fn wait_until(timeout_ms: u32, predicate: impl Fn() -> bool) -> bool {
  let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
  while !predicate() {
    if Instant::now() >= deadline {
      return false;
    }
    thread::sleep(Duration::from_millis(1));
  }
  true
}
