// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::ffi::CString;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};

use dynwinrt::{
    WinRtImplementation, WinRtImplementationCallback, WinRtImplementationPlan,
    WinRtInterfaceDefinition, WinRtMethodDefinition, WinRtThreadingPolicy,
};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::pyclass::{PyTraverseError, PyVisit};
use pyo3::types::PyList;
use windows::core::{Error, HRESULT};

use crate::errors::map_windows_error;
use crate::runtime::{
    DynWinRTMethodSig, DynWinRTType, DynWinRTValue, PYWINRT_E_UNRAISABLE_PYTHON_EXCEPTION, WinGUID,
    wrap_python_callback_context,
};

const RO_E_CLOSED: HRESULT = HRESULT(0x80000013_u32 as i32);
const E_FAIL: HRESULT = HRESULT(0x80004005_u32 as i32);

fn closed_error() -> Error {
    Error::new(
        RO_E_CLOSED,
        "Python WinRT implementation callbacks are disconnected or the interpreter is shutting down",
    )
}

#[pyclass(frozen, from_py_object, module = "dynwinrt")]
#[derive(Clone)]
pub struct DynWinRTImplementationMethod(WinRtMethodDefinition);

#[pymethods]
impl DynWinRTImplementationMethod {
    #[new]
    fn new(name: String, vtable_index: usize, signature: &DynWinRTMethodSig) -> Self {
        Self(WinRtMethodDefinition {
            name,
            vtable_index,
            signature: signature.0.clone(),
        })
    }

    #[getter]
    fn name(&self) -> &str {
        &self.0.name
    }

    #[getter]
    fn vtable_index(&self) -> usize {
        self.0.vtable_index
    }

    #[getter]
    fn signature(&self) -> DynWinRTMethodSig {
        DynWinRTMethodSig(self.0.signature.clone())
    }
}

#[pyclass(frozen, from_py_object, module = "dynwinrt")]
#[derive(Clone)]
pub struct DynWinRTInterfacePlan(WinRtInterfaceDefinition);

#[pymethods]
impl DynWinRTInterfacePlan {
    #[staticmethod]
    #[pyo3(
        signature = (name, interface_type, methods, required_iids=Vec::new()),
        text_signature = "(name, interface_type, methods, required_iids=())"
    )]
    fn create(
        name: String,
        interface_type: &DynWinRTType,
        methods: Vec<DynWinRTImplementationMethod>,
        required_iids: Vec<WinGUID>,
    ) -> PyResult<Self> {
        let definition = WinRtInterfaceDefinition {
            name,
            interface_type: interface_type.0.clone(),
            required_iids: required_iids.into_iter().map(|iid| iid.0).collect(),
            methods: methods.into_iter().map(|method| method.0).collect(),
        };
        WinRtImplementationPlan::validate_interface(&definition).map_err(map_windows_error)?;
        Ok(Self(definition))
    }

    #[getter]
    fn name(&self) -> &str {
        &self.0.name
    }

    #[getter]
    fn interface_type(&self) -> DynWinRTType {
        DynWinRTType(self.0.interface_type.clone())
    }

    #[getter]
    fn methods(&self) -> Vec<DynWinRTImplementationMethod> {
        self.0
            .methods
            .iter()
            .cloned()
            .map(DynWinRTImplementationMethod)
            .collect()
    }

    #[getter]
    fn required_iids(&self) -> Vec<WinGUID> {
        self.0.required_iids.iter().copied().map(WinGUID).collect()
    }
}

#[derive(Default)]
struct InterpreterState {
    stopping: AtomicBool,
    callbacks: Mutex<Vec<Weak<CallbackCell>>>,
}

impl InterpreterState {
    fn register(&self, callback: &Arc<CallbackCell>) -> PyResult<()> {
        let mut callbacks = self.callbacks.lock().map_err(|_| {
            PyRuntimeError::new_err("WinRT implementation shutdown registry is poisoned")
        })?;
        if self.stopping.load(Ordering::Acquire) {
            return Err(PyRuntimeError::new_err(
                "cannot create a WinRT implementation while the interpreter is shutting down",
            ));
        }
        callbacks.retain(|callback| callback.strong_count() != 0);
        callbacks.push(Arc::downgrade(callback));
        Ok(())
    }

    fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        let callbacks = {
            let mut registry = self
                .callbacks
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            std::mem::take(&mut *registry)
        };
        for callback in callbacks {
            if let Some(callback) = callback.upgrade() {
                callback.clear();
            }
        }
    }
}

#[pyclass(frozen, name = "_DynWinRTImplementationRuntime")]
struct ImplementationRuntime(Arc<InterpreterState>);

#[pymethods]
impl ImplementationRuntime {
    fn shutdown(&self) {
        self.0.shutdown();
    }
}

struct CallbackCell {
    // Exactly one Python reference is shared by the native callback and the GC
    // visitor. The owner and shutdown registry hold only Weak references here.
    callback: Mutex<Option<Py<PyAny>>>,
    interpreter: Arc<InterpreterState>,
}

impl CallbackCell {
    fn clear(&self) {
        let callback = self
            .callback
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        // Decref may execute a handler's finalizer and reenter dispose().
        drop(callback);
    }

    fn invoke(
        &self,
        interface_index: usize,
        vtable_index: usize,
        args: &[dynwinrt::WinRTValue],
    ) -> windows::core::Result<Vec<dynwinrt::WinRTValue>> {
        if self.interpreter.stopping.load(Ordering::Acquire) {
            return Err(closed_error());
        }
        Python::try_attach(|py| {
            if self.interpreter.stopping.load(Ordering::Acquire) {
                return Err(closed_error());
            }
            let callback = self
                .callback
                .lock()
                .map_err(|_| Error::new(E_FAIL, "Python WinRT callback state is poisoned"))?
                .as_ref()
                .map(|callback| callback.clone_ref(py))
                .ok_or_else(closed_error)?;
            let result = (|| -> PyResult<Vec<dynwinrt::WinRTValue>> {
                let inputs = args
                    .iter()
                    .map(|value| Py::new(py, DynWinRTValue(value.clone())))
                    .collect::<PyResult<Vec<_>>>()?;
                let inputs = PyList::new(py, inputs)?;
                let outputs = callback.call1(py, (interface_index, vtable_index, inputs))?;
                Ok(outputs
                    .extract::<Vec<DynWinRTValue>>(py)?
                    .into_iter()
                    .map(|value| value.0)
                    .collect())
            })();
            result.map_err(|error| {
                let message = format!(
                    "Python WinRT implementation callback at interface {interface_index}, \
                     slot {vtable_index}: {error}"
                );
                error.write_unraisable(py, Some(callback.bind(py)));
                Error::new(PYWINRT_E_UNRAISABLE_PYTHON_EXCEPTION, message)
            })
        })
        .unwrap_or_else(|| Err(closed_error()))
    }
}

#[pyclass(weakref, module = "dynwinrt")]
pub struct DynWinRTImplementation {
    native: Mutex<Option<WinRtImplementation>>,
    callback: Weak<CallbackCell>,
    interpreter: Arc<InterpreterState>,
}

struct NativeLease<'a> {
    slot: &'a Mutex<Option<WinRtImplementation>>,
    value: Option<WinRtImplementation>,
}

impl Drop for NativeLease<'_> {
    fn drop(&mut self) {
        let mut slot = self.slot.lock().unwrap_or_else(|error| error.into_inner());
        debug_assert!(slot.is_none());
        *slot = self.value.take();
    }
}

impl DynWinRTImplementation {
    fn with_native<T>(
        &self,
        action: impl FnOnce(&mut WinRtImplementation) -> PyResult<T>,
    ) -> PyResult<T> {
        let value = self
            .native
            .lock()
            .map_err(|_| PyRuntimeError::new_err("WinRT implementation owner state is poisoned"))?
            .take()
            .ok_or_else(|| {
                PyRuntimeError::new_err("WinRT implementation owner has an active native operation")
            })?;
        let mut lease = NativeLease {
            slot: &self.native,
            value: Some(value),
        };
        // Restore the controller even on unwind, without holding a binding
        // state lock across native AddRef/Release, disconnection, or destruction.
        action(lease.value.as_mut().expect("the lease owns the controller"))
    }

    fn disconnect_native(&self, release: bool, only_if_unique: bool) -> PyResult<()> {
        // Keep the cell alive until the controller is restored. Dropping the
        // captured handler can then reenter this owner through a finalizer.
        // An in-flight native callback retains its own root until it returns.
        let callback = self.callback.upgrade();
        self.with_native(|native| {
            if only_if_unique && !native.has_unique_native_reference() {
                return Ok(());
            }
            if release {
                native.dispose().map_err(map_windows_error)?;
            } else {
                native.disconnect().map_err(map_windows_error)?;
            }
            Ok(())
        })?;
        drop(callback);
        Ok(())
    }
}

#[pymethods]
impl DynWinRTImplementation {
    #[staticmethod]
    #[pyo3(signature = (interfaces, callback, runtime_class_name=None))]
    fn create(
        py: Python<'_>,
        interfaces: Vec<DynWinRTInterfacePlan>,
        callback: Py<PyAny>,
        runtime_class_name: Option<&str>,
    ) -> PyResult<Self> {
        // PyGILState attachment targets the main interpreter. Do not accept a
        // subinterpreter-owned callable and later attach to the wrong one.
        if unsafe { pyo3::ffi::PyInterpreterState_Get() != pyo3::ffi::PyInterpreterState_Main() } {
            return Err(PyRuntimeError::new_err(
                "WinRT interface implementations do not support Python subinterpreters",
            ));
        }
        let module = py.import("dynwinrt.dynwinrt")?;
        let runtime = module.getattr("_dynwinrt_implementation_runtime")?;
        let interpreter = runtime
            .extract::<PyRef<'_, ImplementationRuntime>>()?
            .0
            .clone();
        if interpreter.stopping.load(Ordering::Acquire) {
            return Err(PyRuntimeError::new_err(
                "cannot create a WinRT implementation while the interpreter is shutting down",
            ));
        }
        let plan = WinRtImplementationPlan::new(
            interfaces
                .into_iter()
                .map(|interface| interface.0)
                .collect(),
            WinRtThreadingPolicy::OwnerThread,
        )
        .map_err(map_windows_error)?;
        let callback = module
            .getattr("_dynwinrt_checked_implementation_callback")?
            .call1((callback,))?
            .unbind();
        let callback = Arc::new(CallbackCell {
            callback: Mutex::new(Some(wrap_python_callback_context(py, callback)?)),
            interpreter: interpreter.clone(),
        });
        let native_callback = callback.clone();
        let native_callback: WinRtImplementationCallback =
            Arc::new(move |interface, slot, args| native_callback.invoke(interface, slot, args));
        let native = WinRtImplementation::new(plan, native_callback, runtime_class_name)
            .map_err(map_windows_error)?;
        interpreter.register(&callback)?;
        Ok(Self {
            native: Mutex::new(Some(native)),
            callback: Arc::downgrade(&callback),
            interpreter,
        })
    }

    fn to_value(&self) -> PyResult<DynWinRTValue> {
        self.with_native(|native| {
            native
                .to_value()
                .map(DynWinRTValue)
                .map_err(map_windows_error)
        })
    }

    /// Drop this owner's native reference without disconnecting retained views.
    fn release(&self) -> PyResult<()> {
        let callback = self.callback.upgrade();
        self.with_native(|native| {
            native.release();
            Ok(())
        })?;
        drop(callback);
        Ok(())
    }

    /// Disconnect every native view and release this owner's reference.
    fn dispose(&self) -> PyResult<()> {
        self.disconnect_native(true, false)
    }

    fn disconnect(&self) -> PyResult<()> {
        self.disconnect_native(false, false)
    }

    #[getter]
    fn is_closed(&self) -> PyResult<bool> {
        Ok(self.interpreter.stopping.load(Ordering::Acquire)
            || self.with_native(|native| Ok(native.is_closed()))?)
    }

    fn take_error(&self) -> PyResult<Option<String>> {
        self.with_native(|native| Ok(native.take_error()))
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyResult<PyRef<'_, Self>> {
        if slf.is_closed()? {
            return Err(map_windows_error(closed_error()));
        }
        // A released owner may still have live native aliases.
        slf.with_native(|native| native.to_value().map_err(map_windows_error))?;
        Ok(slf)
    }

    fn __exit__(
        &self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        self.dispose()?;
        Ok(false)
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        // Native aliases are external roots, not Python-owned graph edges.
        // try_lock also avoids GC deadlocks during reentrant native cleanup.
        if let Ok(slot) = self.native.try_lock() {
            if let Some(native) = slot.as_ref() {
                if native.has_unique_native_reference() {
                    if let Some(callback) = self.callback.upgrade() {
                        if let Ok(captured) = callback.callback.try_lock() {
                            if let Some(captured) = captured.as_ref() {
                                visit.call(captured)?;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn __clear__(slf: &Bound<'_, Self>) {
        // Recheck: a finalizer may have retained a native alias since traverse.
        let result = (|| -> PyResult<()> { slf.try_borrow()?.disconnect_native(true, true) })();
        if let Err(error) = result {
            error.write_unraisable(slf.py(), Some(slf.as_any()));
        }
    }
}

pub(crate) fn init(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<DynWinRTImplementationMethod>()?;
    module.add_class::<DynWinRTInterfacePlan>()?;
    module.add_class::<DynWinRTImplementation>()?;
    let runtime = Py::new(
        module.py(),
        ImplementationRuntime(Arc::new(InterpreterState::default())),
    )?;
    module.py().import("atexit")?.call_method1(
        "register",
        (runtime.bind(module.py()).getattr("shutdown")?,),
    )?;
    module.add("_dynwinrt_implementation_runtime", runtime)?;
    let source = CString::new(include_str!("implementation.py")).expect("Python source has no NUL");
    module.py().run(&source, Some(&module.dict()), None)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::{Barrier, mpsc};
    use std::time::Duration;

    use pyo3::types::PyDict;
    use windows::core::Interface;

    use super::*;

    #[test]
    fn gc_cleanup_reports_unexpected_owner_errors_as_unraisable() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<DynWinRTImplementation>();

        let _serial = crate::errors::UNRAISABLE_HOOK_TEST_LOCK.lock().unwrap();
        Python::initialize();
        Python::attach(|py| {
            let table = dynwinrt::MetadataTable::new();
            let plan = WinRtImplementationPlan::new(
                vec![WinRtInterfaceDefinition {
                    name: "Windows.Foundation.IClosable".into(),
                    interface_type: table.interface(windows::Foundation::IClosable::IID),
                    required_iids: vec![],
                    methods: vec![WinRtMethodDefinition {
                        name: "Close".into(),
                        vtable_index: 6,
                        signature: dynwinrt::MethodSignature::new(&table),
                    }],
                }],
                WinRtThreadingPolicy::OwnerThread,
            )
            .unwrap();
            let native =
                WinRtImplementation::new(plan, Arc::new(|_, _, _| Ok(vec![])), None).unwrap();
            let owner = Bound::new(
                py,
                DynWinRTImplementation {
                    native: Mutex::new(Some(native)),
                    callback: Weak::new(),
                    interpreter: Arc::new(InterpreterState::default()),
                },
            )
            .unwrap();
            let poisoned = catch_unwind(AssertUnwindSafe(|| {
                let owner = owner.borrow();
                let _guard = owner.native.lock().unwrap();
                panic!("inject a poisoned owner lock");
            }));
            assert!(poisoned.is_err());

            let captured = PyList::empty(py);
            let sys = py.import("sys").unwrap();
            let previous_hook = sys.getattr("unraisablehook").unwrap();
            sys.setattr("unraisablehook", captured.getattr("append").unwrap())
                .unwrap();
            DynWinRTImplementation::__clear__(&owner);
            sys.setattr("unraisablehook", previous_hook).unwrap();

            assert_eq!(captured.len(), 1);
            let record = captured.get_item(0).unwrap();
            let error = record.getattr("exc_value").unwrap();
            assert!(error.is_instance_of::<PyRuntimeError>());
            assert!(error.str().unwrap().to_str().unwrap().contains("poisoned"));
            assert!(record.getattr("object").unwrap().is(&owner));

            owner.borrow().native.clear_poison();
            owner.borrow().dispose().unwrap();
        });
    }

    #[test]
    fn dispose_preserves_already_dispatched_callback_waiting_to_enter_python() {
        Python::initialize();
        Python::attach(|py| {
            let table = dynwinrt::MetadataTable::new();
            let iid = windows::Foundation::IStringable::IID;
            let signature = dynwinrt::MethodSignature::new(&table).add_out(table.hstring());
            let typ = table
                .register_interface("Windows.Foundation.IStringable", iid)
                .add_method("ToString", signature.clone());
            let plan = WinRtImplementationPlan::new(
                vec![WinRtInterfaceDefinition {
                    name: "Windows.Foundation.IStringable".into(),
                    interface_type: typ.clone(),
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
            let globals = PyDict::new(py);
            globals
                .set_item(
                    "result",
                    DynWinRTValue(dynwinrt::WinRTValue::HString("finished".into())),
                )
                .unwrap();
            let function = py
                .eval(c"lambda *args: [result]", Some(&globals), None)
                .unwrap();
            let function_ref = py
                .import("weakref")
                .unwrap()
                .call_method1("ref", (&function,))
                .unwrap();
            let interpreter = Arc::new(InterpreterState::default());
            let cell = Arc::new(CallbackCell {
                callback: Mutex::new(Some(function.unbind())),
                interpreter: interpreter.clone(),
            });
            let (entered, receiver) = mpsc::channel();
            let proceed = Arc::new(Barrier::new(2));
            let callback_cell = cell.clone();
            let callback_proceed = proceed.clone();
            let native = WinRtImplementation::new(
                plan,
                Arc::new(move |interface, slot, args| {
                    entered.send(()).unwrap();
                    callback_proceed.wait();
                    callback_cell.invoke(interface, slot, args)
                }),
                None,
            )
            .unwrap();
            let owner = Bound::new(
                py,
                DynWinRTImplementation {
                    native: Mutex::new(Some(native)),
                    callback: Arc::downgrade(&cell),
                    interpreter,
                },
            )
            .unwrap();
            drop(cell);
            let view = owner
                .call_method0("to_value")
                .unwrap()
                .call_method1("cast", (WinGUID(iid),))
                .unwrap();
            let method = Bound::new(py, DynWinRTType(typ))
                .unwrap()
                .call_method1("method", (6,))
                .unwrap();
            let worker_owner = owner.clone().unbind();
            let worker = std::thread::spawn(move || {
                receiver.recv_timeout(Duration::from_secs(10)).unwrap();
                Python::attach(|py| {
                    worker_owner.bind(py).call_method0("dispose").unwrap();
                });
                proceed.wait();
            });

            let result = method.call_method1("invoke_detached", (&view, PyList::empty(py)));
            py.detach(|| worker.join()).unwrap();
            let result = result.unwrap();
            assert_eq!(
                result
                    .call_method0("to_string")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "finished"
            );
            assert!(owner.borrow().is_closed().unwrap());
            assert!(function_ref.call0().unwrap().is_none());
            let error = method
                .call_method1("invoke", (&view, PyList::empty(py)))
                .unwrap_err();
            assert_eq!(
                error
                    .value(py)
                    .getattr("winerror")
                    .unwrap()
                    .extract::<i32>()
                    .unwrap(),
                RO_E_CLOSED.0
            );
            view.call_method0("release").unwrap();
        });
    }
}
