// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python runtime class generation.

use super::imports::{emit_type_checking_imports, format_py_type_import};
use super::structs::generate_struct_helpers;
use super::*;
use crate::codegen::python::collections::{
    CollectionKind, class_interface, interface_kind, map_iterable_name, runtime_mixin,
};

/// Generate a Python file for a single RuntimeClass.
pub fn generate_class(
    class: &ClassMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
    shared_iids: &HashSet<String>,
) -> String {
    let used_structs = collect_used_structs_from_class(class);
    let collection_iface = class_interface(class);
    let collection_kind = collection_iface.and_then(interface_kind);
    let collection_uses_default = collection_iface.is_some_and(|collection_iface| {
        class
            .default_interface
            .as_ref()
            .is_some_and(|default_iface| default_iface.name == collection_iface.name)
    });
    let collection_obj_expr = if collection_uses_default {
        "self._obj"
    } else {
        "self._collection_obj"
    };

    let mut out = String::new();

    // Header
    out.push_str(HEADER);
    out.push_str(FUTURE_ANNOTATIONS);
    out.push_str(IMPORT_LINE);
    let mut collection_mixins = class
        .default_interface
        .iter()
        .chain(class.required_interfaces.iter())
        .filter_map(interface_kind)
        .filter_map(runtime_mixin)
        .collect::<Vec<_>>();
    collection_mixins.sort_unstable();
    collection_mixins.dedup();
    if !collection_mixins.is_empty() {
        out.push_str(&format!(
            "from dynwinrt_py.dynwinrt_py import {}\n",
            collection_mixins.join(", ")
        ));
    }
    if methods_have_async_output(
        class
            .all_interfaces()
            .flat_map(|interface| interface.methods.iter()),
    ) {
        out.push_str(ASYNC_IMPORT_LINE);
    }
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
    let mut argument_iids = Vec::new();
    for iface in &all_class_ifaces {
        for method in &iface.methods {
            for parameter in &method.params {
                if parameter.direction == ParamDirection::In {
                    py_collect_runtime_class_iid_consts(&parameter.typ, &mut argument_iids);
                }
            }
        }
    }
    argument_iids.sort();
    argument_iids.dedup();
    for (name, iid) in argument_iids {
        if declared_iids.insert(name.clone()) {
            out.push_str(&format!("{} = WinGUID.parse('{}')\n", name, iid));
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
    if let Some(mixin) = collection_kind.and_then(runtime_mixin) {
        out.push_str(&format!("\nclass {}({mixin}):\n", class.name));
    } else {
        out.push_str(&format!("\nclass {}:\n", class.name));
    }
    {
        let doc = crate::codegen::shared::docs::DocText {
            summary: class.doc.as_deref(),
            deprecated: class.deprecated.as_deref(),
            ..Default::default()
        };
        out.push_str(&crate::codegen::python::docs::format_pydoc(&doc, "    "));
    }

    out.push_str(&generate_python_constructor(
        class,
        known_types,
        delegate_type_names,
        collection_iface,
        collection_uses_default,
    ));

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
        let ctor_name = default_constructor_name(has_create_factory);
        out.push_str("    @staticmethod\n");
        out.push_str(&format!("    def {}() -> '{}':\n", ctor_name, class.name));
        out.push_str(&format!(
            "        return {}._from_native(_IActivationFactory.method(6).invoke(DynWinRTValue.activation_factory('{}'), []))\n",
            class.name, class.full_name
        ));
        out.push('\n');
    }

    let static_methods = class
        .factory_interfaces
        .iter()
        .flat_map(|iface| iface.methods.iter())
        .chain(
            class
                .static_interfaces
                .iter()
                .flat_map(|iface| iface.methods.iter()),
        )
        .collect::<Vec<_>>();
    let static_method_names =
        crate::codegen::python::overloads::method_names(static_methods.iter().copied());
    let mut static_groups: Vec<(String, Vec<StaticOverload<'_>>)> = Vec::new();
    for (kind, interfaces) in [
        (StaticOverloadKind::Factory, &class.factory_interfaces),
        (StaticOverloadKind::Static, &class.static_interfaces),
    ] {
        for iface in interfaces {
            for method in &iface.methods {
                let mut key = crate::codegen::python::overloads::method_group_key(
                    method,
                    &static_method_names,
                );
                if method.is_property_getter
                    || method.is_property_setter
                    || method.is_event_add
                    || method.is_event_remove
                {
                    key = format!("{}#{key}", iface.name);
                }
                let overload = StaticOverload {
                    class,
                    iface,
                    method,
                    kind,
                };
                if let Some((_, group)) = static_groups
                    .iter_mut()
                    .find(|(group_key, _)| group_key == &key)
                {
                    group.push(overload);
                } else {
                    static_groups.push((key, vec![overload]));
                }
            }
        }
    }
    for (_, overloads) in static_groups {
        out.push('\n');
        out.push_str(&generate_static_method_group(
            &overloads,
            known_types,
            &delegate_names,
        ));
    }

    let mut method_groups: Vec<(String, Vec<InstanceOverload<'_>>)> = Vec::new();
    let instance_ifaces = class
        .default_interface
        .iter()
        .chain(class.required_interfaces.iter())
        .filter(|iface| iface.iid != "30d5a829-7fa4-4026-83bb-d75bae4ea99e")
        .collect::<Vec<_>>();
    let instance_method_names = crate::codegen::python::overloads::method_names(
        instance_ifaces
            .iter()
            .flat_map(|iface| iface.methods.iter()),
    );
    for iface in instance_ifaces {
        let obj_expr = if collection_iface.is_some_and(|collection| collection.name == iface.name) {
            collection_obj_expr
        } else if class
            .default_interface
            .as_ref()
            .is_some_and(|default_iface| default_iface.name == iface.name)
        {
            "self._obj"
        } else {
            ""
        };
        let obj_expr = if obj_expr.is_empty() {
            format!("self._obj.cast(IID_{})", iface.name)
        } else {
            obj_expr.to_string()
        };
        for method in reorder_getters_before_setters(&iface.methods) {
            let key =
                crate::codegen::python::overloads::method_group_key(method, &instance_method_names);
            let overload = InstanceOverload {
                iface_var: format!("_{}", iface.name),
                obj_expr: obj_expr.clone(),
                method,
            };
            if let Some((_, group)) = method_groups
                .iter_mut()
                .find(|(group_key, _)| group_key == &key)
            {
                group.push(overload);
            } else {
                method_groups.push((key, vec![overload]));
            }
        }
    }
    for (_, overloads) in method_groups {
        out.push('\n');
        out.push_str(&generate_instance_method_group(
            &overloads,
            known_types,
            &delegate_names,
        ));
    }

    if matches!(
        collection_kind,
        Some(CollectionKind::Mapping | CollectionKind::MutableMapping)
    ) && let Some(iterable_name) =
        collection_iface.and_then(|iface| map_iterable_name(&iface.generic_args))
    {
        out.push('\n');
        out.push_str("    def _iter_pairs(self):\n");
        out.push_str(&format!(
            "        return iter({}({}))\n",
            py_runtime_symbol(&iterable_name, &iterable_name),
            collection_obj_expr
        ));
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
        out.push_str("        if self._closed:\n            return\n");
        out.push_str(&format!(
            "        {}.from_value(self._obj).close()\n",
            py_runtime_symbol("IClosable", "IClosable")
        ));
        out.push_str("        self._closed = True\n\n");
        out.push_str("    def __enter__(self):\n");
        out.push_str(
            "        if self._closed:\n\
             \x20           raise RuntimeError('cannot enter a closed WinRT object')\n\
             \x20       return self\n\n",
        );
        out.push_str("    def __exit__(self, _exc_type, _exc_value, _traceback):\n");
        out.push_str("        self.close()\n        return False\n");
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
        if let Some(mixin) = interface_kind(req_iface).and_then(runtime_mixin) {
            out.push_str(&format!("\nclass {}({mixin}):\n", req_iface.name));
        } else {
            out.push_str(&format!("\nclass {}:\n", req_iface.name));
        }
        out.push_str("    def __init__(self, obj: DynWinRTValue):\n");
        out.push_str(&format!(
            "        self._obj = obj.cast(IID_{})\n",
            req_iface.name
        ));
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
        if matches!(
            interface_kind(req_iface),
            Some(CollectionKind::Mapping | CollectionKind::MutableMapping)
        ) && let Some(iterable_name) = map_iterable_name(&req_iface.generic_args)
        {
            out.push('\n');
            out.push_str("    def _iter_pairs(self):\n");
            out.push_str(&format!(
                "        return iter({}(self._obj))\n",
                py_runtime_symbol(&iterable_name, &iterable_name)
            ));
        }
    }
    out
}

fn default_constructor_name(has_create_factory: bool) -> &'static str {
    if has_create_factory {
        "create_default"
    } else {
        "create"
    }
}

fn generate_python_constructor(
    class: &ClassMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
    collection_iface: Option<&InterfaceMeta>,
    collection_uses_default: bool,
) -> String {
    let mut out = String::new();
    out.push_str("    def _set_native(self, obj: DynWinRTValue):\n");
    if let Some(default_iface) = &class.default_interface {
        if default_iface.iid.is_empty() {
            out.push_str("        self._obj = obj\n");
        } else {
            out.push_str(&format!(
                "        self._obj = obj.cast(IID_{})\n",
                default_iface.name
            ));
        }
    } else {
        out.push_str("        self._obj = obj\n");
    }
    if let Some(collection_iface) = collection_iface
        && !collection_uses_default
    {
        out.push_str(&format!(
            "        self._collection_obj = obj.cast(IID_{})\n",
            collection_iface.name
        ));
    }
    if class
        .required_interfaces
        .iter()
        .any(|iface| iface.iid == "30d5a829-7fa4-4026-83bb-d75bae4ea99e")
    {
        out.push_str("        self._closed = False\n");
    }
    out.push('\n');
    out.push_str("    @classmethod\n");
    out.push_str("    def _from_native(cls, obj: DynWinRTValue):\n");
    out.push_str("        instance = cls.__new__(cls)\n");
    out.push_str("        instance._set_native(obj)\n");
    out.push_str("        return instance\n\n");
    out.push_str("    def __init__(self, *args, **kwargs):\n");
    out.push_str(
        "        if len(args) == 1 and not kwargs and isinstance(args[0], DynWinRTValue):\n\
         \x20           self._set_native(args[0])\n\
         \x20           return\n",
    );

    let factory_methods = class
        .factory_interfaces
        .iter()
        .flat_map(|iface| iface.methods.iter())
        .collect::<Vec<_>>();
    let factory_names =
        crate::codegen::python::overloads::method_names(factory_methods.iter().copied());
    let has_create_factory = factory_methods.iter().any(|method| {
        let name = to_snake_case(&method.name);
        name == "create" || name.starts_with("create")
    });
    if class.has_default_constructor {
        let constructor_name = default_constructor_name(has_create_factory);
        out.push_str("        _bound = _dynwinrt_bind_overload((), args, kwargs)\n");
        out.push_str("        if _bound is not None:\n");
        out.push_str(&format!(
            "            self._set_native(type(self).{}()._obj)\n            return\n",
            constructor_name
        ));
    }
    for method in factory_methods {
        let in_params = crate::codegen::shared::imports::get_in_params(method);
        let parameter_names = in_params
            .iter()
            .map(|param| format!("'{}'", to_snake_case(&param.name)))
            .collect::<Vec<_>>()
            .join(", ");
        let parameter_names = if parameter_names.is_empty() {
            "()".to_string()
        } else {
            format!("({parameter_names},)")
        };
        let public_name =
            crate::codegen::python::overloads::method_group_key(method, &factory_names);
        let overload_count = factory_methods_for_name(class, &factory_names, &public_name);
        let call_name = if overload_count > 1 {
            format!("_{public_name}_{}", method.vtable_index)
        } else {
            to_snake_case(&method.name)
        };
        out.push_str(&format!(
            "        _bound = _dynwinrt_bind_overload({parameter_names}, args, kwargs)\n"
        ));
        let guards = in_params
            .iter()
            .enumerate()
            .map(|(index, param)| {
                py_method_type_guard(
                    &format!("_bound[{index}]"),
                    &param.typ,
                    known_types,
                    delegate_type_names,
                )
            })
            .collect::<Vec<_>>();
        let condition = if guards.is_empty() {
            "_bound is not None".to_string()
        } else {
            format!("_bound is not None and {}", guards.join(" and "))
        };
        out.push_str(&format!(
            "        if {condition}:\n\
             \x20           self._set_native(type(self).{call_name}(*_bound)._obj)\n\
             \x20           return\n"
        ));
    }
    out.push_str(&format!(
        "        raise TypeError(\"No matching constructor for {}\")\n\n",
        class.name
    ));
    out
}

fn factory_methods_for_name(
    class: &ClassMeta,
    names: &HashSet<String>,
    public_name: &str,
) -> usize {
    class
        .factory_interfaces
        .iter()
        .flat_map(|iface| iface.methods.iter())
        .filter(|method| {
            crate::codegen::python::overloads::method_group_key(method, names) == public_name
        })
        .count()
}
