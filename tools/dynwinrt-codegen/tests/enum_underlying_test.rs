// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod common;

use dynwinrt_codegen::codegen::{project, python, python_stub, render_dts, render_js};
use dynwinrt_codegen::meta::{self, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::{EnumMember, FieldMeta, TypeMeta};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::process::{Command, Stdio};

fn run_node(source: &str) {
    let mut child = Command::new("node")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Node is required for generated enum regression tests");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );
}

fn enumeration(unsigned: bool) -> TypeMeta {
    TypeMeta::Enum {
        namespace: "Tests.EnumContracts".into(),
        name: "Options".into(),
        underlying: Box::new(if unsigned {
            TypeMeta::U32
        } else {
            TypeMeta::I32
        }),
        members: vec![
            EnumMember {
                name: "High".into(),
                value: i32::MIN,
                doc: None,
            },
            EnumMember {
                name: "All".into(),
                value: -1,
                doc: None,
            },
        ],
        is_flags: unsigned,
        doc: None,
        deprecated: None,
    }
}

fn parameter(name: &str, typ: TypeMeta, direction: ParamDirection) -> ParamMeta {
    ParamMeta {
        name: name.into(),
        typ,
        direction,
    }
}

fn render(interface: &InterfaceMeta) -> (String, String) {
    let known = HashSet::from([
        "Options".into(),
        "FlagRecord".into(),
        interface.name.clone(),
    ]);
    let projected = project::project_interface(
        &Default::default(),
        interface,
        &known,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    (
        render_js::render(&projected),
        common::generate_interface(interface, &known, &HashSet::new()),
    )
}

#[test]
fn sdk_enum_metadata_retains_signed_and_unsigned_backing_types() {
    let winmd = std::env::var("DYNWINRT_WINDOWS_WINMD")
        .ok()
        .into_iter()
        .chain(dynwinrt_codegen::com_metadata::discover_newest_windows_winmd())
        .find(|candidate| {
            meta::parse_enums(candidate, "Windows.Gaming.Input")
                .iter()
                .any(|typ| matches!(typ, TypeMeta::Enum { name, .. } if name == "GamepadButtons"))
        });
    let Some(winmd) = winmd else {
        eprintln!("Skipping: complete Windows SDK metadata is unavailable");
        return;
    };
    for (namespace, name, expected, flags) in [
        (
            "Windows.Gaming.Input",
            "GamepadButtons",
            TypeMeta::U32,
            true,
        ),
        (
            "Windows.Foundation.Metadata",
            "AttributeTargets",
            TypeMeta::U32,
            true,
        ),
        (
            "Windows.UI.Core",
            "CoreDispatcherPriority",
            TypeMeta::I32,
            false,
        ),
    ] {
        let enumeration = meta::parse_enums(&winmd, namespace)
            .into_iter()
            .find(|typ| matches!(typ, TypeMeta::Enum { name: found, .. } if found == name))
            .unwrap_or_else(|| panic!("SDK metadata must contain {namespace}.{name}"));
        let TypeMeta::Enum {
            underlying,
            is_flags,
            members,
            ..
        } = enumeration
        else {
            unreachable!()
        };
        assert_eq!(*underlying, expected, "{namespace}.{name}");
        assert_eq!(is_flags, flags, "{namespace}.{name}");
        if name == "AttributeTargets" {
            let all = members.iter().find(|member| member.name == "All").unwrap();
            assert_eq!(all.numeric_value(&underlying), 0xffff_ffff);
        } else if name == "CoreDispatcherPriority" {
            let low = members.iter().find(|member| member.name == "Low").unwrap();
            assert_eq!(low.numeric_value(&underlying), -1);
        }
    }
}

#[test]
fn enum_constants_preserve_high_bits_in_javascript_python_and_declarations() {
    for unsigned in [false, true] {
        let enumeration = enumeration(unsigned);
        let file = project::project_enum(&enumeration).unwrap();
        let context =
            python::PythonProjectionContext::packaged([enumeration.type_identity()]).unwrap();
        for source in [
            render_js::render(&file),
            render_dts::render(&file),
            python::generate_enum(&context, &enumeration).unwrap(),
            python_stub::generate_enum_stub(&context, &enumeration).unwrap(),
        ] {
            for expected in if unsigned {
                ["2147483648", "4294967295"]
            } else {
                ["-2147483648", "-1"]
            } {
                assert!(source.contains(expected), "{source}");
            }
            if unsigned {
                assert!(!source.contains("-2147483648"), "{source}");
            }
        }
    }
}

#[test]
fn enum_projection_preserves_backing_type_in_properties_arrays_structs_and_callbacks() {
    for unsigned in [false, true] {
        let enumeration = enumeration(unsigned);
        let array = TypeMeta::Array(Box::new(enumeration.clone()));
        let record = TypeMeta::Struct {
            namespace: "Tests.EnumContracts".into(),
            name: "FlagRecord".into(),
            fields: vec![FieldMeta {
                name: "Bits".into(),
                typ: enumeration.clone(),
            }],
        };
        let interface = InterfaceMeta {
            namespace: "Tests.EnumContracts".into(),
            name: "IProbe".into(),
            iid: "ecbd857b-c237-4dc2-bb9b-915f89090d04".into(),
            methods: vec![
                MethodMeta {
                    name: "get_Flags".into(),
                    raw_name: "get_Flags".into(),
                    vtable_index: 6,
                    is_property_getter: true,
                    return_type: Some(enumeration.clone()),
                    ..Default::default()
                },
                MethodMeta {
                    name: "put_Flags".into(),
                    raw_name: "put_Flags".into(),
                    vtable_index: 7,
                    is_property_setter: true,
                    params: vec![parameter("value", enumeration, ParamDirection::In)],
                    ..Default::default()
                },
                MethodMeta {
                    name: "EchoArray".into(),
                    raw_name: "EchoArray".into(),
                    vtable_index: 8,
                    params: vec![parameter("values", array.clone(), ParamDirection::In)],
                    return_type: Some(array),
                    ..Default::default()
                },
                MethodMeta {
                    name: "EchoRecord".into(),
                    raw_name: "EchoRecord".into(),
                    vtable_index: 9,
                    params: vec![parameter("value", record.clone(), ParamDirection::In)],
                    return_type: Some(record),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let (js, py) = render(&interface);
        if unsigned {
            for expected in [
                "[2147483648, 4294967295], DynWinRtType.u32())",
                "DynWinRtValue.u32(_flagsU32(value))",
                "s.getU32(0)",
                "s.setU32(0, _flagsU32(",
                "fromU32Values(values.map(item => _flagsU32(item)))",
                ".toU32Vec()",
                "_m.setU32(this._obj, _flagsU32(value))",
                "DynWinRtValue.enumValue(",
                "_flagsU32(((v) => __implementationCheck(",
                "v >= -2147483648 && v <= 4294967295",
                "v <= 4294967295",
            ] {
                assert!(js.contains(expected), "{expected}: {js}");
            }
            assert_eq!(js.matches("const _flagsU32 =").count(), 1, "{js}");
            assert!(!js.contains("_m.getI32("), "{js}");
            for expected in [
                "[2147483648, 4294967295], DynWinRTType.u32_type())",
                "s.get_u32(0)",
                "s.set_u32(0,",
                ".to_u32_list()",
                "v <= 4294967295",
            ] {
                assert!(py.contains(expected), "{expected}: {py}");
            }
        } else {
            assert!(!js.contains("_flagsU32"), "{js}");
            assert!(js.contains("[-2147483648, -1])"), "{js}");
            assert!(js.contains("_m.getI32("), "{js}");
            assert!(js.contains(".toI32Vec()"), "{js}");
            assert!(py.contains("[-2147483648, -1])"), "{py}");
            assert!(py.contains(".to_i32_list()"), "{py}");
        }
    }
}

#[test]
fn unsigned_enum_vector_helpers_use_unsigned_batch_storage() {
    let enumeration = enumeration(true);
    let array = TypeMeta::Array(Box::new(enumeration.clone()));
    let interface = InterfaceMeta {
        namespace: "Windows.Foundation.Collections".into(),
        name: "IVector_Options".into(),
        generic_name: Some("IVector".into()),
        generic_piid: Some(meta::PIID_IVECTOR.into()),
        generic_args: vec![enumeration],
        methods: vec![
            MethodMeta {
                name: "GetMany".into(),
                raw_name: "GetMany".into(),
                vtable_index: 16,
                params: vec![
                    parameter("startIndex", TypeMeta::U32, ParamDirection::In),
                    parameter("items", array.clone(), ParamDirection::OutFill),
                ],
                return_type: Some(TypeMeta::U32),
                ..Default::default()
            },
            MethodMeta {
                name: "ReplaceAll".into(),
                raw_name: "ReplaceAll".into(),
                vtable_index: 17,
                params: vec![parameter("items", array, ParamDirection::In)],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let (js, _) = render(&interface);
    assert!(
        js.contains("DynWinRtArray.fromU32Values(new Array(count).fill(0))"),
        "{js}"
    );
    assert!(
        js.contains("DynWinRtArray.fromU32Values(items.map(item => _flagsU32(item)))"),
        "{js}"
    );
    assert!(js.contains(".toU32Vec()"), "{js}");
    assert!(js.contains("DynWinRtValue.u32(_flagsU32(i))"), "{js}");
}

#[test]
fn only_uint32_flags_receive_signed_bit_pattern_normalization() {
    let mut unsigned_non_flags = enumeration(true);
    let TypeMeta::Enum { is_flags, .. } = &mut unsigned_non_flags else {
        unreachable!()
    };
    *is_flags = false;
    let interface = InterfaceMeta {
        namespace: "Tests.EnumContracts".into(),
        name: "INonFlags".into(),
        iid: "4e92ff20-69cf-4860-b729-76635431fbc4".into(),
        methods: vec![MethodMeta {
            name: "put_Value".into(),
            raw_name: "put_Value".into(),
            vtable_index: 6,
            is_property_setter: true,
            params: vec![parameter("value", unsigned_non_flags, ParamDirection::In)],
            ..Default::default()
        }],
        ..Default::default()
    };
    let (js, _) = render(&interface);
    assert!(js.contains("_m.setU32(this._obj, value)"), "{js}");
    assert!(js.contains("DynWinRtValue.u32(value)"), "{js}");
    assert!(!js.contains("_flagsU32"), "{js}");
}

#[test]
fn flags_normalization_covers_references_vectors_and_maps() {
    let flags = enumeration(true);
    let reference = TypeMeta::Parameterized {
        namespace: "Windows.Foundation".into(),
        name: "IReference`1".into(),
        piid: "61c17706-2d65-11e0-9ae8-d48564015472".into(),
        args: vec![flags.clone()],
    };
    let vector = TypeMeta::Parameterized {
        namespace: "Windows.Foundation.Collections".into(),
        name: "IVector`1".into(),
        piid: meta::PIID_IVECTOR.into(),
        args: vec![flags.clone()],
    };
    let map = TypeMeta::Parameterized {
        namespace: "Windows.Foundation.Collections".into(),
        name: "IMap`2".into(),
        piid: "3c2925fe-8519-45c1-aa79-197b6718c1c1".into(),
        args: vec![flags.clone(), flags],
    };
    let interface = InterfaceMeta {
        namespace: "Tests.EnumContracts".into(),
        name: "IContainers".into(),
        iid: "c067186d-3400-403f-a116-bff5f10aad68".into(),
        methods: vec![
            MethodMeta {
                name: "TakeReference".into(),
                raw_name: "TakeReference".into(),
                vtable_index: 6,
                params: vec![parameter("value", reference, ParamDirection::In)],
                ..Default::default()
            },
            MethodMeta {
                name: "TakeVector".into(),
                raw_name: "TakeVector".into(),
                vtable_index: 7,
                params: vec![parameter("value", vector, ParamDirection::In)],
                ..Default::default()
            },
            MethodMeta {
                name: "TakeMap".into(),
                raw_name: "TakeMap".into(),
                vtable_index: 8,
                params: vec![parameter("value", map, ParamDirection::In)],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let (js, _) = render(&interface);
    assert!(js.contains("DynWinRtValue.enumValue("), "{js}");
    assert!(js.contains("_flagsU32(value)"), "{js}");
    assert!(
        js.contains(".map(_i => DynWinRtValue.u32(_flagsU32(_i)))"),
        "{js}"
    );
    assert!(
        js.contains(".map(_k => DynWinRtValue.u32(_flagsU32(_k)))"),
        "{js}"
    );
    assert!(
        js.contains(".map(_v => DynWinRtValue.u32(_flagsU32(_v)))"),
        "{js}"
    );
    assert_eq!(js.matches("const _flagsU32 =").count(), 1, "{js}");
}

#[test]
fn flags_helper_avoids_type_parameter_and_suffixed_name_collisions() {
    for type_name in ["_flagsU32", "IFlagsProbe"] {
        let interface = parse_flags_collision_fixture(type_name);
        let (js, _) = render(&interface);
        assert!(js.contains("const _flagsU32_3 ="), "{js}");
        run_node(&format!(
            r#"
const assert = require('node:assert/strict');
const vm = require('node:vm');
let calls = 0;
let received;
const signature = {{ addIn() {{ return this; }} }};
const iface = {{
  addMethod() {{ return this; }},
  method() {{ return {{ invoke(_obj, args) {{ calls++; received = args; }} }}; }}
}};
const runtime = {{
  WinGuid: {{ parse: value => value }},
  DynWinRtType: {{ u32() {{}}, enumType() {{}}, registerInterface() {{ return iface; }} }},
  DynWinRtMethodSig: function() {{ return signature; }},
  DynWinRtValue: {{ u32(value) {{
    assert.ok(Number.isInteger(value) && value >= 0 && value <= 0xffffffff);
    return value;
  }} }}
}};
const exports = {{}};
vm.runInNewContext({}, {{ exports, require() {{ return runtime; }} }});
const obj = new exports[{}]({{}});
for (const value of [1, -2147483647, 2147483649, -1]) {{
  obj.take(value, value, value);
  assert.deepEqual(Array.from(received), [value >>> 0, value >>> 0, value >>> 0]);
}}
for (const value of [-2147483649, 4294967296, 1.5, NaN, Infinity, '1', null, undefined]) {{
  const before = calls;
  assert.throws(() => obj.take(value, 1, 1));
  assert.equal(calls, before);
}}
"#,
            serde_json::to_string(&js).unwrap(),
            serde_json::to_string(type_name).unwrap(),
        ));
    }
}

fn parse_flags_collision_fixture(name: &str) -> InterfaceMeta {
    use windows_metadata::{
        FieldAttributes, MethodAttributes, MethodCallAttributes, MethodImplAttributes,
        ParamAttributes, Signature, Type, TypeAttributes, Value, writer,
    };
    const NS: &str = "Tests.EnumContracts";
    let mut file = writer::File::new("FlagsNames");
    let enum_base = file.TypeRef("System", "Enum");
    let enumeration = file.TypeDef(
        NS,
        "Options",
        writer::TypeDefOrRef::TypeRef(enum_base),
        TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
    );
    file.Field(
        "value__",
        &Type::U32,
        FieldAttributes::Public | FieldAttributes::SpecialName | FieldAttributes::RTSpecialName,
    );
    let flags = file.TypeRef("System", "FlagsAttribute");
    let ctor = file.MemberRef(
        ".ctor",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::Void,
            types: vec![],
        },
        writer::MemberRefParent::TypeRef(flags),
    );
    file.Attribute(
        writer::HasAttribute::TypeDef(enumeration),
        writer::AttributeType::MemberRef(ctor),
        &[],
    );
    let interface = file.TypeDef(
        NS,
        name,
        writer::TypeDefOrRef::default(),
        TypeAttributes::Public
            | TypeAttributes::Abstract
            | TypeAttributes::Interface
            | TypeAttributes::WindowsRuntime,
    );
    let guid = file.TypeRef("Windows.Foundation.Metadata", "GuidAttribute");
    let ctor = file.MemberRef(
        ".ctor",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::Void,
            types: vec![
                Type::U32,
                Type::U16,
                Type::U16,
                Type::U8,
                Type::U8,
                Type::U8,
                Type::U8,
                Type::U8,
                Type::U8,
                Type::U8,
                Type::U8,
            ],
        },
        writer::MemberRefParent::TypeRef(guid),
    );
    let values = [
        Value::U32(0x37d06f6d),
        Value::U16(0x734a),
        Value::U16(0x4192),
        Value::U8(0x81),
        Value::U8(0x90),
        Value::U8(0xbc),
        Value::U8(0xdf),
        Value::U8(0x83),
        Value::U8(0x51),
        Value::U8(0x72),
        Value::U8(0xf0),
    ];
    file.Attribute(
        writer::HasAttribute::TypeDef(interface),
        writer::AttributeType::MemberRef(ctor),
        &values
            .into_iter()
            .map(|value| (String::new(), value))
            .collect::<Vec<_>>(),
    );
    file.MethodDef(
        "Take",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::Void,
            types: vec![Type::named(NS, "Options"); 3],
        },
        MethodAttributes::Public
            | MethodAttributes::Virtual
            | MethodAttributes::Abstract
            | MethodAttributes::NewSlot,
        MethodImplAttributes::default(),
    );
    for (index, name) in ["_flagsU32", "_flagsU32_1", "_flagsU32_2"]
        .iter()
        .enumerate()
    {
        file.Param(name, (index + 1) as u16, ParamAttributes::In);
    }
    let path = std::env::temp_dir().join(format!(
        "dynwinrt-flags-names-{}-{name}.winmd",
        std::process::id()
    ));
    std::fs::write(&path, file.into_stream()).unwrap();
    let parsed = meta::parse_interfaces(path.to_str().unwrap(), NS);
    std::fs::remove_file(path).unwrap();
    let interface = parsed
        .into_iter()
        .find(|interface| interface.name == name)
        .unwrap();
    assert!(matches!(&interface.methods[0].params[0].typ,
        TypeMeta::Enum { underlying, is_flags: true, .. } if underlying.as_ref() == &TypeMeta::U32));
    interface
}
