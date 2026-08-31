// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! JavaScript struct field helpers.

use crate::codegen::winrt::shared::imports::ireference_inner_type;
use crate::types::TypeMeta;

use super::signature::{ref_marker, wrap_arg};

// ======================================================================
// Struct field type helpers (TypeScript)
// ======================================================================

/// Map a struct field type to its TypeScript type annotation.
pub(crate) fn ts_struct_field_type(typ: &TypeMeta) -> String {
    if let Some(inner) = ireference_inner_type(typ) {
        let TypeMeta::Parameterized {
            namespace,
            name,
            piid,
            args,
        } = typ
        else {
            unreachable!()
        };
        let wrapper = super::projected_parameterized_name(namespace, name, piid, args);
        return format!("{} | null | {}", ts_ireference_inner_type(inner), wrapper);
    }

    ts_struct_field_read_type(typ)
}

fn ts_ireference_inner_type(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Enum { name, .. } => name.clone(),
        _ => ts_struct_field_read_type(typ),
    }
}

fn ts_struct_field_read_type(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => "boolean".to_string(),
        TypeMeta::String => "string".to_string(),
        TypeMeta::Guid => "string".to_string(),
        TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::Char16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::F32
        | TypeMeta::F64
        | TypeMeta::Enum { .. } => "number".to_string(),
        TypeMeta::I64 | TypeMeta::U64 => "bigint".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "number".to_string(),
        TypeMeta::Struct { name, .. } => name.clone(),
        _ => "DynWinRtValue".to_string(),
    }
}

/// Generate a `DynWinRtStruct.getXxx(index)` expression for a struct field.
pub(crate) fn struct_field_getter(typ: &TypeMeta, index: usize) -> String {
    if ireference_inner_type(typ).is_some() {
        let TypeMeta::Parameterized {
            namespace,
            name,
            piid,
            args,
        } = typ
        else {
            unreachable!()
        };
        let wrapper = ref_marker(&super::projected_parameterized_name(
            namespace, name, piid, args,
        ));
        return format!(
            "((value) => value.isNull() ? null : new {}(value).value)(s.getObject({}))",
            wrapper, index
        );
    }

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
        TypeMeta::Struct { name, .. } => {
            format!("_unpack{}(s.getStruct({}).toValue())", name, index)
        }
        _ => format!("s.getObject({})", index), // IReference<T> etc.
    }
}

/// Generate a `s.setXxx(index, expr)` statement for a struct field.
pub(crate) fn struct_field_setter(typ: &TypeMeta, index: usize, value_expr: &str) -> String {
    if ireference_inner_type(typ).is_some() {
        return format!("s.setObject({}, {})", index, wrap_arg(value_expr, typ));
    }

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
        TypeMeta::Struct { name, .. } if name == "HResult" => {
            format!("s.setI32({}, {})", index, value_expr)
        }
        TypeMeta::Struct { name, .. } => {
            format!("s.setStruct({}, _pack{}({}))", index, name, value_expr)
        }
        _ => format!("s.setObject({}, {})", index, value_expr), // IReference<T> etc.
    }
}
