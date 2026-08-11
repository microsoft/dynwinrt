// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use dynwinrt_codegen::codegen::winrt::javascript::bundle::{
    BindingBundleSpec, binding_bundle_redirect, generate_binding_bundle,
};

fn test_directory(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("{name}-{}", std::process::id()))
}

fn run_bundle(directory: &Path, spec: &str) -> Output {
    run_bundles(directory, &[spec])
}

fn run_bundles(directory: &Path, specs: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"));
    command.arg("bundle").arg("--output").arg(directory);
    for spec in specs {
        command.arg("--bundle").arg(spec);
    }
    command.output().expect("run dynwinrt-codegen bundle")
}

#[test]
fn bundle_preserves_cycles_externals_named_exports_and_dts_paths() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("Skipping binding bundle runtime test: node is unavailable");
        return;
    }

    let directory = test_directory("binding-bundle-runtime");
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("A.js"),
        "\
exports.AName = 'A';\n\
exports.A = class A {};\n\
exports.B = class WrongB {};\n\
const b = require('./B.js');\n\
const lifetime = require('./lifetime.js');\n\
const path = require('node:path');\n\
exports.APeer = () => b.BName;\n\
exports.Track = lifetime.track;\n\
exports.Separator = path.sep;\n",
    )
    .unwrap();
    fs::write(
        directory.join("lifetime.js"),
        "\
const tracked = new Set();\n\
exports.track = (value) => { tracked.add(value); return value; };\n\
exports.count = () => tracked.size;\n",
    )
    .unwrap();
    fs::write(
        directory.join("A.d.ts"),
        "\
export declare const AName: string;\n\
export declare class A {}\n\
export interface Point { x: number; y: number; }\n\
export type Rect = { x: number; y: number; width: number; height: number };\n\
export declare const APeer: () => string;\n\
export declare const Track: (value: object) => object;\n\
export declare const Separator: string;\n",
    )
    .unwrap();
    fs::write(
        directory.join("B.js"),
        "\
exports.BName = 'B';\n\
exports.B = class B {};\n\
const a = require('./A.js');\n\
exports.BPeer = () => a.AName;\n",
    )
    .unwrap();
    fs::write(
        directory.join("B.d.ts"),
        "\
export declare const BName: string;\n\
export declare const BPeer: () => string;\n",
    )
    .unwrap();

    let generated = generate_binding_bundle(
        &directory,
        &BindingBundleSpec {
            name: "first-screen".into(),
            modules: vec!["A".into(), "B".into()],
        },
    )
    .unwrap();
    assert_eq!(generated.module_count, 3);
    assert!(generated.js.contains("require('node:path')"));
    assert!(
        generated
            .js
            .contains("__defineBundleExport('AName', () => __load('./A.js').AName);")
    );
    assert!(
        generated
            .js
            .contains("__defineBundleExport('BName', () => __load('./B.js').BName);")
    );
    assert!(generated.exports.contains("track"));
    assert!(generated.dts.contains("from './A.js';"));
    assert!(generated.dts.contains("from './B.js';"));
    assert!(
        generated
            .dts
            .contains("export type { Point, Rect } from './A.js';")
    );

    fs::write(directory.join("first-screen.js"), generated.js).unwrap();
    fs::write(directory.join("first-screen.d.ts"), generated.dts).unwrap();
    assert!(
        fs::read_to_string(directory.join("A.d.ts"))
            .unwrap()
            .contains("export interface Point")
    );
    for module in &generated.modules {
        fs::write(
            directory.join(format!("{module}.js")),
            binding_bundle_redirect("first-screen", module),
        )
        .unwrap();
    }
    fs::write(
        directory.join("test.js"),
        "\
const assert = require('node:assert/strict');\n\
const bundle = require('./first-screen.js');\n\
const deepA = require('./A.js');\n\
const deepB = require('./B.js');\n\
const deepLifetime = require('./lifetime.js');\n\
assert.equal(bundle.AName, 'A');\n\
assert.equal(bundle.BName, 'B');\n\
assert.equal(bundle.APeer(), 'B');\n\
assert.equal(bundle.BPeer(), 'A');\n\
assert.equal(typeof bundle.Separator, 'string');\n\
assert.strictEqual(bundle.A, deepA.A);\n\
assert.strictEqual(bundle.B, deepB.B);\n\
assert.strictEqual(bundle.Track, deepLifetime.track);\n\
bundle.Track({});\n\
assert.equal(deepLifetime.count(), 1);\n",
    )
    .unwrap();

    let output = Command::new("node")
        .arg("test.js")
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "node failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    fs::write(
        directory.join("test.mjs"),
        "\
import assert from 'node:assert/strict';\n\
import { A, AName } from './first-screen.js';\n\
import { A as DeepA, AName as DeepAName } from './A.js';\n\
assert.strictEqual(A, DeepA);\n\
assert.equal(AName, DeepAName);\n",
    )
    .unwrap();
    let output = Command::new("node")
        .arg("test.mjs")
        .current_dir(&directory)
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&directory);
    assert!(
        output.status.success(),
        "node ESM failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn bundle_rejects_case_insensitive_names_and_fixed_entrypoints() {
    let directory = test_directory("binding-bundle-portable-names");
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("A.js"), "exports.A = 'A';\n").unwrap();

    let duplicate = run_bundles(&directory, &["First=A", "first=A"]);
    let stderr = String::from_utf8_lossy(&duplicate.stderr);
    assert!(!duplicate.status.success());
    assert!(stderr.contains("case-insensitively"));
    assert!(!directory.join("First.js").exists());
    assert!(!directory.join("first.js").exists());

    let module_collision = run_bundle(&directory, "a=A");
    let stderr = String::from_utf8_lossy(&module_collision.stderr);
    assert!(!module_collision.status.success());
    assert!(stderr.contains("collides with an existing generated module `A`"));
    assert!(!directory.join("a.d.ts").exists());

    let module_casing = run_bundle(&directory, "first=a");
    let stderr = String::from_utf8_lossy(&module_casing.stderr);
    assert!(!module_casing.status.success());
    assert!(stderr.contains("module `a` with non-portable casing"));
    assert!(!directory.join("first.js").exists());

    let reserved = run_bundle(&directory, "PrOxY=A");
    let stderr = String::from_utf8_lossy(&reserved.stderr);
    assert!(!reserved.status.success());
    assert!(stderr.contains("Reserved bundle name"));
    assert!(!directory.join("PrOxY.js").exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn bundle_rerun_rejects_stale_existing_artifacts() {
    let directory = test_directory("binding-bundle-stale-rerun");
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("A.js"), "exports.A = 'old';\n").unwrap();
    fs::write(
        directory.join("A.d.ts"),
        "export declare const A: string;\n",
    )
    .unwrap();

    let first = run_bundle(&directory, "first-screen=A");
    assert!(
        first.status.success(),
        "initial bundle failed:\n{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let original_bundle = fs::read_to_string(directory.join("first-screen.js")).unwrap();
    fs::write(directory.join("A.js"), "exports.A = 'new';\n").unwrap();

    let rerun = run_bundle(&directory, "first-screen=A");
    let stderr = String::from_utf8_lossy(&rerun.stderr);
    assert!(!rerun.status.success(), "rebundling unexpectedly succeeded");
    assert!(stderr.contains("already contains bundle artifacts or redirect shims"));
    assert!(stderr.contains("new or cleaned unbundled output directory"));
    assert_eq!(
        fs::read_to_string(directory.join("A.js")).unwrap(),
        "exports.A = 'new';\n"
    );
    assert_eq!(
        fs::read_to_string(directory.join("first-screen.js")).unwrap(),
        original_bundle
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn bundle_rejects_malicious_inventory_paths_without_writing_outside_output() {
    let directory = test_directory("binding-bundle-malicious-inventory");
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("A.js"), "exports.A = 'A';\n").unwrap();
    let escaped_name = format!("binding-bundle-escaped-{}", std::process::id());
    let escaped = directory
        .parent()
        .unwrap()
        .join(format!("{escaped_name}.js"));
    let _ = fs::remove_file(&escaped);
    fs::write(directory.join(".dynwinrt-binding-bundles"), "first-screen").unwrap();
    fs::write(
        directory.join(".dynwinrt-binding-bundle-first-screen"),
        format!("../{escaped_name}"),
    )
    .unwrap();

    let output = run_bundle(&directory, "first-screen=A");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "malicious inventory was accepted");
    assert!(stderr.contains("Invalid bundle inventory module"));
    assert!(!escaped.exists());
    assert!(!directory.join("lifetime.js").exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn bundle_fails_atomically_when_a_configured_root_is_missing() {
    let directory = test_directory("binding-bundle-missing-root");
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("A.js"), "exports.A = 'A';\n").unwrap();

    let output = run_bundle(&directory, "first-screen=A,Missing");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "bundle with a missing root succeeded"
    );
    assert!(stderr.contains("configured root module(s) are missing: Missing"));
    assert!(!directory.join("first-screen.js").exists());
    assert!(!directory.join("lifetime.js").exists());
    assert_eq!(
        fs::read_to_string(directory.join("A.js")).unwrap(),
        "exports.A = 'A';\n"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn bundle_fails_atomically_when_a_relative_dependency_is_missing() {
    let directory = test_directory("binding-bundle-missing-dependency");
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("A.js"),
        "const b = require('./B.js');\nexports.A = () => b.B;\n",
    )
    .unwrap();

    let output = run_bundle(&directory, "first-screen=A");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "bundle with an incomplete closure succeeded"
    );
    assert!(stderr.contains("dependency closure is incomplete"));
    assert!(stderr.contains("requires missing generated sibling `B`"));
    assert!(!directory.join("first-screen.js").exists());
    assert!(!directory.join("lifetime.js").exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn multiple_bundles_share_common_dependencies_without_duplicate_identity() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("Skipping multi-bundle identity test: node is unavailable");
        return;
    }

    let directory = test_directory("binding-bundle-shared-dependency");
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("A.js"),
        "const lifetime = require('./lifetime.js');\n\
         exports.A = class A {};\n\
         exports.TrackA = lifetime.projectAs;\n",
    )
    .unwrap();
    fs::write(
        directory.join("B.js"),
        "const lifetime = require('./lifetime.js');\n\
         exports.B = class B {};\n\
         exports.TrackB = lifetime.projectAs;\n",
    )
    .unwrap();

    let output = run_bundles(&directory, &["first=A", "second=B"]);
    assert!(
        output.status.success(),
        "multi-bundle generation failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fs::read_to_string(directory.join("lifetime.js"))
            .unwrap()
            .contains("exports.projectAs = projectAs;")
    );
    assert!(
        !fs::read_to_string(directory.join(".dynwinrt-binding-bundle-first"))
            .unwrap()
            .contains("lifetime")
    );
    assert!(
        !fs::read_to_string(directory.join(".dynwinrt-binding-bundle-second"))
            .unwrap()
            .contains("lifetime")
    );
    fs::write(
        directory.join("multi-test.js"),
        "\
const assert = require('node:assert/strict');\n\
const first = require('./first.js');\n\
const second = require('./second.js');\n\
const lifetime = require('./lifetime.js');\n\
assert.strictEqual(first.TrackA, lifetime.projectAs);\n\
assert.strictEqual(second.TrackB, lifetime.projectAs);\n\
assert.strictEqual(require('./A.js').A, first.A);\n\
assert.strictEqual(require('./B.js').B, second.B);\n",
    )
    .unwrap();
    let node = Command::new("node")
        .arg("multi-test.js")
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(
        node.status.success(),
        "multi-bundle runtime failed:\n{}",
        String::from_utf8_lossy(&node.stderr)
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn multiple_bundles_use_the_ordinary_barrel_canonical_export_owner() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("Skipping canonical bundle owner test: node is unavailable");
        return;
    }

    let directory = test_directory("binding-bundle-canonical-owner");
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("IPropertyValue.js"),
        "\
exports.IPropertyValue = class IPropertyValue {};\n\
exports.Point = { owner: 'IPropertyValue' };\n",
    )
    .unwrap();
    fs::write(
        directory.join("PropertyValue.js"),
        "\
exports.PropertyValue = class PropertyValue {};\n\
exports.Point = { owner: 'PropertyValue' };\n",
    )
    .unwrap();

    let output = run_bundles(
        &directory,
        &["property=PropertyValue", "interface=IPropertyValue"],
    );
    assert!(
        output.status.success(),
        "multi-bundle generation failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let root = fs::read_to_string(directory.join("index.js")).unwrap();
    assert!(root.contains("__exportLazy('Point', './interface.js');"));
    assert!(!root.contains("__exportLazy('Point', './property.js');"));

    fs::write(
        directory.join("canonical-owner-test.js"),
        "\
const assert = require('node:assert/strict');\n\
const root = require('./index.js');\n\
const interfaceBundle = require('./interface.js');\n\
const propertyBundle = require('./property.js');\n\
const deepInterface = require('./IPropertyValue.js');\n\
const deepProperty = require('./PropertyValue.js');\n\
assert.strictEqual(root.Point, interfaceBundle.Point);\n\
assert.notStrictEqual(root.Point, propertyBundle.Point);\n\
assert.strictEqual(deepInterface.Point, interfaceBundle.Point);\n\
assert.strictEqual(deepProperty.Point, propertyBundle.Point);\n",
    )
    .unwrap();
    let node = Command::new("node")
        .arg("canonical-owner-test.js")
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(
        node.status.success(),
        "canonical owner runtime failed:\n{}",
        String::from_utf8_lossy(&node.stderr)
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn bundle_uses_canonical_dependency_owner_for_duplicate_exports() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("Skipping canonical dependency owner test: node is unavailable");
        return;
    }

    let directory = test_directory("binding-bundle-canonical-dependency-owner");
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("AAux.js"),
        "exports.Helper = { owner: 'AAux' };\n",
    )
    .unwrap();
    fs::write(
        directory.join("B.js"),
        "\
require('./AAux.js');\n\
exports.B = class B {};\n\
exports.Helper = { owner: 'B' };\n",
    )
    .unwrap();

    let output = run_bundle(&directory, "combo=B");
    assert!(
        output.status.success(),
        "bundle generation failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let root = fs::read_to_string(directory.join("index.js")).unwrap();
    let bundle = fs::read_to_string(directory.join("combo.js")).unwrap();
    assert!(root.contains("__exportLazy('Helper', './combo.js');"));
    assert!(bundle.contains("__defineBundleExport('Helper', () => __load('./AAux.js').Helper);"));

    fs::write(
        directory.join("canonical-dependency-test.js"),
        "\
const assert = require('node:assert/strict');\n\
const root = require('./index.js');\n\
const combo = require('./combo.js');\n\
const deepAux = require('./AAux.js');\n\
const deepB = require('./B.js');\n\
assert.strictEqual(root.Helper, combo.Helper);\n\
assert.strictEqual(root.Helper, deepAux.Helper);\n\
assert.notStrictEqual(root.Helper, deepB.Helper);\n",
    )
    .unwrap();
    let node = Command::new("node")
        .arg("canonical-dependency-test.js")
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(
        node.status.success(),
        "canonical dependency runtime failed:\n{}",
        String::from_utf8_lossy(&node.stderr)
    );
    fs::remove_dir_all(directory).unwrap();
}
