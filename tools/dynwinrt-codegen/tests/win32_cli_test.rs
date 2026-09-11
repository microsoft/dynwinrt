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
    command(winmd, output, args)
        .output()
        .expect("execute generator")
}

fn command(winmd: &str, output: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"));
    command
        .args(["generate", "--winmd", winmd, "--output"])
        .arg(output)
        .args(args);
    command
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

#[test]
fn win32_dry_run_does_not_mutate_an_existing_mixed_package() {
    let Some(winmd) = metadata() else { return };
    let scratch = Scratch::new();
    let output = scratch.output();
    success(generate(&winmd, &output, &["--namespace", REGISTRY]));
    fs::write(output.join("keep.txt"), "handwritten user note").unwrap();
    let before = files(&output);
    success(generate(
        &winmd,
        &output,
        &["--namespace", SYSTEM, "--dry-run"],
    ));
    assert_eq!(files(&output), before);
    let result = generate(
        &winmd,
        &output,
        &[
            "--class-name",
            "Windows.Win32.System.Mapi.Apis",
            "--dry-run",
        ],
    );
    assert!(!result.status.success());
    assert_eq!(files(&output), before);
}

#[test]
fn win32_failed_publish_rolls_back_and_a_retry_commits_all_files() {
    let Some(winmd) = metadata() else { return };
    for point in ["before_publish", "after_backup_rename"] {
        let scratch = Scratch::new();
        let output = scratch.output();
        success(generate(&winmd, &output, &["--namespace", REGISTRY]));
        fs::write(output.join("keep.txt"), b"preserve existing data").unwrap();
        let before = files(&output);
        let failed = command(&winmd, &output, &["--namespace", SYSTEM])
            .env("DYNWINRT_CODEGEN_TEST_FAIL_OUTPUT_COMMIT", point)
            .output()
            .unwrap();
        assert!(!failed.status.success(), "{point}");
        assert!(
            String::from_utf8_lossy(&failed.stderr).contains("Injected output transaction failure"),
            "{}",
            String::from_utf8_lossy(&failed.stderr)
        );
        assert_eq!(files(&output), before, "{point}");
        success(generate(&winmd, &output, &["--namespace", SYSTEM]));
        assert!(
            output
                .join(r"win32\windows\win32\system\system-information\Apis.js")
                .is_file()
        );
        assert_eq!(
            fs::read(output.join("keep.txt")).unwrap(),
            b"preserve existing data"
        );
        assert_eq!(
            fs::read(output.join(r"win32\windows\win32\system\registry\Apis.js")).unwrap(),
            before[Path::new(r"win32\windows\win32\system\registry\Apis.js")]
        );
    }
}

#[test]
fn win32_missing_modules_and_malformed_manifests_fail_without_publication() {
    let Some(winmd) = metadata() else { return };
    let scratch = Scratch::new();
    let output = scratch.output();
    success(generate(&winmd, &output, &["--namespace", REGISTRY]));
    let module = output.join(r"win32\windows\win32\system\registry\Apis.d.ts");
    let bytes = fs::read(&module).unwrap();
    fs::remove_file(&module).unwrap();
    let before = files(&output);
    let missing = generate(&winmd, &output, &["--namespace", SYSTEM]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("missing"));
    assert_eq!(files(&output), before);
    fs::write(module, bytes).unwrap();

    let manifest = output.join(r"win32\.dynwinrt-win32-manifest.json");
    let original: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    for field in ["version", "unexpected"] {
        let mut malformed = original.clone();
        malformed[field] = 123.into();
        fs::write(&manifest, serde_json::to_vec(&malformed).unwrap()).unwrap();
        let before = files(&output);
        let result = generate(&winmd, &output, &["--namespace", SYSTEM]);
        assert!(!result.status.success());
        assert_eq!(files(&output), before);
    }
}

#[test]
fn win32_output_rejects_unmanaged_files_and_traversal_manifest_paths() {
    let Some(winmd) = metadata() else { return };
    let scratch = Scratch::new();
    let output = scratch.output();
    fs::create_dir_all(output.join("win32")).unwrap();
    let sentinel = output.join(r"win32\custom.js");
    fs::write(&sentinel, "module.exports = 'mine'").unwrap();
    let before = files(&output);
    let failed = generate(&winmd, &output, &["--namespace", REGISTRY]);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("no generation manifest"));
    assert_eq!(files(&output), before);
    fs::remove_file(sentinel).unwrap();
    success(generate(&winmd, &output, &["--namespace", REGISTRY]));
    let manifest = output.join(r"win32\.dynwinrt-win32-manifest.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["namespaces"][REGISTRY]["extra_files"]["../outside.js"] = serde_json::json!({
        "sha256": "0".repeat(64), "exports": ["outside"]
    });
    fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    fs::write(scratch.0.join("outside.js"), b"outside unchanged").unwrap();
    let before = files(&output);
    let failed = generate(&winmd, &output, &["--namespace", SYSTEM]);
    assert!(!failed.status.success());
    assert_eq!(files(&output), before);
    assert_eq!(
        fs::read(scratch.0.join("outside.js")).unwrap(),
        b"outside unchanged"
    );
}

#[test]
fn win32_com_and_winrt_share_one_atomic_batch_without_export_collisions() {
    let Some(winmd) = metadata() else { return };
    let scratch = Scratch::new();
    let output = scratch.output();
    let combined_metadata = format!("{winmd};{SDK}");
    success(generate(
        &combined_metadata,
        &output,
        &[
            "--class-name",
            "Windows.Foundation.Uri,Windows.Win32.System.Com.IPersistFile",
        ],
    ));
    let baseline = files(&output);
    success(generate(&winmd, &output, &["--namespace", REGISTRY]));
    for (path, content) in baseline {
        if path != Path::new("package.json") {
            assert_eq!(
                fs::read(output.join(&path)).unwrap(),
                content,
                "{}",
                path.display()
            );
        }
    }
    let package: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("package.json")).unwrap()).unwrap();
    for entry in [".", "./com", "./win32"] {
        assert!(package["exports"].get(entry).is_some(), "{entry}");
    }
    let root = fs::read_to_string(output.join("index.d.ts")).unwrap();
    assert!(!root.contains("regOpenKeyEx"));
    assert!(!root.contains("IPersistFile"));

    let batch = Scratch::new();
    success(generate(
        &combined_metadata,
        &batch.output(),
        &[
            "--class-name",
            "Windows.Win32.System.Registry.Apis,Windows.Foundation.Uri,Windows.Win32.System.Com.IPersistFile",
        ],
    ));
    assert_eq!(files(&output), files(&batch.output()));
    let before = files(&output);
    let failed = generate(
        &combined_metadata,
        &output,
        &[
            "--class-name",
            "Windows.Win32.System.SystemInformation.Apis,Windows.Foundation.ClassThatDoesNotExist",
        ],
    );
    assert!(!failed.status.success());
    assert_eq!(files(&output), before);
}

#[test]
fn win32_relative_runtime_imports_load_cjs_esm_and_enum_subpaths() {
    let Some(winmd) = metadata() else { return };
    for import in ["./runtime helpers/win32.js", r".\runtime helpers\win32.js"] {
        let scratch = Scratch::new();
        let output = scratch.0.join("generated bindings");
        success(generate(
            &winmd,
            &output,
            &[
                "--class-name",
                "Windows.Win32.System.Registry.Apis,Windows.Win32.System.SystemInformation.Apis",
                "--import-name",
                import,
            ],
        ));
        let runtime = output.join("runtime helpers");
        fs::create_dir(&runtime).unwrap();
        let stub = r#"
globalThis.win32RuntimeLoads = (globalThis.win32RuntimeLoads ?? 0) + 1;
module.exports = {
  DynWin32: {},
  DynWin32Function: { bind() { throw new Error('import must not dispatch a native function'); } },
};
"#;
        fs::write(runtime.join("win32.js"), stub).unwrap();
        fs::write(runtime.join("win32-unsafe.js"), stub).unwrap();
        let script = r#"
const assert = require('node:assert/strict');
const { createRequire } = require('node:module');
const { join } = require('node:path');
const { pathToFileURL } = require('node:url');
const fs = require('node:fs');
(async () => {
  const root = process.argv[1];
  const req = createRequire(join(root, 'package.json'));
  assert.throws(() => req('@winapp/bindings'), { code: 'ERR_PACKAGE_PATH_NOT_EXPORTED' });
  assert.equal(globalThis.win32RuntimeLoads, undefined);
  const ns = '@winapp/bindings/win32/windows/win32/system/registry';
  const registry = req(ns);
  const module = req(`${ns}/Apis`);
  assert.equal(registry.regOpenKeyEx, module.regOpenKeyExW);
  assert.equal(registry.regQueryValueEx, module.regQueryValueExW);
  assert.equal(typeof registry.regCloseKey, 'function');
  const esm = await import(pathToFileURL(join(root, 'win32/windows/win32/system/registry/index.mjs')).href);
  assert.equal(esm.regOpenKeyExW, registry.regOpenKeyExW);
  const manifest = JSON.parse(fs.readFileSync(join(root, 'win32/.dynwinrt-win32-manifest.json')));
  const files = manifest.namespaces['Windows.Win32.System.Registry'].extra_files;
  let enums = 0;
  for (const [file, entry] of Object.entries(files)) {
    if (!file.endsWith('.js')) continue;
    const values = req(`${ns}/${file.slice(0, -3)}`);
    for (const name of entry.exports) {
      assert.equal(registry[name], values[name]);
      assert.equal(esm[name], values[name]);
      assert(Object.isFrozen(values[name]));
      ++enums;
    }
  }
  assert(enums > 0);
  const barrel = req('@winapp/bindings/win32');
  assert.equal(barrel.Windows_Win32_System_Registry, registry);
  assert.equal(typeof barrel.Windows_Win32_System_SystemInformation.getTickCount64, 'function');
  assert.equal(globalThis.win32RuntimeLoads, 2);
  console.log('module-contracts-ok');
})().catch(error => { console.error(error); process.exitCode = 1; });
"#;
        let node = Command::new("node")
            .args(["--eval", script])
            .arg(&output)
            .output()
            .expect("Node is required for generated module tests");
        success(node);
    }
}
