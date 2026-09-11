// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::mem::MaybeUninit;
use std::sync::{LazyLock, Mutex, MutexGuard};

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

pub struct DynWin32SubsystemContext {
  kind: SubsystemKind,
  closed: Mutex<bool>,
}
super::win32::boundary::carrier!(DynWin32SubsystemContext, 6, "DynWin32SubsystemContext");

pub(super) struct SubsystemCallGuard<'a> {
  _closed: MutexGuard<'a, bool>,
}

impl DynWin32SubsystemContext {
  pub fn subsystem(&self) -> &'static str {
    self.kind.name()
  }

  pub fn closed(&self) -> bool {
    *self
      .closed
      .lock()
      .unwrap_or_else(|error| error.into_inner())
  }

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

  static TEST_SERIAL: Mutex<()> = Mutex::new(());

  fn lease_count(kind: SubsystemKind) -> usize {
    match kind {
      SubsystemKind::Winsock => WINSOCK_STATE.lock().unwrap().leases,
      SubsystemKind::GdiPlus => GDIPLUS_STATE.lock().unwrap().leases,
      SubsystemKind::MediaFoundation => MEDIA_FOUNDATION_STATE.lock().unwrap().leases,
      SubsystemKind::MapiUtilities => MAPI_UTILITIES_STATE.lock().unwrap().leases,
    }
  }

  #[test]
  fn winsock_context_is_counted_and_rejects_use_after_close() {
    let _serial = TEST_SERIAL
      .lock()
      .unwrap_or_else(|error| error.into_inner());
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
    let _serial = TEST_SERIAL
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    let winsock = initialize("winsock").unwrap();
    let error = require(&winsock, "gdiplus").unwrap_err();
    assert!(error.reason.contains("received winsock"));
    winsock.close().unwrap();
  }

  #[test]
  fn call_guard_blocks_concurrent_close() {
    let _serial = TEST_SERIAL
      .lock()
      .unwrap_or_else(|error| error.into_inner());
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

    started_receiver
      .recv_timeout(std::time::Duration::from_secs(5))
      .unwrap();
    assert!(matches!(
      winsock.closed.try_lock(),
      Err(std::sync::TryLockError::WouldBlock)
    ));
    assert!(matches!(
      finished_receiver.try_recv(),
      Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    drop(guard);
    finished_receiver
      .recv_timeout(std::time::Duration::from_secs(5))
      .unwrap()
      .unwrap();
    thread.join().unwrap();
  }

  #[test]
  fn gdiplus_media_foundation_and_mapi_utility_contexts_are_counted() {
    let _serial = TEST_SERIAL
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    for subsystem in ["gdiplus", "mediaFoundation", "mapiUtilities"] {
      let first = initialize(subsystem).unwrap();
      let second = initialize(subsystem).unwrap();
      first.close().unwrap();
      require(&second, subsystem).unwrap();
      second.close().unwrap();
    }
  }

  #[test]
  fn subsystem_counts_remain_independent_across_kind_and_alias_closure() {
    let _serial = TEST_SERIAL
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    let winsock = initialize("winsock").unwrap();
    let winsock_alias = initialize("winsock").unwrap();
    let gdi = initialize("gdi+").unwrap();
    let media = initialize("media_foundation").unwrap();
    let mapi = initialize("mapi_utilities").unwrap();
    winsock.close().unwrap();
    require(&winsock_alias, "winsock").unwrap();
    require(&gdi, "gdiplus").unwrap();
    require(&media, "mediaFoundation").unwrap();
    require(&mapi, "mapiUtilities").unwrap();
    gdi.close().unwrap();
    require(&winsock_alias, "winsock").unwrap();
    require(&media, "mediaFoundation").unwrap();
    winsock_alias.close().unwrap();
    require(&media, "mediaFoundation").unwrap();
    media.close().unwrap();
    require(&mapi, "mapiUtilities").unwrap();
    mapi.close().unwrap();
    for context in [&winsock, &winsock_alias, &gdi, &media, &mapi] {
      assert!(context.closed());
      context.close().unwrap();
    }
  }

  #[test]
  fn every_closed_context_rejects_dispatch_and_unknown_kinds_fail_closed() {
    let _serial = TEST_SERIAL
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    for kind in [
      SubsystemKind::Winsock,
      SubsystemKind::GdiPlus,
      SubsystemKind::MediaFoundation,
      SubsystemKind::MapiUtilities,
    ] {
      let context = DynWin32SubsystemContext {
        kind,
        closed: Mutex::new(true),
      };
      assert!(call_guard(&context, kind.name())
        .err()
        .unwrap()
        .reason
        .contains("closed"));
      context.close().unwrap();
      assert!(context.closed());
    }
    for name in ["", "winsock2", "mapi", "gdiplusSuffix"] {
      assert!(SubsystemKind::parse(name).is_err());
    }
  }

  #[test]
  fn dropping_live_contexts_returns_each_subsystems_exact_lease_count() {
    let _serial = TEST_SERIAL
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    for kind in [
      SubsystemKind::Winsock,
      SubsystemKind::GdiPlus,
      SubsystemKind::MediaFoundation,
      SubsystemKind::MapiUtilities,
    ] {
      let before = lease_count(kind);
      let first = initialize(kind.name()).unwrap();
      let last = initialize(kind.name()).unwrap();
      assert_eq!(lease_count(kind), before + 2);
      drop(first);
      assert_eq!(lease_count(kind), before + 1);
      require(&last, kind.name()).unwrap();
      drop(last);
      assert_eq!(lease_count(kind), before);
    }
  }
}
