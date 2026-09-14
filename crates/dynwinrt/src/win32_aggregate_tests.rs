// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use std::{
    cell::Cell,
    fs::File,
    os::windows::io::{AsRawHandle, IntoRawHandle},
};
use windows::Win32::Foundation::{
    HANDLE_FLAG_PROTECT_FROM_CLOSE, HANDLE_FLAGS, SetHandleInformation,
};

#[path = "win32_aggregate_effect_tests.rs"]
mod effect_tests;

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

fn native_handles(buffer: &NativeAggregateBuffer) -> [usize; 2] {
    [
        usize::from_ne_bytes(buffer.read(0).unwrap()),
        usize::from_ne_bytes(buffer.read(size_of::<usize>()).unwrap()),
    ]
}

fn protect_handle(raw: usize, protected: bool) {
    unsafe {
        SetHandleInformation(
            HANDLE(raw as *mut c_void),
            HANDLE_FLAG_PROTECT_FROM_CLOSE.0,
            if protected {
                HANDLE_FLAG_PROTECT_FROM_CLOSE
            } else {
                HANDLE_FLAGS(0)
            },
        )
    }
    .unwrap();
}

fn retirement_counts(handles: [usize; 2]) -> [(usize, usize); 2] {
    let counts = handles.map(|raw| {
        (
            outcome_tests::cleanup_count_for(raw),
            outcome_tests::successful_cleanup_count_for(raw),
        )
    });
    // Measure before reclaiming leaked fixture handles, so a regression still fails.
    for (raw, (_, successes)) in handles.into_iter().zip(counts) {
        if successes == 0 {
            unsafe { CloseHandle(HANDLE(raw as *mut c_void)) }.unwrap();
        }
    }
    counts
}

#[test]
#[forbid(unsafe_code)]
fn safe_aggregate_storage_does_not_adopt_borrowed_handles() {
    outcome_tests::reset();
    let file = File::open(std::env::current_exe().unwrap()).unwrap();
    let handle = file.as_raw_handle() as usize;
    let layout = layout();
    let mut bytes = vec![0; layout.size()];
    for offset in [
        std::mem::offset_of!(NativeInfo, process),
        std::mem::offset_of!(NativeInfo, thread),
    ] {
        bytes[offset..offset + size_of::<usize>()].copy_from_slice(&handle.to_ne_bytes());
    }
    for initialize in [false, true] {
        for prepare in [false, true] {
            let buffer = NativeAggregateBuffer::new(
                Arc::clone(&layout),
                initialize.then_some(bytes.as_slice()),
            )
            .unwrap();
            if !initialize {
                buffer.write(0, &bytes).unwrap();
            }
            assert_eq!(buffer.bytes().unwrap(), bytes);
            for field in 0..2 {
                assert!(buffer.field_value(field).is_err());
                assert!(buffer.take_field(field).is_err());
            }
            if prepare {
                buffer.prepare().unwrap();
                assert!(file.metadata().is_ok());
            }
            drop(buffer);
            assert_eq!(outcome_tests::cleanup_count_for(handle), 0);
            assert!(file.metadata().is_ok());
        }
    }
}

#[test]
fn unsafe_legacy_adoption_transfers_and_retires_ownership_once() {
    for take in [false, true] {
        outcome_tests::reset();
        let buffer = NativeAggregateBuffer::new(layout(), None).unwrap();
        let handle = File::open(std::env::current_exe().unwrap())
            .unwrap()
            .into_raw_handle() as usize;
        buffer.write(0, &handle.to_ne_bytes()).unwrap();
        // IntoRawHandle relinquished File's unique ownership; CloseHandle matches it.
        unsafe { buffer.mark_legacy(true) }.unwrap();
        assert_eq!(
            buffer.field_value(0).unwrap().resource().unwrap().raw(),
            handle
        );
        let taken = take.then(|| buffer.take_field(0).unwrap());
        drop(buffer);
        assert_eq!(outcome_tests::cleanup_count_for(handle), usize::from(!take));
        if let Some(value) = taken {
            let resource = value.resource().unwrap();
            assert_eq!(resource.raw(), handle);
            resource.close().unwrap();
            resource.close().unwrap();
            drop(value);
        }
        assert_eq!(outcome_tests::cleanup_count_for(handle), 1);
    }
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
fn safe_writes_cannot_replace_pending_raw_cleanup_targets() {
    let file = File::open(std::env::current_exe().unwrap()).unwrap();
    let borrowed = file.as_raw_handle() as usize;
    let borrowed_bytes = borrowed.to_ne_bytes();
    let offset = std::mem::offset_of!(NativeInfo, thread);
    let width = size_of::<usize>();
    for prepare in [false, true] {
        outcome_tests::reset();
        let layout = layout();
        let buffer = NativeAggregateBuffer::new(Arc::clone(&layout), None).unwrap();
        outcome_tests::fail_decode(ResultTarget::AggregateField {
            parameter: 0,
            field: 1,
        });
        assert!(
            unsafe { planned(layout).invoke(&[Value::AggregatePointer(Arc::clone(&buffer))]) }
                .is_err()
        );
        let handles = WRITTEN.get();
        assert!(buffer.field_value(1).is_err());
        let original = buffer.bytes().unwrap();
        let mut replacement = original.clone();
        replacement[offset..offset + width].copy_from_slice(&borrowed_bytes);
        for (start, bytes) in [
            (offset, borrowed_bytes.as_slice()),
            (offset, &borrowed_bytes[..1]),
            (offset + width - 1, &borrowed_bytes[..1]),
            (offset - 1, &borrowed_bytes[..2]),
            (offset + width - 1, &borrowed_bytes[..2]),
            (0, replacement.as_slice()),
        ] {
            assert!(
                buffer
                    .write(start, bytes)
                    .unwrap_err()
                    .message()
                    .contains("owned result awaits cleanup")
            );
            assert_eq!(buffer.bytes().unwrap(), original);
        }
        buffer.write(offset, &[]).unwrap();
        buffer.write(buffer.layout().size(), &[]).unwrap();
        let scalar_offset = std::mem::offset_of!(NativeInfo, process_id);
        buffer.write(scalar_offset, &99u32.to_ne_bytes()).unwrap();
        assert_eq!(u32::from_ne_bytes(buffer.read(scalar_offset).unwrap()), 99);

        // Captured owners no longer use mutable field bytes as their cleanup targets.
        buffer.write(0, &borrowed_bytes).unwrap();
        assert_eq!(
            buffer.field_value(0).unwrap().resource().unwrap().raw(),
            handles[0]
        );
        if prepare {
            buffer.prepare().unwrap();
            for handle in handles {
                assert_eq!(outcome_tests::cleanup_count_for(handle), 1);
            }
            buffer.write(offset, &borrowed_bytes).unwrap();
        }
        drop(buffer);
        for handle in handles {
            assert_eq!(outcome_tests::cleanup_count_for(handle), 1);
        }
        assert_eq!(outcome_tests::cleanup_count_for(borrowed), 0);
        assert!(file.metadata().is_ok());
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
    // The completed native call has already established the fields' ownership.
    assert!(unsafe { buffer.mark_legacy(false) }.is_err());
    unsafe { buffer.mark_legacy(true) }.unwrap();
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

#[test]
fn failed_discard_preserves_raw_cleanup_and_write_protection_until_retirement() {
    unsafe extern "system" fn protected_outputs(info: *mut NativeInfo, succeeded: i32) -> i32 {
        unsafe { produce(info) };
        protect_handle(unsafe { (*info).process }, true);
        succeeded
    }

    for succeeded in [false, true] {
        for prepare in [false, true] {
            outcome_tests::reset();
            let layout = layout();
            let buffer = NativeAggregateBuffer::new(Arc::clone(&layout), None).unwrap();
            let mut spec = spec();
            spec.parameters.push(Parameter::input(Type::Bool32, false));
            spec.parameter_aggregates.push(None);
            let pointees = vec![Some(layout), None];
            let mut contract = CallContract::default()
                .upgrade_metadata(&contract::shape_with_pointees(&spec, &pointees))
                .unwrap();
            for result in &mut contract.results {
                if matches!(
                    result.target,
                    ResultTarget::AggregateField { field: 0 | 1, .. }
                ) {
                    let policy = ResultPolicy::discarded(ResultOwnership::Owned {
                        cleanup: dynwinrt_win32_contracts::Cleanup::CloseHandle,
                    });
                    result.on_success = policy;
                    result.on_failure = policy;
                }
            }
            let mut plan = Arc::try_unwrap(
                unsafe { CallPlan::new_with_contract_and_pointees(spec, contract, pointees) }
                    .unwrap(),
            )
            .unwrap();
            plan.function = protected_outputs as *const () as usize;
            let call_failed = unsafe {
                plan.invoke(&[
                    Value::AggregatePointer(Arc::clone(&buffer)),
                    Value::Bool(succeeded),
                ])
            }
            .is_err();
            let raw = native_handles(&buffer);
            let guard_after_call = buffer.lock().raw_cleanup[0];
            let write_rejected = buffer.write(0, &raw[0].to_ne_bytes()).is_err();
            let partial_write_rejected = buffer
                .write(
                    size_of::<usize>() - 1,
                    &raw[0].to_ne_bytes()[size_of::<usize>() - 1..],
                )
                .is_err();
            let retry_failed = buffer.prepare().is_err();
            let guard_after_retry = buffer.lock().raw_cleanup[0];
            let raw_after_retry = native_handles(&buffer);
            let retry_write_rejected = buffer.write(0, &raw[0].to_ne_bytes()).is_err();
            protect_handle(raw[0], false);
            if prepare {
                buffer.prepare().unwrap();
                buffer.prepare().unwrap();
                buffer.write(0, &0usize.to_ne_bytes()).unwrap();
            }
            drop(buffer);
            let counts = retirement_counts(raw);
            assert!(call_failed);
            assert!(retry_failed);
            assert_eq!(guard_after_call, Cleanup::CloseHandle);
            assert_eq!(guard_after_retry, Cleanup::CloseHandle);
            assert!(write_rejected && partial_write_rejected && retry_write_rejected);
            assert_eq!(raw_after_retry, [raw[0], 0]);
            assert_eq!(
                counts,
                [(3, 1), (1, 1)],
                "succeeded={succeeded}, prepare={prepare}"
            );
        }
    }
}

#[test]
fn all_aggregate_states_are_recorded_before_fallible_field_processing() {
    unsafe extern "system" fn outputs(
        first: *mut NativeInfo,
        second: *mut NativeInfo,
        succeeded: i32,
        protect_first: i32,
    ) -> i32 {
        unsafe {
            produce(first);
            produce(second);
            if protect_first != 0 {
                protect_handle((*first).process, true);
            }
        }
        succeeded
    }

    for succeeded in [false, true] {
        for failed_field in [Some(0), Some(1), None] {
            for prepare in [false, true] {
                outcome_tests::reset();
                let layout = layout();
                let first = NativeAggregateBuffer::new(Arc::clone(&layout), None).unwrap();
                let second = NativeAggregateBuffer::new(Arc::clone(&layout), None).unwrap();
                let mut spec = spec();
                spec.parameters.extend([
                    Parameter::input(Type::Pointer, false),
                    Parameter::input(Type::Bool32, false),
                    Parameter::input(Type::Bool32, false),
                ]);
                spec.parameter_aggregates.resize(4, None);
                let pointees = vec![Some(Arc::clone(&layout)), Some(layout), None, None];
                let mut contract = CallContract::default()
                    .upgrade_metadata(&contract::shape_with_pointees(&spec, &pointees))
                    .unwrap();
                for result in &mut contract.results {
                    if matches!(
                        result.target,
                        ResultTarget::AggregateField { field: 0 | 1, .. }
                    ) {
                        result.on_failure = result.on_success;
                    }
                    if failed_field.is_none()
                        && result.target
                            == (ResultTarget::AggregateField {
                                parameter: 0,
                                field: 0,
                            })
                    {
                        let policy = ResultPolicy::discarded(ResultOwnership::Owned {
                            cleanup: dynwinrt_win32_contracts::Cleanup::CloseHandle,
                        });
                        result.on_success = policy;
                        result.on_failure = policy;
                    }
                }
                let mut plan = Arc::try_unwrap(
                    unsafe { CallPlan::new_with_contract_and_pointees(spec, contract, pointees) }
                        .unwrap(),
                )
                .unwrap();
                plan.function = outputs as *const () as usize;
                if let Some(field) = failed_field {
                    outcome_tests::fail_decode(ResultTarget::AggregateField {
                        parameter: 0,
                        field,
                    });
                }
                let error = unsafe {
                    plan.invoke(&[
                        Value::AggregatePointer(Arc::clone(&first)),
                        Value::AggregatePointer(Arc::clone(&second)),
                        Value::Bool(succeeded),
                        Value::Bool(failed_field.is_none()),
                    ])
                }
                .unwrap_err();
                let first_raw = native_handles(&first);
                let second_raw = native_handles(&second);
                if failed_field.is_none() {
                    protect_handle(first_raw[0], false);
                }
                let (recorded, native_succeeded, guards_before) = {
                    let state = second.lock();
                    (
                        state.native_recorded,
                        state.succeeded,
                        [state.raw_cleanup[0], state.raw_cleanup[1]],
                    )
                };
                // The native writes completed; this is their actual success/failure status.
                let marker = unsafe { second.mark_legacy(succeeded) };
                let guards_after = {
                    let state = second.lock();
                    [state.raw_cleanup[0], state.raw_cleanup[1]]
                };
                let write_rejected = second.write(0, &second_raw[0].to_ne_bytes()).is_err();
                drop(first);
                if prepare {
                    second.prepare().unwrap();
                }
                drop(second);
                let first_counts = retirement_counts(first_raw);
                let second_counts = retirement_counts(second_raw);
                if failed_field.is_some() {
                    assert!(
                        error
                            .message()
                            .contains("Injected native result conversion failure")
                    );
                }
                assert!(recorded);
                assert_eq!(native_succeeded, Some(succeeded));
                assert!(marker.is_ok());
                assert_eq!(guards_before, [Cleanup::CloseHandle; 2]);
                assert_eq!(guards_after, guards_before);
                assert!(write_rejected);
                assert_eq!(
                    first_counts,
                    [(if failed_field.is_none() { 2 } else { 1 }, 1), (1, 1)]
                );
                assert_eq!(second_counts, [(1, 1); 2]);
            }
        }
    }
}

#[test]
fn undescribed_aggregate_calls_keep_the_legacy_registration_path() {
    outcome_tests::reset();
    let buffer = NativeAggregateBuffer::new(layout(), None).unwrap();
    buffer.prepare().unwrap();
    let mut plan = Arc::try_unwrap(unsafe { CallPlan::new(spec()) }.unwrap()).unwrap();
    plan.function = produce as *const () as usize;
    let result = unsafe { plan.invoke(&[Value::AggregatePointer(Arc::clone(&buffer))]) }.unwrap();
    let raw = native_handles(&buffer);
    let recorded = buffer.lock().native_recorded;
    // This legacy call transferred both live event handles into the caller's fields.
    unsafe { buffer.mark_legacy(result.succeeded) }.unwrap();
    let owned = [0, 1].map(|field| {
        buffer
            .field_value(field)
            .is_ok_and(|value| value.resource().is_some())
    });
    drop(buffer);
    let counts = retirement_counts(raw);
    assert!(result.succeeded);
    assert!(!recorded);
    assert_eq!(owned, [true; 2]);
    assert_eq!(counts, [(1, 1); 2]);
}
