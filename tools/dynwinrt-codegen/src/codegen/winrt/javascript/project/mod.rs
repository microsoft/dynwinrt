// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Build projected IR from parsed WinRT metadata.
//!
//! All projection decisions — naming, type mapping, async detection, event
//! pairing, import classification, IStringable/IClosable detection — happen
//! here. The renderers consume the IR and only format.

mod collections;
mod constructors;
mod methods;
mod structs;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::codegen::winrt::extensions::winui::{self, WinUiAbiType};
use crate::meta::{
    ClassMeta, InterfaceMeta, MethodMeta, PIID_IOBSERVABLE_VECTOR, PIID_IVECTOR, ParamDirection,
};
use crate::types::{TypeKind, TypeMeta};

thread_local! {
    static RUNTIME_IMPORT_NAME: RefCell<String> = RefCell::new("@microsoft/dynwinrt".into());
}

/// Set the runtime package import name used in generated JS/TS files.
/// Must be called before any `project_*` functions.
pub fn set_import_name(name: &str) {
    RUNTIME_IMPORT_NAME.with(|n| *n.borrow_mut() = name.to_string());
}

pub fn get_import_name() -> String {
    RUNTIME_IMPORT_NAME.with(|n| n.borrow().clone())
}

use crate::codegen::winrt::shared::imports::{
    NO_DEFERRED, collect_iface_type_imports, collect_type_imports,
    collect_used_generics_from_class, collect_used_generics_from_methods, fill_array_output_index,
    fill_array_uses_retval_count, get_in_params, ireference_inner_type, method_abi_output_count,
};
use crate::codegen::winrt::shared::structs::{
    collect_used_structs_from_class, collect_used_structs_from_iface,
};

use super::ir::*;
use super::method::{
    ts_array_element_type, ts_param_type_dts, ts_param_type_safe, ts_return_type_safe,
};
use super::naming::{capitalize, infer_const_type, to_camel_case};
use super::signature::{
    build_args_expr, collect_runtime_class_iid_consts, convert_array_return, convert_return,
    generate_interface_registration, js_argument_kind, ref_marker, ts_dynwinrt_type, wrap_arg,
};
use super::structs::{struct_field_getter, struct_field_setter, ts_struct_field_type};

use collections::{
    project_collection_create, project_collection_helpers, should_skip_raw_collection_method,
};
use constructors::{default_activation_method_name, project_constructor};
use methods::{project_factory_method, project_instance_method, project_static_method};
use structs::project_struct_helpers;

// ======================================================================
// PIIDs of well-known collection interfaces
// ======================================================================
const PIID_IVECTOR_VIEW: &str = "bbe1fa4c-b0e3-4583-baef-1f1b2e483e56";
const PIID_IITERATOR: &str = "6a79e863-4300-459a-9966-cbb660963ee1";
const PIID_IITERABLE: &str = "faa585ea-6214-4217-afda-7f46de5869b3";
const PIID_IMAP: &str = "3c2925fe-8519-45c1-aa79-197b6718c1c1";
const PIID_IMAP_VIEW: &str = "e480ce40-a338-4ada-adcf-272272e48cb9";
const ICLOSABLE_IID: &str = "30d5a829-7fa4-4026-83bb-d75bae4ea99e";
const ISTRINGABLE_IID: &str = "96369f54-8eb6-48f0-abce-c1b211e627c3";

fn project_winui_abi_types(types: &[WinUiAbiType]) -> String {
    types
        .iter()
        .map(|typ| match typ {
            WinUiAbiType::Object => "DynWinRtType.object()",
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn interface_iid_rhs(iface: &InterfaceMeta) -> Option<String> {
    if let Some(ref piid) = iface.generic_piid {
        let args = iface
            .generic_args
            .iter()
            .map(ts_dynwinrt_type)
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!(
            "DynWinRtType.parameterized(WinGuid.parse('{}'), [{}]).iid()",
            piid, args
        ))
    } else if !iface.iid.is_empty() {
        Some(format!("WinGuid.parse('{}')", iface.iid))
    } else {
        None
    }
}

// ======================================================================
// Top-level projection functions
// ======================================================================

/// Build a map from delegate name → (TypeScript callback signature, referenced type names).
///
/// For example, `StreamedFileDataRequestedHandler` → `("(streamedFileDataRequest: StreamedFileDataRequest) => void", ["StreamedFileDataRequest"])`.
pub fn build_delegate_signatures(
    all_interfaces: &[InterfaceMeta],
    delegate_type_names: &HashSet<String>,
    known_types: &HashSet<String>,
) -> (
    HashMap<String, String>,
    HashMap<String, Vec<String>>,
    HashMap<String, Vec<String>>,
) {
    let mut sigs = HashMap::new();
    let mut refs = HashMap::new();
    let mut wraps = HashMap::new();
    for i in all_interfaces.iter().filter(|i| {
        i.methods.iter().any(|m| m.name == ".ctor") && i.methods.iter().any(|m| m.name == "Invoke")
    }) {
        let Some(invoke) = i.methods.iter().find(|m| m.name == "Invoke") else {
            continue;
        };
        let mut ref_types = Vec::new();
        let mut param_wraps = Vec::new();
        let in_params: Vec<_> = invoke
            .params
            .iter()
            .filter(|p| p.direction == ParamDirection::In)
            .collect();
        let params: Vec<String> = in_params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                let ts = ts_param_type_safe(&p.typ, known_types);
                let arg_var = format!("__a{}__", idx);
                // If the type is a known delegate, use DynWinRtValue (avoid recursion)
                let ts_clean = if delegate_type_names.contains(&ts) {
                    param_wraps.push(arg_var.clone());
                    "DynWinRtValue".to_string()
                } else {
                    if known_types.contains(&ts) {
                        ref_types.push(ts.clone());
                    }
                    // Build wrapping expression: new Type(argN) for known types, argN.toString() for string, etc.
                    let wrap =
                        convert_return(&arg_var, Some(&p.typ), false, known_types, &NO_DEFERRED);
                    param_wraps.push(wrap);
                    ts
                };
                format!("{}: {}", to_camel_case(&p.name), ts_clean)
            })
            .collect();
        let ret = if invoke.return_type.is_some() {
            "DynWinRtValue"
        } else {
            "void"
        };
        sigs.insert(
            i.name.clone(),
            format!("({}) => {}", params.join(", "), ret),
        );
        refs.insert(i.name.clone(), ref_types);
        wraps.insert(i.name.clone(), param_wraps);
    }
    (sigs, refs, wraps)
}

/// Project a single RuntimeClass into a ProjectedFile.
pub fn project_class(
    class: &ClassMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
    shared_iids: &HashSet<String>,
    delegate_sigs: &HashMap<String, String>,
    delegate_sig_refs: &HashMap<String, Vec<String>>,
    delegate_param_wraps: &HashMap<String, Vec<String>>,
) -> ProjectedFile {
    let used_structs = collect_used_structs_from_class(class);
    let winui_bootstrap = winui::resolve_application_bootstrap(class, known_types);
    let supports_unpackaged_xaml =
        winui_bootstrap.is_some_and(|bootstrap| bootstrap.supports_unpackaged_resources);

    // Collect delegate names only from interfaces of THIS class (not the entire batch)
    // for delegate imports; but also include global delegate_type_names for type filtering
    let mut delegate_names: HashSet<String> = HashSet::new();
    let all_ifaces: Vec<&InterfaceMeta> = class.all_interfaces().collect();
    for iface in &all_ifaces {
        collect_delegate_names_from_methods(&iface.methods, &mut delegate_names);
        collect_known_delegate_names_from_methods(
            &iface.methods,
            delegate_type_names,
            &mut delegate_names,
        );
    }
    // All known delegate type names (used to filter type imports — delegates typed as Interface
    // in parameter metadata must not be imported as regular type imports)
    let all_delegate_names: HashSet<String> =
        delegate_names.union(delegate_type_names).cloned().collect();

    // Build imports
    let mut imports = Vec::new();

    // Runtime import
    let has_structs = !used_structs.is_empty();
    let mut runtime_import = build_runtime_import(has_structs);
    if supports_unpackaged_xaml {
        runtime_import.symbols.extend([
            "getWinappsdkResourcePriPath".into(),
            "hasPackageIdentity".into(),
        ]);
    }
    imports.push(runtime_import);

    // Collection generics imports
    let collection_names = collect_used_generics_from_class(class);
    for cname in &collection_names {
        if !all_delegate_names.contains(cname) {
            imports.push(ProjectedImport {
                symbols: vec![cname.clone()],
                from: format!("./{}.js", cname),
                runtime_only: false,
                dts_only: false,
                is_runtime_package: false,
            });
        }
    }

    // Delegate imports (runtime only — IID + PARAM_TYPES)
    let mut sorted_delegates: Vec<_> = delegate_names.iter().collect();
    sorted_delegates.sort();
    for dname in &sorted_delegates {
        imports.push(ProjectedImport {
            symbols: vec![format!("IID_{}", dname), format!("{}_PARAM_TYPES", dname)],
            from: format!("./{}.js", dname),
            runtime_only: true,
            dts_only: false,
            is_runtime_package: false,
        });
        // DTS-only: import the delegate type alias (for typed param signatures)
        if delegate_sigs.contains_key(*dname) {
            imports.push(ProjectedImport {
                symbols: vec![dname.to_string()],
                from: format!("./{}.js", dname),
                runtime_only: false,
                dts_only: true,
                is_runtime_package: false,
            });
        }
    }

    // Type imports
    let mut imported_names: HashSet<String> = HashSet::new();
    let type_imports = collect_type_imports(class);
    let mut sorted_imports: Vec<_> = type_imports.iter().collect();
    sorted_imports
        .sort_by(|a, b| (&a.namespace, &a.name, &a.kind).cmp(&(&b.namespace, &b.name, &b.kind)));
    for r in &sorted_imports {
        // Never emit an import from the file that's currently being generated.
        // `collect_type_imports` runs with `include_self_interfaces: true` so
        // the class's own primary interface (which shares the class name) can
        // appear here; letting it through emits `import { X } from './X.js'`
        // in `X.js`, which ESM treats as a duplicate identifier at parse time.
        if r.name == class.name {
            continue;
        }
        if known_types.contains(&r.name) && !all_delegate_names.contains(&r.name) {
            imports.push(format_type_import_projected(&r.name, r.kind));
            imported_names.insert(r.name.clone());
            if r.kind == TypeKind::Interface {
                imported_names.insert(format!("IID_{}", r.name));
            }
        } else if all_delegate_names.contains(&r.name)
            && delegate_sigs.contains_key(&r.name)
            && !delegate_names.contains(&r.name)
        {
            // Delegate typed as Interface in params — add DTS-only type import
            imports.push(ProjectedImport {
                symbols: vec![r.name.clone()],
                from: format!("./{}.js", r.name),
                runtime_only: false,
                dts_only: true,
                is_runtime_package: false,
            });
            imported_names.insert(r.name.clone());
        }
    }

    if let Some(bootstrap) = winui_bootstrap {
        let spec = bootstrap.spec;
        let mut names = vec![spec.metadata_provider.name, spec.controls_resources.name];
        if bootstrap.supports_unpackaged_resources {
            names.push(spec.resource_manager.name);
        }
        for name in names {
            if imported_names.insert(name.into()) {
                imports.push(format_type_import_projected(name, TypeKind::Class));
            }
        }
    }

    // Import shared required interfaces
    for req_iface in &class.required_interfaces {
        if req_iface.generic_piid.is_none()
            && !req_iface.iid.is_empty()
            && shared_iids.contains(&req_iface.iid)
            && !imported_names.contains(&req_iface.name)
        {
            imports.push(format_type_import_projected(
                &req_iface.name,
                TypeKind::Interface,
            ));
            imported_names.insert(req_iface.name.clone());
            imported_names.insert(format!("IID_{}", req_iface.name));
        }
    }

    // Import types referenced in delegate callback signatures (e.g. StreamedFileDataRequest)
    for dname in &sorted_delegates {
        if let Some(ref_types) = delegate_sig_refs.get(*dname) {
            for rt in ref_types {
                if !imported_names.contains(rt) && !delegate_names.contains(rt) && rt != &class.name
                {
                    imports.push(format_type_import_projected(rt, TypeKind::Class));
                    imported_names.insert(rt.clone());
                }
            }
        }
    }

    // Preemptive IClosable import: `close()` (added later) references
    // IClosable by name. Register the import here so the IID-const loop below
    // sees `IID_IClosable` in `imported_names` and skips declaring it,
    // avoiding a duplicate identifier in single-class emission.
    let needs_iclosable = class.name != "IClosable"
        && class
            .required_interfaces
            .iter()
            .any(|ri| ri.iid == ICLOSABLE_IID);
    if needs_iclosable && !imported_names.contains("IClosable") {
        imports.push(format_type_import_projected(
            "IClosable",
            TypeKind::Interface,
        ));
        imported_names.insert("IClosable".into());
        imported_names.insert("IID_IClosable".into());
    }

    // IID consts(private, for class-internal use)
    let mut iid_consts: Vec<ProjectedIidConst> = Vec::new();
    let all_class_ifaces: Vec<&InterfaceMeta> = class.all_interfaces().collect();
    let mut declared_iids: HashSet<String> = HashSet::new();
    for iface in &all_class_ifaces {
        let iid_name = format!("IID_{}", iface.name);
        let Some(rhs_expr) = interface_iid_rhs(iface) else {
            continue;
        };
        if declared_iids.contains(&iid_name) {
            continue;
        }
        if imported_names.contains(&iid_name) {
            continue;
        }
        iid_consts.push(ProjectedIidConst {
            name: iid_name.clone(),
            rhs_expr,
            ts_type: "WinGuid".into(),
            exported: false,
        });
        declared_iids.insert(iid_name);
    }
    // Export `IID_<ClassName>` = default interface's IID, so downstream files that
    // reference the class via a synthesized Interface typeref (e.g.
    // `import { IID_UIElement, UIElement } from './UIElement.js'`) resolve to a
    // valid COM QueryInterface IID. Without this, ESM import fails with
    // "does not provide an export named IID_<ClassName>".
    if let Some(ref di) = class.default_interface {
        if di.name != class.name {
            let alias_name = format!("IID_{}", class.name);
            if let Some(rhs_expr) = interface_iid_rhs(di)
                && !declared_iids.contains(&alias_name)
                && !imported_names.contains(&alias_name)
            {
                iid_consts.push(ProjectedIidConst {
                    name: alias_name.clone(),
                    rhs_expr,
                    ts_type: "WinGuid".into(),
                    exported: true,
                });
                declared_iids.insert(alias_name);
            }
        }
    }
    let mut argument_iids = Vec::new();
    for iface in &all_class_ifaces {
        for method in &iface.methods {
            for parameter in &method.params {
                if parameter.direction == ParamDirection::In {
                    collect_runtime_class_iid_consts(&parameter.typ, &mut argument_iids);
                }
            }
        }
    }
    argument_iids.sort();
    argument_iids.dedup();
    for (name, iid_expr) in argument_iids {
        if declared_iids.insert(name.clone()) && !imported_names.contains(&name) {
            iid_consts.push(ProjectedIidConst {
                name,
                rhs_expr: iid_expr,
                ts_type: "WinGuid".into(),
                exported: false,
            });
        }
    }

    // Interface registrations
    let mut registrations = Vec::new();
    // Track already-emitted registration variable names so we never emit the
    // same `_IFoo` block twice — this defends against class metadata that
    // lists the same interface (e.g. `IFrameworkView`) as both the class's
    // default interface *and* one of its required interfaces (a well-known
    // pattern in `Windows.ApplicationModel.Core`).
    let mut emitted_reg_vars: HashSet<String> = HashSet::new();
    let push_registration =
        |registrations: &mut Vec<String>, emitted: &mut HashSet<String>, iface: &InterfaceMeta| {
            let var_name = format!("_{}", iface.name);
            if !emitted.insert(var_name.clone()) {
                return;
            }
            registrations.push(generate_interface_registration(iface, &var_name));
        };
    if let Some(ref iface) = class.default_interface {
        push_registration(&mut registrations, &mut emitted_reg_vars, iface);
    }
    for iface in &class.factory_interfaces {
        push_registration(&mut registrations, &mut emitted_reg_vars, iface);
    }
    for iface in &class.static_interfaces {
        push_registration(&mut registrations, &mut emitted_reg_vars, iface);
    }
    for iface in &class.required_interfaces {
        // Register locally regardless of shared/imported status — the flatten
        // step below references `_<InterfaceName>.method(...)` in every method
        // body, so the registration must exist in this file. `registerInterface`
        // is idempotent in the runtime (dedup by IID), so re-registering a
        // shared interface across files is safe.
        push_registration(&mut registrations, &mut emitted_reg_vars, iface);
    }
    // Struct helpers
    let structs = project_struct_helpers(&used_structs);

    // Build class members
    let mut members = Vec::new();

    // Public activation constructor, or an inaccessible constructor for system-returned classes.
    members.push(ProjectedMember::Constructor(project_constructor(
        class,
        known_types,
        &delegate_names,
        delegate_sigs,
        delegate_param_wraps,
    )));
    let tracker = ref_marker("trackProjectedValue");
    let owned_obj = if let Some(ref iface) = class.default_interface {
        if iface.iid.is_empty() {
            format!("{}(obj, '{}')", tracker, class.name)
        } else {
            let caster = ref_marker("castProjectedValueOwned");
            format!("{}(obj, IID_{}, '{}')", caster, iface.name, class.name)
        }
    } else {
        format!("{}(obj, '{}')", tracker, class.name)
    };
    let borrowed_obj = if let Some(ref iface) = class.default_interface {
        if iface.iid.is_empty() {
            format!("{}(obj, '{}')", tracker, class.name)
        } else {
            let caster = ref_marker("castProjectedValueBorrowed");
            format!("{}(obj, IID_{}, '{}')", caster, iface.name, class.name)
        }
    } else {
        format!("{}(obj, '{}')", tracker, class.name)
    };
    members.push(ProjectedMember::Method(ProjectedMethod {
        name: "_fromNative".into(),
        doc: None,
        params: vec![ProjectedParam {
            name: "obj".into(),
            ts_type: "DynWinRtValue".into(),
            optional: false,
            delegate_wrap: None,
        }],
        argument_kinds: vec![],
        return_type: class.name.clone(),
        async_kind: AsyncKind::None,
        is_static: true,
        invoke_expr: String::new(),
        sync_return_expr: Some(format!(
            "Object.assign(Object.create({0}.prototype), {{ _obj: {1} }})",
            class.name, owned_obj
        )),
        async_convert_v: None,
        progress_convert: None,
        is_void: false,
        array_return_expr: None,
        delegate_wraps: vec![],
        js_only: true,
        overload_of: None,
    }));
    members.push(ProjectedMember::Method(ProjectedMethod {
        name: "_fromNativeBorrowed".into(),
        doc: None,
        params: vec![ProjectedParam {
            name: "obj".into(),
            ts_type: "DynWinRtValue".into(),
            optional: false,
            delegate_wrap: None,
        }],
        argument_kinds: vec![],
        return_type: class.name.clone(),
        async_kind: AsyncKind::None,
        is_static: true,
        invoke_expr: String::new(),
        sync_return_expr: Some(format!(
            "Object.assign(Object.create({0}.prototype), {{ _obj: {1} }})",
            class.name, borrowed_obj
        )),
        async_convert_v: None,
        progress_convert: None,
        is_void: false,
        array_return_expr: None,
        delegate_wraps: vec![],
        js_only: true,
        overload_of: None,
    }));

    // Default constructor (static create/createDefault)
    if class.has_default_constructor {
        let ctor_name = default_activation_method_name(class);
        members.push(ProjectedMember::Method(ProjectedMethod {
            name: ctor_name.into(),
            doc: Some(DocInfo {
                summary: Some(format!("Create a new `{}` instance.", class.name)),
                deprecated: None,
                returns: None,
                params: vec![],
            }),
            params: vec![],
            argument_kinds: vec![],
            return_type: class.name.clone(),
            async_kind: AsyncKind::None,
            is_static: true,
            invoke_expr: format!(
                "_IActivationFactory.method(6).invoke(DynWinRtValue.activationFactory('{}'), [])",
                class.full_name
            ),
            sync_return_expr: Some(format!(
                "{}._fromNative(_IActivationFactory.method(6).invoke(DynWinRtValue.activationFactory('{}'), []))",
                class.name, class.full_name
            )),
            async_convert_v: None,
            is_void: false,
            array_return_expr: None,
            delegate_wraps: vec![],
            progress_convert: None,
            js_only: false, overload_of: None,
        }));
    }

    // Factory methods
    let has_explicit_create_factory = class.factory_interfaces.iter().any(|iface| {
        iface.methods.iter().any(|m| {
            let camel = to_camel_case(&m.name);
            camel == "create"
        })
    });
    for iface in &class.factory_interfaces {
        for method in &iface.methods {
            let projected = project_factory_method(
                class,
                iface,
                method,
                known_types,
                &delegate_names,
                delegate_sigs,
                delegate_param_wraps,
            );
            members.push(projected.clone());

            // Composable factories commonly expose a no-argument
            // `CreateInstance` method. Preserve the real API shape
            // (`createInstance`) and add an ergonomic `create()` alias when
            // there is no other explicit `create` factory/default constructor.
            if !class.has_default_constructor
                && !has_explicit_create_factory
                && method.name == "CreateInstance"
                && get_in_params(method).is_empty()
            {
                if let ProjectedMember::Method(mut alias) = projected {
                    alias.name = "create".into();
                    alias.doc = Some(DocInfo {
                        summary: Some(format!(
                            "Create a new `{}` instance. Alias for `createInstance()`.",
                            class.name
                        )),
                        deprecated: None,
                        returns: None,
                        params: vec![],
                    });
                    alias.overload_of = None;
                    members.push(ProjectedMember::Method(alias));
                }
            }
        }
    }

    if let Some(bootstrap) = winui_bootstrap {
        let spec = bootstrap.spec;
        let launched_callback_types = project_winui_abi_types(spec.launched_callback_params);
        let unpackaged_resource_setup = if bootstrap.supports_unpackaged_resources {
            format!(
                "if (!hasPackageIdentity()) {{ const _resourceManager = (__get_{resource_manager}()).createInstance(getWinappsdkResourcePriPath()); _app.onResourceManagerRequested((_sender, args) => {{ if (args === null) throw new Error('WinUI ResourceManagerRequested did not supply event arguments'); args.customResourceManager = _resourceManager; }}); }}",
                resource_manager = spec.resource_manager.name,
            )
        } else {
            String::new()
        };
        members.push(ProjectedMember::Method(ProjectedMethod {
            name: "createWithMetadataProvider".into(),
            doc: Some(DocInfo {
                summary: Some(
                    "Create a composed `Application` that exposes the supplied WinUI XAML metadata provider."
                        .into(),
                ),
                deprecated: None,
                returns: None,
                params: vec![(
                    "onLaunched".into(),
                    "Runs from `Application.OnLaunched`, when XAML resources and windows can be initialized."
                        .into(),
                )],
            }),
            params: vec![
                ProjectedParam {
                    name: "metadataProvider".into(),
                    ts_type: spec.metadata_provider.name.into(),
                    optional: false,
                    delegate_wrap: None,
                },
                ProjectedParam {
                    name: "onLaunched".into(),
                    ts_type: "() => void".into(),
                    optional: true,
                    delegate_wrap: None,
                },
            ],
            argument_kinds: vec![],
            return_type: class.name.clone(),
            async_kind: AsyncKind::None,
            is_static: true,
            invoke_expr: String::new(),
            sync_return_expr: Some(format!(
                "(() => {{ const _launched = onLaunched == null ? null : DynWinRtDelegate.create(WinGuid.parse('{callback_iid}'), [{callback_types}], onLaunched).toValue(); return {class_name}._fromNative(DynWinRtValue.createXamlApplication(_unwrap(metadataProvider), _launched)); }})()",
                callback_iid = spec.launched_callback_iid,
                callback_types = launched_callback_types,
                class_name = class.name,
            )),
            async_convert_v: None,
            progress_convert: None,
            is_void: false,
            array_return_expr: None,
            delegate_wraps: vec![],
            js_only: false,
            overload_of: None,
        }));
        members.push(ProjectedMember::Method(ProjectedMethod {
            name: "create".into(),
            doc: Some(DocInfo {
                summary: Some(
                    "Create a composed `Application`, configure unpackaged resource resolution, and install WinUI's default Fluent resources before the launch callback."
                        .into(),
                ),
                deprecated: None,
                returns: None,
                params: vec![(
                    "onLaunched".into(),
                    "Runs after `XamlControlsResources` has been added to the application resources."
                        .into(),
                )],
            }),
            params: vec![ProjectedParam {
                name: "onLaunched".into(),
                ts_type: "() => void".into(),
                optional: true,
                delegate_wrap: None,
            }],
            argument_kinds: vec![],
            return_type: class.name.clone(),
            async_kind: AsyncKind::None,
            is_static: true,
            invoke_expr: String::new(),
            sync_return_expr: Some(format!(
                "(() => {{ const _provider = (__get_{provider}()).create(); let _resourcesInitialized = false; const _launched = DynWinRtDelegate.create(WinGuid.parse('{callback_iid}'), [{callback_types}], () => {{ const _app = {class_name}.current; if (_app === null) throw new Error('WinUI Application.Current is unavailable during OnLaunched'); if (!_resourcesInitialized) {{ _app.resources.mergedDictionaries.append((__get_{resources}()).create()); _resourcesInitialized = true; }} onLaunched?.(); }}); const _app = {class_name}._fromNative(DynWinRtValue.createXamlApplication(_unwrap(_provider), _launched.toValue())); {unpackaged_resource_setup} return _app; }})()",
                callback_iid = spec.launched_callback_iid,
                callback_types = launched_callback_types,
                provider = spec.metadata_provider.name,
                resources = spec.controls_resources.name,
                class_name = class.name,
            )),
            async_convert_v: None,
            progress_convert: None,
            is_void: false,
            array_return_expr: None,
            delegate_wraps: vec![],
            js_only: false,
            overload_of: None,
        }));
    }

    // Static methods
    for iface in &class.static_interfaces {
        for method in &iface.methods {
            let projected = project_static_method(
                class,
                iface,
                method,
                known_types,
                &delegate_names,
                delegate_sigs,
                delegate_param_wraps,
            );
            if winui::is_application(class) && method.name == "Start" {
                if let ProjectedMember::Method(projected_method) = &projected {
                    let mut scheduled = projected_method.clone();
                    scheduled.name = "startScheduled".into();
                    scheduled.return_type = "Promise<void>".into();
                    scheduled.async_kind = AsyncKind::None;
                    scheduled.is_void = false;
                    let invoke_expr =
                        scheduled
                            .invoke_expr
                            .replacen(".invoke(", ".invokeScheduled(", 1);
                    scheduled.invoke_expr = invoke_expr.clone();
                    scheduled.sync_return_expr = Some(invoke_expr);
                    scheduled.async_convert_v = None;
                    scheduled.progress_convert = None;
                    scheduled.array_return_expr = None;
                    scheduled.overload_of = None;
                    members.push(projected);
                    members.push(ProjectedMember::Method(scheduled));
                    continue;
                }
            }
            members.push(projected);
        }
    }

    // Instance methods (from default interface)
    if let Some(ref default_iface) = class.default_interface {
        let iface_var = format!("_{}", default_iface.name);
        for method in &default_iface.methods {
            if should_skip_raw_collection_method(default_iface, &method.name) {
                continue;
            }
            if let Some(m) = project_instance_method(
                &iface_var,
                "this._obj",
                method,
                known_types,
                &delegate_names,
                Some(&default_iface.methods),
                delegate_sigs,
                delegate_param_wraps,
            ) {
                members.push(m);
            }
        }
        project_collection_helpers(
            default_iface,
            known_types,
            &mut members,
            &mut imports,
            "this._obj",
        );
    }

    // Merge overload names in default interface members
    merge_overload_names(&mut members);

    // IClosable → close()  (import already registered pre-emptively above)
    if class
        .required_interfaces
        .iter()
        .any(|ri| ri.iid == ICLOSABLE_IID)
    {
        members.push(ProjectedMember::Close);
    }

    // IStringable → toString, toPrimitive, toStringTag
    let has_istringable = class
        .required_interfaces
        .iter()
        .any(|ri| ri.iid == ISTRINGABLE_IID);
    if has_istringable {
        let iface_name = class
            .required_interfaces
            .iter()
            .find(|ri| ri.iid == ISTRINGABLE_IID)
            .map(|ri| ri.name.clone())
            .unwrap_or_else(|| "IStringable".into());
        members.push(ProjectedMember::Symbol(ProjectedSymbol {
            kind: SymbolKind::ToString {
                iface_name: iface_name.clone(),
            },
            doc: None,
        }));
        members.push(ProjectedMember::Symbol(ProjectedSymbol {
            kind: SymbolKind::ToPrimitive,
            doc: None,
        }));
        members.push(ProjectedMember::Symbol(ProjectedSymbol {
            kind: SymbolKind::ToStringTag {
                tag: class.name.clone(),
            },
            doc: None,
        }));
    }

    // .as() method
    if !class.required_interfaces.is_empty() {
        members.push(ProjectedMember::AsCast);
    }

    // Static cache fields + accessors
    let mut static_cache_fields = Vec::new();
    let mut static_accessors = Vec::new();
    let mut declared: HashSet<String> = HashSet::new();
    for iface in &class.factory_interfaces {
        let key = format!("f_{}", iface.name);
        if !iface.iid.is_empty() && declared.insert(key.clone()) {
            static_cache_fields.push(format!("static _{};", key));
            static_accessors.push(format!(
                "static {k}() {{ return {cls}._{k} ??= DynWinRtValue.activationFactory('{full}').cast(IID_{iface}); }}",
                k = key, cls = class.name, iface = iface.name, full = class.full_name
            ));
        }
    }
    for iface in &class.static_interfaces {
        let key = format!("s_{}", iface.name);
        if !iface.iid.is_empty() && declared.insert(key.clone()) {
            static_cache_fields.push(format!("static _{};", key));
            static_accessors.push(format!(
                "static {k}() {{ return {cls}._{k} ??= DynWinRtValue.activationFactory('{full}').cast(IID_{iface}); }}",
                k = key, cls = class.name, iface = iface.name, full = class.full_name
            ));
        }
    }

    // Required interface inline wrappers
    let mut required_ifaces = Vec::new();
    // Track names already on the main class to avoid conflicts
    let mut main_member_names: HashSet<String> = members
        .iter()
        .filter_map(|m| match m {
            ProjectedMember::Method(pm) => Some(pm.name.clone()),
            ProjectedMember::Property(pp) => Some(pp.name.clone()),
            ProjectedMember::Event(pe) => Some(pe.subscribe_name.clone()),
            ProjectedMember::Symbol(ps) => Some(symbol_dedup_key(&ps.kind)),
            ProjectedMember::Close => Some("close".into()),
            ProjectedMember::AsCast => Some("as".into()),
            _ => None,
        })
        .collect();

    for req_iface in &class.required_interfaces {
        if req_iface.iid.is_empty() {
            continue;
        }
        let is_imported = imported_names.contains(&req_iface.name);

        let reg_var = format!("_{}", req_iface.name);
        let mut ri_members = Vec::new();
        // Use cast expression for the flattened methods so the COM pointer
        // targets the correct interface vtable.
        let cast_obj = format!("this._obj.cast(IID_{})", req_iface.name);
        for method in &req_iface.methods {
            if should_skip_raw_collection_method(req_iface, &method.name) {
                continue;
            }
            if let Some(m) = project_instance_method(
                &reg_var,
                &cast_obj,
                method,
                known_types,
                &delegate_names,
                Some(&req_iface.methods),
                delegate_sigs,
                delegate_param_wraps,
            ) {
                ri_members.push(m);
            }
        }
        project_collection_helpers(
            req_iface,
            known_types,
            &mut ri_members,
            &mut imports,
            &cast_obj,
        );

        // Also build members with this._obj for the standalone interface class
        // (only emitted when we produce an inline wrapper below).
        let mut ri_own_members = Vec::new();
        if !is_imported {
            for method in &req_iface.methods {
                if should_skip_raw_collection_method(req_iface, &method.name) {
                    continue;
                }
                if let Some(m) = project_instance_method(
                    &reg_var,
                    "this._obj",
                    method,
                    known_types,
                    &delegate_names,
                    Some(&req_iface.methods),
                    delegate_sigs,
                    delegate_param_wraps,
                ) {
                    ri_own_members.push(m);
                }
            }
            project_collection_helpers(
                req_iface,
                known_types,
                &mut ri_own_members,
                &mut imports,
                "this._obj",
            );
        }

        // Merge overload names within the required interface members before flatten
        merge_overload_names(&mut ri_members);

        // Flatten: copy members onto the main class.
        // Always flatten, even when the interface is shared/imported — the
        // imported .js file gives users an escape hatch (`new IContentControl(x)...`),
        // but the main class must still expose inherited members directly so that
        // e.g. `button.background = ...` (from Control) reaches the underlying COM
        // vtable instead of silently creating a JS-only field.
        // Allow multiple methods with the same name (overloads).
        for member in &ri_members {
            let name = match member {
                ProjectedMember::Method(pm) => pm.name.clone(),
                ProjectedMember::Property(pp) => pp.name.clone(),
                ProjectedMember::Event(pe) => pe.subscribe_name.clone(),
                ProjectedMember::Symbol(ps) => symbol_dedup_key(&ps.kind),
                ProjectedMember::Close => "close".into(),
                ProjectedMember::AsCast => "as".into(),
                _ => continue,
            };
            // For methods: allow same name (overloads), dedup by name+param count.
            // Also check plain name to prevent duplicating Symbol-based members (e.g. toString).
            if let ProjectedMember::Method(pm) = member {
                if main_member_names.contains(&name) {
                    continue; // already exists as a Symbol or other member
                }
                let sig_key = format!("{}#{}", name, pm.params.len());
                if main_member_names.insert(sig_key) {
                    members.push(member.clone());
                }
            } else if main_member_names.insert(name) {
                members.push(member.clone());
            }
        }

        // Inline wrapper class inside this file — only when the interface is NOT
        // imported from a shared IContentControl.js. When imported, downstream
        // code uses that standalone file's class instead.
        if !is_imported {
            required_ifaces.push(ProjectedRequiredIface {
                name: req_iface.name.clone(),
                iid: req_iface.iid.clone(),
                disposition: RequiredIfaceDisposition::InlineWrapper,
                members: ri_own_members,
                registration: None,
                has_static_from: true,
                has_parameterized_cast: false,
            });
        }
    }

    // Merge overloaded method names: rename `foo2`, `foo3` to `foo` when `foo` exists.
    // Must happen after flatten so required-interface methods are included.
    merge_overload_names(&mut members);

    // Check if _unwrap is used
    let needs_unwrap = check_needs_unwrap(&members, &required_ifaces);

    let doc = build_doc_info(class.doc.as_deref(), class.deprecated.as_deref(), None, &[]);

    ProjectedFile {
        name: class.name.clone(),
        imports,
        iid_consts,
        registrations,
        structs,
        classes: vec![ProjectedClass {
            name: class.name.clone(),
            doc,
            members,
            required_ifaces,
            static_cache_fields,
            static_accessors,
        }],
        enums: vec![],
        ifaces: vec![],
        delegates: vec![],
        needs_unwrap_helper: needs_unwrap,
        needs_activation_factory: class.has_default_constructor,
    }
}

/// Project a single interface into a ProjectedFile.
pub fn project_interface(
    iface: &InterfaceMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
    delegate_sigs: &HashMap<String, String>,
    delegate_sig_refs: &HashMap<String, Vec<String>>,
    delegate_param_wraps: &HashMap<String, Vec<String>>,
) -> ProjectedFile {
    // Check if delegate
    let is_delegate = iface.methods.iter().any(|m| m.name == ".ctor")
        && iface.methods.iter().any(|m| m.name == "Invoke");
    if is_delegate {
        return project_delegate(iface, delegate_sigs, delegate_sig_refs);
    }

    let used_structs = collect_used_structs_from_iface(iface);
    let has_structs = !used_structs.is_empty();

    let mut delegate_names: HashSet<String> = HashSet::new();
    collect_delegate_names_from_methods(&iface.methods, &mut delegate_names);
    collect_known_delegate_names_from_methods(
        &iface.methods,
        delegate_type_names,
        &mut delegate_names,
    );

    // Build imports
    let mut imports = Vec::new();
    imports.push(build_runtime_import(has_structs));

    let collection_names = collect_used_generics_from_methods(&iface.methods);
    for cname in &collection_names {
        if cname != &iface.name && !delegate_names.contains(cname) {
            imports.push(ProjectedImport {
                symbols: vec![cname.clone()],
                from: format!("./{}.js", cname),
                runtime_only: false,
                dts_only: false,
                is_runtime_package: false,
            });
        }
    }

    let mut sorted_delegates: Vec<_> = delegate_names.iter().collect();
    sorted_delegates.sort();
    for dname in &sorted_delegates {
        imports.push(ProjectedImport {
            symbols: vec![format!("IID_{}", dname), format!("{}_PARAM_TYPES", dname)],
            from: format!("./{}.js", dname),
            runtime_only: true,
            dts_only: false,
            is_runtime_package: false,
        });
    }

    let type_imports = collect_iface_type_imports(iface);
    let mut sorted_type_imports: Vec<_> = type_imports.iter().collect();
    sorted_type_imports
        .sort_by(|a, b| (&a.namespace, &a.name, &a.kind).cmp(&(&b.namespace, &b.name, &b.kind)));
    let mut imported_names: HashSet<String> = HashSet::new();
    for r in &sorted_type_imports {
        if known_types.contains(&r.name) && !delegate_names.contains(&r.name) {
            imports.push(format_type_import_projected(&r.name, r.kind));
            imported_names.insert(r.name.clone());
        }
    }

    // Import types referenced in delegate callback signatures
    for dname in &sorted_delegates {
        if let Some(ref_types) = delegate_sig_refs.get(*dname) {
            for rt in ref_types {
                if !imported_names.contains(rt) && !delegate_names.contains(rt) && rt != &iface.name
                {
                    imports.push(format_type_import_projected(rt, TypeKind::Class));
                    imported_names.insert(rt.clone());
                }
            }
        }
    }

    // IID const
    let mut iid_consts = Vec::new();
    if let Some(rhs) = interface_iid_rhs(iface) {
        let ty = infer_const_type(&format!("IID_{}", iface.name), &rhs);
        iid_consts.push(ProjectedIidConst {
            name: format!("IID_{}", iface.name),
            rhs_expr: rhs,
            ts_type: ty,
            exported: true,
        });
    }
    let mut argument_iids = Vec::new();
    for method in &iface.methods {
        for parameter in &method.params {
            if parameter.direction == ParamDirection::In {
                collect_runtime_class_iid_consts(&parameter.typ, &mut argument_iids);
            }
        }
    }
    argument_iids.sort();
    argument_iids.dedup();
    let mut declared_iids = iid_consts
        .iter()
        .map(|constant| constant.name.clone())
        .collect::<HashSet<_>>();
    for (name, iid_expr) in argument_iids {
        if declared_iids.insert(name.clone()) {
            iid_consts.push(ProjectedIidConst {
                name,
                rhs_expr: iid_expr,
                ts_type: "WinGuid".into(),
                exported: false,
            });
        }
    }

    // Registration
    let registrations = vec![generate_interface_registration(
        iface,
        &format!("_{}", iface.name),
    )];

    // Struct helpers
    let structs = project_struct_helpers(&used_structs);

    // Members
    let iface_var = format!("_{}", iface.name);
    let mut members = Vec::new();
    for method in &iface.methods {
        if should_skip_raw_collection_method(iface, &method.name) {
            continue;
        }
        if let Some(m) = project_instance_method(
            &iface_var,
            "this._obj",
            method,
            known_types,
            &delegate_names,
            Some(&iface.methods),
            delegate_sigs,
            delegate_param_wraps,
        ) {
            members.push(m);
        }
    }

    // Collection helpers
    project_collection_helpers(iface, known_types, &mut members, &mut imports, "this._obj");

    // Static create() for IVector / IMap
    project_collection_create(iface, known_types, &mut members, &mut imports);

    if iface.name == "IElementFactory" {
        imports[0].symbols.push("DynWinRtElementFactory".into());
        members.push(ProjectedMember::Method(ProjectedMethod {
            name: "create".into(),
            doc: Some(DocInfo {
                summary: Some(
                    "Create an IElementFactory backed by synchronous JavaScript callbacks.".into(),
                ),
                deprecated: None,
                returns: None,
                params: vec![],
            }),
            params: vec![
                ProjectedParam {
                    name: "getElement".into(),
                    ts_type:
                        "(args: ElementFactoryGetArgs) => UIElement"
                            .into(),
                    optional: false,
                    delegate_wrap: None,
                },
                ProjectedParam {
                    name: "recycleElement".into(),
                    ts_type:
                        "(args: ElementFactoryRecycleArgs) => void"
                            .into(),
                    optional: false,
                    delegate_wrap: None,
                },
            ],
            argument_kinds: vec![],
            return_type: format!(
                "{} & {{ releaseCallbacks(): void }}",
                iface.name,
            ),
            async_kind: AsyncKind::None,
            is_static: true,
            invoke_expr: String::new(),
            sync_return_expr: Some(format!(
                "(() => {{ const elements = new Map(); const implementation = DynWinRtElementFactory.create((__load_UIElement()).IID_UIElement, (args) => {{ const element = getElement((__get_ElementFactoryGetArgs())._fromNative(args)); const nativeElement = _unwrap(element); elements.set(nativeElement.identityRaw(), element); return nativeElement; }}, (args) => {{ const projectedArgs = (__get_ElementFactoryRecycleArgs())._fromNative(args); const projectedElement = projectedArgs.element; const identity = _unwrap(projectedElement).identityRaw(); const element = elements.get(identity) ?? projectedElement; elements.delete(identity); const recycleArgs = Object.create(projectedArgs); Object.defineProperty(recycleArgs, 'element', {{ value: element }}); recycleElement(recycleArgs); }}); const factory = new {0}(implementation.toValue()); Object.defineProperty(factory, 'releaseCallbacks', {{ value: () => implementation.releaseCallbacks() }}); return factory; }})()",
                iface.name,
            )),
            async_convert_v: None,
            is_void: false,
            array_return_expr: None,
            delegate_wraps: vec![],
            progress_convert: None,
            js_only: false,
            overload_of: None,
        }));
    }

    let has_parameterized_cast = iface.generic_piid.is_some();
    let needs_unwrap = check_needs_unwrap_simple(&members);

    let doc = build_doc_info(iface.doc.as_deref(), iface.deprecated.as_deref(), None, &[]);

    ProjectedFile {
        name: iface.name.clone(),
        imports,
        iid_consts,
        registrations,
        structs,
        classes: vec![],
        enums: vec![],
        ifaces: vec![ProjectedIface {
            name: iface.name.clone(),
            doc,
            iid_const: None, // already in file-level iid_consts
            has_static_from: !iface.iid.is_empty(),
            has_parameterized_cast,
            members,
            is_delegate: false,
        }],
        delegates: vec![],
        needs_unwrap_helper: needs_unwrap,
        needs_activation_factory: false,
    }
}

/// Project an enum into a ProjectedFile.
pub fn project_enum(en: &TypeMeta) -> Option<ProjectedFile> {
    let (name, members_meta, enum_doc, enum_dep) = match en {
        TypeMeta::Enum {
            name,
            members,
            doc,
            deprecated,
            ..
        } => (name, members, doc.as_deref(), deprecated.as_deref()),
        _ => return None,
    };

    let members: Vec<ProjectedEnumMember> = members_meta
        .iter()
        .map(|m| ProjectedEnumMember {
            name: m.name.clone(),
            value: m.value as i64,
            doc: m.doc.clone(),
        })
        .collect();

    let doc = build_doc_info(enum_doc, enum_dep, None, &[]);

    Some(ProjectedFile {
        name: name.clone(),
        imports: vec![],
        iid_consts: vec![],
        registrations: vec![],
        structs: vec![],
        classes: vec![],
        enums: vec![ProjectedEnum {
            name: name.clone(),
            doc,
            members,
        }],
        ifaces: vec![],
        delegates: vec![],
        needs_unwrap_helper: false,
        needs_activation_factory: false,
    })
}

/// Project a delegate interface into a ProjectedFile.
pub fn project_delegate(
    iface: &InterfaceMeta,
    delegate_sigs: &HashMap<String, String>,
    delegate_sig_refs: &HashMap<String, Vec<String>>,
) -> ProjectedFile {
    let invoke = iface.methods.iter().find(|m| m.name == "Invoke");
    let param_exprs: Vec<String> = invoke
        .map(|inv| {
            inv.params
                .iter()
                .filter(|p| p.direction == ParamDirection::In)
                .map(|p| ts_dynwinrt_type(&p.typ))
                .collect()
        })
        .unwrap_or_default();
    let iid_arg_exprs: Vec<String> = if iface.generic_args.is_empty() {
        param_exprs.clone()
    } else {
        iface.generic_args.iter().map(ts_dynwinrt_type).collect()
    };

    let iid_rhs =
        if !iface.iid.is_empty() && iface.generic_args.is_empty() && iface.generic_piid.is_none() {
            format!("WinGuid.parse('{}')", iface.iid)
        } else if !iface.iid.is_empty() {
            format!(
                "DynWinRtType.parameterized(WinGuid.parse('{}'), [{}]).iid()",
                iface.iid,
                iid_arg_exprs.join(", ")
            )
        } else {
            "undefined".into()
        };

    let iid_ts_type = if !iface.iid.is_empty() {
        "WinGuid"
    } else {
        "any"
    };

    let param_types_expr = format!("[{}]", param_exprs.join(", "));

    let callback_type = delegate_sigs.get(&iface.name).cloned();

    let mut imports = vec![ProjectedImport {
        symbols: vec!["DynWinRtType".into(), "WinGuid".into()],
        from: get_import_name(),
        runtime_only: false,
        dts_only: false,
        is_runtime_package: false,
    }];
    if let Some(ref_types) = delegate_sig_refs.get(&iface.name) {
        for rt in ref_types {
            imports.push(ProjectedImport {
                symbols: vec![rt.clone()],
                from: format!("./{}.js", rt),
                runtime_only: false,
                dts_only: true,
                is_runtime_package: false,
            });
        }
    }

    ProjectedFile {
        name: iface.name.clone(),
        imports,
        iid_consts: vec![],
        registrations: vec![],
        structs: vec![],
        classes: vec![],
        enums: vec![],
        ifaces: vec![],
        delegates: vec![ProjectedDelegate {
            name: iface.name.clone(),
            iid_rhs,
            iid_ts_type: iid_ts_type.into(),
            has_param_types: invoke.is_some(),
            param_types_expr,
            callback_type,
        }],
        needs_unwrap_helper: false,
        needs_activation_factory: false,
    }
}

// ======================================================================
// Utility helpers
// ======================================================================

/// Returns a dedup key for a SymbolKind so flatten can detect duplicate symbols.
pub fn symbol_dedup_key(kind: &SymbolKind) -> String {
    match kind {
        // toString renders as a plain `toString()` method — use the actual name for dedup
        SymbolKind::ToString { .. } => "toString".into(),
        SymbolKind::ToPrimitive => "Symbol::toPrimitive".into(),
        SymbolKind::ToStringTag { .. } => "Symbol::toStringTag".into(),
        SymbolKind::Iterator { .. } => "Symbol::iterator".into(),
        SymbolKind::CollectionLength => "length".into(),
        SymbolKind::CollectionAt { .. } => "at".into(),
        SymbolKind::CollectionToArray { .. } => "toArray".into(),
        SymbolKind::IteratorNext { .. } => "next".into(),
    }
}

fn build_runtime_import(has_structs: bool) -> ProjectedImport {
    let mut symbols = vec![
        "DynWinRtType".into(),
        "DynWinRtMethodSig".into(),
        "DynWinRtValue".into(),
        "DynWinRtArray".into(),
    ];
    if has_structs {
        symbols.push("DynWinRtStruct".into());
    }
    symbols.push("DynWinRtDelegate".into());
    symbols.push("WinGuid".into());
    ProjectedImport {
        symbols,
        from: get_import_name(),
        runtime_only: false,
        dts_only: false,
        is_runtime_package: true,
    }
}

fn format_type_import_projected(name: &str, kind: TypeKind) -> ProjectedImport {
    if kind == TypeKind::Interface {
        ProjectedImport {
            symbols: vec![format!("IID_{}", name), name.into()],
            from: format!("./{}.js", name),
            runtime_only: false,
            dts_only: false,
            is_runtime_package: false,
        }
    } else {
        ProjectedImport {
            symbols: vec![name.into()],
            from: format!("./{}.js", name),
            runtime_only: false,
            dts_only: false,
            is_runtime_package: false,
        }
    }
}

fn collect_delegate_names_from_methods(
    methods: &[MethodMeta],
    delegate_names: &mut HashSet<String>,
) {
    for method in methods {
        for p in &method.params {
            match &p.typ {
                TypeMeta::Delegate { name, .. } => {
                    delegate_names.insert(name.clone());
                }
                TypeMeta::Interface { name, .. }
                    if method.is_event_add || method.is_event_remove =>
                {
                    delegate_names.insert(name.clone());
                }
                _ => {}
            }
            if method.is_event_add || method.is_event_remove {
                if let TypeMeta::Parameterized { name, args, .. } = &p.typ {
                    delegate_names.insert(crate::meta::make_parameterized_name(name, args));
                }
            }
        }
    }
}

fn collect_known_delegate_names_from_methods(
    methods: &[MethodMeta],
    known_delegate_names: &HashSet<String>,
    delegate_names: &mut HashSet<String>,
) {
    for method in methods {
        for parameter in &method.params {
            collect_known_delegate_names_from_type(
                &parameter.typ,
                known_delegate_names,
                delegate_names,
            );
        }
        if let Some(return_type) = &method.return_type {
            collect_known_delegate_names_from_type(
                return_type,
                known_delegate_names,
                delegate_names,
            );
        }
    }
}

fn collect_known_delegate_names_from_type(
    typ: &TypeMeta,
    known_delegate_names: &HashSet<String>,
    delegate_names: &mut HashSet<String>,
) {
    match typ {
        TypeMeta::Delegate { name, .. } => {
            delegate_names.insert(name.clone());
        }
        TypeMeta::Interface { name, .. } => {
            if known_delegate_names.contains(name) {
                delegate_names.insert(name.clone());
            }
        }
        TypeMeta::AsyncActionWithProgress(inner)
        | TypeMeta::AsyncOperation(inner)
        | TypeMeta::Array(inner) => {
            collect_known_delegate_names_from_type(
                inner,
                known_delegate_names,
                delegate_names,
            );
        }
        TypeMeta::AsyncOperationWithProgress(
            result,
            progress,
        ) => {
            collect_known_delegate_names_from_type(
                result,
                known_delegate_names,
                delegate_names,
            );
            collect_known_delegate_names_from_type(
                progress,
                known_delegate_names,
                delegate_names,
            );
        }
        TypeMeta::Parameterized { name, args, .. } => {
            let concrete =
                crate::meta::make_parameterized_name(
                    name,
                    args,
                );
            if known_delegate_names.contains(&concrete) {
                delegate_names.insert(concrete);
            }
            for argument in args {
                collect_known_delegate_names_from_type(
                    argument,
                    known_delegate_names,
                    delegate_names,
                );
            }
        }
        TypeMeta::RuntimeClass {
            default_interface: Some(interface),
            ..
        } => {
            collect_known_delegate_names_from_type(
                interface,
                known_delegate_names,
                delegate_names,
            );
        }
        TypeMeta::Struct { fields, .. } => {
            for field in fields {
                collect_known_delegate_names_from_type(
                    &field.typ,
                    known_delegate_names,
                    delegate_names,
                );
            }
        }
        _ => {}
    }
}

fn project_params(
    in_params: &[&crate::meta::ParamMeta],
    known_types: &HashSet<String>,
    delegate_names: &HashSet<String>,
    delegate_sigs: &HashMap<String, String>,
    delegate_param_wraps: &HashMap<String, Vec<String>>,
) -> Vec<ProjectedParam> {
    in_params
        .iter()
        .map(|p| {
            let mut ts = ts_param_type_dts(&p.typ, known_types);
            let mut delegate_wrap = None;
            if delegate_names.contains(&ts) {
                let orig_name = ts.clone();
                if let Some(sig) = delegate_sigs.get(&ts) {
                    ts = sig.clone();
                    let wraps = delegate_param_wraps
                        .get(&orig_name)
                        .cloned()
                        .unwrap_or_default();
                    delegate_wrap = Some(DelegateWrapInfo {
                        delegate_name: orig_name,
                        callback_type: sig.clone(),
                        param_wraps: wraps,
                    });
                } else {
                    ts = "DynWinRtValue".to_string();
                }
            }
            ProjectedParam {
                name: to_camel_case(&p.name),
                ts_type: ts,
                optional: false,
                delegate_wrap,
            }
        })
        .collect()
}

fn build_doc_info(
    summary: Option<&str>,
    deprecated: Option<&str>,
    returns: Option<&str>,
    params: &[(&str, &str)],
) -> Option<DocInfo> {
    if summary.is_none() && deprecated.is_none() && returns.is_none() && params.is_empty() {
        return None;
    }
    Some(DocInfo {
        summary: summary.map(|s| s.to_string()),
        deprecated: deprecated.map(|s| s.to_string()),
        returns: returns.map(|s| s.to_string()),
        params: params
            .iter()
            .map(|(n, d)| (n.to_string(), d.to_string()))
            .collect(),
    })
}

fn build_method_doc(method: &MethodMeta, in_params: &[&crate::meta::ParamMeta]) -> Option<DocInfo> {
    let params_display: Vec<(String, String)> = in_params
        .iter()
        .filter_map(|p| {
            crate::codegen::winrt::shared::docs::find_param_doc(&method.param_docs, &p.name)
                .map(|d| (to_camel_case(&p.name), d.to_string()))
        })
        .collect();
    let params_refs: Vec<(&str, &str)> = params_display
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_str()))
        .collect();

    // If XML doc exists, use it
    if method.doc.is_some()
        || method.deprecated.is_some()
        || method.returns_doc.is_some()
        || !params_refs.is_empty()
    {
        return build_doc_info(
            method.doc.as_deref(),
            method.deprecated.as_deref(),
            method.returns_doc.as_deref(),
            &params_refs,
        );
    }

    // Synthesize doc for overloaded methods (names containing "Overload")
    let camel = to_camel_case(&method.name);
    if let Some(summary) = synthesize_overload_doc(&camel, &method.name) {
        return Some(DocInfo {
            summary: Some(summary),
            deprecated: None,
            returns: None,
            params: vec![],
        });
    }

    None
}

/// Generate a helpful doc string for WinRT overloaded method names.
fn synthesize_overload_doc(camel_name: &str, raw_name: &str) -> Option<String> {
    // Pattern: fooOverloadDefault... or fooOverloadDefault
    if !raw_name.contains("Overload") {
        return None;
    }
    // Extract the base method name (before "Overload")
    let base = if let Some(pos) = camel_name.find("Overload") {
        &camel_name[..pos]
    } else {
        return None;
    };
    // Extract the suffix after "Overload" to describe what's defaulted
    let suffix = &camel_name[camel_name.find("Overload").unwrap() + "Overload".len()..];
    if suffix.is_empty() || suffix == "Default" {
        Some(format!("Parameterless overload of `{}`.", base))
    } else if suffix.starts_with("Default") {
        let defaulted = suffix.strip_prefix("Default").unwrap_or(suffix);
        let words = split_pascal_case(defaulted);
        if words.is_empty() {
            Some(format!("Default overload of `{}`.", base))
        } else {
            Some(format!(
                "Overload of `{}` with default {}.",
                base,
                words.join(", ")
            ))
        }
    } else {
        let words = split_pascal_case(suffix);
        Some(format!("Overload of `{}` ({}).", base, words.join(", ")))
    }
}

/// Split PascalCase into lowercase words: "OptionsStartAndCount" → ["options", "start", "count"]
fn split_pascal_case(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for ch in s.chars() {
        if ch.is_uppercase() && !current.is_empty() {
            let word = current.to_lowercase();
            if word != "and" {
                words.push(word);
            }
            current.clear();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        let word = current.to_lowercase();
        if word != "and" {
            words.push(word);
        }
    }
    words
}

fn async_inner_type(rt: Option<&TypeMeta>) -> Option<&TypeMeta> {
    match rt {
        Some(TypeMeta::AsyncOperation(i)) => Some(i),
        Some(TypeMeta::AsyncOperationWithProgress(i, _)) => Some(i),
        _ => None,
    }
}

fn check_needs_unwrap(members: &[ProjectedMember], req_ifaces: &[ProjectedRequiredIface]) -> bool {
    if check_needs_unwrap_simple(members) {
        return true;
    }
    for ri in req_ifaces {
        if check_needs_unwrap_simple(&ri.members) {
            return true;
        }
    }
    false
}

fn check_needs_unwrap_simple(members: &[ProjectedMember]) -> bool {
    for m in members {
        match m {
            ProjectedMember::Method(pm) => {
                if contains_unwrap(&pm.invoke_expr) {
                    return true;
                }
                if pm
                    .sync_return_expr
                    .as_deref()
                    .map_or(false, contains_unwrap)
                {
                    return true;
                }
            }
            ProjectedMember::Property(pp) => {
                if contains_unwrap(&pp.getter_expr) {
                    return true;
                }
                if pp.setter_line.as_deref().map_or(false, contains_unwrap) {
                    return true;
                }
            }
            ProjectedMember::Event(pe) => {
                if pe.sender_wrap.as_deref().map_or(false, contains_unwrap) {
                    return true;
                }
                if pe.args_wrap.as_deref().map_or(false, contains_unwrap) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn contains_unwrap(s: &str) -> bool {
    s.contains("_unwrap(")
}

/// Collect delegate wraps from projected params into (param_name, delegate_name) pairs.
fn collect_delegate_wraps(params: &[ProjectedParam]) -> Vec<(String, String)> {
    params
        .iter()
        .filter_map(|p| {
            p.delegate_wrap
                .as_ref()
                .map(|dw| (p.name.clone(), dw.delegate_name.clone()))
        })
        .collect()
}

/// Replace `_unwrap(paramName)` with `_{paramName}_d` in invoke expressions for delegate params.
fn rewrite_delegate_args_in_expr(expr: &str, params: &[ProjectedParam]) -> String {
    let mut result = expr.to_string();
    for p in params {
        if p.delegate_wrap.is_some() {
            let from = format!("_unwrap({})", p.name);
            let to = format!("_{}_d", p.name);
            result = result.replace(&from, &to);
        }
    }
    result
}

/// Rename methods that have `overload_of` set, or whose name ends with digits
/// and a base-name sibling exists, to use the base name.
/// E.g. `generateResponseAsync2` becomes `generateResponseAsync` when
/// `generateResponseAsync` already exists in the same member list.
fn merge_overload_names(members: &mut [ProjectedMember]) {
    // First pass: apply explicit overload_of
    for member in members.iter_mut() {
        if let ProjectedMember::Method(method) = member {
            if let Some(ref base_name) = method.overload_of {
                method.name = base_name.clone();
            }
        }
    }

    // Second pass: detect numeric suffix patterns (e.g. foo2, foo3 when foo exists)
    let all_names: std::collections::HashSet<String> = members
        .iter()
        .filter_map(|m| {
            if let ProjectedMember::Method(method) = m {
                Some(method.name.clone())
            } else {
                None
            }
        })
        .collect();

    for member in members.iter_mut() {
        if let ProjectedMember::Method(method) = member {
            let name = &method.name;
            // Strip trailing digits to find base name
            let base = name.trim_end_matches(|c: char| c.is_ascii_digit());
            if base.len() < name.len()
                && !base.is_empty()
                && all_names.contains(base)
                && base != name
            {
                method.name = base.to_string();
            }
        }
    }
}
