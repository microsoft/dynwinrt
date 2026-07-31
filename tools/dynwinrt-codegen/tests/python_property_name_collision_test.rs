// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;

use dynwinrt_codegen::codegen::{python, python_stub};
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

    let py = python::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let pyi = python_stub::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());

    assert!(py.contains("from builtins import property as _property"));
    assert_eq!(py.matches("    @_property\n").count(), 2);
    assert!(py.contains("def property(self)"));
    assert!(py.contains("def old_value(self)"));

    assert!(pyi.contains("import builtins"));
    assert_eq!(pyi.matches("    @builtins.property\n").count(), 2);
}
