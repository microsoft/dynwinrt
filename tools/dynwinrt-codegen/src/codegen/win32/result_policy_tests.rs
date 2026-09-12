// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::{ir::*, test_support::*, *};
use crate::win32_metadata::*;

fn handle_result(output: bool, capture_last_error: bool) -> (FunctionContract, ResultTarget) {
    let handle = RawType {
        base: RawBaseType::Named {
            namespace: "Tests.Resources".into(),
            name: "NATIVE_HANDLE".into(),
            kind: RawNamedKind::Handle {
                cleanup: Some("CloseHandle".into()),
            },
        },
        pointer_depth: 0,
        constness: RawConstness::Unspecified,
    };
    let mut raw = synthetic_function("NativeResult");
    raw.parameters = vec![
        parameter("source", handle.clone(), RawDirection::In),
        parameter("flags", scalar(RawScalar::U8), RawDirection::In),
    ];
    raw.supports_last_error = capture_last_error;
    if output {
        let mut slot = parameter(
            "pHandle",
            pointer(handle, 1, RawConstness::Mutable),
            RawDirection::Out,
        );
        slot.free_with = Some("CloseHandle".into());
        raw.parameters.push(slot);
        raw.return_status = RawStatusSemantics::ZeroIsSuccess;
    }
    let mut native = semantic(&raw);
    let target = if output {
        ResultTarget::Parameter { index: 2 }
    } else {
        native.return_type = Some(native.parameters[0].typ.clone());
        native.return_abi = Some(AbiType::Handle);
        native.return_native_name = native.parameters[0].native_name.clone();
        native.return_cleanup = Cleanup::CloseHandle;
        native.success_rule = SuccessRule::ReturnValidHandle;
        ResultTarget::Return {}
    };
    (native, target)
}

fn result_projection(
    function: &ProjectedFunction,
    target: ResultTarget,
) -> (&SurfaceType, Conversion, bool) {
    match (&function.return_shape, target) {
        (
            ReturnShape::Direct {
                typ,
                conversion,
                may_be_unavailable,
            },
            ResultTarget::Return {},
        ) => (typ, *conversion, *may_be_unavailable),
        (
            ReturnShape::Object {
                return_value: Some((typ, conversion)),
                return_may_be_unavailable,
                ..
            },
            ResultTarget::Return {},
        ) => (typ, *conversion, *return_may_be_unavailable),
        (ReturnShape::Object { outputs, .. }, ResultTarget::Parameter { .. }) => (
            &outputs[0].typ,
            outputs[0].conversion,
            outputs[0].may_be_unavailable,
        ),
        _ => panic!("expected projected native result"),
    }
}

#[test]
fn result_matrix_separates_success_failure_ownership_delivery_and_projection() {
    let owned = ResultOwnership::Owned {
        cleanup: Cleanup::CloseHandle,
    };
    let borrowed = ResultOwnership::Borrowed {};
    let alias = ResultOwnership::AliasInput { parameter: 0 };
    let cases = [
        (
            "undefined",
            ResultPolicy::delivered(owned),
            ResultPolicy::Undefined {},
            SurfaceType::Resource,
            Conversion::Resource,
            true,
        ),
        (
            "owned-discarded",
            ResultPolicy::delivered(owned),
            ResultPolicy::discarded(owned),
            SurfaceType::Resource,
            Conversion::Resource,
            true,
        ),
        (
            "owned-delivered",
            ResultPolicy::delivered(owned),
            ResultPolicy::delivered(owned),
            SurfaceType::Resource,
            Conversion::Resource,
            false,
        ),
        (
            "borrowed-delivered",
            ResultPolicy::delivered(owned),
            ResultPolicy::delivered(borrowed),
            SurfaceType::ResourceOrHandle,
            Conversion::ResourceOrHandle,
            false,
        ),
        (
            "alias-delivered",
            ResultPolicy::delivered(owned),
            ResultPolicy::delivered(alias),
            SurfaceType::ResourceOrHandle,
            Conversion::ResourceOrHandle,
            false,
        ),
        (
            "alias-discarded",
            ResultPolicy::delivered(owned),
            ResultPolicy::discarded(alias),
            SurfaceType::Resource,
            Conversion::Resource,
            true,
        ),
        (
            "borrowed-discarded",
            ResultPolicy::delivered(owned),
            ResultPolicy::discarded(borrowed),
            SurfaceType::Resource,
            Conversion::Resource,
            true,
        ),
        (
            "only-cleanup-owned",
            ResultPolicy::delivered(borrowed),
            ResultPolicy::discarded(owned),
            SurfaceType::Handle("NATIVE_HANDLE".into()),
            Conversion::BigInt,
            true,
        ),
        (
            "never-delivered",
            ResultPolicy::Undefined {},
            ResultPolicy::discarded(owned),
            SurfaceType::Handle("NATIVE_HANDLE".into()),
            Conversion::BigInt,
            true,
        ),
    ];
    for output in [false, true] {
        for capture in [false, true] {
            for (name, on_success, on_failure, typ, conversion, unavailable) in &cases {
                let (mut native, target) = handle_result(output, capture);
                let evidence = ResultContract {
                    target,
                    on_success: *on_success,
                    on_failure: *on_failure,
                    overrides: Vec::new(),
                };
                native.result_contracts = vec![evidence.clone()];
                let projected = project::project_function(&native)
                    .unwrap_or_else(|error| panic!("{name}: {error}"));
                assert_eq!(
                    projected.runtime.call_contract.version,
                    dynwinrt_win32_contracts::CURRENT_VERSION
                );
                assert!(projected.runtime.call_contract.outputs.is_empty());
                assert_eq!(
                    projected.runtime.call_contract.result(target),
                    Some(&evidence)
                );
                assert_eq!(
                    result_projection(&projected, target),
                    (typ, *conversion, *unavailable),
                    "{name}, output={output}, capture={capture}"
                );
                for width in [32, 64] {
                    projected
                        .runtime
                        .call_contract
                        .validate_signature(&projected.runtime.signature_shape(width))
                        .unwrap();
                }
            }
        }
    }
}

#[test]
fn legacy_owned_output_cleanup_is_not_promoted_to_new_metadata_failure_guarantees() {
    let (mut native, target) = handle_result(true, false);
    let legacy = CallContract::decode("{}").unwrap();
    assert_eq!(legacy.version, dynwinrt_win32_contracts::LEGACY_VERSION);
    native.call_contract = legacy.clone();
    let projected = project::project_function(&native).unwrap();
    let shape = projected.runtime.signature_shape(64);
    let compatibility = legacy.upgrade(&shape).unwrap();
    let historical = compatibility.result(target).unwrap();
    let fresh = projected.runtime.call_contract.result(target).unwrap();
    assert_eq!(
        historical.on_failure,
        ResultPolicy::discarded(ResultOwnership::Owned {
            cleanup: Cleanup::CloseHandle
        })
    );
    assert_eq!(fresh.on_success, historical.on_success);
    assert_eq!(fresh.on_failure, ResultPolicy::Undefined {});
    assert_eq!(
        projected
            .runtime
            .call_contract
            .result(ResultTarget::Return {}),
        compatibility.result(ResultTarget::Return {}),
        "scalar/status failure results remain defined"
    );

    native.call_contract =
        CallContract::decode(&serde_json::to_string(&compatibility).unwrap()).unwrap();
    assert_eq!(
        project::project_function(&native)
            .unwrap()
            .runtime
            .call_contract,
        compatibility,
        "an explicit complete v2 contract retains its failure guarantee"
    );

    native.call_contract = legacy;
    native.result_contracts = vec![historical.clone()];
    let evidenced = project::project_function(&native).unwrap();
    assert_eq!(
        evidenced.runtime.call_contract.result(target),
        Some(historical),
        "reviewed per-result evidence may explicitly define failure storage and cleanup"
    );
}

#[test]
fn resolved_override_policies_determine_return_and_output_resource_unions() {
    for output in [false, true] {
        let (mut native, target) = handle_result(output, false);
        let owned = ResultOwnership::Owned {
            cleanup: Cleanup::CloseHandle,
        };
        native.result_contracts = vec![ResultContract {
            target,
            on_success: ResultPolicy::delivered(owned),
            on_failure: ResultPolicy::Undefined {},
            overrides: vec![
                ResultOverride {
                    when: Condition {
                        succeeded: Some(true),
                        ..Condition::default()
                    },
                    policy: ResultPolicy::delivered(ResultOwnership::AliasInput { parameter: 0 }),
                },
                ResultOverride {
                    when: Condition {
                        succeeded: Some(false),
                        ..Condition::default()
                    },
                    policy: ResultPolicy::discarded(owned),
                },
            ],
        }];
        let projected = project::project_function(&native).unwrap();
        assert_eq!(
            result_projection(&projected, target),
            (
                &SurfaceType::ResourceOrHandle,
                Conversion::ResourceOrHandle,
                true
            )
        );
        let roundtrip =
            CallContract::decode(&serde_json::to_string(&projected.runtime.call_contract).unwrap())
                .unwrap();
        assert_eq!(roundtrip, projected.runtime.call_contract);
        assert!(roundtrip.result(target).unwrap().may_alias());
    }
}

#[test]
fn ambiguous_and_forbidden_result_contracts_fail_before_rendering() {
    let (base, target) = handle_result(false, false);
    let evidence = ResultContract {
        target,
        on_success: ResultPolicy::delivered(ResultOwnership::Owned {
            cleanup: Cleanup::CloseHandle,
        }),
        on_failure: ResultPolicy::Undefined {},
        overrides: Vec::new(),
    };
    for ownership in [
        ResultOwnership::Value {},
        ResultOwnership::Owned {
            cleanup: Cleanup::None,
        },
        ResultOwnership::Owned {
            cleanup: Cleanup::RegCloseKey,
        },
        ResultOwnership::AliasInput { parameter: 1 },
        ResultOwnership::AliasInput { parameter: 99 },
    ] {
        let mut native = base.clone();
        let mut result = evidence.clone();
        result.on_failure = ResultPolicy::delivered(ownership);
        native.result_contracts = vec![result];
        assert!(project::project_function(&native).is_err(), "{ownership:?}");
    }
    for conditions in [
        vec![Condition::default(), Condition::default()],
        vec![
            Condition {
                succeeded: Some(true),
                ..Condition::default()
            },
            Condition {
                return_value: Some(7),
                ..Condition::default()
            },
        ],
        vec![
            Condition {
                inputs: vec![InputPredicate::BitsIn {
                    parameter: 1,
                    mask: 3,
                    values: vec![1],
                }],
                ..Condition::default()
            },
            Condition {
                inputs: vec![InputPredicate::BitsIn {
                    parameter: 1,
                    mask: 1,
                    values: vec![1],
                }],
                ..Condition::default()
            },
        ],
        vec![Condition {
            inputs: vec![
                InputPredicate::BitsIn {
                    parameter: 1,
                    mask: 3,
                    values: vec![1],
                },
                InputPredicate::BitsIn {
                    parameter: 1,
                    mask: 3,
                    values: vec![2],
                },
            ],
            ..Condition::default()
        }],
        vec![Condition {
            succeeded: Some(true),
            return_value: Some(0),
            ..Condition::default()
        }],
    ] {
        let mut native = base.clone();
        let mut result = evidence.clone();
        result.overrides = conditions
            .into_iter()
            .map(|when| ResultOverride {
                when,
                policy: ResultPolicy::Undefined {},
            })
            .collect();
        native.result_contracts = vec![result];
        assert!(project::project_function(&native).is_err());
    }
    let mut native = base.clone();
    native.call_contract = CallContract::current(Vec::new(), Vec::new());
    assert!(
        project::project_function(&native).is_err(),
        "v2 cannot omit a native result"
    );
    for target in [
        ResultTarget::Parameter { index: 0 },
        ResultTarget::Parameter { index: 99 },
    ] {
        let mut native = base.clone();
        let mut result = evidence.clone();
        result.target = target;
        native.result_contracts = vec![result];
        assert!(project::project_function(&native).is_err(), "{target:?}");
    }
    for change in [
        |native: &mut FunctionContract| {
            native.parameters[0].consumes_resource = true;
        },
        |native: &mut FunctionContract| {
            native.parameters[0].native_name.as_mut().unwrap().1 = "UNRELATED_HANDLE".into();
        },
        |native: &mut FunctionContract| {
            native.parameters[0].pointer_depth = 1;
        },
    ] {
        let mut native = base.clone();
        change(&mut native);
        native.result_contracts = vec![ResultContract {
            on_success: ResultPolicy::delivered(ResultOwnership::AliasInput { parameter: 0 }),
            ..evidence.clone()
        }];
        assert!(project::project_function(&native).is_err());
    }
}

#[test]
fn scalar_and_resource_non_delivery_is_guarded_for_direct_status_and_output_slots() {
    let mut functions = Vec::new();
    for (name, output, status) in [
        ("ScalarDirect", false, false),
        ("ScalarResults", true, false),
        ("ScalarStatus", true, true),
    ] {
        let mut raw = synthetic_function(name);
        raw.return_status = if status {
            RawStatusSemantics::ZeroIsSuccess
        } else {
            RawStatusSemantics::None
        };
        if output {
            raw.parameters.push(parameter(
                "pValue",
                pointer(scalar(RawScalar::U32), 1, RawConstness::Mutable),
                RawDirection::Out,
            ));
        }
        let mut native = semantic(&raw);
        native.result_contracts.push(ResultContract {
            target: ResultTarget::Return {},
            on_success: ResultPolicy::delivered(ResultOwnership::Value {}),
            on_failure: ResultPolicy::discarded(ResultOwnership::Value {}),
            overrides: Vec::new(),
        });
        if output {
            native.result_contracts.push(ResultContract {
                target: ResultTarget::Parameter { index: 0 },
                on_success: ResultPolicy::delivered(ResultOwnership::Value {}),
                on_failure: ResultPolicy::Undefined {},
                overrides: Vec::new(),
            });
        }
        functions.push(project::project_function(&native).unwrap());
    }
    for (output, name) in [(false, "OwnedDirect"), (true, "OwnedOutput")] {
        let (mut native, _) = handle_result(output, false);
        native.name = name.into();
        native.entry_point = name.into();
        functions.push(project::project_function(&native).unwrap());
    }
    let generated = render::render(
        &ProjectedApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions,
            enums: Vec::new(),
            native_builders: Vec::new(),
            async_functions: Vec::new(),
        },
        "@test/runtime/win32",
    );
    assert!(generated.dts.contains("scalarDirect(): number | null"));
    assert!(generated.dts.contains("readonly status: number | null"));
    assert!(generated.dts.contains("readonly result: number | null"));
    assert!(generated.dts.contains("readonly value: number | null"));
    run_js(
        &generated,
        r#"
const assert=require('node:assert/strict')
const unavailable=Symbol('undefined native result')
const discarded=Symbol('defined native result discarded after cleanup')
let value=0
let binds=0
let conversions=0
const runtime={DynWin32:{
  isUnavailable:value=>value===unavailable||value===discarded,
  handle:value=>value,u8:value=>value,
  toNumber(value){assert.notEqual(value,unavailable);assert.notEqual(value,discarded);return Number(value)},
  toResource(value){assert.notEqual(value,unavailable);assert.notEqual(value,discarded);conversions++;return value}
},DynWin32Function:{bind(spec){
  binds++
  assert.equal(JSON.parse(spec.callContractDescriptor).version,2)
  return {invoke(){return {returnValue:spec.entryPoint==='OwnedOutput'?0:value,outputs:[value]}}}
}}}
"#,
        r#"
assert.equal(binds,0,'module import must not bind or dispatch native functions')
for(value of [unavailable,discarded,42]){
  const expected=typeof value==='symbol'?null:42
  assert.equal(projected.scalarDirect(),expected)
  const results=projected.scalarResults()
  assert.equal(results.result,expected)
  assert.equal(results.value,expected)
  const status=projected.scalarStatus()
  assert.equal(status.status,expected)
  assert.equal(status.value,expected)
}
const owner={}
for(value of [unavailable,discarded,owner]){
  const expected=typeof value==='symbol'?null:owner
  assert.equal(projected.ownedDirect(1n,0),expected)
  assert.equal(projected.ownedOutput(1n,0).handle,expected)
}
assert.equal(conversions,2,'non-delivered resources must not reach converters')
assert.equal(binds,5,'native plans remain lazy and cached')
"#,
    );
}

#[test]
fn actual_metadata_resolves_physical_buffers_aggregate_returns_and_evidenced_direct_returns() {
    let Some(path) = metadata() else { return };
    for (namespace, name) in [
        ("Windows.Win32.System.Registry", "RegQueryValueExW"),
        ("Windows.Win32.System.SystemInformation", "GetSystemTime"),
        (
            "Windows.Win32.System.Console",
            "GetLargestConsoleWindowSize",
        ),
        ("Windows.Win32.Storage.FileSystem", "CreateFileA"),
        ("Windows.Win32.Storage.FileSystem", "CreateFileW"),
    ] {
        let raw = metadata_function(&path, namespace, name);
        let native = semantic(&raw);
        let projected = project_one(raw);
        let shape = projected.runtime.signature_shape(64);
        projected
            .runtime
            .call_contract
            .validate_signature(&shape)
            .unwrap();
        assert_eq!(
            projected
                .runtime
                .call_contract
                .results
                .iter()
                .map(|result| result.target)
                .collect::<Vec<_>>(),
            shape.result_targets().collect::<Vec<_>>()
        );
        for (index, parameter) in projected.runtime.parameters.iter().enumerate() {
            if parameter.direction == Direction::In {
                assert!(
                    projected
                        .runtime
                        .call_contract
                        .result(ResultTarget::Parameter { index })
                        .is_none()
                );
            }
        }
        if name == "GetLargestConsoleWindowSize" {
            assert_eq!(
                shape.return_type,
                Some(dynwinrt_win32_contracts::NativeType::Aggregate)
            );
            assert_eq!(
                projected
                    .runtime
                    .call_contract
                    .result(ResultTarget::Return {})
                    .unwrap()
                    .on_success,
                ResultPolicy::delivered(ResultOwnership::Value {})
            );
        }
        if name.starts_with("CreateFile") {
            assert_eq!(native.result_contracts.len(), 1);
            assert_eq!(
                projected
                    .runtime
                    .call_contract
                    .result(ResultTarget::Return {}),
                Some(&native.result_contracts[0])
            );
        }
    }
}
