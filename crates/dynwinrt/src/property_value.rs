// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use windows::Foundation::{IPropertyValue, PropertyType};
use windows_core::{Array, GUID, HRESULT, HSTRING, Interface};

use crate::{Error, Result, WinRTValue};

const E_NOINTERFACE: HRESULT = HRESULT(0x80004002_u32 as i32);
const E_NOTIMPL: HRESULT = HRESULT(0x80004001_u32 as i32);

/// A language-neutral value explicitly read from a WinRT `IPropertyValue`.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValueData {
    UInt8(u8),
    Int16(i16),
    UInt16(u16),
    Int32(i32),
    UInt32(u32),
    Int64(i64),
    UInt64(u64),
    Single(f32),
    Double(f64),
    Char16(u16),
    Boolean(bool),
    String(String),
    Guid(GUID),
    UInt8Array(Vec<u8>),
    Int16Array(Vec<i16>),
    UInt16Array(Vec<u16>),
    Int32Array(Vec<i32>),
    UInt32Array(Vec<u32>),
    Int64Array(Vec<i64>),
    UInt64Array(Vec<u64>),
    SingleArray(Vec<f32>),
    DoubleArray(Vec<f64>),
    Char16Array(Vec<u16>),
    BooleanArray(Vec<bool>),
    StringArray(Vec<String>),
    GuidArray(Vec<GUID>),
}

/// Result of explicitly attempting to unbox a raw WinRT object.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValueUnboxResult {
    /// The input was the WinRT null object.
    Null,
    /// The input does not implement `IPropertyValue` and must be preserved.
    NotPropertyValue,
    /// The input was a supported boxed property value.
    Value(PropertyValueData),
}

macro_rules! read_array {
    ($property_value:expr, $getter:ident, $typ:ty) => {{
        let mut values = Array::<$typ>::new();
        $property_value.$getter(&mut values)?;
        values.to_vec()
    }};
}

/// Explicitly reads a supported `IPropertyValue` without consuming or mutating `value`.
///
/// A QueryInterface failure with `E_NOINTERFACE` is reported as
/// [`PropertyValueUnboxResult::NotPropertyValue`]. Other QueryInterface failures,
/// unsupported `PropertyType` values, and getter failures are surfaced as errors.
pub fn unbox_property_value(value: &WinRTValue) -> Result<PropertyValueUnboxResult> {
    if value.is_null_object() {
        return Ok(PropertyValueUnboxResult::Null);
    }

    let Some(object) = value.as_object() else {
        return Ok(PropertyValueUnboxResult::NotPropertyValue);
    };
    let property_value = match object.cast::<IPropertyValue>() {
        Ok(value) => value,
        Err(error) if error.code() == E_NOINTERFACE => {
            return Ok(PropertyValueUnboxResult::NotPropertyValue);
        }
        Err(error) => return Err(Error::WindowsError(error)),
    };

    let property_type = property_value.Type()?;
    let value = match property_type {
        PropertyType::UInt8 => PropertyValueData::UInt8(property_value.GetUInt8()?),
        PropertyType::Int16 => PropertyValueData::Int16(property_value.GetInt16()?),
        PropertyType::UInt16 => PropertyValueData::UInt16(property_value.GetUInt16()?),
        PropertyType::Int32 => PropertyValueData::Int32(property_value.GetInt32()?),
        PropertyType::UInt32 => PropertyValueData::UInt32(property_value.GetUInt32()?),
        PropertyType::Int64 => PropertyValueData::Int64(property_value.GetInt64()?),
        PropertyType::UInt64 => PropertyValueData::UInt64(property_value.GetUInt64()?),
        PropertyType::Single => PropertyValueData::Single(property_value.GetSingle()?),
        PropertyType::Double => PropertyValueData::Double(property_value.GetDouble()?),
        PropertyType::Char16 => PropertyValueData::Char16(property_value.GetChar16()?),
        PropertyType::Boolean => PropertyValueData::Boolean(property_value.GetBoolean()?),
        PropertyType::String => PropertyValueData::String(property_value.GetString()?.to_string()),
        PropertyType::Guid => PropertyValueData::Guid(property_value.GetGuid()?),
        PropertyType::UInt8Array => {
            PropertyValueData::UInt8Array(read_array!(property_value, GetUInt8Array, u8))
        }
        PropertyType::Int16Array => {
            PropertyValueData::Int16Array(read_array!(property_value, GetInt16Array, i16))
        }
        PropertyType::UInt16Array => {
            PropertyValueData::UInt16Array(read_array!(property_value, GetUInt16Array, u16))
        }
        PropertyType::Int32Array => {
            PropertyValueData::Int32Array(read_array!(property_value, GetInt32Array, i32))
        }
        PropertyType::UInt32Array => {
            PropertyValueData::UInt32Array(read_array!(property_value, GetUInt32Array, u32))
        }
        PropertyType::Int64Array => {
            PropertyValueData::Int64Array(read_array!(property_value, GetInt64Array, i64))
        }
        PropertyType::UInt64Array => {
            PropertyValueData::UInt64Array(read_array!(property_value, GetUInt64Array, u64))
        }
        PropertyType::SingleArray => {
            PropertyValueData::SingleArray(read_array!(property_value, GetSingleArray, f32))
        }
        PropertyType::DoubleArray => {
            PropertyValueData::DoubleArray(read_array!(property_value, GetDoubleArray, f64))
        }
        PropertyType::Char16Array => {
            PropertyValueData::Char16Array(read_array!(property_value, GetChar16Array, u16))
        }
        PropertyType::BooleanArray => {
            PropertyValueData::BooleanArray(read_array!(property_value, GetBooleanArray, bool))
        }
        PropertyType::StringArray => {
            let values = read_array!(property_value, GetStringArray, HSTRING);
            PropertyValueData::StringArray(
                values.into_iter().map(|value| value.to_string()).collect(),
            )
        }
        PropertyType::GuidArray => {
            PropertyValueData::GuidArray(read_array!(property_value, GetGuidArray, GUID))
        }
        _ => {
            return Err(Error::WindowsError(windows_core::Error::new(
                E_NOTIMPL,
                format!("Unsupported WinRT IPropertyValue type: {}", property_type.0),
            )));
        }
    };

    Ok(PropertyValueUnboxResult::Value(value))
}

#[cfg(test)]
mod tests {
    use windows::Foundation::{DateTime, PropertyValue, Uri};
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    use windows_core::{HSTRING, Interface};

    use super::*;

    fn as_value<T: Interface>(value: &T) -> WinRTValue {
        WinRTValue::Object(value.cast().expect("IPropertyValue as IUnknown"))
    }

    #[test]
    fn unboxes_scalar_and_array_property_values() -> windows_core::Result<()> {
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        let string = PropertyValue::CreateString(&HSTRING::from("BLE Device"))?;
        assert_eq!(
            unbox_property_value(&as_value(&string)).unwrap(),
            PropertyValueUnboxResult::Value(PropertyValueData::String("BLE Device".to_string()))
        );

        let int64 = PropertyValue::CreateInt64(i64::MIN)?;
        assert_eq!(
            unbox_property_value(&as_value(&int64)).unwrap(),
            PropertyValueUnboxResult::Value(PropertyValueData::Int64(i64::MIN))
        );

        let bytes = PropertyValue::CreateUInt8Array(&[0, 1, 127, 255])?;
        assert_eq!(
            unbox_property_value(&as_value(&bytes)).unwrap(),
            PropertyValueUnboxResult::Value(PropertyValueData::UInt8Array(vec![0, 1, 127, 255]))
        );

        let strings =
            PropertyValue::CreateStringArray(&[HSTRING::from("one"), HSTRING::from("two")])?;
        assert_eq!(
            unbox_property_value(&as_value(&strings)).unwrap(),
            PropertyValueUnboxResult::Value(PropertyValueData::StringArray(vec![
                "one".to_string(),
                "two".to_string()
            ]))
        );

        Ok(())
    }

    #[test]
    fn preserves_null_and_non_property_objects() -> windows_core::Result<()> {
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        assert_eq!(
            unbox_property_value(&WinRTValue::Null).unwrap(),
            PropertyValueUnboxResult::Null
        );

        let uri = Uri::CreateUri(&HSTRING::from("https://example.com"))?;
        let raw = uri.as_raw();
        let value = as_value(&uri);
        assert_eq!(
            unbox_property_value(&value).unwrap(),
            PropertyValueUnboxResult::NotPropertyValue
        );
        assert_eq!(value.as_object().unwrap().as_raw(), raw);
        assert_eq!(uri.Host()?, "example.com");

        Ok(())
    }

    #[test]
    fn rejects_unsupported_property_types() -> windows_core::Result<()> {
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        let date_time = PropertyValue::CreateDateTime(DateTime { UniversalTime: 0 })?;
        let error = unbox_property_value(&as_value(&date_time)).unwrap_err();
        assert!(error.message().contains(&format!(
            "Unsupported WinRT IPropertyValue type: {}",
            PropertyType::DateTime.0
        )));

        Ok(())
    }
}
