// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use windows_metadata::{FieldAttributes, Type, TypeAttributes, Value, writer};

const STAMP: &str = ".dynwinrt-generator.json";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    metadata: PathBuf,
    output: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "dpg{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&root).unwrap();
        let metadata = root.join("VersionGuard.winmd");
        let mut file = writer::File::new("PythonVersionGuard");
        let base = file.TypeRef("System", "Enum");
        file.TypeDef(
            "Windows.VersionGuard",
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
            &Type::named("Windows.VersionGuard", "Options"),
            FieldAttributes::Public
                | FieldAttributes::Static
                | FieldAttributes::Literal
                | FieldAttributes::HasDefault,
        );
        file.Constant(writer::HasConstant::Field(member), &Value::I32(1));
        fs::write(&metadata, file.into_stream()).unwrap();
        let output = root.join("bindings");
        Self {
            root,
            metadata,
            output,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"));
        command
            .args(["generate", "--winmd"])
            .arg(&self.metadata)
            .args([
                "--namespace",
                "Windows.VersionGuard",
                "--lang",
                "py",
                "--output",
            ])
            .arg(&self.output)
            .env_remove("DYNWINRT_CODEGEN_TEST_FAIL_OUTPUT_COMMIT");
        command
    }

    fn generate(&self) {
        let output = self.command().output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    fn refused_unchanged(&self, command: &mut Command) -> String {
        let before = files(&self.output);
        let output: Output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("Manually clean the dedicated codegen output"),
            "{error}"
        );
        assert!(
            error.contains("regenerate the entire bindings selection"),
            "{error}"
        );
        assert!(
            error.contains("Existing output has not been changed"),
            "{error}"
        );
        assert_eq!(files(&self.output), before);
        for entry in fs::read_dir(&self.root).unwrap() {
            let name = entry.unwrap().file_name();
            let name = name.to_string_lossy();
            assert!(!name.contains(".dynwinrt-stage-"), "{name}");
            assert!(!name.contains(".dynwinrt-backup-"), "{name}");
        }
        error.into_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
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

#[test]
fn new_and_non_generated_output_stamp_the_current_producer() {
    for existing in ["new", "empty", "user"] {
        let fixture = Fixture::new();
        if existing != "new" {
            fs::create_dir(&fixture.output).unwrap();
        }
        if existing == "user" {
            fs::write(fixture.output.join("manual.py"), "VALUE = 42\n").unwrap();
            fs::write(fixture.output.join("manual.pyi"), "VALUE: int\n").unwrap();
        }
        fixture.generate();
        let stamp: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.output.join(STAMP)).unwrap()).unwrap();
        assert_eq!(stamp["generator_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(stamp.as_object().unwrap().len(), 1);
        let inventory =
            fs::read_to_string(fixture.output.join(".dynwinrt-generated-files")).unwrap();
        assert!(inventory.lines().any(|line| line == STAMP));
        if existing == "user" {
            assert_eq!(
                fs::read_to_string(fixture.output.join("manual.py")).unwrap(),
                "VALUE = 42\n"
            );
        }
        let before = files(&fixture.output);
        fixture.generate();
        assert_eq!(files(&fixture.output), before);
    }
}

#[test]
fn changed_preview_upgrade_and_downgrade_producers_refuse_before_writes() {
    let fixture = Fixture::new();
    fixture.generate();
    for version in ["0.0.1", "99.0.0", "0.1.0-preview.21"] {
        fs::write(
            fixture.output.join(STAMP),
            format!(r#"{{"generator_version":"{version}"}}"#),
        )
        .unwrap();
        for dry_run in [false, true] {
            let mut command = fixture.command();
            if dry_run {
                command.arg("--dry-run");
            }
            let error = fixture.refused_unchanged(&mut command);
            assert!(error.contains(version), "{error}");
            assert!(error.contains(env!("CARGO_PKG_VERSION")), "{error}");
        }
    }
}

#[test]
fn legacy_output_with_or_without_inventories_requires_full_regeneration() {
    for artifacts in [
        vec![".dynwinrt-generated-files"],
        vec![".dynwinrt-generated-types"],
        vec!["old.py"],
        vec!["old.pyi"],
        vec!["windows\\__init__.py", "windows\\old.py"],
    ] {
        let fixture = Fixture::new();
        fs::create_dir(&fixture.output).unwrap();
        for artifact in artifacts {
            let path = fixture.output.join(artifact);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(
                path,
                "# Generated by dynwinrt-codegen \u{2014} do not edit\n",
            )
            .unwrap();
        }
        let error = fixture.refused_unchanged(&mut fixture.command());
        assert!(error.contains("no recorded generator_version"), "{error}");
    }
}

#[test]
fn missing_and_corrupt_producer_metadata_preserve_all_existing_output() {
    let fixture = Fixture::new();
    fixture.generate();
    fs::remove_file(fixture.output.join(STAMP)).unwrap();
    fixture.refused_unchanged(&mut fixture.command());
    for stamp in [
        "",
        "{",
        "{}",
        r#"{"generator_version":null}"#,
        r#"{"generator_version":1}"#,
        r#"{"generator_version":""}"#,
        r#"{"generator_version":"0.1.0","schema_version":2}"#,
        r#"{"generator_version":"0.1.0","generator_version":"0.1.0"}"#,
    ] {
        fs::write(fixture.output.join(STAMP), stamp).unwrap();
        fixture.refused_unchanged(&mut fixture.command());
    }
    fs::remove_file(fixture.output.join(STAMP)).unwrap();
    fs::create_dir(fixture.output.join(STAMP)).unwrap();
    fixture.refused_unchanged(&mut fixture.command());
}

#[test]
fn failed_publication_does_not_stamp_or_modify_the_old_output() {
    let fixture = Fixture::new();
    fs::create_dir(&fixture.output).unwrap();
    fs::write(fixture.output.join("manual.py"), "VALUE = 42\n").unwrap();
    let before = files(&fixture.output);
    let output = fixture
        .command()
        .env("DYNWINRT_CODEGEN_TEST_FAIL_OUTPUT_COMMIT", "before_publish")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Injected output transaction failure")
    );
    assert_eq!(files(&fixture.output), before);
    assert!(!fixture.output.join(STAMP).exists());
}
