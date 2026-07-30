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
    Mutex,
    atomic::{AtomicI64, Ordering},
};
use windows_core::{GUID, HRESULT, HSTRING, IUnknown, Interface};

use crate::com_helpers::{
    E_BOUNDS, E_FAIL, E_NOTIMPL, IInspectableVtbl, S_OK, com_to_usize, com_usize_addref_out,
    com_usize_release,
};
#[allow(unused_imports)]
use crate::com_helpers::{dual_vtable_com, inspectable_stubs, lock_or, single_vtable_com};

// ======================================================================
// IIDs for collection PIIDs
// ======================================================================

/// All IIDs needed for an IVector<T> collection.
#[derive(Debug, Clone)]
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct CollectionStorage {
    pub(crate) is_value_type: bool,
    pub(crate) is_hstring: bool,
    pub(crate) elem_size: usize,
}

impl CollectionStorage {
    pub(crate) fn is_large_value_type(self) -> bool {
        self.is_value_type && self.elem_size > std::mem::size_of::<usize>()
    }
}

pub(crate) fn collection_storage(
    element_type: &crate::TypeHandle,
) -> crate::Result<CollectionStorage> {
    use crate::TypeKind;

    let kind = element_type.kind();
    let elem_size = element_type.size_of();
    if matches!(kind, TypeKind::F32 | TypeKind::F64 | TypeKind::Guid) {
        return Err(crate::Error::UnsupportedCollectionElement(kind));
    }
    Ok(CollectionStorage {
        is_hstring: kind == TypeKind::HString,
        is_value_type: matches!(
            kind,
            TypeKind::Bool
                | TypeKind::I8
                | TypeKind::U8
                | TypeKind::I16
                | TypeKind::U16
                | TypeKind::Char16
                | TypeKind::I32
                | TypeKind::U32
                | TypeKind::I64
                | TypeKind::U64
                | TypeKind::Enum(_)
                | TypeKind::HResult
                | TypeKind::Struct(_)
        ),
        elem_size,
    })
}

/// Write a raw usize item to an output pointer, AddRef'ing if it's a COM reference type.
/// For value types, only writes `elem_size` bytes to avoid overwriting adjacent memory.
#[inline(always)]
pub(crate) unsafe fn write_item_out(
    storage: CollectionStorage,
    raw: usize,
    result: *mut *mut c_void,
) {
    if storage.is_hstring {
        *result = clone_hstring_raw(raw) as *mut c_void;
    } else if storage.is_value_type {
        // Write only elem_size bytes, clamped to usize width for safety.
        let write_size = storage.elem_size.min(std::mem::size_of::<usize>());
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
    if storage.is_hstring {
        clone_hstring_raw(raw)
    } else if storage.is_value_type {
        raw
    } else {
        com_to_usize(raw as *mut c_void)
    }
}

pub(crate) unsafe fn store_abi_item(storage: CollectionStorage, raw: *mut c_void) -> usize {
    if storage.is_hstring {
        clone_hstring_raw(raw as usize)
    } else if storage.is_value_type {
        normalize_value_word(raw as usize, storage.elem_size)
    } else {
        com_to_usize(raw)
    }
}

pub(crate) unsafe fn release_stored_item(storage: CollectionStorage, raw: usize) {
    if storage.is_hstring {
        release_hstring_raw(raw);
    } else if !storage.is_value_type {
        com_usize_release(raw);
    }
}

pub(crate) unsafe fn stored_items_equal(
    storage: CollectionStorage,
    left: usize,
    right: usize,
) -> bool {
    if !storage.is_hstring {
        return if storage.is_value_type {
            normalize_value_word(left, storage.elem_size)
                == normalize_value_word(right, storage.elem_size)
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
/// For value types (structs ≤ pointer size), each usize holds the struct
/// bytes directly — no refcounting needed.
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
    iids: VectorIids,
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
        let view = SingleThreadedVectorView::create(snapshot, me.storage, me.iids.clone());
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
        let me = Self::from_vector_ptr(this);
        // Large structs use a wider by-value ABI on ARM64, so the arguments
        // following `value` do not match this pointer-sized vtable thunk.
        // Only inspect `this` before returning.
        if me.storage.is_large_value_type() {
            return E_NOTIMPL;
        }
        let items = lock_or!(me.items, E_FAIL);
        let needle = value as usize;
        for (i, &item) in items.iter().enumerate() {
            if stored_items_equal(me.storage, item, needle) {
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
        let me = Self::from_vector_ptr(this);
        {
            let mut items = lock_or!(me.items, E_FAIL);
            if (index as usize) >= items.len() {
                return E_BOUNDS;
            }
            if me.storage.is_large_value_type() {
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
        let me = Self::from_vector_ptr(this);
        {
            let mut items = lock_or!(me.items, E_FAIL);
            if (index as usize) > items.len() {
                return E_BOUNDS;
            }
            if me.storage.is_large_value_type() {
                return E_NOTIMPL;
            }
            let val = store_abi_item(me.storage, value);
            items.insert(index as usize, val);
        }
        me.notify_changed(COLLECTION_CHANGE_ITEM_INSERTED, index)
    }

    unsafe extern "system" fn remove_at(this: *mut c_void, index: u32) -> HRESULT {
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
        let me = Self::from_vector_ptr(this);
        if me.storage.is_large_value_type() {
            return E_NOTIMPL;
        }
        let val = store_abi_item(me.storage, value);
        let index = {
            let mut items = lock_or!(me.items, E_FAIL);
            let index = items.len() as u32;
            items.push(val);
            index
        };
        me.notify_changed(COLLECTION_CHANGE_ITEM_INSERTED, index)
    }

    unsafe extern "system" fn remove_at_end(this: *mut c_void) -> HRESULT {
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
        for i in 0..count {
            let raw = items[start + i];
            write_item_out(me.storage, raw, items_out.add(i));
        }
        *actual = count as u32;
        S_OK
    }

    unsafe extern "system" fn replace_all(
        this: *mut c_void,
        count: u32,
        values: *const *mut c_void,
    ) -> HRESULT {
        let me = Self::from_vector_ptr(this);
        if count > 0 && me.storage.is_large_value_type() {
            return E_NOTIMPL;
        }
        let old_items: Vec<usize> = lock_or!(me.items, E_FAIL).drain(..).collect();
        for raw in old_items {
            release_stored_item(me.storage, raw);
        }
        let mut items = lock_or!(me.items, E_FAIL);
        for i in 0..count as usize {
            let raw = *values.add(i);
            let val = store_abi_item(me.storage, raw);
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
        let me = Self::from_view_ptr(this);
        if me.storage.is_large_value_type() {
            return E_NOTIMPL;
        }
        let items = lock_or!(me.items, E_FAIL);
        let needle = value as usize;
        for (i, &item) in items.iter().enumerate() {
            if stored_items_equal(me.storage, item, needle) {
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
        for i in 0..count {
            write_item_out(me.storage, items[start + i], items_out.add(i));
        }
        *actual = count as u32;
        S_OK
    }
}

impl Drop for SingleThreadedVector {
    fn drop(&mut self) {
        if let Ok(items) = self.items.lock() {
            for &raw in items.iter() {
                unsafe {
                    release_stored_item(self.storage, raw);
                }
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
    iids: VectorIids,
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

    fn create(items: Vec<usize>, storage: CollectionStorage, iids: VectorIids) -> IUnknown {
        let view = Box::new(Self {
            vtable_iterable: &Self::ITERABLE_VTBL,
            vtable_view: &Self::VIEW_VTBL,
            ref_count: windows_core::imp::RefCount::new(1),
            items,
            storage,
            iids,
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
        let me = Self::from_view_ptr(this);
        if me.storage.is_large_value_type() {
            return E_NOTIMPL;
        }
        let needle = value as usize;
        for (i, &item) in me.items.iter().enumerate() {
            if stored_items_equal(me.storage, item, needle) {
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
        for i in 0..count {
            let raw = me.items[start + i];
            write_item_out(me.storage, raw, items_out.add(i));
        }
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
        for i in 0..count {
            let raw = me.items[*cursor + i];
            write_item_out(me.storage, raw, items_out.add(i));
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
/// Automatically handles both reference types (COM objects → AddRef/Release)
/// and value types (structs ≤ pointer size → raw bytes, no refcounting).
pub fn create_vector_from_values(
    items: &[crate::WinRTValue],
    element_type: &crate::TypeHandle,
    iids: VectorIids,
) -> crate::Result<IUnknown> {
    let storage = collection_storage(element_type)?;
    if !items.is_empty() && storage.is_large_value_type() {
        return Err(crate::Error::UnsupportedCollectionElement(
            element_type.kind(),
        ));
    }
    for item in items {
        validate_collection_item(item, storage)?;
    }
    let packed = items
        .iter()
        .map(|item| pack_validated_collection_item(item, storage))
        .collect();
    Ok(new_vector(packed, storage, iids))
}

pub(crate) fn validate_collection_item(
    item: &crate::WinRTValue,
    storage: CollectionStorage,
) -> crate::Result<()> {
    let valid = if storage.is_hstring {
        matches!(item, crate::WinRTValue::HString(_))
    } else if !storage.is_value_type {
        matches!(
            item,
            crate::WinRTValue::Object(_) | crate::WinRTValue::Async(_) | crate::WinRTValue::Null
        )
    } else {
        matches!(
            item,
            crate::WinRTValue::Bool(_)
                | crate::WinRTValue::I8(_)
                | crate::WinRTValue::U8(_)
                | crate::WinRTValue::I16(_)
                | crate::WinRTValue::U16(_)
                | crate::WinRTValue::I32(_)
                | crate::WinRTValue::U32(_)
                | crate::WinRTValue::I64(_)
                | crate::WinRTValue::U64(_)
                | crate::WinRTValue::Enum { .. }
                | crate::WinRTValue::HResult(_)
                | crate::WinRTValue::Struct(_)
        )
    };
    if valid {
        Ok(())
    } else {
        let expected = if storage.is_hstring {
            "HSTRING"
        } else if storage.is_value_type {
            "a pointer-sized scalar or struct"
        } else {
            "a COM object or null"
        };
        Err(crate::Error::InvalidCollectionValue(expected))
    }
}

pub(crate) fn pack_validated_collection_item(
    item: &crate::WinRTValue,
    storage: CollectionStorage,
) -> usize {
    if storage.is_hstring {
        return match item {
            crate::WinRTValue::HString(value) => {
                let cloned: *mut c_void = unsafe { std::mem::transmute(value.clone()) };
                cloned as usize
            }
            _ => unreachable!("collection item was validated"),
        };
    }
    if !storage.is_value_type {
        return match item {
            crate::WinRTValue::Null => 0,
            _ => {
                let object = item.as_object().expect("collection item was validated");
                unsafe { com_to_usize(object.as_raw()) }
            }
        };
    }

    match item {
        crate::WinRTValue::Bool(value) => usize::from(*value),
        crate::WinRTValue::I8(value) => *value as usize,
        crate::WinRTValue::U8(value) => *value as usize,
        crate::WinRTValue::I16(value) => *value as usize,
        crate::WinRTValue::U16(value) => *value as usize,
        crate::WinRTValue::I32(value) => *value as usize,
        crate::WinRTValue::U32(value) => *value as usize,
        crate::WinRTValue::I64(value) => *value as usize,
        crate::WinRTValue::U64(value) => *value as usize,
        crate::WinRTValue::Enum { value, .. } => *value as usize,
        crate::WinRTValue::HResult(value) => value.0 as usize,
        crate::WinRTValue::Struct(data) => {
            let mut value = 0usize;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    &mut value as *mut usize as *mut u8,
                    storage.elem_size,
                );
            }
            value
        }
        _ => unreachable!("collection item was validated"),
    }
}

/// Create an IVector<T> COM object from a Vec of IUnknown items (reference types).
pub fn create_vector(items: Vec<IUnknown>, iids: VectorIids) -> IUnknown {
    let raw_items: Vec<usize> = items
        .into_iter()
        .map(|obj| obj.into_raw() as usize)
        .collect();
    new_vector(
        raw_items,
        CollectionStorage {
            is_value_type: false,
            is_hstring: false,
            elem_size: std::mem::size_of::<*mut c_void>(),
        },
        iids,
    )
}

/// Create an IVector<T> COM object for value types (structs ≤ pointer size).
pub fn create_value_vector(items: Vec<Vec<u8>>, elem_size: usize, iids: VectorIids) -> IUnknown {
    if !items.is_empty() {
        assert!(
            elem_size <= std::mem::size_of::<usize>(),
            "create_value_vector: elem_size {} exceeds pointer size; not yet supported",
            elem_size
        );
    }
    let packed: Vec<usize> = items
        .iter()
        .map(|bytes| {
            let mut val: usize = 0;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    &mut val as *mut usize as *mut u8,
                    bytes.len().min(std::mem::size_of::<usize>()),
                );
            }
            val
        })
        .collect();
    new_vector(
        packed,
        CollectionStorage {
            is_value_type: true,
            is_hstring: false,
            elem_size,
        },
        iids,
    )
}

fn new_vector(items: Vec<usize>, storage: CollectionStorage, iids: VectorIids) -> IUnknown {
    let vector = Box::new(SingleThreadedVector {
        vtable_iterable: &SingleThreadedVector::ITERABLE_VTBL,
        vtable_vector: &SingleThreadedVector::VECTOR_VTBL,
        vtable_view: &SingleThreadedVector::VIEW_VTBL,
        vtable_observable: &SingleThreadedVector::OBSERVABLE_VTBL,
        ref_count: windows_core::imp::RefCount::new(1),
        items: Mutex::new(items),
        handlers: Mutex::new(HashMap::new()),
        next_token: AtomicI64::new(1),
        storage,
        iids,
    });
    unsafe { IUnknown::from_raw(Box::into_raw(vector) as *mut c_void) }
}

// ======================================================================
// Tests
// ======================================================================

#[cfg(test)]
#[allow(unused_must_use)]
mod tests {
    use super::*;
    use crate::metadata_table::MetadataTable;

    #[test]
    fn test_vector_basic_operations() {
        // Create a vector of IUnknown items using Uri objects
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

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

        let vector = create_vector(items, iids.clone());

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

        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let table = MetadataTable::new();
        let object_type = table.object();
        let iids = table.vector_iids(&object_type);
        let vector = create_vector(Vec::new(), iids.clone());
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

        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let table = MetadataTable::new();
        let iids = table.vector_iids(&table.object());
        let vector = create_vector(Vec::new(), iids.clone());
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
    fn test_empty_large_struct_vector_is_supported_but_not_mutable() {
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct RectInt32Abi {
            x: i32,
            y: i32,
            width: i32,
            height: i32,
        }

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
        let vector = create_vector_from_values(&[], &rect, iids.clone()).unwrap();
        let mut vector_ptr = std::ptr::null_mut();
        unsafe { vector.query(&iids.vector, &mut vector_ptr) }
            .ok()
            .unwrap();
        let vtable = unsafe { *(vector_ptr as *const *const VectorVtbl) };
        let mut size = u32::MAX;

        assert_eq!(unsafe { ((*vtable).get_size)(vector_ptr, &mut size) }, S_OK);
        assert_eq!(size, 0);
        assert_eq!(
            unsafe { ((*vtable).append)(vector_ptr, std::ptr::null_mut()) },
            E_NOTIMPL
        );

        let rect = RectInt32Abi {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        };
        let vector_index_of: unsafe extern "system" fn(
            *mut c_void,
            RectInt32Abi,
            *mut u32,
            *mut bool,
        ) -> HRESULT = unsafe { std::mem::transmute((*vtable).index_of) };
        let mut index = u32::MAX;
        let mut found = true;
        assert_eq!(
            unsafe { vector_index_of(vector_ptr, rect, &mut index, &mut found) },
            E_NOTIMPL
        );
        assert_eq!(index, u32::MAX);
        assert!(found);

        let mut live_view_ptr = std::ptr::null_mut();
        unsafe { vector.query(&iids.vector_view, &mut live_view_ptr) }
            .ok()
            .unwrap();
        let live_view_vtable = unsafe { *(live_view_ptr as *const *const VectorViewVtbl) };
        let live_view_index_of: unsafe extern "system" fn(
            *mut c_void,
            RectInt32Abi,
            *mut u32,
            *mut bool,
        ) -> HRESULT = unsafe { std::mem::transmute((*live_view_vtable).index_of) };
        assert_eq!(
            unsafe { live_view_index_of(live_view_ptr, rect, &mut index, &mut found) },
            E_NOTIMPL
        );

        let mut snapshot_view_ptr = std::ptr::null_mut();
        assert_eq!(
            unsafe { ((*vtable).get_view)(vector_ptr, &mut snapshot_view_ptr) },
            S_OK
        );
        let snapshot_view_vtable = unsafe { *(snapshot_view_ptr as *const *const VectorViewVtbl) };
        let snapshot_view_index_of: unsafe extern "system" fn(
            *mut c_void,
            RectInt32Abi,
            *mut u32,
            *mut bool,
        ) -> HRESULT = unsafe { std::mem::transmute((*snapshot_view_vtable).index_of) };
        assert_eq!(
            unsafe { snapshot_view_index_of(snapshot_view_ptr, rect, &mut index, &mut found) },
            E_NOTIMPL
        );

        drop(unsafe { IUnknown::from_raw(snapshot_view_ptr) });
        drop(unsafe { IUnknown::from_raw(live_view_ptr) });
        drop(unsafe { IUnknown::from_raw(vector_ptr) });
    }

    #[test]
    fn test_nonempty_large_struct_vector_remains_unsupported() {
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

        assert!(matches!(
            create_vector_from_values(&[item], &rect, iids),
            Err(crate::Error::UnsupportedCollectionElement(
                crate::TypeKind::Struct(_)
            ))
        ));
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
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        let table = MetadataTable::new();
        let iids = table.vector_iids(&table.object());

        // Start with empty vector
        let vector = create_vector(Vec::new(), iids.clone());

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
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        let table = MetadataTable::new();
        let iids = table.vector_iids(&table.object());

        let uri1 =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com/1")).unwrap();
        let uri2 =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com/2")).unwrap();

        let items: Vec<IUnknown> = vec![uri1.cast().unwrap(), uri2.cast().unwrap()];

        let vector = create_vector(items, iids.clone());

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
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        let table = MetadataTable::new();
        let iids = table.vector_iids(&table.object());

        let uri =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com")).unwrap();
        let items: Vec<IUnknown> = vec![uri.cast().unwrap()];
        let vector = create_vector(items, iids.clone());

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
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        let table = MetadataTable::new();
        let iids = table.vector_iids(&table.object());

        let uri =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com")).unwrap();
        let items: Vec<IUnknown> = vec![uri.cast().unwrap()];
        let vector = create_vector(items, iids.clone());

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
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        let table = MetadataTable::new();
        let iids = table.vector_iids(&table.object());

        let vector = create_vector(Vec::new(), iids.clone());

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

        let vector = create_value_vector(items, 4, iids.clone());

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
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        let table = MetadataTable::new();
        let iids = table.vector_iids(&table.object());

        let uri =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com")).unwrap();
        let items: Vec<IUnknown> = vec![uri.cast().unwrap()];
        let vector = create_vector(items, iids.clone());

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
