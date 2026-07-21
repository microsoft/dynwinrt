// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python enum, interface, and delegate generation.

use super::imports::{emit_type_checking_imports, format_py_type_import};
use super::structs::generate_struct_helpers;
use super::*;

/// Generate a Python file for a single enum.
pub fn generate_enum(en: &TypeMeta) -> Option<String> {
    let (name, members, enum_doc, enum_dep) = match en {
        TypeMeta::Enum {
            name,
            members,
            doc,
            deprecated,
            ..
        } => (name, members, doc.as_deref(), deprecated.as_deref()),
        _ => return None,
    };

    let mut out = String::new();
    out.push_str(HEADER);
    out.push_str("from enum import IntEnum\n\n\n");
    out.push_str(&format!("class {}(IntEnum):\n", name));
    let type_doc = crate::codegen::shared::docs::DocText {
        summary: enum_doc,
        deprecated: enum_dep,
        returns: None,
        params: Vec::new(),
    };
    let type_ds = crate::codegen::python::docs::format_pydoc(&type_doc, "    ");
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
    if methods_have_async_output(iface.methods.iter()) {
        out.push_str(ASYNC_IMPORT_LINE);
    }
    if has_ireference_input(iface.methods.iter()) {
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
            type_checking_imports.push(format_py_type_import(&r.name, r.kind));
        }
    }
    emit_type_checking_imports(&mut out, type_checking_imports);

    // IID constant
    if let Some(iid_expr) = py_interface_iid_expr(iface) {
        out.push_str(&format!("IID_{} = {}\n\n", iface.name, iid_expr));
    }

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
    out.push_str(&format!("\nclass {}:\n", iface.name));
    {
        let doc = crate::codegen::shared::docs::DocText {
            summary: iface.doc.as_deref(),
            deprecated: iface.deprecated.as_deref(),
            ..Default::default()
        };
        out.push_str(&crate::codegen::python::docs::format_pydoc(&doc, "    "));
    }
    out.push_str("    def __init__(self, obj: DynWinRTValue):\n");
    if iface.generic_piid.is_some() {
        out.push_str(&format!(
            "        self._obj = obj.cast(IID_{})\n",
            iface.name
        ));
    } else {
        out.push_str("        self._obj = obj\n");
    }
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
        if piid == "913337e9-11a1-4345-a3a2-4e7f956e222d" && iface.generic_args.len() == 1 {
            let elem_type = py_dynwinrt_type(&iface.generic_args[0]);
            out.push_str("    @staticmethod\n");
            out.push_str(&format!(
                "    def create(items: list[DynWinRTValue]) -> '{}':\n",
                iface.name
            ));
            out.push_str(&format!(
                "        return {}(DynWinRTValue.create_vector([getattr(i, '_obj', i) for i in items], {}))\n",
                iface.name, elem_type
            ));
            out.push('\n');
        } else if piid == "3c2925fe-8519-45c1-aa79-197b6718c1c1" && iface.generic_args.len() == 2 {
            let key_type = py_dynwinrt_type(&iface.generic_args[0]);
            let val_type = py_dynwinrt_type(&iface.generic_args[1]);
            out.push_str("    @staticmethod\n");
            out.push_str(&format!(
                "    def create(keys: list[DynWinRTValue], values: list[DynWinRTValue]) -> '{}':\n",
                iface.name
            ));
            out.push_str(&format!(
                "        return {}(DynWinRTValue.create_map([getattr(k, '_obj', k) for k in keys], [getattr(v, '_obj', v) for v in values], {}, {}))\n",
                iface.name, key_type, val_type
            ));
            out.push('\n');
        }
    }

    // Instance methods (reorder so @property comes before @x.setter)
    let iface_var = format!("_{}", iface.name);
    for method in reorder_getters_before_setters(&iface.methods) {
        out.push('\n');
        out.push_str(&generate_iface_instance_method(
            iface,
            &iface_var,
            method,
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
    let param_exprs: Vec<String> = invoke
        .map(|inv| {
            inv.params
                .iter()
                .filter(|p| p.direction == ParamDirection::In)
                .map(|p| py_dynwinrt_type(&p.typ))
                .collect()
        })
        .unwrap_or_default();

    if iface.generic_piid.is_some() {
        out.push_str(&format!(
            "IID_{} = DynWinRTType.parameterized(WinGUID.parse('{}'), [{}]).iid()\n",
            iface.name,
            iface.iid,
            param_exprs.join(", ")
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
}
