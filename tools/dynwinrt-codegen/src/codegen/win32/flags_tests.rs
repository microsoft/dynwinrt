// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::{ir::*, test_support::*, *};
use crate::win32_metadata::*;

fn enum_type(name: &str, underlying: RawScalar, is_flags: bool, high: i128) -> RawType {
    RawType {
        base: RawBaseType::Named {
            namespace: "Tests.Enums".into(),
            name: name.into(),
            kind: RawNamedKind::Enum {
                underlying,
                is_flags,
                members: vec![
                    RawEnumMember {
                        name: "FIRST".into(),
                        value: 1,
                    },
                    RawEnumMember {
                        name: "SECOND".into(),
                        value: 2,
                    },
                    RawEnumMember {
                        name: "HIGH".into(),
                        value: high,
                    },
                ],
            },
        },
        pointer_depth: 0,
        constness: RawConstness::Unspecified,
    }
}

#[test]
fn flags_allow_combinations_without_widening_ordinary_enum_members() {
    for (underlying, high, abi, conversion) in [
        (RawScalar::I8, -128, AbiType::I8, Conversion::I8),
        (RawScalar::U8, 128, AbiType::U8, Conversion::U8),
        (RawScalar::I16, -32768, AbiType::I16, Conversion::I16),
        (RawScalar::U16, 32768, AbiType::U16, Conversion::U16),
        (RawScalar::I32, -2147483648, AbiType::I32, Conversion::I32),
        (
            RawScalar::U32,
            2147483648,
            AbiType::U32,
            Conversion::U32Flags,
        ),
    ] {
        let mut function = synthetic_function("UseEnums");
        function.parameters = vec![
            parameter(
                "flags",
                enum_type("MODE_FLAGS", underlying, true, high),
                RawDirection::In,
            ),
            parameter(
                "mode",
                enum_type("MODE", underlying, false, high),
                RawDirection::In,
            ),
        ];
        let native = semantic(&function);
        assert!(
            native
                .enums
                .iter()
                .find(|e| e.name == "MODE_FLAGS")
                .unwrap()
                .is_flags
        );
        assert!(
            !native
                .enums
                .iter()
                .find(|e| e.name == "MODE")
                .unwrap()
                .is_flags
        );
        let projected = project_one(function.clone());
        assert_eq!(projected.runtime.parameters[0].abi, abi);
        assert_eq!(projected.runtime.parameters[1].abi, abi);
        assert_eq!(
            projected.parameters[0].typ,
            SurfaceType::Enum("MODE_FLAGS".into())
        );
        assert_eq!(
            projected.inputs[0],
            InputExpression::Surface {
                parameter_index: 0,
                conversion
            }
        );
        if abi == AbiType::U32 {
            assert_eq!(
                projected.inputs[1],
                InputExpression::Surface {
                    parameter_index: 1,
                    conversion: Conversion::U32
                }
            );
        }
        let (generated, omitted) =
            generate_apis_files(&apis(vec![function]), "@test/runtime/win32");
        assert!(omitted.is_empty());
        let file = |name: &str| {
            generated
                .extra_files
                .iter()
                .find(|(file, _)| file == name)
                .unwrap()
                .1
                .as_str()
        };
        assert!(file("MODE_FLAGS.d.ts").contains("export type MODE_FLAGS = number\n"));
        assert!(
            file("MODE.d.ts").contains("export type MODE = (typeof MODE)[keyof typeof MODE]\n")
        );
        for name in ["MODE_FLAGS.d.ts", "MODE.d.ts"] {
            assert!(file(name).contains(&format!("readonly HIGH: {high}\n")));
        }
        assert_eq!(
            generated.js.contains("function _u32Flags("),
            abi == AbiType::U32
        );
    }
}

#[test]
fn unsigned_flags_preserve_signed_javascript_bitwise_results_without_silent_truncation() {
    let mut function = synthetic_function("UseEnums");
    function.parameters = vec![
        parameter(
            "flags",
            enum_type("MODE_FLAGS", RawScalar::U32, true, 0x8000_0000),
            RawDirection::In,
        ),
        parameter(
            "mode",
            enum_type("MODE", RawScalar::U32, false, 0x8000_0000),
            RawDirection::In,
        ),
    ];
    let (generated, omitted) = generate_apis_files(&apis(vec![function]), "@test/runtime/win32");
    assert!(omitted.is_empty());
    let flags_js = generated
        .extra_files
        .iter()
        .find(|(name, _)| name == "MODE_FLAGS.js")
        .unwrap();
    run_js(
        &generated,
        &format!(
            r#"
const assert=require('node:assert/strict')
const constants={{}}
require('node:vm').runInNewContext({},{{exports:constants}})
const flags=constants.MODE_FLAGS
let calls=0
let received
const runtime={{DynWin32:{{
  u32(value) {{
    if(!Number.isInteger(value) || value<0 || value>0xffffffff) throw new RangeError('native u32')
    return value
  }},
  toNumber:Number
}},DynWin32Function:{{bind(spec){{
  assert.equal(spec.parameters[0].type,'u32')
  assert.equal(spec.parameters[1].type,'u32')
  assert.equal(JSON.parse(spec.callContractDescriptor).version,2)
  return {{invoke(args){{calls++;received=args;return {{returnValue:9,outputs:[]}}}}}}
}}}}}}
"#,
            serde_json::to_string(&flags_js.1).unwrap()
        ),
        r#"
for(const [value, expected] of [
  [flags.FIRST | flags.SECOND, 3],
  [flags.HIGH | flags.FIRST, 0x80000001],
  [flags.HIGH | flags.SECOND, 0x80000002],
  [flags.HIGH, 0x80000000],
  [-1, 0xffffffff],
  [0xffffffff, 0xffffffff],
  [0, 0],
]) {
  assert.equal(projected.useEnums(value,1),9)
  assert.equal(received[0],expected)
  assert.equal(received[1],1)
}
for(const invalid of [-0x80000001,0x100000000,1.5,NaN,Infinity,-Infinity,'1',true,null,undefined,1n,Symbol('flags'),{}]) {
  const before=calls
  assert.throws(()=>projected.useEnums(invalid,1),/native 32-bit integer/)
  assert.equal(calls,before)
}
const before=calls
assert.throws(()=>projected.useEnums(1,flags.HIGH|flags.FIRST),/native u32/)
assert.equal(calls,before)
"#,
    );
}
