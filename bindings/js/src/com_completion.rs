// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
  cell::RefCell,
  collections::HashMap,
  ptr,
  sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
  },
};

use dynwinrt::com::completion::{
  CompletionError, CompletionSignal, NativeSignalSink, OneShotContract, OneShotPlan,
  OwnerOperation, ResultArgument, StartArgument,
};
use napi::{
  bindgen_prelude::{FromNapiValue, PromiseRaw, ToNapiValue},
  sys, Env, JsError, Status,
};
use napi_derive::napi;
use serde::Deserialize;
use windows::core::GUID;

use super::{managed_tsfn::ManagedTsfn, DynWinRTValue, WinGUID};

const MAX_PENDING: usize = 256;
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

// These maps contain COM pointers and napi_deferred, and are NEVER shared with
// workers. Tokens are process-unique, so a recycled napi_env cannot revive one.
thread_local! {
  static OWNERS: RefCell<HashMap<usize, HashMap<u64, OwnerRecord>>> =
    RefCell::new(HashMap::new());
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
  version: u32,
  kind: String,
  library: String,
  export: String,
  calling_convention: String,
  has_this: bool,
  start_parameters: Vec<StartParameter>,
  handler: HandlerDescriptor,
  result: ResultDescriptor,
  allowed_targets: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StartParameter {
  Utf16Path,
  RefIid,
  NullPropVariant,
  NativeSignal,
  OwnedOperation,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HandlerDescriptor {
  iid: String,
  root: String,
  slot: usize,
  input: String,
  return_type: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultDescriptor {
  iid: String,
  root: String,
  slot: usize,
  outputs: Vec<ResultParameter>,
  return_type: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ResultParameter {
  Hresult,
  NullableOwnedInterface,
}

fn parse_contract(descriptor: &str) -> napi::Result<OneShotContract> {
  let descriptor: Descriptor = serde_json::from_str(descriptor).map_err(|error| {
    napi::Error::from_reason(format!("Invalid native completion descriptor: {error}"))
  })?;
  if descriptor.version != 1
    || descriptor.kind != "audio-activation"
    || descriptor.calling_convention != "system"
    || descriptor.has_this
    || descriptor.handler.root != "IUnknown"
    || descriptor.handler.input != "borrowed-operation"
    || descriptor.handler.return_type != "hresult"
    || descriptor.result.root != "IUnknown"
    || descriptor.result.return_type != "hresult"
  {
    return Err(napi::Error::from_reason(
      "Unsupported native completion descriptor",
    ));
  }
  fn guid(text: &str) -> napi::Result<GUID> {
    GUID::try_from(text).map_err(|_| napi::Error::from_reason("Invalid native completion IID"))
  }
  Ok(OneShotContract {
    library: descriptor.library,
    export: descriptor.export,
    start_arguments: descriptor
      .start_parameters
      .into_iter()
      .map(|parameter| match parameter {
        StartParameter::Utf16Path => StartArgument::Utf16Path,
        StartParameter::RefIid => StartArgument::RefIid,
        StartParameter::NullPropVariant => StartArgument::NullPropVariant,
        StartParameter::NativeSignal => StartArgument::NativeSignal,
        StartParameter::OwnedOperation => StartArgument::OwnedOperation,
      })
      .collect(),
    handler_iid: guid(&descriptor.handler.iid)?,
    handler_slot: descriptor.handler.slot,
    operation_iid: guid(&descriptor.result.iid)?,
    result_slot: descriptor.result.slot,
    result_arguments: descriptor
      .result
      .outputs
      .into_iter()
      .map(|parameter| match parameter {
        ResultParameter::Hresult => ResultArgument::HResult,
        ResultParameter::NullableOwnedInterface => ResultArgument::NullableOwnedInterface,
      })
      .collect(),
    allowed_targets: descriptor
      .allowed_targets
      .iter()
      .map(|iid| guid(iid))
      .collect::<napi::Result<_>>()?,
  })
}

struct PendingCompletion {
  deferred: sys::napi_deferred,
  signal: Arc<CompletionSignal>,
  operation: OwnerOperation,
  _handler: NativeSignalSink,
  plan: OneShotPlan,
  target: GUID,
}

enum OwnerRecord {
  Starting,
  Pending(Box<PendingCompletion>),
}

struct StartReservation {
  env: usize,
  token: u64,
  armed: bool,
}

impl StartReservation {
  fn reserve(env: Env, token: u64) -> napi::Result<Self> {
    let key = env.raw() as usize;
    OWNERS.with(|owners| {
      let mut owners = owners.borrow_mut();
      let pending = owners
        .get_mut(&key)
        .ok_or_else(|| napi::Error::from_reason("Native COM completion environment is closing"))?;
      if pending.len() >= MAX_PENDING {
        return Err(napi::Error::from_reason(
          "Native COM completion limit (256) reached",
        ));
      }
      pending.insert(token, OwnerRecord::Starting);
      Ok(Self {
        env: key,
        token,
        armed: true,
      })
    })
  }

  fn publish(mut self, pending: PendingCompletion) -> napi::Result<()> {
    let mut pending = Some(pending);
    let published = OWNERS.with(|owners| {
      let mut owners = owners.borrow_mut();
      if let Some(record @ OwnerRecord::Starting) = owners
        .get_mut(&self.env)
        .and_then(|records| records.get_mut(&self.token))
      {
        *record = OwnerRecord::Pending(Box::new(pending.take().expect("unpublished completion")));
        true
      } else {
        false
      }
    });
    if !published {
      return Err(napi::Error::from_reason(
        "Native COM completion environment closed during start",
      ));
    }
    self.armed = false;
    Ok(())
  }
}

impl Drop for StartReservation {
  fn drop(&mut self) {
    if self.armed {
      OWNERS.with(|owners| {
        if let Some(records) = owners.borrow_mut().get_mut(&self.env) {
          records.remove(&self.token);
        }
      });
    }
  }
}

impl Drop for PendingCompletion {
  fn drop(&mut self) {
    self.signal.close();
  }
}

fn ensure_owner_environment(env: Env) -> napi::Result<()> {
  let key = env.raw() as usize;
  if OWNERS.with(|owners| owners.borrow().contains_key(&key)) {
    return Ok(());
  }
  env.add_env_cleanup_hook(key, |key| {
    close_owner_environment(key);
  })?;
  OWNERS.with(|owners| owners.borrow_mut().insert(key, HashMap::new()));
  Ok(())
}

fn close_owner_environment(key: usize) {
  let pending = OWNERS.with(|owners| owners.borrow_mut().remove(&key));
  // Close all signals before any owner references are released. Windows may
  // still hold the handler; this does not cancel the native operation.
  if let Some(pending) = pending {
    for record in pending.values() {
      if let OwnerRecord::Pending(record) = record {
        record.signal.close();
      }
    }
    drop(pending);
  }
}

fn take_pending(env: sys::napi_env, token: u64) -> Option<PendingCompletion> {
  let record = OWNERS.with(|owners| {
    owners
      .borrow_mut()
      .get_mut(&(env as usize))
      .and_then(|pending| pending.remove(&token))
  });
  match record {
    Some(OwnerRecord::Pending(record)) => Some(*record),
    Some(OwnerRecord::Starting) | None => None,
  }
}

fn status(context: &str, result: sys::napi_status) -> napi::Result<()> {
  if result == sys::Status::napi_ok {
    Ok(())
  } else {
    Err(napi::Error::from_reason(format!(
      "{context}: {}",
      Status::from(result)
    )))
  }
}

fn reject(
  env: sys::napi_env,
  deferred: sys::napi_deferred,
  error: CompletionError,
) -> napi::Result<()> {
  let value = unsafe { JsError::from(napi::Error::from_reason(error.message)).into_value(env) };
  let stage = unsafe { String::to_napi_value(env, error.stage.to_owned()) }?;
  status("Set native completion stage", unsafe {
    sys::napi_set_named_property(env, value, c"stage".as_ptr(), stage)
  })?;
  if let Some(hr) = error.hresult {
    let hr = unsafe { i32::to_napi_value(env, hr.0) }?;
    status("Set native completion HRESULT", unsafe {
      sys::napi_set_named_property(env, value, c"hresult".as_ptr(), hr)
    })?;
  }
  status("Reject native completion", unsafe {
    sys::napi_reject_deferred(env, deferred, value)
  })
}

fn dispatch(token: u64, env: sys::napi_env) -> napi::Result<()> {
  let Some(pending) = take_pending(env, token) else {
    return Ok(());
  };
  pending.signal.close();
  let result = pending
    .plan
    .collect(&pending.operation, pending.target)
    .and_then(|object| {
      // napi-rs class construction can fail during worker termination before
      // installing its native finalizer. Keep the owned COM result in Rust
      // until an empty, finalizer-backed JS value has actually been created.
      let value = unsafe {
        DynWinRTValue::to_napi_value(env, DynWinRTValue::new(dynwinrt::WinRTValue::Null))
      }
      .map_err(|error| CompletionError::contract("projection", error.to_string()))?;
      let wrapped = unsafe { <&mut DynWinRTValue>::from_napi_value(env, value) }
        .map_err(|error| CompletionError::contract("projection", error.to_string()))?;
      wrapped.0 = dynwinrt::WinRTValue::Object(object);
      wrapped
        .bind_current_com_apartment()
        .map_err(|error| CompletionError::contract("projection", error.to_string()))?;
      Ok(value)
    });
  match result {
    Ok(value) => status("Resolve native completion", unsafe {
      sys::napi_resolve_deferred(env, pending.deferred, value)
    }),
    Err(error) => reject(env, pending.deferred, error),
  }
}

fn finalized(token: u64, env: sys::napi_env) {
  if env.is_null() {
    return;
  }
  if let Some(pending) = take_pending(env, token) {
    let error = CompletionError::contract(
      "dispatch",
      "Native completion dispatcher closed before delivery",
    );
    if let Err(error) = reject(env, pending.deferred, error) {
      eprintln!("[dynwinrt] native completion finalization failed: {error}");
    }
  }
}

pub(crate) fn start_with_plan<'env>(
  env: Env,
  plan: OneShotPlan,
  path: &str,
  target: GUID,
) -> napi::Result<PromiseRaw<'env, DynWinRTValue>> {
  ensure_owner_environment(env)?;
  let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
  // Reserve before entering COM, which can reenter the caller's apartment.
  let reservation = StartReservation::reserve(env, token)?;
  let mut deferred = ptr::null_mut();
  let mut promise = ptr::null_mut();
  status("Create native completion promise", unsafe {
    sys::napi_create_promise(env.raw(), &mut deferred, &mut promise)
  })?;
  let tsfn = ManagedTsfn::create_native_dispatch(
    env.raw(),
    dispatch,
    Some(Box::new(move |env| finalized(token, env))),
  )?;
  let signal = CompletionSignal::new(move || {
    let result = tsfn.call(token);
    if result != Status::Ok && result != Status::Closing {
      // Releasing this last handle schedules owner-side finalization, which
      // rejects any still-pending deferred rather than leaking a live TSFN.
      eprintln!("[dynwinrt] native completion enqueue failed: {result}");
    }
  });
  #[cfg(feature = "test-hooks")]
  super::com_completion_test_hooks::observe_signal(Arc::downgrade(&signal));
  let started = plan.create_signal_sink(signal.clone()).and_then(|handler| {
    plan
      .start(path, target, &handler)
      .map(|operation| (handler, operation))
  });
  match started {
    Ok((handler, operation)) => {
      reservation.publish(PendingCompletion {
        deferred,
        signal: signal.clone(),
        operation,
        _handler: handler,
        plan,
        target,
      })?;
      signal.publish();
    }
    Err(error) => {
      signal.close();
      reject(env.raw(), deferred, error)?;
    }
  }
  Ok(PromiseRaw::new(env.raw(), promise))
}

#[napi]
pub struct DynComAsync;

#[napi]
impl DynComAsync {
  #[napi]
  pub fn activate_audio_interface<'env>(
    env: Env,
    descriptor: String,
    path: String,
    target: &WinGUID,
  ) -> napi::Result<PromiseRaw<'env, DynWinRTValue>> {
    let contract = parse_contract(&descriptor)?;
    #[cfg(feature = "test-hooks")]
    let plan = super::com_completion_test_hooks::prepare_plan(contract)
      .map_err(|error| napi::Error::from_reason(error.to_string()))?;
    #[cfg(not(feature = "test-hooks"))]
    let plan = OneShotPlan::prepare(contract)
      .map_err(|error| napi::Error::from_reason(error.to_string()))?;
    start_with_plan(env, plan, &path, target.0)
  }
}

#[cfg(feature = "test-hooks")]
pub(crate) fn pending_count(env: Env) -> usize {
  OWNERS.with(|owners| {
    owners
      .borrow()
      .get(&(env.raw() as usize))
      .map_or(0, HashMap::len)
  })
}

#[cfg(feature = "test-hooks")]
#[napi]
pub fn com_completion_test_close_owner(env: Env) {
  close_owner_environment(env.raw() as usize);
}
