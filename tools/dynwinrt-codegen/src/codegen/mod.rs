// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

pub mod com;
pub mod common;
pub mod package;
pub mod winrt;

// Preserve the existing public module paths while callers migrate to
// `codegen::winrt::{javascript, python}`.
pub use winrt::{javascript, python};

// Preserve the existing public API while the implementations live under
// language-specific modules.
pub mod project {
    pub use super::javascript::project::*;
}

pub mod projected {
    pub use super::javascript::ir::*;
}

pub mod python_stub {
    pub use super::python::stubs::*;
}

pub mod render_dts {
    pub use super::javascript::render::declarations::*;
}

pub mod render_js {
    pub use super::javascript::render::javascript::*;
}

pub mod render_package_json {
    pub use super::package::*;
}

pub mod typescript {
    pub use super::javascript::generator::*;
}
