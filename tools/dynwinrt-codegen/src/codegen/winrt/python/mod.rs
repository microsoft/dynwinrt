// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

pub(crate) mod collections;
mod docs;
mod generator;
pub(crate) mod method;
pub(crate) mod naming;
mod native_types;
pub(crate) mod overloads;
mod shared;
pub(crate) mod signature;
pub(crate) mod structs;
pub(crate) mod stub_helpers;
pub mod stubs;
pub(crate) mod type_helpers;

pub use generator::*;
pub use naming::{
    PythonTypeIdentity, install_python_module_layout, python_module_name,
    python_namespace_segments, python_public_module_name, to_snake_case_filename,
};

pub(crate) fn collect_referenced_delegate_names(
    methods: &[crate::meta::MethodMeta],
    known_delegate_names: &std::collections::HashSet<String>,
) -> std::collections::HashSet<String> {
    fn collect(
        typ: &crate::types::TypeMeta,
        known: &std::collections::HashSet<String>,
        result: &mut std::collections::HashSet<String>,
    ) {
        use crate::types::TypeMeta;
        match typ {
            TypeMeta::Delegate { name, .. } => {
                result.insert(name.clone());
            }
            TypeMeta::Interface { name, .. } => {
                if known.contains(name) {
                    result.insert(name.clone());
                }
            }
            TypeMeta::AsyncActionWithProgress(inner)
            | TypeMeta::AsyncOperation(inner)
            | TypeMeta::Array(inner) => {
                collect(inner, known, result);
            }
            TypeMeta::AsyncOperationWithProgress(
                result_type,
                progress,
            ) => {
                collect(result_type, known, result);
                collect(progress, known, result);
            }
            TypeMeta::Parameterized { name, args, .. } => {
                let concrete =
                    crate::meta::make_parameterized_name(
                        name,
                        args,
                    );
                if known.contains(&concrete) {
                    result.insert(concrete);
                }
                for argument in args {
                    collect(argument, known, result);
                }
            }
            TypeMeta::RuntimeClass {
                default_interface: Some(interface),
                ..
            } => {
                collect(interface, known, result);
            }
            TypeMeta::Struct { fields, .. } => {
                for field in fields {
                    collect(&field.typ, known, result);
                }
            }
            _ => {}
        }
    }

    let mut result = std::collections::HashSet::new();
    for method in methods {
        for parameter in &method.params {
            collect(
                &parameter.typ,
                known_delegate_names,
                &mut result,
            );
        }
        if let Some(return_type) = &method.return_type {
            collect(
                return_type,
                known_delegate_names,
                &mut result,
            );
        }
    }
    result
}

pub fn package_struct_identities(
    classes: &[crate::meta::ClassMeta],
    interfaces: &[crate::meta::InterfaceMeta],
) -> Vec<(String, String)> {
    use crate::codegen::winrt::shared::structs::{
        collect_used_structs_from_class, collect_used_structs_from_iface,
    };
    use crate::types::TypeMeta;
    use std::collections::HashSet;

    let mut identities = HashSet::new();
    for typ in classes
        .iter()
        .flat_map(collect_used_structs_from_class)
        .chain(interfaces.iter().flat_map(collect_used_structs_from_iface))
    {
        if let TypeMeta::Struct {
            namespace, name, ..
        } = typ
        {
            identities.insert((namespace, name));
        }
    }
    identities.into_iter().collect()
}
