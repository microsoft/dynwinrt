// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::{ir::*, test_support::*, *};
use crate::win32_metadata::*;

fn field(name: &str, typ: RawType, count: Option<usize>) -> RawNativeField {
    RawNativeField {
        name: name.into(),
        typ,
        fixed_count: count,
        bitfield: false,
        flexible_array: false,
    }
}

fn layout(kind: RawLayoutKind, fields: Vec<RawNativeField>) -> RawNativeLayoutSet {
    RawNativeLayoutSet {
        recursive: false,
        evidence: None,
        variants: vec![RawNativeLayout {
            architectures: RawArchitectures {
                x86: true,
                x64: true,
                arm64: true,
            },
            kind,
            packing: RawPacking::Default,
            declared_size: None,
            forced_alignment: None,
            fields,
        }],
    }
}

fn record(name: &str, layout: RawNativeLayoutSet, depth: u8) -> RawType {
    RawType {
        base: RawBaseType::Named {
            namespace: "Tests.Layouts".into(),
            name: name.into(),
            kind: RawNamedKind::NativeStruct {
                layout: Box::new(layout),
            },
        },
        pointer_depth: depth,
        constness: RawConstness::Mutable,
    }
}

fn argument(
    name: &str,
    layout: RawNativeLayoutSet,
    depth: u8,
    direction: RawDirection,
) -> RawFunction {
    let mut function = synthetic_function(name);
    function.parameters.push(parameter(
        "record",
        record("RECORD", layout, depth),
        direction,
    ));
    function
}

fn projected_layout(function: &ProjectedFunction) -> &NativeLayout {
    let InputExpression::NativeAggregate { layout, .. } = &function.inputs[0] else {
        panic!("native aggregate input")
    };
    layout
}

#[test]
fn native_layout_padding_arrays_and_pointer_sized_scalars_match_each_architecture() {
    let fixed = layout(
        RawLayoutKind::Sequential,
        vec![
            field("tag", scalar(RawScalar::U8), None),
            field("wide", scalar(RawScalar::U64), None),
            field("samples", scalar(RawScalar::U16), Some(3)),
        ],
    );
    let projected = project_one(argument("FixedArrayRecord", fixed, 1, RawDirection::In));
    let native = projected_layout(&projected);
    for architecture in [&native.x86, &native.x64, &native.arm64] {
        assert_eq!((architecture.size, architecture.alignment), (24, 8));
        assert_eq!(
            architecture.fields,
            vec![
                NativeField {
                    name: "tag".into(),
                    offset: 0,
                    count: 1,
                    typ: NativeFieldType::Scalar(NativeScalar::U8)
                },
                NativeField {
                    name: "wide".into(),
                    offset: 8,
                    count: 1,
                    typ: NativeFieldType::Scalar(NativeScalar::U64)
                },
                NativeField {
                    name: "samples".into(),
                    offset: 16,
                    count: 3,
                    typ: NativeFieldType::Scalar(NativeScalar::U16)
                },
            ]
        );
    }
    let sized = layout(
        RawLayoutKind::Sequential,
        vec![
            field("tag", scalar(RawScalar::U8), None),
            field("word", scalar(RawScalar::NativeUsize), None),
            field("tail", scalar(RawScalar::U16), None),
        ],
    );
    let projected = project_one(argument("NativeWidthRecord", sized, 0, RawDirection::In));
    let native = projected_layout(&projected);
    assert!(native.by_value_compatible);
    for (architecture, size, alignment, offsets) in [
        (&native.x86, 12, 4, [0, 4, 8]),
        (&native.x64, 24, 8, [0, 8, 16]),
        (&native.arm64, 24, 8, [0, 8, 16]),
    ] {
        assert_eq!(
            (architecture.size, architecture.alignment),
            (size, alignment)
        );
        assert_eq!(
            architecture
                .fields
                .iter()
                .map(|f| f.offset)
                .collect::<Vec<_>>(),
            offsets
        );
    }
    assert_eq!(
        projected.runtime.parameters[0].aggregate.as_ref(),
        Some(native)
    );
    assert!(matches!(
        projected.inputs[0],
        InputExpression::NativeAggregate { by_value: true, .. }
    ));
}

#[test]
fn union_members_overlap_but_union_values_cannot_be_passed_by_value() {
    let union = layout(
        RawLayoutKind::Union,
        vec![
            field("small", scalar(RawScalar::U32), None),
            field("large", scalar(RawScalar::U64), None),
        ],
    );
    let nested = layout(
        RawLayoutKind::Sequential,
        vec![
            field("tag", scalar(RawScalar::U8), None),
            field("payload", record("PAYLOAD", union.clone(), 0), Some(2)),
        ],
    );
    for direction in [RawDirection::In, RawDirection::Out, RawDirection::InOut] {
        let projected = project_one(argument("UnionPointer", union.clone(), 1, direction));
        let native = projected_layout(&projected);
        assert_eq!(native.kind, NativeAggregateKind::Union);
        assert_eq!((native.x64.size, native.x64.alignment), (8, 8));
        assert_eq!(
            native
                .x64
                .fields
                .iter()
                .map(|f| f.offset)
                .collect::<Vec<_>>(),
            [0, 0]
        );
        assert_eq!(
            projected.parameters[0].typ,
            SurfaceType::NativeUnion("RECORD".into())
        );
        assert_eq!(projected.runtime.parameters[0].direction, Direction::In);
        assert_eq!(
            projected.return_shape,
            ReturnShape::Direct {
                typ: SurfaceType::Number,
                conversion: Conversion::Number,
                may_be_unavailable: false,
            }
        );
    }
    let projected = project_one(argument(
        "NestedUnionPointer",
        nested.clone(),
        1,
        RawDirection::In,
    ));
    let native = projected_layout(&projected);
    assert_eq!((native.x64.size, native.x64.alignment), (24, 8));
    assert_eq!(native.x64.fields[1].offset, 8);
    assert_eq!(native.x64.fields[1].count, 2);
    assert!(
        matches!(&native.x64.fields[1].typ,NativeFieldType::Union { layout, .. }
        if layout.size == 8 && layout.fields.iter().all(|f|f.offset == 0))
    );
    for shape in [union, nested] {
        let function = argument("UnsupportedUnionValue", shape, 0, RawDirection::In);
        let (generated, omitted) =
            generate_apis_files(&apis(vec![function]), "@test/runtime/win32");
        assert_eq!(omitted.len(), 1);
        assert!(omitted[0].reason.contains("union"));
        assert!(!generated.js.contains("exports.unsupportedUnionValue"));
    }
}

#[test]
fn packing_forced_alignment_and_declared_tail_padding_are_not_guessed() {
    for (packing, forced, declared, size, alignment, second_offset, by_value) in [
        (RawPacking::Default, None, None, 8, 4, 4, true),
        (RawPacking::Explicit(1), None, None, 5, 1, 1, false),
        (RawPacking::Explicit(2), None, None, 6, 2, 2, false),
        (RawPacking::Default, Some(8), None, 8, 8, 4, false),
        (RawPacking::Default, None, Some(12), 12, 4, 4, true),
    ] {
        let mut shape = layout(
            RawLayoutKind::Sequential,
            vec![
                field("first", scalar(RawScalar::U8), None),
                field("second", scalar(RawScalar::U32), None),
            ],
        );
        shape.variants[0].packing = packing;
        shape.variants[0].forced_alignment = forced;
        shape.variants[0].declared_size = declared;
        let projected = project_one(argument(
            "PackedPointer",
            shape.clone(),
            1,
            RawDirection::In,
        ));
        let native = projected_layout(&projected);
        assert_eq!((native.x64.size, native.x64.alignment), (size, alignment));
        assert_eq!(native.x64.fields[1].offset, second_offset);
        let projection = project_apis(&apis(vec![argument(
            "PackedValue",
            shape,
            0,
            RawDirection::In,
        )]));
        assert_eq!(
            projection.omitted.is_empty(),
            by_value,
            "{packing:?} / {forced:?}"
        );
    }
}

#[test]
fn invalid_layout_facts_fail_before_emitting_a_native_wrapper() {
    let base = layout(
        RawLayoutKind::Sequential,
        vec![field("value", scalar(RawScalar::U64), None)],
    );
    let cases: &[(&str, fn(&mut RawNativeLayoutSet))] = &[
        ("recursive", |l| l.recursive = true),
        ("unknown", |l| l.variants[0].kind = RawLayoutKind::Unknown),
        ("empty", |l| l.variants[0].fields.clear()),
        ("packing", |l| {
            l.variants[0].packing = RawPacking::Explicit(3)
        }),
        ("alignment", |l| l.variants[0].forced_alignment = Some(16)),
        ("alignment", |l| l.variants[0].forced_alignment = Some(3)),
        ("declared", |l| l.variants[0].declared_size = Some(4)),
        ("declared", |l| l.variants[0].declared_size = Some(9)),
        ("zero", |l| l.variants[0].fields[0].fixed_count = Some(0)),
        ("exceeds u32", |l| {
            l.variants[0].fields[0].fixed_count = Some(usize::MAX)
        }),
        ("flexible", |l| {
            l.variants[0].fields[0].flexible_array = true
        }),
        ("ambiguous", |l| l.variants.push(l.variants[0].clone())),
        ("missing", |l| l.variants[0].architectures.arm64 = false),
    ];
    for (reason, mutate) in cases {
        let mut shape = base.clone();
        mutate(&mut shape);
        let (generated, omitted) = generate_apis_files(
            &apis(vec![argument("InvalidLayout", shape, 1, RawDirection::In)]),
            "@test/runtime/win32",
        );
        assert_eq!(omitted.len(), 1, "{reason}");
        assert!(
            omitted[0].reason.contains(reason),
            "{reason}: {:?}",
            omitted[0]
        );
        assert!(!generated.js.contains("exports.invalidLayout"));
    }
}

#[test]
fn scalar_and_guid_pointer_contracts_preserve_constness_width_and_storage_direction() {
    for (kind, expected_abi, expected_surface) in [
        (RawScalar::I16, AbiType::I16, SurfaceType::Number),
        (RawScalar::U64, AbiType::U64, SurfaceType::BigInt),
        (RawScalar::Bool32, AbiType::Bool32, SurfaceType::Boolean),
    ] {
        for direction in [RawDirection::In, RawDirection::Out, RawDirection::InOut] {
            let mut function = synthetic_function("PointerValue");
            let constness = if direction == RawDirection::In {
                RawConstness::Const
            } else {
                RawConstness::Mutable
            };
            function.parameters.push(parameter(
                "value",
                pointer(scalar(kind), 1, constness),
                direction,
            ));
            let native = semantic(&function);
            assert_eq!(native.parameters[0].pointer_depth, 1);
            assert_eq!(
                native.parameters[0].constness,
                if direction == RawDirection::In {
                    Constness::Const
                } else {
                    Constness::Mutable
                }
            );
            let projected = project_one(function);
            if direction == RawDirection::In {
                assert_eq!(projected.parameters[0].typ, expected_surface);
                assert!(matches!(
                    projected.inputs[0],
                    InputExpression::ScalarPointer {
                        parameter_index: 0,
                        nullable: false,
                        ..
                    }
                ));
                assert_eq!(projected.runtime.parameters[0].abi, AbiType::Pointer);
                assert_eq!(projected.runtime.parameters[0].direction, Direction::In);
            } else {
                assert_eq!(projected.runtime.parameters[0].abi, expected_abi);
                assert_eq!(
                    projected.runtime.parameters[0].direction,
                    if direction == RawDirection::Out {
                        Direction::Out
                    } else {
                        Direction::InOut
                    }
                );
                let ReturnShape::Object { outputs, .. } = &projected.return_shape else {
                    panic!("output cell")
                };
                assert_eq!(outputs[0].output_index, 0);
                assert_eq!(outputs[0].typ, expected_surface);
            }
        }
    }
    for direction in [RawDirection::In, RawDirection::Out, RawDirection::InOut] {
        let guid = RawType {
            base: RawBaseType::Named {
                namespace: "System".into(),
                name: "Guid".into(),
                kind: RawNamedKind::Guid,
            },
            pointer_depth: 1,
            constness: RawConstness::Const,
        };
        let mut function = synthetic_function("GuidPointer");
        let mut guid = parameter("guid", guid, direction);
        guid.nullable = true;
        function.parameters.push(guid);
        let projected = project_one(function);
        assert_eq!(
            projected.parameters[0],
            SurfaceParameter {
                name: "guid".into(),
                typ: SurfaceType::Buffer,
                nullable: true,
                minimum_bytes: Some(16),
                alignment: Some(4),
            }
        );
        assert_eq!(projected.runtime.parameters[0].direction, Direction::In);
        assert_eq!(projected.runtime.parameters[0].abi, AbiType::Pointer);
        assert_eq!(
            projected.inputs,
            vec![InputExpression::Surface {
                parameter_index: 0,
                conversion: Conversion::DataPointer
            }]
        );
    }
}
