// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use libffi::middle::Arg;
use windows::Win32::System::WinRT::IActivationFactory;
use windows_core::{GUID, IUnknown, Interface};
use windows_future::IAsyncInfo;

use crate::{
    metadata_table::{TypeHandle, TypeKind},
    result,
};

#[derive(Debug)]
pub struct ArrayOfIUnknownData(pub windows::core::Array<IUnknown>);

pub use crate::array::ArrayData;

impl Clone for ArrayOfIUnknownData {
    fn clone(&self) -> Self {
        let mut arr = windows::core::Array::<IUnknown>::with_len(self.0.len());
        for i in 0..self.0.len() {
            arr[i] = self.0[i].clone();
        }
        ArrayOfIUnknownData(arr)
    }
}

/// Metadata for a dynamic WinRT async operation.
#[derive(Debug, Clone)]
pub struct AsyncInfo {
    pub info: IAsyncInfo,
    pub async_type: TypeHandle,
}

impl AsyncInfo {
    pub fn iid(&self) -> GUID {
        self.async_type.iid().expect("async type must have IID")
    }

    pub fn handler_iid(&self) -> GUID {
        self.async_type
            .completed_handler_iid()
            .expect("async type must have handler IID")
    }

    pub fn result_type(&self) -> Option<TypeHandle> {
        match self.async_type.kind() {
            TypeKind::IAsyncOperation(_idx) | TypeKind::IAsyncOperationWithProgress(_idx) => {
                let inner = match self.async_type.kind() {
                    TypeKind::IAsyncOperation(idx) => self.async_type.table().get_inner_type(idx),
                    TypeKind::IAsyncOperationWithProgress(idx) => {
                        self.async_type.table().get_inner_type_pair(idx).0
                    }
                    _ => unreachable!(),
                };
                Some(self.async_type.table().make(inner))
            }
            _ => None,
        }
    }

    pub fn progress_type(&self) -> Option<TypeHandle> {
        match self.async_type.kind() {
            TypeKind::IAsyncActionWithProgress(idx) => {
                let inner = self.async_type.table().get_inner_type(idx);
                Some(self.async_type.table().make(inner))
            }
            TypeKind::IAsyncOperationWithProgress(idx) => {
                let (_, progress) = self.async_type.table().get_inner_type_pair(idx);
                Some(self.async_type.table().make(progress))
            }
            _ => None,
        }
    }

    pub fn progress_handler_iid(&self) -> Option<GUID> {
        self.async_type.progress_handler_iid()
    }

    /// Register a progress handler on a WithProgress async operation (vtable index 6 = put_Progress).
    pub fn set_progress_handler(&self, handler: &IUnknown) -> result::Result<()> {
        match self.async_type.kind() {
            TypeKind::IAsyncActionWithProgress(_) | TypeKind::IAsyncOperationWithProgress(_) => {
                let iid = self.iid();
                let mut concrete_ptr = std::ptr::null_mut();
                unsafe { self.info.query(&iid, &mut concrete_ptr) }
                    .ok()
                    .map_err(result::Error::WindowsError)?;
                let concrete = unsafe { IUnknown::from_raw(concrete_ptr) };
                // put_Progress is at vtable index 6 for WithProgress types
                // (IUnknown[0-2], IInspectable[3-5], put_Progress[6])
                let hr = crate::call::call_winrt_method_1(6, concrete.as_raw(), handler.as_raw());
                hr.ok().map_err(result::Error::WindowsError)?;
                Ok(())
            }
            _ => Err(result::Error::WindowsError(
                windows_core::Error::from_hresult(windows_core::HRESULT(0x80070057u32 as i32)),
            )),
        }
    }

    /// Cancel the async operation. Causes the operation to transition to
    /// `AsyncStatus::Canceled` (asynchronously); subsequent awaits will reject
    /// with `Error::Canceled`.
    ///
    /// Calling cancel on an already-completed or already-canceled operation is
    /// a no-op (success).
    pub fn cancel(&self) -> result::Result<()> {
        self.info.Cancel().map_err(result::Error::WindowsError)
    }
}

#[derive(Debug, Clone)]
pub enum WinRTValue {
    Bool(bool),
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
    Object(IUnknown),
    /// Null COM object pointer. Separate from Object because IUnknown::from_raw(null)
    /// crashes on clone/drop (dereferences null vtable pointer).
    Null,
    HString(windows_core::HSTRING),
    HResult(windows_core::HRESULT),
    Guid(windows_core::GUID),
    /// Raw pointer buffer for COM out-parameters. Avoids IUnknown::from_raw(null) UB.
    /// COM writes a valid pointer into this slot; after the call, from_out() wraps it.
    RawPtr(*mut std::ffi::c_void),
    OutValue(*mut std::ffi::c_void, TypeHandle),
    Async(AsyncInfo),
    ArrayOfIUnknown(ArrayOfIUnknownData),
    Enum {
        value: i32,
        type_handle: TypeHandle,
    },
    Struct(crate::metadata_table::ValueTypeData),
    Array(ArrayData),
}
unsafe impl Send for WinRTValue {}
unsafe impl Sync for WinRTValue {}

impl WinRTValue {
    pub fn from_activation_factory(name: &windows::core::HSTRING) -> result::Result<WinRTValue> {
        let factory = unsafe {
            windows::Win32::System::WinRT::RoGetActivationFactory::<IActivationFactory>(name)
        };
        match factory {
            Ok(factory) => Ok(WinRTValue::Object(factory.cast()?)),
            Err(e) => Err(result::Error::WindowsError(e)),
        }
    }

    pub fn as_hstring(&self) -> Option<windows::core::HSTRING> {
        match self {
            WinRTValue::HString(hstr) => Some((*hstr).clone()),
            _ => None,
        }
    }

    pub fn as_i32(&self) -> Option<i32> {
        match self {
            WinRTValue::Bool(b) => Some(*b as i32),
            WinRTValue::I8(v) => Some(*v as i32),
            WinRTValue::U8(v) => Some(*v as i32),
            WinRTValue::I16(v) => Some(*v as i32),
            WinRTValue::U16(v) => Some(*v as i32),
            WinRTValue::I32(v) => Some(*v),
            WinRTValue::U32(v) => Some(*v as i32),
            WinRTValue::Enum { value, .. } => Some(*value),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<IUnknown> {
        match self {
            WinRTValue::Object(obj) => {
                if obj.as_raw().is_null() {
                    None
                } else {
                    Some(obj.clone())
                }
            }
            WinRTValue::Async(a) => Some(a.info.cast().ok()?),
            _ => None,
        }
    }

    /// Returns true if this value is a null COM object pointer.
    pub fn is_null_object(&self) -> bool {
        matches!(self, WinRTValue::Null)
    }

    /// If this is an Object wrapping a null IUnknown, replace with Null to prevent
    /// crash on clone/drop (IUnknown::from_raw(null) is invalid).
    pub fn sanitize_null_object(&mut self) {
        let is_null = matches!(self, WinRTValue::Object(o) if o.as_raw().is_null());
        if is_null {
            // mem::forget the null IUnknown to prevent Drop from calling Release on null
            let old = std::mem::replace(self, WinRTValue::Null);
            if let WinRTValue::Object(o) = old {
                std::mem::forget(o);
            }
        }
    }

    pub fn cast(&self, iid: &GUID) -> result::Result<WinRTValue> {
        match self {
            WinRTValue::Object(obj) => {
                let mut result = std::ptr::null_mut();
                unsafe { obj.query(iid, &mut result) }.ok()?;
                Ok(WinRTValue::Object(unsafe { IUnknown::from_raw(result) }))
            }
            _ => Err(result::Error::ExpectObjectTypeError(self.get_type_kind())),
        }
    }

    pub fn get_type_kind(&self) -> TypeKind {
        match self {
            WinRTValue::Bool(_) => TypeKind::Bool,
            WinRTValue::I8(_) => TypeKind::I8,
            WinRTValue::U8(_) => TypeKind::U8,
            WinRTValue::I16(_) => TypeKind::I16,
            WinRTValue::U16(_) => TypeKind::U16,
            WinRTValue::I32(_) => TypeKind::I32,
            WinRTValue::Enum { type_handle, .. } => type_handle.kind(),
            WinRTValue::U32(_) => TypeKind::U32,
            WinRTValue::I64(_) => TypeKind::I64,
            WinRTValue::U64(_) => TypeKind::U64,
            WinRTValue::F32(_) => TypeKind::F32,
            WinRTValue::F64(_) => TypeKind::F64,
            WinRTValue::Object(_) | WinRTValue::Null | WinRTValue::RawPtr(_) => TypeKind::Object,
            WinRTValue::HString(_) => TypeKind::HString,
            WinRTValue::HResult(_) => TypeKind::HResult,
            WinRTValue::Guid(_) => TypeKind::Guid,
            WinRTValue::OutValue(_, handle) => handle.kind(),
            WinRTValue::Async(_) => TypeKind::Object,
            WinRTValue::ArrayOfIUnknown(_) => TypeKind::ArrayOfIUnknown,
            WinRTValue::Struct(data) => data.type_handle().kind(),
            WinRTValue::Array(data) => data.element_type.kind(),
        }
    }

    pub fn out_ptr(&mut self) -> *mut std::ffi::c_void {
        match self {
            WinRTValue::Bool(v) => v as *mut bool as _,
            WinRTValue::I8(v) => v as *mut i8 as _,
            WinRTValue::U8(v) => v as *mut u8 as _,
            WinRTValue::I16(v) => v as *mut i16 as _,
            WinRTValue::U16(v) => v as *mut u16 as _,
            WinRTValue::I32(v) => v as *mut i32 as _,
            WinRTValue::Enum { value, .. } => value as *mut i32 as _,
            WinRTValue::U32(v) => v as *mut u32 as _,
            WinRTValue::I64(v) => v as *mut i64 as _,
            WinRTValue::U64(v) => v as *mut u64 as _,
            WinRTValue::F32(v) => v as *mut f32 as _,
            WinRTValue::F64(v) => v as *mut f64 as _,
            WinRTValue::HString(s) => s as *mut windows_core::HSTRING as _,
            WinRTValue::Object(o) => o as *mut IUnknown as _,
            WinRTValue::HResult(hr) => hr as *mut windows_core::HRESULT as _,
            WinRTValue::Guid(g) => g as *mut windows_core::GUID as _,
            WinRTValue::RawPtr(p) => p as *mut *mut std::ffi::c_void as *mut std::ffi::c_void,
            WinRTValue::OutValue(ptr, _) => *ptr,
            WinRTValue::ArrayOfIUnknown(data) => data.0.as_ptr() as *mut std::ffi::c_void,
            WinRTValue::Null => panic!("Cannot get out_ptr for Null value"),
            WinRTValue::Async(_) => panic!("Cannot get out_ptr for async value"),
            WinRTValue::Struct(data) => data.as_mut_ptr() as *mut std::ffi::c_void,
            WinRTValue::Array(_) => {
                panic!("Cannot get out_ptr for Array; arrays expand to two ABI parameters")
            }
        }
    }

    pub fn libffi_arg(&self) -> Arg<'_> {
        use libffi::middle::arg;
        match &self {
            WinRTValue::Bool(v) => arg(v),
            WinRTValue::I8(v) => arg(v),
            WinRTValue::U8(v) => arg(v),
            WinRTValue::I16(v) => arg(v),
            WinRTValue::U16(v) => arg(v),
            WinRTValue::I32(v) => arg(v),
            WinRTValue::Enum { value, .. } => arg(value),
            WinRTValue::U32(v) => arg(v),
            WinRTValue::I64(v) => arg(v),
            WinRTValue::U64(v) => arg(v),
            WinRTValue::F32(v) => arg(v),
            WinRTValue::F64(v) => arg(v),
            WinRTValue::Object(p) => arg(p),
            WinRTValue::HString(hstr) => arg(hstr),
            WinRTValue::HResult(hr) => arg(hr),
            WinRTValue::Guid(g) => arg(g),
            WinRTValue::RawPtr(p) => arg(p),
            WinRTValue::OutValue(p, _) => arg(p),
            WinRTValue::Null => arg(&std::ptr::null::<std::ffi::c_void>()),
            WinRTValue::Async(_) => panic!("Cannot pass async value as libffi arg"),
            WinRTValue::ArrayOfIUnknown(data) => arg(&data.0),
            WinRTValue::Struct(data) => unsafe { arg(&*data.as_ptr()) },
            WinRTValue::Array(_) => {
                panic!("Cannot pass Array as single libffi arg; arrays expand to two args")
            }
        }
    }

    pub fn as_array(&self) -> Option<&ArrayData> {
        match self {
            WinRTValue::Array(data) => Some(data),
            _ => None,
        }
    }

    pub fn as_struct(&self) -> Option<&crate::metadata_table::ValueTypeData> {
        match self {
            WinRTValue::Struct(data) => Some(data),
            _ => None,
        }
    }

    pub fn as_struct_mut(&mut self) -> Option<&mut crate::metadata_table::ValueTypeData> {
        match self {
            WinRTValue::Struct(data) => Some(data),
            _ => None,
        }
    }
}
