// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
    ffi::OsString,
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
    sync::Mutex,
};
use windows::Win32::{
    Foundation::{E_UNEXPECTED, ERROR_MOD_NOT_FOUND, FreeLibrary, HMODULE},
    System::{
        LibraryLoader::{
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GetModuleFileNameW, GetModuleHandleExW,
        },
        WinRT::IActivationFactory,
    },
};
use windows_core::{Error, HSTRING, IUnknown, Interface, PCWSTR};

use crate::{Result, WinRTValue};

#[cfg(feature = "test-hooks")]
static OWNED_REFERENCES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Opt-in implementation-module ownership for a process-lifetime WinUI host.
///
/// Only WinUI activation uses this policy. COM objects and apartments are still
/// caller-owned; this owner retains module mappings, not activation factories.
/// Store it in a static: published references intentionally survive until the
/// process terminates, including COM cleanup on other threads.
pub struct WinUiProcessModules {
    modules: Mutex<Modules>,
}

impl Default for WinUiProcessModules {
    fn default() -> Self {
        Self::new()
    }
}

impl WinUiProcessModules {
    pub const fn new() -> Self {
        Self {
            modules: Mutex::new(Modules {
                xaml: None,
                controls: None,
            }),
        }
    }

    /// Resolve a factory, retaining WinUI's implementation when actually used.
    /// Other classes keep the ordinary activation and fallback policy.
    pub fn activation_factory(&'static self, name: &HSTRING) -> Result<WinRTValue> {
        crate::roapi::activation_factory_with_modules(name, is_winui_class(name).then_some(self))
    }

    /// Compose an Application using this same activation lifetime policy.
    pub fn create_xaml_application(
        &'static self,
        metadata_provider: &IUnknown,
        launched_callback: Option<&IUnknown>,
    ) -> Result<WinRTValue> {
        crate::xaml_application::create_xaml_application_with_factory(
            metadata_provider,
            launched_callback,
            |name| self.activation_factory(name),
        )
    }

    pub(crate) fn retain_factory(
        &'static self,
        class_name: &HSTRING,
        factory: &IActivationFactory,
        fallback: &mut Option<ModuleReference>,
    ) -> Result<()> {
        // A COM object's address is heap storage, not an address in its DLL.
        // The typed factory vtable remains valid while the factory is alive.
        let implementation = VerifiedModule::from_reference(ModuleReference::from_address(
            std::ptr::from_ref(factory.vtable()).cast(),
        )?)?;
        validate_fallback(&implementation, fallback)?;
        let xaml = match implementation.identity.kind {
            ModuleKind::Xaml => None,
            ModuleKind::Controls => {
                if class_name == "Microsoft.UI.Xaml.Application" {
                    return Err(invalid_module(
                        "WinUI Application factory is not implemented by Microsoft.UI.Xaml.dll",
                    )
                    .into());
                }
                let path = implementation
                    .identity
                    .path
                    .with_file_name(ModuleKind::Xaml.name());
                let reference = match ModuleReference::from_path(&path) {
                    Ok(reference) => reference,
                    Err(error)
                        if error.code()
                            == windows_core::HRESULT::from_win32(ERROR_MOD_NOT_FOUND.0) =>
                    {
                        // A metadata provider can load Controls before XAML.
                        // Resolve the real Application factory through the same
                        // activation policy, not an arbitrary DLL search path.
                        self.activation_factory(&HSTRING::from("Microsoft.UI.Xaml.Application"))?;
                        ModuleReference::from_path(&path)?
                    }
                    Err(error) => return Err(error.into()),
                };
                let module = VerifiedModule::from_reference(reference)?;
                if module.identity.kind != ModuleKind::Xaml || module.identity.path != path {
                    return Err(invalid_module(
                        "WinUI's loaded XAML dependency has a different path",
                    )
                    .into());
                }
                Some(module)
            }
        };
        self.publish(implementation, xaml, fallback)
            .map_err(Into::into)
    }

    fn publish(
        &self,
        implementation: VerifiedModule,
        xaml: Option<VerifiedModule>,
        fallback: &mut Option<ModuleReference>,
    ) -> windows_core::Result<()> {
        validate_fallback(&implementation, fallback)?;

        let required = xaml.as_ref().unwrap_or(&implementation);
        let mut modules = self
            .modules
            .lock()
            .map_err(|_| invalid_module("WinUI module ownership lock was poisoned"))?;
        modules.validate(required)?;
        if fallback.is_some() {
            modules.validate(&implementation)?;
        }

        let mut redundant = Vec::new();
        // Validate the entire transaction before taking the fallback's only
        // loader reference. On error it must outlive the caller's factory.
        if let Some(reference) = fallback.take() {
            redundant.push(modules.insert(VerifiedModule {
                identity: implementation.identity.clone(),
                reference,
            }));
        }
        if let Some(xaml) = xaml {
            redundant.push(modules.insert(xaml));
        } else {
            redundant.push(modules.insert(implementation));
        }
        drop(modules);
        drop(redundant);
        Ok(())
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn retained_module_paths(&self) -> windows_core::Result<Vec<PathBuf>> {
        let modules = self
            .modules
            .lock()
            .map_err(|_| invalid_module("WinUI module ownership lock was poisoned"))?;
        Ok([&modules.xaml, &modules.controls]
            .into_iter()
            .flatten()
            .map(|module| module.identity.path.clone())
            .collect())
    }

    #[cfg(feature = "test-hooks")]
    pub fn owned_reference_count(&self) -> usize {
        OWNED_REFERENCES.load(std::sync::atomic::Ordering::SeqCst)
    }
}

fn validate_fallback(
    implementation: &VerifiedModule,
    fallback: &Option<ModuleReference>,
) -> windows_core::Result<()> {
    if fallback
        .as_ref()
        .is_some_and(|module| module.handle != implementation.reference.handle)
    {
        Err(invalid_module(
            "WinUI fallback factory is not implemented by the DLL that returned it",
        ))
    } else {
        Ok(())
    }
}

fn is_winui_class(name: &HSTRING) -> bool {
    let name = name.to_string();
    name.strip_prefix("Microsoft.UI.Xaml.")
        .is_some_and(|suffix| !suffix.is_empty() && !suffix.contains('\0'))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModuleKind {
    Xaml,
    Controls,
}

impl ModuleKind {
    fn name(self) -> &'static str {
        match self {
            Self::Xaml => "Microsoft.UI.Xaml.dll",
            Self::Controls => "Microsoft.UI.Xaml.Controls.dll",
        }
    }

    fn from_path(path: &Path) -> windows_core::Result<Self> {
        let name = path.file_name().and_then(|name| name.to_str());
        [Self::Xaml, Self::Controls]
            .into_iter()
            .find(|kind| name.is_some_and(|name| name.eq_ignore_ascii_case(kind.name())))
            .filter(|_| path.is_absolute())
            .ok_or_else(|| {
                invalid_module(format!(
                    "WinUI activation factory has an unrecognized implementation module: {}",
                    path.display()
                ))
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ModuleIdentity {
    kind: ModuleKind,
    path: PathBuf,
}

struct VerifiedModule {
    identity: ModuleIdentity,
    reference: ModuleReference,
}

impl VerifiedModule {
    fn from_reference(reference: ModuleReference) -> windows_core::Result<Self> {
        let path = reference.path()?;
        Ok(Self {
            identity: ModuleIdentity {
                kind: ModuleKind::from_path(&path)?,
                path,
            },
            reference,
        })
    }
}

struct Modules {
    controls: Option<VerifiedModule>,
    xaml: Option<VerifiedModule>,
}

impl Modules {
    fn slot(&mut self, kind: ModuleKind) -> &mut Option<VerifiedModule> {
        match kind {
            ModuleKind::Xaml => &mut self.xaml,
            ModuleKind::Controls => &mut self.controls,
        }
    }

    fn validate(&mut self, candidate: &VerifiedModule) -> windows_core::Result<()> {
        if let Some(existing) = self.slot(candidate.identity.kind) {
            if existing.identity != candidate.identity
                || existing.reference.handle != candidate.reference.handle
            {
                return Err(invalid_module(format!(
                    "WinUI implementation cannot change within a process: {} -> {}",
                    existing.identity.path.display(),
                    candidate.identity.path.display()
                )));
            }
        }
        Ok(())
    }

    fn insert(&mut self, candidate: VerifiedModule) -> Option<VerifiedModule> {
        let slot = self.slot(candidate.identity.kind);
        if slot.is_some() {
            Some(candidate)
        } else {
            *slot = Some(candidate);
            None
        }
    }
}

pub(crate) struct ModuleReference {
    handle: HMODULE,
    #[cfg(test)]
    released: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
}

// A loader reference has no apartment affinity. Publication only shares the
// immutable handle; COM factories are never stored in this owner.
unsafe impl Send for ModuleReference {}
unsafe impl Sync for ModuleReference {}

impl ModuleReference {
    /// Adopt exactly one successful LoadLibrary/GetModuleHandleEx reference.
    pub(crate) unsafe fn adopt(handle: HMODULE) -> Self {
        #[cfg(feature = "test-hooks")]
        OWNED_REFERENCES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self {
            handle,
            #[cfg(test)]
            released: None,
        }
    }

    fn from_address(address: *const std::ffi::c_void) -> windows_core::Result<Self> {
        let mut handle = HMODULE::default();
        unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
                PCWSTR(address.cast()),
                &mut handle,
            )
        }
        .map_err(|error| {
            Error::new(
                error.code(),
                format!("Cannot retain WinUI factory implementation: {error}"),
            )
        })?;
        Ok(unsafe { Self::adopt(handle) })
    }

    fn from_path(path: &Path) -> windows_core::Result<Self> {
        let mut handle = HMODULE::default();
        unsafe { GetModuleHandleExW(0, &HSTRING::from(path.as_os_str()), &mut handle) }.map_err(
            |error| {
                Error::new(
                    error.code(),
                    format!(
                        "Cannot retain WinUI's already-loaded dependency {}: {error}",
                        path.display()
                    ),
                )
            },
        )?;
        Ok(unsafe { Self::adopt(handle) })
    }

    fn path(&self) -> windows_core::Result<PathBuf> {
        let mut buffer = vec![0; 260];
        loop {
            let length = unsafe { GetModuleFileNameW(Some(self.handle), &mut buffer) } as usize;
            if length == 0 {
                return Err(Error::from_thread());
            }
            if length < buffer.len() {
                return Ok(PathBuf::from(OsString::from_wide(&buffer[..length])));
            }
            if buffer.len() >= 32768 {
                return Err(invalid_module(
                    "WinUI implementation module path is too long",
                ));
            }
            buffer.resize((buffer.len() * 2).min(32768), 0);
        }
    }
}

impl Drop for ModuleReference {
    fn drop(&mut self) {
        #[cfg(test)]
        if let Some(released) = &self.released {
            released.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return;
        }
        if let Err(error) = unsafe { FreeLibrary(self.handle) } {
            eprintln!("[dynwinrt] WinUI module candidate cleanup failed: {error}");
        } else {
            #[cfg(feature = "test-hooks")]
            OWNED_REFERENCES.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

fn invalid_module(message: impl AsRef<str>) -> Error {
    Error::new(E_UNEXPECTED, message.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    };

    fn reference(handle: usize, released: &Arc<AtomicUsize>) -> ModuleReference {
        ModuleReference {
            handle: HMODULE(handle as *mut std::ffi::c_void),
            released: Some(Arc::clone(released)),
        }
    }

    fn module(kind: ModuleKind, released: &Arc<AtomicUsize>) -> VerifiedModule {
        VerifiedModule {
            identity: ModuleIdentity {
                kind,
                path: Path::new(r"C:\runtime").join(kind.name()),
            },
            reference: reference(
                match kind {
                    ModuleKind::Xaml => 1,
                    ModuleKind::Controls => 2,
                },
                released,
            ),
        }
    }

    #[test]
    fn policy_requires_an_actual_winui_class_name() {
        for name in [
            "Windows.Foundation.Uri",
            "Microsoft.UI.Dispatching.DispatcherQueue",
            "Microsoft.UI.Xaml",
            "Microsoft.UI.Xaml.",
            "Microsoft.UI.XamlFake.Application",
            "Microsoft.UI.Xaml.Application\0bad",
        ] {
            assert!(!is_winui_class(&HSTRING::from(name)), "{name}");
        }
        for name in [
            "Microsoft.UI.Xaml.Application",
            "Microsoft.UI.Xaml.Controls.Button",
            "Microsoft.UI.Xaml.XamlTypeInfo.XamlControlsXamlMetaDataProvider",
        ] {
            assert!(is_winui_class(&HSTRING::from(name)), "{name}");
        }
    }

    #[test]
    fn identity_requires_recognized_absolute_module_path() {
        assert_eq!(
            ModuleKind::from_path(Path::new(r"C:\runtime\microsoft.ui.xaml.DLL")).unwrap(),
            ModuleKind::Xaml
        );
        for path in [
            r"Microsoft.UI.Xaml.dll",
            r"C:\runtime\Microsoft.UI.Xaml.dll.fake",
            r"C:\runtime\combase.dll",
            r"C:\runtime\Microsoft.UI.Xaml.Resources.dll",
        ] {
            assert!(ModuleKind::from_path(Path::new(path)).is_err());
        }
    }

    #[test]
    fn acquisition_rejects_heap_addresses_and_missing_loaded_dependencies() {
        let heap = Box::new(0usize);
        assert!(ModuleReference::from_address(std::ptr::from_ref(heap.as_ref()).cast()).is_err());
        assert!(
            ModuleReference::from_path(Path::new(r"C:\dynwinrt-not-loaded\Microsoft.UI.Xaml.dll"))
                .is_err()
        );
    }

    #[test]
    fn plain_activation_never_publishes_winui_modules() {
        fn require_send_sync<T: Send + Sync>() {}
        require_send_sync::<WinUiProcessModules>();
        static OWNER: WinUiProcessModules = WinUiProcessModules::new();
        std::thread::spawn(|| {
            use windows::Win32::System::WinRT::{
                RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize,
            };
            unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.unwrap();
            let result = OWNER.activation_factory(&HSTRING::from("Windows.Foundation.Uri"));
            assert!(OWNER.retained_module_paths().unwrap().is_empty());
            drop(result.unwrap());
            unsafe { RoUninitialize() };
        })
        .join()
        .unwrap();
    }

    #[test]
    fn repeated_factories_retain_one_reference_not_com_objects() {
        let released = Arc::new(AtomicUsize::new(0));
        let owner = WinUiProcessModules::new();
        for _ in 0..8 {
            owner
                .publish(module(ModuleKind::Xaml, &released), None, &mut None)
                .unwrap();
        }
        assert_eq!(owner.retained_module_paths().unwrap().len(), 1);
        assert_eq!(released.load(Ordering::SeqCst), 7);
        drop(owner);
        assert_eq!(released.load(Ordering::SeqCst), 8);
    }

    #[test]
    fn fallback_reference_is_transferred_and_duplicate_loads_are_released() {
        let released = Arc::new(AtomicUsize::new(0));
        let owner = WinUiProcessModules::new();
        for _ in 0..8 {
            let mut fallback = Some(reference(1, &released));
            owner
                .publish(module(ModuleKind::Xaml, &released), None, &mut fallback)
                .unwrap();
            assert!(fallback.is_none());
        }
        assert_eq!(owner.retained_module_paths().unwrap().len(), 1);
        assert_eq!(released.load(Ordering::SeqCst), 15);
        drop(owner);
        assert_eq!(released.load(Ordering::SeqCst), 16);
    }

    #[test]
    fn normal_controls_factory_does_not_retain_controls() {
        let released = Arc::new(AtomicUsize::new(0));
        let owner = WinUiProcessModules::new();
        owner
            .publish(
                module(ModuleKind::Controls, &released),
                Some(module(ModuleKind::Xaml, &released)),
                &mut None,
            )
            .unwrap();
        assert_eq!(
            owner.retained_module_paths().unwrap(),
            [PathBuf::from(r"C:\runtime\Microsoft.UI.Xaml.dll")]
        );
        assert_eq!(released.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn controls_fallback_retains_one_reference_per_required_module() {
        let released = Arc::new(AtomicUsize::new(0));
        let owner = WinUiProcessModules::new();
        for _ in 0..4 {
            let mut fallback = Some(reference(2, &released));
            owner
                .publish(
                    module(ModuleKind::Controls, &released),
                    Some(module(ModuleKind::Xaml, &released)),
                    &mut fallback,
                )
                .unwrap();
            assert!(fallback.is_none());
        }
        assert_eq!(owner.retained_module_paths().unwrap().len(), 2);
        assert_eq!(released.load(Ordering::SeqCst), 10);
        drop(owner);
        assert_eq!(released.load(Ordering::SeqCst), 12);
    }

    #[test]
    fn identity_mismatch_keeps_fallback_alive_and_does_not_poison_retry() {
        let released = Arc::new(AtomicUsize::new(0));
        let owner = WinUiProcessModules::new();
        let mut fallback = Some(reference(3, &released));
        assert!(
            owner
                .publish(module(ModuleKind::Xaml, &released), None, &mut fallback)
                .is_err()
        );
        assert!(fallback.is_some());
        assert!(owner.retained_module_paths().unwrap().is_empty());
        assert_eq!(released.load(Ordering::SeqCst), 1);
        drop(fallback.take());
        fallback = Some(reference(1, &released));
        owner
            .publish(module(ModuleKind::Xaml, &released), None, &mut fallback)
            .unwrap();
        assert!(fallback.is_none());
        assert_eq!(owner.retained_module_paths().unwrap().len(), 1);
    }

    #[test]
    fn module_replacement_fails_without_partially_publishing_fallback() {
        let released = Arc::new(AtomicUsize::new(0));
        let owner = WinUiProcessModules::new();
        owner
            .publish(module(ModuleKind::Xaml, &released), None, &mut None)
            .unwrap();
        let mut replacement = module(ModuleKind::Xaml, &released);
        replacement.identity.path = PathBuf::from(r"C:\other\Microsoft.UI.Xaml.dll");
        let mut fallback = Some(reference(2, &released));
        assert!(
            owner
                .publish(
                    module(ModuleKind::Controls, &released),
                    Some(replacement),
                    &mut fallback
                )
                .is_err()
        );
        assert!(fallback.is_some());
        assert_eq!(owner.retained_module_paths().unwrap().len(), 1);
    }

    #[test]
    fn concurrent_publish_keeps_only_one_owned_reference() {
        let owner = Arc::new(WinUiProcessModules::new());
        let released = Arc::new(AtomicUsize::new(0));
        let ready = Arc::new(Barrier::new(8));
        let threads = (0..8)
            .map(|_| {
                let owner = Arc::clone(&owner);
                let released = Arc::clone(&released);
                let ready = Arc::clone(&ready);
                std::thread::spawn(move || {
                    let candidate = module(ModuleKind::Xaml, &released);
                    ready.wait();
                    owner.publish(candidate, None, &mut None).unwrap();
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(owner.retained_module_paths().unwrap().len(), 1);
        assert_eq!(released.load(Ordering::SeqCst), 7);
        drop(owner);
        assert_eq!(released.load(Ordering::SeqCst), 8);
    }
}
