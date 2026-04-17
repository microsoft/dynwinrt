// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! E2E snapshot test: generate TypeScript for Windows.Foundation.Uri and compare
//! against committed snapshots.
//!
//! To update snapshots after an intentional output change, run:
//!   cargo run -p dynwinrt-codegen -- generate --namespace Windows.Foundation --class-name Uri --output tests/snapshots/uri

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use dynwinrt_codegen::codegen::common::to_snake_case_filename;
use dynwinrt_codegen::codegen::python_stub;
use dynwinrt_codegen::codegen::typescript;
use dynwinrt_codegen::meta;
use dynwinrt_codegen::types::TypeMeta;

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

/// Generate TypeScript for Uri and compare every file against the snapshot.
#[test]
fn snapshot_uri_class() {
    let winmd = WINDOWS_WINMD;
    let classes = match meta::parse_class(winmd, "Windows.Foundation", "Uri") {
        Some(c) => vec![c],
        None => {
            eprintln!("Skipping snapshot test: Windows.winmd not found");
            return;
        }
    };

    let deps = meta::resolve_dependencies(winmd, &classes, &[], &[]);
    let mut all_classes = classes;
    all_classes.extend(deps.classes);
    let all_interfaces = deps.interfaces;
    let all_enums = deps.enums;

    let mut known_types: HashSet<String> = HashSet::new();
    for c in &all_classes { known_types.insert(c.name.clone()); }
    for i in &all_interfaces { known_types.insert(i.name.clone()); }
    for e in &all_enums {
        if let TypeMeta::Enum { name, .. } = e { known_types.insert(name.clone()); }
    }

    let delegate_type_names: HashSet<String> = all_interfaces.iter()
        .filter(|i| i.methods.iter().any(|m| m.name == ".ctor") && i.methods.iter().any(|m| m.name == "Invoke"))
        .map(|i| i.name.clone())
        .collect();

    let shared_iids: HashSet<String> = HashSet::new();

    // Generate all files into a map
    let mut generated: HashMap<String, String> = HashMap::new();
    for iface in &all_interfaces {
        let code = typescript::generate_interface(iface, &known_types, &delegate_type_names);
        generated.insert(format!("{}.ts", iface.name), code);
    }
    for class in &all_classes {
        let code = typescript::generate_class(class, &known_types, &delegate_type_names, &shared_iids);
        generated.insert(format!("{}.ts", class.name), code);
    }

    // Compare against snapshots
    let snapshot_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/uri");
    assert!(snapshot_dir.exists(), "Snapshot directory not found: {}", snapshot_dir.display());

    let mut mismatches = Vec::new();
    for (filename, actual) in &generated {
        let snapshot_path = snapshot_dir.join(filename);
        if !snapshot_path.exists() {
            mismatches.push(format!("  missing snapshot: {}", filename));
            continue;
        }
        let expected = fs::read_to_string(&snapshot_path)
            .unwrap_or_else(|e| panic!("Failed to read snapshot {}: {}", snapshot_path.display(), e));
        if *actual != expected {
            mismatches.push(format!("  differs: {}", filename));
        }
    }

    // Check for extra snapshot files not in generated output
    if let Ok(entries) = fs::read_dir(&snapshot_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".ts") && !generated.contains_key(&name) {
                mismatches.push(format!("  extra snapshot not generated: {}", name));
            }
        }
    }

    if !mismatches.is_empty() {
        panic!(
            "Snapshot mismatch for Windows.Foundation.Uri!\n{}\n\n\
             To update snapshots, run:\n  \
             cargo run -p dynwinrt-codegen -- generate --namespace Windows.Foundation --class-name Uri --output tests/snapshots/uri",
            mismatches.join("\n")
        );
    }
}


/// Generate .pyi stubs for Uri and compare against committed snapshots.
#[test]
fn snapshot_uri_pyi_class() {
    let winmd = WINDOWS_WINMD;
    let classes = match meta::parse_class(winmd, "Windows.Foundation", "Uri") {
        Some(c) => vec![c],
        None => {
            eprintln!("Skipping snapshot test: Windows.winmd not found");
            return;
        }
    };

    let deps = meta::resolve_dependencies(winmd, &classes, &[], &[]);
    let mut all_classes = classes;
    all_classes.extend(deps.classes);
    let all_interfaces = deps.interfaces;
    let all_enums = deps.enums;

    let mut known_types: HashSet<String> = HashSet::new();
    for c in &all_classes { known_types.insert(c.name.clone()); }
    for i in &all_interfaces { known_types.insert(i.name.clone()); }
    for e in &all_enums {
        if let TypeMeta::Enum { name, .. } = e { known_types.insert(name.clone()); }
    }
    let delegate_type_names: HashSet<String> = all_interfaces.iter()
        .filter(|i| i.methods.iter().any(|m| m.name == ".ctor") && i.methods.iter().any(|m| m.name == "Invoke"))
        .map(|i| i.name.clone())
        .collect();
    let shared_iids: HashSet<String> = HashSet::new();

    let mut generated: HashMap<String, String> = HashMap::new();
    for iface in &all_interfaces {
        let code = python_stub::generate_interface_stub(iface, &known_types, &delegate_type_names);
        generated.insert(format!("{}.pyi", to_snake_case_filename(&iface.name)), code);
    }
    for class in &all_classes {
        let code = python_stub::generate_class_stub(class, &known_types, &delegate_type_names, &shared_iids);
        generated.insert(format!("{}.pyi", to_snake_case_filename(&class.name)), code);
    }
    let index = python_stub::generate_index_stub(&all_classes, &all_interfaces, &all_enums);
    generated.insert("__init__.pyi".to_string(), index);

    let snapshot_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/uri_pyi");
    assert!(snapshot_dir.exists(), "Snapshot directory not found: {}", snapshot_dir.display());

    let mut mismatches = Vec::new();
    for (filename, actual) in &generated {
        let snapshot_path = snapshot_dir.join(filename);
        if !snapshot_path.exists() {
            mismatches.push(format!("  missing snapshot: {}", filename));
            continue;
        }
        let expected = fs::read_to_string(&snapshot_path).unwrap();
        if *actual != expected {
            mismatches.push(format!("  differs: {}", filename));
        }
    }
    if let Ok(entries) = fs::read_dir(&snapshot_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".pyi") && !generated.contains_key(&name) {
                mismatches.push(format!("  extra snapshot not generated: {}", name));
            }
        }
    }
    if !mismatches.is_empty() {
        panic!("Snapshot mismatch for Uri .pyi!\n{}", mismatches.join("\n"));
    }
}

/// Generate .py files for Uri and compare against committed snapshots.
/// Guards Phase 3 refactor (common.rs split + Lang trait) from drifting output.
#[test]
fn snapshot_uri_py_class() {
    use dynwinrt_codegen::codegen::python;
    let winmd = WINDOWS_WINMD;
    let classes = match meta::parse_class(winmd, "Windows.Foundation", "Uri") {
        Some(c) => vec![c],
        None => {
            eprintln!("Skipping snapshot test: Windows.winmd not found");
            return;
        }
    };
    let deps = meta::resolve_dependencies(winmd, &classes, &[], &[]);
    let mut all_classes = classes;
    all_classes.extend(deps.classes);
    let all_interfaces = deps.interfaces;
    let all_enums = deps.enums;

    let mut known_types: HashSet<String> = HashSet::new();
    for c in &all_classes { known_types.insert(c.name.clone()); }
    for i in &all_interfaces { known_types.insert(i.name.clone()); }
    for e in &all_enums {
        if let TypeMeta::Enum { name, .. } = e { known_types.insert(name.clone()); }
    }
    let delegate_type_names: HashSet<String> = all_interfaces.iter()
        .filter(|i| i.methods.iter().any(|m| m.name == ".ctor") && i.methods.iter().any(|m| m.name == "Invoke"))
        .map(|i| i.name.clone())
        .collect();
    let shared_iids: HashSet<String> = HashSet::new();

    let mut generated: HashMap<String, String> = HashMap::new();
    for iface in &all_interfaces {
        let code = python::generate_interface(iface, &known_types, &delegate_type_names);
        generated.insert(format!("{}.py", to_snake_case_filename(&iface.name)), code);
    }
    for class in &all_classes {
        let code = python::generate_class(class, &known_types, &delegate_type_names, &shared_iids);
        generated.insert(format!("{}.py", to_snake_case_filename(&class.name)), code);
    }
    let index = python::generate_index(&all_classes, &all_interfaces, &all_enums);
    generated.insert("__init__.py".to_string(), index);

    let snapshot_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/uri_py");
    assert!(snapshot_dir.exists(), "Snapshot directory not found: {}", snapshot_dir.display());

    let mut mismatches = Vec::new();
    for (filename, actual) in &generated {
        let snapshot_path = snapshot_dir.join(filename);
        if !snapshot_path.exists() {
            mismatches.push(format!("  missing snapshot: {}", filename));
            continue;
        }
        let expected = fs::read_to_string(&snapshot_path).unwrap();
        if *actual != expected {
            mismatches.push(format!("  differs: {}", filename));
        }
    }
    if let Ok(entries) = fs::read_dir(&snapshot_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".py") && !generated.contains_key(&name) {
                mismatches.push(format!("  extra snapshot not generated: {}", name));
            }
        }
    }
    if !mismatches.is_empty() {
        panic!(
            "Snapshot mismatch for Uri .py!\n{}\n\nTo update: cargo run -p dynwinrt-codegen -- generate --namespace Windows.Foundation --class-name Uri --lang py --output tools/dynwinrt-codegen/tests/snapshots/uri_py",
            mismatches.join("\n")
        );
    }
}
