// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python enum, interface, and delegate generation.

use super::imports::{emit_type_checking_imports, format_py_type_import};
use super::structs::generate_struct_helpers;
use super::*;
use crate::codegen::winrt::python::collections::{
    CollectionKind, interface_kind, map_iterable_name, observable_vector_name, runtime_mixin,
};

/// Generate a Python file for a single enum.
pub fn generate_enum(en: &TypeMeta) -> Option<String> {
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
pub fn generate_interface(
    iface: &InterfaceMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
) -> String {
    let is_delegate = iface.methods.iter().any(|m| m.name == ".ctor")
        && iface.methods.iter().any(|m| m.name == "Invoke");
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
        out.push_str("from dynwinrt_py import DynWinRtElementFactory\n");
    }
    let collection_kind = interface_kind(iface);
    let observable_vector = observable_vector_name(iface);
    if observable_vector.is_none()
        && let Some(mixin) = collection_kind.and_then(runtime_mixin)
    {
        out.push_str(&format!("from dynwinrt_py.dynwinrt_py import {mixin}\n"));
    }
    if methods_have_async_output(iface.methods.iter()) {
        out.push_str(ASYNC_IMPORT_LINE);
    }
    if has_ireference_input(iface.methods.iter()) || has_ireference_struct_field(&used_structs) {
        out.push_str(IREFERENCE_HELPER);
    }
    out.push('\n');
    let mut type_checking_imports = Vec::new();

    // Collect delegate names
    let mut delegate_names: HashSet<String> = delegate_type_names.clone();
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

    // Import parameterized collection types (skip delegates)
    let collection_names = collect_used_generics_from_methods(&iface.methods);
    for cname in &collection_names {
        if cname != &iface.name && !delegate_names.contains(cname) {
            let module = to_snake_case_filename(cname);
            type_checking_imports
                .push(format!("from .{} import {}  # noqa: F401\n", module, cname));
        }
        if let Some(vector_name) = &observable_vector
            && !collection_names.contains(vector_name)
        {
            let module = to_snake_case_filename(vector_name);
            type_checking_imports.push(format!(
                "from .{module} import {vector_name}  # noqa: F401\n"
            ));
        }
        if observable_vector.is_some() {
            let event_args = "IVectorChangedEventArgs";
            let module = to_snake_case_filename(event_args);
            type_checking_imports.push(format!(
                "from .{module} import {event_args}  # noqa: F401\n"
            ));
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

    // Type imports for referenced types
    let type_imports = collect_iface_type_imports(iface);
    let mut sorted_type_imports: Vec<_> = type_imports.iter().collect();
    sorted_type_imports
        .sort_by(|a, b| (&a.namespace, &a.name, &a.kind).cmp(&(&b.namespace, &b.name, &b.kind)));
    for r in &sorted_type_imports {
        if known_types.contains(&r.name) && !delegate_names.contains(&r.name) {
            type_checking_imports.push(format_py_type_import(&r.namespace, &r.name, r.kind));
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
    ));
    out.push('\n');

    // Struct helpers
    for s in &used_structs {
        out.push_str(&generate_struct_helpers(s));
        out.push('\n');
    }

    // Wrapper class
    if let Some(vector_name) = &observable_vector {
        out.push_str(&format!(
            "\nclass {}({}):\n",
            iface.name,
            py_runtime_symbol(vector_name, vector_name)
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
    out.push_str("    def __init__(self, obj: DynWinRTValue):\n");
    if let Some(vector_name) = &observable_vector {
        out.push_str(&format!(
            "        {}.__init__(self, obj)\n",
            py_runtime_symbol(vector_name, vector_name)
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
    out.push_str(&format!(
        "        _dynwinrt_track_projected(self, '{}.{}')\n",
        iface.namespace, iface.name
    ));
    out.push('\n');

    // static from() — QI cast
    if !iface.iid.is_empty() || iface.generic_piid.is_some() {
        out.push_str("    @staticmethod\n");
        out.push_str(&format!(
            "    def from_value(obj: DynWinRTValue) -> '{}':\n",
            iface.name
        ));
        out.push_str(&format!(
            "        return {}(obj.cast(IID_{}))\n",
            iface.name, iface.name
        ));
        out.push('\n');
    }

    // static create() for IVector<T> and IMap<K,V>
    if let Some(ref piid) = iface.generic_piid {
        if piid == "5917eb53-50b4-4a0d-b309-65862b3f1dbc" && iface.generic_args.len() == 1 {
            let elem_type = py_dynwinrt_type(&iface.generic_args[0]);
            let elem_annotation = crate::codegen::winrt::python::type_helpers::py_param_type_safe(
                &iface.generic_args[0],
                known_types,
            );
            let wrap = py_wrap_native_value("item", &iface.generic_args[0]);
            let vector_name = observable_vector
                .as_ref()
                .expect("observable vector companion");
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
            out.push_str(&format!(
                "        return {}(self._obj)\n",
                py_runtime_symbol(vector_name, vector_name)
            ));
            out.push('\n');
        } else if piid == "913337e9-11a1-4345-a3a2-4e7f956e222d" && iface.generic_args.len() == 1 {
            let elem_type = py_dynwinrt_type(&iface.generic_args[0]);
            let elem_annotation = crate::codegen::winrt::python::type_helpers::py_param_type_safe(
                &iface.generic_args[0],
                known_types,
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
                known_types,
            );
            let val_annotation = crate::codegen::winrt::python::type_helpers::py_param_type_safe(
                &iface.generic_args[1],
                known_types,
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

    if is_element_factory {
        let get_args = py_runtime_symbol("ElementFactoryGetArgs", "ElementFactoryGetArgs");
        let recycle_args =
            py_runtime_symbol("ElementFactoryRecycleArgs", "ElementFactoryRecycleArgs");
        let ui_element_iid = py_runtime_symbol("UIElement", "IID_IUIElement");
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
             \x20       factory = IElementFactory(implementation.to_value())\n\
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
    ) && let Some(iterable_name) = map_iterable_name(&iface.generic_args)
    {
        out.push_str("    def _iter_pairs(self):\n");
        out.push_str(&format!(
            "        return iter({}(self._obj))\n\n",
            py_runtime_symbol(&iterable_name, &iterable_name)
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
        out.push_str(&generate_instance_method_group(
            &overloads,
            known_types,
            &delegate_names,
        ));
    }

    out
}

/// Generate a Python file for a delegate type.
fn generate_delegate(iface: &InterfaceMeta) -> String {
    let mut out = String::new();
    out.push_str(HEADER);
    out.push_str(FUTURE_ANNOTATIONS);
    out.push_str("from dynwinrt_py import DynWinRTType, WinGUID\n\n");

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

        let code = generate_enum(&value).unwrap();
        assert!(code.contains("from enum import IntFlag"));
        assert!(code.contains("class Options(IntFlag):"));
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
        let code = generate_interface(
            &iface,
            &HashSet::from(["Widget".into(), "IVector_Widget".into()]),
            &HashSet::new(),
        );

        assert!(code.contains("def create(items: Iterable['Widget'])"));
        assert!(!code.contains("def create(items: Iterable[Widget | None])"));
    }
}
