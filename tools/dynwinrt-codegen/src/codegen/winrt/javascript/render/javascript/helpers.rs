// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Member, overload, and asynchronous JavaScript emitters.

use crate::codegen::winrt::javascript::ir::*;

// ======================================================================
// Async scaffolding
// ======================================================================

pub(super) fn emit_abortable_async_body(
    out: &mut String,
    invoke_expr: &str,
    convert_v_expr: &str,
    is_action: bool,
) {
    out.push_str("        if (signal?.aborted) throw signal.reason;\n");
    out.push_str(&format!("        const _op = {};\n", invoke_expr));
    out.push_str("        const _onAbort = signal ? () => { try { _op.cancel(); } catch (_ce) { /* cancel after completion is a no-op per WinRT spec */ } } : undefined;\n");
    out.push_str("        if (_onAbort) { signal.addEventListener('abort', _onAbort, { once: true }); if (signal.aborted) _onAbort(); }\n");
    out.push_str("        try {\n");
    if is_action {
        out.push_str("            await _op.toPromise();\n");
        out.push_str("            return;\n");
    } else {
        out.push_str("            const _v = await _op.toPromise();\n");
        out.push_str(&format!("            return {};\n", convert_v_expr));
    }
    out.push_str("        } catch (_e) {\n");
    out.push_str("            if (signal?.aborted) throw signal.reason;\n");
    out.push_str("            throw _e;\n");
    out.push_str("        } finally {\n");
    out.push_str("            if (_onAbort) signal.removeEventListener('abort', _onAbort);\n");
    out.push_str("        }\n");
}

pub(super) fn emit_with_progress_body(
    out: &mut String,
    invoke_expr: &str,
    inner_convert: &str,
    is_action: bool,
    progress_convert: Option<&str>,
) {
    out.push_str("        if (signal?.aborted) {\n");
    out.push_str("            const _rej = Promise.reject(signal.reason);\n");
    out.push_str("            _rej.catch(() => {});\n");
    out.push_str("            return Object.assign(_rej, {\n");
    out.push_str("                progress(_cb) { return this; },\n");
    out.push_str("                toPromise() { const _p = Promise.reject(signal.reason); _p.catch(() => {}); return _p; },\n");
    out.push_str("                cancel() {},\n");
    out.push_str("            });\n");
    out.push_str("        }\n");
    out.push_str(&format!("        const _op = {};\n", invoke_expr));
    out.push_str("        const _onAbort = signal ? () => { try { _op.cancel(); } catch (_ce) { /* cancel after completion is a no-op per WinRT spec */ } } : undefined;\n");
    out.push_str("        if (_onAbort) { signal.addEventListener('abort', _onAbort, { once: true }); if (signal.aborted) _onAbort(); }\n");
    let then_expr = if is_action {
        "() => undefined".to_string()
    } else {
        format!("(_v) => {}", inner_convert)
    };
    out.push_str(&format!(
        "        const _wrap = (p) => p.then({then}, (_e) => {{ if (signal?.aborted) throw signal.reason; throw _e; }}).finally(() => {{ if (_onAbort) signal.removeEventListener('abort', _onAbort); }});\n",
        then = then_expr
    ));
    out.push_str("        const _promise = _wrap(_op.toPromise());\n");
    out.push_str("        return Object.assign(_promise, {\n");
    // Wrap progress callback to convert raw DynWinRtValue to the projected type
    if let Some(p_conv) = progress_convert {
        out.push_str(&format!(
            "            progress(cb) {{ _op.onProgress((_p) => cb({})); return this; }},\n",
            p_conv
        ));
    } else {
        out.push_str("            progress(cb) { _op.onProgress(cb); return this; },\n");
    }
    out.push_str("            toPromise() { return _wrap(_op.toPromise()); },\n");
    out.push_str("            cancel() { try { _op.cancel(); } catch (_ce) { /* cancel after completion is a no-op per WinRT spec */ } },\n");
    out.push_str("        });\n");
}

// ======================================================================
// Helpers
// ======================================================================

pub(super) fn render_jsdoc(doc: &DocInfo, indent: &str) -> String {
    let doc_text = crate::codegen::winrt::shared::docs::DocText {
        summary: doc.summary.as_deref(),
        deprecated: doc.deprecated.as_deref(),
        returns: doc.returns.as_deref(),
        params: doc
            .params
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_str()))
            .collect(),
    };
    crate::codegen::winrt::javascript::docs::format_jsdoc(&doc_text, indent)
}

pub(super) fn inject_unwrap(code: String) -> String {
    if !code.contains("_unwrap(") {
        return code;
    }
    let mut last_import_end: Option<usize> = None;
    let mut cursor = 0usize;
    for line in code.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("import ") || trimmed.starts_with("import{") {
            last_import_end = Some(cursor + line.len());
        }
        cursor += line.len();
    }
    match last_import_end {
        Some(insert_at) => {
            let mut out = String::with_capacity(code.len() + 64);
            out.push_str(&code[..insert_at]);
            out.push_str("const _unwrap = (x) => x?._obj ?? x;\n");
            out.push_str(&code[insert_at..]);
            out
        }
        None => code,
    }
}

pub(super) fn emit_delegate_wraps(out: &mut String, method: &ProjectedMethod) {
    for (param_name, delegate_name) in &method.delegate_wraps {
        let needs_wrap = method
            .params
            .iter()
            .find(|p| &p.name == param_name)
            .and_then(|p| p.delegate_wrap.as_ref())
            .map(|dw| {
                dw.param_wraps
                    .iter()
                    .enumerate()
                    .any(|(i, wrap)| *wrap != format!("__a{}__", i))
            })
            .unwrap_or(false);
        if needs_wrap {
            let wrap = method
                .params
                .iter()
                .find(|p| &p.name == param_name)
                .and_then(|p| p.delegate_wrap.as_ref())
                .unwrap();
            let arg_vars: Vec<String> = (0..wrap.param_wraps.len())
                .map(|i| format!("__a{}__", i))
                .collect();
            out.push_str(&format!(
                "        const _{}_wrapped = ({}) => {}({});\n",
                param_name,
                arg_vars.join(", "),
                param_name,
                wrap.param_wraps.join(", ")
            ));
            out.push_str(&format!(
                "        const _{}_d = DynWinRtDelegate.create(IID_{}, {}_PARAM_TYPES, _{}_wrapped).toValue();\n",
                param_name, delegate_name, delegate_name, param_name
            ));
        } else {
            out.push_str(&format!(
                "        const _{}_d = DynWinRtDelegate.create(IID_{}, {}_PARAM_TYPES, {}).toValue();\n",
                param_name, delegate_name, delegate_name, param_name
            ));
        }
    }
}

fn argument_kind_condition(index: usize, kind: JsArgumentKind, matches: bool) -> String {
    let condition = match kind {
        JsArgumentKind::String => format!("typeof args[{index}] === 'string'"),
        JsArgumentKind::Number => format!("typeof args[{index}] === 'number'"),
        JsArgumentKind::BigInt => format!("typeof args[{index}] === 'bigint'"),
        JsArgumentKind::Boolean => format!("typeof args[{index}] === 'boolean'"),
        JsArgumentKind::Object => format!(
            "(args[{index}] === null || typeof args[{index}] === 'object' || typeof args[{index}] === 'function')"
        ),
        JsArgumentKind::Array => format!("Array.isArray(args[{index}])"),
    };
    if matches {
        condition
    } else {
        format!("!({condition})")
    }
}

/// Generate a JS dispatcher for same-class overloads (from OverloadAttribute).
/// Emits internal `_methodName__N` implementations and a public dispatcher.
pub(super) fn render_same_class_overload_js(
    out: &mut String,
    first: &ProjectedMethod,
    others: &[&ProjectedMethod],
) {
    let mut all: Vec<&ProjectedMethod> = vec![first];
    all.extend(others.iter().copied());
    // Sort by param count ascending (fewest params first)
    all.sort_by_key(|m| m.params.len());

    // Emit internal implementations as _name__1, _name__2, etc.
    for (idx, method) in all.iter().enumerate() {
        let internal_name = format!("_{}_{}", first.name, idx + 1);
        let static_kw = if method.is_static { "static " } else { "" };
        let async_kw = match method.async_kind {
            AsyncKind::Action | AsyncKind::Operation(_) => "async ",
            _ => "",
        };
        let params_str = method
            .params
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "    {}{}{}({}) {{\n",
            static_kw, async_kw, internal_name, params_str
        ));
        emit_delegate_wraps(out, method);
        match &method.async_kind {
            AsyncKind::None => {
                if let Some(ref arr_expr) = method.array_return_expr {
                    out.push_str(&format!("        return {};\n", arr_expr));
                } else if method.is_void {
                    out.push_str(&format!("        {};\n", method.invoke_expr));
                } else if let Some(ref ret) = method.sync_return_expr {
                    out.push_str(&format!("        return {};\n", ret));
                }
            }
            AsyncKind::Action => {
                emit_abortable_async_body(out, &method.invoke_expr, "", true);
            }
            AsyncKind::Operation(_) => {
                let convert = method.async_convert_v.as_deref().unwrap_or("_v");
                emit_abortable_async_body(out, &method.invoke_expr, convert, false);
            }
            AsyncKind::ActionWithProgress(_) => {
                let convert = method.async_convert_v.as_deref().unwrap_or("undefined");
                emit_with_progress_body(
                    out,
                    &method.invoke_expr,
                    convert,
                    true,
                    method.progress_convert.as_deref(),
                );
            }
            AsyncKind::OperationWithProgress(_, _) => {
                let convert = method.async_convert_v.as_deref().unwrap_or("_v");
                emit_with_progress_body(
                    out,
                    &method.invoke_expr,
                    convert,
                    false,
                    method.progress_convert.as_deref(),
                );
            }
        }
        out.push_str("    }\n");
    }

    // Emit public dispatcher
    if let Some(ref doc) = first.doc {
        out.push_str(&render_jsdoc(doc, "    "));
    }
    let static_kw = if first.is_static { "static " } else { "" };
    out.push_str(&format!("    {}{}(...args) {{\n", static_kw, first.name));

    // Dispatch branches from most-params to fewest-params
    for (idx, method) in all.iter().enumerate().rev() {
        let required_count = method.params.iter().filter(|p| !p.optional).count();
        let internal_name = format!("_{}_{}", first.name, idx + 1);

        if idx == 0 {
            // Fewest params — default fallthrough
            let args = (0..method.params.len())
                .map(|i| format!("args[{}]", i))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!(
                "        return this.{}({});\n",
                internal_name, args
            ));
        } else {
            // Build condition to distinguish this overload from shorter ones.
            // Compare against the next-shorter overload to find the discriminator.
            let shorter = all[idx - 1];
            let shorter_required = shorter.params.iter().filter(|p| !p.optional).count();

            let mut conditions = Vec::new();

            if required_count > shorter_required {
                // Different param count: check args.length
                conditions.push(format!("args.length >= {}", required_count));
                // Also check the distinguishing arg is not AbortSignal
                let diff_idx = shorter_required;
                conditions.push(format!("args[{}] !== undefined", diff_idx));
                conditions.push(format!("!(args[{}] instanceof AbortSignal)", diff_idx));
            } else if required_count == shorter_required {
                // Same required count: use the first argument whose projected
                // JavaScript kinds differ. This keeps named enums numeric while
                // runtime classes and object parameters select object overloads.
                conditions.push(format!("args.length >= {}", required_count));
                let discriminator = method
                    .params
                    .iter()
                    .zip(&shorter.params)
                    .enumerate()
                    .find_map(|(param_index, (_current, _previous))| {
                        let current_kind =
                            method.argument_kinds.get(param_index).copied().flatten();
                        let previous_kind =
                            shorter.argument_kinds.get(param_index).copied().flatten();
                        if current_kind == previous_kind {
                            return None;
                        }
                        current_kind
                            .map(|kind| argument_kind_condition(param_index, kind, true))
                            .or_else(|| {
                                previous_kind
                                    .map(|kind| argument_kind_condition(param_index, kind, false))
                            })
                    });
                if let Some(discriminator) = discriminator {
                    conditions.push(discriminator);
                } else {
                    let first_param = &method.params[0].ts_type;
                    if first_param == "string" {
                        conditions.push("typeof args[0] === 'string'".to_string());
                    } else {
                        conditions.push("typeof args[0] !== 'string'".to_string());
                    }
                }
            }

            let cond = conditions.join(" && ");
            let args = (0..method.params.len())
                .map(|i| format!("args[{}]", i))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("        if ({}) {{\n", cond));
            out.push_str(&format!(
                "            return this.{}({});\n",
                internal_name, args
            ));
            out.push_str("        }\n");
        }
    }

    out.push_str("    }\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn method(
        param_name: &str,
        ts_type: &str,
        invoke_expr: &str,
        argument_kind: JsArgumentKind,
    ) -> ProjectedMethod {
        ProjectedMethod {
            name: "setPresenter".into(),
            doc: None,
            params: vec![ProjectedParam {
                name: param_name.into(),
                ts_type: ts_type.into(),
                optional: false,
                delegate_wrap: None,
            }],
            argument_kinds: vec![Some(argument_kind)],
            return_type: "void".into(),
            async_kind: AsyncKind::None,
            is_static: false,
            invoke_expr: invoke_expr.into(),
            sync_return_expr: None,
            async_convert_v: None,
            progress_convert: None,
            is_void: true,
            array_return_expr: None,
            delegate_wraps: Vec::new(),
            js_only: false,
            overload_of: None,
        }
    }

    #[test]
    fn same_arity_enum_and_runtime_class_dispatch_by_javascript_kind() {
        let presenter = method(
            "appWindowPresenter",
            "AppWindowPresenter",
            "_IAppWindow.method(25).invoke(this._obj, [(appWindowPresenter == null ? DynWinRtValue.nullValue() : _unwrap(appWindowPresenter))])",
            JsArgumentKind::Object,
        );
        let presenter_kind = method(
            "appWindowPresenterKind",
            "AppWindowPresenterKind",
            "_IAppWindow.method(26).invoke(this._obj, [DynWinRtValue.i32(appWindowPresenterKind)])",
            JsArgumentKind::Number,
        );
        let mut output = String::new();

        render_same_class_overload_js(&mut output, &presenter, &[&presenter_kind]);

        assert!(output.contains("typeof args[0] === 'number'"));
        assert!(output.contains("return this._setPresenter_2(args[0]);"));
        assert!(output.contains("return this._setPresenter_1(args[0]);"));
        assert!(!output.contains("typeof args[0] !== 'string'"));
    }

    #[test]
    fn same_arity_runtime_class_branch_accepts_projected_objects() {
        let presenter_kind = method(
            "appWindowPresenterKind",
            "AppWindowPresenterKind",
            "_IAppWindow.method(26).invoke(this._obj, [DynWinRtValue.i32(appWindowPresenterKind)])",
            JsArgumentKind::Number,
        );
        let presenter = method(
            "appWindowPresenter",
            "AppWindowPresenter",
            "_IAppWindow.method(25).invoke(this._obj, [(appWindowPresenter == null ? DynWinRtValue.nullValue() : _unwrap(appWindowPresenter))])",
            JsArgumentKind::Object,
        );
        let mut output = String::new();

        render_same_class_overload_js(&mut output, &presenter_kind, &[&presenter]);

        assert!(output.contains("typeof args[0] === 'object'"));
        assert!(output.contains("return this._setPresenter_2(args[0]);"));
        assert!(output.contains("return this._setPresenter_1(args[0]);"));
    }

    #[test]
    fn same_arity_i64_and_number_dispatch_bigint_branch() {
        let number = method(
            "value",
            "number",
            "_ITest.method(6).invoke(this._obj, [DynWinRtValue.i32(value)])",
            JsArgumentKind::Number,
        );
        let bigint = method(
            "value",
            "bigint",
            "_ITest.method(7).invoke(this._obj, [DynWinRtValue.i64(value)])",
            JsArgumentKind::BigInt,
        );
        let mut output = String::new();

        render_same_class_overload_js(&mut output, &number, &[&bigint]);

        assert!(output.contains("typeof args[0] === 'bigint'"));
        assert!(!output.contains("typeof args[0] === 'number'"));
        assert!(output.contains("return this._setPresenter_2(args[0]);"));
        assert!(output.contains("return this._setPresenter_1(args[0]);"));
    }

    #[test]
    fn same_arity_u64_and_number_dispatch_number_branch() {
        let bigint = method(
            "value",
            "bigint",
            "_ITest.method(6).invoke(this._obj, [DynWinRtValue.u64(value)])",
            JsArgumentKind::BigInt,
        );
        let number = method(
            "value",
            "number",
            "_ITest.method(7).invoke(this._obj, [DynWinRtValue.f64(value)])",
            JsArgumentKind::Number,
        );
        let mut output = String::new();

        render_same_class_overload_js(&mut output, &bigint, &[&number]);

        assert!(output.contains("typeof args[0] === 'number'"));
        assert!(!output.contains("typeof args[0] === 'bigint'"));
        assert!(output.contains("return this._setPresenter_2(args[0]);"));
        assert!(output.contains("return this._setPresenter_1(args[0]);"));
    }
}
/// and required interface overloads based on argument count.
/// E.g. for `rewriteAsync(text, signal?)` (main) + `rewriteAsync(text, tone, signal?)` (ITextRewriter2),
/// emits:
/// ```js
/// rewriteAsync(text, ...args) {
///     if (args.length >= 1 && typeof args[0] !== 'object') {
///         return ITextRewriter2.from(this._obj).rewriteAsync(text, ...args);
///     }
///     // original body for 1-arg version
/// }
/// ```
pub(super) fn render_overload_dispatcher_js(
    out: &mut String,
    main_method: &ProjectedMethod,
    overloads: &[(&ProjectedMethod, &str)],
    _class_name: &str,
) {
    // Count required params (non-optional, non-signal) in main method
    let main_required: usize = main_method.params.iter().filter(|p| !p.optional).count();

    // Collect all overloads sorted by required param count (descending) for dispatch
    let mut all_overloads: Vec<(&ProjectedMethod, &str, usize)> = overloads
        .iter()
        .map(|(m, iface)| {
            let required = m.params.iter().filter(|p| !p.optional).count();
            (*m, *iface, required)
        })
        .collect();
    all_overloads.sort_by(|a, b| b.2.cmp(&a.2));

    // Build the dispatcher: use rest args, dispatch by argument count
    if let Some(ref doc) = main_method.doc {
        out.push_str(&render_jsdoc(doc, "    "));
    }
    let static_kw = if main_method.is_static { "static " } else { "" };
    let async_kw = match main_method.async_kind {
        AsyncKind::Action | AsyncKind::Operation(_) => "async ",
        _ => "",
    };
    // Use all param names from the longest overload for the signature
    let all_param_names: Vec<&str> = {
        let longest = std::iter::once(main_method)
            .chain(all_overloads.iter().map(|(m, _, _)| *m))
            .max_by_key(|m| m.params.len())
            .unwrap();
        longest.params.iter().map(|p| p.name.as_str()).collect()
    };
    let params_str = all_param_names.join(", ");
    out.push_str(&format!(
        "    {}{}{}({}) {{\n",
        static_kw, async_kw, main_method.name, params_str
    ));

    // Dispatch branches: check if extra args are provided (not AbortSignal)
    for (overload, iface_name, required_count) in &all_overloads {
        if *required_count > main_required {
            // The differentiating param is the one at index main_required
            let diff_param = &all_param_names[main_required];
            out.push_str(&format!(
                "        if ({} !== undefined && !({} instanceof AbortSignal)) {{\n",
                diff_param, diff_param
            ));
            // Forward to required interface
            let fwd_args = overload
                .params
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let obj_expr = if main_method.is_static {
                format!("{}.s_{}()", _class_name, iface_name)
            } else {
                "this._obj".to_string()
            };
            out.push_str(&format!(
                "            return {}.from({}).{}({});\n",
                iface_name, obj_expr, main_method.name, fwd_args
            ));
            out.push_str("        }\n");
        }
    }

    // Fall through to original method body
    // Re-render the original method body inline (skip the signature, we already emitted it)
    emit_delegate_wraps(out, main_method);
    match &main_method.async_kind {
        AsyncKind::None => {
            if let Some(ref arr_expr) = main_method.array_return_expr {
                out.push_str(&format!("        return {};\n", arr_expr));
            } else if main_method.is_void {
                out.push_str(&format!("        {};\n", main_method.invoke_expr));
            } else if let Some(ref ret) = main_method.sync_return_expr {
                out.push_str(&format!("        return {};\n", ret));
            }
        }
        AsyncKind::Action => {
            emit_abortable_async_body(out, &main_method.invoke_expr, "", true);
        }
        AsyncKind::Operation(_) => {
            let convert = main_method.async_convert_v.as_deref().unwrap_or("_v");
            emit_abortable_async_body(out, &main_method.invoke_expr, convert, false);
        }
        AsyncKind::ActionWithProgress(_) => {
            let convert = main_method
                .async_convert_v
                .as_deref()
                .unwrap_or("undefined");
            emit_with_progress_body(
                out,
                &main_method.invoke_expr,
                convert,
                true,
                main_method.progress_convert.as_deref(),
            );
        }
        AsyncKind::OperationWithProgress(_, _) => {
            let convert = main_method.async_convert_v.as_deref().unwrap_or("_v");
            emit_with_progress_body(
                out,
                &main_method.invoke_expr,
                convert,
                false,
                main_method.progress_convert.as_deref(),
            );
        }
    }
    out.push_str("    }\n");
}
