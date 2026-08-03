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
    assert!(
        status2.success(),
        "incremental codegen exited non-zero: {:?}",
        status2
    );
    assert!(tmp.join("lifetime.js").exists());
    assert!(tmp.join("lifetime.d.ts").exists());
    let index = fs::read_to_string(tmp.join("index.d.ts")).expect("read index.d.ts");
    assert!(index.contains("createProjectedLifetimeScope"));
    assert!(index.contains("projectAs"));
    assert!(index.contains("releaseProjected"));

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
    assert!(
        status.success(),
        "full-namespace codegen exited non-zero: {:?}",
        status
    );
    assert!(tmp.join("lifetime.js").exists());
    assert!(tmp.join("lifetime.d.ts").exists());
    let index = fs::read_to_string(tmp.join("index.d.ts")).expect("read index.d.ts");
    assert!(index.contains("createProjectedLifetimeScope"));
    assert!(index.contains("projectAs"));
    assert!(index.contains("releaseProjected"));
    assert!(!index.contains("trackProjectedValue"));

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

#[test]
fn python_emits_stubs_by_default_and_supports_opt_out() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }

    let exe = env!("CARGO_BIN_EXE_dynwinrt-codegen");
    let default_dir = std::env::temp_dir().join(format!(
        "dynwinrt-codegen-python-stubs-default-{}",
        std::process::id()
    ));
    let opt_out_dir = std::env::temp_dir().join(format!(
        "dynwinrt-codegen-python-stubs-opt-out-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&default_dir);
    let _ = fs::remove_dir_all(&opt_out_dir);

    let default_status = Command::new(exe)
        .args([
            "generate",
            "--namespace",
            "Windows.Foundation",
            "--class-name",
            "Uri",
            "--lang",
            "py",
            "--output",
        ])
        .arg(&default_dir)
        .status()
        .expect("spawn dynwinrt-codegen (Python defaults)");
    assert!(default_status.success());
    let default_namespace = default_dir.join("windows").join("foundation");
    assert!(default_namespace.join("uri.py").exists());
    assert!(default_namespace.join("uri.pyi").exists());
    assert!(default_dir.join("windows__foundation__uri.py").exists());
    assert!(default_dir.join("windows__foundation__uri.pyi").exists());
    assert!(default_dir.join("__init__.pyi").exists());
    assert!(default_dir.join("py.typed").exists());
    assert!(default_dir.join("pyproject.toml").exists());

    let opt_out_status = Command::new(exe)
        .args([
            "generate",
            "--namespace",
            "Windows.Foundation",
            "--class-name",
            "Uri",
            "--lang",
            "py",
            "--no-pyi",
            "--output",
        ])
        .arg(&opt_out_dir)
        .status()
        .expect("spawn dynwinrt-codegen (Python stub opt-out)");
    assert!(opt_out_status.success());
    let opt_out_namespace = opt_out_dir.join("windows").join("foundation");
    assert!(opt_out_namespace.join("uri.py").exists());
    assert!(!opt_out_namespace.join("uri.pyi").exists());
    assert!(opt_out_dir.join("windows__foundation__uri.py").exists());
    assert!(!opt_out_dir.join("windows__foundation__uri.pyi").exists());
    assert!(!opt_out_dir.join("__init__.pyi").exists());
    assert!(!opt_out_dir.join("py.typed").exists());

    let _ = fs::remove_dir_all(&default_dir);
    let _ = fs::remove_dir_all(&opt_out_dir);
}
