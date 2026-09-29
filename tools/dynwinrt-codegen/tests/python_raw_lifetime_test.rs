// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

struct Generated {
    root: PathBuf,
    package: String,
}

impl Generated {
    fn new() -> Option<Self> {
        if !Path::new(WINDOWS_WINMD).is_file() {
            eprintln!("Skipping raw lifetime regression: Windows.winmd not found");
            return None;
        }
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let package = format!("raw_lifetime_{}", std::process::id());
        let root = repo.join("target").join(&package);
        let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
            .args([
                "generate",
                "--winmd",
                WINDOWS_WINMD,
                "--class-name",
                "Windows.Foundation.PropertyValue",
                "--lang",
                "py",
                "--output",
            ])
            .arg(&root)
            .output()
            .expect("generate PropertyValue bindings");
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
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

    fn run(&self, scenario: &str, script: &str) {
        let output = Command::new(self.python())
            .args(["-B", "-c", &script.replace("PY_PACKAGE", &self.package)])
            .env("PYTHONPATH", self.root.parent().unwrap())
            .output()
            .expect("run isolated Python lifetime regression");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{scenario}: Python exited {:?}\n{stdout}\n{stderr}",
            output.status.code()
        );
        assert!(stdout.contains(scenario), "{scenario}:\n{stdout}\n{stderr}");
    }
}

impl Drop for Generated {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn generated_raw_outputs_release_before_apartment_exit_even_when_they_escape() {
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
        "raw lifetime regression requires the matching Python binding"
    );
    if !available {
        eprintln!("Skipping raw lifetime regression: matching Python binding not installed");
        return;
    }

    generated.run(
        "unscoped-explicit-release",
        r#"
from dynwinrt import RoApartment
from PY_PACKAGE.windows.foundation import PropertyValue
with RoApartment():
    raw = PropertyValue.create_uint32(8080)
    assert not raw.is_released()
    raw.release()
assert raw.is_released()
print('unscoped-explicit-release', flush=True)
"#,
    );
    generated.run(
        "explicit-release-control",
        r#"
from dynwinrt import RoApartment, projected_lifetime_scope
from PY_PACKAGE.windows.foundation import PropertyValue
with RoApartment(), projected_lifetime_scope() as scope:
    raw = PropertyValue.create_uint32(8080)
    assert not raw.is_released()
    raw.release()
    assert raw.is_released()
assert scope.disposed and raw.is_released()
print('explicit-release-control', flush=True)
"#,
    );
    generated.run(
        "escaped-raw-shutdown",
        r#"
from dynwinrt import RoApartment, WinGUID, projected_lifetime_scope
from PY_PACKAGE.windows.foundation import PropertyValue
def escaped():
    with RoApartment(), projected_lifetime_scope() as scope:
        raw = PropertyValue.create_uint32(8080)
        assert not raw.is_released()
        return raw, scope
raw, scope = escaped()
assert scope.disposed and raw.is_released()
try:
    raw.cast(WinGUID.parse('00000000-0000-0000-c000-000000000046'))
except RuntimeError as error:
    assert 'released' in str(error)
else:
    raise AssertionError('escaped native value remained callable')
print('escaped-raw-shutdown', flush=True)
# Keep raw alive through interpreter shutdown, past RoApartment.__exit__.
"#,
    );
    generated.run(
        "direct-native-outputs",
        r#"
from dynwinrt import DynWinRTValue, RoApartment, projected_lifetime_scope
from PY_PACKAGE.windows__foundation__property_value import (
    IID_IPropertyValueStatics, _IPropertyValueStatics,
)
with RoApartment(), projected_lifetime_scope():
    scalar = DynWinRTValue.from_u32(8080)
    factory = DynWinRTValue.activation_factory(
        'Windows.Foundation.PropertyValue'
    ).cast(IID_IPropertyValueStatics)
    raw = _IPropertyValueStatics.method(11).invoke(
        factory, [DynWinRTValue.from_u32(8080)]
    )
    outputs = _IPropertyValueStatics.method(11).invoke_all(
        factory, [DynWinRTValue.from_u32(8080)]
    )
    assert len(outputs) == 1
    assert not raw.is_released() and not outputs[0].is_released()
assert factory.is_released() and raw.is_released() and outputs[0].is_released()
assert not scalar.is_released() and scalar.to_u32() == 8080
print('direct-native-outputs', flush=True)
"#,
    );
    generated.run(
        "borrowed-source-retains-ownership",
        r#"
from dynwinrt import DynWinRTValue, RoApartment, projected_lifetime_scope
from PY_PACKAGE.windows__foundation__property_value import IID_IPropertyValueStatics
with RoApartment():
    source = DynWinRTValue.activation_factory('Windows.Foundation.PropertyValue')
    with projected_lifetime_scope():
        view = source.cast(IID_IPropertyValueStatics)
        assert not source.is_released() and not view.is_released()
    assert view.is_released() and not source.is_released()
    independent = source.cast(IID_IPropertyValueStatics)
    independent.release()
    source.release()
assert source.is_released()
print('borrowed-source-retains-ownership', flush=True)
"#,
    );
    generated.run(
        "original-error-preserved",
        r#"
from dynwinrt import RoApartment, projected_lifetime_scope
from PY_PACKAGE.windows.foundation import PropertyValue
try:
    with RoApartment(), projected_lifetime_scope():
        raw = PropertyValue.create_uint32(8080)
        raise ValueError('original failure')
except ValueError as error:
    assert str(error) == 'original failure'
else:
    raise AssertionError('scope suppressed the original failure')
assert raw.is_released()
print('original-error-preserved', flush=True)
"#,
    );
    generated.run(
        "nested-and-callback-scopes",
        r#"
from concurrent.futures import ThreadPoolExecutor
from dynwinrt import RoApartment, projected_lifetime_scope
from dynwinrt.dynwinrt import _dynwinrt_wrap_delegate_callback
from PY_PACKAGE.windows.foundation import PropertyValue

with RoApartment(), projected_lifetime_scope():
    callback = _dynwinrt_wrap_delegate_callback(
        lambda: PropertyValue.create_uint32(1)
    )
    same_thread = callback()
    with projected_lifetime_scope():
        nested = PropertyValue.create_uint32(2)
    assert nested.is_released() and not same_thread.is_released()

    def foreign_thread():
        with RoApartment():
            raw = callback()
            assert not raw.is_released()
            raw.release()
            return raw.is_released()

    with ThreadPoolExecutor(max_workers=1) as executor:
        assert executor.submit(foreign_thread).result()
    assert not same_thread.is_released()
assert same_thread.is_released()
print('nested-and-callback-scopes', flush=True)
"#,
    );
    generated.run(
        "foreign-scope-rejected",
        r#"
from concurrent.futures import ThreadPoolExecutor
from contextvars import copy_context
from dynwinrt import RoApartment, projected_lifetime_scope
from PY_PACKAGE.windows.foundation import PropertyValue

with RoApartment(), projected_lifetime_scope() as scope:
    inherited = copy_context()

    def foreign_thread():
        with RoApartment():
            return inherited.run(lambda: PropertyValue.create_uint32(8080))

    with ThreadPoolExecutor(max_workers=1) as executor:
        try:
            executor.submit(foreign_thread).result()
        except RuntimeError as error:
            assert 'different thread' in str(error)
        else:
            raise AssertionError('foreign output entered the owner thread scope')
    assert not scope.disposed
assert scope.disposed
print('foreign-scope-rejected', flush=True)
"#,
    );
}
