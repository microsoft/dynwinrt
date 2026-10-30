// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use dynwinrt::{copy_from_ibuffer, copy_to_ibuffer};
use windows::Storage::Streams::IBuffer;
use windows::Win32::Foundation::CO_E_NOTINITIALIZED;
use windows::Win32::System::Com::{APTTYPE, APTTYPEQUALIFIER, CoGetApartmentType};
use windows::Win32::System::WinRT::{
    RO_INIT_MULTITHREADED, RO_INIT_SINGLETHREADED, RoInitialize, RoUninitialize,
};
use windows_core::Interface;

struct Apartment;

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

fn assert_uninitialized() {
    let mut apartment = APTTYPE::default();
    let mut qualifier = APTTYPEQUALIFIER::default();
    let error = unsafe { CoGetApartmentType(&mut apartment, &mut qualifier) }.unwrap_err();
    assert_eq!(error.code(), CO_E_NOTINITIALIZED);
}

// This executable must not share the unit tests' process-lifetime MTA.
#[test]
fn copied_buffers_survive_complete_apartment_teardown() -> dynwinrt::Result<()> {
    assert_uninitialized();
    for model in [
        RO_INIT_MULTITHREADED,
        RO_INIT_MULTITHREADED,
        RO_INIT_SINGLETHREADED,
        RO_INIT_SINGLETHREADED,
        RO_INIT_MULTITHREADED,
    ] {
        {
            unsafe { RoInitialize(model) }?;
            let _apartment = Apartment;
            for bytes in [b"abc".as_slice(), &[], &[0, 1, 0xff, 0x80, 0]] {
                let value = copy_to_ibuffer(bytes)?;
                let buffer: IBuffer = value.as_object().unwrap().cast()?;
                assert_eq!(buffer.Length()?, bytes.len() as u32);
                assert_eq!(buffer.Capacity()?, bytes.len() as u32);
                let copied = copy_from_ibuffer(&value)?;
                drop(buffer);
                drop(value);
                assert_eq!(copied, bytes);
            }
        }
        assert_uninitialized();
    }
    Ok(())
}
