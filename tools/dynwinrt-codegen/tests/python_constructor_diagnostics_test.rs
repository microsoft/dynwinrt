// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

struct Generated(PathBuf);

impl Drop for Generated {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove generated constructor test package");
    }
}

fn assert_success(output: Output) -> String {
    assert!(
        output.status.success(),
        "exit {:?}\n{}\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn generated_constructor_errors_preserve_valid_calls_and_native_failures() {
    let winmd = std::env::var_os("DYNWINRT_WINDOWS_WINMD")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(WINDOWS_WINMD));
    if !winmd.is_file() {
        eprintln!("Skipping constructor diagnostics: Windows.winmd not found");
        return;
    }
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join("target"));
    let package = format!("constructor_diagnostics_{}", std::process::id());
    let generated = Generated(target.join(&package));
    assert_success(
        Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
            .args(["generate", "--winmd"])
            .arg(winmd)
            .args([
                "--class-name",
                "Windows.Foundation.Uri,Windows.Storage.Streams.Buffer,Windows.Storage.Streams.DataWriter,Windows.Storage.Streams.InMemoryRandomAccessStream",
                "--lang",
                "py",
                "--output",
            ])
            .arg(&generated.0)
            .output()
            .expect("generate constructor test package"),
    );
    let uri = fs::read_to_string(generated.0.join("windows__foundation__uri.py")).unwrap();
    let stub = fs::read_to_string(generated.0.join("windows__foundation__uri.pyi")).unwrap();
    for params in ["uri: str", "base_uri: str, relative_uri: str"] {
        assert!(uri.contains(&format!("Uri({params})")), "{uri}");
        assert!(
            stub.contains(&format!("def __init__(self, {params}) -> None: ...")),
            "{stub}"
        );
    }

    let python = std::env::var_os("DYNWINRT_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let venv = repo.join(r"bindings\py\.venv\Scripts\python.exe");
            if venv.is_file() {
                venv
            } else {
                PathBuf::from("python")
            }
        });
    let available = Command::new(&python)
        .args([
            "-c",
            "from dynwinrt import RoApartment, projected_lifetime_scope, DynWinRTImplementationHandle",
        ])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref() != Ok("1"),
        "constructor diagnostics require the matching Python binding"
    );
    if !available {
        eprintln!("Skipping native constructor diagnostics: matching binding not installed");
        return;
    }

    let script = r#"
import inspect
import sys
from dynwinrt import RoApartment, projected_lifetime_scope
from PY_PACKAGE.windows.foundation import Uri
from PY_PACKAGE.windows.storage.streams import Buffer, DataWriter, InMemoryRandomAccessStream

uri_signatures = "Uri(uri: str); Uri(base_uri: str, relative_uri: str)"

def mismatch(call, received, signatures=uri_signatures):
    try:
        call()
    except TypeError as error:
        assert type(error) is TypeError
        expected = f"No matching constructor for {received}. Expected one of: {signatures}"
        assert str(error) == expected, str(error)
    else:
        raise AssertionError(f"accepted invalid call: {received}")

class NoRepr:
    def __repr__(self):
        raise AssertionError("constructor diagnostic must not format argument values")

    __str__ = __repr__

class DerivedUri(Uri):
    pass

class NoTypeName(type):
    @property
    def __name__(cls):
        raise AssertionError("constructor diagnostic must not call metaclass hooks")

class CustomType(metaclass=NoTypeName):
    pass

class NoFormat(str):
    def __format__(self, spec):
        raise AssertionError("constructor diagnostic must not format keyword objects")

with RoApartment(), projected_lifetime_scope():
    modules_before = set(sys.modules)
    mismatch(lambda: Uri(url="https://private.invalid/secret"), "Uri(url=str)")
    mismatch(lambda: Uri(42), "Uri(int)")
    assert set(sys.modules) == modules_before, "diagnostics must not import more modules"
    mismatch(lambda: Uri(), "Uri()")
    mismatch(lambda: Uri("a", "b", "c"), "Uri(str, str, str)")
    mismatch(lambda: Uri(uri=42), "Uri(uri=int)")
    mismatch(lambda: Uri("a", uri="b"), "Uri(str, uri=str)")
    mismatch(lambda: Uri(base_uri="a"), "Uri(base_uri=str)")
    mismatch(lambda: Uri(uri="a", relative_uri="b"), "Uri(uri=str, relative_uri=str)")
    mismatch(lambda: Uri(NoRepr()), "Uri(NoRepr)")
    mismatch(lambda: Uri(url=NoRepr()), "Uri(url=NoRepr)")
    mismatch(lambda: Uri(CustomType()), "Uri(CustomType)")
    mismatch(lambda: Uri(**{NoFormat("url"): CustomType()}), "Uri(url=CustomType)")
    mismatch(lambda: DerivedUri(42), "Uri(int)")

    base = "https://example.com/"
    for create in (
        lambda: Uri(base + "child"),
        lambda: Uri(uri=base + "child"),
        lambda: Uri(base, "child"),
        lambda: Uri(base_uri=base, relative_uri="child"),
        lambda: Uri(base, relative_uri="child"),
        lambda: Uri(relative_uri="child", base_uri=base),
        lambda: Uri.create_uri(uri=base + "child"),
        lambda: Uri.create_with_relative_uri(base_uri=base, relative_uri="child"),
        lambda: DerivedUri(base, relative_uri="child"),
    ):
        uri = create()
        assert uri.absolute_uri == base + "child"
    assert str(inspect.signature(Uri)) == "(*args, **kwargs)"
    assert Uri(uri._obj).absolute_uri == base + "child"

    for create in (lambda: Buffer(3), lambda: Buffer(capacity=3)):
        assert create().capacity == 3
    mismatch(lambda: Buffer(2**32), "Buffer(int)", "Buffer(capacity: int)")
    mismatch(lambda: Buffer(True), "Buffer(bool)", "Buffer(capacity: int)")

    writer_signatures = "DataWriter(); DataWriter(output_stream: 'IOutputStream')"
    mismatch(lambda: DataWriter(42), "DataWriter(int)", writer_signatures)
    mismatch(lambda: DataWriter(stream=42), "DataWriter(stream=int)", writer_signatures)
    with DataWriter() as writer:
        writer.write_byte(65)
        assert writer.unstored_buffer_length == 1
    for keyword in (False, True):
        with InMemoryRandomAccessStream() as stream:
            writer = DataWriter(output_stream=stream) if keyword else DataWriter(stream)
            with writer:
                writer.write_byte(66)
                assert writer.unstored_buffer_length == 1

    try:
        Uri("not a valid absolute URI")
    except OSError as error:
        assert error.winerror == -2147024809
        assert "No matching constructor" not in str(error)
    else:
        raise AssertionError("native activation error was swallowed")

print("constructor-diagnostics-native-ok")
"#;
    let output = assert_success(
        Command::new(python)
            .args(["-B", "-c", &script.replace("PY_PACKAGE", &package)])
            .env("PYTHONPATH", &target)
            .output()
            .expect("run native constructor diagnostics"),
    );
    assert!(output.contains("constructor-diagnostics-native-ok"));
}
