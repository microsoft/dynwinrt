// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Language-neutral boxing and unboxing of WinRT `IPropertyValue` objects.
//!
//! This module is the single core definition of which
//! `Windows.Foundation.PropertyType` values a projection can convert to and from
//! plain data. Language bindings map [`PropertyValueData`] to their own value
//! model; they never call `IPropertyValue` getters or `PropertyValue` factories
//! themselves.

use windows::Foundation::{
    DateTime, IPropertyValue, Point, PropertyType, PropertyValue, Rect, Size, TimeSpan,
};
use windows_core::{Array, GUID, HRESULT, HSTRING, IInspectable, IUnknown, Interface};

use crate::{Error, Result, WinRTValue};

const E_NOINTERFACE: HRESULT = HRESULT(0x80004002_u32 as i32);

/// The payload of a WinRT `IPropertyValue`, for every `PropertyType` that has a
/// language-neutral representation.
///
/// `Empty`, `Inspectable`, `OtherType` and `OtherTypeArray` carry no payload a
/// projection could reproduce, so reading them yields
/// [`PropertyValueUnboxResult::Unsupported`] instead of a value.
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
    DateTime(DateTime),
    TimeSpan(TimeSpan),
    Point(Point),
    Size(Size),
    Rect(Rect),
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
    /// Arbitrary WinRT objects; `None` is a null element. Bindings convert the
    /// elements with their own `Object` conversion, recursively.
    InspectableArray(Vec<Option<IUnknown>>),
    DateTimeArray(Vec<DateTime>),
    TimeSpanArray(Vec<TimeSpan>),
    GuidArray(Vec<GUID>),
    PointArray(Vec<Point>),
    SizeArray(Vec<Size>),
    RectArray(Vec<Rect>),
}

impl PropertyValueData {
    /// The `Windows.Foundation.PropertyType` this payload boxes as.
    pub fn property_type(&self) -> PropertyType {
        match self {
            Self::UInt8(_) => PropertyType::UInt8,
            Self::Int16(_) => PropertyType::Int16,
            Self::UInt16(_) => PropertyType::UInt16,
            Self::Int32(_) => PropertyType::Int32,
            Self::UInt32(_) => PropertyType::UInt32,
            Self::Int64(_) => PropertyType::Int64,
            Self::UInt64(_) => PropertyType::UInt64,
            Self::Single(_) => PropertyType::Single,
            Self::Double(_) => PropertyType::Double,
            Self::Char16(_) => PropertyType::Char16,
            Self::Boolean(_) => PropertyType::Boolean,
            Self::String(_) => PropertyType::String,
            Self::Guid(_) => PropertyType::Guid,
            Self::DateTime(_) => PropertyType::DateTime,
            Self::TimeSpan(_) => PropertyType::TimeSpan,
            Self::Point(_) => PropertyType::Point,
            Self::Size(_) => PropertyType::Size,
            Self::Rect(_) => PropertyType::Rect,
            Self::UInt8Array(_) => PropertyType::UInt8Array,
            Self::Int16Array(_) => PropertyType::Int16Array,
            Self::UInt16Array(_) => PropertyType::UInt16Array,
            Self::Int32Array(_) => PropertyType::Int32Array,
            Self::UInt32Array(_) => PropertyType::UInt32Array,
            Self::Int64Array(_) => PropertyType::Int64Array,
            Self::UInt64Array(_) => PropertyType::UInt64Array,
            Self::SingleArray(_) => PropertyType::SingleArray,
            Self::DoubleArray(_) => PropertyType::DoubleArray,
            Self::Char16Array(_) => PropertyType::Char16Array,
            Self::BooleanArray(_) => PropertyType::BooleanArray,
            Self::StringArray(_) => PropertyType::StringArray,
            Self::InspectableArray(_) => PropertyType::InspectableArray,
            Self::DateTimeArray(_) => PropertyType::DateTimeArray,
            Self::TimeSpanArray(_) => PropertyType::TimeSpanArray,
            Self::GuidArray(_) => PropertyType::GuidArray,
            Self::PointArray(_) => PropertyType::PointArray,
            Self::SizeArray(_) => PropertyType::SizeArray,
            Self::RectArray(_) => PropertyType::RectArray,
        }
    }
}

/// Result of attempting to unbox a raw WinRT object.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValueUnboxResult {
    /// The input was the WinRT null object.
    Null,
    /// The input does not implement `IPropertyValue` and must be preserved.
    NotPropertyValue,
    /// The input is an `IPropertyValue` whose `PropertyType` has no
    /// language-neutral payload (`Empty`, `Inspectable`, `OtherType`,
    /// `OtherTypeArray`, or a value unknown to this version). It must be
    /// preserved like [`PropertyValueUnboxResult::NotPropertyValue`].
    Unsupported(PropertyType),
    /// The input was a boxed value with a language-neutral payload.
    Value(PropertyValueData),
}

macro_rules! read_array {
    ($property_value:expr, $getter:ident, $typ:ty) => {{
        let mut values = Array::<$typ>::new();
        $property_value.$getter(&mut values)?;
        values.to_vec()
    }};
}

/// Reads a boxed value without consuming or mutating `value`.
///
/// Null and non-`IPropertyValue` objects are classified rather than rejected,
/// and so are `IPropertyValue` objects without a language-neutral payload. A
/// QueryInterface failure other than `E_NOINTERFACE` and a failing getter for a
/// supported `PropertyType` are surfaced as errors: those objects are broken,
/// not merely unsupported.
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

    let pv = &property_value;
    let data = match pv.Type()? {
        PropertyType::UInt8 => PropertyValueData::UInt8(pv.GetUInt8()?),
        PropertyType::Int16 => PropertyValueData::Int16(pv.GetInt16()?),
        PropertyType::UInt16 => PropertyValueData::UInt16(pv.GetUInt16()?),
        PropertyType::Int32 => PropertyValueData::Int32(pv.GetInt32()?),
        PropertyType::UInt32 => PropertyValueData::UInt32(pv.GetUInt32()?),
        PropertyType::Int64 => PropertyValueData::Int64(pv.GetInt64()?),
        PropertyType::UInt64 => PropertyValueData::UInt64(pv.GetUInt64()?),
        PropertyType::Single => PropertyValueData::Single(pv.GetSingle()?),
        PropertyType::Double => PropertyValueData::Double(pv.GetDouble()?),
        PropertyType::Char16 => PropertyValueData::Char16(pv.GetChar16()?),
        PropertyType::Boolean => PropertyValueData::Boolean(pv.GetBoolean()?),
        PropertyType::String => PropertyValueData::String(pv.GetString()?.to_string()),
        PropertyType::Guid => PropertyValueData::Guid(pv.GetGuid()?),
        PropertyType::DateTime => PropertyValueData::DateTime(pv.GetDateTime()?),
        PropertyType::TimeSpan => PropertyValueData::TimeSpan(pv.GetTimeSpan()?),
        PropertyType::Point => PropertyValueData::Point(pv.GetPoint()?),
        PropertyType::Size => PropertyValueData::Size(pv.GetSize()?),
        PropertyType::Rect => PropertyValueData::Rect(pv.GetRect()?),
        PropertyType::UInt8Array => {
            PropertyValueData::UInt8Array(read_array!(pv, GetUInt8Array, u8))
        }
        PropertyType::Int16Array => {
            PropertyValueData::Int16Array(read_array!(pv, GetInt16Array, i16))
        }
        PropertyType::UInt16Array => {
            PropertyValueData::UInt16Array(read_array!(pv, GetUInt16Array, u16))
        }
        PropertyType::Int32Array => {
            PropertyValueData::Int32Array(read_array!(pv, GetInt32Array, i32))
        }
        PropertyType::UInt32Array => {
            PropertyValueData::UInt32Array(read_array!(pv, GetUInt32Array, u32))
        }
        PropertyType::Int64Array => {
            PropertyValueData::Int64Array(read_array!(pv, GetInt64Array, i64))
        }
        PropertyType::UInt64Array => {
            PropertyValueData::UInt64Array(read_array!(pv, GetUInt64Array, u64))
        }
        PropertyType::SingleArray => {
            PropertyValueData::SingleArray(read_array!(pv, GetSingleArray, f32))
        }
        PropertyType::DoubleArray => {
            PropertyValueData::DoubleArray(read_array!(pv, GetDoubleArray, f64))
        }
        PropertyType::Char16Array => {
            PropertyValueData::Char16Array(read_array!(pv, GetChar16Array, u16))
        }
        PropertyType::BooleanArray => {
            PropertyValueData::BooleanArray(read_array!(pv, GetBooleanArray, bool))
        }
        PropertyType::StringArray => PropertyValueData::StringArray(
            read_array!(pv, GetStringArray, HSTRING)
                .into_iter()
                .map(|value| value.to_string())
                .collect(),
        ),
        PropertyType::InspectableArray => PropertyValueData::InspectableArray(
            read_array!(pv, GetInspectableArray, IInspectable)
                .into_iter()
                .map(|value| value.map(IUnknown::from))
                .collect(),
        ),
        PropertyType::DateTimeArray => {
            PropertyValueData::DateTimeArray(read_array!(pv, GetDateTimeArray, DateTime))
        }
        PropertyType::TimeSpanArray => {
            PropertyValueData::TimeSpanArray(read_array!(pv, GetTimeSpanArray, TimeSpan))
        }
        PropertyType::GuidArray => {
            PropertyValueData::GuidArray(read_array!(pv, GetGuidArray, GUID))
        }
        PropertyType::PointArray => {
            PropertyValueData::PointArray(read_array!(pv, GetPointArray, Point))
        }
        PropertyType::SizeArray => {
            PropertyValueData::SizeArray(read_array!(pv, GetSizeArray, Size))
        }
        PropertyType::RectArray => {
            PropertyValueData::RectArray(read_array!(pv, GetRectArray, Rect))
        }
        unsupported => return Ok(PropertyValueUnboxResult::Unsupported(unsupported)),
    };

    Ok(PropertyValueUnboxResult::Value(data))
}

/// Boxes `data` as a system `Windows.Foundation.PropertyValue`.
///
/// The result implements `IPropertyValue` (reporting [`PropertyValueData::property_type`])
/// and the matching `IReference<T>` / `IReferenceArray<T>`, exactly like values
/// boxed by C#, C++/WinRT or `PropertyValue.Create*`.
pub fn box_property_value(data: &PropertyValueData) -> Result<WinRTValue> {
    let boxed = match data {
        PropertyValueData::UInt8(value) => PropertyValue::CreateUInt8(*value),
        PropertyValueData::Int16(value) => PropertyValue::CreateInt16(*value),
        PropertyValueData::UInt16(value) => PropertyValue::CreateUInt16(*value),
        PropertyValueData::Int32(value) => PropertyValue::CreateInt32(*value),
        PropertyValueData::UInt32(value) => PropertyValue::CreateUInt32(*value),
        PropertyValueData::Int64(value) => PropertyValue::CreateInt64(*value),
        PropertyValueData::UInt64(value) => PropertyValue::CreateUInt64(*value),
        PropertyValueData::Single(value) => PropertyValue::CreateSingle(*value),
        PropertyValueData::Double(value) => PropertyValue::CreateDouble(*value),
        PropertyValueData::Char16(value) => PropertyValue::CreateChar16(*value),
        PropertyValueData::Boolean(value) => PropertyValue::CreateBoolean(*value),
        PropertyValueData::String(value) => PropertyValue::CreateString(&HSTRING::from(value)),
        PropertyValueData::Guid(value) => PropertyValue::CreateGuid(*value),
        PropertyValueData::DateTime(value) => PropertyValue::CreateDateTime(*value),
        PropertyValueData::TimeSpan(value) => PropertyValue::CreateTimeSpan(*value),
        PropertyValueData::Point(value) => PropertyValue::CreatePoint(*value),
        PropertyValueData::Size(value) => PropertyValue::CreateSize(*value),
        PropertyValueData::Rect(value) => PropertyValue::CreateRect(*value),
        PropertyValueData::UInt8Array(values) => PropertyValue::CreateUInt8Array(values),
        PropertyValueData::Int16Array(values) => PropertyValue::CreateInt16Array(values),
        PropertyValueData::UInt16Array(values) => PropertyValue::CreateUInt16Array(values),
        PropertyValueData::Int32Array(values) => PropertyValue::CreateInt32Array(values),
        PropertyValueData::UInt32Array(values) => PropertyValue::CreateUInt32Array(values),
        PropertyValueData::Int64Array(values) => PropertyValue::CreateInt64Array(values),
        PropertyValueData::UInt64Array(values) => PropertyValue::CreateUInt64Array(values),
        PropertyValueData::SingleArray(values) => PropertyValue::CreateSingleArray(values),
        PropertyValueData::DoubleArray(values) => PropertyValue::CreateDoubleArray(values),
        PropertyValueData::Char16Array(values) => PropertyValue::CreateChar16Array(values),
        PropertyValueData::BooleanArray(values) => PropertyValue::CreateBooleanArray(values),
        PropertyValueData::StringArray(values) => {
            PropertyValue::CreateStringArray(&values.iter().map(HSTRING::from).collect::<Vec<_>>())
        }
        PropertyValueData::InspectableArray(values) => {
            let values = values
                .iter()
                .map(|value| value.as_ref().map(Interface::cast).transpose())
                .collect::<windows_core::Result<Vec<Option<IInspectable>>>>()?;
            PropertyValue::CreateInspectableArray(&values)
        }
        PropertyValueData::DateTimeArray(values) => PropertyValue::CreateDateTimeArray(values),
        PropertyValueData::TimeSpanArray(values) => PropertyValue::CreateTimeSpanArray(values),
        PropertyValueData::GuidArray(values) => PropertyValue::CreateGuidArray(values),
        PropertyValueData::PointArray(values) => PropertyValue::CreatePointArray(values),
        PropertyValueData::SizeArray(values) => PropertyValue::CreateSizeArray(values),
        PropertyValueData::RectArray(values) => PropertyValue::CreateRectArray(values),
    }?;
    Ok(WinRTValue::Object(boxed.into()))
}

#[cfg(test)]
mod tests {
    use windows::Foundation::{IPropertyValue_Impl, Uri};
    use windows_core::{HSTRING, implement};

    use super::*;
    use crate::test_apartment::initialize_mta;

    const E_NOTIMPL: HRESULT = HRESULT(0x80004001_u32 as i32);

    fn as_value<T: Interface>(value: &T) -> WinRTValue {
        WinRTValue::Object(value.cast().expect("WinRT object as IUnknown"))
    }

    fn samples() -> Vec<PropertyValueData> {
        let uri: IUnknown = Uri::CreateUri(&HSTRING::from("https://example.com"))
            .unwrap()
            .cast()
            .unwrap();
        let boxed = box_property_value(&PropertyValueData::UInt32(7))
            .unwrap()
            .as_object()
            .unwrap();
        let guid = GUID::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
        let date = DateTime {
            UniversalTime: 133_000_000_000_000_001,
        };
        let span = TimeSpan {
            Duration: -12_345_678,
        };
        let point = Point { X: 1.5, Y: -2.25 };
        let size = Size {
            Width: 3.0,
            Height: 4.5,
        };
        let rect = Rect {
            X: 1.0,
            Y: 2.0,
            Width: 3.0,
            Height: 4.0,
        };
        vec![
            PropertyValueData::UInt8(u8::MAX),
            PropertyValueData::Int16(i16::MIN),
            PropertyValueData::UInt16(u16::MAX),
            PropertyValueData::Int32(i32::MIN),
            PropertyValueData::UInt32(u32::MAX),
            PropertyValueData::Int64(i64::MIN),
            PropertyValueData::UInt64(u64::MAX),
            PropertyValueData::Single(0.1),
            PropertyValueData::Double(-0.1),
            PropertyValueData::Char16(0xD800),
            PropertyValueData::Boolean(true),
            PropertyValueData::String("BLE Device".into()),
            PropertyValueData::Guid(guid),
            PropertyValueData::DateTime(date),
            PropertyValueData::TimeSpan(span),
            PropertyValueData::Point(point),
            PropertyValueData::Size(size),
            PropertyValueData::Rect(rect),
            PropertyValueData::UInt8Array(vec![0, 1, 127, 255]),
            PropertyValueData::Int16Array(vec![i16::MIN, 0, i16::MAX]),
            PropertyValueData::UInt16Array(vec![0, u16::MAX]),
            PropertyValueData::Int32Array(vec![-1, 0, 42]),
            PropertyValueData::UInt32Array(vec![0, u32::MAX]),
            PropertyValueData::Int64Array(vec![i64::MIN, i64::MAX]),
            PropertyValueData::UInt64Array(vec![0, u64::MAX]),
            PropertyValueData::SingleArray(vec![0.5, -1.25]),
            PropertyValueData::DoubleArray(vec![0.1, f64::MAX]),
            PropertyValueData::Char16Array(vec![0xD800, 0x61]),
            PropertyValueData::BooleanArray(vec![true, false]),
            PropertyValueData::StringArray(vec!["one".into(), String::new()]),
            PropertyValueData::InspectableArray(vec![Some(uri), None, Some(boxed)]),
            PropertyValueData::DateTimeArray(vec![date, DateTime::default()]),
            PropertyValueData::TimeSpanArray(vec![span]),
            PropertyValueData::GuidArray(vec![guid, GUID::zeroed()]),
            PropertyValueData::PointArray(vec![point]),
            PropertyValueData::SizeArray(vec![size, Size::default()]),
            PropertyValueData::RectArray(vec![rect]),
            PropertyValueData::Int32Array(Vec::new()),
            PropertyValueData::StringArray(Vec::new()),
        ]
    }

    #[test]
    fn round_trips_every_supported_property_type() {
        initialize_mta();

        let samples = samples();
        let mut covered = samples
            .iter()
            .map(|sample| sample.property_type().0)
            .collect::<Vec<_>>();
        covered.sort_unstable();
        covered.dedup();
        // Every PropertyType except Empty, Inspectable, OtherType and OtherTypeArray.
        assert_eq!(covered.len(), 37);

        for sample in samples {
            let boxed = box_property_value(&sample).unwrap();
            let property_value: IPropertyValue = boxed.as_object().unwrap().cast().unwrap();
            assert_eq!(property_value.Type().unwrap(), sample.property_type());
            assert_eq!(
                unbox_property_value(&boxed).unwrap(),
                PropertyValueUnboxResult::Value(sample.clone()),
                "{sample:?}"
            );
            let rebox = box_property_value(&match unbox_property_value(&boxed).unwrap() {
                PropertyValueUnboxResult::Value(data) => data,
                other => panic!("unexpected {other:?}"),
            })
            .unwrap();
            let property_value: IPropertyValue = rebox.as_object().unwrap().cast().unwrap();
            assert_eq!(property_value.Type().unwrap(), sample.property_type());
        }
    }

    #[test]
    fn preserves_null_and_non_property_objects() -> windows_core::Result<()> {
        initialize_mta();

        assert_eq!(
            unbox_property_value(&WinRTValue::Null).unwrap(),
            PropertyValueUnboxResult::Null
        );
        assert_eq!(
            unbox_property_value(&WinRTValue::I32(5)).unwrap(),
            PropertyValueUnboxResult::NotPropertyValue
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

    #[implement(IPropertyValue)]
    struct CustomPropertyValue(PropertyType);

    macro_rules! not_implemented {
        ($($getter:ident -> $typ:ty),* $(,)?) => {
            $(fn $getter(&self) -> windows_core::Result<$typ> {
                Err(E_NOTIMPL.into())
            })*
        };
    }

    macro_rules! not_implemented_arrays {
        ($($getter:ident: $typ:ty),* $(,)?) => {
            $(fn $getter(&self, _value: &mut Array<$typ>) -> windows_core::Result<()> {
                Err(E_NOTIMPL.into())
            })*
        };
    }

    impl IPropertyValue_Impl for CustomPropertyValue_Impl {
        fn Type(&self) -> windows_core::Result<PropertyType> {
            Ok(self.0)
        }

        not_implemented!(
            IsNumericScalar -> bool, GetUInt8 -> u8, GetInt16 -> i16, GetUInt16 -> u16,
            GetInt32 -> i32, GetUInt32 -> u32, GetInt64 -> i64, GetUInt64 -> u64,
            GetSingle -> f32, GetDouble -> f64, GetChar16 -> u16, GetBoolean -> bool,
            GetString -> HSTRING, GetGuid -> GUID, GetDateTime -> DateTime,
            GetTimeSpan -> TimeSpan, GetPoint -> Point, GetSize -> Size, GetRect -> Rect,
        );

        not_implemented_arrays!(
            GetUInt8Array: u8, GetInt16Array: i16, GetUInt16Array: u16, GetInt32Array: i32,
            GetUInt32Array: u32, GetInt64Array: i64, GetUInt64Array: u64, GetSingleArray: f32,
            GetDoubleArray: f64, GetChar16Array: u16, GetBooleanArray: bool,
            GetStringArray: HSTRING, GetInspectableArray: IInspectable, GetGuidArray: GUID,
            GetDateTimeArray: DateTime, GetTimeSpanArray: TimeSpan, GetPointArray: Point,
            GetSizeArray: Size, GetRectArray: Rect,
        );
    }

    fn custom(property_type: PropertyType) -> WinRTValue {
        let value: IPropertyValue = CustomPropertyValue(property_type).into();
        as_value(&value)
    }

    #[test]
    fn classifies_payloadless_property_types_without_failing() {
        initialize_mta();

        for property_type in [
            PropertyType::Empty,
            PropertyType::Inspectable,
            PropertyType::OtherType,
            PropertyType::OtherTypeArray,
            PropertyType(0x7fff),
        ] {
            assert_eq!(
                unbox_property_value(&custom(property_type)).unwrap(),
                PropertyValueUnboxResult::Unsupported(property_type)
            );
        }
    }

    #[test]
    fn surfaces_getter_failures_for_supported_property_types() {
        initialize_mta();

        let error = unbox_property_value(&custom(PropertyType::UInt32)).unwrap_err();
        assert!(matches!(error, Error::WindowsError(ref error) if error.code() == E_NOTIMPL));
    }
}
