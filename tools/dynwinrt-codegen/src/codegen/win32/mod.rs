// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod ir;
mod model;
mod project;
mod render;

use crate::win32_metadata::RawApis;

pub use ir::{OmittedFunction, ProjectionResult};
pub use render::GeneratedOutput;

pub fn generate_apis_files(
    raw: &RawApis,
    runtime_import: &str,
) -> (GeneratedOutput, Vec<OmittedFunction>) {
    let projection = project::project_apis(raw);
    let output = render::render(&projection.projected, runtime_import);
    (output, projection.omitted)
}

pub fn project_apis(raw: &RawApis) -> ProjectionResult {
    project::project_apis(raw)
}

pub fn validate_function(function: &crate::win32_metadata::RawFunction) -> Result<(), String> {
    model::validate_function(function).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::ir::{
        AbiType, Conversion, Direction as ProjectedDirection, ReturnShape, SurfaceType,
    };
    use super::*;
    use crate::win32_metadata::{
        RawApis, RawArchitectures, RawBaseType, RawBuffer, RawBufferSize, RawCallingConvention,
        RawConstness, RawDirection, RawEnumMember, RawFunction, RawLayoutKind, RawNamedKind,
        RawNativeField, RawNativeLayout, RawNativeLayoutSet, RawPacking, RawParameter, RawScalar,
        RawStatusSemantics, RawType,
    };

    fn scalar(scalar: RawScalar) -> RawType {
        RawType {
            base: RawBaseType::Scalar(scalar),
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        }
    }

    fn synthetic_function(name: &str) -> RawFunction {
        RawFunction {
            namespace: "Tests".into(),
            container: "Apis".into(),
            name: name.into(),
            dll: "kernel32.dll".into(),
            entry_point: name.into(),
            return_type: scalar(RawScalar::U32),
            parameters: Vec::new(),
            return_status: RawStatusSemantics::None,
            return_free_with: None,
            supports_last_error: false,
            calling_convention: RawCallingConvention::System,
            architectures: RawArchitectures {
                x86: true,
                x64: true,
                arm64: true,
            },
            variadic: false,
        }
    }

    #[test]
    fn registry_projection_uses_immutable_plans_and_safe_pointers() {
        let Ok(winmd) = std::env::var("DYNWINRT_WIN32_WINMD") else {
            return;
        };
        if !std::path::Path::new(&winmd).is_file() {
            return;
        }
        let raw =
            crate::win32_metadata::parse_apis(&winmd, "Windows.Win32.System.Registry", "Apis")
                .unwrap();
        let (output, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert!(output.js.contains("DynWin32Function.bind"));
        assert!(!output.js.contains("DynWin32Unsafe"));
        assert!(!output.dts.contains("bigint | Buffer"));
        assert!(output.js.contains("exports.regOpenKeyExW"), "{omitted:#?}");
        assert!(output.js.contains("exports.regOpenKeyEx"), "{omitted:#?}");
    }

    #[test]
    fn fixed_buffer_projection_validates_minimum_storage() {
        let mut function = synthetic_function("ReadFourBytes");
        function.parameters.push(RawParameter {
            name: "buffer".into(),
            typ: RawType {
                base: RawBaseType::Scalar(RawScalar::U8),
                pointer_depth: 1,
                constness: RawConstness::Mutable,
            },
            direction: RawDirection::Out,
            nullable: false,
            reserved: false,
            null_null_terminated: false,
            buffer: Some(RawBuffer {
                element: scalar(RawScalar::U8),
                size: RawBufferSize::Constant(4),
            }),
            free_with: None,
        });
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function],
        };
        let (output, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert!(omitted.is_empty());
        assert!(output.js.contains("must contain at least 4 bytes"));
    }

    #[test]
    fn duplicate_exports_fail_closed_before_rendering() {
        let function = synthetic_function("Duplicate");
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function.clone(), function],
        };
        let projection = project_apis(&raw);
        assert_eq!(projection.complete_count(), 0);
        assert_eq!(projection.omitted.len(), 2);
        assert!(
            projection
                .omitted
                .iter()
                .all(|omission| omission.reason.contains("collision"))
        );
    }

    #[test]
    fn by_value_inout_handle_remains_a_direct_input() {
        let mut function = synthetic_function("FindClose");
        function.parameters.push(RawParameter {
            name: "hFindFile".into(),
            typ: RawType {
                base: RawBaseType::Named {
                    namespace: "Windows.Win32.Foundation".into(),
                    name: "HANDLE".into(),
                    kind: RawNamedKind::Handle {
                        cleanup: Some("CloseHandle".into()),
                    },
                },
                pointer_depth: 0,
                constness: RawConstness::Unspecified,
            },
            direction: RawDirection::InOut,
            nullable: false,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: None,
        });
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function],
        };
        let (output, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert!(omitted.is_empty());
        assert!(output.js.contains(
            r#"type: "handle", direction: "in", nullable: false, cleanup: "none", consumesResource: false, resourceCleanup: "closeHandle", aggregateDescriptor: undefined"#
        ));
        assert!(!output.dts.contains("readonly hFindFile"));
    }

    #[test]
    fn bool_failure_rule_precedes_owned_output_adoption() {
        let mut function = synthetic_function("OpenToken");
        function.return_type = scalar(RawScalar::Bool32);
        function.parameters.push(RawParameter {
            name: "token".into(),
            typ: RawType {
                base: RawBaseType::Named {
                    namespace: "Windows.Win32.Foundation".into(),
                    name: "HANDLE".into(),
                    kind: RawNamedKind::Handle {
                        cleanup: Some("CloseHandle".into()),
                    },
                },
                pointer_depth: 1,
                constness: RawConstness::Mutable,
            },
            direction: RawDirection::Out,
            nullable: false,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: Some("CloseHandle".into()),
        });
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function],
        };
        let (output, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert!(omitted.is_empty());
        assert!(output.js.contains(r#"successRule: "nonzero""#));
    }

    #[test]
    fn generic_handle_output_without_function_cleanup_fails_closed() {
        let mut function = synthetic_function("LsaConnectUntrusted");
        function.parameters.push(RawParameter {
            name: "handle".into(),
            typ: RawType {
                base: RawBaseType::Named {
                    namespace: "Windows.Win32.Foundation".into(),
                    name: "HANDLE".into(),
                    kind: RawNamedKind::Handle {
                        cleanup: Some("CloseHandle".into()),
                    },
                },
                pointer_depth: 1,
                constness: RawConstness::Mutable,
            },
            direction: RawDirection::Out,
            nullable: false,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: None,
        });
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function],
        };
        let projection = project_apis(&raw);
        assert_eq!(projection.complete_count(), 0);
        assert!(
            projection.omitted[0]
                .reason
                .contains("function-specific ownership")
        );
    }

    #[test]
    fn owning_inout_handle_fails_closed() {
        let mut function = synthetic_function("ReplaceHandle");
        function.parameters.push(RawParameter {
            name: "handle".into(),
            typ: RawType {
                base: RawBaseType::Named {
                    namespace: "Windows.Win32.Foundation".into(),
                    name: "HANDLE".into(),
                    kind: RawNamedKind::Handle {
                        cleanup: Some("CloseHandle".into()),
                    },
                },
                pointer_depth: 1,
                constness: RawConstness::Mutable,
            },
            direction: RawDirection::InOut,
            nullable: false,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: Some("CloseHandle".into()),
        });
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function],
        };
        let projection = project_apis(&raw);
        assert_eq!(projection.complete_count(), 0);
        assert!(projection.omitted[0].reason.contains("owning InOut handle"));
    }

    #[test]
    fn ntstatus_uses_signed_nonnegative_success() {
        let mut function = synthetic_function("NtFunction");
        function.return_type = scalar(RawScalar::I32);
        function.return_status = RawStatusSemantics::SignedNonNegativeIsSuccess;
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function],
        };
        let (output, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert!(omitted.is_empty());
        assert!(output.js.contains(r#"successRule: "nonnegative""#));
    }

    #[test]
    fn nullable_handle_uses_the_explicit_null_contract() {
        let mut function = synthetic_function("OptionalWindow");
        function.parameters.push(RawParameter {
            name: "hWnd".into(),
            typ: RawType {
                base: RawBaseType::Named {
                    namespace: "Windows.Win32.Foundation".into(),
                    name: "HWND".into(),
                    kind: RawNamedKind::Handle { cleanup: None },
                },
                pointer_depth: 0,
                constness: RawConstness::Unspecified,
            },
            direction: RawDirection::In,
            nullable: true,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: None,
        });
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function],
        };
        let (output, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert!(omitted.is_empty());
        assert!(output.js.contains("DynWin32.handle(hWnd, true)"));
        assert!(output.dts.contains("hWnd: HWND | null"));
    }

    #[test]
    fn cdecl_is_preserved_and_variadic_functions_fail_closed() {
        let mut function = synthetic_function("CdeclScalar");
        function.calling_convention = RawCallingConvention::Cdecl;
        function.parameters.push(RawParameter {
            name: "value".into(),
            typ: scalar(RawScalar::F32),
            direction: RawDirection::In,
            nullable: false,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: None,
        });
        function.return_type = scalar(RawScalar::F32);
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function.clone()],
        };
        let (output, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert!(omitted.is_empty());
        assert!(output.js.contains(r#"callingConvention: "cdecl""#));

        function.variadic = true;
        let projection = project_apis(&RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function],
        });
        assert_eq!(projection.complete_count(), 0);
        assert!(projection.omitted[0].reason.contains("variadic"));
    }

    fn point_layout(bitfield: bool) -> RawNativeLayoutSet {
        let fields = ["x", "y"]
            .into_iter()
            .map(|name| RawNativeField {
                name: name.into(),
                typ: scalar(RawScalar::I32),
                fixed_count: None,
                bitfield,
                flexible_array: false,
            })
            .collect();
        RawNativeLayoutSet {
            recursive: false,
            variants: vec![RawNativeLayout {
                architectures: RawArchitectures {
                    x86: true,
                    x64: true,
                    arm64: true,
                },
                kind: RawLayoutKind::Sequential,
                packing: RawPacking::Default,
                declared_size: None,
                forced_alignment: None,
                fields,
            }],
        }
    }

    #[test]
    fn native_struct_pointer_projects_typed_aligned_storage() {
        let mut function = synthetic_function("OffsetPoint");
        function.parameters.push(RawParameter {
            name: "point".into(),
            typ: RawType {
                base: RawBaseType::Named {
                    namespace: "Tests".into(),
                    name: "POINT".into(),
                    kind: RawNamedKind::NativeStruct {
                        layout: Box::new(point_layout(false)),
                    },
                },
                pointer_depth: 1,
                constness: RawConstness::Mutable,
            },
            direction: RawDirection::InOut,
            nullable: false,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: None,
        });
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function],
        };
        let (output, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert!(omitted.is_empty(), "{omitted:#?}");
        assert!(output.js.contains("DynWin32.createNativeStruct"));
        assert!(output.js.contains("DynWin32.nativeStruct(point"));
        assert!(
            output
                .dts
                .contains("createPOINT(bytes?: Buffer | Uint8Array): POINT")
        );
    }

    #[test]
    fn native_bitfield_layout_fails_closed() {
        let mut function = synthetic_function("Bitfield");
        function.parameters.push(RawParameter {
            name: "value".into(),
            typ: RawType {
                base: RawBaseType::Named {
                    namespace: "Tests".into(),
                    name: "BITS".into(),
                    kind: RawNamedKind::NativeStruct {
                        layout: Box::new(point_layout(true)),
                    },
                },
                pointer_depth: 1,
                constness: RawConstness::Mutable,
            },
            direction: RawDirection::In,
            nullable: false,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: None,
        });
        let projection = project_apis(&RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function],
        });
        assert_eq!(projection.complete_count(), 0);
        assert!(projection.omitted[0].reason.contains("bitfield"));
    }

    #[test]
    fn zero_argument_native_struct_return_declares_its_layout() {
        let mut function = synthetic_function("GetPoint");
        function.return_type = RawType {
            base: RawBaseType::Named {
                namespace: "Tests".into(),
                name: "POINT".into(),
                kind: RawNamedKind::NativeStruct {
                    layout: Box::new(point_layout(false)),
                },
            },
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        };
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function],
        };
        let (output, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert!(omitted.is_empty(), "{omitted:#?}");
        assert!(output.js.contains("const _nativeLayout_POINT"));
        assert!(
            output
                .js
                .contains("DynWin32.toNativeStruct(_return, _nativeLayout_POINT)")
        );
    }

    #[test]
    fn one_byte_metadata_bool_is_not_win32_bool() {
        let mut function = synthetic_function("Boolean8");
        function.parameters.push(RawParameter {
            name: "value".into(),
            typ: scalar(RawScalar::Bool8),
            direction: RawDirection::In,
            nullable: false,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: None,
        });
        function.return_type = scalar(RawScalar::Bool8);
        let (output, omitted) = generate_apis_files(
            &RawApis {
                namespace: "Tests".into(),
                class_name: "Apis".into(),
                functions: vec![function],
            },
            "@microsoft/dynwinrt/win32",
        );
        assert!(omitted.is_empty());
        assert!(output.js.contains("type: \"u8\""));
        assert!(output.js.contains("DynWin32.bool8(value)"));
        assert!(output.dts.contains("boolean8(value: boolean): boolean"));
    }

    #[test]
    fn scalar_returns_preserve_exact_abi_widths_and_js_shapes() {
        let cases = [
            ("ReturnI8", RawScalar::I8, AbiType::I8, SurfaceType::Number),
            (
                "ReturnI16",
                RawScalar::I16,
                AbiType::I16,
                SurfaceType::Number,
            ),
            (
                "ReturnI64",
                RawScalar::I64,
                AbiType::I64,
                SurfaceType::BigInt,
            ),
            (
                "ReturnU64",
                RawScalar::U64,
                AbiType::U64,
                SurfaceType::BigInt,
            ),
            (
                "ReturnF32",
                RawScalar::F32,
                AbiType::F32,
                SurfaceType::Number,
            ),
            (
                "ReturnF64",
                RawScalar::F64,
                AbiType::F64,
                SurfaceType::Number,
            ),
        ];
        let functions = cases
            .iter()
            .map(|(name, scalar, _, _)| {
                let mut function = synthetic_function(name);
                function.return_type = self::scalar(*scalar);
                function
            })
            .collect();
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions,
        };

        let projection = project_apis(&raw);
        assert!(projection.omitted.is_empty());
        for (name, _, abi, surface) in &cases {
            let function = projection
                .projected
                .functions
                .iter()
                .find(|function| function.metadata_name == *name)
                .unwrap();
            assert_eq!(function.runtime.return_abi, Some(*abi));
            assert!(matches!(
                &function.return_shape,
                ReturnShape::Direct { typ, .. } if typ == surface
            ));
        }

        let (output, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert!(omitted.is_empty());
        for abi in ["i8", "i16", "i64", "u64", "f32", "f64"] {
            assert!(output.js.contains(&format!("returnType: \"{abi}\"")));
        }
        assert_eq!(
            output
                .js
                .matches("return DynWin32.toBigint(_return)")
                .count(),
            2
        );
        assert!(output.dts.contains("returnI8(): number"));
        assert!(output.dts.contains("returnI16(): number"));
        assert!(output.dts.contains("returnI64(): bigint"));
        assert!(output.dts.contains("returnU64(): bigint"));
        assert!(output.dts.contains("returnF32(): number"));
        assert!(output.dts.contains("returnF64(): number"));
    }

    #[test]
    fn bool32_return_and_output_project_as_booleans() {
        let mut direct = synthetic_function("ReturnsBool32");
        direct.return_type = scalar(RawScalar::Bool32);

        let mut output = synthetic_function("GetFlag");
        output.return_type = RawType {
            base: RawBaseType::Void,
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        };
        output.parameters.push(RawParameter {
            name: "enabled".into(),
            typ: RawType {
                base: RawBaseType::Scalar(RawScalar::Bool32),
                pointer_depth: 1,
                constness: RawConstness::Mutable,
            },
            direction: RawDirection::Out,
            nullable: false,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: None,
        });

        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![direct, output],
        };
        let projection = project_apis(&raw);
        assert!(projection.omitted.is_empty());
        let get_flag = projection
            .projected
            .functions
            .iter()
            .find(|function| function.metadata_name == "GetFlag")
            .unwrap();
        assert_eq!(
            get_flag.runtime.parameters[0].direction,
            ProjectedDirection::Out
        );
        assert!(matches!(
            &get_flag.return_shape,
            ReturnShape::Object { outputs, .. }
                if outputs.len() == 1
                    && outputs[0].typ == SurfaceType::Boolean
                    && outputs[0].conversion == Conversion::Boolean
        ));

        let (generated, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert!(omitted.is_empty());
        assert!(generated.js.contains("return DynWin32.toBoolean(_return)"));
        assert!(
            generated
                .js
                .contains("enabled: DynWin32.toBoolean(_outputs[0])")
        );
        assert!(generated.dts.contains("returnsBool32(): boolean"));
        assert!(
            generated
                .dts
                .contains("getFlag(): { readonly enabled: boolean }")
        );
    }

    #[test]
    fn no_argument_void_returns_omit_result_fields() {
        let void_type = RawType {
            base: RawBaseType::Void,
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        };
        let mut no_outputs = synthetic_function("VoidNoOutputs");
        no_outputs.return_type = void_type.clone();

        let mut with_output = synthetic_function("VoidWithOutput");
        with_output.return_type = void_type;
        with_output.parameters.push(RawParameter {
            name: "value".into(),
            typ: RawType {
                base: RawBaseType::Scalar(RawScalar::U32),
                pointer_depth: 1,
                constness: RawConstness::Mutable,
            },
            direction: RawDirection::Out,
            nullable: false,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: None,
        });
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![no_outputs, with_output],
        };

        let (output, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert!(omitted.is_empty());
        assert!(output.js.contains("returnType: \"void\""));
        assert!(output.js.contains("return undefined"));
        assert!(output.dts.contains("voidNoOutputs(): void"));
        assert!(
            output
                .dts
                .contains("voidWithOutput(): { readonly value: number }")
        );
        assert!(!output.dts.contains("readonly result"));
    }

    #[test]
    fn unknown_by_value_type_fails_but_explicit_data_pointer_is_safe() {
        let mut unknown = synthetic_function("UnknownValue");
        unknown.parameters.push(RawParameter {
            name: "value".into(),
            typ: RawType {
                base: RawBaseType::Unknown("missing native definition".into()),
                pointer_depth: 0,
                constness: RawConstness::Unspecified,
            },
            direction: RawDirection::In,
            nullable: false,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: None,
        });

        let mut pointer = synthetic_function("OpaquePointer");
        pointer.parameters.push(RawParameter {
            name: "data".into(),
            typ: RawType {
                base: RawBaseType::Named {
                    namespace: "Tests".into(),
                    name: "PVOID".into(),
                    kind: RawNamedKind::DataPointer,
                },
                pointer_depth: 0,
                constness: RawConstness::Const,
            },
            direction: RawDirection::In,
            nullable: true,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: None,
        });

        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![unknown, pointer],
        };
        let (output, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert_eq!(omitted.len(), 1, "{omitted:#?}");
        assert!(omitted[0].reason.contains("native type is unknown"));
        assert!(!output.js.contains("unknownValue"));
        assert!(output.js.contains("DynWin32.dataPointer(data, true)"));
        assert!(
            output
                .dts
                .contains("opaquePointer(data: Buffer | Uint8Array | null): number")
        );
    }

    #[test]
    fn pointer_returns_require_explicit_lifetime_and_cleanup() {
        let pointer_type = RawType {
            base: RawBaseType::Named {
                namespace: "Tests".into(),
                name: "PVOID".into(),
                kind: RawNamedKind::DataPointer,
            },
            pointer_depth: 0,
            constness: RawConstness::Mutable,
        };
        let mut unowned = synthetic_function("UnownedPointer");
        unowned.return_type = pointer_type.clone();

        let mut owned = synthetic_function("OwnedPointer");
        owned.return_type = pointer_type;
        owned.return_free_with = Some("LocalFree".into());

        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![unowned, owned],
        };
        let (output, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert_eq!(omitted.len(), 1, "{omitted:#?}");
        assert!(
            omitted[0]
                .reason
                .contains("pointer return lifetime and ownership")
        );
        assert!(!output.js.contains("unownedPointer"));
        assert!(output.js.contains("returnCleanup: \"localFree\""));
        assert!(output.js.contains("return DynWin32.toResource(_return)"));
        assert!(
            output
                .dts
                .contains("ownedPointer(): DynWin32Resource | null")
        );
    }

    #[test]
    fn pointer_depth_two_fails_closed_before_rendering() {
        let mut function = synthetic_function("DoublePointerOutput");
        function.parameters.push(RawParameter {
            name: "values".into(),
            typ: RawType {
                base: RawBaseType::Scalar(RawScalar::U32),
                pointer_depth: 2,
                constness: RawConstness::Mutable,
            },
            direction: RawDirection::Out,
            nullable: false,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: None,
        });
        let projection = project_apis(&RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function],
        });
        assert_eq!(projection.complete_count(), 0);
        assert_eq!(projection.omitted.len(), 1);
        assert!(
            projection.omitted[0]
                .reason
                .contains("unsupported scalar pointer depth 2")
        );
    }

    #[test]
    fn counted_buffer_hides_and_derives_its_element_count() {
        let mut function = synthetic_function("WriteWords");
        function.parameters = vec![
            RawParameter {
                name: "values".into(),
                typ: RawType {
                    base: RawBaseType::Scalar(RawScalar::U16),
                    pointer_depth: 1,
                    constness: RawConstness::Const,
                },
                direction: RawDirection::In,
                nullable: false,
                reserved: false,
                null_null_terminated: false,
                buffer: Some(RawBuffer {
                    element: scalar(RawScalar::U16),
                    size: RawBufferSize::ElementCountParam(1),
                }),
                free_with: None,
            },
            RawParameter {
                name: "count".into(),
                typ: scalar(RawScalar::U32),
                direction: RawDirection::In,
                nullable: false,
                reserved: false,
                null_null_terminated: false,
                buffer: None,
                free_with: None,
            },
        ];
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function],
        };
        let (output, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert!(omitted.is_empty());
        assert!(output.js.contains("_bufferCount(values, 2)"));
        assert!(
            output
                .dts
                .contains("writeWords(values: Buffer | Uint8Array): number")
        );
        assert!(!output.dts.contains("count: number"));
    }

    fn enum_type(namespace: &str, name: &str, underlying: RawScalar) -> RawType {
        RawType {
            base: RawBaseType::Named {
                namespace: namespace.into(),
                name: name.into(),
                kind: RawNamedKind::Enum {
                    underlying,
                    members: vec![
                        RawEnumMember {
                            name: "NONE".into(),
                            value: 0,
                        },
                        RawEnumMember {
                            name: "HIGH_BIT".into(),
                            value: 0x8000_0000,
                        },
                    ],
                    is_flags: true,
                },
            },
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        }
    }

    #[test]
    fn unsigned_enum_high_bit_preserves_u32_abi_and_value() {
        let mut function = synthetic_function("SetSecurity");
        function.parameters.push(RawParameter {
            name: "securityInformation".into(),
            typ: enum_type("Tests.Security", "SECURITY_INFORMATION", RawScalar::U32),
            direction: RawDirection::In,
            nullable: false,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: None,
        });
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function],
        };
        let projection = project_apis(&raw);
        assert!(projection.omitted.is_empty());
        let projected = &projection.projected.functions[0];
        assert_eq!(projected.runtime.parameters[0].abi, AbiType::U32);
        assert!(matches!(
            projected.inputs[0],
            super::ir::InputExpression::Surface {
                conversion: Conversion::U32,
                ..
            }
        ));

        let (output, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert!(omitted.is_empty());
        assert!(output.js.contains("DynWin32.u32(securityInformation)"));
        let enum_js = output
            .extra_files
            .iter()
            .find(|(name, _)| name == "SECURITY_INFORMATION.js")
            .map(|(_, content)| content)
            .unwrap();
        assert!(enum_js.contains("HIGH_BIT: 2147483648"));
    }

    #[test]
    fn enum_simple_name_collisions_fail_closed() {
        let mut first = synthetic_function("UseFirstMode");
        first.parameters.push(RawParameter {
            name: "mode".into(),
            typ: enum_type("Tests.First", "MODE", RawScalar::U32),
            direction: RawDirection::In,
            nullable: false,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: None,
        });
        let mut second = synthetic_function("UseSecondMode");
        second.parameters.push(RawParameter {
            name: "mode".into(),
            typ: enum_type("Tests.Second", "MODE", RawScalar::U32),
            direction: RawDirection::In,
            nullable: false,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: None,
        });

        let projection = project_apis(&RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![first, second],
        });
        assert_eq!(projection.complete_count(), 0);
        assert_eq!(projection.omitted.len(), 2);
        assert!(
            projection
                .omitted
                .iter()
                .all(|omission| { omission.reason.contains("enum simple name is ambiguous") })
        );
    }

    #[test]
    fn unsupported_enum_underlying_type_fails_closed() {
        let mut function = synthetic_function("LargeEnum");
        function.return_type = enum_type("Tests", "LARGE_ENUM", RawScalar::U64);
        let projection = project_apis(&RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function],
        });
        assert_eq!(projection.complete_count(), 0);
        assert!(
            projection.omitted[0]
                .reason
                .contains("enum underlying type is not representable"),
            "{:#?}",
            projection.omitted
        );
    }

    #[test]
    fn generation_is_deterministic_without_renderer_snapshots() {
        let mut enum_function = synthetic_function("SetSecurity");
        enum_function.parameters.push(RawParameter {
            name: "securityInformation".into(),
            typ: enum_type("Tests", "SECURITY_INFORMATION", RawScalar::U32),
            direction: RawDirection::In,
            nullable: false,
            reserved: false,
            null_null_terminated: false,
            buffer: None,
            free_with: None,
        });
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![synthetic_function("Zulu"), enum_function],
        };
        let first = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        let second = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert_eq!(first, second);
    }

    #[test]
    fn byte_counted_opaque_buffer_does_not_require_element_layout() {
        let mut function = synthetic_function("QueryOpaqueData");
        function.parameters = vec![
            RawParameter {
                name: "data".into(),
                typ: RawType {
                    base: RawBaseType::Unknown("variable native record".into()),
                    pointer_depth: 1,
                    constness: RawConstness::Mutable,
                },
                direction: RawDirection::Out,
                nullable: true,
                reserved: false,
                null_null_terminated: false,
                buffer: Some(RawBuffer {
                    element: RawType {
                        base: RawBaseType::Unknown("variable native record".into()),
                        pointer_depth: 0,
                        constness: RawConstness::Unspecified,
                    },
                    size: RawBufferSize::ByteCountParam(1),
                }),
                free_with: None,
            },
            RawParameter {
                name: "size".into(),
                typ: RawType {
                    base: RawBaseType::Scalar(RawScalar::U32),
                    pointer_depth: 1,
                    constness: RawConstness::Mutable,
                },
                direction: RawDirection::InOut,
                nullable: false,
                reserved: false,
                null_null_terminated: false,
                buffer: None,
                free_with: None,
            },
        ];
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function.clone()],
        };
        let (output, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert!(omitted.is_empty(), "{omitted:#?}");
        assert!(output.js.contains("_bufferCount(data, 1)"));
        assert!(
            output
                .js
                .contains("DynWin32.alignedDataPointer(data, 8, true)")
        );
        assert!(
            output
                .dts
                .contains("queryOpaqueData(data: Buffer | Uint8Array | null)")
        );
        assert!(output.dts.contains("readonly size: number"));

        function.parameters[0].buffer.as_mut().unwrap().size = RawBufferSize::ElementCountParam(1);
        let projection = project_apis(&RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function],
        });
        assert_eq!(projection.complete_count(), 0);
        assert!(
            projection.omitted[0]
                .reason
                .contains("native buffer element")
        );

        function = synthetic_function("QueryDoublePointer");
        function.parameters = vec![
            RawParameter {
                name: "data".into(),
                typ: RawType {
                    base: RawBaseType::Scalar(RawScalar::U8),
                    pointer_depth: 2,
                    constness: RawConstness::Mutable,
                },
                direction: RawDirection::Out,
                nullable: true,
                reserved: false,
                null_null_terminated: false,
                buffer: Some(RawBuffer {
                    element: scalar(RawScalar::U8),
                    size: RawBufferSize::ByteCountParam(1),
                }),
                free_with: None,
            },
            RawParameter {
                name: "size".into(),
                typ: scalar(RawScalar::U32),
                direction: RawDirection::In,
                nullable: false,
                reserved: false,
                null_null_terminated: false,
                buffer: None,
                free_with: None,
            },
        ];
        let projection = project_apis(&RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions: vec![function],
        });
        assert_eq!(projection.complete_count(), 0);
        assert!(
            projection.omitted[0]
                .reason
                .contains("one data indirection")
        );
    }

    fn handle_type(namespace: &str, name: &str, cleanup: &str) -> RawType {
        RawType {
            base: RawBaseType::Named {
                namespace: namespace.into(),
                name: name.into(),
                kind: RawNamedKind::Handle {
                    cleanup: Some(cleanup.into()),
                },
            },
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        }
    }

    #[test]
    fn direct_handle_ownership_uses_exact_function_evidence() {
        let cases = [
            (
                "Windows.Win32.System.Memory",
                "LocalAlloc",
                "HLOCAL",
                "LocalFree",
                "localFree",
            ),
            (
                "Windows.Win32.System.Memory",
                "GlobalAlloc",
                "HGLOBAL",
                "GlobalFree",
                "globalFree",
            ),
            (
                "Windows.Win32.System.LibraryLoader",
                "LoadLibraryW",
                "HMODULE",
                "FreeLibrary",
                "freeLibrary",
            ),
            (
                "Windows.Win32.System.Services",
                "OpenSCManagerW",
                "SC_HANDLE",
                "CloseServiceHandle",
                "closeServiceHandle",
            ),
        ];
        let mut functions = Vec::new();
        for (namespace, name, handle, cleanup, _) in cases {
            let mut function = synthetic_function(name);
            function.namespace = namespace.into();
            function.return_type = handle_type(namespace, handle, cleanup);
            functions.push(function);
        }
        let mut unknown = synthetic_function("MysteryAlloc");
        unknown.namespace = "Windows.Win32.System.Memory".into();
        unknown.return_type = handle_type("Windows.Win32.Foundation", "HGLOBAL", "GlobalFree");
        functions.push(unknown);
        let raw = RawApis {
            namespace: "Tests".into(),
            class_name: "Apis".into(),
            functions,
        };
        let (output, omitted) = generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        assert_eq!(omitted.len(), 1, "{omitted:#?}");
        assert!(omitted[0].identity.ends_with("::MysteryAlloc"));
        for (_, _, _, _, cleanup) in cases {
            assert!(output.js.contains(&format!("returnCleanup: \"{cleanup}\"")));
        }
        assert_eq!(output.js.matches("successRule: \"validHandle\"").count(), 4);
    }
}
