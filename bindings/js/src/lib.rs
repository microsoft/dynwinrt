// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#![deny(clippy::all)]
#![allow(clippy::missing_safety_doc)]

use std::{
  cell::Cell,
  mem,
  sync::{Arc, Mutex, OnceLock},
};

use dynwinrt;
use napi::bindgen_prelude::{BigInt, Either, FromNapiValue, PromiseRaw, ToNapiValue, Unknown};
use napi::Env;
use napi_derive::napi;
use windows::core::{IUnknown, Interface, HSTRING};

mod com;
pub use com::{
  initialize_com, DynCom, DynComDispatchInvokeResult, DynComDispatchParams, DynComExcepInfo,
  DynComInterface, DynComMethodHandle, DynComMethodSig, DynComNativeStruct,
  DynComNativeStructArray, DynComNativeUnion, DynComPropVariant, DynComSafeArray,
  DynComSafeArrayBound, DynComType, DynComUnsafe, DynComUnsafeInterface, DynComVariant,
};
mod async_promise;
mod managed_tsfn;
mod scheduled_start;
#[cfg(feature = "test-hooks")]
mod tsfn_test_hooks;

/// Shared MetadataTable — created once, used everywhere.
static TABLE: std::sync::LazyLock<Arc<dynwinrt::MetadataTable>> =
  std::sync::LazyLock::new(|| dynwinrt::MetadataTable::new());

fn ensure_progress_type_supported(progress_type: &dynwinrt::TypeHandle) -> napi::Result<()> {
  match progress_type.kind() {
    dynwinrt::TypeKind::Guid
    | dynwinrt::TypeKind::ArrayOfIUnknown
    | dynwinrt::TypeKind::Generic { .. }
    | dynwinrt::TypeKind::OutValue(_)
    | dynwinrt::TypeKind::Array(_) => Err(napi::Error::from_reason(format!(
      "onProgress: progress callbacks do not support {:?} values",
      progress_type.kind()
    ))),
    dynwinrt::TypeKind::Bool
    | dynwinrt::TypeKind::I8
    | dynwinrt::TypeKind::U8
    | dynwinrt::TypeKind::I16
    | dynwinrt::TypeKind::U16
    | dynwinrt::TypeKind::Char16
    | dynwinrt::TypeKind::I32
    | dynwinrt::TypeKind::U32
    | dynwinrt::TypeKind::I64
    | dynwinrt::TypeKind::U64
    | dynwinrt::TypeKind::F32
    | dynwinrt::TypeKind::F64
    | dynwinrt::TypeKind::HString
    | dynwinrt::TypeKind::Object
    | dynwinrt::TypeKind::HResult
    | dynwinrt::TypeKind::Interface(_)
    | dynwinrt::TypeKind::Delegate(_)
    | dynwinrt::TypeKind::IAsyncAction
    | dynwinrt::TypeKind::IAsyncActionWithProgress(_)
    | dynwinrt::TypeKind::IAsyncOperation(_)
    | dynwinrt::TypeKind::IAsyncOperationWithProgress(_)
    | dynwinrt::TypeKind::RuntimeClass(_)
    | dynwinrt::TypeKind::Parameterized(_)
    | dynwinrt::TypeKind::Enum(_)
    | dynwinrt::TypeKind::Struct(_) => Ok(()),
  }
}

// ======================================================================
// Runtime initialization
// ======================================================================

struct InitializedWinAppSdk {
  major: u32,
  minor: u32,
  context: dynwinrt::WinAppSdkContext,
}

static WINAPP_SDK: OnceLock<InitializedWinAppSdk> = OnceLock::new();

thread_local! {
  static WINUI_DISPATCHER_LOOP_ACTIVE: Cell<bool> = const { Cell::new(false) };
  static WINUI_DISPATCHER_LOOP_ENTERED: Cell<bool> = const { Cell::new(false) };
}

fn winui_dispatcher_loop_active() -> bool {
  WINUI_DISPATCHER_LOOP_ACTIVE.with(Cell::get)
}

fn winui_dispatcher_loop_exited() -> bool {
  WINUI_DISPATCHER_LOOP_ENTERED.with(Cell::get) && !winui_dispatcher_loop_active()
}

fn js_u64(value: Either<BigInt, f64>, context: &str) -> napi::Result<u64> {
  match value {
    Either::A(value) => {
      let (negative, value, lossless) = value.get_u64();
      if negative || !lossless {
        return Err(napi::Error::from_reason(format!(
          "{context}: bigint value must fit in an unsigned 64-bit integer",
        )));
      }
      Ok(value)
    }
    Either::B(value) => {
      if !value.is_finite()
        || value.fract() != 0.0
        || !(0.0..=9_007_199_254_740_991.0).contains(&value)
      {
        return Err(napi::Error::from_reason(format!(
          "{context}: number value must be a non-negative safe integer; use bigint for larger values",
        )));
      }
      Ok(value as u64)
    }
  }
}

fn js_i64(value: Either<BigInt, f64>, context: &str) -> napi::Result<i64> {
  match value {
    Either::A(value) => {
      let (value, lossless) = value.get_i64();
      if !lossless {
        return Err(napi::Error::from_reason(format!(
          "{context}: bigint value must fit in a signed 64-bit integer",
        )));
      }
      Ok(value)
    }
    Either::B(value) => {
      if !value.is_finite() || value.fract() != 0.0 || value.abs() > 9_007_199_254_740_991.0 {
        return Err(napi::Error::from_reason(format!(
          "{context}: number value must be a safe integer; use bigint for larger values",
        )));
      }
      Ok(value as i64)
    }
  }
}

fn js_i32(value: f64, context: &str) -> napi::Result<i32> {
  if !value.is_finite()
    || value.fract() != 0.0
    || value < f64::from(i32::MIN)
    || value > f64::from(i32::MAX)
  {
    return Err(napi::Error::from_reason(format!(
      "{context}: value must be an integer in the i32 range",
    )));
  }
  Ok(value as i32)
}

fn js_u32(value: f64, context: &str) -> napi::Result<u32> {
  if !value.is_finite() || value.fract() != 0.0 || !(0.0..=f64::from(u32::MAX)).contains(&value) {
    return Err(napi::Error::from_reason(format!(
      "{context}: value must be an integer in the u32 range",
    )));
  }
  Ok(value as u32)
}

fn js_safe_i64(value: i64, context: &str) -> napi::Result<i64> {
  const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
  if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) {
    return Err(napi::Error::from_reason(format!(
      "{context}: value is outside the JavaScript safe-integer range; use the bigint conversion instead",
    )));
  }

  fn property_value_guid(value: windows::core::GUID) -> String {
    format!("{value:?}")
  }

  fn to_unknown<'env, T: ToNapiValue>(env: Env, value: T) -> napi::Result<Unknown<'env>> {
    let value = unsafe { T::to_napi_value(env.raw(), value) }?;
    unsafe { Unknown::from_napi_value(env.raw(), value) }
  }

  fn utf16_unknown<'env>(env: Env, value: &[u16]) -> napi::Result<Unknown<'env>> {
    let length = isize::try_from(value.len())
      .map_err(|_| napi::Error::from_reason("WinRT Char16 string is too large for JavaScript"))?;
    let mut result = std::ptr::null_mut();
    napi::check_status!(
      unsafe {
        napi::sys::napi_create_string_utf16(env.raw(), value.as_ptr(), length, &mut result)
      },
      "Failed to create JavaScript UTF-16 string"
    )?;
    unsafe { Unknown::from_napi_value(env.raw(), result) }
  }

  fn utf16_array_unknown<'env>(env: Env, values: Vec<u16>) -> napi::Result<Unknown<'env>> {
    let mut result = std::ptr::null_mut();
    napi::check_status!(
      unsafe { napi::sys::napi_create_array_with_length(env.raw(), values.len(), &mut result) },
      "Failed to create JavaScript Char16 array"
    )?;
    for (index, value) in values.into_iter().enumerate() {
      let value = utf16_unknown(env, &[value])?;
      napi::check_status!(
        unsafe {
          napi::sys::napi_set_element(env.raw(), result, index as u32, napi::JsValue::raw(&value))
        },
        "Failed to populate JavaScript Char16 array"
      )?;
    }
    unsafe { Unknown::from_napi_value(env.raw(), result) }
  }

  fn null_unknown<'env>(env: Env) -> napi::Result<Unknown<'env>> {
    let mut value = std::ptr::null_mut();
    napi::check_status!(
      unsafe { napi::sys::napi_get_null(env.raw(), &mut value) },
      "Failed to create JavaScript null"
    )?;
    unsafe { Unknown::from_napi_value(env.raw(), value) }
  }

  fn property_value_to_javascript<'env>(
    env: Env,
    value: dynwinrt::PropertyValueData,
  ) -> napi::Result<Unknown<'env>> {
    use dynwinrt::PropertyValueData;

    match value {
      PropertyValueData::UInt8(value) => to_unknown(env, u32::from(value)),
      PropertyValueData::Int16(value) => to_unknown(env, i32::from(value)),
      PropertyValueData::UInt16(value) => to_unknown(env, u32::from(value)),
      PropertyValueData::Int32(value) => to_unknown(env, value),
      PropertyValueData::UInt32(value) => to_unknown(env, value),
      PropertyValueData::Int64(value) => to_unknown(env, BigInt::from(value)),
      PropertyValueData::UInt64(value) => to_unknown(env, BigInt::from(value)),
      PropertyValueData::Single(value) => to_unknown(env, value),
      PropertyValueData::Double(value) => to_unknown(env, value),
      PropertyValueData::Char16(value) => utf16_unknown(env, &[value]),
      PropertyValueData::Boolean(value) => to_unknown(env, value),
      PropertyValueData::String(value) => to_unknown(env, value),
      PropertyValueData::Guid(value) => to_unknown(env, property_value_guid(value)),
      PropertyValueData::UInt8Array(value) => {
        to_unknown(env, napi::bindgen_prelude::Buffer::from(value))
      }
      PropertyValueData::Int16Array(value) => {
        to_unknown(env, value.into_iter().map(i32::from).collect::<Vec<_>>())
      }
      PropertyValueData::UInt16Array(value) => {
        to_unknown(env, value.into_iter().map(u32::from).collect::<Vec<_>>())
      }
      PropertyValueData::Int32Array(value) => to_unknown(env, value),
      PropertyValueData::UInt32Array(value) => to_unknown(env, value),
      PropertyValueData::Int64Array(value) => {
        to_unknown(env, value.into_iter().map(BigInt::from).collect::<Vec<_>>())
      }
      PropertyValueData::UInt64Array(value) => {
        to_unknown(env, value.into_iter().map(BigInt::from).collect::<Vec<_>>())
      }
      PropertyValueData::SingleArray(value) => to_unknown(env, value),
      PropertyValueData::DoubleArray(value) => to_unknown(env, value),
      PropertyValueData::Char16Array(value) => utf16_array_unknown(env, value),
      PropertyValueData::BooleanArray(value) => to_unknown(env, value),
      PropertyValueData::StringArray(value) => to_unknown(env, value),
      PropertyValueData::GuidArray(value) => to_unknown(
        env,
        value
          .into_iter()
          .map(property_value_guid)
          .collect::<Vec<_>>(),
      ),
    }
  }

  /// Explicitly unbox a supported WinRT `IPropertyValue`.
  ///
  /// JavaScript `null` and WinRT null are returned as `null`. A non-`IPropertyValue`
  /// `DynWinRtValue` is returned unchanged, preserving JavaScript and COM identity.
  #[napi(
    ts_args_type = "value: DynWinRtValue | null",
    ts_return_type = "boolean | number | bigint | string | Uint8Array | number[] | bigint[] | boolean[] | string[] | DynWinRtValue | null"
  )]
  pub fn unbox_object<'env>(env: Env, value: Unknown<'env>) -> napi::Result<Unknown<'env>> {
    if value.get_type()? == napi::ValueType::Null {
      return Ok(value);
    }

    let raw = unsafe {
      <&DynWinRTValue as FromNapiValue>::from_napi_value(env.raw(), napi::JsValue::raw(&value))
    }?;
    match dynwinrt::unbox_property_value(&raw.0)
      .map_err(|error| napi::Error::from_reason(error.message()))?
    {
      dynwinrt::PropertyValueUnboxResult::Null => null_unknown(env),
      dynwinrt::PropertyValueUnboxResult::NotPropertyValue => Ok(value),
      dynwinrt::PropertyValueUnboxResult::Value(value) => property_value_to_javascript(env, value),
    }
  }
  Ok(value)
}

fn collect_typed_array<T, U>(
  array: &dynwinrt::ArrayData,
  method: &str,
  expected: impl Fn(dynwinrt::TypeKind) -> bool,
  from_raw: impl Fn(T) -> U,
  from_value: impl Fn(dynwinrt::WinRTValue) -> Option<U>,
) -> napi::Result<Vec<U>>
where
  T: Copy,
{
  let actual = array.element_type.kind();
  if !expected(actual) {
    return Err(napi::Error::from_reason(format!(
      "{method} cannot read an array with element type {actual:?}",
    )));
  }

  if let Some(values) = unsafe { array.try_as_typed_slice::<T>() } {
    return Ok(values.iter().copied().map(from_raw).collect());
  }

  (0..array.len())
    .map(|index| {
      let value = array.get(index);
      let actual = value.get_type_kind();
      from_value(value).ok_or_else(|| {
        napi::Error::from_reason(format!(
          "{method} found incompatible stored value {actual:?} at index {index}",
        ))
      })
    })
    .collect()
}

/// Add Windows App SDK to the process package graph without changing the calling thread's apartment.
#[napi]
pub fn init_winappsdk(major: u32, minor: u32) -> napi::Result<()> {
  if let Some(initialized) = WINAPP_SDK.get() {
    return ensure_winappsdk_version(initialized, major, minor);
  }

  let context = dynwinrt::initialize_winappsdk(major, minor)
    .map_err(|e| napi::Error::from_reason(e.message()))?;
  let initialized = InitializedWinAppSdk {
    major,
    minor,
    context,
  };

  match WINAPP_SDK.set(initialized) {
    Ok(()) => Ok(()),
    Err(_) => ensure_winappsdk_version(WINAPP_SDK.get().unwrap(), major, minor),
  }
}

fn ensure_winappsdk_version(
  initialized: &InitializedWinAppSdk,
  major: u32,
  minor: u32,
) -> napi::Result<()> {
  if initialized.major == major && initialized.minor == minor {
    Ok(())
  } else {
    Err(napi::Error::from_reason(format!(
      "Windows App SDK is already initialized for {}.{}; cannot reinitialize for {major}.{minor}",
      initialized.major, initialized.minor
    )))
  }
}

/// Return the framework resources.pri path selected by initWinappsdk.
#[napi]
pub fn get_winappsdk_resource_pri_path() -> napi::Result<String> {
  let initialized = WINAPP_SDK.get().ok_or_else(|| {
    napi::Error::from_reason(
      "Windows App SDK is not initialized; call initWinappsdk before requesting its resources",
    )
  })?;

  initialized
    .context
    .resource_pri_path()
    .map_err(|e| napi::Error::from_reason(e.to_string()))
}

#[napi]
pub fn ro_initialize(apartment_type: Option<i32>) {
  use windows::Win32::System::WinRT::{
    RoInitialize, RO_INIT_MULTITHREADED, RO_INIT_SINGLETHREADED,
  };
  let init_type = match apartment_type.unwrap_or(1) {
    0 => RO_INIT_SINGLETHREADED,
    _ => RO_INIT_MULTITHREADED,
  };
  // Ignore "already initialized" (S_FALSE) and "changed mode" (RPC_E_CHANGED_MODE)
  // This allows dynwinrt to work in hosts like Electron that pre-initialize COM.
  let _ = unsafe { RoInitialize(init_type) };
}

pub(crate) fn set_winui_dispatcher_loop_active(active: bool) {
  if active {
    WINUI_DISPATCHER_LOOP_ENTERED.with(|state| state.set(true));
  }
  WINUI_DISPATCHER_LOOP_ACTIVE.with(|state| state.set(active));
}

// ======================================================================
// Core types — DynWinRTType, DynWinRtMethodSig, DynWinRtMethodHandle, WinGUID
// ======================================================================

#[napi]
pub struct DynWinRTType(dynwinrt::TypeHandle);

impl DynWinRTType {
  pub(crate) fn type_handle(&self) -> dynwinrt::TypeHandle {
    self.0.clone()
  }
}

#[napi]
impl DynWinRTType {
  #[napi]
  pub fn i32() -> Self {
    DynWinRTType(TABLE.i32_type())
  }

  #[napi]
  pub fn i64() -> Self {
    DynWinRTType(TABLE.i64_type())
  }

  #[napi]
  pub fn hstring() -> Self {
    DynWinRTType(TABLE.hstring())
  }

  #[napi]
  pub fn object() -> Self {
    DynWinRTType(TABLE.object())
  }

  #[napi]
  pub fn f64() -> Self {
    DynWinRTType(TABLE.f64_type())
  }

  #[napi]
  pub fn f32() -> Self {
    DynWinRTType(TABLE.f32_type())
  }

  #[napi]
  pub fn u8() -> Self {
    DynWinRTType(TABLE.u8_type())
  }

  #[napi]
  pub fn u32() -> Self {
    DynWinRTType(TABLE.u32_type())
  }

  #[napi]
  pub fn u64() -> Self {
    DynWinRTType(TABLE.u64_type())
  }

  #[napi]
  pub fn i8_type() -> Self {
    DynWinRTType(TABLE.i8_type())
  }

  #[napi]
  pub fn i16() -> Self {
    DynWinRTType(TABLE.i16_type())
  }

  #[napi]
  pub fn u16() -> Self {
    DynWinRTType(TABLE.u16_type())
  }

  #[napi]
  pub fn bool_type() -> Self {
    DynWinRTType(TABLE.bool_type())
  }

  #[napi]
  pub fn runtime_class(name: String, default_interface_type: &DynWinRTType) -> Self {
    DynWinRTType(TABLE.runtime_class(name, &default_interface_type.0))
  }

  #[napi]
  pub fn guid_type() -> Self {
    DynWinRTType(TABLE.guid_type())
  }

  #[napi]
  pub fn char16() -> Self {
    DynWinRTType(TABLE.char16_type())
  }

  #[napi]
  pub fn hresult() -> Self {
    DynWinRTType(TABLE.hresult())
  }

  #[napi]
  pub fn interface(iid: &WinGUID) -> Self {
    DynWinRTType(TABLE.interface(iid.0))
  }

  #[napi]
  pub fn delegate(iid: &WinGUID) -> Self {
    DynWinRTType(TABLE.delegate(iid.0))
  }

  #[napi]
  pub fn i_async_action() -> Self {
    DynWinRTType(TABLE.async_action())
  }

  #[napi]
  pub fn i_async_action_with_progress(progress_type: &DynWinRTType) -> Self {
    DynWinRTType(TABLE.async_action_with_progress(&progress_type.0))
  }

  #[napi]
  pub fn i_async_operation(result_type: &DynWinRTType) -> Self {
    DynWinRTType(TABLE.async_operation(&result_type.0))
  }

  #[napi]
  pub fn i_async_operation_with_progress(
    result_type: &DynWinRTType,
    progress_type: &DynWinRTType,
  ) -> Self {
    DynWinRTType(TABLE.async_operation_with_progress(&result_type.0, &progress_type.0))
  }

  /// Create a named struct type with WinRT full name (for correct IID signature).
  /// Deduplicates by name — calling with the same name twice returns the existing handle.
  #[napi]
  pub fn struct_type(name: String, fields: Vec<&DynWinRTType>) -> Self {
    let handles: Vec<dynwinrt::TypeHandle> = fields.iter().map(|f| f.0.clone()).collect();
    DynWinRTType(TABLE.struct_type(&name, &handles))
  }

  /// Create a named enum type (ABI = i32, carries name for signature).
  /// `member_names` and `member_values` are parallel arrays of enum member definitions.
  #[napi]
  pub fn enum_type(
    name: String,
    member_names: Option<Vec<String>>,
    member_values: Option<Vec<i32>>,
  ) -> Self {
    let members = match (member_names, member_values) {
      (Some(names), Some(values)) => names.into_iter().zip(values).collect(),
      _ => Vec::new(),
    };
    DynWinRTType(TABLE.enum_type(&name, members))
  }

  /// Look up an enum member's i32 value by name.
  #[napi]
  pub fn get_enum_value(enum_name: String, member_name: String) -> Option<i32> {
    TABLE.get_enum_value(&enum_name, &member_name)
  }

  /// Declare a parameterized type (generic instantiation, e.g. IReference<UInt64>).
  #[napi]
  pub fn parameterized(generic_iid: &WinGUID, args: Vec<&DynWinRTType>) -> Self {
    let handles: Vec<dynwinrt::TypeHandle> = args.iter().map(|a| a.0.clone()).collect();
    let generic = TABLE.generic(generic_iid.0, handles.len() as u32);
    DynWinRTType(TABLE.parameterized(&generic, &handles))
  }

  /// Declare an array-of-element type for method signatures.
  #[napi]
  pub fn array_type(element_type: &DynWinRTType) -> Self {
    DynWinRTType(TABLE.array(&element_type.0))
  }

  /// Register an interface in the MetadataTable.
  /// Returns self (Interface TypeHandle) for chaining `.addMethod()`.
  #[napi]
  pub fn register_interface(name: String, iid: &WinGUID) -> Self {
    DynWinRTType(TABLE.register_interface(&name, iid.0))
  }

  /// Add a method to this interface using a MethodSignature.
  /// Methods are numbered starting at vtable index 6.
  #[napi]
  pub fn add_method(&self, name: String, sig: &DynWinRTMethodSig) -> DynWinRTType {
    DynWinRTType(self.0.clone().add_method(&name, sig.0.clone()))
  }

  /// Get a MethodHandle by vtable index (6 = first user method).
  #[napi]
  pub fn method(&self, vtable_index: i32) -> napi::Result<DynWinRTMethodHandle> {
    self
      .0
      .method(vtable_index as usize)
      .map(DynWinRTMethodHandle)
      .ok_or_else(|| {
        napi::Error::from_reason(format!("No method at vtable index {}", vtable_index))
      })
  }

  /// Get a MethodHandle by method name.
  #[napi]
  pub fn method_by_name(&self, name: String) -> napi::Result<DynWinRTMethodHandle> {
    self
      .0
      .method_by_name(&name)
      .map(DynWinRTMethodHandle)
      .ok_or_else(|| napi::Error::from_reason(format!("Method '{}' not found", name)))
  }

  /// Compute the IID for this type (works for Interface, Parameterized, RuntimeClass, etc.)
  #[napi]
  pub fn iid(&self) -> napi::Result<WinGUID> {
    self
      .0
      .iid()
      .map(WinGUID)
      .ok_or_else(|| napi::Error::from_reason("Type has no IID"))
  }
}

#[napi]
#[derive(Debug, Clone, Copy)]
pub struct WinGUID(windows::core::GUID);

#[napi]
impl WinGUID {
  #[napi]
  pub fn parse(guid_str: String) -> napi::Result<Self> {
    let guid = windows::core::GUID::try_from(guid_str.as_str())
      .map_err(|_| napi::Error::from_reason(format!("Invalid GUID: '{}'", guid_str)))?;
    Ok(WinGUID(guid))
  }

  #[napi]
  pub fn to_string(&self) -> String {
    format!("{:?}", self.0)
  }
}

// ======================================================================
// MethodSignature binding — builder for method parameter descriptions
// ======================================================================

#[napi]
pub struct DynWinRTMethodSig(dynwinrt::MethodSignature);
unsafe impl Send for DynWinRTMethodSig {}
unsafe impl Sync for DynWinRTMethodSig {}

#[napi]
impl DynWinRTMethodSig {
  #[napi(constructor)]
  pub fn new() -> Self {
    DynWinRTMethodSig(dynwinrt::MethodSignature::new(&*TABLE))
  }

  /// Add an [in] parameter.
  #[napi]
  pub fn add_in(&self, typ: &DynWinRTType) -> DynWinRTMethodSig {
    DynWinRTMethodSig(self.0.clone().add_in(typ.0.clone()))
  }

  /// Add an [out] parameter.
  #[napi]
  pub fn add_out(&self, typ: &DynWinRTType) -> DynWinRTMethodSig {
    DynWinRTMethodSig(self.0.clone().add_out(typ.0.clone()))
  }

  /// Add a FillArray [out] parameter: caller allocates buffer, callee fills it.
  #[napi]
  pub fn add_out_fill(&self, typ: &DynWinRTType) -> DynWinRTMethodSig {
    DynWinRTMethodSig(self.0.clone().add_out_fill(typ.0.clone()))
  }
}

// ======================================================================
// MethodHandle binding
// ======================================================================

#[napi]
pub struct DynWinRTMethodHandle(dynwinrt::MethodHandle);
unsafe impl Send for DynWinRTMethodHandle {}
unsafe impl Sync for DynWinRTMethodHandle {}

#[napi]
impl DynWinRTMethodHandle {
  /// Invoke this method on a COM object.
  #[napi]
  pub fn invoke(
    &self,
    obj: &DynWinRTValue,
    args: Vec<&DynWinRTValue>,
  ) -> napi::Result<DynWinRTValue> {
    let raw = match &obj.0 {
      dynwinrt::WinRTValue::Object(o) => o.as_raw(),
      _ => {
        return Err(napi::Error::from_reason(
          "invoke() requires an Object value",
        ));
      }
    };
    let wrt_args: Vec<dynwinrt::WinRTValue> = args.iter().map(|a| a.0.clone()).collect();
    let results = self
      .0
      .invoke(raw, &wrt_args)
      .map_err(|e| napi::Error::from_reason(e.message()))?;
    if results.is_empty() {
      Ok(DynWinRTValue::new(dynwinrt::WinRTValue::I32(0)))
    } else {
      Ok(DynWinRTValue::new(results.into_iter().next().ok_or_else(
        || napi::Error::from_reason("invoke: method returned no results"),
      )?))
    }
  }

  /// Schedule a blocking WinUI Application.Start invocation after the current
  /// JavaScript callback unwinds.
  #[napi]
  pub fn invoke_scheduled<'env>(
    &self,
    env: Env,
    obj: &DynWinRTValue,
    args: Vec<&DynWinRTValue>,
  ) -> napi::Result<PromiseRaw<'env, ()>> {
    scheduled_start::schedule(
      env,
      self.0.clone(),
      obj.0.clone(),
      args.into_iter().map(|value| value.0.clone()).collect(),
    )
  }

  /// Like `invoke`, but returns all out-parameters as an array.
  /// Used for methods with multiple out params (e.g. IVector.IndexOf → [u32 index, bool found]).
  #[napi]
  pub fn invoke_all(
    &self,
    obj: &DynWinRTValue,
    args: Vec<&DynWinRTValue>,
  ) -> napi::Result<Vec<DynWinRTValue>> {
    let raw = match &obj.0 {
      dynwinrt::WinRTValue::Object(o) => o.as_raw(),
      _ => {
        return Err(napi::Error::from_reason(
          "invoke_all() requires an Object value",
        ));
      }
    };
    let wrt_args: Vec<dynwinrt::WinRTValue> = args.iter().map(|a| a.0.clone()).collect();
    let results = self
      .0
      .invoke(raw, &wrt_args)
      .map_err(|e| napi::Error::from_reason(e.message()))?;
    Ok(results.into_iter().map(DynWinRTValue::new).collect())
  }

  // --- Fast paths: skip Vec alloc + skip DynWinRTValue wrapping for result ---

  /// Getter → string (0 args, returns JS string directly, zero Vec allocation)
  #[napi]
  pub fn get_string(&self, obj: &DynWinRTValue) -> napi::Result<String> {
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("get_string: not an Object"))?
      .as_raw();
    let hs = self
      .0
      .call_getter_hstring(raw)
      .map_err(|e| napi::Error::from_reason(e.message()))?;
    Ok(hs.to_string())
  }

  /// Getter → i32 (0 args, returns JS number directly, zero Vec allocation)
  #[napi]
  pub fn get_i32(&self, obj: &DynWinRTValue) -> napi::Result<i32> {
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("get_i32: not an Object"))?
      .as_raw();
    self
      .0
      .call_getter_i32(raw)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  /// Getter → bool (0 args, returns JS boolean directly, zero Vec allocation)
  #[napi]
  pub fn get_bool(&self, obj: &DynWinRTValue) -> napi::Result<bool> {
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("get_bool: not an Object"))?
      .as_raw();
    self
      .0
      .call_getter_bool(raw)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  /// Getter → DynWinRTValue (0 args, returns wrapped object, zero Vec allocation)
  #[napi]
  pub fn get_obj(&self, obj: &DynWinRTValue) -> napi::Result<DynWinRTValue> {
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("get_obj: not an Object"))?
      .as_raw();
    self
      .0
      .call_getter_object(raw)
      .map(DynWinRTValue::new)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  #[napi]
  pub fn set_hstring(&self, obj: &DynWinRTValue, value: String) -> napi::Result<()> {
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("set_hstring: not an Object"))?
      .as_raw();
    self
      .0
      .call_setter_hstring(raw, &HSTRING::from(value))
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  #[napi]
  pub fn set_bool(&self, obj: &DynWinRTValue, value: bool) -> napi::Result<()> {
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("set_bool: not an Object"))?
      .as_raw();
    self
      .0
      .call_setter_bool(raw, value)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  #[napi]
  pub fn set_i32(&self, obj: &DynWinRTValue, value: i32) -> napi::Result<()> {
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("set_i32: not an Object"))?
      .as_raw();
    self
      .0
      .call_setter_i32(raw, value)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  #[napi]
  pub fn set_u32(&self, obj: &DynWinRTValue, value: u32) -> napi::Result<()> {
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("set_u32: not an Object"))?
      .as_raw();
    self
      .0
      .call_setter_u32(raw, value)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  #[napi]
  pub fn set_f32(&self, obj: &DynWinRTValue, value: f64) -> napi::Result<()> {
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("set_f32: not an Object"))?
      .as_raw();
    self
      .0
      .call_setter_f32(raw, value as f32)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  #[napi]
  pub fn set_f64(&self, obj: &DynWinRTValue, value: f64) -> napi::Result<()> {
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("set_f64: not an Object"))?
      .as_raw();
    self
      .0
      .call_setter_f64(raw, value)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  /// 1-arg invoke with hstring input → DynWinRTValue result
  #[napi]
  pub fn invoke_hstring(&self, obj: &DynWinRTValue, arg: String) -> napi::Result<DynWinRTValue> {
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("invoke_hstring: not an Object"))?
      .as_raw();
    let results = self
      .0
      .invoke(raw, &[dynwinrt::WinRTValue::HString(HSTRING::from(arg))])
      .map_err(|e| napi::Error::from_reason(e.message()))?;
    Ok(DynWinRTValue::new(results.into_iter().next().ok_or_else(
      || napi::Error::from_reason("invoke_hstring: no result"),
    )?))
  }

  /// 1-arg invoke with i32 input → DynWinRTValue result
  #[napi]
  pub fn invoke_i32(&self, obj: &DynWinRTValue, arg: i32) -> napi::Result<DynWinRTValue> {
    let raw = obj
      .0
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("invoke_i32: not an Object"))?
      .as_raw();
    let results = self
      .0
      .invoke(raw, &[dynwinrt::WinRTValue::I32(arg)])
      .map_err(|e| napi::Error::from_reason(e.message()))?;
    Ok(DynWinRTValue::new(results.into_iter().next().ok_or_else(
      || napi::Error::from_reason("invoke_i32: no result"),
    )?))
  }
}

// ======================================================================
// DynWinRTValue — main value container
// ======================================================================

#[napi]
pub struct DynWinRTValue(
  dynwinrt::WinRTValue,
  Option<com::NativePointerOwner>,
  com::PointerProvenance,
  Option<dynwinrt::com::NativeStructValue>,
  Option<dynwinrt::com::ComBufferValue>,
  Option<com::AutomationValue>,
  Option<ComApartmentBinding>,
);
unsafe impl Send for DynWinRTValue {}
unsafe impl Sync for DynWinRTValue {}

#[derive(Clone)]
struct ComApartmentBinding {
  owner_thread: std::thread::ThreadId,
}

impl DynWinRTValue {
  fn new(value: dynwinrt::WinRTValue) -> Self {
    Self(
      value,
      None,
      com::PointerProvenance::None,
      None,
      None,
      None,
      None,
    )
  }

  pub(crate) fn bind_current_com_apartment(&mut self) -> napi::Result<()> {
    if self.0.as_object().is_none() {
      return Err(napi::Error::from_reason(
        "Classic COM apartment binding requires a managed COM object",
      ));
    }
    let current = std::thread::current().id();
    match &self.6 {
      Some(binding) if binding.owner_thread != current => Err(napi::Error::from_reason(
        "Classic COM object is already bound to a different apartment thread",
      )),
      Some(_) => Ok(()),
      None => {
        self.6 = Some(ComApartmentBinding {
          owner_thread: current,
        });
        Ok(())
      }
    }
  }

  pub(crate) fn ensure_com_apartment(&self) -> napi::Result<()> {
    let current = std::thread::current().id();
    match &self.6 {
      Some(binding) if binding.owner_thread == current => Ok(()),
      Some(_) => Err(napi::Error::from_reason(
        "Classic COM object used from a different apartment thread",
      )),
      None => Err(napi::Error::from_reason(
        "Classic COM object must be apartment-bound before native invocation",
      )),
    }
  }

  pub(crate) fn ensure_existing_com_apartment(&self) -> napi::Result<()> {
    if self.6.is_some() {
      self.ensure_com_apartment()
    } else {
      Ok(())
    }
  }

  fn with_pointer_owner(value: dynwinrt::WinRTValue, owner: com::NativePointerOwner) -> Self {
    Self(
      value,
      Some(owner),
      com::PointerProvenance::Borrowed,
      None,
      None,
      None,
      None,
    )
  }

  fn with_borrowed_pointer(value: dynwinrt::WinRTValue) -> Self {
    Self(
      value,
      None,
      com::PointerProvenance::Borrowed,
      None,
      None,
      None,
      None,
    )
  }

  fn with_com_buffer(
    buffer: dynwinrt::com::ComBufferValue,
    owner: com::NativePointerOwner,
  ) -> Self {
    Self(
      dynwinrt::WinRTValue::Null,
      Some(owner),
      com::PointerProvenance::Borrowed,
      None,
      Some(buffer),
      None,
      None,
    )
  }

  fn from_com_result(
    value: dynwinrt::WinRTValue,
    output_kind: dynwinrt::com::PointerOutputKind,
  ) -> Self {
    let provenance = if matches!(value, dynwinrt::WinRTValue::RawPtr(_)) {
      match output_kind {
        dynwinrt::com::PointerOutputKind::None | dynwinrt::com::PointerOutputKind::Unclassified => {
          com::PointerProvenance::UnclassifiedOutput
        }
        dynwinrt::com::PointerOutputKind::Com => com::PointerProvenance::ComOutput,
        dynwinrt::com::PointerOutputKind::CoTaskMem => com::PointerProvenance::CoTaskMemOutput,
        dynwinrt::com::PointerOutputKind::Bstr => com::PointerProvenance::BstrOutput,
      }
    } else {
      com::PointerProvenance::None
    };
    let apartment = matches!(value, dynwinrt::WinRTValue::Object(_)).then(|| ComApartmentBinding {
      owner_thread: std::thread::current().id(),
    });
    Self(value, None, provenance, None, None, None, apartment)
  }

  fn from_com_value(
    value: dynwinrt::com::Value,
    output_kind: dynwinrt::com::PointerOutputKind,
  ) -> Self {
    match value {
      dynwinrt::com::Value::WinRt(value) => Self::from_com_result(value, output_kind),
      dynwinrt::com::Value::Bstr(value) => Self(
        dynwinrt::WinRTValue::Null,
        None,
        com::PointerProvenance::None,
        None,
        None,
        Some(com::AutomationValue::new(dynwinrt::com::Value::Bstr(value))),
        None,
      ),
      dynwinrt::com::Value::NativeStruct(value) => Self(
        dynwinrt::WinRTValue::Null,
        None,
        com::PointerProvenance::None,
        Some(value),
        None,
        None,
        None,
      ),
      dynwinrt::com::Value::NativeUnion(value) => Self(
        dynwinrt::WinRTValue::Null,
        None,
        com::PointerProvenance::None,
        None,
        None,
        Some(com::AutomationValue::new(
          dynwinrt::com::Value::NativeUnion(value),
        )),
        None,
      ),
      dynwinrt::com::Value::Variant(value) => Self(
        dynwinrt::WinRTValue::Null,
        None,
        com::PointerProvenance::None,
        None,
        None,
        Some(com::AutomationValue::new(dynwinrt::com::Value::Variant(
          value,
        ))),
        None,
      ),
      dynwinrt::com::Value::SafeArray(value) => Self(
        dynwinrt::WinRTValue::Null,
        None,
        com::PointerProvenance::None,
        None,
        None,
        Some(com::AutomationValue::new(dynwinrt::com::Value::SafeArray(
          value,
        ))),
        None,
      ),
      dynwinrt::com::Value::PropVariant(value) => Self(
        dynwinrt::WinRTValue::Null,
        None,
        com::PointerProvenance::None,
        None,
        None,
        Some(com::AutomationValue::new(
          dynwinrt::com::Value::PropVariant(value),
        )),
        None,
      ),
      dynwinrt::com::Value::DispatchParams(value) => Self(
        dynwinrt::WinRTValue::Null,
        None,
        com::PointerProvenance::None,
        None,
        None,
        Some(com::AutomationValue::new(
          dynwinrt::com::Value::DispatchParams(value),
        )),
        None,
      ),
      dynwinrt::com::Value::ExcepInfo(value) => Self(
        dynwinrt::WinRTValue::Null,
        None,
        com::PointerProvenance::None,
        None,
        None,
        Some(com::AutomationValue::new(dynwinrt::com::Value::ExcepInfo(
          value,
        ))),
        None,
      ),
      dynwinrt::com::Value::StatStg(value) => Self(
        dynwinrt::WinRTValue::Null,
        None,
        com::PointerProvenance::None,
        None,
        None,
        Some(com::AutomationValue::new(dynwinrt::com::Value::StatStg(
          value,
        ))),
        None,
      ),
      dynwinrt::com::Value::Buffer(value) => Self(
        dynwinrt::WinRTValue::Null,
        None,
        com::PointerProvenance::None,
        None,
        Some(value),
        None,
        None,
      ),
    }
  }

  fn to_com_value(&self) -> napi::Result<dynwinrt::com::Value> {
    if let Some(value) = &self.5 {
      value.to_com_value()
    } else if let Some(value) = &self.4 {
      Ok(dynwinrt::com::Value::Buffer(value.clone()))
    } else if let Some(value) = &self.3 {
      Ok(dynwinrt::com::Value::NativeStruct(value.clone()))
    } else {
      Ok(dynwinrt::com::Value::WinRt(self.0.clone()))
    }
  }

  fn release_native_pointer_output(&mut self) {
    let ptr = match &self.0 {
      dynwinrt::WinRTValue::RawPtr(ptr) => *ptr,
      _ => return,
    };
    let provenance = self.2;
    self.0 = dynwinrt::WinRTValue::Null;
    self.2 = com::PointerProvenance::None;
    if ptr.is_null() {
      return;
    }
    match provenance {
      com::PointerProvenance::ComOutput => {
        drop(unsafe { IUnknown::from_raw(ptr) });
      }
      com::PointerProvenance::CoTaskMemOutput => unsafe {
        windows::Win32::System::Com::CoTaskMemFree(Some(ptr));
      },
      com::PointerProvenance::BstrOutput => {
        drop(unsafe { windows::core::BSTR::from_raw(ptr.cast()) });
      }
      com::PointerProvenance::None
      | com::PointerProvenance::Borrowed
      | com::PointerProvenance::UnclassifiedOutput => {}
    }
  }
}

impl Drop for DynWinRTValue {
  fn drop(&mut self) {
    if self
      .6
      .as_ref()
      .is_some_and(|binding| binding.owner_thread != std::thread::current().id())
    {
      let value = mem::replace(&mut self.0, dynwinrt::WinRTValue::Null);
      mem::forget(value);
      return;
    }
    // After Application.Start returns, XAML has already torn down its thread
    // state. Leaking late projected COM references is safer than releasing
    // them into a destroyed DXamlCore; normal application teardown must call
    // release()/releaseProjected() before this process-exit fallback is needed.
    if winui_dispatcher_loop_exited() {
      self.2 = com::PointerProvenance::None;
      let value = mem::replace(&mut self.0, dynwinrt::WinRTValue::Null);
      mem::forget(value);
      if let Some(value) = self.4.take() {
        mem::forget(value);
      }
      if let Some(value) = &mut self.5 {
        value.leak_for_shutdown();
      }
    } else {
      self.release_native_pointer_output();
    }
  }
}

#[napi]
impl DynWinRTValue {
  #[napi]
  pub fn release(&mut self) -> napi::Result<()> {
    if self.6.is_some() {
      self.ensure_com_apartment()?;
    }
    self.release_native_pointer_output();
    self.0 = dynwinrt::WinRTValue::Null;
    self.1 = None;
    self.2 = com::PointerProvenance::None;
    self.3 = None;
    self.4 = None;
    self.5 = None;
    self.6 = None;
    Ok(())
  }

  #[napi]
  pub fn activation_factory(name: String) -> napi::Result<DynWinRTValue> {
    let factory = dynwinrt::ro_get_activation_factory_2(&HSTRING::from(&name)).map_err(|e| {
      napi::Error::from_reason(format!("ActivationFactory '{}': {}", name, e.message()))
    })?;
    Ok(DynWinRTValue::new(factory))
  }

  /// Create a composed WinUI Application that forwards IXamlMetadataProvider
  /// calls to the supplied provider.
  #[napi]
  pub fn create_xaml_application(
    metadata_provider: &DynWinRTValue,
    launched_callback: Option<&DynWinRTValue>,
  ) -> napi::Result<DynWinRTValue> {
    let provider = metadata_provider.0.as_object().ok_or_else(|| {
      napi::Error::from_reason("createXamlApplication: metadataProvider must be an Object")
    })?;
    let callback = launched_callback
      .map(|value| {
        value.0.as_object().ok_or_else(|| {
          napi::Error::from_reason("createXamlApplication: launchedCallback must be an Object")
        })
      })
      .transpose()?;
    dynwinrt::create_xaml_application(&provider, callback.as_ref())
      .map(DynWinRTValue::new)
      .map_err(|e| {
        napi::Error::from_reason(format!("createXamlApplication failed: {}", e.message()))
      })
  }

  #[napi]
  pub fn bool_value(value: bool) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::Bool(value))
  }
  #[napi]
  pub fn i8_value(value: i32) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::I8(value as i8))
  }
  #[napi]
  pub fn u8_value(value: u32) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::U8(value as u8))
  }
  #[napi]
  pub fn i16(value: i32) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::I16(value as i16))
  }
  #[napi]
  pub fn u16(value: u32) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::U16(value as u16))
  }
  #[napi]
  pub fn i32(value: i32) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::I32(value))
  }
  #[napi]
  pub fn u32(value: u32) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::U32(value))
  }
  #[napi]
  pub fn i64(value: Either<BigInt, f64>) -> napi::Result<DynWinRTValue> {
    let value = js_i64(value, "i64")?;
    Ok(DynWinRTValue::new(dynwinrt::WinRTValue::I64(value)))
  }
  #[napi]
  pub fn u64(value: Either<BigInt, f64>) -> napi::Result<DynWinRTValue> {
    let value = js_u64(value, "u64")?;
    Ok(DynWinRTValue::new(dynwinrt::WinRTValue::U64(value)))
  }
  #[napi]
  pub fn f32(value: f64) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::F32(value as f32))
  }
  #[napi]
  pub fn f64(value: f64) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::F64(value))
  }
  /// Create an enum value from an i32. The type_handle must be an enum type.
  #[napi]
  pub fn enum_value(enum_type: &DynWinRTType, value: i32) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::Enum {
      value,
      type_handle: enum_type.0.clone(),
    })
  }

  #[napi]
  pub fn box_reference(
    value: &DynWinRTValue,
    value_type: &DynWinRTType,
  ) -> napi::Result<DynWinRTValue> {
    dynwinrt::box_ireference(value.0.clone(), value_type.0.clone())
      .map(DynWinRTValue::new)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  /// Get the i32 value of an enum. Returns None if not an enum.
  #[napi]
  pub fn get_enum_int(&self) -> Option<i32> {
    match &self.0 {
      dynwinrt::WinRTValue::Enum { value, .. } => Some(*value),
      _ => None,
    }
  }

  /// Get the member name of an enum value. Returns None if not an enum or no matching member.
  #[napi]
  pub fn get_enum_name(&self) -> Option<String> {
    match &self.0 {
      dynwinrt::WinRTValue::Enum { value, type_handle } => type_handle.enum_member_name(*value),
      _ => None,
    }
  }

  #[napi]
  pub fn hstring(value: String) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::HString(HSTRING::from(value)))
  }
  #[napi]
  pub fn guid(value: &WinGUID) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::Guid(value.0))
  }
  #[napi]
  pub fn null_value() -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::Null)
  }

  /// Create an IVector<T> from items. The element_type is used for IID computation.
  /// Items are passed as DynWinRTValue objects (Object or Struct-wrapped values).
  #[napi]
  pub fn create_vector(
    items: Vec<&DynWinRTValue>,
    element_type: &DynWinRTType,
  ) -> napi::Result<DynWinRTValue> {
    let iids = TABLE.vector_iids(&element_type.0);
    let wrt_items: Vec<dynwinrt::WinRTValue> = items.iter().map(|i| i.0.clone()).collect();
    let vector = dynwinrt::vector::create_vector_from_values(&wrt_items, &element_type.0, iids)
      .map_err(|error| napi::Error::from_reason(error.message()))?;
    Ok(DynWinRTValue::new(dynwinrt::WinRTValue::Object(vector)))
  }

  /// Create an IMap<K,V> from parallel key/value arrays.
  /// Keys and values must be Object values (e.g. PropertyValue-boxed strings/ints).
  #[napi]
  pub fn create_map(
    keys: Vec<&DynWinRTValue>,
    values: Vec<&DynWinRTValue>,
    key_type: &DynWinRTType,
    value_type: &DynWinRTType,
  ) -> napi::Result<DynWinRTValue> {
    if keys.len() != values.len() {
      return Err(napi::Error::from_reason(
        "createMap: keys and values must have the same length",
      ));
    }
    let iids = TABLE.map_iids(&key_type.0, &value_type.0);
    let entries: Vec<(dynwinrt::WinRTValue, dynwinrt::WinRTValue)> = keys
      .iter()
      .zip(values.iter())
      .map(|(key, value)| (key.0.clone(), value.0.clone()))
      .collect();
    let map = dynwinrt::map::create_map_from_values(&entries, &key_type.0, &value_type.0, iids)
      .map_err(|error| napi::Error::from_reason(error.message()))?;
    Ok(DynWinRTValue::new(dynwinrt::WinRTValue::Object(map)))
  }

  #[napi]
  pub fn to_promise<'env>(&self, env: Env) -> napi::Result<PromiseRaw<'env, DynWinRTValue>> {
    let operation = match &self.0 {
      dynwinrt::WinRTValue::Async(_) => self.0.clone(),
      _ => return Err(napi::Error::from_reason("toPromise: not an async value")),
    };
    async_promise::to_promise(env, operation)
  }

  /// Cancel the underlying WinRT async operation (calls `IAsyncInfo::Cancel`).
  /// Safe to call multiple times or on already-completed operations.
  ///
  /// Throws if this value is not an async operation.
  #[napi]
  pub fn cancel(&self) -> napi::Result<()> {
    let async_info = match &self.0 {
      dynwinrt::WinRTValue::Async(a) => a,
      _ => return Err(napi::Error::from_reason("cancel: not an async value")),
    };
    async_info
      .cancel()
      .map_err(|e| napi::Error::from_reason(format!("Cancel failed: {}", e.message())))
  }

  #[napi]
  pub fn on_progress(
    &self,
    #[napi(ts_arg_type = "(progress: DynWinRtValue) => void")]
    callback: napi::bindgen_prelude::Function<'static, DynWinRTValue, ()>,
  ) -> napi::Result<()> {
    let async_info = match &self.0 {
      dynwinrt::WinRTValue::Async(a) => a,
      _ => return Err(napi::Error::from_reason("onProgress: not an async value")),
    };

    let progress_type = async_info
      .progress_type()
      .ok_or_else(|| napi::Error::from_reason("onProgress: not a WithProgress async type"))?;
    ensure_progress_type_supported(&progress_type)?;

    let handler_iid = async_info
      .progress_handler_iid()
      .ok_or_else(|| napi::Error::from_reason("onProgress: cannot compute progress handler IID"))?;

    use napi::bindgen_prelude::ToNapiValue;
    use napi::JsValue;

    // Progress callbacks must not keep an otherwise idle Node process alive.
    let raw_env = callback.value().env;
    let raw_callback = napi::JsValue::raw(&callback);
    let tsfn = managed_tsfn::ManagedTsfn::create(
      raw_env,
      raw_callback,
      1024,
      true,
      |value: DynWinRTValue, env| {
        unsafe { DynWinRTValue::to_napi_value(env, value) }.map(|value| vec![value])
      },
      None,
    )?;
    let progress_cb: dynwinrt::ProgressResultCallback =
      Box::new(move |val: dynwinrt::WinRTValue| {
        let status = tsfn.call(DynWinRTValue::new(val));
        if status == napi::Status::Ok {
          windows::core::HRESULT(0)
        } else {
          if status != napi::Status::QueueFull {
            eprintln!("[dynwinrt] progress callback queue failed: {status}");
          }
          windows::core::HRESULT(0x80004005u32 as i32)
        }
      });
    let handler =
      dynwinrt::try_create_progress_handler_with_result(handler_iid, progress_type, progress_cb)
        .map_err(|error| {
          napi::Error::from_reason(format!(
            "onProgress: failed to create progress handler: {}",
            error.message()
          ))
        })?;

    async_info
      .set_progress_handler(&handler)
      .map_err(|e| napi::Error::from_reason(format!("SetProgress failed: {}", e.message())))?;

    Ok(())
  }

  #[napi]
  pub fn to_string(&self) -> String {
    if let Some(automation) = &self.5 {
      if let Ok(dynwinrt::com::Value::Bstr(value)) = automation.to_com_value() {
        return value.as_deref().unwrap_or_default().to_string();
      }
    }
    match &self.0 {
      dynwinrt::WinRTValue::HString(s) => s.to_string(),
      dynwinrt::WinRTValue::I32(i) => i.to_string(),
      dynwinrt::WinRTValue::I64(i) => i.to_string(),
      dynwinrt::WinRTValue::Guid(g) => format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        g.data1,
        g.data2,
        g.data3,
        g.data4[0],
        g.data4[1],
        g.data4[2],
        g.data4[3],
        g.data4[4],
        g.data4[5],
        g.data4[6],
        g.data4[7]
      ),
      dynwinrt::WinRTValue::Object(o) => format!("Object: {:?}", o),
      _ => "Unsupported type".to_string(),
    }
  }

  #[napi]
  pub fn cast(&self, iid: &WinGUID) -> napi::Result<DynWinRTValue> {
    self.ensure_existing_com_apartment()?;
    let result = self
      .0
      .cast(&iid.0)
      .map_err(|e| napi::Error::from_reason(format!("QueryInterface failed: {}", e.message())))?;
    let mut result = DynWinRTValue::new(result);
    result.6 = self.6.clone();
    Ok(result)
  }

  #[napi]
  pub fn to_number(&self) -> napi::Result<f64> {
    Ok(match &self.0 {
      dynwinrt::WinRTValue::Bool(b) => {
        if *b {
          1.0
        } else {
          0.0
        }
      }
      dynwinrt::WinRTValue::I8(i) => f64::from(*i),
      dynwinrt::WinRTValue::U8(i) => f64::from(*i),
      dynwinrt::WinRTValue::I16(i) => f64::from(*i),
      dynwinrt::WinRTValue::U16(i) => f64::from(*i),
      dynwinrt::WinRTValue::I32(i) => f64::from(*i),
      dynwinrt::WinRTValue::U32(i) => f64::from(*i),
      dynwinrt::WinRTValue::HResult(hr) => f64::from(hr.0),
      dynwinrt::WinRTValue::Enum { value, .. } => f64::from(*value),
      _ => {
        return Err(napi::Error::from_reason(format!(
          "Cannot convert {:?} to number",
          self.0.get_type_kind(),
        )));
      }
    })
  }

  #[napi]
  pub fn to_bool(&self) -> napi::Result<bool> {
    match &self.0 {
      dynwinrt::WinRTValue::Bool(b) => Ok(*b),
      _ => self.to_number().map(|value| value != 0.0),
    }
  }

  #[napi]
  pub fn to_i64(&self) -> napi::Result<i64> {
    let value = match &self.0 {
      dynwinrt::WinRTValue::I64(i) => *i,
      dynwinrt::WinRTValue::U64(i) => i64::try_from(*i).map_err(|_| {
        napi::Error::from_reason("Cannot convert u64 value greater than i64::MAX to i64")
      })?,
      _ => self.to_number().map(|value| value as i64)?,
    };
    js_safe_i64(value, "toI64")
  }

  #[napi]
  pub fn to_i64_bigint(&self) -> napi::Result<BigInt> {
    match &self.0 {
      dynwinrt::WinRTValue::I64(value) => Ok(BigInt::from(*value)),
      _ => Err(napi::Error::from_reason(
        "toI64Bigint requires a signed 64-bit value",
      )),
    }
  }

  #[napi]
  pub fn to_u64_bigint(&self) -> napi::Result<BigInt> {
    match &self.0 {
      dynwinrt::WinRTValue::U64(value) => Ok(BigInt::from(*value)),
      _ => Err(napi::Error::from_reason(
        "toU64Bigint requires an unsigned 64-bit value",
      )),
    }
  }

  #[napi]
  pub fn to_f64(&self) -> napi::Result<f64> {
    match &self.0 {
      dynwinrt::WinRTValue::F64(f) => Ok(*f),
      dynwinrt::WinRTValue::F32(f) => Ok(*f as f64),
      _ => self.to_number(),
    }
  }

  #[napi]
  pub fn to_guid(&self) -> napi::Result<WinGUID> {
    match &self.0 {
      dynwinrt::WinRTValue::Guid(g) => Ok(WinGUID(*g)),
      _ => Err(napi::Error::from_reason("Value is not a GUID")),
    }
  }

  #[napi]
  pub fn is_null(&self) -> bool {
    self.3.is_none() && self.4.is_none() && self.5.is_none() && self.0.is_null_object()
  }

  #[napi]
  pub fn as_raw(&self) -> napi::Result<i64> {
    match &self.0 {
      dynwinrt::WinRTValue::Object(o) => Ok(o.as_raw() as i64),
      _ => Err(napi::Error::from_reason(
        "Cannot get raw pointer from non-object",
      )),
    }
  }

  #[napi]
  pub fn identity_raw(&self) -> napi::Result<i64> {
    match &self.0 {
      dynwinrt::WinRTValue::Object(object) => object
        .cast::<IUnknown>()
        .map(|identity| identity.as_raw() as i64)
        .map_err(|error| napi::Error::from_reason(error.message())),
      _ => Err(napi::Error::from_reason(
        "Cannot get COM identity from a non-object value",
      )),
    }
  }

  // -- Array / Struct extraction --

  #[napi]
  pub fn is_array(&self) -> bool {
    self.0.as_array().is_some()
  }

  #[napi]
  pub fn as_array(&self) -> napi::Result<DynWinRTArray> {
    match &self.0 {
      dynwinrt::WinRTValue::Array(data) => Ok(DynWinRTArray(data.clone())),
      _ => Err(napi::Error::from_reason("Value is not an Array")),
    }
  }

  #[napi]
  pub fn is_struct(&self) -> bool {
    self.0.as_struct().is_some()
  }

  #[napi]
  pub fn as_struct(&self) -> napi::Result<DynWinRTStruct> {
    match &self.0 {
      dynwinrt::WinRTValue::Struct(data) => Ok(DynWinRTStruct(data.clone())),
      _ => Err(napi::Error::from_reason("Value is not a Struct")),
    }
  }
}

// ======================================================================
// Array binding — blittable fast path via typed Vec, generic fallback
// ======================================================================

#[napi]
pub struct DynWinRTArray(dynwinrt::ArrayData);
unsafe impl Send for DynWinRTArray {}
unsafe impl Sync for DynWinRTArray {}

#[napi]
impl DynWinRTArray {
  #[napi]
  pub fn len(&self) -> u32 {
    self.0.len() as u32
  }

  /// Per-element access (works for all element types).
  #[napi]
  pub fn get(&self, index: f64) -> napi::Result<DynWinRTValue> {
    let index = js_u32(index, "get")? as usize;
    if index >= self.0.len() {
      return Err(napi::Error::from_reason(format!(
        "Array index {index} is out of bounds for length {}",
        self.0.len(),
      )));
    }
    Ok(DynWinRTValue::new(self.0.get(index)))
  }

  /// Convert all elements to DynWinRTValue array.
  #[napi]
  pub fn to_values(&self) -> Vec<DynWinRTValue> {
    (0..self.0.len())
      .map(|i| DynWinRTValue::new(self.0.get(i)))
      .collect()
  }

  // -- Typed batch conversions --

  #[napi]
  pub fn to_i8_vec(&self) -> napi::Result<Vec<i32>> {
    collect_typed_array(
      &self.0,
      "toI8Vec",
      |kind| kind == dynwinrt::TypeKind::I8,
      |value: i8| i32::from(value),
      |value| match value {
        dynwinrt::WinRTValue::I8(value) => Some(i32::from(value)),
        _ => None,
      },
    )
  }

  #[napi]
  pub fn to_u8_vec(&self) -> napi::Result<Vec<u8>> {
    collect_typed_array(
      &self.0,
      "toU8Vec",
      |kind| matches!(kind, dynwinrt::TypeKind::U8 | dynwinrt::TypeKind::Bool),
      |value: u8| value,
      |value| match value {
        dynwinrt::WinRTValue::U8(value) => Some(value),
        dynwinrt::WinRTValue::Bool(value) => Some(u8::from(value)),
        _ => None,
      },
    )
  }

  /// Return the u8 array data as a Node.js Buffer (zero-copy friendly, much
  /// more memory-efficient than to_u8_vec for large byte arrays).
  #[napi]
  pub fn to_buffer(&self) -> napi::Result<napi::bindgen_prelude::Buffer> {
    self.to_u8_vec().map(Into::into)
  }

  #[napi]
  pub fn to_i16_vec(&self) -> napi::Result<Vec<i32>> {
    collect_typed_array(
      &self.0,
      "toI16Vec",
      |kind| kind == dynwinrt::TypeKind::I16,
      |value: i16| i32::from(value),
      |value| match value {
        dynwinrt::WinRTValue::I16(value) => Some(i32::from(value)),
        _ => None,
      },
    )
  }

  #[napi]
  pub fn to_u16_vec(&self) -> napi::Result<Vec<u32>> {
    collect_typed_array(
      &self.0,
      "toU16Vec",
      |kind| matches!(kind, dynwinrt::TypeKind::U16 | dynwinrt::TypeKind::Char16),
      |value: u16| u32::from(value),
      |value| match value {
        dynwinrt::WinRTValue::U16(value) => Some(u32::from(value)),
        _ => None,
      },
    )
  }

  #[napi]
  pub fn to_i32_vec(&self) -> napi::Result<Vec<i32>> {
    collect_typed_array(
      &self.0,
      "toI32Vec",
      |kind| {
        matches!(
          kind,
          dynwinrt::TypeKind::I32 | dynwinrt::TypeKind::Enum(_) | dynwinrt::TypeKind::HResult
        )
      },
      |value: i32| value,
      |value| match value {
        dynwinrt::WinRTValue::I32(value) | dynwinrt::WinRTValue::Enum { value, .. } => Some(value),
        dynwinrt::WinRTValue::HResult(value) => Some(value.0),
        _ => None,
      },
    )
  }

  #[napi]
  pub fn to_u32_vec(&self) -> napi::Result<Vec<u32>> {
    collect_typed_array(
      &self.0,
      "toU32Vec",
      |kind| kind == dynwinrt::TypeKind::U32,
      |value: u32| value,
      |value| match value {
        dynwinrt::WinRTValue::U32(value) => Some(value),
        _ => None,
      },
    )
  }

  #[napi]
  pub fn to_f32_vec(&self) -> napi::Result<Vec<f32>> {
    collect_typed_array(
      &self.0,
      "toF32Vec",
      |kind| kind == dynwinrt::TypeKind::F32,
      |value: f32| value,
      |value| match value {
        dynwinrt::WinRTValue::F32(value) => Some(value),
        _ => None,
      },
    )
  }

  #[napi]
  pub fn to_f64_vec(&self) -> napi::Result<Vec<f64>> {
    collect_typed_array(
      &self.0,
      "toF64Vec",
      |kind| kind == dynwinrt::TypeKind::F64,
      |value: f64| value,
      |value| match value {
        dynwinrt::WinRTValue::F64(value) => Some(value),
        _ => None,
      },
    )
  }

  #[napi]
  pub fn to_i64_vec(&self) -> napi::Result<Vec<BigInt>> {
    collect_typed_array(
      &self.0,
      "toI64Vec",
      |kind| kind == dynwinrt::TypeKind::I64,
      |value: i64| BigInt::from(value),
      |value| match value {
        dynwinrt::WinRTValue::I64(value) => Some(BigInt::from(value)),
        _ => None,
      },
    )
  }

  #[napi]
  pub fn to_u64_vec(&self) -> napi::Result<Vec<BigInt>> {
    collect_typed_array(
      &self.0,
      "toU64Vec",
      |kind| kind == dynwinrt::TypeKind::U64,
      |value: u64| BigInt::from(value),
      |value| match value {
        dynwinrt::WinRTValue::U64(value) => Some(BigInt::from(value)),
        _ => None,
      },
    )
  }

  // -- Batch string conversion --

  #[napi]
  pub fn to_string_vec(&self) -> Vec<String> {
    (0..self.0.len())
      .map(|i| match self.0.get(i) {
        dynwinrt::WinRTValue::HString(s) => s.to_string(),
        other => format!("{:?}", other),
      })
      .collect()
  }

  // -- Construction from JS typed arrays --

  #[napi]
  pub fn from_i8_values(values: Vec<i32>) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> = values
      .into_iter()
      .map(|v| dynwinrt::WinRTValue::I8(v as i8))
      .collect();
    DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.i8_type(), &wvals))
  }

  #[napi]
  pub fn from_u8_values(values: Vec<u8>) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> =
      values.into_iter().map(dynwinrt::WinRTValue::U8).collect();
    DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.u8_type(), &wvals))
  }

  /// Build a u8 DynWinRtArray from a JS `Uint8Array` (zero-copy view into V8
  /// memory on the way in; much more efficient than fromU8Values for large
  /// byte buffers because the caller doesn't need to allocate a boxed
  /// `Array<number>` of length N).
  #[napi]
  pub fn from_uint8_array(values: napi::bindgen_prelude::Uint8Array) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> = values
      .iter()
      .map(|&v| dynwinrt::WinRTValue::U8(v))
      .collect();
    DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.u8_type(), &wvals))
  }

  #[napi]
  pub fn from_i16_values(values: Vec<i32>) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> = values
      .into_iter()
      .map(|v| dynwinrt::WinRTValue::I16(v as i16))
      .collect();
    DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.i16_type(), &wvals))
  }

  #[napi]
  pub fn from_u16_values(values: Vec<u32>) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> = values
      .into_iter()
      .map(|v| dynwinrt::WinRTValue::U16(v as u16))
      .collect();
    DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.u16_type(), &wvals))
  }

  #[napi]
  pub fn from_i32_values(values: Vec<i32>) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> =
      values.into_iter().map(dynwinrt::WinRTValue::I32).collect();
    DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.i32_type(), &wvals))
  }

  #[napi]
  pub fn from_u32_values(values: Vec<u32>) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> =
      values.into_iter().map(dynwinrt::WinRTValue::U32).collect();
    DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.u32_type(), &wvals))
  }

  #[napi]
  pub fn from_f32_values(values: Vec<f64>) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> = values
      .into_iter()
      .map(|v| dynwinrt::WinRTValue::F32(v as f32))
      .collect();
    DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.f32_type(), &wvals))
  }

  #[napi]
  pub fn from_f64_values(values: Vec<f64>) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> =
      values.into_iter().map(dynwinrt::WinRTValue::F64).collect();
    DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.f64_type(), &wvals))
  }

  #[napi]
  pub fn from_i64_values(values: Vec<Either<BigInt, f64>>) -> napi::Result<DynWinRTArray> {
    let wvals: Vec<dynwinrt::WinRTValue> = values
      .into_iter()
      .enumerate()
      .map(|(index, value)| {
        js_i64(value, &format!("fromI64Values[{index}]")).map(dynwinrt::WinRTValue::I64)
      })
      .collect::<napi::Result<_>>()?;
    Ok(DynWinRTArray(dynwinrt::ArrayData::from_values(
      TABLE.i64_type(),
      &wvals,
    )))
  }

  #[napi]
  pub fn from_u64_values(values: Vec<Either<BigInt, f64>>) -> napi::Result<DynWinRTArray> {
    let wvals: Vec<dynwinrt::WinRTValue> = values
      .into_iter()
      .enumerate()
      .map(|(index, value)| {
        js_u64(value, &format!("fromU64Values[{index}]")).map(dynwinrt::WinRTValue::U64)
      })
      .collect::<napi::Result<_>>()?;
    Ok(DynWinRTArray(dynwinrt::ArrayData::from_values(
      TABLE.u64_type(),
      &wvals,
    )))
  }

  #[napi]
  pub fn from_string_values(values: Vec<String>) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> = values
      .into_iter()
      .map(|s| dynwinrt::WinRTValue::HString(HSTRING::from(&s)))
      .collect();
    DynWinRTArray(dynwinrt::ArrayData::from_values(
      TABLE.make(dynwinrt::TypeKind::HString),
      &wvals,
    ))
  }

  /// Build a DynWinRtArray of WinRT object/interface elements.
  ///
  /// Use for `T[]` ABI in-parameters where `T` is a runtime class or
  /// interface — for example, `ModelCatalog(ModelCatalogSource[] sources)`.
  /// Items are passed as DynWinRTValue handles (typically Object-wrapped),
  /// and the element type drives ABI size and IID computation.
  #[napi]
  pub fn from_object_values(
    values: Vec<&DynWinRTValue>,
    element_type: &DynWinRTType,
  ) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> = values.iter().map(|v| v.0.clone()).collect();
    DynWinRTArray(dynwinrt::ArrayData::from_values(
      element_type.0.clone(),
      &wvals,
    ))
  }

  /// Wrap as DynWinRTValue::Array for passing to call().
  #[napi]
  pub fn to_value(&self) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::Array(self.0.clone()))
  }
}

#[cfg(test)]
mod js_boundary_tests {
  use super::*;

  #[test]
  fn hresult_arrays_use_the_i32_projection() {
    let array = DynWinRTArray(dynwinrt::ArrayData::from_values(
      TABLE.hresult(),
      &[dynwinrt::WinRTValue::HResult(windows::core::HRESULT(
        0x80004005u32 as i32,
      ))],
    ));

    assert_eq!(
      array.to_i32_vec().expect("HRESULT array conversion"),
      [0x80004005u32 as i32],
    );
  }

  #[test]
  fn progress_type_validation_allows_structs_and_rejects_unsupported_shapes() {
    let progress = TABLE.struct_type("Test.JsStructProgress", &[TABLE.u64_type()]);
    assert!(ensure_progress_type_supported(&progress).is_ok());

    let value_type = TABLE.u32_type();
    for unsupported in [
      TABLE.guid_type(),
      TABLE.array_of_iunknown(),
      TABLE.generic(
        windows::core::GUID::from_u128(0x11111111_2222_3333_4444_555555555555),
        1,
      ),
      TABLE.out_value(&value_type),
      TABLE.array(&value_type),
    ] {
      assert!(ensure_progress_type_supported(&unsupported).is_err());
    }
  }
}

// ======================================================================
// Struct binding — typed field access by index
// ======================================================================

#[napi]
pub struct DynWinRTStruct(dynwinrt::ValueTypeData);
unsafe impl Send for DynWinRTStruct {}
unsafe impl Sync for DynWinRTStruct {}

impl DynWinRTStruct {
  fn checked_field_index(
    &self,
    index: f64,
    method: &str,
    expected: &str,
    accepts: impl Fn(dynwinrt::TypeKind) -> bool,
  ) -> napi::Result<usize> {
    let index = js_u32(index, method)? as usize;
    let handle = self.0.type_handle();
    if index >= handle.field_count() {
      return Err(napi::Error::from_reason(format!(
        "{method}: field index {index} is out of bounds for {} fields",
        handle.field_count(),
      )));
    }
    let actual = handle.field_type(index).kind();
    if !accepts(actual) {
      return Err(napi::Error::from_reason(format!(
        "{method}: field {index} has type {actual:?}, expected {expected}",
      )));
    }
    Ok(index)
  }
}

#[napi]
impl DynWinRTStruct {
  /// Create a zero-initialized struct of the given type.
  #[napi]
  pub fn create(typ: &DynWinRTType) -> napi::Result<DynWinRTStruct> {
    if !matches!(typ.0.kind(), dynwinrt::TypeKind::Struct(_)) {
      return Err(napi::Error::from_reason(format!(
        "DynWinRtStruct.create requires a struct type, found {:?}",
        typ.0.kind(),
      )));
    }
    Ok(DynWinRTStruct(typ.0.default_value()))
  }

  #[napi]
  pub fn get_i8(&self, index: f64) -> napi::Result<i32> {
    let index =
      self.checked_field_index(index, "getI8", "i8", |kind| kind == dynwinrt::TypeKind::I8)?;
    Ok(i32::from(self.0.get_field::<i8>(index)))
  }
  #[napi]
  pub fn set_i8(&mut self, index: f64, value: f64) -> napi::Result<()> {
    let index =
      self.checked_field_index(index, "setI8", "i8", |kind| kind == dynwinrt::TypeKind::I8)?;
    let value = i8::try_from(js_i32(value, "setI8")?)
      .map_err(|_| napi::Error::from_reason("setI8: value is outside the i8 range"))?;
    self.0.set_field(index, value);
    Ok(())
  }

  #[napi]
  pub fn get_u8(&self, index: f64) -> napi::Result<u32> {
    let index = self.checked_field_index(index, "getU8", "u8 or bool", |kind| {
      matches!(kind, dynwinrt::TypeKind::U8 | dynwinrt::TypeKind::Bool)
    })?;
    Ok(u32::from(self.0.get_field::<u8>(index)))
  }
  #[napi]
  pub fn set_u8(&mut self, index: f64, value: f64) -> napi::Result<()> {
    let index = self.checked_field_index(index, "setU8", "u8 or bool", |kind| {
      matches!(kind, dynwinrt::TypeKind::U8 | dynwinrt::TypeKind::Bool)
    })?;
    let value = u8::try_from(js_u32(value, "setU8")?)
      .map_err(|_| napi::Error::from_reason("setU8: value is outside the u8 range"))?;
    self.0.set_field(index, value);
    Ok(())
  }

  #[napi]
  pub fn get_i16(&self, index: f64) -> napi::Result<i32> {
    let index = self.checked_field_index(index, "getI16", "i16", |kind| {
      kind == dynwinrt::TypeKind::I16
    })?;
    Ok(i32::from(self.0.get_field::<i16>(index)))
  }
  #[napi]
  pub fn set_i16(&mut self, index: f64, value: f64) -> napi::Result<()> {
    let index = self.checked_field_index(index, "setI16", "i16", |kind| {
      kind == dynwinrt::TypeKind::I16
    })?;
    let value = i16::try_from(js_i32(value, "setI16")?)
      .map_err(|_| napi::Error::from_reason("setI16: value is outside the i16 range"))?;
    self.0.set_field(index, value);
    Ok(())
  }

  #[napi]
  pub fn get_u16(&self, index: f64) -> napi::Result<u32> {
    let index = self.checked_field_index(index, "getU16", "u16 or char16", |kind| {
      matches!(kind, dynwinrt::TypeKind::U16 | dynwinrt::TypeKind::Char16)
    })?;
    Ok(u32::from(self.0.get_field::<u16>(index)))
  }
  #[napi]
  pub fn set_u16(&mut self, index: f64, value: f64) -> napi::Result<()> {
    let index = self.checked_field_index(index, "setU16", "u16 or char16", |kind| {
      matches!(kind, dynwinrt::TypeKind::U16 | dynwinrt::TypeKind::Char16)
    })?;
    let value = u16::try_from(js_u32(value, "setU16")?)
      .map_err(|_| napi::Error::from_reason("setU16: value is outside the u16 range"))?;
    self.0.set_field(index, value);
    Ok(())
  }

  #[napi]
  pub fn get_i32(&self, index: f64) -> napi::Result<i32> {
    let index = self.checked_field_index(index, "getI32", "i32, enum, or HRESULT", |kind| {
      matches!(
        kind,
        dynwinrt::TypeKind::I32 | dynwinrt::TypeKind::Enum(_) | dynwinrt::TypeKind::HResult
      )
    })?;
    Ok(self.0.get_field::<i32>(index))
  }
  #[napi]
  pub fn set_i32(&mut self, index: f64, value: f64) -> napi::Result<()> {
    let index = self.checked_field_index(index, "setI32", "i32, enum, or HRESULT", |kind| {
      matches!(
        kind,
        dynwinrt::TypeKind::I32 | dynwinrt::TypeKind::Enum(_) | dynwinrt::TypeKind::HResult
      )
    })?;
    self.0.set_field(index, js_i32(value, "setI32")?);
    Ok(())
  }

  #[napi]
  pub fn get_u32(&self, index: f64) -> napi::Result<u32> {
    let index = self.checked_field_index(index, "getU32", "u32", |kind| {
      kind == dynwinrt::TypeKind::U32
    })?;
    Ok(self.0.get_field::<u32>(index))
  }
  #[napi]
  pub fn set_u32(&mut self, index: f64, value: f64) -> napi::Result<()> {
    let index = self.checked_field_index(index, "setU32", "u32", |kind| {
      kind == dynwinrt::TypeKind::U32
    })?;
    self.0.set_field(index, js_u32(value, "setU32")?);
    Ok(())
  }

  #[napi]
  pub fn get_f32(&self, index: f64) -> napi::Result<f64> {
    let index = self.checked_field_index(index, "getF32", "f32", |kind| {
      kind == dynwinrt::TypeKind::F32
    })?;
    Ok(f64::from(self.0.get_field::<f32>(index)))
  }
  #[napi]
  pub fn set_f32(&mut self, index: f64, value: f64) -> napi::Result<()> {
    let index = self.checked_field_index(index, "setF32", "f32", |kind| {
      kind == dynwinrt::TypeKind::F32
    })?;
    let converted = value as f32;
    if value.is_finite() && !converted.is_finite() {
      return Err(napi::Error::from_reason(
        "setF32: finite value is outside the f32 range",
      ));
    }
    self.0.set_field(index, converted);
    Ok(())
  }

  #[napi]
  pub fn get_f64(&self, index: f64) -> napi::Result<f64> {
    let index = self.checked_field_index(index, "getF64", "f64", |kind| {
      kind == dynwinrt::TypeKind::F64
    })?;
    Ok(self.0.get_field::<f64>(index))
  }
  #[napi]
  pub fn set_f64(&mut self, index: f64, value: f64) -> napi::Result<()> {
    let index = self.checked_field_index(index, "setF64", "f64", |kind| {
      kind == dynwinrt::TypeKind::F64
    })?;
    self.0.set_field(index, value);
    Ok(())
  }

  #[napi]
  pub fn get_i64(&self, index: f64) -> napi::Result<BigInt> {
    let index = self.checked_field_index(index, "getI64", "i64", |kind| {
      kind == dynwinrt::TypeKind::I64
    })?;
    Ok(BigInt::from(self.0.get_field::<i64>(index)))
  }
  #[napi]
  pub fn set_i64(&mut self, index: f64, value: BigInt) -> napi::Result<()> {
    let index = self.checked_field_index(index, "setI64", "i64", |kind| {
      kind == dynwinrt::TypeKind::I64
    })?;
    let (n, lossless) = value.get_i64();
    if !lossless {
      return Err(napi::Error::from_reason(
        "setI64: bigint value must fit in a signed 64-bit integer",
      ));
    }
    self.0.set_field(index, n);
    Ok(())
  }

  #[napi]
  pub fn get_u64(&self, index: f64) -> napi::Result<BigInt> {
    let index = self.checked_field_index(index, "getU64", "u64", |kind| {
      kind == dynwinrt::TypeKind::U64
    })?;
    Ok(BigInt::from(self.0.get_field::<u64>(index)))
  }
  #[napi]
  pub fn set_u64(&mut self, index: f64, value: Either<BigInt, f64>) -> napi::Result<()> {
    let index = self.checked_field_index(index, "setU64", "u64", |kind| {
      kind == dynwinrt::TypeKind::U64
    })?;
    let n = js_u64(value, "setU64")?;
    self.0.set_field(index, n);
    Ok(())
  }

  // -- Non-blittable field access --

  #[napi]
  pub fn get_hstring(&self, index: f64) -> napi::Result<String> {
    let index = self.checked_field_index(index, "getHstring", "HSTRING", |kind| {
      kind == dynwinrt::TypeKind::HString
    })?;
    self
      .0
      .get_field_hstring(index)
      .map(|value| value.to_string())
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  #[napi]
  pub fn set_hstring(&mut self, index: f64, value: String) -> napi::Result<()> {
    let index = self.checked_field_index(index, "setHstring", "HSTRING", |kind| {
      kind == dynwinrt::TypeKind::HString
    })?;
    self
      .0
      .set_field_hstring(index, HSTRING::from(value))
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  #[napi]
  pub fn get_guid(&self, index: f64) -> napi::Result<WinGUID> {
    let index = self.checked_field_index(index, "getGuid", "GUID", |kind| {
      kind == dynwinrt::TypeKind::Guid
    })?;
    let guid = self.0.get_field::<windows::core::GUID>(index as usize);
    Ok(WinGUID(guid))
  }

  #[napi]
  pub fn set_guid(&mut self, index: f64, value: &WinGUID) -> napi::Result<()> {
    let index = self.checked_field_index(index, "setGuid", "GUID", |kind| {
      kind == dynwinrt::TypeKind::Guid
    })?;
    self.0.set_field(index, value.0);
    Ok(())
  }

  #[napi]
  pub fn get_struct(&self, index: f64) -> napi::Result<DynWinRTStruct> {
    let index = self.checked_field_index(index, "getStruct", "struct", |kind| {
      matches!(kind, dynwinrt::TypeKind::Struct(_))
    })?;
    Ok(DynWinRTStruct(self.0.get_field_struct(index)))
  }

  #[napi]
  pub fn set_struct(&mut self, index: f64, value: &DynWinRTStruct) -> napi::Result<()> {
    let index = self.checked_field_index(index, "setStruct", "struct", |kind| {
      matches!(kind, dynwinrt::TypeKind::Struct(_))
    })?;
    let expected = self.0.type_handle().field_type(index).kind();
    let actual = value.0.type_handle().kind();
    if expected != actual {
      return Err(napi::Error::from_reason(format!(
        "setStruct: field {index} requires {expected:?}, found {actual:?}",
      )));
    }
    self.0.set_field_struct(index, &value.0);
    Ok(())
  }

  #[napi]
  pub fn get_object(&self, index: f64) -> napi::Result<DynWinRTValue> {
    let index = self.checked_field_index(index, "getObject", "WinRT object", |kind| {
      kind.is_com_pointer()
    })?;
    match self
      .0
      .get_field_object(index)
      .map_err(|error| napi::Error::from_reason(error.message()))?
    {
      Some(object) => Ok(DynWinRTValue::new(dynwinrt::WinRTValue::Object(object))),
      None => Ok(DynWinRTValue::new(dynwinrt::WinRTValue::Null)),
    }
  }

  #[napi]
  pub fn set_object(&mut self, index: f64, value: &DynWinRTValue) -> napi::Result<()> {
    let index = self.checked_field_index(index, "setObject", "WinRT object", |kind| {
      kind.is_com_pointer()
    })?;
    match &value.0 {
      dynwinrt::WinRTValue::Object(obj) => self
        .0
        .set_field_object(index, Some(obj))
        .map_err(|error| napi::Error::from_reason(error.message())),
      dynwinrt::WinRTValue::Null => self
        .0
        .set_field_object(index, None)
        .map_err(|error| napi::Error::from_reason(error.message())),
      _ => Err(napi::Error::from_reason(
        "setObject requires a WinRT object or null value",
      )),
    }
  }

  /// Wrap as DynWinRTValue::Struct for passing to call().
  #[napi]
  pub fn to_value(&self) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::Struct(self.0.clone()))
  }
}

// ======================================================================
// System info
// ======================================================================

#[napi]
pub fn has_package_identity() -> bool {
  use windows::ApplicationModel::AppInfo;
  match AppInfo::Current() {
    Ok(_) => true,
    Err(_) => false,
  }
}

#[napi]
pub fn get_computer_name() -> napi::Result<String> {
  #[cfg(target_os = "windows")]
  {
    use windows::core::PWSTR;
    use windows::Win32::System::WindowsProgramming::GetComputerNameW;

    let mut buffer = [0u16; 256];
    let mut size = buffer.len() as u32;

    unsafe {
      if GetComputerNameW(Some(PWSTR(buffer.as_mut_ptr())), &mut size).is_ok() {
        let name = String::from_utf16_lossy(&buffer[..size as usize]);
        Ok(name)
      } else {
        Err(napi::Error::from_reason("Failed to get computer name"))
      }
    }
  }

  #[cfg(not(target_os = "windows"))]
  {
    Err(napi::Error::from_reason(
      "This function is only available on Windows",
    ))
  }
}

#[napi]
pub fn get_windows_directory() -> napi::Result<String> {
  #[cfg(target_os = "windows")]
  {
    use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;

    let mut buffer = [0u16; 260]; // MAX_PATH

    unsafe {
      let len = GetWindowsDirectoryW(Some(&mut buffer));
      if len > 0 {
        let path = String::from_utf16_lossy(&buffer[..len as usize]);
        Ok(path)
      } else {
        Err(napi::Error::from_reason("Failed to get Windows directory"))
      }
    }
  }

  #[cfg(not(target_os = "windows"))]
  {
    Err(napi::Error::from_reason(
      "This function is only available on Windows",
    ))
  }
}

// ======================================================================
// Rust static benchmark — windows crate direct projection (no dynwinrt)
// ======================================================================

#[napi]
pub struct RustStaticBench;

fn map_win_err(e: windows::core::Error) -> napi::Error {
  napi::Error::from_reason(e.message())
}

/// Pre-created Uri for static benchmark (stores typed interface, no QI on access).
#[napi]
pub struct StaticUri(windows::Foundation::Uri);
unsafe impl Send for StaticUri {}
unsafe impl Sync for StaticUri {}

/// Pre-created opaque COM object for static benchmark (factory results).
#[napi]
pub struct StaticObj(#[allow(dead_code)] windows::core::IInspectable);
unsafe impl Send for StaticObj {}
unsafe impl Sync for StaticObj {}

#[napi]
impl RustStaticBench {
  // --- Uri ---

  #[napi]
  pub fn uri_create(url: String) -> napi::Result<StaticUri> {
    let uri = windows::Foundation::Uri::CreateUri(&HSTRING::from(url)).map_err(map_win_err)?;
    Ok(StaticUri(uri))
  }

  #[napi]
  pub fn uri_get_host(url: String) -> napi::Result<String> {
    let uri = windows::Foundation::Uri::CreateUri(&HSTRING::from(url)).map_err(map_win_err)?;
    Ok(uri.Host().map_err(map_win_err)?.to_string())
  }

  #[napi]
  pub fn uri_host_from_obj(obj: &StaticUri) -> napi::Result<String> {
    Ok(obj.0.Host().map_err(map_win_err)?.to_string())
  }

  #[napi]
  pub fn uri_port_from_obj(obj: &StaticUri) -> napi::Result<i32> {
    Ok(obj.0.Port().map_err(map_win_err)?)
  }

  #[napi]
  pub fn uri_suspicious_from_obj(obj: &StaticUri) -> napi::Result<bool> {
    Ok(obj.0.Suspicious().map_err(map_win_err)?)
  }

  #[napi]
  pub fn uri_query_parsed_from_obj(obj: &StaticUri) -> napi::Result<StaticObj> {
    Ok(StaticObj(obj.0.QueryParsed().map_err(map_win_err)?.into()))
  }

  #[napi]
  pub fn uri_combine(obj: &StaticUri, relative: String) -> napi::Result<StaticUri> {
    let result = obj
      .0
      .CombineUri(&HSTRING::from(relative))
      .map_err(map_win_err)?;
    Ok(StaticUri(result))
  }

  #[napi]
  pub fn uri_create_with_relative(base: String, relative: String) -> napi::Result<StaticUri> {
    let uri = windows::Foundation::Uri::CreateWithRelativeUri(
      &HSTRING::from(base),
      &HSTRING::from(relative),
    )
    .map_err(map_win_err)?;
    Ok(StaticUri(uri))
  }

  // --- PropertyValue ---

  #[napi]
  pub fn pv_create_i32(value: i32) -> napi::Result<StaticObj> {
    Ok(StaticObj(
      windows::Foundation::PropertyValue::CreateInt32(value)
        .map_err(map_win_err)?
        .into(),
    ))
  }

  #[napi]
  pub fn pv_create_f64(value: f64) -> napi::Result<StaticObj> {
    Ok(StaticObj(
      windows::Foundation::PropertyValue::CreateDouble(value)
        .map_err(map_win_err)?
        .into(),
    ))
  }

  #[napi]
  pub fn pv_create_bool(value: bool) -> napi::Result<StaticObj> {
    Ok(StaticObj(
      windows::Foundation::PropertyValue::CreateBoolean(value)
        .map_err(map_win_err)?
        .into(),
    ))
  }

  #[napi]
  pub fn pv_create_string(value: String) -> napi::Result<StaticObj> {
    Ok(StaticObj(
      windows::Foundation::PropertyValue::CreateString(&HSTRING::from(value))
        .map_err(map_win_err)?
        .into(),
    ))
  }

  // --- Geopoint ---

  #[napi]
  pub fn geopoint_create(lat: f64, lon: f64, alt: f64) -> napi::Result<StaticObj> {
    use windows::Devices::Geolocation::{BasicGeoposition, Geopoint};
    let pos = BasicGeoposition {
      Latitude: lat,
      Longitude: lon,
      Altitude: alt,
    };
    Ok(StaticObj(
      Geopoint::Create(pos).map_err(map_win_err)?.into(),
    ))
  }
}

// ======================================================================
// DynWinRtDelegate — dynamic WinRT delegate (callback) binding
// ======================================================================

struct DirectJsCallback {
  env: napi::sys::napi_env,
  callback_ref: napi::sys::napi_ref,
  async_context: napi::sys::napi_async_context,
  lifecycle: Arc<managed_tsfn::TsfnLifecycle>,
}

unsafe impl Send for DirectJsCallback {}
unsafe impl Sync for DirectJsCallback {}

fn napi_status(name: &str, status: napi::sys::napi_status) -> napi::Result<()> {
  if status == napi::sys::Status::napi_ok {
    Ok(())
  } else {
    Err(napi::Error::from_reason(format!(
      "{name} failed with status {}",
      napi::Status::from(status),
    )))
  }
}

fn create_direct_callback_resources(
  env: napi::sys::napi_env,
  callback: napi::sys::napi_value,
  resource_name_bytes: &[u8],
) -> napi::Result<(
  napi::sys::napi_ref,
  napi::sys::napi_async_context,
  Box<managed_tsfn::TsfnFinalizer>,
)> {
  let mut callback_ref = std::ptr::null_mut();
  napi_status("napi_create_reference(callback)", unsafe {
    napi::sys::napi_create_reference(env, callback, 1, &mut callback_ref)
  })?;

  let result = (|| {
    let mut resource = std::ptr::null_mut();
    napi_status("napi_create_object(callback resource)", unsafe {
      napi::sys::napi_create_object(env, &mut resource)
    })?;

    let mut resource_ref = std::ptr::null_mut();
    napi_status("napi_create_reference(callback resource)", unsafe {
      napi::sys::napi_create_reference(env, resource, 1, &mut resource_ref)
    })?;

    let async_context = (|| {
      let mut resource_name = std::ptr::null_mut();
      napi_status("napi_create_string_utf8(callback resource)", unsafe {
        napi::sys::napi_create_string_utf8(
          env,
          resource_name_bytes.as_ptr().cast(),
          resource_name_bytes.len() as isize,
          &mut resource_name,
        )
      })?;
      let mut async_context = std::ptr::null_mut();
      napi_status("napi_async_init(callback)", unsafe {
        napi::sys::napi_async_init(env, resource, resource_name, &mut async_context)
      })?;
      Ok::<_, napi::Error>(async_context)
    })();

    let async_context = match async_context {
      Ok(async_context) => async_context,
      Err(error) => {
        unsafe {
          napi::sys::napi_delete_reference(env, resource_ref);
        }
        return Err(error);
      }
    };

    let finalizer: Box<managed_tsfn::TsfnFinalizer> = Box::new(move |env| {
      if env.is_null() {
        return;
      }
      let async_status = unsafe { napi::sys::napi_async_destroy(env, async_context) };
      if async_status != napi::sys::Status::napi_ok {
        eprintln!(
          "[dynwinrt] callback async context cleanup failed: {}",
          napi::Status::from(async_status)
        );
      }
      for reference in [callback_ref, resource_ref] {
        let status = unsafe { napi::sys::napi_delete_reference(env, reference) };
        if status != napi::sys::Status::napi_ok {
          eprintln!(
            "[dynwinrt] callback reference cleanup failed: {}",
            napi::Status::from(status)
          );
        }
      }
    });
    Ok((callback_ref, async_context, finalizer))
  })();

  if result.is_err() {
    unsafe {
      napi::sys::napi_delete_reference(env, callback_ref);
    }
  }
  result
}

fn invoke_direct_js_callback<R>(
  direct: &DirectJsCallback,
  build_args: impl FnOnce(napi::sys::napi_env) -> napi::Result<Vec<napi::sys::napi_value>>,
  parse_result: impl FnOnce(napi::sys::napi_env, napi::sys::napi_value) -> napi::Result<R>,
) -> napi::Result<R> {
  if direct.lifecycle.is_closing() {
    return Err(napi::Error::from_reason(
      "Cannot invoke a callback while the Node environment is closing",
    ));
  }

  let env = direct.env;
  unsafe {
    let mut scope: napi::sys::napi_handle_scope = std::ptr::null_mut();
    napi_status(
      "napi_open_handle_scope(callback)",
      napi::sys::napi_open_handle_scope(env, &mut scope),
    )?;

    let result = (|| {
      let mut function = std::ptr::null_mut();
      napi_status(
        "napi_get_reference_value(callback)",
        napi::sys::napi_get_reference_value(env, direct.callback_ref, &mut function),
      )?;
      let args = build_args(env)?;
      let mut receiver = std::ptr::null_mut();
      napi_status(
        "napi_get_global(callback)",
        napi::sys::napi_get_global(env, &mut receiver),
      )?;
      let mut result = std::ptr::null_mut();
      let status = napi::sys::napi_make_callback(
        env,
        direct.async_context,
        receiver,
        function,
        args.len(),
        args.as_ptr(),
        &mut result,
      );
      if status != napi::sys::Status::napi_ok {
        let mut is_pending = false;
        napi_status(
          "napi_is_exception_pending(callback)",
          napi::sys::napi_is_exception_pending(env, &mut is_pending),
        )?;
        if is_pending {
          let mut error = std::ptr::null_mut();
          napi_status(
            "napi_get_and_clear_last_exception(callback)",
            napi::sys::napi_get_and_clear_last_exception(env, &mut error),
          )?;
          napi_status(
            "napi_fatal_exception(callback)",
            napi::sys::napi_fatal_exception(env, error),
          )?;
        }
        return Err(napi::Error::from_reason(format!(
          "napi_make_callback failed with status {status}",
        )));
      }
      parse_result(env, result)
    })();

    napi_status(
      "napi_close_handle_scope(callback)",
      napi::sys::napi_close_handle_scope(env, scope),
    )?;
    result
  }
}

#[napi]
pub struct DynWinRtDelegate(dynwinrt::WinRTValue);

#[napi]
impl DynWinRtDelegate {
  /// Create a delegate COM object from a JS callback function.
  ///
  /// - `iid`: delegate interface IID
  /// - `param_types`: Invoke parameter types
  /// - `callback`: JS function called when WinRT fires the event
  #[napi(factory)]
  pub fn create(
    iid: &WinGUID,
    param_types: Vec<&DynWinRTType>,
    #[napi(ts_arg_type = "(...args: DynWinRTValue[]) => void")]
    callback: napi::bindgen_prelude::Function<'static, Vec<DynWinRTValue>, ()>,
  ) -> napi::Result<DynWinRtDelegate> {
    use napi::bindgen_prelude::ToNapiValue;
    use napi::JsValue;
    use windows::Win32::System::Threading::GetCurrentThreadId;

    // Track the thread we were registered on. WinRT delegate callbacks that
    // fire on this same thread are dispatched synchronously (see closure below),
    // which is required when the JS thread is running a WinUI/DispatcherQueue
    // message pump: in that state libuv is starved and the TSFN uv_async_send
    // path never wakes up. Any other thread falls back to the TSFN.
    //
    let register_tid = unsafe { GetCurrentThreadId() };
    let raw_env = callback.value().env;
    let raw_callback = napi::JsValue::raw(&callback);
    let (callback_ref, async_context, finalizer) =
      create_direct_callback_resources(raw_env, raw_callback, b"dynwinrt.delegate")?;
    let tsfn = managed_tsfn::ManagedTsfn::create(
      raw_env,
      raw_callback,
      1024,
      false,
      |values: Vec<DynWinRTValue>, env| {
        values
          .into_iter()
          .map(|value| unsafe { DynWinRTValue::to_napi_value(env, value) })
          .collect()
      },
      Some(finalizer),
    )?;
    let lifecycle = tsfn.lifecycle();
    let direct = Arc::new(DirectJsCallback {
      env: raw_env,
      callback_ref,
      async_context,
      lifecycle,
    });

    let type_handles: Vec<dynwinrt::TypeHandle> = param_types.iter().map(|t| t.0.clone()).collect();

    let delegate_callback: dynwinrt::delegate::DelegateCallback =
      Box::new(move |args: &[dynwinrt::WinRTValue]| {
        // Well-known HRESULTs used below to signal failure to the WinRT event
        // source (rather than silently returning S_OK, which would look like
        // the delegate ran).
        const E_FAIL: windows::core::HRESULT = windows::core::HRESULT(0x80004005u32 as i32);
        const E_UNEXPECTED: windows::core::HRESULT = windows::core::HRESULT(0x8000FFFFu32 as i32);

        let current_tid = unsafe { GetCurrentThreadId() };
        let js_args: Vec<DynWinRTValue> =
          args.iter().map(|a| DynWinRTValue::new(a.clone())).collect();

        if current_tid == register_tid {
          // Same-thread synchronous direct invocation. Bypass the TSFN because
          // libuv may be blocked (e.g. DispatcherQueue.runEventLoop), so
          // uv_async_send would queue the callback but never fire it.
          //
          // The entire body is wrapped in `catch_unwind`: this closure is
          // ultimately called by an `extern "system"` COM stub, and letting a
          // Rust panic unwind through the FFI boundary is UB. On panic we
          // convert to E_UNEXPECTED so the WinRT caller sees a clean failure.
          let unwind_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || -> windows::core::HRESULT {
              let call_result = invoke_direct_js_callback(
                &direct,
                |env| {
                  js_args
                    .into_iter()
                    .map(|value| unsafe { DynWinRTValue::to_napi_value(env, value) })
                    .collect()
                },
                |_env, _result| Ok(()),
              );
              if let Err(error) = call_result {
                eprintln!("[dynwinrt] delegate dispatch error: {error}");
                return E_FAIL;
              }
              windows::core::HRESULT(0)
            },
          ));
          return match unwind_result {
            Ok(hr) => hr,
            Err(_) => {
              eprintln!("[dynwinrt] delegate: panic caught at FFI boundary");
              E_UNEXPECTED
            }
          };
        }

        // Cross-thread fallback: schedule via the TSFN. This requires libuv to
        // be pumping on the JS thread, which is fine for classic Node.js work
        // but not for a JS thread stuck inside a foreign message pump.
        let status = tsfn.call(js_args);
        if status == napi::Status::Ok {
          windows::core::HRESULT(0)
        } else {
          if status != napi::Status::QueueFull {
            eprintln!("[dynwinrt] delegate callback queue failed: {status}");
          }
          E_FAIL
        }
      });

    let value =
      dynwinrt::delegate::try_create_delegate_value(iid.0, type_handles, delegate_callback)
        .map_err(|error| {
          napi::Error::from_reason(format!("DynWinRtDelegate.create: {}", error.message()))
        })?;
    Ok(DynWinRtDelegate(value))
  }

  /// Get the delegate as a DynWinRtValue for passing to WinRT methods.
  #[napi]
  pub fn to_value(&self) -> DynWinRTValue {
    DynWinRTValue::new(self.0.clone())
  }
}

// ======================================================================
// DynWinRtElementFactory — synchronous WinUI IElementFactory binding
// ======================================================================

type ElementFactoryGetFunction =
  napi::bindgen_prelude::Function<'static, DynWinRTValue, DynWinRTValue>;
type ElementFactoryRecycleFunction = napi::bindgen_prelude::Function<'static, DynWinRTValue, ()>;

struct ElementFactoryCallbackRefs {
  get_element: Option<Arc<napi::bindgen_prelude::FunctionRef<DynWinRTValue, DynWinRTValue>>>,
  recycle_element: Option<Arc<napi::bindgen_prelude::FunctionRef<DynWinRTValue, ()>>>,
}

#[napi]
pub struct DynWinRtElementFactory {
  value: dynwinrt::WinRTValue,
  callbacks: Arc<Mutex<ElementFactoryCallbackRefs>>,
}

unsafe fn take_pending_exception_message(env: napi::sys::napi_env) -> Option<String> {
  let mut pending = false;
  if napi::sys::napi_is_exception_pending(env, &mut pending) != napi::sys::Status::napi_ok
    || !pending
  {
    return None;
  }

  let mut exception = std::ptr::null_mut();
  if napi::sys::napi_get_and_clear_last_exception(env, &mut exception) != napi::sys::Status::napi_ok
  {
    return None;
  }
  let mut text = std::ptr::null_mut();
  if napi::sys::napi_coerce_to_string(env, exception, &mut text) != napi::sys::Status::napi_ok {
    return None;
  }

  let mut length = 0usize;
  if napi::sys::napi_get_value_string_utf8(env, text, std::ptr::null_mut(), 0, &mut length)
    != napi::sys::Status::napi_ok
  {
    return None;
  }
  let mut buffer = vec![0u8; length + 1];
  let mut written = 0usize;
  if napi::sys::napi_get_value_string_utf8(
    env,
    text,
    buffer.as_mut_ptr().cast(),
    buffer.len(),
    &mut written,
  ) != napi::sys::Status::napi_ok
  {
    return None;
  }
  Some(String::from_utf8_lossy(&buffer[..written]).into_owned())
}

#[napi]
impl DynWinRtElementFactory {
  #[napi(factory)]
  pub fn create(
    element_iid: &WinGUID,
    #[napi(ts_arg_type = "(args: DynWinRtValue) => DynWinRtValue")]
    get_element: ElementFactoryGetFunction,
    #[napi(ts_arg_type = "(args: DynWinRtValue) => void")]
    recycle_element: ElementFactoryRecycleFunction,
  ) -> napi::Result<DynWinRtElementFactory> {
    use napi::bindgen_prelude::{FromNapiValue, ToNapiValue};
    use napi::JsValue;
    use windows::Win32::System::Threading::GetCurrentThreadId;

    const E_FAIL: windows::core::HRESULT = windows::core::HRESULT(0x80004005u32 as i32);
    const E_UNEXPECTED: windows::core::HRESULT = windows::core::HRESULT(0x8000FFFFu32 as i32);
    const RPC_E_WRONG_THREAD: windows::core::HRESULT = windows::core::HRESULT(0x8001010Eu32 as i32);
    const RO_E_CLOSED: windows::core::HRESULT = windows::core::HRESULT(0x80000013u32 as i32);

    struct SendableEnv(napi::sys::napi_env);
    unsafe impl Send for SendableEnv {}
    unsafe impl Sync for SendableEnv {}

    let register_tid = unsafe { GetCurrentThreadId() };
    let element_iid = element_iid.0;
    let raw_env = Arc::new(SendableEnv(get_element.value().env));
    let callbacks = Arc::new(Mutex::new(ElementFactoryCallbackRefs {
      get_element: Some(Arc::new(get_element.create_ref()?)),
      recycle_element: Some(Arc::new(recycle_element.create_ref()?)),
    }));

    let get_env = raw_env.clone();
    let get_callbacks = callbacks.clone();
    let get_callback: dynwinrt::ElementFactoryGetCallback = Box::new(move |args| {
      if unsafe { GetCurrentThreadId() } != register_tid {
        return Err(RPC_E_WRONG_THREAD);
      }

      let get_ref = match get_callbacks.lock() {
        Ok(callbacks) => {
          let Some(callback) = callbacks.get_element.as_ref() else {
            return Err(RO_E_CLOSED);
          };
          callback.clone()
        }
        Err(_) => return Err(E_FAIL),
      };
      let raw_env = get_env.0;
      let js_arg = DynWinRTValue::new(args.clone());
      let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> napi::Result<dynwinrt::WinRTValue> {
          unsafe {
            let mut scope = std::ptr::null_mut();
            if napi::sys::napi_open_handle_scope(raw_env, &mut scope) != napi::sys::Status::napi_ok
            {
              return Err(napi::Error::from_reason("napi_open_handle_scope failed"));
            }

            let call_result = (|| -> napi::Result<dynwinrt::WinRTValue> {
              let env = napi::Env::from_raw(raw_env);
              let function = get_ref.borrow_back(&env)?;
              let function_value = napi::JsValue::raw(&function);
              let argument = DynWinRTValue::to_napi_value(raw_env, js_arg)?;
              let mut receiver = std::ptr::null_mut();
              napi::sys::napi_get_global(raw_env, &mut receiver);
              let mut raw_result = std::ptr::null_mut();
              let status = napi::sys::napi_make_callback(
                raw_env,
                std::ptr::null_mut(),
                receiver,
                function_value,
                1,
                &argument,
                &mut raw_result,
              );
              if status != napi::sys::Status::napi_ok {
                let detail = take_pending_exception_message(raw_env)
                  .unwrap_or_else(|| "unknown JavaScript exception".into());
                return Err(napi::Error::from_reason(format!(
                  "IElementFactory getElement callback failed: {detail}"
                )));
              }
              <&DynWinRTValue>::from_napi_value(raw_env, raw_result)?
                .0
                .cast(&element_iid)
                .map_err(|error| napi::Error::from_reason(error.message()))
            })();
            let call_result =
              call_result.map_err(|error| match take_pending_exception_message(raw_env) {
                Some(detail) => napi::Error::from_reason(format!("{error}: {detail}")),
                None => error,
              });
            napi::sys::napi_close_handle_scope(raw_env, scope);
            call_result
          }
        },
      ));

      match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => {
          eprintln!("[dynwinrt] IElementFactory getElement dispatch error: {error}");
          Err(E_FAIL)
        }
        Err(_) => Err(E_UNEXPECTED),
      }
    });

    let recycle_env = raw_env.clone();
    let recycle_callbacks = callbacks.clone();
    let recycle_callback: dynwinrt::ElementFactoryRecycleCallback = Box::new(move |args| {
      if unsafe { GetCurrentThreadId() } != register_tid {
        return RPC_E_WRONG_THREAD;
      }

      let recycle_ref = match recycle_callbacks.lock() {
        Ok(callbacks) => {
          let Some(callback) = callbacks.recycle_element.as_ref() else {
            return RO_E_CLOSED;
          };
          callback.clone()
        }
        Err(_) => return E_FAIL,
      };
      let raw_env = recycle_env.0;
      let js_arg = DynWinRTValue::new(args.clone());
      let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> napi::Result<()> {
          unsafe {
            let mut scope = std::ptr::null_mut();
            if napi::sys::napi_open_handle_scope(raw_env, &mut scope) != napi::sys::Status::napi_ok
            {
              return Err(napi::Error::from_reason("napi_open_handle_scope failed"));
            }

            let call_result = (|| -> napi::Result<()> {
              let env = napi::Env::from_raw(raw_env);
              let function = recycle_ref.borrow_back(&env)?;
              let function_value = napi::JsValue::raw(&function);
              let argument = DynWinRTValue::to_napi_value(raw_env, js_arg)?;
              let mut receiver = std::ptr::null_mut();
              napi::sys::napi_get_global(raw_env, &mut receiver);
              let mut raw_result = std::ptr::null_mut();
              let status = napi::sys::napi_make_callback(
                raw_env,
                std::ptr::null_mut(),
                receiver,
                function_value,
                1,
                &argument,
                &mut raw_result,
              );
              if status != napi::sys::Status::napi_ok {
                let detail = take_pending_exception_message(raw_env)
                  .unwrap_or_else(|| "unknown JavaScript exception".into());
                return Err(napi::Error::from_reason(format!(
                  "IElementFactory recycleElement callback failed: {detail}"
                )));
              }
              Ok(())
            })();
            let call_result =
              call_result.map_err(|error| match take_pending_exception_message(raw_env) {
                Some(detail) => napi::Error::from_reason(format!("{error}: {detail}")),
                None => error,
              });
            napi::sys::napi_close_handle_scope(raw_env, scope);
            call_result
          }
        }));

      match result {
        Ok(Ok(())) => windows::core::HRESULT(0),
        Ok(Err(error)) => {
          eprintln!("[dynwinrt] IElementFactory recycleElement dispatch error: {error}");
          E_FAIL
        }
        Err(_) => E_UNEXPECTED,
      }
    });

    Ok(DynWinRtElementFactory {
      value: dynwinrt::create_element_factory_value(get_callback, recycle_callback),
      callbacks,
    })
  }

  #[napi]
  pub fn to_value(&self) -> DynWinRTValue {
    DynWinRTValue::new(self.value.clone())
  }

  #[napi]
  pub fn release_callbacks(&self) -> napi::Result<()> {
    let mut callbacks = self
      .callbacks
      .lock()
      .map_err(|_| napi::Error::from_reason("IElementFactory callback state is poisoned"))?;
    callbacks.get_element = None;
    callbacks.recycle_element = None;
    Ok(())
  }
}

// ======================================================================
// Raw N-API fast getters — bypass napi-rs macro layer entirely
// ======================================================================
//
// These use napi_sys to unwrap napi-rs managed objects directly,
// call dynwinrt's zero-alloc getter path, and return JS primitives.
// Registered as standalone functions: rawGetString(method, obj) → string

// Standalone #[napi] functions — same zero-alloc getter path as methods,
// but as free functions for benchmark comparison.
// napi-rs overhead here: unwrap 2 class refs + return primitive.

/// rawGetString(methodHandle, objValue) → string
#[napi]
pub fn raw_get_string(method: &DynWinRTMethodHandle, obj: &DynWinRTValue) -> napi::Result<String> {
  let raw = match &obj.0 {
    dynwinrt::WinRTValue::Object(o) => o.as_raw(),
    _ => return Err(napi::Error::from_reason("not an Object")),
  };
  Ok(
    method
      .0
      .call_getter_hstring(raw)
      .map_err(|e| napi::Error::from_reason(e.message()))?
      .to_string(),
  )
}

/// rawGetI32(methodHandle, objValue) → number
#[napi]
pub fn raw_get_i32(method: &DynWinRTMethodHandle, obj: &DynWinRTValue) -> napi::Result<i32> {
  let raw = match &obj.0 {
    dynwinrt::WinRTValue::Object(o) => o.as_raw(),
    _ => return Err(napi::Error::from_reason("not an Object")),
  };
  method
    .0
    .call_getter_i32(raw)
    .map_err(|e| napi::Error::from_reason(e.message()))
}
