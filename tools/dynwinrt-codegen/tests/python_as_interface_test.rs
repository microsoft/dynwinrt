// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod common;

use std::collections::HashSet;

use dynwinrt_codegen::codegen::python::generate_runtime_support_module;
use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta};

const AS_INTERFACE: &str = "    def as_interface(self, interface_class):
        return _dynwinrt_as_interface(self._obj, interface_class)
";

fn interface(name: &str, iid: &str) -> InterfaceMeta {
    InterfaceMeta {
        name: name.into(),
        namespace: "Contoso".into(),
        iid: iid.into(),
        ..Default::default()
    }
}

fn assert_imports_helper(module: &str) {
    let (_, imports) = module
        .split_once("from ._runtime import (")
        .expect("runtime support import");
    let (imports, _) = imports.split_once(')').expect("closed import list");
    assert!(
        imports
            .split(',')
            .any(|name| name.trim() == "_dynwinrt_as_interface"),
        "{module}"
    );
}

#[test]
fn every_as_interface_delegates_to_the_runtime_support_helper() {
    let class = ClassMeta {
        name: "Widget".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.Widget".into(),
        default_interface: Some(interface("IWidget", "11111111-1111-1111-1111-111111111111")),
        required_interfaces: vec![interface("IExtra", "22222222-2222-2222-2222-222222222222")],
        ..Default::default()
    };
    let known = HashSet::from(["Widget".to_string()]);
    let py = common::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    assert_imports_helper(&py);
    let (runtime_class, embedded_interface) = py
        .split_once("\nclass IExtra:")
        .expect("embedded runtime interface");
    assert!(
        runtime_class.contains("_dynwinrt_runtime_class_type = True")
            && runtime_class.contains(AS_INTERFACE),
        "{runtime_class}"
    );
    assert!(
        embedded_interface.contains(AS_INTERFACE),
        "{embedded_interface}"
    );

    let py = common::generate_interface(
        &interface("IWidget", "11111111-1111-1111-1111-111111111111"),
        &HashSet::from(["IWidget".to_string()]),
        &HashSet::new(),
    );
    assert_imports_helper(&py);
    assert!(py.contains(AS_INTERFACE), "{py}");
}

#[test]
fn as_interface_helper_rejects_runtime_classes_with_project_as_guidance() {
    let runtime = generate_runtime_support_module();
    let helper = runtime
        .split_once("def _dynwinrt_as_interface(native, interface_class):\n")
        .map(|(_, helper)| helper.split("\n\n\n").next().unwrap_or(helper))
        .expect("as_interface helper");
    for expected in [
        "from_value = getattr(interface_class, 'from_value', None)",
        "return from_value(native)",
        "_dynwinrt_runtime_class_type",
        "_dynwinrt_projectable_class_type",
        "raise TypeError(",
        "as_interface() requires a generated interface class, but {name} is a",
        "Use dynwinrt.project_as(obj, {name}) to cast to a runtime class.",
        "as_interface() requires a generated interface class, not {name}.",
    ] {
        assert!(helper.contains(expected), "missing {expected:?}:\n{helper}");
    }
}
