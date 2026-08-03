// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Classic-COM metadata projection and JavaScript generation.

mod ir;
mod javascript;
mod project;

use crate::com_metadata::{ComCoclassMeta, ComInterfaceMeta};

pub use javascript::render::ComGeneratedOutput;

pub fn generate_com_interface_files(
    meta: &ComInterfaceMeta,
    winmd_paths: &str,
) -> Result<ComGeneratedOutput, String> {
    let projected = project::project_com_interface(meta, winmd_paths)?;
    Ok(javascript::render::render_com_interface(&projected))
}

pub fn generate_com_coclass_files(
    meta: &ComCoclassMeta,
    winmd_paths: &str,
) -> Result<ComGeneratedOutput, String> {
    let projected = project::project_com_coclass(meta, winmd_paths)?;
    Ok(javascript::render::render_com_coclass(&projected))
}
