// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use serde_json::{Value, json};

fn function_entry() -> FunctionEntry {
    Registry::builtin()
        .unwrap()
        .functions
        .get(&(
            "Windows.Win32.Foundation".into(),
            "Apis".into(),
            "FreeLibrary".into(),
        ))
        .unwrap()
        .clone()
}

fn aggregate_entry() -> TypeEntry {
    Registry::builtin()
        .unwrap()
        .type_entry("Windows.Win32.Security", "SECURITY_ATTRIBUTES")
        .unwrap()
        .clone()
}

fn load_modified_group(name: &str, edit: impl FnOnce(&mut Value)) -> Result<Registry, String> {
    let mut files = FILES
        .iter()
        .map(|(name, data)| ((*name).to_owned(), (*data).to_owned()))
        .collect::<Vec<_>>();
    let selected = files
        .iter_mut()
        .find(|(candidate, _)| candidate == name)
        .unwrap();
    let mut group: Value = serde_json::from_str(&selected.1).unwrap();
    edit(&mut group);
    selected.1 = group.to_string();
    let digest = sha256(selected.1.as_bytes());
    let mut manifest: Value = serde_json::from_str(MANIFEST).unwrap();
    manifest["files"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|file| file["file"] == name)
        .unwrap()["sha256"] = digest.into();
    Registry::load(
        &manifest.to_string(),
        SCHEMA,
        &files
            .iter()
            .map(|(name, data)| (name.as_str(), data.as_str()))
            .collect::<Vec<_>>(),
    )
}

#[test]
fn every_closed_function_effect_has_typed_json_and_rejects_extra_or_missing_fields() {
    let cases: Vec<(Value, fn(FunctionEffect) -> bool)> = vec![
        (
            json!({"kind":"counted-buffer","parameter":0,"countParameter":1,"nullable":true}),
            |e| {
                matches!(
                    e,
                    FunctionEffect::CountedBuffer {
                        parameter: 0,
                        count_parameter: 1,
                        nullable: true
                    }
                )
            },
        ),
        (
            json!({"kind":"unsupported-count-unit","parameter":1}),
            |e| matches!(e, FunctionEffect::UnsupportedCountUnit { parameter: 1 }),
        ),
        (json!({"kind":"owned-return","cleanup":"local-free"}), |e| {
            matches!(
                e,
                FunctionEffect::OwnedReturn {
                    cleanup: Cleanup::LocalFree
                }
            )
        }),
        (json!({"kind":"borrowed-return"}), |e| {
            matches!(e, FunctionEffect::BorrowedReturn {})
        }),
        (
            json!({"kind":"owned-output","parameter":2,"cleanup":"close-handle"}),
            |e| {
                matches!(
                    e,
                    FunctionEffect::OwnedOutput {
                        parameter: 2,
                        cleanup: Cleanup::CloseHandle
                    }
                )
            },
        ),
        (
            json!({"kind":"consumed-input","parameter":0,"cleanup":"free-library"}),
            |e| {
                matches!(
                    e,
                    FunctionEffect::ConsumedInput {
                        parameter: 0,
                        cleanup: Cleanup::FreeLibrary
                    }
                )
            },
        ),
        (json!({"kind":"mutable-string","parameter":1}), |e| {
            matches!(e, FunctionEffect::MutableString { parameter: 1 })
        }),
        (
            json!({"kind":"call-contract","contract":{}}),
            |e| matches!(e, FunctionEffect::CallContract { contract } if contract.is_empty()),
        ),
        (
            json!({"kind":"overlapped-io","operation":"read","fileParameter":0,"bufferParameter":1,"countParameter":2,"transferredParameter":3,"overlappedParameter":4}),
            |e| {
                matches!(
                    e,
                    FunctionEffect::OverlappedIo {
                        operation: AsyncIoKind::Read,
                        file_parameter: 0,
                        buffer_parameter: 1,
                        count_parameter: 2,
                        transferred_parameter: 3,
                        overlapped_parameter: 4
                    }
                )
            },
        ),
        (json!({"kind":"subsystem","subsystem":"winsock"}), |e| {
            matches!(
                e,
                FunctionEffect::Subsystem {
                    subsystem: Subsystem::Winsock
                }
            )
        }),
        (json!({"kind":"subsystem-exempt"}), |e| {
            matches!(e, FunctionEffect::SubsystemExempt {})
        }),
        (
            json!({"kind":"managed-lifecycle","subsystem":"media-foundation"}),
            |e| {
                matches!(
                    e,
                    FunctionEffect::ManagedLifecycle {
                        subsystem: Subsystem::MediaFoundation
                    }
                )
            },
        ),
    ];
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    for (value, accepts) in cases {
        let parsed = serde_json::from_value::<FunctionEffect>(value.clone()).unwrap();
        assert!(accepts(parsed), "{value}");
        let definition = schema["$defs"]["effect"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .find(|definition| definition["properties"]["kind"]["const"] == value["kind"])
            .unwrap();
        assert_eq!(definition["additionalProperties"], false);
        let required = definition["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            required,
            value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect()
        );
        for field in value.as_object().unwrap().keys() {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<FunctionEffect>(missing).is_err(),
                "missing {field}: {value}"
            );
        }
        let mut extra = value;
        extra["runtimeMethod"] = "unchecked".into();
        assert!(serde_json::from_value::<FunctionEffect>(extra).is_err());
    }
}

#[test]
fn every_closed_type_and_aggregate_field_meaning_is_typed() {
    let cases: Vec<(Value, fn(TypeMeaning) -> bool)> = vec![
        (json!({"kind":"scalar","scalar":"bool32"}), |t| {
            matches!(
                t,
                TypeMeaning::Scalar {
                    scalar: RawScalar::Bool32
                }
            )
        }),
        (json!({"kind":"handle"}), |t| {
            matches!(t, TypeMeaning::Handle {})
        }),
        (json!({"kind":"data-pointer"}), |t| {
            matches!(t, TypeMeaning::DataPointer {})
        }),
        (
            json!({"kind":"string-pointer","encoding":"utf16","isConst":true}),
            |t| {
                matches!(
                    t,
                    TypeMeaning::StringPointer {
                        encoding: Encoding::Utf16,
                        is_const: true
                    }
                )
            },
        ),
        (
            json!({"kind":"string-pointer","encoding":"ansi","isConst":false}),
            |t| {
                matches!(
                    t,
                    TypeMeaning::StringPointer {
                        encoding: Encoding::Ansi,
                        is_const: false
                    }
                )
            },
        ),
        (json!({"kind":"function-pointer"}), |t| {
            matches!(t, TypeMeaning::FunctionPointer {})
        }),
        (json!({"kind":"unsupported"}), |t| {
            matches!(t, TypeMeaning::Unsupported {})
        }),
    ];
    for (mut value, accepts) in cases {
        assert!(accepts(serde_json::from_value(value.clone()).unwrap()));
        value["typescript"] = "unknown".into();
        assert!(serde_json::from_value::<TypeMeaning>(value).is_err());
    }
    for (token, expected) in [
        ("bool8", RawScalar::Bool8),
        ("i8", RawScalar::I8),
        ("u8", RawScalar::U8),
        ("i16", RawScalar::I16),
        ("u16", RawScalar::U16),
        ("i32", RawScalar::I32),
        ("u32", RawScalar::U32),
        ("i64", RawScalar::I64),
        ("u64", RawScalar::U64),
        ("f32", RawScalar::F32),
        ("f64", RawScalar::F64),
        ("char16", RawScalar::Char16),
        ("bool32", RawScalar::Bool32),
        ("native-isize", RawScalar::NativeIsize),
        ("native-usize", RawScalar::NativeUsize),
    ] {
        let value =
            serde_json::from_value::<TypeMeaning>(json!({"kind":"scalar","scalar":token})).unwrap();
        assert!(
            matches!(value,TypeMeaning::Scalar { scalar } if scalar == expected),
            "{token}"
        );
    }
    let fields: Vec<(Value, fn(FieldContract) -> bool)> = vec![
        (
            json!({"kind":"retained-pointer","nullable":true,"optional":false}),
            |v| {
                matches!(
                    v,
                    FieldContract::RetainedPointer {
                        nullable: true,
                        optional: false
                    }
                )
            },
        ),
        (json!({"kind":"null-pointer"}), |v| {
            matches!(v, FieldContract::NullPointer {})
        }),
        (json!({"kind":"borrowed-handle"}), |v| {
            matches!(v, FieldContract::BorrowedHandle {})
        }),
        (
            json!({"kind":"owned-handle","cleanup":"close-handle"}),
            |v| {
                matches!(
                    v,
                    FieldContract::OwnedHandle {
                        cleanup: Cleanup::CloseHandle
                    }
                )
            },
        ),
        (json!({"kind":"boolean-input","optional":true}), |v| {
            matches!(v, FieldContract::BooleanInput { optional: true })
        }),
        (json!({"kind":"output-u32"}), |v| {
            matches!(v, FieldContract::OutputU32 {})
        }),
    ];
    for (mut value, accepts) in fields {
        assert!(accepts(serde_json::from_value(value.clone()).unwrap()));
        value["allocatorCode"] = "not data".into();
        assert!(serde_json::from_value::<FieldContract>(value).is_err());
    }
    for value in [
        json!({"kind":"scalar","scalar":"i128"}),
        json!({"kind":"string-pointer","encoding":"utf8","isConst":true}),
        json!({"kind":"union","rust":"native_wrapper()"}),
        json!({"kind":"scalar","scalar":{"u32":null}}),
    ] {
        assert!(serde_json::from_value::<TypeMeaning>(value).is_err());
    }
}

#[test]
fn manifest_and_group_validation_rejects_each_integrity_and_schema_boundary() {
    let mutations: &[fn(&mut Value)] = &[
        |v| v["schemaVersion"] = 1.into(),
        |v| v["metadata"]["package"] = "unreviewed".into(),
        |v| v["metadata"]["version"] = "new".into(),
        |v| v["metadata"]["sha256"] = "0".repeat(64).into(),
        |v| v["schema"]["file"] = "other.json".into(),
        |v| v["schema"]["sha256"] = "0".repeat(64).into(),
        |v| {
            v["files"].as_array_mut().unwrap().pop();
        },
        |v| v["files"][1] = v["files"][0].clone(),
        |v| v["files"][0]["file"] = "..\\outside.json".into(),
        |v| v["files"][0]["sha256"] = "not a digest".into(),
        |v| v["metadata"]["unknown"] = true.into(),
        |v| v["files"][0]["unknown"] = true.into(),
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut manifest = serde_json::from_str::<Value>(MANIFEST).unwrap();
        mutate(&mut manifest);
        assert!(
            Registry::load(&manifest.to_string(), SCHEMA, FILES).is_err(),
            "manifest mutation {index}"
        );
    }
    for (name, _) in FILES {
        assert!(
            load_modified_group(name, |group| group["schemaVersion"] = 0.into()).is_err(),
            "{name}"
        );
        assert!(
            load_modified_group(name, |group| group["extra"] = true.into()).is_err(),
            "{name}"
        );
        assert!(
            load_modified_group(name, |group| group["entries"][0]["extra"] = true.into()).is_err(),
            "{name}"
        );
    }
    let mut manifest = serde_json::from_str::<Value>(MANIFEST).unwrap();
    manifest["files"].as_array_mut().unwrap().reverse();
    assert!(Registry::load(&manifest.to_string(), SCHEMA, FILES).is_ok());
    assert!(
        load_modified_group("function-contracts.json", |group| {
            let mut entry = group["entries"][0].clone();
            entry["id"] = "tests.additional-existing-kind.v1".into();
            entry["selector"]["name"] = "AdditionalCleanup".into();
            entry["selector"]["entryPoint"] = "AdditionalCleanup".into();
            group["entries"].as_array_mut().unwrap().push(entry);
        })
        .is_ok(),
        "same-kind additions must not require a Rust manifest checksum change"
    );
}

#[test]
fn contract_conflicts_and_invalid_roles_are_rejected_not_prioritized() {
    let base = function_entry();
    let mut same_id = base.clone();
    same_id.selector.name += "Different";
    assert!(Registry::validate(vec![base.clone(), same_id], Vec::new(), Vec::new()).is_err());
    for effects in [
        vec![
            FunctionEffect::OwnedReturn {
                cleanup: Cleanup::LocalFree,
            },
            FunctionEffect::BorrowedReturn {},
        ],
        vec![FunctionEffect::CountedBuffer {
            parameter: 0,
            count_parameter: 0,
            nullable: true,
        }],
        vec![
            FunctionEffect::CountedBuffer {
                parameter: 0,
                count_parameter: 1,
                nullable: true,
            },
            FunctionEffect::UnsupportedCountUnit { parameter: 0 },
        ],
        vec![
            FunctionEffect::OwnedOutput {
                parameter: 1,
                cleanup: Cleanup::CloseHandle,
            },
            FunctionEffect::ConsumedInput {
                parameter: 1,
                cleanup: Cleanup::CloseHandle,
            },
        ],
        vec![FunctionEffect::OwnedReturn {
            cleanup: Cleanup::None,
        }],
        vec![FunctionEffect::ConsumedInput {
            parameter: 1024,
            cleanup: Cleanup::CloseHandle,
        }],
        vec![FunctionEffect::CallContract {
            contract: CallContract {
                outputs: vec![OutputRule {
                    parameter: 0,
                    when: Condition::default(),
                    action: OutputAction::AliasInput { parameter: 0 },
                }],
                resource_effects: Vec::new(),
            },
        }],
        vec![FunctionEffect::OverlappedIo {
            operation: AsyncIoKind::Read,
            file_parameter: 0,
            buffer_parameter: 1,
            count_parameter: 2,
            transferred_parameter: 3,
            overlapped_parameter: 3,
        }],
        vec![
            FunctionEffect::Subsystem {
                subsystem: Subsystem::Winsock,
            },
            FunctionEffect::SubsystemExempt {},
        ],
    ] {
        let mut entry = base.clone();
        entry.contracts = effects;
        assert!(Registry::validate(vec![entry], Vec::new(), Vec::new()).is_err());
    }
    let base = aggregate_entry();
    let mutations: &[fn(&mut TypeEntry)] = &[
        |e| e.selector.namespace = "Untrusted".into(),
        |e| e.selector.name = "../Unsafe".into(),
        |e| e.selector.source_fingerprint = "not a digest".into(),
        |e| e.meaning = Some(TypeMeaning::Handle {}),
        |e| {
            let a = e.aggregate.as_mut().unwrap();
            a.fields.push(a.fields[0].clone());
        },
        |e| {
            e.aggregate.as_mut().unwrap().fields[0].contract = FieldContract::OwnedHandle {
                cleanup: Cleanup::None,
            }
        },
        |e| e.aggregate.as_mut().unwrap().size_field = Some("not.a.field".into()),
    ];
    for mutate in mutations {
        let mut entry = base.clone();
        mutate(&mut entry);
        assert!(Registry::validate(Vec::new(), vec![entry], Vec::new()).is_err());
    }
    let mut duplicate = base.clone();
    duplicate.id += ".another";
    assert!(Registry::validate(Vec::new(), vec![base, duplicate], Vec::new()).is_err());
    for domains in [
        vec![DomainPolicy::ProviderLifecycle {
            dll: "..\\mapi32.dll".into(),
        }],
        vec![DomainPolicy::Subsystem {
            namespace: "Unknown".into(),
            subsystem: Subsystem::Winsock,
        }],
        vec![
            DomainPolicy::ProviderLifecycle {
                dll: "mapi32.dll".into(),
            },
            DomainPolicy::ProviderLifecycle {
                dll: "MAPI32.dll".into(),
            },
        ],
    ] {
        assert!(Registry::validate(Vec::new(), Vec::new(), domains).is_err());
    }
}

#[test]
fn anonymous_layout_recipes_are_bounded_and_retain_native_field_identity() {
    let mut recipe = LayoutRecipe {
        name: "Anonymous".into(),
        kind: RecordKind::Union,
        fields: vec![RecipeField {
            name: "value".into(),
            typ: RecipeType::Scalar {
                scalar: RawScalar::U32,
            },
        }],
    };
    assert!(validate_recipe(&recipe, 0).is_ok());
    assert!(validate_recipe(&recipe, 17).is_err());
    recipe.fields.push(recipe.fields[0].clone());
    assert!(validate_recipe(&recipe, 0).is_err());
    recipe.fields.pop();
    recipe.fields[0].typ = RecipeType::Named {
        namespace: "Tests".into(),
        name: "Unproven".into(),
    };
    assert!(validate_recipe(&recipe, 0).is_err());
    recipe.fields.clear();
    assert!(validate_recipe(&recipe, 0).is_err());
}

#[test]
fn real_aggregate_provenance_and_nested_field_drift_fail_closed() {
    let Some(path) = metadata() else { return };
    let raw = metadata_function(&path, "Windows.Win32.System.Threading", "CreateProcessW");
    let RawBaseType::Named {
        kind: RawNamedKind::NativeStruct { layout },
        ..
    } = &raw.parameters[2].typ.base
    else {
        panic!("SECURITY_ATTRIBUTES retains its native layout");
    };
    let registry = Registry::builtin().unwrap();
    assert!(registry.layout_contract(layout).unwrap().is_some());
    let mutations: &[fn(&mut crate::win32_metadata::RawNativeLayoutSet)] = &[
        |l| l.variants[0].fields[0].name.push('x'),
        |l| l.variants[0].fields[1].typ.pointer_depth += 1,
        |l| l.variants[0].declared_size = Some(1),
        |l| l.variants[0].packing = crate::win32_metadata::RawPacking::Explicit(1),
        |l| std::sync::Arc::make_mut(l.evidence.as_mut().unwrap()).metadata_sha256 = "0".repeat(64),
        |l| {
            std::sync::Arc::make_mut(l.evidence.as_mut().unwrap())
                .selector
                .source_fingerprint = "0".repeat(64)
        },
    ];
    for mutate in mutations {
        let mut changed = (**layout).clone();
        mutate(&mut changed);
        assert!(registry.layout_contract(&changed).is_err());
    }
}
