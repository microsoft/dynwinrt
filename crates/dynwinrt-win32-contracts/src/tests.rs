// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;

fn registry_shape() -> SignatureShape {
    SignatureShape {
        pointer_width: 64,
        parameters: vec![
            ParameterShape {
                typ: NativeType::Handle,
                direction: Direction::In,
                cleanup: Cleanup::None,
                resource_cleanup: Cleanup::RegCloseKey,
                consumes_resource: false,
            },
            ParameterShape {
                typ: NativeType::Pointer,
                direction: Direction::In,
                cleanup: Cleanup::None,
                resource_cleanup: Cleanup::None,
                consumes_resource: false,
            },
            ParameterShape {
                typ: NativeType::Handle,
                direction: Direction::Out,
                cleanup: Cleanup::RegCloseKey,
                resource_cleanup: Cleanup::None,
                consumes_resource: false,
            },
        ],
        return_type: Some(NativeType::I32),
        return_cleanup: Cleanup::None,
        success_rule: SuccessRule::ReturnZero,
    }
}

#[test]
fn legacy_descriptors_upgrade_to_complete_explicit_result_contracts() {
    let legacy = CallContract::decode(r#"{"outputs":[{"parameter":2,"when":{"inputs":[{"kind":"null-or-empty","parameter":1,"elementWidth":2}]},"action":{"kind":"alias-input","parameter":0}}]}"#).unwrap();
    assert_eq!(legacy.version, LEGACY_VERSION);
    let current = legacy.upgrade(&registry_shape()).unwrap();
    assert_eq!(current.version, CURRENT_VERSION);
    assert!(current.outputs.is_empty());
    assert_eq!(current.results.len(), 2);
    let output = current
        .result(ResultTarget::Parameter { index: 2 })
        .unwrap();
    assert_eq!(output.overrides.len(), 2);
    assert!(output.may_alias());
    assert_eq!(output.overrides[1].policy, ResultPolicy::Undefined {});
    assert_eq!(output.overrides[1].when.succeeded, Some(false));
    current.validate_signature(&registry_shape()).unwrap();
    let round_trip = CallContract::decode(&serde_json::to_string(&current).unwrap()).unwrap();
    assert_eq!(round_trip, current);
}

#[test]
fn every_return_and_output_slot_has_one_complete_policy() {
    let shape = registry_shape();
    let mut current = CallContract::defaults(&shape);
    current.validate_signature(&shape).unwrap();
    current.results.pop();
    assert!(
        current
            .validate_signature(&shape)
            .unwrap_err()
            .to_string()
            .contains("every")
    );
    let mut duplicate = CallContract::defaults(&shape);
    duplicate.results.push(duplicate.results[0].clone());
    assert!(duplicate.validate_structure().is_err());
}

#[test]
fn undefined_and_defined_discarded_ownership_are_different_states() {
    let shape = registry_shape();
    let current = CallContract::defaults(&shape);
    let output = current
        .result(ResultTarget::Parameter { index: 2 })
        .unwrap();
    assert_eq!(
        output.on_failure,
        ResultPolicy::discarded(ResultOwnership::Owned {
            cleanup: Cleanup::RegCloseKey
        })
    );
    assert_ne!(output.on_failure, ResultPolicy::Undefined {});
    assert!(!output.on_failure.is_delivered());
    let invalid = r#"{"version":2,"results":[{"target":{"kind":"return"},"onSuccess":{"kind":"undefined","ownership":{"kind":"owned","cleanup":"reg-close-key"}},"onFailure":{"kind":"undefined"}}]}"#;
    assert!(CallContract::decode(invalid).is_err());
}

#[test]
fn metadata_owned_output_does_not_infer_defined_failure_storage() {
    let shape = registry_shape();
    let legacy = CallContract::default();
    let compatible = legacy.upgrade(&shape).unwrap();
    let generated = legacy.upgrade_metadata(&shape).unwrap();
    let target = ResultTarget::Parameter { index: 2 };
    assert_eq!(
        compatible.result(target).unwrap().on_failure,
        ResultPolicy::discarded(ResultOwnership::Owned {
            cleanup: Cleanup::RegCloseKey
        })
    );
    assert_eq!(
        generated.result(target).unwrap().on_failure,
        ResultPolicy::Undefined {}
    );
    generated.validate_signature(&shape).unwrap();
}

#[test]
fn direct_return_alias_and_owned_discard_are_validated_like_output_slots() {
    let mut shape = registry_shape();
    shape.parameters.pop();
    shape.return_type = Some(NativeType::Handle);
    shape.return_cleanup = Cleanup::RegCloseKey;
    shape.success_rule = SuccessRule::ReturnNonNull;
    let mut contract = CallContract::defaults(&shape);
    contract.results[0].on_success =
        ResultPolicy::delivered(ResultOwnership::AliasInput { parameter: 0 });
    contract.results[0].on_failure = ResultPolicy::discarded(ResultOwnership::Owned {
        cleanup: Cleanup::RegCloseKey,
    });
    contract.validate_signature(&shape).unwrap();
    shape.parameters[0].consumes_resource = true;
    assert!(contract.validate_signature(&shape).is_err());
}

#[test]
fn protocol_versions_and_unknown_fields_fail_closed() {
    for json in [
        r#"{"version":0}"#,
        r#"{"version":3}"#,
        r#"{"version":2,"outputs":[{"parameter":0,"when":{},"action":{"kind":"unavailable"}}]}"#,
        r#"{"version":1,"results":[{"target":{"kind":"return"},"onSuccess":{"kind":"undefined"},"onFailure":{"kind":"undefined"}}]}"#,
        r#"{"version":2,"unknown":true}"#,
        r#"{"version":2,"results":[{"target":{"kind":"return"},"onSuccess":{"kind":"defined","ownership":{"kind":"owned","cleanup":"none"},"delivery":"discard"},"onFailure":{"kind":"undefined"}}]}"#,
        r#"{"version":2,"results":[{"target":{"kind":"return","extra":1},"onSuccess":{"kind":"undefined"},"onFailure":{"kind":"undefined"}}]}"#,
        r#"{"version":2,"results":[{"target":{"kind":"return"},"onSuccess":{"kind":"defined","ownership":{"kind":"value","cleanup":"reg-close-key"},"delivery":"deliver"},"onFailure":{"kind":"undefined"}}]}"#,
        r#"{"version":2,"results":[{"target":{"kind":"return"},"onSuccess":{"kind":"defined","ownership":{"kind":"value"},"delivery":{"deliver":null}},"onFailure":{"kind":"undefined"}}]}"#,
        r#"{"version":2,"results":[{"target":{"kind":"return"},"onSuccess":{"kind":"defined","ownership":{"kind":"owned","cleanup":{"reg-close-key":null}},"delivery":"discard"},"onFailure":{"kind":"undefined"}}]}"#,
    ] {
        assert!(CallContract::decode(json).is_err(), "{json}");
    }

    assert!(CallContract::decode(&" ".repeat(MAX_CONTRACT_BYTES + 1)).is_err());
}

#[test]
fn structural_validation_bounds_indices_and_rejects_self_aliases() {
    for json in [
        r#"{"outputs":[{"parameter":1024,"when":{},"action":{"kind":"unavailable"}}]}"#,
        r#"{"outputs":[{"parameter":0,"when":{"inputs":[{"kind":"handle-in","parameter":1024,"values":[-1]}]},"action":{"kind":"unavailable"}}]}"#,
        r#"{"outputs":[{"parameter":0,"when":{},"action":{"kind":"alias-input","parameter":0}}]}"#,
        r#"{"outputs":[{"parameter":0,"when":{},"action":{"kind":"alias-input","parameter":1024}}]}"#,
        r#"{"resourceEffects":[{"kind":"add-file-completion-modes","handleParameter":0,"flagsParameter":1024}]}"#,
        r#"{"version":2,"results":[{"target":{"kind":"parameter","index":1024},"onSuccess":{"kind":"undefined"},"onFailure":{"kind":"undefined"}}]}"#,
        r#"{"version":2,"results":[{"target":{"kind":"parameter","index":0},"onSuccess":{"kind":"defined","ownership":{"kind":"alias-input","parameter":0},"delivery":"deliver"},"onFailure":{"kind":"undefined"}}]}"#,
    ] {
        assert!(CallContract::decode(json).is_err(), "{json}");
    }
}

#[test]
fn ambiguous_or_contradictory_result_conditions_are_rejected() {
    let shape = registry_shape();
    let mut contract = CallContract::defaults(&shape);
    let case = ResultOverride {
        when: Condition {
            return_value: Some(5),
            ..Condition::default()
        },
        policy: ResultPolicy::Undefined {},
    };
    contract.results[1].overrides = vec![case.clone(), case.clone()];
    assert!(
        contract
            .validate_signature(&shape)
            .unwrap_err()
            .to_string()
            .contains("overlapping")
    );
    contract.results[1].overrides[1].when.return_value = Some(6);
    contract.validate_signature(&shape).unwrap();
    contract.results[1].overrides[0].when.succeeded = Some(true);
    assert!(
        contract
            .validate_signature(&shape)
            .unwrap_err()
            .to_string()
            .contains("contradict")
    );
}

#[test]
fn native_handle_predicates_validate_pointer_width_without_truncation() {
    let mut shape = registry_shape();
    let mut contract = CallContract::defaults(&shape);
    contract.results[1].overrides.push(ResultOverride {
        when: Condition {
            inputs: vec![InputPredicate::HandleIn {
                parameter: 0,
                values: vec![-2147483646],
            }],
            ..Condition::default()
        },
        policy: ResultPolicy::Undefined {},
    });
    contract.validate_signature(&shape).unwrap();
    shape.pointer_width = 32;
    contract.validate_signature(&shape).unwrap();
    let InputPredicate::HandleIn { values, .. } =
        &mut contract.results[1].overrides[0].when.inputs[0]
    else {
        unreachable!()
    };
    values.push(i64::MAX);
    assert!(contract.validate_signature(&shape).is_err());
}

#[test]
fn schema_is_derived_from_the_same_serialized_types() {
    let schema = schema();
    let defs = schema.get("$defs").unwrap();
    for name in [
        "ResultTarget",
        "ResultContract",
        "ResultPolicy",
        "ResultOwnership",
        "InputPredicate",
        "ResourceEffect",
    ] {
        assert!(defs.get(name).is_some(), "{name}");
    }
    assert_eq!(schema["additionalProperties"], false);
}
