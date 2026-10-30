// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};
use windows_metadata::{FieldAttributes, Type, TypeAttributes, Value, writer};

mod common;

static NEXT: AtomicU64 = AtomicU64::new(0);
const DLL: &str = "Microsoft.WindowsAppRuntime.Bootstrap.dll";
const MANIFEST: &str = ".dynwinrt-winappsdk.json";

struct Fixture {
    root: PathBuf,
    metadata: PathBuf,
    output: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "dwb{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let metadata = root.join("BootstrapTest.winmd");
        let mut file = writer::File::new("BootstrapTest");
        let base = file.TypeRef("System", "Enum");
        file.TypeDef(
            "Windows.BootstrapTest",
            "Options",
            writer::TypeDefOrRef::TypeRef(base),
            TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
        );
        file.Field(
            "value__",
            &Type::I32,
            FieldAttributes::Public | FieldAttributes::SpecialName | FieldAttributes::RTSpecialName,
        );
        let member = file.Field(
            "One",
            &Type::named("Windows.BootstrapTest", "Options"),
            FieldAttributes::Public
                | FieldAttributes::Static
                | FieldAttributes::Literal
                | FieldAttributes::HasDefault,
        );
        file.Constant(writer::HasConstant::Field(member), &Value::I32(1));
        let object = file.TypeRef("System", "Object");
        file.TypeDef(
            "Windows.BootstrapTest",
            "Widget",
            writer::TypeDefOrRef::TypeRef(object),
            TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
        );
        file.TypeDef(
            "_dynwinrt_build",
            "Wheel",
            writer::TypeDefOrRef::TypeRef(object),
            TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
        );
        fs::write(&metadata, file.into_stream()).unwrap();
        for (architecture, machine) in [("x64", 0x8664), ("arm64", 0xaa64)] {
            let directory = root.join("inputs").join(architecture);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join(DLL), pe(machine)).unwrap();
        }
        fs::write(
            root.join("inputs").join("license.txt"),
            "SDK test license\n",
        )
        .unwrap();
        let output = root.join("bindings");
        Self {
            root,
            metadata,
            output,
        }
    }

    fn command(&self, language: &str, configured: bool) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"));
        command
            .args(["generate", "--winmd"])
            .arg(&self.metadata)
            .args([
                "--namespace",
                "Windows.BootstrapTest",
                "--lang",
                language,
                "--output",
            ])
            .arg(&self.output)
            .env_remove("DYNWINRT_CODEGEN_TEST_FAIL_OUTPUT_COMMIT");
        if configured {
            command
                .args(["--winappsdk-version", "2.5", "--winappsdk-bootstrap-path"])
                .arg(self.input("x64"))
                .arg("--winappsdk-bootstrap-path")
                .arg(self.input("arm64"));
        }
        command
    }

    fn package(&self, language: &str) -> PathBuf {
        if language == "py" {
            common::python_fixture_package(&self.output)
        } else {
            self.output.clone()
        }
    }

    fn input(&self, architecture: &str) -> PathBuf {
        self.root.join("inputs").join(architecture).join(DLL)
    }

    fn command_with_paths(&self, language: &str, paths: &[PathBuf]) -> Command {
        let mut command = self.command(language, false);
        command.args(["--winappsdk-version", "2.5"]);
        for path in paths {
            command.arg("--winappsdk-bootstrap-path").arg(path);
        }
        command
    }

    fn generate(&self, language: &str, configured: bool) {
        succeeded(self.command(language, configured).output().unwrap());
    }

    fn refused_unchanged(&self, command: &mut Command, message: &str) -> String {
        let before = files(&self.output);
        let output = command.output().unwrap();
        assert!(!output.status.success(), "unexpected success");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(files(&self.output), before);
        assert!(fs::read_dir(&self.root).unwrap().all(|entry| {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            !name.contains(".dynwinrt-stage-") && !name.contains(".dynwinrt-backup-")
        }));
        String::from_utf8_lossy(&output.stderr).into_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn succeeded(output: Output) {
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
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    if root.is_dir() {
        visit(root, root, &mut result);
    }
    result
}

// A bounded PE fixture with one exported function; never loaded or executed.
fn pe(machine: u16) -> Vec<u8> {
    fn u16_at(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    fn u32_at(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    let mut bytes = vec![0; 0x600];
    bytes[0..2].copy_from_slice(b"MZ");
    u32_at(&mut bytes, 0x3c, 0x80);
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    u16_at(&mut bytes, 0x84, machine);
    u16_at(&mut bytes, 0x86, 1);
    let pe32 = machine == 0x014c;
    let optional_size = if pe32 { 0xe0 } else { 0xf0 };
    u16_at(&mut bytes, 0x94, optional_size);
    u16_at(&mut bytes, 0x96, 0x2022);
    let optional = 0x98;
    u16_at(&mut bytes, optional, if pe32 { 0x10b } else { 0x20b });
    u32_at(&mut bytes, optional + 16, 0x1100);
    u32_at(&mut bytes, optional + 20, 0x1000);
    u32_at(&mut bytes, optional + 32, 0x1000);
    u32_at(&mut bytes, optional + 36, 0x200);
    u32_at(&mut bytes, optional + 56, 0x2000);
    u32_at(&mut bytes, optional + 60, 0x200);
    u16_at(&mut bytes, optional + 68, 3);
    let directories = optional + if pe32 { 96 } else { 112 };
    u32_at(&mut bytes, directories - 4, 16);
    u32_at(&mut bytes, directories, 0x1000);
    u32_at(&mut bytes, directories + 4, 0xa0);
    let section = optional + usize::from(optional_size);
    bytes[section..section + 5].copy_from_slice(b".text");
    u32_at(&mut bytes, section + 8, 0x400);
    u32_at(&mut bytes, section + 12, 0x1000);
    u32_at(&mut bytes, section + 16, 0x400);
    u32_at(&mut bytes, section + 20, 0x200);
    u32_at(&mut bytes, section + 36, 0x60000020);
    u32_at(&mut bytes, 0x20c, 0x1060);
    u32_at(&mut bytes, 0x210, 1);
    u32_at(&mut bytes, 0x214, 1);
    u32_at(&mut bytes, 0x218, 1);
    u32_at(&mut bytes, 0x21c, 0x1030);
    u32_at(&mut bytes, 0x220, 0x1034);
    u32_at(&mut bytes, 0x224, 0x1038);
    u32_at(&mut bytes, 0x230, 0x1100);
    u32_at(&mut bytes, 0x234, 0x1080);
    bytes[0x260..0x26c].copy_from_slice(b"fixture.dll\0");
    let export = b"MddBootstrapInitialize2\0";
    bytes[0x280..0x280 + export.len()].copy_from_slice(export);
    bytes[0x300] = 0xc3;
    bytes
}

#[test]
fn bundle_generation_is_opt_in_and_exposes_precise_capabilities() {
    let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .arg("capabilities")
        .output()
        .unwrap();
    let capabilities = String::from_utf8_lossy(&output.stdout);
    for name in [
        "python.winappsdk-bootstrap-config",
        "javascript.winappsdk-bootstrap-config",
        "input.ref-list",
    ] {
        assert!(capabilities.lines().any(|line| line == name));
    }
    for language in ["py", "js"] {
        let fixture = Fixture::new();
        fixture.generate(language, false);
        assert!(!fixture.output.join(MANIFEST).exists());
        assert!(!fixture.package(language).join("_winappsdk.json").exists());
        assert!(!fixture.output.join("_dynwinrt_build").exists());
        let before = files(&fixture.output);
        succeeded(
            fixture
                .command(language, true)
                .arg("--dry-run")
                .output()
                .unwrap(),
        );
        assert_eq!(files(&fixture.output), before);
    }
}

#[test]
fn original_dll_bytes_licenses_and_runtime_configuration_are_packaged() {
    for language in ["py", "js"] {
        let fixture = Fixture::new();
        fixture.generate(language, true);
        for arch in ["x64", "arm64"] {
            assert_eq!(
                fs::read(
                    fixture
                        .package(language)
                        .join("_native")
                        .join("winappsdk")
                        .join(arch)
                        .join(DLL)
                )
                .unwrap(),
                fs::read(fixture.root.join("inputs").join(arch).join(DLL)).unwrap()
            );
        }
        assert_eq!(
            fs::read_to_string(
                fixture
                    .package(language)
                    .join("_native/winappsdk/LICENSE.txt")
            )
            .unwrap(),
            "SDK test license\n"
        );
        assert!(
            fs::read_to_string(
                fixture
                    .package(language)
                    .join("_native/winappsdk/NOTICE.txt")
            )
            .unwrap()
            .contains("NOT licensed under dynwinrt's MIT")
        );
        let config = fs::read_to_string(fixture.package(language).join("_winappsdk.json")).unwrap();
        assert!(!config.contains(&fixture.root.to_string_lossy().to_string()));
        if language == "py" {
            let project = fs::read_to_string(fixture.output.join("pyproject.toml")).unwrap();
            assert!(project.contains("[project.entry-points.\"dynwinrt.winappsdk\"]"));
            assert!(project.contains("bindings._winappsdk:get_config"));
            assert!(project.contains("_native/winappsdk/*/*.dll"));
            assert!(
                fs::read_to_string(fixture.output.join("src/_dynwinrt_build/wheel.py"))
                    .unwrap()
                    .contains("win_amd64.win_arm64")
            );
        } else {
            let project: serde_json::Value =
                serde_json::from_slice(&fs::read(fixture.output.join("package.json")).unwrap())
                    .unwrap();
            assert_eq!(project["sideEffects"], true);
            assert_eq!(project["cpu"], serde_json::json!(["arm64", "x64"]));
            assert_eq!(
                project["peerDependencies"]["@microsoft/dynwinrt"],
                env!("CARGO_PKG_VERSION")
            );
            assert!(
                project["files"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|entry| entry == "_native/winappsdk")
            );
            let manifest = fs::read_to_string(fixture.output.join("package.json")).unwrap();
            let root_export = manifest.split("\".\": {").nth(1).unwrap();
            assert!(
                root_export.find("\"types\"").unwrap() < root_export.find("\"import\"").unwrap()
            );
            for name in [
                "index.js",
                "index.mjs",
                "index.proxy.js",
                "windows/bootstrap-test/Options.js",
            ] {
                let path = fixture.output.join(name);
                assert!(path.is_file(), "missing generated path {}", path.display());
                assert!(fs::read_to_string(path).unwrap().contains("_winappsdk.cjs"));
            }
        }
    }
}

#[test]
fn javascript_bootstrap_accepts_scoped_and_unscoped_package_names() {
    for runtime in [
        "@microsoft/dynwinrt",
        "dynwinrt-local",
        "@custom-scope/dynwinrt.runtime",
        "dynwinrt.runtime",
    ] {
        let fixture = Fixture::new();
        succeeded(
            fixture
                .command("js", true)
                .args(["--import-name", runtime])
                .output()
                .unwrap(),
        );
        let package: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.output.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(
            package["peerDependencies"],
            serde_json::json!({ runtime: env!("CARGO_PKG_VERSION") })
        );
        let registration = fs::read_to_string(fixture.output.join("_winappsdk.cjs")).unwrap();
        assert!(registration.contains(&format!("require(\"{runtime}/winappsdk\")")));
        let before = files(&fixture.output);
        succeeded(
            fixture
                .command("js", false)
                .args(["--import-name", runtime])
                .output()
                .unwrap(),
        );
        assert_eq!(files(&fixture.output), before);
    }
}

#[test]
fn javascript_bootstrap_rejects_non_package_imports_before_writing() {
    let fixture = Fixture::new();
    for existing in [false, true] {
        if existing {
            fixture.generate("js", false);
        }
        for runtime in [
            "../../bindings/js/dist/winrt.js",
            "./runtime.js",
            r"..\..\bindings\js\dist\winrt.js",
            r"C:\runtime\winrt.js",
            "C:/runtime/winrt.js",
            "/runtime/winrt.js",
            r"\\server\share\runtime.js",
            "file:///C:/runtime/winrt.js",
            "@microsoft/dynwinrt/dist/winrt.js",
            "dynwinrt/dist/winrt.js",
            "@scope",
            "@scope/",
            "@scope//runtime",
            "dynwinrt@0.1.0",
            "runtime name",
            "node_modules",
            ".runtime",
            "_runtime",
            "",
        ] {
            for dry_run in [false, true] {
                let mut command = fixture.command("js", true);
                command.args(["--import-name", runtime]);
                if dry_run {
                    command.arg("--dry-run");
                }
                let error = fixture.refused_unchanged(
                    &mut command,
                    "Bootstrap bundling requires --import-name to be an npm package name",
                );
                assert!(error.contains("clean output directory"));
                assert_eq!(fixture.output.exists(), existing);
            }
        }
    }
}

#[test]
fn javascript_file_imports_remain_supported_without_bootstrap() {
    for runtime in [
        "../../bindings/js/dist/winrt.js",
        "./runtime.js",
        "C:/runtime/winrt.js",
        "@microsoft/dynwinrt/dist/winrt.js",
    ] {
        let fixture = Fixture::new();
        succeeded(
            fixture
                .command("js", false)
                .args(["--import-name", runtime])
                .output()
                .unwrap(),
        );
        assert!(!fixture.output.join(MANIFEST).exists());
        assert!(!fixture.output.join("_winappsdk.cjs").exists());
        let package: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.output.join("package.json")).unwrap())
                .unwrap();
        assert!(package.get("peerDependencies").is_none());
    }
}

#[test]
fn retained_javascript_bootstrap_validates_imports_when_options_are_omitted() {
    let fixture = Fixture::new();
    fixture.generate("js", true);
    let runtime = "../../bindings/js/dist/winrt.js";
    fixture.refused_unchanged(
        fixture
            .command("js", false)
            .args(["--import-name", runtime]),
        "Bootstrap bundling requires --import-name to be an npm package name",
    );

    // Reproduce a bundle created by the old generator with a file-path runtime import.
    let manifest_path = fixture.output.join(MANIFEST);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["runtime_import"] = runtime.into();
    let registration = fs::read_to_string(fixture.output.join("_winappsdk.cjs"))
        .unwrap()
        .replace("@microsoft/dynwinrt", runtime);
    manifest["files"]["_winappsdk.cjs"] =
        format!("{:x}", Sha256::digest(registration.as_bytes())).into();
    fs::write(fixture.output.join("_winappsdk.cjs"), registration).unwrap();
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    for incremental in [false, true] {
        let mut command = fixture.command("js", false);
        command.args(["--import-name", runtime]);
        if incremental {
            command.args(["--class-name", "Widget"]);
        }
        fixture.refused_unchanged(
            &mut command,
            "Bootstrap bundling requires --import-name to be an npm package name",
        );
    }
}

#[test]
fn same_bundle_survives_full_and_incremental_passes_with_omitted_options() {
    for language in ["py", "js"] {
        let fixture = Fixture::new();
        fixture.generate(language, true);
        for explicit in [true, false] {
            let before = files(&fixture.output);
            fixture.generate(language, explicit);
            assert_eq!(files(&fixture.output), before);
        }
        succeeded(
            fixture
                .command(language, false)
                .args(["--class-name", "Widget"])
                .output()
                .unwrap(),
        );
        assert!(
            fixture
                .package(language)
                .join("_native/winappsdk/x64")
                .join(DLL)
                .is_file()
        );
        if language == "py" {
            for incremental in [true, false] {
                let mut command = fixture.command(language, false);
                command.arg("--no-pyi");
                if incremental {
                    command.args(["--class-name", "Widget"]);
                }
                succeeded(command.output().unwrap());
                assert!(!fixture.package(language).join("_winappsdk.pyi").exists());
                let inventory =
                    fs::read_to_string(fixture.output.join(".dynwinrt-generated-files")).unwrap();
                assert!(inventory.contains(DLL));
                assert!(fixture.output.join(MANIFEST).is_file());
            }
            fixture.generate(language, false);
            assert!(fixture.package(language).join("_winappsdk.pyi").is_file());
        }
    }
}

#[test]
fn conflicting_version_or_payload_requires_complete_regeneration_without_writes() {
    for language in ["py", "js"] {
        let fixture = Fixture::new();
        fixture.generate(language, true);
        let mut version = fixture.command(language, false);
        version
            .args(["--winappsdk-version", "2.6", "--winappsdk-bootstrap-path"])
            .arg(fixture.input("x64"))
            .arg("--winappsdk-bootstrap-path")
            .arg(fixture.input("arm64"));
        fixture.refused_unchanged(
            &mut version,
            "Conflicting Windows App SDK version or payload",
        );
        let mut bytes = pe(0x8664);
        bytes[0x500] = 1;
        fs::write(fixture.root.join("inputs/x64").join(DLL), bytes).unwrap();
        fixture.refused_unchanged(
            &mut fixture.command(language, true),
            "Conflicting Windows App SDK version or payload",
        );
        if language == "py" {
            let old_cache = fs::read_to_string(fixture.output.join("setup.cfg")).unwrap();
            let fresh = fixture.root.join("fresh").join("bindings");
            let mut command = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"));
            command
                .args(["generate", "--winmd"])
                .arg(&fixture.metadata)
                .args([
                    "--namespace",
                    "Windows.BootstrapTest",
                    "--lang",
                    "py",
                    "--output",
                ])
                .arg(&fresh)
                .args(["--winappsdk-version", "2.5", "--winappsdk-bootstrap-path"])
                .arg(fixture.input("x64"))
                .arg("--winappsdk-bootstrap-path")
                .arg(fixture.input("arm64"));
            succeeded(command.output().unwrap());
            let new_cache = fs::read_to_string(fresh.join("setup.cfg")).unwrap();
            assert_ne!(
                old_cache, new_cache,
                "payload changes must isolate setuptools build caches"
            );
        }
    }
}

#[test]
fn concrete_input_selects_its_pe_architecture_without_using_codegen_host_architecture() {
    for language in ["py", "js"] {
        for architecture in ["x64", "arm64"] {
            let fixture = Fixture::new();
            let other = if architecture == "x64" {
                "arm64"
            } else {
                "x64"
            };
            let renamed = fixture.input(other).with_file_name("renamed.bin");
            fs::copy(fixture.input(architecture), &renamed).unwrap();
            succeeded(
                fixture
                    .command_with_paths(language, std::slice::from_ref(&renamed))
                    .output()
                    .unwrap(),
            );
            let config: serde_json::Value = serde_json::from_slice(
                &fs::read(fixture.package(language).join("_winappsdk.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(config["dlls"].as_object().unwrap().len(), 1);
            assert!(config["dlls"].get(architecture).is_some());
            assert!(
                !fixture
                    .package(language)
                    .join("_native/winappsdk")
                    .join(other)
                    .exists()
            );
            if language == "py" {
                let setup = fs::read_to_string(fixture.output.join("src/_dynwinrt_build/wheel.py"))
                    .unwrap();
                let expected = if architecture == "x64" {
                    "win_amd64"
                } else {
                    "win_arm64"
                };
                assert!(setup.contains(&format!("'py3', 'none', '{expected}'")));
            } else {
                let package: serde_json::Value =
                    serde_json::from_slice(&fs::read(fixture.output.join("package.json")).unwrap())
                        .unwrap();
                assert_eq!(package["cpu"], serde_json::json!([architecture]));
            }
            let before = files(&fixture.output);
            succeeded(
                fixture
                    .command_with_paths(
                        language,
                        &[renamed.clone(), fixture.input(architecture), renamed],
                    )
                    .output()
                    .unwrap(),
            );
            assert_eq!(files(&fixture.output), before);
        }
    }
}

#[test]
fn malformed_pe_and_missing_exports_fail_before_output_changes() {
    for language in ["py", "js"] {
        for mutation in [
            "truncated",
            "not-dll",
            "missing-export",
            "x86",
            "unknown-machine",
            "section-bounds",
        ] {
            let fixture = Fixture::new();
            fixture.generate(language, false);
            let mut bytes = pe(0x8664);
            match mutation {
                "truncated" => bytes.truncate(80),
                "not-dll" => bytes[0x97] &= !0x20,
                "missing-export" => bytes[0x280] = b'X',
                "x86" => bytes[0x84..0x86].copy_from_slice(&0x014c_u16.to_le_bytes()),
                "unknown-machine" => bytes[0x84..0x86].copy_from_slice(&0xffff_u16.to_le_bytes()),
                "section-bounds" => {
                    bytes[0x19c..0x1a0].copy_from_slice(&0xfffffff0_u32.to_le_bytes())
                }
                _ => unreachable!(),
            }
            fs::write(fixture.input("x64"), bytes).unwrap();
            fixture.refused_unchanged(&mut fixture.command(language, true), "Bootstrap");
        }
    }
}

#[test]
fn invalid_flags_manual_collisions_and_publication_failures_are_atomic() {
    let fixture = Fixture::new();
    fixture.generate("py", false);
    for version in ["2", "2.5.1", "2.5-preview", "-1.2", "65536.1", "02.5"] {
        let mut command = fixture.command("py", false);
        command
            .arg(format!("--winappsdk-version={version}"))
            .arg("--winappsdk-bootstrap-path")
            .arg(fixture.input("x64"));
        fixture.refused_unchanged(&mut command, "--winappsdk-version");
    }
    for field in [
        "--winappsdk-version=2.5",
        "--winappsdk-bootstrap-path=missing.dll",
    ] {
        fixture.refused_unchanged(fixture.command("py", false).arg(field), "required");
    }
    fs::write(fixture.package("py").join("_winappsdk.json"), "user-owned").unwrap();
    fixture.refused_unchanged(
        &mut fixture.command("py", true),
        "Refusing to overwrite non-generated",
    );
    fs::remove_file(fixture.package("py").join("_winappsdk.json")).unwrap();
    fixture.refused_unchanged(
        fixture
            .command("py", true)
            .env("DYNWINRT_CODEGEN_TEST_FAIL_OUTPUT_COMMIT", "before_publish"),
        "Injected output transaction failure",
    );
    fixture.generate("py", true);
    fs::write(
        fixture
            .package("py")
            .join("_native/winappsdk/x64")
            .join(DLL),
        "modified",
    )
    .unwrap();
    fixture.refused_unchanged(
        &mut fixture.command("py", false),
        "generated resource was modified",
    );
}

#[test]
fn lost_bundle_manifest_cannot_silently_drop_configuration_or_orphan_dlls() {
    for language in ["py", "js"] {
        let fixture = Fixture::new();
        fixture.generate(language, true);
        fs::remove_file(fixture.output.join(MANIFEST)).unwrap();
        fixture.refused_unchanged(
            &mut fixture.command(language, false),
            "bundle artifacts have no ownership manifest",
        );
    }
}

#[test]
fn projected_namespace_and_private_build_hook_remain_distinct() {
    let fixture = Fixture::new();
    succeeded(
        fixture
            .command("py", true)
            .args(["--class-name", "_dynwinrt_build.Wheel"])
            .output()
            .unwrap(),
    );
    let projected =
        fs::read_to_string(fixture.package("py").join("dynwinrt_build/wheel.py")).unwrap();
    let build_hook =
        fs::read_to_string(fixture.output.join("src/_dynwinrt_build/wheel.py")).unwrap();
    assert!(projected.contains(" import Wheel"));
    assert!(!projected.contains("class BootstrapWheel"));
    assert!(build_hook.contains("class BootstrapWheel"));
}

#[test]
fn python_project_rename_relocates_owned_package_and_bundle_without_changing_type_identity() {
    for bundled in [false, true] {
        let mut fixture = Fixture::new();
        fixture.generate("py", bundled);
        let types = fs::read(fixture.output.join(".dynwinrt-generated-types")).unwrap();
        let old_source = fs::read(
            fixture
                .package("py")
                .join("windows__bootstrap_test__widget.py"),
        )
        .unwrap();
        fs::write(
            fixture.package("py").join("manual.txt"),
            "preserve user content",
        )
        .unwrap();
        let renamed = fixture.root.join("Renamed Bindings");
        fs::rename(&fixture.output, &renamed).unwrap();
        fixture.output = renamed;
        succeeded(
            fixture
                .command("py", false)
                .args(["--class-name", "Widget"])
                .output()
                .unwrap(),
        );
        let package = fixture.package("py");
        assert!(package.ends_with("src/renamed_bindings"));
        assert_eq!(
            fs::read(package.join("windows__bootstrap_test__widget.py")).unwrap(),
            old_source
        );
        assert_eq!(
            fs::read(fixture.output.join(".dynwinrt-generated-types")).unwrap(),
            types
        );
        assert_eq!(
            fs::read_to_string(fixture.output.join("src/bindings/manual.txt")).unwrap(),
            "preserve user content"
        );
        assert!(!fixture.output.join("src/bindings/__init__.py").exists());
        let inventory =
            fs::read_to_string(fixture.output.join(".dynwinrt-generated-files")).unwrap();
        assert!(!inventory.replace('\\', "/").contains("src/bindings/"));
        assert!(
            inventory
                .replace('\\', "/")
                .contains("src/renamed_bindings/py.typed")
        );
        let project = fs::read_to_string(fixture.output.join("pyproject.toml")).unwrap();
        assert!(project.contains("name = \"renamed-bindings\""));
        assert!(project.contains("package-dir = { \"\" = \"src\" }"));
        if bundled {
            assert!(project.contains("renamed_bindings._winappsdk:get_config"));
            assert_eq!(
                fs::read(package.join("_native/winappsdk/x64").join(DLL)).unwrap(),
                fs::read(fixture.input("x64")).unwrap()
            );
            assert!(
                !fs::read_to_string(fixture.output.join(MANIFEST))
                    .unwrap()
                    .contains("src/bindings/")
            );
        }
        let before = files(&fixture.output);
        succeeded(
            fixture
                .command("py", false)
                .args(["--class-name", "Widget"])
                .output()
                .unwrap(),
        );
        assert_eq!(files(&fixture.output), before);
    }
}

#[test]
fn python_project_rename_conflicts_and_publication_failures_preserve_both_layouts() {
    let mut fixture = Fixture::new();
    fixture.generate("py", true);
    let renamed = fixture.root.join("renamed_bindings");
    fs::rename(&fixture.output, &renamed).unwrap();
    fixture.output = renamed;
    let destination = fixture.output.join("src/renamed_bindings/__init__.py");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(&destination, "user-owned").unwrap();
    fixture.refused_unchanged(
        &mut fixture.command("py", false),
        "destination already exists",
    );
    fs::remove_file(destination).unwrap();
    fixture.refused_unchanged(
        fixture
            .command("py", false)
            .env("DYNWINRT_CODEGEN_TEST_FAIL_OUTPUT_COMMIT", "before_publish"),
        "Injected output transaction failure",
    );
    assert!(fixture.output.join("src/bindings/__init__.py").is_file());
    assert!(
        !fixture
            .output
            .join("src/renamed_bindings/__init__.py")
            .exists()
    );
}

#[test]
fn python_bootstrap_rejects_build_helper_as_runtime_package_name() {
    let mut fixture = Fixture::new();
    fixture.output = fixture.root.join("_dynwinrt_build");
    fixture.refused_unchanged(
        &mut fixture.command("py", true),
        "conflicts with the build-only helper",
    );
}

#[test]
fn input_order_and_identical_duplicates_preserve_payloads_licenses_and_cache() {
    for language in ["py", "js"] {
        let mut fixture = Fixture::new();
        fs::remove_file(fixture.root.join("inputs/license.txt")).unwrap();
        let duplicate = fixture.root.join("licensed-copy/renamed.bin");
        fs::create_dir(duplicate.parent().unwrap()).unwrap();
        fs::copy(fixture.input("x64"), &duplicate).unwrap();
        fs::write(
            duplicate.parent().unwrap().join("license.txt"),
            "License from a duplicate payload's source\n",
        )
        .unwrap();
        let mut paths = vec![
            fixture.input("x64"),
            fixture.input("arm64"),
            fixture.input("x64"),
            duplicate.clone(),
        ];
        succeeded(
            fixture
                .command_with_paths(language, &paths)
                .output()
                .unwrap(),
        );
        assert_eq!(
            fs::read_to_string(
                fixture
                    .package(language)
                    .join("_native/winappsdk/LICENSE.txt")
            )
            .unwrap(),
            "License from a duplicate payload's source\n"
        );
        let before = files(&fixture.output);
        assert_eq!(
            before
                .keys()
                .filter(|path| path.extension().is_some_and(|extension| extension == "dll"))
                .count(),
            2
        );
        paths.reverse();
        succeeded(
            fixture
                .command_with_paths(language, &paths)
                .output()
                .unwrap(),
        );
        assert_eq!(files(&fixture.output), before);
        fixture.output = fixture.root.join("reverse/bindings");
        succeeded(
            fixture
                .command_with_paths(language, &paths)
                .output()
                .unwrap(),
        );
        assert_eq!(files(&fixture.output), before);
        fixture.output = fixture.root.join("deduplicated/bindings");
        succeeded(
            fixture
                .command_with_paths(language, &[duplicate, fixture.input("arm64")])
                .output()
                .unwrap(),
        );
        assert_eq!(files(&fixture.output), before);
    }
}

#[test]
fn different_valid_payloads_for_one_architecture_report_both_sources_and_preserve_output() {
    for language in ["py", "js"] {
        let mut fixture = Fixture::new();
        let original = fixture.input("x64");
        let serviced = original.with_file_name("serviced.dll");
        let mut bytes = pe(0x8664);
        bytes[0x500] = 1;
        fs::write(&serviced, bytes).unwrap();

        fixture.output = fixture.root.join("serviced/bindings");
        succeeded(
            fixture
                .command_with_paths(language, std::slice::from_ref(&serviced))
                .output()
                .unwrap(),
        );
        fixture.output = fixture.root.join("bindings");
        fixture.generate(language, true);
        for paths in [
            [original.clone(), serviced.clone()],
            [serviced.clone(), original.clone()],
        ] {
            let error = fixture.refused_unchanged(
                &mut fixture.command_with_paths(language, &paths),
                "Conflicting Bootstrap DLL inputs for x64",
            );
            assert!(error.contains(&original.display().to_string()), "{error}");
            assert!(error.contains(&serviced.display().to_string()), "{error}");
        }
    }
}

#[test]
fn identical_dlls_do_not_hide_conflicting_source_licenses() {
    for language in ["py", "js"] {
        let fixture = Fixture::new();
        fixture.generate(language, true);
        let duplicate = fixture.root.join("alternate/renamed.dll");
        fs::create_dir(duplicate.parent().unwrap()).unwrap();
        fs::copy(fixture.input("x64"), &duplicate).unwrap();
        let duplicate_license = duplicate.parent().unwrap().join("license.txt");
        fs::write(&duplicate_license, "Different SDK license\n").unwrap();
        let original_license = fixture.root.join("inputs").join("license.txt");
        for paths in [
            [fixture.input("x64"), duplicate.clone()],
            [duplicate.clone(), fixture.input("x64")],
        ] {
            let error = fixture.refused_unchanged(
                &mut fixture.command_with_paths(language, &paths),
                "conflicting license.txt files",
            );
            assert!(
                error.contains(&original_license.display().to_string()),
                "{error}"
            );
            assert!(
                error.contains(&duplicate_license.display().to_string()),
                "{error}"
            );
        }
    }
}

#[test]
fn missing_and_empty_paths_never_skip_an_input_or_change_existing_output() {
    for language in ["py", "js"] {
        let fixture = Fixture::new();
        fixture.generate(language, true);
        fixture.refused_unchanged(&mut fixture.command_with_paths(language, &[]), "required");
        fixture.refused_unchanged(
            fixture
                .command(language, false)
                .arg("--winappsdk-bootstrap-path")
                .arg(fixture.input("x64")),
            "required",
        );
        fixture.refused_unchanged(
            fixture.command(language, false).args([
                "--winappsdk-version",
                "2.5",
                "--winappsdk-bootstrap-path",
            ]),
            "required",
        );
        for path in ["", "   "] {
            fixture.refused_unchanged(
                &mut fixture
                    .command_with_paths(language, &[fixture.input("x64"), PathBuf::from(path)]),
                "--winappsdk-bootstrap-path",
            );
        }
        let empty = fixture.root.join("empty.dll");
        fs::write(&empty, []).unwrap();
        for path in [fixture.root.join("missing.dll"), empty] {
            fixture.refused_unchanged(
                &mut fixture.command_with_paths(language, &[fixture.input("x64"), path]),
                "Bootstrap input",
            );
        }
        fixture.refused_unchanged(
            fixture
                .command_with_paths(language, &[fixture.input("x64")])
                .arg(fixture.input("arm64")),
            "unexpected argument",
        );
    }
}

#[test]
fn directory_inputs_match_files_recursively_without_changing_the_bundle() {
    for language in ["py", "js"] {
        let fixture = Fixture::new();
        fixture.generate(language, true);
        let before = files(&fixture.output);
        let nested = fixture.root.join("inputs/deep/nested");
        fs::create_dir_all(&nested).unwrap();
        fs::copy(fixture.input("x64"), nested.join(DLL.to_ascii_lowercase())).unwrap();
        fs::write(nested.join("unrelated.dll"), "not a PE").unwrap();
        fs::write(fixture.root.join(DLL), "outside the search root").unwrap();
        let inputs = fixture.root.join("inputs");
        for paths in [
            vec![inputs.clone()],
            vec![
                fixture.input("arm64"),
                inputs.clone(),
                nested.clone(),
                inputs.clone(),
            ],
            vec![nested, inputs, fixture.input("x64")],
        ] {
            succeeded(
                fixture
                    .command_with_paths(language, &paths)
                    .output()
                    .unwrap(),
            );
            assert_eq!(files(&fixture.output), before);
        }
        fixture.generate(language, false);
        assert_eq!(files(&fixture.output), before);
    }
}

#[test]
fn directory_scans_ignore_unsupported_pe_architectures_but_explicit_files_remain_strict() {
    for language in ["py", "js"] {
        let fixture = Fixture::new();
        fixture.generate(language, true);
        let before = files(&fixture.output);
        let inputs = fixture.root.join("inputs");
        for (folder, machine) in [("x86", 0x014c), ("unknown", 0xffff), ("arm64ec", 0x8664)] {
            let dir = inputs.join(folder);
            fs::create_dir(&dir).unwrap();
            fs::write(dir.join(DLL), pe(machine)).unwrap();
        }
        let result = fixture
            .command_with_paths(language, std::slice::from_ref(&inputs))
            .output()
            .unwrap();
        let diagnostics = String::from_utf8_lossy(&result.stderr);
        assert!(
            diagnostics.contains("ignoring unsupported Bootstrap PE machine 0x014c"),
            "{diagnostics}"
        );
        assert!(
            diagnostics.contains("ignoring unsupported Bootstrap PE machine 0xffff"),
            "{diagnostics}"
        );
        succeeded(result);
        assert_eq!(files(&fixture.output), before);
        for folder in ["x86", "unknown"] {
            let file = inputs.join(folder).join(DLL);
            for paths in [
                vec![inputs.clone(), file.clone()],
                vec![file, inputs.clone()],
            ] {
                fixture.refused_unchanged(
                    &mut fixture.command_with_paths(language, &paths),
                    "in explicit file",
                );
            }
        }
    }
}

#[test]
fn empty_and_unsupported_only_directories_fail_even_with_other_valid_inputs() {
    for language in ["py", "js"] {
        let fixture = Fixture::new();
        fixture.generate(language, true);
        let empty = fixture.root.join("empty");
        let unsupported = fixture.root.join("unsupported");
        fs::create_dir(&empty).unwrap();
        fs::create_dir(&unsupported).unwrap();
        fs::write(empty.join("another-name.dll"), pe(0x8664)).unwrap();
        fs::write(unsupported.join(DLL), pe(0x014c)).unwrap();
        for directory in [empty, unsupported] {
            for paths in [
                vec![directory.clone()],
                vec![fixture.input("x64"), directory.clone()],
            ] {
                let error = fixture.refused_unchanged(
                    &mut fixture.command_with_paths(language, &paths),
                    "No supported Bootstrap DLLs found in directory",
                );
                assert!(error.contains(&directory.display().to_string()), "{error}");
            }
        }
    }
}

#[test]
fn damaged_directory_matches_cannot_be_ignored_as_unsupported() {
    for language in ["py", "js"] {
        let fixture = Fixture::new();
        fixture.generate(language, true);
        let directory = fixture.root.join("inputs/bad");
        fs::create_dir(&directory).unwrap();
        for machine in [0x8664, 0x014c, 0xffff] {
            let mut bytes = pe(machine);
            bytes[0x280] = b'X';
            fs::write(directory.join(DLL), bytes).unwrap();
            fixture.refused_unchanged(
                &mut fixture.command_with_paths(language, &[fixture.root.join("inputs")]),
                "does not export MddBootstrapInitialize2",
            );
        }
        fs::write(directory.join(DLL), b"MZ").unwrap();
        fixture.refused_unchanged(
            &mut fixture.command_with_paths(language, &[fixture.root.join("inputs")]),
            "Invalid Bootstrap PE",
        );
    }
}

#[test]
fn directory_payload_and_license_conflicts_preserve_existing_output() {
    for language in ["py", "js"] {
        let fixture = Fixture::new();
        fixture.generate(language, true);
        let inputs = fixture.root.join("inputs");
        let directory = inputs.join("conflicting");
        fs::create_dir(&directory).unwrap();
        let mut changed = pe(0x8664);
        changed[0x500] = 1;
        fs::write(directory.join(DLL), changed).unwrap();
        let error = fixture.refused_unchanged(
            &mut fixture.command_with_paths(language, std::slice::from_ref(&inputs)),
            "Conflicting Bootstrap DLL inputs for x64",
        );
        assert!(
            error.contains("conflicting") && error.contains("x64"),
            "{error}"
        );
        fs::copy(fixture.input("x64"), directory.join(DLL)).unwrap();
        fs::write(directory.join("license.txt"), "Different source license").unwrap();
        fixture.refused_unchanged(
            &mut fixture.command_with_paths(language, &[inputs]),
            "conflicting license.txt files",
        );
    }
}

#[cfg(windows)]
#[test]
fn directory_scans_reject_junction_escapes_cycles_and_linked_roots() {
    let node = std::env::var_os("DYNWINRT_TEST_NODE").unwrap_or_else(|| "node".into());
    for language in ["py", "js"] {
        let fixture = Fixture::new();
        fixture.generate(language, true);
        let inputs = fixture.root.join("inputs");
        let outside = fixture.root.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join(DLL), "outside DLL must never be read").unwrap();
        let junction = inputs.join("linked");
        for target in [&outside, &inputs] {
            succeeded(Command::new(&node).args([
                        "-e",
                        "require('node:fs').symlinkSync(process.argv[1], process.argv[2], 'junction')",
                    ]).arg(target).arg(&junction).output().unwrap());
            let error = fixture.refused_unchanged(
                &mut fixture.command_with_paths(language, std::slice::from_ref(&inputs)),
                "Refusing linked Bootstrap directory entry",
            );
            assert!(error.contains("linked"), "{error}");
            fixture.refused_unchanged(
                &mut fixture.command_with_paths(language, std::slice::from_ref(&junction)),
                "Refusing linked Bootstrap directory",
            );
            fs::remove_dir(&junction).unwrap();
        }
    }
}

#[cfg(windows)]
#[test]
fn unreadable_directory_matches_and_subdirectories_fail_before_publication() {
    use std::os::windows::fs::OpenOptionsExt;

    for language in ["py", "js"] {
        let fixture = Fixture::new();
        fixture.generate(language, true);
        let inputs = fixture.root.join("inputs");
        let file = fixture.input("x64");
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&file)
            .unwrap();
        fixture.refused_unchanged(
            &mut fixture.command_with_paths(language, std::slice::from_ref(&inputs)),
            "Bootstrap",
        );
        drop(lock);
        let directory = inputs.join("unreadable");
        fs::create_dir(&directory).unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .custom_flags(0x02000000) // FILE_FLAG_BACKUP_SEMANTICS opens the directory handle.
            .open(&directory)
            .unwrap();
        fixture.refused_unchanged(
            &mut fixture.command_with_paths(language, &[inputs]),
            "Bootstrap",
        );
        drop(lock);
    }
}

#[test]
fn bootstrap_cli_help_uses_the_final_sdk_scoped_name_without_a_legacy_alias() {
    let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args(["generate", "--help"])
        .output()
        .unwrap();
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("--winappsdk-version <MAJOR.MINOR>"));
    assert!(help.contains("--winappsdk-bootstrap-path <PATH>"));
    assert!(help.contains("directory to search recursively"));
    assert!(!help.contains("--bootstrap-dll-path"));
    succeeded(output);
    let fixture = Fixture::new();
    fixture.refused_unchanged(
        fixture
            .command("py", false)
            .args(["--winappsdk-version", "2.5", "--bootstrap-dll-path"])
            .arg(fixture.input("x64")),
        "unexpected argument",
    );
}

#[test]
fn input_paths_are_literal_without_architecture_placeholder_expansion() {
    for language in ["py", "js"] {
        let fixture = Fixture::new();
        fixture.generate(language, false);
        let literal = fixture.input("{arch}");
        fixture.refused_unchanged(
            &mut fixture.command_with_paths(language, std::slice::from_ref(&literal)),
            "Cannot inspect Bootstrap input",
        );
        fs::create_dir(literal.parent().unwrap()).unwrap();
        fs::copy(fixture.input("x64"), &literal).unwrap();
        succeeded(
            fixture
                .command_with_paths(language, &[literal])
                .output()
                .unwrap(),
        );
        let config: serde_json::Value = serde_json::from_slice(
            &fs::read(fixture.package(language).join("_winappsdk.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(config["dlls"].as_object().unwrap().len(), 1);
        assert!(config["dlls"].get("x64").is_some());
        assert!(
            !fixture
                .package(language)
                .join("_native/winappsdk/arm64")
                .exists()
        );
    }
}

fn native_metadata() -> Option<PathBuf> {
    let path = std::env::var_os("DYNWINRT_WIN32_WINMD")
        .map(PathBuf::from)
        .filter(|path| path.is_file());
    if path.is_none() {
        assert_ne!(
            std::env::var("DYNWINRT_REQUIRE_WIN32_METADATA").as_deref(),
            Ok("1"),
            "Bootstrap/native generation tests require DYNWINRT_WIN32_WINMD"
        );
        eprintln!("Skipping: DYNWINRT_WIN32_WINMD is not configured");
    }
    path
}

fn native_command(fixture: &Fixture, winmd: &Path, classes: &str, configured: bool) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"));
    command
        .args(["generate", "--winmd"])
        .arg(format!(
            "{};{}",
            winmd.display(),
            fixture.metadata.display()
        ))
        .args(["--lang", "js", "--class-name", classes, "--output"])
        .arg(&fixture.output)
        .env_remove("DYNWINRT_CODEGEN_TEST_FAIL_OUTPUT_COMMIT");
    if configured {
        command
            .args(["--winappsdk-version", "2.5", "--winappsdk-bootstrap-path"])
            .arg(fixture.input("x64"));
    }
    command
}

#[test]
fn win32_hashes_include_registration_before_repeated_generation_and_verify_tampering() {
    let Some(winmd) = native_metadata() else {
        return;
    };
    let namespace = "Windows.Win32.System.SystemInformation";
    let class = format!("{namespace}.Apis");
    for existing_unconfigured in [false, true] {
        let fixture = Fixture::new();
        succeeded(
            native_command(&fixture, &winmd, &class, !existing_unconfigured)
                .output()
                .unwrap(),
        );
        if existing_unconfigured {
            succeeded(
                fixture
                    .command_with_paths("js", &[fixture.input("x64")])
                    .output()
                    .unwrap(),
            );
        }
        let before = files(&fixture.output);
        for configured in [true, false] {
            succeeded(
                native_command(&fixture, &winmd, &class, configured)
                    .output()
                    .unwrap(),
            );
            assert_eq!(files(&fixture.output), before);
        }
        let path = fixture
            .output
            .join("win32/windows/win32/system/system-information/Apis.js");
        let bytes = fs::read(&path).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(fixture.output.join("win32/.dynwinrt-win32-manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            manifest["namespaces"][namespace]["javascript_sha256"],
            format!("{:x}", Sha256::digest(&bytes))
        );
        fs::write(&path, [bytes.as_slice(), b"\n// changed\n"].concat()).unwrap();
        fixture.refused_unchanged(
            &mut native_command(&fixture, &winmd, &class, false),
            "does not match its manifest",
        );
    }
}

#[test]
fn shared_com_helpers_remain_identical_when_configuration_is_retained_or_added() {
    let Some(winmd) = native_metadata() else {
        return;
    };
    let first = "Windows.Win32.Graphics.Dxgi.IDXGIObject";
    let second = "Windows.Win32.System.ClrHosting.IHostMalloc";
    for existing_unconfigured in [false, true] {
        let fixture = Fixture::new();
        succeeded(
            native_command(&fixture, &winmd, first, !existing_unconfigured)
                .output()
                .unwrap(),
        );
        if existing_unconfigured {
            succeeded(
                fixture
                    .command_with_paths("js", &[fixture.input("x64")])
                    .output()
                    .unwrap(),
            );
        }
        let shared = fixture.output.join("com/unsafe/runtime.js");
        let before = fs::read(&shared).unwrap();
        succeeded(
            native_command(&fixture, &winmd, second, false)
                .output()
                .unwrap(),
        );
        assert_eq!(fs::read(&shared).unwrap(), before);
        succeeded(
            native_command(&fixture, &winmd, first, true)
                .output()
                .unwrap(),
        );
        assert_eq!(fs::read(&shared).unwrap(), before);
    }
}

#[test]
fn configured_report_only_com_errors_roll_back_instead_of_stripping_package_metadata() {
    let Some(winmd) = native_metadata() else {
        return;
    };
    for classes in [
        "Windows.Win32.Media.MediaFoundation.MFASYNCRESULT",
        "Windows.Win32.Media.MediaFoundation.MFASYNCRESULT,Windows.BootstrapTest.Widget",
    ] {
        let fixture = Fixture::new();
        fixture.generate("js", true);
        fixture.refused_unchanged(
            &mut native_command(&fixture, &winmd, classes, false),
            "emitted a support report but no callable class",
        );
    }
}

#[test]
fn npm_wrapper_pins_its_published_version_not_the_precompiled_cargo_version() {
    let node = std::env::var_os("DYNWINRT_TEST_NODE").unwrap_or_else(|| "node".into());
    let probe = Command::new(&node).args(["-p", "process.arch"]).output();
    let Ok(probe) = probe else {
        eprintln!("Skipping npm wrapper check: Node.js is unavailable");
        return;
    };
    assert!(probe.status.success());
    let architecture = String::from_utf8(probe.stdout).unwrap();
    let fixture = Fixture::new();
    let wrapper = fixture.root.join("npm");
    let binary = wrapper.join("bin").join(architecture.trim());
    fs::create_dir_all(&binary).unwrap();
    fs::copy(
        env!("CARGO_BIN_EXE_dynwinrt-codegen"),
        binary.join("dynwinrt-codegen.exe"),
    )
    .unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("npm/cli.js"),
        wrapper.join("cli.js"),
    )
    .unwrap();
    let version = "0.1.0-preview.999";
    fs::write(
        wrapper.join("package.json"),
        format!(r#"{{"version":"{version}"}}"#),
    )
    .unwrap();
    for configured in [true, false] {
        let args = fixture.command("js", configured);
        succeeded(
            Command::new(&node)
                .arg(wrapper.join("cli.js"))
                .args(args.get_args())
                .env("DYNWINRT_CODEGEN_NPM_VERSION", "9.9.9")
                .output()
                .unwrap(),
        );
        let package: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.output.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(package["version"], version);
        assert_eq!(package["peerDependencies"]["@microsoft/dynwinrt"], version);
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.output.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(manifest["generator_version"], version);
    }
    fixture.refused_unchanged(
        &mut fixture.command("js", false),
        "Unsupported Windows App SDK bundle manifest",
    );
    let python = Fixture::new();
    let args = python.command("py", true);
    succeeded(
        Command::new(node)
            .arg(wrapper.join("cli.js"))
            .args(args.get_args())
            .output()
            .unwrap(),
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(python.output.join(MANIFEST)).unwrap()).unwrap();
    assert_eq!(manifest["generator_version"], env!("CARGO_PKG_VERSION"));
}
