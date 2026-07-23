// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Classic-COM metadata projection and JavaScript generation.

mod naming;
mod projection;
mod render;
mod type_mapping;

pub use render::{ComGeneratedOutput, generate_com_interface_files};
