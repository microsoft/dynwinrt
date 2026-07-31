// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::sync::Arc;

use dynwinrt;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use windows::core::{GUID, HSTRING, IUnknown, Interface};

use crate::errors::{map_dynwinrt_error, map_dynwinrt_error_with_context, map_windows_error};

/// Shared MetadataTable — created once, used everywhere.
static TABLE: std::sync::LazyLock<Arc<dynwinrt::MetadataTable>> =
    std::sync::LazyLock::new(|| dynwinrt::MetadataTable::new());

// ======================================================================
// Runtime initialization
// ======================================================================

#[pyclass]
pub struct WinAppSDKContext(pub(crate) dynwinrt::WinAppSdkContext);

#[pymethods]
impl WinAppSDKContext {
    /// Return the framework resources.pri path selected by init_winappsdk.
    fn resource_pri_path(&self) -> PyResult<String> {
        self.0.resource_pri_path().map_err(map_windows_error)
    }
}

#[pyclass(unsendable)]
pub struct RoApartment {
    apartment_type: i32,
    active: bool,
}

impl RoApartment {
    fn initialize(&mut self) -> PyResult<()> {
        if self.active {
            return Err(PyRuntimeError::new_err(
                "the COM apartment context is already active",
            ));
        }
        use windows::Win32::System::WinRT::{
            RO_INIT_MULTITHREADED, RO_INIT_SINGLETHREADED, RoInitialize,
        };
        let init_type = match self.apartment_type {
            0 => RO_INIT_SINGLETHREADED,
            _ => RO_INIT_MULTITHREADED,
        };
        unsafe { RoInitialize(init_type) }.map_err(map_windows_error)?;
        self.active = true;
        Ok(())
    }

    fn uninitialize(&mut self) {
        if self.active {
            unsafe { windows::Win32::System::WinRT::RoUninitialize() };
            self.active = false;
        }
    }
}

impl Drop for RoApartment {
    fn drop(&mut self) {
        self.uninitialize();
    }
}

#[pymethods]
impl RoApartment {
    #[new]
    #[pyo3(signature = (apartment_type=None))]
    fn new(apartment_type: Option<i32>) -> Self {
        Self {
            apartment_type: apartment_type.unwrap_or(1),
            active: false,
        }
    }

    fn __enter__(mut slf: PyRefMut<'_, Self>) -> PyResult<PyRefMut<'_, Self>> {
        slf.initialize()?;
        Ok(slf)
    }

    fn __exit__(
        &mut self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> bool {
        self.uninitialize();
        false
    }

    fn close(&mut self) {
        self.uninitialize();
    }

    fn __repr__(&self) -> String {
        format!(
            "RoApartment(apartment_type={}, active={})",
            self.apartment_type, self.active
        )
    }
}

#[pyfunction]
pub fn init_winappsdk(major: u32, minor: u32) -> PyResult<WinAppSDKContext> {
    dynwinrt::initialize_winappsdk(major, minor)
        .map(WinAppSDKContext)
        .map_err(map_dynwinrt_error)
}

#[pyfunction]
pub fn ro_initialize(apartment_type: Option<i32>) -> PyResult<()> {
    use windows::Win32::System::WinRT::{
        RO_INIT_MULTITHREADED, RO_INIT_SINGLETHREADED, RoInitialize,
    };
    let init_type = match apartment_type.unwrap_or(1) {
        0 => RO_INIT_SINGLETHREADED,
        _ => RO_INIT_MULTITHREADED,
    };
    unsafe { RoInitialize(init_type) }.map_err(map_windows_error)
}

#[pyfunction]
pub fn ro_uninitialize() {
    use windows::Win32::System::WinRT::RoUninitialize;
    unsafe { RoUninitialize() };
}

// ======================================================================
// WinGUID
// ======================================================================

#[pyclass(from_py_object)]
#[derive(Debug, Clone, Copy)]
pub struct WinGUID(pub(crate) GUID);

#[pymethods]
impl WinGUID {
    #[staticmethod]
    fn parse(guid_str: &str) -> PyResult<Self> {
        let guid = GUID::try_from(guid_str)
            .map_err(|e| PyRuntimeError::new_err(format!("Invalid GUID: {:?}", e)))?;
        Ok(WinGUID(guid))
    }

    fn to_string(&self) -> String {
        format!("{:?}", self.0)
    }

    fn __repr__(&self) -> String {
        format!("WinGUID({:?})", self.0)
    }
}

// ======================================================================
// DynWinRTType — wraps TypeHandle
// ======================================================================

#[pyclass(from_py_object)]
#[derive(Clone)]
pub struct DynWinRTType(pub(crate) dynwinrt::TypeHandle);

#[pymethods]
impl DynWinRTType {
    // -- Primitive types --

    #[staticmethod]
    fn i32_type() -> Self {
        DynWinRTType(TABLE.i32_type())
    }
    #[staticmethod]
    fn i64_type() -> Self {
        DynWinRTType(TABLE.i64_type())
    }
    #[staticmethod]
    fn hstring() -> Self {
        DynWinRTType(TABLE.hstring())
    }
    #[staticmethod]
    fn object() -> Self {
        DynWinRTType(TABLE.object())
    }
    #[staticmethod]
    fn f64_type() -> Self {
        DynWinRTType(TABLE.f64_type())
    }
    #[staticmethod]
    fn f32_type() -> Self {
        DynWinRTType(TABLE.f32_type())
    }
    #[staticmethod]
    fn u8_type() -> Self {
        DynWinRTType(TABLE.u8_type())
    }
    #[staticmethod]
    fn u16_type() -> Self {
        DynWinRTType(TABLE.u16_type())
    }
    #[staticmethod]
    fn u32_type() -> Self {
        DynWinRTType(TABLE.u32_type())
    }
    #[staticmethod]
    fn u64_type() -> Self {
        DynWinRTType(TABLE.u64_type())
    }
    #[staticmethod]
    fn i8_type() -> Self {
        DynWinRTType(TABLE.i8_type())
    }
    #[staticmethod]
    fn i16_type() -> Self {
        DynWinRTType(TABLE.i16_type())
    }
    #[staticmethod]
    fn bool_type() -> Self {
        DynWinRTType(TABLE.bool_type())
    }
    #[staticmethod]
    fn guid_type() -> Self {
        DynWinRTType(TABLE.guid_type())
    }
    #[staticmethod]
    fn char16() -> Self {
        DynWinRTType(TABLE.char16_type())
    }
    #[staticmethod]
    fn hresult() -> Self {
        DynWinRTType(TABLE.hresult())
    }

    // -- Class / interface types --

    #[staticmethod]
    fn runtime_class(name: String, default_interface_type: &DynWinRTType) -> Self {
        DynWinRTType(TABLE.runtime_class(name, &default_interface_type.0))
    }

    #[staticmethod]
    fn interface(iid: &WinGUID) -> Self {
        DynWinRTType(TABLE.interface(iid.0))
    }

    #[staticmethod]
    fn delegate(iid: &WinGUID) -> Self {
        DynWinRTType(TABLE.delegate(iid.0))
    }

    // -- Async types --

    #[staticmethod]
    fn i_async_action() -> Self {
        DynWinRTType(TABLE.async_action())
    }

    #[staticmethod]
    fn i_async_action_with_progress(progress_type: &DynWinRTType) -> Self {
        DynWinRTType(TABLE.async_action_with_progress(&progress_type.0))
    }

    #[staticmethod]
    fn i_async_operation(result_type: &DynWinRTType) -> Self {
        DynWinRTType(TABLE.async_operation(&result_type.0))
    }

    #[staticmethod]
    fn i_async_operation_with_progress(
        result_type: &DynWinRTType,
        progress_type: &DynWinRTType,
    ) -> Self {
        DynWinRTType(TABLE.async_operation_with_progress(&result_type.0, &progress_type.0))
    }

    // -- Composite types --

    #[staticmethod]
    fn struct_type(name: String, fields: Vec<DynWinRTType>) -> Self {
        let handles: Vec<dynwinrt::TypeHandle> = fields.iter().map(|f| f.0.clone()).collect();
        DynWinRTType(TABLE.struct_type(&name, &handles))
    }

    #[staticmethod]
    #[pyo3(signature = (name, member_names=None, member_values=None))]
    fn enum_type(
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
    #[staticmethod]
    fn get_enum_value(enum_name: String, member_name: String) -> Option<i32> {
        TABLE.get_enum_value(&enum_name, &member_name)
    }

    #[staticmethod]
    fn parameterized(generic_iid: &WinGUID, args: Vec<DynWinRTType>) -> Self {
        let handles: Vec<dynwinrt::TypeHandle> = args.iter().map(|a| a.0.clone()).collect();
        let generic = TABLE.generic(generic_iid.0, handles.len() as u32);
        DynWinRTType(TABLE.parameterized(&generic, &handles))
    }

    #[staticmethod]
    fn array_type(element_type: &DynWinRTType) -> Self {
        DynWinRTType(TABLE.array(&element_type.0))
    }

    // -- Interface registration & method management --

    #[staticmethod]
    fn register_interface(name: String, iid: &WinGUID) -> Self {
        DynWinRTType(TABLE.register_interface(&name, iid.0))
    }

    /// Add a method to this interface. Returns new DynWinRTType for chaining.
    fn add_method(&self, name: String, sig: &DynWinRTMethodSig) -> DynWinRTType {
        DynWinRTType(self.0.clone().add_method(&name, sig.0.clone()))
    }

    /// Get a MethodHandle by vtable index (6 = first user method).
    fn method(&self, vtable_index: usize) -> PyResult<DynWinRTMethodHandle> {
        self.0
            .method(vtable_index)
            .map(DynWinRTMethodHandle)
            .ok_or_else(|| {
                PyRuntimeError::new_err(format!("No method at vtable index {}", vtable_index))
            })
    }

    /// Get a MethodHandle by method name.
    fn method_by_name(&self, name: &str) -> PyResult<DynWinRTMethodHandle> {
        self.0
            .method_by_name(name)
            .map(DynWinRTMethodHandle)
            .ok_or_else(|| PyRuntimeError::new_err(format!("Method '{}' not found", name)))
    }

    /// Compute the IID for this type.
    fn iid(&self) -> PyResult<WinGUID> {
        self.0
            .iid()
            .map(WinGUID)
            .ok_or_else(|| PyRuntimeError::new_err("Type has no IID"))
    }

    fn __repr__(&self) -> String {
        format!("DynWinRTType({:?})", self.0.kind())
    }
}

// ======================================================================
// DynWinRTMethodSig — builder for method parameter descriptions
// ======================================================================

#[pyclass(from_py_object)]
#[derive(Clone)]
pub struct DynWinRTMethodSig(pub(crate) dynwinrt::MethodSignature);

#[pymethods]
impl DynWinRTMethodSig {
    #[new]
    fn new() -> Self {
        DynWinRTMethodSig(dynwinrt::MethodSignature::new(&*TABLE))
    }

    /// Add an [in] parameter. Returns new sig for chaining.
    fn add_in(&self, typ: &DynWinRTType) -> DynWinRTMethodSig {
        DynWinRTMethodSig(self.0.clone().add_in(typ.0.clone()))
    }

    /// Add an [out] parameter. Returns new sig for chaining.
    fn add_out(&self, typ: &DynWinRTType) -> DynWinRTMethodSig {
        DynWinRTMethodSig(self.0.clone().add_out(typ.0.clone()))
    }

    /// Add a FillArray [out] parameter. Returns new sig for chaining.
    fn add_out_fill(&self, typ: &DynWinRTType) -> DynWinRTMethodSig {
        DynWinRTMethodSig(self.0.clone().add_out_fill(typ.0.clone()))
    }
}

// ======================================================================
// DynWinRTMethodHandle — method invocation wrapper
// ======================================================================

#[pyclass]
pub struct DynWinRTMethodHandle(dynwinrt::MethodHandle);

#[pymethods]
impl DynWinRTMethodHandle {
    /// Invoke this method on a COM object.
    fn invoke(&self, obj: &DynWinRTValue, args: Vec<DynWinRTValue>) -> PyResult<DynWinRTValue> {
        let raw = match &obj.0 {
            dynwinrt::WinRTValue::Object(o) => o.as_raw(),
            _ => return Err(PyRuntimeError::new_err("invoke() requires an Object value")),
        };
        let wrt_args: Vec<dynwinrt::WinRTValue> = args.iter().map(|a| a.0.clone()).collect();
        let results = self.0.invoke(raw, &wrt_args).map_err(map_dynwinrt_error)?;
        if results.is_empty() {
            Ok(DynWinRTValue(dynwinrt::WinRTValue::I32(0)))
        } else {
            Ok(DynWinRTValue(results.into_iter().next().unwrap()))
        }
    }

    /// Like `invoke`, but returns all out-parameters as a list.
    /// Used for methods with multiple out params (e.g. IVector.IndexOf → [index, found]).
    fn invoke_all(
        &self,
        obj: &DynWinRTValue,
        args: Vec<DynWinRTValue>,
    ) -> PyResult<Vec<DynWinRTValue>> {
        let raw = match &obj.0 {
            dynwinrt::WinRTValue::Object(o) => o.as_raw(),
            _ => {
                return Err(PyRuntimeError::new_err(
                    "invoke_all() requires an Object value",
                ));
            }
        };
        let wrt_args: Vec<dynwinrt::WinRTValue> = args.iter().map(|a| a.0.clone()).collect();
        let results = self.0.invoke(raw, &wrt_args).map_err(map_dynwinrt_error)?;
        Ok(results.into_iter().map(DynWinRTValue).collect())
    }

    // --- Fast paths: skip Vec alloc for common getter patterns ---

    /// Getter → string (0 args, zero Vec allocation)
    fn get_string(&self, obj: &DynWinRTValue) -> PyResult<String> {
        let raw = obj
            .0
            .as_object()
            .ok_or_else(|| PyRuntimeError::new_err("get_string: not an Object"))?
            .as_raw();
        let hs = self
            .0
            .call_getter_hstring(raw)
            .map_err(map_dynwinrt_error)?;
        Ok(hs.to_string())
    }

    /// Getter → i32 (0 args, zero Vec allocation)
    fn get_i32(&self, obj: &DynWinRTValue) -> PyResult<i32> {
        let raw = obj
            .0
            .as_object()
            .ok_or_else(|| PyRuntimeError::new_err("get_i32: not an Object"))?
            .as_raw();
        self.0.call_getter_i32(raw).map_err(map_dynwinrt_error)
    }

    /// Getter → bool (0 args, zero Vec allocation)
    fn get_bool(&self, obj: &DynWinRTValue) -> PyResult<bool> {
        let raw = obj
            .0
            .as_object()
            .ok_or_else(|| PyRuntimeError::new_err("get_bool: not an Object"))?
            .as_raw();
        self.0.call_getter_bool(raw).map_err(map_dynwinrt_error)
    }

    /// Getter → DynWinRTValue object (0 args, zero Vec allocation)
    fn get_obj(&self, obj: &DynWinRTValue) -> PyResult<DynWinRTValue> {
        let raw = obj
            .0
            .as_object()
            .ok_or_else(|| PyRuntimeError::new_err("get_obj: not an Object"))?
            .as_raw();
        self.0
            .call_getter_object(raw)
            .map(DynWinRTValue)
            .map_err(map_dynwinrt_error)
    }

    /// 1-arg invoke with hstring input → DynWinRTValue result
    fn invoke_hstring(&self, obj: &DynWinRTValue, arg: String) -> PyResult<DynWinRTValue> {
        let raw = obj
            .0
            .as_object()
            .ok_or_else(|| PyRuntimeError::new_err("invoke_hstring: not an Object"))?
            .as_raw();
        let results = self
            .0
            .invoke(raw, &[dynwinrt::WinRTValue::HString(HSTRING::from(arg))])
            .map_err(map_dynwinrt_error)?;
        Ok(DynWinRTValue(results.into_iter().next().ok_or_else(
            || PyRuntimeError::new_err("invoke_hstring: no result"),
        )?))
    }

    /// 1-arg invoke with i32 input → DynWinRTValue result
    fn invoke_i32(&self, obj: &DynWinRTValue, arg: i32) -> PyResult<DynWinRTValue> {
        let raw = obj
            .0
            .as_object()
            .ok_or_else(|| PyRuntimeError::new_err("invoke_i32: not an Object"))?
            .as_raw();
        let results = self
            .0
            .invoke(raw, &[dynwinrt::WinRTValue::I32(arg)])
            .map_err(map_dynwinrt_error)?;
        Ok(DynWinRTValue(results.into_iter().next().ok_or_else(
            || PyRuntimeError::new_err("invoke_i32: no result"),
        )?))
    }
}

// ======================================================================
// DynWinRTValue — main value container
// ======================================================================

#[pyclass(from_py_object)]
#[derive(Clone)]
pub struct DynWinRTValue(pub(crate) dynwinrt::WinRTValue);

#[pymethods]
impl DynWinRTValue {
    #[staticmethod]
    fn activation_factory(name: String) -> PyResult<DynWinRTValue> {
        dynwinrt::ro_get_activation_factory_2(&HSTRING::from(name))
            .map(DynWinRTValue)
            .map_err(map_dynwinrt_error)
    }

    /// Compose a WinUI `Microsoft.UI.Xaml.Application` whose outer object
    /// exposes the supplied `IXamlMetadataProvider`.
    ///
    /// Mirrors JS `DynWinRtValue.createXamlApplication(metadataProvider, launchedCallback)`.
    #[staticmethod]
    #[pyo3(signature = (metadata_provider, launched_callback=None))]
    fn create_xaml_application(
        metadata_provider: &DynWinRTValue,
        launched_callback: Option<&DynWinRTValue>,
    ) -> PyResult<DynWinRTValue> {
        let provider = metadata_provider.0.as_object().ok_or_else(|| {
            PyRuntimeError::new_err("create_xaml_application: metadata_provider must be an Object")
        })?;
        let callback = launched_callback
            .map(|value| {
                value.0.as_object().ok_or_else(|| {
                    PyRuntimeError::new_err(
                        "create_xaml_application: launched_callback must be an Object",
                    )
                })
            })
            .transpose()?;
        dynwinrt::create_xaml_application(&provider, callback.as_ref())
            .map(DynWinRTValue)
            .map_err(map_dynwinrt_error)
    }

    // -- Scalar constructors (full parity with JS) --

    #[staticmethod]
    fn from_bool(value: bool) -> DynWinRTValue {
        DynWinRTValue(dynwinrt::WinRTValue::Bool(value))
    }
    #[staticmethod]
    fn from_i8(value: i32) -> DynWinRTValue {
        DynWinRTValue(dynwinrt::WinRTValue::I8(value as i8))
    }
    #[staticmethod]
    fn from_u8(value: u32) -> DynWinRTValue {
        DynWinRTValue(dynwinrt::WinRTValue::U8(value as u8))
    }
    #[staticmethod]
    fn from_i16(value: i32) -> DynWinRTValue {
        DynWinRTValue(dynwinrt::WinRTValue::I16(value as i16))
    }
    #[staticmethod]
    fn from_u16(value: u32) -> DynWinRTValue {
        DynWinRTValue(dynwinrt::WinRTValue::U16(value as u16))
    }
    #[staticmethod]
    fn from_i32(value: i32) -> DynWinRTValue {
        DynWinRTValue(dynwinrt::WinRTValue::I32(value))
    }
    #[staticmethod]
    fn from_u32(value: u32) -> DynWinRTValue {
        DynWinRTValue(dynwinrt::WinRTValue::U32(value))
    }
    #[staticmethod]
    fn from_i64(value: i64) -> DynWinRTValue {
        DynWinRTValue(dynwinrt::WinRTValue::I64(value))
    }
    #[staticmethod]
    fn from_u64(value: u64) -> DynWinRTValue {
        DynWinRTValue(dynwinrt::WinRTValue::U64(value))
    }
    #[staticmethod]
    fn from_f32(value: f32) -> DynWinRTValue {
        DynWinRTValue(dynwinrt::WinRTValue::F32(value))
    }
    #[staticmethod]
    fn from_f64(value: f64) -> DynWinRTValue {
        DynWinRTValue(dynwinrt::WinRTValue::F64(value))
    }
    #[staticmethod]
    fn from_hstring(value: String) -> DynWinRTValue {
        DynWinRTValue(dynwinrt::WinRTValue::HString(HSTRING::from(value)))
    }
    #[staticmethod]
    fn from_guid(value: &WinGUID) -> DynWinRTValue {
        DynWinRTValue(dynwinrt::WinRTValue::Guid(value.0))
    }
    #[staticmethod]
    fn null_value() -> DynWinRTValue {
        DynWinRTValue(dynwinrt::WinRTValue::Null)
    }

    /// Create an enum value from an i32 and its type handle.
    #[staticmethod]
    fn enum_value(enum_type: &DynWinRTType, value: i32) -> DynWinRTValue {
        DynWinRTValue(dynwinrt::WinRTValue::Enum {
            value,
            type_handle: enum_type.0.clone(),
        })
    }

    #[staticmethod]
    fn box_reference(value: &DynWinRTValue, value_type: &DynWinRTType) -> PyResult<DynWinRTValue> {
        dynwinrt::box_ireference(value.0.clone(), value_type.0.clone())
            .map(DynWinRTValue)
            .map_err(map_dynwinrt_error)
    }

    /// Get the i32 value of an enum. Returns None if not an enum.
    fn get_enum_int(&self) -> Option<i32> {
        match &self.0 {
            dynwinrt::WinRTValue::Enum { value, .. } => Some(*value),
            _ => None,
        }
    }

    /// Get the member name of an enum value.
    fn get_enum_name(&self) -> Option<String> {
        match &self.0 {
            dynwinrt::WinRTValue::Enum { value, type_handle } => {
                type_handle.enum_member_name(*value)
            }
            _ => None,
        }
    }

    /// Create an IVector<T> from items.
    #[staticmethod]
    fn create_vector(
        items: Vec<DynWinRTValue>,
        element_type: &DynWinRTType,
    ) -> PyResult<DynWinRTValue> {
        let iids = TABLE.vector_iids(&element_type.0);
        let wrt_items: Vec<dynwinrt::WinRTValue> = items.iter().map(|i| i.0.clone()).collect();
        let vector = dynwinrt::vector::create_vector_from_values(&wrt_items, &element_type.0, iids)
            .map_err(map_dynwinrt_error)?;
        Ok(DynWinRTValue(dynwinrt::WinRTValue::Object(vector)))
    }

    /// Create an IMap<K,V> from parallel key/value lists.
    #[staticmethod]
    fn create_map(
        keys: Vec<DynWinRTValue>,
        values: Vec<DynWinRTValue>,
        key_type: &DynWinRTType,
        value_type: &DynWinRTType,
    ) -> PyResult<DynWinRTValue> {
        if keys.len() != values.len() {
            return Err(PyRuntimeError::new_err(
                "create_map: keys and values must have the same length",
            ));
        }
        let iids = TABLE.map_iids(&key_type.0, &value_type.0);
        let entries: Vec<(dynwinrt::WinRTValue, dynwinrt::WinRTValue)> = keys
            .iter()
            .zip(values.iter())
            .map(|(key, value)| (key.0.clone(), value.0.clone()))
            .collect();
        let map = dynwinrt::map::create_map_from_values(&entries, &key_type.0, &value_type.0, iids)
            .map_err(map_dynwinrt_error)?;
        Ok(DynWinRTValue(dynwinrt::WinRTValue::Object(map)))
    }

    /// Await an async WinRT operation (blocks the current thread).
    /// Releases the Python GIL while waiting so other threads can proceed.
    fn wait(&self, py: Python<'_>) -> PyResult<DynWinRTValue> {
        super::async_runtime::wait_for_async(&self.0, py).map(DynWinRTValue)
    }

    fn _get_async_results(&self) -> PyResult<DynWinRTValue> {
        dynwinrt::get_async_results(&self.0)
            .map(DynWinRTValue)
            .map_err(map_dynwinrt_error)
    }

    /// Cancel the underlying WinRT async operation (calls `IAsyncInfo::Cancel`).
    /// Safe to call multiple times or on already-completed operations.
    ///
    /// Raises if this value is not an async operation.
    fn cancel(&self) -> PyResult<()> {
        let async_info = match &self.0 {
            dynwinrt::WinRTValue::Async(a) => a,
            _ => return Err(PyRuntimeError::new_err("cancel: not an async value")),
        };
        async_info
            .cancel()
            .map_err(|error| map_dynwinrt_error_with_context(error, "Cancel failed"))
    }

    /// Register a progress callback on an async-with-progress operation.
    fn on_progress(&self, callback: Py<PyAny>) -> PyResult<()> {
        let async_info = match &self.0 {
            dynwinrt::WinRTValue::Async(a) => a,
            _ => return Err(PyRuntimeError::new_err("on_progress: not an async value")),
        };

        let progress_type = async_info
            .progress_type()
            .ok_or_else(|| PyRuntimeError::new_err("on_progress: not a WithProgress async type"))?;
        super::async_runtime::ensure_progress_type_supported(&progress_type)?;

        let handler_iid = async_info.progress_handler_iid().ok_or_else(|| {
            PyRuntimeError::new_err("on_progress: cannot compute progress handler IID")
        })?;

        let progress_cb: dynwinrt::ProgressCallback = Box::new(move |val: dynwinrt::WinRTValue| {
            Python::attach(|py| {
                let result = (|| -> PyResult<()> {
                    let py_val = Py::new(py, DynWinRTValue(val))?;
                    callback.call1(py, (py_val,))?;
                    Ok(())
                })();
                if let Err(error) = result {
                    error.write_unraisable(py, Some(callback.bind(py)));
                }
            });
        });
        let handler = dynwinrt::create_progress_handler(handler_iid, progress_type, progress_cb);

        async_info
            .set_progress_handler(&handler)
            .map_err(|error| map_dynwinrt_error_with_context(error, "SetProgress failed"))?;

        Ok(())
    }

    // -- Conversion methods --

    fn to_string(&self) -> String {
        match &self.0 {
            dynwinrt::WinRTValue::HString(s) => s.to_string(),
            dynwinrt::WinRTValue::I32(i) => i.to_string(),
            dynwinrt::WinRTValue::I64(i) => i.to_string(),
            dynwinrt::WinRTValue::U32(i) => i.to_string(),
            dynwinrt::WinRTValue::U64(i) => i.to_string(),
            dynwinrt::WinRTValue::F32(f) => f.to_string(),
            dynwinrt::WinRTValue::F64(f) => f.to_string(),
            dynwinrt::WinRTValue::Bool(b) => b.to_string(),
            dynwinrt::WinRTValue::Object(o) => format!("Object({:?})", o),
            dynwinrt::WinRTValue::Enum { value, type_handle } => {
                if let Some(name) = type_handle.enum_member_name(*value) {
                    name
                } else {
                    value.to_string()
                }
            }
            _ => "Unsupported type".to_string(),
        }
    }

    fn __repr__(&self) -> String {
        format!("DynWinRTValue({})", self.to_string())
    }

    fn __str__(&self) -> String {
        self.to_string()
    }

    fn to_number(&self) -> PyResult<i64> {
        match &self.0 {
            dynwinrt::WinRTValue::Bool(b) => Ok(if *b { 1 } else { 0 }),
            dynwinrt::WinRTValue::I8(i) => Ok(*i as i64),
            dynwinrt::WinRTValue::U8(i) => Ok(*i as i64),
            dynwinrt::WinRTValue::I16(i) => Ok(*i as i64),
            dynwinrt::WinRTValue::U16(i) => Ok(*i as i64),
            dynwinrt::WinRTValue::I32(i) => Ok(*i as i64),
            dynwinrt::WinRTValue::U32(i) => Ok(*i as i64),
            dynwinrt::WinRTValue::HResult(hr) => Ok(hr.0 as i64),
            dynwinrt::WinRTValue::Enum { value, .. } => Ok(*value as i64),
            _ => Err(PyRuntimeError::new_err(format!(
                "Cannot convert {:?} to number",
                self.0.get_type_kind()
            ))),
        }
    }

    fn to_int(&self) -> PyResult<i128> {
        match &self.0 {
            dynwinrt::WinRTValue::I32(i) => Ok(*i as i128),
            dynwinrt::WinRTValue::I64(i) => Ok(*i as i128),
            dynwinrt::WinRTValue::U32(i) => Ok(*i as i128),
            dynwinrt::WinRTValue::U64(i) => Ok(*i as i128),
            dynwinrt::WinRTValue::Bool(b) => Ok(*b as i128),
            dynwinrt::WinRTValue::I8(i) => Ok(*i as i128),
            dynwinrt::WinRTValue::U8(i) => Ok(*i as i128),
            dynwinrt::WinRTValue::I16(i) => Ok(*i as i128),
            dynwinrt::WinRTValue::U16(i) => Ok(*i as i128),
            dynwinrt::WinRTValue::Enum { value, .. } => Ok(*value as i128),
            _ => Err(PyRuntimeError::new_err("Cannot convert to int")),
        }
    }

    fn to_float(&self) -> PyResult<f64> {
        match &self.0 {
            dynwinrt::WinRTValue::F32(f) => Ok(*f as f64),
            dynwinrt::WinRTValue::F64(f) => Ok(*f),
            dynwinrt::WinRTValue::I32(i) => Ok(*i as f64),
            dynwinrt::WinRTValue::I64(i) => Ok(*i as f64),
            _ => Err(PyRuntimeError::new_err("Cannot convert to float")),
        }
    }

    fn to_bool(&self) -> PyResult<bool> {
        match &self.0 {
            dynwinrt::WinRTValue::Bool(b) => Ok(*b),
            _ => self.to_number().map(|n| n != 0),
        }
    }

    fn to_i64(&self) -> PyResult<i64> {
        match &self.0 {
            dynwinrt::WinRTValue::I64(i) => Ok(*i),
            dynwinrt::WinRTValue::U64(i) => i64::try_from(*i)
                .map_err(|_| PyRuntimeError::new_err("UInt64 value does not fit in Int64")),
            _ => self.to_number(),
        }
    }

    fn to_u32(&self) -> PyResult<u32> {
        match &self.0 {
            dynwinrt::WinRTValue::U32(i) => Ok(*i),
            _ => Err(PyRuntimeError::new_err("Value is not a UInt32")),
        }
    }

    fn to_u64(&self) -> PyResult<u64> {
        match &self.0 {
            dynwinrt::WinRTValue::U64(i) => Ok(*i),
            _ => Err(PyRuntimeError::new_err("Value is not a UInt64")),
        }
    }

    fn to_f64(&self) -> PyResult<f64> {
        match &self.0 {
            dynwinrt::WinRTValue::F64(f) => Ok(*f),
            dynwinrt::WinRTValue::F32(f) => Ok(*f as f64),
            _ => self.to_number().map(|n| n as f64),
        }
    }

    fn to_guid(&self) -> PyResult<WinGUID> {
        match &self.0 {
            dynwinrt::WinRTValue::Guid(g) => Ok(WinGUID(*g)),
            _ => Err(PyRuntimeError::new_err("Value is not a GUID")),
        }
    }

    fn is_null(&self) -> bool {
        self.0.is_null_object()
    }

    fn as_raw(&self) -> PyResult<i64> {
        match &self.0 {
            dynwinrt::WinRTValue::Object(o) => Ok(o.as_raw() as i64),
            _ => Err(PyRuntimeError::new_err(
                "Cannot get raw pointer from non-object",
            )),
        }
    }

    /// COM QueryInterface — cast to a different interface.
    fn cast(&self, iid: &WinGUID) -> PyResult<DynWinRTValue> {
        self.0
            .cast(&iid.0)
            .map(DynWinRTValue)
            .map_err(map_dynwinrt_error)
    }

    /// Call IActivationFactory::ActivateInstance (vtable[6]) to create a default instance.
    /// Use on the result of activation_factory() for classes with parameterless constructors.
    fn activate(&self) -> PyResult<DynWinRTValue> {
        let method = dynwinrt::MethodSignature::new(&*TABLE)
            .add_out(TABLE.object())
            .build(6);
        let raw = self
            .0
            .as_object()
            .ok_or_else(|| PyRuntimeError::new_err("activate: not an Object"))?
            .as_raw();
        let result = method.call_dynamic(raw, &[]).map_err(map_windows_error)?;
        Ok(DynWinRTValue(result.into_iter().next().ok_or_else(
            || PyRuntimeError::new_err("activate: no result"),
        )?))
    }

    // -- Convenience call methods (match JS API) --

    /// Call a method with no args and one out param.
    fn call_0(&self, method_index: usize, return_type: &DynWinRTType) -> PyResult<DynWinRTValue> {
        let method = dynwinrt::MethodSignature::new(&*TABLE)
            .add_out(return_type.0.clone())
            .build(method_index);
        let obj_raw = self
            .0
            .as_object()
            .ok_or_else(|| PyRuntimeError::new_err("call_0 requires an Object value"))?
            .as_raw();
        let result = method
            .call_dynamic(obj_raw, &[])
            .map_err(map_windows_error)?;
        Ok(DynWinRTValue(result.into_iter().next().unwrap()))
    }

    /// Call a method with one arg and one out param.
    fn call_1(
        &self,
        method_index: usize,
        return_type: &DynWinRTType,
        v1: &DynWinRTValue,
    ) -> PyResult<DynWinRTValue> {
        let in_type = TABLE.handle_from_kind(v1.0.get_type_kind());
        let method = dynwinrt::MethodSignature::new(&*TABLE)
            .add_in(in_type)
            .add_out(return_type.0.clone())
            .build(method_index);
        let obj_raw = self
            .0
            .as_object()
            .ok_or_else(|| PyRuntimeError::new_err("call_1 requires an Object value"))?
            .as_raw();
        let result = method
            .call_dynamic(obj_raw, &[v1.0.clone()])
            .map_err(map_windows_error)?;
        Ok(DynWinRTValue(result.into_iter().next().unwrap()))
    }

    /// General-purpose method call with explicit types and args.
    fn call(
        &self,
        method_index: usize,
        return_type: &DynWinRTType,
        in_types: Vec<DynWinRTType>,
        args: Vec<DynWinRTValue>,
    ) -> PyResult<DynWinRTValue> {
        let mut method = dynwinrt::MethodSignature::new(&*TABLE);
        for t in &in_types {
            method = method.add_in(t.0.clone());
        }
        method = method.add_out(return_type.0.clone());

        let obj = match &self.0 {
            dynwinrt::WinRTValue::Object(o) => o.as_raw(),
            _ => return Err(PyRuntimeError::new_err("call() requires an Object value")),
        };

        let mut iface =
            dynwinrt::InterfaceSignature::define_from_iinspectable("", Default::default(), &*TABLE);
        let target_index = method_index;
        for _ in 6..target_index {
            iface.add_method(dynwinrt::MethodSignature::new(&*TABLE));
        }
        iface.add_method(method);

        let winrt_args: Vec<dynwinrt::WinRTValue> = args.iter().map(|a| a.0.clone()).collect();
        let result = iface.methods[target_index]
            .call_dynamic(obj, &winrt_args)
            .map_err(map_windows_error)?;

        if result.is_empty() {
            Ok(DynWinRTValue(dynwinrt::WinRTValue::I32(0)))
        } else {
            Ok(DynWinRTValue(result.into_iter().next().unwrap()))
        }
    }

    // -- Array / Struct extraction --

    fn is_array(&self) -> bool {
        self.0.as_array().is_some()
    }

    fn as_array(&self) -> PyResult<DynWinRTArray> {
        match &self.0 {
            dynwinrt::WinRTValue::Array(data) => Ok(DynWinRTArray(data.clone())),
            _ => Err(PyRuntimeError::new_err("Value is not an Array")),
        }
    }

    fn is_struct(&self) -> bool {
        self.0.as_struct().is_some()
    }

    fn as_struct(&self) -> PyResult<DynWinRTStruct> {
        match &self.0 {
            dynwinrt::WinRTValue::Struct(data) => Ok(DynWinRTStruct(data.clone())),
            _ => Err(PyRuntimeError::new_err("Value is not a Struct")),
        }
    }
}

// ======================================================================
// DynWinRTArray — array container with blittable fast paths
// ======================================================================

#[pyclass(unsendable, from_py_object)]
#[derive(Clone)]
pub struct DynWinRTArray(dynwinrt::ArrayData);

#[pymethods]
impl DynWinRTArray {
    fn __len__(&self) -> usize {
        self.0.len()
    }

    /// Per-element access.
    fn get(&self, index: usize) -> DynWinRTValue {
        DynWinRTValue(self.0.get(index))
    }

    /// Convert all elements to a list of DynWinRTValue.
    fn to_values(&self) -> Vec<DynWinRTValue> {
        (0..self.0.len())
            .map(|i| DynWinRTValue(self.0.get(i)))
            .collect()
    }

    // -- Typed list extraction (works for both Values and CoTaskMem arrays) --

    fn to_i8_list(&self) -> Vec<i32> {
        (0..self.0.len())
            .map(|i| self.0.get(i).as_i32().unwrap_or(0))
            .collect()
    }
    fn to_u8_list(&self) -> Vec<u8> {
        (0..self.0.len())
            .map(|i| match self.0.get(i) {
                dynwinrt::WinRTValue::U8(v) => v,
                other => other.as_i32().unwrap_or(0) as u8,
            })
            .collect()
    }
    fn to_i16_list(&self) -> Vec<i32> {
        (0..self.0.len())
            .map(|i| self.0.get(i).as_i32().unwrap_or(0))
            .collect()
    }
    fn to_u16_list(&self) -> Vec<u32> {
        (0..self.0.len())
            .map(|i| self.0.get(i).as_i32().unwrap_or(0) as u32)
            .collect()
    }
    fn to_i32_list(&self) -> Vec<i32> {
        (0..self.0.len())
            .map(|i| self.0.get(i).as_i32().unwrap_or(0))
            .collect()
    }
    fn to_u32_list(&self) -> Vec<u32> {
        (0..self.0.len())
            .map(|i| self.0.get(i).as_i32().unwrap_or(0) as u32)
            .collect()
    }
    fn to_f32_list(&self) -> Vec<f32> {
        (0..self.0.len())
            .map(|i| match self.0.get(i) {
                dynwinrt::WinRTValue::F32(v) => v,
                dynwinrt::WinRTValue::F64(v) => v as f32,
                other => other.as_i32().unwrap_or(0) as f32,
            })
            .collect()
    }
    fn to_f64_list(&self) -> Vec<f64> {
        (0..self.0.len())
            .map(|i| match self.0.get(i) {
                dynwinrt::WinRTValue::F64(v) => v,
                dynwinrt::WinRTValue::F32(v) => v as f64,
                other => other.as_i32().unwrap_or(0) as f64,
            })
            .collect()
    }
    fn to_i64_list(&self) -> Vec<i64> {
        (0..self.0.len())
            .map(|i| match self.0.get(i) {
                dynwinrt::WinRTValue::I64(v) => v,
                other => other.as_i32().unwrap_or(0) as i64,
            })
            .collect()
    }
    fn to_u64_list(&self) -> Vec<u64> {
        (0..self.0.len())
            .map(|i| match self.0.get(i) {
                dynwinrt::WinRTValue::U64(v) => v,
                other => other.as_i32().unwrap_or(0) as u64,
            })
            .collect()
    }
    fn to_string_list(&self) -> Vec<String> {
        (0..self.0.len())
            .map(|i| match self.0.get(i) {
                dynwinrt::WinRTValue::HString(s) => s.to_string(),
                other => format!("{:?}", other),
            })
            .collect()
    }

    // -- Construction from Python lists --

    #[staticmethod]
    fn from_i8_values(values: Vec<i32>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> = values
            .into_iter()
            .map(|v| dynwinrt::WinRTValue::I8(v as i8))
            .collect();
        DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.i8_type(), &wvals))
    }
    #[staticmethod]
    fn from_u8_values(values: Vec<u8>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> =
            values.into_iter().map(dynwinrt::WinRTValue::U8).collect();
        DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.u8_type(), &wvals))
    }
    #[staticmethod]
    fn from_i16_values(values: Vec<i32>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> = values
            .into_iter()
            .map(|v| dynwinrt::WinRTValue::I16(v as i16))
            .collect();
        DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.i16_type(), &wvals))
    }
    #[staticmethod]
    fn from_u16_values(values: Vec<u32>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> = values
            .into_iter()
            .map(|v| dynwinrt::WinRTValue::U16(v as u16))
            .collect();
        DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.u16_type(), &wvals))
    }
    #[staticmethod]
    fn from_i32_values(values: Vec<i32>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> =
            values.into_iter().map(dynwinrt::WinRTValue::I32).collect();
        DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.i32_type(), &wvals))
    }
    #[staticmethod]
    fn from_u32_values(values: Vec<u32>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> =
            values.into_iter().map(dynwinrt::WinRTValue::U32).collect();
        DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.u32_type(), &wvals))
    }
    #[staticmethod]
    fn from_f32_values(values: Vec<f32>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> =
            values.into_iter().map(dynwinrt::WinRTValue::F32).collect();
        DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.f32_type(), &wvals))
    }
    #[staticmethod]
    fn from_f64_values(values: Vec<f64>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> =
            values.into_iter().map(dynwinrt::WinRTValue::F64).collect();
        DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.f64_type(), &wvals))
    }
    #[staticmethod]
    fn from_i64_values(values: Vec<i64>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> =
            values.into_iter().map(dynwinrt::WinRTValue::I64).collect();
        DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.i64_type(), &wvals))
    }
    #[staticmethod]
    fn from_u64_values(values: Vec<u64>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> =
            values.into_iter().map(dynwinrt::WinRTValue::U64).collect();
        DynWinRTArray(dynwinrt::ArrayData::from_values(TABLE.u64_type(), &wvals))
    }
    #[staticmethod]
    fn from_string_values(values: Vec<String>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> = values
            .into_iter()
            .map(|s| dynwinrt::WinRTValue::HString(HSTRING::from(&s)))
            .collect();
        DynWinRTArray(dynwinrt::ArrayData::from_values(
            TABLE.make(dynwinrt::TypeKind::HString),
            &wvals,
        ))
    }

    #[staticmethod]
    fn from_values(values: Vec<DynWinRTValue>, element_type: &DynWinRTType) -> DynWinRTArray {
        let values: Vec<dynwinrt::WinRTValue> =
            values.iter().map(|value| value.0.clone()).collect();
        DynWinRTArray(dynwinrt::ArrayData::from_values(
            element_type.0.clone(),
            &values,
        ))
    }

    /// Build a DynWinRTArray of WinRT object/interface elements.
    ///
    /// Use for `T[]` ABI in-parameters where `T` is a runtime class or
    /// interface — for example, `ModelCatalog(ModelCatalogSource[] sources)`.
    /// Items are passed as DynWinRTValue handles (typically Object-wrapped),
    /// and the element type drives ABI size and IID computation.
    #[staticmethod]
    fn from_object_values(
        values: Vec<DynWinRTValue>,
        element_type: &DynWinRTType,
    ) -> DynWinRTArray {
        Self::from_values(values, element_type)
    }

    /// Return the u8 array data as a Python `bytes` object. Safe for both
    /// `Values`-backed and `CoTaskMem`-backed arrays.
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, pyo3::types::PyBytes> {
        let len = self.0.len();
        let mut buf: Vec<u8> = Vec::with_capacity(len);
        for i in 0..len {
            buf.push(match self.0.get(i) {
                dynwinrt::WinRTValue::U8(v) => v,
                other => other.as_i32().unwrap_or(0) as u8,
            });
        }
        pyo3::types::PyBytes::new(py, &buf)
    }

    /// Build a u8 DynWinRTArray from a Python `bytes` or `bytearray` (much more
    /// efficient than `from_u8_values` for large byte buffers because the caller
    /// avoids boxing each byte into a Python int).
    #[staticmethod]
    fn from_bytes(data: &Bound<'_, PyAny>) -> PyResult<DynWinRTArray> {
        let slice: Vec<u8> = if let Ok(b) = data.cast::<pyo3::types::PyBytes>() {
            b.as_bytes().to_vec()
        } else if let Ok(ba) = data.cast::<pyo3::types::PyByteArray>() {
            ba.to_vec()
        } else {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "from_bytes: expected bytes or bytearray",
            ));
        };
        let wvals: Vec<dynwinrt::WinRTValue> =
            slice.into_iter().map(dynwinrt::WinRTValue::U8).collect();
        Ok(DynWinRTArray(dynwinrt::ArrayData::from_values(
            TABLE.u8_type(),
            &wvals,
        )))
    }

    /// Wrap as DynWinRTValue::Array for passing to call().
    fn to_value(&self) -> DynWinRTValue {
        DynWinRTValue(dynwinrt::WinRTValue::Array(self.0.clone()))
    }

    fn __repr__(&self) -> String {
        format!("DynWinRTArray(len={})", self.0.len())
    }
}

// ======================================================================
// DynWinRTStruct — typed field access by index
// ======================================================================

#[pyclass(unsendable, from_py_object)]
#[derive(Clone)]
pub struct DynWinRTStruct(dynwinrt::ValueTypeData);

#[pymethods]
impl DynWinRTStruct {
    /// Create a zero-initialized struct of the given type.
    #[staticmethod]
    fn create(typ: &DynWinRTType) -> DynWinRTStruct {
        DynWinRTStruct(typ.0.default_value())
    }

    // -- Blittable field access (get/set pairs) --

    fn get_i8(&self, index: usize) -> i32 {
        self.0.get_field::<i8>(index) as i32
    }
    fn set_i8(&mut self, index: usize, value: i32) {
        self.0.set_field(index, value as i8);
    }

    fn get_u8(&self, index: usize) -> u32 {
        self.0.get_field::<u8>(index) as u32
    }
    fn set_u8(&mut self, index: usize, value: u32) {
        self.0.set_field(index, value as u8);
    }

    fn get_i16(&self, index: usize) -> i32 {
        self.0.get_field::<i16>(index) as i32
    }
    fn set_i16(&mut self, index: usize, value: i32) {
        self.0.set_field(index, value as i16);
    }

    fn get_u16(&self, index: usize) -> u32 {
        self.0.get_field::<u16>(index) as u32
    }
    fn set_u16(&mut self, index: usize, value: u32) {
        self.0.set_field(index, value as u16);
    }

    fn get_i32(&self, index: usize) -> i32 {
        self.0.get_field::<i32>(index)
    }
    fn set_i32(&mut self, index: usize, value: i32) {
        self.0.set_field(index, value);
    }

    fn get_u32(&self, index: usize) -> u32 {
        self.0.get_field::<u32>(index)
    }
    fn set_u32(&mut self, index: usize, value: u32) {
        self.0.set_field(index, value);
    }

    fn get_f32(&self, index: usize) -> f64 {
        self.0.get_field::<f32>(index) as f64
    }
    fn set_f32(&mut self, index: usize, value: f64) {
        self.0.set_field(index, value as f32);
    }

    fn get_f64(&self, index: usize) -> f64 {
        self.0.get_field::<f64>(index)
    }
    fn set_f64(&mut self, index: usize, value: f64) {
        self.0.set_field(index, value);
    }

    fn get_i64(&self, index: usize) -> i64 {
        self.0.get_field::<i64>(index)
    }
    fn set_i64(&mut self, index: usize, value: i64) {
        self.0.set_field(index, value);
    }

    fn get_u64(&self, index: usize) -> u64 {
        self.0.get_field::<u64>(index)
    }
    fn set_u64(&mut self, index: usize, value: u64) {
        self.0.set_field(index, value);
    }

    // -- Non-blittable field access --

    fn get_hstring(&self, index: usize) -> String {
        let inner = self.0.get_field_struct(index);
        let hstr: HSTRING = unsafe {
            let raw = *(inner.as_ptr() as *const *mut std::ffi::c_void);
            if raw.is_null() {
                HSTRING::new()
            } else {
                let hstr_ref: &HSTRING =
                    &*((&raw) as *const *mut std::ffi::c_void as *const HSTRING);
                hstr_ref.clone()
            }
        };
        hstr.to_string()
    }

    fn set_hstring(&mut self, index: usize, value: String) {
        let hstr = HSTRING::from(&value);
        let field_handle = self.0.type_handle().field_type(index);
        let mut field_val = field_handle.default_value();
        unsafe {
            let raw: *mut std::ffi::c_void = std::mem::transmute(hstr);
            (field_val.as_mut_ptr() as *mut *mut std::ffi::c_void).write(raw);
        }
        self.0.set_field_struct(index, &field_val);
    }

    fn get_guid(&self, index: usize) -> WinGUID {
        WinGUID(self.0.get_field::<GUID>(index))
    }

    fn set_guid(&mut self, index: usize, value: &WinGUID) {
        self.0.set_field(index, value.0);
    }

    fn get_struct(&self, index: usize) -> DynWinRTStruct {
        DynWinRTStruct(self.0.get_field_struct(index))
    }

    fn set_struct(&mut self, index: usize, value: &DynWinRTStruct) {
        self.0.set_field_struct(index, &value.0);
    }

    fn get_object(&self, index: usize) -> PyResult<DynWinRTValue> {
        let inner = self.0.get_field_struct(index);
        let raw = unsafe { *(inner.as_ptr() as *const *mut std::ffi::c_void) };
        if raw.is_null() {
            Ok(DynWinRTValue(dynwinrt::WinRTValue::Null))
        } else {
            let obj = unsafe { IUnknown::from_raw_borrowed(&raw) }
                .ok_or_else(|| PyRuntimeError::new_err("null COM pointer"))?
                .clone();
            Ok(DynWinRTValue(dynwinrt::WinRTValue::Object(obj)))
        }
    }

    fn set_object(&mut self, index: usize, value: &DynWinRTValue) {
        match &value.0 {
            dynwinrt::WinRTValue::Object(obj) => {
                let field_handle = self.0.type_handle().field_type(index);
                let mut field_val = field_handle.default_value();
                unsafe {
                    let cloned = obj.clone();
                    let raw = cloned.into_raw();
                    (field_val.as_mut_ptr() as *mut *mut std::ffi::c_void).write(raw);
                }
                self.0.set_field_struct(index, &field_val);
            }
            dynwinrt::WinRTValue::Null => {
                let field_handle = self.0.type_handle().field_type(index);
                let field_val = field_handle.default_value();
                self.0.set_field_struct(index, &field_val);
            }
            _ => {}
        }
    }

    /// Wrap as DynWinRTValue::Struct for passing to call().
    fn to_value(&self) -> DynWinRTValue {
        DynWinRTValue(dynwinrt::WinRTValue::Struct(self.0.clone()))
    }

    fn __repr__(&self) -> String {
        "DynWinRTStruct(...)".to_string()
    }
}

// ======================================================================
// DynWinRtDelegate — dynamic WinRT delegate (callback) binding
// ======================================================================

#[pyclass]
pub struct DynWinRtDelegate(dynwinrt::WinRTValue);

const PYWINRT_E_UNRAISABLE_PYTHON_EXCEPTION: windows::core::HRESULT =
    windows::core::HRESULT(0xA0EE4005_u32 as i32);

fn create_python_delegate(
    iid: GUID,
    type_handles: Vec<dynwinrt::TypeHandle>,
    callback: Py<PyAny>,
) -> dynwinrt::WinRTValue {
    let delegate_callback: dynwinrt::delegate::DelegateCallback =
        Box::new(move |args: &[dynwinrt::WinRTValue]| {
            Python::attach(|py| {
                let result = (|| -> PyResult<()> {
                    let py_args = args
                        .iter()
                        .map(|arg| {
                            Ok(DynWinRTValue(arg.clone())
                                .into_pyobject(py)?
                                .into_any()
                                .unbind())
                        })
                        .collect::<PyResult<Vec<Py<PyAny>>>>()?;
                    let py_tuple = pyo3::types::PyTuple::new(py, &py_args)?;
                    callback.call1(py, py_tuple)?;
                    Ok(())
                })();
                match result {
                    Ok(()) => windows::core::HRESULT(0),
                    Err(error) => {
                        error.write_unraisable(py, Some(callback.bind(py)));
                        PYWINRT_E_UNRAISABLE_PYTHON_EXCEPTION
                    }
                }
            })
        });
    dynwinrt::delegate::create_delegate_value(iid, type_handles, delegate_callback)
}

#[pymethods]
impl DynWinRtDelegate {
    /// Create a delegate COM object from a Python callback function.
    ///
    /// - `iid`: delegate interface IID
    /// - `param_types`: Invoke parameter types
    /// - `callback`: Python callable invoked when WinRT fires the event
    #[staticmethod]
    fn create(
        iid: &WinGUID,
        param_types: Vec<PyRef<DynWinRTType>>,
        callback: Py<PyAny>,
    ) -> PyResult<DynWinRtDelegate> {
        let type_handles: Vec<dynwinrt::TypeHandle> =
            param_types.iter().map(|t| t.0.clone()).collect();
        let value = create_python_delegate(iid.0, type_handles, callback);
        Ok(DynWinRtDelegate(value))
    }

    /// Get the delegate as a DynWinRTValue for passing to WinRT methods.
    fn to_value(&self) -> DynWinRTValue {
        DynWinRTValue(self.0.clone())
    }

    fn __repr__(&self) -> String {
        "DynWinRtDelegate(...)".to_string()
    }
}

// ======================================================================
// System info utilities
// ======================================================================

#[pyfunction]
pub fn has_package_identity() -> bool {
    use windows::ApplicationModel::AppInfo;
    AppInfo::Current().is_ok()
}

/// Return the framework resources.pri path selected by init_winappsdk.
/// The Windows App SDK must have been initialized (via `init_winappsdk`)
/// before calling this function.
#[pyfunction]
pub fn get_winappsdk_resource_pri_path() -> PyResult<String> {
    dynwinrt::WinAppSdkContext {}
        .resource_pri_path()
        .map_err(map_windows_error)
}

#[pyfunction]
pub fn get_computer_name() -> PyResult<String> {
    use windows::Win32::System::WindowsProgramming::GetComputerNameW;
    use windows::core::PWSTR;

    let mut buffer = [0u16; 256];
    let mut size = buffer.len() as u32;

    unsafe {
        if GetComputerNameW(Some(PWSTR(buffer.as_mut_ptr())), &mut size).is_ok() {
            Ok(String::from_utf16_lossy(&buffer[..size as usize]))
        } else {
            Err(PyRuntimeError::new_err("Failed to get computer name"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::types::PyDict;

    #[repr(C)]
    struct TestDelegateVtbl {
        base: windows::core::IUnknown_Vtbl,
        invoke: unsafe extern "system" fn(
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
        ) -> windows::core::HRESULT,
    }

    unsafe fn invoke_delegate(value: &dynwinrt::WinRTValue) -> windows::core::HRESULT {
        let dynwinrt::WinRTValue::Object(object) = value else {
            panic!("expected delegate object")
        };
        let raw = object.as_raw();
        let vtable = unsafe { *(raw as *const *const TestDelegateVtbl) };
        unsafe { ((*vtable).invoke)(raw, std::ptr::null_mut(), std::ptr::null_mut()) }
    }

    #[test]
    fn python_delegate_reports_unraisable_callback_errors() {
        Python::initialize();
        Python::attach(|py| {
            let locals = PyDict::new(py);
            py.run(
                c"captured = []\ndef hook(args):\n    captured.append(args)\ndef success():\n    pass\ndef failure():\n    raise RuntimeError('callback failed')",
                Some(&locals),
                Some(&locals),
            )
            .unwrap();

            let sys = py.import("sys").unwrap();
            let original_hook = sys.getattr("unraisablehook").unwrap().unbind();
            sys.setattr("unraisablehook", locals.get_item("hook").unwrap().unwrap())
                .unwrap();

            let success = create_python_delegate(
                GUID::zeroed(),
                Vec::new(),
                locals.get_item("success").unwrap().unwrap().unbind(),
            );
            let failure = create_python_delegate(
                GUID::zeroed(),
                Vec::new(),
                locals
                    .get_item("failure")
                    .unwrap()
                    .unwrap()
                    .clone()
                    .unbind(),
            );
            let cross_thread_failure = create_python_delegate(
                GUID::zeroed(),
                Vec::new(),
                locals.get_item("failure").unwrap().unwrap().unbind(),
            );

            let success_result = unsafe { invoke_delegate(&success) };
            let failure_result = unsafe { invoke_delegate(&failure) };
            let cross_thread_result = py.detach(|| {
                std::thread::spawn(move || unsafe { invoke_delegate(&cross_thread_failure) })
                    .join()
                    .unwrap()
            });
            sys.setattr("unraisablehook", original_hook).unwrap();

            assert_eq!(success_result, windows::core::HRESULT(0));
            assert_eq!(failure_result, PYWINRT_E_UNRAISABLE_PYTHON_EXCEPTION);
            assert_eq!(cross_thread_result, PYWINRT_E_UNRAISABLE_PYTHON_EXCEPTION);
            let captured = locals
                .get_item("captured")
                .unwrap()
                .unwrap()
                .cast_into::<pyo3::types::PyList>()
                .unwrap();
            assert_eq!(captured.len(), 2);
            assert!(
                captured
                    .get_item(0)
                    .unwrap()
                    .getattr("exc_value")
                    .unwrap()
                    .is_instance_of::<pyo3::exceptions::PyRuntimeError>()
            );
        });
    }
}
