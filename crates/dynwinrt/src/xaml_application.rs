// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#![allow(unsafe_op_in_unsafe_fn)]

use core::ffi::c_void;
use std::sync::Mutex;

use windows::Win32::System::Com::{CoTaskMemAlloc, CoTaskMemFree};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
};
use windows_core::{GUID, HRESULT, HSTRING, IInspectable, IUnknown, Interface};

use crate::com_helpers::{E_FAIL, E_NOINTERFACE, E_POINTER, IInspectableVtbl, S_OK};
use crate::{Result, WinRTValue};

const IID_IAPPLICATION_FACTORY: GUID = GUID::from_u128(0x9fd96657_5294_5a65_a1db_4fea143597da);
const IID_IAPPLICATION_OVERRIDES: GUID = GUID::from_u128(0xa33e81ef_c665_503b_8827_d27ef1720a06);
const IID_IXAML_METADATA_PROVIDER: GUID = GUID::from_u128(0xa96251f0_2214_5d53_8746_ce99a2593cd7);
const IID_XAML_LAUNCHED_CALLBACK: GUID = GUID::from_u128(0xf81c4e72_7a18_4a30_9126_6f62b6bdac83);

#[repr(C)]
struct ApplicationFactoryVtbl {
    base: IInspectableVtbl,
    create_instance: unsafe extern "system" fn(
        this: *mut c_void,
        outer: *mut c_void,
        inner: *mut *mut c_void,
        instance: *mut *mut c_void,
    ) -> HRESULT,
}

#[repr(C)]
struct ApplicationOverridesVtbl {
    base: IInspectableVtbl,
    on_launched: unsafe extern "system" fn(this: *mut c_void, args: *mut c_void) -> HRESULT,
}

#[repr(C)]
struct DelegateVtbl {
    base: windows_core::IUnknown_Vtbl,
    invoke: unsafe extern "system" fn(
        this: *mut c_void,
        arg: *mut c_void,
        unused: *mut c_void,
    ) -> HRESULT,
}

#[repr(C)]
struct XamlMetadataProviderVtbl {
    base: IInspectableVtbl,
    get_xaml_type: unsafe extern "system" fn(
        this: *mut c_void,
        type_name: AbiTypeName,
        result: *mut *mut c_void,
    ) -> HRESULT,
    get_xaml_type_by_full_name: unsafe extern "system" fn(
        this: *mut c_void,
        full_name: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT,
    get_xmlns_definitions: unsafe extern "system" fn(
        this: *mut c_void,
        count: *mut u32,
        result: *mut *mut c_void,
    ) -> HRESULT,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AbiTypeName {
    name: *mut c_void,
    kind: i32,
}

#[repr(C)]
struct XamlApplicationHost {
    vtable_overrides: *const ApplicationOverridesVtbl,
    vtable_metadata: *const XamlMetadataProviderVtbl,
    ref_count: windows_core::imp::WeakRefCount,
    metadata_provider: IUnknown,
    launched_callback: Option<IUnknown>,
    inner: Mutex<Option<IUnknown>>,
}

unsafe impl Send for XamlApplicationHost {}
unsafe impl Sync for XamlApplicationHost {}

impl XamlApplicationHost {
    const OVERRIDES_VTBL: ApplicationOverridesVtbl = ApplicationOverridesVtbl {
        base: IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::query_interface_overrides,
                AddRef: Self::add_ref_overrides,
                Release: Self::release_overrides,
            },
            get_iids: Self::get_iids_overrides,
            get_runtime_class_name: Self::get_runtime_class_name_overrides,
            get_trust_level: Self::get_trust_level_overrides,
        },
        on_launched: Self::on_launched,
    };

    const METADATA_VTBL: XamlMetadataProviderVtbl = XamlMetadataProviderVtbl {
        base: IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::query_interface_metadata,
                AddRef: Self::add_ref_metadata,
                Release: Self::release_metadata,
            },
            get_iids: Self::get_iids_metadata,
            get_runtime_class_name: Self::get_runtime_class_name_metadata,
            get_trust_level: Self::get_trust_level_metadata,
        },
        get_xaml_type: Self::get_xaml_type,
        get_xaml_type_by_full_name: Self::get_xaml_type_by_full_name,
        get_xmlns_definitions: Self::get_xmlns_definitions,
    };

    fn new(metadata_provider: IUnknown, launched_callback: Option<IUnknown>) -> Box<Self> {
        Box::new(Self {
            vtable_overrides: &Self::OVERRIDES_VTBL,
            vtable_metadata: &Self::METADATA_VTBL,
            ref_count: windows_core::imp::WeakRefCount::new(),
            metadata_provider,
            launched_callback,
            inner: Mutex::new(None),
        })
    }

    unsafe fn from_overrides_ptr(this: *mut c_void) -> &'static Self {
        &*(this as *const Self)
    }

    unsafe fn from_metadata_ptr(this: *mut c_void) -> &'static Self {
        let base = (this as *const *const c_void).sub(1) as *const Self;
        &*base
    }

    fn identity_ptr(&self) -> *mut c_void {
        self as *const Self as *mut c_void
    }

    fn metadata_ptr(&self) -> *mut c_void {
        unsafe { (self.identity_ptr() as *mut *mut c_void).add(1) as *mut c_void }
    }

    fn inner(&self) -> std::result::Result<Option<IUnknown>, HRESULT> {
        self.inner
            .lock()
            .map(|inner| inner.clone())
            .map_err(|_| E_FAIL)
    }

    unsafe extern "system" fn query_interface_overrides(
        this: *mut c_void,
        iid: *const GUID,
        result: *mut *mut c_void,
    ) -> HRESULT {
        Self::query_interface_impl(Self::from_overrides_ptr(this), iid, result)
    }

    unsafe extern "system" fn query_interface_metadata(
        this: *mut c_void,
        iid: *const GUID,
        result: *mut *mut c_void,
    ) -> HRESULT {
        Self::query_interface_impl(Self::from_metadata_ptr(this), iid, result)
    }

    unsafe fn query_interface_impl(
        host: &Self,
        iid: *const GUID,
        result: *mut *mut c_void,
    ) -> HRESULT {
        if iid.is_null() || result.is_null() {
            return E_POINTER;
        }

        *result = std::ptr::null_mut();
        let iid = &*iid;
        if *iid == IUnknown::IID
            || *iid == IInspectable::IID
            || *iid == IID_IAPPLICATION_OVERRIDES
            || *iid == windows_core::imp::IAgileObject::IID
        {
            *result = host.identity_ptr();
            host.ref_count.add_ref();
            return S_OK;
        }
        if *iid == IID_IXAML_METADATA_PROVIDER {
            *result = host.metadata_ptr();
            host.ref_count.add_ref();
            return S_OK;
        }
        if *iid == windows_core::imp::IMarshal::IID {
            host.ref_count.add_ref();
            return windows_core::imp::marshaler(core::mem::transmute(host.identity_ptr()), result);
        }
        let tear_off = host.ref_count.query(iid, host.identity_ptr());
        if !tear_off.is_null() {
            *result = tear_off;
            return S_OK;
        }

        match host.inner() {
            Ok(Some(inner)) => inner.query(iid, result),
            Ok(None) => E_NOINTERFACE,
            Err(hr) => hr,
        }
    }

    unsafe extern "system" fn add_ref_overrides(this: *mut c_void) -> u32 {
        Self::from_overrides_ptr(this).ref_count.add_ref()
    }

    unsafe extern "system" fn add_ref_metadata(this: *mut c_void) -> u32 {
        Self::from_metadata_ptr(this).ref_count.add_ref()
    }

    unsafe extern "system" fn release_overrides(this: *mut c_void) -> u32 {
        Self::release_impl(Self::from_overrides_ptr(this))
    }

    unsafe extern "system" fn release_metadata(this: *mut c_void) -> u32 {
        Self::release_impl(Self::from_metadata_ptr(this))
    }

    unsafe fn release_impl(host: &Self) -> u32 {
        let remaining = host.ref_count.release();
        if remaining == 0 {
            drop(Box::from_raw(host.identity_ptr() as *mut Self));
        }
        remaining
    }

    unsafe extern "system" fn get_iids_overrides(
        this: *mut c_void,
        count: *mut u32,
        result: *mut *mut GUID,
    ) -> HRESULT {
        Self::get_iids_impl(Self::from_overrides_ptr(this), count, result)
    }

    unsafe extern "system" fn get_iids_metadata(
        this: *mut c_void,
        count: *mut u32,
        result: *mut *mut GUID,
    ) -> HRESULT {
        Self::get_iids_impl(Self::from_metadata_ptr(this), count, result)
    }

    unsafe fn get_iids_impl(host: &Self, count: *mut u32, result: *mut *mut GUID) -> HRESULT {
        if count.is_null() || result.is_null() {
            return E_POINTER;
        }

        let local_iids = [IID_IAPPLICATION_OVERRIDES, IID_IXAML_METADATA_PROVIDER];
        let mut inner_count = 0;
        let mut inner_iids = std::ptr::null_mut();
        match host.inner() {
            Ok(Some(inner)) => {
                let vtable = *(inner.as_raw() as *const *const IInspectableVtbl);
                let hr = ((*vtable).get_iids)(inner.as_raw(), &mut inner_count, &mut inner_iids);
                if hr.is_err() {
                    return hr;
                }
            }
            Ok(None) => {}
            Err(hr) => return hr,
        }

        let total = local_iids.len() + inner_count as usize;
        let bytes = total
            .checked_mul(std::mem::size_of::<GUID>())
            .unwrap_or(usize::MAX);
        let combined = CoTaskMemAlloc(bytes) as *mut GUID;
        if combined.is_null() {
            if !inner_iids.is_null() {
                CoTaskMemFree(Some(inner_iids.cast()));
            }
            return HRESULT(0x8007000Eu32 as i32);
        }

        std::ptr::copy_nonoverlapping(local_iids.as_ptr(), combined, local_iids.len());
        if inner_count != 0 {
            std::ptr::copy_nonoverlapping(
                inner_iids,
                combined.add(local_iids.len()),
                inner_count as usize,
            );
        }
        if !inner_iids.is_null() {
            CoTaskMemFree(Some(inner_iids.cast()));
        }

        *count = total as u32;
        *result = combined;
        S_OK
    }

    unsafe extern "system" fn get_runtime_class_name_overrides(
        this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        Self::get_runtime_class_name_impl(Self::from_overrides_ptr(this), result)
    }

    unsafe extern "system" fn get_runtime_class_name_metadata(
        this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        Self::get_runtime_class_name_impl(Self::from_metadata_ptr(this), result)
    }

    unsafe fn get_runtime_class_name_impl(host: &Self, result: *mut *mut c_void) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        match host.inner() {
            Ok(Some(inner)) => {
                let vtable = *(inner.as_raw() as *const *const IInspectableVtbl);
                ((*vtable).get_runtime_class_name)(inner.as_raw(), result)
            }
            Ok(None) => {
                *result = std::ptr::null_mut();
                S_OK
            }
            Err(hr) => hr,
        }
    }

    unsafe extern "system" fn get_trust_level_overrides(
        this: *mut c_void,
        result: *mut i32,
    ) -> HRESULT {
        Self::get_trust_level_impl(Self::from_overrides_ptr(this), result)
    }

    unsafe extern "system" fn get_trust_level_metadata(
        this: *mut c_void,
        result: *mut i32,
    ) -> HRESULT {
        Self::get_trust_level_impl(Self::from_metadata_ptr(this), result)
    }

    unsafe fn get_trust_level_impl(host: &Self, result: *mut i32) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        match host.inner() {
            Ok(Some(inner)) => {
                let vtable = *(inner.as_raw() as *const *const IInspectableVtbl);
                ((*vtable).get_trust_level)(inner.as_raw(), result)
            }
            Ok(None) => {
                *result = 0;
                S_OK
            }
            Err(hr) => hr,
        }
    }

    unsafe extern "system" fn on_launched(this: *mut c_void, args: *mut c_void) -> HRESULT {
        let host = Self::from_overrides_ptr(this);
        let Some(callback) = host.launched_callback.as_ref() else {
            return S_OK;
        };
        let callback_ptr = callback.as_raw();
        let vtable = *(callback_ptr as *const *const DelegateVtbl);
        ((*vtable).invoke)(callback_ptr, args, std::ptr::null_mut())
    }

    unsafe extern "system" fn get_xaml_type(
        this: *mut c_void,
        type_name: AbiTypeName,
        result: *mut *mut c_void,
    ) -> HRESULT {
        let host = Self::from_metadata_ptr(this);
        let provider = host.metadata_provider.as_raw();
        let vtable = *(provider as *const *const XamlMetadataProviderVtbl);
        ((*vtable).get_xaml_type)(provider, type_name, result)
    }

    unsafe extern "system" fn get_xaml_type_by_full_name(
        this: *mut c_void,
        full_name: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        let host = Self::from_metadata_ptr(this);
        let provider = host.metadata_provider.as_raw();
        let vtable = *(provider as *const *const XamlMetadataProviderVtbl);
        ((*vtable).get_xaml_type_by_full_name)(provider, full_name, result)
    }

    unsafe extern "system" fn get_xmlns_definitions(
        this: *mut c_void,
        count: *mut u32,
        result: *mut *mut c_void,
    ) -> HRESULT {
        let host = Self::from_metadata_ptr(this);
        let provider = host.metadata_provider.as_raw();
        let vtable = *(provider as *const *const XamlMetadataProviderVtbl);
        ((*vtable).get_xmlns_definitions)(provider, count, result)
    }
}

fn query_interface(object: &IUnknown, iid: &GUID) -> windows_core::Result<IUnknown> {
    let mut result = std::ptr::null_mut();
    unsafe {
        object.query(iid, &mut result).ok()?;
        Ok(IUnknown::from_raw(result))
    }
}

fn enable_per_monitor_v2() -> windows_core::Result<()> {
    // WinUI windows inherit DPI awareness from the UI thread that creates them.
    let previous =
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if previous.0.is_null() {
        Err(windows_core::Error::from_thread())
    } else {
        Ok(())
    }
}

struct InitialHostRef(*mut XamlApplicationHost);

impl Drop for InitialHostRef {
    fn drop(&mut self) {
        unsafe {
            XamlApplicationHost::release_impl(&*self.0);
        }
    }
}

fn compose_xaml_application(
    factory: &IUnknown,
    metadata_provider: IUnknown,
    launched_callback: Option<IUnknown>,
) -> windows_core::Result<IUnknown> {
    let host = XamlApplicationHost::new(metadata_provider, launched_callback);
    let host_ptr = Box::into_raw(host);
    let _initial_ref = InitialHostRef(host_ptr);
    let outer = host_ptr as *mut c_void;
    let mut inner = std::ptr::null_mut();
    let mut instance = std::ptr::null_mut();

    let vtable = unsafe { *(factory.as_raw() as *const *const ApplicationFactoryVtbl) };
    let hr =
        unsafe { ((*vtable).create_instance)(factory.as_raw(), outer, &mut inner, &mut instance) };
    if hr.is_err() {
        unsafe {
            if !instance.is_null() {
                drop(IUnknown::from_raw(instance));
            }
            if !inner.is_null() {
                drop(IUnknown::from_raw(inner));
            }
        }
        return Err(windows_core::Error::from_hresult(hr));
    }
    if inner.is_null() || instance.is_null() {
        unsafe {
            if !instance.is_null() {
                drop(IUnknown::from_raw(instance));
            }
            if !inner.is_null() {
                drop(IUnknown::from_raw(inner));
            }
        }
        return Err(windows_core::Error::from_hresult(E_POINTER));
    }

    unsafe {
        let inner = IUnknown::from_raw(inner);
        let application = IUnknown::from_raw(instance);
        *(*host_ptr)
            .inner
            .lock()
            .map_err(|_| windows_core::Error::from_hresult(E_FAIL))? = Some(inner);
        Ok(application)
    }
}

/// Create a composed WinUI `Application` whose outer object exposes the supplied
/// `IXamlMetadataProvider`.
pub fn create_xaml_application(
    metadata_provider: &IUnknown,
    launched_callback: Option<&IUnknown>,
) -> Result<WinRTValue> {
    enable_per_monitor_v2()?;
    let provider = query_interface(metadata_provider, &IID_IXAML_METADATA_PROVIDER)?;
    let activation_factory =
        crate::ro_get_activation_factory_2(&HSTRING::from("Microsoft.UI.Xaml.Application"))?;
    let factory = activation_factory
        .as_object()
        .expect("activation factory must be an object");
    let factory = query_interface(&factory, &IID_IAPPLICATION_FACTORY)?;
    let callback = launched_callback
        .map(|callback| query_interface(callback, &IID_XAML_LAUNCHED_CALLBACK))
        .transpose()?;
    let application = compose_xaml_application(&factory, provider, callback)?;
    Ok(WinRTValue::Object(application))
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::HiDpi::{AreDpiAwarenessContextsEqual, GetThreadDpiAwarenessContext};

    #[test]
    fn enables_per_monitor_v2_on_the_ui_thread() {
        enable_per_monitor_v2().unwrap();

        let current = unsafe { GetThreadDpiAwarenessContext() };
        assert!(unsafe {
            AreDpiAwarenessContextsEqual(current, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
                .as_bool()
        });
    }
}
