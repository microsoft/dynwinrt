// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;

use crate::meta::{ClassMeta, InterfaceMeta, MethodMeta};
use crate::types::TypeMeta;

use crate::codegen::winrt::extensions::winui::{self, WinUiCallBehavior};
use crate::codegen::winrt::shared::imports::{
    fill_array_output_index, fill_array_uses_retval_count, get_in_params,
};

use super::naming::to_snake_case;
use super::signature::{
    py_convert_return, py_runtime_symbol, py_type_guard, py_wrap_arg, py_wrap_async,
    py_wrap_async_with_converters,
};
use super::type_helpers::{
    method_pydoc, py_delegate_callable_type, py_factory_return_type, py_method_abi_output_count,
    py_method_outputs, py_method_return_type, py_output_type, py_param_list,
};

fn is_delegate_type(typ: &TypeMeta, delegate_type_names: &HashSet<String>) -> bool {
    delegate_name(typ, delegate_type_names).is_some()
}

fn delegate_value_converter(
    typ: &TypeMeta,
    delegate_type_names: &HashSet<String>,
) -> Option<String> {
    if is_delegate_type(typ, delegate_type_names) {
        return Some("lambda value: None if value.is_null() else value".into());
    }
    if let TypeMeta::Array(inner) = typ
        && is_delegate_type(inner, delegate_type_names)
    {
        return Some(
            "lambda value: [None if item.is_null() else item for item in value.as_array().to_values()]"
                .into(),
        );
    }
    None
}

/// Build a Python callback signature + wrapper expression for an event delegate.
///
/// Returns `(signature, wrapper)`:
/// - `signature` is a Python type annotation (e.g., `Callable[['Foo', 'Bar'], object]`).
/// - `wrapper` is an expression that produces the ABI-facing callable, unwrapping
///   raw `DynWinRTValue` sender/args back into projected Python objects before
///   invoking the user's `callback`.
///
/// The wrapper falls back to a passthrough (`callback`) for unknown delegate shapes.
fn build_event_wrapper(typ: Option<&TypeMeta>, known_types: &HashSet<String>) -> (String, String) {
    match typ {
        Some(typ @ TypeMeta::Parameterized { name, args, .. })
            if name.split('`').next() == Some("TypedEventHandler") && args.len() == 2 =>
        {
            let sender_conv = py_convert_return("__sender__", Some(&args[0]), false, known_types);
            let args_conv = py_convert_return("__args__", Some(&args[1]), false, known_types);
            let sig = py_delegate_callable_type(typ, known_types);
            let wrapper = format!(
                "(lambda callback=callback: (lambda __sender__, __args__: callback({}, {})))()",
                sender_conv, args_conv
            );
            (sig, wrapper)
        }
        Some(typ @ TypeMeta::Parameterized { name, args, .. })
            if name.split('`').next() == Some("EventHandler") && args.len() == 1 =>
        {
            let args_conv = py_convert_return("__args__", Some(&args[0]), false, known_types);
            let sig = py_delegate_callable_type(typ, known_types);
            let wrapper = format!(
                "(lambda callback=callback: (lambda __sender__, __args__: callback(__sender__, {})))()",
                args_conv
            );
            (sig, wrapper)
        }
        Some(typ @ TypeMeta::Parameterized { name, args, .. })
            if name.split('`').next() == Some("VectorChangedEventHandler") && args.len() == 1 =>
        {
            let observable_name = crate::meta::make_parameterized_name("IObservableVector", args);
            let sender = format!(
                "(lambda value: None if value.is_null() else {}(value))(__sender__)",
                py_runtime_symbol(&observable_name, &observable_name)
            );
            let event_args = format!(
                "(lambda value: None if value.is_null() else {}(value))(__args__)",
                py_runtime_symbol("IVectorChangedEventArgs", "IVectorChangedEventArgs")
            );
            let sig = py_delegate_callable_type(typ, known_types);
            let wrapper = format!(
                "(lambda callback=callback: (lambda __sender__, __args__: callback({}, {})))()",
                sender, event_args
            );
            (sig, wrapper)
        }
        _ => ("Callable[..., object]".to_string(), "callback".to_string()),
    }
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

pub(crate) fn py_wrap_method_arg(
    name: &str,
    typ: &TypeMeta,
    delegate_type_names: &HashSet<String>,
) -> String {
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
    if let Some(converter) = delegate_value_converter(typ, delegate_type_names) {
        return format!("({converter})({expr})");
    }
    match typ {
        TypeMeta::AsyncOperation(result) => {
            return py_wrap_async_with_converters(
                expr,
                typ,
                delegate_value_converter(result, delegate_type_names),
                None,
                known_types,
            );
        }
        TypeMeta::AsyncActionWithProgress(progress) => {
            return py_wrap_async_with_converters(
                expr,
                typ,
                None,
                delegate_value_converter(progress, delegate_type_names),
                known_types,
            );
        }
        TypeMeta::AsyncOperationWithProgress(result, progress) => {
            return py_wrap_async_with_converters(
                expr,
                typ,
                delegate_value_converter(result, delegate_type_names),
                delegate_value_converter(progress, delegate_type_names),
                known_types,
            );
        }
        _ => {}
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
    } else if winui::call_behavior(iface_var.trim_start_matches('_'), &method.name)
        == WinUiCallBehavior::BlockingReentrant
    {
        "invoke_detached"
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
    iface: &InterfaceMeta,
    iface_var: &str,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
) -> String {
    let property_has_getter = !method.is_property_setter
        || method.name.strip_prefix("put_").is_some_and(|suffix| {
            iface
                .methods
                .iter()
                .any(|candidate| candidate.name == format!("get_{suffix}"))
        });
    generate_method_body(
        iface_var,
        "self._obj",
        method,
        known_types,
        delegate_type_names,
        None,
        Some(&iface.methods),
        property_has_getter,
    )
}

pub(crate) struct InstanceOverload<'a> {
    pub(crate) iface_var: String,
    pub(crate) obj_expr: String,
    pub(crate) method: &'a MethodMeta,
    pub(crate) sibling_methods: Option<&'a [MethodMeta]>,
    pub(crate) property_has_getter: bool,
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
            overload.sibling_methods,
            overload.property_has_getter,
        );
    }

    let mut ordered_overloads = overloads.iter().collect::<Vec<_>>();
    ordered_overloads.sort_by(|left, right| {
        super::overloads::cmp_python_dispatch_methods(left.method, right.method)
    });

    let overload_names =
        super::overloads::method_names(ordered_overloads.iter().map(|overload| overload.method));
    let public_name =
        super::overloads::method_group_key(ordered_overloads[0].method, &overload_names);
    let mut out = String::new();
    let mut private_names = Vec::with_capacity(ordered_overloads.len());
    for overload in &ordered_overloads {
        let private_name = format!("_{}_{}", public_name, overload.method.vtable_index);
        out.push_str(&generate_method_body(
            &overload.iface_var,
            &overload.obj_expr,
            overload.method,
            known_types,
            delegate_type_names,
            Some(&private_name),
            overload.sibling_methods,
            overload.property_has_getter,
        ));
        out.push('\n');
        private_names.push(private_name);
    }

    out.push_str(&format!("    def {public_name}(self, *args, **kwargs):\n"));
    let public_params = get_in_params(ordered_overloads[0].method);
    out.push_str(&method_pydoc(ordered_overloads[0].method, &public_params));
    for (overload, private_name) in ordered_overloads.iter().zip(private_names) {
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

    let mut ordered_overloads = overloads.iter().collect::<Vec<_>>();
    ordered_overloads.sort_by(|left, right| {
        super::overloads::cmp_python_dispatch_methods(left.method, right.method)
    });

    let overload_names =
        super::overloads::method_names(ordered_overloads.iter().map(|overload| overload.method));
    let public_name =
        super::overloads::method_group_key(ordered_overloads[0].method, &overload_names);
    let mut out = String::new();
    let mut private_names = Vec::with_capacity(ordered_overloads.len());
    for overload in &ordered_overloads {
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
    let public_params = get_in_params(ordered_overloads[0].method);
    out.push_str(&method_pydoc(ordered_overloads[0].method, &public_params));
    for (overload, private_name) in ordered_overloads.iter().zip(private_names) {
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
    sibling_methods: Option<&[MethodMeta]>,
    property_has_getter: bool,
) -> String {
    let in_params = get_in_params(method);
    let return_type = method.return_type.as_ref();

    let mut out = String::new();

    // Event add: preserve the established token-returning API. When a
    // matching remove method is available, also emit subscribe_<event> and
    // once_<event> helpers that return idempotent unsubscribe callables.
    if method.is_event_add {
        let suffix = method.name.strip_prefix("add_").unwrap_or(&method.name);
        let event_name = to_snake_case(suffix);
        let delegate_typ = in_params.first().map(|p| &p.typ);
        let delegate_name = delegate_typ.and_then(|typ| delegate_name(typ, delegate_type_names));
        // Find matching remove_<Suffix> in the same interface to know its vtable index.
        let remove_target = format!("remove_{}", suffix);
        let remove_idx = sibling_methods.and_then(|methods| {
            methods
                .iter()
                .find(|m| m.name == remove_target)
                .map(|m| m.vtable_index)
        });

        // Compute callback wrapper: for TypedEventHandler<S, A> / EventHandler<A>,
        // wrap raw ABI args back into projected values before invoking the user callback.
        let (callback_signature, wrapper) = build_event_wrapper(delegate_typ, known_types);

        out.push_str(&format!(
            "    def on_{}(self, callback: {}):\n",
            event_name, callback_signature,
        ));
        out.push_str(&method_pydoc(method, &in_params));
        // Wrapping expression bound to `_wrapped` before delegate construction.
        out.push_str(&format!("        _wrapped = {}\n", wrapper));
        if let Some(ref dname) = delegate_name {
            let iid = py_runtime_symbol(dname, &format!("IID_{}", dname));
            let param_types = py_runtime_symbol(dname, &format!("{}_PARAM_TYPES", dname));
            out.push_str(&format!(
                "        _handler = _dynwinrt_create_delegate({}, {}, _wrapped)\n",
                iid, param_types
            ));
        } else {
            out.push_str(
                "        _handler = _dynwinrt_create_delegate(DynWinRTType.object().iid(), [DynWinRTType.object(), DynWinRTType.object()], _wrapped)\n"
            );
        }
        out.push_str(&format!(
            "        return {}.method({}).invoke({}, [_handler.to_value()])\n",
            iface_var, method.vtable_index, obj_expr
        ));

        // subscribe_<event>: ergonomic, idempotent cancellation while keeping
        // on_<event>/off_<event> source compatibility.
        if remove_idx.is_some() {
            out.push('\n');
            out.push_str(&format!(
                "    def subscribe_{}(self, callback: {}):\n",
                event_name, callback_signature,
            ));
            out.push_str(&format!(
                "        _token = self.on_{}(callback)\n",
                event_name
            ));
            out.push_str("        _active = [True]\n");
            out.push_str("        def _unsubscribe():\n");
            out.push_str("            if not _active[0]:\n");
            out.push_str("                return\n");
            out.push_str("            _active[0] = False\n");
            out.push_str("            try:\n");
            out.push_str(&format!(
                "                self.off_{}(_token)\n",
                event_name
            ));
            out.push_str("            except Exception:\n");
            out.push_str("                _active[0] = True\n");
            out.push_str("                raise\n");
            out.push_str("        return _unsubscribe\n");

            // once_<event>: clear the active flag before callback invocation
            // so same-thread reentrant delivery cannot invoke it twice.
            out.push('\n');
            out.push_str(&format!(
                "    def once_{}(self, callback: {}):\n",
                event_name, callback_signature,
            ));
            out.push_str("        _state = [True, None]\n");
            out.push_str("        def _once(*args, **kwargs):\n");
            out.push_str("            if not _state[0]:\n");
            out.push_str("                return None\n");
            out.push_str("            _state[0] = False\n");
            out.push_str("            _unsub = _state[1]\n");
            out.push_str("            if _unsub is not None:\n");
            out.push_str("                _unsub()\n");
            out.push_str("            return callback(*args, **kwargs)\n");
            out.push_str(&format!(
                "        _unsubscribe = self.subscribe_{}(_once)\n",
                event_name
            ));
            out.push_str("        _state[1] = _unsubscribe\n");
            out.push_str("        if not _state[0]:\n");
            out.push_str("            _unsubscribe()\n");
            out.push_str("        return _unsubscribe\n");
        }
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
        let py_return = return_type
            .map(|typ| py_output_type(typ, known_types, delegate_type_names))
            .unwrap_or_else(|| "None".to_string());
        out.push_str("    @_property\n");
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
        let param_type = in_params
            .first()
            .map(|p| {
                if is_delegate_type(&p.typ, delegate_type_names) {
                    // Reuse py_delegate_param_type via a temporary param_list call.
                    let params = super::type_helpers::py_param_list(
                        std::slice::from_ref(p),
                        known_types,
                        delegate_type_names,
                    );
                    // params is "name: Type" — extract the "Type" part.
                    params
                        .splitn(2, ": ")
                        .nth(1)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "Callable[..., object] | 'DynWinRTValue'".to_string())
                } else {
                    super::type_helpers::py_param_type_safe(&p.typ, known_types)
                }
            })
            .unwrap_or_else(|| "object".to_string());
        if property_has_getter {
            out.push_str(&format!("    @{}.setter\n", prop_name));
            out.push_str(&format!(
                "    def {}(self, value: {}):\n",
                prop_name, param_type
            ));
        } else {
            out.push_str(&format!(
                "    def set_{}(self, value: {}):\n",
                prop_name, param_type
            ));
        }
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
    use crate::meta::{ParamDirection, ParamMeta};
    use std::process::Command;

    fn overloaded_method(name: &str, vtable_index: usize, typ: TypeMeta) -> MethodMeta {
        MethodMeta {
            name: name.into(),
            raw_name: name.into(),
            vtable_index,
            params: vec![ParamMeta {
                name: "value".into(),
                typ,
                direction: ParamDirection::In,
            }],
            ..Default::default()
        }
    }

    fn instance_overload(method: &MethodMeta) -> InstanceOverload<'_> {
        InstanceOverload {
            iface_var: "_IReader".into(),
            obj_expr: "self._obj".into(),
            method,
            sibling_methods: None,
            property_has_getter: true,
        }
    }

    fn static_overload<'a>(
        class: &'a ClassMeta,
        iface: &'a InterfaceMeta,
        method: &'a MethodMeta,
    ) -> StaticOverload<'a> {
        StaticOverload {
            class,
            iface,
            method,
            kind: StaticOverloadKind::Static,
        }
    }

    fn enum_type(name: &str, is_flags: bool) -> TypeMeta {
        TypeMeta::Enum {
            namespace: "Contoso".into(),
            name: name.into(),
            underlying: Box::new(TypeMeta::I32),
            members: Vec::new(),
            is_flags,
            doc: None,
            deprecated: None,
        }
    }

    fn assert_contains_in_order(text: &str, first: &str, second: &str) {
        let first_index = text
            .find(first)
            .unwrap_or_else(|| panic!("missing `{first}` in:\n{text}"));
        let second_index = text
            .find(second)
            .unwrap_or_else(|| panic!("missing `{second}` in:\n{text}"));
        assert!(first_index < second_index, "{text}");
    }

    fn extract_generated_block(code: &str, marker: &str) -> String {
        code.find(marker)
            .map(|index| code[index..].to_string())
            .unwrap_or_else(|| panic!("missing `{marker}` in:\n{code}"))
    }

    fn run_python(script: &str) -> String {
        fn invoke(
            program: &str,
            args: &[&str],
            script: &str,
        ) -> std::io::Result<std::process::Output> {
            let mut command = Command::new(program);
            for arg in args {
                command.arg(arg);
            }
            command.arg(script).output()
        }

        let output = invoke("python", &["-c"], script).or_else(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                invoke("py", &["-3", "-c"], script)
            } else {
                Err(error)
            }
        });
        let output = output.unwrap_or_else(|error| panic!("failed to launch Python: {error}"));
        assert!(
            output.status.success(),
            "python script failed\nstdout:\n{}\nstderr:\n{}\nscript:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            script
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

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
            code.contains("def get_handler() -> DynWinRTValue | None:"),
            "{code}"
        );
        assert!(code.contains("None if value.is_null() else value"));
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
            None,
            true,
        );

        assert!(code.contains("def load_async(self) -> WinRTAsync[int]:"));
        assert!(code.contains("return _dynwinrt_track_projected(_DynWinRTAsync("));
        assert!(code.contains("'WinRTAsync')"));
        assert!(code.contains("lambda value: value.to_u32()"));
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
            None,
            true,
        );

        assert!(code.contains(
            "def write_async(self, buffer: 'DynWinRTValue') -> WinRTAsyncWithProgress[int, int]:"
        ));
        assert!(code.contains("return _dynwinrt_track_projected(_DynWinRTAsyncWithProgress("));
        assert!(code.contains("'WinRTAsyncWithProgress')"));
        assert!(code.contains("lambda value: value.to_u32()"));
        assert!(code.contains("lambda value: value.to_u64()"));
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
                sibling_methods: None,
                property_has_getter: true,
            },
            InstanceOverload {
                iface_var: "_IReader".into(),
                obj_expr: "self._obj".into(),
                method: &second,
                sibling_methods: None,
                property_has_getter: true,
            },
        ];

        let code = generate_instance_method_group(&overloads, &HashSet::new(), &HashSet::new());
        assert!(code.contains("def _read_6(self, value: str)"));
        assert!(code.contains("def _read_7(self, value: int)"));
        assert!(code.contains("def read(self, *args, **kwargs)"));
        assert!(code.contains("isinstance(_bound[0], str)"));
        assert!(code.contains(
            "isinstance(_bound[0], int) and not isinstance(_bound[0], bool) and not isinstance(_bound[0], __import__('enum').Enum)"
        ));
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
                sibling_methods: None,
                property_has_getter: true,
            },
            InstanceOverload {
                iface_var: "_IRunner".into(),
                obj_expr: "self._obj".into(),
                method: &text,
                sibling_methods: None,
                property_has_getter: true,
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
    fn event_helpers_preserve_tokens_and_are_idempotent() {
        let handler_type = TypeMeta::Parameterized {
            namespace: "Windows.Foundation".into(),
            name: "TypedEventHandler`2".into(),
            piid: "11111111-1111-1111-1111-111111111111".into(),
            args: vec![
                TypeMeta::RuntimeClass {
                    namespace: "Contoso".into(),
                    name: "Widget".into(),
                    default_interface: None,
                },
                TypeMeta::Object,
            ],
        };
        let add = MethodMeta {
            name: "add_Changed".into(),
            raw_name: "add_Changed".into(),
            vtable_index: 6,
            params: vec![ParamMeta {
                name: "handler".into(),
                typ: handler_type,
                direction: crate::meta::ParamDirection::In,
            }],
            is_event_add: true,
            ..Default::default()
        };
        let remove = MethodMeta {
            name: "remove_Changed".into(),
            raw_name: "remove_Changed".into(),
            vtable_index: 7,
            params: vec![ParamMeta {
                name: "token".into(),
                typ: TypeMeta::I64,
                direction: crate::meta::ParamDirection::In,
            }],
            is_event_remove: true,
            ..Default::default()
        };
        let siblings = vec![add.clone(), remove];
        let code = generate_method_body(
            "_IWidget",
            "self._obj",
            &add,
            &HashSet::from(["Widget".into()]),
            &HashSet::from(["TypedEventHandler_Widget_Object".into()]),
            None,
            Some(&siblings),
            true,
        );

        assert!(code.contains("def on_changed(self, callback:"));
        assert!(code.contains("_dynwinrt_create_delegate("));
        assert!(code.contains("return _IWidget.method(6).invoke("));
        assert!(code.contains("def subscribe_changed(self, callback:"));
        assert!(code.contains("if not _active[0]:"));
        assert!(code.contains("self.off_changed(_token)"));
        assert!(code.contains("except Exception:\n                _active[0] = True"));
        assert!(code.contains("def once_changed(self, callback:"));
        assert!(code.contains("if not _state[0]:"));
        assert!(code.contains("_state[0] = False"));
        assert!(code.contains("if not _state[0]:\n            _unsubscribe()"));
    }

    #[test]
    fn fixed_event_delegate_uses_generated_iid_and_parameter_types() {
        let add = MethodMeta {
            name: "add_Click".into(),
            raw_name: "add_Click".into(),
            vtable_index: 6,
            params: vec![ParamMeta {
                name: "handler".into(),
                typ: TypeMeta::Interface {
                    namespace: "Contoso".into(),
                    name: "RoutedEventHandler".into(),
                    iid: "11111111-1111-1111-1111-111111111111".into(),
                },
                direction: crate::meta::ParamDirection::In,
            }],
            is_event_add: true,
            ..Default::default()
        };
        let code = generate_method_body(
            "_IButton",
            "self._obj",
            &add,
            &HashSet::new(),
            &HashSet::from(["RoutedEventHandler".into()]),
            None,
            Some(std::slice::from_ref(&add)),
            true,
        );

        assert!(code.contains("'routed_event_handler', 'IID_RoutedEventHandler'"));
        assert!(code.contains("'routed_event_handler', 'RoutedEventHandler_PARAM_TYPES'"));
        assert!(!code.contains("DynWinRTType.object().iid()"));
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

    #[test]
    fn python_numeric_overload_instance_dispatch_is_declaration_order_independent() {
        let wide = overloaded_method("Read2", 7, TypeMeta::I32);
        let narrow = overloaded_method("Read", 6, TypeMeta::I8);

        let forward = generate_instance_method_group(
            &[instance_overload(&wide), instance_overload(&narrow)],
            &HashSet::new(),
            &HashSet::new(),
        );
        let reverse = generate_instance_method_group(
            &[instance_overload(&narrow), instance_overload(&wide)],
            &HashSet::new(),
            &HashSet::new(),
        );

        assert_eq!(forward, reverse);
        assert_contains_in_order(
            &forward,
            "-128 <= _bound[0] <= 127",
            "-2147483648 <= _bound[0] <= 2147483647",
        );
    }

    #[test]
    fn python_numeric_overload_dispatch_separates_bool_char16_ranges_and_float() {
        let float = overloaded_method("Pick6", 11, TypeMeta::F64);
        let unsigned = overloaded_method("Pick5", 10, TypeMeta::U8);
        let string = overloaded_method("Pick4", 9, TypeMeta::String);
        let char16 = overloaded_method("Pick3", 8, TypeMeta::Char16);
        let boolean = overloaded_method("Pick2", 7, TypeMeta::Bool);
        let signed = overloaded_method("Pick", 6, TypeMeta::I8);
        let overloads = vec![
            instance_overload(&float),
            instance_overload(&unsigned),
            instance_overload(&string),
            instance_overload(&char16),
            instance_overload(&boolean),
            instance_overload(&signed),
        ];

        let code = generate_instance_method_group(&overloads, &HashSet::new(), &HashSet::new());

        assert_contains_in_order(
            &code,
            "if _bound is not None and isinstance(_bound[0], bool):",
            "if _bound is not None and isinstance(_bound[0], int) and not isinstance(_bound[0], bool) and not isinstance(_bound[0], __import__('enum').Enum) and -128 <= _bound[0] <= 127:",
        );
        assert_contains_in_order(
            &code,
            "if _bound is not None and isinstance(_bound[0], str) and len(_bound[0]) == 1 and ord(_bound[0]) <= 65535:",
            "if _bound is not None and isinstance(_bound[0], str):",
        );
        assert_contains_in_order(
            &code,
            "if _bound is not None and isinstance(_bound[0], int) and not isinstance(_bound[0], bool) and not isinstance(_bound[0], __import__('enum').Enum) and -128 <= _bound[0] <= 127:",
            "if _bound is not None and isinstance(_bound[0], int) and not isinstance(_bound[0], bool) and not isinstance(_bound[0], __import__('enum').Enum) and 0 <= _bound[0] <= 255:",
        );
        assert_contains_in_order(
            &code,
            "if _bound is not None and isinstance(_bound[0], int) and not isinstance(_bound[0], bool) and not isinstance(_bound[0], __import__('enum').Enum) and 0 <= _bound[0] <= 255:",
            "if _bound is not None and isinstance(_bound[0], (int, float)) and not isinstance(_bound[0], bool) and not isinstance(_bound[0], __import__('enum').Enum):",
        );
        assert!(code.contains("raise TypeError(\"No matching overload for pick\")"));
        assert!(
            !code.contains(
                "if _bound is not None and isinstance(_bound[0], int) and not isinstance(_bound[0], bool) and not isinstance(_bound[0], __import__('enum').Enum):"
            ),
            "{code}"
        );
    }

    #[test]
    fn python_numeric_overload_static_dispatch_is_declaration_order_independent() {
        let class = ClassMeta {
            name: "Factory".into(),
            ..Default::default()
        };
        let iface = InterfaceMeta {
            name: "IFactoryStatics".into(),
            ..Default::default()
        };
        let integer = overloaded_method("Create", 6, TypeMeta::I16);
        let float = overloaded_method("Create2", 7, TypeMeta::F64);

        let forward = generate_static_method_group(
            &[
                static_overload(&class, &iface, &float),
                static_overload(&class, &iface, &integer),
            ],
            &HashSet::new(),
            &HashSet::new(),
        );
        let reverse = generate_static_method_group(
            &[
                static_overload(&class, &iface, &integer),
                static_overload(&class, &iface, &float),
            ],
            &HashSet::new(),
            &HashSet::new(),
        );

        assert_eq!(forward, reverse);
        assert_contains_in_order(
            &forward,
            "-32768 <= _bound[0] <= 32767",
            "isinstance(_bound[0], (int, float)) and not isinstance(_bound[0], bool) and not isinstance(_bound[0], __import__('enum').Enum)",
        );
    }

    #[test]
    fn python_known_int_enum_instance_overload_prefers_enum_over_i8_in_both_orders() {
        let integer = overloaded_method("Read", 6, TypeMeta::I8);
        let enumeration = overloaded_method("Read2", 7, enum_type("Mode", false));
        let known_types = HashSet::from(["Mode".to_string()]);

        let forward = generate_instance_method_group(
            &[instance_overload(&integer), instance_overload(&enumeration)],
            &known_types,
            &HashSet::new(),
        );
        let reverse = generate_instance_method_group(
            &[instance_overload(&enumeration), instance_overload(&integer)],
            &known_types,
            &HashSet::new(),
        );

        let forward_dispatcher =
            extract_generated_block(&forward, "    def read(self, *args, **kwargs):\n");
        let reverse_dispatcher =
            extract_generated_block(&reverse, "    def read(self, *args, **kwargs):\n");
        let script = format!(
            r#"from enum import IntEnum
import json

class DynWinRTValue:
    pass

def _dynwinrt_bind_overload(parameter_names, args, kwargs):
    if kwargs:
        if args:
            return None
        if len(kwargs) != len(parameter_names) or any(name not in kwargs for name in parameter_names):
            return None
        return tuple(kwargs[name] for name in parameter_names)
    return args if len(args) == len(parameter_names) else None

def _dynwinrt_symbol(module, name):
    return globals()[name]

class Mode(IntEnum):
    VALUE = 1

class OtherMode(IntEnum):
    VALUE = 1

class ReaderForward:
    def _read_6(self, value):
        return "i8"

    def _read_7(self, value):
        return "enum"

{forward_dispatcher}

class ReaderReverse:
    def _read_6(self, value):
        return "i8"

    def _read_7(self, value):
        return "enum"

{reverse_dispatcher}

def exercise(reader_type):
    reader = reader_type()
    results = [reader.read(Mode.VALUE), reader.read(7)]
    try:
        reader.read(OtherMode.VALUE)
    except TypeError as error:
        results.append(type(error).__name__)
    else:
        results.append("unexpected")
    return results

print(json.dumps([exercise(ReaderForward), exercise(ReaderReverse)]))
"#
        );

        assert_eq!(
            run_python(&script),
            r#"[["enum", "i8", "TypeError"], ["enum", "i8", "TypeError"]]"#
        );
    }

    #[test]
    fn python_known_int_flag_static_overload_prefers_enum_over_i32_in_both_orders() {
        let integer = overloaded_method("Create", 6, TypeMeta::I32);
        let flags = overloaded_method("Create2", 7, enum_type("Options", true));
        let known_types = HashSet::from(["Options".to_string()]);
        let iface = InterfaceMeta {
            name: "IFactoryStatics".into(),
            ..Default::default()
        };
        let class_forward = ClassMeta {
            name: "FactoryForward".into(),
            ..Default::default()
        };
        let class_reverse = ClassMeta {
            name: "FactoryReverse".into(),
            ..Default::default()
        };

        let forward = generate_static_method_group(
            &[
                static_overload(&class_forward, &iface, &integer),
                static_overload(&class_forward, &iface, &flags),
            ],
            &known_types,
            &HashSet::new(),
        );
        let reverse = generate_static_method_group(
            &[
                static_overload(&class_reverse, &iface, &flags),
                static_overload(&class_reverse, &iface, &integer),
            ],
            &known_types,
            &HashSet::new(),
        );

        let forward_dispatcher = extract_generated_block(
            &forward,
            "    @staticmethod\n    def create(*args, **kwargs):\n",
        );
        let reverse_dispatcher = extract_generated_block(
            &reverse,
            "    @staticmethod\n    def create(*args, **kwargs):\n",
        );
        let script = format!(
            r#"from enum import IntFlag
import json

class DynWinRTValue:
    pass

def _dynwinrt_bind_overload(parameter_names, args, kwargs):
    if kwargs:
        if args:
            return None
        if len(kwargs) != len(parameter_names) or any(name not in kwargs for name in parameter_names):
            return None
        return tuple(kwargs[name] for name in parameter_names)
    return args if len(args) == len(parameter_names) else None

def _dynwinrt_symbol(module, name):
    return globals()[name]

class Options(IntFlag):
    A = 1
    B = 2

class OtherOptions(IntFlag):
    A = 1

class FactoryForward:
    @staticmethod
    def _create_6(value):
        return "i32"

    @staticmethod
    def _create_7(value):
        return "enum"

{forward_dispatcher}

class FactoryReverse:
    @staticmethod
    def _create_6(value):
        return "i32"

    @staticmethod
    def _create_7(value):
        return "enum"

{reverse_dispatcher}

def exercise(factory_type):
    results = [factory_type.create(Options.A | Options.B), factory_type.create(42)]
    try:
        factory_type.create(OtherOptions.A)
    except TypeError as error:
        results.append(type(error).__name__)
    else:
        results.append("unexpected")
    return results

print(json.dumps([exercise(FactoryForward), exercise(FactoryReverse)]))
"#
        );

        assert_eq!(
            run_python(&script),
            r#"[["enum", "i32", "TypeError"], ["enum", "i32", "TypeError"]]"#
        );
    }
}
