// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! WinRT method planning.
//!
//! This public surface intentionally exposes only WinRT's HRESULT plus
//! input/output conventions. Classic COM lowers through its own planner in
//! `com`, while both planners share the private `native_call` executor.

use std::sync::Arc;

use windows::core::{GUID, HSTRING};

use crate::{
    metadata_table::{MetadataTable, TypeHandle, TypeKind},
    native_call::{
        AbiMethodSignature, Method as NativeMethod, MethodInfo, MethodReturn, OutputCleanup,
        ParamKind, Parameter, ParameterType, PreparedCall,
    },
    value::WinRTValue,
};

#[derive(Debug, Clone)]
pub struct MethodSignature(AbiMethodSignature);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WinRtParameterDirection {
    In,
    Out,
    FillArray,
}

impl MethodSignature {
    pub fn new(table: &Arc<MetadataTable>) -> Self {
        Self(AbiMethodSignature::new(table))
    }

    pub fn new_with_registry(table: &Arc<MetadataTable>) -> Self {
        Self::new(table)
    }

    pub fn add_in(self, typ: TypeHandle) -> Self {
        Self(self.0.add_in_type(ParameterType::winrt(typ)))
    }

    pub fn add_out(self, typ: TypeHandle) -> Self {
        Self(self.0.add_out_type(ParameterType::winrt(typ)))
    }

    pub fn add_out_fill(self, typ: TypeHandle) -> Self {
        Self(self.0.add_out_fill_type(ParameterType::winrt(typ)))
    }

    pub fn build(self, index: usize) -> Method {
        Method(self.0.build(index))
    }

    pub(crate) fn assert_parameter_owners(&self, table: &MetadataTable, name: &str) {
        for (index, parameter) in self.0.parameters().iter().enumerate() {
            let typ = parameter.typ.as_winrt().expect("WinRT signature parameter");
            assert!(
                std::ptr::eq(table, typ.table().as_ref()),
                "registered method '{name}' parameter {index} must use the same MetadataTable"
            );
        }
    }

    pub(crate) fn build_registered(self, index: usize) -> RegisteredMethod {
        let (info, prepared) = self.0.build(index).into_parts();
        let parameters = info
            .parameters
            .into_iter()
            .map(|parameter| {
                let direction = match parameter.kind {
                    ParamKind::In => WinRtParameterDirection::In,
                    ParamKind::Out => WinRtParameterDirection::Out,
                    ParamKind::OutFillArray => WinRtParameterDirection::FillArray,
                    ParamKind::OptionalOut | ParamKind::InOut => {
                        unreachable!("WinRT signature direction")
                    }
                };
                debug_assert!(parameter.canonical_format_input.is_none());
                RegisteredParameter {
                    typ: parameter
                        .typ
                        .as_winrt()
                        .expect("WinRT signature parameter")
                        .kind(),
                    direction,
                    output_cleanup: parameter.output_cleanup,
                    value_index: parameter.value_index,
                    input_index: parameter.input_index,
                }
            })
            .collect();
        debug_assert!(matches!(info.return_kind, MethodReturn::HResult));
        RegisteredMethod {
            index: info.index,
            parameters,
            input_count: info.input_count,
            out_count: info.out_count,
            prepared,
        }
    }

    pub(crate) fn implementation_parameters(
        &self,
    ) -> windows_core::Result<Vec<(TypeHandle, WinRtParameterDirection)>> {
        use crate::native_call::ParamKind;

        self.0
            .parameters()
            .iter()
            .map(|parameter| {
                let direction = match parameter.kind {
                    ParamKind::In => WinRtParameterDirection::In,
                    ParamKind::Out => WinRtParameterDirection::Out,
                    ParamKind::OutFillArray => WinRtParameterDirection::FillArray,
                    ParamKind::OptionalOut | ParamKind::InOut => {
                        return Err(windows_core::Error::new(
                            windows_core::HRESULT(0x80070057u32 as i32),
                            "WinRT implementations require WinRT input/output contracts",
                        ));
                    }
                };
                let typ = parameter.typ.as_winrt().ok_or_else(|| {
                    windows_core::Error::new(
                        windows_core::HRESULT(0x80070057u32 as i32),
                        "WinRT implementations cannot contain Classic COM parameter types",
                    )
                })?;
                Ok((typ.clone(), direction))
            })
            .collect()
    }
}

#[derive(Debug)]
struct RegisteredParameter {
    typ: TypeKind,
    direction: WinRtParameterDirection,
    output_cleanup: OutputCleanup,
    value_index: usize,
    input_index: Option<usize>,
}

/// Table-local method metadata must not retain owning TypeHandles.
#[derive(Debug)]
pub(crate) struct RegisteredMethod {
    index: usize,
    parameters: Vec<RegisteredParameter>,
    input_count: usize,
    out_count: usize,
    prepared: Arc<PreparedCall>,
}

impl RegisteredMethod {
    pub(crate) fn bind(&self, table: &Arc<MetadataTable>) -> Method {
        let parameters = self
            .parameters
            .iter()
            .map(|parameter| Parameter {
                typ: ParameterType::winrt(table.make(parameter.typ)),
                kind: match parameter.direction {
                    WinRtParameterDirection::In => ParamKind::In,
                    WinRtParameterDirection::Out => ParamKind::Out,
                    WinRtParameterDirection::FillArray => ParamKind::OutFillArray,
                },
                output_cleanup: parameter.output_cleanup,
                canonical_format_input: None,
                value_index: parameter.value_index,
                input_index: parameter.input_index,
            })
            .collect();
        Method(NativeMethod::from_parts(
            MethodInfo {
                index: self.index,
                parameters,
                input_count: self.input_count,
                out_count: self.out_count,
                return_kind: MethodReturn::HResult,
            },
            Arc::clone(&self.prepared),
        ))
    }
}

#[derive(Debug)]
pub struct Method(NativeMethod);

impl Method {
    #[cfg(test)]
    pub(crate) fn prepared_call(&self) -> &Arc<PreparedCall> {
        self.0.prepared_call()
    }

    pub fn call_getter_i32(&self, obj: *mut std::ffi::c_void) -> windows_core::Result<i32> {
        self.0.call_getter_i32(obj)
    }

    pub fn call_getter_bool(&self, obj: *mut std::ffi::c_void) -> windows_core::Result<bool> {
        self.0.call_getter_bool(obj)
    }

    pub fn call_getter_hstring(
        &self,
        obj: *mut std::ffi::c_void,
    ) -> windows_core::Result<windows_core::HSTRING> {
        self.0.call_getter_hstring(obj)
    }

    pub fn call_getter_object(
        &self,
        obj: *mut std::ffi::c_void,
    ) -> windows_core::Result<WinRTValue> {
        self.0.call_getter_object(obj)
    }

    pub fn call_setter_hstring(
        &self,
        obj: *mut std::ffi::c_void,
        value: &windows_core::HSTRING,
    ) -> windows_core::Result<()> {
        self.0.call_setter_hstring(obj, value)
    }

    pub fn call_setter_bool(
        &self,
        obj: *mut std::ffi::c_void,
        value: bool,
    ) -> windows_core::Result<()> {
        self.0.call_setter_bool(obj, value)
    }

    pub fn call_setter_i32(
        &self,
        obj: *mut std::ffi::c_void,
        value: i32,
    ) -> windows_core::Result<()> {
        self.0.call_setter_i32(obj, value)
    }

    pub fn call_setter_u32(
        &self,
        obj: *mut std::ffi::c_void,
        value: u32,
    ) -> windows_core::Result<()> {
        self.0.call_setter_u32(obj, value)
    }

    pub fn call_setter_f32(
        &self,
        obj: *mut std::ffi::c_void,
        value: f32,
    ) -> windows_core::Result<()> {
        self.0.call_setter_f32(obj, value)
    }

    pub fn call_setter_f64(
        &self,
        obj: *mut std::ffi::c_void,
        value: f64,
    ) -> windows_core::Result<()> {
        self.0.call_setter_f64(obj, value)
    }

    pub fn call_dynamic(
        &self,
        obj: *mut std::ffi::c_void,
        args: &[WinRTValue],
    ) -> windows_core::Result<Vec<WinRTValue>> {
        self.0.call_dynamic(obj, args)
    }
}

pub struct InterfaceSignature {
    pub name: String,
    pub iid: GUID,
    pub methods: Vec<Method>,
    #[allow(dead_code)]
    table: Arc<MetadataTable>,
}

impl InterfaceSignature {
    pub fn define_interface(name: String, iid: GUID, table: &Arc<MetadataTable>) -> Self {
        Self {
            name,
            iid,
            methods: Vec::new(),
            table: Arc::clone(table),
        }
    }

    pub fn define_from_iunknown(name: &str, iid: GUID, table: &Arc<MetadataTable>) -> Self {
        let mut result = Self::define_interface(name.to_owned(), iid, table);
        result
            .add_method(MethodSignature::new(table))
            .add_method(MethodSignature::new(table))
            .add_method(MethodSignature::new(table));
        result
    }

    pub fn define_from_iinspectable(name: &str, iid: GUID, table: &Arc<MetadataTable>) -> Self {
        let mut result = Self::define_from_iunknown(name, iid, table);
        result
            .add_method(MethodSignature::new(table))
            .add_method(MethodSignature::new(table).add_out(table.hstring()))
            .add_method(MethodSignature::new(table));
        result
    }

    pub fn add_method(&mut self, signature: MethodSignature) -> &mut Self {
        let method = signature.build(self.methods.len());
        self.methods.push(method);
        self
    }
}

#[allow(dead_code)]
pub struct RuntimeClassSignature {
    name: HSTRING,
    static_interfaces: Vec<InterfaceSignature>,
    instance_interfaces: Vec<InterfaceSignature>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winrt_signature_exposes_only_winrt_parameter_contracts() {
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table)
            .add_in(table.i32_type())
            .add_out(table.hstring())
            .add_out_fill(table.array(&table.object()));

        let _ = signature.build(6);
    }
}
