// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiType {
    Bool,
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
    Guid,
    Ptr,
}

impl AbiType {
    pub fn default_value(&self) -> AbiValue {
        match self {
            AbiType::Bool => AbiValue::Bool(0),
            AbiType::I8 => AbiValue::I8(0),
            AbiType::U8 => AbiValue::U8(0),
            AbiType::I16 => AbiValue::I16(0),
            AbiType::U16 => AbiValue::U16(0),
            AbiType::I32 => AbiValue::I32(0),
            AbiType::U32 => AbiValue::U32(0),
            AbiType::I64 => AbiValue::I64(0),
            AbiType::U64 => AbiValue::U64(0),
            AbiType::F32 => AbiValue::F32(0.0),
            AbiType::F64 => AbiValue::F64(0.0),
            AbiType::Guid => AbiValue::Guid(windows_core::GUID::zeroed()),
            AbiType::Ptr => AbiValue::Pointer(std::ptr::null_mut()),
        }
    }

    pub fn libffi_type(&self) -> libffi::middle::Type {
        match self {
            AbiType::Bool | AbiType::U8 => libffi::middle::Type::u8(),
            AbiType::I8 => libffi::middle::Type::i8(),
            AbiType::I16 => libffi::middle::Type::i16(),
            AbiType::U16 => libffi::middle::Type::u16(),
            AbiType::I32 => libffi::middle::Type::i32(),
            AbiType::U32 => libffi::middle::Type::u32(),
            AbiType::I64 => libffi::middle::Type::i64(),
            AbiType::U64 => libffi::middle::Type::u64(),
            AbiType::F32 => libffi::middle::Type::f32(),
            AbiType::F64 => libffi::middle::Type::f64(),
            AbiType::Guid => guid_libffi_type(),
            AbiType::Ptr => libffi::middle::Type::pointer(),
        }
    }
}

pub(crate) fn guid_libffi_type() -> libffi::middle::Type {
    use libffi::middle::Type;

    Type::structure(vec![
        Type::u32(),
        Type::u16(),
        Type::u16(),
        Type::u8(),
        Type::u8(),
        Type::u8(),
        Type::u8(),
        Type::u8(),
        Type::u8(),
        Type::u8(),
        Type::u8(),
    ])
}

#[derive(Debug)]
pub enum AbiValue {
    Bool(u8),
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
    Guid(windows_core::GUID),
    Pointer(*mut std::ffi::c_void),
}

impl AbiValue {
    pub fn as_out_ptr(&self) -> *const std::ffi::c_void {
        match self {
            AbiValue::Bool(v) => std::ptr::from_ref(v).cast(),
            AbiValue::I8(v) => std::ptr::from_ref(v).cast(),
            AbiValue::U8(v) => std::ptr::from_ref(v).cast(),
            AbiValue::I16(v) => std::ptr::from_ref(v).cast(),
            AbiValue::U16(v) => std::ptr::from_ref(v).cast(),
            AbiValue::I32(v) => std::ptr::from_ref(v).cast(),
            AbiValue::U32(v) => std::ptr::from_ref(v).cast(),
            AbiValue::I64(v) => std::ptr::from_ref(v).cast(),
            AbiValue::U64(v) => std::ptr::from_ref(v).cast(),
            AbiValue::F32(v) => std::ptr::from_ref(v).cast(),
            AbiValue::F64(v) => std::ptr::from_ref(v).cast(),
            AbiValue::Guid(v) => std::ptr::from_ref(v).cast(),
            AbiValue::Pointer(p) => std::ptr::from_ref(p).cast(),
        }
    }

    pub fn abi_type(&self) -> AbiType {
        match self {
            AbiValue::Bool(_) => AbiType::Bool,
            AbiValue::I8(_) => AbiType::I8,
            AbiValue::U8(_) => AbiType::U8,
            AbiValue::I16(_) => AbiType::I16,
            AbiValue::U16(_) => AbiType::U16,
            AbiValue::I32(_) => AbiType::I32,
            AbiValue::U32(_) => AbiType::U32,
            AbiValue::I64(_) => AbiType::I64,
            AbiValue::U64(_) => AbiType::U64,
            AbiValue::F32(_) => AbiType::F32,
            AbiValue::F64(_) => AbiType::F64,
            AbiValue::Guid(_) => AbiType::Guid,
            AbiValue::Pointer(_) => AbiType::Ptr,
        }
    }
}
