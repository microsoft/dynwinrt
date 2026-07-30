// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;
use std::path::Path;

use dynwinrt_codegen::codegen::{project, render_js};
use dynwinrt_codegen::meta;
use dynwinrt_codegen::types::TypeMeta;

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

#[test]
fn delegate_overloads_emit_local_delegate_values() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }

    let Some(dispatcher_queue) =
        meta::parse_class(WINDOWS_WINMD, "Windows.System", "DispatcherQueue")
    else {
        panic!("DispatcherQueue metadata not found");
    };

    let roots = vec![dispatcher_queue];
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
        .find(|class| class.name == "DispatcherQueue")
        .expect("DispatcherQueue missing after dependency resolution");
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

    assert_eq!(
        js.matches("const _callback_d = DynWinRtDelegate.create(")
            .count(),
        2,
        "Each tryEnqueue overload must declare its callback delegate:\n{js}"
    );
    assert!(js.contains("_tryEnqueue_1(callback)"));
    assert!(js.contains("_tryEnqueue_2(priority, callback)"));
    assert!(js.contains("(callback == null ? DynWinRtValue.nullValue() : _callback_d)"));
}
