// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;

#[test]
fn win32_contracts_are_strict_and_integrity_checked() {
    let registry = Registry::builtin().unwrap();
    assert!(registry.functions.len() > 1000);
    assert!(registry.types.len() > 100);
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
    let Ok(path) = std::env::var("DYNWINRT_WIN32_WINMD") else {
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
    let Ok(path) = std::env::var("DYNWINRT_WIN32_WINMD") else {
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
fn win32_free_library_matches_frozen_consuming_semantics() {
    let Ok(path) = std::env::var("DYNWINRT_WIN32_WINMD") else {
        return;
    };
    let mut raw =
        crate::win32_metadata::parse_apis(&path, "Windows.Win32.Foundation", "Apis").unwrap();
    raw.functions
        .retain(|function| function.name == "FreeLibrary");
    assert_eq!(raw.functions.len(), 1);
    let projected = crate::codegen::win32::project_apis(&raw);
    assert!(projected.omitted.is_empty(), "{:?}", projected.omitted);
    let function = &projected.projected.functions[0];
    assert!(function.runtime.parameters[0].consumes_resource);
    assert_eq!(
        function.runtime.parameters[0].resource_cleanup,
        Cleanup::FreeLibrary
    );
    let baseline: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests\\fixtures\\win32-pr102.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let expected = baseline["containers"]["Windows.Win32.Foundation.Apis"]["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row[0] == "FreeLibrary")
        .unwrap();
    let semantic = format!(
        "{:?}\n{:?}\n{:?}\n{:?}",
        function.parameters, function.inputs, function.runtime, function.return_shape,
    );
    assert_eq!(sha256(semantic.as_bytes()), expected[6].as_str().unwrap());
}

#[test]
fn win32_registry_hkey_policies_preserve_frozen_default_plans() {
    use crate::codegen::win32::ir::ProjectedCallPolicy;
    let Ok(path) = std::env::var("DYNWINRT_WIN32_WINMD") else {
        return;
    };
    let mut raw =
        crate::win32_metadata::parse_apis(&path, "Windows.Win32.System.Registry", "Apis").unwrap();
    raw.functions.retain(|function| {
        [
            "RegOpenKeyExA",
            "RegOpenKeyExW",
            "RegQueryValueExA",
            "RegQueryValueExW",
        ]
        .contains(&function.name.as_str())
    });
    let projection = crate::codegen::win32::project_apis(&raw);
    assert!(projection.omitted.is_empty(), "{:?}", projection.omitted);
    let fixture: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests\\fixtures\\win32-pr102.json"),
        )
        .unwrap(),
    )
    .unwrap();
    for function in &projection.projected.functions {
        let expected = fixture["containers"]["Windows.Win32.System.Registry.Apis"]["functions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row[0] == function.metadata_name)
            .unwrap();
        let semantic = format!(
            "{:?}\n{:?}\n{:?}\n{:?}",
            function.parameters, function.inputs, function.runtime, function.return_shape
        );
        assert_eq!(
            sha256(semantic.as_bytes()),
            expected[6].as_str().unwrap(),
            "{}",
            function.metadata_name
        );
        assert_eq!(
            function.call_policies.len(),
            1,
            "{}",
            function.metadata_name
        );
        match &function.call_policies[0] {
            ProjectedCallPolicy::HkeyPerformanceDataCount {
                handle_parameter,
                output_index,
                undefined_status,
            } => {
                assert_eq!(*handle_parameter, 0);
                assert_eq!(*output_index, 1);
                assert_eq!(*undefined_status, 234);
            }
            ProjectedCallPolicy::BorrowedPredefinedHkeyOutput {
                runtime,
                output_index,
                ..
            } => {
                assert_eq!(*output_index, 0);
                assert_eq!(runtime.parameters[4].cleanup, Cleanup::None);
                assert_eq!(function.runtime.parameters[4].cleanup, Cleanup::RegCloseKey);
            }
        }
    }
}
