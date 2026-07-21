// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Dynamic WinRT delegate (callback) implementation.
//!
//! A delegate is a COM object with IUnknown + a single `Invoke` method.
//! `DynamicDelegate` creates such objects at runtime, marshalling ABI
//! parameters to `WinRTValue` and forwarding to a user-supplied callback.

use core::ffi::c_void;
use windows_core::{GUID, HRESULT, IUnknown, Interface};
use windows_future::IAsyncInfo;

use crate::metadata_table::TypeHandle;
use crate::value::{AsyncInfo, WinRTValue};

// ======================================================================
// DynamicDelegate — general-purpose WinRT delegate COM object
// ======================================================================

/// Callback type: receives marshalled Invoke arguments, returns HRESULT.
pub type DelegateCallback = Box<dyn Fn(&[WinRTValue]) -> HRESULT + Send + Sync>;

/// Vtable for a delegate with 2 pointer-sized ABI params (covers ~95% of delegates).
#[repr(C)]
struct Delegate2Vtbl {
    base: windows_core::IUnknown_Vtbl,
    invoke: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
}

/// Vtable variant for delegates where param1 is pointer and param2 is f64.
/// On ARM64, f64 goes into d-registers (not x-registers), so a different
/// function signature is needed for correct ABI.
#[repr(C)]
struct DelegatePtrF64Vtbl {
    base: windows_core::IUnknown_Vtbl,
    invoke: unsafe extern "system" fn(*mut c_void, *mut c_void, f64) -> HRESULT,
}

/// Vtable variant for delegates where param1 is pointer and param2 is f32.
#[repr(C)]
struct DelegatePtrF32Vtbl {
    base: windows_core::IUnknown_Vtbl,
    invoke: unsafe extern "system" fn(*mut c_void, *mut c_void, f32) -> HRESULT,
}

/// Vtable variant for 1-param delegate where the param is f64.
#[repr(C)]
struct Delegate1F64Vtbl {
    base: windows_core::IUnknown_Vtbl,
    invoke: unsafe extern "system" fn(*mut c_void, f64) -> HRESULT,
}

/// Vtable variant for 1-param delegate where the param is f32.
#[repr(C)]
struct Delegate1F32Vtbl {
    base: windows_core::IUnknown_Vtbl,
    invoke: unsafe extern "system" fn(*mut c_void, f32) -> HRESULT,
}

/// A dynamically-constructed WinRT delegate COM object.
///
/// Supports delegates with up to 2 ABI parameters (pointer-sized).
/// This covers TypedEventHandler<T,U>, AsyncOperationCompletedHandler<T>,
/// EventHandler<T>, and most other standard delegates.
#[repr(C)]
struct DynamicDelegate {
    vtable: *const Delegate2Vtbl,
    ref_count: windows_core::imp::RefCount,
    delegate_iid: GUID,
    param_types: Vec<TypeHandle>,
    callback: DelegateCallback,
}

// Safety: DynamicDelegate is ref-counted and the callback is Send+Sync.
unsafe impl Send for DynamicDelegate {}
unsafe impl Sync for DynamicDelegate {}

impl DynamicDelegate {
    const VTBL: Delegate2Vtbl = Delegate2Vtbl {
        base: windows_core::IUnknown_Vtbl {
            QueryInterface: Self::qi,
            AddRef: Self::add_ref,
            Release: Self::release,
        },
        invoke: Self::invoke_2,
    };

    const VTBL_PTR_F64: DelegatePtrF64Vtbl = DelegatePtrF64Vtbl {
        base: windows_core::IUnknown_Vtbl {
            QueryInterface: Self::qi,
            AddRef: Self::add_ref,
            Release: Self::release,
        },
        invoke: Self::invoke_ptr_f64,
    };

    const VTBL_PTR_F32: DelegatePtrF32Vtbl = DelegatePtrF32Vtbl {
        base: windows_core::IUnknown_Vtbl {
            QueryInterface: Self::qi,
            AddRef: Self::add_ref,
            Release: Self::release,
        },
        invoke: Self::invoke_ptr_f32,
    };

    const VTBL_1_F64: Delegate1F64Vtbl = Delegate1F64Vtbl {
        base: windows_core::IUnknown_Vtbl {
            QueryInterface: Self::qi,
            AddRef: Self::add_ref,
            Release: Self::release,
        },
        invoke: Self::invoke_1_f64,
    };

    const VTBL_1_F32: Delegate1F32Vtbl = Delegate1F32Vtbl {
        base: windows_core::IUnknown_Vtbl {
            QueryInterface: Self::qi,
            AddRef: Self::add_ref,
            Release: Self::release,
        },
        invoke: Self::invoke_1_f32,
    };

    /// Create a new dynamic delegate as an IUnknown COM pointer.
    ///
    /// - `delegate_iid`: the IID of the delegate interface (for QueryInterface)
    /// - `param_types`: types of the Invoke method's parameters (excluding `this`)
    /// - `callback`: function called when WinRT invokes the delegate
    pub fn create(
        delegate_iid: GUID,
        param_types: Vec<TypeHandle>,
        callback: DelegateCallback,
    ) -> IUnknown {
        assert!(
            param_types.len() <= 2,
            "DynamicDelegate currently supports up to 2 parameters, got {}",
            param_types.len()
        );

        use crate::metadata_table::TypeKind;

        // Pick the right vtable based on parameter types.
        // Float types go in float registers on ARM64/x64, so each distinct
        // float signature needs its own trampoline with the correct ABI.
        let vtable: *const Delegate2Vtbl = match param_types.len() {
            1 => match param_types[0].kind() {
                TypeKind::F64 => &Self::VTBL_1_F64 as *const _ as *const Delegate2Vtbl,
                TypeKind::F32 => &Self::VTBL_1_F32 as *const _ as *const Delegate2Vtbl,
                _ => &Self::VTBL,
            },
            2 => match param_types[1].kind() {
                TypeKind::F64 => &Self::VTBL_PTR_F64 as *const _ as *const Delegate2Vtbl,
                TypeKind::F32 => &Self::VTBL_PTR_F32 as *const _ as *const Delegate2Vtbl,
                _ => &Self::VTBL,
            },
            _ => &Self::VTBL,
        };

        let delegate = Box::new(Self {
            vtable,
            ref_count: windows_core::imp::RefCount::new(1),
            delegate_iid,
            param_types,
            callback,
        });
        unsafe { IUnknown::from_raw(Box::into_raw(delegate) as *mut c_void) }
    }

    // ------------------------------------------------------------------
    // IUnknown
    // ------------------------------------------------------------------

    unsafe extern "system" fn qi(
        this: *mut c_void,
        iid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> HRESULT {
        if iid.is_null() || ppv.is_null() {
            return HRESULT(-2147467261); // E_INVALIDARG
        }
        let iid = unsafe { &*iid };
        let delegate = unsafe { &*(this as *const Self) };
        if *iid == IUnknown::IID
            || *iid == windows_core::imp::IAgileObject::IID
            || *iid == delegate.delegate_iid
        {
            unsafe { *ppv = this };
            unsafe { Self::add_ref(this) };
            HRESULT(0)
        } else if *iid == windows_core::imp::IMarshal::IID {
            unsafe {
                delegate.ref_count.add_ref();
                windows_core::imp::marshaler(core::mem::transmute(this), ppv)
            }
        } else {
            unsafe { *ppv = std::ptr::null_mut() };
            HRESULT(-2147467262) // E_NOINTERFACE
        }
    }

    unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
        let delegate = unsafe { &*(this as *const Self) };
        delegate.ref_count.add_ref()
    }

    unsafe extern "system" fn release(this: *mut c_void) -> u32 {
        let delegate = unsafe { &*(this as *const Self) };
        let remaining = delegate.ref_count.release();
        if remaining == 0 {
            unsafe { drop(Box::from_raw(this as *mut Self)) };
        }
        remaining
    }

    // ------------------------------------------------------------------
    // Invoke trampoline (2 pointer-sized ABI params)
    // ------------------------------------------------------------------

    unsafe extern "system" fn invoke_2(
        this: *mut c_void,
        arg0: *mut c_void,
        arg1: *mut c_void,
    ) -> HRESULT {
        let delegate = unsafe { &*(this as *const Self) };
        let raw_args = [arg0, arg1];
        let mut values = Vec::with_capacity(delegate.param_types.len());

        for (i, pt) in delegate.param_types.iter().enumerate() {
            if i < raw_args.len() {
                match marshal_abi_ptr(raw_args[i], pt) {
                    Ok(value) => values.push(value),
                    Err(error) => return error,
                }
            }
        }

        (delegate.callback)(&values)
    }

    /// Invoke trampoline for delegates where arg1 is f64.
    /// The f64 parameter uses a float register (d0 on ARM64, XMM2 on x64),
    /// so a separate function signature is needed for correct ABI.
    unsafe extern "system" fn invoke_ptr_f64(
        this: *mut c_void,
        arg0: *mut c_void,
        arg1: f64,
    ) -> HRESULT {
        let delegate = unsafe { &*(this as *const Self) };
        let mut values = Vec::with_capacity(delegate.param_types.len());

        if delegate.param_types.len() >= 1 {
            match marshal_abi_ptr(arg0, &delegate.param_types[0]) {
                Ok(value) => values.push(value),
                Err(error) => return error,
            }
        }
        if delegate.param_types.len() >= 2 {
            values.push(WinRTValue::F64(arg1));
        }

        (delegate.callback)(&values)
    }

    /// Invoke trampoline for delegates where arg1 is f32.
    unsafe extern "system" fn invoke_ptr_f32(
        this: *mut c_void,
        arg0: *mut c_void,
        arg1: f32,
    ) -> HRESULT {
        let delegate = unsafe { &*(this as *const Self) };
        let mut values = Vec::with_capacity(delegate.param_types.len());

        if delegate.param_types.len() >= 1 {
            match marshal_abi_ptr(arg0, &delegate.param_types[0]) {
                Ok(value) => values.push(value),
                Err(error) => return error,
            }
        }
        if delegate.param_types.len() >= 2 {
            values.push(WinRTValue::F32(arg1));
        }

        (delegate.callback)(&values)
    }

    /// Invoke trampoline for 1-param delegate where the param is f64.
    unsafe extern "system" fn invoke_1_f64(this: *mut c_void, arg0: f64) -> HRESULT {
        let delegate = unsafe { &*(this as *const Self) };
        (delegate.callback)(&[WinRTValue::F64(arg0)])
    }

    /// Invoke trampoline for 1-param delegate where the param is f32.
    unsafe extern "system" fn invoke_1_f32(this: *mut c_void, arg0: f32) -> HRESULT {
        let delegate = unsafe { &*(this as *const Self) };
        (delegate.callback)(&[WinRTValue::F32(arg0)])
    }
}

/// Convert a raw ABI pointer-sized argument to WinRTValue, based on type.
fn marshal_abi_ptr(raw: *mut c_void, typ: &TypeHandle) -> Result<WinRTValue, HRESULT> {
    use crate::metadata_table::TypeKind;
    match typ.kind() {
        // Pointer-sized types: wrap as Object (AddRef via from_raw_borrowed + clone)
        TypeKind::Object
        | TypeKind::Interface(_)
        | TypeKind::RuntimeClass(_)
        | TypeKind::Delegate(_)
        | TypeKind::Parameterized(_) => {
            if raw.is_null() {
                Ok(WinRTValue::Null)
            } else {
                let obj = unsafe { IUnknown::from_raw_borrowed(&raw) }.unwrap();
                Ok(WinRTValue::Object(obj.clone()))
            }
        }
        TypeKind::IAsyncAction
        | TypeKind::IAsyncOperation(_)
        | TypeKind::IAsyncActionWithProgress(_)
        | TypeKind::IAsyncOperationWithProgress(_) => {
            if raw.is_null() {
                Ok(WinRTValue::Null)
            } else {
                let object = unsafe { IUnknown::from_raw_borrowed(&raw) }
                    .unwrap()
                    .clone();
                let info: IAsyncInfo = object.cast().map_err(|error| error.code())?;
                Ok(WinRTValue::Async(AsyncInfo {
                    info,
                    async_type: typ.clone(),
                }))
            }
        }
        // HString: transmute the raw HSTRING handle
        TypeKind::HString => {
            if raw.is_null() {
                Ok(WinRTValue::HString(windows_core::HSTRING::new()))
            } else {
                let hstr: &windows_core::HSTRING =
                    unsafe { &*(&raw as *const *mut c_void as *const windows_core::HSTRING) };
                Ok(WinRTValue::HString(hstr.clone()))
            }
        }
        // Small integer types packed into pointer-sized arg
        TypeKind::Bool => Ok(WinRTValue::Bool((raw as usize) != 0)),
        TypeKind::I8 => Ok(WinRTValue::I8(raw as i8)),
        TypeKind::U8 => Ok(WinRTValue::U8(raw as u8)),
        TypeKind::I16 => Ok(WinRTValue::I16(raw as i16)),
        TypeKind::U16 => Ok(WinRTValue::U16(raw as u16)),
        TypeKind::Char16 => Ok(WinRTValue::U16(raw as u16)),
        TypeKind::I32 => Ok(WinRTValue::I32(raw as i32)),
        TypeKind::Enum(_) => Ok(WinRTValue::Enum {
            value: raw as i32,
            type_handle: typ.clone(),
        }),
        TypeKind::U32 => Ok(WinRTValue::U32(raw as u32)),
        TypeKind::HResult => Ok(WinRTValue::HResult(HRESULT(raw as i32))),
        TypeKind::I64 => Ok(WinRTValue::I64(raw as i64)),
        TypeKind::U64 => Ok(WinRTValue::U64(raw as u64)),
        TypeKind::F64 => {
            // f64 passed as pointer-sized raw bits (only valid on platforms where
            // the caller puts it in a GPR; see invoke_ptr_f64 for float-register ABI)
            Ok(WinRTValue::F64(f64::from_bits(raw as u64)))
        }
        TypeKind::F32 => Ok(WinRTValue::F32(f32::from_bits(raw as u32))),
        _ => Err(HRESULT(0x80004001u32 as i32)),
    }
}

// ======================================================================
// Public API
// ======================================================================

/// Create a dynamic WinRT delegate COM object.
///
/// # Arguments
/// - `delegate_iid`: the delegate interface IID
/// - `param_types`: Invoke parameter types (max 2)
/// - `callback`: called when WinRT invokes the delegate
///
/// # Returns
/// An `IUnknown` smart pointer to the delegate COM object.
/// Pass this to WinRT methods that accept the delegate (e.g. event subscriptions).
pub fn create_delegate(
    delegate_iid: GUID,
    param_types: Vec<TypeHandle>,
    callback: DelegateCallback,
) -> IUnknown {
    DynamicDelegate::create(delegate_iid, param_types, callback)
}

/// Convenience: create a delegate and wrap as WinRTValue::Object.
pub fn create_delegate_value(
    delegate_iid: GUID,
    param_types: Vec<TypeHandle>,
    callback: DelegateCallback,
) -> WinRTValue {
    WinRTValue::Object(create_delegate(delegate_iid, param_types, callback))
}

// ======================================================================
// Tests
// ======================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata_table::MetadataTable;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    /// P2: Verify that a 2-param delegate with f32 second param
    /// correctly receives F32 (not F64) via the f32 trampoline.
    #[test]
    fn test_delegate_ptr_f32_trampoline() {
        let table = MetadataTable::new();
        let iid = GUID::from_u128(0x11111111_1111_1111_1111_111111111111);

        let received_f32 = Arc::new(std::sync::Mutex::new(0.0f32));
        let received_clone = received_f32.clone();

        let delegate = create_delegate(
            iid,
            vec![table.object(), table.f32_type()],
            Box::new(move |args| {
                // arg[1] should be WinRTValue::F32, not F64
                if let WinRTValue::F32(v) = &args[1] {
                    *received_clone.lock().unwrap() = *v;
                } else {
                    panic!("Expected F32, got {:?}", args[1]);
                }
                HRESULT(0)
            }),
        );

        // Simulate calling the delegate with the f32 trampoline
        // by calling through the vtable directly
        let raw = delegate.as_raw();
        let vtbl = unsafe { *(raw as *const *const DelegatePtrF32Vtbl) };
        let hr = unsafe { ((*vtbl).invoke)(raw, std::ptr::null_mut(), 3.14f32) };
        assert_eq!(hr, HRESULT(0));
        assert!((*received_f32.lock().unwrap() - 3.14f32).abs() < 1e-6);
    }

    /// P2: Verify that a 1-param delegate with f64 param
    /// correctly receives F64 via the 1-param f64 trampoline.
    #[test]
    fn test_delegate_1_f64_trampoline() {
        let table = MetadataTable::new();
        let iid = GUID::from_u128(0x22222222_2222_2222_2222_222222222222);

        let received = Arc::new(std::sync::Mutex::new(0.0f64));
        let received_clone = received.clone();

        let delegate = create_delegate(
            iid,
            vec![table.f64_type()],
            Box::new(move |args| {
                assert_eq!(args.len(), 1);
                if let WinRTValue::F64(v) = &args[0] {
                    *received_clone.lock().unwrap() = *v;
                } else {
                    panic!("Expected F64, got {:?}", args[0]);
                }
                HRESULT(0)
            }),
        );

        let raw = delegate.as_raw();
        let vtbl = unsafe { *(raw as *const *const Delegate1F64Vtbl) };
        let hr = unsafe { ((*vtbl).invoke)(raw, 2.71828f64) };
        assert_eq!(hr, HRESULT(0));
        assert!((*received.lock().unwrap() - 2.71828f64).abs() < 1e-10);
    }

    /// P2: Verify that a 1-param delegate with f32 param
    /// correctly receives F32.
    #[test]
    fn test_delegate_1_f32_trampoline() {
        let table = MetadataTable::new();
        let iid = GUID::from_u128(0x33333333_3333_3333_3333_333333333333);

        let called = Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();

        let delegate = create_delegate(
            iid,
            vec![table.f32_type()],
            Box::new(move |args| {
                assert_eq!(args.len(), 1);
                match &args[0] {
                    WinRTValue::F32(v) => assert!((*v - 1.5f32).abs() < 1e-6),
                    other => panic!("Expected F32, got {:?}", other),
                }
                called_clone.store(true, Ordering::SeqCst);
                HRESULT(0)
            }),
        );

        let raw = delegate.as_raw();
        let vtbl = unsafe { *(raw as *const *const Delegate1F32Vtbl) };
        let hr = unsafe { ((*vtbl).invoke)(raw, 1.5f32) };
        assert_eq!(hr, HRESULT(0));
        assert!(called.load(Ordering::SeqCst));
    }
}
