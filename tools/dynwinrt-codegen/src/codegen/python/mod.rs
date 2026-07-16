// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

pub(crate) mod docs;
mod generator;
pub(crate) mod method;
pub(crate) mod naming;
pub(crate) mod shared;
pub(crate) mod signature;
pub(crate) mod structs;
pub(crate) mod stub_helpers;
pub mod stubs;
pub(crate) mod type_helpers;

pub use generator::*;
pub use naming::to_snake_case_filename;
