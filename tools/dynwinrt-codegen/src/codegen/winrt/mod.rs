// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Windows Runtime code generation.

pub mod extensions;
pub mod javascript;
pub mod python;
pub(crate) mod shared;

pub(crate) const IBUFFER_IID: &str = "905a0fe0-bc53-11df-8c49-001e4fc686da";

pub(crate) fn is_ibuffer_interface(namespace: &str, name: &str, iid: &str) -> bool {
    namespace == "Windows.Storage.Streams"
        && name == "IBuffer"
        && iid.eq_ignore_ascii_case(IBUFFER_IID)
}

pub(crate) fn is_buffer_class(namespace: &str, name: &str) -> bool {
    namespace == "Windows.Storage.Streams" && name == "Buffer"
}
