// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use core::{ffi::c_void, mem::ManuallyDrop};
use std::{
    cell::UnsafeCell,
    sync::{Arc, Mutex, MutexGuard},
};

use windows::{
    Win32::{
        Foundation::{FILETIME, SysAllocStringLen, VARIANT_BOOL},
        System::{
            Com::{
                BLOB, CoTaskMemAlloc, DISPPARAMS, EXCEPINFO, IDispatch, SAFEARRAY, SAFEARRAYBOUND,
                StructuredStorage::{
                    CABOOL, CAC, CACLSID, CADBL, CAFILETIME, CAFLT, CAH, CAI, CAL, CALPWSTR, CAUB,
                    CAUH, CAUI, CAUL, PROPVARIANT, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
                    PropVariantClear,
                },
            },
            Ole::{
                SafeArrayAccessData, SafeArrayCopy, SafeArrayCreate, SafeArrayCreateEx,
                SafeArrayDestroy, SafeArrayGetDim, SafeArrayGetElemsize, SafeArrayGetIID,
                SafeArrayGetLBound, SafeArrayGetUBound, SafeArrayGetVartype, SafeArrayUnaccessData,
            },
            Variant::{
                VARENUM, VARIANT, VARIANT_0_0, VARIANT_0_0_0, VT_ARRAY, VT_BLOB, VT_BOOL, VT_BSTR,
                VT_BYREF, VT_CLSID, VT_DISPATCH, VT_EMPTY, VT_FILETIME, VT_I1, VT_I2, VT_I4, VT_I8,
                VT_INT, VT_LPWSTR, VT_NULL, VT_R4, VT_R8, VT_TYPEMASK, VT_UI1, VT_UI2, VT_UI4,
                VT_UI8, VT_UINT, VT_UNKNOWN, VT_VARIANT, VT_VECTOR, VariantClear, VariantCopy,
                VariantInit,
            },
        },
    },
    core::{BSTR, GUID, IUnknown, Interface, PWSTR},
};

use crate::result;

const E_INVALIDARG: windows_core::HRESULT = windows_core::HRESULT(0x80070057u32 as i32);
const E_OUTOFMEMORY: windows_core::HRESULT = windows_core::HRESULT(0x8007000Eu32 as i32);
const MAX_SAFEARRAY_RANK: usize = 8;

fn invalid_argument(message: impl AsRef<str>) -> result::Error {
    result::Error::WindowsError(windows_core::Error::new(E_INVALIDARG, message.as_ref()))
}

fn out_of_memory(message: &'static str) -> result::Error {
    result::Error::WindowsError(windows_core::Error::new(E_OUTOFMEMORY, message))
}

fn windows_error(error: windows_core::Error) -> result::Error {
    result::Error::WindowsError(error)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariantType {
    Empty,
    Null,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    Int,
    UInt,
    F32,
    F64,
    Bool,
    Bstr,
    Unknown,
    Dispatch,
    SafeArray(SafeArrayElementType),
}

impl VariantType {
    pub const fn vartype(self) -> u16 {
        match self {
            Self::Empty => VT_EMPTY.0,
            Self::Null => VT_NULL.0,
            Self::I8 => VT_I1.0,
            Self::U8 => VT_UI1.0,
            Self::I16 => VT_I2.0,
            Self::U16 => VT_UI2.0,
            Self::I32 => VT_I4.0,
            Self::U32 => VT_UI4.0,
            Self::I64 => VT_I8.0,
            Self::U64 => VT_UI8.0,
            Self::Int => VT_INT.0,
            Self::UInt => VT_UINT.0,
            Self::F32 => VT_R4.0,
            Self::F64 => VT_R8.0,
            Self::Bool => VT_BOOL.0,
            Self::Bstr => VT_BSTR.0,
            Self::Unknown => VT_UNKNOWN.0,
            Self::Dispatch => VT_DISPATCH.0,
            Self::SafeArray(element) => VT_ARRAY.0 | element.vartype(),
        }
    }
}

pub enum VariantData {
    Empty,
    Null,
    I8(i8),
    U8(u8),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    Int(i32),
    UInt(u32),
    F32(f32),
    F64(f64),
    Bool(bool),
    Bstr(String),
    Unknown(Option<IUnknown>),
    Dispatch(Option<IUnknown>),
    SafeArray(SafeArrayValue),
}

impl std::fmt::Debug for VariantData {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("Empty"),
            Self::Null => formatter.write_str("Null"),
            Self::I8(value) => formatter.debug_tuple("I8").field(value).finish(),
            Self::U8(value) => formatter.debug_tuple("U8").field(value).finish(),
            Self::I16(value) => formatter.debug_tuple("I16").field(value).finish(),
            Self::U16(value) => formatter.debug_tuple("U16").field(value).finish(),
            Self::I32(value) => formatter.debug_tuple("I32").field(value).finish(),
            Self::U32(value) => formatter.debug_tuple("U32").field(value).finish(),
            Self::I64(value) => formatter.debug_tuple("I64").field(value).finish(),
            Self::U64(value) => formatter.debug_tuple("U64").field(value).finish(),
            Self::Int(value) => formatter.debug_tuple("Int").field(value).finish(),
            Self::UInt(value) => formatter.debug_tuple("UInt").field(value).finish(),
            Self::F32(value) => formatter.debug_tuple("F32").field(value).finish(),
            Self::F64(value) => formatter.debug_tuple("F64").field(value).finish(),
            Self::Bool(value) => formatter.debug_tuple("Bool").field(value).finish(),
            Self::Bstr(value) => formatter.debug_tuple("Bstr").field(value).finish(),
            Self::Unknown(value) => formatter
                .debug_tuple("Unknown")
                .field(&value.is_some())
                .finish(),
            Self::Dispatch(value) => formatter
                .debug_tuple("Dispatch")
                .field(&value.is_some())
                .finish(),
            Self::SafeArray(value) => formatter.debug_tuple("SafeArray").field(value).finish(),
        }
    }
}

struct VariantInner {
    raw: UnsafeCell<VARIANT>,
}

impl Drop for VariantInner {
    fn drop(&mut self) {
        let _ = unsafe { VariantClear(self.raw.get()) };
    }
}

#[derive(Clone)]
pub struct VariantValue {
    inner: Arc<VariantInner>,
}

impl std::fmt::Debug for VariantValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VariantValue")
            .field("vartype", &self.vartype())
            .finish()
    }
}

impl VariantValue {
    fn from_initialized(raw: VARIANT) -> Self {
        Self {
            inner: Arc::new(VariantInner {
                raw: UnsafeCell::new(raw),
            }),
        }
    }

    pub(crate) unsafe fn from_owned_raw(raw: VARIANT) -> result::Result<Self> {
        validate_raw_variant(&raw)?;
        Ok(Self::from_initialized(raw))
    }

    unsafe fn record(&self) -> &VARIANT_0_0 {
        unsafe {
            &*((&(*self.raw()).Anonymous.Anonymous as *const ManuallyDrop<VARIANT_0_0>)
                .cast::<VARIANT_0_0>())
        }
    }

    unsafe fn record_mut(&self) -> &mut VARIANT_0_0 {
        unsafe {
            &mut *((&mut (*self.raw_mut()).Anonymous.Anonymous as *mut ManuallyDrop<VARIANT_0_0>)
                .cast::<VARIANT_0_0>())
        }
    }

    unsafe fn payload(&self) -> &VARIANT_0_0_0 {
        unsafe { &self.record().Anonymous }
    }

    unsafe fn payload_mut(&self) -> &mut VARIANT_0_0_0 {
        unsafe { &mut self.record_mut().Anonymous }
    }

    pub fn empty() -> Self {
        Self::from_initialized(unsafe { VariantInit() })
    }

    pub fn null() -> Self {
        let value = Self::empty();
        unsafe { value.set_vartype(VT_NULL) };
        value
    }

    pub fn from_i8(value: i8) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_I1);
            result.payload_mut().cVal = value;
        }
        result
    }

    pub fn from_u8(value: u8) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_UI1);
            result.payload_mut().bVal = value;
        }
        result
    }

    pub fn from_i16(value: i16) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_I2);
            result.payload_mut().iVal = value;
        }
        result
    }

    pub fn from_u16(value: u16) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_UI2);
            result.payload_mut().uiVal = value;
        }
        result
    }

    pub fn from_i32(value: i32) -> Self {
        Self::from_i32_with_type(value, VT_I4)
    }

    pub fn from_int(value: i32) -> Self {
        Self::from_i32_with_type(value, VT_INT)
    }

    fn from_i32_with_type(value: i32, vartype: VARENUM) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(vartype);
            if vartype == VT_INT {
                result.payload_mut().intVal = value;
            } else {
                result.payload_mut().lVal = value;
            }
        }
        result
    }

    pub fn from_u32(value: u32) -> Self {
        Self::from_u32_with_type(value, VT_UI4)
    }

    pub fn from_uint(value: u32) -> Self {
        Self::from_u32_with_type(value, VT_UINT)
    }

    fn from_u32_with_type(value: u32, vartype: VARENUM) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(vartype);
            if vartype == VT_UINT {
                result.payload_mut().uintVal = value;
            } else {
                result.payload_mut().ulVal = value;
            }
        }
        result
    }

    pub fn from_i64(value: i64) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_I8);
            result.payload_mut().llVal = value;
        }
        result
    }

    pub fn from_u64(value: u64) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_UI8);
            result.payload_mut().ullVal = value;
        }
        result
    }

    pub fn from_f32(value: f32) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_R4);
            result.payload_mut().fltVal = value;
        }
        result
    }

    pub fn from_f64(value: f64) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_R8);
            result.payload_mut().dblVal = value;
        }
        result
    }

    pub fn from_bool(value: bool) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_BOOL);
            result.payload_mut().boolVal = VARIANT_BOOL(if value { -1 } else { 0 });
        }
        result
    }

    pub fn from_bstr(value: &str) -> result::Result<Self> {
        let utf16 = value.encode_utf16().collect::<Vec<_>>();
        let bstr = unsafe { SysAllocStringLen(Some(&utf16)) };
        if bstr.is_empty() && !utf16.is_empty() {
            return Err(out_of_memory("SysAllocStringLen failed for VARIANT"));
        }
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_BSTR);
            result.payload_mut().bstrVal = ManuallyDrop::new(bstr);
        }
        Ok(result)
    }

    pub fn from_unknown(value: Option<&IUnknown>) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_UNKNOWN);
            result.payload_mut().punkVal = ManuallyDrop::new(value.cloned());
        }
        result
    }

    pub fn from_dispatch(value: Option<&IUnknown>) -> result::Result<Self> {
        let dispatch = value
            .map(|value| value.cast::<IDispatch>().map_err(windows_error))
            .transpose()?;
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_DISPATCH);
            result.payload_mut().pdispVal = ManuallyDrop::new(dispatch);
        }
        Ok(result)
    }

    pub fn from_safe_array(value: &SafeArrayValue) -> result::Result<Self> {
        let native = value.lock_native()?;
        let copy = unsafe { SafeArrayCopy(native.as_raw()) }.map_err(windows_error)?;
        let result = Self::empty();
        unsafe {
            result.set_vartype(VARENUM(VT_ARRAY.0 | value.element_type().vartype()));
            result.payload_mut().parray = copy;
        }
        Ok(result)
    }

    pub fn variant_type(&self) -> result::Result<VariantType> {
        variant_type(self.vartype())
    }

    pub fn vartype(&self) -> u16 {
        unsafe { self.record().vt.0 }
    }

    pub fn data(&self) -> result::Result<VariantData> {
        let typ = self.variant_type()?;
        let payload = unsafe { self.payload() };
        Ok(match typ {
            VariantType::Empty => VariantData::Empty,
            VariantType::Null => VariantData::Null,
            VariantType::I8 => VariantData::I8(unsafe { payload.cVal }),
            VariantType::U8 => VariantData::U8(unsafe { payload.bVal }),
            VariantType::I16 => VariantData::I16(unsafe { payload.iVal }),
            VariantType::U16 => VariantData::U16(unsafe { payload.uiVal }),
            VariantType::I32 => VariantData::I32(unsafe { payload.lVal }),
            VariantType::U32 => VariantData::U32(unsafe { payload.ulVal }),
            VariantType::I64 => VariantData::I64(unsafe { payload.llVal }),
            VariantType::U64 => VariantData::U64(unsafe { payload.ullVal }),
            VariantType::Int => VariantData::Int(unsafe { payload.intVal }),
            VariantType::UInt => VariantData::UInt(unsafe { payload.uintVal }),
            VariantType::F32 => VariantData::F32(unsafe { payload.fltVal }),
            VariantType::F64 => VariantData::F64(unsafe { payload.dblVal }),
            VariantType::Bool => {
                let value = unsafe { payload.boolVal.0 };
                if !matches!(value, 0 | -1) {
                    return Err(invalid_argument(format!(
                        "VARIANT VT_BOOL payload must be 0 or -1, received {value}"
                    )));
                }
                VariantData::Bool(value == -1)
            }
            VariantType::Bstr => {
                let value = unsafe { &payload.bstrVal };
                VariantData::Bstr(value.to_string())
            }
            VariantType::Unknown => {
                let value = unsafe { &payload.punkVal };
                VariantData::Unknown(value.as_ref().cloned())
            }
            VariantType::Dispatch => {
                let value = unsafe { &payload.pdispVal };
                VariantData::Dispatch(
                    value
                        .as_ref()
                        .map(|value| value.cast::<IUnknown>().map_err(windows_error))
                        .transpose()?,
                )
            }
            VariantType::SafeArray(element) => {
                let array = unsafe { payload.parray };
                let copy = unsafe { SafeArrayCopy(array) }.map_err(windows_error)?;
                VariantData::SafeArray(unsafe {
                    SafeArrayValue::from_owned_raw(copy, Some(element), None)?
                })
            }
        })
    }

    pub(crate) fn validate_supported(&self) -> result::Result<()> {
        self.variant_type().map(|_| ())
    }

    pub(crate) fn copy_raw(&self) -> result::Result<VARIANT> {
        self.validate_supported()?;
        let mut copy = unsafe { VariantInit() };
        if let Err(error) = unsafe { VariantCopy(&mut copy, self.raw()) } {
            let _ = unsafe { VariantClear(&mut copy) };
            return Err(windows_error(error));
        }
        Ok(copy)
    }

    pub(crate) fn raw(&self) -> *const VARIANT {
        self.inner.raw.get()
    }

    pub(crate) fn raw_mut(&self) -> *mut VARIANT {
        self.inner.raw.get()
    }

    unsafe fn set_vartype(&self, vartype: VARENUM) {
        unsafe { self.record_mut().vt = vartype };
    }
}

pub(crate) struct VariantCopyValue {
    raw: VARIANT,
}

impl VariantCopyValue {
    pub(crate) fn new(value: &VariantValue) -> result::Result<Self> {
        Ok(Self {
            raw: value.copy_raw()?,
        })
    }

    pub(crate) fn as_ref(&self) -> &VARIANT {
        &self.raw
    }
}

impl Drop for VariantCopyValue {
    fn drop(&mut self) {
        let _ = unsafe { VariantClear(&mut self.raw) };
    }
}

pub(crate) struct VariantArrayCopyValue {
    raw: Box<[VARIANT]>,
}

impl VariantArrayCopyValue {
    pub(crate) fn new(values: &[VariantValue]) -> result::Result<Self> {
        let mut raw: Vec<VARIANT> = Vec::with_capacity(values.len());
        for value in values {
            match value.copy_raw() {
                Ok(value) => raw.push(value),
                Err(error) => {
                    for value in &mut raw {
                        let _ = unsafe { VariantClear(value) };
                    }
                    return Err(error);
                }
            }
        }
        Ok(Self {
            raw: raw.into_boxed_slice(),
        })
    }

    pub(crate) fn as_mut_ptr(&mut self) -> *mut VARIANT {
        self.raw.as_mut_ptr()
    }

    pub(crate) fn len(&self) -> usize {
        self.raw.len()
    }
}

impl Drop for VariantArrayCopyValue {
    fn drop(&mut self) {
        for value in &mut self.raw {
            let _ = unsafe { VariantClear(value) };
        }
    }
}

pub(crate) fn variant_size() -> usize {
    std::mem::size_of::<VARIANT>()
}

pub(crate) fn variant_alignment() -> usize {
    std::mem::align_of::<VARIANT>()
}

pub(crate) unsafe fn initialize_variant_slot(slot: *mut std::ffi::c_void) {
    unsafe { slot.cast::<VARIANT>().write(VariantInit()) };
}

pub(crate) unsafe fn clear_variant_slot(slot: *mut std::ffi::c_void) {
    let _ = unsafe { VariantClear(slot.cast::<VARIANT>()) };
}

pub(crate) unsafe fn validate_variant_slot(slot: *const std::ffi::c_void) -> result::Result<()> {
    unsafe { validate_raw_variant(&*slot.cast::<VARIANT>()) }
}

pub(crate) unsafe fn take_variant_slot(
    slot: *mut std::ffi::c_void,
) -> result::Result<VariantValue> {
    let raw = unsafe { slot.cast::<VARIANT>().read() };
    unsafe { slot.cast::<VARIANT>().write(VariantInit()) };
    unsafe { VariantValue::from_owned_raw(raw) }
}

fn validate_raw_variant(raw: &VARIANT) -> result::Result<()> {
    let record = unsafe {
        &*((&raw.Anonymous.Anonymous as *const ManuallyDrop<VARIANT_0_0>).cast::<VARIANT_0_0>())
    };
    variant_type(record.vt.0).map(|_| ())
}

struct DispatchParamsInner {
    raw: UnsafeCell<DISPPARAMS>,
    arguments: Box<[VARIANT]>,
    named_dispids: Box<[i32]>,
}

impl Drop for DispatchParamsInner {
    fn drop(&mut self) {
        for argument in &mut self.arguments {
            let _ = unsafe { VariantClear(argument) };
        }
    }
}

#[derive(Clone)]
pub struct DispatchParamsValue {
    inner: Arc<DispatchParamsInner>,
}

impl std::fmt::Debug for DispatchParamsValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DispatchParamsValue")
            .field("argument_count", &self.argument_count())
            .field("named_dispids", &self.named_dispids())
            .finish()
    }
}

impl DispatchParamsValue {
    pub fn new(arguments: &[VariantValue], named_dispids: &[i32]) -> result::Result<Self> {
        if named_dispids.len() > arguments.len() {
            return Err(invalid_argument(format!(
                "DISPPARAMS named DISPID count {} exceeds argument count {}",
                named_dispids.len(),
                arguments.len()
            )));
        }
        for argument in arguments {
            argument.validate_supported()?;
        }
        let argument_count = u32::try_from(arguments.len())
            .map_err(|_| invalid_argument("DISPPARAMS argument count exceeds UINT"))?;
        let named_count = u32::try_from(named_dispids.len())
            .map_err(|_| invalid_argument("DISPPARAMS named DISPID count exceeds UINT"))?;

        let mut native_arguments = Vec::with_capacity(arguments.len());
        for argument in arguments.iter().rev() {
            match argument.copy_raw() {
                Ok(argument) => native_arguments.push(argument),
                Err(error) => {
                    for argument in &mut native_arguments {
                        let _ = unsafe { VariantClear(argument) };
                    }
                    return Err(error);
                }
            }
        }
        let mut native_arguments = native_arguments.into_boxed_slice();
        let mut native_named_dispids = named_dispids
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let raw = DISPPARAMS {
            rgvarg: if native_arguments.is_empty() {
                std::ptr::null_mut()
            } else {
                native_arguments.as_mut_ptr()
            },
            rgdispidNamedArgs: if native_named_dispids.is_empty() {
                std::ptr::null_mut()
            } else {
                native_named_dispids.as_mut_ptr()
            },
            cArgs: argument_count,
            cNamedArgs: named_count,
        };
        Ok(Self {
            inner: Arc::new(DispatchParamsInner {
                raw: UnsafeCell::new(raw),
                arguments: native_arguments,
                named_dispids: native_named_dispids,
            }),
        })
    }

    pub fn argument_count(&self) -> usize {
        self.inner.arguments.len()
    }

    pub fn named_dispids(&self) -> Vec<i32> {
        self.inner.named_dispids.iter().rev().copied().collect()
    }

    pub(crate) fn raw_mut(&self) -> *mut DISPPARAMS {
        self.inner.raw.get()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcepInfoValue {
    code: u16,
    source: Option<String>,
    description: Option<String>,
    help_file: Option<String>,
    help_context: u32,
    scode: i32,
}

impl ExcepInfoValue {
    pub fn code(&self) -> u16 {
        self.code
    }

    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn help_file(&self) -> Option<&str> {
        self.help_file.as_deref()
    }

    pub fn help_context(&self) -> u32 {
        self.help_context
    }

    pub fn scode(&self) -> i32 {
        self.scode
    }

    pub(crate) fn is_meaningful(&self) -> bool {
        self.code != 0
            || self.source.is_some()
            || self.description.is_some()
            || self.help_file.is_some()
            || self.help_context != 0
            || self.scode != 0
    }
}

pub(crate) struct ExcepInfoOutput {
    raw: EXCEPINFO,
    deferred_invoked: bool,
}

impl ExcepInfoOutput {
    pub(crate) fn new() -> Self {
        Self {
            raw: EXCEPINFO::default(),
            deferred_invoked: false,
        }
    }

    pub(crate) fn as_mut_ptr(&mut self) -> *mut EXCEPINFO {
        &mut self.raw
    }

    fn invoke_deferred(&mut self) -> result::Result<()> {
        let Some(deferred) = self.raw.pfnDeferredFillIn.take() else {
            return Ok(());
        };
        if self.deferred_invoked {
            return Err(invalid_argument(
                "EXCEPINFO deferred fill callback was installed more than once",
            ));
        }
        self.deferred_invoked = true;
        let hr = unsafe { deferred(&mut self.raw) };
        if self.raw.pfnDeferredFillIn.take().is_some() {
            return Err(invalid_argument(
                "EXCEPINFO deferred fill callback installed another callback",
            ));
        }
        hr.ok().map_err(windows_error)
    }

    pub(crate) fn into_value(mut self) -> result::Result<ExcepInfoValue> {
        self.invoke_deferred()?;
        if self.raw.wReserved != 0 || !self.raw.pvReserved.is_null() {
            return Err(invalid_argument(
                "EXCEPINFO reserved fields must be zero/null",
            ));
        }
        let source = take_excep_bstr(&mut self.raw.bstrSource)?;
        let description = take_excep_bstr(&mut self.raw.bstrDescription)?;
        let help_file = take_excep_bstr(&mut self.raw.bstrHelpFile)?;
        Ok(ExcepInfoValue {
            code: self.raw.wCode,
            source,
            description,
            help_file,
            help_context: self.raw.dwHelpContext,
            scode: self.raw.scode,
        })
    }
}

impl Drop for ExcepInfoOutput {
    fn drop(&mut self) {
        drop_excep_bstr(&mut self.raw.bstrSource);
        drop_excep_bstr(&mut self.raw.bstrDescription);
        drop_excep_bstr(&mut self.raw.bstrHelpFile);
        self.raw.pfnDeferredFillIn = None;
    }
}

fn take_excep_bstr(value: &mut ManuallyDrop<BSTR>) -> result::Result<Option<String>> {
    let owned = unsafe { ManuallyDrop::take(value) };
    *value = ManuallyDrop::new(BSTR::new());
    let raw = owned.into_raw();
    if raw.is_null() {
        return Ok(None);
    }
    let owned = unsafe { BSTR::from_raw(raw) };
    String::try_from(&owned)
        .map(Some)
        .map_err(|_| invalid_argument("EXCEPINFO contains an invalid UTF-16 BSTR"))
}

fn drop_excep_bstr(value: &mut ManuallyDrop<BSTR>) {
    let owned = unsafe { ManuallyDrop::take(value) };
    *value = ManuallyDrop::new(BSTR::new());
    drop(owned);
}

fn variant_type(vartype: u16) -> result::Result<VariantType> {
    if vartype & VT_BYREF.0 != 0 {
        return Err(invalid_argument(format!(
            "VARIANT BYREF combinations are not supported (VARTYPE 0x{vartype:04x})"
        )));
    }

    if vartype & VT_ARRAY.0 != 0 {
        if vartype & !(VT_ARRAY.0 | VT_TYPEMASK.0) != 0 {
            return Err(invalid_argument(format!(
                "unsupported VARIANT array flags in VARTYPE 0x{vartype:04x}"
            )));
        }

        return SafeArrayElementType::from_vartype(vartype & VT_TYPEMASK.0)
            .map(VariantType::SafeArray);
    }
    Ok(match vartype {
        value if value == VT_EMPTY.0 => VariantType::Empty,
        value if value == VT_NULL.0 => VariantType::Null,
        value if value == VT_I1.0 => VariantType::I8,
        value if value == VT_UI1.0 => VariantType::U8,
        value if value == VT_I2.0 => VariantType::I16,
        value if value == VT_UI2.0 => VariantType::U16,
        value if value == VT_I4.0 => VariantType::I32,
        value if value == VT_UI4.0 => VariantType::U32,
        value if value == VT_I8.0 => VariantType::I64,
        value if value == VT_UI8.0 => VariantType::U64,
        value if value == VT_INT.0 => VariantType::Int,
        value if value == VT_UINT.0 => VariantType::UInt,
        value if value == VT_R4.0 => VariantType::F32,
        value if value == VT_R8.0 => VariantType::F64,
        value if value == VT_BOOL.0 => VariantType::Bool,
        value if value == VT_BSTR.0 => VariantType::Bstr,
        value if value == VT_UNKNOWN.0 => VariantType::Unknown,
        value if value == VT_DISPATCH.0 => VariantType::Dispatch,
        _ => {
            return Err(invalid_argument(format!(
                "unsupported VARIANT VARTYPE 0x{vartype:04x}"
            )));
        }
    })
}

#[cfg(test)]
pub(super) unsafe fn set_variant_vartype_for_test(raw: *mut VARIANT, vartype: u16) {
    let record = unsafe {
        &mut *((&mut (*raw).Anonymous.Anonymous as *mut ManuallyDrop<VARIANT_0_0>)
            .cast::<VARIANT_0_0>())
    };
    record.vt = VARENUM(vartype);
}

#[cfg(test)]
pub(super) unsafe fn variant_vartype_for_test(raw: *const VARIANT) -> u16 {
    let record = unsafe {
        &*((&(*raw).Anonymous.Anonymous as *const ManuallyDrop<VARIANT_0_0>).cast::<VARIANT_0_0>())
    };
    record.vt.0
}

#[cfg(test)]
pub(super) unsafe fn variant_i32_for_test(raw: *const VARIANT) -> i32 {
    let record = unsafe {
        &*((&(*raw).Anonymous.Anonymous as *const ManuallyDrop<VARIANT_0_0>).cast::<VARIANT_0_0>())
    };
    assert_eq!(record.vt, VT_I4);
    unsafe { record.Anonymous.lVal }
}

#[cfg(test)]
pub(super) unsafe fn variant_bstr_for_test(raw: *const VARIANT) -> String {
    let record = unsafe {
        &*((&(*raw).Anonymous.Anonymous as *const ManuallyDrop<VARIANT_0_0>).cast::<VARIANT_0_0>())
    };
    assert_eq!(record.vt, VT_BSTR);
    unsafe { record.Anonymous.bstrVal.to_string() }
}

#[cfg(test)]
pub(super) unsafe fn variant_unknown_is_non_null_for_test(raw: *const VARIANT) -> bool {
    let record = unsafe {
        &*((&(*raw).Anonymous.Anonymous as *const ManuallyDrop<VARIANT_0_0>).cast::<VARIANT_0_0>())
    };
    assert_eq!(record.vt, VT_UNKNOWN);
    unsafe { record.Anonymous.punkVal.is_some() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeArrayElementType {
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
    Bool,
    Bstr,
    Unknown,
    Dispatch,
    Variant,
}

impl SafeArrayElementType {
    pub const fn vartype(self) -> u16 {
        match self {
            Self::I8 => VT_I1.0,
            Self::U8 => VT_UI1.0,
            Self::I16 => VT_I2.0,
            Self::U16 => VT_UI2.0,
            Self::I32 => VT_I4.0,
            Self::U32 => VT_UI4.0,
            Self::I64 => VT_I8.0,
            Self::U64 => VT_UI8.0,
            Self::F32 => VT_R4.0,
            Self::F64 => VT_R8.0,
            Self::Bool => VT_BOOL.0,
            Self::Bstr => VT_BSTR.0,
            Self::Unknown => VT_UNKNOWN.0,
            Self::Dispatch => VT_DISPATCH.0,
            Self::Variant => VT_VARIANT.0,
        }
    }

    fn element_size(self) -> usize {
        match self {
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 | Self::Bool => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::I64 | Self::U64 | Self::F64 => 8,
            Self::Bstr | Self::Unknown | Self::Dispatch => std::mem::size_of::<usize>(),
            Self::Variant => std::mem::size_of::<VARIANT>(),
        }
    }

    fn element_alignment(self) -> usize {
        match self {
            Self::I8 | Self::U8 => std::mem::align_of::<u8>(),
            Self::I16 | Self::U16 | Self::Bool => std::mem::align_of::<u16>(),
            Self::I32 | Self::U32 | Self::F32 => std::mem::align_of::<u32>(),
            Self::I64 | Self::U64 | Self::F64 => std::mem::align_of::<u64>(),
            Self::Bstr | Self::Unknown | Self::Dispatch => std::mem::align_of::<usize>(),
            Self::Variant => std::mem::align_of::<VARIANT>(),
        }
    }

    fn from_vartype(vartype: u16) -> result::Result<Self> {
        Ok(match vartype {
            value if value == VT_I1.0 => Self::I8,
            value if value == VT_UI1.0 => Self::U8,
            value if value == VT_I2.0 => Self::I16,
            value if value == VT_UI2.0 => Self::U16,
            value if value == VT_I4.0 => Self::I32,
            value if value == VT_UI4.0 => Self::U32,
            value if value == VT_I8.0 => Self::I64,
            value if value == VT_UI8.0 => Self::U64,
            value if value == VT_R4.0 => Self::F32,
            value if value == VT_R8.0 => Self::F64,
            value if value == VT_BOOL.0 => Self::Bool,
            value if value == VT_BSTR.0 => Self::Bstr,
            value if value == VT_UNKNOWN.0 => Self::Unknown,
            value if value == VT_DISPATCH.0 => Self::Dispatch,
            value if value == VT_VARIANT.0 => Self::Variant,
            _ => {
                return Err(invalid_argument(format!(
                    "unsupported SAFEARRAY element VARTYPE 0x{vartype:04x}"
                )));
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafeArrayBound {
    lower_bound: i32,
    length: u32,
}

impl SafeArrayBound {
    pub fn new(lower_bound: i32, length: u32) -> result::Result<Self> {
        let exclusive_upper = i64::from(lower_bound) + i64::from(length);
        if exclusive_upper > i64::from(i32::MAX) + 1 || (length == 0 && lower_bound == i32::MIN) {
            return Err(invalid_argument(
                "SAFEARRAY bound cannot be represented by SafeArrayGetUBound",
            ));
        }
        Ok(Self {
            lower_bound,
            length,
        })
    }

    pub const fn lower_bound(self) -> i32 {
        self.lower_bound
    }

    pub const fn length(self) -> u32 {
        self.length
    }
}

#[derive(Clone)]
pub enum SafeArrayElementValue {
    I8(i8),
    U8(u8),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    F32(f32),
    F64(f64),
    Bool(bool),
    Bstr(String),
    Unknown(Option<IUnknown>),
    Dispatch(Option<IUnknown>),
    Variant(VariantValue),
}

impl std::fmt::Debug for SafeArrayElementValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::I8(value) => formatter.debug_tuple("I8").field(value).finish(),
            Self::U8(value) => formatter.debug_tuple("U8").field(value).finish(),
            Self::I16(value) => formatter.debug_tuple("I16").field(value).finish(),
            Self::U16(value) => formatter.debug_tuple("U16").field(value).finish(),
            Self::I32(value) => formatter.debug_tuple("I32").field(value).finish(),
            Self::U32(value) => formatter.debug_tuple("U32").field(value).finish(),
            Self::I64(value) => formatter.debug_tuple("I64").field(value).finish(),
            Self::U64(value) => formatter.debug_tuple("U64").field(value).finish(),
            Self::F32(value) => formatter.debug_tuple("F32").field(value).finish(),
            Self::F64(value) => formatter.debug_tuple("F64").field(value).finish(),
            Self::Bool(value) => formatter.debug_tuple("Bool").field(value).finish(),
            Self::Bstr(value) => formatter.debug_tuple("Bstr").field(value).finish(),
            Self::Unknown(value) => formatter
                .debug_tuple("Unknown")
                .field(&value.is_some())
                .finish(),
            Self::Dispatch(value) => formatter
                .debug_tuple("Dispatch")
                .field(&value.is_some())
                .finish(),
            Self::Variant(value) => formatter.debug_tuple("Variant").field(value).finish(),
        }
    }
}

struct SafeArrayInner {
    raw: *mut SAFEARRAY,
    element_type: SafeArrayElementType,
    interface_iid: Option<GUID>,
    bounds: Vec<SafeArrayBound>,
    length: usize,
    access_lock: Mutex<()>,
}

pub(crate) struct SafeArrayNativeGuard<'a> {
    _guard: MutexGuard<'a, ()>,
    raw: *mut SAFEARRAY,
}

impl SafeArrayNativeGuard<'_> {
    pub(crate) fn as_raw(&self) -> *mut SAFEARRAY {
        self.raw
    }
}

impl Drop for SafeArrayInner {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            let _ = unsafe { SafeArrayDestroy(self.raw) };
            self.raw = std::ptr::null_mut();
        }
    }
}

#[derive(Clone)]
pub struct SafeArrayValue {
    inner: Arc<SafeArrayInner>,
}

impl std::fmt::Debug for SafeArrayValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SafeArrayValue")
            .field("element_type", &self.element_type())
            .field("bounds", &self.bounds())
            .field("length", &self.len())
            .finish()
    }
}

struct SafeArrayDataGuard {
    array: *mut SAFEARRAY,
    data: *mut c_void,
}

impl SafeArrayDataGuard {
    unsafe fn new(array: *mut SAFEARRAY) -> result::Result<Self> {
        let mut data = std::ptr::null_mut();
        unsafe { SafeArrayAccessData(array, &mut data) }.map_err(windows_error)?;
        Ok(Self { array, data })
    }

    fn finish(mut self) -> result::Result<()> {
        let array = std::mem::replace(&mut self.array, std::ptr::null_mut());
        unsafe { SafeArrayUnaccessData(array) }.map_err(windows_error)
    }
}

impl Drop for SafeArrayDataGuard {
    fn drop(&mut self) {
        if !self.array.is_null() {
            let _ = unsafe { SafeArrayUnaccessData(self.array) };
            self.array = std::ptr::null_mut();
        }
    }
}

pub(crate) struct SafeArrayOutput {
    raw: *mut SAFEARRAY,
}

impl SafeArrayOutput {
    pub(crate) fn new() -> Self {
        Self {
            raw: std::ptr::null_mut(),
        }
    }

    pub(crate) fn as_mut_ptr(&mut self) -> *mut *mut SAFEARRAY {
        &mut self.raw
    }

    pub(crate) fn is_null(&self) -> bool {
        self.raw.is_null()
    }

    pub(crate) fn into_value(
        mut self,
        expected: Option<SafeArrayElementType>,
        expected_interface_iid: Option<GUID>,
    ) -> result::Result<SafeArrayValue> {
        let raw = std::mem::replace(&mut self.raw, std::ptr::null_mut());
        unsafe { SafeArrayValue::from_owned_raw(raw, expected, expected_interface_iid) }
    }
}

impl Drop for SafeArrayOutput {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            let _ = unsafe { SafeArrayDestroy(self.raw) };
            self.raw = std::ptr::null_mut();
        }
    }
}

impl SafeArrayValue {
    pub fn new(
        element_type: SafeArrayElementType,
        bounds: Vec<SafeArrayBound>,
        elements: Vec<SafeArrayElementValue>,
    ) -> result::Result<Self> {
        Self::new_with_identity(element_type, None, bounds, elements)
    }

    pub fn new_interface(
        iid: GUID,
        bounds: Vec<SafeArrayBound>,
        elements: Vec<SafeArrayElementValue>,
    ) -> result::Result<Self> {
        if iid == GUID::zeroed() {
            return Err(invalid_argument("SAFEARRAY interface IID must not be zero"));
        }
        let elements = elements
            .into_iter()
            .map(|element| match element {
                SafeArrayElementValue::Unknown(Some(value)) => {
                    let mut queried = std::ptr::null_mut();
                    unsafe { value.query(&iid, &mut queried) }
                        .ok()
                        .map_err(windows_error)?;
                    Ok(SafeArrayElementValue::Unknown(Some(unsafe {
                        IUnknown::from_raw(queried)
                    })))
                }
                SafeArrayElementValue::Unknown(None) => Ok(SafeArrayElementValue::Unknown(None)),
                _ => Err(invalid_argument(
                    "typed interface SAFEARRAY values require VT_UNKNOWN elements",
                )),
            })
            .collect::<result::Result<Vec<_>>>()?;
        Self::new_with_identity(SafeArrayElementType::Unknown, Some(iid), bounds, elements)
    }

    fn new_with_identity(
        element_type: SafeArrayElementType,
        interface_iid: Option<GUID>,
        bounds: Vec<SafeArrayBound>,
        elements: Vec<SafeArrayElementValue>,
    ) -> result::Result<Self> {
        if interface_iid.is_some() && element_type != SafeArrayElementType::Unknown {
            return Err(invalid_argument(
                "exact interface IID is valid only for VT_UNKNOWN SAFEARRAY elements",
            ));
        }
        validate_safearray_bounds(&bounds)?;
        let length = bounds.iter().try_fold(1usize, |total, bound| {
            total
                .checked_mul(bound.length as usize)
                .ok_or_else(|| invalid_argument("SAFEARRAY element count overflows"))
        })?;
        validate_contiguous_size(length, element_type.element_size(), "SAFEARRAY")?;
        if elements.len() != length {
            return Err(invalid_argument(format!(
                "SAFEARRAY bounds require {length} element(s), received {}",
                elements.len()
            )));
        }
        for element in &elements {
            if safe_array_element_type(element) != element_type {
                return Err(invalid_argument(format!(
                    "SAFEARRAY element type mismatch: expected {element_type:?}, received {:?}",
                    safe_array_element_type(element)
                )));
            }
        }
        let native_bounds = bounds
            .iter()
            .map(|bound| SAFEARRAYBOUND {
                cElements: bound.length,
                lLbound: bound.lower_bound,
            })
            .collect::<Vec<_>>();
        let raw = unsafe {
            if let Some(iid) = interface_iid.as_ref() {
                SafeArrayCreateEx(
                    VARENUM(element_type.vartype()),
                    native_bounds.len() as u32,
                    native_bounds.as_ptr(),
                    (iid as *const GUID).cast(),
                )
            } else {
                SafeArrayCreate(
                    VARENUM(element_type.vartype()),
                    native_bounds.len() as u32,
                    native_bounds.as_ptr(),
                )
            }
        };
        if raw.is_null() {
            return Err(out_of_memory("SafeArrayCreate failed"));
        }
        let output = SafeArrayOutput { raw };
        {
            let guard = unsafe { SafeArrayDataGuard::new(raw)? };
            let write_result = write_safe_array_elements(guard.data, element_type, &elements);
            let unlock_result = guard.finish();
            write_result?;
            unlock_result?;
        }
        output.into_value(Some(element_type), interface_iid)
    }

    pub(crate) unsafe fn from_owned_raw(
        raw: *mut SAFEARRAY,
        expected: Option<SafeArrayElementType>,
        expected_interface_iid: Option<GUID>,
    ) -> result::Result<Self> {
        if raw.is_null() {
            return Err(invalid_argument("owned SAFEARRAY output is null"));
        }
        let mut owner = SafeArrayOutput { raw };
        let vartype = unsafe { SafeArrayGetVartype(raw) }
            .map_err(windows_error)?
            .0;
        let element_type = SafeArrayElementType::from_vartype(vartype)?;
        if expected.is_some_and(|expected| expected != element_type) {
            return Err(invalid_argument(format!(
                "SAFEARRAY VARTYPE mismatch: expected {:?}, received {element_type:?}",
                expected.unwrap()
            )));
        }
        if expected_interface_iid.is_some() && element_type != SafeArrayElementType::Unknown {
            return Err(invalid_argument(
                "exact SAFEARRAY interface IID requires VT_UNKNOWN elements",
            ));
        }
        let descriptor_interface_iid = if element_type == SafeArrayElementType::Unknown {
            match unsafe { SafeArrayGetIID(raw) } {
                Ok(iid) if iid != GUID::zeroed() => Some(iid),
                Ok(_) if expected_interface_iid.is_none() => None,
                Err(error) if error.code() == E_INVALIDARG => None,
                Err(error) => return Err(windows_error(error)),
                Ok(_) => {
                    return Err(invalid_argument(
                        "SAFEARRAY descriptor has no exact interface IID",
                    ));
                }
            }
        } else {
            None
        };
        if let Some(expected_iid) = expected_interface_iid {
            match descriptor_interface_iid {
                Some(actual_iid) if actual_iid == expected_iid || actual_iid == IUnknown::IID => {}
                None => {}
                Some(actual_iid) => {
                    return Err(invalid_argument(format!(
                        "SAFEARRAY interface IID mismatch: expected {expected_iid:?}, received {actual_iid:?}"
                    )));
                }
            }
        }
        let rank = unsafe { SafeArrayGetDim(raw) } as usize;
        if rank == 0 || rank > MAX_SAFEARRAY_RANK {
            return Err(invalid_argument(format!(
                "SAFEARRAY rank {rank} is unsupported; supported ranks are 1 through {MAX_SAFEARRAY_RANK}"
            )));
        }
        let element_size = unsafe { SafeArrayGetElemsize(raw) } as usize;
        if element_size != element_type.element_size() {
            return Err(invalid_argument(format!(
                "SAFEARRAY element width mismatch: VARTYPE {element_type:?} requires {} bytes, descriptor reports {element_size}",
                element_type.element_size()
            )));
        }
        let mut bounds = Vec::with_capacity(rank);
        for dimension in 1..=rank {
            let lower =
                unsafe { SafeArrayGetLBound(raw, dimension as u32) }.map_err(windows_error)?;
            let upper =
                unsafe { SafeArrayGetUBound(raw, dimension as u32) }.map_err(windows_error)?;
            let length = if upper < lower {
                0
            } else {
                u32::try_from(i64::from(upper) - i64::from(lower) + 1)
                    .map_err(|_| invalid_argument("SAFEARRAY bound length exceeds u32"))?
            };
            bounds.push(SafeArrayBound::new(lower, length)?);
        }
        let length = bounds.iter().try_fold(1usize, |total, bound| {
            total
                .checked_mul(bound.length as usize)
                .ok_or_else(|| invalid_argument("SAFEARRAY element count overflows"))
        })?;
        validate_contiguous_size(length, element_size, "SAFEARRAY")?;
        {
            let guard = unsafe { SafeArrayDataGuard::new(raw)? };
            let validation_result = validate_data_pointer(
                length,
                guard.data,
                element_type.element_alignment(),
                "SAFEARRAY",
            )
            .and_then(|_| {
                if let Some(expected_iid) = expected_interface_iid {
                    validate_safe_array_interface_elements(guard.data, length, expected_iid)?;
                }
                Ok(())
            });
            let unlock_result = guard.finish();
            validation_result?;
            unlock_result?;
        }
        owner.raw = std::ptr::null_mut();
        Ok(Self {
            inner: Arc::new(SafeArrayInner {
                raw,
                element_type,
                interface_iid: expected_interface_iid.or(descriptor_interface_iid),
                bounds,
                length,
                access_lock: Mutex::new(()),
            }),
        })
    }

    pub fn element_type(&self) -> SafeArrayElementType {
        self.inner.element_type
    }

    pub fn interface_iid(&self) -> Option<GUID> {
        self.inner.interface_iid
    }

    pub fn bounds(&self) -> &[SafeArrayBound] {
        &self.inner.bounds
    }

    pub fn len(&self) -> usize {
        self.inner.length
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn elements(&self) -> result::Result<Vec<SafeArrayElementValue>> {
        let _access = self
            .inner
            .access_lock
            .lock()
            .map_err(|_| invalid_argument("SAFEARRAY access lock is poisoned"))?;
        let guard = unsafe { SafeArrayDataGuard::new(self.inner.raw)? };
        let read_result =
            read_safe_array_elements(guard.data, self.inner.element_type, self.inner.length);
        let unlock_result = guard.finish();
        let values = read_result?;
        unlock_result?;
        Ok(values)
    }

    pub(crate) fn lock_native(&self) -> result::Result<SafeArrayNativeGuard<'_>> {
        let guard = self
            .inner
            .access_lock
            .lock()
            .map_err(|_| invalid_argument("SAFEARRAY access lock is poisoned"))?;
        Ok(SafeArrayNativeGuard {
            _guard: guard,
            raw: self.inner.raw,
        })
    }

    pub(crate) fn identity(&self) -> usize {
        self.inner.raw as usize
    }
}

fn validate_safe_array_interface_elements(
    data: *mut c_void,
    length: usize,
    expected_iid: GUID,
) -> result::Result<()> {
    if length == 0 {
        return Ok(());
    }
    let elements = data.cast::<*mut c_void>();
    for index in 0..length {
        let raw = unsafe { *elements.add(index) };
        let Some(value) = (unsafe { IUnknown::from_raw_borrowed(&raw) }) else {
            continue;
        };
        let mut queried = std::ptr::null_mut();
        let query_result = unsafe { value.query(&expected_iid, &mut queried) }.ok();
        if let Err(error) = query_result {
            if !queried.is_null() {
                drop(unsafe { IUnknown::from_raw(queried) });
            }
            return Err(invalid_argument(format!(
                "SAFEARRAY interface element {index} does not support expected IID {expected_iid:?}: {error}"
            )));
        }
        if queried.is_null() {
            return Err(invalid_argument(format!(
                "SAFEARRAY interface element {index} returned null for expected IID {expected_iid:?}"
            )));
        }
        drop(unsafe { IUnknown::from_raw(queried) });
    }
    Ok(())
}

fn validate_safearray_bounds(bounds: &[SafeArrayBound]) -> result::Result<()> {
    if bounds.is_empty() || bounds.len() > MAX_SAFEARRAY_RANK {
        return Err(invalid_argument(format!(
            "SAFEARRAY ranks must be between 1 and {MAX_SAFEARRAY_RANK}"
        )));
    }
    Ok(())
}

fn safe_array_element_type(value: &SafeArrayElementValue) -> SafeArrayElementType {
    match value {
        SafeArrayElementValue::I8(_) => SafeArrayElementType::I8,
        SafeArrayElementValue::U8(_) => SafeArrayElementType::U8,
        SafeArrayElementValue::I16(_) => SafeArrayElementType::I16,
        SafeArrayElementValue::U16(_) => SafeArrayElementType::U16,
        SafeArrayElementValue::I32(_) => SafeArrayElementType::I32,
        SafeArrayElementValue::U32(_) => SafeArrayElementType::U32,
        SafeArrayElementValue::I64(_) => SafeArrayElementType::I64,
        SafeArrayElementValue::U64(_) => SafeArrayElementType::U64,
        SafeArrayElementValue::F32(_) => SafeArrayElementType::F32,
        SafeArrayElementValue::F64(_) => SafeArrayElementType::F64,
        SafeArrayElementValue::Bool(_) => SafeArrayElementType::Bool,
        SafeArrayElementValue::Bstr(_) => SafeArrayElementType::Bstr,
        SafeArrayElementValue::Unknown(_) => SafeArrayElementType::Unknown,
        SafeArrayElementValue::Dispatch(_) => SafeArrayElementType::Dispatch,
        SafeArrayElementValue::Variant(_) => SafeArrayElementType::Variant,
    }
}

fn write_safe_array_elements(
    data: *mut c_void,
    element_type: SafeArrayElementType,
    values: &[SafeArrayElementValue],
) -> result::Result<()> {
    if values.is_empty() {
        return Ok(());
    }
    if data.is_null() {
        return Err(invalid_argument(
            "SafeArrayAccessData returned null storage for a non-empty array",
        ));
    }
    validate_data_pointer(
        values.len(),
        data,
        element_type.element_alignment(),
        "SAFEARRAY",
    )?;
    unsafe {
        macro_rules! write_plain {
            ($variant:ident, $native:ty) => {{
                let output = data.cast::<$native>();
                for (index, value) in values.iter().enumerate() {
                    let SafeArrayElementValue::$variant(value) = value else {
                        unreachable!("validated SAFEARRAY element kind")
                    };
                    output.add(index).write(*value);
                }
            }};
        }
        match element_type {
            SafeArrayElementType::I8 => write_plain!(I8, i8),
            SafeArrayElementType::U8 => write_plain!(U8, u8),
            SafeArrayElementType::I16 => write_plain!(I16, i16),
            SafeArrayElementType::U16 => write_plain!(U16, u16),
            SafeArrayElementType::I32 => write_plain!(I32, i32),
            SafeArrayElementType::U32 => write_plain!(U32, u32),
            SafeArrayElementType::I64 => write_plain!(I64, i64),
            SafeArrayElementType::U64 => write_plain!(U64, u64),
            SafeArrayElementType::F32 => write_plain!(F32, f32),
            SafeArrayElementType::F64 => write_plain!(F64, f64),
            SafeArrayElementType::Bool => {
                let output = data.cast::<VARIANT_BOOL>();
                for (index, value) in values.iter().enumerate() {
                    let SafeArrayElementValue::Bool(value) = value else {
                        unreachable!("validated SAFEARRAY element kind")
                    };
                    output
                        .add(index)
                        .write(VARIANT_BOOL(if *value { -1 } else { 0 }));
                }
            }
            SafeArrayElementType::Bstr => {
                let output = data.cast::<BSTR>();
                for (index, value) in values.iter().enumerate() {
                    let SafeArrayElementValue::Bstr(value) = value else {
                        unreachable!("validated SAFEARRAY element kind")
                    };
                    let utf16 = value.encode_utf16().collect::<Vec<_>>();
                    let bstr = SysAllocStringLen(Some(&utf16));
                    if bstr.is_empty() && !utf16.is_empty() {
                        return Err(out_of_memory(
                            "SysAllocStringLen failed for SAFEARRAY element",
                        ));
                    }
                    output.add(index).write(bstr);
                }
            }
            SafeArrayElementType::Unknown => {
                let output = data.cast::<*mut c_void>();
                for (index, value) in values.iter().enumerate() {
                    let SafeArrayElementValue::Unknown(value) = value else {
                        unreachable!("validated SAFEARRAY element kind")
                    };
                    output.add(index).write(
                        value
                            .as_ref()
                            .map_or(std::ptr::null_mut(), |value| value.clone().into_raw()),
                    );
                }
            }
            SafeArrayElementType::Dispatch => {
                let output = data.cast::<*mut c_void>();
                for (index, value) in values.iter().enumerate() {
                    let SafeArrayElementValue::Dispatch(value) = value else {
                        unreachable!("validated SAFEARRAY element kind")
                    };
                    let dispatch = value
                        .as_ref()
                        .map(|value| value.cast::<IDispatch>().map_err(windows_error))
                        .transpose()?;
                    output
                        .add(index)
                        .write(dispatch.map_or(std::ptr::null_mut(), |value| value.into_raw()));
                }
            }
            SafeArrayElementType::Variant => {
                let output = data.cast::<VARIANT>();
                for (index, value) in values.iter().enumerate() {
                    let SafeArrayElementValue::Variant(value) = value else {
                        unreachable!("validated SAFEARRAY element kind")
                    };
                    output.add(index).write(VariantInit());
                    VariantCopy(output.add(index), value.raw()).map_err(windows_error)?;
                }
            }
        }
    }
    Ok(())
}

fn read_safe_array_elements(
    data: *mut c_void,
    element_type: SafeArrayElementType,
    length: usize,
) -> result::Result<Vec<SafeArrayElementValue>> {
    if length == 0 {
        return Ok(Vec::new());
    }
    if data.is_null() {
        return Err(invalid_argument(
            "SafeArrayAccessData returned null storage for a non-empty array",
        ));
    }
    validate_contiguous_size(length, element_type.element_size(), "SAFEARRAY")?;
    validate_data_pointer(length, data, element_type.element_alignment(), "SAFEARRAY")?;
    let mut values = Vec::with_capacity(length);
    unsafe {
        for index in 0..length {
            values.push(match element_type {
                SafeArrayElementType::I8 => {
                    SafeArrayElementValue::I8(*data.cast::<i8>().add(index))
                }
                SafeArrayElementType::U8 => {
                    SafeArrayElementValue::U8(*data.cast::<u8>().add(index))
                }
                SafeArrayElementType::I16 => {
                    SafeArrayElementValue::I16(*data.cast::<i16>().add(index))
                }
                SafeArrayElementType::U16 => {
                    SafeArrayElementValue::U16(*data.cast::<u16>().add(index))
                }
                SafeArrayElementType::I32 => {
                    SafeArrayElementValue::I32(*data.cast::<i32>().add(index))
                }
                SafeArrayElementType::U32 => {
                    SafeArrayElementValue::U32(*data.cast::<u32>().add(index))
                }
                SafeArrayElementType::I64 => {
                    SafeArrayElementValue::I64(*data.cast::<i64>().add(index))
                }
                SafeArrayElementType::U64 => {
                    SafeArrayElementValue::U64(*data.cast::<u64>().add(index))
                }
                SafeArrayElementType::F32 => {
                    SafeArrayElementValue::F32(*data.cast::<f32>().add(index))
                }
                SafeArrayElementType::F64 => {
                    SafeArrayElementValue::F64(*data.cast::<f64>().add(index))
                }
                SafeArrayElementType::Bool => {
                    let value = (*data.cast::<VARIANT_BOOL>().add(index)).0;
                    if !matches!(value, 0 | -1) {
                        return Err(invalid_argument(format!(
                            "SAFEARRAY VT_BOOL element must be 0 or -1, received {value}"
                        )));
                    }
                    SafeArrayElementValue::Bool(value == -1)
                }
                SafeArrayElementType::Bstr => {
                    let value = &*data.cast::<BSTR>().add(index);
                    SafeArrayElementValue::Bstr(value.to_string())
                }
                SafeArrayElementType::Unknown => {
                    let raw = *data.cast::<*mut c_void>().add(index);
                    let value = IUnknown::from_raw_borrowed(&raw).cloned();
                    SafeArrayElementValue::Unknown(value)
                }
                SafeArrayElementType::Dispatch => {
                    let raw = *data.cast::<*mut c_void>().add(index);
                    let value = IDispatch::from_raw_borrowed(&raw)
                        .map(|value| value.cast::<IUnknown>().map_err(windows_error))
                        .transpose()?;
                    SafeArrayElementValue::Dispatch(value)
                }
                SafeArrayElementType::Variant => {
                    let mut copy = VariantInit();
                    VariantCopy(&mut copy, data.cast::<VARIANT>().add(index))
                        .map_err(windows_error)?;
                    let value = VariantValue::from_initialized(copy);
                    value.validate_supported()?;
                    SafeArrayElementValue::Variant(value)
                }
            });
        }
    }
    Ok(values)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropVariantType {
    Empty,
    Null,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    Int,
    UInt,
    F32,
    F64,
    Bool,
    String,
    Guid,
    FileTime,
    Blob,
    Vector(PropVariantVectorType),
}

impl PropVariantType {
    pub const fn vartype(self) -> u16 {
        match self {
            Self::Empty => VT_EMPTY.0,
            Self::Null => VT_NULL.0,
            Self::I8 => VT_I1.0,
            Self::U8 => VT_UI1.0,
            Self::I16 => VT_I2.0,
            Self::U16 => VT_UI2.0,
            Self::I32 => VT_I4.0,
            Self::U32 => VT_UI4.0,
            Self::I64 => VT_I8.0,
            Self::U64 => VT_UI8.0,
            Self::Int => VT_INT.0,
            Self::UInt => VT_UINT.0,
            Self::F32 => VT_R4.0,
            Self::F64 => VT_R8.0,
            Self::Bool => VT_BOOL.0,
            Self::String => VT_LPWSTR.0,
            Self::Guid => VT_CLSID.0,
            Self::FileTime => VT_FILETIME.0,
            Self::Blob => VT_BLOB.0,
            Self::Vector(element) => VT_VECTOR.0 | element.vartype(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropVariantVectorType {
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
    Bool,
    String,
    Guid,
    FileTime,
}

impl PropVariantVectorType {
    pub const fn vartype(self) -> u16 {
        match self {
            Self::I8 => VT_I1.0,
            Self::U8 => VT_UI1.0,
            Self::I16 => VT_I2.0,
            Self::U16 => VT_UI2.0,
            Self::I32 => VT_I4.0,
            Self::U32 => VT_UI4.0,
            Self::I64 => VT_I8.0,
            Self::U64 => VT_UI8.0,
            Self::F32 => VT_R4.0,
            Self::F64 => VT_R8.0,
            Self::Bool => VT_BOOL.0,
            Self::String => VT_LPWSTR.0,
            Self::Guid => VT_CLSID.0,
            Self::FileTime => VT_FILETIME.0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum PropVariantData {
    Empty,
    Null,
    I8(i8),
    U8(u8),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    Int(i32),
    UInt(u32),
    F32(f32),
    F64(f64),
    Bool(bool),
    String(String),
    Guid(GUID),
    FileTime(u64),
    Blob(Vec<u8>),
    Vector(PropVariantVector),
}

#[derive(Debug, Clone)]
pub enum PropVariantVector {
    I8(Vec<i8>),
    U8(Vec<u8>),
    I16(Vec<i16>),
    U16(Vec<u16>),
    I32(Vec<i32>),
    U32(Vec<u32>),
    I64(Vec<i64>),
    U64(Vec<u64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
    Bool(Vec<bool>),
    String(Vec<String>),
    Guid(Vec<GUID>),
    FileTime(Vec<u64>),
}

struct PropVariantInner {
    raw: UnsafeCell<PROPVARIANT>,
}

// The PROPVARIANT is immutable after publication. Native mutation is limited
// to fresh output storage before the Arc is returned to callers.
unsafe impl Send for PropVariantInner {}
unsafe impl Sync for PropVariantInner {}

impl Drop for PropVariantInner {
    fn drop(&mut self) {
        let _ = unsafe { PropVariantClear(self.raw.get()) };
    }
}

#[derive(Clone)]
pub struct PropVariantValue {
    inner: Arc<PropVariantInner>,
}

impl std::fmt::Debug for PropVariantValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PropVariantValue")
            .field("vartype", &self.vartype())
            .finish()
    }
}

impl PropVariantValue {
    fn init() -> PROPVARIANT {
        // PropVariantInit is an inline Windows SDK helper that zero-initializes
        // the complete PROPVARIANT, including VT_EMPTY.
        unsafe { std::mem::zeroed() }
    }

    fn from_initialized(raw: PROPVARIANT) -> Self {
        Self {
            inner: Arc::new(PropVariantInner {
                raw: UnsafeCell::new(raw),
            }),
        }
    }

    unsafe fn record(&self) -> &PROPVARIANT_0_0 {
        unsafe {
            &*((&(*self.raw()).Anonymous.Anonymous as *const ManuallyDrop<PROPVARIANT_0_0>)
                .cast::<PROPVARIANT_0_0>())
        }
    }

    unsafe fn record_mut(&self) -> &mut PROPVARIANT_0_0 {
        unsafe {
            &mut *((&mut (*self.raw_mut()).Anonymous.Anonymous
                as *mut ManuallyDrop<PROPVARIANT_0_0>)
                .cast::<PROPVARIANT_0_0>())
        }
    }

    unsafe fn payload(&self) -> &PROPVARIANT_0_0_0 {
        unsafe { &self.record().Anonymous }
    }

    unsafe fn payload_mut(&self) -> &mut PROPVARIANT_0_0_0 {
        unsafe { &mut self.record_mut().Anonymous }
    }

    pub fn empty() -> Self {
        Self::from_initialized(Self::init())
    }

    pub fn null() -> Self {
        let result = Self::empty();
        unsafe { result.set_vartype(VT_NULL) };
        result
    }

    pub fn from_i8(value: i8) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_I1);
            result.payload_mut().cVal = value;
        }
        result
    }

    pub fn from_u8(value: u8) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_UI1);
            result.payload_mut().bVal = value;
        }
        result
    }

    pub fn from_i16(value: i16) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_I2);
            result.payload_mut().iVal = value;
        }
        result
    }

    pub fn from_u16(value: u16) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_UI2);
            result.payload_mut().uiVal = value;
        }
        result
    }

    pub fn from_i32(value: i32) -> Self {
        Self::from_i32_with_type(value, VT_I4)
    }

    pub fn from_int(value: i32) -> Self {
        Self::from_i32_with_type(value, VT_INT)
    }

    fn from_i32_with_type(value: i32, vartype: VARENUM) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(vartype);
            if vartype == VT_INT {
                result.payload_mut().intVal = value;
            } else {
                result.payload_mut().lVal = value;
            }
        }
        result
    }

    pub fn from_u32(value: u32) -> Self {
        Self::from_u32_with_type(value, VT_UI4)
    }

    pub fn from_uint(value: u32) -> Self {
        Self::from_u32_with_type(value, VT_UINT)
    }

    fn from_u32_with_type(value: u32, vartype: VARENUM) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(vartype);
            if vartype == VT_UINT {
                result.payload_mut().uintVal = value;
            } else {
                result.payload_mut().ulVal = value;
            }
        }
        result
    }

    pub fn from_i64(value: i64) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_I8);
            result.payload_mut().hVal = value;
        }
        result
    }

    pub fn from_u64(value: u64) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_UI8);
            result.payload_mut().uhVal = value;
        }
        result
    }

    pub fn from_f32(value: f32) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_R4);
            result.payload_mut().fltVal = value;
        }
        result
    }

    pub fn from_f64(value: f64) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_R8);
            result.payload_mut().dblVal = value;
        }
        result
    }

    pub fn from_bool(value: bool) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_BOOL);
            result.payload_mut().boolVal = VARIANT_BOOL(if value { -1 } else { 0 });
        }
        result
    }

    pub fn from_string(value: &str) -> result::Result<Self> {
        let string = allocate_cotaskmem_wide(value)?;
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_LPWSTR);
            result.payload_mut().pwszVal = string;
        }
        Ok(result)
    }

    pub fn from_guid(value: GUID) -> result::Result<Self> {
        let ptr = unsafe { CoTaskMemAlloc(std::mem::size_of::<GUID>()) }.cast::<GUID>();
        if ptr.is_null() {
            return Err(out_of_memory("CoTaskMemAlloc failed for PROPVARIANT GUID"));
        }
        unsafe { ptr.write(value) };
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_CLSID);
            result.payload_mut().puuid = ptr;
        }
        Ok(result)
    }

    pub fn from_filetime(value: u64) -> Self {
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_FILETIME);
            result.payload_mut().filetime = u64_to_filetime(value);
        }
        result
    }

    pub fn from_blob(value: &[u8]) -> result::Result<Self> {
        let length = u32::try_from(value.len())
            .map_err(|_| invalid_argument("PROPVARIANT blob length exceeds u32"))?;
        let ptr = allocate_cotaskmem_bytes(value)?;
        let result = Self::empty();
        unsafe {
            result.set_vartype(VT_BLOB);
            result.payload_mut().blob = BLOB {
                cbSize: length,
                pBlobData: ptr,
            };
        }
        Ok(result)
    }

    pub fn from_vector(value: PropVariantVector) -> result::Result<Self> {
        let result = Self::empty();
        write_propvariant_vector(&result, value)?;
        Ok(result)
    }

    pub fn vartype(&self) -> u16 {
        unsafe { self.record().vt.0 }
    }

    pub fn propvariant_type(&self) -> result::Result<PropVariantType> {
        propvariant_type(self.vartype())
    }

    pub fn data(&self) -> result::Result<PropVariantData> {
        let typ = self.propvariant_type()?;
        let payload = unsafe { self.payload() };
        Ok(match typ {
            PropVariantType::Empty => PropVariantData::Empty,
            PropVariantType::Null => PropVariantData::Null,
            PropVariantType::I8 => PropVariantData::I8(unsafe { payload.cVal }),
            PropVariantType::U8 => PropVariantData::U8(unsafe { payload.bVal }),
            PropVariantType::I16 => PropVariantData::I16(unsafe { payload.iVal }),
            PropVariantType::U16 => PropVariantData::U16(unsafe { payload.uiVal }),
            PropVariantType::I32 => PropVariantData::I32(unsafe { payload.lVal }),
            PropVariantType::U32 => PropVariantData::U32(unsafe { payload.ulVal }),
            PropVariantType::I64 => PropVariantData::I64(unsafe { payload.hVal }),
            PropVariantType::U64 => PropVariantData::U64(unsafe { payload.uhVal }),
            PropVariantType::Int => PropVariantData::Int(unsafe { payload.intVal }),
            PropVariantType::UInt => PropVariantData::UInt(unsafe { payload.uintVal }),
            PropVariantType::F32 => PropVariantData::F32(unsafe { payload.fltVal }),
            PropVariantType::F64 => PropVariantData::F64(unsafe { payload.dblVal }),
            PropVariantType::Bool => {
                let value = unsafe { payload.boolVal.0 };
                if !matches!(value, 0 | -1) {
                    return Err(invalid_argument(format!(
                        "PROPVARIANT VT_BOOL payload must be 0 or -1, received {value}"
                    )));
                }
                PropVariantData::Bool(value == -1)
            }
            PropVariantType::String => {
                PropVariantData::String(read_wide_string(unsafe { payload.pwszVal })?)
            }
            PropVariantType::Guid => {
                let ptr = unsafe { payload.puuid };
                if ptr.is_null() {
                    return Err(invalid_argument("PROPVARIANT VT_CLSID pointer is null"));
                }
                PropVariantData::Guid(unsafe { *ptr })
            }
            PropVariantType::FileTime => {
                PropVariantData::FileTime(filetime_to_u64(unsafe { payload.filetime }))
            }
            PropVariantType::Blob => {
                let blob = unsafe { payload.blob };
                if blob.cbSize > 0 && blob.pBlobData.is_null() {
                    return Err(invalid_argument(
                        "PROPVARIANT VT_BLOB has a null pointer with non-zero length",
                    ));
                }
                PropVariantData::Blob(if blob.cbSize == 0 {
                    Vec::new()
                } else {
                    validate_contiguous_size(blob.cbSize as usize, 1, "PROPVARIANT blob")?;
                    unsafe {
                        std::slice::from_raw_parts(blob.pBlobData, blob.cbSize as usize).to_vec()
                    }
                })
            }
            PropVariantType::Vector(element) => {
                PropVariantData::Vector(read_propvariant_vector(payload, element)?)
            }
        })
    }

    pub(crate) fn validate_supported(&self) -> result::Result<()> {
        self.propvariant_type().map(|_| ())
    }

    pub(crate) fn raw(&self) -> *const PROPVARIANT {
        self.inner.raw.get()
    }

    pub(crate) fn raw_mut(&self) -> *mut PROPVARIANT {
        self.inner.raw.get()
    }

    unsafe fn set_vartype(&self, vartype: VARENUM) {
        unsafe { self.record_mut().vt = vartype };
    }
}

fn propvariant_type(vartype: u16) -> result::Result<PropVariantType> {
    if vartype & VT_BYREF.0 != 0 {
        return Err(invalid_argument(format!(
            "PROPVARIANT BYREF combinations are not supported (VARTYPE 0x{vartype:04x})"
        )));
    }
    if vartype & VT_VECTOR.0 != 0 {
        if vartype & !(VT_VECTOR.0 | VT_TYPEMASK.0) != 0 {
            return Err(invalid_argument(format!(
                "unsupported PROPVARIANT vector flags in VARTYPE 0x{vartype:04x}"
            )));
        }
        return propvariant_vector_type(vartype & VT_TYPEMASK.0).map(PropVariantType::Vector);
    }
    Ok(match vartype {
        value if value == VT_EMPTY.0 => PropVariantType::Empty,
        value if value == VT_NULL.0 => PropVariantType::Null,
        value if value == VT_I1.0 => PropVariantType::I8,
        value if value == VT_UI1.0 => PropVariantType::U8,
        value if value == VT_I2.0 => PropVariantType::I16,
        value if value == VT_UI2.0 => PropVariantType::U16,
        value if value == VT_I4.0 => PropVariantType::I32,
        value if value == VT_UI4.0 => PropVariantType::U32,
        value if value == VT_I8.0 => PropVariantType::I64,
        value if value == VT_UI8.0 => PropVariantType::U64,
        value if value == VT_INT.0 => PropVariantType::Int,
        value if value == VT_UINT.0 => PropVariantType::UInt,
        value if value == VT_R4.0 => PropVariantType::F32,
        value if value == VT_R8.0 => PropVariantType::F64,
        value if value == VT_BOOL.0 => PropVariantType::Bool,
        value if value == VT_LPWSTR.0 => PropVariantType::String,
        value if value == VT_CLSID.0 => PropVariantType::Guid,
        value if value == VT_FILETIME.0 => PropVariantType::FileTime,
        value if value == VT_BLOB.0 => PropVariantType::Blob,
        _ => {
            return Err(invalid_argument(format!(
                "unsupported PROPVARIANT VARTYPE 0x{vartype:04x}"
            )));
        }
    })
}

fn propvariant_vector_type(vartype: u16) -> result::Result<PropVariantVectorType> {
    Ok(match vartype {
        value if value == VT_I1.0 => PropVariantVectorType::I8,
        value if value == VT_UI1.0 => PropVariantVectorType::U8,
        value if value == VT_I2.0 => PropVariantVectorType::I16,
        value if value == VT_UI2.0 => PropVariantVectorType::U16,
        value if value == VT_I4.0 => PropVariantVectorType::I32,
        value if value == VT_UI4.0 => PropVariantVectorType::U32,
        value if value == VT_I8.0 => PropVariantVectorType::I64,
        value if value == VT_UI8.0 => PropVariantVectorType::U64,
        value if value == VT_R4.0 => PropVariantVectorType::F32,
        value if value == VT_R8.0 => PropVariantVectorType::F64,
        value if value == VT_BOOL.0 => PropVariantVectorType::Bool,
        value if value == VT_LPWSTR.0 => PropVariantVectorType::String,
        value if value == VT_CLSID.0 => PropVariantVectorType::Guid,
        value if value == VT_FILETIME.0 => PropVariantVectorType::FileTime,
        value if value == VT_VARIANT.0 => {
            return Err(invalid_argument(
                "nested VT_VECTOR | VT_VARIANT PROPVARIANT values are not supported",
            ));
        }
        _ => {
            return Err(invalid_argument(format!(
                "unsupported PROPVARIANT vector element VARTYPE 0x{vartype:04x}"
            )));
        }
    })
}

fn write_propvariant_vector(
    output: &PropVariantValue,
    vector: PropVariantVector,
) -> result::Result<()> {
    unsafe {
        macro_rules! write_plain_vector {
            ($values:expr, $vt:expr, $ca:ident, $native:ty, $container:ident) => {{
                let values = $values;
                let count = u32::try_from(values.len())
                    .map_err(|_| invalid_argument("PROPVARIANT vector length exceeds u32"))?;
                let ptr = allocate_cotaskmem_slice::<$native>(values.len())?;
                if !ptr.is_null() {
                    std::ptr::copy_nonoverlapping(values.as_ptr(), ptr, values.len());
                }
                output.set_vartype(VARENUM(VT_VECTOR.0 | $vt.0));
                output.payload_mut().$ca = $container {
                    cElems: count,
                    pElems: ptr,
                };
            }};
        }
        match vector {
            PropVariantVector::I8(values) => {
                let count = u32::try_from(values.len())
                    .map_err(|_| invalid_argument("PROPVARIANT vector length exceeds u32"))?;
                let ptr = allocate_cotaskmem_slice::<i8>(values.len())?;
                if !ptr.is_null() {
                    std::ptr::copy_nonoverlapping(values.as_ptr(), ptr, values.len());
                }
                output.set_vartype(VARENUM(VT_VECTOR.0 | VT_I1.0));
                output.payload_mut().cac = CAC {
                    cElems: count,
                    pElems: windows_core::PSTR(ptr.cast()),
                };
            }
            PropVariantVector::U8(values) => {
                write_plain_vector!(values, VT_UI1, caub, u8, CAUB)
            }
            PropVariantVector::I16(values) => {
                write_plain_vector!(values, VT_I2, cai, i16, CAI)
            }
            PropVariantVector::U16(values) => {
                write_plain_vector!(values, VT_UI2, caui, u16, CAUI)
            }
            PropVariantVector::I32(values) => {
                write_plain_vector!(values, VT_I4, cal, i32, CAL)
            }
            PropVariantVector::U32(values) => {
                write_plain_vector!(values, VT_UI4, caul, u32, CAUL)
            }
            PropVariantVector::I64(values) => {
                write_plain_vector!(values, VT_I8, cah, i64, CAH)
            }
            PropVariantVector::U64(values) => {
                write_plain_vector!(values, VT_UI8, cauh, u64, CAUH)
            }
            PropVariantVector::F32(values) => {
                write_plain_vector!(values, VT_R4, caflt, f32, CAFLT)
            }
            PropVariantVector::F64(values) => {
                write_plain_vector!(values, VT_R8, cadbl, f64, CADBL)
            }
            PropVariantVector::Bool(values) => {
                let count = u32::try_from(values.len())
                    .map_err(|_| invalid_argument("PROPVARIANT vector length exceeds u32"))?;
                let ptr = allocate_cotaskmem_slice::<VARIANT_BOOL>(values.len())?;
                for (index, value) in values.iter().enumerate() {
                    ptr.add(index)
                        .write(VARIANT_BOOL(if *value { -1 } else { 0 }));
                }
                output.set_vartype(VARENUM(VT_VECTOR.0 | VT_BOOL.0));
                output.payload_mut().cabool = CABOOL {
                    cElems: count,
                    pElems: ptr,
                };
            }
            PropVariantVector::String(values) => {
                let count = u32::try_from(values.len())
                    .map_err(|_| invalid_argument("PROPVARIANT vector length exceeds u32"))?;
                let ptr = allocate_cotaskmem_slice::<PWSTR>(values.len())?;
                if !ptr.is_null() {
                    std::ptr::write_bytes(ptr, 0, values.len());
                }
                output.set_vartype(VARENUM(VT_VECTOR.0 | VT_LPWSTR.0));
                output.payload_mut().calpwstr = CALPWSTR {
                    cElems: count,
                    pElems: ptr,
                };
                for (index, value) in values.iter().enumerate() {
                    ptr.add(index).write(allocate_cotaskmem_wide(value)?);
                }
            }
            PropVariantVector::Guid(values) => {
                let count = u32::try_from(values.len())
                    .map_err(|_| invalid_argument("PROPVARIANT vector length exceeds u32"))?;
                let ptr = allocate_cotaskmem_slice::<GUID>(values.len())?;
                if !ptr.is_null() {
                    std::ptr::copy_nonoverlapping(values.as_ptr(), ptr, values.len());
                }
                output.set_vartype(VARENUM(VT_VECTOR.0 | VT_CLSID.0));
                output.payload_mut().cauuid = CACLSID {
                    cElems: count,
                    pElems: ptr,
                };
            }
            PropVariantVector::FileTime(values) => {
                let count = u32::try_from(values.len())
                    .map_err(|_| invalid_argument("PROPVARIANT vector length exceeds u32"))?;
                let ptr = allocate_cotaskmem_slice::<FILETIME>(values.len())?;
                for (index, value) in values.iter().enumerate() {
                    ptr.add(index).write(u64_to_filetime(*value));
                }
                output.set_vartype(VARENUM(VT_VECTOR.0 | VT_FILETIME.0));
                output.payload_mut().cafiletime = CAFILETIME {
                    cElems: count,
                    pElems: ptr,
                };
            }
        }
    }
    Ok(())
}

fn read_propvariant_vector(
    payload: &windows::Win32::System::Com::StructuredStorage::PROPVARIANT_0_0_0,
    typ: PropVariantVectorType,
) -> result::Result<PropVariantVector> {
    unsafe {
        macro_rules! read_plain_vector {
            ($ca:ident, $variant:ident, $native:ty) => {{
                let ca = payload.$ca;
                let length = validate_vector_pointer(ca.cElems, ca.pElems.cast::<$native>())?;
                PropVariantVector::$variant(if length == 0 {
                    Vec::new()
                } else {
                    std::slice::from_raw_parts(ca.pElems.cast::<$native>(), length).to_vec()
                })
            }};
        }
        Ok(match typ {
            PropVariantVectorType::I8 => {
                let ca = payload.cac;
                let length = validate_vector_pointer(ca.cElems, ca.pElems.0.cast::<i8>())?;
                PropVariantVector::I8(if length == 0 {
                    Vec::new()
                } else {
                    std::slice::from_raw_parts(ca.pElems.0.cast::<i8>(), length).to_vec()
                })
            }
            PropVariantVectorType::U8 => read_plain_vector!(caub, U8, u8),
            PropVariantVectorType::I16 => read_plain_vector!(cai, I16, i16),
            PropVariantVectorType::U16 => read_plain_vector!(caui, U16, u16),
            PropVariantVectorType::I32 => read_plain_vector!(cal, I32, i32),
            PropVariantVectorType::U32 => read_plain_vector!(caul, U32, u32),
            PropVariantVectorType::I64 => read_plain_vector!(cah, I64, i64),
            PropVariantVectorType::U64 => read_plain_vector!(cauh, U64, u64),
            PropVariantVectorType::F32 => read_plain_vector!(caflt, F32, f32),
            PropVariantVectorType::F64 => read_plain_vector!(cadbl, F64, f64),
            PropVariantVectorType::Bool => {
                let ca = payload.cabool;
                let length = validate_vector_pointer(ca.cElems, ca.pElems)?;
                let mut values = Vec::with_capacity(length);
                let elements = if length == 0 {
                    &[]
                } else {
                    std::slice::from_raw_parts(ca.pElems, length)
                };
                for value in elements {
                    if !matches!(value.0, 0 | -1) {
                        return Err(invalid_argument(format!(
                            "PROPVARIANT vector VT_BOOL element must be 0 or -1, received {}",
                            value.0
                        )));
                    }
                    values.push(value.0 == -1);
                }
                PropVariantVector::Bool(values)
            }
            PropVariantVectorType::String => {
                let ca = payload.calpwstr;
                let length = validate_vector_pointer(ca.cElems, ca.pElems)?;
                let mut values = Vec::with_capacity(length);
                let elements = if length == 0 {
                    &[]
                } else {
                    std::slice::from_raw_parts(ca.pElems, length)
                };
                for value in elements {
                    values.push(read_wide_string(*value)?);
                }
                PropVariantVector::String(values)
            }
            PropVariantVectorType::Guid => {
                let ca = payload.cauuid;
                let length = validate_vector_pointer(ca.cElems, ca.pElems)?;
                PropVariantVector::Guid(if length == 0 {
                    Vec::new()
                } else {
                    std::slice::from_raw_parts(ca.pElems, length).to_vec()
                })
            }
            PropVariantVectorType::FileTime => {
                let ca = payload.cafiletime;
                let length = validate_vector_pointer(ca.cElems, ca.pElems)?;
                let elements = if length == 0 {
                    &[]
                } else {
                    std::slice::from_raw_parts(ca.pElems, length)
                };
                PropVariantVector::FileTime(elements.iter().copied().map(filetime_to_u64).collect())
            }
        })
    }
}

fn validate_vector_pointer<T>(count: u32, ptr: *const T) -> result::Result<usize> {
    let length = count as usize;
    validate_contiguous_size(length, std::mem::size_of::<T>(), "PROPVARIANT vector")?;
    validate_data_pointer(
        length,
        ptr.cast_mut().cast(),
        std::mem::align_of::<T>(),
        "PROPVARIANT vector",
    )?;
    Ok(length)
}

fn validate_contiguous_size(
    length: usize,
    element_size: usize,
    name: &str,
) -> result::Result<usize> {
    let byte_length = length
        .checked_mul(element_size)
        .ok_or_else(|| invalid_argument(format!("{name} byte length overflows")))?;
    if byte_length > isize::MAX as usize {
        return Err(invalid_argument(format!(
            "{name} byte length exceeds the addressable slice limit"
        )));
    }
    Ok(byte_length)
}

fn validate_data_pointer(
    length: usize,
    ptr: *mut c_void,
    alignment: usize,
    name: &str,
) -> result::Result<()> {
    if length == 0 {
        return Ok(());
    }
    if ptr.is_null() {
        return Err(invalid_argument(format!(
            "{name} has a null pointer with non-zero length"
        )));
    }
    if ptr as usize % alignment != 0 {
        return Err(invalid_argument(format!(
            "{name} pointer does not satisfy {alignment}-byte alignment"
        )));
    }
    Ok(())
}

fn allocate_cotaskmem_wide(value: &str) -> result::Result<PWSTR> {
    if value.encode_utf16().any(|unit| unit == 0) {
        return Err(invalid_argument(
            "PROPVARIANT LPWSTR values cannot contain embedded NUL characters",
        ));
    }
    let utf16 = value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let bytes = utf16
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .ok_or_else(|| invalid_argument("UTF-16 allocation size overflows"))?;
    let ptr = unsafe { CoTaskMemAlloc(bytes) }.cast::<u16>();
    if ptr.is_null() {
        return Err(out_of_memory(
            "CoTaskMemAlloc failed for PROPVARIANT string",
        ));
    }
    unsafe { std::ptr::copy_nonoverlapping(utf16.as_ptr(), ptr, utf16.len()) };
    Ok(PWSTR(ptr))
}

fn read_wide_string(value: PWSTR) -> result::Result<String> {
    if value.is_null() {
        return Ok(String::new());
    }
    let mut length = 0usize;
    unsafe {
        while *value.0.add(length) != 0 {
            length = length
                .checked_add(1)
                .ok_or_else(|| invalid_argument("PROPVARIANT string length overflows"))?;
        }
        String::from_utf16(std::slice::from_raw_parts(value.0, length))
            .map_err(|_| invalid_argument("PROPVARIANT string is not valid UTF-16"))
    }
}

fn allocate_cotaskmem_bytes(value: &[u8]) -> result::Result<*mut u8> {
    if value.is_empty() {
        return Ok(std::ptr::null_mut());
    }
    let ptr = unsafe { CoTaskMemAlloc(value.len()) }.cast::<u8>();
    if ptr.is_null() {
        return Err(out_of_memory("CoTaskMemAlloc failed for PROPVARIANT blob"));
    }
    unsafe { std::ptr::copy_nonoverlapping(value.as_ptr(), ptr, value.len()) };
    Ok(ptr)
}

unsafe fn allocate_cotaskmem_slice<T>(length: usize) -> result::Result<*mut T> {
    if length == 0 {
        return Ok(std::ptr::null_mut());
    }
    let bytes = validate_contiguous_size(length, std::mem::size_of::<T>(), "PROPVARIANT vector")?;
    let ptr = unsafe { CoTaskMemAlloc(bytes) }.cast::<T>();
    if ptr.is_null() {
        Err(out_of_memory(
            "CoTaskMemAlloc failed for PROPVARIANT vector",
        ))
    } else {
        Ok(ptr)
    }
}

fn u64_to_filetime(value: u64) -> FILETIME {
    FILETIME {
        dwLowDateTime: value as u32,
        dwHighDateTime: (value >> 32) as u32,
    }
}

fn filetime_to_u64(value: FILETIME) -> u64 {
    u64::from(value.dwLowDateTime) | (u64::from(value.dwHighDateTime) << 32)
}

pub(crate) fn cleanup_safearray(raw: *mut c_void) {
    if !raw.is_null() {
        let _ = unsafe { SafeArrayDestroy(raw.cast()) };
    }
}

pub(crate) fn cleanup_variant(raw: *mut c_void) {
    if !raw.is_null() {
        let _ = unsafe { VariantClear(raw.cast()) };
    }
}

pub(crate) fn cleanup_propvariant(raw: *mut c_void) {
    if !raw.is_null() {
        let _ = unsafe { PropVariantClear(raw.cast()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    unsafe fn variant_i32(raw: *const VARIANT) -> i32 {
        let record = unsafe {
            &*((&(*raw).Anonymous.Anonymous as *const ManuallyDrop<VARIANT_0_0>)
                .cast::<VARIANT_0_0>())
        };
        assert_eq!(record.vt, VT_I4);
        unsafe { record.Anonymous.lVal }
    }

    #[test]
    fn automation_compound_abi_matches_windows_rs() {
        assert_eq!(offset_of!(DISPPARAMS, rgvarg), 0);
        assert_eq!(
            offset_of!(DISPPARAMS, rgdispidNamedArgs),
            size_of::<usize>()
        );
        assert_eq!(offset_of!(DISPPARAMS, cArgs), size_of::<usize>() * 2);
        assert_eq!(
            offset_of!(DISPPARAMS, cNamedArgs),
            size_of::<usize>() * 2 + 4
        );

        assert_eq!(offset_of!(EXCEPINFO, wCode), 0);
        assert_eq!(offset_of!(EXCEPINFO, wReserved), 2);
        assert_eq!(offset_of!(EXCEPINFO, bstrSource), size_of::<usize>());
        assert_eq!(
            offset_of!(EXCEPINFO, bstrDescription),
            size_of::<usize>() * 2
        );
        assert_eq!(offset_of!(EXCEPINFO, bstrHelpFile), size_of::<usize>() * 3);
        assert_eq!(offset_of!(EXCEPINFO, dwHelpContext), size_of::<usize>() * 4);

        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!((size_of::<DISPPARAMS>(), align_of::<DISPPARAMS>()), (24, 8));
            assert_eq!((size_of::<EXCEPINFO>(), align_of::<EXCEPINFO>()), (64, 8));
            assert_eq!(offset_of!(EXCEPINFO, pvReserved), 40);
            assert_eq!(offset_of!(EXCEPINFO, pfnDeferredFillIn), 48);
            assert_eq!(offset_of!(EXCEPINFO, scode), 56);
        }
        #[cfg(target_pointer_width = "32")]
        {
            assert_eq!((size_of::<DISPPARAMS>(), align_of::<DISPPARAMS>()), (16, 4));
            assert_eq!((size_of::<EXCEPINFO>(), align_of::<EXCEPINFO>()), (32, 4));
            assert_eq!(offset_of!(EXCEPINFO, pvReserved), 20);
            assert_eq!(offset_of!(EXCEPINFO, pfnDeferredFillIn), 24);
            assert_eq!(offset_of!(EXCEPINFO, scode), 28);
        }
    }

    #[test]
    fn dispatch_params_reverses_arguments_and_named_dispids_in_stable_storage() {
        let value = DispatchParamsValue::new(
            &[
                VariantValue::from_i32(10),
                VariantValue::from_i32(20),
                VariantValue::from_i32(30),
            ],
            &[100, 200],
        )
        .unwrap();
        let raw = unsafe { &*value.raw_mut() };
        assert_eq!((raw.cArgs, raw.cNamedArgs), (3, 2));
        assert!(!raw.rgvarg.is_null());
        assert!(!raw.rgdispidNamedArgs.is_null());
        assert_eq!(
            unsafe { std::slice::from_raw_parts(raw.rgdispidNamedArgs, raw.cNamedArgs as usize) },
            &[200, 100]
        );
        assert_eq!(unsafe { variant_i32(raw.rgvarg) }, 30);
        assert_eq!(unsafe { variant_i32(raw.rgvarg.add(1)) }, 20);
        assert_eq!(unsafe { variant_i32(raw.rgvarg.add(2)) }, 10);
        assert_eq!(
            unsafe { raw.rgvarg.add(1).byte_offset_from(raw.rgvarg) },
            size_of::<VARIANT>() as isize
        );

        let clone = value.clone();
        assert_eq!(clone.raw_mut(), value.raw_mut());
        assert_eq!(clone.named_dispids(), [100, 200]);
        assert!(DispatchParamsValue::new(&[VariantValue::empty()], &[1, 2]).is_err());
    }

    #[test]
    fn variant_bool_uses_automation_true() {
        let value = VariantValue::from_bool(true);
        assert_eq!(value.vartype(), VT_BOOL.0);
        assert!(matches!(value.data().unwrap(), VariantData::Bool(true)));
        assert_eq!(unsafe { value.payload().boolVal.0 }, -1);
    }

    #[test]
    fn byref_and_next_unsupported_variant_fail_closed() {
        assert!(variant_type(VT_BYREF.0 | VT_I4.0).is_err());
        assert!(variant_type(windows::Win32::System::Variant::VT_DATE.0).is_err());
    }

    #[test]
    fn safearray_preserves_bounds_and_typed_elements() {
        let array = SafeArrayValue::new(
            SafeArrayElementType::I32,
            vec![
                SafeArrayBound::new(-2, 2).unwrap(),
                SafeArrayBound::new(5, 2).unwrap(),
            ],
            vec![
                SafeArrayElementValue::I32(1),
                SafeArrayElementValue::I32(2),
                SafeArrayElementValue::I32(3),
                SafeArrayElementValue::I32(4),
            ],
        )
        .unwrap();
        let native = array.lock_native().unwrap();
        let (mut lower, mut upper) = unsafe {
            (
                SafeArrayGetLBound(native.as_raw(), 1).unwrap(),
                SafeArrayGetUBound(native.as_raw(), 1).unwrap(),
            )
        };
        assert_eq!((lower, upper), (-2, -1));
        (lower, upper) = unsafe {
            (
                SafeArrayGetLBound(native.as_raw(), 2).unwrap(),
                SafeArrayGetUBound(native.as_raw(), 2).unwrap(),
            )
        };
        assert_eq!((lower, upper), (5, 6));
        drop(native);
        assert_eq!(array.bounds()[0], SafeArrayBound::new(-2, 2).unwrap());
        assert_eq!(array.bounds()[1], SafeArrayBound::new(5, 2).unwrap());
        assert!(matches!(
            array.elements().unwrap().as_slice(),
            [
                SafeArrayElementValue::I32(1),
                SafeArrayElementValue::I32(2),
                SafeArrayElementValue::I32(3),
                SafeArrayElementValue::I32(4)
            ]
        ));
    }

    #[test]
    fn safearray_rejects_mismatched_elements_and_rank() {
        assert!(
            SafeArrayValue::new(
                SafeArrayElementType::I32,
                vec![SafeArrayBound::new(0, 1).unwrap()],
                vec![SafeArrayElementValue::U32(1)],
            )
            .is_err()
        );
        assert!(SafeArrayValue::new(SafeArrayElementType::I32, vec![], vec![],).is_err());
        assert!(SafeArrayBound::new(i32::MAX, 2).is_err());
        assert!(SafeArrayBound::new(i32::MIN, 0).is_err());
    }

    #[test]
    fn propvariant_scalar_owned_and_vector_values_roundtrip() {
        let string = PropVariantValue::from_string("dynwinrt").unwrap();
        assert!(matches!(
            string.data().unwrap(),
            PropVariantData::String(value) if value == "dynwinrt"
        ));

        let guid = GUID::from_u128(0x11223344_5566_7788_99aa_bbccddeeff00);
        let vector = PropVariantValue::from_vector(PropVariantVector::Guid(vec![guid])).unwrap();
        assert!(matches!(
            vector.data().unwrap(),
            PropVariantData::Vector(PropVariantVector::Guid(values)) if values == vec![guid]
        ));

        let bools = PropVariantValue::from_vector(PropVariantVector::Bool(Vec::new())).unwrap();
        assert!(matches!(
            bools.data().unwrap(),
            PropVariantData::Vector(PropVariantVector::Bool(values)) if values.is_empty()
        ));
        let strings = PropVariantValue::from_vector(PropVariantVector::String(Vec::new())).unwrap();
        assert!(matches!(
            strings.data().unwrap(),
            PropVariantData::Vector(PropVariantVector::String(values)) if values.is_empty()
        ));
        let filetimes =
            PropVariantValue::from_vector(PropVariantVector::FileTime(Vec::new())).unwrap();
        assert!(matches!(
            filetimes.data().unwrap(),
            PropVariantData::Vector(PropVariantVector::FileTime(values)) if values.is_empty()
        ));
    }

    #[test]
    fn propvariant_nested_vector_fails_closed() {
        assert!(propvariant_type(VT_VECTOR.0 | VT_VARIANT.0).is_err());
        assert!(propvariant_type(windows::Win32::System::Variant::VT_STREAM.0).is_err());
    }

    #[test]
    fn propvariant_strings_reject_embedded_nuls() {
        assert!(PropVariantValue::from_string("before\0after").is_err());
        assert!(
            PropVariantValue::from_vector(PropVariantVector::String(vec![
                "valid".into(),
                "bad\0value".into(),
            ]))
            .is_err()
        );
    }
}
