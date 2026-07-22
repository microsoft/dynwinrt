// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

pub mod com;
pub mod common;
pub mod javascript;
pub mod python;
pub(crate) mod shared;

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
    pub use super::javascript::render::package_json::*;
}

pub mod typescript {
    pub use super::javascript::generator::*;
}
