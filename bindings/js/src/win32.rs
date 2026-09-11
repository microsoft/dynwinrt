// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Win32-only N-API carriers. Native type tags, not JS prototypes/properties,
//! establish handle, resource and plan identity.

use std::{
  ffi::c_void,
  ptr,
  sync::{Arc, OnceLock},
};

use dynwinrt::win32::{
  CallPlan, Handle, Output, Parameter, Resource, ReturnKind, Specification, Value,
};
use napi::{
  bindgen_prelude::{BigInt, Buffer, Either, FromNapiValue, ToNapiValue, Unknown},
  sys, Env, JsValue,
};
use napi_derive::napi;

#[repr(C)]
struct TypeTag {
  lower: u64,
  upper: u64,
}

const CARRIER_TAG: TypeTag = TypeTag {
  lower: 0x8485_314d_f6a9_0e67,
  upper: 0x397a_626b_dd58_45cb,
};

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
      // napi-sys does not expose the N-API 8 type-tag functions. Resolve those
      // exact host exports; do not weaken identity to spoofable instanceof.
      use windows::core::PCSTR;
      use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
      let host = unsafe { GetModuleHandleW(None) }.map_err(|e| e.to_string())?;
      let tag = unsafe { GetProcAddress(host, PCSTR(c"napi_type_tag_object".as_ptr().cast())) }
        .ok_or("win32.carrier: host must provide N-API 8 type tags")?;
      let check =
        unsafe { GetProcAddress(host, PCSTR(c"napi_check_object_type_tag".as_ptr().cast())) }
          .ok_or("win32.carrier: host must provide N-API 8 type tags")?;
      Ok(TagApi {
        tag: unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, TagObject>(tag) },
        check: unsafe {
          std::mem::transmute::<unsafe extern "system" fn() -> isize, CheckTag>(check)
        },
      })
    })
    .as_ref()
    .map_err(|reason| error(reason.clone()))
}

enum Carrier {
  Handle(Handle),
  Resource(Resource),
  Plan(Arc<CallPlan>),
}

fn error(reason: impl Into<String>) -> napi::Error {
  napi::Error::from_reason(reason.into())
}

unsafe extern "C" fn finalize(_: sys::napi_env, data: *mut c_void, _: *mut c_void) {
  drop(unsafe { Box::from_raw(data.cast::<Carrier>()) });
}

fn wrap(env: Env, carrier: Carrier) -> napi::Result<Unknown<'static>> {
  let tags = tags()?;
  let mut object = ptr::null_mut();
  napi::check_status!(unsafe { sys::napi_create_object(env.raw(), &mut object) })?;
  let pointer = Box::into_raw(Box::new(carrier));
  let status = unsafe {
    sys::napi_wrap(
      env.raw(),
      object,
      pointer.cast(),
      Some(finalize),
      ptr::null_mut(),
      ptr::null_mut(),
    )
  };
  if status != sys::Status::napi_ok {
    drop(unsafe { Box::from_raw(pointer) });
    return Err(error("win32.carrier: cannot register native finalizer"));
  }
  // After wrap succeeds the finalizer owns the carrier, including if tagging
  // fails. No resource is ever published without a registered finalizer.
  napi::check_status!(unsafe { (tags.tag)(env.raw(), object, &CARRIER_TAG) })?;
  unsafe { Unknown::from_napi_value(env.raw(), object) }
}

fn carrier<'a>(value: &'a Unknown<'_>) -> napi::Result<&'a Carrier> {
  let env = value.value().env;
  let raw = value.value().value;
  if value.get_type()? != napi::ValueType::Object {
    return Err(error("win32.value: expected a native Win32 carrier"));
  }
  let mut tagged = false;
  napi::check_status!(unsafe { (tags()?.check)(env, raw, &CARRIER_TAG, &mut tagged) })?;
  if !tagged {
    return Err(error("win32.value: expected a native Win32 carrier"));
  }
  let mut pointer = ptr::null_mut();
  napi::check_status!(unsafe { sys::napi_unwrap(env, raw, &mut pointer) })?;
  if pointer.is_null() {
    return Err(error("win32.value: missing native Win32 state"));
  }
  Ok(unsafe { &*pointer.cast::<Carrier>() })
}

/// Internal facade implementation; never exported from the WinRT root.
#[napi]
pub fn win32_hkey(env: Env, bits: BigInt) -> napi::Result<Unknown<'static>> {
  let (negative, unsigned, lossless) = bits.get_u64();
  let bits = if negative {
    let (signed, lossless) = bits.get_i64();
    if !lossless || signed as isize as i64 != signed {
      return Err(error(
        "win32.handle: HKEY must fit the native pointer width",
      ));
    }
    signed as isize as usize
  } else {
    if !lossless || unsigned as usize as u64 != unsigned {
      return Err(error(
        "win32.handle: HKEY must fit the native pointer width",
      ));
    }
    unsigned as usize
  };
  wrap(env, Carrier::Handle(Handle::hkey(bits).map_err(error)?))
}

#[napi]
pub fn win32_close(resource: Unknown<'_>) -> napi::Result<u32> {
  let Carrier::Resource(resource) = carrier(&resource)? else {
    return Err(error("win32.value: close requires a managed HKEY resource"));
  };
  resource.close().map_err(error)
}

#[napi]
pub fn win32_closed(resource: Unknown<'_>) -> napi::Result<bool> {
  let Carrier::Resource(resource) = carrier(&resource)? else {
    return Err(error("win32.value: expected a managed HKEY resource"));
  };
  Ok(resource.closed())
}

/// Manual descriptors are an unsafe capability, not a safe registry extension.
#[napi]
pub fn win32_bind(env: Env, descriptor: String) -> napi::Result<Unknown<'static>> {
  if descriptor.len() > 64 * 1024 {
    return Err(error("win32.plan: descriptor exceeds the 64 KiB limit"));
  }
  let specification: Specification = serde_json::from_str(&descriptor)
    .map_err(|e| error(format!("win32.plan: invalid descriptor: {e}")))?;
  let plan = unsafe { CallPlan::bind(specification) }.map_err(error)?;
  wrap(env, Carrier::Plan(Arc::new(plan)))
}

fn buffer_storage(value: &Unknown<'_>) -> napi::Result<Vec<u8>> {
  let env = value.value().env;
  let raw = value.value().value;
  let mut is_typed_array = false;
  napi::check_status!(unsafe { sys::napi_is_typedarray(env, raw, &mut is_typed_array) })?;
  if !is_typed_array {
    return Err(error("win32.value: expected Buffer or Uint8Array"));
  }
  let mut kind = 0;
  let mut length = 0;
  let mut data = ptr::null_mut();
  let mut backing = ptr::null_mut();
  let mut offset = 0;
  napi::check_status!(unsafe {
    sys::napi_get_typedarray_info(
      env,
      raw,
      &mut kind,
      &mut length,
      &mut data,
      &mut backing,
      &mut offset,
    )
  })?;
  if kind != sys::TypedarrayType::uint8_array as i32 {
    return Err(error("win32.value: expected an unsigned byte buffer"));
  }
  let mut is_array_buffer = false;
  napi::check_status!(unsafe { sys::napi_is_arraybuffer(env, backing, &mut is_array_buffer) })?;
  if !is_array_buffer {
    return Err(error(
      "win32.value: SharedArrayBuffer storage is unsupported",
    ));
  }
  let mut detached = false;
  napi::check_status!(unsafe { sys::napi_is_detached_arraybuffer(env, backing, &mut detached) })?;
  if detached {
    return Err(error("win32.value: detached ArrayBuffer"));
  }
  let mut backing_data = ptr::null_mut();
  let mut backing_length = 0;
  napi::check_status!(unsafe {
    sys::napi_get_arraybuffer_info(env, backing, &mut backing_data, &mut backing_length)
  })?;
  if length > dynwinrt::win32::MAX_BUFFER_BYTES
    || offset
      .checked_add(length)
      .is_none_or(|end| end > backing_length)
    || (length != 0 && data.is_null())
  {
    return Err(error(
      "win32.value: invalid or excessive native buffer capacity",
    ));
  }
  // Only capacity is used for these output-only contracts; no JS .length,
  // .byteLength, getters, pointers or shared storage survive into libffi.
  let mut bytes = Vec::new();
  bytes
    .try_reserve_exact(length)
    .map_err(|_| error("win32.value: unable to allocate byte storage"))?;
  bytes.resize(length, 0);
  Ok(bytes)
}

fn utf16_text(value: &Unknown<'_>) -> napi::Result<String> {
  let env = value.value().env;
  let raw = value.value().value;
  let mut length = 0;
  napi::check_status!(unsafe {
    sys::napi_get_value_string_utf16(env, raw, ptr::null_mut(), 0, &mut length)
  })?;
  if length > dynwinrt::win32::MAX_BUFFER_BYTES / 2 {
    return Err(error(
      "win32.value: UTF-16 input exceeds the bounded allocation limit",
    ));
  }
  let mut units = Vec::new();
  units
    .try_reserve_exact(length + 1)
    .map_err(|_| error("win32.value: unable to allocate UTF-16 storage"))?;
  units.resize(length + 1, 0);
  let mut written = 0;
  napi::check_status!(unsafe {
    sys::napi_get_value_string_utf16(env, raw, units.as_mut_ptr(), units.len(), &mut written)
  })?;
  String::from_utf16(&units[..written])
    .map_err(|_| error("win32.value: input contains ill-formed UTF-16"))
}

fn input(parameter: &Parameter, value: &Unknown<'_>) -> napi::Result<Value> {
  let env = value.value().env;
  let raw = value.value().value;
  let typ = value.get_type()?;
  match parameter {
    Parameter::U32 {} => {
      if typ != napi::ValueType::Number {
        return Err(error("win32.value: expected a u32 number"));
      }
      let number = unsafe { f64::from_napi_value(env, raw) }?;
      if !number.is_finite() || number.fract() != 0.0 || !(0.0..=u32::MAX as f64).contains(&number)
      {
        return Err(error("win32.value: expected a u32 integer"));
      }
      Ok(Value::U32(number as u32))
    }
    Parameter::Utf16 { nullable: true } if typ == napi::ValueType::Null => Ok(Value::Utf16(None)),
    Parameter::Utf16 { .. } if typ == napi::ValueType::String => {
      Ok(Value::Utf16(Some(utf16_text(value)?)))
    }
    Parameter::BorrowHkey { .. } => match carrier(value)? {
      Carrier::Handle(handle) => Ok(Value::Hkey(*handle)),
      Carrier::Resource(resource) => Ok(Value::Resource(resource.clone())),
      Carrier::Plan(_) => Err(error("win32.value: expected a typed HKEY, not a call plan")),
    },
    Parameter::ConsumeHkey {} => match carrier(value)? {
      Carrier::Resource(resource) => Ok(Value::Resource(resource.clone())),
      Carrier::Handle(_) | Carrier::Plan(_) => Err(error(
        "win32.value: consuming calls require a managed HKEY resource",
      )),
    },
    Parameter::Bytes { .. } if typ == napi::ValueType::Null => Ok(Value::Bytes(None)),
    Parameter::Bytes { .. } => Ok(Value::Bytes(Some(buffer_storage(value)?))),
    Parameter::Utf16 { .. } => Err(error(
      "win32.value: expected UTF-16 string or permitted null",
    )),
    Parameter::OwnHkey { .. }
    | Parameter::ReservedNull {}
    | Parameter::OutU32 {}
    | Parameter::ByteCount { .. } => Err(error(
      "win32.value: hidden output/reserved parameter cannot be supplied",
    )),
  }
}

#[napi(object)]
pub struct Win32NativeResult {
  pub value: Either<f64, BigInt>,
  pub outputs: Vec<Unknown<'static>>,
  pub kinds: Vec<String>,
}

#[napi]
pub fn win32_invoke(
  env: Env,
  plan: Unknown<'_>,
  args: Vec<Unknown<'_>>,
) -> napi::Result<Win32NativeResult> {
  let Carrier::Plan(plan) = carrier(&plan)? else {
    return Err(error("win32.value: expected an immutable Win32 call plan"));
  };
  let parameters = plan
    .specification()
    .parameters
    .iter()
    .filter(|p| p.is_input())
    .collect::<Vec<_>>();
  if args.len() != parameters.len() {
    return Err(error("win32.value: argument count does not match the plan"));
  }
  let values = parameters
    .iter()
    .zip(&args)
    .map(|(p, value)| input(p, value))
    .collect::<napi::Result<Vec<_>>>()?;
  let result = plan.invoke(&values).map_err(error)?;
  let mut outputs = Vec::new();
  let mut kinds = Vec::new();
  for value in result.outputs {
    let (value, kind) = match value {
      Output::Resource(resource) => (wrap(env, Carrier::Resource(resource))?, "resource"),
      Output::Hkey(handle) => (wrap(env, Carrier::Handle(handle))?, "handle"),
      Output::U32(number) => (
        unsafe { Unknown::from_napi_value(env.raw(), u32::to_napi_value(env.raw(), number)?)? },
        "value",
      ),
      Output::Bytes(bytes) => (
        unsafe {
          Unknown::from_napi_value(
            env.raw(),
            Buffer::to_napi_value(env.raw(), Buffer::from(bytes))?,
          )?
        },
        "value",
      ),
      Output::Null => {
        let mut raw = ptr::null_mut();
        napi::check_status!(unsafe { sys::napi_get_null(env.raw(), &mut raw) })?;
        (
          unsafe { Unknown::from_napi_value(env.raw(), raw) }?,
          "value",
        )
      }
    };
    outputs.push(value);
    kinds.push(kind.into());
  }
  Ok(Win32NativeResult {
    value: if plan.specification().returns == ReturnKind::U64 {
      Either::B(BigInt::from(result.value))
    } else {
      Either::A(result.value as f64)
    },
    outputs,
    kinds,
  })
}
