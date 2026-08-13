// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! TDD tests for classic-COM (option A) code generation from Windows.Win32.winmd.
//!
//! These tests drive the implementation of:
//! - Base-aware vtable slot computation (walks interface_impls chain)
//! - IUnknown vs IInspectable base offset (3 vs 6)
//! - Coclass CLSID discovery and newable coclass wrappers
//! - Natural TS/JS wrapper generation for classic-COM interfaces and coclasses
//!
//! Tests are skipped (with an `eprintln!` note) when the Win32 winmd is not
//! present at the well-known path.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use dynwinrt_codegen::codegen::com;
use dynwinrt_codegen::codegen::project::{get_import_name, set_import_name};
use dynwinrt_codegen::com_metadata;
use dynwinrt_codegen::com_metadata::{RawNativeType, RawParamDirection};
use dynwinrt_codegen::meta;
use dynwinrt_codegen::types::TypeMeta;

/// Path to `Windows.Win32.winmd`. Overridable via the `DYNWINRT_WIN32_WINMD`
/// environment variable so this suite can run on CI and other machines without
/// editing the source; falls back to the common local checkout path.
fn win32_winmd() -> String {
    std::env::var("DYNWINRT_WIN32_WINMD")
        .unwrap_or_else(|_| r"C:\s\win32metadata\Windows.Win32.winmd".to_string())
}

fn win32_available() -> bool {
    Path::new(&win32_winmd()).exists()
}

fn isolate_com_method(
    interface: &com_metadata::ComInterfaceMeta,
    method_name: &str,
) -> com_metadata::ComInterfaceMeta {
    let raw_index = interface
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .position(|method| method.metadata_name == method_name)
        .unwrap();
    let method_index = interface
        .interface
        .methods
        .iter()
        .position(|method| method.name == method_name)
        .unwrap();
    let mut isolated = interface.clone();
    isolated.raw_methods = Some(vec![
        isolated.raw_methods.as_ref().unwrap()[raw_index].clone(),
    ]);
    isolated.interface.methods = vec![isolated.interface.methods[method_index].clone()];
    isolated.own_methods_start = 0;
    isolated
}

fn project_isolated_com_method(
    namespace: &str,
    interface_name: &str,
    method_name: &str,
) -> com::ComGeneratedOutput {
    let interface =
        com_metadata::parse_com_interface(&win32_winmd(), namespace, interface_name).unwrap();
    com::generate_com_interface_files(&isolate_com_method(&interface, method_name), &win32_winmd())
        .unwrap()
}

#[test]
fn required_win32_metadata_is_present() {
    if std::env::var("DYNWINRT_REQUIRE_WIN32_METADATA").as_deref() == Ok("1") {
        assert!(
            win32_available(),
            "DYNWINRT_REQUIRE_WIN32_METADATA=1 but metadata is missing at {}",
            win32_winmd()
        );
    }
}

#[test]
fn real_metadata_preserves_automation_pointer_contracts_and_fails_closed() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let property_bag = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.Com.StructuredStorage",
        "IPropertyBag",
    )
    .expect("IPropertyBag must exist");
    let read = property_bag
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .find(|method| method.metadata_name == "Read")
        .unwrap();
    let variant = read
        .params
        .iter()
        .find(|param| param.name == "pVar")
        .unwrap();
    assert_eq!(variant.direction, RawParamDirection::InOut);
    assert_eq!(variant.typ.pointer_depth, 1);
    assert!(matches!(
        &variant.typ.native_type,
        RawNativeType::Named { namespace, name, .. }
            if namespace == "Windows.Win32.System.Variant" && name == "VARIANT"
    ));
    let error = com::generate_com_interface_files(&property_bag, &win32_winmd()).unwrap_err();
    assert!(error.contains("Automation BYREF/InOut"), "{error}");

    let property_store = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell.PropertiesSystem",
        "IPropertyStore",
    )
    .expect("IPropertyStore must exist");
    let get_value = property_store
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .find(|method| method.metadata_name == "GetValue")
        .unwrap();
    let propvariant = get_value
        .params
        .iter()
        .find(|param| param.name == "pv")
        .unwrap();
    assert_eq!(propvariant.direction, RawParamDirection::Out);
    assert_eq!(propvariant.typ.pointer_depth, 1);
    assert!(matches!(
        &propvariant.typ.native_type,
        RawNativeType::Named { namespace, name, .. }
            if namespace == "Windows.Win32.System.Com.StructuredStorage"
                && name == "PROPVARIANT"
    ));

    let context = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.Wmi",
        "IWbemContext",
    )
    .expect("IWbemContext must exist");
    let get_names = context
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .find(|method| method.metadata_name == "GetNames")
        .unwrap();
    let safe_array = get_names
        .params
        .iter()
        .find(|param| param.name == "pNames")
        .unwrap();
    assert_eq!(safe_array.direction, RawParamDirection::Out);
    assert_eq!(safe_array.typ.pointer_depth, 2);
    assert!(matches!(
        &safe_array.typ.native_type,
        RawNativeType::Named { namespace, name, .. }
            if namespace == "Windows.Win32.System.Com" && name == "SAFEARRAY"
    ));
    let raw_get_names = context
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .position(|method| method.metadata_name == "GetNames")
        .unwrap();
    let projected_get_names = context
        .interface
        .methods
        .iter()
        .position(|method| method.name == "GetNames")
        .unwrap();
    let mut isolated_get_names = context.clone();
    isolated_get_names.raw_methods = Some(vec![
        isolated_get_names.raw_methods.as_ref().unwrap()[raw_get_names].clone(),
    ]);
    isolated_get_names.interface.methods =
        vec![isolated_get_names.interface.methods[projected_get_names].clone()];
    isolated_get_names.own_methods_start = 0;
    let output = com::generate_com_interface_files(&isolated_get_names, &win32_winmd())
        .expect("documented IWbemContext.GetNames BSTR SAFEARRAY must project");
    assert!(output.js.contains("DynCom.safeArrayType('bstr')"));
    isolated_get_names.raw_methods.as_mut().unwrap()[0]
        .params
        .iter_mut()
        .find(|param| param.name == "pNames")
        .unwrap()
        .typ
        .pointer_depth = 0;
    let error = com::generate_com_interface_files(&isolated_get_names, &win32_winmd())
        .expect_err("bare SAFEARRAY output must fail closed");
    assert!(
        error.contains("SAFEARRAY signature no longer matches exact documented evidence"),
        "{error}"
    );
    isolated_get_names.raw_methods.as_mut().unwrap()[0]
        .params
        .iter_mut()
        .find(|param| param.name == "pNames")
        .unwrap()
        .typ
        .pointer_depth = 2;
    isolated_get_names.raw_methods.as_mut().unwrap()[0].declaring_interface =
        "INoSafeArrayEvidence".into();
    let error = com::generate_com_interface_files(&isolated_get_names, &win32_winmd())
        .expect_err("SAFEARRAY declaration identity drift must fail closed");
    assert!(
        error.contains("SAFEARRAY evidence is no longer registered"),
        "{error}"
    );

    let type_comp =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.System.Com", "ITypeComp")
            .expect("ITypeComp must exist");
    let bind = type_comp
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .find(|method| method.metadata_name == "Bind")
        .unwrap();
    let bind_ptr = bind
        .params
        .iter()
        .find(|param| param.name == "pBindPtr")
        .unwrap();
    assert_eq!(bind_ptr.direction, RawParamDirection::Out);
    assert_eq!(bind_ptr.typ.pointer_depth, 1);
    assert!(matches!(
        &bind_ptr.typ.native_type,
        RawNativeType::Named {
            namespace,
            name,
            layout: Some(layout),
            ..
        } if namespace == "Windows.Win32.System.Com"
            && name == "BINDPTR"
            && layout.variants.iter().all(|variant| variant.is_union)
    ));
    let error = com::generate_com_interface_files(&type_comp, &win32_winmd()).unwrap_err();
    assert!(
        error.contains("requires native layout projection"),
        "{error}"
    );
}

#[test]
fn real_metadata_projects_required_variant_by_value_inputs() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let accessible = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Accessibility",
        "IAccessible",
    )
    .expect("IAccessible must exist");
    assert_eq!(
        accessible.interface.iid,
        "618736e0-3c3d-11cf-810c-00aa00389b71"
    );
    let acc_select = accessible
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .find(|method| method.metadata_name == "accSelect")
        .unwrap();
    assert_eq!(acc_select.vtable_index, 21);
    let child = acc_select
        .params
        .iter()
        .find(|param| param.name == "varChild")
        .unwrap();
    assert_eq!(child.direction, RawParamDirection::In);
    assert!(!child.optional);
    assert_eq!(child.typ.pointer_depth, 0);
    assert!(matches!(
        &child.typ.native_type,
        RawNativeType::Named {
            namespace,
            name,
            ..
        } if namespace == "Windows.Win32.System.Variant" && name == "VARIANT"
    ));

    let isolated_accessible = isolate_com_method(&accessible, "accSelect");
    let output = com::generate_com_interface_files(&isolated_accessible, &win32_winmd())
        .expect("IAccessible::accSelect by-value VARIANT must project");
    assert!(output.js.contains(
        ".addMethodAt(21, 'accSelect', new DynComMethodSig().addIn(DynCom.i32Type()).addIn(DynCom.variantByValueType()))"
    ));
    assert!(output.js.contains("DynCom.variant(varChild)"));
    assert!(
        output
            .dts
            .contains("accSelect(flagsSelect: number, varChild: DynComVariant): void;")
    );
    assert!(!output.dts.contains("varChild: DynComVariant | null"));

    let full_accessible = com::generate_com_interface_files(&accessible, &win32_winmd())
        .expect("complete IAccessible must project after by-value VARIANT support");
    assert!(full_accessible.js.contains(".addMethodAt(21, 'accSelect'"));
    assert!(
        full_accessible
            .dts
            .contains("get_accName(varChild: DynComVariant): string;")
    );

    let automation = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Accessibility",
        "IUIAutomation",
    )
    .expect("IUIAutomation must exist");
    assert_eq!(
        automation.interface.iid,
        "30cbe57d-d9d0-452a-ab13-7ac5ac4825ee"
    );
    let create_property_condition = automation
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .find(|method| method.metadata_name == "CreatePropertyCondition")
        .unwrap();
    assert_eq!(create_property_condition.vtable_index, 23);
    let value = create_property_condition
        .params
        .iter()
        .find(|param| param.name == "value")
        .unwrap();
    assert_eq!(value.direction, RawParamDirection::In);
    assert!(!value.optional);
    assert_eq!(value.typ.pointer_depth, 0);
    assert!(matches!(
        &value.typ.native_type,
        RawNativeType::Named {
            namespace,
            name,
            ..
        } if namespace == "Windows.Win32.System.Variant" && name == "VARIANT"
    ));

    let isolated_automation = isolate_com_method(&automation, "CreatePropertyCondition");
    let output = com::generate_com_interface_files(&isolated_automation, &win32_winmd())
        .expect("IUIAutomation::CreatePropertyCondition by-value VARIANT must project");
    assert!(output.js.contains(
        ".addMethodAt(23, 'CreatePropertyCondition', new DynComMethodSig().addIn(DynCom.i32Type()).addIn(DynCom.variantByValueType()).addOut(DynCom.interfaceType(WinGuid.parse('352ffba8-0973-437c-a61f-f64cafd81df9'))))"
    ));
    assert!(output.js.contains("DynCom.variant(value)"));
    assert!(output.dts.contains(
        "createPropertyCondition(propertyId: UIA_PROPERTY_ID, value: DynComVariant): DynWinRtValue;"
    ));

    let error = com::generate_com_interface_files(&automation, &win32_winmd())
        .expect_err("complete IUIAutomation must retain its next fail-closed blocker");
    assert!(error.contains("ElementFromPoint parameter `pt`"), "{error}");
    assert!(error.contains("native layout projection"), "{error}");

    let mut optional = isolated_automation.clone();
    optional.raw_methods.as_mut().unwrap()[0]
        .params
        .iter_mut()
        .find(|param| param.name == "value")
        .unwrap()
        .optional = true;
    let error = com::generate_com_interface_files(&optional, &win32_winmd())
        .expect_err("optional aggregate VARIANT cannot fabricate null/default semantics");
    assert!(
        error.contains(
            "optional by-value VARIANT input has no proven native null/default semantics"
        ),
        "{error}"
    );

    let mut wrong_direction = isolated_automation;
    wrong_direction.raw_methods.as_mut().unwrap()[0]
        .params
        .iter_mut()
        .find(|param| param.name == "value")
        .unwrap()
        .direction = RawParamDirection::Out;
    let error = com::generate_com_interface_files(&wrong_direction, &win32_winmd())
        .expect_err("bare VARIANT output must require pointer metadata");
    assert!(error.contains("VARIANT"), "{error}");
    assert!(
        error.contains("requires native layout projection"),
        "{error}"
    );

    let property_bag = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.Com.StructuredStorage",
        "IPropertyBag",
    )
    .expect("IPropertyBag must exist");
    let isolated_write = isolate_com_method(&property_bag, "Write");
    let pointer_variant = isolated_write.raw_methods.as_ref().unwrap()[0]
        .params
        .iter()
        .find(|param| param.name == "pVar")
        .unwrap();
    assert_eq!(pointer_variant.direction, RawParamDirection::In);
    assert_eq!(pointer_variant.typ.pointer_depth, 1);
    let output = com::generate_com_interface_files(&isolated_write, &win32_winmd())
        .expect("existing VARIANT pointer input must remain supported");
    assert!(output.js.contains(".addIn(DynCom.variantType())"));
    assert!(!output.js.contains("variantByValueType"));
}

#[test]
fn real_metadata_projects_exact_documented_safearray_families() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    for (namespace, interface, method, expected_abi) in [
        (
            "Windows.Win32.UI.Accessibility",
            "IAccessibleEx",
            "GetRuntimeId",
            "DynCom.safeArrayType('i32')",
        ),
        (
            "Windows.Win32.UI.Accessibility",
            "IUIAutomationProxyFactoryEntry",
            "SetWinEventsForAutomationEvent",
            "DynCom.safeArrayType('u32')",
        ),
        (
            "Windows.Win32.UI.Accessibility",
            "ITextRangeProvider",
            "GetBoundingRectangles",
            "DynCom.safeArrayType('f64')",
        ),
        (
            "Windows.Win32.Media.DirectShow",
            "IESEvent",
            "GetData",
            "DynCom.safeArrayType('u8')",
        ),
        (
            "Windows.Win32.System.Wmi",
            "IWbemContext",
            "GetNames",
            "DynCom.safeArrayType('bstr')",
        ),
        (
            "Windows.Win32.System.Performance",
            "IPerformanceCounterDataCollector",
            "get_PerformanceCounters",
            "DynCom.safeArrayType('bstr')",
        ),
        (
            "Windows.Win32.System.Performance",
            "ITraceDataProvider",
            "get_FilterData",
            "DynCom.safeArrayType('u8')",
        ),
        (
            "Windows.Win32.System.RemoteDesktop",
            "IWorkspaceResTypeRegistry",
            "GetRegisteredFileExtensions",
            "DynCom.safeArrayType('bstr')",
        ),
        (
            "Windows.Win32.System.WindowsProgramming",
            "ICameraUIControl",
            "GetSelectedItems",
            "DynCom.safeArrayType('bstr')",
        ),
        (
            "Windows.Win32.Storage.FileServerResourceManager",
            "IFsrmActionReport",
            "get_ReportTypes",
            "DynCom.safeArrayType('variant')",
        ),
        (
            "Windows.Win32.UI.Accessibility",
            "ITextProvider",
            "GetVisibleRanges",
            "DynCom.safeArrayType('unknown', WinGuid.parse('5347ad7b-c355-46f8-aff5-909033582f63'))",
        ),
        (
            "Windows.Win32.UI.Accessibility",
            "IUIAutomationClientInfoSource",
            "GetConnectedClients",
            "DynCom.safeArrayType('unknown', WinGuid.parse('b2e8a3f1-4c5d-4e7a-8f6b-3d2e1c9a0b8f'))",
        ),
    ] {
        let output = project_isolated_com_method(namespace, interface, method);
        assert!(output.js.contains(expected_abi), "{interface}.{method}");
        assert!(
            output.dts.contains("DynComSafeArray"),
            "{interface}.{method}"
        );
    }

    for interface in [
        "IAlertDataCollector",
        "IApiTracingDataCollector",
        "IConfigurationDataCollector",
        "IDataCollectorSet",
        "IPerformanceCounterDataCollector",
        "ITraceDataProvider",
    ] {
        let parsed = com_metadata::parse_com_interface(
            &win32_winmd(),
            "Windows.Win32.System.Performance",
            interface,
        )
        .unwrap();
        com::generate_com_interface_files(&parsed, &win32_winmd())
            .unwrap_or_else(|error| panic!("{interface} must project completely: {error}"));
    }

    let client_info = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Accessibility",
        "IUIAutomationClientInfo",
    )
    .unwrap();
    assert_eq!(
        client_info.interface.iid,
        "b2e8a3f1-4c5d-4e7a-8f6b-3d2e1c9a0b8f"
    );
}

#[test]
fn text_provider_selection_preserves_its_documented_nullable_safearray() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let output = project_isolated_com_method(
        "Windows.Win32.UI.Accessibility",
        "ITextProvider",
        "GetSelection",
    );
    assert!(output.js.contains(
        "DynCom.safeArrayType('unknown', WinGuid.parse('5347ad7b-c355-46f8-aff5-909033582f63'), true)"
    ));
    assert!(output.js.contains("DynCom.takeNullableSafeArray("));
    assert!(
        output
            .dts
            .contains("getSelection(): DynComSafeArray | null;")
    );
}

#[test]
fn fragment_embedded_roots_preserve_their_documented_nullable_safearray() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let output = project_isolated_com_method(
        "Windows.Win32.UI.Accessibility",
        "IRawElementProviderFragment",
        "GetEmbeddedFragmentRoots",
    );
    assert!(output.js.contains(
        "DynCom.safeArrayType('unknown', WinGuid.parse('620ce2a5-ab8f-40a9-86cb-de3c75599b58'), true)"
    ));
    assert!(output.js.contains("DynCom.takeNullableSafeArray("));
    assert!(
        output
            .dts
            .contains("getEmbeddedFragmentRoots(): DynComSafeArray | null;")
    );

    let grabbed = project_isolated_com_method(
        "Windows.Win32.UI.Accessibility",
        "IDragProvider",
        "GetGrabbedItems",
    );
    assert!(grabbed.js.contains(
        "DynCom.safeArrayType('unknown', WinGuid.parse('d6dd68d1-86fd-4332-8666-9abedea2d24c'), true)"
    ));
    assert!(grabbed.js.contains("DynCom.takeNullableSafeArray("));
    assert!(
        grabbed
            .dts
            .contains("getGrabbedItems(): DynComSafeArray | null;")
    );

    let runtime_id = project_isolated_com_method(
        "Windows.Win32.UI.Accessibility",
        "IRawElementProviderFragment",
        "GetRuntimeId",
    );
    assert!(
        runtime_id
            .js
            .contains("DynCom.safeArrayType('i32', undefined, true)")
    );
    assert!(runtime_id.js.contains("DynCom.takeNullableSafeArray("));
    assert!(
        runtime_id
            .dts
            .contains("getRuntimeId(): DynComSafeArray | null;")
    );

    let hosting = project_isolated_com_method(
        "Windows.Win32.UI.Accessibility",
        "IAccessibleHostingElementProviders",
        "GetEmbeddedFragmentRoots",
    );
    assert!(hosting.js.contains(
        "DynCom.safeArrayType('unknown', WinGuid.parse('620ce2a5-ab8f-40a9-86cb-de3c75599b58'))"
    ));
    assert!(!hosting.js.contains("DynCom.takeNullableSafeArray("));
    assert!(
        hosting
            .dts
            .contains("getEmbeddedFragmentRoots(): DynComSafeArray;")
    );

    let fragment = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Accessibility",
        "IRawElementProviderFragment",
    )
    .unwrap();
    let mut drifted = isolate_com_method(&fragment, "GetEmbeddedFragmentRoots");
    drifted.raw_methods.as_mut().unwrap()[0].params[0].name = "changed".into();
    let error = com::generate_com_interface_files(&drifted, &win32_winmd())
        .expect_err("nullable SAFEARRAY signature drift must fail closed");
    assert!(
        error.contains("SAFEARRAY signature no longer matches exact documented evidence"),
        "{error}"
    );
}

#[test]
fn nearest_unproven_safearray_shapes_remain_fail_closed() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    for (namespace, interface, method) in [
        (
            "Windows.Win32.UI.TabletPC",
            "ITextInputPanelEventSink",
            "TextInserting",
        ),
        (
            "Windows.Win32.UI.Input.Ime",
            "IImePlugInDictDictionaryList",
            "GetDictionariesInUse",
        ),
        (
            "Windows.Win32.Networking.WinHttp",
            "IWinHttpRequestEvents",
            "OnResponseDataAvailable",
        ),
        (
            "Windows.Win32.System.ComponentServices",
            "ICOMAdminCatalog",
            "GetCollectionByQuery",
        ),
        (
            "Windows.Win32.System.TaskScheduler",
            "IEmailAction",
            "get_Attachments",
        ),
        (
            "Windows.Win32.UI.Xaml.Diagnostics",
            "IVisualTreeService",
            "GetEnums",
        ),
    ] {
        let parsed =
            com_metadata::parse_com_interface(&win32_winmd(), namespace, interface).unwrap();
        let error =
            com::generate_com_interface_files(&isolate_com_method(&parsed, method), &win32_winmd())
                .expect_err("unproven SAFEARRAY shape must fail closed");
        assert!(
            error.contains("SAFEARRAY") || error.contains("safe array"),
            "{interface}.{method}: {error}"
        );
    }
}

#[test]
fn real_metadata_projects_complete_idispatch_with_automation_compounds() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let dispatch =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.System.Com", "IDispatch")
            .expect("IDispatch must exist");
    let raw_index = dispatch
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .position(|method| method.metadata_name == "GetIDsOfNames")
        .unwrap();
    let compatibility_index = dispatch
        .interface
        .methods
        .iter()
        .position(|method| method.name == "GetIDsOfNames")
        .unwrap();
    let raw = &dispatch.raw_methods.as_ref().unwrap()[raw_index];
    let names = &raw.params[1];
    assert_eq!(names.direction, RawParamDirection::In);
    assert_eq!(names.typ.pointer_depth, 1);
    assert_eq!(
        names.native_array.as_ref().unwrap().count_param_index,
        Some(2)
    );
    assert!(matches!(
        names.string_pointer_array.as_ref(),
        Some(com_metadata::RawStringPointerArray {
            encoding: com_metadata::RawStringEncoding::Utf16,
            pointer_depth: 1,
            ownership: com_metadata::RawElementOwnership::Borrowed,
            ..
        })
    ));
    let outputs = &raw.params[4];
    assert_eq!(outputs.direction, RawParamDirection::Out);
    assert_eq!(outputs.typ.pointer_depth, 1);
    assert_eq!(
        outputs.native_array.as_ref().unwrap().count_param_index,
        Some(2)
    );

    let mut isolated = dispatch.clone();
    isolated.raw_methods = Some(vec![
        isolated.raw_methods.as_ref().unwrap()[raw_index].clone(),
    ]);
    isolated.interface.methods = vec![isolated.interface.methods[compatibility_index].clone()];
    isolated.own_methods_start = 0;
    let output = com::generate_com_interface_files(&isolated, &win32_winmd())
        .expect("IDispatch::GetIDsOfNames must project in isolation");
    assert!(output.js.contains(".addInputStringArray(true, 2)"));
    assert!(
        output
            .js
            .contains(".addCallerOutputBuffer(DynCom.i32Type(), 2, undefined, false, false)")
    );
    assert!(output.js.contains("DynCom.wideStringArray("));
    assert!(output.js.contains("DynCom.callerOutputArray("));
    assert!(output.js.contains("DynCom.takeI32Array("));
    assert_eq!(output.js.matches(".invoke(").count(), 1);
    assert!(!output.js.contains("Buffer.alloc"));
    assert!(output.dts.contains("rgszNames: string[]"));
    assert!(output.dts.contains("): number[];"));
    assert!(!output.dts.contains("cNames:"));
    assert!(!output.dts.contains("rgDispId:"));

    let invoke = dispatch
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .find(|method| method.metadata_name == "Invoke")
        .unwrap();
    assert_eq!(invoke.params.len(), 8);
    assert!(
        invoke.params[5..]
            .iter()
            .all(|param| { param.direction == RawParamDirection::Out && param.optional })
    );

    let output = com::generate_com_interface_files(&dispatch, &win32_winmd())
        .expect("complete inherited IDispatch must project");
    assert!(output.js.contains(".addMethodAt(3, 'GetTypeInfoCount'"));
    assert!(output.js.contains(".addMethodAt(4, 'GetTypeInfo'"));
    assert!(output.js.contains(".addMethodAt(5, 'GetIDsOfNames'"));
    assert!(output.js.contains(".addMethodAt(6, 'Invoke'"));
    assert!(output.js.contains(".addIn(DynCom.dispatchParamsType())"));
    assert!(output.js.contains(".addOptionalOut(DynCom.variantType())"));
    assert!(
        output
            .js
            .contains(".addOptionalOut(DynCom.excepInfoType())")
    );
    assert!(output.js.contains(".addOptionalOut(DynCom.u32Type())"));
    assert!(output.js.contains(".captureDispatchInvokeHresult()"));
    assert!(output.js.contains("DynCom.dispatchParams(dispParams)"));
    assert!(output.js.contains(".invokeDispatch(this._obj"));
    assert!(output.js.contains("DynCom.takeVariant(_resultValue)"));
    assert!(output.js.contains("DynCom.takeExcepInfo(_excepInfoValue)"));
    assert!(output.js.contains("_error.hresult = _call.hresult"));
    assert!(output.js.contains("_error.excepInfo = _excepInfo"));
    assert!(output.js.contains("_error.argErr = _call.argErr"));
    assert!(output.js.contains("_error.cause = new Error"));
    assert!(
        output
            .js
            .contains("_description ? `: ${_description}` : ''")
    );
    assert!(output.js.contains("options.result ?? true"));
    assert!(output.js.contains("options.excepInfo ?? true"));
    assert!(output.js.contains("options.argErr ?? true"));
    assert!(!output.js.contains("_result.excepInfo"));
    assert!(!output.js.contains("_result.argErr"));
    assert!(!output.js.contains("nativeStructType"));
    assert!(output.js.contains("DynCom.iidPointer(WinGuid.parse(riid))"));
    assert!(output.dts.contains(
        "invoke(dispIdMember: number, riid: string, lcid: number, wFlags: DISPATCH_FLAGS, dispParams: DynComDispatchParams, options?: { result?: boolean; excepInfo?: boolean; argErr?: boolean }): { result?: DynComVariant };"
    ));

    let invoke_raw_index = dispatch
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .position(|method| method.metadata_name == "Invoke")
        .unwrap();
    let mut invalid_dispatch_output = dispatch.clone();
    invalid_dispatch_output.raw_methods.as_mut().unwrap()[invoke_raw_index].params[4].direction =
        RawParamDirection::Out;
    let error = com::generate_com_interface_files(&invalid_dispatch_output, &win32_winmd())
        .expect_err("DISPPARAMS output must fail closed");
    assert!(
        error.contains("DISPPARAMS") || error.contains("input-only"),
        "{error}"
    );

    let mut invalid_excep_input = dispatch.clone();
    invalid_excep_input.raw_methods.as_mut().unwrap()[invoke_raw_index].params[6].direction =
        RawParamDirection::In;
    invalid_excep_input.raw_methods.as_mut().unwrap()[invoke_raw_index].params[6].optional = false;
    let error = com::generate_com_interface_files(&invalid_excep_input, &win32_winmd())
        .expect_err("EXCEPINFO input must fail closed");
    assert!(
        error.contains("EXCEPINFO") || error.contains("output-only"),
        "{error}"
    );

    let mut invalid_dispatch_indirection = dispatch;
    invalid_dispatch_indirection.raw_methods.as_mut().unwrap()[invoke_raw_index].params[4]
        .typ
        .pointer_depth = 2;
    let error = com::generate_com_interface_files(&invalid_dispatch_indirection, &win32_winmd())
        .expect_err("nested DISPPARAMS pointer indirection must fail closed");
    assert!(
        error.contains("Invoke") || error.contains("DISPPARAMS"),
        "{error}"
    );
}

/// Resolve a `Windows.winmd` from the newest installed Windows SDK, matching
/// the discovery logic the codegen itself uses. Returns `None` if no SDK is
/// installed on this machine (the test that calls this should skip in that
/// case, consistent with other tests in this module).
fn discovered_windows_winmd() -> Option<String> {
    com_metadata::discover_newest_windows_winmd()
}

fn run_codegen_command(
    metadata: &str,
    namespace: Option<&str>,
    class_names: &str,
    output_dir: &Path,
) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"));
    command
        .arg("generate")
        .arg("--winmd")
        .arg(metadata)
        .arg("--class-name")
        .arg(class_names)
        .arg("--output")
        .arg(output_dir);
    if let Some(namespace) = namespace {
        command.arg("--namespace").arg(namespace);
    }

    let output = command.output().expect("spawn dynwinrt-codegen");
    assert!(
        output.status.success(),
        "generation failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_mixed_package_shape(output_dir: &Path) {
    assert!(output_dir.join("Uri.js").is_file());
    assert!(output_dir.join("com").join("ITaskbarList3.js").is_file());
    assert!(output_dir.join("com").join("package.json").is_file());

    let root_index = fs::read_to_string(output_dir.join("index.d.ts")).unwrap();
    assert!(root_index.contains("Uri"));
    assert!(!root_index.contains("ITaskbarList3"));
    assert!(!root_index.contains("TBPFLAG"));

    let com_index = fs::read_to_string(output_dir.join("com").join("index.d.ts")).unwrap();
    assert!(com_index.contains("ITaskbarList3"));
    assert!(com_index.contains("TBPFLAG"));
    assert!(!com_index.contains("Uri"));

    let package = fs::read_to_string(output_dir.join("package.json")).unwrap();
    assert!(package.contains("\"type\": \"commonjs\""));
    assert!(package.contains("\"./Uri\""));
    assert!(package.contains("\"./com\""));
    assert!(package.contains("\"./com/*\""));
    assert!(package.contains("\"types\": \"./com/*.d.ts\""));
    assert!(package.contains("\"import\": \"./com/*.js\""));
    assert!(package.contains("\"require\": \"./com/index.js\""));
    assert!(package.contains("\"require\": \"./com/*.js\""));

    let com_package = fs::read_to_string(output_dir.join("com").join("package.json")).unwrap();
    assert!(com_package.contains("\"type\": \"commonjs\""));
    assert!(output_dir.join("com").join("index.mjs").is_file());
}

// -------------------------------------------------------------------------
// NORMAL tests
// -------------------------------------------------------------------------

/// 1. Parse ITaskbarList3 from Win32 metadata → correct IID.
#[test]
fn parse_itaskbarlist3_iid() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available at {}", &win32_winmd());
        return;
    }
    let com_iface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "ITaskbarList3",
    )
    .expect("ITaskbarList3 must exist in Win32 metadata");
    assert_eq!(com_iface.interface.name, "ITaskbarList3");
    assert_eq!(com_iface.interface.namespace, "Windows.Win32.UI.Shell");
    assert_eq!(
        com_iface.interface.iid,
        "ea1afb91-9e28-4b86-90e9-9e9f8a5eefaf"
    );
}

/// 2. Base-aware vtable slots: full interface_impls chain determines absolute slots.
///    ITaskbarList3 inherits: IUnknown (3 methods) + ITaskbarList (5) + ITaskbarList2 (1).
///    So HrInit = 3, SetProgressValue = 9, SetProgressState = 10.
#[test]
fn parse_itaskbarlist3_vtable_slots() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let com_iface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "ITaskbarList3",
    )
    .expect("ITaskbarList3 must exist");

    let by_name = |n: &str| -> usize {
        com_iface
            .interface
            .methods
            .iter()
            .find(|m| m.name == n)
            .unwrap_or_else(|| {
                panic!(
                    "method {} not found (methods: {:?})",
                    n,
                    com_iface
                        .interface
                        .methods
                        .iter()
                        .map(|m| &m.name)
                        .collect::<Vec<_>>()
                )
            })
            .vtable_index
    };

    assert_eq!(
        by_name("HrInit"),
        3,
        "HrInit is the first ITaskbarList method after IUnknown"
    );
    assert_eq!(by_name("AddTab"), 4);
    assert_eq!(by_name("DeleteTab"), 5);
    assert_eq!(by_name("ActivateTab"), 6);
    assert_eq!(by_name("SetActiveAlt"), 7);
    assert_eq!(
        by_name("MarkFullscreenWindow"),
        8,
        "ITaskbarList2's only method"
    );
    assert_eq!(by_name("SetProgressValue"), 9);
    assert_eq!(by_name("SetProgressState"), 10);
}

/// 3. Base detection: ITaskbarList3 is IUnknown-rooted → base offset (first user
///    method slot) is 3, NOT 6.
#[test]
fn itaskbarlist3_is_iunknown_rooted() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let com_iface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "ITaskbarList3",
    )
    .unwrap();
    assert_eq!(com_iface.base_offset, 3);
    assert!(com_iface.is_iunknown_rooted);
    // Base chain should include ITaskbarList2, ITaskbarList (and stop at IUnknown)
    let base_names: Vec<&str> = com_iface.base_chain.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        base_names,
        ["ITaskbarList2", "ITaskbarList", "IUnknown"],
        "base chain order matters"
    );
}

/// 4. CLSID resolution: ITaskbarList3 → TaskbarList coclass → CLSID
///    56fdf344-fd6d-11d0-958a-006097c9a090
#[test]
fn itaskbarlist3_clsid_resolution() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let com_iface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "ITaskbarList3",
    )
    .unwrap();
    assert_eq!(
        com_iface.coclass_clsid.as_deref(),
        Some("56fdf344-fd6d-11d0-958a-006097c9a090")
    );
    assert_eq!(com_iface.coclass_name.as_deref(), Some("TaskbarList"));
}

#[test]
fn taskbarlist_coclass_selects_unique_most_derived_interface() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let coclass =
        com_metadata::parse_com_coclass(&win32_winmd(), "Windows.Win32.UI.Shell", "TaskbarList")
            .unwrap()
            .expect("TaskbarList must be recognized as a COM coclass");

    assert_eq!(coclass.clsid, "56fdf344-fd6d-11d0-958a-006097c9a090");
    assert_eq!(coclass.primary_interface.interface.name, "ITaskbarList4");
    assert_eq!(
        coclass
            .associated_interfaces
            .iter()
            .map(|interface| interface.interface.name.as_str())
            .collect::<Vec<_>>(),
        [
            "ITaskbarList",
            "ITaskbarList2",
            "ITaskbarList3",
            "ITaskbarList4"
        ]
    );
}

#[test]
fn taskbarlist_coclass_generates_newable_class_and_interface_views() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let coclass =
        com_metadata::parse_com_coclass(&win32_winmd(), "Windows.Win32.UI.Shell", "TaskbarList")
            .unwrap()
            .unwrap();
    let out = com::generate_com_coclass_files(&coclass, &win32_winmd()).unwrap();

    assert!(out.js.contains("class TaskbarList extends ITaskbarList4"));
    assert!(out.js.contains("exports.TaskbarList = TaskbarList;"));
    assert!(
        out.js
            .contains("const obj = DynCom.coCreateInstance(CLSID_TaskbarList, IID_ITaskbarList4)")
    );
    assert!(out.js.contains("super(obj)"));
    assert!(out.js.contains("obj.release()"));
    assert!(out.js.contains("as(InterfaceClass)"));
    assert!(out.js.contains("DynCom.tryCast"));
    assert!(out.dts.contains("constructor();"));
    for interface in [
        "ITaskbarList.js",
        "ITaskbarList2.js",
        "ITaskbarList3.js",
        "ITaskbarList4.js",
    ] {
        assert!(
            out.extra_files.iter().any(|(name, _)| name == interface),
            "missing {interface}"
        );
    }
}

/// 5. Param type mapping: HWND → pointer/handle, TBPFLAG → enum, HRESULT → void.
#[test]
fn param_type_mapping() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let com_iface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "ITaskbarList3",
    )
    .unwrap();

    // Generate wrapper as a text bundle we can inspect for the mapping decisions
    let out = com::generate_com_interface_files(&com_iface, &win32_winmd())
        .expect("codegen must succeed for classic-COM interface");

    let dts = out.dts.as_str();
    let js = out.js.as_str();

    // HWND inputs accept Electron's pointer-width Buffer through the centralized
    // runtime conversion, while the HWND value alias remains numeric.
    assert!(
        dts.contains("export type HWND = bigint | number;"),
        "HWND value aliases should remain numeric in .d.ts, got:\n{}",
        dts
    );
    assert!(
        dts.contains("hwnd: HWND | Buffer | Uint8Array")
            && js.contains("DynCom.pointer(DynCom.handleValue(hwnd))")
            && !js.contains("function _handleArg("),
        "HWND inputs must use centralized DynCom.handleValue in .js, got:\n{}",
        js
    );

    // ULONGLONG (U64) → bigint
    // setProgressValue's completed/total params are U64
    assert!(
        dts.contains("bigint"),
        "U64 params should surface as bigint"
    );

    // TBPFLAG enum → surfaced by name (either an enum decl or a union)
    assert!(
        dts.contains("TBPFLAG") || dts.contains("TbpFlag"),
        ".d.ts must reference the TBPFLAG enum:\n{}",
        dts
    );

    // HRESULT-returning methods project to `void` (throw on failure); no HRESULT surface
    assert!(
        !dts.contains(": HRESULT")
            && !dts.contains("-> HRESULT")
            && !dts.contains("Promise<HRESULT>"),
        "HRESULT must not leak into the .d.ts surface:\n{}",
        dts
    );

    // JS body: the SetProgressState signature must include u32 (TBPFLAG's underlying) for the enum arg
    // Look for slot 10 invocation:
    assert!(
        js.contains("method(10)"),
        ".js must call vtable slot 10 for SetProgressState"
    );
    assert!(
        js.contains("method(9)"),
        ".js must call vtable slot 9 for SetProgressValue"
    );
}

#[test]
fn shelllink_scalar_out_pointers_preserve_pointee_types() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let interface =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.UI.Shell", "IShellLinkW")
            .unwrap();

    let get_show_cmd = interface
        .interface
        .methods
        .iter()
        .find(|method| method.name == "GetShowCmd")
        .unwrap();
    assert!(matches!(
        &get_show_cmd.params[0].typ,
        TypeMeta::Enum { underlying, .. } if matches!(**underlying, TypeMeta::I32)
    ));
    let get_hotkey = interface
        .interface
        .methods
        .iter()
        .find(|method| method.name == "GetHotkey")
        .unwrap();
    assert!(matches!(get_hotkey.params[0].typ, TypeMeta::U16));
    let get_icon_location = interface
        .interface
        .methods
        .iter()
        .find(|method| method.name == "GetIconLocation")
        .unwrap();
    assert!(matches!(get_icon_location.params[2].typ, TypeMeta::I32));
    let get_hotkey_vtable = get_hotkey.vtable_index;
    let get_show_cmd_vtable = get_show_cmd.vtable_index;

    let output = com::generate_com_interface_files(&interface, &win32_winmd()).unwrap();
    assert!(output.js.contains(&format!(
        ".addMethodAt({get_hotkey_vtable}, 'GetHotkey', new DynComMethodSig().addOut(DynCom.u16Type()))"
    )));
    assert!(output.js.contains(&format!(
        ".addMethodAt({get_show_cmd_vtable}, 'GetShowCmd', new DynComMethodSig().addOut(DynCom.i32Type()))"
    )));
    assert!(
        output
            .dts
            .contains("getIconLocation(cch?: number): [string, number];")
    );
}

#[test]
fn nullable_native_pod_pointer_preserves_null_projection() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let interface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "ITaskbarList3",
    )
    .unwrap();
    let output = com::generate_com_interface_files(&interface, &win32_winmd()).unwrap();

    assert!(
        output
            .dts
            .contains("setThumbnailClip(hwnd: HWND | Buffer | Uint8Array, prcClip: RECT | null)")
    );
    assert!(
        output
            .js
            .contains("DynCom.nativeStructPointerType(_nativeLayout_RECT, true)")
    );
    assert!(
        output
            .js
            .contains("prcClip === null ? DynCom.nullNativeStructPointer()")
    );
}

/// 6. Partial generation: generating a single class-name yields ONLY that
///    interface plus its immediate deps (enum, coclass metadata), NOT the
///    entire Windows.Win32.UI.Shell namespace.
#[test]
fn partial_generation_only_emits_target_interface() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let com_iface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "ITaskbarList3",
    )
    .unwrap();
    let out = com::generate_com_interface_files(&com_iface, &win32_winmd())
        .expect("codegen must succeed for classic-COM interface");

    // Expected files: ITaskbarList3.js, ITaskbarList3.d.ts, TBPFLAG.js, TBPFLAG.d.ts
    let file_names: Vec<&str> = out.extra_files.iter().map(|(n, _)| n.as_str()).collect();

    // Should NOT include unrelated Shell types like IShellItem or IApplicationActivationManager
    assert!(
        !file_names.iter().any(|n| n.starts_with("IShellItem")),
        "Partial generation must not include IShellItem: {:?}",
        file_names
    );
    assert!(
        !file_names
            .iter()
            .any(|n| n.starts_with("IApplicationActivationManager")),
        "Partial generation must not include unrelated types: {:?}",
        file_names
    );

    // Should include TBPFLAG (a direct dep)
    let has_tbpflag = file_names.iter().any(|n| n.starts_with("TBPFLAG"));
    assert!(
        has_tbpflag,
        "TBPFLAG (direct enum dep) must be included: {:?}",
        file_names
    );
}

/// 7. Generated `.d.ts` has PascalCase type + camelCase methods and
///    no raw IID/vtable-index/CoCreateInstance leaked into the TYPED surface.
#[test]
fn dts_surface_is_natural_and_clean() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let com_iface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "ITaskbarList3",
    )
    .unwrap();
    let out = com::generate_com_interface_files(&com_iface, &win32_winmd())
        .expect("codegen must succeed for classic-COM interface");
    let dts = out.dts.as_str();

    // PascalCase class name
    assert!(
        dts.contains("class ITaskbarList3"),
        ".d.ts must export class ITaskbarList3, got:\n{}",
        dts
    );

    // camelCase methods
    for cc in &["hrInit", "setProgressValue", "setProgressState", "addTab"] {
        assert!(
            dts.contains(cc),
            ".d.ts must declare camelCase method `{}`, got:\n{}",
            cc,
            dts
        );
    }
    // No PascalCase leaked method names
    for pc in &[
        "HrInit(",
        "SetProgressValue(",
        "SetProgressState(",
        "AddTab(",
    ] {
        assert!(
            !dts.contains(pc),
            ".d.ts must not expose PascalCase method `{}`, got:\n{}",
            pc,
            dts
        );
    }

    // No raw IID leak in .d.ts
    assert!(
        !dts.contains("ea1afb91-9e28-4b86-90e9-9e9f8a5eefaf"),
        "raw IID must not leak into .d.ts:\n{}",
        dts
    );
    // No raw CLSID leak
    assert!(
        !dts.contains("56fdf344-fd6d-11d0-958a-006097c9a090"),
        "raw CLSID must not leak into .d.ts:\n{}",
        dts
    );
    // No CoCreateInstance leak
    assert!(
        !dts.contains("CoCreateInstance") && !dts.contains("coCreateInstance"),
        "CoCreateInstance must not leak into .d.ts:\n{}",
        dts
    );
    // No vtable index leak in .d.ts
    for slot in &["method(3)", "method(9)", "method(10)", "vtable"] {
        assert!(
            !dts.contains(slot),
            "vtable detail `{}` must not appear in .d.ts:\n{}",
            slot,
            dts
        );
    }
}

/// 8. Generated interface `.js` contains only IID/vtable behavior; activation
///    is emitted by the separate coclass wrapper.
#[test]
fn js_body_uses_cocreateinstance_and_correct_slots() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let com_iface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "ITaskbarList3",
    )
    .unwrap();
    let out = com::generate_com_interface_files(&com_iface, &win32_winmd())
        .expect("codegen must succeed for classic-COM interface");
    let js = out.js.as_str();

    // Only the IID appears in the interface wrapper.
    assert!(!js.contains("56fdf344-fd6d-11d0-958a-006097c9a090"));
    assert!(
        js.contains("ea1afb91-9e28-4b86-90e9-9e9f8a5eefaf"),
        ".js must embed the IID:\n{}",
        js
    );

    assert!(!js.contains("coCreateInstance"));

    // Classic COM registration is kept out of the WinRT type namespace.
    assert!(
        js.contains("DynCom.registerIUnknownInterface"),
        ".js must use DynCom registration for classic COM:\n{}",
        js
    );
    assert!(js.contains("Windows.Win32.UI.Shell.ITaskbarList3"));

    // Base-aware slots
    assert!(js.contains("method(3)"), "HrInit slot 3");
    assert!(js.contains("method(9)"), "SetProgressValue slot 9");
    assert!(js.contains("method(10)"), "SetProgressState slot 10");

    // Should NOT contain WinRT `.method(6)` for a user method (that would be
    // the IInspectable-rooted slot for the first user method).
    // HrInit at slot 6 would be the failing case — we accept `method(6)` only
    // if that's ActivateTab (slot 6). ActivateTab IS at 6, so it's a valid
    // occurrence. Just check the file doesn't say something like `HrInit ... method(6)`.
    // This is covered by the exact per-method assertion above.
}

/// Requirement 4 (acronym casing): `ITaskbarList3::SetTabActive(HWND hwndTab,
/// HWND hwndMDI, DWORD dwReserved)` must project `hwndMDI` as `mdi` (the
/// whole `MDI` acronym lowercased), not the naive first-letter-only `mDI`.
#[test]
fn taskbarlist3_settabactive_projects_hwndmdi_as_mdi_not_mdi_mixed_case() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let com_iface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "ITaskbarList3",
    )
    .unwrap();
    let out = com::generate_com_interface_files(&com_iface, &win32_winmd())
        .expect("codegen must succeed for classic-COM interface");
    let js = out.js.as_str();
    assert!(
        js.contains("setTabActive(tab, mdi, reserved)"),
        ".js must lowercase the whole `MDI` acronym in `hwndMDI` -> `mdi`, not `mDI`:\n{}",
        js
    );
    assert!(
        !js.contains("mDI"),
        ".js must not contain the naive-lowering artifact `mDI`:\n{}",
        js
    );
    let dts = out.dts.as_str();
    assert!(
        dts.contains("mdi:"),
        ".d.ts must also project the parameter as `mdi`:\n{}",
        dts
    );
}

/// Lifecycle ergonomics + doc rendering on a generated interface wrapper.
/// Activation lives on the separately generated coclass; the interface must
/// expose `release()`, a protected constructor/IID descriptor for generated
/// coclass inheritance and `as()`, and — since win32metadata attaches a
/// `DocumentationAttribute` (a `learn.microsoft.com` URL) to most methods —
/// an `@see` JSDoc comment referencing that URL.
#[test]
fn taskbarlist3_generated_output_has_lifecycle_members_and_doc_links() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let com_iface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "ITaskbarList3",
    )
    .unwrap();
    let out = com::generate_com_interface_files(&com_iface, &win32_winmd())
        .expect("codegen must succeed for classic-COM interface");
    let js = out.js.as_str();
    let dts = out.dts.as_str();

    assert!(
        js.contains("release()") && js.contains("this._obj.release();"),
        ".js must expose release() delegating to the managed native value:\n{}",
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
        ".d.ts must expose only the generated-coclass/interface-view construction surface:\n{}",
        dts
    );
    assert!(
        js.contains("@see {@link https://learn.microsoft.com/"),
        ".js must render win32metadata's DocumentationAttribute URL as an @see JSDoc comment:\n{}",
        js
    );
    assert!(
        dts.contains("@see {@link https://learn.microsoft.com/"),
        ".d.ts must render win32metadata's DocumentationAttribute URL as an @see JSDoc comment:\n{}",
        dts
    );
}

// -------------------------------------------------------------------------
// CORNER tests
// -------------------------------------------------------------------------

/// 9. Regression: WinRT-style (IInspectable-based) interfaces still compute
///    base offset 6 (i.e. the existing WinRT path is unaffected).
#[test]
fn winrt_interfaces_still_use_offset_6() {
    // Parse a well-known WinRT interface (Windows.Foundation.IUriRuntimeClass or similar)
    // via the existing WinRT path — its first method should still have vtable_index = 6.
    let Some(windows_winmd) = discovered_windows_winmd() else {
        eprintln!(
            "Skipping winrt_interfaces_still_use_offset_6: no Windows SDK Windows.winmd discoverable"
        );
        return;
    };
    // Take Windows.Foundation.Uri's default interface — pick one that has methods.
    let class = meta::parse_class(&windows_winmd, "Windows.Foundation", "Uri")
        .expect("Windows.Foundation.Uri must be present");
    let default_iface = class
        .default_interface
        .as_ref()
        .expect("Uri must have a default interface");

    // Its first method's vtable_index must still be 6 (unchanged from existing
    // WinRT behavior); classic-COM support must not regress this.
    let first_slot = default_iface
        .methods
        .first()
        .map(|m| m.vtable_index)
        .expect("Uri default interface must have methods");
    assert_eq!(
        first_slot, 6,
        "WinRT interfaces retain the IInspectable base offset of 6"
    );
}

/// 10. Interface-not-found is a clean Option::None, not a panic.
#[test]
fn interface_not_found_is_clean_none() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let missing = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "IDoesNotExist_XYZ",
    );
    assert!(missing.is_none());
}

/// 11. QI-only interface (no coclass) → wrapper emitted WITHOUT `create()`,
///     only a static from-raw / QI entry point.
#[test]
fn qi_only_interface_has_no_create() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    // IPersist is IUnknown-rooted (has 1 own method: GetClassID) and has NO
    // "Persist" coclass anywhere in the metadata — verified via probe.
    let com_iface =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.System.Com", "IPersist")
            .expect("IPersist must exist in Win32 metadata");
    assert!(
        com_iface.coclass_clsid.is_none(),
        "IPersist has no associated coclass CLSID"
    );
    let get_class_id = com_iface
        .interface
        .methods
        .iter()
        .find(|method| method.name == "GetClassID")
        .unwrap();
    assert!(matches!(get_class_id.params[0].typ, TypeMeta::Guid));

    let out = com::generate_com_interface_files(&com_iface, &win32_winmd())
        .expect("codegen must succeed for classic-COM interface");
    let js = out.js.as_str();
    let dts = out.dts.as_str();
    assert!(js.contains(".addOut(DynCom.guidType())"));

    // No `create()` in either surface
    assert!(
        !dts.contains("static create()") && !dts.contains("static create(): "),
        "QI-only interface must not expose static create() in .d.ts:\n{}",
        dts
    );
    assert!(
        !js.contains("coCreateInstance"),
        "QI-only interface must not call coCreateInstance in .js:\n{}",
        js
    );

    // Must still have a fromNative / QI-only entry
    assert!(
        js.contains("_fromNative") || js.contains("fromRaw"),
        "QI-only interface must expose a from-raw entry:\n{}",
        js
    );

    // Slot 3 for GetClassID (only method, IUnknown-rooted)
    assert!(
        js.contains("method(3)"),
        "IPersist.GetClassID must invoke slot 3:\n{}",
        js
    );
}

/// 12. Determinism: regenerating ITaskbarList3 twice produces byte-identical output.
#[test]
fn generation_is_deterministic() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let a = {
        let com = com_metadata::parse_com_interface(
            &win32_winmd(),
            "Windows.Win32.UI.Shell",
            "ITaskbarList3",
        )
        .unwrap();
        com::generate_com_interface_files(&com, &win32_winmd())
            .expect("codegen must succeed for classic-COM interface")
    };
    let b = {
        let com = com_metadata::parse_com_interface(
            &win32_winmd(),
            "Windows.Win32.UI.Shell",
            "ITaskbarList3",
        )
        .unwrap();
        com::generate_com_interface_files(&com, &win32_winmd())
            .expect("codegen must succeed for classic-COM interface")
    };
    assert_eq!(a.js, b.js);
    assert_eq!(a.dts, b.dts);
    assert_eq!(a.extra_files, b.extra_files);
}

// -------------------------------------------------------------------------
// SNAPSHOT test
// -------------------------------------------------------------------------

/// Snapshot test: lock generated TaskbarList coclass and interface files.
///
/// To update snapshots after an intentional change:
///   cargo run -p dynwinrt-codegen -- generate \
///     --winmd C:\s\win32metadata\Windows.Win32.winmd \
///     --namespace Windows.Win32.UI.Shell \
///     --class-name TaskbarList \
///     --output tools/dynwinrt-codegen/tests/snapshots/itaskbarlist3
#[test]
fn snapshot_itaskbarlist3() {
    if !win32_available() {
        eprintln!("Skipping snapshot test: Win32 winmd not available");
        return;
    }
    let coclass =
        com_metadata::parse_com_coclass(&win32_winmd(), "Windows.Win32.UI.Shell", "TaskbarList")
            .unwrap()
            .expect("TaskbarList must exist");
    let out = com::generate_com_coclass_files(&coclass, &win32_winmd())
        .expect("codegen must succeed for TaskbarList");

    let snapshot_dir: PathBuf =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/itaskbarlist3");
    assert!(
        snapshot_dir.exists(),
        "Snapshot directory not found: {}",
        snapshot_dir.display()
    );

    let mut generated: Vec<(String, String)> = Vec::new();
    generated.push(("TaskbarList.js".into(), out.js.clone()));
    generated.push(("TaskbarList.d.ts".into(), out.dts.clone()));
    for (name, content) in &out.extra_files {
        generated.push((name.clone(), content.clone()));
    }

    let mut mismatches = Vec::new();
    for (name, actual) in &generated {
        let path = snapshot_dir.join(name);
        if !path.exists() {
            mismatches.push(format!("  missing snapshot: {}", name));
            continue;
        }
        let expected = fs::read_to_string(&path).unwrap();
        if actual.trim_end() != expected.trim_end() {
            mismatches.push(format!("  differs: {}", name));
        }
    }

    // Any extra snapshot file not produced by the generator is also a mismatch.
    if let Ok(entries) = fs::read_dir(&snapshot_dir) {
        let names: std::collections::HashSet<String> =
            generated.iter().map(|(n, _)| n.clone()).collect();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !names.contains(&name) {
                mismatches.push(format!("  extra snapshot not generated: {}", name));
            }
        }
    }

    if !mismatches.is_empty() {
        panic!(
            "ITaskbarList3 snapshot mismatch!\n{}\n\n\
             To update, re-run the generator or copy the actual output into the snapshot dir.",
            mismatches.join("\n")
        );
    }
}

#[test]
fn snapshot_ishelllinkw_native_pod() {
    if !win32_available() {
        eprintln!("Skipping snapshot test: Win32 winmd not available");
        return;
    }
    let interface =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.UI.Shell", "IShellLinkW")
            .expect("IShellLinkW must exist");
    let out = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect("codegen must succeed for IShellLinkW");
    let snapshot_dir: PathBuf =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/ishelllinkw");

    for (name, actual) in [("IShellLinkW.js", out.js), ("IShellLinkW.d.ts", out.dts)] {
        let path = snapshot_dir.join(name);
        let expected = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("Failed to read {}: {error}", path.display()));
        assert_eq!(
            actual.trim_end(),
            expected.trim_end(),
            "{name} native POD snapshot changed"
        );
    }
}

// -------------------------------------------------------------------------
// --import-name honored by classic-COM path
// -------------------------------------------------------------------------

/// Regression test for a bug where the classic-COM generator hardcoded the
/// runtime import as `'@microsoft/dynwinrt'`, ignoring the `--import-name`
/// CLI flag (which the WinRT path already honored via
/// `codegen::project::set_import_name`). Fixing this makes it possible to
/// regenerate the Node E2E wrappers from `Windows.Win32.winmd` without
/// hand-patching the import line.
///
/// The test uses the same thread-local as `set_import_name`, so it
/// save/restores the default around the assertion to avoid contaminating
/// other tests that assume the `@microsoft/dynwinrt` default (notably the
/// snapshot tests). `#[serial]` is intentionally NOT used — because
/// `RUNTIME_IMPORT_NAME` is a `thread_local!`, cargo's parallel test runner
/// gives each thread its own copy; restoring on the same thread is enough.
#[test]
fn import_name_flag_is_honored_by_com_path() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let previous = get_import_name();
    set_import_name("../dist/com-unsafe.js");

    let result = std::panic::catch_unwind(|| {
        let com_iface = com_metadata::parse_com_interface(
            &win32_winmd(),
            "Windows.Win32.UI.Shell",
            "ITaskbarList3",
        )
        .expect("ITaskbarList3 must exist");
        com::generate_com_interface_files(&com_iface, &win32_winmd())
            .expect("codegen must succeed for classic-COM interface")
    });

    // Always restore before propagating any assertion failure.
    set_import_name(&previous);

    let out = result.unwrap_or_else(|e| std::panic::resume_unwind(e));

    // Custom import must appear on the runtime import line...
    assert!(
        out.js.contains("require('../dist/com-unsafe.js')"),
        "classic-COM .js must honor --import-name (expected `require('../dist/com-unsafe.js')`):\n{}",
        out.js
    );
    // ...and the hardcoded default must NOT be present in the generated body.
    assert!(
        !out.js.contains("'@microsoft/dynwinrt'"),
        "classic-COM .js must NOT hardcode '@microsoft/dynwinrt' when --import-name is set:\n{}",
        out.js
    );

    // Sanity: after restoring the default, subsequent generation reverts.
    let default_out = {
        let com_iface = com_metadata::parse_com_interface(
            &win32_winmd(),
            "Windows.Win32.UI.Shell",
            "ITaskbarList3",
        )
        .expect("ITaskbarList3 must exist");
        com::generate_com_interface_files(&com_iface, &win32_winmd())
            .expect("codegen must succeed for classic-COM interface")
    };
    assert!(
        default_out
            .js
            .contains("require('@microsoft/dynwinrt/com/unsafe')"),
        "after restoring, default import must use '@microsoft/dynwinrt/com/unsafe':\n{}",
        default_out.js
    );
}

/// Same test for the interop bridge generation path.
#[test]
fn import_name_flag_is_honored_by_interop_wrapper() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    if discovered_windows_winmd().is_none() {
        eprintln!(
            "Skipping: no Windows SDK Windows.winmd discoverable (needed for interop resolution)"
        );
        return;
    }

    let previous = get_import_name();
    set_import_name("../dist/com-unsafe.js");

    let result = std::panic::catch_unwind(|| {
        let com_iface = com_metadata::parse_com_interface(
            &win32_winmd(),
            "Windows.Win32.UI.Shell",
            "IDataTransferManagerInterop",
        )
        .expect("IDataTransferManagerInterop must exist");
        com::generate_com_interface_files(&com_iface, &win32_winmd())
            .expect("codegen must succeed for classic-COM interop interface")
    });

    set_import_name(&previous);
    let out = result.unwrap_or_else(|e| std::panic::resume_unwind(e));

    // The interop .js itself must honor the flag.
    assert!(
        out.js.contains("require('../dist/com-unsafe.js')"),
        "interop .js must honor --import-name:\n{}",
        out.js
    );
    assert!(
        !out.js.contains("'@microsoft/dynwinrt'"),
        "interop .js must NOT hardcode '@microsoft/dynwinrt':\n{}",
        out.js
    );

    assert!(
        out.dts.contains("from '../dist/com.js'"),
        "interop .d.ts must honor --import-name:\n{}",
        out.dts
    );
    assert!(
        !out.extra_files
            .iter()
            .any(|(name, _)| name.starts_with("DataTransferManager.")),
        "COM codegen must not emit a projected WinRT companion"
    );
}

#[test]
fn shellitem_getdisplayname_is_not_classified_as_caller_owned_string_buffer() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let com_iface =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.UI.Shell", "IShellItem")
            .expect("IShellItem must exist");
    let out = com::generate_com_interface_files(&com_iface, &win32_winmd())
        .expect("codegen must succeed for IShellItem");

    assert!(
        out.js.contains(".addOut(DynCom.coTaskMemPointerType())")
            && out.js.contains(
                "'GetDisplayName', new DynComMethodSig().addIn(DynCom.i32Type()).addOut(DynCom.coTaskMemPointerType())"
            ),
        "PWSTR* callee-allocated output must be registered as an owned CoTaskMem output, not the unclassified pointer type:\n{}",
        out.js
    );
    assert!(
        !out.js.contains("getDisplayName(sigdnName = 260)")
            && !out.js.contains("_decodeWideString"),
        "IShellItem.GetDisplayName must not allocate/decode a caller-owned buffer:\n{}",
        out.js
    );
}

#[test]
fn u16_input_param_uses_existing_u16_value_ctor_not_u16value() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    // IShellLinkW.SetHotkey takes a [in] u16 (WORD). The classic-COM value
    // ctor is DynCom.u16(...) — there is no `u16Value`/`i16Value`. Regression
    // guard: the arg-wrapper must emit the ctor that actually exists, or the
    // generated call throws at runtime.
    let com_iface =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.UI.Shell", "IShellLinkW")
            .expect("IShellLinkW must exist");
    let out = com::generate_com_interface_files(&com_iface, &win32_winmd())
        .expect("codegen must succeed for IShellLinkW");

    assert!(
        out.js.contains("DynCom.u16(wHotkey)"),
        "u16 input param must wrap via the existing DynCom.u16(...):\n{}",
        out.js
    );
    assert!(
        !out.js.contains("u16Value(") && !out.js.contains("i16Value("),
        "codegen must not emit non-existent u16Value/i16Value ctor:\n{}",
        out.js
    );
}

#[test]
fn sid_buffers_keep_data_pointer_address_semantics() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let mut interface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.Storage.FileSystem",
        "IDiskQuotaControl",
    )
    .expect("IDiskQuotaControl must exist");
    interface
        .interface
        .methods
        .retain(|method| method.name == "AddUserSid");
    interface
        .raw_methods
        .as_mut()
        .unwrap()
        .retain(|method| method.projected_name == "AddUserSid");
    let output = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect("isolated IDiskQuotaControl.AddUserSid generation should succeed");

    assert!(
        output.dts.contains("addUserSid(pUserSid: PSID,"),
        "PSID input must accept backing storage rather than handle bytes:\n{}",
        output.dts
    );
    assert!(
        output.js.contains("DynCom.safeDataPointer(pUserSid)")
            && !output.js.contains("handleValue(pUserSid)"),
        "PSID Buffer must pass its address, not decoded contents:\n{}",
        output.js
    );
}

#[test]
fn typed_counted_buffers_generate_from_real_win32_contracts() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let stream = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.Com",
        "ISequentialStream",
    )
    .expect("ISequentialStream must exist");
    let output = com::generate_com_interface_files(&stream, &win32_winmd())
        .expect("documented ISequentialStream buffer overrides must project");
    assert!(
        output
            .js
            .contains(".addCallerOutputBuffer(DynCom.u8Type(), 1, 2, true, false)")
            && output
                .js
                .contains(".addInputBuffer(DynCom.u8Type(), 1, 2, true)"),
        "stream signatures must preserve byte capacity and actual-length relationships:\n{}",
        output.js
    );
    assert!(
        output.js.contains("read(pv)")
            && output.js.contains("write(pv)")
            && !output.js.contains("read(pv, cb")
            && !output.js.contains("write(pv, cb"),
        "derived byte counts must be hidden from the JS surface:\n{}",
        output.js
    );
    assert!(
        output
            .dts
            .contains("read(pv: Buffer | ArrayBufferView): [number, Buffer]")
            && output
                .dts
                .contains("write(pv: Buffer | ArrayBufferView): [number, number]"),
        "stream buffers must use natural typed-buffer declarations:\n{}",
        output.dts
    );

    let opc = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.Storage.Packaging.Opc",
        "IOpcSignatureCustomObject",
    )
    .expect("IOpcSignatureCustomObject must exist");
    let output = com::generate_com_interface_files(&opc, &win32_winmd())
        .expect("documented CoTaskMem byte output must project");
    assert!(
        output
            .js
            .contains(".addCoTaskMemOutputBuffer(DynCom.u8Type(), 1, true)")
            && output.js.contains("return DynCom.takeBuffer(_out)")
            && output.dts.contains("getXml(): Buffer"),
        "callee-allocated bytes must carry their exact allocator into generated code:\n{}",
        output.js
    );
}

#[test]
fn recorder_two_call_buffer_uses_bounded_exact_projection() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let mut recorder = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.Storage.Imapi",
        "IDiscRecorder",
    )
    .expect("IDiscRecorder must exist");
    let raw = recorder
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .position(|method| method.metadata_name == "GetRecorderGUID")
        .expect("GetRecorderGUID raw metadata");
    let projected = recorder
        .interface
        .methods
        .iter()
        .position(|method| method.name == "GetRecorderGUID")
        .expect("GetRecorderGUID projected metadata");
    recorder.raw_methods = Some(vec![recorder.raw_methods.as_ref().unwrap()[raw].clone()]);
    recorder.interface.methods = vec![recorder.interface.methods[projected].clone()];
    recorder.own_methods_start = 0;

    let output = com::generate_com_interface_files(&recorder, &win32_winmd())
        .expect("the isolated exact documented two-call method must project");
    assert!(
        output
            .js
            .contains(".addCallerOutputBuffer(DynCom.u8Type(), 1, 2, true, true)")
            && output.js.contains("getRecorderGUID()")
            && output
                .js
                .contains("for (let _attempt = 0; _attempt <= 2; _attempt++)")
            && output.js.contains("DynCom.bufferCount")
            && output.js.contains("DynCom.nullBuffer()")
            && output.js.contains("DynCom.bufferAllocationLength(_actual)")
            && output
                .js
                .contains("COM buffer size changed during bounded two-call retry"),
        "two-call sizing must be exact and bounded:\n{}",
        output.js
    );
    assert!(output.dts.contains("getRecorderGUID(): Buffer"));
}

#[test]
fn imf_attributes_get_blob_uses_exact_fixed_capacity_bytes() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let mut attributes = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.Media.MediaFoundation",
        "IMFAttributes",
    )
    .expect("IMFAttributes must exist");
    assert_eq!(
        attributes.interface.iid,
        "2cd2d921-c447-44a7-a13c-4adabfc247e3"
    );
    assert_eq!(attributes.base_chain, ["IUnknown"]);
    assert_eq!(attributes.base_offset, 3);
    assert_eq!(attributes.own_methods_start, 3);

    let raw_index = attributes
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .position(|method| method.metadata_name == "GetBlob")
        .expect("GetBlob raw metadata");
    let raw = &attributes.raw_methods.as_ref().unwrap()[raw_index];
    assert_eq!(raw.vtable_index, 15);
    assert_eq!(raw.params.len(), 4);
    let contract = raw
        .exact_contract
        .as_ref()
        .expect("GetBlob exact contract evidence");
    assert_eq!(
        contract.kind,
        com_metadata::RawExactMethodContractKind::FixedCapacityBytes
    );
    assert_eq!(
        (
            contract.buffer_param_index,
            contract.capacity_param_index,
            contract.actual_length_param_index,
        ),
        (1, 2, Some(3))
    );
    assert!(contract.citation.contains("imfattributes-getblob"));
    assert_eq!(raw.params[0].typ.pointer_depth, 1);
    assert!(raw.params[0].const_attribute);
    assert_eq!(raw.params[0].direction, RawParamDirection::In);
    assert_eq!(raw.params[1].typ.pointer_depth, 1);
    assert_eq!(raw.params[1].direction, RawParamDirection::Out);
    assert!(!raw.params[1].optional);
    let relation = raw.params[1]
        .native_array
        .as_ref()
        .expect("GetBlob byte relation");
    assert_eq!(relation.count_param_index, Some(2));
    assert_eq!(relation.actual_length_param_index, Some(3));
    assert_eq!(relation.unit, com_metadata::RawCountUnit::Bytes);
    assert!(relation.projected_capacity);
    assert!(!relation.two_call);
    assert_eq!(raw.params[2].direction, RawParamDirection::In);
    assert!(matches!(raw.params[2].typ.native_type, RawNativeType::U32));
    assert_eq!(raw.params[3].direction, RawParamDirection::Out);
    assert!(raw.params[3].optional);
    assert_eq!(raw.params[3].typ.pointer_depth, 1);

    let projected_index = attributes
        .interface
        .methods
        .iter()
        .position(|method| method.name == "GetBlob")
        .expect("GetBlob compatibility metadata");
    attributes.raw_methods = Some(vec![raw.clone()]);
    attributes.interface.methods = vec![attributes.interface.methods[projected_index].clone()];
    attributes.own_methods_start = 0;

    let output = com::generate_com_interface_files(&attributes, &win32_winmd())
        .expect("isolated exact GetBlob must project");
    assert!(
        output
            .js
            .contains(".addCallerOutputBuffer(DynCom.u8Type(), 2, 3, true, false)")
            && output.js.contains("getBlob(guidKey, capacity)")
            && output
                .js
                .contains("Number.isSafeInteger(capacity) || capacity < 0")
            && output
                .js
                .contains("DynCom.bufferAllocationLength(BigInt(capacity))")
            && output
                .js
                .contains("DynCom.callerOutputArray(DynCom.u8Type(), BigInt(_capacity))")
            && output.js.contains("return DynCom.takeBuffer(_out);"),
        "fixed-capacity GetBlob must allocate exclusive runtime storage and hide native counts:\n{}",
        output.js
    );
    assert!(
        output
            .dts
            .contains("getBlob(guidKey: string, capacity: number): Buffer;"),
        "{}",
        output.dts
    );

    let mut invalid_actual = attributes.clone();
    invalid_actual.raw_methods.as_mut().unwrap()[0].params[3].direction = RawParamDirection::InOut;
    let error = com::generate_com_interface_files(&invalid_actual, &win32_winmd())
        .expect_err("a distinct actual length must not accept InOut metadata");
    assert!(
        (error.contains("actual-length") && error.contains("must be Out"))
            || error.contains("no longer matches exact contract evidence"),
        "{error}"
    );

    use windows::Win32::Media::MediaFoundation::IMFAttributes;
    use windows::core::Interface;
    assert_eq!(
        format!("{:?}", IMFAttributes::IID).to_ascii_lowercase(),
        attributes.interface.iid
    );
}

#[test]
fn get_private_data_families_fail_closed_for_interface_ownership() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let families = [
        (
            "Windows.Win32.Graphics.Dxgi",
            "IDXGIObject",
            "aec22fb8-76f3-4639-9be0-28eb43a67a2e",
            5,
        ),
        (
            "Windows.Win32.Graphics.Direct3D10",
            "ID3D10DeviceChild",
            "9b7e4c00-342c-4106-a19f-4f2704f689f0",
            4,
        ),
        (
            "Windows.Win32.Graphics.Direct3D10",
            "ID3D10Device",
            "9b7e4c0f-342c-4106-a19f-4f2704f689f0",
            66,
        ),
        (
            "Windows.Win32.Graphics.Direct3D11",
            "ID3D11DeviceChild",
            "1841e5c8-16b0-489b-bcc8-44cfb0d5deae",
            4,
        ),
        (
            "Windows.Win32.Graphics.Direct3D11",
            "ID3D11Device",
            "db6f6ddb-ac77-4e88-8253-819df9bbf140",
            34,
        ),
        (
            "Windows.Win32.Graphics.Direct3D12",
            "ID3D12Object",
            "c4fec28f-7966-4e95-9f94-f431cb56c3b8",
            3,
        ),
        (
            "Windows.Win32.AI.MachineLearning.DirectML",
            "IDMLObject",
            "c8263aac-9e0c-4a2d-9b8e-007521a3317c",
            3,
        ),
    ];
    for (namespace, name, iid, slot) in families {
        let mut interface =
            com_metadata::parse_com_interface(&win32_winmd(), namespace, name).unwrap();
        assert_eq!(interface.interface.iid, iid);
        assert_eq!(interface.base_chain, ["IUnknown"]);
        let raw_index = interface
            .raw_methods
            .as_ref()
            .unwrap()
            .iter()
            .position(|method| method.metadata_name == "GetPrivateData")
            .unwrap();
        let projected_index = interface
            .interface
            .methods
            .iter()
            .position(|method| method.name == "GetPrivateData")
            .unwrap();
        let raw = interface.raw_methods.as_ref().unwrap()[raw_index].clone();
        assert_eq!(raw.vtable_index, slot);
        assert_eq!(
            raw.exact_contract.as_ref().unwrap().kind,
            com_metadata::RawExactMethodContractKind::UnsafePrivateData
        );
        interface.raw_methods = Some(vec![raw]);
        interface.interface.methods = vec![interface.interface.methods[projected_index].clone()];
        interface.own_methods_start = 0;
        let error = com::generate_com_interface_files(&interface, &win32_winmd())
            .expect_err("generic GetPrivateData Buffer projection must remain unsupported");
        assert!(
            error.contains("AddRef")
                && error.contains("interface pointer")
                && (error.contains("ownership") || error.contains("destructive")),
            "{namespace}.{name}: {error}"
        );
    }
}

#[test]
fn exact_fixed_capacity_byte_contract_mutations_fail_closed() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    fn isolated() -> com_metadata::ComInterfaceMeta {
        let mut interface = com_metadata::parse_com_interface(
            &win32_winmd(),
            "Windows.Win32.Media.MediaFoundation",
            "IMFAttributes",
        )
        .unwrap();
        let raw_index = interface
            .raw_methods
            .as_ref()
            .unwrap()
            .iter()
            .position(|method| method.metadata_name == "GetBlob")
            .unwrap();
        let projected_index = interface
            .interface
            .methods
            .iter()
            .position(|method| method.name == "GetBlob")
            .unwrap();
        interface.raw_methods = Some(vec![
            interface.raw_methods.as_ref().unwrap()[raw_index].clone(),
        ]);
        interface.interface.methods = vec![interface.interface.methods[projected_index].clone()];
        interface.own_methods_start = 0;
        interface
    }

    let mutations: Vec<Box<dyn Fn(&mut com_metadata::ComInterfaceMeta)>> = vec![
        Box::new(|interface| {
            interface.interface.iid = "11111111-2222-3333-4444-555555555555".into()
        }),
        Box::new(|interface| {
            interface.raw_methods.as_mut().unwrap()[0].metadata_name = "GetBlob2".into()
        }),
        Box::new(|interface| interface.raw_methods.as_mut().unwrap()[0].vtable_index += 1),
        Box::new(|interface| {
            interface.raw_methods.as_mut().unwrap()[0].params[0]
                .typ
                .pointer_depth = 0
        }),
        Box::new(|interface| {
            interface.raw_methods.as_mut().unwrap()[0].params[0].const_attribute = false
        }),
        Box::new(|interface| {
            interface.raw_methods.as_mut().unwrap()[0].params[1].direction = RawParamDirection::In
        }),
        Box::new(|interface| {
            interface.raw_methods.as_mut().unwrap()[0].params[1]
                .typ
                .pointer_depth = 2
        }),
        Box::new(|interface| interface.raw_methods.as_mut().unwrap()[0].params[1].optional = true),
        Box::new(|interface| {
            interface.raw_methods.as_mut().unwrap()[0].params[2].direction = RawParamDirection::Out
        }),
        Box::new(|interface| {
            interface.raw_methods.as_mut().unwrap()[0].params[2]
                .typ
                .native_type = RawNativeType::U16
        }),
        Box::new(|interface| {
            interface.raw_methods.as_mut().unwrap()[0].params[3].direction =
                RawParamDirection::InOut
        }),
        Box::new(|interface| interface.raw_methods.as_mut().unwrap()[0].params[3].optional = false),
        Box::new(|interface| {
            interface.raw_methods.as_mut().unwrap()[0].params[3]
                .typ
                .pointer_depth = 2
        }),
        Box::new(|interface| {
            interface.raw_methods.as_mut().unwrap()[0]
                .return_type
                .native_type = RawNativeType::Void
        }),
    ];
    for mutate in mutations {
        let mut interface = isolated();
        mutate(&mut interface);
        let error = com::generate_com_interface_files(&interface, &win32_winmd())
            .expect_err("mutated exact byte contract must fail");
        assert!(
            error.contains("GetBlob") || error.contains("exact contract"),
            "{error}"
        );
    }
}

#[test]
fn unsupported_counted_buffer_contracts_fail_closed() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let enum_string = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.Com",
        "IEnumString",
    )
    .expect("IEnumString must exist");
    com::generate_com_interface_files(&enum_string, &win32_winmd())
        .expect("exact IEnumString ownership must project");
    let mut missing_array = enum_string.clone();
    missing_array
        .raw_methods
        .as_mut()
        .unwrap()
        .iter_mut()
        .find(|method| method.metadata_name == "Next")
        .unwrap()
        .params[1]
        .native_array = None;
    let error = com::generate_com_interface_files(&missing_array, &win32_winmd())
        .expect_err("IEnumString without count evidence must fail");
    assert!(
        error.contains("Next")
            && (error.contains("pointer")
                || error.contains("array")
                || error.contains("EnumeratorNext")),
        "missing initialized-range evidence must fail closed: {error}"
    );
    let mut unknown_allocator = enum_string;
    unknown_allocator
        .raw_methods
        .as_mut()
        .unwrap()
        .iter_mut()
        .find(|method| method.metadata_name == "Next")
        .unwrap()
        .params[1]
        .string_pointer_array
        .as_mut()
        .unwrap()
        .ownership = com_metadata::RawElementOwnership::Unknown;
    let error = com::generate_com_interface_files(&unknown_allocator, &win32_winmd())
        .expect_err("IEnumString without CoTaskMem ownership must fail");
    assert!(
        error.contains("Next")
            && (error.contains("ownership")
                || error.contains("allocator")
                || error.contains("cleanup")
                || error.contains("caller-sized")),
        "unknown element allocator must fail closed: {error}"
    );

    let mut stream = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.Com",
        "ISequentialStream",
    )
    .unwrap();
    let read = stream
        .raw_methods
        .as_mut()
        .unwrap()
        .iter_mut()
        .find(|method| method.metadata_name == "Read")
        .unwrap();
    read.params[0].native_array = None;
    let error = com::generate_com_interface_files(&stream, &win32_winmd())
        .expect_err("a writable void pointer without array evidence must fail");
    assert!(
        error.contains("Read") && error.contains("pv"),
        "absent evidence must fail at the buffer parameter: {error}"
    );

    let mut stream = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.Com",
        "ISequentialStream",
    )
    .unwrap();
    stream
        .raw_methods
        .as_mut()
        .unwrap()
        .iter_mut()
        .find(|method| method.metadata_name == "Read")
        .unwrap()
        .params[0]
        .native_array
        .as_mut()
        .unwrap()
        .count_param_index = Some(99);
    let error = com::generate_com_interface_files(&stream, &win32_winmd())
        .expect_err("an out-of-range count relation must fail");
    assert!(
        error.contains("Read")
            && error.contains("pv")
            && (error.contains("outside the method")
                || error.contains("caller-sized native buffers")),
        "mismatched count relationships must fail closed: {error}"
    );

    let mut opc = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.Storage.Packaging.Opc",
        "IOpcSignatureCustomObject",
    )
    .unwrap();
    opc.raw_methods.as_mut().unwrap()[0].params[0].free_with = None;
    let error = com::generate_com_interface_files(&opc, &win32_winmd())
        .expect_err("callee allocation without its allocator must fail");
    assert!(
        error.contains("GetXml")
            && error.contains("xmlMarkup")
            && (error.contains("ownership") || error.contains("caller-sized native buffers")),
        "unknown allocator must fail closed: {error}"
    );
}

#[test]
fn standard_enumerator_next_projects_partial_arrays_and_exact_interface_ownership() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let guid_enum =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.System.Com", "IEnumGUID")
            .expect("IEnumGUID must exist");
    let next = guid_enum
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .find(|method| method.metadata_name == "Next")
        .unwrap();
    let plan = next
        .enumerator_next
        .as_ref()
        .expect("exact IEnumGUID::Next override");
    assert_eq!(
        (
            plan.capacity_param_index,
            plan.values_param_index,
            plan.fetched_param_index,
            plan.fetched_optional_for_single,
        ),
        (0, 1, 2, true)
    );
    let output = com::generate_com_interface_files(&guid_enum, &win32_winmd())
        .expect("IEnumGUID must project completely");
    assert!(
        output.js.contains(
            ".addEnumeratorNextBuffer(DynCom.guidType(), 0, 2).addOptionalOut(DynCom.u32Type()).preserveEnumeratorNextHresult()"
        ) && output
            .js
            .contains("DynCom.enumeratorOutputArray(DynCom.guidType(), BigInt(count))")
            && output.js.contains("DynCom.takeGuidArray(_out[1])")
            && output.dts.contains("next(count: number): string[]")
            && output.js.contains(".addMethodAt(6, 'Clone'"),
        "IEnumGUID must expose its complete interface with a natural partial-array Next:\n{}",
        output.js
    );

    let connection_enum = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.Com",
        "IEnumConnectionPoints",
    )
    .expect("IEnumConnectionPoints must exist");
    let next = connection_enum
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .find(|method| method.metadata_name == "Next")
        .unwrap();
    assert!(
        !next
            .enumerator_next
            .as_ref()
            .expect("exact IEnumConnectionPoints::Next override")
            .fetched_optional_for_single
    );
    let output = com::generate_com_interface_files(&connection_enum, &win32_winmd())
        .expect("IEnumConnectionPoints must project completely");
    assert!(
        output.js.contains(
            ".addEnumeratorNextBuffer(DynCom.interfaceType(WinGuid.parse('b196b286-bab4-101a-b69c-00aa00341d07')), 0, 2).addOut(DynCom.u32Type()).preserveEnumeratorNextHresult()"
        ) && output.js.contains("DynCom.takeComArray(_out[1])")
            && output
                .js
                .contains("return IConnectionPoint._fromNative(value);")
            && output.js.contains("finally { value.release(); }")
            && output
                .dts
                .contains("next(count: number): IConnectionPoint[]")
            && output
                .extra_files
                .iter()
                .any(|(name, _)| name == "IConnectionPoint.js")
            && output
                .extra_files
                .iter()
                .any(|(name, _)| name == "IConnectionPoint.d.ts"),
        "owned interface elements must use managed wrappers and emit their complete dependency:\n{}",
        output.js
    );

    let variant_enum = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.Ole",
        "IEnumVARIANT",
    )
    .expect("IEnumVARIANT must exist");
    let output = com::generate_com_interface_files(&variant_enum, &win32_winmd())
        .expect("IEnumVARIANT must project completely");
    assert!(
        output
            .js
            .contains(".addEnumeratorNextBuffer(DynCom.variantType(), 0, 2)")
            && output.js.contains("DynCom.takeVariantArray(_out[1])")
            && output.dts.contains("next(count: number): DynComVariant[]"),
        "{}",
        output.js
    );

    let string_enum = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.Com",
        "IEnumString",
    )
    .expect("IEnumString must exist");
    let output = com::generate_com_interface_files(&string_enum, &win32_winmd())
        .expect("IEnumString must project completely");
    assert!(
        output
            .js
            .contains(".addEnumeratorNextBuffer(DynCom.coTaskMemWideStringType(), 0, 2)")
            && output
                .js
                .contains("DynCom.takeCoTaskMemWideStringArray(_out[1])")
            && output.dts.contains("next(count: number): string[]"),
        "{}",
        output.js
    );

    let inherited_enum = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.TextServices",
        "IEnumITfCompositionView",
    )
    .expect("IEnumITfCompositionView must exist");
    let next = inherited_enum
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .find(|method| method.metadata_name == "Next")
        .unwrap();
    assert_eq!(next.vtable_index, 4);
    assert!(next.enumerator_next.is_some());
    let output = com::generate_com_interface_files(&inherited_enum, &win32_winmd())
        .expect("exact non-slot-3 interface enumerator must project");
    assert!(
        output.js.contains(".preserveEnumeratorNextHresultAt(4)")
            && output
                .dts
                .contains("next(count: number): ITfCompositionView[]"),
        "{}",
        output.js
    );

    use windows::Win32::System::Com::{
        IConnectionPoint, IEnumConnectionPoints, IEnumGUID, IEnumString,
    };
    use windows::Win32::System::Ole::IEnumVARIANT;
    use windows::core::Interface;
    assert_eq!(
        format!("{:?}", IEnumGUID::IID).to_ascii_lowercase(),
        "0002e000-0000-0000-c000-000000000046"
    );
    assert_eq!(
        format!("{:?}", IEnumConnectionPoints::IID).to_ascii_lowercase(),
        "b196b285-bab4-101a-b69c-00aa00341d07"
    );
    assert_eq!(
        format!("{:?}", IConnectionPoint::IID).to_ascii_lowercase(),
        "b196b286-bab4-101a-b69c-00aa00341d07"
    );
    assert_eq!(
        format!("{:?}", IEnumVARIANT::IID).to_ascii_lowercase(),
        "00020404-0000-0000-c000-000000000046"
    );
    assert_eq!(
        format!("{:?}", IEnumString::IID).to_ascii_lowercase(),
        "00000101-0000-0000-c000-000000000046"
    );
    assert_eq!(std::mem::size_of::<windows::core::GUID>(), 16);
    assert_eq!(std::mem::align_of::<windows::core::GUID>(), 4);
    assert_eq!(
        std::mem::size_of::<IConnectionPoint>(),
        std::mem::size_of::<*mut std::ffi::c_void>()
    );
    assert_eq!(
        std::mem::align_of::<IConnectionPoint>(),
        std::mem::align_of::<*mut std::ffi::c_void>()
    );
    assert_eq!(
        std::mem::size_of::<windows::Win32::System::Variant::VARIANT>(),
        if cfg!(target_pointer_width = "64") {
            24
        } else {
            16
        }
    );
    assert_eq!(
        std::mem::size_of::<windows::core::PWSTR>(),
        std::mem::size_of::<*mut std::ffi::c_void>()
    );
}

#[test]
fn enumerator_next_exact_evidence_fails_closed_when_mutated() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let mut mismatched =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.System.Com", "IEnumGUID")
            .unwrap();
    let next = mismatched
        .raw_methods
        .as_mut()
        .unwrap()
        .iter_mut()
        .find(|method| method.metadata_name == "Next")
        .unwrap();
    next.params[1]
        .native_array
        .as_mut()
        .unwrap()
        .count_param_index = Some(2);
    let error = com::generate_com_interface_files(&mismatched, &win32_winmd()).unwrap_err();
    assert!(
        error.contains("Next")
            && (error.contains("count")
                || error.contains("caller-sized native buffers")
                || error.contains("Unknown native type")
                || error.contains("unknown native type")),
        "{error}"
    );

    let mut nonstandard =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.System.Com", "IEnumGUID")
            .unwrap();
    nonstandard
        .raw_methods
        .as_mut()
        .unwrap()
        .iter_mut()
        .find(|method| method.metadata_name == "Next")
        .unwrap()
        .vtable_index = 4;
    let error = com::generate_com_interface_files(&nonstandard, &win32_winmd()).unwrap_err();
    assert!(
        error.contains("slot-3 ABI shape")
            || error.contains("duplicate vtable slot")
            || error.contains("vtable")
            || error.contains("caller-sized native buffers"),
        "{error}"
    );

    let mut variant_enum = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.Ole",
        "IEnumVARIANT",
    )
    .expect("IEnumVARIANT must exist");
    variant_enum
        .raw_methods
        .as_mut()
        .unwrap()
        .iter_mut()
        .find(|method| method.metadata_name == "Next")
        .unwrap()
        .params[1]
        .typ
        .pointer_depth = 2;
    let error = com::generate_com_interface_files(&variant_enum, &win32_winmd())
        .expect_err("mutated VARIANT enumerator must fail closed");
    assert!(
        error.contains("Next")
            && (error.contains("initialized-range cleanup")
                || error.contains("Automation")
                || error.contains("caller-sized native buffers")
                || error.contains("unsupported")
                || error.contains("exact enumerator evidence")),
        "{error}"
    );
}

#[test]
fn real_owning_counted_arrays_project_natural_inputs_and_outputs() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let type_info =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.System.Com", "ITypeInfo")
            .expect("ITypeInfo must exist");
    let get_names = type_info
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .find(|method| method.metadata_name == "GetNames")
        .unwrap();
    let names = get_names.params[1].native_array.as_ref().unwrap();
    assert_eq!(names.count_param_index, Some(2));
    assert_eq!(names.actual_length_param_index, Some(3));
    assert!(names.evidence.iter().any(|evidence| matches!(
        evidence,
        com_metadata::RawEvidence::Override { citation, .. }
            if citation.contains("itypeinfo-getnames")
    )));
    let get_names_output = com::generate_com_interface_files(
        &isolate_com_method(&type_info, "GetNames"),
        &win32_winmd(),
    )
    .unwrap();
    assert!(
        get_names_output
            .js
            .contains(".addCallerOutputBuffer(DynCom.bstrType(), 2, 3, false, false)")
    );

    let mut invalid = isolate_com_method(&type_info, "GetNames");
    invalid.raw_methods.as_mut().unwrap()[0].params[3].direction = RawParamDirection::In;
    let error = com::generate_com_interface_files(&invalid, &win32_winmd())
        .expect_err("GetNames must require an authoritative output actual count");
    assert!(
        error.contains("actual length")
            || error.contains("output")
            || error.contains("caller-sized native buffers"),
        "{error}"
    );

    let cases = [
        (
            "Windows.Win32.System.RemoteDesktop",
            "ITSGAuthorizeResourceSink",
            "OnChannelAuthorized",
            "DynCom.bstrArray(",
            "string[]",
        ),
        (
            "Windows.Win32.System.Wmi",
            "IWbemObjectSink",
            "Indicate",
            "DynCom.interfaceArray(",
            "IWbemClassObject[]",
        ),
        (
            "Windows.Win32.System.Com.StructuredStorage",
            "IPropertyBag2",
            "Write",
            "DynCom.variantArray(",
            "DynComVariant[]",
        ),
        (
            "Windows.Win32.System.Com",
            "ITypeInfo",
            "GetNames",
            "DynCom.takeBstrArray(",
            "string[]",
        ),
    ];
    for (namespace, interface, method, js, dts) in cases {
        let output = project_isolated_com_method(namespace, interface, method);
        assert!(
            output.js.contains(js),
            "{}.{}:\n{}",
            interface,
            method,
            output.js
        );
        assert!(
            output.dts.contains(dts),
            "{}.{}:\n{}",
            interface,
            method,
            output.dts
        );
    }
    let sink =
        project_isolated_com_method("Windows.Win32.System.Wmi", "IWbemObjectSink", "Indicate");
    let wrapper = sink
        .extra_files
        .iter()
        .find(|(name, _)| name == "IWbemClassObject.js")
        .expect("nominal element wrapper dependency");
    assert!(
        wrapper.1.contains("class IWbemClassObject"),
        "{}",
        wrapper.1
    );
    assert!(
        !wrapper.1.contains(".addMethodAt("),
        "an incomplete dependency must remain an opaque nominal wrapper:\n{}",
        wrapper.1
    );
}

#[test]
fn real_borrowed_hwnd_outputs_are_exact_numeric_getters() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let cases = [
        (
            "Windows.Win32.System.Ole",
            "IOleWindow",
            "GetWindow",
            "getWindow(): HWND;",
        ),
        (
            "Windows.Win32.System.Mmc",
            "IConsole",
            "GetMainWindow",
            "getMainWindow(): HWND;",
        ),
        (
            "Windows.Win32.Devices.ImageAcquisition",
            "IWiaAppErrorHandler",
            "GetWindow",
            "getWindow(): HWND;",
        ),
        (
            "Windows.Win32.UI.Shell",
            "ICredentialProviderCredentialEvents",
            "OnCreatingWindow",
            "onCreatingWindow(): HWND;",
        ),
        (
            "Windows.Win32.Media.PictureAcquisition",
            "IPhotoProgressDialog",
            "GetWindow",
            "getWindow(): HWND;",
        ),
        (
            "Windows.Win32.Media.DirectShow",
            "IOverlay",
            "GetWindowHandle",
            "getWindowHandle(): HWND;",
        ),
        (
            "Windows.Win32.Media.DirectShow.Tv",
            "IMSVidCtl",
            "get_Window",
            "get_Window(): HWND;",
        ),
        (
            "Windows.Win32.Media.DirectShow.Tv",
            "IMSVidRect",
            "get_HWnd",
            "get_HWnd(): HWND;",
        ),
        (
            "Windows.Win32.Media.MediaFoundation",
            "IMFPMediaPlayer",
            "GetVideoWindow",
            "getVideoWindow(): HWND;",
        ),
        (
            "Windows.Win32.Media.MediaFoundation",
            "IMFVideoDisplayControl",
            "GetVideoWindow",
            "getVideoWindow(): HWND;",
        ),
        (
            "Windows.Win32.System.WinRT",
            "ICoreWindowInterop",
            "get_WindowHandle",
            "get_WindowHandle(): HWND;",
        ),
        (
            "Windows.Win32.System.WinRT",
            "IShareWindowCommandEventArgsInterop",
            "GetWindow",
            "getWindow(): HWND;",
        ),
        (
            "Windows.Win32.System.WinRT.Xaml",
            "IDesktopWindowXamlSourceNative",
            "get_WindowHandle",
            "get_WindowHandle(): HWND;",
        ),
        (
            "Windows.Win32.System.UpdateAgent",
            "IUpdateInstaller",
            "get_ParentHwnd",
            "get_ParentHwnd(): HWND;",
        ),
        (
            "Windows.Win32.UI.Accessibility",
            "IUIAutomationElement",
            "get_CachedNativeWindowHandle",
            "get_CachedNativeWindowHandle(): HWND;",
        ),
        (
            "Windows.Win32.UI.Accessibility",
            "IUIAutomationElement",
            "get_CurrentNativeWindowHandle",
            "get_CurrentNativeWindowHandle(): HWND;",
        ),
        (
            "Windows.Win32.UI.Shell",
            "ILaunchSourceViewSizePreference",
            "GetSourceViewToPosition",
            "getSourceViewToPosition(): HWND;",
        ),
        (
            "Windows.Win32.UI.Shell",
            "IFileIsInUse",
            "GetSwitchToHWND",
            "getSwitchToHWND(): HWND;",
        ),
        (
            "Windows.Win32.UI.Shell",
            "IPreviewHandler",
            "QueryFocus",
            "queryFocus(): HWND;",
        ),
        (
            "Windows.Win32.UI.TabletPC",
            "ITextInputPanel",
            "get_AttachedEditWindow",
            "get_AttachedEditWindow(): HWND;",
        ),
        (
            "Windows.Win32.UI.TextServices",
            "ITfContextOwner",
            "GetWnd",
            "getWnd(): HWND;",
        ),
        (
            "Windows.Win32.UI.TextServices",
            "ITfContextView",
            "GetWnd",
            "getWnd(): HWND;",
        ),
    ];
    assert_eq!(cases.len(), 22);
    for (namespace, interface, method, declaration) in cases {
        let output = project_isolated_com_method(namespace, interface, method);
        assert!(
            output
                .js
                .contains(".addOut(DynCom.borrowedHandleOutputType())"),
            "{interface}.{method}:\n{}",
            output.js
        );
        assert!(
            output.js.contains("DynCom.asPointerBigint("),
            "{interface}.{method}:\n{}",
            output.js
        );
        assert!(
            output.dts.contains(declaration),
            "{interface}.{method}:\n{}",
            output.dts
        );
        assert!(!output.js.contains("adoptComPointer"));
        assert!(!output.js.contains("DestroyWindow"));
        assert!(!output.dts.contains("Buffer"));
    }

    let ole =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.System.Ole", "IOleWindow")
            .unwrap();
    let mut drifted = isolate_com_method(&ole, "GetWindow");
    drifted.raw_methods.as_mut().unwrap()[0].declaring_iid =
        "00000000-0000-0000-c000-000000000046".into();
    let error = com::generate_com_interface_files(&drifted, &win32_winmd())
        .expect_err("borrowed HWND IID drift must fail closed");
    assert!(error.contains("borrowed HWND evidence identity"), "{error}");

    let mut drifted = isolate_com_method(&ole, "GetWindow");
    drifted.raw_methods.as_mut().unwrap()[0].vtable_index += 1;
    let error = com::generate_com_interface_files(&drifted, &win32_winmd())
        .expect_err("borrowed HWND slot drift must fail closed");
    assert!(error.contains("borrowed HWND evidence identity"), "{error}");

    let mut drifted = isolate_com_method(&ole, "GetWindow");
    drifted.raw_methods.as_mut().unwrap()[0].params[0].name = "changed".into();
    let error = com::generate_com_interface_files(&drifted, &win32_winmd())
        .expect_err("borrowed HWND parameter drift must fail closed");
    assert!(
        error.contains("exact documented borrowed HWND evidence"),
        "{error}"
    );

    let mut drifted = isolate_com_method(&ole, "GetWindow");
    drifted.raw_methods.as_mut().unwrap()[0].params[0]
        .typ
        .pointer_depth = 2;
    let error = com::generate_com_interface_files(&drifted, &win32_winmd())
        .expect_err("borrowed HWND pointer-depth drift must fail closed");
    assert!(
        error.contains("exact documented borrowed HWND evidence"),
        "{error}"
    );

    let mut drifted = isolate_com_method(&ole, "GetWindow");
    drifted.raw_methods.as_mut().unwrap()[0].params[0].direction = RawParamDirection::InOut;
    let error = com::generate_com_interface_files(&drifted, &win32_winmd())
        .expect_err("borrowed HWND direction drift must fail closed");
    assert!(
        error.contains("exact documented borrowed HWND evidence"),
        "{error}"
    );

    let mut drifted = isolate_com_method(&ole, "GetWindow");
    drifted.raw_methods.as_mut().unwrap()[0].params[0].free_with =
        Some(com_metadata::RawFreeWith {
            function: "DestroyWindow".into(),
            evidence: com_metadata::RawEvidence::Override {
                reason: "mutation",
                citation: "mutation",
            },
        });
    let error = com::generate_com_interface_files(&drifted, &win32_winmd())
        .expect_err("borrowed HWND cleanup drift must fail closed");
    assert!(
        error.contains("exact documented borrowed HWND evidence"),
        "{error}"
    );

    let mut drifted = isolate_com_method(&ole, "GetWindow");
    drifted.raw_methods.as_mut().unwrap()[0].params.clear();
    let error = com::generate_com_interface_files(&drifted, &win32_winmd())
        .expect_err("borrowed HWND parameter-count drift must fail closed");
    assert!(error.contains("borrowed HWND evidence"), "{error}");

    let mut drifted = isolate_com_method(&ole, "GetWindow");
    drifted.raw_methods.as_mut().unwrap()[0].params[0].optional = true;
    let error = com::generate_com_interface_files(&drifted, &win32_winmd())
        .expect_err("borrowed HWND optionality drift must fail closed");
    assert!(
        error.contains("exact documented borrowed HWND evidence"),
        "{error}"
    );

    let mut drifted = isolate_com_method(&ole, "GetWindow");
    drifted.raw_methods.as_mut().unwrap()[0]
        .return_type
        .native_type = com_metadata::RawNativeType::I32;
    let error = com::generate_com_interface_files(&drifted, &win32_winmd())
        .expect_err("borrowed HWND return-convention drift must fail closed");
    assert!(
        error.contains("exact documented borrowed HWND evidence"),
        "{error}"
    );
}

#[test]
fn real_iolewindow_family_generates_with_borrowed_inherited_hwnd() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let interfaces = [
        ("Windows.Win32.System.Ole", "IOleWindow"),
        ("Windows.Win32.System.Ole", "IOleInPlaceActiveObject"),
        ("Windows.Win32.System.Ole", "IOleInPlaceFrame"),
        ("Windows.Win32.System.Ole", "IOleInPlaceObject"),
        ("Windows.Win32.System.Ole", "IOleInPlaceObjectWindowless"),
        ("Windows.Win32.System.Ole", "IOleInPlaceUIWindow"),
        ("Windows.Win32.UI.Shell", "IDeskBand"),
        ("Windows.Win32.UI.Shell", "IDeskBand2"),
        ("Windows.Win32.UI.Shell", "IDeskBar"),
        ("Windows.Win32.UI.Shell", "IDeskBarClient"),
        ("Windows.Win32.UI.Shell", "IDockingWindow"),
        ("Windows.Win32.UI.Shell", "IDockingWindowFrame"),
        ("Windows.Win32.UI.Shell", "IDockingWindowSite"),
        ("Windows.Win32.UI.Shell", "IMenuPopup"),
    ];
    for (namespace, interface) in interfaces {
        let meta = com_metadata::parse_com_interface(&win32_winmd(), namespace, interface).unwrap();
        let output = com::generate_com_interface_files(&meta, &win32_winmd())
            .unwrap_or_else(|error| panic!("{interface}: {error}"));
        assert!(
            output.dts.contains("getWindow(): HWND;"),
            "{interface}:\n{}",
            output.dts
        );
        assert!(
            output
                .js
                .contains(".addOut(DynCom.borrowedHandleOutputType())"),
            "{interface}:\n{}",
            output.js
        );
    }
}

#[test]
fn canonical_iunknown_owning_arrays_use_managed_values_directly() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let cases = [
        ("Windows.Win32.System.Com", "IEnumUnknown"),
        ("Windows.Win32.Storage.VirtualDiskService", "IEnumVdsObject"),
        ("Windows.Win32.System.Com.Events", "IEnumEventObject"),
    ];
    for (namespace, interface) in cases {
        let meta = com_metadata::parse_com_interface(&win32_winmd(), namespace, interface).unwrap();
        let output = com::generate_com_interface_files(&meta, &win32_winmd()).unwrap();
        assert!(
            output.js.contains("Array.from(DynCom.takeComArray("),
            "{interface}:\n{}",
            output.js
        );
        assert!(
            output.dts.contains("DynWinRtValue[]"),
            "{interface}:\n{}",
            output.dts
        );
        assert!(!output.js.contains("require('./IUnknown.js')"));
        assert!(!output.dts.contains("IUnknown[]"));
        assert!(
            output
                .extra_files
                .iter()
                .all(|(name, _)| name != "IUnknown.js" && name != "IUnknown.d.ts")
        );
        let names = output
            .extra_files
            .iter()
            .map(|(name, _)| name)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names.len(), output.extra_files.len());
    }
}

#[test]
fn windows_iid_oracle_matches_ole_window_and_iunknown() {
    use windows::core::Interface;

    assert_eq!(
        windows::Win32::System::Ole::IOleWindow::IID,
        windows::core::GUID::from_u128(0x00000114_0000_0000_c000_000000000046)
    );
    assert_eq!(
        windows::core::IUnknown::IID,
        windows::core::GUID::from_u128(0x00000000_0000_0000_c000_000000000046)
    );
}

#[test]
fn dynamic_iid_requires_exact_guid_and_output_pointer_depths() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let interface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "IDataTransferManagerInterop",
    )
    .unwrap();
    for (param_from_end, depth) in [(2usize, 2usize), (1usize, 3usize)] {
        let mut invalid = interface.clone();
        let method = invalid
            .raw_methods
            .as_mut()
            .unwrap()
            .iter_mut()
            .find(|method| method.metadata_name == "GetForWindow")
            .unwrap();
        let index = method.params.len() - param_from_end;
        method.params[index].typ.pointer_depth = depth;
        let error = com::generate_com_interface_files(&invalid, &win32_winmd()).unwrap_err();
        assert!(
            error.contains("GetForWindow") && error.contains("dynamic-IID"),
            "{error}"
        );
    }
}

#[test]
fn generalized_dynamic_iid_unlocks_non_positional_real_interfaces() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let surface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.Graphics.DirectComposition",
        "IDCompositionSurface",
    )
    .expect("IDCompositionSurface must exist");
    let output = com::generate_com_interface_files(&surface, &win32_winmd())
        .expect("IDCompositionSurface must project completely");
    assert!(
        output.dts.contains(
            "beginDraw(updateRect: RECT | null, iid: string): [DynWinRtValue, POINT];"
        ) && output.js.contains(
            ".addNullableIn(DynCom.nativeStructPointerType(_nativeLayout_RECT, true)).addIn(DynCom.pointerType()).addOut(DynCom.ownedComPointerType()).addOut(DynCom.nativeStructType(_nativeLayout_POINT))"
        ) && output.js.contains(
            "invokeAll(this._obj, [updateRect === null ? DynCom.nullNativeStructPointer() : DynCom.nativeStruct(_nativeLayout_RECT, updateRect), DynCom.iidPointer(_iid)])"
        ) && output.js.contains(
            "return [DynCom.adoptComPointer(_r[0], _iid), DynCom.nativeStructBytes(_nativeLayout_POINT, _r[1])];"
        ),
        "BeginDraw must preserve the non-terminal dynamic output and following POINT result:\n{}",
        output.js
    );

    let texture = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.Graphics.DirectComposition",
        "IDCompositionTexture",
    )
    .expect("IDCompositionTexture must exist");
    let output = com::generate_com_interface_files(&texture, &win32_winmd())
        .expect("IDCompositionTexture must project completely");
    assert!(
        output
            .dts
            .contains("getAvailableFence(iid: string): [bigint, DynWinRtValue];")
            && output.js.contains(
                ".addOut(DynCom.u64Type()).addIn(DynCom.pointerType()).addOut(DynCom.ownedComPointerType())"
            )
            && output
                .js
                .contains("invokeAll(this._obj, [DynCom.iidPointer(_iid)])")
            && output.js.contains(
                "return [DynCom.toU64Bigint(_r[0]), DynCom.adoptComPointer(_r[1], _iid)];"
            ),
        "GetAvailableFence must preserve output order before dynamic adoption:\n{}",
        output.js
    );

    let class_factory = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.Ole",
        "IClassFactory2",
    )
    .expect("IClassFactory2 must exist");
    let output = com::generate_com_interface_files(&class_factory, &win32_winmd())
        .expect("IClassFactory2 must project completely");
    assert!(
        output.dts.contains(
            "createInstanceLic(pUnkOuter: DynWinRtValue | null, pUnkReserved: DynWinRtValue | null, riid: string, bstrKey: string): DynWinRtValue;"
        ) && output.js.contains(
            "createInstanceLic(pUnkOuter, pUnkReserved, riid, bstrKey)"
        ) && output.js.contains(
            "DynCom.iidPointer(_iid), DynCom.bstr(bstrKey)"
        ) && output.js.contains("DynCom.adoptComPointer(_out, _iid)"),
        "CreateInstanceLic must keep the visible BSTR after its non-adjacent IID:\n{}",
        output.js
    );

    use windows::Win32::Graphics::DirectComposition::{IDCompositionSurface, IDCompositionTexture};
    use windows::Win32::System::Ole::IClassFactory2;
    use windows::core::Interface;
    assert_eq!(
        surface.interface.iid,
        format!("{:?}", IDCompositionSurface::IID).to_ascii_lowercase()
    );
    assert_eq!(
        texture.interface.iid,
        format!("{:?}", IDCompositionTexture::IID).to_ascii_lowercase()
    );
    assert_eq!(
        class_factory.interface.iid,
        format!("{:?}", IClassFactory2::IID).to_ascii_lowercase()
    );
}

#[test]
fn real_dynamic_iid_optional_and_multi_interface_shapes_fail_closed() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let dxc = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.Graphics.Direct3D.Dxc",
        "IDxcResult",
    )
    .expect("IDxcResult must exist");
    let get_output = dxc
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .find(|method| method.metadata_name == "GetOutput")
        .unwrap();
    assert!(get_output.params[2].optional);
    let error = com::generate_com_interface_files(&dxc, &win32_winmd())
        .expect_err("optional IDxcResult dynamic output must fail closed");
    assert!(
        error.contains("GetOutput") && error.contains("dynamic-IID"),
        "{error}"
    );

    let viewport = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.Graphics.DirectManipulation",
        "IDirectManipulationViewport2",
    )
    .expect("IDirectManipulationViewport2 must exist");
    let get_tag = viewport
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .find(|method| method.metadata_name == "GetTag")
        .unwrap();
    assert!(get_tag.params[1].optional);
    let error = com::generate_com_interface_files(&viewport, &win32_winmd())
        .expect_err("optional GetTag dynamic output must fail closed");
    assert!(
        error.contains("GetTag") && error.contains("dynamic-IID"),
        "{error}"
    );

    let primary = isolate_com_method(&viewport, "GetPrimaryContent");
    let output = com::generate_com_interface_files(&primary, &win32_winmd())
        .expect("required GetPrimaryContent shape must project in isolation");
    assert!(
        output
            .dts
            .contains("getPrimaryContent(riid: string): DynWinRtValue;")
            && output.js.contains("DynCom.adoptComPointer(_out, _iid)"),
        "{}",
        output.js
    );

    let lookup = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.Media.MediaFoundation",
        "IMFTopologyServiceLookup",
    )
    .expect("IMFTopologyServiceLookup must exist");
    let error = com::generate_com_interface_files(&lookup, &win32_winmd())
        .expect_err("multi-interface lookup array must fail closed");
    assert!(
        error.contains("LookupService")
            && error.contains("dynamic-IID")
            && error.contains("in, out"),
        "{error}"
    );

    let netcfg = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.NetworkManagement.NetManagement",
        "INetCfg",
    )
    .expect("INetCfg must exist");
    let error = com::generate_com_interface_files(&netcfg, &win32_winmd())
        .expect_err("optional QueryNetCfgClass output must no longer be adopted");
    assert!(
        error.contains("QueryNetCfgClass") && error.contains("dynamic-IID"),
        "{error}"
    );

    use windows::Win32::Graphics::Direct3D::Dxc::IDxcResult;
    use windows::Win32::Graphics::DirectManipulation::IDirectManipulationViewport2;
    use windows::core::Interface;
    assert_eq!(
        dxc.interface.iid,
        format!("{:?}", IDxcResult::IID).to_ascii_lowercase()
    );
    assert_eq!(
        viewport.interface.iid,
        format!("{:?}", IDirectManipulationViewport2::IID).to_ascii_lowercase()
    );
}

#[test]
fn owned_handle_outputs_fail_without_a_projected_cleanup_owner() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let interface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "IExtractIconW",
    )
    .expect("IExtractIconW must exist");
    let error = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect_err("owned HICON outputs must not project as borrowed bigint values");
    assert!(
        error.contains("owned handle output") || error.contains("cleanup"),
        "{error}"
    );
}

#[test]
fn production_codegen_requires_validated_raw_com_facts() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let mut interface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "ITaskbarList3",
    )
    .expect("ITaskbarList3 must exist");
    let pbutton = interface
        .raw_methods
        .as_mut()
        .unwrap()
        .iter_mut()
        .find(|method| method.metadata_name == "ThumbBarAddButtons")
        .and_then(|method| {
            method
                .params
                .iter_mut()
                .find(|param| param.name == "pButton")
        })
        .expect("ThumbBarAddButtons.pButton raw facts must exist");
    pbutton.typ.constness = com_metadata::RawConstness::Unspecified;

    let error = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect_err("production projection must reject incomplete raw pointer facts");
    assert!(
        error.contains("ThumbBarAddButtons")
            && error.contains("pButton")
            && error.contains("unknown pointer meaning"),
        "semantic validation must fail before the legacy adapter: {error}"
    );
}

#[test]
fn production_projection_ignores_legacy_abi_fields() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let mut interface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "ITaskbarList3",
    )
    .expect("ITaskbarList3 must exist");
    let baseline = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect("baseline generation must succeed");

    let method = interface
        .interface
        .methods
        .iter_mut()
        .find(|method| method.name == "SetProgressValue")
        .expect("SetProgressValue must exist");
    method.params[1].typ = TypeMeta::Bool;
    method.params[1].direction = com_metadata::ParamDirection::Out;
    method.vtable_index = 999;
    method.return_type = None;
    method.preserve_hresult = true;
    method.owned_outputs = vec![com_metadata::OwnedOutput {
        param_index: 1,
        free_with: "NotARealCleanup".into(),
    }];
    interface
        .referenced_enums
        .iter_mut()
        .find(|definition| definition.name == "TBPFLAG")
        .unwrap()
        .members[0]
        .value = com_metadata::ComEnumValue::Signed(999);

    let projected = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect("validated raw contracts, not compatibility ABI fields, drive projection");
    assert_eq!(projected.js, baseline.js);
    assert_eq!(projected.dts, baseline.dts);
    assert_eq!(projected.extra_files, baseline.extra_files);
}

#[test]
fn semantic_interface_rejects_reserved_and_duplicate_slots() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let mut inspectable = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.WinRT",
        "ISystemMediaTransportControlsInterop",
    )
    .unwrap();
    inspectable.raw_methods.as_mut().unwrap()[0].vtable_index = 3;
    let error = com::generate_com_interface_files(&inspectable, &win32_winmd()).unwrap_err();
    assert!(
        error.contains("reserved vtable slot") && error.contains("first user slot is 6"),
        "{error}"
    );

    let mut taskbar = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "ITaskbarList3",
    )
    .unwrap();
    let methods = taskbar.raw_methods.as_mut().unwrap();
    methods[1].vtable_index = methods[0].vtable_index;
    let error = com::generate_com_interface_files(&taskbar, &win32_winmd()).unwrap_err();
    assert!(error.contains("duplicate vtable slot"), "{error}");
}

#[test]
fn coclass_activation_requires_a_valid_nonzero_clsid() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let mut coclass =
        com_metadata::parse_com_coclass(&win32_winmd(), "Windows.Win32.UI.Shell", "TaskbarList")
            .unwrap()
            .unwrap();

    coclass.primary_interface.interface.iid =
        coclass.primary_interface.interface.iid.to_ascii_uppercase();
    com::generate_com_coclass_files(&coclass, &win32_winmd())
        .expect("valid IID casing must not affect primary-interface selection");

    coclass.clsid = "not-a-guid".into();
    let error = com::generate_com_coclass_files(&coclass, &win32_winmd()).unwrap_err();
    assert!(error.contains("invalid GUID"), "{error}");

    coclass.clsid = "00000000-0000-0000-0000-000000000000".into();
    let error = com::generate_com_coclass_files(&coclass, &win32_winmd()).unwrap_err();
    assert!(error.contains("zero CLSID"), "{error}");
}

#[test]
fn unresolved_named_pointer_aliases_fail_closed_before_name_classification() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let mut interface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "ITaskbarList3",
    )
    .unwrap();
    let hwnd = interface
        .raw_methods
        .as_mut()
        .unwrap()
        .iter_mut()
        .find(|method| method.metadata_name == "AddTab")
        .unwrap()
        .params
        .first_mut()
        .unwrap();
    if let com_metadata::RawNativeType::Named { kind, .. } = &mut hwnd.typ.native_type {
        *kind = com_metadata::RawNamedKind::Unknown;
    } else {
        panic!("AddTab HWND must be a named type");
    }

    let error = com::generate_com_interface_files(&interface, &win32_winmd()).unwrap_err();
    assert!(error.contains("unknown native type"), "{error}");
}

#[test]
fn pointer_sized_integers_use_runtime_width() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let interface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.ClrHosting",
        "IApartmentCallback",
    )
    .expect("IApartmentCallback must exist");
    let output = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect("pointer-sized parameters should be supported");

    assert!(
        output
            .js
            .contains(".addIn(DynCom.usizeType()).addIn(DynCom.usizeType())"),
        "USize parameters must use runtime-width ABI types:\n{}",
        output.js
    );
    assert!(
        output.js.contains("DynCom.usize(BigInt(pFunc))")
            && output.js.contains("DynCom.usize(BigInt(pData))"),
        "USize values must use runtime-width constructors:\n{}",
        output.js
    );
    assert!(
        !output.js.contains("DynCom.u64Type()"),
        "pointer-sized parameters must not be fixed to 64 bits:\n{}",
        output.js
    );
}

#[test]
fn required_parameters_after_string_buffer_count_remain_required() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let mut interface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "IExtractImage",
    )
    .expect("IExtractImage must exist");
    interface
        .interface
        .methods
        .retain(|method| method.name == "GetLocation");
    interface
        .raw_methods
        .as_mut()
        .unwrap()
        .retain(|method| method.projected_name == "GetLocation");
    let output = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect("isolated IExtractImage.GetLocation generation should succeed");

    assert!(
        output
            .js
            .contains("getLocation(cch, pdwPriority, prgSize, recClrDepth, pdwFlags)"),
        "required parameters, including cch before them, must not get defaults:\n{}",
        output.js
    );
    assert!(
        !output.js.contains("prgSize = 0")
            && !output.js.contains("dwRecClrDepth = 0")
            && !output.js.contains("pdwFlags = 0"),
        "required native arguments must not be silently defaulted:\n{}",
        output.js
    );
    assert!(
        output.dts.contains(
            "getLocation(cch: number, pdwPriority: number, prgSize: SIZE, recClrDepth: number, pdwFlags: number)"
        ),
        "declarations must keep the parameters required:\n{}",
        output.dts
    );
}

#[test]
fn bstr_outputs_are_decoded_and_freed() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let interface =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.System.Com", "IErrorInfo")
            .expect("IErrorInfo must exist");
    let output = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect("IErrorInfo generation should succeed");

    assert!(
        output.js.contains("return DynCom.takeBstr(_out);"),
        "BSTR outputs must be converted through the freeing helper:\n{}",
        output.js
    );
    assert!(
        output.dts.contains("getDescription(): string;"),
        "BSTR outputs must project as strings:\n{}",
        output.dts
    );
}

#[test]
fn com_p0_interfaces_generate_with_exact_pointer_and_ownership_contracts() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let malloc =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.System.Com", "IMalloc")
            .expect("IMalloc must exist");
    let malloc_output = com::generate_com_interface_files(&malloc, &win32_winmd())
        .expect("IMalloc generation should succeed");
    assert!(
        malloc_output
            .dts
            .contains("alloc(cb: bigint): DynComAllocation | null;")
            && malloc_output.dts.contains(
                "realloc(pv: DynComAllocation | null, cb: bigint): DynComAllocation | null;"
            )
            && malloc_output
                .dts
                .contains("free(pv: DynComAllocation | null): void;"),
        "{}",
        malloc_output.dts
    );
    assert!(
        malloc_output
            .js
            .contains("return DynCom.takeMallocAllocation(this._obj, _out);")
            && malloc_output
                .js
                .contains("DynCom.mallocAllocationPointer(this._obj, pv)")
            && malloc_output
                .js
                .contains("DynCom.mallocInspectionPointer(pv)")
            && !malloc_output
                .js
                .contains("didAlloc(pv) {\n        const _out = _IMalloc.method(7).invoke(this._obj, [DynCom.mallocAllocationPointer(this._obj, pv)])")
            && malloc_output
                .js
                .contains("DynCom.takeMallocAllocationPointer(this._obj, pv)")
            && malloc_output.js.contains("const _mallocSize = BigInt(cb);")
            && malloc_output
                .js
                .contains("DynCom.finishMallocReallocation(this._obj, pv, _mallocSize, _out)"),
        "{}",
        malloc_output.js
    );

    let class_factory = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.Com",
        "IClassFactory",
    )
    .expect("IClassFactory must exist");
    let class_factory_output = com::generate_com_interface_files(&class_factory, &win32_winmd())
        .expect("IClassFactory generation should succeed");
    assert!(
        class_factory_output.dts.contains(
            "createInstance(pUnkOuter: DynWinRtValue | null, riid: string): DynWinRtValue;"
        ) && class_factory_output
            .js
            .contains("return DynCom.adoptComPointer(_out, _iid);"),
        "{}\n{}",
        class_factory_output.dts,
        class_factory_output.js
    );

    let create_error = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.Ole",
        "ICreateErrorInfo",
    )
    .expect("ICreateErrorInfo must exist");
    let create_error_output = com::generate_com_interface_files(&create_error, &win32_winmd())
        .expect("ICreateErrorInfo generation should succeed");
    assert!(
        create_error_output
            .dts
            .contains("setGUID(rguid: string): void;")
            && create_error_output
                .js
                .contains("DynCom.iidPointer(WinGuid.parse(rguid))"),
        "{}\n{}",
        create_error_output.dts,
        create_error_output.js
    );
}

#[test]
fn imalloc_exact_contract_fails_closed_on_metadata_drift() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let mut malloc =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.System.Com", "IMalloc")
            .expect("IMalloc must exist");
    let alloc = malloc
        .raw_methods
        .as_mut()
        .expect("raw COM methods")
        .iter_mut()
        .find(|method| method.metadata_name == "Alloc")
        .expect("IMalloc::Alloc");
    alloc.return_type.pointer_depth = 0;
    let error = com::generate_com_interface_files(&malloc, &win32_winmd()).unwrap_err();
    assert!(
        error.contains("IMalloc.Alloc signature no longer matches exact contract evidence"),
        "{error}"
    );
}

#[test]
fn unsigned_enum_values_preserve_their_value() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let enum_type = com_metadata::parse_com_enum(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "FILEOPERATION_FLAGS",
    )
    .expect("FILEOPERATION_FLAGS must exist");
    let value = enum_type
        .members
        .iter()
        .find(|member| member.name == "FOFX_DONTDISPLAYLOCATIONS")
        .expect("FOFX_DONTDISPLAYLOCATIONS must exist")
        .value
        .clone();

    assert!(matches!(enum_type.underlying, TypeMeta::U32));
    assert_eq!(value, com_metadata::ComEnumValue::Unsigned(2_147_483_648));

    let shared_type = meta::parse_enums(&win32_winmd(), "Windows.Win32.UI.Shell")
        .into_iter()
        .find(|typ| {
            matches!(
                typ,
                TypeMeta::Enum { name, .. } if name == "FILEOPERATION_FLAGS"
            )
        })
        .expect("shared enum parser must still find FILEOPERATION_FLAGS");
    let TypeMeta::Enum {
        underlying,
        members,
        ..
    } = shared_type
    else {
        unreachable!()
    };
    assert!(
        matches!(*underlying, TypeMeta::I32),
        "the shared WinRT model must remain unchanged"
    );
    assert_eq!(
        members
            .iter()
            .find(|member| member.name == "FOFX_DONTDISPLAYLOCATIONS")
            .unwrap()
            .value,
        i32::MIN,
        "unsigned Win32 values must be corrected only in the COM-local model"
    );
}

#[test]
fn shelllink_getpath_projects_validated_find_data_pod() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let interface =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.UI.Shell", "IShellLinkW")
            .expect("IShellLinkW must exist");
    let output = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect("WIN32_FIND_DATAW has a complete validated POD layout");
    assert!(
        output
            .dts
            .contains("getPath(cch: number, pfd: WIN32_FIND_DATAW | null, fFlags: number): [string, WIN32_FIND_DATAW | null];")
            && output.dts.contains("createWIN32_FIND_DATAW")
            && output.js.contains("DynCom.nativeStructPointerType(")
            && output.js.contains("DynCom.nullNativeStructPointer()")
            && output.js.contains("DynCom.nativeStructBytes(")
            && output.js.contains("\"size\":592")
            && output.js.contains("\"count\":260")
            && output.js.contains("Windows.Win32.Foundation.FILETIME"),
        "GetPath must render only the validated nested/fixed-array POD plan:\n{}",
        output.dts
    );
}

#[test]
fn real_win32_pods_cover_value_pointer_out_and_inout_shapes() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let drop_target = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.Ole",
        "IDropTarget",
    )
    .expect("IDropTarget must exist");
    let drop_output = com::generate_com_interface_files(&drop_target, &win32_winmd())
        .expect("POINTL by-value methods must project");
    assert!(
        drop_output
            .dts
            .contains("export type POINTL = DynComNativeStruct &")
    );
    assert!(
        drop_output
            .dts
            .contains("dragOver(grfKeyState: MODIFIERKEYS_FLAGS, pt: POINTL")
    );
    assert!(
        drop_output
            .js
            .contains("\"name\":\"Windows.Win32.Foundation.POINTL\",\"x86\":{\"size\":8")
    );

    let bind_ctx =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.System.Com", "IBindCtx")
            .expect("IBindCtx must exist");
    let bind_output = com::generate_com_interface_files(&bind_ctx, &win32_winmd())
        .expect("BIND_OPTS must project with an exact cbStruct initializer");
    assert!(
        bind_output
            .dts
            .contains("export declare function createBIND_OPTS(bytes?: Buffer): BIND_OPTS;")
            && bind_output
                .dts
                .contains("setBindOptions(pbindopts: BIND_OPTS): void;")
            && bind_output
                .dts
                .contains("getBindOptions(pbindopts: BIND_OPTS): BIND_OPTS;"),
        "{}",
        bind_output.dts
    );
    assert!(
        bind_output
            .js
            .contains("\"initializers\":[{\"kind\":\"sizeOfLayout\",\"field\":\"cbStruct\"}]")
            && bind_output
                .js
                .contains(".addInOut(DynCom.nativeStructType(_nativeLayout_BIND_OPTS))"),
        "{}",
        bind_output.js
    );

    let stream =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.System.Com", "IStream")
            .expect("IStream must exist");
    let stream_output = com::generate_com_interface_files(&stream, &win32_winmd())
        .expect("the complete IStream inheritance chain must project");
    assert!(
        stream_output
            .dts
            .contains("stat(grfStatFlag: number): DynComStatStg;")
            && stream_output
                .dts
                .contains("seek(dlibMove: bigint, origin: STREAM_SEEK): bigint;")
            && stream_output.dts.contains("clone(): DynWinRtValue;"),
        "{}",
        stream_output.dts
    );
    assert!(
        stream_output
            .js
            .contains(".addMethodAt(3, 'Read', new DynComMethodSig().addCallerOutputBuffer")
            && stream_output.js.contains(
                ".addMethodAt(12, 'Stat', new DynComMethodSig().addOut(DynCom.statStgType())"
            )
            && stream_output
                .js
                .contains("return DynCom.takeStatStg(_out);"),
        "{}",
        stream_output.js
    );

    let running_object_table = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.Com",
        "IRunningObjectTable",
    )
    .expect("IRunningObjectTable must exist");
    let rot_output = com::generate_com_interface_files(&running_object_table, &win32_winmd())
        .expect("FILETIME input and output methods must project");
    assert!(
        rot_output
            .dts
            .contains("noteChangeTime(register: number, pfiletime: FILETIME): void;")
    );
    assert!(
        rot_output
            .dts
            .contains("getTimeOfLastChange(pmkObjectName: DynWinRtValue): FILETIME;")
    );
    assert!(
        rot_output
            .js
            .contains("DynCom.nativeStructPointerType(_nativeLayout_FILETIME)")
    );
    assert!(
        rot_output
            .js
            .contains(".addOut(DynCom.nativeStructType(_nativeLayout_FILETIME))")
    );
}

#[test]
fn istream_stat_exact_contract_fails_closed_on_metadata_drift() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let mut stream =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.System.Com", "IStream")
            .expect("IStream must exist");
    let stat = stream
        .raw_methods
        .as_mut()
        .expect("raw COM methods")
        .iter_mut()
        .find(|method| method.metadata_name == "Stat")
        .expect("IStream::Stat");
    stat.params[0].name = "drifted".into();
    let error = com::generate_com_interface_files(&stream, &win32_winmd()).unwrap_err();
    assert!(
        error.contains("IStream.Stat signature no longer matches exact contract evidence"),
        "{error}"
    );

    let mut stream =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.System.Com", "IStream")
            .expect("IStream must exist");
    let stat = stream
        .raw_methods
        .as_mut()
        .expect("raw COM methods")
        .iter_mut()
        .find(|method| method.metadata_name == "Stat")
        .expect("IStream::Stat");
    stat.declaring_iid = "11111111-2222-3333-4444-555555555555".into();
    let error = com::generate_com_interface_files(&stream, &win32_winmd()).unwrap_err();
    assert!(
        error.contains("IStream.Stat declaring interface identity no longer matches"),
        "{error}"
    );
}

#[test]
fn shelllink_other_methods_remain_unchanged_with_pod_support() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let interface =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.UI.Shell", "IShellLinkW")
            .expect("IShellLinkW must exist");
    let output = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect("IShellLinkW generation should succeed");

    assert!(
        output
            .dts
            .contains("getIconLocation(cch?: number): [string, number];"),
        "other string-buffer methods must still project correctly:\n{}",
        output.dts
    );
}

#[test]
fn namespace_mode_rejects_classic_com_instead_of_using_winrt_slots() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args([
            "generate",
            "--winmd",
            &win32_winmd(),
            "--namespace",
            "Windows.Win32.UI.Shell",
            "--dry-run",
        ])
        .output()
        .expect("spawn dynwinrt-codegen");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "namespace mode must fail closed");
    assert!(
        stderr.contains("classic-COM namespace projection is not supported")
            && stderr.contains("--class-name"),
        "failure must direct callers to the safe class mode:\n{stderr}"
    );
}

#[test]
fn correlation_vector_hstring_output_is_owned_and_projected_as_string() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let interface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.WinRT",
        "ICorrelationVectorSource",
    )
    .expect("ICorrelationVectorSource must exist");
    let method = interface
        .interface
        .methods
        .iter()
        .find(|method| method.name == "get_CorrelationVector")
        .expect("get_CorrelationVector must exist");

    assert!(matches!(method.params[0].typ, TypeMeta::String));
    let output = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect("HSTRING output generation must succeed");
    assert!(output.js.contains(".addOut(DynCom.hstringType())"));
    assert!(output.js.contains("return _out.toString();"));
    assert!(output.dts.contains("get_CorrelationVector(): string;"));
}

#[test]
fn unresolved_external_interface_fails_until_reference_metadata_is_loaded() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let unresolved = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.WinRT.Composition",
        "ICompositorInterop",
    )
    .expect("ICompositorInterop must exist");
    let error = com::generate_com_interface_files(&unresolved, &win32_winmd())
        .expect_err("missing Windows metadata must fail closed");
    assert!(error.contains("ICompositionSurface"));
    assert!(error.contains("--ref"));

    let Some(windows_winmd) = discovered_windows_winmd() else {
        eprintln!("Skipping resolved-reference half: Windows.winmd not available");
        return;
    };
    let metadata = format!("{};{}", win32_winmd(), windows_winmd);
    let resolved = com_metadata::parse_com_interface(
        &metadata,
        "Windows.Win32.System.WinRT.Composition",
        "ICompositorInterop",
    )
    .expect("ICompositorInterop must resolve with Windows.winmd");
    let output = com::generate_com_interface_files(&resolved, &metadata)
        .expect("resolved external interface generation must succeed");

    let create_graphics_device = resolved
        .interface
        .methods
        .iter()
        .find(|method| method.name == "CreateGraphicsDevice")
        .expect("CreateGraphicsDevice must exist");
    let TypeMeta::RuntimeClass {
        default_interface: Some(default_interface),
        ..
    } = &create_graphics_device.params[1].typ
    else {
        panic!("CreateGraphicsDevice must return a resolved runtime class");
    };
    let TypeMeta::Interface { iid, .. } = default_interface.as_ref() else {
        panic!("runtime class default must resolve to an interface");
    };
    assert!(!iid.is_empty());
    assert!(output.js.contains(&format!(
        ".addOut(DynCom.interfaceType(WinGuid.parse('{iid}')))"
    )));
    assert!(output.dts.contains("DynWinRtValue"));
}

#[test]
fn semantic_hresult_preserves_metadata_and_documented_contracts() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let mut interface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.Com",
        "IPersistFile",
    )
    .expect("IPersistFile must exist");
    let is_dirty = interface
        .interface
        .methods
        .iter()
        .find(|method| method.name == "IsDirty")
        .expect("IsDirty must exist");
    let load = interface
        .interface
        .methods
        .iter()
        .find(|method| method.name == "Load")
        .expect("Load must exist");
    let get_cur_file = interface
        .interface
        .methods
        .iter()
        .find(|method| method.name == "GetCurFile")
        .expect("GetCurFile must exist");

    assert!(is_dirty.preserve_hresult);
    assert!(get_cur_file.preserve_hresult);
    assert!(!load.preserve_hresult);

    let mut get_cur_file_interface = interface.clone();
    interface
        .interface
        .methods
        .retain(|method| method.name == "IsDirty");
    interface
        .raw_methods
        .as_mut()
        .unwrap()
        .retain(|method| method.projected_name == "IsDirty");
    let output = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect("semantic HRESULT generation must succeed");
    assert!(output.js.contains(".preserveHresult()"));
    assert!(output.js.contains("return DynCom.toNumber(_out);"));
    assert!(output.dts.contains("isDirty(): number;"));

    get_cur_file_interface
        .interface
        .methods
        .retain(|method| method.name == "GetCurFile");
    get_cur_file_interface
        .raw_methods
        .as_mut()
        .unwrap()
        .retain(|method| method.projected_name == "GetCurFile");
    let output = com::generate_com_interface_files(&get_cur_file_interface, &win32_winmd())
        .expect("GetCurFile semantic HRESULT generation must succeed");
    assert!(output.js.contains(".preserveHresult()"));
    assert!(output.js.contains(".invokeAll("));
    assert!(output.js.contains("DynCom.toNumber(_r[0])"));
    assert!(output.js.contains("DynCom.takeCoTaskMemWideString(_r[1])"));
    assert!(output.dts.contains("getCurFile(): [number, string];"));
}

#[test]
fn scalar_typedef_uses_its_underlying_abi_not_pointer_abi() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let interface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "IPreviewHandlerVisuals",
    )
    .expect("IPreviewHandlerVisuals must exist");
    let output = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect("COLORREF scalar typedef must generate");

    assert!(output.dts.contains("export type COLORREF = number;"));
    assert!(
        output
            .dts
            .contains("setBackgroundColor(color: COLORREF): void;")
    );
    assert!(output.js.contains(".addIn(DynCom.u32Type())"));
    assert!(output.js.contains("DynCom.u32(color)"));
    assert!(!output.js.contains("DynCom.pointer(color)"));
}

#[test]
fn metadata_delegate_parameter_fails_closed_as_a_delegate() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let interface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.Graphics.Direct2D",
        "ID2D1Factory1",
    )
    .expect("ID2D1Factory1 must exist");
    let register = interface
        .interface
        .methods
        .iter()
        .find(|method| method.name == "RegisterEffectFromStream")
        .expect("RegisterEffectFromStream must exist");
    assert!(register.params.iter().any(|param| {
        matches!(
            &param.typ,
            TypeMeta::Delegate { name, .. } if name == "PD2D1_EFFECT_FACTORY"
        )
    }));

    let error = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect_err("delegate parameters require a managed callback projection");
    assert!(
        (error.contains("PD2D1_EFFECT_FACTORY") && error.contains("managed callback projection"))
            || (error.contains("D2D1_PROPERTY_BINDING")
                && error.contains("nested string/interface ownership")),
        "{error}"
    );
}

#[test]
fn by_value_guid_is_not_treated_as_a_dynamic_iid_pointer() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let interface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.System.WinRT.Display",
        "IDisplayDeviceInterop",
    )
    .expect("IDisplayDeviceInterop must exist");
    let open = interface
        .interface
        .methods
        .iter()
        .find(|method| method.name == "OpenSharedHandle")
        .expect("OpenSharedHandle must exist");
    assert!(matches!(open.params[1].typ, TypeMeta::Guid));

    let error = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect_err("by-value GUID plus void** must not be treated as REFIID interop");
    assert!(error.contains("untyped pointer output has no ownership projection"));
}

#[test]
fn com_only_generation_emits_an_importable_package_shape() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let output_dir = std::env::temp_dir().join(format!(
        "dynwinrt-codegen-com-package-{}",
        std::process::id()
    ));
    if output_dir.exists() {
        fs::remove_dir_all(&output_dir).expect("remove stale COM package test directory");
    }

    let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args([
            "generate",
            "--winmd",
            &win32_winmd(),
            "--namespace",
            "Windows.Win32.UI.Shell",
            "--class-name",
            "ITaskbarList3",
            "--output",
            output_dir.to_str().unwrap(),
        ])
        .output()
        .expect("spawn dynwinrt-codegen");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "COM generation failed:\n{stderr}");

    for name in ["index.js", "index.d.ts", "package.json"] {
        assert!(
            output_dir.join(name).is_file(),
            "COM-only compatibility output must include {name}"
        );
    }
    for name in [
        "index.js",
        "index.mjs",
        "index.d.ts",
        "package.json",
        "ITaskbarList3.js",
        "ITaskbarList3.d.ts",
    ] {
        assert!(
            output_dir.join("com").join(name).is_file(),
            "COM subpackage must include {name}"
        );
    }
    assert!(!output_dir.join("ITaskbarList3.js").exists());
    let index = fs::read_to_string(output_dir.join("index.js")).unwrap();
    assert!(index.contains("ITaskbarList3") && index.contains("TBPFLAG"));
    assert!(index.contains("__exportLazy('ITaskbarList3', './com/ITaskbarList3.js')"));
    let com_index = fs::read_to_string(output_dir.join("com").join("index.js")).unwrap();
    assert!(com_index.contains("__exportLazy('ITaskbarList3', './ITaskbarList3.js')"));
    let com_esm_index = fs::read_to_string(output_dir.join("com").join("index.mjs")).unwrap();
    assert!(com_esm_index.contains("import * as __m"));
    let package = fs::read_to_string(output_dir.join("package.json")).unwrap();
    assert!(package.contains("\"type\": \"commonjs\""));
    assert!(package.contains("\"./ITaskbarList3\""));
    assert!(package.contains("\"./com\""));
    assert!(package.contains("\"./com/*\""));
    assert!(package.contains("\"import\": \"./com/index.mjs\""));
    assert!(package.contains("\"require\": \"./com/index.js\""));

    let incremental = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args([
            "generate",
            "--winmd",
            &win32_winmd(),
            "--namespace",
            "Windows.Win32.UI.Shell",
            "--class-name",
            "IShellLinkW",
            "--output",
            output_dir.to_str().unwrap(),
        ])
        .output()
        .expect("spawn incremental dynwinrt-codegen");
    let incremental_stderr = String::from_utf8_lossy(&incremental.stderr);
    assert!(
        incremental.status.success(),
        "incremental COM generation failed:\n{incremental_stderr}"
    );
    let incremental_index = fs::read_to_string(output_dir.join("com").join("index.js")).unwrap();
    assert!(
        incremental_index.contains("ITaskbarList3")
            && incremental_index.contains("IShellLinkW")
            && incremental_index.contains("TBPFLAG"),
        "incremental generation must preserve earlier exports:\n{incremental_index}"
    );
    let incremental_package = fs::read_to_string(output_dir.join("package.json")).unwrap();
    assert!(
        incremental_package.contains("\"./ITaskbarList3\"")
            && incremental_package.contains("\"./IShellLinkW\""),
        "incremental generation must preserve earlier package subpaths:\n{incremental_package}"
    );

    let manifest_path = output_dir.join("com").join(".dynwinrt-com-manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    let root = manifest["roots"]["Windows.Win32.UI.Shell.ITaskbarList3"]
        .as_array_mut()
        .expect("ITaskbarList3 manifest root");
    root.push(serde_json::Value::String("Stale.js".into()));
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(
        output_dir.join("com").join("Stale.js"),
        "exports.Stale = 1;\n",
    )
    .unwrap();
    let regenerate = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args([
            "generate",
            "--winmd",
            &win32_winmd(),
            "--namespace",
            "Windows.Win32.UI.Shell",
            "--class-name",
            "ITaskbarList3",
            "--output",
            output_dir.to_str().unwrap(),
        ])
        .output()
        .expect("regenerate manifest root");
    assert!(
        regenerate.status.success(),
        "manifest regeneration failed:\n{}",
        String::from_utf8_lossy(&regenerate.stderr)
    );
    assert!(
        !output_dir.join("com").join("Stale.js").exists(),
        "regenerating one COM root must remove only its stale manifest-owned files"
    );
    let regenerated_index = fs::read_to_string(output_dir.join("com").join("index.js")).unwrap();
    assert!(
        regenerated_index.contains("ITaskbarList3")
            && regenerated_index.contains("IShellLinkW")
            && !regenerated_index.contains("Stale"),
        "manifest cleanup must preserve other incremental roots:\n{regenerated_index}"
    );

    fs::remove_dir_all(&output_dir).expect("remove COM package test directory");
}

#[test]
fn mixed_generation_supports_one_command_and_both_incremental_orders() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let Some(windows_winmd) = discovered_windows_winmd() else {
        eprintln!("Skipping: Windows.winmd not available");
        return;
    };
    let metadata = format!("{};{}", windows_winmd, win32_winmd());
    let temp = std::env::temp_dir();
    let mixed = temp.join(format!(
        "dynwinrt-codegen-mixed-package-{}",
        std::process::id()
    ));
    let winrt_first = temp.join(format!(
        "dynwinrt-codegen-mixed-winrt-first-{}",
        std::process::id()
    ));
    let com_first = temp.join(format!(
        "dynwinrt-codegen-mixed-com-first-{}",
        std::process::id()
    ));
    let legacy_com_first = temp.join(format!(
        "dynwinrt-codegen-mixed-legacy-com-first-{}",
        std::process::id()
    ));
    for output_dir in [&mixed, &winrt_first, &com_first, &legacy_com_first] {
        if output_dir.exists() {
            fs::remove_dir_all(output_dir).expect("remove stale mixed package test directory");
        }
    }

    run_codegen_command(
        &metadata,
        None,
        "Windows.Foundation.Uri,Windows.Win32.UI.Shell.ITaskbarList3",
        &mixed,
    );
    assert_mixed_package_shape(&mixed);

    run_codegen_command(&metadata, Some("Windows.Foundation"), "Uri", &winrt_first);
    run_codegen_command(
        &metadata,
        Some("Windows.Win32.UI.Shell"),
        "ITaskbarList3",
        &winrt_first,
    );
    assert_mixed_package_shape(&winrt_first);

    run_codegen_command(
        &metadata,
        Some("Windows.Win32.UI.Shell"),
        "ITaskbarList3",
        &com_first,
    );
    let com_only_package = fs::read_to_string(com_first.join("package.json")).unwrap();
    assert!(com_only_package.contains("\"type\": \"commonjs\""));
    assert!(
        fs::read_to_string(com_first.join("index.d.ts"))
            .unwrap()
            .contains("ITaskbarList3")
    );

    run_codegen_command(&metadata, Some("Windows.Foundation"), "Uri", &com_first);
    assert_mixed_package_shape(&com_first);

    run_codegen_command(
        &metadata,
        Some("Windows.Win32.UI.Shell"),
        "ITaskbarList3",
        &legacy_com_first,
    );
    let com_dir = legacy_com_first.join("com");
    let legacy_index = fs::read_to_string(com_dir.join("index.js")).unwrap();
    let legacy_dts = fs::read_to_string(com_dir.join("index.d.ts")).unwrap();
    for entry in fs::read_dir(&com_dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name != "index.js"
            && name != "index.mjs"
            && name != "index.d.ts"
            && name != "package.json"
        {
            fs::copy(entry.path(), legacy_com_first.join(name.as_ref())).unwrap();
        }
    }
    fs::write(legacy_com_first.join("index.js"), &legacy_index).unwrap();
    fs::write(legacy_com_first.join("index.d.ts"), legacy_dts).unwrap();
    fs::remove_dir_all(com_dir).unwrap();

    run_codegen_command(
        &metadata,
        Some("Windows.Foundation"),
        "Uri",
        &legacy_com_first,
    );
    assert_mixed_package_shape(&legacy_com_first);
    assert!(!legacy_com_first.join("ITaskbarList3.js").exists());
    assert!(!legacy_com_first.join("TBPFLAG.js").exists());

    for output_dir in [&mixed, &winrt_first, &com_first, &legacy_com_first] {
        fs::remove_dir_all(output_dir).expect("remove mixed package test directory");
    }
}

/// Requirement 8: multi-interface COM generation must be atomic enough that
/// a later interface's projection failure doesn't leave an earlier,
/// successfully-projected interface's files (or the `com/` barrel) newly
/// written on disk. `ITaskbarList3` projects cleanly; `IShellFolder` fails
/// closed because `ParseDisplayName` has an untyped owned pointer output.
/// Requesting both in one `--class-name` batch must
/// fail the whole command AND leave no `com/` output directory at all (since
/// this is a from-scratch generation into a directory that doesn't exist
/// yet).
#[test]
fn multi_interface_com_generation_does_not_leave_partial_output_on_projection_failure() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let output_dir = std::env::temp_dir().join(format!(
        "dynwinrt-codegen-com-atomic-{}",
        std::process::id()
    ));
    if output_dir.exists() {
        fs::remove_dir_all(&output_dir).expect("remove stale atomic COM test directory");
    }

    let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args([
            "generate",
            "--winmd",
            &win32_winmd(),
            "--namespace",
            "Windows.Win32.UI.Shell",
            "--class-name",
            "ITaskbarList3,IShellFolder",
            "--output",
            output_dir.to_str().unwrap(),
        ])
        .output()
        .expect("spawn dynwinrt-codegen");

    assert!(
        !output.status.success(),
        "generation must fail overall when any requested interface fails to project"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("IShellFolder") && stderr.contains("ParseDisplayName"),
        "failure must name the offending interface/parameter:\n{stderr}"
    );

    // No partial `com/` output (nor any of ITaskbarList3's already-projected
    // files) may have been written as a side effect of the failed batch.
    assert!(
        !output_dir.join("com").exists(),
        "a failed multi-interface COM batch must not leave a partial com/ directory: {:?}",
        output_dir.join("com")
    );

    if output_dir.exists() {
        fs::remove_dir_all(&output_dir).expect("remove atomic COM test directory");
    }
}
