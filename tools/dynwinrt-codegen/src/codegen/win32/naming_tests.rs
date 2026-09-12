// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::{ir::*, test_support::*, *};
use crate::win32_metadata::*;
use std::collections::BTreeSet;

#[test]
fn strict_bindings_and_generated_locals_have_unique_projected_parameter_names() {
    let raw_names = [
        "eval",
        "arguments",
        "eval_",
        "arguments_",
        "lpValue",
        "value",
        "_call",
        "_return",
        "_outputs",
        "_subsystem",
        "_nativeAggregate",
        "_bufferCount",
        "_scalarPointer",
        "_u32Flags",
        "_bindCollisionValuesPlan",
        "undefined",
        "pClass",
        "class",
    ];
    let mut function = synthetic_function("CollisionValues");
    function.parameters = raw_names
        .iter()
        .map(|name| parameter(name, scalar(RawScalar::U32), RawDirection::In))
        .collect();
    let projected = project_one(function.clone());
    assert_eq!(projected.js_name, "collisionValues");
    assert_eq!(projected.parameters.len(), raw_names.len());
    let names = projected
        .parameters
        .iter()
        .map(|p| p.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names.iter().copied().collect::<BTreeSet<_>>().len(),
        names.len()
    );
    assert_eq!(
        &names[..6],
        &[
            "eval_",
            "arguments_",
            "eval__",
            "arguments__",
            "value",
            "value_"
        ]
    );
    for original in &raw_names[6..] {
        assert!(!names.contains(original), "{original}");
    }
    for (index, input) in projected.inputs.iter().enumerate() {
        assert_eq!(
            *input,
            InputExpression::Surface {
                parameter_index: index,
                conversion: Conversion::U32,
            }
        );
    }
    let (generated, omitted) = generate_apis_files(&apis(vec![function]), "@test/runtime/win32");
    assert!(omitted.is_empty());
    for name in &names {
        assert!(generated.dts.contains(&format!("{name}: number")));
    }
    run_js(
        &generated,
        r#"
const assert=require('node:assert/strict')
let received
const runtime={DynWin32:{u32:value=>value,toNumber:Number},DynWin32Function:{bind(){
  return {invoke(args){received=args;return {returnValue:42,outputs:[]}}}
}}}
"#,
        &format!(
            r#"
const supplied=Array.from({{length:{}}},(_,index)=>index+1)
assert.equal(projected.collisionValues(...supplied),42)
assert.deepEqual(Array.from(received),supplied)
assert.equal(typeof projected.Apis.collisionValues,'function')
"#,
            raw_names.len()
        ),
    );
}

#[test]
fn retired_policy_helpers_no_longer_reserve_native_parameter_names() {
    let raw_names = [
        "_borrowedHkeyOutput",
        "_performanceDataCount0",
        "_hkeyBits",
        "_isPredefinedHkey",
        "_emptyNativeString",
        "_bindNoPoliciesPlanBorrowed",
    ];
    let mut function = synthetic_function("NoPolicies");
    function.parameters = raw_names
        .iter()
        .map(|name| parameter(name, scalar(RawScalar::U32), RawDirection::In))
        .collect();
    let projected = project_one(function);
    assert_eq!(
        projected
            .parameters
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        raw_names
    );
    assert!(projected.runtime.call_contract.is_empty());
}

#[test]
fn normalized_output_names_do_not_overwrite_other_native_results() {
    let mut function = synthetic_function("CollidingOutputs");
    function.parameters = ["lpValue", "value", "lpResult", "result", "value_"]
        .into_iter()
        .map(|name| {
            parameter(
                name,
                pointer(scalar(RawScalar::U32), 1, RawConstness::Mutable),
                RawDirection::Out,
            )
        })
        .collect();
    let projected = project_one(function.clone());
    let ReturnShape::Object { outputs, .. } = &projected.return_shape else {
        panic!("outputs")
    };
    assert_eq!(
        outputs.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(),
        ["value", "value_", "value__", "value___", "value____"]
    );
    assert_eq!(
        outputs.iter().map(|o| o.output_index).collect::<Vec<_>>(),
        [0, 1, 2, 3, 4]
    );
    let (generated, omitted) = generate_apis_files(&apis(vec![function]), "@test/runtime/win32");
    assert!(omitted.is_empty());
    run_js(
        &generated,
        r#"
const assert=require('node:assert/strict')
const runtime={DynWin32:{toNumber:Number},DynWin32Function:{bind(){
  return {invoke(){return {returnValue:9,outputs:[1,2,3,4,5]}}}
}}}
"#,
        r#"
const result=projected.collidingOutputs()
assert.deepEqual(Object.keys(result),['result','value','value_','value__','value___','value____'])
assert.deepEqual(Object.values(result),[9,1,2,3,4,5])
"#,
    );
}

#[test]
fn layout_constants_are_not_shadowed_by_projected_inputs() {
    let shape = RawNativeLayoutSet {
        recursive: false,
        evidence: None,
        variants: vec![RawNativeLayout {
            architectures: RawArchitectures {
                x86: true,
                x64: true,
                arm64: true,
            },
            kind: RawLayoutKind::Sequential,
            packing: RawPacking::Default,
            declared_size: None,
            forced_alignment: None,
            fields: vec![RawNativeField {
                name: "x".into(),
                typ: scalar(RawScalar::I32),
                fixed_count: None,
                bitfield: false,
                flexible_array: false,
            }],
        }],
    };
    let mut function = synthetic_function("LayoutCollision");
    function.parameters = vec![
        parameter(
            "_nativeLayout_POINT",
            scalar(RawScalar::U32),
            RawDirection::In,
        ),
        parameter(
            "point",
            RawType {
                base: RawBaseType::Named {
                    namespace: "Tests".into(),
                    name: "POINT".into(),
                    kind: RawNamedKind::NativeStruct {
                        layout: Box::new(shape),
                    },
                },
                pointer_depth: 1,
                constness: RawConstness::Const,
            },
            RawDirection::In,
        ),
    ];
    let projected = project_one(function.clone());
    assert_eq!(projected.parameters[0].name, "_nativeLayout_POINT_");
    let (generated, omitted) = generate_apis_files(&apis(vec![function]), "@test/runtime/win32");
    assert!(omitted.is_empty());
    run_js(
        &generated,
        r#"
const assert=require('node:assert/strict')
let received
const runtime={DynWin32:{
  u32:value=>value,toNumber:Number,
  nativeStruct(value,descriptor){assert.equal(JSON.parse(descriptor).name,'Tests.POINT');return value}
},DynWin32Function:{bind(){
  return {invoke(args){received=args;return {returnValue:0,outputs:[]}}}
}}}
"#,
        r#"
const point={}
projected.layoutCollision(123,point)
assert.equal(received[0],123)
assert.equal(received[1],point)
"#,
    );
}

#[test]
fn real_raise_exception_module_is_strict_mode_safe_without_invoking_native_code() {
    let Some(path) = metadata() else { return };
    let raw = metadata_function(
        &path,
        "Windows.Win32.System.Diagnostics.Debug",
        "RaiseException",
    );
    let projected = project_one(raw.clone());
    assert_eq!(projected.metadata_name, "RaiseException");
    assert_eq!(projected.js_name, "raiseException");
    assert_eq!(
        projected
            .parameters
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["dwExceptionCode", "dwExceptionFlags", "arguments_"]
    );
    let (generated, omitted) = generate_apis_files(&apis(vec![raw]), "@test/runtime/win32");
    assert!(omitted.is_empty());
    run_js(
        &generated,
        r#"
const assert=require('node:assert/strict')
const runtime={DynWin32:{},DynWin32Function:{bind(){throw Error('native bind during import')}}}
"#,
        r#"
assert.equal(typeof projected.raiseException,'function')
assert.equal(projected.Apis.raiseException,projected.raiseException)
assert(Object.isFrozen(projected.Apis))
"#,
    );
}
