// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{ffi::c_void, sync::OnceLock};

use windows::{
    System::DispatcherQueueController,
    Win32::{
        Foundation::{FreeLibrary, HMODULE},
        System::{
            LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
            WinRT::DispatcherQueueOptions,
        },
    },
};
use windows_core::{Error, HRESULT, Result, s, w};

type CreateDispatcherQueueControllerFn =
    unsafe extern "system" fn(DispatcherQueueOptions, *mut *mut c_void) -> HRESULT;

static CREATE_CONTROLLER: OnceLock<CreateDispatcherQueueControllerFn> = OnceLock::new();

struct Module(HMODULE);

impl Drop for Module {
    fn drop(&mut self) {
        let _ = unsafe { FreeLibrary(self.0) };
    }
}

pub(super) fn create_controller(
    options: DispatcherQueueOptions,
) -> Result<DispatcherQueueController> {
    let create = resolve(&CREATE_CONTROLLER, load_module, find_create_controller)?;
    invoke(*create, options)
}

fn load_module() -> Result<Module> {
    unsafe { LoadLibraryExW(w!("CoreMessaging.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
        .map(Module)
}

fn find_create_controller(module: &Module) -> Result<CreateDispatcherQueueControllerFn> {
    let address = unsafe { GetProcAddress(module.0, s!("CreateDispatcherQueueController")) }
        .ok_or_else(Error::from_thread)?;
    // dispatcherqueue.h and windows-rs declare HRESULT WINAPI with options by
    // value and an IDispatcherQueueController** output. The module is retained
    // before this pointer is published.
    Ok(unsafe {
        std::mem::transmute::<unsafe extern "system" fn() -> isize, CreateDispatcherQueueControllerFn>(
            address,
        )
    })
}

fn resolve<M>(
    cache: &OnceLock<CreateDispatcherQueueControllerFn>,
    load: impl FnOnce() -> Result<M>,
    find: impl FnOnce(&M) -> Result<CreateDispatcherQueueControllerFn>,
) -> Result<&CreateDispatcherQueueControllerFn> {
    if let Some(create) = cache.get() {
        return Ok(create);
    }

    // Native loading stays outside the publication lock; failures leave the
    // cache empty so a later explicit request can retry.
    let module = load().map_err(|error| {
        Error::new(
            error.code(),
            format!("Failed to load CoreMessaging.dll for DispatcherQueue creation: {error}"),
        )
    })?;
    let create = find(&module).map_err(|error| {
        Error::new(
            error.code(),
            format!("Failed to resolve CoreMessaging.dll!CreateDispatcherQueueController: {error}"),
        )
    })?;

    Ok(cache.get_or_init(|| {
        // Only the winner retains a loader reference for the process lifetime,
        // keeping controller/callback code valid too. Losing closures drop
        // their temporary modules, as does failed export resolution above.
        std::mem::forget(module);
        create
    }))
}

fn invoke(
    create: CreateDispatcherQueueControllerFn,
    options: DispatcherQueueOptions,
) -> Result<DispatcherQueueController> {
    let mut controller = std::ptr::null_mut();
    // Match windows-rs: adopt the owned output only on a successful HRESULT.
    // A successful null becomes Error::empty(); failed outputs are undefined.
    unsafe {
        create(options, &mut controller).and_then(|| windows_core::Type::from_abi(controller))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };
    use std::time::Duration;
    use windows::Win32::Foundation::{E_ACCESSDENIED, ERROR_MOD_NOT_FOUND, ERROR_PROC_NOT_FOUND};

    use super::*;

    unsafe extern "system" fn successful_null(
        _: DispatcherQueueOptions,
        _: *mut *mut c_void,
    ) -> HRESULT {
        HRESULT(0)
    }

    unsafe extern "system" fn successful_status_with_null(
        _: DispatcherQueueOptions,
        _: *mut *mut c_void,
    ) -> HRESULT {
        HRESULT(1)
    }

    unsafe extern "system" fn failed_creation(
        _: DispatcherQueueOptions,
        _: *mut *mut c_void,
    ) -> HRESULT {
        E_ACCESSDENIED
    }

    struct ModuleProbe(Arc<AtomicUsize>);

    impl Drop for ModuleProbe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn native_result_preserves_failure_and_successful_null() {
        for create in [
            successful_null as CreateDispatcherQueueControllerFn,
            successful_status_with_null,
        ] {
            let error = invoke(create, DispatcherQueueOptions::default()).unwrap_err();
            assert_eq!(error.code(), Error::empty().code());
        }
        let error = invoke(failed_creation, DispatcherQueueOptions::default()).unwrap_err();
        assert_eq!(error.code(), E_ACCESSDENIED);
    }

    #[test]
    fn missing_system_module_reports_deferred_load_error() {
        let cache = OnceLock::new();
        let error = resolve(
            &cache,
            || {
                unsafe {
                    LoadLibraryExW(
                        w!("dynwinrt-test-missing-coremessaging.dll"),
                        None,
                        LOAD_LIBRARY_SEARCH_SYSTEM32,
                    )
                }
                .map(Module)
            },
            find_create_controller,
        )
        .unwrap_err();
        assert_eq!(error.code(), HRESULT::from_win32(ERROR_MOD_NOT_FOUND.0));
        assert!(error.message().contains("CoreMessaging.dll"));
        assert!(error.message().contains("DispatcherQueue creation"));
        assert!(cache.get().is_none());
    }

    #[test]
    fn missing_native_export_reports_deferred_resolution_error() {
        let cache = OnceLock::new();
        let error = resolve(&cache, load_module, |module| {
            let address = unsafe {
                GetProcAddress(
                    module.0,
                    s!("DynwinrtTestMissingCreateDispatcherQueueController"),
                )
            };
            assert!(address.is_none());
            Err(Error::from_thread())
        })
        .unwrap_err();
        assert_eq!(error.code(), HRESULT::from_win32(ERROR_PROC_NOT_FOUND.0));
        assert!(
            error
                .message()
                .contains("CoreMessaging.dll!CreateDispatcherQueueController")
        );
        assert!(cache.get().is_none());
    }

    #[test]
    fn failed_resolution_releases_module_and_can_retry() {
        let cache = OnceLock::new();
        let dropped = Arc::new(AtomicUsize::new(0));
        let error = resolve(
            &cache,
            || Ok(ModuleProbe(dropped.clone())),
            |_| {
                Err(Error::from_hresult(HRESULT::from_win32(
                    ERROR_PROC_NOT_FOUND.0,
                )))
            },
        )
        .unwrap_err();
        assert_eq!(error.code(), HRESULT::from_win32(ERROR_PROC_NOT_FOUND.0));
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert!(cache.get().is_none());

        let first = resolve(
            &cache,
            || Ok(ModuleProbe(dropped.clone())),
            |_| Ok(successful_null),
        )
        .unwrap();
        let second = resolve(
            &cache,
            || -> Result<ModuleProbe> { panic!("cached module must not be loaded again") },
            |_| panic!("cached export must not be resolved again"),
        )
        .unwrap();
        assert!(std::ptr::eq(first, second));
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failed_load_leaves_cache_retryable() {
        let cache = OnceLock::new();
        let error = resolve(
            &cache,
            || -> Result<ModuleProbe> {
                Err(Error::from_hresult(HRESULT::from_win32(
                    ERROR_MOD_NOT_FOUND.0,
                )))
            },
            |_| panic!("a failed load must not resolve an export"),
        )
        .unwrap_err();
        assert_eq!(error.code(), HRESULT::from_win32(ERROR_MOD_NOT_FOUND.0));
        assert!(cache.get().is_none());

        let dropped = Arc::new(AtomicUsize::new(0));
        resolve(
            &cache,
            || Ok(ModuleProbe(dropped.clone())),
            |_| Ok(successful_null),
        )
        .unwrap();
        assert!(cache.get().is_some());
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn concurrent_resolution_retains_only_the_winning_module() {
        const THREADS: usize = 8;
        let cache = OnceLock::new();
        let dropped = Arc::new(AtomicUsize::new(0));
        let (ready_tx, ready_rx) = mpsc::channel();
        std::thread::scope(|scope| {
            let threads = (0..THREADS)
                .map(|_| {
                    scope.spawn(|| {
                        let create = resolve(
                            &cache,
                            || Ok(ModuleProbe(dropped.clone())),
                            |_| {
                                let (release_tx, release_rx) = mpsc::channel();
                                ready_tx.send(release_tx).unwrap();
                                release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                                Ok(successful_null)
                            },
                        )
                        .unwrap();
                        create as *const _ as usize
                    })
                })
                .collect::<Vec<_>>();
            let releases = (0..THREADS)
                .map(|_| ready_rx.recv_timeout(Duration::from_secs(10)).unwrap())
                .collect::<Vec<_>>();
            for release in releases {
                release.send(()).unwrap();
            }
            let published = threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect::<Vec<_>>();
            assert!(published.iter().all(|address| *address == published[0]));
        });
        assert_eq!(dropped.load(Ordering::SeqCst), THREADS - 1);
        assert!(cache.get().is_some());
    }
}
