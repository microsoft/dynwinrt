// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python generated-module import formatting.

use super::*;

/// Format a Python import line based on type kind.
pub(super) fn format_py_type_import(
    context: &PythonProjectionContext,
    namespace: &str,
    name: &str,
    kind: TypeKind,
) -> String {
    let identity_kind = match kind {
        TypeKind::Class => crate::types::TypeIdentityKind::Class,
        TypeKind::Enum => crate::types::TypeIdentityKind::Enum,
        TypeKind::Interface => crate::types::TypeIdentityKind::Interface,
    };
    let identity = crate::types::TypeIdentity::named(identity_kind, namespace, name);
    let module = context.implementation_module(&identity);
    let projected_name = context.projected_name(&identity);
    let reference_name = context.reference_name(&identity);
    let imported = |name: &str, alias: &str| {
        if name == alias {
            name.to_string()
        } else {
            format!("{name} as {alias}")
        }
    };
    if kind == TypeKind::Interface {
        format!(
            "from .{module} import {}, {}  # noqa: F401\n",
            imported(
                &format!("IID_{projected_name}"),
                &format!("IID_{reference_name}")
            ),
            imported(&projected_name, &reference_name)
        )
    } else if kind == TypeKind::Class {
        format!(
            "from .{module} import {}, {}  # noqa: F401\n",
            imported(&projected_name, &reference_name),
            imported(
                &format!("{projected_name}Like"),
                &format!("{reference_name}Like")
            )
        )
    } else {
        format!(
            "from .{module} import {}  # noqa: F401\n",
            imported(&projected_name, &reference_name)
        )
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
