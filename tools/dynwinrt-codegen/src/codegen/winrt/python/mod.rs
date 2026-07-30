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
pub use naming::to_snake_case_filename;
