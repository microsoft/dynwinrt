// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;
use std::path::Path;

use dynwinrt_codegen::codegen::{project, python, python_stub, render_dts, render_js};
use dynwinrt_codegen::meta;
use dynwinrt_codegen::types::TypeMeta;

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

#[test]
fn nullable_runtime_class_returns_are_guarded_without_type_changes() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }

    let Some(accelerometer) =
        meta::parse_class(WINDOWS_WINMD, "Windows.Devices.Sensors", "Accelerometer")
    else {
        panic!("Accelerometer metadata not found");
    };

    let roots = vec![accelerometer];
    let deps = meta::resolve_dependencies(WINDOWS_WINMD, &roots, &[], &[]);
    let mut classes = roots;
    classes.extend(deps.classes);
    let interfaces = deps.interfaces;

    let mut known_types = HashSet::new();
    for class in &classes {
        known_types.insert(class.name.clone());
    }
    for interface in &interfaces {
        known_types.insert(interface.name.clone());
    }
    for en in &deps.enums {
        if let TypeMeta::Enum { name, .. } = en {
            known_types.insert(name.clone());
        }
    }

    let delegate_type_names: HashSet<String> = interfaces
        .iter()
        .filter(|interface| {
            interface
                .methods
                .iter()
                .any(|method| method.name == ".ctor")
                && interface
                    .methods
                    .iter()
                    .any(|method| method.name == "Invoke")
        })
        .map(|interface| interface.name.clone())
        .collect();
    let (delegate_sigs, delegate_sig_refs, delegate_param_wraps) =
        project::build_delegate_signatures(&interfaces, &delegate_type_names, &known_types);

    let class = classes
        .iter()
        .find(|class| class.name == "Accelerometer")
        .expect("Accelerometer class missing after dependency resolution");
    let projected = project::project_class(
        class,
        &known_types,
        &delegate_type_names,
        &HashSet::new(),
        &delegate_sigs,
        &delegate_sig_refs,
        &delegate_param_wraps,
    );
    let js = render_js::render(&projected);
    let dts = render_dts::render(&projected);

    assert!(js.contains("v.isNull() ? null : Accelerometer._fromNative(v)"));
    assert!(dts.contains("static getDefault(): Accelerometer;"));
    assert!(dts.contains("Promise<Accelerometer>"));
    assert!(dts.contains("getCurrentReading(): AccelerometerReading;"));
}

#[test]
fn ireference_values_are_projected_as_native_nullable_values() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }

    let Some(contact_date) = meta::parse_class(
        WINDOWS_WINMD,
        "Windows.ApplicationModel.Contacts",
        "ContactDate",
    ) else {
        panic!("ContactDate metadata not found");
    };

    let roots = vec![contact_date];
    let deps = meta::resolve_dependencies(WINDOWS_WINMD, &roots, &[], &[]);
    let mut classes = roots;
    classes.extend(deps.classes);
    let interfaces = deps.interfaces;

    let mut known_types = HashSet::new();
    for class in &classes {
        known_types.insert(class.name.clone());
    }
    for interface in &interfaces {
        known_types.insert(interface.name.clone());
    }
    for en in &deps.enums {
        if let TypeMeta::Enum { name, .. } = en {
            known_types.insert(name.clone());
        }
    }

    let delegate_type_names: HashSet<String> = interfaces
        .iter()
        .filter(|interface| {
            interface
                .methods
                .iter()
                .any(|method| method.name == ".ctor")
                && interface
                    .methods
                    .iter()
                    .any(|method| method.name == "Invoke")
        })
        .map(|interface| interface.name.clone())
        .collect();

    let class = classes
        .iter()
        .find(|class| class.name == "ContactDate")
        .expect("ContactDate class missing after dependency resolution");
    let (delegate_sigs, delegate_sig_refs, delegate_param_wraps) =
        project::build_delegate_signatures(&interfaces, &delegate_type_names, &known_types);
    let projected = project::project_class(
        class,
        &known_types,
        &delegate_type_names,
        &HashSet::new(),
        &delegate_sigs,
        &delegate_sig_refs,
        &delegate_param_wraps,
    );
    let js = render_js::render(&projected);
    let dts = render_dts::render(&projected);
    let py = python::generate_class(class, &known_types, &delegate_type_names, &HashSet::new());
    let pyi = python_stub::generate_class_stub(
        class,
        &known_types,
        &delegate_type_names,
        &HashSet::new(),
    );

    assert!(py.contains("def day(self) -> int | None:"));
    assert!(py.contains(
        "None if value.is_null() else _dynwinrt_symbol('i_reference_uint32', 'IReference_UInt32')(value).value"
    ));
    assert!(py.contains("def day(self, value: int | None | IReference_UInt32):"));
    assert!(py.contains(
        "_dynwinrt_box_reference(value, DynWinRTType.u32_type(), lambda value: DynWinRTValue.from_u32(value))"
    ));
    assert!(pyi.contains("def day(self) -> int | None: ..."));
    assert!(pyi.contains("def day(self, value: int | None | IReference_UInt32) -> None: ..."));
    assert!(
        js.contains("v.isNull() ? null")
            && js.contains("IReference_UInt32")
            && js.contains("(v).value"),
        "JavaScript IReference getter must unbox nullable values:\n{js}"
    );
    assert!(
        js.contains("DynWinRtValue.boxReference(DynWinRtValue.u32(value), DynWinRtType.u32())")
    );
    assert!(dts.contains("get day(): number | null;"));
    assert!(dts.contains("set day(value: number | null | IReference_UInt32);"));
}
