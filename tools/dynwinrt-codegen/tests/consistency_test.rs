// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Structural consistency test: for every type we generate, the `.js` and `.d.ts`
//! outputs must agree on the set of exported members (class names, method names,
//! getter/setter names, static members).
//!
//! This catches drift where a Mode::Js branch is updated but Mode::Dts is not
//! (or vice versa), which would leave consumers with IntelliSense that doesn't
//! match the runtime behavior.

use std::collections::{HashMap, HashSet};

use dynwinrt_codegen::codegen::projected::{ProjectedFile, ProjectedMember};
use dynwinrt_codegen::codegen::{project, render_dts, render_js};
use dynwinrt_codegen::meta;
use dynwinrt_codegen::types::TypeMeta;

use regex::Regex;

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

/// Extract exported class names from generated code.
/// Supports both ESM `export class X` and our CJS output shape (top-level
/// `class X { ... }` followed by `exports.X = X;`).
fn extract_class_names(code: &str) -> Vec<String> {
    let esm_re = Regex::new(r"(?m)^export\s+(?:declare\s+)?class\s+(\w+)").unwrap();
    let cjs_class_re = Regex::new(r"(?m)^class\s+(\w+)").unwrap();
    let cjs_export_re = Regex::new(r"(?m)^exports\.(\w+)\s*=").unwrap();

    let mut names: Vec<String> = esm_re
        .captures_iter(code)
        .map(|c| c[1].to_string())
        .collect();
    if !names.is_empty() {
        return names;
    }
    // CJS: a class is emitted at top-level *and* re-exported via `exports.X = X`.
    // Intersect the two sets to identify true classes (vs plain consts/functions).
    let cjs_classes: HashSet<String> = cjs_class_re
        .captures_iter(code)
        .map(|c| c[1].to_string())
        .collect();
    let cjs_exports: Vec<String> = cjs_export_re
        .captures_iter(code)
        .map(|c| c[1].to_string())
        .collect();
    for name in cjs_exports {
        if cjs_classes.contains(&name) && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Extract member names from a class body (methods, getters, setters, static).
/// Looks for patterns like:
///   `    methodName(` or `    get propName()` or `    static methodName(`
fn extract_members(code: &str, class_name: &str) -> Vec<String> {
    // Find the class body start. Supports both ESM `export class X {` and CJS
    // `class X {` (no `export` prefix, since our CJS conversion strips it).
    let class_pattern = format!(
        r"(?m)^(?:export\s+(?:declare\s+)?)?class\s+{}(?:\s+extends\s+\S+)?\s*\{{",
        regex::escape(class_name)
    );
    let class_re = Regex::new(&class_pattern).unwrap();
    let class_start = match class_re.find(code) {
        Some(m) => m.end(),
        None => return Vec::new(),
    };

    // Find the matching closing brace by counting braces
    let body = &code[class_start..];
    let mut depth = 1i32;
    let mut class_end = body.len();
    for (i, ch) in body.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    class_end = i;
                    break;
                }
            }
            _ => {}
        }
    }
    let class_body = &body[..class_end];

    // Extract member signatures at indent level 1 (4 spaces)
    // Handles: `name(`, `name<T>(`, `get name()`, `static name(`, `[Symbol.x](`,
    //          `*[Symbol.x](` (generators), `readonly name:`, `name;` (field declarations),
    //          `async name(`, `static async name(`
    let member_re = Regex::new(
        r"(?m)^    (?:static\s+)?(?:readonly\s+)?(?:async\s+)?(?:get\s+|set\s+)?\*?(\w+|\[Symbol\.\w+\])\s*[<(;:]"
    ).unwrap();

    let mut members: Vec<String> = member_re
        .captures_iter(class_body)
        .map(|c| c[1].to_string())
        // Filter out internal/private members that only exist in JS
        .filter(|n| !n.starts_with("s_") && !n.starts_with("f_") && !n.starts_with('_'))
        .collect();
    members.sort();
    members.dedup();
    members
}

/// Extract exported enum/const/function names (NOT classes — those come from
/// `extract_class_names`).
/// Supports both ESM `export const|function|enum X` and our CJS output shape
/// where top-level `const|function X` is re-exported via `exports.X = X;`.
fn extract_exports(code: &str) -> Vec<String> {
    let esm_re =
        Regex::new(r"(?m)^export\s+(?:declare\s+)?(?:const|function|enum)\s+(\w+)").unwrap();
    let esm_names: Vec<String> = esm_re
        .captures_iter(code)
        .map(|c| c[1].to_string())
        .collect();
    if !esm_names.is_empty() {
        return esm_names;
    }
    // CJS: every top-level declaration that we re-export is `exports.X = X;`.
    // Filter out class names — they are tested separately via `extract_class_names`.
    let cjs_class_re = Regex::new(r"(?m)^class\s+(\w+)").unwrap();
    let cjs_classes: HashSet<String> = cjs_class_re
        .captures_iter(code)
        .map(|c| c[1].to_string())
        .collect();
    let cjs_re = Regex::new(r"(?m)^exports\.(\w+)\s*=").unwrap();
    cjs_re
        .captures_iter(code)
        .filter_map(|c| {
            let name = c[1].to_string();
            if cjs_classes.contains(&name) {
                None
            } else {
                Some(name)
            }
        })
        .collect()
}

/// Helper: build known_types, delegate_type_names, shared_iids from parsed metadata.
fn setup_metadata(
    winmd: &str,
    ns: &str,
    class_name: &str,
) -> Option<(
    Vec<meta::ClassMeta>,
    Vec<meta::InterfaceMeta>,
    Vec<TypeMeta>,
    HashSet<String>,
    HashSet<String>,
    HashSet<project::StandaloneInterfaceIdentity>,
    HashMap<String, String>,
    HashMap<String, Vec<String>>,
    HashMap<String, Vec<String>>,
)> {
    let classes = match meta::parse_class(winmd, ns, class_name) {
        Some(c) => vec![c],
        None => return None,
    };
    let deps = meta::resolve_dependencies(winmd, &classes, &[], &[]);
    let mut all_classes = classes;
    all_classes.extend(deps.classes);
    let all_interfaces = deps.interfaces;
    let all_enums = deps.enums;

    let mut known_types: HashSet<String> = HashSet::new();
    for c in &all_classes {
        known_types.insert(c.name.clone());
    }
    for i in &all_interfaces {
        known_types.insert(i.name.clone());
    }
    for e in &all_enums {
        if let TypeMeta::Enum { name, .. } = e {
            known_types.insert(name.clone());
        }
    }
    let delegate_type_names: HashSet<String> = all_interfaces
        .iter()
        .filter(|i| {
            i.methods.iter().any(|m| m.name == ".ctor")
                && i.methods.iter().any(|m| m.name == "Invoke")
        })
        .map(|i| i.name.clone())
        .collect();
    let shared_iids: HashSet<project::StandaloneInterfaceIdentity> = HashSet::new();
    let (delegate_sigs, delegate_sig_refs, delegate_param_wraps) =
        project::build_delegate_signatures(&all_interfaces, &delegate_type_names, &known_types);

    Some((
        all_classes,
        all_interfaces,
        all_enums,
        known_types,
        delegate_type_names,
        shared_iids,
        delegate_sigs,
        delegate_sig_refs,
        delegate_param_wraps,
    ))
}

/// Extract js_only method names from a projected file (methods hidden from DTS intentionally).
fn extract_js_only_names(file: &ProjectedFile) -> HashSet<String> {
    let mut names = HashSet::new();
    let collect = |members: &[ProjectedMember], names: &mut HashSet<String>| {
        for m in members {
            if let ProjectedMember::Method(pm) = m {
                if pm.js_only {
                    names.insert(pm.name.clone());
                }
            }
        }
    };
    for class in &file.classes {
        collect(&class.members, &mut names);
        for ri in &class.required_ifaces {
            collect(&ri.members, &mut names);
        }
    }
    for iface in &file.ifaces {
        collect(&iface.members, &mut names);
    }
    names
}

/// For a given (js, dts) pair, assert structural consistency.
fn assert_js_dts_consistent(js: &str, dts: &str, type_name: &str, js_only_names: &HashSet<String>) {
    let mut errors: Vec<String> = Vec::new();

    // 1. Same set of exported class names
    let js_classes = extract_class_names(js);
    let dts_classes = extract_class_names(dts);
    if js_classes != dts_classes {
        errors.push(format!(
            "  class list mismatch:\n    JS:  {:?}\n    DTS: {:?}",
            js_classes, dts_classes
        ));
    }

    // 2. For each class, check member consistency.
    // JS may have members intentionally hidden from DTS (constructor, as, from, _obj).
    // Methods with DynWinRtArray params/return are js_only (hidden from DTS).
    // But DTS must not have members absent from JS.
    let intentionally_hidden =
        |name: &str| -> bool { name == "constructor" || name == "as" || name == "from" };
    for cls in &js_classes {
        let js_members = extract_members(js, cls);
        let dts_members = extract_members(dts, cls);
        let js_set: HashSet<_> = js_members.iter().collect();
        let dts_set: HashSet<_> = dts_members.iter().collect();
        let in_js_only: Vec<_> = js_set
            .difference(&dts_set)
            .filter(|n| !intentionally_hidden(n) && !js_only_names.contains(n.as_str()))
            .collect();
        let in_dts_only: Vec<_> = dts_set.difference(&js_set).collect();
        if !in_js_only.is_empty() || !in_dts_only.is_empty() {
            errors.push(format!(
                "  class {} member mismatch:\n    JS-only (unexpected):  {:?}\n    DTS-only: {:?}",
                cls, in_js_only, in_dts_only
            ));
        }
    }

    // 3. Same set of top-level exports (enums, consts, functions)
    let js_exports = extract_exports(js);
    let dts_exports = extract_exports(dts);
    // Filter: JS may have internal consts (IID_, registration vars) that DTS omits
    let js_public: Vec<_> = js_exports
        .iter()
        .filter(|n| !n.starts_with("IID_") && !n.starts_with('_'))
        .cloned()
        .collect();
    let dts_public: Vec<_> = dts_exports
        .iter()
        .filter(|n| !n.starts_with("IID_") && !n.starts_with('_'))
        .cloned()
        .collect();
    let js_pub_set: HashSet<_> = js_public.iter().collect();
    let dts_pub_set: HashSet<_> = dts_public.iter().collect();
    let missing_in_dts: Vec<_> = js_pub_set.difference(&dts_pub_set).collect();
    let extra_in_dts: Vec<_> = dts_pub_set.difference(&js_pub_set).collect();
    if !missing_in_dts.is_empty() || !extra_in_dts.is_empty() {
        errors.push(format!(
            "  top-level export mismatch:\n    JS-only:  {:?}\n    DTS-only: {:?}",
            missing_in_dts, extra_in_dts
        ));
    }

    assert!(
        errors.is_empty(),
        "JS/DTS structural drift for {}:\n{}",
        type_name,
        errors.join("\n")
    );
}

/// Test structural consistency for Uri (class with many methods, properties, IStringable).
#[test]
fn js_dts_structural_consistency_uri() {
    let (
        all_classes,
        all_interfaces,
        _,
        known_types,
        delegate_type_names,
        shared_iids,
        delegate_sigs,
        delegate_sig_refs,
        delegate_param_wraps,
    ) = match setup_metadata(WINDOWS_WINMD, "Windows.Foundation", "Uri") {
        Some(v) => v,
        None => {
            eprintln!("Skipping: Windows.winmd not found");
            return;
        }
    };

    for class in &all_classes {
        let projected = project::project_class(
            class,
            &known_types,
            &delegate_type_names,
            &shared_iids,
            &delegate_sigs,
            &delegate_sig_refs,
            &delegate_param_wraps,
        );
        let js = render_js::render(&projected);
        let dts = render_dts::render(&projected);
        let js_only = extract_js_only_names(&projected);
        assert_js_dts_consistent(&js, &dts, &class.name, &js_only);
    }
    for iface in &all_interfaces {
        let projected = project::project_interface(
            iface,
            &known_types,
            &delegate_type_names,
            &delegate_sigs,
            &delegate_sig_refs,
            &delegate_param_wraps,
        );
        let js = render_js::render(&projected);
        let dts = render_dts::render(&projected);
        let js_only = extract_js_only_names(&projected);
        assert_js_dts_consistent(&js, &dts, &iface.name, &js_only);
    }
}

/// Test structural consistency for StorageFile (has events, async, required interfaces).
#[test]
fn js_dts_structural_consistency_storage_file() {
    let (
        all_classes,
        all_interfaces,
        _,
        known_types,
        delegate_type_names,
        shared_iids,
        delegate_sigs,
        delegate_sig_refs,
        delegate_param_wraps,
    ) = match setup_metadata(WINDOWS_WINMD, "Windows.Storage", "StorageFile") {
        Some(v) => v,
        None => {
            eprintln!("Skipping: Windows.winmd not found");
            return;
        }
    };

    for class in &all_classes {
        let projected = project::project_class(
            class,
            &known_types,
            &delegate_type_names,
            &shared_iids,
            &delegate_sigs,
            &delegate_sig_refs,
            &delegate_param_wraps,
        );
        let js = render_js::render(&projected);
        let dts = render_dts::render(&projected);
        let js_only = extract_js_only_names(&projected);
        assert_js_dts_consistent(&js, &dts, &class.name, &js_only);
    }
    for iface in &all_interfaces {
        let projected = project::project_interface(
            iface,
            &known_types,
            &delegate_type_names,
            &delegate_sigs,
            &delegate_sig_refs,
            &delegate_param_wraps,
        );
        let js = render_js::render(&projected);
        let dts = render_dts::render(&projected);
        let js_only = extract_js_only_names(&projected);
        assert_js_dts_consistent(&js, &dts, &iface.name, &js_only);
    }
}

/// Test structural consistency for UserWatcher (has multiple event pairs).
#[test]
fn js_dts_structural_consistency_user_watcher() {
    let winmd = WINDOWS_WINMD;
    let classes = match meta::parse_class(winmd, "Windows.System", "UserWatcher") {
        Some(c) => vec![c],
        None => {
            eprintln!("Skipping: Windows.winmd not found");
            return;
        }
    };
    let deps = meta::resolve_dependencies(winmd, &classes, &[], &[]);
    let mut all_classes = classes;
    all_classes.extend(deps.classes);
    let all_interfaces = deps.interfaces;

    let mut known_types: HashSet<String> = HashSet::new();
    for c in &all_classes {
        known_types.insert(c.name.clone());
    }
    for i in &all_interfaces {
        known_types.insert(i.name.clone());
    }
    let delegate_type_names: HashSet<String> = all_interfaces
        .iter()
        .filter(|i| {
            i.methods.iter().any(|m| m.name == ".ctor")
                && i.methods.iter().any(|m| m.name == "Invoke")
        })
        .map(|i| i.name.clone())
        .collect();
    let shared_iids: HashSet<project::StandaloneInterfaceIdentity> = HashSet::new();
    let (delegate_sigs, delegate_sig_refs, delegate_param_wraps) =
        project::build_delegate_signatures(&all_interfaces, &delegate_type_names, &known_types);

    for class in &all_classes {
        let projected = project::project_class(
            class,
            &known_types,
            &delegate_type_names,
            &shared_iids,
            &delegate_sigs,
            &delegate_sig_refs,
            &delegate_param_wraps,
        );
        let js = render_js::render(&projected);
        let dts = render_dts::render(&projected);
        let js_only = extract_js_only_names(&projected);
        assert_js_dts_consistent(&js, &dts, &class.name, &js_only);
    }
}
