// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{HashMap, HashSet};

use dynwinrt_codegen::codegen::winrt::javascript::parameterized_name;
use dynwinrt_codegen::codegen::{project, render_dts, render_js};
use dynwinrt_codegen::meta::{
    ClassMeta, ConstructorKind, ConstructorMeta, InterfaceMeta, MethodMeta, ParamDirection,
    ParamMeta,
};
use dynwinrt_codegen::types::{FieldMeta, TypeKind, TypeMeta, TypeRef};

const COLLECTIONS: &str = "Windows.Foundation.Collections";
const VECTOR: &str = "913337e9-11a1-4345-a3a2-4e7f956e222d";

fn collection(name: &str, piid: &str, args: Vec<TypeMeta>) -> TypeMeta {
    TypeMeta::Parameterized {
        namespace: COLLECTIONS.into(),
        name: name.into(),
        piid: piid.into(),
        args,
    }
}

fn param(name: &str, typ: TypeMeta) -> ParamMeta {
    ParamMeta {
        name: name.into(),
        typ,
        direction: ParamDirection::In,
    }
}

fn render_method(typ: TypeMeta) -> (String, String) {
    let interface = InterfaceMeta {
        namespace: "Tests".into(),
        name: "IProbe".into(),
        iid: "ce717198-69ad-4f8b-a282-9e8da4d50e1a".into(),
        methods: vec![MethodMeta {
            name: "Take".into(),
            raw_name: "Take".into(),
            vtable_index: 6,
            params: vec![
                param("before", TypeMeta::U32),
                param("value", typ),
                param("after", TypeMeta::String),
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    let projected = project::project_interface(
        &Default::default(),
        &interface,
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    (
        render_js::render(&projected),
        render_dts::render(&projected),
    )
}

#[test]
fn collection_null_projection_uses_piid_and_arity_not_spelling() {
    for (piid, args, native) in [
        (VECTOR, vec![TypeMeta::U32], "number[]"),
        (
            "bbe1fa4c-b0e3-4583-baef-1f1b2e483e56",
            vec![TypeMeta::U32],
            "number[]",
        ),
        (
            "faa585ea-6214-4217-afda-7f46de5869b3",
            vec![TypeMeta::U32],
            "number[]",
        ),
        (
            "3c2925fe-8519-45c1-aa79-197b6718c1c1",
            vec![TypeMeta::String, TypeMeta::U32],
            "Map<string, number>",
        ),
        (
            "e480ce40-a338-4ada-adcf-272272e48cb9",
            vec![TypeMeta::String, TypeMeta::U32],
            "Map<string, number>",
        ),
    ] {
        for identity in [piid.to_string(), piid.to_uppercase()] {
            let name = parameterized_name(COLLECTIONS, "Renamed", &identity, &args);
            let (js, dts) = render_method(collection("Renamed", &identity, args.clone()));
            assert!(
                dts.contains(&format!(
                    "take(before: number, value: {name} | {native} | null, after: string): void;"
                )),
                "{dts}"
            );
            assert!(
                js.contains("value === null ? DynWinRtValue.nullValue()"),
                "{js}"
            );
            assert!(
                js.contains("value instanceof DynWinRtValue && value.isNull()"),
                "{js}"
            );
            assert!(
                js.contains("value.cast(DynWinRtType.parameterized("),
                "{js}"
            );
            assert_eq!(js.matches("_unwrap(value)").count(), 1, "{js}");
            assert!(!dts.contains("value?:"));
        }
    }

    for typ in [
        collection("IVector", "", vec![TypeMeta::U32]),
        collection(
            "IVector",
            "ce717198-69ad-4f8b-a282-9e8da4d50e1a",
            vec![TypeMeta::U32],
        ),
        collection("IVector", VECTOR, vec![TypeMeta::U32, TypeMeta::U32]),
        collection("IVector", VECTOR, vec![]),
        collection(
            "IObservableVector",
            "5917eb53-50b4-4a0d-b309-65862b3f1dbc",
            vec![TypeMeta::U32],
        ),
        TypeMeta::Array(Box::new(TypeMeta::U32)),
        TypeMeta::String,
        TypeMeta::U32,
    ] {
        let (js, dts) = render_method(typ);
        let declaration = dts
            .split("export declare class IProbe")
            .nth(1)
            .unwrap()
            .lines()
            .find(|line| line.contains("take("))
            .unwrap();
        assert!(!declaration.contains("null"), "{declaration}");
        assert!(
            !js.contains("value === null ? DynWinRtValue.nullValue()"),
            "{js}"
        );
    }
}

#[test]
fn hint_shaped_factory_and_constructor_keep_three_required_nullable_slots() {
    let geometry = |name: &str, fields: &[&str]| TypeMeta::Struct {
        namespace: "Windows.Graphics".into(),
        name: name.into(),
        fields: fields
            .iter()
            .map(|field| FieldMeta {
                name: (*field).into(),
                typ: TypeMeta::I32,
            })
            .collect(),
    };
    let rect = geometry("RectInt32", &["X", "Y", "Width", "Height"]);
    let point = geometry("PointInt32", &["X", "Y"]);
    let factory = InterfaceMeta {
        namespace: "Tests".into(),
        name: "IHintFactory".into(),
        iid: "17cd68ba-e9b5-4c2e-8f0d-22c9bc75b5b4".into(),
        methods: vec![MethodMeta {
            name: "CreateInstance".into(),
            raw_name: "CreateInstance".into(),
            vtable_index: 6,
            params: [
                ("includeRects", rect.clone()),
                ("includePoints", point.clone()),
                ("excludePoints", point.clone()),
            ]
            .into_iter()
            .map(|(name, typ)| param(name, collection("IVector", VECTOR, vec![typ])))
            .collect(),
            return_type: Some(TypeMeta::RuntimeClass {
                namespace: "Tests".into(),
                name: "Hint".into(),
                default_interface: Some(Box::new(TypeMeta::Interface {
                    namespace: "Tests".into(),
                    name: "IHint".into(),
                    iid: "fd0752d2-ec78-4fb3-825e-9b9412fe1b55".into(),
                })),
            }),
            ..Default::default()
        }],
        ..Default::default()
    };
    let class = ClassMeta {
        namespace: "Tests".into(),
        name: "Hint".into(),
        full_name: "Tests.Hint".into(),
        default_interface: Some(InterfaceMeta {
            namespace: "Tests".into(),
            name: "IHint".into(),
            iid: "fd0752d2-ec78-4fb3-825e-9b9412fe1b55".into(),
            ..Default::default()
        }),
        factory_interfaces: vec![factory],
        constructors: vec![ConstructorMeta {
            kind: ConstructorKind::FactoryActivation,
            factory_interface: Some(TypeRef {
                namespace: "Tests".into(),
                name: "IHintFactory".into(),
                kind: TypeKind::Interface,
            }),
        }],
        ..Default::default()
    };
    let projected = project::project_class(
        &Default::default(),
        &class,
        &HashSet::from(["Hint".into()]),
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let js = render_js::render(&projected);
    let dts = render_dts::render(&projected);
    let rect_name = parameterized_name(COLLECTIONS, "IVector", VECTOR, &[rect]);
    let point_name = parameterized_name(COLLECTIONS, "IVector", VECTOR, &[point]);
    let params = format!(
        "includeRects: {rect_name} | RectInt32[] | null, \
         includePoints: {point_name} | PointInt32[] | null, \
         excludePoints: {point_name} | PointInt32[] | null"
    );
    assert!(dts.contains(&format!("constructor({params});")), "{dts}");
    assert!(
        dts.contains(&format!("static createInstance({params}): Hint;")),
        "{dts}"
    );
    assert!(js.contains("if (args.length === 3)"), "{js}");
    assert!(
        js.contains("Hint.createInstance(args[0], args[1], args[2])"),
        "{js}"
    );
    for name in ["includeRects", "includePoints", "excludePoints"] {
        assert!(
            js.contains(&format!("{name} === null ? DynWinRtValue.nullValue()")),
            "{js}"
        );
        assert!(
            js.contains(&format!(
                "Array.isArray({name}) ? DynWinRtValue.createVector("
            )),
            "{js}"
        );
    }
    assert!(!js.contains(".length === 0"));
    assert!(!dts.contains("(PointInt32 | null)[]"));
    assert!(!dts.contains("(RectInt32 | null)[]"));
}
