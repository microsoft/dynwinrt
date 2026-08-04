// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{HashMap, HashSet};

use dynwinrt_codegen::codegen::{project, python, python_stub, render_dts, render_js};
use dynwinrt_codegen::meta::{InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::TypeMeta;

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
    assert!(py.contains("from dynwinrt_py import DynWinRtElementFactory"));
    assert!(py.contains("def create(get_element, recycle_element):"));
    assert!(py.contains("native_element = native.cast("));
    assert!(py.contains("elements[native_element.identity_raw()] = element"));
    assert!(py.contains("element = elements.pop(native.identity_raw(), projected_element)"));
    assert!(py.contains("callback_state = [True]"));
    assert!(py.contains("callback_state[0] = False"));
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
