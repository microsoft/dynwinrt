// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::process::Command;

use dynwinrt_codegen::codegen::{project, render_dts, render_js};
use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::TypeMeta;

fn value_interface() -> InterfaceMeta {
    InterfaceMeta {
        name: "IValue".into(),
        namespace: "Contoso".into(),
        iid: "11111111-1111-1111-1111-111111111111".into(),
        methods: vec![
            MethodMeta {
                name: "get_Value".into(),
                raw_name: "get_Value".into(),
                vtable_index: 6,
                return_type: Some(TypeMeta::I32),
                is_property_getter: true,
                ..Default::default()
            },
            MethodMeta {
                name: "put_Value".into(),
                raw_name: "put_Value".into(),
                vtable_index: 7,
                params: vec![ParamMeta {
                    name: "value".into(),
                    typ: TypeMeta::I32,
                    direction: ParamDirection::In,
                }],
                is_property_setter: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn widget_class(interface: &InterfaceMeta) -> ClassMeta {
    ClassMeta {
        name: "Widget".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.Widget".into(),
        required_interfaces: vec![interface.clone()],
        ..Default::default()
    }
}

#[test]
fn opt_in_reuses_shared_interface_descriptors_without_changing_dts() {
    let interface = value_interface();
    let class = widget_class(&interface);
    let known_types = HashSet::from(["Widget".into(), "IValue".into()]);
    let shared_iids = HashSet::from([interface.iid.clone()]);

    project::set_shared_interface_members(true);
    let interface_file = project::project_interface(
        &interface,
        &known_types,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let class_file = project::project_class(
        &class,
        &known_types,
        &HashSet::new(),
        &shared_iids,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    project::set_shared_interface_members(false);

    let interface_js = render_js::render(&interface_file);
    let class_js = render_js::render(&class_file);
    let class_dts = render_dts::render(&class_file);

    assert!(interface_js.contains("const __interfaceInstances = new WeakSet();"));
    assert!(interface_js.contains("__interfaceValue(this)"));
    assert!(class_js.contains("__copyInterfaceMembers(Widget, (__get_IValue()), ['value']);"));
    assert!(!class_js.contains("_IValue.method(6).invoke(this._obj.cast(IID_IValue)"));
    assert!(class_dts.contains("get value(): number;"));
    assert!(class_dts.contains("set value(value: number);"));

    let default_class_js = render_js::render(&project::project_class(
        &class,
        &known_types,
        &HashSet::new(),
        &shared_iids,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    ));
    assert!(!default_class_js.contains("__copyInterfaceMembers"));
    assert!(default_class_js.contains("_IValue.method(6).invoke(this._obj.cast(IID_IValue)"));
}

#[test]
fn shared_interface_members_preserve_overload_dispatch_and_declarations() {
    let interface = InterfaceMeta {
        name: "IOverloaded".into(),
        namespace: "Contoso".into(),
        iid: "22222222-2222-2222-2222-222222222222".into(),
        methods: vec![
            MethodMeta {
                name: "DoThing".into(),
                raw_name: "DoThing".into(),
                vtable_index: 6,
                params: vec![ParamMeta {
                    name: "value".into(),
                    typ: TypeMeta::I32,
                    direction: ParamDirection::In,
                }],
                ..Default::default()
            },
            MethodMeta {
                name: "DoThing2".into(),
                raw_name: "DoThing".into(),
                vtable_index: 7,
                params: vec![
                    ParamMeta {
                        name: "value".into(),
                        typ: TypeMeta::I32,
                        direction: ParamDirection::In,
                    },
                    ParamMeta {
                        name: "other".into(),
                        typ: TypeMeta::I32,
                        direction: ParamDirection::In,
                    },
                ],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let class = widget_class(&interface);
    let known_types = HashSet::from(["Widget".into(), "IOverloaded".into()]);
    let shared_iids = HashSet::from([interface.iid.clone()]);

    project::set_shared_interface_members(true);
    let interface_js = render_js::render(&project::project_interface(
        &interface,
        &known_types,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    ));
    let class_file = project::project_class(
        &class,
        &known_types,
        &HashSet::new(),
        &shared_iids,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    project::set_shared_interface_members(false);
    let class_js = render_js::render(&class_file);
    let class_dts = render_dts::render(&class_file);

    assert!(interface_js.contains("doThing(value)"), "{interface_js}");
    assert!(
        interface_js.contains("doThing2(value, other)"),
        "{interface_js}"
    );
    assert!(!class_js.contains("__copyInterfaceMembers"));
    assert!(class_js.contains("_doThing_1(value)"), "{class_js}");
    assert!(class_js.contains("_doThing_2(value, other)"), "{class_js}");
    assert!(class_js.contains("doThing(...args)"), "{class_js}");
    assert_eq!(class_dts.matches("doThing(").count(), 2);
}

#[test]
fn shared_interface_members_preserve_cross_interface_overload_dispatch() {
    let default_interface = InterfaceMeta {
        name: "IWidget".into(),
        namespace: "Contoso".into(),
        iid: "33333333-3333-3333-3333-333333333333".into(),
        methods: vec![MethodMeta {
            name: "DoThing2".into(),
            raw_name: "DoThing2".into(),
            vtable_index: 6,
            params: vec![
                ParamMeta {
                    name: "value".into(),
                    typ: TypeMeta::I32,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "other".into(),
                    typ: TypeMeta::I32,
                    direction: ParamDirection::In,
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    let required_interface = InterfaceMeta {
        name: "IRequired".into(),
        namespace: "Contoso".into(),
        iid: "44444444-4444-4444-4444-444444444444".into(),
        methods: vec![MethodMeta {
            name: "DoThing".into(),
            raw_name: "DoThing".into(),
            vtable_index: 6,
            params: vec![ParamMeta {
                name: "value".into(),
                typ: TypeMeta::I32,
                direction: ParamDirection::In,
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let class = ClassMeta {
        default_interface: Some(default_interface),
        required_interfaces: vec![required_interface.clone()],
        ..widget_class(&required_interface)
    };
    let known_types = HashSet::from(["Widget".into(), "IWidget".into(), "IRequired".into()]);
    let shared_iids = HashSet::from([required_interface.iid.clone()]);

    project::set_shared_interface_members(true);
    let class_file = project::project_class(
        &class,
        &known_types,
        &HashSet::new(),
        &shared_iids,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    project::set_shared_interface_members(false);
    let class_js = render_js::render(&class_file);
    let class_dts = render_dts::render(&class_file);

    assert!(!class_js.contains("__copyInterfaceMembers"));
    assert!(class_js.contains("_doThing_1(value)"), "{class_js}");
    assert!(class_js.contains("_doThing_2(value, other)"), "{class_js}");
    assert!(class_js.contains("doThing(...args)"), "{class_js}");
    assert_eq!(class_dts.matches("doThing(").count(), 2);
}

#[test]
fn shared_interface_descriptor_executes_for_raw_and_concrete_views() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("Skipping shared-interface runtime test: node is unavailable");
        return;
    }

    let interface = value_interface();
    let class = widget_class(&interface);
    let known_types = HashSet::from(["Widget".into(), "IValue".into()]);
    let shared_iids = HashSet::from([interface.iid.clone()]);

    project::set_import_name("./runtime.js");
    project::set_shared_interface_members(true);
    let interface_js = render_js::render(&project::project_interface(
        &interface,
        &known_types,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    ));
    let class_js = render_js::render(&project::project_class(
        &class,
        &known_types,
        &HashSet::new(),
        &shared_iids,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    ));
    project::set_shared_interface_members(false);
    project::set_import_name("@microsoft/dynwinrt");

    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("shared-interface-runtime-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("IValue.js"), interface_js).unwrap();
    fs::write(directory.join("Widget.js"), class_js).unwrap();
    fs::write(
        directory.join("lifetime.js"),
        "\
exports.castProjectedValueBorrowed = (value) => value;\n\
exports.castProjectedValueOwned = (value) => value;\n\
exports.trackProjectedValue = (value) => value;\n",
    )
    .unwrap();
    fs::write(
        directory.join("runtime.js"),
        "\
class DynWinRtMethodSig {\n\
  addIn() { return this; }\n\
  addOut() { return this; }\n\
}\n\
const DynWinRtType = {\n\
  registerInterface() {\n\
    return {\n\
      addMethod() { return this; },\n\
      method(index) { return { invoke: (obj, args) => obj.invoke(index, args) }; },\n\
    };\n\
  },\n\
  i32() { return {}; },\n\
};\n\
const DynWinRtValue = { i32: (value) => value };\n\
const DynWinRtArray = {};\n\
const DynWinRtDelegate = {};\n\
const WinGuid = { parse: (value) => value };\n\
module.exports = { DynWinRtType, DynWinRtMethodSig, DynWinRtValue, DynWinRtArray, DynWinRtDelegate, WinGuid };\n",
    )
    .unwrap();
    fs::write(
        directory.join("test.js"),
        "\
const assert = require('node:assert/strict');\n\
const { IValue } = require('./IValue.js');\n\
const { Widget } = require('./Widget.js');\n\
let current = 41;\n\
let casts = 0;\n\
const interfaceValue = {\n\
  invoke(index, args) {\n\
    if (index === 6) return { toNumber: () => current };\n\
    if (index === 7) { current = args[0]; return undefined; }\n\
    throw new Error(`unexpected slot ${index}`);\n\
  },\n\
};\n\
const raw = { cast() { casts++; return interfaceValue; } };\n\
const widget = Object.assign(Object.create(Widget.prototype), { _obj: raw });\n\
const concreteDescriptor = Object.getOwnPropertyDescriptor(Widget.prototype, 'value');\n\
const interfaceDescriptor = Object.getOwnPropertyDescriptor(IValue.prototype, 'value');\n\
assert.equal(concreteDescriptor.get, interfaceDescriptor.get);\n\
assert.equal(widget.value, 41);\n\
widget.value = 52;\n\
assert.equal(widget.value, 52);\n\
const view = IValue.from(raw);\n\
assert.equal(view.value, 52);\n\
assert.equal(casts, 4);\n",
    )
    .unwrap();

    let output = Command::new("node")
        .arg("test.js")
        .current_dir(&directory)
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&directory);
    assert!(
        output.status.success(),
        "node failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
