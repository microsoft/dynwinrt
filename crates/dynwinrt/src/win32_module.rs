// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Controlled module ownership shared by Win32 calls and lifecycle adapters.

use std::{
    collections::HashMap,
    ffi::{CStr, c_void},
    sync::{Arc, Mutex, OnceLock},
};
use windows::Win32::{
    Foundation::{FreeLibrary, HMODULE},
    System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
};
use windows_core::{HRESULT, HSTRING, PCSTR};

use crate::result::{Error, Result};

#[derive(Debug)]
pub struct SystemModule {
    name: String,
    handle: HMODULE,
    #[cfg(test)]
    released: Option<Arc<std::sync::atomic::AtomicUsize>>,
}

// Windows module references and export lookup are thread-safe. The handle is
// immutable, and the last Arc releases only this object's LoadLibrary reference.
unsafe impl Send for SystemModule {}
unsafe impl Sync for SystemModule {}

#[derive(Default)]
struct ModuleCache {
    modules: Mutex<HashMap<String, Arc<SystemModule>>>,
}

impl ModuleCache {
    fn get_or_load(
        &self,
        dll: &str,
        load: impl FnOnce(&str) -> Result<SystemModule>,
    ) -> Result<Arc<SystemModule>> {
        if !is_bare_system_module_name(dll) {
            return Err(super::invalid_argument("invalid System32 module name"));
        }
        let key = dll.to_ascii_lowercase();
        if let Some(module) = self
            .modules
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&key)
            .cloned()
        {
            return Ok(module);
        }
        let loaded = Arc::new(load(&key)?);
        let published = {
            let mut modules = self
                .modules
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            Arc::clone(modules.entry(key).or_insert_with(|| Arc::clone(&loaded)))
        };
        // A concurrent loser's reference is released outside the cache lock.
        Ok(published)
    }
}

impl SystemModule {
    /// Returns a process-cached system module. Failed loads are not cached.
    ///
    /// # Safety
    ///
    /// Loading a DLL can execute native initialization code. The caller must
    /// ensure loading the selected system DLL is appropriate for this process.
    pub unsafe fn cached(dll: &str) -> Result<Arc<Self>> {
        static CACHE: OnceLock<ModuleCache> = OnceLock::new();
        CACHE
            .get_or_init(ModuleCache::default)
            .get_or_load(dll, |name| unsafe { Self::load(name) })
    }

    unsafe fn load(dll: &str) -> Result<Self> {
        let handle =
            unsafe { LoadLibraryExW(&HSTRING::from(dll), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
                .map_err(|error| {
                    Error::WindowsError(windows_core::Error::new(
                        error.code(),
                        format!("System DLL `{dll}` could not be loaded from System32: {error}"),
                    ))
                })?;
        Ok(Self {
            name: dll.into(),
            handle,
            #[cfg(test)]
            released: None,
        })
    }

    /// Resolves an export without invoking it. Calling the address requires an
    /// exact native signature; the cached module keeps it valid for the process.
    pub fn export(&self, name: &CStr) -> Result<*mut c_void> {
        unsafe { GetProcAddress(self.handle, PCSTR(name.as_ptr().cast())) }
            .map(|function| function as *const () as *mut c_void)
            .ok_or_else(|| {
                Error::WindowsError(windows_core::Error::new(
                    HRESULT(0x8007007Fu32 as i32),
                    format!(
                        "Export `{}` was not found in `{}`",
                        name.to_string_lossy(),
                        self.name
                    ),
                ))
            })
    }
}

impl Drop for SystemModule {
    fn drop(&mut self) {
        if let Err(error) = unsafe { FreeLibrary(self.handle) } {
            eprintln!(
                "[dynwinrt] system module `{}` cleanup failed: {error}",
                self.name
            );
        } else {
            #[cfg(test)]
            if let Some(released) = &self.released {
                released.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }
}

pub(super) fn is_bare_system_module_name(dll: &str) -> bool {
    let lower = dll.to_ascii_lowercase();
    !dll.is_empty()
        && (lower.ends_with(".dll") || lower.ends_with(".drv"))
        && !dll.encode_utf16().any(|unit| unit == 0)
        && !dll
            .chars()
            .any(|character| matches!(character, '/' | '\\' | ':'))
        && !matches!(dll, "." | "..")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Barrier,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn module_names_fail_before_native_loading() {
        let cache = ModuleCache::default();
        for dll in [
            "",
            "mfplat",
            r"C:\Windows\System32\mfplat.dll",
            r"..\mfplat.dll",
            "../mfplat.dll",
            "bad\0.dll",
        ] {
            assert!(
                cache
                    .get_or_load(dll, |_| panic!("invalid DLL name reached the loader"))
                    .is_err()
            );
        }
        assert!(is_bare_system_module_name("kernel32.dll"));
        assert!(is_bare_system_module_name("winspool.drv"));
    }

    #[test]
    fn failed_loads_retry_and_successful_modules_share_case_insensitive_identity() {
        let cache = ModuleCache::default();
        let failure = cache.get_or_load("kernel32.dll", |_| {
            Err(Error::WindowsError(windows_core::Error::new(
                HRESULT(0x8007007Eu32 as i32),
                "injected missing module",
            )))
        });
        assert!(failure.is_err());
        assert!(cache.modules.lock().unwrap().is_empty());
        let module = cache
            .get_or_load("KERNEL32.dll", |name| unsafe { SystemModule::load(name) })
            .unwrap();
        let reused = cache
            .get_or_load("kernel32.dll", |_| panic!("cached module loaded again"))
            .unwrap();
        assert!(Arc::ptr_eq(&module, &reused));
        assert!(module.export(c"GetCurrentProcess").is_ok());
        let missing = module.export(c"dynwinrt_missing_export").unwrap_err();
        assert!(missing.message().contains("dynwinrt_missing_export"));
        assert!(missing.message().contains("kernel32.dll"));
        assert!(module.export(c"GetCurrentProcess").is_ok());
    }

    #[test]
    fn simultaneous_first_loads_release_every_losing_module_reference() {
        let cache = Arc::new(ModuleCache::default());
        let barrier = Arc::new(Barrier::new(8));
        let released = Arc::new(AtomicUsize::new(0));
        let threads = (0..8)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let barrier = Arc::clone(&barrier);
                let released = Arc::clone(&released);
                std::thread::spawn(move || {
                    cache
                        .get_or_load("kernel32.dll", |name| {
                            let mut module = unsafe { SystemModule::load(name) }?;
                            module.released = Some(released);
                            barrier.wait();
                            Ok(module)
                        })
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();
        let modules = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert!(
            modules
                .iter()
                .all(|module| Arc::ptr_eq(&modules[0], module))
        );
        assert_eq!(released.load(Ordering::SeqCst), 7);
        drop(modules);
        drop(cache);
        assert_eq!(released.load(Ordering::SeqCst), 8);
    }

    #[test]
    fn module_ownership_and_exports_are_shareable_across_threads() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<SystemModule>();
        send_sync::<Arc<SystemModule>>();
        let module = unsafe { SystemModule::cached("kernel32.dll") }.unwrap();
        std::thread::spawn(move || {
            assert!(!module.export(c"GetCurrentProcess").unwrap().is_null());
        })
        .join()
        .unwrap();
    }
}
