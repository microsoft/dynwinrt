// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use crate::{MetadataTable, TypeHandle, WinRTValue};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use windows::Foundation::{IClosable, IStringable, PropertyValue};
use windows_collections::IMap;
use windows_core::{HSTRING, IInspectable};

fn checked_map(
    entries: &[(WinRTValue, WinRTValue)],
    key: &TypeHandle,
    value: &TypeHandle,
) -> crate::Result<IUnknown> {
    create_map_from_values(entries, key, value, key.table().map_iids(key, value))
}

fn text(value: &str) -> WinRTValue {
    WinRTValue::HString(HSTRING::from(value))
}

#[test]
fn duplicate_constructor_strings_match_insert_and_preserve_order() {
    let table = MetadataTable::new();
    let typ = table.hstring();
    for (keys, expected) in [
        (vec![], vec![]),
        (vec!["A", "B"], vec![("A", "0"), ("B", "1")]),
        (vec!["same", "same"], vec![("same", "1")]),
        (vec!["A", "B", "A"], vec![("A", "2"), ("B", "1")]),
        (
            vec!["same", "Same", "same\0tail", "", "same\0tail", "", "same"],
            vec![("same", "6"), ("Same", "1"), ("same\0tail", "4"), ("", "5")],
        ),
    ] {
        let entries: Vec<_> = keys
            .iter()
            .enumerate()
            .map(|(index, key)| (text(key), text(&index.to_string())))
            .collect();
        let map: IMap<HSTRING, HSTRING> =
            checked_map(&entries, &typ, &typ).unwrap().cast().unwrap();
        let control: IMap<HSTRING, HSTRING> = checked_map(&[], &typ, &typ).unwrap().cast().unwrap();
        for (index, key) in keys.iter().enumerate() {
            assert_eq!(
                control
                    .Insert(&HSTRING::from(*key), &HSTRING::from(index.to_string()))
                    .unwrap(),
                keys[..index].contains(key)
            );
        }
        assert_eq!(map.Size().unwrap(), expected.len() as u32);
        assert_eq!(map.Size().unwrap(), control.Size().unwrap());
        let view = map.GetView().unwrap();
        let iterator = map.First().unwrap();
        let control_iterator = control.First().unwrap();
        for (key, value) in &expected {
            let key = HSTRING::from(*key);
            let value = HSTRING::from(*value);
            assert_eq!(map.Lookup(&key).unwrap(), value);
            assert_eq!(control.Lookup(&key).unwrap(), value);
            assert_eq!(view.Lookup(&key).unwrap(), value);
            for iterator in [&iterator, &control_iterator] {
                let pair = iterator.Current().unwrap();
                assert_eq!(pair.Key().unwrap(), key);
                assert_eq!(pair.Value().unwrap(), value);
                iterator.MoveNext().unwrap();
            }
            map.Remove(&key).unwrap();
            assert!(!map.HasKey(&key).unwrap());
            assert_eq!(map.Lookup(&key).unwrap_err().code(), E_BOUNDS);
        }
        assert!(!iterator.HasCurrent().unwrap());
        assert!(!control_iterator.HasCurrent().unwrap());
        assert_eq!(map.Size().unwrap(), 0);
        assert_eq!(view.Size().unwrap(), expected.len() as u32);
        assert_eq!(
            view.First().unwrap().HasCurrent().unwrap(),
            !expected.is_empty()
        );
    }
}

#[test]
fn duplicate_constructor_normalizes_scalar_and_enum_aliases() {
    let table = MetadataTable::new();
    macro_rules! check {
        ($native:ty, $typ:expr, $first:expr, $alias:expr, $key:expr) => {{
            let typ = $typ;
            let map: IMap<$native, i32> = checked_map(
                &[($first, WinRTValue::I32(1)), ($alias, WinRTValue::I32(2))],
                &typ,
                &table.i32_type(),
            )
            .unwrap()
            .cast()
            .unwrap();
            assert_eq!(map.Size().unwrap(), 1);
            assert_eq!(map.Lookup($key).unwrap(), 2);
            assert!(map.Insert($key, 3).unwrap());
            assert_eq!(map.Size().unwrap(), 1);
            assert_eq!(map.Lookup($key).unwrap(), 3);
            map.Remove($key).unwrap();
            assert!(!map.HasKey($key).unwrap());
        }};
    }
    check!(
        bool,
        table.bool_type(),
        WinRTValue::Bool(true),
        WinRTValue::Bool(true),
        true
    );
    check!(
        i8,
        table.i8_type(),
        WinRTValue::I8(-1),
        WinRTValue::I32(-1),
        -1
    );
    check!(
        u8,
        table.u8_type(),
        WinRTValue::U8(255),
        WinRTValue::I32(255),
        255
    );
    check!(
        i16,
        table.i16_type(),
        WinRTValue::I16(-1),
        WinRTValue::I16(-1),
        -1
    );
    check!(
        u16,
        table.u16_type(),
        WinRTValue::U16(65535),
        WinRTValue::U16(65535),
        65535
    );
    check!(
        i32,
        table.i32_type(),
        WinRTValue::I32(-1),
        WinRTValue::I32(-1),
        -1
    );
    check!(
        u32,
        table.u32_type(),
        WinRTValue::U32(u32::MAX),
        WinRTValue::U32(u32::MAX),
        u32::MAX
    );
    check!(
        i32,
        table.hresult(),
        WinRTValue::HResult(HRESULT(-1)),
        WinRTValue::I32(-1),
        -1
    );
    if cfg!(target_pointer_width = "64") {
        check!(
            i64,
            table.i64_type(),
            WinRTValue::I64(i64::MIN),
            WinRTValue::I64(i64::MIN),
            i64::MIN
        );
        check!(
            u64,
            table.u64_type(),
            WinRTValue::U64(u64::MAX),
            WinRTValue::U64(u64::MAX),
            u64::MAX
        );
    }
    let typ = table.enum_type("Windows.Foundation.AsyncStatus", vec![]);
    check!(
        windows_future::AsyncStatus,
        typ.clone(),
        WinRTValue::Enum {
            value: 1,
            type_handle: typ.clone()
        },
        WinRTValue::I32(1),
        windows_future::AsyncStatus(1)
    );
}

#[windows_core::implement(IStringable, IClosable)]
struct Tracked(Arc<AtomicUsize>);

impl windows::Foundation::IStringable_Impl for Tracked_Impl {
    fn ToString(&self) -> windows_core::Result<HSTRING> {
        Ok(HSTRING::from("retained"))
    }
}

impl windows::Foundation::IClosable_Impl for Tracked_Impl {
    fn Close(&self) -> windows_core::Result<()> {
        Ok(())
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn tracked(dropped: &Arc<AtomicUsize>) -> WinRTValue {
    let source: IClosable = Tracked(dropped.clone()).into();
    // Preparation must adjust this IClosable pointer to the declared interface.
    WinRTValue::Object(unsafe { IUnknown::from_raw(source.into_raw()) })
}

fn stringable(value: &WinRTValue) -> IStringable {
    let WinRTValue::Object(object) = value else {
        panic!("expected an object");
    };
    object.cast().unwrap()
}

#[test]
fn duplicate_constructor_retains_typed_keys_and_only_the_latest_value() {
    let table = MetadataTable::new();
    let typ = table.interface(IStringable::IID);
    let key_drops = Arc::new(AtomicUsize::new(0));
    let old_drops = Arc::new(AtomicUsize::new(0));
    let latest_drops = Arc::new(AtomicUsize::new(0));
    let key = tracked(&key_drops);
    let old = tracked(&old_drops);
    let latest = tracked(&latest_drops);
    let expected_key = stringable(&key);
    let alias = WinRTValue::Object(unsafe { IUnknown::from_raw(expected_key.clone().into_raw()) });
    let entries = [(key.clone(), old.clone()), (alias, latest.clone())];
    let map: IMap<IStringable, IStringable> =
        checked_map(&entries, &typ, &typ).unwrap().cast().unwrap();
    assert_eq!(map.Size().unwrap(), 1);
    assert_eq!(map.Lookup(&expected_key).unwrap(), stringable(&latest));
    drop(entries);
    assert_eq!(old_drops.load(Ordering::SeqCst), 0);
    assert_eq!(
        stringable(&old).ToString().unwrap(),
        HSTRING::from("retained")
    );
    drop(old);
    assert_eq!(old_drops.load(Ordering::SeqCst), 1);
    drop(key);
    drop(latest);
    let view = map.GetView().unwrap();
    let iterator = map.First().unwrap();
    map.Remove(&expected_key).unwrap();
    assert_eq!(map.Size().unwrap(), 0);
    assert!(!map.HasKey(&expected_key).unwrap());
    drop(map);
    assert_eq!(view.Size().unwrap(), 1);
    let pair = iterator.Current().unwrap();
    assert_eq!(pair.Key().unwrap(), expected_key);
    assert_eq!(
        pair.Value().unwrap().ToString().unwrap(),
        HSTRING::from("retained")
    );
    assert!(!iterator.MoveNext().unwrap());
    drop(expected_key);
    drop(view);
    drop(iterator);
    assert_eq!(key_drops.load(Ordering::SeqCst), 0);
    assert_eq!(latest_drops.load(Ordering::SeqCst), 0);
    drop(pair);
    assert_eq!(key_drops.load(Ordering::SeqCst), 1);
    assert_eq!(latest_drops.load(Ordering::SeqCst), 1);
}

#[test]
fn duplicate_constructor_preserves_first_boxed_string_key_identity() {
    crate::test_apartment::initialize_mta();
    let table = MetadataTable::new();
    let first = PropertyValue::CreateString(windows_core::h!("same")).unwrap();
    let second = PropertyValue::CreateString(windows_core::h!("same")).unwrap();
    assert_ne!(first, second);
    let map: IMap<IInspectable, i32> = checked_map(
        &[
            (
                WinRTValue::Object(first.cast().unwrap()),
                WinRTValue::I32(1),
            ),
            (
                WinRTValue::Object(second.cast().unwrap()),
                WinRTValue::I32(2),
            ),
        ],
        &table.object(),
        &table.i32_type(),
    )
    .unwrap()
    .cast()
    .unwrap();
    let control: IMap<IInspectable, i32> = checked_map(&[], &table.object(), &table.i32_type())
        .unwrap()
        .cast()
        .unwrap();
    assert!(!control.Insert(&first, 1).unwrap());
    assert!(control.Insert(&second, 2).unwrap());
    for map in [map, control] {
        assert_eq!(map.Size().unwrap(), 1);
        assert_eq!(map.Lookup(&second).unwrap(), 2);
        assert_eq!(
            map.First().unwrap().Current().unwrap().Key().unwrap(),
            first
        );
        map.Remove(&second).unwrap();
        assert!(!map.HasKey(&first).unwrap());
    }
}

#[test]
fn duplicate_constructor_coalesces_managed_null_keys_and_values() {
    let table = MetadataTable::new();
    let typ = table.interface(IStringable::IID);
    let dropped = Arc::new(AtomicUsize::new(0));
    let input = tracked(&dropped);
    let map: IMap<IStringable, IStringable> = checked_map(
        &[
            (WinRTValue::Null, input.clone()),
            (WinRTValue::Null, WinRTValue::Null),
        ],
        &typ,
        &typ,
    )
    .unwrap()
    .cast()
    .unwrap();
    assert_eq!(map.Size().unwrap(), 1);
    drop(input);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(map.HasKey(None).unwrap());
    assert!(map.Insert(None, None).unwrap());
    map.Remove(None).unwrap();
    assert!(!map.HasKey(None).unwrap());
}

#[test]
fn duplicate_constructor_validates_every_value_and_releases_failed_staging() {
    crate::test_apartment::initialize_mta();
    let table = MetadataTable::new();
    let typ = table.interface(IStringable::IID);
    for wrong_iid in [false, true] {
        let key_drops = Arc::new(AtomicUsize::new(0));
        let value_drops = Arc::new(AtomicUsize::new(0));
        let key = tracked(&key_drops);
        let value = tracked(&value_drops);
        let invalid = if wrong_iid {
            WinRTValue::Object(
                PropertyValue::CreateString(windows_core::h!("not IStringable"))
                    .unwrap()
                    .cast()
                    .unwrap(),
            )
        } else {
            WinRTValue::I32(7)
        };
        // An invalid value cannot be hidden by either an earlier or a later duplicate.
        for entries in [
            vec![(key.clone(), value.clone()), (key.clone(), invalid.clone())],
            vec![(key.clone(), invalid.clone()), (key.clone(), value.clone())],
            vec![
                (key.clone(), value.clone()),
                (key.clone(), value.clone()),
                (invalid.clone(), value.clone()),
            ],
        ] {
            assert!(checked_map(&entries, &typ, &typ).is_err());
        }
        assert_eq!(key_drops.load(Ordering::SeqCst), 0);
        assert_eq!(value_drops.load(Ordering::SeqCst), 0);
        assert_eq!(
            stringable(&key).ToString().unwrap(),
            HSTRING::from("retained")
        );
        assert_eq!(
            stringable(&value).ToString().unwrap(),
            HSTRING::from("retained")
        );
        drop(key);
        drop(value);
        assert_eq!(key_drops.load(Ordering::SeqCst), 1);
        assert_eq!(value_drops.load(Ordering::SeqCst), 1);
    }
}
