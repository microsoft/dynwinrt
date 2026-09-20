// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{HashMap, HashSet};

use dynwinrt_codegen::codegen::{project, render_dts, render_js};
use dynwinrt_codegen::meta::{InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::{FieldMeta, TypeMeta};

const VECTOR: &str = "913337e9-11a1-4345-a3a2-4e7f956e222d";
const OBSERVABLE_VECTOR: &str = "5917eb53-50b4-4a0d-b309-65862b3f1dbc";
const MAP: &str = "3c2925fe-8519-45c1-aa79-197b6718c1c1";

fn project_factory(piid: &str, args: Vec<TypeMeta>) -> (String, String) {
    let interface = InterfaceMeta {
        name: "Collection".into(),
        namespace: "Windows.Foundation.Collections".into(),
        generic_piid: Some(piid.into()),
        // Include ordinary typed inputs so struct helpers and class IIDs are
        // collected exactly as they are for real Append/Insert methods.
        methods: vec![MethodMeta {
            name: "Insert".into(),
            raw_name: "Insert".into(),
            vtable_index: 6,
            params: args
                .iter()
                .enumerate()
                .map(|(i, typ)| ParamMeta {
                    name: format!("arg{i}"),
                    typ: typ.clone(),
                    direction: ParamDirection::In,
                })
                .collect(),
            ..Default::default()
        }],
        generic_args: args,
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
fn collection_factories_wrap_declared_scalar_types_in_every_role() {
    for (typ, constructor) in [
        (TypeMeta::String, "hstring"),
        (TypeMeta::Bool, "boolValue"),
        (TypeMeta::I8, "i32"),
        (TypeMeta::U8, "i32"),
        (TypeMeta::Char16, "i32"),
        (TypeMeta::I16, "i16"),
        (TypeMeta::U16, "u16"),
        (TypeMeta::I32, "i32"),
        (TypeMeta::U32, "u32"),
        (TypeMeta::I64, "i64"),
        (TypeMeta::U64, "u64"),
        // Conversion does not imply admission by the native producer.
        (TypeMeta::F32, "f32"),
        (TypeMeta::F64, "f64"),
    ] {
        for piid in [VECTOR, OBSERVABLE_VECTOR] {
            let (js, _) = project_factory(piid, vec![typ.clone()]);
            assert!(
                js.contains(&format!("items.map(i => DynWinRtValue.{constructor}(i))")),
                "{typ:?}: {js}"
            );
        }
        let (js, _) = project_factory(MAP, vec![typ.clone(), TypeMeta::String]);
        assert!(js.contains(&format!("keys.map(k => DynWinRtValue.{constructor}(k))")));
        assert!(js.contains("values.map(v => DynWinRtValue.hstring(v))"));
        let (js, _) = project_factory(MAP, vec![TypeMeta::String, typ]);
        assert!(js.contains("keys.map(k => DynWinRtValue.hstring(k))"));
        assert!(js.contains(&format!("values.map(v => DynWinRtValue.{constructor}(v))")));
    }
}

#[test]
fn collection_factories_preserve_projected_references_and_struct_packing() {
    let interface = TypeMeta::Interface {
        namespace: "Windows.Foundation".into(),
        name: "IStringable".into(),
        iid: "96369f54-8eb6-48f0-abce-c1b211e627c3".into(),
    };
    let reference = TypeMeta::Parameterized {
        namespace: "Windows.Foundation".into(),
        name: "IReference`1".into(),
        piid: "61c17706-2d65-11e0-9ae8-d48564015472".into(),
        args: vec![TypeMeta::U32],
    };
    for typ in [TypeMeta::Object, interface] {
        let (js, _) = project_factory(VECTOR, vec![typ.clone()]);
        assert!(
            js.contains("items.map(i => (i == null ? DynWinRtValue.nullValue() : _unwrap(i)))")
        );
        let (js, _) = project_factory(MAP, vec![TypeMeta::String, typ]);
        assert!(
            js.contains("values.map(v => (v == null ? DynWinRtValue.nullValue() : _unwrap(v)))")
        );
    }
    for (piid, args) in [
        (VECTOR, vec![reference.clone()]),
        (OBSERVABLE_VECTOR, vec![reference.clone()]),
        (MAP, vec![reference.clone(), reference]),
    ] {
        let (js, _) = project_factory(piid, args);
        let factory = js.split("static create(").nth(1).unwrap();
        assert!(factory.contains("value == null ? DynWinRtValue.nullValue()"));
        assert!(factory.contains("value instanceof DynWinRtValue ? value : value?._obj"));
        assert!(
            factory.contains(
                "DynWinRtValue.boxReference(DynWinRtValue.u32(value), DynWinRtType.u32())"
            )
        );
    }

    for (typ, expression) in [
        (
            TypeMeta::Enum {
                namespace: "Windows.Foundation".into(),
                name: "PropertyType".into(),
                underlying: Box::new(TypeMeta::I32),
                members: vec![],
                is_flags: false,
                doc: None,
                deprecated: None,
            },
            "DynWinRtValue.i32(i)",
        ),
        (
            TypeMeta::Struct {
                namespace: "Windows.Foundation".into(),
                name: "DateTime".into(),
                fields: vec![FieldMeta {
                    name: "UniversalTime".into(),
                    typ: TypeMeta::I64,
                }],
            },
            "_packDateTime(i).toValue()",
        ),
        (
            TypeMeta::Struct {
                namespace: "Windows.Foundation".into(),
                name: "HResult".into(),
                fields: vec![],
            },
            "DynWinRtValue.hresult(i)",
        ),
        (TypeMeta::Guid, "DynWinRtValue.guid(WinGuid.parse(i))"),
    ] {
        for piid in [VECTOR, OBSERVABLE_VECTOR] {
            let (js, _) = project_factory(piid, vec![typ.clone()]);
            assert!(
                js.contains(&format!("items.map(i => {expression})")),
                "{js}"
            );
        }
        let (js, _) = project_factory(MAP, vec![typ.clone(), typ]);
        assert!(js.contains(&format!(
            "keys.map(k => {})",
            expression.replace("(i)", "(k)")
        )));
        assert!(js.contains(&format!(
            "values.map(v => {})",
            expression.replace("(i)", "(v)")
        )));
    }
}

#[test]
fn string_collection_factory_declarations_remain_native_js_arrays() {
    let (_, dts) = project_factory(VECTOR, vec![TypeMeta::String]);
    assert!(dts.contains("static create(items: string[]): Collection;"));
    let (_, dts) = project_factory(MAP, vec![TypeMeta::String, TypeMeta::String]);
    assert!(dts.contains("static create(keys: string[], values: string[]): Collection;"));
}

#[test]
fn reference_factories_preserve_managed_nulls_before_querying_every_role() {
    let class = TypeMeta::RuntimeClass {
        namespace: "Windows.ApplicationModel.Contacts".into(),
        name: "ContactDate".into(),
        default_interface: Some(Box::new(TypeMeta::Interface {
            namespace: "Windows.ApplicationModel.Contacts".into(),
            name: "IContactDate".into(),
            iid: "fe98ae66-b205-4934-9174-0ff2b0565707".into(),
        })),
    };
    let collection_type = |name: &str, piid: &str, args| TypeMeta::Parameterized {
        namespace: "Windows.Foundation.Collections".into(),
        name: name.into(),
        piid: piid.into(),
        args,
    };
    for typ in [
        class.clone(),
        collection_type("IVector", VECTOR, vec![class.clone()]),
        collection_type("IMap", MAP, vec![class.clone(), class]),
    ] {
        for (piid, args, variables) in [
            (VECTOR, vec![typ.clone()], vec!["i"]),
            (OBSERVABLE_VECTOR, vec![typ.clone()], vec!["i"]),
            (MAP, vec![typ.clone(), typ.clone()], vec!["k", "v"]),
        ] {
            let (js, _) = project_factory(piid, args);
            let factory = js.split("static create(").nth(1).unwrap();
            let guard = "value instanceof DynWinRtValue && value.isNull() ? value : value.cast(";
            assert!(factory.contains(guard), "{factory}");
            for var in variables {
                let unwrap = format!("_unwrap({var})");
                assert_eq!(factory.matches(&unwrap).count(), 1, "{factory}");
                assert!(factory.contains(&format!("))({unwrap})")), "{factory}");
            }
            // Class elements in automatically converted arrays/maps must use
            // the same null-preserving expected-IID query as direct inputs.
            if matches!(typ, TypeMeta::Parameterized { .. }) {
                assert!(factory.contains(&format!(
                    "{guard}IID_ARG_Windows_ApplicationModel_Contacts_ContactDate)"
                )));
            }
        }
    }
}
