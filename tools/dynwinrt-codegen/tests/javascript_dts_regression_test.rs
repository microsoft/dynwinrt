// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{HashMap, HashSet};

use dynwinrt_codegen::codegen::{project, render_dts};
use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::TypeMeta;

#[test]
fn delegate_callback_return_imports_dynwinrt_value() {
    let delegate = InterfaceMeta {
        name: "CreateDefaultValueCallback".into(),
        iid: "11111111-1111-1111-1111-111111111111".into(),
        methods: vec![
            MethodMeta {
                name: ".ctor".into(),
                ..Default::default()
            },
            MethodMeta {
                name: "Invoke".into(),
                return_type: Some(TypeMeta::Object),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let delegate_names = HashSet::from([delegate.name.clone()]);
    let (signatures, references, _) = project::build_delegate_signatures(
        std::slice::from_ref(&delegate),
        &delegate_names,
        &delegate_names,
    );

    let projected = project::project_delegate(&delegate, &signatures, &references);
    let dts = render_dts::render(&projected);

    assert!(
        dts.contains("import { DynWinRtType, WinGuid, DynWinRtValue } from '@microsoft/dynwinrt';"),
        "{dts}"
    );
    assert!(
        dts.contains("export type CreateDefaultValueCallback = () => DynWinRtValue;"),
        "{dts}"
    );
}

#[test]
fn imported_collection_interface_is_not_redeclared_inline() {
    let map_type = TypeMeta::Parameterized {
        namespace: "Windows.Foundation.Collections".into(),
        name: "IMap".into(),
        piid: "3c2925fe-8519-45c1-aa79-197b6718c1c1".into(),
        args: vec![TypeMeta::Object, TypeMeta::Object],
    };
    let map_interface = InterfaceMeta {
        name: "IMap_Object_Object".into(),
        iid: "22222222-2222-2222-2222-222222222222".into(),
        generic_piid: Some("3c2925fe-8519-45c1-aa79-197b6718c1c1".into()),
        generic_args: vec![TypeMeta::Object, TypeMeta::Object],
        ..Default::default()
    };
    let class = ClassMeta {
        name: "ResourceDictionary".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.ResourceDictionary".into(),
        default_interface: Some(InterfaceMeta {
            name: "IResourceDictionary".into(),
            iid: "33333333-3333-3333-3333-333333333333".into(),
            methods: vec![MethodMeta {
                name: "get_ThemeDictionaries".into(),
                return_type: Some(map_type),
                is_property_getter: true,
                ..Default::default()
            }],
            ..Default::default()
        }),
        required_interfaces: vec![map_interface],
        ..Default::default()
    };
    let known_types = HashSet::from(["ResourceDictionary".into(), "IMap_Object_Object".into()]);

    let projected = project::project_class(
        &class,
        &known_types,
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let dts = render_dts::render(&projected);

    assert_eq!(
        dts.matches("import { IMap_Object_Object } from './IMap_Object_Object.js';")
            .count(),
        1,
        "{dts}"
    );
    assert!(
        !dts.contains("export declare class IMap_Object_Object"),
        "{dts}"
    );
}

#[test]
fn imported_required_interface_contributes_method_overloads() {
    let rewrite = |with_tone: bool| {
        let mut params = vec![ParamMeta {
            name: "text".into(),
            typ: TypeMeta::String,
            direction: ParamDirection::In,
        }];
        if with_tone {
            params.push(ParamMeta {
                name: "tone".into(),
                typ: TypeMeta::Enum {
                    namespace: "Contoso".into(),
                    name: "Tone".into(),
                    underlying: Box::new(TypeMeta::I32),
                    members: Vec::new(),
                    is_flags: false,
                    doc: None,
                    deprecated: None,
                },
                direction: ParamDirection::In,
            });
        }
        MethodMeta {
            name: "RewriteAsync".into(),
            raw_name: "RewriteAsync".into(),
            params,
            return_type: Some(TypeMeta::AsyncOperation(Box::new(TypeMeta::String))),
            ..Default::default()
        }
    };
    let versioned_iid = "22222222-2222-2222-2222-222222222222";
    let class = ClassMeta {
        name: "Rewriter".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.Rewriter".into(),
        default_interface: Some(InterfaceMeta {
            name: "IRewriter".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
            methods: vec![rewrite(false)],
            ..Default::default()
        }),
        required_interfaces: vec![InterfaceMeta {
            name: "IRewriter2".into(),
            iid: versioned_iid.into(),
            methods: vec![rewrite(true)],
            ..Default::default()
        }],
        ..Default::default()
    };
    let projected = project::project_class(
        &class,
        &HashSet::from(["Rewriter".into(), "IRewriter2".into(), "Tone".into()]),
        &HashSet::new(),
        &HashSet::from([versioned_iid.into()]),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let dts = render_dts::render(&projected);

    assert!(
        dts.contains("rewriteAsync(text: string, signal?: AbortSignal)"),
        "{dts}"
    );
    assert!(
        dts.contains("rewriteAsync(text: string, tone: Tone, signal?: AbortSignal)"),
        "{dts}"
    );
}
