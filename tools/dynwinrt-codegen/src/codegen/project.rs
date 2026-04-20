// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Build projected IR from parsed WinRT metadata.
//!
//! All projection decisions — naming, type mapping, async detection, event
//! pairing, import classification, IStringable/IClosable detection — happen
//! here. The renderers consume the IR and only format.

use std::collections::{HashMap, HashSet};

use crate::meta::{ClassMeta, InterfaceMeta, MethodMeta, ParamDirection};
use crate::types::{TypeKind, TypeMeta};

use super::common::{
    collect_used_structs_from_class, collect_used_structs_from_iface,
    ts_struct_field_type, struct_field_getter, struct_field_setter,
    ts_dynwinrt_type, generate_interface_registration,
    collect_used_generics_from_methods, collect_used_generics_from_class,
    collect_iface_type_imports, collect_type_imports,
    to_camel_case, capitalize, infer_const_type,
    NO_DEFERRED, build_args_expr, convert_array_return, convert_return,
    get_in_params, wrap_arg,
};
use super::method::{ts_param_type_safe, ts_param_type_dts, ts_return_type_safe, ts_array_element_type};
use super::projected::*;

// ======================================================================
// PIIDs of well-known collection interfaces
// ======================================================================
const PIID_IVECTOR: &str = "913337e9-11a1-4345-a3a2-4e7f956e222d";
const PIID_IVECTOR_VIEW: &str = "bbe1fa4c-b0e3-4583-baef-1f1b2e483e56";
const PIID_IITERATOR: &str = "6a79e863-4300-459a-9966-cbb660963ee1";
const PIID_IITERABLE: &str = "faa585ea-6214-4217-afda-7f46de5869b3";
const PIID_IMAP: &str = "3c2925fe-8519-45c1-aa79-197b6718c1c1";
const PIID_IMAP_VIEW: &str = "e480ce40-a338-4ada-adcf-272272e48cb9";
const ICLOSABLE_IID: &str = "30d5a829-7fa4-4026-83bb-d75bae4ea99e";
const ISTRINGABLE_IID: &str = "96369f54-8eb6-48f0-abce-c1b211e627c3";

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
) -> (HashMap<String, String>, HashMap<String, Vec<String>>, HashMap<String, Vec<String>>) {
    let mut sigs = HashMap::new();
    let mut refs = HashMap::new();
    let mut wraps = HashMap::new();
    for i in all_interfaces.iter()
        .filter(|i| i.methods.iter().any(|m| m.name == ".ctor") && i.methods.iter().any(|m| m.name == "Invoke"))
    {
        let Some(invoke) = i.methods.iter().find(|m| m.name == "Invoke") else { continue };
        let mut ref_types = Vec::new();
        let mut param_wraps = Vec::new();
        let in_params: Vec<_> = invoke.params.iter()
            .filter(|p| p.direction == ParamDirection::In)
            .collect();
        let params: Vec<String> = in_params.iter().enumerate()
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
                    let wrap = convert_return(&arg_var, Some(&p.typ), false, known_types, &NO_DEFERRED);
                    param_wraps.push(wrap);
                    ts
                };
                format!("{}: {}", to_camel_case(&p.name), ts_clean)
            })
            .collect();
        let ret = if invoke.return_type.is_some() { "DynWinRtValue" } else { "void" };
        sigs.insert(i.name.clone(), format!("({}) => {}", params.join(", "), ret));
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

    // Collect delegate names only from interfaces of THIS class (not the entire batch)
    // for delegate imports; but also include global delegate_type_names for type filtering
    let mut delegate_names: HashSet<String> = HashSet::new();
    let all_ifaces: Vec<&InterfaceMeta> = class.default_interface.iter()
        .chain(class.factory_interfaces.iter())
        .chain(class.static_interfaces.iter())
        .chain(class.required_interfaces.iter())
        .collect();
    for iface in &all_ifaces {
        collect_delegate_names_from_methods(&iface.methods, &mut delegate_names);
    }
    // All known delegate type names (used to filter type imports — delegates typed as Interface
    // in parameter metadata must not be imported as regular type imports)
    let all_delegate_names: HashSet<String> = delegate_names.union(delegate_type_names).cloned().collect();

    // Build imports
    let mut imports = Vec::new();

    // Runtime import
    let has_structs = !used_structs.is_empty();
    imports.push(build_runtime_import(has_structs));

    // Collection generics imports
    let collection_names = collect_used_generics_from_class(class);
    for cname in &collection_names {
        if !all_delegate_names.contains(cname) {
            imports.push(ProjectedImport {
                symbols: vec![cname.clone()],
                from: format!("./{}.js", cname),
                runtime_only: false, dts_only: false,
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
            runtime_only: true, dts_only: false,
        });
        // DTS-only: import the delegate type alias (for typed param signatures)
        if delegate_sigs.contains_key(*dname) {
            imports.push(ProjectedImport {
                symbols: vec![dname.to_string()],
                from: format!("./{}.js", dname),
                runtime_only: false, dts_only: true,
            });
        }
    }

    // Type imports
    let mut imported_names: HashSet<String> = HashSet::new();
    let type_imports = collect_type_imports(class);
    let mut sorted_imports: Vec<_> = type_imports.iter().collect();
    sorted_imports.sort_by(|a, b| (&a.namespace, &a.name, &a.kind).cmp(&(&b.namespace, &b.name, &b.kind)));
    for r in &sorted_imports {
        if known_types.contains(&r.name) && !all_delegate_names.contains(&r.name) {
            imports.push(format_type_import_projected(&r.name, r.kind));
            imported_names.insert(r.name.clone());
            if r.kind == TypeKind::Interface {
                imported_names.insert(format!("IID_{}", r.name));
            }
        } else if all_delegate_names.contains(&r.name) && delegate_sigs.contains_key(&r.name) && !delegate_names.contains(&r.name) {
            // Delegate typed as Interface in params — add DTS-only type import
            imports.push(ProjectedImport {
                symbols: vec![r.name.clone()],
                from: format!("./{}.js", r.name),
                runtime_only: false, dts_only: true,
            });
            imported_names.insert(r.name.clone());
        }
    }

    // Import shared required interfaces
    for req_iface in &class.required_interfaces {
        if !req_iface.iid.is_empty() && shared_iids.contains(&req_iface.iid) && !imported_names.contains(&req_iface.name) {
            imports.push(format_type_import_projected(&req_iface.name, TypeKind::Interface));
            imported_names.insert(req_iface.name.clone());
            imported_names.insert(format!("IID_{}", req_iface.name));
        }
    }

    // Import types referenced in delegate callback signatures (e.g. StreamedFileDataRequest)
    for dname in &sorted_delegates {
        if let Some(ref_types) = delegate_sig_refs.get(*dname) {
            for rt in ref_types {
                if !imported_names.contains(rt) && !delegate_names.contains(rt) && rt != &class.name {
                    imports.push(format_type_import_projected(rt, TypeKind::Class));
                    imported_names.insert(rt.clone());
                }
            }
        }
    }

    // IID consts(private, for class-internal use)
    let mut iid_consts = Vec::new();
    let all_class_ifaces: Vec<&InterfaceMeta> = class.default_interface.iter()
        .chain(class.factory_interfaces.iter())
        .chain(class.static_interfaces.iter())
        .chain(class.required_interfaces.iter())
        .collect();
    for iface in &all_class_ifaces {
        let iid_name = format!("IID_{}", iface.name);
        if !iface.iid.is_empty() && !imported_names.contains(&iid_name) {
            iid_consts.push(ProjectedIidConst {
                name: iid_name,
                rhs_expr: format!("WinGuid.parse('{}')", iface.iid),
                ts_type: "WinGuid".into(),
                exported: false,
            });
        }
    }

    // Interface registrations
    let mut registrations = Vec::new();
    if let Some(ref iface) = class.default_interface {
        registrations.push(generate_interface_registration(iface, &format!("_{}", iface.name)));
    }
    for iface in &class.factory_interfaces {
        registrations.push(generate_interface_registration(iface, &format!("_{}", iface.name)));
    }
    for iface in &class.static_interfaces {
        registrations.push(generate_interface_registration(iface, &format!("_{}", iface.name)));
    }
    for iface in &class.required_interfaces {
        if !iface.iid.is_empty() && shared_iids.contains(&iface.iid) && imported_names.contains(&iface.name) {
            continue;
        }
        registrations.push(generate_interface_registration(iface, &format!("_{}", iface.name)));
    }

    // Struct helpers
    let structs = project_struct_helpers(&used_structs);

    // Build class members
    let mut members = Vec::new();

    // Constructor
    let mut ctor_body = Vec::new();
    if let Some(ref iface) = class.default_interface {
        if !iface.iid.is_empty() {
            ctor_body.push(format!("this._obj = obj.cast(IID_{});", iface.name));
        } else {
            ctor_body.push("this._obj = obj;".into());
        }
    } else {
        ctor_body.push("this._obj = obj;".into());
    }
    members.push(ProjectedMember::Constructor(ProjectedConstructor {
        params: vec![ProjectedParam { name: "obj".into(), ts_type: "DynWinRtValue".into(), optional: false, delegate_wrap: None }],
        body_lines: ctor_body,
    }));

    // Default constructor (static create/createDefault)
    if class.has_default_constructor {
        let has_create_factory = class.factory_interfaces.iter()
            .any(|iface| iface.methods.iter().any(|m| {
                let camel = to_camel_case(&m.name);
                camel == "create" || camel.starts_with("create")
            }));
        let ctor_name = if has_create_factory { "createDefault" } else { "create" };
        members.push(ProjectedMember::Method(ProjectedMethod {
            name: ctor_name.into(),
            doc: Some(DocInfo {
                summary: Some(format!("Create a new `{}` instance.", class.name)),
                deprecated: None,
                returns: None,
                params: vec![],
            }),
            params: vec![],
            return_type: class.name.clone(),
            async_kind: AsyncKind::None,
            is_static: true,
            invoke_expr: format!(
                "_IActivationFactory.method(6).invoke(DynWinRtValue.activationFactory('{}'), [])",
                class.full_name
            ),
            sync_return_expr: Some(format!(
                "new {}(_IActivationFactory.method(6).invoke(DynWinRtValue.activationFactory('{}'), []))",
                class.name, class.full_name
            )),
            async_convert_v: None,
            is_void: false,
            array_return_expr: None,
            delegate_wraps: vec![],
            progress_convert: None,
            js_only: false,
        }));
    }

    // Factory methods
    for iface in &class.factory_interfaces {
        for method in &iface.methods {
            members.push(project_factory_method(class, iface, method, known_types, &delegate_names, delegate_sigs, delegate_param_wraps));
        }
    }

    // Static methods
    for iface in &class.static_interfaces {
        for method in &iface.methods {
            members.push(project_static_method(class, iface, method, known_types, &delegate_names, delegate_sigs, delegate_param_wraps));
        }
    }

    // Instance methods (from default interface)
    if let Some(ref default_iface) = class.default_interface {
        let iface_var = format!("_{}", default_iface.name);
        for method in &default_iface.methods {
            if let Some(m) = project_instance_method(
                &iface_var, "this._obj", method, known_types, &delegate_names,
                Some(&default_iface.methods), delegate_sigs, delegate_param_wraps,
            ) {
                members.push(m);
            }
        }
    }

    // IClosable → close()
    if class.required_interfaces.iter().any(|ri| ri.iid == ICLOSABLE_IID) {
        members.push(ProjectedMember::Close);
    }

    // IStringable → toString, toPrimitive, toStringTag
    let has_istringable = class.required_interfaces.iter().any(|ri| ri.iid == ISTRINGABLE_IID);
    if has_istringable {
        let iface_name = class.required_interfaces.iter()
            .find(|ri| ri.iid == ISTRINGABLE_IID)
            .map(|ri| ri.name.clone())
            .unwrap_or_else(|| "IStringable".into());
        members.push(ProjectedMember::Symbol(ProjectedSymbol {
            kind: SymbolKind::ToString { iface_name: iface_name.clone() },
            doc: None,
        }));
        members.push(ProjectedMember::Symbol(ProjectedSymbol {
            kind: SymbolKind::ToPrimitive,
            doc: None,
        }));
        members.push(ProjectedMember::Symbol(ProjectedSymbol {
            kind: SymbolKind::ToStringTag { tag: class.name.clone() },
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
    let mut main_member_names: HashSet<String> = members.iter().filter_map(|m| match m {
        ProjectedMember::Method(pm) => Some(pm.name.clone()),
        ProjectedMember::Property(pp) => Some(pp.name.clone()),
        ProjectedMember::Event(pe) => Some(pe.subscribe_name.clone()),
        ProjectedMember::Symbol(ps) => Some(symbol_dedup_key(&ps.kind)),
        ProjectedMember::Close => Some("close".into()),
        ProjectedMember::AsCast => Some("as".into()),
        _ => None,
    }).collect();

    for req_iface in &class.required_interfaces {
        if req_iface.iid.is_empty() { continue; }
        if imported_names.contains(&req_iface.name) { continue; }

        let reg_var = format!("_{}", req_iface.name);
        let mut ri_members = Vec::new();
        for method in &req_iface.methods {
            if let Some(m) = project_instance_method(
                &reg_var, "this._obj", method, known_types, &delegate_names,
                Some(&req_iface.methods), delegate_sigs, delegate_param_wraps,
            ) {
                ri_members.push(m);
            }
        }

        // Flatten: copy non-conflicting members onto the main class
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
            if main_member_names.insert(name) {
                members.push(member.clone());
            }
        }

        required_ifaces.push(ProjectedRequiredIface {
            name: req_iface.name.clone(),
            iid: req_iface.iid.clone(),
            disposition: RequiredIfaceDisposition::InlineWrapper,
            members: ri_members,
            registration: None,
            has_static_from: true,
            has_parameterized_cast: false,
        });
    }

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

    let mut delegate_names: HashSet<String> = delegate_type_names.clone();
    collect_delegate_names_from_methods(&iface.methods, &mut delegate_names);

    // Build imports
    let mut imports = Vec::new();
    imports.push(build_runtime_import(has_structs));

    let collection_names = collect_used_generics_from_methods(&iface.methods);
    for cname in &collection_names {
        if cname != &iface.name && !delegate_names.contains(cname) {
            imports.push(ProjectedImport {
                symbols: vec![cname.clone()],
                from: format!("./{}.js", cname),
                runtime_only: false, dts_only: false,
            });
        }
    }

    let mut sorted_delegates: Vec<_> = delegate_names.iter().collect();
    sorted_delegates.sort();
    for dname in &sorted_delegates {
        imports.push(ProjectedImport {
            symbols: vec![format!("IID_{}", dname), format!("{}_PARAM_TYPES", dname)],
            from: format!("./{}.js", dname),
            runtime_only: true, dts_only: false,
        });
    }

    let type_imports = collect_iface_type_imports(iface);
    let mut sorted_type_imports: Vec<_> = type_imports.iter().collect();
    sorted_type_imports.sort_by(|a, b| (&a.namespace, &a.name, &a.kind).cmp(&(&b.namespace, &b.name, &b.kind)));
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
                if !imported_names.contains(rt) && !delegate_names.contains(rt) && rt != &iface.name {
                    imports.push(format_type_import_projected(rt, TypeKind::Class));
                    imported_names.insert(rt.clone());
                }
            }
        }
    }

    // IID const
    let mut iid_consts = Vec::new();
    if let Some(ref piid) = iface.generic_piid {
        let args_ts: Vec<String> = iface.generic_args.iter().map(|a| ts_dynwinrt_type(a)).collect();
        let rhs = format!("DynWinRtType.parameterized(WinGuid.parse('{}'), [{}]).iid()", piid, args_ts.join(", "));
        let ty = infer_const_type(&format!("IID_{}", iface.name), &rhs);
        iid_consts.push(ProjectedIidConst {
            name: format!("IID_{}", iface.name),
            rhs_expr: rhs,
            ts_type: ty,
            exported: true,
        });
    } else if !iface.iid.is_empty() {
        iid_consts.push(ProjectedIidConst {
            name: format!("IID_{}", iface.name),
            rhs_expr: format!("WinGuid.parse('{}')", iface.iid),
            ts_type: "WinGuid".into(),
            exported: true,
        });
    }

    // Registration
    let registrations = vec![
        generate_interface_registration(iface, &format!("_{}", iface.name)),
    ];

    // Struct helpers
    let structs = project_struct_helpers(&used_structs);

    // Members
    let iface_var = format!("_{}", iface.name);
    let is_collection = iface.generic_piid.as_deref()
        .is_some_and(|p| [PIID_IVECTOR, PIID_IVECTOR_VIEW, PIID_IITERABLE].contains(&p));
    let is_map_collection = iface.generic_piid.as_deref()
        .is_some_and(|p| [PIID_IMAP, PIID_IMAP_VIEW].contains(&p));
    let mut members = Vec::new();
    for method in &iface.methods {
        // Skip raw GetMany/ReplaceAll/IndexOf on vector collections — JS-friendly helpers are added below
        if is_collection && (method.name == "GetMany" || method.name == "ReplaceAll" || method.name == "IndexOf") {
            continue;
        }
        // Skip Split on IMapView — multi-out-param method with no JS-friendly equivalent
        if is_map_collection && method.name == "Split" {
            continue;
        }
        if let Some(m) = project_instance_method(
            &iface_var, "this._obj", method, known_types, &delegate_names,
            Some(&iface.methods), delegate_sigs, delegate_param_wraps,
        ) {
            members.push(m);
        }
    }

    // Collection helpers
    project_collection_helpers(iface, known_types, &mut members, &mut imports);

    // Static create() for IVector / IMap
    project_collection_create(iface, known_types, &mut members);

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
        TypeMeta::Enum { name, members, doc, deprecated, .. } =>
            (name, members, doc.as_deref(), deprecated.as_deref()),
        _ => return None,
    };

    let members: Vec<ProjectedEnumMember> = members_meta.iter().map(|m| {
        ProjectedEnumMember {
            name: m.name.clone(),
            value: m.value as i64,
            doc: m.doc.clone(),
        }
    }).collect();

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
pub fn project_delegate(iface: &InterfaceMeta, delegate_sigs: &HashMap<String, String>, delegate_sig_refs: &HashMap<String, Vec<String>>) -> ProjectedFile {
    let invoke = iface.methods.iter().find(|m| m.name == "Invoke");
    let param_exprs: Vec<String> = invoke.map(|inv| {
        inv.params.iter()
            .filter(|p| p.direction == ParamDirection::In)
            .map(|p| ts_dynwinrt_type(&p.typ))
            .collect()
    }).unwrap_or_default();

    let iid_rhs = if !iface.iid.is_empty() {
        format!(
            "DynWinRtType.parameterized(WinGuid.parse('{}'), [{}]).iid()",
            iface.iid, param_exprs.join(", ")
        )
    } else {
        "undefined".into()
    };

    let iid_ts_type = if !iface.iid.is_empty() { "WinGuid" } else { "any" };

    let param_types_expr = format!("[{}]", param_exprs.join(", "));

    let callback_type = delegate_sigs.get(&iface.name).cloned();

    let mut imports = vec![ProjectedImport {
        symbols: vec!["DynWinRtType".into(), "WinGuid".into()],
        from: "@microsoft/dynwinrt".into(),
        runtime_only: false, dts_only: false,
    }];
    if let Some(ref_types) = delegate_sig_refs.get(&iface.name) {
        for rt in ref_types {
            imports.push(ProjectedImport {
                symbols: vec![rt.clone()],
                from: format!("./{}.js", rt),
                runtime_only: false, dts_only: true,
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
// Method projection helpers
// ======================================================================

fn project_factory_method(
    class: &ClassMeta,
    iface: &InterfaceMeta,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_names: &HashSet<String>,
    delegate_sigs: &HashMap<String, String>,
    delegate_param_wraps: &HashMap<String, Vec<String>>,
) -> ProjectedMember {
    let in_params = get_in_params(method);
    let params = project_params(&in_params, known_types, delegate_names, delegate_sigs, delegate_param_wraps);
    let args_expr = build_args_expr(&in_params);

    let is_async = method.return_type.as_ref().is_some_and(|rt| rt.is_async());

    let mut invoke_expr = format!(
        "_{iface}.method({idx}).invoke({cls}.f_{iface}(), [{args}])",
        iface = iface.name, idx = method.vtable_index, cls = class.name, args = args_expr
    );
    invoke_expr = rewrite_delegate_args_in_expr(&invoke_expr, &params);

    let return_type;
    let async_kind;
    let sync_return_expr;
    let async_convert_v;

    if is_async {
        return_type = format!("Promise<{}>", class.name);
        async_kind = AsyncKind::Operation(class.name.clone());
        sync_return_expr = None;
        async_convert_v = Some(format!("new {}(_v)", class.name));
    } else {
        return_type = class.name.clone();
        async_kind = AsyncKind::None;
        sync_return_expr = Some(format!("new {}({})", class.name, invoke_expr));
        async_convert_v = None;
    }

    let mut ts_params = params;
    if is_async {
        ts_params.push(ProjectedParam { name: "signal".into(), ts_type: "AbortSignal".into(), optional: true, delegate_wrap: None });
    }

    let mut doc = build_method_doc(method, &in_params);
    if is_async {
        if let Some(ref mut d) = doc {
            d.params.push(("signal".into(), "Abort signal to cancel the underlying WinRT async operation.".into()));
        }
    }
    let delegate_wraps = collect_delegate_wraps(&ts_params);

    ProjectedMember::Method(ProjectedMethod {
        name: to_camel_case(&method.name),
        doc,
        params: ts_params,
        return_type,
        async_kind,
        is_static: true,
        invoke_expr,
        sync_return_expr,
        async_convert_v,
        progress_convert: None,
        is_void: false,
        array_return_expr: None,
        delegate_wraps,
        js_only: false,
    })
}

fn project_static_method(
    class: &ClassMeta,
    iface: &InterfaceMeta,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_names: &HashSet<String>,
    delegate_sigs: &HashMap<String, String>,
    delegate_param_wraps: &HashMap<String, Vec<String>>,
) -> ProjectedMember {
    let in_params = get_in_params(method);
    let return_type_meta = method.return_type.as_ref();
    let is_with_progress = return_type_meta.is_some_and(|rt| matches!(rt,
        TypeMeta::AsyncOperationWithProgress(_, _) | TypeMeta::AsyncActionWithProgress(_)));
    let is_async = return_type_meta.is_some_and(|rt| rt.is_async()) && !is_with_progress;

    let statics_call = format!("{cls}.s_{iface}()", cls = class.name, iface = iface.name);

    // Static property getter
    if method.is_property_getter && in_params.is_empty() {
        let prop_name = to_camel_case(method.name.strip_prefix("get_").unwrap_or(&method.name));
        let ts_return = ts_return_type_safe(return_type_meta, false, known_types);
        let invoke_expr = format!(
            "_{}.method({}).invoke({}, [])",
            iface.name, method.vtable_index, statics_call
        );
        let converted = convert_return(&invoke_expr, return_type_meta, false, known_types, &NO_DEFERRED);
        let doc = build_method_doc(method, &in_params);
        return ProjectedMember::Property(ProjectedProperty {
            name: prop_name,
            ts_type: ts_return,
            readonly: true,
            is_static: true,
            doc,
            getter_expr: converted,
            setter_line: None,
        });
    }

    let ts_return = ts_return_type_safe(return_type_meta, is_async, known_types);
    let params = project_params(&in_params, known_types, delegate_names, delegate_sigs, delegate_param_wraps);    let args_expr = build_args_expr(&in_params);
    let mut invoke_expr = format!(
        "_{}.method({}).invoke({}, [{}])",
        iface.name, method.vtable_index, statics_call, args_expr
    );
    invoke_expr = rewrite_delegate_args_in_expr(&invoke_expr, &params);

    let async_kind;
    let sync_return_expr;
    let async_convert_v;
    let mut progress_convert = None;

    if is_with_progress {
        let inner_type = match return_type_meta {
            Some(TypeMeta::AsyncOperationWithProgress(inner, _)) => Some(inner.as_ref()),
            _ => None,
        };
        let progress_type = match return_type_meta {
            Some(TypeMeta::AsyncOperationWithProgress(_, p)) => Some(p.as_ref()),
            Some(TypeMeta::AsyncActionWithProgress(p)) => Some(p.as_ref()),
            _ => None,
        };
        let progress_ts = progress_type
            .map(|p| ts_return_type_safe(Some(p), false, known_types))
            .unwrap_or_else(|| "unknown".to_string());
        // Build conversion expression for progress value
        let p_convert = convert_return("_p", progress_type, false, known_types, &NO_DEFERRED);
        if p_convert != "_p" {
            progress_convert = Some(p_convert);
        }
        let is_action = matches!(return_type_meta, Some(TypeMeta::AsyncActionWithProgress(_)));
        let inner_convert = convert_return("_v", inner_type, false, known_types, &NO_DEFERRED);
        if is_action {
            async_kind = AsyncKind::ActionWithProgress(progress_ts);
        } else {
            let inner_ts = ts_return_type_safe(inner_type, false, known_types);
            async_kind = AsyncKind::OperationWithProgress(inner_ts, progress_ts);
        }
        sync_return_expr = None;
        async_convert_v = Some(inner_convert);
    } else if is_async {
        let inner_type = async_inner_type(return_type_meta);
        let is_action = matches!(return_type_meta, Some(TypeMeta::AsyncAction));
        if is_action {
            async_kind = AsyncKind::Action;
            async_convert_v = None;
        } else {
            let convert_v = convert_return("_v", inner_type, false, known_types, &NO_DEFERRED);
            let inner_ts = ts_return_type_safe(inner_type, false, known_types);
            async_kind = AsyncKind::Operation(inner_ts);
            async_convert_v = Some(convert_v);
        }
        sync_return_expr = None;
    } else {
        async_kind = AsyncKind::None;
        let converted = convert_return(&invoke_expr, return_type_meta, false, known_types, &NO_DEFERRED);
        sync_return_expr = if return_type_meta.is_some() { Some(converted) } else { None };
        async_convert_v = None;
    }

    let mut ts_params = params;
    if is_async || is_with_progress {
        ts_params.push(ProjectedParam { name: "signal".into(), ts_type: "AbortSignal".into(), optional: true, delegate_wrap: None });
    }

    let mut doc = build_method_doc(method, &in_params);
    if is_async || is_with_progress {
        if let Some(ref mut d) = doc {
            d.params.push(("signal".into(), "Abort signal to cancel the underlying WinRT async operation.".into()));
        }
    }
    let delegate_wraps = collect_delegate_wraps(&ts_params);

    ProjectedMember::Method(ProjectedMethod {
        name: to_camel_case(&method.name),
        doc,
        params: ts_params,
        return_type: ts_return,
        async_kind,
        is_static: true,
        invoke_expr,
        sync_return_expr,
        async_convert_v,
        progress_convert,
        is_void: return_type_meta.is_none() && !is_async,
        array_return_expr: None,
        delegate_wraps,
        js_only: false,
    })
}

fn project_instance_method(
    iface_var: &str,
    obj_expr: &str,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
    iface_methods: Option<&[MethodMeta]>,
    delegate_sigs: &HashMap<String, String>,
    delegate_param_wraps: &HashMap<String, Vec<String>>,
) -> Option<ProjectedMember> {
    let in_params = get_in_params(method);
    let return_type_meta = method.return_type.as_ref();
    let is_with_progress = return_type_meta.is_some_and(|rt| matches!(rt,
        TypeMeta::AsyncOperationWithProgress(_, _) | TypeMeta::AsyncActionWithProgress(_)));
    let is_async = return_type_meta.is_some_and(|rt| rt.is_async()) && !is_with_progress;
    let has_array_out = method.params.iter().any(|p| {
        (p.direction == ParamDirection::Out || p.direction == ParamDirection::OutFill)
            && matches!(p.typ, TypeMeta::Array(_))
    });
    let has_return = return_type_meta.is_some() || has_array_out;

    let is_delegate_type = |typ: Option<&TypeMeta>| -> bool {
        match typ {
            Some(TypeMeta::Delegate { .. }) => true,
            Some(TypeMeta::Interface { name, .. }) => delegate_type_names.contains(name),
            _ => false,
        }
    };

    let mut doc = build_method_doc(method, &in_params);

    // Event add
    if method.is_event_add {
        return Some(project_event_add(iface_var, obj_expr, method, known_types, iface_methods, doc));
    }

    // Event remove
    if method.is_event_remove {
        let event_name = to_camel_case(method.name.strip_prefix("remove_").unwrap_or(&method.name));
        return Some(ProjectedMember::Event(ProjectedEvent {
            subscribe_name: String::new(),
            unsubscribe_name: format!("off{}", capitalize(&event_name)),
            callback_type: String::new(),
            doc,
            delegate_name: None,
            add_iface_var: String::new(),
            add_vtable_index: 0,
            add_obj_expr: String::new(),
            remove_vtable_index: Some(method.vtable_index),
            remove_iface_var: iface_var.into(),
            remove_obj_expr: obj_expr.into(),
            needs_wrap: false,
            sender_wrap: None,
            args_wrap: None,
        }));
    }

    // Property getter
    if method.is_property_getter && in_params.is_empty() {
        let prop_name = to_camel_case(method.name.strip_prefix("get_").unwrap_or(&method.name));
        let ts_return = if is_delegate_type(return_type_meta) {
            "DynWinRtValue".to_string()
        } else {
            ts_return_type_safe(return_type_meta, false, known_types)
        };
        let invoke_expr = format!(
            "{}.method({}).invoke({}, [])",
            iface_var, method.vtable_index, obj_expr
        );
        let converted = if is_delegate_type(return_type_meta) {
            invoke_expr.clone()
        } else {
            convert_return(&invoke_expr, return_type_meta, false, known_types, &NO_DEFERRED)
        };

        // Check if there's a corresponding setter
        let setter_line = find_setter_for_property(method, iface_var, obj_expr, iface_methods);

        return Some(ProjectedMember::Property(ProjectedProperty {
            name: prop_name,
            ts_type: ts_return,
            readonly: setter_line.is_none(),
            is_static: false,
            doc,
            getter_expr: converted,
            setter_line,
        }));
    }

    // Property setter (standalone — if paired with getter, handled above)
    if method.is_property_setter {
        let prop_name = to_camel_case(method.name.strip_prefix("put_").unwrap_or(&method.name));
        let param_type = if in_params.first().is_some_and(|p| is_delegate_type(Some(&p.typ))) {
            "DynWinRtValue".to_string()
        } else {
            in_params.first()
                .map(|p| ts_param_type_safe(&p.typ, known_types))
                .unwrap_or_else(|| "any".to_string())
        };
        let arg = in_params.first()
            .map(|p| wrap_arg("value", &p.typ))
            .unwrap_or_else(|| "value".to_string());
        let setter_line = format!(
            "{}.method({}).invoke({}, [{}]);",
            iface_var, method.vtable_index, obj_expr, arg
        );

        // Check if there's a corresponding getter (if so, it will add the property)
        let getter_name = format!("get_{}", method.name.strip_prefix("put_").unwrap_or(&method.name));
        let has_getter = iface_methods.map_or(false, |methods| {
            methods.iter().any(|m| m.name == getter_name && m.is_property_getter)
        });
        if has_getter {
            // The getter will create the property with this setter included — skip
            return None;
        }

        return Some(ProjectedMember::Property(ProjectedProperty {
            name: prop_name,
            ts_type: param_type,
            readonly: false,
            is_static: false,
            doc,
            getter_expr: String::new(),
            setter_line: Some(setter_line),
        }));
    }

    // Normal method
    let params = project_params(&in_params, known_types, delegate_type_names, delegate_sigs, delegate_param_wraps);    let array_out_elem = if has_array_out && return_type_meta.is_none() {
        method.params.iter().find_map(|p| {
            if p.direction == ParamDirection::Out || p.direction == ParamDirection::OutFill {
                if let TypeMeta::Array(inner) = &p.typ { Some(inner.as_ref()) } else { None }
            } else { None }
        })
    } else { None };

    let ts_return = if let Some(elem) = array_out_elem {
        ts_array_element_type(elem, known_types)
    } else {
        ts_return_type_safe(return_type_meta, is_async, known_types)
    };

    let args_expr = build_args_expr(&in_params);
    let mut invoke_expr = format!(
        "{}.method({}).invoke({}, [{}])",
        iface_var, method.vtable_index, obj_expr, args_expr
    );
    invoke_expr = rewrite_delegate_args_in_expr(&invoke_expr, &params);

    let async_kind;
    let sync_return_expr;
    let async_convert_v;
    let array_return_expr;
    let mut progress_convert = None;

    if is_with_progress {
        let inner_type = match return_type_meta {
            Some(TypeMeta::AsyncOperationWithProgress(inner, _)) => Some(inner.as_ref()),
            _ => None,
        };
        let progress_type = match return_type_meta {
            Some(TypeMeta::AsyncOperationWithProgress(_, p)) => Some(p.as_ref()),
            Some(TypeMeta::AsyncActionWithProgress(p)) => Some(p.as_ref()),
            _ => None,
        };
        let progress_ts = progress_type
            .map(|p| ts_return_type_safe(Some(p), false, known_types))
            .unwrap_or_else(|| "unknown".to_string());
        let p_convert = convert_return("_p", progress_type, false, known_types, &NO_DEFERRED);
        if p_convert != "_p" {
            progress_convert = Some(p_convert);
        }
        let is_action = matches!(return_type_meta, Some(TypeMeta::AsyncActionWithProgress(_)));
        let inner_convert = convert_return("_v", inner_type, false, known_types, &NO_DEFERRED);
        if is_action {
            async_kind = AsyncKind::ActionWithProgress(progress_ts);
        } else {
            let inner_ts = ts_return_type_safe(inner_type, false, known_types);
            async_kind = AsyncKind::OperationWithProgress(inner_ts, progress_ts);
        }
        sync_return_expr = None;
        async_convert_v = Some(inner_convert);
        array_return_expr = None;
    } else if is_async {
        let inner_type = async_inner_type(return_type_meta);
        let is_action = matches!(return_type_meta, Some(TypeMeta::AsyncAction));
        if is_action {
            async_kind = AsyncKind::Action;
            async_convert_v = None;
        } else {
            let convert_v = convert_return("_v", inner_type, false, known_types, &NO_DEFERRED);
            let inner_ts = ts_return_type_safe(inner_type, false, known_types);
            async_kind = AsyncKind::Operation(inner_ts);
            async_convert_v = Some(convert_v);
        }
        sync_return_expr = None;
        array_return_expr = None;
    } else if let Some(elem) = array_out_elem {
        async_kind = AsyncKind::None;
        let arr_expr = format!("{}.asArray()", invoke_expr);
        let converted = convert_array_return(&arr_expr, elem, known_types, &NO_DEFERRED);
        sync_return_expr = None;
        async_convert_v = None;
        array_return_expr = Some(converted);
    } else {
        async_kind = AsyncKind::None;
        if has_return {
            let converted = convert_return(&invoke_expr, return_type_meta, false, known_types, &NO_DEFERRED);
            sync_return_expr = Some(converted);
        } else {
            sync_return_expr = None;
        }
        async_convert_v = None;
        array_return_expr = None;
    }

    let is_void = !has_return && !is_async;

    let mut ts_params = params;
    if is_async || is_with_progress {
        ts_params.push(ProjectedParam { name: "signal".into(), ts_type: "AbortSignal".into(), optional: true, delegate_wrap: None });
        // Add @param signal doc
        if let Some(ref mut d) = doc {
            d.params.push(("signal".into(), "Abort signal to cancel the underlying WinRT async operation.".into()));
        }
    }

    let delegate_wraps = collect_delegate_wraps(&ts_params);

    Some(ProjectedMember::Method(ProjectedMethod {
        name: to_camel_case(&method.name),
        doc,
        params: ts_params,
        return_type: ts_return,
        async_kind,
        is_static: false,
        invoke_expr,
        sync_return_expr,
        async_convert_v,
        progress_convert,
        is_void,
        array_return_expr,
        delegate_wraps,
        js_only: false,
    }))
}

fn project_event_add(
    iface_var: &str,
    obj_expr: &str,
    method: &MethodMeta,
    known_types: &HashSet<String>,
    iface_methods: Option<&[MethodMeta]>,
    doc: Option<DocInfo>,
) -> ProjectedMember {
    let in_params = get_in_params(method);
    let event_name = to_camel_case(method.name.strip_prefix("add_").unwrap_or(&method.name));
    let cap = capitalize(&event_name);
    let delegate_first_param = in_params.first().map(|p| &p.typ);
    let delegate_name = delegate_first_param.and_then(|t| match t {
        TypeMeta::Parameterized { name, args, .. } =>
            Some(crate::meta::make_parameterized_name(name, args)),
        TypeMeta::Delegate { name, .. } => Some(name.clone()),
        _ => None,
    });
    let suffix = method.name.strip_prefix("add_").unwrap_or(&method.name);
    let remove_idx = iface_methods.and_then(|methods| {
        let target = format!("remove_{}", suffix);
        methods.iter().find(|m| m.name == target).map(|m| m.vtable_index)
    });

    let (callback_ts, sender_wrap, args_wrap) = match delegate_first_param {
        Some(TypeMeta::Parameterized { name, args, .. })
            if name.split('`').next() == Some("TypedEventHandler") && args.len() == 2 =>
        {
            let s_ts = ts_return_type_safe(Some(&args[0]), false, known_types);
            let a_ts = ts_return_type_safe(Some(&args[1]), false, known_types);
            // Map DynWinRtValue → unknown for event callback params (more TS-idiomatic)
            let s_ts_pub = if s_ts == "DynWinRtValue" { "unknown".to_string() } else { s_ts };
            let a_ts_pub = if a_ts == "DynWinRtValue" { "unknown".to_string() } else { a_ts };
            let s_wrap = convert_return("__a0__", Some(&args[0]), false, known_types, &NO_DEFERRED);
            let a_wrap = convert_return("__a1__", Some(&args[1]), false, known_types, &NO_DEFERRED);
            (
                format!("(sender: {}, args: {}) => void", s_ts_pub, a_ts_pub),
                Some(s_wrap),
                Some(a_wrap),
            )
        }
        Some(TypeMeta::Parameterized { name, args, .. })
            if name.split('`').next() == Some("EventHandler") && args.len() == 1 =>
        {
            let a_ts = ts_return_type_safe(Some(&args[0]), false, known_types);
            let a_ts_pub = if a_ts == "DynWinRtValue" { "unknown".to_string() } else { a_ts };
            let a_wrap = convert_return("__a1__", Some(&args[0]), false, known_types, &NO_DEFERRED);
            (
                format!("(sender: unknown, args: {}) => void", a_ts_pub),
                None,
                Some(a_wrap),
            )
        }
        _ => ("(...args: unknown[]) => void".to_string(), None, None),
    };

    let needs_wrap = sender_wrap.as_deref().is_some_and(|s| s != "__a0__")
        || args_wrap.as_deref().is_some_and(|s| s != "__a1__");

    ProjectedMember::Event(ProjectedEvent {
        subscribe_name: format!("on{}", cap),
        unsubscribe_name: format!("off{}", cap),
        callback_type: callback_ts,
        doc,
        delegate_name,
        add_iface_var: iface_var.into(),
        add_vtable_index: method.vtable_index,
        add_obj_expr: obj_expr.into(),
        remove_vtable_index: remove_idx,
        remove_iface_var: iface_var.into(),
        remove_obj_expr: obj_expr.into(),
        needs_wrap,
        sender_wrap,
        args_wrap,
    })
}

fn find_setter_for_property(
    getter: &MethodMeta,
    iface_var: &str,
    obj_expr: &str,
    iface_methods: Option<&[MethodMeta]>,
) -> Option<String> {
    let prop_suffix = getter.name.strip_prefix("get_")?;
    let setter_name = format!("put_{}", prop_suffix);
    let methods = iface_methods?;
    let setter = methods.iter().find(|m| m.name == setter_name && m.is_property_setter)?;
    let setter_in_params = get_in_params(setter);
    let arg = setter_in_params.first()
        .map(|p| wrap_arg("value", &p.typ))
        .unwrap_or_else(|| "value".to_string());
    Some(format!(
        "{}.method({}).invoke({}, [{}]);",
        iface_var, setter.vtable_index, obj_expr, arg
    ))
}

// ======================================================================
// Collection helpers
// ======================================================================

/// Create a fill-array expression for getMany: allocates a DynWinRtArray of
/// `count_var` elements, pre-filled with type-appropriate defaults.
/// Returns `None` for element types that have no typed batch constructor.
fn ts_fill_array_create(count_var: &str, elem: &TypeMeta) -> Option<String> {
    let (method, fill) = match elem {
        TypeMeta::I8 => ("fromI8Values", "0"),
        TypeMeta::U8 => ("fromU8Values", "0"),
        TypeMeta::I16 => ("fromI16Values", "0"),
        TypeMeta::U16 | TypeMeta::Char16 => ("fromU16Values", "0"),
        TypeMeta::I32 | TypeMeta::Enum { .. } => ("fromI32Values", "0"),
        TypeMeta::U32 => ("fromU32Values", "0"),
        TypeMeta::I64 => ("fromI64Values", "0"),
        TypeMeta::U64 => ("fromU64Values", "0"),
        TypeMeta::F32 => ("fromF32Values", "0"),
        TypeMeta::F64 => ("fromF64Values", "0"),
        TypeMeta::String => ("fromStringValues", "''"),
        _ => return None,
    };
    Some(format!("DynWinRtArray.{}(new Array({}).fill({}))", method, count_var, fill))
}

/// Create a DynWinRtArray from a JS array variable for replaceAll.
/// Returns `None` for element types that have no typed batch constructor.
fn ts_array_from_items(items_var: &str, elem: &TypeMeta) -> Option<String> {
    let method = match elem {
        TypeMeta::I8 => "fromI8Values",
        TypeMeta::U8 => "fromU8Values",
        TypeMeta::I16 => "fromI16Values",
        TypeMeta::U16 | TypeMeta::Char16 => "fromU16Values",
        TypeMeta::I32 | TypeMeta::Enum { .. } => "fromI32Values",
        TypeMeta::U32 => "fromU32Values",
        TypeMeta::I64 => "fromI64Values",
        TypeMeta::U64 => "fromU64Values",
        TypeMeta::F32 => "fromF32Values",
        TypeMeta::F64 => "fromF64Values",
        TypeMeta::String => "fromStringValues",
        _ => return None,
    };
    Some(format!("DynWinRtArray.{}({})", method, items_var))
}

fn project_collection_helpers(
    iface: &InterfaceMeta,
    known_types: &HashSet<String>,
    members: &mut Vec<ProjectedMember>,
    imports: &mut Vec<ProjectedImport>,
) {
    let Some(piid) = iface.generic_piid.as_deref() else { return };

    // If the generic arg is a known parameterized type, it needs to be imported
    // for the IterableIterator<T> / T[] type annotations in DTS
    if !iface.generic_args.is_empty() {
        for arg in &iface.generic_args {
            let type_name = ts_param_type_safe(arg, known_types);
            // Primitive types (string, number, boolean, DynWinRtValue, any) don't need import
            if !["string", "number", "boolean", "DynWinRtValue", "DynWinRtArray", "any", "void"]
                .contains(&type_name.as_str())
                && known_types.contains(&type_name)
            {
                let already_imported = imports.iter().any(|i| i.symbols.contains(&type_name));
                if !already_imported {
                    imports.push(ProjectedImport {
                        symbols: vec![type_name],
                        from: format!("./{}.js", ts_param_type_safe(arg, known_types)),
                        runtime_only: false, dts_only: false,
                    });
                }
            }
        }
    }

    match piid {
        PIID_IVECTOR | PIID_IVECTOR_VIEW if iface.generic_args.len() == 1 => {
            let elem_ts = ts_param_type_safe(&iface.generic_args[0], known_types);
            members.push(ProjectedMember::Symbol(ProjectedSymbol {
                kind: SymbolKind::CollectionLength,
                doc: Some("Alias for {@link size}; matches Array.length / TypedArray.length.".into()),
            }));
            members.push(ProjectedMember::Symbol(ProjectedSymbol {
                kind: SymbolKind::CollectionAt { element_type: elem_ts.clone() },
                doc: Some("Element at `index`. Negative indices count from the end (Array.prototype.at semantics).".into()),
            }));
            members.push(ProjectedMember::Symbol(ProjectedSymbol {
                kind: SymbolKind::CollectionToArray { element_type: elem_ts.clone() },
                doc: Some("Materialize as a plain JS array.".into()),
            }));
            members.push(ProjectedMember::Symbol(ProjectedSymbol {
                kind: SymbolKind::Iterator {
                    element_type: elem_ts.clone(),
                    body_lines: vec![
                        "const n = this.size;".into(),
                        "for (let i = 0; i < n; i++) yield this.getAt(i);".into(),
                    ],
                },
                doc: None,
            }));

            // JS-native indexOf: returns index or -1 (instead of WinRT boolean)
            members.push(ProjectedMember::Method(ProjectedMethod {
                name: "indexOf".into(),
                doc: Some(DocInfo {
                    summary: Some("Return the index of `value`, or -1 if not found.".into()),
                    deprecated: None, returns: None, params: vec![],
                }),
                params: vec![ProjectedParam {
                    name: "value".into(),
                    ts_type: elem_ts.clone(),
                    optional: false,
                    delegate_wrap: None,
                }],
                return_type: "number".into(),
                async_kind: AsyncKind::None,
                is_static: false,
                invoke_expr: String::new(),
                sync_return_expr: Some(
                    "(() => { const n = this.size; for (let i = 0; i < n; i++) { if (this.getAt(i) === value) return i; } return -1; })()".into()
                ),
                async_convert_v: None,
                is_void: false,
                array_return_expr: None,
                delegate_wraps: vec![],
                progress_convert: None,
                js_only: false,
            }));

            // High-level getMany: T[] wrapper over the raw FillArray-based method
            let iface_var = format!("_{}", iface.name);
            let elem = &iface.generic_args[0];
            if let Some(get_many_idx) = iface.methods.iter()
                .find(|m| m.name == "GetMany")
                .map(|m| m.vtable_index)
            {
                if let Some(fill_expr) = ts_fill_array_create("count", elem) {
                    let invoke = format!(
                        "{iface_var}.method({get_many_idx}).invoke(this._obj, \
                         [DynWinRtValue.i32(startIndex), _a.toValue()])"
                    );
                    let arr_convert = convert_array_return(
                        &format!("{invoke}.asArray()"), elem, known_types, &NO_DEFERRED,
                    );
                    let return_expr = format!(
                        "(() => {{ const _a = {fill_expr}; return {arr_convert}; }})()"
                    );
                    members.push(ProjectedMember::Method(ProjectedMethod {
                        name: "getMany".into(),
                        doc: Some(DocInfo {
                            summary: Some(
                                "Copy elements starting at `startIndex` into a new array \
                                 of length `count`."
                                    .into(),
                            ),
                            params: vec![],
                            returns: None,
                            deprecated: None,
                        }),
                        params: vec![
                            ProjectedParam {
                                name: "startIndex".into(),
                                ts_type: "number".into(),
                                optional: false,
                                delegate_wrap: None,
                            },
                            ProjectedParam {
                                name: "count".into(),
                                ts_type: "number".into(),
                                optional: false,
                                delegate_wrap: None,
                            },
                        ],
                        return_type: format!("{}[]", elem_ts),
                        async_kind: AsyncKind::None,
                        is_static: false,
                        invoke_expr: String::new(),
                        sync_return_expr: Some(return_expr),
                        async_convert_v: None,
                        is_void: false,
                        array_return_expr: None,
                        delegate_wraps: vec![],
                        progress_convert: None,
                        js_only: false,
                    }));
                }
            }

            // High-level replaceAll (IVector only): accepts T[] instead of DynWinRtArray
            if piid == PIID_IVECTOR {
                if let Some(replace_all_idx) = iface.methods.iter()
                    .find(|m| m.name == "ReplaceAll")
                    .map(|m| m.vtable_index)
                {
                    if let Some(items_expr) = ts_array_from_items("items", elem) {
                        let invoke = format!(
                            "{iface_var}.method({replace_all_idx}).invoke(this._obj, \
                             [{items_expr}.toValue()])"
                        );
                        members.push(ProjectedMember::Method(ProjectedMethod {
                            name: "replaceAll".into(),
                            doc: Some(DocInfo {
                                summary: Some(
                                    "Replace all elements in the vector with the provided items."
                                        .into(),
                                ),
                                params: vec![],
                                returns: None,
                                deprecated: None,
                            }),
                            params: vec![ProjectedParam {
                                name: "items".into(),
                                ts_type: format!("{}[]", elem_ts),
                                optional: false,
                                delegate_wrap: None,
                            }],
                            return_type: "void".into(),
                            async_kind: AsyncKind::None,
                            is_static: false,
                            invoke_expr: invoke,
                            sync_return_expr: None,
                            async_convert_v: None,
                            is_void: true,
                            array_return_expr: None,
                            delegate_wraps: vec![],
                            progress_convert: None,
                            js_only: false,
                        }));
                    }
                }
            }
        }
        PIID_IITERATOR if iface.generic_args.len() == 1 => {
            let elem_ts = ts_param_type_safe(&iface.generic_args[0], known_types);
            members.push(ProjectedMember::Symbol(ProjectedSymbol {
                kind: SymbolKind::IteratorNext { element_type: elem_ts.clone() },
                doc: Some("JS iterator protocol: returns the current element and advances.".into()),
            }));
            // IIterator is already the iterator — [Symbol.iterator]() returns this
            members.push(ProjectedMember::Symbol(ProjectedSymbol {
                kind: SymbolKind::Iterator {
                    element_type: elem_ts,
                    body_lines: vec!["return this;".into()],
                },
                doc: None,
            }));
        }
        PIID_IITERABLE if iface.generic_args.len() == 1 => {
            let elem_ts = ts_param_type_safe(&iface.generic_args[0], known_types);
            members.push(ProjectedMember::Symbol(ProjectedSymbol {
                kind: SymbolKind::Iterator {
                    element_type: elem_ts,
                    body_lines: vec![],
                },
                doc: None,
            }));
        }
        PIID_IMAP | PIID_IMAP_VIEW if iface.generic_args.len() == 2 => {
            let key_ts = ts_param_type_safe(&iface.generic_args[0], known_types);
            let val_ts = ts_param_type_safe(&iface.generic_args[1], known_types);
            let key_ts = if key_ts == "DynWinRtValue" { "unknown".to_string() } else { key_ts };
            let val_ts = if val_ts == "DynWinRtValue" { "unknown".to_string() } else { val_ts };
            // JS Map-like aliases
            let iface_var = format!("_{}", iface.name);
            // get(key) — alias for lookup
            if let Some(lookup_idx) = iface.methods.iter().find(|m| m.name == "Lookup").map(|m| m.vtable_index) {
                let key_wrap = wrap_arg("key", &iface.generic_args[0]);
                let return_convert = convert_return(
                    &format!("{iface_var}.method({lookup_idx}).invoke(this._obj, [{key_wrap}])"),
                    Some(&iface.generic_args[1]), false, known_types, &NO_DEFERRED,
                );
                members.push(ProjectedMember::Method(ProjectedMethod {
                    name: "get".into(),
                    doc: Some(DocInfo {
                        summary: Some(format!("Get the value for `key`. Alias for `lookup()`.")),
                        deprecated: None, returns: None, params: vec![],
                    }),
                    params: vec![ProjectedParam { name: "key".into(), ts_type: key_ts.clone(), optional: false, delegate_wrap: None }],
                    return_type: format!("{} | undefined", val_ts),
                    async_kind: AsyncKind::None, is_static: false,
                    invoke_expr: String::new(),
                    sync_return_expr: Some(format!("(() => {{ try {{ return {}; }} catch {{ return undefined; }} }})()", return_convert)),
                    async_convert_v: None, is_void: false, array_return_expr: None,
                    delegate_wraps: vec![], progress_convert: None, js_only: false,
                }));
            }
            // has(key) — alias for hasKey
            if let Some(has_idx) = iface.methods.iter().find(|m| m.name == "HasKey").map(|m| m.vtable_index) {
                let key_wrap = wrap_arg("key", &iface.generic_args[0]);
                members.push(ProjectedMember::Method(ProjectedMethod {
                    name: "has".into(),
                    doc: Some(DocInfo {
                        summary: Some("Check if the map contains `key`. Alias for `hasKey()`.".into()),
                        deprecated: None, returns: None, params: vec![],
                    }),
                    params: vec![ProjectedParam { name: "key".into(), ts_type: key_ts.clone(), optional: false, delegate_wrap: None }],
                    return_type: "boolean".into(),
                    async_kind: AsyncKind::None, is_static: false,
                    invoke_expr: format!("{iface_var}.method({has_idx}).invoke(this._obj, [{key_wrap}])"),
                    sync_return_expr: Some(format!("{iface_var}.method({has_idx}).invoke(this._obj, [{key_wrap}]).toBool()")),
                    async_convert_v: None, is_void: false, array_return_expr: None,
                    delegate_wraps: vec![], progress_convert: None, js_only: false,
                }));
            }
            // set(key, value) — alias for insert (IMap only)
            if piid == PIID_IMAP {
                if let Some(insert_idx) = iface.methods.iter().find(|m| m.name == "Insert").map(|m| m.vtable_index) {
                    let key_wrap = wrap_arg("key", &iface.generic_args[0]);
                    let val_wrap = wrap_arg("value", &iface.generic_args[1]);
                    members.push(ProjectedMember::Method(ProjectedMethod {
                        name: "set".into(),
                        doc: Some(DocInfo {
                            summary: Some("Set a key-value pair. Alias for `insert()`.".into()),
                            deprecated: None, returns: None, params: vec![],
                        }),
                        params: vec![
                            ProjectedParam { name: "key".into(), ts_type: key_ts.clone(), optional: false, delegate_wrap: None },
                            ProjectedParam { name: "value".into(), ts_type: val_ts.clone(), optional: false, delegate_wrap: None },
                        ],
                        return_type: "void".into(),
                        async_kind: AsyncKind::None, is_static: false,
                        invoke_expr: format!("{iface_var}.method({insert_idx}).invoke(this._obj, [{key_wrap}, {val_wrap}])"),
                        sync_return_expr: None,
                        async_convert_v: None, is_void: true, array_return_expr: None,
                        delegate_wraps: vec![], progress_convert: None, js_only: false,
                    }));
                }
                // delete(key) — alias for remove
                if let Some(remove_idx) = iface.methods.iter().find(|m| m.name == "Remove").map(|m| m.vtable_index) {
                    let key_wrap = wrap_arg("key", &iface.generic_args[0]);
                    members.push(ProjectedMember::Method(ProjectedMethod {
                        name: "delete".into(),
                        doc: Some(DocInfo {
                            summary: Some("Remove entry by key. Alias for `remove()`.".into()),
                            deprecated: None, returns: None, params: vec![],
                        }),
                        params: vec![ProjectedParam { name: "key".into(), ts_type: key_ts.clone(), optional: false, delegate_wrap: None }],
                        return_type: "void".into(),
                        async_kind: AsyncKind::None, is_static: false,
                        invoke_expr: format!("{iface_var}.method({remove_idx}).invoke(this._obj, [{key_wrap}])"),
                        sync_return_expr: None,
                        async_convert_v: None, is_void: true, array_return_expr: None,
                        delegate_wraps: vec![], progress_convert: None, js_only: false,
                    }));
                }
            }
            // forEach — iterate over entries
            members.push(ProjectedMember::Symbol(ProjectedSymbol {
                kind: SymbolKind::CollectionLength,
                doc: Some("Number of entries. Alias for `size`.".into()),
            }));
        }
        _ => {}
    }
}

fn project_collection_create(
    iface: &InterfaceMeta,
    known_types: &HashSet<String>,
    members: &mut Vec<ProjectedMember>,
) {
    let Some(ref piid) = iface.generic_piid else { return };
    if piid == PIID_IVECTOR && iface.generic_args.len() == 1 {
        let elem_type = ts_dynwinrt_type(&iface.generic_args[0]);
        let elem_ts = ts_param_type_safe(&iface.generic_args[0], known_types);
        members.push(ProjectedMember::Method(ProjectedMethod {
            name: "create".into(),
            doc: Some(DocInfo {
                summary: Some("Create a new IVector from an array of items.".into()),
                deprecated: None, returns: None, params: vec![],
            }),
            params: vec![ProjectedParam { name: "items".into(), ts_type: format!("{}[]", elem_ts), optional: false, delegate_wrap: None }],
            return_type: iface.name.clone(),
            async_kind: AsyncKind::None,
            is_static: true,
            invoke_expr: String::new(),
            sync_return_expr: Some(format!(
                "new {}(DynWinRtValue.createVector(items.map(i => _unwrap(i)), {}))",
                iface.name, elem_type
            )),
            async_convert_v: None,
            is_void: false,
            array_return_expr: None,
            delegate_wraps: vec![],
            progress_convert: None,
            js_only: false,
        }));
    } else if piid == PIID_IMAP && iface.generic_args.len() == 2 {
        let key_type = ts_dynwinrt_type(&iface.generic_args[0]);
        let val_type = ts_dynwinrt_type(&iface.generic_args[1]);
        let key_ts = ts_param_type_safe(&iface.generic_args[0], known_types);
        let key_ts = if key_ts == "DynWinRtValue" { "unknown".to_string() } else { key_ts };
        let val_ts = ts_param_type_safe(&iface.generic_args[1], known_types);
        let val_ts = if val_ts == "DynWinRtValue" { "unknown".to_string() } else { val_ts };
        members.push(ProjectedMember::Method(ProjectedMethod {
            name: "create".into(),
            doc: Some(DocInfo {
                summary: Some("Create a new IMap from parallel arrays of keys and values.".into()),
                deprecated: None, returns: None, params: vec![],
            }),
            params: vec![
                ProjectedParam { name: "keys".into(), ts_type: format!("{}[]", key_ts), optional: false, delegate_wrap: None },
                ProjectedParam { name: "values".into(), ts_type: format!("{}[]", val_ts), optional: false, delegate_wrap: None },
            ],
            return_type: iface.name.clone(),
            async_kind: AsyncKind::None,
            is_static: true,
            invoke_expr: String::new(),
            sync_return_expr: Some(format!(
                "new {}(DynWinRtValue.createMap(keys.map(k => _unwrap(k)), values.map(v => _unwrap(v)), {}, {}))",
                iface.name, key_type, val_type
            )),
            async_convert_v: None,
            is_void: false,
            array_return_expr: None,
            delegate_wraps: vec![],
            progress_convert: None,
            js_only: false,
        }));
    }
}

// ======================================================================
// Struct projection
// ======================================================================

fn project_struct_helpers(used_structs: &[TypeMeta]) -> Vec<ProjectedStruct> {
    used_structs.iter().filter_map(|s| {
        let (namespace, name, fields) = match s {
            TypeMeta::Struct { namespace, name, fields } => (namespace, name, fields),
            _ => return None,
        };
        let full_name = format!("{}.{}", namespace, name);
        let field_types: Vec<String> = fields.iter().map(|f| ts_dynwinrt_type(&f.typ)).collect();
        let type_expr = format!("DynWinRtType.structType('{}', [{}])", full_name, field_types.join(", "));

        let ts_fields: Vec<(String, String)> = fields.iter().map(|f| {
            (to_camel_case(&f.name), ts_struct_field_type(&f.typ))
        }).collect();

        let unpack_body = {
            let field_exprs: Vec<String> = fields.iter().enumerate().map(|(i, f)| {
                format!("{}: {}", to_camel_case(&f.name), struct_field_getter(&f.typ, i))
            }).collect();
            vec![
                "const s = v.asStruct();".into(),
                format!("return {{ {} }};", field_exprs.join(", ")),
            ]
        };

        let pack_body = {
            let mut lines = vec![format!("const s = DynWinRtStruct.create({}_Type);", name)];
            for (i, f) in fields.iter().enumerate() {
                lines.push(format!("{};", struct_field_setter(&f.typ, i, &format!("v.{}", to_camel_case(&f.name)))));
            }
            lines.push("return s;".into());
            lines
        };

        Some(ProjectedStruct {
            name: name.clone(),
            fields: ts_fields,
            unpack_body,
            pack_body,
            type_expr,
            namespace: namespace.clone(),
        })
    }).collect()
}

// ======================================================================
// Utility helpers
// ======================================================================

/// Returns a dedup key for a SymbolKind so flatten can detect duplicate symbols.
fn symbol_dedup_key(kind: &SymbolKind) -> String {
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
        "DynWinRtType".into(), "DynWinRtMethodSig".into(), "DynWinRtValue".into(),
        "DynWinRtArray".into(),
    ];
    if has_structs {
        symbols.push("DynWinRtStruct".into());
    }
    symbols.push("DynWinRtDelegate".into());
    symbols.push("WinGuid".into());
    ProjectedImport {
        symbols,
        from: "@microsoft/dynwinrt".into(),
        runtime_only: false, dts_only: false,
    }
}

fn format_type_import_projected(name: &str, kind: TypeKind) -> ProjectedImport {
    if kind == TypeKind::Interface {
        ProjectedImport {
            symbols: vec![format!("IID_{}", name), name.into()],
            from: format!("./{}.js", name),
            runtime_only: false, dts_only: false,
        }
    } else {
        ProjectedImport {
            symbols: vec![name.into()],
            from: format!("./{}.js", name),
            runtime_only: false, dts_only: false,
        }
    }
}

fn collect_delegate_names_from_methods(methods: &[MethodMeta], delegate_names: &mut HashSet<String>) {
    for method in methods {
        for p in &method.params {
            match &p.typ {
                TypeMeta::Delegate { name, .. } => { delegate_names.insert(name.clone()); }
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

fn project_params(in_params: &[&crate::meta::ParamMeta], known_types: &HashSet<String>, delegate_names: &HashSet<String>, delegate_sigs: &HashMap<String, String>, delegate_param_wraps: &HashMap<String, Vec<String>>) -> Vec<ProjectedParam> {
    in_params.iter().map(|p| {
        let mut ts = ts_param_type_dts(&p.typ, known_types);
        let mut delegate_wrap = None;
        if delegate_names.contains(&ts) {
            let orig_name = ts.clone();
            if let Some(sig) = delegate_sigs.get(&ts) {
                ts = sig.clone();
                let wraps = delegate_param_wraps.get(&orig_name).cloned().unwrap_or_default();
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
    }).collect()
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
        params: params.iter().map(|(n, d)| (n.to_string(), d.to_string())).collect(),
    })
}

fn build_method_doc(method: &MethodMeta, in_params: &[&crate::meta::ParamMeta]) -> Option<DocInfo> {
    let params_display: Vec<(String, String)> = in_params.iter()
        .filter_map(|p| {
            super::xml_text::find_param_doc(&method.param_docs, &p.name)
                .map(|d| (to_camel_case(&p.name), d.to_string()))
        })
        .collect();
    let params_refs: Vec<(&str, &str)> = params_display.iter()
        .map(|(n, d)| (n.as_str(), d.as_str()))
        .collect();

    // If XML doc exists, use it
    if method.doc.is_some() || method.deprecated.is_some() || method.returns_doc.is_some() || !params_refs.is_empty() {
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
            Some(format!("Overload of `{}` with default {}.", base, words.join(", ")))
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
    if check_needs_unwrap_simple(members) { return true; }
    for ri in req_ifaces {
        if check_needs_unwrap_simple(&ri.members) { return true; }
    }
    false
}

fn check_needs_unwrap_simple(members: &[ProjectedMember]) -> bool {
    for m in members {
        match m {
            ProjectedMember::Method(pm) => {
                if contains_unwrap(&pm.invoke_expr) { return true; }
                if pm.sync_return_expr.as_deref().map_or(false, contains_unwrap) { return true; }
            }
            ProjectedMember::Property(pp) => {
                if contains_unwrap(&pp.getter_expr) { return true; }
                if pp.setter_line.as_deref().map_or(false, contains_unwrap) { return true; }
            }
            ProjectedMember::Event(pe) => {
                if pe.sender_wrap.as_deref().map_or(false, contains_unwrap) { return true; }
                if pe.args_wrap.as_deref().map_or(false, contains_unwrap) { return true; }
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
    params.iter()
        .filter_map(|p| p.delegate_wrap.as_ref().map(|dw| (p.name.clone(), dw.delegate_name.clone())))
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
