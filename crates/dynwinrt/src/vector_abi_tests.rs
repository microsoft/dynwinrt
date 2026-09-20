// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use crate::{MetadataTable, TypeHandle, WinRTValue};
use windows::Devices::Geolocation::BasicGeoposition;
use windows::Foundation::{Point, Rect, Size};
use windows::Graphics::{PointInt32, RectInt32};
use windows::UI::Input::ManipulationDelta;
use windows_collections::{IVector, IVectorView};

macro_rules! exercise_vector {
    ($native:ty, $element:expr, $item:expr, $values:expr) => {{
        let element = $element;
        assert_eq!(element.size_of(), size_of::<$native>());
        let values: [$native; 3] = $values;
        let object =
            create_vector_from_values(&[$item], &element, element.table().vector_iids(&element))
                .unwrap();
        let vector: IVector<$native> = object.cast().unwrap();
        let live: IVectorView<$native> = object.cast().unwrap();
        let snapshot = vector.GetView().unwrap();
        assert_eq!(vector.GetAt(0).unwrap(), values[0]);
        let mut index = u32::MAX;
        assert!(vector.IndexOf(values[0], &mut index).unwrap());
        assert_eq!(index, 0);
        vector.Append(values[1]).unwrap();
        vector.SetAt(0, values[2]).unwrap();
        vector.InsertAt(1, values[0]).unwrap();
        assert_eq!(live.GetAt(0).unwrap(), values[2]);
        assert_eq!(vector.GetAt(1).unwrap(), values[0]);
        assert_eq!(snapshot.GetAt(0).unwrap(), values[0]);
        assert!(live.IndexOf(values[0], &mut index).unwrap());
        assert_eq!(index, if values[2] == values[0] { 0 } else { 1 });
        assert!(snapshot.IndexOf(values[0], &mut index).unwrap());
        assert_eq!(index, 0);
        vector.ReplaceAll(&values).unwrap();
        let mut output = [values[2]; 4];
        assert_eq!(vector.GetMany(0, &mut output[..3]).unwrap(), 3);
        assert_eq!(&output[..3], &values);
        assert_eq!(output[3], values[2]);
        let iterator = vector.First().unwrap();
        vector.Clear().unwrap();
        assert_eq!(iterator.GetMany(&mut output[..3]).unwrap(), 3);
        assert_eq!(&output[..3], &values);
    }};
}

macro_rules! exercise_empty {
    ($native:ty, $element:expr, $value:expr, $supported:expr) => {{
        let element = $element;
        assert_eq!(element.size_of(), size_of::<$native>());
        let result =
            create_vector_from_values(&[], &element, element.table().vector_iids(&element));
        assert!(
            create_vector_from_values(
                &[WinRTValue::Struct(element.default_value())],
                &element,
                element.table().vector_iids(&element),
            )
            .is_err()
        );
        if $supported {
            let object = result.unwrap();
            let vector: IVector<$native> = object.cast().unwrap();
            let live: IVectorView<$native> = object.cast().unwrap();
            let snapshot = vector.GetView().unwrap();
            let value: $native = $value;
            let mut index = 123;
            assert_eq!(
                vector.IndexOf(value, &mut index).unwrap_err().code(),
                E_NOTIMPL
            );
            assert_eq!(index, 123);
            for view in [live, snapshot] {
                assert_eq!(
                    view.IndexOf(value, &mut index).unwrap_err().code(),
                    E_NOTIMPL
                );
                assert_eq!(index, 123);
                let mut out = [value; 2];
                assert_eq!(view.GetMany(0, &mut out).unwrap(), 0);
                assert_eq!(out, [value; 2]);
            }
            assert_eq!(vector.Append(value).unwrap_err().code(), E_NOTIMPL);
            assert_eq!(vector.SetAt(0, value).unwrap_err().code(), E_BOUNDS);
            assert_eq!(vector.InsertAt(0, value).unwrap_err().code(), E_NOTIMPL);
            assert_eq!(vector.InsertAt(1, value).unwrap_err().code(), E_BOUNDS);
            assert_eq!(vector.ReplaceAll(&[value]).unwrap_err().code(), E_NOTIMPL);
            vector.ReplaceAll(&[]).unwrap();
            assert_eq!(vector.Size().unwrap(), 0);
            let mut out = [value; 2];
            assert_eq!(vector.GetMany(0, &mut out).unwrap(), 0);
            assert_eq!(out, [value; 2]);
            assert_eq!(vector.GetAt(0).unwrap_err().code(), E_BOUNDS);
            let iterator = vector.First().unwrap();
            assert!(!iterator.HasCurrent().unwrap());
            assert_eq!(iterator.GetMany(&mut out).unwrap(), 0);
            assert_eq!(out, [value; 2]);
            assert_eq!(iterator.Current().unwrap_err().code(), E_BOUNDS);
            vector.Clear().unwrap();
        } else {
            assert!(matches!(
                result,
                Err(crate::Error::UnsupportedCollectionElement(_))
            ));
        }
    }};
}

fn pair(typ: &TypeHandle, x: i32, y: i32) -> WinRTValue {
    let mut value = typ.default_value();
    value.set_field(0, x);
    value.set_field(1, y);
    WinRTValue::Struct(value)
}

#[test]
fn typed_scalar_and_enum_vector_operations() {
    let table = MetadataTable::new();
    exercise_vector!(
        bool,
        table.bool_type(),
        WinRTValue::Bool(false),
        [false, true, false]
    );
    exercise_vector!(i8, table.i8_type(), WinRTValue::I32(-8), [-8, 7, -1]);
    exercise_vector!(u8, table.u8_type(), WinRTValue::I32(255), [255, 7, 1]);
    exercise_vector!(i16, table.i16_type(), WinRTValue::I16(-8), [-8, 7, -1]);
    exercise_vector!(u16, table.u16_type(), WinRTValue::U16(65535), [65535, 7, 1]);
    exercise_vector!(i32, table.i32_type(), WinRTValue::I32(-8), [-8, 7, -1]);
    exercise_vector!(
        i32,
        table.hresult(),
        WinRTValue::HResult(HRESULT(-8)),
        [-8, 7, -1]
    );
    exercise_vector!(
        u32,
        table.u32_type(),
        WinRTValue::U32(u32::MAX),
        [u32::MAX, 7, 1]
    );
    if cfg!(target_pointer_width = "64") {
        exercise_vector!(
            i64,
            table.i64_type(),
            WinRTValue::I64(i64::MIN),
            [i64::MIN, i64::MAX, -1]
        );
        exercise_vector!(
            u64,
            table.u64_type(),
            WinRTValue::U64(u64::MAX),
            [u64::MAX, 7, 1]
        );
    } else {
        for element in [table.i64_type(), table.u64_type()] {
            assert!(create_vector_from_values(&[], &element, table.vector_iids(&element)).is_err());
        }
    }
    use windows_future::AsyncStatus;
    exercise_vector!(
        AsyncStatus,
        table.enum_type("Windows.Foundation.AsyncStatus", vec![]),
        WinRTValue::I32(1),
        [AsyncStatus(1), AsyncStatus(2), AsyncStatus(99)]
    );
}

#[test]
fn typed_point_size_and_integer_pair_boundaries() {
    let table = MetadataTable::new();
    let color = table.struct_type("Windows.UI.Color", &vec![table.u8_type(); 4]);
    let mut data = color.default_value();
    for index in 0..4 {
        data.set_field(index, (index + 1) as u8);
    }
    exercise_vector!(
        windows::UI::Color,
        color,
        WinRTValue::Struct(data),
        [
            windows::UI::Color {
                A: 1,
                R: 2,
                G: 3,
                B: 4
            },
            windows::UI::Color {
                A: 5,
                R: 6,
                G: 7,
                B: 8
            },
            windows::UI::Color {
                A: 9,
                R: 10,
                G: 11,
                B: 12
            }
        ]
    );
    let point = table.struct_type(
        "Windows.Foundation.Point",
        &[table.f32_type(), table.f32_type()],
    );
    let size = table.struct_type(
        "Windows.Foundation.Size",
        &[table.f32_type(), table.f32_type()],
    );
    let point_int = table.struct_type(
        "Windows.Graphics.PointInt32",
        &[table.i32_type(), table.i32_type()],
    );
    for element in [&point, &size] {
        let mut data = element.default_value();
        data.set_field(0, 1.0f32);
        data.set_field(1, 2.0f32);
        let item = WinRTValue::Struct(data);
        if cfg!(target_arch = "x86_64") {
            if element == &point {
                exercise_vector!(
                    Point,
                    element.clone(),
                    item,
                    [
                        Point { X: 1.0, Y: 2.0 },
                        Point { X: 3.0, Y: 4.0 },
                        Point { X: 5.0, Y: 6.0 }
                    ]
                );
            } else {
                exercise_vector!(
                    Size,
                    element.clone(),
                    item,
                    [
                        Size {
                            Width: 1.0,
                            Height: 2.0
                        },
                        Size {
                            Width: 3.0,
                            Height: 4.0
                        },
                        Size {
                            Width: 5.0,
                            Height: 6.0
                        }
                    ]
                );
            }
        } else {
            for values in [vec![], vec![item]] {
                assert!(
                    create_vector_from_values(&values, element, table.vector_iids(element))
                        .is_err()
                );
            }
        }
    }
    if cfg!(target_pointer_width = "64") {
        exercise_vector!(
            PointInt32,
            point_int.clone(),
            pair(&point_int, 1, 2),
            [
                PointInt32 { X: 1, Y: 2 },
                PointInt32 { X: 3, Y: 4 },
                PointInt32 { X: 5, Y: 6 }
            ]
        );
    } else {
        assert!(create_vector_from_values(&[], &point_int, table.vector_iids(&point_int)).is_err());
    }
}

#[cfg(target_arch = "x86_64")]
fn point_value(typ: &TypeHandle, point: Point) -> WinRTValue {
    let mut value = typ.default_value();
    value.set_field(0, point.X);
    value.set_field(1, point.Y);
    WinRTValue::Struct(value)
}

#[cfg(target_arch = "x86_64")]
#[test]
fn struct_equality_point_signed_zero_matches_all_views() {
    let table = MetadataTable::new();
    let typ = table.struct_type(
        "Windows.Foundation.Point",
        &[table.f32_type(), table.f32_type()],
    );
    for zero in [0.0f32, -0.0] {
        let stored = Point { X: zero, Y: 1.0 };
        let opposite = Point { X: -zero, Y: 1.0 };
        assert_eq!(stored, opposite);
        assert_ne!(stored.X.to_bits(), opposite.X.to_bits());
        let object =
            create_vector_from_values(&[point_value(&typ, stored)], &typ, table.vector_iids(&typ))
                .unwrap();
        let vector: IVector<Point> = object.cast().unwrap();
        let live: IVectorView<Point> = object.cast().unwrap();
        let snapshot = vector.GetView().unwrap();
        for (query, expected) in [
            (stored, true),
            (opposite, true),
            (Point { X: 2.0, Y: 1.0 }, false),
            (Point { X: zero, Y: 2.0 }, false),
        ] {
            let mut index = u32::MAX;
            assert_eq!(vector.IndexOf(query, &mut index).unwrap(), expected);
            assert_eq!(index, 0);
            for view in [&live, &snapshot] {
                index = u32::MAX;
                assert_eq!(view.IndexOf(query, &mut index).unwrap(), expected);
                assert_eq!(index, 0);
            }
        }
        assert_eq!(vector.GetAt(0).unwrap().X.to_bits(), zero.to_bits());
    }
}

#[cfg(target_arch = "x86_64")]
#[test]
fn struct_equality_point_nan_never_matches_even_identical_bits() {
    let table = MetadataTable::new();
    let typ = table.struct_type(
        "Windows.Foundation.Point",
        &[table.f32_type(), table.f32_type()],
    );
    let nan = f32::from_bits(0x7fc0_0001);
    let other_nan = f32::from_bits(0x7fc0_0002);
    for (stored, other) in [
        (
            Point { X: nan, Y: 1.0 },
            Point {
                X: other_nan,
                Y: 1.0,
            },
        ),
        (
            Point { X: 1.0, Y: nan },
            Point {
                X: 1.0,
                Y: other_nan,
            },
        ),
    ] {
        assert_ne!(stored, stored);
        let object =
            create_vector_from_values(&[point_value(&typ, stored)], &typ, table.vector_iids(&typ))
                .unwrap();
        let vector: IVector<Point> = object.cast().unwrap();
        let live: IVectorView<Point> = object.cast().unwrap();
        let snapshot = vector.GetView().unwrap();
        for query in [stored, other] {
            let mut index = u32::MAX;
            assert!(!vector.IndexOf(query, &mut index).unwrap());
            assert_eq!(index, 0);
            for view in [&live, &snapshot] {
                assert!(!view.IndexOf(query, &mut index).unwrap());
                assert_eq!(index, 0);
            }
        }
        let output = vector.GetAt(0).unwrap();
        assert_eq!(output.X.to_bits(), stored.X.to_bits());
        assert_eq!(output.Y.to_bits(), stored.Y.to_bits());
    }
}

#[cfg(target_arch = "x86_64")]
#[test]
fn struct_equality_survives_mutation_snapshots_and_metadata_drop() {
    let (object, weak) = {
        let table = MetadataTable::new();
        let typ = table.struct_type(
            "Windows.Foundation.Point",
            &[table.f32_type(), table.f32_type()],
        );
        (
            create_vector_from_values(&[], &typ, table.vector_iids(&typ)).unwrap(),
            std::sync::Arc::downgrade(&table),
        )
    };
    assert!(weak.upgrade().is_none());
    let vector: IVector<Point> = object.cast().unwrap();
    let live: IVectorView<Point> = object.cast().unwrap();
    let positive = Point { X: 0.0, Y: 1.0 };
    let negative = Point { X: -0.0, Y: 1.0 };
    let different = Point { X: 2.0, Y: 3.0 };
    let mut index = u32::MAX;
    vector.Append(positive).unwrap();
    assert!(vector.IndexOf(negative, &mut index).unwrap());
    assert!(live.IndexOf(negative, &mut index).unwrap());
    let snapshot = vector.GetView().unwrap();
    vector.SetAt(0, different).unwrap();
    assert!(!live.IndexOf(negative, &mut index).unwrap());
    assert!(snapshot.IndexOf(negative, &mut index).unwrap());
    vector.InsertAt(0, negative).unwrap();
    assert!(live.IndexOf(positive, &mut index).unwrap());
    assert_eq!(index, 0);
    assert_eq!(vector.GetAt(0).unwrap().X.to_bits(), (-0.0f32).to_bits());
    vector.ReplaceAll(&[different, positive]).unwrap();
    assert!(vector.IndexOf(negative, &mut index).unwrap());
    assert_eq!(index, 1);
    assert!(live.IndexOf(negative, &mut index).unwrap());
    assert_eq!(index, 1);
    vector.Clear().unwrap();
    drop(live);
    drop(vector);
    drop(object);
    assert!(snapshot.IndexOf(negative, &mut index).unwrap());
    assert_eq!(index, 0);
    assert_eq!(snapshot.GetAt(0).unwrap().X.to_bits(), 0);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn struct_equality_point_map_keys_agree_with_mutators_and_snapshot() {
    use windows_collections::IMap;

    let positive = Point { X: 0.0, Y: 1.0 };
    let negative = Point { X: -0.0, Y: 1.0 };
    let (object, weak) = {
        let table = MetadataTable::new();
        let typ = table.struct_type(
            "Windows.Foundation.Point",
            &[table.f32_type(), table.f32_type()],
        );
        (
            crate::map::create_map_from_values(
                &[(point_value(&typ, positive), WinRTValue::I32(7))],
                &typ,
                &table.i32_type(),
                table.map_iids(&typ, &table.i32_type()),
            )
            .unwrap(),
            std::sync::Arc::downgrade(&table),
        )
    };
    assert!(weak.upgrade().is_none());
    let map: IMap<Point, i32> = object.cast().unwrap();
    assert_eq!(map.Lookup(negative).unwrap(), 7);
    assert!(map.HasKey(negative).unwrap());
    assert!(!map.HasKey(Point { X: 1.0, Y: 1.0 }).unwrap());
    let snapshot = map.GetView().unwrap();
    assert!(map.Insert(negative, 9).unwrap());
    assert_eq!(map.Size().unwrap(), 1);
    assert_eq!(map.Lookup(positive).unwrap(), 9);
    assert_eq!(map.Lookup(negative).unwrap(), 9);
    assert_eq!(
        map.First()
            .unwrap()
            .Current()
            .unwrap()
            .Key()
            .unwrap()
            .X
            .to_bits(),
        0
    );
    map.Remove(negative).unwrap();
    assert_eq!(map.Size().unwrap(), 0);
    assert!(!map.HasKey(positive).unwrap());
    drop(map);
    drop(object);
    assert!(snapshot.HasKey(negative).unwrap());
    assert_eq!(snapshot.Lookup(negative).unwrap(), 7);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn struct_equality_point_nan_map_key_is_not_found() {
    use windows_collections::IMap;

    let table = MetadataTable::new();
    let typ = table.struct_type(
        "Windows.Foundation.Point",
        &[table.f32_type(), table.f32_type()],
    );
    let point = Point {
        X: f32::from_bits(0x7fc0_0001),
        Y: 1.0,
    };
    let object = crate::map::create_map_from_values(
        &[(point_value(&typ, point), WinRTValue::I32(7))],
        &typ,
        &table.i32_type(),
        table.map_iids(&typ, &table.i32_type()),
    )
    .unwrap();
    let map: IMap<Point, i32> = object.cast().unwrap();
    let snapshot = map.GetView().unwrap();
    assert!(!map.HasKey(point).unwrap());
    assert_eq!(map.Lookup(point).unwrap_err().code(), E_BOUNDS);
    assert_eq!(map.Remove(point).unwrap_err().code(), E_BOUNDS);
    assert!(!snapshot.HasKey(point).unwrap());
    assert_eq!(snapshot.Lookup(point).unwrap_err().code(), E_BOUNDS);
    assert_eq!(map.Size().unwrap(), 1);
    map.Clear().unwrap();
    assert_eq!(map.Size().unwrap(), 0);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn struct_equality_raw_constructor_preserves_packed_byte_comparison() {
    let table = MetadataTable::new();
    let typ = table.struct_type(
        "Windows.Foundation.Point",
        &[table.f32_type(), table.f32_type()],
    );
    for x in [0.0f32, f32::from_bits(0x7fc0_0001)] {
        let bytes = [x.to_ne_bytes(), 1.0f32.to_ne_bytes()].concat();
        let object = unsafe { create_value_vector(vec![bytes], 8, table.vector_iids(&typ)) };
        let vector: IVector<Point> = object.cast().unwrap();
        let mut index = u32::MAX;
        assert!(vector.IndexOf(Point { X: x, Y: 1.0 }, &mut index).unwrap());
        assert_eq!(index, 0);
        if x == 0.0 {
            assert!(
                !vector
                    .IndexOf(Point { X: -0.0, Y: 1.0 }, &mut index)
                    .unwrap()
            );
        }
    }
}

#[test]
fn typed_empty_only_uses_indirect_argument_abi() {
    let table = MetadataTable::new();
    let rect_int = table.struct_type("Windows.Graphics.RectInt32", &vec![table.i32_type(); 4]);
    exercise_empty!(
        RectInt32,
        rect_int,
        RectInt32 {
            X: 1,
            Y: 2,
            Width: 3,
            Height: 4
        },
        cfg!(target_arch = "x86_64")
    );
    let rect = table.struct_type("Windows.Foundation.Rect", &vec![table.f32_type(); 4]);
    exercise_empty!(
        Rect,
        rect,
        Rect {
            X: 1.0,
            Y: 2.0,
            Width: 3.0,
            Height: 4.0
        },
        cfg!(target_arch = "x86_64")
    );
    let position = table.struct_type(
        "Windows.Devices.Geolocation.BasicGeoposition",
        &vec![table.f64_type(); 3],
    );
    exercise_empty!(
        BasicGeoposition,
        position,
        BasicGeoposition {
            Latitude: 1.0,
            Longitude: 2.0,
            Altitude: 3.0
        },
        cfg!(target_arch = "x86_64")
    );
    let point = table.struct_type(
        "Windows.Foundation.Point",
        &[table.f32_type(), table.f32_type()],
    );
    let delta = table.struct_type(
        "Windows.UI.Input.ManipulationDelta",
        &[point, table.f32_type(), table.f32_type(), table.f32_type()],
    );
    exercise_empty!(
        ManipulationDelta,
        delta,
        ManipulationDelta {
            Translation: Point { X: 1.0, Y: 2.0 },
            Scale: 3.0,
            Rotation: 4.0,
            Expansion: 5.0
        },
        cfg!(any(target_arch = "x86_64", target_arch = "aarch64"))
    );
}

#[test]
fn typed_constructors_reject_mismatched_iids_before_publication() {
    let table = MetadataTable::new();
    let element = table.i32_type();
    let expected = table.vector_iids(&element);
    for field in 0..6 {
        let mut iids = expected.clone();
        let fields = [
            &mut iids.iterable,
            &mut iids.vector,
            &mut iids.vector_view,
            &mut iids.observable_vector,
            &mut iids.vector_changed_handler,
            &mut iids.iterator,
        ];
        *fields.into_iter().nth(field).unwrap() = GUID::zeroed();
        assert!(create_vector_from_values(&[], &element, iids).is_err());
    }
}

#[test]
fn raw_byte_constructor_checks_lengths_and_target_sizes() {
    let table = MetadataTable::new();
    for bytes in [vec![1], vec![1; 5]] {
        let result = std::panic::catch_unwind(|| unsafe {
            create_value_vector(vec![bytes], 4, table.vector_iids(&table.i32_type()))
        });
        assert!(result.is_err());
    }

    assert!(std::panic::catch_unwind(|| CollectionStorage::raw_value(0, true)).is_err());
    if cfg!(any(target_arch = "x86", target_arch = "aarch64")) {
        assert!(std::panic::catch_unwind(|| CollectionStorage::raw_value(16, true)).is_err());
    }
}

#[windows_core::implement(windows::Foundation::IStringable, windows::Foundation::IClosable)]
struct Tracked(std::sync::Arc<std::sync::atomic::AtomicUsize>);

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
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn tracked_value(dropped: &std::sync::Arc<std::sync::atomic::AtomicUsize>) -> WinRTValue {
    let source: windows::Foundation::IClosable = Tracked(dropped.clone()).into();
    // This owns the real IClosable view; collection preparation must QI to IStringable.
    WinRTValue::Object(unsafe { IUnknown::from_raw(source.into_raw()) })
}

#[test]
fn nonstruct_collection_comparison_preserves_content_and_identity() {
    use windows::Foundation::IStringable;
    use windows_collections::IMap;

    let table = MetadataTable::new();
    let strings = create_vector_from_values(
        &[WinRTValue::HString(HSTRING::from("same content"))],
        &table.hstring(),
        table.vector_iids(&table.hstring()),
    )
    .unwrap();
    let vector: IVector<HSTRING> = strings.cast().unwrap();
    let mut index = u32::MAX;
    assert!(
        vector
            .IndexOf(&HSTRING::from("same content"), &mut index)
            .unwrap()
    );
    assert_eq!(index, 0);
    assert!(
        !vector
            .IndexOf(&HSTRING::from("different"), &mut index)
            .unwrap()
    );

    let dropped = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let first: IStringable = Tracked(dropped.clone()).into();
    let other: IStringable = Tracked(dropped.clone()).into();
    assert_eq!(first.ToString().unwrap(), other.ToString().unwrap());
    let typ = table.interface(IStringable::IID);
    let value = WinRTValue::Object(first.cast().unwrap());
    let object =
        create_vector_from_values(std::slice::from_ref(&value), &typ, table.vector_iids(&typ))
            .unwrap();
    let vector: IVector<IStringable> = object.cast().unwrap();
    assert!(vector.IndexOf(&first, &mut index).unwrap());
    assert_eq!(index, 0);
    assert!(!vector.IndexOf(&other, &mut index).unwrap());
    let mapping = crate::map::create_map_from_values(
        &[(value, WinRTValue::I32(7))],
        &typ,
        &table.i32_type(),
        table.map_iids(&typ, &table.i32_type()),
    )
    .unwrap();
    let map: IMap<IStringable, i32> = mapping.cast().unwrap();
    assert_eq!(map.Lookup(&first).unwrap(), 7);
    assert!(!map.HasKey(&other).unwrap());
    vector.Clear().unwrap();
    map.Clear().unwrap();
    drop(first);
    drop(other);
    assert_eq!(dropped.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[test]
fn typed_interface_preparation_adjusts_and_retains_the_expected_view() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use windows::Foundation::IStringable;
    let table = MetadataTable::new();
    let element = table.interface(IStringable::IID);
    let dropped = Arc::new(AtomicUsize::new(0));
    let input = tracked_value(&dropped);
    let object = create_vector_from_values(
        &[input.clone(), WinRTValue::Null],
        &element,
        table.vector_iids(&element),
    )
    .unwrap();
    let vector: IVector<IStringable> = object.cast().unwrap();
    let output = vector.GetAt(0).unwrap();
    drop(input);
    vector.Clear().unwrap();
    drop(vector);
    drop(object);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    assert_eq!(output.ToString().unwrap(), HSTRING::from("retained"));
    drop(output);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

#[test]
fn failed_preparation_releases_earlier_vector_and_map_items() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use windows::Foundation::IStringable;
    let table = MetadataTable::new();
    let element = table.interface(IStringable::IID);
    let dropped = Arc::new(AtomicUsize::new(0));
    let input = tracked_value(&dropped);
    assert!(
        create_vector_from_values(
            &[input.clone(), WinRTValue::I32(2)],
            &element,
            table.vector_iids(&element)
        )
        .is_err()
    );
    let wrong = table.interface(windows::Devices::Geolocation::IGeopoint::IID);
    assert!(
        create_vector_from_values(
            std::slice::from_ref(&input),
            &wrong,
            table.vector_iids(&wrong)
        )
        .is_err()
    );
    assert!(
        create_vector_from_values(
            &[WinRTValue::RawPtr(std::ptr::null_mut())],
            &element,
            table.vector_iids(&element)
        )
        .is_err()
    );
    let key = table.hstring();
    assert!(
        crate::map::create_map_from_values(
            &[
                (WinRTValue::HString(HSTRING::from("first")), input.clone()),
                (WinRTValue::I32(2), input.clone())
            ],
            &key,
            &element,
            table.map_iids(&key, &element),
        )
        .is_err()
    );
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    drop(input);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

#[test]
fn typed_maps_preserve_hstring_and_interface_outputs() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use windows::Foundation::IStringable;
    use windows_collections::IMap;
    let table = MetadataTable::new();
    let key = table.hstring();
    let value = table.interface(IStringable::IID);
    let dropped = Arc::new(AtomicUsize::new(0));
    let input = tracked_value(&dropped);
    let object = crate::map::create_map_from_values(
        &[(WinRTValue::HString(HSTRING::from("key")), input.clone())],
        &key,
        &value,
        table.map_iids(&key, &value),
    )
    .unwrap();
    let map: IMap<HSTRING, IStringable> = object.cast().unwrap();
    drop(input);
    let view = map.GetView().unwrap();
    let pair = map.First().unwrap().Current().unwrap();
    let output = map.Lookup(windows_core::h!("key")).unwrap();
    assert!(map.HasKey(windows_core::h!("key")).unwrap());
    assert!(map.Insert(windows_core::h!("key"), &output).unwrap());
    map.Remove(windows_core::h!("key")).unwrap();
    assert!(!map.HasKey(windows_core::h!("key")).unwrap());
    drop(map);
    drop(object);
    assert_eq!(pair.Key().unwrap(), HSTRING::from("key"));
    assert_eq!(
        pair.Value().unwrap().ToString().unwrap(),
        HSTRING::from("retained")
    );
    assert_eq!(
        view.Lookup(windows_core::h!("key"))
            .unwrap()
            .ToString()
            .unwrap(),
        HSTRING::from("retained")
    );
    drop(view);
    drop(pair);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    drop(output);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

#[test]
fn typed_maps_handle_struct_keys_and_values() {
    use windows_collections::IMap;
    let table = MetadataTable::new();
    let point = table.struct_type(
        "Windows.Graphics.PointInt32",
        &[table.i32_type(), table.i32_type()],
    );
    if cfg!(target_arch = "x86") {
        for (key, value) in [
            (&point, table.i32_type()),
            (&table.i32_type(), point.clone()),
        ] {
            assert!(
                crate::map::create_map_from_values(&[], key, &value, table.map_iids(key, &value))
                    .is_err()
            );
        }
        return;
    }
    let object = crate::map::create_map_from_values(
        &[(WinRTValue::I32(7), pair(&point, 1, 2))],
        &table.i32_type(),
        &point,
        table.map_iids(&table.i32_type(), &point),
    )
    .unwrap();
    let map: IMap<i32, PointInt32> = object.cast().unwrap();
    assert_eq!(map.Lookup(7).unwrap(), PointInt32 { X: 1, Y: 2 });
    assert!(!map.Insert(8, PointInt32 { X: 3, Y: 4 }).unwrap());
    let view = map.GetView().unwrap();
    assert_eq!(view.Lookup(8).unwrap(), PointInt32 { X: 3, Y: 4 });
    assert_eq!(
        map.First().unwrap().Current().unwrap().Value().unwrap(),
        PointInt32 { X: 1, Y: 2 }
    );
    let object = crate::map::create_map_from_values(
        &[(pair(&point, 1, 2), WinRTValue::I32(7))],
        &point,
        &table.i32_type(),
        table.map_iids(&point, &table.i32_type()),
    )
    .unwrap();
    let map: IMap<PointInt32, i32> = object.cast().unwrap();
    let key = PointInt32 { X: 1, Y: 2 };
    assert_eq!(map.Lookup(key).unwrap(), 7);
    assert!(map.HasKey(key).unwrap());
    assert!(map.Insert(key, 9).unwrap());
    assert_eq!(map.GetView().unwrap().Lookup(key).unwrap(), 9);
    assert_eq!(map.First().unwrap().Current().unwrap().Key().unwrap(), key);
    map.Remove(key).unwrap();
    assert!(!map.HasKey(key).unwrap());
}

#[test]
fn maps_reject_unsupported_shapes_iid_sets_and_cross_table_handles() {
    let table = MetadataTable::new();
    let scalar = table.i32_type();
    let point = table.struct_type(
        "Windows.Foundation.Point",
        &[table.f32_type(), table.f32_type()],
    );
    let large = table.struct_type("Windows.Graphics.RectInt32", &vec![scalar.clone(); 4]);
    let odd = table.struct_type("Test.Odd", &vec![table.u8_type(); 3]);
    for (element, supported) in [
        (point, cfg!(target_arch = "x86_64")),
        (large, false),
        (odd, cfg!(not(target_arch = "x86_64"))),
    ] {
        for (key, value) in [(&element, &scalar), (&scalar, &element)] {
            assert_eq!(
                crate::map::create_map_from_values(&[], key, value, table.map_iids(key, value))
                    .is_ok(),
                supported
            );
        }
    }
    let expected = table.map_iids(&scalar, &scalar);
    for field in 0..5 {
        let mut iids = expected.clone();
        let fields = [
            &mut iids.iterable,
            &mut iids.map,
            &mut iids.map_view,
            &mut iids.kvp,
            &mut iids.iterator,
        ];
        *fields.into_iter().nth(field).unwrap() = GUID::zeroed();
        assert!(crate::map::create_map_from_values(&[], &scalar, &scalar, iids).is_err());
    }
    let other_table = MetadataTable::new();
    assert!(
        crate::map::create_map_from_values(&[], &scalar, &other_table.i32_type(), expected)
            .is_err()
    );
}
