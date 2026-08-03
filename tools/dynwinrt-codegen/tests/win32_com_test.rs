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

/// Returns a clone of `iface` with the named method removed.
///
/// `IShellLinkW::GetPath` takes a caller-writable `WIN32_FIND_DATAW*` (`pfd`)
/// that has no safe layout/size projection (see the classic-COM-ABI skill's
/// fail-closed requirement for unsupported writable native structs), so the
/// whole interface now fails closed if it is included. Tests that only need
/// `IShellLinkW`'s *other* members use this helper to exercise them without
/// tripping that (separately, correctly, regression-tested) failure.
fn without_method(
    mut iface: com_metadata::ComInterfaceMeta,
    name: &str,
) -> com_metadata::ComInterfaceMeta {
    iface.interface.methods.retain(|method| method.name != name);
    iface
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
            .contains("super(DynCom.coCreateInstance(CLSID_TaskbarList, IID_ITaskbarList4))")
    );
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

    // `GetPath` is intentionally unsupported (see `without_method`); it is
    // covered by its own fail-closed regression test below.
    let interface_without_get_path = without_method(interface, "GetPath");
    let output =
        com::generate_com_interface_files(&interface_without_get_path, &win32_winmd()).unwrap();
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
    set_import_name("../dist/com.js");

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
        out.js.contains("require('../dist/com.js')"),
        "classic-COM .js must honor --import-name (expected `require('../dist/com.js')`):\n{}",
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
            .contains("require('@microsoft/dynwinrt/com')"),
        "after restoring, default import must use '@microsoft/dynwinrt/com':\n{}",
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
    set_import_name("../dist/com.js");

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
        out.js.contains("require('../dist/com.js')"),
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
    let com_iface = without_method(com_iface, "GetPath");
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

    let interface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.Storage.FileSystem",
        "IDiskQuotaControl",
    )
    .expect("IDiskQuotaControl must exist");
    let output = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect("IDiskQuotaControl generation should succeed");

    assert!(
        output
            .dts
            .contains("addUserSid(pUserSid: PSID | Buffer | Uint8Array"),
        "PSID input must accept backing storage rather than handle bytes:\n{}",
        output.dts
    );
    assert!(
        output.js.contains("DynCom.pointer(pUserSid)")
            && !output.js.contains("handleValue(pUserSid)"),
        "PSID Buffer must pass its address, not decoded contents:\n{}",
        output.js
    );
}

#[test]
fn native_array_buffers_fail_closed() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let interface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.Storage.Imapi",
        "IDiscRecorder",
    )
    .expect("IDiscRecorder must exist");
    let error = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect_err("NativeArrayInfo byte buffers must not become scalar in/out storage");

    assert!(
        error.contains("GetRecorderGUID")
            && error.contains("pbyUniqueID")
            && error.contains("caller-sized native buffers are not supported"),
        "generation must fail with a targeted buffer diagnostic: {error}"
    );
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

    let interface = com_metadata::parse_com_interface(
        &win32_winmd(),
        "Windows.Win32.UI.Shell",
        "IExtractImage",
    )
    .expect("IExtractImage must exist");
    let output = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect("IExtractImage generation should succeed");

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
            "getLocation(cch: number, pdwPriority: number, prgSize: bigint | Buffer, recClrDepth: number, pdwFlags: number)"
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
fn shelllink_getpath_fails_closed_on_unsupported_find_data_struct() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    // `IShellLinkW::GetPath` takes a caller-writable `WIN32_FIND_DATAW* pfd`
    // (an [in, out] multi-field struct with no NativeArrayInfo count and no
    // recognized single-field pointer-alias/scalar shape). There is no safe
    // layout/size projection for it, so generation must fail closed with an
    // actionable diagnostic instead of silently treating it as an opaque
    // caller-owned buffer (the removed `pfd`/`finddata` name heuristic).
    let interface =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.UI.Shell", "IShellLinkW")
            .expect("IShellLinkW must exist");
    let error = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect_err("GetPath's WIN32_FIND_DATAW* must fail closed, not fall back to a heuristic");
    assert!(
        error.contains("GetPath") && error.contains("pfd") && error.contains("WIN32_FIND_DATAW"),
        "diagnostic must name the interface, method, and offending parameter:\n{error}"
    );
}

#[test]
fn shelllink_other_methods_still_generate_when_getpath_is_excluded() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    // Everything else on IShellLinkW (including its OTHER string-buffer
    // methods, e.g. GetIconLocation) is unaffected by GetPath's fail-closed
    // struct-layout limitation; a consumer can still generate the interface
    // by excluding the one unsupported method.
    let interface =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.UI.Shell", "IShellLinkW")
            .expect("IShellLinkW must exist");
    let interface = without_method(interface, "GetPath");
    let output = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect("IShellLinkW generation should succeed once GetPath is excluded");

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
    let output = com::generate_com_interface_files(&interface, &win32_winmd())
        .expect("semantic HRESULT generation must succeed");
    assert!(output.js.contains(".preserveHresult()"));
    assert!(output.js.contains("return DynCom.toNumber(_out);"));
    assert!(output.dts.contains("isDirty(): number;"));

    get_cur_file_interface
        .interface
        .methods
        .retain(|method| method.name == "GetCurFile");
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
    assert!(error.contains("PD2D1_EFFECT_FACTORY"));
    assert!(error.contains("managed callback projection"));
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
            // `IShellLinkW` is deliberately NOT used here: its `GetPath`
            // parameter `pfd` (`WIN32_FIND_DATAW*`) has no safe layout/size
            // projection and now correctly fails closed (see the
            // `shelllink_getpath_fails_closed_on_unsupported_find_data_struct`
            // regression test), which would fail this whole interface's
            // generation. `IShellItem` exercises the same incremental
            // multi-interface + referenced-enum path (it references both
            // `SIGDN` and `SFGAO_FLAGS`) without hitting that limitation.
            "IShellItem",
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
            && incremental_index.contains("IShellItem")
            && incremental_index.contains("TBPFLAG")
            && incremental_index.contains("SIGDN"),
        "incremental generation must preserve earlier exports:\n{incremental_index}"
    );
    let incremental_package = fs::read_to_string(output_dir.join("package.json")).unwrap();
    assert!(
        incremental_package.contains("\"./ITaskbarList3\"")
            && incremental_package.contains("\"./IShellItem\""),
        "incremental generation must preserve earlier package subpaths:\n{incremental_package}"
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
/// written on disk. `ITaskbarList3` projects cleanly; `IShellLinkW` fails
/// closed (its `GetPath` parameter `pfd` is an unsupported
/// `WIN32_FIND_DATAW*`). Requesting both in one `--class-name` batch must
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
            "ITaskbarList3,IShellLinkW",
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
        stderr.contains("IShellLinkW") && stderr.contains("pfd"),
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
