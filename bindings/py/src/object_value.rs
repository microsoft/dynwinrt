// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! The single conversion layer between Python values and WinRT `Object`
//! (`IInspectable`) values.
//!
//! Generated code converts every `Object` position through exactly two
//! functions: [`to_winrt_object`] for inputs and [`from_winrt_object`] for
//! outputs. The Python data model they share (PropertyType-tagged scalars,
//! typed arrays and geometry values) is defined in `python/dynwinrt/_values.py`;
//! the language-neutral `IPropertyValue` payload is `dynwinrt::PropertyValueData`.
//!
//! Contract (see bindings/py/tests/test_object_values.py):
//! - R1: a boxed value read and written back keeps its PropertyType and value
//!   (DateTime and TimeSpan at Python's microsecond resolution).
//! - R2: a supported Python value written and read back compares equal.
//! - I1: objects that are not boxed values, and boxes without a Python
//!   representation, pass through in both directions as the same Python object.
//! - I2: boxed values have value semantics; every write creates a new box.

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
use windows::Foundation::{DateTime, Point, Rect, Size, TimeSpan};
use windows::core::{GUID, IUnknown};

use crate::errors::map_dynwinrt_error;
use crate::runtime::{DynWinRTType, DynWinRTValue, TABLE};

const ENUM_MARKER: &str = "_dynwinrt_enum_type";
const ENUM_HANDLE_CACHE: &str = "_dynwinrt_enum_handle";
const GEOMETRY_MARKER: &str = "_dynwinrt_property_type";

const SUPPORTED_INPUTS: &str = "None, a DynWinRTValue or projected WinRT object, bool, \
a dynwinrt tag (UInt8, Int16, UInt16, Int32, UInt32, Int64, UInt64, Single, Double, Char16), \
a generated WinRT enum, dynwinrt.Point/Size/Rect or the generated Windows.Foundation structs, \
int, float, str, a timezone-aware datetime.datetime, datetime.timedelta, uuid.UUID, \
bytes, bytearray, memoryview, a dynwinrt typed array, or a list/tuple of those";

/// Nested `InspectableArray` levels converted in either direction. Deeper (or
/// cyclic) Python lists raise `RecursionError`; deeper boxed arrays stay native.
const MAX_NESTING: usize = 64;

const EMPTY_SEQUENCE: &str = "cannot infer the WinRT array type of an empty list or tuple; \
use a dynwinrt typed array such as dynwinrt.StringArray([]) or dynwinrt.InspectableArray([])";

/// The one place this module creates Python-visible native values.
fn native(value: WinRTValue) -> DynWinRTValue {
    DynWinRTValue(value)
}

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

    fn is_float(self) -> bool {
        matches!(self, Self::Single | Self::Double)
    }

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
}

const TAGS: [Element; 10] = [
    Element::UInt8,
    Element::Int16,
    Element::UInt16,
    Element::Int32,
    Element::UInt32,
    Element::Int64,
    Element::UInt64,
    Element::Single,
    Element::Double,
    Element::Char16,
];

const ARRAYS: [Element; 18] = [
    Element::Int16,
    Element::UInt16,
    Element::Int32,
    Element::UInt32,
    Element::Int64,
    Element::UInt64,
    Element::Single,
    Element::Double,
    Element::Char16,
    Element::Boolean,
    Element::String,
    Element::Inspectable,
    Element::DateTime,
    Element::TimeSpan,
    Element::Guid,
    Element::Point,
    Element::Size,
    Element::Rect,
];

const GEOMETRY: [Element; 3] = [Element::Point, Element::Size, Element::Rect];

/// Python types of the `_values.py` data model and the helpers it relies on.
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

fn model(py: Python<'_>) -> PyResult<&'static ValueModel> {
    MODEL.get(py).ok_or_else(|| {
        pyo3::exceptions::PyRuntimeError::new_err("dynwinrt value model is not initialized")
    })
}

/// Bind the data model after `_values.py` has run in the native module.
pub(crate) fn init(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = module.py();
    let class = |name: &str| -> PyResult<Py<PyType>> {
        Ok(module.getattr(name)?.cast_into::<PyType>()?.unbind())
    };
    let function = |name: &str| -> PyResult<Py<PyAny>> { Ok(module.getattr(name)?.unbind()) };
    let builtin_new = |typ: Bound<'_, PyType>| -> PyResult<Py<PyAny>> {
        Ok(typ.getattr(intern!(py, "__new__"))?.unbind())
    };
    let model = ValueModel {
        tags: TAGS
            .iter()
            .map(|element| Ok((class(element.name())?, *element)))
            .collect::<PyResult<_>>()?,
        scalar_base: class("WinRTScalar")?,
        arrays: ARRAYS
            .iter()
            .map(|element| Ok((class(&format!("{}Array", element.name()))?, *element)))
            .collect::<PyResult<_>>()?,
        array_base: class("WinRTArray")?,
        geometry: GEOMETRY
            .iter()
            .map(|element| Ok((class(element.name())?, *element)))
            .collect::<PyResult<_>>()?,
        enum_base: py
            .import("enum")?
            .getattr("Enum")?
            .cast_into::<PyType>()?
            .unbind(),
        uuid: py
            .import("uuid")?
            .getattr("UUID")?
            .cast_into::<PyType>()?
            .unbind(),
        datetime: py
            .import("datetime")?
            .getattr("datetime")?
            .cast_into::<PyType>()?
            .unbind(),
        timedelta: py
            .import("datetime")?
            .getattr("timedelta")?
            .cast_into::<PyType>()?
            .unbind(),
        int_new: builtin_new(py.get_type::<PyInt>())?,
        float_new: builtin_new(py.get_type::<PyFloat>())?,
        str_new: builtin_new(py.get_type::<PyString>())?,
        ticks_to_datetime: function("_dynwinrt_ticks_to_datetime")?,
        datetime_to_ticks: function("_dynwinrt_datetime_to_ticks")?,
        ticks_to_timedelta: function("_dynwinrt_ticks_to_timedelta")?,
        timedelta_to_ticks: function("_dynwinrt_timedelta_to_ticks")?,
    };
    // A second initialization of the same interpreter keeps the first model.
    let _ = MODEL.set(py, model);
    Ok(())
}

// ======================================================================
// from_winrt_object: DynWinRTValue -> Python value
// ======================================================================

/// Convert a WinRT `Object` value to its Python representation.
///
/// Null becomes `None`; boxed `IPropertyValue` payloads become Python values
/// (tagged where the plain write rule would change their type); every other
/// object, including boxes without a Python representation, is returned as the
/// same `DynWinRTValue`. Non-`DynWinRTValue` inputs are returned unchanged, so
/// the conversion is idempotent.
#[pyfunction]
#[pyo3(signature = (value, /))]
pub fn from_winrt_object(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    unbox_at(py, value, 0)
}

fn unbox_at(py: Python<'_>, value: &Bound<'_, PyAny>, depth: usize) -> PyResult<Py<PyAny>> {
    let Ok(raw) = value.cast::<DynWinRTValue>() else {
        return Ok(value.clone().unbind());
    };
    let result =
        dynwinrt::unbox_property_value(&raw.try_borrow()?.0).map_err(map_dynwinrt_error)?;
    match result {
        PropertyValueUnboxResult::Null => Ok(py.None()),
        PropertyValueUnboxResult::NotPropertyValue | PropertyValueUnboxResult::Unsupported(_) => {
            Ok(value.clone().unbind())
        }
        PropertyValueUnboxResult::Value(data) => {
            Ok(data_to_python(py, model(py)?, data, depth)?
                .unwrap_or_else(|| value.clone().unbind()))
        }
    }
}

impl ValueModel {
    fn tag_type(&self, element: Element) -> &Py<PyType> {
        self.tags
            .iter()
            .find(|(_, candidate)| *candidate == element)
            .map(|(typ, _)| typ)
            .expect("every tag element has a class")
    }

    fn array_type(&self, element: Element) -> &Py<PyType> {
        self.arrays
            .iter()
            .find(|(_, candidate)| *candidate == element)
            .map(|(typ, _)| typ)
            .expect("every array element has a class")
    }

    fn geometry_type(&self, element: Element) -> &Py<PyType> {
        self.geometry
            .iter()
            .find(|(_, candidate)| *candidate == element)
            .map(|(typ, _)| typ)
            .expect("every geometry element has a class")
    }

    fn integer<'py, T: IntoPyObject<'py>>(
        &self,
        py: Python<'py>,
        element: Element,
        value: T,
    ) -> PyResult<Py<PyAny>> {
        // int.__new__(Tag, value) skips the validating Tag.__new__: the value
        // came from WinRT and is in range by construction.
        self.int_new.call1(py, (self.tag_type(element), value))
    }

    fn float(&self, py: Python<'_>, element: Element, value: f64) -> PyResult<Py<PyAny>> {
        self.float_new.call1(py, (self.tag_type(element), value))
    }

    fn char16(&self, py: Python<'_>, value: u16) -> PyResult<Py<PyAny>> {
        // PyUnicode_FromOrdinal preserves unpaired surrogates, unlike Rust strings.
        let character = unsafe {
            Bound::from_owned_ptr_or_err(py, ffi::PyUnicode_FromOrdinal(c_int::from(value)))?
        };
        self.str_new
            .call1(py, (self.tag_type(Element::Char16), character))
    }

    fn uuid(&self, py: Python<'_>, value: GUID) -> PyResult<Py<PyAny>> {
        let kwargs = PyDict::new(py);
        kwargs.set_item(intern!(py, "int"), value.to_u128())?;
        self.uuid.call(py, (), Some(&kwargs))
    }

    /// `None` when the instant is outside Python's datetime range.
    fn datetime(&self, py: Python<'_>, value: DateTime) -> PyResult<Option<Py<PyAny>>> {
        representable(py, self.ticks_to_datetime.call1(py, (value.UniversalTime,)))
    }

    /// `None` when the duration is outside Python's timedelta range.
    fn timedelta(&self, py: Python<'_>, value: TimeSpan) -> PyResult<Option<Py<PyAny>>> {
        representable(py, self.ticks_to_timedelta.call1(py, (value.Duration,)))
    }

    fn point(&self, py: Python<'_>, value: Point) -> PyResult<Py<PyAny>> {
        self.geometry_type(Element::Point)
            .call1(py, (value.X, value.Y))
    }

    fn size(&self, py: Python<'_>, value: Size) -> PyResult<Py<PyAny>> {
        self.geometry_type(Element::Size)
            .call1(py, (value.Width, value.Height))
    }

    fn rect(&self, py: Python<'_>, value: Rect) -> PyResult<Py<PyAny>> {
        self.geometry_type(Element::Rect)
            .call1(py, (value.X, value.Y, value.Width, value.Height))
    }

    fn array(
        &self,
        py: Python<'_>,
        element: Element,
        items: impl IntoIterator<Item = PyResult<Py<PyAny>>>,
    ) -> PyResult<Py<PyAny>> {
        let items = items.into_iter().collect::<PyResult<Vec<_>>>()?;
        self.array_type(element)
            .call1(py, (PyList::new(py, items)?,))
    }

    /// `None` when any element is outside Python's representable range.
    fn optional_array(
        &self,
        py: Python<'_>,
        element: Element,
        items: impl IntoIterator<Item = PyResult<Option<Py<PyAny>>>>,
    ) -> PyResult<Option<Py<PyAny>>> {
        let mut values = Vec::new();
        for item in items {
            let Some(item) = item? else {
                return Ok(None);
            };
            values.push(item);
        }
        Ok(Some(
            self.array_type(element)
                .call1(py, (PyList::new(py, values)?,))?,
        ))
    }
}

fn representable(py: Python<'_>, result: PyResult<Py<PyAny>>) -> PyResult<Option<Py<PyAny>>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.is_instance_of::<PyOverflowError>(py) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Map a boxed payload to the Python data model; `None` keeps the raw value.
fn data_to_python(
    py: Python<'_>,
    model: &ValueModel,
    data: Data,
    depth: usize,
) -> PyResult<Option<Py<PyAny>>> {
    let value = match data {
        Data::UInt8(value) => model.integer(py, Element::UInt8, value)?,
        Data::Int16(value) => model.integer(py, Element::Int16, value)?,
        Data::UInt16(value) => model.integer(py, Element::UInt16, value)?,
        Data::Int32(value) => value.into_py_any(py)?,
        Data::UInt32(value) => model.integer(py, Element::UInt32, value)?,
        Data::Int64(value) => model.integer(py, Element::Int64, value)?,
        Data::UInt64(value) => model.integer(py, Element::UInt64, value)?,
        Data::Single(value) => model.float(py, Element::Single, f64::from(value))?,
        Data::Double(value) => value.into_py_any(py)?,
        Data::Char16(value) => model.char16(py, value)?,
        Data::Boolean(value) => PyBool::new(py, value).to_owned().into_any().unbind(),
        Data::String(value) => value.into_py_any(py)?,
        Data::Guid(value) => model.uuid(py, value)?,
        Data::DateTime(value) => return model.datetime(py, value),
        Data::TimeSpan(value) => return model.timedelta(py, value),
        Data::Point(value) => model.point(py, value)?,
        Data::Size(value) => model.size(py, value)?,
        Data::Rect(value) => model.rect(py, value)?,
        Data::UInt8Array(values) => PyBytes::new(py, &values).into_any().unbind(),
        Data::Int16Array(values) => model.array(
            py,
            Element::Int16,
            values
                .into_iter()
                .map(|value| model.integer(py, Element::Int16, value)),
        )?,
        Data::UInt16Array(values) => model.array(
            py,
            Element::UInt16,
            values
                .into_iter()
                .map(|value| model.integer(py, Element::UInt16, value)),
        )?,
        Data::Int32Array(values) => model.array(
            py,
            Element::Int32,
            values.into_iter().map(|value| value.into_py_any(py)),
        )?,
        Data::UInt32Array(values) => model.array(
            py,
            Element::UInt32,
            values
                .into_iter()
                .map(|value| model.integer(py, Element::UInt32, value)),
        )?,
        Data::Int64Array(values) => model.array(
            py,
            Element::Int64,
            values
                .into_iter()
                .map(|value| model.integer(py, Element::Int64, value)),
        )?,
        Data::UInt64Array(values) => model.array(
            py,
            Element::UInt64,
            values
                .into_iter()
                .map(|value| model.integer(py, Element::UInt64, value)),
        )?,
        Data::SingleArray(values) => model.array(
            py,
            Element::Single,
            values
                .into_iter()
                .map(|value| model.float(py, Element::Single, f64::from(value))),
        )?,
        Data::DoubleArray(values) => model.array(
            py,
            Element::Double,
            values.into_iter().map(|value| value.into_py_any(py)),
        )?,
        Data::Char16Array(values) => model.array(
            py,
            Element::Char16,
            values.into_iter().map(|value| model.char16(py, value)),
        )?,
        Data::BooleanArray(values) => model.array(
            py,
            Element::Boolean,
            values
                .into_iter()
                .map(|value| Ok(PyBool::new(py, value).to_owned().into_any().unbind())),
        )?,
        Data::StringArray(values) => model.array(
            py,
            Element::String,
            values.into_iter().map(|value| value.into_py_any(py)),
        )?,
        Data::InspectableArray(values) => model.array(
            py,
            Element::Inspectable,
            values.into_iter().map(|value| match value {
                None => Ok(py.None()),
                Some(object) => {
                    let element = Bound::new(py, native(WinRTValue::Object(object)))?;
                    if depth >= MAX_NESTING {
                        Ok(element.into_any().unbind())
                    } else {
                        unbox_at(py, element.as_any(), depth + 1)
                    }
                }
            }),
        )?,
        Data::DateTimeArray(values) => {
            return model.optional_array(
                py,
                Element::DateTime,
                values.into_iter().map(|value| model.datetime(py, value)),
            );
        }
        Data::TimeSpanArray(values) => {
            return model.optional_array(
                py,
                Element::TimeSpan,
                values.into_iter().map(|value| model.timedelta(py, value)),
            );
        }
        Data::GuidArray(values) => model.array(
            py,
            Element::Guid,
            values.into_iter().map(|value| model.uuid(py, value)),
        )?,
        Data::PointArray(values) => model.array(
            py,
            Element::Point,
            values.into_iter().map(|value| model.point(py, value)),
        )?,
        Data::SizeArray(values) => model.array(
            py,
            Element::Size,
            values.into_iter().map(|value| model.size(py, value)),
        )?,
        Data::RectArray(values) => model.array(
            py,
            Element::Rect,
            values.into_iter().map(|value| model.rect(py, value)),
        )?,
    };
    Ok(Some(value))
}

// ======================================================================
// to_winrt_object: Python value -> DynWinRTValue
// ======================================================================

/// Convert a Python value for a WinRT `Object` position.
///
/// `DynWinRTValue` objects and projected wrappers pass through unchanged
/// (wrappers as their `_obj`); `None` becomes the null object; supported Python
/// values are boxed as `Windows.Foundation.PropertyValue` objects and generated
/// enums as `IReference<Enum>`. Anything else raises `TypeError`.
#[pyfunction]
#[pyo3(signature = (value, /))]
pub fn to_winrt_object(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    match box_object(value, model(py)?, 0)? {
        Boxed::Existing(object) => Ok(object),
        Boxed::New(boxed) => Ok(Bound::new(py, native(boxed))?.into_any().unbind()),
    }
}

enum Boxed {
    /// An existing native value, passed through as the same Python object.
    Existing(Py<PyAny>),
    New(WinRTValue),
}

/// A Python value classified by the write rules, in their pinned order.
enum Kind<'py> {
    Null,
    Existing(Bound<'py, PyAny>),
    /// A `DynWinRTValue` holding a primitive, string, GUID, enum, struct or array.
    Native(Bound<'py, DynWinRTValue>),
    Boolean(bool),
    Tag(Element),
    Enum,
    Geometry(Element),
    Int,
    Float,
    Str,
    DateTime,
    TimeSpan,
    Guid,
    Bytes,
    TypedArray(Element),
    Sequence,
}

fn is_native_object(value: &WinRTValue) -> bool {
    matches!(
        value,
        WinRTValue::Object(_) | WinRTValue::Null | WinRTValue::Async(_)
    )
}

/// Apply the write rules in their pinned order:
/// None -> DynWinRTValue/projected wrapper -> bool -> tags -> enum marker ->
/// geometry markers -> int -> float -> str -> datetime/timedelta/UUID ->
/// bytes-like -> typed arrays -> list/tuple. `None` means unsupported.
///
/// Order matters because bool, IntEnum/IntFlag and the integer tags are int
/// subclasses, typed arrays are lists, and wrappers may be anything.
fn classify<'py>(value: &Bound<'py, PyAny>, model: &ValueModel) -> PyResult<Option<Kind<'py>>> {
    let py = value.py();
    // Exact built-in scalars cannot match any earlier rule, so this fast path
    // is equivalent to the ordered chain below.
    if value.is_exact_instance_of::<PyInt>() {
        return Ok(Some(Kind::Int));
    }
    if value.is_exact_instance_of::<PyString>() {
        return Ok(Some(Kind::Str));
    }
    if value.is_exact_instance_of::<PyFloat>() {
        return Ok(Some(Kind::Float));
    }
    if let Ok(boolean) = value.cast::<PyBool>() {
        return Ok(Some(Kind::Boolean(boolean.is_true())));
    }

    // 1. None
    if value.is_none() {
        return Ok(Some(Kind::Null));
    }
    // 2. DynWinRTValue and projected wrappers
    if let Ok(raw) = value.cast::<DynWinRTValue>() {
        return Ok(Some(if is_native_object(&raw.try_borrow()?.0) {
            Kind::Existing(value.clone())
        } else {
            Kind::Native(raw.clone())
        }));
    }
    if let Some(inner) = value.getattr_opt(intern!(py, "_obj"))?
        && inner.cast::<DynWinRTValue>().is_ok()
    {
        return Ok(Some(Kind::Existing(inner)));
    }
    // 3. bool: handled by the fast path; bool cannot be subclassed.
    let typ = value.get_type();
    // 4. PropertyType tags
    if value.is_instance(model.scalar_base.bind(py))? {
        for (tag, element) in &model.tags {
            if typ.is(tag) || value.is_instance(tag.bind(py))? {
                return Ok(Some(Kind::Tag(*element)));
            }
        }
    }
    // 5. generated enums
    if value.is_instance(model.enum_base.bind(py))? && typ.hasattr(ENUM_MARKER)? {
        return Ok(Some(Kind::Enum));
    }
    // 6. Windows.Foundation Point/Size/Rect: dynwinrt values or generated structs
    for (geometry, element) in &model.geometry {
        if typ.is(geometry) {
            return Ok(Some(Kind::Geometry(*element)));
        }
    }
    if let Some(marker) = typ.getattr_opt(GEOMETRY_MARKER)?
        && let Ok(marker) = marker.extract::<String>()
        && let Some(element) = GEOMETRY.iter().find(|element| element.name() == marker)
    {
        return Ok(Some(Kind::Geometry(*element)));
    }
    // 7-9. int (including unmarked IntEnum), float, str subclasses
    if value.is_instance_of::<PyInt>() {
        return Ok(Some(Kind::Int));
    }
    if value.is_instance_of::<PyFloat>() {
        return Ok(Some(Kind::Float));
    }
    if value.is_instance_of::<PyString>() {
        return Ok(Some(Kind::Str));
    }
    // 10. datetime, timedelta, UUID
    if value.is_instance(model.datetime.bind(py))? {
        return Ok(Some(Kind::DateTime));
    }
    if value.is_instance(model.timedelta.bind(py))? {
        return Ok(Some(Kind::TimeSpan));
    }
    if value.is_instance(model.uuid.bind(py))? {
        return Ok(Some(Kind::Guid));
    }
    // 11. bytes-like
    if value.is_instance_of::<PyBytes>()
        || value.is_instance_of::<PyByteArray>()
        || value.is_instance_of::<PyMemoryView>()
    {
        return Ok(Some(Kind::Bytes));
    }
    // 12. typed arrays
    if value.is_instance(model.array_base.bind(py))? {
        for (array, element) in &model.arrays {
            if typ.is(array) || value.is_instance(array.bind(py))? {
                return Ok(Some(Kind::TypedArray(*element)));
            }
        }
        return Err(PyTypeError::new_err(
            "dynwinrt.WinRTArray is abstract; use a typed array such as dynwinrt.Int32Array",
        ));
    }
    // 13. list/tuple
    if value.is_instance_of::<PyList>() || value.is_instance_of::<PyTuple>() {
        return Ok(Some(Kind::Sequence));
    }
    Ok(None)
}

fn unsupported(value: &Bound<'_, PyAny>) -> PyErr {
    let name = value
        .get_type()
        .qualname()
        .map(|name| name.to_string())
        .unwrap_or_else(|_| "value".into());
    PyTypeError::new_err(format!(
        "cannot convert {name} to a WinRT Object; expected {SUPPORTED_INPUTS}"
    ))
}

fn box_object(value: &Bound<'_, PyAny>, model: &ValueModel, depth: usize) -> PyResult<Boxed> {
    if depth > MAX_NESTING {
        return Err(PyRecursionError::new_err(format!(
            "WinRT Object arrays cannot nest more than {MAX_NESTING} levels"
        )));
    }
    let Some(kind) = classify(value, model)? else {
        return Err(unsupported(value));
    };
    match kind {
        Kind::Null => Ok(Boxed::New(WinRTValue::Null)),
        Kind::Existing(object) => Ok(Boxed::Existing(object.unbind())),
        Kind::Native(raw) => box_native(&raw.try_borrow()?.0).map(Boxed::New),
        Kind::Enum => box_enum(value).map(Boxed::New),
        kind => box_data(&payload(value, kind, model, depth)?).map(Boxed::New),
    }
}

fn box_data(data: &Data) -> PyResult<WinRTValue> {
    dynwinrt::box_property_value(data).map_err(map_dynwinrt_error)
}

/// The boxed payload of a value classified as `kind`.
fn payload(
    value: &Bound<'_, PyAny>,
    kind: Kind<'_>,
    model: &ValueModel,
    depth: usize,
) -> PyResult<Data> {
    Ok(match kind {
        Kind::Boolean(value) => Data::Boolean(value),
        Kind::Tag(element) | Kind::Geometry(element) => scalar(value, element, model, None)?,
        Kind::Int => plain_integer(value)?,
        Kind::Float => Data::Double(value.extract()?),
        Kind::Str => Data::String(value.extract()?),
        Kind::DateTime => scalar(value, Element::DateTime, model, None)?,
        Kind::TimeSpan => scalar(value, Element::TimeSpan, model, None)?,
        Kind::Guid => scalar(value, Element::Guid, model, None)?,
        Kind::Bytes => Data::UInt8Array(bytes_like(value)?),
        Kind::TypedArray(element) => array(&items(value)?, element, model, depth)?,
        Kind::Sequence => sequence(&items(value)?, model, depth)?,
        Kind::Null | Kind::Existing(_) | Kind::Native(_) | Kind::Enum => {
            unreachable!("handled by box_object")
        }
    })
}

/// Plain int: Int32 when it fits, then Int64, then UInt64.
fn plain_integer(value: &Bound<'_, PyAny>) -> PyResult<Data> {
    let number = big_integer(value, "Object")?;
    if let Ok(number) = i32::try_from(number) {
        Ok(Data::Int32(number))
    } else if let Ok(number) = i64::try_from(number) {
        Ok(Data::Int64(number))
    } else if let Ok(number) = u64::try_from(number) {
        Ok(Data::UInt64(number))
    } else {
        Err(integer_overflow(number, "Int32, Int64 or UInt64"))
    }
}

fn big_integer(value: &Bound<'_, PyAny>, label: &str) -> PyResult<i128> {
    value.extract::<i128>().map_err(|error| {
        if error.is_instance_of::<PyOverflowError>(value.py()) {
            PyOverflowError::new_err(format!(
                "{} is out of range for a WinRT {label} value",
                value
                    .repr()
                    .map(|repr| repr.to_string())
                    .unwrap_or_default()
            ))
        } else {
            error
        }
    })
}

fn integer_overflow(number: i128, target: &str) -> PyErr {
    PyOverflowError::new_err(format!("{number} is out of range for WinRT {target}"))
}

fn context(label: Option<&str>, message: String) -> String {
    match label {
        Some(label) => format!("{label}: {message}"),
        None => message,
    }
}

fn real(value: &Bound<'_, PyAny>, label: &str) -> PyResult<f64> {
    if value.cast::<PyBool>().is_ok()
        || !(value.is_instance_of::<PyInt>() || value.is_instance_of::<PyFloat>())
    {
        return Err(PyTypeError::new_err(format!(
            "WinRT {label} requires a real number, not {}",
            value.get_type().qualname()?
        )));
    }
    value.extract::<f64>()
}

fn to_f32(value: f64, label: &str) -> PyResult<f32> {
    let single = value as f32;
    if value.is_finite() && single.is_infinite() {
        return Err(PyOverflowError::new_err(format!(
            "{value} is out of range for WinRT {label}"
        )));
    }
    Ok(single)
}

fn char16(value: &Bound<'_, PyAny>, label: &str) -> PyResult<u16> {
    let Ok(text) = value.cast::<PyString>() else {
        return Err(PyTypeError::new_err(format!(
            "WinRT {label} requires a str, not {}",
            value.get_type().qualname()?
        )));
    };
    let unit = if unsafe { ffi::PyUnicode_GetLength(text.as_ptr()) } == 1 {
        u16::try_from(unsafe { ffi::PyUnicode_ReadChar(text.as_ptr(), 0) }).ok()
    } else {
        None
    };
    unit.ok_or_else(|| {
        PyValueError::new_err(format!(
            "WinRT {label} requires exactly one UTF-16 code unit, not {}",
            value
                .repr()
                .map(|repr| repr.to_string())
                .unwrap_or_default()
        ))
    })
}

fn geometry_field(value: &Bound<'_, PyAny>, name: &str, label: &str) -> PyResult<f32> {
    let label = format!("{label}.{name}");
    to_f32(real(&value.getattr(name)?, &label)?, &label)
}

/// Convert one value to exactly `element`, as a scalar payload.
///
/// `label` names the enclosing array element in error messages.
fn scalar(
    value: &Bound<'_, PyAny>,
    element: Element,
    model: &ValueModel,
    label: Option<&str>,
) -> PyResult<Data> {
    let py = value.py();
    let name = label.unwrap_or(element.name());
    let type_error = |expected: &str| -> PyResult<Data> {
        Err(PyTypeError::new_err(context(
            label,
            format!(
                "WinRT {} requires {expected}, not {}",
                element.name(),
                value.get_type().qualname()?
            ),
        )))
    };
    if let Some((low, high)) = element.integer_bounds() {
        if value.cast::<PyBool>().is_ok() || !value.is_instance_of::<PyInt>() {
            return type_error("an int");
        }
        let number = big_integer(value, element.name())?;
        if number < low || number > high {
            return Err(PyOverflowError::new_err(context(
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
            Element::UInt64 => Data::UInt64(number as u64),
            _ => unreachable!("integer element"),
        });
    }
    Ok(match element {
        Element::Single => Data::Single(to_f32(real(value, name)?, name)?),
        Element::Double => Data::Double(real(value, name)?),
        Element::Char16 => Data::Char16(char16(value, name)?),
        Element::Boolean => match value.cast::<PyBool>() {
            Ok(boolean) => Data::Boolean(boolean.is_true()),
            Err(_) => return type_error("a bool"),
        },
        Element::String => {
            if !value.is_instance_of::<PyString>() {
                return type_error("a str");
            }
            Data::String(value.extract()?)
        }
        Element::DateTime => {
            if !value.is_instance(model.datetime.bind(py))? {
                return type_error("a timezone-aware datetime.datetime");
            }
            Data::DateTime(DateTime {
                UniversalTime: model.datetime_to_ticks.call1(py, (value,))?.extract(py)?,
            })
        }
        Element::TimeSpan => {
            if !value.is_instance(model.timedelta.bind(py))? {
                return type_error("a datetime.timedelta");
            }
            Data::TimeSpan(TimeSpan {
                Duration: model.timedelta_to_ticks.call1(py, (value,))?.extract(py)?,
            })
        }
        Element::Guid => {
            if !value.is_instance(model.uuid.bind(py))? {
                return type_error("a uuid.UUID");
            }
            Data::Guid(GUID::from_u128(
                value.getattr(intern!(py, "int"))?.extract()?,
            ))
        }
        Element::Point | Element::Size | Element::Rect => {
            if !matches!(classify(value, model)?, Some(Kind::Geometry(found)) if found == element) {
                return type_error(&format!(
                    "a dynwinrt.{0} or generated Windows.Foundation.{0}",
                    element.name()
                ));
            }
            match element {
                Element::Point => Data::Point(Point {
                    X: geometry_field(value, "x", name)?,
                    Y: geometry_field(value, "y", name)?,
                }),
                Element::Size => Data::Size(Size {
                    Width: geometry_field(value, "width", name)?,
                    Height: geometry_field(value, "height", name)?,
                }),
                _ => Data::Rect(Rect {
                    X: geometry_field(value, "x", name)?,
                    Y: geometry_field(value, "y", name)?,
                    Width: geometry_field(value, "width", name)?,
                    Height: geometry_field(value, "height", name)?,
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

fn bytes_like(value: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    if let Ok(bytes) = value.cast::<PyBytes>() {
        return Ok(bytes.as_bytes().to_vec());
    }
    if let Ok(bytes) = value.cast::<PyByteArray>() {
        return Ok(bytes.to_vec());
    }
    // memoryview: Python's bytes() semantics (the raw bytes of the buffer).
    Ok(value
        .call_method0(intern!(value.py(), "tobytes"))?
        .cast_into::<PyBytes>()?
        .as_bytes()
        .to_vec())
}

fn items<'py>(value: &Bound<'py, PyAny>) -> PyResult<Vec<Bound<'py, PyAny>>> {
    value.try_iter()?.collect()
}

/// Convert every item to exactly `element`.
fn array(
    items: &[Bound<'_, PyAny>],
    element: Element,
    model: &ValueModel,
    depth: usize,
) -> PyResult<Data> {
    let array_name = format!("{}Array", element.name());
    let label = |index: usize| format!("{array_name} element {index}");
    macro_rules! collect {
        ($variant:ident, $scalar:ident) => {
            Data::$variant(
                items
                    .iter()
                    .enumerate()
                    .map(
                        |(index, item)| match scalar(item, element, model, Some(&label(index)))? {
                            Data::$scalar(value) => Ok(value),
                            _ => unreachable!("scalar returns the requested element"),
                        },
                    )
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
        Element::Inspectable => Data::InspectableArray(
            items
                .iter()
                .map(|item| {
                    let object = match box_object(item, model, depth + 1)? {
                        Boxed::Existing(object) => object
                            .bind(item.py())
                            .cast::<DynWinRTValue>()?
                            .try_borrow()?
                            .0
                            .clone(),
                        Boxed::New(value) => value,
                    };
                    Ok(object.as_object())
                })
                .collect::<PyResult<Vec<Option<IUnknown>>>>()?,
        ),
    })
}

/// How one item of an untagged list or tuple constrains the array type.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Candidate {
    Exact(Element),
    PlainInt,
    PlainFloat,
    Other,
}

/// Infer the array type of an untagged list or tuple.
///
/// Homogeneous items give the matching array; plain ints give the smallest of
/// Int32/Int64/UInt64 that fits all of them; plain ints mixed with plain floats
/// give DoubleArray; plain numbers adopt a single numeric tag present in the
/// list; anything else, including None and objects, gives an InspectableArray
/// whose items are converted one by one. Empty sequences carry no element type
/// and are rejected.
fn sequence(items: &[Bound<'_, PyAny>], model: &ValueModel, depth: usize) -> PyResult<Data> {
    if items.is_empty() {
        return Err(PyTypeError::new_err(EMPTY_SEQUENCE));
    }
    let mut exact: Option<Element> = None;
    let (mut ints, mut floats, mut mixed) = (false, false, false);
    for item in items {
        let candidate = match classify(item, model)? {
            Some(Kind::Boolean(_)) => Candidate::Exact(Element::Boolean),
            Some(Kind::Tag(element) | Kind::Geometry(element)) => Candidate::Exact(element),
            Some(Kind::Str) => Candidate::Exact(Element::String),
            Some(Kind::DateTime) => Candidate::Exact(Element::DateTime),
            Some(Kind::TimeSpan) => Candidate::Exact(Element::TimeSpan),
            Some(Kind::Guid) => Candidate::Exact(Element::Guid),
            Some(Kind::Int) => Candidate::PlainInt,
            Some(Kind::Float) => Candidate::PlainFloat,
            Some(_) => Candidate::Other,
            None => return Err(unsupported(item)),
        };
        match candidate {
            Candidate::Exact(element) => match exact {
                None => exact = Some(element),
                Some(existing) if existing == element => {}
                Some(_) => mixed = true,
            },
            Candidate::PlainInt => ints = true,
            Candidate::PlainFloat => floats = true,
            Candidate::Other => mixed = true,
        }
    }
    let element = match exact {
        _ if mixed => Element::Inspectable,
        None if floats => Element::Double,
        None => return plain_integer_array(items),
        Some(element) if !ints && !floats => element,
        Some(element) if element.integer_bounds().is_some() && !floats => element,
        Some(element) if element.is_float() => element,
        Some(_) => Element::Inspectable,
    };
    array(items, element, model, depth)
}

fn plain_integer_array(items: &[Bound<'_, PyAny>]) -> PyResult<Data> {
    let numbers = items
        .iter()
        .map(|item| big_integer(item, "array element"))
        .collect::<PyResult<Vec<i128>>>()?;
    if let Ok(values) = numbers
        .iter()
        .map(|n| i32::try_from(*n))
        .collect::<Result<_, _>>()
    {
        Ok(Data::Int32Array(values))
    } else if let Ok(values) = numbers
        .iter()
        .map(|n| i64::try_from(*n))
        .collect::<Result<_, _>>()
    {
        Ok(Data::Int64Array(values))
    } else if let Ok(values) = numbers
        .iter()
        .map(|n| u64::try_from(*n))
        .collect::<Result<_, _>>()
    {
        Ok(Data::UInt64Array(values))
    } else {
        Err(PyOverflowError::new_err(
            "integers in this list do not all fit one WinRT Int32, Int64 or UInt64 array",
        ))
    }
}

// ======================================================================
// Enums and native values
// ======================================================================

/// Box a generated WinRT enum member as `IReference<Enum>`, as C# does.
fn box_enum(value: &Bound<'_, PyAny>) -> PyResult<WinRTValue> {
    let handle = enum_handle(&value.get_type())?;
    let number: i64 = value.extract()?;
    let member = handle.enum_value(number).map_err(map_dynwinrt_error)?;
    dynwinrt::box_ireference(member, handle).map_err(map_dynwinrt_error)
}

/// The enum's TypeHandle, registered from the class marker once and cached on
/// the generated class.
fn enum_handle(typ: &Bound<'_, PyType>) -> PyResult<dynwinrt::TypeHandle> {
    if let Some(cached) = typ.getattr_opt(ENUM_HANDLE_CACHE)?
        && let Ok(cached) = cached.cast::<DynWinRTType>()
    {
        return Ok(cached.try_borrow()?.0.clone());
    }
    let (name, backing): (String, String) = typ.getattr(ENUM_MARKER)?.extract()?;
    let underlying = match backing.as_str() {
        "Int32" => dynwinrt::TypeKind::I32,
        "UInt32" => dynwinrt::TypeKind::U32,
        _ => {
            return Err(PyTypeError::new_err(format!(
                "invalid WinRT enum backing type {backing:?} for {name}"
            )));
        }
    };
    let members = typ
        .getattr("__members__")?
        .call_method0("items")?
        .try_iter()?
        .map(|item| {
            let (member, value): (String, i64) = item?.extract()?;
            let bits = match underlying {
                dynwinrt::TypeKind::U32 => u32::try_from(value).ok().map(|value| value as i32),
                _ => i32::try_from(value).ok(),
            };
            bits.map(|bits| (member, bits)).ok_or_else(|| {
                PyOverflowError::new_err(format!("{name}.{value} does not fit {backing}"))
            })
        })
        .collect::<PyResult<Vec<_>>>()?;
    let handle = TABLE
        .enum_type_with_underlying(&name, members, underlying)
        .map_err(map_dynwinrt_error)?;
    typ.setattr(ENUM_HANDLE_CACHE, DynWinRTType(handle.clone()))?;
    Ok(handle)
}

/// Box a `DynWinRTValue` that holds a non-object value by its exact WinRT kind.
fn box_native(value: &WinRTValue) -> PyResult<WinRTValue> {
    if let WinRTValue::Enum { type_handle, .. } = value {
        return dynwinrt::box_ireference(value.clone(), type_handle.clone())
            .map_err(map_dynwinrt_error);
    }
    box_data(&native_payload(value)?)
}

fn native_payload(value: &WinRTValue) -> PyResult<Data> {
    let unsupported = || {
        PyTypeError::new_err(format!(
            "a DynWinRTValue of kind {:?} has no IPropertyValue representation",
            value.get_type_kind()
        ))
    };
    Ok(match value {
        WinRTValue::Bool(value) => Data::Boolean(*value),
        WinRTValue::U8(value) => Data::UInt8(*value),
        WinRTValue::I16(value) => Data::Int16(*value),
        WinRTValue::U16(value) => Data::UInt16(*value),
        WinRTValue::I32(value) => Data::Int32(*value),
        WinRTValue::U32(value) => Data::UInt32(*value),
        WinRTValue::I64(value) => Data::Int64(*value),
        WinRTValue::U64(value) => Data::UInt64(*value),
        WinRTValue::F32(value) => Data::Single(*value),
        WinRTValue::F64(value) => Data::Double(*value),
        WinRTValue::HString(value) => Data::String(value.to_string()),
        WinRTValue::Guid(value) => Data::Guid(*value),
        WinRTValue::Struct(data) => match data.type_handle().signature_string().as_str() {
            "struct(Windows.Foundation.Point;f4;f4)" => Data::Point(Point {
                X: data.get_field(0),
                Y: data.get_field(1),
            }),
            "struct(Windows.Foundation.Size;f4;f4)" => Data::Size(Size {
                Width: data.get_field(0),
                Height: data.get_field(1),
            }),
            "struct(Windows.Foundation.Rect;f4;f4;f4;f4)" => Data::Rect(Rect {
                X: data.get_field(0),
                Y: data.get_field(1),
                Width: data.get_field(2),
                Height: data.get_field(3),
            }),
            "struct(Windows.Foundation.DateTime;i8)" => Data::DateTime(DateTime {
                UniversalTime: data.get_field(0),
            }),
            "struct(Windows.Foundation.TimeSpan;i8)" => Data::TimeSpan(TimeSpan {
                Duration: data.get_field(0),
            }),
            _ => return Err(unsupported()),
        },
        WinRTValue::Array(array) => {
            let values = (0..array.len())
                .map(|index| array.try_get(index).map_err(map_dynwinrt_error))
                .collect::<PyResult<Vec<_>>>()?;
            macro_rules! collect {
                ($variant:ident, $scalar:ident) => {
                    Data::$variant(
                        values
                            .iter()
                            .map(|value| match native_payload(value)? {
                                Data::$scalar(value) => Ok(value),
                                _ => Err(unsupported()),
                            })
                            .collect::<PyResult<_>>()?,
                    )
                };
            }
            match array.element_type.kind() {
                dynwinrt::TypeKind::U8 => collect!(UInt8Array, UInt8),
                dynwinrt::TypeKind::I16 => collect!(Int16Array, Int16),
                dynwinrt::TypeKind::U16 => collect!(UInt16Array, UInt16),
                dynwinrt::TypeKind::I32 => collect!(Int32Array, Int32),
                dynwinrt::TypeKind::U32 => collect!(UInt32Array, UInt32),
                dynwinrt::TypeKind::I64 => collect!(Int64Array, Int64),
                dynwinrt::TypeKind::U64 => collect!(UInt64Array, UInt64),
                dynwinrt::TypeKind::F32 => collect!(SingleArray, Single),
                dynwinrt::TypeKind::F64 => collect!(DoubleArray, Double),
                dynwinrt::TypeKind::Bool => collect!(BooleanArray, Boolean),
                dynwinrt::TypeKind::Char16 => Data::Char16Array(
                    values
                        .iter()
                        .map(|value| match value {
                            WinRTValue::U16(unit) => Ok(*unit),
                            _ => Err(unsupported()),
                        })
                        .collect::<PyResult<_>>()?,
                ),
                dynwinrt::TypeKind::HString => collect!(StringArray, String),
                dynwinrt::TypeKind::Guid => collect!(GuidArray, Guid),
                dynwinrt::TypeKind::Object => {
                    Data::InspectableArray(values.iter().map(WinRTValue::as_object).collect())
                }
                _ => return Err(unsupported()),
            }
        }
        _ => return Err(unsupported()),
    })
}
