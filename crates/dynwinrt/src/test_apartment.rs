// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::sync::Once;
use windows::Win32::System::Com::CoIncrementMTAUsage;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

pub(crate) fn initialize_mta() {
    static PROCESS_MTA: Once = Once::new();
    PROCESS_MTA.call_once(|| {
        // Retain this native MTA usage until process exit: SDK factory caches outlive
        // libtest worker threads, so the last worker must not tear down their MTA.
        let _cookie = unsafe { CoIncrementMTAUsage() }.expect("keep the test-process MTA alive");
    });
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.expect("initialize the test-thread MTA");
}
