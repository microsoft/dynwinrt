// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{ptr, sync::Arc};

use dynwinrt::win32::{CallError, ResultTarget};
use napi::{
  bindgen_prelude::{JsObjectValue, Object, ToNapiValue},
  sys, Env, Property, PropertyAttributes,
};
use napi_derive::napi;

use super::{boundary, DynWin32Resource};

pub struct DynWin32CallError {
  context: String,
  error: Option<CallError>,
}
boundary::carrier!(DynWin32CallError, 8, "DynWin32CallError");

impl DynWin32CallError {
  fn error(&self) -> napi::Result<&CallError> {
    self
      .error
      .as_ref()
      .ok_or_else(|| napi::Error::from_reason("Win32 call error has not been installed"))
  }

  fn message(&self) -> napi::Result<String> {
    Ok(format!("{}{}", self.context, self.error()?.message()))
  }
}

pub(super) struct PreparedCallError {
  env: Env,
  object: sys::napi_value,
  state: *mut DynWin32CallError,
}

impl PreparedCallError {
  /// Prepare all JS storage before native dispatch can produce an owned result.
  pub(super) fn new(env: Env, context: String) -> napi::Result<Self> {
    let message = unsafe { String::to_napi_value(env.raw(), String::new()) }?;
    let mut object = ptr::null_mut();
    napi::check_status!(unsafe {
      sys::napi_create_error(env.raw(), ptr::null_mut(), message, &mut object)
    })?;
    let state = unsafe {
      boundary::wrap_existing(
        env.raw(),
        object,
        DynWin32CallError {
          context,
          error: None,
        },
      )
    }?;
    Object::from_raw(env.raw(), object).define_properties(&[
      Property::new()
        .with_utf8_name("message")?
        .with_getter(message_getter)
        .with_property_attributes(PropertyAttributes::Configurable),
      Property::new()
        .with_utf8_name("name")?
        .with_napi_value(&env, "DynWin32CallError")?
        .with_property_attributes(PropertyAttributes::Configurable | PropertyAttributes::Writable),
      Property::new()
        .with_utf8_name("code")?
        .with_napi_value(&env, "GenericFailure")?
        .with_property_attributes(PropertyAttributes::Configurable | PropertyAttributes::Writable),
    ])?;
    Ok(Self { env, object, state })
  }

  pub(super) fn raise(self, error: CallError) -> napi::Error {
    // The callback's handle scope roots this object until napi_throw takes over.
    let state = unsafe { &mut *self.state };
    if error.cleanup_failures().is_empty() {
      return napi::Error::from_reason(format!("{}{}", state.context, error.message()));
    }
    state.error = Some(error);
    // No allocation, coercion, or user-controlled toString/cause access occurs
    // between installing the native owner and throwing the existing Error.
    let status = unsafe { sys::napi_throw(self.env.raw(), self.object) };
    if status != sys::Status::napi_ok {
      return napi::Error::new(
        napi::Status::from(status),
        "Unable to throw the retained Win32 cleanup error",
      );
    }
    napi::Error::new(napi::Status::PendingException, String::new())
  }
}

unsafe extern "C" fn message_getter(
  env: sys::napi_env,
  info: sys::napi_callback_info,
) -> sys::napi_value {
  let result = (|| {
    let mut receiver = ptr::null_mut();
    napi::check_status!(unsafe {
      sys::napi_get_cb_info(
        env,
        info,
        ptr::null_mut(),
        ptr::null_mut(),
        &mut receiver,
        ptr::null_mut(),
      )
    })?;
    let value = unsafe { &*boundary::unwrap::<DynWin32CallError>(env, receiver)? };
    unsafe { String::to_napi_value(env, value.message()?) }
  })();
  match result {
    Ok(value) => value,
    Err(error) => {
      unsafe { napi::JsError::from(error).throw_into(env) };
      ptr::null_mut()
    }
  }
}

fn field<T: ToNapiValue>(env: &Env, name: &str, value: T) -> napi::Result<Property> {
  // Define own data fields without invoking inherited setters during recovery.
  Ok(
    Property::new()
      .with_utf8_name(name)?
      .with_napi_value(env, value)?
      .with_property_attributes(PropertyAttributes::Enumerable),
  )
}

#[napi(ts_return_type = "DynWin32CleanupFailure[]")]
pub fn win32_call_error_cleanup_failures<'env>(
  env: Env,
  value: &DynWin32CallError,
) -> napi::Result<Vec<Object<'env>>> {
  value
    .error()?
    .cleanup_failures()
    .iter()
    .map(|failure| {
      let mut target = Object::new(&env)?;
      let indices = match failure.target() {
        ResultTarget::Return {} => {
          target.define_properties(&[field(&env, "kind", "return")?])?;
          Vec::new()
        }
        ResultTarget::Parameter { index } => {
          target.define_properties(&[field(&env, "kind", "parameter")?])?;
          vec![("index", index)]
        }
        ResultTarget::AggregateField {
          parameter,
          field: index,
        } => {
          target.define_properties(&[field(&env, "kind", "aggregate-field")?])?;
          vec![("parameter", parameter), ("field", index)]
        }
      };
      for (name, index) in indices {
        let index = u32::try_from(index)
          .map_err(|_| napi::Error::from_reason("Win32 cleanup target index exceeds u32"))?;
        target.define_properties(&[field(&env, name, index)?])?;
      }
      let mut error = Object::new(&env)?;
      error.define_properties(&[
        field(&env, "code", failure.error().code().0)?,
        field(&env, "message", failure.error().to_string())?,
      ])?;
      let mut record = Object::new(&env)?;
      record.define_properties(&[
        field(&env, "target", target)?,
        field(&env, "error", error)?,
        field(
          &env,
          "resource",
          DynWin32Resource(Arc::clone(failure.resource())),
        )?,
      ])?;
      Ok(record)
    })
    .collect()
}

#[napi]
pub fn win32_call_error_retry_cleanup(value: &DynWin32CallError) -> napi::Result<()> {
  value
    .error()?
    .retry_cleanup()
    .map_err(|error| napi::Error::from_reason(error.to_string()))
}

#[cfg(feature = "test-hooks")]
#[napi]
pub fn win32_test_cleanup_failure(env: Env) -> napi::Result<()> {
  let plan = dynwinrt::win32::test_cleanup_failure_plan()
    .map_err(|error| napi::Error::from_reason(error.message()))?;
  super::DynWin32Function(plan)
    .invoke(env, Vec::new())
    .map(|_| ())
}
