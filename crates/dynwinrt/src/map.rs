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

use crate::com_helpers::{E_BOUNDS, E_FAIL, E_POINTER, IInspectableVtbl, S_OK};
#[allow(unused_imports)]
use crate::com_helpers::{dual_vtable_com, inspectable_stubs, lock_or, single_vtable_com};
use crate::vector::SingleThreadedIterator;

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

/// Compare two WinRT keys by their raw COM pointer identity.
/// For HSTRING keys, this compares by pointer — callers must ensure
/// they pass the same IUnknown-boxed HSTRING. For most practical uses
/// (IMap<String, Object> in WinRT), the runtime boxes each HSTRING
/// into an IReference<String> and compares by string content.
///
/// For our implementation we compare by raw pointer identity, which
/// works correctly when the same IUnknown objects are used as keys.
unsafe fn keys_equal(a: *mut c_void, b: *mut c_void) -> bool {
    a == b
}

/// Specialized string key comparison: if the key is an HSTRING boxed as IPropertyValue,
/// we try to compare by QI to IPropertyValue and reading the string.
/// Falls back to pointer identity.
unsafe fn find_key_index(entries: &[(IUnknown, IUnknown)], key: *mut c_void) -> Option<usize> {
    // Try HSTRING comparison via IReference<String>
    // IPropertyValue IID: {4BD682DD-7554-40E9-9A9B-82654EDE7E62}
    let ipv_iid = GUID::from_u128(0x4BD682DD_7554_40E9_9A9B_82654EDE7E62);

    // Try to get string from search key
    let search_str = unsafe { get_hstring_from_inspectable(key, &ipv_iid) };

    if let Some(ref search) = search_str {
        // Compare as strings
        for (i, (k, _)) in entries.iter().enumerate() {
            if let Some(ref entry_str) =
                unsafe { get_hstring_from_inspectable(k.as_raw(), &ipv_iid) }
            {
                if search == entry_str {
                    return Some(i);
                }
            }
        }
        return None;
    }

    // Fallback: pointer identity
    for (i, (k, _)) in entries.iter().enumerate() {
        if unsafe { keys_equal(k.as_raw(), key) } {
            return Some(i);
        }
    }
    None
}

/// Try to read an HSTRING from an IPropertyValue object.
unsafe fn get_hstring_from_inspectable(ptr: *mut c_void, ipv_iid: &GUID) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut ipv_ptr = std::ptr::null_mut();
    let obj = unsafe { IUnknown::from_raw_borrowed(&ptr) }?;
    if obj.query(ipv_iid, &mut ipv_ptr).is_err() {
        return None;
    }
    let ipv = unsafe { IUnknown::from_raw(ipv_ptr) };
    // IPropertyValue vtable[6] = get_Type, [7] = get_IsNumericScalar, [8] = GetUInt8, ..., [18] = GetString
    // Actually, let's use the Windows crate for this
    let pv: Result<windows::Foundation::IPropertyValue, _> = ipv.cast();
    match pv {
        Ok(pv) => match pv.GetString() {
            Ok(s) => Some(s.to_string()),
            Err(_) => None,
        },
        Err(_) => None,
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
    entries: Mutex<Vec<(IUnknown, IUnknown)>>,
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
                SingleThreadedKeyValuePair::create(k.clone(), v.clone(), me.iids.kvp).into_raw()
                    as usize
            })
            .collect();
        let iter = SingleThreadedIterator::create(
            kvp_items,
            false,
            std::mem::size_of::<*mut c_void>(),
            me.iids.iterator,
        );
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
        match find_key_index(&entries, key) {
            Some(i) => {
                *result = entries[i].1.clone().into_raw();
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
        *result = find_key_index(&entries, key).is_some();
        S_OK
    }

    unsafe extern "system" fn get_view(this: *mut c_void, result: *mut *mut c_void) -> HRESULT {
        let me = Self::from_map_ptr(this);
        let snapshot = lock_or!(me.entries, E_FAIL).clone();
        let view = SingleThreadedMapView::create(snapshot, me.iids.clone());
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
        let new_key = match IUnknown::from_raw_borrowed(&key) {
            Some(k) => k.clone(),
            None => return E_POINTER,
        };
        let new_val = match IUnknown::from_raw_borrowed(&value) {
            Some(v) => v.clone(),
            None => return E_POINTER,
        };
        match find_key_index(&entries, key) {
            Some(i) => {
                entries[i].1 = new_val;
                *replaced = true;
            }
            None => {
                entries.push((new_key, new_val));
                *replaced = false;
            }
        }
        S_OK
    }

    unsafe extern "system" fn remove(this: *mut c_void, key: *mut c_void) -> HRESULT {
        let me = Self::from_map_ptr(this);
        let mut entries = lock_or!(me.entries, E_FAIL);
        match find_key_index(&entries, key) {
            Some(i) => {
                entries.remove(i);
                S_OK
            }
            None => E_BOUNDS,
        }
    }

    unsafe extern "system" fn clear(this: *mut c_void) -> HRESULT {
        let me = Self::from_map_ptr(this);
        lock_or!(me.entries, E_FAIL).clear();
        S_OK
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
    entries: Vec<(IUnknown, IUnknown)>,
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

    fn create(entries: Vec<(IUnknown, IUnknown)>, iids: MapIids) -> IUnknown {
        let view = Box::new(Self {
            vtable_iterable: &Self::ITERABLE_VTBL,
            vtable_view: &Self::VIEW_VTBL,
            ref_count: windows_core::imp::RefCount::new(1),
            entries,
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
                SingleThreadedKeyValuePair::create(k.clone(), v.clone(), me.iids.kvp).into_raw()
                    as usize
            })
            .collect();
        let iter = SingleThreadedIterator::create(
            kvp_items,
            false,
            std::mem::size_of::<*mut c_void>(),
            me.iids.iterator,
        );
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
        match find_key_index(&me.entries, key) {
            Some(i) => {
                *result = me.entries[i].1.clone().into_raw();
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
        *result = find_key_index(&me.entries, key).is_some();
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

// ======================================================================
// SingleThreadedKeyValuePair
// ======================================================================

#[repr(C)]
struct SingleThreadedKeyValuePair {
    vtable: *const KeyValuePairVtbl,
    ref_count: windows_core::imp::RefCount,
    key: IUnknown,
    value: IUnknown,
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

    fn create(key: IUnknown, value: IUnknown, iid_kvp: GUID) -> IUnknown {
        let kvp = Box::new(Self {
            vtable: &Self::VTBL,
            ref_count: windows_core::imp::RefCount::new(1),
            key,
            value,
            iid_kvp,
        });
        unsafe { IUnknown::from_raw(Box::into_raw(kvp) as *mut c_void) }
    }

    single_vtable_com!(|me: &Self| me.iid_kvp);
    inspectable_stubs!(stub);

    unsafe extern "system" fn get_key(this: *mut c_void, result: *mut *mut c_void) -> HRESULT {
        let me = &*(this as *const Self);
        *result = me.key.clone().into_raw();
        S_OK
    }

    unsafe extern "system" fn get_value(this: *mut c_void, result: *mut *mut c_void) -> HRESULT {
        let me = &*(this as *const Self);
        *result = me.value.clone().into_raw();
        S_OK
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
    let map = Box::new(SingleThreadedMap {
        vtable_iterable: &SingleThreadedMap::ITERABLE_VTBL,
        vtable_map: &SingleThreadedMap::MAP_VTBL,
        ref_count: windows_core::imp::RefCount::new(1),
        entries: Mutex::new(entries),
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
    fn test_key_value_pair() {
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        let table = MetadataTable::new();
        let iids = table.map_iids(&table.hstring(), &table.object());

        let uri =
            windows::Foundation::Uri::CreateUri(windows_core::h!("https://example.com")).unwrap();
        let key: IUnknown =
            windows::Foundation::PropertyValue::CreateString(windows_core::h!("mykey"))
                .unwrap()
                .cast()
                .unwrap();
        let val: IUnknown = uri.cast().unwrap();

        let kvp = SingleThreadedKeyValuePair::create(key, val, iids.kvp);

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

        let _ = unsafe { IUnknown::from_raw(key_ptr) };
        let _ = unsafe { IUnknown::from_raw(val_ptr) };
        let _ = unsafe { IUnknown::from_raw(kvp_ptr) };
    }
}
