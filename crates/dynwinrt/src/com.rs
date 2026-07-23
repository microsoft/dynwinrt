// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use core::ffi::c_void;
use std::cell::RefCell;

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoCreateInstance,
    CoInitializeEx, CoUninitialize,
};
use windows_core::{GUID, IUnknown, Interface as WindowsInterface};

use crate::{
    MetadataTable, MethodHandle, TypeHandle, WinRTValue, result,
    signature::{AbiMethodSignature, ParameterType},
};

const RPC_E_CHANGED_MODE: windows_core::HRESULT = windows_core::HRESULT(0x80010106u32 as i32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceBase {
    IUnknown,
    IInspectable,
}

impl InterfaceBase {
    pub const fn first_method_slot(self) -> usize {
        match self {
            Self::IUnknown => 3,
            Self::IInspectable => 6,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Type(ParameterType);

impl Type {
    pub fn winrt(typ: TypeHandle) -> Self {
        Self(ParameterType::winrt(typ))
    }

    pub fn pointer() -> Self {
        Self(ParameterType::pointer())
    }
}

#[derive(Debug, Clone)]
pub struct MethodSignature(AbiMethodSignature);

impl MethodSignature {
    pub fn new(table: &std::sync::Arc<MetadataTable>) -> Self {
        Self(AbiMethodSignature::new(table))
    }

    pub fn add_in(self, typ: Type) -> Self {
        Self(self.0.add_in_type(typ.0))
    }

    pub fn add_out(self, typ: Type) -> Self {
        Self(self.0.add_out_type(typ.0))
    }

    pub fn add_in_out(self, typ: Type) -> Self {
        Self(self.0.add_in_out_type(typ.0))
    }

    pub fn add_out_fill(self, typ: Type) -> Self {
        Self(self.0.add_out_fill_type(typ.0))
    }

    pub fn returns(self, typ: Type) -> Self {
        Self(self.0.returns_type(typ.0))
    }

    pub fn returns_void(self) -> Self {
        Self(self.0.returns_void())
    }
}

#[derive(Debug, Clone)]
pub struct Interface(TypeHandle);

impl Interface {
    pub fn add_method(self, name: &str, signature: MethodSignature) -> Self {
        self.0
            .clone()
            .add_method(name, crate::MethodSignature::from_abi(signature.0));
        self
    }

    pub fn method(&self, vtable_index: usize) -> Option<MethodHandle> {
        self.0.method(vtable_index)
    }
}

pub fn register_interface(
    table: &std::sync::Arc<MetadataTable>,
    name: &str,
    iid: GUID,
    base: InterfaceBase,
) -> Interface {
    Interface(table.register_com_interface(name, iid, base.first_method_slot()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApartmentType {
    SingleThreaded,
    MultiThreaded,
}

impl ApartmentType {
    fn as_flag(self) -> windows::Win32::System::Com::COINIT {
        match self {
            Self::SingleThreaded => COINIT_APARTMENTTHREADED,
            Self::MultiThreaded => COINIT_MULTITHREADED,
        }
    }
}

struct ComApartment {
    apartment_type: ApartmentType,
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

enum ComInitialization {
    Uninitialized,
    Owned(ComApartment),
}

thread_local! {
    static COM_INITIALIZATION: RefCell<ComInitialization> =
        const { RefCell::new(ComInitialization::Uninitialized) };
}

pub fn initialize_apartment(apartment_type: ApartmentType) -> result::Result<()> {
    COM_INITIALIZATION.with(|state| {
        if let ComInitialization::Owned(existing) = &*state.borrow() {
            return if existing.apartment_type == apartment_type {
                Ok(())
            } else {
                Err(result::Error::WindowsError(
                    windows_core::Error::from_hresult(RPC_E_CHANGED_MODE),
                ))
            };
        }

        let hr = unsafe { CoInitializeEx(None, apartment_type.as_flag()) };
        if hr.is_ok() {
            *state.borrow_mut() = ComInitialization::Owned(ComApartment { apartment_type });
            Ok(())
        } else {
            Err(result::Error::WindowsError(
                windows_core::Error::from_hresult(hr),
            ))
        }
    })
}

pub fn co_create_instance(clsid: GUID, iid: GUID) -> result::Result<WinRTValue> {
    let unknown: IUnknown = unsafe { CoCreateInstance(&clsid, None, CLSCTX_INPROC_SERVER) }
        .map_err(result::Error::WindowsError)?;
    let mut result = std::ptr::null_mut();
    unsafe { unknown.query(&iid, &mut result) }
        .ok()
        .map_err(result::Error::WindowsError)?;
    Ok(WinRTValue::Object(unsafe { IUnknown::from_raw(result) }))
}

/// Adopt an AddRef-owned COM interface pointer into a managed Object value.
///
/// The pointer must represent a caller-owned COM reference (+1). This function
/// takes ownership with `IUnknown::from_raw` and must not be used for borrowed
/// pointers.
pub unsafe fn adopt_com_pointer(ptr: *mut c_void) -> WinRTValue {
    if ptr.is_null() {
        WinRTValue::Null
    } else {
        WinRTValue::Object(unsafe { IUnknown::from_raw(ptr) })
    }
}

pub fn call_method(
    vtable_index: usize,
    obj: *mut c_void,
    signature: MethodSignature,
    args: &[WinRTValue],
) -> result::Result<Vec<WinRTValue>> {
    signature
        .0
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
    use std::sync::atomic::{AtomicU32, Ordering};
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

    #[repr(C)]
    struct FakeComObject {
        vtable: *const *mut c_void,
    }

    unsafe extern "system" fn return_u32(_this: *mut c_void) -> u32 {
        u32::MAX
    }

    static VOID_CALLS: AtomicU32 = AtomicU32::new(0);

    unsafe extern "system" fn return_void(_this: *mut c_void) {
        VOID_CALLS.fetch_add(1, Ordering::Relaxed);
    }

    unsafe extern "system" fn increment_i32(
        _this: *mut c_void,
        value: *mut i32,
    ) -> windows_core::HRESULT {
        unsafe { *value += 1 };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn write_native_pointer(
        _this: *mut c_void,
        value: *mut *mut c_void,
    ) -> windows_core::HRESULT {
        unsafe { *value = 0x1234usize as *mut c_void };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn write_guid_and_i32(
        _this: *mut c_void,
        guid: *mut GUID,
        value: *mut i32,
    ) -> windows_core::HRESULT {
        unsafe {
            *guid = GUID::from_u128(0x11111111_2222_3333_4444_555555555555);
            *value = 42;
        }
        windows_core::HRESULT(0)
    }

    #[test]
    fn direct_native_return_is_not_interpreted_as_hresult() {
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table).returns(Type::winrt(table.u32_type()));
        let vtable = [return_u32 as *mut c_void];
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };

        let values = call_method(
            0,
            (&mut object as *mut FakeComObject).cast(),
            signature,
            &[],
        )
        .expect("u32::MAX is a value, not a failed HRESULT");

        assert!(matches!(values.as_slice(), [WinRTValue::U32(u32::MAX)]));
    }

    #[test]
    fn native_void_return_does_not_read_hresult_register() {
        VOID_CALLS.store(0, Ordering::Relaxed);
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table).returns_void();
        let vtable = [return_void as *mut c_void];
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };

        let values = call_method(
            0,
            (&mut object as *mut FakeComObject).cast(),
            signature,
            &[],
        )
        .unwrap();

        assert!(values.is_empty());
        assert_eq!(VOID_CALLS.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn in_out_parameter_preserves_input_and_returns_updated_value() {
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table).add_in_out(Type::winrt(table.i32_type()));
        let vtable = [increment_i32 as *mut c_void];
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };

        let values = call_method(
            0,
            (&mut object as *mut FakeComObject).cast(),
            signature,
            &[WinRTValue::I32(41)],
        )
        .unwrap();

        assert!(matches!(values.as_slice(), [WinRTValue::I32(42)]));
    }

    #[test]
    fn native_pointer_out_is_not_adopted_as_com_object() {
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table).add_out(Type::pointer());
        let vtable = [write_native_pointer as *mut c_void];
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };

        let values = call_method(
            0,
            (&mut object as *mut FakeComObject).cast(),
            signature,
            &[],
        )
        .unwrap();

        assert!(matches!(
            values.as_slice(),
            [WinRTValue::RawPtr(ptr)] if *ptr == 0x1234usize as *mut c_void
        ));
    }

    #[test]
    fn multi_output_guid_uses_full_sized_storage() {
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table)
            .add_out(Type::winrt(table.guid_type()))
            .add_out(Type::winrt(table.i32_type()));
        let vtable = [write_guid_and_i32 as *mut c_void];
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };

        let values = call_method(
            0,
            (&mut object as *mut FakeComObject).cast(),
            signature,
            &[],
        )
        .unwrap();

        assert!(matches!(
            values.as_slice(),
            [WinRTValue::Guid(guid), WinRTValue::I32(42)]
                if *guid == GUID::from_u128(0x11111111_2222_3333_4444_555555555555)
        ));
    }

    const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
    const IID_ISHELL_LINK_W: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
    const REGDB_E_CLASSNOTREG: windows_core::HRESULT = windows_core::HRESULT(0x80040154u32 as i32);

    fn shell_link() -> result::Result<WinRTValue> {
        initialize_apartment(ApartmentType::MultiThreaded)?;
        co_create_instance(CLSID_SHELL_LINK, IID_ISHELL_LINK_W)
    }

    #[test]
    #[should_panic(expected = "already registered with a different type or IID")]
    fn interface_names_cannot_alias_different_iids() {
        let table = MetadataTable::new();
        register_interface(
            &table,
            "Windows.Win32.Example.IThing",
            GUID::from_u128(1),
            InterfaceBase::IUnknown,
        );
        register_interface(
            &table,
            "Windows.Win32.Example.IThing",
            GUID::from_u128(2),
            InterfaceBase::IUnknown,
        );
    }

    fn shell_link_signature(table: &std::sync::Arc<MetadataTable>) -> InterfaceSignature {
        let mut iface =
            InterfaceSignature::define_from_iunknown("IShellLinkW", IID_ISHELL_LINK_W, table);
        iface
            .add_method(crate::MethodSignature::new(table)) // 3 GetPath
            .add_method(crate::MethodSignature::new(table)) // 4 GetIDList
            .add_method(crate::MethodSignature::new(table)) // 5 SetIDList
            .add_method(crate::MethodSignature::new(table)) // 6 GetDescription
            .add_method(crate::MethodSignature::new(table)) // 7 SetDescription
            .add_method(crate::MethodSignature::new(table)) // 8 GetWorkingDirectory
            .add_method(crate::MethodSignature::new(table)) // 9 SetWorkingDirectory
            .add_method(crate::MethodSignature::new(table)) // 10 GetArguments
            .add_method(crate::MethodSignature::new(table)) // 11 SetArguments
            .add_method(crate::MethodSignature::new(table).add_out(table.u16_type())) // 12 GetHotkey
            .add_method(crate::MethodSignature::new(table).add_in(table.u16_type())) // 13 SetHotkey
            .add_method(crate::MethodSignature::new(table).add_out(table.i32_type())) // 14 GetShowCmd
            .add_method(crate::MethodSignature::new(table).add_in(table.i32_type())); // 15 SetShowCmd
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
    fn adopt_com_pointer_accepts_addref_owned_pointer() -> result::Result<()> {
        let shell_link = shell_link()?.as_object().unwrap();
        let shell_link_raw = shell_link.as_raw();
        let borrowed = unsafe { IUnknown::from_raw_borrowed(&shell_link_raw) }.unwrap();
        let addref_owned = borrowed.clone();
        let raw = addref_owned.as_raw();
        std::mem::forget(addref_owned);

        let adopted = unsafe { adopt_com_pointer(raw) };
        let adopted = adopted.as_object().expect("adopted value must be Object");
        let table = MetadataTable::new();
        let iface = shell_link_signature(&table);

        iface.methods[15].call_dynamic(adopted.as_raw(), &[WinRTValue::I32(7)])?;
        let result = iface.methods[14].call_dynamic(adopted.as_raw(), &[])?;

        assert_eq!(result[0].as_i32().unwrap(), 7);
        Ok(())
    }

    #[test]
    fn co_create_instance_with_bogus_clsid_returns_error() -> result::Result<()> {
        initialize_apartment(ApartmentType::MultiThreaded)?;
        let bogus = GUID::from_u128(0xaaaaaaaa_bbbb_cccc_dddd_eeeeeeeeeeee);

        let err = co_create_instance(bogus, IID_ISHELL_LINK_W).unwrap_err();
        match err {
            result::Error::WindowsError(err) => assert_eq!(err.code(), REGDB_E_CLASSNOTREG),
            err => panic!("expected REGDB_E_CLASSNOTREG, got {err:?}"),
        }
        Ok(())
    }

    #[test]
    fn co_create_instance_does_not_choose_an_apartment_implicitly() {
        let remains_uninitialized = std::thread::spawn(|| {
            let _ = co_create_instance(
                GUID::from_u128(0xaaaaaaaa_bbbb_cccc_dddd_eeeeeeeeeeee),
                IID_ISHELL_LINK_W,
            );
            COM_INITIALIZATION
                .with(|state| matches!(*state.borrow(), ComInitialization::Uninitialized))
        })
        .join()
        .unwrap();

        assert!(remains_uninitialized);
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
        initialize_apartment(ApartmentType::MultiThreaded)?;

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
        let iface = register_interface(
            &table,
            "IDataTransferManagerInterop",
            IDataTransferManagerInterop::IID,
            InterfaceBase::IUnknown,
        )
        .add_method(
            "GetForWindow",
            MethodSignature::new(&table)
                .add_in(Type::pointer())
                .add_in(Type::pointer())
                .add_out(Type::winrt(table.object())),
        );

        let target_iid = DataTransferManager::IID;
        let result = iface.method(3).unwrap().invoke(
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
