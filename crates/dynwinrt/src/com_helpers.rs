// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#![allow(unsafe_op_in_unsafe_fn)]
//! Shared COM boilerplate macros and helpers for dynamic WinRT collection implementations.
//!
//! Used by both `vector.rs` and `map.rs` to avoid duplicating IUnknown/IInspectable plumbing.

use core::ffi::c_void;
use windows_core::{GUID, HRESULT, IUnknown, Interface};

// ======================================================================
// Shared vtable layout
// ======================================================================

/// IInspectable vtable (shared base for all WinRT interfaces).
#[repr(C)]
pub(crate) struct IInspectableVtbl {
    pub base: windows_core::IUnknown_Vtbl,
    pub get_iids: unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut GUID) -> HRESULT,
    pub get_runtime_class_name: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub get_trust_level: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
}

// ======================================================================
// Shared constants
// ======================================================================

pub(crate) const E_BOUNDS: HRESULT = HRESULT(0x8000000Bu32 as i32);
pub(crate) const E_NOINTERFACE: HRESULT = HRESULT(0x80004002u32 as i32);
pub(crate) const E_FAIL: HRESULT = HRESULT(0x80004005u32 as i32);
pub(crate) const E_POINTER: HRESULT = HRESULT(0x80004003u32 as i32);
pub(crate) const S_OK: HRESULT = HRESULT(0);

// ======================================================================
// COM helper functions
// ======================================================================

/// Store a COM pointer as usize, AddRef'ing it.
pub(crate) unsafe fn com_to_usize(raw: *mut c_void) -> usize {
    if !raw.is_null() {
        let borrowed = IUnknown::from_raw_borrowed(&raw).unwrap();
        let cloned = borrowed.clone(); // AddRef
        std::mem::forget(cloned); // don't Release
    }
    raw as usize
}

/// Read a stored COM usize for output: AddRef before handing out.
pub(crate) unsafe fn com_usize_addref_out(raw: usize) -> *mut c_void {
    if raw != 0 {
        let ptr = raw as *mut c_void;
        let borrowed = IUnknown::from_raw_borrowed(&ptr).unwrap();
        let cloned = borrowed.clone(); // AddRef
        std::mem::forget(cloned); // caller owns the ref
    }
    raw as *mut c_void
}

/// Release a COM pointer stored as usize. No-op if null/zero.
pub(crate) unsafe fn com_usize_release(raw: usize) {
    if raw != 0 {
        let _ = IUnknown::from_raw(raw as *mut c_void);
    }
}

// ======================================================================
// Macros
// ======================================================================

/// Generate IInspectable stub functions with unique names.
/// Each invocation creates get_iids_$suffix, get_runtime_class_name_$suffix,
/// and get_trust_level_$suffix — all returning trivial defaults.
macro_rules! inspectable_stubs {
    ($($suffix:ident),+ $(,)?) => {
        $(
            paste::paste! {
                unsafe extern "system" fn [<get_iids_ $suffix>](
                    _this: *mut ::core::ffi::c_void,
                    count: *mut u32,
                    iids: *mut *mut ::windows_core::GUID,
                ) -> ::windows_core::HRESULT {
                    *count = 0;
                    *iids = ::std::ptr::null_mut();
                    $crate::com_helpers::S_OK
                }

                unsafe extern "system" fn [<get_runtime_class_name_ $suffix>](
                    _this: *mut ::core::ffi::c_void,
                    name: *mut *mut ::core::ffi::c_void,
                ) -> ::windows_core::HRESULT {
                    *name = ::std::ptr::null_mut();
                    $crate::com_helpers::S_OK
                }

                unsafe extern "system" fn [<get_trust_level_ $suffix>](
                    _this: *mut ::core::ffi::c_void,
                    level: *mut i32,
                ) -> ::windows_core::HRESULT {
                    *level = 0; // BaseTrust
                    $crate::com_helpers::S_OK
                }
            }
        )+
    };
}

/// Generate triple-vtable COM boilerplate for structs with three vtable pointers
/// (first = iterable, second = vector, third = vector_view).
///
/// The struct layout must be: vtable_first, vtable_second, vtable_third, ...
macro_rules! triple_vtable_com {
    ($first_suffix:ident, $second_suffix:ident, $third_suffix:ident,
     $second_iid_field:ident, $third_iid_field:ident) => {
        paste::paste! {
            /// Recover &Self from the first (iterable) vtable pointer.
            unsafe fn [<from_ $first_suffix _ptr>](this: *mut ::core::ffi::c_void) -> &'static Self {
                &*(this as *const Self)
            }

            /// Recover &Self from the second vtable pointer.
            unsafe fn [<from_ $second_suffix _ptr>](this: *mut ::core::ffi::c_void) -> &'static Self {
                let base = (this as *const *const ::core::ffi::c_void).sub(1) as *const Self;
                &*base
            }

            /// Recover &Self from the third vtable pointer.
            unsafe fn [<from_ $third_suffix _ptr>](this: *mut ::core::ffi::c_void) -> &'static Self {
                let base = (this as *const *const ::core::ffi::c_void).sub(2) as *const Self;
                &*base
            }

            /// Get the first (identity) interface pointer.
            fn [<as_ $first_suffix _ptr>](this: *const Self) -> *mut ::core::ffi::c_void {
                this as *mut ::core::ffi::c_void
            }

            // -- IUnknown for first interface --

            unsafe extern "system" fn [<qi_ $first_suffix>](
                this: *mut ::core::ffi::c_void,
                iid: *const ::windows_core::GUID,
                ppv: *mut *mut ::core::ffi::c_void,
            ) -> ::windows_core::HRESULT {
                Self::qi_impl(Self::[<from_ $first_suffix _ptr>](this), this, iid, ppv)
            }

            unsafe extern "system" fn [<add_ref_ $first_suffix>](this: *mut ::core::ffi::c_void) -> u32 {
                Self::[<from_ $first_suffix _ptr>](this).ref_count.add_ref()
            }

            unsafe extern "system" fn [<release_ $first_suffix>](this: *mut ::core::ffi::c_void) -> u32 {
                let me = Self::[<from_ $first_suffix _ptr>](this);
                let remaining = me.ref_count.release();
                if remaining == 0 {
                    drop(Box::from_raw(this as *mut Self));
                }
                remaining
            }

            // -- IUnknown for second interface --

            unsafe extern "system" fn [<qi_ $second_suffix>](
                this: *mut ::core::ffi::c_void,
                iid: *const ::windows_core::GUID,
                ppv: *mut *mut ::core::ffi::c_void,
            ) -> ::windows_core::HRESULT {
                let me = Self::[<from_ $second_suffix _ptr>](this);
                Self::qi_impl(me, Self::[<as_ $first_suffix _ptr>](me as *const Self), iid, ppv)
            }

            unsafe extern "system" fn [<add_ref_ $second_suffix>](this: *mut ::core::ffi::c_void) -> u32 {
                Self::[<from_ $second_suffix _ptr>](this).ref_count.add_ref()
            }

            unsafe extern "system" fn [<release_ $second_suffix>](this: *mut ::core::ffi::c_void) -> u32 {
                let me = Self::[<from_ $second_suffix _ptr>](this);
                let remaining = me.ref_count.release();
                if remaining == 0 {
                    let base = Self::[<as_ $first_suffix _ptr>](me as *const Self);
                    drop(Box::from_raw(base as *mut Self));
                }
                remaining
            }

            // -- IUnknown for third interface --

            unsafe extern "system" fn [<qi_ $third_suffix>](
                this: *mut ::core::ffi::c_void,
                iid: *const ::windows_core::GUID,
                ppv: *mut *mut ::core::ffi::c_void,
            ) -> ::windows_core::HRESULT {
                let me = Self::[<from_ $third_suffix _ptr>](this);
                Self::qi_impl(me, Self::[<as_ $first_suffix _ptr>](me as *const Self), iid, ppv)
            }

            unsafe extern "system" fn [<add_ref_ $third_suffix>](this: *mut ::core::ffi::c_void) -> u32 {
                Self::[<from_ $third_suffix _ptr>](this).ref_count.add_ref()
            }

            unsafe extern "system" fn [<release_ $third_suffix>](this: *mut ::core::ffi::c_void) -> u32 {
                let me = Self::[<from_ $third_suffix _ptr>](this);
                let remaining = me.ref_count.release();
                if remaining == 0 {
                    let base = Self::[<as_ $first_suffix _ptr>](me as *const Self);
                    drop(Box::from_raw(base as *mut Self));
                }
                remaining
            }

            // -- Shared QI --

            unsafe fn qi_impl(
                me: &Self,
                identity: *mut ::core::ffi::c_void,
                iid: *const ::windows_core::GUID,
                ppv: *mut *mut ::core::ffi::c_void,
            ) -> ::windows_core::HRESULT {
                if iid.is_null() || ppv.is_null() {
                    return ::windows_core::HRESULT(-2147467261); // E_INVALIDARG
                }
                let iid = &*iid;
                if *iid == ::windows_core::IUnknown::IID
                    || *iid == ::windows_core::IInspectable::IID
                    || *iid == ::windows_core::imp::IAgileObject::IID
                    || *iid == me.iids.iterable
                {
                    *ppv = identity;
                    me.ref_count.add_ref();
                    $crate::com_helpers::S_OK
                } else if *iid == me.iids.$second_iid_field {
                    *ppv = (identity as *const *const ::core::ffi::c_void).add(1) as *mut ::core::ffi::c_void;
                    me.ref_count.add_ref();
                    $crate::com_helpers::S_OK
                } else if *iid == me.iids.$third_iid_field {
                    *ppv = (identity as *const *const ::core::ffi::c_void).add(2) as *mut ::core::ffi::c_void;
                    me.ref_count.add_ref();
                    $crate::com_helpers::S_OK
                } else if *iid == ::windows_core::imp::IMarshal::IID {
                    me.ref_count.add_ref();
                    ::windows_core::imp::marshaler(
                        ::core::mem::transmute(identity),
                        ppv,
                    )
                } else {
                    *ppv = ::std::ptr::null_mut();
                    $crate::com_helpers::E_NOINTERFACE
                }
            }
        }
    };
}

/// Generate dual-vtable COM boilerplate for structs with two vtable pointers
/// (first = iterable, second = the main interface).
///
/// Generates: from_first_ptr, from_second_ptr, as_first_ptr,
///   qi_first, add_ref_first, release_first,
///   qi_second, add_ref_second, release_second,
///   qi_impl (parameterized by $second_iid_field).
///
/// $first_suffix / $second_suffix: identifiers used in function names (e.g. iterable / vector).
/// $second_iid_field: the field on `self.iids` checked for the second interface (e.g. vector, vector_view).
macro_rules! dual_vtable_com {
    ($first_suffix:ident, $second_suffix:ident, $second_iid_field:ident) => {
        paste::paste! {
            /// Recover &Self from the first (iterable) vtable pointer.
            unsafe fn [<from_ $first_suffix _ptr>](this: *mut ::core::ffi::c_void) -> &'static Self {
                &*(this as *const Self)
            }

            /// Recover &Self from the second vtable pointer.
            unsafe fn [<from_ $second_suffix _ptr>](this: *mut ::core::ffi::c_void) -> &'static Self {
                let base = (this as *const *const ::core::ffi::c_void).sub(1) as *const Self;
                &*base
            }

            /// Get the first (identity) interface pointer.
            fn [<as_ $first_suffix _ptr>](this: *const Self) -> *mut ::core::ffi::c_void {
                this as *mut ::core::ffi::c_void
            }

            // -- IUnknown for first interface --

            unsafe extern "system" fn [<qi_ $first_suffix>](
                this: *mut ::core::ffi::c_void,
                iid: *const ::windows_core::GUID,
                ppv: *mut *mut ::core::ffi::c_void,
            ) -> ::windows_core::HRESULT {
                Self::qi_impl(Self::[<from_ $first_suffix _ptr>](this), this, iid, ppv)
            }

            unsafe extern "system" fn [<add_ref_ $first_suffix>](this: *mut ::core::ffi::c_void) -> u32 {
                Self::[<from_ $first_suffix _ptr>](this).ref_count.add_ref()
            }

            unsafe extern "system" fn [<release_ $first_suffix>](this: *mut ::core::ffi::c_void) -> u32 {
                let me = Self::[<from_ $first_suffix _ptr>](this);
                let remaining = me.ref_count.release();
                if remaining == 0 {
                    drop(Box::from_raw(this as *mut Self));
                }
                remaining
            }

            // -- IUnknown for second interface --

            unsafe extern "system" fn [<qi_ $second_suffix>](
                this: *mut ::core::ffi::c_void,
                iid: *const ::windows_core::GUID,
                ppv: *mut *mut ::core::ffi::c_void,
            ) -> ::windows_core::HRESULT {
                let me = Self::[<from_ $second_suffix _ptr>](this);
                Self::qi_impl(me, Self::[<as_ $first_suffix _ptr>](me as *const Self), iid, ppv)
            }

            unsafe extern "system" fn [<add_ref_ $second_suffix>](this: *mut ::core::ffi::c_void) -> u32 {
                Self::[<from_ $second_suffix _ptr>](this).ref_count.add_ref()
            }

            unsafe extern "system" fn [<release_ $second_suffix>](this: *mut ::core::ffi::c_void) -> u32 {
                let me = Self::[<from_ $second_suffix _ptr>](this);
                let remaining = me.ref_count.release();
                if remaining == 0 {
                    let base = Self::[<as_ $first_suffix _ptr>](me as *const Self);
                    drop(Box::from_raw(base as *mut Self));
                }
                remaining
            }

            // -- Shared QI --

            unsafe fn qi_impl(
                me: &Self,
                identity: *mut ::core::ffi::c_void,
                iid: *const ::windows_core::GUID,
                ppv: *mut *mut ::core::ffi::c_void,
            ) -> ::windows_core::HRESULT {
                if iid.is_null() || ppv.is_null() {
                    return ::windows_core::HRESULT(-2147467261); // E_INVALIDARG
                }
                let iid = &*iid;
                if *iid == ::windows_core::IUnknown::IID
                    || *iid == ::windows_core::IInspectable::IID
                    || *iid == ::windows_core::imp::IAgileObject::IID
                    || *iid == me.iids.iterable
                {
                    *ppv = identity;
                    me.ref_count.add_ref();
                    $crate::com_helpers::S_OK
                } else if *iid == me.iids.$second_iid_field {
                    // Return pointer to the second vtable (second field)
                    *ppv = (identity as *const *const ::core::ffi::c_void).add(1) as *mut ::core::ffi::c_void;
                    me.ref_count.add_ref();
                    $crate::com_helpers::S_OK
                } else if *iid == ::windows_core::imp::IMarshal::IID {
                    me.ref_count.add_ref();
                    ::windows_core::imp::marshaler(
                        ::core::mem::transmute(identity),
                        ppv,
                    )
                } else {
                    *ppv = ::std::ptr::null_mut();
                    $crate::com_helpers::E_NOINTERFACE
                }
            }
        }
    };
}

/// Generate single-vtable COM boilerplate (qi, add_ref, release) for structs
/// with one vtable pointer. The QI checks IUnknown, IInspectable, IAgileObject,
/// $iid_expr, and IMarshal.
macro_rules! single_vtable_com {
    ($iid_expr:expr) => {
        unsafe extern "system" fn qi(
            this: *mut ::core::ffi::c_void,
            iid: *const ::windows_core::GUID,
            ppv: *mut *mut ::core::ffi::c_void,
        ) -> ::windows_core::HRESULT {
            if iid.is_null() || ppv.is_null() {
                return ::windows_core::HRESULT(-2147467261);
            }
            let iid = &*iid;
            let me = &*(this as *const Self);
            if *iid == ::windows_core::IUnknown::IID
                || *iid == ::windows_core::IInspectable::IID
                || *iid == ::windows_core::imp::IAgileObject::IID
                || *iid == $iid_expr(me)
            {
                *ppv = this;
                me.ref_count.add_ref();
                $crate::com_helpers::S_OK
            } else if *iid == ::windows_core::imp::IMarshal::IID {
                me.ref_count.add_ref();
                ::windows_core::imp::marshaler(::core::mem::transmute(this), ppv)
            } else {
                *ppv = ::std::ptr::null_mut();
                $crate::com_helpers::E_NOINTERFACE
            }
        }

        unsafe extern "system" fn add_ref(this: *mut ::core::ffi::c_void) -> u32 {
            let me = &*(this as *const Self);
            me.ref_count.add_ref()
        }

        unsafe extern "system" fn release(this: *mut ::core::ffi::c_void) -> u32 {
            let me = &*(this as *const Self);
            let remaining = me.ref_count.release();
            if remaining == 0 {
                drop(Box::from_raw(this as *mut Self));
            }
            remaining
        }
    };
}

/// Generate a Drop impl that releases all COM items if !is_value_type.
/// $items_expr: how to get an iterable of &usize from &self (e.g. self.items.lock().unwrap()).
macro_rules! impl_drop_release_items {
    ($ty:ty, lock) => {
        impl Drop for $ty {
            fn drop(&mut self) {
                if !self.is_value_type {
                    // Use if-let to avoid panic on poisoned mutex during drop.
                    // If the lock is poisoned, we leak COM references rather than
                    // panicking across an FFI boundary.
                    if let Ok(items) = self.items.lock() {
                        for &raw in items.iter() {
                            unsafe {
                                $crate::com_helpers::com_usize_release(raw);
                            }
                        }
                    }
                }
            }
        }
    };
    ($ty:ty, direct) => {
        impl Drop for $ty {
            fn drop(&mut self) {
                if !self.is_value_type {
                    for &raw in &self.items {
                        unsafe {
                            $crate::com_helpers::com_usize_release(raw);
                        }
                    }
                }
            }
        }
    };
}

/// Safely lock a Mutex in a COM vtable callback. Returns E_FAIL on poison.
/// Usage: `let items = lock_or!(me.items, E_FAIL);`
macro_rules! lock_or {
    ($mutex:expr, $hr:expr) => {
        match $mutex.lock() {
            Ok(guard) => guard,
            Err(_) => return $hr,
        }
    };
}

// Export macros for use within the crate
pub(crate) use dual_vtable_com;
pub(crate) use impl_drop_release_items;
pub(crate) use inspectable_stubs;
pub(crate) use lock_or;
pub(crate) use single_vtable_com;
