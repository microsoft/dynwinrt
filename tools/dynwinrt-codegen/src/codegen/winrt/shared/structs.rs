// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Language-neutral struct collection from metadata.

use std::collections::HashSet;

use crate::codegen::winrt::shared::imports::input_delegate_invokes;
use crate::meta::{ClassMeta, InterfaceMeta};
use crate::types::TypeMeta;

// ======================================================================
// Struct collection helpers
// ======================================================================

/// Recursively collect non-HResult struct types from a type tree.
pub(crate) fn collect_used_structs_from_type(
    typ: &TypeMeta,
    seen: &mut HashSet<String>,
    result: &mut Vec<TypeMeta>,
) {
    match typ {
        TypeMeta::Struct {
            namespace,
            name,
            fields,
        } => {
            if name != "HResult" {
                let full = format!("{}.{}", namespace, name);
                if !seen.insert(full) {
                    return; // already collected
                }
            }
            // Recurse into fields FIRST so nested structs appear before this one
            for f in fields {
                collect_used_structs_from_type(&f.typ, seen, result);
            }
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
            for arg in args {
                collect_used_structs_from_type(arg, seen, result);
            }
        }
        _ => {}
    }
}

pub(crate) fn collect_used_structs_from_struct(typ: &TypeMeta) -> Vec<TypeMeta> {
    let TypeMeta::Struct {
        namespace,
        name,
        fields,
    } = typ
    else {
        return Vec::new();
    };
    let mut seen = HashSet::from([format!("{namespace}.{name}")]);
    let mut result = Vec::new();
    for field in fields {
        collect_used_structs_from_type(&field.typ, &mut seen, &mut result);
    }
    result
}

pub(crate) fn collect_used_structs_from_class(class: &ClassMeta) -> Vec<TypeMeta> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for iface in class.all_interfaces() {
        for m in &iface.methods {
            for p in &m.params {
                collect_used_structs_from_type(&p.typ, &mut seen, &mut result);
            }
            if let Some(ref rt) = m.return_type {
                collect_used_structs_from_type(rt, &mut seen, &mut result);
            }
        }
    }
    result
}

/// Structs used by a class, followed by structs that only the delegates its
/// methods accept pass to Python callbacks.
pub(crate) fn collect_used_structs_from_class_and_callbacks(class: &ClassMeta) -> Vec<TypeMeta> {
    let mut result = collect_used_structs_from_class(class);
    let mut seen = result
        .iter()
        .filter_map(|typ| match typ {
            TypeMeta::Struct {
                namespace, name, ..
            } => Some(format!("{namespace}.{name}")),
            _ => None,
        })
        .collect::<HashSet<_>>();
    for invoke in class.all_interfaces().flat_map(input_delegate_invokes) {
        for p in &invoke.params {
            collect_used_structs_from_type(&p.typ, &mut seen, &mut result);
        }
    }
    result
}

pub(crate) fn collect_used_structs_from_iface(iface: &InterfaceMeta) -> Vec<TypeMeta> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for m in iface.methods.iter().chain(
        iface
            .implementation_metadata
            .delegates
            .iter()
            .map(|delegate| &delegate.invoke),
    ) {
        for p in &m.params {
            collect_used_structs_from_type(&p.typ, &mut seen, &mut result);
        }
        if let Some(ref rt) = m.return_type {
            collect_used_structs_from_type(rt, &mut seen, &mut result);
        }
    }
    result
}
