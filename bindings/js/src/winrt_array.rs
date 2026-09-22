// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::{com_input, js_numbers::*, DynWinRTType, DynWinRTValue, TABLE};
use napi::bindgen_prelude::{BigInt, Either};
use napi_derive::napi;
use std::mem;
use windows::core::HSTRING;

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
  if !expected(array.element_type.underlying_kind()) {
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

native_class! {
  pub struct DynWinRTArray(
    pub(crate) dynwinrt::ArrayData,
    pub(crate) Option<com_input::InputBindings>,
  );
}

impl DynWinRTArray {
  pub(crate) fn new(data: dynwinrt::ArrayData) -> Self {
    Self(data, None)
  }

  fn element_value(&self, index: usize) -> DynWinRTValue {
    let mut value = DynWinRTValue::new(self.0.get(index));
    if let Some(inputs) = &self.1 {
      value.set_input_bindings(Some(inputs.element_bookkeeping(index)));
    }
    value
  }

  fn ensure_existing_com_apartment(&self) -> napi::Result<()> {
    if let Some(inputs) = &self.1 {
      if inputs.is_apartment_bound() {
        inputs.ensure_owner()?;
      }
    }
    Ok(())
  }
}
unsafe impl Send for DynWinRTArray {}
unsafe impl Sync for DynWinRTArray {}

impl Drop for DynWinRTArray {
  fn drop(&mut self) {
    if self
      .1
      .as_ref()
      .is_some_and(|inputs| inputs.is_apartment_bound() && !inputs.is_owner())
    {
      let empty = dynwinrt::ArrayData::empty(self.0.element_type.clone());
      mem::forget(mem::replace(&mut self.0, empty));
    }
  }
}

#[napi]
impl DynWinRTArray {
  #[napi]
  pub fn len(&self) -> u32 {
    self.0.len() as u32
  }

  /// Per-element access (works for all element types).
  #[napi]
  pub fn get(&self, index: f64) -> napi::Result<DynWinRTValue> {
    self.ensure_existing_com_apartment()?;
    let index = js_u32(index, "get")? as usize;
    if index >= self.0.len() {
      return Err(napi::Error::from_reason(format!(
        "Array index {index} is out of bounds for length {}",
        self.0.len(),
      )));
    }
    Ok(self.element_value(index))
  }

  /// Convert all elements to DynWinRTValue array.
  #[napi]
  pub fn to_values(&self) -> napi::Result<Vec<DynWinRTValue>> {
    self.ensure_existing_com_apartment()?;
    Ok((0..self.0.len()).map(|i| self.element_value(i)).collect())
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
      |kind| matches!(kind, dynwinrt::TypeKind::I32 | dynwinrt::TypeKind::HResult),
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
        dynwinrt::WinRTValue::Enum { value, type_handle }
          if type_handle.underlying_kind() == dynwinrt::TypeKind::U32 =>
        {
          Some(value as u32)
        }
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
    DynWinRTArray::new(dynwinrt::ArrayData::from_values(TABLE.i8_type(), &wvals))
  }

  #[napi]
  pub fn from_u8_values(values: Vec<u8>) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> =
      values.into_iter().map(dynwinrt::WinRTValue::U8).collect();
    DynWinRTArray::new(dynwinrt::ArrayData::from_values(TABLE.u8_type(), &wvals))
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
    DynWinRTArray::new(dynwinrt::ArrayData::from_values(TABLE.u8_type(), &wvals))
  }

  #[napi]
  pub fn from_i16_values(values: Vec<i32>) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> = values
      .into_iter()
      .map(|v| dynwinrt::WinRTValue::I16(v as i16))
      .collect();
    DynWinRTArray::new(dynwinrt::ArrayData::from_values(TABLE.i16_type(), &wvals))
  }

  #[napi]
  pub fn from_u16_values(values: Vec<u32>) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> = values
      .into_iter()
      .map(|v| dynwinrt::WinRTValue::U16(v as u16))
      .collect();
    DynWinRTArray::new(dynwinrt::ArrayData::from_values(TABLE.u16_type(), &wvals))
  }

  #[napi]
  pub fn from_i32_values(values: Vec<i32>) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> =
      values.into_iter().map(dynwinrt::WinRTValue::I32).collect();
    DynWinRTArray::new(dynwinrt::ArrayData::from_values(TABLE.i32_type(), &wvals))
  }

  /// Build an array whose native element type is HRESULT, not I32.
  #[napi]
  pub fn from_hresult_values(values: Vec<f64>) -> napi::Result<DynWinRTArray> {
    let values = values
      .into_iter()
      .map(|value| {
        js_i32(value, "fromHresultValues")
          .map(|value| dynwinrt::WinRTValue::HResult(windows::core::HRESULT(value)))
      })
      .collect::<napi::Result<Vec<_>>>()?;
    Ok(DynWinRTArray::new(dynwinrt::ArrayData::from_values(
      TABLE.hresult(),
      &values,
    )))
  }

  #[napi]
  pub fn from_u32_values(values: Vec<f64>) -> napi::Result<DynWinRTArray> {
    let wvals = values
      .into_iter()
      .map(|value| js_u32(value, "fromU32Values").map(dynwinrt::WinRTValue::U32))
      .collect::<napi::Result<Vec<_>>>()?;
    Ok(DynWinRTArray::new(dynwinrt::ArrayData::from_values(
      TABLE.u32_type(),
      &wvals,
    )))
  }

  #[napi]
  pub fn from_f32_values(values: Vec<f64>) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> = values
      .into_iter()
      .map(|v| dynwinrt::WinRTValue::F32(v as f32))
      .collect();
    DynWinRTArray::new(dynwinrt::ArrayData::from_values(TABLE.f32_type(), &wvals))
  }

  #[napi]
  pub fn from_f64_values(values: Vec<f64>) -> DynWinRTArray {
    let wvals: Vec<dynwinrt::WinRTValue> =
      values.into_iter().map(dynwinrt::WinRTValue::F64).collect();
    DynWinRTArray::new(dynwinrt::ArrayData::from_values(TABLE.f64_type(), &wvals))
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
    Ok(DynWinRTArray::new(dynwinrt::ArrayData::from_values(
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
    Ok(DynWinRTArray::new(dynwinrt::ArrayData::from_values(
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
    DynWinRTArray::new(dynwinrt::ArrayData::from_values(
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
  ) -> napi::Result<DynWinRTArray> {
    for value in &values {
      value.ensure_existing_com_apartment()?;
    }
    let inputs = com_input::InputBindings::inherit(&values);
    let wvals: Vec<dynwinrt::WinRTValue> = values.iter().map(|v| v.winrt().clone()).collect();
    Ok(DynWinRTArray(
      dynwinrt::ArrayData::from_values(element_type.0.clone(), &wvals),
      Some(inputs),
    ))
  }

  /// Wrap as DynWinRTValue::Array for passing to call().
  #[napi]
  pub fn to_value(&self) -> napi::Result<DynWinRTValue> {
    self.ensure_existing_com_apartment()?;
    let mut value = DynWinRTValue::new(dynwinrt::WinRTValue::Array(self.0.clone()));
    if let Some(inputs) = &self.1 {
      value.set_input_bindings(Some(inputs.copy_bookkeeping()));
    }
    Ok(value)
  }
}
