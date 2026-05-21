// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use windows_core::{GUID, Interface};

// ===========================================================================
// Well-known async interface IIDs/PIIDs
// ===========================================================================

pub const IASYNC_ACTION: GUID = windows_future::IAsyncAction::IID;
pub const IASYNC_ACTION_WITH_PROGRESS: GUID =
    GUID::from_u128(0x1f6db258_e803_48a1_9546_eb7353398884);
pub const IASYNC_OPERATION: GUID =
    GUID::from_u128(0x9fc2b0bb_e446_44e2_aa61_9cab8f636af2);
pub const IASYNC_OPERATION_WITH_PROGRESS: GUID =
    GUID::from_u128(0xb5d036d7_e297_498f_ba60_0289e76e23dd);
pub const IVECTOR: GUID =
    GUID::from_u128(0x913337e9_11a1_4345_a3a2_4e7f956e222d);
pub const IVECTOR_VIEW: GUID =
    GUID::from_u128(0xbbe1fa4c_b0e3_4583_baef_1f1b2e483e56);
pub const IITERABLE: GUID =
    GUID::from_u128(0xfaa585ea_6214_4217_afda_7f46de5869b3);
pub const IITERATOR: GUID =
    GUID::from_u128(0x6a79e863_4300_459a_9966_cbb660963ee1);
pub const IMAP: GUID =
    GUID::from_u128(0x3c2925fe_8519_45c1_aa79_197b6718c1c1);
pub const IMAP_VIEW: GUID =
    GUID::from_u128(0xe9bdaaf0_cbf6_4c39_de49_316b34326a17);
pub const IKEY_VALUE_PAIR: GUID =
    GUID::from_u128(0x02b51929_c1c4_4a7e_8940_0312b5c18500);
pub const IOBSERVABLE_VECTOR: GUID =
    GUID::from_u128(0x5917eb53_50b4_4a0d_b309_65862b3f1dbc);
pub const IREFERENCE: GUID =
    GUID::from_u128(0x61c17706_2d65_11e0_9ae8_d48564015472);

pub const ASYNC_ACTION_COMPLETED_HANDLER: GUID =
    windows_future::AsyncActionCompletedHandler::IID;
pub const ASYNC_OPERATION_COMPLETED_HANDLER: GUID =
    GUID::from_u128(0xfcdcf02c_e5d8_4478_915a_4d90b74b83a5);
pub const ASYNC_ACTION_WITH_PROGRESS_COMPLETED_HANDLER: GUID =
    GUID::from_u128(0x9c029f91_cc84_44fd_ac26_0a6c4e555281);
pub const ASYNC_OPERATION_WITH_PROGRESS_COMPLETED_HANDLER: GUID =
    GUID::from_u128(0xe85df41d_6aa7_46e3_a8e2_f009d840c627);

// Progress handler PIIDs
pub const ASYNC_ACTION_PROGRESS_HANDLER: GUID =
    GUID::from_u128(0x6d844858_0cff_4590_ae89_95a5a5c8b4b8);
pub const ASYNC_OPERATION_PROGRESS_HANDLER: GUID =
    GUID::from_u128(0x55690902_0aab_421a_8778_f8ce5026d758);

// ===========================================================================
// TypeKind — unified type discriminator (Copy)
// ===========================================================================

/// Unified type identifier covering all WinRT types. Always `Copy`.
/// Non-Copy data (strings, type-arg lists) is stored in `TypeRegistry` Vecs,
/// referenced by `u32` index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeKind {
    // Primitives (blittable, valid as struct fields)
    Bool,
    I8,
    U8,
    I16,
    U16,
    Char16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    Guid,

    // Reference / special types
    HString,
    Object,
    HResult,
    ArrayOfIUnknown,

    // Types with inline GUID
    Interface(GUID),
    Delegate(GUID),
    Generic { piid: GUID, arity: u32 },

    // Async sugar
    IAsyncAction,
    IAsyncActionWithProgress(u32),     // idx → inner_types
    IAsyncOperation(u32),              // idx → inner_types
    IAsyncOperationWithProgress(u32),  // idx → inner_type_pairs

    // Indexed non-Copy data
    RuntimeClass(u32),                 // idx → runtime_classes
    Parameterized(u32),                // idx → parameterized_types

    // ABI-only
    OutValue(u32),                     // idx → inner_types

    // Named enum — ABI is i32, but carries name for signature computation
    Enum(u32),                         // idx → enum_entries

    // Composite
    Struct(u32),                       // idx → structs
    Array(u32),                        // idx → inner_types
}

impl TypeKind {
    /// Size in bytes for blittable types that can appear in struct fields or arrays.
    /// Returns `None` for non-blittable / compound types whose size depends on registry data.
    pub fn primitive_size(self) -> Option<usize> {
        match self {
            TypeKind::Bool | TypeKind::I8 | TypeKind::U8 => Some(1),
            TypeKind::I16 | TypeKind::U16 | TypeKind::Char16 => Some(2),
            TypeKind::I32 | TypeKind::U32 | TypeKind::F32 | TypeKind::HResult | TypeKind::Enum(_) => Some(4),
            TypeKind::I64 | TypeKind::U64 | TypeKind::F64 => Some(8),
            TypeKind::Guid => Some(16),
            _ => None,
        }
    }

    pub fn primitive_align(self) -> Option<usize> {
        match self {
            TypeKind::Guid => Some(4),
            _ => self.primitive_size(),
        }
    }

    /// libffi type for simple (non-struct) kinds. Returns `None` for Struct.
    pub fn primitive_libffi_type(self) -> Option<libffi::middle::Type> {
        use libffi::middle::Type;
        match self {
            TypeKind::Bool => Some(Type::u8()),
            TypeKind::I8 => Some(Type::i8()),
            TypeKind::U8 => Some(Type::u8()),
            TypeKind::I16 => Some(Type::i16()),
            TypeKind::U16 | TypeKind::Char16 => Some(Type::u16()),
            TypeKind::I32 | TypeKind::HResult | TypeKind::Enum(_) => Some(Type::i32()),
            TypeKind::U32 => Some(Type::u32()),
            TypeKind::I64 => Some(Type::i64()),
            TypeKind::U64 => Some(Type::u64()),
            TypeKind::F32 => Some(Type::f32()),
            TypeKind::F64 => Some(Type::f64()),
            TypeKind::Guid => Some(Type::structure(vec![
                Type::u32(), Type::u16(), Type::u16(),
                Type::u8(), Type::u8(), Type::u8(), Type::u8(),
                Type::u8(), Type::u8(), Type::u8(), Type::u8(),
            ])),
            _ => None,
        }
    }

    /// True for types that are COM interface pointers (need AddRef/Release).
    pub fn is_com_pointer(self) -> bool {
        matches!(
            self,
            TypeKind::Object
                | TypeKind::Interface(_)
                | TypeKind::Delegate(_)
                | TypeKind::RuntimeClass(_)
                | TypeKind::Parameterized(_)
        )
    }

    /// True for types that are heap-allocated when stored in a struct field.
    /// These need special handling in Drop (release) and Clone (duplicate).
    /// Note: Struct fields are handled separately via recursive traversal,
    /// so Struct(_) itself is not listed here.
    pub fn needs_drop(self) -> bool {
        self.is_com_pointer() || matches!(self, TypeKind::HString)
    }

    /// True if this type, or any nested struct field, requires Drop/Clone handling.
    /// Unlike `needs_drop()`, this recurses into Struct fields.
    pub fn needs_drop_recursive(self) -> bool {
        self.needs_drop() || matches!(self, TypeKind::Struct(_))
    }

    /// True for types that can appear as struct fields (memcpy-safe).
    pub fn is_blittable(self) -> bool {
        matches!(
            self,
            TypeKind::Bool | TypeKind::I8 | TypeKind::U8
            | TypeKind::I16 | TypeKind::U16 | TypeKind::Char16
            | TypeKind::I32 | TypeKind::U32 | TypeKind::I64 | TypeKind::U64
            | TypeKind::F32 | TypeKind::F64 | TypeKind::Guid
            | TypeKind::Struct(_)
        )
    }

    /// WinRT type signature string for primitive/simple types.
    /// Returns `None` for types that need registry data (RuntimeClass, Parameterized, etc.).
    pub fn signature(self) -> Option<&'static str> {
        match self {
            TypeKind::Bool => Some("b1"),
            TypeKind::I8 => Some("i1"),
            TypeKind::U8 => Some("u1"),
            TypeKind::I16 => Some("i2"),
            TypeKind::U16 => Some("u2"),
            TypeKind::I32 => Some("i4"),
            TypeKind::U32 => Some("u4"),
            TypeKind::I64 => Some("i8"),
            TypeKind::U64 => Some("u8"),
            TypeKind::F32 => Some("f4"),
            TypeKind::F64 => Some("f8"),
            TypeKind::Char16 => Some("c2"),
            TypeKind::HString => Some("string"),
            TypeKind::Guid => Some("g16"),
            _ => None,
        }
    }
}

// ===========================================================================
// Helper functions (moved from types.rs)
// ===========================================================================

/// Format a GUID as `{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}` (lowercase, with braces).
pub(crate) fn format_guid_braced(guid: &GUID) -> String {
    format!("{{{:?}}}", guid).to_lowercase()
}

/// Build a pinterface signature string: `pinterface({piid};arg1;arg2;...)`
pub(crate) fn pinterface_signature_from_strings(piid_sig: &str, arg_sigs: &[String]) -> String {
    let mut s = format!("pinterface({}", piid_sig);
    for arg in arg_sigs {
        s.push(';');
        s.push_str(arg);
    }
    s.push(')');
    s
}
