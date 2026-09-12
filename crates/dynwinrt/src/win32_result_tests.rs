// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use std::cell::RefCell;

thread_local! { static ALLOCATED: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) }; }

fn allocate() -> *mut c_void {
    let pointer = unsafe { windows::Win32::System::Com::CoTaskMemAlloc(16) };
    assert!(!pointer.is_null());
    ALLOCATED.with(|values| values.borrow_mut().push(pointer as usize));
    pointer
}

fn reset() {
    outcome_tests::reset();
    ALLOCATED.with(|values| values.borrow_mut().clear());
}

fn assert_freed_once() {
    ALLOCATED.with(|values| {
        for pointer in values.borrow().iter().copied() {
            assert_eq!(
                outcome_tests::cleanup_count_for(pointer),
                1,
                "0x{pointer:x}"
            );
        }
    });
}

fn spec(
    parameters: Vec<Parameter>,
    return_type: Type,
    return_cleanup: Cleanup,
    success_rule: SuccessRule,
) -> CallPlanSpec {
    CallPlanSpec {
        dll: "kernel32.dll".into(),
        entry_point: "GetLastError".into(),
        parameter_aggregates: vec![None; parameters.len()],
        parameters,
        return_type: Some(return_type),
        return_cleanup,
        success_rule,
        capture_last_error: false,
        calling_convention: CallingConvention::System,
        return_aggregate: None,
    }
}

fn plan(
    function: usize,
    spec: CallPlanSpec,
    configure: impl FnOnce(&mut CallContract),
) -> Arc<CallPlan> {
    let mut contract = CallContract::defaults(&contract::shape(&spec));
    configure(&mut contract);
    let mut plan =
        Arc::try_unwrap(unsafe { CallPlan::new_with_contract(spec, contract) }.unwrap()).unwrap();
    plan.function = function;
    Arc::new(plan)
}

fn owned(delivery: Delivery) -> ResultPolicy {
    ResultPolicy::Defined {
        ownership: ResultOwnership::Owned {
            cleanup: dynwinrt_win32_contracts::Cleanup::CoTaskMemFree,
        },
        delivery,
    }
}

#[test]
fn failed_valid_owned_outputs_can_be_delivered_or_discarded_with_exact_cleanup() {
    unsafe extern "system" fn partial(output: *mut *mut c_void) -> i32 {
        unsafe { *output = allocate() };
        5
    }
    for delivery in [Delivery::Deliver, Delivery::Discard] {
        reset();
        let call = plan(
            partial as *const () as usize,
            spec(
                vec![Parameter::output(Type::Handle, Cleanup::CoTaskMemFree)],
                Type::I32,
                Cleanup::None,
                SuccessRule::ReturnZero,
            ),
            |contract| {
                contract
                    .results
                    .iter_mut()
                    .find(|result| result.target == (ResultTarget::Parameter { index: 0 }))
                    .unwrap()
                    .on_failure = owned(delivery)
            },
        );
        let result = unsafe { call.invoke(&[]) }.unwrap();
        assert!(!result.succeeded);
        match delivery {
            Delivery::Deliver => {
                assert!(matches!(result.outputs[0], Value::Resource(_)));
                ALLOCATED.with(|values| {
                    assert_eq!(outcome_tests::cleanup_count_for(values.borrow()[0]), 0)
                });
            }
            Delivery::Discard => assert!(matches!(result.outputs[0], Value::Discarded)),
        }
        drop(result);
        assert_freed_once();
    }
}

#[test]
fn direct_owned_returns_are_cleaned_even_when_not_delivered_on_business_failure() {
    extern "system" fn partial() -> *mut c_void {
        allocate()
    }
    reset();
    let call = plan(
        partial as *const () as usize,
        spec(
            vec![],
            Type::Handle,
            Cleanup::CoTaskMemFree,
            SuccessRule::ReturnZero,
        ),
        |contract| contract.results[0].on_failure = owned(Delivery::Discard),
    );
    let result = unsafe { call.invoke(&[]) }.unwrap();
    assert!(!result.succeeded);
    assert!(matches!(result.return_value, Some(Value::Discarded)));
    drop(result);
    assert_freed_once();
}

#[test]
fn direct_return_aliases_share_the_input_owner_without_adoption() {
    extern "system" fn echo(input: *mut c_void) -> *mut c_void {
        input
    }
    reset();
    let input =
        unsafe { OwnedResource::adopt(allocate() as usize, Cleanup::CoTaskMemFree) }.unwrap();
    let call = plan(
        echo as *const () as usize,
        spec(
            vec![Parameter {
                resource_cleanup: Cleanup::CoTaskMemFree,
                ..Parameter::input(Type::Handle, false)
            }],
            Type::Handle,
            Cleanup::CoTaskMemFree,
            SuccessRule::ReturnNonNull,
        ),
        |contract| {
            contract.results[0].on_success =
                ResultPolicy::delivered(ResultOwnership::AliasInput { parameter: 0 })
        },
    );
    let result = unsafe { call.invoke(&[Value::Resource(Arc::clone(&input))]) }.unwrap();
    let alias = result.return_value.as_ref().unwrap().resource().unwrap();
    assert!(Arc::ptr_eq(alias, &input));
    drop(result);
    assert!(!input.is_closed());
    drop(input);
    assert_freed_once();
}

#[test]
fn undefined_direct_pointer_is_not_adopted_or_freed() {
    extern "system" fn opaque(input: *mut c_void) -> *mut c_void {
        input
    }
    reset();
    let input =
        unsafe { OwnedResource::adopt(allocate() as usize, Cleanup::CoTaskMemFree) }.unwrap();
    let call = plan(
        opaque as *const () as usize,
        spec(
            vec![Parameter {
                resource_cleanup: Cleanup::CoTaskMemFree,
                ..Parameter::input(Type::Handle, false)
            }],
            Type::Handle,
            Cleanup::CoTaskMemFree,
            SuccessRule::Always,
        ),
        |contract| contract.results[0].on_success = ResultPolicy::Undefined {},
    );
    let result = unsafe { call.invoke(&[Value::Resource(Arc::clone(&input))]) }.unwrap();
    assert!(matches!(result.return_value, Some(Value::Unavailable)));
    assert!(outcome_tests::decoded_outputs().is_empty());
    assert!(!input.is_closed());
    drop(result);
    drop(input);
    assert_freed_once();
}

#[test]
fn conversion_failure_cleans_all_valid_untransferred_results_once() {
    unsafe extern "system" fn produce(
        first: *mut *mut c_void,
        second: *mut *mut c_void,
    ) -> *mut c_void {
        unsafe {
            *first = allocate();
            *second = allocate();
        }
        allocate()
    }
    for failure in [
        ResultTarget::Return {},
        ResultTarget::Parameter { index: 0 },
        ResultTarget::Parameter { index: 1 },
    ] {
        reset();
        let call = plan(
            produce as *const () as usize,
            spec(
                vec![Parameter::output(Type::Handle, Cleanup::CoTaskMemFree); 2],
                Type::Handle,
                Cleanup::CoTaskMemFree,
                SuccessRule::Always,
            ),
            |_| {},
        );
        outcome_tests::fail_decode(failure);
        assert!(
            unsafe { call.invoke(&[]) }
                .unwrap_err()
                .message()
                .contains("conversion failure")
        );
        assert_freed_once();
    }
}

#[test]
fn discarded_results_and_failed_delivery_do_not_skip_cleanup_or_double_release() {
    unsafe extern "system" fn produce(output: *mut *mut c_void) -> *mut c_void {
        unsafe { *output = allocate() };
        allocate()
    }
    reset();
    let call = plan(
        produce as *const () as usize,
        spec(
            vec![Parameter::output(Type::Handle, Cleanup::CoTaskMemFree)],
            Type::Handle,
            Cleanup::CoTaskMemFree,
            SuccessRule::Always,
        ),
        |contract| contract.results[0].on_success = owned(Delivery::Discard),
    );
    outcome_tests::fail_decode(ResultTarget::Parameter { index: 0 });
    assert!(unsafe { call.invoke(&[]) }.is_err());
    assert_freed_once();
}
