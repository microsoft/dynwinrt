// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Runtime class method projection.

use super::*;
use crate::codegen::winrt::javascript::JavaScriptProjectionContext;

fn projected_method_outputs(method: &MethodMeta) -> Vec<(usize, &TypeMeta)> {
    let mut result_index = 0;
    let mut outputs = Vec::new();
    for param in &method.params {
        match param.direction {
            ParamDirection::Out | ParamDirection::OutFill => {
                outputs.push((result_index, &param.typ));
                result_index += 1;
            }
            ParamDirection::In => {}
        }
    }
    if !fill_array_uses_retval_count(method)
        && let Some(return_type) = method.return_type.as_ref()
    {
        outputs.push((result_index, return_type));
    }
    outputs
}

fn is_projected_delegate_type(
    context: &JavaScriptProjectionContext,
    typ: Option<&TypeMeta>,
    delegate_names: &HashSet<String>,
) -> bool {
    match typ {
        Some(TypeMeta::Delegate { .. }) => true,
        Some(TypeMeta::Interface { name, .. }) => delegate_names.contains(name),
        Some(TypeMeta::Parameterized {
            namespace,
            name,
            piid,
            args,
        }) => delegate_names
            .contains(&context.projected_parameterized_name(namespace, name, piid, args)),
        _ => false,
    }
}

fn projected_ts_return_type(
    context: &JavaScriptProjectionContext,
    typ: Option<&TypeMeta>,
    is_async: bool,
    known_types: &HashSet<String>,
    delegate_names: &HashSet<String>,
) -> String {
    match typ {
        Some(TypeMeta::AsyncOperation(inner)) => format!(
            "Promise<{}>",
            projected_ts_return_type(context, Some(inner), false, known_types, delegate_names)
        ),
        Some(TypeMeta::AsyncOperationWithProgress(result, _)) => {
            let inner =
                projected_ts_return_type(context, Some(result), false, known_types, delegate_names);
            format!(
                "Promise<{i}> & {{ progress(cb: (value: unknown) => void): Promise<{i}> & {{ progress: any; toPromise(): Promise<{i}>; cancel(): void; }}; toPromise(): Promise<{i}>; cancel(): void; }}",
                i = inner
            )
        }
        Some(TypeMeta::Array(inner))
            if is_projected_delegate_type(context, Some(inner), delegate_names) =>
        {
            "Array<DynWinRtValue | null>".into()
        }
        _ if is_projected_delegate_type(context, typ, delegate_names) => {
            if is_async {
                "Promise<DynWinRtValue | null>".into()
            } else {
                "DynWinRtValue | null".into()
            }
        }
        _ => ts_return_type_safe(context, typ, is_async, known_types),
    }
}

fn convert_projected_return(
    context: &JavaScriptProjectionContext,
    expr: &str,
    typ: Option<&TypeMeta>,
    known_types: &HashSet<String>,
    delegate_names: &HashSet<String>,
) -> String {
    match typ {
        Some(TypeMeta::Array(inner))
            if is_projected_delegate_type(context, Some(inner), delegate_names) =>
        {
            format!(
                "{}.asArray().toValues().map(v => v.isNull() ? null : v)",
                expr
            )
        }
        _ if is_projected_delegate_type(context, typ, delegate_names) => {
            format!("((v) => v.isNull() ? null : v)({})", expr)
        }
        _ => convert_return(context, expr, typ, false, known_types, &NO_DEFERRED),
    }
}

fn fast_getter_expression(
    context: &JavaScriptProjectionContext,
    iface_var: &str,
    vtable_index: usize,
    obj_expr: &str,
    typ: Option<&TypeMeta>,
    invoke_expr: &str,
    known_types: &HashSet<String>,
    delegate_names: &HashSet<String>,
) -> Option<String> {
    let method = match typ {
        Some(TypeMeta::String) => "getString",
        Some(TypeMeta::Bool) => "getBool",
        Some(TypeMeta::I32 | TypeMeta::Enum { .. }) => "getI32",
        Some(
            TypeMeta::Object
            | TypeMeta::Interface { .. }
            | TypeMeta::RuntimeClass { .. }
            | TypeMeta::Delegate { .. }
            | TypeMeta::Parameterized { .. },
        ) => "getObj",
        _ => return None,
    };
    let fallback = convert_projected_return(context, invoke_expr, typ, known_types, delegate_names);
    if method != "getObj" {
        return Some(format!(
            "(() => {{ const _m = {iface}.method({index}); return typeof _m.{method} === 'function' ? _m.{method}({obj}) : {fallback}; }})()",
            iface = iface_var,
            index = vtable_index,
            obj = obj_expr,
        ));
    }

    let fast_value = format!(
        "(() => {{ const _m = {iface}.method({index}); return typeof _m.getObj === 'function' ? _m.getObj({obj}) : _m.invoke({obj}, []); }})()",
        iface = iface_var,
        index = vtable_index,
        obj = obj_expr,
    );
    Some(convert_projected_return(
        context,
        &fast_value,
        typ,
        known_types,
        delegate_names,
    ))
}

fn setter_line(
    context: &JavaScriptProjectionContext,
    iface_var: &str,
    vtable_index: usize,
    obj_expr: &str,
    typ: Option<&TypeMeta>,
) -> String {
    let wrapped = typ
        .map(|typ| wrap_arg(context, "value", typ))
        .unwrap_or_else(|| "value".into());
    let fallback = format!("_m.invoke({}, [{}]);", obj_expr, wrapped);
    let Some(method) = (match typ {
        Some(TypeMeta::String) => Some("setHstring"),
        Some(TypeMeta::Bool) => Some("setBool"),
        Some(TypeMeta::I32 | TypeMeta::Enum { .. }) => Some("setI32"),
        Some(TypeMeta::U32) => Some("setU32"),
        Some(TypeMeta::F32) => Some("setF32"),
        Some(TypeMeta::F64) => Some("setF64"),
        _ => None,
    }) else {
        return format!(
            "{}.method({}).invoke({}, [{}]);",
            iface_var, vtable_index, obj_expr, wrapped,
        );
    };
    format!(
        "{{ const _m = {iface}.method({index}); if (typeof _m.{method} === 'function') _m.{method}({obj}, value); else {fallback} }}",
        iface = iface_var,
        index = vtable_index,
        obj = obj_expr,
    )
}

fn output_ts_type(
    context: &JavaScriptProjectionContext,
    typ: &TypeMeta,
    known_types: &HashSet<String>,
    delegate_names: &HashSet<String>,
) -> String {
    match typ {
        TypeMeta::Array(inner)
            if is_projected_delegate_type(context, Some(inner), delegate_names) =>
        {
            "Array<DynWinRtValue | null>".into()
        }
        TypeMeta::Array(inner) => ts_array_element_type(inner, known_types),
        _ => projected_ts_return_type(context, Some(typ), false, known_types, delegate_names),
    }
}

fn convert_output(
    context: &JavaScriptProjectionContext,
    expr: &str,
    typ: &TypeMeta,
    known_types: &HashSet<String>,
    delegate_names: &HashSet<String>,
    deferred: &HashSet<String>,
) -> String {
    match typ {
        TypeMeta::Array(inner)
            if is_projected_delegate_type(context, Some(inner), delegate_names) =>
        {
            format!(
                "{}.asArray().toValues().map(v => v.isNull() ? null : v)",
                expr
            )
        }
        TypeMeta::Array(inner) => convert_array_return(
            context,
            &format!("{}.asArray()", expr),
            inner,
            known_types,
            deferred,
        ),
        _ if is_projected_delegate_type(context, Some(typ), delegate_names) => {
            format!("((v) => v.isNull() ? null : v)({})", expr)
        }
        _ => convert_return(context, expr, Some(typ), false, known_types, deferred),
    }
}

fn project_multi_output(
    context: &JavaScriptProjectionContext,
    method: &MethodMeta,
    invoke_expr: &str,
    known_types: &HashSet<String>,
    delegate_names: &HashSet<String>,
) -> (String, String) {
    let outputs = projected_method_outputs(method);
    let return_type = match outputs.as_slice() {
        [(_, typ)] => output_ts_type(context, typ, known_types, delegate_names),
        _ => format!(
            "[{}]",
            outputs
                .iter()
                .map(|(_, typ)| output_ts_type(context, typ, known_types, delegate_names))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };

    let converted = outputs
        .iter()
        .map(|(index, typ)| {
            let converted = convert_output(
                context,
                &format!("_r[{}]", index),
                typ,
                known_types,
                delegate_names,
                &NO_DEFERRED,
            );
            if fill_array_uses_retval_count(method)
                && fill_array_output_index(method) == Some(*index)
            {
                format!(
                    "{}.slice(0, _r[{}].toNumber())",
                    converted,
                    method_abi_output_count(method) - 1
                )
            } else {
                converted
            }
        })
        .collect::<Vec<_>>();
    let result = if converted.len() == 1 {
        converted[0].clone()
    } else {
        format!("[{}]", converted.join(", "))
    };
    (
        return_type,
        format!(
            "(() => {{ const _r = {}; return {}; }})()",
            invoke_expr, result
        ),
    )
}

// ======================================================================
// Method projection helpers
// ======================================================================

pub(super) fn project_factory_method(
    context: &JavaScriptProjectionContext,
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
        context,
        &in_params,
        known_types,
        delegate_names,
        delegate_sigs,
        delegate_param_wraps,
    );
    let mut argument_kinds = in_params
        .iter()
        .map(|param| js_argument_kind(&param.typ))
        .collect::<Vec<_>>();
    let args_expr = build_args_expr(context, &in_params);
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
        async_convert_v = Some(format!("{}._fromNative(_v)", class.name));
    } else {
        return_type = class.name.clone();
        async_kind = AsyncKind::None;
        sync_return_expr = Some(format!("{}._fromNative({})", class.name, invoke_expr));
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
        argument_kinds.push(None);
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
        argument_kinds,
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
    context: &JavaScriptProjectionContext,
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
    let is_multi_output = method_abi_output_count(method) > 1;
    let has_array_out = method.params.iter().any(|p| {
        (p.direction == ParamDirection::Out || p.direction == ParamDirection::OutFill)
            && matches!(p.typ, TypeMeta::Array(_))
    });
    let has_return = method_abi_output_count(method) > 0;
    let array_out_elem =
        if has_array_out && (return_type_meta.is_none() || fill_array_uses_retval_count(method)) {
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
    let single_out = if has_return && return_type_meta.is_none() && array_out_elem.is_none() {
        projected_method_outputs(method)
            .into_iter()
            .next()
            .map(|(_, typ)| typ)
    } else {
        None
    };

    let statics_call = format!("{cls}.s_{iface}()", cls = class.name, iface = iface.name);

    // Static property getter
    if method.is_property_getter && in_params.is_empty() {
        let prop_name = to_camel_case(method.name.strip_prefix("get_").unwrap_or(&method.name));
        let ts_return = projected_ts_return_type(
            context,
            return_type_meta,
            false,
            known_types,
            delegate_names,
        );
        let invoke_expr = format!(
            "_{}.method({}).invoke({}, [])",
            iface.name, method.vtable_index, statics_call
        );
        let converted = fast_getter_expression(
            context,
            &format!("_{}", iface.name),
            method.vtable_index,
            &statics_call,
            return_type_meta,
            &invoke_expr,
            known_types,
            delegate_names,
        )
        .unwrap_or_else(|| {
            convert_projected_return(
                context,
                &invoke_expr,
                return_type_meta,
                known_types,
                delegate_names,
            )
        });
        let doc = build_method_doc(method, &in_params);
        return ProjectedMember::Property(ProjectedProperty {
            name: prop_name,
            ts_type: ts_return,
            setter_ts_type: None,
            readonly: true,
            is_static: true,
            doc,
            getter_expr: converted,
            setter_line: None,
        });
    }

    let mut ts_return = if let Some(elem) = array_out_elem {
        if is_projected_delegate_type(context, Some(elem), delegate_names) {
            "Array<DynWinRtValue | null>".into()
        } else {
            ts_array_element_type(elem, known_types)
        }
    } else if let Some(output) = single_out {
        output_ts_type(context, output, known_types, delegate_names)
    } else {
        projected_ts_return_type(
            context,
            return_type_meta,
            is_async,
            known_types,
            delegate_names,
        )
    };
    let params = project_params(
        context,
        &in_params,
        known_types,
        delegate_names,
        delegate_sigs,
        delegate_param_wraps,
    );
    let mut argument_kinds = in_params
        .iter()
        .map(|param| js_argument_kind(&param.typ))
        .collect::<Vec<_>>();
    let args_expr = build_args_expr(context, &in_params);
    let invoke = if is_multi_output {
        "invokeAll"
    } else {
        "invoke"
    };
    let mut invoke_expr = format!(
        "_{}.method({}).{}({}, [{}])",
        iface.name, method.vtable_index, invoke, statics_call, args_expr
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
            .map(|p| projected_ts_return_type(context, Some(p), false, known_types, delegate_names))
            .unwrap_or_else(|| "unknown".to_string());
        // Build conversion expression for progress value
        let p_convert =
            convert_projected_return(context, "_p", progress_type, known_types, delegate_names);
        if p_convert != "_p" {
            progress_convert = Some(p_convert);
        }
        let is_action = matches!(return_type_meta, Some(TypeMeta::AsyncActionWithProgress(_)));
        let inner_convert =
            convert_projected_return(context, "_v", inner_type, known_types, delegate_names);
        if is_action {
            async_kind = AsyncKind::ActionWithProgress(progress_ts);
        } else {
            let inner_ts =
                projected_ts_return_type(context, inner_type, false, known_types, delegate_names);
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
            let convert_v =
                convert_projected_return(context, "_v", inner_type, known_types, delegate_names);
            let inner_ts =
                projected_ts_return_type(context, inner_type, false, known_types, delegate_names);
            async_kind = AsyncKind::Operation(inner_ts);
            async_convert_v = Some(convert_v);
        }
        sync_return_expr = None;
        array_return_expr = None;
    } else if is_multi_output {
        async_kind = AsyncKind::None;
        let (multi_return, multi_expr) =
            project_multi_output(context, method, &invoke_expr, known_types, delegate_names);
        ts_return = multi_return;
        sync_return_expr = Some(multi_expr);
        async_convert_v = None;
        array_return_expr = None;
    } else if let Some(elem) = array_out_elem {
        async_kind = AsyncKind::None;
        let arr_expr = if fill_array_uses_retval_count(method) {
            format!(
                "_r[{}].asArray()",
                fill_array_output_index(method).expect("FillArray output index")
            )
        } else {
            format!("{}.asArray()", invoke_expr)
        };
        let converted = if is_projected_delegate_type(context, Some(elem), delegate_names) {
            format!("{}.toValues().map(v => v.isNull() ? null : v)", arr_expr)
        } else {
            convert_array_return(context, &arr_expr, elem, known_types, &NO_DEFERRED)
        };
        sync_return_expr = None;
        async_convert_v = None;
        array_return_expr = if fill_array_uses_retval_count(method) {
            Some(format!(
                "(() => {{ const _r = {}; return {}.slice(0, _r[{}].toNumber()); }})()",
                invoke_expr,
                converted,
                method_abi_output_count(method) - 1
            ))
        } else {
            Some(converted)
        };
    } else if let Some(output) = single_out {
        async_kind = AsyncKind::None;
        sync_return_expr = Some(convert_output(
            context,
            &invoke_expr,
            output,
            known_types,
            delegate_names,
            &NO_DEFERRED,
        ));
        async_convert_v = None;
        array_return_expr = None;
    } else {
        async_kind = AsyncKind::None;
        let converted = convert_projected_return(
            context,
            &invoke_expr,
            return_type_meta,
            known_types,
            delegate_names,
        );
        sync_return_expr = if return_type_meta.is_some() {
            Some(converted)
        } else {
            None
        };
        async_convert_v = None;
        array_return_expr = None;
    }

    let mut ts_params = params;
    if is_async || is_with_progress {
        ts_params.push(ProjectedParam {
            name: "signal".into(),
            ts_type: "AbortSignal".into(),
            optional: true,
            delegate_wrap: None,
        });
        argument_kinds.push(None);
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
        argument_kinds,
        return_type: ts_return,
        async_kind,
        is_static: true,
        invoke_expr,
        sync_return_expr,
        async_convert_v,
        progress_convert,
        is_void: !has_return && !is_async,
        array_return_expr,
        delegate_wraps,
        js_only: false,
        overload_of,
    })
}

pub(super) fn project_instance_method(
    context: &JavaScriptProjectionContext,
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
    let is_multi_output = method_abi_output_count(method) > 1;
    let has_array_out = method.params.iter().any(|p| {
        (p.direction == ParamDirection::Out || p.direction == ParamDirection::OutFill)
            && matches!(p.typ, TypeMeta::Array(_))
    });
    let has_return = method_abi_output_count(method) > 0;

    let is_delegate_type =
        |typ: Option<&TypeMeta>| is_projected_delegate_type(context, typ, delegate_type_names);

    let mut doc = build_method_doc(method, &in_params);

    // Event add
    if method.is_event_add {
        return Some(project_event_add(
            context,
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
        let ts_return = projected_ts_return_type(
            context,
            return_type_meta,
            false,
            known_types,
            delegate_type_names,
        );
        let invoke_expr = format!(
            "{}.method({}).invoke({}, [])",
            iface_var, method.vtable_index, obj_expr
        );
        let converted = fast_getter_expression(
            context,
            iface_var,
            method.vtable_index,
            obj_expr,
            return_type_meta,
            &invoke_expr,
            known_types,
            delegate_type_names,
        )
        .unwrap_or_else(|| {
            convert_projected_return(
                context,
                &invoke_expr,
                return_type_meta,
                known_types,
                delegate_type_names,
            )
        });

        // Check if there's a corresponding setter
        let setter = find_setter_for_property(
            context,
            method,
            iface_var,
            obj_expr,
            iface_methods,
            known_types,
        );
        let (setter_line, setter_ts_type) = match setter {
            Some((line, ts_type)) => (Some(line), ts_type),
            None => (None, None),
        };

        return Some(ProjectedMember::Property(ProjectedProperty {
            name: prop_name,
            ts_type: ts_return,
            setter_ts_type,
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
                .map(|p| ts_param_type_safe(context, &p.typ, known_types))
                .unwrap_or_else(|| "any".to_string())
        };
        let setter_line = setter_line(
            context,
            iface_var,
            method.vtable_index,
            obj_expr,
            in_params.first().map(|param| &param.typ),
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
            ts_type: param_type.clone(),
            setter_ts_type: Some(param_type),
            readonly: false,
            is_static: false,
            doc,
            getter_expr: String::new(),
            setter_line: Some(setter_line),
        }));
    }

    // Normal method
    let params = project_params(
        context,
        &in_params,
        known_types,
        delegate_type_names,
        delegate_sigs,
        delegate_param_wraps,
    );
    let mut argument_kinds = in_params
        .iter()
        .map(|param| js_argument_kind(&param.typ))
        .collect::<Vec<_>>();
    let array_out_elem =
        if has_array_out && (return_type_meta.is_none() || fill_array_uses_retval_count(method)) {
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
    let single_out = if has_return && return_type_meta.is_none() && array_out_elem.is_none() {
        projected_method_outputs(method)
            .into_iter()
            .next()
            .map(|(_, typ)| typ)
    } else {
        None
    };

    let mut ts_return = if let Some(elem) = array_out_elem {
        if is_projected_delegate_type(context, Some(elem), delegate_type_names) {
            "Array<DynWinRtValue | null>".into()
        } else {
            ts_array_element_type(elem, known_types)
        }
    } else if let Some(output) = single_out {
        output_ts_type(context, output, known_types, delegate_type_names)
    } else {
        projected_ts_return_type(
            context,
            return_type_meta,
            is_async,
            known_types,
            delegate_type_names,
        )
    };

    let args_expr = build_args_expr(context, &in_params);
    let invoke = if is_multi_output {
        "invokeAll"
    } else {
        "invoke"
    };
    let mut invoke_expr = format!(
        "{}.method({}).{}({}, [{}])",
        iface_var, method.vtable_index, invoke, obj_expr, args_expr
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
            .map(|p| {
                projected_ts_return_type(context, Some(p), false, known_types, delegate_type_names)
            })
            .unwrap_or_else(|| "unknown".to_string());
        let p_convert = convert_projected_return(
            context,
            "_p",
            progress_type,
            known_types,
            delegate_type_names,
        );
        if p_convert != "_p" {
            progress_convert = Some(p_convert);
        }
        let is_action = matches!(return_type_meta, Some(TypeMeta::AsyncActionWithProgress(_)));
        let inner_convert =
            convert_projected_return(context, "_v", inner_type, known_types, delegate_type_names);
        if is_action {
            async_kind = AsyncKind::ActionWithProgress(progress_ts);
        } else {
            let inner_ts = projected_ts_return_type(
                context,
                inner_type,
                false,
                known_types,
                delegate_type_names,
            );
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
            let convert_v = convert_projected_return(
                context,
                "_v",
                inner_type,
                known_types,
                delegate_type_names,
            );
            let inner_ts = projected_ts_return_type(
                context,
                inner_type,
                false,
                known_types,
                delegate_type_names,
            );
            async_kind = AsyncKind::Operation(inner_ts);
            async_convert_v = Some(convert_v);
        }
        sync_return_expr = None;
        array_return_expr = None;
    } else if is_multi_output && !fill_array_uses_retval_count(method) {
        async_kind = AsyncKind::None;
        let (multi_return, multi_expr) = project_multi_output(
            context,
            method,
            &invoke_expr,
            known_types,
            delegate_type_names,
        );
        ts_return = multi_return;
        sync_return_expr = Some(multi_expr);
        async_convert_v = None;
        array_return_expr = None;
    } else if let Some(elem) = array_out_elem {
        async_kind = AsyncKind::None;
        let arr_expr = if fill_array_uses_retval_count(method) {
            format!(
                "_r[{}].asArray()",
                fill_array_output_index(method).expect("FillArray output index")
            )
        } else {
            format!("{}.asArray()", invoke_expr)
        };
        let converted = if is_projected_delegate_type(context, Some(elem), delegate_type_names) {
            format!("{}.toValues().map(v => v.isNull() ? null : v)", arr_expr)
        } else {
            convert_array_return(context, &arr_expr, elem, known_types, &NO_DEFERRED)
        };
        sync_return_expr = None;
        async_convert_v = None;
        array_return_expr = if fill_array_uses_retval_count(method) {
            Some(format!(
                "(() => {{ const _r = {}; return {}.slice(0, _r[{}].toNumber()); }})()",
                invoke_expr,
                converted,
                method_abi_output_count(method) - 1
            ))
        } else {
            Some(converted)
        };
    } else if let Some(output) = single_out {
        async_kind = AsyncKind::None;
        sync_return_expr = Some(convert_output(
            context,
            &invoke_expr,
            output,
            known_types,
            delegate_type_names,
            &NO_DEFERRED,
        ));
        async_convert_v = None;
        array_return_expr = None;
    } else {
        async_kind = AsyncKind::None;
        if has_return {
            let converted = convert_projected_return(
                context,
                &invoke_expr,
                return_type_meta,
                known_types,
                delegate_type_names,
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
        argument_kinds.push(None);
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
        argument_kinds,
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
    context: &JavaScriptProjectionContext,
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
        TypeMeta::Parameterized {
            namespace,
            name,
            piid,
            args,
        } => Some(context.projected_parameterized_name(namespace, name, piid, args)),
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
            let s_ts = ts_return_type_safe(context, Some(&args[0]), false, known_types);
            let a_ts = ts_return_type_safe(context, Some(&args[1]), false, known_types);
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
            let s_wrap = convert_return(
                context,
                "__a0__",
                Some(&args[0]),
                false,
                known_types,
                &NO_DEFERRED,
            );
            let a_wrap = convert_return(
                context,
                "__a1__",
                Some(&args[1]),
                false,
                known_types,
                &NO_DEFERRED,
            );
            (
                format!("(sender: {}, args: {}) => void", s_ts_pub, a_ts_pub),
                Some(s_wrap),
                Some(a_wrap),
            )
        }
        Some(TypeMeta::Parameterized { name, args, .. })
            if name.split('`').next() == Some("EventHandler") && args.len() == 1 =>
        {
            let a_ts = ts_return_type_safe(context, Some(&args[0]), false, known_types);
            let a_ts_pub = if a_ts == "DynWinRtValue" {
                "unknown".to_string()
            } else {
                a_ts
            };
            let a_wrap = convert_return(
                context,
                "__a1__",
                Some(&args[0]),
                false,
                known_types,
                &NO_DEFERRED,
            );
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
    context: &JavaScriptProjectionContext,
    getter: &MethodMeta,
    iface_var: &str,
    obj_expr: &str,
    iface_methods: Option<&[MethodMeta]>,
    known_types: &HashSet<String>,
) -> Option<(String, Option<String>)> {
    let prop_suffix = getter.name.strip_prefix("get_")?;
    let setter_name = format!("put_{}", prop_suffix);
    let methods = iface_methods?;
    let setter = methods
        .iter()
        .find(|m| m.name == setter_name && m.is_property_setter)?;
    let setter_in_params = get_in_params(setter);
    let setter_ts_type = setter_in_params.first().and_then(|param| {
        ireference_inner_type(&param.typ)
            .is_some()
            .then(|| ts_param_type_safe(context, &param.typ, known_types))
    });
    Some((
        setter_line(
            context,
            iface_var,
            setter.vtable_index,
            obj_expr,
            setter_in_params.first().map(|param| &param.typ),
        ),
        setter_ts_type,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::ParamMeta;
    use std::sync::LazyLock;

    fn context() -> &'static JavaScriptProjectionContext {
        static CONTEXT: LazyLock<JavaScriptProjectionContext> = LazyLock::new(|| {
            crate::codegen::winrt::javascript::create_javascript_projection_context([])
                .expect("empty test projection context")
        });
        &CONTEXT
    }

    fn project_instance_method(
        iface_var: &str,
        obj_expr: &str,
        method: &MethodMeta,
        known_types: &HashSet<String>,
        delegate_type_names: &HashSet<String>,
        iface_methods: Option<&[MethodMeta]>,
        delegate_sigs: &HashMap<String, String>,
        delegate_param_wraps: &HashMap<String, Vec<String>>,
    ) -> Option<ProjectedMember> {
        super::project_instance_method(
            context(),
            iface_var,
            obj_expr,
            method,
            known_types,
            delegate_type_names,
            iface_methods,
            delegate_sigs,
            delegate_param_wraps,
        )
    }

    fn project_static_method(
        class: &ClassMeta,
        iface: &InterfaceMeta,
        method: &MethodMeta,
        known_types: &HashSet<String>,
        delegate_names: &HashSet<String>,
        delegate_sigs: &HashMap<String, String>,
        delegate_param_wraps: &HashMap<String, Vec<String>>,
    ) -> ProjectedMember {
        super::project_static_method(
            context(),
            class,
            iface,
            method,
            known_types,
            delegate_names,
            delegate_sigs,
            delegate_param_wraps,
        )
    }

    fn fill_array_with_boolean_result() -> MethodMeta {
        MethodMeta {
            name: "TryGetValues".into(),
            raw_name: "TryGetValues".into(),
            vtable_index: 6,
            params: vec![ParamMeta {
                name: "values".into(),
                typ: TypeMeta::Array(Box::new(TypeMeta::F32)),
                direction: ParamDirection::OutFill,
            }],
            return_type: Some(TypeMeta::Bool),
            ..Default::default()
        }
    }

    #[test]
    fn projects_non_counted_multi_outputs_for_instance_and_static_methods() {
        let method = fill_array_with_boolean_result();
        let delegate_sigs: HashMap<String, String> = HashMap::new();
        let delegate_wraps: HashMap<String, Vec<String>> = HashMap::new();
        let instance = project_instance_method(
            "_IWidget",
            "this._obj",
            &method,
            &HashSet::new(),
            &HashSet::new(),
            None,
            &delegate_sigs,
            &delegate_wraps,
        )
        .expect("instance method");
        let ProjectedMember::Method(instance) = instance else {
            panic!("expected method");
        };
        assert_eq!(instance.return_type, "[number[], boolean]");
        let instance_expr = instance.sync_return_expr.expect("instance return");
        assert!(instance_expr.contains(".invokeAll("));
        assert!(instance_expr.contains("_r[0].asArray().toF32Vec()"));
        assert!(instance_expr.contains("_r[1].toBool()"));

        let iface = InterfaceMeta {
            name: "IWidgetStatics".into(),
            methods: vec![method.clone()],
            ..Default::default()
        };
        let class = ClassMeta {
            name: "Widget".into(),
            ..Default::default()
        };
        let ProjectedMember::Method(static_method) = project_static_method(
            &class,
            &iface,
            &method,
            &HashSet::new(),
            &HashSet::new(),
            &delegate_sigs,
            &delegate_wraps,
        ) else {
            panic!("expected static method");
        };
        assert_eq!(static_method.return_type, "[number[], boolean]");
        let static_expr = static_method.sync_return_expr.expect("static return");
        assert!(static_expr.contains(".invokeAll("));
        assert!(static_expr.contains("_r[0].asArray().toF32Vec()"));
        assert!(static_expr.contains("_r[1].toBool()"));
    }
}
