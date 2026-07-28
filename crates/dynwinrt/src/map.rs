// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#![allow(unsafe_op_in_unsafe_fn)]
//! Dynamic WinRT IMap<K,V> / IMapView<K,V> / IKeyValuePair<K,V> / IIterable implementation.
//!
//! Creates COM objects at runtime that implement the WinRT map interfaces,
//! allowing JS callers to construct maps and pass them to WinRT APIs.

use core::ffi::c_void;
use std::sync::Mutex;
use windows_core::{GUID, HRESULT, IUnknown, Interface};

use crate::com_helpers::{E_BOUNDS, E_FAIL, IInspectableVtbl, S_OK};
#[allow(unused_imports)]
use crate::com_helpers::{dual_vtable_com, inspectable_stubs, lock_or, single_vtable_com};
use crate::vector::SingleThreadedIterator;
use crate::vector::{
    CollectionStorage, clone_stored_item, collection_storage, pack_validated_collection_item,
    release_stored_item, store_abi_item, stored_items_equal, validate_collection_item,
    write_item_out,
};

// ======================================================================
// IIDs
// ======================================================================

/// All IIDs needed for an IMap<K,V> collection.
#[derive(Debug, Clone)]
pub struct MapIids {
    pub iterable: GUID, // IIterable<IKeyValuePair<K,V>>
    pub map: GUID,      // IMap<K,V>
    pub map_view: GUID, // IMapView<K,V>
    pub kvp: GUID,      // IKeyValuePair<K,V>
    pub iterator: GUID, // IIterator<IKeyValuePair<K,V>>
}

// ======================================================================
// COM vtable layouts
// ======================================================================

/// IIterable<IKeyValuePair<K,V>> vtable
#[repr(C)]
struct IterableVtbl {
    base: IInspectableVtbl,
    first: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

/// IMap<K,V> vtable: IInspectable + 7 methods
#[repr(C)]
struct MapVtbl {
    base: IInspectableVtbl,
    lookup: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
    get_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    has_key: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut bool) -> HRESULT,
    get_view: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    insert: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut bool) -> HRESULT,
    remove: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    clear: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}

/// IMapView<K,V> vtable: IInspectable + 4 methods
#[repr(C)]
struct MapViewVtbl {
    base: IInspectableVtbl,
    lookup: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
    get_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    has_key: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut bool) -> HRESULT,
    split: unsafe extern "system" fn(*mut c_void, *mut *mut c_void, *mut *mut c_void) -> HRESULT,
}

/// IKeyValuePair<K,V> vtable: IInspectable + 2 methods
#[repr(C)]
struct KeyValuePairVtbl {
    base: IInspectableVtbl,
    get_key: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    get_value: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

// ======================================================================
// Key comparison helper
// ======================================================================

unsafe fn find_key_index(
    entries: &[(usize, usize)],
    key: *mut c_void,
    storage: CollectionStorage,
) -> Option<usize> {
    let key = key as usize;
    if !storage.is_hstring
        && !storage.is_value_type
        && let Some(search) = boxed_hstring(key)
    {
        return entries
            .iter()
            .position(|(stored, _)| boxed_hstring(*stored).is_some_and(|value| value == search));
    }
    entries
        .iter()
        .position(|(stored, _)| stored_items_equal(storage, *stored, key))
}

unsafe fn boxed_hstring(raw: usize) -> Option<windows_core::HSTRING> {
    if raw == 0 {
        return None;
    }
    let ptr = raw as *mut c_void;
    let object = IUnknown::from_raw_borrowed(&ptr)?;
    let value: windows::Foundation::IPropertyValue = object.cast().ok()?;
    value.GetString().ok()
}

const fn object_storage() -> CollectionStorage {
    CollectionStorage {
        is_value_type: false,
        is_hstring: false,
        elem_size: std::mem::size_of::<*mut c_void>(),
    }
}

// ======================================================================
// SingleThreadedMap
// ======================================================================

#[repr(C)]
struct SingleThreadedMap {
    vtable_iterable: *const IterableVtbl,
    vtable_map: *const MapVtbl,
    ref_count: windows_core::imp::RefCount,
    entries: Mutex<Vec<(usize, usize)>>,
    key_storage: CollectionStorage,
    value_storage: CollectionStorage,
    iids: MapIids,
}

unsafe impl Send for SingleThreadedMap {}
unsafe impl Sync for SingleThreadedMap {}

impl SingleThreadedMap {
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

    const MAP_VTBL: MapVtbl = MapVtbl {
        base: IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::qi_map,
                AddRef: Self::add_ref_map,
                Release: Self::release_map,
            },
            get_iids: Self::get_iids_map,
            get_runtime_class_name: Self::get_runtime_class_name_map,
            get_trust_level: Self::get_trust_level_map,
        },
        lookup: Self::lookup,
        get_size: Self::get_size,
        has_key: Self::has_key,
        get_view: Self::get_view,
        insert: Self::insert,
        remove: Self::remove,
        clear: Self::clear,
    };

    dual_vtable_com!(iterable, map, map);
    inspectable_stubs!(iterable, map);

    // -- IIterable<IKeyValuePair<K,V>> --

    unsafe extern "system" fn first(this: *mut c_void, result: *mut *mut c_void) -> HRESULT {
        let me = Self::from_iterable_ptr(this);
        let entries = lock_or!(me.entries, E_FAIL);
        let kvp_items: Vec<usize> = entries
            .iter()
            .map(|(k, v)| {
                SingleThreadedKeyValuePair::create(
                    clone_stored_item(me.key_storage, *k),
                    clone_stored_item(me.value_storage, *v),
                    me.key_storage,
                    me.value_storage,
                    me.iids.kvp,
                )
                .into_raw() as usize
            })
            .collect();
        let iter = SingleThreadedIterator::create(kvp_items, object_storage(), me.iids.iterator);
        *result = iter.into_raw();
        S_OK
    }

    // -- IMap<K,V> --

    unsafe extern "system" fn lookup(
        this: *mut c_void,
        key: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        let me = Self::from_map_ptr(this);
        let entries = lock_or!(me.entries, E_FAIL);
        match find_key_index(&entries, key, me.key_storage) {
            Some(i) => {
                write_item_out(me.value_storage, entries[i].1, result);
                S_OK
            }
            None => E_BOUNDS,
        }
    }

    unsafe extern "system" fn get_size(this: *mut c_void, result: *mut u32) -> HRESULT {
        let me = Self::from_map_ptr(this);
        *result = lock_or!(me.entries, E_FAIL).len() as u32;
        S_OK
    }

    unsafe extern "system" fn has_key(
        this: *mut c_void,
        key: *mut c_void,
        result: *mut bool,
    ) -> HRESULT {
        let me = Self::from_map_ptr(this);
        let entries = lock_or!(me.entries, E_FAIL);
        *result = find_key_index(&entries, key, me.key_storage).is_some();
        S_OK
    }

    unsafe extern "system" fn get_view(this: *mut c_void, result: *mut *mut c_void) -> HRESULT {
        let me = Self::from_map_ptr(this);
        let snapshot = lock_or!(me.entries, E_FAIL)
            .iter()
            .map(|(key, value)| {
                (
                    clone_stored_item(me.key_storage, *key),
                    clone_stored_item(me.value_storage, *value),
                )
            })
            .collect();
        let view = SingleThreadedMapView::create(
            snapshot,
            me.key_storage,
            me.value_storage,
            me.iids.clone(),
        );
        // WinRT ABI: get_view must return an IMapView pointer (second vtable),
        // not the identity/IIterable pointer (first vtable).
        let identity = view.into_raw();
        *result = (identity as *const *const c_void).add(1) as *mut c_void;
        S_OK
    }

    unsafe extern "system" fn insert(
        this: *mut c_void,
        key: *mut c_void,
        value: *mut c_void,
        replaced: *mut bool,
    ) -> HRESULT {
        let me = Self::from_map_ptr(this);
        let mut entries = lock_or!(me.entries, E_FAIL);
        match find_key_index(&entries, key, me.key_storage) {
            Some(i) => {
                let old_value = entries[i].1;
                entries[i].1 = store_abi_item(me.value_storage, value);
                release_stored_item(me.value_storage, old_value);
                *replaced = true;
            }
            None => {
                entries.push((
                    store_abi_item(me.key_storage, key),
                    store_abi_item(me.value_storage, value),
                ));
                *replaced = false;
            }
        }
        S_OK
    }

    unsafe extern "system" fn remove(this: *mut c_void, key: *mut c_void) -> HRESULT {
        let me = Self::from_map_ptr(this);
        let mut entries = lock_or!(me.entries, E_FAIL);
        match find_key_index(&entries, key, me.key_storage) {
            Some(i) => {
                let (key, value) = entries.remove(i);
                release_stored_item(me.key_storage, key);
                release_stored_item(me.value_storage, value);
                S_OK
            }
            None => E_BOUNDS,
        }
    }

    unsafe extern "system" fn clear(this: *mut c_void) -> HRESULT {
        let me = Self::from_map_ptr(this);
        let entries: Vec<_> = lock_or!(me.entries, E_FAIL).drain(..).collect();
        for (key, value) in entries {
            release_stored_item(me.key_storage, key);
            release_stored_item(me.value_storage, value);
        }
        S_OK
    }
}

impl Drop for SingleThreadedMap {
    fn drop(&mut self) {
        if let Ok(entries) = self.entries.lock() {
            for &(key, value) in entries.iter() {
                unsafe {
                    release_stored_item(self.key_storage, key);
                    release_stored_item(self.value_storage, value);
                }
            }
        }
    }
}

// ======================================================================
// SingleThreadedMapView
// ======================================================================

#[repr(C)]
struct SingleThreadedMapView {
    vtable_iterable: *const IterableVtbl,
    vtable_view: *const MapViewVtbl,
    ref_count: windows_core::imp::RefCount,
    entries: Vec<(usize, usize)>,
    key_storage: CollectionStorage,
    value_storage: CollectionStorage,
    iids: MapIids,
}

unsafe impl Send for SingleThreadedMapView {}
unsafe impl Sync for SingleThreadedMapView {}

impl SingleThreadedMapView {
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

    const VIEW_VTBL: MapViewVtbl = MapViewVtbl {
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
        lookup: Self::lookup,
        get_size: Self::get_size,
        has_key: Self::has_key,
        split: Self::split,
    };

    fn create(
        entries: Vec<(usize, usize)>,
        key_storage: CollectionStorage,
        value_storage: CollectionStorage,
        iids: MapIids,
    ) -> IUnknown {
        let view = Box::new(Self {
            vtable_iterable: &Self::ITERABLE_VTBL,
            vtable_view: &Self::VIEW_VTBL,
            ref_count: windows_core::imp::RefCount::new(1),
            entries,
            key_storage,
            value_storage,
            iids,
        });
        unsafe { IUnknown::from_raw(Box::into_raw(view) as *mut c_void) }
    }

    dual_vtable_com!(iterable, view, map_view);
    inspectable_stubs!(iterable, view);

    // -- IIterable --

    unsafe extern "system" fn first(this: *mut c_void, result: *mut *mut c_void) -> HRESULT {
        let me = Self::from_iterable_ptr(this);
        let kvp_items: Vec<usize> = me
            .entries
            .iter()
            .map(|(k, v)| {
                SingleThreadedKeyValuePair::create(
                    clone_stored_item(me.key_storage, *k),
                    clone_stored_item(me.value_storage, *v),
                    me.key_storage,
                    me.value_storage,
                    me.iids.kvp,
                )
                .into_raw() as usize
            })
            .collect();
        let iter = SingleThreadedIterator::create(kvp_items, object_storage(), me.iids.iterator);
        *result = iter.into_raw();
        S_OK
    }

    // -- IMapView --

    unsafe extern "system" fn lookup(
        this: *mut c_void,
        key: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        let me = Self::from_view_ptr(this);
        match find_key_index(&me.entries, key, me.key_storage) {
            Some(i) => {
                write_item_out(me.value_storage, me.entries[i].1, result);
                S_OK
            }
            None => E_BOUNDS,
        }
    }

    unsafe extern "system" fn get_size(this: *mut c_void, result: *mut u32) -> HRESULT {
        let me = Self::from_view_ptr(this);
        *result = me.entries.len() as u32;
        S_OK
    }

    unsafe extern "system" fn has_key(
        this: *mut c_void,
        key: *mut c_void,
        result: *mut bool,
    ) -> HRESULT {
        let me = Self::from_view_ptr(this);
        *result = find_key_index(&me.entries, key, me.key_storage).is_some();
        S_OK
    }

    unsafe extern "system" fn split(
        _this: *mut c_void,
        first: *mut *mut c_void,
        second: *mut *mut c_void,
    ) -> HRESULT {
        // Split is optional; return empty halves
        unsafe {
            *first = std::ptr::null_mut();
            *second = std::ptr::null_mut();
        }
        S_OK
    }
}

impl Drop for SingleThreadedMapView {
    fn drop(&mut self) {
        for &(key, value) in &self.entries {
            unsafe {
                release_stored_item(self.key_storage, key);
                release_stored_item(self.value_storage, value);
            }
        }
    }
}

// ======================================================================
// SingleThreadedKeyValuePair
// ======================================================================

#[repr(C)]
struct SingleThreadedKeyValuePair {
    vtable: *const KeyValuePairVtbl,
    ref_count: windows_core::imp::RefCount,
    key: usize,
    value: usize,
    key_storage: CollectionStorage,
    value_storage: CollectionStorage,
    iid_kvp: GUID,
}

unsafe impl Send for SingleThreadedKeyValuePair {}
unsafe impl Sync for SingleThreadedKeyValuePair {}

impl SingleThreadedKeyValuePair {
    const VTBL: KeyValuePairVtbl = KeyValuePairVtbl {
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
        get_key: Self::get_key,
        get_value: Self::get_value,
    };

    fn create(
        key: usize,
        value: usize,
        key_storage: CollectionStorage,
        value_storage: CollectionStorage,
        iid_kvp: GUID,
    ) -> IUnknown {
        let kvp = Box::new(Self {
            vtable: &Self::VTBL,
            ref_count: windows_core::imp::RefCount::new(1),
            key,
            value,
            key_storage,
            value_storage,
            iid_kvp,
        });
        unsafe { IUnknown::from_raw(Box::into_raw(kvp) as *mut c_void) }
    }

    single_vtable_com!(|me: &Self| me.iid_kvp);
    inspectable_stubs!(stub);

    unsafe extern "system" fn get_key(this: *mut c_void, result: *mut *mut c_void) -> HRESULT {
        let me = &*(this as *const Self);
        write_item_out(me.key_storage, me.key, result);
        S_OK
    }

    unsafe extern "system" fn get_value(this: *mut c_void, result: *mut *mut c_void) -> HRESULT {
        let me = &*(this as *const Self);
        write_item_out(me.value_storage, me.value, result);
        S_OK
    }
}

impl Drop for SingleThreadedKeyValuePair {
    fn drop(&mut self) {
        unsafe {
            release_stored_item(self.key_storage, self.key);
            release_stored_item(self.value_storage, self.value);
        }
    }
}

// ======================================================================
// Public API
// ======================================================================

/// Create an IMap<K,V> COM object from key-value pairs.
///
/// The returned IUnknown supports QI for IMap<K,V>, IIterable<IKeyValuePair<K,V>>,
/// IMapView<K,V> (via GetView), IKeyValuePair<K,V> (via iteration), and IIterator (via First).
pub fn create_map(entries: Vec<(IUnknown, IUnknown)>, iids: MapIids) -> IUnknown {
    let entries = entries
        .into_iter()
        .map(|(key, value)| (key.into_raw() as usize, value.into_raw() as usize))
        .collect();
    let storage = CollectionStorage {
        is_value_type: false,
        is_hstring: false,
        elem_size: std::mem::size_of::<*mut c_void>(),
    };
    new_map(entries, storage, storage, iids)
}

pub fn create_map_from_values(
    entries: &[(crate::WinRTValue, crate::WinRTValue)],
    key_type: &crate::TypeHandle,
    value_type: &crate::TypeHandle,
    iids: MapIids,
) -> crate::Result<IUnknown> {
    let key_storage = collection_storage(key_type)?;
    let value_storage = collection_storage(value_type)?;
    if key_storage.is_large_value_type() {
        return Err(crate::Error::UnsupportedCollectionElement(key_type.kind()));
    }
    if value_storage.is_large_value_type() {
        return Err(crate::Error::UnsupportedCollectionElement(
            value_type.kind(),
        ));
    }
    for (key, value) in entries {
        validate_collection_item(key, key_storage)?;
        validate_collection_item(value, value_storage)?;
    }
    let entries = entries
        .iter()
        .map(|(key, value)| {
            (
                pack_validated_collection_item(key, key_storage),
                pack_validated_collection_item(value, value_storage),
            )
        })
        .collect();
    Ok(new_map(entries, key_storage, value_storage, iids))
}

fn new_map(
    entries: Vec<(usize, usize)>,
    key_storage: CollectionStorage,
    value_storage: CollectionStorage,
    iids: MapIids,
) -> IUnknown {
    let map = Box::new(SingleThreadedMap {
        vtable_iterable: &SingleThreadedMap::ITERABLE_VTBL,
        vtable_map: &SingleThreadedMap::MAP_VTBL,
        ref_count: windows_core::imp::RefCount::new(1),
        entries: Mutex::new(entries),
        key_storage,
        value_storage,
        iids,
    });
    unsafe { IUnknown::from_raw(Box::into_raw(map) as *mut c_void) }
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
    fn test_map_basic_operations() {
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        let table = MetadataTable::new();
        let iids = table.map_iids(&table.hstring(), &table.object());

        // Create empty map
        let map = create_map(Vec::new(), iids.clone());

        // QI to IMap
        let mut map_ptr = std::ptr::null_mut();
        unsafe { map.query(&iids.map, &mut map_ptr) }.ok().unwrap();
        assert!(!map_ptr.is_null());

        // Get size (should be 0)
        let vtbl = unsafe { *(map_ptr as *const *const MapVtbl) };
        let mut size: u32 = 0;
        unsafe { ((*vtbl).get_size)(map_ptr, &mut size) };
        assert_eq!(size, 0);

        let _ = unsafe { IUnknown::from_raw(map_ptr) };
    }

    #[test]
    fn test_map_iid_computation() {
        let table = MetadataTable::new();
        let iids = table.map_iids(&table.hstring(), &table.object());

        // All IIDs should be non-zero
        assert_ne!(iids.iterable, GUID::zeroed());
        assert_ne!(iids.map, GUID::zeroed());
        assert_ne!(iids.map_view, GUID::zeroed());
        assert_ne!(iids.kvp, GUID::zeroed());
        assert_ne!(iids.iterator, GUID::zeroed());

        // All should be different
        assert_ne!(iids.map, iids.map_view);
        assert_ne!(iids.map, iids.kvp);
        assert_ne!(iids.kvp, iids.iterator);
    }

    #[test]
    fn test_object_keys_compare_boxed_strings_by_value() {
        let first =
            windows::Foundation::PropertyValue::CreateString(windows_core::h!("same")).unwrap();
        let second =
            windows::Foundation::PropertyValue::CreateString(windows_core::h!("same")).unwrap();
        let stored: IUnknown = first.cast().unwrap();
        let search: IUnknown = second.cast().unwrap();
        let stored_raw = stored.into_raw() as usize;
        let entries = vec![(stored_raw, 0)];

        assert_eq!(
            unsafe { find_key_index(&entries, search.as_raw(), object_storage()) },
            Some(0)
        );

        unsafe { release_stored_item(object_storage(), stored_raw) };
    }

    #[test]
    fn test_key_value_pair() {
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        let table = MetadataTable::new();
        let iids = table.map_iids(&table.hstring(), &table.object());
        let key_storage = collection_storage(&table.hstring()).unwrap();
        let value_storage = collection_storage(&table.object()).unwrap();

        let uri =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com")).unwrap();
        let key = windows_core::HSTRING::from("mykey");
        let key_raw: *mut c_void = unsafe { std::mem::transmute(key) };
        let val: IUnknown = uri.cast().unwrap();

        let kvp = SingleThreadedKeyValuePair::create(
            key_raw as usize,
            val.into_raw() as usize,
            key_storage,
            value_storage,
            iids.kvp,
        );

        // QI for IKeyValuePair
        let mut kvp_ptr = std::ptr::null_mut();
        unsafe { kvp.query(&iids.kvp, &mut kvp_ptr) }.ok().unwrap();
        assert!(!kvp_ptr.is_null());

        // Get key and value
        let vtbl = unsafe { *(kvp_ptr as *const *const KeyValuePairVtbl) };
        let mut key_ptr: *mut c_void = std::ptr::null_mut();
        let mut val_ptr: *mut c_void = std::ptr::null_mut();
        unsafe { ((*vtbl).get_key)(kvp_ptr, &mut key_ptr) };
        unsafe { ((*vtbl).get_value)(kvp_ptr, &mut val_ptr) };
        assert!(!key_ptr.is_null());
        assert!(!val_ptr.is_null());

        let key: windows_core::HSTRING = unsafe { std::mem::transmute(key_ptr) };
        assert_eq!(key, "mykey");
        let _ = unsafe { IUnknown::from_raw(val_ptr) };
        let _ = unsafe { IUnknown::from_raw(kvp_ptr) };
    }
}
