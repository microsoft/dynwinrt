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
    method_pydoc, py_delegate_callable_type, py_factory_return_type, py_method_abi_output_count,
    py_method_outputs, py_method_return_type, py_param_list, py_return_type_safe,
};

fn is_delegate_type(typ: &TypeMeta, delegate_type_names: &HashSet<String>) -> bool {
    delegate_name(typ, delegate_type_names).is_some()
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
        return format!("(lambda value: None if value.is_null() else value)({expr})");
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
    iface: &InterfaceMeta,
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
        Some(&iface.methods),
    )
}

pub(crate) struct InstanceOverload<'a> {
    pub(crate) iface_var: String,
    pub(crate) obj_expr: String,
    pub(crate) method: &'a MethodMeta,
    pub(crate) sibling_methods: Option<&'a [MethodMeta]>,
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
            overload.sibling_methods,
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
    sibling_methods: Option<&[MethodMeta]>,
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
        let delegate_name = delegate_typ.and_then(|typ| match typ {
            TypeMeta::Parameterized { name, args, .. } => {
                Some(crate::meta::make_parameterized_name(name, args))
            }
            TypeMeta::Delegate { name, .. } => Some(name.clone()),
            _ => None,
        });
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
                "        _handler = DynWinRtDelegate.create({}, {}, _wrapped)\n",
                iid, param_types
            ));
        } else {
            out.push_str(
                "        _handler = DynWinRtDelegate.create(DynWinRTType.object().iid(), [DynWinRTType.object(), DynWinRTType.object()], _wrapped)\n"
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
        let py_return = if return_type.is_some_and(|typ| is_delegate_type(typ, delegate_type_names))
        {
            "DynWinRTValue | None".to_string()
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
        );

        assert!(code.contains("def load_async(self) -> WinRTAsync[int]:"));
        assert!(code.contains("return _DynWinRTAsync("));
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
        );

        assert!(code.contains(
            "def write_async(self, buffer: 'DynWinRTValue') -> WinRTAsyncWithProgress[int, int]:"
        ));
        assert!(code.contains("return _DynWinRTAsyncWithProgress("));
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
            },
            InstanceOverload {
                iface_var: "_IReader".into(),
                obj_expr: "self._obj".into(),
                method: &second,
                sibling_methods: None,
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
                sibling_methods: None,
            },
            InstanceOverload {
                iface_var: "_IRunner".into(),
                obj_expr: "self._obj".into(),
                method: &text,
                sibling_methods: None,
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
        );

        assert!(code.contains("def on_changed(self, callback:"));
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
