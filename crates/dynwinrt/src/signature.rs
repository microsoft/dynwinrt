// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use libffi::middle::Cif;
use std::sync::Arc;
use windows::core::{GUID, HSTRING, IInspectable, Interface};

use crate::{
    abi::{AbiType, AbiValue},
    call,
    call::ArgumentList,
    metadata_table::{MetadataTable, TypeHandle, TypeKind},
    value::WinRTValue,
};

#[derive(Debug, Clone)]
pub(crate) enum ParameterType {
    WinRT(TypeHandle),
    Pointer,
}

impl ParameterType {
    pub(crate) fn winrt(typ: TypeHandle) -> Self {
        Self::WinRT(typ)
    }

    pub(crate) fn pointer() -> Self {
        Self::Pointer
    }

    pub(crate) fn as_winrt(&self) -> Option<&TypeHandle> {
        match self {
            Self::WinRT(typ) => Some(typ),
            Self::Pointer => None,
        }
    }

    pub(crate) fn is_array(&self) -> bool {
        self.as_winrt().is_some_and(TypeHandle::is_array)
    }

    pub(crate) fn is_struct(&self) -> bool {
        matches!(self, Self::WinRT(typ) if matches!(typ.kind(), TypeKind::Struct(_)))
    }

    pub(crate) fn is_hstring(&self) -> bool {
        matches!(self, Self::WinRT(typ) if matches!(typ.kind(), TypeKind::HString))
    }

    pub(crate) fn is_u32(&self) -> bool {
        matches!(self, Self::WinRT(typ) if matches!(typ.kind(), TypeKind::U32))
    }

    pub(crate) fn is_guid(&self) -> bool {
        matches!(self, Self::WinRT(typ) if matches!(typ.kind(), TypeKind::Guid))
    }

    pub(crate) fn supports_in_out(&self) -> bool {
        matches!(self, Self::Pointer)
            || matches!(
                self,
                Self::WinRT(typ)
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
                            | TypeKind::Struct(_)
                    )
            )
    }

    pub(crate) fn supports_direct_return(&self) -> bool {
        matches!(self, Self::Pointer)
            || matches!(
                self,
                Self::WinRT(typ)
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

    pub(crate) fn abi_type(&self) -> AbiType {
        match self {
            Self::WinRT(typ) => typ.abi_type(),
            Self::Pointer => AbiType::Ptr,
        }
    }

    pub(crate) fn libffi_type(&self) -> libffi::middle::Type {
        match self {
            Self::WinRT(typ) => typ.libffi_type(),
            Self::Pointer => libffi::middle::Type::pointer(),
        }
    }

    pub(crate) fn array_element_type(&self) -> TypeHandle {
        self.as_winrt()
            .expect("native pointer is not an array")
            .array_element_type()
    }

    pub(crate) fn default_struct_value(&self) -> crate::metadata_table::ValueTypeData {
        self.as_winrt()
            .expect("native pointer is not a struct")
            .default_value()
    }

    pub(crate) fn default_value(&self) -> WinRTValue {
        match self {
            Self::WinRT(typ) => typ.default_winrt_value(),
            Self::Pointer => WinRTValue::RawPtr(std::ptr::null_mut()),
        }
    }

    pub(crate) fn from_out(&self, ptr: *mut std::ffi::c_void) -> crate::result::Result<WinRTValue> {
        match self {
            Self::WinRT(typ) => typ.from_out(ptr),
            Self::Pointer => Ok(WinRTValue::RawPtr(ptr)),
        }
    }

    pub(crate) fn from_out_value(&self, value: &AbiValue) -> crate::result::Result<WinRTValue> {
        match (self, value) {
            (Self::WinRT(typ), value) => typ.from_out_value(value),
            (Self::Pointer, AbiValue::Pointer(ptr)) => Ok(WinRTValue::RawPtr(*ptr)),
            (Self::Pointer, value) => Err(crate::result::Error::InvalidTypeAbiToWinRT(
                TypeKind::Object,
                value.abi_type(),
            )),
        }
    }
}

/// How a parameter is passed at the ABI level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    In,
    Out,
    InOut,
    /// FillArray: caller allocates buffer, callee fills it.
    /// ABI expands to 2 params: (u32 capacity, T* items).
    OutFillArray,
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub(crate) typ: ParameterType,
    /// Index in the method result vector for out and FillArray parameters.
    pub value_index: usize,
    /// Index in the caller-provided argument slice. FillArray parameters have
    /// both an input index (capacity buffer) and an output index (filled data).
    pub input_index: Option<usize>,
    pub kind: ParamKind,
}

impl Parameter {
    pub fn is_input(&self) -> bool {
        matches!(self.kind, ParamKind::In | ParamKind::InOut)
    }

    pub fn is_out(&self) -> bool {
        matches!(
            self.kind,
            ParamKind::Out | ParamKind::InOut | ParamKind::OutFillArray
        )
    }

    pub fn is_in_out(&self) -> bool {
        self.kind == ParamKind::InOut
    }

    pub fn is_fill_array(&self) -> bool {
        self.kind == ParamKind::OutFillArray
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AbiMethodSignature {
    out_count: usize,
    input_count: usize,
    parameters: Vec<Parameter>,
    return_kind: MethodReturn,
    #[allow(dead_code)]
    is_opaque: bool,
    #[allow(dead_code)]
    table: Arc<MetadataTable>,
}

#[derive(Debug, Clone)]
pub(crate) enum MethodReturn {
    HResult,
    Void,
    Value(ParameterType),
}

impl MethodReturn {
    fn libffi_type(&self) -> libffi::middle::Type {
        match self {
            Self::HResult => libffi::middle::Type::i32(),
            Self::Void => libffi::middle::Type::void(),
            Self::Value(typ) => typ.libffi_type(),
        }
    }
}

impl AbiMethodSignature {
    pub(crate) fn new(table: &Arc<MetadataTable>) -> Self {
        AbiMethodSignature {
            out_count: 0,
            input_count: 0,
            parameters: Vec::new(),
            return_kind: MethodReturn::HResult,
            is_opaque: false,
            table: Arc::clone(table),
        }
    }

    pub(crate) fn add_in_type(mut self, typ: ParameterType) -> Self {
        let input_index = self.input_count;
        self.input_count += 1;
        self.parameters.push(Parameter {
            kind: ParamKind::In,
            typ,
            value_index: input_index,
            input_index: Some(input_index),
        });
        self
    }

    pub(crate) fn add_out_type(mut self, typ: ParameterType) -> Self {
        self.parameters.push(Parameter {
            kind: ParamKind::Out,
            typ,
            value_index: self.out_count,
            input_index: None,
        });
        self.out_count += 1;
        self
    }

    pub(crate) fn add_in_out_type(mut self, typ: ParameterType) -> Self {
        assert!(
            typ.supports_in_out(),
            "in/out currently supports native scalars, pointers, enums, and structs"
        );
        let input_index = self.input_count;
        self.input_count += 1;
        self.parameters.push(Parameter {
            kind: ParamKind::InOut,
            typ,
            value_index: self.out_count,
            input_index: Some(input_index),
        });
        self.out_count += 1;
        self
    }

    pub(crate) fn add_out_fill_type(mut self, typ: ParameterType) -> Self {
        let input_index = self.input_count;
        self.input_count += 1;
        self.parameters.push(Parameter {
            kind: ParamKind::OutFillArray,
            typ,
            value_index: self.out_count,
            input_index: Some(input_index),
        });
        self.out_count += 1;
        self
    }

    pub(crate) fn returns_type(mut self, typ: ParameterType) -> Self {
        assert!(
            typ.supports_direct_return(),
            "direct native returns currently support scalars, enums, and pointers"
        );
        self.return_kind = MethodReturn::Value(typ);
        self
    }

    pub(crate) fn returns_void(mut self) -> Self {
        self.return_kind = MethodReturn::Void;
        self
    }

    pub(crate) fn build(self, index: usize) -> Method {
        use libffi::middle::Type;
        let mut types: Vec<Type> = Vec::with_capacity(self.parameters.len() + 1);
        types.push(Type::pointer()); // com object's this pointer
        for param in &self.parameters {
            if param.is_fill_array() {
                // FillArray: UINT32 capacity, T* items
                types.push(Type::u32());
                types.push(Type::pointer());
            } else if param.typ.is_array() {
                if param.is_out() {
                    // ReceiveArray: UINT32* out_length, T** out_data
                    types.push(Type::pointer());
                    types.push(Type::pointer());
                } else {
                    // PassArray: UINT32 length, T* data
                    types.push(Type::u32());
                    types.push(Type::pointer());
                }
            } else if param.is_out() {
                types.push(Type::pointer());
            } else {
                types.push(param.typ.libffi_type());
            }
        }
        let in_count = self.parameters.iter().filter(|p| p.is_input()).count();
        let has_complex_param = self
            .parameters
            .iter()
            .any(|p| p.typ.is_array() || p.is_fill_array() || p.is_in_out() || p.typ.is_struct());

        // Check if the single in-param (if any) is a simple non-HString, non-Struct type
        let simple_in = !has_complex_param && in_count == 1 && {
            let in_param = self.parameters.iter().find(|p| p.is_input()).unwrap();
            !in_param.typ.is_hstring()
        };

        // Classify array parameters
        let array_in_count = self
            .parameters
            .iter()
            .filter(|p| p.is_input() && p.typ.is_array())
            .count();
        let fill_out_count = self.parameters.iter().filter(|p| p.is_fill_array()).count();
        let array_out_count = self
            .parameters
            .iter()
            .filter(|p| p.is_out() && p.typ.is_array() && !p.is_fill_array())
            .count();
        let scalar_in_count = in_count - array_in_count;
        let scalar_out_count = self.out_count - fill_out_count - array_out_count;

        let returns_hresult = matches!(self.return_kind, MethodReturn::HResult);
        let strategy = if returns_hresult
            && !has_complex_param
            && in_count == 0
            && self.out_count == 1
        {
            CallStrategy::Direct0In1Out
        } else if returns_hresult && !has_complex_param && in_count == 0 && self.out_count == 0 {
            CallStrategy::Direct0In0Out
        } else if returns_hresult && simple_in && self.out_count == 0 {
            CallStrategy::Direct1In0Out
        } else if returns_hresult && simple_in && self.out_count == 1 {
            CallStrategy::Direct1In1Out
        // ReceiveArray only: fn(this, *mut u32, *mut *mut c_void) -> HRESULT
        } else if returns_hresult
            && scalar_in_count == 0
            && array_in_count == 0
            && array_out_count == 1
            && fill_out_count == 0
            && scalar_out_count == 0
        {
            CallStrategy::DirectReceiveArray
        // PassArray + 1 out: fn(this, u32, *const u8, out) -> HRESULT
        } else if returns_hresult
            && scalar_in_count == 0
            && array_in_count == 1
            && array_out_count == 0
            && fill_out_count == 0
            && scalar_out_count == 1
        {
            CallStrategy::DirectPassArray1Out
        // FillArray only: fn(this, u32, *mut u8, *mut u32) -> HRESULT
        } else if returns_hresult
            && scalar_in_count == 0
            && array_in_count == 0
            && fill_out_count == 1
            && array_out_count == 0
            && scalar_out_count == 0
        {
            CallStrategy::DirectFillArray
        // 1 scalar in + FillArray: fn(this, val, u32, *mut u8, *mut u32) -> HRESULT
        } else if returns_hresult
            && scalar_in_count == 1
            && array_in_count == 0
            && fill_out_count == 1
            && array_out_count == 0
            && scalar_out_count == 0
        {
            let in_param = self
                .parameters
                .iter()
                .find(|p| p.is_input() && !p.typ.is_array())
                .unwrap();
            if !in_param.typ.is_hstring() && !in_param.typ.is_struct() {
                CallStrategy::Direct1InFillArray
            } else {
                CallStrategy::Libffi(Cif::new(types.into_iter(), self.return_kind.libffi_type()))
            }
        } else {
            CallStrategy::Libffi(Cif::new(types.into_iter(), self.return_kind.libffi_type()))
        };

        Method {
            info: MethodInfo {
                index,
                parameters: self.parameters,
                out_count: self.out_count,
                return_kind: self.return_kind,
            },
            strategy,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MethodSignature(AbiMethodSignature);

impl MethodSignature {
    pub(crate) fn from_abi(signature: AbiMethodSignature) -> Self {
        Self(signature)
    }

    pub fn new(table: &Arc<MetadataTable>) -> Self {
        Self(AbiMethodSignature::new(table))
    }

    pub fn new_with_registry(table: &Arc<MetadataTable>) -> Self {
        Self::new(table)
    }

    pub fn add_in(self, typ: TypeHandle) -> Self {
        Self(self.0.add_in_type(ParameterType::winrt(typ)))
    }

    pub fn add_out(self, typ: TypeHandle) -> Self {
        Self(self.0.add_out_type(ParameterType::winrt(typ)))
    }

    pub fn add_out_fill(self, typ: TypeHandle) -> Self {
        Self(self.0.add_out_fill_type(ParameterType::winrt(typ)))
    }

    pub fn build(self, index: usize) -> Method {
        self.0.build(index)
    }
}

#[derive(Debug)]
pub struct MethodInfo {
    pub index: usize,
    pub parameters: Vec<Parameter>,
    pub out_count: usize,
    pub(crate) return_kind: MethodReturn,
}

/// How a Method should be invoked — decided once at build time.
#[derive(Debug)]
enum CallStrategy {
    /// 0 in + 0 out: fn(this) -> HRESULT.
    Direct0In0Out,
    /// 0 in + 1 out (getter): fn(this, out) -> HRESULT.
    Direct0In1Out,
    /// 1 in + 0 out (setter, non-HString): fn(this, val) -> HRESULT.
    Direct1In0Out,
    /// 1 in + 1 out (factory/query, non-HString in): fn(this, val, out) -> HRESULT.
    Direct1In1Out,
    /// ReceiveArray: fn(this, *mut u32, *mut *mut c_void) -> HRESULT.
    DirectReceiveArray,
    /// PassArray + 1 out: fn(this, u32, *const u8, out) -> HRESULT.
    DirectPassArray1Out,
    /// FillArray only: fn(this, u32, *mut u8) -> HRESULT.
    DirectFillArray,
    /// 1 scalar in + FillArray: fn(this, val, u32, *mut u8) -> HRESULT.
    Direct1InFillArray,
    /// General case → libffi via cached Cif.
    Libffi(Cif),
}

#[derive(Debug)]
pub struct Method {
    info: MethodInfo,
    strategy: CallStrategy,
}

fn expected_object_iid(typ: &TypeHandle) -> Option<GUID> {
    match typ.kind() {
        TypeKind::Object => Some(IInspectable::IID),
        TypeKind::Interface(_)
        | TypeKind::Delegate(_)
        | TypeKind::RuntimeClass(_)
        | TypeKind::Parameterized(_)
        | TypeKind::IAsyncAction
        | TypeKind::IAsyncActionWithProgress(_)
        | TypeKind::IAsyncOperation(_)
        | TypeKind::IAsyncOperationWithProgress(_) => typ.iid(),
        _ => None,
    }
}

fn coerce_input_object(
    expected: &TypeHandle,
    value: &WinRTValue,
) -> windows_core::Result<Option<WinRTValue>> {
    let Some(iid) = expected_object_iid(expected) else {
        return Ok(None);
    };
    // Null objects are always allowed (`WinRTValue::Null` and the
    // Object-typed null variant both project as "no coercion needed" — the
    // ABI receives a null pointer directly).
    if value.is_null_object() {
        return Ok(None);
    }
    // Raw pointers never satisfy a WinRT object parameter. Otherwise arbitrary
    // pointer bits could reach a typed COM slot without QueryInterface validation.
    if matches!(value, WinRTValue::RawPtr(_)) {
        return Err(windows_core::Error::new(
            windows_core::HRESULT(0x80070057u32 as i32),
            &format!(
                "Refusing to pass a raw pointer as a typed COM parameter ({}). \
                 Use a Pointer signature for native pointers and handles; for \
                 COM parameters pass a real object (or one obtained via `.cast(IID)`).",
                expected.signature_string(),
            ),
        ));
    }

    let object = value.as_object().ok_or_else(|| {
        windows_core::Error::new(
            windows_core::HRESULT(0x80070057u32 as i32),
            &format!(
                "Expected object argument for {}, found {:?}",
                expected.signature_string(),
                value.get_type_kind()
            ),
        )
    })?;

    WinRTValue::Object(object)
        .cast(&iid)
        .map(Some)
        .map_err(|error| match error {
            crate::result::Error::WindowsError(error) => error,
            other => windows_core::Error::new(
                windows_core::HRESULT(0x80070057u32 as i32),
                &other.message(),
            ),
        })
}

fn coerce_input_array(
    expected: &TypeHandle,
    value: &WinRTValue,
) -> windows_core::Result<Option<WinRTValue>> {
    if !expected.is_array() {
        return Ok(None);
    }

    let element_type = expected.array_element_type();
    if expected_object_iid(&element_type).is_none() {
        return Ok(None);
    }

    let array = value.as_array().ok_or_else(|| {
        windows_core::Error::new(
            windows_core::HRESULT(0x80070057u32 as i32),
            &format!(
                "Expected array argument for {}, found {:?}",
                expected.signature_string(),
                value.get_type_kind()
            ),
        )
    })?;
    let mut values = Vec::with_capacity(array.len());
    let mut changed = false;
    for index in 0..array.len() {
        let value = array.get(index);
        if let Some(coerced) = coerce_input_object(&element_type, &value)? {
            values.push(coerced);
            changed = true;
        } else {
            values.push(value);
        }
    }

    Ok(changed
        .then(|| WinRTValue::Array(crate::array::ArrayData::from_values(element_type, &values))))
}

struct InvocationArgs<'a> {
    original: &'a [WinRTValue],
    replacements: Option<Vec<Option<WinRTValue>>>,
}

impl<'a> InvocationArgs<'a> {
    fn new(original: &'a [WinRTValue]) -> Self {
        Self {
            original,
            replacements: None,
        }
    }

    fn replace(&mut self, index: usize, value: WinRTValue) {
        self.replacements.get_or_insert_with(|| {
            std::iter::repeat_with(|| None)
                .take(self.original.len())
                .collect()
        })[index] = Some(value);
    }
}

impl call::ArgumentList for InvocationArgs<'_> {
    fn get_value(&self, index: usize) -> &WinRTValue {
        self.replacements
            .as_ref()
            .and_then(|values| values[index].as_ref())
            .unwrap_or(&self.original[index])
    }
}

impl Method {
    // --- Fast getter paths: zero Vec/WinRTValue allocation ---

    /// Getter → i32 (0 in, 1 out). Writes directly to stack i32.
    pub fn call_getter_i32(&self, obj: *mut std::ffi::c_void) -> windows_core::Result<i32> {
        let mut out: i32 = 0;
        let hr = call::call_winrt_method_1(
            self.info.index,
            obj,
            &mut out as *mut i32 as *mut std::ffi::c_void,
        );
        hr.ok()?;
        Ok(out)
    }

    /// Getter → bool (0 in, 1 out). Writes directly to stack bool.
    pub fn call_getter_bool(&self, obj: *mut std::ffi::c_void) -> windows_core::Result<bool> {
        let mut out: i32 = 0; // WinRT bool is i32 on ABI
        let hr = call::call_winrt_method_1(
            self.info.index,
            obj,
            &mut out as *mut i32 as *mut std::ffi::c_void,
        );
        hr.ok()?;
        Ok(out != 0)
    }

    /// Getter → HSTRING (0 in, 1 out). Writes directly to stack HSTRING ptr.
    pub fn call_getter_hstring(
        &self,
        obj: *mut std::ffi::c_void,
    ) -> windows_core::Result<windows_core::HSTRING> {
        // HSTRING is a pointer-sized handle on ABI. Let WinRT write it directly.
        let mut out = windows_core::HSTRING::new();
        let hr = call::call_winrt_method_1(
            self.info.index,
            obj,
            &mut out as *mut windows_core::HSTRING as *mut std::ffi::c_void,
        );
        hr.ok()?;
        Ok(out)
    }

    /// Getter → COM object (0 in, 1 out). Writes directly to stack pointer.
    pub fn call_getter_object(
        &self,
        obj: *mut std::ffi::c_void,
    ) -> windows_core::Result<WinRTValue> {
        let mut out: *mut std::ffi::c_void = std::ptr::null_mut();
        let hr = call::call_winrt_method_1(
            self.info.index,
            obj,
            &mut out as *mut _ as *mut std::ffi::c_void,
        );
        hr.ok()?;
        if out.is_null() {
            Ok(WinRTValue::Null)
        } else {
            Ok(WinRTValue::Object(unsafe {
                windows_core::IUnknown::from_raw(out)
            }))
        }
    }

    pub fn call_dynamic(
        &self,
        obj: *mut std::ffi::c_void,
        args: &[WinRTValue],
    ) -> windows_core::Result<Vec<WinRTValue>> {
        let mut args = InvocationArgs::new(args);
        for parameter in self.info.parameters.iter().filter(|p| p.is_input()) {
            let input_index = parameter.input_index.expect("input parameter index");
            let value = args.get_value(input_index);
            let coerced = if let Some(typ) = parameter.typ.as_winrt() {
                if typ.is_array() {
                    coerce_input_array(typ, value)?
                } else {
                    coerce_input_object(typ, value)?
                }
            } else {
                None
            };
            if let Some(value) = coerced {
                args.replace(input_index, value);
            }
        }

        match &self.strategy {
            CallStrategy::Direct0In0Out => {
                // 0 in + 0 out: fn(this) -> HRESULT
                let hr = call::call_winrt_method_0(self.info.index, obj);
                hr.ok()?;
                Ok(vec![])
            }
            CallStrategy::Direct0In1Out => {
                // 0 in + 1 out: fn(this, out) -> HRESULT
                let param = &self.info.parameters[0];
                let mut out = param.typ.default_value();
                let hr = call::call_winrt_method_1(self.info.index, obj, out.out_ptr());
                hr.ok()?;
                // COM pointer types use RawPtr(null) as buffer to avoid IUnknown::from_raw(null) UB.
                // After COM writes the pointer, convert via from_out.
                if let WinRTValue::RawPtr(raw_ptr) = out {
                    out = param.typ.from_out(raw_ptr).map_err(|e| {
                        windows_core::Error::new(windows_core::HRESULT(-1), &format!("{:?}", e))
                    })?;
                }
                out.sanitize_null_object();
                Ok(vec![out])
            }
            CallStrategy::Direct1In0Out => {
                // 1 in + 0 out: fn(this, val) -> HRESULT
                let hr = call::call_1in(self.info.index, obj, args.get_value(0));
                hr.ok()?;
                Ok(vec![])
            }
            CallStrategy::Direct1In1Out => {
                // 1 in + 1 out: fn(this, val, out) -> HRESULT
                let out_param = self.info.parameters.iter().find(|p| p.is_out()).unwrap();
                let mut out = out_param.typ.default_value();
                let hr =
                    call::call_1in_1out(self.info.index, obj, args.get_value(0), out.out_ptr());
                hr.ok()?;
                if let WinRTValue::RawPtr(raw_ptr) = out {
                    out = out_param.typ.from_out(raw_ptr).map_err(|e| {
                        windows_core::Error::new(windows_core::HRESULT(-1), &format!("{:?}", e))
                    })?;
                }
                out.sanitize_null_object();
                Ok(vec![out])
            }
            CallStrategy::DirectReceiveArray => {
                // fn(this, *mut u32, *mut *mut c_void) -> HRESULT
                let param = &self.info.parameters[0];
                let elem_type = param.typ.array_element_type();
                let mut length: u32 = 0;
                let mut data_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
                let fptr = call::get_vtable_function_ptr(obj, self.info.index);
                let hr: windows_core::HRESULT = unsafe {
                    let method: unsafe extern "system" fn(
                        *mut std::ffi::c_void,
                        *mut u32,
                        *mut *mut std::ffi::c_void,
                    )
                        -> windows_core::HRESULT = std::mem::transmute(fptr);
                    method(obj, &mut length, &mut data_ptr)
                };
                if hr.is_err() {
                    // Callee may have allocated a buffer before returning failure.
                    // Wrap in ArrayData to release elements + CoTaskMemFree.
                    if !data_ptr.is_null() {
                        let _ = crate::array::ArrayData::from_cotaskmem(
                            elem_type.clone(),
                            data_ptr,
                            length as usize,
                        );
                    }
                    hr.ok()?;
                }
                let array = if data_ptr.is_null() || length == 0 {
                    if !data_ptr.is_null() {
                        unsafe {
                            windows::Win32::System::Com::CoTaskMemFree(Some(data_ptr));
                        }
                    }

                    crate::array::ArrayData::empty(elem_type)
                } else {
                    crate::array::ArrayData::from_cotaskmem(elem_type, data_ptr, length as usize)
                };
                Ok(vec![WinRTValue::Array(array)])
            }
            CallStrategy::DirectPassArray1Out => {
                // fn(this, u32, *const u8, out) -> HRESULT
                let in_param = self.info.parameters.iter().find(|p| p.is_input()).unwrap();
                let out_param = self.info.parameters.iter().find(|p| p.is_out()).unwrap();
                let array_data = args.get_value(in_param.value_index).as_array().unwrap();
                let buffer = array_data.serialize_for_abi();
                let mut out = out_param.typ.default_value();
                let fptr = call::get_vtable_function_ptr(obj, self.info.index);
                let hr: windows_core::HRESULT = unsafe {
                    let method: unsafe extern "system" fn(
                        *mut std::ffi::c_void,
                        u32,
                        *const u8,
                        *mut std::ffi::c_void,
                    )
                        -> windows_core::HRESULT = std::mem::transmute(fptr);
                    method(obj, array_data.len() as u32, buffer.as_ptr(), out.out_ptr())
                };
                hr.ok()?;
                if let WinRTValue::RawPtr(raw_ptr) = out {
                    out = out_param.typ.from_out(raw_ptr).map_err(|e| {
                        windows_core::Error::new(windows_core::HRESULT(-1), &format!("{:?}", e))
                    })?;
                }
                out.sanitize_null_object();
                Ok(vec![out])
            }
            CallStrategy::DirectFillArray => {
                // fn(this, u32, *mut u8) -> HRESULT
                // FillArray: caller provides buffer of known capacity, callee fills it.
                let param = &self.info.parameters[0];
                let elem_type = param.typ.array_element_type();
                let fptr = call::get_vtable_function_ptr(obj, self.info.index);

                assert!(
                    param
                        .input_index
                        .is_some_and(|index| { args.get_value(index).as_array().is_some() }),
                    "DirectFillArray requires a pre-allocated array argument with the desired capacity. \
                     Pass an ArrayData with the expected number of elements."
                );
                let array_data = args
                    .get_value(param.input_index.unwrap())
                    .as_array()
                    .unwrap();
                let capacity = array_data.len() as u32;
                let total_bytes = capacity as usize * elem_type.element_size();
                let buffer_ptr =
                    unsafe { windows::Win32::System::Com::CoTaskMemAlloc(total_bytes) as *mut u8 };
                assert!(!buffer_ptr.is_null(), "CoTaskMemAlloc failed for FillArray");
                unsafe { std::ptr::write_bytes(buffer_ptr, 0, total_bytes) };
                let hr: windows_core::HRESULT = unsafe {
                    let method: unsafe extern "system" fn(
                        *mut std::ffi::c_void,
                        u32,
                        *mut u8,
                    )
                        -> windows_core::HRESULT = std::mem::transmute(fptr);
                    method(obj, capacity, buffer_ptr)
                };
                if hr.is_err() {
                    // Callee may have written elements before failing.
                    // Buffer was zero-initialized, so null slots are safe to release.
                    // Use capacity as cleanup length — ArrayData::Drop skips null elements.
                    let _ = crate::array::ArrayData::from_cotaskmem(
                        elem_type.clone(),
                        buffer_ptr as _,
                        capacity as usize,
                    );
                    hr.ok()?;
                }
                let array = crate::array::ArrayData::from_cotaskmem(
                    elem_type,
                    buffer_ptr as _,
                    capacity as usize,
                );
                Ok(vec![WinRTValue::Array(array)])
            }
            CallStrategy::Direct1InFillArray => {
                // fn(this, val, u32, *mut u8) -> HRESULT
                let in_param = self.info.parameters.iter().find(|p| p.is_input()).unwrap();
                let fill_param = self
                    .info
                    .parameters
                    .iter()
                    .find(|p| p.is_fill_array())
                    .unwrap();
                let array_data = args
                    .get_value(fill_param.input_index.unwrap())
                    .as_array()
                    .unwrap();
                let elem_type = fill_param.typ.array_element_type();
                let capacity = array_data.len() as u32;
                let total_bytes = capacity as usize * elem_type.element_size();
                let buffer_ptr =
                    unsafe { windows::Win32::System::Com::CoTaskMemAlloc(total_bytes) as *mut u8 };
                assert!(!buffer_ptr.is_null(), "CoTaskMemAlloc failed for FillArray");
                unsafe { std::ptr::write_bytes(buffer_ptr, 0, total_bytes) };
                let fptr = call::get_vtable_function_ptr(obj, self.info.index);
                let hr = call::call_fill_array_1in(
                    fptr,
                    obj,
                    args.get_value(in_param.value_index),
                    capacity,
                    buffer_ptr,
                );
                if hr.is_err() {
                    // Buffer was zero-initialized; use capacity for cleanup.
                    let _ = crate::array::ArrayData::from_cotaskmem(
                        elem_type.clone(),
                        buffer_ptr as _,
                        capacity as usize,
                    );
                    hr.ok()?;
                }
                let array = crate::array::ArrayData::from_cotaskmem(
                    elem_type,
                    buffer_ptr as _,
                    capacity as usize,
                );
                Ok(vec![WinRTValue::Array(array)])
            }
            CallStrategy::Libffi(cif) => call::call_method_dynamic(
                self.info.index,
                obj,
                &self.info.parameters,
                &args,
                self.info.out_count,
                &self.info.return_kind,
                cif,
            ),
        }
    }
}

#[derive(Debug)]
pub struct InterfaceSignature {
    pub name: String,
    pub iid: windows_core::GUID,
    pub methods: Vec<Method>,
    #[allow(dead_code)]
    table: Arc<MetadataTable>,
}

impl InterfaceSignature {
    pub fn define_interface(
        name: String,
        iid: windows_core::GUID,
        table: &Arc<MetadataTable>,
    ) -> Self {
        InterfaceSignature {
            name,
            iid,
            methods: Vec::new(),
            table: Arc::clone(table),
        }
    }

    pub fn define_from_iunknown(name: &str, iid: GUID, table: &Arc<MetadataTable>) -> Self {
        let mut t = InterfaceSignature::define_interface(name.to_owned(), iid, table);
        t.add_method(MethodSignature::new(table)) // 0 QueryInterface
            .add_method(MethodSignature::new(table)) // 1 AddRef
            .add_method(MethodSignature::new(table)); // 2 Release
        t
    }

    pub fn define_from_iinspectable(name: &str, iid: GUID, table: &Arc<MetadataTable>) -> Self {
        let mut t = Self::define_from_iunknown(name, iid, table);
        t.add_method(MethodSignature::new(table)) // 3 GetIids
            .add_method(MethodSignature::new(table).add_out(table.hstring())) // 4 GetRuntimeClassName
            .add_method(MethodSignature::new(table)); // 5 GetTrustLevel
        t
    }

    pub fn add_method(&mut self, signature: MethodSignature) -> &mut Self {
        let method = signature.build(self.methods.len());
        self.methods.push(method);
        self
    }
}

#[allow(dead_code)]
pub struct RuntimeClassSignature {
    name: HSTRING,
    static_interfaces: Vec<InterfaceSignature>,
    instance_interfaces: Vec<InterfaceSignature>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Foundation::{IStringable, IUriRuntimeClass, Uri};
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    use windows_core::{IInspectable, Interface, h};

    #[test]
    fn fill_array_tracks_distinct_input_and_output_indices() {
        let table = MetadataTable::new();
        let method = MethodSignature::new(&table)
            .add_in(table.u32_type())
            .add_out_fill(table.array(&table.hstring()))
            .add_out(table.u32_type())
            .build(6);

        assert_eq!(method.info.parameters[0].value_index, 0);
        assert_eq!(method.info.parameters[0].input_index, Some(0));
        assert_eq!(method.info.parameters[1].value_index, 0);
        assert_eq!(method.info.parameters[1].input_index, Some(1));
        assert_eq!(method.info.parameters[2].value_index, 1);
        assert_eq!(method.info.parameters[2].input_index, None);
    }

    #[test]
    fn coerces_object_inputs_to_the_expected_interface() -> windows_core::Result<()> {
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let uri = Uri::CreateUri(h!("https://example.com"))?;
        let default_interface: IUriRuntimeClass = uri.cast()?;
        let expected_interface: IStringable = uri.cast()?;
        assert_ne!(
            default_interface.as_raw(),
            expected_interface.as_raw(),
            "test requires distinct default and requested interface pointers"
        );

        let table = MetadataTable::new();
        let expected_type = table.interface(IStringable::IID);
        let value = WinRTValue::Object(default_interface.cast()?);
        let coerced = coerce_input_object(&expected_type, &value)?
            .expect("interface parameters must be coerced");
        assert_eq!(
            coerced.as_object().unwrap().as_raw(),
            expected_interface.as_raw()
        );

        let inspectable: IInspectable = uri.cast()?;
        let coerced_object = coerce_input_object(&table.object(), &value)?
            .expect("Object parameters must be coerced to IInspectable");
        assert_eq!(
            coerced_object.as_object().unwrap().as_raw(),
            inspectable.as_raw()
        );
        Ok(())
    }

    #[test]
    fn raw_pointer_is_rejected_for_winrt_object_params() {
        let table = MetadataTable::new();
        let bogus = WinRTValue::RawPtr(0xDEADBEEF as *mut std::ffi::c_void);

        let object_ty = table.object();
        let object_err = coerce_input_object(&object_ty, &bogus)
            .expect_err("RawPtr into Object must be rejected");
        assert_eq!(object_err.code().0, 0x80070057u32 as i32);

        let iface_ty = table.interface(IStringable::IID);
        let err = coerce_input_object(&iface_ty, &bogus)
            .expect_err("RawPtr into a typed interface must be rejected");
        assert_eq!(
            err.code().0,
            0x80070057u32 as i32,
            "typed-interface RawPtr rejection must use E_INVALIDARG (got {:?})",
            err
        );
        let msg = err.message();
        assert!(
            msg.contains("raw pointer") && msg.contains("typed"),
            "rejection error must explain the constraint, got: {}",
            msg
        );

        let null_object = WinRTValue::Null;
        assert!(
            coerce_input_object(&object_ty, &null_object)
                .expect("null into TypeKind::Object must be allowed")
                .is_none(),
        );
        assert!(
            coerce_input_object(&iface_ty, &null_object)
                .expect("null into typed interface must be allowed")
                .is_none(),
        );
    }

    #[test]
    fn coerces_object_array_elements_to_the_expected_interface() -> windows_core::Result<()> {
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let uri = Uri::CreateUri(h!("https://example.com"))?;
        let default_interface: IUriRuntimeClass = uri.cast()?;
        let expected_interface: IStringable = uri.cast()?;

        let table = MetadataTable::new();
        let element_type = table.interface(IStringable::IID);
        let array_type = table.array(&element_type);
        let value = WinRTValue::Array(crate::array::ArrayData::from_values(
            element_type,
            &[WinRTValue::Object(default_interface.cast()?)],
        ));
        let coerced = coerce_input_array(&array_type, &value)?
            .expect("object array elements must be coerced");
        assert_eq!(
            coerced
                .as_array()
                .unwrap()
                .get(0)
                .as_object()
                .unwrap()
                .as_raw(),
            expected_interface.as_raw()
        );
        Ok(())
    }
}
