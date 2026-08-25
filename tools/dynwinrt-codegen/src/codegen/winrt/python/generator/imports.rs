// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python generated-module import formatting.

use super::*;

/// Format a Python import line based on type kind.
pub(super) fn format_py_type_import(namespace: &str, name: &str, kind: TypeKind) -> String {
    let module = python_module_name(namespace, name);
    if kind == TypeKind::Interface {
        format!("from .{module} import IID_{name}, {name}  # noqa: F401\n")
    } else if kind == TypeKind::Class {
        format!("from .{module} import {name}, {name}Like  # noqa: F401\n")
    } else {
        format!("from .{module} import {name}  # noqa: F401\n")
    }
}

pub(super) fn emit_type_checking_imports(out: &mut String, imports: Vec<String>) {
    let mut imports = imports;
    imports.sort();
    imports.dedup();
    if imports.is_empty() {
        return;
    }

    out.push_str("if TYPE_CHECKING:\n");
    for import in imports {
        out.push_str("    ");
        out.push_str(&import);
    }
    out.push('\n');
}
