// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Classic-COM metadata projection and JavaScript generation.

mod ir;
mod javascript;
mod model;
mod project;

use crate::com_metadata::{ComCoclassMeta, ComInterfaceMeta};

pub use javascript::render::ComGeneratedOutput;

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
        insert_extra(&mut extras, format!("{name}.js"), rendered.js)?;
        insert_extra(&mut extras, format!("{name}.d.ts"), rendered.dts)?;
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
