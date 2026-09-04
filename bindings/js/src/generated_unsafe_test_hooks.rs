// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
  ffi::c_void,
  sync::atomic::{AtomicI32, AtomicU32, AtomicUsize, Ordering},
};

use napi::bindgen_prelude::BigInt;
use napi_derive::napi;
use windows::core::{IUnknown, IUnknown_Vtbl, Interface, GUID, HRESULT};

use crate::DynWinRTValue;

const IID_IWBEM_SERVICES: GUID = GUID::from_u128(0x9556dc99_828c_11cf_a37e_00aa003240c7);
const IID_IWBEM_CONTEXT: GUID = GUID::from_u128(0x44aca674_e8fc_11d0_a07c_00c04fb68820);
const IID_IWBEM_CALL_RESULT: GUID = GUID::from_u128(0x44aca675_e8fc_11d0_a07c_00c04fb68820);
const IID_IAUDIO_CLIENT: GUID = GUID::from_u128(0x1cb9ad4c_dbfa_4c32_b178_c2f568a703b2);
const E_NOINTERFACE: HRESULT = HRESULT(0x80004002u32 as i32);
const E_POINTER: HRESULT = HRESULT(0x80004003u32 as i32);
const E_NOTIMPL: HRESULT = HRESULT(0x80004001u32 as i32);

static QUERY_INTERFACE_CALLS: AtomicU32 = AtomicU32::new(0);
static ADD_REF_CALLS: AtomicU32 = AtomicU32::new(0);
static RELEASE_CALLS: AtomicU32 = AtomicU32::new(0);
static OPEN_NAMESPACE_CALLS: AtomicU32 = AtomicU32::new(0);
static OPEN_NAMESPACE_MODE: AtomicI32 = AtomicI32::new(0);
static OPEN_NAMESPACE_WORKING_SLOT_NULL: AtomicU32 = AtomicU32::new(0);
static OPEN_NAMESPACE_WORKING_ARGUMENT_NULL: AtomicU32 = AtomicU32::new(0);
static OPEN_NAMESPACE_RESULT_SLOT_NULL: AtomicU32 = AtomicU32::new(0);
static OPEN_NAMESPACE_RESULT_ARGUMENT_NULL: AtomicU32 = AtomicU32::new(0);
static OPEN_NAMESPACE_CONTEXT_ARGUMENT_NULL: AtomicU32 = AtomicU32::new(0);
static OPEN_NAMESPACE_LAST_FLAGS: AtomicI32 = AtomicI32::new(0);
static QUERY_OBJECT_SINK_CALLS: AtomicU32 = AtomicU32::new(0);
static LAST_FLAGS: AtomicI32 = AtomicI32::new(0);
static CURRENT_REF_COUNT: AtomicU32 = AtomicU32::new(0);
static LAST_OUTPUT_ADDRESS: AtomicUsize = AtomicUsize::new(0);
static AUDIO_IS_FORMAT_SUPPORTED_CALLS: AtomicU32 = AtomicU32::new(0);
static AUDIO_GET_SERVICE_CALLS: AtomicU32 = AtomicU32::new(0);
static AUDIO_LAST_SHARE_MODE: AtomicI32 = AtomicI32::new(0);
static AUDIO_LAST_FORMAT_TAG: AtomicU32 = AtomicU32::new(0);
static AUDIO_GET_SERVICE_MODE: AtomicI32 = AtomicI32::new(0);
static AUDIO_ADD_REF_CALLS: AtomicU32 = AtomicU32::new(0);
static AUDIO_RELEASE_CALLS: AtomicU32 = AtomicU32::new(0);
static AUDIO_CURRENT_REF_COUNT: AtomicU32 = AtomicU32::new(0);

#[repr(C)]
struct GeneratedIWbemServicesVtbl {
  base__: IUnknown_Vtbl,
  open_namespace: unsafe extern "system" fn(
    *mut c_void,
    *mut u16,
    i32,
    *mut c_void,
    *mut *mut c_void,
    *mut *mut c_void,
  ) -> HRESULT,
  cancel_async_call: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
  query_object_sink: unsafe extern "system" fn(*mut c_void, i32, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct GeneratedIWbemServicesFake {
  vtable: *const GeneratedIWbemServicesVtbl,
  references: AtomicU32,
}

unsafe extern "system" fn query_interface(
  this: *mut c_void,
  iid: *const GUID,
  result: *mut *mut c_void,
) -> HRESULT {
  QUERY_INTERFACE_CALLS.fetch_add(1, Ordering::SeqCst);
  if iid.is_null() || result.is_null() {
    return E_POINTER;
  }
  unsafe {
    *result = std::ptr::null_mut();
    if *iid != IUnknown::IID
      && *iid != IID_IWBEM_SERVICES
      && *iid != IID_IWBEM_CONTEXT
      && *iid != IID_IWBEM_CALL_RESULT
    {
      return E_NOINTERFACE;
    }
    *result = this;
    add_ref(this);
  }
  HRESULT(0)
}

unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
  ADD_REF_CALLS.fetch_add(1, Ordering::SeqCst);
  let object = unsafe { &*this.cast::<GeneratedIWbemServicesFake>() };
  let count = object.references.fetch_add(1, Ordering::SeqCst) + 1;
  CURRENT_REF_COUNT.store(count, Ordering::SeqCst);
  count
}

unsafe extern "system" fn release(this: *mut c_void) -> u32 {
  RELEASE_CALLS.fetch_add(1, Ordering::SeqCst);
  let object = unsafe { &*this.cast::<GeneratedIWbemServicesFake>() };
  let count = object.references.fetch_sub(1, Ordering::SeqCst) - 1;
  CURRENT_REF_COUNT.store(count, Ordering::SeqCst);
  if count == 0 {
    unsafe {
      drop(Box::from_raw(this.cast::<GeneratedIWbemServicesFake>()));
    }
  }
  count
}

unsafe extern "system" fn open_namespace(
  this: *mut c_void,
  _namespace: *mut u16,
  flags: i32,
  context: *mut c_void,
  working_namespace: *mut *mut c_void,
  result: *mut *mut c_void,
) -> HRESULT {
  OPEN_NAMESPACE_CALLS.fetch_add(1, Ordering::SeqCst);
  OPEN_NAMESPACE_LAST_FLAGS.store(flags, Ordering::SeqCst);
  OPEN_NAMESPACE_CONTEXT_ARGUMENT_NULL.store(u32::from(context.is_null()), Ordering::SeqCst);
  OPEN_NAMESPACE_WORKING_ARGUMENT_NULL
    .store(u32::from(working_namespace.is_null()), Ordering::SeqCst);
  let working_is_null = working_namespace.is_null() || unsafe { (*working_namespace).is_null() };
  OPEN_NAMESPACE_WORKING_SLOT_NULL.store(u32::from(working_is_null), Ordering::SeqCst);
  OPEN_NAMESPACE_RESULT_ARGUMENT_NULL.store(u32::from(result.is_null()), Ordering::SeqCst);
  let result_is_null = result.is_null() || unsafe { (*result).is_null() };
  OPEN_NAMESPACE_RESULT_SLOT_NULL.store(u32::from(result_is_null), Ordering::SeqCst);
  if !context.is_null()
    || (!working_namespace.is_null() && !working_is_null)
    || (!result.is_null() && !result_is_null)
  {
    return E_POINTER;
  }
  match OPEN_NAMESPACE_MODE.load(Ordering::SeqCst) {
    0 if flags == 0 && !working_namespace.is_null() && result.is_null() => {
      add_ref(this);
      unsafe { *working_namespace = this };
      HRESULT(0)
    }
    1 if flags == 0x10 && working_namespace.is_null() && !result.is_null() => {
      add_ref(this);
      unsafe { *result = this };
      HRESULT(0)
    }
    -1 => {
      if !working_namespace.is_null() {
        add_ref(this);
        unsafe { *working_namespace = this };
      }
      if !result.is_null() {
        add_ref(this);
        unsafe { *result = this };
      }
      HRESULT(0x80004005u32 as i32)
    }
    _ => E_NOTIMPL,
  }
}

unsafe extern "system" fn cancel_async_call(_this: *mut c_void, _sink: *mut c_void) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn query_object_sink(
  _this: *mut c_void,
  flags: i32,
  result: *mut *mut c_void,
) -> HRESULT {
  if result.is_null() {
    return E_POINTER;
  }
  QUERY_OBJECT_SINK_CALLS.fetch_add(1, Ordering::SeqCst);
  LAST_FLAGS.store(flags, Ordering::SeqCst);
  let output = std::ptr::with_exposed_provenance_mut::<c_void>(0x1234);
  LAST_OUTPUT_ADDRESS.store(output.expose_provenance(), Ordering::SeqCst);
  unsafe {
    *result = output;
  }
  HRESULT(0)
}

static VTABLE: GeneratedIWbemServicesVtbl = GeneratedIWbemServicesVtbl {
  base__: IUnknown_Vtbl {
    QueryInterface: query_interface,
    AddRef: add_ref,
    Release: release,
  },
  open_namespace,
  cancel_async_call,
  query_object_sink,
};

#[napi(object)]
pub struct GeneratedUnsafeComStats {
  pub query_interface_calls: u32,
  pub add_ref_calls: u32,
  pub release_calls: u32,
  pub open_namespace_calls: u32,
  pub open_namespace_working_slot_null: bool,
  pub open_namespace_working_argument_null: bool,
  pub open_namespace_result_slot_null: bool,
  pub open_namespace_result_argument_null: bool,
  pub open_namespace_context_argument_null: bool,
  pub open_namespace_last_flags: i32,
  pub query_object_sink_calls: u32,
  pub last_flags: i32,
  pub current_ref_count: u32,
  pub last_output_address: u32,
}

#[napi]
pub fn create_generated_iwbem_services_fake() -> napi::Result<DynWinRTValue> {
  QUERY_INTERFACE_CALLS.store(0, Ordering::SeqCst);
  ADD_REF_CALLS.store(0, Ordering::SeqCst);
  RELEASE_CALLS.store(0, Ordering::SeqCst);
  OPEN_NAMESPACE_CALLS.store(0, Ordering::SeqCst);
  OPEN_NAMESPACE_MODE.store(0, Ordering::SeqCst);
  OPEN_NAMESPACE_WORKING_SLOT_NULL.store(0, Ordering::SeqCst);
  OPEN_NAMESPACE_WORKING_ARGUMENT_NULL.store(0, Ordering::SeqCst);
  OPEN_NAMESPACE_RESULT_SLOT_NULL.store(0, Ordering::SeqCst);
  OPEN_NAMESPACE_RESULT_ARGUMENT_NULL.store(0, Ordering::SeqCst);
  OPEN_NAMESPACE_CONTEXT_ARGUMENT_NULL.store(0, Ordering::SeqCst);
  OPEN_NAMESPACE_LAST_FLAGS.store(0, Ordering::SeqCst);
  QUERY_OBJECT_SINK_CALLS.store(0, Ordering::SeqCst);
  LAST_FLAGS.store(0, Ordering::SeqCst);
  CURRENT_REF_COUNT.store(1, Ordering::SeqCst);
  LAST_OUTPUT_ADDRESS.store(0, Ordering::SeqCst);

  let object = Box::new(GeneratedIWbemServicesFake {
    vtable: &VTABLE,
    references: AtomicU32::new(1),
  });
  let unknown = unsafe { IUnknown::from_raw(Box::into_raw(object).cast()) };
  crate::com::apartment_bound_com_object(unknown)
}

#[napi]
pub fn generated_unsafe_com_stats() -> GeneratedUnsafeComStats {
  GeneratedUnsafeComStats {
    query_interface_calls: QUERY_INTERFACE_CALLS.load(Ordering::SeqCst),
    add_ref_calls: ADD_REF_CALLS.load(Ordering::SeqCst),
    release_calls: RELEASE_CALLS.load(Ordering::SeqCst),
    open_namespace_calls: OPEN_NAMESPACE_CALLS.load(Ordering::SeqCst),
    open_namespace_working_slot_null: OPEN_NAMESPACE_WORKING_SLOT_NULL.load(Ordering::SeqCst) != 0,
    open_namespace_working_argument_null: OPEN_NAMESPACE_WORKING_ARGUMENT_NULL
      .load(Ordering::SeqCst)
      != 0,
    open_namespace_result_slot_null: OPEN_NAMESPACE_RESULT_SLOT_NULL.load(Ordering::SeqCst) != 0,
    open_namespace_result_argument_null: OPEN_NAMESPACE_RESULT_ARGUMENT_NULL.load(Ordering::SeqCst)
      != 0,
    open_namespace_context_argument_null: OPEN_NAMESPACE_CONTEXT_ARGUMENT_NULL
      .load(Ordering::SeqCst)
      != 0,
    open_namespace_last_flags: OPEN_NAMESPACE_LAST_FLAGS.load(Ordering::SeqCst),
    query_object_sink_calls: QUERY_OBJECT_SINK_CALLS.load(Ordering::SeqCst),
    last_flags: LAST_FLAGS.load(Ordering::SeqCst),
    current_ref_count: CURRENT_REF_COUNT.load(Ordering::SeqCst),
    last_output_address: LAST_OUTPUT_ADDRESS.load(Ordering::SeqCst) as u32,
  }
}

#[napi]
pub fn set_generated_iwbem_services_open_namespace_mode(mode: i32) -> napi::Result<()> {
  if !matches!(mode, -1..=1) {
    return Err(napi::Error::from_reason(
      "IWbemServices::OpenNamespace test mode must be -1, 0, or 1",
    ));
  }
  OPEN_NAMESPACE_MODE.store(mode, Ordering::SeqCst);
  Ok(())
}

#[repr(C)]
struct GeneratedAudioClientVtbl {
  base__: IUnknown_Vtbl,
  initialize:
    unsafe extern "system" fn(*mut c_void, i32, u32, i64, i64, *mut c_void, *const GUID) -> HRESULT,
  get_buffer_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
  get_stream_latency: unsafe extern "system" fn(*mut c_void, *mut i64) -> HRESULT,
  get_current_padding: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
  is_format_supported:
    unsafe extern "system" fn(*mut c_void, i32, *mut c_void, *mut *mut c_void) -> HRESULT,
  get_mix_format: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
  get_device_period: unsafe extern "system" fn(*mut c_void, *mut i64, *mut i64) -> HRESULT,
  start: unsafe extern "system" fn(*mut c_void) -> HRESULT,
  stop: unsafe extern "system" fn(*mut c_void) -> HRESULT,
  reset: unsafe extern "system" fn(*mut c_void) -> HRESULT,
  set_event_handle: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
  get_service: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct GeneratedAudioClientFake {
  vtable: *const GeneratedAudioClientVtbl,
  references: AtomicU32,
}

unsafe extern "system" fn audio_query_interface(
  this: *mut c_void,
  iid: *const GUID,
  result: *mut *mut c_void,
) -> HRESULT {
  if iid.is_null() || result.is_null() {
    return E_POINTER;
  }
  unsafe {
    *result = std::ptr::null_mut();
    if *iid != IUnknown::IID && *iid != IID_IAUDIO_CLIENT {
      return E_NOINTERFACE;
    }
    *result = this;
    audio_add_ref(this);
  }
  HRESULT(0)
}

unsafe extern "system" fn audio_add_ref(this: *mut c_void) -> u32 {
  AUDIO_ADD_REF_CALLS.fetch_add(1, Ordering::SeqCst);
  let object = unsafe { &*this.cast::<GeneratedAudioClientFake>() };
  let count = object.references.fetch_add(1, Ordering::SeqCst) + 1;
  AUDIO_CURRENT_REF_COUNT.store(count, Ordering::SeqCst);
  count
}

unsafe extern "system" fn audio_release(this: *mut c_void) -> u32 {
  AUDIO_RELEASE_CALLS.fetch_add(1, Ordering::SeqCst);
  let object = unsafe { &*this.cast::<GeneratedAudioClientFake>() };
  let count = object.references.fetch_sub(1, Ordering::SeqCst) - 1;
  AUDIO_CURRENT_REF_COUNT.store(count, Ordering::SeqCst);
  if count == 0 {
    unsafe {
      drop(Box::from_raw(this.cast::<GeneratedAudioClientFake>()));
    }
  }
  count
}

unsafe extern "system" fn audio_initialize(
  _this: *mut c_void,
  _share_mode: i32,
  _stream_flags: u32,
  _buffer_duration: i64,
  _periodicity: i64,
  _format: *mut c_void,
  _session: *const GUID,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn audio_get_u32(_this: *mut c_void, _value: *mut u32) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn audio_get_i64(_this: *mut c_void, _value: *mut i64) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn audio_is_format_supported(
  _this: *mut c_void,
  share_mode: i32,
  format: *mut c_void,
  closest: *mut *mut c_void,
) -> HRESULT {
  if format.is_null() || closest.is_null() {
    return E_POINTER;
  }
  AUDIO_IS_FORMAT_SUPPORTED_CALLS.fetch_add(1, Ordering::SeqCst);
  AUDIO_LAST_SHARE_MODE.store(share_mode, Ordering::SeqCst);
  AUDIO_LAST_FORMAT_TAG.store(
    unsafe { u32::from(*format.cast::<u16>()) },
    Ordering::SeqCst,
  );
  let allocation = unsafe { windows::Win32::System::Com::CoTaskMemAlloc(16) };
  if allocation.is_null() {
    return HRESULT(0x8007000eu32 as i32);
  }
  unsafe {
    allocation.cast::<u32>().write(0xaabb_ccdd);
    *closest = allocation;
  }
  if share_mode == 2 {
    HRESULT(0x80004005u32 as i32)
  } else {
    HRESULT(1)
  }
}

unsafe extern "system" fn audio_get_mix_format(
  _this: *mut c_void,
  _format: *mut *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn audio_get_device_period(
  _this: *mut c_void,
  _default: *mut i64,
  _minimum: *mut i64,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn audio_no_args(_this: *mut c_void) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn audio_set_event_handle(
  _this: *mut c_void,
  _event: *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn audio_get_service(
  this: *mut c_void,
  _iid: *const GUID,
  result: *mut *mut c_void,
) -> HRESULT {
  if result.is_null() {
    return E_POINTER;
  }
  AUDIO_GET_SERVICE_CALLS.fetch_add(1, Ordering::SeqCst);
  let mode = AUDIO_GET_SERVICE_MODE.load(Ordering::SeqCst);
  unsafe {
    *result = std::ptr::null_mut();
    if mode == 2 {
      return HRESULT(0);
    }
    if mode == -2 {
      return HRESULT(0x80004005u32 as i32);
    }
    if mode.abs() == 3 {
      *result = windows::core::BSTR::from("stage2-bstr")
        .into_raw()
        .cast_mut()
        .cast();
      return if mode < 0 {
        HRESULT(0x80004005u32 as i32)
      } else {
        HRESULT(0)
      };
    }
    if mode.abs() == 4 {
      *result =
        windows::Win32::System::Memory::LocalAlloc(windows::Win32::System::Memory::LMEM_FIXED, 32)
          .map_or(std::ptr::null_mut(), |value| value.0);
      return if mode < 0 {
        HRESULT(0x80004005u32 as i32)
      } else {
        HRESULT(0)
      };
    }
    if mode.abs() == 5 {
      *result =
        windows::Win32::System::Memory::GlobalAlloc(windows::Win32::System::Memory::GMEM_FIXED, 32)
          .map_or(std::ptr::null_mut(), |value| value.0);
      return if mode < 0 {
        HRESULT(0x80004005u32 as i32)
      } else {
        HRESULT(0)
      };
    }
    audio_add_ref(this);
    *result = this;
  }
  if mode == 1 {
    HRESULT(0x80004005u32 as i32)
  } else {
    HRESULT(0)
  }
}

static AUDIO_VTABLE: GeneratedAudioClientVtbl = GeneratedAudioClientVtbl {
  base__: IUnknown_Vtbl {
    QueryInterface: audio_query_interface,
    AddRef: audio_add_ref,
    Release: audio_release,
  },
  initialize: audio_initialize,
  get_buffer_size: audio_get_u32,
  get_stream_latency: audio_get_i64,
  get_current_padding: audio_get_u32,
  is_format_supported: audio_is_format_supported,
  get_mix_format: audio_get_mix_format,
  get_device_period: audio_get_device_period,
  start: audio_no_args,
  stop: audio_no_args,
  reset: audio_no_args,
  set_event_handle: audio_set_event_handle,
  get_service: audio_get_service,
};

#[napi(object)]
pub struct GeneratedAudioClientStats {
  pub is_format_supported_calls: u32,
  pub get_service_calls: u32,
  pub last_share_mode: i32,
  pub last_format_tag: u32,
  pub add_ref_calls: u32,
  pub release_calls: u32,
  pub current_ref_count: u32,
}

#[napi]
pub fn create_generated_audio_client_fake() -> napi::Result<DynWinRTValue> {
  AUDIO_IS_FORMAT_SUPPORTED_CALLS.store(0, Ordering::SeqCst);
  AUDIO_GET_SERVICE_CALLS.store(0, Ordering::SeqCst);
  AUDIO_LAST_SHARE_MODE.store(0, Ordering::SeqCst);
  AUDIO_LAST_FORMAT_TAG.store(0, Ordering::SeqCst);
  AUDIO_GET_SERVICE_MODE.store(0, Ordering::SeqCst);
  AUDIO_ADD_REF_CALLS.store(0, Ordering::SeqCst);
  AUDIO_RELEASE_CALLS.store(0, Ordering::SeqCst);
  AUDIO_CURRENT_REF_COUNT.store(1, Ordering::SeqCst);
  let object = Box::new(GeneratedAudioClientFake {
    vtable: &AUDIO_VTABLE,
    references: AtomicU32::new(1),
  });
  let unknown = unsafe { IUnknown::from_raw(Box::into_raw(object).cast()) };
  crate::com::apartment_bound_com_object(unknown)
}

#[napi]
pub fn set_generated_audio_client_get_service_mode(mode: i32) -> napi::Result<()> {
  if !(-5..=5).contains(&mode) {
    return Err(napi::Error::from_reason(
      "generated audio GetService mode must be between -5 and 5",
    ));
  }
  AUDIO_GET_SERVICE_MODE.store(mode, Ordering::SeqCst);
  Ok(())
}

#[napi]
pub fn generated_audio_client_stats() -> GeneratedAudioClientStats {
  GeneratedAudioClientStats {
    is_format_supported_calls: AUDIO_IS_FORMAT_SUPPORTED_CALLS.load(Ordering::SeqCst),
    get_service_calls: AUDIO_GET_SERVICE_CALLS.load(Ordering::SeqCst),
    last_share_mode: AUDIO_LAST_SHARE_MODE.load(Ordering::SeqCst),
    last_format_tag: AUDIO_LAST_FORMAT_TAG.load(Ordering::SeqCst),
    add_ref_calls: AUDIO_ADD_REF_CALLS.load(Ordering::SeqCst),
    release_calls: AUDIO_RELEASE_CALLS.load(Ordering::SeqCst),
    current_ref_count: AUDIO_CURRENT_REF_COUNT.load(Ordering::SeqCst),
  }
}

#[napi]
pub fn create_generated_native_cleanup_resource(kind: u32) -> napi::Result<BigInt> {
  let address = match kind {
    1 => unsafe {
      windows::Win32::System::Threading::CreateEventW(None, true, false, None)
        .map(|value| value.0 as usize)
    },
    2 => {
      let and_mask = [0xffu8; 32];
      let xor_mask = [0u8; 32];
      unsafe {
        windows::Win32::UI::WindowsAndMessaging::CreateIcon(
          None,
          16,
          16,
          1,
          1,
          and_mask.as_ptr(),
          xor_mask.as_ptr(),
        )
        .map(|value| value.0 as usize)
      }
    }
    3 => {
      let brush = unsafe {
        windows::Win32::Graphics::Gdi::CreateSolidBrush(windows::Win32::Foundation::COLORREF(
          0x0000ff,
        ))
      };
      if brush.is_invalid() {
        Err(windows::core::Error::from_thread())
      } else {
        Ok(brush.0 as usize)
      }
    }
    _ => {
      return Err(napi::Error::from_reason(
        "generated cleanup resource kind must be 1, 2, or 3",
      ));
    }
  }
  .map_err(|error| napi::Error::from_reason(error.message()))?;
  Ok(BigInt::from(address as u64))
}
