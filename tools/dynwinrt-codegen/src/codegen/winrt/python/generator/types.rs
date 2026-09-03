// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python enum, interface, and delegate generation.

use super::imports::{emit_type_checking_imports, format_py_type_import};
use super::structs::{generate_struct_helpers, generate_struct_imports};
use super::*;
use crate::codegen::winrt::python::collections::{
    CollectionKind, interface_kind, map_iterable_identity, observable_vector_identity,
    runtime_mixin,
};
use crate::types::{TypeIdentity, TypeIdentityKind};

/// Generate a Python file for a single enum.
pub fn generate_enum(_context: &PythonProjectionContext, en: &TypeMeta) -> Option<String> {
    let (name, members, is_flags, enum_doc, enum_dep) = match en {
        TypeMeta::Enum {
            name,
            members,
            is_flags,
            doc,
            deprecated,
            ..
        } => (
            name,
            members,
            *is_flags,
            doc.as_deref(),
            deprecated.as_deref(),
        ),
        _ => return None,
    };

    let mut out = String::new();
    out.push_str(HEADER);
    let enum_base = if is_flags { "IntFlag" } else { "IntEnum" };
    out.push_str(&format!("from enum import {enum_base}\n\n\n"));
    out.push_str(&format!("class {}({enum_base}):\n", name));
    let type_doc = crate::codegen::winrt::shared::docs::DocText {
        summary: enum_doc,
        deprecated: enum_dep,
        returns: None,
        params: Vec::new(),
    };
    let type_ds = crate::codegen::winrt::python::docs::format_pydoc(&type_doc, "    ");
    if !type_ds.is_empty() {
        out.push_str(&type_ds);
        out.push('\n');
    }
    if members.is_empty() && type_ds.is_empty() {
        out.push_str("    pass\n");
    } else {
        for member in members {
            let member_name = if is_py_reserved(&member.name) {
                format!("{}_", member.name)
            } else {
                member.name.clone()
            };
            // Emit docs as leading `#` comments. A standalone docstring after the
            // assignment does not actually attach to the enum member in Python.
            if let Some(d) = member.doc.as_deref() {
                for line in d.lines() {
                    let line = line.trim_end();
                    if line.is_empty() {
                        out.push_str("    #\n");
                    } else {
                        out.push_str(&format!("    # {}\n", line));
                    }
                }
            }
            out.push_str(&format!("    {} = {}\n", member_name, member.value));
        }
    }
    Some(out)
}

/// Generate a Python file for a WinRT interface (non-exclusive).
pub fn generate_interface(context: &PythonProjectionContext, iface: &InterfaceMeta) -> String {
    let mut projected_iface = iface.clone();
    projected_iface.name = context.projected_name_for_interface(iface);
    let iface = &projected_iface;
    let is_delegate = iface.is_delegate();
    if is_delegate {
        return generate_delegate(iface);
    }

    let used_structs = collect_used_structs_from_iface(iface);

    let mut out = String::new();
    out.push_str(HEADER);
    out.push_str(FUTURE_ANNOTATIONS);
    out.push_str(IMPORT_LINE);
    let is_element_factory =
        iface.namespace == "Microsoft.UI.Xaml" && iface.name == "IElementFactory";
    if is_element_factory {
        out.push_str("from dynwinrt import DynWinRtElementFactory\n");
    }
    let collection_kind = interface_kind(iface);
    let observable_vector = observable_vector_identity(iface);
    if observable_vector.is_none()
        && let Some(mixin) = collection_kind.and_then(runtime_mixin)
    {
        out.push_str(&format!("from dynwinrt.dynwinrt import {mixin}\n"));
    }
    if methods_have_async_output(iface.methods.iter()) {
        out.push_str(ASYNC_IMPORT_LINE);
    }
    if context.is_packaged() {
        out.push_str(&generate_struct_imports(context, &used_structs));
    }
    if has_ireference_input(iface.methods.iter()) || has_ireference_struct_field(&used_structs) {
        out.push_str(IREFERENCE_HELPER);
    }
    out.push('\n');
    let mut type_checking_imports = Vec::new();

    // Collect delegate names
    let delegate_names = super::super::collect_referenced_delegate_names(&iface.methods, context);
    let runtime_delegate_names =
        super::super::collect_runtime_delegate_names(&iface.methods, context);

    // Import parameterized collection types (skip delegates)
    let collection_identities = collect_used_generic_identities_from_methods(&iface.methods);
    for identity in &collection_identities {
        let identity = context.normalize_identity(identity);
        if identity != iface.type_identity() && !delegate_names.contains(&identity) {
            let module = context.implementation_module(&identity);
            let name = context.projected_name(&identity);
            let reference_name = context.reference_name(&identity);
            let import = if name == reference_name {
                name
            } else {
                format!("{name} as {reference_name}")
            };
            type_checking_imports.push(format!("from .{module} import {import}  # noqa: F401\n"));
        }
    }
    if let Some(identity) = &observable_vector {
        if !collection_identities
            .iter()
            .any(|candidate| context.normalize_identity(candidate) == *identity)
        {
            let module = context.implementation_module(&identity);
            let vector_name = context.projected_name(&identity);
            let reference_name = context.reference_name(&identity);
            let import = if vector_name == reference_name {
                vector_name
            } else {
                format!("{vector_name} as {reference_name}")
            };
            type_checking_imports.push(format!("from .{module} import {import}  # noqa: F401\n"));
        }
    }
    if observable_vector.is_some() {
        let event_args = "IVectorChangedEventArgs";
        let identity = TypeIdentity::named(
            TypeIdentityKind::Interface,
            crate::meta::WINDOWS_FOUNDATION_COLLECTIONS_NAMESPACE,
            event_args,
        );
        let module = context.implementation_module(&identity);
        type_checking_imports.push(format!(
            "from .{module} import {event_args}  # noqa: F401\n"
        ));
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

    // Type imports for referenced types
    let type_imports = collect_iface_type_imports(iface);
    let mut sorted_type_imports: Vec<_> = type_imports.iter().collect();
    sorted_type_imports
        .sort_by(|a, b| (&a.namespace, &a.name, &a.kind).cmp(&(&b.namespace, &b.name, &b.kind)));
    for r in &sorted_type_imports {
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
        }
    }
    emit_type_checking_imports(&mut out, type_checking_imports);

    // IID constant
    if let Some(iid_expr) = py_interface_iid_expr(iface) {
        out.push_str(&format!("IID_{} = {}\n", iface.name, iid_expr));
    }
    let mut argument_iids = Vec::new();
    for method in &iface.methods {
        for parameter in &method.params {
            if parameter.direction == ParamDirection::In {
                py_collect_runtime_class_iid_consts(&parameter.typ, &mut argument_iids);
            }
        }
    }
    argument_iids.sort();
    argument_iids.dedup();
    for (name, iid) in argument_iids {
        out.push_str(&format!("{} = WinGUID.parse('{}')\n", name, iid));
    }
    out.push('\n');

    // Interface registration
    out.push_str(&py_generate_interface_registration(
        iface,
        &format!("_{}", iface.name),
        &iface.name,
    ));
    out.push('\n');

    // Struct helpers
    if !context.is_packaged() {
        for s in &used_structs {
            out.push_str(&generate_struct_helpers(context, s));
            out.push('\n');
        }
    }

    // Wrapper class
    if let Some(identity) = &observable_vector {
        let vector_name = context.projected_name(identity);
        out.push_str(&format!(
            "\nclass {}({}):\n",
            iface.name,
            py_runtime_symbol(context, identity, &vector_name)
        ));
    } else if let Some(mixin) = collection_kind.and_then(runtime_mixin) {
        out.push_str(&format!("\nclass {}({mixin}):\n", iface.name));
    } else {
        out.push_str(&format!("\nclass {}:\n", iface.name));
    }
    {
        let doc = crate::codegen::winrt::shared::docs::DocText {
            summary: iface.doc.as_deref(),
            deprecated: iface.deprecated.as_deref(),
            ..Default::default()
        };
        out.push_str(&crate::codegen::winrt::python::docs::format_pydoc(
            &doc, "    ",
        ));
    }
    out.push_str("    _dynwinrt_interface_type = True\n");
    if !iface.iid.is_empty() || iface.generic_piid.is_some() {
        out.push_str(&format!(
            "    _dynwinrt_interface_iid = IID_{}\n",
            iface.name
        ));
    } else {
        out.push_str("    _dynwinrt_interface_iid = None\n");
    }
    out.push_str("    def __new__(cls, *args, **kwargs):\n");
    out.push_str(
        "        if len(args) == 1 and not kwargs and isinstance(args[0], DynWinRTValue):\n\
         \x20           return _dynwinrt_projected_from_native(cls, args[0], '_set_native')\n\
         \x20       return super().__new__(cls)\n\n",
    );
    out.push_str("    def _set_native(self, obj: DynWinRTValue):\n");
    if let Some(identity) = &observable_vector {
        let vector_name = context.projected_name(identity);
        out.push_str(&format!(
            "        {}._set_native(self, obj)\n",
            py_runtime_symbol(context, identity, &vector_name)
        ));
        out.push_str(&format!(
            "        self._observable_obj = obj.cast(IID_{})\n",
            iface.name
        ));
    } else if iface.generic_piid.is_some() {
        out.push_str(&format!(
            "        self._obj = obj.cast(IID_{})\n",
            iface.name
        ));
    } else {
        out.push_str("        self._obj = obj\n");
    }
    out.push_str("        self._dynwinrt_native_ready = True\n");
    out.push_str(&format!(
        "        _dynwinrt_track_projected(self, '{}.{}')\n",
        iface.namespace, iface.name
    ));
    out.push_str("        _dynwinrt_cache_projected(self)\n");
    out.push('\n');
    out.push_str("    def __init__(self, obj: DynWinRTValue):\n");
    out.push_str(
        "        if getattr(self, '_dynwinrt_native_ready', False):\n\
         \x20           return\n",
    );
    out.push_str(&format!("        {}._set_native(self, obj)\n", iface.name));
    out.push('\n');
    out.push_str("    @classmethod\n");
    out.push_str(&format!(
        "    def _from_native(cls, obj: DynWinRTValue) -> '{}':\n",
        iface.name
    ));
    out.push_str("        return cls(obj)\n");
    out.push('\n');

    // static from() — QI cast
    if !iface.iid.is_empty() || iface.generic_piid.is_some() {
        out.push_str("    @classmethod\n");
        out.push_str(&format!(
            "    def from_value(cls, obj: DynWinRTValue) -> '{}':\n",
            iface.name
        ));
        out.push_str(&format!(
            "        return cls._from_native(obj.cast(IID_{}))\n",
            iface.name
        ));
        out.push('\n');
        out.push_str("    def as_interface(self, interface_class):\n");
        out.push_str("        return interface_class.from_value(self._obj)\n");
        out.push('\n');
    }

    // static create() for IVector<T> and IMap<K,V>
    if let Some(ref piid) = iface.generic_piid {
        if piid == "5917eb53-50b4-4a0d-b309-65862b3f1dbc" && iface.generic_args.len() == 1 {
            let elem_type = py_dynwinrt_type(&iface.generic_args[0]);
            let elem_annotation = crate::codegen::winrt::python::type_helpers::py_param_type_safe(
                &iface.generic_args[0],
                context,
            );
            let wrap = py_wrap_native_value("item", &iface.generic_args[0]);
            let vector_identity = observable_vector
                .as_ref()
                .expect("observable vector companion");
            let vector_name = context.projected_name(vector_identity);
            out.push_str("    @staticmethod\n");
            out.push_str(&format!(
                "    def create(items: Iterable[{}]) -> '{}':\n",
                elem_annotation, iface.name
            ));
            out.push_str(&format!(
                "        return {}(_dynwinrt_new_vector(items, lambda item: {}, {}))\n\n",
                iface.name, wrap, elem_type
            ));
            out.push_str(&format!("    def as_vector(self) -> '{}':\n", vector_name));
            out.push_str(&format!("        return {}(self._obj)\n", {
                py_runtime_symbol(context, vector_identity, &vector_name)
            }));
            out.push('\n');
        } else if piid == "913337e9-11a1-4345-a3a2-4e7f956e222d" && iface.generic_args.len() == 1 {
            let elem_type = py_dynwinrt_type(&iface.generic_args[0]);
            let elem_annotation = crate::codegen::winrt::python::type_helpers::py_param_type_safe(
                &iface.generic_args[0],
                context,
            );
            let wrap = py_wrap_native_value("item", &iface.generic_args[0]);
            out.push_str("    @staticmethod\n");
            out.push_str(&format!(
                "    def create(items: Iterable[{}]) -> '{}':\n",
                elem_annotation, iface.name
            ));
            out.push_str(&format!(
                "        return {}(_dynwinrt_vector(items, lambda item: {}, {}))\n",
                iface.name, wrap, elem_type
            ));
            out.push('\n');
        } else if piid == "3c2925fe-8519-45c1-aa79-197b6718c1c1" && iface.generic_args.len() == 2 {
            let key_type = py_dynwinrt_type(&iface.generic_args[0]);
            let val_type = py_dynwinrt_type(&iface.generic_args[1]);
            let key_annotation = crate::codegen::winrt::python::type_helpers::py_param_type_safe(
                &iface.generic_args[0],
                context,
            );
            let val_annotation = crate::codegen::winrt::python::type_helpers::py_param_type_safe(
                &iface.generic_args[1],
                context,
            );
            let wrap_key = py_wrap_native_value("item", &iface.generic_args[0]);
            let wrap_value = py_wrap_native_value("item", &iface.generic_args[1]);
            out.push_str("    @staticmethod\n");
            out.push_str(&format!(
                "    def create(items: Mapping[{}, {}]) -> '{}':\n",
                key_annotation, val_annotation, iface.name
            ));
            out.push_str(&format!(
                "        return {}(_dynwinrt_map(items, lambda item: {}, lambda item: {}, {}, {}))\n",
                iface.name, wrap_key, wrap_value, key_type, val_type
            ));
            out.push('\n');
        }
    }

    if crate::codegen::winrt::is_ibuffer_interface(&iface.namespace, &iface.name, &iface.iid) {
        out.push_str(
            "    @staticmethod\n\
             \x20   def from_bytes(data: bytes | bytearray) -> 'IBuffer':\n\
             \x20       \"\"\"Create an owned IBuffer by copying bytes or bytearray data.\"\"\"\n\
             \x20       return IBuffer._from_native(DynWinRTValue.from_bytes(data))\n\n\
             \x20   def to_bytes(self) -> bytes:\n\
             \x20       \"\"\"Copy the initialized IBuffer data into a new bytes object.\"\"\"\n\
             \x20       return self._obj.to_bytes()\n\n",
        );
    }

    if is_element_factory {
        let get_args = py_runtime_named_symbol(
            context,
            TypeIdentityKind::Class,
            "Microsoft.UI.Xaml",
            "ElementFactoryGetArgs",
            "ElementFactoryGetArgs",
        );
        let recycle_args = py_runtime_named_symbol(
            context,
            TypeIdentityKind::Class,
            "Microsoft.UI.Xaml",
            "ElementFactoryRecycleArgs",
            "ElementFactoryRecycleArgs",
        );
        let ui_element_iid = py_runtime_named_symbol(
            context,
            TypeIdentityKind::Class,
            "Microsoft.UI.Xaml",
            "UIElement",
            "IID_IUIElement",
        );
        out.push_str(
            r#"    @staticmethod
    def create(get_element, recycle_element):
        elements = {}
        callback_state = [True]

        class RecycleArgsProxy:
            def __init__(self, source, element):
                object.__setattr__(self, '_source', source)
                object.__setattr__(self, '_element', element)

            @property
            def element(self):
                return self._element

            def __getattr__(self, name):
                return getattr(self._source, name)

            def __setattr__(self, name, value):
                if name == 'element':
                    object.__setattr__(self, '_element', value)
                setattr(self._source, name, value)

"#,
        );
        out.push_str(&format!(
            "        def get_native(args):\n\
             \x20           if not callback_state[0]:\n\
             \x20               raise RuntimeError('IElementFactory callbacks have been released.')\n\
             \x20           projected_args = {get_args}._from_native(args)\n\
             \x20           element = get_element(projected_args)\n\
             \x20           native = getattr(element, '_obj', element)\n\
             \x20           if not isinstance(native, DynWinRTValue):\n\
             \x20               raise TypeError('get_element must return a projected UIElement.')\n\
             \x20           native_element = native.cast({ui_element_iid})\n\
             \x20           if not callback_state[0]:\n\
             \x20               native_element.release()\n\
             \x20               raise RuntimeError('IElementFactory callbacks have been released.')\n\
             \x20           elements[native_element.identity_raw()] = element\n\
             \x20           return native_element\n\n"
        ));
        out.push_str(&format!(
            "        def recycle_native(args):\n\
             \x20           if not callback_state[0]:\n\
             \x20               raise RuntimeError('IElementFactory callbacks have been released.')\n\
             \x20           projected_args = {recycle_args}._from_native(args)\n\
             \x20           projected_element = projected_args.element\n\
             \x20           if projected_element is None:\n\
             \x20               if not callback_state[0]:\n\
             \x20                   raise RuntimeError('IElementFactory callbacks have been released.')\n\
             \x20               recycle_element(projected_args)\n\
             \x20               return\n\
             \x20           native = getattr(projected_element, '_obj', projected_element)\n\
             \x20           element = elements.pop(native.identity_raw(), projected_element)\n\
             \x20           if not callback_state[0]:\n\
             \x20               raise RuntimeError('IElementFactory callbacks have been released.')\n\
             \x20           recycle_element(RecycleArgsProxy(projected_args, element))\n\n"
        ));
        out.push_str(&format!(
            "        implementation = DynWinRtElementFactory.create(\n\
             \x20           {ui_element_iid}, get_native, recycle_native\n\
             \x20       )\n\
             \x20       factory = IElementFactory._from_native(implementation.to_value())\n\
             \x20       factory._element_factory_implementation = implementation\n\
             \x20       factory._element_factory_elements = elements\n\
             \x20       factory._element_factory_callback_state = callback_state\n\
             \x20       _dynwinrt_track_projected(factory, 'Microsoft.UI.Xaml.IElementFactory')\n\
             \x20       return factory\n\n"
        ));
        out.push_str(
            "    def release_callbacks(self):\n\
             \x20       callback_state = getattr(self, '_element_factory_callback_state', None)\n\
             \x20       if callback_state is not None:\n\
             \x20           callback_state[0] = False\n\
             \x20       elements = getattr(self, '_element_factory_elements', None)\n\
             \x20       if elements is not None:\n\
             \x20           elements.clear()\n\
             \x20       implementation = getattr(self, '_element_factory_implementation', None)\n\
             \x20       if implementation is not None:\n\
             \x20           implementation.release_callbacks()\n\
             \x20           self._element_factory_implementation = None\n\n",
        );
    }

    if matches!(
        collection_kind,
        Some(CollectionKind::Mapping | CollectionKind::MutableMapping)
    ) && let Some(iterable_identity) = map_iterable_identity(&iface.generic_args)
    {
        let iterable_name = context.projected_name(&iterable_identity);
        out.push_str("    def _iter_pairs(self):\n");
        out.push_str(&format!(
            "        return iter({}(self._obj))\n\n",
            py_runtime_symbol(context, &iterable_identity, &iterable_name)
        ));
    }

    // Instance methods (reorder so @property comes before @x.setter)
    let iface_var = format!("_{}", iface.name);
    let obj_expr = if observable_vector.is_some() {
        "self._observable_obj"
    } else {
        "self._obj"
    };
    for methods in crate::codegen::winrt::python::overloads::grouped_methods(
        reorder_getters_before_setters(&iface.methods),
    ) {
        out.push('\n');
        let overloads = methods
            .into_iter()
            .map(|method| InstanceOverload {
                iface_var: iface_var.clone(),
                obj_expr: obj_expr.to_string(),
                method,
                sibling_methods: Some(iface.methods.as_slice()),
                property_has_getter: !method.is_property_setter
                    || method.name.strip_prefix("put_").is_some_and(|suffix| {
                        iface
                            .methods
                            .iter()
                            .any(|candidate| candidate.name == format!("get_{suffix}"))
                    }),
            })
            .collect::<Vec<_>>();
        out.push_str(&generate_instance_method_group(&overloads, context));
    }
    let aliases = generate_compatibility_aliases(iface.methods.iter());
    if !aliases.is_empty() {
        out.push('\n');
        out.push_str(&aliases);
    }

    out
}

/// Generate a Python file for a delegate type.
fn generate_delegate(iface: &InterfaceMeta) -> String {
    let mut out = String::new();
    out.push_str(HEADER);
    out.push_str(FUTURE_ANNOTATIONS);
    out.push_str("from dynwinrt import DynWinRTType, WinGUID\n\n");

    let invoke = iface.methods.iter().find(|m| m.name == "Invoke");
    if iface.generic_piid.is_some() {
        let generic_arg_exprs = iface
            .generic_args
            .iter()
            .map(py_dynwinrt_type)
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "IID_{} = DynWinRTType.parameterized(WinGUID.parse('{}'), [{}]).iid()\n",
            iface.name, iface.iid, generic_arg_exprs
        ));
    } else if !iface.iid.is_empty() {
        out.push_str(&format!(
            "IID_{} = WinGUID.parse('{}')\n",
            iface.name, iface.iid
        ));
    } else {
        out.push_str(&format!("IID_{} = None\n", iface.name));
    }

    if let Some(invoke) = invoke {
        let param_exprs: Vec<String> = invoke
            .params
            .iter()
            .filter(|p| p.direction == ParamDirection::In)
            .map(|p| py_dynwinrt_type(&p.typ))
            .collect();
        out.push_str(&format!(
            "{}_PARAM_TYPES = [{}]\n",
            iface.name,
            param_exprs.join(", ")
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::EnumMember;

    #[test]
    fn fixed_delegate_uses_declared_iid() {
        let iface = InterfaceMeta {
            name: "WorkItemHandler".into(),
            iid: "1d1a8b8b-fa66-414f-9cbd-b65fc99d17fa".into(),
            methods: vec![MethodMeta {
                name: "Invoke".into(),
                params: vec![crate::meta::ParamMeta {
                    name: "operation".into(),
                    typ: TypeMeta::AsyncAction,
                    direction: ParamDirection::In,
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        let code = generate_delegate(&iface);
        assert!(code.contains(
            "IID_WorkItemHandler = WinGUID.parse('1d1a8b8b-fa66-414f-9cbd-b65fc99d17fa')"
        ));
        assert!(!code.contains("DynWinRTType.parameterized"));
    }

    #[test]
    fn flags_enum_uses_int_flag() {
        let value = TypeMeta::Enum {
            namespace: "Test".into(),
            name: "Options".into(),
            underlying: Box::new(TypeMeta::U32),
            members: vec![EnumMember {
                name: "First".into(),
                value: 1,
                doc: None,
            }],
            is_flags: true,
            doc: None,
            deprecated: None,
        };

        let code = generate_enum(&PythonProjectionContext::default(), &value).unwrap();
        assert!(code.contains("from enum import IntFlag"));
        assert!(code.contains("class Options(IntFlag):"));
    }

    #[test]
    fn interface_projection_exposes_its_target_iid() {
        let iface = InterfaceMeta {
            name: "IWidget".into(),
            namespace: "Contoso".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
            ..Default::default()
        };
        let context = PythonProjectionContext::standalone([iface.type_identity()]).unwrap();
        let code = generate_interface(&context, &iface);
        assert!(code.contains("_dynwinrt_interface_type = True"));
        assert!(code.contains("_dynwinrt_interface_iid = IID_IWidget"));
        assert!(code.contains("@classmethod\n    def from_value(cls, obj: DynWinRTValue)"));
        assert!(code.contains("return cls._from_native(obj.cast(IID_IWidget))"));
    }

    #[test]
    fn collection_create_inputs_remain_non_nullable() {
        let iface = InterfaceMeta {
            name: "IVector_Widget".into(),
            iid: "913337e9-11a1-4345-a3a2-4e7f956e222d".into(),
            generic_piid: Some("913337e9-11a1-4345-a3a2-4e7f956e222d".into()),
            generic_args: vec![TypeMeta::RuntimeClass {
                namespace: "Contoso".into(),
                name: "Widget".into(),
                default_interface: None,
            }],
            ..Default::default()
        };
        let context = PythonProjectionContext::standalone([
            iface.type_identity(),
            TypeIdentity::named(TypeIdentityKind::Class, "Contoso", "Widget"),
        ])
        .unwrap();
        let code = generate_interface(&context, &iface);

        assert!(code.contains("def create(items: Iterable['WidgetLike'])"));
        assert!(!code.contains("def create(items: Iterable[WidgetLike | None])"));
    }
}
