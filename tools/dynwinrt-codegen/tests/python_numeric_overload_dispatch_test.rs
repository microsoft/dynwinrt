// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;

use dynwinrt_codegen::codegen::python_stub;
use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::TypeMeta;

fn overloaded_method(name: &str, vtable_index: usize, typ: TypeMeta) -> MethodMeta {
    MethodMeta {
        name: name.into(),
        raw_name: name.into(),
        vtable_index,
        params: vec![ParamMeta {
            name: "value".into(),
            typ,
            direction: ParamDirection::In,
        }],
        ..Default::default()
    }
}

#[test]
fn python_numeric_overload_stubs_retain_typing_overload() {
    let class = ClassMeta {
        name: "Reader".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.Reader".into(),
        default_interface: Some(InterfaceMeta {
            name: "IReader".into(),
            namespace: "Contoso".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
            methods: vec![
                overloaded_method("Pick2", 7, TypeMeta::F64),
                overloaded_method("Pick", 6, TypeMeta::I32),
            ],
            ..Default::default()
        }),
        ..Default::default()
    };
    let known = HashSet::from(["Reader".into()]);

    let pyi = python_stub::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());

    assert!(pyi.contains("overload, timedelta"));
    assert_eq!(pyi.matches("    @overload\n").count(), 2);
    assert!(
        pyi.find("def pick(self, value: int)") < pyi.find("def pick(self, value: float)"),
        "{pyi}"
    );
}
