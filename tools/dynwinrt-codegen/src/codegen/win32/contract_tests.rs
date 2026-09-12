// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
#[path = "call_contract_json_tests.rs"]
mod call_contract_json;
#[path = "contract_validation_tests.rs"]
mod validation;

use crate::codegen::win32::{
    ir::{
        AbiType, CallingConvention, Condition, Conversion, Direction, EnumUnderlying,
        InputExpression, OutputRule, ProjectedOutput, ReturnShape, RuntimeParameter, RuntimePlan,
        StringEncoding, SuccessRule, SurfaceParameter, SurfaceType, ValueType,
    },
    test_support::{metadata, metadata_function, project_one, semantic},
};
use crate::win32_metadata::{
    RawBaseType, RawBufferSize, RawCallingConvention, RawConstness, RawDirection, RawNamedKind,
    RawStatusSemantics,
};

#[test]
fn win32_contracts_are_strict_and_integrity_checked() {
    let registry = Registry::builtin().unwrap();
    let schema: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
    let dll_pattern = regex::Regex::new(
        schema["$defs"]["functionSelector"]["properties"]["dll"]["pattern"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    for entry in registry.functions.values() {
        assert!(dll_pattern.is_match(&entry.selector.dll), "{}", entry.id);
    }
    assert!(registry.functions.contains_key(&(
        "Windows.Win32.Foundation".into(),
        "Apis".into(),
        "FreeLibrary".into(),
    )));
    assert!(matches!(
        registry
            .type_entry("Windows.Win32.Foundation", "BOOL")
            .unwrap()
            .meaning,
        Some(TypeMeaning::Scalar {
            scalar: RawScalar::Bool32
        })
    ));
    assert!(Registry::load(MANIFEST, &format!("{SCHEMA} "), FILES).is_err());
    let mut changed = FILES.to_vec();
    let altered = format!("{} ", changed[0].1);
    changed[0].1 = &altered;
    assert!(Registry::load(MANIFEST, SCHEMA, &changed).is_err());
    let mut manifest: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
    manifest["unexpected"] = true.into();
    assert!(Registry::load(&manifest.to_string(), SCHEMA, FILES).is_err());
    for data in [
        r#"{"kind":"callback","body":"anything"}"#,
        r#"{"kind":"borrowed-return","js":"unsafe()"}"#,
        r#"{"kind":"owned-return","cleanup":"arbitraryCleaner"}"#,
        r#"{"kind":"owned-return","cleanup":{"close-handle":null}}"#,
        r#"{"kind":"overlapped-io","runtimeMethod":"beginWhatever"}"#,
        r#"{"kind":"reject-hkey-performance-data","parameter":0}"#,
        r#"{"kind":"hkey-performance-data-count","handleParameter":0,"countParameter":5,"undefinedOn":"more-data"}"#,
        r#"{"kind":"borrowed-predefined-hkey-output","handleParameter":0,"stringParameter":1,"outputParameter":4}"#,
        r#"{"kind":"call-contract","contract":{"outputs":[],"javascript":"unchecked()"} }"#,
    ] {
        assert!(
            serde_json::from_str::<FunctionEffect>(data).is_err(),
            "{data}"
        );
    }
    assert!(
        serde_json::from_str::<TypeMeaning>(r#"{"kind":"handle","typescript":"bigint"}"#).is_err()
    );
    assert!(
        serde_json::from_str::<TypeMeaning>(r#"{"kind":"scalar","scalar":{"u32":null}}"#).is_err()
    );
    assert!(
        serde_json::from_str::<FieldContract>(r#"{"kind":"null-pointer","snippet":"0"}"#).is_err()
    );
    let mut entries = registry.functions.values().cloned().collect::<Vec<_>>();
    entries.push(entries[0].clone());
    assert!(Registry::validate(entries.clone(), Vec::new(), Vec::new()).is_err());
    entries.last_mut().unwrap().id += ".duplicate";
    assert!(Registry::validate(entries, Vec::new(), Vec::new()).is_err());
    let mut one = registry.functions.values().next().unwrap().clone();
    one.contracts.push(one.contracts[0].clone());
    assert!(Registry::validate(vec![one], Vec::new(), Vec::new()).is_err());
}

#[test]
fn win32_contract_text_pins_use_lf_normalization_without_changing_binary_fingerprints() {
    let files = FILES
        .iter()
        .map(|(name, contents)| (*name, contents.replace('\n', "\r\n")))
        .collect::<Vec<_>>();
    let files = files
        .iter()
        .map(|(name, contents)| (*name, contents.as_str()))
        .collect::<Vec<_>>();
    Registry::load(
        &MANIFEST.replace('\n', "\r\n"),
        &SCHEMA.replace('\n', "\r\n"),
        &files,
    )
    .unwrap();
    assert_ne!(sha256(b"native\r\nbytes"), sha256(b"native\nbytes"));
}

#[test]
fn win32_contracts_pin_every_official_selector() {
    let Some(path) = metadata() else {
        return;
    };
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(sha256(&bytes), METADATA_SHA256);
    let index = reader::Index::new(vec![reader::File::new(bytes).unwrap()]);
    let registry = Registry::builtin().unwrap();
    let mut found = BTreeSet::new();
    for definition in index.all() {
        for method in definition.methods() {
            let key = (
                definition.namespace().to_string(),
                definition.name().to_string(),
                method.name().to_string(),
            );
            let Some(entry) = registry.functions.get(&key) else {
                continue;
            };
            assert_eq!(
                method_fingerprint(definition.namespace(), definition.name(), &method),
                entry.selector.source_fingerprint,
                "{}",
                entry.id
            );
            let import = method.impl_map().unwrap();
            assert_eq!(import.import_scope().name(), entry.selector.dll);
            assert_eq!(import.import_name(), entry.selector.entry_point);
            assert!(found.insert(key), "duplicate official declaration");
        }
    }
    assert_eq!(found.len(), registry.functions.len());
    for entry in registry.types.values() {
        assert_eq!(
            type_fingerprint(&index, &entry.selector.namespace, &entry.selector.name),
            entry.selector.source_fingerprint,
            "{}",
            entry.id
        );
    }
}

#[test]
fn win32_contract_drift_and_forged_facts_fail_closed() {
    let Some(path) = metadata() else {
        return;
    };
    let raw = crate::win32_metadata::parse_apis(&path, "Windows.Win32.System.Registry", "Apis")
        .unwrap()
        .functions
        .into_iter()
        .find(|f| f.name == "RegOpenKeyExW")
        .unwrap();
    let registry = Registry::builtin().unwrap();
    registry.function_policy(&raw).unwrap();
    let key = (
        raw.namespace.clone(),
        raw.container.clone(),
        raw.name.clone(),
    );
    let entry = registry.function_entry(&raw).unwrap().clone();
    let mutations: &[fn(&mut FunctionSelector)] = &[
        |s| s.namespace.push('x'),
        |s| s.container.push('x'),
        |s| s.name.push('x'),
        |s| s.dll.push('x'),
        |s| s.entry_point.push('x'),
        |s| s.calling_convention.push('x'),
        |s| s.architectures = 1,
        |s| s.source_fingerprint = "0".repeat(64),
    ];
    for mutation in mutations {
        let mut entry = entry.clone();
        mutation(&mut entry.selector);
        let changed = Registry {
            functions: BTreeMap::from([(key.clone(), entry)]),
            types: BTreeMap::new(),
            domains: Vec::new(),
        };
        assert!(changed.function_policy(&raw).is_err());
    }

    let mutations: &[fn(&mut RawFunction)] = &[
        |f| f.parameters[0].nullable = true,
        |f| f.parameters[0].typ.pointer_depth = 1,
        |f| f.parameters[1].typ.constness = crate::win32_metadata::RawConstness::Mutable,
        |f| f.parameters[1].direction = crate::win32_metadata::RawDirection::Out,
        |f| f.parameters[4].free_with = Some("CloseHandle".into()),
        |f| f.supports_last_error = !f.supports_last_error,
        |f| f.evidence = None,
    ];
    for mutation in mutations {
        let mut changed = raw.clone();
        mutation(&mut changed);
        assert!(registry.function_policy(&changed).is_err());
    }
    let mut forged = entry;
    forged.contracts = vec![FunctionEffect::OwnedOutput {
        parameter: 0,
        cleanup: Cleanup::RegCloseKey,
    }];
    let changed = Registry {
        functions: BTreeMap::from([(key, forged)]),
        types: BTreeMap::new(),
        domains: Vec::new(),
    };
    assert!(changed.function_policy(&raw).is_err());
}

#[test]
fn win32_free_library_has_a_consuming_hmodule_abi_and_ordered_bool_result() {
    let Some(path) = metadata() else {
        return;
    };
    let raw = metadata_function(&path, "Windows.Win32.Foundation", "FreeLibrary");
    assert_eq!(raw.calling_convention, RawCallingConvention::System);
    assert!(raw.architectures.x86 && raw.architectures.x64 && raw.architectures.arm64);
    assert!(!raw.variadic);
    assert!(raw.supports_last_error);
    assert_eq!(raw.return_status, RawStatusSemantics::None);
    assert_eq!(raw.return_type.base, RawBaseType::Scalar(RawScalar::Bool32));
    let [module] = raw.parameters.as_slice() else {
        panic!("FreeLibrary takes one HMODULE")
    };
    assert_eq!(module.name, "hLibModule");
    assert_eq!(module.direction, RawDirection::In);
    assert_eq!(module.typ.pointer_depth, 0);
    assert!(!module.nullable && !module.reserved);
    assert!(
        matches!(&module.typ.base, RawBaseType::Named { namespace, name, kind:RawNamedKind::Handle { cleanup:Some(cleanup) } }
        if namespace == "Windows.Win32.Foundation" && name == "HMODULE" && cleanup == "FreeLibrary")
    );

    let projected = project_one(raw);
    assert_eq!(projected.metadata_name, "FreeLibrary");
    assert_eq!(projected.js_name, "freeLibrary");
    assert_eq!(projected.unicode_alias, None);
    assert_eq!(projected.subsystem, None);
    assert!(projected.runtime.call_contract.is_empty());
    assert_eq!(
        projected.parameters,
        vec![surface(
            "hLibModule",
            SurfaceType::ManagedResource,
            false,
            None
        )]
    );
    assert_eq!(
        projected.inputs,
        vec![InputExpression::Surface {
            parameter_index: 0,
            conversion: Conversion::ResourceInput(Cleanup::FreeLibrary),
        }]
    );
    let mut module = slot(
        AbiType::Handle,
        Direction::In,
        false,
        Cleanup::None,
        Cleanup::FreeLibrary,
    );
    module.consumes_resource = true;
    assert_eq!(
        projected.runtime,
        RuntimePlan {
            dll: "KERNEL32.dll".into(),
            entry_point: "FreeLibrary".into(),
            parameters: vec![module],
            return_abi: Some(AbiType::Bool32),
            return_aggregate: None,
            return_cleanup: Cleanup::None,
            success_rule: SuccessRule::ReturnNonZero,
            capture_last_error: true,
            calling_convention: CallingConvention::System,
            call_contract: CallContract::default(),
        }
    );
    assert_eq!(
        projected.return_shape,
        ReturnShape::Object {
            status: false,
            return_value: Some((SurfaceType::Boolean, Conversion::Boolean)),
            outputs: Vec::new(),
            last_error: true,
        }
    );
}

#[test]
fn win32_registry_abis_keep_ownership_counts_and_status_validity_explicit() {
    let Some(path) = metadata() else {
        return;
    };
    let raw =
        crate::win32_metadata::parse_apis(&path, "Windows.Win32.System.Registry", "Apis").unwrap();
    for (suffix, encoding, conversion) in [
        ("A", StringEncoding::Ansi, Conversion::AnsiString),
        ("W", StringEncoding::Wide, Conversion::WideString),
    ] {
        let open = raw
            .functions
            .iter()
            .find(|f| f.name == format!("RegOpenKeyEx{suffix}"))
            .unwrap();
        assert_eq!(
            open.parameters
                .iter()
                .map(|p| p.direction)
                .collect::<Vec<_>>(),
            [
                RawDirection::In,
                RawDirection::In,
                RawDirection::In,
                RawDirection::In,
                RawDirection::Out
            ]
        );
        assert_eq!(
            open.parameters
                .iter()
                .map(|p| p.typ.pointer_depth)
                .collect::<Vec<_>>(),
            [0, 0, 0, 0, 1]
        );
        assert_eq!(open.parameters[1].typ.constness, RawConstness::Const);
        assert!(open.parameters[1].nullable);
        assert_eq!(open.return_status, RawStatusSemantics::ZeroIsSuccess);
        let native = semantic(open);
        assert_eq!(native.return_abi, Some(AbiType::U32));
        assert_eq!(native.success_rule, SuccessRule::ReturnZero);
        assert_eq!(native.parameters[4].cleanup, Cleanup::RegCloseKey);
        assert_eq!(native.parameters[1].typ, ValueType::StringPointer(encoding));
        let open = project_one(open.clone());
        assert_eq!(
            open.parameters,
            vec![
                surface("hKey", SurfaceType::Handle("HKEY".into()), false, None),
                surface("subKey", SurfaceType::String(encoding), true, None),
                surface("ulOptions", SurfaceType::Number, false, None),
                surface(
                    "samDesired",
                    SurfaceType::Enum("REG_SAM_FLAGS".into()),
                    false,
                    None
                ),
            ]
        );
        assert_eq!(
            open.inputs,
            vec![
                InputExpression::Surface {
                    parameter_index: 0,
                    conversion: Conversion::Handle
                },
                InputExpression::Surface {
                    parameter_index: 1,
                    conversion
                },
                InputExpression::Surface {
                    parameter_index: 2,
                    conversion: Conversion::U32
                },
                InputExpression::Surface {
                    parameter_index: 3,
                    conversion: Conversion::U32Flags
                },
            ]
        );
        assert_eq!(
            open.runtime.parameters,
            vec![
                slot(
                    AbiType::Handle,
                    Direction::In,
                    false,
                    Cleanup::None,
                    Cleanup::RegCloseKey
                ),
                slot(
                    AbiType::Pointer,
                    Direction::In,
                    true,
                    Cleanup::None,
                    Cleanup::None
                ),
                slot(
                    AbiType::U32,
                    Direction::In,
                    false,
                    Cleanup::None,
                    Cleanup::None
                ),
                slot(
                    AbiType::U32,
                    Direction::In,
                    false,
                    Cleanup::None,
                    Cleanup::None
                ),
                slot(
                    AbiType::Handle,
                    Direction::Out,
                    false,
                    Cleanup::RegCloseKey,
                    Cleanup::None
                ),
            ]
        );
        assert_status_plan(&open.runtime, &format!("RegOpenKeyEx{suffix}"));
        assert_eq!(
            open.return_shape,
            ReturnShape::Object {
                status: true,
                return_value: None,
                outputs: vec![ProjectedOutput {
                    name: "key".into(),
                    output_index: 0,
                    typ: SurfaceType::ResourceOrHandle,
                    conversion: Conversion::ResourceOrHandle,
                    may_be_unavailable: false,
                }],
                last_error: false,
            }
        );
        assert_eq!(
            open.runtime.call_contract,
            CallContract {
                outputs: vec![OutputRule {
                    parameter: 4,
                    when: Condition {
                        inputs: vec![
                            InputPredicate::HandleIn {
                                parameter: 0,
                                values: vec![
                                    -2147483648,
                                    -2147483647,
                                    -2147483646,
                                    -2147483645,
                                    -2147483644,
                                    -2147483643,
                                    -2147483642,
                                    -2147483568,
                                    -2147483552,
                                ],
                            },
                            InputPredicate::NullOrEmpty {
                                parameter: 1,
                                element_width: if encoding == StringEncoding::Wide {
                                    2
                                } else {
                                    1
                                },
                            },
                        ],
                        return_value: None,
                    },
                    action: OutputAction::AliasInput { parameter: 0 },
                }],
                resource_effects: Vec::new(),
            }
        );
        assert_eq!(native.call_contract, open.runtime.call_contract);

        let query = raw
            .functions
            .iter()
            .find(|f| f.name == format!("RegQueryValueEx{suffix}"))
            .unwrap();
        assert_eq!(
            query
                .parameters
                .iter()
                .map(|p| p.direction)
                .collect::<Vec<_>>(),
            [
                RawDirection::In,
                RawDirection::In,
                RawDirection::In,
                RawDirection::Out,
                RawDirection::Out,
                RawDirection::InOut
            ]
        );
        assert_eq!(
            query
                .parameters
                .iter()
                .map(|p| p.typ.pointer_depth)
                .collect::<Vec<_>>(),
            [0, 0, 1, 1, 1, 1]
        );
        assert!(
            query.parameters[2].reserved
                && query.parameters[4].nullable
                && query.parameters[5].nullable
        );
        let buffer = query.parameters[4].buffer.as_ref().unwrap();
        assert_eq!(buffer.size, RawBufferSize::ByteCountParam(5));
        assert_eq!(buffer.element.base, RawBaseType::Scalar(RawScalar::U8));
        assert_eq!(buffer.element.pointer_depth, 0);
        let native = semantic(query);
        assert!(
            matches!(&native.parameters[3].typ, ValueType::Enum { name, underlying: EnumUnderlying::U32, .. } if name == "REG_VALUE_TYPE")
        );
        let query = project_one(query.clone());
        assert_eq!(
            query.parameters,
            vec![
                surface("hKey", SurfaceType::Handle("HKEY".into()), false, None),
                surface("valueName", SurfaceType::String(encoding), true, None),
                surface("data", SurfaceType::Buffer, true, Some(1)),
            ]
        );
        assert_eq!(
            query.inputs,
            vec![
                InputExpression::Surface {
                    parameter_index: 0,
                    conversion: Conversion::Handle
                },
                InputExpression::Surface {
                    parameter_index: 1,
                    conversion
                },
                InputExpression::NullPointer,
                InputExpression::Surface {
                    parameter_index: 2,
                    conversion: Conversion::DataPointer
                },
                InputExpression::BufferLength {
                    parameter_index: 2,
                    divisor: 1,
                    abi: AbiType::U32
                },
            ]
        );
        assert_eq!(
            query.runtime.parameters,
            vec![
                slot(
                    AbiType::Handle,
                    Direction::In,
                    false,
                    Cleanup::None,
                    Cleanup::RegCloseKey
                ),
                slot(
                    AbiType::Pointer,
                    Direction::In,
                    true,
                    Cleanup::None,
                    Cleanup::None
                ),
                slot(
                    AbiType::Pointer,
                    Direction::In,
                    true,
                    Cleanup::None,
                    Cleanup::None
                ),
                slot(
                    AbiType::U32,
                    Direction::Out,
                    false,
                    Cleanup::None,
                    Cleanup::None
                ),
                slot(
                    AbiType::Pointer,
                    Direction::In,
                    true,
                    Cleanup::None,
                    Cleanup::None
                ),
                slot(
                    AbiType::U32,
                    Direction::InOut,
                    false,
                    Cleanup::None,
                    Cleanup::None
                ),
            ]
        );
        assert_status_plan(&query.runtime, &format!("RegQueryValueEx{suffix}"));
        assert_eq!(
            query.return_shape,
            ReturnShape::Object {
                status: true,
                return_value: None,
                last_error: false,
                outputs: vec![
                    ProjectedOutput {
                        name: "type".into(),
                        output_index: 0,
                        typ: SurfaceType::Enum("REG_VALUE_TYPE".into()),
                        conversion: Conversion::Number,
                        may_be_unavailable: false,
                    },
                    ProjectedOutput {
                        name: "dataSize".into(),
                        output_index: 1,
                        typ: SurfaceType::Number,
                        conversion: Conversion::Number,
                        may_be_unavailable: true,
                    },
                ],
            }
        );
        assert_eq!(
            query.runtime.call_contract,
            CallContract {
                outputs: vec![OutputRule {
                    parameter: 5,
                    when: Condition {
                        inputs: vec![InputPredicate::BitsIn {
                            parameter: 0,
                            mask: 0xffff_ffff,
                            values: vec![0x8000_0004],
                        }],
                        return_value: Some(234),
                    },
                    action: OutputAction::Unavailable {},
                }],
                resource_effects: Vec::new(),
            }
        );
        assert_eq!(native.call_contract, query.runtime.call_contract);
        for function in [&open, &query] {
            assert_eq!(function.subsystem, None);
            assert_eq!(function.unicode_alias.is_some(), suffix == "W");
        }
    }
}

#[test]
fn win32_registry_base_open_and_get_value_use_exact_native_output_rules() {
    let Some(path) = metadata() else {
        return;
    };
    let raw =
        crate::win32_metadata::parse_apis(&path, "Windows.Win32.System.Registry", "Apis").unwrap();
    for (suffix, encoding, width) in [
        ("A", StringEncoding::Ansi, 1),
        ("W", StringEncoding::Wide, 2),
    ] {
        let open = raw
            .functions
            .iter()
            .find(|f| f.name == format!("RegOpenKey{suffix}"))
            .unwrap();
        assert_eq!(open.parameters.len(), 3);
        assert_eq!(open.parameters[0].direction, RawDirection::In);
        assert_eq!(open.parameters[0].typ.pointer_depth, 0);
        assert!(open.parameters[1].nullable);
        assert_eq!(open.parameters[1].typ.constness, RawConstness::Const);
        assert_eq!(open.parameters[2].direction, RawDirection::Out);
        assert_eq!(open.parameters[2].typ.pointer_depth, 1);
        let native = semantic(open);
        assert_eq!(native.parameters[0].resource_cleanup, Cleanup::RegCloseKey);
        assert_eq!(native.parameters[1].typ, ValueType::StringPointer(encoding));
        assert_eq!(native.parameters[2].cleanup, Cleanup::RegCloseKey);
        assert_eq!(
            native.call_contract,
            CallContract {
                outputs: vec![OutputRule {
                    parameter: 2,
                    when: Condition {
                        inputs: vec![InputPredicate::NullOrEmpty {
                            parameter: 1,
                            element_width: width,
                        }],
                        return_value: None,
                    },
                    action: OutputAction::AliasInput { parameter: 0 },
                }],
                resource_effects: Vec::new(),
            }
        );
        let projected = project_one(open.clone());
        assert_eq!(projected.parameters.len(), 2);
        assert_eq!(
            projected.runtime.parameters[2].cleanup,
            Cleanup::RegCloseKey
        );
        assert_eq!(projected.runtime.call_contract, native.call_contract);
        assert_status_plan(&projected.runtime, &format!("RegOpenKey{suffix}"));
        assert_eq!(
            projected.return_shape,
            ReturnShape::Object {
                status: true,
                return_value: None,
                outputs: vec![ProjectedOutput {
                    name: "key".into(),
                    output_index: 0,
                    typ: SurfaceType::ResourceOrHandle,
                    conversion: Conversion::ResourceOrHandle,
                    may_be_unavailable: false,
                }],
                last_error: false,
            }
        );

        let get = raw
            .functions
            .iter()
            .find(|f| f.name == format!("RegGetValue{suffix}"))
            .unwrap();
        assert_eq!(get.parameters.len(), 7);
        assert_eq!(get.parameters[5].direction, RawDirection::Out);
        assert_eq!(get.parameters[5].typ.pointer_depth, 1);
        assert_eq!(
            get.parameters[5].buffer.as_ref().unwrap().size,
            RawBufferSize::ByteCountParam(6)
        );
        assert_eq!(get.parameters[6].direction, RawDirection::InOut);
        assert_eq!(get.parameters[6].typ.pointer_depth, 1);
        assert_eq!(
            get.parameters[6].typ.base,
            RawBaseType::Scalar(RawScalar::U32)
        );
        assert!(get.parameters[5].nullable && get.parameters[6].nullable);
        let native = semantic(get);
        assert_eq!(
            native.call_contract,
            CallContract {
                outputs: vec![OutputRule {
                    parameter: 6,
                    when: Condition {
                        inputs: vec![InputPredicate::BitsIn {
                            parameter: 0,
                            mask: 0xffff_ffff,
                            values: vec![0x8000_0004],
                        }],
                        return_value: Some(234),
                    },
                    action: OutputAction::Unavailable {},
                }],
                resource_effects: Vec::new(),
            }
        );
        let projected = project_one(get.clone());
        assert_eq!(projected.parameters.len(), 5);
        assert_eq!(projected.runtime.call_contract, native.call_contract);
        assert_eq!(projected.runtime.parameters[6].abi, AbiType::U32);
        assert_eq!(projected.runtime.parameters[6].direction, Direction::InOut);
        assert_eq!(
            projected.inputs[3],
            InputExpression::Surface {
                parameter_index: 3,
                conversion: Conversion::U32Flags
            }
        );
        assert_eq!(
            projected.inputs[5],
            InputExpression::BufferLength {
                parameter_index: 4,
                divisor: 1,
                abi: AbiType::U32
            }
        );
        assert_status_plan(&projected.runtime, &format!("RegGetValue{suffix}"));
        let ReturnShape::Object { outputs, .. } = &projected.return_shape else {
            panic!("native status and output slots")
        };
        assert_eq!(outputs.len(), 2);
        assert!(!outputs[0].may_be_unavailable);
        assert_eq!(
            outputs[1],
            ProjectedOutput {
                name: "dataSize".into(),
                output_index: 1,
                typ: SurfaceType::Number,
                conversion: Conversion::Number,
                may_be_unavailable: true,
            }
        );
    }
}

#[test]
fn win32_file_completion_modes_declare_shared_native_resource_state() {
    let Some(path) = metadata() else {
        return;
    };
    let raw = metadata_function(
        &path,
        "Windows.Win32.Storage.FileSystem",
        "SetFileCompletionNotificationModes",
    );
    assert_eq!(raw.parameters.len(), 2);
    assert_eq!(raw.parameters[0].direction, RawDirection::In);
    assert_eq!(raw.parameters[0].typ.pointer_depth, 0);
    assert_eq!(raw.parameters[1].direction, RawDirection::In);
    assert_eq!(raw.parameters[1].typ.pointer_depth, 0);
    assert_eq!(
        raw.parameters[1].typ.base,
        RawBaseType::Scalar(RawScalar::U8)
    );
    assert_eq!(raw.return_type.base, RawBaseType::Scalar(RawScalar::Bool32));
    assert!(raw.supports_last_error);
    let native = semantic(&raw);
    assert_eq!(native.parameters[0].cleanup, Cleanup::None);
    assert_eq!(native.parameters[0].resource_cleanup, Cleanup::CloseHandle);
    assert!(!native.parameters[0].consumes_resource);
    assert_eq!(
        native.call_contract,
        CallContract {
            outputs: Vec::new(),
            resource_effects: vec![ResourceEffect::AddFileCompletionModes {
                handle_parameter: 0,
                flags_parameter: 1,
            }],
        }
    );
    let projected = project_one(raw);
    assert_eq!(projected.runtime.call_contract, native.call_contract);
    assert_eq!(projected.runtime.return_abi, Some(AbiType::Bool32));
    assert_eq!(projected.runtime.success_rule, SuccessRule::ReturnNonZero);
    assert!(projected.runtime.capture_last_error);
    assert_eq!(
        projected.inputs,
        [
            InputExpression::Surface {
                parameter_index: 0,
                conversion: Conversion::Handle
            },
            InputExpression::Surface {
                parameter_index: 1,
                conversion: Conversion::U8
            },
        ]
    );
    assert_eq!(
        projected.return_shape,
        ReturnShape::Object {
            status: false,
            return_value: Some((SurfaceType::Boolean, Conversion::Boolean)),
            outputs: Vec::new(),
            last_error: true,
        }
    );
}

fn slot(
    abi: AbiType,
    direction: Direction,
    nullable: bool,
    cleanup: Cleanup,
    resource_cleanup: Cleanup,
) -> RuntimeParameter {
    RuntimeParameter {
        abi,
        direction,
        nullable,
        cleanup,
        resource_cleanup,
        consumes_resource: false,
        aggregate: None,
    }
}

fn surface(
    name: &str,
    typ: SurfaceType,
    nullable: bool,
    alignment: Option<usize>,
) -> SurfaceParameter {
    SurfaceParameter {
        name: name.into(),
        typ,
        nullable,
        minimum_bytes: None,
        alignment,
    }
}

fn assert_status_plan(plan: &RuntimePlan, entry: &str) {
    assert_eq!(plan.dll, "ADVAPI32.dll");
    assert_eq!(plan.entry_point, entry);
    assert_eq!(plan.calling_convention, CallingConvention::System);
    assert_eq!(plan.return_abi, Some(AbiType::U32));
    assert_eq!(plan.return_aggregate, None);
    assert_eq!(plan.return_cleanup, Cleanup::None);
    assert_eq!(plan.success_rule, SuccessRule::ReturnZero);
    assert!(!plan.capture_last_error);
}
