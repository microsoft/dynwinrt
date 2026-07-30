// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;

use crate::meta::{ClassMeta, InterfaceMeta, MethodMeta};
use crate::types::TypeMeta;

use crate::codegen::winrt::shared::imports::{
    fill_array_output_index, fill_array_uses_retval_count, get_in_params,
};

use super::naming::to_snake_case;
use super::signature::{
    py_convert_return, py_runtime_symbol, py_type_guard, py_wrap_arg, py_wrap_async,
};
use super::type_helpers::{
    method_pydoc, py_factory_return_type, py_method_abi_output_count, py_method_outputs,
    py_method_return_type, py_param_list, py_param_type_safe, py_return_type_safe,
};

fn is_delegate_type(typ: &TypeMeta, delegate_type_names: &HashSet<String>) -> bool {
    delegate_name(typ, delegate_type_names).is_some()
}

fn delegate_name(typ: &TypeMeta, delegate_type_names: &HashSet<String>) -> Option<String> {
    match typ {
        TypeMeta::Delegate { name, .. } => Some(name.clone()),
        TypeMeta::Interface { name, .. } if delegate_type_names.contains(name) => {
            Some(name.clone())
        }
        TypeMeta::Parameterized { name, args, .. } => {
            let concrete = crate::meta::make_parameterized_name(name, args);
            delegate_type_names.contains(&concrete).then_some(concrete)
        }
        _ => None,
    }
}

fn py_wrap_method_arg(name: &str, typ: &TypeMeta, delegate_type_names: &HashSet<String>) -> String {
    if let Some(delegate) = delegate_name(typ, delegate_type_names) {
        return format!(
            "_dynwinrt_delegate({name}, {}, {})",
            py_runtime_symbol(&delegate, &format!("IID_{delegate}")),
            py_runtime_symbol(&delegate, &format!("{delegate}_PARAM_TYPES"))
        );
    }
    py_wrap_arg(name, typ)
}

fn py_build_method_args_expr(
    in_params: &[&crate::meta::ParamMeta],
    delegate_type_names: &HashSet<String>,
) -> String {
    in_params
        .iter()
        .map(|param| {
            py_wrap_method_arg(&to_snake_case(&param.name), &param.typ, delegate_type_names)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn py_method_type_guard(
    name: &str,
    typ: &TypeMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
) -> String {
    if is_delegate_type(typ, delegate_type_names) {
        return format!(
            "(callable({name}) or isinstance(getattr({name}, '_obj', {name}), DynWinRTValue))"
        );
    }
    py_type_guard(name, typ, known_types)
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
    delegate_type_names: &HashSet<String>,
) -> String {
    generate_factory_method_invoke_named(
        class,
        iface,
        method,
        known_types,
        delegate_type_names,
        None,
    )
}

fn generate_factory_method_invoke_named(
    class: &ClassMeta,
    iface: &InterfaceMeta,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
    name_override: Option<&str>,
) -> String {
    let in_params = get_in_params(method);
    let py_params = py_param_list(&in_params, known_types, delegate_type_names);

    let return_py_type = py_factory_return_type(&class.name, method, known_types);

    let mut out = String::new();
    let method_name = name_override
        .map(str::to_string)
        .unwrap_or_else(|| to_snake_case(&method.name));

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

    let args_expr = py_build_method_args_expr(&in_params, delegate_type_names);
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
    let result_expr =
        if let Some(async_type) = method.return_type.as_ref().filter(|typ| typ.is_async()) {
            let result_converter = match async_type {
                TypeMeta::AsyncOperation(_) | TypeMeta::AsyncOperationWithProgress(_, _) => {
                    Some(format!("lambda value: {}._from_native(value)", class.name))
                }
                _ => None,
            };
            py_wrap_async(&result_expr, async_type, result_converter, known_types)
        } else {
            format!("{}._from_native({})", class.name, result_expr)
        };
    out.push_str(&format!("        return {}\n", result_expr));
    out
}

pub(crate) fn generate_static_method_invoke(
    class: &ClassMeta,
    iface: &InterfaceMeta,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
) -> String {
    generate_static_method_invoke_named(
        class,
        iface,
        method,
        known_types,
        delegate_type_names,
        None,
    )
}

fn generate_static_method_invoke_named(
    class: &ClassMeta,
    iface: &InterfaceMeta,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
    name_override: Option<&str>,
) -> String {
    let in_params = get_in_params(method);
    let py_params = py_param_list(&in_params, known_types, delegate_type_names);

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
        let method_name = name_override
            .map(str::to_string)
            .unwrap_or_else(|| to_snake_case(&method.name));
        if py_params.is_empty() {
            out.push_str(&format!("    def {}() -> {}:\n", method_name, py_return));
        } else {
            out.push_str(&format!(
                "    def {}({}) -> {}:\n",
                method_name, py_params, py_return
            ));
        }
        out.push_str(&method_pydoc(method, &in_params));
        let args_expr = py_build_method_args_expr(&in_params, delegate_type_names);
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

pub(crate) struct InstanceOverload<'a> {
    pub(crate) iface_var: String,
    pub(crate) obj_expr: String,
    pub(crate) method: &'a MethodMeta,
}

pub(crate) fn generate_instance_method_group(
    overloads: &[InstanceOverload<'_>],
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
) -> String {
    if overloads.len() == 1 {
        let overload = &overloads[0];
        return generate_method_body(
            &overload.iface_var,
            &overload.obj_expr,
            overload.method,
            known_types,
            delegate_type_names,
            None,
        );
    }

    let overload_names =
        super::overloads::method_names(overloads.iter().map(|overload| overload.method));
    let public_name = super::overloads::method_group_key(overloads[0].method, &overload_names);
    let mut out = String::new();
    let mut private_names = Vec::with_capacity(overloads.len());
    for overload in overloads {
        let private_name = format!("_{}_{}", public_name, overload.method.vtable_index);
        out.push_str(&generate_method_body(
            &overload.iface_var,
            &overload.obj_expr,
            overload.method,
            known_types,
            delegate_type_names,
            Some(&private_name),
        ));
        out.push('\n');
        private_names.push(private_name);
    }

    out.push_str(&format!("    def {public_name}(self, *args, **kwargs):\n"));
    for (overload, private_name) in overloads.iter().zip(private_names) {
        let in_params = get_in_params(overload.method);
        let parameter_names = in_params
            .iter()
            .map(|param| format!("'{}'", to_snake_case(&param.name)))
            .collect::<Vec<_>>()
            .join(", ");
        let parameter_names = if parameter_names.is_empty() {
            "()".to_string()
        } else {
            format!("({parameter_names},)")
        };
        out.push_str(&format!(
            "        _bound = _dynwinrt_bind_overload({}, args, kwargs)\n",
            parameter_names
        ));
        let guards = in_params
            .iter()
            .enumerate()
            .map(|(index, param)| {
                py_method_type_guard(
                    &format!("_bound[{index}]"),
                    &param.typ,
                    known_types,
                    delegate_type_names,
                )
            })
            .collect::<Vec<_>>();
        let condition = if guards.is_empty() {
            "_bound is not None".to_string()
        } else {
            format!("_bound is not None and {}", guards.join(" and "))
        };
        out.push_str(&format!(
            "        if {condition}:\n            return self.{private_name}(*_bound)\n"
        ));
    }
    out.push_str(&format!(
        "        raise TypeError(\"No matching overload for {public_name}\")\n"
    ));
    out
}

#[derive(Clone, Copy)]
pub(crate) enum StaticOverloadKind {
    Factory,
    Static,
}

pub(crate) struct StaticOverload<'a> {
    pub(crate) class: &'a ClassMeta,
    pub(crate) iface: &'a InterfaceMeta,
    pub(crate) method: &'a MethodMeta,
    pub(crate) kind: StaticOverloadKind,
}

pub(crate) fn generate_static_method_group(
    overloads: &[StaticOverload<'_>],
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
) -> String {
    if overloads.len() == 1 {
        let overload = &overloads[0];
        return match overload.kind {
            StaticOverloadKind::Factory => generate_factory_method_invoke(
                overload.class,
                overload.iface,
                overload.method,
                known_types,
                delegate_type_names,
            ),
            StaticOverloadKind::Static => generate_static_method_invoke(
                overload.class,
                overload.iface,
                overload.method,
                known_types,
                delegate_type_names,
            ),
        };
    }

    let overload_names =
        super::overloads::method_names(overloads.iter().map(|overload| overload.method));
    let public_name = super::overloads::method_group_key(overloads[0].method, &overload_names);
    let mut out = String::new();
    let mut private_names = Vec::with_capacity(overloads.len());
    for overload in overloads {
        let private_name = format!("_{}_{}", public_name, overload.method.vtable_index);
        let code = match overload.kind {
            StaticOverloadKind::Factory => generate_factory_method_invoke_named(
                overload.class,
                overload.iface,
                overload.method,
                known_types,
                delegate_type_names,
                Some(&private_name),
            ),
            StaticOverloadKind::Static => generate_static_method_invoke_named(
                overload.class,
                overload.iface,
                overload.method,
                known_types,
                delegate_type_names,
                Some(&private_name),
            ),
        };
        out.push_str(&code);
        out.push('\n');
        private_names.push(private_name);
    }

    out.push_str("    @staticmethod\n");
    out.push_str(&format!("    def {public_name}(*args, **kwargs):\n"));
    for (overload, private_name) in overloads.iter().zip(private_names) {
        let in_params = get_in_params(overload.method);
        let parameter_names = in_params
            .iter()
            .map(|param| format!("'{}'", to_snake_case(&param.name)))
            .collect::<Vec<_>>()
            .join(", ");
        let parameter_names = if parameter_names.is_empty() {
            "()".to_string()
        } else {
            format!("({parameter_names},)")
        };
        out.push_str(&format!(
            "        _bound = _dynwinrt_bind_overload({parameter_names}, args, kwargs)\n"
        ));
        let guards = in_params
            .iter()
            .enumerate()
            .map(|(index, param)| {
                py_method_type_guard(
                    &format!("_bound[{index}]"),
                    &param.typ,
                    known_types,
                    delegate_type_names,
                )
            })
            .collect::<Vec<_>>();
        let condition = if guards.is_empty() {
            "_bound is not None".to_string()
        } else {
            format!("_bound is not None and {}", guards.join(" and "))
        };
        out.push_str(&format!(
            "        if {condition}:\n            return {}.{private_name}(*_bound)\n",
            overload.class.name
        ));
    }
    out.push_str(&format!(
        "        raise TypeError(\"No matching overload for {public_name}\")\n"
    ));
    out
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
            "Callable[..., object] | 'DynWinRTValue'".to_string()
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
            .map(|p| py_wrap_method_arg("value", &p.typ, delegate_type_names))
            .unwrap_or_else(|| "value".to_string());
        out.push_str(&format!(
            "        {}.method({}).invoke({}, [{}])\n",
            iface_var, method.vtable_index, obj_expr, arg
        ));
    } else {
        let py_params = py_param_list(&in_params, known_types, delegate_type_names);
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

        let args_expr = py_build_method_args_expr(&in_params, delegate_type_names);
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
    use crate::meta::ParamMeta;

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

        assert!(
            code.contains("def get_handler() -> 'DynWinRTValue':"),
            "{code}"
        );
        assert!(code.contains("return _IWidgetStatics.method(6).invoke("));
        assert!(!code.contains("_dynwinrt_symbol('handler', 'Handler')"));
    }

    #[test]
    fn async_operation_returns_typed_awaitable_without_waiting() {
        let method = MethodMeta {
            name: "LoadAsync".into(),
            raw_name: "LoadAsync".into(),
            vtable_index: 6,
            return_type: Some(TypeMeta::AsyncOperation(Box::new(TypeMeta::U32))),
            ..Default::default()
        };

        let code = generate_method_body(
            "_IReader",
            "self._obj",
            &method,
            &HashSet::new(),
            &HashSet::new(),
            None,
        );

        assert!(code.contains("def load_async(self) -> WinRTAsync[int]:"));
        assert!(code.contains("return _DynWinRTAsync("));
        assert!(code.contains("lambda value: value.to_number()"));
        assert!(!code.contains(".wait()"));
    }

    #[test]
    fn async_operation_with_progress_converts_result_and_progress() {
        let method = MethodMeta {
            name: "WriteAsync".into(),
            raw_name: "WriteAsync".into(),
            vtable_index: 6,
            params: vec![ParamMeta {
                name: "buffer".into(),
                typ: TypeMeta::Object,
                direction: crate::meta::ParamDirection::In,
            }],
            return_type: Some(TypeMeta::AsyncOperationWithProgress(
                Box::new(TypeMeta::U32),
                Box::new(TypeMeta::U64),
            )),
            ..Default::default()
        };

        let code = generate_method_body(
            "_IOutputStream",
            "self._obj",
            &method,
            &HashSet::new(),
            &HashSet::new(),
            None,
        );

        assert!(code.contains(
            "def write_async(self, buffer: 'DynWinRTValue') -> WinRTAsyncWithProgress[int, int]:"
        ));
        assert!(code.contains("return _DynWinRTAsyncWithProgress("));
        assert!(code.contains("lambda value: value.to_number()"));
        assert!(code.contains("lambda value: value.to_i64()"));
        assert!(!code.contains(".wait()"));
    }

    #[test]
    fn instance_overloads_merge_numeric_suffixes() {
        let first = MethodMeta {
            name: "Read".into(),
            raw_name: "Read".into(),
            vtable_index: 6,
            params: vec![ParamMeta {
                name: "value".into(),
                typ: TypeMeta::String,
                direction: crate::meta::ParamDirection::In,
            }],
            ..Default::default()
        };
        let second = MethodMeta {
            name: "Read2".into(),
            raw_name: "Read2".into(),
            vtable_index: 7,
            params: vec![ParamMeta {
                name: "value".into(),
                typ: TypeMeta::I32,
                direction: crate::meta::ParamDirection::In,
            }],
            ..Default::default()
        };
        let overloads = vec![
            InstanceOverload {
                iface_var: "_IReader".into(),
                obj_expr: "self._obj".into(),
                method: &first,
            },
            InstanceOverload {
                iface_var: "_IReader".into(),
                obj_expr: "self._obj".into(),
                method: &second,
            },
        ];

        let code = generate_instance_method_group(&overloads, &HashSet::new(), &HashSet::new());
        assert!(code.contains("def _read_6(self, value: str)"));
        assert!(code.contains("def _read_7(self, value: int)"));
        assert!(code.contains("def read(self, *args, **kwargs)"));
        assert!(code.contains("isinstance(_bound[0], str)"));
        assert!(code.contains("isinstance(_bound[0], int)"));
    }

    #[test]
    fn delegate_overload_accepts_python_callable() {
        let callback = MethodMeta {
            name: "Run".into(),
            raw_name: "Run".into(),
            vtable_index: 6,
            params: vec![ParamMeta {
                name: "handler".into(),
                typ: TypeMeta::Delegate {
                    namespace: "Contoso".into(),
                    name: "WorkItemHandler".into(),
                    iid: "11111111-1111-1111-1111-111111111111".into(),
                },
                direction: crate::meta::ParamDirection::In,
            }],
            ..Default::default()
        };
        let text = MethodMeta {
            name: "Run2".into(),
            raw_name: "Run2".into(),
            vtable_index: 7,
            params: vec![ParamMeta {
                name: "value".into(),
                typ: TypeMeta::String,
                direction: crate::meta::ParamDirection::In,
            }],
            ..Default::default()
        };
        let overloads = vec![
            InstanceOverload {
                iface_var: "_IRunner".into(),
                obj_expr: "self._obj".into(),
                method: &callback,
            },
            InstanceOverload {
                iface_var: "_IRunner".into(),
                obj_expr: "self._obj".into(),
                method: &text,
            },
        ];

        let code = generate_instance_method_group(
            &overloads,
            &HashSet::new(),
            &HashSet::from(["WorkItemHandler".into()]),
        );
        assert!(code.contains("callable(_bound[0])"));
        assert!(code.contains("_dynwinrt_delegate(handler,"));
        assert!(code.contains("'work_item_handler', 'IID_WorkItemHandler'"));
        assert!(code.contains("'work_item_handler', 'WorkItemHandler_PARAM_TYPES'"));
    }

    #[test]
    fn static_overloads_generate_one_dispatcher() {
        let class = ClassMeta {
            name: "Factory".into(),
            ..Default::default()
        };
        let iface = InterfaceMeta {
            name: "IFactoryStatics".into(),
            ..Default::default()
        };
        let first = MethodMeta {
            name: "Create".into(),
            raw_name: "Create".into(),
            vtable_index: 6,
            ..Default::default()
        };
        let second = MethodMeta {
            name: "Create2".into(),
            raw_name: "Create2".into(),
            vtable_index: 7,
            params: vec![ParamMeta {
                name: "value".into(),
                typ: TypeMeta::String,
                direction: crate::meta::ParamDirection::In,
            }],
            ..Default::default()
        };
        let overloads = vec![
            StaticOverload {
                class: &class,
                iface: &iface,
                method: &first,
                kind: StaticOverloadKind::Static,
            },
            StaticOverload {
                class: &class,
                iface: &iface,
                method: &second,
                kind: StaticOverloadKind::Static,
            },
        ];

        let code = generate_static_method_group(&overloads, &HashSet::new(), &HashSet::new());
        assert!(code.contains("def _create_6()"));
        assert!(code.contains("def _create_7(value: str)"));
        assert!(code.contains("def create(*args, **kwargs)"));
    }
}
