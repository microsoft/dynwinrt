// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::{ir::*, test_support::*, *};
use crate::win32_metadata::*;

fn conditional_function() -> FunctionContract {
    let handle = RawType {
        base: RawBaseType::Named {
            namespace: "Tests.Resources".into(),
            name: "NATIVE_FILE_HANDLE".into(),
            kind: RawNamedKind::Handle {
                cleanup: Some("CloseHandle".into()),
            },
        },
        pointer_depth: 0,
        constness: RawConstness::Unspecified,
    };
    let mut string = parameter(
        "subKey",
        RawType {
            base: RawBaseType::Named {
                namespace: "Tests.Strings".into(),
                name: "NATIVE_WIDE_STRING".into(),
                kind: RawNamedKind::StringPointer {
                    encoding: RawStringEncoding::Utf16,
                },
            },
            pointer_depth: 0,
            constness: RawConstness::Const,
        },
        RawDirection::In,
    );
    string.nullable = true;
    let mut alias = parameter(
        "pAlias",
        pointer(handle.clone(), 1, RawConstness::Mutable),
        RawDirection::Out,
    );
    alias.free_with = Some("CloseHandle".into());
    let mut data = parameter(
        "data",
        pointer(scalar(RawScalar::U8), 1, RawConstness::Mutable),
        RawDirection::Out,
    );
    data.buffer = Some(RawBuffer {
        element: scalar(RawScalar::U8),
        size: RawBufferSize::ByteCountParam(4),
    });
    let mut raw = synthetic_function("ConditionalNative");
    raw.return_status = RawStatusSemantics::ZeroIsSuccess;
    raw.parameters = vec![
        parameter(
            "tag",
            pointer(scalar(RawScalar::I16), 1, RawConstness::Mutable),
            RawDirection::Out,
        ),
        parameter("file", handle, RawDirection::In),
        string,
        alias,
        parameter(
            "count",
            pointer(scalar(RawScalar::U32), 1, RawConstness::Mutable),
            RawDirection::InOut,
        ),
        parameter("flags", scalar(RawScalar::U8), RawDirection::In),
        data,
    ];
    let mut function = semantic(&raw);
    function.call_contract = CallContract {
        outputs: vec![
            OutputRule {
                parameter: 3,
                when: Condition {
                    inputs: vec![InputPredicate::NullOrEmpty {
                        parameter: 2,
                        element_width: 2,
                    }],
                    return_value: None,
                },
                action: OutputAction::AliasInput { parameter: 1 },
            },
            OutputRule {
                parameter: 4,
                when: Condition {
                    inputs: vec![InputPredicate::BitsIn {
                        parameter: 1,
                        mask: 0xff,
                        values: vec![7],
                    }],
                    return_value: Some(234),
                },
                action: OutputAction::Unavailable {},
            },
        ],
        resource_effects: vec![ResourceEffect::AddFileCompletionModes {
            handle_parameter: 1,
            flags_parameter: 5,
        }],
    };
    function
}

#[test]
fn native_call_rules_keep_native_indices_through_hidden_inputs_and_output_slots() {
    let function = conditional_function();
    let projected = project::project_function(&function).unwrap();
    assert_eq!(projected.runtime.call_contract, function.call_contract);
    assert_eq!(
        projected
            .parameters
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["file", "subKey", "flags", "data"]
    );
    assert_eq!(
        projected.runtime.parameters[3].cleanup,
        Cleanup::CloseHandle
    );
    assert_eq!(projected.runtime.parameters[4].direction, Direction::InOut);
    assert_eq!(
        projected.inputs[2],
        InputExpression::BufferLength {
            parameter_index: 3,
            divisor: 1,
            abi: AbiType::U32,
        }
    );
    let ReturnShape::Object { outputs, .. } = &projected.return_shape else {
        panic!("native output slots")
    };
    assert_eq!(
        outputs,
        &[
            ProjectedOutput {
                name: "tag".into(),
                output_index: 0,
                typ: SurfaceType::Number,
                conversion: Conversion::Number,
                may_be_unavailable: false,
            },
            ProjectedOutput {
                name: "alias".into(),
                output_index: 1,
                typ: SurfaceType::ResourceOrHandle,
                conversion: Conversion::ResourceOrHandle,
                may_be_unavailable: false,
            },
            ProjectedOutput {
                name: "count".into(),
                output_index: 2,
                typ: SurfaceType::Number,
                conversion: Conversion::Number,
                may_be_unavailable: true,
            },
        ]
    );
    let mut renamed = function.clone();
    renamed.name = "UnrelatedNativeOperation".into();
    renamed.entry_point = "DifferentExport".into();
    renamed.dll = "other-native.dll".into();
    assert_eq!(
        project::project_function(&renamed)
            .unwrap()
            .runtime
            .call_contract,
        function.call_contract
    );
    let generated = render::render(
        &ProjectedApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![projected],
            enums: Vec::new(),
            native_builders: Vec::new(),
            async_functions: Vec::new(),
        },
        "@test/runtime/win32",
    );
    assert!(generated.dts.contains("readonly count: number | null"));
    assert!(
        generated
            .dts
            .contains("readonly alias: DynWin32Resource | bigint | null")
    );
    run_js(
        &generated,
        &format!(
            r#"
const assert=require('node:assert/strict')
const expected={}
const unavailable=Symbol('native unavailable')
const owner={{}}
let count=unavailable
let alias=owner
let status=0
let binds=0
let reads=0
const runtime={{DynWin32:{{
  handle:value=>value,wideString:value=>value,u8:value=>value,u32:value=>value,
  dataPointer:value=>value,byteLength:value=>value.length,
  isUnavailable:value=>value===unavailable,
  toNumber(value) {{ assert.notEqual(value,unavailable); return Number(value) }},
  toResourceOrHandle:value=>value
}},DynWin32Function:{{bind(spec){{
  binds++
  assert.equal(typeof spec.callContractDescriptor,'string')
  assert.deepEqual(JSON.parse(spec.callContractDescriptor),expected)
  assert.equal(spec.parameters[3].cleanup,'closeHandle')
  return {{invoke(args){{
    assert.equal(args[0],owner)
    assert.equal(args[2],4)
    assert.equal(args[3],3)
    assert.equal(args[4].length,4)
    return {{returnValue:status,get outputs(){{reads++;return [-5,alias,count]}}}}
  }}}}
}}}}}}
"#,
            serde_json::to_string(&function.call_contract).unwrap()
        ),
        r#"
for(const nativeAlias of [owner,77n,null]) {
  alias=nativeAlias
  for(const nativeCount of [unavailable,4096]) {
    count=nativeCount
    status=count===unavailable ? 0 : 234
    const result=projected.conditionalNative(owner,'not empty',3,Buffer.alloc(4))
    assert.equal(result.status,status)
    assert.equal(result.tag,-5)
    assert.equal(result.alias,nativeAlias)
    assert.equal(result.count,count===unavailable ? null : 4096)
  }
}
assert.equal(binds,1)
assert.equal(reads,6)
"#,
    );
}

#[test]
fn native_handle_predicates_preserve_signed_constants_and_require_handle_inputs() {
    let mut function = conditional_function();
    let predicate = InputPredicate::HandleIn {
        parameter: 1,
        values: vec![i64::MIN, -2147483646, 0, i64::MAX],
    };
    function.call_contract.outputs[0]
        .when
        .inputs
        .push(predicate.clone());
    let projected = project::project_function(&function).unwrap();
    assert_eq!(projected.runtime.call_contract, function.call_contract);
    assert_eq!(
        projected.runtime.call_contract.outputs[0].when.inputs[1],
        predicate
    );
    assert_eq!(
        projected.runtime.call_contract.outputs[0].action,
        OutputAction::AliasInput { parameter: 1 }
    );
    for index in [0, 2, 3, 4, 5, 6, 99] {
        let mut invalid = function.clone();
        invalid.call_contract.outputs[0].when.inputs[1] = InputPredicate::HandleIn {
            parameter: index,
            values: vec![-2147483646],
        };
        assert!(
            project::project_function(&invalid).is_err(),
            "parameter {index}"
        );
    }
    let mutations: &[fn(&mut FunctionContract)] = &[
        |f| f.parameters[1].abi = AbiType::U64,
        |f| f.parameters[1].typ = ValueType::Scalar(Scalar::NativeIsize),
        |f| f.parameters[1].direction = Direction::Out,
        |f| f.parameters[1].pointer_depth = 1,
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut invalid = function.clone();
        mutate(&mut invalid);
        assert!(
            project::project_function(&invalid).is_err(),
            "mutation {index}"
        );
    }
}

#[test]
fn native_call_rule_shapes_fail_before_projection() {
    let mutations: &[(&str, fn(&mut FunctionContract))] = &[
        ("missing output", |f| {
            f.call_contract.outputs[0].parameter = 99
        }),
        ("input target", |f| f.call_contract.outputs[0].parameter = 1),
        ("caller buffer target", |f| {
            f.call_contract.outputs[1].parameter = 6
        }),
        ("duplicate output", |f| {
            f.call_contract
                .outputs
                .push(f.call_contract.outputs[0].clone());
        }),
        ("alias to non-handle", |f| {
            f.call_contract.outputs[0].action = OutputAction::AliasInput { parameter: 5 };
        }),
        ("alias to output", |f| {
            f.call_contract.outputs[0].action = OutputAction::AliasInput { parameter: 0 };
        }),
        ("missing alias input", |f| {
            f.call_contract.outputs[0].action = OutputAction::AliasInput { parameter: 99 };
        }),
        ("non-owning alias target", |f| {
            f.parameters[3].cleanup = Cleanup::None
        }),
        ("alias cleanup mismatch", |f| {
            f.parameters[3].cleanup = Cleanup::RegCloseKey
        }),
        ("consuming alias input", |f| {
            f.parameters[1].consumes_resource = true
        }),
        ("owning alias input", |f| {
            f.parameters[1].cleanup = Cleanup::CloseHandle
        }),
        ("indirect alias input", |f| {
            f.parameters[1].pointer_depth = 1
        }),
        ("InOut alias target", |f| {
            f.parameters[3].direction = Direction::InOut
        }),
        ("missing predicate input", |f| {
            f.call_contract.outputs[0].when.inputs[0] = InputPredicate::NullOrEmpty {
                parameter: 99,
                element_width: 2,
            };
        }),
        ("wrong string encoding width", |f| {
            f.call_contract.outputs[0].when.inputs[0] = InputPredicate::NullOrEmpty {
                parameter: 2,
                element_width: 1,
            };
        }),
        ("unclassified data pointer", |f| {
            f.parameters[2].typ = ValueType::DataPointer
        }),
        ("string pointer slot", |f| f.parameters[2].pointer_depth = 1),
        ("output string", |f| {
            f.parameters[2].direction = Direction::Out
        }),
        ("pointer bits", |f| {
            f.call_contract.outputs[1].when.inputs[0] = InputPredicate::BitsIn {
                parameter: 2,
                mask: 255,
                values: vec![7],
            };
        }),
        ("output bits", |f| {
            f.call_contract.outputs[1].when.inputs[0] = InputPredicate::BitsIn {
                parameter: 0,
                mask: 255,
                values: vec![7],
            };
        }),
        ("overwide input bits", |f| {
            f.call_contract.outputs[1].when.inputs[0] = InputPredicate::BitsIn {
                parameter: 5,
                mask: 256,
                values: vec![256],
            };
        }),
        ("overwide return bits", |f| {
            f.call_contract.outputs[1].when.return_value = Some(1 << 32);
        }),
        ("floating return condition", |f| {
            f.return_abi = Some(AbiType::F32)
        }),
        ("void return condition", |f| f.return_abi = None),
        ("non-file resource state", |f| {
            f.parameters[1].resource_cleanup = Cleanup::RegCloseKey;
            f.parameters[3].cleanup = Cleanup::RegCloseKey;
        }),
        ("wide file modes", |f| {
            f.parameters[5].abi = AbiType::U16;
            f.parameters[5].typ = ValueType::Scalar(Scalar::U16);
        }),
        ("indirect file modes", |f| f.parameters[5].pointer_depth = 1),
        ("output file modes", |f| {
            f.parameters[5].direction = Direction::Out
        }),
        ("missing file state role", |f| {
            f.call_contract.resource_effects[0] = ResourceEffect::AddFileCompletionModes {
                handle_parameter: 99,
                flags_parameter: 5,
            };
        }),
        ("missing file flags role", |f| {
            f.call_contract.resource_effects[0] = ResourceEffect::AddFileCompletionModes {
                handle_parameter: 1,
                flags_parameter: 99,
            };
        }),
        ("duplicate file state role", |f| {
            f.call_contract
                .resource_effects
                .push(f.call_contract.resource_effects[0]);
        }),
    ];
    for (name, mutate) in mutations {
        let mut function = conditional_function();
        mutate(&mut function);
        assert!(model::validate_call_contract(&function).is_err(), "{name}");
        assert!(project::project_function(&function).is_err(), "{name}");
    }
}

#[test]
fn native_bits_predicates_support_exact_signed_unsigned_and_inout_widths() {
    for (scalar, abi, mask) in [
        (Scalar::I8, AbiType::I8, u8::MAX as u64),
        (Scalar::U8, AbiType::U8, u8::MAX as u64),
        (Scalar::I16, AbiType::I16, u16::MAX as u64),
        (Scalar::U16, AbiType::U16, u16::MAX as u64),
        (Scalar::Bool32, AbiType::Bool32, u32::MAX as u64),
        (Scalar::I32, AbiType::I32, u32::MAX as u64),
        (Scalar::U32, AbiType::U32, u32::MAX as u64),
        (Scalar::I64, AbiType::I64, u64::MAX),
        (Scalar::U64, AbiType::U64, u64::MAX),
    ] {
        let mut function = conditional_function();
        function.call_contract.outputs.remove(0);
        function.call_contract.resource_effects.clear();
        function.parameters[5].typ = ValueType::Scalar(scalar);
        function.parameters[5].abi = abi;
        function.call_contract.outputs[0].when.inputs = vec![InputPredicate::BitsIn {
            parameter: 5,
            mask,
            values: vec![mask],
        }];
        function.return_abi = Some(abi);
        function.return_type = Some(ValueType::Scalar(scalar));
        function.call_contract.outputs[0].when.return_value = Some(mask);
        model::validate_call_contract(&function).unwrap();
        if let Some(overwide) = mask.checked_add(1) {
            function.call_contract.outputs[0].when.return_value = Some(overwide);
            assert!(model::validate_call_contract(&function).is_err());
            function.call_contract.outputs[0].when.return_value = Some(mask);
            function.call_contract.outputs[0].when.inputs = vec![InputPredicate::BitsIn {
                parameter: 5,
                mask: overwide,
                values: vec![overwide],
            }];
            assert!(model::validate_call_contract(&function).is_err());
        }
    }
    let mut function = conditional_function();
    function.call_contract.outputs[1].when.inputs = vec![InputPredicate::BitsIn {
        parameter: 4,
        mask: 0xffff_ffff,
        values: vec![4],
    }];
    assert!(project::project_function(&function).is_ok());
}
