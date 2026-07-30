// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use napi::JsValue;
use napi::bindgen_prelude::{BigInt, FromNapiValue, ToNapiValue, Unknown};
use napi_derive::napi;
use windows::core::{GUID, Interface as _};

use super::{DynWinRTValue, TABLE, WinGUID};

#[allow(dead_code)]
pub(super) enum NativePointerOwner {
  Uint8Array {
    value: std::sync::Mutex<napi::bindgen_prelude::Uint8Array>,
    env: napi::sys::napi_env,
    pointer: usize,
    length: usize,
  },
  CoTaskMem(*mut std::ffi::c_void),
  Guid(*mut GUID),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PointerProvenance {
  None,
  Borrowed,
  NativeOutput,
}

impl NativePointerOwner {
  fn validate(&self) -> napi::Result<()> {
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
      _ => {}
    }
  }
}

fn co_create_instance(clsid: String, iid: &WinGUID) -> napi::Result<DynWinRTValue> {
  let parsed = windows::core::GUID::try_from(clsid.as_str())
    .map_err(|_| napi::Error::from_reason(format!("Invalid CLSID: '{clsid}'")))?;
  dynwinrt::com::co_create_instance(parsed, iid.0)
    .map(DynWinRTValue::new)
    .map_err(|error| napi::Error::from_reason(error.message()))
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

pub(super) fn pointer(value: Unknown) -> napi::Result<DynWinRTValue> {
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
  let ptr = take_native_output_pointer(value, "COM interface")?;
  let adopted = unsafe { dynwinrt::com::adopt_com_pointer(ptr) };
  match iid {
    Some(iid) => adopted
      .cast(&iid.0)
      .map(DynWinRTValue::new)
      .map_err(|error| napi::Error::from_reason(error.message())),
    None => Ok(DynWinRTValue::new(adopted)),
  }
}

fn adopt_co_task_mem_pointer(value: &mut DynWinRTValue) -> napi::Result<DynWinRTValue> {
  let ptr = take_native_output_pointer(value, "CoTaskMem allocation")?;
  if ptr.is_null() {
    return Ok(DynWinRTValue::new(dynwinrt::WinRTValue::Null));
  }
  Ok(DynWinRTValue::with_pointer_owner(
    dynwinrt::WinRTValue::RawPtr(ptr),
    NativePointerOwner::CoTaskMem(ptr),
  ))
}

pub(super) fn as_pointer_bigint(value: &DynWinRTValue) -> napi::Result<BigInt> {
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
  let ptr = take_native_output_pointer(value, "wide-string")?;
  if ptr.is_null() {
    return Ok(String::new());
  }
  let result = unsafe { windows::core::PCWSTR(ptr.cast()).to_string() }
    .map_err(|error| napi::Error::from_reason(error.to_string()));
  unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(ptr)) };
  result
}

fn take_co_task_mem_ansi_string(value: &mut DynWinRTValue) -> napi::Result<String> {
  let ptr = take_native_output_pointer(value, "ANSI-string")?;
  if ptr.is_null() {
    return Ok(String::new());
  }
  let result = unsafe { windows::core::PCSTR(ptr.cast()).to_string() }
    .map_err(|error| napi::Error::from_reason(error.to_string()));
  unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(ptr)) };
  result
}

fn take_bstr(value: &mut DynWinRTValue) -> napi::Result<String> {
  let ptr = take_native_output_pointer(value, "BSTR")?;
  if ptr.is_null() {
    return Ok(String::new());
  }
  let value = unsafe { windows::core::BSTR::from_raw(ptr.cast()) };
  String::try_from(&value).map_err(|error| napi::Error::from_reason(error.to_string()))
}

pub(super) fn validate_pointer_owner(value: &DynWinRTValue) -> napi::Result<()> {
  if let Some(owner) = &value.1 {
    owner.validate()?;
  }
  Ok(())
}

fn take_native_output_pointer(
  value: &mut DynWinRTValue,
  description: &str,
) -> napi::Result<*mut std::ffi::c_void> {
  if value.1.is_some() {
    return Err(napi::Error::from_reason(format!(
      "Cannot consume an owner-backed {description} pointer"
    )));
  }
  if value.2 != PointerProvenance::NativeOutput {
    return Err(napi::Error::from_reason(format!(
      "Cannot adopt a borrowed {description} pointer; only owned native outputs may be consumed"
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
  pub fn add_out(&self, typ: &DynComType) -> Self {
    Self(self.0.clone().add_out(typ.0.clone()))
  }

  #[napi]
  pub fn add_in_out(&self, typ: &DynComType) -> Self {
    Self(self.0.clone().add_in_out(typ.0.clone()))
  }

  #[napi]
  pub fn add_out_fill(&self, typ: &DynComType) -> Self {
    Self(self.0.clone().add_out_fill(typ.0.clone()))
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
impl DynComMethodHandle {
  #[napi]
  pub fn get_string(&self, obj: &DynWinRTValue) -> napi::Result<String> {
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("getString() requires a COM object"))?
      .as_raw();
    self
      .0
      .call_getter_hstring(raw)
      .map(|value| value.to_string())
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  #[napi]
  pub fn invoke(
    &self,
    obj: &DynWinRTValue,
    args: Vec<&DynWinRTValue>,
  ) -> napi::Result<DynWinRTValue> {
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("invoke() requires a COM object"))?
      .as_raw();
    for arg in &args {
      validate_pointer_owner(arg)?;
    }
    let args = args.iter().map(|arg| arg.0.clone()).collect::<Vec<_>>();
    let results = self
      .0
      .invoke(raw, &args)
      .map_err(|error| napi::Error::from_reason(error.message()))?;
    Ok(DynWinRTValue::from_com_result(
      results
        .into_iter()
        .next()
        .unwrap_or(dynwinrt::WinRTValue::I32(0)),
    ))
  }

  #[napi]
  pub fn invoke_all(
    &self,
    obj: &DynWinRTValue,
    args: Vec<&DynWinRTValue>,
  ) -> napi::Result<Vec<DynWinRTValue>> {
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("invokeAll() requires a COM object"))?
      .as_raw();
    for arg in &args {
      validate_pointer_owner(arg)?;
    }
    let args = args.iter().map(|arg| arg.0.clone()).collect::<Vec<_>>();
    self
      .0
      .invoke(raw, &args)
      .map(|results| {
        results
          .into_iter()
          .map(DynWinRTValue::from_com_result)
          .collect()
      })
      .map_err(|error| napi::Error::from_reason(error.message()))
  }
}

#[napi]
pub struct DynCom;

#[napi]
impl DynCom {
  #[napi]
  pub fn initialize(apartment_type: Option<i32>) -> napi::Result<()> {
    let apartment_type = match apartment_type.unwrap_or(1) {
      0 => dynwinrt::com::ApartmentType::SingleThreaded,
      _ => dynwinrt::com::ApartmentType::MultiThreaded,
    };
    dynwinrt::com::initialize_apartment(apartment_type)
      .map_err(|error| napi::Error::from_reason(error.message()))
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
  pub fn pointer(
    #[napi(ts_arg_type = "bigint | number | Buffer | Uint8Array | null | undefined")]
    value: Unknown,
  ) -> napi::Result<DynWinRTValue> {
    self::pointer(value)
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
    let mut value = DynWinRTValue::from_com_result(dynwinrt::WinRTValue::RawPtr(ptr));

    assert_eq!(take_co_task_mem_wide_string(&mut value).unwrap(), text);
    assert!(matches!(value.0, dynwinrt::WinRTValue::Null));
  }

  #[test]
  fn consuming_native_output_pointer_clears_source_value() {
    let ptr = 0x1234usize as *mut std::ffi::c_void;
    let mut value = DynWinRTValue::from_com_result(dynwinrt::WinRTValue::RawPtr(ptr));

    assert_eq!(take_native_output_pointer(&mut value, "test").unwrap(), ptr);
    assert!(matches!(value.0, dynwinrt::WinRTValue::Null));
    assert!(take_native_output_pointer(&mut value, "test").is_err());
  }

  #[test]
  fn borrowed_pointer_cannot_be_adopted() {
    let ptr = 0x1234usize as *mut std::ffi::c_void;
    let mut value = DynWinRTValue::with_borrowed_pointer(dynwinrt::WinRTValue::RawPtr(ptr));

    let error = take_native_output_pointer(&mut value, "COM interface").unwrap_err();
    assert!(
      error
        .reason
        .contains("Cannot adopt a borrowed COM interface")
    );
    assert!(matches!(value.0, dynwinrt::WinRTValue::RawPtr(raw) if raw == ptr));
  }

  #[test]
  fn takes_and_frees_bstr() {
    let raw = windows::core::BSTR::from("dynwinrt").into_raw();
    let mut value =
      DynWinRTValue::from_com_result(dynwinrt::WinRTValue::RawPtr(raw as *mut std::ffi::c_void));

    assert_eq!(take_bstr(&mut value).unwrap(), "dynwinrt");
    assert!(matches!(value.0, dynwinrt::WinRTValue::Null));
  }

  #[test]
  fn managed_com_object_address_is_not_exported() {
    dynwinrt::com::initialize_apartment(dynwinrt::com::ApartmentType::MultiThreaded).unwrap();
    let iid = WinGUID(GUID::from_u128(0x000214f9_0000_0000_c000_000000000046));
    let value = co_create_instance("00021401-0000-0000-c000-000000000046".into(), &iid).unwrap();

    let error = as_pointer_bigint(&value).unwrap_err();
    assert!(
      error
        .reason
        .contains("Managed COM objects cannot be exported")
    );
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
