// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use core::ffi::c_void;
use std::cell::RefCell;

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows_core::{GUID, IUnknown, Interface};

use crate::{MethodSignature, WinRTValue, result};

const RPC_E_CHANGED_MODE: windows_core::HRESULT = windows_core::HRESULT(0x80010106u32 as i32);

struct ComApartment;

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

enum ComInitialization {
    Unknown,
    Owned(ComApartment),
    ExistingApartment,
}

thread_local! {
    static COM_INITIALIZATION: RefCell<ComInitialization> =
        const { RefCell::new(ComInitialization::Unknown) };
}

pub fn ensure_com_initialized() -> result::Result<()> {
    COM_INITIALIZATION.with(|state| {
        if !matches!(*state.borrow(), ComInitialization::Unknown) {
            return Ok(());
        }

        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr.is_ok() {
            *state.borrow_mut() = ComInitialization::Owned(ComApartment);
            Ok(())
        } else if hr == RPC_E_CHANGED_MODE {
            *state.borrow_mut() = ComInitialization::ExistingApartment;
            Ok(())
        } else {
            Err(result::Error::WindowsError(
                windows_core::Error::from_hresult(hr),
            ))
        }
    })
}

pub fn co_create_instance(clsid: GUID, iid: GUID) -> result::Result<WinRTValue> {
    ensure_com_initialized()?;

    let unknown: IUnknown = unsafe { CoCreateInstance(&clsid, None, CLSCTX_INPROC_SERVER) }
        .map_err(result::Error::WindowsError)?;
    let mut result = std::ptr::null_mut();
    unsafe { unknown.query(&iid, &mut result) }
        .ok()
        .map_err(result::Error::WindowsError)?;
    Ok(WinRTValue::Object(unsafe { IUnknown::from_raw(result) }))
}

pub fn call_method(
    vtable_index: usize,
    obj: *mut c_void,
    signature: MethodSignature,
    args: &[WinRTValue],
) -> result::Result<Vec<WinRTValue>> {
    signature
        .build(vtable_index)
        .call_dynamic(obj, args)
        .map_err(result::Error::WindowsError)
}

#[cfg(test)]
fn call_method_1_ptr(
    vtable_index: usize,
    obj: *mut c_void,
    ptr: *const c_void,
) -> result::Result<()> {
    crate::call::call_winrt_method_1(vtable_index, obj, ptr)
        .ok()
        .map_err(result::Error::WindowsError)
}

#[cfg(test)]
fn call_method_2_ptr_i32(
    vtable_index: usize,
    obj: *mut c_void,
    ptr: *mut c_void,
    value: i32,
) -> result::Result<()> {
    crate::call::call_winrt_method_2(vtable_index, obj, ptr, value)
        .ok()
        .map_err(result::Error::WindowsError)
}

#[cfg(test)]
fn wide_null(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
fn wide_buffer(characters: usize) -> Vec<u16> {
    vec![0; characters]
}

#[cfg(test)]
fn wide_to_string(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|ch| *ch == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        InterfaceSignature, MetadataTable, com_helpers::E_NOINTERFACE, ro_get_activation_factory_2,
        roapi::query_interface,
    };
    use windows::{
        ApplicationModel::DataTransfer::DataTransferManager,
        Win32::{
            UI::Shell::IDataTransferManagerInterop,
            UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPED,
            },
        },
    };
    use windows_core::{HSTRING, Interface, w};

    const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
    const IID_ISHELL_LINK_W: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
    const REGDB_E_CLASSNOTREG: windows_core::HRESULT = windows_core::HRESULT(0x80040154u32 as i32);

    fn shell_link() -> result::Result<WinRTValue> {
        co_create_instance(CLSID_SHELL_LINK, IID_ISHELL_LINK_W)
    }

    fn shell_link_signature(table: &std::sync::Arc<MetadataTable>) -> InterfaceSignature {
        let mut iface =
            InterfaceSignature::define_from_iunknown("IShellLinkW", IID_ISHELL_LINK_W, table);
        iface
            .add_method(MethodSignature::new(table)) // 3 GetPath
            .add_method(MethodSignature::new(table)) // 4 GetIDList
            .add_method(MethodSignature::new(table)) // 5 SetIDList
            .add_method(MethodSignature::new(table)) // 6 GetDescription
            .add_method(MethodSignature::new(table)) // 7 SetDescription
            .add_method(MethodSignature::new(table)) // 8 GetWorkingDirectory
            .add_method(MethodSignature::new(table)) // 9 SetWorkingDirectory
            .add_method(MethodSignature::new(table)) // 10 GetArguments
            .add_method(MethodSignature::new(table)) // 11 SetArguments
            .add_method(MethodSignature::new(table).add_out(table.u16_type())) // 12 GetHotkey
            .add_method(MethodSignature::new(table).add_in(table.u16_type())) // 13 SetHotkey
            .add_method(MethodSignature::new(table).add_out(table.i32_type())) // 14 GetShowCmd
            .add_method(MethodSignature::new(table).add_in(table.i32_type())); // 15 SetShowCmd
        iface
    }

    #[test]
    fn shell_link_set_get_show_cmd_round_trips_via_classic_com_vtable() -> result::Result<()> {
        let shell_link = shell_link()?.as_object().unwrap();
        let table = MetadataTable::new();
        let iface = shell_link_signature(&table);

        iface.methods[15].call_dynamic(shell_link.as_raw(), &[WinRTValue::I32(3)])?;
        let result = iface.methods[14].call_dynamic(shell_link.as_raw(), &[])?;

        assert_eq!(result[0].as_i32().unwrap(), 3);
        Ok(())
    }

    #[test]
    fn shell_link_set_get_hotkey_round_trips_u16() -> result::Result<()> {
        let shell_link = shell_link()?.as_object().unwrap();
        let table = MetadataTable::new();
        let iface = shell_link_signature(&table);

        iface.methods[13].call_dynamic(shell_link.as_raw(), &[WinRTValue::U16(0x0141)])?;
        let result = iface.methods[12].call_dynamic(shell_link.as_raw(), &[])?;

        assert_eq!(result[0].as_i32().unwrap() as u16, 0x0141);
        Ok(())
    }

    #[test]
    fn shell_link_set_get_description_round_trips_wide_string() -> result::Result<()> {
        let shell_link = shell_link()?.as_object().unwrap();
        let expected = "dynwinrt classic COM";
        let wide = wide_null(expected);

        call_method_1_ptr(7, shell_link.as_raw(), wide.as_ptr() as *const c_void)?;

        let mut buffer = wide_buffer(128);
        call_method_2_ptr_i32(
            6,
            shell_link.as_raw(),
            buffer.as_mut_ptr() as *mut c_void,
            buffer.len() as i32,
        )?;

        assert_eq!(wide_to_string(&buffer), expected);
        Ok(())
    }

    #[test]
    fn co_create_instance_with_bogus_clsid_returns_error() -> result::Result<()> {
        let bogus = GUID::from_u128(0xaaaaaaaa_bbbb_cccc_dddd_eeeeeeeeeeee);

        let err = co_create_instance(bogus, IID_ISHELL_LINK_W).unwrap_err();
        match err {
            result::Error::WindowsError(err) => assert_eq!(err.code(), REGDB_E_CLASSNOTREG),
            err => panic!("expected REGDB_E_CLASSNOTREG, got {err:?}"),
        }
        Ok(())
    }

    #[test]
    fn query_interface_with_unsupported_iid_returns_error() -> result::Result<()> {
        let shell_link = shell_link()?;
        let bogus = GUID::from_u128(0xbbbbbbbb_cccc_dddd_eeee_ffffffffffff);

        let err = shell_link.cast(&bogus).unwrap_err();
        match err {
            result::Error::WindowsError(err) => assert_eq!(err.code(), E_NOINTERFACE),
            err => panic!("expected E_NOINTERFACE, got {err:?}"),
        }
        Ok(())
    }

    #[test]
    fn data_transfer_manager_interop_get_for_window_returns_winrt_object_via_dynamic_iunknown_vtable()
    -> result::Result<()> {
        ensure_com_initialized()?;

        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("dynwinrt data transfer interop test"),
                WS_OVERLAPPED,
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
        }
        .map_err(result::Error::WindowsError)?;
        struct WindowGuard(windows::Win32::Foundation::HWND);
        impl Drop for WindowGuard {
            fn drop(&mut self) {
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }
        let _window = WindowGuard(hwnd);

        let factory = ro_get_activation_factory_2(&HSTRING::from(
            "Windows.ApplicationModel.DataTransfer.DataTransferManager",
        ))?;
        let interop = query_interface(factory, &IDataTransferManagerInterop::IID)
            .map_err(result::Error::WindowsError)?
            .as_object()
            .unwrap();

        let table = MetadataTable::new();
        let mut iface = InterfaceSignature::define_from_iunknown(
            "IDataTransferManagerInterop",
            IDataTransferManagerInterop::IID,
            &table,
        );
        iface.add_method(
            MethodSignature::new(&table)
                .add_in(table.object())
                .add_in(table.object())
                .add_out(table.object()),
        );

        let target_iid = DataTransferManager::IID;
        let result = iface.methods[3].call_dynamic(
            interop.as_raw(),
            &[
                WinRTValue::RawPtr(hwnd.0 as *mut c_void),
                WinRTValue::RawPtr(&target_iid as *const GUID as *mut c_void),
            ],
        )?;

        let manager = result[0].as_object().expect("GetForWindow returned null");
        assert!(!manager.as_raw().is_null());
        let _typed: DataTransferManager = manager.cast().map_err(result::Error::WindowsError)?;
        Ok(())
    }
}
