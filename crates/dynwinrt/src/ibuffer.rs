// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::ffi::c_void;

use windows::Storage::Streams::{Buffer, IBuffer};
use windows_core::{GUID, HRESULT, IUnknown, Interface};

use crate::{Error, Result, WinRTValue};

const IID_IBUFFER_BYTE_ACCESS: GUID = GUID::from_u128(0x905a0fef_bc53_11df_8c49_001e4fc686da);

#[repr(C)]
struct IBufferByteAccessVtbl {
    base__: windows_core::IUnknown_Vtbl,
    buffer: unsafe extern "system" fn(*mut c_void, *mut *mut u8) -> HRESULT,
}

fn validate_bounds(length: u32, capacity: u32) -> Result<usize> {
    if length > capacity {
        return Err(Error::InvalidIBufferBounds { length, capacity });
    }
    Ok(length as usize)
}

fn query_byte_pointer(owner: &IBuffer) -> Result<(IUnknown, *mut u8)> {
    let owner: IUnknown = owner.cast().map_err(Error::WindowsError)?;
    let mut raw_access = std::ptr::null_mut();
    unsafe { owner.query(&IID_IBUFFER_BYTE_ACCESS, &mut raw_access) }
        .ok()
        .map_err(Error::WindowsError)?;
    let access = unsafe { IUnknown::from_raw(raw_access) };
    let vtable = unsafe { *(access.as_raw() as *const *const IBufferByteAccessVtbl) };
    let mut bytes = std::ptr::null_mut();
    unsafe { ((*vtable).buffer)(access.as_raw(), &mut bytes) }
        .ok()
        .map_err(Error::WindowsError)?;
    Ok((access, bytes))
}

fn copy_borrowed_bytes(bytes: *const u8, length: usize) -> Result<Vec<u8>> {
    if length == 0 {
        return Ok(Vec::new());
    }
    if bytes.is_null() {
        return Err(Error::NullIBufferPointer { length });
    }

    let mut copy = vec![0; length];
    unsafe { std::ptr::copy_nonoverlapping(bytes, copy.as_mut_ptr(), length) };
    Ok(copy)
}

/// Copy the initialized bytes from a WinRT `Windows.Storage.Streams.IBuffer`.
///
/// The native byte pointer is borrowed from the buffer and remains internal to
/// this function. The owning `IBuffer` and its `IBufferByteAccess` interface are
/// retained until the copy completes.
pub fn copy_from_ibuffer(value: &WinRTValue) -> Result<Vec<u8>> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::ExpectedIBuffer(value.get_type_kind()))?;
    let buffer: IBuffer = object.cast().map_err(Error::WindowsError)?;
    let capacity = buffer.Capacity().map_err(Error::WindowsError)?;
    let length = buffer.Length().map_err(Error::WindowsError)?;
    let length = validate_bounds(length, capacity)?;
    let (_access, bytes) = query_byte_pointer(&buffer)?;
    copy_borrowed_bytes(bytes, length)
}

/// Create an owned WinRT `IBuffer` whose `Length` and `Capacity` equal the
/// number of copied input bytes.
pub fn copy_to_ibuffer(bytes: &[u8]) -> Result<WinRTValue> {
    let length =
        u32::try_from(bytes.len()).map_err(|_| Error::IBufferInputTooLarge(bytes.len()))?;
    let buffer = Buffer::Create(length).map_err(Error::WindowsError)?;
    let ibuffer: IBuffer = buffer.cast().map_err(Error::WindowsError)?;
    let (_access, destination) = query_byte_pointer(&ibuffer)?;

    if !bytes.is_empty() {
        if destination.is_null() {
            return Err(Error::NullIBufferPointer {
                length: bytes.len(),
            });
        }
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len()) };
    }
    ibuffer.SetLength(length).map_err(Error::WindowsError)?;

    debug_assert_eq!(ibuffer.Length().ok(), Some(length));
    debug_assert_eq!(ibuffer.Capacity().ok(), Some(length));
    Ok(WinRTValue::Object(
        ibuffer.cast().map_err(Error::WindowsError)?,
    ))
}

#[cfg(test)]
mod tests {
    use windows::Foundation::Uri;

    use super::*;

    #[test]
    fn validates_length_does_not_exceed_capacity() {
        assert_eq!(validate_bounds(0, 0).unwrap(), 0);
        assert_eq!(validate_bounds(3, 4).unwrap(), 3);
        assert!(matches!(
            validate_bounds(5, 4),
            Err(Error::InvalidIBufferBounds {
                length: 5,
                capacity: 4
            })
        ));
    }

    #[test]
    fn handles_null_borrowed_pointers_without_dereferencing_them() {
        assert_eq!(
            copy_borrowed_bytes(std::ptr::null(), 0).unwrap(),
            Vec::<u8>::new()
        );
        assert!(matches!(
            copy_borrowed_bytes(std::ptr::null(), 1),
            Err(Error::NullIBufferPointer { length: 1 })
        ));
    }

    #[test]
    fn copies_empty_and_binary_buffers() -> Result<()> {
        let empty = copy_to_ibuffer(&[])?;
        assert_eq!(copy_from_ibuffer(&empty)?, Vec::<u8>::new());

        let expected = vec![0, 1, 2, 0, 0xff, 0x80, 3];
        let buffer = copy_to_ibuffer(&expected)?;
        assert_eq!(copy_from_ibuffer(&buffer)?, expected);
        Ok(())
    }

    #[test]
    fn returned_copy_outlives_owner() -> Result<()> {
        let buffer = copy_to_ibuffer(&[9, 8, 7, 0])?;
        let bytes = copy_from_ibuffer(&buffer)?;
        drop(buffer);
        assert_eq!(bytes, [9, 8, 7, 0]);
        Ok(())
    }

    #[test]
    fn input_is_copied() -> Result<()> {
        let mut input = vec![1, 2, 3];
        let buffer = copy_to_ibuffer(&input)?;
        input.fill(9);
        assert_eq!(copy_from_ibuffer(&buffer)?, [1, 2, 3]);
        Ok(())
    }

    #[test]
    fn rejects_non_ibuffer_objects() {
        let uri = Uri::CreateUri(&"https://example.com".into()).unwrap();
        let value = WinRTValue::Object(uri.cast().unwrap());
        let error = copy_from_ibuffer(&value).unwrap_err();
        assert!(matches!(error, Error::WindowsError(_)));
        assert!(error.message().contains("0x80004002"));
    }

    #[test]
    fn rejects_non_object_values() {
        let error = copy_from_ibuffer(&WinRTValue::U8(1)).unwrap_err();
        assert!(matches!(error, Error::ExpectedIBuffer(_)));
    }
}
