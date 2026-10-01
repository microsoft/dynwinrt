// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#![cfg(windows)]

use std::collections::BTreeMap;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";
const WARNING: &str = "warning: longest Python output path";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "dpp{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn output(&self, name: &str, length: usize) -> PathBuf {
        let parent = self.0.join(name);
        let leaf = "generated_bindings";
        let padding = length
            .checked_sub(path_length(&parent) + leaf.len() + 2)
            .unwrap();
        assert!((1..=255).contains(&padding));
        let output = parent.join("x".repeat(padding)).join(leaf);
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        assert_eq!(path_length(&output), length);
        output
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn path_length(path: &Path) -> usize {
    path.as_os_str().encode_wide().count()
}

fn command(output: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"));
    command
        .args([
            "generate",
            "--winmd",
            WINDOWS_WINMD,
            "--lang",
            "py",
            "--output",
        ])
        .arg(output)
        .env_remove("DYNWINRT_CODEGEN_TEST_FAIL_OUTPUT_COMMIT");
    command
}

fn device_command(output: &Path) -> Command {
    let mut command = command(output);
    command.args([
        "--class-name",
        "Windows.Devices.Enumeration.DeviceInformationCustomPairing",
    ]);
    command
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn successful(output: &Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        stderr(output),
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
fn python_path_diagnostic_uses_actual_final_files_not_transactional_paths() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }
    let fixture = Fixture::new();
    let deep = fixture.output("deep", 137);
    let shallow = fixture.output("short", 130);
    let output = device_command(&deep).output().unwrap();
    successful(&output);
    let generated = files(&deep);
    let (longest, length) = generated
        .keys()
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == "py" || ext == "pyi")
        })
        .map(|relative| {
            let absolute = deep.join(relative);
            let length = path_length(&absolute);
            (absolute, length)
        })
        .max_by(|left, right| (left.1, &left.0).cmp(&(right.1, &right.0)))
        .unwrap();
    assert_eq!(length, 262);
    let message = stderr(&output);
    assert_eq!(message.matches(WARNING).count(), 1, "{message}");
    assert!(message.contains(&format!("{length} UTF-16 code units")));
    assert!(
        message.contains(&longest.display().to_string()),
        "{message}"
    );
    assert!(message.contains("at least 3 code units"));
    assert!(!message.contains(".dynwinrt-stage-"));

    let ordinary = device_command(&shallow).output().unwrap();
    successful(&ordinary);
    assert!(!stderr(&ordinary).contains(WARNING));
    assert_eq!(generated, files(&shallow));
    assert!(
        String::from_utf8_lossy(&ordinary.stdout)
            .lines()
            .filter_map(|line| line.strip_prefix("Generated "))
            .any(|path| path.encode_utf16().count() >= 260),
        "the no-warning control must exercise over-budget staging paths"
    );

    let unpublished = fixture.output("failure", 137);
    let failure = device_command(&unpublished)
        .env("DYNWINRT_CODEGEN_TEST_FAIL_OUTPUT_COMMIT", "before_publish")
        .output()
        .unwrap();
    assert!(!failure.status.success());
    assert!(!unpublished.exists());
    let message = stderr(&failure);
    assert!(
        message.find(WARNING).unwrap()
            < message.find("Injected output transaction failure").unwrap(),
        "{message}",
    );
}

#[test]
fn python_path_diagnostic_respects_relative_dry_run_and_no_pyi_boundary() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }
    let fixture = Fixture::new();
    let output = fixture.output("boundary-\u{1f642}", 135);
    let relative = output.strip_prefix(&fixture.0).unwrap();
    let dry_run = device_command(relative)
        .current_dir(&fixture.0)
        .arg("--dry-run")
        .output()
        .unwrap();
    successful(&dry_run);
    assert!(!output.exists());
    let message = stderr(&dry_run);
    assert_eq!(message.matches(WARNING).count(), 1, "{message}");
    assert!(message.contains("260 UTF-16 code units"));
    assert!(message.contains(&output.display().to_string()));
    let runtime_dry_run = device_command(relative)
        .current_dir(&fixture.0)
        .args(["--dry-run", "--no-pyi"])
        .output()
        .unwrap();
    successful(&runtime_dry_run);
    assert!(!stderr(&runtime_dry_run).contains(WARNING));
    assert!(!output.exists());

    let extended = PathBuf::from(format!(r"\\?\{}", output.display()));
    let typed = device_command(&extended).output().unwrap();
    successful(&typed);
    assert!(stderr(&typed).contains("260 UTF-16 code units"));
    assert!(!stderr(&typed).contains(r"\\?\"));
    let typed_files = files(&output);
    let runtime = device_command(&output).arg("--no-pyi").output().unwrap();
    successful(&runtime);
    assert!(!stderr(&runtime).contains(WARNING));
    let runtime_files = files(&output);
    assert!(
        runtime_files
            .keys()
            .all(|path| path.extension().is_none_or(|ext| ext != "pyi"))
    );
    assert!(!output.join("py.typed").exists());
    for (path, content) in typed_files
        .iter()
        .filter(|(path, _)| path.extension().is_some_and(|ext| ext == "py"))
    {
        assert_eq!(runtime_files.get(path), Some(content));
    }
}

#[test]
fn python_path_diagnostic_precedes_namespace_completion() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }
    let fixture = Fixture::new();
    let output = fixture.output("namespace", 220);
    let log_path = fixture.0.join("combined.log");
    let log = fs::File::create(&log_path).unwrap();
    let status = command(&output)
        .args(["--namespace", "Windows.Security.Cryptography"])
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .status()
        .unwrap();
    let log = fs::read_to_string(log_path).unwrap();
    assert!(status.success(), "{log}");
    assert_eq!(log.matches(WARNING).count(), 1, "{log}");
    assert!(
        log.find(WARNING).unwrap() < log.find("Done.").unwrap(),
        "{log}"
    );
    assert!(!files(&output).is_empty());
}
