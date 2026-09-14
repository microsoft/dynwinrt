// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::tests::{assert_call_guard_blocks_close, lease_count, TEST_SERIAL};
use super::*;
use windows::core::HRESULT;
use windows::Win32::Graphics::GdiPlus::{GdiplusStartupOutput, Status};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
  Startup(u32, u32),
  Shutdown(usize),
  LastError,
}

struct State {
  kind: SubsystemKind,
  load_failed: bool,
  startup_status: i32,
  shutdown_status: i32,
  version: u16,
  token: usize,
  events: Vec<Event>,
}

static FIXTURES: Mutex<Vec<State>> = Mutex::new(Vec::new());

pub(super) struct Fixture(SubsystemKind);

impl Fixture {
  pub(super) fn new(kind: SubsystemKind) -> Self {
    assert_eq!(lease_count(kind), 0);
    let mut fixtures = FIXTURES.lock().unwrap();
    assert!(!fixtures.iter().any(|state| state.kind == kind));
    fixtures.push(State {
      kind,
      load_failed: false,
      startup_status: 0,
      shutdown_status: 0,
      version: WINSOCK_VERSION_2_2,
      token: 17,
      events: Vec::new(),
    });
    Self(kind)
  }

  fn update(&self, update: impl FnOnce(&mut State)) {
    let mut fixtures = FIXTURES.lock().unwrap();
    update(
      fixtures
        .iter_mut()
        .find(|state| state.kind == self.0)
        .unwrap(),
    );
  }

  fn events(&self) -> Vec<Event> {
    let fixtures = FIXTURES.lock().unwrap();
    fixtures
      .iter()
      .find(|state| state.kind == self.0)
      .unwrap()
      .events
      .clone()
  }
}

impl Drop for Fixture {
  fn drop(&mut self) {
    assert_eq!(lease_count(self.0), 0);
    if self.0 == SubsystemKind::Winsock {
      assert!(!WINSOCK_STATE.lock().unwrap().rollback_pending);
    }
    FIXTURES
      .lock()
      .unwrap()
      .retain(|state| state.kind != self.0);
  }
}

fn available(kind: SubsystemKind) -> Option<napi::Result<()>> {
  let fixtures = FIXTURES.lock().unwrap();
  let state = fixtures.iter().find(|state| state.kind == kind)?;
  Some(if state.load_failed {
    Err(napi::Error::from_reason(
      "0x8007007E: fixture lifecycle DLL is unavailable",
    ))
  } else {
    Ok(())
  })
}

fn native(kind: SubsystemKind, action: impl FnOnce(&mut State) -> i32) -> i32 {
  let mut fixtures = FIXTURES.lock().unwrap();
  action(
    fixtures
      .iter_mut()
      .find(|state| state.kind == kind)
      .unwrap(),
  )
}

unsafe extern "system" fn winsock_startup(version: u16, data: *mut WSADATA) -> i32 {
  native(SubsystemKind::Winsock, |state| {
    state.events.push(Event::Startup(u32::from(version), 0));
    if state.startup_status == 0 {
      unsafe {
        data.write(WSADATA::default());
        (*data).wVersion = state.version;
      }
    }
    state.startup_status
  })
}

unsafe extern "system" fn winsock_cleanup() -> i32 {
  native(SubsystemKind::Winsock, |state| {
    state.events.push(Event::Shutdown(0));
    state.shutdown_status
  })
}

unsafe extern "system" fn winsock_last_error() -> i32 {
  native(SubsystemKind::Winsock, |state| {
    state.events.push(Event::LastError);
    10093
  })
}

unsafe extern "system" fn gdi_startup(
  token: *mut usize,
  input: *const GdiplusStartupInput,
  _output: *mut GdiplusStartupOutput,
) -> Status {
  Status(native(SubsystemKind::GdiPlus, |state| {
    state
      .events
      .push(Event::Startup(unsafe { (*input).GdiplusVersion }, 0));
    if state.startup_status == 0 {
      unsafe { token.write(state.token) };
    }
    state.startup_status
  }))
}

unsafe extern "system" fn gdi_shutdown(token: usize) {
  native(SubsystemKind::GdiPlus, |state| {
    state.events.push(Event::Shutdown(token));
    0
  });
}

unsafe extern "system" fn media_startup(version: u32, flags: u32) -> HRESULT {
  HRESULT(native(SubsystemKind::MediaFoundation, |state| {
    state.events.push(Event::Startup(version, flags));
    state.startup_status
  }))
}

unsafe extern "system" fn media_shutdown() -> HRESULT {
  HRESULT(native(SubsystemKind::MediaFoundation, |state| {
    state.events.push(Event::Shutdown(0));
    state.shutdown_status
  }))
}

pub(super) fn winsock_functions() -> Option<napi::Result<WinsockFunctions>> {
  Some(
    available(SubsystemKind::Winsock)?.map(|()| WinsockFunctions {
      startup: winsock_startup,
      cleanup: winsock_cleanup,
      last_error: winsock_last_error,
    }),
  )
}

pub(super) fn gdiplus_functions() -> Option<napi::Result<GdiPlusFunctions>> {
  Some(
    available(SubsystemKind::GdiPlus)?.map(|()| GdiPlusFunctions {
      startup: gdi_startup,
      shutdown: gdi_shutdown,
    }),
  )
}

pub(super) fn media_foundation_functions() -> Option<napi::Result<MediaFoundationFunctions>> {
  Some(
    available(SubsystemKind::MediaFoundation)?.map(|()| MediaFoundationFunctions {
      startup: media_startup,
      shutdown: media_shutdown,
    }),
  )
}

#[test]
fn unavailable_lifecycle_tables_publish_no_context_and_allow_retry() {
  let _serial = TEST_SERIAL.lock().unwrap();
  for kind in [
    SubsystemKind::Winsock,
    SubsystemKind::GdiPlus,
    SubsystemKind::MediaFoundation,
  ] {
    let fixture = Fixture::new(kind);
    fixture.update(|state| state.load_failed = true);
    assert!(initialize(kind.name())
      .err()
      .unwrap()
      .reason
      .contains("unavailable"));
    assert_eq!(lease_count(kind), 0);
    assert!(fixture.events().is_empty());
    fixture.update(|state| state.load_failed = false);
    let context = initialize(kind.name()).unwrap();
    context.close().unwrap();
    assert_eq!(fixture.events().len(), 2);
  }
}

#[test]
fn startup_failure_is_retryable_and_multiple_contexts_share_one_activation() {
  let _serial = TEST_SERIAL.lock().unwrap();
  for kind in [
    SubsystemKind::Winsock,
    SubsystemKind::GdiPlus,
    SubsystemKind::MediaFoundation,
  ] {
    let fixture = Fixture::new(kind);
    fixture.update(|state| {
      state.startup_status = if kind == SubsystemKind::MediaFoundation {
        0x80004005u32 as i32
      } else {
        1
      };
    });
    assert!(initialize(kind.name()).is_err());
    assert_eq!(lease_count(kind), 0);
    assert_eq!(fixture.events().len(), 1);
    fixture.update(|state| state.startup_status = 0);
    let first = initialize(kind.name()).unwrap();
    let second = initialize(kind.name()).unwrap();
    first.close().unwrap();
    assert_eq!(lease_count(kind), 1);
    require(&second, kind.name()).unwrap();
    assert_eq!(fixture.events().len(), 2);
    second.close().unwrap();
    assert_eq!(fixture.events().len(), 3);
    let restarted = initialize(kind.name()).unwrap();
    restarted.close().unwrap();
    assert_eq!(fixture.events().len(), 5);
  }
}

#[test]
fn final_shutdown_failure_keeps_context_active_and_retryable() {
  let _serial = TEST_SERIAL.lock().unwrap();
  for kind in [SubsystemKind::Winsock, SubsystemKind::MediaFoundation] {
    let fixture = Fixture::new(kind);
    let context = initialize(kind.name()).unwrap();
    fixture.update(|state| state.shutdown_status = 0x80004005u32 as i32);
    let failed = context.close().is_err();
    let still_active = !context.closed() && lease_count(kind) == 1;
    let usable = require(&context, kind.name()).is_ok();
    fixture.update(|state| state.shutdown_status = 0);
    context.close().unwrap();
    assert!(failed && still_active && usable);
    assert!(context.closed());
    assert_eq!(lease_count(kind), 0);
    if kind == SubsystemKind::Winsock {
      assert!(fixture.events().contains(&Event::LastError));
    }
  }
}

#[test]
fn winsock_version_rollback_is_reported_and_retried_before_startup() {
  let _serial = TEST_SERIAL.lock().unwrap();
  let fixture = Fixture::new(SubsystemKind::Winsock);
  fixture.update(|state| {
    state.version = 0x0101;
    state.shutdown_status = -1;
  });
  let failed = initialize("winsock").err().unwrap();
  assert!(failed.reason.contains("rollback failed"));
  assert_eq!(lease_count(SubsystemKind::Winsock), 0);
  assert!(WINSOCK_STATE.lock().unwrap().rollback_pending);
  assert!(initialize("winsock")
    .err()
    .unwrap()
    .reason
    .contains("still pending"));
  assert_eq!(
    fixture.events(),
    [
      Event::Startup(0x0202, 0),
      Event::Shutdown(0),
      Event::LastError,
      Event::Shutdown(0),
      Event::LastError,
    ]
  );
  fixture.update(|state| {
    state.version = WINSOCK_VERSION_2_2;
    state.shutdown_status = 0;
  });
  let context = initialize("winsock").unwrap();
  assert!(!WINSOCK_STATE.lock().unwrap().rollback_pending);
  context.close().unwrap();
  assert_eq!(
    &fixture.events()[5..],
    [
      Event::Shutdown(0),
      Event::Startup(0x0202, 0),
      Event::Shutdown(0),
    ]
  );
}

#[test]
fn invalid_gdiplus_token_is_not_published_or_guessed_for_shutdown() {
  let _serial = TEST_SERIAL.lock().unwrap();
  let fixture = Fixture::new(SubsystemKind::GdiPlus);
  fixture.update(|state| state.token = 0);
  assert!(initialize("gdiplus")
    .err()
    .unwrap()
    .reason
    .contains("invalid token"));
  assert_eq!(lease_count(SubsystemKind::GdiPlus), 0);
  assert_eq!(fixture.events(), [Event::Startup(1, 0)]);
  fixture.update(|state| state.token = 17);
  initialize("gdiplus").unwrap().close().unwrap();
  assert_eq!(
    fixture.events(),
    [
      Event::Startup(1, 0),
      Event::Startup(1, 0),
      Event::Shutdown(17),
    ]
  );
}

#[test]
fn all_optional_lifecycle_tables_keep_call_guards_through_shutdown() {
  let _serial = TEST_SERIAL.lock().unwrap();
  for kind in [
    SubsystemKind::Winsock,
    SubsystemKind::GdiPlus,
    SubsystemKind::MediaFoundation,
  ] {
    let fixture = Fixture::new(kind);
    assert_call_guard_blocks_close(kind.name());
    assert_eq!(fixture.events().len(), 2);
  }
}
