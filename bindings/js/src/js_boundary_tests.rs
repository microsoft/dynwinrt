// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::value::ensure_progress_type_supported;

use super::*;

#[test]
fn hresult_arrays_use_the_i32_projection() {
  let array = DynWinRTArray::new(dynwinrt::ArrayData::from_values(
    TABLE.hresult(),
    &[dynwinrt::WinRTValue::HResult(windows::core::HRESULT(
      0x80004005u32 as i32,
    ))],
  ));

  assert_eq!(
    array.to_i32_vec().expect("HRESULT array conversion"),
    [0x80004005u32 as i32],
  );
}

#[test]
fn progress_type_validation_allows_structs_and_rejects_unsupported_shapes() {
  let progress = TABLE.struct_type("Test.JsStructProgress", &[TABLE.u64_type()]);
  assert!(ensure_progress_type_supported(&progress).is_ok());

  let value_type = TABLE.u32_type();
  for unsupported in [
    TABLE.guid_type(),
    TABLE.array_of_iunknown(),
    TABLE.generic(
      windows::core::GUID::from_u128(0x11111111_2222_3333_4444_555555555555),
      1,
    ),
    TABLE.out_value(&value_type),
    TABLE.array(&value_type),
  ] {
    assert!(ensure_progress_type_supported(&unsupported).is_err());
  }
}

#[test]
fn failed_explicit_bitmap_release_preserves_owner_for_retry() {
  dynwinrt::system_helpers::GdiObjectDeleter::resolve().unwrap();
  let bitmap = unsafe { windows::Win32::Graphics::Gdi::CreateBitmap(1, 1, 1, 1, None) };
  assert!(!bitmap.is_invalid());
  let pointer = bitmap.0;

  let mut value = DynWinRTValue::from_com_result(
    dynwinrt::WinRTValue::RawPtr(pointer),
    dynwinrt::com::PointerOutputKind::OwnedHandle(dynwinrt::com::OwnedHandleCleanup::DeleteObject),
  );
  com::DynComOwnedHandle::force_next_delete_object_failure_for_test();
  let error = value.release().unwrap_err();
  assert!(error.reason.contains("DeleteObject failed"));
  assert!(matches!(
    value.winrt(),
    dynwinrt::WinRTValue::RawPtr(raw) if *raw == pointer
  ));
  assert_eq!(
    value.pointer_provenance(),
    crate::com_value::PointerProvenance::OwnedHandleOutput(
      dynwinrt::com::OwnedHandleCleanup::DeleteObject
    )
  );

  value.release().unwrap();
  assert!(matches!(value.winrt(), dynwinrt::WinRTValue::Null));
  assert_eq!(
    value.pointer_provenance(),
    crate::com_value::PointerProvenance::None
  );
  assert!(!unsafe {
    windows::Win32::Graphics::Gdi::DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(pointer))
  }
  .as_bool());
}

#[test]
fn actual_delete_object_failure_preserves_generic_owner() {
  dynwinrt::system_helpers::GdiObjectDeleter::resolve().unwrap();
  let pointer = std::ptr::with_exposed_provenance_mut(1);
  let mut value = DynWinRTValue::from_com_result(
    dynwinrt::WinRTValue::RawPtr(pointer),
    dynwinrt::com::PointerOutputKind::OwnedHandle(dynwinrt::com::OwnedHandleCleanup::DeleteObject),
  );

  let error = value.release().unwrap_err();
  assert!(error.reason.contains("DeleteObject failed"));
  assert!(matches!(
    value.winrt(),
    dynwinrt::WinRTValue::RawPtr(raw) if *raw == pointer
  ));
  assert_eq!(
    value.pointer_provenance(),
    crate::com_value::PointerProvenance::OwnedHandleOutput(
      dynwinrt::com::OwnedHandleCleanup::DeleteObject
    )
  );

  *value.winrt_mut() = dynwinrt::WinRTValue::Null;
  value.clear_pointer_provenance();
}
