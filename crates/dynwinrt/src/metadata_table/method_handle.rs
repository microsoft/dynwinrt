// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::sync::Arc;

use crate::value::WinRTValue;

use super::MetadataTable;

/// A handle to a pre-built method in the MetadataTable's methods arena.
#[derive(Clone)]
pub struct MethodHandle {
    pub(crate) table: Arc<MetadataTable>,
    pub(crate) index: u32,
}

impl std::fmt::Debug for MethodHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MethodHandle")
            .field("index", &self.index)
            .finish()
    }
}

impl MethodHandle {
    pub(crate) fn new(table: Arc<MetadataTable>, index: u32) -> Self {
        MethodHandle { table, index }
    }

    /// Invoke this method on the given COM object with the provided arguments.
    pub fn invoke(
        &self,
        obj: *mut std::ffi::c_void,
        args: &[WinRTValue],
    ) -> crate::result::Result<Vec<WinRTValue>> {
        self.table
            .invoke_method(self.index, obj, args)
            .map_err(crate::result::Error::WindowsError)
    }

    // --- Fast getter paths: zero Vec/WinRTValue allocation ---
    //
    // Each path grabs a stable Method pointer under the read lock via
    // `method_ptr`, drops the guard, and then makes the COM call. See the
    // safety comment on `MetadataTable::methods` for why this is sound.

    pub fn call_getter_i32(&self, obj: *mut std::ffi::c_void) -> crate::result::Result<i32> {
        let method_ptr = self.table.method_ptr(self.index);
        unsafe { (*method_ptr).call_getter_i32(obj) }.map_err(crate::result::Error::WindowsError)
    }

    pub fn call_getter_bool(&self, obj: *mut std::ffi::c_void) -> crate::result::Result<bool> {
        let method_ptr = self.table.method_ptr(self.index);
        unsafe { (*method_ptr).call_getter_bool(obj) }.map_err(crate::result::Error::WindowsError)
    }

    pub fn call_getter_hstring(
        &self,
        obj: *mut std::ffi::c_void,
    ) -> crate::result::Result<windows_core::HSTRING> {
        let method_ptr = self.table.method_ptr(self.index);
        unsafe { (*method_ptr).call_getter_hstring(obj) }
            .map_err(crate::result::Error::WindowsError)
    }

    pub fn call_getter_object(
        &self,
        obj: *mut std::ffi::c_void,
    ) -> crate::result::Result<WinRTValue> {
        let method_ptr = self.table.method_ptr(self.index);
        unsafe { (*method_ptr).call_getter_object(obj) }.map_err(crate::result::Error::WindowsError)
    }

    pub fn call_setter_hstring(
        &self,
        obj: *mut std::ffi::c_void,
        value: &windows_core::HSTRING,
    ) -> crate::result::Result<()> {
        let method_ptr = self.table.method_ptr(self.index);
        unsafe { (*method_ptr).call_setter_hstring(obj, value) }
            .map_err(crate::result::Error::WindowsError)
    }

    pub fn call_setter_bool(
        &self,
        obj: *mut std::ffi::c_void,
        value: bool,
    ) -> crate::result::Result<()> {
        let method_ptr = self.table.method_ptr(self.index);
        unsafe { (*method_ptr).call_setter_bool(obj, value) }
            .map_err(crate::result::Error::WindowsError)
    }

    pub fn call_setter_i32(
        &self,
        obj: *mut std::ffi::c_void,
        value: i32,
    ) -> crate::result::Result<()> {
        let method_ptr = self.table.method_ptr(self.index);
        unsafe { (*method_ptr).call_setter_i32(obj, value) }
            .map_err(crate::result::Error::WindowsError)
    }

    pub fn call_setter_u32(
        &self,
        obj: *mut std::ffi::c_void,
        value: u32,
    ) -> crate::result::Result<()> {
        let method_ptr = self.table.method_ptr(self.index);
        unsafe { (*method_ptr).call_setter_u32(obj, value) }
            .map_err(crate::result::Error::WindowsError)
    }

    pub fn call_setter_f32(
        &self,
        obj: *mut std::ffi::c_void,
        value: f32,
    ) -> crate::result::Result<()> {
        let method_ptr = self.table.method_ptr(self.index);
        unsafe { (*method_ptr).call_setter_f32(obj, value) }
            .map_err(crate::result::Error::WindowsError)
    }

    pub fn call_setter_f64(
        &self,
        obj: *mut std::ffi::c_void,
        value: f64,
    ) -> crate::result::Result<()> {
        let method_ptr = self.table.method_ptr(self.index);
        unsafe { (*method_ptr).call_setter_f64(obj, value) }
            .map_err(crate::result::Error::WindowsError)
    }
}
