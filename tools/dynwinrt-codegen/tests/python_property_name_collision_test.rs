// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod common;

use std::collections::HashSet;

use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta, MethodMeta};
use dynwinrt_codegen::types::TypeMeta;

#[test]
fn property_named_property_does_not_shadow_the_decorator() {
    let class = ClassMeta {
        name: "ChangedEventArgs".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.ChangedEventArgs".into(),
        default_interface: Some(InterfaceMeta {
            name: "IChangedEventArgs".into(),
            namespace: "Contoso".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
            methods: vec![
                MethodMeta {
                    name: "get_Property".into(),
                    raw_name: "get_Property".into(),
                    vtable_index: 6,
                    return_type: Some(TypeMeta::Object),
                    is_property_getter: true,
                    ..Default::default()
                },
                MethodMeta {
                    name: "get_OldValue".into(),
                    raw_name: "get_OldValue".into(),
                    vtable_index: 7,
                    return_type: Some(TypeMeta::Object),
                    is_property_getter: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        ..Default::default()
    };
    let known = HashSet::from(["ChangedEventArgs".into()]);

    let py = common::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let pyi = common::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());

    assert!(py.contains("_property, _weakref_ref,"));
    assert!(pyi.contains("import builtins"));
    for name in ["property", "old_value"] {
        assert!(
            py.contains(&format!(
                "    @_property\n    def {name}(self) -> WinRTObjectValue | None:"
            )),
            "{py}"
        );
        assert!(
            pyi.contains(&format!(
                "    @builtins.property\n    def {name}(self) -> WinRTObjectValue | None: ..."
            )),
            "{pyi}"
        );
    }
    assert!(
        pyi.contains("    @builtins.property\n    def _obj(self) -> DynWinRTValue: ..."),
        "{pyi}"
    );
    assert!(!py.contains("    @property\n"), "{py}");
    assert!(!pyi.contains("    @property\n"), "{pyi}");
}
