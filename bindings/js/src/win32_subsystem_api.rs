// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
  ffi::{c_void, CStr},
  sync::{Arc, Mutex},
};

use dynwinrt::win32::module::SystemModule;
use windows::core::HRESULT;
use windows::Win32::{
  Graphics::GdiPlus::{GdiplusStartupInput, GdiplusStartupOutput, Status},
  Networking::WinSock::WSADATA,
};

use super::{MAPI_DEINIT_EXPORT, MAPI_INIT_EXPORT};

struct SuccessCache<T> {
  value: Mutex<Option<Arc<T>>>,
}

impl<T> SuccessCache<T> {
  const fn new() -> Self {
    Self {
      value: Mutex::new(None),
    }
  }

  fn get_or_try_init(&self, initialize: impl FnOnce() -> napi::Result<T>) -> napi::Result<Arc<T>> {
    if let Some(value) = self
      .value
      .lock()
      .unwrap_or_else(|error| error.into_inner())
      .as_ref()
      .cloned()
    {
      return Ok(value);
    }
    let complete = Arc::new(initialize()?);
    let published = {
      let mut value = self.value.lock().unwrap_or_else(|error| error.into_inner());
      Arc::clone(value.get_or_insert_with(|| Arc::clone(&complete)))
    };
    Ok(published)
  }
}

pub(super) struct LoadedApi<F> {
  pub(super) functions: F,
  _module: Arc<SystemModule>,
}

fn load_api<F>(
  dll: &str,
  resolve: impl FnOnce(&mut dyn FnMut(&CStr) -> napi::Result<*mut c_void>) -> napi::Result<F>,
) -> napi::Result<LoadedApi<F>> {
  // These adapters select fixed system DLLs and validate all lifecycle exports before startup.
  let module = unsafe { SystemModule::cached(dll) }
    .map_err(|error| napi::Error::from_reason(error.message()))?;
  let functions = resolve(&mut |name| {
    module
      .export(name)
      .map_err(|error| napi::Error::from_reason(error.message()))
  })?;
  Ok(LoadedApi {
    functions,
    _module: module,
  })
}

fn resolve_exports<const N: usize>(
  names: [&CStr; N],
  resolve: &mut dyn FnMut(&CStr) -> napi::Result<*mut c_void>,
) -> napi::Result<[*mut c_void; N]> {
  let mut addresses = [std::ptr::null_mut(); N];
  for (index, name) in names.into_iter().enumerate() {
    let address = resolve(name)?;
    if address.is_null() {
      return Err(napi::Error::from_reason(format!(
        "Lifecycle export `{}` has a null address",
        name.to_string_lossy()
      )));
    }
    addresses[index] = address;
  }
  Ok(addresses)
}

#[derive(Clone, Copy)]
pub(super) struct WinsockFunctions {
  pub(super) startup: unsafe extern "system" fn(u16, *mut WSADATA) -> i32,
  pub(super) cleanup: unsafe extern "system" fn() -> i32,
  pub(super) last_error: unsafe extern "system" fn() -> i32,
}

impl WinsockFunctions {
  pub(super) fn resolve(
    resolver: &mut dyn FnMut(&CStr) -> napi::Result<*mut c_void>,
  ) -> napi::Result<Self> {
    let [startup, cleanup, last_error] =
      resolve_exports([c"WSAStartup", c"WSACleanup", c"WSAGetLastError"], resolver)?;
    // The table uses the exact WINAPI signatures from winsock2.h.
    Ok(Self {
      startup: unsafe {
        std::mem::transmute::<*mut c_void, unsafe extern "system" fn(u16, *mut WSADATA) -> i32>(
          startup,
        )
      },
      cleanup: unsafe {
        std::mem::transmute::<*mut c_void, unsafe extern "system" fn() -> i32>(cleanup)
      },
      last_error: unsafe {
        std::mem::transmute::<*mut c_void, unsafe extern "system" fn() -> i32>(last_error)
      },
    })
  }
}

#[derive(Clone, Copy)]
pub(super) struct GdiPlusFunctions {
  pub(super) startup: unsafe extern "system" fn(
    *mut usize,
    *const GdiplusStartupInput,
    *mut GdiplusStartupOutput,
  ) -> Status,
  pub(super) shutdown: unsafe extern "system" fn(usize),
}

impl GdiPlusFunctions {
  pub(super) fn resolve(
    resolver: &mut dyn FnMut(&CStr) -> napi::Result<*mut c_void>,
  ) -> napi::Result<Self> {
    let [startup, shutdown] = resolve_exports([c"GdiplusStartup", c"GdiplusShutdown"], resolver)?;
    Ok(Self {
      startup: unsafe {
        std::mem::transmute::<
          *mut c_void,
          unsafe extern "system" fn(
            *mut usize,
            *const GdiplusStartupInput,
            *mut GdiplusStartupOutput,
          ) -> Status,
        >(startup)
      },
      shutdown: unsafe {
        std::mem::transmute::<*mut c_void, unsafe extern "system" fn(usize)>(shutdown)
      },
    })
  }
}

#[derive(Clone, Copy)]
pub(super) struct MediaFoundationFunctions {
  pub(super) startup: unsafe extern "system" fn(u32, u32) -> HRESULT,
  pub(super) shutdown: unsafe extern "system" fn() -> HRESULT,
}

impl MediaFoundationFunctions {
  pub(super) fn resolve(
    resolver: &mut dyn FnMut(&CStr) -> napi::Result<*mut c_void>,
  ) -> napi::Result<Self> {
    let [startup, shutdown] = resolve_exports([c"MFStartup", c"MFShutdown"], resolver)?;
    Ok(Self {
      startup: unsafe {
        std::mem::transmute::<*mut c_void, unsafe extern "system" fn(u32, u32) -> HRESULT>(startup)
      },
      shutdown: unsafe {
        std::mem::transmute::<*mut c_void, unsafe extern "system" fn() -> HRESULT>(shutdown)
      },
    })
  }
}

#[derive(Clone, Copy)]
pub(super) struct MapiUtilityFunctions {
  pub(super) initialize: unsafe extern "system" fn(u32) -> i32,
  pub(super) deinitialize: unsafe extern "system" fn(),
}

impl MapiUtilityFunctions {
  pub(super) fn resolve(
    resolver: &mut dyn FnMut(&CStr) -> napi::Result<*mut c_void>,
  ) -> napi::Result<Self> {
    let [initialize, deinitialize] =
      resolve_exports([MAPI_INIT_EXPORT, MAPI_DEINIT_EXPORT], resolver)?;
    Ok(Self {
      initialize: unsafe {
        std::mem::transmute::<*mut c_void, unsafe extern "system" fn(u32) -> i32>(initialize)
      },
      deinitialize: unsafe {
        std::mem::transmute::<*mut c_void, unsafe extern "system" fn()>(deinitialize)
      },
    })
  }
}

pub(super) fn winsock() -> napi::Result<Arc<LoadedApi<WinsockFunctions>>> {
  static API: SuccessCache<LoadedApi<WinsockFunctions>> = SuccessCache::new();
  API.get_or_try_init(|| load_api("ws2_32.dll", WinsockFunctions::resolve))
}

pub(super) fn gdiplus() -> napi::Result<Arc<LoadedApi<GdiPlusFunctions>>> {
  static API: SuccessCache<LoadedApi<GdiPlusFunctions>> = SuccessCache::new();
  API.get_or_try_init(|| load_api("gdiplus.dll", GdiPlusFunctions::resolve))
}

pub(super) fn media_foundation() -> napi::Result<Arc<LoadedApi<MediaFoundationFunctions>>> {
  static API: SuccessCache<LoadedApi<MediaFoundationFunctions>> = SuccessCache::new();
  API.get_or_try_init(|| load_api("mfplat.dll", MediaFoundationFunctions::resolve))
}

pub(super) fn mapi() -> napi::Result<Arc<LoadedApi<MapiUtilityFunctions>>> {
  static API: SuccessCache<LoadedApi<MapiUtilityFunctions>> = SuccessCache::new();
  API.get_or_try_init(|| load_api("mapi32.dll", MapiUtilityFunctions::resolve))
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Barrier,
  };

  #[test]
  fn failed_table_initialization_is_not_cached_and_complete_tables_are_reused() {
    let cache = SuccessCache::new();
    let failed =
      cache.get_or_try_init(|| Err::<usize, _>(napi::Error::from_reason("missing export")));
    assert!(failed.is_err());
    assert!(cache.value.lock().unwrap().is_none());
    let complete = cache.get_or_try_init(|| Ok(42)).unwrap();
    let cached = cache
      .get_or_try_init(|| panic!("completed table rebuilt"))
      .unwrap();
    assert!(Arc::ptr_eq(&complete, &cached));
    assert_eq!(*cached, 42);
  }

  #[test]
  fn incomplete_or_null_lifecycle_exports_never_publish_a_table() {
    unsafe extern "system" fn startup(_: u32, _: u32) -> HRESULT {
      panic!("startup must not run while resolving a table");
    }
    let mut names = Vec::new();
    let missing = MediaFoundationFunctions::resolve(&mut |name| {
      names.push(name.to_owned());
      if name == c"MFStartup" {
        Ok(startup as *const () as *mut c_void)
      } else {
        Err(napi::Error::from_reason("MFShutdown missing"))
      }
    });
    assert!(missing.is_err());
    assert_eq!(names, [c"MFStartup".to_owned(), c"MFShutdown".to_owned()]);
    assert!(WinsockFunctions::resolve(&mut |_| Ok(std::ptr::null_mut())).is_err());
    assert!(GdiPlusFunctions::resolve(&mut |_| Ok(std::ptr::null_mut())).is_err());
    assert!(MapiUtilityFunctions::resolve(&mut |_| Ok(std::ptr::null_mut())).is_err());
  }

  #[test]
  fn concurrent_table_publication_keeps_one_complete_owner() {
    struct Table(Arc<AtomicUsize>);
    impl Drop for Table {
      fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
      }
    }
    let cache = Arc::new(SuccessCache::new());
    let barrier = Arc::new(Barrier::new(8));
    let dropped = Arc::new(AtomicUsize::new(0));
    let threads = (0..8)
      .map(|_| {
        let cache = Arc::clone(&cache);
        let barrier = Arc::clone(&barrier);
        let dropped = Arc::clone(&dropped);
        std::thread::spawn(move || {
          cache
            .get_or_try_init(|| {
              barrier.wait();
              Ok(Table(dropped))
            })
            .unwrap()
        })
      })
      .collect::<Vec<_>>();
    let tables = threads
      .into_iter()
      .map(|thread| thread.join().unwrap())
      .collect::<Vec<_>>();
    assert!(tables.iter().all(|table| Arc::ptr_eq(table, &tables[0])));
    assert_eq!(dropped.load(Ordering::SeqCst), 7);
    drop(tables);
    drop(cache);
    assert_eq!(dropped.load(Ordering::SeqCst), 8);
  }
}
