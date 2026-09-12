// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use std::cell::{Cell, RefCell};

#[derive(Debug, PartialEq, Eq)]
struct CleanupCall {
    kind: Cleanup,
    value: usize,
    succeeded: bool,
}

#[derive(Default)]
struct Observations {
    cleanup: Vec<CleanupCall>,
    locks: Vec<usize>,
    fail_return_allocation: bool,
    decoded_outputs: Vec<usize>,
    fail_decode: Option<ResultTarget>,
}

thread_local! {
    static OBSERVED: RefCell<Observations> = RefCell::default();
    static ALLOCATED: Cell<[usize; 2]> = const { Cell::new([0; 2]) };
    static DISPATCHES: Cell<usize> = const { Cell::new(0) };
}

pub(super) fn record_cleanup(kind: Cleanup, value: usize, succeeded: bool) {
    OBSERVED.with(|observed| {
        observed.borrow_mut().cleanup.push(CleanupCall {
            kind,
            value,
            succeeded,
        });
    });
}

pub(super) fn record_call_lock(value: usize) {
    OBSERVED.with(|observed| observed.borrow_mut().locks.push(value));
}

pub(super) fn record_output_decode(index: usize) {
    OBSERVED.with(|observed| observed.borrow_mut().decoded_outputs.push(index));
}

pub(super) fn check_result_decode(target: ResultTarget) -> Result<()> {
    if OBSERVED.with(|observed| {
        let mut observed = observed.borrow_mut();
        if observed.fail_decode == Some(target) {
            observed.fail_decode = None;
            true
        } else {
            false
        }
    }) {
        return Err(invalid_argument(
            "Injected native result conversion failure",
        ));
    }
    Ok(())
}

pub(super) fn fail_decode(target: ResultTarget) {
    OBSERVED.with(|observed| observed.borrow_mut().fail_decode = Some(target));
}

pub(super) fn decoded_outputs() -> Vec<usize> {
    OBSERVED.with(|observed| observed.borrow().decoded_outputs.clone())
}

pub(super) fn cleanup_count_for(value: usize) -> usize {
    OBSERVED.with(|observed| {
        observed
            .borrow()
            .cleanup
            .iter()
            .filter(|call| call.value == value)
            .count()
    })
}

pub(super) fn check_aggregate_return_allocation() -> Result<()> {
    if OBSERVED.with(|observed| std::mem::take(&mut observed.borrow_mut().fail_return_allocation)) {
        Err(out_of_memory(
            "native aggregate return (injected allocation failure)",
        ))
    } else {
        Ok(())
    }
}

pub(super) fn reset() {
    OBSERVED.with(|observed| *observed.borrow_mut() = Observations::default());
    ALLOCATED.set([0; 2]);
    DISPATCHES.set(0);
}

fn cleanups() -> Vec<CleanupCall> {
    OBSERVED.with(|observed| std::mem::take(&mut observed.borrow_mut().cleanup))
}

#[cfg(target_pointer_width = "64")]
fn policy(plan: Arc<CallPlan>, rule: SuccessRule, cleanup: Cleanup) -> Arc<CallPlan> {
    let mut plan = Arc::try_unwrap(plan).unwrap();
    plan.success_rule = rule;
    let spec = CallPlanSpec {
        dll: plan.dll.clone(),
        entry_point: plan.entry_point.clone(),
        parameters: plan
            .parameters
            .iter()
            .map(|parameter| parameter.spec)
            .collect(),
        return_type: plan.return_type,
        return_cleanup: cleanup,
        success_rule: rule,
        capture_last_error: plan.capture_last_error,
        calling_convention: plan.calling_convention,
        parameter_aggregates: plan.parameter_aggregates.clone(),
        return_aggregate: plan.return_aggregate.clone(),
    };
    plan.contract = contract::resolve(&spec, &CallContract::default()).unwrap();
    plan.legacy_surface = contract::LegacySurface::new(&spec, &CallContract::default());
    Arc::new(plan)
}

#[test]
#[cfg(target_pointer_width = "64")]
fn every_success_rule_classifies_native_results_without_erasing_them() {
    extern "system" fn integer(value: i32) -> i32 {
        value
    }
    extern "system" fn pointer(value: *mut c_void) -> *mut c_void {
        value
    }
    for rule in [
        SuccessRule::Always,
        SuccessRule::ReturnZero,
        SuccessRule::ReturnNonZero,
        SuccessRule::HResultSucceeded,
        SuccessRule::SignedNonNegative,
    ] {
        let plan = policy(
            super::tests::fake_plan(
                integer as *const () as usize,
                vec![Parameter::input(Type::I32, false)],
                Some(Type::I32),
                vec![None],
                None,
            ),
            rule,
            Cleanup::None,
        );
        for input in [i32::MIN, -1, 0, 1, i32::MAX] {
            let result = unsafe { plan.invoke(&[Value::I32(input)]) }.unwrap();
            let expected = match rule {
                SuccessRule::Always => true,
                SuccessRule::ReturnZero => input == 0,
                SuccessRule::ReturnNonZero => input != 0,
                SuccessRule::HResultSucceeded | SuccessRule::SignedNonNegative => input >= 0,
                _ => unreachable!(),
            };
            assert_eq!(result.succeeded, expected, "{rule:?}, {input}");
            assert!(matches!(result.return_value, Some(Value::I32(value)) if value == input));
            assert!(result.outputs.is_empty());
            assert_eq!(result.last_error, None);
        }
    }
    let mut byte = 0u8;
    let valid = (&mut byte as *mut u8) as usize;
    for rule in [
        SuccessRule::Always,
        SuccessRule::ReturnZero,
        SuccessRule::ReturnNonZero,
        SuccessRule::ReturnNonNull,
        SuccessRule::ReturnValidHandle,
    ] {
        let plan = policy(
            super::tests::fake_plan(
                pointer as *const () as usize,
                vec![Parameter::input(Type::Handle, true)],
                Some(Type::Handle),
                vec![None],
                None,
            ),
            rule,
            Cleanup::None,
        );
        for input in [0, valid, usize::MAX] {
            let result = unsafe { plan.invoke(&[Value::Handle(input)]) }.unwrap();
            let expected = match rule {
                SuccessRule::Always => true,
                SuccessRule::ReturnZero => input == 0,
                SuccessRule::ReturnNonZero | SuccessRule::ReturnNonNull => input != 0,
                SuccessRule::ReturnValidHandle => input != 0 && input != usize::MAX,
                _ => unreachable!(),
            };
            assert_eq!(result.succeeded, expected, "{rule:?}, {input}");
            assert!(matches!(result.return_value, Some(Value::Handle(value)) if value == input));
        }
    }
}

#[test]
#[cfg(target_pointer_width = "64")]
fn mixed_directions_keep_native_output_order_and_original_inputs() {
    extern "system" fn mix(
        input: i32,
        byte: *mut u8,
        wide: u64,
        narrow: *mut i16,
        floating: *mut f64,
        pointer: *mut *mut c_void,
    ) -> i32 {
        DISPATCHES.set(DISPATCHES.get() + 1);
        unsafe {
            if *byte != 0 || *floating != 0.0 {
                return E_INVALIDARG.0;
            }
            *byte = wide as u8;
            *narrow += input as i16;
            *floating = 6.25;
            *pointer = std::ptr::null_mut();
        }
        1
    }
    reset();
    let plan = policy(
        super::tests::fake_plan(
            mix as *const () as usize,
            vec![
                Parameter::input(Type::I32, false),
                Parameter::output(Type::U8, Cleanup::None),
                Parameter::input(Type::U64, false),
                Parameter::input_output(Type::I16, false, Cleanup::None),
                Parameter::output(Type::F64, Cleanup::None),
                Parameter::input_output(Type::Pointer, true, Cleanup::None),
            ],
            Some(Type::I32),
            vec![None; 6],
            None,
        ),
        SuccessRule::HResultSucceeded,
        Cleanup::None,
    );
    let mut anchor = 0u8;
    let pointer = (&mut anchor as *mut u8).cast();
    let inputs = [
        Value::I32(23),
        Value::U64(7),
        Value::I16(-12),
        Value::Pointer(pointer),
    ];
    let result = unsafe { plan.invoke(&inputs) }.unwrap();
    assert!(result.succeeded);
    assert_eq!((plan.input_count(), plan.output_count()), (4, 4));
    assert!(matches!(result.return_value, Some(Value::I32(1))));
    assert!(matches!(result.outputs.as_slice(), [
        Value::U8(7), Value::I16(11), Value::F64(value), Value::Pointer(pointer),
    ] if *value == 6.25 && pointer.is_null()));
    assert!(matches!(inputs[2], Value::I16(-12)));
    assert!(matches!(inputs[3], Value::Pointer(value) if value == pointer));
    assert!(unsafe { plan.invoke(&inputs[..3]) }.is_err());
    let mut wrong = inputs.clone();
    wrong[1] = Value::U32(7);
    assert!(unsafe { plan.invoke(&wrong) }.is_err());
    assert_eq!(DISPATCHES.get(), 1);
}

#[cfg(target_pointer_width = "64")]
extern "system" fn allocate_outputs(
    first: *mut *mut c_void,
    scalar: *mut u32,
    status: i32,
    last: *mut *mut c_void,
) -> i32 {
    use windows::Win32::System::Com::{CoTaskMemAlloc, CoTaskMemFree};
    unsafe {
        if !(*first).is_null() || !(*last).is_null() || *scalar != 0 {
            return E_INVALIDARG.0;
        }
        let a = CoTaskMemAlloc(16);
        let b = CoTaskMemAlloc(16);
        if a.is_null() || b.is_null() {
            CoTaskMemFree(Some(a));
            CoTaskMemFree(Some(b));
            return 0x8007000eu32 as i32;
        }
        a.cast::<u32>().write(0x1122_3344);
        b.cast::<u32>().write(0x5566_7788);
        *first = a;
        *last = b;
        *scalar = 7;
        ALLOCATED.set([a as usize, b as usize]);
    }
    status
}

#[test]
#[cfg(target_pointer_width = "64")]
fn owning_multi_outputs_cleanup_on_failure_and_last_alias_drop() {
    let plan = policy(
        super::tests::fake_plan(
            allocate_outputs as *const () as usize,
            vec![
                Parameter::output(Type::Pointer, Cleanup::CoTaskMemFree),
                Parameter::output(Type::U32, Cleanup::None),
                Parameter::input(Type::I32, false),
                Parameter::output(Type::Pointer, Cleanup::CoTaskMemFree),
            ],
            Some(Type::I32),
            vec![None; 4],
            None,
        ),
        SuccessRule::HResultSucceeded,
        Cleanup::None,
    );
    for status in [0, 1, 0x80004005u32 as i32] {
        reset();
        let result = unsafe { plan.invoke(&[Value::I32(status)]) }.unwrap();
        let [first, last] = ALLOCATED.get();
        assert_ne!(first, 0);
        assert_ne!(last, 0);
        assert!(matches!(result.outputs[1], Value::U32(7)));
        assert_eq!(result.succeeded, status >= 0);
        if status < 0 {
            assert!(matches!(result.outputs[0], Value::Handle(0)));
            assert!(matches!(result.outputs[2], Value::Handle(0)));
            assert_eq!(
                cleanups(),
                vec![
                    CleanupCall {
                        kind: Cleanup::CoTaskMemFree,
                        value: first,
                        succeeded: true
                    },
                    CleanupCall {
                        kind: Cleanup::CoTaskMemFree,
                        value: last,
                        succeeded: true
                    },
                ]
            );
            drop(result);
            assert!(cleanups().is_empty());
        } else {
            assert!(cleanups().is_empty());
            let alias = Arc::clone(result.outputs[0].resource().unwrap());
            assert_eq!(unsafe { (alias.raw() as *const u32).read() }, 0x1122_3344);
            drop(result);
            assert_eq!(
                cleanups(),
                vec![CleanupCall {
                    kind: Cleanup::CoTaskMemFree,
                    value: last,
                    succeeded: true
                },]
            );
            alias.close().unwrap();
            alias.close().unwrap();
            drop(alias);
            assert_eq!(
                cleanups(),
                vec![CleanupCall {
                    kind: Cleanup::CoTaskMemFree,
                    value: first,
                    succeeded: true
                },]
            );
        }
    }
}

#[test]
#[cfg(target_pointer_width = "64")]
fn aggregate_conversion_failure_cleans_all_initialized_owned_outputs() {
    #[repr(C)]
    struct Summary {
        status: i32,
        count: u32,
    }
    extern "system" fn produce(first: *mut *mut c_void, last: *mut *mut c_void) -> Summary {
        let mut count = 0;
        let status = allocate_outputs(first, &mut count, 0, last);
        Summary { status, count }
    }
    let layout = NativeAggregateLayout::new(
        "Tests.Summary",
        8,
        4,
        FfiType::structure([FfiType::i32(), FfiType::u32()]),
    )
    .unwrap();
    let plan = super::tests::fake_plan(
        produce as *const () as usize,
        vec![Parameter::output(Type::Pointer, Cleanup::CoTaskMemFree); 2],
        None,
        vec![None; 2],
        Some(layout),
    );
    reset();
    OBSERVED.with(|observed| observed.borrow_mut().fail_return_allocation = true);
    let error = unsafe { plan.invoke(&[]) }.unwrap_err();
    assert!(error.message().contains("injected allocation failure"));
    let [first, last] = ALLOCATED.get();
    assert_ne!(first, 0);
    assert_ne!(last, 0);
    assert_eq!(
        cleanups(),
        vec![
            CleanupCall {
                kind: Cleanup::CoTaskMemFree,
                value: first,
                succeeded: true
            },
            CleanupCall {
                kind: Cleanup::CoTaskMemFree,
                value: last,
                succeeded: true
            },
        ]
    );
}

#[cfg(target_pointer_width = "64")]
extern "system" fn allocate_mixed(first: *mut HLOCAL, last: *mut *mut c_void, status: i32) -> i32 {
    use windows::Win32::System::Memory::{LMEM_FIXED, LocalAlloc};
    let local = match unsafe { LocalAlloc(LMEM_FIXED, 16) } {
        Ok(value) => value,
        Err(error) => return error.code().0,
    };
    let task = unsafe { windows::Win32::System::Com::CoTaskMemAlloc(16) };
    if task.is_null() {
        unsafe { LocalFree(Some(local)) };
        return 0x8007000eu32 as i32;
    }
    unsafe {
        first.write(local);
        last.write(task);
    }
    ALLOCATED.set([local.0 as usize, task as usize]);
    status
}

#[test]
#[cfg(target_pointer_width = "64")]
fn heterogeneous_output_cleanup_plans_do_not_share_allocator_policy() {
    let plan = policy(
        super::tests::fake_plan(
            allocate_mixed as *const () as usize,
            vec![
                Parameter::output(Type::Handle, Cleanup::LocalFree),
                Parameter::output(Type::Pointer, Cleanup::CoTaskMemFree),
                Parameter::input(Type::I32, false),
            ],
            Some(Type::I32),
            vec![None; 3],
            None,
        ),
        SuccessRule::HResultSucceeded,
        Cleanup::None,
    );
    for status in [0, 0x80004005u32 as i32] {
        reset();
        let result = unsafe { plan.invoke(&[Value::I32(status)]) }.unwrap();
        let [first, last] = ALLOCATED.get();
        assert_ne!(first, 0);
        assert_ne!(last, 0);
        if status == 0 {
            assert_eq!(
                result.outputs[0].resource().unwrap().cleanup(),
                Cleanup::LocalFree
            );
            assert_eq!(
                result.outputs[1].resource().unwrap().cleanup(),
                Cleanup::CoTaskMemFree
            );
            assert!(cleanups().is_empty());
        } else {
            assert!(matches!(
                result.outputs.as_slice(),
                [Value::Handle(0), Value::Handle(0)]
            ));
        }
        drop(result);
        assert_eq!(
            cleanups(),
            vec![
                CleanupCall {
                    kind: Cleanup::LocalFree,
                    value: first,
                    succeeded: true
                },
                CleanupCall {
                    kind: Cleanup::CoTaskMemFree,
                    value: last,
                    succeeded: true
                },
            ]
        );
    }
}

fn allocated_resource(kind: Cleanup) -> Arc<OwnedResource> {
    use windows::Win32::Security::Credentials::{
        CredMarshalCredentialW, USERNAME_TARGET_CREDENTIAL_INFO, UsernameTargetCredential,
    };
    use windows::Win32::System::Memory::{GMEM_FIXED, GlobalAlloc, LMEM_FIXED, LocalAlloc};
    use windows::Win32::System::Services::{OpenSCManagerW, SC_MANAGER_CONNECT};
    use windows::Win32::System::Threading::CreateEventW;
    use windows::core::{PWSTR, w};
    let raw = match kind {
        Cleanup::CloseHandle => {
            unsafe { CreateEventW(None, true, false, None) }.unwrap().0 as usize
        }
        Cleanup::RegCloseKey => {
            use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, KEY_READ, RegOpenKeyExW};
            let mut key = HKEY::default();
            unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, w!("SOFTWARE"), None, KEY_READ, &mut key) }
                .ok()
                .unwrap();
            key.0 as usize
        }
        Cleanup::LocalFree => unsafe { LocalAlloc(LMEM_FIXED, 16) }.unwrap().0 as usize,
        Cleanup::GlobalFree => unsafe { GlobalAlloc(GMEM_FIXED, 16) }.unwrap().0 as usize,
        Cleanup::FreeLibrary => {
            unsafe {
                windows::Win32::System::LibraryLoader::LoadLibraryExW(
                    w!("version.dll"),
                    None,
                    windows::Win32::System::LibraryLoader::LOAD_LIBRARY_SEARCH_SYSTEM32,
                )
            }
            .unwrap()
            .0 as usize
        }
        Cleanup::CloseServiceHandle => {
            unsafe { OpenSCManagerW(None, None, SC_MANAGER_CONNECT) }
                .unwrap()
                .0 as usize
        }
        Cleanup::CoTaskMemFree => {
            (unsafe { windows::Win32::System::Com::CoTaskMemAlloc(16) }) as usize
        }
        Cleanup::CredFree => {
            let mut name: Vec<u16> = "dynwinrt-unit-test".encode_utf16().chain(Some(0)).collect();
            let info = USERNAME_TARGET_CREDENTIAL_INFO {
                UserName: PWSTR(name.as_mut_ptr()),
            };
            let mut text = PWSTR::null();
            unsafe {
                CredMarshalCredentialW(
                    UsernameTargetCredential,
                    std::ptr::from_ref(&info).cast(),
                    &mut text,
                )
            }
            .unwrap();
            text.0 as usize
        }
        Cleanup::None => panic!("test resource must have a real cleanup"),
    };
    assert_ne!(raw, 0);
    unsafe { OwnedResource::adopt(raw, kind) }.unwrap()
}

#[test]
fn every_cleanup_kind_uses_its_real_allocator_once_on_close_and_drop() {
    for kind in [
        Cleanup::CloseHandle,
        Cleanup::RegCloseKey,
        Cleanup::LocalFree,
        Cleanup::GlobalFree,
        Cleanup::FreeLibrary,
        Cleanup::CloseServiceHandle,
        Cleanup::CoTaskMemFree,
        Cleanup::CredFree,
    ] {
        for explicit in [false, true] {
            let resource = allocated_resource(kind);
            let raw = resource.raw();
            let alias = Arc::clone(&resource);
            reset();
            drop(resource);
            assert!(cleanups().is_empty());
            if explicit {
                alias.close().unwrap();
                assert!(alias.is_closed());
                alias.close().unwrap();
            }
            drop(alias);
            assert_eq!(
                cleanups(),
                vec![CleanupCall {
                    kind,
                    value: raw,
                    succeeded: true
                }]
            );
        }
        reset();
        unsafe { cleanup_owned_resource(0, kind) }.unwrap();
        assert!(cleanups().is_empty());
    }
    let mut value = 123u32;
    unsafe { cleanup_owned_resource((&mut value as *mut u32) as usize, Cleanup::None) }.unwrap();
    assert_eq!(value, 123);
    assert!(cleanups().is_empty());
}

struct CloseProtection(Arc<OwnedResource>);

impl Drop for CloseProtection {
    fn drop(&mut self) {
        use windows::Win32::Foundation::{
            HANDLE_FLAG_PROTECT_FROM_CLOSE, HANDLE_FLAGS, SetHandleInformation,
        };
        let raw = self.0.raw();
        if raw != 0 {
            let _ = unsafe {
                SetHandleInformation(
                    HANDLE(raw as *mut c_void),
                    HANDLE_FLAG_PROTECT_FROM_CLOSE.0,
                    HANDLE_FLAGS(0),
                )
            };
        }
    }
}

#[test]
fn protected_handle_close_failure_can_be_retried_without_losing_ownership() {
    use windows::Win32::Foundation::{
        HANDLE_FLAG_PROTECT_FROM_CLOSE, HANDLE_FLAGS, SetHandleInformation,
    };
    let resource = allocated_resource(Cleanup::CloseHandle);
    let protection = CloseProtection(Arc::clone(&resource));
    let raw = resource.raw();
    unsafe {
        SetHandleInformation(
            HANDLE(raw as *mut c_void),
            HANDLE_FLAG_PROTECT_FROM_CLOSE.0,
            HANDLE_FLAG_PROTECT_FROM_CLOSE,
        )
    }
    .unwrap();
    reset();
    assert!(resource.close().is_err());
    assert_eq!(resource.raw(), raw);
    assert!(!resource.is_closed());
    unsafe {
        SetHandleInformation(
            HANDLE(raw as *mut c_void),
            HANDLE_FLAG_PROTECT_FROM_CLOSE.0,
            HANDLE_FLAGS(0),
        )
    }
    .unwrap();
    resource.close().unwrap();
    resource.close().unwrap();
    drop(protection);
    drop(resource);
    assert_eq!(
        cleanups(),
        vec![
            CleanupCall {
                kind: Cleanup::CloseHandle,
                value: raw,
                succeeded: false
            },
            CleanupCall {
                kind: Cleanup::CloseHandle,
                value: raw,
                succeeded: true
            },
        ]
    );
}

#[test]
#[cfg(target_pointer_width = "64")]
fn duplicate_aliases_reject_before_dispatch_and_distinct_handles_lock_in_one_order() {
    extern "system" fn compare(first: *mut c_void, second: *mut c_void) -> i32 {
        DISPATCHES.set(DISPATCHES.get() + 1);
        i32::from(first != second)
    }
    let a = allocated_resource(Cleanup::CloseHandle);
    let b = allocated_resource(Cleanup::CloseHandle);
    let mut parameter = Parameter::input(Type::Handle, false);
    parameter.resource_cleanup = Cleanup::CloseHandle;
    let plan = super::tests::fake_plan(
        compare as *const () as usize,
        vec![parameter; 2],
        Some(Type::Bool32),
        vec![None; 2],
        None,
    );
    reset();
    assert!(
        unsafe {
            plan.invoke(&[
                Value::Resource(Arc::clone(&a)),
                Value::Resource(Arc::clone(&a)),
            ])
        }
        .unwrap_err()
        .message()
        .contains("multiple parameters")
    );
    assert_eq!(DISPATCHES.get(), 0);
    assert!(OBSERVED.with(|observed| observed.borrow().locks.is_empty()));
    let mut expected = vec![Arc::as_ptr(&a) as usize, Arc::as_ptr(&b) as usize];
    expected.sort_unstable();
    for (first, second) in [(&a, &b), (&b, &a)] {
        OBSERVED.with(|observed| observed.borrow_mut().locks.clear());
        let result = unsafe {
            plan.invoke(&[
                Value::Resource(Arc::clone(first)),
                Value::Resource(Arc::clone(second)),
            ])
        }
        .unwrap();
        assert!(matches!(result.return_value, Some(Value::Bool(true))));
        assert_eq!(
            OBSERVED.with(|observed| observed.borrow().locks.clone()),
            expected
        );
    }
    assert_eq!(DISPATCHES.get(), 2);
    a.close().unwrap();
    b.close().unwrap();
}

#[test]
fn async_leases_keep_resources_open_and_active_counts_balance_once() {
    let resource = allocated_resource(Cleanup::CloseHandle);
    let weak = Arc::downgrade(&resource);
    reset();
    assert!(resource.async_lease(Cleanup::GlobalFree).is_err());
    assert!(!resource.has_async_leases());
    let mut first = resource.async_lease(Cleanup::CloseHandle).unwrap();
    let mut second = resource.async_lease(Cleanup::CloseHandle).unwrap();
    first.mark_active();
    first.mark_active();
    second.mark_active();
    assert_eq!(resource.active_async_io_count(), 2);
    assert_eq!(resource.async_lease_count(), 2);
    assert!(resource.close().is_err());
    first.mark_inactive();
    first.mark_inactive();
    assert_eq!(resource.active_async_io_count(), 1);
    drop(first);
    assert_eq!(resource.async_lease_count(), 1);
    let raw = resource.raw();
    drop(resource);
    assert!(weak.upgrade().is_some());
    assert!(cleanups().is_empty());
    second.mark_inactive();
    drop(second);
    assert!(weak.upgrade().is_none());
    assert_eq!(
        cleanups(),
        vec![CleanupCall {
            kind: Cleanup::CloseHandle,
            value: raw,
            succeeded: true
        },]
    );
}

#[test]
#[cfg(target_pointer_width = "64")]
fn consumption_requires_ownership_and_native_success_and_rejects_reuse() {
    use windows::Win32::Foundation::{
        HANDLE_FLAG_PROTECT_FROM_CLOSE, HANDLE_FLAGS, SetHandleInformation,
    };
    extern "system" fn close(handle: *mut c_void) -> i32 {
        DISPATCHES.set(DISPATCHES.get() + 1);
        i32::from(unsafe { CloseHandle(HANDLE(handle)) }.is_ok())
    }
    let resource = allocated_resource(Cleanup::CloseHandle);
    let protection = CloseProtection(Arc::clone(&resource));
    let raw = resource.raw();
    let parameter = Parameter {
        consumes_resource: true,
        resource_cleanup: Cleanup::CloseHandle,
        ..Parameter::input(Type::Handle, false)
    };
    let plan = policy(
        super::tests::fake_plan(
            close as *const () as usize,
            vec![parameter],
            Some(Type::Bool32),
            vec![None],
            None,
        ),
        SuccessRule::ReturnNonZero,
        Cleanup::None,
    );
    reset();
    assert!(
        unsafe { plan.invoke(&[Value::Handle(raw)]) }
            .unwrap_err()
            .message()
            .contains("managed resource")
    );
    assert_eq!(DISPATCHES.get(), 0);
    unsafe {
        SetHandleInformation(
            HANDLE(raw as *mut c_void),
            HANDLE_FLAG_PROTECT_FROM_CLOSE.0,
            HANDLE_FLAG_PROTECT_FROM_CLOSE,
        )
    }
    .unwrap();
    let failed = unsafe { plan.invoke(&[Value::Resource(Arc::clone(&resource))]) }.unwrap();
    assert!(!failed.succeeded);
    assert!(matches!(failed.return_value, Some(Value::Bool(false))));
    assert_eq!(resource.raw(), raw);
    unsafe {
        SetHandleInformation(
            HANDLE(raw as *mut c_void),
            HANDLE_FLAG_PROTECT_FROM_CLOSE.0,
            HANDLE_FLAGS(0),
        )
    }
    .unwrap();
    let closed = unsafe { plan.invoke(&[Value::Resource(Arc::clone(&resource))]) }.unwrap();
    assert!(closed.succeeded);
    assert!(resource.is_closed());
    assert!(
        unsafe { plan.invoke(&[Value::Resource(Arc::clone(&resource))]) }
            .unwrap_err()
            .message()
            .contains("closed")
    );
    assert_eq!(DISPATCHES.get(), 2);
    resource.close().unwrap();
    drop(protection);
    drop(resource);
    assert!(cleanups().is_empty());
}

#[test]
fn required_pointer_categories_reject_null_bits_without_rejecting_handle_sentinels() {
    for (typ, null) in [
        (Type::Pointer, Value::Pointer(std::ptr::null_mut())),
        (Type::FunctionPointer, Value::FunctionPointer(0)),
    ] {
        assert!(
            value_to_abi(typ, &null, false, Cleanup::None, None).is_err(),
            "{typ:?}"
        );
        assert!(value_to_abi(typ, &Value::Null, false, Cleanup::None, None).is_err());
        assert!(matches!(
            value_to_abi(typ, &null, true, Cleanup::None, None).unwrap(),
            AbiValue::Pointer(pointer) if pointer.is_null()
        ));
        assert!(value_to_abi(typ, &Value::Null, true, Cleanup::None, None).is_ok());
    }
    assert!(value_to_abi(Type::Handle, &Value::Handle(0), false, Cleanup::None, None).is_ok());
    assert!(
        value_to_abi(
            Type::Handle,
            &Value::Handle(usize::MAX),
            false,
            Cleanup::None,
            None
        )
        .is_ok()
    );
}

#[test]
#[cfg(target_pointer_width = "64")]
fn invalid_plan_contracts_fail_before_module_loading_or_native_dispatch() {
    let base = || CallPlanSpec {
        dll: "kernel32.dll".into(),
        entry_point: "GetLastError".into(),
        parameters: Vec::new(),
        return_type: Some(Type::U32),
        return_cleanup: Cleanup::None,
        success_rule: SuccessRule::Always,
        capture_last_error: false,
        calling_convention: CallingConvention::System,
        parameter_aggregates: Vec::new(),
        return_aggregate: None,
    };
    let mutations: &[fn(&mut CallPlanSpec)] = &[
        |spec| spec.dll = r"..\kernel32.dll".into(),
        |spec| spec.entry_point = "GetLastError\0other".into(),
        |spec| spec.parameter_aggregates.push(None),
        |spec| {
            spec.parameters = vec![Parameter::input(Type::U32, false); 1025];
            spec.parameter_aggregates = vec![None; 1025];
        },
        |spec| spec.return_cleanup = Cleanup::CoTaskMemFree,
        |spec| {
            spec.return_type = None;
            spec.success_rule = SuccessRule::ReturnZero;
        },
        |spec| spec.success_rule = SuccessRule::HResultSucceeded,
        |spec| spec.success_rule = SuccessRule::ReturnValidHandle,
        |spec| {
            spec.return_type = Some(Type::F64);
            spec.success_rule = SuccessRule::ReturnNonZero;
        },
        |spec| {
            spec.parameters.push(Parameter::input(Type::I32, true));
            spec.parameter_aggregates.push(None);
        },
        |spec| {
            let mut parameter = Parameter::input(Type::Pointer, false);
            parameter.cleanup = Cleanup::CoTaskMemFree;
            spec.parameters.push(parameter);
            spec.parameter_aggregates.push(None);
        },
        |spec| {
            let mut parameter = Parameter::input(Type::Handle, false);
            parameter.consumes_resource = true;
            spec.parameters.push(parameter);
            spec.parameter_aggregates.push(None);
        },
        |spec| {
            let mut parameter = Parameter::output(Type::Handle, Cleanup::None);
            parameter.resource_cleanup = Cleanup::CloseHandle;
            spec.parameters.push(parameter);
            spec.parameter_aggregates.push(None);
        },
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut spec = base();
        mutate(&mut spec);
        assert!(
            validate_spec(&spec).is_err(),
            "invalid contract case {index}"
        );
    }
}

#[test]
fn bounded_native_layout_rejects_invalid_identity_extent_and_alignment() {
    for (identity, size, alignment) in [
        ("", 8, 8),
        ("  ", 8, 8),
        ("Tests.Empty", 0, 8),
        ("Tests.Unaligned", 8, 0),
        ("Tests.NonPower", 12, 3),
        ("Tests.BadExtent", 12, 8),
        ("Tests.OverAligned", 16, 16),
    ] {
        assert!(
            NativeAggregateLayout::new(
                identity,
                size,
                alignment,
                FfiType::structure([FfiType::u64()]),
            )
            .is_err()
        );
    }
}

#[test]
fn simultaneous_close_and_lease_have_one_complete_ownership_outcome() {
    use std::sync::{Barrier, mpsc};
    use std::time::Duration;
    let resource = allocated_resource(Cleanup::CloseHandle);
    let start = Arc::new(Barrier::new(3));
    let (close_tx, close_rx) = mpsc::channel();
    let (lease_tx, lease_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let closer = Arc::clone(&resource);
    let close_start = Arc::clone(&start);
    let close_thread = std::thread::spawn(move || {
        close_start.wait();
        close_tx.send(closer.close().is_ok()).unwrap();
    });
    let borrower = Arc::clone(&resource);
    let lease_start = Arc::clone(&start);
    let lease_thread = std::thread::spawn(move || {
        lease_start.wait();
        let lease = borrower.async_lease(Cleanup::CloseHandle);
        lease_tx.send(lease.is_ok()).unwrap();
        let _ = release_rx.recv();
        drop(lease);
    });
    start.wait();
    let leased = lease_rx.recv_timeout(Duration::from_secs(5));
    let closed = close_rx.recv_timeout(Duration::from_secs(5));
    let still_owned = if leased.is_ok() && closed.is_ok() {
        Some(!resource.is_closed())
    } else {
        None
    };
    let _ = release_tx.send(());
    close_thread.join().unwrap();
    lease_thread.join().unwrap();
    let (leased, closed) = (leased.unwrap(), closed.unwrap());
    assert_ne!(leased, closed);
    assert_eq!(still_owned, Some(leased));
    assert!(!resource.has_async_leases());
    resource.close().unwrap();
    assert!(resource.is_closed());
}

#[test]
#[cfg(target_pointer_width = "64")]
fn direct_owned_returns_adopt_only_valid_success_and_borrowed_returns_never_cleanup() {
    extern "system" fn create(which: u32) -> *mut c_void {
        match which {
            0 => std::ptr::null_mut(),
            1 => usize::MAX as *mut c_void,
            _ => match unsafe {
                windows::Win32::System::Threading::CreateEventW(None, true, false, None)
            } {
                Ok(handle) => handle.0,
                Err(_) => std::ptr::null_mut(),
            },
        }
    }
    let plan = policy(
        super::tests::fake_plan(
            create as *const () as usize,
            vec![Parameter::input(Type::U32, false)],
            Some(Type::Handle),
            vec![None],
            None,
        ),
        SuccessRule::ReturnValidHandle,
        Cleanup::CloseHandle,
    );
    reset();
    for sentinel in [0, 1] {
        let failed = unsafe { plan.invoke(&[Value::U32(sentinel)]) }.unwrap();
        assert!(!failed.succeeded);
        assert!(matches!(failed.return_value, Some(Value::Handle(0))));
        drop(failed);
        assert!(cleanups().is_empty());
    }
    let result = unsafe { plan.invoke(&[Value::U32(2)]) }.unwrap();
    assert!(result.succeeded);
    let raw = result
        .return_value
        .as_ref()
        .unwrap()
        .resource()
        .unwrap()
        .raw();
    assert_ne!(raw, 0);
    drop(result);
    assert_eq!(
        cleanups(),
        vec![CleanupCall {
            kind: Cleanup::CloseHandle,
            value: raw,
            succeeded: true
        },]
    );

    extern "system" fn borrow(handle: *mut c_void) -> *mut c_void {
        handle
    }
    let owner = allocated_resource(Cleanup::CloseHandle);
    let raw = owner.raw();
    let borrowed = super::tests::fake_plan(
        borrow as *const () as usize,
        vec![Parameter::input(Type::Handle, false)],
        Some(Type::Handle),
        vec![None],
        None,
    );
    reset();
    let result = unsafe { borrowed.invoke(&[Value::Handle(raw)]) }.unwrap();
    assert!(matches!(result.return_value, Some(Value::Handle(value)) if value == raw));
    drop(result);
    assert!(cleanups().is_empty());
    assert_eq!(owner.raw(), raw);
    owner.close().unwrap();
    assert_eq!(
        cleanups(),
        vec![CleanupCall {
            kind: Cleanup::CloseHandle,
            value: raw,
            succeeded: true
        },]
    );
}
