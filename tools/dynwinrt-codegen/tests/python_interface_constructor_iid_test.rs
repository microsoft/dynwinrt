// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Generated {
    root: PathBuf,
    package: String,
}

impl Generated {
    fn new() -> Option<Self> {
        if !Path::new(WINDOWS_WINMD).is_file() {
            eprintln!("Skipping interface constructor regression: Windows.winmd not found");
            return None;
        }
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let package = format!(
            "checked_interface_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let root = repo.join("target").join(&package);
        let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
            .args([
                "generate",
                "--winmd",
                WINDOWS_WINMD,
                "--class-name",
                "Windows.Foundation.Uri,Windows.Storage.Streams.Buffer",
                "--lang",
                "py",
                "--output",
            ])
            .arg(&root)
            .output()
            .expect("generate Uri and Buffer bindings");
        assert_success(output);
        Some(Self { root, package })
    }

    fn python(&self) -> PathBuf {
        std::env::var_os("DYNWINRT_TEST_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let repo = self.root.parent().unwrap().parent().unwrap();
                let venv = repo.join(r"bindings\py\.venv\Scripts\python.exe");
                if venv.is_file() {
                    venv
                } else {
                    PathBuf::from("python")
                }
            })
    }

    fn module(&self, name: &str) -> String {
        fs::read_to_string(self.root.join(name)).expect(name)
    }

    fn run(&self, script: &str) -> Output {
        Command::new(self.python())
            .args(["-B", "-c", &script.replace("PY_PACKAGE", &self.package)])
            .env("PYTHONPATH", self.root.parent().unwrap())
            .output()
            .expect("run isolated constructor consumer")
    }
}

impl Drop for Generated {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn assert_success(output: Output) {
    assert!(
        output.status.success(),
        "exit {:?}\n{}\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn generated_interface_constructor_validates_before_retaining_or_caching() {
    let Some(generated) = Generated::new() else {
        return;
    };
    let buffer = generated.module("windows__storage__streams__i_buffer.py");
    let initializer = buffer
        .split("def _set_native(self, obj: DynWinRTValue, *, cache=True):\n")
        .nth(1)
        .expect("IBuffer native initializer")
        .split("    def __init__")
        .next()
        .unwrap();
    assert!(
        initializer.contains("self._obj = obj.cast(IID_IBuffer)")
            && !initializer.contains("self._obj = obj\n"),
        "{initializer}"
    );
    let validated = initializer
        .find("self._obj = obj.cast(IID_IBuffer)")
        .unwrap();
    let cached = initializer.find("_dynwinrt_cache_projected(self)").unwrap();
    assert!(validated < cached, "{initializer}");
    assert!(
        buffer.contains(
            "return _dynwinrt_projected_from_native(cls, args[0], '_set_native', release_redundant=False)"
        ) && buffer.contains("return cls._from_native(obj.cast(IID_IBuffer))"),
        "{buffer}"
    );
    let support = generated.module("_runtime.py");
    assert!(
        support.contains("_dynwinrt_projected_from_native"),
        "{support}"
    );
}

#[test]
fn real_winrt_rejects_incompatible_direct_and_indirect_interface_projection() {
    let Some(generated) = Generated::new() else {
        return;
    };
    let available = Command::new(generated.python())
        .args([
            "-c",
            "from dynwinrt import DynWinRTImplementationHandle, RoApartment",
        ])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref() != Ok("1"),
        "interface constructor regression requires the matching Python binding"
    );
    if !available {
        eprintln!("Skipping constructor native regression: matching binding not installed");
        return;
    }
    let script = r#"
from dynwinrt import DynWinRTValue, RoApartment, projected_lifetime_scope
from PY_PACKAGE.windows.foundation import Uri
from PY_PACKAGE.windows.storage.streams import Buffer, IBuffer

with RoApartment(), projected_lifetime_scope():
    uri = Uri('https://example.com/unsafe')
    identity = uri._obj.identity_raw()
    for construct in (
        lambda: IBuffer(uri._obj),
        lambda: IBuffer.__new__(IBuffer, uri._obj),
        lambda: IBuffer._from_native(uri._obj),
        lambda: IBuffer.from_value(uri._obj),
        lambda: uri.as_interface(IBuffer),
    ):
        try:
            construct()
        except OSError as error:
            assert error.winerror == -2147467262, error  # E_NOINTERFACE
        else:
            raise AssertionError('IBuffer accepted a native Uri pointer')
        assert uri._obj.identity_raw() == identity
        assert not uri._obj.is_released()

    uninitialized = object.__new__(IBuffer)
    try:
        IBuffer._set_native(uninitialized, uri._obj)
    except OSError as error:
        assert error.winerror == -2147467262
    else:
        raise AssertionError('_set_native stored a native Uri pointer')
    assert not hasattr(uninitialized, '_obj')

    source = DynWinRTValue.from_bytes(b'owned buffer')
    first = IBuffer(source)
    assert first._obj is not source
    assert first._obj.identity_raw() == source.identity_raw()
    assert first.to_bytes() == b'owned buffer'
    assert IBuffer(source) is first
    assert IBuffer.from_value(source) is first
    assert first.as_interface(IBuffer) is first
    assert not source.is_released() and not first._obj.is_released()

    class TaggedBuffer(IBuffer):
        def tag(self):
            return 'tagged'

    tagged = TaggedBuffer(source)
    assert tagged.tag() == 'tagged'
    assert TaggedBuffer.from_value(source) is tagged
    assert tagged.as_interface(TaggedBuffer) is tagged
    assert tagged._obj.identity_raw() == first._obj.identity_raw()

    source.release()
    assert first.to_bytes() == tagged.to_bytes() == b'owned buffer'
    projected = Buffer.from_bytes(b'projected buffer')
    view = projected.as_interface(IBuffer)
    assert projected.to_bytes() == view.to_bytes() == b'projected buffer'
    assert projected._obj.identity_raw() == view._obj.identity_raw()
print('checked-constructor-native-ok', flush=True)
"#;
    let output = generated.run(script);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_success(output);
    assert!(stdout.contains("checked-constructor-native-ok"));
}

const VALID_CONSUMER: &str = r#"
from typing import assert_type
from dynwinrt import DynWinRTValue
from PY_PACKAGE.windows.foundation import Uri
from PY_PACKAGE.windows.storage.streams import Buffer, IBuffer

class TaggedBuffer(IBuffer):
    def tag(self) -> str:
        return 'tagged'

def project(raw: DynWinRTValue, buffer: Buffer, uri: Uri) -> None:
    assert_type(IBuffer.from_value(raw), IBuffer)
    assert_type(IBuffer.from_bytes(b'data'), IBuffer)
    assert_type(buffer.as_interface(IBuffer), IBuffer)
    assert_type(TaggedBuffer.from_value(raw), TaggedBuffer)
    # Only the native QueryInterface can decide whether this raw value is an IBuffer.
    assert_type(IBuffer.from_value(uri._obj), IBuffer)
"#;

const INVALID_CONSUMER: &str = r#"
from PY_PACKAGE.windows.foundation import Uri
from PY_PACKAGE.windows.storage.streams import IBuffer

def misuse(uri: Uri) -> None:
    IBuffer.from_value(uri)
    uri.as_interface(Uri)
    IBuffer(None)
"#;

#[test]
fn interface_projection_typing_keeps_raw_values_explicit() {
    let Some(generated) = Generated::new() else {
        return;
    };
    let available = Command::new(generated.python())
        .args(["-m", "mypy", "--version"])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || std::env::var("DYNWINRT_REQUIRE_MYPY").as_deref() != Ok("1"),
        "strict interface constructor typing requires mypy"
    );
    if !available {
        eprintln!("Skipping constructor typing: mypy not installed");
        return;
    }
    let repo = generated.root.parent().unwrap().parent().unwrap();
    for (source, errors) in [(VALID_CONSUMER, 0), (INVALID_CONSUMER, 4)] {
        let source = source.replace("PY_PACKAGE", &generated.package);
        let mut command = Command::new(generated.python());
        command.args([
            "-m",
            "mypy",
            "--strict",
            "--no-incremental",
            "--no-pretty",
            "--cache-dir",
        ]);
        command.arg(generated.root.join("mypy-cache"));
        command
            .args(["-c", &source])
            .current_dir(generated.root.parent().unwrap());
        command.env(
            "MYPYPATH",
            std::env::join_paths([repo.join(r"bindings\py"), repo.join("target")])
                .expect("MYPYPATH"),
        );
        let output = command.output().expect("run mypy");
        let diagnostics = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            diagnostics.matches(": error:").count(),
            errors,
            "{diagnostics}"
        );
        assert_eq!(output.status.success(), errors == 0, "{diagnostics}");
    }
    if let Some(pyright) = std::env::var_os("DYNWINRT_PYRIGHT") {
        for (file, source, errors) in [
            ("valid.py", VALID_CONSUMER, 0),
            ("invalid.py", INVALID_CONSUMER, 3),
        ] {
            let path = generated.root.join(file);
            fs::write(
                &path,
                format!(
                    "# pyright: strict\n{}",
                    source.replace("PY_PACKAGE", &generated.package)
                ),
            )
            .unwrap();
            let output = Command::new(&pyright)
                .args(["--pythonpath"])
                .arg(generated.python())
                .arg(&path)
                .env("PYTHONPATH", generated.root.parent().unwrap())
                .output()
                .expect("run pyright");
            let diagnostics = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if errors == 0 {
                assert!(!diagnostics.contains(" - error: "), "{diagnostics}");
            } else {
                for line in [7, 8, 9] {
                    assert!(
                        diagnostics
                            .lines()
                            .any(|entry| entry.contains(&format!("invalid.py:{line}:"))
                                && entry.contains(" - error: ")),
                        "{diagnostics}"
                    );
                }
            }
            assert_eq!(output.status.success(), errors == 0, "{diagnostics}");
        }
    }
}
