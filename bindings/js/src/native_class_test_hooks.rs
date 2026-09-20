// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
  ffi::c_void,
  ptr,
  sync::atomic::{AtomicU32, Ordering},
};

use napi::{
  bindgen_prelude::{ClassInstance, FromNapiValue, Reference, Unknown},
  sys, Env, JsValue,
};
use napi_derive::napi;

use crate::DynWinRTValue;

static ENTERED: AtomicU32 = AtomicU32::new(0);
static FINALIZED: AtomicU32 = AtomicU32::new(0);

type ValueAlias = &'static DynWinRTValue;

fn entered() -> u32 {
  ENTERED.fetch_add(1, Ordering::SeqCst) + 1
}

native_class! {
  pub struct NativeClassTestProbe {
    value: u32,
  }
}

#[napi]
impl NativeClassTestProbe {
  #[napi(constructor)]
  pub fn new() -> Self {
    Self { value: 0 }
  }

  #[napi(getter)]
  pub fn value(&self) -> u32 {
    entered();
    self.value
  }

  #[napi(setter)]
  pub fn set_value(&mut self, value: u32) {
    entered();
    self.value = value;
  }

  #[napi]
  pub fn read(_value: &DynWinRTValue) -> u32 {
    entered()
  }

  #[napi]
  pub fn write(_value: &mut DynWinRTValue) -> u32 {
    entered()
  }

  #[napi]
  pub fn alias(#[napi(ts_arg_type = "DynWinRtValue")] _value: ValueAlias) -> u32 {
    entered()
  }

  #[napi]
  pub fn instance(_value: ClassInstance<'_, DynWinRTValue>) -> u32 {
    entered()
  }

  #[napi]
  pub fn reference(_value: Reference<DynWinRTValue>) -> u32 {
    entered()
  }
}

#[napi]
pub fn native_class_test_bridge(value: Unknown<'_>) -> napi::Result<u32> {
  unsafe {
    crate::native_class_ref::with_ref::<DynWinRTValue, _>(value.value().env, value.raw(), |_| {
      Ok(entered())
    })
  }
}

#[napi]
pub fn native_class_test_entries() -> u32 {
  ENTERED.load(Ordering::SeqCst)
}

#[napi]
pub fn native_class_test_finalized() -> u32 {
  FINALIZED.load(Ordering::SeqCst)
}

unsafe extern "C" fn finalize(_: sys::napi_env, data: *mut c_void, _: *mut c_void) {
  drop(unsafe { Box::from_raw(data.cast::<DynWinRTValue>()) });
  FINALIZED.fetch_add(1, Ordering::SeqCst);
}

fn control_value<'env>(env: Env, brand: u32) -> napi::Result<Unknown<'env>> {
  let mut object = ptr::null_mut();
  napi::check_status!(unsafe { sys::napi_create_object(env.raw(), &mut object) })?;
  let mut value = Box::new(DynWinRTValue::new(dynwinrt::WinRTValue::Null));
  napi::check_status!(unsafe {
    sys::napi_wrap(
      env.raw(),
      object,
      (&mut *value as *mut DynWinRTValue).cast(),
      Some(finalize),
      ptr::null_mut(),
      ptr::null_mut(),
    )
  })?;
  let _ = Box::into_raw(value);
  if brand != 0 {
    let tag = napi::bindgen_prelude::type_tag_from_ident(
      "dynwinrt-native-class-control/other-build::jswinrt_rs::value::DynWinRTValue",
    );
    unsafe { napi::bindgen_prelude::tag_object(env.raw(), object, &tag) }?;
    if brand == 2 {
      // The payload is still the same valid Rust type. Only the immutable
      // identity differs; the installed finalizer owns it on tagging failure.
      unsafe { napi::bindgen_prelude::tag_object(env.raw(), object, &tag) }?;
    }
  }
  unsafe { Unknown::from_napi_value(env.raw(), object) }
}

#[napi]
pub fn native_class_test_untagged<'env>(env: Env) -> napi::Result<Unknown<'env>> {
  control_value(env, 0)
}

#[napi]
pub fn native_class_test_other_build<'env>(env: Env) -> napi::Result<Unknown<'env>> {
  control_value(env, 1)
}

#[napi]
pub fn native_class_test_duplicate_tag<'env>(env: Env) -> napi::Result<Unknown<'env>> {
  control_value(env, 2)
}
