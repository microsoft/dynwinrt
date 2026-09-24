// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod common;

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
        &Default::default(),
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
    assert!(js.contains("new (__get_IVector_Object())(value)",), "{js}",);
    assert!(js.contains("onVectorChanged"));
    assert!(
        js.contains("asVector: { value: observable.asVector.bind(observable) }"),
        "{js}",
    );
    assert!(js.contains("asVector() {\n        return new (__get_IVector_Object())(this._obj);",));
    assert!(dts.contains("asVector(): IVector_Object;"));
    assert!(dts.contains(
        "static create(items: unknown[]): IObservableVector_Object & IVector_Object;",
    ));
}

#[test]
fn observable_vector_projects_python_mutable_sequence_and_typed_events() {
    let handler_type = TypeMeta::Parameterized {
        namespace: "Windows.Foundation.Collections".into(),
        name: "VectorChangedEventHandler`1".into(),
        piid: "0c051752-9fbf-4c70-aa0c-0e4c82d9a761".into(),
        args: vec![TypeMeta::Object],
    };
    let observable_type = TypeMeta::Parameterized {
        namespace: "Windows.Foundation.Collections".into(),
        name: "IObservableVector`1".into(),
        piid: "5917eb53-50b4-4a0d-b309-65862b3f1dbc".into(),
        args: vec![TypeMeta::Object],
    };
    let event_args_type = TypeMeta::Interface {
        namespace: "Windows.Foundation.Collections".into(),
        name: "IVectorChangedEventArgs".into(),
        iid: "575933df-34fe-4480-af15-07691f3d5d9b".into(),
    };
    let mut interface = InterfaceMeta {
        name: "IObservableVector_Object".into(),
        namespace: "Windows.Foundation.Collections".into(),
        iid: String::new(),
        generic_piid: Some("5917eb53-50b4-4a0d-b309-65862b3f1dbc".into()),
        generic_args: vec![TypeMeta::Object],
        methods: vec![
            MethodMeta {
                name: "add_VectorChanged".into(),
                raw_name: "add_VectorChanged".into(),
                vtable_index: 6,
                params: vec![ParamMeta {
                    name: "handler".into(),
                    typ: handler_type.clone(),
                    direction: ParamDirection::In,
                }],
                is_event_add: true,
                ..Default::default()
            },
            MethodMeta {
                name: "remove_VectorChanged".into(),
                raw_name: "remove_VectorChanged".into(),
                vtable_index: 7,
                params: vec![ParamMeta {
                    name: "token".into(),
                    typ: TypeMeta::I64,
                    direction: ParamDirection::In,
                }],
                is_event_remove: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    interface
        .implementation_metadata
        .delegates
        .push(common::delegate_invoke(
            handler_type,
            &[("sender", observable_type), ("event", event_args_type)],
        ));
    let known_types = HashSet::from([
        "IObservableVector_Object".into(),
        "IVector_Object".into(),
        "IVectorChangedEventArgs".into(),
    ]);
    let delegate_types = HashSet::from(["VectorChangedEventHandler_Object".into()]);
    let callback = "Callable[['IObservableVector_Object', 'IVectorChangedEventArgs'], object]";

    let py = common::generate_interface(&interface, &known_types, &delegate_types);
    assert!(py.contains(
        "class IObservableVector_Object(_dynwinrt_symbol('i_vector_object', 'IVector_Object')):"
    ));
    assert!(
        py.contains("_dynwinrt_symbol('i_vector_object', 'IVector_Object')._set_native(self, obj)")
    );
    assert!(py.contains("self._observable_obj = obj.cast(IID_IObservableVector_Object)"));
    let create_signature = "def create(items: Iterable['DynWinRTValue | _DynWinRTObject']) -> 'IObservableVector_Object':";
    assert!(py.contains(create_signature), "{py}");
    assert!(
        py.contains(
            "_dynwinrt_new_vector(items, lambda item: getattr(item, '_obj', item), DynWinRTType.object())"
        ),
        "{py}"
    );
    assert!(py.contains("def as_vector(self) -> 'IVector_Object':"));
    for signature in common::event_signatures("vector_changed", callback) {
        assert!(py.contains(&signature), "{signature}\n{py}");
    }
    assert!(
        py.contains(
            "lambda __p0__, __p1__: (\
             (lambda value: None if value.is_null() else \
             _dynwinrt_symbol('i_observable_vector_object', 'IObservableVector_Object')(value))(__p0__), \
             (lambda value: None if value.is_null() else \
             _dynwinrt_symbol('windows__foundation__collections__i_vector_changed_event_args', 'IVectorChangedEventArgs')(value))(__p1__))"
        ),
        "{py}"
    );
    assert!(py.contains("_dynwinrt_delegate(callback,"));
    assert!(py.contains("_IObservableVector_Object.method(6).invoke(self._observable_obj"));

    let pyi = common::generate_interface_stub(&interface, &known_types, &delegate_types);
    assert!(
        pyi.contains(
            "class IObservableVector_Object(_IObservableVector_ObjectIdentity, MutableSequence[DynWinRTValue | None]):"
        ),
        "{pyi}"
    );
    assert!(pyi.contains(
        "from .windows__foundation__collections__i_vector_changed_event_args import IID_IVectorChangedEventArgs, IVectorChangedEventArgs"
    ), "{pyi}");
    assert!(pyi.contains(&format!("{create_signature} ...")), "{pyi}");
    assert!(
        pyi.contains("def __getitem__(self, index: int) -> DynWinRTValue | None: ..."),
        "{pyi}"
    );
    assert!(pyi.contains("def as_vector(self) -> 'IVector_Object': ..."));
    for signature in common::event_stub_signatures("vector_changed", callback) {
        assert!(pyi.contains(&signature), "{signature}\n{pyi}");
    }
}

#[test]
fn generic_delegate_iid_uses_declared_type_arguments() {
    let interface = InterfaceMeta {
        name: "VectorChangedEventHandler_Object".into(),
        namespace: "Windows.Foundation.Collections".into(),
        iid: "0c051752-9fbf-4c70-aa0c-0e4c82d9a761".into(),
        generic_piid: Some("0c051752-9fbf-4c70-aa0c-0e4c82d9a761".into()),
        generic_args: vec![TypeMeta::Object],
        methods: vec![
            MethodMeta {
                name: ".ctor".into(),
                raw_name: ".ctor".into(),
                ..Default::default()
            },
            MethodMeta {
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
            },
        ],
        ..Default::default()
    };
    let projected = project::project_delegate(
        &Default::default(),
        &interface,
        &HashMap::new(),
        &HashMap::new(),
    );
    let js = render_js::render(&projected);
    let py = common::generate_interface(
        &interface,
        &HashSet::new(),
        &HashSet::from(["VectorChangedEventHandler_Object".into()]),
    );

    assert!(js.contains(
        "DynWinRtType.parameterized(WinGuid.parse('0c051752-9fbf-4c70-aa0c-0e4c82d9a761'), [DynWinRtType.object()]).iid()",
    ));
    assert!(js.contains(
        "VectorChangedEventHandler_Object_PARAM_TYPES = [DynWinRtType.object(), DynWinRtType.interface(",
    ));
    assert!(py.contains(
        "DynWinRTType.parameterized(WinGUID.parse('0c051752-9fbf-4c70-aa0c-0e4c82d9a761'), [DynWinRTType.object()]).iid()"
    ), "{py}");
    assert!(py.contains(
        "VectorChangedEventHandler_Object_PARAM_TYPES = [DynWinRTType.object(), DynWinRTType.interface("
    ));
}
