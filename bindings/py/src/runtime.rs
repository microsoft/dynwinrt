// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
    cell::{Cell, RefCell},
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, ThreadId},
};

use dynwinrt;
use pyo3::exceptions::{PyIndexError, PyOverflowError, PyRuntimeError, PyTypeError};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::PyDict;
use windows::Win32::System::WinRT::{
    RO_INIT_MULTITHREADED, RO_INIT_SINGLETHREADED, RO_INIT_TYPE, RoInitialize,
};
use windows::core::{GUID, HSTRING, IInspectable, IUnknown, Interface};

use crate::errors::{
    InputSlot, map_dynwinrt_error, map_dynwinrt_error_with_context, map_windows_error,
    non_object_receiver_error, released_input_error, released_native_container_error,
    released_receiver_error,
};

/// Shared MetadataTable — created once, used everywhere.
static TABLE: std::sync::LazyLock<Arc<dynwinrt::MetadataTable>> =
    std::sync::LazyLock::new(|| dynwinrt::MetadataTable::new());

pub(crate) static WINUI_MODULES: dynwinrt::WinUiProcessModules =
    dynwinrt::WinUiProcessModules::new();
static PYTHON_SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
static CALLBACKS_IN_FLIGHT: Mutex<usize> = Mutex::new(0);
#[cfg(test)]
static CALLBACK_ATTACH_ATTEMPTS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

struct NativeCallbackPermit;

impl Drop for NativeCallbackPermit {
    fn drop(&mut self) {
        match CALLBACKS_IN_FLIGHT.lock() {
            Ok(mut active) if *active > 0 => *active -= 1,
            _ => {
                PYTHON_SHUTTING_DOWN.store(true, Ordering::Release);
                log_unavailable_python_callback();
            }
        }
    }
}

pub(crate) fn python_callback_available() -> bool {
    !PYTHON_SHUTTING_DOWN.load(Ordering::Acquire) && unsafe { pyo3::ffi::Py_IsInitialized() != 0 }
}

pub(crate) fn ensure_python_callbacks_open() -> PyResult<()> {
    if !python_callback_available() {
        return Err(PyRuntimeError::new_err(
            "Python WinRT callbacks have been shut down; register them before shutdown_python_callbacks()",
        ));
    }
    Ok(())
}

pub(crate) fn log_unavailable_python_callback() {
    unsafe {
        OutputDebugStringA(b"dynwinrt: rejecting a WinRT callback after Python shutdown\0".as_ptr())
    };
}

pub(crate) fn with_python_callback<R>(
    callback: impl for<'py> FnOnce(Python<'py>) -> R,
) -> Option<R> {
    if !python_callback_available() {
        log_unavailable_python_callback();
        return None;
    }
    let permit = {
        let mut active = match CALLBACKS_IN_FLIGHT.lock() {
            Ok(active) => active,
            Err(_) => {
                log_unavailable_python_callback();
                return None;
            }
        };
        if !python_callback_available() {
            log_unavailable_python_callback();
            return None;
        }
        let Some(next) = active.checked_add(1) else {
            log_unavailable_python_callback();
            return None;
        };
        *active = next;
        NativeCallbackPermit
    };
    #[cfg(test)]
    CALLBACK_ATTACH_ATTEMPTS.fetch_add(1, Ordering::SeqCst);
    let callback_guard = NativeCallbackGuard::enter();
    let result = Python::try_attach(callback);
    drop(callback_guard);
    drop(permit);
    match result {
        Some(result) => Some(result),
        None => {
            log_unavailable_python_callback();
            None
        }
    }
}

#[pyfunction]
pub(crate) fn _dynwinrt_close_callback_gate() {
    PYTHON_SHUTTING_DOWN.store(true, Ordering::Release);
}

fn close_native_callback_gate() -> PyResult<()> {
    let active = CALLBACKS_IN_FLIGHT.try_lock().map_err(|error| {
        PyRuntimeError::new_err(format!(
            "cannot shut down Python WinRT callbacks while native callback bookkeeping is busy: {error}"
        ))
    })?;
    if *active != 0 {
        return Err(PyRuntimeError::new_err(format!(
            "cannot shut down Python WinRT callbacks while {} callback(s) are in flight; settle them and retry",
            *active
        )));
    }
    PYTHON_SHUTTING_DOWN.store(true, Ordering::Release);
    Ok(())
}

#[pyfunction]
pub fn shutdown_python_callbacks(py: Python<'_>) -> PyResult<()> {
    let runtime = py
        .import("dynwinrt.dynwinrt")?
        .getattr("_dynwinrt_implementation_runtime")?;
    close_native_callback_gate()?;
    runtime.call_method0("shutdown")?;
    Ok(())
}

pub(crate) fn wrap_python_callback_context(
    py: Python<'_>,
    callback: Py<PyAny>,
) -> PyResult<Py<PyAny>> {
    Ok(py
        .import("dynwinrt.dynwinrt")?
        .getattr("_dynwinrt_wrap_delegate_callback")?
        .call1((callback,))?
        .unbind())
}

fn checked_index(index: i64) -> PyResult<usize> {
    usize::try_from(index)
        .map_err(|_| PyIndexError::new_err(format!("index {index} out of bounds")))
}

fn checked_i8(value: i32, context: &str) -> PyResult<i8> {
    i8::try_from(value)
        .map_err(|_| PyOverflowError::new_err(format!("{context}: {value} does not fit in Int8")))
}

fn checked_u8(value: u32, context: &str) -> PyResult<u8> {
    u8::try_from(value)
        .map_err(|_| PyOverflowError::new_err(format!("{context}: {value} does not fit in UInt8")))
}

fn checked_i16(value: i32, context: &str) -> PyResult<i16> {
    i16::try_from(value)
        .map_err(|_| PyOverflowError::new_err(format!("{context}: {value} does not fit in Int16")))
}

fn checked_u16(value: u32, context: &str) -> PyResult<u16> {
    u16::try_from(value)
        .map_err(|_| PyOverflowError::new_err(format!("{context}: {value} does not fit in UInt16")))
}

fn ensure_field_kind(
    value: &dynwinrt::ValueTypeData,
    index: usize,
    expected: dynwinrt::TypeKind,
    accepted: &[dynwinrt::TypeKind],
) -> PyResult<()> {
    let actual = value
        .field_kind_checked(index)
        .map_err(map_dynwinrt_error)?;
    let enum_storage = matches!(actual, dynwinrt::TypeKind::Enum(_))
        && value.type_handle().field_type(index).underlying_kind() == expected;
    let bool_storage = actual == dynwinrt::TypeKind::Bool && expected == dynwinrt::TypeKind::U8;
    let hresult_storage =
        actual == dynwinrt::TypeKind::HResult && expected == dynwinrt::TypeKind::I32;
    if actual == expected
        || accepted.contains(&actual)
        || enum_storage
        || bool_storage
        || hresult_storage
    {
        Ok(())
    } else {
        Err(map_dynwinrt_error(dynwinrt::Error::InvalidType(
            expected, actual,
        )))
    }
}

fn get_typed_field<T: Copy, U>(
    value: &dynwinrt::ValueTypeData,
    index: i64,
    expected: dynwinrt::TypeKind,
    accepted: &[dynwinrt::TypeKind],
    convert: impl FnOnce(T) -> U,
) -> PyResult<U> {
    let index = checked_index(index)?;
    ensure_field_kind(value, index, expected, accepted)?;
    Ok(convert(value.get_field::<T>(index)))
}

fn set_typed_field<T: Copy>(
    value: &mut dynwinrt::ValueTypeData,
    index: i64,
    field_value: T,
    expected: dynwinrt::TypeKind,
    accepted: &[dynwinrt::TypeKind],
) -> PyResult<()> {
    let index = checked_index(index)?;
    ensure_field_kind(value, index, expected, accepted)?;
    value.set_field(index, field_value);
    Ok(())
}

// ======================================================================
// Runtime initialization
// ======================================================================

#[derive(Clone, Copy)]
struct PendingApartment {
    apartment_type: i32,
    retry_cleanup: bool,
}

struct ForeignDropQueue {
    owner_alive: bool,
    pending: Vec<PendingApartment>,
}

struct OwnerDropQueue(Arc<Mutex<ForeignDropQueue>>);

impl OwnerDropQueue {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(ForeignDropQueue {
            owner_alive: true,
            pending: Vec::new(),
        })))
    }
}

impl Drop for OwnerDropQueue {
    fn drop(&mut self) {
        let (pending, poisoned) = {
            let (mut queue, poisoned) = lock_foreign_drops(&self.0);
            queue.owner_alive = false;
            (queue.pending.len(), poisoned)
        };
        if poisoned {
            native_apartment_diagnostic("RoApartment owner-thread pending state was poisoned");
        }
        if pending != 0 {
            native_apartment_diagnostic(&format!(
                "{} dropped RoApartment initialization(s) were not recovered before their \
                 owner OS thread exited",
                pending
            ));
        }
    }
}

fn lock_foreign_drops(queue: &Mutex<ForeignDropQueue>) -> (MutexGuard<'_, ForeignDropQueue>, bool) {
    match queue.lock() {
        Ok(guard) => (guard, false),
        Err(poisoned) => {
            queue.clear_poison();
            (poisoned.into_inner(), true)
        }
    }
}

fn native_apartment_diagnostic(message: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "{message}");
}

thread_local! {
    static MANAGED_APARTMENT_DEPTH: Cell<usize> = const { Cell::new(0) };
    static MANUAL_APARTMENT_DEPTH: Cell<usize> = const { Cell::new(0) };
    static PENDING_APARTMENT_CLOSES: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    static CALLBACK_PENDING_APARTMENTS: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    static PENDING_RETRY_IN_PROGRESS: Cell<bool> = const { Cell::new(false) };
    static NATIVE_CALLBACK_DEPTH: Cell<usize> = const { Cell::new(0) };
    static FOREIGN_DROPS: OwnerDropQueue = OwnerDropQueue::new();
}

pub(crate) struct NativeCallbackGuard {
    counted: bool,
    _owner_thread: PhantomData<Rc<()>>,
}

impl NativeCallbackGuard {
    pub(crate) fn enter() -> Self {
        let counted = NATIVE_CALLBACK_DEPTH
            .try_with(|depth| {
                let Some(next) = depth.get().checked_add(1) else {
                    return false;
                };
                depth.set(next);
                true
            })
            .unwrap_or(false);
        Self {
            counted,
            _owner_thread: PhantomData,
        }
    }
}

impl Drop for NativeCallbackGuard {
    fn drop(&mut self) {
        if self.counted {
            let _ = NATIVE_CALLBACK_DEPTH.try_with(|depth| depth.set(depth.get() - 1));
        }
    }
}

struct PendingRetryGuard;

impl PendingRetryGuard {
    fn enter() -> PyResult<Self> {
        PENDING_RETRY_IN_PROGRESS
            .try_with(|busy| {
                if busy.replace(true) {
                    return Err(PyRuntimeError::new_err(
                        "an apartment cleanup retry is already in progress on this thread",
                    ));
                }
                Ok(Self)
            })
            .map_err(|_| {
                PyRuntimeError::new_err("owner-thread pending state is unavailable during teardown")
            })?
    }
}

impl Drop for PendingRetryGuard {
    fn drop(&mut self) {
        let _ = PENDING_RETRY_IN_PROGRESS.try_with(|busy| busy.set(false));
    }
}

fn check_reentrant_uninitialize(operation: &str) -> PyResult<()> {
    let callback_active = NATIVE_CALLBACK_DEPTH
        .try_with(|depth| depth.get() != 0)
        .map_err(|_| {
            PyRuntimeError::new_err(format!(
                "{operation}: owner-thread callback state is unavailable during teardown"
            ))
        })?;
    let final_apartment = if callback_active {
        MANAGED_APARTMENT_DEPTH
            .try_with(|depth| depth.get() <= 1)
            .map_err(|_| {
                PyRuntimeError::new_err(
                    "owner-thread apartment state is unavailable during teardown",
                )
            })?
    } else {
        false
    };
    if final_apartment {
        return Err(PyRuntimeError::new_err(format!(
            "{operation} cannot risk uninitializing the final COM apartment during a synchronous \
             native callback on this thread; retry after the callback and native invocation \
             return. Recover a dropped context with RoApartment.recover_pending() on this thread."
        )));
    }
    Ok(())
}

#[pyclass]
pub struct WinAppSDKContext(pub(crate) dynwinrt::WinAppSdkContext);

#[pymethods]
impl WinAppSDKContext {
    /// Return the framework resources.pri path selected by init_winappsdk.
    fn resource_pri_path(&self) -> PyResult<String> {
        self.0.resource_pri_path().map_err(map_windows_error)
    }
}

#[pyclass]
pub struct RoApartment {
    apartment_type: i32,
    active: bool,
    close_rejected: bool,
    owner_thread: ThreadId,
    foreign_drops: Arc<Mutex<ForeignDropQueue>>,
    cleanup_failed: bool,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn OutputDebugStringA(message: *const u8);
}

fn log_unsafe_apartment_teardown() {
    unsafe {
        OutputDebugStringA(
            b"dynwinrt: retaining an apartment that cannot be safely closed on its owner thread\0"
                .as_ptr(),
        )
    };
}

pub(crate) fn log_unsafe_native_owner_drop() {
    unsafe {
        OutputDebugStringA(
            b"dynwinrt: quarantining a native COM owner without its apartment or Python GIL\0"
                .as_ptr(),
        )
    };
}

pub(crate) fn current_native_owner_thread(owns_com: bool) -> Option<ThreadId> {
    owns_com.then(|| thread::current().id())
}

fn ensure_native_access_thread(
    owner: Option<ThreadId>,
    release_any_thread: bool,
    name: &str,
) -> PyResult<()> {
    if !release_any_thread && owner.is_some_and(|thread| thread != thread::current().id()) {
        return Err(PyRuntimeError::new_err(format!(
            "{name} requires its owning COM apartment thread"
        )));
    }
    Ok(())
}

pub(crate) fn ensure_native_owner_thread(
    owner: Option<ThreadId>,
    release_any_thread: bool,
    name: &str,
) -> PyResult<()> {
    if !release_any_thread && owner.is_some_and(|thread| thread != thread::current().id()) {
        return Err(PyRuntimeError::new_err(format!(
            "{name}.release() must run on its owning COM apartment thread"
        )));
    }
    Ok(())
}

pub(crate) fn python_gil_usable() -> bool {
    (unsafe { pyo3::ffi::Py_IsInitialized() != 0 && pyo3::ffi::PyGILState_Check() != 0 })
        && Python::try_attach(|_| ()).is_some()
}

pub(crate) fn must_quarantine_owner(owner: Option<ThreadId>, release_any_thread: bool) -> bool {
    owner.is_some()
        && ((!release_any_thread && owner != Some(thread::current().id())) || !python_gil_usable())
}

fn managed_apartment_depth() -> usize {
    MANAGED_APARTMENT_DEPTH.with(Cell::get)
}

fn enter_managed_apartment(apartment_type: i32) -> PyResult<()> {
    MANAGED_APARTMENT_DEPTH.try_with(|_| ()).map_err(|_| {
        PyRuntimeError::new_err("owner-thread apartment state is unavailable during teardown")
    })?;
    unsafe { RoInitialize(ro_init_type(apartment_type)) }.map_err(map_windows_error)?;
    MANAGED_APARTMENT_DEPTH.with(|depth| depth.set(depth.get() + 1));
    Ok(())
}

fn leave_managed_apartment_with_drain(
    operation: &str,
    drain: impl FnOnce() -> PyResult<()>,
) -> PyResult<()> {
    check_reentrant_uninitialize(operation)?;
    let depth = managed_apartment_depth();
    if depth == 0 {
        return Err(PyRuntimeError::new_err(
            "no successful dynwinrt RoInitialize call remains on this thread",
        ));
    }
    if depth == 1 {
        drain()?;
    }
    MANAGED_APARTMENT_DEPTH.with(|state| state.set(depth - 1));
    unsafe { windows::Win32::System::WinRT::RoUninitialize() };
    Ok(())
}

fn leave_managed_apartment(py: Python<'_>, operation: &str) -> PyResult<()> {
    leave_managed_apartment_with_drain(operation, || {
        py.import("dynwinrt.dynwinrt")?
            .getattr("_dynwinrt_drain_apartment_owners")?
            .call0()?;
        Ok(())
    })
}

/// `apartment_type` used when Python omits it: the multithreaded apartment.
const DEFAULT_APARTMENT_TYPE: i32 = RO_INIT_MULTITHREADED.0;

/// Module constants naming the `apartment_type` values Python passes to
/// `RoApartment(...)` and `ro_initialize(...)`.
pub(crate) const APARTMENT_TYPE_CONSTANTS: [(&str, RO_INIT_TYPE); 2] = [
    ("RO_INIT_SINGLETHREADED", RO_INIT_SINGLETHREADED),
    ("RO_INIT_MULTITHREADED", RO_INIT_MULTITHREADED),
];

/// The `RoInitialize` model for a Python `apartment_type`. Values other than
/// `RO_INIT_SINGLETHREADED` keep their historical multithreaded meaning.
fn ro_init_type(apartment_type: i32) -> RO_INIT_TYPE {
    if apartment_type == RO_INIT_SINGLETHREADED.0 {
        RO_INIT_SINGLETHREADED
    } else {
        RO_INIT_MULTITHREADED
    }
}

impl RoApartment {
    fn enqueue_owner_recovery(&self) -> (bool, bool) {
        let (mut queue, poisoned) = lock_foreign_drops(&self.foreign_drops);
        if queue.owner_alive {
            queue.pending.push(PendingApartment {
                apartment_type: self.apartment_type,
                retry_cleanup: self.cleanup_failed,
            });
            (true, poisoned)
        } else {
            (false, poisoned)
        }
    }

    fn check_owner_thread(&self, operation: &str) -> PyResult<()> {
        if thread::current().id() != self.owner_thread {
            return Err(PyRuntimeError::new_err(format!(
                "{operation} must run on the OS thread where RoApartment was created; \
                 its initializing thread and apartment are unchanged. Retry on that owner thread, or recover a \
                 guard dropped on another thread with RoApartment.recover_pending()."
            )));
        }
        Ok(())
    }

    fn initialize(&mut self) -> PyResult<()> {
        self.check_owner_thread("RoApartment.__enter__()")?;
        if self.active {
            return Err(PyRuntimeError::new_err(
                "the COM apartment context is already active",
            ));
        }
        enter_managed_apartment(self.apartment_type)?;
        self.active = true;
        self.close_rejected = false;
        self.cleanup_failed = false;
        Ok(())
    }

    fn finish_with_drain(
        &mut self,
        operation: &str,
        drain: impl FnOnce() -> PyResult<()>,
    ) -> PyResult<()> {
        self.check_owner_thread(operation)?;
        if !self.active {
            return Ok(());
        }
        if let Err(error) = check_reentrant_uninitialize(operation) {
            self.close_rejected = true;
            return Err(error);
        }
        self.close_rejected = false;
        if let Err(error) = leave_managed_apartment_with_drain(operation, drain) {
            self.cleanup_failed = true;
            return Err(error);
        }
        self.active = false;
        self.cleanup_failed = false;
        Ok(())
    }

    fn finish(&mut self, py: Python<'_>, operation: &str) -> PyResult<()> {
        self.finish_with_drain(operation, || {
            py.import("dynwinrt.dynwinrt")?
                .getattr("_dynwinrt_drain_apartment_owners")?
                .call0()?;
            Ok(())
        })
    }

    #[cfg(test)]
    fn uninitialize(&mut self, operation: &str) -> PyResult<()> {
        self.finish_with_drain(operation, || Ok(()))
    }
}

impl Drop for RoApartment {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if thread::current().id() != self.owner_thread {
            let (queued, poisoned) = self.enqueue_owner_recovery();
            self.active = false;
            if poisoned {
                native_apartment_diagnostic(
                    "RoApartment owner-thread pending state was poisoned; its native \
                     initialization remains recoverable on the owner thread.",
                );
            }
            let message = if queued {
                "RoApartment was dropped on a different OS thread; its COM apartment remains \
                 active on the creating thread. Call RoApartment.recover_pending().close() \
                 there after native calls and retained references have been released."
            } else {
                "RoApartment was dropped after its owner OS thread exited; its COM apartment \
                 cannot be recovered on a different thread."
            };
            native_apartment_diagnostic(message);
            return;
        }
        let gil_usable = python_gil_usable();
        if !gil_usable {
            log_unsafe_apartment_teardown();
        }
        if !self.close_rejected
            && !self.cleanup_failed
            && gil_usable
            && check_reentrant_uninitialize("RoApartment.close()").is_ok()
            && Python::try_attach(|py| match self.finish(py, "RoApartment.close()") {
                Ok(()) => true,
                Err(error) => {
                    if !self.close_rejected {
                        error.write_unraisable(py, None);
                    }
                    false
                }
            }) == Some(true)
        {
            return;
        }
        self.active = false;
        if self.cleanup_failed {
            if PENDING_APARTMENT_CLOSES
                .try_with(|pending| {
                    pending
                        .try_borrow_mut()
                        .map(|mut pending| pending.push(self.apartment_type))
                })
                .is_ok_and(|result| result.is_ok())
            {
                return;
            }
        } else if CALLBACK_PENDING_APARTMENTS
            .try_with(|pending| pending.borrow_mut().push(self.apartment_type))
            .is_ok()
        {
            if !self.close_rejected {
                native_apartment_diagnostic(
                    "RoApartment was dropped during a synchronous native callback or without \
                     a usable Python GIL; its COM apartment remains active. Call \
                     RoApartment.recover_pending().close() on this thread after the native call.",
                );
            }
            return;
        }
        let (queued, poisoned) = self.enqueue_owner_recovery();
        if poisoned {
            native_apartment_diagnostic("RoApartment owner-thread pending queue was poisoned");
        }
        if queued {
            native_apartment_diagnostic(
                "RoApartment owner-thread pending state is unavailable; native initialization \
                 remains recoverable with RoApartment.recover_pending()",
            );
        } else {
            native_apartment_diagnostic(
                "RoApartment owner-thread state and recovery queue were unavailable during \
                 teardown; native uninitialization was not attempted",
            );
        }
    }
}

#[pymethods]
impl RoApartment {
    #[new]
    #[pyo3(signature = (apartment_type=None))]
    fn new(apartment_type: Option<i32>) -> PyResult<Self> {
        MANAGED_APARTMENT_DEPTH.try_with(|_| ()).map_err(|_| {
            PyRuntimeError::new_err(
                "RoApartment cannot be created after its owner-thread state was destroyed",
            )
        })?;
        let foreign_drops = FOREIGN_DROPS
            .try_with(|queue| queue.0.clone())
            .map_err(|_| {
                PyRuntimeError::new_err(
                    "RoApartment cannot be created after its owner-thread state was destroyed",
                )
            })?;
        Ok(Self {
            apartment_type: apartment_type.unwrap_or(DEFAULT_APARTMENT_TYPE),
            active: false,
            close_rejected: false,
            owner_thread: thread::current().id(),
            foreign_drops,
            cleanup_failed: false,
        })
    }

    fn __enter__(mut slf: PyRefMut<'_, Self>) -> PyResult<PyRefMut<'_, Self>> {
        slf.initialize()?;
        Ok(slf)
    }

    fn __exit__(
        &mut self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        let py = _exc_type.py();
        match self.finish(py, "RoApartment.__exit__()") {
            Ok(()) => Ok(false),
            Err(cleanup_error) if !_exc_value.is_none() => {
                py.import("dynwinrt.dynwinrt")?
                    .getattr("_dynwinrt_append_exception_cause")?
                    .call1((_exc_value, cleanup_error.value(py)))?;
                Ok(false)
            }
            Err(cleanup_error) => Err(cleanup_error),
        }
    }

    fn close(&mut self, py: Python<'_>) -> PyResult<()> {
        self.finish(py, "RoApartment.close()")
    }

    #[staticmethod]
    fn recover_pending() -> PyResult<Self> {
        let callback_active = NATIVE_CALLBACK_DEPTH
            .try_with(|depth| depth.get() != 0)
            .map_err(|_| {
                PyRuntimeError::new_err(
                    "owner-thread callback state is unavailable during teardown",
                )
            })?;
        if callback_active {
            return Err(PyRuntimeError::new_err(
                "recover_pending() must be called on the owner thread after the synchronous \
                 native callback and invocation return",
            ));
        }
        if PENDING_RETRY_IN_PROGRESS.try_with(Cell::get).map_err(|_| {
            PyRuntimeError::new_err("owner-thread pending state is unavailable during teardown")
        })? {
            return Err(PyRuntimeError::new_err(
                "an apartment cleanup retry is already in progress on this thread",
            ));
        }
        let foreign_drops = FOREIGN_DROPS
            .try_with(|queue| queue.0.clone())
            .map_err(|_| {
                PyRuntimeError::new_err(
                    "owner-thread apartment recovery queue is unavailable during teardown",
                )
            })?;
        let local = CALLBACK_PENDING_APARTMENTS
            .try_with(|state| -> PyResult<Option<i32>> {
                let mut state = state.try_borrow_mut().map_err(|_| {
                    PyRuntimeError::new_err("owner-thread apartment state is already in use")
                })?;
                Ok(state.pop())
            })
            .map_err(|_| {
                PyRuntimeError::new_err(
                    "owner-thread apartment state is unavailable during teardown",
                )
            })??;
        let pending = match local {
            Some(apartment_type) => PendingApartment {
                apartment_type,
                retry_cleanup: false,
            },
            None => {
                let (pending, poisoned) = {
                    let (mut queue, poisoned) = lock_foreign_drops(&foreign_drops);
                    (queue.pending.pop(), poisoned)
                };
                if poisoned {
                    native_apartment_diagnostic(
                        "RoApartment owner-thread pending state was poisoned; recovering its \
                         native initialization before any COM uninitialization.",
                    );
                }
                match pending {
                    Some(pending) => pending,
                    None => {
                        let cleanup_pending = PENDING_APARTMENT_CLOSES
                            .try_with(|state| {
                                state
                                    .try_borrow_mut()
                                    .map(|mut state| state.pop())
                                    .map_err(|_| {
                                        PyRuntimeError::new_err(
                                            "owner-thread pending state is already in use",
                                        )
                                    })
                            })
                            .map_err(|_| {
                                PyRuntimeError::new_err(
                                    "owner-thread pending state is unavailable during teardown",
                                )
                            })??
                            .ok_or_else(|| {
                                PyRuntimeError::new_err(
                                    "no dropped RoApartment is pending on this thread",
                                )
                            })?;
                        PendingApartment {
                            apartment_type: cleanup_pending,
                            retry_cleanup: true,
                        }
                    }
                }
            }
        };
        Ok(Self {
            apartment_type: pending.apartment_type,
            active: true,
            close_rejected: false,
            cleanup_failed: pending.retry_cleanup,
            owner_thread: thread::current().id(),
            foreign_drops,
        })
    }

    fn __repr__(&self) -> PyResult<String> {
        self.check_owner_thread("RoApartment.__repr__()")?;
        Ok(format!(
            "RoApartment(apartment_type={}, active={})",
            self.apartment_type, self.active
        ))
    }
}

#[pyfunction]
pub fn init_winappsdk(major: u32, minor: u32) -> PyResult<WinAppSDKContext> {
    dynwinrt::initialize_winappsdk(major, minor)
        .map(WinAppSDKContext)
        .map_err(map_dynwinrt_error)
}

#[pyfunction]
pub fn ro_initialize(apartment_type: Option<i32>) -> PyResult<()> {
    enter_managed_apartment(apartment_type.unwrap_or(DEFAULT_APARTMENT_TYPE))?;
    MANUAL_APARTMENT_DEPTH.with(|depth| depth.set(depth.get() + 1));
    Ok(())
}

#[pyfunction]
pub fn ro_uninitialize(py: Python<'_>) -> PyResult<()> {
    let depth = MANUAL_APARTMENT_DEPTH.with(Cell::get);
    if depth == 0 {
        return Err(PyRuntimeError::new_err(
            "ro_uninitialize() requires a successful ro_initialize() on this thread",
        ));
    }
    leave_managed_apartment(py, "ro_uninitialize()")?;
    MANUAL_APARTMENT_DEPTH.with(|state| state.set(depth - 1));
    Ok(())
}

#[pyfunction]
pub fn retry_pending_apartment_close(py: Python<'_>) -> PyResult<()> {
    let local_pending = PENDING_APARTMENT_CLOSES
        .try_with(|pending| {
            pending
                .try_borrow()
                .map(|pending| !pending.is_empty())
                .map_err(|_| {
                    PyRuntimeError::new_err("owner-thread pending state is already in use")
                })
        })
        .map_err(|_| {
            PyRuntimeError::new_err("owner-thread pending state is unavailable during teardown")
        })??;
    let foreign_pending = if !local_pending {
        FOREIGN_DROPS
            .try_with(|owner| {
                let (queue, poisoned) = lock_foreign_drops(&owner.0);
                if poisoned {
                    native_apartment_diagnostic(
                        "RoApartment owner-thread pending state was poisoned during cleanup retry",
                    );
                }
                queue.pending.iter().any(|pending| pending.retry_cleanup)
            })
            .map_err(|_| {
                PyRuntimeError::new_err(
                    "owner-thread apartment recovery queue is unavailable during teardown",
                )
            })?
    } else {
        false
    };
    if !local_pending && !foreign_pending {
        return Err(PyRuntimeError::new_err(
            "no failed RoApartment close is pending on this thread",
        ));
    }
    let _retry = PendingRetryGuard::enter()?;
    leave_managed_apartment(py, "retry_pending_apartment_close()")?;
    if local_pending {
        PENDING_APARTMENT_CLOSES.with(|pending| {
            pending
                .borrow_mut()
                .pop()
                .expect("pending cleanup was checked");
        });
    } else {
        FOREIGN_DROPS.with(|owner| {
            let (mut queue, _) = lock_foreign_drops(&owner.0);
            let index = queue
                .pending
                .iter()
                .rposition(|pending| pending.retry_cleanup)
                .expect("pending foreign cleanup was checked");
            queue.pending.remove(index);
        });
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn _managed_apartment_depth() -> usize {
    managed_apartment_depth()
}

// ======================================================================
// Process-local named XAML runtime classes
// ======================================================================

#[pyclass]
pub struct DynWinRTXamlRegistration {
    registration: Option<dynwinrt::XamlRuntimeClassRegistration>,
    instances: Arc<Mutex<Vec<Py<PyAny>>>>,
}

static XAML_INSTANCE_ROOTS: std::sync::LazyLock<Mutex<Vec<Arc<Mutex<Vec<Py<PyAny>>>>>>> =
    std::sync::LazyLock::new(|| Mutex::new(Vec::new()));

#[pymethods]
impl DynWinRTXamlRegistration {
    #[getter]
    fn name(&self) -> Option<String> {
        self.registration
            .as_ref()
            .map(|registration| registration.name().to_string())
    }

    #[getter]
    fn active(&self) -> bool {
        self.registration.is_some()
    }

    #[getter]
    fn supported_overrides(&self) -> Vec<String> {
        self.registration
            .as_ref()
            .map(|registration| registration.supported_overrides().to_vec())
            .unwrap_or_default()
    }

    fn unregister(&mut self) -> bool {
        self.registration
            .take()
            .is_some_and(|registration| registration.unregister())
    }

    fn release_instances(&self) -> PyResult<usize> {
        let instances = {
            let mut instances = self.instances.lock().map_err(|_| {
                PyRuntimeError::new_err("registered XAML instance state is poisoned")
            })?;
            std::mem::take(&mut *instances)
        };
        let count = instances.len();
        drop(instances);
        Ok(count)
    }

    fn close(&mut self) -> bool {
        self.unregister()
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __exit__(
        &mut self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> bool {
        self.unregister();
        false
    }
}

/// Register a Python constructor with the process-local XAML metadata provider.
///
/// The registration is only visible to `Application.create()` /
/// `create_xaml_application`; it never changes machine or package activation
/// metadata. The callback is apartment-bound to the registering thread.
#[pyfunction]
#[pyo3(signature = (runtime_class_name, base_type_name, base_iid, constructor, supported_overrides=None))]
pub fn register_xaml_runtime_class(
    py: Python<'_>,
    runtime_class_name: String,
    base_type_name: String,
    base_iid: &WinGUID,
    constructor: Py<PyAny>,
    supported_overrides: Option<Vec<String>>,
) -> PyResult<DynWinRTXamlRegistration> {
    ensure_python_callbacks_open()?;
    if !constructor.bind(py).is_callable() {
        return Err(PyRuntimeError::new_err(
            "register_xaml_runtime_class: constructor must be callable",
        ));
    }
    let context = py
        .import("contextvars")?
        .call_method0("copy_context")?
        .unbind();
    let callback = constructor.clone_ref(py);
    let callback_for_error = constructor.clone_ref(py);
    let thread_id = std::thread::current().id();
    let instances = Arc::new(Mutex::new(Vec::new()));
    XAML_INSTANCE_ROOTS
        .lock()
        .map_err(|_| PyRuntimeError::new_err("registered XAML root state is poisoned"))?
        .push(instances.clone());
    let active_instances = instances.clone();
    let activator: dynwinrt::XamlRuntimeClassActivator = Arc::new(move || {
        if std::thread::current().id() != thread_id {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x8001010Eu32 as i32,
            )));
        }
        with_python_callback(|py| {
            let result = (|| -> PyResult<IUnknown> {
                let invocation_context = context.call_method0(py, "copy")?;
                let instance = invocation_context.call_method1(py, "run", (callback.bind(py),))?;
                let native = match instance.getattr(py, "_obj") {
                    Ok(native) => native,
                    Err(error)
                        if error.is_instance_of::<pyo3::exceptions::PyAttributeError>(py) =>
                    {
                        instance.clone_ref(py)
                    }
                    Err(error) => return Err(error),
                };
                let value = native.extract::<PyRef<'_, DynWinRTValue>>(py)?;
                let object = value.0.as_object().ok_or_else(|| {
                    PyRuntimeError::new_err(
                        "registered XAML constructor must return an object with a DynWinRTValue '_obj'",
                    )
                })?;
                active_instances
                    .lock()
                    .map_err(|_| {
                        PyRuntimeError::new_err("registered XAML instance state is poisoned")
                    })?
                    .push(instance);
                Ok(object)
            })();
            match result {
                Ok(instance) => Ok(instance),
                Err(error) => {
                    error.write_unraisable(py, Some(callback_for_error.bind(py)));
                    Err(windows::core::Error::from_hresult(
                        PYWINRT_E_UNRAISABLE_PYTHON_EXCEPTION,
                    ))
                }
            }
        })
        .unwrap_or_else(|| {
            Err(windows::core::Error::from_hresult(
                PYWINRT_E_INTERPRETER_CLOSED,
            ))
        })
    });
    let registration = dynwinrt::register_xaml_runtime_class(
        &runtime_class_name,
        &base_type_name,
        base_iid.0,
        supported_overrides.unwrap_or_default(),
        activator,
    )
    .map_err(map_windows_error)?;
    Ok(DynWinRTXamlRegistration {
        registration: Some(registration),
        instances,
    })
}

// ======================================================================
// WinGUID
// ======================================================================

#[pyclass(from_py_object)]
#[derive(Debug, Clone, Copy)]
pub struct WinGUID(pub(crate) GUID);

#[pymethods]
impl WinGUID {
    #[staticmethod]
    fn parse(guid_str: &str) -> PyResult<Self> {
        let guid = GUID::try_from(guid_str)
            .map_err(|e| PyRuntimeError::new_err(format!("Invalid GUID: {:?}", e)))?;
        Ok(WinGUID(guid))
    }

    fn to_string(&self) -> String {
        format!("{:?}", self.0)
    }

    fn __repr__(&self) -> String {
        format!("WinGUID({:?})", self.0)
    }
}

// ======================================================================
// DynWinRTType — wraps TypeHandle
// ======================================================================

#[pyclass(from_py_object)]
#[derive(Clone)]
pub struct DynWinRTType(pub(crate) dynwinrt::TypeHandle);

#[pymethods]
impl DynWinRTType {
    // -- Primitive types --

    #[staticmethod]
    fn i32_type() -> Self {
        DynWinRTType(TABLE.i32_type())
    }
    #[staticmethod]
    fn i64_type() -> Self {
        DynWinRTType(TABLE.i64_type())
    }
    #[staticmethod]
    fn hstring() -> Self {
        DynWinRTType(TABLE.hstring())
    }
    #[staticmethod]
    fn object() -> Self {
        DynWinRTType(TABLE.object())
    }
    #[staticmethod]
    fn f64_type() -> Self {
        DynWinRTType(TABLE.f64_type())
    }
    #[staticmethod]
    fn f32_type() -> Self {
        DynWinRTType(TABLE.f32_type())
    }
    #[staticmethod]
    fn u8_type() -> Self {
        DynWinRTType(TABLE.u8_type())
    }
    #[staticmethod]
    fn u16_type() -> Self {
        DynWinRTType(TABLE.u16_type())
    }
    #[staticmethod]
    fn u32_type() -> Self {
        DynWinRTType(TABLE.u32_type())
    }
    #[staticmethod]
    fn u64_type() -> Self {
        DynWinRTType(TABLE.u64_type())
    }
    #[staticmethod]
    fn i8_type() -> Self {
        DynWinRTType(TABLE.i8_type())
    }
    #[staticmethod]
    fn i16_type() -> Self {
        DynWinRTType(TABLE.i16_type())
    }
    #[staticmethod]
    fn bool_type() -> Self {
        DynWinRTType(TABLE.bool_type())
    }
    #[staticmethod]
    fn guid_type() -> Self {
        DynWinRTType(TABLE.guid_type())
    }
    #[staticmethod]
    fn char16() -> Self {
        DynWinRTType(TABLE.char16_type())
    }
    #[staticmethod]
    fn hresult() -> Self {
        DynWinRTType(TABLE.hresult())
    }

    // -- Class / interface types --

    #[staticmethod]
    fn runtime_class(name: String, default_interface_type: &DynWinRTType) -> Self {
        DynWinRTType(TABLE.runtime_class(name, &default_interface_type.0))
    }

    #[staticmethod]
    fn interface(iid: &WinGUID) -> Self {
        DynWinRTType(TABLE.interface(iid.0))
    }

    #[staticmethod]
    fn delegate(iid: &WinGUID) -> Self {
        DynWinRTType(TABLE.delegate(iid.0))
    }

    // -- Async types --

    #[staticmethod]
    fn i_async_action() -> Self {
        DynWinRTType(TABLE.async_action())
    }

    #[staticmethod]
    fn i_async_action_with_progress(progress_type: &DynWinRTType) -> Self {
        DynWinRTType(TABLE.async_action_with_progress(&progress_type.0))
    }

    #[staticmethod]
    fn i_async_operation(result_type: &DynWinRTType) -> Self {
        DynWinRTType(TABLE.async_operation(&result_type.0))
    }

    #[staticmethod]
    fn i_async_operation_with_progress(
        result_type: &DynWinRTType,
        progress_type: &DynWinRTType,
    ) -> Self {
        DynWinRTType(TABLE.async_operation_with_progress(&result_type.0, &progress_type.0))
    }

    // -- Composite types --

    #[staticmethod]
    fn struct_type(name: String, fields: Vec<DynWinRTType>) -> Self {
        let handles: Vec<dynwinrt::TypeHandle> = fields.iter().map(|f| f.0.clone()).collect();
        DynWinRTType(TABLE.struct_type(&name, &handles))
    }

    #[staticmethod]
    #[pyo3(signature = (name, member_names=None, member_values=None, underlying_type=None))]
    fn enum_type(
        name: String,
        member_names: Option<Vec<String>>,
        member_values: Option<Vec<i64>>,
        underlying_type: Option<&DynWinRTType>,
    ) -> PyResult<Self> {
        let underlying = underlying_type.map_or(dynwinrt::TypeKind::I32, |typ| typ.0.kind());
        if !matches!(
            underlying,
            dynwinrt::TypeKind::I32 | dynwinrt::TypeKind::U32
        ) {
            return Err(PyTypeError::new_err(
                "enum_type requires an i32 or u32 backing type",
            ));
        }
        let members = match (member_names, member_values) {
            (None, None) => Vec::new(),
            (Some(names), Some(values)) if names.len() == values.len() => names
                .into_iter()
                .zip(values)
                .map(|(name, value)| {
                    let bits = if underlying == dynwinrt::TypeKind::U32 {
                        u32::try_from(value).ok().map(|value| value as i32)
                    } else {
                        i32::try_from(value).ok()
                    }
                    .ok_or_else(|| {
                        PyOverflowError::new_err(format!(
                            "enum_type member {name}: {value} does not fit {underlying:?}"
                        ))
                    })?;
                    Ok((name, bits))
                })
                .collect::<PyResult<Vec<_>>>()?,
            _ => {
                return Err(PyTypeError::new_err(
                    "enum_type requires equally sized member name and value arrays",
                ));
            }
        };
        TABLE
            .enum_type_with_underlying(&name, members, underlying)
            .map(DynWinRTType)
            .map_err(map_dynwinrt_error)
    }

    /// Look up an enum member's signed or unsigned numeric value by name.
    #[staticmethod]
    fn get_enum_value(enum_name: String, member_name: String) -> Option<i64> {
        TABLE.get_enum_value_i64(&enum_name, &member_name)
    }

    #[staticmethod]
    fn parameterized(generic_iid: &WinGUID, args: Vec<DynWinRTType>) -> Self {
        let handles: Vec<dynwinrt::TypeHandle> = args.iter().map(|a| a.0.clone()).collect();
        let generic = TABLE.generic(generic_iid.0, handles.len() as u32);
        DynWinRTType(TABLE.parameterized(&generic, &handles))
    }

    #[staticmethod]
    fn array_type(element_type: &DynWinRTType) -> Self {
        DynWinRTType(TABLE.array(&element_type.0))
    }

    // -- Interface registration & method management --

    #[staticmethod]
    fn register_interface(name: String, iid: &WinGUID) -> Self {
        DynWinRTType(TABLE.register_interface(&name, iid.0))
    }

    /// Add a method to this interface. Returns new DynWinRTType for chaining.
    fn add_method(&self, name: String, sig: &DynWinRTMethodSig) -> DynWinRTType {
        DynWinRTType(self.0.clone().add_method(&name, sig.0.clone()))
    }

    /// Get a MethodHandle by vtable index (6 = first user method).
    fn method(&self, vtable_index: usize) -> PyResult<DynWinRTMethodHandle> {
        self.0
            .method(vtable_index)
            .map(DynWinRTMethodHandle)
            .ok_or_else(|| {
                PyRuntimeError::new_err(format!("No method at vtable index {}", vtable_index))
            })
    }

    /// Get a MethodHandle by method name.
    fn method_by_name(&self, name: &str) -> PyResult<DynWinRTMethodHandle> {
        self.0
            .method_by_name(name)
            .map(DynWinRTMethodHandle)
            .ok_or_else(|| PyRuntimeError::new_err(format!("Method '{}' not found", name)))
    }

    /// Compute the IID for this type.
    fn iid(&self) -> PyResult<WinGUID> {
        self.0
            .iid()
            .map(WinGUID)
            .ok_or_else(|| PyRuntimeError::new_err("Type has no IID"))
    }

    fn __repr__(&self) -> String {
        format!("DynWinRTType({:?})", self.0.kind())
    }
}

// ======================================================================
// DynWinRTMethodSig — builder for method parameter descriptions
// ======================================================================

#[pyclass(from_py_object)]
#[derive(Clone)]
pub struct DynWinRTMethodSig(pub(crate) dynwinrt::MethodSignature);

#[pymethods]
impl DynWinRTMethodSig {
    #[new]
    fn new() -> Self {
        DynWinRTMethodSig(dynwinrt::MethodSignature::new(&*TABLE))
    }

    /// Add an [in] parameter. Returns new sig for chaining.
    fn add_in(&self, typ: &DynWinRTType) -> DynWinRTMethodSig {
        DynWinRTMethodSig(self.0.clone().add_in(typ.0.clone()))
    }

    /// Add an [out] parameter. Returns new sig for chaining.
    fn add_out(&self, typ: &DynWinRTType) -> DynWinRTMethodSig {
        DynWinRTMethodSig(self.0.clone().add_out(typ.0.clone()))
    }

    /// Add a FillArray [out] parameter. Returns new sig for chaining.
    fn add_out_fill(&self, typ: &DynWinRTType) -> DynWinRTMethodSig {
        DynWinRTMethodSig(self.0.clone().add_out_fill(typ.0.clone()))
    }
}

// ======================================================================
// DynWinRTMethodHandle — method invocation wrapper
// ======================================================================

#[pyclass]
pub struct DynWinRTMethodHandle(dynwinrt::MethodHandle);

#[pyclass]
pub struct DynWinRTOverrideInterface {
    iid: GUID,
    methods: Vec<dynwinrt::LocalOverrideAbi>,
    callbacks: Vec<(usize, Py<PyAny>)>,
    context: Py<PyAny>,
    thread_id: std::thread::ThreadId,
}

impl DynWinRTOverrideInterface {
    fn to_core(&self, py: Python<'_>) -> PyResult<dynwinrt::LocalOverrideInterface> {
        let mut interface = dynwinrt::LocalOverrideInterface::new(self.iid, self.methods.clone())
            .map_err(map_windows_error)?;
        for (vtable_index, callback) in &self.callbacks {
            let callback = callback.clone_ref(py);
            let context = self.context.clone_ref(py);
            let thread_id = self.thread_id;
            let method_index = vtable_index - 6;
            match self.methods[method_index] {
                dynwinrt::LocalOverrideAbi::Void0 => {
                    let callback = Arc::new(move || {
                        if std::thread::current().id() != thread_id {
                            return windows::core::HRESULT(0x8001010Eu32 as i32);
                        }
                        with_python_callback(|py| {
                            let result = (|| -> PyResult<()> {
                                let invocation_context = context.call_method0(py, "copy")?;
                                invocation_context.call_method1(py, "run", (callback.bind(py),))?;
                                Ok(())
                            })();
                            match result {
                                Ok(()) => windows::core::HRESULT(0),
                                Err(error) => {
                                    error.write_unraisable(py, Some(callback.bind(py)));
                                    PYWINRT_E_UNRAISABLE_PYTHON_EXCEPTION
                                }
                            }
                        })
                        .unwrap_or(PYWINRT_E_INTERPRETER_CLOSED)
                    });
                    interface = interface
                        .with_void_callback(*vtable_index, callback)
                        .map_err(map_windows_error)?;
                }
                dynwinrt::LocalOverrideAbi::SizeF32ToSizeF32 => {
                    let callback = Arc::new(
                        move |width: f32,
                              height: f32,
                              result_width: &mut f32,
                              result_height: &mut f32| {
                            if std::thread::current().id() != thread_id {
                                return windows::core::HRESULT(0x8001010Eu32 as i32);
                            }
                            with_python_callback(|py| {
                                let result = (|| -> PyResult<(f32, f32)> {
                                    let invocation_context = context.call_method0(py, "copy")?;
                                    let result = invocation_context.call_method1(
                                        py,
                                        "run",
                                        (callback.bind(py), (width, height)),
                                    )?;
                                    if let Ok(size) = result.extract::<(f32, f32)>(py) {
                                        return Ok(size);
                                    }
                                    let bound = result.bind(py);
                                    Ok((
                                        bound.getattr("width")?.extract()?,
                                        bound.getattr("height")?.extract()?,
                                    ))
                                })();
                                match result {
                                    Ok((width, height)) => {
                                        *result_width = width;
                                        *result_height = height;
                                        windows::core::HRESULT(0)
                                    }
                                    Err(error) => {
                                        error.write_unraisable(py, Some(callback.bind(py)));
                                        PYWINRT_E_UNRAISABLE_PYTHON_EXCEPTION
                                    }
                                }
                            })
                            .unwrap_or(PYWINRT_E_INTERPRETER_CLOSED)
                        },
                    );
                    interface = interface
                        .with_size_callback(*vtable_index, callback)
                        .map_err(map_windows_error)?;
                }
                dynwinrt::LocalOverrideAbi::HStringBoolToBool => {
                    unreachable!("constructor rejects callbacks for unsupported ABI shapes")
                }
            }
        }
        Ok(interface)
    }
}

#[pymethods]
impl DynWinRTOverrideInterface {
    #[new]
    fn new(
        py: Python<'_>,
        iid: &WinGUID,
        abi_shapes: Vec<String>,
        callbacks: &Bound<'_, PyDict>,
    ) -> PyResult<Self> {
        ensure_python_callbacks_open()?;
        let methods = abi_shapes
            .iter()
            .enumerate()
            .map(|(index, shape)| match shape.as_str() {
                "void0" => Ok(dynwinrt::LocalOverrideAbi::Void0),
                "size_f32_to_size_f32" => Ok(dynwinrt::LocalOverrideAbi::SizeF32ToSizeF32),
                "hstring_bool_to_bool" => Ok(dynwinrt::LocalOverrideAbi::HStringBoolToBool),
                _ => Err(PyRuntimeError::new_err(format!(
                    "unsupported native override ABI shape '{shape}' at vtable index {}",
                    index + 6
                ))),
            })
            .collect::<PyResult<Vec<_>>>()?;
        if methods.is_empty() {
            return Err(PyRuntimeError::new_err(
                "native override interface must contain at least one method",
            ));
        }

        let mut captured = Vec::with_capacity(callbacks.len());
        for (key, value) in callbacks.iter() {
            let vtable_index = key.extract::<usize>().map_err(|_| {
                PyRuntimeError::new_err("native override callback keys must be vtable indexes")
            })?;
            let Some(method_index) = vtable_index.checked_sub(6) else {
                return Err(PyRuntimeError::new_err(format!(
                    "native override vtable index {vtable_index} is below IInspectable slot 6"
                )));
            };
            if !matches!(
                methods.get(method_index),
                Some(
                    dynwinrt::LocalOverrideAbi::Void0
                        | dynwinrt::LocalOverrideAbi::SizeF32ToSizeF32
                )
            ) {
                return Err(PyRuntimeError::new_err(format!(
                    "native override callback at vtable index {vtable_index} has an unsupported ABI shape"
                )));
            }
            if !value.is_callable() {
                return Err(PyRuntimeError::new_err(format!(
                    "native override callback at vtable index {vtable_index} is not callable"
                )));
            }
            captured.push((vtable_index, value.unbind()));
        }
        captured.sort_by_key(|(index, _)| *index);
        let context = py
            .import("contextvars")?
            .call_method0("copy_context")?
            .unbind();
        Ok(Self {
            iid: iid.0,
            methods,
            callbacks: captured,
            context,
            thread_id: std::thread::current().id(),
        })
    }
}

fn pin_method_receiver(
    py: Python<'_>,
    obj: &Py<DynWinRTValue>,
    operation: &str,
    accepts_async: bool,
) -> PyResult<IUnknown> {
    let obj = obj.try_borrow(py)?;
    if accepts_async {
        obj.com_receiver(operation)
    } else {
        obj.receiver(operation).cloned()
    }
}

#[pymethods]
impl DynWinRTMethodHandle {
    /// Invoke this method on a COM object.
    fn invoke(
        &self,
        py: Python<'_>,
        obj: Py<DynWinRTValue>,
        args: Vec<Py<DynWinRTValue>>,
    ) -> PyResult<Py<DynWinRTValue>> {
        let object = pin_method_receiver(py, &obj, "invoke()", false)?;
        let wrt_args = native_arguments(py, "invoke()", args)?;
        let results = self
            .0
            .invoke(object.as_raw(), &wrt_args)
            .map_err(map_dynwinrt_error)?;
        let value = results
            .into_iter()
            .next()
            .unwrap_or(dynwinrt::WinRTValue::I32(0));
        tracked_native_value(py, value)
    }

    /// Invoke a blocking method on the current native thread while releasing
    /// the Python GIL. WinRT callbacks can reacquire it through Python::attach.
    fn invoke_detached(
        &self,
        py: Python<'_>,
        obj: Py<DynWinRTValue>,
        args: Vec<Py<DynWinRTValue>>,
    ) -> PyResult<Py<DynWinRTValue>> {
        struct SameThreadCall {
            method: dynwinrt::MethodHandle,
            object: IUnknown,
            args: Vec<dynwinrt::WinRTValue>,
        }
        struct SameThreadResult(dynwinrt::Result<Vec<dynwinrt::WinRTValue>>);

        impl SameThreadCall {
            fn run(self) -> SameThreadResult {
                SameThreadResult(self.method.invoke(self.object.as_raw(), &self.args))
            }
        }

        // PyO3's stable Ungil approximation requires Send, but Python::detach
        // executes this closure synchronously on the current OS thread. These
        // wrappers never cross an apartment or thread; they only cross the GIL
        // boundary and return before this method continues.
        unsafe impl Send for SameThreadCall {}
        unsafe impl Send for SameThreadResult {}

        // Validate a borrowed Python handle before cloning the native pin.
        // Only the pin and validated arguments cross the detached GIL boundary.
        let object = pin_method_receiver(py, &obj, "invoke_detached()", false)?;
        let call = SameThreadCall {
            method: self.0.clone(),
            object,
            args: native_arguments(py, "invoke_detached()", args)?,
        };
        let results = py
            .detach(move || call.run())
            .0
            .map_err(map_dynwinrt_error)?;
        let value = results
            .into_iter()
            .next()
            .unwrap_or(dynwinrt::WinRTValue::I32(0));
        tracked_native_value(py, value)
    }

    /// Like `invoke`, but returns all out-parameters as a list.
    /// Used for methods with multiple out params (e.g. IVector.IndexOf → [index, found]).
    fn invoke_all(
        &self,
        py: Python<'_>,
        obj: Py<DynWinRTValue>,
        args: Vec<Py<DynWinRTValue>>,
    ) -> PyResult<Vec<Py<DynWinRTValue>>> {
        let object = pin_method_receiver(py, &obj, "invoke_all()", false)?;
        let wrt_args = native_arguments(py, "invoke_all()", args)?;
        let results = self
            .0
            .invoke(object.as_raw(), &wrt_args)
            .map_err(map_dynwinrt_error)?;
        results
            .into_iter()
            .map(|value| tracked_native_value(py, value))
            .collect()
    }

    /// Invoke a WinRT composable factory with a runtime-provided outer host.
    fn invoke_composed(
        &self,
        py: Python<'_>,
        factory: &DynWinRTValue,
        args: Vec<Py<DynWinRTValue>>,
        outer_index: usize,
        inner_output_index: usize,
        instance_output_index: usize,
        agile: bool,
    ) -> PyResult<Py<DynWinRTValue>> {
        let factory = factory.com_receiver("invoke_composed() factory")?;
        let args = native_arguments(py, "invoke_composed()", args)?;
        dynwinrt::compose_winrt(
            &factory,
            &self.0,
            &args,
            outer_index,
            inner_output_index,
            instance_output_index,
            agile,
        )
        .map_err(map_dynwinrt_error)
        .and_then(|value| tracked_native_value(py, value))
    }

    /// Invoke a composable factory with metadata-described local overrides.
    #[allow(clippy::too_many_arguments)]
    fn invoke_composed_with_overrides(
        &self,
        py: Python<'_>,
        factory: &DynWinRTValue,
        args: Vec<Py<DynWinRTValue>>,
        outer_index: usize,
        inner_output_index: usize,
        instance_output_index: usize,
        agile: bool,
        override_interfaces: Vec<PyRef<'_, DynWinRTOverrideInterface>>,
    ) -> PyResult<Py<DynWinRTValue>> {
        if override_interfaces.is_empty() {
            return self.invoke_composed(
                py,
                factory,
                args,
                outer_index,
                inner_output_index,
                instance_output_index,
                agile,
            );
        }
        let factory = factory.com_receiver("invoke_composed_with_overrides() factory")?;
        let args = native_arguments(py, "invoke_composed_with_overrides()", args)?;
        let overrides = override_interfaces
            .iter()
            .map(|interface| interface.to_core(py))
            .collect::<PyResult<Vec<_>>>()?;
        dynwinrt::compose_winrt_with_overrides(
            &factory,
            &self.0,
            &args,
            outer_index,
            inner_output_index,
            instance_output_index,
            agile,
            overrides,
        )
        .map_err(map_dynwinrt_error)
        .and_then(|value| tracked_native_value(py, value))
    }

    // --- Fast paths: skip Vec alloc for common getter patterns ---

    /// Getter → string (0 args, zero Vec allocation)
    fn get_string(&self, py: Python<'_>, obj: Py<DynWinRTValue>) -> PyResult<String> {
        let object = pin_method_receiver(py, &obj, "get_string()", true)?;
        let hs = self
            .0
            .call_getter_hstring(object.as_raw())
            .map_err(map_dynwinrt_error)?;
        Ok(hs.to_string())
    }

    /// Getter → i32 (0 args, zero Vec allocation)
    fn get_i32(&self, py: Python<'_>, obj: Py<DynWinRTValue>) -> PyResult<i32> {
        let object = pin_method_receiver(py, &obj, "get_i32()", true)?;
        self.0
            .call_getter_i32(object.as_raw())
            .map_err(map_dynwinrt_error)
    }

    /// Getter → bool (0 args, zero Vec allocation)
    fn get_bool(&self, py: Python<'_>, obj: Py<DynWinRTValue>) -> PyResult<bool> {
        let object = pin_method_receiver(py, &obj, "get_bool()", true)?;
        self.0
            .call_getter_bool(object.as_raw())
            .map_err(map_dynwinrt_error)
    }

    /// Getter → DynWinRTValue object (0 args, zero Vec allocation)
    fn get_obj(&self, py: Python<'_>, obj: Py<DynWinRTValue>) -> PyResult<Py<DynWinRTValue>> {
        let object = pin_method_receiver(py, &obj, "get_obj()", true)?;
        self.0
            .call_getter_object(object.as_raw())
            .map_err(map_dynwinrt_error)
            .and_then(|value| tracked_native_value(py, value))
    }

    /// 1-arg invoke with hstring input → DynWinRTValue result
    fn invoke_hstring(
        &self,
        py: Python<'_>,
        obj: Py<DynWinRTValue>,
        arg: String,
    ) -> PyResult<Py<DynWinRTValue>> {
        let object = pin_method_receiver(py, &obj, "invoke_hstring()", true)?;
        let results = self
            .0
            .invoke(
                object.as_raw(),
                &[dynwinrt::WinRTValue::HString(HSTRING::from(arg))],
            )
            .map_err(map_dynwinrt_error)?;
        tracked_native_value(
            py,
            results
                .into_iter()
                .next()
                .ok_or_else(|| PyRuntimeError::new_err("invoke_hstring: no result"))?,
        )
    }

    /// 1-arg invoke with i32 input → DynWinRTValue result
    fn invoke_i32(
        &self,
        py: Python<'_>,
        obj: Py<DynWinRTValue>,
        arg: i32,
    ) -> PyResult<Py<DynWinRTValue>> {
        let object = pin_method_receiver(py, &obj, "invoke_i32()", true)?;
        let results = self
            .0
            .invoke(object.as_raw(), &[dynwinrt::WinRTValue::I32(arg)])
            .map_err(map_dynwinrt_error)?;
        tracked_native_value(
            py,
            results
                .into_iter()
                .next()
                .ok_or_else(|| PyRuntimeError::new_err("invoke_i32: no result"))?,
        )
    }
}

// ======================================================================
// DynWinRTValue — main value container
// ======================================================================

#[pyclass(weakref, skip_from_py_object)]
#[derive(Clone)]
pub struct DynWinRTValue(
    pub(crate) dynwinrt::WinRTValue,
    Lifecycle,
    Option<ThreadId>,
    bool,
);

impl Drop for DynWinRTValue {
    fn drop(&mut self) {
        if matches!(self.1, Lifecycle::Live)
            && self.0.contains_com_references()
            && must_quarantine_owner(self.2, self.3)
        {
            std::mem::forget(std::mem::replace(&mut self.0, dynwinrt::WinRTValue::Null));
            log_unsafe_native_owner_drop();
        }
    }
}

static TRACK_NATIVE: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
static TRACK_APARTMENT_CALLBACK_COPY: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

pub(crate) fn init_native_tracking(module: &Bound<'_, PyModule>) -> PyResult<()> {
    PYTHON_SHUTTING_DOWN.store(false, Ordering::Release);
    TRACK_NATIVE.get_or_try_init(module.py(), || {
        Ok::<Py<PyAny>, PyErr>(module.getattr("_dynwinrt_track_native")?.unbind())
    })?;
    TRACK_APARTMENT_CALLBACK_COPY.get_or_try_init(module.py(), || {
        Ok::<Py<PyAny>, PyErr>(
            module
                .getattr("_dynwinrt_track_apartment_callback_copy")?
                .unbind(),
        )
    })?;
    Ok(())
}

pub(crate) fn track_native_owner(py: Python<'_>, owner: Py<PyAny>) -> PyResult<()> {
    if let Some(track) = TRACK_NATIVE.get(py) {
        track.call1(py, (owner,))?;
    }
    Ok(())
}

/// Keep native COM ownership on the creating thread until the active lifetime
/// scope closes. Python retains the exact returned value, not an extra AddRef.
pub(crate) fn tracked_native_value(
    py: Python<'_>,
    value: dynwinrt::WinRTValue,
) -> PyResult<Py<DynWinRTValue>> {
    tracked_native_value_with_policy(py, value, false)
}

pub(crate) fn tracked_native_value_with_policy(
    py: Python<'_>,
    value: dynwinrt::WinRTValue,
    release_any_thread: bool,
) -> PyResult<Py<DynWinRTValue>> {
    let owns_native = value.contains_com_references();
    let agile = release_any_thread || (owns_native && native_value_is_agile(&value)?);
    let output = Py::new(py, DynWinRTValue::new_managed(value, agile))?;
    if owns_native {
        track_native_owner(py, output.clone_ref(py).into_any())?;
    }
    Ok(output)
}

fn native_array_is_agile(array: &dynwinrt::ArrayData) -> PyResult<bool> {
    for index in 0..array.len() {
        if !native_value_is_agile(&array.try_get(index).map_err(map_dynwinrt_error)?)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn native_struct_is_agile(data: &dynwinrt::ValueTypeData) -> PyResult<bool> {
    for index in 0..data.type_handle().field_count() {
        let kind = data.field_kind_checked(index).map_err(map_dynwinrt_error)?;
        if kind.is_com_pointer() {
            if let Some(object) = data.get_field_object(index).map_err(map_dynwinrt_error)?
                && object.cast::<windows::core::imp::IAgileObject>().is_err()
            {
                return Ok(false);
            }
        } else if matches!(kind, dynwinrt::TypeKind::Struct(_)) {
            let nested = data
                .get_field_struct_checked(index)
                .map_err(map_dynwinrt_error)?;
            if !native_struct_is_agile(&nested)? {
                return Ok(false);
            }
        } else if data
            .type_handle()
            .field_type(index)
            .contains_com_references()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn native_value_is_agile(value: &dynwinrt::WinRTValue) -> PyResult<bool> {
    match value {
        dynwinrt::WinRTValue::Object(object) => {
            Ok(object.cast::<windows::core::imp::IAgileObject>().is_ok())
        }
        dynwinrt::WinRTValue::Async(info) => {
            Ok(info.info.cast::<windows::core::imp::IAgileObject>().is_ok())
        }
        dynwinrt::WinRTValue::ArrayOfIUnknown(values) => Ok((0..values.0.len()).all(|index| {
            values.0[index]
                .as_ref()
                .is_none_or(|object| object.cast::<windows::core::imp::IAgileObject>().is_ok())
        })),
        dynwinrt::WinRTValue::Array(array) => native_array_is_agile(array),
        dynwinrt::WinRTValue::Struct(data) => native_struct_is_agile(data),
        _ => Ok(true),
    }
}

pub(crate) fn callback_native_argument(
    py: Python<'_>,
    value: dynwinrt::WinRTValue,
) -> PyResult<Py<DynWinRTValue>> {
    if value.contains_com_references()
        && managed_apartment_depth() > 0
        && !native_value_is_agile(&value)?
    {
        let output = Py::new(py, DynWinRTValue::new_managed(value, false))?;
        let track = TRACK_APARTMENT_CALLBACK_COPY.get(py).ok_or_else(|| {
            PyRuntimeError::new_err("native callback lifetime tracker is missing")
        })?;
        track.call1(py, (output.clone_ref(py).into_any(),))?;
        return Ok(output);
    }
    Py::new(py, DynWinRTValue::new(value))
}

fn tracked_native_array(py: Python<'_>, array: dynwinrt::ArrayData) -> PyResult<Py<DynWinRTArray>> {
    let owns_com = array.contains_com_references();
    let agile = owns_com && native_array_is_agile(&array)?;
    let output = Py::new(
        py,
        DynWinRTArray(Some(array), current_native_owner_thread(owns_com), agile),
    )?;
    if owns_com {
        track_native_owner(py, output.clone_ref(py).into_any())?;
    }
    Ok(output)
}

fn tracked_native_struct(
    py: Python<'_>,
    data: dynwinrt::ValueTypeData,
) -> PyResult<Py<DynWinRTStruct>> {
    let owns_com = data.type_handle().contains_com_references();
    let agile = owns_com && native_struct_is_agile(&data)?;
    let output = Py::new(
        py,
        DynWinRTStruct(Some(data), current_native_owner_thread(owns_com), agile),
    )?;
    if owns_com {
        track_native_owner(py, output.clone_ref(py).into_any())?;
    }
    Ok(output)
}

/// Whether a value still owns its native payload. `release()` is the only
/// transition and leaves `WinRTValue::Null` behind, so this state is what
/// distinguishes a released value from a WinRT null reference.
#[derive(Clone, Copy)]
enum Lifecycle {
    Live,
    Released,
}

impl DynWinRTValue {
    pub(crate) fn new(value: dynwinrt::WinRTValue) -> Self {
        Self(value, Lifecycle::Live, None, false)
    }

    fn new_managed(value: dynwinrt::WinRTValue, release_any_thread: bool) -> Self {
        let owner = current_native_owner_thread(value.contains_com_references());
        Self(value, Lifecycle::Live, owner, release_any_thread)
    }

    /// The WinRT object receiving `operation`.
    fn receiver(&self, operation: &str) -> PyResult<&IUnknown> {
        self.ensure_live()?;
        match &self.0 {
            dynwinrt::WinRTValue::Object(object) => Ok(object),
            _ => Err(self.receiver_error(operation)),
        }
    }

    /// The COM identity receiving `operation`. Unlike `receiver`, this also
    /// accepts async operations, as the legacy convenience entry points do.
    fn com_receiver(&self, operation: &str) -> PyResult<IUnknown> {
        self.ensure_live()?;
        self.0
            .as_object()
            .ok_or_else(|| self.receiver_error(operation))
    }

    /// QueryInterface this value for `operation`.
    pub(crate) fn query(&self, iid: &GUID, operation: &str) -> PyResult<dynwinrt::WinRTValue> {
        self.ensure_live()?;
        self.0.cast(iid).map_err(|error| match error {
            dynwinrt::Error::ExpectObjectTypeError(_) => self.receiver_error(operation),
            error => map_dynwinrt_error(error),
        })
    }

    /// Why this value cannot receive `operation`.
    fn receiver_error(&self, operation: &str) -> PyErr {
        match self.ensure_live() {
            Err(error) => error,
            Ok(()) => non_object_receiver_error(operation, value_kind(&self.0)),
        }
    }

    /// Reject a released value before a native call that reports its own
    /// payload errors, such as IBuffer access.
    fn ensure_live(&self) -> PyResult<()> {
        match self.1 {
            Lifecycle::Live => ensure_native_access_thread(self.2, self.3, "DynWinRTValue"),
            Lifecycle::Released => Err(released_receiver_error()),
        }
    }

    /// Match a native runtime class only after confirming its class interface.
    /// Custom collections with the same generic IID need not provide a class
    /// name (or accept the stock class's element contract).
    fn matches_runtime_class(&self, iid: &GUID, name: &str) -> PyResult<bool> {
        let receiver = self.receiver("collection runtime-class check")?;
        let mut raw = std::ptr::null_mut();
        match unsafe { receiver.query(iid, &mut raw) }.ok() {
            Ok(()) => {
                let class_interface = unsafe { IUnknown::from_raw(raw) };
                let inspectable: IInspectable =
                    class_interface.cast().map_err(map_windows_error)?;
                let actual = inspectable
                    .GetRuntimeClassName()
                    .map_err(map_windows_error)?;
                Ok(actual == name)
            }
            Err(error) if error.code() == windows::Win32::Foundation::E_NOINTERFACE => Ok(false),
            Err(error) => Err(map_windows_error(error)),
        }
    }

    /// Reject this value if released; `slot` names where `operation` received it.
    pub(crate) fn check_input(&self, operation: &str, slot: InputSlot) -> PyResult<()> {
        match self.1 {
            Lifecycle::Live => ensure_native_access_thread(self.2, self.3, "DynWinRTValue"),
            Lifecycle::Released => Err(released_input_error(operation, slot)),
        }
    }
}

/// The native values `operation` received, rejecting released values. `slot`
/// maps each position to where it was passed, such as an argument or element.
fn native_inputs(
    py: Python<'_>,
    operation: &str,
    values: Vec<Py<DynWinRTValue>>,
    slot: fn(usize) -> InputSlot,
) -> PyResult<Vec<dynwinrt::WinRTValue>> {
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let value = value.try_borrow(py)?;
            value.check_input(operation, slot(index))?;
            Ok(value.0.clone())
        })
        .collect()
}

/// The native arguments of `operation`, rejecting released values.
pub(crate) fn native_arguments(
    py: Python<'_>,
    operation: &str,
    args: Vec<Py<DynWinRTValue>>,
) -> PyResult<Vec<dynwinrt::WinRTValue>> {
    native_inputs(py, operation, args, InputSlot::Argument)
}

/// The native values a Python `operation` callback returned, rejecting
/// released values instead of returning them as WinRT null.
pub(crate) fn native_outputs(
    py: Python<'_>,
    operation: &str,
    outputs: Vec<Py<DynWinRTValue>>,
) -> PyResult<Vec<dynwinrt::WinRTValue>> {
    native_inputs(py, operation, outputs, InputSlot::Output)
}

fn value_kind(value: &dynwinrt::WinRTValue) -> &'static str {
    use dynwinrt::WinRTValue;
    match value {
        WinRTValue::Bool(_) => "Bool",
        WinRTValue::I8(_) => "I8",
        WinRTValue::U8(_) => "U8",
        WinRTValue::I16(_) => "I16",
        WinRTValue::U16(_) => "U16",
        WinRTValue::I32(_) => "I32",
        WinRTValue::U32(_) => "U32",
        WinRTValue::I64(_) => "I64",
        WinRTValue::U64(_) => "U64",
        WinRTValue::F32(_) => "F32",
        WinRTValue::F64(_) => "F64",
        WinRTValue::Object(_) => "Object",
        WinRTValue::Null => "null",
        WinRTValue::HString(_) => "HString",
        WinRTValue::HResult(_) => "HResult",
        WinRTValue::Guid(_) => "Guid",
        WinRTValue::RawPtr(_) => "RawPtr",
        WinRTValue::OutValue(..) => "OutValue",
        WinRTValue::Async(_) => "Async",
        WinRTValue::ArrayOfIUnknown(_) => "ArrayOfIUnknown",
        WinRTValue::Enum { .. } => "Enum",
        WinRTValue::Struct(_) => "Struct",
        WinRTValue::Array(_) => "Array",
    }
}

#[pymethods]
impl DynWinRTValue {
    #[staticmethod]
    fn activation_factory(py: Python<'_>, name: String) -> PyResult<Py<DynWinRTValue>> {
        WINUI_MODULES
            .activation_factory(&HSTRING::from(name))
            .map_err(map_dynwinrt_error)
            .and_then(|value| tracked_native_value(py, value))
    }

    /// Create an owned WinRT IBuffer by copying Python bytes or bytearray data.
    #[staticmethod]
    fn from_bytes(py: Python<'_>, data: &Bound<'_, PyAny>) -> PyResult<Py<DynWinRTValue>> {
        let bytes = if let Ok(data) = data.cast::<pyo3::types::PyBytes>() {
            data.as_bytes().to_vec()
        } else if let Ok(data) = data.cast::<pyo3::types::PyByteArray>() {
            data.to_vec()
        } else {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "from_bytes: expected bytes or bytearray",
            ));
        };
        dynwinrt::copy_to_ibuffer(&bytes)
            .map_err(map_dynwinrt_error)
            .and_then(|value| tracked_native_value(py, value))
    }

    /// Compose a WinUI `Microsoft.UI.Xaml.Application` whose outer object
    /// exposes the supplied `IXamlMetadataProvider`.
    ///
    /// Mirrors JS `DynWinRtValue.createXamlApplication(metadataProvider, launchedCallback)`.
    #[staticmethod]
    #[pyo3(signature = (metadata_provider, launched_callback=None))]
    fn create_xaml_application(
        py: Python<'_>,
        metadata_provider: &DynWinRTValue,
        launched_callback: Option<&DynWinRTValue>,
    ) -> PyResult<Py<DynWinRTValue>> {
        metadata_provider.check_input("create_xaml_application()", InputSlot::Argument(0))?;
        let provider = metadata_provider.0.as_object().ok_or_else(|| {
            PyRuntimeError::new_err("create_xaml_application: metadata_provider must be an Object")
        })?;
        let callback = launched_callback
            .map(|value| {
                value.check_input("create_xaml_application()", InputSlot::Argument(1))?;
                value.0.as_object().ok_or_else(|| {
                    PyRuntimeError::new_err(
                        "create_xaml_application: launched_callback must be an Object",
                    )
                })
            })
            .transpose()?;
        WINUI_MODULES
            .create_xaml_application(&provider, callback.as_ref())
            .map_err(map_dynwinrt_error)
            .and_then(|value| tracked_native_value(py, value))
    }

    // -- Scalar constructors (full parity with JS) --

    #[staticmethod]
    fn from_bool(value: bool) -> DynWinRTValue {
        DynWinRTValue::new(dynwinrt::WinRTValue::Bool(value))
    }
    #[staticmethod]
    fn from_i8(value: i32) -> PyResult<DynWinRTValue> {
        Ok(DynWinRTValue::new(dynwinrt::WinRTValue::I8(checked_i8(
            value, "from_i8",
        )?)))
    }
    #[staticmethod]
    fn from_u8(value: u32) -> PyResult<DynWinRTValue> {
        Ok(DynWinRTValue::new(dynwinrt::WinRTValue::U8(checked_u8(
            value, "from_u8",
        )?)))
    }
    #[staticmethod]
    fn from_i16(value: i32) -> PyResult<DynWinRTValue> {
        Ok(DynWinRTValue::new(dynwinrt::WinRTValue::I16(checked_i16(
            value, "from_i16",
        )?)))
    }
    #[staticmethod]
    fn from_u16(value: u32) -> PyResult<DynWinRTValue> {
        Ok(DynWinRTValue::new(dynwinrt::WinRTValue::U16(checked_u16(
            value, "from_u16",
        )?)))
    }
    #[staticmethod]
    fn from_i32(value: i32) -> DynWinRTValue {
        DynWinRTValue::new(dynwinrt::WinRTValue::I32(value))
    }
    #[staticmethod]
    fn from_hresult(value: i32) -> DynWinRTValue {
        DynWinRTValue::new(dynwinrt::WinRTValue::HResult(windows::core::HRESULT(value)))
    }
    #[staticmethod]
    fn from_u32(value: u32) -> DynWinRTValue {
        DynWinRTValue::new(dynwinrt::WinRTValue::U32(value))
    }
    #[staticmethod]
    fn from_i64(value: i64) -> DynWinRTValue {
        DynWinRTValue::new(dynwinrt::WinRTValue::I64(value))
    }
    #[staticmethod]
    fn from_u64(value: u64) -> DynWinRTValue {
        DynWinRTValue::new(dynwinrt::WinRTValue::U64(value))
    }
    #[staticmethod]
    fn from_f32(value: f32) -> DynWinRTValue {
        DynWinRTValue::new(dynwinrt::WinRTValue::F32(value))
    }
    #[staticmethod]
    fn from_f64(value: f64) -> DynWinRTValue {
        DynWinRTValue::new(dynwinrt::WinRTValue::F64(value))
    }
    #[staticmethod]
    fn from_hstring(value: String) -> DynWinRTValue {
        DynWinRTValue::new(dynwinrt::WinRTValue::HString(HSTRING::from(value)))
    }
    #[staticmethod]
    fn from_guid(value: &WinGUID) -> DynWinRTValue {
        DynWinRTValue::new(dynwinrt::WinRTValue::Guid(value.0))
    }
    #[staticmethod]
    fn null_value() -> DynWinRTValue {
        DynWinRTValue::new(dynwinrt::WinRTValue::Null)
    }

    /// Create an enum value within the enum's declared i32 or u32 range.
    #[staticmethod]
    fn enum_value(enum_type: &DynWinRTType, value: i64) -> PyResult<DynWinRTValue> {
        enum_type
            .0
            .enum_value(value)
            .map(DynWinRTValue::new)
            .map_err(map_dynwinrt_error)
    }

    #[staticmethod]
    fn box_reference(
        py: Python<'_>,
        value: &DynWinRTValue,
        value_type: &DynWinRTType,
    ) -> PyResult<Py<DynWinRTValue>> {
        value.check_input("DynWinRTValue.box_reference()", InputSlot::Argument(0))?;
        dynwinrt::box_ireference(value.0.clone(), value_type.0.clone())
            .map_err(map_dynwinrt_error)
            .and_then(|value| tracked_native_value(py, value))
    }

    /// Get the signed or unsigned numeric value of an enum. Returns None if not an enum.
    fn get_enum_int(&self) -> Option<i64> {
        self.0.as_enum_number()
    }

    /// Get the member name of an enum value.
    fn get_enum_name(&self) -> Option<String> {
        match &self.0 {
            dynwinrt::WinRTValue::Enum { value, type_handle } => {
                type_handle.enum_member_name(*value)
            }
            _ => None,
        }
    }

    /// Create an IVector<T> from items.
    #[staticmethod]
    fn create_vector(
        py: Python<'_>,
        items: Vec<Py<DynWinRTValue>>,
        element_type: &DynWinRTType,
    ) -> PyResult<Py<DynWinRTValue>> {
        let wrt_items = native_inputs(
            py,
            "DynWinRTValue.create_vector()",
            items,
            InputSlot::Element,
        )?;
        let iids = TABLE.vector_iids(&element_type.0);
        let vector = dynwinrt::vector::create_vector_from_values(&wrt_items, &element_type.0, iids)
            .map_err(map_dynwinrt_error)?;
        tracked_native_value(py, dynwinrt::WinRTValue::Object(vector))
    }

    /// Create an IMap<K,V> from parallel key/value lists.
    #[staticmethod]
    fn create_map(
        py: Python<'_>,
        keys: Vec<Py<DynWinRTValue>>,
        values: Vec<Py<DynWinRTValue>>,
        key_type: &DynWinRTType,
        value_type: &DynWinRTType,
    ) -> PyResult<Py<DynWinRTValue>> {
        if keys.len() != values.len() {
            return Err(PyRuntimeError::new_err(
                "create_map: keys and values must have the same length",
            ));
        }
        const OPERATION: &str = "DynWinRTValue.create_map()";
        let keys = native_inputs(py, OPERATION, keys, InputSlot::Key)?;
        let values = native_inputs(py, OPERATION, values, InputSlot::Value)?;
        let iids = TABLE.map_iids(&key_type.0, &value_type.0);
        let entries: Vec<(dynwinrt::WinRTValue, dynwinrt::WinRTValue)> =
            keys.into_iter().zip(values).collect();
        let map = dynwinrt::map::create_map_from_values(&entries, &key_type.0, &value_type.0, iids)
            .map_err(map_dynwinrt_error)?;
        tracked_native_value(py, dynwinrt::WinRTValue::Object(map))
    }

    /// Await an async WinRT operation (blocks the current thread).
    /// Releases the Python GIL while waiting so other threads can proceed.
    fn wait(&self, py: Python<'_>) -> PyResult<Py<DynWinRTValue>> {
        tracked_native_value(py, super::async_runtime::wait_for_async(&self.0, py)?)
    }

    fn _get_async_results(&self, py: Python<'_>) -> PyResult<Py<DynWinRTValue>> {
        dynwinrt::get_async_results(&self.0)
            .map_err(map_dynwinrt_error)
            .and_then(|value| tracked_native_value(py, value))
    }

    fn _async_is_started(&self) -> PyResult<bool> {
        self.ensure_live()?;
        match &self.0 {
            dynwinrt::WinRTValue::Async(info) => info.is_started().map_err(map_dynwinrt_error),
            _ => Err(PyRuntimeError::new_err(
                "value is not a WinRT async operation",
            )),
        }
    }

    fn _check_apartment_release(&self) -> PyResult<()> {
        if let dynwinrt::WinRTValue::Async(info) = &self.0
            && info.is_started().map_err(map_dynwinrt_error)?
            && info
                .info
                .cast::<windows::core::imp::IAgileObject>()
                .is_err()
        {
            return Err(PyRuntimeError::new_err(
                "cannot close the COM apartment while a non-agile WinRT async reference is pending; settle it and retry on its owner thread",
            ));
        }
        Ok(())
    }

    /// Cancel the underlying WinRT async operation (calls `IAsyncInfo::Cancel`).
    /// Safe to call multiple times or on already-completed operations.
    ///
    /// Raises if this value is not an async operation.
    fn cancel(&self) -> PyResult<()> {
        let async_info = match &self.0 {
            dynwinrt::WinRTValue::Async(a) => a,
            _ => return Err(PyRuntimeError::new_err("cancel: not an async value")),
        };
        async_info
            .cancel()
            .map_err(|error| map_dynwinrt_error_with_context(error, "Cancel failed"))
    }

    /// Register a progress callback on an async-with-progress operation.
    fn on_progress(&self, py: Python<'_>, callback: Py<PyAny>) -> PyResult<()> {
        ensure_python_callbacks_open()?;
        let async_info = match &self.0 {
            dynwinrt::WinRTValue::Async(a) => a,
            _ => return Err(PyRuntimeError::new_err("on_progress: not an async value")),
        };
        let progress_type = async_info
            .progress_type()
            .ok_or_else(|| PyRuntimeError::new_err("on_progress: not a WithProgress async type"))?;
        super::async_runtime::ensure_progress_type_supported(&progress_type)?;
        if !async_info.is_started().map_err(map_dynwinrt_error)? {
            return Ok(());
        }

        let handler_iid = async_info.progress_handler_iid().ok_or_else(|| {
            PyRuntimeError::new_err("on_progress: cannot compute progress handler IID")
        })?;
        let callback_for_error = callback.clone_ref(py);
        let callback = wrap_python_callback_context(py, callback)?;

        let progress_cb: dynwinrt::ProgressCallback = Box::new(move |val: dynwinrt::WinRTValue| {
            let _ = with_python_callback(|py| {
                let result = (|| -> PyResult<()> {
                    let py_val = callback_native_argument(py, val)?;
                    callback.call1(py, (py_val,))?;
                    Ok(())
                })();
                if let Err(error) = result {
                    error.write_unraisable(py, Some(callback_for_error.bind(py)));
                }
            });
        });
        let handler =
            dynwinrt::try_create_progress_handler(handler_iid, progress_type, progress_cb)
                .map_err(|error| {
                    map_dynwinrt_error_with_context(
                        error,
                        "on_progress: failed to create progress handler",
                    )
                })?;

        super::async_runtime::finish_progress_registration(
            async_info.set_progress_handler(&handler),
            || async_info.is_started().map_err(map_dynwinrt_error),
        )
    }

    // -- Conversion methods --

    fn to_string(&self) -> String {
        match &self.0 {
            dynwinrt::WinRTValue::HString(s) => s.to_string(),
            dynwinrt::WinRTValue::I32(i) => i.to_string(),
            dynwinrt::WinRTValue::I64(i) => i.to_string(),
            dynwinrt::WinRTValue::U32(i) => i.to_string(),
            dynwinrt::WinRTValue::U64(i) => i.to_string(),
            dynwinrt::WinRTValue::F32(f) => f.to_string(),
            dynwinrt::WinRTValue::F64(f) => f.to_string(),
            dynwinrt::WinRTValue::Bool(b) => b.to_string(),
            dynwinrt::WinRTValue::Object(o) => format!("Object({:?})", o),
            dynwinrt::WinRTValue::Enum { value, type_handle } => {
                if let Some(name) = type_handle.enum_member_name(*value) {
                    name
                } else if type_handle.underlying_kind() == dynwinrt::TypeKind::U32 {
                    (*value as u32).to_string()
                } else {
                    value.to_string()
                }
            }
            _ => "Unsupported type".to_string(),
        }
    }

    fn __repr__(&self) -> String {
        format!("DynWinRTValue({})", self.to_string())
    }

    fn __str__(&self) -> String {
        self.to_string()
    }

    fn to_number(&self) -> PyResult<i64> {
        if let Some(value) = self.0.as_enum_number() {
            return Ok(value);
        }
        match &self.0 {
            dynwinrt::WinRTValue::Bool(b) => Ok(if *b { 1 } else { 0 }),
            dynwinrt::WinRTValue::I8(i) => Ok(*i as i64),
            dynwinrt::WinRTValue::U8(i) => Ok(*i as i64),
            dynwinrt::WinRTValue::I16(i) => Ok(*i as i64),
            dynwinrt::WinRTValue::U16(i) => Ok(*i as i64),
            dynwinrt::WinRTValue::I32(i) => Ok(*i as i64),
            dynwinrt::WinRTValue::U32(i) => Ok(*i as i64),
            dynwinrt::WinRTValue::HResult(hr) => Ok(hr.0 as i64),
            _ => Err(PyRuntimeError::new_err(format!(
                "Cannot convert {:?} to number",
                self.0.get_type_kind()
            ))),
        }
    }

    fn to_int(&self) -> PyResult<i128> {
        if let Some(value) = self.0.as_enum_number() {
            return Ok(i128::from(value));
        }
        match &self.0 {
            dynwinrt::WinRTValue::I32(i) => Ok(*i as i128),
            dynwinrt::WinRTValue::I64(i) => Ok(*i as i128),
            dynwinrt::WinRTValue::U32(i) => Ok(*i as i128),
            dynwinrt::WinRTValue::U64(i) => Ok(*i as i128),
            dynwinrt::WinRTValue::Bool(b) => Ok(*b as i128),
            dynwinrt::WinRTValue::I8(i) => Ok(*i as i128),
            dynwinrt::WinRTValue::U8(i) => Ok(*i as i128),
            dynwinrt::WinRTValue::I16(i) => Ok(*i as i128),
            dynwinrt::WinRTValue::U16(i) => Ok(*i as i128),
            _ => Err(PyRuntimeError::new_err("Cannot convert to int")),
        }
    }

    fn to_float(&self) -> PyResult<f64> {
        match &self.0 {
            dynwinrt::WinRTValue::F32(f) => Ok(*f as f64),
            dynwinrt::WinRTValue::F64(f) => Ok(*f),
            dynwinrt::WinRTValue::I32(i) => Ok(*i as f64),
            dynwinrt::WinRTValue::I64(i) => Ok(*i as f64),
            _ => Err(PyRuntimeError::new_err("Cannot convert to float")),
        }
    }

    fn to_bool(&self) -> PyResult<bool> {
        match &self.0 {
            dynwinrt::WinRTValue::Bool(b) => Ok(*b),
            _ => self.to_number().map(|n| n != 0),
        }
    }

    fn to_i64(&self) -> PyResult<i64> {
        match &self.0 {
            dynwinrt::WinRTValue::I64(i) => Ok(*i),
            dynwinrt::WinRTValue::U64(i) => i64::try_from(*i)
                .map_err(|_| PyRuntimeError::new_err("UInt64 value does not fit in Int64")),
            _ => self.to_number(),
        }
    }

    fn to_u32(&self) -> PyResult<u32> {
        match &self.0 {
            dynwinrt::WinRTValue::U32(i) => Ok(*i),
            _ => Err(PyRuntimeError::new_err("Value is not a UInt32")),
        }
    }

    fn to_u64(&self) -> PyResult<u64> {
        match &self.0 {
            dynwinrt::WinRTValue::U64(i) => Ok(*i),
            _ => Err(PyRuntimeError::new_err("Value is not a UInt64")),
        }
    }

    fn to_f64(&self) -> PyResult<f64> {
        match &self.0 {
            dynwinrt::WinRTValue::F64(f) => Ok(*f),
            dynwinrt::WinRTValue::F32(f) => Ok(*f as f64),
            _ => self.to_number().map(|n| n as f64),
        }
    }

    fn to_guid(&self) -> PyResult<WinGUID> {
        match &self.0 {
            dynwinrt::WinRTValue::Guid(g) => Ok(WinGUID(*g)),
            _ => Err(PyRuntimeError::new_err("Value is not a GUID")),
        }
    }

    /// Copy the initialized bytes from a WinRT IBuffer into Python bytes.
    fn to_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyBytes>> {
        self.ensure_live()?;
        dynwinrt::copy_from_ibuffer(&self.0)
            .map(|bytes| pyo3::types::PyBytes::new(py, &bytes))
            .map_err(map_dynwinrt_error)
    }

    fn is_null(&self) -> bool {
        self.0.is_null_object()
    }

    fn _matches_runtime_class(&self, iid: &WinGUID, name: &str) -> PyResult<bool> {
        self.matches_runtime_class(&iid.0, name)
    }

    /// Validate the receiver-specific native collection contract before any
    /// method call. An array may already contain nulls when supplied as a
    /// DynWinRTArray or a raw DynWinRTValue.
    fn _validate_non_null_collection_input(
        &self,
        py: Python<'_>,
        value: Py<DynWinRTValue>,
        iid: &WinGUID,
        name: &str,
    ) -> PyResult<Py<DynWinRTValue>> {
        let contains_null = {
            let value = value.try_borrow(py)?;
            value.check_input("collection input", InputSlot::Argument(0))?;
            match &value.0 {
                dynwinrt::WinRTValue::Array(data) => {
                    (0..data.len()).any(|index| data.get(index).is_null_object())
                }
                other => other.is_null_object(),
            }
        };
        if contains_null && self.matches_runtime_class(&iid.0, name)? {
            return Err(PyTypeError::new_err(format!(
                "{name} requires a non-null IJsonValue; use JsonValue.create_null_value() for JSON null"
            )));
        }
        let (native, release_any_thread) = {
            let value = value.try_borrow(py)?;
            value.check_input("collection input", InputSlot::Argument(0))?;
            (value.0.clone(), value.3)
        };
        tracked_native_value_with_policy(py, native, release_any_thread)
    }

    /// Guard-only QueryInterface probe; never treats a native failure as a non-match.
    fn _try_query_interface(&self, iid: &WinGUID) -> PyResult<bool> {
        self.ensure_live()?;
        if !matches!(
            &self.0,
            dynwinrt::WinRTValue::Object(_) | dynwinrt::WinRTValue::Async(_)
        ) {
            return Ok(false);
        }
        match self.0.cast(&iid.0) {
            Ok(interface) => {
                drop(interface);
                Ok(true)
            }
            Err(dynwinrt::Error::WindowsError(error))
                if error.code() == windows::Win32::Foundation::E_NOINTERFACE =>
            {
                Ok(false)
            }
            Err(error) => Err(map_dynwinrt_error(error)),
        }
    }

    /// Whether `release()` has run on this value, directly or through
    /// `release_projected()` or a closing `projected_lifetime_scope()`.
    ///
    /// A released value is also `is_null()`; this tells it apart from a WinRT
    /// null reference, which remains usable as a null input.
    fn is_released(&self) -> bool {
        matches!(self.1, Lifecycle::Released)
    }

    /// Release resources owned by this value and replace it with Null.
    ///
    /// This is idempotent so projected lifetime scopes can safely retry
    /// cleanup without double-releasing COM references.
    fn release(&mut self) -> PyResult<()> {
        if matches!(self.1, Lifecycle::Released) {
            return Ok(());
        }
        ensure_native_owner_thread(self.2, self.3, "DynWinRTValue")?;
        let value = std::mem::replace(&mut self.0, dynwinrt::WinRTValue::Null);
        self.1 = Lifecycle::Released;
        drop(value);
        Ok(())
    }

    fn as_raw(&self) -> PyResult<i64> {
        Ok(self.receiver("as_raw()")?.as_raw() as i64)
    }

    fn identity_raw(&self) -> PyResult<i64> {
        self.receiver("identity_raw()")?
            .cast::<IUnknown>()
            .map(|identity| identity.as_raw() as i64)
            .map_err(map_windows_error)
    }

    /// COM QueryInterface — cast to a different interface.
    fn cast(&self, py: Python<'_>, iid: &WinGUID) -> PyResult<Py<DynWinRTValue>> {
        tracked_native_value_with_policy(py, self.query(&iid.0, "cast()")?, self.3)
    }

    /// Invoke metadata-described Invoke on an IUnknown-rooted WinRT delegate.
    fn invoke_delegate(
        slf: &Bound<'_, Self>,
        iid: &WinGUID,
        signature: &DynWinRTMethodSig,
        args: Vec<Py<DynWinRTValue>>,
    ) -> PyResult<Vec<Py<DynWinRTValue>>> {
        crate::delegate_method::DynWinRTDelegateMethod::create(iid, signature)?.invoke(slf, args)
    }

    /// Call IActivationFactory::ActivateInstance (vtable[6]) to create a default instance.
    /// Use on the result of activation_factory() for classes with parameterless constructors.
    fn activate(&self, py: Python<'_>) -> PyResult<Py<DynWinRTValue>> {
        let method = dynwinrt::MethodSignature::new(&*TABLE)
            .add_out(TABLE.object())
            .build(6);
        let raw = self.com_receiver("activate()")?.as_raw();
        let result = method.call_dynamic(raw, &[]).map_err(map_windows_error)?;
        tracked_native_value(
            py,
            result
                .into_iter()
                .next()
                .ok_or_else(|| PyRuntimeError::new_err("activate: no result"))?,
        )
    }

    // -- Convenience call methods (match JS API) --

    /// Call a method with no args and one out param.
    fn call_0(
        &self,
        py: Python<'_>,
        method_index: usize,
        return_type: &DynWinRTType,
    ) -> PyResult<Py<DynWinRTValue>> {
        let obj_raw = self.receiver("call_0()")?.as_raw();
        let method = dynwinrt::MethodSignature::new(&*TABLE)
            .add_out(return_type.0.clone())
            .build(method_index);
        let result = method
            .call_dynamic(obj_raw, &[])
            .map_err(map_windows_error)?;
        tracked_native_value(
            py,
            result.into_iter().next().expect("call_0 has one output"),
        )
    }

    /// Call a method with one arg and one out param.
    fn call_1(
        &self,
        py: Python<'_>,
        method_index: usize,
        return_type: &DynWinRTType,
        v1: &DynWinRTValue,
    ) -> PyResult<Py<DynWinRTValue>> {
        let obj_raw = self.receiver("call_1()")?.as_raw();
        v1.check_input("call_1()", InputSlot::Argument(0))?;
        let in_type = TABLE.handle_from_kind(v1.0.get_type_kind());
        let method = dynwinrt::MethodSignature::new(&*TABLE)
            .add_in(in_type)
            .add_out(return_type.0.clone())
            .build(method_index);
        let result = method
            .call_dynamic(obj_raw, &[v1.0.clone()])
            .map_err(map_windows_error)?;
        tracked_native_value(
            py,
            result.into_iter().next().expect("call_1 has one output"),
        )
    }

    /// General-purpose method call with explicit types and args.
    fn call(
        &self,
        py: Python<'_>,
        method_index: usize,
        return_type: &DynWinRTType,
        in_types: Vec<DynWinRTType>,
        args: Vec<Py<DynWinRTValue>>,
    ) -> PyResult<Py<DynWinRTValue>> {
        let mut method = dynwinrt::MethodSignature::new(&*TABLE);
        for t in &in_types {
            method = method.add_in(t.0.clone());
        }
        method = method.add_out(return_type.0.clone());

        let obj = self.receiver("call()")?.as_raw();
        let winrt_args = native_arguments(py, "call()", args)?;

        let mut iface =
            dynwinrt::InterfaceSignature::define_from_iinspectable("", Default::default(), &*TABLE);
        let target_index = method_index;
        for _ in 6..target_index {
            iface.add_method(dynwinrt::MethodSignature::new(&*TABLE));
        }
        iface.add_method(method);

        let result = iface.methods[target_index]
            .call_dynamic(obj, &winrt_args)
            .map_err(map_windows_error)?;

        let value = result
            .into_iter()
            .next()
            .unwrap_or(dynwinrt::WinRTValue::I32(0));
        tracked_native_value(py, value)
    }

    // -- Array / Struct extraction --

    fn is_array(&self) -> bool {
        self.0.as_array().is_some()
    }

    fn as_array(&self, py: Python<'_>) -> PyResult<Py<DynWinRTArray>> {
        self.ensure_live()?;
        match &self.0 {
            dynwinrt::WinRTValue::Array(data) => tracked_native_array(py, data.clone()),
            _ => Err(PyRuntimeError::new_err("Value is not an Array")),
        }
    }

    fn is_struct(&self) -> bool {
        self.0.as_struct().is_some()
    }

    fn as_struct(&self, py: Python<'_>) -> PyResult<Py<DynWinRTStruct>> {
        self.ensure_live()?;
        match &self.0 {
            dynwinrt::WinRTValue::Struct(data) => tracked_native_struct(py, data.clone()),
            _ => Err(PyRuntimeError::new_err("Value is not a Struct")),
        }
    }
}

// ======================================================================
// DynWinRTArray — array container with blittable fast paths
// ======================================================================

#[pyclass(weakref)]
pub struct DynWinRTArray(Option<dynwinrt::ArrayData>, Option<ThreadId>, bool);

// PyO3 enforces exclusive mutable borrows even on free-threaded Python.
// Shared borrows only read the owned buffer, and off-thread COM reads require
// every contained reference to have passed IAgileObject QI.
unsafe impl Send for DynWinRTArray {}
unsafe impl Sync for DynWinRTArray {}

impl Drop for DynWinRTArray {
    fn drop(&mut self) {
        if self
            .0
            .as_ref()
            .is_some_and(|data| data.contains_com_references())
            && must_quarantine_owner(self.1, self.2)
        {
            std::mem::forget(self.0.take());
            log_unsafe_native_owner_drop();
        }
    }
}

impl DynWinRTArray {
    fn data(&self) -> PyResult<&dynwinrt::ArrayData> {
        ensure_native_access_thread(self.1, self.2, "DynWinRTArray")?;
        self.0
            .as_ref()
            .ok_or_else(|| released_native_container_error("DynWinRTArray"))
    }

    fn scalar_array(typ: dynwinrt::TypeHandle, values: &[dynwinrt::WinRTValue]) -> Self {
        Self(
            Some(dynwinrt::ArrayData::from_values(typ, values)),
            None,
            false,
        )
    }

    fn from_elements(
        py: Python<'_>,
        operation: &str,
        values: Vec<Py<DynWinRTValue>>,
        element_type: &DynWinRTType,
    ) -> PyResult<dynwinrt::ArrayData> {
        let values = native_inputs(py, operation, values, InputSlot::Element)?;
        dynwinrt::ArrayData::try_from_values(element_type.0.clone(), &values)
            .map_err(map_windows_error)
    }
}

#[pymethods]
impl DynWinRTArray {
    fn __len__(&self) -> PyResult<usize> {
        Ok(self.data()?.len())
    }

    /// Per-element access.
    fn get(&self, py: Python<'_>, index: i64) -> PyResult<Py<DynWinRTValue>> {
        let data = self.data()?;
        let index = checked_index(index)?;
        data.try_get(index)
            .map_err(map_dynwinrt_error)
            .and_then(|value| tracked_native_value(py, value))
    }

    /// Convert all elements to a list of DynWinRTValue.
    fn to_values(&self, py: Python<'_>) -> PyResult<Vec<Py<DynWinRTValue>>> {
        let data = self.data()?;
        (0..data.len())
            .map(|i| tracked_native_value(py, data.get(i)))
            .collect()
    }

    // -- Typed list extraction (works for both Values and CoTaskMem arrays) --

    fn to_i8_list(&self) -> PyResult<Vec<i32>> {
        let data = self.data()?;
        Ok((0..data.len())
            .map(|i| data.get(i).as_i32().unwrap_or(0))
            .collect())
    }
    fn to_u8_list(&self) -> PyResult<Vec<u8>> {
        let data = self.data()?;
        Ok((0..data.len())
            .map(|i| match data.get(i) {
                dynwinrt::WinRTValue::U8(v) => v,
                other => other.as_i32().unwrap_or(0) as u8,
            })
            .collect())
    }
    fn to_i16_list(&self) -> PyResult<Vec<i32>> {
        let data = self.data()?;
        Ok((0..data.len())
            .map(|i| data.get(i).as_i32().unwrap_or(0))
            .collect())
    }
    fn to_u16_list(&self) -> PyResult<Vec<u32>> {
        let data = self.data()?;
        Ok((0..data.len())
            .map(|i| data.get(i).as_i32().unwrap_or(0) as u32)
            .collect())
    }
    fn to_i32_list(&self) -> PyResult<Vec<i32>> {
        let data = self.data()?;
        (0..data.len())
            .map(|i| data.get_i32(i).map_err(map_dynwinrt_error))
            .collect()
    }
    fn to_u32_list(&self) -> PyResult<Vec<u32>> {
        let data = self.data()?;
        (0..data.len())
            .map(|i| data.get_u32(i).map_err(map_dynwinrt_error))
            .collect()
    }
    fn to_f32_list(&self) -> PyResult<Vec<f32>> {
        let data = self.data()?;
        Ok((0..data.len())
            .map(|i| match data.get(i) {
                dynwinrt::WinRTValue::F32(v) => v,
                dynwinrt::WinRTValue::F64(v) => v as f32,
                other => other.as_i32().unwrap_or(0) as f32,
            })
            .collect())
    }
    fn to_f64_list(&self) -> PyResult<Vec<f64>> {
        let data = self.data()?;
        Ok((0..data.len())
            .map(|i| match data.get(i) {
                dynwinrt::WinRTValue::F64(v) => v,
                dynwinrt::WinRTValue::F32(v) => v as f64,
                other => other.as_i32().unwrap_or(0) as f64,
            })
            .collect())
    }
    fn to_i64_list(&self) -> PyResult<Vec<i64>> {
        let data = self.data()?;
        Ok((0..data.len())
            .map(|i| match data.get(i) {
                dynwinrt::WinRTValue::I64(v) => v,
                other => other.as_i32().unwrap_or(0) as i64,
            })
            .collect())
    }
    fn to_u64_list(&self) -> PyResult<Vec<u64>> {
        let data = self.data()?;
        Ok((0..data.len())
            .map(|i| match data.get(i) {
                dynwinrt::WinRTValue::U64(v) => v,
                other => other.as_i32().unwrap_or(0) as u64,
            })
            .collect())
    }
    fn to_string_list(&self) -> PyResult<Vec<String>> {
        let data = self.data()?;
        Ok((0..data.len())
            .map(|i| match data.get(i) {
                dynwinrt::WinRTValue::HString(s) => s.to_string(),
                other => format!("{:?}", other),
            })
            .collect())
    }

    // -- Construction from Python lists --

    #[staticmethod]
    fn from_i8_values(values: Vec<i32>) -> PyResult<DynWinRTArray> {
        let wvals: Vec<dynwinrt::WinRTValue> = values
            .into_iter()
            .map(|value| {
                Ok(dynwinrt::WinRTValue::I8(checked_i8(
                    value,
                    "from_i8_values",
                )?))
            })
            .collect::<PyResult<_>>()?;
        Ok(Self::scalar_array(TABLE.i8_type(), &wvals))
    }
    #[staticmethod]
    fn from_u8_values(values: Vec<u8>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> =
            values.into_iter().map(dynwinrt::WinRTValue::U8).collect();
        Self::scalar_array(TABLE.u8_type(), &wvals)
    }
    #[staticmethod]
    fn from_i16_values(values: Vec<i32>) -> PyResult<DynWinRTArray> {
        let wvals: Vec<dynwinrt::WinRTValue> = values
            .into_iter()
            .map(|value| {
                Ok(dynwinrt::WinRTValue::I16(checked_i16(
                    value,
                    "from_i16_values",
                )?))
            })
            .collect::<PyResult<_>>()?;
        Ok(Self::scalar_array(TABLE.i16_type(), &wvals))
    }
    #[staticmethod]
    fn from_u16_values(values: Vec<u32>) -> PyResult<DynWinRTArray> {
        let wvals: Vec<dynwinrt::WinRTValue> = values
            .into_iter()
            .map(|value| {
                Ok(dynwinrt::WinRTValue::U16(checked_u16(
                    value,
                    "from_u16_values",
                )?))
            })
            .collect::<PyResult<_>>()?;
        Ok(Self::scalar_array(TABLE.u16_type(), &wvals))
    }
    #[staticmethod]
    fn from_i32_values(values: Vec<i32>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> =
            values.into_iter().map(dynwinrt::WinRTValue::I32).collect();
        Self::scalar_array(TABLE.i32_type(), &wvals)
    }
    #[staticmethod]
    fn from_u32_values(values: Vec<u32>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> =
            values.into_iter().map(dynwinrt::WinRTValue::U32).collect();
        Self::scalar_array(TABLE.u32_type(), &wvals)
    }
    #[staticmethod]
    fn from_f32_values(values: Vec<f32>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> =
            values.into_iter().map(dynwinrt::WinRTValue::F32).collect();
        Self::scalar_array(TABLE.f32_type(), &wvals)
    }
    #[staticmethod]
    fn from_f64_values(values: Vec<f64>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> =
            values.into_iter().map(dynwinrt::WinRTValue::F64).collect();
        Self::scalar_array(TABLE.f64_type(), &wvals)
    }
    #[staticmethod]
    fn from_i64_values(values: Vec<i64>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> =
            values.into_iter().map(dynwinrt::WinRTValue::I64).collect();
        Self::scalar_array(TABLE.i64_type(), &wvals)
    }
    #[staticmethod]
    fn from_u64_values(values: Vec<u64>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> =
            values.into_iter().map(dynwinrt::WinRTValue::U64).collect();
        Self::scalar_array(TABLE.u64_type(), &wvals)
    }
    #[staticmethod]
    fn from_string_values(values: Vec<String>) -> DynWinRTArray {
        let wvals: Vec<dynwinrt::WinRTValue> = values
            .into_iter()
            .map(|s| dynwinrt::WinRTValue::HString(HSTRING::from(&s)))
            .collect();
        Self::scalar_array(TABLE.make(dynwinrt::TypeKind::HString), &wvals)
    }

    #[staticmethod]
    fn from_values(
        py: Python<'_>,
        values: Vec<Py<DynWinRTValue>>,
        element_type: &DynWinRTType,
    ) -> PyResult<Py<DynWinRTArray>> {
        tracked_native_array(
            py,
            Self::from_elements(py, "DynWinRTArray.from_values()", values, element_type)?,
        )
    }

    /// Build a DynWinRTArray of WinRT object/interface elements.
    ///
    /// Use for `T[]` ABI in-parameters where `T` is a runtime class or
    /// interface — for example, `ModelCatalog(ModelCatalogSource[] sources)`.
    /// Items are passed as DynWinRTValue handles (typically Object-wrapped),
    /// and the element type drives ABI size and IID computation.
    #[staticmethod]
    fn from_object_values(
        py: Python<'_>,
        values: Vec<Py<DynWinRTValue>>,
        element_type: &DynWinRTType,
    ) -> PyResult<Py<DynWinRTArray>> {
        tracked_native_array(
            py,
            Self::from_elements(
                py,
                "DynWinRTArray.from_object_values()",
                values,
                element_type,
            )?,
        )
    }

    /// Return the u8 array data as a Python `bytes` object. Safe for both
    /// `Values`-backed and `CoTaskMem`-backed arrays.
    fn to_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyBytes>> {
        let data = self.data()?;
        let len = data.len();
        let mut buf: Vec<u8> = Vec::with_capacity(len);
        for i in 0..len {
            buf.push(match data.get(i) {
                dynwinrt::WinRTValue::U8(v) => v,
                other => other.as_i32().unwrap_or(0) as u8,
            });
        }
        Ok(pyo3::types::PyBytes::new(py, &buf))
    }

    /// Build a u8 DynWinRTArray from a Python `bytes` or `bytearray` (much more
    /// efficient than `from_u8_values` for large byte buffers because the caller
    /// avoids boxing each byte into a Python int).
    #[staticmethod]
    fn from_bytes(data: &Bound<'_, PyAny>) -> PyResult<DynWinRTArray> {
        let slice: Vec<u8> = if let Ok(b) = data.cast::<pyo3::types::PyBytes>() {
            b.as_bytes().to_vec()
        } else if let Ok(ba) = data.cast::<pyo3::types::PyByteArray>() {
            ba.to_vec()
        } else {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "from_bytes: expected bytes or bytearray",
            ));
        };
        let wvals: Vec<dynwinrt::WinRTValue> =
            slice.into_iter().map(dynwinrt::WinRTValue::U8).collect();
        Ok(Self::scalar_array(TABLE.u8_type(), &wvals))
    }

    /// Wrap as DynWinRTValue::Array for passing to call().
    fn to_value(&self, py: Python<'_>) -> PyResult<Py<DynWinRTValue>> {
        tracked_native_value(py, dynwinrt::WinRTValue::Array(self.data()?.clone()))
    }

    fn is_released(&self) -> bool {
        self.0.is_none()
    }

    fn release(&mut self) -> PyResult<()> {
        ensure_native_owner_thread(self.1, self.2, "DynWinRTArray")?;
        drop(self.0.take());
        Ok(())
    }

    fn __repr__(&self) -> String {
        match &self.0 {
            Some(data) => format!("DynWinRTArray(len={})", data.len()),
            None => "DynWinRTArray(released)".to_string(),
        }
    }
}

// ======================================================================
// DynWinRTStruct — typed field access by index
// ======================================================================

#[pyclass(weakref)]
pub struct DynWinRTStruct(Option<dynwinrt::ValueTypeData>, Option<ThreadId>, bool);

// PyO3 serializes mutable borrows of the owned struct allocation. Shared
// reads cannot race setters, and non-agile COM field access stays on its
// creating thread; a foreign Drop quarantines such fields without Release.
unsafe impl Send for DynWinRTStruct {}
unsafe impl Sync for DynWinRTStruct {}

impl Drop for DynWinRTStruct {
    fn drop(&mut self) {
        if self
            .0
            .as_ref()
            .is_some_and(|data| data.type_handle().contains_com_references())
            && must_quarantine_owner(self.1, self.2)
        {
            std::mem::forget(self.0.take());
            log_unsafe_native_owner_drop();
        }
    }
}

impl DynWinRTStruct {
    fn data(&self) -> PyResult<&dynwinrt::ValueTypeData> {
        ensure_native_access_thread(self.1, self.2, "DynWinRTStruct")?;
        self.0
            .as_ref()
            .ok_or_else(|| released_native_container_error("DynWinRTStruct"))
    }

    fn data_mut(&mut self) -> PyResult<&mut dynwinrt::ValueTypeData> {
        ensure_native_access_thread(self.1, self.2, "DynWinRTStruct")?;
        self.0
            .as_mut()
            .ok_or_else(|| released_native_container_error("DynWinRTStruct"))
    }

    fn complete_field_mutation(&mut self, py: Python<'_>, mutation: PyResult<()>) -> PyResult<()> {
        let agility = self
            .0
            .as_ref()
            .ok_or_else(|| released_native_container_error("DynWinRTStruct"))
            .and_then(native_struct_is_agile);
        // Derive foreign-thread eligibility from the payload even if a setter failed.
        self.2 = match &agility {
            Ok(agile) => *agile,
            Err(_) => false,
        };
        match (mutation, agility) {
            (Ok(()), Ok(_)) => Ok(()),
            (Err(error), Ok(_)) | (Ok(()), Err(error)) => Err(error),
            (Err(error), Err(agility_error)) => {
                error.set_cause(py, Some(agility_error));
                Err(error)
            }
        }
    }
}

#[pymethods]
impl DynWinRTStruct {
    /// Create a zero-initialized struct of the given type.
    #[staticmethod]
    fn create(py: Python<'_>, typ: &DynWinRTType) -> PyResult<Py<DynWinRTStruct>> {
        tracked_native_struct(py, typ.0.default_value())
    }

    // -- Blittable field access (get/set pairs) --

    fn get_i8(&self, index: i64) -> PyResult<i32> {
        get_typed_field(
            self.data()?,
            index,
            dynwinrt::TypeKind::I8,
            &[],
            |value: i8| value as i32,
        )
    }
    fn set_i8(&mut self, index: i64, value: i32) -> PyResult<()> {
        set_typed_field(
            self.data_mut()?,
            index,
            checked_i8(value, "set_i8")?,
            dynwinrt::TypeKind::I8,
            &[],
        )
    }

    fn get_u8(&self, index: i64) -> PyResult<u32> {
        get_typed_field(
            self.data()?,
            index,
            dynwinrt::TypeKind::U8,
            &[],
            |value: u8| value as u32,
        )
    }
    fn set_u8(&mut self, index: i64, value: u32) -> PyResult<()> {
        set_typed_field(
            self.data_mut()?,
            index,
            checked_u8(value, "set_u8")?,
            dynwinrt::TypeKind::U8,
            &[],
        )
    }

    fn get_i16(&self, index: i64) -> PyResult<i32> {
        get_typed_field(
            self.data()?,
            index,
            dynwinrt::TypeKind::I16,
            &[],
            |value: i16| value as i32,
        )
    }
    fn set_i16(&mut self, index: i64, value: i32) -> PyResult<()> {
        set_typed_field(
            self.data_mut()?,
            index,
            checked_i16(value, "set_i16")?,
            dynwinrt::TypeKind::I16,
            &[],
        )
    }

    fn get_u16(&self, index: i64) -> PyResult<u32> {
        get_typed_field(
            self.data()?,
            index,
            dynwinrt::TypeKind::U16,
            &[dynwinrt::TypeKind::Char16],
            |value: u16| value as u32,
        )
    }
    fn set_u16(&mut self, index: i64, value: u32) -> PyResult<()> {
        set_typed_field(
            self.data_mut()?,
            index,
            checked_u16(value, "set_u16")?,
            dynwinrt::TypeKind::U16,
            &[dynwinrt::TypeKind::Char16],
        )
    }

    fn get_i32(&self, index: i64) -> PyResult<i32> {
        get_typed_field(
            self.data()?,
            index,
            dynwinrt::TypeKind::I32,
            &[],
            |value: i32| value,
        )
    }
    fn set_i32(&mut self, index: i64, value: i32) -> PyResult<()> {
        set_typed_field(self.data_mut()?, index, value, dynwinrt::TypeKind::I32, &[])
    }

    fn get_u32(&self, index: i64) -> PyResult<u32> {
        get_typed_field(
            self.data()?,
            index,
            dynwinrt::TypeKind::U32,
            &[],
            |value: u32| value,
        )
    }
    fn set_u32(&mut self, index: i64, value: u32) -> PyResult<()> {
        set_typed_field(self.data_mut()?, index, value, dynwinrt::TypeKind::U32, &[])
    }

    fn get_f32(&self, index: i64) -> PyResult<f64> {
        get_typed_field(
            self.data()?,
            index,
            dynwinrt::TypeKind::F32,
            &[],
            |value: f32| value as f64,
        )
    }
    fn set_f32(&mut self, index: i64, value: f64) -> PyResult<()> {
        set_typed_field(
            self.data_mut()?,
            index,
            value as f32,
            dynwinrt::TypeKind::F32,
            &[],
        )
    }

    fn get_f64(&self, index: i64) -> PyResult<f64> {
        get_typed_field(
            self.data()?,
            index,
            dynwinrt::TypeKind::F64,
            &[],
            |value: f64| value,
        )
    }
    fn set_f64(&mut self, index: i64, value: f64) -> PyResult<()> {
        set_typed_field(self.data_mut()?, index, value, dynwinrt::TypeKind::F64, &[])
    }

    fn get_i64(&self, index: i64) -> PyResult<i64> {
        get_typed_field(
            self.data()?,
            index,
            dynwinrt::TypeKind::I64,
            &[],
            |value: i64| value,
        )
    }
    fn set_i64(&mut self, index: i64, value: i64) -> PyResult<()> {
        set_typed_field(self.data_mut()?, index, value, dynwinrt::TypeKind::I64, &[])
    }

    fn get_u64(&self, index: i64) -> PyResult<u64> {
        get_typed_field(
            self.data()?,
            index,
            dynwinrt::TypeKind::U64,
            &[],
            |value: u64| value,
        )
    }
    fn set_u64(&mut self, index: i64, value: u64) -> PyResult<()> {
        set_typed_field(self.data_mut()?, index, value, dynwinrt::TypeKind::U64, &[])
    }

    // -- Non-blittable field access --

    fn get_hstring(&self, index: i64) -> PyResult<String> {
        let index = checked_index(index)?;
        self.data()?
            .get_field_hstring(index)
            .map(|value| value.to_string())
            .map_err(map_dynwinrt_error)
    }

    fn set_hstring(&mut self, index: i64, value: String) -> PyResult<()> {
        let index = checked_index(index)?;
        self.data_mut()?
            .set_field_hstring(index, HSTRING::from(&value))
            .map_err(map_dynwinrt_error)
    }

    fn get_guid(&self, index: i64) -> PyResult<WinGUID> {
        get_typed_field(self.data()?, index, dynwinrt::TypeKind::Guid, &[], WinGUID)
    }

    fn set_guid(&mut self, index: i64, value: &WinGUID) -> PyResult<()> {
        set_typed_field(
            self.data_mut()?,
            index,
            value.0,
            dynwinrt::TypeKind::Guid,
            &[],
        )
    }

    fn get_struct(&self, py: Python<'_>, index: i64) -> PyResult<Py<DynWinRTStruct>> {
        let index = checked_index(index)?;
        let data = self
            .data()?
            .get_field_struct_checked(index)
            .map_err(map_dynwinrt_error)?;
        tracked_native_struct(py, data)
    }

    fn set_struct(&mut self, py: Python<'_>, index: i64, value: &DynWinRTStruct) -> PyResult<()> {
        let index = checked_index(index)?;
        self.data()?;
        let nested = value.data()?;
        if self.1.is_some_and(|owner| owner != thread::current().id())
            && !native_struct_is_agile(nested)?
        {
            return Err(PyRuntimeError::new_err(
                "cannot store non-agile COM fields in a struct from another apartment thread",
            ));
        }
        let mutation = self
            .data_mut()?
            .set_field_struct_checked(index, nested)
            .map_err(map_dynwinrt_error);
        self.complete_field_mutation(py, mutation)
    }

    fn get_object(&self, py: Python<'_>, index: i64) -> PyResult<Py<DynWinRTValue>> {
        let index = checked_index(index)?;
        let value = match self
            .data()?
            .get_field_object(index)
            .map_err(map_dynwinrt_error)?
        {
            Some(object) => dynwinrt::WinRTValue::Object(object),
            None => dynwinrt::WinRTValue::Null,
        };
        tracked_native_value(py, value)
    }

    fn set_object(&mut self, py: Python<'_>, index: i64, value: &DynWinRTValue) -> PyResult<()> {
        let index = checked_index(index)?;
        self.data()?;
        value.check_input("DynWinRTStruct.set_object()", InputSlot::Field(index))?;
        let object = match &value.0 {
            dynwinrt::WinRTValue::Object(obj) => Some(obj),
            dynwinrt::WinRTValue::Null => None,
            _ => {
                return Err(PyTypeError::new_err(
                    "set_object requires a WinRT object or null value",
                ));
            }
        };
        if self.1.is_some_and(|owner| owner != thread::current().id())
            && object.is_some()
            && !native_value_is_agile(&value.0)?
        {
            return Err(PyRuntimeError::new_err(
                "cannot store a non-agile COM field from another apartment thread",
            ));
        }
        let mutation = self
            .data_mut()?
            .set_field_object(index, object)
            .map_err(map_dynwinrt_error);
        self.complete_field_mutation(py, mutation)
    }

    /// Wrap as DynWinRTValue::Struct for passing to call().
    fn to_value(&self, py: Python<'_>) -> PyResult<Py<DynWinRTValue>> {
        tracked_native_value(py, dynwinrt::WinRTValue::Struct(self.data()?.clone()))
    }

    fn is_released(&self) -> bool {
        self.0.is_none()
    }

    fn release(&mut self) -> PyResult<()> {
        ensure_native_owner_thread(self.1, self.2, "DynWinRTStruct")?;
        drop(self.0.take());
        Ok(())
    }

    fn __repr__(&self) -> String {
        if self.is_released() {
            "DynWinRTStruct(released)".to_string()
        } else {
            "DynWinRTStruct(...)".to_string()
        }
    }
}

// ======================================================================
// DynWinRtDelegate — dynamic WinRT delegate (callback) binding
// ======================================================================

#[pyclass(weakref)]
pub struct DynWinRtDelegate(Option<dynwinrt::WinRTValue>, Option<ThreadId>);

impl Drop for DynWinRtDelegate {
    fn drop(&mut self) {
        if self.0.is_some() && must_quarantine_owner(self.1, true) {
            std::mem::forget(self.0.take());
            log_unsafe_native_owner_drop();
        }
    }
}

pub(crate) const PYWINRT_E_UNRAISABLE_PYTHON_EXCEPTION: windows::core::HRESULT =
    windows::core::HRESULT(0xA0EE4005_u32 as i32);
pub(crate) const PYWINRT_E_INTERPRETER_CLOSED: windows::core::HRESULT =
    windows::core::HRESULT(0x80000013_u32 as i32);

fn create_python_delegate(
    iid: GUID,
    type_handles: Vec<dynwinrt::TypeHandle>,
    callback: Py<PyAny>,
) -> PyResult<dynwinrt::WinRTValue> {
    let delegate_callback: dynwinrt::delegate::DelegateCallback =
        Box::new(move |args: &[dynwinrt::WinRTValue]| {
            with_python_callback(|py| {
                let result = (|| -> PyResult<()> {
                    let py_args = args
                        .iter()
                        .map(|arg| Ok(callback_native_argument(py, arg.clone())?.into_any()))
                        .collect::<PyResult<Vec<Py<PyAny>>>>()?;
                    let py_tuple = pyo3::types::PyTuple::new(py, &py_args)?;
                    callback.call1(py, py_tuple)?;
                    Ok(())
                })();
                match result {
                    Ok(()) => windows::core::HRESULT(0),
                    Err(error) => {
                        error.write_unraisable(py, Some(callback.bind(py)));
                        PYWINRT_E_UNRAISABLE_PYTHON_EXCEPTION
                    }
                }
            })
            .unwrap_or(PYWINRT_E_INTERPRETER_CLOSED)
        });
    dynwinrt::delegate::try_create_delegate_value(iid, type_handles, delegate_callback)
        .map_err(|error| map_dynwinrt_error_with_context(error, "DynWinRtDelegate.create failed"))
}

#[pymethods]
impl DynWinRtDelegate {
    /// Create a delegate COM object from a Python callback function.
    ///
    /// - `iid`: delegate interface IID
    /// - `param_types`: Invoke parameter types
    /// - `callback`: Python callable invoked when WinRT fires the event
    #[staticmethod]
    fn create(
        py: Python<'_>,
        iid: &WinGUID,
        param_types: Vec<PyRef<DynWinRTType>>,
        callback: Py<PyAny>,
    ) -> PyResult<Py<DynWinRtDelegate>> {
        ensure_python_callbacks_open()?;
        let type_handles: Vec<dynwinrt::TypeHandle> =
            param_types.iter().map(|t| t.0.clone()).collect();
        let value = create_python_delegate(iid.0, type_handles, callback)?;
        ensure_python_callbacks_open()?;
        let output = Py::new(
            py,
            DynWinRtDelegate(Some(value), current_native_owner_thread(true)),
        )?;
        track_native_owner(py, output.clone_ref(py).into_any())?;
        Ok(output)
    }

    /// Get the delegate as a DynWinRTValue for passing to WinRT methods.
    fn to_value(&self, py: Python<'_>) -> PyResult<Py<DynWinRTValue>> {
        let value = self
            .0
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("DynWinRtDelegate has been released"))?;
        tracked_native_value_with_policy(py, value.clone(), true)
    }

    fn is_released(&self) -> bool {
        self.0.is_none()
    }

    fn release(&mut self) -> PyResult<()> {
        ensure_native_owner_thread(self.1, true, "DynWinRtDelegate")?;
        drop(self.0.take());
        Ok(())
    }

    fn __repr__(&self) -> String {
        "DynWinRtDelegate(...)".to_string()
    }
}

// ======================================================================
// DynWinRtElementFactory — synchronous WinUI IElementFactory binding
// ======================================================================

struct ElementFactoryCallback {
    invoke: Py<PyAny>,
    error_target: Py<PyAny>,
}

struct ElementFactoryCallbacks {
    get_element: Option<ElementFactoryCallback>,
    recycle_element: Option<ElementFactoryCallback>,
}

#[pyclass(weakref)]
pub struct DynWinRtElementFactory {
    value: Option<dynwinrt::WinRTValue>,
    callbacks: Arc<Mutex<ElementFactoryCallbacks>>,
    owner_thread: Option<ThreadId>,
}

impl Drop for DynWinRtElementFactory {
    fn drop(&mut self) {
        if self.value.is_some() && must_quarantine_owner(self.owner_thread, true) {
            std::mem::forget(self.value.take());
            log_unsafe_native_owner_drop();
        }
    }
}

impl DynWinRtElementFactory {
    fn clear_callbacks(&self) -> PyResult<()> {
        let released = {
            let mut callbacks = self.callbacks.lock().map_err(|_| {
                PyRuntimeError::new_err("IElementFactory callback state is poisoned")
            })?;
            (
                callbacks.get_element.take(),
                callbacks.recycle_element.take(),
            )
        };
        drop(released);
        Ok(())
    }
}

#[pymethods]
impl DynWinRtElementFactory {
    #[staticmethod]
    fn create(
        py: Python<'_>,
        element_iid: &WinGUID,
        get_element: Py<PyAny>,
        recycle_element: Py<PyAny>,
    ) -> PyResult<Py<Self>> {
        const E_FAIL: windows::core::HRESULT = windows::core::HRESULT(0x80004005_u32 as i32);
        const RO_E_CLOSED: windows::core::HRESULT = windows::core::HRESULT(0x80000013_u32 as i32);

        ensure_python_callbacks_open()?;
        let element_iid = element_iid.0;
        let get_element = ElementFactoryCallback {
            error_target: get_element.clone_ref(py),
            invoke: wrap_python_callback_context(py, get_element)?,
        };
        let recycle_element = ElementFactoryCallback {
            error_target: recycle_element.clone_ref(py),
            invoke: wrap_python_callback_context(py, recycle_element)?,
        };
        let callbacks = Arc::new(Mutex::new(ElementFactoryCallbacks {
            get_element: Some(get_element),
            recycle_element: Some(recycle_element),
        }));

        let get_callbacks = callbacks.clone();
        let get_callback: dynwinrt::ElementFactoryGetCallback = Box::new(move |args| {
            with_python_callback(|py| {
                let (callback, error_target) = {
                    let callbacks = get_callbacks.lock().map_err(|_| E_FAIL)?;
                    let callback = callbacks.get_element.as_ref().ok_or(RO_E_CLOSED)?;
                    (
                        callback.invoke.clone_ref(py),
                        callback.error_target.clone_ref(py),
                    )
                };
                let result = (|| -> PyResult<dynwinrt::WinRTValue> {
                    let argument = callback_native_argument(py, args.clone())?;
                    let result = callback.call1(py, (argument,))?;
                    let value = result.extract::<PyRef<DynWinRTValue>>(py)?;
                    value.0.cast(&element_iid).map_err(map_dynwinrt_error)
                })();
                match result {
                    Ok(value) => Ok(value),
                    Err(error) => {
                        error.write_unraisable(py, Some(error_target.bind(py)));
                        Err(E_FAIL)
                    }
                }
            })
            .unwrap_or(Err(PYWINRT_E_INTERPRETER_CLOSED))
        });

        let recycle_callbacks = callbacks.clone();
        let recycle_callback: dynwinrt::ElementFactoryRecycleCallback = Box::new(move |args| {
            with_python_callback(|py| {
                let (callback, error_target) = {
                    let callbacks = match recycle_callbacks.lock() {
                        Ok(callbacks) => callbacks,
                        Err(_) => return E_FAIL,
                    };
                    let Some(callback) = callbacks.recycle_element.as_ref() else {
                        return RO_E_CLOSED;
                    };
                    (
                        callback.invoke.clone_ref(py),
                        callback.error_target.clone_ref(py),
                    )
                };
                let result = (|| -> PyResult<()> {
                    let argument = callback_native_argument(py, args.clone())?;
                    callback.call1(py, (argument,))?;
                    Ok(())
                })();
                match result {
                    Ok(()) => windows::core::HRESULT(0),
                    Err(error) => {
                        error.write_unraisable(py, Some(error_target.bind(py)));
                        E_FAIL
                    }
                }
            })
            .unwrap_or(PYWINRT_E_INTERPRETER_CLOSED)
        });

        ensure_python_callbacks_open()?;
        let output = Py::new(
            py,
            Self {
                value: Some(dynwinrt::create_element_factory_value(
                    get_callback,
                    recycle_callback,
                )),
                callbacks,
                owner_thread: current_native_owner_thread(true),
            },
        )?;
        track_native_owner(py, output.clone_ref(py).into_any())?;
        Ok(output)
    }

    fn to_value(&self, py: Python<'_>) -> PyResult<Py<DynWinRTValue>> {
        let value = self
            .value
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("DynWinRtElementFactory has been released"))?;
        tracked_native_value_with_policy(py, value.clone(), true)
    }

    fn release_callbacks(&self) -> PyResult<()> {
        self.clear_callbacks()
    }

    fn release(&mut self) -> PyResult<()> {
        ensure_native_owner_thread(self.owner_thread, true, "DynWinRtElementFactory")?;
        self.clear_callbacks()?;
        drop(self.value.take());
        Ok(())
    }

    fn _release_apartment_owner(&mut self) -> PyResult<()> {
        ensure_native_owner_thread(self.owner_thread, true, "DynWinRtElementFactory")?;
        drop(self.value.take());
        Ok(())
    }

    fn __repr__(&self) -> &'static str {
        "DynWinRtElementFactory(...)"
    }
}

// ======================================================================
// System info utilities
// ======================================================================

#[pyfunction]
pub fn has_package_identity() -> bool {
    use windows::ApplicationModel::AppInfo;
    AppInfo::Current().is_ok()
}

/// Return the framework resources.pri path selected by init_winappsdk.
/// The Windows App SDK must have been initialized (via `init_winappsdk`)
/// before calling this function.
#[pyfunction]
pub fn get_winappsdk_resource_pri_path() -> PyResult<String> {
    dynwinrt::WinAppSdkContext {}
        .resource_pri_path()
        .map_err(map_windows_error)
}

#[pyfunction]
pub fn get_computer_name() -> PyResult<String> {
    use windows::Win32::System::WindowsProgramming::GetComputerNameW;
    use windows::core::PWSTR;

    let mut buffer = [0u16; 256];
    let mut size = buffer.len() as u32;

    unsafe {
        if GetComputerNameW(Some(PWSTR(buffer.as_mut_ptr())), &mut size).is_ok() {
            Ok(String::from_utf16_lossy(&buffer[..size as usize]))
        } else {
            Err(PyRuntimeError::new_err("Failed to get computer name"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::types::PyDict;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    struct LateDropProbe {
        apartment: Option<RoApartment>,
        state_was_destroyed: Arc<std::sync::atomic::AtomicBool>,
        creation_was_rejected: Arc<std::sync::atomic::AtomicBool>,
    }

    impl Drop for LateDropProbe {
        fn drop(&mut self) {
            self.state_was_destroyed.store(
                CALLBACK_PENDING_APARTMENTS.try_with(|_| ()).is_err(),
                Ordering::SeqCst,
            );
            self.creation_was_rejected
                .store(RoApartment::new(None).is_err(), Ordering::SeqCst);
            drop(self.apartment.take());
        }
    }

    thread_local! {
        static LATE_APARTMENT_DROP: RefCell<Option<LateDropProbe>> = const { RefCell::new(None) };
    }

    fn test_ro_uninitialize() -> PyResult<()> {
        let depth = MANUAL_APARTMENT_DEPTH.with(Cell::get);
        if depth == 0 {
            return Err(PyRuntimeError::new_err(
                "ro_uninitialize() requires a successful ro_initialize() on this thread",
            ));
        }
        leave_managed_apartment_with_drain("ro_uninitialize()", || Ok(()))?;
        MANUAL_APARTMENT_DEPTH.with(|state| state.set(depth - 1));
        Ok(())
    }

    #[test]
    fn reentrant_apartment_guard_preserves_nested_and_manual_initializations() {
        Python::initialize();
        std::thread::spawn(|| {
            Python::attach(|py| {
                let mut apartment = RoApartment::new(Some(RO_INIT_SINGLETHREADED.0)).unwrap();
                apartment.initialize().unwrap();
                let outer_callback = NativeCallbackGuard::enter();
                let inner_callback = NativeCallbackGuard::enter();
                let error = apartment.uninitialize("RoApartment.close()").unwrap_err();
                assert!(error.is_instance_of::<PyRuntimeError>(py));
                assert!(error.to_string().contains("retry after the callback"));
                assert!(apartment.active);

                let mut nested = RoApartment::new(Some(RO_INIT_SINGLETHREADED.0)).unwrap();
                nested.initialize().unwrap();
                nested.uninitialize("RoApartment.close()").unwrap();
                ro_initialize(Some(RO_INIT_SINGLETHREADED.0)).unwrap();
                test_ro_uninitialize().unwrap();
                assert!(test_ro_uninitialize().is_err());
                assert!(apartment.active);
                drop(inner_callback);
                assert!(apartment.uninitialize("RoApartment.close()").is_err());
                drop(outer_callback);

                assert!(test_ro_uninitialize().is_err());
                apartment.uninitialize("RoApartment.close()").unwrap();
                assert!(!apartment.active);
                apartment.uninitialize("RoApartment.close()").unwrap();
                let mut other_model = RoApartment::new(Some(RO_INIT_MULTITHREADED.0)).unwrap();
                other_model.initialize().unwrap();
                other_model.uninitialize("RoApartment.close()").unwrap();
            });
        })
        .join()
        .unwrap();
    }

    #[test]
    fn dropped_final_apartment_requires_owner_thread_recovery() {
        Python::initialize();
        std::thread::spawn(|| {
            Python::attach(|py| {
                let mut apartment = RoApartment::new(Some(RO_INIT_SINGLETHREADED.0)).unwrap();
                apartment.initialize().unwrap();
                let callback = NativeCallbackGuard::enter();
                apartment
                    .uninitialize("RoApartment.__exit__()")
                    .unwrap_err();
                drop(apartment);
                assert!(RoApartment::recover_pending().is_err());
                drop(callback);

                let wrong_thread_error = py.detach(|| {
                    std::thread::spawn(|| {
                        Python::attach(|py| {
                            RoApartment::recover_pending()
                                .err()
                                .expect("wrong-thread recovery should fail")
                                .value(py)
                                .str()
                                .unwrap()
                                .to_str()
                                .unwrap()
                                .to_owned()
                        })
                    })
                    .join()
                    .unwrap()
                });
                assert!(wrong_thread_error.contains("on this thread"));

                let mut recovered = RoApartment::recover_pending().unwrap();
                assert!(recovered.active);
                assert!(RoApartment::recover_pending().is_err());
                recovered.uninitialize("RoApartment.close()").unwrap();
                assert!(!recovered.active);
            });
        })
        .join()
        .unwrap();
    }

    #[test]
    fn foreign_close_and_drop_preserve_owner_thread_retry() {
        let _serial = crate::errors::UNRAISABLE_HOOK_TEST_LOCK.lock().unwrap();
        Python::initialize();
        std::thread::spawn(|| {
            let mut apartment = RoApartment::new(Some(RO_INIT_SINGLETHREADED.0)).unwrap();
            apartment.initialize().unwrap();
            let apartment = std::thread::spawn(move || {
                assert!(apartment.uninitialize("RoApartment.close()").is_err());
                assert!(apartment.active);
                apartment
            })
            .join()
            .unwrap();
            assert!(apartment.active);

            let callback = NativeCallbackGuard::enter();
            std::thread::spawn(move || drop(apartment)).join().unwrap();
            assert!(RoApartment::recover_pending().is_err());
            drop(callback);

            let mut recovered = RoApartment::recover_pending().unwrap();
            assert!(recovered.active);
            recovered.uninitialize("RoApartment.close()").unwrap();
            let mut other_model = RoApartment::new(Some(RO_INIT_MULTITHREADED.0)).unwrap();
            other_model.initialize().unwrap();
            other_model.uninitialize("RoApartment.close()").unwrap();
        })
        .join()
        .unwrap();
    }

    #[test]
    fn poisoned_foreign_drop_queue_keeps_lease_recoverable() {
        let _serial = crate::errors::UNRAISABLE_HOOK_TEST_LOCK.lock().unwrap();
        Python::initialize();
        std::thread::spawn(|| {
            let mut apartment = RoApartment::new(Some(RO_INIT_SINGLETHREADED.0)).unwrap();
            apartment.initialize().unwrap();
            let queue = apartment.foreign_drops.clone();
            let poisoned = std::panic::catch_unwind(|| {
                let _guard = queue.lock().unwrap();
                panic!("poison the owner-thread recovery queue");
            });
            assert!(poisoned.is_err());

            std::thread::spawn(move || drop(apartment)).join().unwrap();
            assert!(!queue.is_poisoned());
            let poisoned_again = std::panic::catch_unwind(|| {
                let _guard = queue.lock().unwrap();
                panic!("poison the owner-thread queue before recovery");
            });
            assert!(poisoned_again.is_err());
            let mut recovered = RoApartment::recover_pending().unwrap();
            assert!(!queue.is_poisoned());
            recovered.uninitialize("RoApartment.close()").unwrap();
            assert!(RoApartment::recover_pending().is_err());
        })
        .join()
        .unwrap();
    }

    #[test]
    fn closed_owner_queue_rejects_foreign_drop_without_native_cleanup() {
        let _serial = crate::errors::UNRAISABLE_HOOK_TEST_LOCK.lock().unwrap();
        Python::initialize();
        let (sender, receiver) = std::sync::mpsc::channel();
        let owner = std::thread::spawn(move || {
            let mut apartment = RoApartment::new(Some(RO_INIT_SINGLETHREADED.0)).unwrap();
            apartment.initialize().unwrap();
            sender.send(apartment).unwrap();
        });
        let apartment = receiver.recv().unwrap();
        owner.join().unwrap();

        let queue = apartment.foreign_drops.clone();
        assert!(!lock_foreign_drops(&queue).0.owner_alive);
        drop(apartment);
        let (queue, _) = lock_foreign_drops(&queue);
        assert!(queue.pending.is_empty());
    }

    #[test]
    fn foreign_drop_does_not_wait_for_owners_python_gil() {
        Python::initialize();
        std::thread::spawn(|| {
            Python::attach(|_py| {
                let mut apartment = RoApartment::new(Some(RO_INIT_SINGLETHREADED.0)).unwrap();
                apartment.initialize().unwrap();
                let callback = NativeCallbackGuard::enter();
                let (sent, received) = std::sync::mpsc::channel();
                let worker = std::thread::spawn(move || {
                    drop(apartment);
                    sent.send(()).unwrap();
                });
                assert!(
                    received.recv_timeout(Duration::from_secs(3)).is_ok(),
                    "foreign Drop waited for the owner to release Python's GIL"
                );
                worker.join().unwrap();
                assert!(RoApartment::recover_pending().is_err());
                drop(callback);
                let mut recovered = RoApartment::recover_pending().unwrap();
                recovered.uninitialize("RoApartment.close()").unwrap();
            });
        })
        .join()
        .unwrap();
    }

    #[test]
    fn owner_queue_closes_with_unrecovered_foreign_drop() {
        let (sent, received) = std::sync::mpsc::channel();
        let (continue_owner, owner_waits) = std::sync::mpsc::channel::<()>();
        let owner = std::thread::spawn(move || {
            let mut apartment = RoApartment::new(Some(RO_INIT_SINGLETHREADED.0)).unwrap();
            apartment.initialize().unwrap();
            sent.send(apartment).unwrap();
            owner_waits.recv().unwrap();
        });
        let apartment = received.recv().unwrap();
        let queue = apartment.foreign_drops.clone();
        drop(apartment);
        assert_eq!(lock_foreign_drops(&queue).0.pending.len(), 1);
        continue_owner.send(()).unwrap();
        owner.join().unwrap();
        let (queue, _) = lock_foreign_drops(&queue);
        assert!(!queue.owner_alive);
        assert_eq!(queue.pending.len(), 1);
    }

    #[test]
    fn owner_drop_after_apartment_tls_teardown_fails_closed() {
        let state_was_destroyed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let creation_was_rejected = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = state_was_destroyed.clone();
        let rejected = creation_was_rejected.clone();
        std::thread::spawn(move || {
            LATE_APARTMENT_DROP.with(|_| ());
            CALLBACK_PENDING_APARTMENTS.with(|_| ());
            let mut apartment = RoApartment::new(Some(RO_INIT_SINGLETHREADED.0)).unwrap();
            apartment.initialize().unwrap();
            LATE_APARTMENT_DROP.with(|slot| {
                *slot.borrow_mut() = Some(LateDropProbe {
                    apartment: Some(apartment),
                    state_was_destroyed: observed,
                    creation_was_rejected: rejected,
                });
            });
        })
        .join()
        .unwrap();
        assert!(state_was_destroyed.load(Ordering::SeqCst));
        assert!(creation_was_rejected.load(Ordering::SeqCst));
    }

    #[test]
    fn manual_uninitialize_requires_matching_owner_thread_initialization() {
        Python::initialize();
        std::thread::spawn(|| {
            assert!(test_ro_uninitialize().is_err());
            ro_initialize(Some(RO_INIT_SINGLETHREADED.0)).unwrap();
            assert!(
                std::thread::spawn(|| test_ro_uninitialize().is_err())
                    .join()
                    .unwrap()
            );
            test_ro_uninitialize().unwrap();

            let mut apartment = RoApartment::new(Some(RO_INIT_MULTITHREADED.0)).unwrap();
            apartment.initialize().unwrap();
            apartment.uninitialize("RoApartment.close()").unwrap();
        })
        .join()
        .unwrap();
    }

    fn initialize_embedded_binding(py: Python<'_>) {
        let package_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("python")
            .join("dynwinrt");
        let package = PyModule::new(py, "dynwinrt").unwrap();
        package
            .setattr("__path__", vec![package_path.to_string_lossy().to_string()])
            .unwrap();
        package.setattr("__package__", "dynwinrt").unwrap();
        let native = PyModule::new(py, "dynwinrt.dynwinrt").unwrap();
        native.setattr("__package__", "dynwinrt").unwrap();
        let spec = py
            .import("importlib.machinery")
            .unwrap()
            .getattr("ModuleSpec")
            .unwrap()
            .call1(("dynwinrt.dynwinrt", py.None()))
            .unwrap();
        spec.setattr(
            "origin",
            package_path
                .join("dynwinrt.pyd")
                .to_string_lossy()
                .to_string(),
        )
        .unwrap();
        native.setattr("__spec__", spec).unwrap();
        let modules = py.import("sys").unwrap().getattr("modules").unwrap();
        modules.set_item("dynwinrt", &package).unwrap();
        modules.set_item("dynwinrt.dynwinrt", &native).unwrap();
        package.setattr("dynwinrt", &native).unwrap();
        crate::dynwinrt::init(&native).unwrap();

        let source = std::fs::read_to_string(package_path.join("__init__.py")).unwrap();
        let source = std::ffi::CString::new(source).unwrap();
        py.run(source.as_c_str(), Some(&package.dict()), None)
            .unwrap();
    }

    #[test]
    fn guarded_python_native_containers_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<DynWinRTArray>();
        assert_send_sync::<DynWinRTStruct>();
    }

    #[derive(Default)]
    struct QueryCounts {
        queries: AtomicU32,
        addrefs: AtomicU32,
        releases: AtomicU32,
        wrong_thread_addrefs: AtomicU32,
        wrong_thread_releases: AtomicU32,
    }

    #[repr(C)]
    struct QueryProbe {
        vtable: *const windows::core::IUnknown_Vtbl,
        references: AtomicU32,
        owner_thread: ThreadId,
        counts: Arc<QueryCounts>,
    }

    impl QueryProbe {
        const SUPPORTED: GUID = IUnknown::IID;
        const FAILURE: GUID = GUID::from_u128(0x12113896_999b_42d5_87c1_7c68e83592eb);
        const UNKNOWN: GUID = GUID::from_u128(0x6192657c_dbc2_4262_98c8_8ead575ac434);
        const VTABLE: windows::core::IUnknown_Vtbl = windows::core::IUnknown_Vtbl {
            QueryInterface: Self::query,
            AddRef: Self::add_ref,
            Release: Self::release,
        };

        fn new() -> (IUnknown, Arc<QueryCounts>) {
            let counts = Arc::new(QueryCounts::default());
            let object = Box::new(Self {
                vtable: &Self::VTABLE,
                references: AtomicU32::new(1),
                owner_thread: thread::current().id(),
                counts: counts.clone(),
            });
            (
                unsafe { IUnknown::from_raw(Box::into_raw(object).cast()) },
                counts,
            )
        }

        unsafe extern "system" fn query(
            this: *mut c_void,
            iid: *const GUID,
            result: *mut *mut c_void,
        ) -> windows::core::HRESULT {
            if iid.is_null() || result.is_null() {
                return windows::core::HRESULT(0x80004003u32 as i32);
            }
            let object = unsafe { &*this.cast::<Self>() };
            object.counts.queries.fetch_add(1, Ordering::SeqCst);
            unsafe { *result = std::ptr::null_mut() };
            match unsafe { *iid } {
                Self::SUPPORTED => {
                    unsafe { *result = this };
                    unsafe { Self::add_ref(this) };
                    windows::core::HRESULT(0)
                }
                Self::FAILURE => windows::core::HRESULT(0x80004005u32 as i32),
                _ => windows::Win32::Foundation::E_NOINTERFACE,
            }
        }

        fn received_finalization_array(source: &IUnknown, nested: bool) -> DynWinRTArray {
            use dynwinrt::{
                WinRtImplementation, WinRtImplementationPlan, WinRtInterfaceDefinition,
                WinRtMethodDefinition, WinRtThreadingPolicy,
            };

            let table = dynwinrt::MetadataTable::new();
            let element = if nested {
                let inner =
                    table.struct_type("Tests.FinalizationInner", &[table.interface(IUnknown::IID)]);
                table.struct_type("Tests.FinalizationOuter", &[inner])
            } else {
                table.interface(IUnknown::IID)
            };
            let item = if nested {
                let inner_type = element.field_type(0);
                let mut inner = inner_type.default_value();
                inner.set_field_object(0, Some(source)).unwrap();
                let mut outer = element.default_value();
                outer.set_field_struct_checked(0, &inner).unwrap();
                dynwinrt::WinRTValue::Struct(outer)
            } else {
                dynwinrt::WinRTValue::Object(source.clone())
            };
            let signature = dynwinrt::MethodSignature::new(&table).add_out(table.array(&element));
            let iid = if nested {
                GUID::from_u128(0x38684d40_bab3_42de_998d_26e4cce87c52)
            } else {
                GUID::from_u128(0x38684d40_bab3_42de_998d_26e4cce87c51)
            };
            let plan = WinRtImplementationPlan::new(
                vec![WinRtInterfaceDefinition {
                    name: "Tests.IFinalizationReceivedArray".into(),
                    interface_type: table.interface(iid),
                    required_iids: vec![],
                    methods: vec![WinRtMethodDefinition {
                        name: "GetItems".into(),
                        vtable_index: 6,
                        signature: signature.clone(),
                    }],
                }],
                WinRtThreadingPolicy::OwnerThread,
            )
            .unwrap();
            let outputs = Mutex::new(Some(dynwinrt::WinRTValue::Array(
                dynwinrt::ArrayData::from_values(element, &[item]),
            )));
            let mut host = WinRtImplementation::new(
                plan,
                Arc::new(move |_, _, _| Ok(vec![outputs.lock().unwrap().take().unwrap()])),
                None,
            )
            .unwrap();
            let receiver = host.to_value().unwrap().cast(&iid).unwrap();
            let receiver_object = receiver.as_object().unwrap();
            let mut results = signature
                .build(6)
                .call_dynamic(receiver_object.as_raw(), &[])
                .unwrap();
            drop((receiver_object, receiver));
            host.release();
            let dynwinrt::WinRTValue::Array(array) = results.remove(0) else {
                panic!("expected a received native array");
            };
            assert!(format!("{array:?}").contains("CoTaskMem("));
            DynWinRTArray(Some(array), Some(thread::current().id()), false)
        }

        unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
            let object = unsafe { &*this.cast::<Self>() };
            if thread::current().id() != object.owner_thread {
                object
                    .counts
                    .wrong_thread_addrefs
                    .fetch_add(1, Ordering::SeqCst);
            }
            object.counts.addrefs.fetch_add(1, Ordering::SeqCst);
            object.references.fetch_add(1, Ordering::SeqCst) + 1
        }

        unsafe extern "system" fn release(this: *mut c_void) -> u32 {
            let object = unsafe { &*this.cast::<Self>() };
            if thread::current().id() != object.owner_thread {
                object
                    .counts
                    .wrong_thread_releases
                    .fetch_add(1, Ordering::SeqCst);
            }
            object.counts.releases.fetch_add(1, Ordering::SeqCst);
            let remaining = object.references.fetch_sub(1, Ordering::SeqCst) - 1;
            if remaining == 0 {
                unsafe { drop(Box::from_raw(this.cast::<Self>())) };
            }
            remaining
        }
    }

    #[repr(C)]
    struct TestDelegateVtbl {
        base: windows::core::IUnknown_Vtbl,
        invoke: unsafe extern "system" fn(
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
        ) -> windows::core::HRESULT,
    }

    unsafe fn invoke_delegate(value: &dynwinrt::WinRTValue) -> windows::core::HRESULT {
        let dynwinrt::WinRTValue::Object(object) = value else {
            panic!("expected delegate object")
        };
        let raw = object.as_raw();
        let vtable = unsafe { *(raw as *const *const TestDelegateVtbl) };
        unsafe { ((*vtable).invoke)(raw, std::ptr::null_mut(), std::ptr::null_mut()) }
    }

    #[test]
    fn hresult_arrays_convert_to_signed_integers() {
        let values = [
            dynwinrt::WinRTValue::HResult(windows::core::HRESULT(0)),
            dynwinrt::WinRTValue::HResult(windows::core::HRESULT(0x80004005u32 as i32)),
        ];
        let array = DynWinRTArray::scalar_array(TABLE.hresult(), &values);

        assert_eq!(array.to_i32_list().unwrap(), vec![0, 0x80004005u32 as i32]);
    }

    #[test]
    fn failed_field_mutation_reclassifies_partially_written_com_fields() {
        Python::initialize();
        for nested in [false, true] {
            let (source, counts) = QueryProbe::new();
            let field_type = TABLE.interface(QueryProbe::SUPPORTED);
            let mut record = if nested {
                let inner_type =
                    TABLE.struct_type("Tests.PartialAgilityInner", &[field_type.clone()]);
                let outer_type =
                    TABLE.struct_type("Tests.PartialAgilityOuter", &[inner_type.clone()]);
                let mut inner = inner_type.default_value();
                inner.set_field_object(0, Some(&source)).unwrap();
                let mut outer = DynWinRTStruct(
                    Some(outer_type.default_value()),
                    Some(thread::current().id()),
                    true,
                );
                outer
                    .0
                    .as_mut()
                    .unwrap()
                    .set_field_struct_checked(0, &inner)
                    .unwrap();
                outer
            } else {
                let struct_type = TABLE.struct_type("Tests.PartialAgilityObject", &[field_type]);
                let mut direct = DynWinRTStruct(
                    Some(struct_type.default_value()),
                    Some(thread::current().id()),
                    true,
                );
                direct
                    .0
                    .as_mut()
                    .unwrap()
                    .set_field_object(0, Some(&source))
                    .unwrap();
                direct
            };

            Python::attach(|py| {
                let error = record
                    .complete_field_mutation(
                        py,
                        Err(PyIndexError::new_err("native setter failed after writing")),
                    )
                    .unwrap_err();
                assert!(error.is_instance_of::<PyIndexError>(py));
                assert!(error.to_string().contains("failed after writing"));
            });
            assert!(!record.2, "the partially written COM field is non-agile");
            let mut record = thread::spawn(move || {
                assert!(record.release().is_err());
                assert!(!record.is_released());
                record
            })
            .join()
            .unwrap();
            assert_eq!(counts.wrong_thread_addrefs.load(Ordering::SeqCst), 0);
            assert_eq!(counts.wrong_thread_releases.load(Ordering::SeqCst), 0);
            let actual = if nested {
                record
                    .data()
                    .unwrap()
                    .get_field_struct_checked(0)
                    .unwrap()
                    .get_field_object(0)
                    .unwrap()
                    .unwrap()
            } else {
                record.data().unwrap().get_field_object(0).unwrap().unwrap()
            };
            assert_eq!(actual.as_raw(), source.as_raw());
            drop(actual);
            record.release().unwrap();
            drop(source);
            assert_eq!(
                counts.releases.load(Ordering::SeqCst),
                counts.addrefs.load(Ordering::SeqCst) + 1
            );
        }
    }

    #[test]
    fn private_query_guard_releases_successful_qi_and_preserves_other_failures() {
        Python::initialize();
        Python::attach(|py| {
            let (object, counts) = QueryProbe::new();
            let mut value = DynWinRTValue::new(dynwinrt::WinRTValue::Object(object));
            assert!(
                value
                    ._try_query_interface(&WinGUID(QueryProbe::SUPPORTED))
                    .unwrap()
            );
            assert_eq!(counts.addrefs.load(Ordering::SeqCst), 1);
            assert_eq!(counts.releases.load(Ordering::SeqCst), 1);
            assert!(
                !value
                    ._try_query_interface(&WinGUID(QueryProbe::UNKNOWN))
                    .unwrap()
            );
            let error = value
                ._try_query_interface(&WinGUID(QueryProbe::FAILURE))
                .unwrap_err();
            assert!(error.is_instance_of::<pyo3::exceptions::PyOSError>(py));
            assert_eq!(
                error
                    .value(py)
                    .getattr("winerror")
                    .unwrap()
                    .extract::<i32>()
                    .unwrap(),
                0x80004005u32 as i32
            );
            assert_eq!(counts.queries.load(Ordering::SeqCst), 3);
            assert_eq!(counts.addrefs.load(Ordering::SeqCst), 1);
            assert_eq!(counts.releases.load(Ordering::SeqCst), 1);

            for payload in [
                dynwinrt::WinRTValue::I32(5),
                dynwinrt::WinRTValue::HString("scalar".into()),
                dynwinrt::WinRTValue::Null,
                dynwinrt::WinRTValue::RawPtr(std::ptr::null_mut()),
            ] {
                assert!(
                    !DynWinRTValue::new(payload)
                        ._try_query_interface(&WinGUID(QueryProbe::SUPPORTED))
                        .unwrap()
                );
            }
            assert_eq!(counts.queries.load(Ordering::SeqCst), 3);

            value.release().unwrap();
            assert_eq!(counts.releases.load(Ordering::SeqCst), 2);
            let released = value
                ._try_query_interface(&WinGUID(QueryProbe::SUPPORTED))
                .unwrap_err();
            assert!(released.is_instance_of::<PyRuntimeError>(py));
            assert!(released.to_string().contains("released"));
        });
    }

    #[test]
    fn detached_invocation_releases_the_gil_on_the_same_native_thread() {
        use dynwinrt::{
            WinRtImplementation, WinRtImplementationPlan, WinRtInterfaceDefinition,
            WinRtMethodDefinition, WinRtThreadingPolicy,
        };
        use std::sync::Mutex;

        Python::initialize();
        Python::attach(|py| {
            let table = dynwinrt::MetadataTable::new();
            let iid = GUID::from_u128(0x96369f54_8eb6_48f0_abce_c1b211e627c3);
            let signature = dynwinrt::MethodSignature::new(&table).add_out(table.hstring());
            let interface = table
                .register_interface("Windows.Foundation.IStringable", iid)
                .add_method("ToString", signature.clone());
            let plan = WinRtImplementationPlan::new(
                vec![WinRtInterfaceDefinition {
                    name: "Windows.Foundation.IStringable".into(),
                    interface_type: interface.clone(),
                    required_iids: vec![],
                    methods: vec![WinRtMethodDefinition {
                        name: "ToString".into(),
                        vtable_index: 6,
                        signature,
                    }],
                }],
                WinRtThreadingPolicy::OwnerThread,
            )
            .unwrap();
            let observed = Arc::new(Mutex::new(Vec::new()));
            let callback_observed = observed.clone();
            let mut owner = WinRtImplementation::new(
                plan,
                Arc::new(move |_, slot, args| {
                    assert_eq!(slot, 6);
                    assert!(args.is_empty());
                    // Observe the native boundary before a Python handler's
                    // Python::attach would reacquire the GIL and mask its state.
                    let gil = unsafe { pyo3::ffi::PyGILState_Check() };
                    callback_observed
                        .lock()
                        .unwrap()
                        .push((gil, std::thread::current().id()));
                    Ok(vec![dynwinrt::WinRTValue::HString(
                        "native observer".into(),
                    )])
                }),
                None,
            )
            .unwrap();
            let receiver = Py::new(
                py,
                DynWinRTValue::new(owner.to_value().unwrap().cast(&iid).unwrap()),
            )
            .unwrap();
            let method = DynWinRTMethodHandle(interface.method(6).unwrap());
            let direct = method.invoke(py, receiver.clone_ref(py), vec![]).unwrap();
            let detached = method.invoke_detached(py, receiver, vec![]).unwrap();
            for result in [&direct, &detached] {
                assert!(matches!(
                    &result.borrow(py).0,
                    dynwinrt::WinRTValue::HString(value) if value == "native observer"
                ));
            }
            let invalid = method
                .invoke_detached(
                    py,
                    Py::new(py, DynWinRTValue::new(dynwinrt::WinRTValue::I32(0))).unwrap(),
                    vec![],
                )
                .err()
                .expect("non-object receiver must be rejected");
            assert!(invalid.is_instance_of::<PyRuntimeError>(py));
            assert!(invalid.to_string().contains("requires an Object value"));
            let thread = std::thread::current().id();
            assert_eq!(*observed.lock().unwrap(), vec![(1, thread), (0, thread)]);
            owner.dispose().unwrap();
        });
    }

    #[test]
    fn python_delegate_reports_unraisable_callback_errors() {
        let _serial = crate::errors::UNRAISABLE_HOOK_TEST_LOCK.lock().unwrap();
        Python::initialize();
        Python::attach(|py| {
            let locals = PyDict::new(py);
            py.run(
                c"captured = []\ndef hook(args):\n    captured.append(args)\ndef success():\n    pass\ndef failure():\n    raise RuntimeError('callback failed')",
                Some(&locals),
                Some(&locals),
            )
            .unwrap();

            let sys = py.import("sys").unwrap();
            let original_hook = sys.getattr("unraisablehook").unwrap().unbind();
            sys.setattr("unraisablehook", locals.get_item("hook").unwrap().unwrap())
                .unwrap();

            let success = create_python_delegate(
                GUID::zeroed(),
                Vec::new(),
                locals.get_item("success").unwrap().unwrap().unbind(),
            )
            .unwrap();
            let failure = create_python_delegate(
                GUID::zeroed(),
                Vec::new(),
                locals
                    .get_item("failure")
                    .unwrap()
                    .unwrap()
                    .clone()
                    .unbind(),
            )
            .unwrap();
            let cross_thread_failure = create_python_delegate(
                GUID::zeroed(),
                Vec::new(),
                locals.get_item("failure").unwrap().unwrap().unbind(),
            )
            .unwrap();

            let success_result = unsafe { invoke_delegate(&success) };
            let failure_result = unsafe { invoke_delegate(&failure) };
            let cross_thread_result = py.detach(|| {
                std::thread::spawn(move || unsafe { invoke_delegate(&cross_thread_failure) })
                    .join()
                    .unwrap()
            });
            sys.setattr("unraisablehook", original_hook).unwrap();

            assert_eq!(success_result, windows::core::HRESULT(0));
            assert_eq!(failure_result, PYWINRT_E_UNRAISABLE_PYTHON_EXCEPTION);
            assert_eq!(cross_thread_result, PYWINRT_E_UNRAISABLE_PYTHON_EXCEPTION);
            let captured = locals
                .get_item("captured")
                .unwrap()
                .unwrap()
                .cast_into::<pyo3::types::PyList>()
                .unwrap();
            assert_eq!(captured.len(), 2);
            assert!(
                captured
                    .get_item(0)
                    .unwrap()
                    .getattr("exc_value")
                    .unwrap()
                    .is_instance_of::<pyo3::exceptions::PyRuntimeError>()
            );
        });
    }

    #[test]
    fn managed_native_owner_never_releases_com_on_foreign_thread_or_without_gil() {
        Python::initialize();

        for foreign in [false, true] {
            let (source, counts) = QueryProbe::new();
            let previous = MANAGED_APARTMENT_DEPTH.with(|depth| depth.replace(1));
            assert_eq!(previous, 0);
            let mut owned =
                DynWinRTValue::new_managed(dynwinrt::WinRTValue::Object(source.clone()), false);
            MANAGED_APARTMENT_DEPTH.with(|depth| depth.set(previous));
            if foreign {
                std::thread::spawn(move || {
                    assert!(owned.release().is_err());
                    assert!(!owned.is_released());
                    drop(owned);
                })
                .join()
                .unwrap();
            } else {
                assert_eq!(unsafe { pyo3::ffi::PyGILState_Check() }, 0);
                drop(owned);
            }
            assert_eq!(counts.releases.load(Ordering::SeqCst), 0);
            drop(source);
            assert_eq!(counts.releases.load(Ordering::SeqCst), 1);
            assert_eq!(counts.addrefs.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn no_gil_apartment_finalizer_retains_initialization_for_owner_thread_cleanup() {
        Python::initialize();
        std::thread::spawn(|| {
            assert_eq!(unsafe { pyo3::ffi::PyGILState_Check() }, 0);
            let mut apartment = RoApartment::new(Some(1)).unwrap();
            apartment.initialize().unwrap();
            assert_eq!(managed_apartment_depth(), 1);
            drop(apartment);
            assert_eq!(managed_apartment_depth(), 1);
            // This Rust-only test has no initialized Python owner registry.
            unsafe { windows::Win32::System::WinRT::RoUninitialize() };
            MANAGED_APARTMENT_DEPTH.with(|depth| depth.set(0));
            assert_eq!(managed_apartment_depth(), 0);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn foreign_python_array_and_vector_inputs_reject_before_com_addref() {
        Python::initialize();
        let (source, counts) = QueryProbe::new();
        let (value, element_type, key_type) = Python::attach(|py| {
            (
                Py::new(
                    py,
                    DynWinRTValue::new_managed(dynwinrt::WinRTValue::Object(source.clone()), false),
                )
                .unwrap(),
                Py::new(py, DynWinRTType(TABLE.interface(QueryProbe::SUPPORTED))).unwrap(),
                Py::new(py, DynWinRTType(TABLE.hstring())).unwrap(),
            )
        });
        let (value, element_type, key_type) = thread::spawn(move || {
            Python::attach(|py| {
                let module = PyModule::new(py, "native_input_probe").unwrap();
                module.add_class::<DynWinRTValue>().unwrap();
                module.add_class::<DynWinRTArray>().unwrap();
                module.add_class::<DynWinRTType>().unwrap();
                for name in ["from_values", "from_object_values"] {
                    let error = module
                        .getattr("DynWinRTArray")
                        .unwrap()
                        .call_method1(
                            name,
                            (vec![value.clone_ref(py)], element_type.clone_ref(py)),
                        )
                        .unwrap_err();
                    assert!(error.is_instance_of::<PyRuntimeError>(py));
                    assert!(error.to_string().contains("owning COM apartment thread"));
                }
                let error = module
                    .getattr("DynWinRTValue")
                    .unwrap()
                    .call_method1(
                        "create_vector",
                        (vec![value.clone_ref(py)], element_type.clone_ref(py)),
                    )
                    .unwrap_err();
                assert!(error.is_instance_of::<PyRuntimeError>(py));
                assert!(error.to_string().contains("owning COM apartment thread"));
                let key = Py::new(
                    py,
                    DynWinRTValue::new(dynwinrt::WinRTValue::HString("key".into())),
                )
                .unwrap();
                let error = module
                    .getattr("DynWinRTValue")
                    .unwrap()
                    .call_method1(
                        "create_map",
                        (
                            vec![key],
                            vec![value.clone_ref(py)],
                            key_type.clone_ref(py),
                            element_type.clone_ref(py),
                        ),
                    )
                    .unwrap_err();
                assert!(error.is_instance_of::<PyRuntimeError>(py));
                assert!(error.to_string().contains("owning COM apartment thread"));
                let error =
                    native_outputs(py, "implementation callback", vec![value.clone_ref(py)])
                        .unwrap_err();
                assert!(error.is_instance_of::<PyRuntimeError>(py));
                assert!(error.to_string().contains("owning COM apartment thread"));
            });
            (value, element_type, key_type)
        })
        .join()
        .unwrap();
        assert_eq!(
            counts.wrong_thread_addrefs.load(Ordering::SeqCst),
            0,
            "a foreign Python argument was cloned before its thread check"
        );
        assert_eq!(counts.wrong_thread_releases.load(Ordering::SeqCst), 0);
        Python::attach(|py| value.borrow_mut(py).release().unwrap());
        drop((value, element_type, key_type, source));
    }

    #[test]
    fn real_interpreter_shutdown_quarantines_native_owner_and_apartment() {
        if std::env::var("DYNWINRT_FINALIZE_CHILD").as_deref() != Ok("1") {
            for mode in ["explicit", "skipped", "explicit-last-alias"] {
                let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "runtime::tests::real_interpreter_shutdown_quarantines_native_owner_and_apartment",
                        "--nocapture",
                    ])
                    .env("DYNWINRT_FINALIZE_CHILD", "1")
                    .env("DYNWINRT_FINALIZE_GATE_MODE", mode)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                while child.try_wait().unwrap().is_none() {
                    if std::time::Instant::now() >= deadline {
                        child.kill().unwrap();
                        let output = child.wait_with_output().unwrap();
                        panic!(
                            "Python finalization child deadlocked ({mode}):\n{}\n{}",
                            String::from_utf8_lossy(&output.stdout),
                            String::from_utf8_lossy(&output.stderr)
                        );
                    }
                    thread::sleep(std::time::Duration::from_millis(25));
                }
                let child = child.wait_with_output().unwrap();
                assert!(
                    child.status.success(),
                    "{mode}:\n{}\n{}",
                    String::from_utf8_lossy(&child.stdout),
                    String::from_utf8_lossy(&child.stderr)
                );
                assert!(
                    String::from_utf8_lossy(&child.stdout)
                        .contains(&format!("real-Py_FinalizeEx-quarantined-{mode}"))
                );
            }
            return;
        }

        Python::initialize();
        Python::attach(initialize_embedded_binding);
        let mut apartment = RoApartment::new(Some(1)).unwrap();
        apartment.initialize().unwrap();
        let (source, counts) = QueryProbe::new();
        let owned = DynWinRTValue::new_managed(dynwinrt::WinRTValue::Object(source.clone()), false);
        let values = DynWinRTArray(
            Some(dynwinrt::ArrayData::from_values(
                TABLE.interface(IUnknown::IID),
                &[dynwinrt::WinRTValue::Object(source.clone())],
            )),
            Some(thread::current().id()),
            false,
        );
        let inner_type = TABLE.struct_type(
            "Tests.FinalizationNested",
            &[TABLE.interface(IUnknown::IID)],
        );
        let outer_type = TABLE.struct_type("Tests.FinalizationRecord", &[inner_type.clone()]);
        let mut inner = inner_type.default_value();
        inner.set_field_object(0, Some(&source)).unwrap();
        let mut record = outer_type.default_value();
        record.set_field_struct_checked(0, &inner).unwrap();
        drop(inner);
        let structured = DynWinRTStruct(Some(record), Some(thread::current().id()), false);
        let cotaskmem = QueryProbe::received_finalization_array(&source, false);
        let cotaskmem_nested = QueryProbe::received_finalization_array(&source, true);
        let callback =
            Python::attach(|py| py.eval(c"lambda *args: None", None, None).unwrap().unbind());
        let mut delegate = DynWinRtDelegate(
            Some(create_python_delegate(GUID::zeroed(), vec![], callback).unwrap()),
            Some(thread::current().id()),
        );
        let (mut element_factory, factory_interface) = Python::attach(|py| {
            let callback = py.eval(c"lambda args: None", None, None).unwrap().unbind();
            let native = DynWinRtElementFactory::create(
                py,
                &WinGUID(QueryProbe::SUPPORTED),
                callback.clone_ref(py),
                callback,
            )
            .unwrap();
            let (value, callbacks, owner_thread) = {
                let mut factory = native.borrow_mut(py);
                (
                    factory.value.take(),
                    factory.callbacks.clone(),
                    factory.owner_thread,
                )
            };
            drop(native);
            let interface = value
                .as_ref()
                .unwrap()
                .cast(&dynwinrt::element_factory::IID_IELEMENT_FACTORY)
                .unwrap()
                .as_object()
                .unwrap();
            (
                DynWinRtElementFactory {
                    value,
                    callbacks,
                    owner_thread,
                },
                interface,
            )
        });
        let recycle = dynwinrt::MethodSignature::new(&*TABLE)
            .add_in(TABLE.object())
            .build(7);
        let releases_before = counts.releases.load(Ordering::SeqCst);

        let mode = std::env::var("DYNWINRT_FINALIZE_GATE_MODE").unwrap();
        let delegate_alias = if mode == "explicit-last-alias" {
            let alias = delegate.0.as_ref().unwrap().clone();
            delegate.release().unwrap();
            element_factory._release_apartment_owner().unwrap();
            Some(alias)
        } else {
            None
        };
        let active_delegate = delegate_alias.as_ref().or(delegate.0.as_ref()).unwrap();
        if mode != "skipped" {
            close_native_callback_gate().unwrap();
            assert_eq!(unsafe { pyo3::ffi::Py_IsInitialized() }, 1);
            assert_eq!(
                unsafe { invoke_delegate(active_delegate) },
                PYWINRT_E_INTERPRETER_CLOSED
            );
            let error = recycle
                .call_dynamic(
                    factory_interface.as_raw(),
                    &[dynwinrt::WinRTValue::Object(factory_interface.clone())],
                )
                .unwrap_err();
            assert_eq!(error.code(), PYWINRT_E_INTERPRETER_CLOSED);
        }
        unsafe { pyo3::ffi::PyGILState_Ensure() };
        assert_eq!(unsafe { pyo3::ffi::Py_FinalizeEx() }, 0);
        assert_eq!(unsafe { pyo3::ffi::Py_IsInitialized() }, 0);
        assert_eq!(
            unsafe { invoke_delegate(active_delegate) },
            windows::core::HRESULT(0x80000013u32 as i32),
            "a native callback must fail closed once Python has finalized"
        );
        let error = recycle
            .call_dynamic(
                factory_interface.as_raw(),
                &[dynwinrt::WinRTValue::Object(factory_interface.clone())],
            )
            .unwrap_err();
        assert_eq!(error.code(), PYWINRT_E_INTERPRETER_CLOSED);
        if mode == "explicit-last-alias" {
            drop(delegate_alias);
            drop(factory_interface);
            assert_eq!(Arc::strong_count(&element_factory.callbacks), 1);
        } else {
            std::mem::forget(factory_interface);
        }
        drop((
            owned,
            values,
            structured,
            cotaskmem,
            cotaskmem_nested,
            delegate,
            element_factory,
        ));
        assert_eq!(counts.releases.load(Ordering::SeqCst), releases_before);
        assert_eq!(counts.wrong_thread_releases.load(Ordering::SeqCst), 0);
        drop(apartment);
        assert_eq!(managed_apartment_depth(), 1);
        drop(source);
        assert_eq!(counts.releases.load(Ordering::SeqCst), releases_before + 1);
        println!("real-Py_FinalizeEx-quarantined-{mode}");
    }

    #[test]
    fn embedded_public_callback_gate_survives_real_finalization() {
        fn native_reference_count(object: &IUnknown) -> u32 {
            let raw = object.as_raw();
            let vtable = unsafe { *(raw as *const *const windows::core::IUnknown_Vtbl) };
            let added = unsafe { ((*vtable).AddRef)(raw) };
            let remaining = unsafe { ((*vtable).Release)(raw) };
            assert_eq!(added, remaining + 1);
            remaining
        }

        fn assert_native_callbacks_closed(aliases: &[IUnknown; 3]) {
            let delegate = dynwinrt::WinRTValue::Object(aliases[0].clone());
            assert_eq!(
                unsafe { invoke_delegate(&delegate) },
                PYWINRT_E_INTERPRETER_CLOSED
            );
            drop(delegate);

            let recycle = dynwinrt::MethodSignature::new(&*TABLE)
                .add_in(TABLE.object())
                .build(7);
            let error = recycle
                .call_dynamic(
                    aliases[1].as_raw(),
                    &[dynwinrt::WinRTValue::Object(aliases[1].clone())],
                )
                .unwrap_err();
            assert_eq!(error.code(), PYWINRT_E_INTERPRETER_CLOSED);

            let to_string = dynwinrt::MethodSignature::new(&*TABLE)
                .add_out(TABLE.hstring())
                .build(6);
            let error = to_string
                .call_dynamic(aliases[2].as_raw(), &[])
                .unwrap_err();
            assert_eq!(error.code(), PYWINRT_E_INTERPRETER_CLOSED);
        }

        if std::env::var("DYNWINRT_EMBEDDED_GATE_CHILD").as_deref() != Ok("1") {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "runtime::tests::embedded_public_callback_gate_survives_real_finalization",
                    "--nocapture",
                ])
                .env("DYNWINRT_EMBEDDED_GATE_CHILD", "1")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while child.try_wait().unwrap().is_none() {
                if std::time::Instant::now() >= deadline {
                    child.kill().unwrap();
                    let output = child.wait_with_output().unwrap();
                    panic!(
                        "embedded host child deadlocked:\n{}\n{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                thread::sleep(std::time::Duration::from_millis(25));
            }
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "embedded host child exited {:?}:\n{}\n{}",
                output.status.code(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("embedded-public-gate-complete")
            );
            return;
        }

        Python::initialize();
        let owner_thread = thread::current().id();
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.unwrap();
        let (aliases, attempts_after_gate) = Python::attach(|py| {
            initialize_embedded_binding(py);

            let locals = PyDict::new(py);
            let script = std::ffi::CString::new(
                r#"
import threading
from dynwinrt import (
    DynWinRTImplementation, DynWinRTImplementationMethod, DynWinRTInterfacePlan,
    DynWinRTMethodSig, DynWinRTType, DynWinRTValue, DynWinRtDelegate,
    DynWinRtElementFactory, RoApartment, WinGUID, shutdown_python_callbacks,
)

apartment = RoApartment(1)
apartment.__enter__()
calls = []
errors = []
started = threading.Event()
proceed = threading.Event()
object_type = DynWinRTType.object()
stringable_iid = WinGUID.parse('96369f54-8eb6-48f0-abce-c1b211e627c3')
delegate_iid = WinGUID.parse('13fd99ec-a997-4497-aabc-247345013f26')
factory_iid = WinGUID.parse('75faba47-2cf2-54ae-91e6-0581556fddaa')
delegate_sig = DynWinRTMethodSig().add_in(object_type).add_in(object_type)
string_sig = DynWinRTMethodSig().add_out(DynWinRTType.hstring())
string_type = DynWinRTType.register_interface(
    'Tests.IEmbeddedHostStringable', stringable_iid
).add_method('ToString', string_sig)
string_plan = DynWinRTInterfacePlan.create(
    'Tests.IEmbeddedHostStringable', string_type,
    [DynWinRTImplementationMethod('ToString', 6, string_sig)],
)
factory_type = DynWinRTType.register_interface(
    'Tests.IEmbeddedHostElementFactory', factory_iid
).add_method(
    'GetElement', DynWinRTMethodSig().add_in(object_type).add_out(object_type)
).add_method('RecycleElement', DynWinRTMethodSig().add_in(object_type))

def delegate_callback(_first, _second):
    calls.append('delegate')

def implementation_callback(*_args):
    calls.append('implementation')
    return [DynWinRTValue.from_hstring('alive')]

def recycle(_args):
    calls.append('factory')
    started.set()
    assert proceed.wait(8), 'host did not settle its native callback'

delegate = DynWinRtDelegate.create(
    delegate_iid, [object_type, object_type], delegate_callback
)
delegate_view = delegate.to_value().cast(delegate_iid)
factory = DynWinRtElementFactory.create(
    stringable_iid, lambda _args: DynWinRTValue.null_value(), recycle
)
factory_view = factory.to_value().cast(factory_iid)
implementation = DynWinRTImplementation.create(
    [string_plan], implementation_callback
)
implementation_view = implementation.to_value().cast(stringable_iid)

def invoke_delegate():
    return delegate_view.invoke_delegate(
        delegate_iid, delegate_sig,
        [DynWinRTValue.null_value(), DynWinRTValue.null_value()],
    )

assert invoke_delegate() == []
assert string_type.method(6).invoke(implementation_view, []).to_string() == 'alive'

def invoke_factory():
    with RoApartment(1):
        factory_type.method(7).invoke(factory_view, [factory_view])

def worker():
    try:
        invoke_factory()
    except BaseException as error:
        errors.append(error)

thread = threading.Thread(target=worker)
thread.start()
assert started.wait(5), 'native factory callback did not start'
try:
    shutdown_python_callbacks()
except RuntimeError as error:
    assert 'callback(s) are in flight' in str(error), error
else:
    raise AssertionError('public gate closed while a callback was in flight')
assert invoke_delegate() == [], 'failed gate attempt closed unrelated callbacks'
proceed.set()
thread.join(10)
assert not thread.is_alive() and not errors, errors

delegate.release()
factory._release_apartment_owner()
implementation.release()
assert invoke_delegate() == []
assert string_type.method(6).invoke(implementation_view, []).to_string() == 'alive'
factory_type.method(7).invoke(factory_view, [factory_view])
assert calls == [
    'delegate', 'implementation', 'factory', 'delegate',
    'delegate', 'implementation', 'factory',
]
shutdown_python_callbacks()
shutdown_python_callbacks()
state = {
    'apartment': apartment,
    'views': (delegate_view, factory_view, implementation_view),
    'scalar': DynWinRTValue.from_u32(77),
    'calls': calls,
}
"#,
            )
            .unwrap();
            py.run(script.as_c_str(), Some(&locals), None).unwrap();
            let state = locals.get_item("state").unwrap().unwrap();
            let views = state.get_item("views").unwrap();
            // The host AddRefs each borrowed Python view before apartment cleanup.
            let aliases: [IUnknown; 3] = std::array::from_fn(|index| {
                let raw = views
                    .get_item(index)
                    .unwrap()
                    .call_method0("as_raw")
                    .unwrap()
                    .extract::<i64>()
                    .unwrap() as usize as *mut c_void;
                unsafe { IUnknown::from_raw_borrowed(&raw) }
                    .expect("a live native alias")
                    .clone()
            });
            state
                .get_item("apartment")
                .unwrap()
                .call_method0("close")
                .unwrap();
            assert_eq!(managed_apartment_depth(), 0);
            for index in 0..3 {
                assert!(
                    views
                        .get_item(index)
                        .unwrap()
                        .call_method0("is_released")
                        .unwrap()
                        .extract::<bool>()
                        .unwrap()
                );
            }
            assert_eq!(
                state
                    .get_item("scalar")
                    .unwrap()
                    .call_method0("to_u32")
                    .unwrap()
                    .extract::<u32>()
                    .unwrap(),
                77
            );
            for alias in &aliases {
                assert_eq!(native_reference_count(alias), 1);
            }
            let attempts_after_gate = CALLBACK_ATTACH_ATTEMPTS.load(Ordering::SeqCst);
            assert_native_callbacks_closed(&aliases);
            assert_eq!(
                CALLBACK_ATTACH_ATTEMPTS.load(Ordering::SeqCst),
                attempts_after_gate
            );
            assert_eq!(state.get_item("calls").unwrap().len().unwrap(), 7);
            (aliases, attempts_after_gate)
        });

        assert_eq!(thread::current().id(), owner_thread);
        unsafe { pyo3::ffi::PyGILState_Ensure() };
        assert_eq!(unsafe { pyo3::ffi::Py_FinalizeEx() }, 0);
        assert_eq!(unsafe { pyo3::ffi::Py_IsInitialized() }, 0);
        assert_native_callbacks_closed(&aliases);
        assert_eq!(
            CALLBACK_ATTACH_ATTEMPTS.load(Ordering::SeqCst),
            attempts_after_gate
        );
        for alias in &aliases {
            assert_eq!(native_reference_count(alias), 1);
        }
        drop(aliases);
        assert_eq!(thread::current().id(), owner_thread);
        unsafe { windows::Win32::System::WinRT::RoUninitialize() };
        let mut apartment_type = windows::Win32::System::Com::APTTYPE_CURRENT;
        let mut qualifier = windows::Win32::System::Com::APTTYPEQUALIFIER_NONE;
        let error = unsafe {
            windows::Win32::System::Com::CoGetApartmentType(&mut apartment_type, &mut qualifier)
        }
        .unwrap_err();
        assert_eq!(
            error.code(),
            windows::Win32::Foundation::CO_E_NOTINITIALIZED
        );
        println!(
            "embedded-public-gate-complete: {attempts_after_gate} pre-gate Python attachments, \
             none after gate, three native aliases released on their owner thread"
        );
    }
}
