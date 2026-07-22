// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#![allow(unsafe_op_in_unsafe_fn)]

use core::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};

use windows::Win32::System::Com::CoTaskMemAlloc;
use windows_core::{GUID, HRESULT, IInspectable, IUnknown, Interface};

use crate::com_helpers::{E_FAIL, E_NOINTERFACE, E_POINTER, IInspectableVtbl, S_OK};
use crate::value::WinRTValue;

pub const IID_IELEMENT_FACTORY: GUID = GUID::from_u128(0x75faba47_2cf2_54ae_91e6_0581556fddaa);
const IID_IUIELEMENT: GUID = GUID::from_u128(0xc3c01020_320c_5cf6_9d24_d396bbfa4d8b);

pub type ElementFactoryGetCallback =
    Box<dyn Fn(&WinRTValue) -> std::result::Result<WinRTValue, HRESULT> + Send + Sync>;
pub type ElementFactoryRecycleCallback = Box<dyn Fn(&WinRTValue) -> HRESULT + Send + Sync>;

#[repr(C)]
struct ElementFactoryVtbl {
    base: IInspectableVtbl,
    get_element: unsafe extern "system" fn(
        this: *mut c_void,
        args: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT,
    recycle_element: unsafe extern "system" fn(this: *mut c_void, args: *mut c_void) -> HRESULT,
}

#[repr(C)]
struct DynamicElementFactory {
    vtable: *const ElementFactoryVtbl,
    ref_count: windows_core::imp::RefCount,
    get_element: ElementFactoryGetCallback,
    recycle_element: ElementFactoryRecycleCallback,
}

unsafe impl Send for DynamicElementFactory {}
unsafe impl Sync for DynamicElementFactory {}

impl DynamicElementFactory {
    const VTBL: ElementFactoryVtbl = ElementFactoryVtbl {
        base: IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::query_interface,
                AddRef: Self::add_ref,
                Release: Self::release,
            },
            get_iids: Self::get_iids,
            get_runtime_class_name: Self::get_runtime_class_name,
            get_trust_level: Self::get_trust_level,
        },
        get_element: Self::get_element,
        recycle_element: Self::recycle_element,
    };

    fn create(
        get_element: ElementFactoryGetCallback,
        recycle_element: ElementFactoryRecycleCallback,
    ) -> IUnknown {
        let factory = Box::new(Self {
            vtable: &Self::VTBL,
            ref_count: windows_core::imp::RefCount::new(1),
            get_element,
            recycle_element,
        });
        unsafe { IUnknown::from_raw(Box::into_raw(factory) as *mut c_void) }
    }

    unsafe fn from_ptr(this: *mut c_void) -> &'static Self {
        &*(this as *const Self)
    }

    unsafe extern "system" fn query_interface(
        this: *mut c_void,
        iid: *const GUID,
        result: *mut *mut c_void,
    ) -> HRESULT {
        if iid.is_null() || result.is_null() {
            return E_POINTER;
        }

        *result = std::ptr::null_mut();
        let factory = Self::from_ptr(this);
        let iid = &*iid;
        if *iid == IUnknown::IID || *iid == IInspectable::IID || *iid == IID_IELEMENT_FACTORY {
            *result = this;
            factory.ref_count.add_ref();
            S_OK
        } else {
            E_NOINTERFACE
        }
    }

    unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
        Self::from_ptr(this).ref_count.add_ref()
    }

    unsafe extern "system" fn release(this: *mut c_void) -> u32 {
        let factory = Self::from_ptr(this);
        let remaining = factory.ref_count.release();
        if remaining == 0 {
            drop(Box::from_raw(this as *mut Self));
        }
        remaining
    }

    unsafe extern "system" fn get_iids(
        _this: *mut c_void,
        count: *mut u32,
        result: *mut *mut GUID,
    ) -> HRESULT {
        if count.is_null() || result.is_null() {
            return E_POINTER;
        }

        let allocated = CoTaskMemAlloc(std::mem::size_of::<GUID>()) as *mut GUID;
        if allocated.is_null() {
            return HRESULT(0x8007000Eu32 as i32);
        }
        allocated.write(IID_IELEMENT_FACTORY);
        *count = 1;
        *result = allocated;
        S_OK
    }

    unsafe extern "system" fn get_runtime_class_name(
        _this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        *result = std::ptr::null_mut();
        S_OK
    }

    unsafe extern "system" fn get_trust_level(_this: *mut c_void, result: *mut i32) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        *result = 0;
        S_OK
    }

    unsafe fn wrap_argument(args: *mut c_void) -> std::result::Result<WinRTValue, HRESULT> {
        if args.is_null() {
            return Err(E_POINTER);
        }
        let borrowed = IUnknown::from_raw_borrowed(&args).ok_or(E_POINTER)?;
        Ok(WinRTValue::Object(borrowed.clone()))
    }

    unsafe extern "system" fn get_element(
        this: *mut c_void,
        args: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        *result = std::ptr::null_mut();

        let factory = Self::from_ptr(this);
        let argument = match Self::wrap_argument(args) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let callback_result = catch_unwind(AssertUnwindSafe(|| (factory.get_element)(&argument)));
        let value = match callback_result {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => return error,
            Err(_) => return E_FAIL,
        };

        let WinRTValue::Object(element) = value else {
            return E_FAIL;
        };
        element.query(&IID_IUIELEMENT, result)
    }

    unsafe extern "system" fn recycle_element(this: *mut c_void, args: *mut c_void) -> HRESULT {
        let factory = Self::from_ptr(this);
        let argument = match Self::wrap_argument(args) {
            Ok(value) => value,
            Err(error) => return error,
        };
        match catch_unwind(AssertUnwindSafe(|| (factory.recycle_element)(&argument))) {
            Ok(result) => result,
            Err(_) => E_FAIL,
        }
    }
}

pub fn create_element_factory(
    get_element: ElementFactoryGetCallback,
    recycle_element: ElementFactoryRecycleCallback,
) -> IUnknown {
    DynamicElementFactory::create(get_element, recycle_element)
}

pub fn create_element_factory_value(
    get_element: ElementFactoryGetCallback,
    recycle_element: ElementFactoryRecycleCallback,
) -> WinRTValue {
    WinRTValue::Object(create_element_factory(get_element, recycle_element))
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use windows::Foundation::Uri;
    use windows_core::{HSTRING, Interface};

    use super::*;

    #[test]
    fn element_factory_rejects_non_ui_results_and_recycles() {
        let returned = Uri::CreateUri(&HSTRING::from("https://example.com/returned"))
            .unwrap()
            .cast::<IUnknown>()
            .unwrap();
        let argument = Uri::CreateUri(&HSTRING::from("https://example.com/argument"))
            .unwrap()
            .cast::<IUnknown>()
            .unwrap();
        let recycle_count = Arc::new(AtomicUsize::new(0));
        let recycle_count_callback = recycle_count.clone();
        let returned_raw = returned.as_raw() as usize;
        let factory = create_element_factory(
            Box::new(move |value| {
                assert!(matches!(value, WinRTValue::Object(_)));
                let raw = returned_raw as *mut c_void;
                let borrowed = unsafe { IUnknown::from_raw_borrowed(&raw) }.unwrap();
                Ok(WinRTValue::Object(borrowed.clone()))
            }),
            Box::new(move |value| {
                assert!(matches!(value, WinRTValue::Object(_)));
                recycle_count_callback.fetch_add(1, Ordering::SeqCst);
                S_OK
            }),
        );

        let mut raw_factory = std::ptr::null_mut();
        let hr = unsafe { factory.query(&IID_IELEMENT_FACTORY, &mut raw_factory) };
        assert!(hr.is_ok());
        let interface = unsafe { IUnknown::from_raw(raw_factory) };
        let vtable = unsafe { *(interface.as_raw() as *const *const ElementFactoryVtbl) };

        let mut raw_result = std::ptr::null_mut();
        let hr = unsafe {
            ((*vtable).get_element)(interface.as_raw(), argument.as_raw(), &mut raw_result)
        };
        assert_eq!(hr, E_NOINTERFACE);
        assert!(raw_result.is_null());

        let hr = unsafe { ((*vtable).recycle_element)(interface.as_raw(), argument.as_raw()) };
        assert!(hr.is_ok());
        assert_eq!(recycle_count.load(Ordering::SeqCst), 1);
    }
}
