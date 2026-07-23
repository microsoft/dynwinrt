// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! TDD tests for classic-COM (option A) code generation from Windows.Win32.winmd.
//!
//! These tests drive the implementation of:
//! - Base-aware vtable slot computation (walks interface_impls chain)
//! - IUnknown vs IInspectable base offset (3 vs 6)
//! - Coclass CLSID discovery for `create()` activation
//! - Natural TS/JS wrapper generation for classic-COM interfaces
//!
//! Tests are skipped (with an `eprintln!` note) when the Win32 winmd is not
//! present at the well-known path.

use std::fs;
use std::path::{Path, PathBuf};

use dynwinrt_codegen::codegen::com;
use dynwinrt_codegen::codegen::project::{get_import_name, set_import_name};
use dynwinrt_codegen::meta;

const WIN32_WINMD: &str = r"C:\s\win32metadata\Windows.Win32.winmd";

fn win32_available() -> bool {
    Path::new(WIN32_WINMD).exists()
}

/// Resolve a `Windows.winmd` from the newest installed Windows SDK, matching
/// the discovery logic the codegen itself uses. Returns `None` if no SDK is
/// installed on this machine (the test that calls this should skip in that
/// case, consistent with other tests in this module).
fn discovered_windows_winmd() -> Option<String> {
    meta::discover_newest_windows_winmd()
}

// -------------------------------------------------------------------------
// NORMAL tests
// -------------------------------------------------------------------------

/// 1. Parse ITaskbarList3 from Win32 metadata → correct IID.
#[test]
fn parse_itaskbarlist3_iid() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available at {}", WIN32_WINMD);
        return;
    }
    let com_iface =
        meta::parse_com_interface(WIN32_WINMD, "Windows.Win32.UI.Shell", "ITaskbarList3")
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
    let com_iface =
        meta::parse_com_interface(WIN32_WINMD, "Windows.Win32.UI.Shell", "ITaskbarList3")
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
    let com_iface =
        meta::parse_com_interface(WIN32_WINMD, "Windows.Win32.UI.Shell", "ITaskbarList3").unwrap();
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
    let com_iface =
        meta::parse_com_interface(WIN32_WINMD, "Windows.Win32.UI.Shell", "ITaskbarList3").unwrap();
    assert_eq!(
        com_iface.coclass_clsid.as_deref(),
        Some("56fdf344-fd6d-11d0-958a-006097c9a090")
    );
    assert_eq!(com_iface.coclass_name.as_deref(), Some("TaskbarList"));
}

/// 5. Param type mapping: HWND → pointer/handle, TBPFLAG → enum, HRESULT → void.
#[test]
fn param_type_mapping() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let com_iface =
        meta::parse_com_interface(WIN32_WINMD, "Windows.Win32.UI.Shell", "ITaskbarList3").unwrap();

    // Generate wrapper as a text bundle we can inspect for the mapping decisions
    let out = com::generate_com_interface_files(&com_iface, WIN32_WINMD)
        .expect("codegen must succeed for classic-COM interface");

    let dts = out.dts.as_str();
    let js = out.js.as_str();

    // HWND is a handle type → bigint | Buffer surface
    assert!(
        dts.contains("bigint | Buffer") || dts.contains("bigint|Buffer"),
        "HWND should be projected as `bigint | Buffer` in .d.ts, got:\n{}",
        dts
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

/// 6. Partial generation: generating a single class-name yields ONLY that
///    interface plus its immediate deps (enum, coclass metadata), NOT the
///    entire Windows.Win32.UI.Shell namespace.
#[test]
fn partial_generation_only_emits_target_interface() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let com_iface =
        meta::parse_com_interface(WIN32_WINMD, "Windows.Win32.UI.Shell", "ITaskbarList3").unwrap();
    let out = com::generate_com_interface_files(&com_iface, WIN32_WINMD)
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
    let com_iface =
        meta::parse_com_interface(WIN32_WINMD, "Windows.Win32.UI.Shell", "ITaskbarList3").unwrap();
    let out = com::generate_com_interface_files(&com_iface, WIN32_WINMD)
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

/// 8. Generated `.js`: activation uses a CoCreateInstance path with CLSID + IID;
///    methods invoke at the correct base-aware slots.
#[test]
fn js_body_uses_cocreateinstance_and_correct_slots() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let com_iface =
        meta::parse_com_interface(WIN32_WINMD, "Windows.Win32.UI.Shell", "ITaskbarList3").unwrap();
    let out = com::generate_com_interface_files(&com_iface, WIN32_WINMD)
        .expect("codegen must succeed for classic-COM interface");
    let js = out.js.as_str();

    // CLSID + IID appear in .js
    assert!(
        js.contains("56fdf344-fd6d-11d0-958a-006097c9a090"),
        ".js must embed the CLSID:\n{}",
        js
    );
    assert!(
        js.contains("ea1afb91-9e28-4b86-90e9-9e9f8a5eefaf"),
        ".js must embed the IID:\n{}",
        js
    );

    // Activation via coCreateInstance
    assert!(
        js.contains("coCreateInstance"),
        ".js must use coCreateInstance for activation:\n{}",
        js
    );

    // registerInterfaceUnknown (not registerInterface) since IUnknown-based
    assert!(
        js.contains("registerInterfaceUnknown"),
        ".js must use registerInterfaceUnknown for classic COM:\n{}",
        js
    );

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
    let missing =
        meta::parse_com_interface(WIN32_WINMD, "Windows.Win32.UI.Shell", "IDoesNotExist_XYZ");
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
    let com_iface = meta::parse_com_interface(WIN32_WINMD, "Windows.Win32.System.Com", "IPersist")
        .expect("IPersist must exist in Win32 metadata");
    assert!(
        com_iface.coclass_clsid.is_none(),
        "IPersist has no associated coclass CLSID"
    );

    let out = com::generate_com_interface_files(&com_iface, WIN32_WINMD)
        .expect("codegen must succeed for classic-COM interface");
    let js = out.js.as_str();
    let dts = out.dts.as_str();

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
        let com = meta::parse_com_interface(WIN32_WINMD, "Windows.Win32.UI.Shell", "ITaskbarList3")
            .unwrap();
        com::generate_com_interface_files(&com, WIN32_WINMD)
            .expect("codegen must succeed for classic-COM interface")
    };
    let b = {
        let com = meta::parse_com_interface(WIN32_WINMD, "Windows.Win32.UI.Shell", "ITaskbarList3")
            .unwrap();
        com::generate_com_interface_files(&com, WIN32_WINMD)
            .expect("codegen must succeed for classic-COM interface")
    };
    assert_eq!(a.js, b.js);
    assert_eq!(a.dts, b.dts);
    assert_eq!(a.extra_files, b.extra_files);
}

// -------------------------------------------------------------------------
// SNAPSHOT test
// -------------------------------------------------------------------------

/// Snapshot test: lock generated ITaskbarList3 .js + .d.ts against committed files.
///
/// To update snapshots after an intentional change:
///   cargo run -p dynwinrt-codegen -- generate \
///     --winmd C:\s\win32metadata\Windows.Win32.winmd \
///     --namespace Windows.Win32.UI.Shell \
///     --class-name ITaskbarList3 \
///     --output tools/dynwinrt-codegen/tests/snapshots/itaskbarlist3
#[test]
fn snapshot_itaskbarlist3() {
    if !win32_available() {
        eprintln!("Skipping snapshot test: Win32 winmd not available");
        return;
    }
    let com_iface =
        meta::parse_com_interface(WIN32_WINMD, "Windows.Win32.UI.Shell", "ITaskbarList3")
            .expect("ITaskbarList3 must exist");
    let out = com::generate_com_interface_files(&com_iface, WIN32_WINMD)
        .expect("codegen must succeed for classic-COM interface");

    let snapshot_dir: PathBuf =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/itaskbarlist3");
    assert!(
        snapshot_dir.exists(),
        "Snapshot directory not found: {}",
        snapshot_dir.display()
    );

    let mut generated: Vec<(String, String)> = Vec::new();
    generated.push(("ITaskbarList3.js".into(), out.js.clone()));
    generated.push(("ITaskbarList3.d.ts".into(), out.dts.clone()));
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
    set_import_name("../dist/index.js");

    let result = std::panic::catch_unwind(|| {
        let com_iface =
            meta::parse_com_interface(WIN32_WINMD, "Windows.Win32.UI.Shell", "ITaskbarList3")
                .expect("ITaskbarList3 must exist");
        com::generate_com_interface_files(&com_iface, WIN32_WINMD)
            .expect("codegen must succeed for classic-COM interface")
    });

    // Always restore before propagating any assertion failure.
    set_import_name(&previous);

    let out = result.unwrap_or_else(|e| std::panic::resume_unwind(e));

    // Custom import must appear on the runtime import line...
    assert!(
        out.js.contains("from '../dist/index.js'"),
        "classic-COM .js must honor --import-name (expected `from '../dist/index.js'`):\n{}",
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
        let com_iface =
            meta::parse_com_interface(WIN32_WINMD, "Windows.Win32.UI.Shell", "ITaskbarList3")
                .expect("ITaskbarList3 must exist");
        com::generate_com_interface_files(&com_iface, WIN32_WINMD)
            .expect("codegen must succeed for classic-COM interface")
    };
    assert!(
        default_out.js.contains("from '@microsoft/dynwinrt'"),
        "after restoring, default import name must be back to '@microsoft/dynwinrt':\n{}",
        default_out.js
    );
}

/// Same test for the *interop wrapper* generation path (the second hardcoded
/// site in `com.rs`) — regenerating `IDataTransferManagerInterop` with a
/// custom import name should thread through to the emitted
/// `DataTransferManager.js` companion.
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
    set_import_name("../dist/index.js");

    let result = std::panic::catch_unwind(|| {
        let com_iface = meta::parse_com_interface(
            WIN32_WINMD,
            "Windows.Win32.UI.Shell",
            "IDataTransferManagerInterop",
        )
        .expect("IDataTransferManagerInterop must exist");
        com::generate_com_interface_files(&com_iface, WIN32_WINMD)
            .expect("codegen must succeed for classic-COM interop interface")
    });

    set_import_name(&previous);
    let out = result.unwrap_or_else(|e| std::panic::resume_unwind(e));

    // The interop .js itself must honor the flag.
    assert!(
        out.js.contains("from '../dist/index.js'"),
        "interop .js must honor --import-name:\n{}",
        out.js
    );
    assert!(
        !out.js.contains("'@microsoft/dynwinrt'"),
        "interop .js must NOT hardcode '@microsoft/dynwinrt':\n{}",
        out.js
    );

    // And the projected companion class file must honor it too.
    let companion = out
        .extra_files
        .iter()
        .find(|(name, _)| name == "DataTransferManager.js")
        .map(|(_, content)| content.as_str())
        .expect("DataTransferManager.js companion must be emitted");
    assert!(
        companion.contains("from '../dist/index.js'"),
        "projected companion .js must honor --import-name:\n{}",
        companion
    );
    assert!(
        !companion.contains("'@microsoft/dynwinrt'"),
        "projected companion .js must NOT hardcode '@microsoft/dynwinrt':\n{}",
        companion
    );
}

#[test]
fn shellitem_getdisplayname_is_not_classified_as_caller_owned_string_buffer() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let com_iface = meta::parse_com_interface(WIN32_WINMD, "Windows.Win32.UI.Shell", "IShellItem")
        .expect("IShellItem must exist");
    let out = com::generate_com_interface_files(&com_iface, WIN32_WINMD)
        .expect("codegen must succeed for IShellItem");

    assert!(
        out.js.contains(".addMethod('GetDisplayName', new DynWinRtMethodSig().addIn(DynWinRtType.i32Type()).addOut(DynWinRtType.pointer()))"),
        "PWSTR* callee-allocated output must remain addOut(pointer), not caller-owned buffer:\n{}",
        out.js
    );
    assert!(
        !out.js.contains("getDisplayName(sigdnName = 260)")
            && !out.js.contains("_decodeWideString"),
        "IShellItem.GetDisplayName must not allocate/decode a caller-owned buffer:\n{}",
        out.js
    );
}
