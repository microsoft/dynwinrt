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
        Self::for_class("Windows.Foundation.PropertyValue", "raw_lifetime")
    }

    fn for_class(class_name: &str, prefix: &str) -> Option<Self> {
        if !Path::new(WINDOWS_WINMD).is_file() {
            eprintln!("Skipping raw lifetime regression: Windows.winmd not found");
            return None;
        }
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let package = format!("{prefix}_{}", std::process::id());
        let root = repo.join("target").join(&package);
        let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
            .args([
                "generate",
                "--winmd",
                WINDOWS_WINMD,
                "--class-name",
                class_name,
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

    fn binding_available(&self) -> bool {
        Command::new(self.python())
            .args([
                "-c",
                "from dynwinrt import DynWinRTImplementationHandle, RoApartment",
            ])
            .output()
            .is_ok_and(|output| output.status.success())
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
    let available = generated.binding_available();
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

#[test]
fn unscoped_generated_uri_cannot_outlive_its_managed_apartment() {
    let Some(generated) = Generated::for_class("Windows.Foundation.Uri", "uri_lifetime") else {
        return;
    };
    let available = generated.binding_available();
    assert!(
        available || std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref() != Ok("1"),
        "generated Uri lifetime regression requires the matching Python binding"
    );
    if !available {
        eprintln!("Skipping generated Uri lifetime regression: Python binding not installed");
        return;
    }

    generated.run(
        "unscoped-uri-del",
        r#"
from dynwinrt import RoApartment
from PY_PACKAGE.windows.foundation import Uri
with RoApartment(1):
    live = Uri('https://example.com/c')
    assert live.host == 'example.com'
assert live._obj.is_released()
try:
    live.host
except RuntimeError as error:
    assert 'released' in str(error)
else:
    raise AssertionError('unscoped Uri remained callable after apartment exit')
del live
print('unscoped-uri-del', flush=True)
"#,
    );
    generated.run(
        "unscoped-uri-shutdown",
        r#"
from dynwinrt import RoApartment
from PY_PACKAGE.windows.foundation import Uri
with RoApartment(1):
    live = Uri('https://example.com/c')
    assert live.host == 'example.com'
assert live._obj.is_released()
print('unscoped-uri-shutdown', flush=True)
# Keep live through interpreter shutdown, without an explicit lifetime scope.
"#,
    );
    generated.run(
        "sequential-apartment-statics",
        r#"
import threading
from dynwinrt import RoApartment
from PY_PACKAGE.windows.foundation import Uri

errors = []
def use_uri(index):
    try:
        with RoApartment(1):
            factory = Uri._get_s_IUriEscapeStatics()
            assert Uri.escape_component('hello world') == 'hello%20world'
            uri = Uri(f'https://example.com/{index}')
            assert uri.host == 'example.com'
        assert factory.is_released() and uri._obj.is_released()
    except BaseException as error:
        errors.append(error)

for index in range(3):
    worker = threading.Thread(target=use_uri, args=(index,))
    worker.start()
    worker.join(10)
    assert not worker.is_alive()
if errors:
    raise errors[0]
print('sequential-apartment-statics', flush=True)
"#,
    );
}

#[test]
fn nonagile_generated_async_completes_before_unscoped_apartment_teardown() {
    let Some(generated) = Generated::for_class(
        "Windows.Devices.Enumeration.DeviceInformation",
        "device_lifetime",
    ) else {
        return;
    };
    if !generated.binding_available() {
        assert_ne!(
            std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref(),
            Ok("1"),
            "non-agile async lifetime regression requires the matching Python binding"
        );
        return;
    }
    generated.run(
        "nonagile-async-owner-thread",
        r#"
import asyncio
from dynwinrt import RoApartment
from PY_PACKAGE.windows.devices.enumeration import DeviceInformation

async def query():
    with RoApartment(1):
        operation = DeviceInformation.find_all_async()
        devices = await operation
        assert isinstance(devices.size, int)
    assert devices._obj.is_released()
    try:
        devices.size
    except RuntimeError as error:
        assert 'released' in str(error)
    else:
        raise AssertionError('non-agile result outlived its COM apartment')

asyncio.run(query())
print('nonagile-async-owner-thread', flush=True)
"#,
    );
}

#[test]
fn generated_threadpool_async_close_retries_without_cancelling_work() {
    let Some(generated) =
        Generated::for_class("Windows.System.Threading.ThreadPool", "threadpool_lifetime")
    else {
        return;
    };
    if !generated.binding_available() {
        assert_ne!(
            std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref(),
            Ok("1"),
            "ThreadPool apartment lifetime regression requires the matching Python binding"
        );
        return;
    }
    generated.run(
        "pending-async-retry",
        r#"
import asyncio
import threading
from dynwinrt import RoApartment
from PY_PACKAGE.windows.system.threading import ThreadPool

started = threading.Event()
release = threading.Event()
finished = threading.Event()

def work(_action):
    started.set()
    try:
        assert release.wait(8), 'work item was not unblocked'
    finally:
        finished.set()

async def run():
    with RoApartment(1) as apartment:
        operation = ThreadPool.run_async(work)
        task = asyncio.create_task(operation)
        assert await asyncio.to_thread(started.wait, 5)
        await asyncio.sleep(0)
        assert not task.done()
        try:
            apartment.close()
        except RuntimeError as error:
            assert 'future is pending' in str(error)
        else:
            raise AssertionError('pending async operation closed its apartment')
        release.set()
        await task
        assert not task.cancelled() and finished.is_set()
        apartment.close()
    try:
        operation.wait()
    except RuntimeError as error:
        assert 'released' in str(error)
    else:
        raise AssertionError('async operation outlived its apartment')

asyncio.run(run())
print('pending-async-retry', flush=True)
"#,
    );
    generated.run(
        "agile-pending-work",
        r#"
import threading
from dynwinrt import RoApartment
from PY_PACKAGE.windows.system.threading import ThreadPool

started = threading.Event()
release = threading.Event()
finished = threading.Event()

def work(_action):
    started.set()
    try:
        assert release.wait(8), 'work item was not unblocked'
    finally:
        finished.set()

with RoApartment(1):
    operation = ThreadPool.run_async(work)
    assert started.wait(5)
release.set()
assert finished.wait(5), 'agile work was cancelled when its Python owner exited'
try:
    operation.wait()
except RuntimeError as error:
    assert 'released' in str(error)
else:
    raise AssertionError('async owner outlived its apartment')
print('agile-pending-work', flush=True)
"#,
    );
    generated.run(
        "scoped-pending-async-retry",
        r#"
import asyncio
import threading
from dynwinrt import RoApartment, projected_lifetime_scope
from PY_PACKAGE.windows.system.threading import ThreadPool

started = threading.Event()
release = threading.Event()

def work(_action):
    started.set()
    assert release.wait(8), 'work item was not unblocked'

async def run():
    with RoApartment(1), projected_lifetime_scope() as scope:
        operation = ThreadPool.run_async(work)
        task = asyncio.create_task(operation)
        assert await asyncio.to_thread(started.wait, 5)
        await asyncio.sleep(0)
        assert not task.done()
        try:
            scope.close()
        except RuntimeError as error:
            assert 'future is pending' in str(error)
        else:
            raise AssertionError('scope disposed a pending async owner')
        assert not task.cancelled()
        release.set()
        await task
        scope.close()
    assert scope.disposed and not task.cancelled()
    try:
        operation.wait()
    except RuntimeError as error:
        assert 'released' in str(error)
    else:
        raise AssertionError('async owner remained live after its scope')

asyncio.run(run())
print('scoped-pending-async-retry', flush=True)
"#,
    );
}
