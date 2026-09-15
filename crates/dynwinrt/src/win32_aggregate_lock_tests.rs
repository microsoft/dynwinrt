// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use std::{
    process::Command,
    sync::{Barrier, mpsc},
    thread,
    time::{Duration, Instant},
};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcess};

fn event() -> Arc<OwnedResource> {
    let handle = unsafe { CreateEventW(None, true, false, None) }.unwrap();
    unsafe { OwnedResource::adopt(handle.0 as usize, Cleanup::CloseHandle) }.unwrap()
}

fn handle_layout() -> Arc<NativeAggregatePointerLayout> {
    NativeAggregatePointerLayout::new(
        "Tests.DuplicatedHandle".into(),
        size_of::<usize>(),
        align_of::<usize>(),
        vec![AggregateResultField {
            name: "handle".into(),
            offset: 0,
            typ: Type::Handle,
            cleanup: Cleanup::CloseHandle,
        }],
    )
    .unwrap()
}

fn duplicate_plan(layout: Arc<NativeAggregatePointerLayout>) -> Arc<CallPlan> {
    let mut spec = spec();
    spec.entry_point = "DuplicateHandle".into();
    spec.parameters = vec![
        Parameter::input(Type::Handle, false),
        Parameter {
            resource_cleanup: Cleanup::CloseHandle,
            ..Parameter::input(Type::Handle, false)
        },
        Parameter::input(Type::Handle, false),
        Parameter::input(Type::Pointer, false),
        Parameter::input(Type::U32, false),
        Parameter::input(Type::Bool32, false),
        Parameter::input(Type::U32, false),
    ];
    spec.parameter_aggregates = vec![None; 7];
    spec.capture_last_error = true;
    let mut pointees = vec![None; 7];
    pointees[3] = Some(layout);
    let contract = CallContract::default()
        .upgrade_metadata(&contract::shape_with_pointees(&spec, &pointees))
        .unwrap();
    unsafe { CallPlan::new_with_contract_and_pointees(spec, contract, pointees) }.unwrap()
}

fn duplicate(
    plan: &CallPlan,
    input: &Arc<OwnedResource>,
    buffer: &Arc<NativeAggregateBuffer>,
) -> std::result::Result<CallResult, CallError> {
    duplicate_with_access(plan, input, buffer, Value::U32(0))
}

fn duplicate_with_access(
    plan: &CallPlan,
    input: &Arc<OwnedResource>,
    buffer: &Arc<NativeAggregateBuffer>,
    access: Value,
) -> std::result::Result<CallResult, CallError> {
    let process = unsafe { GetCurrentProcess() }.0 as usize;
    unsafe {
        plan.invoke(&[
            Value::Handle(process),
            Value::Resource(Arc::clone(input)),
            Value::Handle(process),
            Value::AggregatePointer(Arc::clone(buffer)),
            access,
            Value::Bool(false),
            Value::U32(2),
        ])
    }
}

fn field_owner(buffer: &NativeAggregateBuffer) -> Arc<OwnedResource> {
    Arc::clone(buffer.field_value(0).unwrap().resource().unwrap())
}

fn reuse(case: &str) {
    let layout = handle_layout();
    let plan = duplicate_plan(Arc::clone(&layout));
    let buffer = NativeAggregateBuffer::new(layout, None).unwrap();
    let original = event();
    assert!(duplicate(&plan, &original, &buffer).unwrap().succeeded);
    let old = field_owner(&buffer);
    let before = buffer.bytes().unwrap();
    println!("[aggregate-locks] {case}: first DuplicateHandle succeeded");
    match case {
        "shared" => {
            let error = duplicate(&plan, &old, &buffer).unwrap_err();
            assert!(
                error.message().contains("aggregate output owner"),
                "{error:?}"
            );
            assert_eq!(buffer.bytes().unwrap(), before);
            assert!(Arc::ptr_eq(&field_owner(&buffer), &old));
            assert!(!old.is_closed());
            drop(old.lease(Cleanup::CloseHandle).unwrap());
            let taken = buffer.take_field(0).unwrap();
            assert!(
                duplicate(&plan, taken.resource().unwrap(), &buffer)
                    .unwrap()
                    .succeeded
            );
            assert!(!old.is_closed());
        }
        "take" => {
            let taken = buffer.take_field(0).unwrap();
            assert!(
                duplicate(&plan, taken.resource().unwrap(), &buffer)
                    .unwrap()
                    .succeeded
            );
            assert!(!old.is_closed());
        }
        "distinct" | "lock-order" => {
            outcome_tests::reset();
            assert!(duplicate(&plan, &original, &buffer).unwrap().succeeded);
            assert!(old.is_closed());
            assert!(!original.is_closed());
            if case == "lock-order" {
                let mut expected =
                    vec![Arc::as_ptr(&old) as usize, Arc::as_ptr(&original) as usize];
                expected.sort_unstable();
                assert_eq!(outcome_tests::call_lock_order(), expected);
            }
        }
        _ => unreachable!(),
    }
    assert!(!Arc::ptr_eq(&field_owner(&buffer), &old));
    drop(buffer);
    old.close().unwrap();
    original.close().unwrap();
}

fn borrowed_alias_reuse() {
    unsafe extern "system" fn alias(input: usize, output: *mut usize) -> i32 {
        unsafe { *output = input };
        1
    }
    let layout = handle_layout();
    let buffer = NativeAggregateBuffer::new(Arc::clone(&layout), None).unwrap();
    let original = event();
    let mut spec = spec();
    spec.parameters = vec![
        Parameter {
            resource_cleanup: Cleanup::CloseHandle,
            ..Parameter::input(Type::Handle, false)
        },
        Parameter::input(Type::Pointer, false),
    ];
    spec.parameter_aggregates = vec![None; 2];
    let pointees = vec![None, Some(layout)];
    let mut contract = CallContract::default()
        .upgrade_metadata(&contract::shape_with_pointees(&spec, &pointees))
        .unwrap();
    for result in &mut contract.results {
        if matches!(result.target, ResultTarget::AggregateField { .. }) {
            result.on_success =
                ResultPolicy::delivered(ResultOwnership::AliasInput { parameter: 0 });
        }
    }
    let mut plan = Arc::try_unwrap(
        unsafe { CallPlan::new_with_contract_and_pointees(spec, contract, pointees) }.unwrap(),
    )
    .unwrap();
    plan.function = alias as *const () as usize;
    for _ in 0..2 {
        assert!(
            unsafe {
                plan.invoke(&[
                    Value::Resource(Arc::clone(&original)),
                    Value::AggregatePointer(Arc::clone(&buffer)),
                ])
            }
            .unwrap()
            .succeeded
        );
        assert!(Arc::ptr_eq(&field_owner(&buffer), &original));
        assert!(!original.is_closed());
    }
    drop(buffer);
    assert!(!original.is_closed());
    original.close().unwrap();
}

fn conflict_preflight_preserves_all_old_outputs() {
    unsafe extern "system" fn outputs(
        _input: *mut c_void,
        first: *mut NativeInfo,
        second: *mut NativeInfo,
    ) -> i32 {
        unsafe {
            produce(first);
            produce(second);
        }
        1
    }
    let layout = layout();
    let first = NativeAggregateBuffer::new(Arc::clone(&layout), None).unwrap();
    let second = NativeAggregateBuffer::new(Arc::clone(&layout), None).unwrap();
    let original = event();
    let mut spec = spec();
    spec.parameters = vec![
        Parameter {
            resource_cleanup: Cleanup::CloseHandle,
            ..Parameter::input(Type::Handle, false)
        },
        Parameter::input(Type::Pointer, false),
        Parameter::input(Type::Pointer, false),
    ];
    spec.parameter_aggregates = vec![None; 3];
    let pointees = vec![None, Some(Arc::clone(&layout)), Some(layout)];
    let contract = CallContract::default()
        .upgrade_metadata(&contract::shape_with_pointees(&spec, &pointees))
        .unwrap();
    let mut plan = Arc::try_unwrap(
        unsafe { CallPlan::new_with_contract_and_pointees(spec, contract, pointees) }.unwrap(),
    )
    .unwrap();
    plan.function = outputs as *const () as usize;
    let invoke = |input: &Arc<OwnedResource>| unsafe {
        plan.invoke(&[
            Value::Resource(Arc::clone(input)),
            Value::AggregatePointer(Arc::clone(&first)),
            Value::AggregatePointer(Arc::clone(&second)),
        ])
    };
    assert!(invoke(&original).unwrap().succeeded);
    let owners = [
        Arc::clone(first.field_value(0).unwrap().resource().unwrap()),
        Arc::clone(first.field_value(1).unwrap().resource().unwrap()),
        Arc::clone(second.field_value(0).unwrap().resource().unwrap()),
        Arc::clone(second.field_value(1).unwrap().resource().unwrap()),
    ];
    let before = [first.bytes().unwrap(), second.bytes().unwrap()];
    let error = invoke(&owners[3]).unwrap_err();
    assert!(error.message().contains("aggregate output owner"));
    assert_eq!([first.bytes().unwrap(), second.bytes().unwrap()], before);
    for owner in &owners {
        assert!(!owner.is_closed());
    }
    drop(first);
    drop(second);
    for owner in owners {
        owner.close().unwrap();
    }
    original.close().unwrap();
}

fn failed_retirement_keeps_retryable_owner_and_closes_other_fields() {
    let layout = layout();
    let buffer = NativeAggregateBuffer::new(Arc::clone(&layout), None).unwrap();
    let plan = planned(layout);
    let invoke = || unsafe { plan.invoke(&[Value::AggregatePointer(Arc::clone(&buffer))]) };
    assert!(invoke().unwrap().succeeded);
    let first = Arc::clone(buffer.field_value(0).unwrap().resource().unwrap());
    let second = Arc::clone(buffer.field_value(1).unwrap().resource().unwrap());
    let raw = first.raw();
    protect_handle(raw, true);
    let failed = invoke().is_err();
    protect_handle(raw, false);
    assert!(failed);
    assert!(!first.is_closed());
    assert!(second.is_closed());
    assert_eq!(native_handles(&buffer), [raw, 0]);
    assert!(invoke().unwrap().succeeded);
    assert!(first.is_closed());
    assert!(second.is_closed());
    drop(buffer);
}

fn busy_or_invalid_calls_do_not_retire_outputs(case: &str) {
    let layout = handle_layout();
    let plan = duplicate_plan(Arc::clone(&layout));
    let buffer = NativeAggregateBuffer::new(layout, None).unwrap();
    let original = event();
    assert!(duplicate(&plan, &original, &buffer).unwrap().succeeded);
    let old = field_owner(&buffer);
    let before = buffer.bytes().unwrap();
    let lease = (case == "busy-retirement").then(|| old.async_lease(Cleanup::CloseHandle).unwrap());
    let error = if lease.is_some() {
        duplicate(&plan, &original, &buffer).unwrap_err()
    } else {
        duplicate_with_access(&plan, &original, &buffer, Value::Bool(false)).unwrap_err()
    };
    if lease.is_some() {
        assert!(error.message().contains("asynchronous I/O is pending"));
    }
    assert!(!old.is_closed());
    assert!(!original.is_closed());
    assert_eq!(buffer.bytes().unwrap(), before);
    drop(lease);
    assert!(duplicate(&plan, &original, &buffer).unwrap().succeeded);
    assert!(old.is_closed());
    drop(buffer);
    original.close().unwrap();
}

fn crossed_retirement() {
    fn worker(
        plan: Arc<CallPlan>,
        source: Arc<OwnedResource>,
        barrier: Arc<Barrier>,
        send: mpsc::Sender<Arc<OwnedResource>>,
        receive: mpsc::Receiver<Arc<OwnedResource>>,
    ) -> bool {
        let buffer = NativeAggregateBuffer::new(handle_layout(), None).unwrap();
        assert!(duplicate(&plan, &source, &buffer).unwrap().succeeded);
        send.send(field_owner(&buffer)).unwrap();
        let other = receive.recv().unwrap();
        barrier.wait();
        match duplicate(&plan, &other, &buffer) {
            Ok(result) => {
                assert!(result.succeeded);
                true
            }
            Err(error) => {
                assert!(error.message().contains("closed"), "{error:?}");
                false
            }
        }
    }

    let plan = duplicate_plan(handle_layout());
    for _ in 0..8 {
        let source = event();
        let barrier = Arc::new(Barrier::new(2));
        let (a_send, b_receive) = mpsc::channel();
        let (b_send, a_receive) = mpsc::channel();
        let (a_plan, a_source, a_barrier) =
            (Arc::clone(&plan), Arc::clone(&source), Arc::clone(&barrier));
        let a = thread::spawn(move || worker(a_plan, a_source, a_barrier, a_send, a_receive));
        let (b_plan, b_source) = (Arc::clone(&plan), Arc::clone(&source));
        let b = thread::spawn(move || worker(b_plan, b_source, barrier, b_send, b_receive));
        let successes = usize::from(a.join().unwrap()) + usize::from(b.join().unwrap());
        assert_eq!(successes, 1);
        source.close().unwrap();
    }
}

#[test]
fn aggregate_owner_lock_cases_complete_without_deadlock() {
    const CHILD_CASE: &str = "DYNWINRT_AGGREGATE_LOCK_CASE";
    match std::env::var(CHILD_CASE) {
        Ok(case) => {
            match case.as_str() {
                "take" | "distinct" | "shared" | "lock-order" => reuse(&case),
                "borrowed-alias" => borrowed_alias_reuse(),
                "preflight-conflict" => conflict_preflight_preserves_all_old_outputs(),
                "failed-retirement" => {
                    failed_retirement_keeps_retryable_owner_and_closes_other_fields()
                }
                "busy-retirement" | "invalid-input" => {
                    busy_or_invalid_calls_do_not_retire_outputs(&case)
                }
                "crossed-retirement" => crossed_retirement(),
                _ => panic!("Unknown aggregate lock case: {case}"),
            }
            return;
        }
        Err(std::env::VarError::NotPresent) => {}
        Err(error) => panic!("Invalid aggregate lock case: {error}"),
    }
    let name = std::any::type_name_of_val(&aggregate_owner_lock_cases_complete_without_deadlock);
    let name = name.split_once("::").unwrap().1;
    for case in [
        "take",
        "distinct",
        "shared",
        "borrowed-alias",
        "preflight-conflict",
        "failed-retirement",
        "busy-retirement",
        "invalid-input",
        "lock-order",
        "crossed-retirement",
    ] {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env(CHILD_CASE, case)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "{case}: child failed with {status}");
                break;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("{case}: aggregate owner lock case exceeded 10 seconds");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}
