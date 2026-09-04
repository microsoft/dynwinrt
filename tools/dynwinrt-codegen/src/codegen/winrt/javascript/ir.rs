// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Projected IR types for JS/DTS code generation.
//!
//! The projection layer converts `ClassMeta`/`InterfaceMeta`/`TypeMeta` (raw
//! winmd metadata) into these semantic types. All projection decisions —
//! naming, type mapping, async detection, event pairing, import classification
//! — happen during projection. The renderers (`render_js`, `render_dts`) are
//! purely mechanical formatters that consume these types.

use std::collections::{BTreeMap, BTreeSet};

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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum JsArgumentKind {
    String,
    Number,
    BigInt,
    Boolean,
    Object,
    Array,
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
    /// `.close()` from the projected IClosable interface.
    Close {
        interface_name: String,
    },
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
    /// e.g. ["StreamedFileDataRequest._fromNative(__a0__)"]
    /// Empty if no wrapping needed (all params are primitives/DynWinRtValue)
    pub param_wraps: Vec<String>,
}

#[derive(Clone)]
pub struct ProjectedConstructor {
    /// Public TypeScript constructor overloads. Empty means the constructor is internal-only.
    pub overloads: Vec<Vec<ProjectedParam>>,
    /// Lines for the constructor body (JS only)
    pub body_lines: Vec<String>,
}

#[derive(Clone)]
pub struct ProjectedProperty {
    pub name: String,
    pub ts_type: String,
    pub setter_ts_type: Option<String>,
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
    pub argument_kinds: Vec<Option<JsArgumentKind>>,
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
    /// Original method name before OverloadAttribute rename (camelCase).
    /// When set, this method is part of an overload group and should be
    /// merged with other methods sharing the same `overload_of` name.
    pub overload_of: Option<String>,
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
    ToString {
        iface_name: String,
    },
    ToPrimitive,
    ToStringTag {
        tag: String,
    },
    Iterator {
        element_type: String,
        body_lines: Vec<String>,
    },
    CollectionLength,
    CollectionAt {
        element_type: String,
    },
    CollectionToArray {
        element_type: String,
    },
    IteratorNext {
        element_type: String,
    },
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

#[derive(Clone)]
pub struct ProjectedImport {
    pub symbols: Vec<String>,
    pub from: String,
    /// true = JS only (IID_*, PARAM_TYPES), not in DTS
    pub runtime_only: bool,
    /// true = DTS only (type aliases), not in JS
    pub dts_only: bool,
    /// true = the dynwinrt runtime package (`@microsoft/dynwinrt` or the
    /// `--import-name` override); prevents ESM->CJS conversion from
    /// misclassifying it as a sibling module.
    pub is_runtime_package: bool,
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

impl ProjectedFile {
    pub fn public_exports(&self) -> BTreeSet<String> {
        let mut exports = BTreeSet::new();
        exports.extend(self.classes.iter().map(|class| class.name.clone()));
        exports.extend(self.classes.iter().flat_map(|class| {
            class
                .required_ifaces
                .iter()
                .map(|interface| interface.name.clone())
        }));
        exports.extend(self.enums.iter().map(|en| en.name.clone()));
        exports.extend(
            self.ifaces
                .iter()
                .filter(|iface| !iface.is_delegate)
                .map(|iface| iface.name.clone()),
        );
        for structure in &self.structs {
            exports.insert(format!("pack{}", structure.name));
            exports.insert(format!("unpack{}", structure.name));
        }
        exports
    }

    pub fn internal_exports(&self) -> BTreeSet<String> {
        let mut exports = self
            .iid_consts
            .iter()
            .filter(|constant| constant.exported)
            .map(|constant| constant.name.clone())
            .collect::<BTreeSet<_>>();
        for structure in &self.structs {
            exports.insert(format!("{}_Type", structure.name));
        }
        for delegate in &self.delegates {
            exports.insert(format!("IID_{}", delegate.name));
            if delegate.has_param_types {
                exports.insert(format!("{}_PARAM_TYPES", delegate.name));
            }
        }
        exports
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedImport {
    pub source: String,
    pub symbols: BTreeSet<String>,
    pub runtime_only: bool,
    pub dts_only: bool,
    pub is_runtime_package: bool,
    pub canonical_dependency: Option<String>,
}

#[derive(Clone, Debug)]
pub struct GeneratedModule {
    pub canonical_module: String,
    pub package_subpath: String,
    pub javascript: Option<String>,
    pub declarations: Option<String>,
    pub imports: Vec<PlannedImport>,
    pub public_exports: BTreeSet<String>,
    pub primary_export: Option<String>,
    pub internal_exports: BTreeSet<String>,
    pub compatibility_aliases: BTreeSet<String>,
}

impl GeneratedModule {
    pub fn retained(canonical_module: impl Into<String>) -> Self {
        let canonical_module = canonical_module.into();
        Self {
            package_subpath: canonical_module.clone(),
            canonical_module,
            javascript: None,
            declarations: None,
            imports: Vec::new(),
            public_exports: BTreeSet::new(),
            primary_export: None,
            internal_exports: BTreeSet::new(),
            compatibility_aliases: BTreeSet::new(),
        }
    }
}

#[derive(Default)]
pub struct GenerationPlan {
    pub modules: BTreeMap<String, GeneratedModule>,
}

impl GenerationPlan {
    pub fn insert(&mut self, module: GeneratedModule) -> Result<(), String> {
        if let Some(previous) = self.modules.get(&module.canonical_module)
            && previous.javascript.is_some()
            && (previous.javascript != module.javascript
                || previous.declarations != module.declarations
                || previous.public_exports != module.public_exports
                || previous.internal_exports != module.internal_exports)
        {
            return Err(format!(
                "conflicting generated content for JavaScript canonical module `{}`",
                module.canonical_module
            ));
        }
        self.modules.insert(module.canonical_module.clone(), module);
        Ok(())
    }

    pub fn package_subpaths(&self) -> BTreeSet<String> {
        self.modules
            .values()
            .map(|module| module.package_subpath.clone())
            .collect()
    }

    pub fn render_root_index(&self) -> String {
        let mut output = String::from("// Generated by dynwinrt-codegen \u{2014} do not edit\n");
        let primary_owners = self
            .modules
            .values()
            .filter_map(|module| {
                module
                    .primary_export
                    .as_ref()
                    .map(|name| (name.clone(), module.canonical_module.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let mut seen = BTreeSet::new();
        let modules = self
            .modules
            .values()
            .filter(|module| module.canonical_module != "lifetime")
            .chain(self.modules.get("lifetime"));
        for module in modules {
            let exports = module
                .public_exports
                .iter()
                .filter(|name| {
                    primary_owners
                        .get(*name)
                        .is_none_or(|owner| owner == &module.canonical_module)
                        && seen.insert((*name).clone())
                })
                .cloned()
                .collect::<Vec<_>>();
            if !exports.is_empty() {
                output.push_str(&format!(
                    "export {{ {} }} from './{}.js';\n",
                    exports.join(", "),
                    module.canonical_module
                ));
            }
        }
        output
    }

    pub fn validate_dependencies(&self) -> Result<(), String> {
        for module in self.modules.values() {
            for import in &module.imports {
                if let Some(dependency) = &import.canonical_dependency
                    && !self.modules.contains_key(dependency)
                {
                    return Err(format!(
                        "JavaScript module `{}` requires missing canonical module `{dependency}`",
                        module.canonical_module
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod generation_plan_tests {
    use super::*;

    #[test]
    fn root_index_uses_structured_public_exports_only() {
        let mut plan = GenerationPlan::default();
        let mut module = GeneratedModule::retained("windows/foundation/Uri");
        module
            .public_exports
            .extend(["Uri".into(), "packPoint".into()]);
        module.internal_exports.insert("IID_Uri".into());
        module.primary_export = Some("Uri".into());
        plan.insert(module).unwrap();

        assert_eq!(
            plan.render_root_index(),
            "// Generated by dynwinrt-codegen — do not edit\n\
             export { Uri, packPoint } from './windows/foundation/Uri.js';\n"
        );
    }

    #[test]
    fn required_missing_canonical_dependencies_fail_closed() {
        let mut plan = GenerationPlan::default();
        let mut module = GeneratedModule::retained("contoso/Widget");
        module.imports.push(PlannedImport {
            source: "../shared/Missing.js".into(),
            symbols: ["Missing".into()].into_iter().collect(),
            runtime_only: false,
            dts_only: false,
            is_runtime_package: false,
            canonical_dependency: Some("shared/Missing".into()),
        });
        plan.insert(module).unwrap();

        assert_eq!(
            plan.validate_dependencies().unwrap_err(),
            "JavaScript module `contoso/Widget` requires missing canonical module `shared/Missing`"
        );
    }

    #[test]
    fn root_index_preserves_lifetime_as_the_final_module() {
        let mut plan = GenerationPlan::default();
        let mut lifetime = GeneratedModule::retained("lifetime");
        lifetime.public_exports.insert("projectAs".into());
        plan.insert(lifetime).unwrap();
        let mut module = GeneratedModule::retained("windows/foundation/Uri");
        module.public_exports.insert("Uri".into());
        plan.insert(module).unwrap();

        assert_eq!(
            plan.render_root_index(),
            "// Generated by dynwinrt-codegen — do not edit\n\
             export { Uri } from './windows/foundation/Uri.js';\n\
             export { projectAs } from './lifetime.js';\n"
        );
    }
}
