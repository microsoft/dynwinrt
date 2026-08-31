// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;

use super::JavaScriptProjectionContext;
use crate::codegen::winrt::shared::imports::ireference_inner_type;
use crate::types::TypeMeta;

// ======================================================================
// TypeScript type annotation helpers
// ======================================================================

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

/// DTS-specific parameter type: for collection types, also accept the JS-native equivalent.
/// e.g. `IVectorView_Foo | Foo[]`, `IMap_String_Bar | Map<string, Bar>`
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
        let elem_ts = ts_param_type_safe(context, inner, known);
        return format!("{}[]", elem_ts);
    }
    if let TypeMeta::Parameterized {
        name, piid, args, ..
    } = typ
    {
        let base = ts_param_type_safe(context, typ, known);
        // IVector<T>, IVectorView<T>, IIterable<T> → also accept T[]
        if is_vector_like_piid(piid) || is_vector_like_name(name) {
            if let Some(elem) = args.first() {
                let elem_ts = ts_param_type_safe(context, elem, known);
                return format!("{} | {}[]", base, elem_ts);
            }
        }
        // IMap<K,V>, IMapView<K,V> → also accept Map<K,V>
        if is_map_like_piid(piid) || is_map_like_name(name) {
            if args.len() == 2 {
                let k_ts = ts_param_type_safe(context, &args[0], known);
                let v_ts = ts_param_type_safe(context, &args[1], known);
                return format!("{} | Map<{}, {}>", base, k_ts, v_ts);
            }
        }
        return base;
    }
    ts_param_type_safe(context, typ, known)
}

const PIID_IVECTOR: &str = "913337e9-11a1-4345-a3a2-4e7f956e222d";
const PIID_IVECTOR_VIEW: &str = "bbe1fa4c-b0e3-4583-baef-1f1b2e483e56";
const PIID_IITERABLE: &str = "faa585ea-6214-4217-afda-7f46de5869b3";
const PIID_IMAP: &str = "3c2925fe-8519-45c1-aa79-197b6718c1c1";
const PIID_IMAP_VIEW: &str = "e480ce40-a338-4ada-adcf-272272e48cb9";

fn is_vector_like_piid(piid: &str) -> bool {
    piid == PIID_IVECTOR || piid == PIID_IVECTOR_VIEW || piid == PIID_IITERABLE
}

fn is_vector_like_name(name: &str) -> bool {
    name == "IVector" || name == "IVectorView" || name == "IIterable"
}

fn is_map_like_piid(piid: &str) -> bool {
    piid == PIID_IMAP || piid == PIID_IMAP_VIEW
}

fn is_map_like_name(name: &str) -> bool {
    name == "IMap" || name == "IMapView"
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
        TypeMeta::Struct { name, .. } => format!("{}[]", name),
        TypeMeta::RuntimeClass { name, .. } if known_types.contains(name) => format!("{}[]", name),
        TypeMeta::Interface { name, .. } if known_types.contains(name) => format!("{}[]", name),
        _ => "DynWinRtValue[]".to_string(),
    }
}
