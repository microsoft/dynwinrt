// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use napi::bindgen_prelude::{BigInt, Buffer, FromNapiValue, ToNapiValue, Unknown};
use napi::JsValue;
use napi_derive::napi;
use windows::core::{IUnknown, Interface as _, GUID};

use super::{DynWinRTType, DynWinRTValue, WinGUID, TABLE};

#[allow(dead_code)]
pub(super) enum NativePointerOwner {
  Uint8Array {
    value: std::sync::Mutex<napi::bindgen_prelude::Uint8Array>,
    env: napi::sys::napi_env,
    pointer: usize,
    length: usize,
  },
  TypedBuffer {
    env: napi::sys::napi_env,
    reference: napi::sys::napi_ref,
    pointer: usize,
    byte_length: usize,
    typed_array_type: i32,
  },
  CoTaskMem(*mut std::ffi::c_void),
  Guid(*mut GUID),
  WideString(Box<[u16]>),
  AnsiString(Box<[u8]>),
}

enum AutomationValueKind {
  Bstr(dynwinrt::com::BstrValue),
  NativeUnion(dynwinrt::com::NativeUnionValue),
  Variant(dynwinrt::com::VariantValue),
  SafeArray(dynwinrt::com::SafeArrayValue),
  PropVariant(dynwinrt::com::PropVariantValue),
  DispatchParams(dynwinrt::com::DispatchParamsValue),
  ExcepInfo(dynwinrt::com::ExcepInfoValue),
  StatStg(dynwinrt::com::StatStgValue),
}

pub(super) struct AutomationValue {
  owner_thread: std::thread::ThreadId,
  value: Option<AutomationValueKind>,
}

impl AutomationValue {
  pub(super) fn new(value: dynwinrt::com::Value) -> Self {
    let value = match value {
      dynwinrt::com::Value::Bstr(value) => AutomationValueKind::Bstr(value),
      dynwinrt::com::Value::NativeUnion(value) => AutomationValueKind::NativeUnion(value),
      dynwinrt::com::Value::Variant(value) => AutomationValueKind::Variant(value),
      dynwinrt::com::Value::SafeArray(value) => AutomationValueKind::SafeArray(value),
      dynwinrt::com::Value::PropVariant(value) => AutomationValueKind::PropVariant(value),
      dynwinrt::com::Value::DispatchParams(value) => AutomationValueKind::DispatchParams(value),
      dynwinrt::com::Value::ExcepInfo(value) => AutomationValueKind::ExcepInfo(value),
      dynwinrt::com::Value::StatStg(value) => AutomationValueKind::StatStg(value),
      _ => unreachable!("AutomationValue requires an automation COM value"),
    };
    Self {
      owner_thread: std::thread::current().id(),
      value: Some(value),
    }
  }

  fn ensure_owner_thread(&self) -> napi::Result<()> {
    if matches!(self.value, Some(AutomationValueKind::Bstr(_)))
      || std::thread::current().id() == self.owner_thread
    {
      Ok(())
    } else {
      Err(napi::Error::from_reason(
        "Apartment-bound COM Automation value used from a different thread",
      ))
    }
  }

  pub(super) fn to_com_value(&self) -> napi::Result<dynwinrt::com::Value> {
    self.ensure_owner_thread()?;
    let value = self
      .value
      .as_ref()
      .ok_or_else(|| napi::Error::from_reason("COM Automation value has been consumed"))?;
    Ok(match value {
      AutomationValueKind::Bstr(value) => dynwinrt::com::Value::Bstr(value.clone()),
      AutomationValueKind::NativeUnion(value) => dynwinrt::com::Value::NativeUnion(value.clone()),
      AutomationValueKind::Variant(value) => dynwinrt::com::Value::Variant(value.clone()),
      AutomationValueKind::SafeArray(value) => dynwinrt::com::Value::SafeArray(value.clone()),
      AutomationValueKind::PropVariant(value) => dynwinrt::com::Value::PropVariant(value.clone()),
      AutomationValueKind::DispatchParams(value) => {
        dynwinrt::com::Value::DispatchParams(value.clone())
      }
      AutomationValueKind::ExcepInfo(value) => dynwinrt::com::Value::ExcepInfo(value.clone()),
      AutomationValueKind::StatStg(value) => dynwinrt::com::Value::StatStg(value.clone()),
    })
  }

  pub(super) fn take_variant(&mut self) -> napi::Result<dynwinrt::com::VariantValue> {
    self.ensure_owner_thread()?;
    match self.value.take() {
      Some(AutomationValueKind::Variant(value)) => Ok(value),
      value => {
        self.value = value;
        Err(napi::Error::from_reason("Value is not a COM VARIANT"))
      }
    }
  }

  pub(super) fn take_safe_array(&mut self) -> napi::Result<dynwinrt::com::SafeArrayValue> {
    self.ensure_owner_thread()?;
    match self.value.take() {
      Some(AutomationValueKind::SafeArray(value)) => Ok(value),
      value => {
        self.value = value;
        Err(napi::Error::from_reason("Value is not a COM SAFEARRAY"))
      }
    }
  }

  pub(super) fn take_prop_variant(&mut self) -> napi::Result<dynwinrt::com::PropVariantValue> {
    self.ensure_owner_thread()?;
    match self.value.take() {
      Some(AutomationValueKind::PropVariant(value)) => Ok(value),
      value => {
        self.value = value;
        Err(napi::Error::from_reason("Value is not a COM PROPVARIANT"))
      }
    }
  }

  pub(super) fn take_excep_info(&mut self) -> napi::Result<dynwinrt::com::ExcepInfoValue> {
    self.ensure_owner_thread()?;
    match self.value.take() {
      Some(AutomationValueKind::ExcepInfo(value)) => Ok(value),
      value => {
        self.value = value;
        Err(napi::Error::from_reason("Value is not COM EXCEPINFO"))
      }
    }
  }

  pub(super) fn take_stat_stg(&mut self) -> napi::Result<dynwinrt::com::StatStgValue> {
    self.ensure_owner_thread()?;
    match self.value.take() {
      Some(AutomationValueKind::StatStg(value)) => Ok(value),
      value => {
        self.value = value;
        Err(napi::Error::from_reason("Value is not COM STATSTG"))
      }
    }
  }

  pub(super) fn leak_for_shutdown(&mut self) {
    if let Some(value) = self.value.take() {
      std::mem::forget(value);
    }
  }
}

impl Drop for AutomationValue {
  fn drop(&mut self) {
    if !matches!(self.value, Some(AutomationValueKind::Bstr(_)))
      && (std::thread::current().id() != self.owner_thread || super::winui_dispatcher_loop_exited())
    {
      self.leak_for_shutdown();
    }
  }
}

// Safety: access is rejected off the creating apartment. Wrong-thread and
// post-WinUI destruction drops leak the value instead of invoking native
// cleanup on an invalid apartment.
unsafe impl Send for AutomationValue {}
unsafe impl Sync for AutomationValue {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PointerProvenance {
  None,
  Borrowed,
  UnclassifiedOutput,
  ComOutput,
  CoTaskMemOutput,
  BstrOutput,
}

impl NativePointerOwner {
  fn validate(&self) -> napi::Result<()> {
    if let Self::TypedBuffer {
      env,
      reference,
      pointer,
      byte_length,
      typed_array_type,
    } = self
    {
      let mut raw = std::ptr::null_mut();
      napi::check_status!(
        unsafe { napi::sys::napi_get_reference_value(*env, *reference, &mut raw) },
        "Failed to revalidate COM buffer owner"
      )?;
      let info = typed_buffer_info(*env, raw)?;
      if info.pointer != *pointer
        || info.byte_length != *byte_length
        || info.typed_array_type != *typed_array_type
      {
        return Err(napi::Error::from_reason(
          "Cannot use a COM buffer whose TypedArray backing storage changed",
        ));
      }
      return Ok(());
    }
    let Self::Uint8Array {
      value,
      env,
      pointer,
      length,
    } = self
    else {
      return Ok(());
    };
    let mut value = value
      .lock()
      .map_err(|_| napi::Error::from_reason("TypedArray pointer owner lock is poisoned"))?;
    let raw = unsafe {
      <&mut napi::bindgen_prelude::Uint8Array as ToNapiValue>::to_napi_value(*env, &mut *value)
    }?;
    let mut typed_array_type = 0;
    let mut current_length = 0usize;
    let mut current_pointer = std::ptr::null_mut();
    let mut array_buffer = std::ptr::null_mut();
    let mut byte_offset = 0usize;
    napi::check_status!(
      unsafe {
        napi::sys::napi_get_typedarray_info(
          *env,
          raw,
          &mut typed_array_type,
          &mut current_length,
          &mut current_pointer,
          &mut array_buffer,
          &mut byte_offset,
        )
      },
      "Failed to revalidate TypedArray backing storage"
    )?;
    let mut detached = false;
    napi::check_status!(
      unsafe { napi::sys::napi_is_detached_arraybuffer(*env, array_buffer, &mut detached) },
      "Failed to inspect TypedArray backing storage"
    )?;
    if detached {
      return Err(napi::Error::from_reason(
        "Cannot use a pointer whose TypedArray backing ArrayBuffer is detached",
      ));
    }
    let current_pointer = if current_length == 0 {
      0
    } else {
      current_pointer as usize
    };
    if current_length != *length || current_pointer != *pointer {
      return Err(napi::Error::from_reason(
        "Cannot use a pointer whose TypedArray backing storage changed",
      ));
    }
    Ok(())
  }
}

impl Drop for NativePointerOwner {
  fn drop(&mut self) {
    match self {
      Self::CoTaskMem(ptr) => {
        if !ptr.is_null() {
          unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(*ptr)) };
          *ptr = std::ptr::null_mut();
        }
      }
      Self::Guid(ptr) => {
        if !ptr.is_null() {
          drop(unsafe { Box::from_raw(*ptr) });
          *ptr = std::ptr::null_mut();
        }
      }
      Self::TypedBuffer { env, reference, .. } => {
        if !reference.is_null() {
          let _ = unsafe { napi::sys::napi_delete_reference(*env, *reference) };
          *reference = std::ptr::null_mut();
        }
      }
      _ => {}
    }
  }
}

fn parse_clsid(clsid: &str) -> napi::Result<windows::core::GUID> {
  windows::core::GUID::try_from(clsid)
    .map_err(|_| napi::Error::from_reason(format!("Invalid CLSID: '{clsid}'")))
}

fn bind_com_result(result: dynwinrt::Result<dynwinrt::WinRTValue>) -> napi::Result<DynWinRTValue> {
  let mut value = result
    .map(DynWinRTValue::new)
    .map_err(|error| napi::Error::from_reason(error.message()))?;
  value.bind_current_com_apartment()?;
  Ok(value)
}

fn co_create_instance(clsid: String, iid: &WinGUID) -> napi::Result<DynWinRTValue> {
  bind_com_result(dynwinrt::com::co_create_instance(
    parse_clsid(&clsid)?,
    iid.0,
  ))
}

fn co_get_class_object(clsid: String, iid: &WinGUID) -> napi::Result<DynWinRTValue> {
  bind_com_result(dynwinrt::com::co_get_class_object(
    parse_clsid(&clsid)?,
    iid.0,
  ))
}

fn co_get_malloc() -> napi::Result<DynWinRTValue> {
  bind_com_result(dynwinrt::com::co_get_malloc())
}

fn create_error_info() -> napi::Result<DynWinRTValue> {
  bind_com_result(dynwinrt::com::create_error_info())
}

fn set_error_info(value: Option<&DynWinRTValue>) -> napi::Result<()> {
  if let Some(value) = value {
    value.ensure_existing_com_apartment()?;
  }
  dynwinrt::com::set_error_info(value.map(|value| &value.0)).map_err(com_error)
}

fn get_error_info() -> napi::Result<Option<DynWinRTValue>> {
  dynwinrt::com::get_error_info()
    .map_err(com_error)?
    .map(|value| {
      let mut value = DynWinRTValue::new(value);
      value.bind_current_com_apartment()?;
      Ok(value)
    })
    .transpose()
}

fn try_cast(value: &DynWinRTValue, iid: &WinGUID) -> napi::Result<Option<DynWinRTValue>> {
  const E_NOINTERFACE: windows::core::HRESULT = windows::core::HRESULT(0x80004002u32 as i32);

  value.ensure_existing_com_apartment()?;
  match value.0.cast(&iid.0) {
    Ok(value) => {
      let mut value = DynWinRTValue::new(value);
      value.bind_current_com_apartment()?;
      Ok(Some(value))
    }
    Err(dynwinrt::Error::WindowsError(error)) if error.code() == E_NOINTERFACE => Ok(None),
    Err(error) => Err(napi::Error::from_reason(error.message())),
  }
}

fn apartment_bound_com_object(value: IUnknown) -> napi::Result<DynWinRTValue> {
  let mut value = DynWinRTValue::new(dynwinrt::WinRTValue::Object(value));
  value.bind_current_com_apartment()?;
  Ok(value)
}

fn create_test_hwnd() -> napi::Result<BigInt> {
  use std::sync::atomic::{AtomicUsize, Ordering};
  use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, WINDOW_EX_STYLE, WS_POPUP};

  static CACHED_HWND: AtomicUsize = AtomicUsize::new(0);
  let cached = CACHED_HWND.load(Ordering::Acquire);
  if cached != 0 {
    return Ok(BigInt::from(cached as u64));
  }
  let class_name: Vec<u16> = "STATIC".encode_utf16().chain(Some(0)).collect();
  let title: Vec<u16> = "dynwinrt-test-hwnd\0".encode_utf16().collect();
  let hwnd = unsafe {
    CreateWindowExW(
      WINDOW_EX_STYLE(0),
      windows::core::PCWSTR(class_name.as_ptr()),
      windows::core::PCWSTR(title.as_ptr()),
      WS_POPUP,
      0,
      0,
      1,
      1,
      None,
      None,
      None,
      None,
    )
  }
  .map_err(|error| napi::Error::from_reason(format!("CreateWindowExW: {error}")))?;
  let bits = hwnd.0 as usize;
  CACHED_HWND.store(bits, Ordering::Release);
  Ok(BigInt::from(bits as u64))
}

struct Uint8ArrayInfo {
  data: *const u8,
  length: usize,
}

struct TypedBufferInfo {
  pointer: usize,
  byte_length: usize,
  source_element_size: usize,
  raw_bytes: bool,
  typed_array_type: i32,
}

fn reject_shared_array_buffer(
  env: napi::sys::napi_env,
  array_buffer: napi::sys::napi_value,
) -> napi::Result<()> {
  let mut is_array_buffer = false;
  napi::check_status!(
    unsafe { napi::sys::napi_is_arraybuffer(env, array_buffer, &mut is_array_buffer) },
    "Failed to inspect TypedArray backing storage"
  )?;
  if !is_array_buffer {
    return Err(napi::Error::from_reason(
      "SharedArrayBuffer-backed views cannot be passed to native COM calls",
    ));
  }
  Ok(())
}

fn typed_array_element_size(typed_array_type: i32) -> napi::Result<usize> {
  use napi::sys::TypedarrayType;
  match typed_array_type {
    value
      if value == TypedarrayType::int8_array as i32
        || value == TypedarrayType::uint8_array as i32
        || value == TypedarrayType::uint8_clamped_array as i32 =>
    {
      Ok(1)
    }
    value
      if value == TypedarrayType::int16_array as i32
        || value == TypedarrayType::uint16_array as i32 =>
    {
      Ok(2)
    }
    value
      if value == TypedarrayType::int32_array as i32
        || value == TypedarrayType::uint32_array as i32
        || value == TypedarrayType::float32_array as i32 =>
    {
      Ok(4)
    }
    value
      if value == TypedarrayType::float64_array as i32
        || value == TypedarrayType::bigint64_array as i32
        || value == TypedarrayType::biguint64_array as i32 =>
    {
      Ok(8)
    }
    _ => Err(napi::Error::from_reason(
      "Unsupported TypedArray element type for a COM buffer",
    )),
  }
}

fn typed_buffer_info(
  env: napi::sys::napi_env,
  raw: napi::sys::napi_value,
) -> napi::Result<TypedBufferInfo> {
  let mut is_typed_array = false;
  napi::check_status!(
    unsafe { napi::sys::napi_is_typedarray(env, raw, &mut is_typed_array) },
    "Failed to inspect COM buffer value"
  )?;
  if !is_typed_array {
    return Err(napi::Error::from_reason(
      "DynCom.buffer(): expected Buffer or TypedArray",
    ));
  }
  let mut typed_array_type = 0;
  let mut length = 0usize;
  let mut data = std::ptr::null_mut();
  let mut array_buffer = std::ptr::null_mut();
  let mut byte_offset = 0usize;
  napi::check_status!(
    unsafe {
      napi::sys::napi_get_typedarray_info(
        env,
        raw,
        &mut typed_array_type,
        &mut length,
        &mut data,
        &mut array_buffer,
        &mut byte_offset,
      )
    },
    "Failed to inspect COM TypedArray backing storage"
  )?;
  reject_shared_array_buffer(env, array_buffer)?;
  let mut detached = false;
  napi::check_status!(
    unsafe { napi::sys::napi_is_detached_arraybuffer(env, array_buffer, &mut detached) },
    "Failed to inspect COM TypedArray backing storage"
  )?;
  if detached {
    return Err(napi::Error::from_reason(
      "Cannot use a COM buffer whose backing ArrayBuffer is detached",
    ));
  }
  let source_element_size = typed_array_element_size(typed_array_type)?;
  let byte_length = length
    .checked_mul(source_element_size)
    .ok_or_else(|| napi::Error::from_reason("COM buffer byte length overflow"))?;
  let mut is_buffer = false;
  napi::check_status!(
    unsafe { napi::sys::napi_is_buffer(env, raw, &mut is_buffer) },
    "Failed to identify Node Buffer storage"
  )?;
  Ok(TypedBufferInfo {
    pointer: if byte_length == 0 { 0 } else { data as usize },
    byte_length,
    source_element_size,
    raw_bytes: is_buffer,
    typed_array_type,
  })
}

fn com_buffer(value: Unknown) -> napi::Result<DynWinRTValue> {
  let env = value.value().env;
  let raw = value.value().value;
  let info = typed_buffer_info(env, raw)?;
  let mut reference = std::ptr::null_mut();
  napi::check_status!(
    unsafe { napi::sys::napi_create_reference(env, raw, 1, &mut reference) },
    "Failed to retain COM buffer backing storage"
  )?;
  let buffer = unsafe {
    dynwinrt::com::ComBufferValue::borrowed(
      info.pointer as *mut u8,
      info.byte_length,
      info.source_element_size,
      info.raw_bytes,
      true,
    )
  }
  .map_err(|error| napi::Error::from_reason(error.message()))?;
  Ok(DynWinRTValue::with_com_buffer(
    buffer,
    NativePointerOwner::TypedBuffer {
      env,
      reference,
      pointer: info.pointer,
      byte_length: info.byte_length,
      typed_array_type: info.typed_array_type,
    },
  ))
}

fn take_array_bytes(value: &mut DynWinRTValue, width: usize) -> napi::Result<Vec<u8>> {
  let buffer = value
    .4
    .as_ref()
    .ok_or_else(|| napi::Error::from_reason("Value is not an owned COM array result"))?;
  let bytes = buffer
    .snapshot_bytes()
    .map_err(com_error)?
    .ok_or_else(|| napi::Error::from_reason("Borrowed COM arrays cannot be consumed"))?;
  if width == 0 || bytes.len() % width != 0 {
    return Err(napi::Error::from_reason(
      "COM array result has an invalid scalar element width",
    ));
  }
  value.4 = None;
  Ok(bytes)
}

fn validate_wide_string_bytes(bytes: &[u8]) -> napi::Result<()> {
  if bytes.len() < 2 || bytes.len() % 2 != 0 {
    return Err(napi::Error::from_reason(
      "wideStringPointer(): Buffer/Uint8Array must have an even byte length and include a UTF-16 NUL terminator",
    ));
  }
  if bytes[bytes.len() - 2..] != [0, 0] {
    return Err(napi::Error::from_reason(
      "wideStringPointer(): Buffer/Uint8Array must end with a UTF-16 NUL terminator",
    ));
  }
  Ok(())
}

fn validate_ansi_string_bytes(bytes: &[u8]) -> napi::Result<()> {
  if bytes.last() != Some(&0) {
    return Err(napi::Error::from_reason(
      "ansiStringPointer(): Buffer/Uint8Array must end with a NUL terminator",
    ));
  }
  Ok(())
}

fn uint8_array_info(
  env: napi::sys::napi_env,
  raw: napi::sys::napi_value,
) -> napi::Result<Option<Uint8ArrayInfo>> {
  let mut is_typed_array = false;
  napi::check_status!(
    unsafe { napi::sys::napi_is_typedarray(env, raw, &mut is_typed_array) },
    "Failed to inspect TypedArray value"
  )?;
  if !is_typed_array {
    return Ok(None);
  }

  let mut typed_array_type = 0;
  let mut length = 0usize;
  let mut data = std::ptr::null_mut();
  let mut array_buffer = std::ptr::null_mut();
  let mut byte_offset = 0usize;
  napi::check_status!(
    unsafe {
      napi::sys::napi_get_typedarray_info(
        env,
        raw,
        &mut typed_array_type,
        &mut length,
        &mut data,
        &mut array_buffer,
        &mut byte_offset,
      )
    },
    "Failed to inspect TypedArray backing storage"
  )?;
  reject_shared_array_buffer(env, array_buffer)?;
  if typed_array_type != napi::sys::TypedarrayType::uint8_array as i32 {
    return Ok(None);
  }

  let mut detached = false;
  napi::check_status!(
    unsafe { napi::sys::napi_is_detached_arraybuffer(env, array_buffer, &mut detached) },
    "Failed to inspect TypedArray backing storage"
  )?;
  if detached {
    return Err(napi::Error::from_reason(
      "Cannot use a detached Buffer/Uint8Array",
    ));
  }

  Ok(Some(Uint8ArrayInfo {
    data: data.cast(),
    length,
  }))
}

fn pointer(value: Unknown) -> napi::Result<DynWinRTValue> {
  use napi::sys;

  let env = value.value().env;
  let raw = value.value().value;
  let mut value_type = sys::ValueType::napi_undefined;
  unsafe { sys::napi_typeof(env, raw, &mut value_type) };
  if matches!(
    value_type,
    sys::ValueType::napi_null | sys::ValueType::napi_undefined
  ) {
    return Ok(DynWinRTValue::with_borrowed_pointer(
      dynwinrt::WinRTValue::RawPtr(std::ptr::null_mut()),
    ));
  }
  if value_type == sys::ValueType::napi_bigint {
    let bigint = unsafe { BigInt::from_napi_value(env, raw) }?;
    let (negative, bits, lossless) = bigint.get_u64();
    if negative || !lossless || bits as usize as u64 != bits {
      return Err(napi::Error::from_reason(
        "pointer(): bigint must fit in an unsigned pointer",
      ));
    }
    return Ok(DynWinRTValue::with_borrowed_pointer(
      dynwinrt::WinRTValue::RawPtr(bits as usize as *mut std::ffi::c_void),
    ));
  }
  if value_type == sys::ValueType::napi_number {
    let mut number = 0.0;
    unsafe { sys::napi_get_value_double(env, raw, &mut number) };
    if !number.is_finite()
      || number < 0.0
      || number.fract() != 0.0
      || number > 9_007_199_254_740_991.0
      || number as u64 as usize as u64 != number as u64
    {
      return Err(napi::Error::from_reason(
        "pointer(): number must be a non-negative safe integer that fits in a pointer",
      ));
    }
    return Ok(DynWinRTValue::with_borrowed_pointer(
      dynwinrt::WinRTValue::RawPtr(number as usize as *mut std::ffi::c_void),
    ));
  }
  if uint8_array_info(env, raw)?.is_some() {
    let array = unsafe { napi::bindgen_prelude::Uint8Array::from_napi_value(env, raw) }?;
    let length = array.len();
    let pointer = if length == 0 {
      0
    } else {
      array.as_ref().as_ptr() as usize
    };
    return Ok(DynWinRTValue::with_pointer_owner(
      dynwinrt::WinRTValue::RawPtr(pointer as *mut std::ffi::c_void),
      NativePointerOwner::Uint8Array {
        value: std::sync::Mutex::new(array),
        env,
        pointer,
        length,
      },
    ));
  }
  // Reject existing DynWinRtValue inputs. Borrowing an Object's raw COM pointer
  // here would make it indistinguishable from an owned raw pointer to
  // adoptComPointer(), which can double-release the original wrapper's COM
  // object. Callers that already have raw pointer bits should pass those bits.
  if unsafe { <&DynWinRTValue>::from_napi_value(env, raw) }.is_ok() {
    return Err(napi::Error::from_reason(
      "pointer(): DynWinRtValue inputs are not accepted; pass raw pointer bits, Buffer/Uint8Array, or null instead",
    ));
  }
  Err(napi::Error::from_reason(
    "pointer(): expected bigint, number, Buffer, Uint8Array, null, or undefined",
  ))
}

fn safe_data_pointer(value: Unknown, nullable: bool) -> napi::Result<DynWinRTValue> {
  use napi::sys;

  let env = value.value().env;
  let raw = value.value().value;
  let mut value_type = sys::ValueType::napi_undefined;
  unsafe { sys::napi_typeof(env, raw, &mut value_type) };
  if matches!(
    value_type,
    sys::ValueType::napi_null | sys::ValueType::napi_undefined
  ) {
    return if nullable {
      pointer(value)
    } else {
      Err(napi::Error::from_reason(
        "safeDataPointer(): null requires an explicitly nullable parameter",
      ))
    };
  }
  if uint8_array_info(env, raw)?.is_some() {
    return pointer(value);
  }
  Err(napi::Error::from_reason(
    "safeDataPointer(): expected Buffer or Uint8Array; arbitrary numeric addresses require @microsoft/dynwinrt/com/unsafe",
  ))
}

fn wide_string_pointer(value: Unknown) -> napi::Result<DynWinRTValue> {
  use napi::sys;

  let env = value.value().env;
  let raw = value.value().value;
  let mut value_type = sys::ValueType::napi_undefined;
  unsafe { sys::napi_typeof(env, raw, &mut value_type) };
  if value_type == sys::ValueType::napi_string {
    let text = unsafe { String::from_napi_value(env, raw) }?;
    let mut storage = text
      .encode_utf16()
      .chain(std::iter::once(0))
      .collect::<Vec<_>>()
      .into_boxed_slice();
    let ptr = storage.as_mut_ptr().cast();
    return Ok(DynWinRTValue::with_pointer_owner(
      dynwinrt::WinRTValue::RawPtr(ptr),
      NativePointerOwner::WideString(storage),
    ));
  }
  if let Some(array) = uint8_array_info(env, raw)? {
    if !array.data.is_null() && (array.data as usize) % std::mem::align_of::<u16>() != 0 {
      return Err(napi::Error::from_reason(
        "wideStringPointer(): Buffer/Uint8Array backing address must be aligned for UTF-16",
      ));
    }
    let bytes = unsafe { std::slice::from_raw_parts(array.data, array.length) };
    validate_wide_string_bytes(bytes)?;
  }
  pointer(value)
}

fn ansi_string_pointer(value: Unknown) -> napi::Result<DynWinRTValue> {
  use napi::sys;

  let env = value.value().env;
  let raw = value.value().value;
  let mut value_type = sys::ValueType::napi_undefined;
  unsafe { sys::napi_typeof(env, raw, &mut value_type) };
  if value_type == sys::ValueType::napi_string {
    let text = unsafe { String::from_napi_value(env, raw) }?;
    if !text.is_ascii() {
      return Err(napi::Error::from_reason(
        "ansiStringPointer(): non-ASCII strings require an explicitly encoded NUL-terminated Buffer/Uint8Array",
      ));
    }
    let mut storage = text.into_bytes();
    storage.push(0);
    let mut storage = storage.into_boxed_slice();
    let ptr = storage.as_mut_ptr().cast();
    return Ok(DynWinRTValue::with_pointer_owner(
      dynwinrt::WinRTValue::RawPtr(ptr),
      NativePointerOwner::AnsiString(storage),
    ));
  }
  if let Some(array) = uint8_array_info(env, raw)? {
    let bytes = unsafe { std::slice::from_raw_parts(array.data, array.length) };
    validate_ansi_string_bytes(bytes)?;
  }
  pointer(value)
}

fn safe_wide_string_pointer(value: Unknown, nullable: bool) -> napi::Result<DynWinRTValue> {
  safe_string_pointer(value, nullable, true)
}

fn safe_ansi_string_pointer(value: Unknown, nullable: bool) -> napi::Result<DynWinRTValue> {
  safe_string_pointer(value, nullable, false)
}

fn safe_string_pointer(value: Unknown, nullable: bool, wide: bool) -> napi::Result<DynWinRTValue> {
  use napi::sys;

  let env = value.value().env;
  let raw = value.value().value;
  let mut value_type = sys::ValueType::napi_undefined;
  unsafe { sys::napi_typeof(env, raw, &mut value_type) };
  if matches!(
    value_type,
    sys::ValueType::napi_null | sys::ValueType::napi_undefined
  ) {
    return if nullable {
      pointer(value)
    } else {
      Err(napi::Error::from_reason(
        "safe string pointer: null requires an explicitly nullable parameter",
      ))
    };
  }
  if value_type == sys::ValueType::napi_string || uint8_array_info(env, raw)?.is_some() {
    return if wide {
      wide_string_pointer(value)
    } else {
      ansi_string_pointer(value)
    };
  }
  Err(napi::Error::from_reason(
    "safe string pointer: expected string, Buffer, or Uint8Array; arbitrary numeric addresses require @microsoft/dynwinrt/com/unsafe",
  ))
}

fn handle_value(value: Unknown) -> napi::Result<BigInt> {
  use napi::sys;

  let env = value.value().env;
  let raw = value.value().value;
  let mut value_type = sys::ValueType::napi_undefined;
  unsafe { sys::napi_typeof(env, raw, &mut value_type) };
  if value_type == sys::ValueType::napi_bigint {
    let bigint = unsafe { BigInt::from_napi_value(env, raw) }?;
    let (negative, bits, lossless) = bigint.get_u64();
    if negative || !lossless || bits as usize as u64 != bits {
      return Err(napi::Error::from_reason(
        "handleValue(): bigint must fit in an unsigned pointer",
      ));
    }
    return Ok(BigInt::from(bits));
  }
  if value_type == sys::ValueType::napi_number {
    let mut number = 0.0;
    unsafe { sys::napi_get_value_double(env, raw, &mut number) };
    if !number.is_finite()
      || number < 0.0
      || number.fract() != 0.0
      || number > 9_007_199_254_740_991.0
      || number as u64 as usize as u64 != number as u64
    {
      return Err(napi::Error::from_reason(
        "handleValue(): number must be a non-negative safe integer that fits in a pointer",
      ));
    }
    return Ok(BigInt::from(number as u64));
  }
  if let Some(array) = uint8_array_info(env, raw)? {
    let expected = std::mem::size_of::<usize>();
    if array.length != expected {
      return Err(napi::Error::from_reason(format!(
        "handleValue(): Buffer/Uint8Array must contain exactly {expected} bytes on this target",
      )));
    }
    if array.data.is_null() {
      return Err(napi::Error::from_reason(
        "handleValue(): Buffer/Uint8Array backing storage is null",
      ));
    }
    let bytes = unsafe { std::slice::from_raw_parts(array.data, array.length) };
    #[cfg(target_pointer_width = "64")]
    let bits = u64::from_le_bytes(bytes.try_into().expect("validated handle byte length"));
    #[cfg(target_pointer_width = "32")]
    let bits = u32::from_le_bytes(bytes.try_into().expect("validated handle byte length")) as u64;
    return Ok(BigInt::from(bits));
  }
  Err(napi::Error::from_reason(
    "handleValue(): expected bigint, number, Buffer, or Uint8Array",
  ))
}

fn adopt_com_pointer(
  value: &mut DynWinRTValue,
  iid: Option<&WinGUID>,
) -> napi::Result<DynWinRTValue> {
  let ptr = take_native_output_pointer(value, PointerProvenance::ComOutput, "COM interface")?;
  let adopted = unsafe { dynwinrt::com::adopt_com_pointer(ptr) };
  match iid {
    Some(iid) => adopted
      .cast(&iid.0)
      .map(DynWinRTValue::new)
      .map_err(|error| napi::Error::from_reason(error.message()))
      .and_then(|mut value| {
        value.bind_current_com_apartment()?;
        Ok(value)
      }),
    None => {
      let mut value = DynWinRTValue::new(adopted);
      value.bind_current_com_apartment()?;
      Ok(value)
    }
  }
}

fn project_winrt_async(
  value: &DynWinRTValue,
  async_type: &DynWinRTType,
) -> napi::Result<DynWinRTValue> {
  dynwinrt::com::project_winrt_async(&value.0, async_type.type_handle())
    .map(DynWinRTValue::new)
    .map_err(|error| napi::Error::from_reason(error.message()))
}

fn explicit_raw_com_pointer(
  value: Unknown,
  operation: &str,
) -> napi::Result<*mut std::ffi::c_void> {
  let value = pointer(value)?;
  if value.1.is_some() {
    return Err(napi::Error::from_reason(format!(
      "{operation}(): Buffer and Uint8Array backing addresses are not accepted; pass explicit numeric pointer bits",
    )));
  }
  match value.0 {
    dynwinrt::WinRTValue::RawPtr(ptr) => Ok(ptr),
    dynwinrt::WinRTValue::Null => Ok(std::ptr::null_mut()),
    _ => Err(napi::Error::from_reason(format!(
      "{operation}(): expected bigint, number, null, or undefined",
    ))),
  }
}

fn unsafe_adopt_owned_com_pointer(value: Unknown, iid: &WinGUID) -> napi::Result<DynWinRTValue> {
  let ptr = explicit_raw_com_pointer(value, "adoptOwnedComPointer")?;
  adopt_owned_com_pointer_bits(ptr, iid)
}

fn adopt_owned_com_pointer_bits(
  ptr: *mut std::ffi::c_void,
  iid: &WinGUID,
) -> napi::Result<DynWinRTValue> {
  if ptr.is_null() {
    return Ok(DynWinRTValue::new(dynwinrt::WinRTValue::Null));
  }
  unsafe { dynwinrt::com::adopt_com_pointer(ptr) }
    .cast(&iid.0)
    .map(DynWinRTValue::new)
    .map_err(|error| napi::Error::from_reason(error.message()))
    .and_then(|mut value| {
      value.bind_current_com_apartment()?;
      Ok(value)
    })
}

fn unsafe_borrow_com_pointer(value: Unknown, iid: &WinGUID) -> napi::Result<DynWinRTValue> {
  let ptr = explicit_raw_com_pointer(value, "borrowComPointer")?;
  borrow_com_pointer_bits(ptr, iid)
}

fn borrow_com_pointer_bits(
  ptr: *mut std::ffi::c_void,
  iid: &WinGUID,
) -> napi::Result<DynWinRTValue> {
  if ptr.is_null() {
    return Ok(DynWinRTValue::new(dynwinrt::WinRTValue::Null));
  }
  let borrowed = unsafe { windows::core::IUnknown::from_raw_borrowed(&ptr) }
    .ok_or_else(|| napi::Error::from_reason("borrowComPointer(): pointer must be non-null"))?;
  dynwinrt::WinRTValue::Object(borrowed.clone())
    .cast(&iid.0)
    .map(DynWinRTValue::new)
    .map_err(|error| napi::Error::from_reason(error.message()))
    .and_then(|mut value| {
      value.bind_current_com_apartment()?;
      Ok(value)
    })
}

fn adopt_co_task_mem_pointer(value: &mut DynWinRTValue) -> napi::Result<DynWinRTValue> {
  let ptr = take_native_output_pointer(
    value,
    PointerProvenance::CoTaskMemOutput,
    "CoTaskMem allocation",
  )?;
  if ptr.is_null() {
    return Ok(DynWinRTValue::new(dynwinrt::WinRTValue::Null));
  }
  Ok(DynWinRTValue::with_pointer_owner(
    dynwinrt::WinRTValue::RawPtr(ptr),
    NativePointerOwner::CoTaskMem(ptr),
  ))
}

fn as_pointer_bigint(value: &DynWinRTValue) -> napi::Result<BigInt> {
  validate_pointer_owner(value)?;
  let bits = match &value.0 {
    dynwinrt::WinRTValue::Object(_) => {
      return Err(napi::Error::from_reason(
        "Managed COM objects cannot be exported as raw pointer addresses",
      ));
    }
    dynwinrt::WinRTValue::RawPtr(ptr) => *ptr as usize,
    dynwinrt::WinRTValue::Null => 0,
    _ => {
      return Err(napi::Error::from_reason(
        "Value is not a pointer or COM object",
      ));
    }
  };
  Ok(BigInt::from(bits as u64))
}

fn take_co_task_mem_wide_string(value: &mut DynWinRTValue) -> napi::Result<String> {
  let ptr = take_native_output_pointer(value, PointerProvenance::CoTaskMemOutput, "wide-string")?;
  if ptr.is_null() {
    return Ok(String::new());
  }
  let result = unsafe { windows::core::PCWSTR(ptr.cast()).to_string() }
    .map_err(|error| napi::Error::from_reason(error.to_string()));
  unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(ptr)) };
  result
}

fn take_co_task_mem_ansi_string(value: &mut DynWinRTValue) -> napi::Result<String> {
  let ptr = take_native_output_pointer(value, PointerProvenance::CoTaskMemOutput, "ANSI-string")?;
  if ptr.is_null() {
    return Ok(String::new());
  }
  let result = unsafe { windows::core::PCSTR(ptr.cast()).to_string() }
    .map_err(|error| napi::Error::from_reason(error.to_string()));
  unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(ptr)) };
  result
}

fn take_bstr(value: &mut DynWinRTValue) -> napi::Result<String> {
  let ptr = take_native_output_pointer(value, PointerProvenance::BstrOutput, "BSTR")?;
  if ptr.is_null() {
    return Ok(String::new());
  }
  let value = unsafe { windows::core::BSTR::from_raw(ptr.cast()) };
  String::try_from(&value).map_err(|error| napi::Error::from_reason(error.to_string()))
}

fn validate_pointer_owner(value: &DynWinRTValue) -> napi::Result<()> {
  if let Some(owner) = &value.1 {
    owner.validate()?;
  }
  Ok(())
}

fn take_native_output_pointer(
  value: &mut DynWinRTValue,
  expected: PointerProvenance,
  description: &str,
) -> napi::Result<*mut std::ffi::c_void> {
  if value.1.is_some() {
    return Err(napi::Error::from_reason(format!(
      "Cannot consume an owner-backed {description} pointer"
    )));
  }
  if value.2 != expected {
    return Err(napi::Error::from_reason(format!(
      "only owned native outputs may be consumed: cannot consume {description} pointer with {:?} provenance; expected {:?}",
      value.2, expected
    )));
  }
  match std::mem::replace(&mut value.0, dynwinrt::WinRTValue::Null) {
    dynwinrt::WinRTValue::RawPtr(ptr) => {
      value.2 = PointerProvenance::None;
      Ok(ptr)
    }
    dynwinrt::WinRTValue::Null => {
      value.2 = PointerProvenance::None;
      Ok(std::ptr::null_mut())
    }
    other => {
      value.0 = other;
      Err(napi::Error::from_reason(format!(
        "Expected a {description} raw pointer"
      )))
    }
  }
}

fn iid_pointer(value: &WinGUID) -> DynWinRTValue {
  // Owner-backed: the boxed GUID is freed when the returned DynWinRtValue is
  // dropped / GC'd, instead of being leaked into a process-lifetime cache. The
  // REFIID is only read during the synchronous COM call the value is passed to,
  // and the JS temporary holding it outlives that call, so this is safe.
  let ptr = Box::into_raw(Box::new(value.0));
  DynWinRTValue::with_pointer_owner(
    dynwinrt::WinRTValue::RawPtr(ptr as *mut std::ffi::c_void),
    NativePointerOwner::Guid(ptr),
  )
}

fn native_struct_layout(
  descriptor: &str,
) -> napi::Result<std::sync::Arc<dynwinrt::com::NativeStructLayout>> {
  let root: serde_json::Value = serde_json::from_str(descriptor).map_err(|error| {
    napi::Error::from_reason(format!("Invalid native struct descriptor: {error}"))
  })?;
  let name = root
    .get("name")
    .and_then(serde_json::Value::as_str)
    .ok_or_else(|| napi::Error::from_reason("Native struct descriptor is missing `name`"))?;
  #[cfg(target_arch = "x86")]
  let architecture = "x86";
  #[cfg(target_arch = "x86_64")]
  let architecture = "x64";
  #[cfg(target_arch = "aarch64")]
  let architecture = "arm64";
  #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
  return Err(napi::Error::from_reason(
    "Classic COM native structs support only x86, x64, and ARM64",
  ));
  let layout = root.get(architecture).ok_or_else(|| {
    napi::Error::from_reason(format!(
      "Native struct descriptor is missing `{architecture}`"
    ))
  })?;
  let parsed = parse_native_struct_variant(name, layout)?;
  let mut parsed = std::sync::Arc::try_unwrap(parsed)
    .map_err(|_| napi::Error::from_reason("Native struct layout is unexpectedly shared"))?;
  let initializers = root
    .get("initializers")
    .map(|value| {
      value
        .as_array()
        .ok_or_else(|| napi::Error::from_reason("Native struct `initializers` must be an array"))
    })
    .transpose()?;
  for initializer in initializers.into_iter().flatten() {
    let kind = initializer
      .get("kind")
      .and_then(serde_json::Value::as_str)
      .ok_or_else(|| napi::Error::from_reason("Native struct initializer is missing `kind`"))?;
    let field = initializer
      .get("field")
      .and_then(serde_json::Value::as_str)
      .ok_or_else(|| napi::Error::from_reason("Native struct initializer is missing `field`"))?;
    parsed = match kind {
      "sizeOfLayout" => parsed
        .with_size_field_initializer(field)
        .map_err(|error| napi::Error::from_reason(error.message()))?,
      _ => {
        return Err(napi::Error::from_reason(format!(
          "Unsupported native struct initializer `{kind}`"
        )));
      }
    };
  }
  Ok(std::sync::Arc::new(parsed))
}

fn native_union_layout(
  descriptor: &str,
) -> napi::Result<std::sync::Arc<dynwinrt::com::NativeUnionLayout>> {
  let root: serde_json::Value = serde_json::from_str(descriptor).map_err(|error| {
    napi::Error::from_reason(format!("Invalid native union descriptor: {error}"))
  })?;
  let name = root
    .get("name")
    .and_then(serde_json::Value::as_str)
    .ok_or_else(|| napi::Error::from_reason("Native union descriptor is missing `name`"))?;
  #[cfg(target_arch = "x86")]
  let architecture = "x86";
  #[cfg(target_arch = "x86_64")]
  let architecture = "x64";
  #[cfg(target_arch = "aarch64")]
  let architecture = "arm64";
  #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
  return Err(napi::Error::from_reason(
    "Classic COM native unions support only x86, x64, and ARM64",
  ));
  let layout = root.get(architecture).ok_or_else(|| {
    napi::Error::from_reason(format!(
      "Native union descriptor is missing `{architecture}`"
    ))
  })?;
  parse_native_union_variant(name, layout)
}

fn parse_native_union_variant(
  name: &str,
  layout: &serde_json::Value,
) -> napi::Result<std::sync::Arc<dynwinrt::com::NativeUnionLayout>> {
  let size = json_usize(layout, "size")?;
  let alignment = json_usize(layout, "alignment")?;
  let fields = layout
    .get("fields")
    .and_then(serde_json::Value::as_array)
    .ok_or_else(|| napi::Error::from_reason("Native union descriptor is missing `fields`"))?
    .iter()
    .map(|field| {
      let field_name = field
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| napi::Error::from_reason("Native union field is missing `name`"))?;
      let count = u32::try_from(json_usize(field, "count")?)
        .map_err(|_| napi::Error::from_reason("Native union field count exceeds u32"))?;
      let typ = field
        .get("type")
        .ok_or_else(|| napi::Error::from_reason("Native union field is missing `type`"))?;
      let kind = typ
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| napi::Error::from_reason("Native union field type is missing `kind`"))?;
      let typ = match kind {
        "i8" => dynwinrt::com::NativeUnionFieldType::Scalar(dynwinrt::com::NativeStructScalar::I8),
        "u8" => dynwinrt::com::NativeUnionFieldType::Scalar(dynwinrt::com::NativeStructScalar::U8),
        "i16" => {
          dynwinrt::com::NativeUnionFieldType::Scalar(dynwinrt::com::NativeStructScalar::I16)
        }
        "u16" => {
          dynwinrt::com::NativeUnionFieldType::Scalar(dynwinrt::com::NativeStructScalar::U16)
        }
        "i32" => {
          dynwinrt::com::NativeUnionFieldType::Scalar(dynwinrt::com::NativeStructScalar::I32)
        }
        "u32" => {
          dynwinrt::com::NativeUnionFieldType::Scalar(dynwinrt::com::NativeStructScalar::U32)
        }
        "i64" => {
          dynwinrt::com::NativeUnionFieldType::Scalar(dynwinrt::com::NativeStructScalar::I64)
        }
        "u64" => {
          dynwinrt::com::NativeUnionFieldType::Scalar(dynwinrt::com::NativeStructScalar::U64)
        }
        "f32" => {
          dynwinrt::com::NativeUnionFieldType::Scalar(dynwinrt::com::NativeStructScalar::F32)
        }
        "f64" => {
          dynwinrt::com::NativeUnionFieldType::Scalar(dynwinrt::com::NativeStructScalar::F64)
        }
        "isize" => {
          dynwinrt::com::NativeUnionFieldType::Scalar(dynwinrt::com::NativeStructScalar::ISize)
        }
        "usize" => {
          dynwinrt::com::NativeUnionFieldType::Scalar(dynwinrt::com::NativeStructScalar::USize)
        }
        "guid" => dynwinrt::com::NativeUnionFieldType::Guid,
        "pointer" => dynwinrt::com::NativeUnionFieldType::Pointer,
        "struct" => {
          let nested_name = typ
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| napi::Error::from_reason("Nested native struct is missing `name`"))?;
          let nested_layout = typ
            .get("layout")
            .ok_or_else(|| napi::Error::from_reason("Nested native struct is missing `layout`"))?;
          dynwinrt::com::NativeUnionFieldType::Struct(parse_native_struct_variant(
            nested_name,
            nested_layout,
          )?)
        }
        _ => {
          return Err(napi::Error::from_reason(format!(
            "Unsupported native union field kind `{kind}`"
          )));
        }
      };
      dynwinrt::com::NativeUnionField::new(field_name, count, typ)
        .map_err(|error| napi::Error::from_reason(error.message()))
    })
    .collect::<napi::Result<Vec<_>>>()?;
  dynwinrt::com::NativeUnionLayout::new(name, size, alignment, fields)
    .map(std::sync::Arc::new)
    .map_err(|error| napi::Error::from_reason(error.message()))
}

fn parse_native_struct_variant(
  name: &str,
  layout: &serde_json::Value,
) -> napi::Result<std::sync::Arc<dynwinrt::com::NativeStructLayout>> {
  let size = json_usize(layout, "size")?;
  let alignment = json_usize(layout, "alignment")?;
  let fields = layout
    .get("fields")
    .and_then(serde_json::Value::as_array)
    .ok_or_else(|| napi::Error::from_reason("Native struct descriptor is missing `fields`"))?
    .iter()
    .map(|field| {
      let field_name = field
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| napi::Error::from_reason("Native struct field is missing `name`"))?;
      let offset = json_usize(field, "offset")?;
      let count = u32::try_from(json_usize(field, "count")?)
        .map_err(|_| napi::Error::from_reason("Native struct field count exceeds u32"))?;
      let typ = field
        .get("type")
        .ok_or_else(|| napi::Error::from_reason("Native struct field is missing `type`"))?;
      let kind = typ
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| napi::Error::from_reason("Native struct field type is missing `kind`"))?;
      let typ = match kind {
        "i8" => dynwinrt::com::NativeStructFieldType::Scalar(dynwinrt::com::NativeStructScalar::I8),
        "u8" => dynwinrt::com::NativeStructFieldType::Scalar(dynwinrt::com::NativeStructScalar::U8),
        "i16" => {
          dynwinrt::com::NativeStructFieldType::Scalar(dynwinrt::com::NativeStructScalar::I16)
        }
        "u16" => {
          dynwinrt::com::NativeStructFieldType::Scalar(dynwinrt::com::NativeStructScalar::U16)
        }
        "i32" => {
          dynwinrt::com::NativeStructFieldType::Scalar(dynwinrt::com::NativeStructScalar::I32)
        }
        "u32" => {
          dynwinrt::com::NativeStructFieldType::Scalar(dynwinrt::com::NativeStructScalar::U32)
        }
        "i64" => {
          dynwinrt::com::NativeStructFieldType::Scalar(dynwinrt::com::NativeStructScalar::I64)
        }
        "u64" => {
          dynwinrt::com::NativeStructFieldType::Scalar(dynwinrt::com::NativeStructScalar::U64)
        }
        "f32" => {
          dynwinrt::com::NativeStructFieldType::Scalar(dynwinrt::com::NativeStructScalar::F32)
        }
        "f64" => {
          dynwinrt::com::NativeStructFieldType::Scalar(dynwinrt::com::NativeStructScalar::F64)
        }
        "isize" => {
          dynwinrt::com::NativeStructFieldType::Scalar(dynwinrt::com::NativeStructScalar::ISize)
        }
        "usize" => {
          dynwinrt::com::NativeStructFieldType::Scalar(dynwinrt::com::NativeStructScalar::USize)
        }
        "guid" => dynwinrt::com::NativeStructFieldType::Guid,
        "pointer" => dynwinrt::com::NativeStructFieldType::Pointer,
        "struct" => {
          let nested_name = typ
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| napi::Error::from_reason("Nested native struct is missing `name`"))?;
          let nested_layout = typ
            .get("layout")
            .ok_or_else(|| napi::Error::from_reason("Nested native struct is missing `layout`"))?;
          dynwinrt::com::NativeStructFieldType::Struct(parse_native_struct_variant(
            nested_name,
            nested_layout,
          )?)
        }
        _ => {
          return Err(napi::Error::from_reason(format!(
            "Unsupported native struct field kind `{kind}`"
          )));
        }
      };
      dynwinrt::com::NativeStructField::new(field_name, offset, count, typ)
        .map_err(|error| napi::Error::from_reason(error.message()))
    })
    .collect::<napi::Result<Vec<_>>>()?;
  dynwinrt::com::NativeStructLayout::new(name, size, alignment, fields)
    .map(std::sync::Arc::new)
    .map_err(|error| napi::Error::from_reason(error.message()))
}

fn json_usize(value: &serde_json::Value, name: &str) -> napi::Result<usize> {
  value
    .get(name)
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .ok_or_else(|| {
      napi::Error::from_reason(format!("Native struct descriptor has invalid `{name}`"))
    })
}

fn com_error(error: dynwinrt::Error) -> napi::Error {
  napi::Error::from_reason(error.message())
}

fn bigint_i64(value: BigInt, name: &str) -> napi::Result<i64> {
  let (value, lossless) = value.get_i64();
  if lossless {
    Ok(value)
  } else {
    Err(napi::Error::from_reason(format!(
      "{name} value must fit in a signed 64-bit integer"
    )))
  }
}

fn bigint_u64(value: BigInt, name: &str) -> napi::Result<u64> {
  let (negative, value, lossless) = value.get_u64();
  if !negative && lossless {
    Ok(value)
  } else {
    Err(napi::Error::from_reason(format!(
      "{name} value must fit in an unsigned 64-bit integer"
    )))
  }
}

fn checked_signed_number(value: f64, min: i64, max: i64, name: &str) -> napi::Result<i64> {
  if !value.is_finite() || value.fract() != 0.0 || value < min as f64 || value > max as f64 {
    return Err(napi::Error::from_reason(format!(
      "{name} value is out of range or not an integral number ({min}..={max})"
    )));
  }
  Ok(value as i64)
}

fn checked_unsigned_number(value: f64, max: u64, name: &str) -> napi::Result<u64> {
  if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value > max as f64 {
    return Err(napi::Error::from_reason(format!(
      "{name} value is out of range or not an integral number (0..={max})"
    )));
  }
  Ok(value as u64)
}

fn optional_com_object(
  value: Option<&DynWinRTValue>,
  name: &str,
) -> napi::Result<Option<windows::core::IUnknown>> {
  value
    .map(|value| {
      value
        .0
        .as_object()
        .ok_or_else(|| napi::Error::from_reason(format!("{name} requires a COM object or null")))
    })
    .transpose()
}

fn variant_kind(typ: dynwinrt::com::VariantType) -> &'static str {
  match typ {
    dynwinrt::com::VariantType::Empty => "empty",
    dynwinrt::com::VariantType::Null => "null",
    dynwinrt::com::VariantType::I8 => "i8",
    dynwinrt::com::VariantType::U8 => "u8",
    dynwinrt::com::VariantType::I16 => "i16",
    dynwinrt::com::VariantType::U16 => "u16",
    dynwinrt::com::VariantType::I32 => "i32",
    dynwinrt::com::VariantType::U32 => "u32",
    dynwinrt::com::VariantType::I64 => "i64",
    dynwinrt::com::VariantType::U64 => "u64",
    dynwinrt::com::VariantType::Int => "int",
    dynwinrt::com::VariantType::UInt => "uint",
    dynwinrt::com::VariantType::F32 => "f32",
    dynwinrt::com::VariantType::F64 => "f64",
    dynwinrt::com::VariantType::Bool => "bool",
    dynwinrt::com::VariantType::Bstr => "bstr",
    dynwinrt::com::VariantType::Unknown => "unknown",
    dynwinrt::com::VariantType::Dispatch => "dispatch",
    dynwinrt::com::VariantType::SafeArray(_) => "safeArray",
  }
}

fn safe_array_element_kind(typ: dynwinrt::com::SafeArrayElementType) -> &'static str {
  match typ {
    dynwinrt::com::SafeArrayElementType::I8 => "i8",
    dynwinrt::com::SafeArrayElementType::U8 => "u8",
    dynwinrt::com::SafeArrayElementType::I16 => "i16",
    dynwinrt::com::SafeArrayElementType::U16 => "u16",
    dynwinrt::com::SafeArrayElementType::I32 => "i32",
    dynwinrt::com::SafeArrayElementType::U32 => "u32",
    dynwinrt::com::SafeArrayElementType::I64 => "i64",
    dynwinrt::com::SafeArrayElementType::U64 => "u64",
    dynwinrt::com::SafeArrayElementType::F32 => "f32",
    dynwinrt::com::SafeArrayElementType::F64 => "f64",
    dynwinrt::com::SafeArrayElementType::Bool => "bool",
    dynwinrt::com::SafeArrayElementType::Bstr => "bstr",
    dynwinrt::com::SafeArrayElementType::Unknown => "unknown",
    dynwinrt::com::SafeArrayElementType::Dispatch => "dispatch",
    dynwinrt::com::SafeArrayElementType::Variant => "variant",
  }
}

fn safe_array_element_type_from_name(
  name: &str,
) -> napi::Result<dynwinrt::com::SafeArrayElementType> {
  Ok(match name {
    "i8" => dynwinrt::com::SafeArrayElementType::I8,
    "u8" => dynwinrt::com::SafeArrayElementType::U8,
    "i16" => dynwinrt::com::SafeArrayElementType::I16,
    "u16" => dynwinrt::com::SafeArrayElementType::U16,
    "i32" => dynwinrt::com::SafeArrayElementType::I32,
    "u32" => dynwinrt::com::SafeArrayElementType::U32,
    "i64" => dynwinrt::com::SafeArrayElementType::I64,
    "u64" => dynwinrt::com::SafeArrayElementType::U64,
    "f32" => dynwinrt::com::SafeArrayElementType::F32,
    "f64" => dynwinrt::com::SafeArrayElementType::F64,
    "bool" => dynwinrt::com::SafeArrayElementType::Bool,
    "bstr" => dynwinrt::com::SafeArrayElementType::Bstr,
    "unknown" => dynwinrt::com::SafeArrayElementType::Unknown,
    "dispatch" => dynwinrt::com::SafeArrayElementType::Dispatch,
    "variant" => dynwinrt::com::SafeArrayElementType::Variant,
    _ => {
      return Err(napi::Error::from_reason(format!(
        "Unsupported SAFEARRAY element type `{name}`"
      )));
    }
  })
}

#[napi]
pub struct DynComType(dynwinrt::com::Type);

#[napi]
pub struct DynComMethodSig(dynwinrt::com::MethodSignature);

#[napi]
impl DynComMethodSig {
  #[napi(constructor)]
  pub fn new() -> Self {
    Self(dynwinrt::com::MethodSignature::new(&TABLE))
  }

  #[napi]
  pub fn add_in(&self, typ: &DynComType) -> Self {
    Self(self.0.clone().add_in(typ.0.clone()))
  }

  #[napi]
  pub fn add_nullable_in(&self, typ: &DynComType) -> Self {
    Self(self.0.clone().add_nullable_in(typ.0.clone()))
  }

  #[napi]
  pub fn add_out(&self, typ: &DynComType) -> Self {
    Self(self.0.clone().add_out(typ.0.clone()))
  }

  #[napi]
  pub fn add_optional_out(&self, typ: &DynComType) -> Self {
    Self(self.0.clone().add_optional_out(typ.0.clone()))
  }

  #[napi]
  pub fn capture_dispatch_invoke_hresult(&self) -> Self {
    Self(self.0.clone().capture_dispatch_invoke_hresult())
  }

  #[napi]
  pub fn add_in_out(&self, typ: &DynComType) -> Self {
    Self(self.0.clone().add_in_out(typ.0.clone()))
  }

  #[napi]
  pub fn add_nullable_in_out(&self, typ: &DynComType) -> Self {
    Self(self.0.clone().add_nullable_in_out(typ.0.clone()))
  }

  #[napi]
  pub fn add_out_fill(&self, typ: &DynComType) -> Self {
    Self(self.0.clone().add_out_fill(typ.0.clone()))
  }

  #[napi]
  pub fn add_input_buffer(
    &self,
    element_type: &DynComType,
    count_param_index: u32,
    actual_length_param_index: Option<u32>,
    count_in_bytes: bool,
  ) -> napi::Result<Self> {
    self
      .0
      .clone()
      .add_input_buffer(
        element_type.0.clone(),
        count_param_index as usize,
        actual_length_param_index.map(|index| index as usize),
        if count_in_bytes {
          dynwinrt::com::BufferCountUnit::Bytes
        } else {
          dynwinrt::com::BufferCountUnit::Elements
        },
      )
      .map(Self)
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  #[napi]
  pub fn add_input_string_array(&self, wide: bool, count_param_index: u32) -> napi::Result<Self> {
    self
      .0
      .clone()
      .add_input_string_array(
        if wide {
          dynwinrt::com::StringEncoding::Utf16
        } else {
          dynwinrt::com::StringEncoding::Ansi
        },
        count_param_index as usize,
      )
      .map(Self)
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  #[napi]
  pub fn add_caller_output_buffer(
    &self,
    element_type: &DynComType,
    capacity_param_index: u32,
    actual_length_param_index: Option<u32>,
    count_in_bytes: bool,
    two_call: bool,
  ) -> napi::Result<Self> {
    self
      .0
      .clone()
      .add_caller_output_buffer(
        element_type.0.clone(),
        capacity_param_index as usize,
        actual_length_param_index.map(|index| index as usize),
        if count_in_bytes {
          dynwinrt::com::BufferCountUnit::Bytes
        } else {
          dynwinrt::com::BufferCountUnit::Elements
        },
        two_call,
      )
      .map(Self)
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  #[napi]
  pub fn add_enumerator_next_buffer(
    &self,
    element_type: &DynComType,
    capacity_param_index: u32,
    fetched_param_index: u32,
  ) -> napi::Result<Self> {
    self
      .0
      .clone()
      .add_enumerator_next_buffer(
        element_type.0.clone(),
        capacity_param_index as usize,
        fetched_param_index as usize,
      )
      .map(Self)
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  #[napi]
  pub fn add_co_task_mem_output_buffer(
    &self,
    element_type: &DynComType,
    count_param_index: u32,
    count_in_bytes: bool,
  ) -> napi::Result<Self> {
    self
      .0
      .clone()
      .add_callee_allocated_buffer(
        element_type.0.clone(),
        count_param_index as usize,
        if count_in_bytes {
          dynwinrt::com::BufferCountUnit::Bytes
        } else {
          dynwinrt::com::BufferCountUnit::Elements
        },
        dynwinrt::com::BufferAllocator::CoTaskMem,
      )
      .map(Self)
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  #[napi]
  pub fn returns(&self, typ: &DynComType) -> Self {
    Self(self.0.clone().returns(typ.0.clone()))
  }

  #[napi]
  pub fn returns_void(&self) -> Self {
    Self(self.0.clone().returns_void())
  }

  #[napi]
  pub fn preserve_hresult(&self) -> Self {
    Self(self.0.clone().preserve_hresult())
  }

  #[napi]
  pub fn preserve_enumerator_next_hresult(&self) -> Self {
    Self(self.0.clone().preserve_enumerator_next_hresult())
  }

  #[napi]
  pub fn preserve_enumerator_next_hresult_at(&self, vtable_index: u32) -> Self {
    Self(
      self
        .0
        .clone()
        .preserve_enumerator_next_hresult_at(vtable_index as usize),
    )
  }
}

#[napi]
pub struct DynComInterface(dynwinrt::com::Interface);

#[napi]
impl DynComInterface {
  #[napi]
  pub fn add_method(&self, name: String, signature: &DynComMethodSig) -> Self {
    Self(self.0.clone().add_method(&name, signature.0.clone()))
  }

  #[napi]
  pub fn add_method_at(
    &self,
    vtable_index: u32,
    name: String,
    signature: &DynComMethodSig,
  ) -> napi::Result<Self> {
    self
      .0
      .clone()
      .add_method_at(vtable_index as usize, &name, signature.0.clone())
      .map(Self)
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  #[napi]
  pub fn method(&self, vtable_index: i32) -> napi::Result<DynComMethodHandle> {
    self
      .0
      .method(vtable_index as usize)
      .map(DynComMethodHandle)
      .ok_or_else(|| {
        napi::Error::from_reason(format!("No COM method at vtable index {vtable_index}"))
      })
  }
}

#[napi]
pub struct DynComUnsafeInterface(dynwinrt::com::Interface);

#[napi]
impl DynComUnsafeInterface {
  #[napi]
  pub fn add_method_at(
    &self,
    vtable_index: u32,
    name: String,
    signature: &DynComMethodSig,
  ) -> napi::Result<Self> {
    self
      .0
      .clone()
      .add_method_at(vtable_index as usize, &name, signature.0.clone())
      .map(Self)
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  #[napi]
  pub fn method(&self, vtable_index: i32) -> napi::Result<DynComMethodHandle> {
    self
      .0
      .method(vtable_index as usize)
      .map(DynComMethodHandle)
      .ok_or_else(|| {
        napi::Error::from_reason(format!("No COM method at vtable index {vtable_index}"))
      })
  }
}

#[napi]
pub struct DynComMethodHandle(dynwinrt::com::MethodHandle);

#[napi]
pub struct DynComDispatchInvokeResult {
  hresult: i32,
  result: Option<DynWinRTValue>,
  excep_info: Option<DynWinRTValue>,
  arg_err: Option<u32>,
  finalization_error: Option<String>,
}

#[napi]
impl DynComDispatchInvokeResult {
  #[napi(getter)]
  pub fn hresult(&self) -> i32 {
    self.hresult
  }

  #[napi(getter)]
  pub fn arg_err(&self) -> Option<u32> {
    self.arg_err
  }

  #[napi(getter)]
  pub fn finalization_error(&self) -> Option<String> {
    self.finalization_error.clone()
  }

  #[napi]
  pub fn take_result(&mut self) -> Option<DynWinRTValue> {
    self.result.take()
  }

  #[napi]
  pub fn take_excep_info(&mut self) -> Option<DynWinRTValue> {
    self.excep_info.take()
  }
}

#[napi]
impl DynComMethodHandle {
  #[napi]
  pub fn get_string(&self, obj: &DynWinRTValue) -> napi::Result<String> {
    obj.ensure_com_apartment()?;
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("getString() requires a COM object"))?
      .as_raw();
    unsafe { self.0.call_getter_hstring(raw) }
      .map(|value| value.to_string())
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  #[napi]
  pub fn invoke(
    &self,
    obj: &DynWinRTValue,
    args: Vec<&DynWinRTValue>,
  ) -> napi::Result<DynWinRTValue> {
    obj.ensure_com_apartment()?;
    if self.0.result_count() > 1 {
      return Err(napi::Error::from_reason(
        "invoke() cannot discard multiple COM results; use invokeAll()",
      ));
    }
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("invoke() requires a COM object"))?
      .as_raw();
    for arg in &args {
      validate_pointer_owner(arg)?;
    }
    let args = args
      .iter()
      .map(|arg| arg.to_com_value())
      .collect::<napi::Result<Vec<_>>>()?;
    let mut results = unsafe { self.0.invoke_values_with_output_kinds(raw, &args) }
      .map_err(|error| napi::Error::from_reason(error.message()))?;
    let (value, kind) = results.drain(..).next().unwrap_or((
      dynwinrt::com::Value::WinRt(dynwinrt::WinRTValue::I32(0)),
      dynwinrt::com::PointerOutputKind::None,
    ));
    Ok(DynWinRTValue::from_com_value(value, kind))
  }

  #[napi]
  pub fn invoke_all(
    &self,
    obj: &DynWinRTValue,
    args: Vec<&DynWinRTValue>,
  ) -> napi::Result<Vec<DynWinRTValue>> {
    obj.ensure_com_apartment()?;
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("invokeAll() requires a COM object"))?
      .as_raw();
    for arg in &args {
      validate_pointer_owner(arg)?;
    }
    let args = args
      .iter()
      .map(|arg| arg.to_com_value())
      .collect::<napi::Result<Vec<_>>>()?;
    unsafe { self.0.invoke_values_with_output_kinds(raw, &args) }
      .map(|results| {
        results
          .into_iter()
          .map(|(value, kind)| DynWinRTValue::from_com_value(value, kind))
          .collect()
      })
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  #[napi]
  pub fn invoke_dispatch(
    &self,
    obj: &DynWinRTValue,
    args: Vec<&DynWinRTValue>,
  ) -> napi::Result<DynComDispatchInvokeResult> {
    obj.ensure_com_apartment()?;
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("invokeDispatch() requires a COM object"))?
      .as_raw();
    for arg in &args {
      validate_pointer_owner(arg)?;
    }
    let args = args
      .iter()
      .map(|arg| arg.to_com_value())
      .collect::<napi::Result<Vec<_>>>()?;
    let result = unsafe { self.0.invoke_dispatch(raw, &args) }.map_err(com_error)?;
    let (hresult, result, excep_info, arg_err, finalization_error) = result.into_parts();
    Ok(DynComDispatchInvokeResult {
      hresult: hresult.0,
      result: result.map(|value| {
        DynWinRTValue::from_com_value(
          dynwinrt::com::Value::Variant(value),
          dynwinrt::com::PointerOutputKind::None,
        )
      }),
      excep_info: excep_info.map(|value| {
        DynWinRTValue::from_com_value(
          dynwinrt::com::Value::ExcepInfo(value),
          dynwinrt::com::PointerOutputKind::None,
        )
      }),
      arg_err,
      finalization_error: finalization_error.map(|error| error.message()),
    })
  }
}

#[napi]
pub struct DynCom;

/// Explicit opt-in surface for manually declared native COM ABI contracts.
///
/// Supplying an invalid pointer, IID, vtable slot, direction, count relation,
/// or ownership contract can crash the process or corrupt memory.
#[napi]
pub struct DynComUnsafe;

#[napi]
impl DynComUnsafe {
  #[napi(js_name = "registerIUnknownInterface")]
  pub fn register_iunknown_interface(name: String, iid: &WinGUID) -> DynComUnsafeInterface {
    DynComUnsafeInterface(dynwinrt::com::register_interface(
      &TABLE,
      &name,
      iid.0,
      dynwinrt::com::InterfaceBase::IUnknown,
    ))
  }

  #[napi(js_name = "registerIInspectableInterface")]
  pub fn register_iinspectable_interface(name: String, iid: &WinGUID) -> DynComUnsafeInterface {
    DynComUnsafeInterface(dynwinrt::com::register_interface(
      &TABLE,
      &name,
      iid.0,
      dynwinrt::com::InterfaceBase::IInspectable,
    ))
  }

  #[napi]
  pub fn interface_type(iid: &WinGUID) -> DynComType {
    DynComType(dynwinrt::com::Type::winrt(TABLE.interface(iid.0)))
  }

  #[napi]
  pub fn unclassified_pointer_type() -> DynComType {
    DynComType(dynwinrt::com::Type::pointer())
  }

  #[napi]
  pub fn borrowed_handle_output_type() -> DynComType {
    DynComType(dynwinrt::com::Type::borrowed_handle_output())
  }

  #[napi]
  pub fn owned_com_output_type() -> DynComType {
    DynComType(dynwinrt::com::Type::owned_com_pointer())
  }

  #[napi]
  pub fn co_task_mem_output_type() -> DynComType {
    DynComType(dynwinrt::com::Type::co_task_mem_pointer())
  }

  #[napi]
  pub fn bstr_output_type() -> DynComType {
    DynComType(dynwinrt::com::Type::bstr_pointer())
  }

  #[napi]
  pub fn pointer(
    #[napi(ts_arg_type = "bigint | number | Buffer | Uint8Array | null | undefined")]
    value: Unknown,
  ) -> napi::Result<DynWinRTValue> {
    self::pointer(value)
  }

  #[napi]
  pub fn wide_string_pointer(
    #[napi(ts_arg_type = "string | bigint | number | Buffer | Uint8Array | null | undefined")]
    value: Unknown,
  ) -> napi::Result<DynWinRTValue> {
    self::wide_string_pointer(value)
  }

  #[napi]
  pub fn ansi_string_pointer(
    #[napi(ts_arg_type = "string | bigint | number | Buffer | Uint8Array | null | undefined")]
    value: Unknown,
  ) -> napi::Result<DynWinRTValue> {
    self::ansi_string_pointer(value)
  }

  #[napi]
  pub fn iid_pointer(value: &WinGUID) -> DynWinRTValue {
    self::iid_pointer(value)
  }

  #[napi]
  pub fn handle_value(
    #[napi(ts_arg_type = "bigint | number | Buffer | Uint8Array")] value: Unknown,
  ) -> napi::Result<BigInt> {
    self::handle_value(value)
  }

  #[napi]
  pub fn co_create_instance(clsid: String, iid: &WinGUID) -> napi::Result<DynWinRTValue> {
    self::co_create_instance(clsid, iid)
  }

  #[napi]
  pub fn co_get_class_object(clsid: String, iid: &WinGUID) -> napi::Result<DynWinRTValue> {
    self::co_get_class_object(clsid, iid)
  }

  #[napi]
  pub fn co_get_malloc() -> napi::Result<DynWinRTValue> {
    self::co_get_malloc()
  }

  #[napi]
  pub fn create_error_info() -> napi::Result<DynWinRTValue> {
    self::create_error_info()
  }

  #[napi]
  pub fn set_error_info(
    #[napi(ts_arg_type = "DynWinRtValue | null | undefined")] value: Option<&DynWinRTValue>,
  ) -> napi::Result<()> {
    self::set_error_info(value)
  }

  #[napi]
  pub fn get_error_info() -> napi::Result<Option<DynWinRTValue>> {
    self::get_error_info()
  }

  /// Takes ownership of one caller-supplied +1 COM reference.
  #[napi]
  pub fn adopt_owned_com_pointer(
    #[napi(ts_arg_type = "bigint | number | null | undefined")] value: Unknown,
    iid: &WinGUID,
  ) -> napi::Result<DynWinRTValue> {
    unsafe_adopt_owned_com_pointer(value, iid)
  }

  /// Queries a caller-owned borrowed pointer and returns a new managed +1 reference.
  #[napi]
  pub fn borrow_com_pointer(
    #[napi(ts_arg_type = "bigint | number | null | undefined")] value: Unknown,
    iid: &WinGUID,
  ) -> napi::Result<DynWinRTValue> {
    unsafe_borrow_com_pointer(value, iid)
  }
}

#[napi]
pub struct DynComNativeStruct {
  descriptor: String,
  bytes: Vec<u8>,
}

#[napi]
pub struct DynComNativeStructArray {
  descriptor: String,
  bytes: Vec<u8>,
}

#[napi]
impl DynComNativeStructArray {
  #[napi(getter)]
  pub fn length(&self) -> u32 {
    self.bytes.len() as u32
  }

  #[napi(getter)]
  pub fn bytes(&self) -> Buffer {
    Buffer::from(self.bytes.clone())
  }
}

#[napi]
pub struct DynComNativeUnion {
  descriptor: String,
  value: dynwinrt::com::NativeUnionValue,
}

#[napi]
impl DynComNativeUnion {
  #[napi(getter)]
  pub fn active_field(&self) -> String {
    self.value.active_field().into()
  }

  #[napi(getter)]
  pub fn length(&self) -> u32 {
    self.value.bytes().len() as u32
  }

  #[napi(getter)]
  pub fn bytes(&self) -> Buffer {
    Buffer::from(self.value.bytes().to_vec())
  }
}

#[napi]
pub struct DynComVariant {
  value: Option<dynwinrt::com::VariantValue>,
}

impl DynComVariant {
  fn new(value: dynwinrt::com::VariantValue) -> Self {
    Self { value: Some(value) }
  }

  fn value(&self) -> napi::Result<&dynwinrt::com::VariantValue> {
    self
      .value
      .as_ref()
      .ok_or_else(|| napi::Error::from_reason("VARIANT has been released"))
  }
}

impl Drop for DynComVariant {
  fn drop(&mut self) {
    if super::winui_dispatcher_loop_exited() {
      if let Some(value) = self.value.take() {
        std::mem::forget(value);
      }
    }
  }
}

#[napi]
impl DynComVariant {
  #[napi]
  pub fn empty() -> Self {
    Self::new(dynwinrt::com::VariantValue::empty())
  }

  #[napi(js_name = "null")]
  pub fn null_value() -> Self {
    Self::new(dynwinrt::com::VariantValue::null())
  }

  #[napi]
  pub fn i8(value: f64) -> napi::Result<Self> {
    checked_signed_number(value, i8::MIN as i64, i8::MAX as i64, "VARIANT VT_I1")
      .map(|value| Self::new(dynwinrt::com::VariantValue::from_i8(value as i8)))
  }

  #[napi]
  pub fn u8(value: f64) -> napi::Result<Self> {
    checked_unsigned_number(value, u8::MAX as u64, "VARIANT VT_UI1")
      .map(|value| Self::new(dynwinrt::com::VariantValue::from_u8(value as u8)))
  }

  #[napi]
  pub fn i16(value: f64) -> napi::Result<Self> {
    checked_signed_number(value, i16::MIN as i64, i16::MAX as i64, "VARIANT VT_I2")
      .map(|value| Self::new(dynwinrt::com::VariantValue::from_i16(value as i16)))
  }

  #[napi]
  pub fn u16(value: f64) -> napi::Result<Self> {
    checked_unsigned_number(value, u16::MAX as u64, "VARIANT VT_UI2")
      .map(|value| Self::new(dynwinrt::com::VariantValue::from_u16(value as u16)))
  }

  #[napi]
  pub fn i32(value: f64) -> napi::Result<Self> {
    checked_signed_number(value, i32::MIN as i64, i32::MAX as i64, "VARIANT VT_I4")
      .map(|value| Self::new(dynwinrt::com::VariantValue::from_i32(value as i32)))
  }

  #[napi]
  pub fn u32(value: f64) -> napi::Result<Self> {
    checked_unsigned_number(value, u32::MAX as u64, "VARIANT VT_UI4")
      .map(|value| Self::new(dynwinrt::com::VariantValue::from_u32(value as u32)))
  }

  #[napi]
  pub fn int(value: f64) -> napi::Result<Self> {
    checked_signed_number(value, i32::MIN as i64, i32::MAX as i64, "VARIANT VT_INT")
      .map(|value| Self::new(dynwinrt::com::VariantValue::from_int(value as i32)))
  }

  #[napi]
  pub fn uint(value: f64) -> napi::Result<Self> {
    checked_unsigned_number(value, u32::MAX as u64, "VARIANT VT_UINT")
      .map(|value| Self::new(dynwinrt::com::VariantValue::from_uint(value as u32)))
  }

  #[napi]
  pub fn i64(value: BigInt) -> napi::Result<Self> {
    bigint_i64(value, "VARIANT VT_I8")
      .map(dynwinrt::com::VariantValue::from_i64)
      .map(Self::new)
  }

  #[napi]
  pub fn u64(value: BigInt) -> napi::Result<Self> {
    bigint_u64(value, "VARIANT VT_UI8")
      .map(dynwinrt::com::VariantValue::from_u64)
      .map(Self::new)
  }

  #[napi]
  pub fn f32(value: f64) -> Self {
    Self::new(dynwinrt::com::VariantValue::from_f32(value as f32))
  }

  #[napi]
  pub fn f64(value: f64) -> Self {
    Self::new(dynwinrt::com::VariantValue::from_f64(value))
  }

  #[napi]
  pub fn bool(value: bool) -> Self {
    Self::new(dynwinrt::com::VariantValue::from_bool(value))
  }

  #[napi]
  pub fn bstr(value: String) -> napi::Result<Self> {
    dynwinrt::com::VariantValue::from_bstr(&value)
      .map(Self::new)
      .map_err(com_error)
  }

  #[napi]
  pub fn unknown(value: Option<&DynWinRTValue>) -> napi::Result<Self> {
    let value = optional_com_object(value, "VARIANT VT_UNKNOWN")?;
    Ok(Self::new(dynwinrt::com::VariantValue::from_unknown(
      value.as_ref(),
    )))
  }

  #[napi]
  pub fn dispatch(value: Option<&DynWinRTValue>) -> napi::Result<Self> {
    let value = optional_com_object(value, "VARIANT VT_DISPATCH")?;
    dynwinrt::com::VariantValue::from_dispatch(value.as_ref())
      .map(Self::new)
      .map_err(com_error)
  }

  #[napi]
  pub fn safe_array(value: &DynComSafeArray) -> napi::Result<Self> {
    dynwinrt::com::VariantValue::from_safe_array(value.value()?)
      .map(Self::new)
      .map_err(com_error)
  }

  #[napi(getter)]
  pub fn vartype(&self) -> napi::Result<u32> {
    Ok(u32::from(self.value()?.vartype()))
  }

  #[napi(getter)]
  pub fn kind(&self) -> napi::Result<String> {
    Ok(variant_kind(self.value()?.variant_type().map_err(com_error)?).into())
  }

  #[napi]
  pub fn to_number(&self) -> napi::Result<f64> {
    match self.value()?.data().map_err(com_error)? {
      dynwinrt::com::VariantData::I8(value) => Ok(f64::from(value)),
      dynwinrt::com::VariantData::U8(value) => Ok(f64::from(value)),
      dynwinrt::com::VariantData::I16(value) => Ok(f64::from(value)),
      dynwinrt::com::VariantData::U16(value) => Ok(f64::from(value)),
      dynwinrt::com::VariantData::I32(value) | dynwinrt::com::VariantData::Int(value) => {
        Ok(f64::from(value))
      }
      dynwinrt::com::VariantData::U32(value) | dynwinrt::com::VariantData::UInt(value) => {
        Ok(f64::from(value))
      }
      dynwinrt::com::VariantData::F32(value) => Ok(f64::from(value)),
      dynwinrt::com::VariantData::F64(value) => Ok(value),
      _ => Err(napi::Error::from_reason(
        "VARIANT does not contain a number-sized scalar",
      )),
    }
  }

  #[napi]
  pub fn to_bigint(&self) -> napi::Result<BigInt> {
    match self.value()?.data().map_err(com_error)? {
      dynwinrt::com::VariantData::I64(value) => Ok(BigInt::from(value)),
      dynwinrt::com::VariantData::U64(value) => Ok(BigInt::from(value)),
      _ => Err(napi::Error::from_reason(
        "VARIANT does not contain a 64-bit integer",
      )),
    }
  }

  #[napi]
  pub fn to_bool(&self) -> napi::Result<bool> {
    match self.value()?.data().map_err(com_error)? {
      dynwinrt::com::VariantData::Bool(value) => Ok(value),
      _ => Err(napi::Error::from_reason("VARIANT does not contain VT_BOOL")),
    }
  }

  #[napi(js_name = "toStringValue")]
  pub fn to_string_value(&self) -> napi::Result<String> {
    match self.value()?.data().map_err(com_error)? {
      dynwinrt::com::VariantData::Bstr(value) => Ok(value),
      _ => Err(napi::Error::from_reason("VARIANT does not contain VT_BSTR")),
    }
  }

  #[napi]
  pub fn to_interface(&self) -> napi::Result<Option<DynWinRTValue>> {
    match self.value()?.data().map_err(com_error)? {
      dynwinrt::com::VariantData::Unknown(value) | dynwinrt::com::VariantData::Dispatch(value) => {
        value.map(apartment_bound_com_object).transpose()
      }
      _ => Err(napi::Error::from_reason(
        "VARIANT does not contain VT_UNKNOWN or VT_DISPATCH",
      )),
    }
  }

  #[napi]
  pub fn to_safe_array(&self) -> napi::Result<DynComSafeArray> {
    match self.value()?.data().map_err(com_error)? {
      dynwinrt::com::VariantData::SafeArray(value) => Ok(DynComSafeArray::new(value)),
      _ => Err(napi::Error::from_reason(
        "VARIANT does not contain a SAFEARRAY",
      )),
    }
  }

  #[napi]
  pub fn release(&mut self) {
    self.value = None;
  }
}

#[napi]
pub struct DynComDispatchParams {
  owner_thread: std::thread::ThreadId,
  value: Option<dynwinrt::com::DispatchParamsValue>,
}

impl DynComDispatchParams {
  fn from_value(value: dynwinrt::com::DispatchParamsValue) -> Self {
    Self {
      owner_thread: std::thread::current().id(),
      value: Some(value),
    }
  }

  fn value(&self) -> napi::Result<&dynwinrt::com::DispatchParamsValue> {
    if std::thread::current().id() != self.owner_thread {
      return Err(napi::Error::from_reason(
        "Apartment-bound DISPPARAMS used from a different thread",
      ));
    }
    self
      .value
      .as_ref()
      .ok_or_else(|| napi::Error::from_reason("DISPPARAMS has been released"))
  }
}

impl Drop for DynComDispatchParams {
  fn drop(&mut self) {
    if std::thread::current().id() != self.owner_thread || super::winui_dispatcher_loop_exited() {
      if let Some(value) = self.value.take() {
        std::mem::forget(value);
      }
    }
  }
}

// Safety: every public operation rejects access away from the creating
// apartment, and wrong-thread/shutdown destruction deliberately leaks.
unsafe impl Send for DynComDispatchParams {}
unsafe impl Sync for DynComDispatchParams {}

#[napi]
impl DynComDispatchParams {
  #[napi(constructor)]
  pub fn new(
    arguments: Vec<&DynComVariant>,
    named_dispids: Option<Vec<f64>>,
  ) -> napi::Result<Self> {
    let arguments = arguments
      .into_iter()
      .map(DynComVariant::value)
      .collect::<napi::Result<Vec<_>>>()?;
    let named_dispids = named_dispids
      .unwrap_or_default()
      .into_iter()
      .map(|value| {
        checked_signed_number(
          value,
          i32::MIN as i64,
          i32::MAX as i64,
          "DISPPARAMS named DISPID",
        )
        .map(|value| value as i32)
      })
      .collect::<napi::Result<Vec<_>>>()?;
    dynwinrt::com::DispatchParamsValue::new(
      &arguments.into_iter().cloned().collect::<Vec<_>>(),
      &named_dispids,
    )
    .map(Self::from_value)
    .map_err(com_error)
  }

  #[napi(getter)]
  pub fn argument_count(&self) -> napi::Result<u32> {
    u32::try_from(self.value()?.argument_count())
      .map_err(|_| napi::Error::from_reason("DISPPARAMS argument count exceeds UINT"))
  }

  #[napi(getter)]
  pub fn named_dispids(&self) -> napi::Result<Vec<i32>> {
    Ok(self.value()?.named_dispids())
  }

  #[napi(js_name = "clone")]
  pub fn clone_value(&self) -> napi::Result<Self> {
    Ok(Self::from_value(self.value()?.clone()))
  }

  #[napi]
  pub fn release(&mut self) -> napi::Result<()> {
    if std::thread::current().id() != self.owner_thread {
      return Err(napi::Error::from_reason(
        "Apartment-bound DISPPARAMS used from a different thread",
      ));
    }
    self.value = None;
    Ok(())
  }
}

#[napi]
pub struct DynComExcepInfo {
  owner_thread: std::thread::ThreadId,
  value: Option<dynwinrt::com::ExcepInfoValue>,
}

impl DynComExcepInfo {
  fn new(value: dynwinrt::com::ExcepInfoValue) -> Self {
    Self {
      owner_thread: std::thread::current().id(),
      value: Some(value),
    }
  }

  fn value(&self) -> napi::Result<&dynwinrt::com::ExcepInfoValue> {
    if std::thread::current().id() != self.owner_thread {
      return Err(napi::Error::from_reason(
        "Apartment-bound EXCEPINFO used from a different thread",
      ));
    }
    self
      .value
      .as_ref()
      .ok_or_else(|| napi::Error::from_reason("EXCEPINFO has been released"))
  }
}

impl Drop for DynComExcepInfo {
  fn drop(&mut self) {
    if std::thread::current().id() != self.owner_thread || super::winui_dispatcher_loop_exited() {
      if let Some(value) = self.value.take() {
        std::mem::forget(value);
      }
    }
  }
}

unsafe impl Send for DynComExcepInfo {}
unsafe impl Sync for DynComExcepInfo {}

#[napi]
impl DynComExcepInfo {
  #[napi(getter)]
  pub fn code(&self) -> napi::Result<u32> {
    Ok(u32::from(self.value()?.code()))
  }

  #[napi(getter)]
  pub fn source(&self) -> napi::Result<Option<String>> {
    Ok(self.value()?.source().map(str::to_owned))
  }

  #[napi(getter)]
  pub fn description(&self) -> napi::Result<Option<String>> {
    Ok(self.value()?.description().map(str::to_owned))
  }

  #[napi(getter)]
  pub fn help_file(&self) -> napi::Result<Option<String>> {
    Ok(self.value()?.help_file().map(str::to_owned))
  }

  #[napi(getter)]
  pub fn help_context(&self) -> napi::Result<u32> {
    Ok(self.value()?.help_context())
  }

  #[napi(getter)]
  pub fn scode(&self) -> napi::Result<i32> {
    Ok(self.value()?.scode())
  }

  #[napi(js_name = "clone")]
  pub fn clone_value(&self) -> napi::Result<Self> {
    Ok(Self::new(self.value()?.clone()))
  }

  #[napi]
  pub fn release(&mut self) -> napi::Result<()> {
    if std::thread::current().id() != self.owner_thread {
      return Err(napi::Error::from_reason(
        "Apartment-bound EXCEPINFO used from a different thread",
      ));
    }
    self.value = None;
    Ok(())
  }
}

#[napi]
pub struct DynComStatStg {
  owner_thread: std::thread::ThreadId,
  value: Option<dynwinrt::com::StatStgValue>,
}

impl DynComStatStg {
  fn new(value: dynwinrt::com::StatStgValue) -> Self {
    Self {
      owner_thread: std::thread::current().id(),
      value: Some(value),
    }
  }

  fn value(&self) -> napi::Result<&dynwinrt::com::StatStgValue> {
    if std::thread::current().id() != self.owner_thread {
      return Err(napi::Error::from_reason(
        "Apartment-bound STATSTG used from a different thread",
      ));
    }
    self
      .value
      .as_ref()
      .ok_or_else(|| napi::Error::from_reason("STATSTG has been released"))
  }
}

// StatStgValue owns immutable Rust data after conversion; the native name
// pointer has already been adopted and nulled before this wrapper is created.
unsafe impl Send for DynComStatStg {}
unsafe impl Sync for DynComStatStg {}

#[napi]
impl DynComStatStg {
  #[napi(getter)]
  pub fn name(&self) -> napi::Result<Option<String>> {
    Ok(self.value()?.name().map(str::to_owned))
  }

  #[napi(getter)]
  pub fn storage_type(&self) -> napi::Result<u32> {
    Ok(self.value()?.stream_type())
  }

  #[napi(getter)]
  pub fn size(&self) -> napi::Result<BigInt> {
    Ok(self.value()?.size().into())
  }

  #[napi(getter)]
  pub fn modified_time(&self) -> napi::Result<BigInt> {
    Ok(self.value()?.modified_time().into())
  }

  #[napi(getter)]
  pub fn creation_time(&self) -> napi::Result<BigInt> {
    Ok(self.value()?.created_time().into())
  }

  #[napi(getter)]
  pub fn access_time(&self) -> napi::Result<BigInt> {
    Ok(self.value()?.accessed_time().into())
  }

  #[napi(getter)]
  pub fn mode(&self) -> napi::Result<u32> {
    Ok(self.value()?.mode())
  }

  #[napi(getter)]
  pub fn locks_supported(&self) -> napi::Result<u32> {
    Ok(self.value()?.locks_supported())
  }

  #[napi(getter)]
  pub fn class_id(&self) -> napi::Result<String> {
    Ok(format!("{:?}", self.value()?.clsid()))
  }

  #[napi(getter)]
  pub fn state_bits(&self) -> napi::Result<u32> {
    Ok(self.value()?.state_bits())
  }

  #[napi]
  pub fn release(&mut self) -> napi::Result<()> {
    if std::thread::current().id() != self.owner_thread {
      return Err(napi::Error::from_reason(
        "Apartment-bound STATSTG used from a different thread",
      ));
    }
    self.value = None;
    Ok(())
  }
}

#[napi]
pub struct DynComAllocation {
  owner_thread: std::thread::ThreadId,
  allocator: Option<windows::Win32::System::Com::IMalloc>,
  pointer: usize,
}

impl DynComAllocation {
  fn new(allocator: windows::Win32::System::Com::IMalloc, pointer: *mut std::ffi::c_void) -> Self {
    debug_assert!(!pointer.is_null());
    Self {
      owner_thread: std::thread::current().id(),
      allocator: Some(allocator),
      pointer: pointer as usize,
    }
  }

  fn ensure_owner_thread(&self) -> napi::Result<()> {
    if std::thread::current().id() == self.owner_thread {
      Ok(())
    } else {
      Err(napi::Error::from_reason(
        "Apartment-bound IMalloc allocation used from a different thread",
      ))
    }
  }

  fn validate_allocator(
    &self,
    allocator: &windows::Win32::System::Com::IMalloc,
  ) -> napi::Result<()> {
    self.ensure_owner_thread()?;
    let expected: IUnknown = self
      .allocator
      .as_ref()
      .ok_or_else(|| napi::Error::from_reason("IMalloc allocation has been released"))?
      .cast()
      .map_err(|error| napi::Error::from_reason(error.to_string()))?;
    let actual: IUnknown = allocator
      .cast()
      .map_err(|error| napi::Error::from_reason(error.to_string()))?;
    if expected.as_raw() != actual.as_raw() {
      return Err(napi::Error::from_reason(
        "IMalloc allocation belongs to a different allocator",
      ));
    }
    if self.pointer == 0 {
      return Err(napi::Error::from_reason(
        "IMalloc allocation has been released",
      ));
    }
    Ok(())
  }

  fn borrowed_pointer(
    &self,
    allocator: &windows::Win32::System::Com::IMalloc,
  ) -> napi::Result<*mut std::ffi::c_void> {
    self.validate_allocator(allocator)?;
    Ok(self.pointer as *mut std::ffi::c_void)
  }

  fn inspection_pointer(&self) -> napi::Result<*mut std::ffi::c_void> {
    self.ensure_owner_thread()?;
    if self.pointer == 0 {
      return Err(napi::Error::from_reason(
        "IMalloc allocation has been released",
      ));
    }
    Ok(self.pointer as *mut std::ffi::c_void)
  }

  fn take_pointer(
    &mut self,
    allocator: &windows::Win32::System::Com::IMalloc,
  ) -> napi::Result<*mut std::ffi::c_void> {
    self.validate_allocator(allocator)?;
    let pointer = std::mem::replace(&mut self.pointer, 0);
    self.allocator = None;
    Ok(pointer as *mut std::ffi::c_void)
  }

  fn release_inner(&mut self) {
    if self.pointer != 0 {
      if let Some(allocator) = &self.allocator {
        unsafe { allocator.Free(Some(self.pointer as *mut std::ffi::c_void)) };
      }
      self.pointer = 0;
      self.allocator = None;
    }
  }
}

impl Drop for DynComAllocation {
  fn drop(&mut self) {
    if std::thread::current().id() != self.owner_thread || super::winui_dispatcher_loop_exited() {
      // Never invoke an apartment-bound allocator from a foreign or shut-down thread.
      self.pointer = 0;
      if let Some(allocator) = self.allocator.take() {
        std::mem::forget(allocator);
      }
      return;
    }
    self.release_inner();
  }
}

#[napi]
impl DynComAllocation {
  #[napi(getter)]
  pub fn released(&self) -> bool {
    self.pointer == 0
  }

  #[napi]
  pub fn release(&mut self) -> napi::Result<()> {
    self.ensure_owner_thread()?;
    self.release_inner();
    Ok(())
  }
}

fn malloc_allocator(value: &DynWinRTValue) -> napi::Result<windows::Win32::System::Com::IMalloc> {
  value.ensure_existing_com_apartment()?;
  value
    .0
    .as_object()
    .ok_or_else(|| napi::Error::from_reason("IMalloc operation requires a COM object"))?
    .cast()
    .map_err(|error| napi::Error::from_reason(error.to_string()))
}

fn malloc_pointer_value(pointer: *mut std::ffi::c_void) -> DynWinRTValue {
  DynWinRTValue::with_borrowed_pointer(dynwinrt::WinRTValue::RawPtr(pointer))
}

fn take_malloc_return_pointer(value: &mut DynWinRTValue) -> napi::Result<*mut std::ffi::c_void> {
  if value.1.is_some() || value.2 != PointerProvenance::UnclassifiedOutput {
    return Err(napi::Error::from_reason(
      "IMalloc allocation requires an unowned direct pointer return",
    ));
  }
  match std::mem::replace(&mut value.0, dynwinrt::WinRTValue::Null) {
    dynwinrt::WinRTValue::RawPtr(pointer) => {
      value.2 = PointerProvenance::None;
      Ok(pointer)
    }
    dynwinrt::WinRTValue::Null => {
      value.2 = PointerProvenance::None;
      Ok(std::ptr::null_mut())
    }
    other => {
      value.0 = other;
      Err(napi::Error::from_reason(
        "IMalloc allocation result is not a native pointer",
      ))
    }
  }
}

fn malloc_allocation_pointer(
  allocator: &DynWinRTValue,
  allocation: Option<&DynComAllocation>,
) -> napi::Result<DynWinRTValue> {
  let allocator = malloc_allocator(allocator)?;
  allocation
    .map(|allocation| allocation.borrowed_pointer(&allocator))
    .transpose()
    .map(|pointer| malloc_pointer_value(pointer.unwrap_or(std::ptr::null_mut())))
}

fn malloc_inspection_pointer(allocation: Option<&DynComAllocation>) -> napi::Result<DynWinRTValue> {
  allocation
    .map(DynComAllocation::inspection_pointer)
    .transpose()
    .map(|pointer| malloc_pointer_value(pointer.unwrap_or(std::ptr::null_mut())))
}

fn take_malloc_allocation_pointer(
  allocator: &DynWinRTValue,
  allocation: Option<&mut DynComAllocation>,
) -> napi::Result<DynWinRTValue> {
  let allocator = malloc_allocator(allocator)?;
  allocation
    .map(|allocation| allocation.take_pointer(&allocator))
    .transpose()
    .map(|pointer| malloc_pointer_value(pointer.unwrap_or(std::ptr::null_mut())))
}

fn take_malloc_allocation(
  allocator: &DynWinRTValue,
  value: &mut DynWinRTValue,
) -> napi::Result<Option<DynComAllocation>> {
  let allocator = malloc_allocator(allocator)?;
  let pointer = take_malloc_return_pointer(value)?;
  Ok((!pointer.is_null()).then(|| DynComAllocation::new(allocator, pointer)))
}

fn finish_malloc_reallocation(
  allocator: &DynWinRTValue,
  allocation: Option<&mut DynComAllocation>,
  size: BigInt,
  value: &mut DynWinRTValue,
) -> napi::Result<Option<DynComAllocation>> {
  let allocator = malloc_allocator(allocator)?;
  if let Some(allocation) = allocation.as_deref() {
    allocation.validate_allocator(&allocator)?;
  }
  let pointer = take_malloc_return_pointer(value)?;
  if !pointer.is_null() {
    if let Some(allocation) = allocation {
      let _ = allocation.take_pointer(&allocator)?;
    }
    return Ok(Some(DynComAllocation::new(allocator, pointer)));
  }

  let (negative, size, lossless) = size.get_u64();
  if negative || !lossless {
    return Err(napi::Error::from_reason(
      "IMalloc reallocation size must be an unsigned integer",
    ));
  }
  if size == 0 {
    if let Some(allocation) = allocation {
      let _ = allocation.take_pointer(&allocator)?;
    }
  }
  Ok(None)
}

#[napi(object)]
pub struct DynComSafeArrayBound {
  pub lower_bound: f64,
  pub length: f64,
}

#[napi]
pub struct DynComSafeArray {
  value: Option<dynwinrt::com::SafeArrayValue>,
}

impl DynComSafeArray {
  fn new(value: dynwinrt::com::SafeArrayValue) -> Self {
    Self { value: Some(value) }
  }

  fn value(&self) -> napi::Result<&dynwinrt::com::SafeArrayValue> {
    self
      .value
      .as_ref()
      .ok_or_else(|| napi::Error::from_reason("SAFEARRAY has been released"))
  }

  fn create(
    element_type: dynwinrt::com::SafeArrayElementType,
    values: Vec<dynwinrt::com::SafeArrayElementValue>,
    bounds: Option<Vec<DynComSafeArrayBound>>,
  ) -> napi::Result<Self> {
    let bounds = match bounds {
      Some(bounds) => bounds
        .into_iter()
        .map(|bound| {
          let lower_bound = checked_signed_number(
            bound.lower_bound,
            i32::MIN as i64,
            i32::MAX as i64,
            "SAFEARRAY lower bound",
          )? as i32;
          let length =
            checked_unsigned_number(bound.length, u32::MAX as u64, "SAFEARRAY bound length")?
              as u32;
          dynwinrt::com::SafeArrayBound::new(lower_bound, length).map_err(com_error)
        })
        .collect::<napi::Result<Vec<_>>>()?,
      None => vec![dynwinrt::com::SafeArrayBound::new(
        0,
        u32::try_from(values.len())
          .map_err(|_| napi::Error::from_reason("SAFEARRAY length exceeds u32"))?,
      )
      .map_err(com_error)?],
    };
    dynwinrt::com::SafeArrayValue::new(element_type, bounds, values)
      .map(Self::new)
      .map_err(com_error)
  }
}

impl Drop for DynComSafeArray {
  fn drop(&mut self) {
    if super::winui_dispatcher_loop_exited() {
      if let Some(value) = self.value.take() {
        std::mem::forget(value);
      }
    }
  }
}

#[napi]
impl DynComSafeArray {
  #[napi]
  pub fn i8(values: Vec<f64>, bounds: Option<Vec<DynComSafeArrayBound>>) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        checked_signed_number(value, i8::MIN as i64, i8::MAX as i64, "SAFEARRAY VT_I1")
          .map(|value| dynwinrt::com::SafeArrayElementValue::I8(value as i8))
      })
      .collect::<napi::Result<Vec<_>>>()?;
    Self::create(dynwinrt::com::SafeArrayElementType::I8, values, bounds)
  }

  #[napi]
  pub fn u8(values: Vec<f64>, bounds: Option<Vec<DynComSafeArrayBound>>) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        checked_unsigned_number(value, u8::MAX as u64, "SAFEARRAY VT_UI1")
          .map(|value| dynwinrt::com::SafeArrayElementValue::U8(value as u8))
      })
      .collect::<napi::Result<Vec<_>>>()?;
    Self::create(dynwinrt::com::SafeArrayElementType::U8, values, bounds)
  }

  #[napi]
  pub fn i16(values: Vec<f64>, bounds: Option<Vec<DynComSafeArrayBound>>) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        checked_signed_number(value, i16::MIN as i64, i16::MAX as i64, "SAFEARRAY VT_I2")
          .map(|value| dynwinrt::com::SafeArrayElementValue::I16(value as i16))
      })
      .collect::<napi::Result<Vec<_>>>()?;
    Self::create(dynwinrt::com::SafeArrayElementType::I16, values, bounds)
  }

  #[napi]
  pub fn u16(values: Vec<f64>, bounds: Option<Vec<DynComSafeArrayBound>>) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        checked_unsigned_number(value, u16::MAX as u64, "SAFEARRAY VT_UI2")
          .map(|value| dynwinrt::com::SafeArrayElementValue::U16(value as u16))
      })
      .collect::<napi::Result<Vec<_>>>()?;
    Self::create(dynwinrt::com::SafeArrayElementType::U16, values, bounds)
  }

  #[napi]
  pub fn i32(values: Vec<f64>, bounds: Option<Vec<DynComSafeArrayBound>>) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        checked_signed_number(value, i32::MIN as i64, i32::MAX as i64, "SAFEARRAY VT_I4")
          .map(|value| dynwinrt::com::SafeArrayElementValue::I32(value as i32))
      })
      .collect::<napi::Result<Vec<_>>>()?;
    Self::create(dynwinrt::com::SafeArrayElementType::I32, values, bounds)
  }

  #[napi]
  pub fn u32(values: Vec<f64>, bounds: Option<Vec<DynComSafeArrayBound>>) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        checked_unsigned_number(value, u32::MAX as u64, "SAFEARRAY VT_UI4")
          .map(|value| dynwinrt::com::SafeArrayElementValue::U32(value as u32))
      })
      .collect::<napi::Result<Vec<_>>>()?;
    Self::create(dynwinrt::com::SafeArrayElementType::U32, values, bounds)
  }

  #[napi]
  pub fn i64(values: Vec<BigInt>, bounds: Option<Vec<DynComSafeArrayBound>>) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        bigint_i64(value, "SAFEARRAY VT_I8").map(dynwinrt::com::SafeArrayElementValue::I64)
      })
      .collect::<napi::Result<Vec<_>>>()?;
    Self::create(dynwinrt::com::SafeArrayElementType::I64, values, bounds)
  }

  #[napi]
  pub fn u64(values: Vec<BigInt>, bounds: Option<Vec<DynComSafeArrayBound>>) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        bigint_u64(value, "SAFEARRAY VT_UI8").map(dynwinrt::com::SafeArrayElementValue::U64)
      })
      .collect::<napi::Result<Vec<_>>>()?;
    Self::create(dynwinrt::com::SafeArrayElementType::U64, values, bounds)
  }

  #[napi]
  pub fn f32(values: Vec<f64>, bounds: Option<Vec<DynComSafeArrayBound>>) -> napi::Result<Self> {
    Self::create(
      dynwinrt::com::SafeArrayElementType::F32,
      values
        .into_iter()
        .map(|value| dynwinrt::com::SafeArrayElementValue::F32(value as f32))
        .collect(),
      bounds,
    )
  }

  #[napi]
  pub fn f64(values: Vec<f64>, bounds: Option<Vec<DynComSafeArrayBound>>) -> napi::Result<Self> {
    Self::create(
      dynwinrt::com::SafeArrayElementType::F64,
      values
        .into_iter()
        .map(dynwinrt::com::SafeArrayElementValue::F64)
        .collect(),
      bounds,
    )
  }

  #[napi]
  pub fn bool(values: Vec<bool>, bounds: Option<Vec<DynComSafeArrayBound>>) -> napi::Result<Self> {
    Self::create(
      dynwinrt::com::SafeArrayElementType::Bool,
      values
        .into_iter()
        .map(dynwinrt::com::SafeArrayElementValue::Bool)
        .collect(),
      bounds,
    )
  }

  #[napi]
  pub fn bstr(
    values: Vec<String>,
    bounds: Option<Vec<DynComSafeArrayBound>>,
  ) -> napi::Result<Self> {
    Self::create(
      dynwinrt::com::SafeArrayElementType::Bstr,
      values
        .into_iter()
        .map(dynwinrt::com::SafeArrayElementValue::Bstr)
        .collect(),
      bounds,
    )
  }

  #[napi]
  pub fn unknown(
    values: Vec<&DynWinRTValue>,
    bounds: Option<Vec<DynComSafeArrayBound>>,
  ) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        optional_com_object(Some(value), "SAFEARRAY VT_UNKNOWN")
          .map(|value| dynwinrt::com::SafeArrayElementValue::Unknown(value))
      })
      .collect::<napi::Result<Vec<_>>>()?;
    Self::create(dynwinrt::com::SafeArrayElementType::Unknown, values, bounds)
  }

  #[napi]
  pub fn interface(
    iid: &WinGUID,
    values: Vec<&DynWinRTValue>,
    bounds: Option<Vec<DynComSafeArrayBound>>,
  ) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        optional_com_object(Some(value), "SAFEARRAY VT_UNKNOWN")
          .map(dynwinrt::com::SafeArrayElementValue::Unknown)
      })
      .collect::<napi::Result<Vec<_>>>()?;
    let bounds = match bounds {
      Some(bounds) => bounds
        .into_iter()
        .map(|bound| {
          let lower_bound = checked_signed_number(
            bound.lower_bound,
            i32::MIN as i64,
            i32::MAX as i64,
            "SAFEARRAY lower bound",
          )? as i32;
          let length =
            checked_unsigned_number(bound.length, u32::MAX as u64, "SAFEARRAY bound length")?
              as u32;
          dynwinrt::com::SafeArrayBound::new(lower_bound, length).map_err(com_error)
        })
        .collect::<napi::Result<Vec<_>>>()?,
      None => vec![dynwinrt::com::SafeArrayBound::new(
        0,
        u32::try_from(values.len())
          .map_err(|_| napi::Error::from_reason("SAFEARRAY length exceeds u32"))?,
      )
      .map_err(com_error)?],
    };
    dynwinrt::com::SafeArrayValue::new_interface(iid.0, bounds, values)
      .map(Self::new)
      .map_err(com_error)
  }

  #[napi]
  pub fn dispatch(
    values: Vec<&DynWinRTValue>,
    bounds: Option<Vec<DynComSafeArrayBound>>,
  ) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        optional_com_object(Some(value), "SAFEARRAY VT_DISPATCH").and_then(|value| {
          let value = value
            .map(|value| value.cast::<windows::Win32::System::Com::IDispatch>())
            .transpose()
            .map_err(|error| napi::Error::from_reason(error.message()))?
            .map(|value| value.cast::<windows::core::IUnknown>())
            .transpose()
            .map_err(|error| napi::Error::from_reason(error.message()))?;
          Ok(dynwinrt::com::SafeArrayElementValue::Dispatch(value))
        })
      })
      .collect::<napi::Result<Vec<_>>>()?;
    Self::create(
      dynwinrt::com::SafeArrayElementType::Dispatch,
      values,
      bounds,
    )
  }

  #[napi]
  pub fn variant(
    values: Vec<&DynComVariant>,
    bounds: Option<Vec<DynComSafeArrayBound>>,
  ) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        value
          .value()
          .cloned()
          .map(dynwinrt::com::SafeArrayElementValue::Variant)
      })
      .collect::<napi::Result<Vec<_>>>()?;
    Self::create(dynwinrt::com::SafeArrayElementType::Variant, values, bounds)
  }

  #[napi(getter)]
  pub fn element_type(&self) -> napi::Result<String> {
    Ok(safe_array_element_kind(self.value()?.element_type()).into())
  }

  #[napi(getter)]
  pub fn interface_iid(&self) -> napi::Result<Option<WinGUID>> {
    Ok(self.value()?.interface_iid().map(WinGUID))
  }

  #[napi(getter)]
  pub fn length(&self) -> napi::Result<BigInt> {
    Ok(BigInt::from(self.value()?.len() as u64))
  }

  #[napi(getter)]
  pub fn bounds(&self) -> napi::Result<Vec<DynComSafeArrayBound>> {
    Ok(
      self
        .value()?
        .bounds()
        .iter()
        .map(|bound| DynComSafeArrayBound {
          lower_bound: f64::from(bound.lower_bound()),
          length: f64::from(bound.length()),
        })
        .collect(),
    )
  }

  #[napi]
  pub fn to_numbers(&self) -> napi::Result<Vec<f64>> {
    self
      .value()?
      .elements()
      .map_err(com_error)?
      .into_iter()
      .map(|value| match value {
        dynwinrt::com::SafeArrayElementValue::I8(value) => Ok(f64::from(value)),
        dynwinrt::com::SafeArrayElementValue::U8(value) => Ok(f64::from(value)),
        dynwinrt::com::SafeArrayElementValue::I16(value) => Ok(f64::from(value)),
        dynwinrt::com::SafeArrayElementValue::U16(value) => Ok(f64::from(value)),
        dynwinrt::com::SafeArrayElementValue::I32(value) => Ok(f64::from(value)),
        dynwinrt::com::SafeArrayElementValue::U32(value) => Ok(f64::from(value)),
        dynwinrt::com::SafeArrayElementValue::F32(value) => Ok(f64::from(value)),
        dynwinrt::com::SafeArrayElementValue::F64(value) => Ok(value),
        _ => Err(napi::Error::from_reason(
          "SAFEARRAY element type is not number-sized",
        )),
      })
      .collect()
  }

  #[napi]
  pub fn to_bigints(&self) -> napi::Result<Vec<BigInt>> {
    self
      .value()?
      .elements()
      .map_err(com_error)?
      .into_iter()
      .map(|value| match value {
        dynwinrt::com::SafeArrayElementValue::I64(value) => Ok(BigInt::from(value)),
        dynwinrt::com::SafeArrayElementValue::U64(value) => Ok(BigInt::from(value)),
        _ => Err(napi::Error::from_reason(
          "SAFEARRAY element type is not a 64-bit integer",
        )),
      })
      .collect()
  }

  #[napi]
  pub fn to_bools(&self) -> napi::Result<Vec<bool>> {
    self
      .value()?
      .elements()
      .map_err(com_error)?
      .into_iter()
      .map(|value| match value {
        dynwinrt::com::SafeArrayElementValue::Bool(value) => Ok(value),
        _ => Err(napi::Error::from_reason(
          "SAFEARRAY element type is not VT_BOOL",
        )),
      })
      .collect()
  }

  #[napi]
  pub fn to_strings(&self) -> napi::Result<Vec<String>> {
    self
      .value()?
      .elements()
      .map_err(com_error)?
      .into_iter()
      .map(|value| match value {
        dynwinrt::com::SafeArrayElementValue::Bstr(value) => Ok(value),
        _ => Err(napi::Error::from_reason(
          "SAFEARRAY element type is not VT_BSTR",
        )),
      })
      .collect()
  }

  #[napi]
  pub fn to_interfaces(&self) -> napi::Result<Vec<Option<DynWinRTValue>>> {
    self
      .value()?
      .elements()
      .map_err(com_error)?
      .into_iter()
      .map(|value| match value {
        dynwinrt::com::SafeArrayElementValue::Unknown(value)
        | dynwinrt::com::SafeArrayElementValue::Dispatch(value) => {
          value.map(apartment_bound_com_object).transpose()
        }
        _ => Err(napi::Error::from_reason(
          "SAFEARRAY element type is not VT_UNKNOWN or VT_DISPATCH",
        )),
      })
      .collect()
  }

  #[napi]
  pub fn to_variants(&self) -> napi::Result<Vec<DynComVariant>> {
    self
      .value()?
      .elements()
      .map_err(com_error)?
      .into_iter()
      .map(|value| match value {
        dynwinrt::com::SafeArrayElementValue::Variant(value) => Ok(DynComVariant::new(value)),
        _ => Err(napi::Error::from_reason(
          "SAFEARRAY element type is not VT_VARIANT",
        )),
      })
      .collect()
  }

  #[napi]
  pub fn release(&mut self) {
    self.value = None;
  }
}

#[napi]
pub struct DynComPropVariant {
  value: Option<dynwinrt::com::PropVariantValue>,
}

impl DynComPropVariant {
  fn new(value: dynwinrt::com::PropVariantValue) -> Self {
    Self { value: Some(value) }
  }

  fn value(&self) -> napi::Result<&dynwinrt::com::PropVariantValue> {
    self
      .value
      .as_ref()
      .ok_or_else(|| napi::Error::from_reason("PROPVARIANT has been released"))
  }
}

#[napi]
impl DynComPropVariant {
  #[napi]
  pub fn empty() -> Self {
    Self::new(dynwinrt::com::PropVariantValue::empty())
  }

  #[napi(js_name = "null")]
  pub fn null_value() -> Self {
    Self::new(dynwinrt::com::PropVariantValue::null())
  }

  #[napi]
  pub fn i8(value: f64) -> napi::Result<Self> {
    checked_signed_number(value, i8::MIN as i64, i8::MAX as i64, "PROPVARIANT VT_I1")
      .map(|value| Self::new(dynwinrt::com::PropVariantValue::from_i8(value as i8)))
  }

  #[napi]
  pub fn u8(value: f64) -> napi::Result<Self> {
    checked_unsigned_number(value, u8::MAX as u64, "PROPVARIANT VT_UI1")
      .map(|value| Self::new(dynwinrt::com::PropVariantValue::from_u8(value as u8)))
  }

  #[napi]
  pub fn i16(value: f64) -> napi::Result<Self> {
    checked_signed_number(value, i16::MIN as i64, i16::MAX as i64, "PROPVARIANT VT_I2")
      .map(|value| Self::new(dynwinrt::com::PropVariantValue::from_i16(value as i16)))
  }

  #[napi]
  pub fn u16(value: f64) -> napi::Result<Self> {
    checked_unsigned_number(value, u16::MAX as u64, "PROPVARIANT VT_UI2")
      .map(|value| Self::new(dynwinrt::com::PropVariantValue::from_u16(value as u16)))
  }

  #[napi]
  pub fn i32(value: f64) -> napi::Result<Self> {
    checked_signed_number(value, i32::MIN as i64, i32::MAX as i64, "PROPVARIANT VT_I4")
      .map(|value| Self::new(dynwinrt::com::PropVariantValue::from_i32(value as i32)))
  }

  #[napi]
  pub fn u32(value: f64) -> napi::Result<Self> {
    checked_unsigned_number(value, u32::MAX as u64, "PROPVARIANT VT_UI4")
      .map(|value| Self::new(dynwinrt::com::PropVariantValue::from_u32(value as u32)))
  }

  #[napi]
  pub fn int(value: f64) -> napi::Result<Self> {
    checked_signed_number(
      value,
      i32::MIN as i64,
      i32::MAX as i64,
      "PROPVARIANT VT_INT",
    )
    .map(|value| Self::new(dynwinrt::com::PropVariantValue::from_int(value as i32)))
  }

  #[napi]
  pub fn uint(value: f64) -> napi::Result<Self> {
    checked_unsigned_number(value, u32::MAX as u64, "PROPVARIANT VT_UINT")
      .map(|value| Self::new(dynwinrt::com::PropVariantValue::from_uint(value as u32)))
  }

  #[napi]
  pub fn i64(value: BigInt) -> napi::Result<Self> {
    bigint_i64(value, "PROPVARIANT VT_I8")
      .map(dynwinrt::com::PropVariantValue::from_i64)
      .map(Self::new)
  }

  #[napi]
  pub fn u64(value: BigInt) -> napi::Result<Self> {
    bigint_u64(value, "PROPVARIANT VT_UI8")
      .map(dynwinrt::com::PropVariantValue::from_u64)
      .map(Self::new)
  }

  #[napi]
  pub fn f32(value: f64) -> Self {
    Self::new(dynwinrt::com::PropVariantValue::from_f32(value as f32))
  }

  #[napi]
  pub fn f64(value: f64) -> Self {
    Self::new(dynwinrt::com::PropVariantValue::from_f64(value))
  }

  #[napi]
  pub fn bool(value: bool) -> Self {
    Self::new(dynwinrt::com::PropVariantValue::from_bool(value))
  }

  #[napi]
  pub fn string(value: String) -> napi::Result<Self> {
    dynwinrt::com::PropVariantValue::from_string(&value)
      .map(Self::new)
      .map_err(com_error)
  }

  #[napi]
  pub fn guid(value: &WinGUID) -> napi::Result<Self> {
    dynwinrt::com::PropVariantValue::from_guid(value.0)
      .map(Self::new)
      .map_err(com_error)
  }

  #[napi]
  pub fn filetime(value: BigInt) -> napi::Result<Self> {
    bigint_u64(value, "PROPVARIANT VT_FILETIME")
      .map(dynwinrt::com::PropVariantValue::from_filetime)
      .map(Self::new)
  }

  #[napi]
  pub fn blob(value: Buffer) -> napi::Result<Self> {
    dynwinrt::com::PropVariantValue::from_blob(&value)
      .map(Self::new)
      .map_err(com_error)
  }

  #[napi]
  pub fn i8_vector(values: Vec<f64>) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        checked_signed_number(
          value,
          i8::MIN as i64,
          i8::MAX as i64,
          "PROPVARIANT VT_VECTOR|VT_I1",
        )
        .map(|value| value as i8)
      })
      .collect::<napi::Result<Vec<_>>>()?;
    propvariant_vector(dynwinrt::com::PropVariantVector::I8(values))
  }

  #[napi]
  pub fn u8_vector(values: Vec<f64>) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        checked_unsigned_number(value, u8::MAX as u64, "PROPVARIANT VT_VECTOR|VT_UI1")
          .map(|value| value as u8)
      })
      .collect::<napi::Result<Vec<_>>>()?;
    propvariant_vector(dynwinrt::com::PropVariantVector::U8(values))
  }

  #[napi]
  pub fn i16_vector(values: Vec<f64>) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        checked_signed_number(
          value,
          i16::MIN as i64,
          i16::MAX as i64,
          "PROPVARIANT VT_VECTOR|VT_I2",
        )
        .map(|value| value as i16)
      })
      .collect::<napi::Result<Vec<_>>>()?;
    propvariant_vector(dynwinrt::com::PropVariantVector::I16(values))
  }

  #[napi]
  pub fn u16_vector(values: Vec<f64>) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        checked_unsigned_number(value, u16::MAX as u64, "PROPVARIANT VT_VECTOR|VT_UI2")
          .map(|value| value as u16)
      })
      .collect::<napi::Result<Vec<_>>>()?;
    propvariant_vector(dynwinrt::com::PropVariantVector::U16(values))
  }

  #[napi]
  pub fn i32_vector(values: Vec<f64>) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        checked_signed_number(
          value,
          i32::MIN as i64,
          i32::MAX as i64,
          "PROPVARIANT VT_VECTOR|VT_I4",
        )
        .map(|value| value as i32)
      })
      .collect::<napi::Result<Vec<_>>>()?;
    propvariant_vector(dynwinrt::com::PropVariantVector::I32(values))
  }

  #[napi]
  pub fn u32_vector(values: Vec<f64>) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| {
        checked_unsigned_number(value, u32::MAX as u64, "PROPVARIANT VT_VECTOR|VT_UI4")
          .map(|value| value as u32)
      })
      .collect::<napi::Result<Vec<_>>>()?;
    propvariant_vector(dynwinrt::com::PropVariantVector::U32(values))
  }

  #[napi]
  pub fn i64_vector(values: Vec<BigInt>) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| bigint_i64(value, "PROPVARIANT VT_VECTOR|VT_I8"))
      .collect::<napi::Result<Vec<_>>>()?;
    propvariant_vector(dynwinrt::com::PropVariantVector::I64(values))
  }

  #[napi]
  pub fn u64_vector(values: Vec<BigInt>) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| bigint_u64(value, "PROPVARIANT VT_VECTOR|VT_UI8"))
      .collect::<napi::Result<Vec<_>>>()?;
    propvariant_vector(dynwinrt::com::PropVariantVector::U64(values))
  }

  #[napi]
  pub fn f32_vector(values: Vec<f64>) -> napi::Result<Self> {
    propvariant_vector(dynwinrt::com::PropVariantVector::F32(
      values.into_iter().map(|value| value as f32).collect(),
    ))
  }

  #[napi]
  pub fn f64_vector(values: Vec<f64>) -> napi::Result<Self> {
    propvariant_vector(dynwinrt::com::PropVariantVector::F64(values))
  }

  #[napi]
  pub fn bool_vector(values: Vec<bool>) -> napi::Result<Self> {
    propvariant_vector(dynwinrt::com::PropVariantVector::Bool(values))
  }

  #[napi]
  pub fn string_vector(values: Vec<String>) -> napi::Result<Self> {
    propvariant_vector(dynwinrt::com::PropVariantVector::String(values))
  }

  #[napi]
  pub fn guid_vector(values: Vec<&WinGUID>) -> napi::Result<Self> {
    propvariant_vector(dynwinrt::com::PropVariantVector::Guid(
      values.into_iter().map(|value| value.0).collect(),
    ))
  }

  #[napi]
  pub fn filetime_vector(values: Vec<BigInt>) -> napi::Result<Self> {
    let values = values
      .into_iter()
      .map(|value| bigint_u64(value, "PROPVARIANT VT_VECTOR|VT_FILETIME"))
      .collect::<napi::Result<Vec<_>>>()?;
    propvariant_vector(dynwinrt::com::PropVariantVector::FileTime(values))
  }

  #[napi(getter)]
  pub fn vartype(&self) -> napi::Result<u32> {
    Ok(u32::from(self.value()?.vartype()))
  }

  #[napi(getter)]
  pub fn kind(&self) -> napi::Result<String> {
    Ok(propvariant_kind(self.value()?.propvariant_type().map_err(com_error)?).into())
  }

  #[napi]
  pub fn to_number(&self) -> napi::Result<f64> {
    match self.value()?.data().map_err(com_error)? {
      dynwinrt::com::PropVariantData::I8(value) => Ok(f64::from(value)),
      dynwinrt::com::PropVariantData::U8(value) => Ok(f64::from(value)),
      dynwinrt::com::PropVariantData::I16(value) => Ok(f64::from(value)),
      dynwinrt::com::PropVariantData::U16(value) => Ok(f64::from(value)),
      dynwinrt::com::PropVariantData::I32(value) | dynwinrt::com::PropVariantData::Int(value) => {
        Ok(f64::from(value))
      }
      dynwinrt::com::PropVariantData::U32(value) | dynwinrt::com::PropVariantData::UInt(value) => {
        Ok(f64::from(value))
      }
      dynwinrt::com::PropVariantData::F32(value) => Ok(f64::from(value)),
      dynwinrt::com::PropVariantData::F64(value) => Ok(value),
      _ => Err(napi::Error::from_reason(
        "PROPVARIANT does not contain a number-sized scalar",
      )),
    }
  }

  #[napi]
  pub fn to_bigint(&self) -> napi::Result<BigInt> {
    match self.value()?.data().map_err(com_error)? {
      dynwinrt::com::PropVariantData::I64(value) => Ok(BigInt::from(value)),
      dynwinrt::com::PropVariantData::U64(value)
      | dynwinrt::com::PropVariantData::FileTime(value) => Ok(BigInt::from(value)),
      _ => Err(napi::Error::from_reason(
        "PROPVARIANT does not contain a 64-bit integer or FILETIME",
      )),
    }
  }

  #[napi]
  pub fn to_bool(&self) -> napi::Result<bool> {
    match self.value()?.data().map_err(com_error)? {
      dynwinrt::com::PropVariantData::Bool(value) => Ok(value),
      _ => Err(napi::Error::from_reason(
        "PROPVARIANT does not contain VT_BOOL",
      )),
    }
  }

  #[napi(js_name = "toStringValue")]
  pub fn to_string_value(&self) -> napi::Result<String> {
    match self.value()?.data().map_err(com_error)? {
      dynwinrt::com::PropVariantData::String(value) => Ok(value),
      _ => Err(napi::Error::from_reason(
        "PROPVARIANT does not contain VT_LPWSTR",
      )),
    }
  }

  #[napi]
  pub fn to_guid_string(&self) -> napi::Result<String> {
    match self.value()?.data().map_err(com_error)? {
      dynwinrt::com::PropVariantData::Guid(value) => Ok(format!("{value:?}")),
      _ => Err(napi::Error::from_reason(
        "PROPVARIANT does not contain VT_CLSID",
      )),
    }
  }

  #[napi]
  pub fn to_blob(&self) -> napi::Result<Buffer> {
    match self.value()?.data().map_err(com_error)? {
      dynwinrt::com::PropVariantData::Blob(value) => Ok(Buffer::from(value)),
      _ => Err(napi::Error::from_reason(
        "PROPVARIANT does not contain VT_BLOB",
      )),
    }
  }

  #[napi]
  pub fn to_numbers(&self) -> napi::Result<Vec<f64>> {
    match self.value()?.data().map_err(com_error)? {
      dynwinrt::com::PropVariantData::Vector(value) => propvariant_vector_numbers(value),
      _ => Err(napi::Error::from_reason(
        "PROPVARIANT does not contain a numeric vector",
      )),
    }
  }

  #[napi]
  pub fn to_bigints(&self) -> napi::Result<Vec<BigInt>> {
    match self.value()?.data().map_err(com_error)? {
      dynwinrt::com::PropVariantData::Vector(dynwinrt::com::PropVariantVector::I64(values)) => {
        Ok(values.into_iter().map(BigInt::from).collect())
      }
      dynwinrt::com::PropVariantData::Vector(dynwinrt::com::PropVariantVector::U64(values))
      | dynwinrt::com::PropVariantData::Vector(dynwinrt::com::PropVariantVector::FileTime(
        values,
      )) => Ok(values.into_iter().map(BigInt::from).collect()),
      _ => Err(napi::Error::from_reason(
        "PROPVARIANT does not contain a 64-bit integer or FILETIME vector",
      )),
    }
  }

  #[napi]
  pub fn to_bools(&self) -> napi::Result<Vec<bool>> {
    match self.value()?.data().map_err(com_error)? {
      dynwinrt::com::PropVariantData::Vector(dynwinrt::com::PropVariantVector::Bool(values)) => {
        Ok(values)
      }
      _ => Err(napi::Error::from_reason(
        "PROPVARIANT does not contain a VT_BOOL vector",
      )),
    }
  }

  #[napi]
  pub fn to_strings(&self) -> napi::Result<Vec<String>> {
    match self.value()?.data().map_err(com_error)? {
      dynwinrt::com::PropVariantData::Vector(dynwinrt::com::PropVariantVector::String(values)) => {
        Ok(values)
      }
      _ => Err(napi::Error::from_reason(
        "PROPVARIANT does not contain a VT_LPWSTR vector",
      )),
    }
  }

  #[napi]
  pub fn to_guid_strings(&self) -> napi::Result<Vec<String>> {
    match self.value()?.data().map_err(com_error)? {
      dynwinrt::com::PropVariantData::Vector(dynwinrt::com::PropVariantVector::Guid(values)) => Ok(
        values
          .into_iter()
          .map(|value| format!("{value:?}"))
          .collect(),
      ),
      _ => Err(napi::Error::from_reason(
        "PROPVARIANT does not contain a VT_CLSID vector",
      )),
    }
  }

  #[napi]
  pub fn release(&mut self) {
    self.value = None;
  }
}

fn propvariant_vector(value: dynwinrt::com::PropVariantVector) -> napi::Result<DynComPropVariant> {
  dynwinrt::com::PropVariantValue::from_vector(value)
    .map(DynComPropVariant::new)
    .map_err(com_error)
}

fn propvariant_vector_numbers(value: dynwinrt::com::PropVariantVector) -> napi::Result<Vec<f64>> {
  Ok(match value {
    dynwinrt::com::PropVariantVector::I8(values) => values.into_iter().map(f64::from).collect(),
    dynwinrt::com::PropVariantVector::U8(values) => values.into_iter().map(f64::from).collect(),
    dynwinrt::com::PropVariantVector::I16(values) => values.into_iter().map(f64::from).collect(),
    dynwinrt::com::PropVariantVector::U16(values) => values.into_iter().map(f64::from).collect(),
    dynwinrt::com::PropVariantVector::I32(values) => values.into_iter().map(f64::from).collect(),
    dynwinrt::com::PropVariantVector::U32(values) => values.into_iter().map(f64::from).collect(),
    dynwinrt::com::PropVariantVector::F32(values) => values.into_iter().map(f64::from).collect(),
    dynwinrt::com::PropVariantVector::F64(values) => values,
    _ => {
      return Err(napi::Error::from_reason(
        "PROPVARIANT vector element type is not number-sized",
      ));
    }
  })
}

fn propvariant_kind(typ: dynwinrt::com::PropVariantType) -> &'static str {
  match typ {
    dynwinrt::com::PropVariantType::Empty => "empty",
    dynwinrt::com::PropVariantType::Null => "null",
    dynwinrt::com::PropVariantType::I8 => "i8",
    dynwinrt::com::PropVariantType::U8 => "u8",
    dynwinrt::com::PropVariantType::I16 => "i16",
    dynwinrt::com::PropVariantType::U16 => "u16",
    dynwinrt::com::PropVariantType::I32 => "i32",
    dynwinrt::com::PropVariantType::U32 => "u32",
    dynwinrt::com::PropVariantType::I64 => "i64",
    dynwinrt::com::PropVariantType::U64 => "u64",
    dynwinrt::com::PropVariantType::Int => "int",
    dynwinrt::com::PropVariantType::UInt => "uint",
    dynwinrt::com::PropVariantType::F32 => "f32",
    dynwinrt::com::PropVariantType::F64 => "f64",
    dynwinrt::com::PropVariantType::Bool => "bool",
    dynwinrt::com::PropVariantType::String => "string",
    dynwinrt::com::PropVariantType::Guid => "guid",
    dynwinrt::com::PropVariantType::FileTime => "filetime",
    dynwinrt::com::PropVariantType::Blob => "blob",
    dynwinrt::com::PropVariantType::Vector(_) => "vector",
  }
}

#[napi]
pub fn initialize_com(apartment_type: Option<i32>) -> napi::Result<()> {
  let apartment_type = match apartment_type.unwrap_or(1) {
    0 => dynwinrt::com::ApartmentType::SingleThreaded,
    _ => dynwinrt::com::ApartmentType::MultiThreaded,
  };
  dynwinrt::com::initialize_apartment(apartment_type)
    .map_err(|error| napi::Error::from_reason(error.message()))
}

#[napi]
impl DynComNativeStruct {
  #[napi(getter)]
  pub fn length(&self) -> u32 {
    self.bytes.len() as u32
  }

  #[napi(getter)]
  pub fn bytes(&self) -> Buffer {
    Buffer::from(self.bytes.clone())
  }
}

#[napi]
impl DynCom {
  #[napi]
  pub fn initialize(apartment_type: Option<i32>) -> napi::Result<()> {
    initialize_com(apartment_type)
  }

  #[napi(js_name = "registerIUnknownInterface")]
  pub fn register_iunknown_interface(name: String, iid: &WinGUID) -> DynComInterface {
    DynComInterface(dynwinrt::com::register_interface(
      &TABLE,
      &name,
      iid.0,
      dynwinrt::com::InterfaceBase::IUnknown,
    ))
  }

  #[napi(js_name = "registerIInspectableInterface")]
  pub fn register_iinspectable_interface(name: String, iid: &WinGUID) -> DynComInterface {
    DynComInterface(dynwinrt::com::register_interface(
      &TABLE,
      &name,
      iid.0,
      dynwinrt::com::InterfaceBase::IInspectable,
    ))
  }

  #[napi]
  pub fn bool_type() -> DynComType {
    DynComType(dynwinrt::com::Type::winrt(TABLE.bool_type()))
  }

  #[napi]
  pub fn i8_type() -> DynComType {
    DynComType(dynwinrt::com::Type::winrt(TABLE.i8_type()))
  }

  #[napi]
  pub fn u8_type() -> DynComType {
    DynComType(dynwinrt::com::Type::winrt(TABLE.u8_type()))
  }

  #[napi]
  pub fn i16_type() -> DynComType {
    DynComType(dynwinrt::com::Type::winrt(TABLE.i16_type()))
  }

  #[napi]
  pub fn u16_type() -> DynComType {
    DynComType(dynwinrt::com::Type::winrt(TABLE.u16_type()))
  }

  #[napi]
  pub fn i32_type() -> DynComType {
    DynComType(dynwinrt::com::Type::winrt(TABLE.i32_type()))
  }

  #[napi]
  pub fn u32_type() -> DynComType {
    DynComType(dynwinrt::com::Type::winrt(TABLE.u32_type()))
  }

  #[napi]
  pub fn i64_type() -> DynComType {
    DynComType(dynwinrt::com::Type::winrt(TABLE.i64_type()))
  }

  #[napi]
  pub fn u64_type() -> DynComType {
    DynComType(dynwinrt::com::Type::winrt(TABLE.u64_type()))
  }

  #[napi]
  pub fn isize_type() -> DynComType {
    #[cfg(target_pointer_width = "64")]
    {
      DynComType(dynwinrt::com::Type::winrt(TABLE.i64_type()))
    }
    #[cfg(target_pointer_width = "32")]
    {
      DynComType(dynwinrt::com::Type::winrt(TABLE.i32_type()))
    }
  }

  #[napi]
  pub fn usize_type() -> DynComType {
    #[cfg(target_pointer_width = "64")]
    {
      DynComType(dynwinrt::com::Type::winrt(TABLE.u64_type()))
    }
    #[cfg(target_pointer_width = "32")]
    {
      DynComType(dynwinrt::com::Type::winrt(TABLE.u32_type()))
    }
  }

  #[napi]
  pub fn f32_type() -> DynComType {
    DynComType(dynwinrt::com::Type::winrt(TABLE.f32_type()))
  }

  #[napi]
  pub fn f64_type() -> DynComType {
    DynComType(dynwinrt::com::Type::winrt(TABLE.f64_type()))
  }

  #[napi]
  pub fn char16_type() -> DynComType {
    DynComType(dynwinrt::com::Type::winrt(TABLE.char16_type()))
  }

  #[napi]
  pub fn guid_type() -> DynComType {
    DynComType(dynwinrt::com::Type::winrt(TABLE.guid_type()))
  }

  #[napi]
  pub fn hstring_type() -> DynComType {
    DynComType(dynwinrt::com::Type::winrt(TABLE.hstring()))
  }

  #[napi]
  pub fn hstring(value: String) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::HString(windows::core::HSTRING::from(
      value,
    )))
  }

  #[napi]
  pub fn pointer_type() -> DynComType {
    DynComType(dynwinrt::com::Type::pointer())
  }

  #[napi]
  pub fn borrowed_handle_output_type() -> DynComType {
    DynComType(dynwinrt::com::Type::borrowed_handle_output())
  }

  #[napi]
  pub fn owned_com_pointer_type() -> DynComType {
    DynComType(dynwinrt::com::Type::owned_com_pointer())
  }

  #[napi]
  pub fn co_task_mem_pointer_type() -> DynComType {
    DynComType(dynwinrt::com::Type::co_task_mem_pointer())
  }

  #[napi]
  pub fn co_task_mem_wide_string_type() -> DynComType {
    DynComType(dynwinrt::com::Type::co_task_mem_wide_string())
  }

  #[napi]
  pub fn bstr_pointer_type() -> DynComType {
    DynComType(dynwinrt::com::Type::bstr_pointer())
  }

  #[napi]
  pub fn bstr_type() -> DynComType {
    DynComType(dynwinrt::com::Type::bstr())
  }

  #[napi]
  pub fn nullable_bstr_type() -> DynComType {
    DynComType(dynwinrt::com::Type::nullable_bstr())
  }

  #[napi]
  pub fn bstr(value: String) -> DynWinRTValue {
    DynWinRTValue::from_com_value(
      dynwinrt::com::Value::Bstr(dynwinrt::com::BstrValue::new(value)),
      dynwinrt::com::PointerOutputKind::None,
    )
  }

  #[napi]
  pub fn null_bstr() -> DynWinRTValue {
    DynWinRTValue::from_com_value(
      dynwinrt::com::Value::Bstr(dynwinrt::com::BstrValue::null()),
      dynwinrt::com::PointerOutputKind::None,
    )
  }

  #[napi]
  pub fn native_struct_type(descriptor: String) -> napi::Result<DynComType> {
    native_struct_layout(&descriptor)
      .map(dynwinrt::com::Type::native_struct)
      .map(DynComType)
  }

  #[napi]
  pub fn native_struct_pointer_type(
    descriptor: String,
    nullable: Option<bool>,
  ) -> napi::Result<DynComType> {
    native_struct_layout(&descriptor)
      .map(|layout| {
        if nullable.unwrap_or(false) {
          dynwinrt::com::Type::nullable_native_struct_pointer(layout)
        } else {
          dynwinrt::com::Type::native_struct_pointer(layout)
        }
      })
      .map(DynComType)
  }

  #[napi]
  pub fn native_union_pointer_type(descriptor: String) -> napi::Result<DynComType> {
    native_union_layout(&descriptor)
      .map(dynwinrt::com::Type::native_union_pointer)
      .map(DynComType)
  }

  #[napi]
  pub fn variant_type() -> DynComType {
    DynComType(dynwinrt::com::Type::variant())
  }

  #[napi]
  pub fn variant_by_value_type() -> DynComType {
    DynComType(dynwinrt::com::Type::variant_by_value())
  }

  #[napi]
  pub fn safe_array_type(
    element_type: String,
    interface_iid: Option<&WinGUID>,
    nullable: Option<bool>,
  ) -> napi::Result<DynComType> {
    let element_type = safe_array_element_type_from_name(&element_type)?;
    match (interface_iid, nullable.unwrap_or(false)) {
      (Some(iid), false) if element_type == dynwinrt::com::SafeArrayElementType::Unknown => Ok(
        DynComType(dynwinrt::com::Type::typed_interface_safe_array(iid.0)),
      ),
      (Some(iid), true) if element_type == dynwinrt::com::SafeArrayElementType::Unknown => {
        Ok(DynComType(
          dynwinrt::com::Type::nullable_typed_interface_safe_array(iid.0),
        ))
      }
      (Some(_), _) => Err(napi::Error::from_reason(
        "Exact SAFEARRAY interface IID requires VT_UNKNOWN elements",
      )),
      (None, false) => Ok(DynComType(dynwinrt::com::Type::typed_safe_array(
        element_type,
      ))),
      (None, true) => Ok(DynComType(dynwinrt::com::Type::nullable_typed_safe_array(
        element_type,
      ))),
    }
  }

  #[napi]
  pub fn prop_variant_type() -> DynComType {
    DynComType(dynwinrt::com::Type::prop_variant())
  }

  #[napi]
  pub fn dispatch_params_type() -> DynComType {
    DynComType(dynwinrt::com::Type::dispatch_params())
  }

  #[napi]
  pub fn excep_info_type() -> DynComType {
    DynComType(dynwinrt::com::Type::excep_info())
  }

  #[napi]
  pub fn stat_stg_type() -> DynComType {
    DynComType(dynwinrt::com::Type::stat_stg())
  }

  #[napi]
  pub fn interface_type(iid: &WinGUID) -> DynComType {
    DynComType(dynwinrt::com::Type::winrt(TABLE.interface(iid.0)))
  }

  #[napi]
  pub fn bool_value(value: bool) -> DynWinRTValue {
    DynWinRTValue::bool_value(value)
  }

  #[napi]
  pub fn i8_value(value: i32) -> DynWinRTValue {
    DynWinRTValue::i8_value(value)
  }

  #[napi]
  pub fn u8_value(value: u32) -> DynWinRTValue {
    DynWinRTValue::u8_value(value)
  }

  #[napi]
  pub fn i16(value: i32) -> DynWinRTValue {
    DynWinRTValue::i16(value)
  }

  #[napi]
  pub fn u16(value: u32) -> DynWinRTValue {
    DynWinRTValue::u16(value)
  }

  #[napi]
  pub fn i32(value: i32) -> DynWinRTValue {
    DynWinRTValue::i32(value)
  }

  #[napi]
  pub fn u32(value: u32) -> DynWinRTValue {
    DynWinRTValue::u32(value)
  }

  #[napi]
  pub fn i64(value: BigInt) -> napi::Result<DynWinRTValue> {
    let (value, lossless) = value.get_i64();
    if !lossless {
      return Err(napi::Error::from_reason(
        "DynCom.i64(): value must fit in a signed 64-bit integer",
      ));
    }
    Ok(DynWinRTValue::new(dynwinrt::WinRTValue::I64(value)))
  }

  #[napi]
  pub fn u64(value: BigInt) -> napi::Result<DynWinRTValue> {
    let (negative, value, lossless) = value.get_u64();
    if negative || !lossless {
      return Err(napi::Error::from_reason(
        "DynCom.u64(): value must fit in an unsigned 64-bit integer",
      ));
    }
    Ok(DynWinRTValue::new(dynwinrt::WinRTValue::U64(value)))
  }

  #[napi]
  pub fn isize(value: BigInt) -> napi::Result<DynWinRTValue> {
    let (value, lossless) = value.get_i64();
    if !lossless {
      return Err(napi::Error::from_reason(
        "DynCom.isize(): value must fit in a pointer-sized signed integer",
      ));
    }
    #[cfg(target_pointer_width = "64")]
    {
      Ok(DynWinRTValue::new(dynwinrt::WinRTValue::I64(value)))
    }
    #[cfg(target_pointer_width = "32")]
    {
      let value = i32::try_from(value).map_err(|_| {
        napi::Error::from_reason("DynCom.isize(): value must fit in a pointer-sized signed integer")
      })?;
      Ok(DynWinRTValue::new(dynwinrt::WinRTValue::I32(value)))
    }
  }

  #[napi]
  pub fn usize(value: BigInt) -> napi::Result<DynWinRTValue> {
    let (negative, value, lossless) = value.get_u64();
    if negative || !lossless {
      return Err(napi::Error::from_reason(
        "DynCom.usize(): value must fit in a pointer-sized unsigned integer",
      ));
    }
    #[cfg(target_pointer_width = "64")]
    {
      Ok(DynWinRTValue::new(dynwinrt::WinRTValue::U64(value)))
    }
    #[cfg(target_pointer_width = "32")]
    {
      let value = u32::try_from(value).map_err(|_| {
        napi::Error::from_reason(
          "DynCom.usize(): value must fit in a pointer-sized unsigned integer",
        )
      })?;
      Ok(DynWinRTValue::new(dynwinrt::WinRTValue::U32(value)))
    }
  }

  #[napi]
  pub fn f32(value: f64) -> DynWinRTValue {
    DynWinRTValue::f32(value)
  }

  #[napi]
  pub fn f64(value: f64) -> DynWinRTValue {
    DynWinRTValue::f64(value)
  }

  #[napi]
  pub fn char16(value: u32) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::U16(value as u16))
  }

  #[napi]
  pub fn guid(value: &WinGUID) -> DynWinRTValue {
    DynWinRTValue::guid(value)
  }

  #[napi]
  pub fn co_create_instance(clsid: String, iid: &WinGUID) -> napi::Result<DynWinRTValue> {
    self::co_create_instance(clsid, iid)
  }

  #[napi]
  pub fn co_get_class_object(clsid: String, iid: &WinGUID) -> napi::Result<DynWinRTValue> {
    self::co_get_class_object(clsid, iid)
  }

  #[napi]
  pub fn co_get_malloc() -> napi::Result<DynWinRTValue> {
    self::co_get_malloc()
  }

  #[napi]
  pub fn create_error_info() -> napi::Result<DynWinRTValue> {
    self::create_error_info()
  }

  #[napi]
  pub fn set_error_info(
    #[napi(ts_arg_type = "DynWinRtValue | null | undefined")] value: Option<&DynWinRTValue>,
  ) -> napi::Result<()> {
    self::set_error_info(value)
  }

  #[napi]
  pub fn get_error_info() -> napi::Result<Option<DynWinRTValue>> {
    self::get_error_info()
  }

  #[napi]
  pub fn try_cast(value: &DynWinRTValue, iid: &WinGUID) -> napi::Result<Option<DynWinRTValue>> {
    self::try_cast(value, iid)
  }

  #[napi]
  pub fn bind_com_object(value: &mut DynWinRTValue) -> napi::Result<()> {
    value.bind_current_com_apartment()
  }

  #[napi]
  pub fn pointer(
    #[napi(ts_arg_type = "bigint | number | Buffer | Uint8Array | null | undefined")]
    value: Unknown,
  ) -> napi::Result<DynWinRTValue> {
    self::pointer(value)
  }

  #[napi]
  pub fn malloc_allocation_pointer(
    allocator: &DynWinRTValue,
    allocation: Option<&DynComAllocation>,
  ) -> napi::Result<DynWinRTValue> {
    self::malloc_allocation_pointer(allocator, allocation)
  }

  #[napi]
  pub fn malloc_inspection_pointer(
    allocation: Option<&DynComAllocation>,
  ) -> napi::Result<DynWinRTValue> {
    self::malloc_inspection_pointer(allocation)
  }

  #[napi]
  pub fn take_malloc_allocation_pointer(
    allocator: &DynWinRTValue,
    allocation: Option<&mut DynComAllocation>,
  ) -> napi::Result<DynWinRTValue> {
    self::take_malloc_allocation_pointer(allocator, allocation)
  }

  #[napi]
  pub fn take_malloc_allocation(
    allocator: &DynWinRTValue,
    value: &mut DynWinRTValue,
  ) -> napi::Result<Option<DynComAllocation>> {
    self::take_malloc_allocation(allocator, value)
  }

  #[napi]
  pub fn finish_malloc_reallocation(
    allocator: &DynWinRTValue,
    allocation: Option<&mut DynComAllocation>,
    size: BigInt,
    value: &mut DynWinRTValue,
  ) -> napi::Result<Option<DynComAllocation>> {
    self::finish_malloc_reallocation(allocator, allocation, size, value)
  }

  #[napi]
  pub fn safe_data_pointer(
    #[napi(ts_arg_type = "Buffer | Uint8Array | null | undefined")] value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWinRTValue> {
    self::safe_data_pointer(value, nullable.unwrap_or(false))
  }

  #[napi]
  pub fn safe_wide_string_pointer(
    #[napi(ts_arg_type = "string | Buffer | Uint8Array | null | undefined")] value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWinRTValue> {
    self::safe_wide_string_pointer(value, nullable.unwrap_or(false))
  }

  #[napi]
  pub fn safe_ansi_string_pointer(
    #[napi(ts_arg_type = "string | Buffer | Uint8Array | null | undefined")] value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWinRTValue> {
    self::safe_ansi_string_pointer(value, nullable.unwrap_or(false))
  }

  #[napi]
  pub fn null_buffer() -> DynWinRTValue {
    DynWinRTValue::from_com_value(
      dynwinrt::com::Value::Buffer(dynwinrt::com::ComBufferValue::null()),
      dynwinrt::com::PointerOutputKind::None,
    )
  }

  #[napi]
  pub fn null_native_struct_pointer() -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::Null)
  }

  #[napi]
  pub fn is_null_native_struct_pointer(value: &DynWinRTValue) -> bool {
    value.3.is_none() && value.0.is_null_object()
  }

  #[napi]
  pub fn null_com_value() -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::Null)
  }

  #[napi]
  pub fn buffer(
    #[napi(ts_arg_type = "Buffer | ArrayBufferView")] value: Unknown,
  ) -> napi::Result<DynWinRTValue> {
    self::com_buffer(value)
  }

  #[napi]
  pub fn wide_string_array(values: Vec<String>) -> napi::Result<DynWinRTValue> {
    dynwinrt::com::ComBufferValue::string_array(values, dynwinrt::com::StringEncoding::Utf16)
      .map(|value| {
        DynWinRTValue::from_com_value(
          dynwinrt::com::Value::Buffer(value),
          dynwinrt::com::PointerOutputKind::None,
        )
      })
      .map_err(com_error)
  }

  #[napi]
  pub fn ansi_string_array(values: Vec<String>) -> napi::Result<DynWinRTValue> {
    dynwinrt::com::ComBufferValue::string_array(values, dynwinrt::com::StringEncoding::Ansi)
      .map(|value| {
        DynWinRTValue::from_com_value(
          dynwinrt::com::Value::Buffer(value),
          dynwinrt::com::PointerOutputKind::None,
        )
      })
      .map_err(com_error)
  }

  #[napi]
  pub fn interface_array(
    iid: &WinGUID,
    values: Vec<&DynWinRTValue>,
  ) -> napi::Result<DynWinRTValue> {
    let values = values
      .into_iter()
      .map(|value| {
        value
          .0
          .as_object()
          .ok_or_else(|| napi::Error::from_reason("COM interface arrays require managed objects"))
      })
      .collect::<napi::Result<Vec<_>>>()?;
    dynwinrt::com::ComBufferValue::interface_array(iid.0, values)
      .map(|value| {
        DynWinRTValue::from_com_value(
          dynwinrt::com::Value::Buffer(value),
          dynwinrt::com::PointerOutputKind::None,
        )
      })
      .map_err(com_error)
  }

  #[napi]
  pub fn bstr_array(values: Vec<String>) -> DynWinRTValue {
    DynWinRTValue::from_com_value(
      dynwinrt::com::Value::Buffer(dynwinrt::com::ComBufferValue::bstr_array(values)),
      dynwinrt::com::PointerOutputKind::None,
    )
  }

  #[napi]
  pub fn variant_array(values: Vec<&DynComVariant>) -> napi::Result<DynWinRTValue> {
    let values = values
      .into_iter()
      .map(|value| value.value().cloned())
      .collect::<napi::Result<Vec<_>>>()?;
    dynwinrt::com::ComBufferValue::variant_array(values)
      .map(|value| {
        DynWinRTValue::from_com_value(
          dynwinrt::com::Value::Buffer(value),
          dynwinrt::com::PointerOutputKind::None,
        )
      })
      .map_err(com_error)
  }

  #[napi]
  pub fn caller_output_array(
    element_type: &DynComType,
    count: BigInt,
  ) -> napi::Result<DynWinRTValue> {
    let (negative, count, lossless) = count.get_u64();
    if negative || !lossless || count as usize as u64 != count {
      return Err(napi::Error::from_reason(
        "callerOutputArray(): count must fit in an unsigned pointer-sized integer",
      ));
    }
    dynwinrt::com::ComBufferValue::caller_output(&element_type.0, count as usize)
      .map(|value| {
        DynWinRTValue::from_com_value(
          dynwinrt::com::Value::Buffer(value),
          dynwinrt::com::PointerOutputKind::None,
        )
      })
      .map_err(com_error)
  }

  #[napi]
  pub fn enumerator_output_array(
    element_type: &DynComType,
    count: BigInt,
  ) -> napi::Result<DynWinRTValue> {
    let (negative, count, lossless) = count.get_u64();
    if negative || !lossless || count as usize as u64 != count {
      return Err(napi::Error::from_reason(
        "enumeratorOutputArray(): count must fit in an unsigned pointer-sized integer",
      ));
    }
    dynwinrt::com::ComBufferValue::enumerator_output(&element_type.0, count as usize)
      .map(|value| {
        DynWinRTValue::from_com_value(
          dynwinrt::com::Value::Buffer(value),
          dynwinrt::com::PointerOutputKind::None,
        )
      })
      .map_err(com_error)
  }

  #[napi]
  pub fn take_buffer(value: &mut DynWinRTValue) -> napi::Result<Buffer> {
    let buffer = value
      .4
      .take()
      .ok_or_else(|| napi::Error::from_reason("Value is not an owned COM buffer result"))?;
    let bytes = buffer
      .snapshot_bytes()
      .map_err(com_error)?
      .ok_or_else(|| napi::Error::from_reason("Borrowed COM buffers cannot be consumed"))?;
    Ok(Buffer::from(bytes))
  }

  #[napi]
  pub fn take_i8_array(value: &mut DynWinRTValue) -> napi::Result<Vec<i8>> {
    Ok(
      take_array_bytes(value, 1)?
        .into_iter()
        .map(|value| value as i8)
        .collect(),
    )
  }

  #[napi]
  pub fn take_u8_array(value: &mut DynWinRTValue) -> napi::Result<Vec<u8>> {
    take_array_bytes(value, 1)
  }

  #[napi]
  pub fn take_i16_array(value: &mut DynWinRTValue) -> napi::Result<Vec<i16>> {
    Ok(
      take_array_bytes(value, 2)?
        .chunks_exact(2)
        .map(|bytes| i16::from_ne_bytes(bytes.try_into().unwrap()))
        .collect(),
    )
  }

  #[napi]
  pub fn take_u16_array(value: &mut DynWinRTValue) -> napi::Result<Vec<u16>> {
    Ok(
      take_array_bytes(value, 2)?
        .chunks_exact(2)
        .map(|bytes| u16::from_ne_bytes(bytes.try_into().unwrap()))
        .collect(),
    )
  }

  #[napi]
  pub fn take_i32_array(value: &mut DynWinRTValue) -> napi::Result<Vec<i32>> {
    Ok(
      take_array_bytes(value, 4)?
        .chunks_exact(4)
        .map(|bytes| i32::from_ne_bytes(bytes.try_into().unwrap()))
        .collect(),
    )
  }

  #[napi]
  pub fn take_u32_array(value: &mut DynWinRTValue) -> napi::Result<Vec<u32>> {
    Ok(
      take_array_bytes(value, 4)?
        .chunks_exact(4)
        .map(|bytes| u32::from_ne_bytes(bytes.try_into().unwrap()))
        .collect(),
    )
  }

  #[napi]
  pub fn take_i64_array(value: &mut DynWinRTValue) -> napi::Result<Vec<BigInt>> {
    Ok(
      take_array_bytes(value, 8)?
        .chunks_exact(8)
        .map(|bytes| BigInt::from(i64::from_ne_bytes(bytes.try_into().unwrap())))
        .collect(),
    )
  }

  #[napi]
  pub fn take_u64_array(value: &mut DynWinRTValue) -> napi::Result<Vec<BigInt>> {
    Ok(
      take_array_bytes(value, 8)?
        .chunks_exact(8)
        .map(|bytes| BigInt::from(u64::from_ne_bytes(bytes.try_into().unwrap())))
        .collect(),
    )
  }

  #[napi]
  pub fn take_f32_array(value: &mut DynWinRTValue) -> napi::Result<Vec<f64>> {
    Ok(
      take_array_bytes(value, 4)?
        .chunks_exact(4)
        .map(|bytes| f32::from_ne_bytes(bytes.try_into().unwrap()) as f64)
        .collect(),
    )
  }

  #[napi]
  pub fn take_f64_array(value: &mut DynWinRTValue) -> napi::Result<Vec<f64>> {
    Ok(
      take_array_bytes(value, 8)?
        .chunks_exact(8)
        .map(|bytes| f64::from_ne_bytes(bytes.try_into().unwrap()))
        .collect(),
    )
  }

  #[napi]
  pub fn take_bool_array(value: &mut DynWinRTValue) -> napi::Result<Vec<bool>> {
    Ok(
      take_array_bytes(value, 1)?
        .into_iter()
        .map(|value| value != 0)
        .collect(),
    )
  }

  #[napi]
  pub fn take_guid_array(value: &mut DynWinRTValue) -> napi::Result<Vec<String>> {
    Ok(
      take_array_bytes(value, std::mem::size_of::<windows::core::GUID>())?
        .chunks_exact(std::mem::size_of::<windows::core::GUID>())
        .map(|bytes| {
          let guid =
            unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<windows::core::GUID>()) };
          format!("{guid:?}")
        })
        .collect(),
    )
  }

  #[napi]
  pub fn take_native_struct_array(
    value: &mut DynWinRTValue,
    descriptor: String,
  ) -> napi::Result<Vec<DynComNativeStruct>> {
    let layout = native_struct_layout(&descriptor)?;
    Ok(
      take_array_bytes(value, layout.size())?
        .chunks_exact(layout.size())
        .map(|bytes| DynComNativeStruct {
          descriptor: descriptor.clone(),
          bytes: bytes.to_vec(),
        })
        .collect(),
    )
  }

  #[napi]
  pub fn take_com_array(value: &mut DynWinRTValue) -> napi::Result<Vec<DynWinRTValue>> {
    value
      .4
      .take()
      .ok_or_else(|| napi::Error::from_reason("Value is not a managed COM array result"))?
      .into_com_values()
      .map(|values| values.into_iter().map(DynWinRTValue::new).collect())
      .map_err(com_error)
  }

  #[napi]
  pub fn take_bstr_array(value: &mut DynWinRTValue) -> napi::Result<Vec<String>> {
    value
      .4
      .take()
      .ok_or_else(|| napi::Error::from_reason("Value is not an owned BSTR array result"))?
      .into_strings()
      .map_err(com_error)
  }

  #[napi]
  pub fn take_co_task_mem_wide_string_array(
    value: &mut DynWinRTValue,
  ) -> napi::Result<Vec<String>> {
    Self::take_bstr_array(value)
  }

  #[napi]
  pub fn take_variant_array(value: &mut DynWinRTValue) -> napi::Result<Vec<DynComVariant>> {
    value
      .4
      .take()
      .ok_or_else(|| napi::Error::from_reason("Value is not an owned VARIANT array result"))?
      .into_variants()
      .map(|values| values.into_iter().map(DynComVariant::new).collect())
      .map_err(com_error)
  }

  #[napi]
  pub fn take_win32_bool_array(value: &mut DynWinRTValue) -> napi::Result<Vec<bool>> {
    Ok(
      take_array_bytes(value, 4)?
        .chunks_exact(4)
        .map(|bytes| i32::from_ne_bytes(bytes.try_into().unwrap()) != 0)
        .collect(),
    )
  }

  #[napi]
  pub fn take_isize_array(value: &mut DynWinRTValue) -> napi::Result<Vec<BigInt>> {
    #[cfg(target_pointer_width = "64")]
    {
      Self::take_i64_array(value)
    }
    #[cfg(target_pointer_width = "32")]
    {
      Ok(
        Self::take_i32_array(value)?
          .into_iter()
          .map(|value| BigInt::from(i64::from(value)))
          .collect(),
      )
    }
  }

  #[napi]
  pub fn take_usize_array(value: &mut DynWinRTValue) -> napi::Result<Vec<BigInt>> {
    #[cfg(target_pointer_width = "64")]
    {
      Self::take_u64_array(value)
    }
    #[cfg(target_pointer_width = "32")]
    {
      Ok(
        Self::take_u32_array(value)?
          .into_iter()
          .map(|value| BigInt::from(u64::from(value)))
          .collect(),
      )
    }
  }

  #[napi]
  pub fn buffer_count(value: &DynWinRTValue) -> napi::Result<BigInt> {
    value
      .4
      .as_ref()
      .map(|buffer| BigInt::from(buffer.count() as u64))
      .ok_or_else(|| napi::Error::from_reason("Value is not a COM buffer result"))
  }

  #[napi]
  pub fn buffer_element_count(
    value: &DynWinRTValue,
    element_type: &DynComType,
  ) -> napi::Result<BigInt> {
    value
      .4
      .as_ref()
      .ok_or_else(|| napi::Error::from_reason("Value is not a COM buffer"))
      .and_then(|buffer| {
        buffer
          .element_count(&element_type.0)
          .map(|count| BigInt::from(count as u64))
          .map_err(com_error)
      })
  }

  #[napi]
  pub fn buffer_allocation_length(value: BigInt) -> napi::Result<u32> {
    let (negative, value, lossless) = value.get_u64();
    if negative || !lossless || value > i32::MAX as u64 {
      return Err(napi::Error::from_reason(
        "COM buffer size exceeds the supported projected Buffer size",
      ));
    }
    Ok(value as u32)
  }

  #[napi]
  pub fn native_struct(
    descriptor: String,
    value: &DynComNativeStruct,
  ) -> napi::Result<DynWinRTValue> {
    let layout = native_struct_layout(&descriptor)?;
    if value.descriptor != descriptor {
      return Err(napi::Error::from_reason(format!(
        "Native struct type mismatch: expected `{}`, received a differently branded native struct",
        layout.name()
      )));
    }
    let value = dynwinrt::com::NativeStructValue::new(layout, value.bytes.clone())
      .map_err(|error| napi::Error::from_reason(error.message()))?;
    Ok(DynWinRTValue::from_com_value(
      dynwinrt::com::Value::NativeStruct(value),
      dynwinrt::com::PointerOutputKind::None,
    ))
  }

  #[napi]
  pub fn create_native_struct(
    descriptor: String,
    bytes: Option<Buffer>,
  ) -> napi::Result<DynComNativeStruct> {
    let layout = native_struct_layout(&descriptor)?;
    let value = match bytes {
      Some(bytes) => dynwinrt::com::NativeStructValue::new(layout, bytes.to_vec()),
      None => Ok(dynwinrt::com::NativeStructValue::zeroed(layout)),
    }
    .map_err(com_error)?;
    Ok(DynComNativeStruct {
      descriptor,
      bytes: value.bytes().to_vec(),
    })
  }

  #[napi]
  pub fn create_native_struct_array(
    descriptor: String,
    bytes: Buffer,
  ) -> napi::Result<DynComNativeStructArray> {
    let layout = native_struct_layout(&descriptor)?;
    dynwinrt::com::ComBufferValue::native_struct_input(bytes.to_vec(), &layout)
      .map_err(com_error)?;
    Ok(DynComNativeStructArray {
      descriptor,
      bytes: bytes.to_vec(),
    })
  }

  #[napi]
  pub fn native_struct_buffer(
    descriptor: String,
    value: &DynComNativeStructArray,
  ) -> napi::Result<DynWinRTValue> {
    let layout = native_struct_layout(&descriptor)?;
    if value.descriptor != descriptor {
      return Err(napi::Error::from_reason(format!(
        "Native struct array type mismatch: expected `{}`",
        layout.name()
      )));
    }
    let buffer = dynwinrt::com::ComBufferValue::native_struct_input(value.bytes.clone(), &layout)
      .map_err(com_error)?;
    Ok(DynWinRTValue::from_com_value(
      dynwinrt::com::Value::Buffer(buffer),
      dynwinrt::com::PointerOutputKind::None,
    ))
  }

  #[napi]
  pub fn native_struct_bytes(
    descriptor: String,
    value: &DynWinRTValue,
  ) -> napi::Result<DynComNativeStruct> {
    let expected = native_struct_layout(&descriptor)?;
    let value = value
      .3
      .as_ref()
      .ok_or_else(|| napi::Error::from_reason("Value is not a native COM struct"))?;
    if value.layout() != &expected {
      return Err(napi::Error::from_reason(format!(
        "Native struct type mismatch: expected `{}`, received `{}`",
        expected.name(),
        value.layout().name()
      )));
    }
    Ok(DynComNativeStruct {
      descriptor,
      bytes: value.bytes().to_vec(),
    })
  }

  #[napi]
  pub fn create_native_union(
    descriptor: String,
    active_field: String,
    bytes: Option<Buffer>,
  ) -> napi::Result<DynComNativeUnion> {
    let layout = native_union_layout(&descriptor)?;
    let value = match bytes {
      Some(bytes) => dynwinrt::com::NativeUnionValue::new(layout, active_field, bytes.to_vec()),
      None => dynwinrt::com::NativeUnionValue::zeroed(layout, active_field),
    }
    .map_err(com_error)?;
    Ok(DynComNativeUnion { descriptor, value })
  }

  #[napi]
  pub fn native_union(
    descriptor: String,
    value: &DynComNativeUnion,
  ) -> napi::Result<DynWinRTValue> {
    let layout = native_union_layout(&descriptor)?;
    if value.descriptor != descriptor || value.value.layout() != &layout {
      return Err(napi::Error::from_reason(format!(
        "Native union type mismatch: expected `{}`",
        layout.name()
      )));
    }
    Ok(DynWinRTValue::from_com_value(
      dynwinrt::com::Value::NativeUnion(value.value.clone()),
      dynwinrt::com::PointerOutputKind::None,
    ))
  }

  #[napi]
  pub fn variant(value: &DynComVariant) -> napi::Result<DynWinRTValue> {
    Ok(DynWinRTValue::from_com_value(
      dynwinrt::com::Value::Variant(value.value()?.clone()),
      dynwinrt::com::PointerOutputKind::None,
    ))
  }

  #[napi]
  pub fn safe_array(value: &DynComSafeArray) -> napi::Result<DynWinRTValue> {
    Ok(DynWinRTValue::from_com_value(
      dynwinrt::com::Value::SafeArray(value.value()?.clone()),
      dynwinrt::com::PointerOutputKind::None,
    ))
  }

  #[napi]
  pub fn prop_variant(value: &DynComPropVariant) -> napi::Result<DynWinRTValue> {
    Ok(DynWinRTValue::from_com_value(
      dynwinrt::com::Value::PropVariant(value.value()?.clone()),
      dynwinrt::com::PointerOutputKind::None,
    ))
  }

  #[napi]
  pub fn dispatch_params(value: &DynComDispatchParams) -> napi::Result<DynWinRTValue> {
    Ok(DynWinRTValue::from_com_value(
      dynwinrt::com::Value::DispatchParams(value.value()?.clone()),
      dynwinrt::com::PointerOutputKind::None,
    ))
  }

  #[napi]
  pub fn take_variant(value: &mut DynWinRTValue) -> napi::Result<DynComVariant> {
    let result = value
      .5
      .as_mut()
      .ok_or_else(|| napi::Error::from_reason("Value is not a COM VARIANT"))?
      .take_variant()?;
    value.5 = None;
    Ok(DynComVariant::new(result))
  }

  #[napi]
  pub fn take_safe_array(value: &mut DynWinRTValue) -> napi::Result<DynComSafeArray> {
    let result = value
      .5
      .as_mut()
      .ok_or_else(|| napi::Error::from_reason("Value is not a COM SAFEARRAY"))?
      .take_safe_array()?;
    value.5 = None;
    Ok(DynComSafeArray::new(result))
  }

  #[napi]
  pub fn take_nullable_safe_array(
    value: &mut DynWinRTValue,
  ) -> napi::Result<Option<DynComSafeArray>> {
    if value.5.is_none() && value.0.is_null_object() {
      return Ok(None);
    }
    Self::take_safe_array(value).map(Some)
  }

  #[napi]
  pub fn take_prop_variant(value: &mut DynWinRTValue) -> napi::Result<DynComPropVariant> {
    let result = value
      .5
      .as_mut()
      .ok_or_else(|| napi::Error::from_reason("Value is not a COM PROPVARIANT"))?
      .take_prop_variant()?;
    value.5 = None;
    Ok(DynComPropVariant::new(result))
  }

  #[napi]
  pub fn take_excep_info(value: &mut DynWinRTValue) -> napi::Result<DynComExcepInfo> {
    let result = value
      .5
      .as_mut()
      .ok_or_else(|| napi::Error::from_reason("Value is not COM EXCEPINFO"))?
      .take_excep_info()?;
    value.5 = None;
    Ok(DynComExcepInfo::new(result))
  }

  #[napi]
  pub fn take_stat_stg(value: &mut DynWinRTValue) -> napi::Result<DynComStatStg> {
    let result = value
      .5
      .as_mut()
      .ok_or_else(|| napi::Error::from_reason("Value is not COM STATSTG"))?
      .take_stat_stg()?;
    value.5 = None;
    Ok(DynComStatStg::new(result))
  }

  #[napi]
  pub fn wide_string_pointer(
    #[napi(ts_arg_type = "string | bigint | number | Buffer | Uint8Array | null | undefined")]
    value: Unknown,
  ) -> napi::Result<DynWinRTValue> {
    self::wide_string_pointer(value)
  }

  #[napi]
  pub fn ansi_string_pointer(
    #[napi(ts_arg_type = "string | bigint | number | Buffer | Uint8Array | null | undefined")]
    value: Unknown,
  ) -> napi::Result<DynWinRTValue> {
    self::ansi_string_pointer(value)
  }

  #[napi]
  pub fn handle_value(
    #[napi(ts_arg_type = "bigint | number | Buffer | Uint8Array")] value: Unknown,
  ) -> napi::Result<BigInt> {
    self::handle_value(value)
  }

  #[napi]
  pub fn iid_pointer(value: &WinGUID) -> DynWinRTValue {
    self::iid_pointer(value)
  }

  #[napi]
  pub fn adopt_com_pointer(
    value: &mut DynWinRTValue,
    iid: Option<&WinGUID>,
  ) -> napi::Result<DynWinRTValue> {
    self::adopt_com_pointer(value, iid)
  }

  #[napi]
  pub fn project_win_rt_async(
    value: &DynWinRTValue,
    async_type: &DynWinRTType,
  ) -> napi::Result<DynWinRTValue> {
    self::project_winrt_async(value, async_type)
  }

  #[napi]
  pub fn adopt_co_task_mem_pointer(value: &mut DynWinRTValue) -> napi::Result<DynWinRTValue> {
    self::adopt_co_task_mem_pointer(value)
  }

  #[napi]
  pub fn as_pointer_bigint(value: &DynWinRTValue) -> napi::Result<BigInt> {
    self::as_pointer_bigint(value)
  }

  #[napi]
  pub fn to_number(value: &DynWinRTValue) -> i32 {
    value.to_number()
  }

  #[napi]
  pub fn to_bool(value: &DynWinRTValue) -> bool {
    value.to_bool()
  }

  #[napi]
  pub fn to_f64(value: &DynWinRTValue) -> f64 {
    value.to_f64()
  }

  #[napi]
  pub fn to_guid_string(value: &DynWinRTValue) -> napi::Result<String> {
    value.to_guid().map(|guid| guid.to_string())
  }

  #[napi]
  pub fn take_co_task_mem_wide_string(value: &mut DynWinRTValue) -> napi::Result<String> {
    self::take_co_task_mem_wide_string(value)
  }

  #[napi]
  pub fn take_co_task_mem_ansi_string(value: &mut DynWinRTValue) -> napi::Result<String> {
    self::take_co_task_mem_ansi_string(value)
  }

  #[napi]
  pub fn take_bstr(value: &mut DynWinRTValue) -> napi::Result<String> {
    self::take_bstr(value)
  }

  #[napi]
  pub fn to_u32(value: &DynWinRTValue) -> napi::Result<u32> {
    match &value.0 {
      dynwinrt::WinRTValue::U32(value) => Ok(*value),
      _ => Err(napi::Error::from_reason("Value is not a u32")),
    }
  }

  #[napi]
  pub fn to_i64_bigint(value: &DynWinRTValue) -> napi::Result<BigInt> {
    match &value.0 {
      dynwinrt::WinRTValue::I64(value) => Ok(BigInt::from(*value)),
      _ => Err(napi::Error::from_reason("Value is not an i64")),
    }
  }

  #[napi]
  pub fn to_u64_bigint(value: &DynWinRTValue) -> napi::Result<BigInt> {
    match &value.0 {
      dynwinrt::WinRTValue::U64(value) => Ok(BigInt::from(*value)),
      _ => Err(napi::Error::from_reason("Value is not a u64")),
    }
  }

  #[napi]
  pub fn to_isize_bigint(value: &DynWinRTValue) -> napi::Result<BigInt> {
    #[cfg(target_pointer_width = "64")]
    let result = match &value.0 {
      dynwinrt::WinRTValue::I64(value) => Some(BigInt::from(*value)),
      _ => None,
    };
    #[cfg(target_pointer_width = "32")]
    let result = match &value.0 {
      dynwinrt::WinRTValue::I32(value) => Some(BigInt::from(i64::from(*value))),
      _ => None,
    };
    result.ok_or_else(|| napi::Error::from_reason("Value is not a pointer-sized signed integer"))
  }

  #[napi]
  pub fn to_usize_bigint(value: &DynWinRTValue) -> napi::Result<BigInt> {
    #[cfg(target_pointer_width = "64")]
    let result = match &value.0 {
      dynwinrt::WinRTValue::U64(value) => Some(BigInt::from(*value)),
      _ => None,
    };
    #[cfg(target_pointer_width = "32")]
    let result = match &value.0 {
      dynwinrt::WinRTValue::U32(value) => Some(BigInt::from(u64::from(*value))),
      _ => None,
    };
    result.ok_or_else(|| napi::Error::from_reason("Value is not a pointer-sized unsigned integer"))
  }

  #[napi]
  pub fn create_test_hwnd() -> napi::Result<BigInt> {
    self::create_test_hwnd()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::c_void;

  const TEST_POD_DESCRIPTOR: &str = r#"{"name":"Test.Pod","x86":{"size":8,"alignment":4,"fields":[{"name":"first","offset":0,"count":1,"type":{"kind":"u32"}},{"name":"second","offset":4,"count":2,"type":{"kind":"u16"}}]},"x64":{"size":8,"alignment":4,"fields":[{"name":"first","offset":0,"count":1,"type":{"kind":"u32"}},{"name":"second","offset":4,"count":2,"type":{"kind":"u16"}}]},"arm64":{"size":8,"alignment":4,"fields":[{"name":"first","offset":0,"count":1,"type":{"kind":"u32"}},{"name":"second","offset":4,"count":2,"type":{"kind":"u16"}}]}}"#;
  const TEST_INITIALIZED_POD_DESCRIPTOR: &str = r#"{"name":"Test.Initialized","initializers":[{"kind":"sizeOfLayout","field":"size"}],"x86":{"size":8,"alignment":4,"fields":[{"name":"size","offset":0,"count":1,"type":{"kind":"u32"}},{"name":"value","offset":4,"count":1,"type":{"kind":"u32"}}]},"x64":{"size":8,"alignment":4,"fields":[{"name":"size","offset":0,"count":1,"type":{"kind":"u32"}},{"name":"value","offset":4,"count":1,"type":{"kind":"u32"}}]},"arm64":{"size":8,"alignment":4,"fields":[{"name":"size","offset":0,"count":1,"type":{"kind":"u32"}},{"name":"value","offset":4,"count":1,"type":{"kind":"u32"}}]}}"#;
  const TEST_UNION_DESCRIPTOR: &str = r#"{"name":"Test.Union","x86":{"size":8,"alignment":8,"fields":[{"name":"integer","count":1,"type":{"kind":"u64"}},{"name":"pointer","count":1,"type":{"kind":"pointer"}}]},"x64":{"size":8,"alignment":8,"fields":[{"name":"integer","count":1,"type":{"kind":"u64"}},{"name":"pointer","count":1,"type":{"kind":"pointer"}}]},"arm64":{"size":8,"alignment":8,"fields":[{"name":"integer","count":1,"type":{"kind":"u64"}},{"name":"pointer","count":1,"type":{"kind":"pointer"}}]}}"#;

  #[repr(C)]
  struct FakeComObject {
    vtable: *const *mut c_void,
  }

  #[test]
  fn malloc_inspection_pointer_borrows_without_allocator_validation() {
    dynwinrt::com::initialize_apartment(dynwinrt::com::ApartmentType::MultiThreaded).unwrap();
    let allocator = unsafe { windows::Win32::System::Com::CoGetMalloc(1) }.unwrap();
    let pointer = unsafe { allocator.Alloc(16) };
    assert!(!pointer.is_null());
    let allocation = DynComAllocation::new(allocator, pointer);

    let borrowed = malloc_inspection_pointer(Some(&allocation)).unwrap();
    assert_eq!(
      as_pointer_bigint(&borrowed).unwrap().get_u64().1,
      pointer as usize as u64
    );
    assert!(!allocation.released());
  }

  #[test]
  fn bstr_values_are_not_apartment_bound() {
    let text = "embedded\0nul \u{1f642}";
    let value = AutomationValue::new(dynwinrt::com::Value::Bstr(dynwinrt::com::BstrValue::new(
      text,
    )));
    let result = std::thread::spawn(move || {
      let dynwinrt::com::Value::Bstr(value) = value.to_com_value().unwrap() else {
        panic!("expected BSTR")
      };
      value.as_deref().unwrap().to_string()
    })
    .join()
    .unwrap();
    assert_eq!(result, text);

    let apartment_bound = AutomationValue::new(dynwinrt::com::Value::Variant(
      dynwinrt::com::VariantValue::from_i32(1),
    ));
    assert!(
      std::thread::spawn(move || apartment_bound.to_com_value().is_err())
        .join()
        .unwrap()
    );
  }

  unsafe extern "system" fn fill_excep_info(
    _this: *mut c_void,
    info: *mut windows::Win32::System::Com::EXCEPINFO,
  ) -> windows::core::HRESULT {
    unsafe {
      (*info).wCode = 7;
      (*info).bstrSource = std::mem::ManuallyDrop::new(windows::core::BSTR::from("js source"));
      (*info).bstrDescription =
        std::mem::ManuallyDrop::new(windows::core::BSTR::from("js description"));
      (*info).bstrHelpFile = std::mem::ManuallyDrop::new(windows::core::BSTR::from("js help"));
      (*info).dwHelpContext = 19;
      (*info).scode = 0x80020009u32 as i32;
    }
    windows::core::HRESULT(0)
  }

  #[link(name = "oleaut32")]
  unsafe extern "system" {
    #[link_name = "SafeArrayCopy"]
    fn safe_array_copy_for_test(
      input: *mut windows::Win32::System::Com::SAFEARRAY,
      output: *mut *mut windows::Win32::System::Com::SAFEARRAY,
    ) -> windows::core::HRESULT;
  }

  unsafe extern "system" fn copy_safe_array_for_test(
    _this: *mut c_void,
    input: *mut windows::Win32::System::Com::SAFEARRAY,
    output: *mut *mut windows::Win32::System::Com::SAFEARRAY,
  ) -> windows::core::HRESULT {
    unsafe { safe_array_copy_for_test(input, output) }
  }

  #[test]
  fn automation_wrappers_are_tagged_typed_and_transfer_once() {
    let union =
      DynCom::create_native_union(TEST_UNION_DESCRIPTOR.into(), "integer".into(), None).unwrap();
    assert_eq!(union.active_field(), "integer");
    assert_eq!(union.bytes().as_ref(), &[0; 8]);
    assert!(
      DynCom::create_native_union(TEST_UNION_DESCRIPTOR.into(), "missing".into(), None).is_err()
    );

    let variant = DynComVariant::bstr("automation".into()).unwrap();
    assert_eq!(variant.kind().unwrap(), "bstr");
    assert_eq!(variant.to_string_value().unwrap(), "automation");
    assert!(variant.to_bool().is_err());
    assert!(DynComVariant::i8(128.0).is_err());
    assert!(DynComVariant::i8(4_294_967_297.0).is_err());
    assert!(DynComVariant::i32(4_294_967_296.0).is_err());
    assert!(DynComVariant::u32(-1.0).is_err());
    assert!(DynComVariant::bool(true).to_bool().unwrap());

    let array = DynComSafeArray::i32(
      vec![1.0, 2.0, 3.0, 4.0],
      Some(vec![
        DynComSafeArrayBound {
          lower_bound: -2.0,
          length: 2.0,
        },
        DynComSafeArrayBound {
          lower_bound: 5.0,
          length: 2.0,
        },
      ]),
    )
    .unwrap();
    assert_eq!(array.element_type().unwrap(), "i32");
    assert_eq!(array.to_numbers().unwrap(), [1.0, 2.0, 3.0, 4.0]);
    assert_eq!(array.bounds().unwrap()[0].lower_bound, -2.0);
    assert!(array.to_strings().is_err());
    assert!(DynComSafeArray::i32(vec![4_294_967_296.0], None).is_err());
    assert!(DynComSafeArray::i32(
      vec![],
      Some(vec![DynComSafeArrayBound {
        lower_bound: 4_294_967_296.0,
        length: 0.0,
      }]),
    )
    .is_err());

    let array_variant = DynComVariant::safe_array(&array).unwrap();
    assert_eq!(
      array_variant.to_safe_array().unwrap().to_numbers().unwrap(),
      [1.0, 2.0, 3.0, 4.0]
    );

    let prop = DynComPropVariant::string_vector(vec!["a".into(), "b".into()]).unwrap();
    assert_eq!(prop.kind().unwrap(), "vector");
    assert_eq!(prop.to_strings().unwrap(), ["a", "b"]);
    assert!(prop.to_numbers().is_err());
    assert!(DynComPropVariant::i8_vector(vec![4_294_967_297.0]).is_err());
    assert!(DynComPropVariant::string("bad\0value".into()).is_err());
    assert_eq!(
      DynComPropVariant::blob(Buffer::from(vec![1, 2, 3]))
        .unwrap()
        .to_blob()
        .unwrap()
        .as_ref(),
      &[1, 2, 3]
    );

    let mut stored = DynCom::variant(&variant).unwrap();
    let taken = DynCom::take_variant(&mut stored).unwrap();
    assert_eq!(taken.to_string_value().unwrap(), "automation");
    assert!(DynCom::take_variant(&mut stored).is_err());
  }

  #[test]
  fn typed_interface_safearray_outputs_accept_valid_generic_descriptors() {
    use windows::core::Interface as _;
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    let expected_iid = windows::core::GUID::from_u128(0x00000035_0000_0000_c000_000000000046);
    let interface = dynwinrt::com::register_interface(
      &TABLE,
      "Tests.ISafeArrayCopy",
      windows::core::GUID::from_u128(0x10000000_0000_0000_0000_000000000002),
      dynwinrt::com::InterfaceBase::IUnknown,
    )
    .add_method(
      "Copy",
      dynwinrt::com::MethodSignature::new(&TABLE)
        .add_in(dynwinrt::com::Type::typed_safe_array(
          dynwinrt::com::SafeArrayElementType::Unknown,
        ))
        .add_out(dynwinrt::com::Type::typed_interface_safe_array(
          expected_iid,
        )),
    );
    let method = interface.method(3).unwrap();
    let vtable = [
      std::ptr::null_mut(),
      std::ptr::null_mut(),
      std::ptr::null_mut(),
      copy_safe_array_for_test as *mut c_void,
    ];
    let mut object = FakeComObject {
      vtable: vtable.as_ptr(),
    };
    let factory = DynWinRTValue::activation_factory("Windows.Foundation.Uri".into()).unwrap();

    for descriptor_iid in [None, Some(windows::core::IUnknown::IID)] {
      let array = if let Some(iid) = descriptor_iid {
        DynComSafeArray::interface(
          &WinGUID(iid),
          vec![&factory],
          Some(vec![DynComSafeArrayBound {
            lower_bound: -2.0,
            length: 1.0,
          }]),
        )
        .unwrap()
      } else {
        DynComSafeArray::unknown(
          vec![&factory],
          Some(vec![DynComSafeArrayBound {
            lower_bound: -2.0,
            length: 1.0,
          }]),
        )
        .unwrap()
      };
      let mut output = unsafe {
        method.invoke_values_with_output_kinds(
          (&mut object as *mut FakeComObject).cast(),
          &[dynwinrt::com::Value::SafeArray(
            array.value().unwrap().clone(),
          )],
        )
      }
      .unwrap();
      let (value, kind) = output.pop().unwrap();
      let mut stored = DynWinRTValue::from_com_value(value, kind);
      let projected = DynCom::take_safe_array(&mut stored).unwrap();
      assert_eq!(projected.interface_iid().unwrap().unwrap().0, expected_iid);
      assert_eq!(projected.bounds().unwrap()[0].lower_bound, -2.0);
      assert_eq!(projected.to_interfaces().unwrap().len(), 1);
    }
  }

  #[test]
  fn dispatch_params_validate_ranges_clone_and_apartment_ownership() {
    let first = DynComVariant::i32(10.0).unwrap();
    let second = DynComVariant::bstr("second".into()).unwrap();
    let mut params = DynComDispatchParams::new(vec![&first, &second], Some(vec![42.0])).unwrap();
    assert_eq!(params.argument_count().unwrap(), 2);
    assert_eq!(params.named_dispids().unwrap(), [42]);

    let clone = params.clone_value().unwrap();
    params.release().unwrap();
    assert!(params.argument_count().is_err());
    assert_eq!(clone.argument_count().unwrap(), 2);

    assert!(DynComDispatchParams::new(vec![&first], Some(vec![1.0, 2.0])).is_err());
    assert!(DynComDispatchParams::new(vec![&first], Some(vec![1.5])).is_err());
    assert!(DynComDispatchParams::new(vec![&first], Some(vec![i32::MAX as f64 + 1.0])).is_err());

    let cross_thread = clone.clone_value().unwrap();
    let error = std::thread::spawn(move || cross_thread.argument_count().unwrap_err())
      .join()
      .unwrap();
    assert!(error.reason.contains("different thread"));
  }

  #[test]
  fn excep_info_output_transfers_to_the_js_wrapper_once() {
    let interface = dynwinrt::com::register_interface(
      &TABLE,
      "Tests.IExcepInfo",
      windows::core::GUID::from_u128(0x10000000_0000_0000_0000_000000000001),
      dynwinrt::com::InterfaceBase::IUnknown,
    )
    .add_method(
      "GetInfo",
      dynwinrt::com::MethodSignature::new(&TABLE).add_out(dynwinrt::com::Type::excep_info()),
    );
    let method = interface.method(3).unwrap();
    let vtable = [
      std::ptr::null_mut(),
      std::ptr::null_mut(),
      std::ptr::null_mut(),
      fill_excep_info as *mut c_void,
    ];
    let mut object = FakeComObject {
      vtable: vtable.as_ptr(),
    };
    let mut output = unsafe {
      method.invoke_values_with_output_kinds((&mut object as *mut FakeComObject).cast(), &[])
    }
    .unwrap();
    let (value, kind) = output.pop().unwrap();
    let mut stored = DynWinRTValue::from_com_value(value, kind);
    let mut info = DynCom::take_excep_info(&mut stored).unwrap();
    assert_eq!(info.code().unwrap(), 7);
    assert_eq!(info.source().unwrap().as_deref(), Some("js source"));
    assert_eq!(
      info.description().unwrap().as_deref(),
      Some("js description")
    );
    assert_eq!(info.help_file().unwrap().as_deref(), Some("js help"));
    assert_eq!(info.help_context().unwrap(), 19);
    assert_eq!(info.scode().unwrap(), 0x80020009u32 as i32);
    assert!(DynCom::take_excep_info(&mut stored).is_err());
    info.release().unwrap();
    assert!(info.description().is_err());
  }

  #[test]
  fn native_struct_helpers_validate_identity_size_and_zero_initialization() {
    let layout = native_struct_layout(TEST_POD_DESCRIPTOR).unwrap();
    assert_eq!(layout.name(), "Test.Pod");
    assert_eq!(layout.size(), 8);

    let zeroed = DynCom::create_native_struct(TEST_POD_DESCRIPTOR.into(), None).unwrap();
    assert_eq!(zeroed.bytes.as_slice(), &[0; 8]);
    let array =
      DynCom::create_native_struct_array(TEST_POD_DESCRIPTOR.into(), Buffer::from(vec![0; 16]))
        .unwrap();
    assert_eq!(array.length(), 16);
    assert!(DynCom::native_struct_buffer(TEST_POD_DESCRIPTOR.into(), &array).is_ok());
    assert!(DynCom::create_native_struct_array(
      TEST_POD_DESCRIPTOR.into(),
      Buffer::from(vec![0; 9]),
    )
    .is_err());
    let initialized = DynCom::create_native_struct(
      TEST_POD_DESCRIPTOR.into(),
      Some(Buffer::from(vec![1, 2, 3, 4, 5, 6, 7, 8])),
    )
    .unwrap();
    assert_eq!(initialized.bytes.as_slice(), &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert!(DynCom::create_native_struct(
      TEST_POD_DESCRIPTOR.into(),
      Some(Buffer::from(vec![0; 7]))
    )
    .is_err());

    let initialized_zeroed =
      DynCom::create_native_struct(TEST_INITIALIZED_POD_DESCRIPTOR.into(), None).unwrap();
    assert_eq!(
      initialized_zeroed.bytes.as_slice(),
      &[8, 0, 0, 0, 0, 0, 0, 0]
    );
    assert!(DynCom::create_native_struct(
      TEST_INITIALIZED_POD_DESCRIPTOR.into(),
      Some(Buffer::from(vec![0; 8]))
    )
    .is_err());
    assert!(DynCom::create_native_struct(
      TEST_INITIALIZED_POD_DESCRIPTOR.into(),
      Some(Buffer::from(vec![8, 0, 0, 0, 1, 0, 0, 0]))
    )
    .is_ok());

    let branded = DynComNativeStruct {
      descriptor: TEST_POD_DESCRIPTOR.into(),
      bytes: vec![1, 2, 3, 4, 5, 6, 7, 8],
    };
    let mut value = DynCom::native_struct(TEST_POD_DESCRIPTOR.into(), &branded).unwrap();
    assert!(!DynCom::is_null_native_struct_pointer(&value));
    assert!(DynCom::is_null_native_struct_pointer(
      &DynCom::null_native_struct_pointer()
    ));
    assert_eq!(
      DynCom::native_struct_bytes(TEST_POD_DESCRIPTOR.into(), &value)
        .unwrap()
        .bytes
        .as_slice(),
      &[1, 2, 3, 4, 5, 6, 7, 8]
    );
    value.release().unwrap();
    assert!(DynCom::native_struct_bytes(TEST_POD_DESCRIPTOR.into(), &value).is_err());
    let wrong_type = DynComNativeStruct {
      descriptor: TEST_POD_DESCRIPTOR.replace("Test.Pod", "Test.Other"),
      bytes: vec![0; 8],
    };
    assert!(DynCom::native_struct(TEST_POD_DESCRIPTOR.into(), &wrong_type).is_err());
    let wrong_size = DynComNativeStruct {
      descriptor: TEST_POD_DESCRIPTOR.into(),
      bytes: vec![0; 7],
    };
    assert!(DynCom::native_struct(TEST_POD_DESCRIPTOR.into(), &wrong_size).is_err());
  }

  #[test]
  fn projected_buffer_allocation_length_is_bounded() {
    assert_eq!(
      DynCom::buffer_allocation_length(BigInt::from(1024u64)).unwrap(),
      1024
    );
    assert!(DynCom::buffer_allocation_length(BigInt::from(i32::MAX as u64 + 1)).is_err());
  }

  #[test]
  fn string_pointer_buffers_require_matching_nul_terminators() {
    assert!(validate_wide_string_bytes(&[b'a', 0, 0, 0]).is_ok());
    assert!(validate_wide_string_bytes(&[b'a', 0]).is_err());
    assert!(validate_wide_string_bytes(&[b'a', 0, 0]).is_err());
    assert!(validate_ansi_string_bytes(b"text\0").is_ok());
    assert!(validate_ansi_string_bytes(b"text").is_err());
  }

  #[test]
  fn string_arrays_and_scalar_output_arrays_use_owned_runtime_storage() {
    let wide = DynCom::wide_string_array(vec!["First".into(), "Second".into()]).unwrap();
    assert_eq!(
      DynCom::buffer_count(&wide).unwrap().get_u64(),
      (false, 2, true)
    );
    assert!(DynCom::wide_string_array(vec!["embedded\0nul".into()]).is_err());
    assert!(DynCom::ansi_string_array(vec!["caf\u{e9}".into()]).is_err());

    let mut output = DynCom::caller_output_array(&DynCom::i32_type(), BigInt::from(2u64)).unwrap();
    assert_eq!(DynCom::take_i32_array(&mut output).unwrap(), [0, 0]);
    assert!(DynCom::take_i32_array(&mut output).is_err());
  }

  #[test]
  fn enumerator_array_helpers_preserve_typed_zeroed_storage() {
    let mut guids =
      DynCom::enumerator_output_array(&DynCom::guid_type(), BigInt::from(2u64)).unwrap();
    assert_eq!(
      DynCom::buffer_count(&guids).unwrap().get_u64(),
      (false, 2, true)
    );
    assert!(DynCom::take_guid_array(&mut guids)
      .unwrap()
      .iter()
      .all(|value| value.eq_ignore_ascii_case("00000000-0000-0000-0000-000000000000")));
    assert!(DynCom::take_guid_array(&mut guids).is_err());
    let variants =
      DynCom::enumerator_output_array(&DynCom::variant_type(), BigInt::from(1u64)).unwrap();
    assert_eq!(
      DynCom::buffer_count(&variants).unwrap().get_u64(),
      (false, 1, true)
    );
  }

  #[test]
  fn com_typed_buffer_widths_are_exact() {
    use napi::sys::TypedarrayType;

    for typ in [
      TypedarrayType::int8_array,
      TypedarrayType::uint8_array,
      TypedarrayType::uint8_clamped_array,
    ] {
      assert_eq!(typed_array_element_size(typ as i32).unwrap(), 1);
    }
    for typ in [TypedarrayType::int16_array, TypedarrayType::uint16_array] {
      assert_eq!(typed_array_element_size(typ as i32).unwrap(), 2);
    }
    for typ in [
      TypedarrayType::int32_array,
      TypedarrayType::uint32_array,
      TypedarrayType::float32_array,
    ] {
      assert_eq!(typed_array_element_size(typ as i32).unwrap(), 4);
    }
    for typ in [
      TypedarrayType::float64_array,
      TypedarrayType::bigint64_array,
      TypedarrayType::biguint64_array,
    ] {
      assert_eq!(typed_array_element_size(typ as i32).unwrap(), 8);
    }
    assert!(typed_array_element_size(i32::MAX).is_err());
  }

  #[test]
  fn takes_and_clears_cotaskmem_wide_string() {
    let text = "dynwinrt";
    let wide = text.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let bytes = wide.len() * std::mem::size_of::<u16>();
    let ptr = unsafe { windows::Win32::System::Com::CoTaskMemAlloc(bytes) };
    assert!(!ptr.is_null());
    unsafe {
      std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr.cast::<u16>(), wide.len());
    }
    let mut value = DynWinRTValue::from_com_result(
      dynwinrt::WinRTValue::RawPtr(ptr),
      dynwinrt::com::PointerOutputKind::CoTaskMem,
    );

    assert_eq!(take_co_task_mem_wide_string(&mut value).unwrap(), text);
    assert!(matches!(value.0, dynwinrt::WinRTValue::Null));
  }

  #[test]
  fn consuming_native_output_pointer_clears_source_value() {
    let ptr = 0x1234usize as *mut std::ffi::c_void;
    let mut value = DynWinRTValue::from_com_result(
      dynwinrt::WinRTValue::RawPtr(ptr),
      dynwinrt::com::PointerOutputKind::Com,
    );

    assert_eq!(
      take_native_output_pointer(&mut value, PointerProvenance::ComOutput, "test").unwrap(),
      ptr
    );
    assert!(matches!(value.0, dynwinrt::WinRTValue::Null));
    assert!(take_native_output_pointer(&mut value, PointerProvenance::ComOutput, "test").is_err());
  }

  #[test]
  fn borrowed_pointer_cannot_be_adopted() {
    let ptr = 0x1234usize as *mut std::ffi::c_void;
    let mut value = DynWinRTValue::with_borrowed_pointer(dynwinrt::WinRTValue::RawPtr(ptr));

    let error =
      take_native_output_pointer(&mut value, PointerProvenance::ComOutput, "COM interface")
        .unwrap_err();
    assert!(error.reason.contains("Borrowed"));
    assert!(matches!(value.0, dynwinrt::WinRTValue::RawPtr(raw) if raw == ptr));
  }

  #[test]
  fn unsafe_raw_com_ownership_is_explicit_and_preserves_the_caller_reference() {
    let _ = dynwinrt::com::initialize_apartment(dynwinrt::com::ApartmentType::MultiThreaded);
    let iid = WinGUID(windows::core::IUnknown::IID);
    let original = co_create_instance("00021401-0000-0000-c000-000000000046".into(), &iid)
      .expect("ShellLink activation");
    let object = original.0.as_object().unwrap();

    let borrowed = borrow_com_pointer_bits(object.as_raw(), &iid).unwrap();
    drop(borrowed);
    assert!(dynwinrt::WinRTValue::Object(object.clone())
      .cast(&iid.0)
      .is_ok());

    let transferred = std::mem::ManuallyDrop::new(object.clone());
    let adopted = adopt_owned_com_pointer_bits(transferred.as_raw(), &iid).unwrap();
    drop(adopted);
    assert!(dynwinrt::WinRTValue::Object(object.clone())
      .cast(&iid.0)
      .is_ok());

    let transferred = std::mem::ManuallyDrop::new(object.clone());
    let unsupported = WinGUID(windows::core::GUID::from_u128(
      0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff,
    ));
    assert!(adopt_owned_com_pointer_bits(transferred.as_raw(), &unsupported).is_err());
    assert!(dynwinrt::WinRTValue::Object(object.clone())
      .cast(&iid.0)
      .is_ok());
  }

  #[test]
  fn classic_com_objects_reject_cross_thread_use() {
    let _ = dynwinrt::com::initialize_apartment(dynwinrt::com::ApartmentType::MultiThreaded);
    let iid = WinGUID(windows::core::IUnknown::IID);
    let value = co_create_instance("00021401-0000-0000-c000-000000000046".into(), &iid)
      .expect("ShellLink activation");
    value
      .ensure_com_apartment()
      .expect("creating apartment must be accepted");

    let cross_thread_iid = iid;
    let (value, error, cast_error) = std::thread::spawn(move || {
      let error = value
        .ensure_com_apartment()
        .expect_err("cross-thread COM use must fail");
      let cast_error = match value.cast(&cross_thread_iid) {
        Ok(_) => panic!("QueryInterface must validate the source apartment first"),
        Err(error) => error,
      };
      (value, error, cast_error)
    })
    .join()
    .unwrap();

    assert!(error.reason.contains("different apartment thread"));
    assert!(cast_error.reason.contains("different apartment thread"));
    value
      .ensure_com_apartment()
      .expect("ownership returned to the creating apartment");
  }

  #[test]
  fn borrowed_handle_outputs_convert_null_and_pointer_bits_without_adoption() {
    let null = DynWinRTValue::from_com_result(
      dynwinrt::WinRTValue::RawPtr(std::ptr::null_mut()),
      dynwinrt::com::PointerOutputKind::None,
    );
    assert_eq!(
      as_pointer_bigint(&null).unwrap().get_u64(),
      (false, 0, true)
    );

    let ptr = 0x1234usize as *mut std::ffi::c_void;
    let mut value = DynWinRTValue::from_com_result(
      dynwinrt::WinRTValue::RawPtr(ptr),
      dynwinrt::com::PointerOutputKind::None,
    );
    assert_eq!(
      as_pointer_bigint(&value).unwrap().get_u64(),
      (false, 0x1234, true)
    );
    let error =
      take_native_output_pointer(&mut value, PointerProvenance::ComOutput, "COM interface")
        .unwrap_err();
    assert!(error.reason.contains("UnclassifiedOutput"));
    assert!(matches!(value.0, dynwinrt::WinRTValue::RawPtr(raw) if raw == ptr));
  }

  #[test]
  fn pointer_output_requires_the_exact_allocator_provenance() {
    let ptr = unsafe { windows::Win32::System::Com::CoTaskMemAlloc(1) };
    assert!(!ptr.is_null());
    let mut value = DynWinRTValue::from_com_result(
      dynwinrt::WinRTValue::RawPtr(ptr),
      dynwinrt::com::PointerOutputKind::CoTaskMem,
    );

    let error =
      take_native_output_pointer(&mut value, PointerProvenance::ComOutput, "COM interface")
        .unwrap_err();
    assert!(error.reason.contains("CoTaskMemOutput"));
    assert!(matches!(value.0, dynwinrt::WinRTValue::RawPtr(raw) if raw == ptr));
  }

  #[test]
  fn takes_and_frees_bstr() {
    let raw = windows::core::BSTR::from("dynwinrt").into_raw();
    let mut value = DynWinRTValue::from_com_result(
      dynwinrt::WinRTValue::RawPtr(raw as *mut std::ffi::c_void),
      dynwinrt::com::PointerOutputKind::Bstr,
    );

    assert_eq!(take_bstr(&mut value).unwrap(), "dynwinrt");
    assert!(matches!(value.0, dynwinrt::WinRTValue::Null));
  }

  #[test]
  fn managed_com_object_address_is_not_exported() {
    dynwinrt::com::initialize_apartment(dynwinrt::com::ApartmentType::MultiThreaded).unwrap();
    let iid = WinGUID(GUID::from_u128(0x000214f9_0000_0000_c000_000000000046));
    let value = co_create_instance("00021401-0000-0000-c000-000000000046".into(), &iid).unwrap();

    let error = as_pointer_bigint(&value).unwrap_err();
    assert!(error
      .reason
      .contains("Managed COM objects cannot be exported"));
  }

  #[test]
  fn try_cast_distinguishes_no_interface_from_other_errors() {
    dynwinrt::com::initialize_apartment(dynwinrt::com::ApartmentType::MultiThreaded).unwrap();
    let shell_link_iid = WinGUID(GUID::from_u128(0x000214f9_0000_0000_c000_000000000046));
    let value = co_create_instance(
      "00021401-0000-0000-c000-000000000046".into(),
      &shell_link_iid,
    )
    .unwrap();

    assert!(try_cast(&value, &shell_link_iid).unwrap().is_some());
    let unsupported = WinGUID(GUID::from_u128(0xaaaaaaaa_bbbb_cccc_dddd_eeeeeeeeeeee));
    assert!(try_cast(&value, &unsupported).unwrap().is_none());
  }

  #[test]
  fn pointer_sized_values_use_the_current_target_width() {
    let signed = DynCom::isize(BigInt::from(-1i64)).unwrap();
    let unsigned = DynCom::usize(BigInt::from(1u64)).unwrap();
    #[cfg(target_pointer_width = "64")]
    {
      assert!(matches!(signed.0, dynwinrt::WinRTValue::I64(-1)));
      assert!(matches!(unsigned.0, dynwinrt::WinRTValue::U64(1)));
    }
    #[cfg(target_pointer_width = "32")]
    {
      assert!(matches!(signed.0, dynwinrt::WinRTValue::I32(-1)));
      assert!(matches!(unsigned.0, dynwinrt::WinRTValue::U32(1)));
    }
  }

  #[test]
  fn iid_pointer_is_owner_backed_and_holds_the_guid() {
    // Regression (#4): iid_pointer must return an OWNER-BACKED value so the
    // boxed GUID is freed on drop/GC — not leak one Box<GUID> per distinct GUID
    // into a process-lifetime static cache. The pre-fix version returned an
    // unowned RawPtr (`.1 == None`) into a static cache (stable address per
    // GUID), so both assertions below fail against it.
    let guid = GUID::from_u128(0xa5caee9b_8708_49d1_8d36_67d25a8da00c);

    let value = iid_pointer(&WinGUID(guid));
    assert!(
      value.1.is_some(),
      "iid_pointer must be owner-backed (NativePointerOwner::Guid) so it frees on drop"
    );
    match value.0 {
      dynwinrt::WinRTValue::RawPtr(ptr) => {
        assert!(!ptr.is_null());
        let read = unsafe { *(ptr as *const GUID) };
        assert_eq!(
          read, guid,
          "REFIID pointer must hold the correct GUID bytes"
        );
      }
      _ => panic!("iid_pointer must return a RawPtr"),
    }

    // Two concurrently-live calls for the SAME GUID must allocate distinct
    // boxes (distinct addresses) — proving there is no shared static cache.
    let a = iid_pointer(&WinGUID(guid));
    let b = iid_pointer(&WinGUID(guid));
    let pa = match a.0 {
      dynwinrt::WinRTValue::RawPtr(p) => p as usize,
      _ => 0,
    };
    let pb = match b.0 {
      dynwinrt::WinRTValue::RawPtr(p) => p as usize,
      _ => 0,
    };
    assert_ne!(
      pa, pb,
      "each iid_pointer call must own its own boxed GUID, not share a static one"
    );
  }
}
