// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use crate::contract_registry::{self, ContractSelector};

fn configured_index() -> Option<reader::Index> {
    let Ok(winmd) = std::env::var("DYNWINRT_WIN32_WINMD") else {
        eprintln!("Skipping migrated registry metadata tests: DYNWINRT_WIN32_WINMD is not set");
        return None;
    };
    assert_eq!(
        format!("{:X}", Sha256::digest(std::fs::read(&winmd).unwrap())),
        crate::codegen::com::capability::OFFICIAL_METADATA_SHA256,
        "migrated evidence regression requires the pinned metadata"
    );
    Some(crate::meta::load_index(&winmd).unwrap())
}

fn declared_method(index: &reader::Index, selector: &ContractSelector) -> RawComMethod {
    parse_com_interface_from_index(
        index,
        &selector.interface.namespace,
        &selector.interface.name,
    )
    .unwrap()
    .raw_methods
    .unwrap()
    .into_iter()
    .find(|raw| {
        raw.declaring_iid == selector.declaring_iid && raw.vtable_index == selector.absolute_slot
    })
    .unwrap()
}

fn source_fingerprint(raw: &RawComMethod, selector: &ContractSelector) -> String {
    if selector.source_shape.is_some() {
        format!("{:X}", Sha256::digest(raw_method_shape(raw).as_bytes()))
    } else {
        raw_method_fingerprint(raw)
    }
}

fn assert_selector(raw: &RawComMethod, selector: &ContractSelector) {
    assert_eq!(raw.declaring_namespace, selector.interface.namespace);
    assert_eq!(raw.declaring_interface, selector.interface.name);
    assert_eq!(raw.declaring_iid, selector.interface.iid);
    assert_eq!(raw.declaring_iid, selector.declaring_iid);
    assert_eq!(raw.metadata_name, selector.method);
    assert_eq!(raw.vtable_index, selector.absolute_slot);
    assert!(raw_parameters_match_selector(raw, selector), "{selector:?}");
    if let Some(shape) = &selector.source_shape {
        assert_eq!(shape.value, raw_method_shape(raw), "{selector:?}");
    }
    assert_eq!(
        source_fingerprint(raw, selector),
        selector.source_fingerprint,
        "{selector:?}"
    );
}

#[test]
fn all_migrated_records_match_pinned_metadata_and_keep_exact_provenance() {
    let Some(index) = configured_index() else {
        return;
    };
    let mut matched = 0;
    for entry in contract_registry::safe_array_contracts().unwrap() {
        let raw = declared_method(&index, &entry.selector);
        assert_selector(&raw, &entry.selector);
        validate_attached_safe_array_evidence(&raw).unwrap();
        assert_eq!(
            raw.params[entry.contract.parameter_index]
                .safe_array_evidence
                .as_ref()
                .unwrap()
                .entry_id(),
            entry.entry_id
        );
        matched += 1;
    }
    for entry in contract_registry::borrowed_handle_contracts().unwrap() {
        let raw = declared_method(&index, &entry.selector);
        assert_selector(&raw, &entry.selector);
        validate_borrowed_hwnd_output_evidence(&raw).unwrap();
        validate_migrated_source_shape(&raw).unwrap();
        assert!(is_registered_borrowed_hwnd_output(
            &raw,
            entry.contract.parameter_index
        ));
        matched += 1;
    }
    for entry in contract_registry::null_input_contracts().unwrap() {
        let raw = declared_method(&index, &entry.selector);
        assert_selector(&raw, &entry.selector);
        assert!(raw.exact_null_input_contracts.iter().any(|contract| {
            matches!(&contract.evidence, RawEvidence::ExactRegistry { entry_id, .. } if entry_id == &entry.entry_id)
        }));
        matched += 1;
    }
    for entry in contract_registry::parameter_direction_contracts().unwrap() {
        let mut raw = declared_method(&index, &entry.selector);
        assert_eq!(
            raw.params[entry.contract.parameter_index].direction,
            RawParamDirection::Out
        );
        assert!(raw.exact_parameter_direction_contracts.iter().any(|contract| {
            matches!(&contract.evidence, RawEvidence::ExactRegistry { entry_id, .. } if entry_id == &entry.entry_id)
        }));
        raw.params[entry.contract.parameter_index].direction = RawParamDirection::InOut;
        raw.exact_parameter_direction_contracts.clear();
        assert_selector(&raw, &entry.selector);
        matched += 1;
    }
    for entry in contract_registry::enumerator_contracts().unwrap() {
        let raw = declared_method(&index, &entry.selector);
        assert_selector(&raw, &entry.selector);
        validate_attached_enumerator_evidence(&raw).unwrap();
        validate_migrated_source_shape(&raw).unwrap();
        let evidence = &raw.enumerator_next.as_ref().unwrap().evidence;
        match entry.contract.evidence_source {
            contract_registry::EnumeratorEvidenceSource::ComStandard => assert_eq!(
                evidence,
                &RawEvidence::ComStandard(
                    contract_registry::ComStandardRule::GenericEnumeratorNext
                ),
            ),
            contract_registry::EnumeratorEvidenceSource::ExactRegistry => assert!(
                matches!(evidence, RawEvidence::ExactRegistry { entry_id, .. } if entry_id == &entry.entry_id),
            ),
        }
        matched += 1;
    }
    assert_eq!(matched, 333);
}

#[test]
fn migrated_selectors_enforce_every_parameter_field_and_fingerprint() {
    let Some(index) = configured_index() else {
        return;
    };
    let samples = [
        (
            &contract_registry::safe_array_contracts().unwrap()[0].selector,
            None,
        ),
        (
            &contract_registry::borrowed_handle_contracts().unwrap()[0].selector,
            None,
        ),
        (
            &contract_registry::null_input_contracts().unwrap()[0].selector,
            None,
        ),
        (
            &contract_registry::parameter_direction_contracts().unwrap()[0].selector,
            Some(3),
        ),
        (
            &contract_registry::enumerator_contracts().unwrap()[0].selector,
            None,
        ),
    ];
    for (selector, out_override) in samples {
        let mut raw = declared_method(&index, selector);
        if let Some(index) = out_override {
            raw.params[index].direction = RawParamDirection::InOut;
            raw.exact_parameter_direction_contracts.clear();
        }
        assert_selector(&raw, selector);
        for index in 0..raw.params.len() {
            for field in 0..7 {
                let mut drifted = raw.clone();
                let parameter = &mut drifted.params[index];
                match field {
                    0 => parameter.name.push_str("Drift"),
                    1 => parameter.typ.pointer_depth += 1,
                    2 => {
                        parameter.direction = if parameter.direction == RawParamDirection::In {
                            RawParamDirection::Out
                        } else {
                            RawParamDirection::In
                        }
                    }
                    3 => parameter.optional = !parameter.optional,
                    4 => parameter.const_attribute = !parameter.const_attribute,
                    5 => {
                        parameter.typ.constness = if parameter.typ.constness == RawConstness::Const
                        {
                            RawConstness::Mutable
                        } else {
                            RawConstness::Const
                        }
                    }
                    6 => {
                        parameter.typ.native_type =
                            if parameter.typ.native_type == RawNativeType::Void {
                                RawNativeType::U32
                            } else {
                                RawNativeType::Void
                            }
                    }
                    _ => unreachable!(),
                }
                assert!(
                    !raw_parameters_match_selector(&drifted, selector),
                    "{selector:?} parameter {index}, field {field}"
                );
                assert_ne!(
                    source_fingerprint(&drifted, selector),
                    selector.source_fingerprint
                );
            }
        }
        let mut drifted = raw.clone();
        drifted.return_type.pointer_depth += 1;
        assert_ne!(
            source_fingerprint(&drifted, selector),
            selector.source_fingerprint
        );
        let mut drifted = selector.clone();
        drifted.parameters.pop();
        assert!(!raw_parameters_match_selector(&raw, &drifted));
    }
}

#[test]
fn migrated_null_and_direction_overrides_fail_closed_on_source_drift() {
    let Some(index) = configured_index() else {
        return;
    };
    for entry in contract_registry::null_input_contracts().unwrap() {
        let source = declared_method(&index, &entry.selector);
        for parameter_index in 0..source.params.len() {
            let mut drifted = source.clone();
            drifted.exact_null_input_contracts.clear();
            drifted.params[parameter_index].name.push_str("Drift");
            assert!(!apply_exact_null_input_contracts(&mut drifted));
            assert!(drifted.exact_null_input_contracts.is_empty());
        }
    }
    for entry in contract_registry::parameter_direction_contracts().unwrap() {
        let interface = parse_com_interface_from_index(
            &index,
            &entry.selector.interface.namespace,
            &entry.selector.interface.name,
        )
        .unwrap();
        let compatibility = interface
            .interface
            .methods
            .iter()
            .find(|method| method.name == entry.selector.method)
            .unwrap();
        let mut source = declared_method(&index, &entry.selector);
        source.params[entry.contract.parameter_index].direction = RawParamDirection::InOut;
        source.exact_parameter_direction_contracts.clear();
        for parameter_index in 0..source.params.len() {
            let mut drifted = source.clone();
            drifted.params[parameter_index].name.push_str("Drift");
            assert!(!apply_exact_out_parameter_contracts(
                &mut compatibility.clone(),
                &mut drifted
            ));
            assert_eq!(
                drifted.params[entry.contract.parameter_index].direction,
                RawParamDirection::InOut
            );
            assert!(drifted.exact_parameter_direction_contracts.is_empty());
        }
    }
}

#[test]
fn newly_pinned_borrowed_and_enumerator_shapes_reject_additional_drift() {
    let Some(index) = configured_index() else {
        return;
    };
    for selector in [
        &contract_registry::borrowed_handle_contracts().unwrap()[0].selector,
        &contract_registry::enumerator_contracts().unwrap()[0].selector,
    ] {
        let mut raw = declared_method(&index, selector);
        validate_migrated_source_shape(&raw).unwrap();
        raw.return_type.constness = RawConstness::Const;
        assert!(validate_migrated_source_shape(&raw).is_err());
    }
}

#[test]
fn pinned_source_drift_cannot_escape_through_safe_unsafe_or_census_paths() {
    let Some(index) = configured_index() else {
        return;
    };
    let winmd = std::env::var("DYNWINRT_WIN32_WINMD").unwrap();
    let metadata =
        crate::codegen::com::capability::metadata_set_identity_for_paths(&winmd).unwrap();
    for (namespace, name, slot) in [
        ("Windows.Win32.System.Ole", "IOleWindow", 3),
        ("Windows.Win32.System.Com", "IEnumUnknown", 3),
        ("Windows.Win32.System.Search", "IEnumSubscription", 3),
    ] {
        let mut interface = parse_com_interface_from_index(&index, namespace, name).unwrap();
        crate::codegen::com::generate_com_interface_files(&interface, &winmd).unwrap();
        crate::codegen::com::generate_unsafe_interface_files_with_metadata(&interface, &metadata)
            .unwrap();
        interface
            .raw_methods
            .as_mut()
            .unwrap()
            .iter_mut()
            .find(|method| method.vtable_index == slot)
            .unwrap()
            .return_type
            .constness = RawConstness::Const;
        let safe =
            crate::codegen::com::generate_com_interface_files(&interface, &winmd).unwrap_err();
        assert!(safe.contains("source shape"), "{safe}");
        let unsafe_error = crate::codegen::com::generate_unsafe_interface_files_with_metadata(
            &interface, &metadata,
        )
        .unwrap_err();
        assert!(unsafe_error.contains("source shape"), "{unsafe_error}");
        let census = collect_exact_registry_entries(&interface).unwrap_err();
        assert!(census.contains("source shape"), "{census}");
    }
}

#[test]
fn nullable_safearray_pointees_preserve_required_cells_and_declaring_evidence() {
    let Some(index) = configured_index() else {
        return;
    };
    let nullable = contract_registry::safe_array_contracts()
        .unwrap()
        .iter()
        .filter(|entry| entry.contract.nullability.allows_null_output())
        .collect::<Vec<_>>();
    assert_eq!(nullable.len(), 4);
    for entry in nullable {
        let mut raw = declared_method(&index, &entry.selector);
        let parameter = &raw.params[entry.contract.parameter_index];
        assert!(!parameter.optional);
        assert_eq!(parameter.typ.pointer_depth, 2);
        assert_eq!(parameter.direction, RawParamDirection::Out);
        assert!(
            crate::com_safe_array_registry::safe_array_output_allows_null(
                parameter.safe_array_evidence.as_ref().unwrap()
            )
        );
        raw.params[entry.contract.parameter_index].optional = true;
        assert!(validate_attached_safe_array_evidence(&raw).is_err());
    }
    let inherited =
        parse_com_interface_from_index(&index, "Windows.Win32.UI.Accessibility", "ITextProvider2")
            .unwrap();
    let selection = inherited
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .find(|raw| raw.metadata_name == "GetSelection")
        .unwrap();
    assert_eq!(selection.declaring_interface, "ITextProvider");
    let evidence = selection.params[0].safe_array_evidence.as_ref().unwrap();
    assert!(crate::com_safe_array_registry::safe_array_output_allows_null(evidence));
    assert!(
        collect_evidence_dependencies(&inherited)
            .exact_entry_ids
            .contains(&evidence.entry_id())
    );
}
