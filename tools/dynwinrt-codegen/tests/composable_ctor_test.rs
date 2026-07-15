// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Regression test for the composable runtime-class constructor.
//!
//! Unsealed WinRT runtime classes expose a derived-from constructor on the
//! default/required instance interface, whose CLR method name is literally
//! `.ctor`. It implements the COM aggregation pattern and is only meant to be
//! invoked by a host framework (in practice, XAML). Codegen used to fall
//! through to the regular instance-method path for these methods. Python emits
//! invalid `def .ctor(self) -> None:` syntax; JavaScript either emits invalid
//! `.ctor()` syntax or sanitizes it into a misleading `ctor()` method. This test
//! pins the fix: `.ctor` on a non-delegate interface must be skipped at code
//! emission, while still being kept in interface registration so vtable indices
//! remain correct.

use std::collections::{HashMap, HashSet};

use dynwinrt_codegen::codegen::{project, python, python_stub, render_dts, render_js};
use dynwinrt_codegen::meta::{InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::TypeMeta;

/// Build a synthetic instance interface containing a composable `.ctor` plus
/// one ordinary property-getter so we can verify the test interface was
/// actually projected (i.e. we're not just generating an empty class).
fn make_composable_iface() -> InterfaceMeta {
    InterfaceMeta {
        name: "ITestInstance".into(),
        namespace: "Test.Sample".into(),
        iid: "11111111-2222-3333-4444-555555555555".into(),
        methods: vec![
            // Ordinary property — must appear in output.
            MethodMeta {
                name: "get_Name".into(),
                vtable_index: 6,
                params: vec![],
                return_type: Some(TypeMeta::String),
                is_property_getter: true,
                ..Default::default()
            },
            // Composable `.ctor(IInspectable* base, IInspectable** inner)` —
            // must NOT appear in output.
            MethodMeta {
                name: ".ctor".into(),
                vtable_index: 7,
                params: vec![
                    ParamMeta {
                        name: "baseInterface".into(),
                        typ: TypeMeta::Object,
                        direction: ParamDirection::In,
                    },
                    ParamMeta {
                        name: "innerInterface".into(),
                        typ: TypeMeta::Object,
                        direction: ParamDirection::Out,
                    },
                ],
                return_type: None,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn assert_no_ctor(label: &str, output: &str) {
    // The legitimate appearance of `.ctor` is inside the interface-registration
    // string literal (`.addMethod(".ctor", ...)` / `.add_method(".ctor", ...)`).
    // We must NOT see it anywhere else — particularly not as a method
    // declaration, which would be a syntax error in both TS and Python.
    let bad_patterns = [
        ".ctor(",      // TS method/declaration syntax: `.ctor(): void { ... }`
        ".ctor:",      // DTS field syntax: `.ctor: () => void`
        "\n    ctor(", // Sanitized JS method syntax on current main
        "\n    ctor:", // Sanitized DTS method syntax on current main
        "def .ctor",   // Python method definition
        "get .ctor",   // TS getter
        "set .ctor",   // TS setter
        "@.ctor",      // TS decorator (defensive)
    ];
    for pat in bad_patterns {
        assert!(
            !output.contains(pat),
            "{label}: emitted output contains forbidden pattern `{pat}`:\n{output}",
        );
    }
    // Sanity: the test interface must have been projected — otherwise an
    // empty/skipped output would trivially pass.
    assert!(
        output.contains("ITestInstance"),
        "{label}: emitted output is missing the interface name `ITestInstance`:\n{output}",
    );
}

#[test]
fn typescript_codegen_skips_composable_ctor() {
    let iface = make_composable_iface();
    let known_types: HashSet<String> = std::iter::once("ITestInstance".to_string()).collect();
    let delegate_type_names: HashSet<String> = HashSet::new();
    let delegate_sigs: HashMap<String, String> = HashMap::new();
    let delegate_sig_refs: HashMap<String, Vec<String>> = HashMap::new();
    let delegate_param_wraps: HashMap<String, Vec<String>> = HashMap::new();

    let projected = project::project_interface(
        &iface,
        &known_types,
        &delegate_type_names,
        &delegate_sigs,
        &delegate_sig_refs,
        &delegate_param_wraps,
    );

    // The interface must NOT be classified as a delegate (no Invoke method),
    // otherwise this whole test scenario wouldn't apply.
    assert!(
        projected.ifaces.iter().all(|i| !i.is_delegate),
        "synthetic interface should not be classified as a delegate",
    );

    let js = render_js::render(&projected);
    let dts = render_dts::render(&projected);

    assert_no_ctor("render_js", &js);
    assert_no_ctor("render_dts", &dts);

    // The legitimate property must still be there — so we know the interface
    // was actually emitted and not skipped wholesale.
    assert!(
        js.contains("get name()"),
        "render_js missing `name` getter:\n{js}"
    );
    assert!(
        dts.contains("name"),
        "render_dts missing `name` declaration:\n{dts}"
    );
}

#[test]
fn python_codegen_skips_composable_ctor() {
    let iface = make_composable_iface();
    let known_types: HashSet<String> = std::iter::once("ITestInstance".to_string()).collect();
    let delegate_type_names: HashSet<String> = HashSet::new();

    let py = python::generate_interface(&iface, &known_types, &delegate_type_names);
    assert_no_ctor("python::generate_interface", &py);

    let pyi = python_stub::generate_interface_stub(&iface, &known_types, &delegate_type_names);
    assert_no_ctor("python_stub::generate_interface_stub", &pyi);

    // Snake-case `name` property must still be emitted.
    assert!(
        py.contains("def name"),
        "python output missing `name` property:\n{py}"
    );
    assert!(
        pyi.contains("def name"),
        "pyi output missing `name` property:\n{pyi}"
    );
}

#[test]
fn interface_registration_still_records_ctor_vtable_slot() {
    // Even though `.ctor` is suppressed in the emitted public surface, the
    // interface-registration / vtable description must still account for it
    // so IID computation and vtable indices stay correct for downstream
    // method calls. We assert this by checking that the registration block
    // emitted by render_js mentions the vtable slot ABI for `.ctor`'s
    // parameters (two `Object`-shaped slots) — i.e. methods after the `.ctor`
    // would otherwise have wrong vtable indices.
    let iface = make_composable_iface();
    let known_types: HashSet<String> = std::iter::once("ITestInstance".to_string()).collect();
    let projected = project::project_interface(
        &iface,
        &known_types,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let js = render_js::render(&projected);

    // The `.ctor` registration entry must reserve a vtable slot. Implementations
    // typically express this via `.addMethod(...)` calls; the count must equal
    // the number of methods in the interface (including `.ctor`), so that
    // subsequent vtable indices line up.
    let add_method_count = js.matches(".addMethod").count();
    assert_eq!(
        add_method_count,
        iface.methods.len(),
        "registration must reserve a vtable slot for every method (including `.ctor`); \
         got {} addMethod calls for {} methods:\n{}",
        add_method_count,
        iface.methods.len(),
        js,
    );
}
