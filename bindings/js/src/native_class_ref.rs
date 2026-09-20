// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use napi::{
  bindgen_prelude::{FromNapiMutRef, FromNapiRef},
  sys,
};

/// Keep checked native references inside the synchronous bridge. The
/// higher-ranked closure cannot return a borrow of the native payload.
pub(crate) unsafe fn with_ref<T: FromNapiRef + 'static, R>(
  env: sys::napi_env,
  value: sys::napi_value,
  operation: impl for<'a> FnOnce(&'a T) -> napi::Result<R>,
) -> napi::Result<R> {
  let value = unsafe { T::from_napi_ref(env, value) }?;
  operation(value)
}

pub(crate) unsafe fn with_mut<T: FromNapiMutRef + 'static, R>(
  env: sys::napi_env,
  value: sys::napi_value,
  operation: impl for<'a> FnOnce(&'a mut T) -> napi::Result<R>,
) -> napi::Result<R> {
  let value = unsafe { T::from_napi_mut_ref(env, value) }?;
  operation(value)
}
