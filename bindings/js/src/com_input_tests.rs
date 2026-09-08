// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use dynwinrt::com::borrowed::{
  testing::CopyFixture, BorrowedCopyContract, BorrowedCopyPlan, CopyKind, MF_BUFFER,
};
use std::{
  cell::{Cell, RefCell},
  ffi::c_void,
  rc::Rc,
  sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
  },
};
use windows::core::{IUnknown_Vtbl, HRESULT};

fn media(fixture: &CopyFixture) -> DynWinRTValue {
  let mut value = DynWinRTValue::new(dynwinrt::WinRTValue::Object(
    fixture.object("media").unwrap(),
  ));
  value.bind_current_com_apartment().unwrap();
  value
}

fn plan() -> BorrowedCopyPlan {
  BorrowedCopyPlan::prepare(
    &TABLE,
    BorrowedCopyContract::expected(CopyKind::MediaBuffer),
  )
  .unwrap()
}

fn poison(fixture: &CopyFixture, value: &DynWinRTValue) {
  fixture.configure("mediaUnlockHr", 0x80004005).unwrap();
  assert!(plan()
    .read_copy(&value.com_context().unwrap(), &value.0.as_object().unwrap())
    .is_err());
}

fn assert_rejected(value: &DynWinRTValue, reason: &str) {
  let entered = Cell::new(false);
  let error = with_com_invocation_args(&[value], |_| {
    entered.set(true);
    Ok(())
  })
  .unwrap_err();
  assert!(error.reason.contains(reason), "{}", error.reason);
  assert!(!entered.get());
}

fn containers(value: &DynWinRTValue) -> Vec<DynWinRTValue> {
  let variant = DynComVariant::unknown(Some(value)).unwrap();
  let array = DynComSafeArray::unknown(vec![value], None).unwrap();
  let nested = DynComVariant::safe_array(&array).unwrap();
  let variants = DynComSafeArray::variant(vec![&nested], None).unwrap();
  let dispatch = DynComDispatchParams::new(vec![&nested], None).unwrap();
  let winrt = super::super::DynWinRTArray::from_object_values(
    vec![value],
    &DynWinRTType(TABLE.interface(MF_BUFFER)),
  )
  .unwrap();
  vec![
    DynCom::interface_array(&WinGUID(MF_BUFFER), vec![value]).unwrap(),
    DynCom::variant_array(vec![&variant]).unwrap(),
    DynCom::variant(&variant).unwrap(),
    DynCom::safe_array(&array).unwrap(),
    DynCom::variant(&nested).unwrap(),
    DynCom::safe_array(&variants).unwrap(),
    DynCom::dispatch_params(&dispatch).unwrap(),
    winrt.to_value().unwrap(),
  ]
}

#[test]
fn com_arguments_reject_poisoned_objects_and_preexisting_aliases() {
  let fixture = CopyFixture::default();
  let value = media(&fixture);
  let alias = media(&fixture);
  with_com_invocation_args(&[&alias], |_| Ok(())).unwrap();
  poison(&fixture, &value);
  assert_rejected(&value, "poisoned");
  assert_rejected(&alias, "poisoned");
  assert!(alias
    .to_com_value()
    .unwrap_err()
    .reason
    .contains("poisoned"));
  // Identity and deterministic release are not ordinary input admission.
  let mut queried = alias.cast(&WinGUID(MF_BUFFER)).unwrap();
  queried.release().unwrap();
}

#[test]
fn com_argument_containers_pin_later_poison_after_originals_are_released() {
  let mut fixture = CopyFixture::default();
  let mut value = media(&fixture);
  let mut alias = media(&fixture);
  let admission = admit_com_inputs(&[&alias]).unwrap();
  let containers = containers(&alias);
  let mut variant_only = DynComVariant::unknown(Some(&alias)).unwrap();
  let mut array_only = DynComSafeArray::unknown(vec![&alias], None).unwrap();
  let mut dispatch_only = DynComDispatchParams::new(vec![&variant_only], None).unwrap();
  for container in &containers {
    with_com_invocation_args(&[container], |_| Ok(())).unwrap();
  }
  poison(&fixture, &value);
  value.release().unwrap();
  alias.release().unwrap();
  fixture.release_owners();
  assert!(admission.check().unwrap_err().reason.contains("poisoned"));
  drop(admission);
  assert!(!fixture.events().contains(&"drop.media".to_string()));
  for container in &containers {
    assert_rejected(container, "poisoned");
  }
  assert!(DynCom::variant(&variant_only)
    .err()
    .unwrap()
    .reason
    .contains("poisoned"));
  assert!(DynCom::safe_array(&array_only)
    .err()
    .unwrap()
    .reason
    .contains("poisoned"));
  assert!(DynCom::dispatch_params(&dispatch_only)
    .err()
    .unwrap()
    .reason
    .contains("poisoned"));
  variant_only.release().unwrap();
  array_only.release().unwrap();
  dispatch_only.release().unwrap();
  drop(containers);
  assert_eq!(
    fixture
      .events()
      .iter()
      .filter(|e| *e == "drop.media")
      .count(),
    1
  );
}

#[test]
fn com_argument_busy_guard_runs_without_a_javascript_callback() {
  let fixture = CopyFixture::default();
  let value = media(&fixture);
  let alias = media(&fixture);
  let containers = containers(&alias);
  let observed = Rc::new(Cell::new(false));
  let observed_hook = observed.clone();
  fixture.hook(Some(Rc::new(move |event| {
    if event == "media.acquire" {
      assert_rejected(&alias, "busy");
      for container in &containers {
        assert_rejected(container, "busy");
      }
      observed_hook.set(true);
    }
  })));
  plan()
    .read_copy(&value.com_context().unwrap(), &value.0.as_object().unwrap())
    .unwrap();
  fixture.hook(None);
  assert!(observed.get());
  with_com_invocation_args(&[&value], |_| Ok(())).unwrap();
}

const TEST_IID: GUID = GUID::from_u128(0x109302ce_8b8e_4e90_ad9c_5b988abc94b1);
thread_local! {
  static ON_QUERY: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

#[derive(Default)]
struct Counts {
  queries: AtomicU32,
  addrefs: AtomicU32,
  releases: AtomicU32,
  calls: AtomicU32,
}

#[repr(C)]
struct TestObject {
  vtable: &'static TestVtable,
  references: AtomicU32,
  counts: Arc<Counts>,
}

#[repr(C)]
struct TestVtable {
  base: IUnknown_Vtbl,
  call: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
}

unsafe extern "system" fn test_query(
  this: *mut c_void,
  iid: *const GUID,
  out: *mut *mut c_void,
) -> HRESULT {
  let object = unsafe { &*this.cast::<TestObject>() };
  object.counts.queries.fetch_add(1, Ordering::SeqCst);
  if unsafe { *iid } != IUnknown::IID && unsafe { *iid } != TEST_IID {
    unsafe { *out = std::ptr::null_mut() };
    return HRESULT(0x80004002u32 as i32);
  }
  if unsafe { *iid } == TEST_IID {
    let hook = ON_QUERY.with(|hook| hook.borrow_mut().take());
    if let Some(hook) = hook {
      hook();
    }
  }
  unsafe {
    *out = this;
    test_addref(this);
  }
  HRESULT(0)
}

unsafe extern "system" fn test_addref(this: *mut c_void) -> u32 {
  let object = unsafe { &*this.cast::<TestObject>() };
  object.counts.addrefs.fetch_add(1, Ordering::SeqCst);
  object.references.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn test_release(this: *mut c_void) -> u32 {
  let object = unsafe { &*this.cast::<TestObject>() };
  object.counts.releases.fetch_add(1, Ordering::SeqCst);
  let remaining = object.references.fetch_sub(1, Ordering::SeqCst) - 1;
  if remaining == 0 {
    unsafe {
      drop(Box::from_raw(this.cast::<TestObject>()));
    }
  }
  remaining
}

unsafe extern "system" fn test_call(this: *mut c_void, _: *mut c_void, _: *mut c_void) -> HRESULT {
  unsafe { &*this.cast::<TestObject>() }
    .counts
    .calls
    .fetch_add(1, Ordering::SeqCst);
  HRESULT(0)
}

static TEST_VTABLE: TestVtable = TestVtable {
  base: IUnknown_Vtbl {
    QueryInterface: test_query,
    AddRef: test_addref,
    Release: test_release,
  },
  call: test_call,
};

fn test_object() -> (DynWinRTValue, Arc<Counts>) {
  let (mut value, counts) = unbound_test_object();
  value.bind_current_com_apartment().unwrap();
  (value, counts)
}

fn unbound_test_object() -> (DynWinRTValue, Arc<Counts>) {
  let counts = Arc::new(Counts::default());
  let object = Box::new(TestObject {
    vtable: &TEST_VTABLE,
    references: AtomicU32::new(1),
    counts: counts.clone(),
  });
  let unknown = unsafe { IUnknown::from_raw(Box::into_raw(object).cast()) };
  let value = DynWinRTValue::new(dynwinrt::WinRTValue::Object(unknown));
  (value, counts)
}

fn unbound_async_value() -> DynWinRTValue {
  let operation = windows_future::IAsyncOperation::<i32>::ready(Ok(42));
  DynWinRTValue::new(dynwinrt::WinRTValue::Async(dynwinrt::AsyncInfo {
    info: operation.cast().unwrap(),
    async_type: TABLE.async_operation(&TABLE.i32_type()),
  }))
}

#[test]
fn unbound_winrt_bookkeeping_allows_cross_thread_use_and_frees_on_drop() {
  use super::super::{com_input::InputBindings, DynWinRTArray, DynWinRTStruct};
  fn send_sync<T: Send + Sync>() {}
  send_sync::<InputBindings>();

  let (value, counts) = unbound_test_object();
  let async_value = unbound_async_value();
  let object_alive = value.7.as_ref().unwrap().state_alive_probe();
  let async_alive = async_value.7.as_ref().unwrap().state_alive_probe();
  std::thread::spawn(move || {
    let alias = value.cast(&WinGUID(TEST_IID)).unwrap();
    let array =
      DynWinRTArray::from_object_values(vec![&alias], &DynWinRTType(TABLE.interface(TEST_IID)))
        .unwrap();
    let mut structure = DynWinRTStruct::create(&DynWinRTType(
      TABLE.struct_type("UnboundTransferField", &[TABLE.interface(TEST_IID)]),
    ))
    .unwrap();
    structure.set_object(0.0, &alias).unwrap();
    let array_value = array.to_value().unwrap();
    let struct_value = structure.to_value().unwrap();
    let async_alias = async_value
      .cast(&WinGUID(windows_future::IAsyncOperation::<i32>::IID))
      .unwrap();
    let operation: windows_future::IAsyncOperation<i32> =
      async_alias.0.as_object().unwrap().cast().unwrap();
    assert_eq!(operation.GetResults().unwrap(), 42);
    for value in [
      &value,
      &alias,
      &array_value,
      &struct_value,
      &async_value,
      &async_alias,
    ] {
      assert!(!value.7.as_ref().unwrap().is_apartment_bound());
    }
  })
  .join()
  .unwrap();
  assert!(
    !object_alive(),
    "unbound array/struct aliases must not leak their shared bookkeeping"
  );
  assert!(
    !async_alive(),
    "unbound async bookkeeping must be freed on its consumer thread"
  );
  assert_eq!(
    counts.releases.load(Ordering::SeqCst),
    counts.addrefs.load(Ordering::SeqCst) + 1,
  );
}

#[test]
fn deferred_com_ownership_is_claimed_once_without_leaking_empty_state() {
  use super::super::com_input::InputBindings;
  let state = InputBindings::deferred();
  let alive = state.state_alive_probe();
  let first = state.pin().unwrap();
  let second = state.pin().unwrap();
  let barrier = Arc::new(std::sync::Barrier::new(3));
  let first_barrier = barrier.clone();
  let first = std::thread::spawn(move || {
    first_barrier.wait();
    first.bind_current_apartment().is_ok()
  });
  let second_barrier = barrier.clone();
  let second = std::thread::spawn(move || {
    second_barrier.wait();
    second.bind_current_apartment().is_ok()
  });
  barrier.wait();
  assert_ne!(first.join().unwrap(), second.join().unwrap());
  drop(state);
  assert!(
    !alive(),
    "a claimed state with no apartment bindings must still be freed"
  );
}

#[test]
fn unbound_winrt_values_first_enter_com_on_the_consumer_thread() {
  use super::super::{DynWinRTArray, DynWinRTStruct};
  for explicit_binding in [false, true] {
    let (mut value, counts) = unbound_test_object();
    let async_value = unbound_async_value();
    let object_alive = value.7.as_ref().unwrap().state_alive_probe();
    let async_alive = async_value.7.as_ref().unwrap().state_alive_probe();
    let array =
      DynWinRTArray::from_object_values(vec![&value], &DynWinRTType(TABLE.interface(TEST_IID)))
        .unwrap();
    let array_value = array.to_value().unwrap();
    let mut structure = DynWinRTStruct::create(&DynWinRTType(
      TABLE.struct_type("ConsumerAdmissionField", &[TABLE.interface(TEST_IID)]),
    ))
    .unwrap();
    structure.set_object(0.0, &value).unwrap();
    let snapshot = structure.to_value().unwrap();
    std::thread::spawn(move || {
      if explicit_binding {
        value.bind_current_com_apartment().unwrap();
      }
      // In the implicit case the pre-existing container reaches COM first.
      with_com_invocation_args(&[&array_value, &snapshot, &value, &async_value], |_| Ok(()))
        .unwrap();
      value.bind_current_com_apartment().unwrap();
      let mut extracted = array.get(0.0).unwrap();
      extracted.bind_current_com_apartment().unwrap();
      assert!(Rc::ptr_eq(
        &value.6.as_ref().unwrap().context,
        &extracted.6.as_ref().unwrap().context,
      ));
      for input in [&array_value, &snapshot, &value, &async_value] {
        let inputs = input.7.as_ref().unwrap();
        assert!(inputs.is_apartment_bound());
        assert!(inputs.is_owner());
      }
      drop(structure);
    })
    .join()
    .unwrap();
    assert!(!object_alive());
    assert!(!async_alive());
    assert_eq!(
      counts.releases.load(Ordering::SeqCst),
      counts.addrefs.load(Ordering::SeqCst) + 1,
    );
  }
}

#[test]
fn com_argument_apartment_rejection_precedes_all_interface_refcount_and_qi_actions() {
  let (mut value, counts) = unbound_test_object();
  let winrt_array = super::super::DynWinRTArray::from_object_values(
    vec![&value],
    &DynWinRTType(TABLE.interface(TEST_IID)),
  )
  .unwrap();
  let extracted = winrt_array.get(0.0).unwrap();
  let mut structure = super::super::DynWinRTStruct::create(&DynWinRTType(
    TABLE.struct_type("ApartmentGuardField", &[TABLE.interface(TEST_IID)]),
  ))
  .unwrap();
  structure.set_object(0.0, &value).unwrap();
  let struct_value = structure.to_value().unwrap();
  value.bind_current_com_apartment().unwrap();
  let array = DynCom::interface_array(&WinGUID(TEST_IID), vec![&value]).unwrap();
  let variant = DynComVariant::unknown(Some(&value)).unwrap();
  let carrier = DynCom::variant(&variant).unwrap();
  let before = (
    counts.queries.load(Ordering::SeqCst),
    counts.addrefs.load(Ordering::SeqCst),
    counts.releases.load(Ordering::SeqCst),
  );
  let returned = std::thread::spawn(move || {
    for input in [&value, &array, &carrier, &extracted, &struct_value] {
      assert_rejected(input, "different apartment thread");
      assert!(input.to_com_value().is_err());
    }
    assert!(winrt_array.get(0.0).is_err());
    assert!(winrt_array.to_values().is_err());
    assert!(winrt_array.to_value().is_err());
    assert!(structure.get_object(0.0).is_err());
    assert!(structure.to_value().is_err());
    assert!(struct_value.as_struct().is_err());
    assert!(super::super::DynWinRTArray::from_object_values(
      vec![&value],
      &DynWinRTType(TABLE.interface(TEST_IID)),
    )
    .is_err());
    (
      value,
      array,
      carrier,
      winrt_array,
      extracted,
      structure,
      struct_value,
    )
  })
  .join()
  .unwrap();
  assert_eq!(
    before,
    (
      counts.queries.load(Ordering::SeqCst),
      counts.addrefs.load(Ordering::SeqCst),
      counts.releases.load(Ordering::SeqCst)
    )
  );
  drop(returned);
}

fn argument_method() -> DynComMethodHandle {
  DynComMethodHandle(
    dynwinrt::com::register_interface(
      &TABLE,
      "argument-guard",
      TEST_IID,
      dynwinrt::com::InterfaceBase::IUnknown,
    )
    .add_method_at(
      3,
      "Use",
      dynwinrt::com::MethodSignature::new(&TABLE)
        .add_in(dynwinrt::com::Type::winrt(TABLE.interface(TEST_IID)))
        .add_in(dynwinrt::com::Type::winrt(TABLE.interface(MF_BUFFER))),
    )
    .unwrap()
    .method(3)
    .unwrap(),
  )
}

#[test]
fn com_argument_rechecks_after_native_qi_before_the_dispatch_marker() {
  let fixture = Rc::new(CopyFixture::default());
  let value = Rc::new(media(&fixture));
  let (receiver, counts) = test_object();
  let (trigger, _) = test_object();
  let method = argument_method();
  let marker = DynComRawDispatchState {
    entered: Cell::new(false),
  };
  DynComRaw::invoke_all_tracked(&method, &receiver, vec![&trigger, &value], &marker).unwrap();
  assert!(marker.entered.get());
  assert_eq!(counts.calls.load(Ordering::SeqCst), 1);
  let poisoned = value.clone();
  ON_QUERY.with(|hook| *hook.borrow_mut() = Some(Box::new(move || poison(&fixture, &poisoned))));
  let error = DynComRaw::invoke_all_tracked(&method, &receiver, vec![&trigger, &value], &marker)
    .err()
    .unwrap();
  assert!(error.reason.contains("poisoned"), "{}", error.reason);
  assert!(!marker.entered.get());
  assert_eq!(counts.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn com_receiver_rechecks_after_argument_qi_with_idle_arguments() {
  for tracked in [false, true] {
    let fixture = Rc::new(CopyFixture::default());
    let poison_target = Rc::new(media(&fixture));
    let (mut receiver, counts) = test_object();
    // Give the counted native receiver the fixture's lifecycle context without
    // pretending that its vtable implements IMFMediaBuffer.
    receiver
      .retain_com_context(poison_target.com_context().unwrap())
      .unwrap();
    let healthy_fixture = CopyFixture::default();
    let healthy = media(&healthy_fixture);
    let (trigger, _) = test_object();
    let method = argument_method();
    let marker = DynComRawDispatchState {
      entered: Cell::new(false),
    };
    let invoke = || {
      if tracked {
        DynComRaw::invoke_all_tracked(&method, &receiver, vec![&trigger, &healthy], &marker)
          .map(|_| ())
      } else {
        method
          .invoke(&receiver, vec![&trigger, &healthy])
          .map(|_| ())
      }
    };
    invoke().unwrap();
    assert_eq!(counts.calls.load(Ordering::SeqCst), 1);
    ON_QUERY
      .with(|hook| *hook.borrow_mut() = Some(Box::new(move || poison(&fixture, &poison_target))));
    let error = invoke().unwrap_err();
    assert!(error.reason.contains("poisoned"), "{}", error.reason);
    healthy.ensure_tracked_com_idle().unwrap();
    trigger.ensure_tracked_com_idle().unwrap();
    assert_eq!(counts.calls.load(Ordering::SeqCst), 1);
    if tracked {
      assert!(!marker.entered.get());
    }
  }
}

#[test]
fn com_argument_guard_preserves_plain_values_and_winrt_array_construction() {
  let scalar = DynWinRTValue::new(dynwinrt::WinRTValue::I32(42));
  let buffer = DynWinRTValue::from_com_value(
    dynwinrt::com::Value::Buffer(dynwinrt::com::ComBufferValue::from_owned_bytes(
      vec![1, 2],
      2,
    )),
    dynwinrt::com::PointerOutputKind::None,
  );
  let prop = DynComPropVariant::i32(7.0).unwrap();
  let prop = DynCom::prop_variant(&prop).unwrap();
  with_com_invocation_args(&[&scalar, &buffer, &prop], |values| {
    assert_eq!(values.len(), 3);
    Ok(())
  })
  .unwrap();
  let fixture = CopyFixture::default();
  let value = media(&fixture);
  poison(&fixture, &value);
  // Constructing/projecting a WinRT array is unchanged; COM admission is later.
  let array = super::super::DynWinRTArray::from_object_values(
    vec![&value],
    &DynWinRTType(TABLE.interface(MF_BUFFER)),
  )
  .unwrap();
  assert_eq!(array.0.len(), 1);
  assert_rejected(&array.to_value().unwrap(), "poisoned");
}

#[test]
fn com_argument_winrt_array_extraction_retains_only_the_selected_identity() {
  let mut fixture = CopyFixture::default();
  let mut value = media(&fixture);
  let other_fixture = CopyFixture::default();
  let other = media(&other_fixture);
  let array = super::super::DynWinRTArray::from_object_values(
    vec![&value, &other],
    &DynWinRTType(TABLE.interface(MF_BUFFER)),
  )
  .unwrap();
  let extracted = array.get(0.0).unwrap();
  let idle = array.get(1.0).unwrap();
  let all = array.to_values().unwrap();
  drop(array);
  poison(&fixture, &value);
  value.release().unwrap();
  fixture.release_owners();
  assert_rejected(&extracted, "poisoned");
  assert_rejected(&all[0], "poisoned");
  with_com_invocation_args(&[&idle, &all[1]], |_| Ok(())).unwrap();
}

#[test]
fn com_argument_typed_struct_fields_keep_independent_snapshot_bindings() {
  use super::super::{DynWinRTArray, DynWinRTStruct};
  let mut fixture = CopyFixture::default();
  let mut value = media(&fixture);
  let typ = DynWinRTType(TABLE.struct_type("InputInterfaceField", &[TABLE.interface(MF_BUFFER)]));
  let outer_type = DynWinRTType(TABLE.struct_type("InputNestedField", &[typ.0.clone()]));
  let mut inner = DynWinRTStruct::create(&typ).unwrap();
  inner.set_object(0.0, &value).unwrap();
  let mut outer = DynWinRTStruct::create(&outer_type).unwrap();
  outer.set_struct(0.0, &inner).unwrap();
  let snapshot = outer.to_value().unwrap();
  let field = snapshot
    .as_struct()
    .unwrap()
    .get_struct(0.0)
    .unwrap()
    .get_object(0.0)
    .unwrap();
  let array = DynWinRTArray::from_object_values(vec![&snapshot], &outer_type)
    .unwrap()
    .to_value()
    .unwrap();
  with_com_invocation_args(&[&snapshot, &array], |_| Ok(())).unwrap();
  inner
    .set_object(0.0, &DynWinRTValue::new(dynwinrt::WinRTValue::Null))
    .unwrap();
  outer.set_struct(0.0, &inner).unwrap();
  poison(&fixture, &value);
  value.release().unwrap();
  fixture.release_owners();
  assert_rejected(&snapshot, "poisoned");
  assert_rejected(&field, "poisoned");
  assert_rejected(&array, "poisoned");
  with_com_invocation_args(&[&outer.to_value().unwrap()], |_| Ok(())).unwrap();
}
