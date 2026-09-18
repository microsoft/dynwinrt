// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_OVERLAPPED, SetFileCompletionNotificationModes,
};

#[derive(Clone, Copy, Debug)]
enum Failure {
    None,
    Field(usize),
    DiscardCleanup,
    Return,
}

const FAILURES: [Failure; 5] = [
    Failure::None,
    Failure::Field(0),
    Failure::Field(1),
    Failure::DiscardCleanup,
    Failure::Return,
];

fn effect_plan(
    function: usize,
    parameters: Vec<Parameter>,
    layout: Arc<NativeAggregatePointerLayout>,
    failure: Failure,
    resource_effects: Vec<ResourceEffect>,
) -> Arc<CallPlan> {
    let mut spec = spec();
    spec.parameter_aggregates = vec![None; parameters.len()];
    spec.parameters = parameters;
    let mut pointees = vec![None; spec.parameters.len()];
    pointees[1] = Some(layout);
    let mut contract = CallContract {
        resource_effects,
        ..CallContract::default()
    }
    .upgrade_metadata(&contract::shape_with_pointees(&spec, &pointees))
    .unwrap();
    for result in &mut contract.results {
        if matches!(result.target, ResultTarget::AggregateField { .. }) {
            result.on_failure = result.on_success;
        }
        if matches!(failure, Failure::DiscardCleanup)
            && matches!(
                result.target,
                ResultTarget::AggregateField {
                    parameter: 1,
                    field: 0
                }
            )
        {
            let policy = ResultPolicy::discarded(ResultOwnership::Owned {
                cleanup: dynwinrt_win32_contracts::Cleanup::CloseHandle,
            });
            result.on_success = policy;
            result.on_failure = policy;
        }
    }
    let mut plan = Arc::try_unwrap(
        unsafe { CallPlan::new_with_contract_and_pointees(spec, contract, pointees) }.unwrap(),
    )
    .unwrap();
    plan.function = function;
    match failure {
        Failure::Field(field) => outcome_tests::fail_decode(ResultTarget::AggregateField {
            parameter: 1,
            field,
        }),
        Failure::Return => outcome_tests::fail_decode(ResultTarget::Return {}),
        Failure::None | Failure::DiscardCleanup => {}
    }
    Arc::new(plan)
}

#[test]
fn native_consumption_commits_before_fallible_aggregate_results() {
    unsafe extern "system" fn consume(
        input: *mut c_void,
        output: *mut NativeInfo,
        succeeded: i32,
        fail_cleanup: i32,
    ) -> i32 {
        unsafe {
            produce(output);
            if fail_cleanup != 0 {
                protect_handle((*output).process, true);
            }
            // Create outputs before consuming the input so their HANDLEs cannot reuse it.
            if succeeded != 0 {
                CloseHandle(HANDLE(input)).unwrap();
            }
        }
        succeeded
    }

    for succeeded in [false, true] {
        for failure in FAILURES {
            outcome_tests::reset();
            let handle =
                unsafe { windows::Win32::System::Threading::CreateEventW(None, true, false, None) }
                    .unwrap();
            let raw = handle.0 as usize;
            let input = unsafe { OwnedResource::adopt(raw, Cleanup::CloseHandle) }.unwrap();
            let alias = Arc::clone(&input);
            let layout = layout();
            let buffer = NativeAggregateBuffer::new(Arc::clone(&layout), None).unwrap();
            let plan = effect_plan(
                consume as *const () as usize,
                vec![
                    Parameter {
                        consumes_resource: true,
                        resource_cleanup: Cleanup::CloseHandle,
                        ..Parameter::input(Type::Handle, false)
                    },
                    Parameter::input(Type::Pointer, false),
                    Parameter::input(Type::Bool32, false),
                    Parameter::input(Type::Bool32, false),
                ],
                layout,
                failure,
                Vec::new(),
            );
            let discard = matches!(failure, Failure::DiscardCleanup);
            let result = unsafe {
                plan.invoke(&[
                    Value::Resource(Arc::clone(&input)),
                    Value::AggregatePointer(Arc::clone(&buffer)),
                    Value::Bool(succeeded),
                    Value::Bool(discard),
                ])
            };
            let returned_success = result.as_ref().ok().map(|result| result.succeeded);
            let expected_success = matches!(failure, Failure::None).then_some(succeeded);
            drop(result);
            let observed = (
                input.is_closed(),
                alias.raw(),
                input
                    .lease(Cleanup::CloseHandle)
                    .ok()
                    .map(|lease| lease.raw()),
                alias
                    .async_lease(Cleanup::CloseHandle)
                    .ok()
                    .map(|lease| lease.raw()),
            );
            let expected = if succeeded {
                (true, 0, None, None)
            } else {
                (false, raw, Some(raw), Some(raw))
            };
            // Observe first, then repair a stale fixture owner to avoid closing a reused HANDLE.
            if succeeded {
                *input.lock_for_call(ResourceAccess::Consume).unwrap() = 0;
            }
            let output_handles = native_handles(&buffer);
            if discard {
                protect_handle(output_handles[0], false);
            }
            drop(buffer);
            input.close().unwrap();
            input.close().unwrap();
            drop(alias);
            drop(input);
            let output_counts = retirement_counts(output_handles);
            assert_eq!(
                returned_success, expected_success,
                "{succeeded}, {failure:?}"
            );
            assert_eq!(observed, expected, "{succeeded}, {failure:?}");
            assert_eq!(
                outcome_tests::cleanup_count_for(raw),
                usize::from(!succeeded)
            );
            assert_eq!(
                outcome_tests::successful_cleanup_count_for(raw),
                usize::from(!succeeded)
            );
            assert_eq!(output_counts, [(if discard { 2 } else { 1 }, 1), (1, 1)]);
        }
    }
}

#[test]
fn native_resource_effects_commit_before_fallible_aggregate_results() {
    const FILE_SKIP_SET_EVENT_ON_HANDLE: u8 = 2;

    unsafe extern "system" fn set_modes(
        input: *mut c_void,
        output: *mut NativeInfo,
        modes: u8,
        succeeded: i32,
        fail_cleanup: i32,
    ) -> i32 {
        unsafe {
            produce(output);
            if fail_cleanup != 0 {
                protect_handle((*output).process, true);
            }
            if succeeded != 0 {
                SetFileCompletionNotificationModes(HANDLE(input), modes).unwrap();
            }
        }
        succeeded
    }

    for succeeded in [false, true] {
        for failure in FAILURES {
            outcome_tests::reset();
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(FILE_FLAG_OVERLAPPED.0)
                .open(std::env::current_exe().unwrap())
                .unwrap();
            let raw = file.into_raw_handle() as usize;
            let input = unsafe { OwnedResource::adopt(raw, Cleanup::CloseHandle) }.unwrap();
            let initial_modes = input
                .async_lease(Cleanup::CloseHandle)
                .unwrap()
                .completion_modes()
                .unwrap();
            assert_eq!(initial_modes & u32::from(FILE_SKIP_SET_EVENT_ON_HANDLE), 0);
            let layout = layout();
            let buffer = NativeAggregateBuffer::new(Arc::clone(&layout), None).unwrap();
            let plan = effect_plan(
                set_modes as *const () as usize,
                vec![
                    Parameter {
                        resource_cleanup: Cleanup::CloseHandle,
                        ..Parameter::input(Type::Handle, false)
                    },
                    Parameter::input(Type::Pointer, false),
                    Parameter::input(Type::U8, false),
                    Parameter::input(Type::Bool32, false),
                    Parameter::input(Type::Bool32, false),
                ],
                layout,
                failure,
                vec![ResourceEffect::AddFileCompletionModes {
                    handle_parameter: 0,
                    flags_parameter: 2,
                }],
            );
            let discard = matches!(failure, Failure::DiscardCleanup);
            let result = unsafe {
                plan.invoke(&[
                    Value::Resource(Arc::clone(&input)),
                    Value::AggregatePointer(Arc::clone(&buffer)),
                    Value::U8(FILE_SKIP_SET_EVENT_ON_HANDLE),
                    Value::Bool(succeeded),
                    Value::Bool(discard),
                ])
            };
            let returned_success = result.as_ref().ok().map(|result| result.succeeded);
            let expected_success = matches!(failure, Failure::None).then_some(succeeded);
            drop(result);
            let cached_modes = input.file_capability().cached_completion_modes();
            let native_modes = input
                .async_lease(Cleanup::CloseHandle)
                .unwrap()
                .completion_modes()
                .unwrap();
            let output_handles = native_handles(&buffer);
            if discard {
                protect_handle(output_handles[0], false);
            }
            drop(buffer);
            let still_open = !input.is_closed() && input.raw() == raw;
            input.close().unwrap();
            drop(input);
            let output_counts = retirement_counts(output_handles);
            let expected_modes = initial_modes
                | if succeeded {
                    u32::from(FILE_SKIP_SET_EVENT_ON_HANDLE)
                } else {
                    0
                };
            assert_eq!(
                returned_success, expected_success,
                "{succeeded}, {failure:?}"
            );
            assert_eq!(native_modes, expected_modes);
            assert_eq!(
                cached_modes,
                Some(expected_modes),
                "{succeeded}, {failure:?}"
            );
            assert!(still_open);
            assert_eq!(outcome_tests::cleanup_count_for(raw), 1);
            assert_eq!(outcome_tests::successful_cleanup_count_for(raw), 1);
            assert_eq!(output_counts, [(if discard { 2 } else { 1 }, 1), (1, 1)]);
        }
    }
}
