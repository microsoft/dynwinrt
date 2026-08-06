// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Import / generics collection helpers used by orchestrators for module
//! header generation.

use std::collections::HashSet;
use std::sync::LazyLock;

use crate::meta::{ClassMeta, InterfaceMeta, MethodMeta, ParamDirection};
use crate::types::{TypeKind, TypeMeta, TypeRef};

/// Empty set passed as `deferred` for codegen (no circular dep handling needed).
pub(crate) static NO_DEFERRED: LazyLock<HashSet<String>> = LazyLock::new(HashSet::new);

// ======================================================================
// Generic collection helpers
// ======================================================================

pub(crate) fn ireference_inner_type(typ: &TypeMeta) -> Option<&TypeMeta> {
    match typ {
        TypeMeta::Parameterized {
            namespace,
            name,
            args,
            ..
        } if namespace == "Windows.Foundation"
            && name.split('`').next() == Some("IReference")
            && args.len() == 1 =>
        {
            args.first()
        }
        _ => None,
    }
}

/// Collect the set of known generic collection names used in method signatures.
/// Returns e.g. ["IVectorView", "IMap"] for import generation.
pub(crate) fn collect_used_generics_from_methods(methods: &[MethodMeta]) -> Vec<String> {
    let refs: Vec<&MethodMeta> = methods.iter().collect();
    collect_used_generics_from_methods_inner(&refs)
}

/// Shared implementation for collecting generic names from method references.
fn collect_used_generics_from_methods_inner(methods: &[&MethodMeta]) -> Vec<String> {
    let mut names: HashSet<String> = HashSet::new();
    fn visit(typ: &TypeMeta, names: &mut HashSet<String>) {
        match typ {
            TypeMeta::Parameterized { name, args, .. } => {
                names.insert(crate::meta::make_parameterized_name(name, args));
                for arg in args {
                    visit(arg, names);
                }
            }
            TypeMeta::AsyncOperation(inner) | TypeMeta::AsyncActionWithProgress(inner) => {
                visit(inner, names)
            }
            TypeMeta::AsyncOperationWithProgress(r, p) => {
                visit(r, names);
                visit(p, names);
            }
            TypeMeta::Array(inner) => visit(inner, names),
            TypeMeta::Struct { fields, .. } => {
                for field in fields {
                    visit(&field.typ, names);
                }
            }
            _ => {}
        }
    }
    for m in methods {
        for p in &m.params {
            visit(&p.typ, &mut names);
        }
        if let Some(ref rt) = m.return_type {
            visit(rt, &mut names);
        }
    }
    let mut sorted: Vec<String> = names.into_iter().collect();
    sorted.sort();
    sorted
}

/// Collect all used generic names from a class (all its interfaces).
pub(crate) fn collect_used_generics_from_class(class: &ClassMeta) -> Vec<String> {
    let all_methods: Vec<&MethodMeta> = class
        .all_interfaces()
        .flat_map(|iface| &iface.methods)
        .collect();
    // Reuse the same visitor logic as collect_used_generics_from_methods
    collect_used_generics_from_methods_inner(&all_methods)
}

// ======================================================================
// Import collection helpers
// ======================================================================

/// Recursively collect named type references from a TypeMeta tree.
/// `self_name` is excluded from results (the type being generated).
/// `include_self_interfaces` controls whether Interface types with the same name
/// as self_name are included (needed for class imports, not for interface imports).
fn visit_type_for_imports(
    typ: &TypeMeta,
    self_name: &str,
    include_self_interfaces: bool,
    imports: &mut HashSet<TypeRef>,
) {
    match typ {
        TypeMeta::RuntimeClass {
            namespace, name, ..
        } if name != self_name => {
            imports.insert(TypeRef {
                namespace: namespace.clone(),
                name: name.clone(),
                kind: TypeKind::Class,
            });
        }
        TypeMeta::Interface {
            namespace, name, ..
        } => {
            if name != self_name || include_self_interfaces {
                imports.insert(TypeRef {
                    namespace: namespace.clone(),
                    name: name.clone(),
                    kind: TypeKind::Interface,
                });
            }
        }
        TypeMeta::Enum {
            namespace, name, ..
        } => {
            imports.insert(TypeRef {
                namespace: namespace.clone(),
                name: name.clone(),
                kind: TypeKind::Enum,
            });
        }
        TypeMeta::AsyncOperation(inner) | TypeMeta::AsyncActionWithProgress(inner) => {
            visit_type_for_imports(inner, self_name, include_self_interfaces, imports);
        }
        TypeMeta::AsyncOperationWithProgress(result, progress) => {
            visit_type_for_imports(result, self_name, include_self_interfaces, imports);
            visit_type_for_imports(progress, self_name, include_self_interfaces, imports);
        }
        TypeMeta::Struct { fields, .. } => {
            for f in fields {
                visit_type_for_imports(&f.typ, self_name, include_self_interfaces, imports);
            }
        }
        TypeMeta::Array(inner) => {
            visit_type_for_imports(inner, self_name, include_self_interfaces, imports)
        }
        TypeMeta::Parameterized { args, .. } => {
            for arg in args {
                visit_type_for_imports(arg, self_name, include_self_interfaces, imports);
            }
        }
        _ => {}
    }
}

/// Collect type imports from methods using the unified visitor.
fn collect_methods_type_imports(
    methods: &[MethodMeta],
    self_name: &str,
    include_self_interfaces: bool,
    imports: &mut HashSet<TypeRef>,
) {
    for m in methods {
        for p in &m.params {
            visit_type_for_imports(&p.typ, self_name, include_self_interfaces, imports);
        }
        if let Some(ref rt) = m.return_type {
            visit_type_for_imports(rt, self_name, include_self_interfaces, imports);
        }
    }
}

/// Collect type references from an interface for import generation.
pub(crate) fn collect_iface_type_imports(iface: &InterfaceMeta) -> HashSet<TypeRef> {
    let mut imports = HashSet::new();
    collect_methods_type_imports(&iface.methods, &iface.name, false, &mut imports);
    imports
}

/// Collect type references from a class for import generation.
pub(crate) fn collect_type_imports(class: &ClassMeta) -> HashSet<TypeRef> {
    let mut imports = HashSet::new();
    for iface in class.all_interfaces() {
        collect_methods_type_imports(&iface.methods, &class.name, true, &mut imports);
    }
    imports
}

// ======================================================================
// Parameter helpers
// ======================================================================

pub(crate) fn get_in_params(method: &MethodMeta) -> Vec<&crate::meta::ParamMeta> {
    // Include OutFill params as "in" — FillArray requires caller to provide the buffer
    method
        .params
        .iter()
        .filter(|p| p.direction == ParamDirection::In || p.direction == ParamDirection::OutFill)
        .collect()
}

/// WinRT FillArray methods encode the number of filled items as a UInt32
/// retval. The FillArray ABI already carries that value in its actual-count
/// pointer, so it must not be registered as a second out parameter.
pub(crate) fn fill_array_uses_retval_count(method: &MethodMeta) -> bool {
    method
        .params
        .iter()
        .any(|param| param.direction == ParamDirection::OutFill)
        && matches!(method.return_type, Some(TypeMeta::U32))
}

pub(crate) fn method_abi_output_count(method: &MethodMeta) -> usize {
    method
        .params
        .iter()
        .filter(|param| {
            matches!(
                param.direction,
                ParamDirection::Out | ParamDirection::OutFill
            )
        })
        .count()
        + usize::from(method.return_type.is_some())
}

pub(crate) fn fill_array_output_index(method: &MethodMeta) -> Option<usize> {
    let mut result_index = 0;
    for param in &method.params {
        match param.direction {
            ParamDirection::Out => result_index += 1,
            ParamDirection::OutFill => return Some(result_index),
            ParamDirection::In => {}
        }
    }
    None
}
