// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Every opaque Win32 carrier checks a native type tag before typed unwrapping.
//! napi-rs class casts only test JavaScript prototypes, which are mutable.

use std::{cell::RefCell, collections::HashMap, ffi::c_void, ptr, sync::OnceLock};

use napi::{
  bindgen_prelude::{BigInt, Buffer, FromNapiValue, Function, Unknown},
  sys, Env, JsValue,
};
use napi_derive::napi;

use super::{
  DynWin32CallResult, DynWin32Function, DynWin32NativeStruct, DynWin32OverlappedOperation,
  DynWin32Resource, DynWin32SubsystemContext, DynWin32Value, DynWinRTValue,
};

thread_local! {
  static COM_RECEIVER_GUARDS: RefCell<HashMap<usize, Vec<sys::napi_ref>>> = RefCell::new(HashMap::new());
}

#[napi(module_exports, skip_typescript)]
fn capture_com_receiver_guard(env: Env) -> napi::Result<()> {
  let constructor = napi::bindgen_prelude::get_class_constructor("DynWinRtValue\0")
    .ok_or_else(|| napi::Error::from_reason("Managed COM carrier constructor is unavailable"))?;
  let mut class = ptr::null_mut();
  napi::check_status!(unsafe {
    sys::napi_get_reference_value(env.raw(), constructor, &mut class)
  })?;
  let mut prototype = ptr::null_mut();
  let mut method = ptr::null_mut();
  napi::check_status!(unsafe {
    sys::napi_get_named_property(env.raw(), class, c"prototype".as_ptr(), &mut prototype)
  })?;
  napi::check_status!(unsafe {
    sys::napi_get_named_property(env.raw(), prototype, c"isNull".as_ptr(), &mut method)
  })?;
  let mut reference = ptr::null_mut();
  napi::check_status!(unsafe { sys::napi_create_reference(env.raw(), method, 1, &mut reference) })?;
  let key = env.raw() as usize;
  let result = (|| {
    if COM_RECEIVER_GUARDS.with(|guards| !guards.borrow().contains_key(&key)) {
      let _hook = env.add_env_cleanup_hook(key, |key| {
        if let Some(references) =
          COM_RECEIVER_GUARDS.with(|guards| guards.borrow_mut().remove(&key))
        {
          for reference in references {
            let _ = unsafe { sys::napi_delete_reference(key as sys::napi_env, reference) };
          }
        }
      })?;
    }
    COM_RECEIVER_GUARDS.with(|guards| {
      let mut guards = guards.borrow_mut();
      let references = guards.entry(key).or_default();
      references
        .try_reserve_exact(1)
        .map_err(|_| napi::Error::from_reason("Unable to retain native COM carrier validation"))?;
      references.push(reference);
      Ok(())
    })
  })();
  if result.is_err() {
    let _ = unsafe { sys::napi_delete_reference(env.raw(), reference) };
  }
  result
}

pub(super) fn managed_com_value<'a>(value: &'a Unknown) -> napi::Result<&'a DynWinRTValue> {
  let env = value.value().env;
  let raw = value.raw();
  // Node's original napi_define_class method enforces its native receiver
  // signature. Unlike instanceof, this rejects another wrapped Rust type even
  // after prototype spoofing. Retain the method before exports are published so
  // replacing the public prototype cannot replace this validation.
  let references = COM_RECEIVER_GUARDS
    .with(|guards| guards.borrow().get(&(env as usize)).cloned())
    .ok_or_else(|| napi::Error::from_reason("Native COM carrier validation is unavailable"))?;
  for reference in references {
    let mut method = ptr::null_mut();
    let mut result = ptr::null_mut();
    napi::check_status!(unsafe { sys::napi_get_reference_value(env, reference, &mut method) })?;
    let status = unsafe { sys::napi_call_function(env, raw, method, 0, ptr::null(), &mut result) };
    if status == sys::Status::napi_pending_exception {
      let mut exception = ptr::null_mut();
      napi::check_status!(unsafe { sys::napi_get_and_clear_last_exception(env, &mut exception) })?;
      continue;
    }
    napi::check_status!(status)?;
    let mut is_null = false;
    napi::check_status!(unsafe { sys::napi_get_value_bool(env, result, &mut is_null) })?;
    return unsafe { <&DynWinRTValue>::from_napi_value(env, raw) };
  }
  Err(napi::Error::from_reason(
    "DynWin32.comObject(): expected a native managed COM carrier",
  ))
}

#[repr(C)]
struct TypeTag {
  lower: u64,
  upper: u64,
}

type TagObject =
  unsafe extern "C" fn(sys::napi_env, sys::napi_value, *const TypeTag) -> sys::napi_status;
type CheckTag = unsafe extern "C" fn(
  sys::napi_env,
  sys::napi_value,
  *const TypeTag,
  *mut bool,
) -> sys::napi_status;

struct TagApi {
  tag: TagObject,
  check: CheckTag,
}

fn tags() -> napi::Result<&'static TagApi> {
  static API: OnceLock<Result<TagApi, String>> = OnceLock::new();
  API
    .get_or_init(|| {
      use windows::core::PCSTR;
      use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
      let host = unsafe { GetModuleHandleW(None) }.map_err(|error| error.to_string())?;
      let tag = unsafe { GetProcAddress(host, PCSTR(c"napi_type_tag_object".as_ptr().cast())) }
        .ok_or("Win32 carriers require N-API 8 type tags")?;
      let check =
        unsafe { GetProcAddress(host, PCSTR(c"napi_check_object_type_tag".as_ptr().cast())) }
          .ok_or("Win32 carriers require N-API 8 type tags")?;
      Ok(TagApi {
        tag: unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, TagObject>(tag) },
        check: unsafe {
          std::mem::transmute::<unsafe extern "system" fn() -> isize, CheckTag>(check)
        },
      })
    })
    .as_ref()
    .map_err(|reason| napi::Error::from_reason(reason.clone()))
}

fn tag(kind: u32) -> TypeTag {
  static MODULE_BRAND: u8 = 0;
  TypeTag {
    lower: 0x8485_314d_f6a9_0e67 ^ u64::from(kind),
    // Separate addon copies must not unwrap one another's Rust layouts/state.
    upper: 0x397a_626b_dd58_45cb ^ std::ptr::addr_of!(MODULE_BRAND) as usize as u64,
  }
}

pub(crate) trait Carrier: Sized + 'static {
  const KIND: u32;
  const NAME: &'static str;
}

unsafe extern "C" fn finalize<T>(_: sys::napi_env, data: *mut c_void, _: *mut c_void) {
  drop(unsafe { Box::from_raw(data.cast::<T>()) });
}

pub(crate) unsafe fn wrap<T: Carrier>(
  env: sys::napi_env,
  value: T,
) -> napi::Result<sys::napi_value> {
  let api = tags()?;
  let mut object = ptr::null_mut();
  napi::check_status!(unsafe { sys::napi_create_object(env, &mut object) })?;
  let mut value = Box::new(value);
  napi::check_status!(unsafe {
    sys::napi_wrap(
      env,
      object,
      (&mut *value as *mut T).cast(),
      Some(finalize::<T>),
      ptr::null_mut(),
      ptr::null_mut(),
    )
  })?;
  // The registered finalizer owns the value even if tagging or JS projection fails.
  let _ = Box::into_raw(value);
  napi::check_status!(unsafe { (api.tag)(env, object, &tag(T::KIND)) })?;
  Ok(object)
}

unsafe fn has_tag(env: sys::napi_env, raw: sys::napi_value, kind: u32) -> napi::Result<bool> {
  let mut typ = sys::ValueType::napi_undefined;
  napi::check_status!(unsafe { sys::napi_typeof(env, raw, &mut typ) })?;
  if typ != sys::ValueType::napi_object {
    return Ok(false);
  }
  let mut matches = false;
  napi::check_status!(unsafe { (tags()?.check)(env, raw, &tag(kind), &mut matches) })?;
  Ok(matches)
}

pub(crate) unsafe fn unwrap<T: Carrier>(
  env: sys::napi_env,
  raw: sys::napi_value,
) -> napi::Result<*mut T> {
  if !unsafe { has_tag(env, raw, T::KIND) }? {
    return Err(napi::Error::from_reason(format!(
      "Expected a native {} carrier",
      T::NAME
    )));
  }
  let mut pointer = ptr::null_mut();
  napi::check_status!(unsafe { sys::napi_unwrap(env, raw, &mut pointer) })?;
  if pointer.is_null() {
    return Err(napi::Error::from_reason(
      "Missing native Win32 carrier state",
    ));
  }
  Ok(pointer.cast())
}

macro_rules! carrier {
  ($typ:ty, $kind:expr, $name:literal) => {
    impl $crate::win32::boundary::Carrier for $typ {
      const KIND: u32 = $kind;
      const NAME: &'static str = $name;
    }
    impl napi::bindgen_prelude::TypeName for $typ {
      fn type_name() -> &'static str {
        $name
      }
      fn value_type() -> napi::ValueType {
        napi::ValueType::Object
      }
    }
    impl napi::bindgen_prelude::TypeName for &$typ {
      fn type_name() -> &'static str {
        $name
      }
      fn value_type() -> napi::ValueType {
        napi::ValueType::Object
      }
    }
    impl napi::bindgen_prelude::TypeName for &mut $typ {
      fn type_name() -> &'static str {
        $name
      }
      fn value_type() -> napi::ValueType {
        napi::ValueType::Object
      }
    }
    impl napi::bindgen_prelude::ToNapiValue for $typ {
      unsafe fn to_napi_value(
        env: napi::sys::napi_env,
        value: Self,
      ) -> napi::Result<napi::sys::napi_value> {
        unsafe { $crate::win32::boundary::wrap(env, value) }
      }
    }
    impl napi::bindgen_prelude::FromNapiRef for $typ {
      unsafe fn from_napi_ref(
        env: napi::sys::napi_env,
        value: napi::sys::napi_value,
      ) -> napi::Result<&'static Self> {
        Ok(unsafe { &*$crate::win32::boundary::unwrap::<Self>(env, value)? })
      }
    }
    impl napi::bindgen_prelude::FromNapiMutRef for $typ {
      unsafe fn from_napi_mut_ref(
        env: napi::sys::napi_env,
        value: napi::sys::napi_value,
      ) -> napi::Result<&'static mut Self> {
        Ok(unsafe { &mut *$crate::win32::boundary::unwrap::<Self>(env, value)? })
      }
    }
    impl napi::bindgen_prelude::ValidateNapiValue for &$typ {
      unsafe fn validate(
        env: napi::sys::napi_env,
        value: napi::sys::napi_value,
      ) -> napi::Result<napi::sys::napi_value> {
        unsafe { $crate::win32::boundary::unwrap::<$typ>(env, value)? };
        Ok(std::ptr::null_mut())
      }
    }
    impl napi::bindgen_prelude::ValidateNapiValue for &mut $typ {
      unsafe fn validate(
        env: napi::sys::napi_env,
        value: napi::sys::napi_value,
      ) -> napi::Result<napi::sys::napi_value> {
        unsafe { $crate::win32::boundary::unwrap::<$typ>(env, value)? };
        Ok(std::ptr::null_mut())
      }
    }
  };
}
pub(crate) use carrier;

#[napi]
pub fn win32_carrier_kind(value: Unknown) -> napi::Result<u32> {
  for kind in 1..=7 {
    if unsafe { has_tag(value.value().env, value.raw(), kind) }? {
      return Ok(kind);
    }
  }
  Ok(0)
}

pub(super) fn has_carrier(env: sys::napi_env, raw: sys::napi_value) -> napi::Result<bool> {
  for kind in 1..=7 {
    if unsafe { has_tag(env, raw, kind) }? {
      return Ok(true);
    }
  }
  Ok(false)
}

#[napi]
pub fn win32_native_struct_bytes(value: &DynWin32NativeStruct) -> napi::Result<Buffer> {
  value.bytes()
}

#[napi]
pub fn win32_native_struct_length(value: &DynWin32NativeStruct) -> u32 {
  value.length()
}

#[napi]
pub fn win32_resource_value(value: &DynWin32Resource) -> BigInt {
  value.value()
}

#[napi]
pub fn win32_resource_closed(value: &DynWin32Resource) -> bool {
  value.closed()
}

#[napi]
pub fn win32_resource_busy(value: &DynWin32Resource) -> bool {
  value.busy()
}

#[napi]
pub fn win32_resource_active(value: &DynWin32Resource) -> bool {
  value.active()
}

#[napi]
pub fn win32_resource_close(value: &DynWin32Resource) -> napi::Result<()> {
  value.close()
}

#[napi]
pub fn win32_bind(
  #[napi(ts_arg_type = "DynWin32FunctionSpec")] spec: Unknown,
) -> napi::Result<DynWin32Function> {
  DynWin32Function::bind(super::specification::function_spec(spec)?)
}

#[napi]
pub fn win32_function_dll(value: &DynWin32Function) -> String {
  value.dll()
}

#[napi]
pub fn win32_function_entry_point(value: &DynWin32Function) -> String {
  value.entry_point()
}

#[napi]
pub fn win32_invoke(
  value: &DynWin32Function,
  #[napi(ts_arg_type = "DynWin32Value[]")] args: Unknown,
) -> napi::Result<DynWin32CallResult> {
  value.invoke(super::specification::arguments(&args)?)
}

#[napi]
pub fn win32_invoke_with_subsystem(
  value: &DynWin32Function,
  context: &DynWin32SubsystemContext,
  #[napi(ts_arg_type = "string")] subsystem: super::specification::Name,
  #[napi(ts_arg_type = "DynWin32Value[]")] args: Unknown,
) -> napi::Result<DynWin32CallResult> {
  value.invoke_with_subsystem(
    context,
    subsystem.into_string(),
    super::specification::arguments(&args)?,
  )
}

#[napi]
pub fn win32_result_return_value(
  value: &mut DynWin32CallResult,
) -> napi::Result<Option<DynWin32Value>> {
  value.return_value()
}

#[napi]
pub fn win32_result_outputs(value: &mut DynWin32CallResult) -> napi::Result<Vec<DynWin32Value>> {
  value.outputs()
}

#[napi]
pub fn win32_result_last_error(value: &DynWin32CallResult) -> Option<u32> {
  value.last_error()
}

#[napi]
pub fn win32_result_succeeded(value: &DynWin32CallResult) -> bool {
  value.succeeded()
}

#[napi]
pub fn win32_subsystem_name(value: &DynWin32SubsystemContext) -> &'static str {
  value.subsystem()
}

#[napi]
pub fn win32_subsystem_closed(value: &DynWin32SubsystemContext) -> bool {
  value.closed()
}

#[napi]
pub fn win32_subsystem_close(value: &DynWin32SubsystemContext) -> napi::Result<()> {
  value.close()
}

#[napi]
pub fn win32_overlapped_cancel(value: &DynWin32OverlappedOperation) {
  value.cancel()
}

#[napi]
pub fn win32_overlapped_start(
  value: &mut DynWin32OverlappedOperation,
  #[napi(ts_arg_type = "(error: Error | null, bytesTransferred?: number) => void")]
  callback: Function<'static, (), ()>,
) -> napi::Result<()> {
  value.start(callback)
}
