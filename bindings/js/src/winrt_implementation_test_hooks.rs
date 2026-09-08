// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::sync::{
  atomic::{AtomicU32, Ordering},
  Arc, Mutex,
};

use dynwinrt::{
  MethodSignature, WinRTValue, WinRtImplementation, WinRtImplementationPlan,
  WinRtInterfaceDefinition, WinRtMethodDefinition, WinRtThreadingPolicy,
};
use napi_derive::napi;
use windows::{
  core::{self as windows_core, Interface},
  ApplicationModel::Background::{
    BackgroundTaskCanceledEventHandler, BackgroundTaskDeferral, BackgroundTaskRegistration,
    IBackgroundTaskInstance,
  },
  Foundation::IStringable,
};

use crate::{winrt_implementation, DynWinRTValue};

static RETAINED: Mutex<Option<WinRTValue>> = Mutex::new(None);

#[napi(object)]
pub struct WinRtImplementationTestCall {
  pub hresult: i32,
  pub text: Option<String>,
}

#[napi(object)]
pub struct WinRtImplementationTestStats {
  pub live_controls: u32,
  pub finalized_callbacks: u32,
  pub environment_disconnects: u32,
}

#[napi]
pub fn winrt_implementation_test_stats() -> WinRtImplementationTestStats {
  WinRtImplementationTestStats {
    live_controls: winrt_implementation::live_control_count(),
    finalized_callbacks: winrt_implementation::FINALIZED_CALLBACKS.load(Ordering::Relaxed) as u32,
    environment_disconnects: winrt_implementation::ENV_DISCONNECTS.load(Ordering::Relaxed) as u32,
  }
}

#[napi]
pub fn winrt_implementation_test_retain(value: &DynWinRTValue) -> napi::Result<()> {
  value
    .0
    .as_object()
    .ok_or_else(|| napi::Error::from_reason("Expected an object"))?
    .cast::<IStringable>()
    .map_err(|error| napi::Error::from_reason(error.message()))?;
  let previous = RETAINED.lock().unwrap().replace(value.0.clone());
  drop(previous);
  Ok(())
}

fn call_retained(value: WinRTValue) -> WinRtImplementationTestCall {
  let result = value
    .as_object()
    .unwrap()
    .cast::<IStringable>()
    .and_then(|view| view.ToString());
  match result {
    Ok(text) => WinRtImplementationTestCall {
      hresult: 0,
      text: Some(text.to_string()),
    },
    Err(error) => WinRtImplementationTestCall {
      hresult: error.code().0,
      text: None,
    },
  }
}

#[napi]
pub fn winrt_implementation_test_invoke_retained(
  foreign_thread: Option<bool>,
) -> napi::Result<WinRtImplementationTestCall> {
  let value = RETAINED
    .lock()
    .unwrap()
    .clone()
    .ok_or_else(|| napi::Error::from_reason("No retained WinRT implementation"))?;
  if foreign_thread.unwrap_or(false) {
    std::thread::spawn(move || call_retained(value))
      .join()
      .map_err(|_| napi::Error::from_reason("Retained invocation thread panicked"))
  } else {
    Ok(call_retained(value))
  }
}

#[napi]
pub fn winrt_implementation_test_release_retained(
  foreign_thread: Option<bool>,
) -> napi::Result<()> {
  let value = RETAINED.lock().unwrap().take();
  if foreign_thread.unwrap_or(false) {
    std::thread::spawn(move || drop(value))
      .join()
      .map_err(|_| napi::Error::from_reason("Retained release thread panicked"))?;
  } else {
    drop(value);
  }
  Ok(())
}

#[napi]
pub fn winrt_implementation_test_background_instance() -> napi::Result<DynWinRTValue> {
  // The complete nine-slot contract matches the SDK IBackgroundTaskInstance_Vtbl.
  // This fixture never registers an OS task. The SDK projection below is an
  // independent native caller, not a reinterpreted pointer or a vtable alias.
  let table = &*crate::TABLE;
  let signature = || MethodSignature::new(table);
  let methods = [
    ("InstanceId", signature().add_out(table.guid_type())),
    (
      "Task",
      signature().add_out(table.interface(BackgroundTaskRegistration::IID)),
    ),
    ("Progress", signature().add_out(table.u32_type())),
    ("SetProgress", signature().add_in(table.u32_type())),
    ("TriggerDetails", signature().add_out(table.object())),
    (
      "Canceled",
      signature()
        .add_in(table.delegate(BackgroundTaskCanceledEventHandler::IID))
        .add_out(table.i64_type()),
    ),
    ("RemoveCanceled", signature().add_in(table.i64_type())),
    ("SuspendedCount", signature().add_out(table.u32_type())),
    (
      "GetDeferral",
      signature().add_out(table.interface(BackgroundTaskDeferral::IID)),
    ),
  ]
  .into_iter()
  .enumerate()
  .map(|(index, (name, signature))| WinRtMethodDefinition {
    name: name.into(),
    vtable_index: index + 6,
    signature,
  })
  .collect();
  let definition = WinRtInterfaceDefinition {
    name: "Windows.ApplicationModel.Background.IBackgroundTaskInstance".into(),
    interface_type: table.interface(IBackgroundTaskInstance::IID),
    required_iids: Vec::new(),
    methods,
  };
  let result = (|| {
    let plan = WinRtImplementationPlan::new(vec![definition], WinRtThreadingPolicy::OwnerThread)?;
    let progress = AtomicU32::new(42);
    let owner = WinRtImplementation::new(
      plan,
      Arc::new(move |_, slot, values| match slot {
        6 => Ok(vec![WinRTValue::Guid(windows_core::GUID::from_u128(
          0x72be118a_a19e_466a_a7d6_634a79e197b7,
        ))]),
        8 => Ok(vec![WinRTValue::U32(progress.load(Ordering::Relaxed))]),
        9 => {
          if let WinRTValue::U32(value) = values[0] {
            progress.store(value, Ordering::Relaxed);
          }
          Ok(Vec::new())
        }
        13 => Ok(vec![WinRTValue::U32(0)]),
        _ => Err(windows_core::Error::new(
          windows_core::HRESULT(0x80004001u32 as i32),
          "Fixture has no OS task registration, cancellation source, trigger, or deferral",
        )),
      }),
      None,
    )?;
    owner.to_value()
  })();
  result
    .map(DynWinRTValue::new)
    .map_err(|error| napi::Error::from_reason(error.message()))
}

#[napi]
pub fn winrt_implementation_test_background_progress(value: &DynWinRTValue) -> napi::Result<u32> {
  value
    .0
    .as_object()
    .ok_or_else(|| napi::Error::from_reason("Expected an object"))?
    .cast::<IBackgroundTaskInstance>()
    .and_then(|instance| instance.Progress())
    .map_err(|error| napi::Error::from_reason(error.message()))
}
