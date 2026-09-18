// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use core::ffi::c_void;
use std::{
    mem::ManuallyDrop,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    sync::{Arc, Weak},
};

use windows::Win32::System::WinRT::{
    IWeakReference, IWeakReference_Vtbl, IWeakReferenceSource_Vtbl,
};
use windows_core::{GUID, HRESULT, IUnknown, Interface};

use super::{E_FAIL, E_NOINTERFACE, E_POINTER, Host, S_OK};

// One native +1 is one Arc strong reference. Using Arc/Weak keeps failed weak
// resolution on the same destruction path as ordinary Release, including a
// concurrent final Release while QueryInterface rejects the requested IID.
pub(super) unsafe fn borrowed<T>(this: *mut c_void) -> ManuallyDrop<Arc<T>> {
    ManuallyDrop::new(unsafe { Arc::from_raw(this.cast::<T>()) })
}

pub(super) unsafe fn pin<T>(this: *mut c_void) -> Arc<T> {
    unsafe {
        Arc::increment_strong_count(this.cast::<T>());
        Arc::from_raw(this.cast::<T>())
    }
}

pub(super) unsafe extern "system" fn add_ref<T>(this: *mut c_void) -> u32 {
    let owner = unsafe { borrowed::<T>(this) };
    unsafe { Arc::increment_strong_count(this.cast::<T>()) };
    Arc::strong_count(&owner) as u32
}

pub(super) unsafe extern "system" fn release<T>(this: *mut c_void) -> u32 {
    let owner = unsafe { Arc::from_raw(this.cast::<T>()) };
    let remaining = Arc::strong_count(&owner) - 1;
    if catch_unwind(AssertUnwindSafe(|| drop(owner))).is_err() {
        eprintln!("[dynwinrt] Panic while releasing a WinRT implementation reference");
    }
    remaining as u32
}

#[repr(C)]
pub(super) struct WeakSource {
    vtable: *const IWeakReferenceSource_Vtbl,
    pub owner: *mut Host,
}

impl WeakSource {
    pub fn new() -> Self {
        Self {
            vtable: &Self::VTABLE,
            owner: ptr::null_mut(),
        }
    }

    const VTABLE: IWeakReferenceSource_Vtbl = IWeakReferenceSource_Vtbl {
        base__: windows_core::IUnknown_Vtbl {
            QueryInterface: Self::query,
            AddRef: Self::add_ref,
            Release: Self::release,
        },
        GetWeakReference: Self::get_weak_reference,
    };

    unsafe fn owner(this: *mut c_void) -> *mut c_void {
        unsafe { (*this.cast::<Self>()).owner.cast() }
    }

    unsafe extern "system" fn query(
        this: *mut c_void,
        iid: *const GUID,
        result: *mut *mut c_void,
    ) -> HRESULT {
        unsafe { Host::query_interface(Self::owner(this), iid, result) }
    }

    unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
        unsafe { add_ref::<Host>(Self::owner(this)) }
    }

    unsafe extern "system" fn release(this: *mut c_void) -> u32 {
        unsafe { release::<Host>(Self::owner(this)) }
    }

    unsafe extern "system" fn get_weak_reference(
        this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        unsafe { result.write(ptr::null_mut()) };
        match catch_unwind(AssertUnwindSafe(|| {
            let owner = unsafe { borrowed::<Host>(Self::owner(this)) };
            let weak = Arc::new(NativeWeakReference {
                vtable: &NativeWeakReference::VTABLE,
                owner: Arc::downgrade(&owner),
                #[cfg(test)]
                after_upgrade: std::sync::Mutex::new(None),
            });
            unsafe { result.write(Arc::into_raw(weak).cast_mut().cast()) };
        })) {
            Ok(()) => S_OK,
            Err(_) => E_FAIL,
        }
    }
}

#[repr(C)]
pub(super) struct NativeWeakReference {
    vtable: *const IWeakReference_Vtbl,
    owner: Weak<Host>,
    #[cfg(test)]
    after_upgrade: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

// This separate weak-reference identity contains no apartment-bound values.
// It only upgrades an atomic Weak and calls thread-safe Host identity methods.
unsafe impl Send for NativeWeakReference {}
unsafe impl Sync for NativeWeakReference {}

impl NativeWeakReference {
    const VTABLE: IWeakReference_Vtbl = IWeakReference_Vtbl {
        base__: windows_core::IUnknown_Vtbl {
            QueryInterface: Self::query,
            AddRef: add_ref::<Self>,
            Release: release::<Self>,
        },
        Resolve: Self::resolve,
    };

    unsafe extern "system" fn query(
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
        let iid = unsafe { &*iid };
        if *iid == IUnknown::IID
            || *iid == IWeakReference::IID
            || *iid == windows_core::imp::IAgileObject::IID
        {
            unsafe {
                add_ref::<Self>(this);
                result.write(this);
            }
            S_OK
        } else if *iid == windows_core::imp::IMarshal::IID {
            // Encapsulated pinned windows-core helper; only this independent
            // weak-reference object is agile, never its implemented target.
            unsafe {
                add_ref::<Self>(this);
                windows_core::imp::marshaler(IUnknown::from_raw(this), result)
            }
        } else {
            E_NOINTERFACE
        }
    }

    unsafe extern "system" fn resolve(
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
        catch_unwind(AssertUnwindSafe(|| {
            let weak = unsafe { &*this.cast::<Self>() };
            let Some(owner) = weak.owner.upgrade() else {
                return S_OK;
            };
            #[cfg(test)]
            {
                let hook = weak.after_upgrade.lock().unwrap().take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            // `owner` releases through Arc on every return path, including
            // E_NOINTERFACE when this pin became the final strong reference.
            unsafe { Host::query_interface(Arc::as_ptr(&owner).cast_mut().cast(), iid, result) }
        }))
        .unwrap_or(E_FAIL)
    }

    #[cfg(test)]
    pub fn after_upgrade(&self, hook: impl FnOnce() + Send + 'static) {
        *self.after_upgrade.lock().unwrap() = Some(Box::new(hook));
    }
}
