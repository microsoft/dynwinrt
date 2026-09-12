// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Win32 owner identity and access coordination, independent of projections.

use core::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use windows::Win32::Foundation::HANDLE;
use windows_core::HRESULT;

use super::{Cleanup, invalid_argument};
use crate::result::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceAccess {
    Borrow,
    Consume,
    MutateState(&'static str),
}

#[derive(Debug)]
struct ResourceControl {
    value: Mutex<usize>,
    retained: AtomicUsize,
    active: AtomicUsize,
}

impl ResourceControl {
    fn lock(&self, access: ResourceAccess) -> Result<MutexGuard<'_, usize>> {
        let value = self.value.lock().unwrap_or_else(|error| error.into_inner());
        if self.retained.load(Ordering::Acquire) != 0 {
            match access {
                ResourceAccess::Borrow => {}
                ResourceAccess::Consume => {
                    return Err(invalid_argument(
                        "cannot consume a Win32 resource while asynchronous I/O is pending",
                    ));
                }
                ResourceAccess::MutateState(state) => {
                    return Err(invalid_argument(&format!(
                        "cannot change {state} while asynchronous I/O is pending"
                    )));
                }
            }
        }
        Ok(value)
    }
}

#[derive(Debug, Default)]
struct ResourceCapabilities {
    file: OnceLock<FileCapability>,
}

/// State acquired only by operations with a validated file contract. Its
/// identity is the resource owner, never the numeric HANDLE.
#[derive(Debug, Default)]
pub struct FileCapability {
    state: Mutex<FileState>,
}

#[derive(Debug, Default)]
struct FileState {
    completion_modes: Option<u32>,
    completion_port: Option<usize>,
}

impl FileCapability {
    pub fn cached_completion_modes(&self) -> Option<u32> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .completion_modes
    }

    /// Records a verified, successful monotonic mode update. Without a native
    /// query, the other mode bits remain unknown.
    pub fn record_completion_modes(&self, flags: u32) {
        if let Some(modes) = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .completion_modes
            .as_mut()
        {
            *modes |= flags;
        }
    }

    fn query_completion_modes(&self, handle: usize) -> Result<u32> {
        use windows::Wdk::Storage::FileSystem::{
            FileIoCompletionNotificationInformation, NtQueryInformationFile,
        };
        use windows::Wdk::System::SystemServices::FILE_IO_COMPLETION_NOTIFICATION_INFORMATION;
        use windows::Win32::System::IO::IO_STATUS_BLOCK;

        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let mut status_block = IO_STATUS_BLOCK::default();
        let mut information = FILE_IO_COMPLETION_NOTIFICATION_INFORMATION::default();
        let status = unsafe {
            NtQueryInformationFile(
                HANDLE(handle as *mut c_void),
                &mut status_block,
                (&mut information as *mut FILE_IO_COMPLETION_NOTIFICATION_INFORMATION).cast(),
                size_of::<FILE_IO_COMPLETION_NOTIFICATION_INFORMATION>() as u32,
                FileIoCompletionNotificationInformation,
            )
        };
        if status.0 != 0 {
            return Err(Error::WindowsError(windows_core::Error::new(
                HRESULT(status.0 | 0x1000_0000),
                format!(
                    "Cannot query file completion notification modes: NTSTATUS 0x{:08x}",
                    status.0 as u32
                ),
            )));
        }
        state.completion_modes = Some(information.Flags);
        Ok(information.Flags)
    }

    pub(super) fn associate(&self, handle: usize, port: usize) -> Result<()> {
        use windows::Win32::System::IO::CreateIoCompletionPort;

        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(existing) = state.completion_port {
            return if existing == port {
                Ok(())
            } else {
                Err(invalid_argument(
                    "Win32 resource was associated with a different IOCP",
                ))
            };
        }
        let port = HANDLE(port as *mut c_void);
        let associated =
            unsafe { CreateIoCompletionPort(HANDLE(handle as *mut c_void), Some(port), 0, 0) }
                .map_err(|error| {
                    invalid_argument(&format!(
                        "Failed to associate Win32 resource with dynwinrt IOCP: {error}"
                    ))
                })?;
        if associated != port {
            return Err(invalid_argument(
                "Win32 resource was associated with an unexpected IOCP",
            ));
        }
        state.completion_port = Some(port.0 as usize);
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn set_known_completion_modes(&self, flags: u32) {
        self.state.lock().unwrap().completion_modes = Some(flags);
    }
}

#[derive(Debug)]
pub struct OwnedResource {
    control: ResourceControl,
    cleanup: Cleanup,
    capabilities: ResourceCapabilities,
}

pub struct OwnedResourceLease<'a> {
    value: MutexGuard<'a, usize>,
}

pub struct OwnedResourceAsyncLease {
    resource: Arc<OwnedResource>,
    value: usize,
    active: bool,
}

impl OwnedResourceAsyncLease {
    pub fn raw(&self) -> usize {
        self.value
    }

    pub fn completion_modes(&self) -> Result<u32> {
        self.resource
            .file_capability()
            .query_completion_modes(self.value)
    }

    pub(super) fn associate_completion_port(&self, port: usize) -> Result<()> {
        self.resource.file_capability().associate(self.value, port)
    }

    pub fn mark_active(&mut self) {
        if !self.active {
            self.resource.control.active.fetch_add(1, Ordering::AcqRel);
            self.active = true;
        }
    }

    pub fn mark_inactive(&mut self) {
        if self.active {
            self.resource.control.active.fetch_sub(1, Ordering::AcqRel);
            self.active = false;
        }
    }
}

impl Drop for OwnedResourceAsyncLease {
    fn drop(&mut self) {
        self.mark_inactive();
        self.resource
            .control
            .retained
            .fetch_sub(1, Ordering::AcqRel);
    }
}

impl OwnedResourceLease<'_> {
    pub fn raw(&self) -> usize {
        *self.value
    }
}

impl OwnedResource {
    pub(super) fn new(value: usize, cleanup: Cleanup) -> Self {
        debug_assert!(value != 0);
        debug_assert!(cleanup.owns_resource());
        Self {
            control: ResourceControl {
                value: Mutex::new(value),
                retained: AtomicUsize::new(0),
                active: AtomicUsize::new(0),
            },
            cleanup,
            capabilities: ResourceCapabilities::default(),
        }
    }

    /// Adopts a native resource whose exact ownership and cleanup were
    /// validated by the flat Win32 semantic projection.
    pub unsafe fn adopt(value: usize, cleanup: Cleanup) -> Result<Arc<Self>> {
        if value == 0 || !cleanup.owns_resource() {
            return Err(invalid_argument(
                "owned Win32 resource requires a nonzero value and cleanup",
            ));
        }
        Ok(Arc::new(Self::new(value, cleanup)))
    }

    pub fn raw(&self) -> usize {
        *self
            .control
            .value
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub fn is_closed(&self) -> bool {
        self.raw() == 0
    }

    pub fn cleanup(&self) -> Cleanup {
        self.cleanup
    }

    pub fn file_capability(&self) -> &FileCapability {
        self.capabilities.file.get_or_init(FileCapability::default)
    }

    pub fn has_async_leases(&self) -> bool {
        self.control.retained.load(Ordering::Acquire) != 0
    }

    pub fn has_active_async_io(&self) -> bool {
        self.control.active.load(Ordering::Acquire) != 0
    }

    pub fn lease(&self, expected_cleanup: Cleanup) -> Result<OwnedResourceLease<'_>> {
        if self.cleanup != expected_cleanup {
            return Err(invalid_argument(
                "managed Win32 resource cleanup kind does not match",
            ));
        }
        let value = self.control.lock(ResourceAccess::Borrow)?;
        if *value == 0 {
            return Err(invalid_argument("cannot lease a closed Win32 resource"));
        }
        Ok(OwnedResourceLease { value })
    }

    pub(super) fn lock_for_call(&self, access: ResourceAccess) -> Result<MutexGuard<'_, usize>> {
        self.control.lock(access)
    }

    pub fn async_lease(
        self: &Arc<Self>,
        expected_cleanup: Cleanup,
    ) -> Result<OwnedResourceAsyncLease> {
        if self.cleanup != expected_cleanup {
            return Err(invalid_argument(
                "managed Win32 resource cleanup kind does not match",
            ));
        }
        let value = self.control.lock(ResourceAccess::Borrow)?;
        if *value == 0 {
            return Err(invalid_argument("cannot lease a closed Win32 resource"));
        }
        self.control.retained.fetch_add(1, Ordering::AcqRel);
        Ok(OwnedResourceAsyncLease {
            resource: Arc::clone(self),
            value: *value,
            active: false,
        })
    }

    pub fn close(&self) -> windows_core::Result<()> {
        let mut value = self
            .control
            .value
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.has_async_leases() {
            return Err(windows_core::Error::new(
                HRESULT(0x800700AAu32 as i32),
                "cannot close a Win32 resource while asynchronous I/O is pending",
            ));
        }
        unsafe { self.cleanup.run(*value) }?;
        *value = 0;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn async_lease_count(&self) -> usize {
        self.control.retained.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(super) fn active_async_io_count(&self) -> usize {
        self.control.active.load(Ordering::Acquire)
    }
}

impl Drop for OwnedResource {
    fn drop(&mut self) {
        let value = *self
            .control
            .value
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        if let Err(error) = unsafe { self.cleanup.run(value) } {
            eprintln!(
                "[dynwinrt] Win32 resource cleanup failed ({:?}): {error}",
                self.cleanup
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event() -> Arc<OwnedResource> {
        let handle =
            unsafe { windows::Win32::System::Threading::CreateEventW(None, true, false, None) }
                .unwrap();
        unsafe { OwnedResource::adopt(handle.0 as usize, Cleanup::CloseHandle) }.unwrap()
    }

    #[test]
    fn generic_access_excludes_consumption_and_mutation_until_all_retained_leases_drop() {
        let owner = event();
        let alias = Arc::clone(&owner);
        let mut first = owner.async_lease(Cleanup::CloseHandle).unwrap();
        let second = alias.async_lease(Cleanup::CloseHandle).unwrap();
        first.mark_active();
        first.mark_inactive();
        assert!(!owner.has_active_async_io());
        assert!(owner.has_async_leases());
        for access in [
            ResourceAccess::Consume,
            ResourceAccess::MutateState("native state"),
        ] {
            assert!(alias.lock_for_call(access).is_err());
        }
        let borrow = alias.lock_for_call(ResourceAccess::Borrow).unwrap();
        assert_eq!(*borrow, first.raw());
        drop(borrow);
        assert!(owner.close().is_err());
        drop(first);
        assert!(alias.lock_for_call(ResourceAccess::Consume).is_err());
        drop(second);
        drop(
            alias
                .lock_for_call(ResourceAccess::MutateState("native state"))
                .unwrap(),
        );
        alias.close().unwrap();
        assert!(owner.is_closed());
    }

    #[test]
    fn typed_file_state_shares_owner_identity_without_guessing_unknown_mode_bits() {
        let owner = event();
        let alias = Arc::clone(&owner);
        let other = event();
        let file = owner.file_capability();
        assert!(std::ptr::eq(file, alias.file_capability()));
        assert!(!std::ptr::eq(file, other.file_capability()));
        file.record_completion_modes(1);
        assert_eq!(file.cached_completion_modes(), None);
        file.set_known_completion_modes(2);
        file.record_completion_modes(1);
        file.record_completion_modes(0);
        assert_eq!(alias.file_capability().cached_completion_modes(), Some(3));
        assert_eq!(other.file_capability().cached_completion_modes(), None);
        owner.close().unwrap();
        other.close().unwrap();
    }
}
