// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use napi_derive::napi;
use std::{cell::Cell, sync::OnceLock};

struct InitializedWinAppSdk {
  major: u32,
  minor: u32,
  context: dynwinrt::WinAppSdkContext,
}

static WINAPP_SDK: OnceLock<InitializedWinAppSdk> = OnceLock::new();

thread_local! {
  static WINUI_DISPATCHER_LOOP_ACTIVE: Cell<bool> = const { Cell::new(false) };
  static WINUI_DISPATCHER_LOOP_ENTERED: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn winui_dispatcher_loop_active() -> bool {
  WINUI_DISPATCHER_LOOP_ACTIVE.with(Cell::get)
}

pub(crate) fn winui_dispatcher_loop_exited() -> bool {
  WINUI_DISPATCHER_LOOP_ENTERED.with(Cell::get) && !winui_dispatcher_loop_active()
}

/// Add Windows App SDK to the process package graph without changing the calling thread's apartment.
#[napi]
pub fn init_winappsdk(major: u32, minor: u32) -> napi::Result<()> {
  if let Some(initialized) = WINAPP_SDK.get() {
    return ensure_winappsdk_version(initialized, major, minor);
  }

  let context = dynwinrt::initialize_winappsdk(major, minor)
    .map_err(|e| napi::Error::from_reason(e.message()))?;
  let initialized = InitializedWinAppSdk {
    major,
    minor,
    context,
  };

  match WINAPP_SDK.set(initialized) {
    Ok(()) => Ok(()),
    Err(_) => ensure_winappsdk_version(WINAPP_SDK.get().unwrap(), major, minor),
  }
}

fn ensure_winappsdk_version(
  initialized: &InitializedWinAppSdk,
  major: u32,
  minor: u32,
) -> napi::Result<()> {
  if initialized.major == major && initialized.minor == minor {
    Ok(())
  } else {
    Err(napi::Error::from_reason(format!(
      "Windows App SDK is already initialized for {}.{}; cannot reinitialize for {major}.{minor}",
      initialized.major, initialized.minor
    )))
  }
}

/// Return the framework resources.pri path selected by initWinappsdk.
#[napi]
pub fn get_winappsdk_resource_pri_path() -> napi::Result<String> {
  let initialized = WINAPP_SDK.get().ok_or_else(|| {
    napi::Error::from_reason(
      "Windows App SDK is not initialized; call initWinappsdk before requesting its resources",
    )
  })?;

  initialized
    .context
    .resource_pri_path()
    .map_err(|e| napi::Error::from_reason(e.to_string()))
}

#[napi]
pub fn ro_initialize(apartment_type: Option<i32>) {
  use windows::Win32::System::WinRT::{
    RoInitialize, RO_INIT_MULTITHREADED, RO_INIT_SINGLETHREADED,
  };
  let init_type = match apartment_type.unwrap_or(1) {
    0 => RO_INIT_SINGLETHREADED,
    _ => RO_INIT_MULTITHREADED,
  };
  // Ignore "already initialized" (S_FALSE) and "changed mode" (RPC_E_CHANGED_MODE)
  // This allows dynwinrt to work in hosts like Electron that pre-initialize COM.
  let _ = unsafe { RoInitialize(init_type) };
}

pub(crate) fn set_winui_dispatcher_loop_active(active: bool) {
  if active {
    WINUI_DISPATCHER_LOOP_ENTERED.with(|state| state.set(true));
  }
  WINUI_DISPATCHER_LOOP_ACTIVE.with(|state| state.set(active));
}
