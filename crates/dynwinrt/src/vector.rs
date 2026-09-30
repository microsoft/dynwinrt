// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#![allow(unsafe_op_in_unsafe_fn)]
//! Dynamic WinRT IVector<T> / IIterable<T> / IVectorView<T> / IIterator<T> implementation.
//!
//! Creates COM objects at runtime that implement the WinRT collection interfaces,
//! allowing JS callers to construct vectors and pass them to WinRT APIs.

use core::ffi::c_void;
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI64, Ordering},
};
use windows_core::{GUID, HRESULT, HSTRING, IUnknown, Interface};

use crate::collection_element::{
    CollectionElementPlan, CollectionEquality, CollectionStorage, OwnedPod, PreparedCollectionItem,
};
#[path = "vector_pod.rs"]
mod pod;
use crate::com_helpers::{
    E_BOUNDS, E_FAIL, E_NOTIMPL, IInspectableVtbl, S_OK, com_to_usize, com_usize_addref_out,
    com_usize_release,
};
#[allow(unused_imports)]
use crate::com_helpers::{dual_vtable_com, inspectable_stubs, lock_or, single_vtable_com};
use pod::PodVectorVtables;

// ======================================================================
// IIDs for collection PIIDs
// ======================================================================

/// All IIDs needed for an IVector<T> collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorIids {
    pub iterable: GUID,
    pub vector: GUID,
    pub vector_view: GUID,
    pub observable_vector: GUID,
    pub vector_changed_handler: GUID,
    pub iterator: GUID,
}

// ======================================================================
// COM vtable layouts (matching WinRT ABI)
// ======================================================================

/// IIterable<T> vtable: IInspectable + First()
#[repr(C)]
struct IterableVtbl {
    base: IInspectableVtbl,
    first: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

/// IVector<T> vtable: IInspectable + 12 methods
///
/// By-value slots are typed for the word/reference fast path. POD tables replace
/// their code with libffi closures; those slots must be called with the native
/// element signature, not these erased Rust signatures.
#[repr(C)]
struct VectorVtbl {
    base: IInspectableVtbl,
    get_at: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
    get_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    get_view: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    index_of: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut u32, *mut bool) -> HRESULT,
    set_at: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> HRESULT,
    insert_at: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> HRESULT,
    remove_at: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    append: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    remove_at_end: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    clear: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    get_many:
        unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void, *mut u32) -> HRESULT,
    replace_all: unsafe extern "system" fn(*mut c_void, u32, *const *mut c_void) -> HRESULT,
}

/// IVectorView<T> vtable: IInspectable + 4 methods
#[repr(C)]
struct VectorViewVtbl {
    base: IInspectableVtbl,
    get_at: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
    get_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    index_of: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut u32, *mut bool) -> HRESULT,
    get_many:
        unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void, *mut u32) -> HRESULT,
}

/// IObservableVector<T> vtable: IInspectable + VectorChanged add/remove.
#[repr(C)]
struct ObservableVectorVtbl {
    base: IInspectableVtbl,
    add_vector_changed: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i64) -> HRESULT,
    remove_vector_changed: unsafe extern "system" fn(*mut c_void, i64) -> HRESULT,
}

/// IIterator<T> vtable: IInspectable + 4 methods
#[repr(C)]
struct IteratorVtbl {
    base: IInspectableVtbl,
    get_current: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    get_has_current: unsafe extern "system" fn(*mut c_void, *mut bool) -> HRESULT,
    move_next: unsafe extern "system" fn(*mut c_void, *mut bool) -> HRESULT,
    get_many: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void, *mut u32) -> HRESULT,
}

const IID_IVECTOR_CHANGED_EVENT_ARGS: GUID =
    GUID::from_u128(0x575933df_34fe_4480_af15_07691f3d5d9b);
const COLLECTION_CHANGE_RESET: i32 = 0;
const COLLECTION_CHANGE_ITEM_INSERTED: i32 = 1;
const COLLECTION_CHANGE_ITEM_REMOVED: i32 = 2;
const COLLECTION_CHANGE_ITEM_CHANGED: i32 = 3;

#[repr(C)]
struct VectorChangedEventArgsVtbl {
    base: IInspectableVtbl,
    get_collection_change: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
    get_index: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
}

#[repr(C)]
struct VectorChangedEventArgs {
    vtable: *const VectorChangedEventArgsVtbl,
    ref_count: windows_core::imp::RefCount,
    collection_change: i32,
    index: u32,
}

impl VectorChangedEventArgs {
    const VTBL: VectorChangedEventArgsVtbl = VectorChangedEventArgsVtbl {
        base: IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::qi,
                AddRef: Self::add_ref,
                Release: Self::release,
            },
            get_iids: Self::get_iids_stub,
            get_runtime_class_name: Self::get_runtime_class_name_stub,
            get_trust_level: Self::get_trust_level_stub,
        },
        get_collection_change: Self::get_collection_change,
        get_index: Self::get_index,
    };

    fn create(collection_change: i32, index: u32) -> IUnknown {
        let args = Box::new(Self {
            vtable: &Self::VTBL,
            ref_count: windows_core::imp::RefCount::new(1),
            collection_change,
            index,
        });
        unsafe { IUnknown::from_raw(Box::into_raw(args) as *mut c_void) }
    }

    single_vtable_com!(|_me: &Self| IID_IVECTOR_CHANGED_EVENT_ARGS);
    inspectable_stubs!(stub);

    unsafe extern "system" fn get_collection_change(
        this: *mut c_void,
        result: *mut i32,
    ) -> HRESULT {
        if result.is_null() {
            return crate::com_helpers::E_POINTER;
        }
        *result = Self::from_ptr(this).collection_change;
        S_OK
    }

    unsafe extern "system" fn get_index(this: *mut c_void, result: *mut u32) -> HRESULT {
        if result.is_null() {
            return crate::com_helpers::E_POINTER;
        }
        *result = Self::from_ptr(this).index;
        S_OK
    }

    unsafe fn from_ptr(this: *mut c_void) -> &'static Self {
        &*(this as *const Self)
    }
}

/// Write a raw usize item to an output pointer, AddRef'ing if it's a COM reference type.
/// For value types, only writes `elem_size` bytes to avoid overwriting adjacent memory.
#[inline(always)]
pub(crate) unsafe fn write_item_out(
    storage: CollectionStorage,
    raw: usize,
    result: *mut *mut c_void,
) {
    if let CollectionStorage::Pod(layout) = storage {
        std::ptr::copy_nonoverlapping(raw as *const u8, result.cast::<u8>(), layout.size());
    } else if storage.is_hstring() {
        *result = clone_hstring_raw(raw) as *mut c_void;
    } else if storage.is_value_type() {
        let write_size = storage.element_size();
        debug_assert!(!storage.is_empty_only());
        unsafe {
            std::ptr::copy_nonoverlapping(
                &raw as *const usize as *const u8,
                result as *mut u8,
                write_size,
            );
        }
    } else {
        *result = com_usize_addref_out(raw);
    }
}

unsafe fn write_array_items(storage: CollectionStorage, items: &[usize], result: *mut *mut c_void) {
    for (index, &raw) in items.iter().enumerate() {
        let slot = result
            .cast::<u8>()
            .add(index * storage.array_stride())
            .cast();
        write_item_out(storage, raw, slot);
    }
}

unsafe fn store_array_item(
    storage: CollectionStorage,
    values: *const *mut c_void,
    index: usize,
) -> usize {
    let slot = values.cast::<u8>().add(index * storage.array_stride());
    if matches!(storage, CollectionStorage::Pod(_)) {
        return store_abi_item(storage, slot.cast_mut().cast());
    }
    let raw = if storage.is_value_type() {
        debug_assert!(!storage.is_empty_only());
        let mut word = 0usize;
        std::ptr::copy_nonoverlapping(
            slot,
            (&mut word as *mut usize).cast::<u8>(),
            storage.element_size(),
        );
        word as *mut c_void
    } else {
        slot.cast::<*mut c_void>().read()
    };
    store_abi_item(storage, raw)
}

unsafe fn clone_hstring_raw(raw: usize) -> usize {
    if raw == 0 {
        return 0;
    }
    let raw_ptr = raw as *mut c_void;
    let value: &HSTRING = &*(&raw_ptr as *const *mut c_void as *const HSTRING);
    let cloned: *mut c_void = std::mem::transmute(value.clone());
    cloned as usize
}

unsafe fn release_hstring_raw(raw: usize) {
    if raw != 0 {
        let _value: HSTRING = std::mem::transmute(raw as *mut c_void);
    }
}

pub(crate) unsafe fn clone_stored_item(storage: CollectionStorage, raw: usize) -> usize {
    if let CollectionStorage::Pod(layout) = storage {
        OwnedPod::copy(layout, raw as *const u8).into_raw()
    } else if storage.is_hstring() {
        clone_hstring_raw(raw)
    } else if storage.is_value_type() {
        raw
    } else {
        com_to_usize(raw as *mut c_void)
    }
}

// POD input is an address supplied by libffi or a packed-array slot, not word bits.
pub(crate) unsafe fn store_abi_item(storage: CollectionStorage, raw: *mut c_void) -> usize {
    if let CollectionStorage::Pod(layout) = storage {
        OwnedPod::copy(layout, raw.cast()).into_raw()
    } else if storage.is_hstring() {
        clone_hstring_raw(raw as usize)
    } else if storage.is_value_type() {
        normalize_value_word(raw as usize, storage.element_size())
    } else {
        com_to_usize(raw)
    }
}

pub(crate) unsafe fn release_stored_item(storage: CollectionStorage, raw: usize) {
    if let CollectionStorage::Pod(layout) = storage {
        drop(OwnedPod::from_raw(layout, raw));
    } else if storage.is_hstring() {
        release_hstring_raw(raw);
    } else if !storage.is_value_type() {
        com_usize_release(raw);
    }
}

pub(crate) unsafe fn stored_items_equal(
    storage: CollectionStorage,
    equality: &CollectionEquality,
    left: usize,
    right: usize,
) -> bool {
    if let CollectionStorage::Pod(layout) = storage {
        return equality
            .struct_bytes_equal(
                std::slice::from_raw_parts(left as *const u8, layout.size()),
                std::slice::from_raw_parts(right as *const u8, layout.size()),
            )
            .expect("POD storage requires field equality");
    }
    if let Some(equal) = equality.struct_words_equal(left, right) {
        return equal;
    }
    if !storage.is_hstring() {
        return if storage.is_value_type() {
            normalize_value_word(left, storage.element_size())
                == normalize_value_word(right, storage.element_size())
        } else {
            left == right
        };
    }
    if left == 0 || right == 0 {
        return left == right;
    }
    let left_ptr = left as *mut c_void;
    let right_ptr = right as *mut c_void;
    let left: &HSTRING = &*(&left_ptr as *const *mut c_void as *const HSTRING);
    let right: &HSTRING = &*(&right_ptr as *const *mut c_void as *const HSTRING);
    left == right
}

fn normalize_value_word(value: usize, elem_size: usize) -> usize {
    if elem_size == 0 || elem_size >= std::mem::size_of::<usize>() {
        value
    } else {
        value & ((1usize << (elem_size * 8)) - 1)
    }
}

// ======================================================================
// SingleThreadedVector
// ======================================================================

/// A dynamically-constructed observable WinRT vector COM object.
///
/// Stores items as raw `usize` values. For reference types (COM objects),
/// each usize is a raw IUnknown pointer with manual AddRef/Release.
/// For word-ABI value types, each usize holds the bytes directly.
/// Other checked PODs own aligned allocations addressed by
/// each word and use metadata-prepared by-value entrypoints.
///
/// Implements four interfaces:
/// - IIterable<T>: First() for iteration
/// - IVector<T>: mutable collection operations
/// - IVectorView<T>: read-only live view over the same data
/// - IObservableVector<T>: change notifications for native controls
#[repr(C)]
struct SingleThreadedVector {
    vtable_iterable: *const IterableVtbl,
    vtable_vector: *const VectorVtbl,
    vtable_view: *const VectorViewVtbl,
    vtable_observable: *const ObservableVectorVtbl,
    ref_count: windows_core::imp::RefCount,
    items: Mutex<Vec<usize>>,
    handlers: Mutex<HashMap<i64, IUnknown>>,
    next_token: AtomicI64,
    storage: CollectionStorage,
    equality: CollectionEquality,
    iids: VectorIids,
    pod_vtables: Option<Arc<PodVectorVtables>>,
}

unsafe impl Send for SingleThreadedVector {}
unsafe impl Sync for SingleThreadedVector {}

impl SingleThreadedVector {
    const ITERABLE_VTBL: IterableVtbl = IterableVtbl {
        base: IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::qi_iterable,
                AddRef: Self::add_ref_iterable,
                Release: Self::release_iterable,
            },
            get_iids: Self::get_iids_iterable,
            get_runtime_class_name: Self::get_runtime_class_name_iterable,
            get_trust_level: Self::get_trust_level_iterable,
        },
        first: Self::first,
    };

    const VECTOR_VTBL: VectorVtbl = VectorVtbl {
        base: IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::qi_vector,
                AddRef: Self::add_ref_vector,
                Release: Self::release_vector,
            },
            get_iids: Self::get_iids_vector,
            get_runtime_class_name: Self::get_runtime_class_name_vector,
            get_trust_level: Self::get_trust_level_vector,
        },
        get_at: Self::get_at,
        get_size: Self::get_size,
        get_view: Self::get_view,
        index_of: Self::index_of,
        set_at: Self::set_at,
        insert_at: Self::insert_at,
        remove_at: Self::remove_at,
        append: Self::append,
        remove_at_end: Self::remove_at_end,
        clear: Self::clear,
        get_many: Self::get_many,
        replace_all: Self::replace_all,
    };

    const VIEW_VTBL: VectorViewVtbl = VectorViewVtbl {
        base: IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::qi_view,
                AddRef: Self::add_ref_view,
                Release: Self::release_view,
            },
            get_iids: Self::get_iids_view,
            get_runtime_class_name: Self::get_runtime_class_name_view,
            get_trust_level: Self::get_trust_level_view,
        },
        get_at: Self::view_get_at,
        get_size: Self::view_get_size,
        index_of: Self::view_index_of,
        get_many: Self::view_get_many,
    };

    const OBSERVABLE_VTBL: ObservableVectorVtbl = ObservableVectorVtbl {
        base: IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::qi_observable,
                AddRef: Self::add_ref_observable,
                Release: Self::release_observable,
            },
            get_iids: Self::get_iids_observable,
            get_runtime_class_name: Self::get_runtime_class_name_observable,
            get_trust_level: Self::get_trust_level_observable,
        },
        add_vector_changed: Self::add_vector_changed,
        remove_vector_changed: Self::remove_vector_changed,
    };

    quad_vtable_com!(
        iterable,
        vector,
        view,
        observable,
        vector,
        vector_view,
        observable_vector
    );
    inspectable_stubs!(iterable, vector, view, observable);

    // ------------------------------------------------------------------
    // IIterable<T>
    // ------------------------------------------------------------------

    unsafe extern "system" fn first(this: *mut c_void, result: *mut *mut c_void) -> HRESULT {
        let me = Self::from_iterable_ptr(this);
        let items = lock_or!(me.items, E_FAIL);
        let snapshot = items
            .iter()
            .map(|&raw| unsafe { clone_stored_item(me.storage, raw) })
            .collect();
        let iter = SingleThreadedIterator::create(snapshot, me.storage, me.iids.iterator);
        *result = iter.into_raw();
        S_OK
    }

    // ------------------------------------------------------------------
    // IObservableVector<T>
    // ------------------------------------------------------------------

    fn observable_ptr(&self) -> *mut c_void {
        unsafe { (self as *const Self as *const *const c_void).add(3) as *mut c_void }
    }

    // Keep the owner alive for the whole mutation, including input retention,
    // event callbacks and handler destruction, not just the notification borrow.
    unsafe fn retain_mutation(this: *mut c_void) -> IUnknown {
        IUnknown::from_raw_borrowed(&this).unwrap().clone()
    }

    fn notify_changed(&self, collection_change: i32, index: u32) -> HRESULT {
        let handlers: Vec<IUnknown> = match self.handlers.lock() {
            Ok(handlers) => handlers.values().cloned().collect(),
            Err(_) => return E_FAIL,
        };
        if handlers.is_empty() {
            return S_OK;
        }

        let args = VectorChangedEventArgs::create(collection_change, index);
        let sender = self.observable_ptr();
        let mut first_error = S_OK;
        for handler in handlers {
            let function = unsafe {
                let vtable = *(handler.as_raw() as *const *const *mut c_void);
                *vtable.add(3)
            };
            let invoke: unsafe extern "system" fn(
                *mut c_void,
                *mut c_void,
                *mut c_void,
            ) -> HRESULT = unsafe { std::mem::transmute(function) };
            let result = unsafe { invoke(handler.as_raw(), sender, args.as_raw()) };
            if result.is_err() && first_error.is_ok() {
                first_error = result;
            }
        }
        first_error
    }

    unsafe extern "system" fn add_vector_changed(
        this: *mut c_void,
        handler: *mut c_void,
        token: *mut i64,
    ) -> HRESULT {
        if handler.is_null() || token.is_null() {
            return crate::com_helpers::E_POINTER;
        }
        let me = Self::from_observable_ptr(this);
        let borrowed = match IUnknown::from_raw_borrowed(&handler) {
            Some(handler) => handler,
            None => return crate::com_helpers::E_POINTER,
        };
        let next = me.next_token.fetch_add(1, Ordering::Relaxed);
        lock_or!(me.handlers, E_FAIL).insert(next, borrowed.clone());
        *token = next;
        S_OK
    }

    unsafe extern "system" fn remove_vector_changed(this: *mut c_void, token: i64) -> HRESULT {
        let me = Self::from_observable_ptr(this);
        lock_or!(me.handlers, E_FAIL).remove(&token);
        S_OK
    }

    // ------------------------------------------------------------------
    // IVector<T>
    // ------------------------------------------------------------------

    unsafe extern "system" fn get_at(
        this: *mut c_void,
        index: u32,
        result: *mut *mut c_void,
    ) -> HRESULT {
        let me = Self::from_vector_ptr(this);
        let items = lock_or!(me.items, E_FAIL);
        if (index as usize) >= items.len() {
            return E_BOUNDS;
        }
        let raw = items[index as usize];
        write_item_out(me.storage, raw, result);
        S_OK
    }

    unsafe extern "system" fn get_size(this: *mut c_void, result: *mut u32) -> HRESULT {
        let me = Self::from_vector_ptr(this);
        *result = lock_or!(me.items, E_FAIL).len() as u32;
        S_OK
    }

    unsafe extern "system" fn get_view(this: *mut c_void, result: *mut *mut c_void) -> HRESULT {
        let me = Self::from_vector_ptr(this);
        let items = lock_or!(me.items, E_FAIL);
        let snapshot = items
            .iter()
            .map(|&raw| unsafe { clone_stored_item(me.storage, raw) })
            .collect();
        let view = SingleThreadedVectorView::create(
            snapshot,
            me.storage,
            me.equality.clone(),
            me.iids.clone(),
            me.pod_vtables.clone(),
        );
        // WinRT ABI: get_view must return an IVectorView pointer (second vtable),
        // not the identity/IIterable pointer (first vtable).
        let identity = view.into_raw();
        *result = (identity as *const *const c_void).add(1) as *mut c_void;
        S_OK
    }

    unsafe extern "system" fn index_of(
        this: *mut c_void,
        value: *mut c_void,
        index: *mut u32,
        found: *mut bool,
    ) -> HRESULT {
        Self::index_of_impl(this, value, index, found)
    }

    unsafe fn index_of_impl(
        this: *mut c_void,
        value: *mut c_void,
        index: *mut u32,
        found: *mut bool,
    ) -> HRESULT {
        if index.is_null() || found.is_null() {
            return crate::com_helpers::E_POINTER;
        }
        let me = Self::from_vector_ptr(this);
        // Empty-only aggregates lower to one pointer, but have no value storage.
        if me.storage.is_empty_only() {
            return E_NOTIMPL;
        }
        let items = lock_or!(me.items, E_FAIL);
        let needle = value as usize;
        for (i, &item) in items.iter().enumerate() {
            if stored_items_equal(me.storage, &me.equality, item, needle) {
                *index = i as u32;
                *found = true;
                return S_OK;
            }
        }
        *index = 0;
        *found = false;
        S_OK
    }

    unsafe extern "system" fn set_at(this: *mut c_void, index: u32, value: *mut c_void) -> HRESULT {
        Self::set_at_impl(this, index, value)
    }

    unsafe fn set_at_impl(this: *mut c_void, index: u32, value: *mut c_void) -> HRESULT {
        let _keep_alive = Self::retain_mutation(this);
        let me = Self::from_vector_ptr(this);
        {
            let mut items = lock_or!(me.items, E_FAIL);
            if (index as usize) >= items.len() {
                return E_BOUNDS;
            }
            if me.storage.is_empty_only() {
                return E_NOTIMPL;
            }
            let old = items[index as usize];
            items[index as usize] = store_abi_item(me.storage, value);
            release_stored_item(me.storage, old);
        }
        me.notify_changed(COLLECTION_CHANGE_ITEM_CHANGED, index)
    }

    unsafe extern "system" fn insert_at(
        this: *mut c_void,
        index: u32,
        value: *mut c_void,
    ) -> HRESULT {
        Self::insert_at_impl(this, index, value)
    }

    unsafe fn insert_at_impl(this: *mut c_void, index: u32, value: *mut c_void) -> HRESULT {
        let _keep_alive = Self::retain_mutation(this);
        let me = Self::from_vector_ptr(this);
        {
            let mut items = lock_or!(me.items, E_FAIL);
            if (index as usize) > items.len() {
                return E_BOUNDS;
            }
            if me.storage.is_empty_only() {
                return E_NOTIMPL;
            }
            let val = store_abi_item(me.storage, value);
            items.insert(index as usize, val);
        }
        me.notify_changed(COLLECTION_CHANGE_ITEM_INSERTED, index)
    }

    unsafe extern "system" fn remove_at(this: *mut c_void, index: u32) -> HRESULT {
        let _keep_alive = Self::retain_mutation(this);
        let me = Self::from_vector_ptr(this);
        let removed = {
            let mut items = lock_or!(me.items, E_FAIL);
            if (index as usize) >= items.len() {
                return E_BOUNDS;
            }
            items.remove(index as usize)
        };
        release_stored_item(me.storage, removed);
        me.notify_changed(COLLECTION_CHANGE_ITEM_REMOVED, index)
    }

    unsafe extern "system" fn append(this: *mut c_void, value: *mut c_void) -> HRESULT {
        Self::append_impl(this, value)
    }

    unsafe fn append_impl(this: *mut c_void, value: *mut c_void) -> HRESULT {
        let _keep_alive = Self::retain_mutation(this);
        let me = Self::from_vector_ptr(this);
        if me.storage.is_empty_only() {
            return E_NOTIMPL;
        }
        let val = store_abi_item(me.storage, value);
        let index = {
            let mut items = match me.items.lock() {
                Ok(items) => items,
                Err(_) => {
                    release_stored_item(me.storage, val);
                    return E_FAIL;
                }
            };
            let index = items.len() as u32;
            items.push(val);
            index
        };
        me.notify_changed(COLLECTION_CHANGE_ITEM_INSERTED, index)
    }

    unsafe extern "system" fn remove_at_end(this: *mut c_void) -> HRESULT {
        let _keep_alive = Self::retain_mutation(this);
        let me = Self::from_vector_ptr(this);
        let (removed, index) = {
            let mut items = lock_or!(me.items, E_FAIL);
            if items.is_empty() {
                return E_BOUNDS;
            }
            let index = (items.len() - 1) as u32;
            let removed = match items.pop() {
                Some(v) => v,
                None => return E_BOUNDS,
            };
            (removed, index)
        };
        release_stored_item(me.storage, removed);
        me.notify_changed(COLLECTION_CHANGE_ITEM_REMOVED, index)
    }

    unsafe extern "system" fn clear(this: *mut c_void) -> HRESULT {
        let _keep_alive = Self::retain_mutation(this);
        let me = Self::from_vector_ptr(this);
        let old_items: Vec<usize> = lock_or!(me.items, E_FAIL).drain(..).collect();
        for raw in old_items {
            release_stored_item(me.storage, raw);
        }
        me.notify_changed(COLLECTION_CHANGE_RESET, 0)
    }

    unsafe extern "system" fn get_many(
        this: *mut c_void,
        start_index: u32,
        capacity: u32,
        items_out: *mut *mut c_void,
        actual: *mut u32,
    ) -> HRESULT {
        let me = Self::from_vector_ptr(this);
        let items = lock_or!(me.items, E_FAIL);
        let start = start_index as usize;
        if start > items.len() {
            *actual = 0;
            return S_OK;
        }
        let count = std::cmp::min(capacity as usize, items.len() - start);
        write_array_items(me.storage, &items[start..start + count], items_out);
        *actual = count as u32;
        S_OK
    }

    unsafe extern "system" fn replace_all(
        this: *mut c_void,
        count: u32,
        values: *const *mut c_void,
    ) -> HRESULT {
        let _keep_alive = Self::retain_mutation(this);
        let me = Self::from_vector_ptr(this);
        if count > 0 && me.storage.is_empty_only() {
            return E_NOTIMPL;
        }
        let old_items: Vec<usize> = lock_or!(me.items, E_FAIL).drain(..).collect();
        for raw in old_items {
            release_stored_item(me.storage, raw);
        }
        let mut items = lock_or!(me.items, E_FAIL);
        for i in 0..count as usize {
            let val = store_array_item(me.storage, values, i);
            items.push(val);
        }
        drop(items);
        me.notify_changed(COLLECTION_CHANGE_RESET, 0)
    }

    // ------------------------------------------------------------------
    // IVectorView<T> — live read-only view over the same items
    // ------------------------------------------------------------------

    unsafe extern "system" fn view_get_at(
        this: *mut c_void,
        index: u32,
        result: *mut *mut c_void,
    ) -> HRESULT {
        let me = Self::from_view_ptr(this);
        let items = lock_or!(me.items, E_FAIL);
        if (index as usize) >= items.len() {
            return E_BOUNDS;
        }
        write_item_out(me.storage, items[index as usize], result);
        S_OK
    }

    unsafe extern "system" fn view_get_size(this: *mut c_void, result: *mut u32) -> HRESULT {
        let me = Self::from_view_ptr(this);
        *result = lock_or!(me.items, E_FAIL).len() as u32;
        S_OK
    }

    unsafe extern "system" fn view_index_of(
        this: *mut c_void,
        value: *mut c_void,
        index: *mut u32,
        found: *mut bool,
    ) -> HRESULT {
        Self::view_index_of_impl(this, value, index, found)
    }

    unsafe fn view_index_of_impl(
        this: *mut c_void,
        value: *mut c_void,
        index: *mut u32,
        found: *mut bool,
    ) -> HRESULT {
        if index.is_null() || found.is_null() {
            return crate::com_helpers::E_POINTER;
        }
        let me = Self::from_view_ptr(this);
        if me.storage.is_empty_only() {
            return E_NOTIMPL;
        }
        let items = lock_or!(me.items, E_FAIL);
        let needle = value as usize;
        for (i, &item) in items.iter().enumerate() {
            if stored_items_equal(me.storage, &me.equality, item, needle) {
                *index = i as u32;
                *found = true;
                return S_OK;
            }
        }
        *index = 0;
        *found = false;
        S_OK
    }

    unsafe extern "system" fn view_get_many(
        this: *mut c_void,
        start_index: u32,
        capacity: u32,
        items_out: *mut *mut c_void,
        actual: *mut u32,
    ) -> HRESULT {
        let me = Self::from_view_ptr(this);
        let items = lock_or!(me.items, E_FAIL);
        let start = start_index as usize;
        if start > items.len() {
            *actual = 0;
            return S_OK;
        }
        let count = std::cmp::min(capacity as usize, items.len() - start);
        write_array_items(me.storage, &items[start..start + count], items_out);
        *actual = count as u32;
        S_OK
    }
}

impl Drop for SingleThreadedVector {
    fn drop(&mut self) {
        let items = self
            .items
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for &raw in items.iter() {
            unsafe {
                release_stored_item(self.storage, raw);
            }
        }
    }
}

// ======================================================================
// SingleThreadedVectorView
// ======================================================================

#[repr(C)]
struct SingleThreadedVectorView {
    vtable_iterable: *const IterableVtbl,
    vtable_view: *const VectorViewVtbl,
    ref_count: windows_core::imp::RefCount,
    items: Vec<usize>,
    storage: CollectionStorage,
    equality: CollectionEquality,
    iids: VectorIids,
    pod_vtables: Option<Arc<PodVectorVtables>>,
}

unsafe impl Send for SingleThreadedVectorView {}
unsafe impl Sync for SingleThreadedVectorView {}

impl SingleThreadedVectorView {
    const ITERABLE_VTBL: IterableVtbl = IterableVtbl {
        base: IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::qi_iterable,
                AddRef: Self::add_ref_iterable,
                Release: Self::release_iterable,
            },
            get_iids: Self::get_iids_stub,
            get_runtime_class_name: Self::get_runtime_class_name_stub,
            get_trust_level: Self::get_trust_level_stub,
        },
        first: Self::first,
    };

    const VIEW_VTBL: VectorViewVtbl = VectorViewVtbl {
        base: IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::qi_view,
                AddRef: Self::add_ref_view,
                Release: Self::release_view,
            },
            get_iids: Self::get_iids_stub2,
            get_runtime_class_name: Self::get_runtime_class_name_stub2,
            get_trust_level: Self::get_trust_level_stub2,
        },
        get_at: Self::get_at,
        get_size: Self::get_size,
        index_of: Self::index_of,
        get_many: Self::get_many,
    };

    fn create(
        items: Vec<usize>,
        storage: CollectionStorage,
        equality: CollectionEquality,
        iids: VectorIids,
        pod_vtables: Option<Arc<PodVectorVtables>>,
    ) -> IUnknown {
        let view = Box::new(Self {
            vtable_iterable: &Self::ITERABLE_VTBL,
            vtable_view: pod_vtables
                .as_ref()
                .map_or(&Self::VIEW_VTBL, |plan| &plan.snapshot),
            ref_count: windows_core::imp::RefCount::new(1),
            items,
            storage,
            equality,
            iids,
            pod_vtables,
        });
        unsafe { IUnknown::from_raw(Box::into_raw(view) as *mut c_void) }
    }

    dual_vtable_com!(iterable, view, vector_view);
    inspectable_stubs!(stub, stub2);

    // -- IIterable<T> --

    unsafe extern "system" fn first(this: *mut c_void, result: *mut *mut c_void) -> HRESULT {
        let me = Self::from_iterable_ptr(this);
        let snapshot = me
            .items
            .iter()
            .map(|&raw| unsafe { clone_stored_item(me.storage, raw) })
            .collect();
        let iter = SingleThreadedIterator::create(snapshot, me.storage, me.iids.iterator);
        *result = iter.into_raw();
        S_OK
    }

    // -- IVectorView<T> --

    unsafe extern "system" fn get_at(
        this: *mut c_void,
        index: u32,
        result: *mut *mut c_void,
    ) -> HRESULT {
        let me = Self::from_view_ptr(this);
        if (index as usize) >= me.items.len() {
            return E_BOUNDS;
        }
        let raw = me.items[index as usize];
        write_item_out(me.storage, raw, result);
        S_OK
    }

    unsafe extern "system" fn get_size(this: *mut c_void, result: *mut u32) -> HRESULT {
        let me = Self::from_view_ptr(this);
        *result = me.items.len() as u32;
        S_OK
    }

    unsafe extern "system" fn index_of(
        this: *mut c_void,
        value: *mut c_void,
        index: *mut u32,
        found: *mut bool,
    ) -> HRESULT {
        Self::index_of_impl(this, value, index, found)
    }

    unsafe fn index_of_impl(
        this: *mut c_void,
        value: *mut c_void,
        index: *mut u32,
        found: *mut bool,
    ) -> HRESULT {
        if index.is_null() || found.is_null() {
            return crate::com_helpers::E_POINTER;
        }
        let me = Self::from_view_ptr(this);
        if me.storage.is_empty_only() {
            return E_NOTIMPL;
        }
        let needle = value as usize;
        for (i, &item) in me.items.iter().enumerate() {
            if stored_items_equal(me.storage, &me.equality, item, needle) {
                *index = i as u32;
                *found = true;
                return S_OK;
            }
        }
        *index = 0;
        *found = false;
        S_OK
    }

    unsafe extern "system" fn get_many(
        this: *mut c_void,
        start_index: u32,
        capacity: u32,
        items_out: *mut *mut c_void,
        actual: *mut u32,
    ) -> HRESULT {
        let me = Self::from_view_ptr(this);
        let start = start_index as usize;
        if start > me.items.len() {
            *actual = 0;
            return S_OK;
        }
        let count = std::cmp::min(capacity as usize, me.items.len() - start);
        write_array_items(me.storage, &me.items[start..start + count], items_out);
        *actual = count as u32;
        S_OK
    }
}

impl Drop for SingleThreadedVectorView {
    fn drop(&mut self) {
        for &raw in &self.items {
            unsafe {
                release_stored_item(self.storage, raw);
            }
        }
    }
}

// ======================================================================
// SingleThreadedIterator
// ======================================================================

#[repr(C)]
pub(crate) struct SingleThreadedIterator {
    vtable: *const IteratorVtbl,
    ref_count: windows_core::imp::RefCount,
    items: Vec<usize>,
    storage: CollectionStorage,
    cursor: Mutex<usize>,
    iid_iterator: GUID,
}

unsafe impl Send for SingleThreadedIterator {}
unsafe impl Sync for SingleThreadedIterator {}

impl SingleThreadedIterator {
    const VTBL: IteratorVtbl = IteratorVtbl {
        base: IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::qi,
                AddRef: Self::add_ref,
                Release: Self::release,
            },
            get_iids: Self::get_iids_stub,
            get_runtime_class_name: Self::get_runtime_class_name_stub,
            get_trust_level: Self::get_trust_level_stub,
        },
        get_current: Self::get_current,
        get_has_current: Self::get_has_current,
        move_next: Self::move_next,
        get_many: Self::get_many,
    };

    pub(crate) fn create(
        items: Vec<usize>,
        storage: CollectionStorage,
        iid_iterator: GUID,
    ) -> IUnknown {
        let iter = Box::new(Self {
            vtable: &Self::VTBL,
            ref_count: windows_core::imp::RefCount::new(1),
            items,
            storage,
            cursor: Mutex::new(0),
            iid_iterator,
        });
        unsafe { IUnknown::from_raw(Box::into_raw(iter) as *mut c_void) }
    }

    single_vtable_com!(|me: &Self| me.iid_iterator);
    inspectable_stubs!(stub);

    unsafe extern "system" fn get_current(this: *mut c_void, result: *mut *mut c_void) -> HRESULT {
        let me = &*(this as *const Self);
        let cursor = *lock_or!(me.cursor, E_FAIL);
        if cursor >= me.items.len() {
            return E_BOUNDS;
        }
        let raw = me.items[cursor];
        write_item_out(me.storage, raw, result);
        S_OK
    }

    unsafe extern "system" fn get_has_current(this: *mut c_void, result: *mut bool) -> HRESULT {
        let me = &*(this as *const Self);
        *result = *lock_or!(me.cursor, E_FAIL) < me.items.len();
        S_OK
    }

    unsafe extern "system" fn move_next(this: *mut c_void, result: *mut bool) -> HRESULT {
        let me = &*(this as *const Self);
        let mut cursor = lock_or!(me.cursor, E_FAIL);
        if *cursor < me.items.len() {
            *cursor += 1;
        }
        *result = *cursor < me.items.len();
        S_OK
    }

    unsafe extern "system" fn get_many(
        this: *mut c_void,
        capacity: u32,
        items_out: *mut *mut c_void,
        actual: *mut u32,
    ) -> HRESULT {
        let me = &*(this as *const Self);
        let mut cursor = lock_or!(me.cursor, E_FAIL);
        let remaining = me.items.len().saturating_sub(*cursor);
        let count = std::cmp::min(capacity as usize, remaining);
        if count > 0 {
            write_array_items(me.storage, &me.items[*cursor..*cursor + count], items_out);
        }
        *cursor += count;
        *actual = count as u32;
        S_OK
    }
}

impl Drop for SingleThreadedIterator {
    fn drop(&mut self) {
        for &raw in &self.items {
            unsafe {
                release_stored_item(self.storage, raw);
            }
        }
    }
}

// ======================================================================
// Public API
// ======================================================================

/// Create an IVector<T> COM object from WinRTValue items.
///
/// Validates the complete IID set, exact element types, native argument ABI,
/// and ownership before publication. Checked POD structs use aligned owned
/// storage and metadata-prepared native entrypoints when they do not fit the
/// word ABI. Structs with owned fields and incomplete layouts are rejected.
/// Admitted struct fields compare by value, ignoring padding; floating fields
/// use numerical equality, including equal signed zeros and unequal NaNs.
pub fn create_vector_from_values(
    items: &[crate::WinRTValue],
    element_type: &crate::TypeHandle,
    iids: VectorIids,
) -> crate::Result<IUnknown> {
    let plan = CollectionElementPlan::for_vector(element_type)?;
    if iids != element_type.table().vector_iids(element_type) {
        return Err(crate::Error::InvalidCollectionValue(
            "collection IIDs matching the declared element type",
        ));
    }
    let pod_vtables = if matches!(plan.storage, CollectionStorage::Pod(_)) {
        Some(PodVectorVtables::for_type(element_type)?)
    } else {
        None
    };
    let prepared = items
        .iter()
        .map(|item| plan.prepare(item))
        .collect::<crate::Result<Vec<_>>>()?;
    let packed = prepared
        .into_iter()
        .map(PreparedCollectionItem::into_raw)
        .collect();
    Ok(new_vector(
        packed,
        plan.storage,
        plan.equality,
        iids,
        pod_vtables,
    ))
}

/// Create an IVector<T> COM object from a Vec of IUnknown items (reference types).
///
/// Prefer [`create_vector_from_values`] for checked construction.
///
/// # Safety
///
/// All IIDs must be the complete collection IID set for the same COM-reference
/// element type, never a scalar, HSTRING, or struct. Each item must already be
/// adjusted to that element's interface (IInspectable for Object). The collection
/// transfers these owned references and returns that same interface view.
pub unsafe fn create_vector(items: Vec<IUnknown>, iids: VectorIids) -> IUnknown {
    let raw_items: Vec<usize> = items
        .into_iter()
        .map(|obj| obj.into_raw() as usize)
        .collect();
    new_vector(
        raw_items,
        CollectionStorage::Object,
        CollectionEquality::Storage,
        iids,
        None,
    )
}

/// Create an IVector<T> COM object from raw POD value bytes.
///
/// This metadata-free constructor compares packed bytes, including padding,
/// rather than typed struct fields. Prefer [`create_vector_from_values`] for
/// checked construction and metadata-directed value equality.
///
/// # Safety
///
/// All IIDs, `elem_size`, and bytes must describe exactly the same native POD
/// type. Bytes must contain valid initialized values, with no owned fields.
/// By-value arguments must lower to one integer word on this target (never an
/// ARM64 HFA). Empty large values are allowed only when by-value T lowers to a
/// single indirect pointer: x64 aggregates, or ARM64 non-HFA aggregates >16 bytes.
/// No large values may subsequently be stored; their value-taking methods reject.
///
/// # Panics
///
/// Panics for unsupported sizes or any item whose length differs from `elem_size`.
pub unsafe fn create_value_vector(
    items: Vec<Vec<u8>>,
    elem_size: usize,
    iids: VectorIids,
) -> IUnknown {
    let storage = CollectionStorage::raw_value(elem_size, items.is_empty());
    assert!(
        items.iter().all(|bytes| bytes.len() == elem_size),
        "create_value_vector: every item must match the declared element size"
    );
    let packed: Vec<usize> = items
        .iter()
        .map(|bytes| {
            let mut val: usize = 0;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    &mut val as *mut usize as *mut u8,
                    elem_size,
                );
            }
            val
        })
        .collect();
    new_vector(packed, storage, CollectionEquality::Storage, iids, None)
}

fn new_vector(
    items: Vec<usize>,
    storage: CollectionStorage,
    equality: CollectionEquality,
    iids: VectorIids,
    pod_vtables: Option<Arc<PodVectorVtables>>,
) -> IUnknown {
    let vector = Box::new(SingleThreadedVector {
        vtable_iterable: &SingleThreadedVector::ITERABLE_VTBL,
        vtable_vector: pod_vtables
            .as_ref()
            .map_or(&SingleThreadedVector::VECTOR_VTBL, |plan| &plan.vector),
        vtable_view: pod_vtables
            .as_ref()
            .map_or(&SingleThreadedVector::VIEW_VTBL, |plan| &plan.live),
        vtable_observable: &SingleThreadedVector::OBSERVABLE_VTBL,
        ref_count: windows_core::imp::RefCount::new(1),
        items: Mutex::new(items),
        handlers: Mutex::new(HashMap::new()),
        next_token: AtomicI64::new(1),
        storage,
        equality,
        iids,
        pod_vtables,
    });
    unsafe { IUnknown::from_raw(Box::into_raw(vector) as *mut c_void) }
}

// ======================================================================
// Tests
// ======================================================================

#[cfg(test)]
#[path = "vector_bulk_tests.rs"]
mod bulk_tests;

#[cfg(test)]
#[path = "vector_abi_tests.rs"]
mod abi_tests;

#[cfg(test)]
#[allow(unused_must_use)]
mod tests {
    use super::*;
    fn checked_object_vector(items: Vec<IUnknown>, iids: VectorIids) -> IUnknown {
        let table = crate::MetadataTable::new();
        let values = items
            .into_iter()
            .map(crate::WinRTValue::Object)
            .collect::<Vec<_>>();
        create_vector_from_values(&values, &table.object(), iids).unwrap()
    }
    use crate::metadata_table::MetadataTable;

    #[test]
    fn test_vector_basic_operations() {
        // Create a vector of IUnknown items using Uri objects
        crate::test_apartment::initialize_mta();

        let table = MetadataTable::new();
        let iids = table.vector_iids(&table.object());

        // Create Uri objects as test items
        let uri1 =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com/1")).unwrap();
        let uri2 =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com/2")).unwrap();
        let uri3 =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com/3")).unwrap();

        let items: Vec<IUnknown> = vec![
            uri1.cast().unwrap(),
            uri2.cast().unwrap(),
            uri3.cast().unwrap(),
        ];

        let vector = checked_object_vector(items, iids.clone());

        // Test QI for IVector
        let mut vec_ptr = std::ptr::null_mut();
        unsafe { vector.query(&iids.vector, &mut vec_ptr) }
            .ok()
            .unwrap();
        assert!(!vec_ptr.is_null());

        // Test QI for IIterable
        let mut iter_ptr = std::ptr::null_mut();
        unsafe { vector.query(&iids.iterable, &mut iter_ptr) }
            .ok()
            .unwrap();
        assert!(!iter_ptr.is_null());

        // Test get_Size via raw vtable call
        let vec_obj = unsafe { IUnknown::from_raw(vec_ptr) };
        let vtbl = unsafe { *(vec_ptr as *const *const VectorVtbl) };
        let mut size: u32 = 0;
        let hr = unsafe { ((*vtbl).get_size)(vec_ptr, &mut size) };
        assert_eq!(hr, S_OK);
        assert_eq!(size, 3);

        // Test get_At
        let mut item_ptr: *mut c_void = std::ptr::null_mut();
        let hr = unsafe { ((*vtbl).get_at)(vec_ptr, 0, &mut item_ptr) };
        assert_eq!(hr, S_OK);
        assert!(!item_ptr.is_null());
        // Release the item
        let _ = unsafe { IUnknown::from_raw(item_ptr) };

        // Test get_At out of bounds
        let hr = unsafe { ((*vtbl).get_at)(vec_ptr, 10, &mut item_ptr) };
        assert_eq!(hr, E_BOUNDS);

        // Release vector interface ref
        drop(vec_obj);
        // Release iterable interface ref
        let _ = unsafe { IUnknown::from_raw(iter_ptr) };
    }

    #[test]
    fn test_observable_vector_notifications() {
        use std::sync::Arc;

        crate::test_apartment::initialize_mta();
        let table = MetadataTable::new();
        let object_type = table.object();
        let iids = table.vector_iids(&object_type);
        let vector = checked_object_vector(Vec::new(), iids.clone());
        let changes = Arc::new(Mutex::new(Vec::<(i32, u32)>::new()));
        let callback_changes = changes.clone();
        let args_type = table.interface(IID_IVECTOR_CHANGED_EVENT_ARGS);
        let handler = crate::delegate::create_delegate(
            iids.vector_changed_handler,
            vec![table.object(), args_type],
            Box::new(move |args| {
                let event_args = args[1].as_object().unwrap();
                let mut raw_args = std::ptr::null_mut();
                unsafe {
                    event_args
                        .query(&IID_IVECTOR_CHANGED_EVENT_ARGS, &mut raw_args)
                        .ok()
                        .unwrap();
                }
                let args_object = unsafe { IUnknown::from_raw(raw_args) };
                let vtable =
                    unsafe { *(args_object.as_raw() as *const *const VectorChangedEventArgsVtbl) };
                let mut change = -1;
                let mut index = u32::MAX;
                assert_eq!(
                    unsafe { ((*vtable).get_collection_change)(args_object.as_raw(), &mut change) },
                    S_OK,
                );
                assert_eq!(
                    unsafe { ((*vtable).get_index)(args_object.as_raw(), &mut index,) },
                    S_OK,
                );
                callback_changes.lock().unwrap().push((change, index));
                S_OK
            }),
        );

        let mut observable_ptr = std::ptr::null_mut();
        unsafe {
            vector
                .query(&iids.observable_vector, &mut observable_ptr)
                .ok()
                .unwrap();
        }
        let observable = unsafe { IUnknown::from_raw(observable_ptr) };
        let observable_vtable =
            unsafe { *(observable.as_raw() as *const *const ObservableVectorVtbl) };
        let mut token = 0i64;
        assert_eq!(
            unsafe {
                ((*observable_vtable).add_vector_changed)(
                    observable.as_raw(),
                    handler.as_raw(),
                    &mut token,
                )
            },
            S_OK,
        );

        let mut vector_ptr = std::ptr::null_mut();
        unsafe {
            vector.query(&iids.vector, &mut vector_ptr).ok().unwrap();
        }
        let mutable = unsafe { IUnknown::from_raw(vector_ptr) };
        let vector_vtable = unsafe { *(mutable.as_raw() as *const *const VectorVtbl) };
        let uri = |suffix: &str| {
            windows::Foundation::Uri::CreateUri(&windows_core::HSTRING::from(format!(
                "https://example.com/{suffix}",
            )))
            .unwrap()
            .cast::<IUnknown>()
            .unwrap()
        };
        let first = uri("first");
        let second = uri("second");
        let inserted = uri("inserted");

        assert_eq!(
            unsafe { ((*vector_vtable).append)(mutable.as_raw(), first.as_raw(),) },
            S_OK,
        );
        assert_eq!(
            unsafe { ((*vector_vtable).set_at)(mutable.as_raw(), 0, second.as_raw(),) },
            S_OK,
        );
        assert_eq!(
            unsafe { ((*vector_vtable).insert_at)(mutable.as_raw(), 0, inserted.as_raw(),) },
            S_OK,
        );
        assert_eq!(
            unsafe { ((*vector_vtable).remove_at)(mutable.as_raw(), 1,) },
            S_OK,
        );
        assert_eq!(unsafe { ((*vector_vtable).clear)(mutable.as_raw()) }, S_OK,);

        assert_eq!(
            changes.lock().unwrap().as_slice(),
            [
                (COLLECTION_CHANGE_ITEM_INSERTED, 0),
                (COLLECTION_CHANGE_ITEM_CHANGED, 0),
                (COLLECTION_CHANGE_ITEM_INSERTED, 0),
                (COLLECTION_CHANGE_ITEM_REMOVED, 1),
                (COLLECTION_CHANGE_RESET, 0),
            ],
        );

        assert_eq!(
            unsafe { ((*observable_vtable).remove_vector_changed)(observable.as_raw(), token,) },
            S_OK,
        );
        let after_remove = uri("after-remove");
        assert_eq!(
            unsafe { ((*vector_vtable).append)(mutable.as_raw(), after_remove.as_raw(),) },
            S_OK,
        );
        assert_eq!(changes.lock().unwrap().len(), 5);
    }

    #[test]
    fn test_observable_vector_allows_reentrant_unsubscribe_and_mutation() {
        use std::sync::{
            Arc,
            atomic::{AtomicI64, AtomicUsize, Ordering},
        };

        crate::test_apartment::initialize_mta();
        let table = MetadataTable::new();
        let iids = table.vector_iids(&table.object());
        let vector = checked_object_vector(Vec::new(), iids.clone());
        let mut observable_ptr = std::ptr::null_mut();
        let mut vector_ptr = std::ptr::null_mut();
        unsafe {
            vector
                .query(&iids.observable_vector, &mut observable_ptr)
                .ok()
                .unwrap();
            vector.query(&iids.vector, &mut vector_ptr).ok().unwrap();
        }
        let observable = unsafe { IUnknown::from_raw(observable_ptr) };
        let mutable = unsafe { IUnknown::from_raw(vector_ptr) };
        let observable_raw = observable.as_raw() as usize;
        let vector_raw = mutable.as_raw() as usize;
        let token = Arc::new(AtomicI64::new(0));
        let callback_token = token.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let callback_calls = calls.clone();
        let reentrant_item =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com/reentrant"))
                .unwrap()
                .cast::<IUnknown>()
                .unwrap();
        let reentrant_raw = reentrant_item.as_raw() as usize;
        let handler = crate::delegate::create_delegate(
            iids.vector_changed_handler,
            vec![
                table.object(),
                table.interface(IID_IVECTOR_CHANGED_EVENT_ARGS),
            ],
            Box::new(move |_args| {
                callback_calls.fetch_add(1, Ordering::SeqCst);
                let observable_ptr = observable_raw as *mut c_void;
                let observable_vtable =
                    unsafe { *(observable_ptr as *const *const ObservableVectorVtbl) };
                assert_eq!(
                    unsafe {
                        ((*observable_vtable).remove_vector_changed)(
                            observable_ptr,
                            callback_token.load(Ordering::SeqCst),
                        )
                    },
                    S_OK,
                );

                let vector_ptr = vector_raw as *mut c_void;
                let vector_vtable = unsafe { *(vector_ptr as *const *const VectorVtbl) };
                assert_eq!(
                    unsafe { ((*vector_vtable).append)(vector_ptr, reentrant_raw as *mut c_void,) },
                    S_OK,
                );
                S_OK
            }),
        );
        let observable_vtable =
            unsafe { *(observable.as_raw() as *const *const ObservableVectorVtbl) };
        let mut raw_token = 0i64;
        assert_eq!(
            unsafe {
                ((*observable_vtable).add_vector_changed)(
                    observable.as_raw(),
                    handler.as_raw(),
                    &mut raw_token,
                )
            },
            S_OK,
        );
        token.store(raw_token, Ordering::SeqCst);

        let initial =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com/initial"))
                .unwrap()
                .cast::<IUnknown>()
                .unwrap();
        let vector_vtable = unsafe { *(mutable.as_raw() as *const *const VectorVtbl) };
        assert_eq!(
            unsafe { ((*vector_vtable).append)(mutable.as_raw(), initial.as_raw(),) },
            S_OK,
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let mut size = 0u32;
        assert_eq!(
            unsafe { ((*vector_vtable).get_size)(mutable.as_raw(), &mut size,) },
            S_OK,
        );
        assert_eq!(size, 2);
    }

    #[test]
    fn test_invalid_collection_value_returns_error() {
        let table = MetadataTable::new();
        let element_type = table.hstring();
        let iids = table.vector_iids(&element_type);
        let result = create_vector_from_values(&[crate::WinRTValue::I32(1)], &element_type, iids);

        assert!(matches!(
            result,
            Err(crate::Error::InvalidCollectionValue("HSTRING"))
        ));
    }

    #[test]
    fn test_nonempty_large_struct_vector() {
        let table = MetadataTable::new();
        let rect = table.struct_type(
            "Windows.Graphics.RectInt32",
            &[
                table.i32_type(),
                table.i32_type(),
                table.i32_type(),
                table.i32_type(),
            ],
        );
        let iids = table.vector_iids(&rect);
        let item = crate::WinRTValue::Struct(rect.default_value());

        let object = create_vector_from_values(&[item], &rect, iids).unwrap();
        let vector: windows_collections::IVector<windows::Graphics::RectInt32> =
            object.cast().unwrap();
        assert_eq!(vector.Size().unwrap(), 1);
        assert_eq!(
            vector.GetAt(0).unwrap(),
            windows::Graphics::RectInt32::default()
        );
    }

    #[test]
    fn test_vector_iid_computation() {
        use windows::Foundation::Collections::{IObservableVector, VectorChangedEventHandler};
        use windows_core::HSTRING;

        let table = MetadataTable::new();

        // IVector<String> IID should match the known PIID computation
        let iids = table.vector_iids(&table.hstring());

        // Verify all IIDs are non-zero (they should be computed from SHA-1)
        assert_ne!(iids.iterable, GUID::zeroed());
        assert_ne!(iids.vector, GUID::zeroed());
        assert_ne!(iids.vector_view, GUID::zeroed());
        assert_ne!(iids.observable_vector, GUID::zeroed());
        assert_ne!(iids.vector_changed_handler, GUID::zeroed());
        assert_eq!(iids.observable_vector, IObservableVector::<HSTRING>::IID,);
        assert_eq!(
            iids.vector_changed_handler,
            VectorChangedEventHandler::<HSTRING>::IID,
        );
        assert_ne!(iids.iterator, GUID::zeroed());

        // All should be different from each other
        assert_ne!(iids.iterable, iids.vector);
        assert_ne!(iids.vector, iids.vector_view);
        assert_ne!(iids.vector_view, iids.iterator);
    }

    #[test]
    fn test_vector_append_and_clear() {
        crate::test_apartment::initialize_mta();

        let table = MetadataTable::new();
        let iids = table.vector_iids(&table.object());

        // Start with empty vector
        let vector = checked_object_vector(Vec::new(), iids.clone());

        // QI to IVector
        let mut vec_ptr = std::ptr::null_mut();
        unsafe { vector.query(&iids.vector, &mut vec_ptr) }
            .ok()
            .unwrap();
        let vtbl = unsafe { *(vec_ptr as *const *const VectorVtbl) };

        // Size should be 0
        let mut size: u32 = 0;
        unsafe { ((*vtbl).get_size)(vec_ptr, &mut size) };
        assert_eq!(size, 0);

        // Append an item
        let uri =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com")).unwrap();
        let unk: IUnknown = uri.cast().unwrap();
        let raw = unk.clone().into_raw();
        unsafe { ((*vtbl).append)(vec_ptr, raw) };

        // Size should now be 1
        unsafe { ((*vtbl).get_size)(vec_ptr, &mut size) };
        assert_eq!(size, 1);

        // Clear
        unsafe { ((*vtbl).clear)(vec_ptr) };
        unsafe { ((*vtbl).get_size)(vec_ptr, &mut size) };
        assert_eq!(size, 0);

        let _ = unsafe { IUnknown::from_raw(vec_ptr) };
    }

    #[test]
    fn test_vector_iterator() {
        crate::test_apartment::initialize_mta();

        let table = MetadataTable::new();
        let iids = table.vector_iids(&table.object());

        let uri1 =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com/1")).unwrap();
        let uri2 =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com/2")).unwrap();

        let items: Vec<IUnknown> = vec![uri1.cast().unwrap(), uri2.cast().unwrap()];

        let vector = checked_object_vector(items, iids.clone());

        // QI to IIterable
        let mut iter_iface_ptr = std::ptr::null_mut();
        unsafe { vector.query(&iids.iterable, &mut iter_iface_ptr) }
            .ok()
            .unwrap();
        let iterable_vtbl = unsafe { *(iter_iface_ptr as *const *const IterableVtbl) };

        // Call First()
        let mut iterator_ptr: *mut c_void = std::ptr::null_mut();
        unsafe { ((*iterable_vtbl).first)(iter_iface_ptr, &mut iterator_ptr) };
        assert!(!iterator_ptr.is_null());

        let iter_vtbl = unsafe { *(iterator_ptr as *const *const IteratorVtbl) };

        // HasCurrent should be true
        let mut has_current = false;
        unsafe { ((*iter_vtbl).get_has_current)(iterator_ptr, &mut has_current) };
        assert!(has_current);

        // MoveNext
        let mut has_next = false;
        unsafe { ((*iter_vtbl).move_next)(iterator_ptr, &mut has_next) };
        assert!(has_next); // second item

        unsafe { ((*iter_vtbl).move_next)(iterator_ptr, &mut has_next) };
        assert!(!has_next); // past end

        let _ = unsafe { IUnknown::from_raw(iterator_ptr) };
        let _ = unsafe { IUnknown::from_raw(iter_iface_ptr) };
    }

    #[test]
    fn test_vector_qi_vector_view() {
        // DynVector must support QI for IVectorView (like C++/WinRT's single_threaded_vector)
        crate::test_apartment::initialize_mta();

        let table = MetadataTable::new();
        let iids = table.vector_iids(&table.object());

        let uri =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com")).unwrap();
        let items: Vec<IUnknown> = vec![uri.cast().unwrap()];
        let vector = checked_object_vector(items, iids.clone());

        // QI for IVectorView should succeed
        let mut view_ptr = std::ptr::null_mut();
        unsafe { vector.query(&iids.vector_view, &mut view_ptr) }
            .ok()
            .unwrap();
        assert!(!view_ptr.is_null());

        // Read through IVectorView vtable
        let vtbl = unsafe { *(view_ptr as *const *const VectorViewVtbl) };
        let mut size: u32 = 0;
        let hr = unsafe { ((*vtbl).get_size)(view_ptr, &mut size) };
        assert_eq!(hr, S_OK);
        assert_eq!(size, 1);

        // GetAt through IVectorView
        let mut item_ptr: *mut c_void = std::ptr::null_mut();
        let hr = unsafe { ((*vtbl).get_at)(view_ptr, 0, &mut item_ptr) };
        assert_eq!(hr, S_OK);
        assert!(!item_ptr.is_null());
        let _ = unsafe { IUnknown::from_raw(item_ptr) };

        let _ = unsafe { IUnknown::from_raw(view_ptr) };
    }

    #[test]
    fn test_vector_get_view_returns_vector_view_ptr() {
        // get_view() must return an IVectorView pointer, not IIterable.
        // This was the root cause of ImageObjectExtractor E_NOINTERFACE.
        crate::test_apartment::initialize_mta();

        let table = MetadataTable::new();
        let iids = table.vector_iids(&table.object());

        let uri =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com")).unwrap();
        let items: Vec<IUnknown> = vec![uri.cast().unwrap()];
        let vector = checked_object_vector(items, iids.clone());

        // QI to IVector to call get_view
        let mut vec_ptr = std::ptr::null_mut();
        unsafe { vector.query(&iids.vector, &mut vec_ptr) }
            .ok()
            .unwrap();
        let vtbl = unsafe { *(vec_ptr as *const *const VectorVtbl) };

        // Call get_view
        let mut view_ptr: *mut c_void = std::ptr::null_mut();
        let hr = unsafe { ((*vtbl).get_view)(vec_ptr, &mut view_ptr) };
        assert_eq!(hr, S_OK);
        assert!(!view_ptr.is_null());

        // The returned pointer MUST be usable as IVectorView directly (no QI needed)
        let view_vtbl = unsafe { *(view_ptr as *const *const VectorViewVtbl) };
        let mut size: u32 = 0;
        let hr = unsafe { ((*view_vtbl).get_size)(view_ptr, &mut size) };
        assert_eq!(hr, S_OK);
        assert_eq!(size, 1);

        // QI the view for IUnknown should also work (identity pointer)
        let view_unk = unsafe { IUnknown::from_raw_borrowed(&view_ptr) }.unwrap();
        let mut unk_ptr: *mut c_void = std::ptr::null_mut();
        let hr = unsafe { view_unk.query(&IUnknown::IID, &mut unk_ptr) };
        assert_eq!(hr, S_OK);
        assert!(!unk_ptr.is_null());

        // Release all
        let _ = unsafe { IUnknown::from_raw(unk_ptr) };
        let _ = unsafe { IUnknown::from_raw(view_ptr) };
        let _ = unsafe { IUnknown::from_raw(vec_ptr) };
    }

    #[test]
    fn test_vector_get_view_ref_counting() {
        // Verify ref counting: get_view returns ref=1, Release frees correctly.
        crate::test_apartment::initialize_mta();

        let table = MetadataTable::new();
        let iids = table.vector_iids(&table.object());

        let vector = checked_object_vector(Vec::new(), iids.clone());

        let mut vec_ptr = std::ptr::null_mut();
        unsafe { vector.query(&iids.vector, &mut vec_ptr) }
            .ok()
            .unwrap();
        let vtbl = unsafe { *(vec_ptr as *const *const VectorVtbl) };

        // Call get_view twice — each should return an independent VectorView
        let mut view1: *mut c_void = std::ptr::null_mut();
        let mut view2: *mut c_void = std::ptr::null_mut();
        unsafe { ((*vtbl).get_view)(vec_ptr, &mut view1) };
        unsafe { ((*vtbl).get_view)(vec_ptr, &mut view2) };
        assert!(!view1.is_null());
        assert!(!view2.is_null());
        assert_ne!(view1, view2); // Different snapshots

        // Both should work independently
        let vtbl1 = unsafe { *(view1 as *const *const VectorViewVtbl) };
        let vtbl2 = unsafe { *(view2 as *const *const VectorViewVtbl) };
        let mut s1: u32 = 99;
        let mut s2: u32 = 99;
        unsafe { ((*vtbl1).get_size)(view1, &mut s1) };
        unsafe { ((*vtbl2).get_size)(view2, &mut s2) };
        assert_eq!(s1, 0);
        assert_eq!(s2, 0);

        // Release both via the view pointer — should not crash (no use-after-free).
        // IUnknown::from_raw takes ownership; drop calls Release through the vtable at *view_ptr.
        // Since view_ptr is the IVectorView vtable (second slot), Release goes through
        // dual_vtable_com's release_view, which correctly finds the base and frees.
        drop(unsafe { IUnknown::from_raw(view1) });
        drop(unsafe { IUnknown::from_raw(view2) });
        let _ = unsafe { IUnknown::from_raw(vec_ptr) };
    }

    /// P2: Value-type vector with elem_size < pointer size (e.g. i32 = 4 bytes).
    /// Verifies that get_at writes only elem_size bytes, not a full pointer-width.
    #[test]
    fn test_value_vector_small_elem_size() {
        let table = MetadataTable::new();
        let iids = table.vector_iids(&table.i32_type());

        // Pack i32 values into usize slots
        let items: Vec<Vec<u8>> = vec![42i32.to_ne_bytes().to_vec(), 99i32.to_ne_bytes().to_vec()];

        // Initialized i32 bytes and the complete matching IID set have word ABI on all targets.
        let vector = unsafe { create_value_vector(items, 4, iids.clone()) };

        // QI to IVector
        let mut vec_ptr = std::ptr::null_mut();
        unsafe { vector.query(&iids.vector, &mut vec_ptr) }
            .ok()
            .unwrap();
        let vtbl = unsafe { *(vec_ptr as *const *const VectorVtbl) };

        // get_at should write exactly 4 bytes (i32), not 8 bytes (usize).
        // We test by placing a sentinel in the upper 4 bytes.
        let mut out_buf: [u8; 8] = [0xCC; 8]; // sentinel pattern
        let hr = unsafe { ((*vtbl).get_at)(vec_ptr, 0, out_buf.as_mut_ptr() as *mut *mut c_void) };
        assert_eq!(hr, S_OK);

        // First 4 bytes should be the value 42
        let val = i32::from_ne_bytes([out_buf[0], out_buf[1], out_buf[2], out_buf[3]]);
        assert_eq!(val, 42);

        // Upper 4 bytes should be untouched (sentinel 0xCC)
        assert_eq!(out_buf[4], 0xCC, "write_item_out wrote beyond elem_size");
        assert_eq!(out_buf[5], 0xCC);
        assert_eq!(out_buf[6], 0xCC);
        assert_eq!(out_buf[7], 0xCC);

        // Second element
        let mut out_buf2: [u8; 8] = [0xDD; 8];
        let hr = unsafe { ((*vtbl).get_at)(vec_ptr, 1, out_buf2.as_mut_ptr() as *mut *mut c_void) };
        assert_eq!(hr, S_OK);
        let val2 = i32::from_ne_bytes([out_buf2[0], out_buf2[1], out_buf2[2], out_buf2[3]]);
        assert_eq!(val2, 99);
        assert_eq!(out_buf2[4], 0xDD, "write_item_out wrote beyond elem_size");

        let _ = unsafe { IUnknown::from_raw(vec_ptr) };
    }

    /// P1: Verify that vector COM objects are actually thread-safe (Mutex, not RefCell).
    /// Accessing from multiple threads should not panic.
    #[test]
    fn test_vector_thread_safety() {
        crate::test_apartment::initialize_mta();

        let table = MetadataTable::new();
        let iids = table.vector_iids(&table.object());

        let uri =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com")).unwrap();
        let items: Vec<IUnknown> = vec![uri.cast().unwrap()];
        let vector = checked_object_vector(items, iids.clone());

        // QI to IVector from main thread
        let mut vec_ptr = std::ptr::null_mut();
        unsafe { vector.query(&iids.vector, &mut vec_ptr) }
            .ok()
            .unwrap();

        // Access from another thread — with RefCell this would panic
        let vtbl = unsafe { *(vec_ptr as *const *const VectorVtbl) };
        let vec_ptr_usize = vec_ptr as usize;
        let vtbl_usize = vtbl as usize;
        let handle = std::thread::spawn(move || {
            let vp = vec_ptr_usize as *mut c_void;
            let vt = vtbl_usize as *const VectorVtbl;
            let mut size: u32 = 0;
            let hr = unsafe { ((*vt).get_size)(vp, &mut size) };
            assert_eq!(hr, S_OK);
            assert_eq!(size, 1);
        });
        handle.join().expect("Thread should not panic");

        let _ = unsafe { IUnknown::from_raw(vec_ptr) };
    }
}
