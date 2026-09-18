// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Standalone, metadata-planned WinRT interface implementations.
//!
//! These objects are non-agile. Only IUnknown/IInspectable identity operations
//! are available independently of the creating thread. Method callbacks are
//! synchronous and must return every declared output before ownership is
//! published to the native caller.

use core::ffi::c_void;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, ThreadId},
};

use windows::Win32::System::Com::CoTaskMemAlloc;
use windows::Win32::System::WinRT::IWeakReferenceSource;
use windows_core::{GUID, HRESULT, HSTRING, IInspectable, IUnknown, Interface};

use crate::{
    WinRTValue,
    com_helpers::{E_FAIL, E_NOINTERFACE, E_POINTER, IInspectableVtbl, S_OK},
    native_callback::{CallbackAbiType, CallbackSignature},
};

mod identity;
mod plan;
mod values;

pub use plan::{
    WinRtImplementationPlan, WinRtInterfaceDefinition, WinRtMethodDefinition, WinRtThreadingPolicy,
};

const E_INVALIDARG: HRESULT = HRESULT(0x80070057u32 as i32);
const E_OUTOFMEMORY: HRESULT = HRESULT(0x8007000Eu32 as i32);
const RPC_E_WRONG_THREAD: HRESULT = HRESULT(0x8001010Eu32 as i32);
const RO_E_CLOSED: HRESULT = HRESULT(0x80000013u32 as i32);

fn error(code: HRESULT, message: impl AsRef<str>) -> windows_core::Error {
    windows_core::Error::new(code, message.as_ref())
}

fn invalid_argument(message: impl AsRef<str>) -> windows_core::Error {
    error(E_INVALIDARG, message)
}

fn reserved_iid(iid: &GUID) -> bool {
    *iid == GUID::zeroed()
        || *iid == IUnknown::IID
        || *iid == IInspectable::IID
        || *iid == windows_core::imp::IAgileObject::IID
        || *iid == windows_core::imp::IMarshal::IID
        || *iid == windows_core::imp::IWeakReferenceSource::IID
        || *iid == windows_core::imp::IWeakReference::IID
}

/// Returns outputs in signature order (including the projected return value).
/// Incoming references have already been retained for the callback.
pub type WinRtImplementationCallback =
    Arc<dyn Fn(usize, usize, &[WinRTValue]) -> windows_core::Result<Vec<WinRTValue>> + Send + Sync>;

struct Control {
    owner_thread: ThreadId,
    closed: AtomicBool,
    callback: Mutex<Option<WinRtImplementationCallback>>,
    last_error: Mutex<Option<String>>,
}

impl Control {
    fn callback(&self) -> windows_core::Result<WinRtImplementationCallback> {
        if thread::current().id() != self.owner_thread {
            return Err(error(
                RPC_E_WRONG_THREAD,
                "WinRT implementation callbacks require the creating thread; no cross-apartment dispatcher is installed",
            ));
        }
        self.callback
            .lock()
            .map_err(|_| error(E_FAIL, "WinRT implementation callback state is poisoned"))?
            .clone()
            .ok_or_else(|| {
                error(
                    RO_E_CLOSED,
                    "WinRT implementation callbacks are disconnected",
                )
            })
    }

    fn disconnect(&self) -> windows_core::Result<()> {
        let callback = {
            let mut slot = self
                .callback
                .lock()
                .map_err(|_| error(E_FAIL, "WinRT implementation callback state is poisoned"))?;
            self.closed.store(true, Ordering::Release);
            slot.take()
        };
        // A handler/finalizer may reenter this object when its last root drops.
        drop(callback);
        Ok(())
    }

    fn record_error(&self, message: String) {
        match self.last_error.lock() {
            Ok(mut slot) => *slot = Some(message),
            Err(_) => eprintln!("[dynwinrt] WinRT callback error state is poisoned: {message}"),
        }
    }
}

/// Owns one native reference to an implemented WinRT object.
///
/// Dropping or releasing this owner does not disconnect other native owners.
/// `dispose` deliberately disconnects the whole object and breaks callback
/// cycles; an already-dispatched callback retains its resources until return.
pub struct WinRtImplementation {
    identity: Option<IUnknown>,
    control: Arc<Control>,
}

// The only IUnknown stored here is our canonical, atomically owned Host, never
// an arbitrary apartment-bound object. Control state and callback destruction
// are thread-safe. Moving/releasing a controller does not grant callback
// agility: to_value and native dispatch still enforce the owner thread.
unsafe impl Send for WinRtImplementation {}
unsafe impl Sync for WinRtImplementation {}

impl WinRtImplementation {
    pub fn new(
        plan: WinRtImplementationPlan,
        callback: WinRtImplementationCallback,
        runtime_class_name: Option<&str>,
    ) -> windows_core::Result<Self> {
        if runtime_class_name.is_some_and(|name| name.contains('\0')) {
            return Err(invalid_argument(
                "WinRT runtime class names cannot contain NUL",
            ));
        }
        let control = Arc::new(Control {
            owner_thread: thread::current().id(),
            closed: AtomicBool::new(false),
            callback: Mutex::new(Some(callback)),
            last_error: Mutex::new(None),
        });
        let identity = Host::create(
            plan,
            control.clone(),
            HSTRING::from(runtime_class_name.unwrap_or("")),
        )?;
        Ok(Self {
            identity: Some(identity),
            control,
        })
    }

    pub fn to_value(&self) -> windows_core::Result<WinRTValue> {
        if thread::current().id() != self.control.owner_thread {
            return Err(error(
                RPC_E_WRONG_THREAD,
                "WinRT implementation owner belongs to another thread",
            ));
        }
        self.identity
            .as_ref()
            .map(|identity| WinRTValue::Object(identity.clone()))
            .ok_or_else(|| error(RO_E_CLOSED, "WinRT implementation owner has been released"))
    }

    pub fn release(&mut self) {
        self.identity = None;
    }

    pub fn disconnect(&self) -> windows_core::Result<()> {
        self.control.disconnect()
    }

    pub fn dispose(&mut self) -> windows_core::Result<()> {
        self.disconnect()?;
        self.release();
        Ok(())
    }

    pub fn is_closed(&self) -> bool {
        self.control.closed.load(Ordering::Acquire)
    }

    pub fn take_error(&self) -> Option<String> {
        match self.control.last_error.lock() {
            Ok(mut slot) => slot.take(),
            Err(_) => Some("WinRT callback error state is poisoned".into()),
        }
    }

    /// Conservative GC hint: false when any other native owner may exist.
    /// Native weak-reference machinery may also make this return false.
    pub fn has_unique_native_reference(&self) -> bool {
        self.identity.as_ref().is_some_and(|identity| {
            // This controller always owns the canonical Host identity.
            let host = unsafe { identity::borrowed::<Host>(identity.as_raw()) };
            !host.weak_requested.load(Ordering::Acquire) && Arc::strong_count(&host) == 1
        })
    }
}

#[repr(C)]
struct View {
    vtable: *const *const c_void,
    owner: *mut Host,
    index: usize,
    _storage: Box<[*const c_void]>,
}

#[repr(C)]
struct Host {
    vtable: *const IInspectableVtbl,
    control: Arc<Control>,
    plan: WinRtImplementationPlan,
    name: HSTRING,
    iids: Box<[GUID]>,
    // The individually boxed views and vtables never move after publication.
    #[allow(clippy::vec_box)]
    views: Vec<Box<View>>,
    weak_source: Box<identity::WeakSource>,
    weak_requested: AtomicBool,
}

// All native storage is immutable after publication, refcounts are atomic, and
// callback/control state is synchronized. This does NOT grant WinRT agility:
// dispatch rejects foreign threads before accessing language resources.
unsafe impl Send for Host {}
unsafe impl Sync for Host {}

impl Drop for Host {
    fn drop(&mut self) {
        match catch_unwind(AssertUnwindSafe(|| self.control.disconnect())) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => eprintln!("[dynwinrt] WinRT implementation destruction: {error}"),
            Err(_) => eprintln!("[dynwinrt] Panic while destroying WinRT implementation callbacks"),
        }
    }
}

impl Host {
    const VTABLE: IInspectableVtbl = IInspectableVtbl {
        base: windows_core::IUnknown_Vtbl {
            QueryInterface: Self::query_interface,
            AddRef: Self::add_ref,
            Release: Self::release,
        },
        get_iids: Self::get_iids,
        get_runtime_class_name: Self::get_runtime_class_name,
        get_trust_level: Self::get_trust_level,
    };

    fn create(
        plan: WinRtImplementationPlan,
        control: Arc<Control>,
        name: HSTRING,
    ) -> windows_core::Result<IUnknown> {
        match plan.threading {
            WinRtThreadingPolicy::OwnerThread => {}
        }
        let mut views = Vec::with_capacity(plan.interfaces.len());
        for (index, interface) in plan.interfaces.iter().enumerate() {
            let mut slots = vec![
                Self::view_query_interface as *const () as *const c_void,
                Self::view_add_ref as *const () as *const c_void,
                Self::view_release as *const () as *const c_void,
                Self::view_get_iids as *const () as *const c_void,
                Self::view_get_runtime_class_name as *const () as *const c_void,
                Self::view_get_trust_level as *const () as *const c_void,
            ];
            for method in &interface.methods {
                let code = match Self::static_code(method.slot, &method.signature) {
                    Some(code) => code,
                    None => crate::native_callback::callback_code(
                        method.slot,
                        method.signature.clone(),
                        Self::dispatch_libffi,
                    )
                    .map_err(|message| {
                        error(
                            E_FAIL,
                            format!(
                                "{}::{} (slot {}): cannot prepare WinRT callback: {message}",
                                interface.name, method.name, method.slot
                            ),
                        )
                    })?,
                };
                slots.push(code);
            }
            let storage = slots.into_boxed_slice();
            views.push(Box::new(View {
                vtable: storage.as_ptr(),
                owner: ptr::null_mut(),
                index,
                _storage: storage,
            }));
        }
        let iids = plan
            .interfaces
            .iter()
            .map(|interface| interface.iid)
            .collect();
        let mut host = Arc::new(Self {
            vtable: &Self::VTABLE,
            control,
            plan,
            name,
            iids,
            views,
            weak_source: Box::new(identity::WeakSource::new()),
            weak_requested: AtomicBool::new(false),
        });
        let owner = Arc::as_ptr(&host).cast_mut();
        let unpublished =
            Arc::get_mut(&mut host).expect("unpublished WinRT host is uniquely owned");
        for view in &mut unpublished.views {
            view.owner = owner;
        }
        unpublished.weak_source.owner = owner;
        Ok(unsafe { IUnknown::from_raw(Arc::into_raw(host).cast_mut().cast()) })
    }

    unsafe extern "system" fn query_interface(
        this: *mut c_void,
        iid: *const GUID,
        result: *mut *mut c_void,
    ) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        unsafe { result.write(ptr::null_mut()) };
        if iid.is_null() {
            return E_POINTER;
        }
        let host = unsafe { &*this.cast::<Self>() };
        let iid = unsafe { &*iid };
        let view = if *iid == IUnknown::IID || *iid == IInspectable::IID {
            this
        } else if let Some(index) = host.iids.iter().position(|candidate| candidate == iid) {
            ptr::from_ref(host.views[index].as_ref()).cast_mut().cast()
        } else if *iid == IWeakReferenceSource::IID {
            host.weak_requested.store(true, Ordering::Release);
            ptr::from_ref(host.weak_source.as_ref()).cast_mut().cast()
        } else {
            return E_NOINTERFACE;
        };
        unsafe { identity::add_ref::<Self>(this) };
        unsafe { result.write(view) };
        S_OK
    }

    unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
        unsafe { identity::add_ref::<Self>(this) }
    }

    unsafe extern "system" fn release(this: *mut c_void) -> u32 {
        unsafe { identity::release::<Self>(this) }
    }

    unsafe extern "system" fn get_iids(
        this: *mut c_void,
        count: *mut u32,
        result: *mut *mut GUID,
    ) -> HRESULT {
        if !count.is_null() {
            unsafe { count.write(0) };
        }
        if !result.is_null() {
            unsafe { result.write(ptr::null_mut()) };
        }
        if count.is_null() || result.is_null() {
            return E_POINTER;
        }
        let host = unsafe { &*this.cast::<Self>() };
        let Ok(length) = u32::try_from(host.iids.len()) else {
            return E_OUTOFMEMORY;
        };
        let Some(bytes) = host.iids.len().checked_mul(size_of::<GUID>()) else {
            return E_OUTOFMEMORY;
        };
        let allocation = unsafe { CoTaskMemAlloc(bytes).cast::<GUID>() };
        if allocation.is_null() {
            return E_OUTOFMEMORY;
        }
        unsafe {
            ptr::copy_nonoverlapping(host.iids.as_ptr(), allocation, host.iids.len());
            count.write(length);
            result.write(allocation);
        }
        S_OK
    }

    unsafe extern "system" fn get_runtime_class_name(
        this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        unsafe { result.write(ptr::null_mut()) };
        match catch_unwind(AssertUnwindSafe(|| {
            let host = unsafe { &*this.cast::<Self>() };
            unsafe { result.cast::<HSTRING>().write(host.name.clone()) };
        })) {
            Ok(()) => S_OK,
            Err(_) => E_FAIL,
        }
    }

    unsafe extern "system" fn get_trust_level(_this: *mut c_void, result: *mut i32) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        unsafe { result.write(0) };
        S_OK
    }

    unsafe fn owner(this: *mut c_void) -> *mut c_void {
        unsafe { (*this.cast::<View>()).owner.cast() }
    }

    unsafe extern "system" fn view_query_interface(
        this: *mut c_void,
        iid: *const GUID,
        result: *mut *mut c_void,
    ) -> HRESULT {
        unsafe { Self::query_interface(Self::owner(this), iid, result) }
    }

    unsafe extern "system" fn view_add_ref(this: *mut c_void) -> u32 {
        unsafe { Self::add_ref(Self::owner(this)) }
    }

    unsafe extern "system" fn view_release(this: *mut c_void) -> u32 {
        unsafe { Self::release(Self::owner(this)) }
    }

    unsafe extern "system" fn view_get_iids(
        this: *mut c_void,
        count: *mut u32,
        result: *mut *mut GUID,
    ) -> HRESULT {
        unsafe { Self::get_iids(Self::owner(this), count, result) }
    }

    unsafe extern "system" fn view_get_runtime_class_name(
        this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        unsafe { Self::get_runtime_class_name(Self::owner(this), result) }
    }

    unsafe extern "system" fn view_get_trust_level(this: *mut c_void, result: *mut i32) -> HRESULT {
        unsafe { Self::get_trust_level(Self::owner(this), result) }
    }

    unsafe fn dispatch(slot: usize, args: *const *const c_void) -> HRESULT {
        if args.is_null() || unsafe { (*args).is_null() } {
            return E_POINTER;
        }
        let this = unsafe { (*args).cast::<*mut c_void>().read_unaligned() };
        if this.is_null() {
            return E_POINTER;
        }
        let (control, context, invocation) = {
            let view = unsafe { &*this.cast::<View>() };
            let host = unsafe { identity::pin::<Self>(view.owner.cast()) };
            let control = host.control.clone();
            let Some(interface) = host.plan.interfaces.get(view.index) else {
                return E_INVALIDARG;
            };
            let Some(method) = slot
                .checked_sub(6)
                .and_then(|index| interface.methods.get(index))
            else {
                return E_INVALIDARG;
            };
            let context = format!("{}::{} (slot {slot})", interface.name, method.name);
            let invocation = catch_unwind(AssertUnwindSafe(|| -> windows_core::Result<()> {
                // Failure outputs are initialized even for disconnected and
                // wrong-thread calls, before any language/runtime entry.
                let outputs = unsafe { method.initialize_outputs(args)? };
                let callback = control.callback()?;
                let inputs = unsafe { method.inputs(args)? };
                let values = callback(view.index, slot, &inputs)?;
                let prepared = method.prepare_outputs(outputs, &values)?;
                // Every fallible conversion and reference acquisition is done.
                for output in prepared {
                    unsafe { output.commit() };
                }
                Ok(())
            }))
            .unwrap_or_else(|_| Err(error(E_FAIL, "Rust panic in WinRT implementation callback")));
            (control, context, invocation)
        };
        match invocation {
            Ok(()) => S_OK,
            Err(cause) => {
                control.record_error(format!(
                    "{context}: 0x{:08X}: {}",
                    cause.code().0 as u32,
                    cause.message()
                ));
                cause.into()
            }
        }
    }

    unsafe fn dispatch_libffi(
        slot: usize,
        _signature: &CallbackSignature,
        args: *const *const c_void,
        result: *mut c_void,
    ) {
        if !result.is_null() {
            let hr = unsafe { Self::dispatch(slot, args) };
            unsafe { result.cast::<i32>().write(hr.0) };
        }
    }

    unsafe extern "system" fn no_args<const SLOT: usize>(this: *mut c_void) -> HRESULT {
        let args = [ptr::from_ref(&this).cast()];
        unsafe { Self::dispatch(SLOT, args.as_ptr()) }
    }

    unsafe extern "system" fn one_pointer<const SLOT: usize>(
        this: *mut c_void,
        value: *mut c_void,
    ) -> HRESULT {
        let args = [ptr::from_ref(&this).cast(), ptr::from_ref(&value).cast()];
        unsafe { Self::dispatch(SLOT, args.as_ptr()) }
    }

    unsafe extern "system" fn two_pointers<const SLOT: usize>(
        this: *mut c_void,
        first: *mut c_void,
        second: *mut c_void,
    ) -> HRESULT {
        let args = [
            ptr::from_ref(&this).cast(),
            ptr::from_ref(&first).cast(),
            ptr::from_ref(&second).cast(),
        ];
        unsafe { Self::dispatch(SLOT, args.as_ptr()) }
    }

    fn static_slot<const SLOT: usize>(signature: &CallbackSignature) -> Option<*const c_void> {
        match signature.parameters() {
            [] => Some(Self::no_args::<SLOT> as *const () as *const c_void),
            [CallbackAbiType::Pointer] => {
                Some(Self::one_pointer::<SLOT> as *const () as *const c_void)
            }
            [CallbackAbiType::Pointer, CallbackAbiType::Pointer] => {
                Some(Self::two_pointers::<SLOT> as *const () as *const c_void)
            }
            _ => None,
        }
    }

    fn static_code(slot: usize, signature: &CallbackSignature) -> Option<*const c_void> {
        macro_rules! slots {
            ($($slot:literal),+ $(,)?) => {
                match slot {
                    $($slot => Self::static_slot::<$slot>(signature),)+
                    _ => None,
                }
            };
        }
        slots!(6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21)
    }
}

#[cfg(test)]
mod tests;
