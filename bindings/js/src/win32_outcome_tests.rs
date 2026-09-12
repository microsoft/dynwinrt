// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use std::{ffi::c_void, sync::atomic::AtomicU64};
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
