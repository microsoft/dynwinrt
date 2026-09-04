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
pub(super) enum StringEncoding {
    Wide,
    Ansi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum PointerAliasKind {
    HandleValue,
    DataPointer,
    StringPointer(StringEncoding),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SafeArrayElement {
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
    Bool,
    Bstr,
    Interface { iid: [u8; 16] },
    Variant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativePodScalar {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NativePodFieldType {
    Scalar(NativePodScalar),
    Guid,
    Pointer,
    Struct {
        name: String,
        layout: Box<NativePodArchitectureLayout>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NativePodField {
    pub(super) name: String,
    pub(super) offset: usize,
    pub(super) count: u32,
    pub(super) typ: NativePodFieldType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NativePodArchitectureLayout {
    pub(super) size: usize,
    pub(super) alignment: usize,
    pub(super) fields: Vec<NativePodField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NativePodLayout {
    pub(super) namespace: String,
    pub(super) name: String,
    pub(super) initializers: Vec<NativePodInitializer>,
    pub(super) x86: NativePodArchitectureLayout,
    pub(super) x64: NativePodArchitectureLayout,
    pub(super) arm64: NativePodArchitectureLayout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NativePodInitializer {
    SizeOfLayout { field: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NativeUnionFieldType {
    Scalar(NativePodScalar),
    Guid,
    Pointer,
    Struct {
        name: String,
        layout: Box<NativePodArchitectureLayout>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NativeUnionField {
    pub(super) name: String,
    pub(super) count: u32,
    pub(super) typ: NativeUnionFieldType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NativeUnionArchitectureLayout {
    pub(super) size: usize,
    pub(super) alignment: usize,
    pub(super) fields: Vec<NativeUnionField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NativeUnionLayout {
    pub(super) namespace: String,
    pub(super) name: String,
    pub(super) x86: NativeUnionArchitectureLayout,
    pub(super) x64: NativeUnionArchitectureLayout,
    pub(super) arm64: NativeUnionArchitectureLayout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ComType {
    Primitive(ComPrimitive),
    NativeIsize,
    NativeUsize,
    Win32Bool,
    HResult,
    Guid,
    GuidPointer,
    HString,
    Enum {
        namespace: String,
        name: String,
        underlying: ComEnumUnderlying,
    },
    ScalarAlias {
        namespace: String,
        name: String,
        underlying: ComScalarRepr,
    },
    RawPointer,
    ExactNullPointer,
    AllocatorPointer,
    ConsumedAllocatorPointer,
    InspectedAllocatorPointer,
    PointerAlias {
        namespace: String,
        name: String,
        kind: PointerAliasKind,
    },
    NativePod {
        layout: NativePodLayout,
    },
    NativePodPointer {
        layout: NativePodLayout,
    },
    NativeUnionPointer {
        layout: NativeUnionLayout,
    },
    Bstr,
    Variant,
    VariantByValue,
    SafeArray {
        element: SafeArrayElement,
    },
    PropVariant,
    DispatchParams,
    ExcepInfo,
    StatStg,
    ManagedInterface {
        iid: String,
    },
    CoTaskMemWideString,
    StringArray {
        encoding: StringEncoding,
        element_pointer_depth: u8,
        element_const: bool,
    },
    TypedBuffer {
        element: Box<ComType>,
    },
    OwningArray {
        element: Box<ComType>,
        interface: Option<ProjectedInterfaceRef>,
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
    OptionalOut,
    InOut,
    OutStringBuffer,
    InputBuffer,
    CallerOutputBuffer,
    CalleeAllocatedBuffer,
}

impl ComParamDirection {
    pub(super) fn is_input(self) -> bool {
        matches!(
            self,
            Self::In | Self::InOut | Self::InputBuffer | Self::CallerOutputBuffer
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectedComParam {
    pub(super) name: String,
    pub(super) typ: ComType,
    pub(super) direction: ComParamDirection,
    pub(super) surface_input: bool,
    pub(super) surface_result: bool,
    pub(super) nullable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ComReturnConvention {
    HResult,
    SemanticHResult,
    DispatchInvokeHResult,
    Void,
    Direct(ComType),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ResultConversion {
    Value,
    BorrowedHandle,
    OwnedHandle(OwnedHandleCleanup),
    ManagedCom,
    Bstr,
    CoTaskMemString(StringEncoding),
    CoTaskMemData,
    HString,
    DynamicIidAdoption,
    Buffer,
    PlainArray,
    EnumeratorArray {
        interface: Option<ProjectedInterfaceRef>,
    },
    OwningArray {
        interface: Option<ProjectedInterfaceRef>,
    },
    Variant,
    SafeArray,
    PropVariant,
    ExcepInfo,
    StatStg,
    MallocAllocation,
    MallocReallocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OwnedHandleCleanup {
    DeleteObject,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BufferCountUnit {
    Elements,
    Bytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TypedBufferSizing {
    SingleCall,
    FixedCapacity,
    TwoCall { max_retries: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TypedBufferRelation {
    Input {
        count_param_index: usize,
        actual_length_param_index: Option<usize>,
        unit: BufferCountUnit,
    },
    CallerOutput {
        capacity_param_index: usize,
        actual_length_param_index: Option<usize>,
        unit: BufferCountUnit,
        sizing: TypedBufferSizing,
    },
    EnumeratorNext {
        capacity_param_index: usize,
        fetched_param_index: usize,
        fetched_optional_for_single: bool,
    },
    CalleeAllocated {
        count_param_index: usize,
        unit: BufferCountUnit,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TypedBufferPlan {
    pub(super) buffer_param_index: usize,
    pub(super) element: ComType,
    pub(super) relation: TypedBufferRelation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SharedCountPlan {
    StringInputScalarOutput {
        count_param_index: usize,
        string_input_param_index: usize,
        scalar_output_param_index: usize,
    },
    Parallel {
        count_param_index: usize,
        input_param_indices: Vec<usize>,
        output_param_indices: Vec<usize>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ProjectedInterfaceRef {
    pub(super) namespace: String,
    pub(super) name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProjectedComMethodKind {
    Normal,
    FixedCapacityBytes {
        guid_param_index: usize,
    },
    CallerSuppliedDynamicIid {
        iid_param_index: usize,
        output_param_index: usize,
    },
    SynthesizedGetForWindow {
        iid_param_index: usize,
        output_param_index: usize,
        target_iid: String,
    },
    DispatchInvoke {
        result_param_index: usize,
        excep_info_param_index: usize,
        arg_err_param_index: usize,
    },
    EnumeratorNext {
        buffer_param_index: usize,
        capacity_param_index: usize,
        fetched_param_index: usize,
        fetched_optional_for_single: bool,
        interface: Option<ProjectedInterfaceRef>,
    },
    OwningCallerOutput {
        buffer_param_index: usize,
        capacity_param_index: usize,
    },
    FlagSelectedString {
        discriminator_param_index: usize,
        reserved_null_param_index: usize,
        buffer_param_index: usize,
        capacity_param_index: usize,
        string_flags: [u32; 2],
        validation_flag: u32,
    },
    ConditionalInterfaceOutput {
        public_input_param_indices: [Option<usize>; 3],
        flags_param_index: usize,
        context_param_index: usize,
        synchronous_output_param_index: Option<usize>,
        semisynchronous_output_param_index: Option<usize>,
        synchronous_flags: i32,
        semisynchronous_flags: i32,
    },
}

/// The JS runtime `typeof`/shape category a validated value uses at a call
/// site. Only types with a single, unambiguous JS shape participate in
/// overload dispatch — anything that can present as more than one shape (or
/// that overlaps another candidate's shape, e.g. `Buffer` inputs being
/// `typeof 'object'` just like a projected COM object) is deliberately left
/// unclassified (`None` from `dispatch_shape`) so overload grouping fails
/// closed instead of guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DispatchShape {
    Boolean,
    Number,
    BigInt,
    String,
    Object,
}

/// How a single method within an overload group is selected at a call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OverloadDispatch {
    /// This method is the only overload with this argument count; dispatch by
    /// `arguments.length` alone.
    Arity,
    /// Multiple overloads share this argument count; dispatch by inspecting
    /// the JS shape of the argument at `key_param_index`.
    ArityAndShape {
        key_param_index: usize,
        shape: DispatchShape,
    },
}

/// Identifies a method as one branch of a same-name overload set, and how the
/// public dispatcher routes to it. Built once during projection (see
/// `project::group_overloads`) — the renderer only *renders* this decision,
/// it never re-derives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OverloadInfo {
    /// The public JS name shared by every method in the group (e.g. `setOpacity`).
    pub(super) public_name: String,
    /// The private per-branch implementation name (e.g. `_setOpacity_7`).
    pub(super) impl_name: String,
    pub(super) dispatch: OverloadDispatch,
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
    pub(super) typed_buffers: Vec<TypedBufferPlan>,
    pub(super) shared_counts: Vec<SharedCountPlan>,
    pub(super) kind: ProjectedComMethodKind,
    pub(super) doc: Option<String>,
    pub(super) overload: Option<OverloadInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectedComSinkMethod {
    pub(super) vtable_index: usize,
    pub(super) handler_name: String,
    pub(super) return_convention: ComSinkReturnConvention,
    pub(super) output_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ComSinkReturnConvention {
    HResult,
    SemanticHResult,
    Void,
    Direct,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ComSinkPlan {
    pub(super) methods: Vec<ProjectedComSinkMethod>,
}

/// Classifies the JS runtime shape a validated `ComType` presents as, for
/// overload-dispatch purposes. Returns `None` for any type whose JS
/// representation is ambiguous or overlaps another candidate shape (pointer
/// types accept `Buffer`/`bigint`/`number` in ways that collide with other
/// categories), so overload grouping can fail closed rather than guess.
pub(super) fn dispatch_shape(typ: &ComType) -> Option<DispatchShape> {
    match typ {
        ComType::Primitive(ComPrimitive::Bool) | ComType::Win32Bool => Some(DispatchShape::Boolean),
        ComType::Primitive(
            ComPrimitive::I8
            | ComPrimitive::U8
            | ComPrimitive::I16
            | ComPrimitive::U16
            | ComPrimitive::I32
            | ComPrimitive::U32
            | ComPrimitive::F32
            | ComPrimitive::F64
            | ComPrimitive::Char16,
        )
        | ComType::HResult => Some(DispatchShape::Number),
        ComType::Primitive(ComPrimitive::I64 | ComPrimitive::U64)
        | ComType::NativeIsize
        | ComType::NativeUsize => Some(DispatchShape::BigInt),
        ComType::Guid | ComType::HString => Some(DispatchShape::String),
        ComType::Enum { underlying, .. } => match underlying {
            ComEnumUnderlying::I64 | ComEnumUnderlying::U64 => Some(DispatchShape::BigInt),
            _ => Some(DispatchShape::Number),
        },
        ComType::ScalarAlias { underlying, .. } => match underlying {
            ComScalarRepr::Primitive(ComPrimitive::I64 | ComPrimitive::U64)
            | ComScalarRepr::NativeIsize
            | ComScalarRepr::NativeUsize => Some(DispatchShape::BigInt),
            ComScalarRepr::Primitive(_) => Some(DispatchShape::Number),
        },
        ComType::ManagedInterface { .. } | ComType::DispatchParams => Some(DispatchShape::Object),
        // Raw/aliased pointers and BSTR accept multiple overlapping JS input
        // shapes (`bigint`, `number`, `Buffer`, `Uint8Array`, or `string`)
        // depending on position, so they can collide with any other category
        // and are never safe overload-dispatch keys.
        ComType::RawPointer
        | ComType::ExactNullPointer
        | ComType::AllocatorPointer
        | ComType::ConsumedAllocatorPointer
        | ComType::InspectedAllocatorPointer
        | ComType::GuidPointer
        | ComType::PointerAlias { .. }
        | ComType::NativePod { .. }
        | ComType::NativePodPointer { .. }
        | ComType::NativeUnionPointer { .. }
        | ComType::Variant
        | ComType::VariantByValue
        | ComType::SafeArray { .. }
        | ComType::PropVariant
        | ComType::ExcepInfo
        | ComType::StatStg
        | ComType::Bstr
        | ComType::CoTaskMemWideString
        | ComType::StringArray { .. }
        | ComType::TypedBuffer { .. }
        | ComType::OwningArray { .. } => None,
    }
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
    pub(super) namespace: String,
    pub(super) name: String,
    pub(super) underlying: ComEnumUnderlying,
    pub(super) members: Vec<ProjectedComEnumMember>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectedComInterface {
    pub(super) name: String,
    pub(super) namespace: String,
    pub(super) iid: String,
    pub(super) base_iids: Vec<String>,
    pub(super) is_iunknown_rooted: bool,
    pub(super) methods: Vec<ProjectedComMethod>,
    pub(super) activation: ActivationPlan,
    pub(super) referenced_enums: Vec<ProjectedComEnum>,
    pub(super) sink: Option<ComSinkPlan>,
    pub(super) evidence_dependencies: crate::contract_registry::EvidenceDependencies,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectedComCoclass {
    pub(super) name: String,
    pub(super) namespace: String,
    pub(super) clsid: String,
    pub(super) primary_interface: ProjectedComInterface,
    pub(super) associated_interfaces: Vec<ProjectedComInterface>,
}
