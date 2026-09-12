// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::mem::MaybeUninit;
use std::sync::{LazyLock, Mutex, MutexGuard};

use windows::core::{w, PCSTR};
use windows::Win32::Graphics::GdiPlus::{
  GdiplusShutdown, GdiplusStartup, GdiplusStartupInput, Ok as GDIPLUS_OK,
};
use windows::Win32::Media::MediaFoundation::{MFShutdown, MFStartup, MFSTARTUP_FULL, MF_VERSION};
use windows::Win32::Networking::WinSock::{WSACleanup, WSAGetLastError, WSAStartup, WSADATA};
use windows::Win32::{
  Foundation::{FreeLibrary, HMODULE},
  System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32},
};

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

struct MapiModule(usize);

impl Drop for MapiModule {
  fn drop(&mut self) {
    if let Err(error) = unsafe { FreeLibrary(HMODULE(self.0 as *mut std::ffi::c_void)) } {
      eprintln!("[dynwinrt] MAPI utility module cleanup failed: {error}");
    }
  }
}

#[derive(Clone, Copy)]
struct MapiUtilityFunctions {
  initialize: unsafe extern "system" fn(u32) -> i32,
  deinitialize: unsafe extern "system" fn(),
}

struct MapiUtilities {
  functions: MapiUtilityFunctions,
  _module: MapiModule,
}

#[cfg(target_arch = "x86")]
const MAPI_INIT_EXPORT: &std::ffi::CStr = c"ScInitMapiUtil@4";
#[cfg(target_arch = "x86")]
const MAPI_DEINIT_EXPORT: &std::ffi::CStr = c"DeinitMapiUtil@0";
#[cfg(not(target_arch = "x86"))]
const MAPI_INIT_EXPORT: &std::ffi::CStr = c"ScInitMapiUtil";
#[cfg(not(target_arch = "x86"))]
const MAPI_DEINIT_EXPORT: &std::ffi::CStr = c"DeinitMapiUtil";

fn system_mapi_utilities() -> napi::Result<&'static MapiUtilities> {
  static API: LazyLock<Result<MapiUtilities, String>> = LazyLock::new(|| {
    let module = unsafe { LoadLibraryExW(w!("mapi32.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
      .map_err(|error| format!("MAPI utilities are unavailable: {error}"))?;
    let owner = MapiModule(module.0 as usize);
    let initialize = unsafe { GetProcAddress(module, PCSTR(MAPI_INIT_EXPORT.as_ptr().cast())) }
      .ok_or_else(|| {
        format!(
          "System MAPI32.dll has no {}",
          MAPI_INIT_EXPORT.to_string_lossy()
        )
      })?;
    let deinitialize = unsafe { GetProcAddress(module, PCSTR(MAPI_DEINIT_EXPORT.as_ptr().cast())) }
      .ok_or_else(|| {
        format!(
          "System MAPI32.dll has no {}",
          MAPI_DEINIT_EXPORT.to_string_lossy()
        )
      })?;
    // The exact exported signatures are SCODE(ULONG) and void(), both WINAPI.
    // Retaining the module keeps these immutable function pointers valid.
    Ok(MapiUtilities {
      functions: MapiUtilityFunctions {
        initialize: unsafe {
          std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            unsafe extern "system" fn(u32) -> i32,
          >(initialize)
        },
        deinitialize: unsafe {
          std::mem::transmute::<unsafe extern "system" fn() -> isize, unsafe extern "system" fn()>(
            deinitialize,
          )
        },
      },
      _module: owner,
    })
  });
  API
    .as_ref()
    .map_err(|message| napi::Error::from_reason(message.clone()))
}

fn mapi_utilities() -> napi::Result<MapiUtilityFunctions> {
  #[cfg(test)]
  if let Some(functions) = tests::mapi_utility_fixture() {
    return functions;
  }
  Ok(system_mapi_utilities()?.functions)
}

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
    let status = unsafe { (mapi_utilities()?.initialize)(0) };
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
    unsafe { (mapi_utilities()?.deinitialize)() };
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::{
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicI32, Ordering},
    time::{Duration, Instant},
  };

  static TEST_SERIAL: Mutex<()> = Mutex::new(());
  static MAPI_FIXTURE_ENABLED: AtomicBool = AtomicBool::new(false);
  static MAPI_FIXTURE_LOAD_FAILURE: AtomicBool = AtomicBool::new(false);
  static MAPI_FIXTURE_STATUS: AtomicI32 = AtomicI32::new(0);
  static MAPI_FIXTURE_EVENTS: Mutex<Vec<MapiEvent>> = Mutex::new(Vec::new());

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  enum MapiEvent {
    Initialize(u32),
    Deinitialize,
  }

  struct MapiFixture;

  impl MapiFixture {
    fn new(status: i32) -> Self {
      assert_eq!(lease_count(SubsystemKind::MapiUtilities), 0);
      assert!(!MAPI_FIXTURE_ENABLED.swap(true, Ordering::SeqCst));
      MAPI_FIXTURE_LOAD_FAILURE.store(false, Ordering::SeqCst);
      MAPI_FIXTURE_STATUS.store(status, Ordering::SeqCst);
      MAPI_FIXTURE_EVENTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
      Self
    }

    fn events(&self) -> Vec<MapiEvent> {
      MAPI_FIXTURE_EVENTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
    }
  }

  impl Drop for MapiFixture {
    fn drop(&mut self) {
      assert_eq!(lease_count(SubsystemKind::MapiUtilities), 0);
      MAPI_FIXTURE_ENABLED.store(false, Ordering::SeqCst);
    }
  }

  unsafe extern "system" fn fixture_mapi_initialize(flags: u32) -> i32 {
    MAPI_FIXTURE_EVENTS
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .push(MapiEvent::Initialize(flags));
    MAPI_FIXTURE_STATUS.load(Ordering::SeqCst)
  }

  unsafe extern "system" fn fixture_mapi_deinitialize() {
    MAPI_FIXTURE_EVENTS
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .push(MapiEvent::Deinitialize);
  }

  pub(super) fn mapi_utility_fixture() -> Option<napi::Result<MapiUtilityFunctions>> {
    if !MAPI_FIXTURE_ENABLED.load(Ordering::SeqCst) {
      return None;
    }
    if MAPI_FIXTURE_LOAD_FAILURE.load(Ordering::SeqCst) {
      return Some(Err(napi::Error::from_reason(
        "MAPI utility fixture exports are unavailable",
      )));
    }
    Some(Ok(MapiUtilityFunctions {
      initialize: fixture_mapi_initialize,
      deinitialize: fixture_mapi_deinitialize,
    }))
  }

  fn isolated_native_test<F: Fn()>(test: F) -> bool {
    const CHILD: &str = "DYNWINRT_SUBSYSTEM_TEST_CHILD";
    let qualified = std::any::type_name_of_val(&test);
    let (_, name) = qualified
      .split_once("::")
      .expect("native subsystem test has a qualified Rust name");
    if std::env::var(CHILD).as_deref() == Ok(name) {
      return false;
    }
    let mut child = Command::new(std::env::current_exe().unwrap())
      .arg(name)
      .args([
        "--exact",
        "--nocapture",
        "--test-threads=1",
        "--include-ignored",
      ])
      .env(CHILD, name)
      .stdout(Stdio::inherit())
      .stderr(Stdio::inherit())
      .spawn()
      .expect("start isolated native subsystem test");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
      if let Some(status) = child.try_wait().expect("check subsystem test process") {
        assert!(status.success(), "{name}: child failed with {status}");
        return true;
      }
      if Instant::now() >= deadline {
        child.kill().expect("terminate this subsystem test process");
        child.wait().expect("reap subsystem test process");
        panic!("{name}: native subsystem lifecycle exceeded 60 seconds; see the last stage above");
      }
      std::thread::sleep(Duration::from_millis(20));
    }
  }

  fn live_context(name: &str) -> DynWin32SubsystemContext {
    eprintln!("[subsystem-test] initialize {name}");
    let context = initialize(name).unwrap();
    eprintln!("[subsystem-test] initialized {name}");
    context
  }

  fn close_context(context: &DynWin32SubsystemContext) {
    eprintln!("[subsystem-test] close {}", context.subsystem());
    context.close().unwrap();
    eprintln!("[subsystem-test] closed {}", context.subsystem());
  }

  fn drop_context(context: DynWin32SubsystemContext) {
    let name = context.subsystem();
    eprintln!("[subsystem-test] drop {name}");
    drop(context);
    eprintln!("[subsystem-test] dropped {name}");
  }

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

  fn assert_call_guard_blocks_close(subsystem: &str) {
    let context = std::sync::Arc::new(initialize(subsystem).unwrap());
    let guard = call_guard(&context, subsystem).unwrap();
    let closing = std::sync::Arc::clone(&context);
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
      context.closed.try_lock(),
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
  fn call_guard_blocks_concurrent_close() {
    let _serial = TEST_SERIAL
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    assert_call_guard_blocks_close("winsock");
  }

  #[test]
  fn gdiplus_media_foundation_and_mapi_utility_contexts_are_counted() {
    if isolated_native_test(gdiplus_media_foundation_and_mapi_utility_contexts_are_counted) {
      return;
    }
    let _serial = TEST_SERIAL
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    let mapi = MapiFixture::new(0);
    for subsystem in ["gdiplus", "mediaFoundation", "mapiUtilities"] {
      let first = live_context(subsystem);
      let second = live_context(subsystem);
      close_context(&first);
      require(&second, subsystem).unwrap();
      close_context(&second);
    }
    assert_eq!(
      mapi.events(),
      [MapiEvent::Initialize(0), MapiEvent::Deinitialize]
    );
  }

  #[test]
  fn subsystem_counts_remain_independent_across_kind_and_alias_closure() {
    if isolated_native_test(subsystem_counts_remain_independent_across_kind_and_alias_closure) {
      return;
    }
    let _serial = TEST_SERIAL
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    let mapi_fixture = MapiFixture::new(0);
    let winsock = live_context("winsock");
    let winsock_alias = live_context("winsock");
    let gdi = live_context("gdi+");
    let media = live_context("media_foundation");
    let mapi = live_context("mapi_utilities");
    close_context(&winsock);
    require(&winsock_alias, "winsock").unwrap();
    require(&gdi, "gdiplus").unwrap();
    require(&media, "mediaFoundation").unwrap();
    require(&mapi, "mapiUtilities").unwrap();
    close_context(&gdi);
    require(&winsock_alias, "winsock").unwrap();
    require(&media, "mediaFoundation").unwrap();
    close_context(&winsock_alias);
    require(&media, "mediaFoundation").unwrap();
    close_context(&media);
    require(&mapi, "mapiUtilities").unwrap();
    close_context(&mapi);
    for context in [&winsock, &winsock_alias, &gdi, &media, &mapi] {
      assert!(context.closed());
      context.close().unwrap();
    }
    assert_eq!(
      mapi_fixture.events(),
      [MapiEvent::Initialize(0), MapiEvent::Deinitialize]
    );
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
    if isolated_native_test(dropping_live_contexts_returns_each_subsystems_exact_lease_count) {
      return;
    }
    let _serial = TEST_SERIAL
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    let mapi = MapiFixture::new(0);
    for kind in [
      SubsystemKind::Winsock,
      SubsystemKind::GdiPlus,
      SubsystemKind::MediaFoundation,
      SubsystemKind::MapiUtilities,
    ] {
      let before = lease_count(kind);
      let first = live_context(kind.name());
      let last = live_context(kind.name());
      assert_eq!(lease_count(kind), before + 2);
      drop_context(first);
      assert_eq!(lease_count(kind), before + 1);
      require(&last, kind.name()).unwrap();
      drop_context(last);
      assert_eq!(lease_count(kind), before);
    }
    assert_eq!(
      mapi.events(),
      [MapiEvent::Initialize(0), MapiEvent::Deinitialize]
    );
  }

  #[test]
  fn mapi_initialization_failure_does_not_publish_a_lease_and_can_be_retried() {
    let _serial = TEST_SERIAL
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    let mapi = MapiFixture::new(0x8000_4005_u32 as i32);
    let error = initialize("mapiUtilities").err().unwrap();
    assert!(error.reason.contains("SCODE 0x80004005"));
    assert_eq!(lease_count(SubsystemKind::MapiUtilities), 0);
    assert_eq!(mapi.events(), [MapiEvent::Initialize(0)]);

    MAPI_FIXTURE_STATUS.store(0, Ordering::SeqCst);
    let context = initialize("mapi_utilities").unwrap();
    require(&context, "mapiUtilities").unwrap();
    assert_eq!(lease_count(SubsystemKind::MapiUtilities), 1);
    drop(context);
    assert_eq!(
      mapi.events(),
      [
        MapiEvent::Initialize(0),
        MapiEvent::Initialize(0),
        MapiEvent::Deinitialize
      ]
    );
  }

  #[test]
  fn missing_mapi_exports_do_not_publish_or_cleanup_a_lease() {
    let _serial = TEST_SERIAL
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    let mapi = MapiFixture::new(0);
    MAPI_FIXTURE_LOAD_FAILURE.store(true, Ordering::SeqCst);
    let error = initialize("mapiUtilities").err().unwrap();
    assert!(error.reason.contains("exports are unavailable"));
    assert_eq!(lease_count(SubsystemKind::MapiUtilities), 0);
    assert!(mapi.events().is_empty());
  }

  #[test]
  fn mapi_final_close_is_idempotent_and_allows_reinitialization() {
    let _serial = TEST_SERIAL
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    let mapi = MapiFixture::new(0);
    let first = initialize("mapiUtilities").unwrap();
    let second = initialize("mapi_utilities").unwrap();
    assert_eq!(lease_count(SubsystemKind::MapiUtilities), 2);
    first.close().unwrap();
    first.close().unwrap();
    assert_eq!(lease_count(SubsystemKind::MapiUtilities), 1);
    assert_eq!(mapi.events(), [MapiEvent::Initialize(0)]);
    require(&second, "mapiUtilities").unwrap();
    drop(second);
    assert_eq!(lease_count(SubsystemKind::MapiUtilities), 0);
    assert!(release_mapi_utilities().is_err());
    let reopened = initialize("mapiUtilities").unwrap();
    reopened.close().unwrap();
    drop(reopened);
    assert_eq!(
      mapi.events(),
      [
        MapiEvent::Initialize(0),
        MapiEvent::Deinitialize,
        MapiEvent::Initialize(0),
        MapiEvent::Deinitialize
      ]
    );
  }

  #[test]
  fn mapi_call_guard_blocks_final_cleanup_on_another_thread() {
    let _serial = TEST_SERIAL
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    let mapi = MapiFixture::new(0);
    assert_call_guard_blocks_close("mapiUtilities");
    assert_eq!(
      mapi.events(),
      [MapiEvent::Initialize(0), MapiEvent::Deinitialize]
    );
  }

  #[test]
  #[ignore = "requires a configured Extended MAPI provider of the process architecture"]
  fn mapi_utility_contexts_use_installed_provider() {
    if isolated_native_test(mapi_utility_contexts_use_installed_provider) {
      return;
    }
    let _serial = TEST_SERIAL
      .lock()
      .unwrap_or_else(|error| error.into_inner());
    let first = live_context("mapiUtilities");
    let last = live_context("mapi_utilities");
    assert_eq!(lease_count(SubsystemKind::MapiUtilities), 2);
    close_context(&first);
    assert_eq!(lease_count(SubsystemKind::MapiUtilities), 1);
    require(&last, "mapiUtilities").unwrap();
    drop_context(last);
    assert_eq!(lease_count(SubsystemKind::MapiUtilities), 0);
    let reopened = live_context("mapiUtilities");
    close_context(&reopened);
    close_context(&reopened);
    assert_eq!(lease_count(SubsystemKind::MapiUtilities), 0);
  }

  #[test]
  fn mapi_utility_exports_resolve_for_the_process_architecture() {
    assert!(!MAPI_INIT_EXPORT.to_bytes().is_empty());
    assert!(!MAPI_DEINIT_EXPORT.to_bytes().is_empty());
    system_mapi_utilities().expect("resolve the exact system utility exports");
  }
}
