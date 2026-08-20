// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use core::{ffi::c_void, mem::size_of};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, LazyLock, Mutex},
};

use libffi::{low, middle::Type};
use windows_core::HRESULT;

use crate::native_call::system_cif;

const E_FAIL: HRESULT = HRESULT(0x80004005u32 as i32);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum CallbackAbiType {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    Pointer,
    Guid,
    NativeStruct(String, usize),
}

impl CallbackAbiType {
    pub(crate) fn size(&self) -> usize {
        match self {
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::I64 | Self::U64 | Self::F64 => 8,
            Self::Pointer => size_of::<*mut c_void>(),
            Self::Guid => 16,
            Self::NativeStruct(_, size) => *size,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum CallbackReturnAbi {
    HResult,
    Void,
    Value(CallbackAbiType),
}

#[derive(Debug, Clone)]
pub(crate) struct CallbackSignature {
    parameters: Vec<CallbackAbiType>,
    libffi_parameters: Vec<Type>,
    return_abi: CallbackReturnAbi,
    libffi_return: Type,
}

// libffi Type graphs are immutable after construction. CallbackSignature owns
// every graph and only shares it through immutable references while preparing
// cached CIFs.
unsafe impl Send for CallbackSignature {}
unsafe impl Sync for CallbackSignature {}

impl PartialEq for CallbackSignature {
    fn eq(&self, other: &Self) -> bool {
        self.parameters == other.parameters && self.return_abi == other.return_abi
    }
}

impl Eq for CallbackSignature {}

impl Hash for CallbackSignature {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.parameters.hash(state);
        self.return_abi.hash(state);
    }
}

impl CallbackSignature {
    pub(crate) fn hresult(parameters: Vec<(CallbackAbiType, Type)>) -> Self {
        let (parameters, libffi_parameters) = parameters.into_iter().unzip();
        Self {
            parameters,
            libffi_parameters,
            return_abi: CallbackReturnAbi::HResult,
            libffi_return: Type::i32(),
        }
    }

    pub(crate) fn void(parameters: Vec<(CallbackAbiType, Type)>) -> Self {
        let (parameters, libffi_parameters) = parameters.into_iter().unzip();
        Self {
            parameters,
            libffi_parameters,
            return_abi: CallbackReturnAbi::Void,
            libffi_return: Type::void(),
        }
    }

    pub(crate) fn direct(
        parameters: Vec<(CallbackAbiType, Type)>,
        result: (CallbackAbiType, Type),
    ) -> Self {
        let (parameters, libffi_parameters) = parameters.into_iter().unzip();
        Self {
            parameters,
            libffi_parameters,
            return_abi: CallbackReturnAbi::Value(result.0),
            libffi_return: result.1,
        }
    }

    pub(crate) fn parameters(&self) -> &[CallbackAbiType] {
        &self.parameters
    }

    pub(crate) fn return_abi(&self) -> &CallbackReturnAbi {
        &self.return_abi
    }

    pub(crate) unsafe fn initialize_failure_result(&self, result: *mut c_void, error: HRESULT) {
        if result.is_null() {
            return;
        }
        unsafe {
            match self.return_abi() {
                CallbackReturnAbi::HResult => result.cast::<i32>().write(error.0),
                CallbackReturnAbi::Void => {}
                CallbackReturnAbi::Value(value) if value.size() != 0 => {
                    std::ptr::write_bytes(result, 0, value.size())
                }
                CallbackReturnAbi::Value(_) => {}
            }
        }
    }

    fn argument_types(&self) -> Vec<Type> {
        let mut types = vec![Type::pointer()];
        types.extend(self.libffi_parameters.iter().cloned());
        types
    }

    fn result_type(&self) -> Type {
        self.libffi_return.clone()
    }
}

pub(crate) type CallbackDispatch = unsafe fn(
    slot: usize,
    signature: &CallbackSignature,
    args: *const *const c_void,
    result: *mut c_void,
);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ClosureKey {
    slot: usize,
    signature: CallbackSignature,
    dispatch: usize,
}

struct CallbackContext {
    slot: usize,
    signature: CallbackSignature,
    dispatch: CallbackDispatch,
}

struct OwnedCallbackClosure {
    _cif: Box<libffi::middle::Cif>,
    closure: *mut low::ffi_closure,
    code: *const c_void,
    _context: Box<CallbackContext>,
}

// The CIF, context, and executable closure are fully initialized before they
// enter the global cache and remain immutable for the process lifetime.
unsafe impl Send for OwnedCallbackClosure {}
unsafe impl Sync for OwnedCallbackClosure {}

impl OwnedCallbackClosure {
    fn new(
        slot: usize,
        signature: CallbackSignature,
        dispatch: CallbackDispatch,
    ) -> Result<Self, String> {
        let cif = Box::new(system_cif(
            signature.argument_types(),
            signature.result_type(),
        ));
        let context = Box::new(CallbackContext {
            slot,
            signature,
            dispatch,
        });
        let (closure, code) = low::closure_alloc();
        if closure.is_null() || code.as_ptr().is_null() {
            if !closure.is_null() {
                unsafe { low::closure_free(closure) };
            }
            return Err("libffi could not allocate executable callback memory".into());
        }
        let status = unsafe {
            libffi_sys::ffi_prep_closure_loc(
                closure,
                cif.as_raw_ptr(),
                Some(invoke_callback),
                context.as_ref() as *const CallbackContext as *mut c_void,
                code.as_mut_ptr(),
            )
        };
        if status != libffi_sys::ffi_status_FFI_OK {
            unsafe { low::closure_free(closure) };
            return Err(format!(
                "libffi could not prepare callback closure: {status}"
            ));
        }
        Ok(Self {
            _cif: cif,
            closure,
            code: code.as_ptr(),
            _context: context,
        })
    }
}

impl Drop for OwnedCallbackClosure {
    fn drop(&mut self) {
        if !self.closure.is_null() {
            unsafe { low::closure_free(self.closure) };
            self.closure = std::ptr::null_mut();
        }
    }
}

unsafe extern "C" fn invoke_callback(
    _cif: *mut libffi_sys::ffi_cif,
    result: *mut c_void,
    args: *mut *mut c_void,
    userdata: *mut c_void,
) {
    let context = unsafe { &*userdata.cast::<CallbackContext>() };
    let dispatch = catch_unwind(AssertUnwindSafe(|| unsafe {
        (context.dispatch)(
            context.slot,
            &context.signature,
            args.cast_const().cast(),
            result,
        )
    }));
    if dispatch.is_err() {
        unsafe { context.signature.initialize_failure_result(result, E_FAIL) };
    }
}

static CLOSURES: LazyLock<Mutex<HashMap<ClosureKey, Arc<OwnedCallbackClosure>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) fn callback_code(
    slot: usize,
    signature: CallbackSignature,
    dispatch: CallbackDispatch,
) -> Result<*const c_void, String> {
    let key = ClosureKey {
        slot,
        signature: signature.clone(),
        dispatch: dispatch as usize,
    };
    let mut closures = CLOSURES
        .lock()
        .map_err(|_| "libffi callback cache is poisoned".to_string())?;
    if let Some(closure) = closures.get(&key) {
        return Ok(closure.code);
    }
    let closure = Arc::new(OwnedCallbackClosure::new(slot, signature, dispatch)?);
    let code = closure.code;
    closures.insert(key, closure);
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn test_dispatch(
        slot: usize,
        signature: &CallbackSignature,
        args: *const *const c_void,
        result: *mut c_void,
    ) {
        assert_eq!(slot, 3);
        assert_eq!(signature.parameters(), &[CallbackAbiType::I32]);
        let value = unsafe { *(*args.add(1)).cast::<i32>() };
        unsafe { *result.cast::<i32>() = value };
    }

    #[test]
    fn libffi_callback_closures_are_callable_and_cached() {
        let signature = CallbackSignature::hresult(vec![(CallbackAbiType::I32, Type::i32())]);
        let first = callback_code(3, signature.clone(), test_dispatch).unwrap();
        let second = callback_code(3, signature, test_dispatch).unwrap();
        assert_eq!(first, second);
        let callback: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT =
            unsafe { std::mem::transmute(first) };
        assert_eq!(unsafe { callback(std::ptr::null_mut(), 27) }, HRESULT(27));
    }
}
