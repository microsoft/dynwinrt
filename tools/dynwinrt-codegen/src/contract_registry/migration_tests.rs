// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use serde_json::{Value, json};

use super::*;

const MIGRATED_FILES: [(ContractKind, &str); 5] = [
    (ContractKind::Safearray, SAFEARRAYS_JSON),
    (ContractKind::NullInput, NULL_INPUTS_JSON),
    (ContractKind::ParameterDirection, PARAMETER_DIRECTIONS_JSON),
    (ContractKind::BorrowedHandle, BORROWED_HANDLES_JSON),
    (ContractKind::EnumeratorNext, ENUMERATORS_JSON),
];

fn validate(kind: ContractKind, document: Value) -> Result<(), String> {
    let json = serde_json::to_string(&document).unwrap();
    match kind {
        ContractKind::Safearray => {
            families::load_family::<SafeArraySemantics>(&json, "test").map(|_| ())
        }
        ContractKind::NullInput => {
            families::load_family::<NullInputSemantics>(&json, "test").map(|_| ())
        }
        ContractKind::ParameterDirection => {
            families::load_family::<ParameterDirectionSemantics>(&json, "test").map(|_| ())
        }
        ContractKind::BorrowedHandle => {
            families::load_family::<BorrowedHandleSemantics>(&json, "test").map(|_| ())
        }
        ContractKind::EnumeratorNext => {
            families::load_family::<EnumeratorSemantics>(&json, "test").map(|_| ())
        }
        _ => panic!("test only covers the five migrated families"),
    }
}

fn fixture(kind: ContractKind, matches: impl Fn(&Value) -> bool) -> Value {
    let source = MIGRATED_FILES
        .iter()
        .find(|(candidate, _)| *candidate == kind)
        .unwrap()
        .1;
    let document: Value = serde_json::from_str(source).unwrap();
    let entry = document["contracts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| matches(entry))
        .unwrap();
    json!({"schemaVersion":2, "contracts":[entry]})
}

fn rejects(kind: ContractKind, original: &Value, path: &str, replacement: Value) {
    let mut changed = original.clone();
    *changed
        .pointer_mut(path)
        .unwrap_or_else(|| panic!("missing test pointer {path}")) = replacement;
    assert!(
        validate(kind, changed).is_err(),
        "{} accepted drift at {path}",
        kind.key()
    );
}

#[test]
fn five_families_preserve_record_counts_and_evidence_tiers() {
    let registry = load_registry().unwrap();
    assert_eq!(registry.safe_arrays.len(), 209);
    assert_eq!(registry.null_inputs.len(), 2);
    assert_eq!(registry.parameter_directions.len(), 3);
    assert_eq!(registry.borrowed_handles.len(), 22);
    assert_eq!(registry.enumerators.len(), 97);
    assert_eq!(registry.exact_entry_ids().len(), 465);
    assert_eq!(statically_declared_exact_entry_ids().unwrap().len(), 532);
    let generic = registry
        .enumerators
        .iter()
        .filter(|entry| entry.contract.evidence_source == EnumeratorEvidenceSource::ComStandard)
        .collect::<Vec<_>>();
    assert_eq!(generic.len(), 24);
    assert!(
        generic
            .iter()
            .all(|entry| !registry.exact_entry_ids().contains(&entry.entry_id))
    );
    assert!(generic.iter().all(|entry| entry.microsoft_citation()
        == "https://learn.microsoft.com/windows/win32/api/unknwn/nf-unknwn-ienumunknown-next"));
    assert_eq!(
        registry
            .safe_arrays
            .iter()
            .filter(|entry| entry.contract.ownership == SafeArrayOwnership::BorrowedInput)
            .count(),
        69
    );
    assert_eq!(
        registry
            .safe_arrays
            .iter()
            .filter(|entry| entry.contract.ownership == SafeArrayOwnership::OwnedOutput)
            .count(),
        140
    );
    for (vartype, count) in [
        (SafeArrayVartype::I4, 25),
        (SafeArrayVartype::Ui1, 25),
        (SafeArrayVartype::Ui4, 3),
        (SafeArrayVartype::R8, 3),
        (SafeArrayVartype::Bstr, 40),
        (SafeArrayVartype::Unknown, 28),
        (SafeArrayVartype::Variant, 85),
    ] {
        assert_eq!(
            registry
                .safe_arrays
                .iter()
                .filter(|entry| entry.contract.element.vartype == vartype)
                .count(),
            count
        );
    }
}

#[test]
fn schema_discriminates_the_compiled_kinds_families_and_payloads() {
    let schema: Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    let contract = &schema["$defs"]["contract"];
    let variants = contract["oneOf"].as_array().unwrap();
    assert_eq!(variants.len(), COMPILED_FILE_IDENTITIES.len());
    assert_eq!(contract["additionalProperties"], false);
    for (_, kind, family) in COMPILED_FILE_IDENTITIES {
        let variant = variants
            .iter()
            .find(|variant| variant["properties"]["kind"]["const"] == kind.key())
            .unwrap();
        assert_eq!(variant["properties"]["familyId"]["const"], family.id());
        let definition = variant["properties"]["contract"]["$ref"]
            .as_str()
            .unwrap()
            .strip_prefix("#/")
            .unwrap();
        let definition = schema.pointer(&format!("/{definition}")).unwrap();
        assert_eq!(definition["additionalProperties"], false);
    }
}

#[test]
fn every_family_rejects_unknown_missing_and_misclassified_fields() {
    for (kind, _) in MIGRATED_FILES {
        let original = fixture(kind, |_| true);
        validate(kind, original.clone()).unwrap();
        for path in [
            "",
            "/contracts/0",
            "/contracts/0/selector",
            "/contracts/0/selector/interface",
            "/contracts/0/selector/parameters/0",
            "/contracts/0/contract",
            "/contracts/0/evidence/0",
            "/contracts/0/validatedMetadata/0",
        ] {
            let mut changed = original.clone();
            changed
                .pointer_mut(path)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("rendererCode".into(), "arbitrary".into());
            assert!(
                validate(kind, changed)
                    .unwrap_err()
                    .contains("unknown field"),
                "{} {path}",
                kind.key()
            );
        }
        for required in [
            "entryId",
            "familyId",
            "kind",
            "reason",
            "selector",
            "contract",
            "evidence",
            "validatedMetadata",
        ] {
            let mut changed = original.clone();
            changed["contracts"][0]
                .as_object_mut()
                .unwrap()
                .remove(required);
            assert!(
                validate(kind, changed).is_err(),
                "{} missing {required}",
                kind.key()
            );
        }
        rejects(kind, &original, "/schemaVersion", json!(3));
        rejects(kind, &original, "/contracts/0/kind", json!("renderer-code"));
        rejects(kind, &original, "/contracts/0/kind", json!("ownership"));
        rejects(
            kind,
            &original,
            "/contracts/0/familyId",
            json!("com.ownership.v1"),
        );
        rejects(
            kind,
            &original,
            "/contracts/0/selector/parameters/0/direction",
            json!("inferred"),
        );
        rejects(
            kind,
            &original,
            "/contracts/0/selector/parameters/0/constness",
            json!("guessed"),
        );
        rejects(
            kind,
            &original,
            "/contracts/0/selector/parameters/0/index",
            json!(99),
        );
        rejects(
            kind,
            &original,
            "/contracts/0/selector/declaringIid",
            json!("not-a-guid"),
        );
        rejects(
            kind,
            &original,
            "/contracts/0/selector/parameterCount",
            json!(999),
        );
        rejects(kind, &original, "/contracts/0/evidence", json!([]));
        rejects(
            kind,
            &original,
            "/contracts/0/evidence/0/url",
            json!("https://example.com/not-authoritative"),
        );
        rejects(kind, &original, "/contracts/0/validatedMetadata", json!([]));
    }
}

#[test]
fn every_family_rejects_duplicate_ids_and_parameter_selectors() {
    for (kind, _) in MIGRATED_FILES {
        let mut document = fixture(kind, |_| true);
        let second = document["contracts"][0].clone();
        document["contracts"].as_array_mut().unwrap().push(second);
        assert!(
            validate(kind, document.clone())
                .unwrap_err()
                .contains("Duplicate")
        );
        if kind == ContractKind::EnumeratorNext {
            continue;
        }
        let entry = &mut document["contracts"][1];
        let index = entry["contract"]["parameterIndex"].as_u64().unwrap() as usize;
        entry["selector"]["parameters"][index]["name"] = json!("differentName");
        let selector: ContractSelector = serde_json::from_value(entry["selector"].clone()).unwrap();
        let family: ExactFamilyId = serde_json::from_value(entry["familyId"].clone()).unwrap();
        entry["entryId"] = json!(exact_parameter_entry_id(
            family,
            &selector.interface.namespace,
            &selector.interface.name,
            &selector.interface.iid,
            &selector.method,
            selector.absolute_slot,
            index,
            "differentName",
        ));
        assert!(
            validate(kind, document)
                .unwrap_err()
                .contains("Conflicting contract selectors")
        );
    }
}

#[test]
fn file_hashes_and_compiled_manifest_allowlist_are_mandatory() {
    for index in 0..COMPILED_FILES.len() {
        let mut sources = COMPILED_FILES;
        let changed = format!("{} ", sources[index]);
        sources[index] = &changed;
        assert!(
            validate_manifest(MANIFEST_JSON, &sources)
                .unwrap_err()
                .contains("hash mismatch")
        );
        let mut manifest: Value = serde_json::from_str(MANIFEST_JSON).unwrap();
        manifest["files"][index]["path"] = json!("runtime-override.json");
        assert!(
            validate_manifest(&manifest.to_string(), &COMPILED_FILES)
                .unwrap_err()
                .contains("compiled data files")
        );
        let mut manifest: Value = serde_json::from_str(MANIFEST_JSON).unwrap();
        manifest["files"][index]["sha256"] = json!("bad");
        assert!(
            validate_manifest(&manifest.to_string(), &COMPILED_FILES)
                .unwrap_err()
                .contains("SHA-256")
        );
    }
    let mut manifest: Value = serde_json::from_str(MANIFEST_JSON).unwrap();
    manifest["files"].as_array_mut().unwrap().pop();
    assert!(validate_manifest(&manifest.to_string(), &COMPILED_FILES).is_err());
    let mut manifest: Value = serde_json::from_str(MANIFEST_JSON).unwrap();
    manifest["runtimeOverrides"] = json!(true);
    assert!(
        validate_manifest(&manifest.to_string(), &COMPILED_FILES)
            .unwrap_err()
            .contains("unknown field")
    );
}

#[test]
fn source_fingerprints_and_metadata_provenance_cannot_be_inferred() {
    for (kind, _) in MIGRATED_FILES {
        let original = fixture(kind, |_| true);
        rejects(
            kind,
            &original,
            "/contracts/0/selector/sourceFingerprint",
            json!("bad"),
        );
        rejects(
            kind,
            &original,
            "/contracts/0/validatedMetadata/0/sha256",
            json!("bad"),
        );
        rejects(
            kind,
            &original,
            "/contracts/0/validatedMetadata/0/sha256",
            json!("0".repeat(64)),
        );
        rejects(
            kind,
            &original,
            "/contracts/0/validatedMetadata/0/version",
            json!("unreviewed"),
        );
        let mut changed = original.clone();
        changed["contracts"][0]["selector"]["sourceShape"] = Value::Null;
        assert!(validate(kind, changed).is_err());
        if original["contracts"][0]["selector"]
            .get("sourceShape")
            .is_some()
        {
            rejects(
                kind,
                &original,
                "/contracts/0/selector/sourceFingerprint",
                json!("0".repeat(64)),
            );
            rejects(
                kind,
                &original,
                "/contracts/0/selector/sourceShape/format",
                json!("current-input"),
            );
            rejects(
                kind,
                &original,
                "/contracts/0/selector/sourceShape/value",
                json!("drift"),
            );
            let mut changed = original;
            changed["contracts"][0]["selector"]
                .as_object_mut()
                .unwrap()
                .remove("sourceShape");
            assert!(validate(kind, changed).is_err());
        }
    }
}

#[test]
fn safearray_nullability_never_weakens_the_receiving_cell() {
    let nullable = fixture(ContractKind::Safearray, |entry| {
        entry["contract"]["nullability"]["pointee"]["kind"] == "nullable-on-success"
    });
    let index = nullable["contracts"][0]["contract"]["parameterIndex"]
        .as_u64()
        .unwrap();
    for (path, value) in [
        (
            format!("/contracts/0/selector/parameters/{index}/optional"),
            json!(true),
        ),
        (
            format!("/contracts/0/selector/parameters/{index}/pointerDepth"),
            json!(1),
        ),
        (
            "/contracts/0/contract/nullability/kind".into(),
            json!("optional-output-cell"),
        ),
        (
            "/contracts/0/contract/nullability/pointee/evidence".into(),
            json!([]),
        ),
        (
            "/contracts/0/contract/nullability/pointee/reason".into(),
            json!(""),
        ),
        ("/contracts/0/contract/cleanup".into(), json!("none")),
        (
            "/contracts/0/contract/cleanup".into(),
            json!("ArbitraryFree"),
        ),
    ] {
        rejects(ContractKind::Safearray, &nullable, &path, value);
    }
    let input = fixture(ContractKind::Safearray, |entry| {
        entry["contract"]["ownership"] == "borrowed-input"
    });
    rejects(
        ContractKind::Safearray,
        &input,
        "/contracts/0/contract/nullability",
        nullable["contracts"][0]["contract"]["nullability"].clone(),
    );
    let scalar = fixture(ContractKind::Safearray, |entry| {
        entry["contract"]["element"]["vartype"] != "VT_UNKNOWN"
    });
    rejects(
        ContractKind::Safearray,
        &scalar,
        "/contracts/0/contract/element/interfaceIid",
        json!("00000000-0000-0000-c000-000000000046"),
    );
    rejects(
        ContractKind::Safearray,
        &scalar,
        "/contracts/0/contract/element/vartype",
        json!("VT_DISPATCH"),
    );
    let interface = fixture(ContractKind::Safearray, |entry| {
        entry["contract"]["element"]["vartype"] == "VT_UNKNOWN"
    });
    rejects(
        ContractKind::Safearray,
        &interface,
        "/contracts/0/contract/element/interfaceIid",
        Value::Null,
    );
    let mut missing_iid = scalar;
    missing_iid["contracts"][0]["contract"]["element"]
        .as_object_mut()
        .unwrap()
        .remove("interfaceIid");
    assert!(validate(ContractKind::Safearray, missing_iid).is_err());

    let entries = crate::com_safe_array_registry::all_safe_array_evidence();
    let nullable = entries
        .iter()
        .filter(|evidence| crate::com_safe_array_registry::safe_array_output_allows_null(evidence))
        .collect::<Vec<_>>();
    assert_eq!(nullable.len(), 4);
    for evidence in nullable {
        let mut changed = evidence.clone();
        changed.citation = "https://learn.microsoft.com/drift";
        assert!(!crate::com_safe_array_registry::safe_array_output_allows_null(&changed));
        let mut changed = evidence.clone();
        changed.raw_method_shape = "drift";
        assert!(!crate::com_safe_array_registry::safe_array_output_allows_null(&changed));
    }
}

#[test]
fn safearray_unit_nullability_variants_reject_unknown_fields() {
    let mut input = fixture(ContractKind::Safearray, |entry| {
        entry["contract"]["ownership"] == "borrowed-input"
    });
    input["contracts"][0]["contract"]["nullability"]["pointee"] =
        json!({"kind":"nullable-on-success"});
    assert!(
        validate(ContractKind::Safearray, input)
            .unwrap_err()
            .contains("unknown field")
    );
    let mut output = fixture(ContractKind::Safearray, |entry| {
        entry["contract"]["nullability"]["pointee"]["kind"] == "required"
    });
    output["contracts"][0]["contract"]["nullability"]["pointee"]["nullable"] = json!(true);
    assert!(
        validate(ContractKind::Safearray, output)
            .unwrap_err()
            .contains("unknown field")
    );
}

#[test]
fn null_direction_handle_and_enumerator_contracts_remain_closed() {
    for (kind, path, value) in [
        (
            ContractKind::NullInput,
            "/contracts/0/contract/value",
            json!("zeroed-buffer"),
        ),
        (
            ContractKind::NullInput,
            "/contracts/0/selector/parameters/1/nativeType",
            json!("u8"),
        ),
        (
            ContractKind::NullInput,
            "/contracts/0/selector/parameters/1/direction",
            json!("out"),
        ),
        (
            ContractKind::ParameterDirection,
            "/contracts/0/contract/direction",
            json!("inout"),
        ),
        (
            ContractKind::ParameterDirection,
            "/contracts/0/selector/parameters/3/direction",
            json!("out"),
        ),
        (
            ContractKind::BorrowedHandle,
            "/contracts/0/contract/handle",
            json!("Windows.Win32.Foundation.HBITMAP"),
        ),
        (
            ContractKind::BorrowedHandle,
            "/contracts/0/contract/cleanup",
            json!("DestroyWindow"),
        ),
        (
            ContractKind::BorrowedHandle,
            "/contracts/0/contract/receivingCell",
            json!("optional"),
        ),
        (
            ContractKind::BorrowedHandle,
            "/contracts/0/selector/parameters/0/optional",
            json!(true),
        ),
        (
            ContractKind::EnumeratorNext,
            "/contracts/0/contract/capacityParameterIndex",
            json!(1),
        ),
        (
            ContractKind::EnumeratorNext,
            "/contracts/0/contract/fetchedOptionalForSingle",
            json!(false),
        ),
        (
            ContractKind::EnumeratorNext,
            "/contracts/0/contract/hresult",
            json!("discard-s-false"),
        ),
        (
            ContractKind::EnumeratorNext,
            "/contracts/0/contract/evidenceSource",
            json!("metadata"),
        ),
        (
            ContractKind::EnumeratorNext,
            "/contracts/0/selector/parameters/2/direction",
            json!("in"),
        ),
        (
            ContractKind::EnumeratorNext,
            "/contracts/0/selector/parameters/1/pointerDepth",
            json!(2),
        ),
    ] {
        rejects(kind, &fixture(kind, |_| true), path, value);
    }
}
