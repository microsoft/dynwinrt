// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use core::ffi::c_void;
use windows_core::{HSTRING, IUnknown, Interface};

use super::type_handle::TypeHandle;
use super::type_kind::TypeKind;

/// Release non-blittable fields (HString, COM pointers, nested structs) in a struct buffer.
/// Called by Drop and before overwriting. Recurses into nested structs.
unsafe fn release_non_blittable_fields(handle: &TypeHandle, ptr: *const u8) {
    if !matches!(handle.kind(), TypeKind::Struct(_)) {
        return;
    }
    let count = handle.field_count();
    for i in 0..count {
        let kind = handle.table.field_kind(handle.kind, i);
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
                    release_non_blittable_fields(&field_handle, ptr.add(offset));
                }
                _ => {}
            }
        }
    }
}

/// Duplicate non-blittable fields (HString, COM pointers, nested structs) after a memcpy.
/// The source retains its references; the destination gets new ones.
/// Recurses into nested structs.
unsafe fn duplicate_non_blittable_fields(handle: &TypeHandle, ptr: *mut u8) {
    if !matches!(handle.kind(), TypeKind::Struct(_)) {
        return;
    }
    let count = handle.field_count();
    for i in 0..count {
        let kind = handle.table.field_kind(handle.kind, i);
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
                    duplicate_non_blittable_fields(&field_handle, ptr.add(offset));
                }
                _ => {}
            }
        }
    }
}

/// Check if a struct type has any non-blittable fields (recursing into nested structs).
fn has_non_blittable_fields(handle: &TypeHandle) -> bool {
    if !matches!(handle.kind(), TypeKind::Struct(_)) {
        return false;
    }
    let count = handle.field_count();
    for i in 0..count {
        let kind = handle.table.field_kind(handle.kind, i);
        if kind.needs_drop() {
            return true;
        }
        if let TypeKind::Struct(_) = kind {
            let field_handle = handle.field_type(i);
            if has_non_blittable_fields(&field_handle) {
                return true;
            }
        }
    }
    false
}

/// A dynamically-typed value matching a struct layout from the registry.
///
/// Owns an aligned heap allocation. Holds a `TypeHandle` internally so
/// field access methods are self-contained.
pub struct ValueTypeData {
    type_handle: TypeHandle,
    ptr: *mut u8,
}

impl std::fmt::Debug for ValueTypeData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValueTypeData")
            .field("type_handle", &self.type_handle)
            .field("ptr", &self.ptr)
            .finish()
    }
}

impl ValueTypeData {
    pub(crate) fn new(handle: &TypeHandle) -> Self {
        let layout = handle.layout();
        let ptr = if layout.size() > 0 {
            unsafe { std::alloc::alloc_zeroed(layout) }
        } else {
            std::ptr::null_mut()
        };
        Self {
            type_handle: handle.clone(),
            ptr,
        }
    }

    pub fn type_handle(&self) -> &TypeHandle {
        &self.type_handle
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }

    fn checked_field_handle(&self, index: usize) -> crate::result::Result<(TypeHandle, usize)> {
        let kind = self.type_handle.kind();
        if !matches!(kind, TypeKind::Struct(_)) {
            return Err(crate::result::Error::ExpectStructTypeError(kind));
        }
        let field_count = self.type_handle.field_count();
        if index >= field_count {
            return Err(crate::result::Error::IndexOutOfBounds {
                index,
                len: field_count,
            });
        }
        Ok((
            self.type_handle.field_type(index),
            self.type_handle.field_offset(index),
        ))
    }

    pub fn field_type_checked(&self, index: usize) -> crate::result::Result<TypeHandle> {
        self.checked_field_handle(index)
            .map(|(field_handle, _)| field_handle)
    }

    pub fn field_kind_checked(&self, index: usize) -> crate::result::Result<TypeKind> {
        self.field_type_checked(index)
            .map(|field_handle| field_handle.kind())
    }

    pub(crate) unsafe fn copy_to_abi(&self, result: *mut c_void) {
        let layout = self.type_handle.layout();
        if layout.size() == 0 {
            return;
        }

        unsafe {
            std::ptr::copy_nonoverlapping(self.ptr, result as *mut u8, layout.size());
        }
        if has_non_blittable_fields(&self.type_handle) {
            unsafe {
                duplicate_non_blittable_fields(&self.type_handle, result as *mut u8);
            }
        }
    }

    pub fn get_field<T: Copy>(&self, index: usize) -> T {
        let h = &self.type_handle;
        let offset = h.field_offset(index);
        assert_eq!(
            std::mem::size_of::<T>(),
            h.field_type(index).size_of(),
            "get_field<T> size mismatch"
        );
        unsafe { (self.ptr.add(offset) as *const T).read() }
    }

    pub fn set_field<T: Copy>(&mut self, index: usize, value: T) {
        let h = &self.type_handle;
        let offset = h.field_offset(index);
        assert_eq!(
            std::mem::size_of::<T>(),
            h.field_type(index).size_of(),
            "set_field<T> size mismatch"
        );
        unsafe { (self.ptr.add(offset) as *mut T).write(value) }
    }

    pub fn get_field_hstring(&self, index: usize) -> crate::result::Result<HSTRING> {
        let (field_handle, offset) = self.checked_field_handle(index)?;
        if field_handle.kind() != TypeKind::HString {
            return Err(crate::result::Error::InvalidType(
                TypeKind::HString,
                field_handle.kind(),
            ));
        }
        let raw = unsafe { *(self.ptr.add(offset) as *const *mut c_void) };
        if raw.is_null() {
            Ok(HSTRING::new())
        } else {
            let value: &HSTRING = unsafe { &*((&raw) as *const *mut c_void as *const HSTRING) };
            Ok(value.clone())
        }
    }

    pub fn set_field_hstring(&mut self, index: usize, value: HSTRING) -> crate::result::Result<()> {
        let (field_handle, offset) = self.checked_field_handle(index)?;
        if field_handle.kind() != TypeKind::HString {
            return Err(crate::result::Error::InvalidType(
                TypeKind::HString,
                field_handle.kind(),
            ));
        }
        let field = unsafe { &mut *(self.ptr.add(offset) as *mut *mut c_void) };
        let old_raw = std::mem::replace(field, unsafe { std::mem::transmute(value) });
        if !old_raw.is_null() {
            let _old_value: HSTRING = unsafe { std::mem::transmute(old_raw) };
        }
        Ok(())
    }

    pub fn get_field_object(&self, index: usize) -> crate::result::Result<Option<IUnknown>> {
        let (field_handle, offset) = self.checked_field_handle(index)?;
        if !field_handle.kind().is_com_pointer() {
            return Err(crate::result::Error::expect_object_type(
                field_handle.kind(),
            ));
        }
        let raw = unsafe { *(self.ptr.add(offset) as *const *mut c_void) };
        if raw.is_null() {
            Ok(None)
        } else {
            Ok(unsafe { IUnknown::from_raw_borrowed(&raw) }.map(Clone::clone))
        }
    }

    pub fn set_field_object(
        &mut self,
        index: usize,
        value: Option<&IUnknown>,
    ) -> crate::result::Result<()> {
        let (field_handle, offset) = self.checked_field_handle(index)?;
        if !field_handle.kind().is_com_pointer() {
            return Err(crate::result::Error::expect_object_type(
                field_handle.kind(),
            ));
        }
        let field = unsafe { &mut *(self.ptr.add(offset) as *mut *mut c_void) };
        let new_raw = if let Some(object) = value {
            let iid = if field_handle.kind() == TypeKind::Object {
                windows_core::IInspectable::IID
            } else {
                field_handle.iid().ok_or_else(|| {
                    crate::result::Error::NotAnInterface(field_handle.signature_string())
                })?
            };
            let mut queried = std::ptr::null_mut();
            unsafe { object.query(&iid, &mut queried) }.ok()?;
            queried
        } else {
            std::ptr::null_mut()
        };
        let old_raw = std::mem::replace(field, new_raw);
        if !old_raw.is_null() {
            unsafe { drop(IUnknown::from_raw(old_raw)) };
        }
        Ok(())
    }

    pub fn get_field_struct(&self, index: usize) -> ValueTypeData {
        self.get_field_struct_checked(index)
            .expect("get_field_struct failed")
    }

    pub fn get_field_struct_checked(&self, index: usize) -> crate::result::Result<ValueTypeData> {
        let (field_handle, offset) = self.checked_field_handle(index)?;
        if !matches!(field_handle.kind(), TypeKind::Struct(_)) {
            return Err(crate::result::Error::ExpectStructTypeError(
                field_handle.kind(),
            ));
        }
        let layout = field_handle.layout();
        let result = field_handle.default_value();
        if layout.size() > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(self.ptr.add(offset), result.ptr, layout.size());
                // Duplicate non-blittable fields so both copies are valid
                if has_non_blittable_fields(&field_handle) {
                    duplicate_non_blittable_fields(&field_handle, result.ptr);
                }
            }
        }
        Ok(result)
    }

    pub fn set_field_struct(&mut self, index: usize, value: &ValueTypeData) {
        self.set_field_struct_checked(index, value)
            .expect("set_field_struct failed");
    }

    pub fn set_field_struct_checked(
        &mut self,
        index: usize,
        value: &ValueTypeData,
    ) -> crate::result::Result<()> {
        let (field_handle, offset) = self.checked_field_handle(index)?;
        if !matches!(field_handle.kind(), TypeKind::Struct(_)) {
            return Err(crate::result::Error::ExpectStructTypeError(
                field_handle.kind(),
            ));
        }
        if field_handle.kind() != value.type_handle.kind() {
            return Err(crate::result::Error::InvalidType(
                field_handle.kind(),
                value.type_handle.kind(),
            ));
        }
        let size = field_handle.size_of();
        debug_assert_eq!(
            size,
            value.type_handle.size_of(),
            "set_field_struct size mismatch"
        );
        if size > 0 {
            unsafe {
                // Release old non-blittable fields before overwriting
                if has_non_blittable_fields(&field_handle) {
                    release_non_blittable_fields(&field_handle, self.ptr.add(offset));
                }
                std::ptr::copy_nonoverlapping(value.ptr, self.ptr.add(offset), size);
                // Duplicate non-blittable fields so both copies are valid
                if has_non_blittable_fields(&field_handle) {
                    duplicate_non_blittable_fields(&field_handle, self.ptr.add(offset));
                }
            }
        }
        Ok(())
    }

    pub fn call_method_struct_to_object(
        &self,
        obj_raw: *mut std::ffi::c_void,
        method_index: usize,
    ) -> windows_core::Result<windows_core::IUnknown> {
        use crate::call::get_vtable_function_ptr;
        use libffi::middle::{CodePtr, Type, arg};

        let fptr = get_vtable_function_ptr(obj_raw, method_index);
        let cif = crate::native_call::system_cif(
            vec![
                Type::pointer(),
                self.type_handle.libffi_type(),
                Type::pointer(),
            ],
            Type::i32(),
        );

        let mut out: *mut std::ffi::c_void = std::ptr::null_mut();
        let data_ref = unsafe { &*self.ptr };
        let hr: windows_core::HRESULT = unsafe {
            cif.call(
                CodePtr(fptr),
                &[arg(&obj_raw), arg(data_ref), arg(&(&mut out))],
            )
        };
        if hr.is_err() {
            if !out.is_null() {
                drop(unsafe { windows_core::IUnknown::from_raw(out) });
            }
            hr.ok()?;
        }
        if out.is_null() {
            return Err(windows_core::Error::from_hresult(windows_core::HRESULT(
                0x80004003u32 as i32,
            )));
        }
        Ok(unsafe { windows_core::IUnknown::from_raw(out as _) })
    }
}

impl Drop for ValueTypeData {
    fn drop(&mut self) {
        let layout = self.type_handle.layout();
        if layout.size() > 0 {
            // Release non-blittable fields before freeing the buffer
            if has_non_blittable_fields(&self.type_handle) {
                unsafe { release_non_blittable_fields(&self.type_handle, self.ptr) };
            }
            unsafe { std::alloc::dealloc(self.ptr, layout) }
        }
    }
}

impl Clone for ValueTypeData {
    fn clone(&self) -> Self {
        let layout = self.type_handle.layout();
        if layout.size() == 0 {
            return Self {
                type_handle: self.type_handle.clone(),
                ptr: std::ptr::null_mut(),
            };
        }
        let ptr = unsafe {
            let p = std::alloc::alloc(layout);
            std::ptr::copy_nonoverlapping(self.ptr, p, layout.size());
            // Duplicate non-blittable fields so both copies are valid
            if has_non_blittable_fields(&self.type_handle) {
                duplicate_non_blittable_fields(&self.type_handle, p);
            }
            p
        };
        Self {
            type_handle: self.type_handle.clone(),
            ptr,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata_table::MetadataTable;
    use crate::metadata_table::TypeKind;

    #[test]
    fn hstring_field_round_trips_overwrites_and_clones() {
        let table = MetadataTable::new();
        let typ = table.struct_type("Test.HStringField", &[table.i32_type(), table.hstring()]);
        let mut value = typ.default_value();

        value.set_field_hstring(1, HSTRING::from("first")).unwrap();
        assert_eq!(value.get_field_hstring(1).unwrap(), "first");

        value.set_field_hstring(1, HSTRING::from("second")).unwrap();
        assert_eq!(value.get_field_hstring(1).unwrap(), "second");

        let clone = value.clone();
        drop(value);
        assert_eq!(clone.get_field_hstring(1).unwrap(), "second");
    }

    #[test]
    fn hstring_field_access_rejects_other_types() {
        let table = MetadataTable::new();
        let typ = table.struct_type("Test.NotHString", &[table.i32_type()]);
        let mut value = typ.default_value();

        assert!(value.get_field_hstring(0).is_err());
        assert!(value.set_field_hstring(0, HSTRING::from("wrong")).is_err());
    }

    #[test]
    fn checked_field_access_reports_invalid_indices_and_non_struct_values() {
        let table = MetadataTable::new();
        let typ = table.struct_type("Test.CheckedFieldAccess", &[table.i32_type()]);
        let value = typ.default_value();

        assert!(matches!(
            value.field_type_checked(1),
            Err(crate::result::Error::IndexOutOfBounds { index: 1, len: 1 })
        ));
        assert!(matches!(
            value.get_field_struct_checked(0),
            Err(crate::result::Error::ExpectStructTypeError(TypeKind::I32))
        ));

        let non_struct = table.i32_type().default_value();
        assert!(matches!(
            non_struct.field_type_checked(0),
            Err(crate::result::Error::ExpectStructTypeError(TypeKind::I32))
        ));
    }
}
