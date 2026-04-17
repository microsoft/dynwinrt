// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Language abstraction for codegen. TypeScript and Python implementations
//! delegate to existing helpers. Call sites can progressively adopt this trait
//! to write language-agnostic code.

use std::collections::HashSet;

use crate::meta::{InterfaceMeta, MethodMeta};
use crate::types::TypeMeta;

/// Language-specific primitives used by orchestrators. Keep this trait
/// *semantic* (full expression snippets) rather than syntactic
/// (e.g., don't expose `to_number(expr)` — expose `convert_return(expr, typ, ...)`).
pub trait Lang {
    fn name(&self) -> &'static str;

    // Naming
    fn member_name(&self, raw: &str) -> String;
    fn filename_for_type(&self, name: &str) -> String;

    // Type expressions
    fn dynwinrt_type(&self, typ: &TypeMeta) -> String;

    // Signature / call wiring
    fn build_method_sig(&self, method: &MethodMeta) -> String;
    fn wrap_arg(&self, name: &str, typ: &TypeMeta) -> String;
    fn build_args_expr(&self, in_params: &[&crate::meta::ParamMeta]) -> String;
    fn convert_return(
        &self,
        expr: &str,
        return_type: Option<&TypeMeta>,
        is_async: bool,
        known_types: &HashSet<String>,
        deferred: &HashSet<String>,
    ) -> String;

    // Struct helpers
    fn struct_field_type(&self, typ: &TypeMeta) -> String;
    fn struct_field_getter(&self, typ: &TypeMeta, index: usize) -> String;
    fn struct_field_setter(&self, typ: &TypeMeta, index: usize, value_expr: &str) -> String;

    // Interface registration
    fn generate_interface_registration(&self, iface: &InterfaceMeta, var_name: &str) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::py_lang::PyLang;
    use crate::codegen::ts_lang::TsLang;

    #[test]
    fn ts_lang_basic() {
        let l = TsLang;
        assert_eq!(l.name(), "ts");
        assert_eq!(l.member_name("GetValue"), "getValue");
        assert_eq!(l.dynwinrt_type(&TypeMeta::I32), "DynWinRtType.i32()");
    }

    #[test]
    fn py_lang_basic() {
        let l = PyLang;
        assert_eq!(l.name(), "py");
        assert_eq!(l.member_name("GetValue"), "get_value");
        assert_eq!(l.dynwinrt_type(&TypeMeta::I32), "DynWinRTType.i32_type()");
        assert_eq!(l.filename_for_type("MyClass"), "my_class");
    }

    #[test]
    fn trait_is_object_safe() {
        // If this compiles, the trait is object-safe (dyn-compatible)
        let _langs: Vec<Box<dyn Lang>> = vec![Box::new(TsLang), Box::new(PyLang)];
    }
}
