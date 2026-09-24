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
        py.contains(&format!(
            "def on_completed(self, callback: {callback} | 'DynWinRTValue | DynWinRtDelegate'):"
        )),
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
fn static_events_callback_parameters_and_setters_project_callables() {
    let Some(output) = Output::generate(
        "Windows.Gaming.Input.Gamepad,Windows.System.Threading.ThreadPool,\
         Windows.System.Threading.ThreadPoolTimer,Windows.UI.Popups.UICommand",
    ) else {
        return;
    };

    let gamepad = output.read("windows__gaming__input__gamepad", "py");
    assert!(
        gamepad.contains(
            "def add_gamepad_added(value: Callable[[DynWinRTValue | None, 'Gamepad'], object] | 'DynWinRTValue | DynWinRtDelegate')"
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
    assert!(
        thread_pool.contains(
            "def run_async(handler: Callable[[DynWinRTValue], object] | 'DynWinRTValue | DynWinRtDelegate')"
        ),
        "{thread_pool}"
    );
    assert!(
        thread_pool.contains("'WorkItemHandler_PARAM_TYPES'))"),
        "{thread_pool}"
    );
    assert!(
        !thread_pool.contains("'WorkItemHandler_PARAM_TYPES'), lambda "),
        "{thread_pool}"
    );

    let timer_callback =
        "Callable[['ThreadPoolTimer'], object] | 'DynWinRTValue | DynWinRtDelegate'";
    let timer = output.read("windows__system__threading__thread_pool_timer", "py");
    assert!(
        timer.contains(&format!(
            "def create_timer(handler: {timer_callback}, delay: timedelta)"
        )),
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
            "def create_timer(handler: {timer_callback}, delay: timedelta)"
        )),
        "{timer_stub}"
    );

    let command_callback = "Callable[['IUICommand'], object] | 'DynWinRTValue | DynWinRtDelegate'";
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
            "def invoked(self, value: {command_callback}) -> None"
        )),
        "{command_stub}"
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
