// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Node-thread delivery for the native Win32 I/O engine.

use std::{
  cell::RefCell,
  collections::HashMap,
  marker::PhantomData,
  rc::Rc,
  sync::atomic::{AtomicU64, Ordering},
};

use dynwinrt::win32::io::{IoCancellation, IoCompletion, IoKind, IocpRuntime, PreparedIo};
use napi::{
  bindgen_prelude::{Buffer, Function, ToNapiValue, Unknown},
  sys, JsValue,
};

use super::{boundary, storage, DynWin32Resource};
use crate::managed_tsfn::ManagedTsfn;

static NEXT_PENDING_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
  static PENDING: RefCell<HashMap<u64, JsPendingIo>> = RefCell::new(HashMap::new());
}

struct JsPendingIo {
  buffer: Buffer,
  buffer_len: usize,
  buffer_pointer: usize,
  env: usize,
  _owner_thread: PhantomData<Rc<()>>,
}

struct JsPreparedIo {
  native: PreparedIo,
  pending: JsPendingIo,
}

pub struct DynWin32OverlappedOperation {
  task: Option<JsPreparedIo>,
  cancellation: IoCancellation,
}

boundary::carrier!(
  DynWin32OverlappedOperation,
  7,
  "DynWin32OverlappedOperation"
);

impl DynWin32OverlappedOperation {
  pub fn cancel(&self) {
    self.cancellation.cancel();
  }

  pub fn start(&mut self, callback: Function<'static, (), ()>) -> napi::Result<()> {
    let task = self
      .task
      .take()
      .ok_or_else(|| napi::Error::from_reason("OVERLAPPED operation was already started"))?;
    let env = callback.value().env;
    if task.pending.env != env as usize {
      return Err(napi::Error::from_reason(
        "OVERLAPPED operation belongs to a different Node environment",
      ));
    }
    let id = NEXT_PENDING_ID
      .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
        next.checked_add(1)
      })
      .map_err(|_| napi::Error::from_reason("OVERLAPPED delivery identifiers were exhausted"))?;
    let closing = self.cancellation.clone();
    let completion = ManagedTsfn::create(
      env,
      callback.raw(),
      1,
      false,
      move |completion: IoCompletion, env| completion_arguments(id, completion, env),
      Some(Box::new(move |_| {
        closing.cancel();
        // This finalizer runs on the owning environment thread. In particular,
        // an IOCP worker never drops a Buffer or touches its backing store.
        drop(take_pending(id));
      })),
    )?;
    PENDING.with(|pending| {
      let mut pending = pending.borrow_mut();
      pending
        .try_reserve(1)
        .map_err(|_| napi::Error::from_reason("Unable to retain OVERLAPPED delivery state"))?;
      pending.insert(id, task.pending);
      Ok::<_, napi::Error>(())
    })?;
    let result = task.native.submit(move |native| {
      // The payload owns only Rust data. A rejected notification drops it here
      // after native completion; JS references remain in the owner-thread map.
      let status = completion.call(native);
      if !matches!(status, napi::Status::Ok | napi::Status::Closing) {
        eprintln!("[dynwinrt] OVERLAPPED completion notification failed: {status}");
      }
    });
    if let Err(error) = result {
      drop(take_pending(id));
      return Err(napi::Error::from_reason(error.to_string()));
    }
    Ok(())
  }
}

pub(super) fn prepare(
  kind: IoKind,
  file: &DynWin32Resource,
  buffer: Unknown,
  offset: Option<Unknown>,
) -> napi::Result<DynWin32OverlappedOperation> {
  let env = buffer.value().env as usize;
  let buffer = storage::native_buffer(buffer)?;
  let offset = offset
    .as_ref()
    .map(storage::unsigned64)
    .transpose()?
    .unwrap_or(0);
  let runtime =
    IocpRuntime::shared().map_err(|error| napi::Error::from_reason(error.to_string()))?;
  let native = match kind {
    IoKind::Read => runtime.prepare_read(&file.0, buffer.len(), offset),
    IoKind::Write => runtime.prepare_write(&file.0, &buffer, offset),
  }
  .map_err(|error| napi::Error::from_reason(error.to_string()))?;
  let cancellation = native.cancellation();
  Ok(DynWin32OverlappedOperation {
    task: Some(JsPreparedIo {
      native,
      pending: JsPendingIo {
        buffer_len: buffer.len(),
        buffer_pointer: buffer.as_ptr() as usize,
        buffer,
        env,
        _owner_thread: PhantomData,
      },
    }),
    cancellation,
  })
}

fn take_pending(id: u64) -> Option<JsPendingIo> {
  PENDING.with(|pending| pending.borrow_mut().remove(&id))
}

impl JsPendingIo {
  fn resolve(
    self,
    kind: IoKind,
    native: &[u8],
    env: sys::napi_env,
    output: u32,
  ) -> napi::Result<u32> {
    if self.env != env as usize {
      return Err(napi::Error::from_reason(
        "OVERLAPPED result belongs to a different Node environment",
      ));
    }
    let transferred = usize::try_from(output)
      .map_err(|_| napi::Error::from_reason("OVERLAPPED result exceeds usize"))?;
    if transferred > self.buffer_len || transferred > native.len() {
      return Err(napi::Error::from_reason(
        "OVERLAPPED result exceeds the original Buffer length",
      ));
    }
    if kind == IoKind::Read {
      let raw = unsafe { Buffer::to_napi_value(env, self.buffer) }?;
      let mut is_buffer = false;
      napi::check_status!(
        unsafe { sys::napi_is_buffer(env, raw, &mut is_buffer) },
        "Failed to revalidate OVERLAPPED read Buffer"
      )?;
      if !is_buffer {
        return Err(napi::Error::from_reason(
          "OVERLAPPED read Buffer is no longer a Node Buffer",
        ));
      }
      let info = storage::buffer_info(env, raw).map_err(|error| {
        napi::Error::new(
          error.status,
          format!(
            "OVERLAPPED read Buffer backing ArrayBuffer was detached or changed: {}",
            error.reason
          ),
        )
      })?;
      if info.length != self.buffer_len
        || (info.length != 0 && info.pointer as usize != self.buffer_pointer)
      {
        return Err(napi::Error::from_reason(
          "OVERLAPPED read Buffer backing ArrayBuffer was detached or changed",
        ));
      }
      if transferred != 0 {
        unsafe {
          std::ptr::copy_nonoverlapping(native.as_ptr(), info.pointer, transferred);
        }
      }
    }
    Ok(output)
  }
}

fn completion_arguments(
  id: u64,
  native: IoCompletion,
  env: sys::napi_env,
) -> napi::Result<Vec<sys::napi_value>> {
  let result = (|| {
    let pending = take_pending(id)
      .ok_or_else(|| napi::Error::from_reason("OVERLAPPED delivery state is unavailable"))?;
    let output = native
      .result()
      .map_err(|error| napi::Error::from_reason(error.to_string()))?;
    pending.resolve(native.kind(), native.buffer(), env, output)
  })();
  match result {
    Ok(output) => {
      let mut null = std::ptr::null_mut();
      napi::check_status!(
        unsafe { sys::napi_get_null(env, &mut null) },
        "Failed to create OVERLAPPED completion null"
      )?;
      let output = unsafe { u32::to_napi_value(env, output) }?;
      Ok(vec![null, output])
    }

    Err(error) => {
      let mut message = std::ptr::null_mut();
      napi::check_status!(
        unsafe {
          sys::napi_create_string_utf8(
            env,
            error.reason.as_ptr().cast(),
            error.reason.len() as isize,
            &mut message,
          )
        },
        "Failed to create OVERLAPPED completion error message"
      )?;
      let mut error = std::ptr::null_mut();
      napi::check_status!(
        unsafe { sys::napi_create_error(env, std::ptr::null_mut(), message, &mut error) },
        "Failed to create OVERLAPPED completion error"
      )?;
      Ok(vec![error])
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn pending() -> JsPendingIo {
    let buffer = Buffer::from(vec![1, 2, 3]);
    JsPendingIo {
      buffer_len: buffer.len(),
      buffer_pointer: buffer.as_ptr() as usize,
      buffer,
      env: 0,
      _owner_thread: PhantomData,
    }
  }

  #[test]
  fn completed_write_and_invalid_lengths_retire_without_native_buffer_access() {
    assert_eq!(
      pending()
        .resolve(IoKind::Write, &[1, 2, 3], std::ptr::null_mut(), 2)
        .unwrap(),
      2
    );
    for kind in [IoKind::Read, IoKind::Write] {
      assert!(pending()
        .resolve(kind, &[1, 2, 3], std::ptr::null_mut(), 4)
        .unwrap_err()
        .reason
        .contains("original Buffer"));
      assert!(pending()
        .resolve(kind, &[1], std::ptr::null_mut(), 2)
        .is_err());
    }
  }

  #[test]
  fn owner_thread_pending_state_can_only_be_taken_once() {
    let id = NEXT_PENDING_ID.fetch_add(1, Ordering::Relaxed);
    PENDING.with(|entries| entries.borrow_mut().insert(id, pending()));
    assert!(take_pending(id).is_some());
    assert!(take_pending(id).is_none());
  }
}
