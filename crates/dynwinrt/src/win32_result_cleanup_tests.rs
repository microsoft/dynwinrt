// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use std::cell::Cell;
use windows::Win32::{
    Foundation::{HANDLE_FLAG_PROTECT_FROM_CLOSE, HANDLE_FLAGS, SetHandleInformation},
    System::Threading::CreateEventW,
};

thread_local! {
    static HANDLES: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    static PROTECTED: Cell<u8> = const { Cell::new(0) };
}

struct Fixture;

impl Fixture {
    fn new(protected: u8) -> Self {
        reset();
        HANDLES.with(|handles| handles.borrow_mut().clear());
        PROTECTED.set(protected);
        Self
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Reclaim only fixture handles whose tracked cleanup never succeeded.
        HANDLES.with(|handles| {
            for raw in handles.borrow().iter().copied() {
                if outcome_tests::successful_cleanup_count_for(raw) == 0 {
                    protect(raw, false);
                    unsafe { CloseHandle(HANDLE(raw as *mut c_void)) }.unwrap();
                }
            }
        });
    }
}

fn protect(raw: usize, protected: bool) {
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

fn event(index: u8) -> *mut c_void {
    let handle = unsafe { CreateEventW(None, true, false, None) }.unwrap();
    HANDLES.with(|handles| handles.borrow_mut().push(handle.0 as usize));
    if PROTECTED.get() & (1 << index) != 0 {
        protect(handle.0 as usize, true);
    }
    handle.0
}

fn discarded() -> ResultPolicy {
    ResultPolicy::discarded(ResultOwnership::Owned {
        cleanup: dynwinrt_win32_contracts::Cleanup::CloseHandle,
    })
}

#[test]
fn discarded_return_and_out_slots_keep_retryable_owners_on_native_success_or_failure() {
    unsafe extern "system" fn returned() -> *mut c_void {
        event(0)
    }
    unsafe extern "system" fn output(slot: *mut *mut c_void) -> i32 {
        unsafe { *slot = event(0) };
        1
    }
    for direct in [true, false] {
        for native_success in [true, false] {
            for explicit_retry in [true, false] {
                let _fixture = Fixture::new(1);
                let target = if direct {
                    ResultTarget::Return {}
                } else {
                    ResultTarget::Parameter { index: 0 }
                };
                let signature = if direct {
                    spec(
                        Vec::new(),
                        Type::Handle,
                        Cleanup::CloseHandle,
                        if native_success {
                            SuccessRule::ReturnNonNull
                        } else {
                            SuccessRule::ReturnZero
                        },
                    )
                } else {
                    spec(
                        vec![Parameter::output(Type::Handle, Cleanup::CloseHandle)],
                        Type::Bool32,
                        Cleanup::None,
                        if native_success {
                            SuccessRule::ReturnNonZero
                        } else {
                            SuccessRule::ReturnZero
                        },
                    )
                };
                let call = plan(
                    if direct {
                        returned as *const () as usize
                    } else {
                        output as *const () as usize
                    },
                    signature,
                    |contract| {
                        let result = contract
                            .results
                            .iter_mut()
                            .find(|result| result.target == target)
                            .unwrap();
                        result.on_success = discarded();
                        result.on_failure = discarded();
                    },
                );
                assert!(call.has_owned_result_cleanup());
                let error = unsafe { call.invoke(&[]) }.unwrap_err();
                let raw = HANDLES.with(|handles| handles.borrow()[0]);
                assert_eq!(error.cleanup_failures().len(), 1);
                let failure = &error.cleanup_failures()[0];
                assert_eq!(failure.target(), target);
                assert_eq!(failure.resource().raw(), raw);
                assert_eq!(failure.error().code().0 as u32, 0x80070006);
                assert!(matches!(error.source_error(), Error::WindowsError(_)));
                assert_eq!(outcome_tests::cleanup_count_for(raw), 1);
                assert_eq!(outcome_tests::successful_cleanup_count_for(raw), 0);
                if explicit_retry {
                    assert!(error.retry_cleanup().is_err());
                    assert_eq!(error.cleanup_failures()[0].resource().raw(), raw);
                    assert_eq!(outcome_tests::cleanup_count_for(raw), 2);
                    protect(raw, false);
                    let alias = Arc::clone(error.cleanup_failures()[0].resource());
                    error.retry_cleanup().unwrap();
                    error.retry_cleanup().unwrap();
                    assert!(alias.is_closed());
                    assert_eq!(alias.raw(), 0);
                    drop(error);
                    alias.close().unwrap();
                    assert_eq!(outcome_tests::cleanup_count_for(raw), 3);
                } else {
                    protect(raw, false);
                    drop(error);
                    assert_eq!(outcome_tests::cleanup_count_for(raw), 2);
                }
                assert_eq!(outcome_tests::successful_cleanup_count_for(raw), 1);
            }
        }
    }
}

unsafe extern "system" fn three_results(
    first: *mut *mut c_void,
    second: *mut *mut c_void,
) -> *mut c_void {
    let returned = event(0);
    unsafe {
        *first = event(1);
        *second = event(2);
    }
    returned
}

fn three_result_spec() -> CallPlanSpec {
    spec(
        vec![Parameter::output(Type::Handle, Cleanup::CloseHandle); 2],
        Type::Handle,
        Cleanup::CloseHandle,
        SuccessRule::ReturnNonNull,
    )
}

#[test]
fn later_conversion_failure_preserves_every_owner_and_partial_cleanup_retry() {
    let _fixture = Fixture::new(7);
    let call = plan(
        three_results as *const () as usize,
        three_result_spec(),
        |_| {},
    );
    outcome_tests::fail_decode(ResultTarget::Parameter { index: 1 });
    let error = unsafe { call.invoke(&[]) }.unwrap_err();
    assert!(
        error
            .message()
            .contains("Injected native result conversion failure")
    );
    assert_eq!(error.cleanup_failures().len(), 3);
    assert_eq!(
        error
            .cleanup_failures()
            .iter()
            .map(CleanupFailure::target)
            .collect::<Vec<_>>(),
        [
            ResultTarget::Return {},
            ResultTarget::Parameter { index: 0 },
            ResultTarget::Parameter { index: 1 }
        ]
    );
    let raw = HANDLES.with(|handles| handles.borrow().clone());
    for handle in &raw {
        assert_eq!(outcome_tests::cleanup_count_for(*handle), 1);
    }
    protect(raw[1], false);
    assert!(error.retry_cleanup().is_err());
    assert!(!error.cleanup_failures()[0].resource().is_closed());
    assert!(error.cleanup_failures()[1].resource().is_closed());
    assert!(!error.cleanup_failures()[2].resource().is_closed());
    protect(raw[0], false);
    protect(raw[2], false);
    error.retry_cleanup().unwrap();
    error.retry_cleanup().unwrap();
    drop(error);
    for (index, handle) in raw.iter().enumerate() {
        assert_eq!(
            outcome_tests::cleanup_count_for(*handle),
            if index == 1 { 2 } else { 3 }
        );
        assert_eq!(outcome_tests::successful_cleanup_count_for(*handle), 1);
    }
}

#[test]
fn failed_discard_retires_other_delivered_and_unprocessed_results_once() {
    let _fixture = Fixture::new(2);
    let call = plan(
        three_results as *const () as usize,
        three_result_spec(),
        |contract| {
            contract
                .results
                .iter_mut()
                .find(|result| result.target == (ResultTarget::Parameter { index: 0 }))
                .unwrap()
                .on_success = discarded();
        },
    );
    let error = unsafe { call.invoke(&[]) }.unwrap_err();
    let raw = HANDLES.with(|handles| handles.borrow().clone());
    assert_eq!(error.cleanup_failures().len(), 1);
    assert_eq!(
        error.cleanup_failures()[0].target(),
        ResultTarget::Parameter { index: 0 }
    );
    for handle in &raw {
        assert_eq!(outcome_tests::cleanup_count_for(*handle), 1);
    }
    assert_eq!(outcome_tests::successful_cleanup_count_for(raw[0]), 1);
    assert_eq!(outcome_tests::successful_cleanup_count_for(raw[2]), 1);
    protect(raw[1], false);
    error.retry_cleanup().unwrap();
    drop(error);
    assert_eq!(outcome_tests::successful_cleanup_count_for(raw[1]), 1);
    assert_eq!(outcome_tests::cleanup_count_for(raw[1]), 2);
}

#[test]
fn cleanup_recovery_does_not_retire_borrowed_aliases_and_respects_resource_leases() {
    unsafe extern "system" fn alias(input: *mut c_void, output: *mut *mut c_void) -> *mut c_void {
        unsafe { *output = event(1) };
        input
    }
    let _fixture = Fixture::new(2);
    let input = unsafe { OwnedResource::adopt(event(0) as usize, Cleanup::CloseHandle) }.unwrap();
    let input_raw = input.raw();
    let call = plan(
        alias as *const () as usize,
        spec(
            vec![
                Parameter {
                    resource_cleanup: Cleanup::CloseHandle,
                    ..Parameter::input(Type::Handle, false)
                },
                Parameter::output(Type::Handle, Cleanup::CloseHandle),
            ],
            Type::Handle,
            Cleanup::CloseHandle,
            SuccessRule::ReturnNonNull,
        ),
        |contract| {
            for result in &mut contract.results {
                result.on_success = if result.target == (ResultTarget::Return {}) {
                    ResultPolicy::delivered(ResultOwnership::AliasInput { parameter: 0 })
                } else {
                    discarded()
                };
            }
        },
    );
    let error = unsafe { call.invoke(&[Value::Resource(Arc::clone(&input))]) }.unwrap_err();
    assert_eq!(error.cleanup_failures().len(), 1);
    assert!(!input.is_closed());
    assert_eq!(outcome_tests::cleanup_count_for(input_raw), 0);
    let resource = Arc::clone(error.cleanup_failures()[0].resource());
    let raw = resource.raw();
    protect(raw, false);
    let lease = resource.async_lease(Cleanup::CloseHandle).unwrap();
    assert!(error.retry_cleanup().is_err());
    assert!(!resource.is_closed());
    assert_eq!(outcome_tests::cleanup_count_for(raw), 1);
    drop(lease);
    error.retry_cleanup().unwrap();
    assert!(resource.is_closed());
    input.close().unwrap();
    assert_eq!(outcome_tests::successful_cleanup_count_for(raw), 1);
    assert_eq!(outcome_tests::successful_cleanup_count_for(input_raw), 1);
}

#[test]
fn errors_before_dispatch_have_no_cleanup_recovery_resources() {
    let _fixture = Fixture::new(0);
    let call = plan(
        three_results as *const () as usize,
        three_result_spec(),
        |_| {},
    );
    let error = unsafe { call.invoke(&[Value::U32(1)]) }.unwrap_err();
    assert!(error.message().contains("expects 0 inputs"));
    assert!(error.cleanup_failures().is_empty());
    error.retry_cleanup().unwrap();
    assert!(HANDLES.with(|handles| handles.borrow().is_empty()));
}

#[test]
fn scalar_calls_do_not_require_cleanup_error_preparation() {
    extern "system" fn scalar() -> u32 {
        42
    }
    let call = plan(
        scalar as *const () as usize,
        spec(Vec::new(), Type::U32, Cleanup::None, SuccessRule::Always),
        |_| {},
    );
    assert!(!call.has_owned_result_cleanup());
    assert!(matches!(
        unsafe { call.invoke(&[]) }.unwrap().return_value,
        Some(Value::U32(42))
    ));
}

#[test]
fn aggregate_conversion_failure_keeps_field_ownership_separate_from_slot_recovery() {
    unsafe extern "system" fn combined(
        aggregate: *mut usize,
        output: *mut *mut c_void,
    ) -> *mut c_void {
        let returned = event(0);
        unsafe {
            *aggregate = event(1) as usize;
            *output = event(2);
        }
        returned
    }

    let _fixture = Fixture::new(7);
    let layout = NativeAggregatePointerLayout::new(
        "Tests.CleanupResult".into(),
        size_of::<usize>(),
        align_of::<usize>(),
        vec![AggregateResultField {
            name: "handle".into(),
            offset: 0,
            typ: Type::Handle,
            cleanup: Cleanup::CloseHandle,
        }],
    )
    .unwrap();
    let buffer = NativeAggregateBuffer::new(Arc::clone(&layout), None).unwrap();
    let specification = spec(
        vec![
            Parameter::input(Type::Pointer, false),
            Parameter::output(Type::Handle, Cleanup::CloseHandle),
        ],
        Type::Handle,
        Cleanup::CloseHandle,
        SuccessRule::ReturnNonNull,
    );
    let pointees = vec![Some(layout), None];
    let contract =
        CallContract::defaults(&contract::shape_with_pointees(&specification, &pointees));
    let mut call = Arc::try_unwrap(
        unsafe { CallPlan::new_with_contract_and_pointees(specification, contract, pointees) }
            .unwrap(),
    )
    .unwrap();
    call.function = combined as *const () as usize;
    outcome_tests::fail_decode(ResultTarget::AggregateField {
        parameter: 0,
        field: 0,
    });
    let error =
        unsafe { call.invoke(&[Value::AggregatePointer(Arc::clone(&buffer))]) }.unwrap_err();
    let raw = HANDLES.with(|handles| handles.borrow().clone());
    assert_eq!(
        error
            .cleanup_failures()
            .iter()
            .map(CleanupFailure::target)
            .collect::<Vec<_>>(),
        [
            ResultTarget::Return {},
            ResultTarget::Parameter { index: 1 }
        ]
    );
    assert_eq!(usize::from_ne_bytes(buffer.read(0).unwrap()), raw[1]);
    assert_eq!(outcome_tests::cleanup_count_for(raw[1]), 0);
    assert!(buffer.write(0, &0usize.to_ne_bytes()).is_err());
    for handle in &raw {
        protect(*handle, false);
    }
    error.retry_cleanup().unwrap();
    buffer.prepare().unwrap();
    drop(error);
    drop(buffer);
    for (index, handle) in raw.iter().enumerate() {
        assert_eq!(outcome_tests::successful_cleanup_count_for(*handle), 1);
        assert_eq!(
            outcome_tests::cleanup_count_for(*handle),
            if index == 1 { 1 } else { 2 }
        );
    }
}
