// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#![allow(unsafe_op_in_unsafe_fn)]

//! Reusable WinRT outer/inner composition support.
//!
//! A composed outer owns the factory's non-delegating `inner` pointer. Public
//! interfaces are obtained by forwarding QI to that inner, but every forwarded
//! interface must return the outer from QI(IUnknown). This preserves one COM
//! identity and fails closed if a factory does not honor WinRT aggregation.

use core::ffi::c_void;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use windows::Win32::System::Com::{CoTaskMemAlloc, CoTaskMemFree};
use windows_core::{GUID, HRESULT, IInspectable, IUnknown, Interface};

use crate::com_helpers::{E_FAIL, E_NOINTERFACE, E_POINTER, IInspectableVtbl, S_OK};
use crate::{MethodHandle, WinRTValue};

const E_OUTOFMEMORY: HRESULT = HRESULT(0x8007000Eu32 as i32);
const E_INVALIDARG: HRESULT = HRESULT(0x80070057u32 as i32);
const E_UNEXPECTED: HRESULT = HRESULT(0x8000FFFFu32 as i32);

/// ABI shapes currently understood by the dynamic local-interface host.
///
/// A local interface is only constructed when every slot has an exact shape.
/// Python callbacks are currently accepted for `Void0` and
/// `SizeF32ToSizeF32`; other or unimplemented slots forward to the inner object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalOverrideAbi {
    Void0,
    SizeF32ToSizeF32,
    HStringBoolToBool,
}

pub type LocalOverrideVoidCallback = Arc<dyn Fn() -> HRESULT + Send + Sync>;
pub type LocalOverrideSizeCallback =
    Arc<dyn Fn(f32, f32, &mut f32, &mut f32) -> HRESULT + Send + Sync>;

enum LocalOverrideCallback {
    Void(LocalOverrideVoidCallback),
    Size(LocalOverrideSizeCallback),
}

/// Metadata-derived description of one overridable local WinRT interface.
pub struct LocalOverrideInterface {
    iid: GUID,
    methods: Vec<LocalOverrideAbi>,
    callbacks: HashMap<usize, LocalOverrideCallback>,
}

impl LocalOverrideInterface {
    pub fn new(iid: GUID, methods: Vec<LocalOverrideAbi>) -> windows_core::Result<Self> {
        if methods.is_empty() {
            return Err(windows_core::Error::from_hresult(E_INVALIDARG));
        }
        Ok(Self {
            iid,
            methods,
            callbacks: HashMap::new(),
        })
    }

    /// Install a callback by absolute vtable index.
    pub fn with_void_callback(
        mut self,
        vtable_index: usize,
        callback: LocalOverrideVoidCallback,
    ) -> windows_core::Result<Self> {
        let Some(method_index) = vtable_index.checked_sub(6) else {
            return Err(windows_core::Error::from_hresult(E_INVALIDARG));
        };
        if self.methods.get(method_index) != Some(&LocalOverrideAbi::Void0)
            || self
                .callbacks
                .insert(method_index, LocalOverrideCallback::Void(callback))
                .is_some()
        {
            return Err(windows_core::Error::from_hresult(E_INVALIDARG));
        }
        Ok(self)
    }

    pub fn with_size_callback(
        mut self,
        vtable_index: usize,
        callback: LocalOverrideSizeCallback,
    ) -> windows_core::Result<Self> {
        let Some(method_index) = vtable_index.checked_sub(6) else {
            return Err(windows_core::Error::from_hresult(E_INVALIDARG));
        };
        if self.methods.get(method_index) != Some(&LocalOverrideAbi::SizeF32ToSizeF32)
            || self
                .callbacks
                .insert(method_index, LocalOverrideCallback::Size(callback))
                .is_some()
        {
            return Err(windows_core::Error::from_hresult(E_INVALIDARG));
        }
        Ok(self)
    }
}

/// Shared identity, refcount, and owned-inner state for composed outer objects.
///
/// `WeakRefCount` is atomic. The inner pointer is protected because QI and
/// destruction can occur on COM-managed threads. Implementors embedding this
/// state may only claim `Send`/`Sync` when every additional local-interface
/// field and callback is independently thread-safe; the state does not make an
/// apartment-bound WinRT object agile.
pub(crate) struct CompositionState {
    ref_count: windows_core::imp::WeakRefCount,
    inner: Mutex<Option<IUnknown>>,
    agile: bool,
}

impl CompositionState {
    pub(crate) fn new(agile: bool) -> Self {
        Self {
            ref_count: windows_core::imp::WeakRefCount::new(),
            inner: Mutex::new(None),
            agile,
        }
    }

    pub(crate) fn add_ref(&self) -> u32 {
        self.ref_count.add_ref()
    }

    pub(crate) fn release(&self) -> u32 {
        self.ref_count.release()
    }

    pub(crate) fn set_inner(&self, inner: IUnknown) -> windows_core::Result<()> {
        let mut slot = self
            .inner
            .lock()
            .map_err(|_| windows_core::Error::from_hresult(E_FAIL))?;
        if slot.is_some() {
            return Err(windows_core::Error::from_hresult(E_UNEXPECTED));
        }
        *slot = Some(inner);
        Ok(())
    }

    fn inner(&self) -> std::result::Result<Option<IUnknown>, HRESULT> {
        self.inner
            .lock()
            .map(|inner| inner.clone())
            .map_err(|_| E_FAIL)
    }

    unsafe fn inner_interface(&self, iid: &GUID) -> std::result::Result<IUnknown, HRESULT> {
        let Some(inner) = self.inner()? else {
            return Err(E_NOINTERFACE);
        };
        let mut result = std::ptr::null_mut();
        let hr = inner.query(iid, &mut result);
        if hr.is_err() {
            if !result.is_null() {
                drop(IUnknown::from_raw(result));
            }
            return Err(hr);
        }
        if result.is_null() {
            return Err(E_NOINTERFACE);
        }
        Ok(IUnknown::from_raw(result))
    }

    /// Resolve local interfaces before forwarding to the aggregated inner.
    pub(crate) unsafe fn query_interface(
        &self,
        identity: *mut c_void,
        iid: *const GUID,
        result: *mut *mut c_void,
        local: impl FnOnce(&GUID) -> Option<*mut c_void>,
    ) -> HRESULT {
        if iid.is_null() || result.is_null() {
            return E_POINTER;
        }
        *result = std::ptr::null_mut();
        let iid = &*iid;

        if *iid == IUnknown::IID || *iid == IInspectable::IID {
            *result = identity;
            self.add_ref();
            return S_OK;
        }
        if self.agile && *iid == windows_core::imp::IAgileObject::IID {
            *result = identity;
            self.add_ref();
            return S_OK;
        }
        if self.agile && *iid == windows_core::imp::IMarshal::IID {
            self.add_ref();
            return windows_core::imp::marshaler(core::mem::transmute(identity), result);
        }
        if let Some(local) = local(iid) {
            *result = local;
            self.add_ref();
            return S_OK;
        }
        let weak = self.ref_count.query(iid, identity);
        if !weak.is_null() {
            *result = weak;
            return S_OK;
        }

        let Some(inner) = (match self.inner() {
            Ok(inner) => inner,
            Err(error) => return error,
        }) else {
            return E_NOINTERFACE;
        };
        let mut forwarded = std::ptr::null_mut();
        let hr = inner.query(iid, &mut forwarded);
        if hr.is_err() {
            if !forwarded.is_null() {
                drop(IUnknown::from_raw(forwarded));
            }
            return hr;
        }
        if forwarded.is_null() {
            return E_NOINTERFACE;
        }

        // Aggregated public interfaces must delegate IUnknown to this outer.
        let forwarded_owner = IUnknown::from_raw(forwarded);
        let mut controlling = std::ptr::null_mut();
        let identity_hr = forwarded_owner.query(&IUnknown::IID, &mut controlling);
        if identity_hr.is_err() || controlling != identity {
            if !controlling.is_null() {
                drop(IUnknown::from_raw(controlling));
            }
            return E_NOINTERFACE;
        }
        drop(IUnknown::from_raw(controlling));
        *result = forwarded_owner.as_raw();
        std::mem::forget(forwarded_owner);
        S_OK
    }

    pub(crate) unsafe fn get_iids(
        &self,
        local_iids: &[GUID],
        count: *mut u32,
        result: *mut *mut GUID,
    ) -> HRESULT {
        if count.is_null() || result.is_null() {
            return E_POINTER;
        }
        *count = 0;
        *result = std::ptr::null_mut();

        let mut inner_count = 0;
        let mut inner_iids = std::ptr::null_mut();
        match self.inner() {
            Ok(Some(inner)) => {
                let vtable = *(inner.as_raw() as *const *const IInspectableVtbl);
                let hr = ((*vtable).get_iids)(inner.as_raw(), &mut inner_count, &mut inner_iids);
                if hr.is_err() {
                    if !inner_iids.is_null() {
                        CoTaskMemFree(Some(inner_iids.cast()));
                    }
                    return hr;
                }
                if inner_count != 0 && inner_iids.is_null() {
                    return E_FAIL;
                }
            }
            Ok(None) => {}
            Err(error) => return error,
        }

        let total = match local_iids.len().checked_add(inner_count as usize) {
            Some(total) => total,
            None => {
                if !inner_iids.is_null() {
                    CoTaskMemFree(Some(inner_iids.cast()));
                }
                return E_OUTOFMEMORY;
            }
        };
        if total > u32::MAX as usize {
            if !inner_iids.is_null() {
                CoTaskMemFree(Some(inner_iids.cast()));
            }
            return E_OUTOFMEMORY;
        }
        if total == 0 {
            return S_OK;
        }
        let bytes = match total.checked_mul(std::mem::size_of::<GUID>()) {
            Some(bytes) => bytes,
            None => {
                if !inner_iids.is_null() {
                    CoTaskMemFree(Some(inner_iids.cast()));
                }
                return E_OUTOFMEMORY;
            }
        };
        let combined = CoTaskMemAlloc(bytes) as *mut GUID;
        if combined.is_null() {
            if !inner_iids.is_null() {
                CoTaskMemFree(Some(inner_iids.cast()));
            }
            return E_OUTOFMEMORY;
        }
        std::ptr::copy_nonoverlapping(local_iids.as_ptr(), combined, local_iids.len());
        if inner_count != 0 {
            std::ptr::copy_nonoverlapping(
                inner_iids,
                combined.add(local_iids.len()),
                inner_count as usize,
            );
        }
        if !inner_iids.is_null() {
            CoTaskMemFree(Some(inner_iids.cast()));
        }
        *count = total as u32;
        *result = combined;
        S_OK
    }

    pub(crate) unsafe fn get_runtime_class_name(&self, result: *mut *mut c_void) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        match self.inner() {
            Ok(Some(inner)) => {
                let vtable = *(inner.as_raw() as *const *const IInspectableVtbl);
                ((*vtable).get_runtime_class_name)(inner.as_raw(), result)
            }
            Ok(None) => {
                *result = std::ptr::null_mut();
                S_OK
            }
            Err(error) => error,
        }
    }

    pub(crate) unsafe fn get_trust_level(&self, result: *mut i32) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        match self.inner() {
            Ok(Some(inner)) => {
                let vtable = *(inner.as_raw() as *const *const IInspectableVtbl);
                ((*vtable).get_trust_level)(inner.as_raw(), result)
            }
            Ok(None) => {
                *result = 0;
                S_OK
            }
            Err(error) => error,
        }
    }
}

#[repr(C)]
struct GenericOuter {
    vtable: *const IInspectableVtbl,
    state: CompositionState,
}

impl GenericOuter {
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

    fn create(agile: bool) -> IUnknown {
        let outer = Box::new(Self {
            vtable: &Self::VTABLE,
            state: CompositionState::new(agile),
        });
        unsafe { IUnknown::from_raw(Box::into_raw(outer).cast()) }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AbiSize {
    width: f32,
    height: f32,
}

#[repr(C)]
struct DynamicLocalInterface {
    vtable: *const *const c_void,
    owner: *mut DynamicOuter,
    interface_index: usize,
    _vtable_storage: Box<[*const c_void]>,
}

#[repr(C)]
struct DynamicOuter {
    vtable: *const IInspectableVtbl,
    state: CompositionState,
    local_interfaces: Vec<Box<DynamicLocalInterface>>,
    local_iids: Vec<GUID>,
    callbacks: Vec<Vec<Option<LocalOverrideCallback>>>,
}

// The outer's mutable state is synchronized and callbacks explicitly require
// Send + Sync. Raw pointers only point into the same pinned allocation.
unsafe impl Send for DynamicOuter {}
unsafe impl Sync for DynamicOuter {}

macro_rules! void_thunk {
    ($name:ident, $index:expr) => {
        unsafe extern "system" fn $name(this: *mut c_void) -> HRESULT {
            DynamicOuter::dispatch_void0(this, $index)
        }
    };
}

macro_rules! size_thunk {
    ($name:ident, $index:expr) => {
        unsafe extern "system" fn $name(
            this: *mut c_void,
            value: AbiSize,
            result: *mut AbiSize,
        ) -> HRESULT {
            DynamicOuter::dispatch_size_to_size(this, $index, value, result)
        }
    };
}

macro_rules! string_bool_thunk {
    ($name:ident, $index:expr) => {
        unsafe extern "system" fn $name(
            this: *mut c_void,
            value: *mut c_void,
            flag: u8,
            result: *mut u8,
        ) -> HRESULT {
            DynamicOuter::dispatch_hstring_bool_to_bool(this, $index, value, flag, result)
        }
    };
}

void_thunk!(void_0, 0);
void_thunk!(void_1, 1);
void_thunk!(void_2, 2);
void_thunk!(void_3, 3);
void_thunk!(void_4, 4);
void_thunk!(void_5, 5);
void_thunk!(void_6, 6);
void_thunk!(void_7, 7);
size_thunk!(size_0, 0);
size_thunk!(size_1, 1);
size_thunk!(size_2, 2);
size_thunk!(size_3, 3);
size_thunk!(size_4, 4);
size_thunk!(size_5, 5);
size_thunk!(size_6, 6);
size_thunk!(size_7, 7);
string_bool_thunk!(string_bool_0, 0);
string_bool_thunk!(string_bool_1, 1);
string_bool_thunk!(string_bool_2, 2);
string_bool_thunk!(string_bool_3, 3);
string_bool_thunk!(string_bool_4, 4);
string_bool_thunk!(string_bool_5, 5);
string_bool_thunk!(string_bool_6, 6);
string_bool_thunk!(string_bool_7, 7);

const VOID_THUNKS: [unsafe extern "system" fn(*mut c_void) -> HRESULT; 8] = [
    void_0, void_1, void_2, void_3, void_4, void_5, void_6, void_7,
];
const SIZE_THUNKS: [unsafe extern "system" fn(*mut c_void, AbiSize, *mut AbiSize) -> HRESULT; 8] = [
    size_0, size_1, size_2, size_3, size_4, size_5, size_6, size_7,
];
const STRING_BOOL_THUNKS: [unsafe extern "system" fn(
    *mut c_void,
    *mut c_void,
    u8,
    *mut u8,
) -> HRESULT; 8] = [
    string_bool_0,
    string_bool_1,
    string_bool_2,
    string_bool_3,
    string_bool_4,
    string_bool_5,
    string_bool_6,
    string_bool_7,
];

impl DynamicOuter {
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

    fn create(agile: bool, specs: Vec<LocalOverrideInterface>) -> windows_core::Result<IUnknown> {
        if specs.is_empty() {
            return Err(windows_core::Error::from_hresult(E_INVALIDARG));
        }
        let mut local_interfaces = Vec::with_capacity(specs.len());
        let mut local_iids = Vec::with_capacity(specs.len());
        let mut callbacks = Vec::with_capacity(specs.len());
        for spec in specs {
            let mut slots = vec![
                Self::local_query_interface as *const () as *const c_void,
                Self::local_add_ref as *const () as *const c_void,
                Self::local_release as *const () as *const c_void,
                Self::local_get_iids as *const () as *const c_void,
                Self::local_get_runtime_class_name as *const () as *const c_void,
                Self::local_get_trust_level as *const () as *const c_void,
            ];
            if spec.methods.len() > VOID_THUNKS.len() {
                return Err(windows_core::Error::from_hresult(E_INVALIDARG));
            }
            for (index, method) in spec.methods.iter().enumerate() {
                slots.push(match method {
                    LocalOverrideAbi::Void0 => VOID_THUNKS[index] as *const () as *const c_void,
                    LocalOverrideAbi::SizeF32ToSizeF32 => {
                        SIZE_THUNKS[index] as *const () as *const c_void
                    }
                    LocalOverrideAbi::HStringBoolToBool => {
                        STRING_BOOL_THUNKS[index] as *const () as *const c_void
                    }
                });
            }
            let storage = slots.into_boxed_slice();
            let vtable = storage.as_ptr();
            let mut method_callbacks = (0..spec.methods.len()).map(|_| None).collect::<Vec<_>>();
            for (index, callback) in spec.callbacks {
                method_callbacks[index] = Some(callback);
            }
            local_iids.push(spec.iid);
            callbacks.push(method_callbacks);
            local_interfaces.push(Box::new(DynamicLocalInterface {
                vtable,
                owner: std::ptr::null_mut(),
                interface_index: local_interfaces.len(),
                _vtable_storage: storage,
            }));
        }

        let mut outer = Box::new(Self {
            vtable: &Self::VTABLE,
            state: CompositionState::new(agile),
            local_interfaces,
            local_iids,
            callbacks,
        });
        let owner = (&mut *outer) as *mut Self;
        for interface in &mut outer.local_interfaces {
            interface.owner = owner;
        }
        Ok(unsafe { IUnknown::from_raw(Box::into_raw(outer).cast()) })
    }
}

impl DynamicOuter {
    unsafe fn from_ptr(this: *mut c_void) -> &'static Self {
        &*(this as *const Self)
    }

    unsafe fn local_from_ptr(this: *mut c_void) -> &'static DynamicLocalInterface {
        &*(this as *const DynamicLocalInterface)
    }

    unsafe fn owner_from_local(this: *mut c_void) -> &'static Self {
        &*Self::local_from_ptr(this).owner
    }

    unsafe fn local_ptr(&self, index: usize) -> *mut c_void {
        (&*self.local_interfaces[index] as *const DynamicLocalInterface)
            .cast_mut()
            .cast()
    }

    unsafe fn forward_interface(
        &self,
        interface_index: usize,
    ) -> std::result::Result<IUnknown, HRESULT> {
        self.state
            .inner_interface(&self.local_iids[interface_index])
    }

    unsafe extern "system" fn query_interface(
        this: *mut c_void,
        iid: *const GUID,
        result: *mut *mut c_void,
    ) -> HRESULT {
        match catch_unwind(AssertUnwindSafe(|| {
            let outer = Self::from_ptr(this);
            outer.state.query_interface(this, iid, result, |iid| {
                outer
                    .local_iids
                    .iter()
                    .position(|candidate| candidate == iid)
                    .map(|index| outer.local_ptr(index))
            })
        })) {
            Ok(hr) => hr,
            Err(_) => {
                if !result.is_null() {
                    *result = std::ptr::null_mut();
                }
                E_FAIL
            }
        }
    }

    unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
        catch_unwind(AssertUnwindSafe(|| Self::from_ptr(this).state.add_ref())).unwrap_or(0)
    }

    unsafe extern "system" fn release(this: *mut c_void) -> u32 {
        catch_unwind(AssertUnwindSafe(|| {
            let outer = Self::from_ptr(this);
            let remaining = outer.state.release();
            if remaining == 0 {
                drop(Box::from_raw(this as *mut Self));
            }
            remaining
        }))
        .unwrap_or(0)
    }

    unsafe extern "system" fn get_iids(
        this: *mut c_void,
        count: *mut u32,
        result: *mut *mut GUID,
    ) -> HRESULT {
        catch_unwind(AssertUnwindSafe(|| {
            let outer = Self::from_ptr(this);
            outer.state.get_iids(&outer.local_iids, count, result)
        }))
        .unwrap_or(E_FAIL)
    }

    unsafe extern "system" fn get_runtime_class_name(
        this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        catch_unwind(AssertUnwindSafe(|| {
            Self::from_ptr(this).state.get_runtime_class_name(result)
        }))
        .unwrap_or(E_FAIL)
    }

    unsafe extern "system" fn get_trust_level(this: *mut c_void, result: *mut i32) -> HRESULT {
        catch_unwind(AssertUnwindSafe(|| {
            Self::from_ptr(this).state.get_trust_level(result)
        }))
        .unwrap_or(E_FAIL)
    }

    unsafe extern "system" fn local_query_interface(
        this: *mut c_void,
        iid: *const GUID,
        result: *mut *mut c_void,
    ) -> HRESULT {
        match catch_unwind(AssertUnwindSafe(|| {
            let local = Self::local_from_ptr(this);
            Self::query_interface(local.owner.cast(), iid, result)
        })) {
            Ok(hr) => hr,
            Err(_) => {
                if !result.is_null() {
                    *result = std::ptr::null_mut();
                }
                E_FAIL
            }
        }
    }

    unsafe extern "system" fn local_add_ref(this: *mut c_void) -> u32 {
        catch_unwind(AssertUnwindSafe(|| {
            Self::owner_from_local(this).state.add_ref()
        }))
        .unwrap_or(0)
    }

    unsafe extern "system" fn local_release(this: *mut c_void) -> u32 {
        catch_unwind(AssertUnwindSafe(|| {
            let owner = Self::local_from_ptr(this).owner;
            Self::release(owner.cast())
        }))
        .unwrap_or(0)
    }

    unsafe extern "system" fn local_get_iids(
        this: *mut c_void,
        count: *mut u32,
        result: *mut *mut GUID,
    ) -> HRESULT {
        catch_unwind(AssertUnwindSafe(|| {
            let owner = Self::owner_from_local(this);
            owner.state.get_iids(&owner.local_iids, count, result)
        }))
        .unwrap_or(E_FAIL)
    }

    unsafe extern "system" fn local_get_runtime_class_name(
        this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        catch_unwind(AssertUnwindSafe(|| {
            Self::owner_from_local(this)
                .state
                .get_runtime_class_name(result)
        }))
        .unwrap_or(E_FAIL)
    }

    unsafe extern "system" fn local_get_trust_level(
        this: *mut c_void,
        result: *mut i32,
    ) -> HRESULT {
        catch_unwind(AssertUnwindSafe(|| {
            Self::owner_from_local(this).state.get_trust_level(result)
        }))
        .unwrap_or(E_FAIL)
    }

    unsafe fn dispatch_void0(this: *mut c_void, method_index: usize) -> HRESULT {
        match catch_unwind(AssertUnwindSafe(|| {
            let local = Self::local_from_ptr(this);
            let owner = &*local.owner;
            if let Some(LocalOverrideCallback::Void(callback)) =
                owner.callbacks[local.interface_index][method_index].as_ref()
            {
                return callback();
            }
            let Ok(inner) = owner.forward_interface(local.interface_index) else {
                return E_NOINTERFACE;
            };
            crate::call::call_winrt_method_0(method_index + 6, inner.as_raw())
        })) {
            Ok(hr) => hr,
            Err(_) => E_FAIL,
        }
    }

    unsafe fn dispatch_size_to_size(
        this: *mut c_void,
        method_index: usize,
        value: AbiSize,
        result: *mut AbiSize,
    ) -> HRESULT {
        match catch_unwind(AssertUnwindSafe(|| {
            if result.is_null() {
                return E_POINTER;
            }
            let local = Self::local_from_ptr(this);
            let owner = &*local.owner;
            if let Some(LocalOverrideCallback::Size(callback)) =
                owner.callbacks[local.interface_index][method_index].as_ref()
            {
                let mut width = 0.0;
                let mut height = 0.0;
                let hr = callback(value.width, value.height, &mut width, &mut height);
                if hr.is_ok() {
                    *result = AbiSize { width, height };
                }
                return hr;
            }
            let Ok(inner) = owner.forward_interface(local.interface_index) else {
                return E_NOINTERFACE;
            };
            let method = crate::call::get_vtable_function_ptr(inner.as_raw(), method_index + 6);
            let method: unsafe extern "system" fn(*mut c_void, AbiSize, *mut AbiSize) -> HRESULT =
                std::mem::transmute(method);
            method(inner.as_raw(), value, result)
        })) {
            Ok(hr) => hr,
            Err(_) => E_FAIL,
        }
    }

    unsafe fn dispatch_hstring_bool_to_bool(
        this: *mut c_void,
        method_index: usize,
        value: *mut c_void,
        flag: u8,
        result: *mut u8,
    ) -> HRESULT {
        match catch_unwind(AssertUnwindSafe(|| {
            if result.is_null() {
                return E_POINTER;
            }
            let local = Self::local_from_ptr(this);
            let owner = &*local.owner;
            let Ok(inner) = owner.forward_interface(local.interface_index) else {
                return E_NOINTERFACE;
            };
            let method = crate::call::get_vtable_function_ptr(inner.as_raw(), method_index + 6);
            let method: unsafe extern "system" fn(
                *mut c_void,
                *mut c_void,
                u8,
                *mut u8,
            ) -> HRESULT = std::mem::transmute(method);
            method(inner.as_raw(), value, flag, result)
        })) {
            Ok(hr) => hr,
            Err(_) => E_FAIL,
        }
    }
}

impl GenericOuter {
    unsafe fn from_ptr(this: *mut c_void) -> &'static Self {
        &*(this as *const Self)
    }

    unsafe extern "system" fn query_interface(
        this: *mut c_void,
        iid: *const GUID,
        result: *mut *mut c_void,
    ) -> HRESULT {
        match catch_unwind(AssertUnwindSafe(|| {
            Self::from_ptr(this)
                .state
                .query_interface(this, iid, result, |_| None)
        })) {
            Ok(hr) => hr,
            Err(_) => {
                if !result.is_null() {
                    *result = std::ptr::null_mut();
                }
                E_FAIL
            }
        }
    }

    unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
        catch_unwind(AssertUnwindSafe(|| Self::from_ptr(this).state.add_ref())).unwrap_or(0)
    }

    unsafe extern "system" fn release(this: *mut c_void) -> u32 {
        catch_unwind(AssertUnwindSafe(|| {
            let outer = Self::from_ptr(this);
            let remaining = outer.state.release();
            if remaining == 0 {
                drop(Box::from_raw(this as *mut Self));
            }
            remaining
        }))
        .unwrap_or(0)
    }

    unsafe extern "system" fn get_iids(
        this: *mut c_void,
        count: *mut u32,
        result: *mut *mut GUID,
    ) -> HRESULT {
        catch_unwind(AssertUnwindSafe(|| {
            Self::from_ptr(this).state.get_iids(&[], count, result)
        }))
        .unwrap_or(E_FAIL)
    }

    unsafe extern "system" fn get_runtime_class_name(
        this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        catch_unwind(AssertUnwindSafe(|| {
            Self::from_ptr(this).state.get_runtime_class_name(result)
        }))
        .unwrap_or(E_FAIL)
    }

    unsafe extern "system" fn get_trust_level(this: *mut c_void, result: *mut i32) -> HRESULT {
        catch_unwind(AssertUnwindSafe(|| {
            Self::from_ptr(this).state.get_trust_level(result)
        }))
        .unwrap_or(E_FAIL)
    }
}

fn validate_nondelegating_inner(inner: &IUnknown, outer: &IUnknown) -> windows_core::Result<()> {
    let mut inner_identity = std::ptr::null_mut();
    unsafe { inner.query(&IUnknown::IID, &mut inner_identity) }.ok()?;
    if inner_identity.is_null() {
        return Err(windows_core::Error::from_hresult(E_NOINTERFACE));
    }
    let delegates_to_outer = inner_identity == outer.as_raw();
    unsafe { drop(IUnknown::from_raw(inner_identity)) };
    if delegates_to_outer {
        Err(windows_core::Error::from_hresult(E_NOINTERFACE))
    } else {
        Ok(())
    }
}

/// Invoke a WinRT composable factory with a non-null outer and retain its inner.
///
/// `args` contains only public constructor arguments. `outer_index` identifies
/// where the ABI-only outer belongs. Factory outputs are ordered exactly as
/// `MethodHandle::invoke` returns them (out parameters, then return value).
pub fn compose_winrt(
    factory: &IUnknown,
    method: &MethodHandle,
    args: &[WinRTValue],
    outer_index: usize,
    inner_output_index: usize,
    instance_output_index: usize,
    agile: bool,
) -> crate::Result<WinRTValue> {
    compose_winrt_with_outer(
        GenericOuter::create(agile),
        factory,
        method,
        args,
        outer_index,
        inner_output_index,
        instance_output_index,
    )
}

/// Compose using metadata-described local override interfaces.
pub fn compose_winrt_with_overrides(
    factory: &IUnknown,
    method: &MethodHandle,
    args: &[WinRTValue],
    outer_index: usize,
    inner_output_index: usize,
    instance_output_index: usize,
    agile: bool,
    overrides: Vec<LocalOverrideInterface>,
) -> crate::Result<WinRTValue> {
    let outer = DynamicOuter::create(agile, overrides)?;
    compose_winrt_with_outer(
        outer,
        factory,
        method,
        args,
        outer_index,
        inner_output_index,
        instance_output_index,
    )
}

fn compose_winrt_with_outer(
    outer: IUnknown,
    factory: &IUnknown,
    method: &MethodHandle,
    args: &[WinRTValue],
    outer_index: usize,
    inner_output_index: usize,
    instance_output_index: usize,
) -> crate::Result<WinRTValue> {
    let mut invoke_args = args.to_vec();
    if outer_index > invoke_args.len() {
        return Err(windows_core::Error::from_hresult(E_UNEXPECTED).into());
    }
    invoke_args.insert(outer_index, WinRTValue::Object(outer.clone()));
    let mut outputs = method.invoke(factory.as_raw(), &invoke_args)?;
    if inner_output_index >= outputs.len()
        || instance_output_index >= outputs.len()
        || inner_output_index == instance_output_index
    {
        return Err(windows_core::Error::from_hresult(E_UNEXPECTED).into());
    }

    let inner = outputs[inner_output_index]
        .as_object()
        .ok_or_else(|| windows_core::Error::from_hresult(E_POINTER))?;
    let instance = outputs[instance_output_index]
        .as_object()
        .ok_or_else(|| windows_core::Error::from_hresult(E_POINTER))?;
    validate_nondelegating_inner(&inner, &outer)?;
    // CompositionState is at the same offset in both pinned outer layouts.
    let vtable = unsafe { *(outer.as_raw() as *const *const IInspectableVtbl) };
    if std::ptr::eq(vtable, &GenericOuter::VTABLE) {
        unsafe { GenericOuter::from_ptr(outer.as_raw()) }
            .state
            .set_inner(inner)?;
    } else if std::ptr::eq(vtable, &DynamicOuter::VTABLE) {
        unsafe { DynamicOuter::from_ptr(outer.as_raw()) }
            .state
            .set_inner(inner)?;
    } else {
        return Err(windows_core::Error::from_hresult(E_UNEXPECTED).into());
    }

    let mut controlling = std::ptr::null_mut();
    unsafe { instance.query(&IUnknown::IID, &mut controlling) }.ok()?;
    if controlling != outer.as_raw() {
        if !controlling.is_null() {
            unsafe { drop(IUnknown::from_raw(controlling)) };
        }
        return Err(windows_core::Error::from_hresult(E_NOINTERFACE).into());
    }
    unsafe { drop(IUnknown::from_raw(controlling)) };

    // Move the requested output out without cloning; all other factory outputs
    // are released normally.
    Ok(outputs.swap_remove(instance_output_index))
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    const IID_FORWARDED: GUID = GUID::from_u128(0x11111111_2222_3333_4444_555555555555);

    #[repr(C)]
    struct TestInner {
        nondelegating_vtable: *const IInspectableVtbl,
        forwarded_vtable: *const IInspectableVtbl,
        ref_count: windows_core::imp::RefCount,
        outer: *mut c_void,
        drops: Arc<AtomicUsize>,
    }

    impl Drop for TestInner {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl TestInner {
        const NONDELEGATING: IInspectableVtbl = IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::query_nondelegating,
                AddRef: Self::add_ref_nondelegating,
                Release: Self::release_nondelegating,
            },
            get_iids: Self::get_iids,
            get_runtime_class_name: Self::get_runtime_class_name,
            get_trust_level: Self::get_trust_level,
        };
        const FORWARDED: IInspectableVtbl = IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::query_forwarded,
                AddRef: Self::add_ref_forwarded,
                Release: Self::release_forwarded,
            },
            get_iids: Self::get_iids,
            get_runtime_class_name: Self::get_runtime_class_name,
            get_trust_level: Self::get_trust_level,
        };

        fn create(outer: *mut c_void, drops: Arc<AtomicUsize>) -> IUnknown {
            let inner = Box::new(Self {
                nondelegating_vtable: &Self::NONDELEGATING,
                forwarded_vtable: &Self::FORWARDED,
                ref_count: windows_core::imp::RefCount::new(1),
                outer,
                drops,
            });
            unsafe { IUnknown::from_raw(Box::into_raw(inner).cast()) }
        }

        unsafe fn from_nondelegating(this: *mut c_void) -> &'static Self {
            &*(this as *const Self)
        }

        unsafe fn from_forwarded(this: *mut c_void) -> &'static Self {
            &*((this as *const *const c_void).sub(1) as *const Self)
        }

        unsafe fn forwarded_ptr(&self) -> *mut c_void {
            (self as *const Self as *mut *mut c_void).add(1).cast()
        }

        unsafe extern "system" fn query_nondelegating(
            this: *mut c_void,
            iid: *const GUID,
            result: *mut *mut c_void,
        ) -> HRESULT {
            if iid.is_null() || result.is_null() {
                return E_POINTER;
            }
            *result = std::ptr::null_mut();
            let inner = Self::from_nondelegating(this);
            if *iid == IUnknown::IID || *iid == IInspectable::IID {
                *result = this;
                inner.ref_count.add_ref();
                S_OK
            } else if *iid == IID_FORWARDED {
                *result = inner.forwarded_ptr();
                Self::add_ref_forwarded(*result);
                S_OK
            } else {
                E_NOINTERFACE
            }
        }

        unsafe extern "system" fn add_ref_nondelegating(this: *mut c_void) -> u32 {
            Self::from_nondelegating(this).ref_count.add_ref()
        }

        unsafe extern "system" fn release_nondelegating(this: *mut c_void) -> u32 {
            let inner = Self::from_nondelegating(this);
            let remaining = inner.ref_count.release();
            if remaining == 0 {
                drop(Box::from_raw(this as *mut Self));
            }
            remaining
        }

        unsafe extern "system" fn query_forwarded(
            this: *mut c_void,
            iid: *const GUID,
            result: *mut *mut c_void,
        ) -> HRESULT {
            if iid.is_null() || result.is_null() {
                return E_POINTER;
            }
            *result = std::ptr::null_mut();
            let inner = Self::from_forwarded(this);
            if *iid == IUnknown::IID || *iid == IInspectable::IID {
                *result = inner.outer;
                Self::add_ref_forwarded(this);
                S_OK
            } else if *iid == IID_FORWARDED {
                *result = this;
                Self::add_ref_forwarded(this);
                S_OK
            } else {
                E_NOINTERFACE
            }
        }

        unsafe extern "system" fn add_ref_forwarded(this: *mut c_void) -> u32 {
            let outer = Self::from_forwarded(this).outer;
            let vtable = *(outer as *const *const windows_core::IUnknown_Vtbl);
            ((*vtable).AddRef)(outer)
        }

        unsafe extern "system" fn release_forwarded(this: *mut c_void) -> u32 {
            let outer = Self::from_forwarded(this).outer;
            let vtable = *(outer as *const *const windows_core::IUnknown_Vtbl);
            ((*vtable).Release)(outer)
        }

        unsafe extern "system" fn get_iids(
            _this: *mut c_void,
            count: *mut u32,
            result: *mut *mut GUID,
        ) -> HRESULT {
            if count.is_null() || result.is_null() {
                return E_POINTER;
            }
            let allocated = CoTaskMemAlloc(std::mem::size_of::<GUID>()) as *mut GUID;
            if allocated.is_null() {
                return E_OUTOFMEMORY;
            }
            allocated.write(IID_FORWARDED);
            *count = 1;
            *result = allocated;
            S_OK
        }

        unsafe extern "system" fn get_runtime_class_name(
            _this: *mut c_void,
            result: *mut *mut c_void,
        ) -> HRESULT {
            if result.is_null() {
                return E_POINTER;
            }
            *result = std::ptr::null_mut();
            S_OK
        }

        unsafe extern "system" fn get_trust_level(_this: *mut c_void, result: *mut i32) -> HRESULT {
            if result.is_null() {
                return E_POINTER;
            }
            *result = 0;
            S_OK
        }
    }

    #[test]
    fn forwarded_interfaces_preserve_outer_identity_and_inner_ownership() {
        let drops = Arc::new(AtomicUsize::new(0));
        let outer = GenericOuter::create(false);
        let inner = TestInner::create(outer.as_raw(), drops.clone());
        unsafe { GenericOuter::from_ptr(outer.as_raw()) }
            .state
            .set_inner(inner)
            .unwrap();

        let mut forwarded = std::ptr::null_mut();
        unsafe { outer.query(&IID_FORWARDED, &mut forwarded) }
            .ok()
            .unwrap();
        let forwarded = unsafe { IUnknown::from_raw(forwarded) };
        let mut identity = std::ptr::null_mut();
        unsafe { forwarded.query(&IUnknown::IID, &mut identity) }
            .ok()
            .unwrap();
        assert_eq!(identity, outer.as_raw());
        unsafe { drop(IUnknown::from_raw(identity)) };

        drop(forwarded);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(outer);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn local_interfaces_win_before_inner_forwarding() {
        let drops = Arc::new(AtomicUsize::new(0));
        let outer = GenericOuter::create(false);
        let host = unsafe { GenericOuter::from_ptr(outer.as_raw()) };
        host.state
            .set_inner(TestInner::create(outer.as_raw(), drops.clone()))
            .unwrap();

        let mut local = std::ptr::null_mut();
        let hr = unsafe {
            host.state
                .query_interface(outer.as_raw(), &IID_FORWARDED, &mut local, |_| {
                    Some(outer.as_raw())
                })
        };
        assert!(hr.is_ok());
        assert_eq!(local, outer.as_raw());
        unsafe { drop(IUnknown::from_raw(local)) };
        drop(outer);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn inner_can_only_be_attached_once_and_null_outputs_are_rejected() {
        let drops = Arc::new(AtomicUsize::new(0));
        let outer = GenericOuter::create(false);
        let host = unsafe { GenericOuter::from_ptr(outer.as_raw()) };
        host.state
            .set_inner(TestInner::create(outer.as_raw(), drops.clone()))
            .unwrap();
        assert!(
            host.state
                .set_inner(TestInner::create(outer.as_raw(), drops.clone()))
                .is_err()
        );

        let vtable = unsafe { *(outer.as_raw() as *const *const IInspectableVtbl) };
        assert_eq!(
            unsafe {
                ((*vtable).base.QueryInterface)(
                    outer.as_raw(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                )
            },
            E_POINTER
        );
        drop(outer);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn agile_behavior_is_exposed_only_when_declared() {
        let apartment_bound = GenericOuter::create(false);
        let mut result = std::ptr::null_mut();
        assert_eq!(
            unsafe { apartment_bound.query(&windows_core::imp::IAgileObject::IID, &mut result,) },
            E_NOINTERFACE
        );
        assert!(result.is_null());

        let agile = GenericOuter::create(true);
        unsafe {
            agile
                .query(&windows_core::imp::IAgileObject::IID, &mut result)
                .ok()
                .unwrap();
        }
        assert_eq!(result, agile.as_raw());
        unsafe { drop(IUnknown::from_raw(result)) };

        let mut marshal = std::ptr::null_mut();
        unsafe {
            agile
                .query(&windows_core::imp::IMarshal::IID, &mut marshal)
                .ok()
                .unwrap();
        }
        assert!(!marshal.is_null());
        unsafe { drop(IUnknown::from_raw(marshal)) };
    }

    #[test]
    fn delegating_inner_is_rejected_before_attachment() {
        let outer = GenericOuter::create(false);
        assert!(validate_nondelegating_inner(&outer, &outer).is_err());

        let drops = Arc::new(AtomicUsize::new(0));
        let inner = TestInner::create(outer.as_raw(), drops);
        assert!(validate_nondelegating_inner(&inner, &outer).is_ok());
    }

    #[test]
    fn dynamic_local_qi_dispatch_identity_and_callback_lifetime() {
        let calls = Arc::new(AtomicUsize::new(0));
        let retained = calls.clone();
        let callback: LocalOverrideVoidCallback = Arc::new(move || {
            retained.fetch_add(1, Ordering::SeqCst);
            S_OK
        });
        let callback_lifetime = callback.clone();
        let spec = LocalOverrideInterface::new(IID_FORWARDED, vec![LocalOverrideAbi::Void0])
            .unwrap()
            .with_void_callback(6, callback)
            .unwrap();
        let outer = DynamicOuter::create(false, vec![spec]).unwrap();

        let mut local = std::ptr::null_mut();
        unsafe { outer.query(&IID_FORWARDED, &mut local) }
            .ok()
            .unwrap();
        assert_ne!(local, outer.as_raw());
        let local_vtable = unsafe { *(local as *const *const *const c_void) };
        let invoke: unsafe extern "system" fn(*mut c_void) -> HRESULT =
            unsafe { std::mem::transmute(*local_vtable.add(6)) };
        assert_eq!(unsafe { invoke(local) }, S_OK);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let mut identity = std::ptr::null_mut();
        let local_unknown = unsafe { IUnknown::from_raw(local) };
        unsafe { local_unknown.query(&IUnknown::IID, &mut identity) }
            .ok()
            .unwrap();
        assert_eq!(identity, outer.as_raw());
        unsafe { drop(IUnknown::from_raw(identity)) };
        drop(local_unknown);
        drop(outer);
        assert_eq!(Arc::strong_count(&callback_lifetime), 1);
    }

    #[test]
    fn dynamic_local_callback_panics_fail_closed() {
        let callback: LocalOverrideVoidCallback = Arc::new(|| panic!("callback panic"));
        let spec = LocalOverrideInterface::new(IID_FORWARDED, vec![LocalOverrideAbi::Void0])
            .unwrap()
            .with_void_callback(6, callback)
            .unwrap();
        let outer = DynamicOuter::create(false, vec![spec]).unwrap();
        let mut local = std::ptr::null_mut();
        unsafe { outer.query(&IID_FORWARDED, &mut local) }
            .ok()
            .unwrap();
        let vtable = unsafe { *(local as *const *const *const c_void) };
        let invoke: unsafe extern "system" fn(*mut c_void) -> HRESULT =
            unsafe { std::mem::transmute(*vtable.add(6)) };
        assert_eq!(unsafe { invoke(local) }, E_FAIL);
        unsafe { drop(IUnknown::from_raw(local)) };
    }

    #[test]
    fn dynamic_size_override_dispatches_exact_abi() {
        let callback: LocalOverrideSizeCallback =
            Arc::new(|width, height, result_width, result_height| {
                *result_width = width / 2.0;
                *result_height = height / 4.0;
                S_OK
            });
        let spec =
            LocalOverrideInterface::new(IID_FORWARDED, vec![LocalOverrideAbi::SizeF32ToSizeF32])
                .unwrap()
                .with_size_callback(6, callback)
                .unwrap();
        let outer = DynamicOuter::create(false, vec![spec]).unwrap();
        let mut local = std::ptr::null_mut();
        unsafe { outer.query(&IID_FORWARDED, &mut local) }
            .ok()
            .unwrap();
        let vtable = unsafe { *(local as *const *const *const c_void) };
        let invoke: unsafe extern "system" fn(*mut c_void, AbiSize, *mut AbiSize) -> HRESULT =
            unsafe { std::mem::transmute(*vtable.add(6)) };
        let mut result = AbiSize {
            width: 0.0,
            height: 0.0,
        };
        assert_eq!(
            unsafe {
                invoke(
                    local,
                    AbiSize {
                        width: 80.0,
                        height: 40.0,
                    },
                    &mut result,
                )
            },
            S_OK
        );
        assert_eq!((result.width, result.height), (40.0, 10.0));
        unsafe { drop(IUnknown::from_raw(local)) };
    }

    #[test]
    fn dynamic_override_specs_reject_wrong_slots_and_shapes() {
        let callback: LocalOverrideVoidCallback = Arc::new(|| S_OK);
        assert!(LocalOverrideInterface::new(IID_FORWARDED, vec![]).is_err());
        assert!(
            LocalOverrideInterface::new(IID_FORWARDED, vec![LocalOverrideAbi::SizeF32ToSizeF32],)
                .unwrap()
                .with_void_callback(6, callback)
                .is_err()
        );
    }
}
