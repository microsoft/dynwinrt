// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Struct collection and field helpers for TypeScript and Python codegen.

use std::collections::HashSet;

use crate::meta::{ClassMeta, InterfaceMeta};
use crate::types::TypeMeta;

use super::naming::to_snake_case;

// ======================================================================
// Struct collection helpers
// ======================================================================

/// Recursively collect non-HResult struct types from a type tree.
fn collect_used_structs_from_type(typ: &TypeMeta, seen: &mut HashSet<String>, result: &mut Vec<TypeMeta>) {
    match typ {
        TypeMeta::Struct { namespace, name, fields } => {
            if name != "HResult" {
                let full = format!("{}.{}", namespace, name);
                if !seen.insert(full) {
                    return; // already collected
                }
            }
            // Recurse into fields FIRST so nested structs appear before this one
            for f in fields { collect_used_structs_from_type(&f.typ, seen, result); }
            if name != "HResult" {
                result.push(typ.clone());
            }
        }
        TypeMeta::AsyncOperation(inner) | TypeMeta::AsyncActionWithProgress(inner) => {
            collect_used_structs_from_type(inner, seen, result);
        }
        TypeMeta::AsyncOperationWithProgress(r, p) => {
            collect_used_structs_from_type(r, seen, result);
            collect_used_structs_from_type(p, seen, result);
        }
        TypeMeta::Array(inner) => collect_used_structs_from_type(inner, seen, result),
        TypeMeta::Parameterized { args, .. } => {
            for arg in args { collect_used_structs_from_type(arg, seen, result); }
        }
        _ => {}
    }
}

pub(crate) fn collect_used_structs_from_class(class: &ClassMeta) -> Vec<TypeMeta> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for iface in class.all_interfaces() {
        for m in &iface.methods {
            for p in &m.params { collect_used_structs_from_type(&p.typ, &mut seen, &mut result); }
            if let Some(ref rt) = m.return_type { collect_used_structs_from_type(rt, &mut seen, &mut result); }
        }
    }
    result
}

pub(crate) fn collect_used_structs_from_iface(iface: &InterfaceMeta) -> Vec<TypeMeta> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for m in &iface.methods {
        for p in &m.params { collect_used_structs_from_type(&p.typ, &mut seen, &mut result); }
        if let Some(ref rt) = m.return_type { collect_used_structs_from_type(rt, &mut seen, &mut result); }
    }
    result
}

// ======================================================================
// Struct field type helpers (TypeScript)
// ======================================================================

/// Map a struct field type to its TypeScript type annotation.
pub(crate) fn ts_struct_field_type(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => "boolean".to_string(),
        TypeMeta::String => "string".to_string(),
        TypeMeta::Guid => "string".to_string(),
        TypeMeta::I8 | TypeMeta::U8 | TypeMeta::I16 | TypeMeta::U16 | TypeMeta::Char16
        | TypeMeta::I32 | TypeMeta::U32
        | TypeMeta::F32 | TypeMeta::F64 | TypeMeta::Enum { .. } => "number".to_string(),
        TypeMeta::I64 | TypeMeta::U64 => "bigint".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "number".to_string(),
        TypeMeta::Struct { name, .. } => name.clone(),
        _ => "DynWinRtValue".to_string(),
    }
}

/// Generate a `DynWinRtStruct.getXxx(index)` expression for a struct field.
pub(crate) fn struct_field_getter(typ: &TypeMeta, index: usize) -> String {
    match typ {
        TypeMeta::Bool => format!("s.getU8({}) !== 0", index),
        TypeMeta::I8 => format!("s.getI8({})", index),
        TypeMeta::U8 => format!("s.getU8({})", index),
        TypeMeta::I16 => format!("s.getI16({})", index),
        TypeMeta::U16 | TypeMeta::Char16 => format!("s.getU16({})", index),
        TypeMeta::I32 | TypeMeta::Enum { .. } => format!("s.getI32({})", index),
        TypeMeta::U32 => format!("s.getU32({})", index),
        TypeMeta::I64 => format!("s.getI64({})", index),
        TypeMeta::U64 => format!("s.getU64({})", index),
        TypeMeta::F32 => format!("s.getF32({})", index),
        TypeMeta::F64 => format!("s.getF64({})", index),
        TypeMeta::String => format!("s.getHstring({})", index),
        TypeMeta::Guid => format!("s.getGuid({}).toString()", index),
        TypeMeta::Struct { name, .. } if name == "HResult" => format!("s.getI32({})", index),
        TypeMeta::Struct { name, .. } => format!("_unpack{}(s.getStruct({}).toValue())", name, index),
        _ => format!("s.getObject({})", index), // IReference<T> etc.
    }
}

/// Generate a `s.setXxx(index, expr)` statement for a struct field.
pub(crate) fn struct_field_setter(typ: &TypeMeta, index: usize, value_expr: &str) -> String {
    match typ {
        TypeMeta::Bool => format!("s.setU8({}, {} ? 1 : 0)", index, value_expr),
        TypeMeta::I8 => format!("s.setI8({}, {})", index, value_expr),
        TypeMeta::U8 => format!("s.setU8({}, {})", index, value_expr),
        TypeMeta::I16 => format!("s.setI16({}, {})", index, value_expr),
        TypeMeta::U16 | TypeMeta::Char16 => format!("s.setU16({}, {})", index, value_expr),
        TypeMeta::I32 | TypeMeta::Enum { .. } => format!("s.setI32({}, {})", index, value_expr),
        TypeMeta::U32 => format!("s.setU32({}, {})", index, value_expr),
        TypeMeta::I64 => format!("s.setI64({}, {})", index, value_expr),
        TypeMeta::U64 => format!("s.setU64({}, {})", index, value_expr),
        TypeMeta::F32 => format!("s.setF32({}, {})", index, value_expr),
        TypeMeta::F64 => format!("s.setF64({}, {})", index, value_expr),
        TypeMeta::String => format!("s.setHstring({}, {})", index, value_expr),
        TypeMeta::Guid => format!("s.setGuid({}, WinGuid.parse({}))", index, value_expr),
        TypeMeta::Struct { name, .. } if name == "HResult" => format!("s.setI32({}, {})", index, value_expr),
        TypeMeta::Struct { name, .. } => format!("s.setStruct({}, _pack{}({}))", index, name, value_expr),
        _ => format!("s.setObject({}, {})", index, value_expr), // IReference<T> etc.
    }
}

// ======================================================================
// Struct field type helpers (Python)
// ======================================================================

/// Python struct field getter expression.
pub(crate) fn py_struct_field_getter(typ: &TypeMeta, index: usize) -> String {
    match typ {
        TypeMeta::Bool => format!("s.get_u8({}) != 0", index),
        TypeMeta::I8 => format!("s.get_i8({})", index),
        TypeMeta::U8 => format!("s.get_u8({})", index),
        TypeMeta::I16 => format!("s.get_i16({})", index),
        TypeMeta::U16 | TypeMeta::Char16 => format!("s.get_u16({})", index),
        TypeMeta::I32 | TypeMeta::Enum { .. } => format!("s.get_i32({})", index),
        TypeMeta::U32 => format!("s.get_u32({})", index),
        TypeMeta::I64 => format!("s.get_i64({})", index),
        TypeMeta::U64 => format!("s.get_u64({})", index),
        TypeMeta::F32 => format!("s.get_f32({})", index),
        TypeMeta::F64 => format!("s.get_f64({})", index),
        TypeMeta::String => format!("s.get_hstring({})", index),
        TypeMeta::Guid => format!("s.get_guid({})", index),
        TypeMeta::Struct { name, .. } if name == "HResult" => format!("s.get_i32({})", index),
        TypeMeta::Struct { name, .. } => format!("_unpack_{}(s.get_struct({}).to_value())", to_snake_case(name), index),
        _ => format!("s.get_object({})", index),
    }
}

/// Python struct field setter expression.
pub(crate) fn py_struct_field_setter(typ: &TypeMeta, index: usize, value_expr: &str) -> String {
    match typ {
        TypeMeta::Bool => format!("s.set_u8({}, 1 if {} else 0)", index, value_expr),
        TypeMeta::I8 => format!("s.set_i8({}, {})", index, value_expr),
        TypeMeta::U8 => format!("s.set_u8({}, {})", index, value_expr),
        TypeMeta::I16 => format!("s.set_i16({}, {})", index, value_expr),
        TypeMeta::U16 | TypeMeta::Char16 => format!("s.set_u16({}, {})", index, value_expr),
        TypeMeta::I32 | TypeMeta::Enum { .. } => format!("s.set_i32({}, {})", index, value_expr),
        TypeMeta::U32 => format!("s.set_u32({}, {})", index, value_expr),
        TypeMeta::I64 => format!("s.set_i64({}, {})", index, value_expr),
        TypeMeta::U64 => format!("s.set_u64({}, {})", index, value_expr),
        TypeMeta::F32 => format!("s.set_f32({}, {})", index, value_expr),
        TypeMeta::F64 => format!("s.set_f64({}, {})", index, value_expr),
        TypeMeta::String => format!("s.set_hstring({}, {})", index, value_expr),
        TypeMeta::Guid => format!("s.set_guid({}, WinGUID.parse({}))", index, value_expr),
        TypeMeta::Struct { name, .. } if name == "HResult" => format!("s.set_i32({}, {})", index, value_expr),
        TypeMeta::Struct { name, .. } => format!("s.set_struct({}, _pack_{}({}))", index, to_snake_case(name), value_expr),
        _ => format!("s.set_object({}, {})", index, value_expr),
    }
}

/// Python type annotation for a struct field.
pub(crate) fn py_struct_field_type(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => "bool".to_string(),
        TypeMeta::String | TypeMeta::Guid => "str".to_string(),
        TypeMeta::I8 | TypeMeta::U8 | TypeMeta::I16 | TypeMeta::U16 | TypeMeta::Char16
        | TypeMeta::I32 | TypeMeta::U32 | TypeMeta::I64 | TypeMeta::U64 => "int".to_string(),
        TypeMeta::F32 | TypeMeta::F64 => "float".to_string(),
        TypeMeta::Enum { name, .. } => format!("'{}'", name),
        TypeMeta::Struct { name, .. } if name == "HResult" => "int".to_string(),
        TypeMeta::Struct { name, .. } => format!("'{}'", name),
        _ => "'DynWinRTValue'".to_string(),
    }
}
