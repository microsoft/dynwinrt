// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use serde_json::{Value, json};

fn descriptor() -> Value {
    json!({
        "outputs": [
            {
                "parameter": 2,
                "when": {
                    "inputs": [{
                        "kind": "bits-in", "parameter": 0, "mask": 255, "values": [7, 9]
                    }],
                    "returnValue": 234
                },
                "action": {"kind": "unavailable"}
            },
            {
                "parameter": 4,
                "when": {
                    "inputs": [
                        {"kind": "null-or-empty", "parameter": 1, "elementWidth": 2},
                        {"kind": "handle-in", "parameter": 0, "values": [-2147483646]}
                    ],
                    "returnValue": null
                },
                "action": {"kind": "alias-input", "parameter": 0}
            }
        ],
        "resourceEffects": [{
            "kind": "add-file-completion-modes", "handleParameter": 0, "flagsParameter": 3
        }]
    })
}

#[test]
fn native_call_contract_wire_roundtrips_closed_data_and_defaults() {
    let value = descriptor();
    let contract: CallContract = serde_json::from_value(value.clone()).unwrap();
    validate_call_contract_structure(&contract).unwrap();
    assert_eq!(serde_json::to_value(contract).unwrap(), value);
    assert!(
        serde_json::from_str::<CallContract>("{}")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        serde_json::from_str::<Condition>("{}").unwrap(),
        Condition::default()
    );
    for data in [
        r#"{"inputs":[]}"#,
        r#"{"returnValue":null}"#,
        r#"{"inputs":[],"returnValue":null}"#,
    ] {
        assert_eq!(
            serde_json::from_str::<Condition>(data).unwrap(),
            Condition::default()
        );
    }
    let mut value = descriptor();
    value["outputs"][0]["when"]["returnValue"] = u64::MAX.into();
    let condition = &serde_json::from_value::<CallContract>(value)
        .unwrap()
        .outputs[0]
        .when;
    assert_eq!(condition.return_value, Some(u64::MAX));
    let handles = InputPredicate::HandleIn {
        parameter: 0,
        values: vec![i64::MIN, -2147483646, 0, i64::MAX],
    };
    assert_eq!(
        serde_json::from_str::<InputPredicate>(&serde_json::to_string(&handles).unwrap()).unwrap(),
        handles
    );
    for data in [
        r#"{"kind":"handle-in","parameter":0,"values":[9223372036854775808]}"#,
        r#"{"kind":"handle-in","parameter":0,"values":[-9223372036854775809]}"#,
        r#"{"kind":"handle-in","parameter":0,"values":[-2147483646],"mask":4294967295}"#,
    ] {
        assert!(
            serde_json::from_str::<InputPredicate>(data).is_err(),
            "{data}"
        );
    }
}

#[test]
fn native_call_contract_rejects_unknown_fields_and_missing_required_roles() {
    for path in [
        "",
        "/outputs/0",
        "/outputs/0/when",
        "/outputs/0/when/inputs/0",
        "/outputs/0/action",
        "/outputs/1/when/inputs/0",
        "/outputs/1/when/inputs/1",
        "/outputs/1/action",
        "/resourceEffects/0",
    ] {
        let mut value = descriptor();
        value
            .pointer_mut(path)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("script".into(), "arbitrary()".into());
        assert!(
            serde_json::from_value::<CallContract>(value).is_err(),
            "{path}"
        );
    }
    for (path, fields) in [
        ("/outputs/0", &["parameter", "when", "action"][..]),
        (
            "/outputs/0/when/inputs/0",
            &["kind", "parameter", "mask", "values"][..],
        ),
        (
            "/outputs/1/when/inputs/0",
            &["kind", "parameter", "elementWidth"][..],
        ),
        (
            "/outputs/1/when/inputs/1",
            &["kind", "parameter", "values"][..],
        ),
        ("/outputs/0/action", &["kind"][..]),
        ("/outputs/1/action", &["kind", "parameter"][..]),
        (
            "/resourceEffects/0",
            &["kind", "handleParameter", "flagsParameter"][..],
        ),
    ] {
        for field in fields {
            let mut value = descriptor();
            value
                .pointer_mut(path)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(*field);
            assert!(
                serde_json::from_value::<CallContract>(value).is_err(),
                "{path}/{field}"
            );
        }
    }
    for data in [
        r#"{"outputs":null}"#,
        r#"{"resourceEffects":null}"#,
        r#"{"outputs":[{"parameter":0,"when":null,"action":{"kind":"unavailable"}}]}"#,
        r#"{"outputs":[{"parameter":0,"when":{"returnValue":-1},"action":{"kind":"unavailable"}}]}"#,
        r#"{"outputs":[{"parameter":0,"when":{"returnValue":18446744073709551616},"action":{"kind":"unavailable"}}]}"#,
        r#"{"outputs":[{"parameter":0,"when":{"returnValue":"234"},"action":{"kind":"unavailable"}}]}"#,
        r#"{"outputs":[{"parameter":0,"when":{"inputs":null},"action":{"kind":"unavailable"}}]}"#,
        r#"{"outputs":[{"parameter":0,"when":{},"action":{"unavailable":{}}}]}"#,
        r#"{"outputs":[{"parameter":0,"when":{},"action":{"kind":"execute","code":"unchecked()"}}]}"#,
        r#"{"resourceEffects":[{"kind":"custom-adapter","name":"arbitrary"}]}"#,
    ] {
        assert!(
            serde_json::from_str::<CallContract>(data).is_err(),
            "{data}"
        );
    }
}

#[test]
fn native_call_contract_bounds_and_ambiguous_rules_fail_closed() {
    let mutations: &[fn(&mut Value)] = &[
        |v| v["outputs"][0]["parameter"] = 1024.into(),
        |v| v["outputs"][0]["parameter"] = (-1).into(),
        |v| v["outputs"][1]["parameter"] = v["outputs"][0]["parameter"].clone(),
        |v| v["outputs"][0]["when"]["inputs"][0]["parameter"] = 1024.into(),
        |v| v["outputs"][0]["when"]["inputs"][0]["mask"] = 0.into(),
        |v| v["outputs"][0]["when"]["inputs"][0]["mask"] = (-1).into(),
        |v| v["outputs"][0]["when"]["inputs"][0]["values"] = json!([]),
        |v| v["outputs"][0]["when"]["inputs"][0]["values"] = json!([7, 7]),
        |v| v["outputs"][0]["when"]["inputs"][0]["values"] = json!([256]),
        |v| v["outputs"][0]["when"]["inputs"][0]["values"] = json!([7.5]),
        |v| v["outputs"][0]["when"]["inputs"][0]["values"] = json!((0..65).collect::<Vec<_>>()),
        |v| {
            let predicate = v["outputs"][0]["when"]["inputs"][0].clone();
            v["outputs"][0]["when"]["inputs"] = json!(vec![predicate; 17]);
        },
        |v| v["outputs"][1]["when"]["inputs"][0]["elementWidth"] = 0.into(),
        |v| v["outputs"][1]["when"]["inputs"][0]["elementWidth"] = 3.into(),
        |v| v["outputs"][1]["when"]["inputs"][0]["elementWidth"] = 257.into(),
        |v| v["outputs"][1]["when"]["inputs"][1]["parameter"] = 1024.into(),
        |v| v["outputs"][1]["when"]["inputs"][1]["values"] = json!([]),
        |v| v["outputs"][1]["when"]["inputs"][1]["values"] = json!([-2147483646, -2147483646]),
        |v| v["outputs"][1]["when"]["inputs"][1]["values"] = json!((0..65).collect::<Vec<_>>()),
        |v| v["outputs"][1]["when"]["inputs"][1]["values"] = json!([-1.5]),
        |v| v["outputs"][1]["when"]["inputs"][1]["values"] = json!(["-2147483646"]),
        |v| v["outputs"][1]["action"]["parameter"] = 4.into(),
        |v| v["outputs"][1]["action"]["parameter"] = 1024.into(),
        |v| v["resourceEffects"][0]["handleParameter"] = 1024.into(),
        |v| v["resourceEffects"][0]["flagsParameter"] = 1024.into(),
        |v| v["resourceEffects"][0]["flagsParameter"] = 0.into(),
        |v| {
            let effect = v["resourceEffects"][0].clone();
            v["resourceEffects"].as_array_mut().unwrap().push(effect);
        },
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut value = descriptor();
        mutate(&mut value);
        let accepted = serde_json::from_value::<CallContract>(value)
            .is_ok_and(|contract| validate_call_contract_structure(&contract).is_ok());
        assert!(!accepted, "mutation {index}");
    }
}

#[test]
fn native_call_contract_schema_is_closed_at_every_wire_level() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    for name in ["callContract", "outputRule", "condition"] {
        assert_eq!(
            schema["$defs"][name]["additionalProperties"], false,
            "{name}"
        );
    }
    for name in ["inputPredicate", "outputAction", "resourceEffect"] {
        for variant in schema["$defs"][name]["oneOf"].as_array().unwrap() {
            assert_eq!(variant["additionalProperties"], false, "{name}");
            assert_eq!(
                variant["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|field| field.as_str().unwrap())
                    .collect::<BTreeSet<_>>(),
                variant["properties"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect(),
                "{name}"
            );
        }
    }
    assert_eq!(
        schema["$defs"]["callContract"]["properties"]["outputs"]["default"],
        json!([])
    );
    assert_eq!(
        schema["$defs"]["condition"]["properties"]["inputs"]["default"],
        json!([])
    );
    assert_eq!(
        schema["$defs"]["condition"]["properties"]["returnValue"]["default"],
        Value::Null
    );
    assert_eq!(
        schema["$defs"]["inputPredicate"]["oneOf"][0]["properties"]["values"]["uniqueItems"],
        true
    );
}
