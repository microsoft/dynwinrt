// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python delegate callbacks derive their annotation and argument projection
//! from the delegate `Invoke` signature, wherever a callable becomes a delegate.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

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
        py.contains(&format!("def on_completed(self, callback: {callback}):")),
        "{py}"
    );
    assert!(
        py.contains(&format!(
            "_wrapped = (lambda callback=callback: (lambda __sender__, __args__: callback({}, {})))()",
            class_wrapper(module, "BackgroundTaskRegistration", "__sender__"),
            class_wrapper(
                "windows__application_model__background__background_task_completed_event_args",
                "BackgroundTaskCompletedEventArgs",
                "__args__"
            )
        )),
        "{py}"
    );
    let pyi = output.read(module, "pyi");
    assert!(
        pyi.contains(&format!(
            "def subscribe_completed(self, callback: {callback}) -> Callable[[], None]: ..."
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
            "def add_gamepad_added(value: Callable[[DynWinRTValue | None, 'Gamepad'], object] | 'DynWinRTValue')"
        ),
        "{gamepad}"
    );
    assert!(
        gamepad.contains(&format!(
            "'EventHandler_Gamepad_PARAM_TYPES'), lambda callback: (lambda __sender__, __args__: \
             callback((lambda value: None if value.is_null() else value)(__sender__), {})))",
            class_wrapper("windows__gaming__input__gamepad", "Gamepad", "__args__")
        )),
        "{gamepad}"
    );

    let thread_pool = output.read("windows__system__threading__thread_pool", "py");
    assert!(
        thread_pool.contains(
            "def run_async(handler: Callable[[WinRTCoroutine[None]], object] | 'DynWinRTValue')"
        ),
        "{thread_pool}"
    );
    assert!(
        thread_pool.contains(
            "'WorkItemHandler_PARAM_TYPES'), lambda callback: (lambda __operation__: \
             callback(_dynwinrt_track_projected(_DynWinRTAsync(__operation__, lambda _value: None), 'WinRTAsync'))))"
        ),
        "{thread_pool}"
    );

    let timer_callback = "Callable[['ThreadPoolTimer'], object] | 'DynWinRTValue'";
    let timer = output.read("windows__system__threading__thread_pool_timer", "py");
    assert!(
        timer.contains(&format!(
            "def create_timer(handler: {timer_callback}, delay: timedelta)"
        )),
        "{timer}"
    );
    assert!(
        timer.contains(&format!(
            "'TimerElapsedHandler_PARAM_TYPES'), lambda callback: (lambda __timer__: callback({})))",
            class_wrapper(
                "windows__system__threading__thread_pool_timer",
                "ThreadPoolTimer",
                "__timer__"
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

    let command_callback = "Callable[['IUICommand'], object] | 'DynWinRTValue'";
    let command = output.read("windows__ui__popups__ui_command", "py");
    assert!(
        command.contains(&format!("def invoked(self, value: {command_callback}):")),
        "{command}"
    );
    assert!(
        command.contains(
            "'UICommandInvokedHandler_PARAM_TYPES'), lambda callback: (lambda __command__: \
             callback((lambda value: None if value.is_null() else \
             _dynwinrt_symbol('windows__ui__popups__iui_command', 'IUICommand')(value))(__command__))))"
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
