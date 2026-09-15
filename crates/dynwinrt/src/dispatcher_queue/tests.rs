// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use windows::Win32::{
    Foundation::{E_ABORT, E_ACCESSDENIED},
    System::WinRT::{RO_INIT_SINGLETHREADED, RoInitialize, RoUninitialize},
    UI::WindowsAndMessaging::{DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage},
};
use windows_core::{Error, Result};
use windows_future::AsyncStatus;

use super::*;

struct Apartment;

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

fn on_sta(test: impl FnOnce() -> Result<()> + Send + 'static) -> Result<()> {
    thread::spawn(move || {
        unsafe { RoInitialize(RO_INIT_SINGLETHREADED)? };
        let _apartment = Apartment;
        test()
    })
    .join()
    .expect("DispatcherQueue test thread panicked")
}

fn pump_until(mut done: impl FnMut() -> Result<bool>) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done()? {
        if Instant::now() >= deadline {
            return Err(Error::new(E_ABORT, "DispatcherQueue test pump timed out"));
        }
        let mut message = MSG::default();
        for _ in 0..64 {
            if !unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                break;
            }
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

struct TestQueue(SystemDispatcherQueue);

impl TestQueue {
    fn shutdown(&mut self) -> Result<()> {
        self.0.request_shutdown()?;
        if let Some(action) = self.0.shutdown_action.as_ref() {
            pump_until(|| Ok(action.Status()? != AsyncStatus::Started))?;
            action.GetResults()?;
        }
        Ok(())
    }
}

impl Drop for TestQueue {
    fn drop(&mut self) {
        if let Err(error) = self.shutdown() {
            if thread::panicking() {
                eprintln!("DispatcherQueue test cleanup failed: {error}");
            } else {
                panic!("DispatcherQueue test cleanup failed: {error}");
            }
        }
    }
}

#[test]
fn queue_query_failure_does_not_resolve_coremessaging() {
    let result = SystemDispatcherQueue::ensure_for_current_thread_with(
        || Err(Error::from_hresult(E_ACCESSDENIED)),
        |_| panic!("a native queue query failure must propagate without loading"),
    );
    assert_eq!(result.err().unwrap().code(), E_ACCESSDENIED);
}

#[test]
fn successful_null_queue_attempts_creation_with_unchanged_options() {
    let result = SystemDispatcherQueue::ensure_for_current_thread_with(
        || Err(Error::empty()),
        |options| {
            assert_eq!(options.dwSize as usize, size_of::<DispatcherQueueOptions>());
            assert_eq!(options.threadType, DQTYPE_THREAD_CURRENT);
            assert_eq!(options.apartmentType, DQTAT_COM_STA);
            Err(Error::new(E_ABORT, "injected controller creation failure"))
        },
    );
    let error = result.err().unwrap();
    assert_eq!(error.code(), E_ABORT);
    assert!(
        error
            .message()
            .contains("injected controller creation failure")
    );
}

#[test]
fn current_thread_creation_reuse_callbacks_and_shutdown() -> Result<()> {
    on_sta(|| {
        let mut owner = TestQueue(SystemDispatcherQueue::ensure_for_current_thread()?);
        assert!(owner.0.was_created());
        assert!(owner.0.has_thread_access()?);

        let removed_observers = Arc::new(AtomicUsize::new(0));
        let removed_starting = removed_observers.clone();
        let removed_completed = removed_observers.clone();
        owner.0.observe_shutdown(
            move || {
                removed_starting.fetch_add(1, Ordering::SeqCst);
            },
            move || {
                removed_completed.fetch_add(1, Ordering::SeqCst);
            },
        )?;
        let events = Arc::new(Mutex::new(Vec::new()));
        let starting_events = events.clone();
        let completed_events = events.clone();
        owner.0.observe_shutdown(
            move || starting_events.lock().unwrap().push("starting"),
            move || completed_events.lock().unwrap().push("completed"),
        )?;

        let mut reused = SystemDispatcherQueue::ensure_for_current_thread_with(
            DispatcherQueue::GetForCurrentThread,
            |_| panic!("an existing queue must not resolve CoreMessaging"),
        )?;
        assert!(!reused.was_created());
        assert!(reused.has_thread_access()?);
        assert_eq!(reused.queue, owner.0.queue);
        assert!(!reused.request_shutdown()?);
        let removed_starting = removed_observers.clone();
        let removed_completed = removed_observers.clone();
        reused.observe_shutdown(
            move || {
                removed_starting.fetch_add(1, Ordering::SeqCst);
            },
            move || {
                removed_completed.fetch_add(1, Ordering::SeqCst);
            },
        )?;
        drop(reused);

        let owner_thread = thread::current().id();
        let callback_on_owner = Arc::new(AtomicBool::new(false));
        let callback_thread = callback_on_owner.clone();
        let work_events = events.clone();
        let handle = owner.0.handle();
        assert!(
            thread::spawn(move || {
                handle.try_enqueue(move || {
                    callback_thread.store(thread::current().id() == owner_thread, Ordering::SeqCst);
                    work_events.lock().unwrap().push("work");
                    Ok(())
                })
            })
            .join()
            .unwrap()?
        );
        pump_until(|| Ok(!events.lock().unwrap().is_empty()))?;
        assert!(callback_on_owner.load(Ordering::SeqCst));
        assert!(owner.0.request_shutdown()?);
        assert!(!owner.0.request_shutdown()?);
        owner.shutdown()?;
        assert_eq!(*events.lock().unwrap(), ["work", "starting", "completed"]);
        assert_eq!(removed_observers.load(Ordering::SeqCst), 0);
        assert!(!owner.0.handle().try_enqueue(|| Ok(()))?);
        Ok(())
    })
}
