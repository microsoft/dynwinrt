// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python delegate callbacks derive their annotation and argument projection
//! from the delegate `Invoke` signature, wherever a callable becomes a delegate.

mod common;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::{FieldMeta, TypeMeta};
use windows_metadata::{
    MethodAttributes, MethodCallAttributes, MethodImplAttributes, ParamAttributes, Signature, Type,
    TypeAttributes, Value, writer,
};

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Output(PathBuf);

impl Output {
    fn generate(classes: &str) -> Option<Self> {
        let winmd = Path::new(WINDOWS_WINMD);
        if !winmd.is_file() {
            eprintln!("Skipping Windows.winmd delegate checks: SDK metadata unavailable.");
            return None;
        }
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("target")
            .join(format!(
                "dc{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
            .args(["generate", "--winmd"])
            .arg(winmd)
            .args(["--class-name", classes, "--lang", "py", "--output"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Some(Self(path))
    }

    fn read(&self, module: &str, extension: &str) -> String {
        let file = format!("{module}.{extension}");
        fs::read_to_string(self.0.join(&file)).unwrap_or_else(|error| panic!("{file}: {error}"))
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn class_wrapper(module: &str, class: &str, argument: &str) -> String {
    format!(
        "(lambda value: None if value.is_null() else \
         _dynwinrt_symbol('{module}', '{class}')._from_native(value))({argument})"
    )
}

fn guid(file: &mut writer::File, definition: writer::TypeDef, id: u32) {
    let attribute = file.TypeRef("Windows.Foundation.Metadata", "GuidAttribute");
    let constructor = file.MemberRef(
        ".ctor",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::Void,
            types: vec![
                Type::U32,
                Type::U16,
                Type::U16,
                Type::U8,
                Type::U8,
                Type::U8,
                Type::U8,
                Type::U8,
                Type::U8,
                Type::U8,
                Type::U8,
            ],
        },
        writer::MemberRefParent::TypeRef(attribute),
    );
    let values = [
        Value::U32(id),
        Value::U16(0x6281),
        Value::U16(0x4900),
        Value::U8(0xb7),
        Value::U8(0x82),
        Value::U8(4),
        Value::U8(3),
        Value::U8(2),
        Value::U8(1),
        Value::U8(9),
        Value::U8(0x10),
    ]
    .into_iter()
    .map(|value| (String::new(), value))
    .collect::<Vec<_>>();
    file.Attribute(
        writer::HasAttribute::TypeDef(definition),
        writer::AttributeType::MemberRef(constructor),
        &values,
    );
}

fn write_nested_delegate_metadata(path: &Path) {
    let mut file = writer::File::new("NestedDelegateCallbacks");
    let base = file.TypeRef("System", "MulticastDelegate");
    for (name, id, argument) in [
        ("InnerHandler", 0x31d447a1, None),
        (
            "OuterHandler",
            0x31d447a2,
            Some(Type::named("Audit", "InnerHandler")),
        ),
    ] {
        let definition = file.TypeDef(
            "Audit",
            name,
            writer::TypeDefOrRef::TypeRef(base),
            TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
        );
        guid(&mut file, definition, id);
        file.MethodDef(
            ".ctor",
            &Signature {
                flags: MethodCallAttributes::HASTHIS,
                return_type: Type::Void,
                types: vec![],
            },
            MethodAttributes::Public | MethodAttributes::SpecialName,
            MethodImplAttributes::default(),
        );
        file.MethodDef(
            "Invoke",
            &Signature {
                flags: MethodCallAttributes::HASTHIS,
                return_type: Type::Void,
                types: argument.iter().cloned().collect(),
            },
            MethodAttributes::Public | MethodAttributes::Virtual | MethodAttributes::NewSlot,
            MethodImplAttributes::default(),
        );
        if argument.is_some() {
            file.Param("inner", 1, ParamAttributes::In);
        }
    }
    let emitter = file.TypeDef(
        "Audit",
        "IEmitter",
        writer::TypeDefOrRef::default(),
        TypeAttributes::Public
            | TypeAttributes::Interface
            | TypeAttributes::Abstract
            | TypeAttributes::WindowsRuntime,
    );
    guid(&mut file, emitter, 0x31d447a3);
    file.MethodDef(
        "SetHandler",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::Void,
            types: vec![Type::named("Audit", "OuterHandler")],
        },
        MethodAttributes::Public
            | MethodAttributes::Abstract
            | MethodAttributes::Virtual
            | MethodAttributes::NewSlot,
        MethodImplAttributes::default(),
    );
    file.Param("handler", 1, ParamAttributes::In);
    fs::write(path, file.into_stream()).unwrap();
}

fn generate_nested_delegate_views(binary: &Path, metadata: &Path, generated: &Path) {
    let output = Command::new(binary)
        .args(["generate", "--winmd"])
        .arg(metadata)
        .args(["--namespace", "Audit", "--lang", "py", "--output"])
        .arg(generated)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn nested_delegate_callback_argument_preserves_native_null() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target")
        .join(format!(
            "nd{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&root).unwrap();
    let fixture = Output(root);
    let metadata = fixture.0.join("Nested.winmd");
    write_nested_delegate_metadata(&metadata);
    let generated = fixture.0.join("projected");
    generate_nested_delegate_views(
        Path::new(env!("CARGO_BIN_EXE_dynwinrt-codegen")),
        &metadata,
        &generated,
    );
    let stub = fs::read_to_string(generated.join("audit__i_emitter.pyi")).unwrap();
    let signature = stub
        .lines()
        .rev()
        .find(|line| line.contains("def set_handler("))
        .expect("generated IEmitter.set_handler signature");
    assert_eq!(
        signature,
        "    def set_handler(self, handler: Callable[[DynWinRTValue | None], object] | \
         'DynWinRTValue | DynWinRtDelegate') -> None: ..."
    );
    assert!(
        !stub.contains("from .audit__inner_handler import IID_InnerHandler, InnerHandler"),
        "{stub}"
    );
    let runtime = fs::read_to_string(generated.join("audit__i_emitter.py")).unwrap();
    let null_projection =
        "lambda __p0__: ((lambda value: None if value.is_null() else value)(__p0__),)";
    assert!(runtime.contains(null_projection), "{runtime}");

    if let Some(main_codegen) = std::env::var_os("DYNWINRT_MAIN_CODEGEN") {
        let baseline = fixture.0.join("main");
        generate_nested_delegate_views(Path::new(&main_codegen), &metadata, &baseline);
        let main_stub = fs::read_to_string(baseline.join("audit__i_emitter.pyi")).unwrap();
        let main_signature = main_stub
            .lines()
            .rev()
            .find(|line| line.contains("def set_handler("))
            .expect("main IEmitter.set_handler signature");
        assert_eq!(signature, main_signature);
        let main_runtime = fs::read_to_string(baseline.join("audit__i_emitter.py")).unwrap();
        assert!(main_runtime.contains(null_projection), "{main_runtime}");
        eprintln!("#191 main and #190 IEmitter stubs: {signature}");
    }

    let python = std::env::var_os("DYNWINRT_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("python"));
    let checker = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("e2e")
        .join("check_generated_python.py");
    let checked = Command::new(&python)
        .arg(checker)
        .arg(&generated)
        .output()
        .unwrap();
    assert!(
        checked.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&checked.stdout),
        String::from_utf8_lossy(&checked.stderr)
    );
    let mypy_available = Command::new(&python)
        .args(["-m", "mypy", "--version"])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        mypy_available || std::env::var("DYNWINRT_REQUIRE_MYPY").as_deref() != Ok("1"),
        "DYNWINRT_REQUIRE_MYPY=1 but mypy is unavailable"
    );
    for (file, consumer, expected_errors) in [
        (
            "valid.py",
            r#"from typing import assert_type
from dynwinrt import DynWinRTValue, DynWinRtDelegate
from projected.audit import IEmitter

def use(emitter: IEmitter, raw: DynWinRTValue, native: DynWinRtDelegate) -> None:
    def callback(inner: DynWinRTValue | None) -> None:
        if inner is not None:
            inner.identity_raw()
    emitter.set_handler(callback)
    emitter.set_handler(lambda inner: assert_type(inner, DynWinRTValue | None))
    emitter.set_handler(raw)
    emitter.set_handler(native)
"#,
            0,
        ),
        (
            "invalid.py",
            r#"from dynwinrt import DynWinRTValue
from projected.audit import IEmitter

def nonnullable(inner: DynWinRTValue) -> None: ...
def invalid(emitter: IEmitter) -> None:
    emitter.set_handler(nonnullable)
"#,
            1,
        ),
    ] {
        fs::write(
            fixture.0.join(file),
            format!("# pyright: strict, reportPrivateUsage=false\n{consumer}"),
        )
        .unwrap();
        if mypy_available {
            let output = Command::new(&python)
                .args([
                    "-B",
                    "-m",
                    "mypy",
                    "--strict",
                    "--no-incremental",
                    "--follow-imports=silent",
                    "--no-pretty",
                    "--show-error-codes",
                    "--cache-dir",
                    ".mypy_cache",
                ])
                .arg(file)
                .current_dir(&fixture.0)
                .env(
                    "MYPYPATH",
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("..")
                        .join("..")
                        .join("bindings")
                        .join("py"),
                )
                .output()
                .unwrap();
            let diagnostics = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let errors = diagnostics
                .lines()
                .filter(|line| line.contains(": error:"))
                .count();
            assert_eq!(errors, expected_errors, "{diagnostics}");
            assert_eq!(
                output.status.success(),
                expected_errors == 0,
                "{diagnostics}"
            );
            if expected_errors != 0 {
                assert!(diagnostics.contains("[arg-type]"), "{diagnostics}");
            }
        }
        if let Some(pyright) = std::env::var_os("DYNWINRT_PYRIGHT") {
            let output = Command::new(pyright)
                .args(["--pythonpath"])
                .arg(&python)
                .arg(file)
                .current_dir(&fixture.0)
                .output()
                .unwrap();
            let diagnostics = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let errors = diagnostics
                .lines()
                .filter(|line| line.contains(" - error: "))
                .count();
            assert_eq!(errors, expected_errors, "{diagnostics}");
            assert_eq!(
                output.status.success(),
                expected_errors == 0,
                "{diagnostics}"
            );
            if expected_errors != 0 {
                assert!(diagnostics.contains("reportArgumentType"), "{diagnostics}");
            }
        }
    }

    let runtime_available = Command::new(&python)
        .args(["-c", "from dynwinrt import DynWinRTImplementation"])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        runtime_available
            || std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref() != Ok("1"),
        "the nested delegate callback probe requires the matching Python binding"
    );
    if !runtime_available {
        eprintln!("Skipping nested delegate native callback: set DYNWINRT_TEST_PYTHON.");
        return;
    }
    fs::write(
        fixture.0.join("native.py"),
        r#"from dynwinrt import (
    DynWinRTImplementation, DynWinRTImplementationMethod, DynWinRTInterfacePlan,
    DynWinRTMethodSig, DynWinRTType, DynWinRTValue, DynWinRtDelegate, RoApartment,
    WinGUID, projected_lifetime_scope, release_projected,
)
from projected.audit import IEmitter

inner_iid = WinGUID.parse("31d447a1-6281-4900-b782-040302010910")
outer_iid = WinGUID.parse("31d447a2-6281-4900-b782-040302010910")
emitter_iid = WinGUID.parse("31d447a3-6281-4900-b782-040302010910")
emitter_sig = DynWinRTMethodSig().add_in(DynWinRTType.delegate(outer_iid))
emitter_type = DynWinRTType.register_interface("Audit.IEmitter", emitter_iid)
emitter_type = emitter_type.add_method("SetHandler", emitter_sig)
plan = DynWinRTInterfacePlan.create(
    "Audit.IEmitter", emitter_type,
    [DynWinRTImplementationMethod("SetHandler", 6, emitter_sig)],
)
outer_sig = DynWinRTMethodSig().add_in(DynWinRTType.delegate(inner_iid))
received = []

with RoApartment(1), projected_lifetime_scope():
    inner = DynWinRtDelegate.create(inner_iid, [], lambda: None)
    inner_value = inner.to_value()
    def callback(argument):
        if argument is None:
            received.append(None)
        else:
            assert isinstance(argument, DynWinRTValue), type(argument)
            assert not argument.is_null()
            received.append(argument.identity_raw() == inner_value.identity_raw())
    def dispatch(_interface, slot, args):
        assert slot == 6
        args[0].invoke_delegate(outer_iid, outer_sig, [DynWinRTValue.null_value()])
        args[0].invoke_delegate(outer_iid, outer_sig, [inner_value])
        return []
    try:
        with DynWinRTImplementation.create([plan], dispatch) as owner:
            value = owner.to_value()
            try:
                emitter = IEmitter.from_value(value)
                try:
                    emitter.set_handler(callback)
                finally:
                    release_projected(emitter)
            finally:
                value.release()
    finally:
        inner_value.release()
assert received == [None, True], received
print("delegate-typed-native-null-ok", received)
"#,
    )
    .unwrap();
    let output = Command::new(&python)
        .args(["-B", "native.py"])
        .current_dir(&fixture.0)
        .output()
        .unwrap();
    let diagnostics = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{diagnostics}");
    assert!(
        diagnostics.contains("delegate-typed-native-null-ok [None, True]"),
        "{diagnostics}"
    );
    eprintln!("{}", String::from_utf8_lossy(&output.stdout).trim());
}

#[test]
fn bespoke_event_delegates_are_typed_and_projected() {
    let Some(output) =
        Output::generate("Windows.ApplicationModel.Background.BackgroundTaskRegistration")
    else {
        return;
    };
    let module = "windows__application_model__background__background_task_registration";
    let callback =
        "Callable[['BackgroundTaskRegistration', 'BackgroundTaskCompletedEventArgs'], object]";
    let py = output.read(module, "py");
    assert!(
        py.contains(
            "def on_completed(self, callback: Callable[..., object] | DynWinRTValue | DynWinRtDelegate):"
        ),
        "{py}"
    );
    assert!(
        py.contains("def once_completed(self, callback: Callable[..., object]):"),
        "{py}"
    );
    assert!(
        py.contains(&format!(
            "'BackgroundTaskCompletedEventHandler_PARAM_TYPES'), lambda __p0__, __p1__: ({}, {}))",
            class_wrapper(module, "BackgroundTaskRegistration", "__p0__"),
            class_wrapper(
                "windows__application_model__background__background_task_completed_event_args",
                "BackgroundTaskCompletedEventArgs",
                "__p1__"
            )
        )),
        "{py}"
    );
    let pyi = output.read(module, "pyi");
    assert!(
        pyi.contains(&format!(
            "def subscribe_completed(self, callback: {callback} | 'DynWinRTValue | DynWinRtDelegate') -> Callable[[], None]: ..."
        )),
        "{pyi}"
    );
    assert!(
        pyi.contains(&format!(
            "def once_completed(self, callback: {callback}) -> Callable[[], None]: ..."
        )),
        "{pyi}"
    );
    assert!(!pyi.contains("Callable[..., object]"), "{pyi}");
}

#[test]
fn runtime_delegate_annotations_resolve_and_stubs_remain_precise() {
    let Some(output) = Output::generate(
        "Windows.Foundation.Collections.PropertySet,\
         Windows.ApplicationModel.Background.BackgroundTaskRegistration",
    ) else {
        return;
    };
    let property_runtime = output
        .0
        .join("windows__foundation__collections__property_set.py");
    let bespoke_runtime = output
        .0
        .join("windows__application_model__background__background_task_registration.py");
    let property_stub = output.read("windows__foundation__collections__property_set", "pyi");
    let bespoke_stub = output.read(
        "windows__application_model__background__background_task_registration",
        "pyi",
    );
    assert!(
        property_stub.contains(
            "def on_map_changed(self, callback: Callable[['IObservableMap_String_Object', \
             'IMapChangedEventArgs_String'], object] | \
             'DynWinRTValue | DynWinRtDelegate') -> 'DynWinRTValue': ..."
        ),
        "{property_stub}"
    );
    assert!(
        bespoke_stub.contains(
            "def on_completed(self, callback: Callable[['BackgroundTaskRegistration', \
             'BackgroundTaskCompletedEventArgs'], object] | \
             'DynWinRTValue | DynWinRtDelegate') -> 'DynWinRTValue': ..."
        ),
        "{bespoke_stub}"
    );

    let script = r#"
import ast
import collections.abc
import pathlib
import sys
import typing

class DynWinRTValue:
    pass

class DynWinRtDelegate:
    pass

namespace = {
    "Callable": collections.abc.Callable,
    "DynWinRTValue": DynWinRTValue,
    "DynWinRtDelegate": DynWinRtDelegate,
}
cases = [
    (pathlib.Path(sys.argv[1]), "PropertySet", "map_changed"),
    (pathlib.Path(sys.argv[2]), "BackgroundTaskRegistration", "completed"),
]
for path, class_name, event in cases:
    tree = ast.parse(path.read_text(encoding="utf-8"))
    source_class = next(
        node for node in tree.body
        if isinstance(node, ast.ClassDef) and node.name == class_name
    )
    wanted = {f"on_{event}", f"once_{event}"}
    methods = []
    for method in source_class.body:
        if isinstance(method, ast.FunctionDef) and method.name in wanted:
            method.body = [ast.Pass()]
            method.decorator_list = []
            methods.append(method)
    minimal = ast.Module(
        body=[
            ast.ImportFrom(
                module="__future__",
                names=[ast.alias(name="annotations")],
                level=0,
            ),
            ast.ClassDef(
                name=class_name,
                bases=[],
                keywords=[],
                body=methods,
                decorator_list=[],
            ),
        ],
        type_ignores=[],
    )
    ast.fix_missing_locations(minimal)
    local = dict(namespace)
    exec(compile(minimal, str(path), "exec"), local)
    generated = local[class_name]
    on_hints = typing.get_type_hints(getattr(generated, f"on_{event}"))
    once_hints = typing.get_type_hints(getattr(generated, f"once_{event}"))
    expected = (
        collections.abc.Callable[..., object]
        | DynWinRTValue
        | DynWinRtDelegate
    )
    assert on_hints == {"callback": expected}, on_hints
    assert once_hints == {
        "callback": collections.abc.Callable[..., object]
    }, once_hints
print("runtime-delegate-type-hints-ok")
"#;
    let result = Command::new("python")
        .args(["-B", "-c", script])
        .arg(property_runtime)
        .arg(bespoke_runtime)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stdout).contains("runtime-delegate-type-hints-ok"),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
}

#[test]
fn static_events_callback_parameters_and_setters_project_callables() {
    let Some(output) = Output::generate(
        "Windows.Gaming.Input.Gamepad,Windows.System.Threading.ThreadPool,\
         Windows.System.Threading.ThreadPoolTimer,\
         Windows.System.Threading.Core.PreallocatedWorkItem,\
         Windows.UI.Popups.UICommand",
    ) else {
        return;
    };

    let gamepad = output.read("windows__gaming__input__gamepad", "py");
    assert!(
        gamepad.contains(
            "def add_gamepad_added(value: Callable[..., object] | DynWinRTValue | DynWinRtDelegate)"
        ),
        "{gamepad}"
    );
    assert!(
        gamepad.contains(&format!(
            "'EventHandler_Gamepad_PARAM_TYPES'), lambda __p0__, __p1__: (\
             (lambda value: None if value.is_null() else value)(__p0__), {}))",
            class_wrapper("windows__gaming__input__gamepad", "Gamepad", "__p1__")
        )),
        "{gamepad}"
    );

    let thread_pool = output.read("windows__system__threading__thread_pool", "py");
    let runtime_callback = "Callable[..., object] | DynWinRTValue | DynWinRtDelegate";
    assert!(
        thread_pool.contains(
            &format!("def run_async(handler: {runtime_callback})")
        ) && thread_pool.contains(&format!(
            "def run_with_priority_async(handler: {runtime_callback}, priority: 'WorkItemPriority')"
        )) && thread_pool.contains(&format!(
            "def run_with_priority_and_options_async(handler: {runtime_callback}, priority: 'WorkItemPriority', options: 'WorkItemOptions')"
        )) && !thread_pool.contains("def run_async(*args, **kwargs):")
            && !thread_pool.contains("_dynwinrt_legacy_call("),
        "{thread_pool}"
    );
    for slot in [6, 7, 8] {
        assert!(
            thread_pool.contains(&format!("_IThreadPoolStatics.method({slot}).invoke(")),
            "{thread_pool}"
        );
    }
    assert!(
        thread_pool.contains("'WorkItemHandler_PARAM_TYPES'))"),
        "{thread_pool}"
    );
    assert!(
        !thread_pool.contains("'WorkItemHandler_PARAM_TYPES'), lambda "),
        "{thread_pool}"
    );
    let thread_pool_stub = output.read("windows__system__threading__thread_pool", "pyi");
    let work_item_callback =
        "Callable[[DynWinRTValue], object] | 'DynWinRTValue | DynWinRtDelegate'";
    for signature in [
        format!("def run_async(handler: {work_item_callback}) -> WinRTCoroutine[None]: ..."),
        format!(
            "def run_with_priority_async(handler: {work_item_callback}, priority: 'WorkItemPriority') -> WinRTCoroutine[None]: ..."
        ),
        format!(
            "def run_with_priority_and_options_async(handler: {work_item_callback}, priority: 'WorkItemPriority', options: 'WorkItemOptions') -> WinRTCoroutine[None]: ..."
        ),
    ] {
        assert!(thread_pool_stub.contains(&signature), "{thread_pool_stub}");
    }
    assert_eq!(thread_pool_stub.matches("def run_async(").count(), 1);
    assert!(!thread_pool_stub.contains("@overload"));

    let timer_callback = "Callable[..., object] | DynWinRTValue | DynWinRtDelegate";
    let timer_stub_callback =
        "Callable[['ThreadPoolTimer'], object] | 'DynWinRTValue | DynWinRtDelegate'";
    let timer = output.read("windows__system__threading__thread_pool_timer", "py");
    assert!(
        timer.contains(&format!(
            "def _create_timer_7(handler: {timer_callback}, delay: timedelta)"
        )) && timer.contains("def create_timer(*args, **kwargs):")
            && timer.contains(
                "return _dynwinrt_legacy_call(ThreadPoolTimer._create_timer_7, ('handler', 'delay',), args, kwargs, 'create_timer')"
            ),
        "{timer}"
    );
    assert!(
        timer.contains(&format!(
            "'TimerElapsedHandler_PARAM_TYPES'), lambda __p0__: ({},))",
            class_wrapper(
                "windows__system__threading__thread_pool_timer",
                "ThreadPoolTimer",
                "__p0__"
            )
        )),
        "{timer}"
    );
    let timer_stub = output.read("windows__system__threading__thread_pool_timer", "pyi");
    assert!(
        timer_stub.contains(&format!(
            "def create_timer(handler: {timer_stub_callback}, delay: timedelta)"
        )),
        "{timer_stub}"
    );

    let command_callback = "Callable[..., object] | DynWinRTValue | DynWinRtDelegate";
    let command_stub_callback =
        "Callable[['IUICommand'], object] | 'DynWinRTValue | DynWinRtDelegate'";
    let command = output.read("windows__ui__popups__ui_command", "py");
    assert!(
        command.contains(&format!("def invoked(self, value: {command_callback}):")),
        "{command}"
    );
    assert!(
        command.contains(
            "'UICommandInvokedHandler_PARAM_TYPES'), lambda __p0__: (\
             (lambda value: None if value.is_null() else \
             _dynwinrt_symbol('windows__ui__popups__iui_command', 'IUICommand')(value))(__p0__),))"
        ),
        "{command}"
    );
    let command_stub = output.read("windows__ui__popups__ui_command", "pyi");
    assert!(
        command_stub.contains(&format!(
            "def invoked(self, value: {command_stub_callback}) -> None"
        )),
        "{command_stub}"
    );

    let work_item_stub = output.read(
        "windows__system__threading__core__preallocated_work_item",
        "pyi",
    );
    assert!(
        work_item_stub.contains(
            "def __init__(self, handler: Callable[[DynWinRTValue], object] | 'DynWinRtDelegate') -> None: ..."
        ),
        "{work_item_stub}"
    );
    assert!(
        !work_item_stub.contains(
            "def __init__(self, handler: Callable[[DynWinRTValue], object] | 'DynWinRTValue"
        ),
        "{work_item_stub}"
    );
}

#[test]
fn class_event_struct_adapter_has_helpers_and_executes() {
    let payload = TypeMeta::Struct {
        namespace: "Contoso".into(),
        name: "Payload".into(),
        fields: vec![FieldMeta {
            name: "Value".into(),
            typ: TypeMeta::I32,
        }],
    };
    let handler = TypeMeta::Interface {
        namespace: "Contoso".into(),
        name: "ChangedHandler".into(),
        iid: "11111111-1111-1111-1111-111111111111".into(),
    };
    let mut events = InterfaceMeta {
        namespace: "Contoso".into(),
        name: "IWidgetEvents".into(),
        iid: "22222222-2222-2222-2222-222222222222".into(),
        methods: vec![
            MethodMeta {
                name: "add_Changed".into(),
                raw_name: "add_Changed".into(),
                vtable_index: 6,
                params: vec![ParamMeta {
                    name: "handler".into(),
                    typ: handler.clone(),
                    direction: ParamDirection::In,
                }],
                is_event_add: true,
                ..Default::default()
            },
            MethodMeta {
                name: "remove_Changed".into(),
                raw_name: "remove_Changed".into(),
                vtable_index: 7,
                params: vec![ParamMeta {
                    name: "token".into(),
                    typ: TypeMeta::I64,
                    direction: ParamDirection::In,
                }],
                is_event_remove: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    events
        .implementation_metadata
        .delegates
        .push(common::delegate_invoke(
            handler,
            &[("payload", payload.clone())],
        ));
    let class = ClassMeta {
        namespace: "Contoso".into(),
        name: "Widget".into(),
        full_name: "Contoso.Widget".into(),
        default_interface: Some(InterfaceMeta {
            namespace: "Contoso".into(),
            name: "IWidget".into(),
            iid: "33333333-3333-3333-3333-333333333333".into(),
            ..Default::default()
        }),
        required_interfaces: vec![events],
        is_referenced_as_value: true,
        ..Default::default()
    };
    let source = common::generate_class(
        &class,
        &HashSet::from(["Payload".into()]),
        &HashSet::from(["ChangedHandler".into()]),
        &HashSet::new(),
    );
    assert!(source.contains("\nclass Payload:\n"), "{source}");
    assert!(
        source.contains("\ndef unpack_payload(v: DynWinRTValue) -> Payload:\n"),
        "{source}"
    );
    assert!(
        source.contains("_unpack_payload = unpack_payload\n"),
        "{source}"
    );
    assert!(
        source.contains("lambda __p0__: (_unpack_payload(__p0__),)"),
        "{source}"
    );

    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!(
            "delegate-struct-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&directory).unwrap();
    let generated = directory.join("widget.py");
    fs::write(&generated, source).unwrap();
    let script = r#"
import ast
import pathlib
import sys

source = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
tree = ast.parse(source)
payload = next(node for node in tree.body if isinstance(node, ast.ClassDef) and node.name == "Payload")
unpack = next(node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name == "unpack_payload")
alias = next(
    node for node in tree.body
    if isinstance(node, ast.Assign)
    and any(isinstance(target, ast.Name) and target.id == "_unpack_payload" for target in node.targets)
)
event = next(
    node for node in ast.walk(tree)
    if isinstance(node, ast.FunctionDef) and node.name == "on_changed"
)
delegate = next(
    node for node in ast.walk(event)
    if isinstance(node, ast.Call)
    and isinstance(node.func, ast.Name)
    and node.func.id == "_dynwinrt_delegate"
)
projection = delegate.args[3]

class FakeStruct:
    def get_i32(self, index):
        assert index == 0
        return 42

class FakeValue:
    def as_struct(self):
        return FakeStruct()

namespace = {"DynWinRTValue": object}
helpers = ast.Module(body=[payload, unpack, alias], type_ignores=[])
ast.fix_missing_locations(helpers)
exec(compile(helpers, sys.argv[1], "exec"), namespace)
ast.fix_missing_locations(projection)
project = eval(compile(ast.Expression(projection), sys.argv[1], "eval"), namespace)
result = project(FakeValue())
assert len(result) == 1
assert isinstance(result[0], namespace["Payload"])
assert result[0].value == 42
print("delegate-struct-adapter-ok")
"#;
    let output = Command::new("python")
        .args(["-c", script])
        .arg(&generated)
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&directory);
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("delegate-struct-adapter-ok"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
