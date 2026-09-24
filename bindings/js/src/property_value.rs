// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::DynWinRTValue;
use napi::{
  bindgen_prelude::{BigInt, FromNapiValue, ToNapiValue, Unknown},
  Env,
};
use napi_derive::napi;

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
    unsafe { napi::sys::napi_create_string_utf16(env.raw(), value.as_ptr(), length, &mut result) },
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

/// JavaScript does not project these PropertyTypes yet. Report them with the
/// exact error the core raised before it modeled every PropertyType, so the
/// JavaScript contract stays unchanged.
fn unsupported_property_type(property_type: windows::Foundation::PropertyType) -> napi::Error {
  let error = dynwinrt::Error::WindowsError(windows::core::Error::new(
    windows::core::HRESULT(0x80004001_u32 as i32),
    format!("Unsupported WinRT IPropertyValue type: {}", property_type.0),
  ));
  napi::Error::from_reason(error.message())
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
    value @ (PropertyValueData::DateTime(_)
    | PropertyValueData::TimeSpan(_)
    | PropertyValueData::Point(_)
    | PropertyValueData::Size(_)
    | PropertyValueData::Rect(_)
    | PropertyValueData::InspectableArray(_)
    | PropertyValueData::DateTimeArray(_)
    | PropertyValueData::TimeSpanArray(_)
    | PropertyValueData::PointArray(_)
    | PropertyValueData::SizeArray(_)
    | PropertyValueData::RectArray(_)) => Err(unsupported_property_type(value.property_type())),
  }
}

/// Explicitly unbox a supported WinRT `IPropertyValue`.
///
/// JavaScript `null` and WinRT null are returned as `null`. A non-`IPropertyValue`
/// `DynWinRtValue` is returned unchanged, preserving JavaScript and COM identity.
#[napi(
  ts_args_type = "value: unknown",
  ts_return_type = "boolean | number | bigint | string | Uint8Array | number[] | bigint[] | boolean[] | string[] | DynWinRtValue | null"
)]
pub fn unbox_object<'env>(env: Env, value: Unknown<'env>) -> napi::Result<Unknown<'env>> {
  if value.get_type()? == napi::ValueType::Null {
    return Ok(value);
  }

  unsafe {
    crate::native_class_ref::with_ref::<DynWinRTValue, _>(
      env.raw(),
      napi::JsValue::raw(&value),
      |raw| match dynwinrt::unbox_property_value(raw.winrt())
        .map_err(|error| napi::Error::from_reason(error.message()))?
      {
        dynwinrt::PropertyValueUnboxResult::Null => null_unknown(env),
        dynwinrt::PropertyValueUnboxResult::NotPropertyValue => Ok(value),
        dynwinrt::PropertyValueUnboxResult::Unsupported(property_type) => {
          Err(unsupported_property_type(property_type))
        }
        dynwinrt::PropertyValueUnboxResult::Value(value) => {
          property_value_to_javascript(env, value)
        }
      },
    )
  }
}
