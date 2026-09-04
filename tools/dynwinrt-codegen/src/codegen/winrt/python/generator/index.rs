// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python package index generation.

use super::super::native_types::foundation_type;
use super::structs::py_struct_export_names;
use super::*;

/// Generate a Python `__init__.py` that re-exports all generated types.
pub fn generate_index(
    context: &PythonProjectionContext,
    classes: &[ClassMeta],
    interfaces: &[InterfaceMeta],
    enums: &[TypeMeta],
) -> String {
    let mut out = String::new();
    let mut seen: HashSet<String> = HashSet::new();
    out.push_str(HEADER);
    let mut sorted_classes: Vec<_> = classes.iter().collect();
    sorted_classes.sort_by(|a, b| a.name.cmp(&b.name));
    for class in sorted_classes {
        if seen.insert(class.name.clone()) {
            let identity = crate::types::TypeIdentity::named(
                crate::types::TypeIdentityKind::Class,
                class.namespace.clone(),
                class.name.clone(),
            );
            let module = context.implementation_module(&identity);
            let name = context.projected_name(&identity);
            out.push_str(&format!("from .{} import {}  # noqa: F401\n", module, name));
        }
    }
    let mut sorted_ifaces: Vec<_> = interfaces.iter().collect();
    sorted_ifaces.sort_by(|a, b| a.name.cmp(&b.name));
    for iface in sorted_ifaces {
        if !seen.insert(iface.name.clone()) {
            continue;
        }
        let is_delegate = iface.is_delegate();
        let identity = iface.type_identity();
        let module = context.implementation_module(&identity);
        let name = context.projected_name(&identity);
        if is_delegate {
            out.push_str(&format!(
                "from .{module} import IID_{iname}, {iname}_PARAM_TYPES  # noqa: F401\n",
                module = module,
                iname = name
            ));
        } else {
            out.push_str(&format!(
                "from .{module} import IID_{iname}, {iname}  # noqa: F401\n",
                module = module,
                iname = name
            ));
        }
    }
    let mut sorted_enums: Vec<_> = enums.iter().collect();
    sorted_enums.sort_by(|a, b| {
        let name_a = match a {
            TypeMeta::Enum { name, .. } => name.as_str(),
            _ => "",
        };
        let name_b = match b {
            TypeMeta::Enum { name, .. } => name.as_str(),
            _ => "",
        };
        name_a.cmp(name_b)
    });
    for en in sorted_enums {
        if let TypeMeta::Enum { name, .. } = en {
            if seen.insert(name.clone()) {
                let identity = en.type_identity();
                let module = context.implementation_module(&identity);
                let name = context.projected_name(&identity);
                out.push_str(&format!("from .{} import {}  # noqa: F401\n", module, name));
            }
        }
    }
    out
}

pub fn generate_public_index(
    context: &PythonProjectionContext,
    classes: &[ClassMeta],
    interfaces: &[InterfaceMeta],
    enums: &[TypeMeta],
) -> String {
    let mut out = String::from(HEADER);
    let mut seen = HashSet::new();

    let mut classes = classes.iter().collect::<Vec<_>>();
    classes.sort_by(|left, right| left.name.cmp(&right.name));
    for class in classes {
        if seen.insert(class.name.clone()) {
            let identity = crate::types::TypeIdentity::named(
                crate::types::TypeIdentityKind::Class,
                class.namespace.clone(),
                class.name.clone(),
            );
            let name = context.projected_name(&identity);
            out.push_str(&format!(
                "from .{} import {}  # noqa: F401\n",
                context.public_qualified_module(&identity),
                name
            ));
        }
    }

    let mut interfaces = interfaces.iter().collect::<Vec<_>>();
    interfaces.sort_by(|left, right| left.name.cmp(&right.name));
    for interface in interfaces {
        let is_delegate = interface.is_delegate();
        if !is_delegate && seen.insert(interface.name.clone()) {
            let identity = interface.type_identity();
            let name = context.projected_name(&identity);
            out.push_str(&format!(
                "from .{} import {}  # noqa: F401\n",
                context.public_qualified_module(&identity),
                name
            ));
        }
    }

    let mut enums = enums.iter().collect::<Vec<_>>();
    enums.sort_by_key(|typ| match typ {
        TypeMeta::Enum { name, .. } => name.as_str(),
        _ => "",
    });
    for typ in enums {
        let TypeMeta::Enum { name, .. } = typ else {
            continue;
        };
        if seen.insert(name.clone()) {
            let identity = typ.type_identity();
            let name = context.projected_name(&identity);
            out.push_str(&format!(
                "from .{} import {}  # noqa: F401\n",
                context.public_qualified_module(&identity),
                name
            ));
        }
    }
    out
}

pub fn generate_struct_index(context: &PythonProjectionContext, structs: &[TypeMeta]) -> String {
    let mut out = String::from(HEADER);
    let mut seen = HashSet::new();
    let mut sorted = structs.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|typ| match typ {
        TypeMeta::Struct {
            namespace, name, ..
        } => format!("{namespace}.{name}"),
        _ => String::new(),
    });
    for typ in sorted {
        let TypeMeta::Struct {
            namespace, name, ..
        } = typ
        else {
            continue;
        };
        if !seen.insert((namespace, name)) {
            continue;
        }
        out.push_str(&format!(
            "from .{} import {}  # noqa: F401\n",
            context.implementation_module_for_type(typ),
            py_struct_export_names(typ).join(", ")
        ));
    }
    out
}

pub fn generate_public_struct_index(
    context: &PythonProjectionContext,
    structs: &[TypeMeta],
) -> String {
    let mut out = String::from(HEADER);
    let mut seen = HashSet::new();
    let mut sorted = structs.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|typ| match typ {
        TypeMeta::Struct {
            namespace, name, ..
        } => format!("{namespace}.{name}"),
        _ => String::new(),
    });
    for typ in sorted {
        let TypeMeta::Struct {
            namespace, name, ..
        } = typ
        else {
            continue;
        };
        if foundation_type(typ).is_some() || !seen.insert((namespace, name)) {
            continue;
        }
        out.push_str(&format!(
            "from .{} import {}  # noqa: F401\n",
            context.public_qualified_module(&typ.type_identity()),
            name
        ));
    }
    out
}

/// Append new types to an existing `__init__.py`.
pub fn append_to_index(
    context: &PythonProjectionContext,
    existing: &str,
    classes: &[ClassMeta],
    interfaces: &[InterfaceMeta],
    enums: &[TypeMeta],
) -> String {
    let mut seen: HashSet<String> = HashSet::new();
    let mut exported_modules: HashSet<String> = HashSet::new();
    for line in existing.lines() {
        // Parse: from .module import Name1, Name2  # noqa: F401
        if line.starts_with("from .") {
            if let Some(import_start) = line.find("import ") {
                let after_import = &line[import_start + 7..];
                // Strip trailing comment
                let names_part = if let Some(comment_pos) = after_import.find('#') {
                    &after_import[..comment_pos]
                } else {
                    after_import
                };
                for name in names_part.split(',') {
                    let name = name.trim();
                    if !name.is_empty() {
                        seen.insert(name.to_string());
                    }
                }
            }
            // Track module name from `from .module`
            if let Some(rest) = line.strip_prefix("from .") {
                if let Some(space_pos) = rest.find(' ') {
                    let module = &rest[..space_pos];
                    exported_modules.insert(module.to_string());
                }
            }
        }
    }

    let mut out = existing.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }

    let mut sorted_classes: Vec<_> = classes.iter().collect();
    sorted_classes.sort_by(|a, b| a.name.cmp(&b.name));
    for class in sorted_classes {
        let identity = crate::types::TypeIdentity::named(
            crate::types::TypeIdentityKind::Class,
            class.namespace.clone(),
            class.name.clone(),
        );
        let module = context.implementation_module(&identity);
        let name = context.projected_name(&identity);
        if !exported_modules.contains(&module) && seen.insert(class.name.clone()) {
            out.push_str(&format!("from .{} import {}  # noqa: F401\n", module, name));
        }
    }

    let mut sorted_ifaces: Vec<_> = interfaces.iter().collect();
    sorted_ifaces.sort_by(|a, b| a.name.cmp(&b.name));
    for iface in sorted_ifaces {
        let identity = iface.type_identity();
        let module = context.implementation_module(&identity);
        let name = context.projected_name(&identity);
        if exported_modules.contains(&module) || !seen.insert(iface.name.clone()) {
            continue;
        }
        let is_delegate = iface.is_delegate();
        if is_delegate {
            out.push_str(&format!(
                "from .{module} import IID_{iname}, {iname}_PARAM_TYPES  # noqa: F401\n",
                module = module,
                iname = name
            ));
        } else {
            out.push_str(&format!(
                "from .{module} import IID_{iname}, {iname}  # noqa: F401\n",
                module = module,
                iname = name
            ));
        }
    }

    let mut sorted_enums: Vec<_> = enums.iter().collect();
    sorted_enums.sort_by(|a, b| {
        let name_a = match a {
            TypeMeta::Enum { name, .. } => name.as_str(),
            _ => "",
        };
        let name_b = match b {
            TypeMeta::Enum { name, .. } => name.as_str(),
            _ => "",
        };
        name_a.cmp(name_b)
    });
    for en in sorted_enums {
        if let TypeMeta::Enum { name, .. } = en {
            let identity = en.type_identity();
            let module = context.implementation_module(&identity);
            let projected_name = context.projected_name(&identity);
            if !exported_modules.contains(&module) && seen.insert(name.clone()) {
                out.push_str(&format!(
                    "from .{} import {}  # noqa: F401\n",
                    module, projected_name
                ));
            }
        }
    }

    out
}
