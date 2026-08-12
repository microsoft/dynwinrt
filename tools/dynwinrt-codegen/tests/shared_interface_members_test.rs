// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use dynwinrt_codegen::codegen::{project, render_dts, render_js};
use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::TypeMeta;

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

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

fn items_control_interface() -> InterfaceMeta {
    InterfaceMeta {
        name: "IItemsControl".into(),
        namespace: "Contoso.Controls".into(),
        iid: "77777777-7777-7777-7777-777777777777".into(),
        methods: vec![MethodMeta {
            name: "get_Items".into(),
            raw_name: "get_Items".into(),
            vtable_index: 6,
            return_type: Some(TypeMeta::Object),
            is_property_getter: true,
            ..Default::default()
        }],
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

fn shared_sources(interface: &InterfaceMeta) -> HashSet<project::StandaloneInterfaceIdentity> {
    HashSet::from([project::standalone_interface_identity(interface).unwrap()])
}

fn snapshot_hash(contents: &str) -> u64 {
    contents.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

fn cli_test_directory(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("{name}-{}", std::process::id()))
}

fn run_codegen(output: &Path, class_names: &str, shared: bool) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"));
    command.args([
        "generate",
        "--winmd",
        WINDOWS_WINMD,
        "--namespace",
        "Windows.Foundation",
        "--class-name",
        class_names,
        "--output",
    ]);
    command.arg(output);
    command.args(["--import-name", "./runtime.js"]);
    if shared {
        command.arg("--shared-interface-members");
    }
    command.output().expect("run dynwinrt-codegen")
}

fn write_runtime_stub(output: &Path) {
    fs::write(
        output.join("runtime.js"),
        "\
class DynWinRtMethodSig { addIn() { return this; } addOut() { return this; } }\n\
const registration = { addMethod() { return this; }, method() { return {}; } };\n\
const DynWinRtType = new Proxy({\n\
  registerInterface() { return registration; },\n\
  parameterized() { return { iid() { return 'iid'; } }; },\n\
}, { get(target, key) { return target[key] ?? (() => ({})); } });\n\
const callable = new Proxy({}, { get() { return () => ({}); } });\n\
module.exports = {\n\
  DynWinRtType,\n\
  DynWinRtMethodSig,\n\
  DynWinRtValue: callable,\n\
  DynWinRtArray: callable,\n\
  DynWinRtDelegate: callable,\n\
  WinGuid: { parse(value) { return value; } },\n\
};\n",
    )
    .unwrap();
}

fn assert_node_script(output: &Path, name: &str, script: &str) {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("Skipping generated module-load assertion: node is unavailable");
        return;
    }
    let script_path = output.join(name);
    fs::write(&script_path, script).unwrap();
    let result = Command::new("node").arg(&script_path).output().unwrap();
    assert!(
        result.status.success(),
        "node failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );
}

fn snapshot_directory(output: &Path) -> HashMap<String, Vec<u8>> {
    fs::read_dir(output)
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_type().unwrap().is_file())
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

#[test]
fn cli_shared_uri_preserves_deep_raw_interface_exports() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping shared Uri CLI test: Windows.winmd not found");
        return;
    }

    let root = cli_test_directory("shared-uri-deep-interface");
    let baseline = root.join("baseline");
    let shared = root.join("shared");
    let _ = fs::remove_dir_all(&root);

    let baseline_result = run_codegen(&baseline, "Uri", false);
    assert!(
        baseline_result.status.success(),
        "{}",
        String::from_utf8_lossy(&baseline_result.stderr),
    );
    let shared_result = run_codegen(&shared, "Uri", true);
    assert!(
        shared_result.status.success(),
        "{}",
        String::from_utf8_lossy(&shared_result.stderr),
    );

    let baseline_js = fs::read_to_string(baseline.join("Uri.js")).unwrap();
    let baseline_dts = fs::read_to_string(baseline.join("Uri.d.ts")).unwrap();
    let shared_js = fs::read_to_string(shared.join("Uri.js")).unwrap();
    let shared_dts = fs::read_to_string(shared.join("Uri.d.ts")).unwrap();
    let canonical_js = fs::read_to_string(shared.join("IStringable.js")).unwrap();

    assert!(baseline_js.contains("exports.IStringable = IStringable;"));
    assert!(baseline_dts.contains("export declare class IStringable"));
    assert!(shared_js.contains(
        "Object.defineProperty(exports, 'IStringable', { enumerable: true, get: () => require('./IStringable.js').IStringable });"
    ));
    assert!(shared_dts.contains("export { IStringable } from './IStringable.js';"));
    assert!(shared_js.contains("return (__get_IStringable()).from(this._obj).toString();"));
    assert!(
        canonical_js
            .contains("Windows.Foundation.IStringable:96369f54-8eb6-48f0-abce-c1b211e627c3")
    );

    write_runtime_stub(&shared);
    assert_node_script(
        &shared,
        "verify-deep-interface.cjs",
        "\
const assert = require('node:assert/strict');\n\
const deep = require('./Uri.js');\n\
const canonical = require('./IStringable.js');\n\
assert.equal(deep.IStringable, canonical.IStringable);\n",
    );
    assert_node_script(
        &shared,
        "verify-deep-interface.mjs",
        "\
import assert from 'node:assert/strict';\n\
import { IStringable as Deep } from './Uri.js';\n\
import { IStringable as Canonical } from './IStringable.js';\n\
assert.equal(Deep, Canonical);\n",
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_mixed_flag_incremental_generation_preserves_shared_sources() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping mixed-flag CLI test: Windows.winmd not found");
        return;
    }

    let output = cli_test_directory("mixed-flag-shared-source");
    let _ = fs::remove_dir_all(&output);

    let shared_result = run_codegen(&output, "Deferral,MemoryBuffer", true);
    assert!(
        shared_result.status.success(),
        "{}",
        String::from_utf8_lossy(&shared_result.stderr),
    );
    write_runtime_stub(&output);
    let deferral_before = fs::read(output.join("Deferral.js")).unwrap();
    let shared_iclosable = fs::read_to_string(output.join("IClosable.js")).unwrap();
    assert!(
        shared_iclosable
            .contains("Windows.Foundation.IClosable:30d5a829-7fa4-4026-83bb-d75bae4ea99e")
    );
    assert!(shared_iclosable.contains("value._obj.cast(IID_IClosable)"));
    assert_node_script(
        &output,
        "verify-deferral.cjs",
        "require('./Deferral.js');\n",
    );

    let unflagged_result = run_codegen(&output, "MemoryBuffer", false);
    assert!(
        unflagged_result.status.success(),
        "{}",
        String::from_utf8_lossy(&unflagged_result.stderr),
    );
    assert_eq!(
        fs::read(output.join("Deferral.js")).unwrap(),
        deferral_before
    );
    let preserved_iclosable = fs::read_to_string(output.join("IClosable.js")).unwrap();
    assert!(
        preserved_iclosable
            .contains("Windows.Foundation.IClosable:30d5a829-7fa4-4026-83bb-d75bae4ea99e")
    );
    assert!(preserved_iclosable.contains("value._obj.cast(IID_IClosable)"));
    assert_node_script(
        &output,
        "verify-deferral-after-unflagged.cjs",
        "require('./Deferral.js');\n",
    );

    let mismatched_iclosable = preserved_iclosable.replace(
        "Windows.Foundation.IClosable:30d5a829-7fa4-4026-83bb-d75bae4ea99e",
        "Fabrikam.IClosable:11111111-1111-1111-1111-111111111111",
    );
    fs::write(output.join("IClosable.js"), mismatched_iclosable).unwrap();
    let before_rejection = snapshot_directory(&output);
    let rejected = run_codegen(&output, "MemoryBuffer", false);
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("Refusing to overwrite shared interface source")
    );
    assert_eq!(snapshot_directory(&output), before_rejection);

    fs::remove_dir_all(output).unwrap();
}

#[test]
fn default_projection_preserves_legacy_standalone_interface_imports() {
    let interface = value_interface();
    let class = widget_class(&interface);
    let known_types = HashSet::from(["Widget".into(), "IValue".into()]);
    let standalone_interface_iids = HashSet::from([interface.iid.clone()]);

    project::set_shared_interface_members(false);
    let projected = project::project_class(
        &class,
        &known_types,
        &HashSet::new(),
        &standalone_interface_iids,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let separated = project::project_class_with_shared_member_sources(
        &class,
        &known_types,
        &HashSet::new(),
        &standalone_interface_iids,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let descriptor_only = project::project_class_with_shared_member_sources(
        &class,
        &known_types,
        &HashSet::new(),
        &HashSet::new(),
        &shared_sources(&interface),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );

    let interface_imports = projected
        .imports
        .iter()
        .filter(|import| import.from == "./IValue.js")
        .map(|import| (import.symbols.clone(), import.runtime_only, import.dts_only))
        .collect::<Vec<_>>();
    assert_eq!(
        interface_imports,
        vec![(vec!["IID_IValue".into(), "IValue".into()], false, false)]
    );
    assert!(projected.classes[0].required_ifaces.is_empty());
    assert_eq!(descriptor_only.classes[0].required_ifaces.len(), 1);
    assert!(
        descriptor_only
            .imports
            .iter()
            .all(|import| import.from != "./IValue.js")
    );
    assert_eq!(render_js::render(&projected), render_js::render(&separated));
    assert_eq!(
        render_dts::render(&projected),
        render_dts::render(&separated)
    );

    let class_js = render_js::render(&projected);
    let class_dts = render_dts::render(&projected);
    assert_eq!(
        (snapshot_hash(&class_js), snapshot_hash(&class_dts)),
        (14855624038810436456, 13331642975496716118),
        "flag-off output changed from the origin/main compatibility snapshot"
    );
    assert!(class_js.contains("require('./IValue.js')"));
    assert!(class_js.contains("_IValue.method(6).invoke(this._obj.cast(IID_IValue)"));
    assert!(!class_js.contains("__copyInterfaceMembers"));
    assert!(!class_js.contains("export class IValue"));
}

#[test]
fn opt_in_reuses_shared_interface_descriptors_without_changing_dts() {
    let interface = value_interface();
    let class = widget_class(&interface);
    let known_types = HashSet::from(["Widget".into(), "IValue".into()]);
    let shared_iids = shared_sources(&interface);

    project::set_shared_interface_members(true);
    let interface_file = project::project_interface_with_shared_member_source(
        &interface,
        &known_types,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        true,
    );
    let class_file = project::project_class_with_shared_member_sources(
        &class,
        &known_types,
        &HashSet::new(),
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
    assert!(interface_js.contains(
        "Object.defineProperty(IValue, __sharedInterfaceMemberSource, { value: 'Contoso.IValue:11111111-1111-1111-1111-111111111111' });"
    ));
    assert!(class_js.contains(
        "__copyInterfaceMembers(Widget, (__get_IValue()), 'Contoso.IValue:11111111-1111-1111-1111-111111111111', ['value']);"
    ));
    assert!(class_js.contains("source[__sharedInterfaceMemberSource] !== identity"));
    assert!(!class_js.contains("_IValue.method(6).invoke(this._obj.cast(IID_IValue)"));
    assert!(class_dts.contains("get value(): number;"));
    assert!(class_dts.contains("set value(value: number);"));

    let default_class_js = render_js::render(&project::project_class(
        &class,
        &known_types,
        &HashSet::new(),
        &HashSet::from([interface.iid.clone()]),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    ));
    assert!(!default_class_js.contains("__copyInterfaceMembers"));
    assert!(default_class_js.contains("_IValue.method(6).invoke(this._obj.cast(IID_IValue)"));
}

#[test]
fn canonical_interface_source_survives_equivalent_duplicate_emission() {
    let shared = value_interface();
    let mut duplicate = shared.clone();
    duplicate.iid = shared.iid.to_ascii_uppercase();
    let sources = project::canonical_interface_sources(
        &[duplicate.clone()],
        std::slice::from_ref(&shared),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .unwrap();

    assert_eq!(sources.len(), 1);
    assert!(sources[0].shared_member_source);
    assert_eq!(sources[0].interface.iid, shared.iid);

    project::set_shared_interface_members(true);
    let interface_file = project::project_interface_with_shared_member_source(
        &sources[0].interface,
        &HashSet::from(["Widget".into(), "IValue".into()]),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        sources[0].shared_member_source,
    );
    let class_file = project::project_class_with_shared_member_sources(
        &widget_class(&duplicate),
        &HashSet::from(["Widget".into(), "IValue".into()]),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::from([sources[0].identity.clone()]),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    project::set_shared_interface_members(false);

    assert!(render_js::render(&interface_file).contains("__interfaceValue(this)"));
    assert!(render_js::render(&class_file).contains("__copyInterfaceMembers"));
    assert_eq!(
        render_dts::render(&interface_file),
        render_dts::render(&project::project_interface(
            &duplicate,
            &HashSet::from(["Widget".into(), "IValue".into()]),
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        ))
    );
}

#[test]
fn equivalent_cross_batch_emission_preserves_shared_source_marker() {
    let shared = value_interface();
    let identity = project::standalone_interface_identity(&shared).unwrap();
    let forced_shared_sources = HashSet::from([identity]);

    let sources = project::canonical_interface_sources(
        std::slice::from_ref(&shared),
        &[],
        &HashSet::new(),
        &HashSet::new(),
        &forced_shared_sources,
    )
    .unwrap();
    assert_eq!(sources.len(), 1);
    assert!(sources[0].shared_member_source);

    project::set_shared_interface_members(true);
    let interface_file = project::project_interface_with_shared_member_source(
        &sources[0].interface,
        &HashSet::from(["IValue".into()]),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        sources[0].shared_member_source,
    );
    project::set_shared_interface_members(false);
    assert!(render_js::render(&interface_file).contains("__interfaceValue(this)"));
}

#[test]
fn non_interface_output_collision_disables_forced_shared_source() {
    let interface = value_interface();
    let identity = project::standalone_interface_identity(&interface).unwrap();
    let sources = project::canonical_interface_sources(
        std::slice::from_ref(&interface),
        std::slice::from_ref(&interface),
        &HashSet::new(),
        &HashSet::from(["IValue".into()]),
        &HashSet::from([identity]),
    )
    .unwrap();

    assert_eq!(sources.len(), 1);
    assert!(!sources[0].shared_member_source);
    project::set_shared_interface_members(true);
    let class_file = project::project_class(
        &widget_class(&interface),
        &HashSet::from(["Widget".into(), "IValue".into()]),
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    project::set_shared_interface_members(false);
    let class_js = render_js::render(&class_file);
    assert!(!class_js.contains("__copyInterfaceMembers"));
    assert!(class_js.contains("_IValue.method(6).invoke(this._obj.cast(IID_IValue)"));
}

#[test]
fn ambiguous_standalone_interface_identities_remain_class_local() {
    let mut first = value_interface();
    first.methods.push(MethodMeta {
        name: "GetPeer".into(),
        raw_name: "GetPeer".into(),
        vtable_index: 8,
        return_type: Some(TypeMeta::Interface {
            namespace: first.namespace.clone(),
            name: first.name.clone(),
            iid: first.iid.clone(),
        }),
        ..Default::default()
    });
    let mut second = value_interface();
    second.namespace = "Fabrikam".into();
    second.iid = "88888888-8888-8888-8888-888888888888".into();
    second.methods.push(MethodMeta {
        name: "GetPeer".into(),
        raw_name: "GetPeer".into(),
        vtable_index: 8,
        return_type: Some(TypeMeta::Interface {
            namespace: second.namespace.clone(),
            name: second.name.clone(),
            iid: second.iid.clone(),
        }),
        ..Default::default()
    });

    let ambiguous_names = project::ambiguous_standalone_interface_names([&first, &second]);
    assert_eq!(ambiguous_names, HashSet::from(["IValue".into()]));

    let combined_sources = project::canonical_interface_sources(
        &[first.clone(), second.clone()],
        &[first.clone(), second.clone()],
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .expect("ambiguous flat interface names must fall back to normal generation");
    assert_eq!(combined_sources.len(), 1);
    assert!(!combined_sources[0].shared_member_source);
    assert_eq!(
        combined_sources[0].identity,
        project::standalone_interface_identity(&second).unwrap()
    );

    let first_class = widget_class(&first);
    let second_class = ClassMeta {
        name: "Gadget".into(),
        namespace: "Fabrikam".into(),
        full_name: "Fabrikam.Gadget".into(),
        required_interfaces: vec![second.clone()],
        ..Default::default()
    };
    let known_types = HashSet::from(["Widget".into(), "Gadget".into(), "IValue".into()]);

    project::set_shared_interface_members(true);
    for (interface, class) in [(&first, &first_class), (&second, &second_class)] {
        let sources = project::canonical_interface_sources(
            std::slice::from_ref(interface),
            std::slice::from_ref(interface),
            &HashSet::new(),
            &ambiguous_names,
            &HashSet::new(),
        )
        .expect("cross-batch ambiguity must fall back to normal generation");
        assert_eq!(sources.len(), 1);
        assert!(!sources[0].shared_member_source);
        let shared_source_identities = sources
            .iter()
            .filter(|source| source.shared_member_source)
            .map(|source| source.identity.clone())
            .collect::<HashSet<_>>();
        assert!(shared_source_identities.is_empty());

        let interface_file = project::project_interface_with_shared_member_source(
            &sources[0].interface,
            &known_types,
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            sources[0].shared_member_source,
        );
        assert!(!render_js::render(&interface_file).contains("__interfaceValue"));

        let class_file = project::project_class_with_excluded_interface_imports(
            class,
            &known_types,
            &HashSet::new(),
            &HashSet::new(),
            &shared_source_identities,
            &ambiguous_names,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(class_file.classes[0].required_ifaces.len(), 1);
        assert!(
            class_file
                .iid_consts
                .iter()
                .any(|iid| { iid.name == "IID_IValue" && iid.rhs_expr.contains(&interface.iid) })
        );
        let class_js = render_js::render(&class_file);
        assert!(!class_js.contains("__copyInterfaceMembers"));
        assert!(!class_js.contains("require(\"./IValue.js\")"), "{class_js}");
        assert!(
            class_js.contains("_IValue.method(6).invoke(this._obj.cast(IID_IValue)"),
            "{class_js}"
        );
    }
    project::set_shared_interface_members(false);
}

#[test]
fn nonshared_duplicate_interface_keeps_legacy_final_source() {
    let first = value_interface();
    let mut second = first.clone();
    second.namespace = "Fabrikam".into();
    second.iid = "88888888-8888-8888-8888-888888888888".into();

    let sources = project::canonical_interface_sources(
        &[first, second.clone()],
        &[],
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .unwrap();
    assert_eq!(sources.len(), 1);
    assert!(!sources[0].shared_member_source);
    assert_eq!(
        sources[0].identity,
        project::standalone_interface_identity(&second).unwrap()
    );
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
    let shared_iids = shared_sources(&interface);

    project::set_shared_interface_members(true);
    let interface_js = render_js::render(&project::project_interface_with_shared_member_source(
        &interface,
        &known_types,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        true,
    ));
    let class_file = project::project_class_with_shared_member_sources(
        &class,
        &known_types,
        &HashSet::new(),
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
    assert!(class_js.contains(
        "__verifyInterfaceSource((__get_IOverloaded()), 'Contoso.IOverloaded:22222222-2222-2222-2222-222222222222');"
    ));
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
    let shared_iids = shared_sources(&required_interface);

    project::set_shared_interface_members(true);
    let class_file = project::project_class_with_shared_member_sources(
        &class,
        &known_types,
        &HashSet::new(),
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
    assert!(class_js.contains(
        "__verifyInterfaceSource((__get_IRequired()), 'Contoso.IRequired:44444444-4444-4444-4444-444444444444');"
    ));
    assert!(class_js.contains("_doThing_1(value)"), "{class_js}");
    assert!(class_js.contains("_doThing_2(value, other)"), "{class_js}");
    assert!(class_js.contains("doThing(...args)"), "{class_js}");
    assert_eq!(class_dts.matches("doThing(").count(), 2);
}

#[test]
fn shared_interface_event_alias_conflicts_remain_class_local() {
    let default_interface = InterfaceMeta {
        name: "IWidget".into(),
        namespace: "Contoso".into(),
        iid: "55555555-5555-5555-5555-555555555555".into(),
        methods: vec![MethodMeta {
            name: "OnceChanged".into(),
            raw_name: "OnceChanged".into(),
            vtable_index: 6,
            ..Default::default()
        }],
        ..Default::default()
    };
    let required_interface = InterfaceMeta {
        name: "IChanged".into(),
        namespace: "Contoso".into(),
        iid: "66666666-6666-6666-6666-666666666666".into(),
        methods: vec![
            MethodMeta {
                name: "add_Changed".into(),
                raw_name: "add_Changed".into(),
                vtable_index: 6,
                params: vec![ParamMeta {
                    name: "handler".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::In,
                }],
                is_event_add: true,
                ..Default::default()
            },
            MethodMeta {
                name: "remove_Changed".into(),
                raw_name: "remove_Changed".into(),
                vtable_index: 7,
                params: vec![ParamMeta {
                    name: "token".into(),
                    typ: TypeMeta::I64,
                    direction: ParamDirection::In,
                }],
                is_event_remove: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let class = ClassMeta {
        default_interface: Some(default_interface),
        required_interfaces: vec![required_interface.clone()],
        ..widget_class(&required_interface)
    };
    let known_types = HashSet::from(["Widget".into(), "IWidget".into(), "IChanged".into()]);
    let shared_iids = shared_sources(&required_interface);

    project::set_shared_interface_members(true);
    let class_file = project::project_class_with_shared_member_sources(
        &class,
        &known_types,
        &HashSet::new(),
        &HashSet::new(),
        &shared_iids,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    project::set_shared_interface_members(false);
    let class_js = render_js::render(&class_file);

    assert!(!class_js.contains("__copyInterfaceMembers"));
    assert!(class_js.contains("onChanged(callback)"), "{class_js}");
    assert!(
        class_js.contains("_IChanged.method(6).invoke(this._obj.cast(IID_IChanged)"),
        "{class_js}"
    );
}

#[test]
fn noncanonical_required_interfaces_remain_class_local() {
    let interface = value_interface();
    let class = widget_class(&interface);
    let known_types = HashSet::from(["Widget".into(), "IValue".into()]);

    project::set_shared_interface_members(true);
    let class_file = project::project_class(
        &class,
        &known_types,
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    project::set_shared_interface_members(false);
    assert_eq!(class_file.classes[0].required_ifaces.len(), 1);
    let interface_file = project::project_interface_with_shared_member_source(
        &interface,
        &known_types,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        false,
    );
    let class_js = render_js::render(&class_file);
    let interface_js = render_js::render(&interface_file);

    assert!(!class_js.contains("__copyInterfaceMembers"));
    assert!(class_js.contains("_IValue.method(6).invoke(this._obj.cast(IID_IValue)"));
    assert!(!interface_js.contains("__interfaceValue"));
}

#[test]
fn excluded_iclosable_uses_local_wrapper() {
    let interface = InterfaceMeta {
        name: "IClosable".into(),
        namespace: "Windows.Foundation".into(),
        iid: "30d5a829-7fa4-4026-83bb-d75bae4ea99e".into(),
        methods: vec![MethodMeta {
            name: "Close".into(),
            raw_name: "Close".into(),
            vtable_index: 6,
            ..Default::default()
        }],
        ..Default::default()
    };
    let class = widget_class(&interface);

    let class_file = project::project_class_with_excluded_interface_imports(
        &class,
        &HashSet::from(["Widget".into(), "IClosable".into()]),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::from(["IClosable".into()]),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    assert_eq!(class_file.classes[0].required_ifaces.len(), 1);
    assert!(
        class_file
            .iid_consts
            .iter()
            .any(|iid| iid.name == "IID_IClosable" && iid.rhs_expr.contains(&interface.iid))
    );
    let class_js = render_js::render(&class_file);
    assert!(
        !class_js.contains("require(\"./IClosable.js\")"),
        "{class_js}"
    );
    assert!(class_js.contains("close()"), "{class_js}");
}

#[test]
fn collection_getter_casts_concrete_view_and_rejects_invalid_sources() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("Skipping shared-interface runtime test: node is unavailable");
        return;
    }

    let interface = items_control_interface();
    let class = ClassMeta {
        name: "ListView".into(),
        namespace: "Contoso.Controls".into(),
        full_name: "Contoso.Controls.ListView".into(),
        required_interfaces: vec![interface.clone()],
        ..Default::default()
    };
    let known_types = HashSet::from(["ListView".into(), "IItemsControl".into()]);
    let shared_sources = shared_sources(&interface);

    project::set_import_name("./runtime.js");
    project::set_shared_interface_members(true);
    let interface_js = render_js::render(&project::project_interface_with_shared_member_source(
        &interface,
        &known_types,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        true,
    ));
    let unsafe_interface_js =
        render_js::render(&project::project_interface_with_shared_member_source(
            &interface,
            &known_types,
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            false,
        ));
    let mut mismatched_interface = interface.clone();
    mismatched_interface.namespace = "Fabrikam.Controls".into();
    mismatched_interface.iid = "99999999-9999-9999-9999-999999999999".into();
    let mismatched_interface_js =
        render_js::render(&project::project_interface_with_shared_member_source(
            &mismatched_interface,
            &known_types,
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            true,
        ));
    let class_js = render_js::render(&project::project_class_with_shared_member_sources(
        &class,
        &known_types,
        &HashSet::new(),
        &HashSet::new(),
        &shared_sources,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    ));
    project::set_shared_interface_members(false);
    project::set_import_name("@microsoft/dynwinrt");

    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("shared-collection-runtime-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("IItemsControl.js"), &interface_js).unwrap();
    fs::write(directory.join("ListView.js"), class_js).unwrap();
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
      object() { return {}; },\n\
    };\n\
    const DynWinRtValue = {};\n\
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
    const { IItemsControl } = require('./IItemsControl.js');\n\
    const { ListView } = require('./ListView.js');\n\
    const items = { isNull: () => false, kind: 'ItemCollection' };\n\
    const missing = { isNull: () => true };\n\
    let casts = 0;\n\
    const interfaceValue = {\n\
      invoke(index) {\n\
        assert.equal(index, 6);\n\
        return items;\n\
      },\n\
    };\n\
    const raw = {\n\
      invoke(index) {\n\
        assert.equal(index, 6);\n\
        return missing;\n\
      },\n\
      cast() {\n\
        casts++;\n\
        return interfaceValue;\n\
      },\n\
    };\n\
    const list = Object.assign(Object.create(ListView.prototype), { _obj: raw });\n\
    const concreteDescriptor = Object.getOwnPropertyDescriptor(ListView.prototype, 'items');\n\
    const interfaceDescriptor = Object.getOwnPropertyDescriptor(IItemsControl.prototype, 'items');\n\
    assert.equal(concreteDescriptor.get, interfaceDescriptor.get);\n\
    assert.equal(list.items, items);\n\
    assert.equal(IItemsControl.from(raw).items, items);\n\
    assert.equal(casts, 2);\n",
        )
        .unwrap();

    let output = Command::new("node")
        .arg("test.js")
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "node failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    fs::write(directory.join("IItemsControl.js"), unsafe_interface_js).unwrap();
    let fail_closed = Command::new("node")
            .args([
                "-e",
                "require('node:assert/strict').throws(() => require('./ListView.js'), /not a shared member source/)",
            ])
            .current_dir(&directory)
            .output()
            .unwrap();
    assert!(
        fail_closed.status.success(),
        "unmarked fail-closed node check failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&fail_closed.stdout),
        String::from_utf8_lossy(&fail_closed.stderr),
    );

    fs::write(directory.join("IItemsControl.js"), mismatched_interface_js).unwrap();
    let mismatched_fail_closed = Command::new("node")
        .args([
            "-e",
            "require('node:assert/strict').throws(() => require('./ListView.js'), /not a shared member source/)",
        ])
        .current_dir(&directory)
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&directory);
    assert!(
        mismatched_fail_closed.status.success(),
        "mismatched fail-closed node check failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&mismatched_fail_closed.stdout),
        String::from_utf8_lossy(&mismatched_fail_closed.stderr),
    );
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
    let shared_iids = shared_sources(&interface);

    project::set_import_name("./runtime.js");
    project::set_shared_interface_members(true);
    let interface_js = render_js::render(&project::project_interface_with_shared_member_source(
        &interface,
        &known_types,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        true,
    ));
    let class_js = render_js::render(&project::project_class_with_shared_member_sources(
        &class,
        &known_types,
        &HashSet::new(),
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
