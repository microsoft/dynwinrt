// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#![deny(clippy::all)]
#![allow(clippy::missing_safety_doc)]

use std::sync::{Arc, Mutex, OnceLock};

use dynwinrt;
use napi::bindgen_prelude::{BigInt, Either};
use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use napi::JsValue;
use napi_derive::napi;
use windows::core::{IUnknown, Interface, HSTRING};

mod com;
pub use com::{DynCom, DynComInterface, DynComMethodHandle, DynComMethodSig, DynComType};

/// Shared MetadataTable — created once, used everywhere.
static TABLE: std::sync::LazyLock<Arc<dynwinrt::MetadataTable>> =
  std::sync::LazyLock::new(|| dynwinrt::MetadataTable::new());

// ======================================================================
// Runtime initialization
// ======================================================================

struct InitializedWinAppSdk {
  major: u32,
  minor: u32,
  context: dynwinrt::WinAppSdkContext,
}

static WINAPP_SDK: OnceLock<InitializedWinAppSdk> = OnceLock::new();

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

// ======================================================================
// Core types — DynWinRTType, DynWinRTMethodSig, DynWinRTMethodHandle, WinGUID
// ======================================================================

#[napi]
pub struct DynWinRTType(dynwinrt::TypeHandle);

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
  pub fn runtime_class(name: String, default_iid: &WinGUID) -> Self {
    DynWinRTType(TABLE.runtime_class(name, default_iid.0))
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
        ))
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
        ))
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
pub struct DynWinRTValue(dynwinrt::WinRTValue, Option<com::NativePointerOwner>);
unsafe impl Send for DynWinRTValue {}
unsafe impl Sync for DynWinRTValue {}

impl DynWinRTValue {
  fn new(value: dynwinrt::WinRTValue) -> Self {
    Self(value, None)
  }

  fn with_pointer_owner(value: dynwinrt::WinRTValue, owner: com::NativePointerOwner) -> Self {
    Self(value, Some(owner))
  }
}

#[napi]
impl DynWinRTValue {
  #[napi]
  pub fn release(&mut self) {
    self.0 = dynwinrt::WinRTValue::Null;
    self.1 = None;
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

  /// Wrap a pointer/handle (BigInt, number, Buffer, Uint8Array, or null) as a
  /// `WinRTValue::RawPtr` for classic-COM and flat-Win32 (`flatInvoke`) calls
  /// with `void*` / HWND / PWSTR / handle / function-pointer parameters.
  ///
  /// Accepts:
  ///   - BigInt: interpreted as a raw pointer value (u64 on x64).
  ///   - number: a non-negative safe integer, interpreted as a raw pointer
  ///     value (use a BigInt for pointers above `Number.MAX_SAFE_INTEGER`).
  ///   - Buffer: uses the buffer's byte-pointer directly (does not clone).
  ///     Caller keeps the Buffer alive for the duration of the COM call.
  ///   - Uint8Array: same as Buffer — uses the view's data pointer directly.
  ///   - null/undefined: null pointer.
  #[napi]
  pub fn pointer(
    #[napi(
      ts_arg_type = "bigint | number | Buffer | Uint8Array | null | undefined"
    )]
    value: napi::bindgen_prelude::Unknown,
  ) -> napi::Result<DynWinRTValue> {
    use napi::bindgen_prelude::FromNapiValue;
    use napi::sys;

    let raw_env = value.value().env;
    let raw_val = value.value().value;

    // Fast path 1: null / undefined → null pointer
    let mut val_type = sys::ValueType::napi_undefined;
    unsafe { sys::napi_typeof(raw_env, raw_val, &mut val_type) };
    if val_type == sys::ValueType::napi_null || val_type == sys::ValueType::napi_undefined {
      return Ok(DynWinRTValue::new(dynwinrt::WinRTValue::RawPtr(
        std::ptr::null_mut(),
      )));
    }

    // Fast path 2: BigInt → parse as u64 pointer bits.
    //
    // BigInt::get_u64() returns (sign_bit, magnitude, lossless). The tuple
    // silently swallows negative values (sign=true is dropped) and values
    // that don't fit in u64 (lossless=false → magnitude wraps). Validate
    // both so that DynWinRtValue.pointer(-1n) or a >2^64 bigint produce a
    // clean error instead of a fabricated pointer.
    if val_type == sys::ValueType::napi_bigint {
      let bi = unsafe { napi::bindgen_prelude::BigInt::from_napi_value(raw_env, raw_val) }?;
      let (sign_bit, n, lossless) = bi.get_u64();
      if sign_bit {
        return Err(napi::Error::from_reason(
          "pointer(): bigint must be non-negative (pointer values are unsigned)",
        ));
      }
      if !lossless {
        return Err(napi::Error::from_reason(
          "pointer(): bigint exceeds u64 range; pointer values must fit in u64",
        ));
      }
      if (n as usize as u64) != n {
        return Err(napi::Error::from_reason(
          "pointer(): bigint exceeds usize range on this platform",
        ));
      }
      return Ok(DynWinRTValue::new(dynwinrt::WinRTValue::RawPtr(
        n as usize as *mut std::ffi::c_void,
      )));
    }

    // Fast path 3: Number → cast to usize (handy for HWNDs that fit in a
    // JS number; the caller can also pass BigInt for safety).
    //
    // A float→int cast in Rust saturates and silently accepts NaN, negative,
    // fractional, and >2^53 values — any of which could produce a bogus
    // pointer. Validate that the value is a finite, non-negative safe
    // integer that fits in usize, and require BigInt otherwise.
    if val_type == sys::ValueType::napi_number {
      let mut d: f64 = 0.0;
      unsafe { sys::napi_get_value_double(raw_env, raw_val, &mut d) };
      if !d.is_finite() {
        return Err(napi::Error::from_reason(
          "pointer(): number must be finite (got NaN or Infinity); use bigint for arbitrary pointer values",
        ));
      }
      if d < 0.0 {
        return Err(napi::Error::from_reason(
          "pointer(): number must be non-negative; use bigint for arbitrary pointer values",
        ));
      }
      if d.fract() != 0.0 {
        return Err(napi::Error::from_reason(
          "pointer(): number must be an integer; use bigint for arbitrary pointer values",
        ));
      }
      // JS Number can only faithfully represent integers up to 2^53 - 1.
      const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0; // (1 << 53) - 1
      if d > MAX_SAFE_INTEGER {
        return Err(napi::Error::from_reason(
          "pointer(): number exceeds Number.MAX_SAFE_INTEGER; use bigint for arbitrary pointer values",
        ));
      }
      let bits = d as u64;
      if (bits as usize as u64) != bits {
        return Err(napi::Error::from_reason(
          "pointer(): number exceeds usize range on this platform; use bigint",
        ));
      }
      return Ok(DynWinRTValue::new(dynwinrt::WinRTValue::RawPtr(
        bits as usize as *mut std::ffi::c_void,
      )));
    }

    // Fast path 4: Buffer / Uint8Array → base data pointer.
    if let Ok(buf) = unsafe { napi::bindgen_prelude::Buffer::from_napi_value(raw_env, raw_val) } {
      let slice: &[u8] = buf.as_ref();
      return Ok(DynWinRTValue::new(dynwinrt::WinRTValue::RawPtr(
        slice.as_ptr() as *mut std::ffi::c_void
      )));
    }

    // Fast path 4b: plain Uint8Array (NOT a Node.js Buffer subclass) →
    // base data pointer. Buffer::from_napi_value above rejects raw
    // Uint8Array views even though the TS surface (`ts_arg_type`) advertises
    // Uint8Array. Handle it explicitly with the same semantics as Buffer.
    if let Ok(arr) = unsafe { napi::bindgen_prelude::Uint8Array::from_napi_value(raw_env, raw_val) }
    {
      let slice: &[u8] = arr.as_ref();
      return Ok(DynWinRTValue::new(dynwinrt::WinRTValue::RawPtr(
        slice.as_ptr() as *mut std::ffi::c_void
      )));
    }

    // Fast path 5: existing DynWinRtValue → reject. Borrowing an Object's raw
    // COM pointer here makes it indistinguishable from an owned raw pointer to
    // adoptComPointer(), which can double-release the original wrapper's COM
    // object. Callers that already have raw pointer bits should pass those bits.
    if let Ok(v) = unsafe { <&DynWinRTValue>::from_napi_value(raw_env, raw_val) } {
      let kind = v.0.get_type_kind();
      return Err(napi::Error::from_reason(format!(
        "pointer(): DynWinRtValue inputs are not accepted (got {:?}); pass raw pointer bits, Buffer/Uint8Array, or null instead",
        kind
      )));
    }

    Err(napi::Error::from_reason(
      "pointer(): expected bigint, number, Buffer, Uint8Array, null, or undefined",
    ))
  }

  /// Get the underlying pointer of an Object/RawPtr value as a BigInt.
  /// Useful for turning a `flatInvoke` pointer result (e.g. HWND from
  /// `GetConsoleWindow`) into a bigint you can then feed into other calls.
  #[napi]
  pub fn as_pointer_bigint(&self) -> napi::Result<BigInt> {
    let bits: usize = match &self.0 {
      dynwinrt::WinRTValue::Object(o) => o.as_raw() as usize,
      dynwinrt::WinRTValue::RawPtr(p) => *p as usize,
      dynwinrt::WinRTValue::Null => 0,
      _ => {
        return Err(napi::Error::from_reason(format!(
          "asPointerBigint: not a pointer/object value ({:?})",
          self.0.get_type_kind()
        )));
      }
    };
    Ok(BigInt::from(bits as u64))
  }

  /// Decode an I64 value as a JS BigInt without truncating through Number.
  #[napi(js_name = "toI64BigInt")]
  pub fn to_i64_bigint(&self) -> napi::Result<BigInt> {
    match &self.0 {
      dynwinrt::WinRTValue::I64(v) => Ok(BigInt::from(*v)),
      _ => Err(napi::Error::from_reason(format!(
        "toI64BigInt: not an I64 value ({:?})",
        self.0.get_type_kind()
      ))),
    }
  }

  /// Decode a U64 value as a JS BigInt without truncating through Number.
  #[napi(js_name = "toU64BigInt")]
  pub fn to_u64_bigint(&self) -> napi::Result<BigInt> {
    match &self.0 {
      dynwinrt::WinRTValue::U64(v) => Ok(BigInt::from(*v)),
      _ => Err(napi::Error::from_reason(format!(
        "toU64BigInt: not a U64 value ({:?})",
        self.0.get_type_kind()
      ))),
    }
  }

  /// Invoke a flat Win32 export via `LoadLibraryW` + `GetProcAddress` + libffi.
  /// `retKind` selects the return marshalling:
  /// `'Void' | 'I32' | 'U32' | 'I64' | 'U64' | 'F32' | 'F64' | 'Ptr'`.
  ///
  /// `args` may contain: `DynWinRtValue.i32(...)`, `DynWinRtValue.u32(...)`,
  /// `DynWinRtValue.i64(...)`, `DynWinRtValue.u64(...)`,
  /// `DynWinRtValue.f32(...)`, `DynWinRtValue.f64(...)`, or
  /// `DynWinRtValue.pointer(...)`. Other kinds cause a runtime error.
  ///
  /// ## ABI / signature safety (IMPORTANT)
  ///
  /// This performs a raw libffi call using ONLY the `retKind` and the runtime
  /// kinds of the `args` you pass — it has no knowledge of the target export's
  /// real signature. Passing the wrong argument COUNT, the wrong argument ABI
  /// kinds, or the wrong `retKind` for the actual export produces an ABI
  /// mismatch that libffi cannot detect: it can read/write the wrong registers
  /// or stack slots, crash the Node process, or corrupt memory. There is no
  /// safety net here.
  ///
  /// Prefer the generated `dynwinrt-codegen --lang js` wrappers, which encode
  /// the exact parameter/return ABI taken from the winmd for each export. Only
  /// call `flatInvoke` directly if you have independently verified the target's
  /// signature and are marshalling every argument and the return to match it.
  ///
  /// ## DLL loading (SECURITY)
  ///
  /// This ultimately calls `LoadLibraryW`, which uses the default DLL
  /// search order. That means an untrusted DLL name (or a bare short
  /// name where a same-named DLL exists in the process's working
  /// directory / PATH earlier than the intended system location) can
  /// silently resolve to an attacker-controlled binary — the classic
  /// "DLL preloading / hijacking" attack. Pass DLLs that are either:
  ///
  ///   - Well-known system DLLs whose search-order first hit is under
  ///     `System32` (e.g. `'kernel32.dll'`, `'user32.dll'`,
  ///     `'ADVAPI32.dll'`) — safe on standard Windows installs
  ///     provided the app itself has not tampered with the search path.
  ///   - Or a fully qualified absolute path (`C:\\Path\\To\\my.dll`)
  ///     that you control and have integrity-checked.
  ///
  /// Do NOT accept the DLL name from untrusted input. The generated
  /// `--lang js` wrappers emitted by `dynwinrt-codegen` always pass a
  /// hard-coded DLL name matched to a specific export in the winmd.
  ///
  /// ## Buffer lifetimes (IMPORTANT)
  ///
  /// `DynWinRtValue.pointer(Buffer | Uint8Array)` intentionally stores
  /// only the raw pointer bits (`slice.as_ptr()`) — it does NOT retain
  /// the underlying JS Buffer/typed array, so the array is eligible for
  /// GC the moment the last JS reference to it drops. If you inline a
  /// buffer allocation into the argument list, e.g.
  /// `pointer(Buffer.alloc(32))` or `pointer(_wideStringBuffer(x))`,
  /// the temporary buffer becomes unreachable the moment `pointer(...)`
  /// returns, and can be reclaimed BEFORE `flatInvoke` reaches the
  /// native call — passing a dangling pointer to the Win32 export.
  ///
  /// Always keep the original buffer alive in a named local until
  /// `flatInvoke` returns:
  ///
  /// ```js
  /// // BAD — temporary buffer may be GC'd before flatInvoke runs.
  /// DynWinRtValue.flatInvoke(dll, entry, 'I32',
  ///     [DynWinRtValue.pointer(Buffer.alloc(32))]);
  ///
  /// // GOOD — buf remains reachable through the function's scope.
  /// const buf = Buffer.alloc(32);
  /// DynWinRtValue.flatInvoke(dll, entry, 'I32',
  ///     [DynWinRtValue.pointer(buf)]);
  /// ```
  ///
  /// The codegen output emitted by `dynwinrt-codegen --lang js` follows
  /// this rule: every wide/narrow string wrapper and every out-slot
  /// `Buffer.alloc` is hoisted to a named `const` before the
  /// `flatInvoke` call. Hand-written callers must do the same.
  ///
  /// ## DLL residency and `'Ptr'` returns
  ///
  /// Each distinct DLL is loaded once with `LoadLibraryW` and cached for
  /// the lifetime of the process; it is intentionally never `FreeLibrary`'d
  /// (see `flat_call::flat_invoke`). A `retKind: 'Ptr'` result (a raw
  /// pointer / function pointer / handle) that points INTO a loaded module
  /// therefore stays valid after the call returns, because the module is
  /// never unloaded. `LoadLibraryW` uses the default DLL search order, so
  /// pass a trusted or fully qualified DLL path to avoid DLL
  /// preloading/hijacking risks.
  #[napi]
  pub fn flat_invoke(
    dll: String,
    entry: String,
    ret_kind: String,
    args: Vec<&DynWinRTValue>,
  ) -> napi::Result<DynWinRTValue> {
    let ret = match ret_kind.to_ascii_lowercase().as_str() {
      "void" => dynwinrt::flat_call::FlatReturnKind::Void,
      "i32" => dynwinrt::flat_call::FlatReturnKind::I32,
      "u32" => dynwinrt::flat_call::FlatReturnKind::U32,
      "i64" => dynwinrt::flat_call::FlatReturnKind::I64,
      "u64" => dynwinrt::flat_call::FlatReturnKind::U64,
      "f32" => dynwinrt::flat_call::FlatReturnKind::F32,
      "f64" => dynwinrt::flat_call::FlatReturnKind::F64,
      "ptr" | "pointer" => dynwinrt::flat_call::FlatReturnKind::Ptr,
      other => {
        return Err(napi::Error::from_reason(format!(
          "flatInvoke: unsupported return kind '{}' (expected 'Void', 'I32', 'U32', 'I64', 'U64', 'F32', 'F64', or 'Ptr')",
          other
        )));
      }
    };
    let wrt_args: Vec<dynwinrt::WinRTValue> = args.iter().map(|a| a.0.clone()).collect();
    let result = unsafe { dynwinrt::flat_call::flat_invoke(&dll, &entry, ret, &wrt_args) }
      .map_err(|e| {
        napi::Error::from_reason(format!("flatInvoke({}!{}): {}", dll, entry, e.message()))
      })?;
    Ok(DynWinRTValue::new(result))
  }

  /// Return `GetLastError()` as a u32. Companion to `flatInvoke` for functions
  /// that use the SetLastError model (e.g. `GetModuleHandleW`).
  #[napi]
  pub fn flat_last_error() -> u32 {
    dynwinrt::flat_call::get_last_error()
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
  pub fn i64(value: i64) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::I64(value))
  }
  #[napi]
  pub fn u64(
    #[napi(ts_arg_type = "number | bigint")] value: Either<BigInt, f64>,
  ) -> napi::Result<DynWinRTValue> {
    // Accept either a JS `bigint` (full unsigned-64 range) or a plain `number`
    // (the common case — WinRT/collection codegen passes numeric sizes/positions
    // without a BigInt wrapper). The bigint path is lossless; the number path is
    // validated as a non-negative safe integer so an out-of-range or fractional
    // number is rejected rather than silently rounded/truncated into a wrong u64.
    let v: u64 = match value {
      Either::A(bi) => {
        let (negative, value, lossless) = bi.get_u64();
        if negative || !lossless {
          return Err(napi::Error::from_reason(
            "DynWinRtValue.u64(): value must fit in an unsigned 64-bit integer",
          ));
        }
        value
      }
      Either::B(n) => {
        if !n.is_finite() || n < 0.0 || n.fract() != 0.0 || n > 9_007_199_254_740_991.0 {
          return Err(napi::Error::from_reason(
            "DynWinRtValue.u64(): number must be a non-negative safe integer (use a bigint for values above 2^53-1)",
          ));
        }
        n as u64
      }
    };
    Ok(DynWinRTValue::new(dynwinrt::WinRTValue::U64(v)))
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
  pub async fn to_promise(&self) -> napi::Result<DynWinRTValue> {
    let v = (&self.0).await.map_err(|e| match e {
      dynwinrt::Error::Canceled => napi::Error::from_reason("Async operation was canceled"),
      other => napi::Error::from_reason(format!("Async operation failed: {}", other.message())),
    })?;
    Ok(DynWinRTValue::new(v))
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

    let handler_iid = async_info
      .progress_handler_iid()
      .ok_or_else(|| napi::Error::from_reason("onProgress: cannot compute progress handler IID"))?;

    // Progress callbacks must not keep an otherwise idle Node process alive.
    let tsfn = callback
      .build_threadsafe_function()
      .weak::<true>()
      .build()?;
    let progress_cb: dynwinrt::ProgressCallback = Box::new(move |val: dynwinrt::WinRTValue| {
      tsfn.call(
        DynWinRTValue::new(val),
        ThreadsafeFunctionCallMode::NonBlocking,
      );
    });
    let handler = dynwinrt::create_progress_handler(handler_iid, progress_type, progress_cb);

    async_info
      .set_progress_handler(&handler)
      .map_err(|e| napi::Error::from_reason(format!("SetProgress failed: {}", e.message())))?;

    Ok(())
  }

  #[napi]
  pub fn to_string(&self) -> String {
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
    let result = self
      .0
      .cast(&iid.0)
      .map_err(|e| napi::Error::from_reason(format!("QueryInterface failed: {}", e.message())))?;
    Ok(DynWinRTValue::new(result))
  }

  #[napi]
  pub fn to_number(&self) -> i32 {
    match &self.0 {
      dynwinrt::WinRTValue::Bool(b) => {
        if *b {
          1
        } else {
          0
        }
      }
      dynwinrt::WinRTValue::I8(i) => *i as i32,
      dynwinrt::WinRTValue::U8(i) => *i as i32,
      dynwinrt::WinRTValue::I16(i) => *i as i32,
      dynwinrt::WinRTValue::U16(i) => *i as i32,
      dynwinrt::WinRTValue::I32(i) => *i,
      dynwinrt::WinRTValue::U32(i) => *i as i32,
      dynwinrt::WinRTValue::HResult(hr) => hr.0,
      dynwinrt::WinRTValue::Enum { value, .. } => *value,
      _ => panic!("Cannot convert {:?} to number", self.0.get_type_kind()),
    }
  }

  #[napi]
  pub fn to_bool(&self) -> bool {
    match &self.0 {
      dynwinrt::WinRTValue::Bool(b) => *b,
      _ => self.to_number() != 0,
    }
  }

  #[napi]
  pub fn to_i64(&self) -> i64 {
    match &self.0 {
      dynwinrt::WinRTValue::I64(i) => *i,
      dynwinrt::WinRTValue::U64(i) => *i as i64,
      _ => self.to_number() as i64,
    }
  }

  #[napi]
  pub fn to_f64(&self) -> f64 {
    match &self.0 {
      dynwinrt::WinRTValue::F64(f) => *f,
      dynwinrt::WinRTValue::F32(f) => *f as f64,
      _ => self.to_number() as f64,
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
    self.0.is_null_object()
  }

  #[napi]
  pub fn as_raw(&self) -> i64 {
    match &self.0 {
      dynwinrt::WinRTValue::Object(o) => o.as_raw() as i64,
      _ => panic!("Cannot get raw pointer from non-object"),
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
  pub fn get(&self, index: u32) -> DynWinRTValue {
    DynWinRTValue::new(self.0.get(index as usize))
  }

  /// Convert all elements to DynWinRTValue array.
  #[napi]
  pub fn to_values(&self) -> Vec<DynWinRTValue> {
    (0..self.0.len())
      .map(|i| DynWinRTValue::new(self.0.get(i)))
      .collect()
  }

  // -- Blittable fast paths: zero-copy read into typed Vec --

  #[napi]
  pub fn to_i8_vec(&self) -> Vec<i32> {
    unsafe {
      self
        .0
        .as_typed_slice::<i8>()
        .iter()
        .map(|&v| v as i32)
        .collect()
    }
  }

  #[napi]
  pub fn to_u8_vec(&self) -> Vec<u8> {
    unsafe { self.0.as_typed_slice::<u8>().to_vec() }
  }

  /// Return the u8 array data as a Node.js Buffer (zero-copy friendly, much
  /// more memory-efficient than to_u8_vec for large byte arrays).
  #[napi]
  pub fn to_buffer(&self) -> napi::bindgen_prelude::Buffer {
    let data = unsafe { self.0.as_typed_slice::<u8>().to_vec() };
    data.into()
  }

  #[napi]
  pub fn to_i16_vec(&self) -> Vec<i32> {
    unsafe {
      self
        .0
        .as_typed_slice::<i16>()
        .iter()
        .map(|&v| v as i32)
        .collect()
    }
  }

  #[napi]
  pub fn to_u16_vec(&self) -> Vec<u32> {
    unsafe {
      self
        .0
        .as_typed_slice::<u16>()
        .iter()
        .map(|&v| v as u32)
        .collect()
    }
  }

  #[napi]
  pub fn to_i32_vec(&self) -> Vec<i32> {
    unsafe { self.0.as_typed_slice::<i32>().to_vec() }
  }

  #[napi]
  pub fn to_u32_vec(&self) -> Vec<u32> {
    unsafe { self.0.as_typed_slice::<u32>().to_vec() }
  }

  #[napi]
  pub fn to_f32_vec(&self) -> Vec<f32> {
    unsafe { self.0.as_typed_slice::<f32>().to_vec() }
  }

  #[napi]
  pub fn to_f64_vec(&self) -> Vec<f64> {
    unsafe { self.0.as_typed_slice::<f64>().to_vec() }
  }

  #[napi]
  pub fn to_i64_vec(&self) -> Vec<i64> {
    unsafe { self.0.as_typed_slice::<i64>().to_vec() }
  }

  #[napi]
  pub fn to_u64_vec(&self) -> Vec<i64> {
    unsafe {
      self
        .0
        .as_typed_slice::<u64>()
        .iter()
        .map(|&v| v as i64)
        .collect()
    }
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
  pub fn from_i64_values(values: Vec<i64>) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> =
      values.into_iter().map(dynwinrt::WinRTValue::I64).collect();
    DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.i64_type(), &wvals))
  }

  #[napi]
  pub fn from_u64_values(values: Vec<i64>) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> = values
      .into_iter()
      .map(|v| dynwinrt::WinRTValue::U64(v as u64))
      .collect();
    DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.u64_type(), &wvals))
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

// ======================================================================
// Struct binding — typed field access by index
// ======================================================================

#[napi]
pub struct DynWinRTStruct(dynwinrt::ValueTypeData);
unsafe impl Send for DynWinRTStruct {}
unsafe impl Sync for DynWinRTStruct {}

#[napi]
impl DynWinRTStruct {
  /// Create a zero-initialized struct of the given type.
  #[napi]
  pub fn create(typ: &DynWinRTType) -> DynWinRTStruct {
    DynWinRTStruct(typ.0.default_value())
  }

  #[napi]
  pub fn get_i8(&self, index: u32) -> i32 {
    self.0.get_field::<i8>(index as usize) as i32
  }
  #[napi]
  pub fn set_i8(&mut self, index: u32, value: i32) {
    self.0.set_field(index as usize, value as i8);
  }

  #[napi]
  pub fn get_u8(&self, index: u32) -> u32 {
    self.0.get_field::<u8>(index as usize) as u32
  }
  #[napi]
  pub fn set_u8(&mut self, index: u32, value: u32) {
    self.0.set_field(index as usize, value as u8);
  }

  #[napi]
  pub fn get_i16(&self, index: u32) -> i32 {
    self.0.get_field::<i16>(index as usize) as i32
  }
  #[napi]
  pub fn set_i16(&mut self, index: u32, value: i32) {
    self.0.set_field(index as usize, value as i16);
  }

  #[napi]
  pub fn get_u16(&self, index: u32) -> u32 {
    self.0.get_field::<u16>(index as usize) as u32
  }
  #[napi]
  pub fn set_u16(&mut self, index: u32, value: u32) {
    self.0.set_field(index as usize, value as u16);
  }

  #[napi]
  pub fn get_i32(&self, index: u32) -> i32 {
    self.0.get_field::<i32>(index as usize)
  }
  #[napi]
  pub fn set_i32(&mut self, index: u32, value: i32) {
    self.0.set_field(index as usize, value);
  }

  #[napi]
  pub fn get_u32(&self, index: u32) -> u32 {
    self.0.get_field::<u32>(index as usize)
  }
  #[napi]
  pub fn set_u32(&mut self, index: u32, value: u32) {
    self.0.set_field(index as usize, value);
  }

  #[napi]
  pub fn get_f32(&self, index: u32) -> f64 {
    self.0.get_field::<f32>(index as usize) as f64
  }
  #[napi]
  pub fn set_f32(&mut self, index: u32, value: f64) {
    self.0.set_field(index as usize, value as f32);
  }

  #[napi]
  pub fn get_f64(&self, index: u32) -> f64 {
    self.0.get_field::<f64>(index as usize)
  }
  #[napi]
  pub fn set_f64(&mut self, index: u32, value: f64) {
    self.0.set_field(index as usize, value);
  }

  #[napi]
  pub fn get_i64(&self, index: u32) -> BigInt {
    BigInt::from(self.0.get_field::<i64>(index as usize))
  }
  #[napi]
  pub fn set_i64(&mut self, index: u32, value: BigInt) {
    let (n, _lossless) = value.get_i64();
    self.0.set_field(index as usize, n);
  }

  #[napi]
  pub fn get_u64(&self, index: u32) -> BigInt {
    BigInt::from(self.0.get_field::<u64>(index as usize))
  }
  #[napi]
  pub fn set_u64(&mut self, index: u32, value: BigInt) {
    let (_sign, n, _lossless) = value.get_u64();
    self.0.set_field(index as usize, n);
  }

  // -- Non-blittable field access --

  #[napi]
  pub fn get_hstring(&self, index: u32) -> String {
    let inner = self.0.get_field_struct(index as usize);
    // The field is an HSTRING (pointer-sized). Read it as a WinRTValue and convert.
    // get_field_struct handles the duplicate/clone of the HSTRING.
    // We need to read the raw HSTRING pointer from the inner ValueTypeData.
    let hstr: HSTRING = unsafe {
      let raw = *(inner.as_ptr() as *const *mut std::ffi::c_void);
      if raw.is_null() {
        HSTRING::new()
      } else {
        // Clone so we don't steal the reference from inner (which will Drop)
        let hstr_ref: &HSTRING = &*((&raw) as *const *mut std::ffi::c_void as *const HSTRING);
        hstr_ref.clone()
      }
    };
    hstr.to_string()
  }

  #[napi]
  pub fn set_hstring(&mut self, index: u32, value: String) {
    let hstr = HSTRING::from(&value);
    let field_handle = self.0.type_handle().field_type(index as usize);
    let mut field_val = field_handle.default_value();
    unsafe {
      let raw: *mut std::ffi::c_void = std::mem::transmute(hstr);
      (field_val.as_mut_ptr() as *mut *mut std::ffi::c_void).write(raw);
    }
    // set_field_struct duplicates non-blittable fields, so field_val's HSTRING
    // will be cloned into parent. Let field_val drop normally to release the original.
    self.0.set_field_struct(index as usize, &field_val);
  }

  #[napi]
  pub fn get_guid(&self, index: u32) -> WinGUID {
    let guid = self.0.get_field::<windows::core::GUID>(index as usize);
    WinGUID(guid)
  }

  #[napi]
  pub fn set_guid(&mut self, index: u32, value: &WinGUID) {
    self.0.set_field(index as usize, value.0);
  }

  #[napi]
  pub fn get_struct(&self, index: u32) -> DynWinRTStruct {
    DynWinRTStruct(self.0.get_field_struct(index as usize))
  }

  #[napi]
  pub fn set_struct(&mut self, index: u32, value: &DynWinRTStruct) {
    self.0.set_field_struct(index as usize, &value.0);
  }

  #[napi]
  pub fn get_object(&self, index: u32) -> napi::Result<DynWinRTValue> {
    let inner = self.0.get_field_struct(index as usize);
    let raw = unsafe { *(inner.as_ptr() as *const *mut std::ffi::c_void) };
    if raw.is_null() {
      Ok(DynWinRTValue::new(dynwinrt::WinRTValue::Null))
    } else {
      let obj = unsafe { IUnknown::from_raw_borrowed(&raw) }
        .ok_or_else(|| napi::Error::from_reason("null COM pointer"))?
        .clone();
      Ok(DynWinRTValue::new(dynwinrt::WinRTValue::Object(obj)))
    }
  }

  #[napi]
  pub fn set_object(&mut self, index: u32, value: &DynWinRTValue) {
    match &value.0 {
      dynwinrt::WinRTValue::Object(obj) => {
        let field_handle = self.0.type_handle().field_type(index as usize);
        let mut field_val = field_handle.default_value();
        unsafe {
          // Clone the object (AddRef) and write the raw pointer
          let cloned = obj.clone();
          let raw = cloned.into_raw();
          (field_val.as_mut_ptr() as *mut *mut std::ffi::c_void).write(raw);
        }
        // set_field_struct duplicates non-blittable fields, so field_val's COM pointer
        // will be cloned (AddRef) into parent. Let field_val drop to release the original.
        self.0.set_field_struct(index as usize, &field_val);
      }
      dynwinrt::WinRTValue::Null => {
        let field_handle = self.0.type_handle().field_type(index as usize);
        let field_val = field_handle.default_value();
        self.0.set_field_struct(index as usize, &field_val);
      }
      _ => {}
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
    // NOTE: `GetCurrentThreadId` returns a Windows DWORD that the OS is free
    // to recycle once a thread exits. We assume the register thread outlives
    // every delegate invocation — this holds for the common case (delegate is
    // dropped when the subscription is released, and both are typically owned
    // by the JS thread that registered them). If a Node worker exits while
    // its delegate is still reachable from another thread, a recycled TID
    // could steer a cross-thread invocation into the same-thread branch and
    // touch a stale `napi_env`. Fixing that would require a per-thread epoch
    // or TLS handshake; not needed for current use cases.
    let register_tid = unsafe { GetCurrentThreadId() };

    // Raw env is needed to make direct N-API calls from the delegate closure.
    // It's only ever dereferenced on `register_tid`, so we wrap it in a Send+Sync
    // newtype to satisfy the DelegateCallback trait bounds.
    struct SendableEnv(napi::sys::napi_env);
    unsafe impl Send for SendableEnv {}
    unsafe impl Sync for SendableEnv {}
    let raw_env_wrap = Arc::new(SendableEnv(callback.value().env));

    let fn_ref = Arc::new(callback.create_ref()?);
    let tsfn = callback.build_threadsafe_function().build()?;

    let type_handles: Vec<dynwinrt::TypeHandle> = param_types.iter().map(|t| t.0.clone()).collect();

    let raw_env_cb = raw_env_wrap.clone();
    let fn_ref_cb = fn_ref.clone();

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
          let raw_env = raw_env_cb.0;
          let unwind_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || -> windows::core::HRESULT {
              unsafe {
                let mut scope: napi::sys::napi_handle_scope = std::ptr::null_mut();
                if napi::sys::napi_open_handle_scope(raw_env, &mut scope)
                  != napi::sys::Status::napi_ok
                {
                  // Env is probably being torn down. Don't lie to WinRT that we
                  // ran successfully — surface E_FAIL so async ops etc. don't
                  // silently hang waiting for a completion that never happens.
                  eprintln!("[dynwinrt] delegate: napi_open_handle_scope failed (env teardown?)");
                  return E_FAIL;
                }
                let scoped_env = napi::Env::from_raw(raw_env);
                let call_result = (|| -> napi::Result<()> {
                  let fn_scope = fn_ref_cb.borrow_back(&scoped_env)?;
                  let fn_val = napi::JsValue::raw(&fn_scope);
                  // Spread each DynWinRTValue as its own napi_value so the JS
                  // callback receives them as positional args. The blanket
                  // `Vec<T>::into_vec` impl wraps the whole vec as a single JS
                  // Array, which is wrong here — we need one arg per element.
                  let mut argv: Vec<napi::sys::napi_value> = Vec::with_capacity(js_args.len());
                  for v in js_args {
                    let raw = DynWinRTValue::to_napi_value(raw_env, v)?;
                    argv.push(raw);
                  }
                  let mut undefined: napi::sys::napi_value = std::ptr::null_mut();
                  napi::sys::napi_get_undefined(raw_env, &mut undefined);
                  let mut result: napi::sys::napi_value = std::ptr::null_mut();
                  let status = napi::sys::napi_call_function(
                    raw_env,
                    undefined,
                    fn_val,
                    argv.len(),
                    argv.as_ptr(),
                    &mut result,
                  );
                  if status != napi::sys::Status::napi_ok {
                    // Surface any pending JS exception so it doesn't silently poison
                    // future calls. Delegates return HRESULT; there's no clean way to
                    // propagate a JS throw back through WinRT, so we route it through
                    // napi_fatal_exception (same policy tsfn uses).
                    let mut is_pending: bool = false;
                    napi::sys::napi_is_exception_pending(raw_env, &mut is_pending);
                    if is_pending {
                      let mut err: napi::sys::napi_value = std::ptr::null_mut();
                      napi::sys::napi_get_and_clear_last_exception(raw_env, &mut err);
                      napi::sys::napi_fatal_exception(raw_env, err);
                    }
                    return Err(napi::Error::from_reason("napi_call_function failed"));
                  }
                  Ok(())
                })();
                napi::sys::napi_close_handle_scope(raw_env, scope);
                // Log and report any non-exception error to WinRT. Without this,
                // failures like invalid handles or marshaler errors would be
                // silently dropped and the delegate would appear to have run.
                if let Err(e) = call_result {
                  eprintln!("[dynwinrt] delegate dispatch error: {e}");
                  return E_FAIL;
                }
                windows::core::HRESULT(0)
              }
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
        tsfn.call(js_args, ThreadsafeFunctionCallMode::NonBlocking);
        windows::core::HRESULT(0)
      });

    let value = dynwinrt::delegate::create_delegate_value(iid.0, type_handles, delegate_callback);
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
              let mut undefined = std::ptr::null_mut();
              napi::sys::napi_get_undefined(raw_env, &mut undefined);
              let mut raw_result = std::ptr::null_mut();
              let status = napi::sys::napi_call_function(
                raw_env,
                undefined,
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
              Ok(
                <&DynWinRTValue>::from_napi_value(raw_env, raw_result)?
                  .0
                  .clone(),
              )
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
              let mut undefined = std::ptr::null_mut();
              napi::sys::napi_get_undefined(raw_env, &mut undefined);
              let mut raw_result = std::ptr::null_mut();
              let status = napi::sys::napi_call_function(
                raw_env,
                undefined,
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
