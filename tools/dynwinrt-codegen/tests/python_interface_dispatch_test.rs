// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod common;

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use dynwinrt_codegen::meta;
use dynwinrt_codegen::types::TypeMeta;

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("target")
            .join(format!(
                "pid{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn generate_real_class(namespace: &str, name: &str) -> Option<String> {
    let class = meta::parse_class(WINDOWS_WINMD, namespace, name)?;
    let deps = meta::resolve_python_dependencies(WINDOWS_WINMD, &[class.clone()], &[], &[]);
    let mut known = HashSet::new();
    known.insert(class.name.clone());
    known.extend(deps.classes.iter().map(|class| class.name.clone()));
    known.extend(
        deps.interfaces
            .iter()
            .map(|interface| interface.name.clone()),
    );
    known.extend(deps.enums.iter().filter_map(|typ| match typ {
        TypeMeta::Enum { name, .. } => Some(name.clone()),
        _ => None,
    }));
    Some(common::generate_class(
        &class,
        &known,
        &HashSet::new(),
        &HashSet::new(),
    ))
}

/// Return the `if _bound ...` guard lines of the generated block starting at `marker`.
fn guard_lines<'a>(code: &'a str, marker: &str) -> Vec<&'a str> {
    let start = code
        .find(marker)
        .unwrap_or_else(|| panic!("missing `{marker}` in:\n{code}"));
    let block = &code[start + marker.len()..];
    let end = block.find("\n    def ").unwrap_or(block.len());
    block[..end]
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("if _bound is not None"))
        .collect()
}

#[test]
fn data_stream_constructors_accept_runtime_class_streams_through_query_interface() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }
    for (class, interface, module, iid) in [
        (
            "DataWriter",
            "IOutputStream",
            "windows__storage__streams__i_output_stream",
            "905a0fe6-bc53-11df-8c49-001e4fc686da",
        ),
        (
            "DataReader",
            "IInputStream",
            "windows__storage__streams__i_input_stream",
            "905a0fe2-bc53-11df-8c49-001e4fc686da",
        ),
    ] {
        let code = generate_real_class("Windows.Storage.Streams", class).expect("class metadata");
        let constant = format!("IID_ARG_Windows_Storage_Streams_{interface}");
        let exact = format!("isinstance(_bound[0], _dynwinrt_symbol('{module}', '{interface}'))");
        let relaxed = format!("({exact} or _dynwinrt_can_cast(_bound[0], {constant}))");

        assert!(
            code.contains(&format!("\n{constant} = WinGUID.parse('{iid}')\n")),
            "{code}"
        );
        for marker in [
            "    def __new__(cls, *args, **kwargs):\n",
            "    def __init__(self, *args, **kwargs):\n",
        ] {
            let guards = guard_lines(&code, marker);
            let first_relaxed = guards
                .iter()
                .position(|guard| guard.contains(&relaxed))
                .unwrap_or_else(|| panic!("{class} {marker} lacks a QI guard:\n{code}"));
            assert!(
                guards[..first_relaxed]
                    .iter()
                    .any(|guard| guard.contains(&format!("{exact}:"))),
                "{class} {marker} must keep its exact guard first:\n{code}"
            );
            assert!(
                guards[first_relaxed..]
                    .iter()
                    .all(|guard| guard.contains("_dynwinrt_can_cast(_bound[")),
                "{class} {marker} must try every exact guard before QueryInterface:\n{code}"
            );
        }
    }
}

fn identifiers(text: &str, prefix: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut rest = text;
    while let Some(index) = rest.find(prefix) {
        let preceded_by_identifier = rest[..index]
            .chars()
            .next_back()
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_');
        let tail = &rest[index..];
        let end = tail
            .find(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .unwrap_or(tail.len());
        if !preceded_by_identifier {
            found.insert(tail[..end].to_string());
        }
        rest = &tail[end..];
    }
    found
}

#[test]
fn generated_python_modules_define_every_argument_iid_they_reference() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }
    let fixture = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args(["generate", "--winmd", WINDOWS_WINMD, "--class-name"])
        .arg(
            "Windows.Storage.Streams.DataWriter,Windows.Storage.Streams.DataReader,\
             Windows.Storage.Streams.RandomAccessStream,Windows.Storage.StorageFile,\
             Windows.System.Launcher,Windows.Data.Xml.Dom.XmlDocument,\
             Windows.Web.Http.HttpClient,Windows.UI.Notifications.ToastNotifier",
        )
        .args(["--lang", "py", "--no-pyi", "--output"])
        .arg(&fixture.0)
        .output()
        .expect("run dynwinrt-codegen");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let mut modules = 0;
    let mut guarded_modules = 0;
    let mut pending = vec![fixture.0.clone()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.extension().and_then(|extension| extension.to_str()) != Some("py") {
                continue;
            }
            let code = fs::read_to_string(&path).unwrap();
            modules += 1;
            let defined = code
                .lines()
                .filter_map(|line| line.split_once(" = WinGUID.parse(").map(|(name, _)| name))
                .filter(|name| name.starts_with("IID_ARG_"))
                .map(str::to_string)
                .collect::<BTreeSet<_>>();
            let referenced = identifiers(&code, "IID_ARG_");
            let missing = referenced.difference(&defined).collect::<Vec<_>>();
            assert!(
                missing.is_empty(),
                "{} references undefined argument IIDs {missing:?}",
                path.display()
            );
            if code.contains("_dynwinrt_can_cast(_bound[") {
                guarded_modules += 1;
            }
        }
    }
    assert!(
        modules > 20,
        "expected a generated package, found {modules} modules"
    );
    assert!(
        guarded_modules > 0,
        "expected QueryInterface dispatch guards in the generated package"
    );
}
