// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python runtime class generation.

use super::imports::{emit_type_checking_imports, format_py_type_import};
use super::structs::generate_struct_helpers;
use super::*;
use crate::codegen::winrt::extensions::winui::{self, WinUiAbiType};
use crate::codegen::winrt::python::collections::{
    CollectionKind, class_interface, interface_kind, map_iterable_name, runtime_mixin,
};
use crate::meta::{ConstructorKind, ParamMeta};

fn project_winui_abi_types(types: &[WinUiAbiType]) -> String {
    types
        .iter()
        .map(|typ| match typ {
            WinUiAbiType::Object => "DynWinRTType.object()",
        })
        .collect::<Vec<_>>()
        .join(", ")
}

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
    let winui_bootstrap = winui::resolve_application_bootstrap(class, known_types);
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
        let doc = crate::codegen::winrt::shared::docs::DocText {
            summary: class.doc.as_deref(),
            deprecated: class.deprecated.as_deref(),
            ..Default::default()
        };
        out.push_str(&crate::codegen::winrt::python::docs::format_pydoc(
            &doc, "    ",
        ));
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
        crate::codegen::winrt::python::overloads::method_names(static_methods.iter().copied());
    let mut static_groups: Vec<(String, Vec<StaticOverload<'_>>)> = Vec::new();
    for (kind, interfaces) in [
        (StaticOverloadKind::Factory, &class.factory_interfaces),
        (StaticOverloadKind::Static, &class.static_interfaces),
    ] {
        for iface in interfaces {
            for method in &iface.methods {
                let mut key = crate::codegen::winrt::python::overloads::method_group_key(
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

    // Composable factories commonly expose a no-argument `CreateInstance`
    // method. Add an ergonomic `create()` alias when there is no default
    // constructor and no explicit `create` factory of any arity.
    let has_explicit_create_factory = class.factory_interfaces.iter().any(|iface| {
        iface
            .methods
            .iter()
            .any(|m| to_snake_case(&m.name) == "create")
    });
    if !class.has_default_constructor && !has_explicit_create_factory {
        let create_instance_no_args = class.factory_interfaces.iter().any(|iface| {
            iface.methods.iter().any(|m| {
                m.name == "CreateInstance"
                    && crate::codegen::winrt::shared::imports::get_in_params(m).is_empty()
            })
        });
        if create_instance_no_args {
            out.push('\n');
            out.push_str("    @staticmethod\n");
            out.push_str(&format!("    def create() -> '{}':\n", class.name));
            out.push_str(&format!(
                "        \"\"\"Create a new `{}` instance. Alias for `create_instance()`.\"\"\"\n",
                class.name
            ));
            out.push_str(&format!(
                "        return {}.create_instance()\n",
                class.name
            ));
        }
    }

    if let Some(bootstrap) = winui_bootstrap {
        let spec = bootstrap.spec;
        let metadata_provider = spec.metadata_provider.name;
        let controls_resources = spec.controls_resources.name;
        let resource_manager = spec.resource_manager.name;
        let callback_types = project_winui_abi_types(spec.launched_callback_params);
        let metadata_module = to_snake_case_filename(metadata_provider);
        let resources_module = to_snake_case_filename(controls_resources);
        let resource_manager_module = to_snake_case_filename(resource_manager);

        out.push('\n');
        out.push_str("    @staticmethod\n");
        out.push_str(&format!(
            "    def create_with_metadata_provider(metadata_provider: '{metadata_provider}', on_launched: Callable[[], object] | None = None) -> '{}':\n",
            class.name,
        ));
        out.push_str(
            "        \"\"\"Compose a WinUI `Application` that exposes the supplied XAML metadata provider.\n\
             \n\
             `on_launched` is invoked from `Application.OnLaunched`, when XAML resources\n\
             and windows can be initialized.\"\"\"\n",
        );
        out.push_str("        _launched = None\n");
        out.push_str("        if on_launched is not None:\n");
        out.push_str(&format!(
            "            _launched = DynWinRtDelegate.create(WinGUID.parse('{callback_iid}'), [{callback_types}], lambda _args: on_launched()).to_value()\n",
            callback_iid = spec.launched_callback_iid,
        ));
        out.push_str(&format!(
            "        return {}._from_native(DynWinRTValue.create_xaml_application(getattr(metadata_provider, '_obj', metadata_provider), _launched))\n",
            class.name
        ));

        out.push('\n');
        out.push_str("    @staticmethod\n");
        out.push_str(&format!(
            "    def create(on_launched: Callable[[], object] | None = None) -> '{}':\n",
            class.name
        ));
        out.push_str(
            "        \"\"\"Compose a WinUI `Application`, install WinUI's default Fluent resources,\n\
             and optionally configure unpackaged resource resolution before running `on_launched`.\"\"\"\n",
        );
        out.push_str(&format!(
            "        _provider = _dynwinrt_symbol('{metadata_module}', '{metadata_provider}').create()\n",
        ));
        out.push_str("        _resources_initialized = [False]\n");
        out.push_str("        def _on_launched_wrapped(_args):\n");
        out.push_str(&format!(
            "            _app = {}.get_current()\n",
            class.name
        ));
        out.push_str("            if _app is None:\n");
        out.push_str(
            "                raise RuntimeError('WinUI Application.current is unavailable during on_launched')\n",
        );
        out.push_str("            if not _resources_initialized[0]:\n");
        out.push_str(&format!(
            "                _app.resources.merged_dictionaries.append(_dynwinrt_symbol('{resources_module}', '{controls_resources}').create())\n",
        ));
        out.push_str("                _resources_initialized[0] = True\n");
        out.push_str("            if on_launched is not None:\n");
        out.push_str("                on_launched()\n");
        out.push_str(&format!(
            "        _launched = DynWinRtDelegate.create(WinGUID.parse('{callback_iid}'), [{callback_types}], _on_launched_wrapped).to_value()\n",
            callback_iid = spec.launched_callback_iid,
        ));
        out.push_str(&format!(
            "        _app = {}._from_native(DynWinRTValue.create_xaml_application(getattr(_provider, '_obj', _provider), _launched))\n",
            class.name
        ));
        if bootstrap.supports_unpackaged_resources {
            out.push_str(
                "        from dynwinrt_py import has_package_identity, get_winappsdk_resource_pri_path\n",
            );
            out.push_str("        if not has_package_identity():\n");
            out.push_str(&format!(
                "            _resource_manager = _dynwinrt_symbol('{resource_manager_module}', '{resource_manager}').create_instance(get_winappsdk_resource_pri_path())\n",
            ));
            out.push_str("            def _on_resource_manager_requested(_sender, args):\n");
            out.push_str("                if args is None:\n");
            out.push_str(
                "                    raise RuntimeError('WinUI ResourceManagerRequested did not supply event arguments')\n",
            );
            out.push_str("                args.custom_resource_manager = _resource_manager\n");
            out.push_str(
                "            _app.on_resource_manager_requested(_on_resource_manager_requested)\n",
            );
        }
        out.push_str("        return _app\n");
    }

    let mut method_groups: Vec<(String, Vec<InstanceOverload<'_>>)> = Vec::new();
    let instance_ifaces = class
        .default_interface
        .iter()
        .chain(class.required_interfaces.iter())
        .filter(|iface| iface.iid != "30d5a829-7fa4-4026-83bb-d75bae4ea99e")
        .collect::<Vec<_>>();
    let instance_method_names = crate::codegen::winrt::python::overloads::method_names(
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
            let key = crate::codegen::winrt::python::overloads::method_group_key(
                method,
                &instance_method_names,
            );
            let overload = InstanceOverload {
                iface_var: format!("_{}", iface.name),
                obj_expr: obj_expr.clone(),
                method,
                sibling_methods: Some(iface.methods.as_slice()),
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

    // IStringable → __str__ / __repr__ delegates to the runtime IStringable.
    const ISTRINGABLE_IID: &str = "96369f54-8eb6-48f0-abce-c1b211e627c3";
    if class.name != "IStringable"
        && class
            .required_interfaces
            .iter()
            .any(|ri| ri.iid == ISTRINGABLE_IID)
    {
        out.push('\n');
        out.push_str("    def __str__(self) -> str:\n");
        out.push_str(
            "        return _IStringable.method(6).invoke(self._obj.cast(IID_IStringable), []).to_string()\n",
        );
        out.push('\n');
        out.push_str("    def __repr__(self) -> str:\n");
        out.push_str("        return f'{type(self).__name__}({self.__str__()!r})'\n");
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

fn split_composable_params<'a>(
    method: &'a MethodMeta,
    in_params: &[&'a ParamMeta],
) -> Option<(usize, Vec<&'a ParamMeta>)> {
    let outer = *in_params.last()?;
    let outer_name = outer.name.to_ascii_lowercase();
    let is_outer_name = outer_name == "outer"
        || outer_name == "base"
        || outer_name == "baseinterface"
        || outer_name == "outerinterface";
    let has_inner_output = method.params.iter().any(|param| {
        param.direction == ParamDirection::Out
            && matches!(param.typ, TypeMeta::Object)
            && param.name.to_ascii_lowercase().contains("inner")
    });
    if !is_outer_name || !matches!(outer.typ, TypeMeta::Object) || !has_inner_output {
        return None;
    }
    Some((
        in_params.len() - 1,
        in_params[..in_params.len() - 1].to_vec(),
    ))
}

fn method_constructs_class(method: &MethodMeta, class: &ClassMeta) -> bool {
    matches!(
        method.return_type.as_ref(),
        Some(TypeMeta::RuntimeClass { namespace, name, .. })
            if namespace == &class.namespace && name == &class.name
    )
}

/// A constructor candidate for `__init__` dispatch: public params + call expression.
struct PyCtorCandidate<'a> {
    /// Only the public (outer-stripped) input params, in call order.
    public_params: Vec<&'a ParamMeta>,
    /// Full call expression, e.g. `type(self).create_instance(_bound[0], None)`.
    call_expr: String,
}

fn build_ctor_candidates<'a>(
    class: &'a ClassMeta,
    factory_names: &HashSet<String>,
) -> Vec<PyCtorCandidate<'a>> {
    fn push_unique<'a>(candidates: &mut Vec<PyCtorCandidate<'a>>, candidate: PyCtorCandidate<'a>) {
        if candidates.iter().any(|existing| {
            existing.public_params.len() == candidate.public_params.len()
                && existing
                    .public_params
                    .iter()
                    .zip(&candidate.public_params)
                    .all(|(left, right)| left.name == right.name && left.typ == right.typ)
        }) {
            return;
        }
        candidates.push(candidate);
    }

    let mut candidates: Vec<PyCtorCandidate<'a>> = Vec::new();
    let has_create_factory = class.factory_interfaces.iter().any(|iface| {
        iface.methods.iter().any(|m| {
            let snake = to_snake_case(&m.name);
            snake == "create" || snake.starts_with("create")
        })
    });

    for constructor in &class.constructors {
        match constructor.kind {
            ConstructorKind::DefaultActivation => {
                let ctor_name = default_constructor_name(has_create_factory);
                push_unique(
                    &mut candidates,
                    PyCtorCandidate {
                        public_params: Vec::new(),
                        call_expr: format!("type(self).{}()", ctor_name),
                    },
                );
            }
            ConstructorKind::FactoryActivation => {
                let Some(factory_ref) = constructor.factory_interface.as_ref() else {
                    continue;
                };
                let Some(factory) = class.factory_interfaces.iter().find(|iface| {
                    iface.namespace == factory_ref.namespace && iface.name == factory_ref.name
                }) else {
                    continue;
                };
                for method in &factory.methods {
                    if !method_constructs_class(method, class) {
                        continue;
                    }
                    let in_params = crate::codegen::winrt::shared::imports::get_in_params(method);
                    let call_expr =
                        build_factory_call_expr(class, method, &in_params, None, factory_names);
                    push_unique(
                        &mut candidates,
                        PyCtorCandidate {
                            public_params: in_params,
                            call_expr,
                        },
                    );
                }
            }
            ConstructorKind::PublicComposition => {
                let Some(factory_ref) = constructor.factory_interface.as_ref() else {
                    continue;
                };
                let Some(factory) = class.factory_interfaces.iter().find(|iface| {
                    iface.namespace == factory_ref.namespace && iface.name == factory_ref.name
                }) else {
                    continue;
                };
                for method in &factory.methods {
                    if !method_constructs_class(method, class) {
                        continue;
                    }
                    let in_params = crate::codegen::winrt::shared::imports::get_in_params(method);
                    let Some((outer_index, public_params)) =
                        split_composable_params(method, &in_params)
                    else {
                        continue;
                    };
                    let call_expr = build_factory_call_expr(
                        class,
                        method,
                        &in_params,
                        Some(outer_index),
                        factory_names,
                    );
                    push_unique(
                        &mut candidates,
                        PyCtorCandidate {
                            public_params,
                            call_expr,
                        },
                    );
                }
            }
            ConstructorKind::ProtectedComposition => {}
        }
    }

    candidates
}

/// Build a `type(self).<method>(_bound[0], _bound[1], ..., None_for_outer)` call.
fn build_factory_call_expr(
    class: &ClassMeta,
    method: &MethodMeta,
    in_params: &[&ParamMeta],
    outer_index: Option<usize>,
    factory_names: &HashSet<String>,
) -> String {
    let public_name =
        crate::codegen::winrt::python::overloads::method_group_key(method, factory_names);
    let overload_count = factory_methods_for_name(class, factory_names, &public_name);
    let call_name = if overload_count > 1 {
        format!("_{public_name}_{}", method.vtable_index)
    } else {
        to_snake_case(&method.name)
    };
    let mut public_idx = 0usize;
    let args = in_params
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if outer_index == Some(index) {
                "DynWinRTValue.null_value()".to_string()
            } else {
                let arg = format!("_bound[{public_idx}]");
                public_idx += 1;
                arg
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("type(self).{call_name}({args})")
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
        crate::codegen::winrt::python::overloads::method_names(factory_methods.iter().copied());

    // Prefer the new constructors-metadata-driven path when the class carries
    // activation/composable attributes. Fall back to the legacy factory-scan
    // path when meta didn't record constructors (should be rare).
    let candidates = build_ctor_candidates(class, &factory_names);
    if !class.constructors.is_empty() {
        for candidate in &candidates {
            let parameter_names = candidate
                .public_params
                .iter()
                .map(|param| format!("'{}'", to_snake_case(&param.name)))
                .collect::<Vec<_>>()
                .join(", ");
            let parameter_names = if parameter_names.is_empty() {
                "()".to_string()
            } else {
                format!("({parameter_names},)")
            };
            out.push_str(&format!(
                "        _bound = _dynwinrt_bind_overload({parameter_names}, args, kwargs)\n"
            ));
            let guards = candidate
                .public_params
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
                 \x20           self._set_native({call}._obj)\n\
                 \x20           return\n",
                condition = condition,
                call = candidate.call_expr,
            ));
        }
    } else {
        // Legacy fallback (no constructors metadata): treat every factory
        // method as an activation candidate.
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
            let in_params = crate::codegen::winrt::shared::imports::get_in_params(method);
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
                crate::codegen::winrt::python::overloads::method_group_key(method, &factory_names);
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
            crate::codegen::winrt::python::overloads::method_group_key(method, names) == public_name
        })
        .count()
}
