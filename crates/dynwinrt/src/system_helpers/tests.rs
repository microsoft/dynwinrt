// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};
use windows::Win32::{
    Foundation::{ERROR_MOD_NOT_FOUND, ERROR_PROC_NOT_FOUND},
    Graphics::Gdi::{BITMAP, CreateBitmap, GetObjectW},
    UI::WindowsAndMessaging::{CreateIcon, DestroyWindow, WS_POPUP},
};
use windows_core::HRESULT;

struct ModuleProbe(Arc<AtomicUsize>);

impl Drop for ModuleProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn successful_cold_resolution_preserves_incoming_last_error() {
    use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_INVALID_DATA};
    let cache = OnceLock::new();
    unsafe { SetLastError(ERROR_ACCESS_DENIED) };
    resolve(
        &cache,
        "test.dll",
        || {
            unsafe { SetLastError(ERROR_INVALID_DATA) };
            Ok(())
        },
        |_| {
            unsafe { SetLastError(ERROR_PROC_NOT_FOUND) };
            Ok(7usize)
        },
    )
    .unwrap();
    assert_eq!(unsafe { GetLastError() }, ERROR_ACCESS_DENIED);
    resolve(
        &cache,
        "test.dll",
        || -> Result<()> { panic!("cache hit must not call the loader") },
        |_| panic!("cache hit must not resolve exports"),
    )
    .unwrap();
    assert_eq!(unsafe { GetLastError() }, ERROR_ACCESS_DENIED);
}

#[test]
fn missing_system_dll_preserves_error_and_leaves_cache_retryable() {
    let cache = OnceLock::<usize>::new();
    let error = resolve(
        &cache,
        "dynwinrt-missing-ui-helper.dll",
        || load(w!("dynwinrt-missing-ui-helper.dll")),
        |_| panic!("failed loading must not resolve exports"),
    )
    .unwrap_err();
    assert_eq!(error.code(), HRESULT::from_win32(ERROR_MOD_NOT_FOUND.0));
    assert!(error.message().contains("dynwinrt-missing-ui-helper.dll"));
    assert!(cache.get().is_none());
    assert_eq!(
        *resolve(&cache, "test.dll", || Ok(()), |_| Ok(7)).unwrap(),
        7
    );
}

#[test]
fn missing_native_export_preserves_last_error_before_free_library() {
    let cache = OnceLock::new();
    let error = resolve(
        &cache,
        "user32.dll",
        || load(w!("user32.dll")),
        |module| {
            find(
                module,
                s!("DynwinrtMissingUiHelper"),
                "user32.dll!DynwinrtMissingUiHelper",
            )
        },
    )
    .unwrap_err();
    assert_eq!(error.code(), HRESULT::from_win32(ERROR_PROC_NOT_FOUND.0));
    assert!(
        error
            .message()
            .contains("user32.dll!DynwinrtMissingUiHelper")
    );
    assert!(cache.get().is_none());
}

#[test]
fn failed_resolution_releases_module_and_success_is_cached() {
    let cache = OnceLock::new();
    let drops = Arc::new(AtomicUsize::new(0));
    let error = resolve(
        &cache,
        "test.dll",
        || Ok(ModuleProbe(drops.clone())),
        |_| -> Result<usize> {
            Err(Error::from_hresult(HRESULT::from_win32(
                ERROR_PROC_NOT_FOUND.0,
            )))
        },
    )
    .unwrap_err();
    assert_eq!(error.code(), HRESULT::from_win32(ERROR_PROC_NOT_FOUND.0));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(cache.get().is_none());
    let first = resolve(
        &cache,
        "test.dll",
        || Ok(ModuleProbe(drops.clone())),
        |_| Ok(7),
    )
    .unwrap();
    let second = resolve(
        &cache,
        "test.dll",
        || -> Result<ModuleProbe> { panic!("cached helper must not load again") },
        |_| panic!("cached helper must not look up the export again"),
    )
    .unwrap();
    assert!(std::ptr::eq(first, second));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn concurrent_resolution_loads_outside_publication_and_retains_only_winner() {
    const THREADS: usize = 8;
    let cache = OnceLock::new();
    let drops = Arc::new(AtomicUsize::new(0));
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let threads = (0..THREADS)
            .map(|_| {
                scope.spawn(|| {
                    let api = resolve(
                        &cache,
                        "test.dll",
                        || Ok(ModuleProbe(drops.clone())),
                        |_| {
                            let (release_tx, release_rx) = mpsc::channel();
                            ready_tx.send(release_tx).unwrap();
                            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                            Ok(7usize)
                        },
                    )
                    .unwrap();
                    api as *const usize as usize
                })
            })
            .collect::<Vec<_>>();
        let releases = (0..THREADS)
            .map(|_| ready_rx.recv_timeout(Duration::from_secs(10)).unwrap())
            .collect::<Vec<_>>();
        for release in releases {
            release.send(()).unwrap();
        }
        let pointers = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert!(pointers.iter().all(|pointer| *pointer == pointers[0]));
    });
    assert_eq!(drops.load(Ordering::SeqCst), THREADS - 1);
}

#[test]
fn typed_gdi_deleter_releases_real_bitmap_once_without_loading_during_cleanup() {
    let deleter = GdiObjectDeleter::resolve().unwrap();
    let bitmap = unsafe { CreateBitmap(1, 1, 1, 1, None) };
    assert!(!bitmap.is_invalid());
    let object = HGDIOBJ(bitmap.0);
    let mut info = BITMAP::default();
    let size = std::mem::size_of::<BITMAP>() as i32;
    assert_eq!(
        unsafe { GetObjectW(object, size, Some((&mut info as *mut BITMAP).cast())) },
        size
    );
    let before = test_support::delete_calls();
    let _failure =
        test_support::ResolutionFailure::new(HRESULT::from_win32(ERROR_PROC_NOT_FOUND.0));
    assert!(GdiObjectDeleter::resolve().is_err());
    assert!(unsafe { deleter.delete(object) }.as_bool());
    assert_eq!(test_support::delete_calls(), before + 1);
    // GetObjectType can report stale handle type bits even after deletion.
    assert_eq!(
        unsafe { GetObjectW(object, size, Some((&mut info as *mut BITMAP).cast())) },
        0
    );
    // The prepared path also remains available while a new resolution would fail.
    let _ = GdiObjectDeleter::prepared();
}

#[test]
fn typed_destroy_icon_uses_icon_cleanup_not_delete_object() {
    let and_mask = [0xff; 32];
    let xor_mask = [0; 32];
    let icon =
        unsafe { CreateIcon(None, 16, 16, 1, 1, and_mask.as_ptr(), xor_mask.as_ptr()) }.unwrap();
    let before = test_support::delete_calls();
    unsafe { destroy_icon(icon) }.unwrap();
    assert_eq!(test_support::delete_calls(), before);
    assert!(unsafe { destroy_icon(HICON::default()) }.is_err());
}

#[test]
fn typed_create_window_preserves_result_and_last_error() {
    let window = unsafe {
        create_window_ex(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("dynwinrt-lazy-ui-test"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            None,
            None,
        )
    }
    .unwrap();
    unsafe { DestroyWindow(window) }.unwrap();

    let error = unsafe {
        create_window_ex(
            WINDOW_EX_STYLE(0),
            w!("DynwinrtUnregisteredWindowClass"),
            w!(""),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            None,
            None,
        )
    }
    .unwrap_err();
    let sdk_error = unsafe {
        windows::Win32::UI::WindowsAndMessaging::CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("DynwinrtUnregisteredWindowClass"),
            w!(""),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            None,
            None,
        )
    }
    .unwrap_err();
    assert_eq!(error.code(), sdk_error.code());
}
