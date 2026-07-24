// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use napi::bindgen_prelude::{BigInt, FromNapiValue, Unknown};
use napi::JsValue;
use napi_derive::napi;
use windows::core::{GUID, IUnknown, Interface as _};

use super::{DynWinRTValue, WinGUID, TABLE};

#[allow(dead_code)]
pub(super) enum NativePointerOwner {
  Buffer(napi::bindgen_prelude::Buffer),
  Uint8Array(napi::bindgen_prelude::Uint8Array),
  ComObject(IUnknown),
  CoTaskMem(*mut std::ffi::c_void),
  Guid(*mut GUID),
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
    return Ok(DynWinRTValue::new(dynwinrt::WinRTValue::RawPtr(
      std::ptr::null_mut(),
    )));
  }
  if value_type == sys::ValueType::napi_bigint {
    let bigint = unsafe { BigInt::from_napi_value(env, raw) }?;
    let (negative, bits, lossless) = bigint.get_u64();
    if negative || !lossless || bits as usize as u64 != bits {
      return Err(napi::Error::from_reason(
        "pointer(): bigint must fit in an unsigned pointer",
      ));
    }
    return Ok(DynWinRTValue::new(dynwinrt::WinRTValue::RawPtr(
      bits as usize as *mut std::ffi::c_void,
    )));
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
    return Ok(DynWinRTValue::new(dynwinrt::WinRTValue::RawPtr(
      number as usize as *mut std::ffi::c_void,
    )));
  }
  if let Ok(buffer) = unsafe { napi::bindgen_prelude::Buffer::from_napi_value(env, raw) } {
    let ptr = buffer.as_ref().as_ptr() as *mut std::ffi::c_void;
    return Ok(DynWinRTValue::with_pointer_owner(
      dynwinrt::WinRTValue::RawPtr(ptr),
      NativePointerOwner::Buffer(buffer),
    ));
  }
  if let Ok(array) = unsafe { napi::bindgen_prelude::Uint8Array::from_napi_value(env, raw) } {
    let ptr = array.as_ref().as_ptr() as *mut std::ffi::c_void;
    return Ok(DynWinRTValue::with_pointer_owner(
      dynwinrt::WinRTValue::RawPtr(ptr),
      NativePointerOwner::Uint8Array(array),
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

fn adopt_com_pointer(
  value: &mut DynWinRTValue,
  iid: Option<&WinGUID>,
) -> napi::Result<DynWinRTValue> {
  let ptr = take_raw_pointer(value, "COM interface")?;
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
  let ptr = take_raw_pointer(value, "CoTaskMem allocation")?;
  if ptr.is_null() {
    return Ok(DynWinRTValue::new(dynwinrt::WinRTValue::Null));
  }
  Ok(DynWinRTValue::with_pointer_owner(
    dynwinrt::WinRTValue::RawPtr(ptr),
    NativePointerOwner::CoTaskMem(ptr),
  ))
}

fn as_pointer_bigint(value: &DynWinRTValue) -> napi::Result<BigInt> {
  let bits = match &value.0 {
    dynwinrt::WinRTValue::Object(object) => object.as_raw() as usize,
    dynwinrt::WinRTValue::RawPtr(ptr) => *ptr as usize,
    dynwinrt::WinRTValue::Null => 0,
    _ => {
      return Err(napi::Error::from_reason(
        "Value is not a pointer or COM object",
      ))
    }
  };
  Ok(BigInt::from(bits as u64))
}

fn take_co_task_mem_wide_string(value: &mut DynWinRTValue) -> napi::Result<String> {
  let ptr = take_raw_pointer(value, "wide-string")?;
  if ptr.is_null() {
    return Ok(String::new());
  }
  let result = unsafe { windows::core::PCWSTR(ptr.cast()).to_string() }
    .map_err(|error| napi::Error::from_reason(error.to_string()));
  unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(ptr)) };
  result
}

fn take_co_task_mem_ansi_string(value: &mut DynWinRTValue) -> napi::Result<String> {
  let ptr = take_raw_pointer(value, "ANSI-string")?;
  if ptr.is_null() {
    return Ok(String::new());
  }
  let result = unsafe { windows::core::PCSTR(ptr.cast()).to_string() }
    .map_err(|error| napi::Error::from_reason(error.to_string()));
  unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(ptr)) };
  result
}

fn take_raw_pointer(
  value: &mut DynWinRTValue,
  description: &str,
) -> napi::Result<*mut std::ffi::c_void> {
  if value.1.is_some() {
    return Err(napi::Error::from_reason(format!(
      "Cannot consume an owner-backed {description} pointer"
    )));
  }
  match std::mem::replace(&mut value.0, dynwinrt::WinRTValue::Null) {
    dynwinrt::WinRTValue::RawPtr(ptr) => Ok(ptr),
    dynwinrt::WinRTValue::Null => Ok(std::ptr::null_mut()),
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
pub struct DynComMethodHandle(dynwinrt::MethodHandle);

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
    let args = args.iter().map(|arg| arg.0.clone()).collect::<Vec<_>>();
    let results = self
      .0
      .invoke(raw, &args)
      .map_err(|error| napi::Error::from_reason(error.message()))?;
    Ok(DynWinRTValue::new(
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
    let args = args.iter().map(|arg| arg.0.clone()).collect::<Vec<_>>();
    self
      .0
      .invoke(raw, &args)
      .map(|results| results.into_iter().map(DynWinRTValue::new).collect())
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
    #[napi(
      ts_arg_type = "bigint | number | Buffer | Uint8Array | DynWinRtValue | null | undefined"
    )]
    value: Unknown,
  ) -> napi::Result<DynWinRTValue> {
    self::pointer(value)
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
    let mut value = DynWinRTValue::new(dynwinrt::WinRTValue::RawPtr(ptr));

    assert_eq!(take_co_task_mem_wide_string(&mut value).unwrap(), text);
    assert!(matches!(value.0, dynwinrt::WinRTValue::Null));
  }

  #[test]
  fn consuming_raw_pointer_clears_source_value() {
    let ptr = 0x1234usize as *mut std::ffi::c_void;
    let mut value = DynWinRTValue::new(dynwinrt::WinRTValue::RawPtr(ptr));

    assert_eq!(take_raw_pointer(&mut value, "test").unwrap(), ptr);
    assert!(matches!(value.0, dynwinrt::WinRTValue::Null));
    assert!(take_raw_pointer(&mut value, "test").unwrap().is_null());
  }
}
