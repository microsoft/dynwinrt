// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiType {
    Bool32,
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
    Pointer,
    FunctionPointer,
    Handle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
    InOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constness {
    Const,
    Mutable,
    Unspecified,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cleanup {
    None,
    CloseHandle,
    RegCloseKey,
    LocalFree,
    GlobalFree,
    FreeLibrary,
    CloseServiceHandle,
    CoTaskMemFree,
    CredFree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuccessRule {
    Always,
    ReturnZero,
    ReturnNonZero,
    ReturnNonNull,
    HResultSucceeded,
    SignedNonNegative,
    ReturnValidHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallingConvention {
    System,
    Cdecl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringEncoding {
    Wide,
    Ansi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scalar {
    Bool8,
    Bool32,
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
    NativeIsize,
    NativeUsize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumUnderlying {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumMember {
    pub name: String,
    pub value: i128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumDefinition {
    pub namespace: String,
    pub name: String,
    pub underlying: EnumUnderlying,
    pub members: Vec<EnumMember>,
    pub is_flags: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeScalar {
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
    NativeIsize,
    NativeUsize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeAggregateKind {
    Struct,
    Union,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeFieldType {
    Scalar(NativeScalar),
    Guid,
    Pointer,
    Handle {
        cleanup: Cleanup,
    },
    Struct {
        name: String,
        layout: Box<NativeArchitectureLayout>,
        by_value_compatible: bool,
    },
    Union {
        name: String,
        layout: Box<NativeArchitectureLayout>,
        by_value_compatible: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeField {
    pub name: String,
    pub offset: usize,
    pub count: u32,
    pub typ: NativeFieldType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeArchitectureLayout {
    pub size: usize,
    pub alignment: usize,
    pub fields: Vec<NativeField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeLayout {
    pub namespace: String,
    pub name: String,
    pub kind: NativeAggregateKind,
    pub by_value_compatible: bool,
    pub x86: NativeArchitectureLayout,
    pub x64: NativeArchitectureLayout,
    pub arm64: NativeArchitectureLayout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueType {
    Scalar(Scalar),
    Enum {
        namespace: String,
        name: String,
        underlying: EnumUnderlying,
    },
    Handle {
        namespace: String,
        name: String,
    },
    DataPointer,
    StringPointer(StringEncoding),
    FunctionPointer,
    NativeStructPointer {
        layout: NativeLayout,
    },
    NativeUnionPointer {
        layout: NativeLayout,
    },
    NativeStruct {
        layout: NativeLayout,
    },
    ScalarPointer {
        scalar: Scalar,
    },
    GuidPointer,
    NullPointer,
    ComInterface {
        name: String,
        iid: String,
    },
    StringPointerPointer(StringEncoding),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterContract {
    pub name: String,
    pub native_name: Option<(String, String)>,
    pub typ: ValueType,
    pub abi: AbiType,
    pub pointer_depth: u8,
    pub constness: Constness,
    pub direction: Direction,
    pub nullable: bool,
    pub reserved: bool,
    pub null_null_terminated: bool,
    pub cleanup: Cleanup,
    pub consumes_resource: bool,
    pub resource_cleanup: Cleanup,
    pub buffer: Option<BufferContract>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferContract {
    pub count_parameter: Option<usize>,
    pub constant_count: Option<usize>,
    pub count_is_bytes: bool,
    pub element_size: usize,
    pub element_alignment: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionContract {
    pub namespace: String,
    pub container: String,
    pub name: String,
    pub dll: String,
    pub entry_point: String,
    pub parameters: Vec<ParameterContract>,
    pub return_type: Option<ValueType>,
    pub return_abi: Option<AbiType>,
    pub return_aggregate: Option<NativeLayout>,
    pub return_native_name: Option<(String, String)>,
    pub return_pointer_depth: u8,
    pub return_constness: Constness,
    pub return_cleanup: Cleanup,
    pub return_is_status: bool,
    pub success_rule: SuccessRule,
    pub capture_last_error: bool,
    pub calling_convention: CallingConvention,
    pub enums: Vec<EnumDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurfaceType {
    Boolean,
    Number,
    BigInt,
    Enum(String),
    Handle(String),
    Buffer,
    String(StringEncoding),
    MultiString(StringEncoding),
    ManagedResource,
    Resource,
    NativeStruct(String),
    NativeUnion(String),
    ComInterface(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceParameter {
    pub name: String,
    pub typ: SurfaceType,
    pub nullable: bool,
    pub minimum_bytes: Option<usize>,
    pub alignment: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputExpression {
    Surface {
        parameter_index: usize,
        conversion: Conversion,
    },
    BufferLength {
        parameter_index: usize,
        divisor: usize,
        abi: AbiType,
    },
    NullPointer,
    Zero(AbiType),
    NativeAggregate {
        parameter_index: usize,
        layout: NativeLayout,
        nullable: bool,
        by_value: bool,
    },
    ScalarPointer {
        parameter_index: usize,
        scalar: Scalar,
        nullable: bool,
    },
    ComInterface {
        parameter_index: usize,
        iid: String,
    },
    StringPointerPointer {
        parameter_index: usize,
        encoding: StringEncoding,
        nullable: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conversion {
    Boolean8,
    Boolean,
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
    Handle,
    DataPointer,
    WideString,
    AnsiString,
    WideMultiString,
    AnsiMultiString,
    ResourceInput(Cleanup),
    Number,
    BigInt,
    Resource,
    NativeAggregate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedOutput {
    pub name: String,
    pub output_index: usize,
    pub typ: SurfaceType,
    pub conversion: Conversion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeParameter {
    pub abi: AbiType,
    pub direction: Direction,
    pub nullable: bool,
    pub cleanup: Cleanup,
    pub consumes_resource: bool,
    pub resource_cleanup: Cleanup,
    pub aggregate: Option<NativeLayout>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePlan {
    pub dll: String,
    pub entry_point: String,
    pub parameters: Vec<RuntimeParameter>,
    pub return_abi: Option<AbiType>,
    pub return_aggregate: Option<NativeLayout>,
    pub return_cleanup: Cleanup,
    pub success_rule: SuccessRule,
    pub capture_last_error: bool,
    pub calling_convention: CallingConvention,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnShape {
    Void,
    Direct {
        typ: SurfaceType,
        conversion: Conversion,
    },
    Object {
        status: bool,
        return_value: Option<(SurfaceType, Conversion)>,
        outputs: Vec<ProjectedOutput>,
        last_error: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedFunction {
    pub metadata_name: String,
    pub js_name: String,
    pub unicode_alias: Option<String>,
    pub parameters: Vec<SurfaceParameter>,
    pub inputs: Vec<InputExpression>,
    pub runtime: RuntimePlan,
    pub return_shape: ReturnShape,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedApis {
    pub namespace: String,
    pub class_name: String,
    pub functions: Vec<ProjectedFunction>,
    pub enums: Vec<EnumDefinition>,
    pub native_builders: Vec<ProjectedNativeBuilder>,
    pub async_functions: Vec<ProjectedAsyncFunction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsyncIoKind {
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedAsyncFunction {
    pub js_name: String,
    pub kind: AsyncIoKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedNativeBuilder {
    pub layout_name: String,
    pub js_name: String,
    pub size_field: Option<String>,
    pub fields: Vec<ProjectedNativeBuilderField>,
    pub outputs: Vec<ProjectedNativeOutputField>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeBuilderFieldKind {
    Boolean,
    DataPointer { nullable: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedNativeBuilderField {
    pub native_name: String,
    pub surface_name: String,
    pub kind: NativeBuilderFieldKind,
    pub optional: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeOutputFieldKind {
    U32,
    Resource { cleanup: Cleanup },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedNativeOutputField {
    pub native_name: String,
    pub surface_name: String,
    pub kind: NativeOutputFieldKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmittedFunction {
    pub identity: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionResult {
    pub projected: ProjectedApis,
    pub omitted: Vec<OmittedFunction>,
}

impl ProjectionResult {
    pub fn complete_count(&self) -> usize {
        self.projected.functions.len() + self.projected.async_functions.len()
    }
}
