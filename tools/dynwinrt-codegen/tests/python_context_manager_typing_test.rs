// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod common;

use std::collections::HashSet;

use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta};

#[test]
fn closable_context_manager_never_suppresses_exceptions() {
    let class = ClassMeta {
        name: "Resource".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.Resource".into(),
        required_interfaces: vec![InterfaceMeta {
            name: "IClosable".into(),
            namespace: "Windows.Foundation".into(),
            iid: "30d5a829-7fa4-4026-83bb-d75bae4ea99e".into(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let stub = common::generate_class_stub(
        &class,
        &HashSet::from(["IClosable".into(), "Resource".into()]),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(stub.contains("from typing import Literal"), "{stub}");
    assert!(
        stub.contains(
            "def __exit__(self, exc_type: object, exc_value: object, traceback: object) -> Literal[False]: ..."
        ),
        "{stub}"
    );
}
