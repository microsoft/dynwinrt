// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python struct field helpers.

use crate::types::TypeMeta;

use super::naming::to_snake_case;
use super::native_types::{FoundationType, foundation_type};

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
        TypeMeta::U16 => format!("s.get_u16({})", index),
        TypeMeta::Char16 => format!("chr(s.get_u16({}))", index),
        TypeMeta::I32 | TypeMeta::Enum { .. } => format!("s.get_i32({})", index),
        TypeMeta::U32 => format!("s.get_u32({})", index),
        TypeMeta::I64 => format!("s.get_i64({})", index),
        TypeMeta::U64 => format!("s.get_u64({})", index),
        TypeMeta::F32 => format!("s.get_f32({})", index),
        TypeMeta::F64 => format!("s.get_f64({})", index),
        TypeMeta::String => format!("s.get_hstring({})", index),
        TypeMeta::Guid => format!("_dynwinrt_uuid(s.get_guid({}))", index),
        TypeMeta::Struct { name, .. } if name == "HResult" => format!("s.get_i32({})", index),
        TypeMeta::Struct { name, .. } => format!(
            "_unpack_{}(s.get_struct({}).to_value())",
            to_snake_case(name),
            index
        ),
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
        TypeMeta::U16 => format!("s.set_u16({}, {})", index, value_expr),
        TypeMeta::Char16 => format!("s.set_u16({}, ord({}))", index, value_expr),
        TypeMeta::I32 | TypeMeta::Enum { .. } => format!("s.set_i32({}, {})", index, value_expr),
        TypeMeta::U32 => format!("s.set_u32({}, {})", index, value_expr),
        TypeMeta::I64 => format!("s.set_i64({}, {})", index, value_expr),
        TypeMeta::U64 => format!("s.set_u64({}, {})", index, value_expr),
        TypeMeta::F32 => format!("s.set_f32({}, {})", index, value_expr),
        TypeMeta::F64 => format!("s.set_f64({}, {})", index, value_expr),
        TypeMeta::String => format!("s.set_hstring({}, {})", index, value_expr),
        TypeMeta::Guid => format!("s.set_guid({}, _dynwinrt_guid({}))", index, value_expr),
        TypeMeta::Struct { name, .. } if name == "HResult" => {
            format!("s.set_i32({}, {})", index, value_expr)
        }
        TypeMeta::Struct { name, .. } => format!(
            "s.set_struct({}, _pack_{}({}))",
            index,
            to_snake_case(name),
            value_expr
        ),
        _ => format!("s.set_object({}, {})", index, value_expr),
    }
}

/// Python type annotation for a struct field.
pub(crate) fn py_struct_field_type(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => "bool".to_string(),
        TypeMeta::String => "str".to_string(),
        TypeMeta::Guid => "UUID".to_string(),
        TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::I64
        | TypeMeta::U64 => "int".to_string(),
        TypeMeta::Char16 => "str".to_string(),
        TypeMeta::F32 | TypeMeta::F64 => "float".to_string(),
        TypeMeta::Enum { name, .. } => format!("'{}'", name),
        TypeMeta::Struct { name, .. } if name == "HResult" => "int".to_string(),
        typ if foundation_type(typ) == Some(FoundationType::DateTime) => "datetime".to_string(),
        typ if foundation_type(typ) == Some(FoundationType::TimeSpan) => "timedelta".to_string(),
        TypeMeta::Struct { name, .. } => format!("'{}'", name),
        _ => "'DynWinRTValue'".to_string(),
    }
}
