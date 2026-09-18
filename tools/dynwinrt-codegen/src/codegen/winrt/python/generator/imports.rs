// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python generated-module import formatting.

pub(super) use super::super::naming::format_py_type_import;

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
