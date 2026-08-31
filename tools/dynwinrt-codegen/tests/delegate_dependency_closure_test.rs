// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{HashMap, HashSet};

use dynwinrt_codegen::codegen::{project, python, python_stub, render_dts, render_js};
use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::TypeMeta;

fn delegate_interface(name: &str) -> TypeMeta {
    TypeMeta::Interface {
        namespace: "Contoso".into(),
        name: name.into(),
        iid: String::new(),
    }
}

#[test]
fn generated_interfaces_import_only_runtime_delegates() {
    let interface = InterfaceMeta {
        name: "IDataSource".into(),
        namespace: "Contoso".into(),
        iid: "11111111-1111-1111-1111-111111111111".into(),
        methods: vec![
            MethodMeta {
                name: "SetDataProvider".into(),
                raw_name: "SetDataProvider".into(),
                vtable_index: 6,
                params: vec![ParamMeta {
                    name: "callback".into(),
                    typ: delegate_interface("DataProviderHandler"),
                    direction: ParamDirection::In,
                }],
                ..Default::default()
            },
            MethodMeta {
                name: "GetCallbacksAsync".into(),
                raw_name: "GetCallbacksAsync".into(),
                vtable_index: 7,
                return_type: Some(TypeMeta::AsyncOperation(Box::new(TypeMeta::Array(
                    Box::new(delegate_interface("AsyncHandler")),
                )))),
                ..Default::default()
            },
            MethodMeta {
                name: "GetCallback".into(),
                raw_name: "GetCallback".into(),
                vtable_index: 8,
                return_type: Some(delegate_interface("AsyncHandler")),
                ..Default::default()
            },
            MethodMeta {
                name: "GetCallbackOut".into(),
                raw_name: "GetCallbackOut".into(),
                vtable_index: 9,
                params: vec![ParamMeta {
                    name: "callback".into(),
                    typ: delegate_interface("AsyncHandler"),
                    direction: ParamDirection::Out,
                }],
                ..Default::default()
            },
            MethodMeta {
                name: "get_Callbacks".into(),
                raw_name: "get_Callbacks".into(),
                vtable_index: 10,
                return_type: Some(TypeMeta::Array(Box::new(delegate_interface(
                    "AsyncHandler",
                )))),
                is_property_getter: true,
                ..Default::default()
            },
            MethodMeta {
                name: "GetDataProvider".into(),
                raw_name: "GetDataProvider".into(),
                vtable_index: 11,
                return_type: Some(delegate_interface("DataProviderHandler")),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let known_types = HashSet::from([
        "IDataSource".into(),
        "DataProviderHandler".into(),
        "AsyncHandler".into(),
        "UnrelatedHandler".into(),
    ]);
    let delegates = HashSet::from([
        "DataProviderHandler".into(),
        "AsyncHandler".into(),
        "UnrelatedHandler".into(),
    ]);
    let signatures = HashMap::from([
        (
            "DataProviderHandler".into(),
            "(value: unknown) => void".into(),
        ),
        ("AsyncHandler".into(), "() => void".into()),
        ("UnrelatedHandler".into(), "() => void".into()),
    ]);
    let projected = project::project_interface(
        &Default::default(),
        &interface,
        &known_types,
        &delegates,
        &signatures,
        &HashMap::new(),
        &HashMap::new(),
    );
    let js = render_js::render(&projected);
    let dts = render_dts::render(&projected);
    let py = python::generate_interface(&interface, &known_types, &delegates);
    let pyi = python_stub::generate_interface_stub(&interface, &known_types, &delegates);

    assert_eq!(
        js.matches("require('./DataProviderHandler.js')").count(),
        1,
        "{js}",
    );
    assert_eq!(
        py.matches("from .data_provider_handler import").count(),
        1,
        "{py}",
    );
    assert_eq!(
        pyi.matches("from .data_provider_handler import").count(),
        1,
        "{pyi}",
    );
    for generated in [&js, &py, &pyi] {
        assert!(
            !generated.contains("AsyncHandler") && !generated.contains("async_handler"),
            "{generated}",
        );
        assert!(
            !generated.contains("UnrelatedHandler") && !generated.contains("unrelated_handler"),
            "{generated}",
        );
    }

    assert!(
        js.contains(".asArray().toValues().map(v => v.isNull() ? null : v)",)
            && !js.contains("new AsyncHandler")
            && !js.contains("__get_AsyncHandler"),
        "{js}",
    );
    assert!(
        js.contains("getCallback()")
            && js.contains("((v) => v.isNull() ? null : v)",)
            && js.contains("getCallbackOut()"),
        "{js}",
    );
    assert!(
        dts.contains("Promise<Array<DynWinRtValue | null>>",)
            && dts.contains("getCallback(): DynWinRtValue | null",)
            && dts.contains("getCallbackOut(): DynWinRtValue | null",)
            && dts.contains("get callbacks(): Array<DynWinRtValue | null>",)
            && dts.contains("getDataProvider(): DynWinRtValue | null",)
            && !dts.contains("Promise<AsyncHandler[]>"),
        "{dts}",
    );
    assert!(
        py.contains("WinRTAsync[list[DynWinRTValue | None]]",)
            && py.contains("value.as_array().to_values()",)
            && py.contains("def callbacks(self) -> list[DynWinRTValue | None]:",)
            && !py.contains("WinRTAsync[list[AsyncHandler | None]]",),
        "{py}",
    );
    assert!(
        pyi.contains("WinRTAsync[list[DynWinRTValue | None]]",)
            && pyi.contains("def callbacks(self) -> list[DynWinRTValue | None]:",)
            && !pyi.contains("WinRTAsync[list[AsyncHandler | None]]",),
        "{pyi}",
    );
}

#[test]
fn generated_classes_do_not_import_output_only_delegates() {
    let class = ClassMeta {
        name: "CallbackSource".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.CallbackSource".into(),
        default_interface: Some(InterfaceMeta {
            name: "ICallbackSource".into(),
            namespace: "Contoso".into(),
            iid: "33333333-3333-3333-3333-333333333333".into(),
            methods: vec![
                MethodMeta {
                    name: "SetHandler".into(),
                    raw_name: "SetHandler".into(),
                    vtable_index: 6,
                    params: vec![ParamMeta {
                        name: "callback".into(),
                        typ: delegate_interface("InputHandler"),
                        direction: ParamDirection::In,
                    }],
                    ..Default::default()
                },
                MethodMeta {
                    name: "GetHandlerAsync".into(),
                    raw_name: "GetHandlerAsync".into(),
                    vtable_index: 7,
                    return_type: Some(TypeMeta::AsyncOperation(Box::new(delegate_interface(
                        "OutputHandler",
                    )))),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        ..Default::default()
    };
    let known_types = HashSet::from([
        "CallbackSource".into(),
        "ICallbackSource".into(),
        "InputHandler".into(),
        "OutputHandler".into(),
    ]);
    let delegates = HashSet::from(["InputHandler".into(), "OutputHandler".into()]);
    let signatures = HashMap::from([
        ("InputHandler".into(), "() => void".into()),
        ("OutputHandler".into(), "() => void".into()),
    ]);
    let projected = project::project_class(
        &Default::default(),
        &class,
        &known_types,
        &delegates,
        &HashSet::new(),
        &signatures,
        &HashMap::new(),
        &HashMap::new(),
    );
    let js = render_js::render(&projected);
    let dts = render_dts::render(&projected);
    let py = python::generate_class(&class, &known_types, &delegates, &HashSet::new());
    let pyi = python_stub::generate_class_stub(&class, &known_types, &delegates, &HashSet::new());

    assert!(
        js.contains("require('./InputHandler.js')")
            && !js.contains("require('./OutputHandler.js')"),
        "{js}",
    );
    assert!(
        dts.contains("getHandlerAsync(signal?: AbortSignal): Promise<DynWinRtValue | null>",),
        "{dts}",
    );
    for generated in [&py, &pyi] {
        assert!(
            generated.contains("from .input_handler import",)
                && !generated.contains("from .output_handler import",),
            "{generated}",
        );
    }
}

#[test]
fn delegate_free_interfaces_do_not_import_global_delegates() {
    let interface = InterfaceMeta {
        name: "IValue".into(),
        namespace: "Contoso".into(),
        iid: "22222222-2222-2222-2222-222222222222".into(),
        ..Default::default()
    };
    let known_types = HashSet::from(["IValue".into(), "UnrelatedHandler".into()]);
    let delegates = HashSet::from(["UnrelatedHandler".into()]);
    let projected = project::project_interface(
        &Default::default(),
        &interface,
        &known_types,
        &delegates,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );

    assert!(!render_js::render(&projected).contains("UnrelatedHandler"),);
    assert!(
        !python::generate_interface(&interface, &known_types, &delegates,)
            .contains("unrelated_handler"),
    );
    assert!(
        !python_stub::generate_interface_stub(&interface, &known_types, &delegates,)
            .contains("unrelated_handler"),
    );
}
