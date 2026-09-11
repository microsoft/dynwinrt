// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{ffi::CStr, ops::Deref, ptr};

use napi::{
  bindgen_prelude::{FromNapiValue, TypeName, Unknown, ValidateNapiValue},
  sys, JsValue,
};

use super::{
  DynWin32FunctionSpec, DynWin32ParameterSpec, DynWin32Value,
  MAX_NATIVE_AGGREGATE_DESCRIPTOR_LENGTH,
};

pub struct Descriptor(String);

pub struct Name(String);

impl Name {
  pub fn into_string(self) -> String {
    self.0
  }
}

impl Deref for Name {
  type Target = String;
  fn deref(&self) -> &Self::Target {
    &self.0
  }
}

impl std::fmt::Display for Name {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    std::fmt::Display::fmt(&self.0, formatter)
  }
}

impl TypeName for Name {
  fn type_name() -> &'static str {
    "string"
  }
  fn value_type() -> napi::ValueType {
    napi::ValueType::String
  }
}

impl ValidateNapiValue for Name {}

impl FromNapiValue for Name {
  unsafe fn from_napi_value(env: sys::napi_env, raw: sys::napi_value) -> napi::Result<Self> {
    Ok(Self(string(env, raw, 1024, "Win32 contract name")?))
  }
}

impl Descriptor {
  pub fn into_string(self) -> String {
    self.0
  }
}

impl Deref for Descriptor {
  type Target = String;
  fn deref(&self) -> &Self::Target {
    &self.0
  }
}

impl TypeName for Descriptor {
  fn type_name() -> &'static str {
    "string"
  }
  fn value_type() -> napi::ValueType {
    napi::ValueType::String
  }
}

impl ValidateNapiValue for Descriptor {}

impl FromNapiValue for Descriptor {
  unsafe fn from_napi_value(env: sys::napi_env, raw: sys::napi_value) -> napi::Result<Self> {
    Ok(Self(string(
      env,
      raw,
      MAX_NATIVE_AGGREGATE_DESCRIPTOR_LENGTH,
      "native aggregate descriptor",
    )?))
  }
}

fn string(
  env: sys::napi_env,
  raw: sys::napi_value,
  limit: usize,
  description: &str,
) -> napi::Result<String> {
  let mut length = 0;
  napi::check_status!(unsafe {
    sys::napi_get_value_string_utf8(env, raw, ptr::null_mut(), 0, &mut length)
  })?;
  if length > limit {
    return Err(napi::Error::from_reason(format!(
      "{description} exceeds the {limit} byte safety limit"
    )));
  }
  let mut bytes = Vec::new();
  bytes
    .try_reserve_exact(length + 1)
    .map_err(|_| napi::Error::from_reason(format!("Unable to allocate {description}")))?;
  bytes.resize(length + 1, 0);
  let mut written = 0;
  napi::check_status!(unsafe {
    sys::napi_get_value_string_utf8(
      env,
      raw,
      bytes.as_mut_ptr().cast(),
      bytes.len(),
      &mut written,
    )
  })?;
  if written != length {
    return Err(napi::Error::from_reason(format!(
      "{description} length changed"
    )));
  }
  bytes.truncate(length);
  String::from_utf8(bytes)
    .map_err(|_| napi::Error::from_reason(format!("{description} is not valid UTF-8")))
}

fn property(
  env: sys::napi_env,
  object: sys::napi_value,
  name: &CStr,
) -> napi::Result<sys::napi_value> {
  let mut value = ptr::null_mut();
  napi::check_status!(unsafe {
    sys::napi_get_named_property(env, object, name.as_ptr(), &mut value)
  })?;
  Ok(value)
}

fn optional(env: sys::napi_env, raw: sys::napi_value) -> napi::Result<bool> {
  let mut kind = sys::ValueType::napi_undefined;
  napi::check_status!(unsafe { sys::napi_typeof(env, raw, &mut kind) })?;
  Ok(matches!(
    kind,
    sys::ValueType::napi_null | sys::ValueType::napi_undefined
  ))
}

fn optional_string(
  env: sys::napi_env,
  object: sys::napi_value,
  name: &CStr,
  limit: usize,
) -> napi::Result<Option<String>> {
  let value = property(env, object, name)?;
  if optional(env, value)? {
    Ok(None)
  } else {
    Ok(Some(string(env, value, limit, &name.to_string_lossy())?))
  }
}

fn optional_bool(
  env: sys::napi_env,
  object: sys::napi_value,
  name: &CStr,
) -> napi::Result<Option<bool>> {
  let value = property(env, object, name)?;
  if optional(env, value)? {
    return Ok(None);
  }
  let mut boolean = false;
  napi::check_status!(unsafe { sys::napi_get_value_bool(env, value, &mut boolean) })?;
  Ok(Some(boolean))
}

fn closed_object(env: sys::napi_env, object: sys::napi_value, keys: &[&str]) -> napi::Result<()> {
  let mut kind = sys::ValueType::napi_undefined;
  napi::check_status!(unsafe { sys::napi_typeof(env, object, &mut kind) })?;
  if kind != sys::ValueType::napi_object {
    return Err(napi::Error::from_reason(
      "Win32 native specification must be an object",
    ));
  }
  let mut properties = ptr::null_mut();
  napi::check_status!(unsafe { sys::napi_get_property_names(env, object, &mut properties) })?;
  let mut count = 0;
  napi::check_status!(unsafe { sys::napi_get_array_length(env, properties, &mut count) })?;
  if count as usize > keys.len() {
    return Err(napi::Error::from_reason(
      "Win32 native specification has unknown fields",
    ));
  }
  for index in 0..count {
    let mut key = ptr::null_mut();
    napi::check_status!(unsafe { sys::napi_get_element(env, properties, index, &mut key) })?;
    let key = string(env, key, 128, "Win32 specification field")?;
    if !keys.contains(&key.as_str()) {
      return Err(napi::Error::from_reason(format!(
        "Unknown Win32 native specification field `{key}`"
      )));
    }
  }
  Ok(())
}

pub(super) fn function_spec(value: Unknown) -> napi::Result<DynWin32FunctionSpec> {
  let env = value.value().env;
  let raw = value.raw();
  closed_object(
    env,
    raw,
    &[
      "dll",
      "entryPoint",
      "parameters",
      "returnType",
      "returnCleanup",
      "successRule",
      "captureLastError",
      "callingConvention",
      "returnAggregateDescriptor",
    ],
  )?;
  let dll = string(env, property(env, raw, c"dll")?, 1024, "Win32 DLL name")?;
  let entry_point = string(
    env,
    property(env, raw, c"entryPoint")?,
    1024,
    "Win32 export name",
  )?;
  let array = property(env, raw, c"parameters")?;
  let mut is_array = false;
  napi::check_status!(unsafe { sys::napi_is_array(env, array, &mut is_array) })?;
  if !is_array {
    return Err(napi::Error::from_reason(
      "Win32 parameters must be an array",
    ));
  }
  let mut count = 0;
  napi::check_status!(unsafe { sys::napi_get_array_length(env, array, &mut count) })?;
  if count > 1024 {
    return Err(napi::Error::from_reason(
      "Win32 parameter count exceeds the 1024 limit",
    ));
  }
  let mut parameters = Vec::new();
  parameters
    .try_reserve_exact(count as usize)
    .map_err(|_| napi::Error::from_reason("Unable to allocate Win32 parameters"))?;
  for index in 0..count {
    let mut parameter = ptr::null_mut();
    napi::check_status!(unsafe { sys::napi_get_element(env, array, index, &mut parameter) })?;
    closed_object(
      env,
      parameter,
      &[
        "type",
        "direction",
        "nullable",
        "cleanup",
        "consumesResource",
        "resourceCleanup",
        "aggregateDescriptor",
      ],
    )?;
    parameters.push(DynWin32ParameterSpec {
      typ: string(
        env,
        property(env, parameter, c"type")?,
        1024,
        "Win32 parameter type",
      )?,
      direction: string(
        env,
        property(env, parameter, c"direction")?,
        1024,
        "Win32 parameter direction",
      )?,
      nullable: optional_bool(env, parameter, c"nullable")?,
      cleanup: optional_string(env, parameter, c"cleanup", 1024)?,
      consumes_resource: optional_bool(env, parameter, c"consumesResource")?,
      resource_cleanup: optional_string(env, parameter, c"resourceCleanup", 1024)?,
      aggregate_descriptor: optional_string(
        env,
        parameter,
        c"aggregateDescriptor",
        MAX_NATIVE_AGGREGATE_DESCRIPTOR_LENGTH,
      )?,
    });
  }
  Ok(DynWin32FunctionSpec {
    dll,
    entry_point,
    parameters,
    return_type: optional_string(env, raw, c"returnType", 1024)?,
    return_cleanup: optional_string(env, raw, c"returnCleanup", 1024)?,
    success_rule: optional_string(env, raw, c"successRule", 1024)?,
    capture_last_error: optional_bool(env, raw, c"captureLastError")?,
    calling_convention: optional_string(env, raw, c"callingConvention", 1024)?,
    return_aggregate_descriptor: optional_string(
      env,
      raw,
      c"returnAggregateDescriptor",
      MAX_NATIVE_AGGREGATE_DESCRIPTOR_LENGTH,
    )?,
  })
}

pub(super) fn arguments<'a>(value: &'a Unknown) -> napi::Result<Vec<&'a DynWin32Value>> {
  let env = value.value().env;
  let raw = value.raw();
  let mut is_array = false;
  napi::check_status!(unsafe { sys::napi_is_array(env, raw, &mut is_array) })?;
  if !is_array {
    return Err(napi::Error::from_reason(
      "Win32 invocation arguments must be an array",
    ));
  }
  let mut count = 0;
  napi::check_status!(unsafe { sys::napi_get_array_length(env, raw, &mut count) })?;
  if count > 1024 {
    return Err(napi::Error::from_reason(
      "Win32 argument count exceeds the 1024 limit",
    ));
  }
  let mut arguments = Vec::new();
  arguments
    .try_reserve_exact(count as usize)
    .map_err(|_| napi::Error::from_reason("Unable to allocate Win32 invocation arguments"))?;
  for index in 0..count {
    let mut element = ptr::null_mut();
    napi::check_status!(unsafe { sys::napi_get_element(env, raw, index, &mut element) })?;
    arguments.push(unsafe { <&DynWin32Value>::from_napi_value(env, element) }?);
  }
  Ok(arguments)
}
