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
    PythonModuleLayoutGuard, PythonTypeIdentity, install_python_module_layout,
    python_module_layout_installed, python_module_name, python_namespace_segments,
    python_public_module_name, python_public_qualified_module_name, to_snake_case_filename,
};

pub(crate) fn has_projectable_default_interface(class: &crate::meta::ClassMeta) -> bool {
    let Some(default_interface) = class.default_interface.as_ref() else {
        return false;
    };
    if default_interface.iid.is_empty() && default_interface.generic_piid.is_none() {
        return false;
    }
    !default_interface.methods.is_empty()
        || !class.required_interfaces.is_empty()
        || !class.overridable_interfaces.is_empty()
        || class.base_class.is_some()
        || !class.constructors.is_empty()
        || class.is_referenced_as_value
        || class
            .factory_interfaces
            .iter()
            .chain(class.static_interfaces.iter())
            .any(|interface| {
                interface.methods.iter().any(|method| {
                    matches!(
                        method.return_type.as_ref(),
                        Some(crate::types::TypeMeta::RuntimeClass {
                            namespace,
                            name,
                            ..
                        }) if namespace == &class.namespace && name == &class.name
                    )
                })
            })
}

pub(crate) fn has_native_projector(class: &crate::meta::ClassMeta) -> bool {
    has_projectable_default_interface(class)
        || class
            .default_interface
            .as_ref()
            .is_some_and(|interface| !interface.iid.is_empty())
}

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
            TypeMeta::AsyncOperationWithProgress(result_type, progress) => {
                collect(result_type, known, result);
                collect(progress, known, result);
            }
            TypeMeta::Parameterized { name, args, .. } => {
                let concrete = crate::meta::make_parameterized_name(name, args);
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
            collect(&parameter.typ, known_delegate_names, &mut result);
        }
        if let Some(return_type) = &method.return_type {
            collect(return_type, known_delegate_names, &mut result);
        }
    }
    result
}

pub(crate) fn collect_runtime_delegate_names(
    methods: &[crate::meta::MethodMeta],
    known_delegate_names: &std::collections::HashSet<String>,
) -> std::collections::HashSet<String> {
    use crate::meta::ParamDirection;
    use crate::types::TypeMeta;

    let mut result = std::collections::HashSet::new();
    for method in methods {
        for parameter in &method.params {
            if parameter.direction != ParamDirection::In {
                continue;
            }
            match &parameter.typ {
                TypeMeta::Delegate { name, .. } => {
                    result.insert(name.clone());
                }
                TypeMeta::Interface { name, .. } if known_delegate_names.contains(name) => {
                    result.insert(name.clone());
                }
                TypeMeta::Parameterized { name, args, .. } => {
                    let concrete = crate::meta::make_parameterized_name(name, args);
                    if known_delegate_names.contains(&concrete) {
                        result.insert(concrete);
                    }
                }
                _ => {}
            }
        }
    }
    result
}

pub fn package_structs(
    classes: &[crate::meta::ClassMeta],
    interfaces: &[crate::meta::InterfaceMeta],
) -> Vec<crate::types::TypeMeta> {
    use crate::codegen::winrt::shared::structs::{
        collect_used_structs_from_class, collect_used_structs_from_iface,
    };
    use crate::types::TypeMeta;
    use std::collections::BTreeMap;

    let mut structs = BTreeMap::new();
    for typ in classes
        .iter()
        .flat_map(collect_used_structs_from_class)
        .chain(interfaces.iter().flat_map(collect_used_structs_from_iface))
    {
        if let TypeMeta::Struct {
            namespace, name, ..
        } = &typ
        {
            structs
                .entry((namespace.clone(), name.clone()))
                .or_insert(typ);
        }
    }
    structs.into_values().collect()
}

pub fn package_struct_identities(
    classes: &[crate::meta::ClassMeta],
    interfaces: &[crate::meta::InterfaceMeta],
) -> Vec<(String, String)> {
    package_structs(classes, interfaces)
        .into_iter()
        .filter_map(|typ| match typ {
            crate::types::TypeMeta::Struct {
                namespace, name, ..
            } => Some((namespace, name)),
            _ => None,
        })
        .collect()
}

pub fn validate_struct_symbol_uniqueness(
    classes: &[crate::meta::ClassMeta],
    interfaces: &[crate::meta::InterfaceMeta],
) -> Result<(), String> {
    use crate::codegen::winrt::shared::structs::{
        collect_used_structs_from_class, collect_used_structs_from_iface,
        collect_used_structs_from_struct,
    };
    use crate::types::TypeMeta;
    use std::collections::BTreeMap;

    fn validate(owner: &str, structs: impl IntoIterator<Item = TypeMeta>) -> Result<(), String> {
        let mut identities = BTreeMap::<String, String>::new();
        for typ in structs {
            let TypeMeta::Struct {
                namespace, name, ..
            } = typ
            else {
                continue;
            };
            let full_name = format!("{namespace}.{name}");
            if let Some(existing) = identities.insert(name.clone(), full_name.clone())
                && existing != full_name
            {
                return Err(format!(
                    "Python generation cannot safely emit `{owner}` because `{existing}` and \
                     `{full_name}` both require the struct symbols `{name}`, `_{name}_TYPE`, \
                     `_pack_{snake}`, and `_unpack_{snake}`",
                    snake = to_snake_case_filename(&name),
                ));
            }
        }
        Ok(())
    }

    for class in classes {
        validate(&class.full_name, collect_used_structs_from_class(class))?;
    }
    for interface in interfaces {
        validate(
            &format!("{}.{}", interface.namespace, interface.name),
            collect_used_structs_from_iface(interface),
        )?;
    }
    for typ in package_structs(classes, interfaces) {
        let TypeMeta::Struct {
            namespace, name, ..
        } = &typ
        else {
            continue;
        };
        let mut dependencies = vec![typ.clone()];
        dependencies.extend(collect_used_structs_from_struct(&typ));
        validate(&format!("{namespace}.{name}"), dependencies)?;
    }
    Ok(())
}
