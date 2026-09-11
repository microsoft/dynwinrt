// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const REGISTRY: &str = "Windows.Win32.System.Registry";
const SYSTEM: &str = "Windows.Win32.System.SystemInformation";
const SDK: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "dynwinrt-win32-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn output(&self) -> PathBuf {
        self.0.join("generated")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove this test's private output");
    }
}

fn metadata() -> Option<String> {
    let path = std::env::var("DYNWINRT_WIN32_WINMD")
        .ok()
        .filter(|path| Path::new(path).is_file());
    if path.is_none() {
        assert_ne!(
            std::env::var("DYNWINRT_REQUIRE_WIN32_METADATA").as_deref(),
            Ok("1"),
            "Win32 CLI tests require DYNWINRT_WIN32_WINMD"
        );
        eprintln!("Skipping Win32 CLI test: DYNWINRT_WIN32_WINMD is not configured");
    }
    path
}

fn generate(winmd: &str, output: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args(["generate", "--winmd", winmd, "--output"])
        .arg(output)
        .args(args)
        .output()
        .expect("execute generator")
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, current: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(current).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, result);
            } else {
                result.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

#[test]
fn win32_namespace_and_class_modes_are_identical_and_incremental() {
    let Some(winmd) = metadata() else { return };
    let first = Scratch::new();
    let second = Scratch::new();
    let output = first.output();
    success(generate(&winmd, &output, &["--namespace", REGISTRY]));
    success(generate(
        &winmd,
        &second.output(),
        &["--class-name", "Windows.Win32.System.Registry.Apis"],
    ));
    assert_eq!(files(&output), files(&second.output()));

    let registry = output.join(r"win32\windows\win32\system\registry\Apis.js");
    let original = fs::read(&registry).unwrap();
    success(generate(&winmd, &output, &["--namespace", SYSTEM]));
    assert_eq!(fs::read(&registry).unwrap(), original);
    assert!(
        output
            .join(r"win32\windows\win32\system\system-information\Apis.js")
            .is_file()
    );
    assert!(!output.join("Apis.js").exists());
    assert!(!output.join("index.js").exists());
    let package: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("package.json")).unwrap()).unwrap();
    assert!(package["exports"].get(".").is_none());
    assert_eq!(
        package["exports"]["./win32/windows/win32/system/registry"]["require"],
        "./win32/windows/win32/system/registry/index.js"
    );
    assert!(package["exports"].get("./win32").is_some());
    assert!(
        String::from_utf8(original)
            .unwrap()
            .contains("@microsoft/dynwinrt/win32")
    );
}

#[test]
fn win32_dry_run_and_python_rejection_do_not_publish_output() {
    let Some(winmd) = metadata() else { return };
    let scratch = Scratch::new();
    let output = scratch.output();
    let dry_run = generate(&winmd, &output, &["--namespace", REGISTRY, "--dry-run"]);
    assert!(String::from_utf8_lossy(&dry_run.stdout).contains("Would generate flat Win32"));
    success(dry_run);
    assert!(!output.exists());

    fs::create_dir(&output).unwrap();
    fs::write(output.join("sentinel.txt"), b"keep").unwrap();
    for args in [
        vec!["--namespace", REGISTRY, "--lang", "py"],
        vec![
            "--class-name",
            "Windows.Win32.System.Registry.Apis",
            "--lang",
            "py",
        ],
    ] {
        let result = generate(&winmd, &output, &args);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("not supported for flat Win32"));
        assert_eq!(fs::read(output.join("sentinel.txt")).unwrap(), b"keep");
        assert!(!output.join("win32").exists());
    }
}

#[test]
fn win32_unsupported_export_container_fails_without_partial_output() {
    let Some(winmd) = metadata() else { return };
    let scratch = Scratch::new();
    let output = scratch.output();
    let result = generate(
        &winmd,
        &output,
        &[
            "--class-name",
            "Windows.Win32.System.Registry.Apis,Windows.Win32.System.Mapi.Apis",
        ],
    );
    assert!(!result.status.success());
    assert!(!output.exists());
}

#[test]
fn win32_manifest_and_generated_file_drift_fail_closed() {
    let Some(winmd) = metadata() else { return };
    let scratch = Scratch::new();
    let output = scratch.output();
    success(generate(&winmd, &output, &["--namespace", REGISTRY]));
    let path = output.join(r"win32\windows\win32\system\registry\Apis.js");
    fs::write(&path, "// edited").unwrap();
    let before = files(&output);
    let result = generate(&winmd, &output, &["--namespace", SYSTEM]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("does not match its manifest"));
    assert_eq!(files(&output), before);
}

#[test]
fn win32_enum_artifacts_are_exported_hashed_and_cleaned_incrementally() {
    let Some(winmd) = metadata() else { return };
    let scratch = Scratch::new();
    let output = scratch.output();
    success(generate(&winmd, &output, &["--namespace", REGISTRY]));
    let manifest_path = output.join(r"win32\.dynwinrt-win32-manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let directory = output.join(r"win32\windows\win32\system\registry");
    let extras = manifest["namespaces"][REGISTRY]["extra_files"]
        .as_object_mut()
        .unwrap();
    assert!(
        !extras.is_empty(),
        "full Registry declarations require enum modules"
    );
    let index = fs::read_to_string(directory.join("index.mjs")).unwrap();
    for name in extras.keys() {
        assert!(directory.join(name).is_file(), "{name}");
        if name.ends_with(".js") {
            assert!(index.contains(&format!("from './{name}'")), "{name}");
        }
    }
    for (name, text, exports) in [
        (
            "StaleEnum.js",
            "// Generated by dynwinrt-codegen\nexports.StaleEnum = {};\n",
            vec!["StaleEnum"],
        ),
        (
            "StaleEnum.d.ts",
            "// Generated by dynwinrt-codegen\nexport declare const StaleEnum: {};\n",
            vec![],
        ),
    ] {
        fs::write(directory.join(name), text).unwrap();
        extras.insert(
            name.into(),
            serde_json::json!({
                "sha256": format!("{:x}", Sha256::digest(text.as_bytes())),
                "exports": exports,
            }),
        );
    }
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    success(generate(&winmd, &output, &["--namespace", REGISTRY]));
    assert!(!directory.join("StaleEnum.js").exists());
    assert!(!directory.join("StaleEnum.d.ts").exists());

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let enum_file = manifest["namespaces"][REGISTRY]["extra_files"]
        .as_object()
        .unwrap()
        .keys()
        .find(|name| name.ends_with(".js"))
        .unwrap();
    fs::write(directory.join(enum_file), "// edited enum").unwrap();
    let before = files(&output);
    let result = generate(&winmd, &output, &["--namespace", SYSTEM]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("does not match its manifest"));
    assert_eq!(files(&output), before);
}

#[test]
fn win32_mixed_generation_preserves_winrt_modules_and_root_exports() {
    let Some(winmd) = metadata() else { return };
    assert!(
        Path::new(SDK).is_file(),
        "mixed WinRT test requires the Windows SDK"
    );
    let scratch = Scratch::new();
    let output = scratch.output();
    success(generate(
        SDK,
        &output,
        &["--namespace", "Windows.Foundation", "--class-name", "Uri"],
    ));
    let baseline = files(&output);
    let package: serde_json::Value =
        serde_json::from_slice(&baseline[Path::new("package.json")]).unwrap();
    success(generate(&winmd, &output, &["--namespace", REGISTRY]));
    for (path, bytes) in &baseline {
        if path != Path::new("package.json") {
            assert_eq!(
                &fs::read(output.join(path)).unwrap(),
                bytes,
                "{}",
                path.display()
            );
        }
    }
    let mixed: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("package.json")).unwrap()).unwrap();
    for (key, value) in package["exports"].as_object().unwrap() {
        assert_eq!(&mixed["exports"][key], value);
    }

    success(generate(
        SDK,
        &output,
        &["--namespace", "Windows.Foundation", "--class-name", "Uri"],
    ));
    assert!(
        output
            .join(r"win32\windows\win32\system\registry\Apis.js")
            .exists()
    );
    let package: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("package.json")).unwrap()).unwrap();
    assert!(package["exports"].get("./win32").is_some());

    let reverse = Scratch::new();
    success(generate(
        &winmd,
        &reverse.output(),
        &["--namespace", REGISTRY],
    ));
    success(generate(
        SDK,
        &reverse.output(),
        &["--namespace", "Windows.Foundation", "--class-name", "Uri"],
    ));
    assert_eq!(files(&output), files(&reverse.output()));
}
