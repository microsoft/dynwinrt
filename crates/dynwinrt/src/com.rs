// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use core::ffi::c_void;
use std::{
    cell::{RefCell, UnsafeCell},
    collections::{BTreeMap, BTreeSet},
    mem::{align_of, size_of},
    sync::{Arc, Mutex, MutexGuard, RwLock, TryLockError},
};

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoCreateInstance,
    CoInitializeEx, CoUninitialize,
};
use windows_core::{GUID, IUnknown, Interface as WindowsInterface};

use crate::{
    MetadataTable, TypeHandle, TypeKind, WinRTValue,
    native_call::{
        CapturedHResultPlan, Method as NativeMethod, MethodReturn, OutputCleanup, ParamKind,
        ParameterType, lower_completed_method,
    },
    result,
};

#[path = "com_automation.rs"]
pub(crate) mod automation;
pub use automation::{
    DispatchParamsValue, ExcepInfoValue, PropVariantData, PropVariantType, PropVariantValue,
    PropVariantVector, PropVariantVectorType, SafeArrayBound, SafeArrayElementType,
    SafeArrayElementValue, SafeArrayValue, VariantData, VariantType, VariantValue,
};

const RPC_E_CHANGED_MODE: windows_core::HRESULT = windows_core::HRESULT(0x80010106u32 as i32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceBase {
    IUnknown,
    IInspectable,
}

impl InterfaceBase {
    pub const fn first_method_slot(self) -> usize {
        match self {
            Self::IUnknown => 3,
            Self::IInspectable => 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerOutputKind {
    None,
    Unclassified,
    Com,
    CoTaskMem,
    Bstr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeStructScalar {
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
    ISize,
    USize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeStructFieldType {
    Scalar(NativeStructScalar),
    Guid,
    Pointer,
    Struct(Arc<NativeStructLayout>),
}

impl NativeStructFieldType {
    fn size_alignment(&self) -> (usize, usize) {
        match self {
            Self::Scalar(scalar) => match scalar {
                NativeStructScalar::I8 | NativeStructScalar::U8 => (1, 1),
                NativeStructScalar::I16 | NativeStructScalar::U16 => (2, 2),
                NativeStructScalar::I32 | NativeStructScalar::U32 | NativeStructScalar::F32 => {
                    (4, 4)
                }
                NativeStructScalar::I64 | NativeStructScalar::U64 | NativeStructScalar::F64 => {
                    (8, 8)
                }
                NativeStructScalar::ISize | NativeStructScalar::USize => {
                    (size_of::<usize>(), align_of::<usize>())
                }
            },
            Self::Guid => (16, 4),
            Self::Pointer => (size_of::<usize>(), align_of::<usize>()),
            Self::Struct(layout) => (layout.size, layout.alignment),
        }
    }

    fn libffi_type(&self) -> libffi::middle::Type {
        use libffi::middle::Type;
        match self {
            Self::Scalar(scalar) => match scalar {
                NativeStructScalar::I8 => Type::i8(),
                NativeStructScalar::U8 => Type::u8(),
                NativeStructScalar::I16 => Type::i16(),
                NativeStructScalar::U16 => Type::u16(),
                NativeStructScalar::I32 => Type::i32(),
                NativeStructScalar::U32 => Type::u32(),
                NativeStructScalar::I64 => Type::i64(),
                NativeStructScalar::U64 => Type::u64(),
                NativeStructScalar::F32 => Type::f32(),
                NativeStructScalar::F64 => Type::f64(),
                NativeStructScalar::ISize => {
                    if size_of::<usize>() == 8 {
                        Type::i64()
                    } else {
                        Type::i32()
                    }
                }
                NativeStructScalar::USize => {
                    if size_of::<usize>() == 8 {
                        Type::u64()
                    } else {
                        Type::u32()
                    }
                }
            },
            Self::Guid => {
                let mut fields = vec![Type::u32(), Type::u16(), Type::u16()];
                fields.extend(std::iter::repeat_with(Type::u8).take(8));
                Type::structure(fields)
            }
            Self::Pointer => Type::pointer(),
            Self::Struct(layout) => layout.libffi_type(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStructField {
    name: String,
    offset: usize,
    count: u32,
    typ: NativeStructFieldType,
}

impl NativeStructField {
    pub fn new(
        name: impl Into<String>,
        offset: usize,
        count: u32,
        typ: NativeStructFieldType,
    ) -> result::Result<Self> {
        let name = name.into();
        if name.trim().is_empty() || count == 0 {
            return Err(invalid_argument(
                "native struct fields require a name and non-zero count",
            ));
        }
        Ok(Self {
            name,
            offset,
            count,
            typ,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStructLayout {
    name: String,
    size: usize,
    alignment: usize,
    fields: Vec<NativeStructField>,
}

impl NativeStructLayout {
    pub fn new(
        name: impl Into<String>,
        size: usize,
        alignment: usize,
        fields: Vec<NativeStructField>,
    ) -> result::Result<Self> {
        let name = name.into();
        if name.trim().is_empty() || size == 0 {
            return Err(invalid_argument(
                "native struct layout requires a name and non-zero size",
            ));
        }
        if alignment == 0 || !alignment.is_power_of_two() || size % alignment != 0 {
            return Err(invalid_argument(
                "native struct alignment must be a power of two and divide its size",
            ));
        }
        if fields.is_empty() {
            return Err(invalid_argument(
                "native struct layout requires at least one field",
            ));
        }
        let mut names = std::collections::HashSet::new();
        let mut intervals = Vec::with_capacity(fields.len());
        let mut maximum_alignment = 1usize;
        for field in &fields {
            if !names.insert(field.name.as_str()) {
                return Err(invalid_argument(format!(
                    "native struct `{name}` has duplicate field `{}`",
                    field.name
                )));
            }
            let (element_size, field_alignment) = field.typ.size_alignment();
            maximum_alignment = maximum_alignment.max(field_alignment);
            if field.offset % field_alignment != 0 {
                return Err(invalid_argument(format!(
                    "native struct `{name}` field `{}` offset {} violates alignment {field_alignment}",
                    field.name, field.offset
                )));
            }
            let field_size = element_size
                .checked_mul(field.count as usize)
                .ok_or_else(|| invalid_argument("native struct field size overflow"))?;
            let end = field
                .offset
                .checked_add(field_size)
                .ok_or_else(|| invalid_argument("native struct field end overflow"))?;
            if end > size {
                return Err(invalid_argument(format!(
                    "native struct `{name}` field `{}` extends past {size} bytes",
                    field.name
                )));
            }
            if intervals
                .iter()
                .any(|(start, existing_end)| field.offset < *existing_end && *start < end)
            {
                return Err(invalid_argument(format!(
                    "native struct `{name}` has overlapping field `{}`",
                    field.name
                )));
            }
            intervals.push((field.offset, end));
        }
        if maximum_alignment != alignment {
            return Err(invalid_argument(format!(
                "native struct `{name}` declares alignment {alignment}, computed {maximum_alignment}"
            )));
        }
        Ok(Self {
            name,
            size,
            alignment,
            fields,
        })
    }

    pub const fn size(&self) -> usize {
        self.size
    }

    pub const fn alignment(&self) -> usize {
        self.alignment
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn libffi_type(&self) -> libffi::middle::Type {
        let mut fields = self.fields.iter().collect::<Vec<_>>();
        fields.sort_by_key(|field| field.offset);
        let mut elements = Vec::new();
        let mut cursor = 0usize;
        for field in fields {
            elements.extend(
                std::iter::repeat_with(libffi::middle::Type::u8).take(
                    field
                        .offset
                        .checked_sub(cursor)
                        .expect("validated native struct fields do not overlap"),
                ),
            );
            for _ in 0..field.count {
                elements.push(field.typ.libffi_type());
            }
            cursor = field.offset + field.typ.size_alignment().0 * field.count as usize;
        }
        elements.extend(std::iter::repeat_with(libffi::middle::Type::u8).take(self.size - cursor));
        libffi::middle::Type::structure(elements)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStructValue {
    layout: Arc<NativeStructLayout>,
    bytes: Vec<u8>,
}

impl NativeStructValue {
    pub fn new(layout: Arc<NativeStructLayout>, bytes: Vec<u8>) -> result::Result<Self> {
        if bytes.len() != layout.size {
            return Err(invalid_argument(format!(
                "native struct `{}` requires {} bytes, received {}",
                layout.name,
                layout.size,
                bytes.len()
            )));
        }
        Ok(Self { layout, bytes })
    }

    pub fn zeroed(layout: Arc<NativeStructLayout>) -> Self {
        Self {
            bytes: vec![0; layout.size],
            layout,
        }
    }

    pub fn layout(&self) -> &Arc<NativeStructLayout> {
        &self.layout
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeUnionFieldType {
    Scalar(NativeStructScalar),
    Guid,
    Pointer,
    Struct(Arc<NativeStructLayout>),
}

impl NativeUnionFieldType {
    fn size_alignment(&self) -> (usize, usize) {
        match self {
            Self::Scalar(value) => NativeStructFieldType::Scalar(*value).size_alignment(),
            Self::Guid => NativeStructFieldType::Guid.size_alignment(),
            Self::Pointer => NativeStructFieldType::Pointer.size_alignment(),
            Self::Struct(layout) => NativeStructFieldType::Struct(layout.clone()).size_alignment(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeUnionField {
    name: String,
    count: u32,
    typ: NativeUnionFieldType,
}

impl NativeUnionField {
    pub fn new(
        name: impl Into<String>,
        count: u32,
        typ: NativeUnionFieldType,
    ) -> result::Result<Self> {
        let name = name.into();
        if name.trim().is_empty() || count == 0 {
            return Err(invalid_argument(
                "native union fields require a name and non-zero count",
            ));
        }
        Ok(Self { name, count, typ })
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeUnionLayout {
    name: String,
    size: usize,
    alignment: usize,
    fields: Vec<NativeUnionField>,
}

impl NativeUnionLayout {
    pub fn new(
        name: impl Into<String>,
        size: usize,
        alignment: usize,
        fields: Vec<NativeUnionField>,
    ) -> result::Result<Self> {
        let name = name.into();
        if name.trim().is_empty() || size == 0 {
            return Err(invalid_argument(
                "native union layout requires a name and non-zero size",
            ));
        }
        if alignment == 0 || !alignment.is_power_of_two() || size % alignment != 0 {
            return Err(invalid_argument(
                "native union alignment must be a power of two and divide its size",
            ));
        }
        if fields.is_empty() {
            return Err(invalid_argument(
                "native union layout requires at least one field",
            ));
        }
        let mut names = std::collections::HashSet::new();
        let mut maximum_alignment = 1usize;
        for field in &fields {
            if !names.insert(field.name.as_str()) {
                return Err(invalid_argument(format!(
                    "native union `{name}` has duplicate field `{}`",
                    field.name
                )));
            }
            let (element_size, field_alignment) = field.typ.size_alignment();
            maximum_alignment = maximum_alignment.max(field_alignment);
            let field_size = element_size
                .checked_mul(field.count as usize)
                .ok_or_else(|| invalid_argument("native union field size overflow"))?;
            if field_size > size {
                return Err(invalid_argument(format!(
                    "native union `{name}` field `{}` requires {field_size} bytes but the union has {size}",
                    field.name
                )));
            }
        }
        if maximum_alignment != alignment {
            return Err(invalid_argument(format!(
                "native union `{name}` declares alignment {alignment}, computed {maximum_alignment}"
            )));
        }
        Ok(Self {
            name,
            size,
            alignment,
            fields,
        })
    }

    pub const fn size(&self) -> usize {
        self.size
    }

    pub const fn alignment(&self) -> usize {
        self.alignment
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn has_field(&self, name: &str) -> bool {
        self.fields.iter().any(|field| field.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeUnionValue {
    layout: Arc<NativeUnionLayout>,
    active_field: String,
    bytes: Vec<u8>,
}

impl NativeUnionValue {
    pub fn new(
        layout: Arc<NativeUnionLayout>,
        active_field: impl Into<String>,
        bytes: Vec<u8>,
    ) -> result::Result<Self> {
        let active_field = active_field.into();
        if !layout.has_field(&active_field) {
            return Err(invalid_argument(format!(
                "native union `{}` has no active field `{active_field}`",
                layout.name
            )));
        }
        if bytes.len() != layout.size {
            return Err(invalid_argument(format!(
                "native union `{}` requires {} bytes, received {}",
                layout.name,
                layout.size,
                bytes.len()
            )));
        }
        Ok(Self {
            layout,
            active_field,
            bytes,
        })
    }

    pub fn zeroed(
        layout: Arc<NativeUnionLayout>,
        active_field: impl Into<String>,
    ) -> result::Result<Self> {
        let bytes = vec![0; layout.size];
        Self::new(layout, active_field, bytes)
    }

    pub fn layout(&self) -> &Arc<NativeUnionLayout> {
        &self.layout
    }

    pub fn active_field(&self) -> &str {
        &self.active_field
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone)]
pub struct BstrValue(Option<String>);

impl BstrValue {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Some(value.into()))
    }

    pub fn null() -> Self {
        Self(None)
    }

    pub fn as_deref(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

#[derive(Debug, Clone)]
pub enum Value {
    WinRt(WinRTValue),
    Bstr(BstrValue),
    NativeStruct(NativeStructValue),
    NativeUnion(NativeUnionValue),
    Variant(VariantValue),
    SafeArray(SafeArrayValue),
    PropVariant(PropVariantValue),
    DispatchParams(DispatchParamsValue),
    ExcepInfo(ExcepInfoValue),
    Buffer(ComBufferValue),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringEncoding {
    Utf16,
    Ansi,
}

#[derive(Debug, Clone)]
enum EncodedString {
    Utf16(Box<[u16]>),
    Ansi(Box<[u8]>),
}

impl EncodedString {
    fn pointer(&mut self) -> *mut c_void {
        match self {
            Self::Utf16(value) => value.as_mut_ptr().cast(),
            Self::Ansi(value) => value.as_mut_ptr().cast(),
        }
    }
}

#[repr(align(16))]
#[derive(Debug, Clone, Copy)]
struct AlignedBufferBlock {
    _bytes: [u8; 16],
}

#[derive(Debug)]
enum ComBufferStorage {
    Borrowed {
        ptr: *mut u8,
        byte_len: usize,
        source_element_size: usize,
        raw_bytes: bool,
        writable: bool,
        _empty_storage: Option<Arc<EmptyBufferStorage>>,
    },
    OwnedInput {
        bytes: Vec<u8>,
        source_element_size: usize,
        native_layout_name: String,
    },
    StringArray {
        encoding: StringEncoding,
        strings: Vec<EncodedString>,
        pointers: Vec<*mut c_void>,
    },
    InterfaceArray {
        iid: GUID,
        values: Vec<IUnknown>,
        pointers: Vec<*mut c_void>,
    },
    BstrArray {
        values: Vec<String>,
    },
    VariantArray {
        values: Vec<VariantValue>,
    },
    CallerOutput {
        blocks: Arc<Mutex<Vec<AlignedBufferBlock>>>,
        byte_len: usize,
        source_element_size: usize,
        native_layout_name: Option<String>,
        element_kind: BufferElementKind,
    },
    Owned {
        bytes: Vec<u8>,
        count: usize,
    },
    OwnedCom {
        values: Vec<WinRTValue>,
    },
    OwnedStrings {
        values: Vec<String>,
    },
    OwnedVariants {
        values: Vec<VariantValue>,
    },
}

impl Clone for ComBufferStorage {
    fn clone(&self) -> Self {
        match self {
            Self::Borrowed {
                ptr,
                byte_len,
                source_element_size,
                raw_bytes,
                writable,
                _empty_storage,
            } => Self::Borrowed {
                ptr: *ptr,
                byte_len: *byte_len,
                source_element_size: *source_element_size,
                raw_bytes: *raw_bytes,
                writable: *writable,
                _empty_storage: _empty_storage.clone(),
            },
            Self::OwnedInput {
                bytes,
                source_element_size,
                native_layout_name,
            } => Self::OwnedInput {
                bytes: bytes.clone(),
                source_element_size: *source_element_size,
                native_layout_name: native_layout_name.clone(),
            },
            Self::StringArray {
                encoding, strings, ..
            } => {
                let mut strings = strings.clone();
                let pointers = strings.iter_mut().map(EncodedString::pointer).collect();
                Self::StringArray {
                    encoding: *encoding,
                    strings,
                    pointers,
                }
            }
            Self::InterfaceArray { iid, values, .. } => {
                let values = values.clone();
                let pointers = values.iter().map(|value| value.as_raw()).collect();
                Self::InterfaceArray {
                    iid: *iid,
                    values,
                    pointers,
                }
            }
            Self::BstrArray { values } => Self::BstrArray {
                values: values.clone(),
            },
            Self::VariantArray { values } => Self::VariantArray {
                values: values.clone(),
            },
            Self::CallerOutput {
                blocks,
                byte_len,
                source_element_size,
                native_layout_name,
                element_kind,
            } => Self::CallerOutput {
                blocks: Arc::clone(blocks),
                byte_len: *byte_len,
                source_element_size: *source_element_size,
                native_layout_name: native_layout_name.clone(),
                element_kind: *element_kind,
            },
            Self::Owned { bytes, count } => Self::Owned {
                bytes: bytes.clone(),
                count: *count,
            },
            Self::OwnedCom { values } => Self::OwnedCom {
                values: values.clone(),
            },
            Self::OwnedStrings { values } => Self::OwnedStrings {
                values: values.clone(),
            },
            Self::OwnedVariants { values } => Self::OwnedVariants {
                values: values.clone(),
            },
        }
    }
}

#[repr(align(16))]
struct EmptyBufferStorage(UnsafeCell<[u8; 16]>);

impl std::fmt::Debug for EmptyBufferStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmptyBufferStorage").finish_non_exhaustive()
    }
}

// Safety: this storage is used only as a stable, non-null address for native
// contracts with a zero byte count. A conforming callee never dereferences it.
unsafe impl Send for EmptyBufferStorage {}
unsafe impl Sync for EmptyBufferStorage {}

#[derive(Debug, Clone)]
pub struct ComBufferValue {
    storage: ComBufferStorage,
}

impl ComBufferValue {
    pub fn null() -> Self {
        Self {
            storage: ComBufferStorage::Borrowed {
                ptr: std::ptr::null_mut(),
                byte_len: 0,
                source_element_size: 1,
                raw_bytes: true,
                writable: true,
                _empty_storage: None,
            },
        }
    }

    /// # Safety
    ///
    /// `ptr` must remain valid for `byte_len` bytes while this value is used by
    /// a COM invocation. Writable buffers must point to mutable storage.
    pub unsafe fn borrowed(
        ptr: *mut u8,
        byte_len: usize,
        source_element_size: usize,
        raw_bytes: bool,
        writable: bool,
    ) -> result::Result<Self> {
        if source_element_size == 0 {
            return Err(invalid_argument(
                "COM buffer source element size must be non-zero",
            ));
        }

        if byte_len > 0 && ptr.is_null() {
            return Err(invalid_argument(
                "non-empty COM buffers require non-null backing storage",
            ));
        }
        let empty_storage = (byte_len == 0 && ptr.is_null())
            .then(|| Arc::new(EmptyBufferStorage(UnsafeCell::new([0; 16]))));
        let ptr = empty_storage
            .as_ref()
            .map_or(ptr, |storage| storage.0.get().cast::<u8>());
        Ok(Self {
            storage: ComBufferStorage::Borrowed {
                ptr,
                byte_len,
                source_element_size,
                raw_bytes,
                writable,
                _empty_storage: empty_storage,
            },
        })
    }

    pub fn native_struct_input(
        bytes: Vec<u8>,
        layout: &NativeStructLayout,
    ) -> result::Result<Self> {
        if bytes.len() % layout.size() != 0 {
            return Err(invalid_argument(format!(
                "native struct array byte length {} is not a multiple of `{}` size {}",
                bytes.len(),
                layout.name(),
                layout.size()
            )));
        }
        Ok(Self {
            storage: ComBufferStorage::OwnedInput {
                bytes,
                source_element_size: layout.size(),
                native_layout_name: layout.name().to_string(),
            },
        })
    }

    pub fn string_array(values: Vec<String>, encoding: StringEncoding) -> result::Result<Self> {
        let mut strings = Vec::with_capacity(values.len());
        for value in values {
            if value.contains('\0') {
                return Err(invalid_argument(
                    "COM string array elements cannot contain embedded NUL characters",
                ));
            }
            let encoded = match encoding {
                StringEncoding::Utf16 => EncodedString::Utf16(
                    value
                        .encode_utf16()
                        .chain(std::iter::once(0))
                        .collect::<Vec<_>>()
                        .into_boxed_slice(),
                ),
                StringEncoding::Ansi => {
                    if !value.is_ascii() {
                        return Err(invalid_argument(
                            "COM ANSI string arrays currently accept ASCII text only",
                        ));
                    }
                    let mut bytes = value.into_bytes();
                    bytes.push(0);
                    EncodedString::Ansi(bytes.into_boxed_slice())
                }
            };
            strings.push(encoded);
        }
        let pointers = strings.iter_mut().map(EncodedString::pointer).collect();
        Ok(Self {
            storage: ComBufferStorage::StringArray {
                encoding,
                strings,
                pointers,
            },
        })
    }

    pub fn interface_array(iid: GUID, values: Vec<IUnknown>) -> result::Result<Self> {
        let mut exact = Vec::with_capacity(values.len());
        for value in values {
            let mut queried = std::ptr::null_mut();
            unsafe { value.query(&iid, &mut queried) }
                .ok()
                .map_err(result::Error::WindowsError)?;
            exact.push(unsafe { IUnknown::from_raw(queried) });
        }
        let pointers = exact.iter().map(|value| value.as_raw()).collect();
        Ok(Self {
            storage: ComBufferStorage::InterfaceArray {
                iid,
                values: exact,
                pointers,
            },
        })
    }

    pub fn bstr_array(values: Vec<String>) -> Self {
        Self {
            storage: ComBufferStorage::BstrArray { values },
        }
    }

    pub fn variant_array(values: Vec<VariantValue>) -> result::Result<Self> {
        for value in &values {
            value.validate_supported()?;
        }
        Ok(Self {
            storage: ComBufferStorage::VariantArray { values },
        })
    }

    pub fn caller_output(element_type: &Type, count: usize) -> result::Result<Self> {
        let element = BufferElementPlan::from_type(element_type)?;
        Self::caller_output_with_plan(element, count)
    }

    pub fn enumerator_output(element_type: &Type, count: usize) -> result::Result<Self> {
        let element = BufferElementPlan::from_enumerator_type(element_type)?;
        Self::caller_output_with_plan(element, count)
    }

    fn caller_output_with_plan(element: BufferElementPlan, count: usize) -> result::Result<Self> {
        let byte_len = count_bytes(count, &element, BufferCountUnit::Elements)?;
        let block_count = byte_len.div_ceil(size_of::<AlignedBufferBlock>());
        Ok(Self {
            storage: ComBufferStorage::CallerOutput {
                blocks: Arc::new(Mutex::new(vec![
                    AlignedBufferBlock { _bytes: [0; 16] };
                    block_count
                ])),
                byte_len,
                source_element_size: element.size,
                native_layout_name: element.native_layout_name,
                element_kind: element.kind,
            },
        })
    }

    fn owned(bytes: Vec<u8>, count: usize) -> Self {
        Self {
            storage: ComBufferStorage::Owned { bytes, count },
        }
    }

    fn owned_com(values: Vec<WinRTValue>) -> Self {
        Self {
            storage: ComBufferStorage::OwnedCom { values },
        }
    }

    fn owned_strings(values: Vec<String>) -> Self {
        Self {
            storage: ComBufferStorage::OwnedStrings { values },
        }
    }

    fn owned_variants(values: Vec<VariantValue>) -> Self {
        Self {
            storage: ComBufferStorage::OwnedVariants { values },
        }
    }

    pub fn into_com_values(self) -> result::Result<Vec<WinRTValue>> {
        match self.storage {
            ComBufferStorage::OwnedCom { values } => Ok(values),
            _ => Err(invalid_argument(
                "COM buffer result does not contain managed interface elements",
            )),
        }
    }

    pub fn into_strings(self) -> result::Result<Vec<String>> {
        match self.storage {
            ComBufferStorage::OwnedStrings { values } => Ok(values),
            _ => Err(invalid_argument(
                "COM buffer result does not contain owned string elements",
            )),
        }
    }

    pub fn into_variants(self) -> result::Result<Vec<VariantValue>> {
        match self.storage {
            ComBufferStorage::OwnedVariants { values } => Ok(values),
            _ => Err(invalid_argument(
                "COM buffer result does not contain owned VARIANT elements",
            )),
        }
    }

    pub fn bytes(&self) -> Option<&[u8]> {
        match &self.storage {
            ComBufferStorage::Owned { bytes, .. } => Some(bytes),
            ComBufferStorage::OwnedInput { bytes, .. } => Some(bytes),
            ComBufferStorage::Borrowed { .. }
            | ComBufferStorage::StringArray { .. }
            | ComBufferStorage::InterfaceArray { .. }
            | ComBufferStorage::BstrArray { .. }
            | ComBufferStorage::VariantArray { .. }
            | ComBufferStorage::CallerOutput { .. }
            | ComBufferStorage::OwnedCom { .. }
            | ComBufferStorage::OwnedStrings { .. }
            | ComBufferStorage::OwnedVariants { .. } => None,
        }
    }

    pub fn snapshot_bytes(&self) -> result::Result<Option<Vec<u8>>> {
        match &self.storage {
            ComBufferStorage::Owned { bytes, .. } | ComBufferStorage::OwnedInput { bytes, .. } => {
                Ok(Some(bytes.clone()))
            }
            ComBufferStorage::CallerOutput {
                blocks, byte_len, ..
            } => {
                let blocks = blocks
                    .lock()
                    .map_err(|_| invalid_argument("caller-output COM storage lock is poisoned"))?;
                Ok(Some(
                    unsafe { std::slice::from_raw_parts(blocks.as_ptr().cast::<u8>(), *byte_len) }
                        .to_vec(),
                ))
            }
            ComBufferStorage::Borrowed { .. }
            | ComBufferStorage::StringArray { .. }
            | ComBufferStorage::InterfaceArray { .. }
            | ComBufferStorage::BstrArray { .. }
            | ComBufferStorage::VariantArray { .. }
            | ComBufferStorage::OwnedCom { .. }
            | ComBufferStorage::OwnedStrings { .. }
            | ComBufferStorage::OwnedVariants { .. } => Ok(None),
        }
    }

    pub fn count(&self) -> usize {
        match &self.storage {
            ComBufferStorage::Owned { count, .. } => *count,
            ComBufferStorage::StringArray { pointers, .. } => pointers.len(),
            ComBufferStorage::InterfaceArray { pointers, .. } => pointers.len(),
            ComBufferStorage::BstrArray { values } => values.len(),
            ComBufferStorage::VariantArray { values } => values.len(),
            ComBufferStorage::CallerOutput {
                byte_len,
                source_element_size,
                ..
            } => byte_len / source_element_size,
            ComBufferStorage::OwnedCom { values } => values.len(),
            ComBufferStorage::OwnedStrings { values } => values.len(),
            ComBufferStorage::OwnedVariants { values } => values.len(),
            ComBufferStorage::Borrowed {
                byte_len,
                source_element_size,
                ..
            } => byte_len / source_element_size,
            ComBufferStorage::OwnedInput {
                bytes,
                source_element_size,
                ..
            } => bytes.len() / source_element_size,
        }
    }

    pub fn element_count(&self, element_type: &Type) -> result::Result<usize> {
        let element = BufferElementPlan::from_type(element_type)?;
        let prepared = prepare_borrowed_buffer(self, &element, false)?;
        buffer_count(prepared.byte_len, &element, BufferCountUnit::Elements)
    }

    fn borrowed_parts(
        &self,
    ) -> result::Result<(
        *mut u8,
        usize,
        usize,
        bool,
        bool,
        Option<&str>,
        Option<StringEncoding>,
    )> {
        match &self.storage {
            ComBufferStorage::Borrowed {
                ptr,
                byte_len,
                source_element_size,
                raw_bytes,
                writable,
                ..
            } => Ok((
                *ptr,
                *byte_len,
                *source_element_size,
                *raw_bytes,
                *writable,
                None,
                None,
            )),
            ComBufferStorage::OwnedInput {
                bytes,
                source_element_size,
                native_layout_name,
            } => Ok((
                bytes.as_ptr().cast_mut(),
                bytes.len(),
                *source_element_size,
                false,
                false,
                Some(native_layout_name),
                None,
            )),
            ComBufferStorage::StringArray {
                encoding, pointers, ..
            } => Ok((
                pointers.as_ptr().cast_mut().cast(),
                pointers.len() * size_of::<*mut c_void>(),
                size_of::<*mut c_void>(),
                false,
                false,
                None,
                Some(*encoding),
            )),
            ComBufferStorage::InterfaceArray { pointers, .. } => Ok((
                pointers.as_ptr().cast_mut().cast(),
                pointers.len() * size_of::<*mut c_void>(),
                size_of::<*mut c_void>(),
                false,
                false,
                None,
                None,
            )),
            ComBufferStorage::BstrArray { .. } | ComBufferStorage::VariantArray { .. } => Err(
                invalid_argument("owning COM input arrays require call-local prepared storage"),
            ),
            ComBufferStorage::CallerOutput { .. } => Err(invalid_argument(
                "caller-output COM storage requires exclusive invocation access",
            )),
            ComBufferStorage::Owned { .. } => Err(invalid_argument(
                "an owned COM buffer result cannot be reused as borrowed call storage",
            )),
            ComBufferStorage::OwnedCom { .. } => Err(invalid_argument(
                "a managed COM array result cannot be reused as borrowed call storage",
            )),
            ComBufferStorage::OwnedStrings { .. } | ComBufferStorage::OwnedVariants { .. } => {
                Err(invalid_argument(
                    "an owned COM array result cannot be reused as borrowed call storage",
                ))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferCountUnit {
    Elements,
    Bytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferAllocator {
    CoTaskMem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BufferElementCleanup {
    None,
    ComRelease,
    BstrFree,
    VariantClear,
    CoTaskMemFree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BufferElementKind {
    Plain,
    StringPointer(StringEncoding),
    ComInterface(GUID),
    Bstr,
    Variant,
    CoTaskMemWideString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BufferElementPlan {
    size: usize,
    alignment: usize,
    cleanup: BufferElementCleanup,
    native_layout_name: Option<String>,
    kind: BufferElementKind,
}

impl BufferElementPlan {
    fn from_type(typ: &Type) -> result::Result<Self> {
        let (size, alignment) = match &typ.abi {
            ParameterType::WinRT(handle)
                if matches!(
                    handle.kind(),
                    TypeKind::Bool
                        | TypeKind::I8
                        | TypeKind::U8
                        | TypeKind::I16
                        | TypeKind::U16
                        | TypeKind::Char16
                        | TypeKind::I32
                        | TypeKind::U32
                        | TypeKind::I64
                        | TypeKind::U64
                        | TypeKind::F32
                        | TypeKind::F64
                        | TypeKind::Guid
                        | TypeKind::HResult
                        | TypeKind::Enum(_)
                ) =>
            {
                (handle.size_of(), handle.align_of())
            }
            ParameterType::NativeStruct(layout) => (layout.size(), layout.alignment()),
            ParameterType::WinRT(handle) if handle.kind().is_com_pointer() => {
                let iid = handle.iid().ok_or_else(|| {
                    invalid_argument("COM interface buffer elements require an exact IID")
                })?;
                return Ok(Self {
                    size: size_of::<*mut c_void>(),
                    alignment: align_of::<*mut c_void>(),
                    cleanup: BufferElementCleanup::ComRelease,
                    native_layout_name: None,
                    kind: BufferElementKind::ComInterface(iid),
                });
            }
            ParameterType::CoTaskMemWideString => {
                return Ok(Self {
                    size: size_of::<*mut c_void>(),
                    alignment: align_of::<*mut c_void>(),
                    cleanup: BufferElementCleanup::CoTaskMemFree,
                    native_layout_name: None,
                    kind: BufferElementKind::CoTaskMemWideString,
                });
            }
            ParameterType::Pointer
            | ParameterType::NativeStructPointer { .. }
            | ParameterType::NativeUnionPointer(_) => {
                return Err(invalid_argument(
                    "pointer buffer elements require explicit element ownership",
                ));
            }
            ParameterType::Bstr { .. } => {
                return Ok(Self {
                    size: size_of::<*mut c_void>(),
                    alignment: align_of::<*mut c_void>(),
                    cleanup: BufferElementCleanup::BstrFree,
                    native_layout_name: None,
                    kind: BufferElementKind::Bstr,
                });
            }
            ParameterType::Variant => {
                return Ok(Self {
                    size: crate::com::automation::variant_size(),
                    alignment: crate::com::automation::variant_alignment(),
                    cleanup: BufferElementCleanup::VariantClear,
                    native_layout_name: None,
                    kind: BufferElementKind::Variant,
                });
            }
            ParameterType::VariantByValue
            | ParameterType::SafeArray { .. }
            | ParameterType::PropVariant
            | ParameterType::DispatchParams
            | ParameterType::ExcepInfo => {
                return Err(invalid_argument(
                    "Automation buffer elements require dedicated ownership and cleanup plans",
                ));
            }
            ParameterType::WinRT(_) => {
                return Err(invalid_argument(
                    "counted COM buffers currently require primitive, GUID, or enum elements",
                ));
            }
        };
        if size == 0 || alignment == 0 {
            return Err(invalid_argument(
                "counted COM buffer element layout must be non-zero",
            ));
        }
        Ok(Self {
            size,
            alignment,
            cleanup: BufferElementCleanup::None,
            native_layout_name: match &typ.abi {
                ParameterType::NativeStruct(layout) => Some(layout.name().to_string()),
                _ => None,
            },
            kind: BufferElementKind::Plain,
        })
    }

    fn string_pointer(encoding: StringEncoding) -> Self {
        Self {
            size: size_of::<*mut c_void>(),
            alignment: align_of::<*mut c_void>(),
            cleanup: BufferElementCleanup::None,
            native_layout_name: None,
            kind: BufferElementKind::StringPointer(encoding),
        }
    }

    fn from_enumerator_type(typ: &Type) -> result::Result<Self> {
        Self::from_type(typ)
    }
}

#[derive(Debug, Clone)]
pub struct Type {
    abi: ParameterType,
    pointer_output: PointerOutputKind,
}

impl Type {
    pub fn winrt(typ: TypeHandle) -> Self {
        Self {
            abi: ParameterType::winrt(typ),
            pointer_output: PointerOutputKind::None,
        }
    }

    pub fn pointer() -> Self {
        Self::pointer_with_output(PointerOutputKind::Unclassified)
    }

    pub fn borrowed_handle_output() -> Self {
        Self::pointer_with_output(PointerOutputKind::None)
    }

    pub fn owned_com_pointer() -> Self {
        Self::pointer_with_output(PointerOutputKind::Com)
    }

    pub fn co_task_mem_pointer() -> Self {
        Self::pointer_with_output(PointerOutputKind::CoTaskMem)
    }

    pub fn co_task_mem_wide_string() -> Self {
        Self {
            abi: ParameterType::co_task_mem_wide_string(),
            pointer_output: PointerOutputKind::CoTaskMem,
        }
    }

    pub fn bstr_pointer() -> Self {
        Self::pointer_with_output(PointerOutputKind::Bstr)
    }

    pub fn bstr() -> Self {
        Self {
            abi: ParameterType::bstr(false),
            pointer_output: PointerOutputKind::Bstr,
        }
    }

    pub fn nullable_bstr() -> Self {
        Self {
            abi: ParameterType::bstr(true),
            pointer_output: PointerOutputKind::Bstr,
        }
    }

    pub fn native_struct(layout: Arc<NativeStructLayout>) -> Self {
        Self {
            abi: ParameterType::native_struct(layout),
            pointer_output: PointerOutputKind::None,
        }
    }

    pub fn native_struct_pointer(layout: Arc<NativeStructLayout>) -> Self {
        Self {
            abi: ParameterType::native_struct_pointer(layout, false),
            pointer_output: PointerOutputKind::None,
        }
    }

    pub fn nullable_native_struct_pointer(layout: Arc<NativeStructLayout>) -> Self {
        Self {
            abi: ParameterType::native_struct_pointer(layout, true),
            pointer_output: PointerOutputKind::None,
        }
    }

    pub fn native_union_pointer(layout: Arc<NativeUnionLayout>) -> Self {
        Self {
            abi: ParameterType::native_union_pointer(layout),
            pointer_output: PointerOutputKind::None,
        }
    }

    pub fn variant() -> Self {
        Self {
            abi: ParameterType::variant(),
            pointer_output: PointerOutputKind::None,
        }
    }

    pub fn variant_by_value() -> Self {
        Self {
            abi: ParameterType::variant_by_value(),
            pointer_output: PointerOutputKind::None,
        }
    }

    pub fn safe_array() -> Self {
        Self {
            abi: ParameterType::safe_array(None),
            pointer_output: PointerOutputKind::None,
        }
    }

    pub fn typed_safe_array(element: SafeArrayElementType) -> Self {
        Self {
            abi: ParameterType::safe_array(Some(element)),
            pointer_output: PointerOutputKind::None,
        }
    }

    pub fn typed_interface_safe_array(iid: GUID) -> Self {
        Self {
            abi: ParameterType::interface_safe_array(iid),
            pointer_output: PointerOutputKind::None,
        }
    }

    pub fn nullable_typed_safe_array(element: SafeArrayElementType) -> Self {
        Self {
            abi: ParameterType::nullable_safe_array(element, None),
            pointer_output: PointerOutputKind::None,
        }
    }

    pub fn nullable_typed_interface_safe_array(iid: GUID) -> Self {
        Self {
            abi: ParameterType::nullable_safe_array(SafeArrayElementType::Unknown, Some(iid)),
            pointer_output: PointerOutputKind::None,
        }
    }

    pub fn prop_variant() -> Self {
        Self {
            abi: ParameterType::prop_variant(),
            pointer_output: PointerOutputKind::None,
        }
    }

    pub fn dispatch_params() -> Self {
        Self {
            abi: ParameterType::dispatch_params(),
            pointer_output: PointerOutputKind::None,
        }
    }

    pub fn excep_info() -> Self {
        Self {
            abi: ParameterType::excep_info(),
            pointer_output: PointerOutputKind::None,
        }
    }

    fn pointer_with_output(pointer_output: PointerOutputKind) -> Self {
        Self {
            abi: ParameterType::pointer(),
            pointer_output,
        }
    }

    fn output_cleanup(&self) -> OutputCleanup {
        match self.pointer_output {
            PointerOutputKind::Com => OutputCleanup::ComRelease,
            PointerOutputKind::CoTaskMem => OutputCleanup::CoTaskMemFree,
            PointerOutputKind::Bstr => OutputCleanup::BstrFree,
            PointerOutputKind::None | PointerOutputKind::Unclassified => {
                self.abi.default_output_cleanup()
            }
        }
    }

    fn supports_direct_return(&self) -> bool {
        matches!(self.abi, ParameterType::Pointer)
            || matches!(
                &self.abi,
                ParameterType::WinRT(typ)
                    if matches!(
                        typ.kind(),
                        TypeKind::Bool
                            | TypeKind::I8
                            | TypeKind::U8
                            | TypeKind::I16
                            | TypeKind::U16
                            | TypeKind::Char16
                            | TypeKind::I32
                            | TypeKind::U32
                            | TypeKind::I64
                            | TypeKind::U64
                            | TypeKind::F32
                            | TypeKind::F64
                            | TypeKind::HResult
                            | TypeKind::Enum(_)
                    )
            )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComParameterDirection {
    In,
    Out,
    OptionalOut,
    InOut,
    OutFill,
    InputBuffer,
    CallerOutputBuffer,
    CalleeAllocatedBuffer,
}

impl ComParameterDirection {
    const fn is_input(self) -> bool {
        matches!(
            self,
            Self::In
                | Self::OptionalOut
                | Self::InOut
                | Self::OutFill
                | Self::InputBuffer
                | Self::CallerOutputBuffer
        )
    }

    const fn is_output(self) -> bool {
        matches!(
            self,
            Self::Out
                | Self::OptionalOut
                | Self::InOut
                | Self::OutFill
                | Self::CallerOutputBuffer
                | Self::CalleeAllocatedBuffer
        )
    }

    const fn native_kind(self) -> ParamKind {
        match self {
            Self::In | Self::InputBuffer | Self::CallerOutputBuffer => ParamKind::In,
            Self::Out | Self::CalleeAllocatedBuffer => ParamKind::Out,
            Self::OptionalOut => ParamKind::OptionalOut,
            Self::InOut => ParamKind::InOut,
            Self::OutFill => ParamKind::OutFillArray,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComArgumentStorage {
    Value,
    OutputPointer,
    InOutPointer,
    FillBuffer,
    InputBuffer,
    CallerOutputBuffer,
    CalleeAllocatedBuffer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComBufferParamRole {
    InputCount { buffer_param: usize },
    InputActual { buffer_param: usize },
    CallerCapacity { buffer_param: usize },
    CallerActual { buffer_param: usize },
    CallerCapacityActual { buffer_param: usize },
    CalleeCount { buffer_param: usize },
}

impl ComBufferParamRole {
    const fn buffer_param(self) -> usize {
        match self {
            Self::InputCount { buffer_param }
            | Self::InputActual { buffer_param }
            | Self::CallerCapacity { buffer_param }
            | Self::CallerActual { buffer_param }
            | Self::CallerCapacityActual { buffer_param }
            | Self::CalleeCount { buffer_param } => buffer_param,
        }
    }

    const fn hides_input(self) -> bool {
        matches!(
            self,
            Self::InputCount { .. }
                | Self::CallerCapacity { .. }
                | Self::CallerCapacityActual { .. }
        )
    }

    const fn hides_output(self) -> bool {
        !matches!(self, Self::InputActual { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ComBufferRelation {
    Input {
        count_param: usize,
        actual_length_param: Option<usize>,
        unit: BufferCountUnit,
    },
    CallerCapacity {
        capacity_param: usize,
        actual_length_param: Option<usize>,
        unit: BufferCountUnit,
        two_call: bool,
    },
    EnumeratorNext {
        capacity_param: usize,
        fetched_param: usize,
    },
    CalleeAllocated {
        count_param: usize,
        unit: BufferCountUnit,
        allocator: BufferAllocator,
    },
}

impl ComBufferRelation {
    fn related_params(&self) -> impl Iterator<Item = usize> {
        let (first, second) = match self {
            Self::Input {
                count_param,
                actual_length_param,
                ..
            } => (*count_param, *actual_length_param),
            Self::CallerCapacity {
                capacity_param,
                actual_length_param,
                ..
            } => (*capacity_param, *actual_length_param),
            Self::EnumeratorNext {
                capacity_param,
                fetched_param,
            } => (*capacity_param, Some(*fetched_param)),
            Self::CalleeAllocated { count_param, .. } => (*count_param, None),
        };
        std::iter::once(first).chain(second)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ComBufferContract {
    element: BufferElementPlan,
    relation: ComBufferRelation,
}

#[derive(Debug, Clone)]
struct ComParameterSpec {
    direction: ComParameterDirection,
    typ: Type,
    buffer: Option<ComBufferContract>,
}

#[derive(Debug, Clone)]
struct ComArgumentPlan {
    typ: ParameterType,
    direction: ComParameterDirection,
    storage: ComArgumentStorage,
    input_index: Option<usize>,
    output_index: Option<usize>,
    failure_cleanup: OutputCleanup,
    buffer: Option<ComBufferContract>,
    buffer_roles: Vec<ComBufferParamRole>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComResultSource {
    DirectReturn,
    Parameter(usize),
    Buffer(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComSuccessDisposition {
    Value,
    BorrowedPointer,
    OwnedComPointer,
    OwnedCoTaskMemPointer,
    OwnedBstr,
}

impl From<PointerOutputKind> for ComSuccessDisposition {
    fn from(kind: PointerOutputKind) -> Self {
        match kind {
            PointerOutputKind::None => Self::Value,
            PointerOutputKind::Unclassified => Self::BorrowedPointer,
            PointerOutputKind::Com => Self::OwnedComPointer,
            PointerOutputKind::CoTaskMem => Self::OwnedCoTaskMemPointer,
            PointerOutputKind::Bstr => Self::OwnedBstr,
        }
    }
}

impl ComSuccessDisposition {
    const fn pointer_output_kind(self) -> PointerOutputKind {
        match self {
            Self::Value => PointerOutputKind::None,
            Self::BorrowedPointer => PointerOutputKind::Unclassified,
            Self::OwnedComPointer => PointerOutputKind::Com,
            Self::OwnedCoTaskMemPointer => PointerOutputKind::CoTaskMem,
            Self::OwnedBstr => PointerOutputKind::Bstr,
        }
    }
}

#[derive(Debug, Clone)]
struct ComResultPlan {
    source: ComResultSource,
    typ: Option<ParameterType>,
    success: ComSuccessDisposition,
    failure_cleanup: OutputCleanup,
}

#[derive(Debug)]
pub struct DispatchInvokeResult {
    hresult: windows_core::HRESULT,
    result: Option<VariantValue>,
    excep_info: Option<ExcepInfoValue>,
    arg_err: Option<u32>,
    finalization_error: Option<windows_core::Error>,
}

impl DispatchInvokeResult {
    pub fn hresult(&self) -> windows_core::HRESULT {
        self.hresult
    }

    pub fn result(&self) -> Option<&VariantValue> {
        self.result.as_ref()
    }

    pub fn excep_info(&self) -> Option<&ExcepInfoValue> {
        self.excep_info.as_ref()
    }

    pub fn arg_err(&self) -> Option<u32> {
        self.arg_err
    }

    pub fn finalization_error(&self) -> Option<&windows_core::Error> {
        self.finalization_error.as_ref()
    }

    pub fn into_parts(
        self,
    ) -> (
        windows_core::HRESULT,
        Option<VariantValue>,
        Option<ExcepInfoValue>,
        Option<u32>,
        Option<windows_core::Error>,
    ) {
        (
            self.hresult,
            self.result,
            self.excep_info,
            self.arg_err,
            self.finalization_error,
        )
    }
}

#[derive(Debug, Clone)]
enum ComReturnPlan {
    HResult,
    SemanticHResult,
    EnumeratorNextHResult,
    DispatchInvokeHResult(CapturedHResultPlan),
    Void,
    Direct(Type),
}

#[derive(Debug)]
struct ComCallPlan {
    native: NativeMethod,
    arguments: Vec<ComArgumentPlan>,
    results: Vec<ComResultPlan>,
    return_plan: ComReturnPlan,
}

impl ComCallPlan {
    fn new(
        native: NativeMethod,
        parameters: Vec<ComParameterSpec>,
        return_plan: ComReturnPlan,
    ) -> Self {
        let mut buffer_roles = vec![Vec::new(); parameters.len()];
        for (buffer_param, parameter) in parameters.iter().enumerate() {
            let Some(buffer) = &parameter.buffer else {
                continue;
            };
            for related in buffer.relation.related_params() {
                assert!(
                    related < parameters.len() && related != buffer_param,
                    "counted COM buffer relationships must reference another parameter"
                );
            }
            match buffer.relation {
                ComBufferRelation::Input {
                    count_param,
                    actual_length_param,
                    ..
                } => {
                    set_buffer_role(
                        &mut buffer_roles,
                        count_param,
                        ComBufferParamRole::InputCount { buffer_param },
                    );
                    if let Some(actual) = actual_length_param {
                        set_buffer_role(
                            &mut buffer_roles,
                            actual,
                            ComBufferParamRole::InputActual { buffer_param },
                        );
                    }
                }
                ComBufferRelation::CallerCapacity {
                    capacity_param,
                    actual_length_param,
                    ..
                } => {
                    if actual_length_param == Some(capacity_param) {
                        set_buffer_role(
                            &mut buffer_roles,
                            capacity_param,
                            ComBufferParamRole::CallerCapacityActual { buffer_param },
                        );
                    } else {
                        set_buffer_role(
                            &mut buffer_roles,
                            capacity_param,
                            ComBufferParamRole::CallerCapacity { buffer_param },
                        );
                        if let Some(actual) = actual_length_param {
                            set_buffer_role(
                                &mut buffer_roles,
                                actual,
                                ComBufferParamRole::CallerActual { buffer_param },
                            );
                        }
                    }
                }
                ComBufferRelation::EnumeratorNext {
                    capacity_param,
                    fetched_param,
                } => {
                    set_buffer_role(
                        &mut buffer_roles,
                        capacity_param,
                        ComBufferParamRole::CallerCapacity { buffer_param },
                    );
                    set_buffer_role(
                        &mut buffer_roles,
                        fetched_param,
                        ComBufferParamRole::CallerActual { buffer_param },
                    );
                }
                ComBufferRelation::CalleeAllocated { count_param, .. } => {
                    set_buffer_role(
                        &mut buffer_roles,
                        count_param,
                        ComBufferParamRole::CalleeCount { buffer_param },
                    );
                }
            }
        }

        let mut input_index = 0;
        let mut output_index = 0;
        let mut arguments = Vec::with_capacity(parameters.len());
        let mut results = Vec::new();

        match &return_plan {
            ComReturnPlan::SemanticHResult | ComReturnPlan::EnumeratorNextHResult => {
                results.push(ComResultPlan {
                    source: ComResultSource::DirectReturn,
                    typ: None,
                    success: ComSuccessDisposition::Value,
                    failure_cleanup: OutputCleanup::None,
                })
            }
            ComReturnPlan::Direct(typ) => results.push(ComResultPlan {
                source: ComResultSource::DirectReturn,
                typ: Some(typ.abi.clone()),
                success: typ.pointer_output.into(),
                failure_cleanup: typ.output_cleanup(),
            }),
            ComReturnPlan::HResult
            | ComReturnPlan::DispatchInvokeHResult(_)
            | ComReturnPlan::Void => {}
        }

        for (parameter_index, parameter) in parameters.into_iter().enumerate() {
            let direction = parameter.direction;
            let typ = parameter.typ;
            let roles = &buffer_roles[parameter_index];
            let has_logical_input = direction.is_input() && !buffer_roles_hide_input(roles);
            let parameter_input_index = has_logical_input.then_some(input_index);
            if has_logical_input {
                input_index += 1;
            }
            let native_kind = direction.native_kind();
            let has_native_output = matches!(
                native_kind,
                ParamKind::Out
                    | ParamKind::OptionalOut
                    | ParamKind::InOut
                    | ParamKind::OutFillArray
            );
            let parameter_output_index = has_native_output.then_some(output_index);
            if has_native_output {
                output_index += 1;
            }
            let (pointer_output, failure_cleanup) = match direction {
                ComParameterDirection::Out | ComParameterDirection::OptionalOut => {
                    (typ.pointer_output, typ.output_cleanup())
                }
                ComParameterDirection::InOut => {
                    if typ.abi.is_bstr() {
                        (PointerOutputKind::Bstr, OutputCleanup::BstrFree)
                    } else {
                        (PointerOutputKind::Unclassified, OutputCleanup::None)
                    }
                }
                ComParameterDirection::CalleeAllocatedBuffer => {
                    (PointerOutputKind::None, typ.output_cleanup())
                }
                ComParameterDirection::OutFill
                | ComParameterDirection::In
                | ComParameterDirection::InputBuffer
                | ComParameterDirection::CallerOutputBuffer => {
                    (PointerOutputKind::None, OutputCleanup::None)
                }
            };
            let storage = match direction {
                ComParameterDirection::In => ComArgumentStorage::Value,
                ComParameterDirection::Out => ComArgumentStorage::OutputPointer,
                ComParameterDirection::OptionalOut => ComArgumentStorage::OutputPointer,
                ComParameterDirection::InOut => ComArgumentStorage::InOutPointer,
                ComParameterDirection::OutFill => ComArgumentStorage::FillBuffer,
                ComParameterDirection::InputBuffer => ComArgumentStorage::InputBuffer,
                ComParameterDirection::CallerOutputBuffer => ComArgumentStorage::CallerOutputBuffer,
                ComParameterDirection::CalleeAllocatedBuffer => {
                    ComArgumentStorage::CalleeAllocatedBuffer
                }
            };
            if matches!(
                direction,
                ComParameterDirection::CallerOutputBuffer
                    | ComParameterDirection::CalleeAllocatedBuffer
            ) {
                results.push(ComResultPlan {
                    source: ComResultSource::Buffer(parameter_index),
                    typ: None,
                    success: ComSuccessDisposition::Value,
                    failure_cleanup,
                });
            } else if direction.is_output() && !buffer_roles_hide_output(roles) {
                results.push(ComResultPlan {
                    source: ComResultSource::Parameter(parameter_index),
                    typ: Some(typ.abi.clone()),
                    success: pointer_output.into(),
                    failure_cleanup,
                });
            }
            arguments.push(ComArgumentPlan {
                typ: typ.abi,
                direction,
                storage,
                input_index: parameter_input_index,
                output_index: parameter_output_index,
                failure_cleanup,
                buffer: parameter.buffer,
                buffer_roles: buffer_roles[parameter_index].clone(),
            });
        }

        let plan = Self {
            native,
            arguments,
            results,
            return_plan,
        };
        plan.assert_invariants();
        plan
    }

    fn assert_invariants(&self) {
        let mut expected_input = 0;
        let mut expected_output = 0;
        for (parameter_index, argument) in self.arguments.iter().enumerate() {
            let has_logical_input =
                argument.direction.is_input() && !buffer_roles_hide_input(&argument.buffer_roles);
            assert_eq!(
                argument.input_index,
                has_logical_input.then_some(expected_input)
            );
            if has_logical_input {
                expected_input += 1;
            }
            let has_native_output = matches!(
                argument.direction.native_kind(),
                ParamKind::Out
                    | ParamKind::OptionalOut
                    | ParamKind::InOut
                    | ParamKind::OutFillArray
            );
            assert_eq!(
                argument.output_index,
                has_native_output.then_some(expected_output)
            );
            if has_native_output {
                expected_output += 1;
                assert_eq!(
                    self.native.output_cleanup(parameter_index),
                    argument.failure_cleanup
                );
            }
            assert_eq!(
                argument.storage,
                match argument.direction {
                    ComParameterDirection::In => ComArgumentStorage::Value,
                    ComParameterDirection::Out | ComParameterDirection::OptionalOut => {
                        ComArgumentStorage::OutputPointer
                    }
                    ComParameterDirection::InOut => ComArgumentStorage::InOutPointer,
                    ComParameterDirection::OutFill => ComArgumentStorage::FillBuffer,
                    ComParameterDirection::InputBuffer => ComArgumentStorage::InputBuffer,
                    ComParameterDirection::CallerOutputBuffer => {
                        ComArgumentStorage::CallerOutputBuffer
                    }
                    ComParameterDirection::CalleeAllocatedBuffer => {
                        ComArgumentStorage::CalleeAllocatedBuffer
                    }
                }
            );
            assert_eq!(&argument.typ, self.native.parameter_type(parameter_index));
        }
        for result in &self.results {
            if result.typ.is_none() {
                assert!(matches!(
                    result.source,
                    ComResultSource::DirectReturn | ComResultSource::Buffer(_)
                ));
            }
            if let ComResultSource::Parameter(index) | ComResultSource::Buffer(index) =
                result.source
            {
                assert_eq!(
                    result.failure_cleanup,
                    self.arguments[index].failure_cleanup
                );
            }
        }
        let planned_direct_type = self
            .results
            .first()
            .filter(|result| result.source == ComResultSource::DirectReturn)
            .and_then(|result| result.typ.as_ref());
        assert_eq!(planned_direct_type, self.native.direct_return_type());
        let has_direct_result = self
            .results
            .first()
            .is_some_and(|result| result.source == ComResultSource::DirectReturn);
        assert_eq!(
            has_direct_result,
            matches!(
                self.return_plan,
                ComReturnPlan::SemanticHResult
                    | ComReturnPlan::EnumeratorNextHResult
                    | ComReturnPlan::Direct(_)
            )
        );
    }

    fn invoke(&self, obj: *mut c_void, args: &[WinRTValue]) -> result::Result<Vec<WinRTValue>> {
        if self.native.uses_com_value_path()
            || self
                .arguments
                .iter()
                .any(|argument| argument.buffer.is_some())
            || self
                .results
                .iter()
                .any(|result| matches!(result.source, ComResultSource::Buffer(_)))
        {
            return Err(invalid_argument(
                "COM-local struct, union, Automation, or buffer signatures require the COM value invocation path",
            ));
        }
        let values = args.iter().cloned().map(Value::WinRt).collect::<Vec<_>>();
        self.invoke_values(obj, &values)?
            .into_iter()
            .map(|value| match value {
                Value::WinRt(value) => Ok(value),
                Value::NativeStruct(_) => Err(invalid_argument(
                    "native POD result requires the COM value invocation path",
                )),
                Value::NativeUnion(_)
                | Value::Bstr(_)
                | Value::Variant(_)
                | Value::SafeArray(_)
                | Value::PropVariant(_)
                | Value::DispatchParams(_)
                | Value::ExcepInfo(_) => Err(invalid_argument(
                    "COM-local result requires the COM value invocation path",
                )),
                Value::Buffer(_) => Err(invalid_argument(
                    "counted COM buffer result requires the COM value invocation path",
                )),
            })
            .collect()
    }

    fn invoke_values(&self, obj: *mut c_void, args: &[Value]) -> result::Result<Vec<Value>> {
        if matches!(self.return_plan, ComReturnPlan::DispatchInvokeHResult(_)) {
            return Err(invalid_argument(
                "IDispatch::Invoke captured HRESULT calls require invoke_dispatch()",
            ));
        }
        let expected_args = self
            .arguments
            .iter()
            .filter(|argument| argument.input_index.is_some())
            .count();
        if args.len() != expected_args {
            return Err(invalid_argument(format!(
                "COM call expects {expected_args} argument(s), received {}",
                args.len()
            )));
        }

        let mut prepared_buffers = (0..self.arguments.len()).map(|_| None).collect::<Vec<_>>();
        for (parameter_index, argument) in self.arguments.iter().enumerate() {
            if !matches!(
                argument.direction,
                ComParameterDirection::InputBuffer | ComParameterDirection::CallerOutputBuffer
            ) {
                continue;
            }
            let input_index = argument
                .input_index
                .expect("borrowed buffer is a logical input");
            let Value::Buffer(value) = &args[input_index] else {
                return Err(invalid_argument(format!(
                    "COM buffer parameter {parameter_index} requires DynCom.buffer() storage"
                )));
            };
            let contract = argument
                .buffer
                .as_ref()
                .expect("buffer parameter has a contract");
            let prepared = prepare_borrowed_buffer(
                value,
                &contract.element,
                argument.direction == ComParameterDirection::CallerOutputBuffer,
            )?;
            if argument.direction == ComParameterDirection::CallerOutputBuffer {
                prepared.initialize_output(&contract.element);
            }
            prepared_buffers[parameter_index] = Some(prepared);
        }

        for (buffer_param, argument) in self.arguments.iter().enumerate() {
            let Some(ComBufferContract {
                relation: ComBufferRelation::EnumeratorNext { fetched_param, .. },
                ..
            }) = &argument.buffer
            else {
                continue;
            };
            let buffer = prepared_buffers[buffer_param]
                .as_ref()
                .expect("enumerator output buffer prepared");
            let capacity = buffer_count(
                buffer.byte_len,
                &argument.buffer.as_ref().unwrap().element,
                BufferCountUnit::Elements,
            )?;
            if self.arguments[*fetched_param].direction == ComParameterDirection::OptionalOut {
                let input_index = self.arguments[*fetched_param]
                    .input_index
                    .expect("optional fetched output has a request argument");
                let requested = matches!(
                    args.get(input_index),
                    Some(Value::WinRt(WinRTValue::Bool(true)))
                );
                if !requested && capacity != 1 {
                    return Err(invalid_argument(
                        "IEnum::Next permits a null pceltFetched only when requested capacity is exactly one",
                    ));
                }
            }
        }

        let mut native_args = Vec::new();
        for (parameter_index, argument) in self.arguments.iter().enumerate() {
            if !matches!(
                argument.direction.native_kind(),
                ParamKind::In | ParamKind::OptionalOut | ParamKind::InOut
            ) {
                continue;
            }
            if matches!(
                argument.direction,
                ComParameterDirection::InputBuffer | ComParameterDirection::CallerOutputBuffer
            ) {
                let buffer = prepared_buffers[parameter_index]
                    .as_ref()
                    .expect("borrowed buffer prepared");
                native_args.push(Value::WinRt(WinRTValue::RawPtr(buffer.ptr.cast())));
                continue;
            }
            if buffer_roles_hide_input(&argument.buffer_roles) {
                let mut derived_count = None;
                for role in argument
                    .buffer_roles
                    .iter()
                    .copied()
                    .filter(|role| role.hides_input())
                {
                    let buffer_param = role.buffer_param();
                    let buffer = prepared_buffers[buffer_param]
                        .as_ref()
                        .expect("count source buffer prepared");
                    let contract = self.arguments[buffer_param]
                        .buffer
                        .as_ref()
                        .expect("buffer contract");
                    let count = buffer_count(
                        buffer.byte_len,
                        &contract.element,
                        relation_unit(&contract.relation),
                    )?;
                    if derived_count
                        .replace(count)
                        .is_some_and(|existing| existing != count)
                    {
                        return Err(invalid_argument(
                            "COM buffers sharing an authoritative count have different lengths",
                        ));
                    }
                }
                native_args.push(Value::WinRt(count_value(
                    &argument.typ,
                    derived_count.expect("hidden count has a source buffer"),
                )?));
                continue;
            }
            native_args.push(args[argument.input_index.expect("visible native input")].clone());
        }

        let native_result = if self.native.uses_com_value_path()
            || native_args
                .iter()
                .any(|value| !matches!(value, Value::WinRt(_) | Value::Buffer(_)))
        {
            self.native
                .call_com_dynamic(obj, &native_args)
                .map_err(result::Error::WindowsError)
        } else {
            let winrt_args = native_args
                .iter()
                .map(|value| match value {
                    Value::WinRt(value) => Ok(value.clone()),
                    Value::NativeStruct(_) => Err(invalid_argument(
                        "native POD value passed to a non-struct COM method",
                    )),
                    Value::NativeUnion(_)
                    | Value::Bstr(_)
                    | Value::Variant(_)
                    | Value::SafeArray(_)
                    | Value::PropVariant(_)
                    | Value::DispatchParams(_)
                    | Value::ExcepInfo(_) => Err(invalid_argument(
                        "COM-local value passed to a scalar COM method",
                    )),
                    Value::Buffer(_) => Err(invalid_argument(
                        "COM buffer reached the private native-call backend",
                    )),
                })
                .collect::<result::Result<Vec<_>>>()?;
            self.native
                .call_dynamic(obj, &winrt_args)
                .map(|values| values.into_iter().map(Value::WinRt).collect())
                .map_err(result::Error::WindowsError)
        };
        let native_values = match native_result {
            Ok(values) => values,
            Err(error) => {
                cleanup_prepared_owning_outputs(&self.arguments, &prepared_buffers);
                return Err(error);
            }
        };

        let direct_offset = usize::from(matches!(
            self.return_plan,
            ComReturnPlan::SemanticHResult
                | ComReturnPlan::EnumeratorNextHResult
                | ComReturnPlan::Direct(_)
        ));
        let enumerator_hresult = matches!(self.return_plan, ComReturnPlan::EnumeratorNextHResult)
            .then(|| match native_values.first() {
                Some(Value::WinRt(WinRTValue::HResult(value))) => Ok(*value),
                _ => Err(invalid_argument(
                    "IEnum::Next native call did not preserve its HRESULT",
                )),
            })
            .transpose()?;
        let native_param_result = |parameter_index: usize| -> result::Result<&Value> {
            let output_index = self.arguments[parameter_index]
                .output_index
                .ok_or_else(|| {
                    invalid_argument("COM buffer relation does not reference an output")
                })?;
            native_values
                .get(direct_offset + output_index)
                .ok_or_else(|| invalid_argument("native COM output result is missing"))
        };

        let mut guarded_buffer_allocations = BTreeSet::new();
        if let Some(hresult) = enumerator_hresult.filter(|value| value.is_err()) {
            for (parameter_index, argument) in self.arguments.iter().enumerate() {
                let Some(contract) = &argument.buffer else {
                    continue;
                };
                if !matches!(contract.relation, ComBufferRelation::EnumeratorNext { .. })
                    || contract.element.cleanup == BufferElementCleanup::None
                {
                    continue;
                }
                let buffer = prepared_buffers[parameter_index]
                    .as_ref()
                    .expect("enumerator output buffer prepared");
                let capacity = buffer_count(
                    buffer.byte_len,
                    &contract.element,
                    BufferCountUnit::Elements,
                )?;
                buffer.cleanup_slots(&contract.element, 0, capacity);
            }
            self.cleanup_post_call_outputs(
                &native_values,
                direct_offset,
                &guarded_buffer_allocations,
            );
            return Err(result::Error::WindowsError(
                hresult
                    .ok()
                    .expect_err("failed IEnum::Next HRESULT must produce an error"),
            ));
        }
        let processed = (|| -> result::Result<Vec<Value>> {
            let mut values = Vec::with_capacity(self.results.len());
            for result_plan in &self.results {
                let value = match result_plan.source {
                    ComResultSource::DirectReturn => native_values
                        .first()
                        .cloned()
                        .ok_or_else(|| invalid_argument("native COM direct result is missing"))?,
                    ComResultSource::Parameter(parameter_index) => {
                        native_param_result(parameter_index)?.clone()
                    }
                    ComResultSource::Buffer(parameter_index) => {
                        let argument = &self.arguments[parameter_index];
                        let contract = argument.buffer.as_ref().expect("buffer result contract");
                        match &contract.relation {
                            ComBufferRelation::CallerCapacity {
                                actual_length_param,
                                unit,
                                two_call,
                                ..
                            } => {
                                let buffer = prepared_buffers[parameter_index]
                                    .as_ref()
                                    .expect("caller output buffer prepared");
                                let capacity =
                                    buffer_count(buffer.byte_len, &contract.element, *unit)?;
                                let actual = actual_length_param
                                    .map(|index| count_from_value(native_param_result(index)?))
                                    .transpose()?
                                    .unwrap_or(capacity);
                                if actual > capacity && !*two_call {
                                    return Err(invalid_argument(format!(
                                        "COM buffer actual length {actual} exceeds capacity {capacity}"
                                    )));
                                }
                                if contract.element.cleanup != BufferElementCleanup::None {
                                    if actual > capacity {
                                        return Err(invalid_argument(format!(
                                            "COM owning array actual length {actual} exceeds capacity {capacity}"
                                        )));
                                    }
                                    Value::Buffer(buffer.take_owned_slots(
                                        &contract.element,
                                        actual,
                                        capacity,
                                    )?)
                                } else {
                                    let copy_count = actual.min(capacity);
                                    let copy_bytes =
                                        count_bytes(copy_count, &contract.element, *unit)?;
                                    let bytes = if copy_bytes == 0 {
                                        Vec::new()
                                    } else {
                                        unsafe {
                                            std::slice::from_raw_parts(buffer.ptr, copy_bytes)
                                                .to_vec()
                                        }
                                    };
                                    Value::Buffer(ComBufferValue::owned(bytes, actual))
                                }
                            }
                            ComBufferRelation::EnumeratorNext { fetched_param, .. } => {
                                let buffer = prepared_buffers[parameter_index]
                                    .as_ref()
                                    .expect("enumerator output buffer prepared");
                                let capacity = buffer_count(
                                    buffer.byte_len,
                                    &contract.element,
                                    BufferCountUnit::Elements,
                                )?;
                                let fetched_argument = &self.arguments[*fetched_param];
                                let fetched_requested = fetched_argument.direction
                                    != ComParameterDirection::OptionalOut
                                    || matches!(
                                        args.get(
                                            fetched_argument
                                                .input_index
                                                .expect("optional fetched request input")
                                        ),
                                        Some(Value::WinRt(WinRTValue::Bool(true)))
                                    );
                                let actual = if fetched_requested {
                                    count_from_value(native_param_result(*fetched_param)?)?
                                } else {
                                    match enumerator_hresult
                                        .expect("enumerator result has an HRESULT")
                                        .0
                                    {
                                        0 => 1,
                                        1 => 0,
                                        value => {
                                            if contract.element.cleanup
                                                != BufferElementCleanup::None
                                            {
                                                buffer.cleanup_slots(
                                                    &contract.element,
                                                    0,
                                                    capacity,
                                                );
                                            }
                                            return Err(invalid_argument(format!(
                                                "IEnum::Next returned unexpected success HRESULT 0x{:08X} without pceltFetched",
                                                value as u32
                                            )));
                                        }
                                    }
                                };
                                if actual > capacity {
                                    if contract.element.cleanup != BufferElementCleanup::None {
                                        buffer.cleanup_slots(&contract.element, 0, capacity);
                                    }
                                    return Err(invalid_argument(format!(
                                        "IEnum::Next fetched count {actual} exceeds requested capacity {capacity}"
                                    )));
                                }
                                match contract.element.cleanup {
                                    BufferElementCleanup::None => {
                                        let copy_bytes = count_bytes(
                                            actual,
                                            &contract.element,
                                            BufferCountUnit::Elements,
                                        )?;
                                        let bytes = if copy_bytes == 0 {
                                            Vec::new()
                                        } else {
                                            unsafe {
                                                std::slice::from_raw_parts(buffer.ptr, copy_bytes)
                                                    .to_vec()
                                            }
                                        };
                                        Value::Buffer(ComBufferValue::owned(bytes, actual))
                                    }
                                    BufferElementCleanup::ComRelease
                                    | BufferElementCleanup::BstrFree
                                    | BufferElementCleanup::VariantClear
                                    | BufferElementCleanup::CoTaskMemFree => {
                                        Value::Buffer(buffer.take_owned_slots(
                                            &contract.element,
                                            actual,
                                            capacity,
                                        )?)
                                    }
                                }
                            }
                            ComBufferRelation::CalleeAllocated {
                                count_param,
                                unit,
                                allocator,
                            } => {
                                let ptr =
                                    pointer_from_value(native_param_result(parameter_index)?)?;
                                guarded_buffer_allocations.insert(parameter_index);
                                let mut allocation = BufferAllocationGuard::new(ptr, *allocator);
                                let count = count_from_value(native_param_result(*count_param)?)?;
                                let bytes = count_bytes(count, &contract.element, *unit)?;
                                if bytes > 0 && ptr.is_null() {
                                    return Err(invalid_argument(
                                        "callee returned a null buffer with a non-zero count",
                                    ));
                                }
                                let copied = if bytes == 0 {
                                    Vec::new()
                                } else {
                                    unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), bytes) }
                                        .to_vec()
                                };
                                allocation.free();
                                Value::Buffer(ComBufferValue::owned(copied, count))
                            }
                            ComBufferRelation::Input { .. } => {
                                return Err(invalid_argument(
                                    "input-only COM buffer cannot produce a buffer result",
                                ));
                            }
                        }
                    }
                };
                values.push(value);
            }
            Ok(values)
        })();
        if processed.is_err() {
            cleanup_prepared_owning_outputs(&self.arguments, &prepared_buffers);
            self.cleanup_post_call_outputs(
                &native_values,
                direct_offset,
                &guarded_buffer_allocations,
            );
        }
        processed
    }

    fn invoke_dispatch(
        &self,
        obj: *mut c_void,
        args: &[Value],
    ) -> result::Result<DispatchInvokeResult> {
        let ComReturnPlan::DispatchInvokeHResult(plan) = &self.return_plan else {
            return Err(invalid_argument(
                "method does not use the IDispatch::Invoke captured HRESULT convention",
            ));
        };
        if self
            .arguments
            .iter()
            .any(|argument| argument.buffer.is_some())
        {
            return Err(invalid_argument(
                "IDispatch::Invoke captured HRESULT calls cannot use buffer plans",
            ));
        }

        let captured = self
            .native
            .call_com_dynamic_captured(obj, args)
            .map_err(result::Error::WindowsError)?;
        let mut outputs = captured.outputs;
        let result = match outputs[plan.result_output_index].take() {
            Some(crate::native_call::NativeCallValue::Variant(value)) => Some(value),
            None => None,
            Some(_) => {
                return Err(invalid_argument(
                    "IDispatch::Invoke result output was not a VARIANT",
                ));
            }
        };
        let excep_info = match outputs[plan.excep_info_output_index].take() {
            Some(crate::native_call::NativeCallValue::ExcepInfo(value)) => Some(value),
            None => None,
            Some(_) => {
                return Err(invalid_argument(
                    "IDispatch::Invoke exception output was not EXCEPINFO",
                ));
            }
        };
        let arg_err = match outputs[plan.arg_err_output_index].take() {
            Some(crate::native_call::NativeCallValue::WinRt(WinRTValue::U32(value))) => Some(value),
            None => None,
            Some(_) => {
                return Err(invalid_argument(
                    "IDispatch::Invoke argument error output was not UINT",
                ));
            }
        };
        if outputs.into_iter().any(|value| value.is_some()) {
            return Err(invalid_argument(
                "IDispatch::Invoke captured an unexpected native output",
            ));
        }

        Ok(DispatchInvokeResult {
            hresult: captured.hresult,
            result,
            excep_info,
            arg_err,
            finalization_error: captured.finalization_error,
        })
    }

    fn cleanup_post_call_outputs(
        &self,
        native_values: &[Value],
        direct_offset: usize,
        guarded_parameters: &BTreeSet<usize>,
    ) {
        let mut cleaned = BTreeSet::new();
        let mut direct_cleaned = false;
        for result in &self.results {
            let parameter_index = match result.source {
                ComResultSource::Parameter(index) | ComResultSource::Buffer(index) => index,
                ComResultSource::DirectReturn => {
                    if direct_cleaned || result.failure_cleanup == OutputCleanup::None {
                        continue;
                    }
                    direct_cleaned = true;
                    let Some(value) = native_values.first() else {
                        continue;
                    };
                    let Ok(ptr) = pointer_from_value(value) else {
                        continue;
                    };
                    unsafe { result.failure_cleanup.cleanup(ptr) };
                    continue;
                }
            };
            if guarded_parameters.contains(&parameter_index) || !cleaned.insert(parameter_index) {
                continue;
            }
            let cleanup = self.arguments[parameter_index].failure_cleanup;
            if cleanup == OutputCleanup::None {
                continue;
            }
            let Some(output_index) = self.arguments[parameter_index].output_index else {
                continue;
            };
            let Some(value) = native_values.get(direct_offset + output_index) else {
                continue;
            };
            let Ok(ptr) = pointer_from_value(value) else {
                continue;
            };
            unsafe { cleanup.cleanup(ptr) };
        }
    }

    fn invoke_with_output_kinds(
        &self,
        obj: *mut c_void,
        args: &[WinRTValue],
    ) -> result::Result<Vec<(WinRTValue, PointerOutputKind)>> {
        let values = self.invoke(obj, args)?;
        if values.len() != self.results.len() {
            return Err(invalid_argument(format!(
                "COM result plan mismatch: native call returned {} value(s), plan describes {}",
                values.len(),
                self.results.len()
            )));
        }
        Ok(values
            .into_iter()
            .zip(
                self.results
                    .iter()
                    .map(|result| result.success.pointer_output_kind()),
            )
            .collect())
    }

    fn invoke_values_with_output_kinds(
        &self,
        obj: *mut c_void,
        args: &[Value],
    ) -> result::Result<Vec<(Value, PointerOutputKind)>> {
        let values = self.invoke_values(obj, args)?;
        if values.len() != self.results.len() {
            return Err(invalid_argument(format!(
                "COM result plan mismatch: native call returned {} value(s), plan describes {}",
                values.len(),
                self.results.len()
            )));
        }
        Ok(values
            .into_iter()
            .zip(
                self.results
                    .iter()
                    .map(|result| result.success.pointer_output_kind()),
            )
            .collect())
    }
}

fn cleanup_prepared_owning_outputs(
    arguments: &[ComArgumentPlan],
    prepared_buffers: &[Option<PreparedBuffer<'_>>],
) {
    for (index, argument) in arguments.iter().enumerate() {
        let Some(contract) = &argument.buffer else {
            continue;
        };
        if argument.direction != ComParameterDirection::CallerOutputBuffer
            || contract.element.cleanup == BufferElementCleanup::None
        {
            continue;
        }
        let Some(buffer) = prepared_buffers[index].as_ref() else {
            continue;
        };
        let Ok(capacity) = buffer_count(
            buffer.byte_len,
            &contract.element,
            relation_unit(&contract.relation),
        ) else {
            continue;
        };
        buffer.cleanup_slots(&contract.element, 0, capacity);
    }
}

fn set_buffer_role(roles: &mut [Vec<ComBufferParamRole>], index: usize, role: ComBufferParamRole) {
    roles[index].push(role);
}

fn buffer_roles_hide_input(roles: &[ComBufferParamRole]) -> bool {
    roles.iter().any(|role| role.hides_input())
}

fn buffer_roles_hide_output(roles: &[ComBufferParamRole]) -> bool {
    !roles.is_empty() && roles.iter().all(|role| role.hides_output())
}

#[derive(Debug)]
struct PreparedBuffer<'a> {
    ptr: *mut u8,
    byte_len: usize,
    element_kind: BufferElementKind,
    _owned_input: Option<PreparedOwnedInput>,
    _caller_output_guard: Option<MutexGuard<'a, Vec<AlignedBufferBlock>>>,
}

enum PreparedOwnedInput {
    Bstr(Vec<windows_core::BSTR>),
    Variant(crate::com::automation::VariantArrayCopyValue),
}

impl std::fmt::Debug for PreparedOwnedInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Bstr(_) => "PreparedOwnedInput::Bstr",
            Self::Variant(_) => "PreparedOwnedInput::Variant",
        })
    }
}

impl PreparedBuffer<'_> {
    fn initialize_output(&self, element: &BufferElementPlan) {
        debug_assert_eq!(self.element_kind, element.kind);
        if self.byte_len == 0 {
            return;
        }
        match element.cleanup {
            BufferElementCleanup::VariantClear => {
                for index in 0..(self.byte_len / element.size) {
                    unsafe {
                        crate::com::automation::initialize_variant_slot(
                            self.ptr.add(index * element.size).cast(),
                        )
                    };
                }
            }
            BufferElementCleanup::None
            | BufferElementCleanup::ComRelease
            | BufferElementCleanup::BstrFree
            | BufferElementCleanup::CoTaskMemFree => unsafe {
                std::ptr::write_bytes(self.ptr, 0, self.byte_len)
            },
        }
    }

    fn cleanup_slots(&self, element: &BufferElementPlan, start: usize, end: usize) {
        for index in start..end {
            let slot = unsafe { self.ptr.add(index * element.size) };
            match element.cleanup {
                BufferElementCleanup::None => {}
                BufferElementCleanup::ComRelease
                | BufferElementCleanup::BstrFree
                | BufferElementCleanup::CoTaskMemFree => {
                    let slot = slot.cast::<*mut c_void>();
                    let value = unsafe { slot.read() };
                    if !value.is_null() {
                        unsafe {
                            match element.cleanup {
                                BufferElementCleanup::ComRelease => {
                                    OutputCleanup::ComRelease.cleanup(value)
                                }
                                BufferElementCleanup::BstrFree => {
                                    OutputCleanup::BstrFree.cleanup(value)
                                }
                                BufferElementCleanup::CoTaskMemFree => {
                                    OutputCleanup::CoTaskMemFree.cleanup(value)
                                }
                                BufferElementCleanup::None | BufferElementCleanup::VariantClear => {
                                    unreachable!()
                                }
                            }
                        };
                        unsafe { slot.write(std::ptr::null_mut()) };
                    }
                }
                BufferElementCleanup::VariantClear => unsafe {
                    crate::com::automation::clear_variant_slot(slot.cast());
                    crate::com::automation::initialize_variant_slot(slot.cast());
                },
            }
        }
    }

    fn take_owned_slots(
        &self,
        element: &BufferElementPlan,
        actual: usize,
        capacity: usize,
    ) -> result::Result<ComBufferValue> {
        match element.cleanup {
            BufferElementCleanup::ComRelease => self
                .take_com_slots(element, actual, capacity)
                .map(ComBufferValue::owned_com),
            BufferElementCleanup::BstrFree => self
                .take_bstr_slots(element, actual, capacity)
                .map(ComBufferValue::owned_strings),
            BufferElementCleanup::VariantClear => self
                .take_variant_slots(element, actual, capacity)
                .map(ComBufferValue::owned_variants),
            BufferElementCleanup::CoTaskMemFree => self
                .take_wide_string_slots(element, actual, capacity)
                .map(ComBufferValue::owned_strings),
            BufferElementCleanup::None => Err(invalid_argument(
                "plain COM buffers do not use owning element transfer",
            )),
        }
    }

    fn take_com_slots(
        &self,
        element: &BufferElementPlan,
        fetched: usize,
        capacity: usize,
    ) -> result::Result<Vec<WinRTValue>> {
        let mut values = Vec::with_capacity(fetched);
        for index in 0..fetched {
            let slot = unsafe { self.ptr.add(index * element.size).cast::<*mut c_void>() };
            let value = unsafe { slot.read() };
            if value.is_null() {
                self.cleanup_slots(element, index, capacity);
                return Err(invalid_argument(
                    "COM array returned a null interface pointer within the initialized range",
                ));
            }
            unsafe { slot.write(std::ptr::null_mut()) };
            values.push(WinRTValue::Object(unsafe { IUnknown::from_raw(value) }));
        }
        self.cleanup_slots(element, fetched, capacity);
        Ok(values)
    }

    fn take_bstr_slots(
        &self,
        element: &BufferElementPlan,
        actual: usize,
        capacity: usize,
    ) -> result::Result<Vec<String>> {
        let mut values = Vec::with_capacity(actual);
        for index in 0..actual {
            let slot = unsafe { self.ptr.add(index * element.size).cast::<*mut u16>() };
            let raw = unsafe { slot.read() };
            if raw.is_null() {
                values.push(String::new());
                continue;
            }
            let value = unsafe { windows_core::BSTR::from_raw(raw.cast_const()) };
            values.push(value.to_string());
            unsafe { slot.write(std::ptr::null_mut()) };
        }
        self.cleanup_slots(element, actual, capacity);
        Ok(values)
    }

    fn take_variant_slots(
        &self,
        element: &BufferElementPlan,
        actual: usize,
        capacity: usize,
    ) -> result::Result<Vec<VariantValue>> {
        for index in 0..actual {
            let slot = unsafe { self.ptr.add(index * element.size) };
            if let Err(error) =
                unsafe { crate::com::automation::validate_variant_slot(slot.cast()) }
            {
                self.cleanup_slots(element, 0, capacity);
                return Err(error);
            }
        }
        let mut values = Vec::with_capacity(actual);
        for index in 0..actual {
            let slot = unsafe { self.ptr.add(index * element.size) };
            values.push(unsafe { crate::com::automation::take_variant_slot(slot.cast()) }?);
        }
        self.cleanup_slots(element, actual, capacity);
        Ok(values)
    }

    fn take_wide_string_slots(
        &self,
        element: &BufferElementPlan,
        actual: usize,
        capacity: usize,
    ) -> result::Result<Vec<String>> {
        let mut values = Vec::with_capacity(actual);
        for index in 0..actual {
            let slot = unsafe { self.ptr.add(index * element.size).cast::<*mut u16>() };
            let raw = unsafe { slot.read() };
            if raw.is_null() {
                self.cleanup_slots(element, 0, capacity);
                return Err(invalid_argument(
                    "COM string array returned a null pointer within the initialized range",
                ));
            }
            let value = unsafe { windows_core::PWSTR(raw).to_string() }
                .map_err(|error| invalid_argument(format!("invalid UTF-16 COM string: {error}")));
            match value {
                Ok(value) => values.push(value),
                Err(error) => {
                    self.cleanup_slots(element, 0, capacity);
                    return Err(error);
                }
            }
        }
        self.cleanup_slots(element, 0, capacity);
        Ok(values)
    }
}

fn prepare_borrowed_buffer<'a>(
    value: &'a ComBufferValue,
    element: &BufferElementPlan,
    require_writable: bool,
) -> result::Result<PreparedBuffer<'a>> {
    let mut caller_output_guard = None;
    let mut owned_input = None;
    let (
        ptr,
        byte_len,
        source_element_size,
        raw_bytes,
        writable,
        native_layout_name,
        string_encoding,
        source_element_kind,
    ) = match &value.storage {
        ComBufferStorage::CallerOutput {
            blocks,
            byte_len,
            source_element_size,
            native_layout_name,
            element_kind,
        } => {
            let mut guard = blocks.try_lock().map_err(|error| match error {
                TryLockError::WouldBlock => invalid_argument(
                    "caller-output COM storage cannot be aliased or used concurrently",
                ),
                TryLockError::Poisoned(_) => {
                    invalid_argument("caller-output COM storage lock is poisoned")
                }
            })?;
            let ptr = guard.as_mut_ptr().cast::<u8>();
            caller_output_guard = Some(guard);
            (
                ptr,
                *byte_len,
                *source_element_size,
                false,
                true,
                native_layout_name.as_deref(),
                None,
                Some(*element_kind),
            )
        }
        ComBufferStorage::InterfaceArray { iid, pointers, .. } => (
            pointers.as_ptr().cast_mut().cast(),
            pointers.len() * size_of::<*mut c_void>(),
            size_of::<*mut c_void>(),
            false,
            false,
            None,
            None,
            Some(BufferElementKind::ComInterface(*iid)),
        ),
        ComBufferStorage::BstrArray { values } => {
            let mut allocated = Vec::with_capacity(values.len());
            for value in values {
                let utf16 = value.encode_utf16().collect::<Vec<_>>();
                let bstr = unsafe { windows::Win32::Foundation::SysAllocStringLen(Some(&utf16)) };
                if bstr.is_empty() && !utf16.is_empty() {
                    return Err(result::Error::WindowsError(
                        windows_core::Error::from_hresult(windows_core::HRESULT(
                            0x8007000Eu32 as i32,
                        )),
                    ));
                }
                allocated.push(bstr);
            }
            owned_input = Some(PreparedOwnedInput::Bstr(allocated));
            let PreparedOwnedInput::Bstr(values) =
                owned_input.as_mut().expect("BSTR input storage")
            else {
                unreachable!()
            };
            (
                values.as_mut_ptr().cast(),
                values.len() * size_of::<*mut c_void>(),
                size_of::<*mut c_void>(),
                false,
                false,
                None,
                None,
                Some(BufferElementKind::Bstr),
            )
        }
        ComBufferStorage::VariantArray { values } => {
            owned_input = Some(PreparedOwnedInput::Variant(
                crate::com::automation::VariantArrayCopyValue::new(values)?,
            ));
            let PreparedOwnedInput::Variant(values) =
                owned_input.as_mut().expect("VARIANT input storage")
            else {
                unreachable!()
            };
            (
                values.as_mut_ptr().cast(),
                values.len() * crate::com::automation::variant_size(),
                crate::com::automation::variant_size(),
                false,
                false,
                None,
                None,
                Some(BufferElementKind::Variant),
            )
        }
        _ => {
            let parts = value.borrowed_parts()?;
            let source_element_kind = match parts.6 {
                Some(encoding) => Some(BufferElementKind::StringPointer(encoding)),
                None => Some(BufferElementKind::Plain),
            };
            (
                parts.0,
                parts.1,
                parts.2,
                parts.3,
                parts.4,
                parts.5,
                parts.6,
                source_element_kind,
            )
        }
    };
    if require_writable && !writable {
        return Err(invalid_argument(
            "caller-owned COM output buffers require writable backing storage",
        ));
    }
    let _ = string_encoding;
    if source_element_kind != Some(element.kind) {
        return Err(invalid_argument(
            "COM caller-output storage element contract does not match the method",
        ));
    }
    if !raw_bytes && source_element_size != element.size {
        return Err(invalid_argument(format!(
            "COM typed buffer element width mismatch: expected {}, received {}",
            element.size, source_element_size
        )));
    }
    if native_layout_name != element.native_layout_name.as_deref() {
        return Err(invalid_argument(
            "COM native struct buffer element layout identity mismatch",
        ));
    }
    if byte_len % element.size != 0 {
        return Err(invalid_argument(format!(
            "COM buffer byte length {byte_len} is not a multiple of element width {}",
            element.size
        )));
    }
    if byte_len > 0 && ptr as usize % element.alignment != 0 {
        return Err(invalid_argument(format!(
            "COM buffer backing address is not aligned to {} bytes",
            element.alignment
        )));
    }
    Ok(PreparedBuffer {
        ptr,
        byte_len,
        element_kind: element.kind,
        _owned_input: owned_input,
        _caller_output_guard: caller_output_guard,
    })
}

fn relation_unit(relation: &ComBufferRelation) -> BufferCountUnit {
    match relation {
        ComBufferRelation::Input { unit, .. }
        | ComBufferRelation::CallerCapacity { unit, .. }
        | ComBufferRelation::CalleeAllocated { unit, .. } => *unit,
        ComBufferRelation::EnumeratorNext { .. } => BufferCountUnit::Elements,
    }
}

fn buffer_count(
    byte_len: usize,
    element: &BufferElementPlan,
    unit: BufferCountUnit,
) -> result::Result<usize> {
    match unit {
        BufferCountUnit::Elements => {
            if byte_len % element.size != 0 {
                return Err(invalid_argument(
                    "COM buffer length does not contain a whole number of elements",
                ));
            }
            Ok(byte_len / element.size)
        }
        BufferCountUnit::Bytes => Ok(byte_len),
    }
}

fn count_bytes(
    count: usize,
    element: &BufferElementPlan,
    unit: BufferCountUnit,
) -> result::Result<usize> {
    let bytes = match unit {
        BufferCountUnit::Elements => count
            .checked_mul(element.size)
            .ok_or_else(|| invalid_argument("COM buffer byte length overflow")),
        BufferCountUnit::Bytes => {
            if element.size != 1 {
                return Err(invalid_argument(
                    "byte-counted COM buffers currently require one-byte elements",
                ));
            }
            Ok(count)
        }
    }?;
    const MAX_PROJECTED_BUFFER_BYTES: usize = i32::MAX as usize;
    if bytes > isize::MAX as usize || bytes > MAX_PROJECTED_BUFFER_BYTES {
        return Err(invalid_argument(
            "COM buffer byte length exceeds the supported projected Buffer size",
        ));
    }
    Ok(bytes)
}

fn count_value(typ: &ParameterType, value: usize) -> result::Result<WinRTValue> {
    let ParameterType::WinRT(typ) = typ else {
        return Err(invalid_argument(
            "COM buffer count parameters must use integer scalar ABI types",
        ));
    };
    match typ.kind() {
        TypeKind::I8 => i8::try_from(value)
            .map(WinRTValue::I8)
            .map_err(|_| invalid_argument("COM buffer count does not fit i8")),
        TypeKind::U8 => u8::try_from(value)
            .map(WinRTValue::U8)
            .map_err(|_| invalid_argument("COM buffer count does not fit u8")),
        TypeKind::I16 => i16::try_from(value)
            .map(WinRTValue::I16)
            .map_err(|_| invalid_argument("COM buffer count does not fit i16")),
        TypeKind::U16 | TypeKind::Char16 => u16::try_from(value)
            .map(WinRTValue::U16)
            .map_err(|_| invalid_argument("COM buffer count does not fit u16")),
        TypeKind::I32 => i32::try_from(value)
            .map(WinRTValue::I32)
            .map_err(|_| invalid_argument("COM buffer count does not fit i32")),
        TypeKind::U32 => u32::try_from(value)
            .map(WinRTValue::U32)
            .map_err(|_| invalid_argument("COM buffer count does not fit u32")),
        TypeKind::I64 => i64::try_from(value)
            .map(WinRTValue::I64)
            .map_err(|_| invalid_argument("COM buffer count does not fit i64")),
        TypeKind::U64 => u64::try_from(value)
            .map(WinRTValue::U64)
            .map_err(|_| invalid_argument("COM buffer count does not fit u64")),
        _ => Err(invalid_argument(
            "COM buffer count parameters must use an integer scalar ABI type",
        )),
    }
}

fn count_from_value(value: &Value) -> result::Result<usize> {
    let Value::WinRt(value) = value else {
        return Err(invalid_argument(
            "COM buffer count output must use an integer scalar ABI type",
        ));
    };
    let value = match value {
        WinRTValue::I8(value) => usize::try_from(*value)
            .map_err(|_| invalid_argument("COM buffer count cannot be negative"))?,
        WinRTValue::U8(value) => usize::from(*value),
        WinRTValue::I16(value) => usize::try_from(*value)
            .map_err(|_| invalid_argument("COM buffer count cannot be negative"))?,
        WinRTValue::U16(value) => usize::from(*value),
        WinRTValue::I32(value) => usize::try_from(*value)
            .map_err(|_| invalid_argument("COM buffer count cannot be negative"))?,
        WinRTValue::U32(value) => usize::try_from(*value)
            .map_err(|_| invalid_argument("COM buffer count does not fit usize"))?,
        WinRTValue::I64(value) => usize::try_from(*value)
            .map_err(|_| invalid_argument("COM buffer count cannot be negative or exceed usize"))?,
        WinRTValue::U64(value) => usize::try_from(*value)
            .map_err(|_| invalid_argument("COM buffer count does not fit usize"))?,
        _ => {
            return Err(invalid_argument(
                "COM buffer count output must use an integer scalar ABI type",
            ));
        }
    };
    Ok(value)
}

fn pointer_from_value(value: &Value) -> result::Result<*mut c_void> {
    match value {
        Value::WinRt(WinRTValue::RawPtr(ptr)) => Ok(*ptr),
        Value::WinRt(WinRTValue::Null) => Ok(std::ptr::null_mut()),
        _ => Err(invalid_argument(
            "callee-allocated COM buffer output did not return a native pointer",
        )),
    }
}

struct BufferAllocationGuard {
    ptr: *mut c_void,
    allocator: BufferAllocator,
}

impl BufferAllocationGuard {
    fn new(ptr: *mut c_void, allocator: BufferAllocator) -> Self {
        Self { ptr, allocator }
    }

    fn free(&mut self) {
        if self.ptr.is_null() {
            return;
        }
        match self.allocator {
            BufferAllocator::CoTaskMem => unsafe {
                windows::Win32::System::Com::CoTaskMemFree(Some(self.ptr));
            },
        }
        self.ptr = std::ptr::null_mut();
    }
}

impl Drop for BufferAllocationGuard {
    fn drop(&mut self) {
        self.free();
    }
}

// Safety: a ComCallPlan is fully built before publication and remains
// immutable. NativeMethod invokes libffi's CIF only through shared references;
// ffi_call treats the prepared CIF and its type graph as read-only.
unsafe impl Send for ComCallPlan {}
unsafe impl Sync for ComCallPlan {}

#[derive(Debug, Clone)]
pub struct MethodSignature {
    table: Arc<MetadataTable>,
    parameters: Vec<ComParameterSpec>,
    return_plan: ComReturnPlan,
    enumerator_next_vtable_index: Option<usize>,
}

impl MethodSignature {
    pub fn new(table: &std::sync::Arc<MetadataTable>) -> Self {
        Self {
            table: Arc::clone(table),
            parameters: Vec::new(),
            return_plan: ComReturnPlan::HResult,
            enumerator_next_vtable_index: None,
        }
    }

    pub fn add_in(mut self, typ: Type) -> Self {
        self.parameters.push(ComParameterSpec {
            direction: ComParameterDirection::In,
            typ,
            buffer: None,
        });
        self
    }

    pub fn add_out(mut self, typ: Type) -> Self {
        self.parameters.push(ComParameterSpec {
            direction: ComParameterDirection::Out,
            typ,
            buffer: None,
        });
        self
    }

    pub fn add_optional_out(mut self, typ: Type) -> Self {
        self.parameters.push(ComParameterSpec {
            direction: ComParameterDirection::OptionalOut,
            typ,
            buffer: None,
        });
        self
    }

    pub fn add_in_out(mut self, typ: Type) -> Self {
        self.parameters.push(ComParameterSpec {
            direction: ComParameterDirection::InOut,
            typ,
            buffer: None,
        });
        self
    }

    pub fn add_out_fill(mut self, typ: Type) -> Self {
        self.parameters.push(ComParameterSpec {
            direction: ComParameterDirection::OutFill,
            typ,
            buffer: None,
        });
        self
    }

    pub fn add_input_buffer(
        mut self,
        element_type: Type,
        count_param: usize,
        actual_length_param: Option<usize>,
        unit: BufferCountUnit,
    ) -> result::Result<Self> {
        let element = BufferElementPlan::from_type(&element_type)?;
        self.parameters.push(ComParameterSpec {
            direction: ComParameterDirection::InputBuffer,
            typ: Type::pointer(),
            buffer: Some(ComBufferContract {
                element,
                relation: ComBufferRelation::Input {
                    count_param,
                    actual_length_param,
                    unit,
                },
            }),
        });
        Ok(self)
    }

    pub fn add_input_string_array(
        mut self,
        encoding: StringEncoding,
        count_param: usize,
    ) -> result::Result<Self> {
        self.parameters.push(ComParameterSpec {
            direction: ComParameterDirection::InputBuffer,
            typ: Type::pointer(),
            buffer: Some(ComBufferContract {
                element: BufferElementPlan::string_pointer(encoding),
                relation: ComBufferRelation::Input {
                    count_param,
                    actual_length_param: None,
                    unit: BufferCountUnit::Elements,
                },
            }),
        });
        Ok(self)
    }

    pub fn add_caller_output_buffer(
        mut self,
        element_type: Type,
        capacity_param: usize,
        actual_length_param: Option<usize>,
        unit: BufferCountUnit,
        two_call: bool,
    ) -> result::Result<Self> {
        let element = BufferElementPlan::from_type(&element_type)?;
        self.parameters.push(ComParameterSpec {
            direction: ComParameterDirection::CallerOutputBuffer,
            typ: Type::pointer(),
            buffer: Some(ComBufferContract {
                element,
                relation: ComBufferRelation::CallerCapacity {
                    capacity_param,
                    actual_length_param,
                    unit,
                    two_call,
                },
            }),
        });
        Ok(self)
    }

    pub fn add_enumerator_next_buffer(
        mut self,
        element_type: Type,
        capacity_param: usize,
        fetched_param: usize,
    ) -> result::Result<Self> {
        let element = BufferElementPlan::from_enumerator_type(&element_type)?;
        self.parameters.push(ComParameterSpec {
            direction: ComParameterDirection::CallerOutputBuffer,
            typ: Type::pointer(),
            buffer: Some(ComBufferContract {
                element,
                relation: ComBufferRelation::EnumeratorNext {
                    capacity_param,
                    fetched_param,
                },
            }),
        });
        Ok(self)
    }

    pub fn add_callee_allocated_buffer(
        mut self,
        element_type: Type,
        count_param: usize,
        unit: BufferCountUnit,
        allocator: BufferAllocator,
    ) -> result::Result<Self> {
        let element = BufferElementPlan::from_type(&element_type)?;
        let typ = match allocator {
            BufferAllocator::CoTaskMem => Type::co_task_mem_pointer(),
        };
        self.parameters.push(ComParameterSpec {
            direction: ComParameterDirection::CalleeAllocatedBuffer,
            typ,
            buffer: Some(ComBufferContract {
                element,
                relation: ComBufferRelation::CalleeAllocated {
                    count_param,
                    unit,
                    allocator,
                },
            }),
        });
        Ok(self)
    }

    pub fn returns(mut self, typ: Type) -> Self {
        assert!(
            typ.supports_direct_return(),
            "direct native returns currently support scalars, enums, and pointers"
        );
        self.return_plan = ComReturnPlan::Direct(typ);
        self
    }

    pub fn returns_void(mut self) -> Self {
        self.return_plan = ComReturnPlan::Void;
        self
    }

    pub fn preserve_hresult(mut self) -> Self {
        self.return_plan = ComReturnPlan::SemanticHResult;
        self
    }

    pub fn preserve_enumerator_next_hresult(mut self) -> Self {
        self.enumerator_next_vtable_index = Some(3);
        self.return_plan = ComReturnPlan::EnumeratorNextHResult;
        self
    }

    pub fn preserve_enumerator_next_hresult_at(mut self, vtable_index: usize) -> Self {
        self.enumerator_next_vtable_index = Some(vtable_index);
        self.return_plan = ComReturnPlan::EnumeratorNextHResult;
        self
    }

    pub fn capture_dispatch_invoke_hresult(mut self) -> Self {
        self.return_plan = ComReturnPlan::DispatchInvokeHResult(CapturedHResultPlan {
            result_output_index: 0,
            excep_info_output_index: 1,
            arg_err_output_index: 2,
        });
        self
    }

    fn validate_registration(
        &self,
        interface_iid: GUID,
        method_name: &str,
        vtable_index: usize,
    ) -> result::Result<()> {
        const IID_IDISPATCH: GUID = GUID::from_u128(0x00020400_0000_0000_c000_000000000046);
        if matches!(self.return_plan, ComReturnPlan::EnumeratorNextHResult) {
            let exact_contract = interface_iid != GUID::zeroed()
                && method_name == "Next"
                && self.enumerator_next_vtable_index == Some(vtable_index)
                && self.parameters.len() == 3
                && self.parameters[0].direction == ComParameterDirection::In
                && self.parameters[1].direction == ComParameterDirection::CallerOutputBuffer
                && matches!(
                    self.parameters[2].direction,
                    ComParameterDirection::Out | ComParameterDirection::OptionalOut
                )
                && matches!(
                    &self.parameters[0].typ.abi,
                    ParameterType::WinRT(typ) if matches!(typ.kind(), TypeKind::U32)
                )
                && matches!(
                    &self.parameters[2].typ.abi,
                    ParameterType::WinRT(typ) if matches!(typ.kind(), TypeKind::U32)
                )
                && matches!(
                    self.parameters[1]
                        .buffer
                        .as_ref()
                        .map(|contract| &contract.relation),
                    Some(ComBufferRelation::EnumeratorNext {
                        capacity_param: 0,
                        fetched_param: 2,
                    })
                );
            return exact_contract.then_some(()).ok_or_else(|| {
                invalid_argument(
                    "enumerator HRESULT convention is restricted to the exact IEnum*::Next ABI shape",
                )
            });
        }
        if !matches!(self.return_plan, ComReturnPlan::DispatchInvokeHResult(_)) {
            return Ok(());
        }

        let direction = |index: usize| self.parameters[index].direction;
        let exact_contract = interface_iid == IID_IDISPATCH
            && method_name == "Invoke"
            && vtable_index == 6
            && self.parameters.len() == 8
            && self
                .parameters
                .iter()
                .all(|parameter| parameter.buffer.is_none())
            && (0..5).all(|index| direction(index) == ComParameterDirection::In)
            && (5..8).all(|index| direction(index) == ComParameterDirection::OptionalOut)
            && matches!(
                &self.parameters[0].typ.abi,
                ParameterType::WinRT(typ) if matches!(typ.kind(), TypeKind::I32)
            )
            && matches!(&self.parameters[1].typ.abi, ParameterType::Pointer)
            && matches!(
                &self.parameters[2].typ.abi,
                ParameterType::WinRT(typ) if matches!(typ.kind(), TypeKind::U32)
            )
            && matches!(
                &self.parameters[3].typ.abi,
                ParameterType::WinRT(typ) if matches!(typ.kind(), TypeKind::U16)
            )
            && self.parameters[4].typ.abi.is_dispatch_params()
            && self.parameters[5].typ.abi.is_variant()
            && self.parameters[6].typ.abi.is_excep_info()
            && matches!(
                &self.parameters[7].typ.abi,
                ParameterType::WinRT(typ) if matches!(typ.kind(), TypeKind::U32)
            );
        if exact_contract {
            Ok(())
        } else {
            Err(invalid_argument(
                "captured HRESULT convention is restricted to the exact IDispatch::Invoke ABI contract",
            ))
        }
    }

    fn build(self, vtable_index: usize) -> result::Result<RegisteredMethod> {
        validate_automation_contracts(&self.parameters, &self.return_plan)?;
        validate_buffer_contracts(&self.parameters)?;
        let enumerator_buffers = self
            .parameters
            .iter()
            .filter(|parameter| {
                parameter.buffer.as_ref().is_some_and(|contract| {
                    matches!(contract.relation, ComBufferRelation::EnumeratorNext { .. })
                })
            })
            .count();
        if matches!(self.return_plan, ComReturnPlan::EnumeratorNextHResult) {
            if enumerator_buffers != 1 {
                return Err(invalid_argument(
                    "enumerator HRESULT calls require exactly one EnumeratorNext buffer",
                ));
            }
        } else if enumerator_buffers != 0 {
            return Err(invalid_argument(
                "EnumeratorNext buffers require the enumerator HRESULT return convention",
            ));
        }
        let native_parameters = self
            .parameters
            .iter()
            .map(|parameter| {
                let cleanup = match parameter.direction {
                    ComParameterDirection::Out
                    | ComParameterDirection::OptionalOut
                    | ComParameterDirection::CalleeAllocatedBuffer => {
                        parameter.typ.output_cleanup()
                    }
                    ComParameterDirection::InOut if parameter.typ.abi.is_bstr() => {
                        OutputCleanup::BstrFree
                    }
                    ComParameterDirection::In
                    | ComParameterDirection::InOut
                    | ComParameterDirection::OutFill
                    | ComParameterDirection::InputBuffer
                    | ComParameterDirection::CallerOutputBuffer => OutputCleanup::None,
                };
                (
                    parameter.direction.native_kind(),
                    parameter.typ.abi.clone(),
                    cleanup,
                )
            })
            .collect();
        let native_return = match &self.return_plan {
            ComReturnPlan::HResult => MethodReturn::HResult,
            ComReturnPlan::SemanticHResult => MethodReturn::SemanticHResult,
            ComReturnPlan::EnumeratorNextHResult => MethodReturn::PreservedHResult,
            ComReturnPlan::DispatchInvokeHResult(plan) => MethodReturn::CapturedHResult(*plan),
            ComReturnPlan::Void => MethodReturn::Void,
            ComReturnPlan::Direct(typ) => MethodReturn::Value {
                typ: typ.abi.clone(),
                cleanup: typ.output_cleanup(),
            },
        };
        let native =
            lower_completed_method(&self.table, vtable_index, native_parameters, native_return);
        Ok(RegisteredMethod {
            plan: ComCallPlan::new(native, self.parameters, self.return_plan),
        })
    }
}

fn validate_automation_contracts(
    parameters: &[ComParameterSpec],
    return_plan: &ComReturnPlan,
) -> result::Result<()> {
    for parameter in parameters {
        if parameter.typ.abi.native_union_layout().is_some()
            && parameter.direction != ComParameterDirection::In
        {
            return Err(invalid_argument(
                "native union pointers are input-only because outputs lack a proven active-field contract",
            ));
        }
        if parameter.typ.abi.is_dispatch_params()
            && parameter.direction != ComParameterDirection::In
        {
            return Err(invalid_argument("DISPPARAMS is input-only"));
        }
        if parameter.typ.abi.is_excep_info()
            && !matches!(
                parameter.direction,
                ComParameterDirection::Out | ComParameterDirection::OptionalOut
            )
        {
            return Err(invalid_argument("EXCEPINFO is output-only"));
        }
        if parameter.typ.abi.is_excep_info()
            && !matches!(
                return_plan,
                ComReturnPlan::HResult
                    | ComReturnPlan::SemanticHResult
                    | ComReturnPlan::DispatchInvokeHResult(_)
            )
        {
            return Err(invalid_argument(
                "EXCEPINFO outputs require an HRESULT return convention",
            ));
        }
        if parameter.typ.abi.is_variant()
            || parameter.typ.abi.is_safe_array()
            || parameter.typ.abi.is_prop_variant()
        {
            if !matches!(
                parameter.direction,
                ComParameterDirection::In
                    | ComParameterDirection::Out
                    | ComParameterDirection::OptionalOut
            ) {
                return Err(invalid_argument(
                    "Automation values support only explicit input or owned output parameters; BYREF/InOut and buffer combinations are rejected",
                ));
            }
        }
        if parameter.typ.abi.is_nullable_safe_array()
            && parameter.direction != ComParameterDirection::Out
        {
            return Err(invalid_argument(
                "nullable SAFEARRAY is supported only for an exact documented owned output",
            ));
        }
        if parameter.typ.abi.is_variant_by_value()
            && parameter.direction != ComParameterDirection::In
        {
            return Err(invalid_argument(
                "by-value VARIANT is input-only; pointer output and InOut contracts remain unsupported",
            ));
        }
    }
    Ok(())
}

fn validate_buffer_contracts(parameters: &[ComParameterSpec]) -> result::Result<()> {
    let mut related = vec![Vec::new(); parameters.len()];
    for (buffer_index, parameter) in parameters.iter().enumerate() {
        let Some(contract) = &parameter.buffer else {
            continue;
        };
        if contract.element.cleanup != BufferElementCleanup::None {
            let supported = matches!(
                (&parameter.direction, &contract.relation),
                (
                    ComParameterDirection::InputBuffer,
                    ComBufferRelation::Input {
                        unit: BufferCountUnit::Elements,
                        ..
                    }
                ) | (
                    ComParameterDirection::CallerOutputBuffer,
                    ComBufferRelation::CallerCapacity {
                        unit: BufferCountUnit::Elements,
                        two_call: false,
                        ..
                    }
                ) | (
                    ComParameterDirection::CallerOutputBuffer,
                    ComBufferRelation::EnumeratorNext { .. }
                )
            );
            if !supported {
                return Err(invalid_argument(
                    "owned COM buffer elements require an authoritative initialized range and per-element cleanup",
                ));
            }
        }
        if relation_unit(&contract.relation) == BufferCountUnit::Bytes && contract.element.size != 1
        {
            return Err(invalid_argument(
                "byte-counted COM buffers require one-byte elements",
            ));
        }
        if matches!(contract.element.kind, BufferElementKind::StringPointer(_))
            && !matches!(
                (&parameter.direction, &contract.relation),
                (
                    ComParameterDirection::InputBuffer,
                    ComBufferRelation::Input {
                        actual_length_param: None,
                        unit: BufferCountUnit::Elements,
                        ..
                    }
                )
            )
        {
            return Err(invalid_argument(
                "COM string pointer arrays must be borrowed, element-counted inputs",
            ));
        }
        if matches!(
            contract.element.kind,
            BufferElementKind::CoTaskMemWideString
        ) && !matches!(
            (&parameter.direction, &contract.relation),
            (
                ComParameterDirection::CallerOutputBuffer,
                ComBufferRelation::EnumeratorNext { .. }
            )
        ) {
            return Err(invalid_argument(
                "CoTaskMem string array elements require the exact EnumeratorNext contract",
            ));
        }
        let mut contract_indices = BTreeSet::new();
        for index in contract.relation.related_params() {
            if !contract_indices.insert(index) {
                continue;
            }
            if index >= parameters.len() || index == buffer_index {
                return Err(invalid_argument(
                    "COM buffer count relationship references an invalid parameter index",
                ));
            }
            if !related[index].contains(&buffer_index) {
                related[index].push(buffer_index);
            }
            validate_count_type(&parameters[index].typ)?;
        }
        match (&parameter.direction, &contract.relation) {
            (
                ComParameterDirection::InputBuffer,
                ComBufferRelation::Input {
                    count_param,
                    actual_length_param,
                    ..
                },
            ) => {
                require_direction(parameters, *count_param, &[ComParameterDirection::In])?;
                if let Some(actual) = actual_length_param {
                    require_direction(parameters, *actual, &[ComParameterDirection::Out])?;
                }
            }
            (
                ComParameterDirection::CallerOutputBuffer,
                ComBufferRelation::CallerCapacity {
                    capacity_param,
                    actual_length_param,
                    two_call,
                    ..
                },
            ) => {
                if actual_length_param == &Some(*capacity_param) {
                    require_direction(
                        parameters,
                        *capacity_param,
                        &[ComParameterDirection::InOut],
                    )?;
                } else {
                    require_direction(parameters, *capacity_param, &[ComParameterDirection::In])?;
                    if let Some(actual) = actual_length_param {
                        require_direction(parameters, *actual, &[ComParameterDirection::Out])?;
                    }
                }
                if *two_call && actual_length_param.is_none() {
                    return Err(invalid_argument(
                        "two-call COM buffer sizing requires an actual-length output",
                    ));
                }
            }
            (
                ComParameterDirection::CallerOutputBuffer,
                ComBufferRelation::EnumeratorNext {
                    capacity_param,
                    fetched_param,
                },
            ) => {
                require_direction(parameters, *capacity_param, &[ComParameterDirection::In])?;
                require_direction(
                    parameters,
                    *fetched_param,
                    &[
                        ComParameterDirection::Out,
                        ComParameterDirection::OptionalOut,
                    ],
                )?;
                validate_u32_count_type(&parameters[*capacity_param].typ)?;
                validate_u32_count_type(&parameters[*fetched_param].typ)?;
            }
            (
                ComParameterDirection::CalleeAllocatedBuffer,
                ComBufferRelation::CalleeAllocated { count_param, .. },
            ) => {
                require_direction(parameters, *count_param, &[ComParameterDirection::Out])?;
            }
            _ => {
                return Err(invalid_argument(
                    "COM buffer direction and count relationship do not agree",
                ));
            }
        }
    }
    for (count_index, buffers) in related.iter().enumerate() {
        if buffers.len() > 1 {
            validate_shared_count_group(parameters, count_index, buffers)?;
        }
    }
    Ok(())
}

fn validate_shared_count_group(
    parameters: &[ComParameterSpec],
    count_index: usize,
    buffers: &[usize],
) -> result::Result<()> {
    let shared_input_units = buffers
        .iter()
        .map(|&buffer_index| {
            let parameter = &parameters[buffer_index];
            let contract = parameter.buffer.as_ref().expect("validated buffer");
            match (&parameter.direction, &contract.relation) {
                (
                    ComParameterDirection::InputBuffer,
                    ComBufferRelation::Input {
                        count_param,
                        actual_length_param: None,
                        unit,
                    },
                ) if *count_param == count_index => Some(*unit),
                _ => None,
            }
        })
        .collect::<Option<Vec<_>>>();
    if shared_input_units.is_some_and(|units| {
        units
            .first()
            .is_some_and(|first| units.iter().all(|unit| unit == first))
    }) {
        return Ok(());
    }
    let mut parallel_inputs = 0usize;
    let mut parallel_outputs = 0usize;
    let parallel = buffers.iter().all(|&buffer_index| {
        let parameter = &parameters[buffer_index];
        let contract = parameter.buffer.as_ref().expect("validated buffer");
        match (&parameter.direction, &contract.relation) {
            (
                ComParameterDirection::InputBuffer,
                ComBufferRelation::Input {
                    count_param,
                    actual_length_param: None,
                    unit: BufferCountUnit::Elements,
                },
            ) if *count_param == count_index => {
                parallel_inputs += 1;
                true
            }
            (
                ComParameterDirection::CallerOutputBuffer,
                ComBufferRelation::CallerCapacity {
                    capacity_param,
                    actual_length_param: None,
                    unit: BufferCountUnit::Elements,
                    two_call: false,
                },
            ) if *capacity_param == count_index => {
                parallel_outputs += 1;
                true
            }
            _ => false,
        }
    });
    if parallel && parallel_inputs != 0 && parallel_outputs != 0 {
        return Ok(());
    }
    if buffers.len() != 2 {
        return Err(invalid_argument(
            "shared COM counts require exactly one string input array and one caller output array",
        ));
    }
    let mut string_input = false;
    let mut caller_output = false;
    for &buffer_index in buffers {
        let parameter = &parameters[buffer_index];
        let contract = parameter.buffer.as_ref().expect("validated buffer");
        match (
            &parameter.direction,
            &contract.element.kind,
            &contract.relation,
        ) {
            (
                ComParameterDirection::InputBuffer,
                BufferElementKind::StringPointer(_),
                ComBufferRelation::Input {
                    count_param,
                    actual_length_param: None,
                    unit: BufferCountUnit::Elements,
                },
            ) if *count_param == count_index && !string_input => string_input = true,
            (
                ComParameterDirection::CallerOutputBuffer,
                BufferElementKind::Plain,
                ComBufferRelation::CallerCapacity {
                    capacity_param,
                    actual_length_param: None,
                    unit: BufferCountUnit::Elements,
                    two_call: false,
                },
            ) if *capacity_param == count_index && !caller_output => caller_output = true,
            (
                ComParameterDirection::CallerOutputBuffer,
                _,
                ComBufferRelation::EnumeratorNext { .. },
            ) => {
                return Err(invalid_argument(
                    "enumerator counts cannot be shared with unrelated COM buffers",
                ));
            }
            _ => {
                return Err(invalid_argument(
                    "unrelated COM buffers cannot share one count parameter",
                ));
            }
        }
    }
    if string_input && caller_output {
        Ok(())
    } else {
        Err(invalid_argument(
            "shared COM counts require one string input array and one caller output array",
        ))
    }
}

fn validate_count_type(typ: &Type) -> result::Result<()> {
    let ParameterType::WinRT(typ) = &typ.abi else {
        return Err(invalid_argument(
            "COM buffer count parameters require integer scalar ABI types",
        ));
    };
    if matches!(
        typ.kind(),
        TypeKind::I8
            | TypeKind::U8
            | TypeKind::I16
            | TypeKind::U16
            | TypeKind::I32
            | TypeKind::U32
            | TypeKind::I64
            | TypeKind::U64
    ) {
        Ok(())
    } else {
        Err(invalid_argument(
            "COM buffer count parameters require an integer scalar ABI type",
        ))
    }
}

fn validate_u32_count_type(typ: &Type) -> result::Result<()> {
    if matches!(
        &typ.abi,
        ParameterType::WinRT(typ) if matches!(typ.kind(), TypeKind::U32)
    ) {
        Ok(())
    } else {
        Err(invalid_argument(
            "IEnum::Next capacity and fetched parameters must use ULONG/u32",
        ))
    }
}

fn require_direction(
    parameters: &[ComParameterSpec],
    index: usize,
    allowed: &[ComParameterDirection],
) -> result::Result<()> {
    if allowed.contains(&parameters[index].direction) {
        Ok(())
    } else {
        Err(invalid_argument(format!(
            "COM buffer relationship parameter {index} has direction {:?}, expected one of {allowed:?}",
            parameters[index].direction
        )))
    }
}

#[derive(Debug)]
struct RegisteredMethod {
    plan: ComCallPlan,
}

#[derive(Debug, Clone)]
pub struct Interface {
    name: String,
    iid: GUID,
    base_slot: usize,
    methods: Arc<RwLock<BTreeMap<usize, (String, Arc<RegisteredMethod>)>>>,
}

impl Interface {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn iid(&self) -> GUID {
        self.iid
    }

    pub fn add_method(self, name: &str, signature: MethodSignature) -> Self {
        let vtable_index = self
            .methods
            .read()
            .unwrap()
            .last_key_value()
            .map_or(self.base_slot, |(slot, _)| slot + 1);
        self.add_method_at(vtable_index, name, signature)
            .expect("sequential COM method registration must use a free vtable slot")
    }

    pub fn add_method_at(
        self,
        vtable_index: usize,
        name: &str,
        signature: MethodSignature,
    ) -> result::Result<Self> {
        if vtable_index < self.base_slot {
            return Err(invalid_argument(format!(
                "COM method '{name}' uses vtable slot {vtable_index}, before the interface base slot {}",
                self.base_slot
            )));
        }
        signature.validate_registration(self.iid, name, vtable_index)?;

        let mut methods = self.methods.write().unwrap();
        if methods.contains_key(&vtable_index) {
            return Err(invalid_argument(format!(
                "COM vtable slot {vtable_index} is already registered on '{}'",
                self.name
            )));
        }
        methods.insert(
            vtable_index,
            (name.to_string(), Arc::new(signature.build(vtable_index)?)),
        );
        drop(methods);
        Ok(self)
    }

    pub fn method(&self, vtable_index: usize) -> Option<MethodHandle> {
        self.methods
            .read()
            .unwrap()
            .get(&vtable_index)
            .map(|(_, method)| MethodHandle(Arc::clone(method)))
    }
}

#[derive(Clone)]
pub struct MethodHandle(Arc<RegisteredMethod>);

impl std::fmt::Debug for MethodHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MethodHandle").finish_non_exhaustive()
    }
}

impl MethodHandle {
    pub fn result_count(&self) -> usize {
        self.0.plan.results.len()
    }

    /// # Safety
    ///
    /// `obj` must point to a live COM interface whose vtable contains this
    /// method at its registered slot for the duration of the call.
    pub unsafe fn invoke(
        &self,
        obj: *mut c_void,
        args: &[WinRTValue],
    ) -> result::Result<Vec<WinRTValue>> {
        self.0.plan.invoke(obj, args)
    }

    /// # Safety
    ///
    /// `obj` must point to a live COM interface whose vtable contains this
    /// method at its registered slot for the duration of the call.
    pub unsafe fn invoke_with_output_kinds(
        &self,
        obj: *mut c_void,
        args: &[WinRTValue],
    ) -> result::Result<Vec<(WinRTValue, PointerOutputKind)>> {
        self.0.plan.invoke_with_output_kinds(obj, args)
    }

    /// # Safety
    ///
    /// `obj` must point to a live COM interface whose vtable contains this
    /// method at its registered slot for the duration of the call.
    pub unsafe fn invoke_values_with_output_kinds(
        &self,
        obj: *mut c_void,
        args: &[Value],
    ) -> result::Result<Vec<(Value, PointerOutputKind)>> {
        self.0.plan.invoke_values_with_output_kinds(obj, args)
    }

    /// # Safety
    ///
    /// `obj` must point to a live IDispatch interface whose vtable contains
    /// Invoke at slot 6 for the duration of the call.
    pub unsafe fn invoke_dispatch(
        &self,
        obj: *mut c_void,
        args: &[Value],
    ) -> result::Result<DispatchInvokeResult> {
        self.0.plan.invoke_dispatch(obj, args)
    }

    /// # Safety
    ///
    /// `obj` must point to a live COM interface whose vtable contains this
    /// HSTRING getter at its registered slot for the duration of the call.
    pub unsafe fn call_getter_hstring(
        &self,
        obj: *mut c_void,
    ) -> result::Result<windows_core::HSTRING> {
        self.0
            .plan
            .native
            .call_getter_hstring(obj)
            .map_err(result::Error::WindowsError)
    }
}

pub fn register_interface(
    _table: &std::sync::Arc<MetadataTable>,
    name: &str,
    iid: GUID,
    base: InterfaceBase,
) -> Interface {
    Interface {
        name: name.to_string(),
        iid,
        base_slot: base.first_method_slot(),
        methods: Arc::new(RwLock::new(BTreeMap::new())),
    }
}

fn invalid_argument(message: impl Into<String>) -> result::Error {
    let message = message.into();
    result::Error::WindowsError(windows_core::Error::new(
        windows_core::HRESULT(0x80070057u32 as i32),
        &message,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApartmentType {
    SingleThreaded,
    MultiThreaded,
}

impl ApartmentType {
    fn as_flag(self) -> windows::Win32::System::Com::COINIT {
        match self {
            Self::SingleThreaded => COINIT_APARTMENTTHREADED,
            Self::MultiThreaded => COINIT_MULTITHREADED,
        }
    }
}

struct ComApartment {
    apartment_type: ApartmentType,
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

enum ComInitialization {
    Uninitialized,
    Owned(ComApartment),
}

thread_local! {
    static COM_INITIALIZATION: RefCell<ComInitialization> =
        const { RefCell::new(ComInitialization::Uninitialized) };
}

pub fn initialize_apartment(apartment_type: ApartmentType) -> result::Result<()> {
    COM_INITIALIZATION.with(|state| {
        if let ComInitialization::Owned(existing) = &*state.borrow() {
            return if existing.apartment_type == apartment_type {
                Ok(())
            } else {
                Err(result::Error::WindowsError(
                    windows_core::Error::from_hresult(RPC_E_CHANGED_MODE),
                ))
            };
        }

        let hr = unsafe { CoInitializeEx(None, apartment_type.as_flag()) };
        if hr.is_ok() {
            *state.borrow_mut() = ComInitialization::Owned(ComApartment { apartment_type });
            Ok(())
        } else {
            Err(result::Error::WindowsError(
                windows_core::Error::from_hresult(hr),
            ))
        }
    })
}

pub fn co_create_instance(clsid: GUID, iid: GUID) -> result::Result<WinRTValue> {
    let unknown: IUnknown = unsafe { CoCreateInstance(&clsid, None, CLSCTX_INPROC_SERVER) }
        .map_err(result::Error::WindowsError)?;
    let mut result = std::ptr::null_mut();
    unsafe { unknown.query(&iid, &mut result) }
        .ok()
        .map_err(result::Error::WindowsError)?;
    Ok(WinRTValue::Object(unsafe { IUnknown::from_raw(result) }))
}

/// Adopt an AddRef-owned COM interface pointer into a managed Object value.
///
/// The pointer must represent a caller-owned COM reference (+1). This function
/// takes ownership with `IUnknown::from_raw` and must not be used for borrowed
/// pointers.
pub unsafe fn adopt_com_pointer(ptr: *mut c_void) -> WinRTValue {
    if ptr.is_null() {
        WinRTValue::Null
    } else {
        WinRTValue::Object(unsafe { IUnknown::from_raw(ptr) })
    }
}

#[cfg(test)]
fn call_method(
    vtable_index: usize,
    obj: *mut c_void,
    signature: MethodSignature,
    args: &[WinRTValue],
) -> result::Result<Vec<WinRTValue>> {
    signature.build(vtable_index)?.plan.invoke(obj, args)
}

#[cfg(test)]
fn call_method_1_ptr(
    vtable_index: usize,
    obj: *mut c_void,
    ptr: *const c_void,
) -> result::Result<()> {
    crate::call::call_winrt_method_1(vtable_index, obj, ptr)
        .ok()
        .map_err(result::Error::WindowsError)
}

#[cfg(test)]
fn call_method_2_ptr_i32(
    vtable_index: usize,
    obj: *mut c_void,
    ptr: *mut c_void,
    value: i32,
) -> result::Result<()> {
    crate::call::call_winrt_method_2(vtable_index, obj, ptr, value)
        .ok()
        .map_err(result::Error::WindowsError)
}

#[cfg(test)]
fn wide_null(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
fn wide_buffer(characters: usize) -> Vec<u16> {
    vec![0; characters]
}

#[cfg(test)]
fn wide_to_string(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|ch| *ch == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MetadataTable, com_helpers::E_NOINTERFACE, ro_get_activation_factory_2,
        roapi::query_interface,
    };
    use std::sync::atomic::{AtomicU32, Ordering};
    use windows::{
        ApplicationModel::DataTransfer::DataTransferManager,
        Win32::{
            System::Com::{CoGetMalloc, IMalloc, IPersistFile, IStream},
            UI::Shell::{IDataTransferManagerInterop, SHCreateMemStream},
            UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPED,
            },
        },
    };
    use windows_core::{HSTRING, w};

    #[repr(C)]
    struct FakeComObject {
        vtable: *const *mut c_void,
    }

    #[repr(C)]
    struct TrackedComObject {
        vtable: *const windows_core::IUnknown_Vtbl,
        addrefs: AtomicU32,
        releases: AtomicU32,
    }

    unsafe extern "system" fn tracked_query_interface(
        _this: *mut c_void,
        _iid: *const GUID,
        result: *mut *mut c_void,
    ) -> windows_core::HRESULT {
        unsafe { *result = std::ptr::null_mut() };
        E_NOINTERFACE
    }

    unsafe extern "system" fn tracked_add_ref(this: *mut c_void) -> u32 {
        let object = unsafe { &*(this as *const TrackedComObject) };
        object.addrefs.fetch_add(1, Ordering::Relaxed);
        2
    }

    unsafe extern "system" fn tracked_release(this: *mut c_void) -> u32 {
        let object = unsafe { &*(this as *const TrackedComObject) };
        object.releases.fetch_add(1, Ordering::Relaxed);
        1
    }

    static TRACKED_VTABLE: windows_core::IUnknown_Vtbl = windows_core::IUnknown_Vtbl {
        QueryInterface: tracked_query_interface,
        AddRef: tracked_add_ref,
        Release: tracked_release,
    };

    unsafe extern "system" fn tracked_dispatch_query_interface(
        this: *mut c_void,
        _iid: *const GUID,
        result: *mut *mut c_void,
    ) -> windows_core::HRESULT {
        unsafe { *result = this };
        unsafe { tracked_add_ref(this) };
        windows_core::HRESULT(0)
    }

    static TRACKED_DISPATCH_VTABLE: windows_core::IUnknown_Vtbl = windows_core::IUnknown_Vtbl {
        QueryInterface: tracked_dispatch_query_interface,
        AddRef: tracked_add_ref,
        Release: tracked_release,
    };

    #[repr(C)]
    struct FailingOutCall {
        vtable: *const *mut c_void,
        output: *mut c_void,
    }

    unsafe extern "system" fn write_object_then_fail(
        this: *mut c_void,
        output: *mut *mut c_void,
    ) -> windows_core::HRESULT {
        let call = unsafe { &*(this as *const FailingOutCall) };
        unsafe { *output = call.output };
        windows_core::HRESULT(0x80004005u32 as i32)
    }

    unsafe extern "system" fn write_object_and_i32_then_fail(
        this: *mut c_void,
        output: *mut *mut c_void,
        number: *mut i32,
    ) -> windows_core::HRESULT {
        let call = unsafe { &*(this as *const FailingOutCall) };
        unsafe {
            *output = call.output;
            *number = 42;
        }
        windows_core::HRESULT(0x80004005u32 as i32)
    }

    #[test]
    fn interface_and_method_handle_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<ComCallPlan>();
        assert_send_sync::<Interface>();
        assert_send_sync::<MethodHandle>();
    }

    unsafe extern "system" fn return_u32(_this: *mut c_void) -> u32 {
        u32::MAX
    }

    unsafe extern "system" fn return_s_false(_this: *mut c_void) -> windows_core::HRESULT {
        windows_core::HRESULT(1)
    }

    unsafe extern "system" fn return_failure(_this: *mut c_void) -> windows_core::HRESULT {
        windows_core::HRESULT(0x80004005u32 as i32)
    }

    unsafe extern "system" fn return_hstring(
        _this: *mut c_void,
        value: *mut *mut c_void,
    ) -> windows_core::HRESULT {
        unsafe {
            *value = std::mem::transmute(HSTRING::from("dynwinrt HSTRING"));
        }
        windows_core::HRESULT(0)
    }

    static VOID_CALLS: AtomicU32 = AtomicU32::new(0);
    static BSTR_FAKE_ALLOCS: AtomicU32 = AtomicU32::new(0);
    static BSTR_FAKE_FREES: AtomicU32 = AtomicU32::new(0);
    static BSTR_INPUT_MATCHES: AtomicU32 = AtomicU32::new(0);
    static BSTR_EMPTY_INPUT_IS_NONNULL: AtomicU32 = AtomicU32::new(0);
    static BSTR_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    unsafe fn borrowed_bstr_string(value: *const u16) -> String {
        if value.is_null() {
            return String::new();
        }
        let value = std::mem::ManuallyDrop::new(unsafe { windows_core::BSTR::from_raw(value) });
        String::try_from(&*value).unwrap()
    }

    unsafe fn replace_bstr_slot(slot: *mut *const u16, replacement: Option<&str>) {
        let current = unsafe { *slot };
        if !current.is_null() {
            BSTR_FAKE_FREES.fetch_add(1, Ordering::Relaxed);
            drop(unsafe { windows_core::BSTR::from_raw(current) });
        }
        let replacement = replacement.map(|replacement| {
            let utf16 = replacement.encode_utf16().collect::<Vec<_>>();
            let value =
                unsafe { windows::Win32::Foundation::SysAllocStringLen(Some(utf16.as_slice())) };
            assert!(!value.is_empty() || utf16.is_empty());
            BSTR_FAKE_ALLOCS.fetch_add(1, Ordering::Relaxed);
            value.into_raw()
        });
        unsafe { *slot = replacement.unwrap_or(std::ptr::null()) };
    }

    unsafe extern "system" fn observe_bstr_input(
        _this: *mut c_void,
        value: *const u16,
    ) -> windows_core::HRESULT {
        let observed = unsafe { borrowed_bstr_string(value) };
        if observed == "embedded\0nul" {
            BSTR_INPUT_MATCHES.fetch_add(1, Ordering::Relaxed);
        }
        if !value.is_null() && observed.is_empty() {
            BSTR_EMPTY_INPUT_IS_NONNULL.fetch_add(1, Ordering::Relaxed);
        }
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn write_bstr_output(
        _this: *mut c_void,
        value: *mut *const u16,
    ) -> windows_core::HRESULT {
        unsafe { replace_bstr_slot(value, Some("output\0value")) };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn leave_bstr_unchanged(
        _this: *mut c_void,
        _value: *mut *const u16,
    ) -> windows_core::HRESULT {
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn replace_bstr(
        _this: *mut c_void,
        value: *mut *const u16,
    ) -> windows_core::HRESULT {
        unsafe { replace_bstr_slot(value, Some("replaced\0value")) };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn null_bstr(
        _this: *mut c_void,
        value: *mut *const u16,
    ) -> windows_core::HRESULT {
        unsafe { replace_bstr_slot(value, None) };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn leave_bstr_unchanged_then_fail(
        _this: *mut c_void,
        _value: *mut *const u16,
    ) -> windows_core::HRESULT {
        windows_core::HRESULT(0x80004005u32 as i32)
    }

    unsafe extern "system" fn replace_bstr_then_fail(
        _this: *mut c_void,
        value: *mut *const u16,
    ) -> windows_core::HRESULT {
        unsafe { replace_bstr_slot(value, Some("failure replacement")) };
        windows_core::HRESULT(0x80004005u32 as i32)
    }

    unsafe extern "system" fn null_bstr_then_fail(
        _this: *mut c_void,
        value: *mut *const u16,
    ) -> windows_core::HRESULT {
        unsafe { replace_bstr_slot(value, None) };
        windows_core::HRESULT(0x80004005u32 as i32)
    }

    unsafe extern "system" fn return_void(_this: *mut c_void) {
        VOID_CALLS.fetch_add(1, Ordering::Relaxed);
    }

    unsafe extern "system" fn increment_i32(
        _this: *mut c_void,
        value: *mut i32,
    ) -> windows_core::HRESULT {
        unsafe { *value += 1 };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn write_native_pointer(
        _this: *mut c_void,
        value: *mut *mut c_void,
    ) -> windows_core::HRESULT {
        unsafe { *value = 0x1234usize as *mut c_void };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn succeed_without_writing_native_pointer(
        _this: *mut c_void,
        _value: *mut *mut c_void,
    ) -> windows_core::HRESULT {
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn write_guid_and_i32(
        _this: *mut c_void,
        guid: *mut GUID,
        value: *mut i32,
    ) -> windows_core::HRESULT {
        unsafe {
            *guid = GUID::from_u128(0x11111111_2222_3333_4444_555555555555);
            *value = 42;
        }
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn copy_variant_value(
        _this: *mut c_void,
        input: *const windows::Win32::System::Variant::VARIANT,
        output: *mut windows::Win32::System::Variant::VARIANT,
    ) -> windows_core::HRESULT {
        match unsafe { windows::Win32::System::Variant::VariantCopy(output, input) } {
            Ok(()) => windows_core::HRESULT(0),
            Err(error) => error.code(),
        }
    }

    unsafe extern "system" fn observe_variant_by_value_i32(
        _this: *mut c_void,
        input: windows::Win32::System::Variant::VARIANT,
        output: *mut i32,
    ) -> windows_core::HRESULT {
        let input = std::mem::ManuallyDrop::new(input);
        unsafe { *output = automation::variant_i32_for_test(&*input) };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn observe_variant_by_value_bstr(
        _this: *mut c_void,
        input: windows::Win32::System::Variant::VARIANT,
        output: *mut u32,
    ) -> windows_core::HRESULT {
        let input = std::mem::ManuallyDrop::new(input);
        let matches = unsafe { automation::variant_bstr_for_test(&*input) == "by value" };
        unsafe { *output = u32::from(matches) };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn observe_variant_by_value_unknown(
        _this: *mut c_void,
        input: windows::Win32::System::Variant::VARIANT,
        output: *mut u32,
    ) -> windows_core::HRESULT {
        let input = std::mem::ManuallyDrop::new(input);
        let matches = unsafe { automation::variant_unknown_is_non_null_for_test(&*input) };
        unsafe { *output = u32::from(matches) };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn mutate_variant_by_value_then_fail(
        _this: *mut c_void,
        input: windows::Win32::System::Variant::VARIANT,
    ) -> windows_core::HRESULT {
        let mut input = std::mem::ManuallyDrop::new(input);
        unsafe {
            automation::set_variant_vartype_for_test(
                &mut *input,
                windows::Win32::System::Variant::VT_EMPTY.0,
            )
        };
        windows_core::HRESULT(0x80004005u32 as i32)
    }

    static VARIANT_BY_VALUE_DISPATCH_CALLS: AtomicU32 = AtomicU32::new(0);

    unsafe extern "system" fn count_variant_by_value_dispatch(
        _this: *mut c_void,
        _input: windows::Win32::System::Variant::VARIANT,
    ) -> windows_core::HRESULT {
        VARIANT_BY_VALUE_DISPATCH_CALLS.fetch_add(1, Ordering::Relaxed);
        windows_core::HRESULT(0)
    }

    static DEFERRED_FILL_CALLS: AtomicU32 = AtomicU32::new(0);

    unsafe extern "system" fn deferred_fill_success(
        info: *mut windows::Win32::System::Com::EXCEPINFO,
    ) -> windows_core::HRESULT {
        DEFERRED_FILL_CALLS.fetch_add(1, Ordering::Relaxed);
        unsafe {
            (*info).wCode = 23;
            (*info).bstrSource =
                std::mem::ManuallyDrop::new(windows_core::BSTR::from("deferred source"));
            (*info).bstrDescription =
                std::mem::ManuallyDrop::new(windows_core::BSTR::from("deferred description"));
            (*info).bstrHelpFile =
                std::mem::ManuallyDrop::new(windows_core::BSTR::from("deferred help"));
            (*info).dwHelpContext = 42;
            (*info).scode = 0x80020009u32 as i32;
        }
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn deferred_fill_failure(
        info: *mut windows::Win32::System::Com::EXCEPINFO,
    ) -> windows_core::HRESULT {
        DEFERRED_FILL_CALLS.fetch_add(1, Ordering::Relaxed);
        unsafe {
            (*info).bstrDescription =
                std::mem::ManuallyDrop::new(windows_core::BSTR::from("partial failure"));
        }
        windows_core::HRESULT(0x80004005u32 as i32)
    }

    unsafe extern "system" fn deferred_fill_reinstalls_callback(
        info: *mut windows::Win32::System::Com::EXCEPINFO,
    ) -> windows_core::HRESULT {
        DEFERRED_FILL_CALLS.fetch_add(1, Ordering::Relaxed);
        unsafe {
            (*info).bstrDescription =
                std::mem::ManuallyDrop::new(windows_core::BSTR::from("unsupported deferred"));
            (*info).pfnDeferredFillIn = Some(deferred_fill_success);
        }
        windows_core::HRESULT(0)
    }

    static DISPATCH_OPTIONAL_NULL_MASK: AtomicU32 = AtomicU32::new(0);

    unsafe extern "system" fn inspect_dispatch_params_and_fill_outputs(
        _this: *mut c_void,
        params: *mut windows::Win32::System::Com::DISPPARAMS,
        result: *mut windows::Win32::System::Variant::VARIANT,
        info: *mut windows::Win32::System::Com::EXCEPINFO,
        arg_error: *mut u32,
    ) -> windows_core::HRESULT {
        let params = unsafe { &*params };
        assert_eq!((params.cArgs, params.cNamedArgs), (3, 2));
        assert_eq!(
            unsafe {
                std::slice::from_raw_parts(params.rgdispidNamedArgs, params.cNamedArgs as usize)
            },
            &[200, 100]
        );
        assert_eq!(
            unsafe { automation::variant_i32_for_test(params.rgvarg) },
            30
        );
        assert_eq!(
            unsafe { automation::variant_i32_for_test(params.rgvarg.add(1)) },
            20
        );
        assert_eq!(
            unsafe { automation::variant_i32_for_test(params.rgvarg.add(2)) },
            10
        );
        assert_eq!(
            unsafe { params.rgvarg.add(1).byte_offset_from(params.rgvarg) },
            size_of::<windows::Win32::System::Variant::VARIANT>() as isize
        );

        let mut null_mask = 0;
        if result.is_null() {
            null_mask |= 1;
        } else if let Err(error) =
            unsafe { windows::Win32::System::Variant::VariantCopy(result, params.rgvarg) }
        {
            return error.code();
        }
        if info.is_null() {
            null_mask |= 2;
        } else {
            unsafe {
                (*info).wCode = 17;
                (*info).bstrSource =
                    std::mem::ManuallyDrop::new(windows_core::BSTR::from("source"));
                (*info).bstrDescription =
                    std::mem::ManuallyDrop::new(windows_core::BSTR::from("description"));
                (*info).bstrHelpFile =
                    std::mem::ManuallyDrop::new(windows_core::BSTR::from("help.chm"));
                (*info).dwHelpContext = 91;
                (*info).scode = 0x80020009u32 as i32;
            }
        }
        if arg_error.is_null() {
            null_mask |= 4;
        } else {
            unsafe { *arg_error = 2 };
        }
        DISPATCH_OPTIONAL_NULL_MASK.store(null_mask, Ordering::Relaxed);
        windows_core::HRESULT(0)
    }

    const DISP_E_TYPEMISMATCH: windows_core::HRESULT = windows_core::HRESULT(0x80020005u32 as i32);
    const DISP_E_EXCEPTION: windows_core::HRESULT = windows_core::HRESULT(0x80020009u32 as i32);

    static DISPATCH_INVOKE_NULL_MASK: AtomicU32 = AtomicU32::new(0);
    static DISPATCH_DEFERRED_SUCCESS_CALLS: AtomicU32 = AtomicU32::new(0);
    static DISPATCH_DEFERRED_FAILURE_CALLS: AtomicU32 = AtomicU32::new(0);

    type DispatchInvokeFn = unsafe extern "system" fn(
        *mut c_void,
        i32,
        *const GUID,
        u32,
        u16,
        *mut windows::Win32::System::Com::DISPPARAMS,
        *mut windows::Win32::System::Variant::VARIANT,
        *mut windows::Win32::System::Com::EXCEPINFO,
        *mut u32,
    ) -> windows_core::HRESULT;

    unsafe extern "system" fn dispatch_deferred_fill_success(
        info: *mut windows::Win32::System::Com::EXCEPINFO,
    ) -> windows_core::HRESULT {
        DISPATCH_DEFERRED_SUCCESS_CALLS.fetch_add(1, Ordering::Relaxed);
        unsafe {
            (*info).wCode = 23;
            (*info).bstrSource =
                std::mem::ManuallyDrop::new(windows_core::BSTR::from("deferred source"));
            (*info).bstrDescription =
                std::mem::ManuallyDrop::new(windows_core::BSTR::from("deferred description"));
            (*info).bstrHelpFile =
                std::mem::ManuallyDrop::new(windows_core::BSTR::from("deferred help"));
            (*info).dwHelpContext = 42;
            (*info).scode = DISP_E_EXCEPTION.0;
        }
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn dispatch_deferred_fill_failure(
        info: *mut windows::Win32::System::Com::EXCEPINFO,
    ) -> windows_core::HRESULT {
        DISPATCH_DEFERRED_FAILURE_CALLS.fetch_add(1, Ordering::Relaxed);
        unsafe {
            (*info).bstrDescription =
                std::mem::ManuallyDrop::new(windows_core::BSTR::from("partial failure"));
        }
        windows_core::HRESULT(0x80004005u32 as i32)
    }

    unsafe extern "system" fn dispatch_invoke_success(
        _this: *mut c_void,
        _disp_id: i32,
        _riid: *const GUID,
        _lcid: u32,
        _flags: u16,
        _params: *mut windows::Win32::System::Com::DISPPARAMS,
        result: *mut windows::Win32::System::Variant::VARIANT,
        info: *mut windows::Win32::System::Com::EXCEPINFO,
        arg_err: *mut u32,
    ) -> windows_core::HRESULT {
        assert!(!result.is_null());
        let value = VariantValue::from_i32(42);
        unsafe { windows::Win32::System::Variant::VariantCopy(result, value.raw()) }.unwrap();
        if !info.is_null() {
            unsafe {
                (*info).bstrDescription =
                    std::mem::ManuallyDrop::new(windows_core::BSTR::from("ignored on success"));
                (*info).pfnDeferredFillIn = Some(dispatch_deferred_fill_success);
            }
        }
        if !arg_err.is_null() {
            unsafe { *arg_err = 99 };
        }
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn dispatch_invoke_exception(
        _this: *mut c_void,
        _disp_id: i32,
        _riid: *const GUID,
        _lcid: u32,
        _flags: u16,
        _params: *mut windows::Win32::System::Com::DISPPARAMS,
        _result: *mut windows::Win32::System::Variant::VARIANT,
        info: *mut windows::Win32::System::Com::EXCEPINFO,
        _arg_err: *mut u32,
    ) -> windows_core::HRESULT {
        assert!(!info.is_null());
        unsafe { (*info).pfnDeferredFillIn = Some(dispatch_deferred_fill_success) };
        DISP_E_EXCEPTION
    }

    unsafe extern "system" fn dispatch_invoke_exception_with_failing_deferred(
        _this: *mut c_void,
        _disp_id: i32,
        _riid: *const GUID,
        _lcid: u32,
        _flags: u16,
        _params: *mut windows::Win32::System::Com::DISPPARAMS,
        _result: *mut windows::Win32::System::Variant::VARIANT,
        info: *mut windows::Win32::System::Com::EXCEPINFO,
        _arg_err: *mut u32,
    ) -> windows_core::HRESULT {
        assert!(!info.is_null());
        unsafe { (*info).pfnDeferredFillIn = Some(dispatch_deferred_fill_failure) };
        DISP_E_EXCEPTION
    }

    unsafe extern "system" fn dispatch_invoke_type_mismatch(
        _this: *mut c_void,
        _disp_id: i32,
        _riid: *const GUID,
        _lcid: u32,
        _flags: u16,
        _params: *mut windows::Win32::System::Com::DISPPARAMS,
        _result: *mut windows::Win32::System::Variant::VARIANT,
        _info: *mut windows::Win32::System::Com::EXCEPINFO,
        arg_err: *mut u32,
    ) -> windows_core::HRESULT {
        assert!(!arg_err.is_null());
        unsafe { *arg_err = 3 };
        DISP_E_TYPEMISMATCH
    }

    unsafe extern "system" fn dispatch_invoke_fail(
        _this: *mut c_void,
        _disp_id: i32,
        _riid: *const GUID,
        _lcid: u32,
        _flags: u16,
        _params: *mut windows::Win32::System::Com::DISPPARAMS,
        _result: *mut windows::Win32::System::Variant::VARIANT,
        _info: *mut windows::Win32::System::Com::EXCEPINFO,
        _arg_err: *mut u32,
    ) -> windows_core::HRESULT {
        windows_core::HRESULT(0x80004005u32 as i32)
    }

    unsafe extern "system" fn dispatch_invoke_disabled_outputs(
        _this: *mut c_void,
        _disp_id: i32,
        _riid: *const GUID,
        _lcid: u32,
        _flags: u16,
        _params: *mut windows::Win32::System::Com::DISPPARAMS,
        result: *mut windows::Win32::System::Variant::VARIANT,
        info: *mut windows::Win32::System::Com::EXCEPINFO,
        arg_err: *mut u32,
    ) -> windows_core::HRESULT {
        let mut mask = 0;
        if result.is_null() {
            mask |= 1;
        }
        if info.is_null() {
            mask |= 2;
        }
        if arg_err.is_null() {
            mask |= 4;
        }
        DISPATCH_INVOKE_NULL_MASK.store(mask, Ordering::Relaxed);
        windows_core::HRESULT(0)
    }

    #[repr(C)]
    struct DispatchTrackedCall {
        vtable: *const *mut c_void,
        output: *mut c_void,
    }

    unsafe extern "system" fn dispatch_invoke_partial_result_then_fail(
        this: *mut c_void,
        _disp_id: i32,
        _riid: *const GUID,
        _lcid: u32,
        _flags: u16,
        _params: *mut windows::Win32::System::Com::DISPPARAMS,
        result: *mut windows::Win32::System::Variant::VARIANT,
        _info: *mut windows::Win32::System::Com::EXCEPINFO,
        _arg_err: *mut u32,
    ) -> windows_core::HRESULT {
        let call = unsafe { &*(this as *const DispatchTrackedCall) };
        let borrowed = unsafe { IUnknown::from_raw_borrowed(&call.output) }.unwrap();
        let value = VariantValue::from_unknown(Some(borrowed));
        unsafe { windows::Win32::System::Variant::VariantCopy(result, value.raw()) }.unwrap();
        windows_core::HRESULT(0x80004005u32 as i32)
    }

    unsafe extern "system" fn install_deferred_excep_info(
        _this: *mut c_void,
        info: *mut windows::Win32::System::Com::EXCEPINFO,
    ) -> windows_core::HRESULT {
        unsafe { (*info).pfnDeferredFillIn = Some(deferred_fill_success) };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn install_failing_deferred_excep_info(
        _this: *mut c_void,
        info: *mut windows::Win32::System::Com::EXCEPINFO,
    ) -> windows_core::HRESULT {
        unsafe { (*info).pfnDeferredFillIn = Some(deferred_fill_failure) };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn install_reinstalling_deferred_excep_info(
        _this: *mut c_void,
        info: *mut windows::Win32::System::Com::EXCEPINFO,
    ) -> windows_core::HRESULT {
        unsafe { (*info).pfnDeferredFillIn = Some(deferred_fill_reinstalls_callback) };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn write_reserved_excep_info(
        _this: *mut c_void,
        info: *mut windows::Win32::System::Com::EXCEPINFO,
    ) -> windows_core::HRESULT {
        unsafe {
            (*info).wReserved = 1;
            (*info).bstrDescription =
                std::mem::ManuallyDrop::new(windows_core::BSTR::from("reserved"));
        }
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn install_deferred_excep_info_then_fail(
        _this: *mut c_void,
        info: *mut windows::Win32::System::Com::EXCEPINFO,
    ) -> windows_core::HRESULT {
        unsafe { (*info).pfnDeferredFillIn = Some(deferred_fill_success) };
        windows_core::HRESULT(0x80004005u32 as i32)
    }

    unsafe extern "system" fn write_unsupported_variant_and_deferred_excep_info(
        _this: *mut c_void,
        variant: *mut windows::Win32::System::Variant::VARIANT,
        info: *mut windows::Win32::System::Com::EXCEPINFO,
    ) -> windows_core::HRESULT {
        unsafe {
            automation::set_variant_vartype_for_test(
                variant,
                windows::Win32::System::Variant::VT_DATE.0,
            );
            (*info).pfnDeferredFillIn = Some(deferred_fill_success);
        }
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn write_unsupported_variant_and_deferred_excep_info_then_fail(
        this: *mut c_void,
        variant: *mut windows::Win32::System::Variant::VARIANT,
        info: *mut windows::Win32::System::Com::EXCEPINFO,
    ) -> windows_core::HRESULT {
        let _ = unsafe { write_unsupported_variant_and_deferred_excep_info(this, variant, info) };
        windows_core::HRESULT(0x80004005u32 as i32)
    }

    unsafe extern "system" fn write_unsupported_variant_and_object(
        this: *mut c_void,
        variant: *mut windows::Win32::System::Variant::VARIANT,
        output: *mut *mut c_void,
    ) -> windows_core::HRESULT {
        let call = unsafe { &*(this as *const FailingOutCall) };
        unsafe {
            automation::set_variant_vartype_for_test(
                variant,
                windows::Win32::System::Variant::VT_DATE.0,
            );
            *output = call.output;
        }
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn write_object_and_unsupported_variant(
        this: *mut c_void,
        output: *mut *mut c_void,
        variant: *mut windows::Win32::System::Variant::VARIANT,
    ) -> windows_core::HRESULT {
        let call = unsafe { &*(this as *const FailingOutCall) };
        unsafe {
            *output = call.output;
            automation::set_variant_vartype_for_test(
                variant,
                windows::Win32::System::Variant::VT_DATE.0,
            );
        }
        windows_core::HRESULT(0)
    }

    static AUTOMATION_DISPATCH_CALLS: AtomicU32 = AtomicU32::new(0);

    unsafe extern "system" fn count_automation_dispatch(
        _this: *mut c_void,
        _variant: *mut windows::Win32::System::Variant::VARIANT,
    ) -> windows_core::HRESULT {
        AUTOMATION_DISPATCH_CALLS.fetch_add(1, Ordering::Relaxed);
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn return_object_with_unsupported_variant(
        this: *mut c_void,
        variant: *mut windows::Win32::System::Variant::VARIANT,
    ) -> *mut c_void {
        let call = unsafe { &*(this as *const FailingOutCall) };
        unsafe {
            automation::set_variant_vartype_for_test(
                variant,
                windows::Win32::System::Variant::VT_DATE.0,
            );
        }
        call.output
    }

    unsafe extern "system" fn copy_safe_array_value(
        _this: *mut c_void,
        input: *mut windows::Win32::System::Com::SAFEARRAY,
        output: *mut *mut windows::Win32::System::Com::SAFEARRAY,
    ) -> windows_core::HRESULT {
        match unsafe { windows::Win32::System::Ole::SafeArrayCopy(input) } {
            Ok(value) => {
                unsafe { *output = value };
                windows_core::HRESULT(0)
            }
            Err(error) => error.code(),
        }
    }

    unsafe extern "system" fn return_null_safe_array(
        _this: *mut c_void,
        output: *mut *mut windows::Win32::System::Com::SAFEARRAY,
    ) -> windows_core::HRESULT {
        unsafe { *output = std::ptr::null_mut() };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn observe_aliased_safe_arrays(
        _this: *mut c_void,
        first: *mut windows::Win32::System::Com::SAFEARRAY,
        second: *mut windows::Win32::System::Com::SAFEARRAY,
    ) -> windows_core::HRESULT {
        assert_eq!(first, second);
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn copy_prop_variant_value(
        _this: *mut c_void,
        input: *const windows::Win32::System::Com::StructuredStorage::PROPVARIANT,
        output: *mut windows::Win32::System::Com::StructuredStorage::PROPVARIANT,
    ) -> windows_core::HRESULT {
        match unsafe {
            windows::Win32::System::Com::StructuredStorage::PropVariantCopy(output, input)
        } {
            Ok(()) => windows_core::HRESULT(0),
            Err(error) => error.code(),
        }
    }

    unsafe extern "system" fn read_native_union(
        _this: *mut c_void,
        input: *const u64,
        output: *mut u64,
    ) -> windows_core::HRESULT {
        unsafe { *output = *input };
        windows_core::HRESULT(0)
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct TestPod {
        first: u32,
        second: u16,
        tag: u16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct AlignedPod {
        value: u64,
    }

    fn test_pod_layout(name: &str) -> Arc<NativeStructLayout> {
        Arc::new(
            NativeStructLayout::new(
                name,
                size_of::<TestPod>(),
                align_of::<TestPod>(),
                vec![
                    NativeStructField::new(
                        "first",
                        0,
                        1,
                        NativeStructFieldType::Scalar(NativeStructScalar::U32),
                    )
                    .unwrap(),
                    NativeStructField::new(
                        "second",
                        4,
                        1,
                        NativeStructFieldType::Scalar(NativeStructScalar::U16),
                    )
                    .unwrap(),
                    NativeStructField::new(
                        "tag",
                        6,
                        1,
                        NativeStructFieldType::Scalar(NativeStructScalar::U16),
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        )
    }

    fn test_pod_value(layout: Arc<NativeStructLayout>, value: TestPod) -> NativeStructValue {
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&value as *const TestPod).cast::<u8>(),
                size_of::<TestPod>(),
            )
        }
        .to_vec();
        NativeStructValue::new(layout, bytes).unwrap()
    }

    fn read_test_pod(value: &NativeStructValue) -> TestPod {
        unsafe { std::ptr::read_unaligned(value.bytes().as_ptr().cast::<TestPod>()) }
    }

    unsafe extern "system" fn require_aligned_pod(
        _this: *mut c_void,
        value: *const AlignedPod,
    ) -> windows_core::HRESULT {
        if (value as usize) % align_of::<AlignedPod>() == 0 {
            windows_core::HRESULT(0)
        } else {
            windows_core::HRESULT(0x80004005u32 as i32)
        }
    }

    unsafe extern "system" fn sum_pod_by_value(
        _this: *mut c_void,
        value: TestPod,
        result: *mut u32,
    ) -> windows_core::HRESULT {
        unsafe { *result = value.first + u32::from(value.second) + u32::from(value.tag) };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn sum_pod_pointer(
        _this: *mut c_void,
        value: *const TestPod,
        result: *mut u32,
    ) -> windows_core::HRESULT {
        let value = unsafe { &*value };
        unsafe { *result = value.first + u32::from(value.second) + u32::from(value.tag) };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn nullable_pod_pointer_is_null(
        _this: *mut c_void,
        value: *const TestPod,
        result: *mut u32,
    ) -> windows_core::HRESULT {
        unsafe { *result = u32::from(value.is_null()) };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn write_zeroed_pod(
        _this: *mut c_void,
        value: *mut TestPod,
    ) -> windows_core::HRESULT {
        let value = unsafe { &mut *value };
        if value.first != 0 || value.second != 0 || value.tag != 0 {
            return windows_core::HRESULT(0x80004005u32 as i32);
        }
        *value = TestPod {
            first: 40,
            second: 2,
            tag: 7,
        };
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn update_pod_in_out(
        _this: *mut c_void,
        value: *mut TestPod,
    ) -> windows_core::HRESULT {
        let value = unsafe { &mut *value };
        value.first += 1;
        value.second += 2;
        value.tag += 3;
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn update_nullable_pod_in_out(
        _this: *mut c_void,
        value: *mut TestPod,
    ) -> windows_core::HRESULT {
        if !value.is_null() {
            unsafe {
                (*value).first += 1;
                (*value).second += 2;
                (*value).tag += 3;
            }
        }
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn write_pod_then_fail(
        _this: *mut c_void,
        value: *mut TestPod,
    ) -> windows_core::HRESULT {
        unsafe {
            *value = TestPod {
                first: 1,
                second: 2,
                tag: 3,
            };
        }
        windows_core::HRESULT(0x80004005u32 as i32)
    }

    unsafe extern "system" fn read_counted_bytes(
        _this: *mut c_void,
        buffer: *mut u8,
        capacity: u32,
        actual: *mut u32,
    ) -> windows_core::HRESULT {
        let bytes = [4u8, 5, 6];
        let written = bytes.len().min(capacity as usize);
        unsafe {
            assert!((0..capacity as usize).all(|index| *buffer.add(index) == 0));
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, written);
            *actual = written as u32;
        }
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn read_counted_bytes_then_fail(
        _this: *mut c_void,
        buffer: *mut u8,
        capacity: u32,
        actual: *mut u32,
    ) -> windows_core::HRESULT {
        unsafe {
            if capacity > 0 {
                *buffer = 9;
            }
            *actual = usize::min(capacity as usize, 1) as u32;
        }
        windows_core::HRESULT(0x80004005u32 as i32)
    }

    unsafe extern "system" fn report_larger_count(
        _this: *mut c_void,
        buffer: *mut u8,
        capacity: u32,
        actual: *mut u32,
    ) -> windows_core::HRESULT {
        unsafe {
            if capacity > 0 {
                *buffer = 7;
            }
            *actual = capacity.saturating_add(2);
        }
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn get_blob_fixed_capacity(
        _this: *mut c_void,
        guid: *const GUID,
        buffer: *mut u8,
        capacity: u32,
        actual: *mut u32,
    ) -> windows_core::HRESULT {
        assert!(!guid.is_null());
        assert!(!buffer.is_null());
        assert!(!actual.is_null());
        let bytes = [11u8, 22, 33];
        unsafe {
            assert!((0..capacity as usize).all(|index| *buffer.add(index) == 0));
            *actual = bytes.len() as u32;
            if capacity < bytes.len() as u32 {
                if capacity > 0 {
                    *buffer = 0xEE;
                }
                return windows_core::HRESULT(0x8007007Au32 as i32);
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, bytes.len());
        }
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn get_blob_then_fail(
        _this: *mut c_void,
        _guid: *const GUID,
        buffer: *mut u8,
        capacity: u32,
        actual: *mut u32,
    ) -> windows_core::HRESULT {
        unsafe {
            if capacity > 0 {
                *buffer = 0xAA;
            }
            *actual = u32::from(capacity > 0);
        }
        windows_core::HRESULT(0x80004005u32 as i32)
    }

    static ENUMERATOR_NEXT_CALLS: AtomicU32 = AtomicU32::new(0);
    static OPTIONAL_ENUMERATOR_NEXT_CALLS: AtomicU32 = AtomicU32::new(0);

    unsafe extern "system" fn enum_next_partial_u32(
        _this: *mut c_void,
        capacity: u32,
        values: *mut u32,
        fetched: *mut u32,
    ) -> windows_core::HRESULT {
        ENUMERATOR_NEXT_CALLS.fetch_add(1, Ordering::Relaxed);
        assert!(!fetched.is_null());
        unsafe {
            if capacity != 0 {
                *values = 41;
            }
            *fetched = u32::from(capacity != 0);
        }
        windows_core::HRESULT(1)
    }

    unsafe extern "system" fn optional_enum_next_should_not_run(
        _this: *mut c_void,
        _capacity: u32,
        _values: *mut u32,
        _fetched: *mut u32,
    ) -> windows_core::HRESULT {
        OPTIONAL_ENUMERATOR_NEXT_CALLS.fetch_add(1, Ordering::Relaxed);
        windows_core::HRESULT(0)
    }

    #[repr(C)]
    struct EnumInterfaceCall {
        vtable: *const *mut c_void,
        values: [*mut c_void; 2],
        fetched: u32,
        hresult: windows_core::HRESULT,
    }

    unsafe extern "system" fn enum_next_interfaces(
        this: *mut c_void,
        capacity: u32,
        values: *mut *mut c_void,
        fetched: *mut u32,
    ) -> windows_core::HRESULT {
        ENUMERATOR_NEXT_CALLS.fetch_add(1, Ordering::Relaxed);
        let call = unsafe { &*(this as *const EnumInterfaceCall) };
        for (index, value) in call
            .values
            .iter()
            .copied()
            .take(capacity.min(2) as usize)
            .enumerate()
        {
            unsafe { values.add(index).write(value) };
        }
        if !fetched.is_null() {
            unsafe { *fetched = call.fetched };
        }
        call.hresult
    }

    #[repr(C)]
    struct OwningVariantArrayCall {
        vtable: *const *mut c_void,
        values: [*mut c_void; 2],
        actual: u32,
        hresult: windows_core::HRESULT,
        invalid_slot: i32,
    }

    unsafe extern "system" fn fill_variant_array(
        this: *mut c_void,
        values: *mut windows::Win32::System::Variant::VARIANT,
        capacity: u32,
        actual: *mut u32,
    ) -> windows_core::HRESULT {
        use windows::Win32::System::Variant::{
            VT_BYREF, VT_EMPTY, VT_I4, VariantClear, VariantCopy,
        };

        let call = unsafe { &*(this as *const OwningVariantArrayCall) };
        for index in 0..capacity as usize {
            assert_eq!(
                unsafe { automation::variant_vartype_for_test(values.add(index)) },
                VT_EMPTY.0
            );
        }
        for (index, pointer) in call
            .values
            .iter()
            .copied()
            .take(capacity.min(2) as usize)
            .enumerate()
        {
            let unknown =
                std::mem::ManuallyDrop::new(unsafe { IUnknown::from_raw(pointer.cast()) });
            let source = VariantValue::from_unknown(Some(&unknown));
            unsafe { VariantCopy(values.add(index), source.raw()) }.unwrap();
        }
        if call.invalid_slot >= 0 && (call.invalid_slot as u32) < capacity {
            let slot = unsafe { values.add(call.invalid_slot as usize) };
            unsafe { VariantClear(slot) }.unwrap();
            unsafe {
                automation::initialize_variant_slot(slot.cast());
                automation::set_variant_vartype_for_test(slot, VT_I4.0 | VT_BYREF.0);
            }
        }
        unsafe { *actual = call.actual };
        call.hresult
    }

    #[repr(C)]
    struct OwningStringArrayCall {
        vtable: *const *mut c_void,
        actual: u32,
        hresult: windows_core::HRESULT,
        co_task_mem: bool,
    }

    unsafe extern "system" fn fill_string_array(
        this: *mut c_void,
        values: *mut *mut u16,
        capacity: u32,
        actual: *mut u32,
    ) -> windows_core::HRESULT {
        let call = unsafe { &*(this as *const OwningStringArrayCall) };
        for index in 0..capacity as usize {
            assert!(unsafe { values.add(index).read() }.is_null());
        }
        let strings = ["embedded\0nul", "unused"];
        for (index, value) in strings.iter().take(capacity.min(2) as usize).enumerate() {
            let utf16 = value.encode_utf16().collect::<Vec<_>>();
            let pointer = if call.co_task_mem {
                let bytes = (utf16.len() + 1) * size_of::<u16>();
                let pointer =
                    unsafe { windows::Win32::System::Com::CoTaskMemAlloc(bytes) }.cast::<u16>();
                assert!(!pointer.is_null());
                unsafe {
                    std::ptr::copy_nonoverlapping(utf16.as_ptr(), pointer, utf16.len());
                    pointer.add(utf16.len()).write(0);
                }
                pointer
            } else {
                unsafe { windows::Win32::Foundation::SysAllocStringLen(Some(&utf16)) }.into_raw()
                    as *mut u16
            };
            unsafe { values.add(index).write(pointer) };
        }
        unsafe { *actual = call.actual };
        call.hresult
    }

    unsafe extern "system" fn enum_next_string_array(
        this: *mut c_void,
        capacity: u32,
        values: *mut *mut u16,
        fetched: *mut u32,
    ) -> windows_core::HRESULT {
        unsafe { fill_string_array(this, values, capacity, fetched) }
    }

    unsafe extern "system" fn observe_bstr_array(
        _this: *mut c_void,
        values: *const *const u16,
        count: u32,
    ) -> windows_core::HRESULT {
        assert_eq!(count, 2);
        assert_eq!(unsafe { borrowed_bstr_string(*values) }, "embedded\0nul");
        assert_eq!(unsafe { borrowed_bstr_string(*values.add(1)) }, "");
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn observe_variant_array(
        _this: *mut c_void,
        values: *const windows::Win32::System::Variant::VARIANT,
        count: u32,
    ) -> windows_core::HRESULT {
        assert_eq!(count, 2);
        let first = unsafe { VariantValue::from_owned_raw(values.read()) }.unwrap();
        let second = unsafe { VariantValue::from_owned_raw(values.add(1).read()) }.unwrap();
        assert!(matches!(first.data().unwrap(), VariantData::I32(17)));
        assert!(matches!(
            second.data().unwrap(),
            VariantData::Bstr(value) if value == "embedded\0nul"
        ));
        std::mem::forget(first);
        std::mem::forget(second);
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn report_larger_count_with_object(
        this: *mut c_void,
        buffer: *mut u8,
        capacity: u32,
        actual: *mut u32,
        output: *mut *mut c_void,
    ) -> windows_core::HRESULT {
        let call = unsafe { &*(this as *const FailingOutCall) };
        unsafe {
            if capacity > 0 {
                *buffer = 7;
            }
            *actual = capacity.saturating_add(1);
            *output = call.output;
        }
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn return_object_with_larger_count(
        this: *mut c_void,
        buffer: *mut u8,
        capacity: u32,
        actual: *mut u32,
    ) -> *mut c_void {
        let call = unsafe { &*(this as *const FailingOutCall) };
        unsafe {
            if capacity > 0 {
                *buffer = 7;
            }
            *actual = capacity.saturating_add(1);
        }
        call.output
    }

    unsafe extern "system" fn return_cotaskmem_bytes(
        _this: *mut c_void,
        buffer: *mut *mut u8,
        count: *mut u32,
    ) -> windows_core::HRESULT {
        let bytes = [10u8, 20, 30, 40];
        let allocation =
            unsafe { windows::Win32::System::Com::CoTaskMemAlloc(bytes.len()) }.cast::<u8>();
        if allocation.is_null() {
            return windows_core::HRESULT(0x8007000Eu32 as i32);
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), allocation, bytes.len());
            *buffer = allocation;
            *count = bytes.len() as u32;
        }
        windows_core::HRESULT(0)
    }

    static STRING_ARRAY_CALLS: AtomicU32 = AtomicU32::new(0);

    unsafe extern "system" fn resolve_string_ids(
        _this: *mut c_void,
        names: *const *const u16,
        count: u32,
        outputs: *mut i32,
    ) -> windows_core::HRESULT {
        STRING_ARRAY_CALLS.fetch_add(1, Ordering::Relaxed);
        assert_eq!(count, 2);
        let names = unsafe { std::slice::from_raw_parts(names, count as usize) };
        let outputs = unsafe { std::slice::from_raw_parts_mut(outputs, count as usize) };
        assert_eq!(outputs, [0, 0]);
        let decode = |ptr: *const u16| {
            let mut len = 0;
            while unsafe { *ptr.add(len) } != 0 {
                len += 1;
            }
            String::from_utf16(unsafe { std::slice::from_raw_parts(ptr, len) }).unwrap()
        };
        assert_eq!(decode(names[0]), "First");
        assert_eq!(decode(names[1]), "Second");
        outputs.copy_from_slice(&[17, 29]);
        windows_core::HRESULT(0)
    }

    unsafe extern "system" fn resolve_string_ids_then_fail(
        this: *mut c_void,
        names: *const *const u16,
        count: u32,
        outputs: *mut i32,
    ) -> windows_core::HRESULT {
        let _ = unsafe { resolve_string_ids(this, names, count, outputs) };
        windows_core::HRESULT(0x80004005u32 as i32)
    }

    fn invoke_test_pod(
        function: *mut c_void,
        signature: MethodSignature,
        args: &[Value],
    ) -> result::Result<Vec<Value>> {
        let vtable = [function];
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };
        signature
            .build(0)
            .expect("valid native POD signature")
            .plan
            .invoke_values((&mut object as *mut FakeComObject).cast(), args)
    }

    fn reset_bstr_counts() {
        crate::call::reset_bstr_test_counts();
        BSTR_FAKE_ALLOCS.store(0, Ordering::Relaxed);
        BSTR_FAKE_FREES.store(0, Ordering::Relaxed);
        BSTR_INPUT_MATCHES.store(0, Ordering::Relaxed);
        BSTR_EMPTY_INPUT_IS_NONNULL.store(0, Ordering::Relaxed);
    }

    fn assert_bstr_counts(allocations: usize, frees: usize) {
        let (runtime_allocations, runtime_frees) = crate::call::bstr_test_counts();
        let fake_allocations = BSTR_FAKE_ALLOCS.load(Ordering::Relaxed) as usize;
        let fake_frees = BSTR_FAKE_FREES.load(Ordering::Relaxed) as usize;
        assert_eq!(
            runtime_allocations + fake_allocations,
            allocations,
            "runtime allocations {runtime_allocations}, callee allocations {fake_allocations}"
        );
        assert_eq!(
            runtime_frees + fake_frees,
            frees,
            "runtime frees {runtime_frees}, callee/consumer frees {fake_frees}"
        );
    }

    fn take_bstr_output(value: Value) -> Option<String> {
        let Value::WinRt(WinRTValue::RawPtr(value)) = value else {
            panic!("expected raw BSTR result");
        };
        if value.is_null() {
            return None;
        }
        let value = unsafe { windows_core::BSTR::from_raw(value.cast()) };
        let result = String::try_from(&value).unwrap();
        BSTR_FAKE_FREES.fetch_add(1, Ordering::Relaxed);
        drop(value);
        Some(result)
    }

    #[test]
    fn bstr_input_preserves_embedded_nul_and_is_call_local() {
        let _guard = BSTR_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_bstr_counts();
        let original = BstrValue::new("embedded\0nul");
        let first_outputs = invoke_test_pod(
            observe_bstr_input as *mut c_void,
            MethodSignature::new(&MetadataTable::new()).add_in(Type::bstr()),
            &[Value::Bstr(original.clone())],
        )
        .unwrap();
        let empty_outputs = invoke_test_pod(
            observe_bstr_input as *mut c_void,
            MethodSignature::new(&MetadataTable::new()).add_in(Type::bstr()),
            &[Value::Bstr(BstrValue::new(""))],
        )
        .unwrap();

        assert!(first_outputs.is_empty());
        assert!(empty_outputs.is_empty());
        assert_eq!(BSTR_INPUT_MATCHES.load(Ordering::Relaxed), 1);
        assert_eq!(BSTR_EMPTY_INPUT_IS_NONNULL.load(Ordering::Relaxed), 1);
        assert_eq!(original.as_deref(), Some("embedded\0nul"));
        assert_bstr_counts(2, 2);
    }

    #[test]
    fn null_bstr_input_requires_an_explicit_nullable_contract() {
        let _guard = BSTR_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_bstr_counts();
        let null = Value::Bstr(BstrValue::null());
        let required = invoke_test_pod(
            observe_bstr_input as *mut c_void,
            MethodSignature::new(&MetadataTable::new()).add_in(Type::bstr()),
            std::slice::from_ref(&null),
        )
        .unwrap_err();
        assert!(required.message().contains("nullable BSTR"));

        let outputs = invoke_test_pod(
            observe_bstr_input as *mut c_void,
            MethodSignature::new(&MetadataTable::new()).add_in(Type::nullable_bstr()),
            &[null],
        )
        .unwrap();
        assert!(outputs.is_empty());
        assert_eq!(BSTR_EMPTY_INPUT_IS_NONNULL.load(Ordering::Relaxed), 0);
        assert_bstr_counts(0, 0);
    }

    #[test]
    fn bstr_output_preserves_embedded_nul_and_transfers_one_owner() {
        let _guard = BSTR_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_bstr_counts();
        let outputs = invoke_test_pod(
            write_bstr_output as *mut c_void,
            MethodSignature::new(&MetadataTable::new()).add_out(Type::bstr()),
            &[],
        )
        .unwrap();

        assert_eq!(outputs.len(), 1);
        assert_eq!(
            take_bstr_output(outputs.into_iter().next().unwrap()).as_deref(),
            Some("output\0value")
        );
        assert_bstr_counts(1, 1);
    }

    #[test]
    fn bstr_in_out_success_owns_unchanged_replaced_and_null_slots() {
        let _guard = BSTR_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for (function, expected, allocations, frees) in [
            (
                leave_bstr_unchanged as *mut c_void,
                Some("original\0value"),
                1,
                1,
            ),
            (replace_bstr as *mut c_void, Some("replaced\0value"), 2, 2),
            (null_bstr as *mut c_void, None, 1, 1),
        ] {
            reset_bstr_counts();
            let original = BstrValue::new("original\0value");
            let outputs = invoke_test_pod(
                function,
                MethodSignature::new(&MetadataTable::new()).add_in_out(Type::bstr()),
                &[Value::Bstr(original.clone())],
            )
            .unwrap();

            assert_eq!(outputs.len(), 1);
            assert_eq!(
                take_bstr_output(outputs.into_iter().next().unwrap()).as_deref(),
                expected
            );
            assert_eq!(original.as_deref(), Some("original\0value"));
            assert_bstr_counts(allocations, frees);
        }
    }

    #[test]
    fn bstr_in_out_failure_cleans_unchanged_replaced_and_null_slots() {
        let _guard = BSTR_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for (function, allocations, frees) in [
            (leave_bstr_unchanged_then_fail as *mut c_void, 1, 1),
            (replace_bstr_then_fail as *mut c_void, 2, 2),
            (null_bstr_then_fail as *mut c_void, 1, 1),
        ] {
            reset_bstr_counts();
            let original = BstrValue::new("original\0value");
            let error = invoke_test_pod(
                function,
                MethodSignature::new(&MetadataTable::new()).add_in_out(Type::bstr()),
                &[Value::Bstr(original.clone())],
            )
            .unwrap_err();

            assert!(matches!(
                error,
                result::Error::WindowsError(error)
                    if error.code() == windows_core::HRESULT(0x80004005u32 as i32)
            ));
            assert_eq!(original.as_deref(), Some("original\0value"));
            assert_bstr_counts(allocations, frees);
        }
    }

    fn dispatch_invoke_signature(table: &Arc<MetadataTable>) -> MethodSignature {
        MethodSignature::new(table)
            .add_in(Type::winrt(table.i32_type()))
            .add_in(Type::pointer())
            .add_in(Type::winrt(table.u32_type()))
            .add_in(Type::winrt(table.u16_type()))
            .add_in(Type::dispatch_params())
            .add_optional_out(Type::variant())
            .add_optional_out(Type::excep_info())
            .add_optional_out(Type::winrt(table.u32_type()))
            .capture_dispatch_invoke_hresult()
    }

    fn dispatch_invoke_args(
        request_result: bool,
        request_excep_info: bool,
        request_arg_err: bool,
    ) -> Vec<Value> {
        vec![
            Value::WinRt(WinRTValue::I32(7)),
            Value::WinRt(WinRTValue::RawPtr(std::ptr::null_mut())),
            Value::WinRt(WinRTValue::U32(0)),
            Value::WinRt(WinRTValue::U16(1)),
            Value::DispatchParams(DispatchParamsValue::new(&[], &[]).unwrap()),
            Value::WinRt(WinRTValue::Bool(request_result)),
            Value::WinRt(WinRTValue::Bool(request_excep_info)),
            Value::WinRt(WinRTValue::Bool(request_arg_err)),
        ]
    }

    fn dispatch_method(
        table: &Arc<MetadataTable>,
        function: DispatchInvokeFn,
    ) -> (MethodHandle, [*mut c_void; 7]) {
        let interface = register_interface(
            table,
            "Windows.Win32.System.Com.IDispatch",
            GUID::from_u128(0x00020400_0000_0000_c000_000000000046),
            InterfaceBase::IUnknown,
        )
        .add_method_at(6, "Invoke", dispatch_invoke_signature(table))
        .unwrap();
        let mut vtable = [std::ptr::null_mut(); 7];
        vtable[6] = function as *mut c_void;
        (interface.method(6).unwrap(), vtable)
    }

    fn invoke_test_buffer(
        function: *mut c_void,
        signature: MethodSignature,
        args: &[Value],
    ) -> result::Result<Vec<Value>> {
        let vtable = [function];
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };
        signature
            .build(0)?
            .plan
            .invoke_values((&mut object as *mut FakeComObject).cast(), args)
    }

    fn test_union_layout() -> Arc<NativeUnionLayout> {
        Arc::new(
            NativeUnionLayout::new(
                "Tests.NativeUnion",
                size_of::<u64>(),
                align_of::<u64>(),
                vec![
                    NativeUnionField::new(
                        "integer",
                        1,
                        NativeUnionFieldType::Scalar(NativeStructScalar::U64),
                    )
                    .unwrap(),
                    NativeUnionField::new("pointer", 1, NativeUnionFieldType::Pointer).unwrap(),
                ],
            )
            .unwrap(),
        )
    }

    fn borrowed_buffer(bytes: &mut [u8], source_element_size: usize) -> Value {
        Value::Buffer(
            unsafe {
                ComBufferValue::borrowed(
                    bytes.as_mut_ptr(),
                    bytes.len(),
                    source_element_size,
                    source_element_size == 1,
                    true,
                )
            }
            .unwrap(),
        )
    }

    #[test]
    fn shared_string_input_and_scalar_output_arrays_are_safe_and_authoritative() {
        STRING_ARRAY_CALLS.store(0, Ordering::Relaxed);
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table)
            .add_input_string_array(StringEncoding::Utf16, 1)
            .unwrap()
            .add_in(Type::winrt(table.u32_type()))
            .add_caller_output_buffer(
                Type::winrt(table.i32_type()),
                1,
                None,
                BufferCountUnit::Elements,
                false,
            )
            .unwrap();
        let names = Value::Buffer(
            ComBufferValue::string_array(
                vec!["First".into(), "Second".into()],
                StringEncoding::Utf16,
            )
            .unwrap(),
        );
        let outputs = Value::Buffer(
            ComBufferValue::caller_output(&Type::winrt(table.i32_type()), 2).unwrap(),
        );
        let values = invoke_test_buffer(
            resolve_string_ids as *mut c_void,
            signature.clone(),
            &[names.clone(), outputs],
        )
        .unwrap();
        assert_eq!(STRING_ARRAY_CALLS.load(Ordering::Relaxed), 1);
        let Value::Buffer(output) = &values[0] else {
            panic!("caller output must produce an owned scalar array");
        };
        let values = output
            .bytes()
            .unwrap()
            .chunks_exact(size_of::<i32>())
            .map(|bytes| i32::from_ne_bytes(bytes.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(values, [17, 29]);

        let failed_output = Value::Buffer(
            ComBufferValue::caller_output(&Type::winrt(table.i32_type()), 2).unwrap(),
        );
        let error = invoke_test_buffer(
            resolve_string_ids_then_fail as *mut c_void,
            signature.clone(),
            &[
                Value::Buffer(
                    ComBufferValue::string_array(
                        vec!["First".into(), "Second".into()],
                        StringEncoding::Utf16,
                    )
                    .unwrap(),
                ),
                failed_output,
            ],
        )
        .unwrap_err();
        assert!(matches!(error, result::Error::WindowsError(_)));
        assert_eq!(STRING_ARRAY_CALLS.load(Ordering::Relaxed), 2);

        let short_output = Value::Buffer(
            ComBufferValue::caller_output(&Type::winrt(table.i32_type()), 1).unwrap(),
        );
        let error = invoke_test_buffer(
            resolve_string_ids as *mut c_void,
            signature,
            &[names, short_output],
        )
        .unwrap_err();
        assert!(error.message().contains("different lengths"));
        assert_eq!(STRING_ARRAY_CALLS.load(Ordering::Relaxed), 2);

        assert!(
            ComBufferValue::string_array(vec!["embedded\0nul".into()], StringEncoding::Utf16)
                .unwrap_err()
                .message()
                .contains("embedded NUL")
        );
        assert!(
            ComBufferValue::string_array(vec!["caf\u{e9}".into()], StringEncoding::Ansi)
                .unwrap_err()
                .message()
                .contains("ASCII")
        );
        assert_eq!(STRING_ARRAY_CALLS.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn parallel_input_and_output_buffers_can_share_an_authoritative_count() {
        let table = MetadataTable::new();
        MethodSignature::new(&table)
            .add_input_buffer(
                Type::winrt(table.u8_type()),
                1,
                None,
                BufferCountUnit::Elements,
            )
            .unwrap()
            .add_in(Type::winrt(table.u32_type()))
            .add_caller_output_buffer(
                Type::winrt(table.i32_type()),
                1,
                None,
                BufferCountUnit::Elements,
                false,
            )
            .unwrap()
            .build(0)
            .unwrap();
        assert_eq!(
            count_from_value(&Value::WinRt(WinRTValue::I32(3))).unwrap(),
            3
        );
        assert!(
            count_from_value(&Value::WinRt(WinRTValue::I32(-1)))
                .unwrap_err()
                .message()
                .contains("negative")
        );
    }

    #[test]
    fn caller_output_storage_cannot_be_aliased_across_parameters() {
        STRING_ARRAY_CALLS.store(0, Ordering::Relaxed);
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table)
            .add_caller_output_buffer(
                Type::winrt(table.i32_type()),
                2,
                None,
                BufferCountUnit::Elements,
                false,
            )
            .unwrap()
            .add_caller_output_buffer(
                Type::winrt(table.i32_type()),
                3,
                None,
                BufferCountUnit::Elements,
                false,
            )
            .unwrap()
            .add_in(Type::winrt(table.u32_type()))
            .add_in(Type::winrt(table.u32_type()));
        let output = Value::Buffer(
            ComBufferValue::caller_output(&Type::winrt(table.i32_type()), 2).unwrap(),
        );
        let error = invoke_test_buffer(
            resolve_string_ids as *mut c_void,
            signature,
            &[output.clone(), output],
        )
        .unwrap_err();
        assert!(error.message().contains("cannot be aliased"));
        assert_eq!(STRING_ARRAY_CALLS.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn enumerator_next_preserves_partial_success_and_validates_optional_fetched() {
        ENUMERATOR_NEXT_CALLS.store(0, Ordering::Relaxed);
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table)
            .add_in(Type::winrt(table.u32_type()))
            .add_enumerator_next_buffer(Type::winrt(table.u32_type()), 0, 2)
            .unwrap()
            .add_out(Type::winrt(table.u32_type()))
            .preserve_enumerator_next_hresult();
        let interface = register_interface(
            &table,
            "ITestEnumU32",
            GUID::from_u128(0x11111111_2222_3333_4444_555555555555),
            InterfaceBase::IUnknown,
        )
        .add_method_at(3, "Next", signature)
        .unwrap();
        let mut vtable = [std::ptr::null_mut(); 4];
        vtable[3] = enum_next_partial_u32 as *mut c_void;
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };
        let output = ComBufferValue::enumerator_output(&Type::winrt(table.u32_type()), 2).unwrap();
        let values = unsafe {
            interface
                .method(3)
                .unwrap()
                .invoke_values_with_output_kinds(
                    (&mut object as *mut FakeComObject).cast(),
                    &[Value::Buffer(output)],
                )
        }
        .unwrap();
        assert!(matches!(
            values[0].0,
            Value::WinRt(WinRTValue::HResult(value)) if value.0 == 1
        ));
        let Value::Buffer(buffer) = &values[1].0 else {
            panic!("IEnum::Next must return its fetched values");
        };
        assert_eq!(buffer.count(), 1);
        assert_eq!(buffer.bytes(), Some(41u32.to_ne_bytes().as_slice()));
        let optional = MethodSignature::new(&table)
            .add_in(Type::winrt(table.u32_type()))
            .add_enumerator_next_buffer(Type::winrt(table.u32_type()), 0, 2)
            .unwrap()
            .add_optional_out(Type::winrt(table.u32_type()))
            .preserve_enumerator_next_hresult();
        let interface = register_interface(
            &table,
            "ITestEnumOptional",
            GUID::from_u128(0xaaaaaaaa_bbbb_cccc_dddd_eeeeeeeeeeee),
            InterfaceBase::IUnknown,
        )
        .add_method_at(3, "Next", optional)
        .unwrap();
        OPTIONAL_ENUMERATOR_NEXT_CALLS.store(0, Ordering::Relaxed);
        let optional_vtable = [
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            optional_enum_next_should_not_run as *mut c_void,
        ];
        let mut optional_object = FakeComObject {
            vtable: optional_vtable.as_ptr(),
        };
        let output = ComBufferValue::enumerator_output(&Type::winrt(table.u32_type()), 2).unwrap();
        let error = unsafe {
            interface
                .method(3)
                .unwrap()
                .invoke_values_with_output_kinds(
                    (&mut optional_object as *mut FakeComObject).cast(),
                    &[Value::Buffer(output), Value::WinRt(WinRTValue::Bool(false))],
                )
        }
        .unwrap_err();
        assert!(error.message().contains("null pceltFetched"));
        assert_eq!(OPTIONAL_ENUMERATOR_NEXT_CALLS.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn enumerator_next_interface_cleanup_is_bounded_and_exactly_once() {
        ENUMERATOR_NEXT_CALLS.store(0, Ordering::Relaxed);
        let table = MetadataTable::new();
        let element_iid = GUID::from_u128(0xbbbbbbbb_cccc_dddd_eeee_ffffffffffff);
        let element_type = Type::winrt(table.interface(element_iid));
        let signature = MethodSignature::new(&table)
            .add_in(Type::winrt(table.u32_type()))
            .add_enumerator_next_buffer(element_type.clone(), 0, 2)
            .unwrap()
            .add_out(Type::winrt(table.u32_type()))
            .preserve_enumerator_next_hresult();
        let interface = register_interface(
            &table,
            "ITestEnumInterface",
            GUID::from_u128(0x22222222_3333_4444_5555_666666666666),
            InterfaceBase::IUnknown,
        )
        .add_method_at(3, "Next", signature)
        .unwrap();
        let first = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let second = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let mut vtable = [std::ptr::null_mut(); 4];
        vtable[3] = enum_next_interfaces as *mut c_void;
        let mut call = EnumInterfaceCall {
            vtable: vtable.as_ptr(),
            values: [
                (&first as *const TrackedComObject).cast_mut().cast(),
                (&second as *const TrackedComObject).cast_mut().cast(),
            ],
            fetched: 1,
            hresult: windows_core::HRESULT(0),
        };
        let output = ComBufferValue::enumerator_output(&element_type, 2).unwrap();
        let mut values = unsafe {
            interface
                .method(3)
                .unwrap()
                .invoke_values_with_output_kinds(
                    (&mut call as *mut EnumInterfaceCall).cast(),
                    &[Value::Buffer(output)],
                )
        }
        .unwrap();
        assert_eq!(first.releases.load(Ordering::Relaxed), 0);
        assert_eq!(second.releases.load(Ordering::Relaxed), 1);
        let Value::Buffer(buffer) = values.pop().unwrap().0 else {
            panic!("interface enumerator must return a managed array");
        };
        let adopted = buffer.into_com_values().unwrap();
        assert_eq!(adopted.len(), 1);
        drop(adopted);
        assert_eq!(first.releases.load(Ordering::Relaxed), 1);

        call.fetched = 3;
        call.hresult = windows_core::HRESULT(0);
        let output = ComBufferValue::enumerator_output(&element_type, 2).unwrap();
        let error = unsafe {
            interface
                .method(3)
                .unwrap()
                .invoke_values_with_output_kinds(
                    (&mut call as *mut EnumInterfaceCall).cast(),
                    &[Value::Buffer(output)],
                )
        }
        .unwrap_err();
        assert!(error.message().contains("exceeds requested capacity"));
        assert_eq!(first.releases.load(Ordering::Relaxed), 2);
        assert_eq!(second.releases.load(Ordering::Relaxed), 2);

        call.fetched = 1;
        call.hresult = windows_core::HRESULT(0x80004005u32 as i32);
        let output = ComBufferValue::enumerator_output(&element_type, 2).unwrap();
        assert!(
            unsafe {
                interface
                    .method(3)
                    .unwrap()
                    .invoke_values_with_output_kinds(
                        (&mut call as *mut EnumInterfaceCall).cast(),
                        &[Value::Buffer(output)],
                    )
            }
            .is_err()
        );
        assert_eq!(first.releases.load(Ordering::Relaxed), 3);
        assert_eq!(second.releases.load(Ordering::Relaxed), 3);

        call.values[1] = std::ptr::null_mut();
        call.fetched = 2;
        call.hresult = windows_core::HRESULT(0);
        let output = ComBufferValue::enumerator_output(&element_type, 2).unwrap();
        let error = unsafe {
            interface
                .method(3)
                .unwrap()
                .invoke_values_with_output_kinds(
                    (&mut call as *mut EnumInterfaceCall).cast(),
                    &[Value::Buffer(output)],
                )
        }
        .unwrap_err();
        assert!(error.message().contains("null interface pointer"));
        assert_eq!(first.releases.load(Ordering::Relaxed), 4);
        assert_eq!(second.releases.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn owning_variant_arrays_are_transactional_and_clear_each_initialized_slot_once() {
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table)
            .add_caller_output_buffer(
                Type::variant(),
                1,
                Some(2),
                BufferCountUnit::Elements,
                false,
            )
            .unwrap()
            .add_in(Type::winrt(table.u32_type()))
            .add_out(Type::winrt(table.u32_type()));
        let first = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let second = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let vtable = [fill_variant_array as *mut c_void];
        let mut call = OwningVariantArrayCall {
            vtable: vtable.as_ptr(),
            values: [
                (&first as *const TrackedComObject).cast_mut().cast(),
                (&second as *const TrackedComObject).cast_mut().cast(),
            ],
            actual: 1,
            hresult: windows_core::HRESULT(0),
            invalid_slot: -1,
        };
        let invoke = |call: &mut OwningVariantArrayCall| {
            let output =
                ComBufferValue::caller_output(&Type::variant(), 2).expect("VARIANT output");
            signature.clone().build(0).unwrap().plan.invoke_values(
                (call as *mut OwningVariantArrayCall).cast(),
                &[Value::Buffer(output)],
            )
        };

        let mut values = invoke(&mut call).unwrap();
        assert_eq!(first.addrefs.load(Ordering::Relaxed), 2);
        assert_eq!(second.addrefs.load(Ordering::Relaxed), 2);
        assert_eq!(first.releases.load(Ordering::Relaxed), 1);
        assert_eq!(second.releases.load(Ordering::Relaxed), 2);
        let Value::Buffer(output) = values.pop().unwrap() else {
            panic!("VARIANT output must be an owned array");
        };
        let variants = output.into_variants().unwrap();
        assert_eq!(variants.len(), 1);
        drop(variants);
        assert_eq!(first.releases.load(Ordering::Relaxed), 2);

        call.actual = 3;
        let error = invoke(&mut call).unwrap_err();
        assert!(error.message().contains("exceeds capacity"));
        assert_eq!(first.releases.load(Ordering::Relaxed), 4);
        assert_eq!(second.releases.load(Ordering::Relaxed), 4);

        call.actual = 1;
        call.hresult = windows_core::HRESULT(0x80004005u32 as i32);
        assert!(invoke(&mut call).is_err());
        assert_eq!(first.releases.load(Ordering::Relaxed), 6);
        assert_eq!(second.releases.load(Ordering::Relaxed), 6);

        call.hresult = windows_core::HRESULT(0);
        call.invalid_slot = 0;
        let error = invoke(&mut call).unwrap_err();
        assert!(error.message().contains("VARIANT BYREF"));
        assert_eq!(first.releases.load(Ordering::Relaxed), 8);
        assert_eq!(second.releases.load(Ordering::Relaxed), 8);
    }

    #[test]
    fn owning_bstr_and_ienumstring_arrays_preserve_strings_and_initialized_ranges() {
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table)
            .add_caller_output_buffer(Type::bstr(), 1, Some(2), BufferCountUnit::Elements, false)
            .unwrap()
            .add_in(Type::winrt(table.u32_type()))
            .add_out(Type::winrt(table.u32_type()));
        let vtable = [fill_string_array as *mut c_void];
        let mut call = OwningStringArrayCall {
            vtable: vtable.as_ptr(),
            actual: 1,
            hresult: windows_core::HRESULT(0),
            co_task_mem: false,
        };
        let output = ComBufferValue::caller_output(&Type::bstr(), 2).unwrap();
        let mut values = signature
            .build(0)
            .unwrap()
            .plan
            .invoke_values(
                (&mut call as *mut OwningStringArrayCall).cast(),
                &[Value::Buffer(output)],
            )
            .unwrap();
        let Value::Buffer(output) = values.pop().unwrap() else {
            panic!("BSTR output must be an owned string array");
        };
        assert_eq!(output.into_strings().unwrap(), ["embedded\0nul"]);

        let enum_signature = MethodSignature::new(&table)
            .add_in(Type::winrt(table.u32_type()))
            .add_enumerator_next_buffer(Type::co_task_mem_wide_string(), 0, 2)
            .unwrap()
            .add_optional_out(Type::winrt(table.u32_type()))
            .preserve_enumerator_next_hresult();
        let interface = register_interface(
            &table,
            "IEnumString",
            GUID::from_u128(0x00000101_0000_0000_c000_000000000046),
            InterfaceBase::IUnknown,
        )
        .add_method_at(3, "Next", enum_signature)
        .unwrap();
        let mut enum_vtable = [std::ptr::null_mut(); 4];
        enum_vtable[3] = enum_next_string_array as *mut c_void;
        call.vtable = enum_vtable.as_ptr();
        call.actual = 1;
        call.hresult = windows_core::HRESULT(1);
        call.co_task_mem = true;
        let output =
            ComBufferValue::enumerator_output(&Type::co_task_mem_wide_string(), 2).unwrap();
        let mut values = unsafe {
            interface
                .method(3)
                .unwrap()
                .invoke_values_with_output_kinds(
                    (&mut call as *mut OwningStringArrayCall).cast(),
                    &[Value::Buffer(output), Value::WinRt(WinRTValue::Bool(true))],
                )
        }
        .unwrap();
        assert!(matches!(
            values[0].0,
            Value::WinRt(WinRTValue::HResult(value)) if value.0 == 1
        ));
        let Value::Buffer(output) = values.pop().unwrap().0 else {
            panic!("IEnumString output must be an owned string array");
        };
        assert_eq!(output.into_strings().unwrap(), ["embedded"]);
    }

    #[test]
    fn owning_input_arrays_keep_exact_borrowed_resources_alive_for_the_call() {
        let table = MetadataTable::new();
        let bstr_signature = MethodSignature::new(&table)
            .add_input_buffer(Type::bstr(), 1, None, BufferCountUnit::Elements)
            .unwrap()
            .add_in(Type::winrt(table.u32_type()));
        invoke_test_buffer(
            observe_bstr_array as *mut c_void,
            bstr_signature,
            &[Value::Buffer(ComBufferValue::bstr_array(vec![
                "embedded\0nul".into(),
                String::new(),
            ]))],
        )
        .unwrap();

        let variant_signature = MethodSignature::new(&table)
            .add_input_buffer(Type::variant(), 1, None, BufferCountUnit::Elements)
            .unwrap()
            .add_in(Type::winrt(table.u32_type()));
        invoke_test_buffer(
            observe_variant_array as *mut c_void,
            variant_signature,
            &[Value::Buffer(
                ComBufferValue::variant_array(vec![
                    VariantValue::from_i32(17),
                    VariantValue::from_bstr("embedded\0nul").unwrap(),
                ])
                .unwrap(),
            )],
        )
        .unwrap();

        let object = TrackedComObject {
            vtable: &TRACKED_DISPATCH_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let source = std::mem::ManuallyDrop::new(unsafe {
            IUnknown::from_raw(
                (&object as *const TrackedComObject)
                    .cast_mut()
                    .cast::<c_void>(),
            )
        });
        let array = ComBufferValue::interface_array(
            GUID::from_u128(0xaaaaaaaa_bbbb_cccc_dddd_eeeeeeeeeeee),
            vec![(&*source).clone()],
        )
        .unwrap();
        assert_eq!(object.addrefs.load(Ordering::Relaxed), 2);
        drop(array);
        assert_eq!(object.releases.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn enumerator_next_accepts_exact_owning_elements_and_rejects_unproven_shapes() {
        let table = MetadataTable::new();
        assert!(
            MethodSignature::new(&table)
                .add_in(Type::winrt(table.u32_type()))
                .add_enumerator_next_buffer(Type::bstr_pointer(), 0, 2)
                .unwrap_err()
                .message()
                .contains("pointer buffer elements")
        );
        MethodSignature::new(&table)
            .add_in(Type::winrt(table.u32_type()))
            .add_enumerator_next_buffer(Type::bstr(), 0, 2)
            .unwrap();
        MethodSignature::new(&table)
            .add_in(Type::winrt(table.u32_type()))
            .add_enumerator_next_buffer(Type::variant(), 0, 2)
            .unwrap();
        MethodSignature::new(&table)
            .add_caller_output_buffer(Type::bstr(), 1, None, BufferCountUnit::Elements, false)
            .unwrap()
            .add_in(Type::winrt(table.u32_type()))
            .build(0)
            .unwrap();
        MethodSignature::new(&table)
            .add_in(Type::winrt(table.u32_type()))
            .add_enumerator_next_buffer(Type::co_task_mem_wide_string(), 0, 2)
            .unwrap();
        assert!(
            MethodSignature::new(&table)
                .add_in(Type::winrt(table.u32_type()))
                .add_enumerator_next_buffer(Type::co_task_mem_pointer(), 0, 2)
                .unwrap_err()
                .message()
                .contains("pointer buffer elements")
        );
        let nonstandard = MethodSignature::new(&table)
            .add_in(Type::winrt(table.u32_type()))
            .add_enumerator_next_buffer(Type::winrt(table.guid_type()), 0, 2)
            .unwrap()
            .add_out(Type::winrt(table.u32_type()))
            .preserve_enumerator_next_hresult();
        assert!(
            register_interface(
                &table,
                "ITestEnumNonstandard",
                GUID::from_u128(0x33333333_4444_5555_6666_777777777777),
                InterfaceBase::IUnknown,
            )
            .add_method_at(4, "Next", nonstandard)
            .unwrap_err()
            .message()
            .contains("exact IEnum*::Next ABI shape")
        );
        let exact_slot_four = MethodSignature::new(&table)
            .add_in(Type::winrt(table.u32_type()))
            .add_enumerator_next_buffer(Type::winrt(table.guid_type()), 0, 2)
            .unwrap()
            .add_out(Type::winrt(table.u32_type()))
            .preserve_enumerator_next_hresult_at(4);
        register_interface(
            &table,
            "IEnumExactSlotFour",
            GUID::from_u128(0x44444444_5555_6666_7777_888888888888),
            InterfaceBase::IUnknown,
        )
        .add_method_at(4, "Next", exact_slot_four)
        .unwrap();
    }

    #[test]
    fn counted_buffer_runtime_handles_actual_failure_sizing_and_cotaskmem() {
        let table = MetadataTable::new();
        let caller_signature = MethodSignature::new(&table)
            .add_caller_output_buffer(
                Type::winrt(table.u8_type()),
                1,
                Some(2),
                BufferCountUnit::Bytes,
                false,
            )
            .unwrap()
            .add_in(Type::winrt(table.u32_type()))
            .add_out(Type::winrt(table.u32_type()));

        let mut storage = [0xCCu8; 8];
        let values = invoke_test_buffer(
            read_counted_bytes as *mut c_void,
            caller_signature.clone(),
            &[borrowed_buffer(&mut storage, 1)],
        )
        .unwrap();
        let Value::Buffer(output) = &values[0] else {
            panic!("caller output must synthesize an owned buffer");
        };
        assert_eq!(output.bytes(), Some([4, 5, 6].as_slice()));
        assert_eq!(output.count(), 3);

        let mut failed_storage = [0xCCu8; 4];
        let error = invoke_test_buffer(
            read_counted_bytes_then_fail as *mut c_void,
            caller_signature.clone(),
            &[borrowed_buffer(&mut failed_storage, 1)],
        )
        .unwrap_err();
        assert!(matches!(error, result::Error::WindowsError(_)));
        assert_eq!(failed_storage, [9, 0, 0, 0]);

        let mut short_storage = [0u8; 2];
        assert!(
            invoke_test_buffer(
                report_larger_count as *mut c_void,
                caller_signature,
                &[borrowed_buffer(&mut short_storage, 1)],
            )
            .unwrap_err()
            .message()
            .contains("exceeds capacity")
        );

        let two_call_signature = MethodSignature::new(&table)
            .add_caller_output_buffer(
                Type::winrt(table.u8_type()),
                1,
                Some(2),
                BufferCountUnit::Bytes,
                true,
            )
            .unwrap()
            .add_in(Type::winrt(table.u32_type()))
            .add_out(Type::winrt(table.u32_type()));
        let mut sizing_storage = [0u8; 2];
        let sizing = invoke_test_buffer(
            report_larger_count as *mut c_void,
            two_call_signature,
            &[borrowed_buffer(&mut sizing_storage, 1)],
        )
        .unwrap();
        let Value::Buffer(sizing) = &sizing[0] else {
            panic!("two-call sizing must return a buffer result");
        };
        assert_eq!(sizing.bytes(), Some([7, 0].as_slice()));
        assert_eq!(sizing.count(), 4);

        let tracked = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let vtable = [report_larger_count_with_object as *mut c_void];
        let mut call = FailingOutCall {
            vtable: vtable.as_ptr(),
            output: (&tracked as *const TrackedComObject).cast_mut().cast(),
        };
        let post_call_failure_signature = MethodSignature::new(&table)
            .add_caller_output_buffer(
                Type::winrt(table.u8_type()),
                1,
                Some(2),
                BufferCountUnit::Bytes,
                false,
            )
            .unwrap()
            .add_in(Type::winrt(table.u32_type()))
            .add_out(Type::winrt(table.u32_type()))
            .add_out(Type::owned_com_pointer());
        let mut post_call_storage = [0u8; 2];
        let error = post_call_failure_signature
            .build(0)
            .unwrap()
            .plan
            .invoke_values(
                (&mut call as *mut FailingOutCall).cast(),
                &[borrowed_buffer(&mut post_call_storage, 1)],
            )
            .unwrap_err();
        assert!(error.message().contains("exceeds capacity"));
        assert_eq!(tracked.releases.load(Ordering::Relaxed), 1);

        let direct_tracked = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let direct_vtable = [return_object_with_larger_count as *mut c_void];
        let mut direct_call = FailingOutCall {
            vtable: direct_vtable.as_ptr(),
            output: (&direct_tracked as *const TrackedComObject)
                .cast_mut()
                .cast(),
        };
        let direct_failure_signature = MethodSignature::new(&table)
            .add_caller_output_buffer(
                Type::winrt(table.u8_type()),
                1,
                Some(2),
                BufferCountUnit::Bytes,
                false,
            )
            .unwrap()
            .add_in(Type::winrt(table.u32_type()))
            .add_out(Type::winrt(table.u32_type()))
            .returns(Type::owned_com_pointer());
        let mut direct_storage = [0u8; 2];
        let error = direct_failure_signature
            .build(0)
            .unwrap()
            .plan
            .invoke_values(
                (&mut direct_call as *mut FailingOutCall).cast(),
                &[borrowed_buffer(&mut direct_storage, 1)],
            )
            .unwrap_err();
        assert!(error.message().contains("exceeds capacity"));
        assert_eq!(direct_tracked.releases.load(Ordering::Relaxed), 1);

        let callee_signature = MethodSignature::new(&table)
            .add_callee_allocated_buffer(
                Type::winrt(table.u8_type()),
                1,
                BufferCountUnit::Bytes,
                BufferAllocator::CoTaskMem,
            )
            .unwrap()
            .add_out(Type::winrt(table.u32_type()));
        let allocated =
            invoke_test_buffer(return_cotaskmem_bytes as *mut c_void, callee_signature, &[])
                .unwrap();
        let Value::Buffer(allocated) = &allocated[0] else {
            panic!("callee allocation must synthesize an owned buffer");
        };
        assert_eq!(allocated.bytes(), Some([10, 20, 30, 40].as_slice()));
        assert_eq!(allocated.count(), 4);
    }

    #[test]
    fn fixed_capacity_byte_output_uses_exclusive_zeroed_storage() {
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table)
            .add_in(Type::pointer())
            .add_caller_output_buffer(
                Type::winrt(table.u8_type()),
                2,
                Some(3),
                BufferCountUnit::Bytes,
                false,
            )
            .unwrap()
            .add_in(Type::winrt(table.u32_type()))
            .add_out(Type::winrt(table.u32_type()));
        let guid = GUID::from_u128(0x11111111_2222_3333_4444_555555555555);
        let input_guid = Value::WinRt(WinRTValue::RawPtr((&guid as *const GUID).cast_mut().cast()));

        let output =
            Value::Buffer(ComBufferValue::caller_output(&Type::winrt(table.u8_type()), 4).unwrap());
        let values = invoke_test_buffer(
            get_blob_fixed_capacity as *mut c_void,
            signature.clone(),
            &[input_guid.clone(), output],
        )
        .unwrap();
        let Value::Buffer(bytes) = &values[0] else {
            panic!("fixed-capacity output must return an owned byte buffer");
        };
        assert_eq!(bytes.bytes(), Some([11, 22, 33].as_slice()));
        assert_eq!(bytes.count(), 3);

        let too_small =
            Value::Buffer(ComBufferValue::caller_output(&Type::winrt(table.u8_type()), 2).unwrap());
        assert!(matches!(
            invoke_test_buffer(
                get_blob_fixed_capacity as *mut c_void,
                signature.clone(),
                &[input_guid.clone(), too_small],
            ),
            Err(result::Error::WindowsError(_))
        ));

        let failed =
            Value::Buffer(ComBufferValue::caller_output(&Type::winrt(table.u8_type()), 4).unwrap());
        assert!(matches!(
            invoke_test_buffer(
                get_blob_then_fail as *mut c_void,
                signature,
                &[input_guid, failed],
            ),
            Err(result::Error::WindowsError(_))
        ));
    }

    #[test]
    fn empty_buffers_use_non_null_storage_and_slice_lengths_are_bounded() {
        let empty =
            unsafe { ComBufferValue::borrowed(std::ptr::null_mut(), 0, 1, true, false).unwrap() };
        let (ptr, byte_len, ..) = empty.borrowed_parts().unwrap();
        assert!(!ptr.is_null());
        assert_eq!(byte_len, 0);
        let null = ComBufferValue::null();
        let (ptr, byte_len, ..) = null.borrowed_parts().unwrap();
        assert!(ptr.is_null());
        assert_eq!(byte_len, 0);

        let table = MetadataTable::new();
        let element = BufferElementPlan::from_type(&Type::winrt(table.u8_type())).unwrap();
        assert!(
            count_bytes(usize::MAX, &element, BufferCountUnit::Bytes)
                .unwrap_err()
                .message()
                .contains("projected Buffer size")
        );
    }

    #[test]
    fn one_in_out_count_can_be_capacity_and_actual_length() {
        let table = MetadataTable::new();
        MethodSignature::new(&table)
            .add_caller_output_buffer(
                Type::winrt(table.u8_type()),
                1,
                Some(1),
                BufferCountUnit::Bytes,
                false,
            )
            .unwrap()
            .add_in_out(Type::winrt(table.u32_type()))
            .build(0)
            .unwrap();
    }

    #[test]
    fn counted_buffer_runtime_rejects_width_alignment_and_owned_elements() {
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table)
            .add_input_buffer(
                Type::winrt(table.u32_type()),
                1,
                None,
                BufferCountUnit::Elements,
            )
            .unwrap()
            .add_in(Type::winrt(table.u32_type()));

        let mut wrong_width = [0u8; 8];
        let value = Value::Buffer(
            unsafe {
                ComBufferValue::borrowed(
                    wrong_width.as_mut_ptr(),
                    wrong_width.len(),
                    2,
                    false,
                    true,
                )
            }
            .unwrap(),
        );
        assert!(
            invoke_test_buffer(
                read_counted_bytes as *mut c_void,
                signature.clone(),
                &[value]
            )
            .unwrap_err()
            .message()
            .contains("element width mismatch")
        );

        let mut partial_element = [0u8; 6];
        let value = Value::Buffer(
            unsafe {
                ComBufferValue::borrowed(
                    partial_element.as_mut_ptr(),
                    partial_element.len(),
                    1,
                    true,
                    true,
                )
            }
            .unwrap(),
        );
        assert!(
            invoke_test_buffer(
                read_counted_bytes as *mut c_void,
                signature.clone(),
                &[value]
            )
            .unwrap_err()
            .message()
            .contains("not a multiple")
        );

        let mut unaligned = [0u8; 9];
        let offset = (0..=1)
            .find(|offset| (unaligned.as_ptr() as usize + offset) % 4 != 0)
            .expect("one adjacent byte address must be unaligned");
        let value = Value::Buffer(
            unsafe {
                ComBufferValue::borrowed(unaligned.as_mut_ptr().add(offset), 8, 4, false, true)
            }
            .unwrap(),
        );
        assert!(
            invoke_test_buffer(read_counted_bytes as *mut c_void, signature, &[value])
                .unwrap_err()
                .message()
                .contains("not aligned")
        );

        assert!(
            MethodSignature::new(&table)
                .add_input_buffer(Type::pointer(), 1, None, BufferCountUnit::Elements)
                .unwrap_err()
                .message()
                .contains("explicit element ownership")
        );
        let layout = test_pod_layout("BufferPod");
        let element = BufferElementPlan::from_type(&Type::native_struct(layout.clone())).unwrap();
        let mut raw = vec![0u8; layout.size()];
        let unbranded = unsafe {
            ComBufferValue::borrowed(raw.as_mut_ptr(), raw.len(), 1, true, false).unwrap()
        };
        assert!(
            prepare_borrowed_buffer(&unbranded, &element, false)
                .unwrap_err()
                .message()
                .contains("layout identity")
        );
        let branded = ComBufferValue::native_struct_input(raw, &layout).unwrap();
        assert_eq!(branded.count(), 1);
        prepare_borrowed_buffer(&branded, &element, false).unwrap();

        let borrowed =
            unsafe { ComBufferValue::borrowed(unaligned.as_mut_ptr(), 8, 4, false, false) }
                .unwrap();
        assert_eq!(borrowed.count(), 2);
    }

    #[test]
    fn native_pod_runtime_supports_value_pointer_out_and_in_out_storage() {
        let table = MetadataTable::new();
        let layout = test_pod_layout("TestPod");
        let input = test_pod_value(
            layout.clone(),
            TestPod {
                first: 10,
                second: 20,
                tag: 12,
            },
        );

        let by_value = invoke_test_pod(
            sum_pod_by_value as *mut c_void,
            MethodSignature::new(&table)
                .add_in(Type::native_struct(layout.clone()))
                .add_out(Type::winrt(table.u32_type())),
            &[Value::NativeStruct(input.clone())],
        )
        .unwrap();
        assert!(matches!(&by_value[0], Value::WinRt(WinRTValue::U32(42))));

        let pointer = invoke_test_pod(
            sum_pod_pointer as *mut c_void,
            MethodSignature::new(&table)
                .add_in(Type::native_struct_pointer(layout.clone()))
                .add_out(Type::winrt(table.u32_type())),
            &[Value::NativeStruct(input.clone())],
        )
        .unwrap();
        assert!(matches!(&pointer[0], Value::WinRt(WinRTValue::U32(42))));

        let nullable = invoke_test_pod(
            nullable_pod_pointer_is_null as *mut c_void,
            MethodSignature::new(&table)
                .add_in(Type::nullable_native_struct_pointer(layout.clone()))
                .add_out(Type::winrt(table.u32_type())),
            &[Value::WinRt(WinRTValue::Null)],
        )
        .unwrap();
        assert!(matches!(&nullable[0], Value::WinRt(WinRTValue::U32(1))));

        let aligned_layout = Arc::new(
            NativeStructLayout::new(
                "AlignedPod",
                size_of::<AlignedPod>(),
                align_of::<AlignedPod>(),
                vec![
                    NativeStructField::new(
                        "value",
                        0,
                        1,
                        NativeStructFieldType::Scalar(NativeStructScalar::U64),
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        );
        invoke_test_pod(
            require_aligned_pod as *mut c_void,
            MethodSignature::new(&table)
                .add_in(Type::native_struct_pointer(aligned_layout.clone())),
            &[Value::NativeStruct(
                NativeStructValue::new(aligned_layout, 42u64.to_ne_bytes().to_vec()).unwrap(),
            )],
        )
        .unwrap();

        let output = invoke_test_pod(
            write_zeroed_pod as *mut c_void,
            MethodSignature::new(&table).add_out(Type::native_struct(layout.clone())),
            &[],
        )
        .unwrap();
        let Value::NativeStruct(output) = &output[0] else {
            panic!("expected native POD output");
        };
        let output = read_test_pod(output);
        assert_eq!((output.first, output.second, output.tag), (40, 2, 7));

        let in_out_layout = layout.clone();
        let in_out = invoke_test_pod(
            update_pod_in_out as *mut c_void,
            MethodSignature::new(&table).add_in_out(Type::native_struct(layout.clone())),
            &[Value::NativeStruct(input.clone())],
        )
        .unwrap();
        let Value::NativeStruct(in_out) = &in_out[0] else {
            panic!("expected native POD in/out result");
        };
        let in_out = read_test_pod(in_out);
        assert_eq!((in_out.first, in_out.second, in_out.tag), (11, 22, 15));

        let nullable_in_out = invoke_test_pod(
            update_nullable_pod_in_out as *mut c_void,
            MethodSignature::new(&table)
                .add_in_out(Type::nullable_native_struct_pointer(in_out_layout)),
            &[Value::NativeStruct(input)],
        )
        .unwrap();
        let Value::NativeStruct(nullable_in_out) = &nullable_in_out[0] else {
            panic!("expected nullable native POD in/out result");
        };
        let nullable_in_out = read_test_pod(nullable_in_out);
        assert_eq!(
            (
                nullable_in_out.first,
                nullable_in_out.second,
                nullable_in_out.tag
            ),
            (11, 22, 15)
        );

        let nullable_null = invoke_test_pod(
            update_nullable_pod_in_out as *mut c_void,
            MethodSignature::new(&table).add_in_out(Type::nullable_native_struct_pointer(layout)),
            &[Value::WinRt(WinRTValue::Null)],
        )
        .unwrap();
        assert!(matches!(&nullable_null[0], Value::WinRt(WinRTValue::Null)));
    }

    #[test]
    fn native_pod_runtime_rejects_wrong_identity_and_size() {
        let table = MetadataTable::new();
        let expected = test_pod_layout("ExpectedPod");
        let wrong = test_pod_layout("WrongPod");
        assert!(NativeStructValue::new(expected.clone(), vec![0; expected.size() - 1]).is_err());
        let error = invoke_test_pod(
            sum_pod_by_value as *mut c_void,
            MethodSignature::new(&table)
                .add_in(Type::native_struct(expected))
                .add_out(Type::winrt(table.u32_type())),
            &[Value::NativeStruct(NativeStructValue::zeroed(wrong))],
        )
        .unwrap_err();
        assert!(error.message().contains("type mismatch"));
    }

    #[test]
    fn native_union_runtime_requires_brand_and_active_field() {
        let table = MetadataTable::new();
        let layout = test_union_layout();
        let value =
            NativeUnionValue::new(layout.clone(), "integer", 42u64.to_ne_bytes().to_vec()).unwrap();
        let output = invoke_test_pod(
            read_native_union as *mut c_void,
            MethodSignature::new(&table)
                .add_in(Type::native_union_pointer(layout.clone()))
                .add_out(Type::winrt(table.u64_type())),
            &[Value::NativeUnion(value)],
        )
        .unwrap();
        assert!(matches!(
            output.as_slice(),
            [Value::WinRt(WinRTValue::U64(42))]
        ));
        assert!(NativeUnionValue::zeroed(layout.clone(), "missing").is_err());
        assert!(
            MethodSignature::new(&table)
                .add_out(Type::native_union_pointer(layout))
                .build(0)
                .unwrap_err()
                .message()
                .contains("active-field")
        );
    }

    #[test]
    fn automation_values_roundtrip_through_fake_com_vtables() {
        let table = MetadataTable::new();

        let variant = VariantValue::from_bstr("automation").unwrap();
        let output = invoke_test_pod(
            copy_variant_value as *mut c_void,
            MethodSignature::new(&table)
                .add_in(Type::variant())
                .add_out(Type::variant()),
            &[Value::Variant(variant)],
        )
        .unwrap();
        let Value::Variant(output) = &output[0] else {
            panic!("expected VARIANT output");
        };
        assert!(matches!(
            output.data().unwrap(),
            VariantData::Bstr(value) if value == "automation"
        ));

        let array = SafeArrayValue::new(
            SafeArrayElementType::I32,
            vec![SafeArrayBound::new(-1, 2).unwrap()],
            vec![
                SafeArrayElementValue::I32(10),
                SafeArrayElementValue::I32(20),
            ],
        )
        .unwrap();
        invoke_test_pod(
            observe_aliased_safe_arrays as *mut c_void,
            MethodSignature::new(&table)
                .add_in(Type::typed_safe_array(SafeArrayElementType::I32))
                .add_in(Type::typed_safe_array(SafeArrayElementType::I32)),
            &[
                Value::SafeArray(array.clone()),
                Value::SafeArray(array.clone()),
            ],
        )
        .unwrap();
        let output = invoke_test_pod(
            copy_safe_array_value as *mut c_void,
            MethodSignature::new(&table)
                .add_in(Type::typed_safe_array(SafeArrayElementType::I32))
                .add_out(Type::typed_safe_array(SafeArrayElementType::I32)),
            &[Value::SafeArray(array)],
        )
        .unwrap();
        let Value::SafeArray(output) = &output[0] else {
            panic!("expected SAFEARRAY output");
        };
        assert_eq!(output.bounds(), &[SafeArrayBound::new(-1, 2).unwrap()]);
        assert!(matches!(
            output.elements().unwrap().as_slice(),
            [
                SafeArrayElementValue::I32(10),
                SafeArrayElementValue::I32(20)
            ]
        ));

        let propvariant = PropVariantValue::from_vector(PropVariantVector::String(vec![
            "first".into(),
            "second".into(),
        ]))
        .unwrap();
        let output = invoke_test_pod(
            copy_prop_variant_value as *mut c_void,
            MethodSignature::new(&table)
                .add_in(Type::prop_variant())
                .add_out(Type::prop_variant()),
            &[Value::PropVariant(propvariant)],
        )
        .unwrap();
        let Value::PropVariant(output) = &output[0] else {
            panic!("expected PROPVARIANT output");
        };
        assert!(matches!(
            output.data().unwrap(),
            PropVariantData::Vector(PropVariantVector::String(values))
                if values == ["first", "second"]
        ));
    }

    #[test]
    fn variant_by_value_abi_matches_windows_and_preserves_owned_inputs() {
        use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0};

        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(size_of::<VARIANT>(), 24);
            assert_eq!(align_of::<VARIANT>(), 8);
        }
        #[cfg(target_pointer_width = "32")]
        {
            assert_eq!(size_of::<VARIANT>(), 16);
            assert_eq!(align_of::<VARIANT>(), 8);
        }
        assert_eq!(size_of::<VARIANT_0>(), size_of::<VARIANT>());
        assert_eq!(size_of::<VARIANT_0_0>(), size_of::<VARIANT>());
        assert_eq!(size_of::<VARIANT_0_0_0>(), size_of::<VARIANT>() - 8);

        let ffi = ParameterType::variant_by_value().libffi_type();
        let pointer = libffi::middle::Type::pointer();
        let result = libffi::middle::Type::i32();
        let mut argument_types = [pointer.as_raw_ptr(), ffi.as_raw_ptr()];
        let mut cif = libffi::low::ffi_cif::default();
        #[cfg(all(windows, target_arch = "x86"))]
        let abi = libffi_sys::ffi_abi_FFI_STDCALL;
        #[cfg(not(all(windows, target_arch = "x86")))]
        let abi = libffi::middle::ffi_abi_FFI_DEFAULT_ABI;
        unsafe {
            libffi::low::prep_cif(
                &mut cif,
                abi,
                argument_types.len(),
                result.as_raw_ptr(),
                argument_types.as_mut_ptr(),
            )
            .unwrap()
        };
        let raw = unsafe { &*ffi.as_raw_ptr() };
        assert_eq!(raw.size, size_of::<VARIANT>());
        assert_eq!(usize::from(raw.alignment), align_of::<VARIANT>());

        let table = MetadataTable::new();
        let signature = || {
            MethodSignature::new(&table)
                .add_in(Type::variant_by_value())
                .add_out(Type::winrt(table.u32_type()))
        };

        let scalar = invoke_test_pod(
            observe_variant_by_value_i32 as *mut c_void,
            MethodSignature::new(&table)
                .add_in(Type::variant_by_value())
                .add_out(Type::winrt(table.i32_type())),
            &[Value::Variant(VariantValue::from_i32(42))],
        )
        .unwrap();
        assert!(matches!(
            scalar.as_slice(),
            [Value::WinRt(WinRTValue::I32(42))]
        ));

        let bstr = VariantValue::from_bstr("by value").unwrap();
        let bstr_result = invoke_test_pod(
            observe_variant_by_value_bstr as *mut c_void,
            signature(),
            &[Value::Variant(bstr.clone())],
        )
        .unwrap();
        assert!(matches!(
            bstr_result.as_slice(),
            [Value::WinRt(WinRTValue::U32(1))]
        ));
        assert!(matches!(
            bstr.data().unwrap(),
            VariantData::Bstr(value) if value == "by value"
        ));

        let tracked = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let raw = (&tracked as *const TrackedComObject).cast_mut().cast();
        let borrowed = unsafe { IUnknown::from_raw_borrowed(&raw) }.unwrap();
        let interface = VariantValue::from_unknown(Some(borrowed));
        assert_eq!(tracked.addrefs.load(Ordering::Relaxed), 1);
        let interface_result = invoke_test_pod(
            observe_variant_by_value_unknown as *mut c_void,
            signature(),
            &[Value::Variant(interface.clone())],
        )
        .unwrap();
        assert!(matches!(
            interface_result.as_slice(),
            [Value::WinRt(WinRTValue::U32(1))]
        ));
        assert_eq!(tracked.addrefs.load(Ordering::Relaxed), 2);
        assert_eq!(tracked.releases.load(Ordering::Relaxed), 1);

        let error = invoke_test_pod(
            mutate_variant_by_value_then_fail as *mut c_void,
            MethodSignature::new(&table).add_in(Type::variant_by_value()),
            &[Value::Variant(interface.clone())],
        )
        .unwrap_err();
        assert!(error.message().contains("80004005"));
        assert_eq!(tracked.addrefs.load(Ordering::Relaxed), 3);
        assert_eq!(tracked.releases.load(Ordering::Relaxed), 2);
        assert!(matches!(
            interface.data().unwrap(),
            VariantData::Unknown(Some(_))
        ));
        assert_eq!(tracked.addrefs.load(Ordering::Relaxed), 4);
        assert_eq!(tracked.releases.load(Ordering::Relaxed), 3);

        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _copy = automation::VariantCopyValue::new(&interface).unwrap();
            assert_eq!(tracked.addrefs.load(Ordering::Relaxed), 5);
            panic!("exercise panic-safe VARIANT cleanup");
        }));
        assert!(unwind.is_err());
        assert_eq!(tracked.releases.load(Ordering::Relaxed), 4);

        drop(interface);
        assert_eq!(tracked.releases.load(Ordering::Relaxed), 5);

        VARIANT_BY_VALUE_DISPATCH_CALLS.store(0, Ordering::Relaxed);
        let by_value_signature = || MethodSignature::new(&table).add_in(Type::variant_by_value());
        let error = invoke_test_pod(
            count_variant_by_value_dispatch as *mut c_void,
            by_value_signature(),
            &[Value::WinRt(WinRTValue::I32(1))],
        )
        .unwrap_err();
        assert!(error.message().contains("expected by-value VARIANT"));

        let unsupported = VariantValue::from_i32(1);
        unsafe {
            automation::set_variant_vartype_for_test(
                unsupported.raw_mut(),
                windows::Win32::System::Variant::VT_DATE.0,
            )
        };
        let error = invoke_test_pod(
            count_variant_by_value_dispatch as *mut c_void,
            by_value_signature(),
            &[Value::Variant(unsupported)],
        )
        .unwrap_err();
        assert!(error.message().contains("unsupported VARIANT VARTYPE"));

        let byref = VariantValue::from_i32(1);
        unsafe {
            automation::set_variant_vartype_for_test(
                byref.raw_mut(),
                windows::Win32::System::Variant::VT_I4.0
                    | windows::Win32::System::Variant::VT_BYREF.0,
            )
        };
        let error = invoke_test_pod(
            count_variant_by_value_dispatch as *mut c_void,
            by_value_signature(),
            &[Value::Variant(byref)],
        )
        .unwrap_err();
        assert!(error.message().contains("VARIANT BYREF"));
        assert_eq!(VARIANT_BY_VALUE_DISPATCH_CALLS.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn dispatch_params_and_optional_invoke_outputs_roundtrip() {
        let table = MetadataTable::new();
        let params = DispatchParamsValue::new(
            &[
                VariantValue::from_i32(10),
                VariantValue::from_i32(20),
                VariantValue::from_i32(30),
            ],
            &[100, 200],
        )
        .unwrap();
        let signature = MethodSignature::new(&table)
            .add_in(Type::dispatch_params())
            .add_optional_out(Type::variant())
            .add_optional_out(Type::excep_info())
            .add_optional_out(Type::winrt(table.u32_type()));

        DISPATCH_OPTIONAL_NULL_MASK.store(u32::MAX, Ordering::Relaxed);
        let output = invoke_test_pod(
            inspect_dispatch_params_and_fill_outputs as *mut c_void,
            signature.clone(),
            &[
                Value::DispatchParams(params.clone()),
                Value::WinRt(WinRTValue::Bool(true)),
                Value::WinRt(WinRTValue::Bool(true)),
                Value::WinRt(WinRTValue::Bool(true)),
            ],
        )
        .unwrap();
        assert_eq!(DISPATCH_OPTIONAL_NULL_MASK.load(Ordering::Relaxed), 0);
        assert_eq!(output.len(), 3);
        assert!(matches!(
            output[0],
            Value::Variant(ref value)
                if matches!(value.data().unwrap(), VariantData::I32(30))
        ));
        let Value::ExcepInfo(info) = &output[1] else {
            panic!("expected EXCEPINFO output");
        };
        assert_eq!(info.code(), 17);
        assert_eq!(info.source(), Some("source"));
        assert_eq!(info.description(), Some("description"));
        assert_eq!(info.help_file(), Some("help.chm"));
        assert_eq!(info.help_context(), 91);
        assert_eq!(info.scode(), 0x80020009u32 as i32);
        assert!(matches!(output[2], Value::WinRt(WinRTValue::U32(2))));

        let output = invoke_test_pod(
            inspect_dispatch_params_and_fill_outputs as *mut c_void,
            signature,
            &[
                Value::DispatchParams(params),
                Value::WinRt(WinRTValue::Bool(false)),
                Value::WinRt(WinRTValue::Bool(false)),
                Value::WinRt(WinRTValue::Bool(false)),
            ],
        )
        .unwrap();
        assert_eq!(DISPATCH_OPTIONAL_NULL_MASK.load(Ordering::Relaxed), 0b111);
        assert!(
            output
                .iter()
                .all(|value| matches!(value, Value::WinRt(WinRTValue::Null)))
        );
    }

    #[test]
    fn dispatch_invoke_captures_failure_only_outputs_and_success_result() {
        let table = MetadataTable::new();

        DISPATCH_DEFERRED_SUCCESS_CALLS.store(0, Ordering::Relaxed);
        let (method, vtable) = dispatch_method(&table, dispatch_invoke_exception);
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };
        let failure = unsafe {
            method.invoke_dispatch(
                (&mut object as *mut FakeComObject).cast(),
                &dispatch_invoke_args(true, true, true),
            )
        }
        .unwrap();
        assert_eq!(failure.hresult(), DISP_E_EXCEPTION);
        assert!(failure.result().is_none());
        let info = failure.excep_info().expect("deferred EXCEPINFO");
        assert_eq!(info.code(), 23);
        assert_eq!(info.source(), Some("deferred source"));
        assert_eq!(info.description(), Some("deferred description"));
        assert_eq!(info.help_file(), Some("deferred help"));
        assert_eq!(info.help_context(), 42);
        assert_eq!(info.scode(), DISP_E_EXCEPTION.0);
        assert!(failure.arg_err().is_none());
        assert!(failure.finalization_error().is_none());
        assert_eq!(DISPATCH_DEFERRED_SUCCESS_CALLS.load(Ordering::Relaxed), 1);

        let (method, vtable) = dispatch_method(&table, dispatch_invoke_type_mismatch);
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };
        let failure = unsafe {
            method.invoke_dispatch(
                (&mut object as *mut FakeComObject).cast(),
                &dispatch_invoke_args(true, true, true),
            )
        }
        .unwrap();
        assert_eq!(failure.hresult(), DISP_E_TYPEMISMATCH);
        assert!(failure.result().is_none());
        assert!(failure.excep_info().is_none());
        assert_eq!(failure.arg_err(), Some(3));

        let (method, vtable) = dispatch_method(&table, dispatch_invoke_fail);
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };
        let failure = unsafe {
            method.invoke_dispatch(
                (&mut object as *mut FakeComObject).cast(),
                &dispatch_invoke_args(true, true, true),
            )
        }
        .unwrap();
        assert_eq!(
            failure.hresult(),
            windows_core::HRESULT(0x80004005u32 as i32)
        );
        assert!(failure.result().is_none());
        assert!(failure.excep_info().is_none());
        assert!(failure.arg_err().is_none());

        DISPATCH_DEFERRED_SUCCESS_CALLS.store(0, Ordering::Relaxed);
        let (method, vtable) = dispatch_method(&table, dispatch_invoke_success);
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };
        let success = unsafe {
            method.invoke_dispatch(
                (&mut object as *mut FakeComObject).cast(),
                &dispatch_invoke_args(true, true, true),
            )
        }
        .unwrap();
        assert!(success.hresult().is_ok());
        assert!(matches!(
            success.result().unwrap().data().unwrap(),
            VariantData::I32(42)
        ));
        assert!(success.excep_info().is_none());
        assert!(success.arg_err().is_none());
        assert_eq!(DISPATCH_DEFERRED_SUCCESS_CALLS.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn dispatch_invoke_cleans_failed_result_and_preserves_deferred_failure_cause() {
        let table = MetadataTable::new();
        let tracked = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let (method, vtable) = dispatch_method(&table, dispatch_invoke_partial_result_then_fail);
        let mut object = DispatchTrackedCall {
            vtable: vtable.as_ptr(),
            output: (&tracked as *const TrackedComObject).cast_mut().cast(),
        };
        let failure = unsafe {
            method.invoke_dispatch(
                (&mut object as *mut DispatchTrackedCall).cast(),
                &dispatch_invoke_args(true, false, false),
            )
        }
        .unwrap();
        assert!(failure.hresult().is_err());
        assert!(failure.result().is_none());
        assert_eq!(tracked.addrefs.load(Ordering::Relaxed), 2);
        assert_eq!(tracked.releases.load(Ordering::Relaxed), 2);

        DISPATCH_DEFERRED_FAILURE_CALLS.store(0, Ordering::Relaxed);
        let (method, vtable) =
            dispatch_method(&table, dispatch_invoke_exception_with_failing_deferred);
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };
        let failure = unsafe {
            method.invoke_dispatch(
                (&mut object as *mut FakeComObject).cast(),
                &dispatch_invoke_args(false, true, false),
            )
        }
        .unwrap();
        assert_eq!(failure.hresult(), DISP_E_EXCEPTION);
        assert!(failure.excep_info().is_none());
        assert!(
            failure
                .finalization_error()
                .unwrap()
                .message()
                .contains("0x80004005")
        );
        assert_eq!(DISPATCH_DEFERRED_FAILURE_CALLS.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn dispatch_invoke_keeps_disabled_outputs_null_and_fails_closed_elsewhere() {
        let table = MetadataTable::new();
        DISPATCH_INVOKE_NULL_MASK.store(0, Ordering::Relaxed);
        let (method, vtable) = dispatch_method(&table, dispatch_invoke_disabled_outputs);
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };
        let success = unsafe {
            method.invoke_dispatch(
                (&mut object as *mut FakeComObject).cast(),
                &dispatch_invoke_args(false, false, false),
            )
        }
        .unwrap();
        assert!(success.hresult().is_ok());
        assert!(success.result().is_none());
        assert!(success.excep_info().is_none());
        assert!(success.arg_err().is_none());
        assert_eq!(DISPATCH_INVOKE_NULL_MASK.load(Ordering::Relaxed), 0b111);

        let not_dispatch = register_interface(
            &table,
            "Tests.INotDispatch",
            GUID::from_u128(0x10000000_0000_0000_0000_000000000010),
            InterfaceBase::IUnknown,
        );
        let error = not_dispatch
            .add_method_at(6, "Invoke", dispatch_invoke_signature(&table))
            .unwrap_err();
        assert!(
            error
                .message()
                .contains("restricted to the exact IDispatch::Invoke")
        );
    }

    #[test]
    fn excep_info_deferred_fill_runs_once_and_cleans_on_every_failure_path() {
        let table = MetadataTable::new();
        DEFERRED_FILL_CALLS.store(0, Ordering::Relaxed);
        let output = invoke_test_pod(
            install_deferred_excep_info as *mut c_void,
            MethodSignature::new(&table).add_out(Type::excep_info()),
            &[],
        )
        .unwrap();
        assert_eq!(DEFERRED_FILL_CALLS.load(Ordering::Relaxed), 1);
        let Value::ExcepInfo(info) = &output[0] else {
            panic!("expected EXCEPINFO output");
        };
        assert_eq!(info.code(), 23);
        assert_eq!(info.source(), Some("deferred source"));
        assert_eq!(info.description(), Some("deferred description"));
        assert_eq!(info.help_file(), Some("deferred help"));
        assert_eq!(info.help_context(), 42);

        DEFERRED_FILL_CALLS.store(0, Ordering::Relaxed);
        let error = invoke_test_pod(
            install_failing_deferred_excep_info as *mut c_void,
            MethodSignature::new(&table).add_out(Type::excep_info()),
            &[],
        )
        .unwrap_err();
        assert!(error.message().contains("0x80004005"));
        assert_eq!(DEFERRED_FILL_CALLS.load(Ordering::Relaxed), 1);

        DEFERRED_FILL_CALLS.store(0, Ordering::Relaxed);
        let error = invoke_test_pod(
            install_reinstalling_deferred_excep_info as *mut c_void,
            MethodSignature::new(&table).add_out(Type::excep_info()),
            &[],
        )
        .unwrap_err();
        assert!(error.message().contains("installed another callback"));
        assert_eq!(DEFERRED_FILL_CALLS.load(Ordering::Relaxed), 1);

        let error = invoke_test_pod(
            write_reserved_excep_info as *mut c_void,
            MethodSignature::new(&table).add_out(Type::excep_info()),
            &[],
        )
        .unwrap_err();
        assert!(error.message().contains("reserved fields"));

        DEFERRED_FILL_CALLS.store(0, Ordering::Relaxed);
        let error = invoke_test_pod(
            install_deferred_excep_info_then_fail as *mut c_void,
            MethodSignature::new(&table).add_out(Type::excep_info()),
            &[],
        )
        .unwrap_err();
        assert!(error.message().contains("0x80004005"));
        assert_eq!(DEFERRED_FILL_CALLS.load(Ordering::Relaxed), 1);

        DEFERRED_FILL_CALLS.store(0, Ordering::Relaxed);
        let error = invoke_test_pod(
            write_unsupported_variant_and_deferred_excep_info as *mut c_void,
            MethodSignature::new(&table)
                .add_out(Type::variant())
                .add_out(Type::excep_info()),
            &[],
        )
        .unwrap_err();
        assert!(error.message().contains("unsupported VARIANT"));
        assert_eq!(DEFERRED_FILL_CALLS.load(Ordering::Relaxed), 1);

        DEFERRED_FILL_CALLS.store(0, Ordering::Relaxed);
        let error = invoke_test_pod(
            write_unsupported_variant_and_deferred_excep_info_then_fail as *mut c_void,
            MethodSignature::new(&table)
                .add_out(Type::variant())
                .add_out(Type::excep_info()),
            &[],
        )
        .unwrap_err();
        assert!(error.message().contains("0x80004005"));
        assert_eq!(DEFERRED_FILL_CALLS.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn dispatch_params_variant_copies_release_exactly_once() {
        let tracked = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let raw = (&tracked as *const TrackedComObject).cast_mut().cast();
        let borrowed = unsafe { IUnknown::from_raw_borrowed(&raw) }.unwrap();
        let variant = VariantValue::from_unknown(Some(borrowed));
        assert_eq!(tracked.addrefs.load(Ordering::Relaxed), 1);

        let params = DispatchParamsValue::new(std::slice::from_ref(&variant), &[]).unwrap();
        assert_eq!(tracked.addrefs.load(Ordering::Relaxed), 2);
        assert!(DispatchParamsValue::new(std::slice::from_ref(&variant), &[1, 2]).is_err());
        assert_eq!(tracked.addrefs.load(Ordering::Relaxed), 2);

        drop(variant);
        assert_eq!(tracked.releases.load(Ordering::Relaxed), 1);
        drop(params);
        assert_eq!(tracked.releases.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn automation_validation_failure_cleans_other_owned_outputs() {
        let tracked = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let vtable = [write_unsupported_variant_and_object as *mut c_void];
        let mut call = FailingOutCall {
            vtable: vtable.as_ptr(),
            output: (&tracked as *const TrackedComObject).cast_mut().cast(),
        };
        let table = MetadataTable::new();
        let error = MethodSignature::new(&table)
            .add_out(Type::variant())
            .add_out(Type::owned_com_pointer())
            .build(0)
            .unwrap()
            .plan
            .invoke_values((&mut call as *mut FailingOutCall).cast(), &[])
            .unwrap_err();

        assert!(error.message().contains("unsupported VARIANT"));
        assert_eq!(tracked.releases.load(Ordering::Relaxed), 1);

        let reversed_tracked = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let reversed_vtable = [write_object_and_unsupported_variant as *mut c_void];
        let mut reversed_call = FailingOutCall {
            vtable: reversed_vtable.as_ptr(),
            output: (&reversed_tracked as *const TrackedComObject)
                .cast_mut()
                .cast(),
        };
        let error = MethodSignature::new(&table)
            .add_out(Type::owned_com_pointer())
            .add_out(Type::variant())
            .build(0)
            .unwrap()
            .plan
            .invoke_values((&mut reversed_call as *mut FailingOutCall).cast(), &[])
            .unwrap_err();
        assert!(error.message().contains("unsupported VARIANT"));
        assert_eq!(reversed_tracked.releases.load(Ordering::Relaxed), 1);

        let direct_tracked = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let direct_vtable = [return_object_with_unsupported_variant as *mut c_void];
        let mut direct_call = FailingOutCall {
            vtable: direct_vtable.as_ptr(),
            output: (&direct_tracked as *const TrackedComObject)
                .cast_mut()
                .cast(),
        };
        let error = MethodSignature::new(&table)
            .add_out(Type::variant())
            .returns(Type::owned_com_pointer())
            .build(0)
            .unwrap()
            .plan
            .invoke_values((&mut direct_call as *mut FailingOutCall).cast(), &[])
            .unwrap_err();
        assert!(error.message().contains("unsupported VARIANT"));
        assert_eq!(direct_tracked.releases.load(Ordering::Relaxed), 1);

        AUTOMATION_DISPATCH_CALLS.store(0, Ordering::Relaxed);
        let preflight_vtable = [count_automation_dispatch as *mut c_void];
        let mut preflight = FakeComObject {
            vtable: preflight_vtable.as_ptr(),
        };
        let error = call_method(
            0,
            (&mut preflight as *mut FakeComObject).cast(),
            MethodSignature::new(&table).add_out(Type::variant()),
            &[],
        )
        .unwrap_err();
        assert!(error.message().contains("COM value invocation path"));
        assert_eq!(AUTOMATION_DISPATCH_CALLS.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn automation_interface_elements_addref_and_release_exactly_once_per_owner() {
        let tracked = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let raw = (&tracked as *const TrackedComObject).cast_mut().cast();
        let borrowed = unsafe { IUnknown::from_raw_borrowed(&raw) }.unwrap();

        {
            let variant = VariantValue::from_unknown(Some(borrowed));
            let data = variant.data().unwrap();
            assert!(matches!(data, VariantData::Unknown(Some(_))));
            drop(data);
            assert_eq!(tracked.addrefs.load(Ordering::Relaxed), 2);
            assert_eq!(tracked.releases.load(Ordering::Relaxed), 1);
        }
        assert_eq!(tracked.releases.load(Ordering::Relaxed), 2);

        {
            let array = SafeArrayValue::new(
                SafeArrayElementType::Unknown,
                vec![SafeArrayBound::new(0, 1).unwrap()],
                vec![SafeArrayElementValue::Unknown(Some(borrowed.clone()))],
            )
            .unwrap();
            drop(array.elements().unwrap());
        }
        assert_eq!(tracked.addrefs.load(Ordering::Relaxed), 5);
        assert_eq!(tracked.releases.load(Ordering::Relaxed), 5);

        let dispatch = TrackedComObject {
            vtable: &TRACKED_DISPATCH_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let raw = (&dispatch as *const TrackedComObject).cast_mut().cast();
        let borrowed = unsafe { IUnknown::from_raw_borrowed(&raw) }.unwrap();
        {
            let variant = VariantValue::from_dispatch(Some(borrowed)).unwrap();
            assert_eq!(dispatch.addrefs.load(Ordering::Relaxed), 1);
            let data = variant.data().unwrap();
            assert!(matches!(data, VariantData::Dispatch(Some(_))));
            drop(data);
            assert_eq!(dispatch.addrefs.load(Ordering::Relaxed), 2);
            assert_eq!(dispatch.releases.load(Ordering::Relaxed), 1);
        }
        assert_eq!(dispatch.addrefs.load(Ordering::Relaxed), 2);
        assert_eq!(dispatch.releases.load(Ordering::Relaxed), 2);

        let typed = TrackedComObject {
            vtable: &TRACKED_DISPATCH_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let raw = (&typed as *const TrackedComObject).cast_mut().cast();
        let borrowed = unsafe { IUnknown::from_raw_borrowed(&raw) }.unwrap();
        let iid = GUID::from_u128(0x5347ad7b_c355_46f8_aff5_909033582f63);
        {
            let array = SafeArrayValue::new_interface(
                iid,
                vec![SafeArrayBound::new(-1, 1).unwrap()],
                vec![SafeArrayElementValue::Unknown(Some(borrowed.clone()))],
            )
            .unwrap();
            assert_eq!(array.interface_iid(), Some(iid));
            drop(array.elements().unwrap());
        }
        assert_eq!(typed.addrefs.load(Ordering::Relaxed), 5);
        assert_eq!(typed.releases.load(Ordering::Relaxed), 5);

        let mismatched = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let raw = (&mismatched as *const TrackedComObject).cast_mut().cast();
        let borrowed = unsafe { IUnknown::from_raw_borrowed(&raw) }.unwrap();
        assert!(
            SafeArrayValue::new_interface(
                iid,
                vec![SafeArrayBound::new(0, 1).unwrap()],
                vec![SafeArrayElementValue::Unknown(Some(borrowed.clone()))],
            )
            .is_err()
        );
        assert_eq!(mismatched.addrefs.load(Ordering::Relaxed), 1);
        assert_eq!(mismatched.releases.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn typed_interface_outputs_validate_generic_descriptor_elements() {
        let expected_iid = GUID::from_u128(0x5347ad7b_c355_46f8_aff5_909033582f63);
        let table = MetadataTable::new();

        for descriptor_iid in [None, Some(IUnknown::IID)] {
            let tracked = TrackedComObject {
                vtable: &TRACKED_DISPATCH_VTABLE,
                addrefs: AtomicU32::new(0),
                releases: AtomicU32::new(0),
            };
            let raw = (&tracked as *const TrackedComObject).cast_mut().cast();
            let borrowed = unsafe { IUnknown::from_raw_borrowed(&raw) }.unwrap();
            let bounds = vec![SafeArrayBound::new(-3, 1).unwrap()];
            let elements = vec![SafeArrayElementValue::Unknown(Some(borrowed.clone()))];
            let array = if let Some(iid) = descriptor_iid {
                SafeArrayValue::new_interface(iid, bounds, elements).unwrap()
            } else {
                SafeArrayValue::new(SafeArrayElementType::Unknown, bounds, elements).unwrap()
            };

            let mut output = invoke_test_pod(
                copy_safe_array_value as *mut c_void,
                MethodSignature::new(&table)
                    .add_in(Type::typed_safe_array(SafeArrayElementType::Unknown))
                    .add_out(Type::typed_interface_safe_array(expected_iid)),
                &[Value::SafeArray(array)],
            )
            .unwrap();
            let Value::SafeArray(output) = output.pop().unwrap() else {
                panic!("expected typed SAFEARRAY output")
            };
            assert_eq!(output.interface_iid(), Some(expected_iid));
            assert_eq!(output.bounds(), [SafeArrayBound::new(-3, 1).unwrap()]);
            drop(output.elements().unwrap());
            drop(output);
            assert_eq!(
                tracked.addrefs.load(Ordering::Relaxed),
                tracked.releases.load(Ordering::Relaxed)
            );
        }

        let mismatched = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let raw = (&mismatched as *const TrackedComObject).cast_mut().cast();
        let borrowed = unsafe { IUnknown::from_raw_borrowed(&raw) }.unwrap();
        let array = SafeArrayValue::new(
            SafeArrayElementType::Unknown,
            vec![SafeArrayBound::new(0, 1).unwrap()],
            vec![SafeArrayElementValue::Unknown(Some(borrowed.clone()))],
        )
        .unwrap();
        let error = invoke_test_pod(
            copy_safe_array_value as *mut c_void,
            MethodSignature::new(&table)
                .add_in(Type::typed_safe_array(SafeArrayElementType::Unknown))
                .add_out(Type::typed_interface_safe_array(expected_iid)),
            &[Value::SafeArray(array)],
        )
        .unwrap_err();
        assert!(error.message().contains("does not support expected IID"));
        assert_eq!(
            mismatched.addrefs.load(Ordering::Relaxed),
            mismatched.releases.load(Ordering::Relaxed)
        );
    }

    #[test]
    fn automation_in_out_and_typed_array_mismatches_fail_before_dispatch() {
        let table = MetadataTable::new();
        assert!(
            MethodSignature::new(&table)
                .add_out(Type::variant_by_value())
                .build(0)
                .unwrap_err()
                .message()
                .contains("by-value VARIANT is input-only")
        );
        assert!(
            MethodSignature::new(&table)
                .add_in_out(Type::variant_by_value())
                .build(0)
                .unwrap_err()
                .message()
                .contains("by-value VARIANT is input-only")
        );
        assert!(
            MethodSignature::new(&table)
                .add_in_out(Type::variant())
                .build(0)
                .unwrap_err()
                .message()
                .contains("BYREF/InOut")
        );
        assert!(
            MethodSignature::new(&table)
                .add_in_out(Type::safe_array())
                .build(0)
                .unwrap_err()
                .message()
                .contains("BYREF/InOut")
        );
        assert!(
            MethodSignature::new(&table)
                .add_in_out(Type::prop_variant())
                .build(0)
                .unwrap_err()
                .message()
                .contains("BYREF/InOut")
        );
        assert!(
            MethodSignature::new(&table)
                .add_out(Type::dispatch_params())
                .build(0)
                .unwrap_err()
                .message()
                .contains("input-only")
        );
        assert!(
            MethodSignature::new(&table)
                .add_in_out(Type::dispatch_params())
                .build(0)
                .unwrap_err()
                .message()
                .contains("input-only")
        );
        assert!(
            MethodSignature::new(&table)
                .add_in(Type::excep_info())
                .build(0)
                .unwrap_err()
                .message()
                .contains("output-only")
        );
        assert!(
            MethodSignature::new(&table)
                .add_in_out(Type::excep_info())
                .build(0)
                .unwrap_err()
                .message()
                .contains("output-only")
        );
        assert!(
            MethodSignature::new(&table)
                .add_out(Type::excep_info())
                .returns_void()
                .build(0)
                .unwrap_err()
                .message()
                .contains("HRESULT")
        );

        let array = SafeArrayValue::new(
            SafeArrayElementType::U32,
            vec![SafeArrayBound::new(0, 1).unwrap()],
            vec![SafeArrayElementValue::U32(1)],
        )
        .unwrap();
        let error = invoke_test_pod(
            copy_safe_array_value as *mut c_void,
            MethodSignature::new(&table)
                .add_in(Type::typed_safe_array(SafeArrayElementType::I32))
                .add_out(Type::typed_safe_array(SafeArrayElementType::I32)),
            &[Value::SafeArray(array)],
        )
        .unwrap_err();
        assert!(error.message().contains("element type mismatch"));

        let array = SafeArrayValue::new(
            SafeArrayElementType::U32,
            vec![SafeArrayBound::new(0, 1).unwrap()],
            vec![SafeArrayElementValue::U32(1)],
        )
        .unwrap();
        let error = invoke_test_pod(
            copy_safe_array_value as *mut c_void,
            MethodSignature::new(&table)
                .add_in(Type::typed_safe_array(SafeArrayElementType::U32))
                .add_out(Type::typed_safe_array(SafeArrayElementType::I32)),
            &[Value::SafeArray(array)],
        )
        .unwrap_err();
        assert!(error.message().contains("VARTYPE mismatch"));

        let actual_iid = GUID::from_u128(0x5347ad7b_c355_46f8_aff5_909033582f63);
        let other_iid = GUID::from_u128(0xd6dd68d1_86fd_4332_8666_9abedea2d24c);
        let array = SafeArrayValue::new_interface(
            actual_iid,
            vec![SafeArrayBound::new(3, 1).unwrap()],
            vec![SafeArrayElementValue::Unknown(None)],
        )
        .unwrap();
        let error = invoke_test_pod(
            copy_safe_array_value as *mut c_void,
            MethodSignature::new(&table)
                .add_in(Type::typed_interface_safe_array(other_iid))
                .add_out(Type::typed_interface_safe_array(other_iid)),
            &[Value::SafeArray(array.clone())],
        )
        .unwrap_err();
        assert!(error.message().contains("interface IID mismatch"));

        let error = invoke_test_pod(
            copy_safe_array_value as *mut c_void,
            MethodSignature::new(&table)
                .add_in(Type::typed_interface_safe_array(actual_iid))
                .add_out(Type::typed_interface_safe_array(other_iid)),
            &[Value::SafeArray(array)],
        )
        .unwrap_err();
        assert!(error.message().contains("interface IID mismatch"));
    }

    #[test]
    fn nullable_safearray_output_accepts_only_a_documented_null_contract() {
        let table = MetadataTable::new();
        let output = invoke_test_pod(
            return_null_safe_array as *mut c_void,
            MethodSignature::new(&table)
                .add_out(Type::nullable_typed_safe_array(SafeArrayElementType::I32)),
            &[],
        )
        .unwrap();
        assert!(matches!(
            output.as_slice(),
            [Value::WinRt(WinRTValue::Null)]
        ));

        let error = invoke_test_pod(
            return_null_safe_array as *mut c_void,
            MethodSignature::new(&table).add_out(Type::typed_safe_array(SafeArrayElementType::I32)),
            &[],
        )
        .unwrap_err();
        assert!(error.message().contains("owned SAFEARRAY output is null"));

        assert!(
            MethodSignature::new(&table)
                .add_in(Type::nullable_typed_safe_array(SafeArrayElementType::I32))
                .build(0)
                .unwrap_err()
                .message()
                .contains("exact documented owned output")
        );
    }

    #[test]
    fn native_pod_failure_discards_unowned_output_storage() {
        let table = MetadataTable::new();
        let error = invoke_test_pod(
            write_pod_then_fail as *mut c_void,
            MethodSignature::new(&table).add_out(Type::native_struct(test_pod_layout("TestPod"))),
            &[],
        )
        .unwrap_err();
        assert!(error.message().contains("0x80004005"));
    }

    #[test]
    fn direct_native_return_is_not_interpreted_as_hresult() {
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table).returns(Type::winrt(table.u32_type()));
        let vtable = [return_u32 as *mut c_void];
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };

        let values = call_method(
            0,
            (&mut object as *mut FakeComObject).cast(),
            signature,
            &[],
        )
        .expect("u32::MAX is a value, not a failed HRESULT");

        assert!(matches!(values.as_slice(), [WinRTValue::U32(u32::MAX)]));
    }

    #[test]
    fn native_void_return_does_not_read_hresult_register() {
        VOID_CALLS.store(0, Ordering::Relaxed);
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table).returns_void();
        let vtable = [return_void as *mut c_void];
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };

        let values = call_method(
            0,
            (&mut object as *mut FakeComObject).cast(),
            signature,
            &[],
        )
        .unwrap();

        assert!(values.is_empty());
        assert_eq!(VOID_CALLS.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn semantic_hresult_preserves_success_codes_and_throws_failures() {
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table).preserve_hresult();
        let success_vtable = [return_s_false as *mut c_void];
        let mut success = FakeComObject {
            vtable: success_vtable.as_ptr(),
        };

        let result = call_method(
            0,
            (&mut success as *mut FakeComObject).cast(),
            signature.clone(),
            &[],
        )
        .unwrap();
        assert!(matches!(
            result.as_slice(),
            [WinRTValue::HResult(value)] if value.0 == 1
        ));

        let failure_vtable = [return_failure as *mut c_void];
        let mut failure = FakeComObject {
            vtable: failure_vtable.as_ptr(),
        };
        let error = call_method(
            0,
            (&mut failure as *mut FakeComObject).cast(),
            signature,
            &[],
        )
        .unwrap_err();
        match error {
            result::Error::WindowsError(error) => {
                assert_eq!(error.code(), windows_core::HRESULT(0x80004005u32 as i32));
            }
            other => panic!("expected Windows error, got {other:?}"),
        }
    }

    #[test]
    fn classic_com_hstring_output_is_owned_and_decoded() {
        let table = MetadataTable::new();
        let vtable = [return_hstring as *mut c_void];
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };

        let result = call_method(
            0,
            (&mut object as *mut FakeComObject).cast(),
            MethodSignature::new(&table).add_out(Type::winrt(table.hstring())),
            &[],
        )
        .unwrap();

        assert!(matches!(
            result.as_slice(),
            [WinRTValue::HString(value)] if value == "dynwinrt HSTRING"
        ));
    }

    #[test]
    fn in_out_parameter_preserves_input_and_returns_updated_value() {
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table).add_in_out(Type::winrt(table.i32_type()));
        let vtable = [increment_i32 as *mut c_void];
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };

        let values = call_method(
            0,
            (&mut object as *mut FakeComObject).cast(),
            signature,
            &[WinRTValue::I32(41)],
        )
        .unwrap();

        assert!(matches!(values.as_slice(), [WinRTValue::I32(42)]));
    }

    #[test]
    fn native_pointer_out_is_not_adopted_as_com_object() {
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table).add_out(Type::pointer());
        let vtable = [write_native_pointer as *mut c_void];
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };

        let values = call_method(
            0,
            (&mut object as *mut FakeComObject).cast(),
            signature,
            &[],
        )
        .unwrap();

        assert!(matches!(
            values.as_slice(),
            [WinRTValue::RawPtr(ptr)] if *ptr == 0x1234usize as *mut c_void
        ));
    }

    #[test]
    fn borrowed_handle_outputs_use_pointer_width_and_never_cleanup() {
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table).add_out(Type::borrowed_handle_output());
        let registered = signature.clone().build(0).unwrap();
        assert!(matches!(
            registered.plan.arguments[0].storage,
            ComArgumentStorage::OutputPointer
        ));
        assert_eq!(
            registered.plan.native.output_cleanup(0),
            OutputCleanup::None
        );
        assert_eq!(
            registered.plan.results[0].success.pointer_output_kind(),
            PointerOutputKind::None
        );

        let success_vtable = [write_native_pointer as *mut c_void];
        let mut success = FakeComObject {
            vtable: success_vtable.as_ptr(),
        };
        let values = registered
            .plan
            .invoke_with_output_kinds((&mut success as *mut FakeComObject).cast(), &[])
            .unwrap();
        assert!(matches!(
            values.as_slice(),
            [(WinRTValue::RawPtr(ptr), PointerOutputKind::None)]
                if *ptr as usize == 0x1234usize
        ));

        let null_vtable = [succeed_without_writing_native_pointer as *mut c_void];
        let mut null = FakeComObject {
            vtable: null_vtable.as_ptr(),
        };
        let values = call_method(
            0,
            (&mut null as *mut FakeComObject).cast(),
            signature.clone(),
            &[],
        )
        .unwrap();
        assert!(matches!(
            values.as_slice(),
            [WinRTValue::RawPtr(ptr)] if ptr.is_null()
        ));

        let tracked = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let failure_vtable = [write_object_then_fail as *mut c_void];
        let mut failure = FailingOutCall {
            vtable: failure_vtable.as_ptr(),
            output: (&tracked as *const TrackedComObject).cast_mut().cast(),
        };
        let error = call_method(
            0,
            (&mut failure as *mut FailingOutCall).cast(),
            signature,
            &[],
        )
        .unwrap_err();
        assert!(error.message().contains("80004005"));
        assert_eq!(tracked.releases.load(Ordering::Relaxed), 0);
        assert_eq!(size_of::<*mut c_void>(), size_of::<usize>());
    }

    #[test]
    fn failing_hresult_releases_written_interface_output_on_direct_path() {
        let tracked = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let vtable = [write_object_then_fail as *mut c_void];
        let mut call = FailingOutCall {
            vtable: vtable.as_ptr(),
            output: (&tracked as *const TrackedComObject).cast_mut().cast(),
        };
        let table = MetadataTable::new();

        let error = call_method(
            0,
            (&mut call as *mut FailingOutCall).cast(),
            MethodSignature::new(&table).add_out(Type::owned_com_pointer()),
            &[],
        )
        .unwrap_err();

        assert!(error.message().contains("80004005"));
        assert_eq!(tracked.releases.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn failing_hresult_releases_written_interface_output_on_libffi_path() {
        let tracked = TrackedComObject {
            vtable: &TRACKED_VTABLE,
            addrefs: AtomicU32::new(0),
            releases: AtomicU32::new(0),
        };
        let vtable = [write_object_and_i32_then_fail as *mut c_void];
        let mut call = FailingOutCall {
            vtable: vtable.as_ptr(),
            output: (&tracked as *const TrackedComObject).cast_mut().cast(),
        };
        let table = MetadataTable::new();

        let error = call_method(
            0,
            (&mut call as *mut FailingOutCall).cast(),
            MethodSignature::new(&table)
                .add_out(Type::owned_com_pointer())
                .add_out(Type::winrt(table.i32_type())),
            &[],
        )
        .unwrap_err();

        assert!(error.message().contains("80004005"));
        assert_eq!(tracked.releases.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn semantic_pointer_outputs_lower_exact_failure_cleanup_plans() {
        let table = MetadataTable::new();
        let cases = [
            (
                Type::owned_com_pointer(),
                OutputCleanup::ComRelease,
                PointerOutputKind::Com,
            ),
            (
                Type::co_task_mem_pointer(),
                OutputCleanup::CoTaskMemFree,
                PointerOutputKind::CoTaskMem,
            ),
            (
                Type::bstr_pointer(),
                OutputCleanup::BstrFree,
                PointerOutputKind::Bstr,
            ),
        ];

        for (typ, expected_cleanup, expected_output) in cases {
            let registered = MethodSignature::new(&table).add_out(typ).build(0).unwrap();
            assert_eq!(registered.plan.native.output_cleanup(0), expected_cleanup);
            assert_eq!(
                registered
                    .plan
                    .results
                    .iter()
                    .map(|result| result.success.pointer_output_kind())
                    .collect::<Vec<_>>(),
                [expected_output]
            );
        }
    }

    #[test]
    fn immutable_call_plan_preserves_argument_and_result_order() {
        let table = MetadataTable::new();
        let registered = MethodSignature::new(&table)
            .add_in(Type::winrt(table.u32_type()))
            .add_out(Type::owned_com_pointer())
            .add_in_out(Type::winrt(table.i32_type()))
            .preserve_hresult()
            .build(7)
            .unwrap();
        let plan = &registered.plan;

        assert!(matches!(plan.return_plan, ComReturnPlan::SemanticHResult));
        assert_eq!(plan.arguments.len(), 3);
        assert_eq!(plan.arguments[0].input_index, Some(0));
        assert_eq!(plan.arguments[0].output_index, None);
        assert_eq!(plan.arguments[1].input_index, None);
        assert_eq!(plan.arguments[1].output_index, Some(0));
        assert_eq!(plan.arguments[2].input_index, Some(1));
        assert_eq!(plan.arguments[2].output_index, Some(1));
        assert_eq!(
            plan.results
                .iter()
                .map(|result| result.source)
                .collect::<Vec<_>>(),
            [
                ComResultSource::DirectReturn,
                ComResultSource::Parameter(1),
                ComResultSource::Parameter(2),
            ]
        );
        assert_eq!(
            plan.results[1].success,
            ComSuccessDisposition::OwnedComPointer
        );
        assert_eq!(plan.results[1].failure_cleanup, OutputCleanup::ComRelease);
    }

    #[test]
    fn multi_output_guid_uses_full_sized_storage() {
        let table = MetadataTable::new();
        let signature = MethodSignature::new(&table)
            .add_out(Type::winrt(table.guid_type()))
            .add_out(Type::winrt(table.i32_type()));
        let vtable = [write_guid_and_i32 as *mut c_void];
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };

        let values = call_method(
            0,
            (&mut object as *mut FakeComObject).cast(),
            signature,
            &[],
        )
        .unwrap();

        assert!(matches!(
            values.as_slice(),
            [WinRTValue::Guid(guid), WinRTValue::I32(42)]
                if *guid == GUID::from_u128(0x11111111_2222_3333_4444_555555555555)
        ));
    }

    const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
    const IID_ISHELL_LINK_W: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
    const REGDB_E_CLASSNOTREG: windows_core::HRESULT = windows_core::HRESULT(0x80040154u32 as i32);

    fn shell_link() -> result::Result<WinRTValue> {
        initialize_apartment(ApartmentType::MultiThreaded)?;
        co_create_instance(CLSID_SHELL_LINK, IID_ISHELL_LINK_W)
    }

    #[test]
    fn interfaces_with_the_same_name_do_not_alias() {
        let table = MetadataTable::new();
        let first = register_interface(
            &table,
            "Windows.Win32.Example.IThing",
            GUID::from_u128(1),
            InterfaceBase::IUnknown,
        );
        let second = register_interface(
            &table,
            "Windows.Win32.Example.IThing",
            GUID::from_u128(2),
            InterfaceBase::IUnknown,
        );

        assert_eq!(first.name(), second.name());
        assert_ne!(first.iid(), second.iid());
        assert!(!Arc::ptr_eq(&first.methods, &second.methods));
    }

    #[test]
    fn duplicate_method_names_preserve_distinct_vtable_slots() {
        let table = MetadataTable::new();
        let iface = register_interface(
            &table,
            "Windows.Win32.Example.IOverloaded",
            GUID::from_u128(3),
            InterfaceBase::IUnknown,
        )
        .add_method("SetValue", MethodSignature::new(&table))
        .add_method("SetValue", MethodSignature::new(&table))
        .add_method("AfterOverload", MethodSignature::new(&table));

        assert!(iface.method(3).is_some());
        assert!(iface.method(4).is_some());
        assert!(iface.method(5).is_some());
    }

    #[test]
    fn explicit_method_registration_rejects_duplicate_slots() {
        let table = MetadataTable::new();
        let iface = register_interface(
            &table,
            "Windows.Win32.Example.IExplicitSlots",
            GUID::from_u128(4),
            InterfaceBase::IUnknown,
        )
        .add_method_at(7, "First", MethodSignature::new(&table))
        .unwrap();

        let error = iface
            .add_method_at(7, "Second", MethodSignature::new(&table))
            .unwrap_err();
        assert!(error.message().contains("slot 7"));
    }

    #[test]
    fn method_results_preserve_pointer_ownership_kind() {
        let table = MetadataTable::new();
        let iface = register_interface(
            &table,
            "Windows.Win32.Example.IOwnedOutput",
            GUID::from_u128(5),
            InterfaceBase::IUnknown,
        )
        .add_method_at(
            3,
            "GetOwned",
            MethodSignature::new(&table).add_out(Type::owned_com_pointer()),
        )
        .unwrap();
        let vtable = [
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            write_native_pointer as *mut c_void,
        ];
        let mut object = FakeComObject {
            vtable: vtable.as_ptr(),
        };

        let results = unsafe {
            iface
                .method(3)
                .unwrap()
                .invoke_with_output_kinds((&mut object as *mut FakeComObject).cast(), &[])
        }
        .unwrap();

        assert!(matches!(
            results.as_slice(),
            [(WinRTValue::RawPtr(ptr), PointerOutputKind::Com)]
                if *ptr == 0x1234usize as *mut c_void
        ));
    }

    fn shell_link_interface(table: &std::sync::Arc<MetadataTable>) -> Interface {
        register_interface(
            table,
            "Windows.Win32.UI.Shell.IShellLinkW",
            IID_ISHELL_LINK_W,
            InterfaceBase::IUnknown,
        )
        .add_method("GetPath", MethodSignature::new(table))
        .add_method("GetIDList", MethodSignature::new(table))
        .add_method("SetIDList", MethodSignature::new(table))
        .add_method("GetDescription", MethodSignature::new(table))
        .add_method("SetDescription", MethodSignature::new(table))
        .add_method("GetWorkingDirectory", MethodSignature::new(table))
        .add_method("SetWorkingDirectory", MethodSignature::new(table))
        .add_method("GetArguments", MethodSignature::new(table))
        .add_method("SetArguments", MethodSignature::new(table))
        .add_method(
            "GetHotkey",
            MethodSignature::new(table).add_out(Type::winrt(table.u16_type())),
        )
        .add_method(
            "SetHotkey",
            MethodSignature::new(table).add_in(Type::winrt(table.u16_type())),
        )
        .add_method(
            "GetShowCmd",
            MethodSignature::new(table).add_out(Type::winrt(table.i32_type())),
        )
        .add_method(
            "SetShowCmd",
            MethodSignature::new(table).add_in(Type::winrt(table.i32_type())),
        )
    }

    fn native_usize_type(table: &std::sync::Arc<MetadataTable>) -> Type {
        #[cfg(target_pointer_width = "64")]
        {
            Type::winrt(table.u64_type())
        }
        #[cfg(target_pointer_width = "32")]
        {
            Type::winrt(table.u32_type())
        }
    }

    fn native_usize_value(value: usize) -> WinRTValue {
        #[cfg(target_pointer_width = "64")]
        {
            WinRTValue::U64(value as u64)
        }
        #[cfg(target_pointer_width = "32")]
        {
            WinRTValue::U32(value as u32)
        }
    }

    fn read_native_usize(value: &WinRTValue) -> usize {
        #[cfg(target_pointer_width = "64")]
        {
            match value {
                WinRTValue::U64(value) => *value as usize,
                value => panic!("expected native u64, got {value:?}"),
            }
        }
        #[cfg(target_pointer_width = "32")]
        {
            match value {
                WinRTValue::U32(value) => *value as usize,
                value => panic!("expected native u32, got {value:?}"),
            }
        }
    }

    #[test]
    fn shell_link_set_get_show_cmd_round_trips_via_classic_com_vtable() -> result::Result<()> {
        let shell_link = shell_link()?.as_object().unwrap();
        let table = MetadataTable::new();
        let iface = shell_link_interface(&table);

        unsafe {
            iface
                .method(15)
                .unwrap()
                .invoke(shell_link.as_raw(), &[WinRTValue::I32(3)])?;
        }
        let result = unsafe { iface.method(14).unwrap().invoke(shell_link.as_raw(), &[]) }?;

        assert_eq!(result[0].as_i32().unwrap(), 3);
        Ok(())
    }

    #[test]
    fn shell_link_set_get_hotkey_round_trips_u16() -> result::Result<()> {
        let shell_link = shell_link()?.as_object().unwrap();
        let table = MetadataTable::new();
        let iface = shell_link_interface(&table);

        unsafe {
            iface
                .method(13)
                .unwrap()
                .invoke(shell_link.as_raw(), &[WinRTValue::U16(0x0141)])?;
        }
        let result = unsafe { iface.method(12).unwrap().invoke(shell_link.as_raw(), &[]) }?;

        assert_eq!(result[0].as_i32().unwrap() as u16, 0x0141);
        Ok(())
    }

    #[test]
    fn shell_link_set_get_description_round_trips_wide_string() -> result::Result<()> {
        let shell_link = shell_link()?.as_object().unwrap();
        let expected = "dynwinrt classic COM";
        let wide = wide_null(expected);

        call_method_1_ptr(7, shell_link.as_raw(), wide.as_ptr() as *const c_void)?;

        let mut buffer = wide_buffer(128);
        call_method_2_ptr_i32(
            6,
            shell_link.as_raw(),
            buffer.as_mut_ptr() as *mut c_void,
            buffer.len() as i32,
        )?;

        assert_eq!(wide_to_string(&buffer), expected);
        Ok(())
    }

    #[test]
    fn shell_link_query_interface_returns_owned_ipersistfile() -> result::Result<()> {
        let shell_link = shell_link()?;
        let shell_link_object = shell_link
            .as_object()
            .expect("IShellLinkW must be non-null");
        let description = wide_null("semantic HRESULT");
        call_method_1_ptr(7, shell_link_object.as_raw(), description.as_ptr().cast())?;
        let persist = shell_link.cast(&IPersistFile::IID)?;
        let persist = persist.as_object().expect("IPersistFile must be non-null");
        let table = MetadataTable::new();
        let result = call_method(
            3,
            persist.as_raw(),
            MethodSignature::new(&table).add_out(Type::winrt(table.guid_type())),
            &[],
        )?;

        assert!(matches!(
            result.as_slice(),
            [WinRTValue::Guid(clsid)] if *clsid == CLSID_SHELL_LINK
        ));
        let dirty = call_method(
            4,
            persist.as_raw(),
            MethodSignature::new(&table).preserve_hresult(),
            &[],
        )?;
        assert!(matches!(
            dirty.as_slice(),
            [WinRTValue::HResult(value)] if value.0 == 0
        ));
        Ok(())
    }

    #[test]
    fn malloc_exercises_pointer_sized_and_non_hresult_abi() -> result::Result<()> {
        initialize_apartment(ApartmentType::MultiThreaded)?;
        let allocator = unsafe { CoGetMalloc(1) }.map_err(result::Error::WindowsError)?;
        let table = MetadataTable::new();
        let requested = 64usize;
        let allocated = call_method(
            3,
            allocator.as_raw(),
            MethodSignature::new(&table)
                .add_in(native_usize_type(&table))
                .returns(Type::pointer()),
            &[native_usize_value(requested)],
        )?;
        let WinRTValue::RawPtr(ptr) = allocated[0] else {
            panic!("IMalloc::Alloc must return a native pointer");
        };
        assert!(!ptr.is_null());

        struct AllocationGuard {
            allocator: IMalloc,
            ptr: *mut c_void,
        }
        impl Drop for AllocationGuard {
            fn drop(&mut self) {
                if !self.ptr.is_null() {
                    unsafe { self.allocator.Free(Some(self.ptr)) };
                }
            }
        }
        let mut allocation = AllocationGuard {
            allocator: allocator.clone(),
            ptr,
        };

        let size = call_method(
            6,
            allocator.as_raw(),
            MethodSignature::new(&table)
                .add_in(Type::pointer())
                .returns(native_usize_type(&table)),
            &[WinRTValue::RawPtr(ptr)],
        )?;
        assert!(read_native_usize(&size[0]) >= requested);

        let owned = call_method(
            7,
            allocator.as_raw(),
            MethodSignature::new(&table)
                .add_in(Type::pointer())
                .returns(Type::winrt(table.i32_type())),
            &[WinRTValue::RawPtr(ptr)],
        )?;
        assert!(matches!(owned.as_slice(), [WinRTValue::I32(value)] if *value != 0));

        let freed = call_method(
            5,
            allocator.as_raw(),
            MethodSignature::new(&table)
                .add_in(Type::pointer())
                .returns_void(),
            &[WinRTValue::RawPtr(ptr)],
        )?;
        allocation.ptr = std::ptr::null_mut();
        assert!(freed.is_empty());

        let minimized = call_method(
            8,
            allocator.as_raw(),
            MethodSignature::new(&table).returns_void(),
            &[],
        )?;
        assert!(minimized.is_empty());
        Ok(())
    }

    #[test]
    fn memory_stream_exercises_counted_buffers_seek_and_interface_out() -> result::Result<()> {
        initialize_apartment(ApartmentType::MultiThreaded)?;
        let expected = b"dynwinrt";
        let stream = unsafe { SHCreateMemStream(Some(expected)) }
            .expect("SHCreateMemStream must return an IStream");
        let table = MetadataTable::new();
        let mut buffer = vec![0u8; expected.len()];

        let read_signature = MethodSignature::new(&table)
            .add_caller_output_buffer(
                Type::winrt(table.u8_type()),
                1,
                Some(2),
                BufferCountUnit::Bytes,
                false,
            )?
            .add_in(Type::winrt(table.u32_type()))
            .add_out(Type::winrt(table.u32_type()))
            .preserve_hresult();
        let read = read_signature
            .clone()
            .build(3)?
            .plan
            .invoke_values(stream.as_raw(), &[borrowed_buffer(&mut buffer, 1)])?;
        assert!(matches!(
            read.as_slice(),
            [Value::WinRt(WinRTValue::HResult(hr)), Value::Buffer(result)]
                if hr.0 == 0
                    && result.bytes() == Some(expected.as_slice())
                    && result.count() == expected.len()
        ));
        assert_eq!(buffer, expected);

        let position = call_method(
            5,
            stream.as_raw(),
            MethodSignature::new(&table)
                .add_in(Type::winrt(table.i64_type()))
                .add_in(Type::winrt(table.u32_type()))
                .add_out(Type::winrt(table.u64_type())),
            &[WinRTValue::I64(0), WinRTValue::U32(0)],
        )?;
        assert!(matches!(position.as_slice(), [WinRTValue::U64(0)]));

        let mut replacement = b"phase8!!".to_vec();
        let write = MethodSignature::new(&table)
            .add_input_buffer(
                Type::winrt(table.u8_type()),
                1,
                Some(2),
                BufferCountUnit::Bytes,
            )?
            .add_in(Type::winrt(table.u32_type()))
            .add_out(Type::winrt(table.u32_type()))
            .preserve_hresult()
            .build(4)?
            .plan
            .invoke_values(stream.as_raw(), &[borrowed_buffer(&mut replacement, 1)])?;
        assert!(matches!(
            write.as_slice(),
            [Value::WinRt(WinRTValue::HResult(hr)), Value::WinRt(WinRTValue::U32(count))]
                if hr.0 == 0 && *count == replacement.len() as u32
        ));

        call_method(
            5,
            stream.as_raw(),
            MethodSignature::new(&table)
                .add_in(Type::winrt(table.i64_type()))
                .add_in(Type::winrt(table.u32_type()))
                .add_out(Type::winrt(table.u64_type())),
            &[WinRTValue::I64(0), WinRTValue::U32(0)],
        )?;
        let mut reread = vec![0u8; replacement.len()];
        let read = read_signature
            .build(3)?
            .plan
            .invoke_values(stream.as_raw(), &[borrowed_buffer(&mut reread, 1)])?;
        assert!(matches!(
            read.as_slice(),
            [Value::WinRt(WinRTValue::HResult(hr)), Value::Buffer(result)]
                if hr.0 == 0 && result.bytes() == Some(replacement.as_slice())
        ));

        let cloned = call_method(
            13,
            stream.as_raw(),
            MethodSignature::new(&table).add_out(Type::winrt(table.object())),
            &[],
        )?;
        let clone = cloned[0].as_object().expect("IStream::Clone returned null");
        let _: IStream = clone.cast().map_err(result::Error::WindowsError)?;
        Ok(())
    }

    #[test]
    fn adopt_com_pointer_accepts_addref_owned_pointer() -> result::Result<()> {
        let shell_link = shell_link()?.as_object().unwrap();
        let shell_link_raw = shell_link.as_raw();
        let borrowed = unsafe { IUnknown::from_raw_borrowed(&shell_link_raw) }.unwrap();
        let addref_owned = borrowed.clone();
        let raw = addref_owned.as_raw();
        std::mem::forget(addref_owned);

        let adopted = unsafe { adopt_com_pointer(raw) };
        let adopted = adopted.as_object().expect("adopted value must be Object");
        let table = MetadataTable::new();
        let iface = shell_link_interface(&table);

        unsafe {
            iface
                .method(15)
                .unwrap()
                .invoke(adopted.as_raw(), &[WinRTValue::I32(7)])?;
        }
        let result = unsafe { iface.method(14).unwrap().invoke(adopted.as_raw(), &[]) }?;

        assert_eq!(result[0].as_i32().unwrap(), 7);
        Ok(())
    }

    #[test]
    fn co_create_instance_with_bogus_clsid_returns_error() -> result::Result<()> {
        initialize_apartment(ApartmentType::MultiThreaded)?;
        let bogus = GUID::from_u128(0xaaaaaaaa_bbbb_cccc_dddd_eeeeeeeeeeee);

        let err = co_create_instance(bogus, IID_ISHELL_LINK_W).unwrap_err();
        match err {
            result::Error::WindowsError(err) => assert_eq!(err.code(), REGDB_E_CLASSNOTREG),
            err => panic!("expected REGDB_E_CLASSNOTREG, got {err:?}"),
        }
        Ok(())
    }

    #[test]
    fn co_create_instance_does_not_choose_an_apartment_implicitly() {
        let remains_uninitialized = std::thread::spawn(|| {
            let _ = co_create_instance(
                GUID::from_u128(0xaaaaaaaa_bbbb_cccc_dddd_eeeeeeeeeeee),
                IID_ISHELL_LINK_W,
            );
            COM_INITIALIZATION
                .with(|state| matches!(*state.borrow(), ComInitialization::Uninitialized))
        })
        .join()
        .unwrap();

        assert!(remains_uninitialized);
    }

    #[test]
    fn query_interface_with_unsupported_iid_returns_error() -> result::Result<()> {
        let shell_link = shell_link()?;
        let bogus = GUID::from_u128(0xbbbbbbbb_cccc_dddd_eeee_ffffffffffff);

        let err = shell_link.cast(&bogus).unwrap_err();
        match err {
            result::Error::WindowsError(err) => assert_eq!(err.code(), E_NOINTERFACE),
            err => panic!("expected E_NOINTERFACE, got {err:?}"),
        }
        Ok(())
    }

    #[test]
    fn data_transfer_manager_interop_get_for_window_returns_winrt_object_via_dynamic_iunknown_vtable()
    -> result::Result<()> {
        initialize_apartment(ApartmentType::MultiThreaded)?;

        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("dynwinrt data transfer interop test"),
                WS_OVERLAPPED,
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
        }
        .map_err(result::Error::WindowsError)?;
        struct WindowGuard(windows::Win32::Foundation::HWND);
        impl Drop for WindowGuard {
            fn drop(&mut self) {
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }
        let _window = WindowGuard(hwnd);

        let factory = ro_get_activation_factory_2(&HSTRING::from(
            "Windows.ApplicationModel.DataTransfer.DataTransferManager",
        ))?;
        let interop = query_interface(factory, &IDataTransferManagerInterop::IID)
            .map_err(result::Error::WindowsError)?
            .as_object()
            .unwrap();

        let table = MetadataTable::new();
        let iface = register_interface(
            &table,
            "IDataTransferManagerInterop",
            IDataTransferManagerInterop::IID,
            InterfaceBase::IUnknown,
        )
        .add_method(
            "GetForWindow",
            MethodSignature::new(&table)
                .add_in(Type::pointer())
                .add_in(Type::pointer())
                .add_out(Type::winrt(table.object())),
        );

        let target_iid = DataTransferManager::IID;
        let result = unsafe {
            iface.method(3).unwrap().invoke(
                interop.as_raw(),
                &[
                    WinRTValue::RawPtr(hwnd.0 as *mut c_void),
                    WinRTValue::RawPtr(&target_iid as *const GUID as *mut c_void),
                ],
            )
        }?;

        let manager = result[0].as_object().expect("GetForWindow returned null");
        assert!(!manager.as_raw().is_null());
        let _typed: DataTransferManager = manager.cast().map_err(result::Error::WindowsError)?;
        Ok(())
    }
}
