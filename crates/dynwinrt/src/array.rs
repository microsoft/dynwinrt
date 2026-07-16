// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use core::ffi::c_void;
use windows_core::{IUnknown, Interface};

use crate::metadata_table::{TypeHandle, TypeKind};
use crate::value::WinRTValue;

// ======================================================================
// Struct field helpers for array element Drop/Clone
// ======================================================================

/// Check if a struct TypeHandle has any non-blittable fields (recursively).
fn has_struct_non_blittable_fields(handle: &TypeHandle) -> bool {
    let count = handle.field_count();
    for i in 0..count {
        let kind = handle.table().field_kind(handle.kind(), i);
        if kind.needs_drop() {
            return true;
        }
        if let TypeKind::Struct(_) = kind {
            let field_handle = handle.field_type(i);
            if has_struct_non_blittable_fields(&field_handle) {
                return true;
            }
        }
    }
    false
}

/// Release non-blittable fields inside a struct stored in a raw buffer.
/// Recursively handles nested structs. Does NOT free the buffer itself.
unsafe fn release_struct_fields(handle: &TypeHandle, ptr: *const u8) {
    let count = handle.field_count();
    for i in 0..count {
        let kind = handle.table().field_kind(handle.kind(), i);
        if !kind.needs_drop_recursive() {
            continue;
        }
        let offset = handle.field_offset(i);
        unsafe {
            match kind {
                TypeKind::HString => {
                    let raw = *(ptr.add(offset) as *const *mut c_void);
                    if !raw.is_null() {
                        let _hstr: windows_core::HSTRING = std::mem::transmute(raw);
                    }
                }
                kind if kind.is_com_pointer() => {
                    let raw = *(ptr.add(offset) as *const *mut c_void);
                    if !raw.is_null() {
                        let _obj = IUnknown::from_raw(raw);
                    }
                }
                TypeKind::Struct(_) => {
                    let field_handle = handle.field_type(i);
                    release_struct_fields(&field_handle, ptr.add(offset));
                }
                _ => {}
            }
        }
    }
}

/// Duplicate non-blittable fields inside a struct in a raw buffer (after memcpy).
/// Recursively handles nested structs.
unsafe fn duplicate_struct_fields(handle: &TypeHandle, ptr: *mut u8) {
    let count = handle.field_count();
    for i in 0..count {
        let kind = handle.table().field_kind(handle.kind(), i);
        if !kind.needs_drop_recursive() {
            continue;
        }
        let offset = handle.field_offset(i);
        unsafe {
            match kind {
                TypeKind::HString => {
                    let raw = *(ptr.add(offset) as *const *mut c_void);
                    if !raw.is_null() {
                        let hstr: &windows_core::HSTRING =
                            &*((&raw) as *const *mut c_void as *const windows_core::HSTRING);
                        let cloned: *mut c_void = std::mem::transmute(hstr.clone());
                        (ptr.add(offset) as *mut *mut c_void).write(cloned);
                    }
                }
                kind if kind.is_com_pointer() => {
                    let raw = *(ptr.add(offset) as *const *mut c_void);
                    if !raw.is_null() {
                        let obj = IUnknown::from_raw_borrowed(&raw).unwrap().clone();
                        (ptr.add(offset) as *mut *mut c_void).write(obj.into_raw());
                    }
                }
                TypeKind::Struct(_) => {
                    let field_handle = handle.field_type(i);
                    duplicate_struct_fields(&field_handle, ptr.add(offset));
                }
                _ => {}
            }
        }
    }
}

/// How the array data is stored.
enum ArrayBuffer {
    /// User-built array (for PassArray). Elements are owned WinRTValues.
    /// Serialized to raw bytes only at FFI call time.
    Values(Vec<WinRTValue>),
    /// WinRT-allocated buffer (ReceiveArray / FillArray).
    /// Owns the buffer AND the element references.
    /// Drop releases non-blittable elements, then CoTaskMemFree.
    CoTaskMem { ptr: *mut c_void, len: usize },
}

impl std::fmt::Debug for ArrayBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArrayBuffer::Values(v) => write!(f, "Values({} elements)", v.len()),
            ArrayBuffer::CoTaskMem { ptr, len } => {
                write!(f, "CoTaskMem({:p}, {} elements)", ptr, len)
            }
        }
    }
}

/// Holds a dynamically-typed WinRT array.
///
/// Two representations:
/// - `Values`: owned `Vec<WinRTValue>`, used for arrays the caller builds (PassArray).
///   Clone/Drop delegate to WinRTValue which handles refcounting automatically.
/// - `CoTaskMem`: raw byte buffer from WinRT (ReceiveArray/FillArray).
///   Clone/Drop manually handle per-element refcounting on raw bytes.
pub struct ArrayData {
    pub element_type: TypeHandle,
    buffer: ArrayBuffer,
}

impl std::fmt::Debug for ArrayData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArrayData")
            .field("element_type", &self.element_type)
            .field("buffer", &self.buffer)
            .finish()
    }
}

impl ArrayData {
    pub fn empty(element_type: TypeHandle) -> Self {
        ArrayData {
            element_type,
            buffer: ArrayBuffer::CoTaskMem {
                ptr: std::ptr::null_mut(),
                len: 0,
            },
        }
    }

    /// Create an ArrayData from owned WinRTValues.
    /// Used for PassArray in-params. The values are cloned and owned by this ArrayData.
    pub fn from_values(element_type: TypeHandle, values: &[WinRTValue]) -> Self {
        ArrayData {
            element_type,
            buffer: ArrayBuffer::Values(values.to_vec()),
        }
    }

    /// Wrap a CoTaskMem-allocated buffer (ReceiveArray or FillArray pattern).
    /// ArrayData takes ownership and will CoTaskMemFree on drop.
    pub(crate) fn from_cotaskmem(
        element_type: TypeHandle,
        data_ptr: *mut c_void,
        len: usize,
    ) -> Self {
        ArrayData {
            element_type,
            buffer: ArrayBuffer::CoTaskMem { ptr: data_ptr, len },
        }
    }

    pub fn len(&self) -> usize {
        match &self.buffer {
            ArrayBuffer::Values(v) => v.len(),
            ArrayBuffer::CoTaskMem { len, .. } => *len,
        }
    }

    // ------------------------------------------------------------------
    // Blittable element access — zero-copy slice (CoTaskMem only)
    // ------------------------------------------------------------------

    /// Return the raw buffer as a typed slice. Only valid for CoTaskMem-backed arrays
    /// with blittable types where `size_of::<T>() == element_type.element_size()`.
    ///
    /// # Safety
    /// Caller must ensure T matches the actual element layout.
    pub unsafe fn as_typed_slice<T: Copy>(&self) -> &[T] {
        match &self.buffer {
            ArrayBuffer::CoTaskMem { ptr, len } => {
                assert_eq!(
                    std::mem::size_of::<T>(),
                    self.element_type.element_size(),
                    "as_typed_slice<T> size mismatch"
                );
                if *len == 0 {
                    return &[];
                }
                unsafe { std::slice::from_raw_parts(*ptr as *const T, *len) }
            }
            ArrayBuffer::Values(_) => {
                panic!("as_typed_slice not available for Values arrays; use get() instead")
            }
        }
    }

    // ------------------------------------------------------------------
    // Per-element access (works for all types)
    // ------------------------------------------------------------------

    /// Read element at `index` as a WinRTValue.
    /// For Values arrays, returns a clone of the stored value.
    /// For CoTaskMem arrays, reads from raw bytes (AddRef / DuplicateString as needed).
    pub fn get(&self, index: usize) -> WinRTValue {
        assert!(
            index < self.len(),
            "ArrayData::get index {} out of bounds (len {})",
            index,
            self.len()
        );
        match &self.buffer {
            ArrayBuffer::Values(v) => v[index].clone(),
            ArrayBuffer::CoTaskMem { ptr, .. } => self.get_from_raw(index, *ptr as *const u8),
        }
    }

    /// Read element from a raw byte buffer (CoTaskMem path).
    fn get_from_raw(&self, index: usize, base: *const u8) -> WinRTValue {
        let elem_size = self.element_type.element_size();
        unsafe {
            match self.element_type.kind() {
                TypeKind::Bool => WinRTValue::Bool(*base.add(index * elem_size) != 0),
                TypeKind::I8 => WinRTValue::I8(*(base.add(index * elem_size) as *const i8)),
                TypeKind::U8 => WinRTValue::U8(*base.add(index * elem_size)),
                TypeKind::I16 => WinRTValue::I16(*(base.add(index * elem_size) as *const i16)),
                TypeKind::U16 | TypeKind::Char16 => {
                    WinRTValue::U16(*(base.add(index * elem_size) as *const u16))
                }
                TypeKind::I32 => WinRTValue::I32(*(base.add(index * elem_size) as *const i32)),
                TypeKind::Enum(_) => WinRTValue::Enum {
                    value: *(base.add(index * elem_size) as *const i32),
                    type_handle: self.element_type.clone(),
                },
                TypeKind::U32 => WinRTValue::U32(*(base.add(index * elem_size) as *const u32)),
                TypeKind::I64 => WinRTValue::I64(*(base.add(index * elem_size) as *const i64)),
                TypeKind::U64 => WinRTValue::U64(*(base.add(index * elem_size) as *const u64)),
                TypeKind::F32 => WinRTValue::F32(*(base.add(index * elem_size) as *const f32)),
                TypeKind::F64 => WinRTValue::F64(*(base.add(index * elem_size) as *const f64)),
                TypeKind::Guid => {
                    let guid = *(base.add(index * 16) as *const windows_core::GUID);
                    WinRTValue::Guid(guid)
                }
                TypeKind::HString => {
                    let raw = *(base.add(index * elem_size) as *const *mut c_void);
                    // Duplicate: read the handle and clone it (bumps refcount)
                    let hstr: &windows_core::HSTRING =
                        &*((&raw) as *const *mut c_void as *const windows_core::HSTRING);
                    WinRTValue::HString(hstr.clone())
                }
                kind if kind.is_com_pointer() => {
                    let raw = *(base.add(index * elem_size) as *const *mut c_void);
                    if raw.is_null() {
                        WinRTValue::Null
                    } else {
                        // from_raw takes ownership, but we want a clone — so AddRef first
                        let obj = IUnknown::from_raw_borrowed(&raw).unwrap();
                        WinRTValue::Object(obj.clone())
                    }
                }
                TypeKind::Struct(_) => {
                    let sz = self.element_type.size_of();
                    let mut vd = self.element_type.default_value();
                    std::ptr::copy_nonoverlapping(base.add(index * sz), vd.as_mut_ptr(), sz);
                    // Duplicate non-blittable fields so the returned copy owns its own references
                    if has_struct_non_blittable_fields(&self.element_type) {
                        duplicate_struct_fields(&self.element_type, vd.as_mut_ptr());
                    }
                    WinRTValue::Struct(vd)
                }
                other => panic!("ArrayData::get unsupported element type: {:?}", other),
            }
        }
    }

    // ------------------------------------------------------------------
    // Convenience typed getters
    // ------------------------------------------------------------------

    pub fn get_i32(&self, index: usize) -> i32 {
        match &self.buffer {
            ArrayBuffer::Values(v) => v[index].as_i32().unwrap(),
            ArrayBuffer::CoTaskMem { ptr, len } => {
                assert!(index < *len);
                unsafe { *((*ptr as *const u8).add(index * 4) as *const i32) }
            }
        }
    }

    // ------------------------------------------------------------------
    // ABI serialization (for PassArray FFI calls)
    // ------------------------------------------------------------------

    /// Serialize elements to a contiguous byte buffer for PassArray ABI.
    /// Returns an owned Vec<u8> that must be kept alive for the duration of the FFI call.
    pub(crate) fn serialize_for_abi(&self) -> Vec<u8> {
        match &self.buffer {
            ArrayBuffer::Values(values) => serialize_to_buffer(&self.element_type, values),
            ArrayBuffer::CoTaskMem { ptr, len } => {
                let elem_size = self.element_type.element_size();
                let total = *len * elem_size;
                let mut buf = vec![0u8; total];
                if total > 0 {
                    unsafe {
                        std::ptr::copy_nonoverlapping(*ptr as *const u8, buf.as_mut_ptr(), total);
                    }
                }
                buf
            }
        }
    }
}

// ======================================================================
// Drop — release non-blittable elements, then free the buffer
// ======================================================================

impl Drop for ArrayData {
    fn drop(&mut self) {
        // Values: Vec<WinRTValue> drops automatically, WinRTValue handles Release/DeleteString.
        // We only need manual cleanup for CoTaskMem.
        let buffer = std::mem::replace(
            &mut self.buffer,
            ArrayBuffer::CoTaskMem {
                ptr: std::ptr::null_mut(),
                len: 0,
            },
        );

        if let ArrayBuffer::CoTaskMem { ptr, len } = buffer {
            if len > 0 && !ptr.is_null() {
                let base = ptr as *const u8;
                let elem_size = self.element_type.element_size();
                let kind = self.element_type.kind();

                // Release non-blittable elements
                match kind {
                    TypeKind::HString => {
                        for i in 0..len {
                            unsafe {
                                let raw = *(base.add(i * elem_size) as *const *mut c_void);
                                if !raw.is_null() {
                                    let _hstr: windows_core::HSTRING = std::mem::transmute(raw);
                                }
                            }
                        }
                    }
                    kind if kind.is_com_pointer() => {
                        for i in 0..len {
                            unsafe {
                                let raw = *(base.add(i * elem_size) as *const *mut c_void);
                                if !raw.is_null() {
                                    let _obj = IUnknown::from_raw(raw);
                                }
                            }
                        }
                    }
                    TypeKind::Struct(_) => {
                        // Recurse: release non-blittable fields inside each struct element.
                        // Reuse ValueTypeData machinery for correct recursive release.
                        for i in 0..len {
                            unsafe {
                                let elem_ptr = base.add(i * elem_size);
                                release_struct_fields(&self.element_type, elem_ptr);
                            }
                        }
                    }
                    _ => {}
                }
            }

            if !ptr.is_null() {
                unsafe {
                    windows::Win32::System::Com::CoTaskMemFree(Some(ptr));
                }
            }
        }
        // ArrayBuffer::Values is dropped automatically here
    }
}

// ======================================================================
// Clone
// ======================================================================

impl Clone for ArrayData {
    fn clone(&self) -> Self {
        match &self.buffer {
            ArrayBuffer::Values(v) => ArrayData {
                element_type: self.element_type.clone(),
                buffer: ArrayBuffer::Values(v.clone()),
            },
            ArrayBuffer::CoTaskMem { ptr, len } => {
                if *len == 0 || ptr.is_null() {
                    return ArrayData::empty(self.element_type.clone());
                }

                let elem_size = self.element_type.element_size();
                let total_bytes = *len * elem_size;
                let base = *ptr as *const u8;

                let new_ptr = unsafe { windows::Win32::System::Com::CoTaskMemAlloc(total_bytes) };
                assert!(
                    !new_ptr.is_null(),
                    "CoTaskMemAlloc failed in ArrayData::clone"
                );
                let new_buf = new_ptr as *mut u8;

                let kind = self.element_type.kind();
                match kind {
                    TypeKind::HString => {
                        unsafe { std::ptr::write_bytes(new_buf, 0, total_bytes) };
                        for i in 0..*len {
                            unsafe {
                                let raw = *(base.add(i * elem_size) as *const *mut c_void);
                                if !raw.is_null() {
                                    let hstr: &windows_core::HSTRING = &*((&raw)
                                        as *const *mut c_void
                                        as *const windows_core::HSTRING);
                                    let cloned: *mut c_void = std::mem::transmute(hstr.clone());
                                    (new_buf.add(i * elem_size) as *mut *mut c_void).write(cloned);
                                }
                            }
                        }
                    }
                    kind if kind.is_com_pointer() => {
                        unsafe { std::ptr::write_bytes(new_buf, 0, total_bytes) };
                        for i in 0..*len {
                            unsafe {
                                let raw = *(base.add(i * elem_size) as *const *mut c_void);
                                if !raw.is_null() {
                                    let obj = IUnknown::from_raw_borrowed(&raw).unwrap().clone();
                                    (new_buf.add(i * elem_size) as *mut *mut c_void)
                                        .write(obj.into_raw());
                                }
                            }
                        }
                    }
                    TypeKind::Struct(_) if has_struct_non_blittable_fields(&self.element_type) => {
                        // memcpy all elements first, then duplicate non-blittable fields per element
                        unsafe { std::ptr::copy_nonoverlapping(base, new_buf, total_bytes) };
                        for i in 0..*len {
                            unsafe {
                                duplicate_struct_fields(
                                    &self.element_type,
                                    new_buf.add(i * elem_size),
                                );
                            }
                        }
                    }
                    _ => {
                        unsafe { std::ptr::copy_nonoverlapping(base, new_buf, total_bytes) };
                    }
                }

                ArrayData {
                    element_type: self.element_type.clone(),
                    buffer: ArrayBuffer::CoTaskMem {
                        ptr: new_ptr,
                        len: *len,
                    },
                }
            }
        }
    }
}

// ======================================================================
// Serialization — WinRTValue → raw bytes for PassArray ABI
// ======================================================================

fn serialize_to_buffer(element_type: &TypeHandle, values: &[WinRTValue]) -> Vec<u8> {
    let elem_size = element_type.element_size();
    let mut buffer = Vec::with_capacity(values.len() * elem_size);
    for elem in values {
        match elem {
            WinRTValue::Bool(v) => buffer.push(*v as u8),
            WinRTValue::I8(v) => buffer.extend_from_slice(&v.to_ne_bytes()),
            WinRTValue::U8(v) => buffer.push(*v),
            WinRTValue::I16(v) => buffer.extend_from_slice(&v.to_ne_bytes()),
            WinRTValue::U16(v) => buffer.extend_from_slice(&v.to_ne_bytes()),
            WinRTValue::I32(v) => buffer.extend_from_slice(&v.to_ne_bytes()),
            WinRTValue::Enum { value, .. } => buffer.extend_from_slice(&value.to_ne_bytes()),
            WinRTValue::U32(v) => buffer.extend_from_slice(&v.to_ne_bytes()),
            WinRTValue::I64(v) => buffer.extend_from_slice(&v.to_ne_bytes()),
            WinRTValue::U64(v) => buffer.extend_from_slice(&v.to_ne_bytes()),
            WinRTValue::F32(v) => buffer.extend_from_slice(&v.to_ne_bytes()),
            WinRTValue::F64(v) => buffer.extend_from_slice(&v.to_ne_bytes()),
            WinRTValue::Object(obj) => {
                buffer.extend_from_slice(&(obj.as_raw() as usize).to_ne_bytes());
            }
            WinRTValue::HString(s) => {
                let raw: usize = unsafe { std::mem::transmute_copy(s) };
                buffer.extend_from_slice(&raw.to_ne_bytes());
            }
            WinRTValue::Guid(g) => {
                let bytes: &[u8; 16] =
                    unsafe { &*(g as *const windows_core::GUID as *const [u8; 16]) };
                buffer.extend_from_slice(bytes);
            }
            WinRTValue::Struct(vd) => {
                let size = vd.type_handle().size_of();
                let src = unsafe { std::slice::from_raw_parts(vd.as_ptr(), size) };
                buffer.extend_from_slice(src);
            }
            _ => panic!(
                "Unsupported array element type for serialization: {:?}",
                elem
            ),
        }
    }
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata_table::MetadataTable;

    #[test]
    fn test_null_com_element_returns_null_variant() {
        // Verify that reading a null COM pointer from a CoTaskMem array
        // returns WinRTValue::Null, not WinRTValue::Object(null).
        let table = MetadataTable::new();
        let elem = table.object();
        let elem_size = std::mem::size_of::<*mut c_void>();
        let len = 2usize;
        let total = len * elem_size;
        let ptr = unsafe { windows::Win32::System::Com::CoTaskMemAlloc(total) as *mut c_void };
        assert!(!ptr.is_null());
        // Zero the buffer — all elements are null pointers
        unsafe { std::ptr::write_bytes(ptr as *mut u8, 0, total) };

        let array = ArrayData::from_cotaskmem(elem, ptr, len);
        let val = array.get(0);
        assert!(
            val.is_null_object(),
            "Expected WinRTValue::Null for null COM element, got {:?}",
            val
        );
        let val1 = array.get(1);
        assert!(
            val1.is_null_object(),
            "Expected WinRTValue::Null for null COM element, got {:?}",
            val1
        );
    }

    /// P1: CoTaskMem array of structs with HString fields — Clone/Drop must recurse.
    #[test]
    fn test_struct_array_with_hstring_clone_drop() {
        let table = MetadataTable::new();
        // Struct with one HString field (pointer-sized)
        let stype = table.struct_type("Test.NamedItem", &[table.hstring()]);
        let elem_size = stype.element_size();

        // Allocate a CoTaskMem buffer for 2 elements
        let len = 2usize;
        let total = len * elem_size;
        let ptr = unsafe { windows::Win32::System::Com::CoTaskMemAlloc(total) as *mut c_void };
        assert!(!ptr.is_null());
        unsafe { std::ptr::write_bytes(ptr as *mut u8, 0, total) };

        // Write HStrings into the buffer
        let hstr1 = windows_core::HSTRING::from("item_one");
        let hstr2 = windows_core::HSTRING::from("item_two");
        unsafe {
            let base = ptr as *mut u8;
            let raw1: *mut c_void = std::mem::transmute(hstr1);
            let raw2: *mut c_void = std::mem::transmute(hstr2);
            (base as *mut *mut c_void).write(raw1);
            (base.add(elem_size) as *mut *mut c_void).write(raw2);
        }

        let array = ArrayData::from_cotaskmem(stype, ptr, len);

        // Read elements
        let v0 = array.get(0);
        let _v1 = array.get(1);
        if let WinRTValue::Struct(s0) = &v0 {
            let raw: *mut c_void = s0.get_field(0);
            assert!(!raw.is_null(), "field 0 should be non-null HString");
        } else {
            panic!("Expected Struct, got {:?}", v0);
        }

        // Clone the array — this exercises recursive struct field duplication
        let cloned = array.clone();
        assert_eq!(cloned.len(), 2);

        // Drop both — if recursive release is broken, this would crash or leak
        drop(cloned);
        drop(array);
    }
}
