// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use std::cell::Cell;

#[repr(C)]
struct NativeInfo {
    process: usize,
    thread: usize,
    process_id: u32,
    thread_id: u32,
}

thread_local! { static WRITTEN: Cell<[usize; 2]> = const { Cell::new([0; 2]) }; }

unsafe extern "system" fn produce(info: *mut NativeInfo) -> i32 {
    let process =
        unsafe { windows::Win32::System::Threading::CreateEventW(None, true, false, None) }
            .unwrap();
    let thread =
        unsafe { windows::Win32::System::Threading::CreateEventW(None, true, false, None) }
            .unwrap();
    WRITTEN.set([process.0 as usize, thread.0 as usize]);
    unsafe {
        *info = NativeInfo {
            process: process.0 as usize,
            thread: thread.0 as usize,
            process_id: 42,
            thread_id: 84,
        };
    }
    1
}

fn layout() -> Arc<NativeAggregatePointerLayout> {
    NativeAggregatePointerLayout::new(
        "Tests.NativeInfo".into(),
        size_of::<NativeInfo>(),
        align_of::<NativeInfo>(),
        vec![
            AggregateResultField {
                name: "process".into(),
                offset: std::mem::offset_of!(NativeInfo, process),
                typ: Type::Handle,
                cleanup: Cleanup::CloseHandle,
            },
            AggregateResultField {
                name: "thread".into(),
                offset: std::mem::offset_of!(NativeInfo, thread),
                typ: Type::Handle,
                cleanup: Cleanup::CloseHandle,
            },
            AggregateResultField {
                name: "processId".into(),
                offset: std::mem::offset_of!(NativeInfo, process_id),
                typ: Type::U32,
                cleanup: Cleanup::None,
            },
            AggregateResultField {
                name: "threadId".into(),
                offset: std::mem::offset_of!(NativeInfo, thread_id),
                typ: Type::U32,
                cleanup: Cleanup::None,
            },
        ],
    )
    .unwrap()
}

fn spec() -> CallPlanSpec {
    CallPlanSpec {
        dll: "kernel32.dll".into(),
        entry_point: "GetLastError".into(),
        parameters: vec![Parameter::input(Type::Pointer, false)],
        return_type: Some(Type::Bool32),
        return_cleanup: Cleanup::None,
        success_rule: SuccessRule::ReturnNonZero,
        capture_last_error: false,
        calling_convention: CallingConvention::System,
        parameter_aggregates: vec![None],
        return_aggregate: None,
    }
}

fn planned(layout: Arc<NativeAggregatePointerLayout>) -> Arc<CallPlan> {
    let spec = spec();
    let pointees = vec![Some(layout)];
    let shape = contract::shape_with_pointees(&spec, &pointees);
    let contracts = CallContract::default().upgrade_metadata(&shape).unwrap();
    assert_eq!(contracts.results.len(), 5);
    let mut plan = Arc::try_unwrap(
        unsafe { CallPlan::new_with_contract_and_pointees(spec, contracts, pointees) }.unwrap(),
    )
    .unwrap();
    plan.function = produce as *const () as usize;
    Arc::new(plan)
}

#[test]
fn fields_are_owned_before_direct_return_conversion_and_result_delivery() {
    for failure in [Some(ResultTarget::Return {}), None] {
        outcome_tests::reset();
        let layout = layout();
        let buffer = NativeAggregateBuffer::new(Arc::clone(&layout), None).unwrap();
        let plan = planned(layout);
        if let Some(target) = failure {
            outcome_tests::fail_decode(target);
        }
        let result = unsafe { plan.invoke(&[Value::AggregatePointer(Arc::clone(&buffer))]) };
        if failure.is_some() {
            assert!(result.is_err());
        } else {
            drop(result.unwrap());
        }
        assert!(matches!(buffer.field_value(2).unwrap(), Value::U32(42)));
        let handles = WRITTEN.get();
        for handle in handles {
            assert_eq!(outcome_tests::cleanup_count_for(handle), 0);
        }
        drop(buffer);
        for handle in handles {
            assert_eq!(outcome_tests::cleanup_count_for(handle), 1);
        }
    }
}

#[test]
fn partially_converted_fields_keep_all_native_cleanup_responsibilities() {
    outcome_tests::reset();
    let layout = layout();
    let buffer = NativeAggregateBuffer::new(Arc::clone(&layout), None).unwrap();
    let plan = planned(layout);
    outcome_tests::fail_decode(ResultTarget::AggregateField {
        parameter: 0,
        field: 1,
    });
    assert!(unsafe { plan.invoke(&[Value::AggregatePointer(Arc::clone(&buffer))]) }.is_err());
    let handles = WRITTEN.get();
    assert!(buffer.field_value(0).unwrap().resource().is_some());
    assert!(buffer.field_value(1).is_err());
    drop(buffer);
    for handle in handles {
        assert_eq!(outcome_tests::cleanup_count_for(handle), 1);
    }
}

#[test]
fn taking_one_field_transfers_its_owner_without_owning_the_structure() {
    outcome_tests::reset();
    let layout = layout();
    let buffer = NativeAggregateBuffer::new(Arc::clone(&layout), None).unwrap();
    let plan = planned(layout);
    drop(unsafe { plan.invoke(&[Value::AggregatePointer(Arc::clone(&buffer))]) }.unwrap());
    let handles = WRITTEN.get();
    let taken = buffer.take_field(0).unwrap();
    assert_eq!(taken.resource().unwrap().raw(), handles[0]);
    assert!(matches!(buffer.take_field(0).unwrap(), Value::Handle(0)));
    assert_eq!(
        usize::from_ne_bytes(buffer.read::<{ size_of::<usize>() }>(0).unwrap()),
        0
    );
    drop(buffer);
    assert_eq!(outcome_tests::cleanup_count_for(handles[0]), 0);
    assert_eq!(outcome_tests::cleanup_count_for(handles[1]), 1);
    drop(taken);
    assert_eq!(outcome_tests::cleanup_count_for(handles[0]), 1);
}

#[test]
fn undefined_field_outputs_are_never_adopted_or_cleaned() {
    unsafe extern "system" fn failed(info: *mut NativeInfo) -> i32 {
        let [first, second] = WRITTEN.get();
        unsafe {
            (*info).process = first;
            (*info).thread = second;
        }
        0
    }
    outcome_tests::reset();
    let first = unsafe { windows::Win32::System::Threading::CreateEventW(None, true, false, None) }
        .unwrap();
    let second =
        unsafe { windows::Win32::System::Threading::CreateEventW(None, true, false, None) }
            .unwrap();
    WRITTEN.set([first.0 as usize, second.0 as usize]);
    let layout = layout();
    let buffer = NativeAggregateBuffer::new(Arc::clone(&layout), None).unwrap();
    let mut plan = Arc::try_unwrap(planned(layout)).unwrap();
    plan.function = failed as *const () as usize;
    assert!(
        !unsafe { plan.invoke(&[Value::AggregatePointer(Arc::clone(&buffer))]) }
            .unwrap()
            .succeeded
    );
    assert!(buffer.field_value(0).is_err());
    drop(buffer);
    for handle in WRITTEN.get() {
        assert_eq!(outcome_tests::cleanup_count_for(handle), 0);
    }
    unsafe {
        CloseHandle(first).unwrap();
        CloseHandle(second).unwrap();
    }
}

#[test]
fn caller_storage_identity_and_field_ranges_fail_before_dispatch() {
    let layout = layout();
    let plan = planned(Arc::clone(&layout));
    let other = NativeAggregatePointerLayout::new(
        "Other".into(),
        layout.size(),
        layout.alignment(),
        layout.fields().to_vec(),
    )
    .unwrap();
    let buffer = NativeAggregateBuffer::new(other, None).unwrap();
    assert!(
        unsafe { plan.invoke(&[Value::AggregatePointer(buffer)]) }
            .unwrap_err()
            .message()
            .contains("matching managed")
    );
    let mut info = NativeInfo {
        process: 0,
        thread: 0,
        process_id: 0,
        thread_id: 0,
    };
    assert!(
        unsafe { plan.invoke(&[Value::Pointer((&mut info as *mut NativeInfo).cast())]) }.is_err()
    );
    assert!(
        NativeAggregatePointerLayout::new(
            "Bad".into(),
            8,
            8,
            vec![AggregateResultField {
                name: "outside".into(),
                offset: 8,
                typ: Type::Handle,
                cleanup: Cleanup::CloseHandle
            },]
        )
        .is_err()
    );
    assert!(
        NativeAggregatePointerLayout::new(
            "Overlap".into(),
            16,
            8,
            vec![
                AggregateResultField {
                    name: "a".into(),
                    offset: 0,
                    typ: Type::Handle,
                    cleanup: Cleanup::CloseHandle
                },
                AggregateResultField {
                    name: "b".into(),
                    offset: 0,
                    typ: Type::Handle,
                    cleanup: Cleanup::CloseHandle
                },
            ]
        )
        .is_err()
    );
}

#[test]
fn later_language_markers_cannot_erase_native_ownership() {
    outcome_tests::reset();
    let layout = layout();
    let buffer = NativeAggregateBuffer::new(Arc::clone(&layout), None).unwrap();
    drop(
        unsafe { planned(layout).invoke(&[Value::AggregatePointer(Arc::clone(&buffer))]) }.unwrap(),
    );
    assert!(buffer.mark_legacy(false).is_err());
    buffer.mark_legacy(true).unwrap();
    let handles = WRITTEN.get();
    drop(buffer);
    for handle in handles {
        assert_eq!(outcome_tests::cleanup_count_for(handle), 1);
    }
}

#[test]
fn real_create_process_fields_survive_a_failure_before_language_wrapping() {
    use windows::Win32::System::Threading::STARTUPINFOW;
    outcome_tests::reset();
    let output_layout = layout();
    let buffer = NativeAggregateBuffer::new(Arc::clone(&output_layout), None).unwrap();
    let mut startup = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let command = format!("\"{}\" /d /c exit 0", std::env::var("ComSpec").unwrap());
    let mut command = command.encode_utf16().chain([0]).collect::<Vec<_>>();
    let spec = CallPlanSpec {
        dll: "kernel32.dll".into(),
        entry_point: "CreateProcessW".into(),
        parameters: vec![
            Parameter::input(Type::Pointer, true),
            Parameter::input(Type::Pointer, false),
            Parameter::input(Type::Pointer, true),
            Parameter::input(Type::Pointer, true),
            Parameter::input(Type::Bool32, false),
            Parameter::input(Type::U32, false),
            Parameter::input(Type::Pointer, true),
            Parameter::input(Type::Pointer, true),
            Parameter::input(Type::Pointer, false),
            Parameter::input(Type::Pointer, false),
        ],
        return_type: Some(Type::Bool32),
        return_cleanup: Cleanup::None,
        success_rule: SuccessRule::ReturnNonZero,
        capture_last_error: true,
        calling_convention: CallingConvention::System,
        parameter_aggregates: vec![None; 10],
        return_aggregate: None,
    };
    let mut pointees = vec![None; 10];
    pointees[9] = Some(output_layout);
    let contract = CallContract::default()
        .upgrade_metadata(&contract::shape_with_pointees(&spec, &pointees))
        .unwrap();
    let plan =
        unsafe { CallPlan::new_with_contract_and_pointees(spec, contract, pointees) }.unwrap();
    outcome_tests::fail_decode(ResultTarget::Return {});
    let error = unsafe {
        plan.invoke(&[
            Value::Null,
            Value::Pointer(command.as_mut_ptr().cast()),
            Value::Null,
            Value::Null,
            Value::Bool(false),
            Value::U32(0x0800_0000),
            Value::Null,
            Value::Null,
            Value::Pointer((&mut startup as *mut STARTUPINFOW).cast()),
            Value::AggregatePointer(Arc::clone(&buffer)),
        ])
    }
    .unwrap_err();
    assert!(error.message().contains("conversion failure"), "{error:?}");
    assert!(matches!(buffer.field_value(2).unwrap(), Value::U32(pid) if pid > 0));
    let process = buffer.field_value(0).unwrap().resource().unwrap().raw();
    let thread = buffer.field_value(1).unwrap().resource().unwrap().raw();
    assert_ne!(process, 0);
    assert_ne!(thread, 0);
    drop(buffer);
    assert_eq!(outcome_tests::cleanup_count_for(process), 1);
    assert_eq!(outcome_tests::cleanup_count_for(thread), 1);
}

#[test]
fn defined_but_discarded_field_resources_are_cleaned_on_business_failure() {
    unsafe extern "system" fn partial(info: *mut NativeInfo) -> i32 {
        unsafe { produce(info) };
        0
    }
    outcome_tests::reset();
    let layout = layout();
    let buffer = NativeAggregateBuffer::new(Arc::clone(&layout), None).unwrap();
    let spec = spec();
    let pointees = vec![Some(layout)];
    let mut contract = CallContract::default()
        .upgrade_metadata(&contract::shape_with_pointees(&spec, &pointees))
        .unwrap();
    for result in &mut contract.results {
        if matches!(
            result.target,
            ResultTarget::AggregateField { field: 0 | 1, .. }
        ) {
            result.on_failure = ResultPolicy::discarded(ResultOwnership::Owned {
                cleanup: dynwinrt_win32_contracts::Cleanup::CloseHandle,
            });
        }
    }
    let mut plan = Arc::try_unwrap(
        unsafe { CallPlan::new_with_contract_and_pointees(spec, contract, pointees) }.unwrap(),
    )
    .unwrap();
    plan.function = partial as *const () as usize;
    assert!(
        !unsafe { plan.invoke(&[Value::AggregatePointer(Arc::clone(&buffer))]) }
            .unwrap()
            .succeeded
    );
    let handles = WRITTEN.get();
    for handle in handles {
        assert_eq!(outcome_tests::cleanup_count_for(handle), 1);
    }
    assert!(buffer.field_value(0).is_err());
    drop(buffer);
    for handle in handles {
        assert_eq!(outcome_tests::cleanup_count_for(handle), 1);
    }
}
