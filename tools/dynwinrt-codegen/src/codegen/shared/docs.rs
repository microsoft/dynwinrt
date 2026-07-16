// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Language-neutral documentation input and lookup helpers.

use std::collections::HashMap;

/// Input for JSDoc / pydoc formatting: a normalized summary plus optional
/// parameter / return / deprecation text.
#[derive(Default)]
pub struct DocText<'a> {
    pub summary: Option<&'a str>,
    pub deprecated: Option<&'a str>,
    pub returns: Option<&'a str>,
    /// Pairs of (param_display_name, doc_text). Order preserved.
    pub params: Vec<(&'a str, &'a str)>,
}

impl<'a> DocText<'a> {
    pub fn is_empty(&self) -> bool {
        self.summary.is_none()
            && self.deprecated.is_none()
            && self.returns.is_none()
            && self.params.is_empty()
    }
}

/// Lookup helper: resolve a raw param name to its doc, ignoring case.
/// Used by call sites that need to translate codegen display names back to
/// raw names.
pub fn find_param_doc<'a>(
    param_docs: &'a HashMap<String, String>,
    raw_name: &str,
) -> Option<&'a str> {
    if let Some(v) = param_docs.get(raw_name) {
        return Some(v.as_str());
    }
    for (k, v) in param_docs.iter() {
        if k.eq_ignore_ascii_case(raw_name) {
            return Some(v.as_str());
        }
    }
    None
}
