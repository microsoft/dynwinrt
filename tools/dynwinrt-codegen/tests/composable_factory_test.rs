// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod common;

use std::collections::{HashMap, HashSet};

use dynwinrt_codegen::codegen::{javascript, project, render_dts, render_js};
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
                default_interface: Some(Box::new(TypeMeta::Interface {
                    namespace: "Contoso".into(),
                    name: "IWidget".into(),
                    iid: "22222222-2222-2222-2222-222222222222".into(),
                })),
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
        overridable_interfaces: vec![InterfaceMeta {
            name: "IWidgetOverrides".into(),
            namespace: "Contoso".into(),
            iid: "33333333-3333-3333-3333-333333333333".into(),
            methods: vec![MethodMeta {
                name: "MeasureOverride".into(),
                raw_name: "MeasureOverride".into(),
                vtable_index: 6,
                ..Default::default()
            }],
            ..Default::default()
        }],
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
    assert!(js.contains("castProjectedValue"));
    assert!(js.contains("lifetime.js"));
    assert!(js.contains("constructor(...args)"));
    assert!(js.contains("this._obj = Widget.createInstance(null)._obj;"));
    assert!(!dts.contains("_fromNative"));
    assert!(dts.contains("constructor();"));
    assert!(
        dts.contains("static createInstance(outer: unknown): Widget;"),
        "factory declaration must return the runtime class:\n{dts}"
    );

    let py = common::generate_class(
        &class,
        &HashSet::from(["Widget".into()]),
        &HashSet::new(),
        &HashSet::new(),
    );
    let pyi = common::generate_class_stub(
        &class,
        &HashSet::from(["Widget".into()]),
        &HashSet::new(),
        &HashSet::new(),
    );
    assert!(
        py.contains("_IWidgetFactory.method(6).invoke_all(")
            && py.contains("return Widget._from_native(_results[1])"),
        "Python composable factory must select the final public-instance output:\n{py}"
    );
    assert!(!py.contains("_f_IWidgetFactory = None"));
    assert!(py.contains(
        "return DynWinRTValue.activation_factory('Contoso.Widget').cast(IID_IWidgetFactory)"
    ));
    assert!(
        py.contains(
            "self._set_native(type(self).create_instance(DynWinRTValue.null_value())._obj)"
        ),
        "Python composable constructor must pass an explicit null WinRT value for outer:\n{py}"
    );
    assert!(py.contains("if _is_python_subclass:"));
    assert!(py.contains(
        "_IWidgetFactory.method(6).invoke_composed_with_overrides(Widget._get_f_IWidgetFactory(), [], 0, 0, 1, False, _override_interfaces)"
    ));
    assert!(py.contains("DynWinRTOverrideInterface(IID_IWidgetOverrides, ['void0']"));
    assert!(py.contains("_override_callbacks[6] = _override_measure_override"));
    assert!(py.contains("for _type in type(self).__mro__ if _type is not Widget"));
    assert!(py.contains("'measure_override'"));
    assert!(py.contains("_override_target_ref = _weakref_ref(self)"));
    assert!(py.contains("_target = _target_ref()"));
    assert!(py.contains("native overrides require public composable construction"));
    assert!(py.contains(
        "def register_xaml_runtime_class(cls, runtime_class_name: str, control_type: type):"
    ));
    assert!(py.contains(
        "_dynwinrt_register_xaml_runtime_class(runtime_class_name, 'Contoso.Widget', IID_IWidget, control_type, _native_overrides)"
    ));
    assert!(py.contains("control_type must be a Python subclass of Widget"));
    assert!(
        pyi.contains("def __init__(self) -> None: ..."),
        "Python composable constructor stub must hide the ABI-only outer argument:\n{pyi}"
    );
    assert!(pyi.contains("def create_instance(outer: 'DynWinRTValue') -> 'Widget': ..."));
    assert!(pyi.contains(
        "native overrides are registered during construction; unsupported ABI shapes fail closed"
    ));
    assert!(pyi.contains(
        "def register_xaml_runtime_class(cls, runtime_class_name: str, control_type: type[Self]) -> DynWinRTXamlRegistration: ..."
    ));

    let mut duplicate_signature = class.clone();
    duplicate_signature.constructors.insert(
        0,
        ConstructorMeta {
            kind: ConstructorKind::DefaultActivation,
            factory_interface: None,
        },
    );
    let duplicate_py = common::generate_class(
        &duplicate_signature,
        &HashSet::from(["Widget".into()]),
        &HashSet::new(),
        &HashSet::new(),
    );
    assert!(
        duplicate_py.contains(
            "_IWidgetFactory.method(6).invoke_composed_with_overrides(Widget._get_f_IWidgetFactory(), [], 0, 0, 1, False, _override_interfaces)"
        ),
        "same-signature activation must retain composable subclass support:\n{duplicate_py}"
    );

    let mut unsupported_override = class.clone();
    unsupported_override.overridable_interfaces[0].methods[0]
        .params
        .push(ParamMeta {
            name: "value".into(),
            typ: TypeMeta::Object,
            direction: ParamDirection::In,
        });
    let unsupported_py = common::generate_class(
        &unsupported_override,
        &HashSet::from(["Widget".into()]),
        &HashSet::new(),
        &HashSet::new(),
    );
    assert!(unsupported_py.contains("Widget native override ABI is unsupported: "));
    assert!(unsupported_py.contains("difference(())"));
    assert!(!unsupported_py.contains("DynWinRTOverrideInterface(IID_IWidgetOverrides"));

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

    let protected_py = common::generate_class(
        &protected,
        &HashSet::from(["Widget".into()]),
        &HashSet::new(),
        &HashSet::new(),
    );
    let protected_pyi = common::generate_class_stub(
        &protected,
        &HashSet::from(["Widget".into()]),
        &HashSet::new(),
        &HashSet::new(),
    );
    assert!(
        !protected_py.contains("self._set_native(type(self).create_instance("),
        "protected composition must not become a public Python constructor:\n{protected_py}"
    );
    assert!(
        protected_py.contains("raise TypeError(\"Widget cannot be constructed directly\")"),
        "protected composition must remain non-constructible from Python:\n{protected_py}"
    );
    assert!(
        !protected_pyi.contains("def __init__(self) -> None: ..."),
        "protected composition must not advertise a public no-arg constructor:\n{protected_pyi}"
    );
}

#[test]
fn python_zero_arg_create_instance_gets_typed_create_alias() {
    let factory = InterfaceMeta {
        name: "IWidgetFactory".into(),
        namespace: "Contoso".into(),
        iid: "11111111-1111-1111-1111-111111111111".into(),
        methods: vec![MethodMeta {
            name: "CreateInstance".into(),
            raw_name: "CreateInstance".into(),
            vtable_index: 6,
            return_type: Some(TypeMeta::RuntimeClass {
                namespace: "Contoso".into(),
                name: "Widget".into(),
                default_interface: None,
            }),
            ..Default::default()
        }],
        ..Default::default()
    };
    let class = ClassMeta {
        name: "Widget".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.Widget".into(),
        factory_interfaces: vec![factory],
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
    let known = HashSet::from(["Widget".into()]);

    let py = common::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let pyi = common::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());

    assert!(py.contains("def create() -> 'Widget':"));
    assert!(py.contains("return Widget.create_instance()"));
    assert!(pyi.contains("def create() -> 'Widget': ..."));
}

#[test]
fn class_without_default_interface_tracks_each_ownership_path_once() {
    let class = ClassMeta {
        name: "Opaque".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.Opaque".into(),
        ..Default::default()
    };
    let projected = project::project_class(
        &class,
        &HashSet::from(["Opaque".into()]),
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let js = render_js::render(&projected);
    assert_eq!(
        js.matches("(__get_trackProjectedValue())(obj, 'Opaque')")
            .count(),
        2,
        "owned and borrowed raw values must each be tracked once:\n{js}"
    );
    assert!(js.contains("static _fromNative(obj)"));
    assert!(js.contains("static _fromNativeBorrowed(obj)"));
}

#[test]
fn parameterized_default_interface_uses_computed_iid() {
    let piid = "913337e9-11a1-4345-a3a2-4e7f956e222d";
    let row_type = TypeMeta::RuntimeClass {
        namespace: "Microsoft.UI.Xaml.Controls".into(),
        name: "RowDefinition".into(),
        default_interface: Some(Box::new(TypeMeta::Interface {
            namespace: "Microsoft.UI.Xaml.Controls".into(),
            name: "IRowDefinition".into(),
            iid: "fe870f2f-89ef-5dac-9f33-968d0dc577c3".into(),
        })),
    };
    let vector_view_name = javascript::parameterized_name(
        "Windows.Foundation.Collections",
        "IVectorView",
        "bbe1fa4c-b0e3-4583-baef-1f1b2e483e56",
        std::slice::from_ref(&row_type),
    );
    let class = ClassMeta {
        name: "RowDefinitionCollection".into(),
        namespace: "Microsoft.UI.Xaml.Controls".into(),
        full_name: "Microsoft.UI.Xaml.Controls.RowDefinitionCollection".into(),
        default_interface: Some(InterfaceMeta {
            name: "IVector_RowDefinition".into(),
            namespace: "Windows.Foundation.Collections".into(),
            iid: piid.into(),
            generic_piid: Some(piid.into()),
            generic_args: vec![row_type.clone()],
            methods: vec![
                MethodMeta {
                    name: "GetView".into(),
                    raw_name: "GetView".into(),
                    vtable_index: 8,
                    return_type: Some(TypeMeta::Parameterized {
                        namespace: "Windows.Foundation.Collections".into(),
                        name: "IVectorView`1".into(),
                        piid: "bbe1fa4c-b0e3-4583-baef-1f1b2e483e56".into(),
                        args: vec![row_type.clone()],
                    }),
                    ..Default::default()
                },
                MethodMeta {
                    name: "IndexOf".into(),
                    raw_name: "IndexOf".into(),
                    vtable_index: 9,
                    params: vec![
                        ParamMeta {
                            name: "value".into(),
                            typ: row_type.clone(),
                            direction: ParamDirection::In,
                        },
                        ParamMeta {
                            name: "index".into(),
                            typ: TypeMeta::U32,
                            direction: ParamDirection::Out,
                        },
                    ],
                    return_type: Some(TypeMeta::Bool),
                    ..Default::default()
                },
                MethodMeta {
                    name: "Append".into(),
                    raw_name: "Append".into(),
                    vtable_index: 12,
                    params: vec![ParamMeta {
                        name: "value".into(),
                        typ: row_type.clone(),
                        direction: ParamDirection::In,
                    }],
                    ..Default::default()
                },
                MethodMeta {
                    name: "GetMany".into(),
                    raw_name: "GetMany".into(),
                    vtable_index: 16,
                    params: vec![
                        ParamMeta {
                            name: "startIndex".into(),
                            typ: TypeMeta::U32,
                            direction: ParamDirection::In,
                        },
                        ParamMeta {
                            name: "items".into(),
                            typ: TypeMeta::Array(Box::new(row_type.clone())),
                            direction: ParamDirection::OutFill,
                        },
                    ],
                    return_type: Some(TypeMeta::U32),
                    ..Default::default()
                },
                MethodMeta {
                    name: "ReplaceAll".into(),
                    raw_name: "ReplaceAll".into(),
                    vtable_index: 17,
                    params: vec![ParamMeta {
                        name: "items".into(),
                        typ: TypeMeta::Array(Box::new(row_type)),
                        direction: ParamDirection::In,
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        ..Default::default()
    };

    let projected = project::project_class(
        &class,
        &HashSet::from([
            "RowDefinition".into(),
            "RowDefinitionCollection".into(),
            vector_view_name.clone(),
        ]),
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let js = render_js::render(&projected);
    let dts = render_dts::render(&projected);

    let expected = "DynWinRtType.parameterized(WinGuid.parse('913337e9-11a1-4345-a3a2-4e7f956e222d'), [DynWinRtType.runtimeClass('Microsoft.UI.Xaml.Controls.RowDefinition', DynWinRtType.interface(WinGuid.parse('fe870f2f-89ef-5dac-9f33-968d0dc577c3')))]).iid()";
    assert!(js.contains(&format!("const IID_IVector_RowDefinition = {expected};")));
    assert!(js.contains(&format!("const IID_RowDefinitionCollection = {expected};")));
    assert!(js.contains("exports.IID_RowDefinitionCollection = IID_RowDefinitionCollection;"));
    assert!(!js.contains("require('./IVector_RowDefinition.js')"));
    assert!(js.contains(&format!("require('./{vector_view_name}.js')")));
    assert!(js.contains("_IVector_RowDefinition.method(9).invokeAll(this._obj"));
    assert!(dts.contains("indexOf(value: RowDefinition): number;"));
    assert!(dts.contains("append(value: RowDefinition): void;"));
    assert!(dts.contains("getMany(startIndex: number, items: RowDefinition[]): RowDefinition[];"));
    assert!(dts.contains("replaceAll(items: RowDefinition[]): void;"));

    let py = common::generate_class(
        &class,
        &HashSet::from([
            "RowDefinition".into(),
            "RowDefinitionCollection".into(),
            "IVectorView_RowDefinition".into(),
        ]),
        &HashSet::new(),
        &HashSet::new(),
    );
    assert!(py.contains("IID_IVector_RowDefinition = DynWinRTType.parameterized("));
    assert!(py.contains("self._obj = obj.cast(IID_IVector_RowDefinition)"));
    assert!(py.contains("def index_of(self, value: 'RowDefinitionLike') -> tuple[int, bool]:"));
    assert!(py.contains("_IVector_RowDefinition.method(9).invoke_all("));
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
        "Microsoft.UI.Xaml.XamlTypeInfo.XamlControlsXamlMetaDataProvider".into(),
        "XamlControlsResources".into(),
        "Microsoft.UI.Xaml.Controls.XamlControlsResources".into(),
        "ResourceManager".into(),
        "Microsoft.Windows.ApplicationModel.Resources.ResourceManager".into(),
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
    assert!(js.contains("static startScheduled(callback)"));
    assert!(js.contains(".invokeScheduled("));
    assert!(!js.contains("registerWinuiDispatcherQueue"));
    assert!(!js.contains("setWinuiDispatcherLoopActive"));
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
    assert!(dts.contains("static startScheduled(callback:"));
    assert!(dts.contains("): Promise<void>;"));
    assert!(dts.contains("static create(onLaunched?: () => void): Application;"));
    assert!(dts.contains("private constructor();"));

    let py = common::generate_class(&application, &known_types, &delegate_names, &HashSet::new());
    let pyi =
        common::generate_class_stub(&application, &known_types, &delegate_names, &HashSet::new());
    assert!(py.contains("def create_with_metadata_provider("));
    assert!(py.contains("def create(on_launched: Callable[[], object] | None = None)"));
    assert!(
        py.contains(".invoke_detached(Application._get_s_IApplicationStatics()"),
        "Python Application.Start must release the GIL:\n{py}"
    );
    assert!(py.contains("_app = Application.get_current()"));
    assert!(!py.contains("_app = Application.current"));
    assert!(py.contains("[DynWinRTType.object()], lambda _args: on_launched()).to_value()"));
    assert!(!py.contains("[DynWinRTType.object(), DynWinRTType.object()], lambda _sender, _args"));
    assert!(pyi.contains(
        "def create_with_metadata_provider(metadata_provider: 'XamlControlsXamlMetaDataProvider', on_launched: Callable[[], object] | None = ...) -> 'Application': ..."
    ));
    assert!(pyi.contains(
        "def create(on_launched: Callable[[], object] | None = ...) -> 'Application': ..."
    ));
}

#[test]
fn winui_application_without_extension_dependencies_has_no_bootstrap_helpers() {
    let application = ClassMeta {
        name: "Application".into(),
        namespace: "Microsoft.UI.Xaml".into(),
        full_name: "Microsoft.UI.Xaml.Application".into(),
        ..Default::default()
    };
    let known_types = HashSet::from(["Application".into()]);
    let projected = project::project_class(
        &application,
        &known_types,
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let js = render_js::render(&projected);
    let dts = render_dts::render(&projected);
    let py = common::generate_class(&application, &known_types, &HashSet::new(), &HashSet::new());
    let pyi =
        common::generate_class_stub(&application, &known_types, &HashSet::new(), &HashSet::new());

    assert!(!js.contains("createWithMetadataProvider"));
    assert!(!dts.contains("createWithMetadataProvider"));
    assert!(!py.contains("create_with_metadata_provider"));
    assert!(!pyi.contains("create_with_metadata_provider"));
}
