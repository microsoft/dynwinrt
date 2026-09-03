// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
  alloc::{alloc_zeroed, dealloc, Layout},
  ffi::c_void,
  sync::{Arc, Mutex, MutexGuard},
  thread::ThreadId,
};

#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};

use napi::bindgen_prelude::{BigInt, Buffer, Either};
use napi_derive::napi;
use windows::core::{IUnknown, Interface as _};

use super::{
  com::{native_struct_layout, native_union_layout, DynComType, NativePointerOwner},
  DynWinRTValue, WinGUID,
};

const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
const RAW_MAX_AGGREGATE_BYTE_SIZE: usize = 1024 * 1024;
const RAW_MAX_AGGREGATE_NESTING_DEPTH: usize = 32;
const RAW_MAX_FIXED_FIELD_EXPANSION: usize = 65_536;
const RAW_MAX_LIBFFI_ELEMENTS: usize = 65_536;

struct AllocationState {
  address: usize,
  ownership: RawStorageOwnership,
}

#[derive(Clone, Copy)]
enum RawStorageOwnership {
  Owned(Layout),
  External,
}

struct RawAllocationStorage {
  state: AllocationState,
  #[cfg(test)]
  deallocated: Arc<AtomicBool>,
}

pub(super) struct RawAllocation {
  owner_thread: ThreadId,
  size: usize,
  alignment: usize,
  storage: Mutex<Option<Arc<RawAllocationStorage>>>,
  #[cfg(test)]
  deallocated: Arc<AtomicBool>,
}

pub(super) struct RawInvocationLease {
  _storage: Arc<RawAllocationStorage>,
}

pub(super) struct RawComInvocationLease {
  _value: IUnknown,
}

pub(super) struct RawComReference {
  owner_thread: ThreadId,
  value: Mutex<Option<IUnknown>>,
}

impl RawComReference {
  fn new(value: IUnknown) -> Arc<Self> {
    Arc::new(Self {
      owner_thread: std::thread::current().id(),
      value: Mutex::new(Some(value)),
    })
  }

  fn ensure_owner_thread(&self) -> napi::Result<()> {
    if std::thread::current().id() == self.owner_thread {
      Ok(())
    } else {
      Err(napi::Error::from_reason(
        "Raw COM reference used from a different apartment thread",
      ))
    }
  }

  fn lock_value(&self) -> napi::Result<MutexGuard<'_, Option<IUnknown>>> {
    self
      .value
      .lock()
      .map_err(|_| napi::Error::from_reason("Raw COM reference lock is poisoned"))
  }

  pub(super) fn validate_live(&self) -> napi::Result<()> {
    self.ensure_owner_thread()?;
    if self.lock_value()?.is_some() {
      Ok(())
    } else {
      Err(napi::Error::from_reason(
        "Raw COM reference has been consumed or released",
      ))
    }
  }

  fn address(&self) -> napi::Result<usize> {
    self.ensure_owner_thread()?;
    self
      .lock_value()?
      .as_ref()
      .map(|value| value.as_raw() as usize)
      .ok_or_else(|| napi::Error::from_reason("Raw COM reference has been consumed or released"))
  }

  fn clone_value(&self) -> napi::Result<IUnknown> {
    self.ensure_owner_thread()?;
    self
      .lock_value()?
      .as_ref()
      .cloned()
      .ok_or_else(|| napi::Error::from_reason("Raw COM reference has been consumed or released"))
  }

  pub(super) fn acquire_invocation_lease(&self) -> napi::Result<RawComInvocationLease> {
    self
      .clone_value()
      .map(|value| RawComInvocationLease { _value: value })
  }

  fn take_value(&self) -> napi::Result<IUnknown> {
    self.ensure_owner_thread()?;
    self
      .lock_value()?
      .take()
      .ok_or_else(|| napi::Error::from_reason("Raw COM reference has been consumed or released"))
  }

  fn release(&self) -> napi::Result<()> {
    self.ensure_owner_thread()?;
    let value = self.lock_value()?.take();
    drop(value);
    Ok(())
  }

  fn released(&self) -> napi::Result<bool> {
    self.ensure_owner_thread()?;
    Ok(self.lock_value()?.is_none())
  }

  fn transfer_to_memory(&self, memory: &RawAllocation, offset: usize) -> napi::Result<()> {
    self.ensure_owner_thread()?;
    let mut value = self.lock_value()?;
    let address = value
      .as_ref()
      .map(|value| value.as_raw() as usize)
      .ok_or_else(|| napi::Error::from_reason("Raw COM reference has been consumed or released"))?;
    memory.write_usize(offset, address)?;
    let value = value
      .take()
      .expect("validated live COM reference remains present");
    std::mem::forget(value);
    Ok(())
  }

  fn finalize_owner(&self) {
    let value = match self.value.lock() {
      Ok(mut value) => value.take(),
      Err(poisoned) => poisoned.into_inner().take(),
    };
    if let Some(value) = value {
      if std::thread::current().id() != self.owner_thread || super::winui_dispatcher_loop_exited() {
        std::mem::forget(value);
      } else {
        drop(value);
      }
    }
  }
}

impl Drop for RawComReference {
  fn drop(&mut self) {
    let value = match self.value.get_mut() {
      Ok(value) => value.take(),
      Err(poisoned) => poisoned.into_inner().take(),
    };
    if let Some(value) = value {
      if std::thread::current().id() != self.owner_thread || super::winui_dispatcher_loop_exited() {
        std::mem::forget(value);
      } else {
        drop(value);
      }
    }
  }
}

impl RawAllocation {
  fn allocate(size: usize, alignment: usize) -> napi::Result<Arc<Self>> {
    if size == 0 {
      return Err(napi::Error::from_reason(
        "DynComRawMemory.allocate(): size must be greater than zero",
      ));
    }
    if alignment == 0 || !alignment.is_power_of_two() {
      return Err(napi::Error::from_reason(
        "DynComRawMemory.allocate(): alignment must be a nonzero power of two",
      ));
    }
    let layout = Layout::from_size_align(size, alignment).map_err(|_| {
      napi::Error::from_reason(
        "DynComRawMemory.allocate(): size and alignment exceed platform layout limits",
      )
    })?;

    // Safety: `layout` is valid and nonzero. A null result is reported as an
    // allocation error rather than passed to `handle_alloc_error`.
    let pointer = unsafe { alloc_zeroed(layout) };
    if pointer.is_null() {
      return Err(napi::Error::from_reason(format!(
        "DynComRawMemory.allocate(): failed to allocate {size} bytes with alignment {alignment}",
      )));
    }
    let address = pointer.expose_provenance();
    if address.checked_add(size).is_none() {
      // Safety: this successful allocation has not been published and uses
      // the same validated layout passed to `alloc_zeroed`.
      unsafe {
        dealloc(pointer, layout);
      }
      return Err(napi::Error::from_reason(
        "DynComRawMemory.allocate(): allocation address plus size overflowed",
      ));
    }

    #[cfg(test)]
    let deallocated = Arc::new(AtomicBool::new(false));
    let storage = Arc::new(RawAllocationStorage {
      state: AllocationState {
        address,
        ownership: RawStorageOwnership::Owned(layout),
      },
      #[cfg(test)]
      deallocated: deallocated.clone(),
    });

    Ok(Arc::new(Self {
      owner_thread: std::thread::current().id(),
      size,
      alignment,
      storage: Mutex::new(Some(storage)),
      #[cfg(test)]
      deallocated,
    }))
  }

  fn external(
    address: usize,
    size: usize,
    alignment: usize,
    context: &str,
  ) -> napi::Result<Arc<Self>> {
    if alignment == 0 || !alignment.is_power_of_two() {
      return Err(napi::Error::from_reason(format!(
        "{context}: alignment must be a nonzero power of two",
      )));
    }
    Layout::from_size_align(size, alignment).map_err(|_| {
      napi::Error::from_reason(format!(
        "{context}: size and alignment exceed platform layout limits",
      ))
    })?;
    if address == 0 && size != 0 {
      return Err(napi::Error::from_reason(format!(
        "{context}: a nonempty view requires a non-null address",
      )));
    }
    if address != 0 && address % alignment != 0 {
      return Err(napi::Error::from_reason(format!(
        "{context}: address must be aligned to {alignment} bytes",
      )));
    }
    address.checked_add(size).ok_or_else(|| {
      napi::Error::from_reason(format!("{context}: address plus byte length overflowed",))
    })?;

    #[cfg(test)]
    let deallocated = Arc::new(AtomicBool::new(false));
    let storage = Arc::new(RawAllocationStorage {
      state: AllocationState {
        address,
        ownership: RawStorageOwnership::External,
      },
      #[cfg(test)]
      deallocated: deallocated.clone(),
    });
    Ok(Arc::new(Self {
      owner_thread: std::thread::current().id(),
      size,
      alignment,
      storage: Mutex::new(Some(storage)),
      #[cfg(test)]
      deallocated,
    }))
  }

  fn ensure_owner_thread(&self) -> napi::Result<()> {
    if std::thread::current().id() == self.owner_thread {
      Ok(())
    } else {
      Err(napi::Error::from_reason(
        "Raw COM memory used from a different thread",
      ))
    }
  }

  fn lock_storage(&self) -> napi::Result<MutexGuard<'_, Option<Arc<RawAllocationStorage>>>> {
    self
      .storage
      .lock()
      .map_err(|_| napi::Error::from_reason("Raw COM memory lock is poisoned"))
  }

  fn live_storage(&self) -> napi::Result<Arc<RawAllocationStorage>> {
    self.ensure_owner_thread()?;
    self
      .lock_storage()?
      .as_ref()
      .cloned()
      .ok_or_else(|| napi::Error::from_reason("Raw COM memory has been released"))
  }

  pub(super) fn validate_live(&self) -> napi::Result<()> {
    self.live_storage().map(drop)
  }

  pub(super) fn acquire_invocation_lease(&self) -> napi::Result<RawInvocationLease> {
    self
      .live_storage()
      .map(|storage| RawInvocationLease { _storage: storage })
  }

  fn checked_offset(&self, offset: usize, width: usize) -> napi::Result<()> {
    let end = offset
      .checked_add(width)
      .ok_or_else(|| napi::Error::from_reason("Raw COM memory offset plus width overflowed"))?;
    if end > self.size {
      return Err(napi::Error::from_reason(format!(
        "Raw COM memory range [{offset}, {end}) exceeds allocation size {}",
        self.size
      )));
    }
    Ok(())
  }

  fn with_range<T>(
    &self,
    offset: usize,
    width: usize,
    operation: impl FnOnce(*mut u8) -> T,
  ) -> napi::Result<T> {
    self.checked_offset(offset, width)?;
    let storage = self.live_storage()?;
    let address = storage
      .state
      .address
      .checked_add(offset)
      .expect("validated raw memory address range");
    let pointer = std::ptr::with_exposed_provenance_mut::<u8>(address);

    // Safety: `storage` keeps the allocation live without holding the mutex,
    // and checked construction plus `checked_offset` proves that `address` is
    // within or one byte past the bounded view. For external views the caller
    // explicitly guarantees that the range remains valid. The operation
    // receives no lifetime beyond this call.
    Ok(operation(pointer))
  }

  fn pointer_address(&self, offset: usize) -> napi::Result<usize> {
    self.with_range(offset, 0, |pointer| pointer.expose_provenance())
  }

  fn with_aligned_value<T, R>(
    &self,
    offset: usize,
    operation: impl FnOnce(*mut T) -> napi::Result<R>,
  ) -> napi::Result<R> {
    self.with_range(offset, std::mem::size_of::<T>(), |pointer| {
      if pointer as usize % std::mem::align_of::<T>() != 0 {
        return Err(napi::Error::from_reason(format!(
          "Raw cleanup storage must be aligned to {} bytes",
          std::mem::align_of::<T>()
        )));
      }
      operation(pointer.cast())
    })?
  }

  fn read_bytes(&self, offset: usize, width: usize) -> napi::Result<Vec<u8>> {
    self.with_range(offset, width, |pointer| {
      let mut result = Vec::new();
      result.try_reserve_exact(width).map_err(|_| {
        napi::Error::from_reason(format!(
          "Raw COM memory read could not allocate a {width}-byte result",
        ))
      })?;
      result.resize(width, 0);
      if width != 0 {
        // Safety: both ranges are valid for `width` bytes and do not overlap.
        unsafe {
          std::ptr::copy_nonoverlapping(pointer.cast_const(), result.as_mut_ptr(), width);
        }
      }
      Ok(result)
    })?
  }

  /// # Safety
  ///
  /// `source` must be readable for `length` bytes. It may overlap the checked
  /// destination range.
  unsafe fn write_bytes_from_pointer(
    &self,
    offset: usize,
    source: *const u8,
    length: usize,
  ) -> napi::Result<()> {
    self.with_range(offset, length, |pointer| {
      if length != 0 {
        // Safety: the caller validates the source, `with_range` validates the
        // destination, and `copy` provides memmove semantics for overlap.
        unsafe {
          std::ptr::copy(source, pointer, length);
        }
      }
    })
  }

  fn read_native<const N: usize>(&self, offset: usize) -> napi::Result<[u8; N]> {
    let bytes = self.read_bytes(offset, N)?;
    Ok(
      bytes
        .try_into()
        .expect("raw read length matches the requested primitive width"),
    )
  }

  fn write_native<const N: usize>(&self, offset: usize, bytes: [u8; N]) -> napi::Result<()> {
    unsafe { self.write_bytes_from_pointer(offset, bytes.as_ptr(), bytes.len()) }
  }

  fn read_usize(&self, offset: usize) -> napi::Result<usize> {
    #[cfg(target_pointer_width = "64")]
    {
      self
        .read_native(offset)
        .map(u64::from_ne_bytes)
        .map(|value| value as usize)
    }
    #[cfg(target_pointer_width = "32")]
    {
      self
        .read_native(offset)
        .map(u32::from_ne_bytes)
        .map(|value| value as usize)
    }
  }

  fn write_usize(&self, offset: usize, value: usize) -> napi::Result<()> {
    self.write_native(offset, value.to_ne_bytes())
  }

  fn release(&self) -> napi::Result<()> {
    self.ensure_owner_thread()?;
    let storage = self.lock_storage()?.take();
    drop(storage);
    Ok(())
  }

  fn released(&self) -> napi::Result<bool> {
    self.ensure_owner_thread()?;
    Ok(self.lock_storage()?.is_none())
  }

  #[cfg(test)]
  fn deallocated(&self) -> bool {
    self.deallocated.load(Ordering::Acquire)
  }
}

impl Drop for RawAllocationStorage {
  fn drop(&mut self) {
    if let RawStorageOwnership::Owned(layout) = self.state.ownership {
      #[cfg(test)]
      self.deallocated.store(true, Ordering::Release);
      let pointer = std::ptr::with_exposed_provenance_mut::<u8>(self.state.address);
      // Safety: `address` came from the unique successful allocation for this
      // exact `layout`, and this storage owner is created once for that
      // allocation. Its final Arc drop therefore deallocates exactly once.
      unsafe {
        dealloc(pointer, layout);
      }
    }
  }
}

enum RawPointerKind {
  Owned {
    allocation: Arc<RawAllocation>,
    offset: usize,
  },
  ComBorrowed(Arc<RawComReference>),
  DetachedCom(Arc<RawComReference>),
  External(usize),
  Consumed,
}

fn clone_managed_com_value(value: &DynWinRTValue, context: &str) -> napi::Result<IUnknown> {
  value.ensure_existing_com_apartment()?;
  value
    .0
    .as_object()
    .ok_or_else(|| napi::Error::from_reason(format!("{context}: expected a live interface object")))
}

fn query_owned_com_value(value: IUnknown, iid: &WinGUID) -> napi::Result<IUnknown> {
  let mut queried = std::ptr::null_mut();
  let result = unsafe { value.query(&iid.0, &mut queried) };
  drop(value);
  if result.is_err() {
    if !queried.is_null() {
      drop(unsafe { IUnknown::from_raw(queried) });
    }
    let error = result
      .ok()
      .expect_err("failing QueryInterface HRESULT must produce an error");
    return Err(napi::Error::from_reason(error.to_string()));
  }
  if queried.is_null() {
    return Err(napi::Error::from_reason(
      "QueryInterface succeeded with a null interface pointer",
    ));
  }
  Ok(unsafe { IUnknown::from_raw(queried) })
}

fn managed_from_owned_com_value(value: IUnknown) -> napi::Result<DynWinRTValue> {
  let mut value = DynWinRTValue::new(dynwinrt::WinRTValue::Object(value));
  value.bind_current_com_apartment()?;
  Ok(value)
}

#[napi]
pub struct DynComRaw;

#[napi]
impl DynComRaw {
  #[napi]
  pub fn pointer_size() -> u32 {
    std::mem::size_of::<usize>() as u32
  }

  #[napi(js_name = "__validateExactOutputSlot")]
  pub fn validate_exact_output_slot(memory: &DynComRawMemory) -> napi::Result<()> {
    memory.allocation.validate_live()?;
    let width = std::mem::size_of::<usize>();
    if memory.allocation.size < width {
      return Err(napi::Error::from_reason(
        "Exact interface output slot is smaller than pointer width",
      ));
    }
    let address = memory.allocation.pointer_address(0)?;
    if memory.allocation.alignment < width || address % width != 0 {
      return Err(napi::Error::from_reason(
        "Exact interface output slot is not pointer-aligned",
      ));
    }
    Ok(())
  }
}

#[napi]
pub struct DynComRawCleanup;

#[napi]
impl DynComRawCleanup {
  #[napi]
  pub fn co_task_mem_free(pointer: &mut DynComRawPointer) -> napi::Result<()> {
    let address = pointer.take_external_cleanup_address("coTaskMemFree")?;
    if address != 0 {
      unsafe {
        windows::Win32::System::Com::CoTaskMemFree(Some(std::ptr::with_exposed_provenance_mut::<
          c_void,
        >(address)));
      }
    }
    Ok(())
  }

  #[napi]
  pub fn local_free(pointer: &mut DynComRawPointer) -> napi::Result<()> {
    let address = pointer.external_cleanup_address("localFree")?;
    if address == 0 {
      pointer.consume_external_cleanup();
      return Ok(());
    }
    let result = unsafe {
      windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
        std::ptr::with_exposed_provenance_mut(address),
      )))
    };
    if !result.0.is_null() {
      return Err(napi::Error::from_reason(
        windows::core::Error::from_thread().to_string(),
      ));
    }
    pointer.consume_external_cleanup();
    Ok(())
  }

  #[napi]
  pub fn global_free(pointer: &mut DynComRawPointer) -> napi::Result<()> {
    let address = pointer.external_cleanup_address("globalFree")?;
    if address == 0 {
      pointer.consume_external_cleanup();
      return Ok(());
    }
    let result =
      unsafe { raw_global_free(std::ptr::with_exposed_provenance_mut::<c_void>(address)) };
    if !result.is_null() {
      return Err(napi::Error::from_reason(
        windows::core::Error::from_thread().to_string(),
      ));
    }
    pointer.consume_external_cleanup();
    Ok(())
  }

  #[napi]
  pub fn sys_free_string(pointer: &mut DynComRawPointer) -> napi::Result<()> {
    let address = pointer.take_external_cleanup_address("sysFreeString")?;
    if address != 0 {
      drop(unsafe {
        windows::core::BSTR::from_raw(std::ptr::with_exposed_provenance_mut::<u16>(address))
      });
    }
    Ok(())
  }

  #[napi]
  pub fn safe_array_destroy(pointer: &mut DynComRawPointer) -> napi::Result<()> {
    let address = pointer.external_cleanup_address("safeArrayDestroy")?;
    if address == 0 {
      pointer.consume_external_cleanup();
      return Ok(());
    }
    unsafe {
      windows::Win32::System::Ole::SafeArrayDestroy(std::ptr::with_exposed_provenance::<
        windows::Win32::System::Com::SAFEARRAY,
      >(address))
    }
    .map_err(|error| napi::Error::from_reason(error.to_string()))?;
    pointer.consume_external_cleanup();
    Ok(())
  }

  #[napi]
  pub fn variant_clear(
    memory: &DynComRawMemory,
    offset: Option<Either<BigInt, f64>>,
  ) -> napi::Result<()> {
    let offset = optional_offset(offset, "variantClear() offset")?;
    memory
      .allocation
      .with_aligned_value::<windows::Win32::System::Variant::VARIANT, _>(offset, |value| {
        unsafe { windows::Win32::System::Variant::VariantClear(value) }
          .map_err(|error| napi::Error::from_reason(error.to_string()))
      })
  }

  #[napi]
  pub fn prop_variant_clear(
    memory: &DynComRawMemory,
    offset: Option<Either<BigInt, f64>>,
  ) -> napi::Result<()> {
    let offset = optional_offset(offset, "propVariantClear() offset")?;
    memory
      .allocation
      .with_aligned_value::<windows::Win32::System::Com::StructuredStorage::PROPVARIANT, _>(
        offset,
        |value| {
          unsafe { windows::Win32::System::Com::StructuredStorage::PropVariantClear(value) }
            .map_err(|error| napi::Error::from_reason(error.to_string()))
        },
      )
  }

  #[napi]
  pub fn release_stg_medium(
    memory: &DynComRawMemory,
    offset: Option<Either<BigInt, f64>>,
  ) -> napi::Result<()> {
    let offset = optional_offset(offset, "releaseStgMedium() offset")?;
    memory
      .allocation
      .with_aligned_value::<windows::Win32::System::Com::STGMEDIUM, _>(offset, |value| {
        unsafe {
          windows::Win32::System::Ole::ReleaseStgMedium(value);
          std::ptr::write_bytes(
            value.cast::<u8>(),
            0,
            std::mem::size_of::<windows::Win32::System::Com::STGMEDIUM>(),
          );
        }
        Ok(())
      })
  }

  #[napi]
  pub fn close_handle(pointer: &mut DynComRawPointer) -> napi::Result<()> {
    let address = pointer.external_cleanup_address("closeHandle")?;
    unsafe {
      windows::Win32::Foundation::CloseHandle(windows::Win32::Foundation::HANDLE(
        std::ptr::with_exposed_provenance_mut(address),
      ))
    }
    .map_err(|error| napi::Error::from_reason(error.to_string()))?;
    pointer.consume_external_cleanup();
    Ok(())
  }

  #[napi]
  pub fn destroy_icon(pointer: &mut DynComRawPointer) -> napi::Result<()> {
    let address = pointer.external_cleanup_address("destroyIcon")?;
    unsafe {
      windows::Win32::UI::WindowsAndMessaging::DestroyIcon(
        windows::Win32::UI::WindowsAndMessaging::HICON(std::ptr::with_exposed_provenance_mut(
          address,
        )),
      )
    }
    .map_err(|error| napi::Error::from_reason(error.to_string()))?;
    pointer.consume_external_cleanup();
    Ok(())
  }

  #[napi]
  pub fn delete_object(pointer: &mut DynComRawPointer) -> napi::Result<()> {
    let address = pointer.external_cleanup_address("deleteObject")?;
    unsafe {
      windows::Win32::Graphics::Gdi::DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(
        std::ptr::with_exposed_provenance_mut(address),
      ))
    }
    .ok()
    .map_err(|error| napi::Error::from_reason(error.to_string()))?;
    pointer.consume_external_cleanup();
    Ok(())
  }
}

#[napi]
pub struct DynComRawOwnedComPointer {
  state: Arc<RawComReference>,
}

impl Drop for DynComRawOwnedComPointer {
  fn drop(&mut self) {
    self.state.finalize_owner();
  }
}

#[napi]
impl DynComRawOwnedComPointer {
  #[napi(factory)]
  pub fn add_ref(value: &DynWinRTValue) -> napi::Result<Self> {
    clone_managed_com_value(value, "DynComRawOwnedComPointer.addRef()")
      .map(RawComReference::new)
      .map(|state| Self { state })
  }

  #[napi(factory)]
  pub fn query_interface(value: &DynWinRTValue, iid: &WinGUID) -> napi::Result<Self> {
    let value = clone_managed_com_value(value, "DynComRawOwnedComPointer.queryInterface()")?;
    query_owned_com_value(value, iid)
      .map(RawComReference::new)
      .map(|state| Self { state })
  }

  #[napi(factory)]
  pub fn adopt_transferred(
    pointer: &mut DynComRawPointer,
    iid: Option<&WinGUID>,
  ) -> napi::Result<Self> {
    let value = pointer.take_detached_com_value()?;
    let value = match iid {
      Some(iid) => query_owned_com_value(value, iid)?,
      None => value,
    };
    Ok(Self {
      state: RawComReference::new(value),
    })
  }

  #[napi(factory)]
  pub fn assume_transferred(
    pointer: &mut DynComRawPointer,
    iid: Option<&WinGUID>,
  ) -> napi::Result<Self> {
    let address = pointer.take_assumed_transferred_address()?;
    if address == 0 {
      return Err(napi::Error::from_reason(
        "assumeTransferred() requires a non-null caller-owned +1 COM pointer",
      ));
    }
    let value =
      unsafe { IUnknown::from_raw(std::ptr::with_exposed_provenance_mut::<c_void>(address)) };
    let value = match iid {
      Some(iid) => query_owned_com_value(value, iid)?,
      None => value,
    };
    Ok(Self {
      state: RawComReference::new(value),
    })
  }

  #[napi(getter)]
  pub fn address(&self) -> napi::Result<BigInt> {
    self
      .state
      .address()
      .map(|address| BigInt::from(address as u64))
  }

  #[napi(getter)]
  pub fn released(&self) -> napi::Result<bool> {
    self.state.released()
  }

  #[napi]
  pub fn pointer(&self) -> napi::Result<DynComRawPointer> {
    self.state.validate_live()?;
    Ok(DynComRawPointer {
      kind: RawPointerKind::ComBorrowed(self.state.clone()),
    })
  }

  #[napi]
  pub fn query(&self, iid: &WinGUID) -> napi::Result<Self> {
    query_owned_com_value(self.state.clone_value()?, iid)
      .map(RawComReference::new)
      .map(|state| Self { state })
  }

  #[napi]
  pub fn retain(&self) -> napi::Result<Self> {
    self
      .state
      .clone_value()
      .map(RawComReference::new)
      .map(|state| Self { state })
  }

  #[napi]
  pub fn release(&self) -> napi::Result<()> {
    self.state.release()
  }

  #[napi]
  pub fn detach(&self) -> napi::Result<DynComRawPointer> {
    let value = self.state.take_value()?;
    Ok(DynComRawPointer {
      kind: RawPointerKind::DetachedCom(RawComReference::new(value)),
    })
  }

  #[napi]
  pub fn transfer_to(
    &self,
    memory: &DynComRawMemory,
    offset: Option<Either<BigInt, f64>>,
  ) -> napi::Result<()> {
    let offset = optional_offset(offset, "transferTo() offset")?;
    self.state.transfer_to_memory(&memory.allocation, offset)
  }

  #[napi]
  pub fn into_managed(&self, iid: Option<&WinGUID>) -> napi::Result<DynWinRTValue> {
    let value = self.state.take_value()?;
    let value = match iid {
      Some(iid) => query_owned_com_value(value, iid)?,
      None => value,
    };
    managed_from_owned_com_value(value)
  }
}

#[napi]
pub struct DynComRawStructLayout {
  descriptor: String,
  layout: Arc<dynwinrt::com::NativeStructLayout>,
}

#[napi]
impl DynComRawStructLayout {
  #[napi(factory)]
  pub fn from_descriptor(descriptor: String) -> napi::Result<Self> {
    validate_raw_aggregate_descriptor_limits(&descriptor, RawDescriptorKind::Struct)?;
    let layout = native_struct_layout(&descriptor)?;
    validate_raw_qualified_name(layout.name())?;
    Ok(Self { descriptor, layout })
  }

  #[napi(getter)]
  pub fn qualified_name(&self) -> String {
    self.layout.name().to_string()
  }

  #[napi(getter)]
  pub fn descriptor(&self) -> String {
    self.descriptor.clone()
  }

  #[napi(getter)]
  pub fn size(&self) -> BigInt {
    BigInt::from(self.layout.size() as u64)
  }

  #[napi(getter)]
  pub fn alignment(&self) -> BigInt {
    BigInt::from(self.layout.alignment() as u64)
  }

  #[napi]
  pub fn by_value_type(&self) -> napi::Result<DynComType> {
    validate_raw_by_value_descriptor(&self.descriptor, RawDescriptorKind::Struct)?;
    dynwinrt::com::Type::raw_native_struct(self.layout.clone())
      .map(DynComType)
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  #[napi]
  pub fn pointer_type(&self, nullable: Option<bool>) -> DynComType {
    DynComType(dynwinrt::com::Type::raw_native_struct_pointer(
      self.layout.clone(),
      nullable.unwrap_or(false),
    ))
  }

  #[napi]
  pub fn create_value(&self, bytes: Option<Buffer>) -> napi::Result<DynWinRTValue> {
    let value = match bytes {
      Some(bytes) => {
        if bytes.len() != self.layout.size() {
          return Err(napi::Error::from_reason(format!(
            "Raw struct value requires {} bytes, received {}",
            self.layout.size(),
            bytes.len()
          )));
        }
        dynwinrt::com::NativeStructValue::new(
          self.layout.clone(),
          try_copy_buffer(&bytes, "Raw struct value")?,
        )
      }
      None => dynwinrt::com::NativeStructValue::try_zeroed(self.layout.clone()),
    }
    .map_err(|error| napi::Error::from_reason(error.message()))?;
    Ok(DynWinRTValue::from_com_value(
      dynwinrt::com::Value::NativeStruct(value),
      dynwinrt::com::PointerOutputKind::None,
    ))
  }

  #[napi]
  pub fn read_value_bytes(&self, value: &DynWinRTValue) -> napi::Result<Buffer> {
    let Some(value) = &value.3 else {
      return Err(napi::Error::from_reason(
        "Raw struct value does not contain native struct storage",
      ));
    };
    if value.layout() != &self.layout {
      return Err(napi::Error::from_reason(format!(
        "Raw struct value identity mismatch: expected `{}`",
        self.layout.name()
      )));
    }
    Ok(Buffer::from(value.bytes().to_vec()))
  }
}

#[napi]
pub struct DynComRawUnionLayout {
  descriptor: String,
  layout: Arc<dynwinrt::com::NativeUnionLayout>,
}

#[napi]
impl DynComRawUnionLayout {
  #[napi(factory)]
  pub fn from_descriptor(descriptor: String) -> napi::Result<Self> {
    validate_raw_aggregate_descriptor_limits(&descriptor, RawDescriptorKind::Union)?;
    let layout = native_union_layout(&descriptor)?;
    validate_raw_qualified_name(layout.name())?;
    Ok(Self { descriptor, layout })
  }

  #[napi(getter)]
  pub fn qualified_name(&self) -> String {
    self.layout.name().to_string()
  }

  #[napi(getter)]
  pub fn descriptor(&self) -> String {
    self.descriptor.clone()
  }

  #[napi(getter)]
  pub fn size(&self) -> BigInt {
    BigInt::from(self.layout.size() as u64)
  }

  #[napi(getter)]
  pub fn alignment(&self) -> BigInt {
    BigInt::from(self.layout.alignment() as u64)
  }

  #[napi]
  pub fn pointer_type(&self) -> DynComType {
    DynComType(dynwinrt::com::Type::raw_native_union_pointer(
      self.layout.clone(),
    ))
  }

  #[napi]
  pub fn by_value_type(&self) -> napi::Result<DynComType> {
    validate_raw_by_value_descriptor(&self.descriptor, RawDescriptorKind::Union)?;
    dynwinrt::com::Type::raw_native_union(self.layout.clone())
      .map(DynComType)
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  #[napi]
  pub fn create_value(
    &self,
    active_field: String,
    bytes: Option<Buffer>,
  ) -> napi::Result<DynWinRTValue> {
    let value = match bytes {
      Some(bytes) => {
        if bytes.len() != self.layout.size() {
          return Err(napi::Error::from_reason(format!(
            "Raw union value requires {} bytes, received {}",
            self.layout.size(),
            bytes.len()
          )));
        }
        dynwinrt::com::NativeUnionValue::new(
          self.layout.clone(),
          active_field,
          try_copy_buffer(&bytes, "Raw union value")?,
        )
      }
      None => dynwinrt::com::NativeUnionValue::zeroed(self.layout.clone(), active_field),
    }
    .map_err(|error| napi::Error::from_reason(error.message()))?;
    Ok(DynWinRTValue::from_com_value(
      dynwinrt::com::Value::NativeUnion(value),
      dynwinrt::com::PointerOutputKind::None,
    ))
  }

  #[napi]
  pub fn read_value_bytes(&self, value: &DynWinRTValue) -> napi::Result<Buffer> {
    let Some(value) = &value.5 else {
      return Err(napi::Error::from_reason(
        "Raw union value does not contain native union storage",
      ));
    };
    let dynwinrt::com::Value::NativeUnion(value) = value.to_com_value()? else {
      return Err(napi::Error::from_reason(
        "Raw union value does not contain native union storage",
      ));
    };
    if value.layout() != &self.layout {
      return Err(napi::Error::from_reason(format!(
        "Raw union value identity mismatch: expected `{}`",
        self.layout.name()
      )));
    }
    Ok(Buffer::from(value.bytes().to_vec()))
  }

  #[napi]
  pub fn assert_active_field(
    &self,
    value: &DynWinRTValue,
    active_field: String,
  ) -> napi::Result<Buffer> {
    if !self.layout.has_field(&active_field) {
      return Err(napi::Error::from_reason(format!(
        "Raw union `{}` has no field `{active_field}`",
        self.layout.name()
      )));
    }
    self.read_value_bytes(value)
  }
}

#[napi]
pub struct DynComRawMemory {
  allocation: Arc<RawAllocation>,
}

#[napi]
impl DynComRawMemory {
  #[napi(factory)]
  pub fn allocate(
    size: Either<BigInt, f64>,
    alignment: Option<Either<BigInt, f64>>,
  ) -> napi::Result<Self> {
    let size = js_usize(size, "DynComRawMemory.allocate() size")?;
    let alignment = alignment
      .map(|value| js_usize(value, "DynComRawMemory.allocate() alignment"))
      .transpose()?
      .unwrap_or(std::mem::align_of::<usize>());
    RawAllocation::allocate(size, alignment).map(|allocation| Self { allocation })
  }

  #[napi(factory)]
  pub fn from_unsafe_address(
    address: Either<BigInt, f64>,
    size: Either<BigInt, f64>,
    alignment: Either<BigInt, f64>,
  ) -> napi::Result<Self> {
    let address = js_usize(address, "DynComRawMemory.fromUnsafeAddress() address")?;
    let size = js_usize(size, "DynComRawMemory.fromUnsafeAddress() size")?;
    let alignment = js_usize(alignment, "DynComRawMemory.fromUnsafeAddress() alignment")?;
    RawAllocation::external(
      address,
      size,
      alignment,
      "DynComRawMemory.fromUnsafeAddress()",
    )
    .map(|allocation| Self { allocation })
  }

  #[napi(factory)]
  pub fn from_unsafe_pointer(
    pointer: &DynComRawPointer,
    size: Either<BigInt, f64>,
    alignment: Either<BigInt, f64>,
  ) -> napi::Result<Self> {
    let size = js_usize(size, "DynComRawMemory.fromUnsafePointer() size")?;
    let alignment = js_usize(alignment, "DynComRawMemory.fromUnsafePointer() alignment")?;
    let address = match &pointer.kind {
      RawPointerKind::Owned { allocation, offset } => {
        allocation.checked_offset(*offset, size)?;
        pointer.address_bits()?
      }
      RawPointerKind::ComBorrowed(_)
      | RawPointerKind::DetachedCom(_)
      | RawPointerKind::External(_)
      | RawPointerKind::Consumed => pointer.address_bits()?,
    };
    RawAllocation::external(
      address,
      size,
      alignment,
      "DynComRawMemory.fromUnsafePointer()",
    )
    .map(|allocation| Self { allocation })
  }

  #[napi(getter)]
  pub fn size(&self) -> BigInt {
    BigInt::from(self.allocation.size as u64)
  }

  #[napi(getter)]
  pub fn alignment(&self) -> BigInt {
    BigInt::from(self.allocation.alignment as u64)
  }

  #[napi(getter)]
  pub fn released(&self) -> napi::Result<bool> {
    self.allocation.released()
  }

  #[napi]
  pub fn release(&self) -> napi::Result<()> {
    self.allocation.release()
  }

  #[napi]
  pub fn pointer(&self, offset: Option<Either<BigInt, f64>>) -> napi::Result<DynComRawPointer> {
    let offset = optional_offset(offset, "DynComRawMemory.pointer() offset")?;
    self.allocation.pointer_address(offset)?;
    Ok(DynComRawPointer {
      kind: RawPointerKind::Owned {
        allocation: self.allocation.clone(),
        offset,
      },
    })
  }

  #[napi]
  pub fn read_bytes(
    &self,
    offset: Either<BigInt, f64>,
    length: Either<BigInt, f64>,
  ) -> napi::Result<Buffer> {
    let offset = js_usize(offset, "readBytes() offset")?;
    let length = js_usize(length, "readBytes() length")?;
    self.allocation.read_bytes(offset, length).map(Buffer::from)
  }

  #[napi]
  pub fn write_bytes(&self, offset: Either<BigInt, f64>, value: Buffer) -> napi::Result<()> {
    let offset = js_usize(offset, "writeBytes() offset")?;
    unsafe {
      self
        .allocation
        .write_bytes_from_pointer(offset, value.as_ptr(), value.len())
    }
  }

  #[napi]
  pub fn read_i8(&self, offset: Either<BigInt, f64>) -> napi::Result<i8> {
    self
      .allocation
      .read_native(checked_offset(offset, "readI8() offset")?)
      .map(i8::from_ne_bytes)
  }

  #[napi]
  pub fn write_i8(&self, offset: Either<BigInt, f64>, value: f64) -> napi::Result<()> {
    let value = checked_signed_number(value, i8::MIN as i64, i8::MAX as i64, "writeI8() value")?;
    self.allocation.write_native(
      checked_offset(offset, "writeI8() offset")?,
      (value as i8).to_ne_bytes(),
    )
  }

  #[napi]
  pub fn read_u8(&self, offset: Either<BigInt, f64>) -> napi::Result<u8> {
    self
      .allocation
      .read_native(checked_offset(offset, "readU8() offset")?)
      .map(u8::from_ne_bytes)
  }

  #[napi]
  pub fn write_u8(&self, offset: Either<BigInt, f64>, value: f64) -> napi::Result<()> {
    let value = checked_unsigned_number(value, u8::MAX as u64, "writeU8() value")?;
    self.allocation.write_native(
      checked_offset(offset, "writeU8() offset")?,
      (value as u8).to_ne_bytes(),
    )
  }

  #[napi]
  pub fn read_i16(&self, offset: Either<BigInt, f64>) -> napi::Result<i16> {
    self
      .allocation
      .read_native(checked_offset(offset, "readI16() offset")?)
      .map(i16::from_ne_bytes)
  }

  #[napi]
  pub fn write_i16(&self, offset: Either<BigInt, f64>, value: f64) -> napi::Result<()> {
    let value = checked_signed_number(value, i16::MIN as i64, i16::MAX as i64, "writeI16() value")?;
    self.allocation.write_native(
      checked_offset(offset, "writeI16() offset")?,
      (value as i16).to_ne_bytes(),
    )
  }

  #[napi]
  pub fn read_u16(&self, offset: Either<BigInt, f64>) -> napi::Result<u16> {
    self
      .allocation
      .read_native(checked_offset(offset, "readU16() offset")?)
      .map(u16::from_ne_bytes)
  }

  #[napi]
  pub fn write_u16(&self, offset: Either<BigInt, f64>, value: f64) -> napi::Result<()> {
    let value = checked_unsigned_number(value, u16::MAX as u64, "writeU16() value")?;
    self.allocation.write_native(
      checked_offset(offset, "writeU16() offset")?,
      (value as u16).to_ne_bytes(),
    )
  }

  #[napi]
  pub fn read_i32(&self, offset: Either<BigInt, f64>) -> napi::Result<i32> {
    self
      .allocation
      .read_native(checked_offset(offset, "readI32() offset")?)
      .map(i32::from_ne_bytes)
  }

  #[napi]
  pub fn write_i32(&self, offset: Either<BigInt, f64>, value: f64) -> napi::Result<()> {
    let value = checked_signed_number(value, i32::MIN as i64, i32::MAX as i64, "writeI32() value")?;
    self.allocation.write_native(
      checked_offset(offset, "writeI32() offset")?,
      (value as i32).to_ne_bytes(),
    )
  }

  #[napi]
  pub fn read_u32(&self, offset: Either<BigInt, f64>) -> napi::Result<u32> {
    self
      .allocation
      .read_native(checked_offset(offset, "readU32() offset")?)
      .map(u32::from_ne_bytes)
  }

  #[napi]
  pub fn write_u32(&self, offset: Either<BigInt, f64>, value: f64) -> napi::Result<()> {
    let value = checked_unsigned_number(value, u32::MAX as u64, "writeU32() value")?;
    self.allocation.write_native(
      checked_offset(offset, "writeU32() offset")?,
      (value as u32).to_ne_bytes(),
    )
  }

  #[napi]
  pub fn read_i64(&self, offset: Either<BigInt, f64>) -> napi::Result<BigInt> {
    self
      .allocation
      .read_native(checked_offset(offset, "readI64() offset")?)
      .map(i64::from_ne_bytes)
      .map(BigInt::from)
  }

  #[napi]
  pub fn write_i64(&self, offset: Either<BigInt, f64>, value: BigInt) -> napi::Result<()> {
    let (value, lossless) = value.get_i64();
    if !lossless {
      return Err(napi::Error::from_reason(
        "writeI64() value must fit in a signed 64-bit integer",
      ));
    }
    self.allocation.write_native(
      checked_offset(offset, "writeI64() offset")?,
      value.to_ne_bytes(),
    )
  }

  #[napi]
  pub fn read_u64(&self, offset: Either<BigInt, f64>) -> napi::Result<BigInt> {
    self
      .allocation
      .read_native(checked_offset(offset, "readU64() offset")?)
      .map(u64::from_ne_bytes)
      .map(BigInt::from)
  }

  #[napi]
  pub fn write_u64(&self, offset: Either<BigInt, f64>, value: BigInt) -> napi::Result<()> {
    let (negative, value, lossless) = value.get_u64();
    if negative || !lossless {
      return Err(napi::Error::from_reason(
        "writeU64() value must fit in an unsigned 64-bit integer",
      ));
    }
    self.allocation.write_native(
      checked_offset(offset, "writeU64() offset")?,
      value.to_ne_bytes(),
    )
  }

  #[napi]
  pub fn read_f32(&self, offset: Either<BigInt, f64>) -> napi::Result<f64> {
    self
      .allocation
      .read_native(checked_offset(offset, "readF32() offset")?)
      .map(f32::from_ne_bytes)
      .map(f64::from)
  }

  #[napi]
  pub fn write_f32(&self, offset: Either<BigInt, f64>, value: f64) -> napi::Result<()> {
    if value.is_finite() && (value < f32::MIN as f64 || value > f32::MAX as f64) {
      return Err(napi::Error::from_reason(
        "writeF32() value exceeds the finite f32 range",
      ));
    }
    self.allocation.write_native(
      checked_offset(offset, "writeF32() offset")?,
      (value as f32).to_ne_bytes(),
    )
  }

  #[napi]
  pub fn read_f64(&self, offset: Either<BigInt, f64>) -> napi::Result<f64> {
    self
      .allocation
      .read_native(checked_offset(offset, "readF64() offset")?)
      .map(f64::from_ne_bytes)
  }

  #[napi]
  pub fn write_f64(&self, offset: Either<BigInt, f64>, value: f64) -> napi::Result<()> {
    self.allocation.write_native(
      checked_offset(offset, "writeF64() offset")?,
      value.to_ne_bytes(),
    )
  }

  #[napi]
  pub fn read_isize(&self, offset: Either<BigInt, f64>) -> napi::Result<BigInt> {
    let offset = checked_offset(offset, "readIsize() offset")?;
    #[cfg(target_pointer_width = "64")]
    let value = self
      .allocation
      .read_native(offset)
      .map(i64::from_ne_bytes)?;
    #[cfg(target_pointer_width = "32")]
    let value = self
      .allocation
      .read_native(offset)
      .map(i32::from_ne_bytes)
      .map(i64::from)?;
    Ok(BigInt::from(value))
  }

  #[napi]
  pub fn write_isize(&self, offset: Either<BigInt, f64>, value: BigInt) -> napi::Result<()> {
    let (value, lossless) = value.get_i64();
    if !lossless || value < isize::MIN as i64 || value > isize::MAX as i64 {
      return Err(napi::Error::from_reason(
        "writeIsize() value must fit in a signed pointer-width integer",
      ));
    }
    self.allocation.write_native(
      checked_offset(offset, "writeIsize() offset")?,
      (value as isize).to_ne_bytes(),
    )
  }

  #[napi]
  pub fn read_usize(&self, offset: Either<BigInt, f64>) -> napi::Result<BigInt> {
    self
      .allocation
      .read_usize(checked_offset(offset, "readUsize() offset")?)
      .map(|value| BigInt::from(value as u64))
  }

  #[napi]
  pub fn write_usize(&self, offset: Either<BigInt, f64>, value: BigInt) -> napi::Result<()> {
    let value = bigint_usize(value, "writeUsize() value")?;
    self
      .allocation
      .write_usize(checked_offset(offset, "writeUsize() offset")?, value)
  }

  #[napi]
  pub fn read_pointer(&self, offset: Either<BigInt, f64>) -> napi::Result<DynComRawPointer> {
    let bits = self
      .allocation
      .read_usize(checked_offset(offset, "readPointer() offset")?)?;
    Ok(DynComRawPointer {
      kind: RawPointerKind::External(bits),
    })
  }

  #[napi]
  pub fn write_pointer(
    &self,
    offset: Either<BigInt, f64>,
    value: &DynComRawPointer,
  ) -> napi::Result<()> {
    let bits = value.address_bits()?;
    self
      .allocation
      .write_usize(checked_offset(offset, "writePointer() offset")?, bits)
  }
}

#[napi]
pub struct DynComRawPointer {
  kind: RawPointerKind,
}

impl DynComRawPointer {
  fn address_bits(&self) -> napi::Result<usize> {
    match &self.kind {
      RawPointerKind::Owned { allocation, offset } => allocation.pointer_address(*offset),
      RawPointerKind::ComBorrowed(state) => state.address(),
      RawPointerKind::DetachedCom(state) => state.address(),
      RawPointerKind::Consumed => Err(napi::Error::from_reason("Raw pointer has been consumed")),
      RawPointerKind::External(address) => Ok(*address),
    }
  }

  fn take_detached_com_value(&mut self) -> napi::Result<IUnknown> {
    match &self.kind {
      RawPointerKind::DetachedCom(state) => state.take_value(),
      RawPointerKind::Owned { .. }
      | RawPointerKind::ComBorrowed(_)
      | RawPointerKind::External(_) => Err(napi::Error::from_reason(
        "adoptTransferred() requires an RAII detached COM pointer",
      )),
      RawPointerKind::Consumed => Err(napi::Error::from_reason(
        "Raw pointer has already been consumed",
      )),
    }
  }

  fn take_assumed_transferred_address(&mut self) -> napi::Result<usize> {
    match &self.kind {
      RawPointerKind::External(address) => {
        let address = *address;
        self.kind = RawPointerKind::Consumed;
        Ok(address)
      }
      RawPointerKind::Owned { .. }
      | RawPointerKind::ComBorrowed(_)
      | RawPointerKind::DetachedCom(_) => Err(napi::Error::from_reason(
        "assumeTransferred() requires an unowned external pointer from caller-controlled storage",
      )),
      RawPointerKind::Consumed => Err(napi::Error::from_reason(
        "Raw pointer has already been consumed",
      )),
    }
  }

  fn take_external_cleanup_address(&mut self, operation: &str) -> napi::Result<usize> {
    match &self.kind {
      RawPointerKind::External(address) => {
        let address = *address;
        self.kind = RawPointerKind::Consumed;
        Ok(address)
      }
      RawPointerKind::Consumed => Err(napi::Error::from_reason(format!(
        "{operation}(): raw pointer has already been consumed",
      ))),
      _ => Err(napi::Error::from_reason(format!(
        "{operation}(): expected an unowned external resource pointer",
      ))),
    }
  }

  fn external_cleanup_address(&self, operation: &str) -> napi::Result<usize> {
    match &self.kind {
      RawPointerKind::External(address) => Ok(*address),
      RawPointerKind::Consumed => Err(napi::Error::from_reason(format!(
        "{operation}(): raw pointer has already been consumed",
      ))),
      _ => Err(napi::Error::from_reason(format!(
        "{operation}(): expected an unowned external resource pointer",
      ))),
    }
  }

  fn consume_external_cleanup(&mut self) {
    self.kind = RawPointerKind::Consumed;
  }
}

#[napi]
impl DynComRawPointer {
  #[napi(factory)]
  pub fn from_address(bits: Either<BigInt, f64>) -> napi::Result<Self> {
    js_usize(bits, "DynComRawPointer.fromAddress() bits").map(|address| Self {
      kind: RawPointerKind::External(address),
    })
  }

  #[napi(factory)]
  pub fn from_managed_borrowed(value: &DynWinRTValue) -> napi::Result<Self> {
    clone_managed_com_value(value, "DynComRawPointer.fromManagedBorrowed()").map(|value| Self {
      kind: RawPointerKind::ComBorrowed(RawComReference::new(value)),
    })
  }

  #[napi(factory)]
  pub fn null() -> Self {
    Self {
      kind: RawPointerKind::External(0),
    }
  }

  #[napi(getter)]
  pub fn address(&self) -> napi::Result<BigInt> {
    self.address_bits().map(|value| BigInt::from(value as u64))
  }

  #[napi(getter)]
  pub fn is_null(&self) -> napi::Result<bool> {
    self.address_bits().map(|value| value == 0)
  }

  #[napi]
  pub fn offset(&self, byte_offset: Either<BigInt, f64>) -> napi::Result<Self> {
    let byte_offset = js_usize(byte_offset, "DynComRawPointer.offset() byteOffset")?;
    let RawPointerKind::Owned { allocation, offset } = &self.kind else {
      return Err(napi::Error::from_reason(
        "DynComRawPointer.offset() requires an owned raw-memory pointer",
      ));
    };
    let offset = offset.checked_add(byte_offset).ok_or_else(|| {
      napi::Error::from_reason("DynComRawPointer.offset() byte offset overflowed")
    })?;
    allocation.pointer_address(offset)?;
    Ok(Self {
      kind: RawPointerKind::Owned {
        allocation: allocation.clone(),
        offset,
      },
    })
  }

  #[napi]
  pub fn to_value(&self) -> napi::Result<DynWinRTValue> {
    let address = self.address_bits()?;
    let value =
      dynwinrt::WinRTValue::RawPtr(std::ptr::with_exposed_provenance_mut::<c_void>(address));
    Ok(match &self.kind {
      RawPointerKind::Owned { allocation, .. } => {
        DynWinRTValue::with_pointer_owner(value, NativePointerOwner::RawMemory(allocation.clone()))
      }
      RawPointerKind::ComBorrowed(state) => {
        DynWinRTValue::with_pointer_owner(value, NativePointerOwner::RawCom(state.clone()))
      }
      RawPointerKind::DetachedCom(state) => {
        DynWinRTValue::with_detached_com_owner(value, state.clone())
      }
      RawPointerKind::Consumed => {
        return Err(napi::Error::from_reason("Raw pointer has been consumed"));
      }
      RawPointerKind::External(_) => DynWinRTValue::with_borrowed_pointer(value),
    })
  }
}

fn optional_offset(value: Option<Either<BigInt, f64>>, context: &str) -> napi::Result<usize> {
  value
    .map(|value| js_usize(value, context))
    .transpose()
    .map(|value| value.unwrap_or(0))
}

fn validate_raw_qualified_name(name: &str) -> napi::Result<()> {
  let segments = name.split('.').collect::<Vec<_>>();
  if segments.len() < 2
    || segments
      .iter()
      .any(|segment| segment.is_empty() || segment.trim() != *segment)
  {
    return Err(napi::Error::from_reason(
      "Raw aggregate identity must be a qualified dot-separated name",
    ));
  }
  Ok(())
}

#[derive(Clone, Copy)]
enum RawDescriptorKind {
  Struct,
  Union,
}

#[derive(Clone, Copy)]
struct RawAggregateMetrics {
  size: usize,
  expanded_fields: usize,
  libffi_elements: usize,
}

#[derive(Clone, Copy)]
struct RawFieldMetrics {
  size: usize,
  expanded_fields: usize,
  libffi_elements: usize,
}

fn validate_raw_aggregate_descriptor_limits(
  descriptor: &str,
  kind: RawDescriptorKind,
) -> napi::Result<()> {
  let root: serde_json::Value = serde_json::from_str(descriptor).map_err(|error| {
    napi::Error::from_reason(format!("Invalid raw aggregate descriptor: {error}"))
  })?;
  let name = root
    .get("name")
    .and_then(serde_json::Value::as_str)
    .ok_or_else(|| napi::Error::from_reason("Raw aggregate descriptor is missing `name`"))?;
  let layout = root.get(raw_host_architecture()).ok_or_else(|| {
    napi::Error::from_reason(format!(
      "Raw aggregate descriptor is missing `{}`",
      raw_host_architecture()
    ))
  })?;
  validate_raw_layout_limits(name, layout, kind, &mut Vec::new()).map(|_| ())
}

fn validate_raw_by_value_descriptor(descriptor: &str, kind: RawDescriptorKind) -> napi::Result<()> {
  let root: serde_json::Value = serde_json::from_str(descriptor).map_err(|error| {
    napi::Error::from_reason(format!("Invalid raw aggregate descriptor: {error}"))
  })?;
  validate_raw_by_value_schema(&root, kind)?;
  let layout = root.get(raw_host_architecture()).ok_or_else(|| {
    napi::Error::from_reason(format!(
      "Raw aggregate descriptor is missing `{}`",
      raw_host_architecture()
    ))
  })?;
  let contains_union = validate_raw_by_value_layout(layout, kind, true)?;
  #[cfg(target_arch = "aarch64")]
  if contains_union {
    return Err(napi::Error::from_reason(
      "Raw ARM64 by-value unions are compile-validated but disabled until an executable ARM64 ABI oracle is available",
    ));
  }
  let _ = contains_union;
  Ok(())
}

fn validate_raw_by_value_schema(
  root: &serde_json::Value,
  kind: RawDescriptorKind,
) -> napi::Result<()> {
  let root_keys: &[&str] = match kind {
    RawDescriptorKind::Struct => &["name", "x86", "x64", "arm64", "initializers"],
    RawDescriptorKind::Union => &["name", "x86", "x64", "arm64"],
  };
  validate_exact_raw_keys(root, root_keys, "root descriptor")?;
  let name = root
    .get("name")
    .and_then(serde_json::Value::as_str)
    .ok_or_else(|| napi::Error::from_reason("Raw aggregate descriptor is missing `name`"))?;
  validate_raw_qualified_name(name)?;

  let mut architecture_count = 0usize;
  for architecture in ["x86", "x64", "arm64"] {
    if let Some(layout) = root.get(architecture) {
      architecture_count += 1;
      validate_raw_schema_layout(name, layout, kind, &mut Vec::new())?;
    }
  }
  if architecture_count == 0 {
    return Err(napi::Error::from_reason(
      "Raw aggregate descriptor must contain at least one architecture layout",
    ));
  }

  if let Some(initializers) = root.get("initializers") {
    let initializers = initializers.as_array().ok_or_else(|| {
      napi::Error::from_reason("Raw struct descriptor `initializers` must be an array")
    })?;
    for initializer in initializers {
      validate_exact_raw_keys(initializer, &["kind", "field"], "initializer")?;
      let initializer_kind = initializer
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| napi::Error::from_reason("Raw struct initializer is missing `kind`"))?;
      if initializer_kind != "sizeOfLayout" {
        return Err(napi::Error::from_reason(format!(
          "Unsupported raw struct initializer `{initializer_kind}`"
        )));
      }
      initializer
        .get("field")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| napi::Error::from_reason("Raw struct initializer is missing `field`"))?;
    }
  }
  Ok(())
}

fn validate_raw_schema_layout(
  name: &str,
  layout: &serde_json::Value,
  kind: RawDescriptorKind,
  stack: &mut Vec<String>,
) -> napi::Result<()> {
  validate_raw_qualified_name(name)?;
  if stack.len() >= RAW_MAX_AGGREGATE_NESTING_DEPTH {
    return Err(napi::Error::from_reason(format!(
      "Raw aggregate `{name}` exceeds the nesting limit of {RAW_MAX_AGGREGATE_NESTING_DEPTH}",
    )));
  }
  if stack.iter().any(|active| active == name) {
    return Err(napi::Error::from_reason(format!(
      "Raw aggregate descriptor contains a recursive cycle through `{name}`",
    )));
  }
  stack.push(name.to_string());
  let result = validate_raw_schema_layout_body(name, layout, kind, stack);
  stack.pop();
  result
}

fn validate_raw_schema_layout_body(
  name: &str,
  layout: &serde_json::Value,
  kind: RawDescriptorKind,
  stack: &mut Vec<String>,
) -> napi::Result<()> {
  match kind {
    RawDescriptorKind::Struct => {
      validate_exact_raw_keys(layout, &["size", "alignment", "fields"], "struct layout")?
    }
    RawDescriptorKind::Union => {
      validate_exact_raw_keys(
        layout,
        &["size", "alignment", "fields", "complete"],
        "union layout",
      )?;
      if layout.get("complete").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err(napi::Error::from_reason(format!(
          "Raw by-value union `{name}` requires `complete: true`",
        )));
      }
    }
  }
  raw_descriptor_usize(layout, "size")?;
  raw_descriptor_usize(layout, "alignment")?;
  let fields = layout
    .get("fields")
    .and_then(serde_json::Value::as_array)
    .ok_or_else(|| napi::Error::from_reason("Raw aggregate descriptor is missing `fields`"))?;
  for field in fields {
    let allowed_keys: &[&str] = match kind {
      RawDescriptorKind::Struct => &["name", "offset", "count", "type"],
      RawDescriptorKind::Union => &["name", "count", "type"],
    };
    validate_exact_raw_keys(field, allowed_keys, "aggregate field")?;
    field
      .get("name")
      .and_then(serde_json::Value::as_str)
      .ok_or_else(|| napi::Error::from_reason("Raw aggregate field is missing `name`"))?;
    if matches!(kind, RawDescriptorKind::Struct) {
      raw_descriptor_usize(field, "offset")?;
    }
    let count = raw_descriptor_usize(field, "count")?;
    if count == 0 {
      return Err(napi::Error::from_reason(
        "Raw aggregate field `count` must be nonzero",
      ));
    }
    let typ = field
      .get("type")
      .ok_or_else(|| napi::Error::from_reason("Raw aggregate field is missing `type`"))?;
    validate_raw_schema_type(typ, stack)?;
  }
  Ok(())
}

fn validate_raw_schema_type(typ: &serde_json::Value, stack: &mut Vec<String>) -> napi::Result<()> {
  let kind = typ
    .get("kind")
    .and_then(serde_json::Value::as_str)
    .ok_or_else(|| napi::Error::from_reason("Raw aggregate field type is missing `kind`"))?;
  match kind {
    "i8" | "u8" | "i16" | "u16" | "i32" | "u32" | "i64" | "u64" | "f32" | "f64" | "isize"
    | "usize" | "guid" | "pointer" => validate_exact_raw_keys(typ, &["kind"], "scalar field type"),
    "struct" | "union" => {
      validate_exact_raw_keys(typ, &["kind", "name", "layout"], "nested aggregate type")?;
      let name = typ
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| napi::Error::from_reason("Nested raw aggregate is missing `name`"))?;
      let layout = typ
        .get("layout")
        .ok_or_else(|| napi::Error::from_reason("Nested raw aggregate is missing `layout`"))?;
      validate_raw_schema_layout(
        name,
        layout,
        if kind == "struct" {
          RawDescriptorKind::Struct
        } else {
          RawDescriptorKind::Union
        },
        stack,
      )
    }
    _ => Err(napi::Error::from_reason(format!(
      "Unsupported raw by-value aggregate field kind `{kind}`",
    ))),
  }
}

fn validate_exact_raw_keys(
  value: &serde_json::Value,
  allowed: &[&str],
  context: &str,
) -> napi::Result<()> {
  let object = value
    .as_object()
    .ok_or_else(|| napi::Error::from_reason(format!("Raw by-value {context} must be an object")))?;
  for key in object.keys() {
    if allowed.iter().any(|allowed| *allowed == key) {
      continue;
    }
    if is_explicitly_unsupported_raw_key(key) {
      return Err(napi::Error::from_reason(format!(
        "Raw by-value {context} uses unsupported ABI marker `{key}`",
      )));
    }
    return Err(napi::Error::from_reason(format!(
      "Raw by-value {context} contains unknown key `{key}`",
    )));
  }
  Ok(())
}

fn is_explicitly_unsupported_raw_key(key: &str) -> bool {
  let normalized = key
    .chars()
    .filter(|character| !matches!(character, '-' | '_'))
    .flat_map(char::to_lowercase)
    .collect::<String>();
  matches!(
    normalized.as_str(),
    "packed"
      | "pack"
      | "bitwidth"
      | "bitfield"
      | "vector"
      | "hva"
      | "flexible"
      | "flexiblearray"
      | "overaligned"
      | "customalignment"
      | "opaque"
      | "nontrivial"
      | "selectedmember"
      | "selectedmemberonly"
      | "incomplete"
  )
}

fn validate_raw_by_value_layout(
  layout: &serde_json::Value,
  kind: RawDescriptorKind,
  top_level: bool,
) -> napi::Result<bool> {
  let size = raw_descriptor_usize(layout, "size")?;
  if matches!(kind, RawDescriptorKind::Union)
    && layout.get("complete").and_then(serde_json::Value::as_bool) != Some(true)
  {
    return Err(napi::Error::from_reason(
      "Raw by-value union layouts require `complete: true` to assert that every alternative is described",
    ));
  }
  #[cfg(not(target_arch = "x86_64"))]
  let _ = size;
  #[cfg(not(target_arch = "x86_64"))]
  let _ = top_level;
  #[cfg(target_arch = "x86_64")]
  if top_level && matches!(size, 3 | 5 | 6 | 7) {
    return Err(napi::Error::from_reason(format!(
      "Raw Win64 by-value aggregate size {size} is rejected because bundled libffi 3.5.2 has an irregular-size argument passing/copy defect for 3/5/6/7-byte structures",
    )));
  }
  let fields = layout
    .get("fields")
    .and_then(serde_json::Value::as_array)
    .ok_or_else(|| napi::Error::from_reason("Raw aggregate descriptor is missing `fields`"))?;
  let mut contains_union = matches!(kind, RawDescriptorKind::Union);
  for field in fields {
    let typ = field
      .get("type")
      .ok_or_else(|| napi::Error::from_reason("Raw aggregate field is missing `type`"))?;
    match typ.get("kind").and_then(serde_json::Value::as_str) {
      Some("struct") | Some("union") => {
        let nested_kind = if typ.get("kind").and_then(serde_json::Value::as_str) == Some("union") {
          RawDescriptorKind::Union
        } else {
          RawDescriptorKind::Struct
        };
        let nested = typ
          .get("layout")
          .ok_or_else(|| napi::Error::from_reason("Nested raw aggregate is missing `layout`"))?;
        contains_union |= validate_raw_by_value_layout(nested, nested_kind, false)?;
      }
      Some(_) => {}
      None => {
        return Err(napi::Error::from_reason(
          "Raw aggregate field type is missing `kind`",
        ));
      }
    }
  }
  Ok(contains_union)
}

fn validate_raw_layout_limits(
  name: &str,
  layout: &serde_json::Value,
  kind: RawDescriptorKind,
  stack: &mut Vec<String>,
) -> napi::Result<RawAggregateMetrics> {
  validate_raw_qualified_name(name)?;
  if stack.len() >= RAW_MAX_AGGREGATE_NESTING_DEPTH {
    return Err(napi::Error::from_reason(format!(
      "Raw aggregate `{name}` exceeds the nesting limit of {RAW_MAX_AGGREGATE_NESTING_DEPTH}",
    )));
  }
  if stack.iter().any(|active| active == name) {
    return Err(napi::Error::from_reason(format!(
      "Raw aggregate descriptor contains a recursive cycle through `{name}`",
    )));
  }
  stack.push(name.to_string());
  let result = validate_raw_layout_limits_body(name, layout, kind, stack);
  stack.pop();
  result
}

fn validate_raw_layout_limits_body(
  name: &str,
  layout: &serde_json::Value,
  kind: RawDescriptorKind,
  stack: &mut Vec<String>,
) -> napi::Result<RawAggregateMetrics> {
  let size = raw_descriptor_usize(layout, "size")?;
  if size > RAW_MAX_AGGREGATE_BYTE_SIZE {
    return Err(napi::Error::from_reason(format!(
      "Raw aggregate `{name}` size {size} exceeds the {RAW_MAX_AGGREGATE_BYTE_SIZE}-byte limit",
    )));
  }
  let fields = layout
    .get("fields")
    .and_then(serde_json::Value::as_array)
    .ok_or_else(|| napi::Error::from_reason("Raw aggregate descriptor is missing `fields`"))?;
  if fields.len() > RAW_MAX_FIXED_FIELD_EXPANSION {
    return Err(napi::Error::from_reason(format!(
      "Raw aggregate `{name}` field count exceeds the {RAW_MAX_FIXED_FIELD_EXPANSION}-field expansion limit",
    )));
  }

  let mut expanded_fields = 0usize;
  let mut libffi_elements = 0usize;
  let mut struct_fields = Vec::new();
  if matches!(kind, RawDescriptorKind::Struct) {
    struct_fields
      .try_reserve_exact(fields.len())
      .map_err(|_| napi::Error::from_reason("Raw aggregate field validation allocation failed"))?;
  }
  for field in fields {
    let count = raw_descriptor_usize(field, "count")?;
    if count == 0 || count > RAW_MAX_FIXED_FIELD_EXPANSION {
      return Err(napi::Error::from_reason(format!(
        "Raw aggregate `{name}` field count must be between 1 and {RAW_MAX_FIXED_FIELD_EXPANSION}",
      )));
    }
    let typ = field
      .get("type")
      .ok_or_else(|| napi::Error::from_reason("Raw aggregate field is missing `type`"))?;
    let metrics = raw_field_metrics(typ, stack)?;
    let field_expansion = metrics
      .expanded_fields
      .checked_mul(count)
      .ok_or_else(|| napi::Error::from_reason("Raw aggregate field expansion overflow"))?;
    expanded_fields = expanded_fields
      .checked_add(field_expansion)
      .ok_or_else(|| napi::Error::from_reason("Raw aggregate field expansion overflow"))?;
    if expanded_fields > RAW_MAX_FIXED_FIELD_EXPANSION {
      return Err(napi::Error::from_reason(format!(
        "Raw aggregate `{name}` expands beyond {RAW_MAX_FIXED_FIELD_EXPANSION} fixed fields",
      )));
    }

    if matches!(kind, RawDescriptorKind::Struct) {
      let offset = raw_descriptor_usize(field, "offset")?;
      let field_size = metrics
        .size
        .checked_mul(count)
        .ok_or_else(|| napi::Error::from_reason("Raw aggregate field byte size overflow"))?;
      let field_ffi_elements = metrics
        .libffi_elements
        .checked_mul(count)
        .ok_or_else(|| napi::Error::from_reason("Raw aggregate libffi expansion overflow"))?;
      struct_fields.push((offset, field_size, field_ffi_elements));
    }
  }

  if matches!(kind, RawDescriptorKind::Struct) {
    struct_fields.sort_unstable_by_key(|field| field.0);
    let mut cursor = 0usize;
    for (offset, field_size, field_ffi_elements) in struct_fields {
      let padding = offset.checked_sub(cursor).ok_or_else(|| {
        napi::Error::from_reason(format!("Raw aggregate `{name}` fields overlap"))
      })?;
      libffi_elements = libffi_elements
        .checked_add(padding)
        .and_then(|value| value.checked_add(field_ffi_elements))
        .ok_or_else(|| napi::Error::from_reason("Raw aggregate libffi expansion overflow"))?;
      if libffi_elements > RAW_MAX_LIBFFI_ELEMENTS {
        return Err(napi::Error::from_reason(format!(
          "Raw aggregate `{name}` expands beyond {RAW_MAX_LIBFFI_ELEMENTS} libffi elements",
        )));
      }
      cursor = offset
        .checked_add(field_size)
        .ok_or_else(|| napi::Error::from_reason("Raw aggregate field end overflow"))?;
    }
    let tail_padding = size.checked_sub(cursor).ok_or_else(|| {
      napi::Error::from_reason(format!("Raw aggregate `{name}` fields exceed its size"))
    })?;
    libffi_elements = libffi_elements
      .checked_add(tail_padding)
      .ok_or_else(|| napi::Error::from_reason("Raw aggregate libffi expansion overflow"))?;
    if libffi_elements > RAW_MAX_LIBFFI_ELEMENTS {
      return Err(napi::Error::from_reason(format!(
        "Raw aggregate `{name}` expands beyond {RAW_MAX_LIBFFI_ELEMENTS} libffi elements",
      )));
    }
  }

  Ok(RawAggregateMetrics {
    size,
    expanded_fields,
    libffi_elements,
  })
}

fn raw_field_metrics(
  typ: &serde_json::Value,
  stack: &mut Vec<String>,
) -> napi::Result<RawFieldMetrics> {
  let kind = typ
    .get("kind")
    .and_then(serde_json::Value::as_str)
    .ok_or_else(|| napi::Error::from_reason("Raw aggregate field type is missing `kind`"))?;
  let scalar = |size| {
    Ok(RawFieldMetrics {
      size,
      expanded_fields: 1,
      libffi_elements: 1,
    })
  };
  match kind {
    "i8" | "u8" => scalar(1),
    "i16" | "u16" => scalar(2),
    "i32" | "u32" | "f32" => scalar(4),
    "i64" | "u64" | "f64" => scalar(8),
    "isize" | "usize" | "pointer" => scalar(std::mem::size_of::<usize>()),
    "guid" => Ok(RawFieldMetrics {
      size: 16,
      expanded_fields: 1,
      libffi_elements: 11,
    }),
    "struct" => {
      let name = typ
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| napi::Error::from_reason("Nested raw struct is missing `name`"))?;
      let layout = typ
        .get("layout")
        .ok_or_else(|| napi::Error::from_reason("Nested raw struct is missing `layout`"))?;
      let metrics = validate_raw_layout_limits(name, layout, RawDescriptorKind::Struct, stack)?;
      Ok(RawFieldMetrics {
        size: metrics.size,
        expanded_fields: metrics.expanded_fields,
        libffi_elements: metrics.libffi_elements,
      })
    }
    "union" => {
      let name = typ
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| napi::Error::from_reason("Nested raw union is missing `name`"))?;
      let layout = typ
        .get("layout")
        .ok_or_else(|| napi::Error::from_reason("Nested raw union is missing `layout`"))?;
      let metrics = validate_raw_layout_limits(name, layout, RawDescriptorKind::Union, stack)?;
      Ok(RawFieldMetrics {
        size: metrics.size,
        expanded_fields: metrics.expanded_fields,
        // A nonhomogeneous carrier can expand to one element per byte. This
        // conservative bound keeps raw libffi construction within its limit.
        libffi_elements: metrics.size,
      })
    }
    _ => Err(napi::Error::from_reason(format!(
      "Unsupported raw aggregate field kind `{kind}`",
    ))),
  }
}

fn raw_descriptor_usize(value: &serde_json::Value, name: &str) -> napi::Result<usize> {
  value
    .get(name)
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .ok_or_else(|| {
      napi::Error::from_reason(format!("Raw aggregate descriptor has invalid `{name}`"))
    })
}

fn raw_host_architecture() -> &'static str {
  #[cfg(target_arch = "x86")]
  {
    "x86"
  }
  #[cfg(target_arch = "x86_64")]
  {
    "x64"
  }
  #[cfg(target_arch = "aarch64")]
  {
    "arm64"
  }
  #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
  {
    "unsupported"
  }
}

unsafe fn raw_global_free(pointer: *mut c_void) -> *mut c_void {
  windows::core::link!("kernel32.dll" "system" fn GlobalFree(memory: *mut c_void) -> *mut c_void);
  unsafe { GlobalFree(pointer) }
}

fn try_copy_buffer(value: &Buffer, context: &str) -> napi::Result<Vec<u8>> {
  let mut bytes = Vec::new();
  bytes
    .try_reserve_exact(value.len())
    .map_err(|_| napi::Error::from_reason(format!("{context} byte allocation failed")))?;
  bytes.extend_from_slice(value.as_ref());
  Ok(bytes)
}

fn checked_offset(value: Either<BigInt, f64>, context: &str) -> napi::Result<usize> {
  js_usize(value, context)
}

fn js_usize(value: Either<BigInt, f64>, context: &str) -> napi::Result<usize> {
  match value {
    Either::A(value) => bigint_usize(value, context),
    Either::B(value)
      if value.is_finite()
        && value >= 0.0
        && value.fract() == 0.0
        && value <= MAX_SAFE_INTEGER
        && value as u64 as usize as u64 == value as u64 =>
    {
      Ok(value as usize)
    }
    Either::B(_) => Err(napi::Error::from_reason(format!(
      "{context} must be a non-negative safe integer that fits in a pointer-width value",
    ))),
  }
}

fn bigint_usize(value: BigInt, context: &str) -> napi::Result<usize> {
  let (negative, value, lossless) = value.get_u64();
  if negative || !lossless || value as usize as u64 != value {
    return Err(napi::Error::from_reason(format!(
      "{context} must fit in an unsigned pointer-width integer",
    )));
  }
  Ok(value as usize)
}

fn checked_signed_number(value: f64, min: i64, max: i64, context: &str) -> napi::Result<i64> {
  if !value.is_finite() || value.fract() != 0.0 || value < min as f64 || value > max as f64 {
    return Err(napi::Error::from_reason(format!(
      "{context} must be an integer in the range {min} through {max}",
    )));
  }
  Ok(value as i64)
}

fn checked_unsigned_number(value: f64, max: u64, context: &str) -> napi::Result<u64> {
  if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value > max as f64 {
    return Err(napi::Error::from_reason(format!(
      "{context} must be an integer in the range 0 through {max}",
    )));
  }
  Ok(value as u64)
}

#[cfg(test)]
mod tests {
  use std::ffi::c_void;
  use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

  use windows::core::HRESULT;

  use super::*;
  use crate::com::{take_native_output_pointer, with_com_invocation_args, PointerProvenance};

  fn number(value: usize) -> Either<BigInt, f64> {
    Either::B(value as f64)
  }

  fn raw_memory(size: usize, alignment: usize) -> DynComRawMemory {
    DynComRawMemory {
      allocation: RawAllocation::allocate(size, alignment).unwrap(),
    }
  }

  const IID_RAW_TRACKED_ALT: windows::core::GUID =
    windows::core::GUID::from_u128(0x12345678_9abc_def0_1234_56789abcdef0);

  #[repr(C)]
  struct RawTrackedComObject {
    vtable: *const windows::core::IUnknown_Vtbl,
    addrefs: AtomicU32,
    releases: AtomicU32,
  }

  unsafe extern "system" fn raw_tracked_query_interface(
    this: *mut c_void,
    iid: *const windows::core::GUID,
    result: *mut *mut c_void,
  ) -> HRESULT {
    let iid = unsafe { *iid };
    if iid == windows::core::IUnknown::IID || iid == IID_RAW_TRACKED_ALT {
      unsafe {
        *result = this;
        raw_tracked_add_ref(this);
      }
      HRESULT(0)
    } else {
      unsafe {
        *result = std::ptr::null_mut();
      }
      HRESULT(0x80004002u32 as i32)
    }
  }

  unsafe extern "system" fn raw_tracked_add_ref(this: *mut c_void) -> u32 {
    let object = unsafe { &*this.cast::<RawTrackedComObject>() };
    object.addrefs.fetch_add(1, AtomicOrdering::Relaxed) + 2
  }

  unsafe extern "system" fn raw_tracked_release(this: *mut c_void) -> u32 {
    let object = unsafe { &*this.cast::<RawTrackedComObject>() };
    object.releases.fetch_add(1, AtomicOrdering::Relaxed);
    1
  }

  static RAW_TRACKED_VTABLE: windows::core::IUnknown_Vtbl = windows::core::IUnknown_Vtbl {
    QueryInterface: raw_tracked_query_interface,
    AddRef: raw_tracked_add_ref,
    Release: raw_tracked_release,
  };

  fn raw_tracked_managed(object: &mut RawTrackedComObject) -> DynWinRTValue {
    let unknown = unsafe { IUnknown::from_raw((object as *mut RawTrackedComObject).cast()) };
    let mut value = DynWinRTValue::new(dynwinrt::WinRTValue::Object(unknown));
    value.bind_current_com_apartment().unwrap();
    object.addrefs.store(0, AtomicOrdering::Relaxed);
    object.releases.store(0, AtomicOrdering::Relaxed);
    value
  }

  struct SendRawOwnerForWrongThreadTest(DynComRawOwnedComPointer);

  // Safety: the test moves the owner only to verify that access is rejected.
  // Wrong-thread Drop leaks the IUnknown without invoking its vtable.
  unsafe impl Send for SendRawOwnerForWrongThreadTest {}

  #[repr(C)]
  struct RawPointerReplacementCall {
    vtable: *const *mut c_void,
    replacement: *mut c_void,
    saw_initial: bool,
  }

  unsafe extern "system" fn raw_open_namespace_preserve_old_replace(
    this: *mut c_void,
    slot: *mut *mut c_void,
  ) -> HRESULT {
    let call = unsafe { &mut *this.cast::<RawPointerReplacementCall>() };
    call.saw_initial = unsafe { !(*slot).is_null() };
    unsafe {
      raw_tracked_add_ref(call.replacement);
      *slot = call.replacement;
    }
    HRESULT(0)
  }

  unsafe extern "system" fn raw_open_namespace_consume_old_replace(
    this: *mut c_void,
    slot: *mut *mut c_void,
  ) -> HRESULT {
    let call = unsafe { &mut *this.cast::<RawPointerReplacementCall>() };
    call.saw_initial = unsafe { !(*slot).is_null() };
    if !call.saw_initial {
      return HRESULT(0x80004003u32 as i32);
    }
    unsafe {
      raw_tracked_release(*slot);
      raw_tracked_add_ref(call.replacement);
      *slot = call.replacement;
    }
    HRESULT(0)
  }

  unsafe extern "system" fn raw_open_namespace_unchanged(
    this: *mut c_void,
    slot: *mut *mut c_void,
  ) -> HRESULT {
    let call = unsafe { &mut *this.cast::<RawPointerReplacementCall>() };
    call.saw_initial = unsafe { !(*slot).is_null() };
    if call.saw_initial {
      HRESULT(0)
    } else {
      HRESULT(0x80004003u32 as i32)
    }
  }

  fn invoke_raw_pointer_slot(
    function: *mut c_void,
    call: &mut RawPointerReplacementCall,
    value: &DynWinRTValue,
  ) {
    let table = dynwinrt::MetadataTable::new();
    let interface = dynwinrt::com::register_interface(
      &table,
      "Tests.IRawPointerReplacement",
      windows::core::GUID::from_u128(0xabcdefab_cdef_abcd_efab_cdefabcdefab),
      dynwinrt::com::InterfaceBase::IUnknown,
    )
    .add_method_at(
      3,
      "Replace",
      dynwinrt::com::MethodSignature::new(&table).add_in(dynwinrt::com::Type::pointer()),
    )
    .unwrap();
    let mut vtable = [std::ptr::null_mut(); 4];
    vtable[3] = function;
    call.vtable = vtable.as_ptr();
    with_com_invocation_args(&[value], |args| {
      unsafe {
        interface
          .method(3)
          .unwrap()
          .invoke_values_with_output_kinds((call as *mut RawPointerReplacementCall).cast(), args)
      }
      .map(|_| ())
      .map_err(|error| napi::Error::from_reason(error.message()))
    })
    .unwrap();
  }

  enum RawComReentrantAction {
    Release,
    IntoManaged,
    Finalize,
  }

  #[repr(C)]
  struct RawComReentrantCall {
    vtable: *const *mut c_void,
    state: Arc<RawComReference>,
    action: RawComReentrantAction,
    logical_release_visible: bool,
    used_pointer_after_release: bool,
  }

  unsafe extern "system" fn release_raw_com_then_use_pointer(
    this: *mut c_void,
    pointer: *mut c_void,
  ) -> HRESULT {
    let call = unsafe { &mut *this.cast::<RawComReentrantCall>() };
    match call.action {
      RawComReentrantAction::Release => {
        call.state.release().unwrap();
      }
      RawComReentrantAction::IntoManaged => {
        let value = call.state.take_value().unwrap();
        let mut managed = managed_from_owned_com_value(value).unwrap();
        managed.release().unwrap();
      }
      RawComReentrantAction::Finalize => {
        call.state.finalize_owner();
      }
    }
    call.logical_release_visible = call.state.validate_live().is_err();
    if call.logical_release_visible && !pointer.is_null() {
      unsafe {
        raw_tracked_add_ref(pointer);
        raw_tracked_release(pointer);
      }
      call.used_pointer_after_release = true;
    }
    HRESULT(0)
  }

  fn invoke_reentrant_raw_com(
    call: &mut RawComReentrantCall,
    value: &DynWinRTValue,
  ) -> napi::Result<()> {
    let table = dynwinrt::MetadataTable::new();
    let interface = dynwinrt::com::register_interface(
      &table,
      "Tests.IReentrantRawCom",
      windows::core::GUID::from_u128(0xfedcbafe_dcba_fedc_bafe_dcbafedcbafe),
      dynwinrt::com::InterfaceBase::IUnknown,
    )
    .add_method_at(
      3,
      "ReleaseThenUse",
      dynwinrt::com::MethodSignature::new(&table).add_in(dynwinrt::com::Type::pointer()),
    )
    .unwrap();
    let mut vtable = [std::ptr::null_mut(); 4];
    vtable[3] = release_raw_com_then_use_pointer as *mut c_void;
    call.vtable = vtable.as_ptr();
    with_com_invocation_args(&[value], |args| {
      unsafe {
        interface
          .method(3)
          .unwrap()
          .invoke_values_with_output_kinds((call as *mut RawComReentrantCall).cast(), args)
      }
      .map(|_| ())
      .map_err(|error| napi::Error::from_reason(error.message()))
    })
  }

  fn invoke_raw_contract_method(
    function: *mut c_void,
    object: *mut c_void,
    signature: dynwinrt::com::MethodSignature,
    args: &[&DynWinRTValue],
  ) -> napi::Result<Vec<DynWinRTValue>> {
    let table = dynwinrt::MetadataTable::new();
    let interface = dynwinrt::com::register_interface(
      &table,
      "Tests.IFinalRawAbiMatrix",
      windows::core::GUID::from_u128(0x6b1a6b1a_6b1a_6b1a_6b1a_6b1a6b1a6b1a),
      dynwinrt::com::InterfaceBase::IUnknown,
    )
    .add_method_at(3, "Call", signature)
    .map_err(|error| napi::Error::from_reason(error.message()))?;
    let mut vtable = [std::ptr::null_mut(); 4];
    vtable[3] = function;
    // Safety: every matrix object is repr(C) with its vtable pointer first and
    // remains live until this synchronous invocation returns.
    unsafe {
      object.cast::<*const *mut c_void>().write(vtable.as_ptr());
    }
    with_com_invocation_args(args, |args| {
      unsafe {
        interface
          .method(3)
          .expect("registered final raw ABI matrix method")
          .invoke_values_with_output_kinds(object, args)
      }
      .map(|values| {
        values
          .into_iter()
          .map(|(value, kind)| DynWinRTValue::from_com_value(value, kind))
          .collect()
      })
      .map_err(|error| napi::Error::from_reason(error.message()))
    })
  }

  fn raw_value(pointer: &DynComRawPointer) -> DynWinRTValue {
    pointer.to_value().unwrap()
  }

  fn raw_guid_memory(value: windows::core::GUID) -> DynComRawMemory {
    let memory = raw_memory(
      std::mem::size_of::<windows::core::GUID>(),
      std::mem::align_of::<windows::core::GUID>(),
    );
    let bytes = unsafe {
      std::slice::from_raw_parts(
        (&value as *const windows::core::GUID).cast::<u8>(),
        std::mem::size_of::<windows::core::GUID>(),
      )
    };
    memory
      .write_bytes(number(0), Buffer::from(bytes.to_vec()))
      .unwrap();
    memory
  }

  enum RawPrivateDataPayload {
    Bytes,
    Interface { fail: bool },
  }

  #[repr(C)]
  struct RawPrivateDataCall {
    vtable: *const *mut c_void,
    expected_guid: windows::core::GUID,
    bytes: [u8; 6],
    interface: *mut c_void,
    payload: RawPrivateDataPayload,
    calls: u32,
  }

  unsafe extern "system" fn raw_get_private_data_matrix(
    this: *mut c_void,
    guid: *const windows::core::GUID,
    size: *mut u32,
    data: *mut c_void,
  ) -> HRESULT {
    let call = unsafe { &mut *this.cast::<RawPrivateDataCall>() };
    call.calls += 1;
    if guid.is_null() || size.is_null() || unsafe { *guid } != call.expected_guid {
      return HRESULT(0x80070057u32 as i32);
    }
    match call.payload {
      RawPrivateDataPayload::Bytes => {
        let required = call.bytes.len() as u32;
        if data.is_null() {
          unsafe { *size = required };
          return HRESULT(0);
        }
        let capacity = unsafe { *size };
        unsafe { *size = required };
        if capacity < required {
          return HRESULT(0x887a0003u32 as i32);
        }
        unsafe {
          std::ptr::copy_nonoverlapping(call.bytes.as_ptr(), data.cast(), call.bytes.len());
        }
        HRESULT(0)
      }
      RawPrivateDataPayload::Interface { fail } => {
        let required = std::mem::size_of::<usize>() as u32;
        if data.is_null() {
          unsafe { *size = required };
          return HRESULT(0);
        }
        let capacity = unsafe { *size };
        unsafe { *size = required };
        if capacity < required {
          return HRESULT(0x887a0003u32 as i32);
        }
        unsafe {
          raw_tracked_add_ref(call.interface);
          data.cast::<*mut c_void>().write(call.interface);
        }
        if fail {
          HRESULT(0x80004005u32 as i32)
        } else {
          HRESULT(0)
        }
      }
    }
  }

  enum RawFormatSupport {
    Supported,
    Closest,
    Failure,
  }

  #[repr(C)]
  struct RawFormatSupportCall {
    vtable: *const *mut c_void,
    mode: RawFormatSupport,
    calls: u32,
  }

  unsafe extern "system" fn raw_is_format_supported_matrix(
    this: *mut c_void,
    share_mode: windows::Win32::Media::Audio::AUDCLNT_SHAREMODE,
    format: *const windows::Win32::Media::Audio::WAVEFORMATEX,
    closest: *mut *mut windows::Win32::Media::Audio::WAVEFORMATEX,
  ) -> HRESULT {
    let call = unsafe { &mut *this.cast::<RawFormatSupportCall>() };
    call.calls += 1;
    if format.is_null() {
      return HRESULT(0x80070057u32 as i32);
    }
    let format_pointer = format;
    let format = unsafe { format_pointer.read_unaligned() };
    let channels = format.nChannels;
    let samples = format.nSamplesPerSec;
    let extra = format.cbSize as usize;
    if channels != 2 || samples != 48_000 || extra != 4 {
      return HRESULT(0x80070057u32 as i32);
    }
    let extension = unsafe {
      std::slice::from_raw_parts(
        format_pointer.cast::<u8>().add(std::mem::size_of::<
          windows::Win32::Media::Audio::WAVEFORMATEX,
        >()),
        extra,
      )
    };
    if extension != [9, 8, 7, 6] {
      return HRESULT(0x80070057u32 as i32);
    }
    use windows::Win32::Media::Audio::{AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_SHAREMODE_SHARED};
    if share_mode == AUDCLNT_SHAREMODE_SHARED && closest.is_null() {
      return HRESULT(0x80004003u32 as i32);
    }
    if share_mode == AUDCLNT_SHAREMODE_EXCLUSIVE {
      if !closest.is_null() {
        unsafe { *closest = std::ptr::null_mut() };
      }
      return match call.mode {
        RawFormatSupport::Supported => HRESULT(0),
        RawFormatSupport::Closest | RawFormatSupport::Failure => HRESULT(0x88890008u32 as i32),
      };
    }
    if share_mode != AUDCLNT_SHAREMODE_SHARED {
      return HRESULT(0x80070057u32 as i32);
    }
    match call.mode {
      RawFormatSupport::Supported => {
        unsafe { *closest = std::ptr::null_mut() };
        HRESULT(0)
      }
      RawFormatSupport::Closest => {
        let total = std::mem::size_of::<windows::Win32::Media::Audio::WAVEFORMATEX>() + extra;
        let output = unsafe { windows::Win32::System::Com::CoTaskMemAlloc(total) }.cast::<u8>();
        if output.is_null() {
          return HRESULT(0x8007000eu32 as i32);
        }
        unsafe {
          std::ptr::copy_nonoverlapping(
            (&format as *const windows::Win32::Media::Audio::WAVEFORMATEX).cast::<u8>(),
            output,
            std::mem::size_of::<windows::Win32::Media::Audio::WAVEFORMATEX>(),
          );
          std::ptr::copy_nonoverlapping(
            extension.as_ptr(),
            output.add(std::mem::size_of::<
              windows::Win32::Media::Audio::WAVEFORMATEX,
            >()),
            extra,
          );
          *closest = output.cast();
        }
        HRESULT(1)
      }
      RawFormatSupport::Failure => {
        unsafe { *closest = std::ptr::null_mut() };
        HRESULT(0x80004005u32 as i32)
      }
    }
  }

  #[repr(C)]
  struct RawSyntheticDirtyCoTaskMemCall {
    vtable: *const *mut c_void,
    calls: u32,
  }

  unsafe extern "system" fn raw_synthetic_dirty_cotaskmem_then_fail(
    this: *mut c_void,
    output: *mut *mut c_void,
  ) -> HRESULT {
    let call = unsafe { &mut *this.cast::<RawSyntheticDirtyCoTaskMemCall>() };
    call.calls += 1;
    if output.is_null() {
      return HRESULT(0x80004003u32 as i32);
    }
    unsafe {
      *output = windows::Win32::System::Com::CoTaskMemAlloc(32);
    }
    HRESULT(0x80004005u32 as i32)
  }

  #[repr(C)]
  struct RawDataObjectCall {
    vtable: *const *mut c_void,
    hglobal: windows::Win32::Foundation::HGLOBAL,
    release_owner: *mut c_void,
    calls: u32,
  }

  #[repr(C)]
  struct RawMediumReleaseOwner {
    vtable: *const windows::core::IUnknown_Vtbl,
    hglobal: windows::Win32::Foundation::HGLOBAL,
    releases: AtomicU32,
    global_frees: AtomicU32,
  }

  unsafe extern "system" fn raw_medium_owner_query_interface(
    this: *mut c_void,
    iid: *const windows::core::GUID,
    result: *mut *mut c_void,
  ) -> HRESULT {
    if unsafe { *iid } == windows::core::IUnknown::IID {
      unsafe { *result = this };
      2
    } else {
      unsafe { *result = std::ptr::null_mut() };
      0
    };
    if unsafe { (*result).is_null() } {
      HRESULT(0x80004002u32 as i32)
    } else {
      HRESULT(0)
    }
  }

  unsafe extern "system" fn raw_medium_owner_add_ref(_this: *mut c_void) -> u32 {
    2
  }

  unsafe extern "system" fn raw_medium_owner_release(this: *mut c_void) -> u32 {
    let owner = unsafe { &mut *this.cast::<RawMediumReleaseOwner>() };
    if owner.releases.fetch_add(1, AtomicOrdering::Relaxed) == 0 {
      let result = unsafe { raw_global_free(owner.hglobal.0) };
      if result.is_null() {
        owner.global_frees.fetch_add(1, AtomicOrdering::Relaxed);
      }
      owner.hglobal = windows::Win32::Foundation::HGLOBAL(std::ptr::null_mut());
    }
    0
  }

  static RAW_MEDIUM_OWNER_VTABLE: windows::core::IUnknown_Vtbl = windows::core::IUnknown_Vtbl {
    QueryInterface: raw_medium_owner_query_interface,
    AddRef: raw_medium_owner_add_ref,
    Release: raw_medium_owner_release,
  };

  unsafe extern "system" fn raw_get_data_matrix(
    this: *mut c_void,
    format: *const windows::Win32::System::Com::FORMATETC,
    medium: *mut windows::Win32::System::Com::STGMEDIUM,
  ) -> HRESULT {
    let call = unsafe { &mut *this.cast::<RawDataObjectCall>() };
    call.calls += 1;
    if format.is_null() || medium.is_null() {
      return HRESULT(0x80070057u32 as i32);
    }
    let format = unsafe { &*format };
    if format.cfFormat != 13
      || !format.ptd.is_null()
      || format.dwAspect != 1
      || format.lindex != -1
      || format.tymed != windows::Win32::System::Com::TYMED_HGLOBAL.0 as u32
      || call.hglobal.0.is_null()
    {
      return HRESULT(0x80070057u32 as i32);
    }
    unsafe {
      medium.write(windows::Win32::System::Com::STGMEDIUM {
        tymed: windows::Win32::System::Com::TYMED_HGLOBAL.0 as u32,
        u: windows::Win32::System::Com::STGMEDIUM_0 {
          hGlobal: call.hglobal,
        },
        pUnkForRelease: std::mem::ManuallyDrop::new(
          (!call.release_owner.is_null()).then(|| IUnknown::from_raw(call.release_owner)),
        ),
      });
    }
    HRESULT(0)
  }

  #[repr(C)]
  struct RawDepthCall {
    vtable: *const *mut c_void,
    calls: u32,
  }

  unsafe extern "system" fn raw_mutate_depth_matrix(
    this: *mut c_void,
    scalar: *mut i32,
    pointer: *mut *mut c_void,
    triple: *mut *mut *mut usize,
  ) -> HRESULT {
    let call = unsafe { &mut *this.cast::<RawDepthCall>() };
    call.calls += 1;
    if scalar.is_null() || pointer.is_null() || triple.is_null() {
      return HRESULT(0x80004003u32 as i32);
    }
    unsafe {
      *scalar += 7;
      *pointer = std::ptr::with_exposed_provenance_mut::<c_void>(0x4321);
      ***triple = 0xcafe;
    }
    HRESULT(0)
  }

  unsafe extern "system" fn raw_nullable_pointer_matrix(
    this: *mut c_void,
    pointer: *mut c_void,
  ) -> HRESULT {
    let call = unsafe { &mut *this.cast::<RawDepthCall>() };
    call.calls += 1;
    if pointer.is_null() {
      HRESULT(0)
    } else {
      HRESULT(0x80070057u32 as i32)
    }
  }

  fn raw_descriptor(name: &str, layout: serde_json::Value) -> String {
    serde_json::json!({
      "name": name,
      "x86": layout.clone(),
      "x64": layout.clone(),
      "arm64": layout,
    })
    .to_string()
  }

  fn external_pointer(address: usize) -> DynComRawPointer {
    DynComRawPointer::from_address(Either::A(BigInt::from(address as u64))).unwrap()
  }

  fn preserved_hresult(values: &[DynWinRTValue]) -> i32 {
    match values.first().map(|value| &value.0) {
      Some(dynwinrt::WinRTValue::HResult(value)) => value.0,
      other => panic!("expected preserved HRESULT, found {other:?}"),
    }
  }

  #[test]
  fn allocation_is_aligned_zeroed_and_released_once() {
    let memory = raw_memory(64, 32);
    let address = memory.pointer(None).unwrap().address_bits().unwrap();
    assert_eq!(address % 32, 0);
    assert_eq!(memory.allocation.read_bytes(0, 64).unwrap(), vec![0; 64]);
    assert!(!memory.released().unwrap());
    memory.release().unwrap();
    memory.release().unwrap();
    assert!(memory.released().unwrap());
    assert!(memory.allocation.deallocated());
    assert!(memory.allocation.read_bytes(0, 1).is_err());
  }

  #[test]
  fn allocation_rejects_zero_invalid_alignment_and_layout_overflow() {
    assert!(RawAllocation::allocate(0, 1).is_err());
    assert!(RawAllocation::allocate(1, 0).is_err());
    assert!(RawAllocation::allocate(1, 3).is_err());
    assert!(RawAllocation::allocate(1, 1usize << (usize::BITS - 1)).is_err());
    assert!(RawAllocation::allocate(usize::MAX, 2).is_err());
  }

  #[test]
  fn exact_interface_output_slot_preflight_is_pointer_width_aware() {
    let width = std::mem::size_of::<usize>();
    let valid = raw_memory(width, width);
    DynComRaw::validate_exact_output_slot(&valid).unwrap();

    let undersized = raw_memory(width - 1, 1);
    assert!(DynComRaw::validate_exact_output_slot(&undersized)
      .unwrap_err()
      .reason
      .contains("smaller than pointer width"));

    let base = raw_memory(width * 2, width);
    let address = base.pointer(None).unwrap().address_bits().unwrap() + 1;
    let misaligned = DynComRawMemory {
      allocation: RawAllocation::external(address, width, 1, "misaligned exact output").unwrap(),
    };
    assert!(DynComRaw::validate_exact_output_slot(&misaligned)
      .unwrap_err()
      .reason
      .contains("not pointer-aligned"));

    valid.release().unwrap();
    assert!(DynComRaw::validate_exact_output_slot(&valid)
      .unwrap_err()
      .reason
      .contains("released"));
  }

  #[test]
  fn managed_borrowed_raw_pointer_retains_owner_and_is_not_adoptable() {
    let mut object = RawTrackedComObject {
      vtable: &RAW_TRACKED_VTABLE,
      addrefs: AtomicU32::new(0),
      releases: AtomicU32::new(0),
    };
    let mut managed = raw_tracked_managed(&mut object);
    let mut pointer = DynComRawPointer::from_managed_borrowed(&managed).unwrap();
    assert_eq!(object.addrefs.load(AtomicOrdering::Relaxed), 1);
    managed.release().unwrap();
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 1);
    assert_eq!(
      pointer.address().unwrap().get_u64().1,
      &mut object as *mut _ as u64
    );
    assert!(DynComRawOwnedComPointer::adopt_transferred(&mut pointer, None).is_err());

    let call_value = pointer.to_value().unwrap();
    drop(pointer);
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 1);
    let mut call_value_for_adoption = call_value;
    assert!(crate::com::take_native_output_pointer(
      &mut call_value_for_adoption,
      PointerProvenance::ComOutput,
      "COM interface"
    )
    .is_err());
    drop(call_value_for_adoption);
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 2);
  }

  #[test]
  fn owned_raw_com_pointer_addref_qi_and_atomic_managed_transitions() {
    let mut object = RawTrackedComObject {
      vtable: &RAW_TRACKED_VTABLE,
      addrefs: AtomicU32::new(0),
      releases: AtomicU32::new(0),
    };
    let mut managed = raw_tracked_managed(&mut object);
    let owner = DynComRawOwnedComPointer::add_ref(&managed).unwrap();
    assert_eq!(object.addrefs.load(AtomicOrdering::Relaxed), 1);
    assert_eq!(
      owner.address().unwrap().get_u64().1,
      &mut object as *mut _ as u64
    );
    owner.release().unwrap();
    owner.release().unwrap();
    assert!(owner.released().unwrap());
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 1);

    let queried =
      DynComRawOwnedComPointer::query_interface(&managed, &WinGUID(IID_RAW_TRACKED_ALT)).unwrap();
    assert_eq!(
      queried.address().unwrap().get_u64().1,
      &mut object as *mut _ as u64
    );
    queried.release().unwrap();

    let mismatch = DynComRawOwnedComPointer::add_ref(&managed).unwrap();
    let unsupported = WinGUID(windows::core::GUID::from_u128(
      0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff,
    ));
    assert!(mismatch.into_managed(Some(&unsupported)).is_err());
    assert!(mismatch.released().unwrap());

    let success = DynComRawOwnedComPointer::add_ref(&managed).unwrap();
    let mut exact = success
      .into_managed(Some(&WinGUID(IID_RAW_TRACKED_ALT)))
      .unwrap();
    assert!(success.released().unwrap());
    exact.release().unwrap();
    managed.release().unwrap();
    assert_eq!(
      object.releases.load(AtomicOrdering::Relaxed),
      object.addrefs.load(AtomicOrdering::Relaxed) + 1
    );
  }

  #[test]
  fn raw_com_invocation_lease_survives_reentrant_release_and_managed_consumption() {
    for action in [
      RawComReentrantAction::Release,
      RawComReentrantAction::IntoManaged,
      RawComReentrantAction::Finalize,
    ] {
      let mut object = RawTrackedComObject {
        vtable: &RAW_TRACKED_VTABLE,
        addrefs: AtomicU32::new(0),
        releases: AtomicU32::new(0),
      };
      let mut managed = raw_tracked_managed(&mut object);
      let owner = DynComRawOwnedComPointer::add_ref(&managed).unwrap();
      managed.release().unwrap();
      let value = owner.pointer().unwrap().to_value().unwrap();
      let mut call = RawComReentrantCall {
        vtable: std::ptr::null(),
        state: owner.state.clone(),
        action,
        logical_release_visible: false,
        used_pointer_after_release: false,
      };

      invoke_reentrant_raw_com(&mut call, &value).unwrap();

      assert!(call.logical_release_visible);
      assert!(call.used_pointer_after_release);
      assert!(owner.released().unwrap());
      assert!(value.1.as_ref().unwrap().validate().is_err());
      assert_eq!(
        object.releases.load(AtomicOrdering::Relaxed),
        object.addrefs.load(AtomicOrdering::Relaxed) + 1
      );
    }
  }

  #[test]
  fn raw_com_invocation_leases_drop_on_error_and_panic_paths() {
    let run = |panic_path: bool| {
      let mut object = RawTrackedComObject {
        vtable: &RAW_TRACKED_VTABLE,
        addrefs: AtomicU32::new(0),
        releases: AtomicU32::new(0),
      };
      let mut managed = raw_tracked_managed(&mut object);
      let owner = DynComRawOwnedComPointer::add_ref(&managed).unwrap();
      managed.release().unwrap();
      let value = owner.pointer().unwrap().to_value().unwrap();
      let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_com_invocation_args(&[&value], |_| {
          owner.release().unwrap();
          assert!(owner.released().unwrap());
          if panic_path {
            panic!("expected raw COM invocation panic");
          }
          Err::<(), _>(napi::Error::from_reason(
            "expected raw COM invocation error",
          ))
        })
      }));
      if panic_path {
        assert!(result.is_err());
      } else {
        assert!(result.unwrap().is_err());
      }
      assert_eq!(
        object.releases.load(AtomicOrdering::Relaxed),
        object.addrefs.load(AtomicOrdering::Relaxed) + 1
      );
    };
    run(false);
    run(true);
  }

  #[test]
  fn owned_raw_com_pointer_detaches_and_reconciles_exactly_once() {
    let mut object = RawTrackedComObject {
      vtable: &RAW_TRACKED_VTABLE,
      addrefs: AtomicU32::new(0),
      releases: AtomicU32::new(0),
    };
    let mut managed = raw_tracked_managed(&mut object);
    let owner = DynComRawOwnedComPointer::add_ref(&managed).unwrap();
    let mut detached = owner.detach().unwrap();
    assert!(owner.released().unwrap());
    assert!(owner.detach().is_err());
    assert!(detached.to_value().is_ok());

    let adopted = DynComRawOwnedComPointer::adopt_transferred(&mut detached, None).unwrap();
    assert!(detached.address().is_err());
    adopted.release().unwrap();
    assert!(adopted.release().is_ok());
    managed.release().unwrap();
    assert_eq!(object.addrefs.load(AtomicOrdering::Relaxed), 1);
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 2);
  }

  #[test]
  fn detached_pointer_drop_and_iid_mismatch_release_exactly_once() {
    let mut object = RawTrackedComObject {
      vtable: &RAW_TRACKED_VTABLE,
      addrefs: AtomicU32::new(0),
      releases: AtomicU32::new(0),
    };
    let mut managed = raw_tracked_managed(&mut object);
    {
      let owner = DynComRawOwnedComPointer::add_ref(&managed).unwrap();
      let unpublished = owner.detach().unwrap();
      drop(unpublished);
    }
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 1);

    let owner = DynComRawOwnedComPointer::add_ref(&managed).unwrap();
    let mut detached = owner.detach().unwrap();
    let unsupported = WinGUID(windows::core::GUID::from_u128(
      0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff,
    ));
    assert!(
      DynComRawOwnedComPointer::adopt_transferred(&mut detached, Some(&unsupported)).is_err()
    );
    assert!(detached.address().is_err());
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 2);
    managed.release().unwrap();
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 3);
  }

  #[test]
  fn transfer_to_memory_is_atomic_on_failure_and_one_shot_on_success() {
    let mut object = RawTrackedComObject {
      vtable: &RAW_TRACKED_VTABLE,
      addrefs: AtomicU32::new(0),
      releases: AtomicU32::new(0),
    };
    let mut managed = raw_tracked_managed(&mut object);
    let owner = DynComRawOwnedComPointer::add_ref(&managed).unwrap();
    let too_small = raw_memory(1, 1);
    assert!(owner.transfer_to(&too_small, None).is_err());
    assert!(!owner.released().unwrap());
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 0);

    let slot = raw_memory(std::mem::size_of::<usize>(), std::mem::align_of::<usize>());
    let address = owner.address().unwrap().get_u64().1 as usize;
    owner.transfer_to(&slot, None).unwrap();
    assert!(owner.released().unwrap());
    assert!(owner.transfer_to(&slot, None).is_err());
    assert_eq!(slot.allocation.read_usize(0).unwrap(), address);
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 0);

    let mut returned = slot.read_pointer(number(0)).unwrap();
    assert!(DynComRawOwnedComPointer::adopt_transferred(&mut returned, None).is_err());
    let reconciled = DynComRawOwnedComPointer::assume_transferred(&mut returned, None).unwrap();
    reconciled.release().unwrap();
    managed.release().unwrap();
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 2);
  }

  #[test]
  fn raw_interface_inout_consume_old_then_replace_never_reconstructs_old() {
    let mut old_object = RawTrackedComObject {
      vtable: &RAW_TRACKED_VTABLE,
      addrefs: AtomicU32::new(0),
      releases: AtomicU32::new(0),
    };
    let mut new_object = RawTrackedComObject {
      vtable: &RAW_TRACKED_VTABLE,
      addrefs: AtomicU32::new(0),
      releases: AtomicU32::new(0),
    };
    let mut old_managed = raw_tracked_managed(&mut old_object);
    let mut new_managed = raw_tracked_managed(&mut new_object);

    let old_owner = DynComRawOwnedComPointer::add_ref(&old_managed).unwrap();
    let old_address = old_owner.address().unwrap().get_u64().1 as usize;
    let slot = raw_memory(std::mem::size_of::<usize>(), std::mem::align_of::<usize>());
    old_owner.transfer_to(&slot, None).unwrap();
    assert!(old_owner.released().unwrap());
    let slot_value = slot.pointer(None).unwrap().to_value().unwrap();
    let mut in_out_call = RawPointerReplacementCall {
      vtable: std::ptr::null(),
      replacement: (&mut new_object as *mut RawTrackedComObject).cast(),
      saw_initial: false,
    };
    invoke_raw_pointer_slot(
      raw_open_namespace_consume_old_replace as *mut c_void,
      &mut in_out_call,
      &slot_value,
    );
    assert!(in_out_call.saw_initial);
    let mut replacement = slot.read_pointer(number(0)).unwrap();
    assert_ne!(replacement.address_bits().unwrap(), old_address);
    let replacement_owner =
      DynComRawOwnedComPointer::assume_transferred(&mut replacement, None).unwrap();
    replacement_owner.release().unwrap();

    old_managed.release().unwrap();
    new_managed.release().unwrap();
    assert_eq!(old_object.addrefs.load(AtomicOrdering::Relaxed), 1);
    assert_eq!(old_object.releases.load(AtomicOrdering::Relaxed), 2);
    assert_eq!(new_object.addrefs.load(AtomicOrdering::Relaxed), 1);
    assert_eq!(new_object.releases.load(AtomicOrdering::Relaxed), 2);
  }

  #[test]
  fn raw_interface_inout_preserve_old_then_replace_reconciles_both() {
    let mut old_object = RawTrackedComObject {
      vtable: &RAW_TRACKED_VTABLE,
      addrefs: AtomicU32::new(0),
      releases: AtomicU32::new(0),
    };
    let mut new_object = RawTrackedComObject {
      vtable: &RAW_TRACKED_VTABLE,
      addrefs: AtomicU32::new(0),
      releases: AtomicU32::new(0),
    };
    let mut old_managed = raw_tracked_managed(&mut old_object);
    let mut new_managed = raw_tracked_managed(&mut new_object);
    let old_owner = DynComRawOwnedComPointer::add_ref(&old_managed).unwrap();
    let old_address = old_owner.address().unwrap().get_u64().1 as usize;
    let slot = raw_memory(std::mem::size_of::<usize>(), std::mem::align_of::<usize>());
    old_owner.transfer_to(&slot, None).unwrap();
    let slot_value = slot.pointer(None).unwrap().to_value().unwrap();
    let mut call = RawPointerReplacementCall {
      vtable: std::ptr::null(),
      replacement: (&mut new_object as *mut RawTrackedComObject).cast(),
      saw_initial: false,
    };
    invoke_raw_pointer_slot(
      raw_open_namespace_preserve_old_replace as *mut c_void,
      &mut call,
      &slot_value,
    );
    assert!(call.saw_initial);
    let mut replacement = slot.read_pointer(number(0)).unwrap();
    let replacement_owner =
      DynComRawOwnedComPointer::assume_transferred(&mut replacement, None).unwrap();
    let mut old_transferred = external_pointer(old_address);
    let old_reconciled =
      DynComRawOwnedComPointer::assume_transferred(&mut old_transferred, None).unwrap();
    replacement_owner.release().unwrap();
    old_reconciled.release().unwrap();

    old_managed.release().unwrap();
    new_managed.release().unwrap();
    assert_eq!(old_object.addrefs.load(AtomicOrdering::Relaxed), 1);
    assert_eq!(old_object.releases.load(AtomicOrdering::Relaxed), 2);
    assert_eq!(new_object.addrefs.load(AtomicOrdering::Relaxed), 1);
    assert_eq!(new_object.releases.load(AtomicOrdering::Relaxed), 2);
  }

  #[test]
  fn raw_interface_inout_unchanged_adopts_slot_exactly_once() {
    let mut object = RawTrackedComObject {
      vtable: &RAW_TRACKED_VTABLE,
      addrefs: AtomicU32::new(0),
      releases: AtomicU32::new(0),
    };
    let mut managed = raw_tracked_managed(&mut object);
    let owner = DynComRawOwnedComPointer::add_ref(&managed).unwrap();
    let address = owner.address().unwrap().get_u64().1 as usize;
    let slot = raw_memory(std::mem::size_of::<usize>(), std::mem::align_of::<usize>());
    owner.transfer_to(&slot, None).unwrap();
    let slot_value = slot.pointer(None).unwrap().to_value().unwrap();
    let mut call = RawPointerReplacementCall {
      vtable: std::ptr::null(),
      replacement: std::ptr::null_mut(),
      saw_initial: false,
    };
    invoke_raw_pointer_slot(
      raw_open_namespace_unchanged as *mut c_void,
      &mut call,
      &slot_value,
    );
    assert!(call.saw_initial);
    let mut unchanged = slot.read_pointer(number(0)).unwrap();
    assert_eq!(unchanged.address_bits().unwrap(), address);
    let unchanged_owner =
      DynComRawOwnedComPointer::assume_transferred(&mut unchanged, None).unwrap();
    assert!(DynComRawOwnedComPointer::assume_transferred(&mut unchanged, None).is_err());
    unchanged_owner.release().unwrap();
    managed.release().unwrap();
    assert_eq!(object.addrefs.load(AtomicOrdering::Relaxed), 1);
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 2);
  }

  #[test]
  fn owned_raw_com_pointer_enforces_thread_and_finalizer_policy() {
    let mut object = RawTrackedComObject {
      vtable: &RAW_TRACKED_VTABLE,
      addrefs: AtomicU32::new(0),
      releases: AtomicU32::new(0),
    };
    let mut managed = raw_tracked_managed(&mut object);
    let owner =
      SendRawOwnerForWrongThreadTest(DynComRawOwnedComPointer::add_ref(&managed).unwrap());
    std::thread::spawn(move || {
      assert!(owner.0.address().is_err());
      drop(owner);
    })
    .join()
    .unwrap();
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 0);
    managed.release().unwrap();
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 1);

    let mut object = RawTrackedComObject {
      vtable: &RAW_TRACKED_VTABLE,
      addrefs: AtomicU32::new(0),
      releases: AtomicU32::new(0),
    };
    let mut managed = raw_tracked_managed(&mut object);
    {
      let owner = DynComRawOwnedComPointer::add_ref(&managed).unwrap();
      let retained = owner.retain().unwrap();
      owner.release().unwrap();
      assert!(retained.address().is_ok());
      retained.release().unwrap();
    }
    assert_eq!(object.addrefs.load(AtomicOrdering::Relaxed), 2);
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 2);
    managed.release().unwrap();

    let mut object = RawTrackedComObject {
      vtable: &RAW_TRACKED_VTABLE,
      addrefs: AtomicU32::new(0),
      releases: AtomicU32::new(0),
    };
    let mut managed = raw_tracked_managed(&mut object);
    let owner = DynComRawOwnedComPointer::add_ref(&managed).unwrap();
    let child = owner.pointer().unwrap().to_value().unwrap();
    drop(owner);
    assert!(child.1.as_ref().unwrap().validate().is_err());
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 1);
    managed.release().unwrap();
  }

  #[test]
  fn raw_get_private_data_matrix_keeps_bytes_and_interface_ownership_explicit() {
    let key = windows::core::GUID::from_u128(0x00112233_4455_6677_8899_aabbccddeeff);
    let guid = raw_guid_memory(key);
    let size = raw_memory(std::mem::size_of::<u32>(), std::mem::align_of::<u32>());
    let table = dynwinrt::MetadataTable::new();
    let signature = || {
      dynwinrt::com::MethodSignature::new(&table)
        .add_in(dynwinrt::com::Type::pointer())
        .add_in(dynwinrt::com::Type::pointer())
        .add_nullable_in(dynwinrt::com::Type::pointer())
        .preserve_hresult()
    };
    let guid_value = raw_value(&guid.pointer(None).unwrap());
    let size_value = raw_value(&size.pointer(None).unwrap());
    let null_value = raw_value(&DynComRawPointer::null());

    let mut interface = RawTrackedComObject {
      vtable: &RAW_TRACKED_VTABLE,
      addrefs: AtomicU32::new(0),
      releases: AtomicU32::new(0),
    };
    let mut managed_interface = raw_tracked_managed(&mut interface);
    let mut call = RawPrivateDataCall {
      vtable: std::ptr::null(),
      expected_guid: key,
      bytes: [10, 20, 30, 40, 50, 60],
      interface: (&mut interface as *mut RawTrackedComObject).cast(),
      payload: RawPrivateDataPayload::Bytes,
      calls: 0,
    };

    let query = invoke_raw_contract_method(
      raw_get_private_data_matrix as *mut c_void,
      (&mut call as *mut RawPrivateDataCall).cast(),
      signature(),
      &[&guid_value, &size_value, &null_value],
    )
    .unwrap();
    assert_eq!(preserved_hresult(&query), 0);
    assert_eq!(size.read_u32(number(0)).unwrap(), 6);

    let owned = raw_memory(6, 1);
    let owned_value = raw_value(&owned.pointer(None).unwrap());
    let bytes = invoke_raw_contract_method(
      raw_get_private_data_matrix as *mut c_void,
      (&mut call as *mut RawPrivateDataCall).cast(),
      signature(),
      &[&guid_value, &size_value, &owned_value],
    )
    .unwrap();
    assert_eq!(preserved_hresult(&bytes), 0);
    assert_eq!(
      owned.read_bytes(number(0), number(6)).unwrap().as_ref(),
      call.bytes
    );

    let mut external_bytes = [0u8; 6];
    let external = DynComRawMemory::from_unsafe_address(
      Either::A(BigInt::from(
        external_bytes.as_mut_ptr().expose_provenance() as u64,
      )),
      number(external_bytes.len()),
      number(1),
    )
    .unwrap();
    let external_value = raw_value(&external.pointer(None).unwrap());
    let external_result = invoke_raw_contract_method(
      raw_get_private_data_matrix as *mut c_void,
      (&mut call as *mut RawPrivateDataCall).cast(),
      signature(),
      &[&guid_value, &size_value, &external_value],
    )
    .unwrap();
    assert_eq!(preserved_hresult(&external_result), 0);
    assert_eq!(external_bytes, call.bytes);

    call.payload = RawPrivateDataPayload::Interface { fail: false };
    size
      .write_u32(number(0), std::mem::size_of::<usize>() as f64)
      .unwrap();
    let interface_slot = raw_memory(std::mem::size_of::<usize>(), std::mem::align_of::<usize>());
    let interface_slot_value = raw_value(&interface_slot.pointer(None).unwrap());
    let interface_result = invoke_raw_contract_method(
      raw_get_private_data_matrix as *mut c_void,
      (&mut call as *mut RawPrivateDataCall).cast(),
      signature(),
      &[&guid_value, &size_value, &interface_slot_value],
    )
    .unwrap();
    assert_eq!(preserved_hresult(&interface_result), 0);
    let mut interface_pointer = interface_slot.read_pointer(number(0)).unwrap();
    let interface_owner =
      DynComRawOwnedComPointer::assume_transferred(&mut interface_pointer, None).unwrap();
    interface_owner.release().unwrap();

    call.payload = RawPrivateDataPayload::Interface { fail: true };
    interface_slot
      .write_pointer(number(0), &DynComRawPointer::null())
      .unwrap();
    let failure = invoke_raw_contract_method(
      raw_get_private_data_matrix as *mut c_void,
      (&mut call as *mut RawPrivateDataCall).cast(),
      signature(),
      &[&guid_value, &size_value, &interface_slot_value],
    )
    .err()
    .unwrap();
    assert!(failure.reason.contains("80004005"));
    let mut failed_pointer = interface_slot.read_pointer(number(0)).unwrap();
    let failed_owner =
      DynComRawOwnedComPointer::assume_transferred(&mut failed_pointer, None).unwrap();
    failed_owner.release().unwrap();

    managed_interface.release().unwrap();
    assert_eq!(interface.addrefs.load(AtomicOrdering::Relaxed), 2);
    assert_eq!(interface.releases.load(AtomicOrdering::Relaxed), 3);
    assert_eq!(call.calls, 5);
  }

  #[test]
  fn raw_audio_format_support_matrix_handles_variable_cotaskmem_output() {
    let format = windows::Win32::Media::Audio::WAVEFORMATEX {
      wFormatTag: 1,
      nChannels: 2,
      nSamplesPerSec: 48_000,
      nAvgBytesPerSec: 192_000,
      nBlockAlign: 4,
      wBitsPerSample: 16,
      cbSize: 4,
    };
    let base_size = std::mem::size_of::<windows::Win32::Media::Audio::WAVEFORMATEX>();
    let input = raw_memory(base_size + 4, 1);
    let format_bytes = unsafe {
      std::slice::from_raw_parts(
        (&format as *const windows::Win32::Media::Audio::WAVEFORMATEX).cast::<u8>(),
        base_size,
      )
    };
    input
      .write_bytes(number(0), Buffer::from(format_bytes.to_vec()))
      .unwrap();
    input
      .write_bytes(number(base_size), Buffer::from(vec![9, 8, 7, 6]))
      .unwrap();
    let input_value = raw_value(&input.pointer(None).unwrap());
    let output = raw_memory(std::mem::size_of::<usize>(), std::mem::align_of::<usize>());
    let output_value = raw_value(&output.pointer(None).unwrap());
    let null_value = raw_value(&DynComRawPointer::null());
    let table = dynwinrt::MetadataTable::new();
    let signature = || {
      dynwinrt::com::MethodSignature::new(&table)
        .add_in(dynwinrt::com::Type::winrt(table.i32_type()))
        .add_in(dynwinrt::com::Type::pointer())
        .add_nullable_in(dynwinrt::com::Type::pointer())
        .preserve_hresult()
    };
    let shared = DynWinRTValue::new(dynwinrt::WinRTValue::I32(
      windows::Win32::Media::Audio::AUDCLNT_SHAREMODE_SHARED.0,
    ));
    let exclusive = DynWinRTValue::new(dynwinrt::WinRTValue::I32(
      windows::Win32::Media::Audio::AUDCLNT_SHAREMODE_EXCLUSIVE.0,
    ));
    let mut call = RawFormatSupportCall {
      vtable: std::ptr::null(),
      mode: RawFormatSupport::Supported,
      calls: 0,
    };

    let supported = invoke_raw_contract_method(
      raw_is_format_supported_matrix as *mut c_void,
      (&mut call as *mut RawFormatSupportCall).cast(),
      signature(),
      &[&shared, &input_value, &output_value],
    )
    .unwrap();
    assert_eq!(preserved_hresult(&supported), 0);
    assert!(output.read_pointer(number(0)).unwrap().is_null().unwrap());

    call.mode = RawFormatSupport::Closest;
    let closest = invoke_raw_contract_method(
      raw_is_format_supported_matrix as *mut c_void,
      (&mut call as *mut RawFormatSupportCall).cast(),
      signature(),
      &[&shared, &input_value, &output_value],
    )
    .unwrap();
    assert_eq!(preserved_hresult(&closest), 1);
    let mut closest_pointer = output.read_pointer(number(0)).unwrap();
    let header =
      DynComRawMemory::from_unsafe_pointer(&closest_pointer, number(base_size), number(1)).unwrap();
    let extra = header.read_u16(number(16)).unwrap() as usize;
    assert_eq!(extra, 4);
    let full =
      DynComRawMemory::from_unsafe_pointer(&closest_pointer, number(base_size + extra), number(1))
        .unwrap();
    assert_eq!(
      full
        .read_bytes(number(base_size), number(extra))
        .unwrap()
        .as_ref(),
      &[9, 8, 7, 6]
    );
    header.release().unwrap();
    full.release().unwrap();
    DynComRawCleanup::co_task_mem_free(&mut closest_pointer).unwrap();

    call.mode = RawFormatSupport::Supported;
    let exclusive_supported = invoke_raw_contract_method(
      raw_is_format_supported_matrix as *mut c_void,
      (&mut call as *mut RawFormatSupportCall).cast(),
      signature(),
      &[&exclusive, &input_value, &null_value],
    )
    .unwrap();
    assert_eq!(preserved_hresult(&exclusive_supported), 0);

    call.mode = RawFormatSupport::Failure;
    output
      .write_pointer(number(0), &DynComRawPointer::null())
      .unwrap();
    let failure = invoke_raw_contract_method(
      raw_is_format_supported_matrix as *mut c_void,
      (&mut call as *mut RawFormatSupportCall).cast(),
      signature(),
      &[&shared, &input_value, &output_value],
    )
    .err()
    .unwrap();
    assert!(failure.reason.contains("80004005"));
    assert!(output.read_pointer(number(0)).unwrap().is_null().unwrap());

    let missing_shared_output = invoke_raw_contract_method(
      raw_is_format_supported_matrix as *mut c_void,
      (&mut call as *mut RawFormatSupportCall).cast(),
      signature(),
      &[&shared, &input_value, &null_value],
    )
    .err()
    .unwrap();
    assert!(missing_shared_output.reason.contains("80004003"));
    assert_eq!(call.calls, 5);
  }

  #[test]
  fn synthetic_dirty_cotaskmem_output_on_failure_is_explicitly_cleaned() {
    let output = raw_memory(std::mem::size_of::<usize>(), std::mem::align_of::<usize>());
    let output_value = raw_value(&output.pointer(None).unwrap());
    let table = dynwinrt::MetadataTable::new();
    let signature =
      dynwinrt::com::MethodSignature::new(&table).add_in(dynwinrt::com::Type::pointer());
    let mut call = RawSyntheticDirtyCoTaskMemCall {
      vtable: std::ptr::null(),
      calls: 0,
    };
    let failure = invoke_raw_contract_method(
      raw_synthetic_dirty_cotaskmem_then_fail as *mut c_void,
      (&mut call as *mut RawSyntheticDirtyCoTaskMemCall).cast(),
      signature,
      &[&output_value],
    )
    .err()
    .unwrap();
    assert!(failure.reason.contains("80004005"));
    let mut dirty = output.read_pointer(number(0)).unwrap();
    assert!(!dirty.is_null().unwrap());
    DynComRawCleanup::co_task_mem_free(&mut dirty).unwrap();
    assert_eq!(call.calls, 1);
  }

  #[test]
  fn raw_data_object_matrix_uses_actual_formatetc_and_stgmedium_storage() {
    let format = windows::Win32::System::Com::FORMATETC {
      cfFormat: 13,
      ptd: std::ptr::null_mut(),
      dwAspect: 1,
      lindex: -1,
      tymed: windows::Win32::System::Com::TYMED_HGLOBAL.0 as u32,
    };
    let format_memory = raw_memory(
      std::mem::size_of::<windows::Win32::System::Com::FORMATETC>(),
      std::mem::align_of::<windows::Win32::System::Com::FORMATETC>(),
    );
    let format_bytes = unsafe {
      std::slice::from_raw_parts(
        (&format as *const windows::Win32::System::Com::FORMATETC).cast::<u8>(),
        std::mem::size_of::<windows::Win32::System::Com::FORMATETC>(),
      )
    };
    format_memory
      .write_bytes(number(0), Buffer::from(format_bytes.to_vec()))
      .unwrap();
    let medium = raw_memory(
      std::mem::size_of::<windows::Win32::System::Com::STGMEDIUM>(),
      std::mem::align_of::<windows::Win32::System::Com::STGMEDIUM>(),
    );
    let format_value = raw_value(&format_memory.pointer(None).unwrap());
    let medium_value = raw_value(&medium.pointer(None).unwrap());
    let table = dynwinrt::MetadataTable::new();
    let signature = dynwinrt::com::MethodSignature::new(&table)
      .add_in(dynwinrt::com::Type::pointer())
      .add_in(dynwinrt::com::Type::pointer());
    let delegated_hglobal = unsafe {
      windows::Win32::System::Memory::GlobalAlloc(windows::Win32::System::Memory::GMEM_MOVEABLE, 32)
    }
    .unwrap();
    let delegated_hglobal_address = delegated_hglobal.0 as usize;
    let mut owner = RawMediumReleaseOwner {
      vtable: &RAW_MEDIUM_OWNER_VTABLE,
      hglobal: delegated_hglobal,
      releases: AtomicU32::new(0),
      global_frees: AtomicU32::new(0),
    };
    let mut call = RawDataObjectCall {
      vtable: std::ptr::null(),
      hglobal: delegated_hglobal,
      release_owner: (&mut owner as *mut RawMediumReleaseOwner).cast(),
      calls: 0,
    };

    invoke_raw_contract_method(
      raw_get_data_matrix as *mut c_void,
      (&mut call as *mut RawDataObjectCall).cast(),
      signature,
      &[&format_value, &medium_value],
    )
    .unwrap();
    let returned = medium
      .read_bytes(number(0), number(medium.allocation.size))
      .unwrap();
    let returned = unsafe {
      std::ptr::read_unaligned(
        returned
          .as_ref()
          .as_ptr()
          .cast::<windows::Win32::System::Com::STGMEDIUM>(),
      )
    };
    assert_eq!(
      returned.tymed,
      windows::Win32::System::Com::TYMED_HGLOBAL.0 as u32
    );
    assert_eq!(
      unsafe { returned.u.hGlobal }.0 as usize,
      delegated_hglobal_address
    );
    assert_eq!(
      returned
        .pUnkForRelease
        .as_ref()
        .expect("delegated medium has release owner")
        .as_raw(),
      (&mut owner as *mut RawMediumReleaseOwner).cast()
    );
    DynComRawCleanup::release_stg_medium(&medium, None).unwrap();
    assert_eq!(owner.releases.load(AtomicOrdering::Relaxed), 1);
    assert_eq!(owner.global_frees.load(AtomicOrdering::Relaxed), 1);
    DynComRawCleanup::release_stg_medium(&medium, None).unwrap();
    assert_eq!(owner.releases.load(AtomicOrdering::Relaxed), 1);
    assert_eq!(owner.global_frees.load(AtomicOrdering::Relaxed), 1);
    assert_eq!(
      medium
        .read_bytes(number(0), number(medium.allocation.size))
        .unwrap()
        .as_ref(),
      vec![0; medium.allocation.size]
    );

    let direct_hglobal = unsafe {
      windows::Win32::System::Memory::GlobalAlloc(windows::Win32::System::Memory::GMEM_MOVEABLE, 48)
    }
    .unwrap();
    assert_eq!(
      unsafe { windows::Win32::System::Memory::GlobalSize(direct_hglobal) },
      48
    );
    call.hglobal = direct_hglobal;
    call.release_owner = std::ptr::null_mut();
    invoke_raw_contract_method(
      raw_get_data_matrix as *mut c_void,
      (&mut call as *mut RawDataObjectCall).cast(),
      dynwinrt::com::MethodSignature::new(&table)
        .add_in(dynwinrt::com::Type::pointer())
        .add_in(dynwinrt::com::Type::pointer()),
      &[&format_value, &medium_value],
    )
    .unwrap();
    let returned = medium
      .read_bytes(number(0), number(medium.allocation.size))
      .unwrap();
    let returned = unsafe {
      std::ptr::read_unaligned(
        returned
          .as_ref()
          .as_ptr()
          .cast::<windows::Win32::System::Com::STGMEDIUM>(),
      )
    };
    assert_eq!(unsafe { returned.u.hGlobal }, direct_hglobal);
    assert!(returned.pUnkForRelease.is_none());
    DynComRawCleanup::release_stg_medium(&medium, None).unwrap();
    assert_eq!(
      unsafe { windows::Win32::System::Memory::GlobalSize(direct_hglobal) },
      0
    );
    assert_eq!(call.calls, 2);
  }

  #[test]
  fn raw_pointer_depth_scalar_inout_and_nullability_matrix_executes_native_code() {
    let scalar = raw_memory(std::mem::size_of::<i32>(), std::mem::align_of::<i32>());
    scalar.write_i32(number(0), 5.0).unwrap();
    let pointer_slot = raw_memory(std::mem::size_of::<usize>(), std::mem::align_of::<usize>());
    pointer_slot
      .write_pointer(number(0), &DynComRawPointer::null())
      .unwrap();
    let target = raw_memory(std::mem::size_of::<usize>(), std::mem::align_of::<usize>());
    target.write_usize(number(0), BigInt::from(1u64)).unwrap();
    let inner = raw_memory(std::mem::size_of::<usize>(), std::mem::align_of::<usize>());
    inner
      .write_pointer(number(0), &target.pointer(None).unwrap())
      .unwrap();
    let middle = raw_memory(std::mem::size_of::<usize>(), std::mem::align_of::<usize>());
    middle
      .write_pointer(number(0), &inner.pointer(None).unwrap())
      .unwrap();
    let scalar_value = raw_value(&scalar.pointer(None).unwrap());
    let pointer_value = raw_value(&pointer_slot.pointer(None).unwrap());
    let triple_value = raw_value(&middle.pointer(None).unwrap());
    let table = dynwinrt::MetadataTable::new();
    let signature = dynwinrt::com::MethodSignature::new(&table)
      .add_in(dynwinrt::com::Type::pointer())
      .add_in(dynwinrt::com::Type::pointer())
      .add_in(dynwinrt::com::Type::pointer());
    let mut call = RawDepthCall {
      vtable: std::ptr::null(),
      calls: 0,
    };
    invoke_raw_contract_method(
      raw_mutate_depth_matrix as *mut c_void,
      (&mut call as *mut RawDepthCall).cast(),
      signature,
      &[&scalar_value, &pointer_value, &triple_value],
    )
    .unwrap();
    assert_eq!(scalar.read_i32(number(0)).unwrap(), 12);
    assert_eq!(
      pointer_slot
        .read_pointer(number(0))
        .unwrap()
        .address_bits()
        .unwrap(),
      0x4321
    );
    assert_eq!(target.read_usize(number(0)).unwrap().get_u64().1, 0xcafe);

    let null = raw_value(&DynComRawPointer::null());
    let nullable_signature =
      dynwinrt::com::MethodSignature::new(&table).add_nullable_in(dynwinrt::com::Type::pointer());
    invoke_raw_contract_method(
      raw_nullable_pointer_matrix as *mut c_void,
      (&mut call as *mut RawDepthCall).cast(),
      nullable_signature,
      &[&null],
    )
    .unwrap();
    let required_signature =
      dynwinrt::com::MethodSignature::new(&table).add_in(dynwinrt::com::Type::pointer());
    assert!(invoke_raw_contract_method(
      raw_nullable_pointer_matrix as *mut c_void,
      (&mut call as *mut RawDepthCall).cast(),
      required_signature,
      &[&null],
    )
    .is_err());
    assert_eq!(call.calls, 2);
  }

  #[test]
  fn raw_phase1_windows_layout_oracle_matches_host_abi() {
    use std::mem::{align_of, offset_of, size_of};
    use windows::Win32::Media::Audio::WAVEFORMATEX;
    use windows::Win32::Networking::BackgroundIntelligentTransferService::{
      BITS_FILE_PROPERTY_VALUE, BITS_JOB_PROPERTY_VALUE,
    };
    use windows::Win32::System::Com::{FORMATETC, STGMEDIUM};
    use windows::Win32::System::Hypervisor::WHV_ACCESS_GPA_CONTROLS;

    assert_eq!(size_of::<WAVEFORMATEX>(), 18);
    assert_eq!(align_of::<WAVEFORMATEX>(), 1);
    assert_eq!(offset_of!(WAVEFORMATEX, wFormatTag), 0);
    assert_eq!(offset_of!(WAVEFORMATEX, nSamplesPerSec), 4);
    assert_eq!(offset_of!(WAVEFORMATEX, cbSize), 16);
    assert_eq!(
      (
        size_of::<BITS_FILE_PROPERTY_VALUE>(),
        align_of::<BITS_FILE_PROPERTY_VALUE>()
      ),
      (size_of::<usize>(), align_of::<usize>())
    );
    assert_eq!(
      (
        size_of::<BITS_JOB_PROPERTY_VALUE>(),
        align_of::<BITS_JOB_PROPERTY_VALUE>()
      ),
      (16, 8)
    );
    assert_eq!(
      (
        size_of::<WHV_ACCESS_GPA_CONTROLS>(),
        align_of::<WHV_ACCESS_GPA_CONTROLS>()
      ),
      (8, 8)
    );

    if size_of::<usize>() == 8 {
      assert_eq!((size_of::<FORMATETC>(), align_of::<FORMATETC>()), (32, 8));
      assert_eq!(offset_of!(FORMATETC, ptd), 8);
      assert_eq!(offset_of!(FORMATETC, dwAspect), 16);
      assert_eq!((size_of::<STGMEDIUM>(), align_of::<STGMEDIUM>()), (24, 8));
      assert_eq!(offset_of!(STGMEDIUM, u), 8);
      assert_eq!(offset_of!(STGMEDIUM, pUnkForRelease), 16);
      assert_eq!(
        (size_of::<RawAggregate>(), align_of::<RawAggregate>()),
        (32, 8)
      );
    } else {
      assert_eq!((size_of::<FORMATETC>(), align_of::<FORMATETC>()), (20, 4));
      assert_eq!(offset_of!(FORMATETC, ptd), 4);
      assert_eq!(offset_of!(FORMATETC, dwAspect), 8);
      assert_eq!((size_of::<STGMEDIUM>(), align_of::<STGMEDIUM>()), (12, 4));
      assert_eq!(offset_of!(STGMEDIUM, u), 4);
      assert_eq!(offset_of!(STGMEDIUM, pUnkForRelease), 8);
      assert_eq!(
        (size_of::<RawAggregate>(), align_of::<RawAggregate>()),
        (28, 4)
      );
    }
    assert_eq!(size_of::<*mut *mut *mut c_void>(), size_of::<usize>());
  }

  #[repr(align(16))]
  struct ExternalBlock([u8; 32]);

  const RAW_AGGREGATE_DESCRIPTOR: &str = r#"{
    "name": "Tests.RawAggregate",
    "x86": {
      "size": 28,
      "alignment": 4,
      "fields": [
        { "name": "tag", "offset": 0, "count": 1, "type": { "kind": "u32" } },
        {
          "name": "inner",
          "offset": 4,
          "count": 1,
          "type": {
            "kind": "struct",
            "name": "Tests.RawInner",
            "layout": {
              "size": 4,
              "alignment": 2,
              "fields": [
                { "name": "values", "offset": 0, "count": 2, "type": { "kind": "u16" } }
              ]
            }
          }
        },
        { "name": "pointer", "offset": 8, "count": 1, "type": { "kind": "pointer" } },
        { "name": "guid", "offset": 12, "count": 1, "type": { "kind": "guid" } }
      ]
    },
    "x64": {
      "size": 32,
      "alignment": 8,
      "fields": [
        { "name": "tag", "offset": 0, "count": 1, "type": { "kind": "u32" } },
        {
          "name": "inner",
          "offset": 4,
          "count": 1,
          "type": {
            "kind": "struct",
            "name": "Tests.RawInner",
            "layout": {
              "size": 4,
              "alignment": 2,
              "fields": [
                { "name": "values", "offset": 0, "count": 2, "type": { "kind": "u16" } }
              ]
            }
          }
        },
        { "name": "pointer", "offset": 8, "count": 1, "type": { "kind": "pointer" } },
        { "name": "guid", "offset": 16, "count": 1, "type": { "kind": "guid" } }
      ]
    },
    "arm64": {
      "size": 32,
      "alignment": 8,
      "fields": [
        { "name": "tag", "offset": 0, "count": 1, "type": { "kind": "u32" } },
        {
          "name": "inner",
          "offset": 4,
          "count": 1,
          "type": {
            "kind": "struct",
            "name": "Tests.RawInner",
            "layout": {
              "size": 4,
              "alignment": 2,
              "fields": [
                { "name": "values", "offset": 0, "count": 2, "type": { "kind": "u16" } }
              ]
            }
          }
        },
        { "name": "pointer", "offset": 8, "count": 1, "type": { "kind": "pointer" } },
        { "name": "guid", "offset": 16, "count": 1, "type": { "kind": "guid" } }
      ]
    }
  }"#;

  const RAW_UNION_DESCRIPTOR: &str = r#"{
    "name": "Tests.RawUnion",
    "x86": {
      "size": 8,
      "alignment": 8,
      "complete": true,
      "fields": [
        { "name": "integer", "count": 1, "type": { "kind": "u64" } },
        {
          "name": "inner",
          "count": 1,
          "type": {
            "kind": "struct",
            "name": "Tests.RawUnionInner",
            "layout": {
              "size": 4,
              "alignment": 2,
              "fields": [
                { "name": "values", "offset": 0, "count": 2, "type": { "kind": "u16" } }
              ]
            }
          }
        }
      ]
    },
    "x64": {
      "size": 8,
      "alignment": 8,
      "complete": true,
      "fields": [
        { "name": "integer", "count": 1, "type": { "kind": "u64" } },
        {
          "name": "inner",
          "count": 1,
          "type": {
            "kind": "struct",
            "name": "Tests.RawUnionInner",
            "layout": {
              "size": 4,
              "alignment": 2,
              "fields": [
                { "name": "values", "offset": 0, "count": 2, "type": { "kind": "u16" } }
              ]
            }
          }
        }
      ]
    },
    "arm64": {
      "size": 8,
      "alignment": 8,
      "complete": true,
      "fields": [
        { "name": "integer", "count": 1, "type": { "kind": "u64" } },
        {
          "name": "inner",
          "count": 1,
          "type": {
            "kind": "struct",
            "name": "Tests.RawUnionInner",
            "layout": {
              "size": 4,
              "alignment": 2,
              "fields": [
                { "name": "values", "offset": 0, "count": 2, "type": { "kind": "u16" } }
              ]
            }
          }
        }
      ]
    }
  }"#;

  const CYCLIC_AGGREGATE_DESCRIPTOR: &str = r#"{
    "name": "Tests.Cycle",
    "x86": {
      "size": 4,
      "alignment": 4,
      "fields": [
        {
          "name": "self",
          "offset": 0,
          "count": 1,
          "type": {
            "kind": "struct",
            "name": "Tests.Cycle",
            "layout": {
              "size": 4,
              "alignment": 4,
              "fields": [
                { "name": "value", "offset": 0, "count": 1, "type": { "kind": "u32" } }
              ]
            }
          }
        }
      ]
    },
    "x64": {
      "size": 4,
      "alignment": 4,
      "fields": [
        {
          "name": "self",
          "offset": 0,
          "count": 1,
          "type": {
            "kind": "struct",
            "name": "Tests.Cycle",
            "layout": {
              "size": 4,
              "alignment": 4,
              "fields": [
                { "name": "value", "offset": 0, "count": 1, "type": { "kind": "u32" } }
              ]
            }
          }
        }
      ]
    },
    "arm64": {
      "size": 4,
      "alignment": 4,
      "fields": [
        {
          "name": "self",
          "offset": 0,
          "count": 1,
          "type": {
            "kind": "struct",
            "name": "Tests.Cycle",
            "layout": {
              "size": 4,
              "alignment": 4,
              "fields": [
                { "name": "value", "offset": 0, "count": 1, "type": { "kind": "u32" } }
              ]
            }
          }
        }
      ]
    }
  }"#;

  #[repr(C)]
  #[derive(Clone, Copy)]
  struct RawInner {
    values: [u16; 2],
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  struct RawAggregate {
    tag: u32,
    inner: RawInner,
    pointer: *mut c_void,
    guid: windows::core::GUID,
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  union OracleU1 {
    value: u8,
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  union OracleU2 {
    value: u16,
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  union OracleU4 {
    value: u32,
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  union OracleU8 {
    value: u64,
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  union OracleU3 {
    bytes: [u8; 3],
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  union OracleU5 {
    bytes: [u8; 5],
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  union OracleU6 {
    bytes: [u8; 6],
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  union OracleU7 {
    bytes: [u8; 7],
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  union OracleU16 {
    values: [u64; 2],
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  union OracleU24 {
    values: [u64; 3],
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  union OracleHfa2 {
    pair: [f32; 2],
    scalar: f32,
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  union OracleHfa1 {
    scalar: f32,
    values: [f32; 1],
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  union OracleHfa4 {
    scalar: f32,
    values: [f32; 4],
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  union OracleMixedHfa2 {
    pair: [f32; 2],
    integer: u64,
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  struct OracleStructHfa3 {
    union: OracleHfa2,
    tail: f32,
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  struct OracleStructMixed {
    union: OracleMixedHfa2,
    tail: f32,
    tag: u32,
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  union OracleNestedHfa2 {
    inner: OracleHfa2,
    values: [f32; 2],
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  union OracleDoubleHfa3 {
    triple: [f64; 3],
    pair: [f64; 2],
  }

  #[cfg(feature = "test-hooks")]
  unsafe extern "system" {
    fn raw_union_c_return_u1(this: *mut c_void) -> OracleU1;
    fn raw_union_c_return_u2(this: *mut c_void) -> OracleU2;
    fn raw_union_c_return_u3(this: *mut c_void) -> OracleU3;
    fn raw_union_c_return_u4(this: *mut c_void) -> OracleU4;
    fn raw_union_c_return_u5(this: *mut c_void) -> OracleU5;
    fn raw_union_c_return_u6(this: *mut c_void) -> OracleU6;
    fn raw_union_c_return_u7(this: *mut c_void) -> OracleU7;
    fn raw_union_c_return_u8(this: *mut c_void) -> OracleU8;
    fn raw_union_c_return_u16(this: *mut c_void) -> OracleU16;
    fn raw_union_c_return_u24(this: *mut c_void) -> OracleU24;
    fn raw_union_c_return_hfa1(this: *mut c_void) -> OracleHfa1;
    fn raw_union_c_return_hfa2(this: *mut c_void) -> OracleHfa2;
    fn raw_union_c_return_hfa4(this: *mut c_void) -> OracleHfa4;
    fn raw_union_c_return_mixed(this: *mut c_void) -> OracleMixedHfa2;
    fn raw_union_c_return_dhfa3(this: *mut c_void) -> OracleDoubleHfa3;
    fn raw_union_c_return_nested_union(this: *mut c_void) -> OracleNestedHfa2;
    fn raw_union_c_return_nested_struct(this: *mut c_void) -> OracleStructHfa3;
    fn raw_union_c_return_mixed_nested_struct(this: *mut c_void) -> OracleStructMixed;
    fn raw_union_c_u8_first(this: *mut c_void, value: OracleU8, a: u32, b: u32, c: u32) -> i32;
    fn raw_union_c_u8_fourth(
      this: *mut c_void,
      a: u32,
      b: u32,
      c: u32,
      value: OracleU8,
      tail: u32,
    ) -> i32;
    fn raw_union_c_u16_post_register(
      this: *mut c_void,
      a: u32,
      b: u32,
      c: u32,
      d: u32,
      value: OracleU16,
    ) -> i32;
    fn raw_union_c_u16_guarded_copy(
      this: *mut c_void,
      value: OracleU16,
      destination: *mut u8,
    ) -> i32;
    fn raw_union_c_u16_mutate_local(
      this: *mut c_void,
      before: u32,
      value: OracleU16,
      original: *const u8,
      after: u32,
    ) -> u64;
    fn raw_union_c_mixed_nested_struct_input(
      this: *mut c_void,
      before: u32,
      value: OracleStructMixed,
      after: u32,
    ) -> i32;
    fn raw_union_c_hfa1_input(this: *mut c_void, value: OracleHfa1, canary: u32) -> i32;
    fn raw_union_c_hfa2_input(this: *mut c_void, value: OracleHfa2, canary: u32) -> i32;
    fn raw_union_c_hfa4_input(this: *mut c_void, value: OracleHfa4, canary: u32) -> i32;
    fn raw_union_c_nested_union_input(
      this: *mut c_void,
      value: OracleNestedHfa2,
      canary: u32,
    ) -> i32;
  }

  unsafe extern "system" fn return_oracle_u1(_this: *mut c_void) -> OracleU1 {
    OracleU1 { value: 0x7a }
  }

  unsafe extern "system" fn return_oracle_u2(_this: *mut c_void) -> OracleU2 {
    OracleU2 { value: 0x7a6b }
  }

  unsafe extern "system" fn return_oracle_u4(_this: *mut c_void) -> OracleU4 {
    OracleU4 { value: 0x7a6b5c4d }
  }

  unsafe extern "system" fn return_oracle_u8(_this: *mut c_void) -> OracleU8 {
    OracleU8 {
      value: 0x7a6b5c4d3e2f1a0b,
    }
  }

  unsafe extern "system" fn return_oracle_u3(_this: *mut c_void) -> OracleU3 {
    OracleU3 { bytes: [1, 2, 3] }
  }

  unsafe extern "system" fn return_oracle_u5(_this: *mut c_void) -> OracleU5 {
    OracleU5 {
      bytes: [1, 2, 3, 4, 5],
    }
  }

  unsafe extern "system" fn return_oracle_u7(_this: *mut c_void) -> OracleU7 {
    OracleU7 {
      bytes: [1, 2, 3, 4, 5, 6, 7],
    }
  }

  unsafe extern "system" fn return_oracle_u16(_this: *mut c_void) -> OracleU16 {
    OracleU16 {
      values: [0x1111222233334444, 0x5555666677778888],
    }
  }

  unsafe extern "system" fn return_oracle_u24(_this: *mut c_void) -> OracleU24 {
    OracleU24 { values: [1, 2, 3] }
  }

  unsafe extern "system" fn return_oracle_hfa2(_this: *mut c_void) -> OracleHfa2 {
    OracleHfa2 { pair: [1.25, 2.5] }
  }

  unsafe extern "system" fn return_oracle_mixed_hfa2(_this: *mut c_void) -> OracleMixedHfa2 {
    OracleMixedHfa2 {
      integer: 0x1020304050607080,
    }
  }

  unsafe extern "system" fn return_oracle_struct_hfa3(_this: *mut c_void) -> OracleStructHfa3 {
    OracleStructHfa3 {
      union: OracleHfa2 { pair: [1.0, 2.0] },
      tail: 3.0,
    }
  }

  unsafe extern "system" fn return_oracle_double_hfa3(_this: *mut c_void) -> OracleDoubleHfa3 {
    OracleDoubleHfa3 {
      triple: [1.0, 2.0, 3.0],
    }
  }

  unsafe extern "system" fn observe_oracle_u8_first(
    _this: *mut c_void,
    value: OracleU8,
    a: u32,
    b: u32,
    c: u32,
  ) -> HRESULT {
    if unsafe { value.value } == 0x1122334455667788 && (a, b, c) == (11, 22, 33) {
      HRESULT(0)
    } else {
      HRESULT(0x80004005u32 as i32)
    }
  }

  unsafe extern "system" fn observe_oracle_u8_fourth(
    _this: *mut c_void,
    a: u32,
    b: u32,
    c: u32,
    value: OracleU8,
    tail: u32,
  ) -> HRESULT {
    if unsafe { value.value } == 0x1122334455667788 && (a, b, c, tail) == (11, 22, 33, 44) {
      HRESULT(0)
    } else {
      HRESULT(0x80004005u32 as i32)
    }
  }

  unsafe extern "system" fn observe_oracle_u16_post_registers(
    _this: *mut c_void,
    a: u32,
    b: u32,
    c: u32,
    d: u32,
    value: OracleU16,
  ) -> HRESULT {
    if unsafe { value.values } == [0x1111222233334444, 0x5555666677778888]
      && (a, b, c, d) == (1, 2, 3, 4)
    {
      HRESULT(0)
    } else {
      HRESULT(0x80004005u32 as i32)
    }
  }

  unsafe extern "system" fn write_oracle_u8_out(
    _this: *mut c_void,
    value: *mut OracleU8,
  ) -> HRESULT {
    unsafe {
      *value = OracleU8 {
        value: 0xaabbccddeeff0011,
      };
    }
    HRESULT(0)
  }

  unsafe extern "system" fn mutate_oracle_u8_inout(
    _this: *mut c_void,
    value: *mut OracleU8,
  ) -> HRESULT {
    unsafe {
      (*value).value ^= u64::MAX;
    }
    HRESULT(0)
  }

  fn raw_aggregate(tag: u32) -> RawAggregate {
    RawAggregate {
      tag,
      inner: RawInner { values: [5, 7] },
      pointer: std::ptr::with_exposed_provenance_mut::<c_void>(0x1234),
      guid: windows::core::GUID::zeroed(),
    }
  }

  fn raw_aggregate_bytes(value: RawAggregate) -> Vec<u8> {
    unsafe {
      std::slice::from_raw_parts(
        (&value as *const RawAggregate).cast::<u8>(),
        std::mem::size_of::<RawAggregate>(),
      )
    }
    .to_vec()
  }

  fn oracle_bytes<T>(value: &T) -> Vec<u8> {
    unsafe {
      std::slice::from_raw_parts((value as *const T).cast::<u8>(), std::mem::size_of::<T>())
    }
    .to_vec()
  }

  fn oracle_union_layout(
    name: &str,
    size: usize,
    alignment: usize,
    fields: serde_json::Value,
  ) -> DynComRawUnionLayout {
    DynComRawUnionLayout::from_descriptor(raw_descriptor(
      name,
      serde_json::json!({
        "size": size,
        "alignment": alignment,
        "complete": true,
        "fields": fields,
      }),
    ))
    .unwrap()
  }

  fn read_raw_aggregate(bytes: &[u8]) -> RawAggregate {
    unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<RawAggregate>()) }
  }

  fn raw_aggregate_score(value: &RawAggregate) -> u32 {
    value.tag + u32::from(value.inner.values[0]) + u32::from(value.inner.values[1])
  }

  unsafe extern "system" fn score_raw_aggregate_by_value(
    _this: *mut c_void,
    value: RawAggregate,
    result: *mut u32,
  ) -> windows::core::HRESULT {
    unsafe {
      *result = raw_aggregate_score(&value);
    }
    windows::core::HRESULT(0)
  }

  unsafe extern "system" fn score_raw_aggregate_pointer(
    _this: *mut c_void,
    value: *const RawAggregate,
    result: *mut u32,
  ) -> windows::core::HRESULT {
    unsafe {
      *result = raw_aggregate_score(&*value);
    }
    windows::core::HRESULT(0)
  }

  unsafe extern "system" fn write_raw_aggregate(
    _this: *mut c_void,
    value: *mut RawAggregate,
  ) -> windows::core::HRESULT {
    let initial = unsafe { &*value };
    if initial.tag != 0
      || initial.inner.values != [0, 0]
      || !initial.pointer.is_null()
      || initial.guid != windows::core::GUID::zeroed()
    {
      return windows::core::HRESULT(0x80004005u32 as i32);
    }
    unsafe {
      *value = raw_aggregate(40);
    }
    windows::core::HRESULT(0)
  }

  unsafe extern "system" fn update_raw_aggregate(
    _this: *mut c_void,
    value: *mut RawAggregate,
  ) -> windows::core::HRESULT {
    unsafe {
      (*value).tag += 1;
      (*value).inner.values[0] += 2;
      (*value).inner.values[1] += 3;
    }
    windows::core::HRESULT(0)
  }

  unsafe extern "system" fn return_raw_aggregate(_this: *mut c_void) -> RawAggregate {
    raw_aggregate(60)
  }

  #[repr(C)]
  struct RawAggregateComObject {
    vtable: *const *mut c_void,
  }

  fn invoke_raw_aggregate_method(
    table: &Arc<dynwinrt::MetadataTable>,
    function: *mut c_void,
    signature: dynwinrt::com::MethodSignature,
    args: &[&DynWinRTValue],
  ) -> napi::Result<Vec<DynWinRTValue>> {
    let interface = dynwinrt::com::register_interface(
      table,
      "Tests.IRawAggregate",
      windows::core::GUID::from_u128(0xbbbbbbbb_cccc_dddd_eeee_ffffffffffff),
      dynwinrt::com::InterfaceBase::IUnknown,
    )
    .add_method_at(3, "Call", signature)
    .map_err(|error| napi::Error::from_reason(error.message()))?;
    let mut vtable = [std::ptr::null_mut(); 4];
    vtable[3] = function;
    let mut object = RawAggregateComObject {
      vtable: vtable.as_ptr(),
    };
    with_com_invocation_args(args, |args| {
      unsafe {
        interface
          .method(3)
          .unwrap()
          .invoke_values_with_output_kinds((&mut object as *mut RawAggregateComObject).cast(), args)
      }
      .map(|values| {
        values
          .into_iter()
          .map(|(value, kind)| DynWinRTValue::from_com_value(value, kind))
          .collect()
      })
      .map_err(|error| napi::Error::from_reason(error.message()))
    })
  }

  fn direct_union_bytes(layout: &DynComRawUnionLayout, function: *mut c_void) -> Vec<u8> {
    let table = dynwinrt::MetadataTable::new();
    let result = invoke_raw_aggregate_method(
      &table,
      function,
      dynwinrt::com::MethodSignature::new(&table)
        .returns(layout.by_value_type().unwrap().0.clone()),
      &[],
    )
    .unwrap();
    let dynwinrt::com::Value::NativeUnion(returned) = result[0].to_com_value().unwrap() else {
      panic!("expected branded raw union direct return");
    };
    assert!(returned.active_field().is_none());
    layout.read_value_bytes(&result[0]).unwrap().to_vec()
  }

  #[test]
  fn raw_union_classifier_and_x64_oracles_cover_closed_pod_subset() {
    use dynwinrt::com::{NativeHomogeneousAggregate, NativeHomogeneousBase};
    use std::mem::{align_of, offset_of, size_of};

    assert_eq!((size_of::<OracleU1>(), align_of::<OracleU1>()), (1, 1));
    assert_eq!((size_of::<OracleU2>(), align_of::<OracleU2>()), (2, 2));
    assert_eq!((size_of::<OracleU4>(), align_of::<OracleU4>()), (4, 4));
    assert_eq!((size_of::<OracleU8>(), align_of::<OracleU8>()), (8, 8));
    assert_eq!((size_of::<OracleU3>(), align_of::<OracleU3>()), (3, 1));
    assert_eq!((size_of::<OracleU5>(), align_of::<OracleU5>()), (5, 1));
    assert_eq!((size_of::<OracleU7>(), align_of::<OracleU7>()), (7, 1));
    assert_eq!((size_of::<OracleU16>(), align_of::<OracleU16>()), (16, 8));
    assert_eq!((size_of::<OracleU24>(), align_of::<OracleU24>()), (24, 8));
    assert_eq!((size_of::<OracleHfa2>(), align_of::<OracleHfa2>()), (8, 4));
    assert_eq!(
      (size_of::<OracleMixedHfa2>(), align_of::<OracleMixedHfa2>()),
      (8, 8)
    );
    assert_eq!(
      (
        size_of::<OracleStructHfa3>(),
        align_of::<OracleStructHfa3>()
      ),
      (12, 4)
    );
    assert_eq!(offset_of!(OracleStructHfa3, union), 0);
    assert_eq!(offset_of!(OracleStructHfa3, tail), 8);
    assert_eq!(
      (
        size_of::<OracleDoubleHfa3>(),
        align_of::<OracleDoubleHfa3>()
      ),
      (24, 8)
    );

    let u1 = oracle_union_layout(
      "Tests.OracleU1",
      1,
      1,
      serde_json::json!([
        { "name": "value", "count": 1, "type": { "kind": "u8" } }
      ]),
    );
    let u2 = oracle_union_layout(
      "Tests.OracleU2",
      2,
      2,
      serde_json::json!([
        { "name": "value", "count": 1, "type": { "kind": "u16" } }
      ]),
    );
    let u4 = oracle_union_layout(
      "Tests.OracleU4",
      4,
      4,
      serde_json::json!([
        { "name": "value", "count": 1, "type": { "kind": "u32" } }
      ]),
    );
    let u8 = oracle_union_layout(
      "Tests.OracleU8",
      8,
      8,
      serde_json::json!([
        { "name": "value", "count": 1, "type": { "kind": "u64" } }
      ]),
    );
    let u16 = oracle_union_layout(
      "Tests.OracleU16",
      16,
      8,
      serde_json::json!([
        { "name": "values", "count": 2, "type": { "kind": "u64" } }
      ]),
    );
    let u24 = oracle_union_layout(
      "Tests.OracleU24",
      24,
      8,
      serde_json::json!([
        { "name": "values", "count": 3, "type": { "kind": "u64" } }
      ]),
    );
    let hfa2_fields = serde_json::json!([
      { "name": "pair", "count": 2, "type": { "kind": "f32" } },
      { "name": "scalar", "count": 1, "type": { "kind": "f32" } }
    ]);
    let hfa2 = oracle_union_layout("Tests.OracleHfa2", 8, 4, hfa2_fields.clone());
    assert_eq!(
      hfa2.layout.homogeneous_aggregate(),
      Some(NativeHomogeneousAggregate {
        base: NativeHomogeneousBase::F32,
        count: 2,
      })
    );
    let identical_hfa_bytes = oracle_bytes(&OracleHfa2 { pair: [1.0, 2.0] });
    let pair_active = hfa2
      .create_value(
        "pair".into(),
        Some(Buffer::from(identical_hfa_bytes.clone())),
      )
      .unwrap();
    let scalar_active = hfa2
      .create_value(
        "scalar".into(),
        Some(Buffer::from(identical_hfa_bytes.clone())),
      )
      .unwrap();
    assert_eq!(
      hfa2.read_value_bytes(&pair_active).unwrap().as_ref(),
      hfa2.read_value_bytes(&scalar_active).unwrap().as_ref()
    );
    let mixed = oracle_union_layout(
      "Tests.OracleMixedHfa2",
      8,
      8,
      serde_json::json!([
        { "name": "pair", "count": 2, "type": { "kind": "f32" } },
        { "name": "integer", "count": 1, "type": { "kind": "u64" } }
      ]),
    );
    assert_eq!(mixed.layout.homogeneous_aggregate(), None);
    let double_hfa3 = oracle_union_layout(
      "Tests.OracleDoubleHfa3",
      24,
      8,
      serde_json::json!([
        { "name": "triple", "count": 3, "type": { "kind": "f64" } },
        { "name": "pair", "count": 2, "type": { "kind": "f64" } }
      ]),
    );
    assert_eq!(
      double_hfa3.layout.homogeneous_aggregate(),
      Some(NativeHomogeneousAggregate {
        base: NativeHomogeneousBase::F64,
        count: 3,
      })
    );

    let nested_union_layout = serde_json::json!({
      "size": 8,
      "alignment": 4,
      "complete": true,
      "fields": hfa2_fields,
    });
    let struct_hfa_descriptor = raw_descriptor(
      "Tests.OracleStructHfa3",
      serde_json::json!({
        "size": 12,
        "alignment": 4,
        "fields": [
          {
            "name": "union",
            "offset": 0,
            "count": 1,
            "type": {
              "kind": "union",
              "name": "Tests.OracleStructHfa2Child",
              "layout": nested_union_layout
            }
          },
          { "name": "tail", "offset": 8, "count": 1, "type": { "kind": "f32" } }
        ]
      }),
    );
    let struct_hfa = DynComRawStructLayout::from_descriptor(struct_hfa_descriptor).unwrap();
    assert_eq!(
      struct_hfa.layout.homogeneous_aggregate(),
      Some(NativeHomogeneousAggregate {
        base: NativeHomogeneousBase::F32,
        count: 3,
      })
    );

    if !cfg!(target_arch = "aarch64") {
      assert_eq!(
        direct_union_bytes(&u1, return_oracle_u1 as *mut c_void),
        oracle_bytes(&OracleU1 { value: 0x7a })
      );
      assert_eq!(
        direct_union_bytes(&u2, return_oracle_u2 as *mut c_void),
        oracle_bytes(&OracleU2 { value: 0x7a6b })
      );
      assert_eq!(
        direct_union_bytes(&u4, return_oracle_u4 as *mut c_void),
        oracle_bytes(&OracleU4 { value: 0x7a6b5c4d })
      );
      assert_eq!(
        direct_union_bytes(&u8, return_oracle_u8 as *mut c_void),
        oracle_bytes(&OracleU8 {
          value: 0x7a6b5c4d3e2f1a0b,
        })
      );
      assert_eq!(
        direct_union_bytes(&u16, return_oracle_u16 as *mut c_void),
        oracle_bytes(&OracleU16 {
          values: [0x1111222233334444, 0x5555666677778888],
        })
      );
      assert_eq!(
        direct_union_bytes(&u24, return_oracle_u24 as *mut c_void),
        oracle_bytes(&OracleU24 { values: [1, 2, 3] })
      );
      assert_eq!(
        direct_union_bytes(&hfa2, return_oracle_hfa2 as *mut c_void),
        oracle_bytes(&OracleHfa2 { pair: [1.25, 2.5] })
      );
      assert_eq!(
        direct_union_bytes(&mixed, return_oracle_mixed_hfa2 as *mut c_void),
        oracle_bytes(&OracleMixedHfa2 {
          integer: 0x1020304050607080,
        })
      );
      assert_eq!(
        direct_union_bytes(&double_hfa3, return_oracle_double_hfa3 as *mut c_void),
        oracle_bytes(&OracleDoubleHfa3 {
          triple: [1.0, 2.0, 3.0],
        })
      );
      let table = dynwinrt::MetadataTable::new();
      let returned = invoke_raw_aggregate_method(
        &table,
        return_oracle_struct_hfa3 as *mut c_void,
        dynwinrt::com::MethodSignature::new(&table)
          .returns(struct_hfa.by_value_type().unwrap().0.clone()),
        &[],
      )
      .unwrap();
      assert_eq!(
        struct_hfa.read_value_bytes(&returned[0]).unwrap().as_ref(),
        oracle_bytes(&OracleStructHfa3 {
          union: OracleHfa2 { pair: [1.0, 2.0] },
          tail: 3.0,
        })
      );
    }

    for size in [3usize, 5, 6, 7] {
      let odd = oracle_union_layout(
        &format!("Tests.OracleU{size}"),
        size,
        1,
        serde_json::json!([
          { "name": "bytes", "count": size, "type": { "kind": "u8" } }
        ]),
      );
      if cfg!(target_arch = "x86_64") {
        assert!(odd.by_value_type().is_err());
        let odd_struct = DynComRawStructLayout::from_descriptor(raw_descriptor(
          &format!("Tests.OracleStructU{size}"),
          serde_json::json!({
            "size": size,
            "alignment": 1,
            "fields": [
              { "name": "bytes", "offset": 0, "count": size, "type": { "kind": "u8" } }
            ]
          }),
        ))
        .unwrap();
        assert!(odd_struct.by_value_type().is_err());
      } else if cfg!(target_arch = "x86") {
        let (function, expected) = match size {
          3 => (
            return_oracle_u3 as *mut c_void,
            oracle_bytes(&OracleU3 { bytes: [1, 2, 3] }),
          ),
          5 => (
            return_oracle_u5 as *mut c_void,
            oracle_bytes(&OracleU5 {
              bytes: [1, 2, 3, 4, 5],
            }),
          ),
          7 => (
            return_oracle_u7 as *mut c_void,
            oracle_bytes(&OracleU7 {
              bytes: [1, 2, 3, 4, 5, 6, 7],
            }),
          ),
          6 => {
            // The MSVC x86 return ABI for six-byte aggregates is sret. Reuse
            // the seven-byte mirror with an exact six-byte descriptor below.
            continue;
          }
          _ => unreachable!(),
        };
        assert_eq!(direct_union_bytes(&odd, function), expected);
      }
    }
  }

  #[test]
  fn raw_union_arguments_cover_register_and_stack_positions_without_mutation() {
    let u8 = oracle_union_layout(
      "Tests.OracleArgU8",
      8,
      8,
      serde_json::json!([
        { "name": "value", "count": 1, "type": { "kind": "u64" } }
      ]),
    );
    assert_eq!(u8.layout.size(), 8);
  }

  #[cfg(feature = "test-hooks")]
  #[test]
  fn msvc_c_union_oracle_executes_through_libffi() {
    let integer = |name: &str, size: usize, alignment: usize, kind: &str, count: usize| {
      oracle_union_layout(
        name,
        size,
        alignment,
        serde_json::json!([
          { "name": "value", "count": count, "type": { "kind": kind } }
        ]),
      )
    };
    let u1 = integer("Tests.COracleU1", 1, 1, "u8", 1);
    let u2 = integer("Tests.COracleU2", 2, 2, "u16", 1);
    let u3 = integer("Tests.COracleU3", 3, 1, "u8", 3);
    let u4 = integer("Tests.COracleU4", 4, 4, "u32", 1);
    let u5 = integer("Tests.COracleU5", 5, 1, "u8", 5);
    let u6 = integer("Tests.COracleU6", 6, 1, "u8", 6);
    let u7 = integer("Tests.COracleU7", 7, 1, "u8", 7);
    let u8 = integer("Tests.COracleU8", 8, 8, "u64", 1);
    let u16 = integer("Tests.COracleU16", 16, 8, "u64", 2);
    let u24 = integer("Tests.COracleU24", 24, 8, "u64", 3);
    let hfa1 = oracle_union_layout(
      "Tests.COracleHfa1",
      4,
      4,
      serde_json::json!([
        { "name": "scalar", "count": 1, "type": { "kind": "f32" } },
        { "name": "values", "count": 1, "type": { "kind": "f32" } }
      ]),
    );
    let hfa2_fields = serde_json::json!([
      { "name": "scalar", "count": 1, "type": { "kind": "f32" } },
      { "name": "values", "count": 2, "type": { "kind": "f32" } }
    ]);
    let hfa2 = oracle_union_layout("Tests.COracleHfa2", 8, 4, hfa2_fields.clone());
    let hfa4 = oracle_union_layout(
      "Tests.COracleHfa4",
      16,
      4,
      serde_json::json!([
        { "name": "scalar", "count": 1, "type": { "kind": "f32" } },
        { "name": "values", "count": 4, "type": { "kind": "f32" } }
      ]),
    );
    let mixed = oracle_union_layout(
      "Tests.COracleMixed",
      8,
      8,
      serde_json::json!([
        { "name": "values", "count": 2, "type": { "kind": "f32" } },
        { "name": "integer", "count": 1, "type": { "kind": "u64" } }
      ]),
    );
    let dhfa3 = oracle_union_layout(
      "Tests.COracleDoubleHfa3",
      24,
      8,
      serde_json::json!([
        { "name": "scalar", "count": 1, "type": { "kind": "f64" } },
        { "name": "values", "count": 3, "type": { "kind": "f64" } }
      ]),
    );
    let nested_union = oracle_union_layout(
      "Tests.COracleNestedUnion",
      8,
      4,
      serde_json::json!([
        {
          "name": "inner",
          "count": 1,
          "type": {
            "kind": "union",
            "name": "Tests.COracleNestedUnionInner",
            "layout": {
              "size": 8,
              "alignment": 4,
              "complete": true,
              "fields": hfa2_fields
            }
          }
        },
        { "name": "values", "count": 2, "type": { "kind": "f32" } }
      ]),
    );
    let nested_struct = DynComRawStructLayout::from_descriptor(raw_descriptor(
      "Tests.COracleNestedStruct",
      serde_json::json!({
        "size": 12,
        "alignment": 4,
        "fields": [
          {
            "name": "inner",
            "offset": 0,
            "count": 1,
            "type": {
              "kind": "union",
              "name": "Tests.COracleNestedStructUnion",
              "layout": {
                "size": 8,
                "alignment": 4,
                "complete": true,
                "fields": [
                  { "name": "scalar", "count": 1, "type": { "kind": "f32" } },
                  { "name": "values", "count": 2, "type": { "kind": "f32" } }
                ]
              }
            }
          },
          { "name": "tail", "offset": 8, "count": 1, "type": { "kind": "f32" } }
        ]
      }),
    ))
    .unwrap();
    let mixed_nested_struct = DynComRawStructLayout::from_descriptor(raw_descriptor(
      "Tests.COracleMixedNestedStruct",
      serde_json::json!({
        "size": 16,
        "alignment": 8,
        "fields": [
          {
            "name": "inner",
            "offset": 0,
            "count": 1,
            "type": {
              "kind": "union",
              "name": "Tests.COracleMixedNestedStructUnion",
              "layout": {
                "size": 8,
                "alignment": 8,
                "complete": true,
                "fields": [
                  { "name": "values", "count": 2, "type": { "kind": "f32" } },
                  { "name": "integer", "count": 1, "type": { "kind": "u64" } }
                ]
              }
            }
          },
          { "name": "tail", "offset": 8, "count": 1, "type": { "kind": "f32" } },
          { "name": "tag", "offset": 12, "count": 1, "type": { "kind": "u32" } }
        ]
      }),
    ))
    .unwrap();
    assert_eq!(mixed_nested_struct.layout.homogeneous_aggregate(), None);

    macro_rules! assert_direct {
      ($layout:expr, $function:path, $expected:expr) => {
        assert_eq!(
          direct_union_bytes(&$layout, $function as *mut c_void),
          oracle_bytes(&$expected)
        );
      };
    }
    assert_direct!(u1, raw_union_c_return_u1, OracleU1 { value: 0x7a });
    assert_direct!(u2, raw_union_c_return_u2, OracleU2 { value: 0x7a6b });
    assert_direct!(u4, raw_union_c_return_u4, OracleU4 { value: 0x7a6b5c4d });
    assert_direct!(
      u8,
      raw_union_c_return_u8,
      OracleU8 {
        value: 0x7a6b5c4d3e2f1a0b
      }
    );
    assert_direct!(
      u16,
      raw_union_c_return_u16,
      OracleU16 {
        values: [0x1111222233334444, 0x5555666677778888]
      }
    );
    assert_direct!(u24, raw_union_c_return_u24, OracleU24 { values: [1, 2, 3] });
    assert_direct!(hfa1, raw_union_c_return_hfa1, OracleHfa1 { scalar: 1.25 });
    assert_direct!(
      hfa2,
      raw_union_c_return_hfa2,
      OracleHfa2 { pair: [1.25, 2.5] }
    );
    assert_direct!(
      hfa4,
      raw_union_c_return_hfa4,
      OracleHfa4 {
        values: [1.0, 2.0, 3.0, 4.0]
      }
    );
    assert_direct!(
      mixed,
      raw_union_c_return_mixed,
      OracleMixedHfa2 {
        integer: 0x1020304050607080
      }
    );
    assert_direct!(
      dhfa3,
      raw_union_c_return_dhfa3,
      OracleDoubleHfa3 {
        triple: [1.0, 2.0, 3.0]
      }
    );
    assert_direct!(
      nested_union,
      raw_union_c_return_nested_union,
      OracleNestedHfa2 {
        inner: OracleHfa2 { pair: [5.0, 6.0] }
      }
    );
    let table = dynwinrt::MetadataTable::new();
    let nested_return = invoke_raw_aggregate_method(
      &table,
      raw_union_c_return_nested_struct as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table)
        .returns(nested_struct.by_value_type().unwrap().0.clone()),
      &[],
    )
    .unwrap();
    assert_eq!(
      nested_struct
        .read_value_bytes(&nested_return[0])
        .unwrap()
        .as_ref(),
      oracle_bytes(&OracleStructHfa3 {
        union: OracleHfa2 { pair: [1.0, 2.0] },
        tail: 3.0
      })
    );
    let mixed_nested_return = invoke_raw_aggregate_method(
      &table,
      raw_union_c_return_mixed_nested_struct as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table)
        .returns(mixed_nested_struct.by_value_type().unwrap().0.clone()),
      &[],
    )
    .unwrap();
    let mixed_nested_bytes = oracle_bytes(&OracleStructMixed {
      union: OracleMixedHfa2 {
        integer: 0x1020304050607080,
      },
      tail: 3.5,
      tag: 0xa1b2c3d4,
    });
    assert_eq!(
      mixed_nested_struct
        .read_value_bytes(&mixed_nested_return[0])
        .unwrap()
        .as_ref(),
      mixed_nested_bytes
    );

    if cfg!(target_arch = "x86") {
      assert_direct!(u3, raw_union_c_return_u3, OracleU3 { bytes: [1, 2, 3] });
      assert_direct!(
        u5,
        raw_union_c_return_u5,
        OracleU5 {
          bytes: [1, 2, 3, 4, 5]
        }
      );
      assert_direct!(
        u6,
        raw_union_c_return_u6,
        OracleU6 {
          bytes: [1, 2, 3, 4, 5, 6]
        }
      );
      assert_direct!(
        u7,
        raw_union_c_return_u7,
        OracleU7 {
          bytes: [1, 2, 3, 4, 5, 6, 7]
        }
      );
    } else {
      assert!(u3.by_value_type().is_err());
      assert!(u5.by_value_type().is_err());
      assert!(u6.by_value_type().is_err());
      assert!(u7.by_value_type().is_err());
    }

    let u8_bytes = oracle_bytes(&OracleU8 {
      value: 0x1122334455667788,
    });
    let u16_bytes = oracle_bytes(&OracleU16 {
      values: [0x1111222233334444, 0x5555666677778888],
    });
    let u8_value = u8
      .create_value("value".into(), Some(Buffer::from(u8_bytes.clone())))
      .unwrap();
    let u16_value = u16
      .create_value("value".into(), Some(Buffer::from(u16_bytes.clone())))
      .unwrap();
    let scalar = |value| DynWinRTValue::new(dynwinrt::WinRTValue::U32(value));
    invoke_raw_aggregate_method(
      &table,
      raw_union_c_u8_first as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table)
        .add_in(u8.by_value_type().unwrap().0.clone())
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(dynwinrt::com::Type::winrt(table.u32_type())),
      &[&u8_value, &scalar(11), &scalar(22), &scalar(33)],
    )
    .unwrap();
    invoke_raw_aggregate_method(
      &table,
      raw_union_c_u8_fourth as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table)
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(u8.by_value_type().unwrap().0.clone())
        .add_in(dynwinrt::com::Type::winrt(table.u32_type())),
      &[
        &scalar(11),
        &scalar(22),
        &scalar(33),
        &u8_value,
        &scalar(44),
      ],
    )
    .unwrap();
    invoke_raw_aggregate_method(
      &table,
      raw_union_c_u16_post_register as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table)
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(u16.by_value_type().unwrap().0.clone()),
      &[&scalar(1), &scalar(2), &scalar(3), &scalar(4), &u16_value],
    )
    .unwrap();

    let guarded = raw_memory(18, 16);
    guarded.write_u8(number(0), 0xa5 as f64).unwrap();
    guarded.write_u8(number(17), 0x5a as f64).unwrap();
    let guarded_destination = guarded
      .pointer(Some(number(1)))
      .unwrap()
      .to_value()
      .unwrap();
    invoke_raw_aggregate_method(
      &table,
      raw_union_c_u16_guarded_copy as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table)
        .add_in(u16.by_value_type().unwrap().0.clone())
        .add_in(dynwinrt::com::Type::pointer()),
      &[&u16_value, &guarded_destination],
    )
    .unwrap();
    assert_eq!(guarded.read_u8(number(0)).unwrap(), 0xa5);
    assert_eq!(guarded.read_u8(number(17)).unwrap(), 0x5a);
    assert_eq!(
      guarded.read_bytes(number(1), number(16)).unwrap().as_ref(),
      u16_bytes
    );

    let mut guarded_original = [0u8; 18];
    guarded_original[0] = 0xa5;
    guarded_original[1..17].copy_from_slice(&u16_bytes);
    guarded_original[17] = 0x5a;
    let original_pointer = external_pointer(guarded_original[1..].as_ptr().expose_provenance())
      .to_value()
      .unwrap();
    let mutation_result = invoke_raw_aggregate_method(
      &table,
      raw_union_c_u16_mutate_local as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table)
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(u16.by_value_type().unwrap().0.clone())
        .add_in(dynwinrt::com::Type::pointer())
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .returns(dynwinrt::com::Type::winrt(table.u64_type())),
      &[
        &scalar(0x11223344),
        &u16_value,
        &original_pointer,
        &scalar(0x55667788),
      ],
    )
    .unwrap();
    let mutated_first = 0x1111222233334444u64 ^ 0xffff0000ffff0000;
    let mutated_second = 0x5555666677778888u64.wrapping_add(0x0102030405060708);
    assert!(matches!(
      mutation_result.as_slice(),
      [DynWinRTValue(dynwinrt::WinRTValue::U64(value), ..)]
        if *value == mutated_first ^ mutated_second
    ));
    assert_eq!(guarded_original[0], 0xa5);
    assert_eq!(&guarded_original[1..17], u16_bytes);
    assert_eq!(guarded_original[17], 0x5a);
    assert_eq!(
      u16.read_value_bytes(&u16_value).unwrap().as_ref(),
      u16_bytes
    );

    let mixed_nested_value = mixed_nested_struct
      .create_value(Some(Buffer::from(mixed_nested_bytes.clone())))
      .unwrap();
    invoke_raw_aggregate_method(
      &table,
      raw_union_c_mixed_nested_struct_input as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table)
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(mixed_nested_struct.by_value_type().unwrap().0.clone())
        .add_in(dynwinrt::com::Type::winrt(table.u32_type())),
      &[
        &scalar(0x13579bdf),
        &mixed_nested_value,
        &scalar(0x2468ace0),
      ],
    )
    .unwrap();
    assert_eq!(
      mixed_nested_struct
        .read_value_bytes(&mixed_nested_value)
        .unwrap()
        .as_ref(),
      mixed_nested_bytes
    );

    for (layout, bytes, function, canary) in [
      (
        &hfa1,
        oracle_bytes(&OracleHfa1 { scalar: 1.25 }),
        raw_union_c_hfa1_input as *mut c_void,
        0xa1a2a3a4,
      ),
      (
        &hfa2,
        oracle_bytes(&OracleHfa2 { pair: [1.25, 2.5] }),
        raw_union_c_hfa2_input as *mut c_void,
        0xb1b2b3b4,
      ),
      (
        &hfa4,
        oracle_bytes(&OracleHfa4 {
          values: [1.0, 2.0, 3.0, 4.0],
        }),
        raw_union_c_hfa4_input as *mut c_void,
        0xc1c2c3c4,
      ),
    ] {
      let value = layout
        .create_value("values".into(), Some(Buffer::from(bytes.clone())))
        .unwrap();
      invoke_raw_aggregate_method(
        &table,
        function,
        dynwinrt::com::MethodSignature::new(&table)
          .add_in(layout.by_value_type().unwrap().0.clone())
          .add_in(dynwinrt::com::Type::winrt(table.u32_type())),
        &[&value, &scalar(canary)],
      )
      .unwrap();
      assert_eq!(layout.read_value_bytes(&value).unwrap().as_ref(), bytes);
    }
    let identical_hfa_bytes = oracle_bytes(&OracleHfa2 { pair: [1.25, 2.5] });
    for active_field in ["scalar", "values"] {
      let value = hfa2
        .create_value(
          active_field.into(),
          Some(Buffer::from(identical_hfa_bytes.clone())),
        )
        .unwrap();
      invoke_raw_aggregate_method(
        &table,
        raw_union_c_hfa2_input as *mut c_void,
        dynwinrt::com::MethodSignature::new(&table)
          .add_in(hfa2.by_value_type().unwrap().0.clone())
          .add_in(dynwinrt::com::Type::winrt(table.u32_type())),
        &[&value, &scalar(0xb1b2b3b4)],
      )
      .unwrap();
      assert_eq!(
        hfa2.read_value_bytes(&value).unwrap().as_ref(),
        identical_hfa_bytes
      );
    }
    let nested_bytes = oracle_bytes(&OracleNestedHfa2 {
      inner: OracleHfa2 { pair: [5.0, 6.0] },
    });
    let nested_value = nested_union
      .create_value("inner".into(), Some(Buffer::from(nested_bytes.clone())))
      .unwrap();
    invoke_raw_aggregate_method(
      &table,
      raw_union_c_nested_union_input as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table)
        .add_in(nested_union.by_value_type().unwrap().0.clone())
        .add_in(dynwinrt::com::Type::winrt(table.u32_type())),
      &[&nested_value, &scalar(0xd1d2d3d4)],
    )
    .unwrap();
    assert_eq!(
      nested_union
        .read_value_bytes(&nested_value)
        .unwrap()
        .as_ref(),
      nested_bytes
    );
  }

  #[test]
  fn raw_union_argument_positions_and_out_storage() {
    let u8 = oracle_union_layout(
      "Tests.OracleArgU8",
      8,
      8,
      serde_json::json!([
        { "name": "value", "count": 1, "type": { "kind": "u64" } }
      ]),
    );
    let u16 = oracle_union_layout(
      "Tests.OracleArgU16",
      16,
      8,
      serde_json::json!([
        { "name": "values", "count": 2, "type": { "kind": "u64" } }
      ]),
    );
    let u8_bytes = oracle_bytes(&OracleU8 {
      value: 0x1122334455667788,
    });
    let u16_bytes = oracle_bytes(&OracleU16 {
      values: [0x1111222233334444, 0x5555666677778888],
    });
    let u8_value = u8
      .create_value("value".into(), Some(Buffer::from(u8_bytes.clone())))
      .unwrap();
    let u16_value = u16
      .create_value("values".into(), Some(Buffer::from(u16_bytes.clone())))
      .unwrap();
    let table = dynwinrt::MetadataTable::new();
    let scalar = |value| DynWinRTValue::new(dynwinrt::WinRTValue::U32(value));

    invoke_raw_aggregate_method(
      &table,
      observe_oracle_u8_first as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table)
        .add_in(u8.by_value_type().unwrap().0.clone())
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(dynwinrt::com::Type::winrt(table.u32_type())),
      &[&u8_value, &scalar(11), &scalar(22), &scalar(33)],
    )
    .unwrap();
    invoke_raw_aggregate_method(
      &table,
      observe_oracle_u8_fourth as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table)
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(u8.by_value_type().unwrap().0.clone())
        .add_in(dynwinrt::com::Type::winrt(table.u32_type())),
      &[
        &scalar(11),
        &scalar(22),
        &scalar(33),
        &u8_value,
        &scalar(44),
      ],
    )
    .unwrap();
    invoke_raw_aggregate_method(
      &table,
      observe_oracle_u16_post_registers as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table)
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(dynwinrt::com::Type::winrt(table.u32_type()))
        .add_in(u16.by_value_type().unwrap().0.clone()),
      &[&scalar(1), &scalar(2), &scalar(3), &scalar(4), &u16_value],
    )
    .unwrap();
    let output = invoke_raw_aggregate_method(
      &table,
      write_oracle_u8_out as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table).add_out(u8.by_value_type().unwrap().0.clone()),
      &[],
    )
    .unwrap();
    assert_eq!(
      u8.assert_active_field(&output[0], "value".into())
        .unwrap()
        .as_ref(),
      &0xaabbccddeeff0011u64.to_ne_bytes()
    );
    let inout = invoke_raw_aggregate_method(
      &table,
      mutate_oracle_u8_inout as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table).add_in_out(u8.by_value_type().unwrap().0.clone()),
      &[&u8_value],
    )
    .unwrap();
    assert_eq!(
      u8.assert_active_field(&inout[0], "value".into())
        .unwrap()
        .as_ref(),
      &(!0x1122334455667788u64).to_ne_bytes()
    );
    assert_eq!(u8.read_value_bytes(&u8_value).unwrap().as_ref(), u8_bytes);
    assert_eq!(
      u16.read_value_bytes(&u16_value).unwrap().as_ref(),
      u16_bytes
    );
  }

  #[test]
  fn raw_struct_layout_drives_all_existing_aggregate_call_shapes() {
    assert_eq!(
      std::mem::size_of::<RawAggregate>(),
      if cfg!(target_pointer_width = "64") {
        32
      } else {
        28
      }
    );
    let layout = DynComRawStructLayout::from_descriptor(RAW_AGGREGATE_DESCRIPTOR.into()).unwrap();
    assert_eq!(layout.qualified_name(), "Tests.RawAggregate");
    assert_eq!(
      layout.size().get_u64().1,
      std::mem::size_of::<RawAggregate>() as u64
    );
    assert_eq!(
      layout.alignment().get_u64().1,
      std::mem::align_of::<RawAggregate>() as u64
    );
    let input = layout
      .create_value(Some(Buffer::from(raw_aggregate_bytes(raw_aggregate(30)))))
      .unwrap();
    assert_eq!(
      layout.read_value_bytes(&input).unwrap().len(),
      std::mem::size_of::<RawAggregate>()
    );
    let table = dynwinrt::MetadataTable::new();

    let by_value = invoke_raw_aggregate_method(
      &table,
      score_raw_aggregate_by_value as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table)
        .add_in(layout.by_value_type().unwrap().0.clone())
        .add_out(dynwinrt::com::Type::winrt(table.u32_type())),
      &[&input],
    )
    .unwrap();
    assert!(matches!(by_value[0].0, dynwinrt::WinRTValue::U32(42)));

    let pointer = invoke_raw_aggregate_method(
      &table,
      score_raw_aggregate_pointer as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table)
        .add_in(layout.pointer_type(None).0.clone())
        .add_out(dynwinrt::com::Type::winrt(table.u32_type())),
      &[&input],
    )
    .unwrap();
    assert!(matches!(pointer[0].0, dynwinrt::WinRTValue::U32(42)));

    let output = invoke_raw_aggregate_method(
      &table,
      write_raw_aggregate as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table)
        .add_out(layout.by_value_type().unwrap().0.clone()),
      &[],
    )
    .unwrap();
    let output = read_raw_aggregate(&layout.read_value_bytes(&output[0]).unwrap());
    assert_eq!(raw_aggregate_score(&output), 52);

    let in_out = invoke_raw_aggregate_method(
      &table,
      update_raw_aggregate as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table)
        .add_in_out(layout.by_value_type().unwrap().0.clone()),
      &[&input],
    )
    .unwrap();
    let in_out = read_raw_aggregate(&layout.read_value_bytes(&in_out[0]).unwrap());
    assert_eq!(raw_aggregate_score(&in_out), 48);

    let direct = invoke_raw_aggregate_method(
      &table,
      return_raw_aggregate as *mut c_void,
      dynwinrt::com::MethodSignature::new(&table)
        .returns(layout.by_value_type().unwrap().0.clone()),
      &[],
    )
    .unwrap();
    let direct = read_raw_aggregate(&layout.read_value_bytes(&direct[0]).unwrap());
    assert_eq!(raw_aggregate_score(&direct), 72);
  }

  #[test]
  fn raw_aggregate_descriptors_validate_identity_cycles_and_union_limits() {
    let layout = DynComRawStructLayout::from_descriptor(RAW_AGGREGATE_DESCRIPTOR.into()).unwrap();
    let value = layout.create_value(None).unwrap();
    let other = DynComRawStructLayout::from_descriptor(
      RAW_AGGREGATE_DESCRIPTOR.replace("Tests.RawAggregate", "Tests.OtherAggregate"),
    )
    .unwrap();
    assert!(other.read_value_bytes(&value).is_err());
    assert!(DynComRawStructLayout::from_descriptor(CYCLIC_AGGREGATE_DESCRIPTOR.into()).is_err());
    assert!(DynComRawStructLayout::from_descriptor(
      RAW_AGGREGATE_DESCRIPTOR.replace("Tests.RawAggregate", "RawAggregate")
    )
    .is_err());
    let mut overlapping: serde_json::Value =
      serde_json::from_str(RAW_AGGREGATE_DESCRIPTOR).unwrap();
    for architecture in ["x86", "x64", "arm64"] {
      overlapping[architecture]["fields"][1]["offset"] = serde_json::Value::from(0);
    }
    assert!(DynComRawStructLayout::from_descriptor(overlapping.to_string()).is_err());

    let union = DynComRawUnionLayout::from_descriptor(RAW_UNION_DESCRIPTOR.into()).unwrap();
    assert_eq!(union.qualified_name(), "Tests.RawUnion");
    if cfg!(target_arch = "aarch64") {
      assert!(union.by_value_type().is_err());
    } else {
      assert!(union.by_value_type().is_ok());
    }
    let union_value = union
      .create_value(
        "integer".into(),
        Some(Buffer::from(42u64.to_ne_bytes().to_vec())),
      )
      .unwrap();
    assert_eq!(
      union.read_value_bytes(&union_value).unwrap().as_ref(),
      &42u64.to_ne_bytes()
    );
    let _ = union.pointer_type();
  }

  #[test]
  fn raw_by_value_schema_rejects_unsupported_and_unknown_recursive_keys() {
    let descriptor = raw_descriptor(
      "Tests.StrictSchemaStruct",
      serde_json::json!({
        "size": 16,
        "alignment": 8,
        "fields": [
          { "name": "size", "offset": 0, "count": 1, "type": { "kind": "u32" } },
          {
            "name": "value",
            "offset": 8,
            "count": 1,
            "type": {
              "kind": "union",
              "name": "Tests.StrictSchemaUnion",
              "layout": {
                "size": 8,
                "alignment": 8,
                "complete": true,
                "fields": [
                  { "name": "integer", "count": 1, "type": { "kind": "u64" } }
                ]
              }
            }
          }
        ]
      }),
    );
    let mut exact: serde_json::Value = serde_json::from_str(&descriptor).unwrap();
    exact["initializers"] = serde_json::json!([
      { "kind": "sizeOfLayout", "field": "size" }
    ]);
    validate_raw_by_value_schema(&exact, RawDescriptorKind::Struct).unwrap();
    let exact_layout = DynComRawStructLayout::from_descriptor(exact.to_string()).unwrap();
    let _ = exact_layout.pointer_type(None);
    if cfg!(target_arch = "aarch64") {
      assert!(exact_layout.by_value_type().is_err());
    } else {
      assert!(exact_layout.by_value_type().is_ok());
    }

    let assert_rejected = |descriptor: serde_json::Value| {
      let layout = DynComRawStructLayout::from_descriptor(descriptor.to_string()).unwrap();
      let _ = layout.pointer_type(None);
      let error = layout.by_value_type().err().unwrap();
      assert!(
        error.reason.contains("unknown key")
          || error.reason.contains("unsupported ABI marker")
          || error.reason.contains("Unsupported raw by-value")
      );
    };
    let host = raw_host_architecture();

    for key in [
      "packed",
      "pack",
      "opaque",
      "nontrivial",
      "vector",
      "HVA",
      "flexibleArray",
      "overAligned",
      "customAlignment",
      "selectedMemberOnly",
      "incomplete",
    ] {
      let mut invalid = exact.clone();
      invalid[host][key] = serde_json::Value::Bool(true);
      assert_rejected(invalid);
    }
    let mut bitfield = exact.clone();
    bitfield[host]["fields"][0]["bitWidth"] = serde_json::Value::from(3);
    assert_rejected(bitfield);

    let mut unknown_root = exact.clone();
    unknown_root["unknownRoot"] = serde_json::Value::Bool(true);
    assert_rejected(unknown_root);
    let mut unknown_layout = exact.clone();
    unknown_layout[host]["unknownLayout"] = serde_json::Value::Bool(true);
    assert_rejected(unknown_layout);
    let mut unknown_field = exact.clone();
    unknown_field[host]["fields"][1]["unknownField"] = serde_json::Value::Bool(true);
    assert_rejected(unknown_field);
    let mut unknown_type = exact.clone();
    unknown_type[host]["fields"][1]["type"]["unknownType"] = serde_json::Value::Bool(true);
    assert_rejected(unknown_type);
    let mut unknown_nested_layout = exact.clone();
    unknown_nested_layout[host]["fields"][1]["type"]["layout"]["unknownNestedLayout"] =
      serde_json::Value::Bool(true);
    assert_rejected(unknown_nested_layout);
    let mut unknown_nested_field = exact.clone();
    unknown_nested_field[host]["fields"][1]["type"]["layout"]["fields"][0]["unknownNestedField"] =
      serde_json::Value::Bool(true);
    assert_rejected(unknown_nested_field);
    let mut unknown_nested_type = exact.clone();
    unknown_nested_type[host]["fields"][1]["type"]["layout"]["fields"][0]["type"]
      ["unknownNestedType"] = serde_json::Value::Bool(true);
    assert_rejected(unknown_nested_type);
    let mut unknown_initializer = exact;
    unknown_initializer["initializers"][0]["unknownInitializer"] = serde_json::Value::Bool(true);
    assert_rejected(unknown_initializer);
  }

  #[test]
  fn raw_aggregate_limits_reject_size_padding_nested_and_count_expansion() {
    let oversized = raw_descriptor(
      "Tests.Oversized",
      serde_json::json!({
        "size": RAW_MAX_AGGREGATE_BYTE_SIZE + 1,
        "alignment": 1,
        "fields": [
          { "name": "value", "offset": 0, "count": 1, "type": { "kind": "u8" } }
        ]
      }),
    );
    assert!(DynComRawStructLayout::from_descriptor(oversized).is_err());

    let padding = raw_descriptor(
      "Tests.PaddingExpansion",
      serde_json::json!({
        "size": RAW_MAX_LIBFFI_ELEMENTS + 4,
        "alignment": 4,
        "fields": [
          {
            "name": "value",
            "offset": RAW_MAX_LIBFFI_ELEMENTS,
            "count": 1,
            "type": { "kind": "u32" }
          }
        ]
      }),
    );
    let error = DynComRawStructLayout::from_descriptor(padding)
      .err()
      .unwrap();
    assert!(error.reason.contains("libffi elements"));

    let nested_layout = serde_json::json!({
      "size": 256,
      "alignment": 1,
      "fields": [
        { "name": "bytes", "offset": 0, "count": 256, "type": { "kind": "u8" } }
      ]
    });
    let nested = raw_descriptor(
      "Tests.NestedExpansion",
      serde_json::json!({
        "size": 256 * 257,
        "alignment": 1,
        "fields": [
          {
            "name": "items",
            "offset": 0,
            "count": 257,
            "type": {
              "kind": "struct",
              "name": "Tests.NestedExpansionItem",
              "layout": nested_layout
            }
          }
        ]
      }),
    );
    let error = DynComRawStructLayout::from_descriptor(nested)
      .err()
      .unwrap();
    assert!(error.reason.contains("fixed fields"));

    let fixed_count = raw_descriptor(
      "Tests.FixedCountExpansion",
      serde_json::json!({
        "size": RAW_MAX_FIXED_FIELD_EXPANSION + 1,
        "alignment": 1,
        "fields": [
          {
            "name": "bytes",
            "offset": 0,
            "count": RAW_MAX_FIXED_FIELD_EXPANSION + 1,
            "type": { "kind": "u8" }
          }
        ]
      }),
    );
    let error = DynComRawStructLayout::from_descriptor(fixed_count)
      .err()
      .unwrap();
    assert!(error.reason.contains("field count"));

    let oversized_union = raw_descriptor(
      "Tests.OversizedUnion",
      serde_json::json!({
        "size": RAW_MAX_AGGREGATE_BYTE_SIZE + 1,
        "alignment": 1,
        "fields": [
          { "name": "value", "count": 1, "type": { "kind": "u8" } }
        ]
      }),
    );
    assert!(DynComRawUnionLayout::from_descriptor(oversized_union).is_err());

    let union = DynComRawUnionLayout::from_descriptor(RAW_UNION_DESCRIPTOR.into()).unwrap();
    let value = union.create_value("integer".into(), None).unwrap();
    assert_eq!(union.read_value_bytes(&value).unwrap().as_ref(), &[0; 8]);
    assert!(union
      .create_value("integer".into(), Some(Buffer::from(vec![0; 7])))
      .is_err());
  }

  #[test]
  fn bounded_external_memory_reuses_checks_without_owning_storage() {
    let mut backing = ExternalBlock([0; 32]);
    let pointer = DynComRawPointer::from_address(Either::A(BigInt::from(
      backing.0.as_mut_ptr().expose_provenance() as u64,
    )))
    .unwrap();
    let memory = DynComRawMemory::from_unsafe_pointer(&pointer, number(32), number(16)).unwrap();
    memory
      .write_u32(number(4), f64::from(0x1234_5678u32))
      .unwrap();
    assert_eq!(memory.read_u32(number(4)).unwrap(), 0x1234_5678);
    assert_eq!(&backing.0[4..8], &0x1234_5678u32.to_ne_bytes());
    let child = memory.pointer(Some(number(8))).unwrap();

    memory.release().unwrap();
    memory.release().unwrap();
    assert!(memory.released().unwrap());
    assert!(!memory.allocation.deallocated());
    assert!(child.address().is_err());
    backing.0[0] = 0xA5;
    assert_eq!(backing.0[0], 0xA5);
  }

  #[test]
  fn bounded_external_memory_rejects_invalid_ranges_and_alignment() {
    assert!(DynComRawMemory::from_unsafe_address(number(0), number(1), number(1)).is_err());
    assert!(DynComRawMemory::from_unsafe_address(number(0), number(0), number(1)).is_ok());
    assert!(DynComRawMemory::from_unsafe_address(number(0), number(0), number(3)).is_err());

    let mut backing = ExternalBlock([0; 32]);
    let address = backing.0.as_mut_ptr().expose_provenance();
    assert!(DynComRawMemory::from_unsafe_address(
      Either::A(BigInt::from((address + 1) as u64)),
      number(8),
      number(8),
    )
    .is_err());
    assert!(DynComRawMemory::from_unsafe_address(
      Either::A(BigInt::from((usize::MAX - 3) as u64)),
      number(8),
      number(1),
    )
    .is_err());
    assert!(DynComRawMemory::from_unsafe_address(
      Either::A(BigInt::from(address as u64)),
      Either::A(BigInt::from(usize::MAX as u64)),
      number(1),
    )
    .is_err());

    let owned = raw_memory(16, 8);
    let pointer = owned.pointer(Some(number(8))).unwrap();
    assert!(DynComRawMemory::from_unsafe_pointer(&pointer, number(9), number(1)).is_err());
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  struct WaveFormatLike {
    format_tag: u16,
    channels: u16,
    samples_per_sec: u32,
    avg_bytes_per_sec: u32,
    block_align: u16,
    bits_per_sample: u16,
    extra_size: u16,
  }

  #[test]
  fn raw_cleanup_frees_standard_memory_and_automation_resources_once() {
    let wave = WaveFormatLike {
      format_tag: 1,
      channels: 2,
      samples_per_sec: 48_000,
      avg_bytes_per_sec: 192_000,
      block_align: 4,
      bits_per_sample: 16,
      extra_size: 0,
    };
    let co_task =
      unsafe { windows::Win32::System::Com::CoTaskMemAlloc(std::mem::size_of::<WaveFormatLike>()) };
    assert!(!co_task.is_null());
    unsafe {
      std::ptr::copy_nonoverlapping(
        (&wave as *const WaveFormatLike).cast::<u8>(),
        co_task.cast(),
        std::mem::size_of::<WaveFormatLike>(),
      );
    }
    let mut co_task_pointer = external_pointer(co_task as usize);
    DynComRawCleanup::co_task_mem_free(&mut co_task_pointer).unwrap();
    assert!(DynComRawCleanup::co_task_mem_free(&mut co_task_pointer).is_err());

    let local = unsafe {
      windows::Win32::System::Memory::LocalAlloc(windows::Win32::System::Memory::LMEM_FIXED, 32)
    }
    .unwrap();
    let mut local_pointer = external_pointer(local.0 as usize);
    DynComRawCleanup::local_free(&mut local_pointer).unwrap();
    assert!(DynComRawCleanup::local_free(&mut local_pointer).is_err());

    let global = unsafe {
      windows::Win32::System::Memory::GlobalAlloc(windows::Win32::System::Memory::GMEM_FIXED, 32)
    }
    .unwrap();
    let mut global_pointer = external_pointer(global.0 as usize);
    DynComRawCleanup::global_free(&mut global_pointer).unwrap();
    assert!(DynComRawCleanup::global_free(&mut global_pointer).is_err());

    let bstr = windows::core::BSTR::from("raw cleanup");
    let mut bstr_pointer = external_pointer(bstr.as_ptr() as usize);
    std::mem::forget(bstr);
    DynComRawCleanup::sys_free_string(&mut bstr_pointer).unwrap();
    assert!(DynComRawCleanup::sys_free_string(&mut bstr_pointer).is_err());

    let safe_array = unsafe {
      windows::Win32::System::Ole::SafeArrayCreateVector(
        windows::Win32::System::Variant::VT_UI1,
        0,
        4,
      )
    };
    assert!(!safe_array.is_null());
    let mut safe_array_pointer = external_pointer(safe_array as usize);
    DynComRawCleanup::safe_array_destroy(&mut safe_array_pointer).unwrap();
    assert!(DynComRawCleanup::safe_array_destroy(&mut safe_array_pointer).is_err());

    let variant = raw_memory(
      std::mem::size_of::<windows::Win32::System::Variant::VARIANT>(),
      std::mem::align_of::<windows::Win32::System::Variant::VARIANT>(),
    );
    DynComRawCleanup::variant_clear(&variant, None).unwrap();
    DynComRawCleanup::variant_clear(&variant, None).unwrap();

    let prop_variant = raw_memory(
      std::mem::size_of::<windows::Win32::System::Com::StructuredStorage::PROPVARIANT>(),
      std::mem::align_of::<windows::Win32::System::Com::StructuredStorage::PROPVARIANT>(),
    );
    DynComRawCleanup::prop_variant_clear(&prop_variant, None).unwrap();
    DynComRawCleanup::prop_variant_clear(&prop_variant, None).unwrap();
  }

  #[test]
  fn raw_cleanup_releases_stgmedium_delegated_owner_and_zeros_storage() {
    let mut object = RawTrackedComObject {
      vtable: &RAW_TRACKED_VTABLE,
      addrefs: AtomicU32::new(0),
      releases: AtomicU32::new(0),
    };
    let mut managed = raw_tracked_managed(&mut object);
    unsafe {
      raw_tracked_add_ref((&mut object as *mut RawTrackedComObject).cast());
    }
    let delegated = unsafe { IUnknown::from_raw((&mut object as *mut RawTrackedComObject).cast()) };
    let medium = windows::Win32::System::Com::STGMEDIUM {
      tymed: 0,
      u: unsafe { std::mem::zeroed() },
      pUnkForRelease: std::mem::ManuallyDrop::new(Some(delegated)),
    };
    let bytes = unsafe {
      std::slice::from_raw_parts(
        (&medium as *const windows::Win32::System::Com::STGMEDIUM).cast::<u8>(),
        std::mem::size_of::<windows::Win32::System::Com::STGMEDIUM>(),
      )
    };
    let memory = raw_memory(
      bytes.len(),
      std::mem::align_of::<windows::Win32::System::Com::STGMEDIUM>(),
    );
    memory
      .write_bytes(number(0), Buffer::from(bytes.to_vec()))
      .unwrap();
    DynComRawCleanup::release_stg_medium(&memory, None).unwrap();
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 1);
    assert_eq!(
      memory
        .read_bytes(number(0), number(bytes.len()))
        .unwrap()
        .as_ref(),
      vec![0; bytes.len()]
    );
    managed.release().unwrap();
    assert_eq!(object.releases.load(AtomicOrdering::Relaxed), 2);
  }

  #[test]
  fn raw_cleanup_reports_handle_failures_and_consumes_successes() {
    let event =
      unsafe { windows::Win32::System::Threading::CreateEventW(None, true, false, None) }.unwrap();
    let mut event_pointer = external_pointer(event.0 as usize);
    DynComRawCleanup::close_handle(&mut event_pointer).unwrap();
    assert!(DynComRawCleanup::close_handle(&mut event_pointer).is_err());
    let mut invalid_handle = DynComRawPointer::null();
    assert!(DynComRawCleanup::close_handle(&mut invalid_handle).is_err());
    assert!(invalid_handle.address().is_ok());

    let and_mask = [0xffu8; 32];
    let xor_mask = [0u8; 32];
    let icon = unsafe {
      windows::Win32::UI::WindowsAndMessaging::CreateIcon(
        None,
        16,
        16,
        1,
        1,
        and_mask.as_ptr(),
        xor_mask.as_ptr(),
      )
    }
    .unwrap();
    let mut icon_pointer = external_pointer(icon.0 as usize);
    DynComRawCleanup::destroy_icon(&mut icon_pointer).unwrap();
    assert!(DynComRawCleanup::destroy_icon(&mut icon_pointer).is_err());

    let brush = unsafe {
      windows::Win32::Graphics::Gdi::CreateSolidBrush(windows::Win32::Foundation::COLORREF(
        0x0000ff,
      ))
    };
    assert!(!brush.is_invalid());
    let mut brush_pointer = external_pointer(brush.0 as usize);
    DynComRawCleanup::delete_object(&mut brush_pointer).unwrap();
    assert!(DynComRawCleanup::delete_object(&mut brush_pointer).is_err());
  }

  #[test]
  fn primitive_and_byte_access_round_trips_native_endian_values() {
    let memory = raw_memory(128, 16);
    memory.write_i8(number(0), -128.0).unwrap();
    memory.write_u8(number(1), 255.0).unwrap();
    memory.write_i16(number(2), -12_345.0).unwrap();
    memory.write_u16(number(4), 54_321.0).unwrap();
    memory.write_i32(number(6), -1_234_567.0).unwrap();
    memory.write_u32(number(10), 4_000_000_000.0).unwrap();
    memory
      .write_i64(number(14), BigInt::from(i64::MIN + 7))
      .unwrap();
    memory
      .write_u64(number(22), BigInt::from(u64::MAX - 7))
      .unwrap();
    memory.write_f32(number(30), 1.25).unwrap();
    memory.write_f64(number(34), -9.5).unwrap();

    assert_eq!(memory.read_i8(number(0)).unwrap(), -128);
    assert_eq!(memory.read_u8(number(1)).unwrap(), 255);
    assert_eq!(memory.read_i16(number(2)).unwrap(), -12_345);
    assert_eq!(memory.read_u16(number(4)).unwrap(), 54_321);
    assert_eq!(memory.read_i32(number(6)).unwrap(), -1_234_567);
    assert_eq!(memory.read_u32(number(10)).unwrap(), 4_000_000_000);
    assert_eq!(
      memory.read_i64(number(14)).unwrap().get_i64(),
      (i64::MIN + 7, true)
    );
    assert_eq!(
      memory.read_u64(number(22)).unwrap().get_u64(),
      (false, u64::MAX - 7, true)
    );
    assert_eq!(memory.read_f32(number(30)).unwrap(), 1.25);
    assert_eq!(memory.read_f64(number(34)).unwrap(), -9.5);

    memory
      .write_bytes(number(48), Buffer::from(vec![1, 2, 3, 4]))
      .unwrap();
    assert_eq!(
      memory.read_bytes(number(48), number(4)).unwrap().as_ref(),
      &[1, 2, 3, 4]
    );
  }

  #[test]
  fn pointer_width_and_pointer_slots_round_trip_without_transferring_ownership() {
    let memory = raw_memory(
      4 * std::mem::size_of::<usize>(),
      std::mem::align_of::<usize>(),
    );
    memory.write_isize(number(0), BigInt::from(-1i64)).unwrap();
    memory
      .write_usize(
        number(std::mem::size_of::<usize>()),
        BigInt::from(usize::MAX as u64),
      )
      .unwrap();
    let external = DynComRawPointer::from_address(Either::A(BigInt::from(0x1234u64))).unwrap();
    memory
      .write_pointer(number(2 * std::mem::size_of::<usize>()), &external)
      .unwrap();

    assert_eq!(memory.read_isize(number(0)).unwrap().get_i64(), (-1, true));
    assert_eq!(
      memory
        .read_usize(number(std::mem::size_of::<usize>()))
        .unwrap()
        .get_u64(),
      (false, usize::MAX as u64, true)
    );
    let read = memory
      .read_pointer(number(2 * std::mem::size_of::<usize>()))
      .unwrap();
    assert_eq!(read.address_bits().unwrap(), 0x1234);
    assert!(matches!(read.kind, RawPointerKind::External(_)));
  }

  #[test]
  fn offsets_bounds_overflow_and_integer_precision_are_checked() {
    let memory = raw_memory(16, 8);
    assert!(memory.pointer(Some(number(16))).is_ok());
    assert!(memory.pointer(Some(number(17))).is_err());
    assert!(memory.read_u32(number(13)).is_err());
    assert!(memory
      .read_bytes(Either::A(BigInt::from(usize::MAX as u64)), number(2))
      .is_err());
    assert!(DynComRawMemory::allocate(Either::B(1.5), None).is_err());
    assert!(DynComRawMemory::allocate(Either::B(MAX_SAFE_INTEGER + 1.0), None).is_err());
    assert!(memory.write_u8(number(0), 256.0).is_err());
    assert!(memory.write_i32(number(0), 1.5).is_err());

    let pointer = memory.pointer(Some(number(8))).unwrap();
    assert!(pointer.offset(number(8)).is_ok());
    assert!(pointer.offset(number(9)).is_err());
  }

  #[test]
  fn owned_call_values_retain_allocation_and_fail_after_release() {
    let memory = raw_memory(16, 8);
    let weak = Arc::downgrade(&memory.allocation);
    let pointer = memory.pointer(None).unwrap();
    let value = pointer.to_value().unwrap();
    drop(pointer);
    drop(memory);

    let retained = weak.upgrade().expect("call value retains allocation");
    retained.validate_live().unwrap();
    drop(retained);
    assert!(weak.upgrade().is_some());
    drop(value);
    assert!(weak.upgrade().is_none());

    let memory = raw_memory(16, 8);
    let pointer = memory.pointer(None).unwrap();
    let value = pointer.to_value().unwrap();
    memory.release().unwrap();
    assert!(memory.allocation.deallocated());
    assert!(pointer.address().is_err());
    assert!(value.1.as_ref().unwrap().validate().is_err());
  }

  #[test]
  fn raw_pointer_values_are_borrowed_and_cannot_be_adopted() {
    let memory = raw_memory(16, 8);
    let mut owned = memory.pointer(None).unwrap().to_value().unwrap();
    let error =
      take_native_output_pointer(&mut owned, PointerProvenance::ComOutput, "COM interface")
        .unwrap_err();
    assert!(error.reason.contains("owner-backed"));

    let mut external = DynComRawPointer::from_address(Either::A(BigInt::from(0x1234u64)))
      .unwrap()
      .to_value()
      .unwrap();
    let error =
      take_native_output_pointer(&mut external, PointerProvenance::ComOutput, "COM interface")
        .unwrap_err();
    assert!(error.reason.contains("Borrowed"));
  }

  #[test]
  fn external_and_null_pointers_are_pass_through_only() {
    let external = DynComRawPointer::from_address(Either::A(BigInt::from(0x1234u64))).unwrap();
    assert_eq!(external.address_bits().unwrap(), 0x1234);
    assert!(!external.is_null().unwrap());
    assert!(external.offset(number(1)).is_err());

    let null = DynComRawPointer::null();
    assert_eq!(null.address_bits().unwrap(), 0);
    assert!(null.is_null().unwrap());
    assert!(matches!(
      null.to_value().unwrap().0,
      dynwinrt::WinRTValue::RawPtr(pointer) if pointer.is_null()
    ));
  }

  #[test]
  fn raw_memory_operations_are_bound_to_the_creating_thread() {
    let allocation = RawAllocation::allocate(8, 8).unwrap();
    std::thread::spawn(move || allocation.read_bytes(0, 1).unwrap_err())
      .join()
      .unwrap();
  }

  #[test]
  fn bounded_external_memory_is_bound_to_the_creating_thread() {
    let mut backing = ExternalBlock([0; 32]);
    let allocation = RawAllocation::external(
      backing.0.as_mut_ptr().expose_provenance(),
      backing.0.len(),
      16,
      "test external view",
    )
    .unwrap();
    std::thread::spawn(move || allocation.read_bytes(0, 1).unwrap_err())
      .join()
      .unwrap();
  }

  #[repr(C)]
  struct ReentrantReleaseComObject {
    vtable: *const *mut c_void,
    allocation: Arc<RawAllocation>,
    release_succeeded: bool,
    logical_release_visible: bool,
    physical_deallocation_deferred: bool,
    post_release_access_failed: bool,
    pointer_mutated_after_release: bool,
  }

  unsafe extern "system" fn release_then_mutate_pointer(
    this: *mut c_void,
    value: *mut u32,
  ) -> HRESULT {
    let object = unsafe { &mut *this.cast::<ReentrantReleaseComObject>() };
    object.release_succeeded = object.allocation.release().is_ok();
    object.logical_release_visible = matches!(object.allocation.released(), Ok(true));
    object.physical_deallocation_deferred = !object.allocation.deallocated();
    object.post_release_access_failed = object.allocation.read_bytes(0, 1).is_err();
    if object.physical_deallocation_deferred && !value.is_null() {
      let updated = unsafe { value.read_unaligned() }.wrapping_add(1);
      unsafe {
        value.write_unaligned(updated);
      }
      object.pointer_mutated_after_release = unsafe { value.read_unaligned() } == updated;
    }
    HRESULT(0)
  }

  #[test]
  fn invocation_lease_defers_reentrant_release_deallocation() {
    let table = dynwinrt::MetadataTable::new();
    let interface = dynwinrt::com::register_interface(
      &table,
      "Tests.IReentrantRawRelease",
      windows::core::GUID::from_u128(0x11111111_2222_3333_4444_555555555555),
      dynwinrt::com::InterfaceBase::IUnknown,
    )
    .add_method_at(
      3,
      "ReleaseThenMutate",
      dynwinrt::com::MethodSignature::new(&table).add_in(dynwinrt::com::Type::pointer()),
    )
    .unwrap();
    let mut vtable = [std::ptr::null_mut(); 4];
    vtable[3] = release_then_mutate_pointer as *mut c_void;
    let memory = raw_memory(std::mem::size_of::<u32>(), std::mem::align_of::<u32>());
    memory.write_u32(number(0), 41.0).unwrap();
    let mut object = ReentrantReleaseComObject {
      vtable: vtable.as_ptr(),
      allocation: memory.allocation.clone(),
      release_succeeded: false,
      logical_release_visible: false,
      physical_deallocation_deferred: false,
      post_release_access_failed: false,
      pointer_mutated_after_release: false,
    };
    let pointer = memory.pointer(None).unwrap();
    let value = pointer.to_value().unwrap();

    with_com_invocation_args(&[&value], |args| {
      unsafe {
        interface
          .method(3)
          .unwrap()
          .invoke_values_with_output_kinds(
            (&mut object as *mut ReentrantReleaseComObject).cast(),
            args,
          )
      }
      .map(|_| ())
      .map_err(|error| napi::Error::from_reason(error.message()))
    })
    .unwrap();

    assert!(object.release_succeeded);
    assert!(object.logical_release_visible);
    assert!(object.physical_deallocation_deferred);
    assert!(object.post_release_access_failed);
    assert!(object.pointer_mutated_after_release);
    assert!(memory.released().unwrap());
    assert!(memory.allocation.deallocated());
    assert!(pointer.address().is_err());
    assert!(value.1.as_ref().unwrap().validate().is_err());
  }

  #[test]
  fn external_view_invocation_mutates_caller_storage_without_deallocation() {
    let table = dynwinrt::MetadataTable::new();
    let interface = dynwinrt::com::register_interface(
      &table,
      "Tests.IExternalRawView",
      windows::core::GUID::from_u128(0xaaaaaaaa_bbbb_cccc_dddd_eeeeeeeeeeee),
      dynwinrt::com::InterfaceBase::IUnknown,
    )
    .add_method_at(
      3,
      "ReleaseThenMutate",
      dynwinrt::com::MethodSignature::new(&table).add_in(dynwinrt::com::Type::pointer()),
    )
    .unwrap();
    let mut vtable = [std::ptr::null_mut(); 4];
    vtable[3] = release_then_mutate_pointer as *mut c_void;
    let mut backing = ExternalBlock([0; 32]);
    backing.0[0..4].copy_from_slice(&41u32.to_ne_bytes());
    let memory = DynComRawMemory::from_unsafe_address(
      Either::A(BigInt::from(
        backing.0.as_mut_ptr().expose_provenance() as u64
      )),
      number(backing.0.len()),
      number(16),
    )
    .unwrap();
    let mut object = ReentrantReleaseComObject {
      vtable: vtable.as_ptr(),
      allocation: memory.allocation.clone(),
      release_succeeded: false,
      logical_release_visible: false,
      physical_deallocation_deferred: false,
      post_release_access_failed: false,
      pointer_mutated_after_release: false,
    };
    let pointer = memory.pointer(None).unwrap();
    let value = pointer.to_value().unwrap();

    with_com_invocation_args(&[&value], |args| {
      unsafe {
        interface
          .method(3)
          .unwrap()
          .invoke_values_with_output_kinds(
            (&mut object as *mut ReentrantReleaseComObject).cast(),
            args,
          )
      }
      .map(|_| ())
      .map_err(|error| napi::Error::from_reason(error.message()))
    })
    .unwrap();

    assert!(object.release_succeeded);
    assert!(object.logical_release_visible);
    assert!(object.physical_deallocation_deferred);
    assert!(object.post_release_access_failed);
    assert!(object.pointer_mutated_after_release);
    assert_eq!(u32::from_ne_bytes(backing.0[0..4].try_into().unwrap()), 42);
    assert!(!memory.allocation.deallocated());
    assert!(pointer.address().is_err());
  }

  #[test]
  fn invocation_leases_drop_on_error_and_panic_paths() {
    let memory = raw_memory(8, 8);
    let value = memory.pointer(None).unwrap().to_value().unwrap();
    let error = with_com_invocation_args(&[&value], |_| {
      memory.release()?;
      assert!(!memory.allocation.deallocated());
      Err::<(), _>(napi::Error::from_reason("expected invocation failure"))
    })
    .unwrap_err();
    assert!(error.reason.contains("expected invocation failure"));
    assert!(memory.allocation.deallocated());

    let memory = raw_memory(8, 8);
    let value = memory.pointer(None).unwrap().to_value().unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
      let _: napi::Result<()> = with_com_invocation_args(&[&value], |_| {
        memory.release().unwrap();
        assert!(!memory.allocation.deallocated());
        panic!("expected invocation panic");
      });
    }));
    assert!(panic.is_err());
    assert!(memory.allocation.deallocated());
  }
}
