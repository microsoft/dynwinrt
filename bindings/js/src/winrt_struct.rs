// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::{com_input, js_numbers::*, DynWinRTType, DynWinRTValue, WinGUID};
use napi::bindgen_prelude::{BigInt, Either};
use napi_derive::napi;
use std::mem;
use windows::core::HSTRING;

native_class! {
  pub struct DynWinRTStruct(
    pub(crate) dynwinrt::ValueTypeData,
    pub(crate) Vec<Option<com_input::InputBindings>>,
  );
}
unsafe impl Send for DynWinRTStruct {}
unsafe impl Sync for DynWinRTStruct {}

impl DynWinRTStruct {
  pub(crate) fn from_data(
    data: dynwinrt::ValueTypeData,
    inputs: Option<&com_input::InputBindings>,
  ) -> Self {
    let fields = (0..data.type_handle().field_count())
      .map(|index| inputs.map(|inputs| inputs.element_bookkeeping(index)))
      .collect();
    Self(data, fields)
  }

  fn ensure_existing_com_apartment(&self) -> napi::Result<()> {
    for input in self.1.iter().flatten() {
      if input.is_apartment_bound() {
        input.ensure_owner()?;
      }
    }
    Ok(())
  }

  fn checked_field_index(
    &self,
    index: f64,
    method: &str,
    expected: &str,
    accepts: impl Fn(dynwinrt::TypeKind) -> bool,
  ) -> napi::Result<usize> {
    self.ensure_existing_com_apartment()?;
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

impl Drop for DynWinRTStruct {
  fn drop(&mut self) {
    if self
      .1
      .iter()
      .flatten()
      .any(|input| input.is_apartment_bound() && !input.is_owner())
    {
      let empty = self.0.type_handle().default_value();
      mem::forget(mem::replace(&mut self.0, empty));
    }
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
    Ok(DynWinRTStruct::from_data(typ.0.default_value(), None))
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
    Ok(DynWinRTStruct::from_data(
      self.0.get_field_struct(index),
      self.1[index].as_ref(),
    ))
  }

  #[napi]
  pub fn set_struct(&mut self, index: f64, value: &DynWinRTStruct) -> napi::Result<()> {
    value.ensure_existing_com_apartment()?;
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
    let inputs = com_input::InputBindings::fields(&value.1);
    self.0.set_field_struct(index, &value.0);
    self.1[index] = Some(inputs);
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
      Some(object) => {
        let mut value = DynWinRTValue::new(dynwinrt::WinRTValue::Object(object));
        if let Some(inputs) = &self.1[index] {
          value.set_input_bindings(Some(inputs.copy_bookkeeping()));
        }
        Ok(value)
      }
      None => Ok(DynWinRTValue::new(dynwinrt::WinRTValue::Null)),
    }
  }

  #[napi]
  pub fn set_object(&mut self, index: f64, value: &DynWinRTValue) -> napi::Result<()> {
    value.ensure_existing_com_apartment()?;
    let index = self.checked_field_index(index, "setObject", "WinRT object", |kind| {
      kind.is_com_pointer()
    })?;
    let inputs = com_input::InputBindings::inherit(&[value]);
    match value.winrt() {
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
    }?;
    self.1[index] = Some(inputs.element_bookkeeping(0));
    Ok(())
  }

  /// Wrap as DynWinRTValue::Struct for passing to call().
  #[napi]
  pub fn to_value(&self) -> napi::Result<DynWinRTValue> {
    self.ensure_existing_com_apartment()?;
    let mut value = DynWinRTValue::new(dynwinrt::WinRTValue::Struct(self.0.clone()));
    if value.input_bindings().is_some() {
      value.set_input_bindings(Some(com_input::InputBindings::fields(&self.1)));
    }
    Ok(value)
  }
}
