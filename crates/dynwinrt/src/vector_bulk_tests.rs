// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use crate::{MetadataTable, TypeHandle, TypeKind, WinRTValue};
use windows_collections::IVector;
use windows_core::IInspectable;

fn query(object: &IUnknown, iid: &GUID) -> IUnknown {
    let mut pointer = std::ptr::null_mut();
    unsafe { object.query(iid, &mut pointer) }.ok().unwrap();
    unsafe { IUnknown::from_raw(pointer) }
}

fn check_output(
    expected: &[u8],
    count: usize,
    invoke: impl FnOnce(*mut *mut c_void, *mut u32) -> HRESULT,
) {
    const PREFIX: usize = 16;
    let pattern = usize::from_ne_bytes([0xa5; size_of::<usize>()]);
    let mut storage = [pattern; 32];
    let bytes = unsafe {
        std::slice::from_raw_parts_mut(storage.as_mut_ptr().cast::<u8>(), size_of_val(&storage))
    };
    let mut actual = u32::MAX;
    assert_eq!(
        invoke(
            unsafe { bytes.as_mut_ptr().add(PREFIX).cast() },
            &mut actual
        ),
        S_OK
    );
    assert_eq!(actual as usize, count);
    assert!(bytes[..PREFIX].iter().all(|byte| *byte == 0xa5));
    assert_eq!(&bytes[PREFIX..PREFIX + expected.len()], expected);
    assert!(
        bytes[PREFIX + expected.len()..]
            .iter()
            .all(|byte| *byte == 0xa5)
    );
}

fn iterator(object: &IUnknown, iids: &VectorIids) -> IUnknown {
    let iterable = query(object, &iids.iterable);
    let vtable = unsafe { &**(iterable.as_raw() as *const *const IterableVtbl) };
    let mut pointer = std::ptr::null_mut();
    assert_eq!(
        unsafe { (vtable.first)(iterable.as_raw(), &mut pointer) },
        S_OK
    );
    unsafe { IUnknown::from_raw(pointer) }
}

fn check_value_bulk(element: TypeHandle, items: Vec<Vec<u8>>) {
    let iids = element.table().vector_iids(&element);
    let values = items
        .iter()
        .map(|bytes| {
            assert_eq!(bytes.len(), element.size_of());
            match element.kind() {
                TypeKind::Bool => WinRTValue::Bool(bytes[0] != 0),
                TypeKind::I8 => WinRTValue::I8(bytes[0] as i8),
                TypeKind::U8 => WinRTValue::U8(bytes[0]),
                TypeKind::I16 => {
                    WinRTValue::I16(i16::from_ne_bytes(bytes.as_slice().try_into().unwrap()))
                }
                TypeKind::U16 | TypeKind::Char16 => {
                    WinRTValue::U16(u16::from_ne_bytes(bytes.as_slice().try_into().unwrap()))
                }
                TypeKind::I32 | TypeKind::Enum(_) | TypeKind::HResult => {
                    WinRTValue::I32(i32::from_ne_bytes(bytes.as_slice().try_into().unwrap()))
                }
                TypeKind::U32 => {
                    WinRTValue::U32(u32::from_ne_bytes(bytes.as_slice().try_into().unwrap()))
                }
                TypeKind::I64 => {
                    WinRTValue::I64(i64::from_ne_bytes(bytes.as_slice().try_into().unwrap()))
                }
                TypeKind::U64 => {
                    WinRTValue::U64(u64::from_ne_bytes(bytes.as_slice().try_into().unwrap()))
                }
                TypeKind::Struct(_) => {
                    let mut data = element.default_value();
                    data.set_field(0, bytes[0]);
                    data.set_field(1, bytes[1]);
                    WinRTValue::Struct(data)
                }
                kind => panic!("unexpected bulk test type: {kind:?}"),
            }
        })
        .collect::<Vec<_>>();
    let vector = create_vector_from_values(&values, &element, iids.clone()).unwrap();
    let mutable = query(&vector, &iids.vector);
    let vtable = unsafe { &**(mutable.as_raw() as *const *const VectorVtbl) };
    let live = query(&vector, &iids.vector_view);
    let live_vtable = unsafe { &**(live.as_raw() as *const *const VectorViewVtbl) };
    let mut snapshot_pointer = std::ptr::null_mut();
    assert_eq!(
        unsafe { (vtable.get_view)(mutable.as_raw(), &mut snapshot_pointer) },
        S_OK
    );
    let snapshot = unsafe { IUnknown::from_raw(snapshot_pointer) };
    let snapshot_vtable = unsafe { &**(snapshot.as_raw() as *const *const VectorViewVtbl) };

    for (start, capacity) in [(0, 0), (0, 2), (0, 4), (1, 2), (2, 4), (3, 2), (4, 2)] {
        let selected = items.iter().skip(start).take(capacity).collect::<Vec<_>>();
        let expected = selected
            .iter()
            .flat_map(|item| item.iter().copied())
            .collect::<Vec<_>>();
        check_output(&expected, selected.len(), |output, actual| unsafe {
            (vtable.get_many)(
                mutable.as_raw(),
                start as u32,
                capacity as u32,
                output,
                actual,
            )
        });
        check_output(&expected, selected.len(), |output, actual| unsafe {
            (live_vtable.get_many)(live.as_raw(), start as u32, capacity as u32, output, actual)
        });
        check_output(&expected, selected.len(), |output, actual| unsafe {
            (snapshot_vtable.get_many)(
                snapshot.as_raw(),
                start as u32,
                capacity as u32,
                output,
                actual,
            )
        });
    }

    for source in [&vector, &snapshot] {
        let iterator = iterator(source, &iids);
        let vtable = unsafe { &**(iterator.as_raw() as *const *const IteratorVtbl) };
        check_output(&[], 0, |output, actual| unsafe {
            (vtable.get_many)(iterator.as_raw(), 0, output, actual)
        });
        let expected = items[..2].concat();
        check_output(&expected, 2, |output, actual| unsafe {
            (vtable.get_many)(iterator.as_raw(), 2, output, actual)
        });
        check_output(&items[2], 1, |output, actual| unsafe {
            (vtable.get_many)(iterator.as_raw(), 3, output, actual)
        });
        check_output(&[], 0, |output, actual| unsafe {
            (vtable.get_many)(iterator.as_raw(), 2, output, actual)
        });
    }

    let replacement = items.iter().rev().flatten().copied().collect::<Vec<_>>();
    let mut source = [usize::from_ne_bytes([0xa5; size_of::<usize>()]); 32];
    let source_bytes = unsafe {
        std::slice::from_raw_parts_mut(source.as_mut_ptr().cast::<u8>(), size_of_val(&source))
    };
    source_bytes[..replacement.len()].copy_from_slice(&replacement);
    assert_eq!(
        unsafe {
            (vtable.replace_all)(mutable.as_raw(), items.len() as u32, source.as_ptr().cast())
        },
        S_OK
    );
    check_output(&replacement, items.len(), |output, actual| unsafe {
        (vtable.get_many)(mutable.as_raw(), 0, items.len() as u32, output, actual)
    });
    check_output(&replacement, items.len(), |output, actual| unsafe {
        (live_vtable.get_many)(live.as_raw(), 0, items.len() as u32, output, actual)
    });
    check_output(&items.concat(), items.len(), |output, actual| unsafe {
        (snapshot_vtable.get_many)(snapshot.as_raw(), 0, items.len() as u32, output, actual)
    });
    assert_eq!(
        unsafe { (vtable.replace_all)(mutable.as_raw(), 0, std::ptr::null()) },
        S_OK
    );
    check_output(&[], 0, |output, actual| unsafe {
        (vtable.get_many)(mutable.as_raw(), 0, 2, output, actual)
    });
}

#[test]
fn value_bulk_operations_use_element_size() {
    let table = MetadataTable::new();
    let i32_values = [1i32, 9, -7]
        .map(|value| value.to_ne_bytes().to_vec())
        .to_vec();
    for element in [
        table.i32_type(),
        table.hresult(),
        table.enum_type("Regression.Code", vec![]),
    ] {
        check_value_bulk(element, i32_values.clone());
    }
    for element in [table.u8_type(), table.i8_type()] {
        check_value_bulk(element, vec![vec![1], vec![9], vec![0xf9]]);
    }
    check_value_bulk(table.bool_type(), vec![vec![1], vec![0], vec![1]]);
    let u16_values = [1u16, 9, 0xfff9]
        .map(|value| value.to_ne_bytes().to_vec())
        .to_vec();
    for element in [table.u16_type(), table.i16_type(), table.char16_type()] {
        check_value_bulk(element, u16_values.clone());
    }
    check_value_bulk(
        table.struct_type("Regression.Pair", &[table.u8_type(), table.u8_type()]),
        vec![vec![1, 2], vec![3, 4], vec![5, 6]],
    );
    check_value_bulk(table.u32_type(), i32_values);
    #[cfg(target_pointer_width = "64")]
    for element in [table.i64_type(), table.u64_type()] {
        let values = [1i64, 0x1234_5678_9abc_def, -7]
            .map(|value| value.to_ne_bytes().to_vec())
            .to_vec();
        check_value_bulk(element, values);
    }
}

#[test]
fn enum_bulk_operations_match_typed_winrt_abi() {
    use windows_future::AsyncStatus;

    let table = MetadataTable::new();
    let element = table.enum_type("Windows.Foundation.AsyncStatus", vec![]);
    let items = [1, 9].map(|value| WinRTValue::Enum {
        value,
        type_handle: element.clone(),
    });
    let object = create_vector_from_values(&items, &element, table.vector_iids(&element)).unwrap();
    let vector: IVector<AsyncStatus> = object.cast().unwrap();
    let mut output = [AsyncStatus(-1); 8];
    assert_eq!(vector.GetMany(0, &mut output[..2]).unwrap(), 2);
    assert_eq!(&output[..2], &[AsyncStatus(1), AsyncStatus(9)]);
    assert!(output[2..].iter().all(|value| *value == AsyncStatus(-1)));
    vector
        .ReplaceAll(&[AsyncStatus(3), AsyncStatus(7)])
        .unwrap();
    assert_eq!(vector.GetAt(0).unwrap(), AsyncStatus(3));
    assert_eq!(vector.GetAt(1).unwrap(), AsyncStatus(7));
}

#[test]
fn hstring_bulk_operations_preserve_owned_outputs() {
    let table = MetadataTable::new();
    let element = table.hstring();
    let object = create_vector_from_values(&[], &element, table.vector_iids(&element)).unwrap();
    let vector: IVector<HSTRING> = object.cast().unwrap();
    vector
        .ReplaceAll(&[
            HSTRING::from("first"),
            HSTRING::new(),
            HSTRING::from("last"),
        ])
        .unwrap();
    let snapshot = vector.GetView().unwrap();
    let mut output = [HSTRING::new(), HSTRING::new(), HSTRING::new()];
    assert_eq!(snapshot.GetMany(0, &mut output).unwrap(), 3);
    let iterator = vector.First().unwrap();
    vector.ReplaceAll(&[HSTRING::from("replacement")]).unwrap();
    drop(snapshot);
    drop(vector);
    drop(object);
    assert_eq!(
        output,
        [
            HSTRING::from("first"),
            HSTRING::new(),
            HSTRING::from("last")
        ]
    );
    let mut iter_output = [HSTRING::new(), HSTRING::new(), HSTRING::new()];
    assert_eq!(iterator.GetMany(&mut iter_output).unwrap(), 3);
    drop(iterator);
    assert_eq!(iter_output, output);
}

#[windows_core::implement(windows::Foundation::IStringable)]
struct TrackedObject(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl windows::Foundation::IStringable_Impl for TrackedObject_Impl {
    fn ToString(&self) -> windows_core::Result<HSTRING> {
        Ok(HSTRING::from("retained"))
    }
}

impl Drop for TrackedObject {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn object_bulk_operations_preserve_nulls_and_references() {
    use windows::Foundation::IStringable;

    let dropped = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let tracked: IStringable = TrackedObject(dropped.clone()).into();
    let table = MetadataTable::new();
    let element = table.object();
    let object = create_vector_from_values(&[], &element, table.vector_iids(&element)).unwrap();
    let vector: IVector<IInspectable> = object.cast().unwrap();
    vector
        .ReplaceAll(&[Some(tracked.cast().unwrap()), None])
        .unwrap();
    drop(tracked);
    let snapshot = vector.GetView().unwrap();
    let iterator = vector.First().unwrap();
    let mut output: [Option<IInspectable>; 3] = [None, None, None];
    assert_eq!(vector.GetMany(0, &mut output).unwrap(), 2);
    let mut view_output: [Option<IInspectable>; 3] = [None, None, None];
    assert_eq!(snapshot.GetMany(0, &mut view_output).unwrap(), 2);
    vector.Clear().unwrap();
    drop(vector);
    drop(object);
    drop(snapshot);
    let mut iter_output: [Option<IInspectable>; 3] = [None, None, None];
    assert_eq!(iterator.GetMany(&mut iter_output).unwrap(), 2);
    drop(iterator);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    for values in [&output, &view_output, &iter_output] {
        assert_eq!(
            values[0]
                .as_ref()
                .unwrap()
                .cast::<IStringable>()
                .unwrap()
                .ToString()
                .unwrap(),
            HSTRING::from("retained")
        );
        assert!(values[1].is_none());
        assert!(values[2].is_none());
    }
    drop(output);
    drop(view_output);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    drop(iter_output);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}
