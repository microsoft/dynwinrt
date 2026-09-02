// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::abi::AbiType;
use crate::metadata_table::TypeKind;

#[derive(Debug)]
pub enum Error {
    ExpectObjectTypeError(TypeKind),
    ExpectStructTypeError(TypeKind),
    IndexOutOfBounds {
        index: usize,
        len: usize,
    },
    InvalidType(TypeKind, TypeKind),
    InvalidNestedOutType(TypeKind),
    InvalidTypeAbiToWinRT(TypeKind, AbiType),
    WindowsError(windows_core::Error),
    TypeNotFound(String),
    NotAnInterface(String),
    MethodNotFound(String, String),
    ExpectedAsync(TypeKind),
    UnsupportedCollectionElement(TypeKind),
    InvalidCollectionValue(&'static str),
    ExpectedIBuffer(TypeKind),
    InvalidIBufferBounds {
        length: u32,
        capacity: u32,
    },
    NullIBufferPointer {
        length: usize,
    },
    IBufferInputTooLarge(usize),
    /// An async operation was canceled (status == AsyncStatus::Canceled).
    Canceled,
}

impl Error {
    pub fn expect_object_type(actual: TypeKind) -> Self {
        Error::ExpectObjectTypeError(actual)
    }

    pub fn message(&self) -> String {
        match self {
            Error::ExpectObjectTypeError(actual) => {
                format!("Expected object type, found {:?}", actual)
            }
            Error::ExpectStructTypeError(actual) => {
                format!("Expected struct type, found {:?}", actual)
            }
            Error::IndexOutOfBounds { index, len } => {
                format!("Index {index} out of bounds (len {len})")
            }
            Error::InvalidType(expected, actual) => {
                format!("Invalid type: expected {:?}, found {:?}", expected, actual)
            }
            Error::InvalidNestedOutType(actual) => {
                format!("Invalid nested out type: found {:?}", actual)
            }
            Error::InvalidTypeAbiToWinRT(expected, actual) => {
                format!(
                    "Invalid type ABI to WinRT: expected {:?}, found {:?}",
                    expected, actual
                )
            }
            Error::WindowsError(err) => format!("0x{:08X}: {}", err.code().0 as u32, err),
            Error::TypeNotFound(name) => format!("Type not found: {}", name),
            Error::NotAnInterface(name) => format!("Not an interface: {}", name),
            Error::MethodNotFound(iface, method) => {
                format!("Method '{}' not found on interface '{}'", method, iface)
            }
            Error::ExpectedAsync(actual) => {
                format!("Expected an async value, found {:?}", actual)
            }
            Error::UnsupportedCollectionElement(actual) => {
                format!("Unsupported dynamic collection element type: {:?}", actual)
            }
            Error::InvalidCollectionValue(expected) => {
                format!("Invalid dynamic collection value: expected {expected}")
            }
            Error::ExpectedIBuffer(actual) => {
                format!("Expected a Windows.Storage.Streams.IBuffer object, found {actual:?}")
            }
            Error::InvalidIBufferBounds { length, capacity } => {
                format!("Invalid IBuffer bounds: Length {length} exceeds Capacity {capacity}")
            }
            Error::NullIBufferPointer { length } => {
                format!("IBufferByteAccess returned a null pointer for {length} bytes")
            }
            Error::IBufferInputTooLarge(length) => {
                format!(
                    "Cannot create an IBuffer from {length} bytes; the maximum is {}",
                    u32::MAX
                )
            }
            Error::Canceled => "Async operation was canceled".to_string(),
        }
    }
}

impl From<windows::core::Error> for Error {
    fn from(value: windows::core::Error) -> Self {
        Self::WindowsError(value)
    }
}

pub type Result<T> = core::result::Result<T, Error>;
