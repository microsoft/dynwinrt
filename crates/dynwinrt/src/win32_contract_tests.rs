// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;

fn handle_input(cleanup: Cleanup) -> Parameter {
    Parameter {
        resource_cleanup: cleanup,
        ..Parameter::input(Type::Handle, false)
    }
}

fn spec(parameters: Vec<Parameter>) -> CallPlanSpec {
    CallPlanSpec {
        dll: "kernel32.dll".into(),
        entry_point: "GetLastError".into(),
        parameter_aggregates: vec![None; parameters.len()],
        parameters,
        return_type: Some(Type::I32),
        return_cleanup: Cleanup::None,
        success_rule: SuccessRule::ReturnZero,
        capture_last_error: false,
        calling_convention: CallingConvention::System,
        return_aggregate: None,
    }
}

fn plan(function: usize, parameters: Vec<Parameter>, contract: CallContract) -> Arc<CallPlan> {
    let mut plan = Arc::try_unwrap(
        unsafe { CallPlan::new_with_contract(spec(parameters), contract) }.unwrap(),
    )
    .unwrap();
    plan.function = function;
    Arc::new(plan)
}

fn alias_contract(width: u8) -> CallContract {
    CallContract {
        outputs: vec![OutputRule {
            parameter: 2,
            when: Condition {
                inputs: vec![InputPredicate::NullOrEmpty {
                    parameter: 1,
                    element_width: width,
                }],
                return_value: None,
            },
            action: OutputAction::AliasInput { parameter: 0 },
        }],
        ..CallContract::default()
    }
}

fn alias_parameters(cleanup: Cleanup) -> Vec<Parameter> {
    vec![
        handle_input(cleanup),
        Parameter::input(Type::Pointer, true),
        Parameter::output(Type::Handle, cleanup),
    ]
}

unsafe extern "system" fn alias(
    input: *mut c_void,
    _: *const c_void,
    output: *mut *mut c_void,
) -> i32 {
    unsafe { *output = input };
    0
}

fn allocation() -> Arc<OwnedResource> {
    let pointer = unsafe { windows::Win32::System::Com::CoTaskMemAlloc(16) };
    assert!(!pointer.is_null());
    unsafe { OwnedResource::adopt(pointer as usize, Cleanup::CoTaskMemFree) }.unwrap()
}

#[test]
fn aliases_share_owner_close_state_and_exactly_one_cleanup() {
    for width in [1, 2] {
        for null in [false, true] {
            outcome_tests::reset();
            let owner = allocation();
            let address = owner.raw();
            let empty = [0u16];
            let string = if null {
                Value::Null
            } else {
                Value::Pointer(empty.as_ptr().cast_mut().cast())
            };
            let call = plan(
                alias as *const () as usize,
                alias_parameters(Cleanup::CoTaskMemFree),
                alias_contract(width),
            );
            let result =
                unsafe { call.invoke(&[Value::Resource(Arc::clone(&owner)), string]) }.unwrap();
            let alias = result.outputs[0].resource().unwrap();
            assert!(Arc::ptr_eq(alias, &owner));
            alias.close().unwrap();
            assert!(owner.is_closed());
            assert!(alias.is_closed());
            assert!(owner.lease(Cleanup::CoTaskMemFree).is_err());
            drop(result);
            drop(owner);
            assert_eq!(outcome_tests::cleanup_count_for(address), 1);
        }
    }
}

#[test]
fn dropping_an_alias_keeps_the_original_owner_live() {
    outcome_tests::reset();
    let owner = allocation();
    let address = owner.raw();
    let call = plan(
        alias as *const () as usize,
        alias_parameters(Cleanup::CoTaskMemFree),
        alias_contract(2),
    );
    let result =
        unsafe { call.invoke(&[Value::Resource(Arc::clone(&owner)), Value::Null]) }.unwrap();
    drop(result);
    assert!(!owner.is_closed());
    assert_eq!(outcome_tests::cleanup_count_for(address), 0);
    drop(owner);
    assert_eq!(outcome_tests::cleanup_count_for(address), 1);
}

#[test]
fn aliases_of_borrowed_inputs_remain_borrowed() {
    outcome_tests::reset();
    let call = plan(
        alias as *const () as usize,
        alias_parameters(Cleanup::RegCloseKey),
        alias_contract(2),
    );
    let result = unsafe { call.invoke(&[Value::Handle(0x8000_0002), Value::Null]) }.unwrap();
    assert!(matches!(result.outputs[0], Value::Handle(0x8000_0002)));
    drop(result);
    assert_eq!(outcome_tests::cleanup_count_for(0x8000_0002), 0);
}

#[test]
fn native_handle_conditions_preserve_the_complete_pointer_width() {
    let condition = Condition {
        inputs: vec![InputPredicate::HandleIn {
            parameter: 0,
            values: vec![-2147483646],
        }],
        return_value: None,
    };
    assert!(unsafe {
        condition.matches_inputs(&[Some(AbiValue::Pointer((-2147483646isize) as *mut c_void))])
    });
    assert!(!unsafe {
        condition.matches_inputs(&[Some(AbiValue::Pointer(0x8000_0002usize as *mut c_void))])
    });
}

#[test]
fn failed_and_mismatched_alias_outputs_never_close_the_input() {
    unsafe extern "system" fn fail(
        input: *mut c_void,
        _: *const c_void,
        output: *mut *mut c_void,
    ) -> i32 {
        unsafe { *output = input };
        5
    }
    unsafe extern "system" fn mismatch(
        input: *mut c_void,
        _: *const c_void,
        output: *mut *mut c_void,
    ) -> i32 {
        unsafe { *output = input.wrapping_byte_add(1) };
        0
    }
    outcome_tests::reset();
    let owner = allocation();
    let address = owner.raw();
    let failed = plan(
        fail as *const () as usize,
        alias_parameters(Cleanup::CoTaskMemFree),
        alias_contract(1),
    );
    let result =
        unsafe { failed.invoke(&[Value::Resource(Arc::clone(&owner)), Value::Null]) }.unwrap();
    assert!(!result.succeeded);
    assert!(matches!(result.outputs[0], Value::Handle(0)));
    assert!(outcome_tests::decoded_outputs().is_empty());
    let mismatched = plan(
        mismatch as *const () as usize,
        alias_parameters(Cleanup::CoTaskMemFree),
        alias_contract(1),
    );
    assert!(
        unsafe { mismatched.invoke(&[Value::Resource(Arc::clone(&owner)), Value::Null]) }
            .unwrap_err()
            .message()
            .contains("does not equal")
    );
    assert!(!owner.is_closed());
    assert_eq!(outcome_tests::cleanup_count_for(address), 0);
    owner.close().unwrap();
}

#[test]
fn conditions_snapshot_inputs_before_native_mutation() {
    unsafe extern "system" fn change(
        input: *mut c_void,
        text: *mut u8,
        output: *mut *mut c_void,
    ) -> i32 {
        unsafe {
            *text = b'x';
            *output = input;
        }
        0
    }
    let owner = allocation();
    let mut text = [0u8; 2];
    let call = plan(
        change as *const () as usize,
        alias_parameters(Cleanup::CoTaskMemFree),
        alias_contract(1),
    );
    let result = unsafe {
        call.invoke(&[
            Value::Resource(Arc::clone(&owner)),
            Value::Pointer(text.as_mut_ptr().cast()),
        ])
    }
    .unwrap();
    assert_eq!(text[0], b'x');
    assert!(Arc::ptr_eq(result.outputs[0].resource().unwrap(), &owner));
}

#[test]
fn unavailable_outputs_are_not_decoded_for_the_contracted_input_and_status() {
    unsafe extern "system" fn query(_: *mut c_void, status: i32, _: *mut u32) -> i32 {
        status
    }
    let call = plan(
        query as *const () as usize,
        vec![
            Parameter::input(Type::Handle, false),
            Parameter::input(Type::I32, false),
            Parameter::input_output(Type::U32, false, Cleanup::None),
        ],
        CallContract {
            outputs: vec![OutputRule {
                parameter: 2,
                when: Condition {
                    inputs: vec![InputPredicate::BitsIn {
                        parameter: 0,
                        mask: u32::MAX as u64,
                        values: vec![0x8000_0004],
                    }],
                    return_value: Some(234),
                },
                action: OutputAction::Unavailable {},
            }],
            ..CallContract::default()
        },
    );
    for (handle, status, unavailable) in [
        (0x8000_0004, 234, true),
        (0xffff_ffff_8000_0004, 234, true),
        (0x8000_0004, 0, false),
        (0x8000_0002, 234, false),
    ] {
        for capacity in [1, 64] {
            outcome_tests::reset();
            let result = unsafe {
                call.invoke(&[
                    Value::Handle(handle),
                    Value::I32(status),
                    Value::U32(capacity),
                ])
            }
            .unwrap();
            if unavailable {
                assert!(matches!(result.outputs[0], Value::Unavailable));
                assert!(outcome_tests::decoded_outputs().is_empty());
            } else {
                assert!(matches!(result.outputs[0], Value::U32(value) if value == capacity));
                assert_eq!(outcome_tests::decoded_outputs(), [0]);
            }
        }
    }
}

#[test]
fn unavailable_owned_output_is_neither_adopted_nor_cleaned_up() {
    unsafe extern "system" fn undefined(input: *mut c_void, output: *mut *mut c_void) -> i32 {
        unsafe { *output = input };
        234
    }
    outcome_tests::reset();
    let owner = allocation();
    let address = owner.raw();
    let call = plan(
        undefined as *const () as usize,
        vec![
            handle_input(Cleanup::CoTaskMemFree),
            Parameter::output(Type::Handle, Cleanup::CoTaskMemFree),
        ],
        CallContract {
            outputs: vec![OutputRule {
                parameter: 1,
                when: Condition {
                    return_value: Some(234),
                    ..Condition::default()
                },
                action: OutputAction::Unavailable {},
            }],
            ..CallContract::default()
        },
    );
    let result = unsafe { call.invoke(&[Value::Resource(Arc::clone(&owner))]) }.unwrap();
    assert!(matches!(result.outputs[0], Value::Unavailable));
    drop(result);
    assert!(outcome_tests::decoded_outputs().is_empty());
    assert_eq!(outcome_tests::cleanup_count_for(address), 0);
    owner.close().unwrap();
}

#[test]
fn malformed_contracts_fail_before_module_resolution() {
    let make_spec = || {
        let mut spec = spec(alias_parameters(Cleanup::RegCloseKey));
        spec.dll = "absent-dynwinrt-test.dll".into();
        spec
    };
    let mut cases = Vec::new();
    let mut invalid = alias_contract(1);
    invalid.outputs[0].parameter = 9;
    cases.push(invalid);
    let mut invalid = alias_contract(1);
    invalid.outputs.push(invalid.outputs[0].clone());
    cases.push(invalid);
    cases.push(alias_contract(4));
    let mut invalid = alias_contract(1);
    invalid.outputs[0].action = OutputAction::AliasInput { parameter: 1 };
    cases.push(invalid);
    let mut invalid = alias_contract(1);
    invalid.outputs[0].when.inputs = vec![InputPredicate::BitsIn {
        parameter: 0,
        mask: 0xff,
        values: vec![0x100],
    }];
    cases.push(invalid);
    let mut invalid = alias_contract(1);
    invalid.outputs[0].when.return_value = Some(u64::MAX);
    cases.push(invalid);
    cases.push(CallContract {
        resource_effects: vec![ResourceEffect::AddFileCompletionModes {
            handle_parameter: 0,
            flags_parameter: 1,
        }],
        ..CallContract::default()
    });
    for invalid in cases {
        let error = unsafe { CallPlan::new_with_contract(make_spec(), invalid) }.unwrap_err();
        assert!(
            !error.message().contains("absent-dynwinrt-test"),
            "{error:?}"
        );
    }
}

#[test]
fn resource_mode_effects_are_monotonic_success_only_and_exclude_async_leases() {
    use std::cell::Cell;
    thread_local! { static CALLS: Cell<usize> = const { Cell::new(0) }; }
    extern "system" fn set_modes(_: *mut c_void, flags: u8) -> i32 {
        CALLS.set(CALLS.get() + 1);
        i32::from(flags != u8::MAX)
    }
    let handle =
        unsafe { windows::Win32::System::Threading::CreateEventW(None, true, false, None) }
            .unwrap();
    let owner = unsafe { OwnedResource::adopt(handle.0 as usize, Cleanup::CloseHandle) }.unwrap();
    *owner.file_completion_modes.lock().unwrap() = Some(1);
    let mut call = Arc::try_unwrap(plan(
        set_modes as *const () as usize,
        vec![
            handle_input(Cleanup::CloseHandle),
            Parameter::input(Type::U8, false),
        ],
        CallContract {
            resource_effects: vec![ResourceEffect::AddFileCompletionModes {
                handle_parameter: 0,
                flags_parameter: 1,
            }],
            ..CallContract::default()
        },
    ))
    .unwrap();
    call.success_rule = SuccessRule::ReturnNonZero;
    let mut lease = owner.async_lease(Cleanup::CloseHandle).unwrap();
    for active in [false, true] {
        if active {
            lease.mark_active();
        }
        let error = unsafe { call.invoke(&[Value::Resource(Arc::clone(&owner)), Value::U8(2)]) }
            .unwrap_err();
        assert!(error.message().contains("completion modes"));
        assert_eq!(CALLS.get(), 0);
    }
    drop(lease);
    for (flags, succeeded) in [(2, true), (0, true), (u8::MAX, false)] {
        let result =
            unsafe { call.invoke(&[Value::Resource(Arc::clone(&owner)), Value::U8(flags)]) }
                .unwrap();
        assert_eq!(result.succeeded, succeeded);
        assert_eq!(*owner.file_completion_modes.lock().unwrap(), Some(3));
    }
    assert_eq!(CALLS.get(), 3);
    owner.close().unwrap();
}
