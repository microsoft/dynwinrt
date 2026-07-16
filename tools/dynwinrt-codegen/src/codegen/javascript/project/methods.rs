// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Runtime class method projection.

use super::*;

// ======================================================================
// Method projection helpers
// ======================================================================

pub(super) fn project_factory_method(
    class: &ClassMeta,
    iface: &InterfaceMeta,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_names: &HashSet<String>,
    delegate_sigs: &HashMap<String, String>,
    delegate_param_wraps: &HashMap<String, Vec<String>>,
) -> ProjectedMember {
    let in_params = get_in_params(method);
    let params = project_params(
        &in_params,
        known_types,
        delegate_names,
        delegate_sigs,
        delegate_param_wraps,
    );
    let args_expr = build_args_expr(&in_params);
    let out_count = method
        .params
        .iter()
        .filter(|p| p.direction == ParamDirection::Out)
        .count()
        + usize::from(method.return_type.is_some());

    let is_async = method.return_type.as_ref().is_some_and(|rt| rt.is_async());

    let mut invoke_expr = if out_count > 1 {
        format!(
            "_{iface}.method({idx}).invokeAll({cls}.f_{iface}(), [{args}])[{result_index}]",
            iface = iface.name,
            idx = method.vtable_index,
            cls = class.name,
            args = args_expr,
            result_index = out_count - 1,
        )
    } else {
        format!(
            "_{iface}.method({idx}).invoke({cls}.f_{iface}(), [{args}])",
            iface = iface.name,
            idx = method.vtable_index,
            cls = class.name,
            args = args_expr
        )
    };
    invoke_expr = rewrite_delegate_args_in_expr(&invoke_expr, &params);

    let return_type;
    let async_kind;
    let sync_return_expr;
    let async_convert_v;

    if is_async {
        return_type = format!("Promise<{}>", class.name);
        async_kind = AsyncKind::Operation(class.name.clone());
        sync_return_expr = None;
        async_convert_v = Some(format!("new {}(_v)", class.name));
    } else {
        return_type = class.name.clone();
        async_kind = AsyncKind::None;
        sync_return_expr = Some(format!("new {}({})", class.name, invoke_expr));
        async_convert_v = None;
    }

    let mut ts_params = params;
    if is_async {
        ts_params.push(ProjectedParam {
            name: "signal".into(),
            ts_type: "AbortSignal".into(),
            optional: true,
            delegate_wrap: None,
        });
    }

    let mut doc = build_method_doc(method, &in_params);
    if is_async {
        if let Some(ref mut d) = doc {
            d.params.push((
                "signal".into(),
                "Abort signal to cancel the underlying WinRT async operation.".into(),
            ));
        }
    }
    let delegate_wraps = collect_delegate_wraps(&ts_params);

    let js_name = to_camel_case(&method.name);
    let raw_js_name = to_camel_case(&method.raw_name);
    let overload_of = if js_name != raw_js_name {
        Some(raw_js_name)
    } else {
        None
    };

    ProjectedMember::Method(ProjectedMethod {
        name: js_name,
        doc,
        params: ts_params,
        return_type,
        async_kind,
        is_static: true,
        invoke_expr,
        sync_return_expr,
        async_convert_v,
        progress_convert: None,
        is_void: false,
        array_return_expr: None,
        delegate_wraps,
        js_only: false,
        overload_of,
    })
}

pub(super) fn project_static_method(
    class: &ClassMeta,
    iface: &InterfaceMeta,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_names: &HashSet<String>,
    delegate_sigs: &HashMap<String, String>,
    delegate_param_wraps: &HashMap<String, Vec<String>>,
) -> ProjectedMember {
    let in_params = get_in_params(method);
    let return_type_meta = method.return_type.as_ref();
    let is_with_progress = return_type_meta.is_some_and(|rt| {
        matches!(
            rt,
            TypeMeta::AsyncOperationWithProgress(_, _) | TypeMeta::AsyncActionWithProgress(_)
        )
    });
    let is_async = return_type_meta.is_some_and(|rt| rt.is_async()) && !is_with_progress;

    let statics_call = format!("{cls}.s_{iface}()", cls = class.name, iface = iface.name);

    // Static property getter
    if method.is_property_getter && in_params.is_empty() {
        let prop_name = to_camel_case(method.name.strip_prefix("get_").unwrap_or(&method.name));
        let ts_return = ts_return_type_safe(return_type_meta, false, known_types);
        let invoke_expr = format!(
            "_{}.method({}).invoke({}, [])",
            iface.name, method.vtable_index, statics_call
        );
        let converted = convert_return(
            &invoke_expr,
            return_type_meta,
            false,
            known_types,
            &NO_DEFERRED,
        );
        let doc = build_method_doc(method, &in_params);
        return ProjectedMember::Property(ProjectedProperty {
            name: prop_name,
            ts_type: ts_return,
            readonly: true,
            is_static: true,
            doc,
            getter_expr: converted,
            setter_line: None,
        });
    }

    let ts_return = ts_return_type_safe(return_type_meta, is_async, known_types);
    let params = project_params(
        &in_params,
        known_types,
        delegate_names,
        delegate_sigs,
        delegate_param_wraps,
    );
    let args_expr = build_args_expr(&in_params);
    let mut invoke_expr = format!(
        "_{}.method({}).invoke({}, [{}])",
        iface.name, method.vtable_index, statics_call, args_expr
    );
    invoke_expr = rewrite_delegate_args_in_expr(&invoke_expr, &params);

    let async_kind;
    let sync_return_expr;
    let async_convert_v;
    let mut progress_convert = None;

    if is_with_progress {
        let inner_type = match return_type_meta {
            Some(TypeMeta::AsyncOperationWithProgress(inner, _)) => Some(inner.as_ref()),
            _ => None,
        };
        let progress_type = match return_type_meta {
            Some(TypeMeta::AsyncOperationWithProgress(_, p)) => Some(p.as_ref()),
            Some(TypeMeta::AsyncActionWithProgress(p)) => Some(p.as_ref()),
            _ => None,
        };
        let progress_ts = progress_type
            .map(|p| ts_return_type_safe(Some(p), false, known_types))
            .unwrap_or_else(|| "unknown".to_string());
        // Build conversion expression for progress value
        let p_convert = convert_return("_p", progress_type, false, known_types, &NO_DEFERRED);
        if p_convert != "_p" {
            progress_convert = Some(p_convert);
        }
        let is_action = matches!(return_type_meta, Some(TypeMeta::AsyncActionWithProgress(_)));
        let inner_convert = convert_return("_v", inner_type, false, known_types, &NO_DEFERRED);
        if is_action {
            async_kind = AsyncKind::ActionWithProgress(progress_ts);
        } else {
            let inner_ts = ts_return_type_safe(inner_type, false, known_types);
            async_kind = AsyncKind::OperationWithProgress(inner_ts, progress_ts);
        }
        sync_return_expr = None;
        async_convert_v = Some(inner_convert);
    } else if is_async {
        let inner_type = async_inner_type(return_type_meta);
        let is_action = matches!(return_type_meta, Some(TypeMeta::AsyncAction));
        if is_action {
            async_kind = AsyncKind::Action;
            async_convert_v = None;
        } else {
            let convert_v = convert_return("_v", inner_type, false, known_types, &NO_DEFERRED);
            let inner_ts = ts_return_type_safe(inner_type, false, known_types);
            async_kind = AsyncKind::Operation(inner_ts);
            async_convert_v = Some(convert_v);
        }
        sync_return_expr = None;
    } else {
        async_kind = AsyncKind::None;
        let converted = convert_return(
            &invoke_expr,
            return_type_meta,
            false,
            known_types,
            &NO_DEFERRED,
        );
        sync_return_expr = if return_type_meta.is_some() {
            Some(converted)
        } else {
            None
        };
        async_convert_v = None;
    }

    let mut ts_params = params;
    if is_async || is_with_progress {
        ts_params.push(ProjectedParam {
            name: "signal".into(),
            ts_type: "AbortSignal".into(),
            optional: true,
            delegate_wrap: None,
        });
    }

    let mut doc = build_method_doc(method, &in_params);
    if is_async || is_with_progress {
        if let Some(ref mut d) = doc {
            d.params.push((
                "signal".into(),
                "Abort signal to cancel the underlying WinRT async operation.".into(),
            ));
        }
    }
    let delegate_wraps = collect_delegate_wraps(&ts_params);

    let js_name = to_camel_case(&method.name);
    let raw_js_name = to_camel_case(&method.raw_name);
    let overload_of = if js_name != raw_js_name {
        Some(raw_js_name)
    } else {
        None
    };

    ProjectedMember::Method(ProjectedMethod {
        name: js_name,
        doc,
        params: ts_params,
        return_type: ts_return,
        async_kind,
        is_static: true,
        invoke_expr,
        sync_return_expr,
        async_convert_v,
        progress_convert,
        is_void: return_type_meta.is_none() && !is_async,
        array_return_expr: None,
        delegate_wraps,
        js_only: false,
        overload_of,
    })
}

pub(super) fn project_instance_method(
    iface_var: &str,
    obj_expr: &str,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
    iface_methods: Option<&[MethodMeta]>,
    delegate_sigs: &HashMap<String, String>,
    delegate_param_wraps: &HashMap<String, Vec<String>>,
) -> Option<ProjectedMember> {
    let in_params = get_in_params(method);
    let return_type_meta = method.return_type.as_ref();
    let is_with_progress = return_type_meta.is_some_and(|rt| {
        matches!(
            rt,
            TypeMeta::AsyncOperationWithProgress(_, _) | TypeMeta::AsyncActionWithProgress(_)
        )
    });
    let is_async = return_type_meta.is_some_and(|rt| rt.is_async()) && !is_with_progress;
    let has_array_out = method.params.iter().any(|p| {
        (p.direction == ParamDirection::Out || p.direction == ParamDirection::OutFill)
            && matches!(p.typ, TypeMeta::Array(_))
    });
    let has_return = return_type_meta.is_some() || has_array_out;

    let is_delegate_type = |typ: Option<&TypeMeta>| -> bool {
        match typ {
            Some(TypeMeta::Delegate { .. }) => true,
            Some(TypeMeta::Interface { name, .. }) => delegate_type_names.contains(name),
            _ => false,
        }
    };

    let mut doc = build_method_doc(method, &in_params);

    // Event add
    if method.is_event_add {
        return Some(project_event_add(
            iface_var,
            obj_expr,
            method,
            known_types,
            iface_methods,
            doc,
        ));
    }

    // Event remove
    if method.is_event_remove {
        let event_name = to_camel_case(method.name.strip_prefix("remove_").unwrap_or(&method.name));
        return Some(ProjectedMember::Event(ProjectedEvent {
            subscribe_name: String::new(),
            unsubscribe_name: format!("off{}", capitalize(&event_name)),
            callback_type: String::new(),
            doc,
            delegate_name: None,
            add_iface_var: String::new(),
            add_vtable_index: 0,
            add_obj_expr: String::new(),
            remove_vtable_index: Some(method.vtable_index),
            remove_iface_var: iface_var.into(),
            remove_obj_expr: obj_expr.into(),
            needs_wrap: false,
            sender_wrap: None,
            args_wrap: None,
        }));
    }

    // Property getter
    if method.is_property_getter && in_params.is_empty() {
        let prop_name = to_camel_case(method.name.strip_prefix("get_").unwrap_or(&method.name));
        let ts_return = if is_delegate_type(return_type_meta) {
            "DynWinRtValue".to_string()
        } else {
            ts_return_type_safe(return_type_meta, false, known_types)
        };
        let invoke_expr = format!(
            "{}.method({}).invoke({}, [])",
            iface_var, method.vtable_index, obj_expr
        );
        let converted = if is_delegate_type(return_type_meta) {
            invoke_expr.clone()
        } else {
            convert_return(
                &invoke_expr,
                return_type_meta,
                false,
                known_types,
                &NO_DEFERRED,
            )
        };

        // Check if there's a corresponding setter
        let setter_line = find_setter_for_property(method, iface_var, obj_expr, iface_methods);

        return Some(ProjectedMember::Property(ProjectedProperty {
            name: prop_name,
            ts_type: ts_return,
            readonly: setter_line.is_none(),
            is_static: false,
            doc,
            getter_expr: converted,
            setter_line,
        }));
    }

    // Property setter (standalone — if paired with getter, handled above)
    if method.is_property_setter {
        let prop_name = to_camel_case(method.name.strip_prefix("put_").unwrap_or(&method.name));
        let param_type = if in_params
            .first()
            .is_some_and(|p| is_delegate_type(Some(&p.typ)))
        {
            "DynWinRtValue".to_string()
        } else {
            in_params
                .first()
                .map(|p| ts_param_type_safe(&p.typ, known_types))
                .unwrap_or_else(|| "any".to_string())
        };
        let arg = in_params
            .first()
            .map(|p| wrap_arg("value", &p.typ))
            .unwrap_or_else(|| "value".to_string());
        let setter_line = format!(
            "{}.method({}).invoke({}, [{}]);",
            iface_var, method.vtable_index, obj_expr, arg
        );

        // Check if there's a corresponding getter (if so, it will add the property)
        let getter_name = format!(
            "get_{}",
            method.name.strip_prefix("put_").unwrap_or(&method.name)
        );
        let has_getter = iface_methods.map_or(false, |methods| {
            methods
                .iter()
                .any(|m| m.name == getter_name && m.is_property_getter)
        });
        if has_getter {
            // The getter will create the property with this setter included — skip
            return None;
        }

        return Some(ProjectedMember::Property(ProjectedProperty {
            name: prop_name,
            ts_type: param_type,
            readonly: false,
            is_static: false,
            doc,
            getter_expr: String::new(),
            setter_line: Some(setter_line),
        }));
    }

    // Normal method
    let params = project_params(
        &in_params,
        known_types,
        delegate_type_names,
        delegate_sigs,
        delegate_param_wraps,
    );
    let array_out_elem = if has_array_out && return_type_meta.is_none() {
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

    let ts_return = if let Some(elem) = array_out_elem {
        ts_array_element_type(elem, known_types)
    } else {
        ts_return_type_safe(return_type_meta, is_async, known_types)
    };

    let args_expr = build_args_expr(&in_params);
    let mut invoke_expr = format!(
        "{}.method({}).invoke({}, [{}])",
        iface_var, method.vtable_index, obj_expr, args_expr
    );
    invoke_expr = rewrite_delegate_args_in_expr(&invoke_expr, &params);

    let async_kind;
    let sync_return_expr;
    let async_convert_v;
    let array_return_expr;
    let mut progress_convert = None;

    if is_with_progress {
        let inner_type = match return_type_meta {
            Some(TypeMeta::AsyncOperationWithProgress(inner, _)) => Some(inner.as_ref()),
            _ => None,
        };
        let progress_type = match return_type_meta {
            Some(TypeMeta::AsyncOperationWithProgress(_, p)) => Some(p.as_ref()),
            Some(TypeMeta::AsyncActionWithProgress(p)) => Some(p.as_ref()),
            _ => None,
        };
        let progress_ts = progress_type
            .map(|p| ts_return_type_safe(Some(p), false, known_types))
            .unwrap_or_else(|| "unknown".to_string());
        let p_convert = convert_return("_p", progress_type, false, known_types, &NO_DEFERRED);
        if p_convert != "_p" {
            progress_convert = Some(p_convert);
        }
        let is_action = matches!(return_type_meta, Some(TypeMeta::AsyncActionWithProgress(_)));
        let inner_convert = convert_return("_v", inner_type, false, known_types, &NO_DEFERRED);
        if is_action {
            async_kind = AsyncKind::ActionWithProgress(progress_ts);
        } else {
            let inner_ts = ts_return_type_safe(inner_type, false, known_types);
            async_kind = AsyncKind::OperationWithProgress(inner_ts, progress_ts);
        }
        sync_return_expr = None;
        async_convert_v = Some(inner_convert);
        array_return_expr = None;
    } else if is_async {
        let inner_type = async_inner_type(return_type_meta);
        let is_action = matches!(return_type_meta, Some(TypeMeta::AsyncAction));
        if is_action {
            async_kind = AsyncKind::Action;
            async_convert_v = None;
        } else {
            let convert_v = convert_return("_v", inner_type, false, known_types, &NO_DEFERRED);
            let inner_ts = ts_return_type_safe(inner_type, false, known_types);
            async_kind = AsyncKind::Operation(inner_ts);
            async_convert_v = Some(convert_v);
        }
        sync_return_expr = None;
        array_return_expr = None;
    } else if let Some(elem) = array_out_elem {
        async_kind = AsyncKind::None;
        let arr_expr = format!("{}.asArray()", invoke_expr);
        let converted = convert_array_return(&arr_expr, elem, known_types, &NO_DEFERRED);
        sync_return_expr = None;
        async_convert_v = None;
        array_return_expr = Some(converted);
    } else {
        async_kind = AsyncKind::None;
        if has_return {
            let converted = convert_return(
                &invoke_expr,
                return_type_meta,
                false,
                known_types,
                &NO_DEFERRED,
            );
            sync_return_expr = Some(converted);
        } else {
            sync_return_expr = None;
        }
        async_convert_v = None;
        array_return_expr = None;
    }

    let is_void = !has_return && !is_async;

    let mut ts_params = params;
    if is_async || is_with_progress {
        ts_params.push(ProjectedParam {
            name: "signal".into(),
            ts_type: "AbortSignal".into(),
            optional: true,
            delegate_wrap: None,
        });
        // Add @param signal doc
        if let Some(ref mut d) = doc {
            d.params.push((
                "signal".into(),
                "Abort signal to cancel the underlying WinRT async operation.".into(),
            ));
        }
    }

    let delegate_wraps = collect_delegate_wraps(&ts_params);

    let js_name = to_camel_case(&method.name);
    let raw_js_name = to_camel_case(&method.raw_name);
    let overload_of = if js_name != raw_js_name {
        Some(raw_js_name)
    } else {
        None
    };

    Some(ProjectedMember::Method(ProjectedMethod {
        name: js_name,
        doc,
        params: ts_params,
        return_type: ts_return,
        async_kind,
        is_static: false,
        invoke_expr,
        sync_return_expr,
        async_convert_v,
        progress_convert,
        is_void,
        array_return_expr,
        delegate_wraps,
        js_only: false,
        overload_of,
    }))
}

fn project_event_add(
    iface_var: &str,
    obj_expr: &str,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    iface_methods: Option<&[MethodMeta]>,
    doc: Option<DocInfo>,
) -> ProjectedMember {
    let in_params = get_in_params(method);
    let event_name = to_camel_case(method.name.strip_prefix("add_").unwrap_or(&method.name));
    let cap = capitalize(&event_name);
    let delegate_first_param = in_params.first().map(|p| &p.typ);
    let delegate_name = delegate_first_param.and_then(|t| match t {
        TypeMeta::Parameterized { name, args, .. } => {
            Some(crate::meta::make_parameterized_name(name, args))
        }
        TypeMeta::Delegate { name, .. } => Some(name.clone()),
        TypeMeta::Interface { name, .. } => Some(name.clone()),
        _ => None,
    });
    let suffix = method.name.strip_prefix("add_").unwrap_or(&method.name);
    let remove_idx = iface_methods.and_then(|methods| {
        let target = format!("remove_{}", suffix);
        methods
            .iter()
            .find(|m| m.name == target)
            .map(|m| m.vtable_index)
    });

    let (callback_ts, sender_wrap, args_wrap) = match delegate_first_param {
        Some(TypeMeta::Parameterized { name, args, .. })
            if name.split('`').next() == Some("TypedEventHandler") && args.len() == 2 =>
        {
            let s_ts = ts_return_type_safe(Some(&args[0]), false, known_types);
            let a_ts = ts_return_type_safe(Some(&args[1]), false, known_types);
            // Map DynWinRtValue → unknown for event callback params (more TS-idiomatic)
            let s_ts_pub = if s_ts == "DynWinRtValue" {
                "unknown".to_string()
            } else {
                s_ts
            };
            let a_ts_pub = if a_ts == "DynWinRtValue" {
                "unknown".to_string()
            } else {
                a_ts
            };
            let s_wrap = convert_return("__a0__", Some(&args[0]), false, known_types, &NO_DEFERRED);
            let a_wrap = convert_return("__a1__", Some(&args[1]), false, known_types, &NO_DEFERRED);
            (
                format!("(sender: {}, args: {}) => void", s_ts_pub, a_ts_pub),
                Some(s_wrap),
                Some(a_wrap),
            )
        }
        Some(TypeMeta::Parameterized { name, args, .. })
            if name.split('`').next() == Some("EventHandler") && args.len() == 1 =>
        {
            let a_ts = ts_return_type_safe(Some(&args[0]), false, known_types);
            let a_ts_pub = if a_ts == "DynWinRtValue" {
                "unknown".to_string()
            } else {
                a_ts
            };
            let a_wrap = convert_return("__a1__", Some(&args[0]), false, known_types, &NO_DEFERRED);
            (
                format!("(sender: unknown, args: {}) => void", a_ts_pub),
                None,
                Some(a_wrap),
            )
        }
        _ => ("(...args: unknown[]) => void".to_string(), None, None),
    };

    let needs_wrap = sender_wrap.as_deref().is_some_and(|s| s != "__a0__")
        || args_wrap.as_deref().is_some_and(|s| s != "__a1__");

    ProjectedMember::Event(ProjectedEvent {
        subscribe_name: format!("on{}", cap),
        unsubscribe_name: format!("off{}", cap),
        callback_type: callback_ts,
        doc,
        delegate_name,
        add_iface_var: iface_var.into(),
        add_vtable_index: method.vtable_index,
        add_obj_expr: obj_expr.into(),
        remove_vtable_index: remove_idx,
        remove_iface_var: iface_var.into(),
        remove_obj_expr: obj_expr.into(),
        needs_wrap,
        sender_wrap,
        args_wrap,
    })
}

fn find_setter_for_property(
    getter: &MethodMeta,
    iface_var: &str,
    obj_expr: &str,
    iface_methods: Option<&[MethodMeta]>,
) -> Option<String> {
    let prop_suffix = getter.name.strip_prefix("get_")?;
    let setter_name = format!("put_{}", prop_suffix);
    let methods = iface_methods?;
    let setter = methods
        .iter()
        .find(|m| m.name == setter_name && m.is_property_setter)?;
    let setter_in_params = get_in_params(setter);
    let arg = setter_in_params
        .first()
        .map(|p| wrap_arg("value", &p.typ))
        .unwrap_or_else(|| "value".to_string());
    Some(format!(
        "{}.method({}).invoke({}, [{}]);",
        iface_var, setter.vtable_index, obj_expr, arg
    ))
}
