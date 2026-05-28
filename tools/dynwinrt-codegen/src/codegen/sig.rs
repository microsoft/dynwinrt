// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Method signature construction, type expression expansion, argument
//! wrapping, return conversion, and interface registration helpers for both
//! TypeScript and Python codegen.

use std::collections::HashSet;

use crate::meta::{InterfaceMeta, MethodMeta, ParamDirection};
use crate::types::TypeMeta;

use super::naming::{to_camel_case, to_snake_case};

// ======================================================================
// Method signature builder (TypeScript)
// ======================================================================

/// Build a `new DynWinRtMethodSig().addIn(...)...addOut(...)` expression.
pub(crate) fn build_method_sig(method: &MethodMeta) -> String {
    let mut parts = Vec::new();

    // In params
    for param in &method.params {
        if param.direction == ParamDirection::In {
            parts.push(format!(".addIn({})", ts_dynwinrt_type(&param.typ)));
        }
    }

    // Out params (explicit [out] parameters in method signature)
    for param in &method.params {
        if param.direction == ParamDirection::Out {
            parts.push(format!(".addOut({})", ts_dynwinrt_type(&param.typ)));
        } else if param.direction == ParamDirection::OutFill {
            parts.push(format!(".addOutFill({})", ts_dynwinrt_type(&param.typ)));
        }
    }

    // Return type (WinRT return value = [out, retval])
    if let Some(ref return_type) = method.return_type {
        parts.push(format!(".addOut({})", ts_dynwinrt_type(return_type)));
    }

    if parts.is_empty() {
        "new DynWinRtMethodSig()".to_string()
    } else {
        format!("new DynWinRtMethodSig(){}", parts.join(""))
    }
}

// ======================================================================
// Type expression: recursive expansion (TypeScript)
// ======================================================================

/// Map a TypeMeta to a fully-expanded `DynWinRtType.*()` expression.
/// Recursively expands all compound types to leaf primitives.
pub(crate) fn ts_dynwinrt_type(typ: &TypeMeta) -> String {
    match typ {
        // Primitives
        TypeMeta::Bool => "DynWinRtType.boolType()".to_string(),
        TypeMeta::I8 => "DynWinRtType.i8Type()".to_string(),
        TypeMeta::I16 => "DynWinRtType.i16()".to_string(),
        TypeMeta::Char16 => "DynWinRtType.u16()".to_string(),
        TypeMeta::I32 => "DynWinRtType.i32()".to_string(),
        TypeMeta::U8 => "DynWinRtType.u8()".to_string(),
        TypeMeta::U16 => "DynWinRtType.u16()".to_string(),
        TypeMeta::U32 => "DynWinRtType.u32()".to_string(),
        TypeMeta::I64 => "DynWinRtType.i64()".to_string(),
        TypeMeta::U64 => "DynWinRtType.u64()".to_string(),
        TypeMeta::F32 => "DynWinRtType.f32()".to_string(),
        TypeMeta::F64 => "DynWinRtType.f64()".to_string(),

        // Strings
        TypeMeta::String => "DynWinRtType.hstring()".to_string(),

        // GUID — native type in dynwinrt
        TypeMeta::Guid => "DynWinRtType.guidType()".to_string(),

        // Generic object
        TypeMeta::Object => "DynWinRtType.object()".to_string(),

        // Interface — use interface(IID) if available
        TypeMeta::Interface { iid, .. } if !iid.is_empty() => {
            format!("DynWinRtType.interface(WinGuid.parse('{}'))", iid)
        }
        TypeMeta::Interface { .. } => "DynWinRtType.object()".to_string(),

        // RuntimeClass — runtimeClass(fullName, defaultIID)
        TypeMeta::RuntimeClass { namespace, name, default_iid } => {
            let full_name = format!("{}.{}", namespace, name);
            if !default_iid.is_empty() {
                format!(
                    "DynWinRtType.runtimeClass('{}', WinGuid.parse('{}'))",
                    full_name, default_iid
                )
            } else {
                "DynWinRtType.object()".to_string()
            }
        }

        // Delegate — COM pointer
        TypeMeta::Delegate { .. } => "DynWinRtType.object()".to_string(),

        // Async patterns — recursively expand inner types
        TypeMeta::AsyncOperation(inner) => {
            format!("DynWinRtType.iAsyncOperation({})", ts_dynwinrt_type(inner))
        }
        TypeMeta::AsyncOperationWithProgress(result, progress) => {
            format!("DynWinRtType.iAsyncOperationWithProgress({}, {})",
                ts_dynwinrt_type(result), ts_dynwinrt_type(progress))
        }
        TypeMeta::AsyncAction => {
            "DynWinRtType.iAsyncAction()".to_string()
        }
        TypeMeta::AsyncActionWithProgress(progress) => {
            format!("DynWinRtType.iAsyncActionWithProgress({})", ts_dynwinrt_type(progress))
        }

        // Struct — named for correct IID signature, recursively expand fields.
        // HResult is special-cased: WinRT exposes it as a single-field struct,
        // but the runtime has a dedicated HResult kind whose deserialized value
        // (WinRTValue::HResult) is the only one .toNumber() / packStruct
        // helpers know how to unwrap. Emitting structType(...) here would cause
        // the napi binding to deliver a WinRTValue::Struct and panic on read.
        TypeMeta::Struct { name, .. } if name == "HResult" => {
            "DynWinRtType.hresult()".to_string()
        }
        TypeMeta::Struct { namespace, name, fields } => {
            let full_name = format!("{}.{}", namespace, name);
            let field_types: Vec<String> = fields.iter()
                .map(|f| ts_dynwinrt_type(&f.typ))
                .collect();
            format!("DynWinRtType.structType('{}', [{}])", full_name, field_types.join(", "))
        }

        // Array — recursively expand element type
        TypeMeta::Array(inner) => {
            format!("DynWinRtType.arrayType({})", ts_dynwinrt_type(inner))
        }

        // Enum — named for correct IID signature, with member values
        TypeMeta::Enum { namespace, name, members, .. } => {
            let full_name = format!("{}.{}", namespace, name);
            if members.is_empty() {
                format!("DynWinRtType.enumType('{}')", full_name)
            } else {
                let names: Vec<String> = members.iter().map(|m| format!("'{}'", m.name)).collect();
                let values: Vec<String> = members.iter().map(|m| m.value.to_string()).collect();
                format!("DynWinRtType.enumType('{}', [{}], [{}])",
                    full_name, names.join(", "), values.join(", "))
            }
        }

        // Parameterized — preserve generic type info for IID computation
        TypeMeta::Parameterized { piid, args, .. } => {
            if piid.is_empty() {
                "DynWinRtType.object()".to_string()
            } else {
                let arg_types: Vec<String> = args.iter().map(|a| ts_dynwinrt_type(a)).collect();
                format!("DynWinRtType.parameterized(WinGuid.parse('{}'), [{}])", piid, arg_types.join(", "))
            }
        }
    }
}

// ======================================================================
// Argument wrapping (TypeScript)
// ======================================================================

pub(crate) fn build_args_expr(in_params: &[&crate::meta::ParamMeta]) -> String {
    in_params.iter()
        .map(|p| wrap_arg(&to_camel_case(&p.name), &p.typ))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn wrap_arg(name: &str, typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::String => format!("DynWinRtValue.hstring({})", name),
        TypeMeta::Bool => format!("DynWinRtValue.boolValue({})", name),
        TypeMeta::I32 | TypeMeta::Enum { .. }
        | TypeMeta::I8 | TypeMeta::U8 | TypeMeta::Char16 => {
            format!("DynWinRtValue.i32({})", name)
        }
        TypeMeta::U32 => format!("DynWinRtValue.u32({})", name),
        TypeMeta::I16 => format!("DynWinRtValue.i16({})", name),
        TypeMeta::U16 => format!("DynWinRtValue.u16({})", name),
        TypeMeta::I64 => format!("DynWinRtValue.i64({})", name),
        TypeMeta::U64 => format!("DynWinRtValue.u64({})", name),
        TypeMeta::F32 => format!("DynWinRtValue.f32({})", name),
        TypeMeta::F64 => format!("DynWinRtValue.f64({})", name),
        TypeMeta::Guid => format!("DynWinRtValue.guid(WinGuid.parse({}))", name),
        TypeMeta::RuntimeClass { .. } | TypeMeta::Object | TypeMeta::Interface { .. }
        | TypeMeta::Delegate { .. } => {
            format!("_unwrap({})", name)
        }
        TypeMeta::Parameterized { piid, args, name: pname, .. } => {
            // For vector-like collections, auto-wrap JS arrays at runtime.
            //
            // createVector returns a SingleThreadedVector whose identity vtable
            // is IIterable<T> (sibling of IVector/IVectorView, not parent). If
            // the method parameter is IVector<T> or IVectorView<T>, we must QI
            // to that specific interface so the correct vtable pointer (with
            // the right method slots) is forwarded to WinRT — otherwise WinRT
            // reads the wrong vtable slots and the renderer crashes natively.
            if is_vector_like(piid, pname) {
                if let Some(elem) = args.first() {
                    let elem_type = ts_dynwinrt_type(elem);
                    let item_wrap = vector_item_wrap_expr("_i", elem);
                    let target_iid_expr = format!("{}.iid()", ts_dynwinrt_type(typ));
                    return format!(
                        "(Array.isArray({name}) ? DynWinRtValue.createVector({name}.map(_i => {item_wrap}), {elem_type}).cast({target_iid_expr}) : _unwrap({name}).cast({target_iid_expr}))"
                    );
                }
            }
            // For map-like collections, auto-wrap JS Map at runtime.
            // Same vtable-mismatch concern as vectors: createMap returns a
            // SingleThreadedMap whose identity vtable is IIterable
            // <IKeyValuePair<K,V>>; we must QI to IMap<K,V> or IMapView<K,V>.
            if is_map_like(piid, pname) {
                if args.len() == 2 {
                    let key_type = ts_dynwinrt_type(&args[0]);
                    let val_type = ts_dynwinrt_type(&args[1]);
                    let k_wrap = vector_item_wrap_expr("_k", &args[0]);
                    let v_wrap = vector_item_wrap_expr("_v", &args[1]);
                    let target_iid_expr = format!("{}.iid()", ts_dynwinrt_type(typ));
                    return format!(
                        "({name} instanceof Map ? DynWinRtValue.createMap([...{name}.keys()].map(_k => {k_wrap}), [...{name}.values()].map(_v => {v_wrap}), {key_type}, {val_type}).cast({target_iid_expr}) : _unwrap({name}).cast({target_iid_expr}))"
                    );
                }
            }
            format!("_unwrap({})", name)
        }
        TypeMeta::Array(inner) => {
            // Accept both DynWinRtArray (.toValue()) and plain JS array.
            // For primitive arrays, use typed DynWinRtArray constructors.
            // For object arrays, use createVector which WinRT can consume as PassArray.
            // Special: byte[] (U8) also accepts Uint8Array — far more memory-efficient
            // than `new Array(N).fill(0)` (8 bytes/elem) for large pixel buffers.
            if matches!(inner.as_ref(), TypeMeta::U8) {
                return format!(
                    "({name} instanceof Uint8Array ? DynWinRtArray.fromUint8Array({name}).toValue() : Array.isArray({name}) ? DynWinRtArray.fromU8Values({name}).toValue() : {name}.toValue())"
                );
            }
            let from_array_expr = match inner.as_ref() {
                TypeMeta::I8 => format!("DynWinRtArray.fromI8Values({name})"),
                TypeMeta::U8 => format!("DynWinRtArray.fromU8Values({name})"),
                TypeMeta::I16 => format!("DynWinRtArray.fromI16Values({name})"),
                TypeMeta::U16 | TypeMeta::Char16 => format!("DynWinRtArray.fromU16Values({name})"),
                TypeMeta::I32 | TypeMeta::Enum { .. } => format!("DynWinRtArray.fromI32Values({name})"),
                TypeMeta::U32 => format!("DynWinRtArray.fromU32Values({name})"),
                TypeMeta::I64 => format!("DynWinRtArray.fromI64Values({name})"),
                TypeMeta::U64 => format!("DynWinRtArray.fromU64Values({name})"),
                TypeMeta::F32 => format!("DynWinRtArray.fromF32Values({name})"),
                TypeMeta::F64 => format!("DynWinRtArray.fromF64Values({name})"),
                TypeMeta::String => format!("DynWinRtArray.fromStringValues({name})"),
                _ => {
                    // Object types: wrap via DynWinRtArray.fromObjectValues so the
                    // ABI receives a native WinRTValue::Array (PassArray) rather
                    // than an IVector COM object. Required for `T[]` in-params
                    // where T is a runtime class or interface.
                    let elem_type = ts_dynwinrt_type(inner);
                    return format!(
                        "(Array.isArray({name}) ? DynWinRtArray.fromObjectValues({name}.map(_i => _unwrap(_i)), {elem_type}).toValue() : _unwrap({name}))"
                    );
                }
            };
            format!("(Array.isArray({name}) ? {from_array_expr}.toValue() : {name}.toValue())")
        }
        TypeMeta::Struct { name: struct_name, .. } if struct_name == "HResult" => {
            format!("DynWinRtValue.i32({})", name)
        }
        TypeMeta::Struct { name: struct_name, .. } => {
            format!("_pack{}({}).toValue()", struct_name, name)
        }
        _ => name.to_string(),
    }
}

fn is_vector_like(piid: &str, name: &str) -> bool {
    const PIID_IVECTOR: &str = "913337e9-11a1-4345-a3a2-4e7f956e222d";
    const PIID_IVECTOR_VIEW: &str = "bbe1fa4c-b0e3-4583-baef-1f1b2e483e56";
    const PIID_IITERABLE: &str = "faa585ea-6214-4217-afda-7f46de5869b3";
    piid == PIID_IVECTOR || piid == PIID_IVECTOR_VIEW || piid == PIID_IITERABLE
        || name == "IVector" || name == "IVectorView" || name == "IIterable"
}

fn is_map_like(piid: &str, name: &str) -> bool {
    const PIID_IMAP: &str = "3c2925fe-8519-45c1-aa79-197b6718c1c1";
    const PIID_IMAP_VIEW: &str = "e480ce40-a338-4ada-adcf-272272e48cb9";
    piid == PIID_IMAP || piid == PIID_IMAP_VIEW
        || name == "IMap" || name == "IMapView"
}

/// Generate the JS expression to wrap a single element for createVector/createMap.
/// For structs, uses pack function; for primitives, wraps as DynWinRtValue; for objects, _unwrap.
fn vector_item_wrap_expr(var: &str, elem: &TypeMeta) -> String {
    match elem {
        TypeMeta::Struct { name, .. } if name != "HResult" => {
            format!("_pack{}({}).toValue()", name, var)
        }
        TypeMeta::String => format!("DynWinRtValue.hstring({})", var),
        TypeMeta::Bool => format!("DynWinRtValue.boolValue({})", var),
        TypeMeta::I32 | TypeMeta::Enum { .. }
        | TypeMeta::I8 | TypeMeta::U8 | TypeMeta::Char16 => {
            format!("DynWinRtValue.i32({})", var)
        }
        TypeMeta::U32 => format!("DynWinRtValue.u32({})", var),
        TypeMeta::I16 => format!("DynWinRtValue.i16({})", var),
        TypeMeta::U16 => format!("DynWinRtValue.u16({})", var),
        TypeMeta::I64 => format!("DynWinRtValue.i64({})", var),
        TypeMeta::U64 => format!("DynWinRtValue.u64({})", var),
        TypeMeta::F32 => format!("DynWinRtValue.f32({})", var),
        TypeMeta::F64 => format!("DynWinRtValue.f64({})", var),
        _ => format!("_unwrap({})", var),
    }
}

// ======================================================================
// Return conversion (TypeScript)
// ======================================================================

/// Resolve a type name, using `_m_X.X` for deferred (lazy module ref) imports.
pub(crate) fn resolve_type_name(name: &str, deferred: &HashSet<String>) -> String {
    if deferred.contains(name) {
        format!("_m_{0}.{0}", name)
    } else {
        name.to_string()
    }
}

/// Convert an array return expression to the appropriate JS array type.
pub(crate) fn convert_array_return(arr_expr: &str, inner: &TypeMeta, known_types: &HashSet<String>, deferred: &HashSet<String>) -> String {
    match inner {
        TypeMeta::I8 => format!("{}.toI8Vec()", arr_expr),
        // U8 returns: hand back a Node Buffer (Uint8Array view), avoiding the
        // ~8x V8 heap blow-up of an Array<number>. Buffer is assignment-
        // compatible with Uint8Array and works with `Array.from(...)`,
        // `Buffer.from(...)`, indexing, and `.length`.
        TypeMeta::U8 => format!("{}.toBuffer()", arr_expr),
        TypeMeta::I16 => format!("{}.toI16Vec()", arr_expr),
        TypeMeta::U16 | TypeMeta::Char16 => format!("{}.toU16Vec()", arr_expr),
        TypeMeta::I32 | TypeMeta::Enum { .. } => format!("{}.toI32Vec()", arr_expr),
        TypeMeta::U32 => format!("{}.toU32Vec()", arr_expr),
        TypeMeta::I64 => format!("{}.toI64Vec()", arr_expr),
        TypeMeta::U64 => format!("{}.toU64Vec()", arr_expr),
        TypeMeta::F32 => format!("{}.toF32Vec()", arr_expr),
        TypeMeta::F64 => format!("{}.toF64Vec()", arr_expr),
        TypeMeta::Bool => format!("{}.toValues().map(v => v.toBool())", arr_expr),
        TypeMeta::String => format!("{}.toStringVec()", arr_expr),
        TypeMeta::Guid => format!("{}.toValues().map(v => v.toString())", arr_expr),
        TypeMeta::Struct { name, .. } if name == "HResult" => format!("{}.toI32Vec()", arr_expr),
        TypeMeta::Struct { name, .. } => format!("{}.toValues().map(v => _unpack{}(v))", arr_expr, name),
        TypeMeta::RuntimeClass { name, .. } if known_types.contains(name) => {
            let r = resolve_type_name(name, deferred);
            format!("{}.toValues().map(v => new {}(v))", arr_expr, r)
        }
        TypeMeta::Interface { name, .. } if known_types.contains(name) => {
            let r = resolve_type_name(name, deferred);
            format!("{}.toValues().map(v => new {}(v))", arr_expr, r)
        }
        _ => format!("{}.toValues()", arr_expr),
    }
}

pub(crate) fn convert_return(expr: &str, return_type: Option<&TypeMeta>, is_async: bool, known_types: &HashSet<String>, deferred: &HashSet<String>) -> String {
    if is_async {
        let inner_type = match return_type {
            Some(TypeMeta::AsyncOperation(inner)) => Some(inner.as_ref()),
            Some(TypeMeta::AsyncOperationWithProgress(inner, _)) => Some(inner.as_ref()),
            _ => None,
        };
        let awaited = format!("(await {}.toPromise())", expr);
        return convert_return(&awaited, inner_type, false, known_types, deferred);
    }
    match return_type {
        Some(TypeMeta::String) | Some(TypeMeta::Guid) => format!("{}.toString()", expr),
        Some(TypeMeta::I8 | TypeMeta::U8 | TypeMeta::I16 | TypeMeta::U16 | TypeMeta::Char16
            | TypeMeta::I32 | TypeMeta::U32) => format!("{}.toNumber()", expr),
        Some(TypeMeta::I64 | TypeMeta::U64) => format!("{}.toI64()", expr),
        Some(TypeMeta::F32 | TypeMeta::F64) => format!("{}.toF64()", expr),
        Some(TypeMeta::Bool) => format!("{}.toBool()", expr),
        Some(TypeMeta::Enum { .. }) => format!("{}.toNumber()", expr),
        Some(TypeMeta::RuntimeClass { name, .. }) if known_types.contains(name) => {
            let r = resolve_type_name(name, deferred);
            format!("new {}({})", r, expr)
        }
        Some(TypeMeta::Struct { name, .. }) if name == "HResult" => format!("{}.toNumber()", expr),
        Some(TypeMeta::Struct { name, .. }) => format!("_unpack{}({})", name, expr),
        Some(TypeMeta::Delegate { .. }) => expr.to_string(),
        Some(TypeMeta::Interface { name, .. }) if known_types.contains(name) => {
            let r = resolve_type_name(name, deferred);
            format!("new {}({})", r, expr)
        }
        Some(TypeMeta::Parameterized { name, args, .. }) => {
            let concrete = crate::meta::make_parameterized_name(name, args);
            if known_types.contains(&concrete) {
                let r = resolve_type_name(&concrete, deferred);
                format!("new {}({})", r, expr)
            } else {
                expr.to_string()
            }
        }
        Some(TypeMeta::Array(inner)) => {
            let arr_expr = format!("{}.asArray()", expr);
            convert_array_return(&arr_expr, inner, known_types, deferred)
        }
        _ => expr.to_string(),
    }
}

// ======================================================================
// Interface registration helper (TypeScript)
// ======================================================================

/// Generate a `const <var_name> = DynWinRtType.registerInterface(...)` block.
/// `var_name` controls the JS variable name (e.g. `"_IFoo"` for class-internal use).
pub(crate) fn generate_interface_registration(iface: &InterfaceMeta, var_name: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("const {} = DynWinRtType.registerInterface(\n", var_name));
    out.push_str(&format!("    \"{}\", IID_{})\n", iface.name, iface.name));
    for method in &iface.methods {
        out.push_str(&format!(
            "    .addMethod(\"{}\", {})\n",
            method.name,
            build_method_sig(method)
        ));
    }
    trim_trailing_newline_add_semicolon(&mut out);
    out
}

pub(crate) fn trim_trailing_newline_add_semicolon(out: &mut String) {
    if out.ends_with(")\n") {
        out.truncate(out.len() - 1);
        out.push_str(";\n");
    }
}

// ======================================================================
// Python type expression
// ======================================================================

/// Map a TypeMeta to a `DynWinRTType.*()` Python expression.
pub(crate) fn py_dynwinrt_type(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => "DynWinRTType.bool_type()".to_string(),
        TypeMeta::I8 => "DynWinRTType.i8_type()".to_string(),
        TypeMeta::I16 => "DynWinRTType.i16_type()".to_string(),
        TypeMeta::Char16 => "DynWinRTType.char16()".to_string(),
        TypeMeta::I32 => "DynWinRTType.i32_type()".to_string(),
        TypeMeta::U8 => "DynWinRTType.u8_type()".to_string(),
        TypeMeta::U16 => "DynWinRTType.u16_type()".to_string(),
        TypeMeta::U32 => "DynWinRTType.u32_type()".to_string(),
        TypeMeta::I64 => "DynWinRTType.i64_type()".to_string(),
        TypeMeta::U64 => "DynWinRTType.u64_type()".to_string(),
        TypeMeta::F32 => "DynWinRTType.f32_type()".to_string(),
        TypeMeta::F64 => "DynWinRTType.f64_type()".to_string(),
        TypeMeta::String => "DynWinRTType.hstring()".to_string(),
        TypeMeta::Guid => "DynWinRTType.guid_type()".to_string(),
        TypeMeta::Object => "DynWinRTType.object()".to_string(),
        TypeMeta::Interface { iid, .. } if !iid.is_empty() => {
            format!("DynWinRTType.interface(WinGUID.parse('{}'))", iid)
        }
        TypeMeta::Interface { .. } => "DynWinRTType.object()".to_string(),
        TypeMeta::RuntimeClass { namespace, name, default_iid } => {
            let full_name = format!("{}.{}", namespace, name);
            if !default_iid.is_empty() {
                format!(
                    "DynWinRTType.runtime_class('{}', WinGUID.parse('{}'))",
                    full_name, default_iid
                )
            } else {
                "DynWinRTType.object()".to_string()
            }
        }
        TypeMeta::Delegate { .. } => "DynWinRTType.object()".to_string(),
        TypeMeta::AsyncOperation(inner) => {
            format!("DynWinRTType.i_async_operation({})", py_dynwinrt_type(inner))
        }
        TypeMeta::AsyncOperationWithProgress(result, progress) => {
            format!("DynWinRTType.i_async_operation_with_progress({}, {})",
                py_dynwinrt_type(result), py_dynwinrt_type(progress))
        }
        TypeMeta::AsyncAction => "DynWinRTType.i_async_action()".to_string(),
        TypeMeta::AsyncActionWithProgress(progress) => {
            format!("DynWinRTType.i_async_action_with_progress({})", py_dynwinrt_type(progress))
        }
        TypeMeta::Struct { name, .. } if name == "HResult" => {
            "DynWinRTType.hresult()".to_string()
        }
        TypeMeta::Struct { namespace, name, fields } => {
            let full_name = format!("{}.{}", namespace, name);
            let field_types: Vec<String> = fields.iter()
                .map(|f| py_dynwinrt_type(&f.typ))
                .collect();
            format!("DynWinRTType.struct_type('{}', [{}])", full_name, field_types.join(", "))
        }
        TypeMeta::Array(inner) => {
            format!("DynWinRTType.array_type({})", py_dynwinrt_type(inner))
        }
        TypeMeta::Enum { namespace, name, members, .. } => {
            let full_name = format!("{}.{}", namespace, name);
            if members.is_empty() {
                format!("DynWinRTType.enum_type('{}')", full_name)
            } else {
                let names: Vec<String> = members.iter().map(|m| format!("'{}'", m.name)).collect();
                let values: Vec<String> = members.iter().map(|m| m.value.to_string()).collect();
                format!("DynWinRTType.enum_type('{}', [{}], [{}])",
                    full_name, names.join(", "), values.join(", "))
            }
        }
        TypeMeta::Parameterized { piid, args, .. } => {
            if piid.is_empty() {
                "DynWinRTType.object()".to_string()
            } else {
                let arg_types: Vec<String> = args.iter().map(|a| py_dynwinrt_type(a)).collect();
                format!("DynWinRTType.parameterized(WinGUID.parse('{}'), [{}])", piid, arg_types.join(", "))
            }
        }
    }
}

/// Build a `DynWinRTMethodSig().add_in(...)...` Python expression.
pub(crate) fn py_build_method_sig(method: &MethodMeta) -> String {
    let mut parts = Vec::new();

    for param in &method.params {
        if param.direction == ParamDirection::In {
            parts.push(format!(".add_in({})", py_dynwinrt_type(&param.typ)));
        }
    }
    for param in &method.params {
        if param.direction == ParamDirection::Out {
            parts.push(format!(".add_out({})", py_dynwinrt_type(&param.typ)));
        } else if param.direction == ParamDirection::OutFill {
            parts.push(format!(".add_out_fill({})", py_dynwinrt_type(&param.typ)));
        }
    }
    if let Some(ref return_type) = method.return_type {
        parts.push(format!(".add_out({})", py_dynwinrt_type(return_type)));
    }

    if parts.is_empty() {
        "DynWinRTMethodSig()".to_string()
    } else {
        format!("DynWinRTMethodSig(){}", parts.join(""))
    }
}

/// Wrap a Python variable name into a DynWinRTValue expression.
pub(crate) fn py_wrap_arg(name: &str, typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::String => format!("DynWinRTValue.from_hstring({})", name),
        TypeMeta::Bool => format!("DynWinRTValue.from_bool({})", name),
        TypeMeta::I32 | TypeMeta::U32 | TypeMeta::Enum { .. }
        | TypeMeta::I8 | TypeMeta::U8 | TypeMeta::I16 | TypeMeta::U16
        | TypeMeta::Char16 => {
            format!("DynWinRTValue.from_i32({})", name)
        }
        TypeMeta::I64 | TypeMeta::U64 => format!("DynWinRTValue.from_i64({})", name),
        TypeMeta::F32 => format!("DynWinRTValue.from_f32({})", name),
        TypeMeta::F64 => format!("DynWinRTValue.from_f64({})", name),
        TypeMeta::Guid => format!("DynWinRTValue.from_guid({})", name),
        TypeMeta::RuntimeClass { .. } | TypeMeta::Object | TypeMeta::Interface { .. }
        | TypeMeta::Parameterized { .. } | TypeMeta::Delegate { .. } => {
            format!("getattr({}, '_obj', {})", name, name)
        }
        TypeMeta::Array(_) => format!("{}.to_value()", name),
        TypeMeta::Struct { name: struct_name, .. } if struct_name == "HResult" => {
            format!("DynWinRTValue.from_i32({})", name)
        }
        TypeMeta::Struct { name: struct_name, .. } => {
            format!("_pack_{}({}).to_value()", to_snake_case(struct_name), name)
        }
        _ => name.to_string(),
    }
}

/// Build Python args list expression for method call.
pub(crate) fn py_build_args_expr(in_params: &[&crate::meta::ParamMeta]) -> String {
    in_params.iter()
        .map(|p| py_wrap_arg(&to_snake_case(&p.name), &p.typ))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Convert a Python return expression, given the raw `.call()` result expression.
pub(crate) fn py_convert_return(expr: &str, return_type: Option<&TypeMeta>, is_async: bool, known_types: &HashSet<String>) -> String {
    if is_async {
        let inner_type = match return_type {
            Some(TypeMeta::AsyncOperation(inner)) => Some(inner.as_ref()),
            Some(TypeMeta::AsyncOperationWithProgress(inner, _)) => Some(inner.as_ref()),
            _ => None,
        };
        let waited = format!("{}.wait()", expr);
        return py_convert_return(&waited, inner_type, false, known_types);
    }
    match return_type {
        Some(TypeMeta::String) | Some(TypeMeta::Guid) => format!("{}.to_string()", expr),
        Some(TypeMeta::I8 | TypeMeta::U8 | TypeMeta::I16 | TypeMeta::U16 | TypeMeta::Char16
            | TypeMeta::I32 | TypeMeta::U32) => format!("{}.to_number()", expr),
        Some(TypeMeta::I64 | TypeMeta::U64) => format!("{}.to_i64()", expr),
        Some(TypeMeta::F32 | TypeMeta::F64) => format!("{}.to_f64()", expr),
        Some(TypeMeta::Bool) => format!("{}.to_bool()", expr),
        Some(TypeMeta::Enum { .. }) => format!("{}.to_number()", expr),
        Some(TypeMeta::RuntimeClass { name, .. }) if known_types.contains(name) => {
            format!("{}({})", name, expr)
        }
        Some(TypeMeta::Struct { name, .. }) if name == "HResult" => format!("{}.to_number()", expr),
        Some(TypeMeta::Struct { name, .. }) => format!("_unpack_{}({})", to_snake_case(name), expr),
        Some(TypeMeta::Delegate { .. }) => expr.to_string(),
        Some(TypeMeta::Interface { name, .. }) if known_types.contains(name) => {
            format!("{}({})", name, expr)
        }
        Some(TypeMeta::Parameterized { name, args, .. }) => {
            let concrete = crate::meta::make_parameterized_name(name, args);
            if known_types.contains(&concrete) {
                format!("{}({})", concrete, expr)
            } else {
                expr.to_string()
            }
        }
        Some(TypeMeta::Array(inner)) => {
            let arr_expr = format!("{}.as_array()", expr);
            py_convert_array_return(&arr_expr, inner, known_types)
        }
        _ => expr.to_string(),
    }
}

/// Convert an array return expression to the appropriate Python list.
pub(crate) fn py_convert_array_return(arr_expr: &str, inner: &TypeMeta, known_types: &HashSet<String>) -> String {
    match inner {
        TypeMeta::I8 => format!("{}.to_i8_list()", arr_expr),
        TypeMeta::U8 => format!("{}.to_u8_list()", arr_expr),
        TypeMeta::I16 => format!("{}.to_i16_list()", arr_expr),
        TypeMeta::U16 | TypeMeta::Char16 => format!("{}.to_u16_list()", arr_expr),
        TypeMeta::I32 | TypeMeta::Enum { .. } => format!("{}.to_i32_list()", arr_expr),
        TypeMeta::U32 => format!("{}.to_u32_list()", arr_expr),
        TypeMeta::I64 => format!("{}.to_i64_list()", arr_expr),
        TypeMeta::U64 => format!("{}.to_u64_list()", arr_expr),
        TypeMeta::F32 => format!("{}.to_f32_list()", arr_expr),
        TypeMeta::F64 => format!("{}.to_f64_list()", arr_expr),
        TypeMeta::Bool => format!("[v.to_bool() for v in {}.to_values()]", arr_expr),
        TypeMeta::String => format!("{}.to_string_list()", arr_expr),
        TypeMeta::Guid => format!("[v.to_string() for v in {}.to_values()]", arr_expr),
        TypeMeta::Struct { name, .. } if name == "HResult" => format!("{}.to_i32_list()", arr_expr),
        TypeMeta::Struct { name, .. } => format!("[_unpack_{}(v) for v in {}.to_values()]", to_snake_case(name), arr_expr),
        TypeMeta::RuntimeClass { name, .. } if known_types.contains(name) => {
            format!("[{}(v) for v in {}.to_values()]", name, arr_expr)
        }
        TypeMeta::Interface { name, .. } if known_types.contains(name) => {
            format!("[{}(v) for v in {}.to_values()]", name, arr_expr)
        }
        _ => format!("{}.to_values()", arr_expr),
    }
}

/// Generate a Python `_IFoo = DynWinRTType.register_interface(...)` block.
pub(crate) fn py_generate_interface_registration(iface: &InterfaceMeta, var_name: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("{} = DynWinRTType.register_interface(\n", var_name));
    out.push_str(&format!("    \"{}\", IID_{}) \\\n", iface.name, iface.name));
    for (i, method) in iface.methods.iter().enumerate() {
        let trailing = if i + 1 < iface.methods.len() { " \\" } else { "" };
        out.push_str(&format!(
            "    .add_method(\"{}\", {}){}\n",
            method.name,
            py_build_method_sig(method),
            trailing
        ));
    }
    out
}
