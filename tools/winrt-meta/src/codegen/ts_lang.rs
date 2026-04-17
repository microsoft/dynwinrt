// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! TypeScript implementation of the [`Lang`] trait.

use std::collections::HashSet;

use crate::meta::{InterfaceMeta, MethodMeta, ParamMeta};
use crate::types::TypeMeta;

use super::common;
use super::lang::Lang;

/// TypeScript codegen language driver.
pub struct TsLang;

impl Lang for TsLang {
    fn name(&self) -> &'static str { "ts" }

    fn member_name(&self, raw: &str) -> String {
        common::to_camel_case(raw)
    }

    fn filename_for_type(&self, name: &str) -> String {
        // TypeScript keeps PascalCase filenames (e.g. `Uri.ts`, `IStringable.ts`).
        name.to_string()
    }

    fn dynwinrt_type(&self, typ: &TypeMeta) -> String {
        common::ts_dynwinrt_type(typ)
    }

    fn build_method_sig(&self, method: &MethodMeta) -> String {
        common::build_method_sig(method)
    }

    fn wrap_arg(&self, name: &str, typ: &TypeMeta) -> String {
        common::wrap_arg(name, typ)
    }

    fn build_args_expr(&self, in_params: &[&ParamMeta]) -> String {
        common::build_args_expr(in_params)
    }

    fn convert_return(
        &self,
        expr: &str,
        return_type: Option<&TypeMeta>,
        is_async: bool,
        known_types: &HashSet<String>,
        deferred: &HashSet<String>,
    ) -> String {
        common::convert_return(expr, return_type, is_async, known_types, deferred)
    }

    fn struct_field_type(&self, typ: &TypeMeta) -> String {
        common::ts_struct_field_type(typ)
    }

    fn struct_field_getter(&self, typ: &TypeMeta, index: usize) -> String {
        common::struct_field_getter(typ, index)
    }

    fn struct_field_setter(&self, typ: &TypeMeta, index: usize, value_expr: &str) -> String {
        common::struct_field_setter(typ, index, value_expr)
    }

    fn generate_interface_registration(&self, iface: &InterfaceMeta, var_name: &str) -> String {
        common::generate_interface_registration(iface, var_name)
    }
}
