// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python package index generation.

use super::structs::py_struct_export_names;
use super::*;

/// Generate a Python `__init__.py` that re-exports all generated types.
pub fn generate_index(
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
            let struct_names: Vec<_> = collect_used_structs_from_class(class)
                .iter()
                .flat_map(|s| py_struct_export_names(s))
                .filter(|n| seen.insert(n.clone()))
                .collect();
            let module = python_module_name(&class.namespace, &class.name);
            if struct_names.is_empty() {
                out.push_str(&format!(
                    "from .{} import {}  # noqa: F401\n",
                    module, class.name
                ));
            } else {
                out.push_str(&format!(
                    "from .{} import {}, {}  # noqa: F401\n",
                    module,
                    class.name,
                    struct_names.join(", ")
                ));
            }
        }
    }
    let mut sorted_ifaces: Vec<_> = interfaces.iter().collect();
    sorted_ifaces.sort_by(|a, b| a.name.cmp(&b.name));
    for iface in sorted_ifaces {
        if !seen.insert(iface.name.clone()) {
            continue;
        }
        let is_delegate = iface.methods.iter().any(|m| m.name == ".ctor")
            && iface.methods.iter().any(|m| m.name == "Invoke");
        let module = python_module_name(&iface.namespace, &iface.name);
        let struct_names: Vec<_> = collect_used_structs_from_iface(iface)
            .iter()
            .flat_map(|s| py_struct_export_names(s))
            .filter(|n| seen.insert(n.clone()))
            .collect();
        if is_delegate {
            out.push_str(&format!(
                "from .{module} import IID_{iname}, {iname}_PARAM_TYPES  # noqa: F401\n",
                module = module,
                iname = iface.name
            ));
        } else {
            if struct_names.is_empty() {
                out.push_str(&format!(
                    "from .{module} import IID_{iname}, {iname}  # noqa: F401\n",
                    module = module,
                    iname = iface.name
                ));
            } else {
                out.push_str(&format!(
                    "from .{module} import IID_{iname}, {iname}, {structs}  # noqa: F401\n",
                    module = module,
                    iname = iface.name,
                    structs = struct_names.join(", ")
                ));
            }
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
        if let TypeMeta::Enum {
            namespace, name, ..
        } = en
        {
            if seen.insert(name.clone()) {
                let module = python_module_name(namespace, name);
                out.push_str(&format!("from .{} import {}  # noqa: F401\n", module, name));
            }
        }
    }
    out
}

/// Append new types to an existing `__init__.py`.
pub fn append_to_index(
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
        let module = python_module_name(&class.namespace, &class.name);
        if !exported_modules.contains(&module) && seen.insert(class.name.clone()) {
            let struct_names: Vec<_> = collect_used_structs_from_class(class)
                .iter()
                .flat_map(|s| py_struct_export_names(s))
                .filter(|n| seen.insert(n.clone()))
                .collect();
            if struct_names.is_empty() {
                out.push_str(&format!(
                    "from .{} import {}  # noqa: F401\n",
                    module, class.name
                ));
            } else {
                out.push_str(&format!(
                    "from .{} import {}, {}  # noqa: F401\n",
                    module,
                    class.name,
                    struct_names.join(", ")
                ));
            }
        }
    }

    let mut sorted_ifaces: Vec<_> = interfaces.iter().collect();
    sorted_ifaces.sort_by(|a, b| a.name.cmp(&b.name));
    for iface in sorted_ifaces {
        let module = python_module_name(&iface.namespace, &iface.name);
        if exported_modules.contains(&module) || !seen.insert(iface.name.clone()) {
            continue;
        }
        let is_delegate = iface.methods.iter().any(|m| m.name == ".ctor")
            && iface.methods.iter().any(|m| m.name == "Invoke");
        let struct_names: Vec<_> = collect_used_structs_from_iface(iface)
            .iter()
            .flat_map(|s| py_struct_export_names(s))
            .filter(|n| seen.insert(n.clone()))
            .collect();
        if is_delegate {
            out.push_str(&format!(
                "from .{module} import IID_{iname}, {iname}_PARAM_TYPES  # noqa: F401\n",
                module = module,
                iname = iface.name
            ));
        } else {
            if struct_names.is_empty() {
                out.push_str(&format!(
                    "from .{module} import IID_{iname}, {iname}  # noqa: F401\n",
                    module = module,
                    iname = iface.name
                ));
            } else {
                out.push_str(&format!(
                    "from .{module} import IID_{iname}, {iname}, {structs}  # noqa: F401\n",
                    module = module,
                    iname = iface.name,
                    structs = struct_names.join(", ")
                ));
            }
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
        if let TypeMeta::Enum {
            namespace, name, ..
        } = en
        {
            let module = python_module_name(namespace, name);
            if !exported_modules.contains(&module) && seen.insert(name.clone()) {
                out.push_str(&format!("from .{} import {}  # noqa: F401\n", module, name));
            }
        }
    }

    out
}
