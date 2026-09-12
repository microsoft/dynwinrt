// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::{ir::*, test_support::*, *};
use crate::win32_metadata::*;

fn counted(
    name: &str,
    size: RawBufferSize,
    count_direction: RawDirection,
    nullable: bool,
) -> RawFunction {
    let mut function = synthetic_function(name);
    let mut buffer = parameter(
        "data",
        pointer(scalar(RawScalar::U16), 1, RawConstness::Mutable),
        RawDirection::Out,
    );
    buffer.nullable = nullable;
    buffer.buffer = Some(RawBuffer {
        element: scalar(RawScalar::U16),
        size,
    });
    let count_type = if count_direction == RawDirection::In {
        scalar(RawScalar::U32)
    } else {
        pointer(scalar(RawScalar::U32), 1, RawConstness::Mutable)
    };
    function.parameters = vec![buffer, parameter("count", count_type, count_direction)];
    function
}

#[test]
fn byte_and_element_capacity_relations_preserve_inout_output_order_and_nullability() {
    for (size, divisor) in [
        (RawBufferSize::ElementCountParam(1), 2),
        (RawBufferSize::ByteCountParam(1), 1),
    ] {
        for count_direction in [RawDirection::In, RawDirection::InOut] {
            for nullable in [false, true] {
                let function = counted("ReadCapacity", size.clone(), count_direction, nullable);
                let native = semantic(&function);
                let buffer = native.parameters[0].buffer.as_ref().unwrap();
                assert_eq!(buffer.count_parameter, Some(1));
                assert_eq!(buffer.constant_count, None);
                assert_eq!(buffer.count_is_bytes, divisor == 1);
                assert_eq!(buffer.element_size, divisor);
                let projected = project_one(function);
                assert_eq!(
                    projected.parameters,
                    vec![SurfaceParameter {
                        name: "data".into(),
                        typ: SurfaceType::Buffer,
                        nullable,
                        minimum_bytes: None,
                        alignment: Some(2),
                    }]
                );
                assert_eq!(
                    projected.inputs,
                    vec![
                        InputExpression::Surface {
                            parameter_index: 0,
                            conversion: Conversion::DataPointer
                        },
                        InputExpression::BufferLength {
                            parameter_index: 0,
                            divisor,
                            abi: AbiType::U32
                        },
                    ]
                );
                assert_eq!(projected.runtime.parameters[0].abi, AbiType::Pointer);
                assert_eq!(projected.runtime.parameters[0].direction, Direction::In);
                assert_eq!(projected.runtime.parameters[0].nullable, nullable);
                if count_direction == RawDirection::InOut {
                    assert_eq!(projected.runtime.parameters[1].direction, Direction::InOut);
                    assert_eq!(
                        projected.return_shape,
                        ReturnShape::Object {
                            return_may_be_unavailable: false,
                            status: false,
                            return_value: Some((SurfaceType::Number, Conversion::Number)),
                            last_error: false,
                            outputs: vec![ProjectedOutput {
                                name: "count".into(),
                                output_index: 0,
                                typ: SurfaceType::Number,
                                conversion: Conversion::Number,
                                may_be_unavailable: false,
                            }],
                        }
                    );
                } else {
                    assert_eq!(projected.runtime.parameters[1].direction, Direction::In);
                    assert!(matches!(projected.return_shape, ReturnShape::Direct { .. }));
                }
            }
        }
    }
}

#[test]
fn incomplete_conflicting_or_nonintegral_counts_never_emit_a_callable_export() {
    let cases: &[(&str, fn(&mut RawFunction))] = &[
        ("missing count", |f| {
            f.parameters[0].buffer.as_mut().unwrap().size = RawBufferSize::ElementCountParam(9)
        }),
        ("counts itself", |f| {
            f.parameters[0].buffer.as_mut().unwrap().size = RawBufferSize::ElementCountParam(0)
        }),
        ("integer scalar", |f| {
            f.parameters[1].typ = scalar(RawScalar::F32)
        }),
        ("complete size", |f| {
            f.parameters[0].buffer.as_mut().unwrap().size = RawBufferSize::Unknown
        }),
        ("multiple buffers", |f| {
            let mut b = f.parameters[0].clone();
            b.name = "second".into();
            f.parameters.push(b);
        }),
        ("overflows", |f| {
            f.parameters[0].buffer.as_mut().unwrap().size = RawBufferSize::Constant(usize::MAX)
        }),
    ];
    for (reason, mutate) in cases {
        let mut function = counted(
            "InvalidCount",
            RawBufferSize::ElementCountParam(1),
            RawDirection::In,
            true,
        );
        mutate(&mut function);
        let raw = apis(vec![function]);
        let projection = project_apis(&raw);
        assert!(projection.projected.functions.is_empty(), "{reason}");
        assert_eq!(projection.omitted.len(), 1);
        assert!(
            projection.omitted[0].reason.contains(reason),
            "{reason}: {:?}",
            projection.omitted
        );
        let (generated, _) = generate_apis_files(&raw, "@test/runtime/win32");
        assert!(!generated.js.contains("exports.invalidCount"));
    }
}

#[test]
fn generated_buffer_counts_use_backing_extent_check_divisibility_and_keep_input_positions() {
    let element = counted(
        "ReadElements",
        RawBufferSize::ElementCountParam(1),
        RawDirection::InOut,
        true,
    );
    let bytes = counted(
        "ReadBytes",
        RawBufferSize::ByteCountParam(1),
        RawDirection::InOut,
        true,
    );
    let mut fixed = synthetic_function("ReadFixed");
    let mut buffer = parameter(
        "data",
        pointer(scalar(RawScalar::U16), 1, RawConstness::Mutable),
        RawDirection::Out,
    );
    buffer.buffer = Some(RawBuffer {
        element: scalar(RawScalar::U16),
        size: RawBufferSize::Constant(3),
    });
    fixed.parameters.push(buffer);
    let (generated, omitted) =
        generate_apis_files(&apis(vec![element, bytes, fixed]), "@test/runtime/win32");
    assert!(omitted.is_empty());
    run_js(
        &generated,
        r#"
const assert=require('node:assert/strict')
const calls=[]
const byteLength=Object.getOwnPropertyDescriptor(Object.getPrototypeOf(Uint8Array.prototype),'byteLength').get
const native={
  byteLength(value) { return Reflect.apply(byteLength,value,[]) },
  alignedDataPointer(value,alignment,nullable) {
    if (value===null && !nullable) throw new TypeError('required pointer')
    return { value,alignment,nullable }
  },
  u32: value => ({scalar:value}),
  toNumber: value => value,
}
const runtime={DynWin32:native,DynWin32Function:{bind(spec) {
  return {invoke(args) {
    calls.push({spec,args})
    let reads=0
    return {returnValue:7,get outputs() { assert.equal(++reads,1); return [11] }}
  }}
}}}
"#,
        r#"
const storage=Buffer.alloc(6)
Object.defineProperty(storage,'byteLength',{get(){throw Error('JS length getter was read')}})
Object.defineProperty(storage,'length',{value:0xffffffff})
let result=projected.readElements(storage)
assert.equal(result.result,7)
assert.equal(result.count,11)
assert.equal(calls.at(-1).args[0].value,storage)
assert.equal(calls.at(-1).args[0].alignment,2)
assert.equal(calls.at(-1).args[1].scalar,3)
projected.readBytes(storage)
assert.equal(calls.at(-1).args[1].scalar,6)
projected.readElements(null)
assert.equal(calls.at(-1).args[0].value,null)
assert.equal(calls.at(-1).args[1].scalar,0)
const before=calls.length
assert.throws(()=>projected.readElements(Buffer.alloc(5)),/divisible/)
assert.throws(()=>projected.readFixed(Buffer.alloc(5)),/at least 6 bytes/)
assert.throws(()=>projected.readFixed(null),/required/)
assert.equal(calls.length,before)
projected.readFixed(Buffer.alloc(6))
assert.equal(calls.at(-1).args.length,1)
"#,
    );
}

#[test]
fn native_return_conventions_last_error_and_outputs_are_typed_independently() {
    for (kind, status, capture, success, return_shape) in [
        (
            RawScalar::U32,
            RawStatusSemantics::None,
            false,
            SuccessRule::Always,
            ReturnShape::Direct {
                typ: SurfaceType::Number,
                conversion: Conversion::Number,
                may_be_unavailable: false,
            },
        ),
        (
            RawScalar::U32,
            RawStatusSemantics::ZeroIsSuccess,
            false,
            SuccessRule::ReturnZero,
            ReturnShape::Object {
                status: true,
                return_may_be_unavailable: false,
                return_value: None,
                outputs: vec![],
                last_error: false,
            },
        ),
        (
            RawScalar::I32,
            RawStatusSemantics::SignedNonNegativeIsSuccess,
            false,
            SuccessRule::SignedNonNegative,
            ReturnShape::Object {
                status: true,
                return_may_be_unavailable: false,
                return_value: None,
                outputs: vec![],
                last_error: false,
            },
        ),
        (
            RawScalar::Bool32,
            RawStatusSemantics::None,
            true,
            SuccessRule::ReturnNonZero,
            ReturnShape::Object {
                status: false,
                return_may_be_unavailable: false,
                return_value: Some((SurfaceType::Boolean, Conversion::Boolean)),
                outputs: vec![],
                last_error: true,
            },
        ),
    ] {
        let mut function = synthetic_function("NativeReturn");
        function.return_type = scalar(kind);
        function.return_status = status;
        function.supports_last_error = capture;
        let projected = project_one(function);
        assert_eq!(projected.runtime.success_rule, success);
        assert_eq!(projected.runtime.capture_last_error, capture);
        assert_eq!(projected.runtime.return_cleanup, Cleanup::None);
        assert_eq!(projected.return_shape, return_shape);
    }
}

#[test]
fn allocator_owned_pointer_returns_use_exact_cleanup_and_nonnull_success() {
    for (native_cleanup, cleanup) in [
        ("LocalFree", Cleanup::LocalFree),
        ("GlobalFree", Cleanup::GlobalFree),
        ("CoTaskMemFree", Cleanup::CoTaskMemFree),
        ("CredFree", Cleanup::CredFree),
    ] {
        let mut function = synthetic_function("AllocateData");
        function.return_type = RawType {
            base: RawBaseType::Void,
            pointer_depth: 1,
            constness: RawConstness::Mutable,
        };
        function.return_free_with = Some(native_cleanup.into());
        let projected = project_one(function);
        assert_eq!(projected.runtime.return_abi, Some(AbiType::Pointer));
        assert_eq!(projected.runtime.return_cleanup, cleanup);
        assert_eq!(projected.runtime.success_rule, SuccessRule::ReturnNonNull);
        assert_eq!(
            projected.return_shape,
            ReturnShape::Direct {
                typ: SurfaceType::Resource,
                conversion: Conversion::Resource,
                may_be_unavailable: true,
            }
        );
    }
    for cleanup in ["UnknownFree", "LocalFreeExtra", "CoTaskMemFreeCallback"] {
        let mut function = synthetic_function("UnknownAllocator");
        function.return_type = RawType {
            base: RawBaseType::Void,
            pointer_depth: 1,
            constness: RawConstness::Mutable,
        };
        function.return_free_with = Some(cleanup.into());
        let (generated, omitted) =
            generate_apis_files(&apis(vec![function]), "@test/runtime/win32");
        assert!(omitted[0].reason.contains("unsupported native cleanup"));
        assert!(!generated.js.contains("exports.unknownAllocator"));
    }
}

#[test]
fn mixed_out_inout_and_input_slots_keep_native_and_projected_result_order() {
    let mut function = synthetic_function("MixedSlots");
    function.return_type = scalar(RawScalar::Bool32);
    function.supports_last_error = true;
    function.parameters = vec![
        parameter(
            "first",
            pointer(scalar(RawScalar::I16), 1, RawConstness::Mutable),
            RawDirection::Out,
        ),
        parameter("input", scalar(RawScalar::U32), RawDirection::In),
        parameter(
            "cursor",
            pointer(scalar(RawScalar::U64), 1, RawConstness::Mutable),
            RawDirection::InOut,
        ),
        parameter(
            "enabled",
            pointer(scalar(RawScalar::Bool32), 1, RawConstness::Mutable),
            RawDirection::Out,
        ),
    ];
    let projected = project_one(function.clone());
    assert_eq!(
        projected
            .runtime
            .parameters
            .iter()
            .map(|p| p.direction)
            .collect::<Vec<_>>(),
        [
            Direction::Out,
            Direction::In,
            Direction::InOut,
            Direction::Out
        ]
    );
    assert_eq!(
        projected
            .runtime
            .parameters
            .iter()
            .map(|p| p.abi)
            .collect::<Vec<_>>(),
        [AbiType::I16, AbiType::U32, AbiType::U64, AbiType::Bool32]
    );
    assert_eq!(
        projected.inputs,
        vec![
            InputExpression::Surface {
                parameter_index: 0,
                conversion: Conversion::U32
            },
            InputExpression::Surface {
                parameter_index: 1,
                conversion: Conversion::U64
            },
        ]
    );
    assert_eq!(
        projected.return_shape,
        ReturnShape::Object {
            return_may_be_unavailable: false,
            status: false,
            return_value: Some((SurfaceType::Boolean, Conversion::Boolean)),
            last_error: true,
            outputs: vec![
                ProjectedOutput {
                    name: "first".into(),
                    output_index: 0,
                    typ: SurfaceType::Number,
                    conversion: Conversion::Number,
                    may_be_unavailable: false,
                },
                ProjectedOutput {
                    name: "cursor".into(),
                    output_index: 1,
                    typ: SurfaceType::BigInt,
                    conversion: Conversion::BigInt,
                    may_be_unavailable: false,
                },
                ProjectedOutput {
                    name: "enabled".into(),
                    output_index: 2,
                    typ: SurfaceType::Boolean,
                    conversion: Conversion::Boolean,
                    may_be_unavailable: false,
                },
            ],
        }
    );
    let (generated, _) = generate_apis_files(&apis(vec![function]), "@test/runtime/win32");
    run_js(
        &generated,
        r#"
const assert=require('node:assert/strict')
const calls=[]
let result=1
const runtime={DynWin32:{
  u32:value=>({kind:'u32',value}),u64:value=>({kind:'u64',value}),
  toNumber:Number,toBigint:BigInt,toBoolean:Boolean
},DynWin32Function:{bind(spec) {return {invoke(args) {
  calls.push({spec,args})
  let reads=0
  return {returnValue:result,lastError:123,get outputs(){assert.equal(++reads,1);return [-7,99n,1]}}
}}}}}
"#,
        r#"
for (const returned of [1,0]) {
  result=returned
  const actual=projected.mixedSlots(4,5n)
  assert.deepEqual(Object.keys(actual),['result','first','cursor','enabled','lastError'])
  assert.equal(actual.result,!!returned)
  assert.equal(actual.first,-7)
  assert.equal(actual.cursor,99n)
  assert.equal(actual.enabled,true)
  assert.equal(actual.lastError,123)
  assert.equal(calls.at(-1).args[0].value,4)
  assert.equal(calls.at(-1).args[1].value,5n)
}
"#,
    );
}

fn native_string(encoding: RawStringEncoding) -> RawType {
    RawType {
        base: RawBaseType::Named {
            namespace: "Tests.Strings".into(),
            name: "Text".into(),
            kind: RawNamedKind::StringPointer { encoding },
        },
        pointer_depth: 0,
        constness: RawConstness::Const,
    }
}

#[test]
fn string_encodings_multistrings_and_reserved_values_keep_separate_contracts() {
    let mut functions = Vec::new();
    for (name, encoding, projected_encoding, single, multiple) in [
        (
            "WideText",
            RawStringEncoding::Utf16,
            StringEncoding::Wide,
            Conversion::WideString,
            Conversion::WideMultiString,
        ),
        (
            "AnsiText",
            RawStringEncoding::Ansi,
            StringEncoding::Ansi,
            Conversion::AnsiString,
            Conversion::AnsiMultiString,
        ),
    ] {
        let mut function = synthetic_function(name);
        let mut one = parameter("text", native_string(encoding), RawDirection::In);
        one.nullable = true;
        let mut many = one.clone();
        many.name = "list".into();
        many.null_null_terminated = true;
        function.parameters = vec![one, many];
        let native = semantic(&function);
        assert_eq!(native.parameters[0].constness, Constness::Const);
        assert!(native.parameters[1].null_null_terminated);
        let projected = project_one(function.clone());
        assert_eq!(
            projected.parameters[0].typ,
            SurfaceType::String(projected_encoding)
        );
        assert_eq!(
            projected.parameters[1].typ,
            SurfaceType::MultiString(projected_encoding)
        );
        assert_eq!(
            projected.inputs,
            vec![
                InputExpression::Surface {
                    parameter_index: 0,
                    conversion: single
                },
                InputExpression::Surface {
                    parameter_index: 1,
                    conversion: multiple
                },
            ]
        );
        functions.push(function);
    }
    let mut reserved = synthetic_function("ReservedSlots");
    let mut flag = parameter("reservedBool", scalar(RawScalar::Bool32), RawDirection::In);
    flag.reserved = true;
    let mut pointer_slot = parameter(
        "reservedAddress",
        RawType {
            base: RawBaseType::Named {
                namespace: "Tests".into(),
                name: "Callback".into(),
                kind: RawNamedKind::FunctionPointer,
            },
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        },
        RawDirection::In,
    );
    pointer_slot.reserved = true;
    let mut real = parameter("reservedDouble", scalar(RawScalar::F64), RawDirection::In);
    real.reserved = true;
    reserved.parameters = vec![
        flag,
        parameter("value", scalar(RawScalar::U32), RawDirection::In),
        pointer_slot,
        real,
    ];
    let projected = project_one(reserved.clone());
    assert_eq!(projected.parameters.len(), 1);
    assert_eq!(projected.parameters[0].name, "value");
    assert_eq!(
        projected.inputs,
        vec![
            InputExpression::Zero(AbiType::Bool32),
            InputExpression::Surface {
                parameter_index: 0,
                conversion: Conversion::U32
            },
            InputExpression::NullPointer,
            InputExpression::Zero(AbiType::F64),
        ]
    );
    functions.push(reserved);
    let (generated, omitted) = generate_apis_files(&apis(functions), "@test/runtime/win32");
    assert!(omitted.is_empty());
    run_js(
        &generated,
        r#"
const assert=require('node:assert/strict')
const calls=[]
const wrap=kind=>(value,nullable)=>({kind,value,nullable})
const runtime={DynWin32:{
  wideString:wrap('wide'),ansiString:wrap('ansi'),wideMultiString:wrap('wide-list'),ansiMultiString:wrap('ansi-list'),
  bool32:wrap('bool32'),u32:wrap('u32'),f64:wrap('f64'),nullPointer:()=>null,toNumber:Number
},DynWin32Function:{bind(spec){return {invoke(args){calls.push({spec,args});return {returnValue:0,outputs:[]}}}}}}
"#,
        r#"
for (const [name,kind] of [['wideText','wide'],['ansiText','ansi']]) {
  projected[name](null,['one','two'])
  const args=calls.at(-1).args
  assert.equal(args[0].kind,kind)
  assert.equal(args[0].value,null)
  assert.equal(args[0].nullable,true)
  assert.equal(args[1].kind,kind+'-list')
  assert.deepEqual(args[1].value,['one','two'])
}
projected.reservedSlots(9)
const args=calls.at(-1).args
assert.equal(args.length,4)
assert.equal(args[0].value,false)
assert.equal(args[1].value,9)
assert.equal(args[2],null)
assert.equal(args[3].value,0)
"#,
    );
}

#[test]
fn unsupported_strings_callbacks_com_outputs_and_architectures_fail_before_render() {
    let mut invalid = synthetic_function("Unsupported");
    let string = parameter(
        "text",
        native_string(RawStringEncoding::Utf16),
        RawDirection::Out,
    );
    let callback = parameter(
        "callback",
        RawType {
            base: RawBaseType::Named {
                namespace: "Tests".into(),
                name: "Callback".into(),
                kind: RawNamedKind::FunctionPointer,
            },
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        },
        RawDirection::In,
    );
    let com = parameter(
        "object",
        RawType {
            base: RawBaseType::Named {
                namespace: "Tests".into(),
                name: "IObject".into(),
                kind: RawNamedKind::ComInterface {
                    iid: "00000000-0000-0000-c000-000000000046".into(),
                },
            },
            pointer_depth: 1,
            constness: RawConstness::Mutable,
        },
        RawDirection::Out,
    );
    for (parameter, reason) in [
        (string, "size relationship"),
        (callback, "callback"),
        (com, "COM interface"),
    ] {
        invalid.parameters = vec![parameter];
        let (generated, omitted) =
            generate_apis_files(&apis(vec![invalid.clone()]), "@test/runtime/win32");
        assert_eq!(omitted.len(), 1);
        assert!(
            omitted[0]
                .reason
                .to_ascii_lowercase()
                .contains(&reason.to_ascii_lowercase()),
            "{omitted:?}"
        );
        assert!(!generated.js.contains("exports.unsupported"));
    }
    for convention in [RawCallingConvention::Unsupported] {
        invalid.parameters.clear();
        invalid.calling_convention = convention;
        assert!(
            validate_function(&invalid)
                .unwrap_err()
                .contains("calling convention")
        );
    }
    invalid.calling_convention = RawCallingConvention::System;
    invalid.architectures.arm64 = false;
    assert!(validate_function(&invalid).unwrap_err().contains("ARM64"));
}
