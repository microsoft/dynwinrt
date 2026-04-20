// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Projected IR types for JS/DTS code generation.
//!
//! The projection layer converts `ClassMeta`/`InterfaceMeta`/`TypeMeta` (raw
//! winmd metadata) into these semantic types. All projection decisions —
//! naming, type mapping, async detection, event pairing, import classification
//! — happen during projection. The renderers (`render_js`, `render_dts`) are
//! purely mechanical formatters that consume these types.

/// How a method invocation is expressed at runtime.
pub enum InvokeExpr {
    /// Normal: `iface_var.method(vtable_idx).invoke(obj_expr, [args])`
    VtableInvoke {
        iface_var: String,
        vtable_index: usize,
        obj_expr: String,
        args: Vec<String>,
    },
    /// Factory: `Class.f_IFactory().method(idx).invoke(DynWinRtValue.activationFactory('full_name'), [args])`
    FactoryInvoke {
        class_name: String,
        factory_accessor: String,
        vtable_index: usize,
        args: Vec<String>,
    },
    /// Activation: `_IActivationFactory.method(6).invoke(DynWinRtValue.activationFactory('{name}'), [])`
    ActivationInvoke { class_full_name: String },
    /// Custom body (e.g. IClosable delegation, Symbol.iterator, collection helpers)
    CustomBody(Vec<String>),
}

/// Async behavior of a method.
#[derive(Clone)]
pub enum AsyncKind {
    None,
    /// IAsyncAction → `Promise<void>`
    Action,
    /// IAsyncOperation<T> → `Promise<T>`; String = TS inner type
    Operation(String),
    /// IAsyncActionWithProgress<P> — String = TS progress type
    ActionWithProgress(String),
    /// IAsyncOperationWithProgress<T,P> — (result_type, progress_type)
    OperationWithProgress(String, String),
}

/// A projected member that appears in the class body.
#[derive(Clone)]
pub enum ProjectedMember {
    Constructor(ProjectedConstructor),
    Property(ProjectedProperty),
    Method(ProjectedMethod),
    Event(ProjectedEvent),
    Symbol(ProjectedSymbol),
    /// `.as<T>()` generic cast
    AsCast,
    /// `.close()` from IClosable
    Close,
}

#[derive(Clone)]
pub struct ProjectedParam {
    pub name: String,
    pub ts_type: String,
    pub optional: bool,
    /// If this param is a delegate, stores info for auto-wrapping in JS
    pub delegate_wrap: Option<DelegateWrapInfo>,
}

#[derive(Clone)]
pub struct DelegateWrapInfo {
    pub delegate_name: String,
    pub callback_type: String,
    /// Per-param conversion expressions for wrapping DynWinRtValue → high-level types
    /// e.g. ["new StreamedFileDataRequest(__a0__)"]
    /// Empty if no wrapping needed (all params are primitives/DynWinRtValue)
    pub param_wraps: Vec<String>,
}

#[derive(Clone)]
pub struct ProjectedConstructor {
    pub params: Vec<ProjectedParam>,
    /// Lines for the constructor body (JS only)
    pub body_lines: Vec<String>,
}

#[derive(Clone)]
pub struct ProjectedProperty {
    pub name: String,
    pub ts_type: String,
    pub readonly: bool,
    pub is_static: bool,
    pub doc: Option<DocInfo>,
    /// Full expression for getter return value (e.g. `_IFoo.method(6).invoke(this._obj, []).toString()`)
    pub getter_expr: String,
    /// Full invocation line for setter (e.g. `_IFoo.method(7).invoke(this._obj, [DynWinRtValue.hstring(value)]);`)
    pub setter_line: Option<String>,
}

#[derive(Clone)]
pub struct ProjectedMethod {
    pub name: String,
    pub doc: Option<DocInfo>,
    pub params: Vec<ProjectedParam>,
    pub return_type: String,
    pub async_kind: AsyncKind,
    pub is_static: bool,
    /// Raw invoke expression producing DynWinRtValue (used by async scaffolding)
    pub invoke_expr: String,
    /// For sync non-void: full return expression including conversion
    pub sync_return_expr: Option<String>,
    /// For async: expression to convert `_v` into the user-facing type
    pub async_convert_v: Option<String>,
    /// For WithProgress: expression to convert progress `_p` into the user-facing type
    pub progress_convert: Option<String>,
    /// True if the method has no return value (not async)
    pub is_void: bool,
    /// For array out params: pre-computed return expression
    pub array_return_expr: Option<String>,
    /// Delegate params that need wrapping in JS before invoke: (param_name, delegate_name)
    pub delegate_wraps: Vec<(String, String)>,
    /// If true, the method is emitted in JS but hidden from DTS declarations
    pub js_only: bool,
}

#[derive(Clone)]
pub struct ProjectedEvent {
    pub subscribe_name: String,
    pub unsubscribe_name: String,
    pub callback_type: String,
    pub doc: Option<DocInfo>,
    // JS body details
    pub delegate_name: Option<String>,
    pub add_iface_var: String,
    pub add_vtable_index: usize,
    pub add_obj_expr: String,
    pub remove_vtable_index: Option<usize>,
    pub remove_iface_var: String,
    pub remove_obj_expr: String,
    /// Whether the callback needs wrapping (sender/args conversion)
    pub needs_wrap: bool,
    pub sender_wrap: Option<String>,
    pub args_wrap: Option<String>,
}

#[derive(Clone)]
pub enum SymbolKind {
    ToString { iface_name: String },
    ToPrimitive,
    ToStringTag { tag: String },
    Iterator { element_type: String, body_lines: Vec<String> },
    CollectionLength,
    CollectionAt { element_type: String },
    CollectionToArray { element_type: String },
    IteratorNext { element_type: String },
}

#[derive(Clone)]
pub struct ProjectedSymbol {
    pub kind: SymbolKind,
    pub doc: Option<String>,
}

#[derive(Clone)]
pub struct DocInfo {
    pub summary: Option<String>,
    pub deprecated: Option<String>,
    pub returns: Option<String>,
    pub params: Vec<(String, String)>,
}

pub struct ProjectedImport {
    pub symbols: Vec<String>,
    pub from: String,
    /// true = JS only (IID_*, PARAM_TYPES), not in DTS
    pub runtime_only: bool,
    /// true = DTS only (type aliases), not in JS
    pub dts_only: bool,
}

/// Disposition of a required interface.
pub enum RequiredIfaceDisposition {
    /// Imported from its own generated file
    Imported,
    /// Inline wrapper class generated within this file
    InlineWrapper,
}

pub struct ProjectedRequiredIface {
    pub name: String,
    pub iid: String,
    pub disposition: RequiredIfaceDisposition,
    pub members: Vec<ProjectedMember>,
    pub registration: Option<String>,
    pub has_static_from: bool,
    pub has_parameterized_cast: bool,
}

pub struct ProjectedStruct {
    pub name: String,
    pub fields: Vec<(String, String)>,
    pub unpack_body: Vec<String>,
    pub pack_body: Vec<String>,
    pub type_expr: String,
    pub namespace: String,
}

pub struct ProjectedEnum {
    pub name: String,
    pub doc: Option<DocInfo>,
    pub members: Vec<ProjectedEnumMember>,
}

pub struct ProjectedEnumMember {
    pub name: String,
    pub value: i64,
    pub doc: Option<String>,
}

pub struct ProjectedClass {
    pub name: String,
    pub doc: Option<DocInfo>,
    pub members: Vec<ProjectedMember>,
    pub required_ifaces: Vec<ProjectedRequiredIface>,
    /// Static factory/static interface cache field declarations (JS only)
    pub static_cache_fields: Vec<String>,
    /// Static factory/static interface accessor methods (JS only)
    pub static_accessors: Vec<String>,
}

pub struct ProjectedIface {
    pub name: String,
    pub doc: Option<DocInfo>,
    pub iid_const: Option<ProjectedIidConst>,
    pub has_static_from: bool,
    pub has_parameterized_cast: bool,
    pub members: Vec<ProjectedMember>,
    pub is_delegate: bool,
}

pub struct ProjectedIidConst {
    pub name: String,
    pub rhs_expr: String,
    pub ts_type: String,
    /// Whether the IID should be `export const` (standalone interfaces) vs private `const` (class-owned)
    pub exported: bool,
}

pub struct ProjectedDelegate {
    pub name: String,
    pub iid_rhs: String,
    pub iid_ts_type: String,
    pub has_param_types: bool,
    pub param_types_expr: String,
    /// Callback type signature for .d.ts, e.g. `(stream: StreamedFileDataRequest) => void`
    pub callback_type: Option<String>,
}

/// A complete projected file ready for rendering.
pub struct ProjectedFile {
    pub name: String,
    pub imports: Vec<ProjectedImport>,
    /// IID constants (rendered as `const` in JS, `declare const` in DTS)
    pub iid_consts: Vec<ProjectedIidConst>,
    /// Interface registration blocks (JS only)
    pub registrations: Vec<String>,
    pub structs: Vec<ProjectedStruct>,
    pub classes: Vec<ProjectedClass>,
    pub enums: Vec<ProjectedEnum>,
    pub ifaces: Vec<ProjectedIface>,
    pub delegates: Vec<ProjectedDelegate>,
    /// Whether the file needs the _unwrap helper
    pub needs_unwrap_helper: bool,
    /// Whether the file needs the IActivationFactory registration (JS only)
    pub needs_activation_factory: bool,
}
