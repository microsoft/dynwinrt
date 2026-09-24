// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Explicit conversion between Python values and WinRT `Object`
//! (`IInspectable`) values.
//!
//! Generated code keeps `Object` values native. Applications convert only
//! where they expect value semantics:
//! - [`unbox_object`] reads a boxed `IPropertyValue` payload;
//! - [`to_winrt_object`] boxes a Python value as a system
//!   `Windows.Foundation.PropertyValue`.
//!
//! No WinRT type is guessed. A plain `int` boxes only as Int32, a list boxes
//! only when its elements share one unambiguous type, and enum members, empty
//! or mixed lists and `InspectableArray` values need an explicit type. The
//! explicit value types live in `python/dynwinrt/values.py`; the
//! language-neutral payload is `dynwinrt::PropertyValueData`.

use std::os::raw::c_int;

use dynwinrt::{PropertyValueData as Data, PropertyValueUnboxResult, WinRTValue};
use pyo3::IntoPyObjectExt;
use pyo3::exceptions::{PyOverflowError, PyRecursionError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{
    PyBool, PyByteArray, PyBytes, PyDict, PyFloat, PyInt, PyList, PyMemoryView, PyString, PyTuple,
    PyType,
};
use pyo3::{ffi, intern};
use windows::Foundation::{DateTime, Point, PropertyType, Rect, Size, TimeSpan};
use windows::core::{GUID, HRESULT, IUnknown};

use crate::errors::{map_dynwinrt_error, map_windows_error};
use crate::runtime::{DynWinRTValue, WinGUID};

const E_NOTIMPL: HRESULT = HRESULT(0x80004001_u32 as i32);

/// Nested `InspectableArray` levels converted in either direction.
const MAX_NESTING: usize = 64;

const SUPPORTED_INPUTS: &str = "None, a DynWinRTValue or projected WinRT object, bool, \
an int in the Int32 range, float, str, a timezone-aware datetime.datetime, datetime.timedelta, \
uuid.UUID, WinGUID, bytes, bytearray, memoryview, a dynwinrt.values tag, Point, Size, Rect or \
typed array, or a non-empty list or tuple whose elements share one of those scalar types";

const EMPTY_SEQUENCE: &str = "cannot infer the WinRT array type of an empty list or tuple; pass \
property_type= (for example dynwinrt.values.PropertyType.StringArray) or a typed array such as \
dynwinrt.values.StringArray([])";

// ======================================================================
// WinRT value kinds
// ======================================================================

/// A scalar `PropertyType`, or the element type of an array `PropertyType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Element {
    UInt8,
    Int16,
    UInt16,
    Int32,
    UInt32,
    Int64,
    UInt64,
    Single,
    Double,
    Char16,
    Boolean,
    String,
    Inspectable,
    DateTime,
    TimeSpan,
    Guid,
    Point,
    Size,
    Rect,
}

impl Element {
    const ALL: [Self; 19] = [
        Self::UInt8,
        Self::Int16,
        Self::UInt16,
        Self::Int32,
        Self::UInt32,
        Self::Int64,
        Self::UInt64,
        Self::Single,
        Self::Double,
        Self::Char16,
        Self::Boolean,
        Self::String,
        Self::Inspectable,
        Self::DateTime,
        Self::TimeSpan,
        Self::Guid,
        Self::Point,
        Self::Size,
        Self::Rect,
    ];

    /// Elements with a `dynwinrt.values` tag class.
    const TAGS: [Self; 10] = [
        Self::UInt8,
        Self::Int16,
        Self::UInt16,
        Self::Int32,
        Self::UInt32,
        Self::Int64,
        Self::UInt64,
        Self::Single,
        Self::Double,
        Self::Char16,
    ];

    const GEOMETRY: [Self; 3] = [Self::Point, Self::Size, Self::Rect];

    fn name(self) -> &'static str {
        match self {
            Self::UInt8 => "UInt8",
            Self::Int16 => "Int16",
            Self::UInt16 => "UInt16",
            Self::Int32 => "Int32",
            Self::UInt32 => "UInt32",
            Self::Int64 => "Int64",
            Self::UInt64 => "UInt64",
            Self::Single => "Single",
            Self::Double => "Double",
            Self::Char16 => "Char16",
            Self::Boolean => "Boolean",
            Self::String => "String",
            Self::Inspectable => "Inspectable",
            Self::DateTime => "DateTime",
            Self::TimeSpan => "TimeSpan",
            Self::Guid => "Guid",
            Self::Point => "Point",
            Self::Size => "Size",
            Self::Rect => "Rect",
        }
    }

    fn scalar_type(self) -> PropertyType {
        match self {
            Self::UInt8 => PropertyType::UInt8,
            Self::Int16 => PropertyType::Int16,
            Self::UInt16 => PropertyType::UInt16,
            Self::Int32 => PropertyType::Int32,
            Self::UInt32 => PropertyType::UInt32,
            Self::Int64 => PropertyType::Int64,
            Self::UInt64 => PropertyType::UInt64,
            Self::Single => PropertyType::Single,
            Self::Double => PropertyType::Double,
            Self::Char16 => PropertyType::Char16,
            Self::Boolean => PropertyType::Boolean,
            Self::String => PropertyType::String,
            Self::Inspectable => PropertyType::Inspectable,
            Self::DateTime => PropertyType::DateTime,
            Self::TimeSpan => PropertyType::TimeSpan,
            Self::Guid => PropertyType::Guid,
            Self::Point => PropertyType::Point,
            Self::Size => PropertyType::Size,
            Self::Rect => PropertyType::Rect,
        }
    }

    fn array_type(self) -> PropertyType {
        match self {
            Self::UInt8 => PropertyType::UInt8Array,
            Self::Int16 => PropertyType::Int16Array,
            Self::UInt16 => PropertyType::UInt16Array,
            Self::Int32 => PropertyType::Int32Array,
            Self::UInt32 => PropertyType::UInt32Array,
            Self::Int64 => PropertyType::Int64Array,
            Self::UInt64 => PropertyType::UInt64Array,
            Self::Single => PropertyType::SingleArray,
            Self::Double => PropertyType::DoubleArray,
            Self::Char16 => PropertyType::Char16Array,
            Self::Boolean => PropertyType::BooleanArray,
            Self::String => PropertyType::StringArray,
            Self::Inspectable => PropertyType::InspectableArray,
            Self::DateTime => PropertyType::DateTimeArray,
            Self::TimeSpan => PropertyType::TimeSpanArray,
            Self::Guid => PropertyType::GuidArray,
            Self::Point => PropertyType::PointArray,
            Self::Size => PropertyType::SizeArray,
            Self::Rect => PropertyType::RectArray,
        }
    }

    fn integer_bounds(self) -> Option<(i128, i128)> {
        match self {
            Self::UInt8 => Some((0, u8::MAX.into())),
            Self::Int16 => Some((i16::MIN.into(), i16::MAX.into())),
            Self::UInt16 => Some((0, u16::MAX.into())),
            Self::Int32 => Some((i32::MIN.into(), i32::MAX.into())),
            Self::UInt32 => Some((0, u32::MAX.into())),
            Self::Int64 => Some((i64::MIN.into(), i64::MAX.into())),
            Self::UInt64 => Some((0, u64::MAX.into())),
            _ => None,
        }
    }

    fn fields(self) -> &'static [&'static str] {
        match self {
            Self::Point => &["x", "y"],
            Self::Size => &["width", "height"],
            Self::Rect => &["x", "y", "width", "height"],
            _ => &[],
        }
    }
}

/// A `PropertyType` that `to_winrt_object` can create.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Scalar(Element),
    Array(Element),
}

impl Target {
    fn from_property_type(property_type: PropertyType) -> Option<Self> {
        Element::ALL.into_iter().find_map(|element| {
            if element != Element::Inspectable && element.scalar_type() == property_type {
                Some(Self::Scalar(element))
            } else if element.array_type() == property_type {
                Some(Self::Array(element))
            } else {
                None
            }
        })
    }
}

fn payloadless_name(property_type: PropertyType) -> Option<&'static str> {
    match property_type {
        PropertyType::Empty => Some("Empty"),
        PropertyType::Inspectable => Some("Inspectable"),
        PropertyType::OtherType => Some("OtherType"),
        PropertyType::OtherTypeArray => Some("OtherTypeArray"),
        _ => None,
    }
}

// ======================================================================
// The dynwinrt.values model
// ======================================================================

/// The `dynwinrt.values` types and the helpers conversions rely on.
struct ValueModel {
    tags: Vec<(Py<PyType>, Element)>,
    scalar_base: Py<PyType>,
    arrays: Vec<(Py<PyType>, Element)>,
    array_base: Py<PyType>,
    geometry: Vec<(Py<PyType>, Element)>,
    enum_base: Py<PyType>,
    uuid: Py<PyType>,
    datetime: Py<PyType>,
    timedelta: Py<PyType>,
    int_new: Py<PyAny>,
    float_new: Py<PyAny>,
    str_new: Py<PyAny>,
    ticks_to_datetime: Py<PyAny>,
    datetime_to_ticks: Py<PyAny>,
    ticks_to_timedelta: Py<PyAny>,
    timedelta_to_ticks: Py<PyAny>,
}

static MODEL: PyOnceLock<ValueModel> = PyOnceLock::new();

/// The value model, loaded on first use so that importing the native module
/// never depends on `dynwinrt.values` being importable yet.
fn model(module: &Bound<'_, PyModule>) -> PyResult<&'static ValueModel> {
    MODEL.get_or_try_init(module.py(), || ValueModel::load(module))
}

impl ValueModel {
    fn load(module: &Bound<'_, PyModule>) -> PyResult<Self> {
        let py = module.py();
        let values = py.import("dynwinrt.values")?;
        let class = |name: &str| -> PyResult<Py<PyType>> {
            Ok(values.getattr(name)?.cast_into::<PyType>()?.unbind())
        };
        let imported = |module: &str, name: &str| -> PyResult<Py<PyType>> {
            Ok(py
                .import(module)?
                .getattr(name)?
                .cast_into::<PyType>()?
                .unbind())
        };
        let constructor = |typ: Bound<'_, PyType>| -> PyResult<Py<PyAny>> {
            Ok(typ.getattr(intern!(py, "__new__"))?.unbind())
        };
        let helper = |name: &str| -> PyResult<Py<PyAny>> { Ok(module.getattr(name)?.unbind()) };
        Ok(Self {
            tags: Element::TAGS
                .into_iter()
                .map(|element| Ok((class(element.name())?, element)))
                .collect::<PyResult<_>>()?,
            scalar_base: class("WinRTScalar")?,
            arrays: Element::ALL
                .into_iter()
                .filter(|element| *element != Element::UInt8)
                .map(|element| Ok((class(&format!("{}Array", element.name()))?, element)))
                .collect::<PyResult<_>>()?,
            array_base: class("WinRTArray")?,
            geometry: Element::GEOMETRY
                .into_iter()
                .map(|element| Ok((class(element.name())?, element)))
                .collect::<PyResult<_>>()?,
            enum_base: imported("enum", "Enum")?,
            uuid: imported("uuid", "UUID")?,
            datetime: imported("datetime", "datetime")?,
            timedelta: imported("datetime", "timedelta")?,
            int_new: constructor(py.get_type::<PyInt>())?,
            float_new: constructor(py.get_type::<PyFloat>())?,
            str_new: constructor(py.get_type::<PyString>())?,
            ticks_to_datetime: helper("_dynwinrt_ticks_to_datetime")?,
            datetime_to_ticks: helper("_dynwinrt_datetime_to_ticks")?,
            ticks_to_timedelta: helper("_dynwinrt_ticks_to_timedelta")?,
            timedelta_to_ticks: helper("_dynwinrt_timedelta_to_ticks")?,
        })
    }

    fn find(types: &[(Py<PyType>, Element)], element: Element) -> &Py<PyType> {
        types
            .iter()
            .find(|(_, candidate)| *candidate == element)
            .map(|(typ, _)| typ)
            .expect("every modeled element has a dynwinrt.values class")
    }

    /// The first class in `types` that `value` is an instance of.
    fn instance_of(
        types: &[(Py<PyType>, Element)],
        value: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Element>> {
        for (typ, element) in types {
            if value.is_instance(typ.bind(value.py()))? {
                return Ok(Some(*element));
            }
        }
        Ok(None)
    }

    fn is_enum(&self, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        value.is_instance(self.enum_base.bind(value.py()))
    }

    fn uuid(&self, py: Python<'_>, value: GUID) -> PyResult<Py<PyAny>> {
        let kwargs = PyDict::new(py);
        kwargs.set_item(intern!(py, "int"), value.to_u128())?;
        self.uuid.call(py, (), Some(&kwargs))
    }

    fn datetime(&self, py: Python<'_>, value: DateTime) -> PyResult<Py<PyAny>> {
        self.ticks_to_datetime
            .call1(py, (value.UniversalTime,))
            .map_err(|error| {
                if error.is_instance_of::<PyOverflowError>(py) {
                    PyOverflowError::new_err(format!(
                        "WinRT DateTime value ({} ticks) is outside the range of datetime.datetime",
                        value.UniversalTime
                    ))
                } else {
                    error
                }
            })
    }

    fn timedelta(&self, py: Python<'_>, value: TimeSpan) -> PyResult<Py<PyAny>> {
        self.ticks_to_timedelta.call1(py, (value.Duration,))
    }

    fn geometry(&self, py: Python<'_>, element: Element, fields: &[f32]) -> PyResult<Py<PyAny>> {
        let fields = PyTuple::new(py, fields.iter().map(|field| f64::from(*field)))?;
        Self::find(&self.geometry, element).call1(py, fields)
    }
}

fn type_name(value: &Bound<'_, PyAny>) -> String {
    if value.is_none() {
        return "None".into();
    }
    value
        .get_type()
        .qualname()
        .map(|name| name.to_string())
        .unwrap_or_else(|_| "value".into())
}

fn repr(value: &Bound<'_, PyAny>) -> String {
    value
        .repr()
        .map(|repr| repr.to_string())
        .unwrap_or_else(|_| type_name(value))
}

fn with_label(label: Option<&str>, message: String) -> String {
    match label {
        Some(label) => format!("{label}: {message}"),
        None => message,
    }
}

/// Prefix `context` to a conversion error raised for a nested value.
fn with_context(py: Python<'_>, error: PyErr, context: &str) -> PyErr {
    let message = format!("{context}: {}", error.value(py));
    let wrapped = if error.is_instance_of::<PyOverflowError>(py) {
        PyOverflowError::new_err(message)
    } else if error.is_instance_of::<PyTypeError>(py) {
        PyTypeError::new_err(message)
    } else if error.is_instance_of::<PyValueError>(py) {
        PyValueError::new_err(message)
    } else {
        return error;
    };
    wrapped.set_cause(py, Some(error));
    wrapped
}

/// The one place that creates the Python values these conversions return.
fn native(value: WinRTValue) -> DynWinRTValue {
    DynWinRTValue(value)
}

/// Borrow the native value of a `DynWinRTValue`.
///
/// Every native input, including nested elements, is read through here.
fn live<'py>(value: &Bound<'py, DynWinRTValue>) -> PyResult<PyRef<'py, DynWinRTValue>> {
    Ok(value.try_borrow()?)
}

fn is_object(value: &WinRTValue) -> bool {
    matches!(
        value,
        WinRTValue::Object(_) | WinRTValue::Null | WinRTValue::Async(_)
    )
}

fn value_kind(value: &WinRTValue) -> &'static str {
    match value {
        WinRTValue::Bool(_) => "Bool",
        WinRTValue::I8(_) => "I8",
        WinRTValue::U8(_) => "U8",
        WinRTValue::I16(_) => "I16",
        WinRTValue::U16(_) => "U16",
        WinRTValue::I32(_) => "I32",
        WinRTValue::U32(_) => "U32",
        WinRTValue::I64(_) => "I64",
        WinRTValue::U64(_) => "U64",
        WinRTValue::F32(_) => "F32",
        WinRTValue::F64(_) => "F64",
        WinRTValue::HString(_) => "HString",
        WinRTValue::HResult(_) => "HResult",
        WinRTValue::Guid(_) => "Guid",
        WinRTValue::Enum { .. } => "Enum",
        WinRTValue::Struct(_) => "Struct",
        WinRTValue::Array(_) | WinRTValue::ArrayOfIUnknown(_) => "Array",
        WinRTValue::RawPtr(_) | WinRTValue::OutValue(..) => "native pointer",
        WinRTValue::Object(_) | WinRTValue::Null | WinRTValue::Async(_) => "Object",
    }
}

/// The `DynWinRTValue` behind `value`: the value itself or a projected
/// wrapper's `_obj`.
fn native_input<'py>(value: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, DynWinRTValue>>> {
    if let Ok(raw) = value.cast::<DynWinRTValue>() {
        return Ok(Some(raw.clone()));
    }
    if let Some(inner) = value.getattr_opt(intern!(value.py(), "_obj"))?
        && let Ok(raw) = inner.cast_into::<DynWinRTValue>()
    {
        return Ok(Some(raw));
    }
    Ok(None)
}

fn unsupported_property_type(property_type: PropertyType) -> PyErr {
    let name = payloadless_name(property_type)
        .map(|name| format!(" ({name})"))
        .unwrap_or_default();
    map_windows_error(windows::core::Error::new(
        E_NOTIMPL,
        format!(
            "Unsupported WinRT IPropertyValue type: {}{name}",
            property_type.0
        ),
    ))
}

// ======================================================================
// unbox_object: DynWinRTValue -> Python value
// ======================================================================

/// Explicitly unbox a WinRT `IPropertyValue`.
///
/// `None` and WinRT null return `None`. Any other value that is not an
/// `IPropertyValue` is returned unchanged, preserving Python and COM identity.
/// Boxed values become Python values, and `InspectableArray` elements are
/// unboxed recursively. With `preserve_type`, scalars whose plain Python value
/// would box as a different `PropertyType` become `dynwinrt.values` tags and
/// arrays become typed arrays, so `to_winrt_object` restores the exact
/// `PropertyType` and value; the new box does not keep the original box's COM
/// identity. `Empty`, `Inspectable`, `OtherType` and `OtherTypeArray` boxes
/// raise `OSError`.
#[pyfunction]
#[pyo3(pass_module, signature = (value, *, preserve_type = false))]
pub fn unbox_object(
    module: &Bound<'_, PyModule>,
    value: &Bound<'_, PyAny>,
    preserve_type: bool,
) -> PyResult<Py<PyAny>> {
    let py = module.py();
    if value.is_none() {
        return Ok(py.None());
    }
    let Ok(raw) = value.cast::<DynWinRTValue>() else {
        return Err(PyTypeError::new_err(format!(
            "unbox_object() requires None or a DynWinRTValue, not {}",
            type_name(value)
        )));
    };
    Reader {
        model: model(module)?,
        preserve_type,
    }
    .unbox(raw, 0)
}

struct Reader<'a> {
    model: &'a ValueModel,
    preserve_type: bool,
}

impl Reader<'_> {
    fn unbox(&self, raw: &Bound<'_, DynWinRTValue>, depth: usize) -> PyResult<Py<PyAny>> {
        let py = raw.py();
        let result = {
            let native = live(raw)?;
            dynwinrt::unbox_property_value(&native.0).map_err(map_dynwinrt_error)?
        };
        match result {
            PropertyValueUnboxResult::Null => Ok(py.None()),
            PropertyValueUnboxResult::NotPropertyValue => Ok(raw.clone().into_any().unbind()),
            PropertyValueUnboxResult::Unsupported(property_type) => {
                Err(unsupported_property_type(property_type))
            }
            PropertyValueUnboxResult::Value(data) => self.read(py, data, depth),
        }
    }

    fn read(&self, py: Python<'_>, data: Data, depth: usize) -> PyResult<Py<PyAny>> {
        let model = self.model;
        let int = |value: i128| value.into_py_any(py);
        match data {
            Data::UInt8(value) => self.integer(py, Element::UInt8, value.into()),
            Data::Int16(value) => self.integer(py, Element::Int16, value.into()),
            Data::UInt16(value) => self.integer(py, Element::UInt16, value.into()),
            Data::Int32(value) => int(value.into()),
            Data::UInt32(value) => self.integer(py, Element::UInt32, value.into()),
            Data::Int64(value) => self.integer(py, Element::Int64, value.into()),
            Data::UInt64(value) => self.integer(py, Element::UInt64, value.into()),
            Data::Single(value) => {
                let value = f64::from(value);
                if self.preserve_type {
                    model
                        .float_new
                        .call1(py, (ValueModel::find(&model.tags, Element::Single), value))
                } else {
                    value.into_py_any(py)
                }
            }
            Data::Double(value) => value.into_py_any(py),
            Data::Char16(value) => {
                let character = char16(py, value)?;
                if self.preserve_type {
                    model.str_new.call1(
                        py,
                        (ValueModel::find(&model.tags, Element::Char16), character),
                    )
                } else {
                    Ok(character.unbind())
                }
            }
            Data::Boolean(value) => Ok(PyBool::new(py, value).to_owned().into_any().unbind()),
            Data::String(value) => value.into_py_any(py),
            Data::Guid(value) => model.uuid(py, value),
            Data::DateTime(value) => model.datetime(py, value),
            Data::TimeSpan(value) => model.timedelta(py, value),
            Data::Point(value) => model.geometry(py, Element::Point, &[value.X, value.Y]),
            Data::Size(value) => model.geometry(py, Element::Size, &[value.Width, value.Height]),
            Data::Rect(value) => model.geometry(
                py,
                Element::Rect,
                &[value.X, value.Y, value.Width, value.Height],
            ),
            Data::UInt8Array(values) => Ok(PyBytes::new(py, &values).into_any().unbind()),
            Data::Int16Array(values) => self.array(
                py,
                Element::Int16,
                values.into_iter().map(|value| int(value.into())),
            ),
            Data::UInt16Array(values) => self.array(
                py,
                Element::UInt16,
                values.into_iter().map(|value| int(value.into())),
            ),
            Data::Int32Array(values) => self.array(
                py,
                Element::Int32,
                values.into_iter().map(|value| int(value.into())),
            ),
            Data::UInt32Array(values) => self.array(
                py,
                Element::UInt32,
                values.into_iter().map(|value| int(value.into())),
            ),
            Data::Int64Array(values) => self.array(
                py,
                Element::Int64,
                values.into_iter().map(|value| int(value.into())),
            ),
            Data::UInt64Array(values) => self.array(
                py,
                Element::UInt64,
                values.into_iter().map(|value| int(value.into())),
            ),
            Data::SingleArray(values) => self.array(
                py,
                Element::Single,
                values
                    .into_iter()
                    .map(|value| f64::from(value).into_py_any(py)),
            ),
            Data::DoubleArray(values) => self.array(
                py,
                Element::Double,
                values.into_iter().map(|value| value.into_py_any(py)),
            ),
            Data::Char16Array(values) => self.array(
                py,
                Element::Char16,
                values
                    .into_iter()
                    .map(|value| Ok(char16(py, value)?.unbind())),
            ),
            Data::BooleanArray(values) => self.array(
                py,
                Element::Boolean,
                values
                    .into_iter()
                    .map(|value| Ok(PyBool::new(py, value).to_owned().into_any().unbind())),
            ),
            Data::StringArray(values) => self.array(
                py,
                Element::String,
                values.into_iter().map(|value| value.into_py_any(py)),
            ),
            Data::InspectableArray(values) => {
                if depth >= MAX_NESTING {
                    return Err(PyRecursionError::new_err(format!(
                        "WinRT InspectableArray values nest more than {MAX_NESTING} levels"
                    )));
                }
                self.array(
                    py,
                    Element::Inspectable,
                    values.into_iter().map(|element| match element {
                        None => Ok(py.None()),
                        Some(object) => {
                            let element = Bound::new(py, native(WinRTValue::Object(object)))?;
                            self.unbox(&element, depth + 1)
                        }
                    }),
                )
            }
            Data::DateTimeArray(values) => self.array(
                py,
                Element::DateTime,
                values.into_iter().map(|value| model.datetime(py, value)),
            ),
            Data::TimeSpanArray(values) => self.array(
                py,
                Element::TimeSpan,
                values.into_iter().map(|value| model.timedelta(py, value)),
            ),
            Data::GuidArray(values) => self.array(
                py,
                Element::Guid,
                values.into_iter().map(|value| model.uuid(py, value)),
            ),
            Data::PointArray(values) => self.array(
                py,
                Element::Point,
                values
                    .into_iter()
                    .map(|value| model.geometry(py, Element::Point, &[value.X, value.Y])),
            ),
            Data::SizeArray(values) => self.array(
                py,
                Element::Size,
                values
                    .into_iter()
                    .map(|value| model.geometry(py, Element::Size, &[value.Width, value.Height])),
            ),
            Data::RectArray(values) => self.array(
                py,
                Element::Rect,
                values.into_iter().map(|value| {
                    model.geometry(
                        py,
                        Element::Rect,
                        &[value.X, value.Y, value.Width, value.Height],
                    )
                }),
            ),
        }
    }

    /// A plain `int`, or the element's tag when preserving types.
    fn integer(&self, py: Python<'_>, element: Element, value: i128) -> PyResult<Py<PyAny>> {
        if self.preserve_type && element != Element::Int32 {
            // int.__new__(Tag, value) skips the tag's range check: WinRT
            // values are in range by construction.
            self.model
                .int_new
                .call1(py, (ValueModel::find(&self.model.tags, element), value))
        } else {
            value.into_py_any(py)
        }
    }

    /// A plain list of array elements, or the typed array when preserving types.
    fn array(
        &self,
        py: Python<'_>,
        element: Element,
        items: impl IntoIterator<Item = PyResult<Py<PyAny>>>,
    ) -> PyResult<Py<PyAny>> {
        let list = PyList::new(py, items.into_iter().collect::<PyResult<Vec<_>>>()?)?;
        if self.preserve_type {
            ValueModel::find(&self.model.arrays, element).call1(py, (list,))
        } else {
            Ok(list.into_any().unbind())
        }
    }
}

/// One UTF-16 code unit as a Python `str`, keeping unpaired surrogates.
fn char16(py: Python<'_>, value: u16) -> PyResult<Bound<'_, PyAny>> {
    unsafe { Bound::from_owned_ptr_or_err(py, ffi::PyUnicode_FromOrdinal(c_int::from(value))) }
}

// ======================================================================
// to_winrt_object: Python value -> DynWinRTValue
// ======================================================================

/// Explicitly box a Python value as a WinRT `Object`.
///
/// `None` becomes WinRT null, and a `DynWinRTValue` object or projected
/// wrapper is returned as its existing `DynWinRTValue`. Other values box as a
/// system `Windows.Foundation.PropertyValue` only when their WinRT type is
/// unambiguous: `bool`, `float`, `str`, timezone-aware `datetime`,
/// `timedelta`, `uuid.UUID`/`WinGUID`, bytes-like values, `dynwinrt.values`
/// tags, typed arrays and geometry, a plain `int` as Int32 only, and
/// homogeneous lists or tuples of those. Enum members, empty or mixed lists
/// and out-of-range ints raise. `property_type` converts a Python value to
/// exactly that `PropertyType`, validating its range.
#[pyfunction]
#[pyo3(pass_module, signature = (value, property_type = None))]
pub fn to_winrt_object(
    module: &Bound<'_, PyModule>,
    value: &Bound<'_, PyAny>,
    property_type: Option<&Bound<'_, PyAny>>,
) -> PyResult<Py<PyAny>> {
    let py = module.py();
    let boxer = Boxer {
        model: model(module)?,
    };
    let boxed = match property_type {
        None => boxer.box_value(value, 0)?,
        Some(property_type) => boxer.box_as(value, parse_property_type(property_type)?)?,
    };
    match boxed {
        Boxed::Existing(object) => Ok(object.into_any().unbind()),
        Boxed::New(value) => Ok(Bound::new(py, native(value))?.into_any().unbind()),
    }
}

fn parse_property_type(value: &Bound<'_, PyAny>) -> PyResult<Target> {
    if value.cast::<PyBool>().is_ok() || !value.is_instance_of::<PyInt>() {
        return Err(PyTypeError::new_err(format!(
            "property_type must be a dynwinrt.values.PropertyType member or an int, not {}",
            type_name(value)
        )));
    }
    let unknown = || {
        PyValueError::new_err(format!(
            "{} is not a Windows.Foundation.PropertyType value",
            repr(value)
        ))
    };
    let property_type = PropertyType(value.extract::<i32>().map_err(|_| unknown())?);
    if let Some(target) = Target::from_property_type(property_type) {
        return Ok(target);
    }
    Err(match property_type {
        PropertyType::Empty => PyValueError::new_err(
            "PropertyType.Empty is WinRT null; pass None without property_type",
        ),
        PropertyType::Inspectable => PyValueError::new_err(
            "PropertyType.Inspectable is not a boxed value; pass the WinRT object without \
             property_type",
        ),
        PropertyType::OtherType | PropertyType::OtherTypeArray => PyValueError::new_err(format!(
            "PropertyType.{} has no language-neutral payload and cannot be created by \
                 to_winrt_object()",
            payloadless_name(property_type).unwrap_or_default()
        )),
        _ => unknown(),
    })
}

enum Boxed<'py> {
    /// An existing WinRT object, returned as the same Python object.
    Existing(Bound<'py, DynWinRTValue>),
    /// WinRT null or a new box.
    New(WinRTValue),
}

/// How the default rules read a Python value that is not `None`, a native
/// value or an exact `bool`, `int`, `float` or `str`.
enum Kind {
    /// A value with exactly one WinRT scalar type.
    Scalar(Element),
    /// A plain `int` (or subclass), which boxes only as Int32.
    Int,
    /// `bytes`, `bytearray` or `memoryview`.
    Bytes,
    /// A `dynwinrt.values` typed array.
    TypedArray(Element),
    /// A `list` or `tuple`, boxed only when its elements are homogeneous.
    Sequence,
    /// An enum member, which needs an explicit integer `property_type`.
    Enum,
}

struct Boxer<'a> {
    model: &'a ValueModel,
}

impl Boxer<'_> {
    fn new_box<'py>(&self, data: &Data) -> PyResult<Boxed<'py>> {
        dynwinrt::box_property_value(data)
            .map(Boxed::New)
            .map_err(map_dynwinrt_error)
    }

    /// Box `value` by the default rules, in this order: None, native values
    /// and wrappers, bool, tags, enums (rejected), int (Int32 only), float,
    /// str, datetime, timedelta, UUID/WinGUID, bytes-like values, dynwinrt
    /// geometry, typed arrays, then homogeneous lists and tuples.
    fn box_value<'py>(&self, value: &Bound<'py, PyAny>, depth: usize) -> PyResult<Boxed<'py>> {
        // Exact built-in scalars cannot match an earlier rule.
        if value.is_exact_instance_of::<PyInt>() {
            return self.new_box(&plain_int(value)?);
        }
        if value.is_exact_instance_of::<PyString>() {
            return self.new_box(&Data::String(value.extract()?));
        }
        if value.is_exact_instance_of::<PyFloat>() {
            return self.new_box(&Data::Double(value.extract()?));
        }
        if let Ok(boolean) = value.cast::<PyBool>() {
            return self.new_box(&Data::Boolean(boolean.is_true()));
        }
        if value.is_none() {
            return Ok(Boxed::New(WinRTValue::Null));
        }
        if let Some(raw) = native_input(value)? {
            let kind = {
                let native = live(&raw)?;
                (!is_object(&native.0)).then(|| value_kind(&native.0))
            };
            return match kind {
                None => Ok(Boxed::Existing(raw)),
                Some(kind) => Err(PyTypeError::new_err(format!(
                    "to_winrt_object() received a DynWinRTValue holding a non-object {kind} \
                     value; pass the Python value instead, with a dynwinrt.values tag or \
                     property_type= to choose its WinRT type"
                ))),
            };
        }
        match self.classify(value)? {
            Some(Kind::Scalar(element)) => self.new_box(&self.scalar(value, element, None)?),
            Some(Kind::Int) => self.new_box(&plain_int(value)?),
            Some(Kind::Bytes) => self.new_box(&Data::UInt8Array(bytes_like(value)?)),
            Some(Kind::TypedArray(element)) => {
                self.new_box(&self.array(&items(value)?, element, depth)?)
            }
            Some(Kind::Sequence) => self.new_box(&self.infer_array(&items(value)?, depth)?),
            Some(Kind::Enum) => Err(PyTypeError::new_err(format!(
                "to_winrt_object() does not box the enum member {} without property_type: a WinRT \
                 enum boxes as IReference<T>, which is not supported yet. To box its integer \
                 value, pass an integer property_type such as \
                 dynwinrt.values.PropertyType.Int32 (UInt32 for flags), or int(value)",
                repr(value)
            ))),
            None => Err(PyTypeError::new_err(format!(
                "cannot convert {} to a WinRT Object; expected {SUPPORTED_INPUTS}. Pass \
                 property_type= to convert other values, such as a generated \
                 Windows.Foundation.Point, explicitly",
                type_name(value)
            ))),
        }
    }

    fn classify(&self, value: &Bound<'_, PyAny>) -> PyResult<Option<Kind>> {
        let py = value.py();
        let model = self.model;
        if value.is_instance(model.scalar_base.bind(py))? {
            return match ValueModel::instance_of(&model.tags, value)? {
                Some(element) => Ok(Some(Kind::Scalar(element))),
                None => Err(PyTypeError::new_err(format!(
                    "{} is not a dynwinrt.values tag; use a tag such as dynwinrt.values.UInt32",
                    type_name(value)
                ))),
            };
        }
        if model.is_enum(value)? {
            return Ok(Some(Kind::Enum));
        }
        if value.is_instance_of::<PyInt>() {
            return Ok(Some(Kind::Int));
        }
        if value.is_instance_of::<PyFloat>() {
            return Ok(Some(Kind::Scalar(Element::Double)));
        }
        if value.is_instance_of::<PyString>() {
            return Ok(Some(Kind::Scalar(Element::String)));
        }
        if value.is_instance(model.datetime.bind(py))? {
            return Ok(Some(Kind::Scalar(Element::DateTime)));
        }
        if value.is_instance(model.timedelta.bind(py))? {
            return Ok(Some(Kind::Scalar(Element::TimeSpan)));
        }
        if value.is_instance(model.uuid.bind(py))? || value.cast::<WinGUID>().is_ok() {
            return Ok(Some(Kind::Scalar(Element::Guid)));
        }
        if value.is_instance_of::<PyBytes>()
            || value.is_instance_of::<PyByteArray>()
            || value.is_instance_of::<PyMemoryView>()
        {
            return Ok(Some(Kind::Bytes));
        }
        if let Some(element) = ValueModel::instance_of(&model.geometry, value)? {
            return Ok(Some(Kind::Scalar(element)));
        }
        if value.is_instance(model.array_base.bind(py))? {
            return match ValueModel::instance_of(&model.arrays, value)? {
                Some(element) => Ok(Some(Kind::TypedArray(element))),
                None => Err(PyTypeError::new_err(format!(
                    "{} is not a dynwinrt.values typed array; use one such as \
                     dynwinrt.values.Int32Array",
                    type_name(value)
                ))),
            };
        }
        if value.is_instance_of::<PyList>() || value.is_instance_of::<PyTuple>() {
            return Ok(Some(Kind::Sequence));
        }
        Ok(None)
    }

    /// The array element type a list item implies, if it is unambiguous.
    fn element_kind(&self, item: &Bound<'_, PyAny>) -> PyResult<Option<Element>> {
        if item.is_exact_instance_of::<PyInt>() {
            return Ok(Some(Element::Int32));
        }
        if item.is_exact_instance_of::<PyString>() {
            return Ok(Some(Element::String));
        }
        if item.is_exact_instance_of::<PyFloat>() {
            return Ok(Some(Element::Double));
        }
        if item.cast::<PyBool>().is_ok() {
            return Ok(Some(Element::Boolean));
        }
        if item.is_none() || native_input(item)?.is_some() {
            return Ok(None);
        }
        Ok(match self.classify(item)? {
            Some(Kind::Scalar(element)) => Some(element),
            Some(Kind::Int) => Some(Element::Int32),
            _ => None,
        })
    }

    /// Box a homogeneous list or tuple as the matching array type.
    fn infer_array(&self, items: &[Bound<'_, PyAny>], depth: usize) -> PyResult<Data> {
        let Some(first) = items.first() else {
            return Err(PyTypeError::new_err(EMPTY_SEQUENCE));
        };
        let mut inferred: Option<Element> = None;
        for (index, item) in items.iter().enumerate() {
            let Some(element) = self.element_kind(item)? else {
                return Err(PyTypeError::new_err(format!(
                    "cannot infer a WinRT array type for a list containing {} (element {index}); \
                     a list boxes only when all of its elements have the same unambiguous type. \
                     Pass property_type= (for example dynwinrt.values.PropertyType.\
                     InspectableArray) or a typed array such as \
                     dynwinrt.values.InspectableArray([...])",
                    type_name(item)
                )));
            };
            match inferred {
                None => inferred = Some(element),
                Some(existing) if existing == element => {}
                Some(_) => {
                    return Err(PyTypeError::new_err(format!(
                        "cannot infer a WinRT array type for a list mixing {} (element 0) and {} \
                         (element {index}); pass property_type= or a typed array such as \
                         dynwinrt.values.InspectableArray([...])",
                        type_name(first),
                        type_name(item)
                    )));
                }
            }
        }
        let element = inferred.expect("a non-empty list infers an element type");
        if element != Element::Int32 {
            return self.array(items, element, depth);
        }
        // Plain ints box only as Int32Array; Int32 tags are exact.
        items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let label = format!("element {index}");
                let data = if item.is_instance(self.model.scalar_base.bind(item.py()))? {
                    self.scalar(item, Element::Int32, Some(&label))?
                } else {
                    plain_int_element(item, index)?
                };
                match data {
                    Data::Int32(number) => Ok(number),
                    _ => unreachable!("an Int32 conversion returns Int32"),
                }
            })
            .collect::<PyResult<_>>()
            .map(Data::Int32Array)
    }

    /// Box a Python value as exactly `target`.
    fn box_as<'py>(&self, value: &Bound<'py, PyAny>, target: Target) -> PyResult<Boxed<'py>> {
        if value.is_none() {
            return Err(PyTypeError::new_err(
                "property_type= converts a Python value; pass None without property_type for \
                 WinRT null",
            ));
        }
        if native_input(value)?.is_some() {
            return Err(PyTypeError::new_err(format!(
                "property_type= converts a Python value; {} is already a WinRT object, so pass \
                 it without property_type",
                type_name(value)
            )));
        }
        let data = match target {
            Target::Scalar(element) => self.scalar(value, element, None)?,
            Target::Array(element) => {
                let bytes = element == Element::UInt8
                    && (value.is_instance_of::<PyBytes>()
                        || value.is_instance_of::<PyByteArray>()
                        || value.is_instance_of::<PyMemoryView>());
                if bytes {
                    Data::UInt8Array(bytes_like(value)?)
                } else if value.is_instance_of::<PyList>() || value.is_instance_of::<PyTuple>() {
                    self.array(&items(value)?, element, 0)?
                } else {
                    return Err(PyTypeError::new_err(format!(
                        "PropertyType.{}Array requires a list or tuple{}, not {}",
                        element.name(),
                        if element == Element::UInt8 {
                            ", bytes, bytearray or memoryview"
                        } else {
                            ""
                        },
                        type_name(value)
                    )));
                }
            }
        };
        self.new_box(&data)
    }

    /// Convert one Python value to exactly `element`. `label` names the
    /// enclosing array element in error messages.
    fn scalar(
        &self,
        value: &Bound<'_, PyAny>,
        element: Element,
        label: Option<&str>,
    ) -> PyResult<Data> {
        let py = value.py();
        let model = self.model;
        let type_error = |expected: &str| {
            PyTypeError::new_err(with_label(
                label,
                format!(
                    "WinRT {} requires {expected}, not {}",
                    element.name(),
                    type_name(value)
                ),
            ))
        };
        if let Some((low, high)) = element.integer_bounds() {
            let number = self.integer(value, element, label)?;
            if number < low || number > high {
                return Err(PyOverflowError::new_err(with_label(
                    label,
                    format!(
                        "{number} is out of range for WinRT {} ({low}..{high})",
                        element.name()
                    ),
                )));
            }
            return Ok(match element {
                Element::UInt8 => Data::UInt8(number as u8),
                Element::Int16 => Data::Int16(number as i16),
                Element::UInt16 => Data::UInt16(number as u16),
                Element::Int32 => Data::Int32(number as i32),
                Element::UInt32 => Data::UInt32(number as u32),
                Element::Int64 => Data::Int64(number as i64),
                _ => Data::UInt64(number as u64),
            });
        }
        if model.is_enum(value)? {
            return Err(PyTypeError::new_err(with_label(
                label,
                format!(
                    "WinRT {} cannot box the enum member {}; enum members box only with an \
                     integer PropertyType",
                    element.name(),
                    repr(value)
                ),
            )));
        }
        Ok(match element {
            Element::Single => Data::Single(to_f32(
                value,
                real(value, "WinRT Single", label)?,
                "WinRT Single",
                label,
            )?),
            Element::Double => Data::Double(real(value, "WinRT Double", label)?),
            Element::Char16 => Data::Char16(char16_unit(value, label)?),
            Element::Boolean => match value.cast::<PyBool>() {
                Ok(boolean) => Data::Boolean(boolean.is_true()),
                Err(_) => return Err(type_error("a bool")),
            },
            Element::String => {
                if !value.is_instance_of::<PyString>() {
                    return Err(type_error("a str"));
                }
                Data::String(value.extract()?)
            }
            Element::DateTime => {
                if !value.is_instance(model.datetime.bind(py))? {
                    return Err(type_error("a timezone-aware datetime.datetime"));
                }
                if value.call_method0(intern!(py, "utcoffset"))?.is_none() {
                    return Err(PyValueError::new_err(with_label(
                        label,
                        "WinRT DateTime requires a timezone-aware datetime.datetime, not a naive \
                         one; attach tzinfo, for example datetime.now(timezone.utc)"
                            .into(),
                    )));
                }
                Data::DateTime(DateTime {
                    UniversalTime: model.datetime_to_ticks.call1(py, (value,))?.extract(py)?,
                })
            }
            Element::TimeSpan => {
                if !value.is_instance(model.timedelta.bind(py))? {
                    return Err(type_error("a datetime.timedelta"));
                }
                let ticks = model.timedelta_to_ticks.call1(py, (value,))?;
                Data::TimeSpan(TimeSpan {
                    Duration: ticks.extract(py).map_err(|_| {
                        PyOverflowError::new_err(with_label(
                            label,
                            format!("{} is out of range for WinRT TimeSpan", repr(value)),
                        ))
                    })?,
                })
            }
            Element::Guid => {
                if let Ok(guid) = value.cast::<WinGUID>() {
                    Data::Guid(guid.borrow().0)
                } else if value.is_instance(model.uuid.bind(py))? {
                    Data::Guid(GUID::from_u128(
                        value.getattr(intern!(py, "int"))?.extract()?,
                    ))
                } else {
                    return Err(type_error("a uuid.UUID or WinGUID"));
                }
            }
            Element::Point | Element::Size | Element::Rect => {
                let fields = self.geometry_fields(value, element, label)?;
                match element {
                    Element::Point => Data::Point(Point {
                        X: fields[0],
                        Y: fields[1],
                    }),
                    Element::Size => Data::Size(Size {
                        Width: fields[0],
                        Height: fields[1],
                    }),
                    _ => Data::Rect(Rect {
                        X: fields[0],
                        Y: fields[1],
                        Width: fields[2],
                        Height: fields[3],
                    }),
                }
            }
            Element::Inspectable
            | Element::UInt8
            | Element::Int16
            | Element::UInt16
            | Element::Int32
            | Element::UInt32
            | Element::Int64
            | Element::UInt64 => unreachable!("not a non-integer scalar element"),
        })
    }

    /// The integer value of an int, or of an enum member boxed with an
    /// explicit integer type.
    fn integer(
        &self,
        value: &Bound<'_, PyAny>,
        element: Element,
        label: Option<&str>,
    ) -> PyResult<i128> {
        let py = value.py();
        let expected = |what: &str| {
            PyTypeError::new_err(with_label(
                label,
                format!(
                    "WinRT {} requires {what}, not {}",
                    element.name(),
                    type_name(value)
                ),
            ))
        };
        let number = if value.cast::<PyBool>().is_ok() {
            return Err(expected("an int"));
        } else if self.model.is_enum(value)? {
            py.get_type::<PyInt>()
                .call1((value,))
                .map_err(|_| expected("an int or an int-valued enum member"))?
        } else if value.is_instance_of::<PyInt>() {
            value.clone()
        } else {
            return Err(expected("an int"));
        };
        number.extract::<i128>().map_err(|_| {
            let (low, high) = element.integer_bounds().unwrap_or_default();
            PyOverflowError::new_err(with_label(
                label,
                format!(
                    "{} is out of range for WinRT {} ({low}..{high})",
                    repr(&number),
                    element.name()
                ),
            ))
        })
    }

    fn geometry_fields(
        &self,
        value: &Bound<'_, PyAny>,
        element: Element,
        label: Option<&str>,
    ) -> PyResult<Vec<f32>> {
        let fields = element.fields();
        if let Some(found) = ValueModel::instance_of(&self.model.geometry, value)?
            && found != element
        {
            return Err(PyTypeError::new_err(with_label(
                label,
                format!(
                    "WinRT {} requires a dynwinrt.values.{} or an object with numeric {} \
                     attributes, not dynwinrt.values.{}",
                    element.name(),
                    element.name(),
                    fields.join("/"),
                    found.name()
                ),
            )));
        }
        fields
            .iter()
            .map(|field| {
                let attribute = value.getattr_opt(*field)?.ok_or_else(|| {
                    PyTypeError::new_err(with_label(
                        label,
                        format!(
                            "WinRT {} requires a dynwinrt.values.{} or an object with numeric {} \
                             attributes, not {}",
                            element.name(),
                            element.name(),
                            fields.join("/"),
                            type_name(value)
                        ),
                    ))
                })?;
                let name = format!("WinRT {}.{field}", element.name());
                to_f32(&attribute, real(&attribute, &name, label)?, &name, label)
            })
            .collect()
    }

    /// Convert every item to exactly `element`, naming the failing element.
    fn array(&self, items: &[Bound<'_, PyAny>], element: Element, depth: usize) -> PyResult<Data> {
        let label = |index: usize| format!("{}Array element {index}", element.name());
        macro_rules! collect {
            ($variant:ident, $scalar:ident) => {
                Data::$variant(
                    items
                        .iter()
                        .enumerate()
                        .map(|(index, item)| {
                            match self.scalar(item, element, Some(&label(index)))? {
                                Data::$scalar(value) => Ok(value),
                                _ => unreachable!("scalar returns the requested element"),
                            }
                        })
                        .collect::<PyResult<_>>()?,
                )
            };
        }
        Ok(match element {
            Element::UInt8 => collect!(UInt8Array, UInt8),
            Element::Int16 => collect!(Int16Array, Int16),
            Element::UInt16 => collect!(UInt16Array, UInt16),
            Element::Int32 => collect!(Int32Array, Int32),
            Element::UInt32 => collect!(UInt32Array, UInt32),
            Element::Int64 => collect!(Int64Array, Int64),
            Element::UInt64 => collect!(UInt64Array, UInt64),
            Element::Single => collect!(SingleArray, Single),
            Element::Double => collect!(DoubleArray, Double),
            Element::Char16 => collect!(Char16Array, Char16),
            Element::Boolean => collect!(BooleanArray, Boolean),
            Element::String => collect!(StringArray, String),
            Element::DateTime => collect!(DateTimeArray, DateTime),
            Element::TimeSpan => collect!(TimeSpanArray, TimeSpan),
            Element::Guid => collect!(GuidArray, Guid),
            Element::Point => collect!(PointArray, Point),
            Element::Size => collect!(SizeArray, Size),
            Element::Rect => collect!(RectArray, Rect),
            Element::Inspectable => {
                if depth >= MAX_NESTING {
                    return Err(PyRecursionError::new_err(format!(
                        "WinRT InspectableArray values cannot nest more than {MAX_NESTING} levels"
                    )));
                }
                Data::InspectableArray(
                    items
                        .iter()
                        .enumerate()
                        .map(|(index, item)| {
                            self.inspectable_element(item, depth + 1)
                                .map_err(|error| with_context(item.py(), error, &label(index)))
                        })
                        .collect::<PyResult<_>>()?,
                )
            }
        })
    }

    /// An `InspectableArray` element, converted by the default rules.
    fn inspectable_element(
        &self,
        item: &Bound<'_, PyAny>,
        depth: usize,
    ) -> PyResult<Option<IUnknown>> {
        Ok(match self.box_value(item, depth)? {
            Boxed::Existing(object) => live(&object)?.0.as_object(),
            Boxed::New(value) => value.as_object(),
        })
    }
}

/// A plain `int`, which boxes only as Int32.
fn plain_int(value: &Bound<'_, PyAny>) -> PyResult<Data> {
    int32(value).map_err(|error| match error {
        Some(error) => error,
        None => PyOverflowError::new_err(format!(
            "{} does not fit in Int32, the only WinRT type a plain int boxes as; pass \
             property_type= (for example dynwinrt.values.PropertyType.Int64 or \
             PropertyType.UInt64) or a tag such as dynwinrt.values.Int64(value)",
            repr(value)
        )),
    })
}

/// An element of a list of plain ints, which boxes only as Int32Array.
fn plain_int_element(value: &Bound<'_, PyAny>, index: usize) -> PyResult<Data> {
    int32(value).map_err(|error| match error {
        Some(error) => error,
        None => PyOverflowError::new_err(format!(
            "element {index}: {} does not fit in Int32, and a list of plain ints boxes only as \
             Int32Array; pass property_type= (for example dynwinrt.values.PropertyType.\
             Int64Array or PropertyType.UInt64Array) or a typed array such as \
             dynwinrt.values.Int64Array([...])",
            repr(value)
        )),
    })
}

/// `value` as Int32; `Err(None)` when it is an int outside the Int32 range.
fn int32(value: &Bound<'_, PyAny>) -> Result<Data, Option<PyErr>> {
    match value.extract::<i32>() {
        Ok(number) => Ok(Data::Int32(number)),
        Err(error) if error.is_instance_of::<PyOverflowError>(value.py()) => Err(None),
        Err(error) => Err(Some(error)),
    }
}

fn real(value: &Bound<'_, PyAny>, name: &str, label: Option<&str>) -> PyResult<f64> {
    if value.cast::<PyBool>().is_ok()
        || !(value.is_instance_of::<PyInt>() || value.is_instance_of::<PyFloat>())
    {
        return Err(PyTypeError::new_err(with_label(
            label,
            format!("{name} requires a real number, not {}", type_name(value)),
        )));
    }
    value.extract::<f64>().map_err(|_| {
        PyOverflowError::new_err(with_label(
            label,
            format!("{} is out of range for {name}", repr(value)),
        ))
    })
}

/// `value` (already read as `number`) rounded to float32, rejecting finite
/// values beyond the float32 range.
fn to_f32(value: &Bound<'_, PyAny>, number: f64, name: &str, label: Option<&str>) -> PyResult<f32> {
    let single = number as f32;
    if number.is_finite() && single.is_infinite() {
        return Err(PyOverflowError::new_err(with_label(
            label,
            format!("{} is out of range for {name}", repr(value)),
        )));
    }
    Ok(single)
}

fn char16_unit(value: &Bound<'_, PyAny>, label: Option<&str>) -> PyResult<u16> {
    let Ok(text) = value.cast::<PyString>() else {
        return Err(PyTypeError::new_err(with_label(
            label,
            format!("WinRT Char16 requires a str, not {}", type_name(value)),
        )));
    };
    // PyUnicode_ReadChar keeps unpaired surrogates, unlike Rust strings.
    let unit = if unsafe { ffi::PyUnicode_GetLength(text.as_ptr()) } == 1 {
        u16::try_from(unsafe { ffi::PyUnicode_ReadChar(text.as_ptr(), 0) }).ok()
    } else {
        None
    };
    unit.ok_or_else(|| {
        PyValueError::new_err(with_label(
            label,
            format!(
                "WinRT Char16 requires exactly one UTF-16 code unit, not {}",
                repr(value)
            ),
        ))
    })
}

fn bytes_like(value: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    if let Ok(bytes) = value.cast::<PyBytes>() {
        return Ok(bytes.as_bytes().to_vec());
    }
    if let Ok(bytes) = value.cast::<PyByteArray>() {
        return Ok(bytes.to_vec());
    }
    // memoryview: Python's bytes() semantics, the raw bytes of the buffer.
    Ok(value
        .call_method0(intern!(value.py(), "tobytes"))?
        .cast_into::<PyBytes>()?
        .as_bytes()
        .to_vec())
}

fn items<'py>(value: &Bound<'py, PyAny>) -> PyResult<Vec<Bound<'py, PyAny>>> {
    value.try_iter()?.collect()
}
