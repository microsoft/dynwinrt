// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Regression test: the JS code generator must only write public artifacts
//! (`.js` + `.d.ts`) into the output directory. No hidden cache files
//! (e.g. `.index.ts`), no temp files, no raw `.ts` sources.

use std::fs;
use std::path::Path;
use std::process::Command;

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

#[test]
fn output_dir_contains_no_internal_cache_files() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }

    let exe = env!("CARGO_BIN_EXE_dynwinrt-codegen");
    let tmp = std::env::temp_dir().join(format!(
        "dynwinrt-codegen-cleanliness-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);

    let status = Command::new(exe)
        .args([
            "generate",
            "--namespace",
            "Windows.Foundation",
            "--class-name",
            "Uri",
            "--lang",
            "js",
            "--output",
        ])
        .arg(&tmp)
        .status()
        .expect("spawn dynwinrt-codegen");
    assert!(status.success(), "codegen exited non-zero: {:?}", status);

    // First-pass: also exercise the incremental round-trip path by appending
    // a second class. This used to write a `.index.ts` cache file.
    let status2 = Command::new(exe)
        .args([
            "generate",
            "--namespace",
            "Windows.Foundation",
            "--class-name",
            "WwwFormUrlDecoder",
            "--lang",
            "js",
            "--output",
        ])
        .arg(&tmp)
        .status()
        .expect("spawn dynwinrt-codegen (incremental)");
    assert!(status2.success(), "incremental codegen exited non-zero: {:?}", status2);

    let mut violations: Vec<String> = Vec::new();
    for entry in fs::read_dir(&tmp).expect("read tmp dir") {
        let e = entry.expect("dir entry");
        let name = e.file_name().to_string_lossy().to_string();
        // No dotfiles in the public output dir.
        if name.starts_with('.') {
            violations.push(format!("hidden file leaked: {}", name));
        }
        // No raw .ts sources — only .d.ts ambient declarations are allowed.
        if name.ends_with(".ts") && !name.ends_with(".d.ts") {
            violations.push(format!("raw .ts source leaked: {}", name));
        }
    }

    let _ = fs::remove_dir_all(&tmp);

    assert!(
        violations.is_empty(),
        "output dir contains internal artifacts: {:?}",
        violations
    );
}

/// Assert the full-namespace generation path (no --class-name) also produces
/// a clean output directory with no hidden cache files.
#[test]
fn output_dir_clean_full_namespace_mode() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }

    let exe = env!("CARGO_BIN_EXE_dynwinrt-codegen");
    let tmp = std::env::temp_dir().join(format!(
        "dynwinrt-codegen-cleanliness-ns-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);

    // Plant a fake .index.ts to simulate a stale cache from an older version.
    fs::create_dir_all(&tmp).expect("create tmp dir");
    fs::write(tmp.join(".index.ts"), "// stale").expect("write stale file");

    let status = Command::new(exe)
        .args([
            "generate",
            "--namespace",
            "Windows.Foundation",
            "--lang",
            "js",
            "--output",
        ])
        .arg(&tmp)
        .status()
        .expect("spawn dynwinrt-codegen (full namespace)");
    assert!(status.success(), "full-namespace codegen exited non-zero: {:?}", status);

    let mut violations: Vec<String> = Vec::new();
    for entry in fs::read_dir(&tmp).expect("read tmp dir") {
        let e = entry.expect("dir entry");
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            violations.push(format!("hidden file leaked: {}", name));
        }
        if name.ends_with(".ts") && !name.ends_with(".d.ts") {
            violations.push(format!("raw .ts source leaked: {}", name));
        }
    }

    let _ = fs::remove_dir_all(&tmp);

    assert!(
        violations.is_empty(),
        "full-namespace output dir contains internal artifacts: {:?}",
        violations
    );
}
