// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::sync::Arc;

use dynwinrt_win32_contracts::NativeType;

use super::{ir::*, test_support::*, *};
use crate::{
    win32_contracts::{METADATA_SHA256, Registry},
    win32_metadata::*,
};

fn raw_layout(function: &mut RawFunction) -> &mut RawNativeLayoutSet {
    let RawBaseType::Named {
        kind: RawNamedKind::NativeStruct { layout },
        ..
    } = &mut function.parameters[1].typ.base
    else {
        panic!("aggregate parameter")
    };
    layout
}

fn layout(function: &mut FunctionContract) -> &mut NativeLayout {
    let ValueType::NativeStructPointer { layout } = &mut function.parameters[1].typ else {
        panic!("aggregate parameter")
    };
    layout
}

fn synthetic_layout_evidence(layout: &mut RawNativeLayoutSet) {
    let entry = Registry::builtin()
        .unwrap()
        .type_entry("Windows.Win32.System.Threading", "PROCESS_INFORMATION")
        .unwrap();
    layout.evidence = Some(Arc::new(RawLayoutEvidence {
        selector: entry.selector.clone(),
        metadata_sha256: METADATA_SHA256.into(),
        shape_fingerprint: layout_shape_fingerprint(layout),
    }));
}

fn raw_result_function(direction: RawDirection) -> RawFunction {
    let handle = RawType {
        base: RawBaseType::Named {
            namespace: "Windows.Win32.Foundation".into(),
            name: "HANDLE".into(),
            kind: RawNamedKind::Handle {
                cleanup: Some("CloseHandle".into()),
            },
        },
        pointer_depth: 0,
        constness: RawConstness::Unspecified,
    };
    let fields = [
        ("hProcess", handle.clone()),
        ("hThread", handle),
        ("dwProcessId", scalar(RawScalar::U32)),
        ("dwThreadId", scalar(RawScalar::U32)),
    ]
    .into_iter()
    .map(|(name, typ)| RawNativeField {
        name: name.into(),
        typ,
        fixed_count: None,
        bitfield: false,
        flexible_array: false,
    })
    .collect();
    let mut function = synthetic_function("ProduceRecord");
    function.return_type = scalar(RawScalar::Bool32);
    function.supports_last_error = true;
    function.parameters = vec![
        parameter("flags", scalar(RawScalar::U32), RawDirection::In),
        parameter(
            "record",
            RawType {
                base: RawBaseType::Named {
                    namespace: "Windows.Win32.System.Threading".into(),
                    name: "PROCESS_INFORMATION".into(),
                    kind: RawNamedKind::NativeStruct {
                        layout: Box::new(RawNativeLayoutSet {
                            recursive: false,
                            evidence: None,
                            variants: vec![RawNativeLayout {
                                architectures: function.architectures,
                                kind: RawLayoutKind::Sequential,
                                packing: RawPacking::Default,
                                declared_size: None,
                                forced_alignment: None,
                                fields,
                            }],
                        }),
                    },
                },
                pointer_depth: 1,
                constness: RawConstness::Mutable,
            },
            direction,
        ),
        parameter(
            "pCount",
            pointer(scalar(RawScalar::U32), 1, RawConstness::Mutable),
            RawDirection::Out,
        ),
    ];
    synthetic_layout_evidence(raw_layout(&mut function));
    function
}

fn field_target(field: usize) -> ResultTarget {
    ResultTarget::AggregateField {
        parameter: 1,
        field,
    }
}

#[test]
fn logical_aggregate_results_survive_physical_pointer_input_lowering() {
    for direction in [RawDirection::Out, RawDirection::InOut] {
        let function = project_one(raw_result_function(direction));
        let parameter = &function.runtime.parameters[1];
        assert_eq!(parameter.abi, AbiType::Pointer);
        assert_eq!(parameter.direction, Direction::In);
        assert_eq!(parameter.cleanup, Cleanup::None);
        assert!(parameter.aggregate.is_none());
        let layout = parameter.pointee_aggregate.as_ref().unwrap();
        assert_eq!(
            layout.output_fields,
            ["hProcess", "hThread", "dwProcessId", "dwThreadId"]
        );
        assert_eq!(
            function
                .runtime
                .call_contract
                .results
                .iter()
                .map(|result| result.target)
                .collect::<Vec<_>>(),
            [
                ResultTarget::Return {},
                ResultTarget::Parameter { index: 2 },
                field_target(0),
                field_target(1),
                field_target(2),
                field_target(3),
            ]
        );
        for width in [32, 64] {
            let shape = function.runtime.signature_shape(width);
            function
                .runtime
                .call_contract
                .validate_signature(&shape)
                .unwrap();
            assert_eq!(shape.parameters[1].typ, NativeType::Pointer);
            assert_eq!(shape.parameters[1].direction, Direction::In);
            assert_eq!(shape.parameters[1].result_fields.len(), 4);
            for field in 0..4 {
                let target = field_target(field);
                let (typ, cleanup, ownership) = if field < 2 {
                    (
                        NativeType::Handle,
                        Cleanup::CloseHandle,
                        ResultOwnership::Owned {
                            cleanup: Cleanup::CloseHandle,
                        },
                    )
                } else {
                    (NativeType::U32, Cleanup::None, ResultOwnership::Value {})
                };
                assert_eq!(shape.result_type(target).unwrap(), typ);
                assert_eq!(shape.result_cleanup(target), cleanup);
                let result = function.runtime.call_contract.result(target).unwrap();
                assert_eq!(result.on_success, ResultPolicy::delivered(ownership));
                assert_eq!(result.on_failure, ResultPolicy::Undefined {});
                assert!(result.overrides.is_empty());
            }
        }
        assert!(
            matches!(&function.return_shape, ReturnShape::Object { outputs, .. }
            if outputs.len() == 1 && outputs[0].output_index == 0)
        );
    }
}

#[test]
fn result_field_indices_follow_declared_outputs_not_native_fields_or_output_cells() {
    let mut raw = raw_result_function(RawDirection::Out);
    let native = raw_layout(&mut raw);
    native.variants[0].fields.rotate_left(2);
    native.variants[0].fields.insert(
        0,
        RawNativeField {
            name: "inputOnly".into(),
            typ: scalar(RawScalar::U32),
            fixed_count: None,
            bitfield: false,
            flexible_array: false,
        },
    );
    synthetic_layout_evidence(native);
    let function = project_one(raw);
    let layout = function.runtime.parameters[1]
        .pointee_aggregate
        .as_ref()
        .unwrap();
    assert_eq!(layout.x64.fields[0].name, "inputOnly");
    assert_eq!(
        layout.output_fields,
        ["dwProcessId", "dwThreadId", "hProcess", "hThread"]
    );
    let shape = function.runtime.signature_shape(64);
    assert_eq!(shape.result_type(field_target(0)).unwrap(), NativeType::U32);
    assert_eq!(
        shape.result_type(field_target(2)).unwrap(),
        NativeType::Handle
    );
    assert!(
        shape
            .result_type(ResultTarget::Parameter { index: 1 })
            .is_err()
    );
    assert!(shape.result_type(field_target(4)).is_err());
}

#[test]
fn input_pointees_and_unproven_output_field_names_do_not_acquire_result_contracts() {
    let mut raw = raw_result_function(RawDirection::In);
    let input = project_one(raw.clone());
    assert!(input.runtime.parameters[1].pointee_aggregate.is_none());
    assert!(
        input.runtime.signature_shape(64).parameters[1]
            .result_fields
            .is_empty()
    );

    raw.parameters[1].direction = RawDirection::Out;
    let native = raw_layout(&mut raw);
    native.evidence = None;
    for field in &mut native.variants[0].fields {
        field.typ = scalar(RawScalar::U32);
    }
    let RawBaseType::Named {
        namespace, name, ..
    } = &mut raw.parameters[1].typ.base
    else {
        unreachable!()
    };
    *namespace = "Tests".into();
    *name = "UnprovenFields".into();
    let output = project_one(raw);
    assert!(output.runtime.parameters[1].pointee_aggregate.is_none());
    assert!(
        output.runtime.signature_shape(64).parameters[1]
            .result_fields
            .is_empty()
    );
}

#[test]
fn dropped_pointee_contracts_and_invalid_field_targets_fail_before_dispatch() {
    let base = semantic(&raw_result_function(RawDirection::Out));
    let function = project::project_function(&base).unwrap();
    let mut runtime = function.runtime.clone();
    runtime.parameters[1].pointee_aggregate = None;
    assert!(
        runtime
            .call_contract
            .validate_signature(&runtime.signature_shape(64))
            .is_err()
    );
    for target in [
        field_target(4),
        field_target(usize::MAX),
        ResultTarget::AggregateField {
            parameter: 0,
            field: 0,
        },
        ResultTarget::AggregateField {
            parameter: 99,
            field: 0,
        },
        ResultTarget::Parameter { index: 1 },
    ] {
        let mut native = base.clone();
        native.result_contracts.push(ResultContract {
            target,
            on_success: ResultPolicy::Undefined {},
            on_failure: ResultPolicy::Undefined {},
            overrides: Vec::new(),
        });
        assert!(project::project_function(&native).is_err(), "{target:?}");
    }
    for direction in [Direction::Out, Direction::InOut] {
        let mut shape = function.runtime.signature_shape(64);
        shape.parameters[1].direction = direction;
        assert!(
            function
                .runtime
                .call_contract
                .validate_signature(&shape)
                .is_err()
        );
    }
    let mut shape = function.runtime.signature_shape(64);
    shape.parameters[1].result_fields[0].typ = NativeType::U32;
    assert!(
        function
            .runtime
            .call_contract
            .validate_signature(&shape)
            .is_err()
    );
}

#[test]
fn aggregate_result_layout_drift_never_silently_drops_or_reinterprets_owned_fields() {
    let base = semantic(&raw_result_function(RawDirection::Out));
    let mutations: &[fn(&mut NativeLayout)] = &[
        |layout| layout.output_fields.clear(),
        |layout| {
            layout.output_fields.remove(0);
        },
        |layout| layout.output_fields.push("unknown".into()),
        |layout| layout.output_fields.push(layout.output_fields[0].clone()),
        |layout| layout.output_fields.swap(0, 1),
        |layout| layout.kind = NativeAggregateKind::Union,
        |layout| layout.x64.fields[0].typ = NativeFieldType::Pointer,
        |layout| {
            layout.x64.fields[0].typ = NativeFieldType::Handle {
                cleanup: Cleanup::None,
            }
        },
        |layout| {
            layout.x64.fields[0].typ = NativeFieldType::Handle {
                cleanup: Cleanup::RegCloseKey,
            }
        },
        |layout| layout.x64.fields[0].count = 2,
        |layout| layout.x64.fields[1].offset = 0,
        |layout| layout.x64.fields[3].offset = layout.x64.size,
        |layout| layout.x86.fields[0].typ = NativeFieldType::Scalar(NativeScalar::U32),
        |layout| layout.arm64.fields[3].name = layout.arm64.fields[2].name.clone(),
        |layout| layout.arm64.size = 1,
        |layout| layout.arm64.alignment = 0,
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut native = base.clone();
        mutate(layout(&mut native));
        assert!(
            project::project_function(&native).is_err(),
            "layout mutation {index}"
        );
    }
}

#[test]
fn field_policies_require_exact_scalar_or_independent_handle_ownership() {
    let base = semantic(&raw_result_function(RawDirection::Out));
    for (field, ownership) in [
        (0, ResultOwnership::Value {}),
        (0, ResultOwnership::Borrowed {}),
        (0, ResultOwnership::AliasInput { parameter: 0 }),
        (
            0,
            ResultOwnership::Owned {
                cleanup: Cleanup::None,
            },
        ),
        (
            0,
            ResultOwnership::Owned {
                cleanup: Cleanup::RegCloseKey,
            },
        ),
        (
            2,
            ResultOwnership::Owned {
                cleanup: Cleanup::CloseHandle,
            },
        ),
        (2, ResultOwnership::Borrowed {}),
    ] {
        let mut native = base.clone();
        native.result_contracts.push(ResultContract {
            target: field_target(field),
            on_success: ResultPolicy::delivered(ownership),
            on_failure: ResultPolicy::Undefined {},
            overrides: Vec::new(),
        });
        assert!(
            project::project_function(&native).is_err(),
            "{field}: {ownership:?}"
        );
    }
    let mut native = base;
    native.result_contracts = vec![
        ResultContract {
            target: field_target(0),
            on_success: ResultPolicy::delivered(ResultOwnership::Owned {
                cleanup: Cleanup::CloseHandle,
            }),
            on_failure: ResultPolicy::discarded(ResultOwnership::Owned {
                cleanup: Cleanup::CloseHandle,
            }),
            overrides: Vec::new(),
        },
        ResultContract {
            target: field_target(2),
            on_success: ResultPolicy::delivered(ResultOwnership::Value {}),
            on_failure: ResultPolicy::delivered(ResultOwnership::Value {}),
            overrides: Vec::new(),
        },
    ];
    let projected = project::project_function(&native).unwrap();
    for evidence in &native.result_contracts {
        assert_eq!(
            projected.runtime.call_contract.result(evidence.target),
            Some(evidence)
        );
    }
    assert_eq!(
        projected
            .runtime
            .call_contract
            .result(field_target(1))
            .unwrap()
            .on_failure,
        ResultPolicy::Undefined {}
    );
}

#[test]
fn new_owned_union_array_nested_and_cleanup_mismatches_fail_closed() {
    for mutation in [
        (|layout: &mut RawNativeLayoutSet| layout.variants[0].kind = RawLayoutKind::Union)
            as fn(&mut RawNativeLayoutSet),
        |layout| layout.variants[0].fields[0].fixed_count = Some(2),
        |layout| {
            let RawBaseType::Named {
                kind: RawNamedKind::Handle { cleanup },
                ..
            } = &mut layout.variants[0].fields[0].typ.base
            else {
                unreachable!()
            };
            *cleanup = Some("RegCloseKey".into());
        },
    ] {
        let mut raw = raw_result_function(RawDirection::Out);
        mutation(raw_layout(&mut raw));
        synthetic_layout_evidence(raw_layout(&mut raw));
        assert!(
            project_apis(&apis(vec![raw]))
                .projected
                .functions
                .is_empty()
        );
    }
    let mut raw = raw_result_function(RawDirection::Out);
    let mut nested = raw.parameters[1].typ.clone();
    nested.pointer_depth = 0;
    let native = raw_layout(&mut raw);
    native.evidence = None;
    native.variants[0].fields = vec![RawNativeField {
        name: "nested".into(),
        typ: nested,
        fixed_count: None,
        bitfield: false,
        flexible_array: false,
    }];
    let RawBaseType::Named {
        namespace, name, ..
    } = &mut raw.parameters[1].typ.base
    else {
        unreachable!()
    };
    *namespace = "Tests".into();
    *name = "NestedResults".into();
    let projected = project_apis(&apis(vec![raw]));
    assert!(projected.projected.functions.is_empty());
    assert!(
        projected.omitted[0]
            .reason
            .contains("nested native aggregate result fields")
    );
}

#[test]
fn rendered_wrappers_delegate_field_lifetimes_before_any_javascript_result_operation() {
    let (generated, omitted) = generate_apis_files(
        &apis(vec![raw_result_function(RawDirection::Out)]),
        "@test/runtime/win32",
    );
    assert!(omitted.is_empty());
    assert!(!generated.js.contains("prepareNativeStructCall"));
    assert!(!generated.js.contains("markNativeStructCallResult"));
    assert!(
        generated
            .js
            .contains("pointeeDescriptor: _nativeLayout_PROCESS_INFORMATION")
    );
    run_js(
        &generated,
        r#"
const assert = require('node:assert/strict')
let failAt
let invoked = 0
const owned = new Map()
const runtime = {DynWin32: {
  u32: value => value,
  toBoolean(value) { if (failAt === 'convert') throw new Error('convert'); return value !== 0 },
  toNumber: value => value,
  createNativeStruct(descriptor) { return { descriptor, length: JSON.parse(descriptor).x64.size } },
  nativeStruct(value, descriptor, nullable) {
    assert.equal(nullable, false)
    assert.equal(value.descriptor, descriptor)
    return value
  },
  takeNativeStructResource(value, descriptor, field, cleanup) {
    assert.equal(value.descriptor, descriptor)
    assert.equal(cleanup, 'closeHandle')
    const fields = owned.get(value)
    const resource = fields.get(field)
    fields.delete(field)
    return resource
  },
  getNativeStructU32(value, descriptor, field) {
    assert.equal(value.descriptor, descriptor)
    return owned.get(value).get(field)
  },
  prepareNativeStructCall() { throw new Error('JS preparation is not a native ownership boundary') },
  markNativeStructCallResult() { throw new Error('JS cannot register native field lifetimes') },
}, DynWin32Function: {bind(spec) {
  const parameter = spec.parameters[1]
  assert.equal(parameter.type, 'pointer')
  assert.equal(parameter.direction, 'in')
  assert.equal(parameter.aggregateDescriptor, undefined)
  assert.equal(parameter.cleanup, 'none')
  const layout = JSON.parse(parameter.pointeeDescriptor)
  assert.deepEqual(layout.outputFields, ['hProcess','hThread','dwProcessId','dwThreadId'])
  const contract = JSON.parse(spec.callContractDescriptor)
  assert.equal(contract.version, 3)
  assert.equal(contract.results.length, 6)
  for (let field = 0; field < 4; field++) {
    const result = contract.results.find(result => result.target.kind === 'aggregate-field' && result.target.field === field)
    assert.equal(result.target.parameter, 1)
    assert.deepEqual(result.onSuccess, {
      kind: 'defined',
      ownership: field < 2 ? {kind: 'owned', cleanup: 'close-handle'} : {kind: 'value'},
      delivery: 'deliver'
    })
    assert.deepEqual(result.onFailure, {kind: 'undefined'})
  }
  return {invoke([flags, record]) {
    assert.equal(record.descriptor, parameter.pointeeDescriptor)
    invoked++
    // The native adapter must have registered field owners before wrapping its return.
    owned.set(record, new Map([
      ['hProcess', {kind:'process', call:invoked}],
      ['hThread', {kind:'thread', call:invoked}],
      ['dwProcessId', 31], ['dwThreadId', 47]
    ]))
    if (failAt === 'wrap') throw new Error('wrap')
    return {
      get returnValue() { if (failAt === 'returnValue') throw new Error('returnValue'); return 1 },
      get outputs() { if (failAt === 'outputs') throw new Error('outputs'); return [7] },
      get succeeded() { throw new Error('JS success marking is forbidden') },
      lastError: 0
    }
  }}
}}}
"#,
        r#"
for (failAt of ['wrap', 'returnValue', 'outputs', 'convert', undefined]) {
  const record = projected.createProcessInformation()
  if (failAt) assert.throws(() => projected.produceRecord(0, record), new RegExp(failAt))
  else assert.equal(projected.produceRecord(0, record).count, 7)
  const process = projected.takeProcessInformationProcess(record)
  const thread = projected.takeProcessInformationThread(record)
  assert.equal(process.kind, 'process')
  assert.equal(thread.kind, 'thread')
  assert.notEqual(process, thread)
  assert.equal(process.call, invoked)
  assert.equal(thread.call, invoked)
  assert.equal(projected.getProcessInformationProcessId(record), 31)
  assert.equal(projected.getProcessInformationThreadId(record), 47)
}
assert.equal(invoked, 5)
"#,
    );
}
