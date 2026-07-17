// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;

use crate::meta::{ClassMeta, InterfaceMeta, MethodMeta};
use crate::types::TypeMeta;

use crate::codegen::shared::imports::{
    fill_array_output_index, fill_array_uses_retval_count, get_in_params,
};

use super::naming::to_snake_case;
use super::signature::{py_build_args_expr, py_convert_return, py_runtime_symbol, py_wrap_arg};
use super::type_helpers::{
    method_pydoc, py_method_abi_output_count, py_method_outputs, py_method_return_type,
    py_param_list, py_param_type_safe, py_return_type_safe,
};

fn is_delegate_type(typ: &TypeMeta, delegate_type_names: &HashSet<String>) -> bool {
    matches!(typ, TypeMeta::Delegate { .. })
        || matches!(
            typ,
            TypeMeta::Interface { name, .. } if delegate_type_names.contains(name)
        )
}

fn convert_method_output(
    expr: &str,
    typ: &TypeMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
) -> String {
    if is_delegate_type(typ, delegate_type_names) {
        return expr.to_string();
    }
    if matches!(
        typ,
        TypeMeta::AsyncAction | TypeMeta::AsyncActionWithProgress(_)
    ) {
        return format!("_dynwinrt_wait_action({})", expr);
    }

    py_convert_return(expr, Some(typ), typ.is_async(), known_types)
}

fn method_call_expr(
    iface_var: &str,
    method: &MethodMeta,
    obj_expr: &str,
    args_expr: &str,
) -> String {
    let invoke = if py_method_abi_output_count(method) > 1 {
        "invoke_all"
    } else {
        "invoke"
    };
    format!(
        "{}.method({}).{}({}, [{}])",
        iface_var, method.vtable_index, invoke, obj_expr, args_expr
    )
}

fn emit_method_result(
    out: &mut String,
    call_expr: &str,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
) {
    let outputs = py_method_outputs(method);
    if outputs.is_empty() {
        out.push_str(&format!("        {}\n", call_expr));
        return;
    }

    if py_method_abi_output_count(method) == 1 {
        let converted =
            convert_method_output(call_expr, outputs[0].1, known_types, delegate_type_names);
        out.push_str(&format!("        return {}\n", converted));
        return;
    }

    out.push_str(&format!("        _results = {}\n", call_expr));
    let converted = outputs
        .iter()
        .map(|(index, typ)| {
            let converted = convert_method_output(
                &format!("_results[{}]", index),
                typ,
                known_types,
                delegate_type_names,
            );
            if fill_array_uses_retval_count(method)
                && fill_array_output_index(method) == Some(*index)
            {
                format!(
                    "{}[:_results[{}].to_number()]",
                    converted,
                    py_method_abi_output_count(method) - 1
                )
            } else {
                converted
            }
        })
        .collect::<Vec<_>>();
    if converted.len() == 1 {
        out.push_str(&format!("        return {}\n", converted[0]));
    } else {
        out.push_str(&format!("        return ({})\n", converted.join(", ")));
    }
}

// ======================================================================
// Method generation — Python call pattern
pub(crate) fn generate_factory_method_invoke(
    class: &ClassMeta,
    iface: &InterfaceMeta,
    method: &MethodMeta,
    known_types: &HashSet<String>,
) -> String {
    let in_params = get_in_params(method);
    let py_params = py_param_list(&in_params, known_types);

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
    let call_expr = method_call_expr(
        &format!("_{}", iface.name),
        method,
        &format!("{}._get_f_{}()", class.name, iface.name),
        &args_expr,
    );
    let result_expr = if py_method_abi_output_count(method) > 1 {
        out.push_str(&format!("        _results = {}\n", call_expr));
        format!("_results[{}]", py_method_abi_output_count(method) - 1)
    } else {
        call_expr
    };
    let result_expr = if method.return_type.as_ref().is_some_and(TypeMeta::is_async) {
        format!("{}.wait()", result_expr)
    } else {
        result_expr
    };
    out.push_str(&format!("        return {}({})\n", class.name, result_expr));
    out
}

pub(crate) fn generate_static_method_invoke(
    class: &ClassMeta,
    iface: &InterfaceMeta,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
) -> String {
    let in_params = get_in_params(method);
    let py_params = py_param_list(&in_params, known_types);

    let py_return = py_method_return_type(method, known_types, delegate_type_names);

    let mut out = String::new();

    let statics_call = format!(
        "{cls}._get_s_{iface}()",
        cls = class.name,
        iface = iface.name
    );

    // Static property getter
    if method.is_property_getter && in_params.is_empty() {
        let prop_name = to_snake_case(method.name.strip_prefix("get_").unwrap_or(&method.name));
        // Python doesn't have static properties directly; use classmethod
        out.push_str("    @classmethod\n");
        out.push_str(&format!(
            "    def get_{}(cls) -> {}:\n",
            prop_name, py_return
        ));
        out.push_str(&method_pydoc(method, &in_params));
        let call_expr = method_call_expr(&format!("_{}", iface.name), method, &statics_call, "");
        emit_method_result(
            &mut out,
            &call_expr,
            method,
            known_types,
            delegate_type_names,
        );
    } else {
        out.push_str("    @staticmethod\n");
        let method_name = to_snake_case(&method.name);
        if py_params.is_empty() {
            out.push_str(&format!("    def {}() -> {}:\n", method_name, py_return));
        } else {
            out.push_str(&format!(
                "    def {}({}) -> {}:\n",
                method_name, py_params, py_return
            ));
        }
        out.push_str(&method_pydoc(method, &in_params));
        let args_expr = py_build_args_expr(&in_params);
        let call_expr = method_call_expr(
            &format!("_{}", iface.name),
            method,
            &statics_call,
            &args_expr,
        );
        emit_method_result(
            &mut out,
            &call_expr,
            method,
            known_types,
            delegate_type_names,
        );
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
    generate_method_body(
        iface_var,
        "self._obj",
        method,
        known_types,
        delegate_type_names,
        None,
    )
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

    let mut out = String::new();

    // Event add: create delegate from Python callback
    if method.is_event_add {
        let event_name = to_snake_case(method.name.strip_prefix("add_").unwrap_or(&method.name));
        let delegate_name = in_params.first().and_then(|p| match &p.typ {
            TypeMeta::Parameterized { name, args, .. } => {
                Some(crate::meta::make_parameterized_name(name, args))
            }
            TypeMeta::Delegate { name, .. } => Some(name.clone()),
            _ => None,
        });
        out.push_str(&format!(
            "    def on_{}(self, callback) -> 'DynWinRTValue':\n",
            event_name
        ));
        out.push_str(&method_pydoc(method, &in_params));
        if let Some(ref dname) = delegate_name {
            let iid = py_runtime_symbol(dname, &format!("IID_{}", dname));
            let param_types = py_runtime_symbol(dname, &format!("{}_PARAM_TYPES", dname));
            out.push_str(&format!(
                "        handler = DynWinRtDelegate.create({}, {}, callback)\n",
                iid, param_types
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

    if method.is_property_getter && in_params.is_empty() {
        let prop_name = to_snake_case(method.name.strip_prefix("get_").unwrap_or(&method.name));
        let py_return = if return_type.is_some_and(|typ| is_delegate_type(typ, delegate_type_names))
        {
            "'DynWinRTValue'".to_string()
        } else {
            py_return_type_safe(return_type, known_types)
        };
        out.push_str("    @property\n");
        out.push_str(&format!("    def {}(self) -> {}:\n", prop_name, py_return));
        out.push_str(&method_pydoc(method, &in_params));
        let call_expr = method_call_expr(iface_var, method, obj_expr, "");
        emit_method_result(
            &mut out,
            &call_expr,
            method,
            known_types,
            delegate_type_names,
        );
    } else if method.is_property_setter {
        let prop_name = to_snake_case(method.name.strip_prefix("put_").unwrap_or(&method.name));
        let param_type = if in_params
            .first()
            .is_some_and(|p| is_delegate_type(&p.typ, delegate_type_names))
        {
            "'DynWinRTValue'".to_string()
        } else {
            in_params
                .first()
                .map(|p| py_param_type_safe(&p.typ, known_types))
                .unwrap_or_else(|| "object".to_string())
        };
        out.push_str(&format!("    @{}.setter\n", prop_name));
        out.push_str(&format!(
            "    def {}(self, value: {}):\n",
            prop_name, param_type
        ));
        out.push_str(&method_pydoc(method, &in_params));
        let arg = in_params
            .first()
            .map(|p| py_wrap_arg("value", &p.typ))
            .unwrap_or_else(|| "value".to_string());
        out.push_str(&format!(
            "        {}.method({}).invoke({}, [{}])\n",
            iface_var, method.vtable_index, obj_expr, arg
        ));
    } else {
        let py_params = py_param_list(&in_params, known_types);
        let py_return = py_method_return_type(method, known_types, delegate_type_names);
        let method_name = name_override
            .map(|s| s.to_string())
            .unwrap_or_else(|| to_snake_case(&method.name));

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
        let call_expr = method_call_expr(iface_var, method, obj_expr, &args_expr);
        emit_method_result(
            &mut out,
            &call_expr,
            method,
            known_types,
            delegate_type_names,
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_delegate_return_stays_raw() {
        let method = MethodMeta {
            name: "GetHandler".into(),
            raw_name: "GetHandler".into(),
            vtable_index: 6,
            return_type: Some(TypeMeta::Interface {
                namespace: "Contoso".into(),
                name: "Handler".into(),
                iid: "11111111-1111-1111-1111-111111111111".into(),
            }),
            ..Default::default()
        };
        let iface = InterfaceMeta {
            name: "IWidgetStatics".into(),
            methods: vec![method.clone()],
            ..Default::default()
        };
        let class = ClassMeta {
            name: "Widget".into(),
            ..Default::default()
        };
        let code = generate_static_method_invoke(
            &class,
            &iface,
            &method,
            &HashSet::from(["Handler".into()]),
            &HashSet::from(["Handler".into()]),
        );

        assert!(code.contains("def get_handler() -> 'DynWinRTValue':"));
        assert!(code.contains("return _IWidgetStatics.method(6).invoke("));
        assert!(!code.contains("_dynwinrt_symbol('handler', 'Handler')"));
    }
}
