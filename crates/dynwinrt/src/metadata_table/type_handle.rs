// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::alloc::Layout;
use std::sync::Arc;

use windows_core::{GUID, IUnknown, Interface};

use crate::abi::{AbiType, AbiValue};
use crate::signature::MethodSignature;
use crate::value::WinRTValue;

use super::MetadataTable;
use super::method_handle::MethodHandle;
use super::type_kind::*;
use super::value_data::ValueTypeData;

/// A handle to a type in the MetadataTable. Carries an `Arc<MetadataTable>` so it
/// can query layout and create values without needing a separate table reference.
#[derive(Clone)]
pub struct TypeHandle {
    pub(crate) table: Arc<MetadataTable>,
    pub(crate) kind: TypeKind,
}

impl std::fmt::Debug for TypeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypeHandle")
            .field("kind", &self.kind)
            .finish()
    }
}

impl PartialEq for TypeHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.table, &other.table) && self.kind == other.kind
    }
}

impl Eq for TypeHandle {}

impl TypeHandle {
    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    pub fn kind(&self) -> TypeKind {
        self.kind
    }

    pub fn table(&self) -> &Arc<MetadataTable> {
        &self.table
    }

    // -----------------------------------------------------------------------
    // Builder: add method to interface
    // -----------------------------------------------------------------------

    /// Add a method to this interface. Only valid for Interface types.
    /// Returns self for chaining.
    pub fn add_method(self, name: &str, sig: MethodSignature) -> Self {
        match self.kind {
            TypeKind::Interface(iid) => {
                self.table.add_method_to_interface(&iid, name, sig);
                self
            }
            _ => panic!(
                "add_method: TypeHandle is not an Interface, got {:?}",
                self.kind
            ),
        }
    }

    // -----------------------------------------------------------------------
    // Method access
    // -----------------------------------------------------------------------

    /// Get a MethodHandle by vtable index (6 = first user method).
    pub fn method(&self, vtable_index: usize) -> Option<MethodHandle> {
        match self.kind {
            TypeKind::Interface(iid) => self.table.method_by_vtable_index(&iid, vtable_index),
            _ => None,
        }
    }

    /// Get a MethodHandle by method name.
    pub fn method_by_name(&self, name: &str) -> Option<MethodHandle> {
        match self.kind {
            TypeKind::Interface(iid) => self.table.method_by_name(&iid, name),
            _ => None,
        }
    }

    // -----------------------------------------------------------------------
    // Struct layout
    // -----------------------------------------------------------------------

    pub fn size_of(&self) -> usize {
        self.table.size_of_kind(self.kind)
    }

    pub fn align_of(&self) -> usize {
        self.table.align_of_kind(self.kind)
    }

    pub fn layout(&self) -> Layout {
        self.table.layout_of_kind(self.kind)
    }

    pub fn field_count(&self) -> usize {
        self.table.field_count_kind(self.kind)
    }

    pub fn field_offset(&self, index: usize) -> usize {
        self.table.field_offset_kind(self.kind, index)
    }

    pub fn field_type(&self, index: usize) -> TypeHandle {
        TypeHandle {
            table: Arc::clone(&self.table),
            kind: self.table.field_kind(self.kind, index),
        }
    }

    /// Create a zero-initialized ValueTypeData. Only valid for Struct types.
    pub fn default_value(&self) -> ValueTypeData {
        ValueTypeData::new(self)
    }

    // -----------------------------------------------------------------------
    // Type methods
    // -----------------------------------------------------------------------

    pub fn abi_type(&self) -> AbiType {
        match self.kind {
            TypeKind::Bool => AbiType::Bool,
            TypeKind::I8 => AbiType::I8,
            TypeKind::U8 => AbiType::U8,
            TypeKind::I16 => AbiType::I16,
            TypeKind::U16 | TypeKind::Char16 => AbiType::U16,
            TypeKind::I32 | TypeKind::HResult | TypeKind::Enum(_) => AbiType::I32,
            TypeKind::U32 => AbiType::U32,
            TypeKind::I64 => AbiType::I64,
            TypeKind::U64 => AbiType::U64,
            TypeKind::F32 => AbiType::F32,
            TypeKind::F64 => AbiType::F64,

            TypeKind::HString
            | TypeKind::Guid
            | TypeKind::Object
            | TypeKind::Interface(_)
            | TypeKind::Delegate(_)
            | TypeKind::RuntimeClass(_)
            | TypeKind::Parameterized(_)
            | TypeKind::IAsyncAction
            | TypeKind::IAsyncActionWithProgress(_)
            | TypeKind::IAsyncOperation(_)
            | TypeKind::IAsyncOperationWithProgress(_)
            | TypeKind::OutValue(_)
            | TypeKind::ArrayOfIUnknown => AbiType::Ptr,

            TypeKind::Generic { piid, .. } => {
                panic!("Cannot get ABI type for uninstantiated Generic({:?})", piid)
            }

            TypeKind::Struct(_) => {
                panic!("Struct types do not have a simple AbiType; use libffi_type() instead")
            }

            TypeKind::Array(_) => {
                panic!(
                    "Array types expand to multiple ABI parameters; cannot map to single AbiType"
                )
            }
        }
    }

    pub fn libffi_type(&self) -> libffi::middle::Type {
        match self.kind {
            TypeKind::Struct(_) => self.table.libffi_type_kind(self.kind),
            TypeKind::Array(_) => {
                panic!("Array types expand to multiple libffi types")
            }
            _ => self.abi_type().libffi_type(),
        }
    }

    pub fn is_async(&self) -> bool {
        match self.kind {
            TypeKind::IAsyncAction
            | TypeKind::IAsyncActionWithProgress(_)
            | TypeKind::IAsyncOperation(_)
            | TypeKind::IAsyncOperationWithProgress(_) => true,
            TypeKind::Parameterized(idx) => {
                let (generic_def, _) = self.table.get_parameterized(idx);
                is_async_piid(generic_def)
            }
            _ => false,
        }
    }

    /// Reverse-lookup an enum member name from its i32 value.
    /// Returns None if not an Enum type or no member matches.
    pub fn enum_member_name(&self, value: i32) -> Option<String> {
        match self.kind {
            TypeKind::Enum(idx) => self.table.get_enum_member_name(idx, value),
            _ => None,
        }
    }

    pub fn signature_string(&self) -> String {
        self.table.signature_string_kind(self.kind)
    }

    pub fn iid(&self) -> Option<GUID> {
        self.table.iid_kind(self.kind)
    }

    pub fn completed_handler_iid(&self) -> Option<GUID> {
        self.table.completed_handler_iid_kind(self.kind)
    }

    pub fn progress_handler_iid(&self) -> Option<GUID> {
        self.table.progress_handler_iid_kind(self.kind)
    }

    pub fn is_array(&self) -> bool {
        matches!(self.kind, TypeKind::Array(_))
    }

    pub fn array_element_type(&self) -> TypeHandle {
        match self.kind {
            TypeKind::Array(idx) => {
                let inner = self.table.get_inner_type(idx);
                TypeHandle {
                    table: self.table.clone(),
                    kind: inner,
                }
            }
            _ => panic!(
                "array_element_type called on non-array type {:?}",
                self.kind
            ),
        }
    }

    pub fn element_size(&self) -> usize {
        self.table.size_of_kind(self.kind)
    }

    pub fn default_winrt_value(&self) -> WinRTValue {
        match self.kind {
            TypeKind::Bool => WinRTValue::Bool(false),
            TypeKind::I8 => WinRTValue::I8(0),
            TypeKind::U8 => WinRTValue::U8(0),
            TypeKind::I16 => WinRTValue::I16(0),
            TypeKind::U16 | TypeKind::Char16 => WinRTValue::U16(0),
            TypeKind::I32 => WinRTValue::I32(0),
            TypeKind::Enum(_) => WinRTValue::Enum {
                value: 0,
                type_handle: self.clone(),
            },
            TypeKind::U32 => WinRTValue::U32(0),
            TypeKind::I64 => WinRTValue::I64(0),
            TypeKind::U64 => WinRTValue::U64(0),
            TypeKind::F32 => WinRTValue::F32(0.0),
            TypeKind::F64 => WinRTValue::F64(0.0),

            // COM pointer types: use RawPtr(null) as out-buffer.
            // We must NOT use IUnknown::from_raw(null) because it is UB — the null
            // vtable pointer triggers undefined behavior under release optimizations.
            // After the COM call writes a valid pointer, from_out() wraps it properly.
            TypeKind::Object
            | TypeKind::Interface(_)
            | TypeKind::Delegate(_)
            | TypeKind::RuntimeClass(_)
            | TypeKind::Parameterized(_)
            | TypeKind::IAsyncAction
            | TypeKind::IAsyncActionWithProgress(_)
            | TypeKind::IAsyncOperation(_)
            | TypeKind::IAsyncOperationWithProgress(_) => WinRTValue::RawPtr(std::ptr::null_mut()),

            TypeKind::HString => WinRTValue::HString(windows_core::HSTRING::new()),
            TypeKind::Guid => WinRTValue::Guid(windows_core::GUID::zeroed()),
            TypeKind::HResult => WinRTValue::HResult(windows_core::HRESULT(0)),

            TypeKind::OutValue(_) => WinRTValue::OutValue(std::ptr::null_mut(), self.clone()),

            TypeKind::Generic { piid, .. } => {
                panic!("Cannot create default value for Generic({:?})", piid)
            }

            TypeKind::ArrayOfIUnknown => WinRTValue::ArrayOfIUnknown(
                crate::value::ArrayOfIUnknownData(windows::core::Array::new()),
            ),

            TypeKind::Struct(_) => WinRTValue::Struct(self.default_value()),

            TypeKind::Array(_) => WinRTValue::Array(crate::array::ArrayData::empty(self.clone())),
        }
    }

    pub fn from_out(&self, ptr: *mut std::ffi::c_void) -> crate::result::Result<WinRTValue> {
        unsafe {
            match self.kind {
                TypeKind::Bool => Ok(WinRTValue::Bool(*(ptr as *mut u8) != 0)),
                TypeKind::I8 => Ok(WinRTValue::I8(*(ptr as *mut i8))),
                TypeKind::U8 => Ok(WinRTValue::U8(*(ptr as *mut u8))),
                TypeKind::I16 => Ok(WinRTValue::I16(*(ptr as *mut i16))),
                TypeKind::U16 | TypeKind::Char16 => Ok(WinRTValue::U16(*(ptr as *mut u16))),
                TypeKind::I32 => Ok(WinRTValue::I32(*(ptr as *mut i32))),
                TypeKind::Enum(_) => Ok(WinRTValue::Enum {
                    value: *(ptr as *mut i32),
                    type_handle: self.clone(),
                }),
                TypeKind::U32 => Ok(WinRTValue::U32(*(ptr as *mut u32))),
                TypeKind::I64 => Ok(WinRTValue::I64(*(ptr as *mut i64))),
                TypeKind::U64 => Ok(WinRTValue::U64(*(ptr as *mut u64))),
                TypeKind::F32 => Ok(WinRTValue::F32(*(ptr as *mut f32))),
                TypeKind::F64 => Ok(WinRTValue::F64(*(ptr as *mut f64))),

                TypeKind::Object
                | TypeKind::Interface(_)
                | TypeKind::Delegate(_)
                | TypeKind::RuntimeClass(_) => Ok(WinRTValue::Object(IUnknown::from_raw(ptr))),

                TypeKind::HString => Ok(WinRTValue::HString(std::mem::transmute(ptr))),

                TypeKind::HResult => Ok(WinRTValue::HResult(windows_core::HRESULT(
                    *(ptr as *mut i32),
                ))),

                TypeKind::Parameterized(idx) => {
                    let (generic_def, args) = self.table.get_parameterized(idx);
                    if is_async_piid(generic_def) {
                        let raw = IUnknown::from_raw(ptr);
                        let iid = self.iid().unwrap();
                        make_async_value_from_kind(raw, generic_def, iid, &args, &self.table)
                    } else {
                        Ok(WinRTValue::Object(IUnknown::from_raw(ptr)))
                    }
                }

                TypeKind::IAsyncAction
                | TypeKind::IAsyncActionWithProgress(_)
                | TypeKind::IAsyncOperation(_)
                | TypeKind::IAsyncOperationWithProgress(_) => {
                    let raw = IUnknown::from_raw(ptr);
                    let info: windows_future::IAsyncInfo = raw
                        .cast()
                        .map_err(|e| crate::result::Error::WindowsError(e))?;
                    Ok(WinRTValue::Async(crate::value::AsyncInfo {
                        info,
                        async_type: self.clone(),
                    }))
                }

                _ => Err(crate::result::Error::InvalidTypeAbiToWinRT(
                    self.kind,
                    AbiType::Ptr,
                )),
            }
        }
    }

    pub fn from_out_value(&self, out: &AbiValue) -> crate::result::Result<WinRTValue> {
        use crate::result::Error;
        match (self.kind, out) {
            (TypeKind::Bool, AbiValue::Bool(v)) => Ok(WinRTValue::Bool(*v != 0)),
            (TypeKind::I8, AbiValue::I8(v)) => Ok(WinRTValue::I8(*v)),
            (TypeKind::U8, AbiValue::U8(v)) => Ok(WinRTValue::U8(*v)),
            (TypeKind::I16, AbiValue::I16(v)) => Ok(WinRTValue::I16(*v)),
            (TypeKind::U16 | TypeKind::Char16, AbiValue::U16(v)) => Ok(WinRTValue::U16(*v)),
            (TypeKind::I32, AbiValue::I32(v)) => Ok(WinRTValue::I32(*v)),
            (TypeKind::Enum(_), AbiValue::I32(v)) => Ok(WinRTValue::Enum {
                value: *v,
                type_handle: self.clone(),
            }),
            (TypeKind::U32, AbiValue::U32(v)) => Ok(WinRTValue::U32(*v)),
            (TypeKind::I64, AbiValue::I64(v)) => Ok(WinRTValue::I64(*v)),
            (TypeKind::U64, AbiValue::U64(v)) => Ok(WinRTValue::U64(*v)),
            (TypeKind::F32, AbiValue::F32(v)) => Ok(WinRTValue::F32(*v)),
            (TypeKind::F64, AbiValue::F64(v)) => Ok(WinRTValue::F64(*v)),

            (
                TypeKind::Object
                | TypeKind::Interface(_)
                | TypeKind::Delegate(_)
                | TypeKind::RuntimeClass(_),
                AbiValue::Pointer(p),
            ) => Ok(WinRTValue::Object(unsafe { IUnknown::from_raw(*p) })),

            (TypeKind::HString, AbiValue::Pointer(p)) => {
                Ok(WinRTValue::HString(unsafe { core::mem::transmute(*p) }))
            }

            (TypeKind::HResult, AbiValue::I32(hr)) => {
                Ok(WinRTValue::HResult(windows_core::HRESULT(*hr)))
            }

            (TypeKind::Parameterized(idx), AbiValue::Pointer(p)) => {
                let (generic_def, args) = self.table.get_parameterized(idx);
                if is_async_piid(generic_def) {
                    let raw = unsafe { IUnknown::from_raw(*p) };
                    let iid = self.iid().unwrap();
                    make_async_value_from_kind(raw, generic_def, iid, &args, &self.table)
                } else {
                    Ok(WinRTValue::Object(unsafe { IUnknown::from_raw(*p) }))
                }
            }

            (
                TypeKind::IAsyncAction
                | TypeKind::IAsyncActionWithProgress(_)
                | TypeKind::IAsyncOperation(_)
                | TypeKind::IAsyncOperationWithProgress(_),
                AbiValue::Pointer(_),
            ) => match out {
                AbiValue::Pointer(p) => self.from_out(*p),
                _ => unreachable!(),
            },

            (TypeKind::OutValue(_), _) => Err(Error::InvalidNestedOutType(self.kind)),
            _ => Err(Error::InvalidTypeAbiToWinRT(self.kind, out.abi_type())),
        }
    }
}

// ===========================================================================
// Free functions (async helpers)
// ===========================================================================

fn is_async_piid(generic_def: TypeKind) -> bool {
    let piid = match generic_def {
        TypeKind::Generic { piid, .. } => piid,
        TypeKind::Interface(iid) => iid,
        _ => return false,
    };
    piid == IASYNC_ACTION
        || piid == IASYNC_OPERATION
        || piid == IASYNC_ACTION_WITH_PROGRESS
        || piid == IASYNC_OPERATION_WITH_PROGRESS
}

fn make_async_value_from_kind(
    raw: IUnknown,
    generic_def: TypeKind,
    _iid: GUID,
    args: &[TypeKind],
    table: &Arc<MetadataTable>,
) -> crate::result::Result<WinRTValue> {
    let piid = match generic_def {
        TypeKind::Generic { piid, .. } => piid,
        TypeKind::Interface(iid) => iid,
        _ => {
            return Err(crate::result::Error::WindowsError(
                windows_core::Error::from_hresult(windows_core::HRESULT(0x80004002u32 as i32)),
            ));
        }
    };

    let info: windows_future::IAsyncInfo = raw
        .cast()
        .map_err(|e| crate::result::Error::WindowsError(e))?;

    let async_type = if piid == IASYNC_ACTION {
        table.async_action()
    } else if piid == IASYNC_OPERATION {
        let t = args.first().copied().unwrap_or(TypeKind::Object);
        let t_h = table.make(t);
        table.async_operation(&t_h)
    } else if piid == IASYNC_ACTION_WITH_PROGRESS {
        let p = args.first().copied().unwrap_or(TypeKind::Object);
        let p_h = table.make(p);
        table.async_action_with_progress(&p_h)
    } else if piid == IASYNC_OPERATION_WITH_PROGRESS {
        let t = args.first().copied().unwrap_or(TypeKind::Object);
        let p = args.get(1).copied().unwrap_or(TypeKind::Object);
        let t_h = table.make(t);
        let p_h = table.make(p);
        table.async_operation_with_progress(&t_h, &p_h)
    } else {
        return Err(crate::result::Error::WindowsError(
            windows_core::Error::from_hresult(windows_core::HRESULT(0x80004002u32 as i32)),
        ));
    };

    Ok(WinRTValue::Async(crate::value::AsyncInfo {
        info,
        async_type,
    }))
}
