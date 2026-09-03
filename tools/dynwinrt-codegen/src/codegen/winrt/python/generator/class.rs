// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python runtime class generation.

use super::imports::{emit_type_checking_imports, format_py_type_import};
use super::structs::{generate_struct_helpers, generate_struct_imports};
use super::*;
use crate::codegen::winrt::extensions::winui::{self, WinUiAbiType};
use crate::codegen::winrt::python::collections::{
    CollectionKind, class_interface, interface_kind, map_iterable_identity, runtime_mixin,
};
use crate::meta::{ConstructorKind, ParamMeta};
use crate::types::{TypeIdentity, TypeIdentityKind};

fn project_winui_abi_types(types: &[WinUiAbiType]) -> String {
    types
        .iter()
        .map(|typ| match typ {
            WinUiAbiType::Object => "DynWinRTType.object()",
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn interface_symbol(context: &PythonProjectionContext, interface: &InterfaceMeta) -> String {
    context.reference_name(&interface.type_identity())
}

/// Generate a Python file for a single RuntimeClass.
pub fn generate_class(
    context: &PythonProjectionContext,
    class: &ClassMeta,
    shared_iids: &HashSet<String>,
) -> String {
    let used_structs = collect_used_structs_from_class(class);
    let collection_iface = class_interface(class);
    let collection_kind = collection_iface.and_then(interface_kind);
    let known_full_names = context.known_full_names();
    let winui_bootstrap = winui::resolve_application_bootstrap(class, &known_full_names);
    let has_public_composition = class
        .constructors
        .iter()
        .any(|constructor| constructor.kind == ConstructorKind::PublicComposition);
    let collection_uses_default = collection_iface.is_some_and(|collection_iface| {
        class
            .default_interface
            .as_ref()
            .is_some_and(|default_iface| {
                default_iface.type_identity() == collection_iface.type_identity()
            })
    });
    let collection_obj_expr = if collection_uses_default {
        "self._obj"
    } else {
        "self._collection_obj"
    };
    let projectable = super::super::has_projectable_default_interface(class);
    let native_projectable = super::super::has_native_projector(class);
    let mut out = String::new();

    // Header
    out.push_str(HEADER);
    out.push_str(FUTURE_ANNOTATIONS);
    out.push_str(IMPORT_LINE);
    if has_public_composition {
        out.push_str(
            "from dynwinrt import register_xaml_runtime_class as _dynwinrt_register_xaml_runtime_class\n",
        );
    }
    if winui::is_dispatcher_queue(class) {
        out.push_str("import asyncio\n");
    }
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
            "from dynwinrt.dynwinrt import {}\n",
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
    if context.is_packaged() {
        out.push_str(&generate_struct_imports(context, &used_structs));
    }
    if has_ireference_input(
        class
            .all_interfaces()
            .flat_map(|interface| interface.methods.iter()),
    ) || has_ireference_struct_field(&used_structs)
    {
        out.push_str(IREFERENCE_HELPER);
    }
    out.push('\n');
    let mut type_checking_imports = Vec::new();

    // Collect delegate names from all interfaces of this class
    let mut delegate_names = HashSet::new();
    let mut runtime_delegate_names = HashSet::new();
    let all_ifaces: Vec<&InterfaceMeta> = class.all_interfaces().collect();
    for iface in &all_ifaces {
        delegate_names.extend(super::super::collect_referenced_delegate_names(
            &iface.methods,
            context,
        ));
        runtime_delegate_names.extend(super::super::collect_runtime_delegate_names(
            &iface.methods,
            context,
        ));
    }

    // Collection generics import (skip delegates)
    let mut imported_names: HashSet<String> = HashSet::new();
    let collection_identities = collect_used_generic_identities_from_class(class);
    for identity in &collection_identities {
        let identity = context.normalize_identity(identity);
        if !delegate_names.contains(&identity) {
            let module = context.implementation_module(&identity);
            let name = context.projected_name(&identity);
            let reference_name = context.reference_name(&identity);
            let import = if name == reference_name {
                name
            } else {
                format!("{name} as {reference_name}")
            };
            type_checking_imports.push(format!("from .{module} import {import}  # noqa: F401\n"));
            imported_names.insert(reference_name);
        }
    }
    for iface in class.all_interfaces() {
        if iface.generic_piid.as_deref()
            == Some(crate::codegen::winrt::python::collections::IOBSERVABLE_VECTOR_PIID)
        {
            let identity = iface.type_identity();
            let reference_name = context.reference_name(&identity);
            if imported_names.insert(reference_name) {
                type_checking_imports.push(format_py_type_import(
                    context,
                    &iface.namespace,
                    &iface.name,
                    crate::types::TypeKind::Interface,
                ));
            }
            let event_args = "IVectorChangedEventArgs";
            if imported_names.insert(event_args.into()) {
                let identity = TypeIdentity::named(
                    TypeIdentityKind::Interface,
                    crate::meta::WINDOWS_FOUNDATION_COLLECTIONS_NAMESPACE,
                    event_args,
                );
                type_checking_imports.push(format!(
                    "from .{} import {event_args}  # noqa: F401\n",
                    context.implementation_module(&identity)
                ));
            }
        }
    }

    // Import delegate IID + PARAM_TYPES
    let mut sorted_delegates: Vec<_> = runtime_delegate_names.iter().collect();
    sorted_delegates.sort();
    for identity in &sorted_delegates {
        let module = context.implementation_module(identity);
        let dname = context.projected_name(identity);
        let reference_name = context.reference_name(identity);
        let imports = if dname == reference_name {
            format!("IID_{dname}, {dname}_PARAM_TYPES")
        } else {
            format!(
                "IID_{dname} as IID_{reference_name}, \
                 {dname}_PARAM_TYPES as {reference_name}_PARAM_TYPES"
            )
        };
        type_checking_imports.push(format!("from .{module} import {imports}  # noqa: F401\n",));
    }

    // Type imports
    let imports = collect_type_imports(class);
    let mut sorted_imports: Vec<_> = imports.iter().collect();
    sorted_imports
        .sort_by(|a, b| (&a.namespace, &a.name, &a.kind).cmp(&(&b.namespace, &b.name, &b.kind)));
    for r in &sorted_imports {
        let identity_kind = match r.kind {
            TypeKind::Class => TypeIdentityKind::Class,
            TypeKind::Enum => TypeIdentityKind::Enum,
            TypeKind::Interface => TypeIdentityKind::Interface,
        };
        let identity = context.normalize_identity(&TypeIdentity::named(
            identity_kind,
            r.namespace.clone(),
            r.name.clone(),
        ));
        if context.is_known_ref(r) && !delegate_names.contains(&identity) {
            type_checking_imports.push(format_py_type_import(
                context,
                &r.namespace,
                &r.name,
                r.kind,
            ));
            let reference_name = context.reference_name(&identity);
            imported_names.insert(reference_name.clone());
            if r.kind == TypeKind::Interface {
                imported_names.insert(format!("IID_{reference_name}"));
            }
        }
    }

    // Import shared required interfaces
    for req_iface in &class.required_interfaces {
        let symbol = interface_symbol(context, req_iface);
        if req_iface.generic_piid.is_none()
            && !req_iface.iid.is_empty()
            && shared_iids.contains(&req_iface.iid)
            && !imported_names.contains(&symbol)
        {
            type_checking_imports.push(format_py_type_import(
                context,
                &req_iface.namespace,
                &req_iface.name,
                TypeKind::Interface,
            ));
            imported_names.insert(symbol.clone());
            imported_names.insert(format!("IID_{symbol}"));
        }
    }
    emit_type_checking_imports(&mut out, type_checking_imports);

    // IID constants are emitted locally even when the interface type is shared.
    // TYPE_CHECKING imports do not define runtime values, and interface
    // registration happens while this module is loading.
    let mut declared_iids = HashSet::new();
    let all_class_ifaces: Vec<&InterfaceMeta> = class
        .all_interfaces()
        .chain(class.overridable_interfaces.iter())
        .collect();
    for iface in &all_class_ifaces {
        let iid_name = format!("IID_{}", interface_symbol(context, iface));
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
        let symbol = interface_symbol(context, iface);
        out.push_str(&py_generate_interface_registration(
            iface,
            &format!("_{symbol}"),
            &symbol,
        ));
        out.push('\n');
    }
    for iface in &class.factory_interfaces {
        let symbol = interface_symbol(context, iface);
        out.push_str(&py_generate_interface_registration(
            iface,
            &format!("_{symbol}"),
            &symbol,
        ));
        out.push('\n');
    }
    for iface in &class.static_interfaces {
        let symbol = interface_symbol(context, iface);
        out.push_str(&py_generate_interface_registration(
            iface,
            &format!("_{symbol}"),
            &symbol,
        ));
        out.push('\n');
    }
    for iface in &class.required_interfaces {
        let symbol = interface_symbol(context, iface);
        out.push_str(&py_generate_interface_registration(
            iface,
            &format!("_{symbol}"),
            &symbol,
        ));
        out.push('\n');
    }
    // IActivationFactory for default constructor
    if class.has_default_activation() {
        out.push_str("_IActivationFactory = DynWinRTType.register_interface(\n");
        out.push_str(
            "    'IActivationFactory', WinGUID.parse('00000035-0000-0000-c000-000000000046')) \\\n",
        );
        out.push_str("    .add_method('ActivateInstance', DynWinRTMethodSig().add_out(DynWinRTType.object()))\n");
        out.push('\n');
    }

    // Struct helpers
    if !context.is_packaged() {
        for s in &used_structs {
            out.push_str(&generate_struct_helpers(context, s));
            out.push('\n');
        }
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
    if projectable {
        out.push_str("    _dynwinrt_runtime_class_type = True\n");
    } else if native_projectable {
        out.push_str("    _dynwinrt_projectable_class_type = True\n");
    }

    out.push_str(&generate_python_constructor(
        context,
        class,
        collection_iface,
        collection_uses_default,
    ));

    if crate::codegen::winrt::is_buffer_class(&class.namespace, &class.name) {
        out.push_str(
            "    @staticmethod\n\
             \x20   def from_bytes(data: bytes | bytearray) -> 'Buffer':\n\
             \x20       \"\"\"Create an owned IBuffer by copying bytes or bytearray data.\"\"\"\n\
             \x20       return Buffer._from_native(DynWinRTValue.from_bytes(data))\n\n\
             \x20   def to_bytes(self) -> bytes:\n\
             \x20       \"\"\"Copy the initialized IBuffer data into a new bytes object.\"\"\"\n\
             \x20       return self._obj.to_bytes()\n\n",
        );
    }

    // Resolve activation factories per call. Keeping COM factories in Python
    // class variables lets them outlive the thread's RoApartment and can crash
    // during interpreter shutdown when WinUI releases them after RoUninitialize.
    let mut declared: HashSet<String> = HashSet::new();
    for iface in &class.factory_interfaces {
        let symbol = interface_symbol(context, iface);
        let key = format!("f_{symbol}");
        if !iface.iid.is_empty() && declared.insert(key.clone()) {
            out.push_str("    @staticmethod\n");
            out.push_str(&format!("    def _get_{}():\n", key));
            out.push_str(&format!(
                "        return DynWinRTValue.activation_factory('{full}').cast(IID_{iface})\n",
                iface = symbol,
                full = class.full_name,
            ));
            out.push('\n');
        }
    }
    for iface in &class.static_interfaces {
        let symbol = interface_symbol(context, iface);
        let key = format!("s_{symbol}");
        if !iface.iid.is_empty() && declared.insert(key.clone()) {
            out.push_str("    @staticmethod\n");
            out.push_str(&format!("    def _get_{}():\n", key));
            out.push_str(&format!(
                "        return DynWinRTValue.activation_factory('{full}').cast(IID_{iface})\n",
                iface = symbol,
                full = class.full_name,
            ));
            out.push('\n');
        }
    }

    // Default constructor
    if class.has_default_activation() {
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
                    key = format!("{}#{key}", interface_symbol(context, iface));
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
        out.push_str(&generate_static_method_group(&overloads, context));
    }
    let static_aliases = generate_compatibility_aliases(static_methods.iter().copied());
    if !static_aliases.is_empty() {
        out.push('\n');
        out.push_str(&static_aliases);
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
    if !class.has_default_activation() && !has_explicit_create_factory {
        let create_instance_no_args = class.factory_interfaces.iter().any(|iface| {
            class.is_public_constructor_factory(iface)
                && iface.methods.iter().any(|method| {
                    method.name == "CreateInstance"
                        && method_constructs_class(method, class)
                        && crate::codegen::winrt::shared::imports::get_in_params(method).is_empty()
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
        let metadata_provider = spec.metadata_provider;
        let controls_resources = spec.controls_resources;
        let resource_manager = spec.resource_manager;
        let callback_types = project_winui_abi_types(spec.launched_callback_params);
        let metadata_module = context.implementation_module_for_named(
            TypeIdentityKind::Class,
            metadata_provider.namespace,
            metadata_provider.name,
        );
        let resources_module = context.implementation_module_for_named(
            TypeIdentityKind::Class,
            controls_resources.namespace,
            controls_resources.name,
        );
        let resource_manager_module = context.implementation_module_for_named(
            TypeIdentityKind::Class,
            resource_manager.namespace,
            resource_manager.name,
        );
        let metadata_provider = metadata_provider.name;
        let controls_resources = controls_resources.name;
        let resource_manager = resource_manager.name;

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
            "            _launched = _dynwinrt_create_delegate(WinGUID.parse('{callback_iid}'), [{callback_types}], lambda _args: on_launched()).to_value()\n",
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
            "        _launched = _dynwinrt_create_delegate(WinGUID.parse('{callback_iid}'), [{callback_types}], _on_launched_wrapped).to_value()\n",
            callback_iid = spec.launched_callback_iid,
        ));
        out.push_str(&format!(
            "        _app = {}._from_native(DynWinRTValue.create_xaml_application(getattr(_provider, '_obj', _provider), _launched))\n",
            class.name
        ));
        if bootstrap.supports_unpackaged_resources {
            out.push_str(
                "        from dynwinrt import has_package_identity, get_winappsdk_resource_pri_path\n",
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
    let property_getters = instance_ifaces
        .iter()
        .flat_map(|iface| iface.methods.iter())
        .filter(|method| method.is_property_getter)
        .filter_map(|method| method.name.strip_prefix("get_"))
        .map(str::to_string)
        .collect::<HashSet<_>>();
    // Python evaluates decorators while building the class. Emit every getter
    // before any cross-interface setter that references it.
    for setter_phase in [false, true] {
        for iface in &instance_ifaces {
            let obj_expr = if collection_iface
                .is_some_and(|collection| collection.type_identity() == iface.type_identity())
            {
                collection_obj_expr
            } else if class
                .default_interface
                .as_ref()
                .is_some_and(|default_iface| default_iface.type_identity() == iface.type_identity())
            {
                "self._obj"
            } else {
                ""
            };
            let iface_symbol = interface_symbol(context, iface);
            let obj_expr = if obj_expr.is_empty() {
                format!("self._obj.cast(IID_{iface_symbol})")
            } else {
                obj_expr.to_string()
            };
            for method in reorder_getters_before_setters(&iface.methods)
                .into_iter()
                .filter(|method| method.is_property_setter == setter_phase)
            {
                let key = crate::codegen::winrt::python::overloads::method_group_key(
                    method,
                    &instance_method_names,
                );
                let overload = InstanceOverload {
                    iface_var: format!("_{iface_symbol}"),
                    obj_expr: obj_expr.clone(),
                    method,
                    sibling_methods: Some(iface.methods.as_slice()),
                    property_has_getter: !method.is_property_setter
                        || method
                            .name
                            .strip_prefix("put_")
                            .is_some_and(|suffix| property_getters.contains(suffix)),
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
    }
    for (_, overloads) in method_groups {
        out.push('\n');
        out.push_str(&generate_instance_method_group(&overloads, context));
    }
    let instance_aliases = generate_compatibility_aliases(
        instance_ifaces
            .iter()
            .flat_map(|iface| iface.methods.iter()),
    );
    if !instance_aliases.is_empty() {
        out.push('\n');
        out.push_str(&instance_aliases);
    }

    if matches!(
        collection_kind,
        Some(CollectionKind::Mapping | CollectionKind::MutableMapping)
    ) && let Some(iterable_identity) =
        collection_iface.and_then(|iface| map_iterable_identity(&iface.generic_args))
    {
        let iterable_name = context.projected_name(&iterable_identity);
        out.push('\n');
        out.push_str("    def _iter_pairs(self):\n");
        out.push_str(&format!(
            "        return iter({}({}))\n",
            py_runtime_symbol(context, &iterable_identity, &iterable_name),
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
            py_runtime_named_symbol(
                context,
                TypeIdentityKind::Interface,
                "Windows.Foundation",
                "IClosable",
                "IClosable",
            )
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

    // .as_interface() method for explicit, IID-checked interface projection.
    if super::super::has_projectable_default_interface(class)
        || !class.required_interfaces.is_empty()
    {
        out.push('\n');
        out.push_str("    def as_interface(self, interface_class):\n");
        out.push_str("        return interface_class.from_value(self._obj)\n");
    }

    if winui::is_dispatcher_queue(class) {
        out.push_str(
            r#"

    async def enqueue_async(self, callback, *args, **kwargs):
        loop = asyncio.get_running_loop()
        future = loop.create_future()

        def complete(result=None, error=None):
            if future.done():
                return
            if error is None:
                future.set_result(result)
            else:
                future.set_exception(error)

        def post_complete(result=None, error=None):
            try:
                loop.call_soon_threadsafe(complete, result, error)
            except RuntimeError:
                pass

        def invoke():
            try:
                result = callback(*args, **kwargs)
            except BaseException as error:
                post_complete(None, error)
            else:
                post_complete(result, None)

        if not self.try_enqueue(invoke):
            raise RuntimeError('DispatcherQueue rejected the callback.')
        return await future

    async def enqueue_with_priority_async(self, priority, callback, *args, **kwargs):
        loop = asyncio.get_running_loop()
        future = loop.create_future()

        def complete(result=None, error=None):
            if future.done():
                return
            if error is None:
                future.set_result(result)
            else:
                future.set_exception(error)

        def post_complete(result=None, error=None):
            try:
                loop.call_soon_threadsafe(complete, result, error)
            except RuntimeError:
                pass

        def invoke():
            try:
                result = callback(*args, **kwargs)
            except BaseException as error:
                post_complete(None, error)
            else:
                post_complete(result, None)

        if not self.try_enqueue_with_priority(priority, invoke):
            raise RuntimeError('DispatcherQueue rejected the callback.')
        return await future
"#,
        );
    }

    // Generate inline wrapper classes for required interfaces (non-default)
    for req_iface in &class.required_interfaces {
        if req_iface.iid.is_empty() {
            continue;
        }
        let symbol = interface_symbol(context, req_iface);
        if imported_names.contains(&symbol) {
            continue;
        }
        let reg_var = format!("_{symbol}");
        out.push('\n');
        if let Some(mixin) = interface_kind(req_iface).and_then(runtime_mixin) {
            out.push_str(&format!("\nclass {symbol}({mixin}):\n"));
        } else {
            out.push_str(&format!("\nclass {symbol}:\n"));
        }
        out.push_str("    _dynwinrt_interface_type = True\n");
        out.push_str(&format!("    _dynwinrt_interface_iid = IID_{symbol}\n"));
        out.push_str("    def __new__(cls, *args, **kwargs):\n");
        out.push_str(
            "        if len(args) == 1 and not kwargs and isinstance(args[0], DynWinRTValue):\n\
             \x20           return _dynwinrt_projected_from_native(cls, args[0], '_set_native')\n\
             \x20       return super().__new__(cls)\n\n",
        );
        out.push_str("    def _set_native(self, obj: DynWinRTValue):\n");
        out.push_str(&format!("        self._obj = obj.cast(IID_{symbol})\n"));
        out.push_str("        self._dynwinrt_native_ready = True\n");
        out.push_str(&format!(
            "        _dynwinrt_track_projected(self, '{}.{}')\n",
            req_iface.namespace, req_iface.name
        ));
        out.push_str("        _dynwinrt_cache_projected(self)\n");
        out.push('\n');
        out.push_str("    def __init__(self, obj: DynWinRTValue):\n");
        out.push_str(
            "        if getattr(self, '_dynwinrt_native_ready', False):\n\
             \x20           return\n",
        );
        out.push_str(&format!("        {symbol}._set_native(self, obj)\n"));
        out.push('\n');
        out.push_str("    @classmethod\n");
        out.push_str(&format!(
            "    def _from_native(cls, obj: DynWinRTValue) -> '{}':\n",
            symbol
        ));
        out.push_str("        return cls(obj)\n");
        out.push('\n');
        out.push_str("    @classmethod\n");
        out.push_str(&format!(
            "    def from_value(cls, obj: DynWinRTValue) -> '{}':\n",
            symbol
        ));
        out.push_str(&format!(
            "        return cls._from_native(obj.cast(IID_{symbol}))\n"
        ));
        out.push('\n');
        out.push_str("    def as_interface(self, interface_class):\n");
        out.push_str("        return interface_class.from_value(self._obj)\n");
        for methods in crate::codegen::winrt::python::overloads::grouped_methods(
            reorder_getters_before_setters(&req_iface.methods),
        ) {
            out.push('\n');
            let overloads = methods
                .into_iter()
                .map(|method| InstanceOverload {
                    iface_var: reg_var.clone(),
                    obj_expr: "self._obj".into(),
                    method,
                    sibling_methods: Some(req_iface.methods.as_slice()),
                    property_has_getter: !method.is_property_setter
                        || method.name.strip_prefix("put_").is_some_and(|suffix| {
                            req_iface
                                .methods
                                .iter()
                                .any(|candidate| candidate.name == format!("get_{suffix}"))
                        }),
                })
                .collect::<Vec<_>>();
            out.push_str(&generate_instance_method_group(&overloads, context));
        }
        let aliases = generate_compatibility_aliases(req_iface.methods.iter());
        if !aliases.is_empty() {
            out.push('\n');
            out.push_str(&aliases);
        }
        if matches!(
            interface_kind(req_iface),
            Some(CollectionKind::Mapping | CollectionKind::MutableMapping)
        ) && let Some(iterable_identity) = map_iterable_identity(&req_iface.generic_args)
        {
            let iterable_name = context.projected_name(&iterable_identity);
            out.push('\n');
            out.push_str("    def _iter_pairs(self):\n");
            out.push_str(&format!(
                "        return iter({}(self._obj))\n",
                py_runtime_symbol(context, &iterable_identity, &iterable_name)
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
    /// Aggregated call for Python subclasses. `None` means subclass activation
    /// is not semantically available for this constructor shape.
    composed_call_expr: Option<String>,
}

fn build_ctor_candidates<'a>(
    context: &PythonProjectionContext,
    class: &'a ClassMeta,
    factory_names: &HashSet<String>,
) -> Vec<PyCtorCandidate<'a>> {
    fn push_unique<'a>(candidates: &mut Vec<PyCtorCandidate<'a>>, candidate: PyCtorCandidate<'a>) {
        if let Some(existing) = candidates.iter_mut().find(|existing| {
            existing.public_params.len() == candidate.public_params.len()
                && existing
                    .public_params
                    .iter()
                    .zip(&candidate.public_params)
                    .all(|(left, right)| left.name == right.name && left.typ == right.typ)
        }) {
            if existing.composed_call_expr.is_none() {
                existing.composed_call_expr = candidate.composed_call_expr;
            }
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
                        composed_call_expr: None,
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
                            composed_call_expr: None,
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
                    let inner_output_index = method
                        .params
                        .iter()
                        .filter(|param| param.direction != ParamDirection::In)
                        .position(|param| param.name.to_ascii_lowercase().contains("inner"))
                        .expect("split_composable_params verified the inner output");
                    let instance_output_index = method
                        .params
                        .iter()
                        .filter(|param| param.direction != ParamDirection::In)
                        .count();
                    let wrapped_args = public_params
                        .iter()
                        .enumerate()
                        .map(|(index, param)| {
                            crate::codegen::winrt::python::method::py_wrap_method_arg(
                                &format!("_bound[{index}]"),
                                &param.typ,
                                context,
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let composed_call_expr = format!(
                        "_{factory}.method({vtable}).invoke_composed_with_overrides({class}._get_f_{factory}(), [{wrapped_args}], {outer_index}, {inner_output_index}, {instance_output_index}, {agile}, _override_interfaces)",
                        factory = interface_symbol(context, factory),
                        class = class.name,
                        vtable = method.vtable_index,
                        agile = if class.is_agile { "True" } else { "False" },
                    );
                    push_unique(
                        &mut candidates,
                        PyCtorCandidate {
                            public_params,
                            call_expr,
                            composed_call_expr: Some(composed_call_expr),
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
    let mut overloads = class
        .factory_interfaces
        .iter()
        .flat_map(|interface| interface.methods.iter())
        .chain(
            class
                .static_interfaces
                .iter()
                .flat_map(|interface| interface.methods.iter()),
        )
        .filter(|candidate| {
            crate::codegen::winrt::python::overloads::method_group_key(candidate, factory_names)
                == public_name
        })
        .collect::<Vec<_>>();
    let call_name = if overloads.len() > 1 {
        overloads.sort_by(|left, right| {
            crate::codegen::winrt::python::overloads::cmp_python_dispatch_methods(left, right)
        });
        let private_names = crate::codegen::winrt::python::method::private_overload_names(
            &public_name,
            overloads.iter().copied(),
        );
        let index = overloads
            .iter()
            .position(|candidate| std::ptr::eq(*candidate, method))
            .expect("constructor method must be present in its static overload group");
        private_names[index].clone()
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

fn is_size_f32(typ: &TypeMeta) -> bool {
    matches!(
        typ,
        TypeMeta::Struct {
            namespace,
            name,
            fields,
        } if namespace == "Windows.Foundation"
            && name == "Size"
            && fields.len() == 2
            && fields.iter().all(|field| field.typ == TypeMeta::F32)
    )
}

fn override_abi_shape(method: &MethodMeta) -> Option<&'static str> {
    let inputs = method
        .params
        .iter()
        .filter(|param| param.direction == ParamDirection::In)
        .collect::<Vec<_>>();
    if method
        .params
        .iter()
        .any(|param| param.direction != ParamDirection::In)
    {
        return None;
    }
    match (inputs.as_slice(), method.return_type.as_ref()) {
        ([], None) => Some("void0"),
        ([input], Some(output)) if is_size_f32(&input.typ) && is_size_f32(output) => {
            Some("size_f32_to_size_f32")
        }
        ([first, second], Some(TypeMeta::Bool))
            if first.typ == TypeMeta::String && second.typ == TypeMeta::Bool =>
        {
            Some("hstring_bool_to_bool")
        }
        _ => None,
    }
}

fn python_tuple(values: &[String]) -> String {
    if values.is_empty() {
        "()".to_string()
    } else {
        format!(
            "({},)",
            values
                .iter()
                .map(|value| format!("'{value}'"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn generate_python_constructor(
    context: &PythonProjectionContext,
    class: &ClassMeta,
    collection_iface: Option<&InterfaceMeta>,
    collection_uses_default: bool,
) -> String {
    let mut out = String::new();
    let native_projectable = super::super::has_native_projector(class);
    let has_public_composition = class
        .constructors
        .iter()
        .any(|constructor| constructor.kind == ConstructorKind::PublicComposition);
    let native_override_names = if has_public_composition {
        let mut names = class
            .overridable_interfaces
            .iter()
            .flat_map(|interface| interface.methods.iter())
            .map(|method| to_snake_case(&method.name))
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        Some(python_tuple(&names))
    } else {
        None
    };
    let supported_override_interfaces = class
        .overridable_interfaces
        .iter()
        .filter_map(|interface| {
            let shapes = interface
                .methods
                .iter()
                .map(override_abi_shape)
                .collect::<Option<Vec<_>>>()?;
            (shapes.len() <= 8).then_some((interface, shapes))
        })
        .collect::<Vec<_>>();
    let mut supported_override_names = supported_override_interfaces
        .iter()
        .flat_map(|(interface, shapes)| {
            interface
                .methods
                .iter()
                .zip(shapes)
                .filter(|(_, shape)| matches!(**shape, "void0" | "size_f32_to_size_f32"))
                .map(|(method, _)| to_snake_case(&method.name))
        })
        .collect::<Vec<_>>();
    supported_override_names.sort();
    supported_override_names.dedup();
    let supported_override_names_expr = python_tuple(&supported_override_names);
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
    let factory_names =
        crate::codegen::winrt::python::overloads::method_names(static_methods.iter().copied());
    let mut candidates = build_ctor_candidates(context, class, &factory_names);
    candidates.sort_by(|left, right| {
        crate::codegen::winrt::python::overloads::cmp_python_dispatch_params(
            &left.public_params,
            &right.public_params,
        )
        .then_with(|| left.call_expr.cmp(&right.call_expr))
    });

    out.push_str("    def __new__(cls, *args, **kwargs):\n");
    if native_projectable {
        out.push_str(
            "        if len(args) == 1 and not kwargs and isinstance(args[0], DynWinRTValue):\n\
             \x20           return _dynwinrt_projected_from_native(cls, args[0], '_set_native')\n",
        );
    }
    if !candidates.is_empty() {
        out.push_str(&format!("        if cls is {}:\n", class.name));
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
                "            _bound = _dynwinrt_bind_overload({parameter_names}, args, kwargs)\n"
            ));
            let guards = candidate
                .public_params
                .iter()
                .enumerate()
                .map(|(index, param)| {
                    py_method_type_guard(&format!("_bound[{index}]"), &param.typ, context)
                })
                .collect::<Vec<_>>();
            let condition = if guards.is_empty() {
                "_bound is not None".to_string()
            } else {
                format!("_bound is not None and {}", guards.join(" and "))
            };
            let call_expr = candidate.call_expr.replace("type(self)", "cls");
            out.push_str(&format!(
                "            if {condition}:\n                return {call_expr}\n"
            ));
        }
    }
    out.push_str("        return super().__new__(cls)\n\n");

    if has_public_composition {
        out.push_str(
            "    def _set_native(self, obj: DynWinRTValue, _allow_native_overrides=False):\n",
        );
    } else {
        out.push_str("    def _set_native(self, obj: DynWinRTValue):\n");
    }
    if let Some(native_override_names) = &native_override_names {
        out.push_str(&format!("        if type(self) is not {}:\n", class.name));
        out.push_str(&format!(
            "            _native_members = set().union(*(set(_type.__dict__) for _type in type(self).__mro__ if _type is not {}))\n",
            class.name
        ));
        out.push_str(&format!(
            "            _native_overrides = sorted(_native_members.intersection({native_override_names}))\n"
        ));
        out.push_str("            if _native_overrides:\n");
        out.push_str(&format!(
            "                if _allow_native_overrides:\n                    pass\n                else:\n                    raise TypeError(\"{} native overrides require public composable construction: \" + \", \".join(_native_overrides))\n",
            class.name
        ));
    }
    if let Some(default_iface) = &class.default_interface {
        if default_iface.iid.is_empty() {
            out.push_str("        self._obj = obj\n");
        } else {
            let symbol = interface_symbol(context, default_iface);
            out.push_str(&format!("        self._obj = obj.cast(IID_{symbol})\n"));
        }
    } else {
        out.push_str("        self._obj = obj\n");
    }
    if let Some(collection_iface) = collection_iface
        && !collection_uses_default
    {
        let symbol = interface_symbol(context, collection_iface);
        out.push_str(&format!(
            "        self._collection_obj = obj.cast(IID_{symbol})\n"
        ));
    }
    if class
        .required_interfaces
        .iter()
        .any(|iface| iface.iid == "30d5a829-7fa4-4026-83bb-d75bae4ea99e")
    {
        out.push_str("        self._closed = False\n");
    }
    out.push_str("        self._dynwinrt_native_ready = True\n");
    out.push_str(&format!(
        "        _dynwinrt_track_projected(self, '{}')\n",
        class.full_name
    ));
    out.push_str("        _dynwinrt_cache_projected(self)\n");
    out.push('\n');
    if native_projectable {
        out.push_str("    @classmethod\n");
        out.push_str("    def _from_native(cls, obj: DynWinRTValue):\n");
        out.push_str("        return cls(obj)\n\n");
    }
    if let Some(native_override_names) = &native_override_names {
        out.push_str("    @classmethod\n");
        out.push_str(
            "    def register_xaml_runtime_class(cls, runtime_class_name: str, control_type: type):\n",
        );
        out.push_str(&format!(
            "        \"\"\"Register a Python `{}` subclass for process-local XAML markup activation.\"\"\"\n",
            class.name
        ));
        out.push_str(&format!(
            "        if not isinstance(control_type, type) or control_type is {} or not issubclass(control_type, {}):\n",
            class.name, class.name
        ));
        out.push_str(&format!(
            "            raise TypeError(\"control_type must be a Python subclass of {}\")\n",
            class.name
        ));
        out.push_str(&format!(
            "        _native_members = set().union(*(set(_type.__dict__) for _type in control_type.__mro__ if _type is not {}))\n",
            class.name
        ));
        out.push_str(&format!(
            "        _native_overrides = sorted(_native_members.intersection({native_override_names}))\n"
        ));
        out.push_str(&format!(
            "        _unsupported_native_overrides = sorted(set(_native_overrides).difference({supported_override_names_expr}))\n"
        ));
        out.push_str("        if _unsupported_native_overrides:\n");
        out.push_str(&format!(
            "            raise TypeError(\"{} native override ABI is unsupported: \" + \", \".join(_unsupported_native_overrides))\n",
            class.name
        ));
        let default_iid = class
            .default_interface
            .as_ref()
            .map(|interface| format!("IID_{}", interface_symbol(context, interface)))
            .expect("public composable runtime class has a default interface");
        out.push_str(&format!(
            "        return _dynwinrt_register_xaml_runtime_class(runtime_class_name, '{}', {default_iid}, control_type, _native_overrides)\n\n",
            class.full_name
        ));
    }
    out.push_str("    def __init__(self, *args, **kwargs):\n");
    out.push_str(
        "        if getattr(self, '_dynwinrt_native_ready', False):\n\
         \x20           return\n",
    );
    if native_projectable {
        out.push_str(
            "        if len(args) == 1 and not kwargs and isinstance(args[0], DynWinRTValue):\n\
             \x20           self._set_native(args[0])\n\
             \x20           return\n",
        );
    }

    if has_public_composition {
        let native_override_names = native_override_names
            .as_ref()
            .expect("public composition override names");
        out.push_str(&format!(
            "        _is_python_subclass = type(self) is not {}\n",
            class.name
        ));
        out.push_str("        if _is_python_subclass:\n");
        out.push_str(&format!(
            "            _native_members = set().union(*(set(_type.__dict__) for _type in type(self).__mro__ if _type is not {}))\n",
            class.name
        ));
        out.push_str(&format!(
            "            _native_overrides = sorted(_native_members.intersection({native_override_names}))\n"
        ));
        out.push_str(&format!(
            "            _unsupported_native_overrides = sorted(set(_native_overrides).difference({supported_override_names_expr}))\n"
        ));
        out.push_str("            if _unsupported_native_overrides:\n");
        out.push_str(&format!(
            "                raise TypeError(\"{} native override ABI is unsupported: \" + \", \".join(_unsupported_native_overrides))\n",
            class.name
        ));
        out.push_str("            _override_interfaces = []\n");
        out.push_str("            _override_target_ref = _weakref_ref(self)\n");
        for (interface, shapes) in &supported_override_interfaces {
            let callback_methods = interface
                .methods
                .iter()
                .zip(shapes)
                .filter(|(_, shape)| matches!(**shape, "void0" | "size_f32_to_size_f32"))
                .collect::<Vec<_>>();
            if callback_methods.is_empty() {
                continue;
            }
            out.push_str("            _override_callbacks = {}\n");
            for (method, _) in callback_methods {
                let name = to_snake_case(&method.name);
                out.push_str(&format!(
                    "            if '{name}' in _native_overrides:\n\
                     \x20               def _override_{name}(*_args, _target_ref=_override_target_ref):\n\
                     \x20                   _target = _target_ref()\n\
                     \x20                   if _target is None:\n\
                     \x20                       raise RuntimeError('Python override target has been released.')\n\
                     \x20                   return _target.{name}(*_args)\n\
                     \x20               _override_callbacks[{}] = _override_{name}\n",
                    method.vtable_index
                ));
            }
            out.push_str("            if _override_callbacks:\n");
            let symbol = interface_symbol(context, interface);
            out.push_str(&format!(
                "                _override_interfaces.append(DynWinRTOverrideInterface(IID_{symbol}, [{}], _override_callbacks))\n",
                shapes
                    .iter()
                    .map(|shape| format!("'{shape}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
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
                py_method_type_guard(&format!("_bound[{index}]"), &param.typ, context)
            })
            .collect::<Vec<_>>();
        let condition = if guards.is_empty() {
            "_bound is not None".to_string()
        } else {
            format!("_bound is not None and {}", guards.join(" and "))
        };
        out.push_str(&format!("        if {condition}:\n"));
        if let Some(composed_call) = &candidate.composed_call_expr {
            out.push_str("            if _is_python_subclass:\n");
            out.push_str(&format!(
                "                self._set_native({composed_call}, _allow_native_overrides=True)\n\
                 \x20               return\n"
            ));
        } else if has_public_composition {
            out.push_str("            if _is_python_subclass:\n");
            out.push_str(&format!(
                "                raise TypeError(\"{} does not support Python subclass construction for this constructor\")\n",
                class.name
            ));
        }
        out.push_str(&format!(
            "            self._set_native({}._obj)\n\
             \x20           return\n",
            candidate.call_expr
        ));
    }
    if candidates.is_empty() {
        out.push_str(&format!(
            "        raise TypeError(\"{} cannot be constructed directly\")\n\n",
            class.name
        ));
    } else {
        out.push_str(&format!(
            "        raise TypeError(\"No matching constructor for {}\")\n\n",
            class.name
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::ConstructorMeta;
    use crate::types::{TypeKind, TypeRef};
    use std::process::Command;

    fn enum_type(name: &str) -> TypeMeta {
        TypeMeta::Enum {
            namespace: "Contoso".into(),
            name: name.into(),
            underlying: Box::new(TypeMeta::I32),
            members: Vec::new(),
            is_flags: false,
            doc: None,
            deprecated: None,
        }
    }

    fn constructor_method(name: &str, vtable_index: usize, typ: TypeMeta) -> MethodMeta {
        MethodMeta {
            name: name.into(),
            raw_name: name.into(),
            vtable_index,
            params: vec![ParamMeta {
                name: "value".into(),
                typ,
                direction: ParamDirection::In,
            }],
            return_type: Some(TypeMeta::RuntimeClass {
                namespace: "Contoso".into(),
                name: "Widget".into(),
                default_interface: None,
            }),
            ..Default::default()
        }
    }

    fn constructor_class(factory_methods: Vec<MethodMeta>) -> ClassMeta {
        ClassMeta {
            name: "Widget".into(),
            namespace: "Contoso".into(),
            full_name: "Contoso.Widget".into(),
            factory_interfaces: vec![InterfaceMeta {
                name: "IWidgetFactory".into(),
                namespace: "Contoso".into(),
                methods: factory_methods,
                ..Default::default()
            }],
            constructors: vec![ConstructorMeta {
                kind: ConstructorKind::FactoryActivation,
                factory_interface: Some(TypeRef {
                    namespace: "Contoso".into(),
                    name: "IWidgetFactory".into(),
                    kind: TypeKind::Interface,
                }),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn constructor_reuses_duplicate_slot_private_overload_names() {
        let integer = constructor_method("Create", 6, TypeMeta::I32);
        let text = constructor_method("Create", 6, TypeMeta::String);
        let factory = |name: &str, method| InterfaceMeta {
            name: name.into(),
            namespace: "Contoso".into(),
            methods: vec![method],
            ..Default::default()
        };
        let class = ClassMeta {
            name: "Widget".into(),
            namespace: "Contoso".into(),
            full_name: "Contoso.Widget".into(),
            factory_interfaces: vec![
                factory("IWidgetFactory", integer),
                factory("IWidgetFactory2", text),
            ],
            constructors: vec![
                ConstructorMeta {
                    kind: ConstructorKind::FactoryActivation,
                    factory_interface: Some(TypeRef {
                        namespace: "Contoso".into(),
                        name: "IWidgetFactory".into(),
                        kind: TypeKind::Interface,
                    }),
                },
                ConstructorMeta {
                    kind: ConstructorKind::FactoryActivation,
                    factory_interface: Some(TypeRef {
                        namespace: "Contoso".into(),
                        name: "IWidgetFactory2".into(),
                        kind: TypeKind::Interface,
                    }),
                },
            ],
            ..Default::default()
        };

        let code = generate_class(&PythonProjectionContext::default(), &class, &HashSet::new());
        assert!(code.contains("def _create_6_0("), "{code}");
        assert!(code.contains("def _create_6_1("), "{code}");
        assert!(code.contains("type(self)._create_6_0(_bound[0])"), "{code}");
        assert!(code.contains("type(self)._create_6_1(_bound[0])"), "{code}");
        assert!(!code.contains("type(self)._create_6(_bound[0])"), "{code}");
    }

    fn run_python(script: &str) -> String {
        fn invoke(
            program: &str,
            args: &[&str],
            script: &str,
        ) -> std::io::Result<std::process::Output> {
            let mut command = Command::new(program);
            for arg in args {
                command.arg(arg);
            }
            command.arg(script).output()
        }

        let output = invoke("python", &["-c"], script).or_else(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                invoke("py", &["-3", "-c"], script)
            } else {
                Err(error)
            }
        });
        let output = output.unwrap_or_else(|error| panic!("failed to launch Python: {error}"));
        assert!(
            output.status.success(),
            "python script failed\nstdout:\n{}\nstderr:\n{}\nscript:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            script
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[test]
    fn python_constructor_enum_overload_prefers_enum_over_i32_in_both_orders() {
        let integer = constructor_method("Create", 6, TypeMeta::I32);
        let enumeration = constructor_method("Create2", 7, enum_type("Mode"));
        let context =
            PythonProjectionContext::packaged([enum_type("Mode").type_identity()]).unwrap();

        let forward = generate_python_constructor(
            &context,
            &constructor_class(vec![integer.clone(), enumeration.clone()]),
            None,
            false,
        );
        let reverse = generate_python_constructor(
            &context,
            &constructor_class(vec![enumeration, integer]),
            None,
            false,
        );

        assert_eq!(forward, reverse);
        assert!(forward.contains(
            "isinstance(_bound[0], int) and not isinstance(_bound[0], bool) and not isinstance(_bound[0], __import__('enum').Enum)"
        ));
        assert!(
            forward.contains("isinstance(_bound[0], _dynwinrt_symbol('contoso__mode', 'Mode'))")
        );
        let forward_script = forward.replace("if cls is Widget:", "if cls is WidgetForward:");
        let reverse_script = reverse.replace("if cls is Widget:", "if cls is WidgetReverse:");

        let script = format!(
            r#"from enum import IntEnum
import json

class DynWinRTValue:
    pass

def _dynwinrt_bind_overload(parameter_names, args, kwargs):
    if kwargs:
        if args:
            return None
        if len(kwargs) != len(parameter_names) or any(name not in kwargs for name in parameter_names):
            return None
        return tuple(kwargs[name] for name in parameter_names)
    return args if len(args) == len(parameter_names) else None

def _dynwinrt_projected_from_native(cls, obj, setter_name):
    return obj

def _dynwinrt_track_projected(obj, name):
    return None

def _dynwinrt_cache_projected(*args, **kwargs):
    return None

def _dynwinrt_from_value(cls, obj):
    return cls(obj)

def _dynwinrt_symbol(module, name):
    return globals()[name]

class Mode(IntEnum):
    VALUE = 1

class OtherMode(IntEnum):
    VALUE = 1

class _CtorResult:
    def __init__(self, value):
        self._obj = value

class WidgetForward:
    @staticmethod
    def _create_6(value):
        return _CtorResult("i32")

    @staticmethod
    def _create_7(value):
        return _CtorResult("enum")

{forward_script}

class WidgetReverse:
    @staticmethod
    def _create_6(value):
        return _CtorResult("i32")

    @staticmethod
    def _create_7(value):
        return _CtorResult("enum")

{reverse_script}

def exercise(widget_type):
    enum_widget = widget_type(Mode.VALUE)
    int_widget = widget_type(42)
    results = [enum_widget._obj, int_widget._obj]
    try:
        widget_type(OtherMode.VALUE)
    except TypeError as error:
        results.append(type(error).__name__)
    else:
        results.append("unexpected")
    return results

print(json.dumps([exercise(WidgetForward), exercise(WidgetReverse)]))
"#
        );

        assert_eq!(
            run_python(&script),
            r#"[["enum", "i32", "TypeError"], ["enum", "i32", "TypeError"]]"#
        );
    }
}
