// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Import / generics collection helpers used by orchestrators for module
//! header generation.

use std::collections::HashSet;
use std::sync::LazyLock;

use crate::meta::{ClassMeta, InterfaceMeta, MethodMeta, ParamDirection};
use crate::types::{TypeIdentity, TypeKind, TypeMeta, TypeRef};

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

fn visit_used_generic_identities(typ: &TypeMeta, identities: &mut HashSet<TypeIdentity>) {
    match typ {
        TypeMeta::Parameterized { args, .. } => {
            identities.insert(typ.type_identity());
            for argument in args {
                visit_used_generic_identities(argument, identities);
            }
        }
        TypeMeta::AsyncOperation(inner) | TypeMeta::AsyncActionWithProgress(inner) => {
            visit_used_generic_identities(inner, identities)
        }
        TypeMeta::AsyncOperationWithProgress(result, progress) => {
            visit_used_generic_identities(result, identities);
            visit_used_generic_identities(progress, identities);
        }
        TypeMeta::Array(inner) => visit_used_generic_identities(inner, identities),
        TypeMeta::Struct { fields, .. } => {
            for field in fields {
                visit_used_generic_identities(&field.typ, identities);
            }
        }
        _ => {}
    }
}

pub(crate) fn collect_used_generic_identities_from_type(typ: &TypeMeta) -> Vec<TypeIdentity> {
    let mut identities = HashSet::new();
    visit_used_generic_identities(typ, &mut identities);
    let mut sorted = identities.into_iter().collect::<Vec<_>>();
    sorted.sort();
    sorted
}

fn collect_used_generic_identities_from_methods_inner(
    methods: &[&MethodMeta],
) -> Vec<TypeIdentity> {
    let mut identities = HashSet::new();
    for method in methods {
        for parameter in &method.params {
            visit_used_generic_identities(&parameter.typ, &mut identities);
        }
        if let Some(return_type) = &method.return_type {
            visit_used_generic_identities(return_type, &mut identities);
        }
    }
    let mut sorted = identities.into_iter().collect::<Vec<_>>();
    sorted.sort();
    sorted
}

pub(crate) fn collect_used_generic_identities_from_methods(
    methods: &[MethodMeta],
) -> Vec<TypeIdentity> {
    let methods = methods.iter().collect::<Vec<_>>();
    collect_used_generic_identities_from_methods_inner(&methods)
}

pub(crate) fn collect_used_generic_identities_from_class(class: &ClassMeta) -> Vec<TypeIdentity> {
    let methods = class
        .all_interfaces()
        .flat_map(|interface| interface.methods.iter())
        .collect::<Vec<_>>();
    collect_used_generic_identities_from_methods_inner(&methods)
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

pub(crate) fn collect_struct_field_type_imports(typ: &TypeMeta) -> HashSet<TypeRef> {
    let TypeMeta::Struct { name, fields, .. } = typ else {
        return HashSet::new();
    };
    let mut imports = HashSet::new();
    for field in fields {
        visit_type_for_imports(&field.typ, name, false, &mut imports);
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
