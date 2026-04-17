// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python implementation of the [`Lang`] trait.

use std::collections::HashSet;

use crate::meta::{InterfaceMeta, MethodMeta, ParamMeta};
use crate::types::TypeMeta;

use super::common;
use super::lang::Lang;

/// Python codegen language driver.
pub struct PyLang;

impl Lang for PyLang {
    fn name(&self) -> &'static str { "py" }

    fn member_name(&self, raw: &str) -> String {
        common::to_snake_case(raw)
    }

    fn filename_for_type(&self, name: &str) -> String {
        common::to_snake_case_filename(name)
    }

    fn dynwinrt_type(&self, typ: &TypeMeta) -> String {
        common::py_dynwinrt_type(typ)
    }

    fn build_method_sig(&self, method: &MethodMeta) -> String {
        common::py_build_method_sig(method)
    }

    fn wrap_arg(&self, name: &str, typ: &TypeMeta) -> String {
        common::py_wrap_arg(name, typ)
    }

    fn build_args_expr(&self, in_params: &[&ParamMeta]) -> String {
        common::py_build_args_expr(in_params)
    }

    fn convert_return(
        &self,
        expr: &str,
        return_type: Option<&TypeMeta>,
        is_async: bool,
        known_types: &HashSet<String>,
        _deferred: &HashSet<String>,
    ) -> String {
        // Python codegen has no deferred (lazy module) mechanism.
        common::py_convert_return(expr, return_type, is_async, known_types)
    }

    fn struct_field_type(&self, typ: &TypeMeta) -> String {
        common::py_struct_field_type(typ)
    }

    fn struct_field_getter(&self, typ: &TypeMeta, index: usize) -> String {
        common::py_struct_field_getter(typ, index)
    }

    fn struct_field_setter(&self, typ: &TypeMeta, index: usize, value_expr: &str) -> String {
        common::py_struct_field_setter(typ, index, value_expr)
    }

    fn generate_interface_registration(&self, iface: &InterfaceMeta, var_name: &str) -> String {
        common::py_generate_interface_registration(iface, var_name)
    }
}
