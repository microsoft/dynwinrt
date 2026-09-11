// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
#[path = "contract_validation_tests.rs"]
mod validation;

use crate::codegen::win32::{
    ir::{
        AbiType, CallingConvention, Conversion, Direction, EnumUnderlying, InputExpression,
        ProjectedCallPolicy, ProjectedOutput, ReturnShape, RuntimeParameter, RuntimePlan,
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
        r#"{"kind":"hkey-performance-data-count","handleParameter":0,"countParameter":5,"undefinedOn":"success"}"#,
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
    assert!(projected.call_policies.is_empty());
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
                    conversion: Conversion::U32
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
                    typ: SurfaceType::Resource,
                    conversion: Conversion::Resource
                }],
                last_error: false,
            }
        );
        let mut borrowed = open.runtime.clone();
        borrowed.parameters[4].cleanup = Cleanup::None;
        assert_eq!(
            open.call_policies,
            vec![ProjectedCallPolicy::BorrowedPredefinedHkeyOutput {
                handle_parameter: 0,
                string_parameter: 1,
                encoding,
                output_index: 0,
                runtime: Box::new(borrowed),
            }]
        );

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
                        conversion: Conversion::Number
                    },
                    ProjectedOutput {
                        name: "dataSize".into(),
                        output_index: 1,
                        typ: SurfaceType::Number,
                        conversion: Conversion::Number
                    },
                ],
            }
        );
        assert_eq!(
            query.call_policies,
            vec![ProjectedCallPolicy::HkeyPerformanceDataCount {
                handle_parameter: 0,
                output_index: 1,
                undefined_status: 234,
            }]
        );
        for function in [&open, &query] {
            assert_eq!(function.subsystem, None);
            assert_eq!(function.unicode_alias.is_some(), suffix == "W");
        }
    }
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
