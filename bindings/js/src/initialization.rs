// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use napi_derive::napi;
use sha2::{Digest, Sha256};
use std::{cell::Cell, io::Read, path::Path, sync::OnceLock};

struct InitializedWinAppSdk {
  major: u32,
  minor: u32,
  bootstrap_dll_digest: Option<[u8; 32]>,
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
pub fn init_winappsdk(
  major: u32,
  minor: u32,
  bootstrap_dll_path: Option<String>,
) -> napi::Result<()> {
  let resolved_path = if let Some(path) = &bootstrap_dll_path {
    if path.trim().is_empty() || path.contains('\0') || !std::path::Path::new(path).is_file() {
      return Err(napi::Error::from_reason(
        "Windows App SDK bootstrapDllPath must name an existing DLL file",
      ));
    }
    Some(std::fs::canonicalize(path).map_err(|error| {
      napi::Error::from_reason(format!("Cannot resolve Bootstrap DLL {path}: {error}"))
    })?)
  } else {
    None
  };
  let dll_digest = resolved_path
    .as_deref()
    .map(bootstrap_digest)
    .transpose()
    .map_err(|error| napi::Error::from_reason(format!("Cannot read Bootstrap DLL: {error}")))?;
  if let Some(initialized) = WINAPP_SDK.get() {
    return ensure_winappsdk_request(initialized, major, minor, dll_digest.as_ref());
  }

  let context = dynwinrt::initialize_winappsdk_with_options(dynwinrt::WinAppSdkBootstrapOptions {
    major_version: major,
    minor_version: minor,
    build_version: 0,
    revision_version: 0,
    bootstrap_dll_path: resolved_path
      .as_ref()
      .map(|path| path.to_string_lossy().into_owned()),
  })
  .map_err(|e| napi::Error::from_reason(e.message()))?;
  let initialized = InitializedWinAppSdk {
    major,
    minor,
    bootstrap_dll_digest: dll_digest,
    context,
  };

  match WINAPP_SDK.set(initialized) {
    Ok(()) => Ok(()),
    Err(_) => {
      ensure_winappsdk_request(WINAPP_SDK.get().unwrap(), major, minor, dll_digest.as_ref())
    }
  }
}

fn bootstrap_digest(path: &Path) -> std::io::Result<[u8; 32]> {
  let mut file = std::fs::File::open(path)?;
  let mut digest = Sha256::new();
  let mut buffer = [0; 16384];
  loop {
    let length = file.read(&mut buffer)?;
    if length == 0 {
      break;
    }
    digest.update(&buffer[..length]);
  }
  Ok(digest.finalize().into())
}

fn ensure_winappsdk_request(
  initialized: &InitializedWinAppSdk,
  major: u32,
  minor: u32,
  bootstrap_dll_digest: Option<&[u8; 32]>,
) -> napi::Result<()> {
  if initialized.major != major || initialized.minor != minor {
    return Err(napi::Error::from_reason(format!(
      "Windows App SDK is already initialized for {}.{}; cannot reinitialize for {major}.{minor}",
      initialized.major, initialized.minor
    )));
  }
  if let Some(digest) = bootstrap_dll_digest {
    if initialized.bootstrap_dll_digest.as_ref() != Some(digest) {
      return Err(napi::Error::from_reason(
        "Windows App SDK is already initialized with another Bootstrap DLL selection; \
         restart the process to select an explicit/generated DLL path. \
         Existing two-version-argument calls keep the original initialization.",
      ));
    }
  }
  Ok(())
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

#[cfg(test)]
mod bootstrap_tests {
  use super::*;

  #[test]
  fn explicit_dll_identity_cannot_bypass_the_native_success_cache() {
    let initialized = InitializedWinAppSdk {
      major: 2,
      minor: 5,
      bootstrap_dll_digest: Some([1; 32]),
      context: dynwinrt::WinAppSdkContext {},
    };
    assert!(ensure_winappsdk_request(&initialized, 2, 5, Some(&[1; 32])).is_ok());
    assert!(ensure_winappsdk_request(&initialized, 2, 5, Some(&[2; 32])).is_err());
    assert!(ensure_winappsdk_request(&initialized, 2, 6, Some(&[1; 32])).is_err());
    assert!(ensure_winappsdk_request(&initialized, 2, 5, None).is_ok());
  }

  #[test]
  fn legacy_calls_keep_version_only_cache_semantics() {
    let initialized = InitializedWinAppSdk {
      major: 2,
      minor: 5,
      bootstrap_dll_digest: None,
      context: dynwinrt::WinAppSdkContext {},
    };
    assert!(ensure_winappsdk_request(&initialized, 2, 5, None).is_ok());
    assert!(ensure_winappsdk_request(&initialized, 2, 6, None).is_err());
    assert!(ensure_winappsdk_request(&initialized, 2, 5, Some(&[1; 32])).is_err());
  }
}
