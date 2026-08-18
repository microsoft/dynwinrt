// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

fn configured_winmd() -> Option<String> {
    std::env::var("DYNWINRT_WIN32_WINMD")
        .ok()
        .filter(|path| std::path::Path::new(path).is_file())
}

#[test]
fn registry_projection_is_natural_and_ownership_aware() {
    let Some(winmd) = configured_winmd() else {
        return;
    };
    let raw = dynwinrt_codegen::win32_metadata::parse_apis(
        &winmd,
        "Windows.Win32.System.Registry",
        "Apis",
    )
    .unwrap();
    let (output, omissions) =
        dynwinrt_codegen::codegen::win32::generate_apis_files(&raw, "@microsoft/dynwinrt/win32");

    assert!(output.js.contains("DynWin32Function.bind"));
    assert!(output.js.contains("DynWin32.dataPointer(data, true)"));
    assert!(output.js.contains("consumesResource: true"));
    assert!(
        output
            .js
            .contains(r#"DynWin32.resource(hKey, "regCloseKey")"#)
    );
    assert!(output.js.contains("const regOpenKeyEx = regOpenKeyExW"));
    assert!(output.js.contains("exports.regOpenKeyEx = regOpenKeyEx"));
    assert!(output.dts.contains(
        "regOpenKeyExW(hKey: HKEY, subKey: string | Buffer | Uint8Array | null, ulOptions: number, samDesired: REG_SAM_FLAGS): { readonly status: number; readonly key: DynWin32Resource | null }"
    ));
    assert!(output.dts.contains(
        "regQueryValueExW(hKey: HKEY, valueName: string | Buffer | Uint8Array | null, data: Buffer | Uint8Array | null): { readonly status: number; readonly type: REG_VALUE_TYPE; readonly dataSize: number }"
    ));
    assert!(output.dts.contains(
        "export declare function regCloseKey(hKey: DynWin32Resource): { readonly status: number }"
    ));
    assert!(!output.dts.contains("bigint | Buffer"));
    assert!(
        output.js.contains("exports.regCreateKeyExW"),
        "{omissions:#?}"
    );
}

#[test]
fn scalar_return_projects_directly_without_result_wrapper() {
    let Some(winmd) = configured_winmd() else {
        return;
    };
    let raw = dynwinrt_codegen::win32_metadata::parse_apis(
        &winmd,
        "Windows.Win32.System.SystemInformation",
        "Apis",
    )
    .unwrap();
    let (output, _) =
        dynwinrt_codegen::codegen::win32::generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
    assert!(
        output
            .dts
            .contains("export declare function getTickCount64(): bigint")
    );
    assert!(output.js.contains("return DynWin32.toBigint(_return)"));
    assert!(output.dts.contains(
        "export declare function createSYSTEMTIME(bytes?: Buffer | Uint8Array): SYSTEMTIME"
    ));
    assert!(
        output
            .dts
            .contains("export declare function getSystemTime(systemTime: SYSTEMTIME): void")
    );
    assert!(output.js.contains("DynWin32.nativeStruct(systemTime"));
}

#[test]
fn generated_safe_surface_uses_unsafe_binding_only_internally() {
    let Some(winmd) = configured_winmd() else {
        return;
    };
    let raw = dynwinrt_codegen::win32_metadata::parse_apis(
        &winmd,
        "Windows.Win32.System.LibraryLoader",
        "Apis",
    )
    .unwrap();
    let (output, _) =
        dynwinrt_codegen::codegen::win32::generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
    assert!(!output.js.contains("DynWin32Unsafe"));
    assert!(output.js.contains("@microsoft/dynwinrt/win32/unsafe"));
    assert!(!output.dts.contains("/win32/unsafe"));
    assert!(output.js.contains("getModuleHandleW"));
    assert!(output.dts.contains("export type HMODULE ="));
}

#[test]
fn ldap_cdecl_metadata_reaches_the_immutable_plan() {
    let Some(winmd) = configured_winmd() else {
        return;
    };
    let raw = dynwinrt_codegen::win32_metadata::parse_apis(
        &winmd,
        "Windows.Win32.Networking.Ldap",
        "Apis",
    )
    .unwrap();
    let (output, omitted) =
        dynwinrt_codegen::codegen::win32::generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
    assert!(
        output
            .dts
            .contains("export declare function ldapGetLastError(): number"),
        "{omitted:#?}"
    );
    assert!(output.js.contains(r#"callingConvention: "cdecl""#));
}

#[test]
fn com_interface_inputs_require_managed_values_and_exact_iids() {
    let Some(winmd) = configured_winmd() else {
        return;
    };
    let raw =
        dynwinrt_codegen::win32_metadata::parse_apis(&winmd, "Windows.Win32.System.Com", "Apis")
            .unwrap();
    let (output, omissions) =
        dynwinrt_codegen::codegen::win32::generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
    assert!(
        output
            .dts
            .contains("export declare function coIsHandlerConnected(")
            && output.dts.contains("DynWinRtValue): boolean"),
        "{omissions:#?}"
    );
    assert!(output.js.contains("DynWin32.comObject("));
}

#[test]
fn unsigned_registry_enum_preserves_high_bit_u32_semantics() {
    let Some(winmd) = configured_winmd() else {
        return;
    };
    let raw = dynwinrt_codegen::win32_metadata::parse_apis(
        &winmd,
        "Windows.Win32.System.Registry",
        "Apis",
    )
    .unwrap();
    let function = raw
        .functions
        .iter()
        .find(|function| function.name == "RegSetKeySecurity")
        .expect("RegSetKeySecurity metadata");
    let parameter = function
        .parameters
        .iter()
        .find(|parameter| parameter.name == "SecurityInformation")
        .expect("SecurityInformation metadata");
    let dynwinrt_codegen::win32_metadata::RawBaseType::Named {
        name,
        kind:
            dynwinrt_codegen::win32_metadata::RawNamedKind::Enum {
                underlying,
                members,
                ..
            },
        ..
    } = &parameter.typ.base
    else {
        panic!("SecurityInformation must remain a named enum");
    };
    assert_eq!(name, "OBJECT_SECURITY_INFORMATION");
    assert_eq!(
        *underlying,
        dynwinrt_codegen::win32_metadata::RawScalar::U32
    );
    assert!(
        members
            .iter()
            .any(|member| member.value > i128::from(i32::MAX))
    );

    let (output, omissions) =
        dynwinrt_codegen::codegen::win32::generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
    assert!(
        output.js.contains("DynWin32.u32(securityInformation)"),
        "{omissions:#?}"
    );
}

#[test]
fn unowned_double_pointer_output_fails_closed() {
    let Some(winmd) = configured_winmd() else {
        return;
    };
    let raw = dynwinrt_codegen::win32_metadata::parse_apis(
        &winmd,
        "Windows.Win32.System.Com.StructuredStorage",
        "Apis",
    )
    .unwrap();
    let function = raw
        .functions
        .iter()
        .find(|function| function.name == "PropVariantToUInt32VectorAlloc")
        .expect("PropVariantToUInt32VectorAlloc metadata");
    let parameter = function
        .parameters
        .iter()
        .find(|parameter| parameter.name == "pprgn")
        .expect("pprgn metadata");
    assert_eq!(parameter.typ.pointer_depth, 2);

    let projection = dynwinrt_codegen::codegen::win32::project_apis(
        &dynwinrt_codegen::win32_metadata::RawApis {
            namespace: raw.namespace,
            class_name: raw.class_name,
            functions: vec![function.clone()],
        },
    );
    assert_eq!(projection.complete_count(), 0);
    assert!(
        projection.omitted[0]
            .identity
            .ends_with("::PropVariantToUInt32VectorAlloc")
    );
}

#[test]
fn sid_data_pointer_and_color_scalar_are_not_handles() {
    let Some(winmd) = configured_winmd() else {
        return;
    };
    let security =
        dynwinrt_codegen::win32_metadata::parse_apis(&winmd, "Windows.Win32.Security", "Apis")
            .unwrap();
    let is_valid_sid = security
        .functions
        .iter()
        .find(|function| function.name == "IsValidSid")
        .expect("IsValidSid metadata");
    let sid = is_valid_sid
        .parameters
        .iter()
        .find(|parameter| parameter.name == "pSid")
        .expect("pSid metadata");
    assert!(matches!(
        sid.typ.base,
        dynwinrt_codegen::win32_metadata::RawBaseType::Named {
            kind: dynwinrt_codegen::win32_metadata::RawNamedKind::DataPointer,
            ..
        }
    ));

    let gdi =
        dynwinrt_codegen::win32_metadata::parse_apis(&winmd, "Windows.Win32.Graphics.Gdi", "Apis")
            .unwrap();
    let get_pixel = gdi
        .functions
        .iter()
        .find(|function| function.name == "GetPixel")
        .expect("GetPixel metadata");
    assert_eq!(
        get_pixel.return_type.base,
        dynwinrt_codegen::win32_metadata::RawBaseType::Scalar(
            dynwinrt_codegen::win32_metadata::RawScalar::U32
        )
    );
}

#[test]
fn flat_win32_cli_rejects_python_generation() {
    let Some(winmd) = configured_winmd() else {
        return;
    };
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args([
            "generate",
            "--winmd",
            &winmd,
            "--namespace",
            "Windows.Win32.System.Registry",
            "--class-name",
            "Apis",
            "--lang",
            "py",
            "--dry-run",
        ])
        .output()
        .expect("run dynwinrt-codegen");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("`--lang py` is not supported"), "{stderr}");
    assert!(
        stderr.contains("flat Win32 DllImport container"),
        "{stderr}"
    );
}

#[test]
fn flat_win32_namespace_mode_routes_to_flat_projection() {
    let Some(winmd) = configured_winmd() else {
        return;
    };
    let js = std::process::Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args([
            "generate",
            "--winmd",
            &winmd,
            "--namespace",
            "Windows.Win32.System.SystemInformation",
            "--dry-run",
        ])
        .output()
        .expect("run dynwinrt-codegen");
    assert!(
        js.status.success(),
        "{}",
        String::from_utf8_lossy(&js.stderr)
    );
    assert!(
        String::from_utf8_lossy(&js.stdout)
            .contains("Would generate flat Win32 Windows.Win32.System.SystemInformation.Apis")
    );

    let py = std::process::Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args([
            "generate",
            "--winmd",
            &winmd,
            "--namespace",
            "Windows.Win32.System.SystemInformation",
            "--lang",
            "py",
            "--dry-run",
        ])
        .output()
        .expect("run dynwinrt-codegen");
    assert!(!py.status.success());
    assert!(
        String::from_utf8_lossy(&py.stderr).contains("is not supported for flat Win32 namespace")
    );
}

#[test]
fn reserved_and_double_null_metadata_drive_projection() {
    let Some(winmd) = configured_winmd() else {
        return;
    };
    let com =
        dynwinrt_codegen::win32_metadata::parse_apis(&winmd, "Windows.Win32.System.Com", "Apis")
            .unwrap();
    let (com_output, com_omissions) = dynwinrt_codegen::codegen::win32::generate_apis_files(
        &dynwinrt_codegen::win32_metadata::RawApis {
            namespace: com.namespace,
            class_name: com.class_name,
            functions: com
                .functions
                .into_iter()
                .filter(|function| {
                    matches!(
                        function.name.as_str(),
                        "CoDisconnectObject" | "CoFreeUnusedLibrariesEx"
                    )
                })
                .collect(),
        },
        "@microsoft/dynwinrt/win32",
    );
    assert!(com_omissions.is_empty(), "{com_omissions:#?}");
    assert!(
        com_output
            .dts
            .contains("coDisconnectObject(unk: DynWinRtValue)")
    );
    assert!(
        com_output
            .dts
            .contains("coFreeUnusedLibrariesEx(dwUnloadDelay: number): void")
    );
    assert!(com_output.js.contains("DynWin32.u32(0)"));
    assert!(!com_output.dts.contains("dwReserved"));

    let globalization =
        dynwinrt_codegen::win32_metadata::parse_apis(&winmd, "Windows.Win32.Globalization", "Apis")
            .unwrap();
    let (globalization_output, globalization_omissions) =
        dynwinrt_codegen::codegen::win32::generate_apis_files(
            &dynwinrt_codegen::win32_metadata::RawApis {
                namespace: globalization.namespace,
                class_name: globalization.class_name,
                functions: globalization
                    .functions
                    .into_iter()
                    .filter(|function| function.name == "SetProcessPreferredUILanguages")
                    .collect(),
            },
            "@microsoft/dynwinrt/win32",
        );
    assert!(
        globalization_omissions.is_empty(),
        "{globalization_omissions:#?}"
    );
    assert!(
        globalization_output
            .dts
            .contains("string | readonly string[] | Buffer | Uint8Array | null")
    );
    assert!(
        globalization_output
            .js
            .contains("DynWin32.wideMultiString(pwszLanguagesBuffer, true)")
    );
}

#[test]
fn opaque_byte_sized_iphelper_buffer_projects_safely() {
    let Some(winmd) = configured_winmd() else {
        return;
    };
    let raw = dynwinrt_codegen::win32_metadata::parse_apis(
        &winmd,
        "Windows.Win32.NetworkManagement.IpHelper",
        "Apis",
    )
    .unwrap();
    let function = raw
        .functions
        .iter()
        .find(|function| function.name == "GetAdaptersAddresses")
        .expect("GetAdaptersAddresses metadata")
        .clone();
    let selected = dynwinrt_codegen::win32_metadata::RawApis {
        namespace: raw.namespace,
        class_name: raw.class_name,
        functions: vec![function],
    };
    let (output, omissions) = dynwinrt_codegen::codegen::win32::generate_apis_files(
        &selected,
        "@microsoft/dynwinrt/win32",
    );
    assert!(omissions.is_empty(), "{omissions:#?}");
    assert!(output.dts.contains("getAdaptersAddresses("));
    assert!(output.dts.contains("Buffer | Uint8Array | null"));
    assert!(
        output
            .js
            .contains("DynWin32.alignedDataPointer(adapterAddresses, 8, true)")
    );
}

#[test]
fn exact_string_buffer_overrides_generate_queryable_surfaces() {
    let Some(winmd) = configured_winmd() else {
        return;
    };
    let raw =
        dynwinrt_codegen::win32_metadata::parse_apis(&winmd, "Windows.Win32.Globalization", "Apis")
            .unwrap();
    let functions = raw
        .functions
        .iter()
        .filter(|function| function.name == "LCMapStringA")
        .cloned()
        .collect();
    let selected = dynwinrt_codegen::win32_metadata::RawApis {
        namespace: raw.namespace,
        class_name: raw.class_name,
        functions,
    };
    let (output, omissions) = dynwinrt_codegen::codegen::win32::generate_apis_files(
        &selected,
        "@microsoft/dynwinrt/win32",
    );
    assert!(omissions.is_empty(), "{omissions:#?}");
    assert!(
        output
            .dts
            .contains("lcMapStringA(locale: number, dwMapFlags:")
    );
    assert!(output.dts.contains("destStr: Buffer | Uint8Array | null"));
    assert!(output.js.contains("_bufferCount(destStr, 1)"));
}

#[test]
fn nested_system_info_union_resolves_but_pointer_fields_stay_closed() {
    let Some(winmd) = configured_winmd() else {
        return;
    };
    let raw = dynwinrt_codegen::win32_metadata::parse_apis(
        &winmd,
        "Windows.Win32.System.SystemInformation",
        "Apis",
    )
    .unwrap();
    let function = raw
        .functions
        .iter()
        .find(|function| function.name == "GetSystemInfo")
        .expect("GetSystemInfo metadata")
        .clone();
    let parameter = &function.parameters[0];
    let dynwinrt_codegen::win32_metadata::RawBaseType::Named {
        kind: dynwinrt_codegen::win32_metadata::RawNamedKind::NativeStruct { layout },
        ..
    } = &parameter.typ.base
    else {
        panic!("SYSTEM_INFO native layout");
    };
    assert!(layout.variants[0].fields.iter().any(|field| {
        matches!(
            &field.typ.base,
            dynwinrt_codegen::win32_metadata::RawBaseType::Named {
                name,
                kind: dynwinrt_codegen::win32_metadata::RawNamedKind::NativeStruct { .. },
                ..
            } if name.contains("_Anonymous_e__Union")
        )
    }));

    let projection = dynwinrt_codegen::codegen::win32::project_apis(
        &dynwinrt_codegen::win32_metadata::RawApis {
            namespace: raw.namespace,
            class_name: raw.class_name,
            functions: vec![function],
        },
    );
    assert_eq!(projection.complete_count(), 0);
    assert!(
        projection.omitted[0]
            .reason
            .contains("retained pointee ownership")
    );

    let input = dynwinrt_codegen::win32_metadata::parse_apis(
        &winmd,
        "Windows.Win32.UI.Input.KeyboardAndMouse",
        "Apis",
    )
    .unwrap();
    let send_input = input
        .functions
        .iter()
        .find(|function| function.name == "SendInput")
        .expect("SendInput metadata")
        .clone();
    let (output, omissions) = dynwinrt_codegen::codegen::win32::generate_apis_files(
        &dynwinrt_codegen::win32_metadata::RawApis {
            namespace: input.namespace,
            class_name: input.class_name,
            functions: vec![send_input],
        },
        "@microsoft/dynwinrt/win32",
    );
    assert!(omissions.is_empty(), "{omissions:#?}");
    assert!(output.dts.contains("sendInput("));
    assert!(output.js.contains("_bufferCount(inputs,"));
}

#[test]
fn exact_direct_handle_ownership_projects_managed_resources() {
    let Some(winmd) = configured_winmd() else {
        return;
    };
    for (namespace, functions, expected_cleanup) in [
        (
            "Windows.Win32.System.Memory",
            &["LocalAlloc", "GlobalAlloc"][..],
            &["localFree", "globalFree"][..],
        ),
        (
            "Windows.Win32.System.LibraryLoader",
            &["LoadLibraryW"][..],
            &["freeLibrary"][..],
        ),
        (
            "Windows.Win32.System.Services",
            &["OpenSCManagerW"][..],
            &["closeServiceHandle"][..],
        ),
    ] {
        let raw = dynwinrt_codegen::win32_metadata::parse_apis(&winmd, namespace, "Apis").unwrap();
        let selected = dynwinrt_codegen::win32_metadata::RawApis {
            namespace: raw.namespace,
            class_name: raw.class_name,
            functions: raw
                .functions
                .into_iter()
                .filter(|function| functions.contains(&function.name.as_str()))
                .collect(),
        };
        let (output, omissions) = dynwinrt_codegen::codegen::win32::generate_apis_files(
            &selected,
            "@microsoft/dynwinrt/win32",
        );
        assert!(omissions.is_empty(), "{namespace}: {omissions:#?}");
        for cleanup in expected_cleanup {
            assert!(
                output.js.contains(&format!("returnCleanup: \"{cleanup}\"")),
                "{namespace}: {}",
                output.js
            );
        }
        assert!(output.dts.contains("DynWin32Resource | null"));
    }
}

#[test]
fn pointer_struct_builders_and_overlapped_io_project_exact_surfaces() {
    let Some(winmd) = configured_winmd() else {
        return;
    };

    let pipes =
        dynwinrt_codegen::win32_metadata::parse_apis(&winmd, "Windows.Win32.System.Pipes", "Apis")
            .unwrap();
    let (pipes_output, pipes_omissions) = dynwinrt_codegen::codegen::win32::generate_apis_files(
        &dynwinrt_codegen::win32_metadata::RawApis {
            namespace: pipes.namespace,
            class_name: pipes.class_name,
            functions: pipes
                .functions
                .into_iter()
                .filter(|function| function.name == "CreatePipe")
                .collect(),
        },
        "@microsoft/dynwinrt/win32",
    );
    assert!(pipes_omissions.is_empty(), "{pipes_omissions:#?}");
    assert!(
        pipes_output.dts.contains(
            "createSecurityAttributes(init?: SecurityAttributesInit): SECURITY_ATTRIBUTES"
        )
    );
    assert!(pipes_output.dts.contains("createPipe("));
    assert!(pipes_output.js.contains("DynWin32.setNativeStructPointer"));

    let threading = dynwinrt_codegen::win32_metadata::parse_apis(
        &winmd,
        "Windows.Win32.System.Threading",
        "Apis",
    )
    .unwrap();
    let (process_output, process_omissions) = dynwinrt_codegen::codegen::win32::generate_apis_files(
        &dynwinrt_codegen::win32_metadata::RawApis {
            namespace: threading.namespace,
            class_name: threading.class_name,
            functions: threading
                .functions
                .into_iter()
                .filter(|function| function.name == "CreateProcessW")
                .collect(),
        },
        "@microsoft/dynwinrt/win32",
    );
    assert!(process_omissions.is_empty(), "{process_omissions:#?}");
    assert!(process_output.dts.contains("createStartupInfoW("));
    assert!(process_output.dts.contains("createProcessInformation("));
    assert!(
        process_output
            .dts
            .contains("takeProcessInformationProcess(")
    );
    assert!(
        process_output
            .dts
            .contains("getProcessInformationProcessId(")
    );

    let files = dynwinrt_codegen::win32_metadata::parse_apis(
        &winmd,
        "Windows.Win32.Storage.FileSystem",
        "Apis",
    )
    .unwrap();
    let read_file = files
        .functions
        .iter()
        .find(|function| function.name == "ReadFile")
        .expect("ReadFile metadata")
        .clone();
    let duplicate_projection = dynwinrt_codegen::codegen::win32::project_apis(
        &dynwinrt_codegen::win32_metadata::RawApis {
            namespace: files.namespace.clone(),
            class_name: files.class_name.clone(),
            functions: vec![read_file.clone(), read_file],
        },
    );
    assert_eq!(duplicate_projection.complete_count(), 0);
    assert_eq!(duplicate_projection.omitted.len(), 2);
    assert!(duplicate_projection.omitted.iter().all(|omission| {
        omission
            .reason
            .contains("overload or architecture collision")
    }));
    let (file_output, file_omissions) = dynwinrt_codegen::codegen::win32::generate_apis_files(
        &dynwinrt_codegen::win32_metadata::RawApis {
            namespace: files.namespace,
            class_name: files.class_name,
            functions: files
                .functions
                .into_iter()
                .filter(|function| matches!(function.name.as_str(), "ReadFile" | "WriteFile"))
                .collect(),
        },
        "@microsoft/dynwinrt/win32",
    );
    assert!(file_omissions.is_empty(), "{file_omissions:#?}");
    assert_eq!(
        file_output.dts.matches("function readFileAsync(").count(),
        1
    );
    assert_eq!(
        file_output.dts.matches("function writeFileAsync(").count(),
        1
    );
    assert!(!file_output.dts.contains("function readFile("));
    assert!(
        file_output
            .js
            .contains("DynWin32.beginReadFile(file, buffer")
    );
    assert!(
        file_output
            .js
            .contains("operation.start((error, bytesTransferred)")
    );
    assert!(!file_output.js.contains("operation.promise()"));
    assert!(file_output.js.contains("signal must be an AbortSignal"));
    assert!(
        file_output
            .js
            .contains("catch (error) { return Promise.reject(error) }")
    );
    assert!(
        file_output
            .js
            .contains("message.includes('Win32 error 995')")
    );
    assert!(file_output.js.contains("error.name = 'AbortError'"));
}
