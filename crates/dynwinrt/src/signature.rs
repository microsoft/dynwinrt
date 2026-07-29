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
    metadata_table::{MetadataTable, TypeHandle},
    native_call::{AbiMethodSignature, Method as NativeMethod, ParameterType},
    value::WinRTValue,
};

#[derive(Debug, Clone)]
pub struct MethodSignature(AbiMethodSignature);

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
}

#[derive(Debug)]
pub struct Method(NativeMethod);

impl Method {
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
