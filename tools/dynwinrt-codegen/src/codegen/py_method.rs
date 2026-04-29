// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;

use crate::meta::{InterfaceMeta, MethodMeta, ParamDirection};
use crate::types::TypeMeta;

use super::common::{
    get_in_params, to_snake_case,
    py_build_args_expr, py_convert_return, py_convert_array_return, py_wrap_arg,
};
use super::xml_text::{DocText, find_param_doc, format_pydoc};

/// Build the Python docstring for a method body. Uses snake_case param display
/// names (matching the generated signature). Returns an empty string when no
/// doc fields are populated, preserving byte-identity for metadata without
/// sibling .xml files.
fn method_pydoc(method: &MethodMeta, in_params: &[&crate::meta::ParamMeta]) -> String {
    if method.doc.is_none()
        && method.deprecated.is_none()
        && method.returns_doc.is_none()
        && method.param_docs.is_empty()
    {
        return String::new();
    }
    let params_snake: Vec<(String, &str)> = in_params.iter()
        .filter_map(|p| find_param_doc(&method.param_docs, &p.name).map(|d| (to_snake_case(&p.name), d)))
        .collect();
    let params_refs: Vec<(&str, &str)> = params_snake.iter().map(|(n, d)| (n.as_str(), *d)).collect();
    let doc = DocText {
        summary: method.doc.as_deref(),
        deprecated: method.deprecated.as_deref(),
        returns: method.returns_doc.as_deref(),
        params: params_refs,
    };
    format_pydoc(&doc, "        ")
}

// ======================================================================
// Python type annotation helpers
// ======================================================================

fn py_param_type(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => "bool".to_string(),
        TypeMeta::I8 | TypeMeta::U8 | TypeMeta::I16 | TypeMeta::U16 | TypeMeta::Char16
        | TypeMeta::I32 | TypeMeta::U32 | TypeMeta::I64 | TypeMeta::U64 => "int".to_string(),
        TypeMeta::F32 | TypeMeta::F64 => "float".to_string(),
        TypeMeta::String | TypeMeta::Guid => "str".to_string(),
        TypeMeta::RuntimeClass { name, .. }
        | TypeMeta::Enum { name, .. }
        | TypeMeta::Interface { name, .. } => format!("'{}'", name),
        TypeMeta::Parameterized { name, args, .. } => {
            format!("'{}'", crate::meta::make_parameterized_name(name, args))
        }
        TypeMeta::Array(_) => "'DynWinRTArray'".to_string(),
        TypeMeta::Object | TypeMeta::Delegate { .. } => "'DynWinRTValue'".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "int".to_string(),
        TypeMeta::Struct { name, .. } => format!("'{}'", name),
        _ => "object".to_string(),
    }
}

pub(crate) fn py_param_type_safe(typ: &TypeMeta, known: &HashSet<String>) -> String {
    match typ {
        TypeMeta::RuntimeClass { name, .. }
        | TypeMeta::Enum { name, .. }
        | TypeMeta::Interface { name, .. } if !known.contains(name) => "'DynWinRTValue'".to_string(),
        _ => py_param_type(typ),
    }
}

pub(crate) fn py_return_type_safe(typ: Option<&TypeMeta>, known: &HashSet<String>) -> String {
    match typ {
        Some(TypeMeta::RuntimeClass { name, .. })
        | Some(TypeMeta::Enum { name, .. })
        | Some(TypeMeta::Interface { name, .. }) if !known.contains(name) => "'DynWinRTValue'".to_string(),
        Some(TypeMeta::AsyncOperation(inner)) => {
            py_return_type_safe(Some(inner), known)
        }
        Some(TypeMeta::AsyncOperationWithProgress(result, _)) => {
            py_return_type_safe(Some(result), known)
        }
        Some(TypeMeta::AsyncActionWithProgress(_)) | Some(TypeMeta::AsyncAction) => "None".to_string(),
        Some(TypeMeta::Array(inner)) => py_array_element_type(inner, known),
        _ => py_return_type(typ),
    }
}

fn py_return_type(typ: Option<&TypeMeta>) -> String {
    match typ {
        Some(TypeMeta::String) | Some(TypeMeta::Guid) => "str".to_string(),
        Some(TypeMeta::Bool) => "bool".to_string(),
        Some(TypeMeta::I8 | TypeMeta::U8 | TypeMeta::I16 | TypeMeta::U16 | TypeMeta::Char16
            | TypeMeta::I32 | TypeMeta::U32 | TypeMeta::I64 | TypeMeta::U64) => "int".to_string(),
        Some(TypeMeta::F32 | TypeMeta::F64) => "float".to_string(),
        Some(TypeMeta::RuntimeClass { name, .. }) => format!("'{}'", name),
        Some(TypeMeta::Enum { name, .. }) => format!("'{}'", name),
        Some(TypeMeta::Interface { name, .. }) => format!("'{}'", name),
        Some(TypeMeta::Parameterized { name, args, .. }) => {
            format!("'{}'", crate::meta::make_parameterized_name(name, args))
        }
        Some(TypeMeta::AsyncOperation(inner)) => py_return_type(Some(inner)),
        Some(TypeMeta::AsyncOperationWithProgress(result, _)) => py_return_type(Some(result)),
        Some(TypeMeta::AsyncAction) | Some(TypeMeta::AsyncActionWithProgress(_)) => "None".to_string(),
        Some(TypeMeta::Array(inner)) => py_array_element_type(inner, &HashSet::new()),
        Some(TypeMeta::Object) | Some(TypeMeta::Delegate { .. }) => "'DynWinRTValue'".to_string(),
        Some(TypeMeta::Struct { name, .. }) if name == "HResult" => "int".to_string(),
        Some(TypeMeta::Struct { name, .. }) => format!("'{}'", name),
        None => "None".to_string(),
    }
}

pub(crate) fn py_array_element_type(inner: &TypeMeta, known_types: &HashSet<String>) -> String {
    match inner {
        TypeMeta::Bool => "list[bool]".to_string(),
        TypeMeta::String | TypeMeta::Guid => "list[str]".to_string(),
        TypeMeta::I8 | TypeMeta::U8 | TypeMeta::I16 | TypeMeta::U16 | TypeMeta::Char16
        | TypeMeta::I32 | TypeMeta::U32 | TypeMeta::I64 | TypeMeta::U64
        | TypeMeta::Enum { .. } => "list[int]".to_string(),
        TypeMeta::F32 | TypeMeta::F64 => "list[float]".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "list[int]".to_string(),
        TypeMeta::Struct { name, .. } => format!("list['{}']", name),
        TypeMeta::RuntimeClass { name, .. } if known_types.contains(name) => format!("list['{}']", name),
        TypeMeta::Interface { name, .. } if known_types.contains(name) => format!("list['{}']", name),
        _ => "list".to_string(),
    }
}

pub(crate) fn py_param_list(in_params: &[&crate::meta::ParamMeta], known_types: &HashSet<String>) -> String {
    in_params.iter()
        .map(|p| format!("{}: {}", to_snake_case(&p.name), py_param_type_safe(&p.typ, known_types)))
        .collect::<Vec<_>>()
        .join(", ")
}

// ======================================================================
// Method generation — Python call pattern
// ======================================================================

use crate::meta::ClassMeta;

pub(crate) fn generate_factory_method_invoke(
    class: &ClassMeta,
    iface: &InterfaceMeta,
    method: &MethodMeta,
    known_types: &HashSet<String>,
) -> String {
    let in_params = get_in_params(method);
    let py_params = py_param_list(&in_params, known_types);

    let is_async = method.return_type.as_ref().is_some_and(|rt| rt.is_async());
    let return_py_type = format!("'{}'", class.name);

    let mut out = String::new();
    let method_name = to_snake_case(&method.name);

    out.push_str("    @staticmethod\n");
    if py_params.is_empty() {
        out.push_str(&format!(
            "    def {}() -> {}:\n",
            method_name, return_py_type
        ));
    } else {
        out.push_str(&format!(
            "    def {}({}) -> {}:\n",
            method_name, py_params, return_py_type
        ));
    }
    out.push_str(&method_pydoc(method, &in_params));

    let args_expr = py_build_args_expr(&in_params);
    let call_expr = format!(
        "_{iface}.method({idx}).invoke({cls}._get_f_{iface}(), [{args}])",
        iface = iface.name, idx = method.vtable_index, cls = class.name, args = args_expr
    );

    if is_async {
        out.push_str(&format!(
            "        return {}({}.wait())\n",
            class.name, call_expr
        ));
    } else {
        out.push_str(&format!(
            "        return {}({})\n",
            class.name, call_expr
        ));
    }
    out
}

pub(crate) fn generate_static_method_invoke(
    class: &ClassMeta,
    iface: &InterfaceMeta,
    method: &MethodMeta,
    known_types: &HashSet<String>,
) -> String {
    let in_params = get_in_params(method);
    let py_params = py_param_list(&in_params, known_types);

    let return_type = method.return_type.as_ref();
    let is_with_progress = return_type.is_some_and(|rt| matches!(rt,
        TypeMeta::AsyncOperationWithProgress(_, _) | TypeMeta::AsyncActionWithProgress(_)));
    let is_async = return_type.is_some_and(|rt| rt.is_async()) && !is_with_progress;
    let py_return = py_return_type_safe(return_type, known_types);

    let mut out = String::new();

    let statics_call = format!("{cls}._get_s_{iface}()", cls = class.name, iface = iface.name);

    // Static property getter
    if method.is_property_getter && in_params.is_empty() {
        let prop_name = to_snake_case(method.name.strip_prefix("get_").unwrap_or(&method.name));
        // Python doesn't have static properties directly; use classmethod
        out.push_str("    @classmethod\n");
        out.push_str(&format!("    def get_{}(cls) -> {}:\n", prop_name, py_return));
        out.push_str(&method_pydoc(method, &in_params));
        let call_expr = format!(
            "_{}.method({}).invoke({}, [])",
            iface.name, method.vtable_index, statics_call
        );
        let converted = py_convert_return(&call_expr, return_type, false, known_types);
        out.push_str(&format!("        return {}\n", converted));
    } else {
        out.push_str("    @staticmethod\n");
        let method_name = to_snake_case(&method.name);
        if py_params.is_empty() {
            out.push_str(&format!(
                "    def {}() -> {}:\n",
                method_name, py_return
            ));
        } else {
            out.push_str(&format!(
                "    def {}({}) -> {}:\n",
                method_name, py_params, py_return
            ));
        }
        out.push_str(&method_pydoc(method, &in_params));
        let args_expr = py_build_args_expr(&in_params);
        let call_expr = format!(
            "_{}.method({}).invoke({}, [{}])",
            iface.name, method.vtable_index, statics_call, args_expr
        );
        if is_with_progress {
            let inner_type = match return_type {
                Some(TypeMeta::AsyncOperationWithProgress(inner, _)) => Some(inner.as_ref()),
                _ => None,
            };
            // For progress pattern in Python: call, optionally set progress callback, then .wait()
            let call_expr_no_idx = format!(
                "_{}.method({}).invoke({}, [{}])",
                iface.name, method.vtable_index, statics_call, args_expr
            );
            let inner_convert = py_convert_return("_op.wait()", inner_type, false, known_types);
            out.push_str(&format!("        _op = {}\n", call_expr_no_idx));
            out.push_str(&format!("        return {}\n", inner_convert));
        } else if is_async && matches!(return_type, Some(TypeMeta::AsyncAction) | Some(TypeMeta::AsyncActionWithProgress(_))) {
            let call_expr_void = format!(
                "_{}.method({}).invoke({}, [{}])",
                iface.name, method.vtable_index, statics_call, args_expr
            );
            out.push_str(&format!("        {}.wait()\n", call_expr_void));
        } else {
            let converted = py_convert_return(&call_expr, return_type, is_async, known_types);
            out.push_str(&format!("        return {}\n", converted));
        }
    }
    out
}

/// Generate an instance method for an interface wrapper class (Python).
pub(crate) fn generate_iface_instance_method(
    _iface: &InterfaceMeta,
    iface_var: &str,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
) -> String {
    // Skip composable `.ctor` on instance interfaces (see project.rs for full rationale).
    // Emitting it would produce `def .ctor(self) -> None:` which is a syntax error.
    if method.name == ".ctor" {
        return String::new();
    }
    generate_method_body(iface_var, "self._obj", method, known_types, delegate_type_names, None)
}

pub(crate) fn generate_method_body(
    iface_var: &str,
    obj_expr: &str,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
    name_override: Option<&str>,
) -> String {
    let in_params = get_in_params(method);
    let return_type = method.return_type.as_ref();
    let is_with_progress = return_type.is_some_and(|rt| matches!(rt,
        TypeMeta::AsyncOperationWithProgress(_, _) | TypeMeta::AsyncActionWithProgress(_)));
    let is_async = return_type.is_some_and(|rt| rt.is_async()) && !is_with_progress;
    let has_array_out = method.params.iter().any(|p| {
        (p.direction == ParamDirection::Out || p.direction == ParamDirection::OutFill)
            && matches!(p.typ, TypeMeta::Array(_))
    });
    let has_return = return_type.is_some() || has_array_out;

    let mut out = String::new();

    // Event add: create delegate from Python callback
    if method.is_event_add {
        let event_name = to_snake_case(method.name.strip_prefix("add_").unwrap_or(&method.name));
        let delegate_name = in_params.first().and_then(|p| match &p.typ {
            TypeMeta::Parameterized { name, args, .. } =>
                Some(crate::meta::make_parameterized_name(name, args)),
            TypeMeta::Delegate { name, .. } => Some(name.clone()),
            _ => None,
        });
        out.push_str(&format!(
            "    def on_{}(self, callback) -> 'DynWinRTValue':\n",
            event_name
        ));
        out.push_str(&method_pydoc(method, &in_params));
        if let Some(ref dname) = delegate_name {
            out.push_str(&format!(
                "        handler = DynWinRtDelegate.create(IID_{}, {}_PARAM_TYPES, callback)\n",
                dname, dname
            ));
        } else {
            out.push_str(
                "        handler = DynWinRtDelegate.create(DynWinRTType.object().iid(), [DynWinRTType.object(), DynWinRTType.object()], callback)\n"
            );
        }
        out.push_str(&format!(
            "        return {}.method({}).invoke({}, [handler.to_value()])\n",
            iface_var, method.vtable_index, obj_expr
        ));
        return out;
    }
    // Event remove
    if method.is_event_remove {
        let event_name = to_snake_case(method.name.strip_prefix("remove_").unwrap_or(&method.name));
        out.push_str(&format!(
            "    def off_{}(self, token: 'DynWinRTValue'):\n",
            event_name
        ));
        out.push_str(&method_pydoc(method, &in_params));
        out.push_str(&format!(
            "        {}.method({}).invoke({}, [token])\n",
            iface_var, method.vtable_index, obj_expr
        ));
        return out;
    }

    let is_delegate_type = |typ: Option<&TypeMeta>| -> bool {
        match typ {
            Some(TypeMeta::Delegate { .. }) => true,
            Some(TypeMeta::Interface { name, .. }) => delegate_type_names.contains(name),
            _ => false,
        }
    };

    if method.is_property_getter && in_params.is_empty() {
        let prop_name = to_snake_case(method.name.strip_prefix("get_").unwrap_or(&method.name));
        let py_return = if is_delegate_type(return_type) {
            "'DynWinRTValue'".to_string()
        } else {
            py_return_type_safe(return_type, known_types)
        };
        out.push_str("    @property\n");
        out.push_str(&format!("    def {}(self) -> {}:\n", prop_name, py_return));
        out.push_str(&method_pydoc(method, &in_params));
        let call_expr = format!(
            "{}.method({}).invoke({}, [])",
            iface_var, method.vtable_index, obj_expr
        );
        let converted = if is_delegate_type(return_type) {
            call_expr.clone()
        } else {
            py_convert_return(&call_expr, return_type, false, known_types)
        };
        out.push_str(&format!("        return {}\n", converted));
    } else if method.is_property_setter {
        let prop_name = to_snake_case(method.name.strip_prefix("put_").unwrap_or(&method.name));
        let param_type = if in_params.first().is_some_and(|p| is_delegate_type(Some(&p.typ))) {
            "'DynWinRTValue'".to_string()
        } else {
            in_params.first()
                .map(|p| py_param_type_safe(&p.typ, known_types))
                .unwrap_or_else(|| "object".to_string())
        };
        out.push_str(&format!("    @{}.setter\n", prop_name));
        out.push_str(&format!("    def {}(self, value: {}):\n", prop_name, param_type));
        out.push_str(&method_pydoc(method, &in_params));
        let arg = in_params.first()
            .map(|p| py_wrap_arg("value", &p.typ))
            .unwrap_or_else(|| "value".to_string());
        out.push_str(&format!(
            "        {}.method({}).invoke({}, [{}])\n",
            iface_var, method.vtable_index, obj_expr, arg
        ));
    } else {
        let py_params = py_param_list(&in_params, known_types);
        let array_out_elem = if has_array_out && return_type.is_none() {
            method.params.iter().find_map(|p| {
                if p.direction == ParamDirection::Out || p.direction == ParamDirection::OutFill {
                    if let TypeMeta::Array(inner) = &p.typ { Some(inner.as_ref()) } else { None }
                } else { None }
            })
        } else { None };
        let py_return = if let Some(elem) = array_out_elem {
            py_array_element_type(elem, known_types)
        } else {
            py_return_type_safe(return_type, known_types)
        };
        let method_name = name_override.map(|s| s.to_string()).unwrap_or_else(|| to_snake_case(&method.name));

        let self_and_params = if py_params.is_empty() {
            "self".to_string()
        } else {
            format!("self, {}", py_params)
        };
        out.push_str(&format!(
            "    def {}({}) -> {}:\n",
            method_name, self_and_params, py_return
        ));
        out.push_str(&method_pydoc(method, &in_params));

        let args_expr = py_build_args_expr(&in_params);
        let call_expr = format!(
            "{}.method({}).invoke({}, [{}])",
            iface_var, method.vtable_index, obj_expr, args_expr
        );

        if is_with_progress {
            let inner_type = match return_type {
                Some(TypeMeta::AsyncOperationWithProgress(inner, _)) => Some(inner.as_ref()),
                _ => None,
            };
            let inner_convert = py_convert_return("_op.wait()", inner_type, false, known_types);
            out.push_str(&format!("        _op = {}\n", call_expr));
            out.push_str(&format!("        return {}\n", inner_convert));
        } else if !has_return && !is_async {
            out.push_str(&format!("        {}\n", call_expr));
        } else if is_async && matches!(return_type, Some(TypeMeta::AsyncAction) | Some(TypeMeta::AsyncActionWithProgress(_))) {
            out.push_str(&format!("        {}.wait()\n", call_expr));
        } else if let Some(elem) = array_out_elem {
            let arr_expr = format!("{}.as_array()", call_expr);
            let converted = py_convert_array_return(&arr_expr, elem, known_types);
            out.push_str(&format!("        return {}\n", converted));
        } else {
            let converted = py_convert_return(&call_expr, return_type, is_async, known_types);
            out.push_str(&format!("        return {}\n", converted));
        }
    }

    out
}
