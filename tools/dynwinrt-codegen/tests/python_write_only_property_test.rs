// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod common;

use std::collections::HashSet;

use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::TypeMeta;

#[test]
fn versioned_setter_uses_main_property_but_standalone_set_method() {
    let getter = MethodMeta {
        name: "get_Value".into(),
        raw_name: "get_Value".into(),
        vtable_index: 6,
        return_type: Some(TypeMeta::I32),
        is_property_getter: true,
        ..Default::default()
    };
    let setter = MethodMeta {
        name: "put_Value".into(),
        raw_name: "put_Value".into(),
        vtable_index: 6,
        params: vec![ParamMeta {
            name: "value".into(),
            typ: TypeMeta::I32,
            direction: ParamDirection::In,
        }],
        is_property_setter: true,
        ..Default::default()
    };
    let class = ClassMeta {
        name: "Widget".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.Widget".into(),
        default_interface: Some(InterfaceMeta {
            name: "IWidget".into(),
            namespace: "Contoso".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
            methods: vec![getter],
            ..Default::default()
        }),
        required_interfaces: vec![InterfaceMeta {
            name: "IWidget2".into(),
            namespace: "Contoso".into(),
            iid: "22222222-2222-2222-2222-222222222222".into(),
            methods: vec![setter],
            ..Default::default()
        }],
        ..Default::default()
    };
    let known = HashSet::from(["Widget".into()]);

    let py = common::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let pyi = common::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());

    let inline_py = py
        .split("class IWidget2:")
        .nth(1)
        .expect("inline interface implementation");
    assert!(py.contains("@value.setter"));
    assert!(inline_py.contains("def set_value(self, value: int):"));
    assert!(!inline_py.contains("@value.setter"));

    let inline_pyi = pyi
        .split("class IWidget2:")
        .nth(1)
        .expect("inline interface stub");
    assert!(pyi.contains("@value.setter"));
    assert!(inline_pyi.contains("def set_value(self, value: int) -> None: ..."));
    assert!(!inline_pyi.contains("@value.setter"));
}

#[test]
fn cross_interface_getter_is_emitted_before_an_earlier_setter() {
    let setter = MethodMeta {
        name: "put_Value".into(),
        raw_name: "put_Value".into(),
        vtable_index: 6,
        params: vec![ParamMeta {
            name: "value".into(),
            typ: TypeMeta::I32,
            direction: ParamDirection::In,
        }],
        is_property_setter: true,
        ..Default::default()
    };
    let getter = MethodMeta {
        name: "get_Value".into(),
        raw_name: "get_Value".into(),
        vtable_index: 6,
        return_type: Some(TypeMeta::I32),
        is_property_getter: true,
        ..Default::default()
    };
    let class = ClassMeta {
        name: "Widget".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.Widget".into(),
        default_interface: Some(InterfaceMeta {
            name: "IWidget".into(),
            namespace: "Contoso".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
            methods: vec![setter],
            ..Default::default()
        }),
        required_interfaces: vec![InterfaceMeta {
            name: "IWidget2".into(),
            namespace: "Contoso".into(),
            iid: "22222222-2222-2222-2222-222222222222".into(),
            methods: vec![getter],
            ..Default::default()
        }],
        ..Default::default()
    };
    let known = HashSet::from(["Widget".into()]);

    let py = common::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let pyi = common::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());

    assert!(
        py.find("    @_property\n    def value")
            .expect("runtime getter")
            < py.find("    @value.setter\n    def value")
                .expect("runtime setter"),
        "runtime getter must precede its cross-interface setter:\n{py}"
    );
    assert!(
        pyi.find("    @builtins.property\n    def value")
            .expect("stub getter")
            < pyi
                .find("    @value.setter\n    def value")
                .expect("stub setter"),
        "stub getter must precede its cross-interface setter:\n{pyi}"
    );
}
