// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Rendering helpers for Python type stubs.

use std::collections::HashSet;

use crate::codegen::shared::imports::get_in_params;
use crate::meta::{MethodMeta, ParamDirection};
use crate::types::{TypeKind, TypeMeta};

use super::naming::{to_snake_case, to_snake_case_filename};
use super::type_helpers::{
    py_array_element_type, py_param_list, py_param_type_safe, py_return_type_safe,
};

pub(super) fn format_py_type_import(name: &str, kind: TypeKind) -> String {
    let module = to_snake_case_filename(name);
    if kind == TypeKind::Interface {
        format!("from .{module} import IID_{name}, {name}  # noqa: F401\n")
    } else {
        format!("from .{module} import {name}  # noqa: F401\n")
    }
}

pub(super) fn py_struct_export_names(s: &TypeMeta) -> Vec<String> {
    match s {
        TypeMeta::Struct { name, .. } => {
            let snake = to_snake_case(name);
            vec![
                format!("{}_TYPE", name),
                format!("pack_{}", snake),
                format!("unpack_{}", snake),
            ]
        }
        _ => vec![],
    }
}

pub(super) fn emit_struct_stub(s: &TypeMeta) -> String {
    let (_namespace, name, fields) = match s {
        TypeMeta::Struct {
            namespace,
            name,
            fields,
        } => (namespace, name, fields),
        _ => return String::new(),
    };
    let mut out = String::new();
    let snake_name = to_snake_case(name);

    out.push_str(&format!("\nclass {}:\n", name));
    if fields.is_empty() {
        out.push_str("    pass\n");
    } else {
        let init_params: Vec<String> = fields
            .iter()
            .map(|f| {
                format!(
                    "{}: {} = ...",
                    to_snake_case(&f.name),
                    py_struct_field_stub_type(&f.typ)
                )
            })
            .collect();
        out.push_str(&format!(
            "    def __init__(self, {}) -> None: ...\n",
            init_params.join(", ")
        ));
        for f in fields {
            out.push_str(&format!(
                "    {}: {}\n",
                to_snake_case(&f.name),
                py_struct_field_stub_type(&f.typ)
            ));
        }
    }
    out.push('\n');

    out.push_str(&format!(
        "def unpack_{}(v: DynWinRTValue) -> {}: ...\n",
        snake_name, name
    ));
    out.push_str(&format!("{}_TYPE: 'DynWinRTType'\n", name));
    out.push_str(&format!(
        "def pack_{}(v: {}) -> DynWinRTStruct: ...\n",
        snake_name, name
    ));
    out
}

pub(super) fn py_struct_field_stub_type(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => "bool".to_string(),
        TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::Char16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::I64
        | TypeMeta::U64
        | TypeMeta::Enum { .. } => "int".to_string(),
        TypeMeta::F32 | TypeMeta::F64 => "float".to_string(),
        TypeMeta::String | TypeMeta::Guid => "str".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "int".to_string(),
        TypeMeta::Struct { name, .. } => format!("'{}'", name),
        _ => "object".to_string(),
    }
}

pub(super) fn emit_method_stub(
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
    indent_spaces: usize,
) -> String {
    let indent = " ".repeat(indent_spaces);
    let in_params = get_in_params(method);
    let return_type = method.return_type.as_ref();
    let has_array_out = method.params.iter().any(|p| {
        (p.direction == ParamDirection::Out || p.direction == ParamDirection::OutFill)
            && matches!(p.typ, TypeMeta::Array(_))
    });

    let is_delegate_type = |typ: Option<&TypeMeta>| -> bool {
        match typ {
            Some(TypeMeta::Delegate { .. }) => true,
            Some(TypeMeta::Interface { name, .. }) => delegate_type_names.contains(name),
            _ => false,
        }
    };

    let mut out = String::new();

    // Events
    if method.is_event_add {
        let event_name = to_snake_case(method.name.strip_prefix("add_").unwrap_or(&method.name));
        out.push_str(&format!(
            "{indent}def on_{}(self, callback) -> 'DynWinRTValue': ...\n",
            event_name
        ));
        return out;
    }
    if method.is_event_remove {
        let event_name = to_snake_case(method.name.strip_prefix("remove_").unwrap_or(&method.name));
        out.push_str(&format!(
            "{indent}def off_{}(self, token: 'DynWinRTValue') -> None: ...\n",
            event_name
        ));
        return out;
    }

    if method.is_property_getter && in_params.is_empty() {
        let prop_name = to_snake_case(method.name.strip_prefix("get_").unwrap_or(&method.name));
        let py_return = if is_delegate_type(return_type) {
            "'DynWinRTValue'".to_string()
        } else {
            py_return_type_safe(return_type, known_types)
        };
        out.push_str(&format!("{indent}@property\n"));
        out.push_str(&format!(
            "{indent}def {}(self) -> {}: ...\n",
            prop_name, py_return
        ));
    } else if method.is_property_setter {
        let prop_name = to_snake_case(method.name.strip_prefix("put_").unwrap_or(&method.name));
        let param_type = if in_params
            .first()
            .is_some_and(|p| is_delegate_type(Some(&p.typ)))
        {
            "'DynWinRTValue'".to_string()
        } else {
            in_params
                .first()
                .map(|p| py_param_type_safe(&p.typ, known_types))
                .unwrap_or_else(|| "object".to_string())
        };
        out.push_str(&format!("{indent}@{}.setter\n", prop_name));
        out.push_str(&format!(
            "{indent}def {}(self, value: {}) -> None: ...\n",
            prop_name, param_type
        ));
    } else {
        let py_params = py_param_list(&in_params, known_types);
        let array_out_elem = if has_array_out && return_type.is_none() {
            method.params.iter().find_map(|p| {
                if p.direction == ParamDirection::Out || p.direction == ParamDirection::OutFill {
                    if let TypeMeta::Array(inner) = &p.typ {
                        Some(inner.as_ref())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
        } else {
            None
        };
        let py_return = if let Some(elem) = array_out_elem {
            py_array_element_type(elem, known_types)
        } else {
            py_return_type_safe(return_type, known_types)
        };
        let method_name = to_snake_case(&method.name);
        let self_and_params = if py_params.is_empty() {
            "self".to_string()
        } else {
            format!("self, {}", py_params)
        };
        out.push_str(&format!(
            "{indent}def {}({}) -> {}: ...\n",
            method_name, self_and_params, py_return
        ));
    }

    out
}

pub(super) fn emit_static_method_stub(
    class_name: &str,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    is_factory: bool,
) -> String {
    let in_params = get_in_params(method);
    let py_params = py_param_list(&in_params, known_types);

    let return_type = method.return_type.as_ref();
    let py_return = if is_factory {
        format!("'{}'", class_name)
    } else {
        py_return_type_safe(return_type, known_types)
    };

    let mut out = String::new();

    if !is_factory && method.is_property_getter && in_params.is_empty() {
        let prop_name = to_snake_case(method.name.strip_prefix("get_").unwrap_or(&method.name));
        out.push_str("    @classmethod\n");
        out.push_str(&format!(
            "    def get_{}(cls) -> {}: ...\n",
            prop_name, py_return
        ));
    } else {
        let method_name = to_snake_case(&method.name);
        out.push_str("    @staticmethod\n");
        if py_params.is_empty() {
            out.push_str(&format!(
                "    def {}() -> {}: ...\n",
                method_name, py_return
            ));
        } else {
            out.push_str(&format!(
                "    def {}({}) -> {}: ...\n",
                method_name, py_params, py_return
            ));
        }
    }
    out
}
