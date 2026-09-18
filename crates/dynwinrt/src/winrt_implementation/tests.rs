// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::sync::atomic::{AtomicPtr, AtomicUsize};

use windows::{
    ApplicationModel::Background::{
        BackgroundTaskCanceledEventHandler, BackgroundTaskDeferral, BackgroundTaskRegistration,
        IBackgroundTask, IBackgroundTaskInstance, IBackgroundTaskInstance_Impl,
    },
    Foundation::{
        IClosable, IMemoryBufferReference, IReference, IStringable, MemoryBuffer,
        TypedEventHandler, Uri,
    },
    Win32::System::Com::CoTaskMemFree,
};

use crate::{ArrayData, MetadataTable, MethodSignature, TypeKind};

use super::*;

const IID_FIXTURE: GUID = GUID::from_u128(0x4ab7eea7_29a1_4f48_a6da_22894e210ec0);
const IID_OTHER: GUID = GUID::from_u128(0x443bc92c_8c7f_4ab0_a8b1_60a765bddb49);

fn definition(
    table: &Arc<MetadataTable>,
    name: &str,
    iid: GUID,
    signatures: Vec<MethodSignature>,
) -> WinRtInterfaceDefinition {
    WinRtInterfaceDefinition {
        name: name.into(),
        interface_type: table.interface(iid),
        required_iids: vec![],
        methods: signatures
            .into_iter()
            .enumerate()
            .map(|(index, signature)| WinRtMethodDefinition {
                name: format!("Method{index}"),
                vtable_index: 6 + index,
                signature,
            })
            .collect(),
    }
}

fn create(
    definitions: Vec<WinRtInterfaceDefinition>,
    callback: impl Fn(usize, usize, &[WinRTValue]) -> windows_core::Result<Vec<WinRTValue>>
    + Send
    + Sync
    + 'static,
) -> WinRtImplementation {
    WinRtImplementation::new(
        WinRtImplementationPlan::new(definitions, WinRtThreadingPolicy::OwnerThread).unwrap(),
        Arc::new(callback),
        Some("DynWinRt.Tests.Standalone"),
    )
    .unwrap()
}

fn query(owner: &WinRtImplementation, iid: &GUID) -> IUnknown {
    owner
        .to_value()
        .unwrap()
        .cast(iid)
        .unwrap()
        .as_object()
        .unwrap()
}

fn stringable_definition(table: &Arc<MetadataTable>) -> WinRtInterfaceDefinition {
    definition(
        table,
        "Windows.Foundation.IStringable",
        IStringable::IID,
        vec![MethodSignature::new(table).add_out(table.hstring())],
    )
}

#[test]
fn standalone_inspectable_has_canonical_identity_public_iids_and_weak_references() {
    let table = MetadataTable::new();
    let closes = Arc::new(AtomicUsize::new(0));
    let callback_closes = closes.clone();
    let mut owner = create(
        vec![
            stringable_definition(&table),
            definition(
                &table,
                "Windows.Foundation.IClosable",
                IClosable::IID,
                vec![MethodSignature::new(&table)],
            ),
        ],
        move |interface, slot, inputs| {
            assert_eq!(slot, 6);
            assert!(inputs.is_empty());
            if interface == 0 {
                Ok(vec![WinRTValue::HString("standalone".into())])
            } else {
                callback_closes.fetch_add(1, Ordering::SeqCst);
                Ok(vec![])
            }
        },
    );
    let canonical = owner.to_value().unwrap().as_object().unwrap();
    let stringable: IStringable = canonical.cast().unwrap();
    let closable: IClosable = canonical.cast().unwrap();
    assert_ne!(stringable.as_raw(), closable.as_raw());
    assert_eq!(stringable.cast::<IUnknown>().unwrap(), canonical);
    assert_eq!(closable.cast::<IUnknown>().unwrap(), canonical);
    let inspectable: IInspectable = closable.cast().unwrap();
    assert_eq!(inspectable.as_raw(), canonical.as_raw());
    assert_eq!(
        inspectable.GetRuntimeClassName().unwrap(),
        "DynWinRt.Tests.Standalone"
    );
    assert_eq!(inspectable.GetTrustLevel().unwrap(), 0);
    let mut count = 0;
    let mut iids = ptr::null_mut();
    unsafe {
        (inspectable.vtable().GetIids)(inspectable.as_raw(), &mut count, &mut iids)
            .ok()
            .unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            &*ptr::slice_from_raw_parts(iids, count as usize),
            &[IStringable::IID, IClosable::IID]
        );
        CoTaskMemFree(Some(iids.cast()));
    }
    assert!(canonical.cast::<windows_core::imp::IAgileObject>().is_err());
    assert!(canonical.cast::<windows_core::imp::IMarshal>().is_err());
    assert_eq!(stringable.ToString().unwrap(), "standalone");
    closable.Close().unwrap();
    assert_eq!(closes.load(Ordering::SeqCst), 1);
    let weak = stringable.downgrade().unwrap();
    owner.release();
    assert_eq!(weak.upgrade().unwrap().ToString().unwrap(), "standalone");
    drop(inspectable);
    drop(canonical);
    drop(closable);
    drop(stringable);
    assert!(weak.upgrade().is_none());
    assert!(owner.is_closed());
}

#[test]
fn release_and_dispose_have_distinct_native_retention_semantics() {
    let table = MetadataTable::new();
    let mut owner = create(vec![stringable_definition(&table)], |_, _, _| {
        Ok(vec![WinRTValue::HString("retained".into())])
    });
    assert!(owner.has_unique_native_reference());
    let retained: IStringable = query(&owner, &IStringable::IID).cast().unwrap();
    assert!(!owner.has_unique_native_reference());
    owner.release();
    assert!(!owner.is_closed());
    assert_eq!(retained.ToString().unwrap(), "retained");
    assert_eq!(owner.to_value().unwrap_err().code(), RO_E_CLOSED);
    owner.dispose().unwrap();
    owner.dispose().unwrap();
    assert!(owner.is_closed());
    assert_eq!(retained.ToString().unwrap_err().code(), RO_E_CLOSED);
    assert!(owner.take_error().unwrap().contains("disconnected"));
    assert!(owner.take_error().is_none());
}

#[test]
fn foreign_thread_callbacks_reject_before_language_entry_and_zero_outputs() {
    let table = MetadataTable::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let callback_calls = calls.clone();
    let owner = create(vec![stringable_definition(&table)], move |_, _, _| {
        callback_calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![WinRTValue::HString("owner thread".into())])
    });
    let value = owner.to_value().unwrap();
    thread::spawn(move || {
        let object = value.as_object().unwrap();
        let stringable: IStringable = object.cast().unwrap();
        let mut output = ptr::null_mut();
        let hr = unsafe { (stringable.vtable().ToString)(stringable.as_raw(), &mut output) };
        assert_eq!(hr, RPC_E_WRONG_THREAD);
        assert!(output.is_null());
        assert_eq!(
            object
                .cast::<IInspectable>()
                .unwrap()
                .GetTrustLevel()
                .unwrap(),
            0
        );
    })
    .join()
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(owner.take_error().unwrap().contains("creating thread"));
    let stringable: IStringable = query(&owner, &IStringable::IID).cast().unwrap();
    assert_eq!(stringable.ToString().unwrap(), "owner thread");
}

#[test]
fn callback_errors_and_panics_are_hresult_failures() {
    let table = MetadataTable::new();
    for panic in [false, true] {
        let owner = create(vec![stringable_definition(&table)], move |_, _, _| {
            if panic {
                panic!("controlled callback panic");
            }
            Err(error(E_INVALIDARG, "controlled callback exception"))
        });
        let stringable: IStringable = query(&owner, &IStringable::IID).cast().unwrap();
        let mut output = ptr::null_mut();
        let hr = unsafe { (stringable.vtable().ToString)(stringable.as_raw(), &mut output) };
        assert_eq!(hr, if panic { E_FAIL } else { E_INVALIDARG });
        assert!(output.is_null());
        let message = owner.take_error().unwrap();
        assert!(message.contains(if panic {
            "Rust panic"
        } else {
            "controlled callback exception"
        }));
    }
}

struct DropCounter(Arc<AtomicUsize>);

impl Drop for DropCounter {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn final_reentrant_release_keeps_static_and_dynamic_dispatch_alive() {
    let table = MetadataTable::new();
    for dynamic in [false, true] {
        let destroyed = Arc::new(AtomicUsize::new(0));
        let counter = DropCounter(destroyed.clone());
        let raw_reference = Arc::new(AtomicPtr::new(ptr::null_mut::<c_void>()));
        let callback_raw = raw_reference.clone();
        let signature = if dynamic {
            MethodSignature::new(&table)
                .add_in(table.f64_type())
                .add_out(table.hstring())
        } else {
            MethodSignature::new(&table).add_out(table.hstring())
        };
        let mut owner = create(
            vec![definition(
                &table,
                "Test.IReentrant",
                IID_FIXTURE,
                vec![signature.clone()],
            )],
            move |_, _, inputs| {
                assert_eq!(inputs.len(), usize::from(dynamic));
                let raw = callback_raw.swap(ptr::null_mut(), Ordering::SeqCst);
                assert!(!raw.is_null());
                drop(unsafe { IUnknown::from_raw(raw) });
                assert_eq!(counter.0.load(Ordering::SeqCst), 0);
                Ok(vec![WinRTValue::HString("completed after Release".into())])
            },
        );
        let raw = query(&owner, &IID_FIXTURE).into_raw();
        raw_reference.store(raw, Ordering::SeqCst);
        owner.release();
        let outputs = signature
            .build(6)
            .call_dynamic(
                raw,
                if dynamic {
                    &[WinRTValue::F64(42.5)]
                } else {
                    &[]
                },
            )
            .unwrap();
        assert_eq!(outputs[0].as_hstring().unwrap(), "completed after Release");
        assert_eq!(destroyed.load(Ordering::SeqCst), 1);
        assert!(owner.is_closed());
    }
}

#[test]
fn reentrant_disconnection_allows_current_callback_to_finish() {
    let table = MetadataTable::new();
    let control = Arc::new(Mutex::new(None::<std::sync::Weak<Control>>));
    let callback_control = control.clone();
    let owner = create(vec![stringable_definition(&table)], move |_, _, _| {
        let control = callback_control
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap();
        control.disconnect().unwrap();
        Ok(vec![WinRTValue::HString("already entered".into())])
    });
    *control.lock().unwrap() = Some(Arc::downgrade(&owner.control));
    let stringable: IStringable = query(&owner, &IStringable::IID).cast().unwrap();
    assert_eq!(stringable.ToString().unwrap(), "already entered");
    assert_eq!(stringable.ToString().unwrap_err().code(), RO_E_CLOSED);
}

#[test]
fn multi_output_conversion_is_atomic_and_releases_prepared_references() {
    let table = MetadataTable::new();
    let destroyed = Arc::new(AtomicUsize::new(0));
    let counter = DropCounter(destroyed.clone());
    let mut returned = create(vec![stringable_definition(&table)], move |_, _, _| {
        let _ = &counter;
        Ok(vec![WinRTValue::HString("child".into())])
    });
    let returned_value = returned.to_value().unwrap();
    let pending = Arc::new(Mutex::new(Some(returned_value)));
    returned.release();
    let signature = MethodSignature::new(&table)
        .add_out(table.interface(IStringable::IID))
        .add_out(table.hstring())
        .add_out(table.u32_type());
    let callback_pending = pending.clone();
    let owner = create(
        vec![definition(
            &table,
            "Test.IAtomicOutputs",
            IID_FIXTURE,
            vec![signature],
        )],
        move |_, _, _| {
            Ok(vec![
                callback_pending.lock().unwrap().take().unwrap(),
                WinRTValue::HString("prepared string".into()),
                WinRTValue::Bool(true),
            ])
        },
    );
    let object = query(&owner, &IID_FIXTURE);
    let function: unsafe extern "system" fn(
        *mut c_void,
        *mut *mut c_void,
        *mut *mut c_void,
        *mut u32,
    ) -> HRESULT =
        unsafe { std::mem::transmute(*(*(object.as_raw().cast::<*const *const c_void>())).add(6)) };
    let mut reference = ptr::null_mut();
    let mut string = ptr::null_mut();
    let mut number = 99;
    assert_eq!(
        unsafe { function(object.as_raw(), &mut reference, &mut string, &mut number) },
        E_INVALIDARG
    );
    assert!(reference.is_null());
    assert!(string.is_null());
    assert_eq!(number, 0);
    assert_eq!(destroyed.load(Ordering::SeqCst), 1);
    assert!(owner.take_error().unwrap().contains("expected u4"));
}

#[test]
fn primitives_guid_structs_and_nullable_references_cross_native_vtables() {
    let table = MetadataTable::new();
    let point = table.struct_type("Test.Point", &[table.f32_type(), table.f32_type()]);
    let holder = table.struct_type(
        "Test.Holder",
        &[point.clone(), table.hstring(), table.object()],
    );
    let uri = Uri::CreateUri(&HSTRING::from("https://example.com/borrowed")).unwrap();
    let mut point_value = point.default_value();
    point_value.set_field(0, 3.5f32);
    point_value.set_field(1, -8.25f32);
    let mut holder_value = holder.default_value();
    holder_value.set_field_struct(0, &point_value);
    holder_value
        .set_field_hstring(1, "retained field".into())
        .unwrap();
    holder_value
        .set_field_object(2, Some(&uri.cast().unwrap()))
        .unwrap();
    let enum_type = table.enum_type("Test.Mode", vec![("A".into(), 3)]);
    let shapes = vec![
        (table.bool_type(), WinRTValue::Bool(true)),
        (table.i8_type(), WinRTValue::I8(-7)),
        (table.u8_type(), WinRTValue::U8(250)),
        (table.i16_type(), WinRTValue::I16(-300)),
        (table.char16_type(), WinRTValue::U16(0x263a)),
        (table.i32_type(), WinRTValue::I32(-42)),
        (table.hresult(), WinRTValue::HResult(E_FAIL)),
        (table.u32_type(), WinRTValue::U32(4_000_000_000)),
        (table.i64_type(), WinRTValue::I64(i64::MIN + 10)),
        (table.u64_type(), WinRTValue::U64(u64::MAX - 10)),
        (table.f32_type(), WinRTValue::F32(3.125)),
        (table.f64_type(), WinRTValue::F64(-1.75)),
        (table.guid_type(), WinRTValue::Guid(IID_FIXTURE)),
        (table.hstring(), WinRTValue::HString("owned string".into())),
        (enum_type, WinRTValue::I32(3)),
        (holder.clone(), WinRTValue::Struct(holder_value)),
        (table.object(), WinRTValue::Object(uri.cast().unwrap())),
        (table.object(), WinRTValue::Null),
    ];
    for (typ, value) in shapes {
        let signature = MethodSignature::new(&table)
            .add_in(typ.clone())
            .add_out(typ.clone());
        let captured = Arc::new(Mutex::new(None));
        let callback_captured = captured.clone();
        let owner = create(
            vec![definition(
                &table,
                "Test.IEcho",
                IID_FIXTURE,
                vec![signature.clone()],
            )],
            move |_, _, inputs| {
                *callback_captured.lock().unwrap() = Some(inputs[0].clone());
                Ok(vec![inputs[0].clone()])
            },
        );
        let object = query(&owner, &IID_FIXTURE);
        let outputs = signature
            .build(6)
            .call_dynamic(object.as_raw(), &[value])
            .unwrap();
        let retained = captured.lock().unwrap().take().unwrap();
        match (typ.kind(), &outputs[0], retained) {
            (TypeKind::Struct(_), WinRTValue::Struct(output), WinRTValue::Struct(retained)) => {
                assert_eq!(output.get_field_hstring(1).unwrap(), "retained field");
                assert_eq!(retained.get_field_struct(0).get_field::<f32>(1), -8.25);
                assert_eq!(
                    output
                        .get_field_object(2)
                        .unwrap()
                        .unwrap()
                        .cast::<Uri>()
                        .unwrap()
                        .Host()
                        .unwrap(),
                    "example.com"
                );
            }
            (TypeKind::Object, WinRTValue::Null, WinRTValue::Null) => {}
            (TypeKind::Object, WinRTValue::Object(output), WinRTValue::Object(retained)) => {
                assert_eq!(
                    output.cast::<IUnknown>().unwrap(),
                    retained.cast::<IUnknown>().unwrap()
                );
            }
            (_, output, retained) => assert_eq!(format!("{output:?}"), format!("{retained:?}")),
        }
    }
}

#[test]
fn pass_receive_and_fill_arrays_preserve_element_ownership() {
    let table = MetadataTable::new();
    let record = table.struct_type("Test.ArrayRecord", &[table.hstring(), table.u64_type()]);
    let mut record_value = record.default_value();
    record_value
        .set_field_hstring(0, "array record".into())
        .unwrap();
    record_value.set_field(1, u64::MAX - 3);
    let object = Uri::CreateUri(&HSTRING::from("https://example.com/array")).unwrap();
    let cases = [
        (
            table.u8_type(),
            vec![WinRTValue::U8(1), WinRTValue::U8(255)],
        ),
        (
            table.f64_type(),
            vec![WinRTValue::F64(0.25), WinRTValue::F64(-2.5)],
        ),
        (
            table.guid_type(),
            vec![WinRTValue::Guid(IID_FIXTURE), WinRTValue::Guid(IID_OTHER)],
        ),
        (
            table.hstring(),
            vec![
                WinRTValue::HString("first".into()),
                WinRTValue::HString("second".into()),
            ],
        ),
        (
            table.object(),
            vec![WinRTValue::Object(object.cast().unwrap()), WinRTValue::Null],
        ),
        (record, vec![WinRTValue::Struct(record_value)]),
    ];
    for (element, values) in cases {
        for empty in [false, true] {
            let input = ArrayData::from_values(element.clone(), if empty { &[] } else { &values });
            let fill = ArrayData::from_values(element.clone(), if empty { &[] } else { &values });
            let array_type = table.array(&element);
            let signature = MethodSignature::new(&table)
                .add_in(array_type.clone())
                .add_out(array_type.clone())
                .add_out_fill(array_type);
            let owner = create(
                vec![definition(
                    &table,
                    "Test.IArrays",
                    IID_FIXTURE,
                    vec![signature.clone()],
                )],
                |_, _, inputs| {
                    let [WinRTValue::Array(array), WinRTValue::U32(capacity)] = inputs else {
                        panic!("expected array plus fill capacity");
                    };
                    assert_eq!(array.len(), *capacity as usize);
                    Ok(vec![
                        WinRTValue::Array(array.clone()),
                        WinRTValue::Array(array.clone()),
                    ])
                },
            );
            let view = query(&owner, &IID_FIXTURE);
            let result = signature
                .build(6)
                .call_dynamic(
                    view.as_raw(),
                    &[WinRTValue::Array(input), WinRTValue::Array(fill)],
                )
                .unwrap();
            assert_eq!(result.len(), 2);
            for output in result {
                let WinRTValue::Array(array) = output else {
                    panic!("array output expected")
                };
                assert_eq!(array.len(), if empty { 0 } else { values.len() });
                for index in 0..array.len() {
                    match (array.get(index), &values[index]) {
                        (WinRTValue::Struct(output), WinRTValue::Struct(expected)) => {
                            assert_eq!(
                                output.get_field_hstring(0).unwrap(),
                                expected.get_field_hstring(0).unwrap()
                            );
                            assert_eq!(output.get_field::<u64>(1), expected.get_field::<u64>(1));
                        }
                        (WinRTValue::Object(output), WinRTValue::Object(expected)) => {
                            assert_eq!(
                                output.cast::<IUnknown>().unwrap(),
                                expected.cast::<IUnknown>().unwrap()
                            );
                        }
                        (output, expected) => {
                            assert_eq!(format!("{output:?}"), format!("{expected:?}"))
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn invalid_outputs_never_publish_partial_arrays() {
    let table = MetadataTable::new();
    let signature = MethodSignature::new(&table)
        .add_out(table.array(&table.hstring()))
        .add_out(table.u32_type());
    let owner = create(
        vec![definition(
            &table,
            "Test.IFailedArray",
            IID_FIXTURE,
            vec![signature],
        )],
        move |_, _, _| {
            Ok(vec![
                WinRTValue::Array(ArrayData::from_values(
                    table.hstring(),
                    &[WinRTValue::HString("first".into()), WinRTValue::Bool(true)],
                )),
                WinRTValue::U32(4),
            ])
        },
    );
    let view = query(&owner, &IID_FIXTURE);
    let function: unsafe extern "system" fn(
        *mut c_void,
        *mut u32,
        *mut *mut c_void,
        *mut u32,
    ) -> HRESULT =
        unsafe { std::mem::transmute(*(*(view.as_raw().cast::<*const *const c_void>())).add(6)) };
    let mut length = 99;
    let mut data = ptr::null_mut();
    let mut number = 99;
    let hr = unsafe { function(view.as_raw(), &mut length, &mut data, &mut number) };
    assert_eq!(hr, E_INVALIDARG);
    assert_eq!(length, 0);
    assert!(data.is_null());
    assert_eq!(number, 0);
}

#[test]
fn null_output_slots_are_rejected_after_initializing_other_outputs() {
    let table = MetadataTable::new();
    let signature = MethodSignature::new(&table)
        .add_out(table.i32_type())
        .add_out(table.u64_type());
    let owner = create(
        vec![definition(
            &table,
            "Test.INullOutputs",
            IID_FIXTURE,
            vec![signature],
        )],
        |_, _, _| panic!("null outputs must fail before dispatch"),
    );
    let view = query(&owner, &IID_FIXTURE);
    let function: unsafe extern "system" fn(*mut c_void, *mut i32, *mut u64) -> HRESULT =
        unsafe { std::mem::transmute(*(*(view.as_raw().cast::<*const *const c_void>())).add(6)) };
    let mut output = u64::MAX;
    assert_eq!(
        unsafe { function(view.as_raw(), ptr::null_mut(), &mut output) },
        E_POINTER
    );
    assert_eq!(output, 0);
}

#[test]
fn invalid_and_incomplete_descriptors_fail_before_publication() {
    let table = MetadataTable::new();
    let validate = |definition: WinRtInterfaceDefinition| {
        WinRtImplementationPlan::new(vec![definition], WinRtThreadingPolicy::OwnerThread)
    };
    let mut missing_slot = stringable_definition(&table);
    missing_slot.methods[0].vtable_index = 7;
    assert!(
        validate(missing_slot)
            .unwrap_err()
            .message()
            .contains("contiguous slot")
    );
    let mut required = stringable_definition(&table);
    required.required_iids.push(IClosable::IID);
    assert!(
        validate(required)
            .unwrap_err()
            .message()
            .contains("requires a separate")
    );
    let mut cycle = stringable_definition(&table);
    cycle.required_iids.push(IStringable::IID);
    assert!(validate(cycle).unwrap_err().message().contains("cyclic"));
    let mut generic = stringable_definition(&table);
    generic.interface_type = table.generic(crate::metadata_table::IVECTOR, 1);
    assert!(
        validate(generic)
            .unwrap_err()
            .message()
            .contains("non-generic")
    );
    for typ in [
        table.generic(crate::metadata_table::IVECTOR, 1),
        table.array(&table.array(&table.u8_type())),
        table.struct_type("Test.Empty", &[]),
        table.interface(GUID::zeroed()),
    ] {
        assert!(
            validate(definition(
                &table,
                "Test.IUnsupported",
                IID_FIXTURE,
                vec![MethodSignature::new(&table).add_in(typ)],
            ))
            .is_err()
        );
    }
    assert!(
        validate(definition(
            &table,
            "Test.IBadFill",
            IID_FIXTURE,
            vec![MethodSignature::new(&table).add_out_fill(table.u8_type())],
        ))
        .unwrap_err()
        .message()
        .contains("FillArray")
    );
    for iid in [
        GUID::zeroed(),
        IUnknown::IID,
        IInspectable::IID,
        windows_core::imp::IAgileObject::IID,
        windows_core::imp::IMarshal::IID,
    ] {
        assert!(validate(definition(&table, "Test.IReserved", iid, vec![])).is_err());
    }
    assert!(
        WinRtImplementationPlan::new(
            vec![stringable_definition(&table), stringable_definition(&table)],
            WinRtThreadingPolicy::OwnerThread,
        )
        .unwrap_err()
        .message()
        .contains("Duplicate")
    );
    assert!(
        values::ValuePlan::new(table.guid_type())
            .unwrap()
            .array_bytes(usize::MAX)
            .is_err()
    );
}

#[test]
fn required_interfaces_use_independent_complete_vtables() {
    let table = MetadataTable::new();
    let mut first = stringable_definition(&table);
    first.required_iids.push(IClosable::IID);
    let owner = create(
        vec![
            first,
            definition(
                &table,
                "Windows.Foundation.IClosable",
                IClosable::IID,
                vec![MethodSignature::new(&table)],
            ),
        ],
        |index, _, _| {
            if index == 0 {
                Ok(vec![WinRTValue::HString("required views".into())])
            } else {
                Ok(vec![])
            }
        },
    );
    let first: IStringable = query(&owner, &IStringable::IID).cast().unwrap();
    let required: IClosable = first.cast().unwrap();
    assert_ne!(first.as_raw(), required.as_raw());
    assert_eq!(first.ToString().unwrap(), "required views");
    required.Close().unwrap();
    assert_eq!(
        first.cast::<IUnknown>().unwrap(),
        required.cast::<IUnknown>().unwrap()
    );
}

#[test]
fn struct_outputs_require_exact_type_identity() {
    let table = MetadataTable::new();
    let expected = table.struct_type("Test.Expected", &[table.f64_type()]);
    let actual = table.struct_type("Test.Actual", &[table.f64_type()]);
    let value = WinRTValue::Struct(actual.default_value());
    let signature = MethodSignature::new(&table).add_out(expected);
    let owner = create(
        vec![definition(
            &table,
            "Test.IStructOutput",
            IID_FIXTURE,
            vec![signature.clone()],
        )],
        move |_, _, _| Ok(vec![value.clone()]),
    );
    let view = query(&owner, &IID_FIXTURE);
    assert_eq!(
        signature
            .build(6)
            .call_dynamic(view.as_raw(), &[])
            .unwrap_err()
            .code(),
        E_INVALIDARG
    );
}

#[test]
fn immutable_winrt_plans_and_host_state_are_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<WinRtImplementation>();
    assert_send_sync::<WinRtImplementationPlan>();
    assert_send_sync::<Host>();
    assert_send_sync::<Control>();
}

#[windows_core::implement(IBackgroundTaskInstance)]
struct BackgroundInstance {
    progress: Arc<AtomicUsize>,
}

impl IBackgroundTaskInstance_Impl for BackgroundInstance_Impl {
    fn InstanceId(&self) -> windows_core::Result<GUID> {
        Ok(IID_FIXTURE)
    }

    fn Task(&self) -> windows_core::Result<BackgroundTaskRegistration> {
        Err(error(
            crate::com_helpers::E_NOTIMPL,
            "Fixture has no OS task registration",
        ))
    }

    fn Progress(&self) -> windows_core::Result<u32> {
        Ok(self.progress.load(Ordering::SeqCst) as u32)
    }

    fn SetProgress(&self, value: u32) -> windows_core::Result<()> {
        self.progress.store(value as usize, Ordering::SeqCst);
        Ok(())
    }

    fn TriggerDetails(&self) -> windows_core::Result<IInspectable> {
        Err(error(
            crate::com_helpers::E_NOTIMPL,
            "Fixture has no OS trigger",
        ))
    }

    fn Canceled(
        &self,
        _: windows_core::Ref<BackgroundTaskCanceledEventHandler>,
    ) -> windows_core::Result<i64> {
        Err(error(
            crate::com_helpers::E_NOTIMPL,
            "Fixture has no cancellation source",
        ))
    }

    fn RemoveCanceled(&self, _: i64) -> windows_core::Result<()> {
        Err(error(
            crate::com_helpers::E_NOTIMPL,
            "Fixture has no cancellation source",
        ))
    }

    fn SuspendedCount(&self) -> windows_core::Result<u32> {
        Ok(0)
    }

    fn GetDeferral(&self) -> windows_core::Result<BackgroundTaskDeferral> {
        Err(error(
            crate::com_helpers::E_NOTIMPL,
            "Fixture does not create OS deferrals",
        ))
    }
}

#[test]
fn background_task_uses_the_same_generic_host_and_real_native_caller() {
    let table = MetadataTable::new();
    let retained = Arc::new(Mutex::new(None::<WinRTValue>));
    let callback_retained = retained.clone();
    let owner = create(
        vec![definition(
            &table,
            "Windows.ApplicationModel.Background.IBackgroundTask",
            IBackgroundTask::IID,
            vec![
                MethodSignature::new(&table).add_in(table.interface(IBackgroundTaskInstance::IID)),
            ],
        )],
        move |_, _, inputs| {
            let task: IBackgroundTaskInstance = inputs[0].as_object().unwrap().cast()?;
            assert_eq!(task.InstanceId()?, IID_FIXTURE);
            task.SetProgress(73)?;
            *callback_retained.lock().unwrap() = Some(inputs[0].clone());
            Ok(vec![])
        },
    );
    let progress = Arc::new(AtomicUsize::new(0));
    let instance: IBackgroundTaskInstance = BackgroundInstance {
        progress: progress.clone(),
    }
    .into();
    let task: IBackgroundTask = query(&owner, &IBackgroundTask::IID).cast().unwrap();
    task.Run(&instance).unwrap();
    drop(instance);
    assert_eq!(progress.load(Ordering::SeqCst), 73);
    let retained: IBackgroundTaskInstance = retained
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .as_object()
        .unwrap()
        .cast()
        .unwrap();
    assert_eq!(retained.Progress().unwrap(), 73);
}

#[test]
fn properties_and_event_accessors_bridge_native_delegate_values() {
    type ClosedHandler = TypedEventHandler<IMemoryBufferReference, IInspectable>;
    let table = MetadataTable::new();
    let mut reference = definition(
        &table,
        "Windows.Foundation.IMemoryBufferReference",
        IMemoryBufferReference::IID,
        vec![
            MethodSignature::new(&table).add_out(table.u32_type()),
            MethodSignature::new(&table)
                .add_in(table.delegate(ClosedHandler::IID))
                .add_out(table.i64_type()),
            MethodSignature::new(&table).add_in(table.i64_type()),
        ],
    );
    reference.required_iids.push(IClosable::IID);
    let current_handler = Arc::new(Mutex::new(None::<WinRTValue>));
    let callback_handler = current_handler.clone();
    let sender = MemoryBuffer::Create(24).unwrap().CreateReference().unwrap();
    let callback_sender = WinRTValue::Object(sender.cast().unwrap());
    let owner = create(
        vec![
            reference,
            definition(
                &table,
                "Windows.Foundation.IClosable",
                IClosable::IID,
                vec![MethodSignature::new(&table)],
            ),
        ],
        move |interface, slot, inputs| {
            if interface == 1 {
                let handler = callback_handler.lock().unwrap().clone();
                if let Some(handler) = handler {
                    let handler: ClosedHandler = handler.as_object().unwrap().cast()?;
                    let sender: IMemoryBufferReference =
                        callback_sender.as_object().unwrap().cast()?;
                    handler.Invoke(&sender, None)?;
                }
                return Ok(vec![]);
            }
            match slot {
                6 => Ok(vec![WinRTValue::U32(64)]),
                7 => {
                    *callback_handler.lock().unwrap() = Some(inputs[0].clone());
                    Ok(vec![WinRTValue::I64(42)])
                }
                8 => {
                    assert!(matches!(inputs, [WinRTValue::I64(42)]));
                    *callback_handler.lock().unwrap() = None;
                    Ok(vec![])
                }
                _ => Err(error(
                    crate::com_helpers::E_NOTIMPL,
                    "Unexpected fixture slot",
                )),
            }
        },
    );
    let reference: IMemoryBufferReference =
        query(&owner, &IMemoryBufferReference::IID).cast().unwrap();
    assert_eq!(reference.Capacity().unwrap(), 64);
    let received = Arc::new(AtomicUsize::new(0));
    let callback_received = received.clone();
    let handler = ClosedHandler::new(move |sender, _| {
        callback_received.fetch_add(
            sender.as_ref().unwrap().Capacity()? as usize,
            Ordering::SeqCst,
        );
        Ok(())
    });
    let token = reference.Closed(&handler).unwrap();
    drop(handler);
    reference.Close().unwrap();
    assert_eq!(received.load(Ordering::SeqCst), 24);
    reference.RemoveClosed(token).unwrap();
    reference.Close().unwrap();
    assert_eq!(received.load(Ordering::SeqCst), 24);
    assert!(current_handler.lock().unwrap().is_none());
}

#[test]
fn closed_generic_nullable_values_pass_without_implementing_generic_interfaces() {
    let table = MetadataTable::new();
    let generic = table.generic(crate::metadata_table::IREFERENCE, 1);
    let reference = table.parameterized(&generic, &[table.u32_type()]);
    let signature = MethodSignature::new(&table)
        .add_in(reference.clone())
        .add_out(reference);
    let owner = create(
        vec![definition(
            &table,
            "Test.IOptionalValue",
            IID_FIXTURE,
            vec![signature.clone()],
        )],
        |_, _, inputs| Ok(vec![inputs[0].clone()]),
    );
    let view = query(&owner, &IID_FIXTURE);
    let method = signature.build(6);
    let boxed = crate::box_ireference(WinRTValue::U32(19), table.u32_type()).unwrap();
    let outputs = method.call_dynamic(view.as_raw(), &[boxed]).unwrap();
    let value: IReference<u32> = outputs[0].as_object().unwrap().cast().unwrap();
    assert_eq!(value.Value().unwrap(), 19);
    assert!(matches!(
        method
            .call_dynamic(view.as_raw(), &[WinRTValue::Null])
            .unwrap()[0],
        WinRTValue::Null
    ));
}

#[test]
fn established_projection_aliases_are_checked_and_preserved_in_outputs() {
    let table = MetadataTable::new();
    for (typ, number) in [
        (table.i8_type(), -127),
        (table.u8_type(), 255),
        (table.char16_type(), 0x263a),
    ] {
        let plan = values::ValuePlan::new(typ).unwrap();
        assert!(plan.prepare(&WinRTValue::I32(number)).is_ok());
        assert!(plan.prepare(&WinRTValue::I32(i32::MAX)).is_err());
    }

    let other_table = MetadataTable::new();
    let enum_type = table.enum_type("Test.ArrayAliasEnum", vec![]);
    for (expected, actual, values) in [
        (
            table.u32_type(),
            other_table.u32_type(),
            vec![WinRTValue::U32(8)],
        ),
        (
            table.u16_type(),
            table.char16_type(),
            vec![WinRTValue::U16(65)],
        ),
        (
            table.char16_type(),
            table.u16_type(),
            vec![WinRTValue::U16(66)],
        ),
        (enum_type, table.i32_type(), vec![WinRTValue::I32(3)]),
    ] {
        let signature = MethodSignature::new(&table).add_out(table.array(&expected));
        let result = WinRTValue::Array(ArrayData::from_values(actual, &values));
        let owner = create(
            vec![definition(
                &table,
                "Test.IArrayAlias",
                IID_FIXTURE,
                vec![signature.clone()],
            )],
            move |_, _, _| Ok(vec![result.clone()]),
        );
        let view = query(&owner, &IID_FIXTURE);
        assert!(signature.build(6).call_dynamic(view.as_raw(), &[]).is_ok());
    }
}

#[test]
fn failed_weak_resolve_racing_final_release_destroys_the_host() {
    use windows::Win32::System::WinRT::{IWeakReference, IWeakReferenceSource};

    let table = MetadataTable::new();
    let destroyed = Arc::new(AtomicUsize::new(0));
    let counter = DropCounter(destroyed.clone());
    let mut owner = create(vec![stringable_definition(&table)], move |_, _, _| {
        let _ = &counter;
        Ok(vec![WinRTValue::HString("weak target".into())])
    });
    let source: IWeakReferenceSource = owner
        .to_value()
        .unwrap()
        .as_object()
        .unwrap()
        .cast()
        .unwrap();
    let canonical = source.cast::<IUnknown>().unwrap();
    assert_eq!(canonical, owner.to_value().unwrap().as_object().unwrap());
    drop(canonical);
    let weak = unsafe { source.GetWeakReference().unwrap() };
    drop(source);
    let upgraded = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    let hook_upgraded = upgraded.clone();
    let hook_resume = resume.clone();
    let native = unsafe { &*weak.as_raw().cast::<identity::NativeWeakReference>() };
    native.after_upgrade(move || {
        hook_upgraded.wait();
        hook_resume.wait();
    });
    let worker_weak = WinRTValue::Object(weak.cast().unwrap());
    let worker = thread::spawn(move || {
        let weak: IWeakReference = worker_weak.as_object().unwrap().cast().unwrap();
        let mut result = ptr::null_mut();
        let hr = unsafe { (weak.vtable().Resolve)(weak.as_raw(), &IClosable::IID, &mut result) };
        assert_eq!(hr, E_NOINTERFACE);
        assert!(result.is_null());
    });
    upgraded.wait();
    owner.release();
    assert_eq!(destroyed.load(Ordering::SeqCst), 0);
    resume.wait();
    worker.join().unwrap();
    assert_eq!(destroyed.load(Ordering::SeqCst), 1);
    assert!(owner.is_closed());
    let mut result = ptr::null_mut();
    assert_eq!(
        unsafe { (weak.vtable().Resolve)(weak.as_raw(), &IStringable::IID, &mut result) },
        S_OK
    );
    assert!(result.is_null());
    assert!(weak.cast::<windows_core::imp::IAgileObject>().is_ok());
    assert!(weak.cast::<IInspectable>().is_err());
}

#[test]
fn async_array_values_keep_their_declared_async_type_in_all_directions() {
    use windows::Storage::StorageFile;

    let table = MetadataTable::new();
    let file_type = table.runtime_class(
        "Windows.Storage.StorageFile".into(),
        &table.interface(StorageFile::IID),
    );
    let operation = StorageFile::GetFileFromPathAsync(&HSTRING::from(
        std::env::current_exe().unwrap().to_string_lossy().as_ref(),
    ))
    .unwrap();
    let async_type = table.async_operation(&file_type);
    let generic_type = table.parameterized(
        &table.generic(crate::metadata_table::IASYNC_OPERATION, 1),
        &[file_type],
    );
    let value = WinRTValue::Async(crate::AsyncInfo {
        info: operation.cast().unwrap(),
        async_type: async_type.clone(),
    });
    for element in [async_type, generic_type] {
        for direction in 0..3 {
            let array_type = table.array(&element);
            let signature = match direction {
                0 => MethodSignature::new(&table)
                    .add_in(array_type.clone())
                    .add_out(array_type),
                1 => MethodSignature::new(&table).add_out(array_type),
                _ => MethodSignature::new(&table).add_out_fill(array_type),
            };
            let array = WinRTValue::Array(ArrayData::from_values(
                element.clone(),
                &[value.clone(), WinRTValue::Null],
            ));
            let returned = array.clone();
            let owner = create(
                vec![definition(
                    &table,
                    "Test.IAsyncArray",
                    IID_FIXTURE,
                    vec![signature.clone()],
                )],
                move |_, _, inputs| {
                    if direction == 0 {
                        let WinRTValue::Array(values) = &inputs[0] else {
                            panic!("expected array")
                        };
                        assert!(matches!(values.get(0), WinRTValue::Async(_)));
                    }
                    Ok(vec![returned.clone()])
                },
            );
            let view = query(&owner, &IID_FIXTURE);
            let inputs = if direction == 1 { vec![] } else { vec![array] };
            let outputs = signature
                .build(6)
                .call_dynamic(view.as_raw(), &inputs)
                .unwrap();
            let WinRTValue::Array(values) = &outputs[0] else {
                panic!("expected array")
            };
            let WinRTValue::Async(result) = values.get(0) else {
                panic!("native array element lost its async type");
            };
            assert_eq!(result.iid(), element.iid().unwrap());
            assert!(matches!(values.get(1), WinRTValue::Null));
        }
    }
}
