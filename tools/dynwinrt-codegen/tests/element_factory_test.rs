// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{HashMap, HashSet};

use dynwinrt_codegen::codegen::{project, python, python_stub, render_dts, render_js};
use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::TypeMeta;

#[test]
fn projection_properties_feature_detect_fast_accessors() {
    fn getter(name: &str, index: usize, typ: TypeMeta) -> MethodMeta {
        MethodMeta {
            name: format!("get_{name}"),
            raw_name: format!("get_{name}"),
            vtable_index: index,
            return_type: Some(typ),
            is_property_getter: true,
            ..Default::default()
        }
    }

    fn setter(name: &str, index: usize, typ: TypeMeta) -> MethodMeta {
        MethodMeta {
            name: format!("put_{name}"),
            raw_name: format!("put_{name}"),
            vtable_index: index,
            params: vec![ParamMeta {
                name: "value".into(),
                typ,
                direction: ParamDirection::In,
            }],
            is_property_setter: true,
            ..Default::default()
        }
    }

    let mode = TypeMeta::Enum {
        namespace: "Contoso".into(),
        name: "Mode".into(),
        underlying: Box::new(TypeMeta::I32),
        members: vec![],
        is_flags: false,
        doc: None,
        deprecated: None,
    };
    let interface = InterfaceMeta {
        name: "IProjectionFastPaths".into(),
        namespace: "Contoso".into(),
        iid: "11111111-1111-1111-1111-111111111111".into(),
        methods: vec![
            getter("Name", 6, TypeMeta::String),
            setter("Name", 7, TypeMeta::String),
            getter("Enabled", 8, TypeMeta::Bool),
            setter("Enabled", 9, TypeMeta::Bool),
            getter("Count", 10, TypeMeta::I32),
            setter("Count", 11, TypeMeta::I32),
            getter("Mode", 12, mode.clone()),
            setter("Mode", 13, mode),
            setter("Limit", 14, TypeMeta::U32),
            setter("Ratio", 15, TypeMeta::F32),
            setter("Scale", 16, TypeMeta::F64),
            getter("Child", 17, TypeMeta::Object),
        ],
        ..Default::default()
    };
    let projected = project::project_interface(
        &Default::default(),
        &interface,
        &HashSet::from(["IProjectionFastPaths".into(), "Mode".into()]),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let js = render_js::render(&projected);

    for method in [
        "getString",
        "getBool",
        "getI32",
        "getObj",
        "setHstring",
        "setBool",
        "setI32",
        "setU32",
        "setF32",
        "setF64",
    ] {
        assert!(
            js.contains(&format!("typeof _m.{method} === 'function'")),
            "missing feature detection for {method}:\n{js}"
        );
    }
    for fallback in [
        "_m.invoke(this._obj, [DynWinRtValue.hstring(value)])",
        "_m.invoke(this._obj, [DynWinRtValue.boolValue(value)])",
        "_m.invoke(this._obj, [DynWinRtValue.i32(value)])",
        "_m.invoke(this._obj, [DynWinRtValue.u32(value)])",
        "_m.invoke(this._obj, [DynWinRtValue.f32(value)])",
        "_m.invoke(this._obj, [DynWinRtValue.f64(value)])",
        "_m.invoke(this._obj, [])",
    ] {
        assert!(
            js.contains(fallback),
            "missing generic invoke fallback {fallback}:\n{js}"
        );
    }
}

#[test]
fn projection_caches_activation_factories_and_activate_method() {
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
        factory_interfaces: vec![InterfaceMeta {
            name: "IWidgetFactory".into(),
            namespace: "Contoso".into(),
            iid: "33333333-3333-3333-3333-333333333333".into(),
            ..Default::default()
        }],
        static_interfaces: vec![InterfaceMeta {
            name: "IWidgetStatics".into(),
            namespace: "Contoso".into(),
            iid: "44444444-4444-4444-4444-444444444444".into(),
            ..Default::default()
        }],
        has_default_constructor: true,
        ..Default::default()
    };
    let projected = project::project_class(
        &Default::default(),
        &class,
        &HashSet::from(["Widget".into()]),
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let js = render_js::render(&projected);

    assert!(js.contains("let __activateInstance;"), "{js}");
    assert!(
        js.contains(
            "const __get_ActivateInstance = () => (__activateInstance ??= _IActivationFactory.method(6));"
        ),
        "{js}"
    );
    assert!(
        js.contains("static __defaultActivationFactory;")
            && js.contains(
                "Widget.__defaultActivationFactory ??= DynWinRtValue.activationFactory('Contoso.Widget')"
            ),
        "{js}"
    );
    assert!(
        js.contains(
            "Widget._f_IWidgetFactory ??= DynWinRtValue.activationFactory('Contoso.Widget').cast(IID_IWidgetFactory)"
        ),
        "{js}"
    );
    assert!(
        js.contains(
            "Widget._s_IWidgetStatics ??= DynWinRtValue.activationFactory('Contoso.Widget').cast(IID_IWidgetStatics)"
        ),
        "{js}"
    );
    assert!(
        js.contains(
            "Widget._fromNative(__get_ActivateInstance().invoke(Widget._defaultActivationFactory(), []))"
        ),
        "{js}"
    );
}

#[test]
fn element_factory_projects_js_callback_constructor() {
    let get_args = TypeMeta::RuntimeClass {
        namespace: "Microsoft.UI.Xaml".into(),
        name: "ElementFactoryGetArgs".into(),
        default_interface: None,
    };
    let recycle_args = TypeMeta::RuntimeClass {
        namespace: "Microsoft.UI.Xaml".into(),
        name: "ElementFactoryRecycleArgs".into(),
        default_interface: None,
    };
    let ui_element = TypeMeta::RuntimeClass {
        namespace: "Microsoft.UI.Xaml".into(),
        name: "UIElement".into(),
        default_interface: None,
    };
    let interface = InterfaceMeta {
        name: "IElementFactory".into(),
        namespace: "Microsoft.UI.Xaml".into(),
        iid: "75faba47-2cf2-54ae-91e6-0581556fddaa".into(),
        methods: vec![
            MethodMeta {
                name: "GetElement".into(),
                raw_name: "GetElement".into(),
                vtable_index: 6,
                params: vec![ParamMeta {
                    name: "args".into(),
                    typ: get_args,
                    direction: ParamDirection::In,
                }],
                return_type: Some(ui_element),
                ..Default::default()
            },
            MethodMeta {
                name: "RecycleElement".into(),
                raw_name: "RecycleElement".into(),
                vtable_index: 7,
                params: vec![ParamMeta {
                    name: "args".into(),
                    typ: recycle_args,
                    direction: ParamDirection::In,
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let known_types = HashSet::from([
        "IElementFactory".into(),
        "ElementFactoryGetArgs".into(),
        "ElementFactoryRecycleArgs".into(),
        "UIElement".into(),
    ]);
    let projected = project::project_interface(
        &Default::default(),
        &interface,
        &known_types,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let js = render_js::render(&projected);
    let dts = render_dts::render(&projected);
    let py = python::generate_interface(&interface, &known_types, &HashSet::new());
    let pyi = python_stub::generate_interface_stub(&interface, &known_types, &HashSet::new());

    assert!(js.contains("DynWinRtElementFactory.create"));
    assert!(js.contains("const elements = new Map()"));
    assert!(js.contains("DynWinRtElementFactory.create((__load_UIElement()).IID_UIElement"));
    assert!(js.contains("nativeElement.identityRaw()"));
    assert!(!js.contains("nativeElement = _unwrap(element).cast("));
    assert!(js.contains("Object.defineProperty(recycleArgs, 'element'"));
    assert!(js.contains("ElementFactoryGetArgs"));
    assert!(js.contains("ElementFactoryRecycleArgs"));
    assert!(dts.contains(
        "static create(getElement: (args: ElementFactoryGetArgs) => UIElement, recycleElement: (args: ElementFactoryRecycleArgs) => void): IElementFactory & { releaseCallbacks(): void };",
    ));
    assert!(py.contains("from dynwinrt import DynWinRtElementFactory"));
    assert!(py.contains("def create(get_element, recycle_element):"));
    assert!(py.contains("native_element = native.cast("));
    assert!(py.contains("elements[native_element.identity_raw()] = element"));
    assert!(py.contains("element = elements.pop(native.identity_raw(), projected_element)"));
    assert!(py.contains("callback_state = [True]"));
    assert!(py.contains("callback_state[0] = False"));
    assert!(py.contains("factory = IElementFactory._from_native(implementation.to_value())"));
    assert!(
        py.matches("IElementFactory callbacks have been released.")
            .count()
            >= 5
    );
    assert!(py.contains("def __setattr__(self, name, value):"));
    assert!(py.contains("setattr(self._source, name, value)"));
    assert!(py.contains("DynWinRtElementFactory.create("));
    assert!(py.contains("'IID_IUIElement'"));
    assert!(py.contains("def release_callbacks(self):"));
    assert!(pyi.contains("get_element: Callable[['ElementFactoryGetArgs'], 'UIElement']"));
    assert!(pyi.contains("def release_callbacks(self) -> None: ..."));
}
