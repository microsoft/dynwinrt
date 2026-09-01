// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;
use std::path::Path;

use dynwinrt_codegen::codegen::{python, python_stub};
use dynwinrt_codegen::meta::{
    self, ClassMeta, ConstructorKind, ConstructorMeta, InterfaceMeta, MethodMeta, ParamDirection,
    ParamMeta,
};
use dynwinrt_codegen::types::{TypeKind, TypeMeta, TypeRef};

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

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
        default_interface: Some(InterfaceMeta {
            name: "ISystemResult".into(),
            namespace: "Contoso".into(),
            iid: "22222222-2222-2222-2222-222222222222".into(),
            ..Default::default()
        }),
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
    assert!(py.contains("_dynwinrt_runtime_class_type = True"));
    assert!(!py.contains("from_value = classmethod"), "{py}");
    assert!(py.contains("isinstance(args[0], DynWinRTValue)"));
    assert!(py.contains("SystemResult cannot be constructed directly"));
    assert!(!py.contains("self._set_native(type(self).create("));
    assert!(!py.contains("_IActivationFactory ="));
    assert!(pyi.contains("def __init__(self, _not_constructible: NoReturn) -> None: ..."));
    assert!(pyi.contains("def get_current() -> SystemResult | None: ..."));
    assert!(!pyi.contains("def from_value("), "{pyi}");
    assert!(!pyi.contains("def __init__(self, obj: DynWinRTValue)"));
    assert!(!pyi.contains("def __init__(self)"));
}

#[test]
fn static_only_class_has_only_qi_checked_projection_entry() {
    let class = ClassMeta {
        name: "ApiInformation".into(),
        namespace: "Windows.Foundation.Metadata".into(),
        full_name: "Windows.Foundation.Metadata.ApiInformation".into(),
        default_interface: Some(InterfaceMeta {
            name: "IApiInformation".into(),
            namespace: "Windows.Foundation.Metadata".into(),
            iid: "22222222-2222-2222-2222-222222222222".into(),
            ..Default::default()
        }),
        static_interfaces: vec![factory("IApiInformationStatics", "IsTypePresent", None)],
        ..Default::default()
    };
    let known = HashSet::from(["ApiInformation".into()]);

    let py = python::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let pyi = python_stub::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());

    assert!(
        py.contains("def _from_native(cls, obj: DynWinRTValue):"),
        "{py}"
    );
    assert!(
        py.contains("_dynwinrt_projectable_class_type = True"),
        "{py}"
    );
    assert!(py.contains("self._obj = obj.cast(IID_IApiInformation)"));
    assert!(!py.contains("_dynwinrt_runtime_class_type = True"), "{py}");
    assert!(!py.contains("from_value = classmethod"), "{py}");
    assert!(py.contains("isinstance(args[0], DynWinRTValue)"), "{py}");
    assert!(!py.contains("def as_interface("), "{py}");
    assert!(!pyi.contains("def from_value("), "{pyi}");
    assert!(!pyi.contains("_DynWinRTRuntimeClass"), "{pyi}");
    assert!(pyi.contains("_DynWinRTProjectableClass"), "{pyi}");
    assert!(!pyi.contains("def as_interface("), "{pyi}");

    if Path::new(WINDOWS_WINMD).exists() {
        let signature_types =
            meta::method_signature_type_names(WINDOWS_WINMD).expect("method signature metadata");
        let mut checked = 0;
        for (namespace, name) in [
            ("Windows.Foundation.Metadata", "ApiInformation"),
            ("Windows.System.Profile", "AnalyticsInfo"),
            ("Windows.UI", "Colors"),
            ("Windows.UI", "ColorHelper"),
        ] {
            let Some(mut class) = meta::parse_class(WINDOWS_WINMD, namespace, name) else {
                continue;
            };
            class.is_referenced_as_value =
                signature_types.contains(&(namespace.to_string(), name.to_string()));
            assert!(
                !class.is_referenced_as_value,
                "{namespace}.{name} is not static-only"
            );
            let has_qi_default = class
                .default_interface
                .as_ref()
                .is_some_and(|interface| !interface.iid.is_empty());
            checked += 1;
            let known = HashSet::from([name.into()]);
            let py = python::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
            let pyi =
                python_stub::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());
            assert!(
                !py.contains("from_value = classmethod"),
                "{namespace}.{name}:\n{py}"
            );
            assert!(
                !py.contains("_dynwinrt_runtime_class_type = True"),
                "{namespace}.{name}:\n{py}"
            );
            assert_eq!(
                py.contains("_dynwinrt_projectable_class_type = True"),
                has_qi_default,
                "{namespace}.{name}:\n{py}"
            );
            assert_eq!(
                py.contains("def _from_native(cls, obj: DynWinRTValue):"),
                has_qi_default,
                "{namespace}.{name}:\n{py}"
            );
            assert!(
                !py.contains("def as_interface("),
                "{namespace}.{name}:\n{py}"
            );
            assert!(
                !pyi.contains("_DynWinRTRuntimeClass"),
                "{namespace}.{name}:\n{pyi}"
            );
            assert_eq!(
                pyi.contains("_DynWinRTProjectableClass"),
                has_qi_default,
                "{namespace}.{name}:\n{pyi}"
            );
            assert!(
                !pyi.contains("def as_interface("),
                "{namespace}.{name}:\n{pyi}"
            );
        }
        assert!(checked >= 2, "expected stock Windows static-only classes");
    }
}

#[test]
fn real_factory_constructors_reference_generated_private_helpers() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }

    let mut checked = 0;
    for (namespace, name) in [
        ("Windows.ApplicationModel.Email", "EmailAttachment"),
        ("Windows.Foundation.Diagnostics", "LoggingChannel"),
        ("Windows.Management.Update", "WindowsUpdateManager"),
    ] {
        let Some(class) = meta::parse_class(WINDOWS_WINMD, namespace, name) else {
            continue;
        };
        checked += 1;
        let code = python::generate_class(
            &class,
            &HashSet::from([name.into()]),
            &HashSet::new(),
            &HashSet::new(),
        );
        for line in code.lines() {
            let Some(call) = line.trim().strip_prefix("return type(self).") else {
                continue;
            };
            let helper = call.split('(').next().expect("constructor helper call");
            if helper.starts_with('_') {
                assert!(
                    code.contains(&format!("def {helper}(")),
                    "{namespace}.{name} constructor references missing {helper}:\n{code}"
                );
            }
        }
    }
    assert!(checked >= 2, "expected real constructor metadata");
}

#[test]
fn externally_returned_empty_class_keeps_native_projection() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }
    let mut class = meta::parse_class(
        WINDOWS_WINMD,
        "Windows.ApplicationModel.Background",
        "DeviceWatcherTrigger",
    )
    .expect("DeviceWatcherTrigger metadata");
    assert!(class.static_interfaces.is_empty());
    let signature_types =
        meta::method_signature_type_names(WINDOWS_WINMD).expect("method signature metadata");
    assert!(signature_types.contains(&(
        "Windows.ApplicationModel.Background".into(),
        "DeviceWatcherTrigger".into()
    )));
    class.is_referenced_as_value = true;
    let known = HashSet::from(["DeviceWatcherTrigger".into()]);
    let py = python::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let pyi = python_stub::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());

    assert!(py.contains("_dynwinrt_runtime_class_type = True"), "{py}");
    assert!(
        py.contains("def _from_native(cls, obj: DynWinRTValue):"),
        "{py}"
    );
    assert!(pyi.contains("_DynWinRTRuntimeClass"), "{pyi}");
}

#[test]
fn externally_returned_empty_class_with_static_interface_keeps_native_projection() {
    let mut class = ClassMeta {
        name: "ExternallyReturned".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.ExternallyReturned".into(),
        default_interface: Some(InterfaceMeta {
            name: "IExternallyReturned".into(),
            namespace: "Contoso".into(),
            iid: "22222222-2222-2222-2222-222222222222".into(),
            ..Default::default()
        }),
        static_interfaces: vec![InterfaceMeta {
            name: "IExternallyReturnedStatics".into(),
            namespace: "Contoso".into(),
            iid: "33333333-3333-3333-3333-333333333333".into(),
            methods: vec![MethodMeta {
                name: "IsSupported".into(),
                return_type: Some(TypeMeta::Bool),
                ..Default::default()
            }],
            ..Default::default()
        }],
        is_referenced_as_value: true,
        ..Default::default()
    };
    let known = HashSet::from(["ExternallyReturned".into()]);

    let py = python::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let pyi = python_stub::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());
    assert!(
        py.contains("def _from_native(cls, obj: DynWinRTValue):"),
        "{py}"
    );
    assert!(py.contains("_dynwinrt_runtime_class_type = True"), "{py}");
    assert!(pyi.contains("_DynWinRTRuntimeClass"), "{pyi}");

    class.is_referenced_as_value = false;
    let static_only = python::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    assert!(static_only.contains("def _from_native("), "{static_only}");
    assert!(
        static_only.contains("_dynwinrt_projectable_class_type = True"),
        "{static_only}"
    );
    assert!(
        !static_only.contains("_dynwinrt_runtime_class_type"),
        "{static_only}"
    );
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
    assert!(pyi.contains("def __init__(self, _not_constructible: NoReturn) -> None: ..."));
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
        assert!(pyi.contains("def __init__(self, _not_constructible: NoReturn) -> None: ..."));
        assert_eq!(pyi.matches("def __init__(self").count(), 1);
    }
}
