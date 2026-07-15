// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{HashMap, HashSet};

use dynwinrt_codegen::codegen::{project, render_dts, render_js};
use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::TypeMeta;

#[test]
fn composable_factory_returns_public_instance() {
    let factory = InterfaceMeta {
        name: "IWidgetFactory".into(),
        namespace: "Contoso".into(),
        iid: "11111111-1111-1111-1111-111111111111".into(),
        methods: vec![MethodMeta {
            name: "CreateInstance".into(),
            raw_name: "CreateInstance".into(),
            vtable_index: 6,
            params: vec![
                ParamMeta {
                    name: "outer".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "inner".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::Out,
                },
            ],
            return_type: Some(TypeMeta::RuntimeClass {
                namespace: "Contoso".into(),
                name: "Widget".into(),
                default_iid: "22222222-2222-2222-2222-222222222222".into(),
            }),
            ..Default::default()
        }],
        ..Default::default()
    };
    let class = ClassMeta {
        name: "Widget".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.Widget".into(),
        default_interface: Some(InterfaceMeta {
            name: "IWidget".into(),
            namespace: "Contoso".into(),
            iid: "22222222-2222-2222-2222-222222222222".into(),
            ..Default::default()
        }),
        factory_interfaces: vec![factory],
        ..Default::default()
    };

    let projected = project::project_class(
        &class,
        &HashSet::from(["Widget".into()]),
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let js = render_js::render(&projected);
    let dts = render_dts::render(&projected);

    assert!(
        js.contains("new Widget(_IWidgetFactory.method(6).invokeAll(") && js.contains("])[1])"),
        "composable factory must select the final public-instance output:\n{js}"
    );
    assert!(
        dts.contains("static createInstance(outer: unknown): Widget;"),
        "factory declaration must return the runtime class:\n{dts}"
    );
}

#[test]
fn winui_application_projects_fluent_bootstrap_helpers() {
    let application = ClassMeta {
        name: "Application".into(),
        namespace: "Microsoft.UI.Xaml".into(),
        full_name: "Microsoft.UI.Xaml.Application".into(),
        default_interface: Some(InterfaceMeta {
            name: "IApplication".into(),
            namespace: "Microsoft.UI.Xaml".into(),
            iid: "33333333-3333-3333-3333-333333333333".into(),
            ..Default::default()
        }),
        static_interfaces: vec![InterfaceMeta {
            name: "IApplicationStatics".into(),
            namespace: "Microsoft.UI.Xaml".into(),
            iid: "44444444-4444-4444-4444-444444444444".into(),
            methods: vec![MethodMeta {
                name: "Start".into(),
                raw_name: "Start".into(),
                vtable_index: 6,
                params: vec![ParamMeta {
                    name: "callback".into(),
                    typ: TypeMeta::Interface {
                        namespace: "Microsoft.UI.Xaml".into(),
                        name: "ApplicationInitializationCallback".into(),
                        iid: "d8eef1c9-1234-56f1-9963-45dd9c80a661".into(),
                    },
                    direction: ParamDirection::In,
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let known_types = HashSet::from([
        "Application".into(),
        "ApplicationInitializationCallback".into(),
        "XamlControlsXamlMetaDataProvider".into(),
        "XamlControlsResources".into(),
    ]);
    let delegate_names = HashSet::from(["ApplicationInitializationCallback".into()]);
    let delegate_sigs = HashMap::from([(
        "ApplicationInitializationCallback".into(),
        "(p: DynWinRtValue) => void".into(),
    )]);

    let projected = project::project_class(
        &application,
        &known_types,
        &delegate_names,
        &HashSet::new(),
        &delegate_sigs,
        &HashMap::new(),
        &HashMap::from([(
            "ApplicationInitializationCallback".into(),
            vec!["__a0__".into()],
        )]),
    );
    let js = render_js::render(&projected);
    let dts = render_dts::render(&projected);

    assert!(js.contains("static createWithMetadataProvider(metadataProvider, onLaunched)"));
    assert!(js.contains("static createWithFluentResources(onLaunched)"));
    assert!(js.contains("const _callback_d = DynWinRtDelegate.create("));
    assert!(js.contains("IID_ApplicationInitializationCallback"));
    assert!(js.contains("f81c4e72-7a18-4a30-9126-6f62b6bdac83"));
    assert!(js.contains("DynWinRtValue.createXamlApplication"));
    assert!(js.contains("let _resourcesInitialized = false"));
    assert!(
        js.contains(
            "resources.mergedDictionaries.append((__get_XamlControlsResources()).create())"
        )
    );
    assert!(dts.contains(
        "static createWithMetadataProvider(metadataProvider: XamlControlsXamlMetaDataProvider, onLaunched?: () => void): Application;"
    ));
    assert!(
        dts.contains("static createWithFluentResources(onLaunched?: () => void): Application;")
    );
}
