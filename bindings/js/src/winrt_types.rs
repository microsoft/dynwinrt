// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::{DynWinRTMethodHandle, DynWinRTMethodSig};
use napi_derive::napi;
use std::sync::Arc;

/// Shared MetadataTable — created once, used everywhere.
pub(crate) static TABLE: std::sync::LazyLock<Arc<dynwinrt::MetadataTable>> =
  std::sync::LazyLock::new(|| dynwinrt::MetadataTable::new());

#[napi]
pub struct DynWinRTType(pub(crate) dynwinrt::TypeHandle);

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
pub struct WinGUID(pub(crate) windows::core::GUID);

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
