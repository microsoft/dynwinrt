// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use napi::{
  bindgen_prelude::{FromNapiValue, Unknown},
  JsValue,
};
use napi_derive::napi;
use windows::core::{IInspectable, IUnknown, Interface, GUID};

use crate::{winrt_implementation::require_instance, DynWinRTMethodSig, DynWinRTValue, WinGUID};

type DelegateCall =
  dyn Fn(&IUnknown, &[dynwinrt::WinRTValue]) -> windows::core::Result<Vec<dynwinrt::WinRTValue>>;

/// A prepared caller for a metadata-described WinRT delegate's Invoke method.
/// Delegates derive from IUnknown, so Invoke is slot 3, not IInspectable slot 6.
/// Supply the full delegate signature and its concrete IID (including generic
/// type arguments). This does not register an interface or use the COM planner.
#[napi]
pub struct DynWinRtDelegateMethod {
  iid: GUID,
  call: Box<DelegateCall>,
}

fn clone_arguments(args: Vec<Unknown<'_>>) -> napi::Result<Vec<dynwinrt::WinRTValue>> {
  args
    .into_iter()
    .map(|value| {
      require_instance::<DynWinRTValue>(&value, "DynWinRtValue")?;
      Ok(
        unsafe { <&DynWinRTValue>::from_napi_value(value.value().env, value.raw()) }?
          .0
          .clone(),
      )
    })
    .collect()
}

pub(crate) fn invoke_delegate_value(
  target: dynwinrt::WinRTValue,
  iid: Unknown<'_>,
  signature: Unknown<'_>,
  args: Vec<Unknown<'_>>,
) -> napi::Result<Vec<DynWinRTValue>> {
  let caller = DynWinRtDelegateMethod::create(iid, signature)?;
  let args = clone_arguments(args)?;
  caller.invoke_native(target, &args)
}

impl DynWinRtDelegateMethod {
  fn invoke_native(
    &self,
    target: dynwinrt::WinRTValue,
    args: &[dynwinrt::WinRTValue],
  ) -> napi::Result<Vec<DynWinRTValue>> {
    if target.as_object().is_none() {
      return Err(napi::Error::from_reason(
        "Delegate invocation requires a live managed Object value",
      ));
    }
    let view = target.cast(&self.iid).map_err(|error| {
      napi::Error::from_reason(format!(
        "Delegate QueryInterface failed: {}",
        error.message()
      ))
    })?;
    let object = view.as_object().ok_or_else(|| {
      napi::Error::from_reason("Delegate QueryInterface did not return a managed Object")
    })?;
    (self.call)(&object, args)
      .map(|values| values.into_iter().map(DynWinRTValue::new).collect())
      .map_err(|error| napi::Error::from_reason(error.message()))
  }
}

#[napi]
impl DynWinRtDelegateMethod {
  #[napi(factory, strict)]
  pub fn create(
    #[napi(ts_arg_type = "WinGuid")] iid: Unknown<'_>,
    #[napi(ts_arg_type = "DynWinRtMethodSig")] signature: Unknown<'_>,
  ) -> napi::Result<Self> {
    require_instance::<WinGUID>(&iid, "WinGuid")?;
    require_instance::<DynWinRTMethodSig>(&signature, "DynWinRtMethodSig")?;
    let iid = unsafe { <&WinGUID>::from_napi_value(iid.value().env, iid.raw()) }?.0;
    if [
      GUID::zeroed(),
      IUnknown::IID,
      IInspectable::IID,
      windows::core::imp::IAgileObject::IID,
      windows::core::imp::IMarshal::IID,
      windows::core::imp::IWeakReference::IID,
      windows::core::imp::IWeakReferenceSource::IID,
    ]
    .contains(&iid)
    {
      return Err(napi::Error::from_reason(
        "A WinRT delegate caller requires a delegate IID, not an infrastructure IID",
      ));
    }
    let signature =
      unsafe { <&DynWinRTMethodSig>::from_napi_value(signature.value().env, signature.raw()) }?;
    let method = signature.0.clone().build(3);
    Ok(Self {
      iid,
      // Keep the native facade's unnameable prepared Method type opaque.
      call: Box::new(move |object, args| method.call_dynamic(object.as_raw(), args)),
    })
  }

  /// Query the delegate IID and synchronously invoke its native Invoke method.
  /// Returns all outputs in signature order; void delegates return [].
  /// Target and arguments are retained across QI and reentrant native calls.
  /// Like outbound invokeAll, FillArray inputs must be preallocated typed Array
  /// values. U32 capacities apply only to reverse interface handler inputs.
  #[napi(strict)]
  pub fn invoke(
    &self,
    #[napi(ts_arg_type = "DynWinRtValue")] value: Unknown<'_>,
    #[napi(ts_arg_type = "DynWinRtValue[]")] args: Vec<Unknown<'_>>,
  ) -> napi::Result<Vec<DynWinRTValue>> {
    require_instance::<DynWinRTValue>(&value, "DynWinRtValue")?;
    let value = unsafe { <&DynWinRTValue>::from_napi_value(value.value().env, value.raw()) }?;
    value.ensure_existing_com_apartment()?;
    let target = value.0.clone();
    let args = clone_arguments(args)?;
    self.invoke_native(target, &args)
  }
}
