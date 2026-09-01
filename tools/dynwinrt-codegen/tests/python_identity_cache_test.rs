// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;

use dynwinrt_codegen::codegen::{python, python_stub};
use dynwinrt_codegen::meta::{
    ClassMeta, ConstructorKind, ConstructorMeta, InterfaceMeta, MethodMeta, ParamDirection,
    ParamMeta,
};
use dynwinrt_codegen::types::{TypeKind, TypeMeta, TypeRef};

#[test]
fn runtime_class_generation_uses_projected_identity_cache() {
    let class = ClassMeta {
        name: "Widget".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.Widget".into(),
        default_interface: Some(InterfaceMeta {
            name: "IWidget".into(),
            namespace: "Contoso".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
            methods: vec![MethodMeta {
                name: "get_Name".into(),
                raw_name: "get_Name".into(),
                vtable_index: 6,
                return_type: Some(TypeMeta::String),
                is_property_getter: true,
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    };

    let py = python::generate_class(
        &class,
        &HashSet::from(["Widget".to_string()]),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(
        py.contains("_dynwinrt_projected_from_native"),
        "missing projected identity helper import:\n{py}"
    );
    assert!(
        py.contains("_dynwinrt_cache_projected"),
        "missing projected cache helper import:\n{py}"
    );
    assert!(
        py.contains("def __new__(cls, *args, **kwargs):"),
        "missing native-wrap __new__:\n{py}"
    );
    assert!(
        py.contains("return _dynwinrt_projected_from_native(cls, args[0], '_set_native')"),
        "missing cached native-wrap path:\n{py}"
    );
    assert!(
        py.contains("self._dynwinrt_native_ready = True"),
        "missing native initialization flag:\n{py}"
    );
    assert!(
        py.contains("_dynwinrt_cache_projected(self)"),
        "missing projected cache registration:\n{py}"
    );
    assert!(
        py.contains("def _from_native(cls, obj: DynWinRTValue):\n        return cls(obj)"),
        "missing cached _from_native helper:\n{py}"
    );
}

#[test]
fn runtime_class_public_constructor_registers_final_self() {
    let class = ClassMeta {
        name: "Widget".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.Widget".into(),
        default_interface: Some(InterfaceMeta {
            name: "IWidget".into(),
            namespace: "Contoso".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
            ..Default::default()
        }),
        factory_interfaces: vec![InterfaceMeta {
            name: "IWidgetFactory".into(),
            namespace: "Contoso".into(),
            iid: "22222222-2222-2222-2222-222222222222".into(),
            methods: vec![MethodMeta {
                name: "CreateWidget".into(),
                raw_name: "CreateWidget".into(),
                vtable_index: 6,
                params: vec![ParamMeta {
                    name: "name".into(),
                    typ: TypeMeta::String,
                    direction: ParamDirection::In,
                }],
                return_type: Some(TypeMeta::RuntimeClass {
                    namespace: "Contoso".into(),
                    name: "Widget".into(),
                    default_interface: None,
                }),
                ..Default::default()
            }],
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
    };

    let py = python::generate_class(
        &class,
        &HashSet::from(["Widget".to_string()]),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(
        py.contains("return cls.create_widget(_bound[0])"),
        "exact-class constructors must return the cached factory wrapper:\n{py}"
    );
    assert!(
        py.contains("self._set_native(type(self).create_widget(_bound[0])._obj)"),
        "subclass constructors must retain the self-binding fallback:\n{py}"
    );
    assert!(
        py.contains("_dynwinrt_cache_projected(self)"),
        "public constructors must register the final self:\n{py}"
    );
}

#[test]
fn interface_generation_uses_projected_identity_cache() {
    let iface = InterfaceMeta {
        name: "IWidget".into(),
        namespace: "Contoso".into(),
        iid: "11111111-1111-1111-1111-111111111111".into(),
        ..Default::default()
    };

    let py = python::generate_interface(
        &iface,
        &HashSet::from(["IWidget".to_string()]),
        &HashSet::new(),
    );

    assert!(
        py.contains("def __new__(cls, *args, **kwargs):"),
        "missing native-wrap __new__:\n{py}"
    );
    assert!(
        py.contains("def _set_native(self, obj: DynWinRTValue):"),
        "missing native initializer:\n{py}"
    );
    assert!(
        py.contains("_dynwinrt_cache_projected(self)"),
        "interfaces should register cache entries for initialized wrappers:\n{py}"
    );
    assert!(
        py.contains(
            "def _from_native(cls, obj: DynWinRTValue) -> 'IWidget':\n        return cls(obj)"
        ),
        "missing cached _from_native helper:\n{py}"
    );
    assert!(
        py.contains("return cls._from_native(obj.cast(IID_IWidget))"),
        "from_value should reuse the cached wrapper path:\n{py}"
    );
}

#[test]
fn embedded_interface_projection_preserves_subclasses_and_qi_helpers() {
    let class = ClassMeta {
        name: "Widget".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.Widget".into(),
        default_interface: Some(InterfaceMeta {
            name: "IWidget".into(),
            namespace: "Contoso".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
            ..Default::default()
        }),
        required_interfaces: vec![InterfaceMeta {
            name: "IExtra".into(),
            namespace: "Contoso".into(),
            iid: "22222222-2222-2222-2222-222222222222".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let known = HashSet::from(["Widget".to_string()]);
    let py = python::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let pyi = python_stub::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());
    let inline = py
        .split("\nclass IExtra:")
        .nth(1)
        .expect("embedded runtime interface");
    let inline_stub = pyi
        .split("\nclass IExtra:")
        .nth(1)
        .expect("embedded stub interface");

    assert!(
        inline.contains("@classmethod\n    def from_value(cls, obj: DynWinRTValue)"),
        "{inline}"
    );
    assert!(
        inline.contains("_dynwinrt_interface_type = True")
            && inline.contains("_dynwinrt_interface_iid = IID_IExtra"),
        "{inline}"
    );
    assert!(
        inline.contains("return cls._from_native(obj.cast(IID_IExtra))"),
        "{inline}"
    );
    assert!(
        inline.contains("def as_interface(self, interface_class):"),
        "{inline}"
    );
    assert!(
        inline_stub.contains("@classmethod\n    def from_value(cls, obj: DynWinRTValue) -> Self:"),
        "{inline_stub}"
    );
    assert!(
        inline_stub.contains(
            "def as_interface(self, interface_class: _DynWinRTProjector[_InterfaceT]) -> _InterfaceT:"
        ),
        "{inline_stub}"
    );
}
