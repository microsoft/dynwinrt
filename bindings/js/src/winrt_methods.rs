// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::{com_value, scheduled_start, DynWinRTType, DynWinRTValue, TABLE};
use napi::{bindgen_prelude::PromiseRaw, Env};
use napi_derive::napi;
use windows::core::{Interface, HSTRING};

native_class! {
  pub struct DynWinRTMethodSig(pub(crate) dynwinrt::MethodSignature);
}
unsafe impl Send for DynWinRTMethodSig {}
unsafe impl Sync for DynWinRTMethodSig {}

#[napi]
impl DynWinRTMethodSig {
  #[napi(constructor)]
  pub fn new() -> Self {
    DynWinRTMethodSig(dynwinrt::MethodSignature::new(&*TABLE))
  }

  /// Add an [in] parameter.
  #[napi]
  pub fn add_in(&self, typ: &DynWinRTType) -> DynWinRTMethodSig {
    DynWinRTMethodSig(self.0.clone().add_in(typ.0.clone()))
  }

  /// Add an [out] parameter.
  #[napi]
  pub fn add_out(&self, typ: &DynWinRTType) -> DynWinRTMethodSig {
    DynWinRTMethodSig(self.0.clone().add_out(typ.0.clone()))
  }

  /// Add a FillArray [out] parameter: caller allocates buffer, callee fills it.
  #[napi]
  pub fn add_out_fill(&self, typ: &DynWinRTType) -> DynWinRTMethodSig {
    DynWinRTMethodSig(self.0.clone().add_out_fill(typ.0.clone()))
  }
}

// ======================================================================
// MethodHandle binding
// ======================================================================

native_class! {
  pub struct DynWinRTMethodHandle(pub(crate) dynwinrt::MethodHandle);
}
unsafe impl Send for DynWinRTMethodHandle {}
unsafe impl Sync for DynWinRTMethodHandle {}

#[napi]
impl DynWinRTMethodHandle {
  /// Invoke this method on a COM object.
  #[napi]
  pub fn invoke(
    &self,
    obj: &DynWinRTValue,
    args: Vec<&DynWinRTValue>,
  ) -> napi::Result<DynWinRTValue> {
    let raw = match obj.winrt() {
      dynwinrt::WinRTValue::Object(o) => o.as_raw(),
      _ => {
        return Err(napi::Error::from_reason(
          "invoke() requires an Object value",
        ));
      }
    };
    let _leases = com_value::collect_invocation_leases(&args)?;
    let wrt_args: Vec<dynwinrt::WinRTValue> = args.iter().map(|a| a.winrt().clone()).collect();
    let results = self
      .0
      .invoke(raw, &wrt_args)
      .map_err(|e| napi::Error::from_reason(e.message()))?;
    if results.is_empty() {
      Ok(DynWinRTValue::new(dynwinrt::WinRTValue::I32(0)))
    } else {
      Ok(DynWinRTValue::new(results.into_iter().next().ok_or_else(
        || napi::Error::from_reason("invoke: method returned no results"),
      )?))
    }
  }

  /// Schedule a blocking WinUI Application.Start invocation after the current
  /// JavaScript callback unwinds.
  #[napi]
  pub fn invoke_scheduled<'env>(
    &self,
    env: Env,
    obj: &DynWinRTValue,
    args: Vec<&DynWinRTValue>,
  ) -> napi::Result<PromiseRaw<'env, ()>> {
    let leases = com_value::collect_invocation_leases(&args)?;
    scheduled_start::schedule(
      env,
      self.0.clone(),
      obj.winrt().clone(),
      args
        .into_iter()
        .map(|value| value.winrt().clone())
        .collect(),
      leases,
    )
  }

  /// Like `invoke`, but returns all out-parameters as an array.
  /// Used for methods with multiple out params (e.g. IVector.IndexOf → [u32 index, bool found]).
  #[napi]
  pub fn invoke_all(
    &self,
    obj: &DynWinRTValue,
    args: Vec<&DynWinRTValue>,
  ) -> napi::Result<Vec<DynWinRTValue>> {
    let raw = match obj.winrt() {
      dynwinrt::WinRTValue::Object(o) => o.as_raw(),
      _ => {
        return Err(napi::Error::from_reason(
          "invoke_all() requires an Object value",
        ));
      }
    };
    let _leases = com_value::collect_invocation_leases(&args)?;
    let wrt_args: Vec<dynwinrt::WinRTValue> = args.iter().map(|a| a.winrt().clone()).collect();
    let results = self
      .0
      .invoke(raw, &wrt_args)
      .map_err(|e| napi::Error::from_reason(e.message()))?;
    Ok(results.into_iter().map(DynWinRTValue::new).collect())
  }

  // --- Fast paths: skip Vec alloc + skip DynWinRTValue wrapping for result ---

  /// Getter → string (0 args, returns JS string directly, zero Vec allocation)
  #[napi]
  pub fn get_string(&self, obj: &DynWinRTValue) -> napi::Result<String> {
    let raw = obj
      .winrt()
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("get_string: not an Object"))?
      .as_raw();
    let hs = self
      .0
      .call_getter_hstring(raw)
      .map_err(|e| napi::Error::from_reason(e.message()))?;
    Ok(hs.to_string())
  }

  /// Getter → i32 (0 args, returns JS number directly, zero Vec allocation)
  #[napi]
  pub fn get_i32(&self, obj: &DynWinRTValue) -> napi::Result<i32> {
    let raw = obj
      .winrt()
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("get_i32: not an Object"))?
      .as_raw();
    self
      .0
      .call_getter_i32(raw)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  /// Getter → bool (0 args, returns JS boolean directly, zero Vec allocation)
  #[napi]
  pub fn get_bool(&self, obj: &DynWinRTValue) -> napi::Result<bool> {
    let raw = obj
      .winrt()
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("get_bool: not an Object"))?
      .as_raw();
    self
      .0
      .call_getter_bool(raw)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  /// Getter → DynWinRTValue (0 args, returns wrapped object, zero Vec allocation)
  #[napi]
  pub fn get_obj(&self, obj: &DynWinRTValue) -> napi::Result<DynWinRTValue> {
    let raw = obj
      .winrt()
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("get_obj: not an Object"))?
      .as_raw();
    self
      .0
      .call_getter_object(raw)
      .map(DynWinRTValue::new)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  #[napi]
  pub fn set_hstring(&self, obj: &DynWinRTValue, value: String) -> napi::Result<()> {
    let raw = obj
      .winrt()
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("set_hstring: not an Object"))?
      .as_raw();
    self
      .0
      .call_setter_hstring(raw, &HSTRING::from(value))
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  #[napi]
  pub fn set_bool(&self, obj: &DynWinRTValue, value: bool) -> napi::Result<()> {
    let raw = obj
      .winrt()
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("set_bool: not an Object"))?
      .as_raw();
    self
      .0
      .call_setter_bool(raw, value)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  #[napi]
  pub fn set_i32(&self, obj: &DynWinRTValue, value: i32) -> napi::Result<()> {
    let raw = obj
      .winrt()
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("set_i32: not an Object"))?
      .as_raw();
    self
      .0
      .call_setter_i32(raw, value)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  #[napi]
  pub fn set_u32(&self, obj: &DynWinRTValue, value: u32) -> napi::Result<()> {
    let raw = obj
      .winrt()
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("set_u32: not an Object"))?
      .as_raw();
    self
      .0
      .call_setter_u32(raw, value)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  #[napi]
  pub fn set_f32(&self, obj: &DynWinRTValue, value: f64) -> napi::Result<()> {
    let raw = obj
      .winrt()
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("set_f32: not an Object"))?
      .as_raw();
    self
      .0
      .call_setter_f32(raw, value as f32)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  #[napi]
  pub fn set_f64(&self, obj: &DynWinRTValue, value: f64) -> napi::Result<()> {
    let raw = obj
      .winrt()
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("set_f64: not an Object"))?
      .as_raw();
    self
      .0
      .call_setter_f64(raw, value)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  /// 1-arg invoke with hstring input → DynWinRTValue result
  #[napi]
  pub fn invoke_hstring(&self, obj: &DynWinRTValue, arg: String) -> napi::Result<DynWinRTValue> {
    let raw = obj
      .winrt()
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("invoke_hstring: not an Object"))?
      .as_raw();
    let results = self
      .0
      .invoke(raw, &[dynwinrt::WinRTValue::HString(HSTRING::from(arg))])
      .map_err(|e| napi::Error::from_reason(e.message()))?;
    Ok(DynWinRTValue::new(results.into_iter().next().ok_or_else(
      || napi::Error::from_reason("invoke_hstring: no result"),
    )?))
  }

  /// 1-arg invoke with i32 input → DynWinRTValue result
  #[napi]
  pub fn invoke_i32(&self, obj: &DynWinRTValue, arg: i32) -> napi::Result<DynWinRTValue> {
    let raw = obj
      .winrt()
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("invoke_i32: not an Object"))?
      .as_raw();
    let results = self
      .0
      .invoke(raw, &[dynwinrt::WinRTValue::I32(arg)])
      .map_err(|e| napi::Error::from_reason(e.message()))?;
    Ok(DynWinRTValue::new(results.into_iter().next().ok_or_else(
      || napi::Error::from_reason("invoke_i32: no result"),
    )?))
  }
}

// ======================================================================
// Raw N-API fast getters — bypass napi-rs macro layer entirely
// ======================================================================
//
// These use napi_sys to unwrap napi-rs managed objects directly,
// call dynwinrt's zero-alloc getter path, and return JS primitives.
// Registered as standalone functions: rawGetString(method, obj) → string

// Standalone #[napi] functions — same zero-alloc getter path as methods,
// but as free functions for benchmark comparison.
// napi-rs overhead here: unwrap 2 class refs + return primitive.

/// rawGetString(methodHandle, objValue) → string
#[napi]
pub fn raw_get_string(
  method: &DynWinRTMethodHandle,
  #[napi(ts_arg_type = "DynWinRtValue")] obj: &DynWinRTValue,
) -> napi::Result<String> {
  let raw = match obj.winrt() {
    dynwinrt::WinRTValue::Object(o) => o.as_raw(),
    _ => return Err(napi::Error::from_reason("not an Object")),
  };
  Ok(
    method
      .0
      .call_getter_hstring(raw)
      .map_err(|e| napi::Error::from_reason(e.message()))?
      .to_string(),
  )
}

/// rawGetI32(methodHandle, objValue) → number
#[napi]
pub fn raw_get_i32(
  method: &DynWinRTMethodHandle,
  #[napi(ts_arg_type = "DynWinRtValue")] obj: &DynWinRTValue,
) -> napi::Result<i32> {
  let raw = match obj.winrt() {
    dynwinrt::WinRTValue::Object(o) => o.as_raw(),
    _ => return Err(napi::Error::from_reason("not an Object")),
  };
  method
    .0
    .call_getter_i32(raw)
    .map_err(|e| napi::Error::from_reason(e.message()))
}
