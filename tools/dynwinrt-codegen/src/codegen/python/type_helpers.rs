// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python type annotations and method documentation helpers.

use std::collections::HashSet;

use crate::codegen::shared::docs::{DocText, find_param_doc};
use crate::meta::MethodMeta;
use crate::types::TypeMeta;

use super::docs::format_pydoc;
use super::naming::to_snake_case;

/// Build the Python docstring for a method body. Uses snake_case param display
/// names (matching the generated signature). Returns an empty string when no
/// doc fields are populated, preserving byte-identity for metadata without
/// sibling .xml files.
pub(super) fn method_pydoc(method: &MethodMeta, in_params: &[&crate::meta::ParamMeta]) -> String {
    if method.doc.is_none()
        && method.deprecated.is_none()
        && method.returns_doc.is_none()
        && method.param_docs.is_empty()
    {
        return String::new();
    }
    let params_snake: Vec<(String, &str)> = in_params
        .iter()
        .filter_map(|p| {
            find_param_doc(&method.param_docs, &p.name).map(|d| (to_snake_case(&p.name), d))
        })
        .collect();
    let params_refs: Vec<(&str, &str)> =
        params_snake.iter().map(|(n, d)| (n.as_str(), *d)).collect();
    let doc = DocText {
        summary: method.doc.as_deref(),
        deprecated: method.deprecated.as_deref(),
        returns: method.returns_doc.as_deref(),
        params: params_refs,
    };
    format_pydoc(&doc, "        ")
}

// ======================================================================
// Python type annotation helpers
// ======================================================================

fn py_param_type(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => "bool".to_string(),
        TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::Char16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::I64
        | TypeMeta::U64 => "int".to_string(),
        TypeMeta::F32 | TypeMeta::F64 => "float".to_string(),
        TypeMeta::String | TypeMeta::Guid => "str".to_string(),
        TypeMeta::RuntimeClass { name, .. }
        | TypeMeta::Enum { name, .. }
        | TypeMeta::Interface { name, .. } => format!("'{}'", name),
        TypeMeta::Parameterized { name, args, .. } => {
            format!("'{}'", crate::meta::make_parameterized_name(name, args))
        }
        TypeMeta::Array(_) => "'DynWinRTArray'".to_string(),
        TypeMeta::Object | TypeMeta::Delegate { .. } => "'DynWinRTValue'".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "int".to_string(),
        TypeMeta::Struct { name, .. } => format!("'{}'", name),
        _ => "object".to_string(),
    }
}

pub(super) fn py_param_type_safe(typ: &TypeMeta, known: &HashSet<String>) -> String {
    match typ {
        TypeMeta::RuntimeClass { name, .. }
        | TypeMeta::Enum { name, .. }
        | TypeMeta::Interface { name, .. }
            if !known.contains(name) =>
        {
            "'DynWinRTValue'".to_string()
        }
        _ => py_param_type(typ),
    }
}

pub(super) fn py_return_type_safe(typ: Option<&TypeMeta>, known: &HashSet<String>) -> String {
    match typ {
        Some(TypeMeta::RuntimeClass { name, .. })
        | Some(TypeMeta::Enum { name, .. })
        | Some(TypeMeta::Interface { name, .. })
            if !known.contains(name) =>
        {
            "'DynWinRTValue'".to_string()
        }
        Some(TypeMeta::AsyncOperation(inner)) => py_return_type_safe(Some(inner), known),
        Some(TypeMeta::AsyncOperationWithProgress(result, _)) => {
            py_return_type_safe(Some(result), known)
        }
        Some(TypeMeta::AsyncActionWithProgress(_)) | Some(TypeMeta::AsyncAction) => {
            "None".to_string()
        }
        Some(TypeMeta::Array(inner)) => py_array_element_type(inner, known),
        _ => py_return_type(typ),
    }
}

fn py_return_type(typ: Option<&TypeMeta>) -> String {
    match typ {
        Some(TypeMeta::String) | Some(TypeMeta::Guid) => "str".to_string(),
        Some(TypeMeta::Bool) => "bool".to_string(),
        Some(
            TypeMeta::I8
            | TypeMeta::U8
            | TypeMeta::I16
            | TypeMeta::U16
            | TypeMeta::Char16
            | TypeMeta::I32
            | TypeMeta::U32
            | TypeMeta::I64
            | TypeMeta::U64,
        ) => "int".to_string(),
        Some(TypeMeta::F32 | TypeMeta::F64) => "float".to_string(),
        Some(TypeMeta::RuntimeClass { name, .. }) => format!("'{}'", name),
        Some(TypeMeta::Enum { name, .. }) => format!("'{}'", name),
        Some(TypeMeta::Interface { name, .. }) => format!("'{}'", name),
        Some(TypeMeta::Parameterized { name, args, .. }) => {
            format!("'{}'", crate::meta::make_parameterized_name(name, args))
        }
        Some(TypeMeta::AsyncOperation(inner)) => py_return_type(Some(inner)),
        Some(TypeMeta::AsyncOperationWithProgress(result, _)) => py_return_type(Some(result)),
        Some(TypeMeta::AsyncAction) | Some(TypeMeta::AsyncActionWithProgress(_)) => {
            "None".to_string()
        }
        Some(TypeMeta::Array(inner)) => py_array_element_type(inner, &HashSet::new()),
        Some(TypeMeta::Object) | Some(TypeMeta::Delegate { .. }) => "'DynWinRTValue'".to_string(),
        Some(TypeMeta::Struct { name, .. }) if name == "HResult" => "int".to_string(),
        Some(TypeMeta::Struct { name, .. }) => format!("'{}'", name),
        None => "None".to_string(),
    }
}

pub(super) fn py_array_element_type(inner: &TypeMeta, known_types: &HashSet<String>) -> String {
    match inner {
        TypeMeta::Bool => "list[bool]".to_string(),
        TypeMeta::String | TypeMeta::Guid => "list[str]".to_string(),
        TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::Char16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::I64
        | TypeMeta::U64
        | TypeMeta::Enum { .. } => "list[int]".to_string(),
        TypeMeta::F32 | TypeMeta::F64 => "list[float]".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "list[int]".to_string(),
        TypeMeta::Struct { name, .. } => format!("list['{}']", name),
        TypeMeta::RuntimeClass { name, .. } if known_types.contains(name) => {
            format!("list['{}']", name)
        }
        TypeMeta::Interface { name, .. } if known_types.contains(name) => {
            format!("list['{}']", name)
        }
        _ => "list".to_string(),
    }
}

pub(super) fn py_param_list(
    in_params: &[&crate::meta::ParamMeta],
    known_types: &HashSet<String>,
) -> String {
    in_params
        .iter()
        .map(|p| {
            format!(
                "{}: {}",
                to_snake_case(&p.name),
                py_param_type_safe(&p.typ, known_types)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}
