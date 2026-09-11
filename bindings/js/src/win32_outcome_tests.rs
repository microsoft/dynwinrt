// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use std::{ffi::c_void, sync::atomic::AtomicU64, time::Duration};
use windows::core::HSTRING;
use windows::Win32::Foundation::{
  CloseHandle, SetHandleInformation, HANDLE_FLAGS, HANDLE_FLAG_PROTECT_FROM_CLOSE,
};
use windows::Win32::System::Threading::{CreateEventW, OpenEventW, SYNCHRONIZATION_SYNCHRONIZE};

struct Event {
  handle: HANDLE,
  name: HSTRING,
}

impl Event {
  fn new() -> Self {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let name = HSTRING::from(format!(
      "Local\\dynwinrt-outcome-{}-{}",
      std::process::id(),
      NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    let handle = unsafe { CreateEventW(None, true, false, &name) }.unwrap();
    Self { handle, name }
  }

  fn take(&mut self) -> usize {
    std::mem::replace(&mut self.handle, HANDLE::default()).0 as usize
  }

  fn exists(&self) -> bool {
    match unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, false, &self.name) } {
      Ok(handle) => {
        unsafe { CloseHandle(handle) }.unwrap();
        true
      }
      Err(error) => {
        assert_eq!(error.code().0 as u32, 0x80070002);
        false
      }
    }
  }
}

impl Drop for Event {
  fn drop(&mut self) {
    if !self.handle.is_invalid() {
      let _ = unsafe { CloseHandle(self.handle) };
    }
  }
}

fn handle_storage() -> Arc<NativeAggregateStorage> {
  let width = std::mem::size_of::<usize>();
  Arc::new(
    NativeAggregateStorage::new(
      width * 2,
      None,
      true,
      vec![
        OwnedNativeField {
          offset: 0,
          cleanup: dynwinrt::win32::Cleanup::CloseHandle,
        },
        OwnedNativeField {
          offset: width,
          cleanup: dynwinrt::win32::Cleanup::CloseHandle,
        },
      ],
    )
    .unwrap(),
  )
}

fn write_handles(storage: &NativeAggregateStorage, first: usize, second: usize) {
  #[repr(C)]
  struct Pair {
    first: HANDLE,
    second: HANDLE,
  }
  unsafe extern "system" fn fill(output: *mut Pair, first: HANDLE, second: HANDLE) {
    unsafe { output.write(Pair { first, second }) };
  }
  unsafe {
    fill(
      storage.pointer().cast(),
      HANDLE(first as *mut c_void),
      HANDLE(second as *mut c_void),
    );
  }
}

#[test]
fn aggregate_success_gates_adoption_reuse_and_unclaimed_cleanup() {
  let mut first = Event::new();
  let mut second = Event::new();
  let storage = handle_storage();
  assert!(storage.require_success().is_err());
  storage.prepare_call().unwrap();
  let (first_raw, second_raw) = (first.take(), second.take());
  write_handles(&storage, first_raw, second_raw);
  storage.mark_call_result(true);
  storage.require_success().unwrap();
  assert!(storage.bytes().is_err());
  let taken = storage.take_usize(0).unwrap();
  assert_eq!(taken, first_raw);
  assert_eq!(storage.take_usize(0).unwrap(), 0);
  let adopted =
    unsafe { dynwinrt::win32::OwnedResource::adopt(taken, dynwinrt::win32::Cleanup::CloseHandle) }
      .unwrap();
  storage.prepare_call().unwrap();
  assert!(first.exists());
  assert!(!second.exists());
  assert!(storage.require_success().is_err());
  assert_eq!(storage.take_usize(std::mem::size_of::<usize>()).unwrap(), 0);
  drop(storage);
  assert!(first.exists());
  adopted.close().unwrap();
  adopted.close().unwrap();
  assert!(!first.exists());

  let mut unclaimed = Event::new();
  let raw = unclaimed.take();
  let storage = handle_storage();
  write_handles(&storage, raw, 0);
  storage.mark_call_result(true);
  drop(storage);
  assert!(!unclaimed.exists());
}

#[test]
fn aggregate_failed_outputs_are_not_adopted_or_guessed_as_owned() {
  let event = Event::new();
  let storage = handle_storage();
  write_handles(&storage, event.handle.0 as usize, 0);
  storage.mark_call_result(false);
  assert!(storage.require_success().is_err());
  storage.prepare_call().unwrap();
  drop(storage);
  assert!(event.exists());
}

struct UnprotectField(Arc<NativeAggregateStorage>);

impl Drop for UnprotectField {
  fn drop(&mut self) {
    let state = self.0.state.lock().unwrap();
    let raw = read_usize_from_words(&state.words, self.0.byte_length, 0).unwrap();
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
fn aggregate_cleanup_failure_retains_failed_field_but_cleans_other_fields() {
  let mut protected = Event::new();
  let mut other = Event::new();
  let storage = handle_storage();
  let protection = UnprotectField(Arc::clone(&storage));
  unsafe {
    SetHandleInformation(
      protected.handle,
      HANDLE_FLAG_PROTECT_FROM_CLOSE.0,
      HANDLE_FLAG_PROTECT_FROM_CLOSE,
    )
  }
  .unwrap();
  let (raw, other_raw) = (protected.take(), other.take());
  write_handles(&storage, raw, other_raw);
  storage.mark_call_result(true);
  assert!(storage.prepare_call().is_err());
  assert!(protected.exists());
  assert!(!other.exists());
  {
    let state = storage.state.lock().unwrap();
    assert_eq!(
      read_usize_from_words(&state.words, storage.byte_length, 0).unwrap(),
      raw
    );
    assert_eq!(
      read_usize_from_words(
        &state.words,
        storage.byte_length,
        std::mem::size_of::<usize>(),
      )
      .unwrap(),
      0
    );
  }
  unsafe {
    SetHandleInformation(
      HANDLE(raw as *mut c_void),
      HANDLE_FLAG_PROTECT_FROM_CLOSE.0,
      HANDLE_FLAGS(0),
    )
  }
  .unwrap();
  storage.prepare_call().unwrap();
  assert!(!protected.exists());
  drop(protection);
}

fn owned_text(value: &[u16]) -> Arc<RetainedNativePointer> {
  let mut value = value.to_vec().into_boxed_slice();
  let pointer = value.as_mut_ptr().cast();
  Arc::new(RetainedNativePointer {
    value: DynWinRTValue::with_pointer_owner(
      dynwinrt::WinRTValue::RawPtr(pointer),
      com::NativePointerOwner::WideString(value),
    ),
    string: Some((true, false)),
  })
}

#[test]
fn aggregate_owner_replacement_and_failed_field_writes_are_atomic() {
  let storage = NativeAggregateStorage::new(16, None, true, Vec::new()).unwrap();
  let first = owned_text(&[65, 0]);
  let first_weak = Arc::downgrade(&first);
  let dynwinrt::WinRTValue::RawPtr(first_pointer) = first.value.0 else {
    unreachable!()
  };
  storage
    .write_field(0, &(first_pointer as usize).to_le_bytes(), Some(first))
    .unwrap();
  assert!(first_weak.upgrade().is_some());
  let second = owned_text(&[66, 0]);
  let second_weak = Arc::downgrade(&second);
  let dynwinrt::WinRTValue::RawPtr(second_pointer) = second.value.0 else {
    unreachable!()
  };
  storage
    .write_field(0, &(second_pointer as usize).to_le_bytes(), Some(second))
    .unwrap();
  assert!(first_weak.upgrade().is_none());
  assert!(second_weak.upgrade().is_some());
  assert!(storage.write_field(usize::MAX, &[1], None).is_err());
  assert!(storage.write_field(15, &[1, 2], None).is_err());
  assert!(second_weak.upgrade().is_some());
  assert_eq!(
    usize::from_le_bytes(storage.read_field(0).unwrap()),
    second_pointer as usize
  );
  storage.write_field(0, &0usize.to_le_bytes(), None).unwrap();
  assert!(second_weak.upgrade().is_none());
}

#[test]
fn aggregate_storage_alignment_copy_isolation_and_field_bounds_are_exact() {
  let storage = NativeAggregateStorage::new(24, Some(&[7; 24]), false, Vec::new()).unwrap();
  assert_eq!(storage.pointer() as usize % 8, 0);
  let mut bytes = storage.bytes().unwrap();
  bytes[0] = 99;
  assert_eq!(storage.bytes().unwrap()[0], 7);
  storage
    .write_field(20, &123u32.to_le_bytes(), None)
    .unwrap();
  assert_eq!(u32::from_le_bytes(storage.read_field(20).unwrap()), 123);
  assert!(storage.read_field::<8>(20).is_err());
  assert!(storage.take_usize(usize::MAX).is_err());
  assert!(NativeAggregateStorage::new(24, Some(&[0; 23]), false, Vec::new()).is_err());
}

fn capacity_state(capacity: &Mutex<IocpCapacity>) -> (usize, usize) {
  let state = capacity.lock().unwrap();
  (state.operations, state.buffer_bytes)
}

#[test]
fn byte_capacity_and_operation_capacity_are_independent_and_failure_is_transactional() {
  let base = IOCP_MAX_PENDING_BUFFER_BYTES - IOCP_MAX_OPERATION_BUFFER_BYTES;
  let capacity = Arc::new(Mutex::new(IocpCapacity {
    operations: 0,
    buffer_bytes: base,
  }));
  let reservation = IocpReservation::acquire(&capacity, IOCP_MAX_OPERATION_BUFFER_BYTES).unwrap();
  assert_eq!(
    capacity_state(&capacity),
    (1, IOCP_MAX_PENDING_BUFFER_BYTES)
  );
  assert!(IocpReservation::acquire(&capacity, 1).is_err());
  assert_eq!(
    capacity_state(&capacity),
    (1, IOCP_MAX_PENDING_BUFFER_BYTES)
  );
  let zero = IocpReservation::acquire(&capacity, 0).unwrap();
  assert_eq!(
    capacity_state(&capacity),
    (2, IOCP_MAX_PENDING_BUFFER_BYTES)
  );
  drop((zero, reservation));
  assert_eq!(capacity_state(&capacity), (0, base));
  assert!(IocpReservation::acquire(&capacity, IOCP_MAX_OPERATION_BUFFER_BYTES + 1).is_err());
  assert_eq!(capacity_state(&capacity), (0, base));
  let failure = (|| -> napi::Result<()> {
    let _reservation = IocpReservation::acquire(&capacity, 16)?;
    Err(napi::Error::from_reason("operation preparation failed"))
  })();
  assert!(failure.is_err());
  assert_eq!(capacity_state(&capacity), (0, base));
  let panicked = std::panic::catch_unwind(|| {
    let _reservation = IocpReservation::acquire(&capacity, 16).unwrap();
    panic!("operation preparation unwound");
  });
  assert!(panicked.is_err());
  assert_eq!(capacity_state(&capacity), (0, base));
}

#[test]
fn racing_capacity_reservations_admit_only_one_operation_without_timing_assumptions() {
  use std::sync::{mpsc, Barrier};
  const WORKERS: usize = 4;
  let base = IocpCapacity {
    operations: IOCP_MAX_PENDING_OPERATIONS - 1,
    buffer_bytes: IOCP_MAX_PENDING_BUFFER_BYTES - 8,
  };
  let capacity = Arc::new(Mutex::new(base));
  let gate = Arc::new(Barrier::new(WORKERS + 1));
  let (result_tx, result_rx) = mpsc::channel();
  let mut releases = Vec::new();
  let mut workers = Vec::new();
  for _ in 0..WORKERS {
    let capacity = Arc::clone(&capacity);
    let gate = Arc::clone(&gate);
    let result_tx = result_tx.clone();
    let (release_tx, release_rx) = mpsc::channel();
    releases.push(release_tx);
    workers.push(std::thread::spawn(move || {
      gate.wait();
      let reservation = IocpReservation::acquire(&capacity, 8);
      result_tx.send(reservation.is_ok()).unwrap();
      let _ = release_rx.recv();
      drop(reservation);
    }));
  }
  gate.wait();
  let outcomes = (0..WORKERS)
    .map(|_| result_rx.recv_timeout(Duration::from_secs(5)))
    .collect::<Vec<_>>();
  let held = capacity_state(&capacity);
  for release in releases {
    let _ = release.send(());
  }
  for worker in workers {
    worker.join().unwrap();
  }
  assert_eq!(
    outcomes
      .into_iter()
      .map(Result::unwrap)
      .filter(|admitted| *admitted)
      .count(),
    1
  );
  assert_eq!(
    held,
    (IOCP_MAX_PENDING_OPERATIONS, IOCP_MAX_PENDING_BUFFER_BYTES)
  );
  assert_eq!(
    capacity_state(&capacity),
    (
      IOCP_MAX_PENDING_OPERATIONS - 1,
      IOCP_MAX_PENDING_BUFFER_BYTES - 8,
    )
  );
}

fn unsubmitted_task(
  kind: OverlappedIoKind,
  capacity: &Arc<Mutex<IocpCapacity>>,
) -> OverlappedIoTask {
  let mut event = Event::new();
  let resource = unsafe {
    dynwinrt::win32::OwnedResource::adopt(event.take(), dynwinrt::win32::Cleanup::CloseHandle)
  }
  .unwrap();
  let lease = resource
    .async_lease(dynwinrt::win32::Cleanup::CloseHandle)
    .unwrap();
  OverlappedIoTask {
    kind,
    resource,
    lease,
    buffer: None,
    buffer_len: 3,
    buffer_pointer: 0,
    native_buffer: vec![1, 2, 3],
    offset: 0,
    state: OverlappedState::new(),
    _reservation: Some(IocpReservation::acquire(capacity, 3).unwrap()),
  }
}

#[test]
fn unsubmitted_successful_and_invalid_completion_tasks_return_all_leases_and_capacity() {
  let capacity = Arc::new(Mutex::new(IocpCapacity::default()));
  let task = unsubmitted_task(OverlappedIoKind::Write, &capacity);
  let resource = Arc::clone(&task.resource);
  assert!(resource.has_async_leases());
  assert_eq!(capacity_state(&capacity), (1, 3));
  drop(task);
  assert!(!resource.has_async_leases());
  assert_eq!(capacity_state(&capacity), (0, 0));
  let task = unsubmitted_task(OverlappedIoKind::Write, &capacity);
  assert_eq!(task.resolve(std::ptr::null_mut(), 2).unwrap(), 2);
  assert_eq!(capacity_state(&capacity), (0, 0));
  for kind in [OverlappedIoKind::Read, OverlappedIoKind::Write] {
    let task = unsubmitted_task(kind, &capacity);
    let resource = Arc::clone(&task.resource);
    assert!(task
      .resolve(std::ptr::null_mut(), 4)
      .unwrap_err()
      .reason
      .contains("original Buffer"));
    assert!(!resource.has_async_leases());
    assert_eq!(capacity_state(&capacity), (0, 0));
    resource.close().unwrap();
  }
  resource.close().unwrap();
}

#[test]
fn completion_deactivation_removes_addresses_before_late_cancellation() {
  let state = OverlappedState::new();
  let event = Event::new();
  let overlapped = Box::new(OVERLAPPED::default());
  state.activate(event.handle.0 as usize, &*overlapped);
  {
    let control = state.control.lock().unwrap();
    assert!(control.active);
    assert_eq!(control.overlapped, std::ptr::from_ref(&*overlapped));
  }
  state.deactivate();
  drop(overlapped);
  state.cancel();
  state.cancel();
  let control = state.control.lock().unwrap();
  assert!(!control.active);
  assert_eq!(control.handle, 0);
  assert!(control.overlapped.is_null());
  assert!(state.cancelled.load(Ordering::Acquire));
}

#[test]
fn string_encodings_alignment_and_double_terminators_are_validated_without_native_reads() {
  use super::storage::{validate_string_bytes, BufferInfo};
  let mut bytes = [65u8, 0, 0, 1];
  let ansi = BufferInfo {
    pointer: bytes.as_mut_ptr(),
    length: 2,
  };
  assert!(validate_string_bytes(&ansi, false, false).is_ok());
  assert!(validate_string_bytes(&ansi, false, true).is_err());
  let list = BufferInfo {
    pointer: bytes.as_mut_ptr(),
    length: 3,
  };
  assert!(validate_string_bytes(&list, false, true).is_ok());
  let mut units = [65u16, 0, 0, 0];
  let wide = BufferInfo {
    pointer: units.as_mut_ptr().cast(),
    length: 4,
  };
  assert!(validate_string_bytes(&wide, true, false).is_ok());
  assert!(validate_string_bytes(&wide, true, true).is_err());
  let list = BufferInfo {
    pointer: units.as_mut_ptr().cast(),
    length: 6,
  };
  assert!(validate_string_bytes(&list, true, true).is_ok());
  let misaligned = BufferInfo {
    pointer: unsafe { units.as_mut_ptr().cast::<u8>().add(1) },
    length: 4,
  };
  assert!(validate_string_bytes(&misaligned, true, false).is_err());
  let odd = BufferInfo {
    pointer: units.as_mut_ptr().cast(),
    length: 5,
  };
  assert!(validate_string_bytes(&odd, true, false).is_err());
  let empty = BufferInfo {
    pointer: std::ptr::null_mut(),
    length: 0,
  };
  assert!(validate_string_bytes(&empty, true, false).is_err());
  assert!(validate_string_bytes(&empty, false, true).is_err());
}
