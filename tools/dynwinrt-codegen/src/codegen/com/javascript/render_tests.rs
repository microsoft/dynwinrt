// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use crate::codegen::com::ir::{ComPrimitive, ComScalarRepr};
use crate::codegen::com::javascript::naming::{camel_case, strip_hungarian};
use crate::codegen::com::project::{
    project_com_interface_for_test as project_com_interface, project_type_for_test as project_type,
};
use crate::com_metadata::{
    ComEnumMeta, ComEnumValue, ComInterfaceMeta, MethodMeta, ParamDirection, ParamMeta,
};
use crate::types::TypeMeta;

type HandleAliasKind = PointerAliasKind;

fn generate_com_interface_files(
    meta: &ComInterfaceMeta,
    winmd_paths: &str,
) -> Result<super::ComGeneratedOutput, String> {
    let projected = project_com_interface(meta, winmd_paths)?;
    Ok(render_com_interface(&projected))
}

#[test]
fn renderer_api_accepts_only_projected_ir() {
    let projected = ProjectedComInterface {
        name: "ITest".into(),
        namespace: "Tests".into(),
        iid: "00000000-0000-0000-0000-000000000001".into(),
        is_iunknown_rooted: true,
        methods: Vec::new(),
        activation: ActivationPlan::None,
        referenced_enums: Vec::new(),
    };
    let output = render_com_interface(&projected);
    assert!(output.js.contains("registerIUnknownInterface"));
    assert!(output.dts.contains("export declare class ITest"));
    assert!(
        output
            .dts
            .contains("import type { WinGuid } from '@microsoft/dynwinrt/com';")
    );
    assert!(
        output
            .dts
            .contains("export declare const IID_ITest: WinGuid;")
    );
}

#[test]
fn renderer_projects_borrowed_hwnd_output_as_numeric_handle() {
    let hwnd = ComType::PointerAlias {
        namespace: "Windows.Win32.Foundation".into(),
        name: "HWND".into(),
        kind: PointerAliasKind::HandleValue,
    };
    let projected = ProjectedComInterface {
        name: "IWindowOwner".into(),
        namespace: "Tests".into(),
        iid: "00000000-0000-0000-0000-000000000001".into(),
        is_iunknown_rooted: true,
        methods: vec![ProjectedComMethod {
            name: "GetWindow".into(),
            camel_name: "getWindow".into(),
            vtable_index: 3,
            params: vec![ProjectedComParam {
                name: "phwnd".into(),
                typ: hwnd.clone(),
                direction: ComParamDirection::Out,
                surface_input: false,
                surface_result: true,
                nullable: false,
            }],
            return_convention: ComReturnConvention::HResult,
            results: vec![ProjectedComResult {
                typ: hwnd,
                source: ResultSource::Param(0),
                conversion: ResultConversion::BorrowedHandle,
            }],
            string_buffer: None,
            typed_buffers: Vec::new(),
            shared_counts: Vec::new(),
            kind: ProjectedComMethodKind::Normal,
            doc: None,
            overload: None,
        }],
        activation: ActivationPlan::None,
        referenced_enums: Vec::new(),
    };

    let output = render_com_interface(&projected);
    assert!(
        output
            .js
            .contains(".addOut(DynCom.borrowedHandleOutputType())")
    );
    assert!(output.js.contains("return DynCom.asPointerBigint(_out);"));
    assert!(output.dts.contains("getWindow(): HWND;"));
    assert!(!output.dts.contains("getWindow(): Buffer"));
    assert!(!output.js.contains("adoptComPointer"));
}

#[test]
fn canonical_iunknown_arrays_use_managed_values_without_nominal_wrappers() {
    let array = ComType::OwningArray {
        element: Box::new(ComType::ManagedInterface {
            iid: "00000000-0000-0000-c000-000000000046".into(),
        }),
        interface: None,
    };
    assert_eq!(type_dts(&array), "DynWinRtValue[]");
    assert_eq!(
        super::super::types::wrap_arg_js(&array, "values"),
        "DynCom.interfaceArray(WinGuid.parse('00000000-0000-0000-c000-000000000046'), values)"
    );
    let result = ProjectedComResult {
        typ: array,
        source: ResultSource::Param(0),
        conversion: ResultConversion::OwningArray { interface: None },
    };
    assert_eq!(result_type_dts(&result), "DynWinRtValue[]");
    assert_eq!(
        unwrap_result_js(&result, "_out"),
        "Array.from(DynCom.takeComArray(_out))"
    );

    let projected = ProjectedComInterface {
        name: "IUnknownArray".into(),
        namespace: "Tests".into(),
        iid: "00000000-0000-0000-0000-000000000001".into(),
        is_iunknown_rooted: true,
        methods: vec![ProjectedComMethod {
            name: "GetValues".into(),
            camel_name: "getValues".into(),
            vtable_index: 3,
            params: vec![ProjectedComParam {
                name: "values".into(),
                typ: result.typ.clone(),
                direction: ComParamDirection::Out,
                surface_input: false,
                surface_result: true,
                nullable: false,
            }],
            results: vec![result],
            return_convention: ComReturnConvention::HResult,
            string_buffer: None,
            typed_buffers: Vec::new(),
            shared_counts: Vec::new(),
            kind: ProjectedComMethodKind::Normal,
            doc: None,
            overload: None,
        }],
        activation: ActivationPlan::None,
        referenced_enums: Vec::new(),
    };
    let output = render_com_interface(&projected);
    assert!(
        output
            .dts
            .contains("import type { DynWinRtValue } from '@microsoft/dynwinrt/com';")
    );
}

#[test]
fn renderer_projects_bstr_replacement_as_a_string_roundtrip() {
    let projected = ProjectedComInterface {
        name: "IReplaceText".into(),
        namespace: "Tests".into(),
        iid: "00000000-0000-0000-0000-000000000001".into(),
        is_iunknown_rooted: true,
        methods: vec![ProjectedComMethod {
            name: "Replace".into(),
            camel_name: "replace".into(),
            vtable_index: 3,
            params: vec![ProjectedComParam {
                name: "value".into(),
                typ: ComType::Bstr,
                direction: ComParamDirection::InOut,
                surface_input: true,
                surface_result: true,
                nullable: false,
            }],
            return_convention: ComReturnConvention::HResult,
            results: vec![ProjectedComResult {
                typ: ComType::Bstr,
                source: ResultSource::Param(0),
                conversion: ResultConversion::Bstr,
            }],
            string_buffer: None,
            typed_buffers: Vec::new(),
            shared_counts: Vec::new(),
            kind: ProjectedComMethodKind::Normal,
            doc: None,
            overload: None,
        }],
        activation: ActivationPlan::None,
        referenced_enums: Vec::new(),
    };
    let output = render_com_interface(&projected);

    assert!(
        output
            .js
            .contains("new DynComMethodSig().addInOut(DynCom.bstrType())")
    );
    assert!(output.js.contains("DynCom.bstr(value)"));
    assert!(output.js.contains("DynCom.takeBstr("));
    assert!(output.dts.contains("replace(value: string): string;"));
    assert!(!output.js.contains("Buffer"));
    assert!(!output.dts.contains("bigint"));
}

#[test]
fn renderer_allows_null_only_for_nullable_bstr_inputs() {
    let projected = ProjectedComInterface {
        name: "IOptionalText".into(),
        namespace: "Tests".into(),
        iid: "00000000-0000-0000-0000-000000000001".into(),
        is_iunknown_rooted: true,
        methods: vec![ProjectedComMethod {
            name: "SetOptional".into(),
            camel_name: "setOptional".into(),
            vtable_index: 3,
            params: vec![ProjectedComParam {
                name: "value".into(),
                typ: ComType::Bstr,
                direction: ComParamDirection::In,
                surface_input: true,
                surface_result: false,
                nullable: true,
            }],
            return_convention: ComReturnConvention::HResult,
            results: Vec::new(),
            string_buffer: None,
            typed_buffers: Vec::new(),
            shared_counts: Vec::new(),
            kind: ProjectedComMethodKind::Normal,
            doc: None,
            overload: None,
        }],
        activation: ActivationPlan::None,
        referenced_enums: Vec::new(),
    };
    let output = render_com_interface(&projected);

    assert!(
        output
            .js
            .contains("new DynComMethodSig().addIn(DynCom.nullableBstrType())")
    );
    assert!(
        output
            .js
            .contains("value === null ? DynCom.nullBstr() : DynCom.bstr(value)")
    );
    assert!(
        output
            .dts
            .contains("setOptional(value: string | null): void;")
    );
}

#[test]
fn real_automation_bstr_interfaces_render_natural_string_apis() {
    let Some(winmd) = std::env::var("DYNWINRT_WIN32_WINMD")
        .ok()
        .filter(|path| std::path::Path::new(path).exists())
    else {
        return;
    };
    for (namespace, interface_name) in [
        ("Windows.Win32.Web.InternetExplorer", "ITargetNotify2"),
        ("Windows.Win32.Storage.Imapi", "IDiscRecorder"),
        ("Windows.Win32.System.TaskScheduler", "IExecAction2"),
    ] {
        let meta =
            crate::com_metadata::parse_com_interface(&winmd, namespace, interface_name).unwrap();
        let projected = crate::codegen::com::project::project_com_interface(&meta, &winmd)
            .unwrap_or_else(|error| panic!("{namespace}.{interface_name}: {error}"));
        let output = render_com_interface(&projected);
        assert!(output.js.contains("DynCom.bstrType()"));
        assert!(output.js.contains("DynCom.takeBstr("));
        assert!(output.dts.contains("string"));
        if interface_name == "IDiscRecorder" {
            assert!(
                output
                    .dts
                    .contains("getDisplayNames(): [string, string, string];")
            );
        }
    }
}

#[test]
fn renderer_keeps_dynamic_iid_native_order_and_all_results() {
    let input = |name: &str, typ| ProjectedComParam {
        name: name.into(),
        typ,
        direction: ComParamDirection::In,
        surface_input: true,
        surface_result: false,
        nullable: false,
    };
    let output = |name: &str, typ| ProjectedComParam {
        name: name.into(),
        typ,
        direction: ComParamDirection::Out,
        surface_input: false,
        surface_result: true,
        nullable: false,
    };
    let method = ProjectedComMethod {
        name: "Resolve".into(),
        camel_name: "resolve".into(),
        vtable_index: 7,
        params: vec![
            input("context", ComType::Primitive(ComPrimitive::U32)),
            output("status", ComType::Primitive(ComPrimitive::I32)),
            input("riid", ComType::RawPointer),
            output("object", ComType::RawPointer),
            input("flags", ComType::Primitive(ComPrimitive::U16)),
            output("cookie", ComType::Primitive(ComPrimitive::U64)),
        ],
        return_convention: ComReturnConvention::HResult,
        results: vec![
            ProjectedComResult {
                typ: ComType::Primitive(ComPrimitive::I32),
                source: ResultSource::Param(1),
                conversion: ResultConversion::Value,
            },
            ProjectedComResult {
                typ: ComType::RawPointer,
                source: ResultSource::Param(3),
                conversion: ResultConversion::DynamicIidAdoption,
            },
            ProjectedComResult {
                typ: ComType::Primitive(ComPrimitive::U64),
                source: ResultSource::Param(5),
                conversion: ResultConversion::Value,
            },
        ],
        string_buffer: None,
        typed_buffers: Vec::new(),
        shared_counts: Vec::new(),
        kind: ProjectedComMethodKind::CallerSuppliedDynamicIid {
            iid_param_index: 2,
            output_param_index: 3,
        },
        doc: None,
        overload: None,
    };
    let output = render_com_interface(&ProjectedComInterface {
        name: "IResolve".into(),
        namespace: "Tests".into(),
        iid: "00000000-0000-0000-0000-000000000010".into(),
        is_iunknown_rooted: true,
        methods: vec![method],
        activation: ActivationPlan::None,
        referenced_enums: Vec::new(),
    });

    assert!(output.js.contains(
        ".addIn(DynCom.u32Type()).addOut(DynCom.i32Type()).addIn(DynCom.pointerType()).addOut(DynCom.ownedComPointerType()).addIn(DynCom.u16Type()).addOut(DynCom.u64Type())"
    ));
    assert!(output.js.contains("resolve(context, riid, flags)"));
    assert!(output.js.contains("const _iid = WinGuid.parse(riid)"));
    assert!(output.js.contains(
        "invokeAll(this._obj, [DynCom.u32(context), DynCom.iidPointer(_iid), DynCom.u16(flags)])"
    ));
    assert!(
        output.js.contains(
            "return [DynCom.toNumber(_r[0]), DynCom.adoptComPointer(_r[1], _iid), DynCom.toU64Bigint(_r[2])];"
        ),
        "{}",
        output.js
    );
    assert!(output.dts.contains(
        "resolve(context: number, riid: string, flags: number): [number, DynWinRtValue, bigint];"
    ));
}

#[test]
fn renderer_emits_distinct_by_value_variant_inputs() {
    let projected = ProjectedComInterface {
        name: "IVariantInput".into(),
        namespace: "Tests".into(),
        iid: "00000000-0000-0000-0000-000000000002".into(),
        is_iunknown_rooted: true,
        methods: vec![ProjectedComMethod {
            name: "UseVariant".into(),
            camel_name: "useVariant".into(),
            vtable_index: 3,
            params: vec![ProjectedComParam {
                name: "value".into(),
                typ: ComType::VariantByValue,
                direction: ComParamDirection::In,
                surface_input: true,
                surface_result: false,
                nullable: false,
            }],
            return_convention: ComReturnConvention::HResult,
            results: Vec::new(),
            string_buffer: None,
            typed_buffers: Vec::new(),
            shared_counts: Vec::new(),
            kind: ProjectedComMethodKind::Normal,
            doc: None,
            overload: None,
        }],
        activation: ActivationPlan::None,
        referenced_enums: Vec::new(),
    };

    let output = render_com_interface(&projected);
    assert!(output.js.contains(
        ".addMethodAt(3, 'UseVariant', new DynComMethodSig().addIn(DynCom.variantByValueType()))"
    ));
    assert!(output.js.contains("DynCom.variant(value)"));
    assert!(
        output
            .dts
            .contains("useVariant(value: DynComVariant): void;")
    );
    assert!(!output.dts.contains("DynComVariant | null"));
}

#[test]
fn typed_buffer_scalar_aliases_are_collected_for_declarations() {
    let alias = ComType::ScalarAlias {
        namespace: "Windows.Win32.System.Com".into(),
        name: "DISPID".into(),
        underlying: ComScalarRepr::Primitive(ComPrimitive::I32),
    };
    let projected = ProjectedComInterface {
        name: "ITest".into(),
        namespace: "Tests".into(),
        iid: "00000000-0000-0000-0000-000000000001".into(),
        is_iunknown_rooted: true,
        methods: vec![ProjectedComMethod {
            name: "Resolve".into(),
            camel_name: "resolve".into(),
            vtable_index: 3,
            params: vec![ProjectedComParam {
                name: "values".into(),
                typ: ComType::TypedBuffer {
                    element: Box::new(alias),
                },
                direction: ComParamDirection::In,
                surface_input: true,
                surface_result: false,
                nullable: false,
            }],
            return_convention: ComReturnConvention::HResult,
            results: Vec::new(),
            string_buffer: None,
            typed_buffers: Vec::new(),
            shared_counts: Vec::new(),
            kind: ProjectedComMethodKind::Normal,
            doc: None,
            overload: None,
        }],
        activation: ActivationPlan::None,
        referenced_enums: Vec::new(),
    };

    assert_eq!(
        collect_scalar_aliases(&projected),
        vec![("DISPID".into(), ComScalarRepr::Primitive(ComPrimitive::I32))]
    );
}

#[test]
fn renderer_serializes_fixed_capacity_bytes_from_projected_ir() {
    let byte_buffer = ComType::TypedBuffer {
        element: Box::new(ComType::Primitive(ComPrimitive::U8)),
    };
    let projected = ProjectedComInterface {
        name: "ITestBlob".into(),
        namespace: "Tests".into(),
        iid: "00000000-0000-0000-0000-000000000001".into(),
        is_iunknown_rooted: true,
        methods: vec![ProjectedComMethod {
            name: "GetBlob".into(),
            camel_name: "getBlob".into(),
            vtable_index: 15,
            params: vec![
                ProjectedComParam {
                    name: "guidKey".into(),
                    typ: ComType::Guid,
                    direction: ComParamDirection::In,
                    surface_input: true,
                    surface_result: false,
                    nullable: false,
                },
                ProjectedComParam {
                    name: "pBuf".into(),
                    typ: byte_buffer.clone(),
                    direction: ComParamDirection::CallerOutputBuffer,
                    surface_input: false,
                    surface_result: true,
                    nullable: false,
                },
                ProjectedComParam {
                    name: "capacity".into(),
                    typ: ComType::Primitive(ComPrimitive::U32),
                    direction: ComParamDirection::In,
                    surface_input: true,
                    surface_result: false,
                    nullable: false,
                },
                ProjectedComParam {
                    name: "pcbBlobSize".into(),
                    typ: ComType::Primitive(ComPrimitive::U32),
                    direction: ComParamDirection::Out,
                    surface_input: false,
                    surface_result: false,
                    nullable: false,
                },
            ],
            return_convention: ComReturnConvention::HResult,
            results: vec![ProjectedComResult {
                typ: byte_buffer,
                source: ResultSource::Param(1),
                conversion: ResultConversion::Buffer,
            }],
            string_buffer: None,
            typed_buffers: vec![TypedBufferPlan {
                buffer_param_index: 1,
                element: ComType::Primitive(ComPrimitive::U8),
                relation: TypedBufferRelation::CallerOutput {
                    capacity_param_index: 2,
                    actual_length_param_index: Some(3),
                    unit: BufferCountUnit::Bytes,
                    sizing: TypedBufferSizing::FixedCapacity,
                },
            }],
            shared_counts: Vec::new(),
            kind: ProjectedComMethodKind::FixedCapacityBytes {
                guid_param_index: 0,
            },
            doc: None,
            overload: None,
        }],
        activation: ActivationPlan::None,
        referenced_enums: Vec::new(),
    };

    let output = render_com_interface(&projected);
    assert!(
        output
            .js
            .contains(".addCallerOutputBuffer(DynCom.u8Type(), 2, 3, true, false)")
    );
    assert!(output.js.contains("getBlob(guidKey, capacity)"));
    assert!(
        output
            .js
            .contains("DynCom.callerOutputArray(DynCom.u8Type(), BigInt(_capacity))")
    );
    assert!(
        output
            .dts
            .contains("getBlob(guidKey: string, capacity: number): Buffer;")
    );
}

#[test]
fn parallel_arrays_use_semantic_element_counts_and_guid_conversion() {
    let projected = ProjectedComInterface {
        name: "IParallel".into(),
        namespace: "Tests".into(),
        iid: "00000000-0000-0000-0000-000000000010".into(),
        is_iunknown_rooted: true,
        methods: vec![ProjectedComMethod {
            name: "Copy".into(),
            camel_name: "copy".into(),
            vtable_index: 3,
            params: vec![
                ProjectedComParam {
                    name: "values".into(),
                    typ: ComType::TypedBuffer {
                        element: Box::new(ComType::Primitive(ComPrimitive::U32)),
                    },
                    direction: ComParamDirection::InputBuffer,
                    surface_input: true,
                    surface_result: false,
                    nullable: false,
                },
                ProjectedComParam {
                    name: "ids".into(),
                    typ: ComType::TypedBuffer {
                        element: Box::new(ComType::Guid),
                    },
                    direction: ComParamDirection::CallerOutputBuffer,
                    surface_input: false,
                    surface_result: true,
                    nullable: false,
                },
                ProjectedComParam {
                    name: "count".into(),
                    typ: ComType::Primitive(ComPrimitive::U32),
                    direction: ComParamDirection::In,
                    surface_input: false,
                    surface_result: false,
                    nullable: false,
                },
            ],
            return_convention: ComReturnConvention::HResult,
            results: vec![ProjectedComResult {
                typ: ComType::TypedBuffer {
                    element: Box::new(ComType::Guid),
                },
                source: ResultSource::Param(1),
                conversion: ResultConversion::PlainArray,
            }],
            string_buffer: None,
            typed_buffers: vec![
                TypedBufferPlan {
                    buffer_param_index: 0,
                    element: ComType::Primitive(ComPrimitive::U32),
                    relation: TypedBufferRelation::Input {
                        count_param_index: 2,
                        actual_length_param_index: None,
                        unit: BufferCountUnit::Elements,
                    },
                },
                TypedBufferPlan {
                    buffer_param_index: 1,
                    element: ComType::Guid,
                    relation: TypedBufferRelation::CallerOutput {
                        capacity_param_index: 2,
                        actual_length_param_index: None,
                        unit: BufferCountUnit::Elements,
                        sizing: TypedBufferSizing::SingleCall,
                    },
                },
            ],
            shared_counts: vec![SharedCountPlan::Parallel {
                count_param_index: 2,
                input_param_indices: vec![0],
                output_param_indices: vec![1],
            }],
            kind: ProjectedComMethodKind::Normal,
            doc: None,
            overload: None,
        }],
        activation: ActivationPlan::None,
        referenced_enums: Vec::new(),
    };

    let output = render_com_interface(&projected);
    assert!(
        output
            .js
            .contains("DynCom.bufferElementCount(_sharedInput0, DynCom.u32Type())")
    );
    assert!(output.js.contains("DynCom.takeGuidArray("));
}

#[test]
fn owning_array_bigint_capacity_is_not_number_validated() {
    assert!(count_type_uses_bigint(&ComType::NativeUsize));
    assert!(count_type_uses_bigint(&ComType::Primitive(
        ComPrimitive::U64
    )));
    assert!(!count_type_uses_bigint(&ComType::Primitive(
        ComPrimitive::U32
    )));
}

#[test]
fn renderer_emits_tagged_unions_and_automation_runtime_transfers() {
    use crate::codegen::com::ir::{
        NativePodScalar, NativeUnionArchitectureLayout, NativeUnionField, NativeUnionFieldType,
        NativeUnionLayout, ProjectedComResult,
    };

    let union_architecture = NativeUnionArchitectureLayout {
        size: 8,
        alignment: 8,
        fields: vec![
            NativeUnionField {
                name: "integer".into(),
                count: 1,
                typ: NativeUnionFieldType::Scalar(NativePodScalar::U64),
            },
            NativeUnionField {
                name: "pointer".into(),
                count: 1,
                typ: NativeUnionFieldType::Pointer,
            },
        ],
    };
    let union = ComType::NativeUnionPointer {
        layout: NativeUnionLayout {
            namespace: "Tests".into(),
            name: "TaggedValue".into(),
            x86: union_architecture.clone(),
            x64: union_architecture.clone(),
            arm64: union_architecture,
        },
    };
    let method =
        |name: &str, slot: usize, input: ComType, output: ComType, conversion: ResultConversion| {
            ProjectedComMethod {
                name: name.into(),
                camel_name: camel_case(name),
                vtable_index: slot,
                params: vec![
                    ProjectedComParam {
                        name: "value".into(),
                        typ: input,
                        direction: ComParamDirection::In,
                        surface_input: true,
                        surface_result: false,
                        nullable: false,
                    },
                    ProjectedComParam {
                        name: "result".into(),
                        typ: output.clone(),
                        direction: ComParamDirection::Out,
                        surface_input: false,
                        surface_result: true,
                        nullable: false,
                    },
                ],
                return_convention: ComReturnConvention::HResult,
                results: vec![ProjectedComResult {
                    typ: output,
                    source: ResultSource::Param(1),
                    conversion,
                }],
                string_buffer: None,
                typed_buffers: Vec::new(),
                shared_counts: Vec::new(),
                kind: ProjectedComMethodKind::Normal,
                doc: None,
                overload: None,
            }
        };
    let projected = ProjectedComInterface {
        name: "IAutomationTest".into(),
        namespace: "Tests".into(),
        iid: "00000000-0000-0000-0000-000000000009".into(),
        is_iunknown_rooted: true,
        methods: vec![
            method(
                "UseUnion",
                3,
                union,
                ComType::Variant,
                ResultConversion::Variant,
            ),
            method(
                "UseVariant",
                4,
                ComType::Variant,
                ComType::SafeArray {
                    element: crate::codegen::com::ir::SafeArrayElement::Bstr,
                },
                ResultConversion::SafeArray,
            ),
            method(
                "UseSafeArray",
                5,
                ComType::SafeArray {
                    element: crate::codegen::com::ir::SafeArrayElement::Bstr,
                },
                ComType::PropVariant,
                ResultConversion::PropVariant,
            ),
        ],
        activation: ActivationPlan::None,
        referenced_enums: Vec::new(),
    };

    let output = render_com_interface(&projected);
    assert!(output.js.contains("createTaggedValue(activeField, bytes)"));
    assert!(output.js.contains("DynCom.nativeUnion("));
    assert!(output.js.contains("DynCom.variant("));
    assert!(output.js.contains("DynCom.safeArray("));
    assert!(output.js.contains("DynCom.takeVariant("));
    assert!(output.js.contains("DynCom.takeSafeArray("));
    assert!(output.js.contains("DynCom.takePropVariant("));
    assert!(output.dts.contains("activeField: 'integer' | 'pointer'"));
    assert!(output.dts.contains("DynComNativeUnion"));
    assert!(output.dts.contains("DynComVariant"));
    assert!(output.dts.contains("DynComSafeArray"));
    assert!(output.dts.contains("DynComPropVariant"));
}

#[test]
fn renderer_emits_explicit_idispatch_invoke_options_and_compound_types() {
    let input = |name: &str, typ| ProjectedComParam {
        name: name.into(),
        typ,
        direction: ComParamDirection::In,
        surface_input: true,
        surface_result: false,
        nullable: false,
    };
    let optional_output = |name: &str, typ| ProjectedComParam {
        name: name.into(),
        typ,
        direction: ComParamDirection::OptionalOut,
        surface_input: false,
        surface_result: true,
        nullable: false,
    };
    let failure_output = |name: &str, typ| ProjectedComParam {
        surface_result: false,
        ..optional_output(name, typ)
    };
    let method = ProjectedComMethod {
        name: "Invoke".into(),
        camel_name: "invoke".into(),
        vtable_index: 6,
        params: vec![
            input("dispIdMember", ComType::Primitive(ComPrimitive::I32)),
            input("riid", ComType::RawPointer),
            input("lcid", ComType::Primitive(ComPrimitive::U32)),
            input("wFlags", ComType::Primitive(ComPrimitive::U16)),
            input("dispParams", ComType::DispatchParams),
            optional_output("varResult", ComType::Variant),
            failure_output("excepInfo", ComType::ExcepInfo),
            failure_output("argErr", ComType::Primitive(ComPrimitive::U32)),
        ],
        return_convention: ComReturnConvention::DispatchInvokeHResult,
        results: vec![ProjectedComResult {
            typ: ComType::Variant,
            source: ResultSource::Param(5),
            conversion: ResultConversion::Variant,
        }],
        string_buffer: None,
        typed_buffers: Vec::new(),
        shared_counts: Vec::new(),
        kind: ProjectedComMethodKind::DispatchInvoke {
            result_param_index: 5,
            excep_info_param_index: 6,
            arg_err_param_index: 7,
        },
        doc: None,
        overload: None,
    };
    let output = render_com_interface(&ProjectedComInterface {
        name: "IDispatch".into(),
        namespace: "Windows.Win32.System.Com".into(),
        iid: "00020400-0000-0000-c000-000000000046".into(),
        is_iunknown_rooted: true,
        methods: vec![method],
        activation: ActivationPlan::None,
        referenced_enums: Vec::new(),
    });

    assert!(output.js.contains(".addIn(DynCom.dispatchParamsType())"));
    assert!(
        output
            .js
            .contains(".addOptionalOut(DynCom.excepInfoType())")
    );
    assert!(
        output
            .js
            .contains("invoke(dispIdMember, riid, lcid, wFlags, dispParams, options = {})")
    );
    assert!(output.js.contains("DynCom.dispatchParams(dispParams)"));
    assert!(output.js.contains("DynCom.boolValue(_requestExcepInfo)"));
    assert!(output.js.contains(".captureDispatchInvokeHresult()"));
    assert!(output.js.contains(".invokeDispatch(this._obj"));
    assert!(output.js.contains("_error.hresult = _call.hresult"));
    assert!(output.js.contains("_error.excepInfo = _excepInfo"));
    assert!(output.js.contains("_error.argErr = _call.argErr"));
    assert!(output.js.contains("_error.cause = new Error"));
    assert!(output.js.contains("HRESULT ${_hresult}"));
    assert!(
        output
            .js
            .contains("_description ? `: ${_description}` : ''")
    );
    assert!(
        output
            .js
            .contains("_result.result = DynCom.takeVariant(_resultValue)")
    );
    assert!(!output.js.contains("_result.excepInfo"));
    assert!(!output.js.contains("_result.argErr"));
    assert!(output.dts.contains("dispParams: DynComDispatchParams"));
    assert!(output.dts.contains("): { result?: DynComVariant };"));
}

#[test]
fn coclass_renderer_uses_new_and_runtime_query_interface_views() {
    let primary = ProjectedComInterface {
        name: "ITest4".into(),
        namespace: "Tests".into(),
        iid: "00000000-0000-0000-0000-000000000004".into(),
        is_iunknown_rooted: true,
        methods: Vec::new(),
        activation: ActivationPlan::None,
        referenced_enums: Vec::new(),
    };
    let coclass = ProjectedComCoclass {
        name: "Test".into(),
        namespace: "Tests".into(),
        clsid: "10000000-0000-0000-0000-000000000001".into(),
        primary_interface: primary.clone(),
        associated_interfaces: vec![primary],
    };

    let output = render_com_coclass(&coclass);

    assert!(output.js.contains("class Test extends ITest4"));
    assert!(
        output
            .js
            .contains("const { ITest4, IID_ITest4 } = require('./ITest4.js');")
    );
    assert!(output.js.contains("exports.Test = Test;"));
    assert!(output.js.contains("constructor()"));
    assert!(
        output
            .js
            .contains("DynCom.coCreateInstance(CLSID_Test, IID_ITest4)")
    );
    assert!(output.js.contains("as(InterfaceClass)"));
    assert!(output.js.contains("DynCom.tryCast"));
    assert!(
        output
            .dts
            .contains("export declare class Test extends ITest4")
    );
    assert!(output.dts.contains("tryAs<T>"));
    assert!(
        output
            .extra_files
            .iter()
            .any(|(name, _)| name == "ITest4.js")
    );
}

#[test]
fn allocator_contract_rejects_trailing_separator_and_whitespace() {
    for free_with in ["CoTaskMemFree:", " CoTaskMemFree", "CoTaskMemFree "] {
        let method = MethodMeta {
            name: "GetData".into(),
            params: vec![ParamMeta {
                name: "data".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::Out,
            }],
            return_type: Some(make_hresult()),
            owned_outputs: vec![crate::com_metadata::OwnedOutput {
                param_index: 0,
                free_with: free_with.into(),
            }],
            ..Default::default()
        };

        let error = generate_com_interface_files(&plain_iface_with_method(method), "")
            .expect_err("malformed allocator names must fail closed");
        assert!(error.contains("unsupported output cleanup contract"));
    }
}

#[test]
fn cotaskmem_handle_and_inout_ownership_fail_closed() {
    let handle = TypeMeta::Struct {
        namespace: "Windows.Win32.Foundation".into(),
        name: "HWND".into(),
        fields: vec![crate::types::FieldMeta {
            name: "Value".into(),
            typ: TypeMeta::Object,
        }],
    };
    for (typ, direction, expected) in [
        (
            handle,
            ParamDirection::Out,
            "requires an Out data or string pointer",
        ),
        (
            TypeMeta::Object,
            ParamDirection::InOut,
            "allocator ownership transfer for [in, out]",
        ),
    ] {
        let method = MethodMeta {
            name: "GetData".into(),
            params: vec![ParamMeta {
                name: "data".into(),
                typ,
                direction,
            }],
            return_type: Some(make_hresult()),
            owned_outputs: vec![crate::com_metadata::OwnedOutput {
                param_index: 0,
                free_with: "CoTaskMemFree".into(),
            }],
            ..Default::default()
        };

        let error = generate_com_interface_files(&plain_iface_with_method(method), "")
            .expect_err("CoTaskMem ownership must not apply to handles or InOut");
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn dynamic_iid_output_rejects_cleanup_contract() {
    let method = MethodMeta {
        name: "GetThing".into(),
        params: vec![
            ParamMeta {
                name: "riid".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::In,
            },
            ParamMeta {
                name: "result".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::Out,
            },
        ],
        return_type: Some(make_hresult()),
        owned_outputs: vec![crate::com_metadata::OwnedOutput {
            param_index: 1,
            free_with: "CoTaskMemFree".into(),
        }],
        ..Default::default()
    };

    let error = generate_com_interface_files(&plain_iface_with_method(method), "")
        .expect_err("dynamic COM outputs cannot carry allocator cleanup");

    assert!(error.contains("dynamic-IID interface output cannot declare an allocator"));
}

fn handle_type_name(typ: &TypeMeta) -> Option<String> {
    match project_type(typ).ok()? {
        ComType::PointerAlias { name, .. } => Some(name),
        _ => None,
    }
}

fn handle_alias_kind(typ: &TypeMeta) -> Option<HandleAliasKind> {
    match project_type(typ).ok()? {
        ComType::PointerAlias { kind, .. } => Some(kind),
        _ => None,
    }
}

fn is_hresult(typ: &TypeMeta) -> bool {
    matches!(project_type(typ), Ok(ComType::HResult))
}

fn ts_type_expr_dts(typ: &TypeMeta) -> String {
    super::super::types::type_dts(&project_type(typ).unwrap())
}

fn ts_type_expr_js(typ: &TypeMeta) -> String {
    abi_type_js(&project_type(typ).unwrap())
}

fn wrap_arg_js(typ: &TypeMeta, variable: &str) -> String {
    super::super::types::wrap_arg_js(&project_type(typ).unwrap(), variable)
}

fn render_js(meta: &ComInterfaceMeta, _interop: Option<()>) -> String {
    let projected = project_com_interface(meta, "").unwrap();
    super::render_js(&projected)
}

fn render_dts(meta: &ComInterfaceMeta, _interop: Option<()>) -> String {
    let projected = project_com_interface(meta, "").unwrap();
    super::render_dts(&projected)
}

fn build_method_sig_js(method: &MethodMeta) -> String {
    let projected = project_com_interface(&plain_iface_with_method(method.clone()), "").unwrap();
    super::build_method_sig_js(&projected.methods[0])
}

fn render_enum_files(en: &ComEnumMeta) -> (String, String) {
    let underlying = match en.underlying {
        TypeMeta::I8 => ComEnumUnderlying::I8,
        TypeMeta::U8 => ComEnumUnderlying::U8,
        TypeMeta::I16 => ComEnumUnderlying::I16,
        TypeMeta::U16 => ComEnumUnderlying::U16,
        TypeMeta::I32 => ComEnumUnderlying::I32,
        TypeMeta::U32 => ComEnumUnderlying::U32,
        TypeMeta::I64 => ComEnumUnderlying::I64,
        TypeMeta::U64 => ComEnumUnderlying::U64,
        _ => panic!("unsupported test enum underlying type"),
    };
    let projected = ProjectedComEnum {
        namespace: en.namespace.clone(),
        name: en.name.clone(),
        underlying,
        members: en
            .members
            .iter()
            .map(|member| ProjectedComEnumMember {
                name: member.name.clone(),
                value: match member.value {
                    ComEnumValue::Signed(value) => ProjectedEnumValue::Signed(value),
                    ComEnumValue::Unsigned(value) => ProjectedEnumValue::Unsigned(value),
                },
            })
            .collect(),
    };
    super::render_enum_files(&projected)
}

fn method_is_interop_shape(method: &MethodMeta) -> Option<Vec<ParamMeta>> {
    if !method
        .return_type
        .as_ref()
        .is_some_and(|typ| matches!(project_type(typ), Ok(ComType::HResult)))
        || method.params.len() < 2
    {
        return None;
    }
    let output = method.params.last()?;
    let iid = &method.params[method.params.len() - 2];
    let iid_name = iid.name.to_ascii_lowercase();
    if output.direction != ParamDirection::Out
        || output.typ != TypeMeta::Object
        || iid.direction != ParamDirection::In
        || iid.typ != TypeMeta::Object
        || !matches!(iid_name.as_str(), "iid" | "riid")
        || method.params[..method.params.len() - 2]
            .iter()
            .any(|param| param.direction != ParamDirection::In)
    {
        return None;
    }
    Some(method.params[..method.params.len() - 2].to_vec())
}

#[test]
fn camel_case_basic() {
    assert_eq!(camel_case("HrInit"), "hrInit");
    assert_eq!(camel_case("SetProgressValue"), "setProgressValue");
    assert_eq!(camel_case("AddTab"), "addTab");
    assert_eq!(camel_case("URL"), "url");
    assert_eq!(camel_case("IOHandle"), "ioHandle");
}

#[test]
fn default_runtime_import_uses_com_subpath() {
    let previous = crate::codegen::project::get_import_name();
    crate::codegen::project::set_import_name("@microsoft/dynwinrt");
    assert_eq!(com_runtime_import_name(), "@microsoft/dynwinrt/com/unsafe");
    assert_eq!(com_public_import_name(), "@microsoft/dynwinrt/com");
    crate::codegen::project::set_import_name("../dist/com.js");
    assert_eq!(com_runtime_import_name(), "../dist/com-unsafe.js");
    assert_eq!(com_public_import_name(), "../dist/com.js");
    crate::codegen::project::set_import_name("./mycom.js");
    assert_eq!(com_runtime_import_name(), "./mycom.js");
    assert_eq!(com_public_import_name(), "./mycom.js");
    crate::codegen::project::set_import_name(&previous);
}

#[test]
fn strip_hungarian_only_at_word_boundary() {
    assert_eq!(strip_hungarian("dwReserved"), "Reserved");
    assert_eq!(strip_hungarian("hwndTab"), "Tab");
    // "hwnd" alone must NOT be stripped (no uppercase follow-up).
    assert_eq!(strip_hungarian("hwnd"), "hwnd");
}

#[test]
fn handle_type_name_recognizes_hwnd_shape() {
    let hwnd = TypeMeta::Struct {
        namespace: "Windows.Win32.Foundation".into(),
        name: "HWND".into(),
        fields: vec![crate::types::FieldMeta {
            name: "Value".into(),
            typ: TypeMeta::Object,
        }],
    };
    assert_eq!(handle_type_name(&hwnd).as_deref(), Some("HWND"));
}

#[test]
fn handle_alias_kind_distinguishes_handle_values_from_string_pointers() {
    let hwnd = TypeMeta::Struct {
        namespace: "Windows.Win32.Foundation".into(),
        name: "HWND".into(),
        fields: vec![crate::types::FieldMeta {
            name: "Value".into(),
            typ: TypeMeta::Object,
        }],
    };
    assert_eq!(handle_alias_kind(&hwnd), Some(HandleAliasKind::HandleValue));
    assert_eq!(
        handle_alias_kind(&pwstr_struct()),
        Some(HandleAliasKind::StringPointer(StringEncoding::Wide))
    );
    assert_eq!(
        handle_alias_kind(&pstr_struct()),
        Some(HandleAliasKind::StringPointer(StringEncoding::Ansi))
    );
    let psid = TypeMeta::Struct {
        namespace: "Windows.Win32.Security".into(),
        name: "PSID".into(),
        fields: vec![crate::types::FieldMeta {
            name: "Value".into(),
            typ: TypeMeta::Object,
        }],
    };
    assert_eq!(handle_alias_kind(&psid), Some(HandleAliasKind::DataPointer));
}

#[test]
fn hresult_is_not_a_handle() {
    let hr = TypeMeta::Struct {
        namespace: "Windows.Win32.Foundation".into(),
        name: "HRESULT".into(),
        fields: vec![crate::types::FieldMeta {
            name: "Value".into(),
            typ: TypeMeta::I32,
        }],
    };
    assert!(handle_type_name(&hr).is_none());
    assert!(is_hresult(&hr));
}

#[test]
fn non_win32_struct_is_not_a_handle() {
    let rect = TypeMeta::Struct {
        namespace: "Windows.Foundation".into(),
        name: "Rect".into(),
        fields: vec![
            crate::types::FieldMeta {
                name: "X".into(),
                typ: TypeMeta::F32,
            },
            crate::types::FieldMeta {
                name: "Y".into(),
                typ: TypeMeta::F32,
            },
            crate::types::FieldMeta {
                name: "Width".into(),
                typ: TypeMeta::F32,
            },
            crate::types::FieldMeta {
                name: "Height".into(),
                typ: TypeMeta::F32,
            },
        ],
    };
    assert!(handle_type_name(&rect).is_none());
}

// ---- Fix 2 (BOOL → boolean/i32) ----

fn win32_bool_struct() -> TypeMeta {
    TypeMeta::Struct {
        namespace: "Windows.Win32.Foundation".into(),
        name: "BOOL".into(),
        fields: vec![crate::types::FieldMeta {
            name: "Value".into(),
            typ: TypeMeta::I32,
        }],
    }
}

#[test]
fn win32_bool_is_not_a_handle() {
    let b = win32_bool_struct();
    // Sanity: it's the exact shape of a handle (single Value: I32) — the
    // special-case must WIN over the generic handle heuristic.
    assert!(
        handle_type_name(&b).is_none(),
        "BOOL must not be emitted as an opaque handle typedef"
    );
}

#[test]
fn win32_bool_projects_as_boolean_and_i32() {
    let b = win32_bool_struct();
    // .d.ts surface: boolean (not `BOOL` or `bigint | Buffer`)
    assert_eq!(ts_type_expr_dts(&b), "boolean");
    // .js registration: i32 type (not pointer)
    assert_eq!(ts_type_expr_js(&b), "DynCom.i32Type()");
    // .js argument marshalling: truthy→1, falsy→0 as an i32 (not pointer)
    assert_eq!(
        wrap_arg_js(&b, "fFullscreen"),
        "DynCom.i32(fFullscreen ? 1 : 0)"
    );
}

#[test]
fn hresult_input_projects_as_number_and_i32_value() {
    let hr = make_hresult();
    assert_eq!(ts_type_expr_dts(&hr), "number");
    assert_eq!(ts_type_expr_js(&hr), "DynCom.i32Type()");
    assert_eq!(wrap_arg_js(&hr, "hr"), "DynCom.i32(hr)");

    let m = MethodMeta {
        name: "Close".into(),
        vtable_index: 4,
        params: vec![ParamMeta {
            name: "hr".into(),
            typ: hr,
            direction: ParamDirection::In,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    let com = plain_iface_with_method(m);
    let js = render_js(&com, None);
    let dts = render_dts(&com, None);
    assert!(
        js.contains(".addMethodAt(4, 'Close', new DynComMethodSig().addIn(DynCom.i32Type()))"),
        ".js must register HRESULT in-param as i32:\n{}",
        js
    );
    assert!(
        js.contains("DynCom.i32(hr)"),
        ".js must pass HRESULT by value as i32:\n{}",
        js
    );
    assert!(
        !js.contains("DynCom.pointer(hr)"),
        ".js must not pass HRESULT as a pointer:\n{}",
        js
    );
    assert!(
        dts.contains("close(hr: number): void;"),
        ".d.ts must type HRESULT in-param as number:\n{}",
        dts
    );
    assert!(
        !dts.contains("HRESULT"),
        ".d.ts must not expose an undefined HRESULT alias:\n{}",
        dts
    );
}

#[test]
fn method_doc_url_renders_as_see_link_in_js_and_dts() {
    let m = MethodMeta {
        name: "Close".into(),
        vtable_index: 4,
        params: vec![ParamMeta {
            name: "hr".into(),
            typ: make_hresult(),
            direction: ParamDirection::In,
        }],
        return_type: Some(make_hresult()),
        doc: Some(
            "https://learn.microsoft.com/windows/win32/api/objidl/nf-objidl-ipersist-close".into(),
        ),
        ..Default::default()
    };
    let com = plain_iface_with_method(m);
    let js = render_js(&com, None);
    let dts = render_dts(&com, None);
    let expected_doc = "/** @see {@link https://learn.microsoft.com/windows/win32/api/objidl/nf-objidl-ipersist-close} */";
    assert!(
        js.contains(expected_doc),
        ".js must render the metadata doc URL as an @see JSDoc comment above the method:\n{}",
        js
    );
    assert!(
        dts.contains(expected_doc),
        ".d.ts must render the metadata doc URL as an @see JSDoc comment above the signature:\n{}",
        dts
    );
}

#[test]
fn method_without_doc_renders_no_see_comment() {
    let m = MethodMeta {
        name: "Close".into(),
        vtable_index: 4,
        params: vec![ParamMeta {
            name: "hr".into(),
            typ: make_hresult(),
            direction: ParamDirection::In,
        }],
        return_type: Some(make_hresult()),
        doc: None,
        ..Default::default()
    };
    let com = plain_iface_with_method(m);
    let js = render_js(&com, None);
    let dts = render_dts(&com, None);
    assert!(
        !js.contains("@see"),
        "renderer must not invent documentation when metadata has none:\n{}",
        js
    );
    assert!(!dts.contains("@see"), "same for .d.ts:\n{}", dts);
}

// ---- Fix 3 (REFIID-guarded interop heuristic) ----

/// Helper: construct a MethodMeta with HRESULT return type.
fn make_hresult() -> TypeMeta {
    TypeMeta::Struct {
        namespace: "Windows.Win32.Foundation".into(),
        name: "HRESULT".into(),
        fields: vec![crate::types::FieldMeta {
            name: "Value".into(),
            typ: TypeMeta::I32,
        }],
    }
}

#[test]
fn interop_shape_accepts_riid_named_object_trailing_in() {
    // Real Windows.Win32 shape: `HRESULT GetForWindow(HWND appWindow, REFIID riid, out void** ppv)`.
    // REFIID typically projects to TypeMeta::Object with name "riid".
    let m = MethodMeta {
        name: "GetForWindow".into(),
        vtable_index: 3,
        params: vec![
            ParamMeta {
                name: "appWindow".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::In,
            },
            ParamMeta {
                name: "riid".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::In,
            },
            ParamMeta {
                name: "ppv".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::Out,
            },
        ],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    let natural = method_is_interop_shape(&m)
        .expect("REFIID-shaped trailing in-param named `riid` must be recognised as interop");
    // Natural in-params = every in EXCEPT the trailing REFIID.
    assert_eq!(natural.len(), 1);
    assert_eq!(natural[0].name, "appWindow");
}

#[test]
fn interop_shape_rejects_guid_passed_by_value() {
    let m = MethodMeta {
        name: "GetSomething".into(),
        vtable_index: 3,
        params: vec![
            ParamMeta {
                name: "target".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::In,
            },
            ParamMeta {
                name: "riid".into(),
                typ: TypeMeta::Guid,
                direction: ParamDirection::In,
            },
            ParamMeta {
                name: "out".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::Out,
            },
        ],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    assert!(
        method_is_interop_shape(&m).is_none(),
        "a by-value GUID must not be passed as a REFIID pointer"
    );
}

/// FIX 3 REGRESSION: a method returning HRESULT with an [out] Object and a
/// trailing In-Object whose name is NOT `riid`/`iid` (e.g. a real application
/// COM interface pointer like `original`) must NOT be mis-classified as
/// interop-shape. Otherwise the codegen would silently drop the caller's
/// meaningful argument.
#[test]
fn interop_shape_rejects_non_refiid_trailing_object() {
    let m = MethodMeta {
        name: "CloneWithOriginal".into(),
        vtable_index: 3,
        params: vec![
            ParamMeta {
                name: "context".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::In,
            },
            ParamMeta {
                // NOT `riid`/`iid`, NOT Guid — a real COM pointer in-param.
                name: "original".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::In,
            },
            ParamMeta {
                name: "cloned".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::Out,
            },
        ],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    assert!(
        method_is_interop_shape(&m).is_none(),
        "trailing in-param `original` is a real Object argument, NOT a REFIID — \
         it must not be dropped by the interop heuristic"
    );
}

#[test]
fn interop_shape_rejects_iid_named_non_object_param() {
    // A parameter named `riid` but typed as a plain I32 is not a REFIID —
    // reject rather than silently drop.
    let m = MethodMeta {
        name: "Weird".into(),
        vtable_index: 3,
        params: vec![
            ParamMeta {
                name: "hwnd".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::In,
            },
            ParamMeta {
                name: "riid".into(),
                typ: TypeMeta::I32,
                direction: ParamDirection::In,
            },
            ParamMeta {
                name: "out".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::Out,
            },
        ],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    assert!(
        method_is_interop_shape(&m).is_none(),
        "an I32 named `riid` is not a REFIID — must be rejected"
    );
}

// ---- Fix 1 (winmd-derived interop IID, fail-loud on unresolved) ----

/// Build a fully synthetic ComInterfaceMeta for an `IFooInterop`-style
/// interface whose derived projected class name (`Foo`) does NOT exist
/// anywhere reachable. The generator must FAIL LOUDLY rather than emit
/// a NULL riid.
#[test]
fn interop_generation_fails_when_target_iid_unresolvable() {
    use crate::com_metadata::{ComInterfaceMeta, InterfaceMeta};

    let iface = InterfaceMeta {
        name: "IThisRuntimeClassDoesNotExist_DynWinrtInterop".into(),
        namespace: "Windows.Win32.System.WinRT".into(),
        iid: "00000000-0000-0000-0000-000000000000".into(),
        methods: vec![MethodMeta {
            name: "GetForWindow".into(),
            vtable_index: 3,
            params: vec![
                ParamMeta {
                    name: "appWindow".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "riid".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "ppv".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::Out,
                },
            ],
            return_type: Some(make_hresult()),
            ..Default::default()
        }],
        generic_piid: None,
        generic_args: Vec::new(),
        doc: None,
        deprecated: None,
    };
    let com = ComInterfaceMeta {
        interface: iface,
        base_offset: 3,
        is_iunknown_rooted: true,
        base_chain: vec!["IUnknown".into()],
        coclass_clsid: None,
        coclass_name: None,
        own_methods_start: 3,
        referenced_enums: Vec::new(),
        raw_referenced_enums: None,
        raw_methods: None,
    };
    // Pass empty winmd_paths — even with the newest-SDK fallback, the
    // synthetic class name won't be found anywhere.
    let result = generate_com_interface_files(&com, "");
    assert!(
        result.is_err(),
        "generator must fail loudly when the projected runtime-class IID \
         cannot be resolved; got Ok(_)"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("ThisRuntimeClassDoesNotExist_Dynwinrt")
            || err.contains("ThisRuntimeClassDoesNotExist_DynWinrt"),
        "error must name the class it failed to resolve: {}",
        err
    );
    assert!(
        !err.is_empty(),
        "error message must be non-empty (fail-loud contract)"
    );
}

#[test]
fn non_interop_iunknown_interface_still_generates_without_winmd_lookup() {
    // A vanilla IUnknown-rooted interface with no coclass and no
    // interop shape must succeed even when we pass empty winmd paths.
    use crate::com_metadata::{ComInterfaceMeta, InterfaceMeta};
    let iface = InterfaceMeta {
        name: "IMyPlainClassicCom".into(),
        namespace: "Windows.Win32.System.Com".into(),
        iid: "11111111-2222-3333-4444-555555555555".into(),
        methods: vec![MethodMeta {
            name: "DoStuff".into(),
            vtable_index: 3,
            params: vec![],
            return_type: Some(make_hresult()),
            ..Default::default()
        }],
        generic_piid: None,
        generic_args: Vec::new(),
        doc: None,
        deprecated: None,
    };
    let com = ComInterfaceMeta {
        interface: iface,
        base_offset: 3,
        is_iunknown_rooted: true,
        base_chain: vec!["IUnknown".into()],
        coclass_clsid: None,
        coclass_name: None,
        own_methods_start: 3,
        referenced_enums: Vec::new(),
        raw_referenced_enums: None,
        raw_methods: None,
    };
    let out = generate_com_interface_files(&com, "")
        .expect("plain classic-COM codegen must succeed with no winmds");
    assert!(out.js.contains("DynCom.registerIUnknownInterface"));
    assert!(out.js.contains("method(3)"));
}

// ---- Fix 4 (classic-COM plain `[out]` param → return-value projection) ----

fn plain_iface_with_method(m: MethodMeta) -> crate::com_metadata::ComInterfaceMeta {
    use crate::com_metadata::{ComInterfaceMeta, InterfaceMeta};
    let iface = InterfaceMeta {
        name: "IHasOut".into(),
        namespace: "Windows.Win32.System.Com".into(),
        iid: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(),
        methods: vec![m],
        generic_piid: None,
        generic_args: Vec::new(),
        doc: None,
        deprecated: None,
    };
    ComInterfaceMeta {
        interface: iface,
        base_offset: 3,
        is_iunknown_rooted: true,
        base_chain: vec!["IUnknown".into()],
        coclass_clsid: None,
        coclass_name: None,
        own_methods_start: 3,
        referenced_enums: Vec::new(),
        raw_referenced_enums: None,
        raw_methods: None,
    }
}

#[test]
fn unsupported_struct_in_out_fails_closed() {
    let method = MethodMeta {
        name: "Read".into(),
        params: vec![ParamMeta {
            name: "value".into(),
            typ: TypeMeta::Struct {
                namespace: "Windows.Win32.System.Com".into(),
                name: "VARIANT".into(),
                fields: vec![
                    crate::types::FieldMeta {
                        name: "vt".into(),
                        typ: TypeMeta::U16,
                    },
                    crate::types::FieldMeta {
                        name: "data".into(),
                        typ: TypeMeta::U64,
                    },
                ],
            },
            direction: ParamDirection::InOut,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };

    let error = generate_com_interface_files(&plain_iface_with_method(method), "")
        .expect_err("unsupported struct in/out must not emit a wrong T** ABI");
    assert!(error.contains("requires native layout projection"));
}

#[test]
fn unsupported_by_value_struct_fails_closed() {
    let method = MethodMeta {
        name: "DragEnter".into(),
        params: vec![ParamMeta {
            name: "point".into(),
            typ: TypeMeta::Struct {
                namespace: "Windows.Win32.Foundation".into(),
                name: "POINTL".into(),
                fields: vec![
                    crate::types::FieldMeta {
                        name: "x".into(),
                        typ: TypeMeta::I32,
                    },
                    crate::types::FieldMeta {
                        name: "y".into(),
                        typ: TypeMeta::I32,
                    },
                ],
            },
            direction: ParamDirection::In,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    let error = generate_com_interface_files(&plain_iface_with_method(method), "")
        .expect_err("struct layout must fail closed");
    assert!(error.contains("requires native layout projection"));
}

#[test]
fn unsupported_struct_direct_return_fails_closed() {
    let method = MethodMeta {
        name: "GetPoint".into(),
        return_type: Some(TypeMeta::Struct {
            namespace: "Windows.Win32.Foundation".into(),
            name: "POINT".into(),
            fields: vec![
                crate::types::FieldMeta {
                    name: "x".into(),
                    typ: TypeMeta::I32,
                },
                crate::types::FieldMeta {
                    name: "y".into(),
                    typ: TypeMeta::I32,
                },
            ],
        }),
        ..Default::default()
    };

    let error = generate_com_interface_files(&plain_iface_with_method(method), "")
        .expect_err("unsupported struct return must not panic at invocation time");
    assert!(error.contains("unsupported direct native return"));
}

#[test]
fn plain_method_single_out_scalar_projects_as_return() {
    // Model: `HRESULT GetShowCmd([out] int* pcmd)` — the classic single-out
    // int shape. The out-int must become the method's return value.
    let m = MethodMeta {
        name: "GetShowCmd".into(),
        vtable_index: 8,
        params: vec![ParamMeta {
            name: "pcmd".into(),
            typ: TypeMeta::I32,
            direction: ParamDirection::Out,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    let com = plain_iface_with_method(m);
    let js = render_js(&com, None);
    let dts = render_dts(&com, None);
    // .js: must capture `_out` and return it as a JS number.
    assert!(
        js.contains("const _out = _IHasOut.method(8).invoke(this._obj, [])"),
        ".js must capture invoke() result into _out:\n{}",
        js
    );
    assert!(
        js.contains("return DynCom.toNumber(_out);"),
        ".js must unwrap the I32 out:\n{}",
        js
    );
    // .d.ts: return type must be `number`, not `void`.
    assert!(
        dts.contains("getShowCmd(): number;"),
        ".d.ts must project single-out I32 as `number`:\n{}",
        dts
    );
}

#[test]
fn plain_method_single_out_guid_projects_as_string() {
    // Model: `HRESULT GetClassID([out] GUID* pClassID)` (IPersist shape).
    let m = MethodMeta {
        name: "GetClassID".into(),
        vtable_index: 3,
        params: vec![ParamMeta {
            name: "pClassID".into(),
            typ: TypeMeta::Guid,
            direction: ParamDirection::Out,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    let com = plain_iface_with_method(m);
    let js = render_js(&com, None);
    let dts = render_dts(&com, None);
    assert!(
        js.contains("const _out = _IHasOut.method(3).invoke(this._obj, [])"),
        ".js must capture invoke() result into _out:\n{}",
        js
    );
    assert!(
        js.contains("return DynCom.toGuidString(_out);"),
        ".js must unwrap GUID out:\n{}",
        js
    );
    assert!(
        dts.contains("getClassID(): string;"),
        ".d.ts must project single-out GUID as `string`:\n{}",
        dts
    );
}

#[test]
fn generated_interface_exposes_release_iid_and_protected_constructor() {
    let m = MethodMeta {
        name: "Close".into(),
        vtable_index: 4,
        params: Vec::new(),
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    let com = plain_iface_with_method(m);
    let js = render_js(&com, None);
    let dts = render_dts(&com, None);

    assert!(
        js.contains("release()") && js.contains("this._obj.release();"),
        ".js class must expose a release() method delegating to the managed native value:\n{}",
        js
    );
    assert!(
        dts.contains("release(): void;"),
        ".d.ts must declare release(): void;:\n{}",
        dts
    );
    assert!(
        dts.contains("protected constructor(obj: unknown);")
            && dts.contains("static readonly IID:"),
        ".d.ts must make the interface non-publicly constructible while allowing generated coclass subclasses:\n{}",
        dts
    );
    assert!(js.contains("Object.create(IHasOut.prototype)"));
}

#[test]
fn plain_method_single_out_enum_projects_as_underlying() {
    // Model: `HRESULT GetKind([out] MyKind* pk)` where MyKind is an I32
    // enum. Underlying-scalar unwrap → `.toNumber()`; .d.ts uses the enum
    // type name.
    let m = MethodMeta {
        name: "GetKind".into(),
        vtable_index: 5,
        params: vec![ParamMeta {
            name: "pk".into(),
            typ: TypeMeta::Enum {
                namespace: "Windows.Win32.System.Com".into(),
                name: "MyKind".into(),
                underlying: Box::new(TypeMeta::I32),
                members: Vec::new(),
                is_flags: false,
                doc: None,
                deprecated: None,
            },
            direction: ParamDirection::Out,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    let com = plain_iface_with_method(m);
    let js = render_js(&com, None);
    let dts = render_dts(&com, None);
    assert!(
        js.contains("return DynCom.toNumber(_out);"),
        ".js must unwrap enum out via its underlying scalar:\n{}",
        js
    );
    assert!(
        dts.contains("getKind(): MyKind;"),
        ".d.ts must project enum out under the enum's declared name:\n{}",
        dts
    );
}

#[test]
fn plain_method_multi_out_uses_invoke_all_and_tuple_return() {
    // Model: `HRESULT Q([out] uint32_t* a, [out] BOOL* found)` — two
    // trailing out params must flip to `.invokeAll()` and a tuple return.
    let m = MethodMeta {
        name: "Q".into(),
        vtable_index: 6,
        params: vec![
            ParamMeta {
                name: "a".into(),
                typ: TypeMeta::U32,
                direction: ParamDirection::Out,
            },
            ParamMeta {
                name: "found".into(),
                typ: TypeMeta::Bool,
                direction: ParamDirection::Out,
            },
        ],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    let com = plain_iface_with_method(m);
    let js = render_js(&com, None);
    let dts = render_dts(&com, None);
    assert!(
        js.contains("const _r = _IHasOut.method(6).invokeAll(this._obj, [])"),
        ".js multi-out must use .invokeAll():\n{}",
        js
    );
    assert!(
        js.contains("return [DynCom.toU32(_r[0]), DynCom.toBool(_r[1])];"),
        ".js multi-out must return a tuple with each out unwrapped:\n{}",
        js
    );
    assert!(
        dts.contains("q(): [number, boolean];"),
        ".d.ts multi-out must project a tuple type:\n{}",
        dts
    );
}

#[test]
fn plain_method_zero_out_still_discards_result() {
    // No out params: existing behavior — invoke and discard.
    let m = MethodMeta {
        name: "DoIt".into(),
        vtable_index: 4,
        params: vec![ParamMeta {
            name: "arg".into(),
            typ: TypeMeta::I32,
            direction: ParamDirection::In,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    let com = plain_iface_with_method(m);
    let js = render_js(&com, None);
    let dts = render_dts(&com, None);
    assert!(
        !js.contains("const _out ="),
        ".js zero-out must not capture invoke() result:\n{}",
        js
    );
    assert!(
        !js.contains("invokeAll"),
        ".js zero-out must not use .invokeAll():\n{}",
        js
    );
    assert!(
        js.contains("_IHasOut.method(4).invoke(this._obj,"),
        ".js zero-out must call plain .invoke():\n{}",
        js
    );
    assert!(
        dts.contains("doIt(arg: number): void;"),
        ".d.ts zero-out must still be `void`:\n{}",
        dts
    );
}

#[test]
fn direct_native_return_uses_return_abi_instead_of_synthetic_out_param() {
    let method = MethodMeta {
        name: "RetryRejectedCall".into(),
        vtable_index: 5,
        return_type: Some(TypeMeta::U32),
        ..Default::default()
    };
    let signature = build_method_sig_js(&method);
    assert_eq!(signature, "new DynComMethodSig().returns(DynCom.u32Type())");

    let com = plain_iface_with_method(method);
    let js = render_js(&com, None);
    let dts = render_dts(&com, None);
    assert!(js.contains("const _out = _IHasOut.method(5).invoke(this._obj, [])"));
    assert!(js.contains("return DynCom.toU32(_out);"));
    assert!(dts.contains("retryRejectedCall(): number;"));
}

#[test]
fn native_void_return_is_declared_explicitly() {
    let method = MethodMeta {
        name: "OnClose".into(),
        vtable_index: 8,
        return_type: None,
        ..Default::default()
    };
    assert_eq!(
        build_method_sig_js(&method),
        "new DynComMethodSig().returnsVoid()"
    );

    let com = plain_iface_with_method(method);
    let js = render_js(&com, None);
    assert!(js.contains("_IHasOut.method(8).invoke(this._obj, [])"));
    assert!(!js.contains("const _out ="));
}

#[test]
fn direct_64_bit_returns_use_bigint_accessors() {
    let i64_method = MethodMeta {
        name: "GetSigned".into(),
        return_type: Some(TypeMeta::I64),
        ..Default::default()
    };
    let u64_method = MethodMeta {
        name: "GetUnsigned".into(),
        return_type: Some(TypeMeta::U64),
        ..Default::default()
    };

    let i64_js = render_js(&plain_iface_with_method(i64_method), None);
    let u64_js = render_js(&plain_iface_with_method(u64_method), None);
    assert!(i64_js.contains("return DynCom.toI64Bigint(_out);"));
    assert!(u64_js.contains("return DynCom.toU64Bigint(_out);"));
}

#[test]
fn return_only_handle_declares_its_alias() {
    let hwnd = TypeMeta::Struct {
        namespace: "Windows.Win32.Foundation".into(),
        name: "HWND".into(),
        fields: vec![crate::types::FieldMeta {
            name: "Value".into(),
            typ: TypeMeta::Object,
        }],
    };
    let method = MethodMeta {
        name: "GetWindow".into(),
        return_type: Some(hwnd),
        ..Default::default()
    };
    let com = plain_iface_with_method(method);
    let dts = render_dts(&com, None);
    assert!(dts.contains("export type HWND = bigint | number;"));
    assert!(dts.contains("getWindow(): HWND;"));
}

#[test]
fn handle_value_arg_accepts_buffer_and_string_pointer_keeps_buffer() {
    let hwnd = TypeMeta::Struct {
        namespace: "Windows.Win32.Foundation".into(),
        name: "HWND".into(),
        fields: vec![crate::types::FieldMeta {
            name: "Value".into(),
            typ: TypeMeta::Object,
        }],
    };
    let method = MethodMeta {
        name: "SetOverlayIcon".into(),
        params: vec![
            ParamMeta {
                name: "hwnd".into(),
                typ: hwnd,
                direction: ParamDirection::In,
            },
            ParamMeta {
                name: "description".into(),
                typ: pwstr_struct(),
                direction: ParamDirection::In,
            },
        ],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    let iface = plain_iface_with_method(method);
    let dts = render_dts(&iface, None);
    let js = render_js(&iface, None);

    // HWND inputs accept Electron's pointer-width Buffer, but the HWND
    // output alias remains a numeric handle value.
    assert!(dts.contains("export type HWND = bigint | number;"));
    assert!(dts.contains("export type PWSTR = string | Buffer | Uint8Array | bigint;"));
    assert!(
        dts.contains("Pass a JS `string` (encoded automatically via DynCom.wideStringPointer)")
    );
    assert!(
        dts.contains("setOverlayIcon(hwnd: HWND | Buffer | Uint8Array, description: PWSTR): void;")
    );

    // Handle-value conversion is centralized in the runtime; string
    // pointers use the encoding-aware wide/ANSI constructors so the runtime
    // knows how to marshal a JS `string` input.
    assert!(
        js.contains("DynCom.pointer(DynCom.handleValue(hwnd))"),
        "HWND arg must use DynCom.handleValue:\n{js}"
    );
    assert!(!js.contains("function _handleArg("));
    assert!(js.contains("DynCom.wideStringPointer(description)"));
}

#[test]
fn data_pointer_alias_does_not_read_buffer_contents_as_a_handle() {
    let psid = TypeMeta::Struct {
        namespace: "Windows.Win32.Security".into(),
        name: "PSID".into(),
        fields: vec![crate::types::FieldMeta {
            name: "Value".into(),
            typ: TypeMeta::Object,
        }],
    };
    let method = MethodMeta {
        name: "AddUserSid".into(),
        params: vec![ParamMeta {
            name: "userSid".into(),
            typ: psid,
            direction: ParamDirection::In,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    let iface = plain_iface_with_method(method);
    let js = render_js(&iface, None);
    let dts = render_dts(&iface, None);

    assert!(dts.contains("export type PSID = bigint | number;"));
    assert!(dts.contains("addUserSid(userSid: PSID | Buffer | Uint8Array): void;"));
    assert!(js.contains("DynCom.pointer(userSid)"));
    assert!(!js.contains("handleValue(userSid)"));
}

#[test]
fn hwnd_in_out_uses_runtime_handle_conversion_without_inline_helper() {
    let hwnd = TypeMeta::Struct {
        namespace: "Windows.Win32.Foundation".into(),
        name: "HWND".into(),
        fields: vec![crate::types::FieldMeta {
            name: "Value".into(),
            typ: TypeMeta::Object,
        }],
    };
    let method = MethodMeta {
        name: "Create".into(),
        params: vec![ParamMeta {
            name: "window".into(),
            typ: hwnd,
            direction: ParamDirection::InOut,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    let iface = plain_iface_with_method(method);
    let js = render_js(&iface, None);

    assert!(js.contains("DynCom.pointer(DynCom.handleValue(window))"));
    assert!(!js.contains("function _handleArg("));
}

#[test]
fn return_only_enum_emits_import_and_sibling_files() {
    let kind = TypeMeta::Enum {
        namespace: "Windows.Win32.Example".into(),
        name: "THING_KIND".into(),
        underlying: Box::new(TypeMeta::I32),
        members: Vec::new(),
        is_flags: false,
        doc: None,
        deprecated: None,
    };
    let method = MethodMeta {
        name: "GetKind".into(),
        return_type: Some(kind.clone()),
        ..Default::default()
    };
    let mut com = plain_iface_with_method(method);
    com.referenced_enums.push(ComEnumMeta {
        namespace: "Windows.Win32.Example".into(),
        name: "THING_KIND".into(),
        underlying: TypeMeta::I32,
        members: Vec::new(),
        is_flags: false,
    });

    let output = generate_com_interface_files(&com, "").unwrap();
    assert!(
        output
            .dts
            .contains("import { THING_KIND } from './THING_KIND.js';")
    );
    assert!(
        output
            .extra_files
            .iter()
            .any(|(name, _)| name == "THING_KIND.d.ts")
    );
}

#[test]
fn unsigned_enum_literals_preserve_u32_and_u64_values() {
    let u32_enum = ComEnumMeta {
        namespace: "Windows.Win32.Example".into(),
        name: "U32_FLAGS".into(),
        underlying: TypeMeta::U32,
        members: vec![crate::com_metadata::ComEnumMember {
            name: "HIGH_BIT".into(),
            value: ComEnumValue::Unsigned(2_147_483_648),
        }],
        is_flags: true,
    };
    let u64_enum = ComEnumMeta {
        namespace: "Windows.Win32.Example".into(),
        name: "U64_FLAGS".into(),
        underlying: TypeMeta::U64,
        members: vec![crate::com_metadata::ComEnumMember {
            name: "HIGH_BIT".into(),
            value: ComEnumValue::Unsigned(9_223_372_036_854_775_808),
        }],
        is_flags: true,
    };

    let (u32_js, u32_dts) = render_enum_files(&u32_enum);
    assert!(u32_js.contains("HIGH_BIT: 2147483648"));
    assert!(u32_dts.contains("readonly HIGH_BIT: 2147483648;"));
    let (u64_js, u64_dts) = render_enum_files(&u64_enum);
    assert!(u64_js.contains("HIGH_BIT: 9223372036854775808n"));
    assert!(u64_dts.contains("readonly HIGH_BIT: 9223372036854775808n;"));
}

#[test]
fn in_out_parameter_is_both_argument_and_result() {
    let method = MethodMeta {
        name: "Adjust".into(),
        vtable_index: 4,
        params: vec![ParamMeta {
            name: "value".into(),
            typ: TypeMeta::I32,
            direction: ParamDirection::InOut,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    assert_eq!(
        build_method_sig_js(&method),
        "new DynComMethodSig().addInOut(DynCom.i32Type())"
    );

    let com = plain_iface_with_method(method);
    let js = render_js(&com, None);
    let dts = render_dts(&com, None);
    assert!(js.contains("adjust(value)"));
    assert!(js.contains("const _out = _IHasOut.method(4).invoke(this._obj, [DynCom.i32(value)])"));
    assert!(js.contains("return DynCom.toNumber(_out);"));
    assert!(dts.contains("adjust(value: number): number;"));
}

#[test]
fn unsupported_outfill_fails_closed() {
    let m = MethodMeta {
        name: "GetPath".into(),
        vtable_index: 2,
        params: vec![
            ParamMeta {
                name: "pszFile".into(),
                typ: TypeMeta::String, // PWSTR buffer, caller-allocated
                direction: ParamDirection::OutFill,
            },
            ParamMeta {
                name: "cch".into(),
                typ: TypeMeta::I32,
                direction: ParamDirection::In,
            },
        ],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    let com = plain_iface_with_method(m);
    let error = generate_com_interface_files(&com, "")
        .expect_err("unsupported caller-allocated arrays must fail closed");
    assert!(error.contains("caller-allocated array outputs are not supported"));
}

fn pwstr_struct() -> TypeMeta {
    TypeMeta::Struct {
        namespace: "Windows.Win32.Foundation".into(),
        name: "PWSTR".into(),
        fields: vec![crate::types::FieldMeta {
            name: "Value".into(),
            typ: TypeMeta::Object,
        }],
    }
}

fn pstr_struct() -> TypeMeta {
    TypeMeta::Struct {
        namespace: "Windows.Win32.Foundation".into(),
        name: "PSTR".into(),
        fields: vec![crate::types::FieldMeta {
            name: "Value".into(),
            typ: TypeMeta::Object,
        }],
    }
}

#[test]
fn out_string_buffer_allocates_decodes_and_returns_string() {
    let m = MethodMeta {
        name: "GetDescription".into(),
        vtable_index: 6,
        params: vec![
            ParamMeta {
                name: "pszName".into(),
                typ: pwstr_struct(),
                direction: ParamDirection::OutStringBuffer {
                    count_param_index: 1,
                },
            },
            ParamMeta {
                name: "cch".into(),
                typ: TypeMeta::I32,
                direction: ParamDirection::In,
            },
        ],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    let com = plain_iface_with_method(m);
    let js = render_js(&com, None);
    let dts = render_dts(&com, None);
    assert!(
        js.contains("function _normalizeStringBufferCount"),
        ".js must emit string buffer validation helper:\n{}",
        js
    );
    assert!(
        js.contains(".addMethodAt(6, 'GetDescription', new DynComMethodSig().addIn(DynCom.pointerType()).addIn(DynCom.i32Type()))"),
        ".js must register string buffer as an input pointer:\n{}",
        js
    );
    assert!(
        js.contains("getDescription(cch = 260)") && js.contains("Buffer.alloc(cch * 2)"),
        ".js must default cch and allocate a UTF-16 buffer:\n{}",
        js
    );
    assert!(
        js.contains("const _text = _decodeWideString(_buffer);") && js.contains("return _text;"),
        ".js must return the decoded wide string:\n{}",
        js
    );
    assert!(
        dts.contains("getDescription(cch?: number): string;"),
        ".d.ts must expose optional count and string return:\n{}",
        dts
    );
}

#[test]
fn callee_allocated_pwstr_is_decoded_and_freed() {
    let method = MethodMeta {
        name: "GetDisplayName".into(),
        vtable_index: 5,
        params: vec![ParamMeta {
            name: "name".into(),
            typ: pwstr_struct(),
            direction: ParamDirection::Out,
        }],
        return_type: Some(make_hresult()),
        owned_outputs: vec![crate::com_metadata::OwnedOutput {
            param_index: 0,
            free_with: "CoTaskMemFree".into(),
        }],
        ..Default::default()
    };
    let com = plain_iface_with_method(method);
    let js = render_js(&com, None);
    let dts = render_dts(&com, None);

    assert!(js.contains("return DynCom.takeCoTaskMemWideString(_out);"));
    assert!(dts.contains("getDisplayName(): string;"));
}

#[test]
fn string_pointer_output_without_allocator_fails_closed() {
    let method = MethodMeta {
        name: "GetDisplayName".into(),
        params: vec![ParamMeta {
            name: "name".into(),
            typ: pwstr_struct(),
            direction: ParamDirection::Out,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };

    let error = generate_com_interface_files(&plain_iface_with_method(method), "")
        .expect_err("string pointer outputs require an allocator contract");

    assert!(error.contains("string pointer output"));
    assert!(error.contains("no ownership projection"));
}

#[test]
fn unknown_output_cleanup_contract_fails_closed() {
    let method = MethodMeta {
        name: "GetData".into(),
        params: vec![ParamMeta {
            name: "data".into(),
            typ: TypeMeta::Object,
            direction: ParamDirection::Out,
        }],
        return_type: Some(make_hresult()),
        owned_outputs: vec![crate::com_metadata::OwnedOutput {
            param_index: 0,
            free_with: "LocalFree".into(),
        }],
        ..Default::default()
    };

    let error = generate_com_interface_files(&plain_iface_with_method(method), "")
        .expect_err("unknown allocators must fail before rendering");

    assert!(error.contains("unsupported output cleanup contract"));
    assert!(error.contains("LocalFree"));
}

#[test]
fn allocator_name_must_match_exactly() {
    let method = MethodMeta {
        name: "GetData".into(),
        params: vec![ParamMeta {
            name: "data".into(),
            typ: TypeMeta::Object,
            direction: ParamDirection::Out,
        }],
        return_type: Some(make_hresult()),
        owned_outputs: vec![crate::com_metadata::OwnedOutput {
            param_index: 0,
            free_with: "CoTaskMemFreeEx".into(),
        }],
        ..Default::default()
    };

    let error = generate_com_interface_files(&plain_iface_with_method(method), "")
        .expect_err("allocator prefixes must not be accepted");

    assert!(error.contains("unsupported output cleanup contract"));
    assert!(error.contains("CoTaskMemFreeEx"));
}

#[test]
fn multiple_string_buffers_fail_closed_before_rendering() {
    let method = MethodMeta {
        name: "GetNames".into(),
        params: vec![
            ParamMeta {
                name: "first".into(),
                typ: pwstr_struct(),
                direction: ParamDirection::OutStringBuffer {
                    count_param_index: 1,
                },
            },
            ParamMeta {
                name: "firstCount".into(),
                typ: TypeMeta::I32,
                direction: ParamDirection::In,
            },
            ParamMeta {
                name: "second".into(),
                typ: pwstr_struct(),
                direction: ParamDirection::OutStringBuffer {
                    count_param_index: 3,
                },
            },
            ParamMeta {
                name: "secondCount".into(),
                typ: TypeMeta::I32,
                direction: ParamDirection::In,
            },
        ],
        return_type: Some(make_hresult()),
        ..Default::default()
    };

    let error = generate_com_interface_files(&plain_iface_with_method(method), "")
        .expect_err("only one validated string buffer can be rendered");

    assert!(error.contains("multiple caller-owned string buffers"));
}

#[test]
fn semantic_hresult_dynamic_iid_fails_closed() {
    let method = MethodMeta {
        name: "GetThing".into(),
        params: vec![
            ParamMeta {
                name: "riid".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::In,
            },
            ParamMeta {
                name: "result".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::Out,
            },
        ],
        return_type: Some(make_hresult()),
        preserve_hresult: true,
        ..Default::default()
    };

    let error = generate_com_interface_files(&plain_iface_with_method(method), "")
        .expect_err("dynamic IID rendering cannot discard semantic HRESULT");

    assert!(error.contains("semantic HRESULT dynamic-IID methods are not supported"));
}

#[test]
fn managed_interface_input_imports_the_bridge_type() {
    let method = MethodMeta {
        name: "SetThing".into(),
        params: vec![ParamMeta {
            name: "thing".into(),
            typ: TypeMeta::Interface {
                namespace: "Tests".into(),
                name: "IThing".into(),
                iid: "11111111-2222-3333-4444-555555555555".into(),
            },
            direction: ParamDirection::In,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };

    let output = generate_com_interface_files(&plain_iface_with_method(method), "").unwrap();

    assert!(output.dts.contains("import type { DynWinRtValue }"));
    assert!(output.dts.contains("setThing(thing: DynWinRtValue): void;"));
}

#[test]
fn untyped_sysfree_output_fails_closed() {
    let method = MethodMeta {
        name: "GetAllFileTypes".into(),
        vtable_index: 4,
        params: vec![ParamMeta {
            name: "types".into(),
            typ: TypeMeta::Object,
            direction: ParamDirection::Out,
        }],
        return_type: Some(make_hresult()),
        owned_outputs: vec![crate::com_metadata::OwnedOutput {
            param_index: 0,
            free_with: "SysFreeString".into(),
        }],
        ..Default::default()
    };
    let error = generate_com_interface_files(&plain_iface_with_method(method), "")
        .expect_err("BSTR**-style untyped outputs must fail closed");
    assert!(error.contains("SysFreeString ownership requires a scalar Out BSTR"));
}

#[test]
fn string_buffer_preserves_additional_outputs() {
    let method = MethodMeta {
        name: "GetIconLocation".into(),
        vtable_index: 16,
        params: vec![
            ParamMeta {
                name: "path".into(),
                typ: pwstr_struct(),
                direction: ParamDirection::OutStringBuffer {
                    count_param_index: 1,
                },
            },
            ParamMeta {
                name: "cch".into(),
                typ: TypeMeta::I32,
                direction: ParamDirection::In,
            },
            ParamMeta {
                name: "icon".into(),
                typ: TypeMeta::I32,
                direction: ParamDirection::Out,
            },
        ],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    let com = plain_iface_with_method(method);
    let js = render_js(&com, None);
    let dts = render_dts(&com, None);

    assert!(js.contains("const _out = _IHasOut.method(16).invoke"));
    assert!(js.contains("return [_text, DynCom.toNumber(_out)];"));
    assert!(dts.contains("getIconLocation(cch?: number): [string, number];"));
}

#[test]
fn interface_out_param_projects_as_explicit_bridge_value() {
    let m = MethodMeta {
        name: "GetThing".into(),
        vtable_index: 7,
        params: vec![ParamMeta {
            name: "thing".into(),
            typ: TypeMeta::Interface {
                namespace: "Windows.Win32.System.Com".into(),
                name: "IThing".into(),
                iid: "11111111-2222-3333-4444-555555555555".into(),
            },
            direction: ParamDirection::Out,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    let com = plain_iface_with_method(m);
    let js = render_js(&com, None);
    let dts = render_dts(&com, None);
    assert!(
        !js.contains("from './IThing.js'"),
        ".js must not depend on an ungenerated wrapper:\n{}",
        js
    );
    assert!(js.contains(
        ".addOut(DynCom.interfaceType(WinGuid.parse('11111111-2222-3333-4444-555555555555')))"
    ));
    assert!(
        js.contains("return _out;"),
        ".js must return the managed bridge value:\n{}",
        js
    );
    assert!(
        dts.contains("import type { DynWinRtValue }"),
        ".d.ts must import the bridge type:\n{}",
        dts
    );
    assert!(
        dts.contains("getThing(): DynWinRtValue;"),
        ".d.ts must return the explicit bridge value:\n{}",
        dts
    );
}

#[test]
fn caller_supplied_riid_output_is_adopted() {
    let method = MethodMeta {
        name: "BindToHandler".into(),
        vtable_index: 4,
        params: vec![
            ParamMeta {
                name: "pbc".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::In,
            },
            ParamMeta {
                name: "riid".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::In,
            },
            ParamMeta {
                name: "ppv".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::Out,
            },
        ],
        return_type: Some(make_hresult()),
        ..Default::default()
    };
    let com = plain_iface_with_method(method);
    let output = generate_com_interface_files(&com, "").unwrap();

    assert!(output.js.contains("bindToHandler(pbc, riid)"));
    assert!(output.js.contains("const _iid = WinGuid.parse(riid)"));
    assert!(output.js.contains("DynCom.adoptComPointer(_out, _iid)"));
    assert!(
        output
            .dts
            .contains("bindToHandler(pbc: bigint | Buffer, riid: string): DynWinRtValue;")
    );
}

#[test]
fn hstring_output_uses_owned_hstring_projection() {
    let method = MethodMeta {
        name: "get_CorrelationVector".into(),
        vtable_index: 3,
        params: vec![ParamMeta {
            name: "cv".into(),
            typ: TypeMeta::String,
            direction: ParamDirection::Out,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };

    let output = generate_com_interface_files(&plain_iface_with_method(method), "").unwrap();

    assert!(output.js.contains(".addOut(DynCom.hstringType())"));
    assert!(output.js.contains("return _out.toString();"));
    assert!(output.dts.contains("get_CorrelationVector(): string;"));
}

#[test]
fn unresolved_interface_iid_fails_closed() {
    let method = MethodMeta {
        name: "CreateSurface".into(),
        vtable_index: 3,
        params: vec![ParamMeta {
            name: "result".into(),
            typ: TypeMeta::Interface {
                namespace: "Windows.UI.Composition".into(),
                name: "ICompositionSurface".into(),
                iid: String::new(),
            },
            direction: ParamDirection::Out,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };

    let error = generate_com_interface_files(&plain_iface_with_method(method), "")
        .expect_err("an unresolved interface must not degrade to a raw pointer");

    assert!(error.contains("ICompositionSurface"));
    assert!(error.contains("no resolvable IID"));
    assert!(error.contains("--ref"));
}

#[test]
fn parameterized_interface_fails_closed_even_with_a_piid() {
    let method = MethodMeta {
        name: "GetItems".into(),
        vtable_index: 3,
        params: vec![ParamMeta {
            name: "result".into(),
            typ: TypeMeta::Parameterized {
                namespace: "Windows.Foundation.Collections".into(),
                name: "IVectorView`1".into(),
                piid: "bbe1fa4c-b0e3-4583-baef-1f1b2e483e56".into(),
                args: vec![TypeMeta::String],
            },
            direction: ParamDirection::Out,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };

    let error = generate_com_interface_files(&plain_iface_with_method(method), "")
        .expect_err("a PIID alone is not a closed interface IID");

    assert!(error.contains("computed closed IID"));
    assert!(error.contains("raw-pointer fallback is not allowed"));
}

#[test]
fn async_interface_fails_closed_without_a_closed_iid() {
    let method = MethodMeta {
        name: "OpenAsync".into(),
        vtable_index: 3,
        params: vec![ParamMeta {
            name: "result".into(),
            typ: TypeMeta::AsyncOperation(Box::new(TypeMeta::String)),
            direction: ParamDirection::Out,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };

    let error = generate_com_interface_files(&plain_iface_with_method(method), "")
        .expect_err("async interfaces must not degrade to raw pointers");

    assert!(error.contains("async interface requires a computed closed IID"));
    assert!(error.contains("raw-pointer fallback is not allowed"));
}

#[test]
fn native_array_fails_closed_without_count_and_ownership() {
    let method = MethodMeta {
        name: "GetItems".into(),
        vtable_index: 3,
        params: vec![ParamMeta {
            name: "result".into(),
            typ: TypeMeta::Array(Box::new(TypeMeta::Interface {
                namespace: "Contoso".into(),
                name: "IItem".into(),
                iid: "11111111-2222-3333-4444-555555555555".into(),
            })),
            direction: ParamDirection::Out,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };

    let error = generate_com_interface_files(&plain_iface_with_method(method), "")
        .expect_err("native arrays must not degrade to raw pointers");

    assert!(error.contains("explicit count and element-ownership projection"));
    assert!(error.contains("raw-pointer fallback is not allowed"));
}

#[test]
fn delegate_fails_closed_without_a_callback_projection() {
    let method = MethodMeta {
        name: "SetHandler".into(),
        vtable_index: 3,
        params: vec![ParamMeta {
            name: "handler".into(),
            typ: TypeMeta::Delegate {
                namespace: "Contoso".into(),
                name: "Handler".into(),
                iid: "11111111-2222-3333-4444-555555555555".into(),
            },
            direction: ParamDirection::In,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };

    let error = generate_com_interface_files(&plain_iface_with_method(method), "")
        .expect_err("delegates require an explicit managed projection");

    assert!(error.contains("managed callback projection"));
    assert!(error.contains("raw-pointer fallback is not allowed"));
}

#[test]
fn runtime_class_uses_its_resolved_default_interface() {
    let method = MethodMeta {
        name: "CreateDevice".into(),
        vtable_index: 3,
        params: vec![ParamMeta {
            name: "result".into(),
            typ: TypeMeta::RuntimeClass {
                namespace: "Windows.UI.Composition".into(),
                name: "CompositionGraphicsDevice".into(),
                default_interface: Some(Box::new(TypeMeta::Interface {
                    namespace: "Windows.UI.Composition".into(),
                    name: "ICompositionGraphicsDevice".into(),
                    iid: "a329b321-0d69-4b89-9951-28de94dc998d".into(),
                })),
            },
            direction: ParamDirection::Out,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };

    let output = generate_com_interface_files(&plain_iface_with_method(method), "").unwrap();

    assert!(output.js.contains(
        ".addOut(DynCom.interfaceType(WinGuid.parse('a329b321-0d69-4b89-9951-28de94dc998d')))"
    ));
    assert!(output.js.contains("return _out;"));
    assert!(output.dts.contains("createDevice(): DynWinRtValue;"));
}

#[test]
fn runtime_class_without_a_default_interface_fails_closed() {
    let method = MethodMeta {
        name: "CreateDevice".into(),
        vtable_index: 3,
        params: vec![ParamMeta {
            name: "result".into(),
            typ: TypeMeta::RuntimeClass {
                namespace: "Windows.UI.Composition".into(),
                name: "CompositionGraphicsDevice".into(),
                default_interface: None,
            },
            direction: ParamDirection::Out,
        }],
        return_type: Some(make_hresult()),
        ..Default::default()
    };

    let error = generate_com_interface_files(&plain_iface_with_method(method), "")
        .expect_err("runtime classes require a resolved default interface");

    assert!(error.contains("no resolvable default interface"));
    assert!(error.contains("--ref"));
}

#[test]
fn semantic_hresult_is_preserved_as_a_number() {
    let method = MethodMeta {
        name: "IsDirty".into(),
        vtable_index: 4,
        return_type: Some(make_hresult()),
        preserve_hresult: true,
        ..Default::default()
    };

    let output = generate_com_interface_files(&plain_iface_with_method(method), "").unwrap();

    assert!(output.js.contains(".preserveHresult()"));
    assert!(output.js.contains("return DynCom.toNumber(_out);"));
    assert!(output.dts.contains("isDirty(): number;"));
}

#[test]
fn ordinary_hresult_remains_throw_or_void() {
    let method = MethodMeta {
        name: "Load".into(),
        vtable_index: 5,
        return_type: Some(make_hresult()),
        ..Default::default()
    };

    let output = generate_com_interface_files(&plain_iface_with_method(method), "").unwrap();

    assert!(!output.js.contains(".preserveHresult()"));
    assert!(output.dts.contains("load(): void;"));
}
