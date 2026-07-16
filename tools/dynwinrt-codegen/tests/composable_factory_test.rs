// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{HashMap, HashSet};

use dynwinrt_codegen::codegen::{project, render_dts, render_js};
use dynwinrt_codegen::meta::{
    ClassMeta, ConstructorKind, ConstructorMeta, InterfaceMeta, MethodMeta, ParamDirection,
    ParamMeta,
};
use dynwinrt_codegen::types::{TypeKind, TypeMeta, TypeRef};

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
        constructors: vec![ConstructorMeta {
            kind: ConstructorKind::PublicComposition,
            factory_interface: Some(TypeRef {
                namespace: "Contoso".into(),
                name: "IWidgetFactory".into(),
                kind: TypeKind::Interface,
            }),
        }],
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
        js.contains("Widget._fromNative(_IWidgetFactory.method(6).invokeAll(")
            && js.contains("])[1])"),
        "composable factory must select the final public-instance output:\n{js}"
    );
    assert!(js.contains("static _fromNative(obj)"));
    assert!(js.contains("Object.assign(Object.create(Widget.prototype)"));
    assert!(js.contains("constructor(...args)"));
    assert!(js.contains("this._obj = Widget.createInstance(null)._obj;"));
    assert!(!dts.contains("_fromNative"));
    assert!(dts.contains("constructor();"));
    assert!(
        dts.contains("static createInstance(outer: unknown): Widget;"),
        "factory declaration must return the runtime class:\n{dts}"
    );

    let mut protected = class.clone();
    protected.constructors[0].kind = ConstructorKind::ProtectedComposition;
    let projected = project::project_class(
        &protected,
        &HashSet::from(["Widget".into()]),
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let protected_js = render_js::render(&projected);
    let protected_dts = render_dts::render(&projected);
    assert!(protected_js.contains("Widget cannot be constructed directly."));
    assert!(protected_dts.contains("private constructor();"));
    assert!(protected_dts.contains("static createInstance(outer: unknown): Widget;"));
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
        constructors: vec![ConstructorMeta {
            kind: ConstructorKind::DefaultActivation,
            factory_interface: None,
        }],
        ..Default::default()
    };
    let known_types = HashSet::from([
        "Application".into(),
        "ApplicationInitializationCallback".into(),
        "XamlControlsXamlMetaDataProvider".into(),
        "XamlControlsResources".into(),
        "ResourceManager".into(),
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
    assert!(js.contains("static create(onLaunched)"));
    assert!(js.contains("const _callback_d = DynWinRtDelegate.create("));
    assert!(js.contains("IID_ApplicationInitializationCallback"));
    assert!(js.contains("f81c4e72-7a18-4a30-9126-6f62b6bdac83"));
    assert!(js.contains("DynWinRtValue.createXamlApplication"));
    assert!(js.contains("let _resourcesInitialized = false"));
    assert!(js.contains("getWinappsdkResourcePriPath"));
    assert!(js.contains("hasPackageIdentity"));
    assert!(js.contains("onResourceManagerRequested"));
    assert!(js.contains("(__get_ResourceManager()).createInstance"));
    assert!(
        js.contains(
            "resources.mergedDictionaries.append((__get_XamlControlsResources()).create())"
        )
    );
    assert!(dts.contains(
        "static createWithMetadataProvider(metadataProvider: XamlControlsXamlMetaDataProvider, onLaunched?: () => void): Application;"
    ));
    assert!(dts.contains("static create(onLaunched?: () => void): Application;"));
    assert!(dts.contains("private constructor();"));
}
