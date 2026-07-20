// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python runtime class generation.

use super::imports::{emit_type_checking_imports, format_py_type_import};
use super::structs::generate_struct_helpers;
use super::*;

/// Generate a Python file for a single RuntimeClass.
pub fn generate_class(
    class: &ClassMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
    shared_iids: &HashSet<String>,
) -> String {
    let used_structs = collect_used_structs_from_class(class);

    let mut out = String::new();

    // Header
    out.push_str(HEADER);
    out.push_str(FUTURE_ANNOTATIONS);
    out.push_str(IMPORT_LINE);
    if has_ireference_input(
        class
            .all_interfaces()
            .flat_map(|interface| interface.methods.iter()),
    ) {
        out.push_str(IREFERENCE_HELPER);
    }
    out.push('\n');
    let mut type_checking_imports = Vec::new();

    // Collect delegate names from all interfaces of this class
    let mut delegate_names: HashSet<String> = delegate_type_names.clone();
    let all_ifaces: Vec<&InterfaceMeta> = class.all_interfaces().collect();
    for iface in &all_ifaces {
        for method in &iface.methods {
            for p in &method.params {
                if let TypeMeta::Delegate { name, .. } = &p.typ {
                    delegate_names.insert(name.clone());
                }
                if method.is_event_add || method.is_event_remove {
                    if let TypeMeta::Parameterized { name, args, .. } = &p.typ {
                        delegate_names.insert(crate::meta::make_parameterized_name(name, args));
                    }
                }
            }
        }
    }

    // Collection generics import (skip delegates)
    let collection_names = collect_used_generics_from_class(class);
    for cname in &collection_names {
        if !delegate_names.contains(cname) {
            let module = to_snake_case_filename(cname);
            type_checking_imports
                .push(format!("from .{} import {}  # noqa: F401\n", module, cname));
        }
    }

    // Import delegate IID + PARAM_TYPES
    let mut sorted_delegates: Vec<_> = delegate_names.iter().collect();
    sorted_delegates.sort();
    for dname in &sorted_delegates {
        let module = to_snake_case_filename(dname);
        type_checking_imports.push(format!(
            "from .{module} import IID_{dname}, {dname}_PARAM_TYPES  # noqa: F401\n",
        ));
    }

    // Type imports
    let mut imported_names: HashSet<String> = HashSet::new();
    let imports = collect_type_imports(class);
    let mut sorted_imports: Vec<_> = imports.iter().collect();
    sorted_imports
        .sort_by(|a, b| (&a.namespace, &a.name, &a.kind).cmp(&(&b.namespace, &b.name, &b.kind)));
    for r in &sorted_imports {
        if known_types.contains(&r.name) && !delegate_names.contains(&r.name) {
            type_checking_imports.push(format_py_type_import(&r.name, r.kind));
            imported_names.insert(r.name.clone());
            if r.kind == TypeKind::Interface {
                imported_names.insert(format!("IID_{}", r.name));
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
            type_checking_imports.push(format_py_type_import(&req_iface.name, TypeKind::Interface));
            imported_names.insert(req_iface.name.clone());
            imported_names.insert(format!("IID_{}", req_iface.name));
        }
    }
    emit_type_checking_imports(&mut out, type_checking_imports);

    // IID constants are emitted locally even when the interface type is shared.
    // TYPE_CHECKING imports do not define runtime values, and interface
    // registration happens while this module is loading.
    let mut declared_iids = HashSet::new();
    let all_class_ifaces: Vec<&InterfaceMeta> = class.all_interfaces().collect();
    for iface in &all_class_ifaces {
        let iid_name = format!("IID_{}", iface.name);
        if declared_iids.insert(iid_name.clone()) {
            if let Some(iid_expr) = py_interface_iid_expr(iface) {
                out.push_str(&format!("{} = {}\n", iid_name, iid_expr));
            }
        }
    }
    out.push('\n');

    // Interface registrations
    if let Some(ref iface) = class.default_interface {
        out.push_str(&py_generate_interface_registration(
            iface,
            &format!("_{}", iface.name),
        ));
        out.push('\n');
    }
    for iface in &class.factory_interfaces {
        out.push_str(&py_generate_interface_registration(
            iface,
            &format!("_{}", iface.name),
        ));
        out.push('\n');
    }
    for iface in &class.static_interfaces {
        out.push_str(&py_generate_interface_registration(
            iface,
            &format!("_{}", iface.name),
        ));
        out.push('\n');
    }
    for iface in &class.required_interfaces {
        if !iface.iid.is_empty()
            && shared_iids.contains(&iface.iid)
            && imported_names.contains(&iface.name)
        {
            continue;
        }
        out.push_str(&py_generate_interface_registration(
            iface,
            &format!("_{}", iface.name),
        ));
        out.push('\n');
    }
    // IActivationFactory for default constructor
    if class.has_default_constructor {
        out.push_str("_IActivationFactory = DynWinRTType.register_interface(\n");
        out.push_str(
            "    'IActivationFactory', WinGUID.parse('00000035-0000-0000-c000-000000000046')) \\\n",
        );
        out.push_str("    .add_method('ActivateInstance', DynWinRTMethodSig().add_out(DynWinRTType.object()))\n");
        out.push('\n');
    }

    // Struct helpers
    for s in &used_structs {
        out.push_str(&generate_struct_helpers(s));
        out.push('\n');
    }

    // Class declaration
    out.push_str(&format!("\nclass {}:\n", class.name));
    {
        let doc = crate::codegen::shared::docs::DocText {
            summary: class.doc.as_deref(),
            deprecated: class.deprecated.as_deref(),
            ..Default::default()
        };
        out.push_str(&crate::codegen::python::docs::format_pydoc(&doc, "    "));
    }

    // Constructor
    out.push_str("    def __init__(self, obj: DynWinRTValue):\n");
    if let Some(ref iface) = class.default_interface {
        if !iface.iid.is_empty() {
            out.push_str(&format!(
                "        self._obj = obj.cast(IID_{})\n",
                iface.name
            ));
        } else {
            out.push_str("        self._obj = obj\n");
        }
    } else {
        out.push_str("        self._obj = obj\n");
    }
    out.push('\n');

    // Lazy-cached factory/static interface accessors
    let mut declared: HashSet<String> = HashSet::new();
    for iface in &class.factory_interfaces {
        let key = format!("f_{}", iface.name);
        if !iface.iid.is_empty() && declared.insert(key.clone()) {
            out.push_str(&format!("    _{} = None\n", key));
            out.push('\n');
            out.push_str("    @classmethod\n");
            out.push_str(&format!("    def _get_{}(cls):\n", key));
            out.push_str(&format!(
                "        if cls._{k} is None:\n\
                 \x20           cls._{k} = DynWinRTValue.activation_factory('{full}').cast(IID_{iface})\n\
                 \x20       return cls._{k}\n",
                k = key, iface = iface.name, full = class.full_name
            ));
            out.push('\n');
        }
    }
    for iface in &class.static_interfaces {
        let key = format!("s_{}", iface.name);
        if !iface.iid.is_empty() && declared.insert(key.clone()) {
            out.push_str(&format!("    _{} = None\n", key));
            out.push('\n');
            out.push_str("    @classmethod\n");
            out.push_str(&format!("    def _get_{}(cls):\n", key));
            out.push_str(&format!(
                "        if cls._{k} is None:\n\
                 \x20           cls._{k} = DynWinRTValue.activation_factory('{full}').cast(IID_{iface})\n\
                 \x20       return cls._{k}\n",
                k = key, iface = iface.name, full = class.full_name
            ));
            out.push('\n');
        }
    }

    // Default constructor
    if class.has_default_constructor {
        let has_create_factory = class.factory_interfaces.iter().any(|iface| {
            iface.methods.iter().any(|m| {
                let snake = to_snake_case(&m.name);
                snake == "create" || snake.starts_with("create")
            })
        });
        let ctor_name = if has_create_factory {
            "create_default"
        } else {
            "create"
        };
        out.push_str("    @staticmethod\n");
        out.push_str(&format!("    def {}() -> '{}':\n", ctor_name, class.name));
        out.push_str(&format!(
            "        return {}(_IActivationFactory.method(6).invoke(DynWinRTValue.activation_factory('{}'), []))\n",
            class.name, class.full_name
        ));
        out.push('\n');
    }

    // Factory methods
    for iface in &class.factory_interfaces {
        for method in &iface.methods {
            out.push('\n');
            out.push_str(&generate_factory_method_invoke(
                class,
                iface,
                method,
                known_types,
                &delegate_names,
            ));
        }
    }

    // Static methods
    for iface in &class.static_interfaces {
        for method in &iface.methods {
            out.push('\n');
            out.push_str(&generate_static_method_invoke(
                class,
                iface,
                method,
                known_types,
                &delegate_names,
            ));
        }
    }

    // Instance methods: default interface (reorder for Python @property before @setter)
    if let Some(ref default_iface) = class.default_interface {
        let iface_var = format!("_{}", default_iface.name);
        for method in reorder_getters_before_setters(&default_iface.methods) {
            out.push('\n');
            out.push_str(&generate_iface_instance_method(
                default_iface,
                &iface_var,
                method,
                known_types,
                &delegate_names,
            ));
        }
    }

    // Auto-generate close() if class implements IClosable
    const ICLOSABLE_IID: &str = "30d5a829-7fa4-4026-83bb-d75bae4ea99e";
    if class
        .required_interfaces
        .iter()
        .any(|ri| ri.iid == ICLOSABLE_IID)
    {
        out.push('\n');
        out.push_str("    def close(self):\n");
        out.push_str(&format!(
            "        {}.from_value(self._obj).close()\n",
            py_runtime_symbol("IClosable", "IClosable")
        ));
    }

    // .as_interface() method for accessing non-default interfaces
    if !class.required_interfaces.is_empty() {
        out.push('\n');
        out.push_str("    def as_interface(self, interface_class):\n");
        out.push_str("        return interface_class.from_value(self._obj)\n");
    }

    // Generate inline wrapper classes for required interfaces (non-default)
    for req_iface in &class.required_interfaces {
        if req_iface.iid.is_empty() {
            continue;
        }
        if imported_names.contains(&req_iface.name) {
            continue;
        }
        let reg_var = format!("_{}", req_iface.name);
        out.push('\n');
        out.push_str(&format!("\nclass {}:\n", req_iface.name));
        out.push_str("    def __init__(self, obj: DynWinRTValue):\n");
        out.push_str("        self._obj = obj\n");
        out.push('\n');
        out.push_str("    @staticmethod\n");
        out.push_str(&format!(
            "    def from_value(obj: DynWinRTValue) -> '{}':\n",
            req_iface.name
        ));
        out.push_str(&format!(
            "        return {}(obj.cast(IID_{}))\n",
            req_iface.name, req_iface.name
        ));
        for method in reorder_getters_before_setters(&req_iface.methods) {
            out.push('\n');
            out.push_str(&generate_iface_instance_method(
                req_iface,
                &reg_var,
                method,
                known_types,
                &delegate_names,
            ));
        }
    }
    out
}
