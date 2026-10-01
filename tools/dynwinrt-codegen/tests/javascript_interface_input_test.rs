// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::process::Command;

use dynwinrt_codegen::codegen::projected::ProjectedMember;
use dynwinrt_codegen::codegen::{project, render_dts, render_js};
use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::TypeMeta;

fn method(name: &str, raw_name: &str, slot: usize, inputs: &[&str]) -> MethodMeta {
    MethodMeta {
        name: name.into(),
        raw_name: raw_name.into(),
        vtable_index: slot,
        params: inputs
            .iter()
            .map(|name| ParamMeta {
                name: (*name).into(),
                typ: TypeMeta::I32,
                direction: ParamDirection::In,
            })
            .collect(),
        return_type: Some(TypeMeta::I32),
        ..Default::default()
    }
}

fn reader() -> ClassMeta {
    ClassMeta {
        name: "Reader".into(),
        namespace: "Tests".into(),
        full_name: "Tests.Reader".into(),
        default_interface: Some(InterfaceMeta {
            name: "IReader".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
            methods: vec![
                method("ReadShort", "Read", 6, &["value"]),
                method("Read", "Read", 7, &["value", "mode"]),
                method("Fetch", "Fetch", 8, &[]),
            ],
            ..Default::default()
        }),
        required_interfaces: vec![InterfaceMeta {
            name: "IExtra".into(),
            iid: "22222222-2222-2222-2222-222222222222".into(),
            methods: vec![
                method("WriteShort", "Write", 6, &["value"]),
                method("Fetch2", "Fetch2", 7, &["value"]),
            ],
            ..Default::default()
        }],
        static_interfaces: vec![InterfaceMeta {
            name: "IReaderStatics".into(),
            iid: "33333333-3333-3333-3333-333333333333".into(),
            methods: vec![method("MakeShort", "Make", 6, &["value"])],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn project_reader(
    class: &ClassMeta,
    imported: bool,
) -> dynwinrt_codegen::codegen::projected::ProjectedFile {
    let shared = if imported {
        HashSet::from([class.required_interfaces[0].iid.clone()])
    } else {
        HashSet::new()
    };
    project::project_class(
        &Default::default(),
        class,
        &HashSet::from(["Reader".into(), "IReader".into(), "IExtra".into()]),
        &HashSet::new(),
        &shared,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    )
}

#[test]
fn class_keeps_interface_names_without_changing_existing_overloads() {
    for imported in [false, true] {
        let class = reader();
        let projected = project_reader(&class, imported);
        let members = &projected.classes[0].members;
        for (alias, canonical, slot, interface, object) in [
            ("readShort", "read", 6, "_IReader", "this._obj"),
            (
                "writeShort",
                "write",
                6,
                "_IExtra",
                "this._obj.cast(IID_IExtra)",
            ),
            (
                "fetch2",
                "fetch",
                7,
                "_IExtra",
                "this._obj.cast(IID_IExtra)",
            ),
        ] {
            let find = |name: &str| {
                members
                    .iter()
                    .find_map(|member| match member {
                        ProjectedMember::Method(method)
                            if method.name == name
                                && method
                                    .invoke_expr
                                    .starts_with(&format!("{interface}.method({slot})")) =>
                        {
                            Some(method)
                        }
                        _ => None,
                    })
                    .expect("projected interface method")
            };
            let alias_method = find(alias);
            let canonical_method = find(canonical);
            assert_eq!(alias_method.invoke_expr, canonical_method.invoke_expr);
            assert_eq!(
                alias_method.sync_return_expr,
                canonical_method.sync_return_expr
            );
            assert_eq!(alias_method.return_type, canonical_method.return_type);
            assert_eq!(alias_method.overload_of, None);
            assert!(alias_method.invoke_expr.contains(object));
        }

        let declarations = render_dts::render(&projected);
        let class_declarations = declarations
            .split_once("export declare class Reader {\n")
            .unwrap()
            .1
            .split_once("\n}")
            .unwrap()
            .0;
        let signatures = class_declarations
            .lines()
            .filter(|line| {
                ["read", "write", "fetch"]
                    .iter()
                    .any(|prefix| line.trim_start().starts_with(prefix))
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            signatures,
            [
                "    read(value: number): number;",
                "    read(value: number, mode: number): number;",
                "    fetch(): number;",
                "    write(value: number): number;",
                "    fetch(value: number): number;",
                "    readShort(value: number): number;",
                "    writeShort(value: number): number;",
                "    fetch2(value: number): number;",
            ]
            .join("\n"),
            "instance declaration snapshot"
        );
        assert!(!declarations.contains("makeShort("));
        assert!(declarations.contains("static make(value: number): number;"));
    }
}

#[test]
fn compatibility_alias_does_not_replace_an_existing_public_member() {
    let mut class = reader();
    class.required_interfaces[0].methods.push(method(
        "ReadShort",
        "ReadShort",
        8,
        &["value", "mode"],
    ));
    let projected = project_reader(&class, false);
    let methods = projected.classes[0]
        .members
        .iter()
        .filter_map(|member| match member {
            ProjectedMember::Method(method) if method.name == "readShort" => Some(method),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 1);
    assert!(methods[0].invoke_expr.starts_with("_IExtra.method(8)"));

    let mut class = reader();
    class
        .default_interface
        .as_mut()
        .unwrap()
        .methods
        .push(MethodMeta {
            is_property_getter: true,
            ..method("get_ReadShort", "get_ReadShort", 9, &[])
        });
    let projected = project_reader(&class, false);
    assert!(!projected.classes[0].members.iter().any(|member| {
        matches!(member, ProjectedMember::Method(method) if method.name == "readShort")
    }));
    assert!(projected.classes[0].members.iter().any(|member| {
        matches!(member, ProjectedMember::Property(property) if property.name == "readShort")
    }));
}

#[test]
fn interface_aliases_execute_the_original_slots_and_qi_views() {
    let class = reader();
    let output = std::env::temp_dir().join(format!(
        "dynwinrt-javascript-interface-input-{}",
        std::process::id()
    ));
    fs::create_dir_all(&output).unwrap();
    fs::write(
        output.join("Reader.js"),
        render_js::render(&project_reader(&class, false)),
    )
    .unwrap();
    for interface in class
        .default_interface
        .iter()
        .chain(&class.required_interfaces)
    {
        let projected = project::project_interface(
            &Default::default(),
            interface,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        );
        fs::write(
            output.join(format!("{}.js", interface.name)),
            render_js::render(&projected),
        )
        .unwrap();
    }
    fs::write(
        output.join("lifetime.js"),
        "exports.trackProjectedValue = value => value;\n",
    )
    .unwrap();
    let runtime = output
        .join("node_modules")
        .join("@microsoft")
        .join("dynwinrt");
    fs::create_dir_all(&runtime).unwrap();
    fs::write(
        runtime.join("index.js"),
        r#"
const assert = require('node:assert/strict');
const calls = [];
class DynWinRtMethodSig { addIn() { return this; } addOut() { return this; } }
class DynWinRtType {
    static i32() { return new this(); }
    static registerInterface(name, iid) {
        return {
            addMethod() { return this; },
            method(slot) {
                return { invoke(obj, args) {
                    assert.equal(obj.view, iid, 'the call must use the declaring interface');
                    calls.push({ iid, slot, args: args.map(arg => arg.value) });
                    return { toNumber: () => slot };
                } };
            },
        };
    }
}
class DynWinRtValue { static i32(value) { return { value }; } }
class WinGuid { static parse(iid) { return iid; } }
module.exports = { DynWinRtType, DynWinRtMethodSig, DynWinRtValue, WinGuid, calls };
"#,
    )
    .unwrap();
    let result = Command::new("node")
        .args([
            "-e",
            r#"
const assert = require('node:assert/strict');
const { Reader } = require('./Reader.js');
const { IReader } = require('./IReader.js');
const { IExtra } = require('./IExtra.js');
const { calls } = require('@microsoft/dynwinrt');
const readerIid = '11111111-1111-1111-1111-111111111111';
const extraIid = '22222222-2222-2222-2222-222222222222';
const obj = { view: readerIid, cast(iid) { return { view: iid }; } };
const reader = Object.assign(Object.create(Reader.prototype), { _obj: obj });
assert.equal(reader.readShort(10), 6);
assert.equal(reader.read(10), 6);
assert.equal(reader.read(10, 20), 7);
assert.equal(reader.writeShort(30), 6);
assert.equal(reader.write(30), 6);
assert.equal(reader.fetch2(40), 7);
assert.equal(reader.fetch(40), 7);
assert.equal(reader.fetch(), 8);
assert.equal(IReader.from(obj).readShort(50), 6);
assert.equal(IExtra.from(obj).writeShort(60), 6);
assert.equal(IExtra.from(obj).fetch2(70), 7);
assert.deepEqual(calls, [
    { iid: readerIid, slot: 6, args: [10] },
    { iid: readerIid, slot: 6, args: [10] },
    { iid: readerIid, slot: 7, args: [10, 20] },
    { iid: extraIid, slot: 6, args: [30] },
    { iid: extraIid, slot: 6, args: [30] },
    { iid: extraIid, slot: 7, args: [40] },
    { iid: extraIid, slot: 7, args: [40] },
    { iid: readerIid, slot: 8, args: [] },
    { iid: readerIid, slot: 6, args: [50] },
    { iid: extraIid, slot: 6, args: [60] },
    { iid: extraIid, slot: 7, args: [70] },
]);
"#,
        ])
        .current_dir(&output)
        .output()
        .expect("execute projected aliases with node");
    fs::remove_dir_all(&output).unwrap();
    assert!(
        result.status.success(),
        "alias execution failed:\n{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
