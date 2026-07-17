// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use core::ffi::c_void;
use libffi::middle::{Arg, arg};
use windows_core::{HRESULT, Interface};

use crate::{abi::AbiValue, signature::Parameter, value::WinRTValue};

pub(crate) trait ArgumentList {
    fn get_value(&self, index: usize) -> &WinRTValue;
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
/// fn(this, val, u32 capacity, *mut u8 items, *mut u32 actual) -> HRESULT
pub fn call_fill_array_1in(
    fptr: *mut c_void,
    obj: *mut c_void,
    in_val: &WinRTValue,
    capacity: u32,
    buffer: *mut u8,
    actual: *mut u32,
) -> HRESULT {
    dispatch_scalar!(in_val, |v| unsafe {
        let method: unsafe extern "system" fn(*mut c_void, _, u32, *mut u8, *mut u32) -> HRESULT =
            std::mem::transmute(fptr);
        method(obj, v, capacity, buffer, actual)
    })
}

use crate::metadata_table::{TypeHandle, TypeKind};

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
    actual_count: u32,   // callee writes the actual number of elements filled
    element_type: TypeHandle,
}

impl Drop for FillArraySlot {
    fn drop(&mut self) {
        // Release elements + free buffer if ownership was not transferred to ArrayData.
        // Buffer was zero-initialized before the call, so null slots are safe to release.
        // Use capacity as cleanup length — ArrayData::Drop skips null elements.
        if !self.buffer_ptr.is_null() {
            let cleanup_len = if self.actual_count == 0 && self.capacity > 0 {
                self.capacity as usize
            } else {
                std::cmp::min(self.actual_count, self.capacity) as usize
            };
            let _ = crate::array::ArrayData::from_cotaskmem(
                self.element_type.clone(),
                self.buffer_ptr as *mut c_void,
                cleanup_len,
            );
            self.buffer_ptr = std::ptr::null_mut();
        }
    }
}

pub fn call_winrt_method_dynamic<A: ArgumentList + ?Sized>(
    vtable_index: usize,
    obj: *mut c_void,
    parameters: &[Parameter],
    args: &A,
    out_count: usize,
    cif: &libffi::middle::Cif,
) -> windows_core::Result<Vec<WinRTValue>> {
    use crate::metadata_table::ValueTypeData;
    use libffi::middle::CodePtr;

    let fptr = get_vtable_function_ptr(obj, vtable_index);
    let mut ffi_args: Vec<Arg> = Vec::with_capacity(parameters.len() * 2 + 1);
    let mut out_values: Vec<AbiValue> = Vec::with_capacity(out_count);
    let mut out_ptrs: Vec<*const std::ffi::c_void> = Vec::with_capacity(out_count);
    let mut struct_out_values: Vec<Option<ValueTypeData>> = Vec::with_capacity(out_count);

    // Array storage: Box'd for pointer stability (addresses don't change after creation)
    let mut array_out_slots: Vec<Box<ArrayOutSlot>> = Vec::new();
    // Map out value_index → array_out_slots index (None if not array)
    let mut array_out_map: Vec<Option<usize>> = Vec::with_capacity(out_count);
    // Pre-computed pointers into array_out_slots for use as ffi args
    let mut array_out_len_ptrs: Vec<*mut u32> = Vec::new();
    let mut array_out_data_ptrs: Vec<*mut *mut c_void> = Vec::new();

    // Array in-param storage: pre-compute all before building ffi_args
    let mut array_in_slots: Vec<Box<ArrayInSlot>> = Vec::new();

    // FillArray storage: caller-allocated buffers
    let mut fill_array_slots: Vec<Box<FillArraySlot>> = Vec::new();
    let mut fill_array_map: Vec<Option<usize>> = Vec::with_capacity(out_count);
    // Pre-computed pointers to actual_count fields (must outlive ffi call)
    let mut fill_array_actual_ptrs: Vec<*mut u32> = Vec::new();

    ffi_args.push(arg(&obj));

    // Phase 1a: Pre-allocate all out parameters
    for p in parameters {
        if p.is_out() {
            if p.is_fill_array() {
                // FillArray: caller allocates buffer. Use the capacity from args.
                let array_data = args
                    .get_value(p.value_index)
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
                    actual_count: 0,
                    element_type: elem_type,
                });
                let slot_idx = fill_array_slots.len();
                fill_array_map.push(Some(slot_idx));
                fill_array_slots.push(slot);
                let slot_ref = &mut *fill_array_slots[slot_idx];
                fill_array_actual_ptrs.push(&mut slot_ref.actual_count);
                // Placeholders for index alignment
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                out_ptrs.push(std::ptr::null());
                struct_out_values.push(None);
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
                fill_array_map.push(None);
            } else if matches!(p.typ.kind(), TypeKind::Struct(_)) {
                let val = p.typ.default_value();
                out_ptrs.push(val.as_ptr() as *const std::ffi::c_void);
                out_values.push(AbiValue::Pointer(std::ptr::null_mut()));
                struct_out_values.push(Some(val));
                array_out_map.push(None);
                fill_array_map.push(None);
            } else {
                out_values.push(p.typ.abi_type().default_value());
                out_ptrs.push(out_values.last().unwrap().as_out_ptr());
                struct_out_values.push(None);
                array_out_map.push(None);
                fill_array_map.push(None);
            }
        }
    }

    // Phase 1b: Pre-compute all array in-param data (must happen before Phase 2)
    for p in parameters {
        if !p.is_out() && p.typ.is_array() {
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
        }
    }

    // Phase 2: Build ffi_args
    let mut array_in_idx = 0usize;
    let mut array_out_idx = 0usize;
    for p in parameters {
        if p.is_out() {
            if let Some(slot_idx) = fill_array_map[p.value_index] {
                // FillArray: push THREE args (capacity, buffer pointer, actual count pointer)
                let slot = &*fill_array_slots[slot_idx];
                ffi_args.push(arg(&slot.capacity));
                ffi_args.push(arg(&slot.buffer_ptr));
                ffi_args.push(arg(&fill_array_actual_ptrs[slot_idx]));
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
        } else {
            ffi_args.push(args.get_value(p.value_index).libffi_arg());
        }
    }

    // Phase 3: Call
    let hr: windows_core::HRESULT = unsafe { cif.call(CodePtr(fptr), &ffi_args) };
    hr.ok()?;

    // Phase 4: Extract results
    let mut result_values: Vec<WinRTValue> = Vec::with_capacity(out_count);
    for p in parameters {
        if p.is_out() {
            if let Some(slot_idx) = fill_array_map[p.value_index] {
                // FillArray: transfer CoTaskMem buffer ownership to ArrayData
                let slot = &mut fill_array_slots[slot_idx];
                // If callee didn't set actual_count (left as 0), assume full buffer.
                let raw_count = if slot.actual_count == 0 && slot.capacity > 0 {
                    slot.capacity
                } else {
                    slot.actual_count
                };
                // Clamp to capacity to prevent OOB reads if callee misbehaves.
                let actual = std::cmp::min(raw_count as usize, slot.capacity as usize);
                let ptr = slot.buffer_ptr as *mut c_void;
                slot.buffer_ptr = std::ptr::null_mut(); // prevent FillArraySlot::drop from freeing
                result_values.push(WinRTValue::Array(crate::array::ArrayData::from_cotaskmem(
                    slot.element_type.clone(),
                    ptr,
                    actual,
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
                result_values.push(WinRTValue::Array(array_value));
            } else if let Some(struct_val) = struct_out_values[p.value_index].take() {
                result_values.push(WinRTValue::Struct(struct_val));
            } else {
                let mut out_value = p.typ.from_out_value(&out_values[p.value_index]).unwrap();
                // Safety: null IUnknown crashes on clone/drop. Replace with Null variant.
                out_value.sanitize_null_object();
                result_values.push(out_value);
            }
        }
    }
    Ok(result_values)
}
