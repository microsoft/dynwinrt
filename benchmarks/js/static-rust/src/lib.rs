// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Raw N-API Rust static benchmark — minimal overhead, no napi-rs macros.
//! Matches the C++ static-bench-cpp addon 1:1.

use std::ffi::c_void;
use std::ptr;

use napi_sys::*;
use windows::core::HSTRING;
use windows::Foundation::{PropertyValue, Uri};
use windows_core::{IUnknown, Interface};

// ======================================================================
// Helpers
// ======================================================================

/// Store a COM object (as IUnknown raw ptr) in a napi_external.
/// Release callback handles dropping.
unsafe fn wrap_obj<T: Interface>(env: napi_env, obj: T) -> napi_value {
  let raw = obj.into_raw(); // transfers ownership (+1 ref)
  let mut result: napi_value = ptr::null_mut();
  unsafe extern "C" fn release(_env: napi_env, data: *mut c_void, _hint: *mut c_void) {
    if !data.is_null() {
      let unk = IUnknown::from_raw(data);
      drop(unk); // Release
    }
  }
  napi_create_external(env, raw, Some(release), ptr::null_mut(), &mut result);
  result
}

/// Recover typed COM object from napi_external.
/// Equivalent to C++ copy_from_abi: AddRef + reinterpret, no QI.
unsafe fn unwrap_obj<T: Interface>(env: napi_env, val: napi_value) -> T {
  let mut data: *mut c_void = ptr::null_mut();
  napi_get_value_external(env, val, &mut data);
  // AddRef via IUnknown, then reinterpret as T (no QI).
  // All COM interfaces are repr(transparent) over a single pointer,
  // so this is equivalent to C++ copy_from_abi.
  let unk = IUnknown::from_raw(data);
  let addrefed = unk.clone(); // AddRef
  std::mem::forget(unk); // External owns the original ref
  let result = std::mem::transmute_copy::<IUnknown, T>(&addrefed);
  std::mem::forget(addrefed); // don't double-Release
  result
}

/// Get JS string arg as HSTRING (UTF-16).
unsafe fn get_hstring(env: napi_env, val: napi_value) -> HSTRING {
  let mut len = 0usize;
  napi_get_value_string_utf16(env, val, ptr::null_mut(), 0, &mut len);
  let mut buf = vec![0u16; len + 1];
  let mut copied = 0usize;
  napi_get_value_string_utf16(env, val, buf.as_mut_ptr(), buf.len(), &mut copied);
  HSTRING::from_wide(&buf[..copied])
}

/// Create JS string from HSTRING.
unsafe fn from_hstring(env: napi_env, hs: &HSTRING) -> napi_value {
  let wide: &[u16] = unsafe { std::slice::from_raw_parts(hs.as_ptr(), hs.len()) };
  let mut result: napi_value = ptr::null_mut();
  napi_create_string_utf16(env, wide.as_ptr(), wide.len(), &mut result);
  result
}

/// Create JS number from i32.
unsafe fn from_i32(env: napi_env, v: i32) -> napi_value {
  let mut result: napi_value = ptr::null_mut();
  napi_create_int32(env, v, &mut result);
  result
}

/// Create JS boolean.
unsafe fn from_bool(env: napi_env, v: bool) -> napi_value {
  let mut result: napi_value = ptr::null_mut();
  napi_get_boolean(env, v, &mut result);
  result
}

/// Get i32 from JS number.
unsafe fn get_i32(env: napi_env, val: napi_value) -> i32 {
  let mut v = 0i32;
  napi_get_value_int32(env, val, &mut v);
  v
}

/// Get f64 from JS number.
unsafe fn get_f64(env: napi_env, val: napi_value) -> f64 {
  let mut v = 0f64;
  napi_get_value_double(env, val, &mut v);
  v
}

/// Get bool from JS value.
unsafe fn get_bool(env: napi_env, val: napi_value) -> bool {
  let mut v = false;
  napi_get_value_bool(env, val, &mut v);
  v
}

/// Get arg[index] from CallbackInfo.
unsafe fn get_arg(env: napi_env, info: napi_callback_info, index: usize) -> napi_value {
  let count = index + 1;
  let mut args = vec![ptr::null_mut(); count];
  let mut argc = count;
  napi_get_cb_info(
    env,
    info,
    &mut argc,
    args.as_mut_ptr(),
    ptr::null_mut(),
    ptr::null_mut(),
  );
  args[index]
}

/// Get multiple args.
unsafe fn get_args(env: napi_env, info: napi_callback_info, count: usize) -> Vec<napi_value> {
  let mut args = vec![ptr::null_mut(); count];
  let mut argc = count;
  napi_get_cb_info(
    env,
    info,
    &mut argc,
    args.as_mut_ptr(),
    ptr::null_mut(),
    ptr::null_mut(),
  );
  args
}

// ======================================================================
// Uri functions
// ======================================================================

unsafe extern "C" fn uri_create(env: napi_env, info: napi_callback_info) -> napi_value {
  let url = get_hstring(env, get_arg(env, info, 0));
  let uri = Uri::CreateUri(&url).unwrap();
  wrap_obj(env, uri)
}

unsafe extern "C" fn uri_get_host(env: napi_env, info: napi_callback_info) -> napi_value {
  let url = get_hstring(env, get_arg(env, info, 0));
  let uri = Uri::CreateUri(&url).unwrap();
  from_hstring(env, &uri.Host().unwrap())
}

unsafe extern "C" fn uri_host_from_obj(env: napi_env, info: napi_callback_info) -> napi_value {
  let uri: Uri = unwrap_obj(env, get_arg(env, info, 0));
  from_hstring(env, &uri.Host().unwrap())
}

unsafe extern "C" fn uri_port_from_obj(env: napi_env, info: napi_callback_info) -> napi_value {
  let uri: Uri = unwrap_obj(env, get_arg(env, info, 0));
  from_i32(env, uri.Port().unwrap())
}

unsafe extern "C" fn uri_suspicious_from_obj(
  env: napi_env,
  info: napi_callback_info,
) -> napi_value {
  let uri: Uri = unwrap_obj(env, get_arg(env, info, 0));
  from_bool(env, uri.Suspicious().unwrap())
}

unsafe extern "C" fn uri_query_parsed_from_obj(
  env: napi_env,
  info: napi_callback_info,
) -> napi_value {
  let uri: Uri = unwrap_obj(env, get_arg(env, info, 0));
  let parsed = uri.QueryParsed().unwrap();
  wrap_obj(env, parsed)
}

unsafe extern "C" fn uri_combine(env: napi_env, info: napi_callback_info) -> napi_value {
  let args = get_args(env, info, 2);
  let uri: Uri = unwrap_obj(env, args[0]);
  let relative = get_hstring(env, args[1]);
  let result = uri.CombineUri(&relative).unwrap();
  wrap_obj(env, result)
}

unsafe extern "C" fn uri_create_with_relative(
  env: napi_env,
  info: napi_callback_info,
) -> napi_value {
  let args = get_args(env, info, 2);
  let base = get_hstring(env, args[0]);
  let relative = get_hstring(env, args[1]);
  let uri = Uri::CreateWithRelativeUri(&base, &relative).unwrap();
  wrap_obj(env, uri)
}

// ======================================================================
// PropertyValue functions
// ======================================================================

unsafe extern "C" fn pv_create_i32(env: napi_env, info: napi_callback_info) -> napi_value {
  let v = get_i32(env, get_arg(env, info, 0));
  wrap_obj(env, PropertyValue::CreateInt32(v).unwrap())
}

unsafe extern "C" fn pv_create_f64(env: napi_env, info: napi_callback_info) -> napi_value {
  let v = get_f64(env, get_arg(env, info, 0));
  wrap_obj(env, PropertyValue::CreateDouble(v).unwrap())
}

unsafe extern "C" fn pv_create_bool(env: napi_env, info: napi_callback_info) -> napi_value {
  let v = get_bool(env, get_arg(env, info, 0));
  wrap_obj(env, PropertyValue::CreateBoolean(v).unwrap())
}

unsafe extern "C" fn pv_create_string(env: napi_env, info: napi_callback_info) -> napi_value {
  let hs = get_hstring(env, get_arg(env, info, 0));
  wrap_obj(env, PropertyValue::CreateString(&hs).unwrap())
}

// ======================================================================
// Geopoint
// ======================================================================

unsafe extern "C" fn geopoint_create(env: napi_env, info: napi_callback_info) -> napi_value {
  use windows::Devices::Geolocation::{BasicGeoposition, Geopoint};
  let args = get_args(env, info, 3);
  let pos = BasicGeoposition {
    Latitude: get_f64(env, args[0]),
    Longitude: get_f64(env, args[1]),
    Altitude: get_f64(env, args[2]),
  };
  wrap_obj(env, Geopoint::Create(pos).unwrap())
}

// ======================================================================
// Dynamic (dynwinrt) via raw N-API — the theoretical minimum
// ======================================================================
//
// Uses dynwinrt's zero-alloc getter path + raw napi-sys.
// No napi-rs macro overhead, no Vec, no WinRTValue wrapping.

use std::sync::{Arc, LazyLock};

static DYN_TABLE: LazyLock<Arc<dynwinrt::MetadataTable>> =
  LazyLock::new(|| dynwinrt::MetadataTable::new());

/// Store pre-built MethodHandle + pre-cast COM pointer in an External.
struct DynGetter {
  method: dynwinrt::MethodHandle,
  obj_raw: *mut c_void, // pre-QI'd interface pointer, caller keeps alive
}
unsafe impl Send for DynGetter {}
unsafe impl Sync for DynGetter {}

/// dynSetupUriGetters(uriIidStr) → External containing { getHost, getPort } handles
/// Call once to register interface and create method handles.
unsafe extern "C" fn dyn_setup_uri_getters(env: napi_env, info: napi_callback_info) -> napi_value {
  let iid_str = get_hstring(env, get_arg(env, info, 0));
  let uri_iid = windows::core::GUID::try_from(iid_str.to_string().as_str()).unwrap();

  // Register IUriRuntimeClass with all methods in correct vtable order
  let iuri = DYN_TABLE
    .register_interface("IUriRuntimeClass_raw", uri_iid)
    .add_method(
      "get_AbsoluteUri",
      dynwinrt::MethodSignature::new(&DYN_TABLE).add_out(DYN_TABLE.hstring()),
    )
    .add_method(
      "get_DisplayUri",
      dynwinrt::MethodSignature::new(&DYN_TABLE).add_out(DYN_TABLE.hstring()),
    )
    .add_method(
      "get_Domain",
      dynwinrt::MethodSignature::new(&DYN_TABLE).add_out(DYN_TABLE.hstring()),
    )
    .add_method(
      "get_Extension",
      dynwinrt::MethodSignature::new(&DYN_TABLE).add_out(DYN_TABLE.hstring()),
    )
    .add_method(
      "get_Fragment",
      dynwinrt::MethodSignature::new(&DYN_TABLE).add_out(DYN_TABLE.hstring()),
    )
    .add_method(
      "get_Host",
      dynwinrt::MethodSignature::new(&DYN_TABLE).add_out(DYN_TABLE.hstring()),
    )
    .add_method(
      "get_Password",
      dynwinrt::MethodSignature::new(&DYN_TABLE).add_out(DYN_TABLE.hstring()),
    )
    .add_method(
      "get_Path",
      dynwinrt::MethodSignature::new(&DYN_TABLE).add_out(DYN_TABLE.hstring()),
    )
    .add_method(
      "get_Query",
      dynwinrt::MethodSignature::new(&DYN_TABLE).add_out(DYN_TABLE.hstring()),
    )
    .add_method(
      "get_QueryParsed",
      dynwinrt::MethodSignature::new(&DYN_TABLE).add_out(DYN_TABLE.object()),
    )
    .add_method(
      "get_RawUri",
      dynwinrt::MethodSignature::new(&DYN_TABLE).add_out(DYN_TABLE.hstring()),
    )
    .add_method(
      "get_SchemeName",
      dynwinrt::MethodSignature::new(&DYN_TABLE).add_out(DYN_TABLE.hstring()),
    )
    .add_method(
      "get_UserName",
      dynwinrt::MethodSignature::new(&DYN_TABLE).add_out(DYN_TABLE.hstring()),
    )
    .add_method(
      "get_Port",
      dynwinrt::MethodSignature::new(&DYN_TABLE).add_out(DYN_TABLE.i32_type()),
    )
    .add_method(
      "get_Suspicious",
      dynwinrt::MethodSignature::new(&DYN_TABLE).add_out(DYN_TABLE.bool_type()),
    );

  let m_host = iuri.method_by_name("get_Host").unwrap();
  let m_port = iuri.method_by_name("get_Port").unwrap();

  // Create Uri and QI to IUriRuntimeClass
  let uri = Uri::CreateUri(&windows::core::HSTRING::from(
    "https://example.com:8080/path?q=1",
  ))
  .unwrap();
  let mut uri_ptr = ptr::null_mut();
  let unk: IUnknown = uri.cast().unwrap();
  unk.query(&uri_iid, &mut uri_ptr).ok().unwrap();
  // uri_ptr is now AddRef'd IUriRuntimeClass pointer.
  // We leak it (never Release) since it lives for the process lifetime.

  // Return an object with getHost/getPort External handles
  let mut result: napi_value = ptr::null_mut();
  napi_create_object(env, &mut result);

  // getHost External
  let host_getter = Box::into_raw(Box::new(DynGetter {
    method: m_host,
    obj_raw: uri_ptr,
  }));
  let mut ext: napi_value = ptr::null_mut();
  napi_create_external(
    env,
    host_getter as *mut c_void,
    None,
    ptr::null_mut(),
    &mut ext,
  );
  let key = std::ffi::CString::new("getHost").unwrap();
  napi_set_named_property(env, result, key.as_ptr(), ext);

  // getPort External
  let port_getter = Box::into_raw(Box::new(DynGetter {
    method: m_port,
    obj_raw: uri_ptr,
  }));
  napi_create_external(
    env,
    port_getter as *mut c_void,
    None,
    ptr::null_mut(),
    &mut ext,
  );
  let key = std::ffi::CString::new("getPort").unwrap();
  napi_set_named_property(env, result, key.as_ptr(), ext);

  result
}

/// dynGetString(getterExternal) → JS string
unsafe extern "C" fn dyn_get_string(env: napi_env, info: napi_callback_info) -> napi_value {
  let ext = get_arg(env, info, 0);
  let mut data: *mut c_void = ptr::null_mut();
  napi_get_value_external(env, ext, &mut data);
  let getter = &*(data as *const DynGetter);

  match getter.method.call_getter_hstring(getter.obj_raw) {
    Ok(hs) => {
      let wide: &[u16] = std::slice::from_raw_parts(hs.as_ptr(), hs.len());
      let mut result: napi_value = ptr::null_mut();
      napi_create_string_utf16(env, wide.as_ptr(), wide.len(), &mut result);
      result
    }
    Err(_) => ptr::null_mut(),
  }
}

/// dynGetI32(getterExternal) → JS number
unsafe extern "C" fn dyn_get_i32(env: napi_env, info: napi_callback_info) -> napi_value {
  let ext = get_arg(env, info, 0);
  let mut data: *mut c_void = ptr::null_mut();
  napi_get_value_external(env, ext, &mut data);
  let getter = &*(data as *const DynGetter);

  match getter.method.call_getter_i32(getter.obj_raw) {
    Ok(v) => {
      let mut result: napi_value = ptr::null_mut();
      napi_create_int32(env, v, &mut result);
      result
    }
    Err(_) => ptr::null_mut(),
  }
}

// ======================================================================
// Module init
// ======================================================================

unsafe fn register_fn(env: napi_env, exports: napi_value, name: &str, cb: napi_callback) {
  let mut func: napi_value = ptr::null_mut();
  let cname = std::ffi::CString::new(name).unwrap();
  napi_create_function(
    env,
    cname.as_ptr(),
    name.len(),
    cb,
    ptr::null_mut(),
    &mut func,
  );
  napi_set_named_property(env, exports, cname.as_ptr(), func);
}

#[no_mangle]
unsafe extern "C" fn napi_register_module_v1(env: napi_env, exports: napi_value) -> napi_value {
  // Initialize napi-sys dynamic symbol loading
  napi_sys::setup();

  register_fn(env, exports, "uriCreate", Some(uri_create));
  register_fn(env, exports, "uriGetHost", Some(uri_get_host));
  register_fn(env, exports, "uriHostFromObj", Some(uri_host_from_obj));
  register_fn(env, exports, "uriPortFromObj", Some(uri_port_from_obj));
  register_fn(
    env,
    exports,
    "uriSuspiciousFromObj",
    Some(uri_suspicious_from_obj),
  );
  register_fn(
    env,
    exports,
    "uriQueryParsedFromObj",
    Some(uri_query_parsed_from_obj),
  );
  register_fn(env, exports, "uriCombine", Some(uri_combine));
  register_fn(
    env,
    exports,
    "uriCreateWithRelative",
    Some(uri_create_with_relative),
  );
  register_fn(env, exports, "pvCreateI32", Some(pv_create_i32));
  register_fn(env, exports, "pvCreateF64", Some(pv_create_f64));
  register_fn(env, exports, "pvCreateBool", Some(pv_create_bool));
  register_fn(env, exports, "pvCreateString", Some(pv_create_string));
  register_fn(env, exports, "geopointCreate", Some(geopoint_create));
  register_fn(
    env,
    exports,
    "dynSetupUriGetters",
    Some(dyn_setup_uri_getters),
  );
  register_fn(env, exports, "dynGetString", Some(dyn_get_string));
  register_fn(env, exports, "dynGetI32", Some(dyn_get_i32));
  exports
}
