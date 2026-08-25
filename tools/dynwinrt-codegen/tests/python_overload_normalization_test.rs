// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;
use std::path::Path;

use dynwinrt_codegen::codegen::{python, python_stub};
use dynwinrt_codegen::meta::{self, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::{TypeMeta, TypeMeta::AsyncOperation};

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

fn storage_file() -> TypeMeta {
    TypeMeta::RuntimeClass {
        namespace: "Windows.Storage".into(),
        name: "StorageFile".into(),
        default_interface: None,
    }
}

fn method(name: &str, index: usize, params: Vec<ParamMeta>) -> MethodMeta {
    MethodMeta {
        name: name.into(),
        raw_name: name.into(),
        vtable_index: index,
        params,
        return_type: Some(AsyncOperation(Box::new(storage_file()))),
        ..Default::default()
    }
}

#[test]
fn default_option_method_is_one_python_overload_group_with_legacy_alias() {
    let interface = InterfaceMeta {
        name: "IStorageFolder".into(),
        namespace: "Windows.Storage".into(),
        iid: "11111111-1111-1111-1111-111111111111".into(),
        methods: vec![
            method(
                "CreateFileAsync",
                6,
                vec![
                    ParamMeta {
                        name: "desiredName".into(),
                        typ: TypeMeta::String,
                        direction: ParamDirection::In,
                    },
                    ParamMeta {
                        name: "options".into(),
                        typ: TypeMeta::Enum {
                            namespace: "Windows.Storage".into(),
                            name: "CreationCollisionOption".into(),
                            underlying: Box::new(TypeMeta::I32),
                            members: Vec::new(),
                            is_flags: false,
                            doc: None,
                            deprecated: None,
                        },
                        direction: ParamDirection::In,
                    },
                ],
            ),
            method(
                "CreateFileAsyncOverloadDefaultOptions",
                7,
                vec![ParamMeta {
                    name: "desiredName".into(),
                    typ: TypeMeta::String,
                    direction: ParamDirection::In,
                }],
            ),
        ],
        ..Default::default()
    };
    let known = HashSet::from([
        "CreationCollisionOption".into(),
        "IStorageFolder".into(),
        "StorageFile".into(),
    ]);

    let runtime = python::generate_interface(&interface, &known, &HashSet::new());
    let stub = python_stub::generate_interface_stub(&interface, &known, &HashSet::new());

    assert_eq!(runtime.matches("def create_file_async(self, *args").count(), 1);
    assert!(runtime.contains("def _create_file_async_6("), "{runtime}");
    assert!(runtime.contains("def _create_file_async_7("), "{runtime}");
    assert!(
        runtime.contains(
            "create_file_async_overload_default_options = create_file_async"
        ),
        "{runtime}"
    );
    assert_eq!(stub.matches("def create_file_async(").count(), 2, "{stub}");
    assert_eq!(stub.matches("@overload").count(), 2, "{stub}");
    assert_eq!(
        stub.matches("def create_file_async_overload_default_options(")
            .count(),
        1,
        "{stub}"
    );
}

#[test]
fn real_storage_folder_default_options_method_is_normalized() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }
    let class = meta::parse_class(WINDOWS_WINMD, "Windows.Storage", "StorageFolder")
        .expect("StorageFolder metadata");
    let known = HashSet::from([
        "CreationCollisionOption".into(),
        "StorageFile".into(),
        "StorageFolder".into(),
    ]);

    let runtime =
        python::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let stub =
        python_stub::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());

    assert!(runtime.contains("def create_file_async(self, *args"), "{runtime}");
    assert!(
        runtime.contains(
            "create_file_async_overload_default_options = create_file_async"
        ),
        "{runtime}"
    );
    assert_eq!(stub.matches("def create_file_async(").count(), 2, "{stub}");
}
