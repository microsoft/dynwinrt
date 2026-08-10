// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Rendering helpers for Python type stubs.

use std::collections::HashSet;

use crate::codegen::winrt::shared::imports::get_in_params;
use crate::meta::MethodMeta;
use crate::types::{TypeKind, TypeMeta};

use super::naming::{python_module_name, to_snake_case};
use super::native_types::{FoundationType, foundation_type};
use super::structs::{py_struct_field_read_type, py_struct_field_type};
use super::type_helpers::{
    py_delegate_callable_type, py_factory_return_type, py_method_return_type, py_output_type,
    py_param_list, py_param_type_safe,
};
use crate::codegen::winrt::shared::imports::ireference_inner_type;

pub(super) fn format_py_type_import(namespace: &str, name: &str, kind: TypeKind) -> String {
    let module = python_module_name(namespace, name);
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
            let mut names = if foundation_type(s).is_none() {
                vec![name.clone()]
            } else {
                Vec::new()
            };
            names.extend([
                format!("{}_TYPE", name),
                format!("pack_{}", snake),
                format!("unpack_{}", snake),
            ]);
            names
        }
        _ => vec![],
    }
}

pub(super) fn emit_struct_stub(s: &TypeMeta) -> String {
    if let Some(kind) = foundation_type(s) {
        let TypeMeta::Struct { name, .. } = s else {
            unreachable!()
        };
        let snake_name = to_snake_case(name);
        let native_type = match kind {
            FoundationType::DateTime => "datetime",
            FoundationType::TimeSpan => "timedelta",
        };
        return format!(
            "\ndef unpack_{snake_name}(v: DynWinRTValue) -> {native_type}: ...\n\
             {name}_TYPE: 'DynWinRTType'\n\
             def pack_{snake_name}(v: {native_type}) -> DynWinRTStruct: ...\n"
        );
    }

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
            let snake = to_snake_case(&f.name);
            if ireference_inner_type(&f.typ).is_some() {
                out.push_str(&format!(
                    "    @property\n\
                     \x20   def {snake}(self) -> {read_type}: ...\n\
                     \x20   @{snake}.setter\n\
                     \x20   def {snake}(self, value: {write_type}) -> None: ...\n",
                    read_type = py_struct_field_read_type(&f.typ),
                    write_type = py_struct_field_type(&f.typ),
                ));
            } else {
                out.push_str(&format!(
                    "    {}: {}\n",
                    snake,
                    py_struct_field_stub_type(&f.typ)
                ));
            }
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
    if ireference_inner_type(typ).is_some() {
        return py_struct_field_type(typ);
    }

    match typ {
        TypeMeta::Bool => "bool".to_string(),
        TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::I64
        | TypeMeta::U64
        | TypeMeta::Enum { .. } => "int".to_string(),
        TypeMeta::Char16 => "str".to_string(),
        TypeMeta::F32 | TypeMeta::F64 => "float".to_string(),
        TypeMeta::String => "str".to_string(),
        TypeMeta::Guid => "UUID".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "int".to_string(),
        typ if foundation_type(typ) == Some(FoundationType::DateTime) => "datetime".to_string(),
        typ if foundation_type(typ) == Some(FoundationType::TimeSpan) => "timedelta".to_string(),
        TypeMeta::Struct { name, .. } => format!("'{}'", name),
        _ => "object".to_string(),
    }
}

pub(super) fn emit_method_stub(
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
    indent_spaces: usize,
    event_has_remove: bool,
    property_has_getter: bool,
) -> String {
    emit_method_stub_named(
        method,
        known_types,
        delegate_type_names,
        indent_spaces,
        None,
        event_has_remove,
        property_has_getter,
    )
}

pub(super) fn emit_method_stub_named(
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
    indent_spaces: usize,
    name_override: Option<&str>,
    event_has_remove: bool,
    property_has_getter: bool,
) -> String {
    let indent = " ".repeat(indent_spaces);
    let in_params = get_in_params(method);
    let return_type = method.return_type.as_ref();

    let is_delegate_type = |typ: Option<&TypeMeta>| -> bool {
        match typ {
            Some(TypeMeta::Delegate { .. }) => true,
            Some(TypeMeta::Interface { name, .. }) => delegate_type_names.contains(name),
            Some(TypeMeta::Parameterized { name, args, .. }) => {
                delegate_type_names.contains(&crate::meta::make_parameterized_name(name, args))
            }
            _ => false,
        }
    };

    let mut out = String::new();

    // Events
    if method.is_event_add {
        let suffix = method.name.strip_prefix("add_").unwrap_or(&method.name);
        let event_name = to_snake_case(suffix);
        // Build a typed callback signature matching the runtime .py side.
        let delegate_typ = in_params.first().map(|p| &p.typ);
        let callback_sig = delegate_typ
            .map(|typ| py_delegate_callable_type(typ, known_types))
            .unwrap_or_else(|| "Callable[..., object]".to_string());
        out.push_str(&format!(
            "{indent}def on_{}(self, callback: {}) -> 'DynWinRTValue': ...\n",
            event_name, callback_sig
        ));
        if event_has_remove {
            out.push_str(&format!(
                "{indent}def subscribe_{}(self, callback: {}) -> Callable[[], None]: ...\n",
                event_name, callback_sig
            ));
            out.push_str(&format!(
                "{indent}def once_{}(self, callback: {}) -> Callable[[], None]: ...\n",
                event_name, callback_sig
            ));
        }
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
        let py_return = return_type
            .map(|typ| py_output_type(typ, known_types, delegate_type_names))
            .unwrap_or_else(|| "None".to_string());
        out.push_str(&format!("{indent}@builtins.property\n"));
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
            "Callable[..., object] | 'DynWinRTValue'".to_string()
        } else {
            in_params
                .first()
                .map(|p| py_param_type_safe(&p.typ, known_types))
                .unwrap_or_else(|| "object".to_string())
        };
        if property_has_getter {
            out.push_str(&format!("{indent}@{}.setter\n", prop_name));
            out.push_str(&format!(
                "{indent}def {}(self, value: {}) -> None: ...\n",
                prop_name, param_type
            ));
        } else {
            out.push_str(&format!(
                "{indent}def set_{}(self, value: {}) -> None: ...\n",
                prop_name, param_type
            ));
        }
    } else {
        let py_params = py_param_list(&in_params, known_types, delegate_type_names);
        let py_return = py_method_return_type(method, known_types, delegate_type_names);
        let method_name = name_override
            .map(str::to_string)
            .unwrap_or_else(|| to_snake_case(&method.name));
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
    delegate_type_names: &HashSet<String>,
) -> String {
    emit_static_method_stub_named(
        class_name,
        method,
        known_types,
        is_factory,
        delegate_type_names,
        None,
    )
}

pub(super) fn emit_static_method_stub_named(
    class_name: &str,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    is_factory: bool,
    delegate_type_names: &HashSet<String>,
    name_override: Option<&str>,
) -> String {
    let in_params = get_in_params(method);
    let py_params = py_param_list(&in_params, known_types, delegate_type_names);

    let py_return = if is_factory {
        py_factory_return_type(class_name, method, known_types)
    } else {
        py_method_return_type(method, known_types, delegate_type_names)
    };

    let mut out = String::new();

    if is_factory || !method.is_property_getter || !in_params.is_empty() {
        let method_name = name_override
            .map(str::to_string)
            .unwrap_or_else(|| to_snake_case(&method.name));
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
    } else {
        let prop_name = to_snake_case(method.name.strip_prefix("get_").unwrap_or(&method.name));
        out.push_str("    @classmethod\n");
        out.push_str(&format!(
            "    def get_{}(cls) -> {}: ...\n",
            prop_name, py_return
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{ParamDirection, ParamMeta};

    fn event_add() -> MethodMeta {
        MethodMeta {
            name: "add_Changed".into(),
            raw_name: "add_Changed".into(),
            params: vec![ParamMeta {
                name: "handler".into(),
                typ: TypeMeta::Parameterized {
                    namespace: "Windows.Foundation".into(),
                    name: "EventHandler`1".into(),
                    piid: "11111111-1111-1111-1111-111111111111".into(),
                    args: vec![TypeMeta::Object],
                },
                direction: ParamDirection::In,
            }],
            is_event_add: true,
            ..Default::default()
        }
    }

    #[test]
    fn paired_event_stubs_preserve_token_api_and_add_helpers() {
        let code = emit_method_stub(
            &event_add(),
            &HashSet::new(),
            &HashSet::new(),
            4,
            true,
            true,
        );
        assert!(code.contains("def on_changed("));
        assert!(code.contains("-> 'DynWinRTValue': ..."));
        assert!(code.contains("def subscribe_changed("));
        assert!(code.contains("def once_changed("));
    }

    #[test]
    fn add_only_event_stub_does_not_advertise_unavailable_helpers() {
        let code = emit_method_stub(
            &event_add(),
            &HashSet::new(),
            &HashSet::new(),
            4,
            false,
            true,
        );
        assert!(code.contains("def on_changed("));
        assert!(!code.contains("subscribe_changed"));
        assert!(!code.contains("once_changed"));
    }
}
