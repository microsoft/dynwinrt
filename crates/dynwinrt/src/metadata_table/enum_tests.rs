// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use crate::{
    ArrayData, WinRTValue, WinRtImplementation, WinRtImplementationPlan, WinRtInterfaceDefinition,
    WinRtMethodDefinition, WinRtThreadingPolicy,
};
use windows::Foundation::IReference;
use windows::Gaming::Input::{GamepadButtons, GamepadReading};
use windows_collections::{IIterable, IIterator, IMap, IVector, IVectorView};
use windows_core::{Interface, RuntimeType};

fn flags(table: &Arc<MetadataTable>) -> TypeHandle {
    table
        .enum_type_with_underlying(
            "Windows.Gaming.Input.GamepadButtons",
            vec![("High".into(), i32::MIN), ("All".into(), -1)],
            TypeKind::U32,
        )
        .unwrap()
}

#[test]
fn enum_signedness_matches_sdk_signatures_and_closed_iids() {
    let table = MetadataTable::new();
    let flags = flags(&table);
    assert_eq!(
        flags.signature_string().as_bytes(),
        GamepadButtons::SIGNATURE.as_slice()
    );
    assert_eq!(flags.abi_type(), crate::abi::AbiType::U32);
    assert_eq!(flags.size_of(), 4);
    assert_eq!(flags.align_of(), 4);
    let iids = table.vector_iids(&flags);
    assert_eq!(iids.vector, IVector::<GamepadButtons>::IID);
    assert_eq!(iids.vector_view, IVectorView::<GamepadButtons>::IID);
    assert_eq!(iids.iterable, IIterable::<GamepadButtons>::IID);
    assert_eq!(iids.iterator, IIterator::<GamepadButtons>::IID);
    assert_eq!(
        table
            .parameterized(&table.generic(IREFERENCE, 1), &[flags.clone()])
            .iid(),
        Some(IReference::<GamepadButtons>::IID)
    );
    assert_eq!(
        table.map_iids(&flags, &flags).map,
        IMap::<GamepadButtons, GamepadButtons>::IID
    );

    let reading = table.struct_type(
        "Windows.Gaming.Input.GamepadReading",
        &[
            table.u64_type(),
            flags,
            table.f64_type(),
            table.f64_type(),
            table.f64_type(),
            table.f64_type(),
            table.f64_type(),
            table.f64_type(),
        ],
    );
    assert_eq!(
        reading.signature_string().as_bytes(),
        GamepadReading::SIGNATURE.as_slice()
    );
    assert_eq!(
        table.vector_iids(&reading).vector,
        IVector::<GamepadReading>::IID
    );

    let signed = table.enum_type("Windows.Foundation.AsyncStatus", vec![("Error".into(), 3)]);
    assert_eq!(signed.abi_type(), crate::abi::AbiType::I32);
    assert_eq!(
        signed.signature_string().as_bytes(),
        windows_future::AsyncStatus::SIGNATURE.as_slice()
    );
    assert_eq!(
        table.vector_iids(&signed).vector,
        IVector::<windows_future::AsyncStatus>::IID
    );
    assert_eq!(signed.enum_value(-1).unwrap().as_enum_number(), Some(-1));
}

#[test]
fn enum_registration_and_values_reject_incompatible_backing_types() {
    let table = MetadataTable::new();
    let unsigned = flags(&table);
    assert_eq!(flags(&table), unsigned);
    assert!(
        table
            .enum_type_with_underlying("Windows.Gaming.Input.GamepadButtons", vec![], TypeKind::I32)
            .is_err()
    );
    assert!(
        table
            .enum_type_with_underlying("Test.Invalid", vec![], TypeKind::U64)
            .is_err()
    );
    table.struct_type("Test.Conflict", &[table.u32_type()]);
    assert!(
        table
            .enum_type_with_underlying("Test.Conflict", vec![], TypeKind::U32)
            .is_err()
    );
    assert_eq!(
        table.get_enum_value_i64("Windows.Gaming.Input.GamepadButtons", "High"),
        Some(0x8000_0000)
    );
    assert_eq!(
        table.get_enum_value_i64("Windows.Gaming.Input.GamepadButtons", "All"),
        Some(0xffff_ffff)
    );
    assert_eq!(
        table.get_enum_value("Windows.Gaming.Input.GamepadButtons", "All"),
        Some(-1)
    );
    for invalid in [-1, 0x1_0000_0000] {
        assert!(unsigned.enum_value(invalid).is_err());
    }
    let signed = table.enum_type("Test.Signed", vec![("MinusOne".into(), -1)]);
    assert_eq!(
        table.get_enum_value_i64("Test.Signed", "MinusOne"),
        Some(-1)
    );
    assert!(signed.enum_value(0x8000_0000).is_err());
    assert!(signed.enum_value(i64::from(i32::MIN) - 1).is_err());
    assert!(table.u32_type().enum_value(1).is_err());
    assert_eq!(
        unsigned.enum_value(0xffff_ffff).unwrap().as_enum_number(),
        Some(0xffff_ffff)
    );
}

#[test]
fn enum_flags_roundtrip_through_sdk_vectors_maps_and_references() {
    let table = MetadataTable::new();
    let typ = flags(&table);
    let values = [0, 0x8000_0000, 0xffff_ffff];
    let inputs = values.map(|value| typ.enum_value(value).unwrap());
    let raw =
        crate::vector::create_vector_from_values(&inputs, &typ, table.vector_iids(&typ)).unwrap();
    let vector: IVector<GamepadButtons> = raw.cast().unwrap();
    for (index, expected) in values.into_iter().enumerate() {
        assert_eq!(i64::from(vector.GetAt(index as u32).unwrap().0), expected);
    }
    vector.Append(GamepadButtons(0x8000_0001)).unwrap();
    let mut index = 0;
    assert!(
        vector
            .IndexOf(GamepadButtons(0xffff_ffff), &mut index)
            .unwrap()
    );
    assert_eq!(index, 2);
    let mut output = [GamepadButtons(0); 4];
    assert_eq!(vector.GetMany(0, &mut output).unwrap(), 4);
    assert_eq!(
        output.map(|value| value.0),
        [0, 0x8000_0000, 0xffff_ffff, 0x8000_0001]
    );
    assert_eq!(vector.GetView().unwrap().GetAt(2).unwrap().0, u32::MAX);

    let get = MethodSignature::new(&table)
        .add_in(table.u32_type())
        .add_out(typ.clone())
        .build(6);
    let returned = get
        .call_dynamic(vector.as_raw(), &[WinRTValue::U32(2)])
        .unwrap();
    assert_eq!(returned[0].as_enum_number(), Some(0xffff_ffff));
    let append = MethodSignature::new(&table).add_in(typ.clone()).build(13);
    append
        .call_dynamic(vector.as_raw(), &[typ.enum_value(0xffff_ffff).unwrap()])
        .unwrap();
    append
        .call_dynamic(vector.as_raw(), &[WinRTValue::U32(0x8000_0000)])
        .unwrap();
    assert!(
        append
            .call_dynamic(vector.as_raw(), &[WinRTValue::I32(-1)])
            .is_err()
    );
    assert_eq!(vector.GetAt(5).unwrap().0, 0x8000_0000);

    let reference =
        crate::box_ireference(typ.enum_value(0xffff_ffff).unwrap(), typ.clone()).unwrap();
    assert_eq!(
        reference
            .as_object()
            .unwrap()
            .cast::<IReference<GamepadButtons>>()
            .unwrap()
            .Value()
            .unwrap()
            .0,
        u32::MAX
    );
    let primitive = crate::box_ireference(WinRTValue::U32(0x8000_0000), typ.clone()).unwrap();
    assert_eq!(
        primitive
            .as_object()
            .unwrap()
            .cast::<IReference<GamepadButtons>>()
            .unwrap()
            .Value()
            .unwrap()
            .0,
        0x8000_0000
    );
    assert!(crate::box_ireference(WinRTValue::I32(-1), typ.clone()).is_err());

    let entries = [(
        typ.enum_value(0x8000_0000).unwrap(),
        typ.enum_value(0xffff_ffff).unwrap(),
    )];
    let map = crate::map::create_map_from_values(&entries, &typ, &typ, table.map_iids(&typ, &typ))
        .unwrap();
    let map: IMap<GamepadButtons, GamepadButtons> = map.cast().unwrap();
    assert_eq!(map.Lookup(GamepadButtons(0x8000_0000)).unwrap().0, u32::MAX);
    assert_eq!(
        map.GetView()
            .unwrap()
            .Lookup(GamepadButtons(0x8000_0000))
            .unwrap()
            .0,
        u32::MAX
    );
}

#[test]
fn enum_arrays_preserve_unsigned_storage_and_reject_signed_aliases() {
    let table = MetadataTable::new();
    let typ = flags(&table);
    let values = [0x8000_0000, 0xffff_ffff];
    let array = ArrayData::from_values(
        typ.clone(),
        &values.map(|value| typ.enum_value(value).unwrap()),
    );
    assert_eq!(array.get_u32(0).unwrap(), 0x8000_0000);
    assert_eq!(array.get_u32(1).unwrap(), u32::MAX);
    assert!(array.get_i32(0).is_err());
    assert!(array.get_u32(2).is_err());
    assert!(crate::native_call::array_element_types_match(
        &typ,
        &table.u32_type()
    ));
    assert!(!crate::native_call::array_element_types_match(
        &typ,
        &table.i32_type()
    ));

    let raw = crate::vector::create_vector_from_values(&[], &typ, table.vector_iids(&typ)).unwrap();
    let vector: IVector<GamepadButtons> = raw.cast().unwrap();
    let replace = MethodSignature::new(&table)
        .add_in(table.array(&typ))
        .build(17);
    let input = WinRTValue::Array(ArrayData::from_values(
        table.u32_type(),
        &[WinRTValue::U32(0x8000_0000), WinRTValue::U32(u32::MAX)],
    ));
    replace.call_dynamic(vector.as_raw(), &[input]).unwrap();
    assert_eq!(vector.GetAt(1).unwrap().0, u32::MAX);
    let fill = MethodSignature::new(&table)
        .add_in(table.u32_type())
        .add_out_fill(table.array(&typ))
        .add_out(table.u32_type())
        .build(16);
    let capacity = WinRTValue::Array(ArrayData::from_values(
        table.u32_type(),
        &[WinRTValue::U32(0), WinRTValue::U32(0)],
    ));
    let outputs = fill
        .call_dynamic(vector.as_raw(), &[WinRTValue::U32(0), capacity])
        .unwrap();
    let returned = outputs[0].as_array().unwrap();
    assert_eq!(returned.get_u32(0).unwrap(), 0x8000_0000);
    assert_eq!(returned.get_u32(1).unwrap(), u32::MAX);
    let wrong = WinRTValue::Array(ArrayData::from_values(
        table.i32_type(),
        &[WinRTValue::I32(-1)],
    ));
    assert!(replace.call_dynamic(vector.as_raw(), &[wrong]).is_err());
}

#[test]
fn enum_callback_scalar_struct_and_array_plans_preserve_high_bits() {
    let table = MetadataTable::new();
    let typ = flags(&table);
    let pair = table.struct_type("Test.EnumPair", &[typ.clone(), table.i32_type()]);
    let kinds = [typ.clone(), pair.clone(), table.array(&typ)];
    let signatures: Vec<_> = kinds
        .iter()
        .map(|kind| {
            MethodSignature::new(&table)
                .add_in(kind.clone())
                .add_out(kind.clone())
        })
        .collect();
    let iid = GUID::from_u128(0x493fbc81_57a7_4ed2_abed_bfa8b4a1ef55);
    let definitions = vec![WinRtInterfaceDefinition {
        name: "Test.IEnumProbe".into(),
        interface_type: table.interface(iid),
        required_iids: vec![],
        methods: signatures
            .iter()
            .enumerate()
            .map(|(index, signature)| WinRtMethodDefinition {
                name: format!("Echo{index}"),
                vtable_index: 6 + index,
                signature: signature.clone(),
            })
            .collect(),
    }];
    let plan =
        WinRtImplementationPlan::new(definitions, WinRtThreadingPolicy::OwnerThread).unwrap();
    let mut owner =
        WinRtImplementation::new(plan, Arc::new(|_, _, inputs| Ok(inputs.to_vec())), None).unwrap();
    let object = owner
        .to_value()
        .unwrap()
        .cast(&iid)
        .unwrap()
        .as_object()
        .unwrap();
    let mut structure = pair.default_value();
    structure.set_field(0, u32::MAX);
    structure.set_field(1, -7i32);
    let inputs = [
        typ.enum_value(0xffff_ffff).unwrap(),
        WinRTValue::Struct(structure),
        WinRTValue::Array(ArrayData::from_values(
            typ.clone(),
            &[typ.enum_value(0x8000_0000).unwrap()],
        )),
    ];
    for (index, (signature, input)) in signatures.into_iter().zip(inputs).enumerate() {
        let returned = signature
            .build(6 + index)
            .call_dynamic(object.as_raw(), &[input])
            .unwrap();
        match index {
            0 => assert_eq!(returned[0].as_enum_number(), Some(0xffff_ffff)),
            1 => {
                let fields = returned[0].as_struct().unwrap();
                assert_eq!(fields.get_field::<u32>(0), u32::MAX);
                assert_eq!(fields.get_field::<i32>(1), -7);
            }
            2 => assert_eq!(
                returned[0].as_array().unwrap().get_u32(0).unwrap(),
                0x8000_0000
            ),
            _ => unreachable!(),
        }
    }
    owner.dispose().unwrap();
}

#[test]
fn enum_delegate_callbacks_preserve_unsigned_values() {
    let table = MetadataTable::new();
    let typ = flags(&table);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let called = calls.clone();
    let delegate = crate::delegate::try_create_delegate_value(
        GUID::from_u128(0xc3a48481_02cc_4321_8b44_93fd8c14293a),
        vec![typ.clone(), table.u32_type()],
        Box::new(move |values| {
            assert_eq!(values[0].as_enum_number(), Some(0xffff_ffff));
            assert!(matches!(values[1], WinRTValue::U32(0x8000_0000)));
            called.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            windows_core::HRESULT(0)
        }),
    )
    .unwrap();
    MethodSignature::new(&table)
        .add_in(typ.clone())
        .add_in(table.u32_type())
        .build(3)
        .call_dynamic(
            delegate.as_object().unwrap().as_raw(),
            &[
                typ.enum_value(0xffff_ffff).unwrap(),
                WinRTValue::U32(0x8000_0000),
            ],
        )
        .unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn enum_fast_setter_uses_the_declared_unsigned_abi() {
    use std::sync::atomic::{AtomicU32, Ordering};

    let table = MetadataTable::new();
    let typ = flags(&table);
    let signatures = [
        MethodSignature::new(&table).add_in(typ.clone()),
        MethodSignature::new(&table).add_out(typ.clone()),
    ];
    let iid = GUID::from_u128(0x87f68938_c786_4cde_a649_1f0132aeb25d);
    let plan = WinRtImplementationPlan::new(
        vec![WinRtInterfaceDefinition {
            name: "Test.IEnumProperty".into(),
            interface_type: table.interface(iid),
            required_iids: vec![],
            methods: signatures
                .iter()
                .enumerate()
                .map(|(index, signature)| WinRtMethodDefinition {
                    name: format!("Property{index}"),
                    vtable_index: 6 + index,
                    signature: signature.clone(),
                })
                .collect(),
        }],
        WinRtThreadingPolicy::OwnerThread,
    )
    .unwrap();
    let stored = AtomicU32::new(0);
    let mut owner = WinRtImplementation::new(
        plan,
        Arc::new(move |_, slot, inputs| {
            if slot == 6 {
                stored.store(inputs[0].as_enum_number().unwrap() as u32, Ordering::SeqCst);
                Ok(vec![])
            } else {
                Ok(vec![
                    typ.enum_value(i64::from(stored.load(Ordering::SeqCst)))
                        .unwrap(),
                ])
            }
        }),
        None,
    )
    .unwrap();
    let object = owner
        .to_value()
        .unwrap()
        .cast(&iid)
        .unwrap()
        .as_object()
        .unwrap();
    let setter = signatures[0].clone().build(6);
    let getter = signatures[1].clone().build(7);
    setter.call_setter_u32(object.as_raw(), u32::MAX).unwrap();
    assert!(setter.call_setter_i32(object.as_raw(), -1).is_err());
    assert!(getter.call_getter_i32(object.as_raw()).is_err());
    assert_eq!(
        getter.call_dynamic(object.as_raw(), &[]).unwrap()[0].as_enum_number(),
        Some(0xffff_ffff)
    );
    owner.dispose().unwrap();
}
