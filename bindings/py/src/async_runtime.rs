// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::cell::Cell;
use std::future::IntoFuture;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::errors::{
    map_dynwinrt_error, map_dynwinrt_error_with_context, map_windows_error_with_context,
};
use crate::runtime::DynWinRTValue;
use pyo3::exceptions::{PyRuntimeError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::PyList;
use windows::Win32::Foundation::CO_E_NOTINITIALIZED;
use windows::Win32::System::Com::{
    APTTYPE_CURRENT, APTTYPE_MAINSTA, APTTYPE_STA, APTTYPEQUALIFIER, APTTYPEQUALIFIER_NONE,
    CoGetApartmentType,
};
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};

thread_local! {
    static TOKIO_RO_INITIALIZED: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn init_async_runtime() {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.enable_all();
    builder.on_thread_start(|| {
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
            .expect("failed to initialize a dynwinrt asyncio worker as MTA");
        TOKIO_RO_INITIALIZED.set(true);
    });
    builder.on_thread_stop(|| {
        if TOKIO_RO_INITIALIZED.replace(false) {
            unsafe { RoUninitialize() };
        }
    });
    pyo3_async_runtimes::tokio::init(builder);
}

fn ensure_async(value: &dynwinrt::WinRTValue) -> PyResult<()> {
    match value {
        dynwinrt::WinRTValue::Async(_) => Ok(()),
        _ => Err(PyRuntimeError::new_err(
            "value is not a WinRT async operation",
        )),
    }
}

pub(crate) fn ensure_progress_type_supported(progress_type: &dynwinrt::TypeHandle) -> PyResult<()> {
    match progress_type.kind() {
        dynwinrt::TypeKind::Guid
        | dynwinrt::TypeKind::ArrayOfIUnknown
        | dynwinrt::TypeKind::Generic { .. }
        | dynwinrt::TypeKind::OutValue(_)
        | dynwinrt::TypeKind::Array(_) => Err(PyRuntimeError::new_err(format!(
            "progress callbacks do not support {:?} values",
            progress_type.kind()
        ))),
        dynwinrt::TypeKind::Bool
        | dynwinrt::TypeKind::I8
        | dynwinrt::TypeKind::U8
        | dynwinrt::TypeKind::I16
        | dynwinrt::TypeKind::U16
        | dynwinrt::TypeKind::Char16
        | dynwinrt::TypeKind::I32
        | dynwinrt::TypeKind::U32
        | dynwinrt::TypeKind::I64
        | dynwinrt::TypeKind::U64
        | dynwinrt::TypeKind::F32
        | dynwinrt::TypeKind::F64
        | dynwinrt::TypeKind::HString
        | dynwinrt::TypeKind::Object
        | dynwinrt::TypeKind::HResult
        | dynwinrt::TypeKind::Interface(_)
        | dynwinrt::TypeKind::Delegate(_)
        | dynwinrt::TypeKind::IAsyncAction
        | dynwinrt::TypeKind::IAsyncActionWithProgress(_)
        | dynwinrt::TypeKind::IAsyncOperation(_)
        | dynwinrt::TypeKind::IAsyncOperationWithProgress(_)
        | dynwinrt::TypeKind::RuntimeClass(_)
        | dynwinrt::TypeKind::Parameterized(_)
        | dynwinrt::TypeKind::Enum(_)
        | dynwinrt::TypeKind::Struct(_) => Ok(()),
    }
}

fn current_thread_is_sta() -> PyResult<bool> {
    let mut apartment_type = APTTYPE_CURRENT;
    let mut qualifier: APTTYPEQUALIFIER = APTTYPEQUALIFIER_NONE;
    match unsafe { CoGetApartmentType(&mut apartment_type, &mut qualifier) } {
        Ok(()) => Ok(apartment_type == APTTYPE_STA || apartment_type == APTTYPE_MAINSTA),
        Err(error) if error.code() == CO_E_NOTINITIALIZED => Ok(false),
        Err(error) => Err(map_windows_error_with_context(
            error,
            "failed to inspect the current COM apartment",
        )),
    }
}

fn has_running_event_loop(py: Python<'_>) -> PyResult<bool> {
    match py.import("asyncio")?.call_method0("get_running_loop") {
        Ok(_) => Ok(true),
        Err(error) if error.is_instance_of::<PyRuntimeError>(py) => Ok(false),
        Err(error) => Err(error),
    }
}

pub(crate) fn wait_for_async(
    value: &dynwinrt::WinRTValue,
    py: Python<'_>,
) -> PyResult<dynwinrt::WinRTValue> {
    let is_started = match value {
        dynwinrt::WinRTValue::Async(info) => info.is_started().map_err(map_dynwinrt_error)?,
        _ => {
            return Err(PyRuntimeError::new_err(
                "value is not a WinRT async operation",
            ));
        }
    };
    if is_started {
        if has_running_event_loop(py)? {
            return Err(PyRuntimeError::new_err(
                "wait() cannot block a started WinRT operation while an asyncio event loop is running; use 'await' instead",
            ));
        }
        if current_thread_is_sta()? {
            return Err(PyRuntimeError::new_err(
                "wait() cannot block a started WinRT operation on an STA thread; use 'await' instead",
            ));
        }
    }

    py.detach(|| pollster::block_on(async { value.await }).map_err(map_dynwinrt_error))
}

enum ExecutionState {
    Idle,
    Blocking,
    Future(Py<PyAny>),
}

enum CoroutineExecutionState {
    New,
    Executing(Py<PyAny>),
    Suspended {
        owner: Py<PyAny>,
        coroutine: Py<PyAny>,
    },
    Finished,
    Closed,
}

struct CoroutineProtocol {
    state: Mutex<CoroutineExecutionState>,
}

impl CoroutineProtocol {
    fn new() -> Self {
        Self {
            state: Mutex::new(CoroutineExecutionState::New),
        }
    }

    fn lock_state(&self) -> PyResult<MutexGuard<'_, CoroutineExecutionState>> {
        self.state
            .lock()
            .map_err(|_| PyRuntimeError::new_err("coroutine state lock was poisoned"))
    }

    fn ensure_awaitable(&self) -> PyResult<()> {
        if matches!(*self.lock_state()?, CoroutineExecutionState::Closed) {
            return Err(PyRuntimeError::new_err(
                "cannot reuse closed WinRT async coroutine",
            ));
        }
        Ok(())
    }

    fn current_owner(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(py.import("asyncio")?.call_method0("current_task")?.unbind())
    }

    fn begin_call(
        &self,
        operation: Option<&AsyncOperation>,
        py: Python<'_>,
        require_started: bool,
    ) -> PyResult<(Py<PyAny>, Py<PyAny>)> {
        let owner = self.current_owner(py)?;
        let existing = {
            let mut state = self.lock_state()?;
            match &*state {
                CoroutineExecutionState::New => None,
                CoroutineExecutionState::Suspended {
                    owner: current_owner,
                    ..
                } if !current_owner.bind(py).is(owner.bind(py)) => {
                    return Err(PyRuntimeError::new_err(
                        "WinRT async coroutine is already being awaited by another task",
                    ));
                }
                CoroutineExecutionState::Suspended { .. } => {
                    match std::mem::replace(
                        &mut *state,
                        CoroutineExecutionState::Executing(owner.clone_ref(py)),
                    ) {
                        CoroutineExecutionState::Suspended { coroutine, .. } => Some(coroutine),
                        _ => unreachable!(),
                    }
                }
                CoroutineExecutionState::Executing(_) => {
                    return Err(PyRuntimeError::new_err(
                        "WinRT async coroutine is already executing",
                    ));
                }
                CoroutineExecutionState::Finished | CoroutineExecutionState::Closed => {
                    return Err(PyRuntimeError::new_err(
                        "cannot reuse already awaited WinRT async coroutine",
                    ));
                }
            }
        };

        if let Some(coroutine) = existing {
            return Ok((owner, coroutine));
        }
        if require_started {
            return Err(PyTypeError::new_err(
                "can't send non-None value to a just-started coroutine",
            ));
        }

        let operation = operation.ok_or_else(|| {
            PyRuntimeError::new_err("the WinRT async operation has been released")
        })?;
        let future = operation.future(py)?;
        let coroutine = py
            .import("dynwinrt.dynwinrt")?
            .getattr("_dynwinrt_drive_future")?
            .call1((future,))?
            .unbind();
        *self.lock_state()? = CoroutineExecutionState::Executing(owner.clone_ref(py));
        Ok((owner, coroutine))
    }

    fn finish_call(
        &self,
        py: Python<'_>,
        owner: Py<PyAny>,
        coroutine: Py<PyAny>,
        result: &PyResult<Py<PyAny>>,
    ) -> PyResult<()> {
        let mut state = self.lock_state()?;
        match &*state {
            CoroutineExecutionState::Executing(current_owner)
                if current_owner.bind(py).is(owner.bind(py)) => {}
            _ => {
                return Err(PyRuntimeError::new_err(
                    "WinRT async coroutine execution state changed unexpectedly",
                ));
            }
        }
        *state = if result.is_ok() {
            CoroutineExecutionState::Suspended { owner, coroutine }
        } else {
            CoroutineExecutionState::Finished
        };
        Ok(())
    }

    fn send(
        &self,
        operation: Option<&AsyncOperation>,
        py: Python<'_>,
        value: Py<PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let require_started = !value.bind(py).is_none();
        let (owner, coroutine) = self.begin_call(operation, py, require_started)?;
        let result = coroutine.call_method1(py, "send", (value,));
        self.finish_call(py, owner, coroutine, &result)?;
        let cleanup_result = match operation {
            Some(operation) if result.is_err() => operation.clear_progress_dispatcher(py),
            _ => Ok(()),
        };
        finish_with_cleanup(py, result, cleanup_result)
    }

    fn throw(
        &self,
        operation: Option<&AsyncOperation>,
        py: Python<'_>,
        typ: Py<PyAny>,
        value: Option<Py<PyAny>>,
        traceback: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let validation_value = value
            .as_ref()
            .map(|value| value.clone_ref(py))
            .unwrap_or_else(|| py.None());
        let validation_traceback = traceback
            .as_ref()
            .map(|traceback| traceback.clone_ref(py))
            .unwrap_or_else(|| py.None());
        py.import("dynwinrt.dynwinrt")?
            .getattr("_dynwinrt_validate_throw")?
            .call1((typ.clone_ref(py), validation_value, validation_traceback))?;

        let was_new = {
            let mut state = self.lock_state()?;
            if matches!(*state, CoroutineExecutionState::New) {
                *state = CoroutineExecutionState::Closed;
                true
            } else {
                false
            }
        };
        if was_new {
            let result = py
                .import("dynwinrt.dynwinrt")?
                .getattr("_dynwinrt_empty_coroutine")?
                .call0()
                .and_then(|coroutine| throw_into_coroutine(py, &coroutine, typ, value, traceback));
            let cleanup_result = match operation {
                Some(operation) => operation.stop(py),
                None => Ok(()),
            };
            return finish_with_cleanup(py, result, cleanup_result);
        }

        let (owner, coroutine) = self.begin_call(operation, py, false)?;
        let result = throw_into_coroutine(py, coroutine.bind(py), typ, value, traceback);
        self.finish_call(py, owner, coroutine, &result)?;
        if let Err(primary_error) = &result {
            if let Some(operation) = operation {
                let progress_result = operation.clear_progress_dispatcher(py);
                let stop_result = match operation.future_is_done(py) {
                    Ok(true) => Ok(()),
                    Ok(false) => operation.stop(py),
                    Err(error) => Err(error),
                };
                let cleanup_result = finish_with_cleanup(py, stop_result, progress_result);
                if let Err(cleanup_error) = cleanup_result {
                    append_cleanup_error(py, primary_error, &cleanup_error)?;
                }
            }
        }
        result
    }

    fn close(&self, operation: &AsyncOperation, py: Python<'_>) -> PyResult<()> {
        let coroutine = {
            let mut state = self.lock_state()?;
            match &*state {
                CoroutineExecutionState::Closed => None,
                CoroutineExecutionState::Finished => return Ok(()),
                CoroutineExecutionState::Executing(_) => {
                    return Err(PyRuntimeError::new_err(
                        "cannot close a WinRT async coroutine while it is executing",
                    ));
                }
                CoroutineExecutionState::New => {
                    *state = CoroutineExecutionState::Closed;
                    None
                }
                CoroutineExecutionState::Suspended { .. } => {
                    match std::mem::replace(&mut *state, CoroutineExecutionState::Closed) {
                        CoroutineExecutionState::Suspended { coroutine, .. } => Some(coroutine),
                        _ => unreachable!(),
                    }
                }
            }
        };

        let close_result = match coroutine {
            Some(coroutine) => coroutine.call_method0(py, "close").map(|_| ()),
            None => Ok(()),
        };
        let cleanup_result = operation.stop(py);
        match (close_result, cleanup_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
            (Err(primary_error), Ok(())) => Err(primary_error),
            (Err(primary_error), Err(cleanup_error)) => {
                append_cleanup_error(py, &primary_error, &cleanup_error)?;
                Err(primary_error)
            }
        }
    }
}

fn throw_into_coroutine(
    py: Python<'_>,
    coroutine: &Bound<'_, PyAny>,
    typ: Py<PyAny>,
    value: Option<Py<PyAny>>,
    traceback: Option<Py<PyAny>>,
) -> PyResult<Py<PyAny>> {
    match (value, traceback) {
        (None, None) => coroutine.call_method1("throw", (typ,)).map(Bound::unbind),
        (Some(value), None) => coroutine
            .call_method1("throw", (typ, value))
            .map(Bound::unbind),
        (value, Some(traceback)) => coroutine
            .call_method1(
                "throw",
                (typ, value.unwrap_or_else(|| py.None()), traceback),
            )
            .map(Bound::unbind),
    }
}

fn finish_with_cleanup<T>(
    py: Python<'_>,
    result: PyResult<T>,
    cleanup_result: PyResult<()>,
) -> PyResult<T> {
    match (result, cleanup_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
        (Err(primary_error), Ok(())) => Err(primary_error),
        (Err(primary_error), Err(cleanup_error)) => {
            append_cleanup_error(py, &primary_error, &cleanup_error)?;
            Err(primary_error)
        }
    }
}

fn append_cleanup_error(py: Python<'_>, primary: &PyErr, cleanup: &PyErr) -> PyResult<()> {
    py.import("dynwinrt.dynwinrt")?
        .getattr("_dynwinrt_append_exception_cause")?
        .call1((primary.value(py), cleanup.value(py)))?;
    Ok(())
}

struct AsyncOperation {
    value: dynwinrt::WinRTValue,
    converter: Py<PyAny>,
    state: Mutex<ExecutionState>,
    progress_dispatcher: Mutex<Option<Arc<ProgressDispatcher>>>,
}

struct ProgressDispatcher {
    event_loop: Py<PyAny>,
    // Loop handles retain this list, which is cleared to release captures and disable queued work.
    dispatch_state: Py<PyAny>,
    dispatch_progress: Py<PyAny>,
    callback_context: Py<PyAny>,
}

impl ProgressDispatcher {
    fn deactivate(&self, py: Python<'_>) -> PyResult<()> {
        self.dispatch_state.call_method0(py, "clear").map(|_| ())
    }

    fn dispatch(&self, py: Python<'_>, value: dynwinrt::WinRTValue) -> PyResult<()> {
        if self.event_loop.call_method0(py, "is_closed")?.extract(py)? {
            return Ok(());
        }

        let raw = Py::new(py, DynWinRTValue(value))?;
        let context = self.callback_context.call_method0(py, "copy")?;
        let context_run = context.getattr(py, "run")?;
        self.event_loop.call_method1(
            py,
            "call_soon_threadsafe",
            (
                context_run,
                self.dispatch_progress.clone_ref(py),
                self.dispatch_state.clone_ref(py),
                raw,
            ),
        )?;
        Ok(())
    }
}

impl AsyncOperation {
    fn new(value: &DynWinRTValue, converter: Py<PyAny>) -> PyResult<Self> {
        ensure_async(&value.0)?;
        Ok(Self {
            value: value.0.clone(),
            converter,
            state: Mutex::new(ExecutionState::Idle),
            progress_dispatcher: Mutex::new(None),
        })
    }

    fn lock_state(&self) -> PyResult<MutexGuard<'_, ExecutionState>> {
        self.state
            .lock()
            .map_err(|_| PyRuntimeError::new_err("async operation state lock was poisoned"))
    }

    fn stop(&self, py: Python<'_>) -> PyResult<()> {
        let progress_result = self.clear_progress_dispatcher(py);
        let cancel_result = self.cancel();
        let future = {
            let mut state = self.lock_state()?;
            match std::mem::replace(&mut *state, ExecutionState::Idle) {
                ExecutionState::Future(future) => Some(future),
                ExecutionState::Idle | ExecutionState::Blocking => None,
            }
        };
        let future_result = match future {
            Some(future) => future.call_method0(py, "cancel").map(|_| ()),
            None => Ok(()),
        };
        let cancel_result = finish_with_cleanup(py, cancel_result, future_result);
        finish_with_cleanup(py, cancel_result, progress_result)
    }

    fn lock_progress_dispatcher(
        &self,
    ) -> PyResult<MutexGuard<'_, Option<Arc<ProgressDispatcher>>>> {
        self.progress_dispatcher
            .lock()
            .map_err(|_| PyRuntimeError::new_err("progress dispatcher lock was poisoned"))
    }

    fn set_progress_dispatcher(
        &self,
        py: Python<'_>,
        dispatcher: Arc<ProgressDispatcher>,
    ) -> PyResult<()> {
        let previous = self.lock_progress_dispatcher()?.replace(dispatcher);
        match previous {
            Some(previous) => previous.deactivate(py),
            None => Ok(()),
        }
    }

    fn clear_progress_dispatcher(&self, py: Python<'_>) -> PyResult<()> {
        let dispatcher = self.lock_progress_dispatcher()?.take();
        match dispatcher {
            Some(dispatcher) => dispatcher.deactivate(py),
            None => Ok(()),
        }
    }

    fn future_is_done(&self, py: Python<'_>) -> PyResult<bool> {
        let future = {
            let state = self.lock_state()?;
            match &*state {
                ExecutionState::Future(future) => Some(future.clone_ref(py)),
                ExecutionState::Idle | ExecutionState::Blocking => None,
            }
        };
        match future {
            Some(future) => future.call_method0(py, "done")?.extract(py),
            None => Ok(false),
        }
    }

    fn future<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let mut state = self.lock_state()?;
        match &*state {
            ExecutionState::Future(future) => return Ok(future.clone_ref(py).into_bound(py)),
            ExecutionState::Blocking => {
                return Err(PyRuntimeError::new_err(
                    "cannot await a WinRT operation while wait() is running",
                ));
            }
            ExecutionState::Idle => {}
        }

        let value = self.value.clone();
        let winrt_future = value.into_future().defer_get_results().cancel_on_drop();
        let raw_future = pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = winrt_future.await;
            let result = result.map_err(map_dynwinrt_error)?;
            Ok(DynWinRTValue(result))
        })?;

        let converter = self.converter.clone_ref(py);
        let convert_future = py
            .import("dynwinrt.dynwinrt")?
            .getattr("_dynwinrt_convert_future")?;
        let coroutine = convert_future.call1((raw_future.clone(), converter))?;
        let future = py
            .import("asyncio")?
            .call_method0("get_running_loop")?
            .call_method1("create_task", (coroutine,))?;
        py.import("dynwinrt.dynwinrt")?
            .getattr("_dynwinrt_link_cancellation")?
            .call1((future.clone(), raw_future))?;
        let future = future.unbind();
        let result = future.clone_ref(py).into_bound(py);
        *state = ExecutionState::Future(future);
        Ok(result)
    }

    fn await_iter<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.future(py)?.call_method0("__await__")
    }

    fn wait(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        {
            let mut state = self.lock_state()?;
            match &*state {
                ExecutionState::Idle => *state = ExecutionState::Blocking,
                ExecutionState::Blocking => {
                    return Err(PyRuntimeError::new_err("wait() is already running"));
                }
                ExecutionState::Future(_) => {
                    return Err(PyRuntimeError::new_err(
                        "cannot call wait() after awaiting has started",
                    ));
                }
            }
        }

        let result = wait_for_async(&self.value, py);
        {
            let mut state = self.lock_state()?;
            *state = ExecutionState::Idle;
        }

        let raw = Py::new(py, DynWinRTValue(result?))?;
        self.converter.call1(py, (raw,))
    }

    fn cancel(&self) -> PyResult<()> {
        match &self.value {
            dynwinrt::WinRTValue::Async(info) => info
                .cancel()
                .map_err(|error| map_dynwinrt_error_with_context(error, "Cancel failed")),
            _ => Err(PyRuntimeError::new_err(
                "value is not a WinRT async operation",
            )),
        }
    }
}

pub(crate) fn finish_progress_registration(
    set_result: dynwinrt::Result<()>,
    is_started_after: impl FnOnce() -> PyResult<bool>,
) -> PyResult<()> {
    match set_result {
        Ok(()) => Ok(()),
        Err(error) => {
            if is_started_after()? {
                Err(map_dynwinrt_error_with_context(error, "SetProgress failed"))
            } else {
                Ok(())
            }
        }
    }
}

#[pyclass(name = "_DynWinRTAsync")]
pub struct DynWinRTAsync {
    operation: Option<Arc<AsyncOperation>>,
    coroutine: CoroutineProtocol,
}

impl DynWinRTAsync {
    fn operation(&self) -> PyResult<&Arc<AsyncOperation>> {
        self.operation
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("the WinRT async operation has been released"))
    }
}

#[pymethods]
impl DynWinRTAsync {
    #[new]
    fn new(value: &DynWinRTValue, result_converter: Py<PyAny>) -> PyResult<Self> {
        Ok(Self {
            operation: Some(Arc::new(AsyncOperation::new(value, result_converter)?)),
            coroutine: CoroutineProtocol::new(),
        })
    }

    fn __await__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let operation = self.operation()?;
        self.coroutine.ensure_awaitable()?;
        operation.await_iter(py)
    }

    fn send(&self, py: Python<'_>, value: Py<PyAny>) -> PyResult<Py<PyAny>> {
        self.coroutine.send(self.operation.as_deref(), py, value)
    }

    #[pyo3(signature = (typ, value=None, traceback=None))]
    fn throw(
        &self,
        py: Python<'_>,
        typ: Py<PyAny>,
        value: Option<Py<PyAny>>,
        traceback: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        self.coroutine
            .throw(self.operation.as_deref(), py, typ, value, traceback)
    }

    fn close(&self, py: Python<'_>) -> PyResult<()> {
        self.coroutine.close(self.operation()?, py)
    }

    fn wait(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.operation()?.wait(py)
    }

    fn cancel(&self, py: Python<'_>) -> PyResult<()> {
        let operation = self.operation()?;
        let progress_result = operation.clear_progress_dispatcher(py);
        finish_with_cleanup(py, operation.cancel(), progress_result)
    }

    fn release(&mut self, py: Python<'_>) -> PyResult<()> {
        if let Some(operation) = &self.operation {
            operation.stop(py)?;
        }
        self.operation = None;
        Ok(())
    }

    fn __repr__(&self) -> &'static str {
        "_DynWinRTAsync(...)"
    }
}

#[pyclass(name = "_DynWinRTAsyncWithProgress")]
pub struct DynWinRTAsyncWithProgress {
    operation: Option<Arc<AsyncOperation>>,
    coroutine: CoroutineProtocol,
    progress_converter: Py<PyAny>,
}

impl DynWinRTAsyncWithProgress {
    fn operation(&self) -> PyResult<&Arc<AsyncOperation>> {
        self.operation
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("the WinRT async operation has been released"))
    }
}

#[pymethods]
impl DynWinRTAsyncWithProgress {
    #[new]
    fn new(
        value: &DynWinRTValue,
        result_converter: Py<PyAny>,
        progress_converter: Py<PyAny>,
    ) -> PyResult<Self> {
        let operation = Arc::new(AsyncOperation::new(value, result_converter)?);
        let has_progress = match &operation.value {
            dynwinrt::WinRTValue::Async(info) => info.progress_type().is_some(),
            _ => false,
        };
        if !has_progress {
            return Err(PyRuntimeError::new_err(
                "value is not a WinRT async operation with progress",
            ));
        }
        Ok(Self {
            operation: Some(operation),
            coroutine: CoroutineProtocol::new(),
            progress_converter,
        })
    }

    fn __await__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let operation = self.operation()?;
        self.coroutine.ensure_awaitable()?;
        operation.await_iter(py)
    }

    fn send(&self, py: Python<'_>, value: Py<PyAny>) -> PyResult<Py<PyAny>> {
        self.coroutine.send(self.operation.as_deref(), py, value)
    }

    #[pyo3(signature = (typ, value=None, traceback=None))]
    fn throw(
        &self,
        py: Python<'_>,
        typ: Py<PyAny>,
        value: Option<Py<PyAny>>,
        traceback: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        self.coroutine
            .throw(self.operation.as_deref(), py, typ, value, traceback)
    }

    fn close(&self, py: Python<'_>) -> PyResult<()> {
        self.coroutine.close(self.operation()?, py)
    }

    fn wait(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.operation()?.wait(py)
    }

    fn cancel(&self, py: Python<'_>) -> PyResult<()> {
        let operation = self.operation()?;
        let progress_result = operation.clear_progress_dispatcher(py);
        finish_with_cleanup(py, operation.cancel(), progress_result)
    }

    fn release(&mut self, py: Python<'_>) -> PyResult<()> {
        if let Some(operation) = &self.operation {
            operation.stop(py)?;
        }
        self.operation = None;
        Ok(())
    }

    fn progress(&self, py: Python<'_>, callback: Py<PyAny>) -> PyResult<()> {
        let loop_ = py
            .import("asyncio")?
            .call_method0("get_running_loop")
            .map_err(|_| {
                PyRuntimeError::new_err("progress() requires a running asyncio event loop")
            })?
            .unbind();

        let operation = self.operation()?;
        let info = match &operation.value {
            dynwinrt::WinRTValue::Async(info) => info,
            _ => {
                return Err(PyRuntimeError::new_err(
                    "value is not a WinRT async operation",
                ));
            }
        };
        if !info.is_started().map_err(map_dynwinrt_error)? {
            // Fast operations may already be terminal by the time Python
            // registers progress. No future progress can be delivered.
            return Ok(());
        }
        let progress_type = info.progress_type().ok_or_else(|| {
            PyRuntimeError::new_err("value is not a WinRT async operation with progress")
        })?;
        ensure_progress_type_supported(&progress_type)?;
        let handler_iid = info.progress_handler_iid().ok_or_else(|| {
            PyRuntimeError::new_err("cannot compute the WinRT progress handler IID")
        })?;
        let dispatcher = Arc::new(ProgressDispatcher {
            event_loop: loop_,
            dispatch_state: PyList::new(py, [callback, self.progress_converter.clone_ref(py)])?
                .into_any()
                .unbind(),
            dispatch_progress: py
                .import("dynwinrt.dynwinrt")?
                .getattr("_dynwinrt_dispatch_progress")?
                .unbind(),
            callback_context: py
                .import("contextvars")?
                .getattr("copy_context")?
                .call0()?
                .unbind(),
        });
        // The native handler must not extend the Python callback or event-loop lifetime.
        let weak_dispatcher = Arc::downgrade(&dispatcher);

        let progress_callback: dynwinrt::ProgressCallback = Box::new(move |value| {
            Python::attach(|py| {
                let Some(dispatcher) = weak_dispatcher.upgrade() else {
                    return;
                };
                if let Err(error) = dispatcher.dispatch(py, value) {
                    error.write_unraisable(py, Some(dispatcher.dispatch_state.bind(py)));
                }
            });
        });
        let handler =
            dynwinrt::try_create_progress_handler(handler_iid, progress_type, progress_callback)
                .map_err(|error| {
                    map_dynwinrt_error_with_context(
                        error,
                        "failed to create WinRT progress handler",
                    )
                })?;
        match info.set_progress_handler(&handler) {
            Ok(()) => operation.set_progress_dispatcher(py, dispatcher),
            Err(error) => {
                let registration_result = finish_progress_registration(Err(error), || {
                    info.is_started().map_err(map_dynwinrt_error)
                });
                finish_with_cleanup(py, registration_result, dispatcher.deactivate(py))
            }
        }
    }

    fn __repr__(&self) -> &'static str {
        "_DynWinRTAsyncWithProgress(...)"
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    };

    use super::*;

    fn set_progress_error() -> dynwinrt::Error {
        dynwinrt::Error::WindowsError(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x80004005u32 as i32,
        )))
    }

    #[test]
    fn progress_registration_ignores_failure_after_concurrent_completion() {
        let started = Arc::new(AtomicBool::new(true));
        let begin_transition = Arc::new(Barrier::new(2));
        let transition_done = Arc::new(Barrier::new(2));
        let worker_started = started.clone();
        let worker_begin = begin_transition.clone();
        let worker_done = transition_done.clone();
        let worker = std::thread::spawn(move || {
            worker_begin.wait();
            worker_started.store(false, Ordering::SeqCst);
            worker_done.wait();
        });

        let result = finish_progress_registration(Err(set_progress_error()), || {
            begin_transition.wait();
            transition_done.wait();
            Ok(started.load(Ordering::SeqCst))
        });

        worker.join().expect("completion worker failed");
        assert!(result.is_ok());
    }

    #[test]
    fn progress_registration_surfaces_failure_while_operation_is_started() {
        let result = finish_progress_registration(Err(set_progress_error()), || Ok(true));
        assert!(result.is_err());
    }

    #[test]
    fn progress_type_validation_allows_structs_and_rejects_unsupported_shapes() {
        let table = dynwinrt::MetadataTable::new();
        let progress = table.struct_type("Test.PythonStructProgress", &[table.u64_type()]);
        assert!(ensure_progress_type_supported(&progress).is_ok());

        let value_type = table.u32_type();
        for unsupported in [
            table.guid_type(),
            table.array_of_iunknown(),
            table.generic(
                windows::core::GUID::from_u128(0xaaaaaaaa_bbbb_cccc_dddd_eeeeeeeeeeee),
                1,
            ),
            table.out_value(&value_type),
            table.array(&value_type),
        ] {
            assert!(ensure_progress_type_supported(&unsupported).is_err());
        }
    }
}
