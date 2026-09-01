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

use crate::metadata_table::{TypeHandle, ValueTypeData};
use crate::native_callback::{CallbackAbiType, CallbackSignature};
use crate::value::{AsyncInfo, WinRTValue};

const E_FAIL: HRESULT = HRESULT(0x80004005u32 as i32);
const E_INVALIDARG: HRESULT = HRESULT(0x80070057u32 as i32);
const E_NOTIMPL: HRESULT = HRESULT(0x80004001u32 as i32);
const E_POINTER: HRESULT = HRESULT(0x80004003u32 as i32);

fn delegate_creation_error(code: HRESULT, message: impl AsRef<str>) -> crate::result::Error {
    crate::result::Error::WindowsError(windows_core::Error::new(code, message.as_ref()))
}

// ======================================================================
// DynamicDelegate — general-purpose WinRT delegate COM object
// ======================================================================

/// Callback type: receives marshalled Invoke arguments, returns HRESULT.
pub type DelegateCallback = Box<dyn Fn(&[WinRTValue]) -> HRESULT + Send + Sync>;

/// Static vtable for a delegate with two general-purpose-register parameters.
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

#[repr(C)]
struct DynamicDelegateVtbl {
    base: windows_core::IUnknown_Vtbl,
    invoke: *const c_void,
}

/// A dynamically-constructed WinRT delegate COM object.
///
/// Supports delegates with up to 2 ABI parameters. Struct parameters use an
/// ABI-aware libffi closure; established scalar and float shapes keep their
/// static trampolines.
/// This covers TypedEventHandler<T,U>, AsyncOperationCompletedHandler<T>,
/// EventHandler<T>, and most other standard delegates.
#[repr(C)]
struct DynamicDelegate {
    vtable: *const c_void,
    ref_count: windows_core::imp::RefCount,
    delegate_iid: GUID,
    param_types: Vec<TypeHandle>,
    callback: DelegateCallback,
    _owned_vtable: Option<Box<DynamicDelegateVtbl>>,
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
    fn try_create(
        delegate_iid: GUID,
        param_types: Vec<TypeHandle>,
        callback: DelegateCallback,
    ) -> crate::result::Result<IUnknown> {
        if param_types.len() > 2 {
            return Err(delegate_creation_error(
                E_INVALIDARG,
                format!(
                    "DynamicDelegate supports up to 2 parameters, got {}",
                    param_types.len()
                ),
            ));
        }

        use crate::metadata_table::TypeKind;

        let owned_vtable = if param_types
            .iter()
            .any(|typ| matches!(typ.kind(), TypeKind::Struct(_)))
        {
            let signature = Self::libffi_signature(&param_types).ok_or_else(|| {
                delegate_creation_error(
                    E_NOTIMPL,
                    "WinRT delegate struct callbacks do not support one or more parameter types",
                )
            })?;
            let invoke = crate::native_callback::callback_code(3, signature, Self::invoke_libffi)
                .map_err(|error| {
                delegate_creation_error(
                    E_FAIL,
                    format!("failed to create the WinRT delegate callback closure: {error}"),
                )
            })?;
            Some(Box::new(DynamicDelegateVtbl {
                base: windows_core::IUnknown_Vtbl {
                    QueryInterface: Self::qi,
                    AddRef: Self::add_ref,
                    Release: Self::release,
                },
                invoke,
            }))
        } else {
            None
        };

        // Pick the right vtable based on parameter types.
        // Float types go in float registers on ARM64/x64, so each distinct
        // float signature needs its own trampoline with the correct ABI.
        let vtable = if let Some(vtable) = owned_vtable.as_deref() {
            vtable as *const DynamicDelegateVtbl as *const c_void
        } else {
            match param_types.len() {
                1 => match param_types[0].kind() {
                    TypeKind::F64 => &Self::VTBL_1_F64 as *const _ as *const c_void,
                    TypeKind::F32 => &Self::VTBL_1_F32 as *const _ as *const c_void,
                    _ => &Self::VTBL as *const _ as *const c_void,
                },
                2 => match param_types[1].kind() {
                    TypeKind::F64 => &Self::VTBL_PTR_F64 as *const _ as *const c_void,
                    TypeKind::F32 => &Self::VTBL_PTR_F32 as *const _ as *const c_void,
                    _ => &Self::VTBL as *const _ as *const c_void,
                },
                _ => &Self::VTBL as *const _ as *const c_void,
            }
        };

        let delegate = Box::new(Self {
            vtable,
            ref_count: windows_core::imp::RefCount::new(1),
            delegate_iid,
            param_types,
            callback,
            _owned_vtable: owned_vtable,
        });
        Ok(unsafe { IUnknown::from_raw(Box::into_raw(delegate) as *mut c_void) })
    }

    fn libffi_signature(param_types: &[TypeHandle]) -> Option<CallbackSignature> {
        let parameters = param_types
            .iter()
            .map(|typ| Some((Self::callback_abi_type(typ)?, typ.libffi_type())))
            .collect::<Option<Vec<_>>>()?;
        Some(CallbackSignature::hresult(parameters))
    }

    fn callback_abi_type(typ: &TypeHandle) -> Option<CallbackAbiType> {
        use crate::metadata_table::TypeKind;

        match typ.kind() {
            TypeKind::I8 => Some(CallbackAbiType::I8),
            TypeKind::Bool | TypeKind::U8 => Some(CallbackAbiType::U8),
            TypeKind::I16 => Some(CallbackAbiType::I16),
            TypeKind::U16 | TypeKind::Char16 => Some(CallbackAbiType::U16),
            TypeKind::I32 | TypeKind::HResult | TypeKind::Enum(_) => Some(CallbackAbiType::I32),
            TypeKind::U32 => Some(CallbackAbiType::U32),
            TypeKind::I64 => Some(CallbackAbiType::I64),
            TypeKind::U64 => Some(CallbackAbiType::U64),
            TypeKind::F32 => Some(CallbackAbiType::F32),
            TypeKind::F64 => Some(CallbackAbiType::F64),
            TypeKind::Guid => Some(CallbackAbiType::Guid),
            TypeKind::HString
            | TypeKind::Object
            | TypeKind::Interface(_)
            | TypeKind::Delegate(_)
            | TypeKind::RuntimeClass(_)
            | TypeKind::Parameterized(_)
            | TypeKind::IAsyncAction
            | TypeKind::IAsyncActionWithProgress(_)
            | TypeKind::IAsyncOperation(_)
            | TypeKind::IAsyncOperationWithProgress(_) => Some(CallbackAbiType::Pointer),
            // The full recursive WinRT signature identifies the ABI layout in
            // the process-wide closure cache. Dispatch still recovers the
            // table-qualified TypeHandle from the delegate instance.
            TypeKind::Struct(_) => {
                let size = typ.size_of();
                if size == 0 {
                    return None;
                }
                Some(CallbackAbiType::NativeStruct(
                    typ.table()
                        .try_closed_signature_string_kind(typ.kind())
                        .ok()?,
                    size,
                ))
            }
            TypeKind::ArrayOfIUnknown
            | TypeKind::Generic { .. }
            | TypeKind::OutValue(_)
            | TypeKind::Array(_) => None,
        }
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

    unsafe fn invoke_libffi(
        _slot: usize,
        signature: &CallbackSignature,
        args: *const *const c_void,
        result: *mut c_void,
    ) {
        if result.is_null() {
            return;
        }
        unsafe { signature.initialize_failure_result(result, E_FAIL) };
        let invocation = (|| -> Result<HRESULT, HRESULT> {
            if args.is_null() {
                return Err(E_POINTER);
            }
            let this_storage = unsafe { *args };
            if this_storage.is_null() {
                return Err(E_POINTER);
            }
            let this = unsafe { *this_storage.cast::<*mut c_void>() };
            if this.is_null() {
                return Err(E_POINTER);
            }
            let delegate = unsafe { &*(this.cast::<Self>()) };
            if signature.parameters().len() != delegate.param_types.len() {
                return Err(E_FAIL);
            }
            let mut values = Vec::with_capacity(delegate.param_types.len());
            for (index, typ) in delegate.param_types.iter().enumerate() {
                let storage = unsafe { *args.add(index + 1) };
                values.push(unsafe { marshal_libffi_arg(storage, typ)? });
            }
            Ok((delegate.callback)(&values))
        })();
        let hresult = invocation.unwrap_or_else(|error| error);
        unsafe { result.cast::<i32>().write(hresult.0) };
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

/// Convert libffi callback argument storage to an owned WinRT value.
///
/// # Safety
///
/// `storage` must point to a readable ABI value matching `typ`.
unsafe fn marshal_libffi_arg(
    storage: *const c_void,
    typ: &TypeHandle,
) -> Result<WinRTValue, HRESULT> {
    use crate::metadata_table::TypeKind;

    if storage.is_null() {
        return Err(E_POINTER);
    }
    match typ.kind() {
        TypeKind::Bool => Ok(WinRTValue::Bool(unsafe { *storage.cast::<u8>() } != 0)),
        TypeKind::I8 => Ok(WinRTValue::I8(unsafe { *storage.cast::<i8>() })),
        TypeKind::U8 => Ok(WinRTValue::U8(unsafe { *storage.cast::<u8>() })),
        TypeKind::I16 => Ok(WinRTValue::I16(unsafe { *storage.cast::<i16>() })),
        TypeKind::U16 | TypeKind::Char16 => Ok(WinRTValue::U16(unsafe { *storage.cast::<u16>() })),
        TypeKind::I32 => Ok(WinRTValue::I32(unsafe { *storage.cast::<i32>() })),
        TypeKind::Enum(_) => Ok(WinRTValue::Enum {
            value: unsafe { *storage.cast::<i32>() },
            type_handle: typ.clone(),
        }),
        TypeKind::U32 => Ok(WinRTValue::U32(unsafe { *storage.cast::<u32>() })),
        TypeKind::HResult => Ok(WinRTValue::HResult(HRESULT(unsafe {
            *storage.cast::<i32>()
        }))),
        TypeKind::I64 => Ok(WinRTValue::I64(unsafe {
            std::ptr::read_unaligned(storage.cast::<i64>())
        })),
        TypeKind::U64 => Ok(WinRTValue::U64(unsafe {
            std::ptr::read_unaligned(storage.cast::<u64>())
        })),
        TypeKind::F32 => Ok(WinRTValue::F32(unsafe { *storage.cast::<f32>() })),
        TypeKind::F64 => Ok(WinRTValue::F64(unsafe {
            std::ptr::read_unaligned(storage.cast::<f64>())
        })),
        TypeKind::Guid => Ok(WinRTValue::Guid(unsafe { *storage.cast::<GUID>() })),
        TypeKind::Struct(_) => Ok(WinRTValue::Struct(unsafe {
            ValueTypeData::from_borrowed_abi(typ, storage)
        })),
        TypeKind::HString
        | TypeKind::Object
        | TypeKind::Interface(_)
        | TypeKind::Delegate(_)
        | TypeKind::RuntimeClass(_)
        | TypeKind::Parameterized(_)
        | TypeKind::IAsyncAction
        | TypeKind::IAsyncActionWithProgress(_)
        | TypeKind::IAsyncOperation(_)
        | TypeKind::IAsyncOperationWithProgress(_) => {
            let raw = unsafe { *storage.cast::<*mut c_void>() };
            marshal_abi_ptr(raw, typ)
        }
        TypeKind::ArrayOfIUnknown
        | TypeKind::Generic { .. }
        | TypeKind::OutValue(_)
        | TypeKind::Array(_) => Err(E_NOTIMPL),
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
pub fn try_create_delegate(
    delegate_iid: GUID,
    param_types: Vec<TypeHandle>,
    callback: DelegateCallback,
) -> crate::result::Result<IUnknown> {
    DynamicDelegate::try_create(delegate_iid, param_types, callback)
}

/// Create a dynamic WinRT delegate, panicking if its callback backend cannot be created.
///
/// Language bindings should use [`try_create_delegate`] so closure allocation
/// failures become language exceptions.
pub fn create_delegate(
    delegate_iid: GUID,
    param_types: Vec<TypeHandle>,
    callback: DelegateCallback,
) -> IUnknown {
    try_create_delegate(delegate_iid, param_types, callback)
        .expect("failed to create dynamic WinRT delegate")
}

/// Fallible convenience wrapper around [`try_create_delegate`].
pub fn try_create_delegate_value(
    delegate_iid: GUID,
    param_types: Vec<TypeHandle>,
    callback: DelegateCallback,
) -> crate::result::Result<WinRTValue> {
    try_create_delegate(delegate_iid, param_types, callback).map(WinRTValue::Object)
}

/// Convenience: create a delegate and wrap as WinRTValue::Object.
pub fn create_delegate_value(
    delegate_iid: GUID,
    param_types: Vec<TypeHandle>,
    callback: DelegateCallback,
) -> WinRTValue {
    try_create_delegate_value(delegate_iid, param_types, callback)
        .expect("failed to create dynamic WinRT delegate value")
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
    use windows_core::IUnknown_Vtbl;

    #[repr(C)]
    struct StructProgressHandlerVtbl<T> {
        base: IUnknown_Vtbl,
        invoke: unsafe extern "system" fn(*mut c_void, *mut c_void, T) -> HRESULT,
    }

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

    #[test]
    fn try_create_delegate_reports_unsupported_struct_callback_signature() {
        let table = MetadataTable::new();
        let element_type = table.u32_type();
        let unsupported_field = table.out_value(&element_type);
        let progress_type = table.struct_type(
            "Test.FallibleProgress",
            std::slice::from_ref(&unsupported_field),
        );

        let result = try_create_delegate(
            GUID::from_u128(0x66666666_6666_6666_6666_666666666666),
            vec![table.object(), progress_type],
            Box::new(|_| HRESULT(0)),
        );

        let error = match result {
            Ok(_) => panic!("unsupported struct callback signature unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(
            error
                .message()
                .contains("do not support one or more parameter types")
        );
    }

    #[test]
    fn try_create_delegate_rejects_zero_sized_struct_without_panicking() {
        let table = MetadataTable::new();
        let progress_type = table.struct_type("Test.EmptyProgress", &[]);

        let result = try_create_delegate(
            GUID::from_u128(0x77777777_7777_7777_7777_777777777777),
            vec![table.object(), progress_type],
            Box::new(|_| HRESULT(0)),
        );

        let error = match result {
            Ok(_) => panic!("zero-sized struct callback unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(
            error
                .message()
                .contains("do not support one or more parameter types")
        );
    }

    #[test]
    fn marshal_libffi_arg_reads_eight_byte_scalars_from_four_byte_alignment() {
        #[repr(align(8))]
        struct AlignedStorage([u8; 16]);

        let table = MetadataTable::new();
        let mut storage = AlignedStorage([0; 16]);
        let pointer = unsafe { storage.0.as_mut_ptr().add(4).cast::<c_void>() };
        assert_eq!(pointer as usize % 4, 0);
        assert_ne!(pointer as usize % 8, 0);

        let expected_i64 = -0x1020_3040_5060_708i64;
        storage.0[4..12].copy_from_slice(&expected_i64.to_ne_bytes());
        assert!(matches!(
            unsafe { marshal_libffi_arg(pointer, &table.i64_type()) },
            Ok(WinRTValue::I64(value)) if value == expected_i64
        ));

        let expected_u64 = 0xfedc_ba98_7654_3210u64;
        storage.0[4..12].copy_from_slice(&expected_u64.to_ne_bytes());
        assert!(matches!(
            unsafe { marshal_libffi_arg(pointer, &table.u64_type()) },
            Ok(WinRTValue::U64(value)) if value == expected_u64
        ));

        let expected_f64 = -12345.625f64;
        storage.0[4..12].copy_from_slice(&expected_f64.to_ne_bytes());
        assert!(matches!(
            unsafe { marshal_libffi_arg(pointer, &table.f64_type()) },
            Ok(WinRTValue::F64(value)) if value.to_bits() == expected_f64.to_bits()
        ));
    }

    #[test]
    fn test_delegate_struct_trampoline_marshals_wide_value() {
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct WideProgress {
            completed: u64,
            total: u64,
            stage: u32,
        }

        let table = MetadataTable::new();
        let progress_type = table.struct_type(
            "Test.WideProgress",
            &[table.u64_type(), table.u64_type(), table.u32_type()],
        );
        assert!(progress_type.size_of() > std::mem::size_of::<usize>());

        let received = Arc::new(std::sync::Mutex::new(None));
        let received_callback = received.clone();
        let handler = create_delegate(
            GUID::from_u128(0x44444444_4444_4444_4444_444444444444),
            vec![table.object(), progress_type.clone()],
            Box::new(move |args| {
                *received_callback.lock().unwrap() = Some(args[1].clone());
                HRESULT(0)
            }),
        );

        let vtable = unsafe {
            &**(handler.as_raw() as *const *const StructProgressHandlerVtbl<WideProgress>)
        };
        let result = unsafe {
            (vtable.invoke)(
                handler.as_raw(),
                std::ptr::null_mut(),
                WideProgress {
                    completed: 17,
                    total: 42,
                    stage: 3,
                },
            )
        };
        assert_eq!(result, HRESULT(0));

        let received = received.lock().unwrap().take().unwrap();
        let data = received.as_struct().unwrap();
        assert_eq!(data.type_handle(), &progress_type);
        assert_eq!(data.get_field::<u64>(0), 17);
        assert_eq!(data.get_field::<u64>(1), 42);
        assert_eq!(data.get_field::<u32>(2), 3);
    }

    #[test]
    fn test_delegate_struct_trampoline_owns_nested_non_blittable_fields() {
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct ReferenceProgress {
            label: *mut c_void,
            total: *mut c_void,
        }

        #[repr(C)]
        #[derive(Clone, Copy)]
        struct NestedProgress {
            sequence: u64,
            reference: ReferenceProgress,
        }

        let table = MetadataTable::new();
        let value_type = table.u64_type();
        let generic = table.generic(crate::metadata_table::IREFERENCE, 1);
        let reference_type = table.parameterized(&generic, std::slice::from_ref(&value_type));
        let reference_progress =
            table.struct_type("Test.ReferenceProgress", &[table.hstring(), reference_type]);
        let progress_type = table.struct_type(
            "Test.NestedReferenceProgress",
            &[table.u64_type(), reference_progress],
        );

        let label = windows_core::HSTRING::from("retained progress");
        let label_raw: *mut c_void = unsafe { std::mem::transmute_copy(&label) };
        let boxed = crate::box_ireference(WinRTValue::U64(99), value_type).unwrap();
        let boxed_object = boxed.as_object().unwrap();

        let received = Arc::new(std::sync::Mutex::new(None));
        let received_callback = received.clone();
        let handler = create_delegate(
            GUID::from_u128(0x55555555_5555_5555_5555_555555555555),
            vec![table.object(), progress_type.clone()],
            Box::new(move |args| {
                *received_callback.lock().unwrap() = Some(args[1].clone());
                HRESULT(0)
            }),
        );

        let vtable = unsafe {
            &**(handler.as_raw() as *const *const StructProgressHandlerVtbl<NestedProgress>)
        };
        let result = unsafe {
            (vtable.invoke)(
                handler.as_raw(),
                std::ptr::null_mut(),
                NestedProgress {
                    sequence: 7,
                    reference: ReferenceProgress {
                        label: label_raw,
                        total: boxed_object.as_raw(),
                    },
                },
            )
        };
        assert_eq!(result, HRESULT(0));

        drop(boxed);
        drop(boxed_object);
        drop(label);

        let received = received.lock().unwrap().take().unwrap();
        let data = received.as_struct().unwrap();
        assert_eq!(data.type_handle(), &progress_type);
        assert_eq!(data.get_field::<u64>(0), 7);
        let reference = data.get_field_struct(1);
        assert_eq!(reference.get_field_hstring(0).unwrap(), "retained progress");
        let total = reference.get_field_object(1).unwrap().unwrap();
        let total: windows::Foundation::IReference<u64> = total.cast().unwrap();
        assert_eq!(total.Value().unwrap(), 99);
    }
}
