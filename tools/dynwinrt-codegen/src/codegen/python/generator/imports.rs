// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python generated-module import formatting.

use super::*;

/// Format a Python import line based on type kind.
pub(super) fn format_py_type_import(name: &str, kind: TypeKind) -> String {
    let module = to_snake_case_filename(name);
    if kind == TypeKind::Interface {
        format!("from .{module} import IID_{name}, {name}  # noqa: F401\n")
    } else {
        format!("from .{module} import {name}  # noqa: F401\n")
    }
}
