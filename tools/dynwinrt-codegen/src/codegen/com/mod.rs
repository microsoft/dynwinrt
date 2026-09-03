// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Classic-COM metadata projection and JavaScript generation.

pub mod capability;
mod generated_unsafe;
mod ir;
mod javascript;
mod model;
mod project;
mod typedef_inventory;

use crate::com_metadata::{ComCoclassMeta, ComInterfaceMeta};

pub use generated_unsafe::{
    Stage2Coverage, UNSAFE_SUPPORT_SCHEMA_VERSION, UnsafeGeneratedOutput, UnsafeInterfaceSupport,
    generate_unsafe_interface_files, generate_unsafe_interface_files_with_metadata,
    measure_stage2_coverage, render_unsafe_package_files, validate_unsafe_supports,
    windows_relative_path_key,
};
pub use javascript::render::ComGeneratedOutput;

fn module_path(namespace: &str, name: &str) -> String {
    let namespace = crate::codegen::javascript_layout::canonical_namespace_path(namespace);
    if namespace.is_empty() {
        name.to_string()
    } else {
        format!("{namespace}/{name}")
    }
}

pub fn canonical_module_path(namespace: &str, name: &str) -> Result<String, String> {
    let path = module_path(namespace, name);
    windows_relative_path_key(&path)?;
    if path.split('/').next() == Some("unsafe") {
        return Err(format!(
            "Classic-COM module `{namespace}.{name}` collides with the reserved unsafe package"
        ));
    }
    Ok(path)
}

pub fn canonical_module_file_path(
    namespace: &str,
    name: &str,
    extension: &str,
) -> Result<String, String> {
    let module = canonical_module_path(namespace, name)?;
    let path = format!("{module}.{extension}");
    windows_relative_path_key(&path)?;
    Ok(path)
}

pub(in crate::codegen::com) fn relative_module_specifier(
    from_namespace: &str,
    from_name: &str,
    to_namespace: &str,
    to_name: &str,
) -> String {
    let from = module_path(from_namespace, from_name);
    let to = module_path(to_namespace, to_name);
    let mut from_segments = from.split('/').collect::<Vec<_>>();
    from_segments.pop();
    let to_segments = to.split('/').collect::<Vec<_>>();
    let common = from_segments
        .iter()
        .zip(&to_segments)
        .take_while(|(left, right)| left == right)
        .count();
    let mut segments = vec![".."; from_segments.len() - common];
    segments.extend(to_segments[common..].iter().copied());
    let path = segments.join("/");
    if path.starts_with("../") {
        format!("{path}.js")
    } else {
        format!("./{path}.js")
    }
}

pub(in crate::codegen::com) fn canonical_namespace_depth(namespace: &str) -> usize {
    let path = crate::codegen::javascript_layout::canonical_namespace_path(namespace);
    if path.is_empty() {
        0
    } else {
        path.split('/').count()
    }
}

pub(in crate::codegen::com) fn rebase_runtime_import(
    import_name: String,
    parent_depth: usize,
) -> String {
    if !import_name.starts_with('.') || parent_depth == 0 {
        return import_name;
    }
    let prefix = "../".repeat(parent_depth);
    if let Some(import_name) = import_name.strip_prefix("./") {
        format!("{prefix}{import_name}")
    } else {
        format!("{prefix}{import_name}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeCleanupAvailability {
    NoneRequired,
    StandardSupported,
}

pub fn safe_interface_cleanup_availability(
    meta: &ComInterfaceMeta,
    winmd_paths: &str,
) -> Result<SafeCleanupAvailability, String> {
    let projected = project::project_com_interface(meta, winmd_paths)?;
    let owns_native_result = projected
        .methods
        .iter()
        .flat_map(|method| &method.results)
        .any(|result| {
            !matches!(
                result.conversion,
                ir::ResultConversion::Value
                    | ir::ResultConversion::BorrowedHandle
                    | ir::ResultConversion::Buffer
                    | ir::ResultConversion::PlainArray
            )
        });
    Ok(if owns_native_result {
        SafeCleanupAvailability::StandardSupported
    } else {
        SafeCleanupAvailability::NoneRequired
    })
}

pub fn generate_com_interface_files(
    meta: &ComInterfaceMeta,
    winmd_paths: &str,
) -> Result<ComGeneratedOutput, String> {
    let projected = project::project_com_interface(meta, winmd_paths)?;
    let mut output = javascript::render::render_com_interface(&projected);
    let mut pending = enumerator_interface_refs(&projected);
    let mut generated = std::collections::BTreeSet::from([(
        meta.interface.namespace.clone(),
        meta.interface.name.clone(),
    )]);
    let mut extras = output
        .extra_files
        .drain(..)
        .collect::<std::collections::BTreeMap<_, _>>();
    while let Some((namespace, name)) = pending.pop_first() {
        if !generated.insert((namespace.clone(), name.clone())) {
            continue;
        }
        let referenced = crate::com_metadata::parse_com_interface(winmd_paths, &namespace, &name)
            .ok_or_else(|| {
            format!("owning array element interface {namespace}.{name} could not be resolved")
        })?;
        let projected = project::project_com_interface(&referenced, winmd_paths)
            .or_else(|_| project::project_com_reference_interface(&referenced))?;
        pending.extend(enumerator_interface_refs(&projected));
        let rendered = javascript::render::render_com_interface(&projected);
        insert_extra(
            &mut extras,
            canonical_module_file_path(&namespace, &name, "js")?,
            rendered.js,
        )?;
        insert_extra(
            &mut extras,
            canonical_module_file_path(&namespace, &name, "d.ts")?,
            rendered.dts,
        )?;
        for (file, content) in rendered.extra_files {
            insert_extra(&mut extras, file, content)?;
        }
    }
    output.extra_files = extras.into_iter().collect();
    Ok(output)
}

fn enumerator_interface_refs(
    projected: &ir::ProjectedComInterface,
) -> std::collections::BTreeSet<(String, String)> {
    let mut refs = std::collections::BTreeSet::new();
    for method in &projected.methods {
        if let ir::ProjectedComMethodKind::EnumeratorNext {
            interface: Some(interface),
            ..
        } = &method.kind
        {
            refs.insert((interface.namespace.clone(), interface.name.clone()));
        }
        for typ in method
            .params
            .iter()
            .map(|param| &param.typ)
            .chain(method.results.iter().map(|result| &result.typ))
        {
            if let ir::ComType::OwningArray {
                interface: Some(interface),
                ..
            } = typ
            {
                refs.insert((interface.namespace.clone(), interface.name.clone()));
            }
        }
    }
    refs
}

fn insert_extra(
    extras: &mut std::collections::BTreeMap<String, String>,
    file: String,
    content: String,
) -> Result<(), String> {
    if let Some(existing) = extras.insert(file.clone(), content.clone())
        && existing != content
    {
        return Err(format!(
            "EnumeratorNext referenced interface generation produced conflicting `{file}`"
        ));
    }
    Ok(())
}

pub fn generate_com_coclass_files(
    meta: &ComCoclassMeta,
    winmd_paths: &str,
) -> Result<ComGeneratedOutput, String> {
    let projected = project::project_com_coclass(meta, winmd_paths)?;
    Ok(javascript::render::render_com_coclass(&projected))
}
