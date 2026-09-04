// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
  ffi::c_void,
  sync::atomic::{AtomicI32, AtomicU32, AtomicUsize, Ordering},
};

use napi::bindgen_prelude::BigInt;
use napi_derive::napi;
use windows::{
  core::{IUnknown, IUnknown_Vtbl, Interface, GUID, HRESULT, PCWSTR},
  Win32::System::{
    Com::SAFEARRAY,
    Variant::VARIANT,
    Wmi::{
      IWbemCallResult_Vtbl, IWbemClassObject_Vtbl, IWbemServices_Vtbl, WBEM_COMPARISON_FLAG,
      WBEM_CONDITION_FLAG_TYPE, WBEM_GENERIC_FLAG_TYPE,
    },
  },
};

use crate::DynWinRTValue;

const IID_IWBEM_SERVICES: GUID = GUID::from_u128(0x9556dc99_828c_11cf_a37e_00aa003240c7);
const IID_IWBEM_CALL_RESULT: GUID = GUID::from_u128(0x44aca675_e8fc_11d0_a07c_00c04fb68820);
const IID_IWBEM_CLASS_OBJECT: GUID = GUID::from_u128(0xdc12a681_737f_11cf_884d_00aa004b2e24);
const IID_ITHUMBNAIL_PROVIDER: GUID = GUID::from_u128(0xe357fccd_a995_4576_b01f_234630154e96);
const IID_IDATA_OBJECT: GUID = GUID::from_u128(0x0000010e_0000_0000_c000_000000000046);
const IID_IAUDIO_CLIENT: GUID = GUID::from_u128(0x1cb9ad4c_dbfa_4c32_b178_c2f568a703b2);
const E_NOINTERFACE: HRESULT = HRESULT(0x80004002u32 as i32);
const E_POINTER: HRESULT = HRESULT(0x80004003u32 as i32);
const E_NOTIMPL: HRESULT = HRESULT(0x80004001u32 as i32);

static QUERY_INTERFACE_CALLS: AtomicU32 = AtomicU32::new(0);
static ADD_REF_CALLS: AtomicU32 = AtomicU32::new(0);
static RELEASE_CALLS: AtomicU32 = AtomicU32::new(0);
static OPEN_NAMESPACE_CALLS: AtomicU32 = AtomicU32::new(0);
static OPEN_NAMESPACE_WORKING_SLOT_NULL: AtomicU32 = AtomicU32::new(0);
static OPEN_NAMESPACE_WORKING_ARGUMENT_NULL: AtomicU32 = AtomicU32::new(0);
static OPEN_NAMESPACE_RESULT_SLOT_NULL: AtomicU32 = AtomicU32::new(0);
static OPEN_NAMESPACE_RESULT_ARGUMENT_NULL: AtomicU32 = AtomicU32::new(0);
static OPEN_NAMESPACE_CONTEXT_ARGUMENT_NULL: AtomicU32 = AtomicU32::new(0);
static OPEN_NAMESPACE_LAST_FLAGS: AtomicI32 = AtomicI32::new(0);
static WMI_CONDITIONAL_MODE: AtomicI32 = AtomicI32::new(0);
static GET_OBJECT_CALLS: AtomicU32 = AtomicU32::new(0);
static PUT_CLASS_CALLS: AtomicU32 = AtomicU32::new(0);
static DELETE_CLASS_CALLS: AtomicU32 = AtomicU32::new(0);
static PUT_INSTANCE_CALLS: AtomicU32 = AtomicU32::new(0);
static DELETE_INSTANCE_CALLS: AtomicU32 = AtomicU32::new(0);
static EXEC_METHOD_CALLS: AtomicU32 = AtomicU32::new(0);
static QUERY_OBJECT_SINK_CALLS: AtomicU32 = AtomicU32::new(0);
static LAST_FLAGS: AtomicI32 = AtomicI32::new(0);
static CURRENT_REF_COUNT: AtomicU32 = AtomicU32::new(0);
static CLASS_OBJECT_END_ENUMERATION_CALLS: AtomicU32 = AtomicU32::new(0);
static CALL_RESULT_GET_STATUS_CALLS: AtomicU32 = AtomicU32::new(0);
static LAST_OUTPUT_ADDRESS: AtomicUsize = AtomicUsize::new(0);
static AUDIO_IS_FORMAT_SUPPORTED_CALLS: AtomicU32 = AtomicU32::new(0);
static AUDIO_GET_SERVICE_CALLS: AtomicU32 = AtomicU32::new(0);
static AUDIO_LAST_SHARE_MODE: AtomicI32 = AtomicI32::new(0);
static AUDIO_LAST_FORMAT_TAG: AtomicU32 = AtomicU32::new(0);
static AUDIO_GET_SERVICE_MODE: AtomicI32 = AtomicI32::new(0);
static AUDIO_ADD_REF_CALLS: AtomicU32 = AtomicU32::new(0);
static AUDIO_RELEASE_CALLS: AtomicU32 = AtomicU32::new(0);
static AUDIO_CURRENT_REF_COUNT: AtomicU32 = AtomicU32::new(0);
static THUMBNAIL_CALLS: AtomicU32 = AtomicU32::new(0);
static THUMBNAIL_CURRENT_REF_COUNT: AtomicU32 = AtomicU32::new(0);
static DATA_OBJECT_GET_DATA_CALLS: AtomicU32 = AtomicU32::new(0);
static DATA_OBJECT_GET_DATA_HERE_CALLS: AtomicU32 = AtomicU32::new(0);
static DATA_OBJECT_QUERY_GET_DATA_CALLS: AtomicU32 = AtomicU32::new(0);
static DATA_OBJECT_CANONICAL_CALLS: AtomicU32 = AtomicU32::new(0);
static DATA_OBJECT_SET_DATA_CALLS: AtomicU32 = AtomicU32::new(0);
static DATA_OBJECT_LAST_SET_RELEASE: AtomicI32 = AtomicI32::new(-1);
static DATA_OBJECT_LAST_OUTPUT_HANDLE: AtomicUsize = AtomicUsize::new(0);
static DATA_OBJECT_LAST_GET_DATA_HERE_HANDLE: AtomicUsize = AtomicUsize::new(0);
static DATA_OBJECT_LAST_SET_DATA_HANDLE: AtomicUsize = AtomicUsize::new(0);
static DATA_OBJECT_CURRENT_REF_COUNT: AtomicU32 = AtomicU32::new(0);

#[repr(C)]
struct GeneratedIWbemServicesFake {
  services_vtable: *const IWbemServices_Vtbl,
  class_object_vtable: *const IWbemClassObject_Vtbl,
  call_result_vtable: *const IWbemCallResult_Vtbl,
  references: AtomicU32,
}

unsafe fn owner_from_class_object(this: *mut c_void) -> *mut GeneratedIWbemServicesFake {
  unsafe {
    this
      .cast::<u8>()
      .sub(std::mem::offset_of!(
        GeneratedIWbemServicesFake,
        class_object_vtable
      ))
      .cast()
  }
}

unsafe fn owner_from_call_result(this: *mut c_void) -> *mut GeneratedIWbemServicesFake {
  unsafe {
    this
      .cast::<u8>()
      .sub(std::mem::offset_of!(
        GeneratedIWbemServicesFake,
        call_result_vtable
      ))
      .cast()
  }
}

unsafe fn services_pointer(owner: *mut GeneratedIWbemServicesFake) -> *mut c_void {
  owner.cast()
}

unsafe fn class_object_pointer(owner: *mut GeneratedIWbemServicesFake) -> *mut c_void {
  unsafe { std::ptr::addr_of_mut!((*owner).class_object_vtable).cast() }
}

unsafe fn call_result_pointer(owner: *mut GeneratedIWbemServicesFake) -> *mut c_void {
  unsafe { std::ptr::addr_of_mut!((*owner).call_result_vtable).cast() }
}

unsafe fn query_interface_owner(
  owner: *mut GeneratedIWbemServicesFake,
  iid: *const GUID,
  result: *mut *mut c_void,
) -> HRESULT {
  QUERY_INTERFACE_CALLS.fetch_add(1, Ordering::SeqCst);
  if iid.is_null() || result.is_null() {
    return E_POINTER;
  }
  unsafe {
    *result = std::ptr::null_mut();
    *result = if *iid == IUnknown::IID || *iid == IID_IWBEM_SERVICES {
      services_pointer(owner)
    } else if *iid == IID_IWBEM_CLASS_OBJECT {
      class_object_pointer(owner)
    } else if *iid == IID_IWBEM_CALL_RESULT {
      call_result_pointer(owner)
    } else {
      return E_NOINTERFACE;
    };
    add_ref_owner(owner);
  }
  HRESULT(0)
}

unsafe extern "system" fn services_query_interface(
  this: *mut c_void,
  iid: *const GUID,
  result: *mut *mut c_void,
) -> HRESULT {
  unsafe { query_interface_owner(this.cast(), iid, result) }
}

unsafe extern "system" fn class_object_query_interface(
  this: *mut c_void,
  iid: *const GUID,
  result: *mut *mut c_void,
) -> HRESULT {
  unsafe { query_interface_owner(owner_from_class_object(this), iid, result) }
}

unsafe extern "system" fn call_result_query_interface(
  this: *mut c_void,
  iid: *const GUID,
  result: *mut *mut c_void,
) -> HRESULT {
  unsafe { query_interface_owner(owner_from_call_result(this), iid, result) }
}

unsafe fn add_ref_owner(owner: *mut GeneratedIWbemServicesFake) -> u32 {
  ADD_REF_CALLS.fetch_add(1, Ordering::SeqCst);
  let object = unsafe { &*owner };
  let count = object.references.fetch_add(1, Ordering::SeqCst) + 1;
  CURRENT_REF_COUNT.store(count, Ordering::SeqCst);
  count
}

unsafe extern "system" fn services_add_ref(this: *mut c_void) -> u32 {
  unsafe { add_ref_owner(this.cast()) }
}

unsafe extern "system" fn class_object_add_ref(this: *mut c_void) -> u32 {
  unsafe { add_ref_owner(owner_from_class_object(this)) }
}

unsafe extern "system" fn call_result_add_ref(this: *mut c_void) -> u32 {
  unsafe { add_ref_owner(owner_from_call_result(this)) }
}

unsafe fn release_owner(owner: *mut GeneratedIWbemServicesFake) -> u32 {
  RELEASE_CALLS.fetch_add(1, Ordering::SeqCst);
  let object = unsafe { &*owner };
  let count = object.references.fetch_sub(1, Ordering::SeqCst) - 1;
  CURRENT_REF_COUNT.store(count, Ordering::SeqCst);
  if count == 0 {
    unsafe {
      drop(Box::from_raw(owner));
    }
  }
  count
}

unsafe extern "system" fn services_release(this: *mut c_void) -> u32 {
  unsafe { release_owner(this.cast()) }
}

unsafe extern "system" fn class_object_release(this: *mut c_void) -> u32 {
  unsafe { release_owner(owner_from_class_object(this)) }
}

unsafe extern "system" fn call_result_release(this: *mut c_void) -> u32 {
  unsafe { release_owner(owner_from_call_result(this)) }
}

#[derive(Clone, Copy)]
enum WbemOutputView {
  Services,
  ClassObject,
  CallResult,
}

unsafe fn write_owned_view(
  owner: *mut GeneratedIWbemServicesFake,
  output: *mut *mut c_void,
  view: WbemOutputView,
) {
  unsafe {
    add_ref_owner(owner);
    *output = match view {
      WbemOutputView::Services => services_pointer(owner),
      WbemOutputView::ClassObject => class_object_pointer(owner),
      WbemOutputView::CallResult => call_result_pointer(owner),
    };
  }
}

unsafe extern "system" fn open_namespace(
  this: *mut c_void,
  _namespace: *mut c_void,
  flags: WBEM_GENERIC_FLAG_TYPE,
  context: *mut c_void,
  working_namespace: *mut *mut c_void,
  result: *mut *mut c_void,
) -> HRESULT {
  OPEN_NAMESPACE_CALLS.fetch_add(1, Ordering::SeqCst);
  OPEN_NAMESPACE_LAST_FLAGS.store(flags.0, Ordering::SeqCst);
  OPEN_NAMESPACE_CONTEXT_ARGUMENT_NULL.store(u32::from(context.is_null()), Ordering::SeqCst);
  OPEN_NAMESPACE_WORKING_ARGUMENT_NULL
    .store(u32::from(working_namespace.is_null()), Ordering::SeqCst);
  let working_is_null = working_namespace.is_null() || unsafe { (*working_namespace).is_null() };
  OPEN_NAMESPACE_WORKING_SLOT_NULL.store(u32::from(working_is_null), Ordering::SeqCst);
  OPEN_NAMESPACE_RESULT_ARGUMENT_NULL.store(u32::from(result.is_null()), Ordering::SeqCst);
  let result_is_null = result.is_null() || unsafe { (*result).is_null() };
  OPEN_NAMESPACE_RESULT_SLOT_NULL.store(u32::from(result_is_null), Ordering::SeqCst);
  if !context.is_null()
    || (!working_namespace.is_null() && !working_is_null)
    || (!result.is_null() && !result_is_null)
  {
    return E_POINTER;
  }
  let owner = this.cast::<GeneratedIWbemServicesFake>();
  match WMI_CONDITIONAL_MODE.load(Ordering::SeqCst) {
    0 if flags.0 == 0 && !working_namespace.is_null() && result.is_null() => {
      unsafe { write_owned_view(owner, working_namespace, WbemOutputView::Services) };
      HRESULT(0)
    }
    1 if flags.0 == 0x10 && working_namespace.is_null() && !result.is_null() => {
      unsafe { write_owned_view(owner, result, WbemOutputView::CallResult) };
      HRESULT(0)
    }
    -1 => {
      if !working_namespace.is_null() {
        unsafe { write_owned_view(owner, working_namespace, WbemOutputView::Services) };
      }
      if !result.is_null() {
        unsafe { write_owned_view(owner, result, WbemOutputView::CallResult) };
      }
      HRESULT(0x80004005u32 as i32)
    }
    _ => E_NOTIMPL,
  }
}

unsafe fn complete_conditional_call(
  this: *mut c_void,
  flags: WBEM_GENERIC_FLAG_TYPE,
  context: *mut c_void,
  synchronous_output: *mut *mut c_void,
  semisynchronous_output: *mut *mut c_void,
  synchronous_view: Option<WbemOutputView>,
) -> HRESULT {
  if !context.is_null()
    || (!synchronous_output.is_null() && unsafe { !(*synchronous_output).is_null() })
    || (!semisynchronous_output.is_null() && unsafe { !(*semisynchronous_output).is_null() })
  {
    return E_POINTER;
  }
  let owner = this.cast::<GeneratedIWbemServicesFake>();
  match WMI_CONDITIONAL_MODE.load(Ordering::SeqCst) {
    0 if flags.0 == 0
      && semisynchronous_output.is_null()
      && (synchronous_view.is_some() == !synchronous_output.is_null()) =>
    {
      if let Some(view) = synchronous_view {
        unsafe { write_owned_view(owner, synchronous_output, view) };
      }
      HRESULT(0)
    }
    1 if flags.0 == 0x10 && synchronous_output.is_null() && !semisynchronous_output.is_null() => {
      unsafe {
        write_owned_view(owner, semisynchronous_output, WbemOutputView::CallResult);
      }
      HRESULT(0)
    }
    -1 => {
      if !synchronous_output.is_null() {
        if let Some(view) = synchronous_view {
          unsafe { write_owned_view(owner, synchronous_output, view) };
        }
      }
      if !semisynchronous_output.is_null() {
        unsafe {
          write_owned_view(owner, semisynchronous_output, WbemOutputView::CallResult);
        }
      }
      HRESULT(0x80004005u32 as i32)
    }
    _ => E_NOTIMPL,
  }
}

unsafe extern "system" fn get_object(
  this: *mut c_void,
  object_path: *mut c_void,
  flags: WBEM_GENERIC_FLAG_TYPE,
  context: *mut c_void,
  object: *mut *mut c_void,
  result: *mut *mut c_void,
) -> HRESULT {
  GET_OBJECT_CALLS.fetch_add(1, Ordering::SeqCst);
  if object_path.is_null() {
    return E_POINTER;
  }
  unsafe {
    complete_conditional_call(
      this,
      flags,
      context,
      object,
      result,
      Some(WbemOutputView::ClassObject),
    )
  }
}

unsafe extern "system" fn put_class(
  this: *mut c_void,
  object: *mut c_void,
  flags: WBEM_GENERIC_FLAG_TYPE,
  context: *mut c_void,
  result: *mut *mut c_void,
) -> HRESULT {
  PUT_CLASS_CALLS.fetch_add(1, Ordering::SeqCst);
  if object.is_null() {
    return E_POINTER;
  }
  unsafe { complete_conditional_call(this, flags, context, std::ptr::null_mut(), result, None) }
}

unsafe extern "system" fn delete_class(
  this: *mut c_void,
  class_name: *mut c_void,
  flags: WBEM_GENERIC_FLAG_TYPE,
  context: *mut c_void,
  result: *mut *mut c_void,
) -> HRESULT {
  DELETE_CLASS_CALLS.fetch_add(1, Ordering::SeqCst);
  if class_name.is_null() {
    return E_POINTER;
  }
  unsafe { complete_conditional_call(this, flags, context, std::ptr::null_mut(), result, None) }
}

unsafe extern "system" fn put_instance(
  this: *mut c_void,
  instance: *mut c_void,
  flags: WBEM_GENERIC_FLAG_TYPE,
  context: *mut c_void,
  result: *mut *mut c_void,
) -> HRESULT {
  PUT_INSTANCE_CALLS.fetch_add(1, Ordering::SeqCst);
  if instance.is_null() {
    return E_POINTER;
  }
  unsafe { complete_conditional_call(this, flags, context, std::ptr::null_mut(), result, None) }
}

unsafe extern "system" fn delete_instance(
  this: *mut c_void,
  object_path: *mut c_void,
  flags: WBEM_GENERIC_FLAG_TYPE,
  context: *mut c_void,
  result: *mut *mut c_void,
) -> HRESULT {
  DELETE_INSTANCE_CALLS.fetch_add(1, Ordering::SeqCst);
  if object_path.is_null() {
    return E_POINTER;
  }
  unsafe { complete_conditional_call(this, flags, context, std::ptr::null_mut(), result, None) }
}

unsafe extern "system" fn exec_method(
  this: *mut c_void,
  object_path: *mut c_void,
  method_name: *mut c_void,
  flags: WBEM_GENERIC_FLAG_TYPE,
  context: *mut c_void,
  input: *mut c_void,
  output: *mut *mut c_void,
  result: *mut *mut c_void,
) -> HRESULT {
  EXEC_METHOD_CALLS.fetch_add(1, Ordering::SeqCst);
  if object_path.is_null() || method_name.is_null() || input.is_null() {
    return E_POINTER;
  }
  unsafe {
    complete_conditional_call(
      this,
      flags,
      context,
      output,
      result,
      Some(WbemOutputView::ClassObject),
    )
  }
}

unsafe extern "system" fn unimplemented_wbem_async_single(
  _this: *mut c_void,
  _value: *mut c_void,
  _flags: WBEM_GENERIC_FLAG_TYPE,
  _context: *mut c_void,
  _response_handler: *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn unimplemented_wbem_enum(
  _this: *mut c_void,
  _filter: *mut c_void,
  _flags: WBEM_GENERIC_FLAG_TYPE,
  _context: *mut c_void,
  _result: *mut *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn unimplemented_wbem_query(
  _this: *mut c_void,
  _query_language: *mut c_void,
  _query: *mut c_void,
  _flags: WBEM_GENERIC_FLAG_TYPE,
  _context: *mut c_void,
  _result: *mut *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn unimplemented_wbem_query_async(
  _this: *mut c_void,
  _query_language: *mut c_void,
  _query: *mut c_void,
  _flags: WBEM_GENERIC_FLAG_TYPE,
  _context: *mut c_void,
  _response_handler: *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn unimplemented_wbem_exec_method_async(
  _this: *mut c_void,
  _object_path: *mut c_void,
  _method_name: *mut c_void,
  _flags: WBEM_GENERIC_FLAG_TYPE,
  _context: *mut c_void,
  _input: *mut c_void,
  _response_handler: *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn cancel_async_call(_this: *mut c_void, _sink: *mut c_void) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn query_object_sink(
  _this: *mut c_void,
  flags: WBEM_GENERIC_FLAG_TYPE,
  result: *mut *mut c_void,
) -> HRESULT {
  if result.is_null() {
    return E_POINTER;
  }
  QUERY_OBJECT_SINK_CALLS.fetch_add(1, Ordering::SeqCst);
  LAST_FLAGS.store(flags.0, Ordering::SeqCst);
  unsafe {
    *result = std::ptr::null_mut();
  }
  LAST_OUTPUT_ADDRESS.store(0, Ordering::SeqCst);
  E_NOTIMPL
}

static SERVICES_VTABLE: IWbemServices_Vtbl = IWbemServices_Vtbl {
  base__: IUnknown_Vtbl {
    QueryInterface: services_query_interface,
    AddRef: services_add_ref,
    Release: services_release,
  },
  OpenNamespace: open_namespace,
  CancelAsyncCall: cancel_async_call,
  QueryObjectSink: query_object_sink,
  GetObject: get_object,
  GetObjectAsync: unimplemented_wbem_async_single,
  PutClass: put_class,
  PutClassAsync: unimplemented_wbem_async_single,
  DeleteClass: delete_class,
  DeleteClassAsync: unimplemented_wbem_async_single,
  CreateClassEnum: unimplemented_wbem_enum,
  CreateClassEnumAsync: unimplemented_wbem_async_single,
  PutInstance: put_instance,
  PutInstanceAsync: unimplemented_wbem_async_single,
  DeleteInstance: delete_instance,
  DeleteInstanceAsync: unimplemented_wbem_async_single,
  CreateInstanceEnum: unimplemented_wbem_enum,
  CreateInstanceEnumAsync: unimplemented_wbem_async_single,
  ExecQuery: unimplemented_wbem_query,
  ExecQueryAsync: unimplemented_wbem_query_async,
  ExecNotificationQuery: unimplemented_wbem_query,
  ExecNotificationQueryAsync: unimplemented_wbem_query_async,
  ExecMethod: exec_method,
  ExecMethodAsync: unimplemented_wbem_exec_method_async,
};

unsafe extern "system" fn class_object_out(
  _this: *mut c_void,
  _result: *mut *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn class_object_get(
  _this: *mut c_void,
  _name: PCWSTR,
  _flags: i32,
  _value: *mut VARIANT,
  _kind: *mut i32,
  _flavor: *mut i32,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn class_object_put(
  _this: *mut c_void,
  _name: PCWSTR,
  _flags: i32,
  _value: *const VARIANT,
  _kind: i32,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn class_object_name(_this: *mut c_void, _name: PCWSTR) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn class_object_get_names(
  _this: *mut c_void,
  _qualifier_name: PCWSTR,
  _flags: WBEM_CONDITION_FLAG_TYPE,
  _qualifier_value: *const VARIANT,
  _names: *mut *mut SAFEARRAY,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn class_object_flags(_this: *mut c_void, _flags: i32) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn class_object_next(
  _this: *mut c_void,
  _flags: i32,
  _name: *mut *mut c_void,
  _value: *mut VARIANT,
  _kind: *mut i32,
  _flavor: *mut i32,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn class_object_end_enumeration(_this: *mut c_void) -> HRESULT {
  CLASS_OBJECT_END_ENUMERATION_CALLS.fetch_add(1, Ordering::SeqCst);
  HRESULT(0)
}

unsafe extern "system" fn class_object_named_out(
  _this: *mut c_void,
  _name: PCWSTR,
  _result: *mut *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn class_object_flags_out(
  _this: *mut c_void,
  _flags: i32,
  _result: *mut *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn class_object_compare(
  _this: *mut c_void,
  _flags: WBEM_COMPARISON_FLAG,
  _other: *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn class_object_get_method(
  _this: *mut c_void,
  _name: PCWSTR,
  _flags: i32,
  _input: *mut *mut c_void,
  _output: *mut *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn class_object_put_method(
  _this: *mut c_void,
  _name: PCWSTR,
  _flags: i32,
  _input: *mut c_void,
  _output: *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn class_object_next_method(
  _this: *mut c_void,
  _flags: i32,
  _name: *mut *mut c_void,
  _input: *mut *mut c_void,
  _output: *mut *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn class_object_end_method_enumeration(_this: *mut c_void) -> HRESULT {
  E_NOTIMPL
}

static CLASS_OBJECT_VTABLE: IWbemClassObject_Vtbl = IWbemClassObject_Vtbl {
  base__: IUnknown_Vtbl {
    QueryInterface: class_object_query_interface,
    AddRef: class_object_add_ref,
    Release: class_object_release,
  },
  GetQualifierSet: class_object_out,
  Get: class_object_get,
  Put: class_object_put,
  Delete: class_object_name,
  GetNames: class_object_get_names,
  BeginEnumeration: class_object_flags,
  Next: class_object_next,
  EndEnumeration: class_object_end_enumeration,
  GetPropertyQualifierSet: class_object_named_out,
  Clone: class_object_out,
  GetObjectText: class_object_flags_out,
  SpawnDerivedClass: class_object_flags_out,
  SpawnInstance: class_object_flags_out,
  CompareTo: class_object_compare,
  GetPropertyOrigin: class_object_named_out,
  InheritsFrom: class_object_name,
  GetMethod: class_object_get_method,
  PutMethod: class_object_put_method,
  DeleteMethod: class_object_name,
  BeginMethodEnumeration: class_object_flags,
  NextMethod: class_object_next_method,
  EndMethodEnumeration: class_object_end_method_enumeration,
  GetMethodQualifierSet: class_object_named_out,
  GetMethodOrigin: class_object_named_out,
};

unsafe extern "system" fn call_result_object(
  _this: *mut c_void,
  _timeout: i32,
  _result: *mut *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn call_result_get_status(
  _this: *mut c_void,
  _timeout: i32,
  status: *mut i32,
) -> HRESULT {
  if status.is_null() {
    return E_POINTER;
  }
  CALL_RESULT_GET_STATUS_CALLS.fetch_add(1, Ordering::SeqCst);
  unsafe {
    *status = 0;
  }
  HRESULT(0)
}

static CALL_RESULT_VTABLE: IWbemCallResult_Vtbl = IWbemCallResult_Vtbl {
  base__: IUnknown_Vtbl {
    QueryInterface: call_result_query_interface,
    AddRef: call_result_add_ref,
    Release: call_result_release,
  },
  GetResultObject: call_result_object,
  GetResultString: call_result_object,
  GetResultServices: call_result_object,
  GetCallStatus: call_result_get_status,
};

#[napi(object)]
pub struct GeneratedUnsafeComStats {
  pub query_interface_calls: u32,
  pub add_ref_calls: u32,
  pub release_calls: u32,
  pub open_namespace_calls: u32,
  pub open_namespace_working_slot_null: bool,
  pub open_namespace_working_argument_null: bool,
  pub open_namespace_result_slot_null: bool,
  pub open_namespace_result_argument_null: bool,
  pub open_namespace_context_argument_null: bool,
  pub open_namespace_last_flags: i32,
  pub get_object_calls: u32,
  pub put_class_calls: u32,
  pub delete_class_calls: u32,
  pub put_instance_calls: u32,
  pub delete_instance_calls: u32,
  pub exec_method_calls: u32,
  pub query_object_sink_calls: u32,
  pub last_flags: i32,
  pub current_ref_count: u32,
  pub last_output_address: u32,
  pub class_object_end_enumeration_calls: u32,
  pub call_result_get_status_calls: u32,
}

#[napi]
pub fn create_generated_iwbem_services_fake() -> napi::Result<DynWinRTValue> {
  QUERY_INTERFACE_CALLS.store(0, Ordering::SeqCst);
  ADD_REF_CALLS.store(0, Ordering::SeqCst);
  RELEASE_CALLS.store(0, Ordering::SeqCst);
  OPEN_NAMESPACE_CALLS.store(0, Ordering::SeqCst);
  WMI_CONDITIONAL_MODE.store(0, Ordering::SeqCst);
  OPEN_NAMESPACE_WORKING_SLOT_NULL.store(0, Ordering::SeqCst);
  OPEN_NAMESPACE_WORKING_ARGUMENT_NULL.store(0, Ordering::SeqCst);
  OPEN_NAMESPACE_RESULT_SLOT_NULL.store(0, Ordering::SeqCst);
  OPEN_NAMESPACE_RESULT_ARGUMENT_NULL.store(0, Ordering::SeqCst);
  OPEN_NAMESPACE_CONTEXT_ARGUMENT_NULL.store(0, Ordering::SeqCst);
  OPEN_NAMESPACE_LAST_FLAGS.store(0, Ordering::SeqCst);
  GET_OBJECT_CALLS.store(0, Ordering::SeqCst);
  PUT_CLASS_CALLS.store(0, Ordering::SeqCst);
  DELETE_CLASS_CALLS.store(0, Ordering::SeqCst);
  PUT_INSTANCE_CALLS.store(0, Ordering::SeqCst);
  DELETE_INSTANCE_CALLS.store(0, Ordering::SeqCst);
  EXEC_METHOD_CALLS.store(0, Ordering::SeqCst);
  QUERY_OBJECT_SINK_CALLS.store(0, Ordering::SeqCst);
  LAST_FLAGS.store(0, Ordering::SeqCst);
  CURRENT_REF_COUNT.store(1, Ordering::SeqCst);
  LAST_OUTPUT_ADDRESS.store(0, Ordering::SeqCst);
  CLASS_OBJECT_END_ENUMERATION_CALLS.store(0, Ordering::SeqCst);
  CALL_RESULT_GET_STATUS_CALLS.store(0, Ordering::SeqCst);

  let object = Box::new(GeneratedIWbemServicesFake {
    services_vtable: &SERVICES_VTABLE,
    class_object_vtable: &CLASS_OBJECT_VTABLE,
    call_result_vtable: &CALL_RESULT_VTABLE,
    references: AtomicU32::new(1),
  });
  let unknown = unsafe { IUnknown::from_raw(Box::into_raw(object).cast()) };
  crate::com::apartment_bound_com_object(unknown)
}

#[napi]
pub fn probe_generated_iwbem_services_unimplemented_slots(
  value: &DynWinRTValue,
) -> napi::Result<()> {
  let raw = match &value.0 {
    dynwinrt::WinRTValue::Object(object) => object.as_raw(),
    _ => {
      return Err(napi::Error::from_reason(
        "IWbemServices probe requires a managed COM object",
      ));
    }
  };
  let vtable = unsafe { *(raw.cast::<*const IWbemServices_Vtbl>()) };
  let mut object_sink = std::ptr::null_mut();
  let query_object_sink =
    unsafe { ((*vtable).QueryObjectSink)(raw, WBEM_GENERIC_FLAG_TYPE(0), &mut object_sink) };
  let get_object_async = unsafe {
    ((*vtable).GetObjectAsync)(
      raw,
      std::ptr::null_mut(),
      WBEM_GENERIC_FLAG_TYPE(0),
      std::ptr::null_mut(),
      std::ptr::null_mut(),
    )
  };
  let exec_method_async = unsafe {
    ((*vtable).ExecMethodAsync)(
      raw,
      std::ptr::null_mut(),
      std::ptr::null_mut(),
      WBEM_GENERIC_FLAG_TYPE(0),
      std::ptr::null_mut(),
      std::ptr::null_mut(),
      std::ptr::null_mut(),
    )
  };
  if query_object_sink != E_NOTIMPL
    || !object_sink.is_null()
    || get_object_async != E_NOTIMPL
    || exec_method_async != E_NOTIMPL
  {
    return Err(napi::Error::from_reason(
      "IWbemServices unimplemented slot probe returned an unexpected HRESULT",
    ));
  }
  Ok(())
}

#[napi]
pub fn generated_unsafe_com_stats() -> GeneratedUnsafeComStats {
  GeneratedUnsafeComStats {
    query_interface_calls: QUERY_INTERFACE_CALLS.load(Ordering::SeqCst),
    add_ref_calls: ADD_REF_CALLS.load(Ordering::SeqCst),
    release_calls: RELEASE_CALLS.load(Ordering::SeqCst),
    open_namespace_calls: OPEN_NAMESPACE_CALLS.load(Ordering::SeqCst),
    open_namespace_working_slot_null: OPEN_NAMESPACE_WORKING_SLOT_NULL.load(Ordering::SeqCst) != 0,
    open_namespace_working_argument_null: OPEN_NAMESPACE_WORKING_ARGUMENT_NULL
      .load(Ordering::SeqCst)
      != 0,
    open_namespace_result_slot_null: OPEN_NAMESPACE_RESULT_SLOT_NULL.load(Ordering::SeqCst) != 0,
    open_namespace_result_argument_null: OPEN_NAMESPACE_RESULT_ARGUMENT_NULL.load(Ordering::SeqCst)
      != 0,
    open_namespace_context_argument_null: OPEN_NAMESPACE_CONTEXT_ARGUMENT_NULL
      .load(Ordering::SeqCst)
      != 0,
    open_namespace_last_flags: OPEN_NAMESPACE_LAST_FLAGS.load(Ordering::SeqCst),
    get_object_calls: GET_OBJECT_CALLS.load(Ordering::SeqCst),
    put_class_calls: PUT_CLASS_CALLS.load(Ordering::SeqCst),
    delete_class_calls: DELETE_CLASS_CALLS.load(Ordering::SeqCst),
    put_instance_calls: PUT_INSTANCE_CALLS.load(Ordering::SeqCst),
    delete_instance_calls: DELETE_INSTANCE_CALLS.load(Ordering::SeqCst),
    exec_method_calls: EXEC_METHOD_CALLS.load(Ordering::SeqCst),
    query_object_sink_calls: QUERY_OBJECT_SINK_CALLS.load(Ordering::SeqCst),
    last_flags: LAST_FLAGS.load(Ordering::SeqCst),
    current_ref_count: CURRENT_REF_COUNT.load(Ordering::SeqCst),
    last_output_address: LAST_OUTPUT_ADDRESS.load(Ordering::SeqCst) as u32,
    class_object_end_enumeration_calls: CLASS_OBJECT_END_ENUMERATION_CALLS.load(Ordering::SeqCst),
    call_result_get_status_calls: CALL_RESULT_GET_STATUS_CALLS.load(Ordering::SeqCst),
  }
}

#[napi]
pub fn set_generated_iwbem_services_conditional_mode(mode: i32) -> napi::Result<()> {
  if !matches!(mode, -1..=1) {
    return Err(napi::Error::from_reason(
      "IWbemServices conditional test mode must be -1, 0, or 1",
    ));
  }
  WMI_CONDITIONAL_MODE.store(mode, Ordering::SeqCst);
  Ok(())
}

#[repr(C)]
struct GeneratedThumbnailProviderVtbl {
  base__: IUnknown_Vtbl,
  get_thumbnail: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void, *mut i32) -> HRESULT,
}

#[repr(C)]
struct GeneratedThumbnailProviderFake {
  vtable: *const GeneratedThumbnailProviderVtbl,
  references: AtomicU32,
}

unsafe extern "system" fn thumbnail_query_interface(
  this: *mut c_void,
  iid: *const GUID,
  result: *mut *mut c_void,
) -> HRESULT {
  if iid.is_null() || result.is_null() {
    return E_POINTER;
  }
  unsafe {
    *result = std::ptr::null_mut();
    if *iid != IUnknown::IID && *iid != IID_ITHUMBNAIL_PROVIDER {
      return E_NOINTERFACE;
    }
    *result = this;
    thumbnail_add_ref(this);
  }
  HRESULT(0)
}

unsafe extern "system" fn thumbnail_add_ref(this: *mut c_void) -> u32 {
  let object = unsafe { &*this.cast::<GeneratedThumbnailProviderFake>() };
  let count = object.references.fetch_add(1, Ordering::SeqCst) + 1;
  THUMBNAIL_CURRENT_REF_COUNT.store(count, Ordering::SeqCst);
  count
}

unsafe extern "system" fn thumbnail_release(this: *mut c_void) -> u32 {
  let object = unsafe { &*this.cast::<GeneratedThumbnailProviderFake>() };
  let count = object.references.fetch_sub(1, Ordering::SeqCst) - 1;
  THUMBNAIL_CURRENT_REF_COUNT.store(count, Ordering::SeqCst);
  if count == 0 {
    unsafe {
      drop(Box::from_raw(this.cast::<GeneratedThumbnailProviderFake>()));
    }
  }
  count
}

unsafe extern "system" fn get_thumbnail(
  _this: *mut c_void,
  size: u32,
  bitmap: *mut *mut c_void,
  alpha_type: *mut i32,
) -> HRESULT {
  THUMBNAIL_CALLS.fetch_add(1, Ordering::SeqCst);
  if size == 0 || bitmap.is_null() || alpha_type.is_null() {
    return E_POINTER;
  }
  if unsafe { !(*bitmap).is_null() } {
    return E_POINTER;
  }
  let bitmap_value = unsafe { windows::Win32::Graphics::Gdi::CreateBitmap(1, 1, 1, 1, None) };
  if bitmap_value.is_invalid() {
    return HRESULT(0x80004005u32 as i32);
  }
  unsafe {
    *bitmap = bitmap_value.0;
    *alpha_type = 2;
  }
  HRESULT(0)
}

static THUMBNAIL_VTABLE: GeneratedThumbnailProviderVtbl = GeneratedThumbnailProviderVtbl {
  base__: IUnknown_Vtbl {
    QueryInterface: thumbnail_query_interface,
    AddRef: thumbnail_add_ref,
    Release: thumbnail_release,
  },
  get_thumbnail,
};

#[napi(object)]
pub struct GeneratedThumbnailProviderStats {
  pub calls: u32,
  pub current_ref_count: u32,
}

#[napi]
pub fn create_generated_thumbnail_provider_fake() -> napi::Result<DynWinRTValue> {
  THUMBNAIL_CALLS.store(0, Ordering::SeqCst);
  THUMBNAIL_CURRENT_REF_COUNT.store(1, Ordering::SeqCst);
  let object = Box::new(GeneratedThumbnailProviderFake {
    vtable: &THUMBNAIL_VTABLE,
    references: AtomicU32::new(1),
  });
  let unknown = unsafe { IUnknown::from_raw(Box::into_raw(object).cast()) };
  crate::com::apartment_bound_com_object(unknown)
}

#[napi]
pub fn generated_thumbnail_provider_stats() -> GeneratedThumbnailProviderStats {
  GeneratedThumbnailProviderStats {
    calls: THUMBNAIL_CALLS.load(Ordering::SeqCst),
    current_ref_count: THUMBNAIL_CURRENT_REF_COUNT.load(Ordering::SeqCst),
  }
}

#[repr(C)]
struct GeneratedDataObjectVtbl {
  base__: IUnknown_Vtbl,
  get_data: unsafe extern "system" fn(
    *mut c_void,
    *const windows::Win32::System::Com::FORMATETC,
    *mut windows::Win32::System::Com::STGMEDIUM,
  ) -> HRESULT,
  get_data_here: unsafe extern "system" fn(
    *mut c_void,
    *const windows::Win32::System::Com::FORMATETC,
    *mut windows::Win32::System::Com::STGMEDIUM,
  ) -> HRESULT,
  query_get_data: unsafe extern "system" fn(
    *mut c_void,
    *const windows::Win32::System::Com::FORMATETC,
  ) -> HRESULT,
  get_canonical_format_etc: unsafe extern "system" fn(
    *mut c_void,
    *const windows::Win32::System::Com::FORMATETC,
    *mut windows::Win32::System::Com::FORMATETC,
  ) -> HRESULT,
  set_data: unsafe extern "system" fn(
    *mut c_void,
    *const windows::Win32::System::Com::FORMATETC,
    *const windows::Win32::System::Com::STGMEDIUM,
    i32,
  ) -> HRESULT,
  enum_format_etc: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
  d_advise: unsafe extern "system" fn(
    *mut c_void,
    *const windows::Win32::System::Com::FORMATETC,
    u32,
    *mut c_void,
    *mut u32,
  ) -> HRESULT,
  d_unadvise: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
  enum_d_advise: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct GeneratedDataObjectFake {
  vtable: *const GeneratedDataObjectVtbl,
  references: AtomicU32,
}

unsafe extern "system" fn data_object_query_interface(
  this: *mut c_void,
  iid: *const GUID,
  result: *mut *mut c_void,
) -> HRESULT {
  if iid.is_null() || result.is_null() {
    return E_POINTER;
  }
  unsafe {
    *result = std::ptr::null_mut();
    if *iid != IUnknown::IID && *iid != IID_IDATA_OBJECT {
      return E_NOINTERFACE;
    }
    *result = this;
    data_object_add_ref(this);
  }
  HRESULT(0)
}

unsafe extern "system" fn data_object_add_ref(this: *mut c_void) -> u32 {
  let object = unsafe { &*this.cast::<GeneratedDataObjectFake>() };
  let count = object.references.fetch_add(1, Ordering::SeqCst) + 1;
  DATA_OBJECT_CURRENT_REF_COUNT.store(count, Ordering::SeqCst);
  count
}

unsafe extern "system" fn data_object_release(this: *mut c_void) -> u32 {
  let object = unsafe { &*this.cast::<GeneratedDataObjectFake>() };
  let count = object.references.fetch_sub(1, Ordering::SeqCst) - 1;
  DATA_OBJECT_CURRENT_REF_COUNT.store(count, Ordering::SeqCst);
  if count == 0 {
    unsafe {
      drop(Box::from_raw(this.cast::<GeneratedDataObjectFake>()));
    }
  }
  count
}

fn valid_hglobal_format(format: *const windows::Win32::System::Com::FORMATETC) -> bool {
  !format.is_null()
    && unsafe {
      (*format).cfFormat == 13
        && (*format).ptd.is_null()
        && (*format).dwAspect == 1
        && (*format).lindex == -1
        && (*format).tymed == windows::Win32::System::Com::TYMED_HGLOBAL.0 as u32
    }
}

unsafe fn write_hglobal_bytes(
  medium: *mut windows::Win32::System::Com::STGMEDIUM,
  bytes: &[u8],
) -> HRESULT {
  if medium.is_null() || unsafe { (*medium).tymed != 0 } {
    return E_POINTER;
  }
  let handle = match unsafe {
    windows::Win32::System::Memory::GlobalAlloc(
      windows::Win32::System::Memory::GMEM_MOVEABLE | windows::Win32::System::Memory::GMEM_ZEROINIT,
      bytes.len(),
    )
  } {
    Ok(handle) => handle,
    Err(_) => return HRESULT(0x8007000eu32 as i32),
  };
  let data = unsafe { windows::Win32::System::Memory::GlobalLock(handle) };
  if data.is_null() {
    unsafe {
      let _ = windows::Win32::Foundation::GlobalFree(Some(handle));
    }
    return HRESULT(0x8007000eu32 as i32);
  }
  unsafe {
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), data.cast::<u8>(), bytes.len());
    let _ = windows::Win32::System::Memory::GlobalUnlock(handle);
    medium.write(windows::Win32::System::Com::STGMEDIUM {
      tymed: windows::Win32::System::Com::TYMED_HGLOBAL.0 as u32,
      u: windows::Win32::System::Com::STGMEDIUM_0 { hGlobal: handle },
      pUnkForRelease: std::mem::ManuallyDrop::new(None),
    });
  }
  DATA_OBJECT_LAST_OUTPUT_HANDLE.store(handle.0.addr(), Ordering::SeqCst);
  HRESULT(0)
}

unsafe fn read_hglobal_bytes(
  medium: *const windows::Win32::System::Com::STGMEDIUM,
) -> Option<Vec<u8>> {
  if medium.is_null()
    || unsafe { (*medium).tymed } != windows::Win32::System::Com::TYMED_HGLOBAL.0 as u32
  {
    return None;
  }
  let handle = unsafe { (*medium).u.hGlobal };
  let size = unsafe { windows::Win32::System::Memory::GlobalSize(handle) };
  let data = unsafe { windows::Win32::System::Memory::GlobalLock(handle) };
  if size > 0 && data.is_null() {
    return None;
  }
  let bytes = if size == 0 {
    Vec::new()
  } else {
    unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size) }.to_vec()
  };
  if !data.is_null() {
    unsafe {
      let _ = windows::Win32::System::Memory::GlobalUnlock(handle);
    }
  }
  Some(bytes)
}

unsafe extern "system" fn data_object_get_data(
  _this: *mut c_void,
  format: *const windows::Win32::System::Com::FORMATETC,
  medium: *mut windows::Win32::System::Com::STGMEDIUM,
) -> HRESULT {
  DATA_OBJECT_GET_DATA_CALLS.fetch_add(1, Ordering::SeqCst);
  if !valid_hglobal_format(format) {
    return HRESULT(0x80040064u32 as i32);
  }
  unsafe { write_hglobal_bytes(medium, &[1, 2, 3, 4]) }
}

unsafe extern "system" fn data_object_get_data_here(
  _this: *mut c_void,
  format: *const windows::Win32::System::Com::FORMATETC,
  medium: *mut windows::Win32::System::Com::STGMEDIUM,
) -> HRESULT {
  DATA_OBJECT_GET_DATA_HERE_CALLS.fetch_add(1, Ordering::SeqCst);
  if !valid_hglobal_format(format) {
    return HRESULT(0x80040064u32 as i32);
  }
  let Some(mut bytes) = (unsafe { read_hglobal_bytes(medium) }) else {
    return HRESULT(0x80040069u32 as i32);
  };
  if bytes.len() < 4 {
    return HRESULT(0x80030070u32 as i32);
  }
  bytes[..4].copy_from_slice(&[9, 8, 7, 6]);
  let handle = unsafe { (*medium).u.hGlobal };
  DATA_OBJECT_LAST_GET_DATA_HERE_HANDLE.store(handle.0.addr(), Ordering::SeqCst);
  let data = unsafe { windows::Win32::System::Memory::GlobalLock(handle) };
  unsafe {
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), data.cast::<u8>(), bytes.len());
    let _ = windows::Win32::System::Memory::GlobalUnlock(handle);
  }
  HRESULT(0)
}

unsafe extern "system" fn data_object_query_get_data(
  _this: *mut c_void,
  format: *const windows::Win32::System::Com::FORMATETC,
) -> HRESULT {
  DATA_OBJECT_QUERY_GET_DATA_CALLS.fetch_add(1, Ordering::SeqCst);
  if valid_hglobal_format(format) {
    HRESULT(0)
  } else {
    HRESULT(0x80040064u32 as i32)
  }
}

unsafe extern "system" fn data_object_get_canonical_format_etc(
  _this: *mut c_void,
  input: *const windows::Win32::System::Com::FORMATETC,
  output: *mut windows::Win32::System::Com::FORMATETC,
) -> HRESULT {
  DATA_OBJECT_CANONICAL_CALLS.fetch_add(1, Ordering::SeqCst);
  if !valid_hglobal_format(input) || output.is_null() {
    return E_POINTER;
  }
  unsafe {
    output.write(windows::Win32::System::Com::FORMATETC {
      cfFormat: (*input).cfFormat,
      ptd: std::ptr::null_mut(),
      dwAspect: (*input).dwAspect,
      lindex: (*input).lindex,
      tymed: (*input).tymed,
    });
  }
  HRESULT(0)
}

unsafe extern "system" fn data_object_set_data(
  _this: *mut c_void,
  format: *const windows::Win32::System::Com::FORMATETC,
  medium: *const windows::Win32::System::Com::STGMEDIUM,
  release_medium: i32,
) -> HRESULT {
  DATA_OBJECT_SET_DATA_CALLS.fetch_add(1, Ordering::SeqCst);
  DATA_OBJECT_LAST_SET_RELEASE.store(release_medium, Ordering::SeqCst);
  if !medium.is_null()
    && unsafe { (*medium).tymed } == windows::Win32::System::Com::TYMED_HGLOBAL.0 as u32
  {
    DATA_OBJECT_LAST_SET_DATA_HANDLE
      .store(unsafe { (*medium).u.hGlobal }.0.addr(), Ordering::SeqCst);
  }
  if !valid_hglobal_format(format)
    || release_medium != 0
    || unsafe { read_hglobal_bytes(medium) }.as_deref() != Some(&[5, 6, 7, 8])
  {
    return HRESULT(0x80070057u32 as i32);
  }
  HRESULT(0)
}

unsafe extern "system" fn data_object_enum_format_etc(
  _this: *mut c_void,
  _direction: u32,
  _result: *mut *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn data_object_d_advise(
  _this: *mut c_void,
  _format: *const windows::Win32::System::Com::FORMATETC,
  _flags: u32,
  _sink: *mut c_void,
  _connection: *mut u32,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn data_object_d_unadvise(_this: *mut c_void, _connection: u32) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn data_object_enum_d_advise(
  _this: *mut c_void,
  _result: *mut *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

static DATA_OBJECT_VTABLE: GeneratedDataObjectVtbl = GeneratedDataObjectVtbl {
  base__: IUnknown_Vtbl {
    QueryInterface: data_object_query_interface,
    AddRef: data_object_add_ref,
    Release: data_object_release,
  },
  get_data: data_object_get_data,
  get_data_here: data_object_get_data_here,
  query_get_data: data_object_query_get_data,
  get_canonical_format_etc: data_object_get_canonical_format_etc,
  set_data: data_object_set_data,
  enum_format_etc: data_object_enum_format_etc,
  d_advise: data_object_d_advise,
  d_unadvise: data_object_d_unadvise,
  enum_d_advise: data_object_enum_d_advise,
};

#[napi(object)]
pub struct GeneratedDataObjectStats {
  pub get_data_calls: u32,
  pub get_data_here_calls: u32,
  pub query_get_data_calls: u32,
  pub canonical_calls: u32,
  pub set_data_calls: u32,
  pub last_set_release: i32,
  pub output_released: bool,
  pub get_data_here_input_released: bool,
  pub set_data_input_released: bool,
  pub current_ref_count: u32,
}

#[napi]
pub fn create_generated_data_object_fake() -> napi::Result<DynWinRTValue> {
  DATA_OBJECT_GET_DATA_CALLS.store(0, Ordering::SeqCst);
  DATA_OBJECT_GET_DATA_HERE_CALLS.store(0, Ordering::SeqCst);
  DATA_OBJECT_QUERY_GET_DATA_CALLS.store(0, Ordering::SeqCst);
  DATA_OBJECT_CANONICAL_CALLS.store(0, Ordering::SeqCst);
  DATA_OBJECT_SET_DATA_CALLS.store(0, Ordering::SeqCst);
  DATA_OBJECT_LAST_SET_RELEASE.store(-1, Ordering::SeqCst);
  DATA_OBJECT_LAST_OUTPUT_HANDLE.store(0, Ordering::SeqCst);
  DATA_OBJECT_LAST_GET_DATA_HERE_HANDLE.store(0, Ordering::SeqCst);
  DATA_OBJECT_LAST_SET_DATA_HANDLE.store(0, Ordering::SeqCst);
  DATA_OBJECT_CURRENT_REF_COUNT.store(1, Ordering::SeqCst);
  let object = Box::new(GeneratedDataObjectFake {
    vtable: &DATA_OBJECT_VTABLE,
    references: AtomicU32::new(1),
  });
  let unknown = unsafe { IUnknown::from_raw(Box::into_raw(object).cast()) };
  crate::com::apartment_bound_com_object(unknown)
}

#[napi]
pub fn generated_data_object_stats() -> GeneratedDataObjectStats {
  let handle_released = |address| {
    address != 0
      && unsafe {
        windows::Win32::System::Memory::GlobalSize(windows::Win32::Foundation::HGLOBAL(
          std::ptr::with_exposed_provenance_mut(address),
        ))
      } == 0
  };
  GeneratedDataObjectStats {
    get_data_calls: DATA_OBJECT_GET_DATA_CALLS.load(Ordering::SeqCst),
    get_data_here_calls: DATA_OBJECT_GET_DATA_HERE_CALLS.load(Ordering::SeqCst),
    query_get_data_calls: DATA_OBJECT_QUERY_GET_DATA_CALLS.load(Ordering::SeqCst),
    canonical_calls: DATA_OBJECT_CANONICAL_CALLS.load(Ordering::SeqCst),
    set_data_calls: DATA_OBJECT_SET_DATA_CALLS.load(Ordering::SeqCst),
    last_set_release: DATA_OBJECT_LAST_SET_RELEASE.load(Ordering::SeqCst),
    output_released: handle_released(DATA_OBJECT_LAST_OUTPUT_HANDLE.load(Ordering::SeqCst)),
    get_data_here_input_released: handle_released(
      DATA_OBJECT_LAST_GET_DATA_HERE_HANDLE.load(Ordering::SeqCst),
    ),
    set_data_input_released: handle_released(
      DATA_OBJECT_LAST_SET_DATA_HANDLE.load(Ordering::SeqCst),
    ),
    current_ref_count: DATA_OBJECT_CURRENT_REF_COUNT.load(Ordering::SeqCst),
  }
}

#[repr(C)]
struct GeneratedAudioClientVtbl {
  base__: IUnknown_Vtbl,
  initialize:
    unsafe extern "system" fn(*mut c_void, i32, u32, i64, i64, *mut c_void, *const GUID) -> HRESULT,
  get_buffer_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
  get_stream_latency: unsafe extern "system" fn(*mut c_void, *mut i64) -> HRESULT,
  get_current_padding: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
  is_format_supported:
    unsafe extern "system" fn(*mut c_void, i32, *mut c_void, *mut *mut c_void) -> HRESULT,
  get_mix_format: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
  get_device_period: unsafe extern "system" fn(*mut c_void, *mut i64, *mut i64) -> HRESULT,
  start: unsafe extern "system" fn(*mut c_void) -> HRESULT,
  stop: unsafe extern "system" fn(*mut c_void) -> HRESULT,
  reset: unsafe extern "system" fn(*mut c_void) -> HRESULT,
  set_event_handle: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
  get_service: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct GeneratedAudioClientFake {
  vtable: *const GeneratedAudioClientVtbl,
  references: AtomicU32,
}

unsafe extern "system" fn audio_query_interface(
  this: *mut c_void,
  iid: *const GUID,
  result: *mut *mut c_void,
) -> HRESULT {
  if iid.is_null() || result.is_null() {
    return E_POINTER;
  }
  unsafe {
    *result = std::ptr::null_mut();
    if *iid != IUnknown::IID && *iid != IID_IAUDIO_CLIENT {
      return E_NOINTERFACE;
    }
    *result = this;
    audio_add_ref(this);
  }
  HRESULT(0)
}

unsafe extern "system" fn audio_add_ref(this: *mut c_void) -> u32 {
  AUDIO_ADD_REF_CALLS.fetch_add(1, Ordering::SeqCst);
  let object = unsafe { &*this.cast::<GeneratedAudioClientFake>() };
  let count = object.references.fetch_add(1, Ordering::SeqCst) + 1;
  AUDIO_CURRENT_REF_COUNT.store(count, Ordering::SeqCst);
  count
}

unsafe extern "system" fn audio_release(this: *mut c_void) -> u32 {
  AUDIO_RELEASE_CALLS.fetch_add(1, Ordering::SeqCst);
  let object = unsafe { &*this.cast::<GeneratedAudioClientFake>() };
  let count = object.references.fetch_sub(1, Ordering::SeqCst) - 1;
  AUDIO_CURRENT_REF_COUNT.store(count, Ordering::SeqCst);
  if count == 0 {
    unsafe {
      drop(Box::from_raw(this.cast::<GeneratedAudioClientFake>()));
    }
  }
  count
}

unsafe extern "system" fn audio_initialize(
  _this: *mut c_void,
  _share_mode: i32,
  _stream_flags: u32,
  _buffer_duration: i64,
  _periodicity: i64,
  _format: *mut c_void,
  _session: *const GUID,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn audio_get_u32(_this: *mut c_void, _value: *mut u32) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn audio_get_i64(_this: *mut c_void, _value: *mut i64) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn audio_is_format_supported(
  _this: *mut c_void,
  share_mode: i32,
  format: *mut c_void,
  closest: *mut *mut c_void,
) -> HRESULT {
  if format.is_null() || closest.is_null() {
    return E_POINTER;
  }
  AUDIO_IS_FORMAT_SUPPORTED_CALLS.fetch_add(1, Ordering::SeqCst);
  AUDIO_LAST_SHARE_MODE.store(share_mode, Ordering::SeqCst);
  AUDIO_LAST_FORMAT_TAG.store(
    unsafe { u32::from(*format.cast::<u16>()) },
    Ordering::SeqCst,
  );
  let allocation = unsafe { windows::Win32::System::Com::CoTaskMemAlloc(16) };
  if allocation.is_null() {
    return HRESULT(0x8007000eu32 as i32);
  }
  unsafe {
    allocation.cast::<u32>().write(0xaabb_ccdd);
    *closest = allocation;
  }
  if share_mode == 2 {
    HRESULT(0x80004005u32 as i32)
  } else {
    HRESULT(1)
  }
}

unsafe extern "system" fn audio_get_mix_format(
  _this: *mut c_void,
  _format: *mut *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn audio_get_device_period(
  _this: *mut c_void,
  _default: *mut i64,
  _minimum: *mut i64,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn audio_no_args(_this: *mut c_void) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn audio_set_event_handle(
  _this: *mut c_void,
  _event: *mut c_void,
) -> HRESULT {
  E_NOTIMPL
}

unsafe extern "system" fn audio_get_service(
  this: *mut c_void,
  _iid: *const GUID,
  result: *mut *mut c_void,
) -> HRESULT {
  if result.is_null() {
    return E_POINTER;
  }
  AUDIO_GET_SERVICE_CALLS.fetch_add(1, Ordering::SeqCst);
  let mode = AUDIO_GET_SERVICE_MODE.load(Ordering::SeqCst);
  unsafe {
    *result = std::ptr::null_mut();
    if mode == 2 {
      return HRESULT(0);
    }
    if mode == -2 {
      return HRESULT(0x80004005u32 as i32);
    }
    if mode.abs() == 3 {
      *result = windows::core::BSTR::from("stage2-bstr")
        .into_raw()
        .cast_mut()
        .cast();
      return if mode < 0 {
        HRESULT(0x80004005u32 as i32)
      } else {
        HRESULT(0)
      };
    }
    if mode.abs() == 4 {
      *result =
        windows::Win32::System::Memory::LocalAlloc(windows::Win32::System::Memory::LMEM_FIXED, 32)
          .map_or(std::ptr::null_mut(), |value| value.0);
      return if mode < 0 {
        HRESULT(0x80004005u32 as i32)
      } else {
        HRESULT(0)
      };
    }
    if mode.abs() == 5 {
      *result =
        windows::Win32::System::Memory::GlobalAlloc(windows::Win32::System::Memory::GMEM_FIXED, 32)
          .map_or(std::ptr::null_mut(), |value| value.0);
      return if mode < 0 {
        HRESULT(0x80004005u32 as i32)
      } else {
        HRESULT(0)
      };
    }
    audio_add_ref(this);
    *result = this;
  }
  if mode == 1 {
    HRESULT(0x80004005u32 as i32)
  } else {
    HRESULT(0)
  }
}

static AUDIO_VTABLE: GeneratedAudioClientVtbl = GeneratedAudioClientVtbl {
  base__: IUnknown_Vtbl {
    QueryInterface: audio_query_interface,
    AddRef: audio_add_ref,
    Release: audio_release,
  },
  initialize: audio_initialize,
  get_buffer_size: audio_get_u32,
  get_stream_latency: audio_get_i64,
  get_current_padding: audio_get_u32,
  is_format_supported: audio_is_format_supported,
  get_mix_format: audio_get_mix_format,
  get_device_period: audio_get_device_period,
  start: audio_no_args,
  stop: audio_no_args,
  reset: audio_no_args,
  set_event_handle: audio_set_event_handle,
  get_service: audio_get_service,
};

#[napi(object)]
pub struct GeneratedAudioClientStats {
  pub is_format_supported_calls: u32,
  pub get_service_calls: u32,
  pub last_share_mode: i32,
  pub last_format_tag: u32,
  pub add_ref_calls: u32,
  pub release_calls: u32,
  pub current_ref_count: u32,
}

#[napi]
pub fn create_generated_audio_client_fake() -> napi::Result<DynWinRTValue> {
  AUDIO_IS_FORMAT_SUPPORTED_CALLS.store(0, Ordering::SeqCst);
  AUDIO_GET_SERVICE_CALLS.store(0, Ordering::SeqCst);
  AUDIO_LAST_SHARE_MODE.store(0, Ordering::SeqCst);
  AUDIO_LAST_FORMAT_TAG.store(0, Ordering::SeqCst);
  AUDIO_GET_SERVICE_MODE.store(0, Ordering::SeqCst);
  AUDIO_ADD_REF_CALLS.store(0, Ordering::SeqCst);
  AUDIO_RELEASE_CALLS.store(0, Ordering::SeqCst);
  AUDIO_CURRENT_REF_COUNT.store(1, Ordering::SeqCst);
  let object = Box::new(GeneratedAudioClientFake {
    vtable: &AUDIO_VTABLE,
    references: AtomicU32::new(1),
  });
  let unknown = unsafe { IUnknown::from_raw(Box::into_raw(object).cast()) };
  crate::com::apartment_bound_com_object(unknown)
}

#[napi]
pub fn set_generated_audio_client_get_service_mode(mode: i32) -> napi::Result<()> {
  if !(-5..=5).contains(&mode) {
    return Err(napi::Error::from_reason(
      "generated audio GetService mode must be between -5 and 5",
    ));
  }
  AUDIO_GET_SERVICE_MODE.store(mode, Ordering::SeqCst);
  Ok(())
}

#[napi]
pub fn generated_audio_client_stats() -> GeneratedAudioClientStats {
  GeneratedAudioClientStats {
    is_format_supported_calls: AUDIO_IS_FORMAT_SUPPORTED_CALLS.load(Ordering::SeqCst),
    get_service_calls: AUDIO_GET_SERVICE_CALLS.load(Ordering::SeqCst),
    last_share_mode: AUDIO_LAST_SHARE_MODE.load(Ordering::SeqCst),
    last_format_tag: AUDIO_LAST_FORMAT_TAG.load(Ordering::SeqCst),
    add_ref_calls: AUDIO_ADD_REF_CALLS.load(Ordering::SeqCst),
    release_calls: AUDIO_RELEASE_CALLS.load(Ordering::SeqCst),
    current_ref_count: AUDIO_CURRENT_REF_COUNT.load(Ordering::SeqCst),
  }
}

#[napi]
pub fn create_generated_native_cleanup_resource(kind: u32) -> napi::Result<BigInt> {
  let address = match kind {
    1 => unsafe {
      windows::Win32::System::Threading::CreateEventW(None, true, false, None)
        .map(|value| value.0 as usize)
    },
    2 => {
      let and_mask = [0xffu8; 32];
      let xor_mask = [0u8; 32];
      unsafe {
        windows::Win32::UI::WindowsAndMessaging::CreateIcon(
          None,
          16,
          16,
          1,
          1,
          and_mask.as_ptr(),
          xor_mask.as_ptr(),
        )
        .map(|value| value.0 as usize)
      }
    }
    3 => {
      let brush = unsafe {
        windows::Win32::Graphics::Gdi::CreateSolidBrush(windows::Win32::Foundation::COLORREF(
          0x0000ff,
        ))
      };
      if brush.is_invalid() {
        Err(windows::core::Error::from_thread())
      } else {
        Ok(brush.0 as usize)
      }
    }
    _ => {
      return Err(napi::Error::from_reason(
        "generated cleanup resource kind must be 1, 2, or 3",
      ));
    }
  }
  .map_err(|error| napi::Error::from_reason(error.message()))?;
  Ok(BigInt::from(address as u64))
}
