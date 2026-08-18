// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::mem::{align_of, size_of};

use windows::Win32::Foundation::{FILETIME, HANDLE, RECT, SYSTEMTIME};

#[test]
fn generated_windows_types_match_stock_windows_abi() {
    assert_eq!(size_of::<HANDLE>(), size_of::<usize>());
    assert_eq!(align_of::<HANDLE>(), align_of::<usize>());
    assert_eq!(size_of::<FILETIME>(), 8);
    assert_eq!(align_of::<FILETIME>(), 4);
    assert_eq!(size_of::<RECT>(), 16);
    assert_eq!(align_of::<RECT>(), 4);
    assert_eq!(size_of::<SYSTEMTIME>(), 16);
    assert_eq!(align_of::<SYSTEMTIME>(), 2);
}

#[test]
fn generated_cdecl_oracle_is_callable() {
    let result = unsafe { windows::Win32::Networking::Ldap::LdapGetLastError() };
    let _: u32 = result;
}
