// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::sync::Arc;

use crate::value::WinRTValue;

use super::MetadataTable;

/// An owning, bound method handle sharing its immutable prepared ABI plan.
#[derive(Clone)]
pub struct MethodHandle {
    // Keep even zero-parameter methods' registry alive.
    _table: Arc<MetadataTable>,
    pub(crate) index: u32,
    method: Arc<crate::signature::Method>,
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
        let descriptor = table.method_ptr(index);
        // The append-only descriptor cannot move or be removed while table is alive.
        let method = Arc::new(unsafe { (*descriptor).bind(&table) });
        MethodHandle {
            _table: table,
            index,
            method,
        }
    }

    /// Invoke this method on the given COM object with the provided arguments.
    pub fn invoke(
        &self,
        obj: *mut std::ffi::c_void,
        args: &[WinRTValue],
    ) -> crate::result::Result<Vec<WinRTValue>> {
        self.method
            .call_dynamic(obj, args)
            .map_err(crate::result::Error::WindowsError)
    }

    // --- Fast getter paths: zero Vec/WinRTValue allocation ---

    pub fn call_getter_i32(&self, obj: *mut std::ffi::c_void) -> crate::result::Result<i32> {
        self.method
            .call_getter_i32(obj)
            .map_err(crate::result::Error::WindowsError)
    }

    pub fn call_getter_bool(&self, obj: *mut std::ffi::c_void) -> crate::result::Result<bool> {
        self.method
            .call_getter_bool(obj)
            .map_err(crate::result::Error::WindowsError)
    }

    pub fn call_getter_hstring(
        &self,
        obj: *mut std::ffi::c_void,
    ) -> crate::result::Result<windows_core::HSTRING> {
        self.method
            .call_getter_hstring(obj)
            .map_err(crate::result::Error::WindowsError)
    }

    pub fn call_getter_object(
        &self,
        obj: *mut std::ffi::c_void,
    ) -> crate::result::Result<WinRTValue> {
        self.method
            .call_getter_object(obj)
            .map_err(crate::result::Error::WindowsError)
    }

    pub fn call_setter_hstring(
        &self,
        obj: *mut std::ffi::c_void,
        value: &windows_core::HSTRING,
    ) -> crate::result::Result<()> {
        self.method
            .call_setter_hstring(obj, value)
            .map_err(crate::result::Error::WindowsError)
    }

    pub fn call_setter_bool(
        &self,
        obj: *mut std::ffi::c_void,
        value: bool,
    ) -> crate::result::Result<()> {
        self.method
            .call_setter_bool(obj, value)
            .map_err(crate::result::Error::WindowsError)
    }

    pub fn call_setter_i32(
        &self,
        obj: *mut std::ffi::c_void,
        value: i32,
    ) -> crate::result::Result<()> {
        self.method
            .call_setter_i32(obj, value)
            .map_err(crate::result::Error::WindowsError)
    }

    pub fn call_setter_u32(
        &self,
        obj: *mut std::ffi::c_void,
        value: u32,
    ) -> crate::result::Result<()> {
        self.method
            .call_setter_u32(obj, value)
            .map_err(crate::result::Error::WindowsError)
    }

    pub fn call_setter_f32(
        &self,
        obj: *mut std::ffi::c_void,
        value: f32,
    ) -> crate::result::Result<()> {
        self.method
            .call_setter_f32(obj, value)
            .map_err(crate::result::Error::WindowsError)
    }

    pub fn call_setter_f64(
        &self,
        obj: *mut std::ffi::c_void,
        value: f64,
    ) -> crate::result::Result<()> {
        self.method
            .call_setter_f64(obj, value)
            .map_err(crate::result::Error::WindowsError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MethodSignature, TypeHandle, native_call::PreparedCall};
    use windows::Foundation::IUriRuntimeClassFactory;
    use windows_core::Interface;

    #[test]
    fn bound_handles_share_and_release_prepared_calls() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<PreparedCall>();
        assert_send_sync::<MetadataTable>();
        assert_send_sync::<TypeHandle>();
        assert_send_sync::<MethodHandle>();
        assert_send_sync::<crate::signature::Method>();

        let table = MetadataTable::new();
        let weak_table = Arc::downgrade(&table);
        let interface = table
            .register_interface("IUriRuntimeClassFactory", IUriRuntimeClassFactory::IID)
            .add_method(
                "CreateUri",
                MethodSignature::new(&table)
                    .add_in(table.hstring())
                    .add_out(table.object()),
            );
        let first = interface.method(6).unwrap();
        let clone = first.clone();
        let second = interface.method_by_name("CreateUri").unwrap();
        assert!(Arc::ptr_eq(&first.method, &clone.method));
        assert!(!Arc::ptr_eq(&first.method, &second.method));
        assert!(Arc::ptr_eq(
            first.method.prepared_call(),
            second.method.prepared_call()
        ));
        let weak_plan = Arc::downgrade(first.method.prepared_call());
        drop(interface);
        drop(table);
        drop(first);
        drop(second);
        assert!(weak_table.upgrade().is_some());
        assert!(weak_plan.upgrade().is_some());
        drop(clone);
        assert!(weak_table.upgrade().is_none());
        assert!(weak_plan.upgrade().is_none());
    }
}
