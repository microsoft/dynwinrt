// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Bounded native one-shot completion. No language callbacks or operation
//! references are stored in the free-threaded signal sink.

use std::{
    ffi::{CString, c_void},
    marker::PhantomData,
    ptr,
    rc::Rc,
    sync::{Arc, Mutex},
    thread::{self, ThreadId},
};

use libffi::middle::{Arg, Cif, CodePtr, Type as FfiType};
use windows::Win32::{
    Foundation::{FreeLibrary, HMODULE},
    System::{
        Com::CoGetApartmentType,
        LibraryLoader::{
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
            GetModuleHandleExW, GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
        },
    },
};
use windows_core::{GUID, HRESULT, IUnknown, Interface as _, PCSTR, PCWSTR};

use super::{CallbackContract, DynamicComSink, InterfaceBase, SinkCallbackResult, Value};
use crate::{MetadataTable, WinRTValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartArgument {
    Utf16Path,
    RefIid,
    NullPropVariant,
    NativeSignal,
    OwnedOperation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultArgument {
    HResult,
    NullableOwnedInterface,
}

/// A completed COM-local contract, supplied only after metadata validation.
#[derive(Debug, Clone)]
pub struct OneShotContract {
    pub library: String,
    pub export: String,
    pub start_arguments: Vec<StartArgument>,
    pub handler_iid: GUID,
    pub handler_slot: usize,
    pub operation_iid: GUID,
    pub result_slot: usize,
    pub result_arguments: Vec<ResultArgument>,
    pub allowed_targets: Vec<GUID>,
}

impl OneShotContract {
    fn validate(&self) -> Result<(), CompletionError> {
        if self.library != "MMDevAPI.dll"
            || self.export != "ActivateAudioInterfaceAsync"
            || self.start_arguments
                != [
                    StartArgument::Utf16Path,
                    StartArgument::RefIid,
                    StartArgument::NullPropVariant,
                    StartArgument::NativeSignal,
                    StartArgument::OwnedOperation,
                ]
            || self.handler_iid != GUID::from_u128(0x41d949ab_9862_444a_80f6_c261334da5eb)
            || self.operation_iid != GUID::from_u128(0x72a22d78_cde4_431d_b8cc_843a71199b6d)
            || self.handler_slot != 3
            || self.result_slot != 3
            || self.result_arguments
                != [
                    ResultArgument::HResult,
                    ResultArgument::NullableOwnedInterface,
                ]
            || self.allowed_targets
                != [
                    GUID::from_u128(0x1cb9ad4c_dbfa_4c32_b178_c2f568a703b2),
                    GUID::from_u128(0x5cdf2c82_841e_4546_9722_0cf74078229a),
                ]
        {
            return Err(CompletionError::contract(
                "descriptor",
                "Unsupported or changed native one-shot contract; regenerate the COM bindings",
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct CompletionError {
    pub stage: &'static str,
    pub hresult: Option<HRESULT>,
    pub message: String,
}

impl CompletionError {
    pub fn contract(stage: &'static str, message: impl Into<String>) -> Self {
        Self {
            stage,
            hresult: None,
            message: message.into(),
        }
    }

    pub fn native(stage: &'static str, hresult: HRESULT) -> Self {
        Self {
            stage,
            hresult: Some(hresult),
            message: format!(
                "Native COM completion {stage} failed (HRESULT 0x{:08X})",
                hresult.0 as u32
            ),
        }
    }
}

impl std::fmt::Display for CompletionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.stage, self.message)
    }
}

impl std::error::Error for CompletionError {}

type Notify = Box<dyn FnOnce() + Send>;

struct SignalState {
    published: bool,
    completed: bool,
    closed: bool,
    notify: Option<Notify>,
}

/// Pointer-free, one-shot rendezvous between start publication and completion.
/// The notification must only enqueue a token; it must not call a language
/// runtime or retain an operation/result interface.
pub struct CompletionSignal(Mutex<SignalState>);

impl CompletionSignal {
    pub fn new(notify: impl FnOnce() + Send + 'static) -> Arc<Self> {
        Arc::new(Self(Mutex::new(SignalState {
            published: false,
            completed: false,
            closed: false,
            notify: Some(Box::new(notify)),
        })))
    }

    pub fn publish(&self) {
        self.advance(true);
    }

    fn complete(&self) {
        self.advance(false);
    }

    fn advance(&self, publish: bool) {
        let notify = {
            let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
            if state.closed {
                return;
            }
            if publish {
                state.published = true;
            } else {
                state.completed = true;
            }
            if state.published && state.completed {
                state.notify.take()
            } else {
                None
            }
        };
        if let Some(notify) = notify {
            notify();
        }
    }

    pub fn close(&self) {
        let notify = {
            let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
            state.closed = true;
            state.notify.take()
        };
        drop(notify);
    }
}

struct Module(HMODULE);

impl Drop for Module {
    fn drop(&mut self) {
        let _ = unsafe { FreeLibrary(self.0) };
    }
}

fn pin_code_module(address: *const c_void) -> Result<(), CompletionError> {
    let mut pinned = HMODULE::default();
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            PCWSTR(address.cast()),
            &mut pinned,
        )
    }
    .map_err(|error| CompletionError::native("load", error.code()))
}

/// Owner-thread-only even when Windows happens to return an agile operation.
///
/// ```compile_fail
/// fn require_send<T: Send>() {}
/// require_send::<dynwinrt::com::completion::OwnerOperation>();
/// ```
pub struct OwnerOperation {
    object: IUnknown,
    owner: ThreadId,
    _apartment: PhantomData<Rc<()>>,
}

pub struct NativeSignalSink {
    object: IUnknown,
    iid: GUID,
}

impl NativeSignalSink {
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn interface(&self) -> &IUnknown {
        &self.object
    }
}

/// Immutable libffi plans and module lifetime, deliberately not Send or Sync.
pub struct OneShotPlan {
    contract: OneShotContract,
    start_cif: Cif,
    result_cif: Cif,
    entry: CodePtr,
    _module: Option<Module>,
    owner: ThreadId,
    _apartment: PhantomData<Rc<()>>,
}

impl OneShotPlan {
    pub fn prepare(contract: OneShotContract) -> Result<Self, CompletionError> {
        contract.validate()?;
        let library = contract
            .library
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let module = Module(
            unsafe { LoadLibraryExW(PCWSTR(library.as_ptr()), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
                .map_err(|error| CompletionError::native("load", error.code()))?,
        );
        let export = CString::new(contract.export.as_bytes())
            .map_err(|_| CompletionError::contract("descriptor", "Embedded NUL in export"))?;
        let entry = unsafe { GetProcAddress(module.0, PCSTR(export.as_ptr().cast())) }
            .ok_or_else(|| CompletionError::native("load", HRESULT(0x8007007fu32 as i32)))?;
        let entry = CodePtr(entry as *const () as *mut c_void);
        // A late OS completion may outlive Node environment teardown. Pin only
        // this allowlisted system module, never an application-supplied DLL.
        pin_code_module(entry.0)?;
        Ok(Self::lower(contract, entry, Some(module)))
    }

    fn lower(contract: OneShotContract, entry: CodePtr, module: Option<Module>) -> Self {
        let start_cif = crate::native_call::system_cif(
            contract
                .start_arguments
                .iter()
                .map(|_| FfiType::pointer())
                .collect(),
            FfiType::i32(),
        );
        let result_cif = crate::native_call::system_cif(
            std::iter::once(FfiType::pointer())
                .chain(contract.result_arguments.iter().map(|_| FfiType::pointer()))
                .collect(),
            FfiType::i32(),
        );
        Self {
            contract,
            start_cif,
            result_cif,
            entry,
            _module: module,
            owner: thread::current().id(),
            _apartment: PhantomData,
        }
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub unsafe fn with_test_export(
        contract: OneShotContract,
        entry: *mut c_void,
    ) -> Result<Self, CompletionError> {
        contract.validate()?;
        if entry.is_null() {
            return Err(CompletionError::contract("test-export", "Null fake export"));
        }
        Ok(Self::lower(contract, CodePtr(entry), None))
    }

    fn ensure_owner(&self) -> Result<(), CompletionError> {
        if thread::current().id() != self.owner {
            return Err(CompletionError::native(
                "apartment",
                HRESULT(0x8001010eu32 as i32),
            ));
        }
        let mut apartment = Default::default();
        let mut qualifier = Default::default();
        unsafe { CoGetApartmentType(&mut apartment, &mut qualifier) }
            .map_err(|error| CompletionError::native("apartment", error.code()))
    }

    pub fn create_signal_sink(
        &self,
        signal: Arc<CompletionSignal>,
    ) -> Result<NativeSignalSink, CompletionError> {
        self.ensure_owner()?;
        // An OS-held callback may outlive the last Node environment that
        // loaded this addon. Its static vtable code must remain mapped too.
        pin_code_module(DynamicComSink::query_interface as *const () as *const c_void)?;
        let table = MetadataTable::new();
        let interface = super::register_interface(
            &table,
            "NativeOneShotSignal",
            self.contract.handler_iid,
            InterfaceBase::IUnknown,
        )
        .add_method_at(
            self.contract.handler_slot,
            "Signal",
            super::MethodSignature::new(&table).add_in(super::Type::winrt(table.object())),
        )
        .map_err(|error| CompletionError::contract("signal", error.message()))?;
        let iid = self.contract.handler_iid;
        let operation_iid = self.contract.operation_iid;
        let callback: super::SinkCallback = Arc::new(move |actual_iid, slot, args, contract| {
            if actual_iid != iid || slot != 3 || contract != CallbackContract::hresult(0) {
                return SinkCallbackResult::hresult(super::SINK_E_FAIL);
            }
            let [Value::WinRt(WinRTValue::Object(operation))] = args else {
                return SinkCallbackResult::hresult(super::SINK_E_POINTER);
            };
            // QI and its temporary +1 stay on the callback apartment. Only the
            // signal is retained; GetActivateResult never runs here.
            let mut borrowed = OwnedOutput::default();
            let hr = unsafe { operation.query(&operation_iid, &mut borrowed.0) };
            if hr.is_err() {
                return SinkCallbackResult::hresult(hr);
            }
            if borrowed.0.is_null() {
                return SinkCallbackResult::hresult(super::SINK_E_POINTER);
            }
            drop(borrowed);
            signal.complete();
            SinkCallbackResult::hresult(HRESULT(0))
        });
        let definition = interface
            .callback_backends()
            .map_err(|error| CompletionError::contract("signal", error.message()))?;
        let identity = DynamicComSink::create_impl(vec![definition], callback, true)
            .map_err(|error| CompletionError::contract("signal", error.message()))?;
        let mut view = OwnedOutput::default();
        let hr = unsafe { identity.query(&iid, &mut view.0) };
        if hr.is_err() {
            return Err(CompletionError::native("signal", hr));
        }
        let object = view.take().ok_or_else(|| {
            CompletionError::contract("signal", "Native signal interface was not published")
        })?;
        Ok(NativeSignalSink { object, iid })
    }

    pub fn start(
        &self,
        path: &str,
        target_iid: GUID,
        sink: &NativeSignalSink,
    ) -> Result<OwnerOperation, CompletionError> {
        self.ensure_owner()?;
        if path.contains('\0') {
            return Err(CompletionError::contract(
                "input",
                "Device interface path contains NUL",
            ));
        }
        if !self.contract.allowed_targets.contains(&target_iid)
            || sink.iid != self.contract.handler_iid
        {
            return Err(CompletionError::contract(
                "input",
                "Unsupported native completion target or signal",
            ));
        }
        let path = path.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let mut output = OwnedOutput::default();
        let mut storage = Vec::<*mut c_void>::with_capacity(self.contract.start_arguments.len());
        for argument in &self.contract.start_arguments {
            storage.push(match argument {
                StartArgument::Utf16Path => path.as_ptr().cast_mut().cast(),
                StartArgument::RefIid => (&target_iid as *const GUID).cast_mut().cast(),
                StartArgument::NullPropVariant => ptr::null_mut(),
                StartArgument::NativeSignal => sink.object.as_raw(),
                StartArgument::OwnedOperation => (&mut output.0 as *mut *mut c_void).cast(),
            });
        }
        let args = storage.iter().map(Arg::new).collect::<Vec<_>>();
        let hr = HRESULT(unsafe { self.start_cif.call::<i32>(self.entry, &args) });
        if hr.is_err() {
            return Err(CompletionError::native("start", hr));
        }
        let object = output.take().ok_or_else(|| {
            CompletionError::contract("start", "Start succeeded with a NULL operation")
        })?;
        Ok(OwnerOperation {
            object,
            owner: self.owner,
            _apartment: PhantomData,
        })
    }

    pub fn collect(
        &self,
        operation: &OwnerOperation,
        target_iid: GUID,
    ) -> Result<IUnknown, CompletionError> {
        self.ensure_owner()?;
        if operation.owner != self.owner || !self.contract.allowed_targets.contains(&target_iid) {
            return Err(CompletionError::contract(
                "result",
                "Owner or target mismatch",
            ));
        }
        let this = operation.object.as_raw();
        let vtable = unsafe { *this.cast::<*const *mut c_void>() };
        let entry = unsafe { *vtable.add(self.contract.result_slot) };
        let mut inner = super::SINK_E_FAIL.0;
        let mut output = OwnedOutput::default();
        let mut storage = vec![this];
        for argument in &self.contract.result_arguments {
            storage.push(match argument {
                ResultArgument::HResult => (&mut inner as *mut i32).cast(),
                ResultArgument::NullableOwnedInterface => {
                    (&mut output.0 as *mut *mut c_void).cast()
                }
            });
        }
        let args = storage.iter().map(Arg::new).collect::<Vec<_>>();
        let outer = HRESULT(unsafe { self.result_cif.call::<i32>(CodePtr(entry), &args) });
        if outer.is_err() {
            return Err(CompletionError::native("result", outer));
        }
        if HRESULT(inner).is_err() {
            return Err(CompletionError::native("activation", HRESULT(inner)));
        }
        let result = output.take().ok_or_else(|| {
            CompletionError::contract("result", "Both HRESULTs succeeded with a NULL interface")
        })?;
        let mut projected = OwnedOutput::default();
        let hr = unsafe { result.query(&target_iid, &mut projected.0) };
        if hr.is_err() {
            return Err(CompletionError::native("projection", hr));
        }
        projected.take().ok_or_else(|| {
            CompletionError::contract("projection", "QueryInterface succeeded with NULL")
        })
    }
}

#[derive(Default)]
struct OwnedOutput(*mut c_void);

impl OwnedOutput {
    fn take(&mut self) -> Option<IUnknown> {
        let pointer = std::mem::replace(&mut self.0, ptr::null_mut());
        (!pointer.is_null()).then(|| unsafe { IUnknown::from_raw(pointer) })
    }
}

impl Drop for OwnedOutput {
    fn drop(&mut self) {
        drop(self.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn contract() -> OneShotContract {
        OneShotContract {
            library: "MMDevAPI.dll".into(),
            export: "ActivateAudioInterfaceAsync".into(),
            start_arguments: vec![
                StartArgument::Utf16Path,
                StartArgument::RefIid,
                StartArgument::NullPropVariant,
                StartArgument::NativeSignal,
                StartArgument::OwnedOperation,
            ],
            handler_iid: GUID::from_u128(0x41d949ab_9862_444a_80f6_c261334da5eb),
            handler_slot: 3,
            operation_iid: GUID::from_u128(0x72a22d78_cde4_431d_b8cc_843a71199b6d),
            result_slot: 3,
            result_arguments: vec![
                ResultArgument::HResult,
                ResultArgument::NullableOwnedInterface,
            ],
            allowed_targets: vec![
                GUID::from_u128(0x1cb9ad4c_dbfa_4c32_b178_c2f568a703b2),
                GUID::from_u128(0x5cdf2c82_841e_4546_9722_0cf74078229a),
            ],
        }
    }

    #[test]
    fn native_completion_loads_only_the_system_export_and_has_real_ftm_identity() {
        thread::spawn(|| {
            super::super::initialize_apartment(super::super::ApartmentType::MultiThreaded).unwrap();
            let plan = OneShotPlan::prepare(contract()).unwrap();
            assert!(!plan.start_cif.as_raw_ptr().is_null());
            let signal =
                CompletionSignal::new(|| panic!("no native activation is started by this test"));
            let sink = plan.create_signal_sink(signal.clone()).unwrap();
            let query = |object: &IUnknown, iid: &GUID| {
                let mut result = OwnedOutput::default();
                unsafe {
                    object.query(iid, &mut result.0).ok().unwrap();
                }
                result.take().unwrap()
            };
            let identity = query(sink.interface(), &IUnknown::IID);
            for iid in [
                plan.contract.handler_iid,
                windows_core::imp::IAgileObject::IID,
                windows_core::imp::IMarshal::IID,
            ] {
                let view = query(sink.interface(), &iid);
                let canonical = query(&view, &IUnknown::IID);
                assert_eq!(canonical.as_raw(), identity.as_raw());
            }
            drop(identity);
            drop(sink);
            assert_eq!(Arc::strong_count(&signal), 1);
            signal.close();
            let mut invalid = contract();
            invalid.start_arguments.push(StartArgument::NativeSignal);
            assert!(OneShotPlan::prepare(invalid).is_err());
            let mut invalid = contract();
            invalid.library = "C:\\untrusted\\MMDevAPI.dll".into();
            assert!(OneShotPlan::prepare(invalid).is_err());
        })
        .join()
        .unwrap();
    }

    #[test]
    fn native_completion_signal_races_publish_without_duplicates() {
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        let signal = CompletionSignal::new(move || {
            called.fetch_add(1, Ordering::SeqCst);
        });
        let mut threads = Vec::new();
        for _ in 0..16 {
            let signal = signal.clone();
            threads.push(thread::spawn(move || signal.complete()));
        }
        signal.publish();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn signal_publication_and_completion_are_once_and_close_is_not_cancellation() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CompletionSignal>();
        for early in [true, false] {
            let calls = Arc::new(AtomicUsize::new(0));
            let called = calls.clone();
            let signal = CompletionSignal::new(move || {
                called.fetch_add(1, Ordering::SeqCst);
            });
            if early {
                signal.complete();
                signal.complete();
                assert_eq!(calls.load(Ordering::SeqCst), 0);
                signal.publish();
            } else {
                signal.publish();
                signal.complete();
            }
            signal.publish();
            signal.complete();
            signal.close();
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
        let signal = CompletionSignal::new(|| panic!("closed signal must not dispatch"));
        signal.complete();
        signal.close();
        signal.publish();
        signal.complete();
    }
}
