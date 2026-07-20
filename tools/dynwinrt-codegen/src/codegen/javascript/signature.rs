// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! JavaScript method signatures, argument wrapping, and return conversion.

use std::collections::HashSet;

use crate::meta::{InterfaceMeta, MethodMeta, ParamDirection};
use crate::types::TypeMeta;

use super::naming::to_camel_case;

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

fn ts_interface_iid(typ: &TypeMeta) -> Option<String> {
    match typ {
        TypeMeta::Interface { iid, .. } if !iid.is_empty() => {
            Some(format!("WinGuid.parse('{}')", iid))
        }
        TypeMeta::Parameterized { .. } => Some(format!("{}.iid()", ts_dynwinrt_type(typ))),
        _ => None,
    }
}

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
        TypeMeta::RuntimeClass {
            namespace,
            name,
            default_interface,
        } => {
            let full_name = format!("{}.{}", namespace, name);
            if let Some(default_iid) = default_interface.as_deref().and_then(ts_interface_iid) {
                format!(
                    "DynWinRtType.runtimeClass('{}', {})",
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
            format!(
                "DynWinRtType.iAsyncOperationWithProgress({}, {})",
                ts_dynwinrt_type(result),
                ts_dynwinrt_type(progress)
            )
        }
        TypeMeta::AsyncAction => "DynWinRtType.iAsyncAction()".to_string(),
        TypeMeta::AsyncActionWithProgress(progress) => {
            format!(
                "DynWinRtType.iAsyncActionWithProgress({})",
                ts_dynwinrt_type(progress)
            )
        }

        // Struct — named for correct IID signature, recursively expand fields.
        // HResult is special-cased: WinRT exposes it as a single-field struct,
        // but the runtime has a dedicated HResult kind whose deserialized value
        // (WinRTValue::HResult) is the only one .toNumber() / packStruct
        // helpers know how to unwrap. Emitting structType(...) here would cause
        // the napi binding to deliver a WinRTValue::Struct and panic on read.
        TypeMeta::Struct { name, .. } if name == "HResult" => "DynWinRtType.hresult()".to_string(),
        TypeMeta::Struct {
            namespace,
            name,
            fields,
        } => {
            let full_name = format!("{}.{}", namespace, name);
            let field_types: Vec<String> =
                fields.iter().map(|f| ts_dynwinrt_type(&f.typ)).collect();
            format!(
                "DynWinRtType.structType('{}', [{}])",
                full_name,
                field_types.join(", ")
            )
        }

        // Array — recursively expand element type
        TypeMeta::Array(inner) => {
            format!("DynWinRtType.arrayType({})", ts_dynwinrt_type(inner))
        }

        // Enum — named for correct IID signature, with member values
        TypeMeta::Enum {
            namespace,
            name,
            members,
            ..
        } => {
            let full_name = format!("{}.{}", namespace, name);
            if members.is_empty() {
                format!("DynWinRtType.enumType('{}')", full_name)
            } else {
                let names: Vec<String> = members.iter().map(|m| format!("'{}'", m.name)).collect();
                let values: Vec<String> = members.iter().map(|m| m.value.to_string()).collect();
                format!(
                    "DynWinRtType.enumType('{}', [{}], [{}])",
                    full_name,
                    names.join(", "),
                    values.join(", ")
                )
            }
        }

        // Parameterized — preserve generic type info for IID computation
        TypeMeta::Parameterized { piid, args, .. } => {
            if piid.is_empty() {
                "DynWinRtType.object()".to_string()
            } else {
                let arg_types: Vec<String> = args.iter().map(|a| ts_dynwinrt_type(a)).collect();
                format!(
                    "DynWinRtType.parameterized(WinGuid.parse('{}'), [{}])",
                    piid,
                    arg_types.join(", ")
                )
            }
        }
    }
}

// ======================================================================
// Argument wrapping (TypeScript)
// ======================================================================

pub(crate) fn build_args_expr(in_params: &[&crate::meta::ParamMeta]) -> String {
    in_params
        .iter()
        .map(|p| wrap_arg(&to_camel_case(&p.name), &p.typ))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn wrap_arg(name: &str, typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::String => format!("DynWinRtValue.hstring({})", name),
        TypeMeta::Bool => format!("DynWinRtValue.boolValue({})", name),
        TypeMeta::I32 | TypeMeta::Enum { .. } | TypeMeta::I8 | TypeMeta::U8 | TypeMeta::Char16 => {
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
        TypeMeta::RuntimeClass { .. }
        | TypeMeta::Object
        | TypeMeta::Interface { .. }
        | TypeMeta::Delegate { .. } => {
            format!(
                "({0} == null ? DynWinRtValue.nullValue() : _unwrap({0}))",
                name
            )
        }
        TypeMeta::Parameterized {
            piid,
            args,
            name: pname,
            ..
        } => {
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
                TypeMeta::I32 | TypeMeta::Enum { .. } => {
                    format!("DynWinRtArray.fromI32Values({name})")
                }
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
        TypeMeta::Struct {
            name: struct_name, ..
        } if struct_name == "HResult" => {
            format!("DynWinRtValue.i32({})", name)
        }
        TypeMeta::Struct {
            name: struct_name, ..
        } => {
            format!("_pack{}({}).toValue()", struct_name, name)
        }
        _ => name.to_string(),
    }
}

fn is_vector_like(piid: &str, name: &str) -> bool {
    const PIID_IVECTOR: &str = "913337e9-11a1-4345-a3a2-4e7f956e222d";
    const PIID_IVECTOR_VIEW: &str = "bbe1fa4c-b0e3-4583-baef-1f1b2e483e56";
    const PIID_IITERABLE: &str = "faa585ea-6214-4217-afda-7f46de5869b3";
    piid == PIID_IVECTOR
        || piid == PIID_IVECTOR_VIEW
        || piid == PIID_IITERABLE
        || name == "IVector"
        || name == "IVectorView"
        || name == "IIterable"
}

fn is_map_like(piid: &str, name: &str) -> bool {
    const PIID_IMAP: &str = "3c2925fe-8519-45c1-aa79-197b6718c1c1";
    const PIID_IMAP_VIEW: &str = "e480ce40-a338-4ada-adcf-272272e48cb9";
    piid == PIID_IMAP || piid == PIID_IMAP_VIEW || name == "IMap" || name == "IMapView"
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
        TypeMeta::I32 | TypeMeta::Enum { .. } | TypeMeta::I8 | TypeMeta::U8 | TypeMeta::Char16 => {
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

/// Marker prefix for cross-file lazy sibling references in emitted JS body strings.
/// See `render_js::resolve_ref_markers` for how these get resolved to target-specific
/// output shapes (e.g. `__DWRT_REF__X__` → `X` in ESM, `(__get_X())` in CJS-lazy).
pub const REF_MARKER_PREFIX: &str = "__DWRT_REF__";
pub const REF_MARKER_SUFFIX: &str = "__";

/// Wrap a bare class/struct/interface identifier in the marker syntax that
/// render_js later resolves per target. Callers use this whenever they emit a
/// cross-file type name into a pre-computed body string, so the renderer can
/// dispatch by target without doing JS tokenization.
pub fn ref_marker(name: &str) -> String {
    format!("{}{}{}", REF_MARKER_PREFIX, name, REF_MARKER_SUFFIX)
}

/// Resolve a type name for embedding in generated JS. Always wraps the name in
/// a `__DWRT_REF__<name>__` marker that the render layer translates per target
/// (real identifier for ESM or same-file self references, `(__get_X())` for
/// CJS cross-file lazy references). The renderer does the sibling-vs-self
/// disambiguation via the file's import set — this function is deliberately
/// context-free so it doesn't need `imported_names` plumbed through every
/// projection call site.
///
/// The `deferred` parameter is preserved for caller compatibility but is
/// unused for JavaScript emission.
pub(crate) fn resolve_type_name(name: &str, _deferred: &HashSet<String>) -> String {
    ref_marker(name)
}

fn wrap_nullable_class_return(expr: &str, wrapper: &str) -> String {
    format!(
        "((v) => v.isNull() ? null : {}._fromNative(v))({})",
        wrapper, expr
    )
}

fn wrap_nullable_interface_return(expr: &str, wrapper: &str) -> String {
    format!("((v) => v.isNull() ? null : new {}(v))({})", wrapper, expr)
}

fn unwrap_nullable_return(expr: &str) -> String {
    format!("((v) => v.isNull() ? null : v)({})", expr)
}

/// Convert an array return expression to the appropriate JS array type.
pub(crate) fn convert_array_return(
    arr_expr: &str,
    inner: &TypeMeta,
    known_types: &HashSet<String>,
    deferred: &HashSet<String>,
) -> String {
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
        TypeMeta::Struct { name, .. } => {
            format!("{}.toValues().map(v => _unpack{}(v))", arr_expr, name)
        }
        TypeMeta::RuntimeClass { name, .. } if known_types.contains(name) => {
            let r = resolve_type_name(name, deferred);
            format!(
                "{}.toValues().map(v => v.isNull() ? null : {}._fromNative(v))",
                arr_expr, r
            )
        }
        TypeMeta::Interface { name, .. } if known_types.contains(name) => {
            let r = resolve_type_name(name, deferred);
            format!(
                "{}.toValues().map(v => v.isNull() ? null : new {}(v))",
                arr_expr, r
            )
        }
        TypeMeta::Parameterized { name, args, .. } => {
            let concrete = crate::meta::make_parameterized_name(name, args);
            if known_types.contains(&concrete) {
                let r = resolve_type_name(&concrete, deferred);
                format!(
                    "{}.toValues().map(v => v.isNull() ? null : new {}(v))",
                    arr_expr, r
                )
            } else {
                format!("{}.toValues().map(v => v.isNull() ? null : v)", arr_expr)
            }
        }
        TypeMeta::Object
        | TypeMeta::Delegate { .. }
        | TypeMeta::RuntimeClass { .. }
        | TypeMeta::Interface { .. } => {
            format!("{}.toValues().map(v => v.isNull() ? null : v)", arr_expr)
        }
        _ => format!("{}.toValues()", arr_expr),
    }
}

pub(crate) fn convert_return(
    expr: &str,
    return_type: Option<&TypeMeta>,
    is_async: bool,
    known_types: &HashSet<String>,
    deferred: &HashSet<String>,
) -> String {
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
        Some(
            TypeMeta::I8
            | TypeMeta::U8
            | TypeMeta::I16
            | TypeMeta::U16
            | TypeMeta::Char16
            | TypeMeta::I32
            | TypeMeta::U32,
        ) => format!("{}.toNumber()", expr),
        Some(TypeMeta::I64 | TypeMeta::U64) => format!("{}.toI64()", expr),
        Some(TypeMeta::F32 | TypeMeta::F64) => format!("{}.toF64()", expr),
        Some(TypeMeta::Bool) => format!("{}.toBool()", expr),
        Some(TypeMeta::Enum { .. }) => format!("{}.toNumber()", expr),
        Some(TypeMeta::RuntimeClass { name, .. }) if known_types.contains(name) => {
            let r = resolve_type_name(name, deferred);
            wrap_nullable_class_return(expr, &r)
        }
        Some(TypeMeta::Struct { name, .. }) if name == "HResult" => format!("{}.toNumber()", expr),
        Some(TypeMeta::Struct { name, .. }) => format!("_unpack{}({})", name, expr),
        Some(TypeMeta::Object | TypeMeta::Delegate { .. }) => unwrap_nullable_return(expr),
        Some(TypeMeta::Interface { name, .. }) if known_types.contains(name) => {
            let r = resolve_type_name(name, deferred);
            wrap_nullable_interface_return(expr, &r)
        }
        Some(TypeMeta::Parameterized { name, args, .. }) => {
            let concrete = crate::meta::make_parameterized_name(name, args);
            if known_types.contains(&concrete) {
                let r = resolve_type_name(&concrete, deferred);
                wrap_nullable_interface_return(expr, &r)
            } else {
                unwrap_nullable_return(expr)
            }
        }
        Some(TypeMeta::RuntimeClass { .. } | TypeMeta::Interface { .. }) => {
            unwrap_nullable_return(expr)
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
    let cache_name = format!("{}Cache", var_name);
    let mut registration = String::new();
    registration.push_str("DynWinRtType.registerInterface(\n");
    registration.push_str(&format!(
        "        \"{}\", IID_{})\n",
        iface.name, iface.name
    ));
    for method in &iface.methods {
        registration.push_str(&format!(
            "        .addMethod(\"{}\", {})\n",
            method.name,
            build_method_sig(method)
        ));
    }
    if registration.ends_with('\n') {
        registration.truncate(registration.len() - 1);
    }

    let mut out = String::new();
    out.push_str(&format!("let {};\n", cache_name));
    out.push_str(&format!("const {} = new Proxy({{}}, {{\n", var_name));
    out.push_str("    get(_target, prop) {\n");
    out.push_str(&format!("        {} ??= {};\n", cache_name, registration));
    out.push_str(&format!("        const value = {}[prop];\n", cache_name));
    out.push_str(&format!(
        "        return typeof value === 'function' ? value.bind({}) : value;\n",
        cache_name
    ));
    out.push_str("    }\n");
    out.push_str("});\n");
    out
}
