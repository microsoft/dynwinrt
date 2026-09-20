// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;

use super::JavaScriptProjectionContext;
use super::input::CollectionInput;
use crate::codegen::winrt::shared::imports::ireference_inner_type;
use crate::types::TypeMeta;

// ======================================================================
// TypeScript type annotation helpers
// ======================================================================

/// Preserve top-level union/intersection precedence without regrouping nested types.
pub(crate) fn ts_array_type(element_type: &str) -> String {
    let mut depth = 0usize;
    for ch in element_type.chars() {
        match ch {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth = depth.saturating_sub(1),
            '|' | '&' if depth == 0 => return format!("({element_type})[]"),
            _ => {}
        }
    }
    format!("{element_type}[]")
}

fn ts_param_type(context: &JavaScriptProjectionContext, typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => "boolean".to_string(),
        TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::Char16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::F32
        | TypeMeta::F64 => "number".to_string(),
        TypeMeta::I64 | TypeMeta::U64 => "bigint".to_string(),
        TypeMeta::String | TypeMeta::Guid => "string".to_string(),
        TypeMeta::RuntimeClass { name, .. }
        | TypeMeta::Enum { name, .. }
        | TypeMeta::Interface { name, .. } => name.clone(),
        TypeMeta::Parameterized {
            namespace,
            name,
            piid,
            args,
        } => context.projected_parameterized_name(namespace, name, piid, args),
        TypeMeta::Array(_) => "DynWinRtArray".to_string(),
        TypeMeta::Object => "unknown".to_string(),
        TypeMeta::Delegate { .. } => "DynWinRtValue".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "number".to_string(),
        TypeMeta::Struct { name, .. } => name.clone(),
        _ => "any".to_string(),
    }
}

pub(crate) fn ts_param_type_safe(
    context: &JavaScriptProjectionContext,
    typ: &TypeMeta,
    known: &HashSet<String>,
) -> String {
    if let Some(inner) = ireference_inner_type(typ) {
        let native = ts_return_type_safe(context, Some(inner), false, known);
        let wrapper = match typ {
            TypeMeta::Parameterized {
                namespace,
                name,
                piid,
                args,
            } => context.projected_parameterized_name(namespace, name, piid, args),
            _ => unreachable!(),
        };
        return format!("{} | null | {}", native, wrapper);
    }

    match typ {
        TypeMeta::RuntimeClass { name, .. }
        | TypeMeta::Enum { name, .. }
        | TypeMeta::Interface { name, .. }
            if !known.contains(name) =>
        {
            "DynWinRtValue".to_string()
        }
        _ => ts_param_type(context, typ),
    }
}

pub(crate) fn ts_reference_input_type(
    context: &JavaScriptProjectionContext,
    typ: &TypeMeta,
    known: &HashSet<String>,
) -> String {
    let base = ts_param_type_safe(context, typ, known);
    if CollectionInput::from_type(typ).is_some() {
        format!("{base} | null")
    } else {
        base
    }
}

/// Collection inputs also accept their JS-native equivalent and an absent reference.
pub(crate) fn ts_param_type_dts(
    context: &JavaScriptProjectionContext,
    typ: &TypeMeta,
    known: &HashSet<String>,
) -> String {
    // Array params: show as T[] in DTS (runtime uses DynWinRtArray but users pass arrays).
    // For byte[] (U8) also advertise Uint8Array — far more memory-efficient than
    // a boxed `Array<number>` of length N for large pixel buffers.
    if let TypeMeta::Array(inner) = typ {
        if matches!(inner.as_ref(), TypeMeta::U8) {
            return "Uint8Array | number[]".to_string();
        }
        let elem_ts = ts_reference_input_type(context, inner, known);
        return ts_array_type(&elem_ts);
    }
    if let Some(collection) = CollectionInput::from_type(typ) {
        let base = ts_param_type_safe(context, typ, known);
        return match collection {
            CollectionInput::Vector(element) => {
                let elem_ts = ts_reference_input_type(context, element, known);
                format!("{} | {} | null", base, ts_array_type(&elem_ts))
            }
            CollectionInput::Map(key, value) => {
                let k_ts = ts_reference_input_type(context, key, known);
                let v_ts = ts_reference_input_type(context, value, known);
                format!("{} | Map<{}, {}> | null", base, k_ts, v_ts)
            }
        };
    }
    ts_param_type_safe(context, typ, known)
}

pub(crate) fn ts_return_type_safe(
    context: &JavaScriptProjectionContext,
    typ: Option<&TypeMeta>,
    is_async: bool,
    known: &HashSet<String>,
) -> String {
    if let Some(inner) = typ.and_then(ireference_inner_type) {
        let native = format!(
            "{} | null",
            ts_return_type_safe(context, Some(inner), false, known)
        );
        return if is_async {
            format!("Promise<{}>", native)
        } else {
            native
        };
    }

    match typ {
        Some(TypeMeta::RuntimeClass { name, .. })
        | Some(TypeMeta::Enum { name, .. })
        | Some(TypeMeta::Interface { name, .. })
            if !known.contains(name) =>
        {
            if is_async {
                "Promise<DynWinRtValue>".to_string()
            } else {
                "DynWinRtValue".to_string()
            }
        }
        Some(TypeMeta::AsyncOperation(inner)) => {
            format!(
                "Promise<{}>",
                ts_return_type_safe(context, Some(inner), false, known)
            )
        }
        Some(TypeMeta::AsyncOperationWithProgress(result, _)) => {
            let inner = ts_return_type_safe(context, Some(result), false, known);
            format!(
                "Promise<{i}> & {{ progress(cb: (value: unknown) => void): Promise<{i}> & {{ progress: any; toPromise(): Promise<{i}>; cancel(): void; }}; toPromise(): Promise<{i}>; cancel(): void; }}",
                i = inner
            )
        }
        Some(TypeMeta::Array(inner)) => {
            let s = ts_array_element_type(inner, known);
            if is_async {
                format!("Promise<{}>", s)
            } else {
                s
            }
        }
        _ => ts_return_type(context, typ, is_async),
    }
}

fn ts_return_type(
    context: &JavaScriptProjectionContext,
    typ: Option<&TypeMeta>,
    is_async: bool,
) -> String {
    let inner = match typ {
        Some(TypeMeta::String) | Some(TypeMeta::Guid) => "string",
        Some(TypeMeta::Bool) => "boolean",
        Some(
            TypeMeta::I8
            | TypeMeta::U8
            | TypeMeta::I16
            | TypeMeta::U16
            | TypeMeta::Char16
            | TypeMeta::I32
            | TypeMeta::U32
            | TypeMeta::F32
            | TypeMeta::F64,
        ) => "number",
        Some(TypeMeta::I64 | TypeMeta::U64) => "bigint",
        Some(TypeMeta::RuntimeClass { name, .. }) => {
            return if is_async {
                format!("Promise<{}>", name)
            } else {
                name.clone()
            };
        }
        Some(TypeMeta::Enum { name, .. }) => {
            return if is_async {
                format!("Promise<{}>", name)
            } else {
                name.clone()
            };
        }
        Some(TypeMeta::Interface { name, .. }) => {
            return if is_async {
                format!("Promise<{}>", name)
            } else {
                name.clone()
            };
        }
        Some(TypeMeta::Parameterized {
            namespace,
            name,
            piid,
            args,
        }) => {
            let s = context.projected_parameterized_name(namespace, name, piid, args);
            return if is_async {
                format!("Promise<{}>", s)
            } else {
                s
            };
        }
        Some(TypeMeta::AsyncOperation(inner)) => {
            return format!("Promise<{}>", ts_return_type(context, Some(inner), false));
        }
        Some(TypeMeta::AsyncOperationWithProgress(result, _)) => {
            let inner = ts_return_type(context, Some(result), false);
            return format!(
                "Promise<{i}> & {{ progress(cb: (value: unknown) => void): Promise<{i}> & {{ progress: any; toPromise(): Promise<{i}>; cancel(): void; }}; toPromise(): Promise<{i}>; cancel(): void; }}",
                i = inner
            );
        }
        Some(TypeMeta::AsyncAction) => return "Promise<void>".to_string(),
        Some(TypeMeta::AsyncActionWithProgress(_)) => {
            return "Promise<void> & { progress(cb: (value: unknown) => void): Promise<void> & { progress: any; toPromise(): Promise<void>; cancel(): void; }; toPromise(): Promise<void>; cancel(): void; }".to_string();
        }
        Some(TypeMeta::Array(inner)) => {
            let s = ts_array_element_type(inner, &HashSet::new());
            return if is_async {
                format!("Promise<{}>", s)
            } else {
                s
            };
        }
        Some(TypeMeta::Object) => "unknown",
        Some(TypeMeta::Delegate { .. }) => "DynWinRtValue",
        Some(TypeMeta::Struct { name, .. }) if name == "HResult" => "number",
        Some(TypeMeta::Struct { name, .. }) => {
            return if is_async {
                format!("Promise<{}>", name)
            } else {
                name.clone()
            };
        }
        None => "void",
    };
    if is_async {
        format!("Promise<{}>", inner)
    } else {
        inner.to_string()
    }
}

/// TypeScript return type annotation for an array element type.
pub(crate) fn ts_array_element_type(inner: &TypeMeta, known_types: &HashSet<String>) -> String {
    match inner {
        TypeMeta::Bool => "boolean[]".to_string(),
        TypeMeta::String | TypeMeta::Guid => "string[]".to_string(),
        // byte[] returns: Node Buffer (Uint8Array subclass) — see convert_array_return.
        TypeMeta::U8 => "Buffer".to_string(),
        TypeMeta::I8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::Char16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::F32
        | TypeMeta::F64
        | TypeMeta::Enum { .. } => "number[]".to_string(),
        TypeMeta::I64 | TypeMeta::U64 => "bigint[]".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "number[]".to_string(),
        TypeMeta::Struct { name, .. } => ts_array_type(name),
        TypeMeta::Object => "Array<DynWinRtValue | null>".to_string(),
        TypeMeta::RuntimeClass { name, .. } if known_types.contains(name) => ts_array_type(name),
        TypeMeta::Interface { name, .. } if known_types.contains(name) => ts_array_type(name),
        _ => "DynWinRtValue[]".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_types_preserve_element_precedence() {
        for (element, expected) in [
            ("number", "number[]"),
            ("string", "string[]"),
            (
                "number | null | IReference_UInt32",
                "(number | null | IReference_UInt32)[]",
            ),
            ("Observable & Vector", "(Observable & Vector)[]"),
            ("(number | null)", "(number | null)[]"),
            ("(number | null)[]", "(number | null)[][]"),
            ("Map<string, number | null>", "Map<string, number | null>[]"),
            ("[number | null, string]", "[number | null, string][]"),
            ("{ value: number | null }", "{ value: number | null }[]"),
        ] {
            assert_eq!(ts_array_type(element), expected);
        }
        assert_eq!(
            ts_array_type(&ts_array_type("number | null")),
            "(number | null)[][]"
        );
    }

    #[test]
    fn array_parameters_keep_byte_and_scalar_projections() {
        for (element, expected) in [
            (TypeMeta::U8, "Uint8Array | number[]"),
            (TypeMeta::U32, "number[]"),
            (TypeMeta::String, "string[]"),
        ] {
            assert_eq!(
                ts_param_type_dts(
                    &Default::default(),
                    &TypeMeta::Array(Box::new(element)),
                    &HashSet::new(),
                ),
                expected,
            );
        }
    }
}
