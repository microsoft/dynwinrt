// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;

use crate::meta::{InterfaceMeta, MethodMeta, ParamDirection};
use crate::types::TypeMeta;

use super::common::{
    NO_DEFERRED, build_args_expr, convert_array_return, convert_return,
    get_in_params, to_camel_case, capitalize, wrap_arg,
};

fn method_jsdoc(method: &MethodMeta, in_params: &[&crate::meta::ParamMeta]) -> String {
    let params_display: Vec<(String, &str)> = in_params.iter()
        .filter_map(|p| {
            super::xml_text::find_param_doc(&method.param_docs, &p.name)
                .map(|d| (to_camel_case(&p.name), d))
        })
        .collect();
    let params_refs: Vec<(&str, &str)> = params_display.iter()
        .map(|(n, d)| (n.as_str(), *d))
        .collect();
    let doc = super::xml_text::DocText {
        summary: method.doc.as_deref(),
        deprecated: method.deprecated.as_deref(),
        returns: method.returns_doc.as_deref(),
        params: params_refs,
    };
    super::xml_text::format_jsdoc(&doc, "    ")
}

// ======================================================================
// TypeScript type annotation helpers
// ======================================================================

fn ts_param_type(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => "boolean".to_string(),
        TypeMeta::I8 | TypeMeta::U8 | TypeMeta::I16 | TypeMeta::U16 | TypeMeta::Char16
        | TypeMeta::I32 | TypeMeta::U32 | TypeMeta::I64 | TypeMeta::U64
        | TypeMeta::F32 | TypeMeta::F64 => {
            "number".to_string()
        }
        TypeMeta::String | TypeMeta::Guid => "string".to_string(),
        TypeMeta::RuntimeClass { name, .. }
        | TypeMeta::Enum { name, .. }
        | TypeMeta::Interface { name, .. } => name.clone(),
        TypeMeta::Parameterized { name, args, .. } => crate::meta::make_parameterized_name(name, args),
        TypeMeta::Array(_) => "DynWinRtArray".to_string(),
        TypeMeta::Object | TypeMeta::Delegate { .. } => "DynWinRtValue".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "number".to_string(),
        TypeMeta::Struct { name, .. } => name.clone(),
        _ => "any".to_string(),
    }
}

pub(crate) fn ts_param_type_safe(typ: &TypeMeta, known: &HashSet<String>) -> String {
    match typ {
        TypeMeta::RuntimeClass { name, .. }
        | TypeMeta::Enum { name, .. }
        | TypeMeta::Interface { name, .. } if !known.contains(name) => "DynWinRtValue".to_string(),
        _ => ts_param_type(typ),
    }
}

pub(crate) fn ts_return_type_safe(typ: Option<&TypeMeta>, is_async: bool, known: &HashSet<String>) -> String {
    match typ {
        Some(TypeMeta::RuntimeClass { name, .. })
        | Some(TypeMeta::Enum { name, .. })
        | Some(TypeMeta::Interface { name, .. }) if !known.contains(name) => {
            if is_async { "Promise<DynWinRtValue>".to_string() } else { "DynWinRtValue".to_string() }
        }
        Some(TypeMeta::AsyncOperation(inner)) => {
            format!("Promise<{}>", ts_return_type_safe(Some(inner), false, known))
        }
        Some(TypeMeta::AsyncOperationWithProgress(result, _)) => {
            let inner = ts_return_type_safe(Some(result), false, known);
            format!("Promise<{i}> & {{ progress(cb: (value: DynWinRtValue) => void): Promise<{i}> & {{ progress: any; toPromise(): Promise<{i}>; }}; toPromise(): Promise<{i}>; }}", i = inner)
        }
        Some(TypeMeta::AsyncActionWithProgress(_)) => {
            "Promise<void> & { progress(cb: (value: DynWinRtValue) => void): Promise<void> & { progress: any; toPromise(): Promise<void>; }; toPromise(): Promise<void>; }".to_string()
        }
        Some(TypeMeta::Array(inner)) => {
            let s = ts_array_element_type(inner, known);
            if is_async { format!("Promise<{}>", s) } else { s }
        }
        _ => ts_return_type(typ, is_async),
    }
}

fn ts_return_type(typ: Option<&TypeMeta>, is_async: bool) -> String {
    let inner = match typ {
        Some(TypeMeta::String) | Some(TypeMeta::Guid) => "string",
        Some(TypeMeta::Bool) => "boolean",
        Some(TypeMeta::I8 | TypeMeta::U8 | TypeMeta::I16 | TypeMeta::U16 | TypeMeta::Char16
            | TypeMeta::I32 | TypeMeta::U32 | TypeMeta::I64 | TypeMeta::U64
            | TypeMeta::F32 | TypeMeta::F64) => "number",
        Some(TypeMeta::RuntimeClass { name, .. }) => {
            return if is_async { format!("Promise<{}>", name) } else { name.clone() }
        }
        Some(TypeMeta::Enum { name, .. }) => {
            return if is_async { format!("Promise<{}>", name) } else { name.clone() }
        }
        Some(TypeMeta::Interface { name, .. }) => {
            return if is_async { format!("Promise<{}>", name) } else { name.clone() }
        }
        Some(TypeMeta::Parameterized { name, args, .. }) => {
            let s = crate::meta::make_parameterized_name(name, args);
            return if is_async { format!("Promise<{}>", s) } else { s };
        }
        Some(TypeMeta::AsyncOperation(inner)) => {
            return format!("Promise<{}>", ts_return_type(Some(inner), false));
        }
        Some(TypeMeta::AsyncOperationWithProgress(result, _)) => {
            let inner = ts_return_type(Some(result), false);
            return format!("Promise<{i}> & {{ progress(cb: (value: DynWinRtValue) => void): Promise<{i}> & {{ progress: any; toPromise(): Promise<{i}>; }}; toPromise(): Promise<{i}>; }}", i = inner);
        }
        Some(TypeMeta::AsyncAction) => return "Promise<void>".to_string(),
        Some(TypeMeta::AsyncActionWithProgress(_)) => {
            return "Promise<void> & { progress(cb: (value: DynWinRtValue) => void): Promise<void> & { progress: any; toPromise(): Promise<void>; }; toPromise(): Promise<void>; }".to_string();
        }
        Some(TypeMeta::Array(inner)) => {
            let s = ts_array_element_type(inner, &HashSet::new());
            return if is_async { format!("Promise<{}>", s) } else { s };
        }
        Some(TypeMeta::Object) | Some(TypeMeta::Delegate { .. }) => "DynWinRtValue",
        Some(TypeMeta::Struct { name, .. }) if name == "HResult" => "number",
        Some(TypeMeta::Struct { name, .. }) => {
            return if is_async { format!("Promise<{}>", name) } else { name.clone() }
        }
        None => "void",
    };
    if is_async { format!("Promise<{}>", inner) } else { inner.to_string() }
}

/// TypeScript return type annotation for an array element type.
pub(crate) fn ts_array_element_type(inner: &TypeMeta, known_types: &HashSet<String>) -> String {
    match inner {
        TypeMeta::Bool => "boolean[]".to_string(),
        TypeMeta::String | TypeMeta::Guid => "string[]".to_string(),
        TypeMeta::I8 | TypeMeta::U8 | TypeMeta::I16 | TypeMeta::U16 | TypeMeta::Char16
        | TypeMeta::I32 | TypeMeta::U32 | TypeMeta::I64 | TypeMeta::U64
        | TypeMeta::F32 | TypeMeta::F64 | TypeMeta::Enum { .. } => "number[]".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "number[]".to_string(),
        TypeMeta::Struct { name, .. } => format!("{}[]", name),
        TypeMeta::RuntimeClass { name, .. } if known_types.contains(name) => format!("{}[]", name),
        TypeMeta::Interface { name, .. } if known_types.contains(name) => format!("{}[]", name),
        _ => "DynWinRtValue[]".to_string(),
    }
}

pub(crate) fn ts_param_list(in_params: &[&crate::meta::ParamMeta], known_types: &HashSet<String>) -> String {
    in_params.iter()
        .map(|p| format!("{}: {}", to_camel_case(&p.name), ts_param_type_safe(&p.typ, known_types)))
        .collect::<Vec<_>>()
        .join(", ")
}

// ======================================================================
// Method generation — invoke pattern
// ======================================================================

use crate::meta::ClassMeta;

pub(crate) fn generate_factory_method_invoke(
    class: &ClassMeta,
    iface: &InterfaceMeta,
    method: &MethodMeta,
    known_types: &HashSet<String>,
) -> String {
    let in_params = get_in_params(method);
    let ts_params = ts_param_list(&in_params, known_types);

    let is_async = method.return_type.as_ref().is_some_and(|rt| rt.is_async());
    let return_ts_type = if is_async {
        format!("Promise<{}>", class.name)
    } else {
        class.name.clone()
    };

    let mut out = String::new();
    out.push_str(&method_jsdoc(method, &in_params));
    let async_kw = if is_async { "async " } else { "" };
    out.push_str(&format!(
        "    static {}{}({}): {} {{\n",
        async_kw, to_camel_case(&method.name), ts_params, return_ts_type
    ));

    let args_expr = build_args_expr(&in_params);
    let invoke_expr = format!(
        "_{iface}.method({idx}).invoke({cls}.f_{iface}(), [{args}])",
        iface = iface.name, idx = method.vtable_index, cls = class.name, args = args_expr
    );

    if is_async {
        out.push_str(&format!(
            "        return new {}(await {}.toPromise());\n",
            class.name, invoke_expr
        ));
    } else {
        out.push_str(&format!(
            "        return new {}({});\n",
            class.name, invoke_expr
        ));
    }
    out.push_str("    }\n");
    out
}

pub(crate) fn generate_static_method_invoke(
    class: &ClassMeta,
    iface: &InterfaceMeta,
    method: &MethodMeta,
    known_types: &HashSet<String>,
) -> String {
    let in_params = get_in_params(method);
    let ts_params = ts_param_list(&in_params, known_types);

    let return_type = method.return_type.as_ref();
    let is_with_progress = return_type.is_some_and(|rt| matches!(rt,
        TypeMeta::AsyncOperationWithProgress(_, _) | TypeMeta::AsyncActionWithProgress(_)));
    let is_async = return_type.is_some_and(|rt| rt.is_async()) && !is_with_progress;
    let ts_return = ts_return_type_safe(return_type, is_async, known_types);

    let mut out = String::new();

    let statics_call = format!("{cls}.s_{iface}()", cls = class.name, iface = iface.name);

    // Static property getter
    if method.is_property_getter && in_params.is_empty() {
        let prop_name = to_camel_case(method.name.strip_prefix("get_").unwrap_or(&method.name));
        out.push_str(&method_jsdoc(method, &in_params));
        out.push_str(&format!("    static get {}(): {} {{\n", prop_name, ts_return));
        let invoke_expr = format!(
            "_{}.method({}).invoke({}, [])",
            iface.name, method.vtable_index, statics_call
        );
        let converted = convert_return(&invoke_expr, return_type, false, known_types, &NO_DEFERRED);
        out.push_str(&format!("        return {};\n", converted));
        out.push_str("    }\n");
    } else {
        let async_kw = if is_async { "async " } else { "" };
        out.push_str(&method_jsdoc(method, &in_params));
        out.push_str(&format!(
            "    static {}{}({}): {} {{\n",
            async_kw, to_camel_case(&method.name), ts_params, ts_return
        ));
        let args_expr = build_args_expr(&in_params);
        let invoke_expr = format!(
            "_{}.method({}).invoke({}, [{}])",
            iface.name, method.vtable_index, statics_call, args_expr
        );
        if is_with_progress {
            let inner_type = match return_type {
                Some(TypeMeta::AsyncOperationWithProgress(inner, _)) => Some(inner.as_ref()),
                _ => None,
            };
            let inner_convert = convert_return("_v", inner_type, false, known_types, &NO_DEFERRED);
            out.push_str(&format!("        const _op = {};\n", invoke_expr));
            out.push_str(&format!("        const _promise = _op.toPromise().then((_v: DynWinRtValue) => {});\n", inner_convert));
            out.push_str(         "        return Object.assign(_promise, {\n");
            out.push_str(         "            progress(cb: (value: DynWinRtValue) => void) { _op.onProgress(cb); return this; },\n");
            out.push_str(&format!("            toPromise() {{ return _op.toPromise().then((_v: DynWinRtValue) => {}); }},\n", inner_convert));
            out.push_str(         "        });\n");
        } else {
            let converted = convert_return(&invoke_expr, return_type, is_async, known_types, &NO_DEFERRED);
            out.push_str(&format!("        return {};\n", converted));
        }
        out.push_str("    }\n");
    }
    out
}

/// Generate an instance method for an interface wrapper class.
pub(crate) fn generate_iface_instance_method(
    _iface: &InterfaceMeta,
    iface_var: &str,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
) -> String {
    generate_method_body(iface_var, "this._obj", method, known_types, delegate_type_names, None)
}

pub(crate) fn generate_method_body(
    iface_var: &str,
    obj_expr: &str,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
    ts_name_override: Option<&str>,
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

    out.push_str(&method_jsdoc(method, &in_params));

    // Event add: create delegate from JS callback, call add_, return token
    if method.is_event_add {
        let event_name = to_camel_case(method.name.strip_prefix("add_").unwrap_or(&method.name));
        let delegate_name = in_params.first().and_then(|p| match &p.typ {
            TypeMeta::Parameterized { name, args, .. } =>
                Some(crate::meta::make_parameterized_name(name, args)),
            TypeMeta::Delegate { name, .. } => Some(name.clone()),
            _ => None,
        });
        out.push_str(&format!(
            "    on{}(callback: (...args: DynWinRtValue[]) => void): DynWinRtValue {{\n",
            capitalize(&event_name)
        ));
        if let Some(ref dname) = delegate_name {
            out.push_str(&format!(
                "        const handler = DynWinRtDelegate.create(IID_{}, {}_PARAM_TYPES, callback);\n",
                dname, dname
            ));
        } else {
            out.push_str(
                "        const handler = DynWinRtDelegate.create(DynWinRtType.object().iid()!, [DynWinRtType.object(), DynWinRtType.object()], callback);\n"
            );
        }
        out.push_str(&format!(
            "        return {}.method({}).invoke({}, [handler.toValue()]);\n",
            iface_var, method.vtable_index, obj_expr
        ));
        out.push_str("    }\n");
        return out;
    }
    // Event remove
    if method.is_event_remove {
        let event_name = to_camel_case(method.name.strip_prefix("remove_").unwrap_or(&method.name));
        out.push_str(&format!(
            "    off{}(token: DynWinRtValue): void {{\n",
            capitalize(&event_name)
        ));
        out.push_str(&format!(
            "        {}.method({}).invoke({}, [token]);\n",
            iface_var, method.vtable_index, obj_expr
        ));
        out.push_str("    }\n");
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
        let prop_name = to_camel_case(method.name.strip_prefix("get_").unwrap_or(&method.name));
        let ts_return = if is_delegate_type(return_type) {
            "DynWinRtValue".to_string()
        } else {
            ts_return_type_safe(return_type, false, known_types)
        };
        out.push_str(&format!("    get {}(): {} {{\n", prop_name, ts_return));
        let invoke_expr = format!(
            "{}.method({}).invoke({}, [])",
            iface_var, method.vtable_index, obj_expr
        );
        let converted = if is_delegate_type(return_type) {
            invoke_expr.clone()
        } else {
            convert_return(&invoke_expr, return_type, false, known_types, &NO_DEFERRED)
        };
        out.push_str(&format!("        return {};\n", converted));
        out.push_str("    }\n");
    } else if method.is_property_setter {
        let prop_name = to_camel_case(method.name.strip_prefix("put_").unwrap_or(&method.name));
        let param_type = if in_params.first().is_some_and(|p| is_delegate_type(Some(&p.typ))) {
            "DynWinRtValue".to_string()
        } else {
            in_params.first()
                .map(|p| ts_param_type_safe(&p.typ, known_types))
                .unwrap_or_else(|| "any".to_string())
        };
        out.push_str(&format!("    set {}(value: {}) {{\n", prop_name, param_type));
        let arg = in_params.first()
            .map(|p| wrap_arg("value", &p.typ))
            .unwrap_or_else(|| "value".to_string());
        out.push_str(&format!(
            "        {}.method({}).invoke({}, [{}]);\n",
            iface_var, method.vtable_index, obj_expr, arg
        ));
        out.push_str("    }\n");
    } else {
        let ts_params = ts_param_list(&in_params, known_types);
        let array_out_elem = if has_array_out && return_type.is_none() {
            method.params.iter().find_map(|p| {
                if p.direction == ParamDirection::Out || p.direction == ParamDirection::OutFill {
                    if let TypeMeta::Array(inner) = &p.typ { Some(inner.as_ref()) } else { None }
                } else { None }
            })
        } else { None };
        let ts_return = if let Some(elem) = array_out_elem {
            ts_array_element_type(elem, known_types)
        } else {
            ts_return_type_safe(return_type, is_async, known_types)
        };
        let method_name = ts_name_override.map(|s| s.to_string()).unwrap_or_else(|| to_camel_case(&method.name));
        let async_kw = if is_async { "async " } else { "" };

        out.push_str(&format!(
            "    {}{}({}): {} {{\n",
            async_kw, method_name, ts_params, ts_return
        ));

        let args_expr = build_args_expr(&in_params);
        let invoke_expr = format!(
            "{}.method({}).invoke({}, [{}])",
            iface_var, method.vtable_index, obj_expr, args_expr
        );

        if is_with_progress {
            let inner_type = match return_type {
                Some(TypeMeta::AsyncOperationWithProgress(inner, _)) => Some(inner.as_ref()),
                _ => None,
            };
            let inner_convert = convert_return("_v", inner_type, false, known_types, &NO_DEFERRED);
            out.push_str(&format!("        const _op = {};\n", invoke_expr));
            out.push_str(&format!("        const _promise = _op.toPromise().then((_v: DynWinRtValue) => {});\n", inner_convert));
            out.push_str(         "        return Object.assign(_promise, {\n");
            out.push_str(         "            progress(cb: (value: DynWinRtValue) => void) { _op.onProgress(cb); return this; },\n");
            out.push_str(&format!("            toPromise() {{ return _op.toPromise().then((_v: DynWinRtValue) => {}); }},\n", inner_convert));
            out.push_str(         "        });\n");
        } else if !has_return && !is_async {
            out.push_str(&format!("        {};\n", invoke_expr));
        } else if is_async && matches!(return_type, Some(TypeMeta::AsyncAction) | Some(TypeMeta::AsyncActionWithProgress(_))) {
            out.push_str(&format!("        await {}.toPromise();\n", invoke_expr));
        } else if let Some(elem) = array_out_elem {
            let arr_expr = format!("{}.asArray()", invoke_expr);
            let converted = convert_array_return(&arr_expr, elem, known_types, &NO_DEFERRED);
            out.push_str(&format!("        return {};\n", converted));
        } else {
            let converted = convert_return(&invoke_expr, return_type, is_async, known_types, &NO_DEFERRED);
            out.push_str(&format!("        return {};\n", converted));
        }
        out.push_str("    }\n");
    }

    out
}
