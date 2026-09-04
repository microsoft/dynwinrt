// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use core::ffi::c_void;
use libffi::middle::{Arg, arg};
use windows_core::{HRESULT, Interface};

use crate::{
    abi::{AbiType, AbiValue},
    native_call::{MethodReturn, NativeCallValue, Parameter},
    value::WinRTValue,
};

struct NativeStructStorage {
    ptr: std::ptr::NonNull<u8>,
    allocation_ptr: std::ptr::NonNull<u8>,
    allocation: std::alloc::Layout,
    size: usize,
}

struct NativeUnionStorage {
    ptr: std::ptr::NonNull<u8>,
    allocation_ptr: std::ptr::NonNull<u8>,
    allocation: std::alloc::Layout,
    size: usize,
}

#[cfg(test)]
const NATIVE_AGGREGATE_GUARD_SIZE: usize = 16;
#[cfg(not(test))]
const NATIVE_AGGREGATE_GUARD_SIZE: usize = 0;
#[cfg(test)]
const NATIVE_AGGREGATE_PREFIX_CANARY: u8 = 0xA5;
#[cfg(test)]
const NATIVE_AGGREGATE_SUFFIX_CANARY: u8 = 0x5A;

struct BstrCallValue {
    raw: *const u16,
}

impl BstrCallValue {
    fn new(value: Option<&str>) -> windows_core::Result<Self> {
        let Some(value) = value else {
            return Ok(Self {
                raw: std::ptr::null_mut(),
            });
        };
        let utf16 = value.encode_utf16().collect::<Vec<_>>();
        let value =
            unsafe { windows::Win32::Foundation::SysAllocStringLen(Some(utf16.as_slice())) };
        let raw = value.into_raw();
        if raw.is_null() {
            return Err(windows_core::Error::from_hresult(windows_core::HRESULT(
                0x8007000Eu32 as i32,
            )));
        }
        #[cfg(test)]
        if !raw.is_null() {
            BSTR_TEST_ALLOCS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(Self { raw })
    }

    fn as_raw(&self) -> *const u16 {
        self.raw
    }

    fn as_mut_ptr(&mut self) -> *mut *const u16 {
        &mut self.raw
    }

    fn take(&mut self) -> *const u16 {
        std::mem::replace(&mut self.raw, std::ptr::null())
    }
}

impl Drop for BstrCallValue {
    fn drop(&mut self) {
        if self.raw.is_null() {
            return;
        }
        #[cfg(test)]
        BSTR_TEST_FREES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        drop(unsafe { windows_core::BSTR::from_raw(self.take()) });
    }
}

#[cfg(test)]
static BSTR_TEST_ALLOCS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
static BSTR_TEST_FREES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
pub(crate) fn reset_bstr_test_counts() {
    BSTR_TEST_ALLOCS.store(0, std::sync::atomic::Ordering::Relaxed);
    BSTR_TEST_FREES.store(0, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
pub(crate) fn bstr_test_counts() -> (usize, usize) {
    (
        BSTR_TEST_ALLOCS.load(std::sync::atomic::Ordering::Relaxed),
        BSTR_TEST_FREES.load(std::sync::atomic::Ordering::Relaxed),
    )
}

impl NativeUnionStorage {
    fn zeroed(layout: &crate::com::NativeUnionLayout) -> windows_core::Result<Self> {
        #[cfg(target_arch = "x86_64")]
        let alignment = layout.alignment().max(16);
        #[cfg(not(target_arch = "x86_64"))]
        let alignment = layout.alignment();
        let allocation_size = layout
            .size()
            .checked_add(NATIVE_AGGREGATE_GUARD_SIZE.saturating_mul(2))
            .ok_or_else(|| {
                windows_core::Error::new(
                    windows_core::HRESULT(0x80070057u32 as i32),
                    "invalid native union allocation size",
                )
            })?;
        let allocation =
            std::alloc::Layout::from_size_align(allocation_size, alignment).map_err(|_| {
                windows_core::Error::new(
                    windows_core::HRESULT(0x80070057u32 as i32),
                    "invalid native union allocation layout",
                )
            })?;
        let allocation_ptr = unsafe { std::alloc::alloc_zeroed(allocation) };
        let allocation_ptr = std::ptr::NonNull::new(allocation_ptr).ok_or_else(|| {
            windows_core::Error::new(
                windows_core::HRESULT(0x8007000Eu32 as i32),
                "native union allocation failed",
            )
        })?;
        let ptr = unsafe {
            std::ptr::NonNull::new_unchecked(
                allocation_ptr.as_ptr().add(NATIVE_AGGREGATE_GUARD_SIZE),
            )
        };
        let storage = Self {
            ptr,
            allocation_ptr,
            allocation,
            size: layout.size(),
        };
        storage.initialize_canaries();
        Ok(storage)
    }

    fn from_value(value: &crate::com::NativeUnionValue) -> windows_core::Result<Self> {
        let layout = value.layout();
        let storage = Self::zeroed(layout)?;
        unsafe {
            std::ptr::copy_nonoverlapping(
                value.bytes().as_ptr(),
                storage.ptr.as_ptr(),
                value.bytes().len(),
            );
        }
        Ok(storage)
    }

    fn as_ptr(&self) -> *const u8 {
        self.ptr.as_ptr()
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.size) }
    }

    fn as_ret(&mut self) -> libffi::middle::Ret<'_> {
        let bytes = unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.size) };
        libffi::middle::Ret::new(bytes)
    }

    fn to_vec(&self) -> Vec<u8> {
        self.as_slice().to_vec()
    }

    fn initialize_canaries(&self) {
        #[cfg(test)]
        unsafe {
            std::ptr::write_bytes(
                self.allocation_ptr.as_ptr(),
                NATIVE_AGGREGATE_PREFIX_CANARY,
                NATIVE_AGGREGATE_GUARD_SIZE,
            );
            std::ptr::write_bytes(
                self.ptr.as_ptr().add(self.size),
                NATIVE_AGGREGATE_SUFFIX_CANARY,
                NATIVE_AGGREGATE_GUARD_SIZE,
            );
        }
    }

    fn validate_canaries(&self) -> windows_core::Result<()> {
        #[cfg(test)]
        {
            let prefix = unsafe {
                std::slice::from_raw_parts(
                    self.allocation_ptr.as_ptr(),
                    NATIVE_AGGREGATE_GUARD_SIZE,
                )
            };
            let suffix = unsafe {
                std::slice::from_raw_parts(
                    self.ptr.as_ptr().add(self.size),
                    NATIVE_AGGREGATE_GUARD_SIZE,
                )
            };
            if prefix
                .iter()
                .any(|byte| *byte != NATIVE_AGGREGATE_PREFIX_CANARY)
                || suffix
                    .iter()
                    .any(|byte| *byte != NATIVE_AGGREGATE_SUFFIX_CANARY)
            {
                return Err(windows_core::Error::new(
                    windows_core::HRESULT(0x80004005u32 as i32),
                    "native union return storage canary was modified",
                ));
            }
        }
        Ok(())
    }
}

impl Drop for NativeUnionStorage {
    fn drop(&mut self) {
        unsafe { std::alloc::dealloc(self.allocation_ptr.as_ptr(), self.allocation) };
    }
}

impl NativeStructStorage {
    fn zeroed(layout: &crate::com::NativeStructLayout) -> windows_core::Result<Self> {
        #[cfg(target_arch = "x86_64")]
        let alignment = layout.alignment().max(16);
        #[cfg(not(target_arch = "x86_64"))]
        let alignment = layout.alignment();
        let allocation_size = layout
            .size()
            .checked_add(NATIVE_AGGREGATE_GUARD_SIZE.saturating_mul(2))
            .ok_or_else(|| {
                windows_core::Error::new(
                    windows_core::HRESULT(0x80070057u32 as i32),
                    "invalid native struct allocation size",
                )
            })?;
        let allocation =
            std::alloc::Layout::from_size_align(allocation_size, alignment).map_err(|_| {
                windows_core::Error::new(
                    windows_core::HRESULT(0x80070057u32 as i32),
                    "invalid native struct allocation layout",
                )
            })?;
        let allocation_ptr = unsafe { std::alloc::alloc_zeroed(allocation) };
        let allocation_ptr = std::ptr::NonNull::new(allocation_ptr).ok_or_else(|| {
            windows_core::Error::new(
                windows_core::HRESULT(0x8007000Eu32 as i32),
                "native struct allocation failed",
            )
        })?;
        let ptr = unsafe {
            std::ptr::NonNull::new_unchecked(
                allocation_ptr.as_ptr().add(NATIVE_AGGREGATE_GUARD_SIZE),
            )
        };
        let storage = Self {
            ptr,
            allocation_ptr,
            allocation,
            size: layout.size(),
        };
        storage.initialize_canaries();
        Ok(storage)
    }

    fn from_bytes(
        layout: &crate::com::NativeStructLayout,
        bytes: &[u8],
    ) -> windows_core::Result<Self> {
        if bytes.len() != layout.size() {
            return Err(windows_core::Error::new(
                windows_core::HRESULT(0x80070057u32 as i32),
                "native struct byte length does not match its layout",
            ));
        }
        let storage = Self::zeroed(layout)?;
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), storage.ptr.as_ptr(), bytes.len());
        }
        Ok(storage)
    }

    fn as_ptr(&self) -> *const u8 {
        self.ptr.as_ptr()
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    fn as_slice(&self) -> &[u8] {
        // Safety: this storage uniquely owns an initialized allocation of
        // exactly `allocation.size()` bytes.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.size) }
    }

    fn as_ret(&mut self) -> libffi::middle::Ret<'_> {
        // Safety: this storage uniquely owns an allocation of exactly
        // `allocation.size()` initialized bytes for the duration of the call.
        let bytes = unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.size) };
        libffi::middle::Ret::new(bytes)
    }

    fn to_vec(&self) -> Vec<u8> {
        self.as_slice().to_vec()
    }

    fn initialize_canaries(&self) {
        #[cfg(test)]
        unsafe {
            std::ptr::write_bytes(
                self.allocation_ptr.as_ptr(),
                NATIVE_AGGREGATE_PREFIX_CANARY,
                NATIVE_AGGREGATE_GUARD_SIZE,
            );
            std::ptr::write_bytes(
                self.ptr.as_ptr().add(self.size),
                NATIVE_AGGREGATE_SUFFIX_CANARY,
                NATIVE_AGGREGATE_GUARD_SIZE,
            );
        }
    }

    fn validate_canaries(&self) -> windows_core::Result<()> {
        #[cfg(test)]
        {
            let prefix = unsafe {
                std::slice::from_raw_parts(
                    self.allocation_ptr.as_ptr(),
                    NATIVE_AGGREGATE_GUARD_SIZE,
                )
            };
            let suffix = unsafe {
                std::slice::from_raw_parts(
                    self.ptr.as_ptr().add(self.size),
                    NATIVE_AGGREGATE_GUARD_SIZE,
                )
            };
            if prefix
                .iter()
                .any(|byte| *byte != NATIVE_AGGREGATE_PREFIX_CANARY)
                || suffix
                    .iter()
                    .any(|byte| *byte != NATIVE_AGGREGATE_SUFFIX_CANARY)
            {
                return Err(windows_core::Error::new(
                    windows_core::HRESULT(0x80004005u32 as i32),
                    "native struct return storage canary was modified",
                ));
            }
        }
        Ok(())
    }
}

impl Drop for NativeStructStorage {
    fn drop(&mut self) {
        unsafe { std::alloc::dealloc(self.allocation_ptr.as_ptr(), self.allocation) };
    }
}

pub(crate) trait ArgumentList {
    fn get_value(&self, index: usize) -> &WinRTValue;

    fn get_bstr(&self, _index: usize) -> Option<&crate::com::BstrValue> {
        None
    }

    fn get_native_struct(&self, _index: usize) -> Option<&crate::com::NativeStructValue> {
        None
    }

    fn get_native_union(&self, _index: usize) -> Option<&crate::com::NativeUnionValue> {
        None
    }

    fn get_variant(&self, _index: usize) -> Option<&crate::com::VariantValue> {
        None
    }

    fn get_safe_array(&self, _index: usize) -> Option<&crate::com::SafeArrayValue> {
        None
    }

    fn get_prop_variant(&self, _index: usize) -> Option<&crate::com::PropVariantValue> {
        None
    }

    fn get_dispatch_params(&self, _index: usize) -> Option<&crate::com::DispatchParamsValue> {
        None
    }

    fn get_format_etc(&self, _index: usize) -> Option<&crate::com::FormatEtcValue> {
        None
    }

    fn get_stg_medium(&self, _index: usize) -> Option<&crate::com::StgMediumValue> {
        None
    }
}

impl ArgumentList for [WinRTValue] {
    fn get_value(&self, index: usize) -> &WinRTValue {
        &self[index]
    }
}

pub fn get_vtable_function_ptr(obj: *mut c_void, method_index: usize) -> *mut c_void {
    unsafe {
        let vtable_ptr = *(obj as *const *const *mut c_void);
        *vtable_ptr.add(method_index)
    }
}

pub fn call_winrt_method_0(vtable_index: usize, obj: *mut c_void) -> HRESULT {
    let method_ptr = get_vtable_function_ptr(obj, vtable_index);
    unsafe {
        let method: extern "system" fn(*mut c_void) -> HRESULT = std::mem::transmute(method_ptr);
        method(obj)
    }
}

pub fn call_winrt_method_1<T1>(vtable_index: usize, obj: *mut c_void, x1: T1) -> HRESULT {
    let method_ptr = get_vtable_function_ptr(obj, vtable_index);
    unsafe {
        let method: extern "system" fn(*mut c_void, T1) -> HRESULT =
            std::mem::transmute(method_ptr);
        method(obj, x1)
    }
}

pub fn call_winrt_method_2<T1, T2>(
    vtable_index: usize,
    obj: *mut c_void,
    x1: T1,
    x2: T2,
) -> HRESULT {
    let method_ptr = get_vtable_function_ptr(obj, vtable_index);
    unsafe {
        let method: extern "system" fn(*mut c_void, T1, T2) -> HRESULT =
            std::mem::transmute(method_ptr);
        method(obj, x1, x2)
    }
}

/// Dispatch a scalar WinRTValue through a closure that receives the raw ABI value.
/// Used by direct call helpers to avoid repeating the same 14-branch match.
macro_rules! dispatch_scalar {
    ($in_val:expr, $call:expr) => {
        match $in_val {
            WinRTValue::Bool(v) => $call(*v),
            WinRTValue::I8(v) => $call(*v),
            WinRTValue::U8(v) => $call(*v),
            WinRTValue::I16(v) => $call(*v),
            WinRTValue::U16(v) => $call(*v),
            WinRTValue::I32(v) => $call(*v),
            WinRTValue::Enum { value: v, .. } => $call(*v),
            WinRTValue::U32(v) => $call(*v),
            WinRTValue::I64(v) => $call(*v),
            WinRTValue::U64(v) => $call(*v),
            WinRTValue::F32(v) => $call(*v),
            WinRTValue::F64(v) => $call(*v),
            WinRTValue::Object(o) => $call(o.as_raw()),
            WinRTValue::Null => $call(std::ptr::null_mut::<c_void>()),
            WinRTValue::RawPtr(p) => $call(*p),
            WinRTValue::Guid(g) => $call(*g),
            _ => panic!("dispatch_scalar: unsupported type {:?}", $in_val),
        }
    };
}

/// Direct call for 1-in + 0-out (setter).
pub fn call_1in(vtable_index: usize, obj: *mut c_void, in_val: &WinRTValue) -> HRESULT {
    dispatch_scalar!(in_val, |v| call_winrt_method_1(vtable_index, obj, v))
}

/// Direct call for 1-in + 1-out.
pub fn call_1in_1out(
    vtable_index: usize,
    obj: *mut c_void,
    in_val: &WinRTValue,
    out_ptr: *mut c_void,
) -> HRESULT {
    dispatch_scalar!(in_val, |v| call_winrt_method_2(
        vtable_index,
        obj,
        v,
        out_ptr
    ))
}

/// Direct call for 1 scalar in + FillArray out.
/// fn(this, val, u32 capacity, *mut u8 items) -> HRESULT
pub fn call_fill_array_1in(
    fptr: *mut c_void,
    obj: *mut c_void,
    in_val: &WinRTValue,
    capacity: u32,
    buffer: *mut u8,
) -> HRESULT {
    dispatch_scalar!(in_val, |v| unsafe {
        let method: unsafe extern "system" fn(*mut c_void, _, u32, *mut u8) -> HRESULT =
            std::mem::transmute(fptr);
        method(obj, v, capacity, buffer)
    })
}

use crate::metadata_table::TypeHandle;

/// Stable heap storage for array in-param data.
/// Owns the serialized byte buffer so it stays alive for the FFI call.
struct ArrayInSlot {
    length: u32,
    data_ptr: *const u8,
    _buffer: Vec<u8>, // keeps serialized bytes alive
}

/// Stable heap storage for array out-param data (callee writes into these fields).
struct ArrayOutSlot {
    length: u32,
    data_ptr: *mut c_void,
    element_type: TypeHandle,
}

impl Drop for ArrayOutSlot {
    fn drop(&mut self) {
        // Release elements + free callee-allocated buffer if ownership was not
        // transferred to ArrayData. Wrapping in ArrayData handles element-level
        // Release (HString, COM pointers, struct fields) before CoTaskMemFree.
        if !self.data_ptr.is_null() {
            let len = self.length as usize;
            if len > 0 {
                let _ = crate::array::ArrayData::from_cotaskmem(
                    self.element_type.clone(),
                    self.data_ptr,
                    len,
                );
            } else {
                unsafe {
                    windows::Win32::System::Com::CoTaskMemFree(Some(self.data_ptr));
                }
            }
            self.data_ptr = std::ptr::null_mut();
        }
    }
}

/// Stable heap storage for FillArray out-param data (caller-allocated via CoTaskMemAlloc).
struct FillArraySlot {
    capacity: u32,
    buffer_ptr: *mut u8, // CoTaskMemAlloc'd
    element_type: TypeHandle,
}

impl Drop for FillArraySlot {
    fn drop(&mut self) {
        // Release elements + free buffer if ownership was not transferred to ArrayData.
        // Buffer was zero-initialized before the call, so null slots are safe to release.
        // Use capacity as cleanup length — ArrayData::Drop skips null elements.
        if !self.buffer_ptr.is_null() {
            let _ = crate::array::ArrayData::from_cotaskmem(
                self.element_type.clone(),
                self.buffer_ptr as *mut c_void,
                self.capacity as usize,
            );
            self.buffer_ptr = std::ptr::null_mut();
        }
    }
}

fn input_abi_value(value: &WinRTValue) -> windows_core::Result<AbiValue> {
    let value = match value {
        WinRTValue::Bool(value) => AbiValue::Bool(u8::from(*value)),
        WinRTValue::I8(value) => AbiValue::I8(*value),
        WinRTValue::U8(value) => AbiValue::U8(*value),
        WinRTValue::I16(value) => AbiValue::I16(*value),
        WinRTValue::U16(value) => AbiValue::U16(*value),
        WinRTValue::I32(value) => AbiValue::I32(*value),
        WinRTValue::U32(value) => AbiValue::U32(*value),
        WinRTValue::I64(value) => AbiValue::I64(*value),
        WinRTValue::U64(value) => AbiValue::U64(*value),
        WinRTValue::F32(value) => AbiValue::F32(*value),
        WinRTValue::F64(value) => AbiValue::F64(*value),
        WinRTValue::HResult(value) => AbiValue::I32(value.0),
        WinRTValue::Enum { value, .. } => AbiValue::I32(*value),
        WinRTValue::RawPtr(value) => AbiValue::Pointer(*value),
        WinRTValue::Null => AbiValue::Pointer(std::ptr::null_mut()),
        _ => {
            return Err(windows_core::Error::new(
                windows_core::HRESULT(0x80070057u32 as i32),
                "unsupported in/out argument value",
            ));
        }
    };
    Ok(value)
}

fn cleanup_failed_outputs(parameters: &[Parameter], out_values: &mut [AbiValue]) {
    for parameter in parameters.iter().filter(|parameter| parameter.is_out()) {
        let Some(AbiValue::Pointer(ptr)) = out_values.get_mut(parameter.value_index) else {
            continue;
        };
        unsafe { parameter.cleanup_failed_pointer(*ptr) };
        *ptr = std::ptr::null_mut();
    }
}

pub(crate) struct CapturedHResultCall {
    pub(crate) hresult: HRESULT,
    pub(crate) outputs: Vec<Option<NativeCallValue>>,
    pub(crate) finalization_error: Option<windows_core::Error>,
}

enum DynamicCallOutcome {
    Values(Vec<NativeCallValue>),
    Captured(CapturedHResultCall),
}

pub(crate) fn call_method_dynamic<A, F>(
    vtable_index: usize,
    obj: *mut c_void,
    parameters: &[Parameter],
    args: &A,
    out_count: usize,
    return_kind: &MethodReturn,
    cif: &libffi::middle::Cif,
    mark_dispatched: F,
) -> windows_core::Result<Vec<NativeCallValue>>
where
    A: ArgumentList + ?Sized,
    F: FnOnce(),
{
    match call_method_dynamic_impl(
        vtable_index,
        obj,
        parameters,
        args,
        out_count,
        return_kind,
        cif,
        mark_dispatched,
    )? {
        DynamicCallOutcome::Values(values) => Ok(values),
        DynamicCallOutcome::Captured(_) => Err(windows_core::Error::new(
            windows_core::HRESULT(0x80070057u32 as i32),
            "captured HRESULT calls require the dedicated COM invocation path",
        )),
    }
}

pub(crate) fn call_method_dynamic_captured<A: ArgumentList + ?Sized>(
    vtable_index: usize,
    obj: *mut c_void,
    parameters: &[Parameter],
    args: &A,
    out_count: usize,
    return_kind: &MethodReturn,
    cif: &libffi::middle::Cif,
) -> windows_core::Result<CapturedHResultCall> {
    match call_method_dynamic_impl(
        vtable_index,
        obj,
        parameters,
        args,
        out_count,
        return_kind,
        cif,
        || {},
    )? {
        DynamicCallOutcome::Captured(call) => Ok(call),
        DynamicCallOutcome::Values(_) => Err(windows_core::Error::new(
            windows_core::HRESULT(0x80070057u32 as i32),
            "method does not use the captured HRESULT convention",
        )),
    }
}

fn call_method_dynamic_impl<A, F>(
    vtable_index: usize,
    obj: *mut c_void,
    parameters: &[Parameter],
    args: &A,
    out_count: usize,
    return_kind: &MethodReturn,
    cif: &libffi::middle::Cif,
    mark_dispatched: F,
) -> windows_core::Result<DynamicCallOutcome>
where
    A: ArgumentList + ?Sized,
    F: FnOnce(),
{
    use crate::metadata_table::ValueTypeData;
    use libffi::middle::CodePtr;

    let fptr = get_vtable_function_ptr(obj, vtable_index);
    let mut ffi_args: Vec<Arg> = Vec::with_capacity(parameters.len() * 2 + 1);
    let mut out_values: Vec<AbiValue> = Vec::with_capacity(out_count);
    let mut out_ptrs: Vec<*const std::ffi::c_void> = Vec::with_capacity(out_count);
    let mut struct_out_values: Vec<Option<ValueTypeData>> = Vec::with_capacity(out_count);
    let mut guid_out_values: Vec<Option<Box<windows_core::GUID>>> = Vec::with_capacity(out_count);
    let mut native_struct_out_values: Vec<Option<NativeStructStorage>> =
        Vec::with_capacity(out_count);
    let mut native_union_out_values =
        std::collections::BTreeMap::<usize, NativeUnionStorage>::new();
    let mut bstr_out_values = std::collections::BTreeMap::<usize, Box<BstrCallValue>>::new();
    let mut variant_out_values: Vec<Option<crate::com::VariantValue>> =
        Vec::with_capacity(out_count);
    let mut safe_array_out_values: Vec<Option<Box<crate::com::automation::SafeArrayOutput>>> =
        Vec::with_capacity(out_count);
    let mut prop_variant_out_values: Vec<Option<crate::com::PropVariantValue>> =
        Vec::with_capacity(out_count);
    let mut excep_info_out_values =
        std::collections::BTreeMap::<usize, Box<crate::com::automation::ExcepInfoOutput>>::new();
    let mut excep_info_values =
        std::collections::BTreeMap::<usize, crate::com::ExcepInfoValue>::new();
    let mut stat_stg_out_values =
        std::collections::BTreeMap::<usize, crate::com::StatStgOutput>::new();
    let mut format_etc_out_values =
        std::collections::BTreeMap::<usize, crate::com::FormatEtcOutput>::new();
    let mut stg_medium_out_values =
        std::collections::BTreeMap::<usize, crate::com::StgMediumStorage>::new();
    let mut optional_out_requests: Vec<Option<bool>> = Vec::with_capacity(out_count);

    // Array storage: Box'd for pointer stability (addresses don't change after creation)
    let mut array_out_slots: Vec<Box<ArrayOutSlot>> = Vec::new();
    // Map out value_index → array_out_slots index (None if not array)
    let mut array_out_map: Vec<Option<usize>> = Vec::with_capacity(out_count);
    // Pre-computed pointers into array_out_slots for use as ffi args
    let mut array_out_len_ptrs: Vec<*mut u32> = Vec::new();
    let mut array_out_data_ptrs: Vec<*mut *mut c_void> = Vec::new();

    // Array in-param storage: pre-compute all before building ffi_args
    let mut array_in_slots: Vec<Box<ArrayInSlot>> = Vec::new();
    let mut native_struct_in_slots: Vec<Option<NativeStructStorage>> = Vec::new();
    let mut bstr_in_slots: Vec<BstrCallValue> = Vec::new();
    let mut native_union_in_slots: Vec<Option<NativeUnionStorage>> = Vec::new();
    let mut variant_by_value_in_slots: Vec<crate::com::automation::VariantCopyValue> = Vec::new();
    let mut format_etc_in_slots: Vec<Box<windows::Win32::System::Com::FORMATETC>> = Vec::new();
    let mut stg_medium_in_slots: Vec<crate::com::StgMediumStorage> = Vec::new();

    // FillArray storage: caller-allocated buffers
    let mut fill_array_slots: Vec<Box<FillArraySlot>> = Vec::new();
    let mut fill_array_map: Vec<Option<usize>> = Vec::with_capacity(out_count);
    ffi_args.push(arg(&obj));

    // Phase 1a: Pre-allocate all out parameters
    for p in parameters {
        if p.is_out() {
            let requested = if p.is_optional_out() {
                match args.get_value(p.input_index.expect("optional output request index")) {
                    WinRTValue::Bool(requested) => *requested,
                    _ => {
                        return Err(windows_core::Error::new(
                            windows_core::HRESULT(0x80070057u32 as i32),
                            "optional COM output request must be Boolean",
                        ));
                    }
                }
            } else {
                true
            };
            optional_out_requests.push(p.is_optional_out().then_some(requested));
            if !requested {
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                out_ptrs.push(std::ptr::null());
                struct_out_values.push(None);
                guid_out_values.push(None);
                native_struct_out_values.push(None);
                variant_out_values.push(None);
                safe_array_out_values.push(None);
                prop_variant_out_values.push(None);
                array_out_map.push(None);
                fill_array_map.push(None);
                continue;
            }
            if p.is_fill_array() {
                // FillArray: caller allocates buffer. Use the capacity from args.
                let array_data = args
                    .get_value(p.input_index.expect("FillArray input index"))
                    .as_array()
                    .expect("Expected WinRTValue::Array with capacity for FillArray parameter");
                let elem_type = p.typ.array_element_type();
                let capacity = array_data.len() as u32;
                let elem_size = elem_type.element_size();
                let total_bytes = capacity as usize * elem_size;
                let buffer_ptr =
                    unsafe { windows::Win32::System::Com::CoTaskMemAlloc(total_bytes) as *mut u8 };
                assert!(!buffer_ptr.is_null(), "CoTaskMemAlloc failed for FillArray");
                unsafe { std::ptr::write_bytes(buffer_ptr, 0, total_bytes) };
                let slot = Box::new(FillArraySlot {
                    capacity,
                    buffer_ptr,
                    element_type: elem_type,
                });
                let slot_idx = fill_array_slots.len();
                fill_array_map.push(Some(slot_idx));
                fill_array_slots.push(slot);
                // Placeholders for index alignment
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                out_ptrs.push(std::ptr::null());
                struct_out_values.push(None);
                guid_out_values.push(None);
                native_struct_out_values.push(None);
                variant_out_values.push(None);
                safe_array_out_values.push(None);
                prop_variant_out_values.push(None);
                array_out_map.push(None);
            } else if p.typ.is_array() {
                let slot = Box::new(ArrayOutSlot {
                    length: 0u32,
                    data_ptr: std::ptr::null_mut(),
                    element_type: p.typ.array_element_type(),
                });
                let slot_idx = array_out_slots.len();
                array_out_map.push(Some(slot_idx));
                array_out_slots.push(slot);
                let slot_ref = &mut *array_out_slots[slot_idx];
                array_out_len_ptrs.push(&mut slot_ref.length);
                array_out_data_ptrs.push(&mut slot_ref.data_ptr);
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                out_ptrs.push(std::ptr::null());
                struct_out_values.push(None);
                guid_out_values.push(None);
                native_struct_out_values.push(None);
                variant_out_values.push(None);
                safe_array_out_values.push(None);
                prop_variant_out_values.push(None);
                fill_array_map.push(None);
            } else if p.typ.is_bstr() {
                let mut value = Box::new(BstrCallValue::new(
                    p.is_in_out()
                        .then(|| {
                            args.get_bstr(p.input_index.expect("BSTR in/out input index"))
                                .expect("validated BSTR in/out value")
                                .as_deref()
                        })
                        .flatten(),
                )?);
                out_ptrs.push(value.as_mut_ptr().cast());
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                struct_out_values.push(None);
                guid_out_values.push(None);
                native_struct_out_values.push(None);
                variant_out_values.push(None);
                safe_array_out_values.push(None);
                prop_variant_out_values.push(None);
                bstr_out_values.insert(p.value_index, value);
                array_out_map.push(None);
                fill_array_map.push(None);
            } else if p.typ.is_guid() {
                let value = Box::new(windows_core::GUID::zeroed());
                out_ptrs.push((&*value as *const windows_core::GUID).cast());
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                struct_out_values.push(None);
                guid_out_values.push(Some(value));
                native_struct_out_values.push(None);
                variant_out_values.push(None);
                safe_array_out_values.push(None);
                prop_variant_out_values.push(None);
                array_out_map.push(None);
                fill_array_map.push(None);
            } else if p.typ.is_native_union() {
                let layout = p
                    .typ
                    .native_union_layout()
                    .expect("native union parameter has a layout");
                let mut value = if p.is_in_out() {
                    NativeUnionStorage::from_value(
                        args.get_native_union(p.input_index.expect("native union input index"))
                            .expect("validated native union in/out value"),
                    )?
                } else {
                    NativeUnionStorage::zeroed(layout)?
                };
                out_ptrs.push(value.as_mut_ptr().cast());
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                struct_out_values.push(None);
                guid_out_values.push(None);
                native_struct_out_values.push(None);
                variant_out_values.push(None);
                safe_array_out_values.push(None);
                prop_variant_out_values.push(None);
                native_union_out_values.insert(p.value_index, value);
                array_out_map.push(None);
                fill_array_map.push(None);
            } else if p.typ.is_native_struct_pointer() {
                if !p.is_in_out() {
                    return Err(windows_core::Error::new(
                        windows_core::HRESULT(0x80070057u32 as i32),
                        "native struct pointer outputs require an in/out contract",
                    ));
                }
                let layout = p
                    .typ
                    .native_struct_layout()
                    .expect("native struct pointer parameter has a layout");
                let mut value = match args
                    .get_native_struct(p.input_index.expect("in/out input index"))
                {
                    Some(value) => Some(NativeStructStorage::from_bytes(layout, value.bytes())?),
                    None if p.typ.is_nullable_native_struct_pointer() => None,
                    None => {
                        return Err(windows_core::Error::new(
                            windows_core::HRESULT(0x80070057u32 as i32),
                            "expected native struct value for in/out parameter",
                        ));
                    }
                };
                out_ptrs.push(
                    value
                        .as_mut()
                        .map_or(std::ptr::null(), |value| value.as_mut_ptr().cast()),
                );
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                struct_out_values.push(None);
                guid_out_values.push(None);
                native_struct_out_values.push(value);
                variant_out_values.push(None);
                safe_array_out_values.push(None);
                prop_variant_out_values.push(None);
                array_out_map.push(None);
                fill_array_map.push(None);
            } else if p.typ.is_native_struct() {
                let layout = p
                    .typ
                    .native_struct_layout()
                    .expect("native struct parameter has a layout");
                let mut value = if p.is_in_out() {
                    NativeStructStorage::from_bytes(
                        layout,
                        args.get_native_struct(p.input_index.expect("in/out input index"))
                            .expect("validated native struct in/out value")
                            .bytes(),
                    )?
                } else {
                    NativeStructStorage::zeroed(layout)?
                };
                out_ptrs.push(value.as_mut_ptr().cast());
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                struct_out_values.push(None);
                guid_out_values.push(None);
                native_struct_out_values.push(Some(value));
                variant_out_values.push(None);
                safe_array_out_values.push(None);
                prop_variant_out_values.push(None);
                array_out_map.push(None);
                fill_array_map.push(None);
            } else if p.typ.is_variant() {
                let value = crate::com::VariantValue::empty();
                out_ptrs.push(value.raw_mut().cast());
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                struct_out_values.push(None);
                guid_out_values.push(None);
                native_struct_out_values.push(None);
                variant_out_values.push(Some(value));
                safe_array_out_values.push(None);
                prop_variant_out_values.push(None);
                array_out_map.push(None);
                fill_array_map.push(None);
            } else if p.typ.is_safe_array() {
                let mut value = Box::new(crate::com::automation::SafeArrayOutput::new());
                out_ptrs.push(value.as_mut_ptr().cast());
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                struct_out_values.push(None);
                guid_out_values.push(None);
                native_struct_out_values.push(None);
                variant_out_values.push(None);
                safe_array_out_values.push(Some(value));
                prop_variant_out_values.push(None);
                array_out_map.push(None);
                fill_array_map.push(None);
            } else if p.typ.is_prop_variant() {
                let value = crate::com::PropVariantValue::empty();
                out_ptrs.push(value.raw_mut().cast());
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                struct_out_values.push(None);
                guid_out_values.push(None);
                native_struct_out_values.push(None);
                variant_out_values.push(None);
                safe_array_out_values.push(None);
                prop_variant_out_values.push(Some(value));
                array_out_map.push(None);
                fill_array_map.push(None);
            } else if p.typ.is_excep_info() {
                let mut value = Box::new(crate::com::automation::ExcepInfoOutput::new());
                out_ptrs.push(value.as_mut_ptr().cast());
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                struct_out_values.push(None);
                guid_out_values.push(None);
                native_struct_out_values.push(None);
                variant_out_values.push(None);
                safe_array_out_values.push(None);
                prop_variant_out_values.push(None);
                excep_info_out_values.insert(p.value_index, value);
                array_out_map.push(None);
                fill_array_map.push(None);
            } else if p.typ.is_stat_stg() {
                let mut value = crate::com::StatStgOutput::new();
                out_ptrs.push(value.as_mut_ptr().cast());
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                struct_out_values.push(None);
                guid_out_values.push(None);
                native_struct_out_values.push(None);
                variant_out_values.push(None);
                safe_array_out_values.push(None);
                prop_variant_out_values.push(None);
                stat_stg_out_values.insert(p.value_index, value);
                array_out_map.push(None);
                fill_array_map.push(None);
            } else if p.typ.is_format_etc() {
                if p.is_in_out() {
                    return Err(windows_core::Error::new(
                        windows_core::HRESULT(0x80070057u32 as i32),
                        "FORMATETC in/out is not supported",
                    ));
                }
                let mut value = crate::com::FormatEtcOutput::new();
                out_ptrs.push(value.as_mut_ptr().cast());
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                struct_out_values.push(None);
                guid_out_values.push(None);
                native_struct_out_values.push(None);
                variant_out_values.push(None);
                safe_array_out_values.push(None);
                prop_variant_out_values.push(None);
                format_etc_out_values.insert(p.value_index, value);
                array_out_map.push(None);
                fill_array_map.push(None);
            } else if p.typ.is_stg_medium() {
                let mut value = if p.is_in_out() {
                    crate::com::StgMediumStorage::from_value(
                        args.get_stg_medium(p.input_index.expect("STGMEDIUM input index"))
                            .expect("validated STGMEDIUM in/out value"),
                    )
                    .map_err(|error| {
                        windows_core::Error::new(
                            windows_core::HRESULT(0x80070057u32 as i32),
                            &error.message(),
                        )
                    })?
                } else {
                    crate::com::StgMediumStorage::output()
                };
                out_ptrs.push(value.as_mut_ptr().cast());
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                struct_out_values.push(None);
                guid_out_values.push(None);
                native_struct_out_values.push(None);
                variant_out_values.push(None);
                safe_array_out_values.push(None);
                prop_variant_out_values.push(None);
                stg_medium_out_values.insert(p.value_index, value);
                array_out_map.push(None);
                fill_array_map.push(None);
            } else if p.typ.is_struct() {
                let val = if p.is_in_out() {
                    args.get_value(p.input_index.expect("in/out input index"))
                        .as_struct()
                        .ok_or_else(|| {
                            windows_core::Error::new(
                                windows_core::HRESULT(0x80070057u32 as i32),
                                "expected struct value for in/out parameter",
                            )
                        })?
                        .clone()
                } else {
                    p.typ.default_struct_value()
                };
                out_ptrs.push(val.as_ptr() as *const std::ffi::c_void);
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                struct_out_values.push(Some(val));
                guid_out_values.push(None);
                native_struct_out_values.push(None);
                variant_out_values.push(None);
                safe_array_out_values.push(None);
                prop_variant_out_values.push(None);
                array_out_map.push(None);
                fill_array_map.push(None);
            } else {
                let value = if p.is_in_out() {
                    input_abi_value(args.get_value(p.input_index.expect("in/out input index")))?
                } else {
                    p.typ.abi_type().default_value()
                };
                out_values.push(value);
                out_ptrs.push(out_values.last().unwrap().as_out_ptr());
                struct_out_values.push(None);
                guid_out_values.push(None);
                native_struct_out_values.push(None);
                variant_out_values.push(None);
                safe_array_out_values.push(None);
                prop_variant_out_values.push(None);
                array_out_map.push(None);
                fill_array_map.push(None);
            }
        }
    }

    // Phase 1b: Pre-compute all array in-param data (must happen before Phase 2)
    for p in parameters {
        if p.is_input() && !p.is_out() && p.typ.is_array() {
            let array_data = args
                .get_value(p.value_index)
                .as_array()
                .expect("Expected WinRTValue::Array for array in-parameter");
            let buffer = array_data.serialize_for_abi();
            let data_ptr = buffer.as_ptr();
            array_in_slots.push(Box::new(ArrayInSlot {
                length: array_data.len() as u32,
                data_ptr,
                _buffer: buffer,
            }));
        } else if p.is_input()
            && !p.is_out()
            && (p.typ.is_native_struct() || p.typ.is_native_struct_pointer())
        {
            let layout = p
                .typ
                .native_struct_layout()
                .expect("native struct input has a layout");
            let input_index = p.input_index.expect("native struct input index");
            let slot = match args.get_native_struct(input_index) {
                Some(value) => Some(NativeStructStorage::from_bytes(layout, value.bytes())?),
                None if p.typ.is_nullable_native_struct_pointer() => None,
                None => panic!("validated native struct input"),
            };
            native_struct_in_slots.push(slot);
        } else if p.is_input() && !p.is_out() && p.typ.is_bstr() {
            bstr_in_slots.push(BstrCallValue::new(
                args.get_bstr(p.input_index.expect("BSTR input index"))
                    .expect("validated BSTR input")
                    .as_deref(),
            )?);
        } else if p.is_input() && !p.is_out() && p.typ.native_union_layout().is_some() {
            let input_index = p.input_index.expect("native union input index");
            let slot = match args.get_native_union(input_index) {
                Some(value) => Some(NativeUnionStorage::from_value(value)?),
                None if p.typ.is_nullable_native_union_pointer() => None,
                None => panic!("validated native union input"),
            };
            native_union_in_slots.push(slot);
        } else if p.is_input() && !p.is_out() && p.typ.is_variant_by_value() {
            variant_by_value_in_slots.push(
                crate::com::automation::VariantCopyValue::new(
                    args.get_variant(p.input_index.expect("by-value VARIANT input index"))
                        .expect("validated by-value VARIANT input"),
                )
                .map_err(|error| {
                    windows_core::Error::new(
                        windows_core::HRESULT(0x80070057u32 as i32),
                        &error.message(),
                    )
                })?,
            );
        } else if p.is_input() && !p.is_out() && p.typ.is_format_etc() {
            format_etc_in_slots.push(Box::new(
                args.get_format_etc(p.input_index.expect("FORMATETC input index"))
                    .expect("validated FORMATETC input")
                    .to_raw(),
            ));
        } else if p.is_input() && !p.is_out() && p.typ.is_stg_medium() {
            stg_medium_in_slots.push(
                crate::com::StgMediumStorage::from_value(
                    args.get_stg_medium(p.input_index.expect("STGMEDIUM input index"))
                        .expect("validated STGMEDIUM input"),
                )
                .map_err(|error| {
                    windows_core::Error::new(
                        windows_core::HRESULT(0x80070057u32 as i32),
                        &error.message(),
                    )
                })?,
            );
        }
    }
    let native_struct_in_ptrs = native_struct_in_slots
        .iter()
        .map(|slot| {
            slot.as_ref()
                .map_or(std::ptr::null(), NativeStructStorage::as_ptr)
        })
        .collect::<Vec<_>>();
    let native_union_in_ptrs = native_union_in_slots
        .iter()
        .map(|slot| {
            slot.as_ref()
                .map_or(std::ptr::null(), NativeUnionStorage::as_ptr)
        })
        .collect::<Vec<_>>();
    let bstr_in_ptrs = bstr_in_slots
        .iter()
        .map(BstrCallValue::as_raw)
        .collect::<Vec<_>>();
    let variant_in_ptrs = parameters
        .iter()
        .filter(|parameter| {
            parameter.is_input() && !parameter.is_out() && parameter.typ.is_variant()
        })
        .map(|parameter| {
            args.get_variant(parameter.input_index.expect("VARIANT input index"))
                .expect("validated VARIANT input")
                .raw()
                .cast::<c_void>()
        })
        .collect::<Vec<_>>();
    let safe_array_inputs = parameters
        .iter()
        .filter(|parameter| {
            parameter.is_input() && !parameter.is_out() && parameter.typ.is_safe_array()
        })
        .map(|parameter| {
            args.get_safe_array(parameter.input_index.expect("SAFEARRAY input index"))
                .expect("validated SAFEARRAY input")
        })
        .collect::<Vec<_>>();
    let unique_safe_arrays = safe_array_inputs
        .iter()
        .map(|value| (value.identity(), *value))
        .collect::<std::collections::BTreeMap<_, _>>();
    let safe_array_in_guards = unique_safe_arrays
        .values()
        .map(|value| {
            value.lock_native().map_err(|error| {
                windows_core::Error::new(
                    windows_core::HRESULT(0x80070057u32 as i32),
                    &error.message(),
                )
            })
        })
        .collect::<windows_core::Result<Vec<_>>>()?;
    let safe_array_raw_by_identity = unique_safe_arrays
        .keys()
        .copied()
        .zip(safe_array_in_guards.iter().map(|guard| guard.as_raw()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let safe_array_in_ptrs = safe_array_inputs
        .iter()
        .map(|value| safe_array_raw_by_identity[&value.identity()].cast::<c_void>())
        .collect::<Vec<_>>();
    let prop_variant_in_ptrs = parameters
        .iter()
        .filter(|parameter| {
            parameter.is_input() && !parameter.is_out() && parameter.typ.is_prop_variant()
        })
        .map(|parameter| {
            args.get_prop_variant(parameter.input_index.expect("PROPVARIANT input index"))
                .expect("validated PROPVARIANT input")
                .raw()
                .cast::<c_void>()
        })
        .collect::<Vec<_>>();
    let dispatch_params_in_ptrs = parameters
        .iter()
        .filter(|parameter| {
            parameter.is_input() && !parameter.is_out() && parameter.typ.is_dispatch_params()
        })
        .map(|parameter| {
            args.get_dispatch_params(parameter.input_index.expect("DISPPARAMS input index"))
                .expect("validated DISPPARAMS input")
                .raw_mut()
                .cast::<c_void>()
        })
        .collect::<Vec<_>>();
    let format_etc_in_ptrs = format_etc_in_slots
        .iter_mut()
        .map(|value| (&mut **value as *mut windows::Win32::System::Com::FORMATETC).cast::<c_void>())
        .collect::<Vec<_>>();
    let stg_medium_in_ptrs = stg_medium_in_slots
        .iter_mut()
        .map(crate::com::StgMediumStorage::as_mut_ptr)
        .collect::<Vec<_>>();

    // Phase 2: Build ffi_args
    let mut array_in_idx = 0usize;
    let mut array_out_idx = 0usize;
    let mut native_struct_in_idx = 0usize;
    let mut native_union_in_idx = 0usize;
    let mut bstr_in_idx = 0usize;
    let mut variant_by_value_in_idx = 0usize;
    let mut variant_in_idx = 0usize;
    let mut safe_array_in_idx = 0usize;
    let mut prop_variant_in_idx = 0usize;
    let mut dispatch_params_in_idx = 0usize;
    let mut format_etc_in_idx = 0usize;
    let mut stg_medium_in_idx = 0usize;
    for p in parameters {
        if p.is_out() {
            if let Some(slot_idx) = fill_array_map[p.value_index] {
                // FillArray: push capacity and caller-allocated buffer.
                let slot = &*fill_array_slots[slot_idx];
                ffi_args.push(arg(&slot.capacity));
                ffi_args.push(arg(&slot.buffer_ptr));
            } else if array_out_map[p.value_index].is_some() {
                // ReceiveArray out: push TWO args (pointer-to-length, pointer-to-data_ptr)
                ffi_args.push(arg(&array_out_len_ptrs[array_out_idx]));
                ffi_args.push(arg(&array_out_data_ptrs[array_out_idx]));
                array_out_idx += 1;
            } else {
                ffi_args.push(arg(&out_ptrs[p.value_index]));
            }
        } else if p.typ.is_array() {
            // Array in: push TWO args (length value, data pointer value)
            let slot = &*array_in_slots[array_in_idx];
            ffi_args.push(arg(&slot.length));
            ffi_args.push(arg(&slot.data_ptr));
            array_in_idx += 1;
        } else if p.typ.is_native_struct() {
            let slot = native_struct_in_slots[native_struct_in_idx]
                .as_ref()
                .expect("by-value native struct cannot be null");
            ffi_args.push(Arg::new(slot.as_slice()));
            native_struct_in_idx += 1;
        } else if p.typ.is_native_struct_pointer() {
            ffi_args.push(arg(&native_struct_in_ptrs[native_struct_in_idx]));
            native_struct_in_idx += 1;
        } else if p.typ.is_native_union() {
            let slot = native_union_in_slots[native_union_in_idx]
                .as_ref()
                .expect("by-value native union input");
            ffi_args.push(Arg::new(slot.as_slice()));
            native_union_in_idx += 1;
        } else if p.typ.native_union_layout().is_some() {
            ffi_args.push(arg(&native_union_in_ptrs[native_union_in_idx]));
            native_union_in_idx += 1;
        } else if p.typ.is_bstr() {
            ffi_args.push(arg(&bstr_in_ptrs[bstr_in_idx]));
            bstr_in_idx += 1;
        } else if p.typ.is_variant_by_value() {
            ffi_args.push(arg(
                variant_by_value_in_slots[variant_by_value_in_idx].as_ref()
            ));
            variant_by_value_in_idx += 1;
        } else if p.typ.is_format_etc() {
            ffi_args.push(arg(&format_etc_in_ptrs[format_etc_in_idx]));
            format_etc_in_idx += 1;
        } else if p.typ.is_stg_medium() {
            ffi_args.push(arg(&stg_medium_in_ptrs[stg_medium_in_idx]));
            stg_medium_in_idx += 1;
        } else if p.typ.is_variant() {
            ffi_args.push(arg(&variant_in_ptrs[variant_in_idx]));
            variant_in_idx += 1;
        } else if p.typ.is_safe_array() {
            ffi_args.push(arg(&safe_array_in_ptrs[safe_array_in_idx]));
            safe_array_in_idx += 1;
        } else if p.typ.is_prop_variant() {
            ffi_args.push(arg(&prop_variant_in_ptrs[prop_variant_in_idx]));
            prop_variant_in_idx += 1;
        } else if p.typ.is_dispatch_params() {
            ffi_args.push(arg(&dispatch_params_in_ptrs[dispatch_params_in_idx]));
            dispatch_params_in_idx += 1;
        } else {
            ffi_args.push(args.get_value(p.value_index).libffi_arg());
        }
    }

    // Phase 3: Call
    let mut mark_dispatched = Some(mark_dispatched);
    let mut mark_dispatched = || {
        mark_dispatched
            .take()
            .expect("native dispatch marker must run exactly once")();
    };
    let call_result: windows_core::Result<(
        Option<NativeCallValue>,
        Option<windows_core::HRESULT>,
    )> = unsafe {
        match return_kind {
            MethodReturn::HResult => {
                mark_dispatched();
                let hr: windows_core::HRESULT = cif.call(CodePtr(fptr), &ffi_args);
                Ok((None, Some(hr)))
            }
            MethodReturn::SemanticHResult => {
                mark_dispatched();
                let hr: windows_core::HRESULT = cif.call(CodePtr(fptr), &ffi_args);
                Ok((
                    hr.is_ok()
                        .then_some(NativeCallValue::WinRt(WinRTValue::HResult(hr))),
                    Some(hr),
                ))
            }
            MethodReturn::PreservedHResult => {
                mark_dispatched();
                let hr: windows_core::HRESULT = cif.call(CodePtr(fptr), &ffi_args);
                Ok((Some(NativeCallValue::WinRt(WinRTValue::HResult(hr))), None))
            }
            MethodReturn::CapturedHResult(_) => {
                mark_dispatched();
                let hr: windows_core::HRESULT = cif.call(CodePtr(fptr), &ffi_args);
                Ok((None, Some(hr)))
            }
            MethodReturn::Void => {
                mark_dispatched();
                cif.call::<()>(CodePtr(fptr), &ffi_args);
                Ok((None, None))
            }
            MethodReturn::Value { typ, .. } => {
                if let crate::native_call::ParameterType::NativeStruct(layout) = typ {
                    NativeStructStorage::zeroed(layout).and_then(|mut storage| {
                        mark_dispatched();
                        cif.call_return_into(CodePtr(fptr), &ffi_args, storage.as_ret());
                        storage.validate_canaries()?;
                        crate::com::NativeStructValue::new(layout.clone(), storage.to_vec())
                            .map(|value| (Some(NativeCallValue::NativeStruct(value)), None))
                            .map_err(|error| {
                                windows_core::Error::new(
                                    windows_core::HRESULT(0x80070057u32 as i32),
                                    &error.message(),
                                )
                            })
                    })
                } else if let crate::native_call::ParameterType::NativeUnion(layout) = typ {
                    NativeUnionStorage::zeroed(layout).and_then(|mut storage| {
                        mark_dispatched();
                        cif.call_return_into(CodePtr(fptr), &ffi_args, storage.as_ret());
                        storage.validate_canaries()?;
                        crate::com::NativeUnionValue::from_returned_bytes(
                            layout.clone(),
                            storage.to_vec(),
                        )
                        .map(|value| (Some(NativeCallValue::NativeUnion(value)), None))
                        .map_err(|error| {
                            windows_core::Error::new(
                                windows_core::HRESULT(0x80070057u32 as i32),
                                &error.message(),
                            )
                        })
                    })
                } else {
                    mark_dispatched();
                    let value = match typ.abi_type() {
                        AbiType::Bool => AbiValue::Bool(cif.call(CodePtr(fptr), &ffi_args)),
                        AbiType::I8 => AbiValue::I8(cif.call(CodePtr(fptr), &ffi_args)),
                        AbiType::U8 => AbiValue::U8(cif.call(CodePtr(fptr), &ffi_args)),
                        AbiType::I16 => AbiValue::I16(cif.call(CodePtr(fptr), &ffi_args)),
                        AbiType::U16 => AbiValue::U16(cif.call(CodePtr(fptr), &ffi_args)),
                        AbiType::I32 => AbiValue::I32(cif.call(CodePtr(fptr), &ffi_args)),
                        AbiType::U32 => AbiValue::U32(cif.call(CodePtr(fptr), &ffi_args)),
                        AbiType::I64 => AbiValue::I64(cif.call(CodePtr(fptr), &ffi_args)),
                        AbiType::U64 => AbiValue::U64(cif.call(CodePtr(fptr), &ffi_args)),
                        AbiType::F32 => AbiValue::F32(cif.call(CodePtr(fptr), &ffi_args)),
                        AbiType::F64 => AbiValue::F64(cif.call(CodePtr(fptr), &ffi_args)),
                        AbiType::Guid => AbiValue::Guid(cif.call(CodePtr(fptr), &ffi_args)),
                        AbiType::Ptr => AbiValue::Pointer(cif.call(CodePtr(fptr), &ffi_args)),
                    };
                    typ.from_out_value(&value)
                        .map(|value| (Some(NativeCallValue::WinRt(value)), None))
                        .map_err(|error| {
                            windows_core::Error::new(
                                windows_core::HRESULT(0x80070057u32 as i32),
                                &error.message(),
                            )
                        })
                }
            }
        }
    };
    let (return_value, native_hresult) = match call_result {
        Ok(result) => result,
        Err(error) => {
            cleanup_failed_outputs(parameters, &mut out_values);
            return Err(error);
        }
    };

    if let MethodReturn::CapturedHResult(plan) = return_kind {
        const DISP_E_PARAMNOTFOUND: HRESULT = HRESULT(0x80020004u32 as i32);
        const DISP_E_TYPEMISMATCH: HRESULT = HRESULT(0x80020005u32 as i32);
        let hresult = native_hresult.expect("captured HRESULT call has a native HRESULT");
        let mut outputs = std::iter::repeat_with(|| None)
            .take(out_count)
            .collect::<Vec<_>>();
        let mut finalization_error = None;

        if hresult.is_ok() {
            if optional_out_requests[plan.result_output_index] == Some(true) {
                let value = variant_out_values[plan.result_output_index]
                    .take()
                    .expect("captured result output uses VARIANT storage");
                value.validate_supported().map_err(|error| {
                    windows_core::Error::new(
                        windows_core::HRESULT(0x80070057u32 as i32),
                        &error.message(),
                    )
                })?;
                outputs[plan.result_output_index] = Some(NativeCallValue::Variant(value));
            }
        } else {
            if optional_out_requests[plan.excep_info_output_index] == Some(true) {
                let output = excep_info_out_values
                    .remove(&plan.excep_info_output_index)
                    .expect("captured exception output uses EXCEPINFO storage");
                match (*output).into_value() {
                    Ok(value) if value.is_meaningful() => {
                        outputs[plan.excep_info_output_index] =
                            Some(NativeCallValue::ExcepInfo(value));
                    }
                    Ok(_) => {}
                    Err(error) => {
                        finalization_error = Some(windows_core::Error::new(
                            windows_core::HRESULT(0x80070057u32 as i32),
                            &error.message(),
                        ));
                    }
                }
            }
            if matches!(hresult, DISP_E_PARAMNOTFOUND | DISP_E_TYPEMISMATCH)
                && optional_out_requests[plan.arg_err_output_index] == Some(true)
            {
                let AbiValue::U32(arg_err) = out_values[plan.arg_err_output_index] else {
                    unreachable!("captured argument error output uses UINT storage");
                };
                outputs[plan.arg_err_output_index] =
                    Some(NativeCallValue::WinRt(WinRTValue::U32(arg_err)));
            }
            cleanup_failed_outputs(parameters, &mut out_values);
        }

        return Ok(DynamicCallOutcome::Captured(CapturedHResultCall {
            hresult,
            outputs,
            finalization_error,
        }));
    }

    let mut excep_info_error = None;
    for (index, output) in std::mem::take(&mut excep_info_out_values) {
        match (*output).into_value() {
            Ok(value) => {
                excep_info_values.insert(index, value);
            }
            Err(error) => {
                if excep_info_error.is_none() {
                    excep_info_error = Some(windows_core::Error::new(
                        windows_core::HRESULT(0x80070057u32 as i32),
                        &error.message(),
                    ));
                }
            }
        }
    }
    if let Some(hr) = native_hresult.filter(|hr| hr.is_err()) {
        cleanup_failed_outputs(parameters, &mut out_values);
        return Err(excep_info_error.unwrap_or_else(|| {
            hr.ok()
                .expect_err("a failing HRESULT must produce a windows-core error")
        }));
    }
    if let Some(error) = excep_info_error {
        cleanup_failed_outputs(parameters, &mut out_values);
        return Err(error);
    }

    // Counted FillArray methods (for example GetMany) carry the actual count
    // as their UInt32 retval. Other FillArray methods write the full capacity.
    let fill_array_actual_count = if fill_array_slots.is_empty() {
        None
    } else {
        parameters
            .iter()
            .rev()
            .find(|param| param.is_out() && !param.is_fill_array())
            .filter(|param| param.typ.is_u32())
            .and_then(|param| match out_values[param.value_index] {
                AbiValue::U32(value) => Some(value),
                _ => None,
            })
    };

    // Phase 4: Extract results
    let mut result_values: Vec<NativeCallValue> =
        Vec::with_capacity(out_count + usize::from(return_value.is_some()));
    let extraction_result = (|| -> windows_core::Result<()> {
        for p in parameters {
            if p.is_out() {
                if optional_out_requests[p.value_index] == Some(false) {
                    result_values.push(NativeCallValue::WinRt(WinRTValue::Null));
                    continue;
                }
                if let Some(slot_idx) = fill_array_map[p.value_index] {
                    // FillArray: transfer CoTaskMem buffer ownership to ArrayData
                    let slot = &mut fill_array_slots[slot_idx];
                    let length = fill_array_actual_count
                        .map(|count| count.min(slot.capacity))
                        .unwrap_or(slot.capacity) as usize;
                    let ptr = slot.buffer_ptr as *mut c_void;
                    slot.buffer_ptr = std::ptr::null_mut(); // prevent FillArraySlot::drop from freeing
                    result_values.push(NativeCallValue::WinRt(WinRTValue::Array(
                        crate::array::ArrayData::from_cotaskmem(
                            slot.element_type.clone(),
                            ptr,
                            length,
                        ),
                    )));
                } else if let Some(slot_idx) = array_out_map[p.value_index] {
                    // ReceiveArray: wrap callee-allocated CoTaskMem buffer directly.
                    // ArrayData takes ownership and will CoTaskMemFree + release elements on drop.
                    let slot = &mut array_out_slots[slot_idx];
                    let length = slot.length as usize;
                    let data_ptr = slot.data_ptr;
                    // Null out data_ptr so ArrayOutSlot::drop won't double-free.
                    slot.data_ptr = std::ptr::null_mut();
                    let array_value = if data_ptr.is_null() || length == 0 {
                        if !data_ptr.is_null() {
                            // Free empty but non-null buffer
                            unsafe {
                                windows::Win32::System::Com::CoTaskMemFree(Some(data_ptr));
                            }
                        }
                        crate::array::ArrayData::empty(slot.element_type.clone())
                    } else {
                        crate::array::ArrayData::from_cotaskmem(
                            slot.element_type.clone(),
                            data_ptr,
                            length,
                        )
                    };
                    result_values.push(NativeCallValue::WinRt(WinRTValue::Array(array_value)));
                } else if let Some(mut value) = bstr_out_values.remove(&p.value_index) {
                    let ptr = value.take().cast_mut().cast();
                    out_values[p.value_index] = AbiValue::Pointer(ptr);
                    result_values.push(NativeCallValue::WinRt(WinRTValue::RawPtr(ptr)));
                } else if let Some(guid) = guid_out_values[p.value_index].take() {
                    result_values.push(NativeCallValue::WinRt(WinRTValue::Guid(*guid)));
                } else if let Some(bytes) = native_struct_out_values[p.value_index].take() {
                    let layout = p
                        .typ
                        .native_struct_layout()
                        .expect("native struct output has a layout")
                        .clone();
                    result_values.push(NativeCallValue::NativeStruct(
                        crate::com::NativeStructValue::new(layout, bytes.to_vec()).map_err(
                            |error| {
                                windows_core::Error::new(
                                    windows_core::HRESULT(0x80070057u32 as i32),
                                    &error.message(),
                                )
                            },
                        )?,
                    ));
                } else if let Some(bytes) = native_union_out_values.remove(&p.value_index) {
                    let layout = p
                        .typ
                        .native_union_layout()
                        .expect("native union output has a layout")
                        .clone();
                    result_values.push(NativeCallValue::NativeUnion(
                        crate::com::NativeUnionValue::from_returned_bytes(layout, bytes.to_vec())
                            .map_err(|error| {
                            windows_core::Error::new(
                                windows_core::HRESULT(0x80070057u32 as i32),
                                &error.message(),
                            )
                        })?,
                    ));
                } else if p.typ.is_nullable_native_struct_pointer() && p.is_in_out() {
                    result_values.push(NativeCallValue::WinRt(WinRTValue::Null));
                } else if let Some(value) = variant_out_values[p.value_index].take() {
                    value.validate_supported().map_err(|error| {
                        windows_core::Error::new(
                            windows_core::HRESULT(0x80070057u32 as i32),
                            &error.message(),
                        )
                    })?;
                    result_values.push(NativeCallValue::Variant(value));
                } else if let Some(value) = safe_array_out_values[p.value_index].take() {
                    if value.is_null() && p.typ.is_nullable_safe_array() {
                        result_values.push(NativeCallValue::WinRt(WinRTValue::Null));
                    } else {
                        result_values.push(NativeCallValue::SafeArray(
                            (*value)
                                .into_value(
                                    p.typ.safe_array_element(),
                                    p.typ.safe_array_interface_iid(),
                                )
                                .map_err(|error| {
                                    windows_core::Error::new(
                                        windows_core::HRESULT(0x80070057u32 as i32),
                                        &error.message(),
                                    )
                                })?,
                        ));
                    }
                } else if let Some(value) = prop_variant_out_values[p.value_index].take() {
                    value.validate_supported().map_err(|error| {
                        windows_core::Error::new(
                            windows_core::HRESULT(0x80070057u32 as i32),
                            &error.message(),
                        )
                    })?;
                    result_values.push(NativeCallValue::PropVariant(value));
                } else if let Some(value) = excep_info_values.remove(&p.value_index) {
                    result_values.push(NativeCallValue::ExcepInfo(value));
                } else if let Some(value) = stat_stg_out_values.remove(&p.value_index) {
                    result_values.push(NativeCallValue::StatStg(value.into_value().map_err(
                        |error| {
                            windows_core::Error::new(
                                windows_core::HRESULT(0x80070057u32 as i32),
                                &error.message(),
                            )
                        },
                    )?));
                } else if let Some(value) = format_etc_out_values.remove(&p.value_index) {
                    result_values.push(NativeCallValue::FormatEtc(value.into_value().map_err(
                        |error| {
                            windows_core::Error::new(
                                windows_core::HRESULT(0x80070057u32 as i32),
                                &error.message(),
                            )
                        },
                    )?));
                } else if let Some(value) = stg_medium_out_values.remove(&p.value_index) {
                    result_values.push(NativeCallValue::StgMedium(value.into_value().map_err(
                        |error| {
                            windows_core::Error::new(
                                windows_core::HRESULT(0x80070057u32 as i32),
                                &error.message(),
                            )
                        },
                    )?));
                } else if let Some(struct_val) = struct_out_values[p.value_index].take() {
                    result_values.push(NativeCallValue::WinRt(WinRTValue::Struct(struct_val)));
                } else {
                    let mut out_value = p.typ.from_out_value(&out_values[p.value_index]).unwrap();
                    // Safety: null IUnknown crashes on clone/drop. Replace with Null variant.
                    out_value.sanitize_null_object();
                    let conversion_owns_native_value = !matches!(out_value, WinRTValue::RawPtr(_));
                    result_values.push(NativeCallValue::WinRt(out_value));
                    if conversion_owns_native_value
                        && let AbiValue::Pointer(ptr) = &mut out_values[p.value_index]
                    {
                        *ptr = std::ptr::null_mut();
                    }
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = extraction_result {
        cleanup_failed_outputs(parameters, &mut out_values);
        if let MethodReturn::Value { cleanup, .. } = return_kind
            && *cleanup != crate::native_call::OutputCleanup::None
            && let Some(NativeCallValue::WinRt(WinRTValue::RawPtr(ptr))) = &return_value
        {
            unsafe { cleanup.cleanup(*ptr) };
        }
        return Err(error);
    }
    if let Some(value) = return_value {
        result_values.insert(0, value);
    }
    Ok(DynamicCallOutcome::Values(result_values))
}

#[cfg(test)]
mod tests {
    use crate::com::{
        NativeStructField, NativeStructFieldType, NativeStructLayout, NativeStructScalar,
        NativeUnionField, NativeUnionFieldType, NativeUnionLayout,
    };

    use super::{NativeStructStorage, NativeUnionStorage};

    #[test]
    fn aggregate_return_storage_canaries_detect_out_of_span_writes() {
        let struct_layout = NativeStructLayout::new(
            "Tests.GuardedReturnStruct",
            8,
            8,
            vec![
                NativeStructField::new(
                    "value",
                    0,
                    1,
                    NativeStructFieldType::Scalar(NativeStructScalar::U64),
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let struct_storage = NativeStructStorage::zeroed(&struct_layout).unwrap();
        struct_storage.validate_canaries().unwrap();
        unsafe { struct_storage.ptr.as_ptr().sub(1).write(0) };
        assert!(struct_storage.validate_canaries().is_err());

        let union_layout = NativeUnionLayout::new(
            "Tests.GuardedReturnUnion",
            8,
            8,
            vec![
                NativeUnionField::new(
                    "value",
                    1,
                    NativeUnionFieldType::Scalar(NativeStructScalar::U64),
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let union_storage = NativeUnionStorage::zeroed(&union_layout).unwrap();
        union_storage.validate_canaries().unwrap();
        unsafe { union_storage.ptr.as_ptr().add(union_layout.size()).write(0) };
        assert!(union_storage.validate_canaries().is_err());
    }
}
