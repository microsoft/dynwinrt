// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use core::ffi::c_void;
use std::ptr;

use libffi::middle::Type;
use windows::Win32::System::Com::{CoTaskMemAlloc, CoTaskMemFree};
use windows_core::{GUID, HRESULT, HSTRING, IInspectable, IUnknown, Interface};

use crate::{
    ArrayData, TypeHandle, TypeKind, ValueTypeData, WinRTValue, native_callback::CallbackAbiType,
    signature::WinRtParameterDirection,
};

use super::{
    E_OUTOFMEMORY, E_POINTER, error, invalid_argument,
    plan::{MethodPlan, ParameterPlan},
};

#[derive(Debug, Clone)]
pub(super) struct ValuePlan {
    pub typ: TypeHandle,
    pub size: usize,
    pub align: usize,
    iid: Option<GUID>,
    fields: Vec<(usize, ValuePlan)>,
}

impl ValuePlan {
    pub fn new(typ: TypeHandle) -> windows_core::Result<Self> {
        Self::with_depth(typ, 0)
    }

    fn with_depth(typ: TypeHandle, depth: usize) -> windows_core::Result<Self> {
        if depth > 64 {
            return Err(invalid_argument("WinRT struct nesting exceeds 64 levels"));
        }
        match typ.kind() {
            TypeKind::Array(_) => {
                return Err(invalid_argument(
                    "Nested arrays and array-valued struct fields are not WinRT callback values",
                ));
            }
            TypeKind::Generic { .. } => {
                return Err(invalid_argument(
                    "Open generic types cannot appear in a WinRT callback",
                ));
            }
            TypeKind::OutValue(_) | TypeKind::ArrayOfIUnknown => {
                return Err(invalid_argument(
                    "Legacy ABI-only types do not describe a complete WinRT callback contract",
                ));
            }
            _ => {}
        }
        typ.table()
            .try_closed_signature_string_kind(typ.kind())
            .map_err(|cause| invalid_argument(cause.message()))?;
        let iid = if typ.kind().is_com_pointer() {
            let iid = if typ.kind() == TypeKind::Object {
                IInspectable::IID
            } else {
                typ.iid().ok_or_else(|| {
                    invalid_argument("WinRT callback reference has no resolved IID")
                })?
            };
            if iid == GUID::zeroed() {
                return Err(invalid_argument(
                    "WinRT callback reference has an empty IID",
                ));
            }
            Some(iid)
        } else {
            None
        };
        let size = typ.size_of();
        let align = typ.align_of();
        if size == 0 || align == 0 || !align.is_power_of_two() || size % align != 0 {
            return Err(invalid_argument(
                "WinRT callback value has an incomplete ABI layout",
            ));
        }
        let mut fields = Vec::new();
        if matches!(typ.kind(), TypeKind::Struct(_)) {
            let mut end = 0;
            for index in 0..typ.field_count() {
                let field = Self::with_depth(typ.field_type(index), depth + 1)?;
                let offset = typ.field_offset(index);
                let field_end = offset
                    .checked_add(field.size)
                    .ok_or_else(|| invalid_argument("WinRT struct field layout overflows"))?;
                if offset < end || offset % field.align != 0 || field_end > size {
                    return Err(invalid_argument("WinRT struct field layout is incomplete"));
                }
                end = field_end;
                fields.push((offset, field));
            }
            if fields.is_empty() {
                return Err(invalid_argument(
                    "Empty WinRT structs cannot be passed by value",
                ));
            }
        }
        Ok(Self {
            typ,
            size,
            align,
            iid,
            fields,
        })
    }

    pub fn callback_abi(&self) -> (CallbackAbiType, Type) {
        let abi = match self.typ.kind() {
            TypeKind::I8 => CallbackAbiType::I8,
            TypeKind::Bool | TypeKind::U8 => CallbackAbiType::U8,
            TypeKind::I16 => CallbackAbiType::I16,
            TypeKind::U16 | TypeKind::Char16 => CallbackAbiType::U16,
            TypeKind::I32 | TypeKind::Enum(_) | TypeKind::HResult => CallbackAbiType::I32,
            TypeKind::U32 => CallbackAbiType::U32,
            TypeKind::I64 => CallbackAbiType::I64,
            TypeKind::U64 => CallbackAbiType::U64,
            TypeKind::F32 => CallbackAbiType::F32,
            TypeKind::F64 => CallbackAbiType::F64,
            TypeKind::Guid => CallbackAbiType::Guid,
            TypeKind::Struct(_) => {
                CallbackAbiType::NativeStruct(self.typ.signature_string(), self.size)
            }
            _ => CallbackAbiType::Pointer,
        };
        (abi, self.typ.libffi_type())
    }

    pub fn array_bytes(&self, length: usize) -> windows_core::Result<usize> {
        length
            .checked_mul(self.size)
            .filter(|size| *size <= isize::MAX as usize)
            .ok_or_else(|| {
                invalid_argument("WinRT array byte length overflows the native address size")
            })
    }

    pub unsafe fn read_borrowed(&self, source: *const c_void) -> windows_core::Result<WinRTValue> {
        if source.is_null() {
            return Err(error(E_POINTER, "Null WinRT callback value storage"));
        }
        macro_rules! scalar {
            ($ty:ty, $variant:ident) => {
                WinRTValue::$variant(unsafe { source.cast::<$ty>().read_unaligned() })
            };
        }
        Ok(match self.typ.kind() {
            TypeKind::Bool => WinRTValue::Bool(unsafe { source.cast::<u8>().read() } != 0),
            TypeKind::I8 => scalar!(i8, I8),
            TypeKind::U8 => scalar!(u8, U8),
            TypeKind::I16 => scalar!(i16, I16),
            TypeKind::U16 | TypeKind::Char16 => scalar!(u16, U16),
            TypeKind::I32 => scalar!(i32, I32),
            TypeKind::U32 => scalar!(u32, U32),
            TypeKind::I64 => scalar!(i64, I64),
            TypeKind::U64 => scalar!(u64, U64),
            TypeKind::F32 => scalar!(f32, F32),
            TypeKind::F64 => scalar!(f64, F64),
            TypeKind::Guid => scalar!(GUID, Guid),
            TypeKind::HResult => scalar!(HRESULT, HResult),
            TypeKind::Enum(_) => WinRTValue::Enum {
                value: unsafe { source.cast::<i32>().read_unaligned() },
                type_handle: self.typ.clone(),
            },
            TypeKind::HString => {
                let raw = unsafe { source.cast::<*mut c_void>().read_unaligned() };
                let borrowed = unsafe { &*ptr::from_ref(&raw).cast::<HSTRING>() };
                WinRTValue::HString(borrowed.clone())
            }
            TypeKind::Struct(_) => {
                WinRTValue::Struct(unsafe { self.prepare_borrowed_struct(source)? })
            }
            kind if kind.is_com_pointer() => {
                let raw = unsafe { source.cast::<*mut c_void>().read_unaligned() };
                if raw.is_null() {
                    WinRTValue::Null
                } else {
                    let borrowed = unsafe { IUnknown::from_raw_borrowed(&raw) }
                        .ok_or_else(|| error(E_POINTER, "Null WinRT callback interface"))?;
                    let owned = borrowed.clone();
                    if self.typ.is_async() {
                        self.typ
                            .from_out(owned.into_raw())
                            .map_err(|cause| invalid_argument(cause.message()))?
                    } else {
                        WinRTValue::Object(owned)
                    }
                }
            }
            _ => return Err(invalid_argument("Unsupported WinRT callback value plan")),
        })
    }

    pub fn prepare(&self, value: &WinRTValue) -> windows_core::Result<PreparedValue> {
        let coerced = if self.iid.is_none() {
            crate::native_call::coerce_scalar_input(&self.typ, value).map_err(|cause| {
                error(
                    cause.code(),
                    format!("WinRT callback output: {}", cause.message()),
                )
            })?
        } else {
            None
        };
        let value = coerced.as_ref().unwrap_or(value);
        macro_rules! primitive {
            ($value:expr, $variant:ident) => {
                Ok(PreparedValue::$variant(*$value))
            };
        }
        match (self.typ.kind(), value) {
            (TypeKind::Bool, WinRTValue::Bool(value)) => primitive!(value, Bool),
            (TypeKind::I8, WinRTValue::I8(value)) => primitive!(value, I8),
            (TypeKind::U8, WinRTValue::U8(value)) => primitive!(value, U8),
            (TypeKind::I16, WinRTValue::I16(value)) => primitive!(value, I16),
            (TypeKind::U16 | TypeKind::Char16, WinRTValue::U16(value)) => primitive!(value, U16),
            (TypeKind::I32 | TypeKind::Enum(_) | TypeKind::HResult, WinRTValue::I32(value)) => {
                primitive!(value, I32)
            }
            (TypeKind::Enum(_), WinRTValue::Enum { value, type_handle })
                if *type_handle == self.typ =>
            {
                primitive!(value, I32)
            }
            (TypeKind::U32, WinRTValue::U32(value)) => primitive!(value, U32),
            (TypeKind::I64, WinRTValue::I64(value)) => primitive!(value, I64),
            (TypeKind::U64, WinRTValue::U64(value)) => primitive!(value, U64),
            (TypeKind::F32, WinRTValue::F32(value)) => primitive!(value, F32),
            (TypeKind::F64, WinRTValue::F64(value)) => primitive!(value, F64),
            (TypeKind::Guid, WinRTValue::Guid(value)) => primitive!(value, Guid),
            (TypeKind::HResult, WinRTValue::HResult(value)) => Ok(PreparedValue::I32(value.0)),
            (TypeKind::HString, WinRTValue::HString(value)) => {
                Ok(PreparedValue::HString(value.clone()))
            }
            (TypeKind::Struct(_), WinRTValue::Struct(value))
                if *value.type_handle() == self.typ =>
            {
                Ok(PreparedValue::Struct(unsafe {
                    self.prepare_borrowed_struct(value.as_ptr().cast())?
                }))
            }
            (kind, WinRTValue::Null) if kind.is_com_pointer() => Ok(PreparedValue::Object(None)),
            (kind, value) if kind.is_com_pointer() => {
                let object = value.as_object().ok_or_else(|| self.mismatch(value))?;
                let mut queried = ptr::null_mut();
                let iid = self.iid.expect("validated WinRT reference IID");
                let hr = unsafe { object.query(&iid, &mut queried) };
                // Own any returned reference immediately, including on failure.
                let queried = unsafe { IUnknown::from_raw_borrowed(&queried) }
                    .map(|_| unsafe { IUnknown::from_raw(queried) });
                hr.ok()?;
                queried
                    .map(|object| PreparedValue::Object(Some(object)))
                    .ok_or_else(|| error(E_POINTER, "QueryInterface returned a null WinRT output"))
            }
            _ => Err(self.mismatch(value)),
        }
    }

    fn mismatch(&self, value: &WinRTValue) -> windows_core::Error {
        invalid_argument(format!(
            "WinRT callback output expected {}, received {:?}",
            self.typ.signature_string(),
            value.get_type_kind()
        ))
    }

    unsafe fn prepare_borrowed_struct(
        &self,
        source: *const c_void,
    ) -> windows_core::Result<ValueTypeData> {
        let mut prepared = ValueTypeData::try_new(&self.typ)?;
        // Populate only retained fields into zeroed storage. A later failure
        // can then drop it without releasing any still-borrowed fields.
        for (offset, field) in &self.fields {
            let source = unsafe { source.byte_add(*offset) };
            let owned = if matches!(field.typ.kind(), TypeKind::Struct(_)) {
                PreparedValue::Struct(unsafe { field.prepare_borrowed_struct(source)? })
            } else {
                let value = unsafe { field.read_borrowed(source)? };
                field.prepare(&value)?
            };
            unsafe { owned.commit(prepared.as_mut_ptr().add(*offset).cast()) };
        }
        Ok(prepared)
    }

    unsafe fn read_array(
        &self,
        source: *const c_void,
        length: u32,
    ) -> windows_core::Result<WinRTValue> {
        self.array_bytes(length as usize)?;
        if source.is_null() && length != 0 {
            return Err(error(
                E_POINTER,
                "Nonempty WinRT input array has a null data pointer",
            ));
        }
        let mut values = Vec::new();
        values
            .try_reserve_exact(length as usize)
            .map_err(|_| error(E_OUTOFMEMORY, "Cannot allocate WinRT callback array inputs"))?;
        for index in 0..length as usize {
            values.push(unsafe { self.read_borrowed(source.byte_add(index * self.size))? });
        }
        Ok(WinRTValue::Array(ArrayData::from_owned_values(
            self.typ.clone(),
            values,
        )))
    }

    fn prepare_array(&self, value: &WinRTValue) -> windows_core::Result<PreparedArray> {
        let WinRTValue::Array(array) = value else {
            return Err(invalid_argument(
                "WinRT callback array output must be an Array value",
            ));
        };
        let compatible = self.iid.is_some()
            || crate::native_call::array_element_types_match(&self.typ, &array.element_type);
        if !compatible {
            return Err(invalid_argument(
                "WinRT callback array element type does not match its plan",
            ));
        }
        let length = u32::try_from(array.len())
            .map_err(|_| invalid_argument("WinRT callback array length exceeds UINT32_MAX"))?;
        let bytes = self.array_bytes(array.len())?;
        let data = if bytes == 0 {
            ptr::null_mut()
        } else {
            let data = unsafe { CoTaskMemAlloc(bytes) };
            if data.is_null() {
                return Err(error(
                    E_OUTOFMEMORY,
                    "Cannot allocate WinRT callback array output",
                ));
            }
            unsafe { ptr::write_bytes(data, 0, bytes) };
            data
        };
        let prepared = PreparedArray {
            data,
            length,
            element_type: self.typ.clone(),
        };
        for index in 0..array.len() {
            let value = array
                .try_get(index)
                .map_err(|cause| invalid_argument(cause.message()))?;
            let owned = self.prepare(&value)?;
            unsafe { owned.commit(data.byte_add(index * self.size)) };
        }
        Ok(prepared)
    }
}

pub(super) enum PreparedValue {
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
    Guid(GUID),
    HString(HSTRING),
    Object(Option<IUnknown>),
    Struct(ValueTypeData),
}

impl PreparedValue {
    unsafe fn commit(self, target: *mut c_void) {
        unsafe {
            match self {
                Self::Bool(value) => target.cast::<u8>().write(u8::from(value)),
                Self::I8(value) => target.cast::<i8>().write(value),
                Self::U8(value) => target.cast::<u8>().write(value),
                Self::I16(value) => target.cast::<i16>().write_unaligned(value),
                Self::U16(value) => target.cast::<u16>().write_unaligned(value),
                Self::I32(value) => target.cast::<i32>().write_unaligned(value),
                Self::U32(value) => target.cast::<u32>().write_unaligned(value),
                Self::I64(value) => target.cast::<i64>().write_unaligned(value),
                Self::U64(value) => target.cast::<u64>().write_unaligned(value),
                Self::F32(value) => target.cast::<f32>().write_unaligned(value),
                Self::F64(value) => target.cast::<f64>().write_unaligned(value),
                Self::Guid(value) => target.cast::<GUID>().write_unaligned(value),
                Self::HString(value) => target.cast::<HSTRING>().write_unaligned(value),
                Self::Object(value) => target
                    .cast::<*mut c_void>()
                    .write_unaligned(value.map_or(ptr::null_mut(), Interface::into_raw)),
                Self::Struct(value) => value.move_to_abi(target),
            }
        }
    }
}

pub(super) struct PreparedArray {
    data: *mut c_void,
    length: u32,
    element_type: TypeHandle,
}

impl Drop for PreparedArray {
    fn drop(&mut self) {
        if !self.data.is_null() {
            drop(ArrayData::from_cotaskmem(
                self.element_type.clone(),
                self.data,
                self.length as usize,
            ));
        }
    }
}

pub(super) enum OutputTarget {
    Value(*mut c_void),
    ReceiveArray {
        length: *mut u32,
        data: *mut *mut c_void,
    },
    FillArray {
        capacity: u32,
        data: *mut c_void,
    },
}

pub(super) enum PreparedOutput {
    Value(*mut c_void, PreparedValue),
    ReceiveArray(*mut u32, *mut *mut c_void, PreparedArray),
    FillArray(*mut c_void, PreparedArray),
}

impl PreparedOutput {
    pub unsafe fn commit(self) {
        unsafe {
            match self {
                Self::Value(target, value) => value.commit(target),
                Self::ReceiveArray(length, data, mut array) => {
                    length.write_unaligned(array.length);
                    data.write_unaligned(array.data);
                    array.data = ptr::null_mut();
                }
                Self::FillArray(target, mut array) => {
                    let bytes = array.length as usize * array.element_type.size_of();
                    if bytes != 0 {
                        ptr::copy_nonoverlapping(array.data.cast::<u8>(), target.cast(), bytes);
                        CoTaskMemFree(Some(array.data));
                        array.data = ptr::null_mut();
                    }
                }
            }
        }
    }
}

impl ParameterPlan {
    unsafe fn pointer(&self, args: *const *const c_void, offset: usize) -> *mut c_void {
        unsafe {
            (*args.add(self.abi_index + offset))
                .cast::<*mut c_void>()
                .read_unaligned()
        }
    }

    unsafe fn length(&self, args: *const *const c_void) -> u32 {
        unsafe { (*args.add(self.abi_index)).cast::<u32>().read_unaligned() }
    }
}

impl MethodPlan {
    pub unsafe fn initialize_outputs(
        &self,
        args: *const *const c_void,
    ) -> windows_core::Result<Vec<OutputTarget>> {
        let mut outputs = Vec::with_capacity(self.output_count);
        let mut failure = None;
        for parameter in &self.parameters {
            if parameter.direction == WinRtParameterDirection::In {
                continue;
            }
            if !parameter.array {
                let target = unsafe { parameter.pointer(args, 0) };
                if target.is_null() {
                    failure.get_or_insert_with(|| error(E_POINTER, "Null WinRT output pointer"));
                } else {
                    unsafe { ptr::write_bytes(target, 0, parameter.value.size) };
                }
                outputs.push(OutputTarget::Value(target));
            } else if parameter.direction == WinRtParameterDirection::Out {
                let length = unsafe { parameter.pointer(args, 0).cast::<u32>() };
                let data = unsafe { parameter.pointer(args, 1).cast::<*mut c_void>() };
                if !length.is_null() {
                    unsafe { length.write_unaligned(0) };
                }
                if !data.is_null() {
                    unsafe { data.write_unaligned(ptr::null_mut()) };
                }
                if length.is_null() || data.is_null() {
                    failure.get_or_insert_with(|| {
                        error(E_POINTER, "Null WinRT ReceiveArray output pointer")
                    });
                }
                outputs.push(OutputTarget::ReceiveArray { length, data });
            } else {
                let capacity = unsafe { parameter.length(args) };
                let data = unsafe { parameter.pointer(args, 1) };
                match parameter.value.array_bytes(capacity as usize) {
                    Ok(bytes) if bytes != 0 && data.is_null() => {
                        failure.get_or_insert_with(|| {
                            error(E_POINTER, "Null nonempty WinRT FillArray buffer")
                        });
                    }
                    Ok(bytes) if bytes != 0 => unsafe { ptr::write_bytes(data, 0, bytes) },
                    Ok(_) => {}
                    Err(error) => {
                        failure.get_or_insert(error);
                    }
                }
                outputs.push(OutputTarget::FillArray { capacity, data });
            }
        }
        failure.map_or(Ok(outputs), Err)
    }

    pub unsafe fn inputs(
        &self,
        args: *const *const c_void,
    ) -> windows_core::Result<Vec<WinRTValue>> {
        let mut inputs = Vec::with_capacity(self.input_count);
        for parameter in &self.parameters {
            if parameter.direction == WinRtParameterDirection::Out {
                continue;
            }
            inputs.push(
                if parameter.direction == WinRtParameterDirection::FillArray {
                    WinRTValue::U32(unsafe { parameter.length(args) })
                } else if parameter.array {
                    unsafe {
                        parameter
                            .value
                            .read_array(parameter.pointer(args, 1), parameter.length(args))?
                    }
                } else {
                    unsafe {
                        parameter
                            .value
                            .read_borrowed(*args.add(parameter.abi_index))?
                    }
                },
            );
        }
        Ok(inputs)
    }

    pub fn prepare_outputs(
        &self,
        targets: Vec<OutputTarget>,
        values: &[WinRTValue],
    ) -> windows_core::Result<Vec<PreparedOutput>> {
        if values.len() != self.output_count {
            return Err(invalid_argument(format!(
                "{}: callback returned {} outputs; the WinRT contract requires {}",
                self.name,
                values.len(),
                self.output_count
            )));
        }
        self.parameters
            .iter()
            .filter(|parameter| parameter.direction != WinRtParameterDirection::In)
            .zip(targets)
            .zip(values)
            .map(|((parameter, target), value)| match target {
                OutputTarget::Value(target) => Ok(PreparedOutput::Value(
                    target,
                    parameter.value.prepare(value)?,
                )),
                OutputTarget::ReceiveArray { length, data } => Ok(PreparedOutput::ReceiveArray(
                    length,
                    data,
                    parameter.value.prepare_array(value)?,
                )),
                OutputTarget::FillArray { capacity, data } => {
                    let array = parameter.value.prepare_array(value)?;
                    if array.length != capacity {
                        return Err(invalid_argument(format!(
                            "{}: FillArray output has {} elements; expected capacity {capacity}",
                            self.name, array.length
                        )));
                    }
                    Ok(PreparedOutput::FillArray(data, array))
                }
            })
            .collect()
    }
}
