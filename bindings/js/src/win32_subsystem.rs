// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::mem::MaybeUninit;
use std::sync::{LazyLock, Mutex, MutexGuard};

use napi_derive::napi;
use windows::Win32::Graphics::GdiPlus::{
  GdiplusShutdown, GdiplusStartup, GdiplusStartupInput, Ok as GDIPLUS_OK,
};
use windows::Win32::Media::MediaFoundation::{MFShutdown, MFStartup, MFSTARTUP_FULL, MF_VERSION};
use windows::Win32::Networking::WinSock::{WSACleanup, WSAGetLastError, WSAStartup, WSADATA};
use windows::Win32::System::AddressBook::{DeinitMapiUtil, ScInitMapiUtil};

const WINSOCK_VERSION_2_2: u16 = 0x0202;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubsystemKind {
  Winsock,
  GdiPlus,
  MediaFoundation,
  MapiUtilities,
}

impl SubsystemKind {
  fn parse(value: &str) -> napi::Result<Self> {
    match value.to_ascii_lowercase().as_str() {
      "winsock" => Ok(Self::Winsock),
      "gdiplus" | "gdi+" => Ok(Self::GdiPlus),
      "mediafoundation" | "media_foundation" => Ok(Self::MediaFoundation),
      "mapiutilities" | "mapi_utilities" => Ok(Self::MapiUtilities),
      _ => Err(napi::Error::from_reason(format!(
        "Unknown flat Win32 subsystem `{value}`"
      ))),
    }
  }

  const fn name(self) -> &'static str {
    match self {
      Self::Winsock => "winsock",
      Self::GdiPlus => "gdiplus",
      Self::MediaFoundation => "mediaFoundation",
      Self::MapiUtilities => "mapiUtilities",
    }
  }
}

#[derive(Default)]
struct CountedState {
  leases: usize,
}

#[derive(Default)]
struct GdiPlusState {
  leases: usize,
  token: usize,
}

static WINSOCK_STATE: LazyLock<Mutex<CountedState>> =
  LazyLock::new(|| Mutex::new(CountedState::default()));
static GDIPLUS_STATE: LazyLock<Mutex<GdiPlusState>> =
  LazyLock::new(|| Mutex::new(GdiPlusState::default()));
static MEDIA_FOUNDATION_STATE: LazyLock<Mutex<CountedState>> =
  LazyLock::new(|| Mutex::new(CountedState::default()));
static MAPI_UTILITIES_STATE: LazyLock<Mutex<CountedState>> =
  LazyLock::new(|| Mutex::new(CountedState::default()));

#[napi]
pub struct DynWin32SubsystemContext {
  kind: SubsystemKind,
  closed: Mutex<bool>,
}

pub(super) struct SubsystemCallGuard<'a> {
  _closed: MutexGuard<'a, bool>,
}

#[napi]
impl DynWin32SubsystemContext {
  #[napi(getter)]
  pub fn subsystem(&self) -> &'static str {
    self.kind.name()
  }

  #[napi(getter)]
  pub fn closed(&self) -> bool {
    *self
      .closed
      .lock()
      .unwrap_or_else(|error| error.into_inner())
  }

  #[napi]
  pub fn close(&self) -> napi::Result<()> {
    let mut closed = self
      .closed
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    if *closed {
      return Ok(());
    }
    release(self.kind)?;
    *closed = true;
    Ok(())
  }
}

impl Drop for DynWin32SubsystemContext {
  fn drop(&mut self) {
    let closed = self
      .closed
      .get_mut()
      .unwrap_or_else(|error| error.into_inner());
    if !*closed {
      if let Err(error) = release(self.kind) {
        eprintln!(
          "[dynwinrt] {} subsystem cleanup failed: {}",
          self.kind.name(),
          error.reason
        );
      }
      *closed = true;
    }
  }
}

pub(super) fn initialize(subsystem: &str) -> napi::Result<DynWin32SubsystemContext> {
  let kind = SubsystemKind::parse(subsystem)?;
  acquire(kind)?;
  Ok(DynWin32SubsystemContext {
    kind,
    closed: Mutex::new(false),
  })
}

pub(super) fn require(context: &DynWin32SubsystemContext, subsystem: &str) -> napi::Result<()> {
  drop(call_guard(context, subsystem)?);
  Ok(())
}

pub(super) fn call_guard<'a>(
  context: &'a DynWin32SubsystemContext,
  subsystem: &str,
) -> napi::Result<SubsystemCallGuard<'a>> {
  let expected = SubsystemKind::parse(subsystem)?;
  if context.kind != expected {
    return Err(napi::Error::from_reason(format!(
      "{} APIs require a {} context, received {}",
      expected.name(),
      expected.name(),
      context.kind.name()
    )));
  }
  let closed = context
    .closed
    .lock()
    .unwrap_or_else(|error| error.into_inner());
  if *closed {
    return Err(napi::Error::from_reason(format!(
      "{} subsystem context is closed",
      expected.name()
    )));
  }
  if !is_active(expected) {
    return Err(napi::Error::from_reason(format!(
      "{} subsystem is not initialized",
      expected.name()
    )));
  }
  Ok(SubsystemCallGuard { _closed: closed })
}

fn acquire(kind: SubsystemKind) -> napi::Result<()> {
  match kind {
    SubsystemKind::Winsock => acquire_winsock(),
    SubsystemKind::GdiPlus => acquire_gdiplus(),
    SubsystemKind::MediaFoundation => acquire_media_foundation(),
    SubsystemKind::MapiUtilities => acquire_mapi_utilities(),
  }
}

fn release(kind: SubsystemKind) -> napi::Result<()> {
  match kind {
    SubsystemKind::Winsock => release_winsock(),
    SubsystemKind::GdiPlus => release_gdiplus(),
    SubsystemKind::MediaFoundation => release_media_foundation(),
    SubsystemKind::MapiUtilities => release_mapi_utilities(),
  }
}

fn is_active(kind: SubsystemKind) -> bool {
  match kind {
    SubsystemKind::Winsock => {
      WINSOCK_STATE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .leases
        != 0
    }
    SubsystemKind::GdiPlus => {
      GDIPLUS_STATE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .leases
        != 0
    }
    SubsystemKind::MediaFoundation => {
      MEDIA_FOUNDATION_STATE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .leases
        != 0
    }
    SubsystemKind::MapiUtilities => {
      MAPI_UTILITIES_STATE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .leases
        != 0
    }
  }
}

fn acquire_winsock() -> napi::Result<()> {
  let mut state = WINSOCK_STATE
    .lock()
    .unwrap_or_else(|error| error.into_inner());
  if state.leases == 0 {
    let mut data = MaybeUninit::<WSADATA>::uninit();
    let status = unsafe { WSAStartup(WINSOCK_VERSION_2_2, data.as_mut_ptr()) };
    if status != 0 {
      return Err(napi::Error::from_reason(format!(
        "WSAStartup(2.2) failed with Winsock error {status}"
      )));
    }
    let data = unsafe { data.assume_init() };
    if data.wVersion != WINSOCK_VERSION_2_2 {
      let _ = unsafe { WSACleanup() };
      return Err(napi::Error::from_reason(format!(
        "Winsock 2.2 is unavailable; negotiated version 0x{:04x}",
        data.wVersion
      )));
    }
  }
  state.leases = state
    .leases
    .checked_add(1)
    .ok_or_else(|| napi::Error::from_reason("Winsock context count overflow"))?;
  Ok(())
}

fn release_winsock() -> napi::Result<()> {
  let mut state = WINSOCK_STATE
    .lock()
    .unwrap_or_else(|error| error.into_inner());
  if state.leases == 0 {
    return Err(napi::Error::from_reason(
      "Winsock subsystem context is not active",
    ));
  }
  if state.leases == 1 {
    let status = unsafe { WSACleanup() };
    if status != 0 {
      return Err(napi::Error::from_reason(format!(
        "WSACleanup failed with Winsock error {}",
        unsafe { WSAGetLastError().0 }
      )));
    }
  }
  state.leases -= 1;
  Ok(())
}

fn acquire_gdiplus() -> napi::Result<()> {
  let mut state = GDIPLUS_STATE
    .lock()
    .unwrap_or_else(|error| error.into_inner());
  if state.leases == 0 {
    let input = GdiplusStartupInput {
      GdiplusVersion: 1,
      DebugEventCallback: 0,
      SuppressBackgroundThread: false.into(),
      SuppressExternalCodecs: false.into(),
    };
    let mut token = 0usize;
    let status = unsafe { GdiplusStartup(&mut token, &input, std::ptr::null_mut()) };
    if status != GDIPLUS_OK {
      return Err(napi::Error::from_reason(format!(
        "GdiplusStartup failed with status {}",
        status.0
      )));
    }
    if token == 0 {
      return Err(napi::Error::from_reason(
        "GdiplusStartup returned an invalid token",
      ));
    }
    state.token = token;
  }
  state.leases = state
    .leases
    .checked_add(1)
    .ok_or_else(|| napi::Error::from_reason("GDI+ context count overflow"))?;
  Ok(())
}

fn release_gdiplus() -> napi::Result<()> {
  let mut state = GDIPLUS_STATE
    .lock()
    .unwrap_or_else(|error| error.into_inner());
  if state.leases == 0 {
    return Err(napi::Error::from_reason(
      "GDI+ subsystem context is not active",
    ));
  }
  state.leases -= 1;
  if state.leases == 0 {
    let token = std::mem::take(&mut state.token);
    unsafe { GdiplusShutdown(token) };
  }
  Ok(())
}

fn acquire_media_foundation() -> napi::Result<()> {
  let mut state = MEDIA_FOUNDATION_STATE
    .lock()
    .unwrap_or_else(|error| error.into_inner());
  if state.leases == 0 {
    unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
      .map_err(|error| napi::Error::from_reason(format!("MFStartup failed: {error}")))?;
  }
  state.leases = state
    .leases
    .checked_add(1)
    .ok_or_else(|| napi::Error::from_reason("Media Foundation context count overflow"))?;
  Ok(())
}

fn release_media_foundation() -> napi::Result<()> {
  let mut state = MEDIA_FOUNDATION_STATE
    .lock()
    .unwrap_or_else(|error| error.into_inner());
  if state.leases == 0 {
    return Err(napi::Error::from_reason(
      "Media Foundation subsystem context is not active",
    ));
  }
  if state.leases == 1 {
    unsafe { MFShutdown() }
      .map_err(|error| napi::Error::from_reason(format!("MFShutdown failed: {error}")))?;
  }
  state.leases -= 1;
  Ok(())
}

fn acquire_mapi_utilities() -> napi::Result<()> {
  let mut state = MAPI_UTILITIES_STATE
    .lock()
    .unwrap_or_else(|error| error.into_inner());
  if state.leases == 0 {
    let status = unsafe { ScInitMapiUtil(0) };
    if status != 0 {
      return Err(napi::Error::from_reason(format!(
        "ScInitMapiUtil(0) failed with SCODE 0x{:08x}",
        status as u32
      )));
    }
  }
  state.leases = state
    .leases
    .checked_add(1)
    .ok_or_else(|| napi::Error::from_reason("MAPI utility context count overflow"))?;
  Ok(())
}

fn release_mapi_utilities() -> napi::Result<()> {
  let mut state = MAPI_UTILITIES_STATE
    .lock()
    .unwrap_or_else(|error| error.into_inner());
  if state.leases == 0 {
    return Err(napi::Error::from_reason(
      "MAPI utility subsystem context is not active",
    ));
  }
  state.leases -= 1;
  if state.leases == 0 {
    unsafe { DeinitMapiUtil() };
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn winsock_context_is_counted_and_rejects_use_after_close() {
    let first = initialize("winsock").unwrap();
    let second = initialize("winsock").unwrap();
    require(&first, "winsock").unwrap();
    first.close().unwrap();
    assert!(require(&first, "winsock").is_err());
    require(&second, "winsock").unwrap();
    second.close().unwrap();
  }

  #[test]
  fn context_kind_mismatch_is_rejected() {
    let winsock = initialize("winsock").unwrap();
    let error = require(&winsock, "gdiplus").unwrap_err();
    assert!(error.reason.contains("received winsock"));
    winsock.close().unwrap();
  }

  #[test]
  fn call_guard_blocks_concurrent_close() {
    let winsock = std::sync::Arc::new(initialize("winsock").unwrap());
    let guard = call_guard(&winsock, "winsock").unwrap();
    let closing = std::sync::Arc::clone(&winsock);
    let (started_sender, started_receiver) = std::sync::mpsc::channel();
    let (finished_sender, finished_receiver) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
      started_sender.send(()).unwrap();
      let result = closing.close();
      finished_sender.send(result).unwrap();
    });

    started_receiver.recv().unwrap();
    assert!(finished_receiver
      .recv_timeout(std::time::Duration::from_millis(50))
      .is_err());
    drop(guard);
    finished_receiver.recv().unwrap().unwrap();
    thread.join().unwrap();
  }

  #[test]
  fn gdiplus_and_media_foundation_contexts_are_counted() {
    for subsystem in ["gdiplus", "mediaFoundation"] {
      let first = initialize(subsystem).unwrap();
      let second = initialize(subsystem).unwrap();
      first.close().unwrap();
      require(&second, subsystem).unwrap();
      second.close().unwrap();
    }
  }
}
