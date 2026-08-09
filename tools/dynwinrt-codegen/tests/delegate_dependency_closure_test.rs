// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{HashMap, HashSet};

use dynwinrt_codegen::codegen::{
    project, python, python_stub, render_js,
};
use dynwinrt_codegen::meta::{
    InterfaceMeta, MethodMeta, ParamDirection, ParamMeta,
};
use dynwinrt_codegen::types::TypeMeta;

fn delegate_interface(name: &str) -> TypeMeta {
    TypeMeta::Interface {
        namespace: "Contoso".into(),
        name: name.into(),
        iid: String::new(),
    }
}

#[test]
fn generated_interfaces_import_only_referenced_delegates() {
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
                return_type: Some(TypeMeta::AsyncOperation(
                    Box::new(TypeMeta::Array(Box::new(
                        delegate_interface("AsyncHandler"),
                    ))),
                )),
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
        (
            "AsyncHandler".into(),
            "() => void".into(),
        ),
        (
            "UnrelatedHandler".into(),
            "() => void".into(),
        ),
    ]);
    let projected = project::project_interface(
        &interface,
        &known_types,
        &delegates,
        &signatures,
        &HashMap::new(),
        &HashMap::new(),
    );
    let js = render_js::render(&projected);
    let py = python::generate_interface(
        &interface,
        &known_types,
        &delegates,
    );
    let pyi = python_stub::generate_interface_stub(
        &interface,
        &known_types,
        &delegates,
    );

    for generated in [&js, &py, &pyi] {
        assert!(
            generated.contains("DataProviderHandler")
                || generated.contains("data_provider_handler"),
            "{generated}",
        );
        assert!(
            generated.contains("AsyncHandler")
                || generated.contains("async_handler"),
            "{generated}",
        );
        assert!(
            !generated.contains("UnrelatedHandler")
                && !generated.contains("unrelated_handler"),
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
    let known_types = HashSet::from([
        "IValue".into(),
        "UnrelatedHandler".into(),
    ]);
    let delegates =
        HashSet::from(["UnrelatedHandler".into()]);
    let projected = project::project_interface(
        &interface,
        &known_types,
        &delegates,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );

    assert!(
        !render_js::render(&projected)
            .contains("UnrelatedHandler"),
    );
    assert!(
        !python::generate_interface(
            &interface,
            &known_types,
            &delegates,
        )
        .contains("unrelated_handler"),
    );
    assert!(
        !python_stub::generate_interface_stub(
            &interface,
            &known_types,
            &delegates,
        )
        .contains("unrelated_handler"),
    );
}
