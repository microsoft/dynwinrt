// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Validated Classic-COM semantic IR.
//!
//! Nothing in this module depends on the shared WinRT metadata model.  A value
//! can enter this IR only after its ABI shape, ownership, and projection have
//! been validated by `project`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ComPrimitive {
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
    Char16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ComEnumUnderlying {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ComScalarRepr {
    Primitive(ComPrimitive),
    NativeIsize,
    NativeUsize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum PointerAliasKind {
    HandleValue,
    DataPointer,
    StringPointer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ComType {
    Primitive(ComPrimitive),
    NativeIsize,
    NativeUsize,
    Win32Bool,
    HResult,
    Guid,
    HString,
    Enum {
        name: String,
        underlying: ComEnumUnderlying,
    },
    ScalarAlias {
        name: String,
        underlying: ComScalarRepr,
    },
    RawPointer,
    PointerAlias {
        name: String,
        kind: PointerAliasKind,
    },
    Bstr,
    ManagedInterface {
        iid: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum UnsupportedComType {
    Array,
    ParameterizedInterface { namespace: String, name: String },
    AsyncInterface,
    Delegate { namespace: String, name: String },
    NativeStructLayout { namespace: String, name: String },
    UnknownPointerAlias { namespace: String, name: String },
    UnresolvedInterface { namespace: String, name: String },
    UnresolvedRuntimeClass { namespace: String, name: String },
    UnknownOwnership { type_name: String },
    UnsupportedDirectReturn { type_name: String },
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ComParamDirection {
    In,
    Out,
    InOut,
    OutStringBuffer,
}

impl ComParamDirection {
    pub(super) fn is_input(self) -> bool {
        matches!(self, Self::In | Self::InOut)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectedComParam {
    pub(super) name: String,
    pub(super) typ: ComType,
    pub(super) direction: ComParamDirection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ComReturnConvention {
    HResult,
    SemanticHResult,
    Void,
    Direct(ComType),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StringEncoding {
    Wide,
    Ansi,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ResultConversion {
    Value,
    ManagedCom,
    Bstr,
    CoTaskMemString(StringEncoding),
    CoTaskMemData,
    HString,
    DynamicIidAdoption,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResultSource {
    DirectReturn,
    Param(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectedComResult {
    pub(super) typ: ComType,
    pub(super) source: ResultSource,
    pub(super) conversion: ResultConversion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StringBufferPlan {
    pub(super) buffer_param_index: usize,
    pub(super) count_param_index: usize,
    pub(super) encoding: StringEncoding,
    pub(super) optional_param_indices: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProjectedComMethodKind {
    Normal,
    CallerSuppliedDynamicIid {
        natural_param_count: usize,
    },
    SynthesizedGetForWindow {
        natural_param_count: usize,
        target_iid: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectedComMethod {
    pub(super) name: String,
    pub(super) camel_name: String,
    pub(super) vtable_index: usize,
    pub(super) params: Vec<ProjectedComParam>,
    pub(super) return_convention: ComReturnConvention,
    pub(super) results: Vec<ProjectedComResult>,
    pub(super) string_buffer: Option<StringBufferPlan>,
    pub(super) kind: ProjectedComMethodKind,
    pub(super) doc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ActivationPlan {
    None,
    Coclass {
        clsid: String,
        coclass_name: String,
    },
    WinRtFactory {
        class_name: String,
        class_namespace: String,
        target_iid: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProjectedEnumValue {
    Signed(i64),
    Unsigned(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectedComEnumMember {
    pub(super) name: String,
    pub(super) value: ProjectedEnumValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectedComEnum {
    pub(super) name: String,
    pub(super) underlying: ComEnumUnderlying,
    pub(super) members: Vec<ProjectedComEnumMember>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectedComInterface {
    pub(super) name: String,
    pub(super) namespace: String,
    pub(super) iid: String,
    pub(super) is_iunknown_rooted: bool,
    pub(super) methods: Vec<ProjectedComMethod>,
    pub(super) activation: ActivationPlan,
    pub(super) referenced_enums: Vec<ProjectedComEnum>,
}
