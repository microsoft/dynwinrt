// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;

use dynwinrt_codegen::codegen::render_js;
use dynwinrt_codegen::codegen::winrt::javascript::project;
use dynwinrt_codegen::codegen::winrt::python;
use dynwinrt_codegen::meta::{InterfaceMeta, parse_class};
use dynwinrt_codegen::types::TypeMeta;

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

#[test]
fn sdk_color_palette_resources_projects_inherited_generic_map_interfaces() {
    if !std::path::Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }

    let class = parse_class(WINDOWS_WINMD, "Windows.UI.Xaml", "ColorPaletteResources").unwrap();
    let map = class
        .required_interfaces
        .iter()
        .find(|interface| interface.name == "IMap_Object_Object")
        .expect("ResourceDictionary's concrete IMap<Object, Object> must be inherited");
    assert_eq!(map.generic_args, [TypeMeta::Object, TypeMeta::Object]);
    assert!(map.methods.iter().any(|method| method.name == "Lookup"));
    assert!(map.methods.iter().any(|method| method.name == "Insert"));

    let iterable = class
        .required_interfaces
        .iter()
        .find(|interface| interface.name == "IIterable_IKeyValuePair_Object_Object")
        .expect("ResourceDictionary's concrete IIterable<IKeyValuePair<Object, Object>> missing");
    assert!(iterable.generic_piid.is_some());
    assert!(iterable.methods.iter().any(|method| method.name == "First"));

    let known_types = HashSet::from(["ColorPaletteResources".to_string()]);
    let interfaces: Vec<InterfaceMeta> = Vec::new();
    let (delegate_sigs, delegate_sig_refs, delegate_param_wraps) =
        project::build_delegate_signatures(
            &Default::default(),
            &interfaces,
            &HashSet::new(),
            &known_types,
        );
    let projected = project::project_class(
        &Default::default(),
        &class,
        &known_types,
        &HashSet::new(),
        &HashSet::new(),
        &delegate_sigs,
        &delegate_sig_refs,
        &delegate_param_wraps,
    );
    let js = render_js::render(&projected);
    let py = python::generate_class(&class, &known_types, &HashSet::new(), &HashSet::new());
    assert!(js.contains("lookup(") && js.contains("insert("), "{js}");
    assert!(
        py.contains("class ColorPaletteResources(_WinRTMutableMappingMixin):")
            && py.contains("def lookup(")
            && py.contains("def insert("),
        "{py}"
    );
}
