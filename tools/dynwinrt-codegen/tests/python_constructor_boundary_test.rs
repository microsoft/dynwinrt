// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;

use dynwinrt_codegen::codegen::{python, python_stub};
use dynwinrt_codegen::meta::{
    ClassMeta, ConstructorKind, ConstructorMeta, InterfaceMeta, MethodMeta, ParamDirection,
    ParamMeta,
};
use dynwinrt_codegen::types::{TypeKind, TypeMeta, TypeRef};

fn runtime_class(name: &str) -> TypeMeta {
    TypeMeta::RuntimeClass {
        namespace: "Contoso".into(),
        name: name.into(),
        default_interface: None,
    }
}

fn factory(name: &str, method_name: &str, parameter: Option<ParamMeta>) -> InterfaceMeta {
    InterfaceMeta {
        name: name.into(),
        namespace: "Contoso".into(),
        iid: "11111111-1111-1111-1111-111111111111".into(),
        methods: vec![MethodMeta {
            name: method_name.into(),
            raw_name: method_name.into(),
            vtable_index: 6,
            params: parameter.into_iter().collect(),
            return_type: Some(runtime_class("SystemResult")),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn system_returned_class_keeps_only_internal_native_wrapping() {
    let class = ClassMeta {
        name: "SystemResult".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.SystemResult".into(),
        // Deliberately contradictory legacy state and an unrelated factory:
        // neither is authoritative without a ConstructorMeta declaration.
        has_default_constructor: true,
        factory_interfaces: vec![factory("ISystemResultFactory", "Create", None)],
        static_interfaces: vec![factory("ISystemResultStatics", "GetCurrent", None)],
        ..Default::default()
    };
    let known = HashSet::from(["SystemResult".into()]);

    let py = python::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let pyi = python_stub::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());

    assert!(py.contains("def _from_native(cls, obj: DynWinRTValue):"));
    assert!(py.contains("isinstance(args[0], DynWinRTValue)"));
    assert!(py.contains("SystemResult cannot be constructed directly"));
    assert!(!py.contains("self._set_native(type(self).create("));
    assert!(!py.contains("_IActivationFactory ="));
    assert!(pyi.contains("def __new__(cls, _not_constructible: NoReturn) -> NoReturn: ..."));
    assert!(!pyi.contains("def __init__(self, obj: DynWinRTValue)"));
    assert!(!pyi.contains("def __init__(self)"));
}

#[test]
fn only_referenced_public_factory_metadata_becomes_a_constructor() {
    let activation_factory = factory(
        "IResultActivationFactory",
        "CreateResult",
        Some(ParamMeta {
            name: "value".into(),
            typ: TypeMeta::String,
            direction: ParamDirection::In,
        }),
    );
    let unrelated_factory = factory("IUnrelatedFactory", "Create", None);
    let class = ClassMeta {
        name: "SystemResult".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.SystemResult".into(),
        factory_interfaces: vec![activation_factory, unrelated_factory],
        constructors: vec![ConstructorMeta {
            kind: ConstructorKind::FactoryActivation,
            factory_interface: Some(TypeRef {
                namespace: "Contoso".into(),
                name: "IResultActivationFactory".into(),
                kind: TypeKind::Interface,
            }),
        }],
        ..Default::default()
    };
    let known = HashSet::from(["SystemResult".into()]);

    let py = python::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let pyi = python_stub::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());

    assert!(py.contains("type(self).create_result(_bound[0])"));
    assert!(!py.contains("self._set_native(type(self).create()._obj)"));
    assert!(pyi.contains("def __init__(self, value: str) -> None: ..."));
    assert!(!pyi.contains("def __init__(self) -> None: ..."));
}

#[test]
fn numeric_constructor_overloads_dispatch_by_specificity() {
    let parameter = |typ| ParamMeta {
        name: "value".into(),
        typ,
        direction: ParamDirection::In,
    };
    let factory = InterfaceMeta {
        name: "ISystemResultFactory".into(),
        namespace: "Contoso".into(),
        iid: "11111111-1111-1111-1111-111111111111".into(),
        methods: vec![
            MethodMeta {
                name: "Create2".into(),
                raw_name: "Create2".into(),
                vtable_index: 7,
                params: vec![parameter(TypeMeta::I32)],
                return_type: Some(runtime_class("SystemResult")),
                ..Default::default()
            },
            MethodMeta {
                name: "Create".into(),
                raw_name: "Create".into(),
                vtable_index: 6,
                params: vec![parameter(TypeMeta::I8)],
                return_type: Some(runtime_class("SystemResult")),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let class = ClassMeta {
        name: "SystemResult".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.SystemResult".into(),
        factory_interfaces: vec![factory],
        constructors: vec![ConstructorMeta {
            kind: ConstructorKind::FactoryActivation,
            factory_interface: Some(TypeRef {
                namespace: "Contoso".into(),
                name: "ISystemResultFactory".into(),
                kind: TypeKind::Interface,
            }),
        }],
        ..Default::default()
    };
    let known = HashSet::from(["SystemResult".into()]);

    let py = python::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let narrow_guard = "-128 <= _bound[0] <= 127";
    let wide_guard = "-2147483648 <= _bound[0] <= 2147483647";
    assert!(
        py.find(narrow_guard).expect("narrow constructor guard")
            < py.find(wide_guard).expect("wide constructor guard"),
        "{py}"
    );

    let pyi = python_stub::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());
    assert_eq!(pyi.matches("    @overload\n").count(), 4, "{pyi}");
}

#[test]
fn protected_composition_is_not_public_construction() {
    let class = ClassMeta {
        name: "SystemResult".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.SystemResult".into(),
        factory_interfaces: vec![factory("ISystemResultFactory", "CreateInstance", None)],
        constructors: vec![ConstructorMeta {
            kind: ConstructorKind::ProtectedComposition,
            factory_interface: Some(TypeRef {
                namespace: "Contoso".into(),
                name: "ISystemResultFactory".into(),
                kind: TypeKind::Interface,
            }),
        }],
        ..Default::default()
    };
    let known = HashSet::from(["SystemResult".into()]);

    let py = python::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let pyi = python_stub::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());

    assert!(py.contains("SystemResult cannot be constructed directly"));
    assert!(pyi.contains("def __new__(cls, _not_constructible: NoReturn) -> NoReturn: ..."));
    assert!(!pyi.contains("def create() -> 'SystemResult'"));
}

#[test]
fn unresolved_or_unsupported_constructor_metadata_fails_closed() {
    let known = HashSet::from(["SystemResult".into()]);
    let missing_dependency = ClassMeta {
        name: "SystemResult".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.SystemResult".into(),
        constructors: vec![ConstructorMeta {
            kind: ConstructorKind::FactoryActivation,
            factory_interface: Some(TypeRef {
                namespace: "Missing.Dependency".into(),
                name: "IResultFactory".into(),
                kind: TypeKind::Interface,
            }),
        }],
        ..Default::default()
    };
    let malformed_composition = ClassMeta {
        name: "SystemResult".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.SystemResult".into(),
        default_interface: Some(InterfaceMeta {
            name: "ISystemResult".into(),
            namespace: "Contoso".into(),
            iid: "22222222-2222-2222-2222-222222222222".into(),
            ..Default::default()
        }),
        factory_interfaces: vec![factory("IResultFactory", "CreateInstance", None)],
        constructors: vec![ConstructorMeta {
            kind: ConstructorKind::PublicComposition,
            factory_interface: Some(TypeRef {
                namespace: "Contoso".into(),
                name: "IResultFactory".into(),
                kind: TypeKind::Interface,
            }),
        }],
        ..Default::default()
    };

    for class in [&missing_dependency, &malformed_composition] {
        let py = python::generate_class(class, &known, &HashSet::new(), &HashSet::new());
        let pyi = python_stub::generate_class_stub(class, &known, &HashSet::new(), &HashSet::new());
        assert!(py.contains("SystemResult cannot be constructed directly"));
        assert!(pyi.contains("from typing import NoReturn"));
        assert!(pyi.contains("def __new__(cls, _not_constructible: NoReturn) -> NoReturn: ..."));
        assert!(!pyi.contains("def __init__(self"));
    }
}
