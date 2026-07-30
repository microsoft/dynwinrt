// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{HashMap, HashSet};

use dynwinrt_codegen::codegen::{project, render_dts, render_js};
use dynwinrt_codegen::meta::{InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::TypeMeta;

#[test]
fn observable_vector_projects_mutable_create_helper() {
    let interface = InterfaceMeta {
        name: "IObservableVector_Object".into(),
        namespace: "Windows.Foundation.Collections".into(),
        iid: String::new(),
        generic_piid: Some("5917eb53-50b4-4a0d-b309-65862b3f1dbc".into()),
        generic_args: vec![TypeMeta::Object],
        ..Default::default()
    };
    let projected = project::project_interface(
        &interface,
        &HashSet::from(["IObservableVector_Object".into(), "IVector_Object".into()]),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let js = render_js::render(&projected);
    let dts = render_dts::render(&projected);

    assert!(js.contains("DynWinRtValue.createVector"));
    assert!(
        js.contains("new ((__load_IVector_Object()).IVector_Object)(value)",),
        "{js}",
    );
    assert!(js.contains("onVectorChanged"));
    assert!(
        js.contains("asVector: { value: observable.asVector.bind(observable) }"),
        "{js}",
    );
    assert!(js.contains(
        "asVector() {\n        return new ((__load_IVector_Object()).IVector_Object)(this._obj);",
    ));
    assert!(dts.contains("asVector(): IVector_Object;"));
    assert!(dts.contains(
        "static create(items: unknown[]): IObservableVector_Object & IVector_Object;",
    ));
}

#[test]
fn generic_delegate_iid_uses_declared_type_arguments() {
    let interface = InterfaceMeta {
        name: "VectorChangedEventHandler_Object".into(),
        namespace: "Windows.Foundation.Collections".into(),
        iid: "0c051752-9fbf-4c70-aa0c-0e4c82d9a761".into(),
        generic_args: vec![TypeMeta::Object],
        methods: vec![MethodMeta {
            name: "Invoke".into(),
            raw_name: "Invoke".into(),
            vtable_index: 3,
            params: vec![
                ParamMeta {
                    name: "sender".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "args".into(),
                    typ: TypeMeta::Interface {
                        namespace: "Windows.Foundation.Collections".into(),
                        name: "IVectorChangedEventArgs".into(),
                        iid: "575933df-34fe-4480-af15-07691f3d5d9b".into(),
                    },
                    direction: ParamDirection::In,
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    let projected = project::project_delegate(&interface, &HashMap::new(), &HashMap::new());
    let js = render_js::render(&projected);

    assert!(js.contains(
        "DynWinRtType.parameterized(WinGuid.parse('0c051752-9fbf-4c70-aa0c-0e4c82d9a761'), [DynWinRtType.object()]).iid()",
    ));
    assert!(js.contains(
        "VectorChangedEventHandler_Object_PARAM_TYPES = [DynWinRtType.object(), DynWinRtType.interface(",
    ));
}
