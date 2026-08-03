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
