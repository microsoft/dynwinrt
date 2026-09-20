// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Internal, typed system helpers shared with the native language bindings.
//! This is not a general DLL loader or a supported projection API.

use std::{ffi::c_void, sync::OnceLock};
use windows::Win32::{
    Foundation::{FreeLibrary, GetLastError, HANDLE, HINSTANCE, HMODULE, HWND, SetLastError},
    Graphics::Gdi::HGDIOBJ,
    System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
    UI::{
        HiDpi::DPI_AWARENESS_CONTEXT,
        WindowsAndMessaging::{HICON, HMENU, WINDOW_EX_STYLE, WINDOW_STYLE},
    },
};
use windows_core::{BOOL, Error, PCSTR, PCWSTR, Result, s, w};

type Export = unsafe extern "system" fn() -> isize;
type DeleteObjectFn = unsafe extern "system" fn(HGDIOBJ) -> BOOL;
type DestroyIconFn = unsafe extern "system" fn(HICON) -> BOOL;
type CreateWindowExWFn = unsafe extern "system" fn(
    WINDOW_EX_STYLE,
    PCWSTR,
    PCWSTR,
    WINDOW_STYLE,
    i32,
    i32,
    i32,
    i32,
    HWND,
    HMENU,
    HINSTANCE,
    *const c_void,
) -> HWND;

struct Module(HMODULE);

impl Drop for Module {
    fn drop(&mut self) {
        let _ = unsafe { FreeLibrary(self.0) };
    }
}

fn load(name: PCWSTR) -> Result<Module> {
    unsafe { LoadLibraryExW(name, None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.map(Module)
}

fn find(module: &Module, name: PCSTR, label: &str) -> Result<Export> {
    unsafe { GetProcAddress(module.0, name) }
        .ok_or_else(Error::from_thread)
        .map_err(|error| Error::new(error.code(), format!("Failed to resolve {label}: {error}")))
}

fn resolve<'a, M, T>(
    cache: &'a OnceLock<T>,
    dll: &str,
    load: impl FnOnce() -> Result<M>,
    find: impl FnOnce(&M) -> Result<T>,
) -> Result<&'a T> {
    if let Some(api) = cache.get() {
        return Ok(api);
    }
    // Loading and export lookup must not run under the publication lock.
    // Only successes are cached; failure and race-loser references are released.
    let last_error = unsafe { GetLastError() };
    let module = load()
        .map_err(|error| Error::new(error.code(), format!("Failed to load {dll}: {error}")))?;
    let api = find(&module)?;
    let api = cache.get_or_init(|| {
        // The winning loader reference outlives every published function pointer.
        std::mem::forget(module);
        api
    });
    // Some helpers do not set LastError on every result. Successful admission
    // must not substitute loader bookkeeping for the caller's incoming value.
    unsafe { SetLastError(last_error) };
    Ok(api)
}

// Each signature below matches the SDK/windows-rs extern "system" declaration,
// not the fallible/generic windows-rs convenience wrapper.
macro_rules! export {
    ($module:expr, $dll:literal, $name:literal, $typ:ty) => {
        unsafe {
            std::mem::transmute::<Export, $typ>(find(
                $module,
                s!($name),
                concat!($dll, "!", $name),
            )?)
        }
    };
}

static DELETE_OBJECT: OnceLock<GdiObjectDeleter> = OnceLock::new();
static DESTROY_ICON: OnceLock<DestroyIconFn> = OnceLock::new();
static CREATE_WINDOW: OnceLock<CreateWindowExWFn> = OnceLock::new();
static DPI_API: OnceLock<DpiApi> = OnceLock::new();

/// A typed deleter backed by a process-lifetime loader reference.
#[derive(Clone, Copy, Debug)]
pub struct GdiObjectDeleter(DeleteObjectFn);

impl GdiObjectDeleter {
    /// Resolve before invoking any native operation that can produce an owned GDI object.
    pub fn resolve() -> Result<Self> {
        #[cfg(test)]
        test_support::check_resolution()?;
        resolve(
            &DELETE_OBJECT,
            "gdi32.dll",
            || load(w!("gdi32.dll")),
            |module| {
                Ok(Self(export!(
                    module,
                    "gdi32.dll",
                    "DeleteObject",
                    DeleteObjectFn
                )))
            },
        )
        .copied()
    }

    /// Use only after admission has resolved the deleter, including on native failure.
    /// This never loads a DLL or performs a fallible export lookup in cleanup.
    pub fn prepared() -> Self {
        *DELETE_OBJECT
            .get()
            .expect("GDI output ownership requires pre-dispatch cleanup admission")
    }

    /// # Safety
    /// `object` must be a GDI object for which the caller may call DeleteObject.
    pub unsafe fn delete(self, object: HGDIOBJ) -> BOOL {
        #[cfg(test)]
        test_support::record_delete();
        unsafe { (self.0)(object) }
    }
}

/// # Safety
/// `icon` must satisfy the SDK DestroyIcon ownership requirements.
pub unsafe fn destroy_icon(icon: HICON) -> Result<()> {
    let destroy = resolve(
        &DESTROY_ICON,
        "user32.dll",
        || load(w!("user32.dll")),
        |module| Ok(export!(module, "user32.dll", "DestroyIcon", DestroyIconFn)),
    )?;
    unsafe { destroy(icon).ok() }
}

/// # Safety
/// Arguments and the calling thread must satisfy the SDK CreateWindowExW contract.
#[allow(clippy::too_many_arguments)]
pub unsafe fn create_window_ex(
    ex_style: WINDOW_EX_STYLE,
    class_name: PCWSTR,
    title: PCWSTR,
    style: WINDOW_STYLE,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    parent: Option<HWND>,
    menu: Option<HMENU>,
    instance: Option<HINSTANCE>,
    param: Option<*const c_void>,
) -> Result<HWND> {
    let create = resolve(
        &CREATE_WINDOW,
        "user32.dll",
        || load(w!("user32.dll")),
        |module| {
            Ok(export!(
                module,
                "user32.dll",
                "CreateWindowExW",
                CreateWindowExWFn
            ))
        },
    )?;
    let window = unsafe {
        create(
            ex_style,
            class_name,
            title,
            style,
            x,
            y,
            width,
            height,
            parent.unwrap_or_default(),
            menu.unwrap_or_default(),
            instance.unwrap_or_default(),
            param.unwrap_or(std::ptr::null()),
        )
    };
    (!window.is_invalid())
        .then_some(window)
        .ok_or_else(Error::from_thread)
}

pub(crate) struct DpiApi {
    pub(crate) contexts_equal:
        unsafe extern "system" fn(DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT) -> BOOL,
    pub(crate) process_context: unsafe extern "system" fn(HANDLE) -> DPI_AWARENESS_CONTEXT,
    pub(crate) set_process_context: unsafe extern "system" fn(DPI_AWARENESS_CONTEXT) -> BOOL,
    pub(crate) set_thread_context:
        unsafe extern "system" fn(DPI_AWARENESS_CONTEXT) -> DPI_AWARENESS_CONTEXT,
}

impl DpiApi {
    pub(crate) fn resolve() -> Result<&'static Self> {
        resolve(
            &DPI_API,
            "user32.dll",
            || load(w!("user32.dll")),
            |module| {
                Ok(Self {
                    contexts_equal: export!(
                        module,
                        "user32.dll",
                        "AreDpiAwarenessContextsEqual",
                        unsafe extern "system" fn(
                            DPI_AWARENESS_CONTEXT,
                            DPI_AWARENESS_CONTEXT,
                        ) -> BOOL
                    ),
                    process_context: export!(
                        module,
                        "user32.dll",
                        "GetDpiAwarenessContextForProcess",
                        unsafe extern "system" fn(HANDLE) -> DPI_AWARENESS_CONTEXT
                    ),
                    set_process_context: export!(
                        module,
                        "user32.dll",
                        "SetProcessDpiAwarenessContext",
                        unsafe extern "system" fn(DPI_AWARENESS_CONTEXT) -> BOOL
                    ),
                    set_thread_context: export!(
                        module,
                        "user32.dll",
                        "SetThreadDpiAwarenessContext",
                        unsafe extern "system" fn(DPI_AWARENESS_CONTEXT) -> DPI_AWARENESS_CONTEXT
                    ),
                })
            },
        )
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::Cell;
    use windows_core::{Error, HRESULT, Result};

    thread_local! {
        static RESOLUTION_FAILURE: Cell<Option<HRESULT>> = const { Cell::new(None) };
        static RESOLUTION_CALLS: Cell<usize> = const { Cell::new(0) };
        static DELETE_CALLS: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) struct ResolutionFailure(Option<HRESULT>);

    impl ResolutionFailure {
        pub(crate) fn new(error: HRESULT) -> Self {
            Self(RESOLUTION_FAILURE.replace(Some(error)))
        }
    }

    impl Drop for ResolutionFailure {
        fn drop(&mut self) {
            RESOLUTION_FAILURE.set(self.0);
        }
    }

    pub(super) fn check_resolution() -> Result<()> {
        RESOLUTION_CALLS.set(RESOLUTION_CALLS.get() + 1);
        match RESOLUTION_FAILURE.get() {
            Some(error) => Err(Error::new(
                error,
                "Failed to resolve gdi32.dll!DeleteObject (test)",
            )),
            None => Ok(()),
        }
    }

    pub(super) fn record_delete() {
        DELETE_CALLS.set(DELETE_CALLS.get() + 1);
    }

    pub(crate) fn delete_calls() -> usize {
        DELETE_CALLS.get()
    }

    pub(crate) fn resolution_calls() -> usize {
        RESOLUTION_CALLS.get()
    }
}

#[cfg(test)]
mod tests;
