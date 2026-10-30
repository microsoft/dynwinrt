// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use dynwinrt::has_package_identity;
use windows::Win32::Foundation::CO_E_NOTINITIALIZED;
use windows::Win32::System::Com::{APTTYPE, APTTYPEQUALIFIER, CoGetApartmentType};

// A new thread still sees the core unit tests' process-lifetime implicit MTA.
// Keep this strict uninitialized-apartment check in its own executable.
#[test]
fn identity_query_does_not_initialize_com() {
    let mut apartment = APTTYPE::default();
    let mut qualifier = APTTYPEQUALIFIER::default();
    let before = unsafe { CoGetApartmentType(&mut apartment, &mut qualifier) }.unwrap_err();
    assert_eq!(before.code(), CO_E_NOTINITIALIZED);
    has_package_identity().unwrap();
    let after = unsafe { CoGetApartmentType(&mut apartment, &mut qualifier) }.unwrap_err();
    assert_eq!(after.code(), before.code());
}
