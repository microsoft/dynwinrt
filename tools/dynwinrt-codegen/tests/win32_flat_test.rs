// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! TDD tests for flat-Win32 `[DllImport]` code generation from Windows.Win32.winmd.
//!
//! Covers:
//! - Metadata discovery of `Apis`-class static DllImport methods (dll, entry
//!   point, params with direction, return type).
//! - Natural JS/DTS wrapper emission via `codegen::flat::generate_flat_apis_files`.
//! - Corner cases: out-param projection, void/no-arg exports, partial generation,
//!   and non-regression of the classic-COM / WinRT paths.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use dynwinrt_codegen::codegen::com;
use dynwinrt_codegen::codegen::flat;
use dynwinrt_codegen::com_metadata;
use dynwinrt_codegen::meta;
use dynwinrt_codegen::meta::{FlatAbiType, FlatDirection};
use dynwinrt_codegen::types::TypeMeta;

/// Path to `Windows.Win32.winmd`. Overridable via the `DYNWINRT_WIN32_WINMD`
/// environment variable so this suite can run on CI and other machines without
/// editing the source; falls back to the common local checkout path.
fn win32_winmd() -> String {
    std::env::var("DYNWINRT_WIN32_WINMD")
        .unwrap_or_else(|_| r"C:\s\win32metadata\Windows.Win32.winmd".to_string())
}
const REGISTRY_NS: &str = "Windows.Win32.System.Registry";

fn win32_available() -> bool {
    Path::new(&win32_winmd()).exists()
}

// ---------------------------------------------------------------------------
// NORMAL: metadata discovery
// ---------------------------------------------------------------------------

/// 1. Discover flat `[DllImport]` static methods for a namespace's `Apis`
///    class. The `Apis` class must NOT be treated as a COM interface.
#[test]
fn discover_flat_apis_for_registry_namespace() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let apis = meta::parse_flat_apis(&win32_winmd(), REGISTRY_NS, "Apis")
        .expect("Registry Apis class should parse as a flat-DllImport container");
    assert_eq!(apis.namespace, REGISTRY_NS);
    assert_eq!(apis.class_name, "Apis");
    assert!(
        !apis.methods.is_empty(),
        "must discover at least one flat method"
    );
    let names: Vec<&str> = apis.methods.iter().map(|m| m.name.as_str()).collect();
    for expected in &["RegOpenKeyExW", "RegQueryValueExW", "RegCloseKey"] {
        assert!(
            names.contains(expected),
            "expected `{expected}` in Registry Apis, got: {names:?}"
        );
    }

    // The `Apis` class is NOT a COM interface — parse_com_interface should
    // return None (no interface with that name) OR a Some whose IID is empty.
    let as_com = com_metadata::parse_com_interface(&win32_winmd(), REGISTRY_NS, "Apis");
    if let Some(ci) = as_com {
        assert!(
            ci.interface.iid.is_empty(),
            "Apis is not a COM interface but parse_com_interface returned an IID"
        );
    }
}

/// 2. `RegOpenKeyExW` parses correctly: dll = advapi32.dll (any case),
///    entry point = "RegOpenKeyExW", params in order, LSTATUS (i32) return.
#[test]
fn parse_reg_open_key_ex_w() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let apis = meta::parse_flat_apis(&win32_winmd(), REGISTRY_NS, "Apis").unwrap();
    let m = apis
        .methods
        .iter()
        .find(|m| m.name == "RegOpenKeyExW")
        .expect("RegOpenKeyExW must be discovered");
    assert!(
        m.dll.to_ascii_lowercase().starts_with("advapi32"),
        "expected advapi32.dll, got {}",
        m.dll
    );
    assert_eq!(m.entry_point, "RegOpenKeyExW");

    // Return type: WIN32_ERROR is a U32 enum but at the ABI it's a 32-bit int
    // (LSTATUS). The generator projects LSTATUS as a signed number.
    match &m.return_type {
        FlatAbiType::Enum {
            name, underlying, ..
        } => {
            assert_eq!(name, "WIN32_ERROR");
            assert!(matches!(**underlying, FlatAbiType::U32 | FlatAbiType::I32));
        }
        other => panic!("expected Enum return type for WIN32_ERROR, got {:?}", other),
    }

    // Params: hKey (HKEY), lpSubKey (PWSTR), ulOptions (u32), samDesired (enum),
    // phkResult (PtrTo(HKEY), out).
    assert_eq!(m.params.len(), 5);
    let by = |n: &str| m.params.iter().find(|p| p.name == n).unwrap();

    let hkey = by("hKey");
    assert!(
        matches!(&hkey.abi, FlatAbiType::Handle { name, .. } if name == "HKEY"),
        "hKey must be Handle{{HKEY}}: {:?}",
        hkey.abi
    );
    assert_eq!(hkey.direction, FlatDirection::In);

    let sub = by("lpSubKey");
    assert_eq!(sub.abi, FlatAbiType::PWStr);
    assert_eq!(sub.direction, FlatDirection::In);

    let opt = by("ulOptions");
    assert_eq!(opt.abi, FlatAbiType::U32);
    assert_eq!(opt.direction, FlatDirection::In);

    let sam = by("samDesired");
    assert!(
        matches!(&sam.abi, FlatAbiType::Enum { name, .. } if name == "REG_SAM_FLAGS"),
        "samDesired must be REG_SAM_FLAGS enum: {:?}",
        sam.abi
    );
    if let FlatAbiType::Enum { underlying, .. } = &sam.abi {
        assert!(
            matches!(**underlying, FlatAbiType::U32),
            "REG_SAM_FLAGS must preserve its unsigned U32 backing type: {:?}",
            sam.abi
        );
    }

    let phk = by("phkResult");
    match &phk.abi {
        FlatAbiType::PtrTo(inner) => match inner.as_ref() {
            FlatAbiType::Handle { name, .. } => assert_eq!(name, "HKEY"),
            other => panic!("expected PtrTo(Handle{{HKEY}}), got PtrTo({:?})", other),
        },
        other => panic!("phkResult must be PtrTo(HKEY): {:?}", other),
    }

    assert_eq!(phk.direction, FlatDirection::Out, "phkResult must be [out]");
}

#[test]
fn parse_unsigned_win32_enum_preserves_u32_backing_and_codegen_coerces_high_bit() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }

    let apis = meta::parse_flat_apis(&win32_winmd(), REGISTRY_NS, "Apis").unwrap();
    let m = apis
        .methods
        .iter()
        .find(|m| m.name == "RegSetKeySecurity")
        .expect("RegSetKeySecurity must be discovered");
    let security_information = m
        .params
        .iter()
        .find(|p| p.name == "SecurityInformation")
        .expect("SecurityInformation param must be discovered");
    match &security_information.abi {
        FlatAbiType::Enum {
            name, underlying, ..
        } => {
            assert_eq!(name, "OBJECT_SECURITY_INFORMATION");
            assert!(
                matches!(**underlying, FlatAbiType::U32),
                "OBJECT_SECURITY_INFORMATION must preserve unsigned U32 backing: {:?}",
                security_information.abi
            );
        }
        other => panic!("expected OBJECT_SECURITY_INFORMATION enum, got {:?}", other),
    }

    let out = flat::generate_flat_apis_files(&apis);
    assert!(
        out.js.contains("DynWinRtValue.u32((securityInformation) >>> 0)"),
        "unsigned high-bit enum args must coerce through >>> 0 before napi u32 conversion:\n{}",
        out.js
    );
}

#[test]
fn parse_get_proc_address_return_is_pointer() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let apis = meta::parse_flat_apis(&win32_winmd(), "Windows.Win32.System.LibraryLoader", "Apis")
        .expect("LibraryLoader Apis should parse");
    let m = apis
        .methods
        .iter()
        .find(|m| m.name == "GetProcAddress")
        .expect("GetProcAddress must be discovered");
    assert_eq!(m.return_type, FlatAbiType::Ptr);
    let out = flat::generate_flat_apis_files(&synth_apis(vec![m.clone()]));
    assert!(
        out.js.contains("'GetProcAddress', 'Ptr'"),
        "GetProcAddress must use Ptr retKind:\n{}",
        out.js
    );
    assert!(
        out.js.contains("_ret.asPointerBigint()"),
        "GetProcAddress must decode pointer returns as BigInt:\n{}",
        out.js
    );
}

#[test]
fn reg_connect_registry_ex_projects_status_like_non_ex_variant() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let out = generate_registry_apis();
    for name in ["regConnectRegistryW", "regConnectRegistryExW"] {
        let idx = out
            .js
            .find(&format!("export function {name}"))
            .unwrap_or_else(|| panic!("{name} must be generated"));
        let body = &out.js[idx..out.js[idx..].find("\n}\n").map(|end| idx + end).unwrap()];
        assert!(
            body.contains("status: _ret.toNumber()"),
            "{name} must project LSTATUS/WIN32_ERROR-family return as status:\n{body}"
        );
        assert!(
            !body.contains("result: _ret.toNumber()"),
            "{name} must not project status-code return as result:\n{body}"
        );
    }

    let numeric = flat::generate_flat_apis_files(&synth_apis(vec![synth_method(
        "PlainI32Value",
        FlatAbiType::I32,
    )]));
    assert!(
        numeric.js.contains("return { result: _ret.toNumber() };"),
        "plain I32 value returns must still project as result:\n{}",
        numeric.js
    );
}

// ---------------------------------------------------------------------------
// NORMAL: natural wrapper emission
// ---------------------------------------------------------------------------

fn generate_registry_apis() -> flat::FlatGeneratedOutput {
    let apis = meta::parse_flat_apis(&win32_winmd(), REGISTRY_NS, "Apis").unwrap();
    flat::generate_flat_apis_files(&apis)
}

#[test]
fn opaque_pointer_param_dts_accepts_uint8array() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    // Regression: opaque pointer params (e.g. Registry `data`) must accept
    // Uint8Array in the .d.ts. The runtime `DynWinRtValue.pointer()` accepts a
    // Uint8Array, so typing only `bigint | Buffer` makes a valid Uint8Array
    // argument a spurious TypeScript error.
    let out = generate_registry_apis();
    assert!(
        out.dts.contains("bigint | Buffer | Uint8Array | null"),
        "opaque pointer .d.ts must accept Uint8Array:\n{}",
        out.dts
    );
    assert!(
        !out.dts.contains("data: bigint | Buffer | null"),
        "opaque pointer .d.ts must not omit Uint8Array:\n{}",
        out.dts
    );
}

/// 3. Emit a NATURAL wrapper whose `.js` calls
///    `DynWinRtValue.flatInvoke('advapi32.dll', 'RegOpenKeyExW', 'I32', [...])`
///    and whose `.d.ts` types params naturally — no raw `flatInvoke` string
///    leaked at the typed surface.
#[test]
fn emit_natural_registry_wrapper() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let out = generate_registry_apis();

    // The generated .js must call flatInvoke against advapi32 for each fn.
    let js = &out.js;
    assert!(
        js.contains("flatInvoke"),
        ".js must invoke DynWinRtValue.flatInvoke: {}",
        js
    );
    assert!(
        js.to_ascii_lowercase().contains("advapi32.dll"),
        ".js must reference advapi32.dll"
    );
    for fname in &["RegOpenKeyExW", "RegQueryValueExW", "RegCloseKey"] {
        assert!(
            js.contains(&format!("'{fname}'")) || js.contains(&format!("\"{fname}\"")),
            ".js must reference entry point `{fname}`"
        );
    }
    // camelCase surface in .js
    for camel in &["regOpenKeyExW", "regQueryValueExW", "regCloseKey"] {
        assert!(
            js.contains(&format!("{camel}(")),
            ".js must expose `{camel}` as a natural function"
        );
    }

    // .d.ts must NOT leak raw flatInvoke; params should be typed naturally.
    let dts = &out.dts;
    assert!(
        !dts.contains("flatInvoke"),
        ".d.ts must not leak raw flatInvoke"
    );
    // Natural types for the primary shapes.
    assert!(
        dts.contains("HKEY") || dts.contains("hkey"),
        ".d.ts should surface HKEY typedef"
    );
    assert!(
        dts.contains("string"),
        ".d.ts should type LPCWSTR params as string"
    );
}

/// 4. Partial generation: only the requested namespace/class is emitted;
///    the CLI reference to another namespace's flat container is not required
///    (this is a unit-level check on the meta layer).
#[test]
fn partial_generation_only_requested_namespace() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let registry = meta::parse_flat_apis(&win32_winmd(), REGISTRY_NS, "Apis").unwrap();
    for m in &registry.methods {
        // Every method belongs to the Registry namespace's advapi32 exports.
        assert!(
            m.dll.to_ascii_lowercase().contains("advapi32")
                || m.dll.to_ascii_lowercase().contains("kernel32")
                || m.dll.to_ascii_lowercase().contains("api-ms-"),
            "Registry Apis unexpectedly refers to {}",
            m.dll
        );
    }
}

/// 5. Determinism: two consecutive generations of the same class emit
///    byte-identical output.
#[test]
fn generation_is_deterministic() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let a = generate_registry_apis();
    let b = generate_registry_apis();
    assert_eq!(a.js, b.js, "generated .js must be deterministic");
    assert_eq!(a.dts, b.dts, "generated .d.ts must be deterministic");
    assert_eq!(
        a.extra_files, b.extra_files,
        "generated sibling files must be deterministic"
    );
}

/// 5b. Snapshot: golden files under
///     `tests/snapshots/registry_apis/`. Update the snapshot by running:
///
///     cargo run -p dynwinrt-codegen -- generate \
///       --winmd C:\s\win32metadata\Windows.Win32.winmd \
///       --namespace Windows.Win32.System.Registry \
///       --class-name Apis \
///       --output tools\dynwinrt-codegen\tests\snapshots\registry_apis
#[test]
fn snapshot_registry_apis() {
    if !win32_available() {
        eprintln!("Skipping snapshot: Win32 winmd not available");
        return;
    }
    let out = generate_registry_apis();

    let snapshot_dir: PathBuf =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/registry_apis");
    if !snapshot_dir.exists() {
        panic!(
            "Snapshot directory not found: {}\n\
             Create it and populate with the CLI shown above.",
            snapshot_dir.display()
        );
    }

    let mut generated: Vec<(String, String)> = Vec::new();
    generated.push(("Apis.js".into(), out.js.clone()));
    generated.push(("Apis.d.ts".into(), out.dts.clone()));
    for (name, content) in &out.extra_files {
        generated.push((name.clone(), content.clone()));
    }

    let mut mismatches: Vec<String> = Vec::new();
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
            "Registry Apis snapshot mismatch!\n{}\n\n\
             To update, re-run the generator into the snapshot dir.",
            mismatches.join("\n")
        );
    }
}

// ---------------------------------------------------------------------------
// CORNER: out-param projection
// ---------------------------------------------------------------------------

/// 6. A flat method with an out-param (PHKEY on RegOpenKeyExW) projects the
///    out as a return value. The generator hoists pure-Out pointer-to-scalar
///    params into the return so the caller doesn't have to allocate a Buffer.
#[test]
fn out_param_projects_as_return() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let out = generate_registry_apis();
    let dts = &out.dts;

    // Locate the regOpenKeyExW declaration.
    let sig_line = dts
        .lines()
        .find(|l| l.contains("regOpenKeyExW"))
        .expect(".d.ts must declare regOpenKeyExW");

    // The PHKEY out-slot must appear in the return type, NOT the parameter list.
    // Extract just the parameter list (text between the FIRST `(` and its
    // matching `)`) and assert `phkResult` is absent there. The return-type
    // portion after the `:` is expected to contain it.
    let open = sig_line
        .find('(')
        .expect("regOpenKeyExW signature must have a param list");
    let close = sig_line[open..]
        .find(')')
        .map(|i| open + i)
        .expect("regOpenKeyExW signature must close its param list");
    let params = &sig_line[open + 1..close];
    assert!(
        !params.contains("phkResult"),
        "regOpenKeyExW must hoist phkResult out of the params list; \
         params were: `{params}` in full sig: {sig_line}"
    );
    // Return shape must include HKEY (either as bare or a field).
    assert!(
        sig_line.to_lowercase().contains("hkey"),
        "regOpenKeyExW return type must expose the HKEY: {sig_line}"
    );
    // And the return type (text after the closing paren) MUST expose phkResult.
    let ret = &sig_line[close..];
    assert!(
        ret.contains("phkResult"),
        "regOpenKeyExW return type must expose phkResult: {ret}"
    );
}

/// 7. Status-only return, single input, no out-params: `RegCloseKey(HKEY)
///    -> LSTATUS` — exercise the "one [in] param + status return, no out
///    projection" shape so we don't regress it when the emitter changes.
#[test]
fn no_arg_and_void_returns_are_emitted() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let out = generate_registry_apis();
    let js = &out.js;
    let dts = &out.dts;
    assert!(js.contains("regCloseKey("), ".js must expose regCloseKey");
    let sig_line = dts
        .lines()
        .find(|l| l.contains("regCloseKey"))
        .expect(".d.ts must declare regCloseKey");
    // RegCloseKey has one [in] HKEY and returns LSTATUS. No out-param projection.
    assert!(
        sig_line.contains("HKEY") || sig_line.contains("hkey"),
        "regCloseKey must accept an HKEY: {sig_line}"
    );
    assert!(
        sig_line.contains("number") || sig_line.contains("void"),
        "regCloseKey must have a numeric LSTATUS or void return: {sig_line}"
    );
}

// ---------------------------------------------------------------------------
// CORNER: non-regression
// ---------------------------------------------------------------------------

/// 8a. Generating a classic-COM interface (ITaskbarList3) still works.
#[test]
fn com_interface_generation_still_works() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let com_iface =
        com_metadata::parse_com_interface(&win32_winmd(), "Windows.Win32.UI.Shell", "ITaskbarList3")
            .expect("ITaskbarList3 must exist");
    let out = com::generate_com_interface_files(&com_iface, &win32_winmd())
        .expect("COM codegen must succeed");
    assert!(out.js.contains("class ITaskbarList3"));
    assert!(out.dts.contains("ITaskbarList3"));
}

/// 8b. WinRT class generation isn't broken by the flat additions: try to
///    invoke the CLI on `Windows.Foundation.Uri` and verify it emits Uri.js
///    and Uri.d.ts. This exercises the full main.rs routing.
#[test]
fn winrt_generation_still_works() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    if com_metadata::discover_newest_windows_winmd().is_none() {
        eprintln!("Skipping: Windows SDK Windows.winmd not available (needed to generate Windows.Foundation.Uri)");
        return;
    }
    // Use a unique per-process directory under the OS temp dir to avoid
    // cross-test interference when Rust runs tests in parallel and to prevent
    // stale state from a previous interrupted run leaking in.
    let out_dir = std::env::temp_dir().join(format!(
        "dynwinrt_codegen_tmp_gen_uri_{}",
        std::process::id()
    ));
    if out_dir.exists() {
        let _ = fs::remove_dir_all(&out_dir);
    }
    fs::create_dir_all(&out_dir).unwrap();

    // Invoke the CLI via `cargo run`.
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.ancestors().nth(2).expect("workspace root");
    let status = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args([
            "generate",
            "--namespace",
            "Windows.Foundation",
            "--class-name",
            "Uri",
            "--output",
        ])
        .arg(out_dir.to_str().unwrap())
        .current_dir(workspace_root)
        .status()
        .expect("run cargo");
    assert!(status.success(), "CLI Uri generation should succeed");
    assert!(out_dir.join("Uri.js").exists(), "expected Uri.js");
    assert!(out_dir.join("Uri.d.ts").exists(), "expected Uri.d.ts");
    // Clean up.
    let _ = fs::remove_dir_all(&out_dir);
}

// ---------------------------------------------------------------------------
// FAIL-LOUD: unsupported return kinds must be skipped, not silently truncated
// (Regression for the I64/U64/F32/F64 → I32 silent-degrade bug caught in code
// review.)
// ---------------------------------------------------------------------------

use dynwinrt_codegen::meta::{FlatApisMeta, FlatMethodMeta, FlatParamMeta};

#[test]
fn flat_skips_methods_with_64bit_or_float_underlying_enum_params() {
    use dynwinrt_codegen::types::EnumMember;
    let enum_param = |ename: &str, underlying: FlatAbiType| FlatParamMeta {
        name: "flags".into(),
        abi: FlatAbiType::Enum {
            namespace: "Fake.Ns".into(),
            name: ename.into(),
            underlying: Box::new(underlying),
            members: vec![EnumMember {
                name: "A".into(),
                value: 0,
                doc: None,
            }],
        },
        direction: FlatDirection::In,
    };
    // A method whose enum param has a U64 underlying is NOT faithfully
    // representable (i32-backed members, number-typed surface) -> must be
    // skipped fail-loud. A U32-underlying enum param IS representable -> kept.
    let mut bad = synth_method("BadEnumMethod", FlatAbiType::U32);
    bad.params = vec![enum_param("BigEnum", FlatAbiType::U64)];
    let mut good = synth_method("GoodEnumMethod", FlatAbiType::U32);
    good.params = vec![enum_param("SmallEnum", FlatAbiType::U32)];

    let out = flat::generate_flat_apis_files(&synth_apis(vec![bad, good]));
    assert!(
        !out.js.contains("badEnumMethod") && !out.dts.contains("badEnumMethod"),
        "method with a 64-bit-underlying enum param must be skipped:\n{}",
        out.js
    );
    assert!(
        out.js.contains("goodEnumMethod"),
        "method with a 32-bit-underlying enum param must be emitted:\n{}",
        out.js
    );
}

fn synth_method(name: &str, ret: FlatAbiType) -> FlatMethodMeta {
    FlatMethodMeta {
        name: name.into(),
        dll: "FAKE.dll".into(),
        entry_point: name.into(),
        return_type: ret,
        params: vec![FlatParamMeta {
            name: "arg".into(),
            abi: FlatAbiType::U32,
            direction: FlatDirection::In,
        }],
        return_is_status: false,
    }
}

fn synth_apis(methods: Vec<FlatMethodMeta>) -> FlatApisMeta {
    FlatApisMeta {
        namespace: "Fake.Ns".into(),
        class_name: "Apis".into(),
        methods,
        referenced_enums: Vec::new(),
    }
}

fn synth_enum_meta(namespace: &str, name: &str, member: &str) -> TypeMeta {
    synth_enum_meta_with_value(namespace, name, member, 0)
}

fn synth_enum_meta_with_value(namespace: &str, name: &str, member: &str, value: i32) -> TypeMeta {
    use dynwinrt_codegen::types::EnumMember;

    TypeMeta::Enum {
        namespace: namespace.into(),
        name: name.into(),
        underlying: Box::new(TypeMeta::I32),
        members: vec![EnumMember {
            name: member.into(),
            value,
            doc: None,
        }],
        is_flags: false,
        doc: None,
        deprecated: None,
    }
}

fn synth_enum_abi(namespace: &str, name: &str, member: &str) -> FlatAbiType {
    synth_enum_abi_with_value(namespace, name, member, 0)
}

fn synth_enum_abi_with_value(namespace: &str, name: &str, member: &str, value: i32) -> FlatAbiType {
    FlatAbiType::Enum {
        namespace: namespace.into(),
        name: name.into(),
        underlying: Box::new(FlatAbiType::I32),
        members: vec![dynwinrt_codegen::types::EnumMember {
            name: member.into(),
            value,
            doc: None,
        }],
    }
}

/// A flat export returning I64/U64 must be emitted with an explicit 64-bit
/// retKind and decoded as BigInt, never through the truncating number path.
#[test]
fn flat_emits_i64_u64_returns_with_bigint_decoders() {
    let apis = synth_apis(vec![
        synth_method("GoodStatus", FlatAbiType::I32),
        synth_method("GetTickCount64", FlatAbiType::U64),
        synth_method("GetLargeCounter", FlatAbiType::I64),
    ]);
    let out = flat::generate_flat_apis_files(&apis);
    assert!(out.js.contains("export function goodStatus"));
    assert!(
        out.js
            .contains("flatInvoke('FAKE.dll', 'GetTickCount64', 'U64'"),
        ".js must invoke U64 returns with retKind U64:\n{}",
        out.js
    );
    assert!(
        out.js.contains("_ret.toU64BigInt()"),
        ".js must decode U64 returns with toU64BigInt():\n{}",
        out.js
    );
    assert!(
        out.js
            .contains("flatInvoke('FAKE.dll', 'GetLargeCounter', 'I64'"),
        ".js must invoke I64 returns with retKind I64:\n{}",
        out.js
    );
    assert!(
        out.js.contains("_ret.toI64BigInt()"),
        ".js must decode I64 returns with toI64BigInt():\n{}",
        out.js
    );
    assert!(
        out.dts
            .contains("getTickCount64(arg: number): { readonly result: bigint }")
            && out
                .dts
                .contains("getLargeCounter(arg: number): { readonly result: bigint }"),
        ".d.ts must declare I64/U64 returns as bigint:\n{}",
        out.dts
    );
}

/// A flat export returning F32/F64 must be emitted with explicit float
/// retKinds and decoded as JS numbers via toF64().
#[test]
fn flat_emits_float_returns_with_number_decoder() {
    let apis = synth_apis(vec![
        synth_method("Ok", FlatAbiType::I32),
        synth_method("FloatFn", FlatAbiType::F32),
        synth_method("DoubleFn", FlatAbiType::F64),
    ]);
    let out = flat::generate_flat_apis_files(&apis);
    assert!(out.js.contains("export function ok"));
    assert!(
        out.js.contains("flatInvoke('FAKE.dll', 'FloatFn', 'F32'"),
        ".js must invoke F32 returns with retKind F32:\n{}",
        out.js
    );
    assert!(
        out.js.contains("flatInvoke('FAKE.dll', 'DoubleFn', 'F64'"),
        ".js must invoke F64 returns with retKind F64:\n{}",
        out.js
    );
    assert!(
        out.js.matches("_ret.toF64()").count() >= 2,
        ".js must decode F32/F64 returns with toF64():\n{}",
        out.js
    );
    assert!(
        out.dts
            .contains("floatFn(arg: number): { readonly result: number }")
            && out
                .dts
                .contains("doubleFn(arg: number): { readonly result: number }"),
        ".d.ts must declare F32/F64 returns as number:\n{}",
        out.dts
    );
}

#[test]
fn flat_bool_return_decodes_boolean_not_number() {
    let apis = synth_apis(vec![
        synth_method("ReturnsBool", FlatAbiType::Bool),
        synth_method("ReturnsBool32", FlatAbiType::Bool32),
        synth_method("ReturnsI32", FlatAbiType::I32),
    ]);
    let out = flat::generate_flat_apis_files(&apis);
    assert!(
        out.js
            .contains("return { result: (_ret.toNumber() !== 0) };"),
        ".js must decode BOOL returns to boolean:\n{}",
        out.js
    );
    assert!(
        out.dts
            .contains("returnsBool(arg: number): { readonly result: boolean }")
            && out
                .dts
                .contains("returnsBool32(arg: number): { readonly result: boolean }"),
        ".d.ts must declare BOOL returns as boolean:\n{}",
        out.dts
    );
    assert!(
        out.js.contains("export function returnsI32")
            && out.js.contains("return { result: _ret.toNumber() };"),
        "non-bool I32 returns must remain numeric:\n{}",
        out.js
    );
}

#[test]
fn flat_bool32_out_slot_decodes_boolean_not_number() {
    let m = FlatMethodMeta {
        name: "GetFlag".into(),
        dll: "FAKE.dll".into(),
        entry_point: "GetFlag".into(),
        return_type: FlatAbiType::Void,
        params: vec![FlatParamMeta {
            name: "enabled".into(),
            abi: FlatAbiType::PtrTo(Box::new(FlatAbiType::Bool32)),
            direction: FlatDirection::Out,
        }],
        return_is_status: false,
    };
    let out = flat::generate_flat_apis_files(&synth_apis(vec![m]));
    assert!(
        out.js
            .contains("enabled: (_enabledSlot.readInt32LE(0) !== 0)"),
        ".js must decode BOOL out slots to boolean:\n{}",
        out.js
    );
    assert!(
        out.dts.contains("getFlag(): { readonly enabled: boolean }"),
        ".d.ts must declare BOOL out slots as boolean:\n{}",
        out.dts
    );
}

/// Unknown return types must be skipped instead of falling back to I32.
#[test]
fn flat_skips_unknown_return_instead_of_silently_truncating() {
    let apis = synth_apis(vec![
        synth_method("Ok", FlatAbiType::I32),
        synth_method("Mystery", FlatAbiType::Unknown),
    ]);
    let out = flat::generate_flat_apis_files(&apis);
    assert!(out.js.contains("export function ok"));
    assert!(
        !out.js.contains("mystery") && !out.dts.contains("mystery"),
        "Unknown-returning export must be skipped, not emitted with I32 fallback:\n{}\n{}",
        out.js,
        out.dts
    );
}

#[test]
fn flat_skips_bare_unknown_param_but_keeps_opaque_pointer_param() {
    let by_value_struct = FlatMethodMeta {
        name: "ByValueStruct".into(),
        dll: "FAKE.dll".into(),
        entry_point: "ByValueStruct".into(),
        return_type: FlatAbiType::I32,
        params: vec![FlatParamMeta {
            name: "value".into(),
            abi: FlatAbiType::Unknown,
            direction: FlatDirection::In,
        }],
        return_is_status: false,
    };
    let struct_pointer = FlatMethodMeta {
        name: "StructPointer".into(),
        dll: "FAKE.dll".into(),
        entry_point: "StructPointer".into(),
        return_type: FlatAbiType::I32,
        params: vec![FlatParamMeta {
            name: "buffer".into(),
            abi: FlatAbiType::PtrTo(Box::new(FlatAbiType::Unknown)),
            direction: FlatDirection::In,
        }],
        return_is_status: false,
    };

    let out = flat::generate_flat_apis_files(&synth_apis(vec![by_value_struct, struct_pointer]));
    assert!(
        !out.js.contains("byValueStruct") && !out.dts.contains("byValueStruct"),
        "bare Unknown by-value params must be skipped to avoid pointer-for-struct ABI mismatch:\n{}\n{}",
        out.js,
        out.dts
    );
    assert!(
        out.js.contains("export function structPointer(buffer)")
            && out.js.contains("DynWinRtValue.pointer(buffer)"),
        "PtrTo(Unknown) struct pointer params remain valid opaque pointer inputs:\n{}",
        out.js
    );
    assert!(
        out.dts
            .contains("structPointer(buffer: bigint | Buffer | Uint8Array | null)"),
        "PtrTo(Unknown) should stay in the typed surface as an opaque pointer:\n{}",
        out.dts
    );
}

#[test]
fn flat_emits_void_return_without_result_field() {
    let apis = synth_apis(vec![
        synth_method("NoOuts", FlatAbiType::Void),
        FlatMethodMeta {
            name: "WithOut".into(),
            dll: "FAKE.dll".into(),
            entry_point: "WithOut".into(),
            return_type: FlatAbiType::Void,
            params: vec![FlatParamMeta {
                name: "value".into(),
                abi: FlatAbiType::PtrTo(Box::new(FlatAbiType::U32)),
                direction: FlatDirection::Out,
            }],
            return_is_status: false,
        },
    ]);
    let out = flat::generate_flat_apis_files(&apis);
    assert!(
        out.js.contains("flatInvoke('FAKE.dll', 'NoOuts', 'Void'")
            && out.js.contains("return undefined;"),
        "void/no-out export must use Void retKind and return undefined:\n{}",
        out.js
    );
    assert!(
        out.js.contains("flatInvoke('FAKE.dll', 'WithOut', 'Void'")
            && out.js.contains("value: _valueSlot.readUInt32LE(0)"),
        "void/out export must omit result and project out params:\n{}",
        out.js
    );
    assert!(
        out.dts.contains("noOuts(arg: number): void")
            && out.dts.contains("withOut(): { readonly value: number }"),
        ".d.ts must model void returns without result fields:\n{}",
        out.dts
    );
}

/// Enum returns whose underlying type is I64/U64/F32/F64 must be skipped too:
/// the underlying-type widening in `flat_ret_kind_literal` would otherwise
/// silently pick the wrong return kind.
#[test]
fn flat_skips_enum_return_over_unsupported_underlying() {
    let bad_enum = FlatAbiType::Enum {
        namespace: "Fake.Ns".into(),
        name: "LargeStatus".into(),
        underlying: Box::new(FlatAbiType::U64),
        members: Vec::new(),
    };
    let apis = synth_apis(vec![
        synth_method("Ok", FlatAbiType::I32),
        synth_method("BigStatus", bad_enum),
    ]);
    let out = flat::generate_flat_apis_files(&apis);
    assert!(out.js.contains("export function ok"));
    assert!(
        !out.js.contains("bigStatus"),
        ".js must NOT include enum export whose underlying is U64:\n{}",
        out.js
    );
}

/// Float PARAMS (not returns) must be wrapped with typed `f32()`/`f64()` —
/// NOT `pointer(...)`, which would silently mis-marshal an IEEE-754 float as
/// a raw pointer. Passing a proper typed value means the wrapper fails
/// loudly at runtime (if the ABI doesn't yet accept floats) rather than
/// producing wrong values.
#[test]
fn flat_float_params_use_typed_wrappers_not_pointer() {
    let m = FlatMethodMeta {
        name: "SetLevel".into(),
        dll: "FAKE.dll".into(),
        entry_point: "SetLevel".into(),
        return_type: FlatAbiType::I32,
        params: vec![
            FlatParamMeta {
                name: "amount".into(),
                abi: FlatAbiType::F32,
                direction: FlatDirection::In,
            },
            FlatParamMeta {
                name: "precise".into(),
                abi: FlatAbiType::F64,
                direction: FlatDirection::In,
            },
        ],
        return_is_status: false,
    };
    let out = flat::generate_flat_apis_files(&synth_apis(vec![m]));
    assert!(
        out.js.contains("DynWinRtValue.f32(amount)"),
        ".js must wrap F32 param with typed f32():\n{}",
        out.js
    );
    assert!(
        out.js.contains("DynWinRtValue.f64(precise)"),
        ".js must wrap F64 param with typed f64():\n{}",
        out.js
    );
    // And crucially, must NOT be `pointer(<float>)`.
    assert!(
        !out.js.contains("DynWinRtValue.pointer(amount)"),
        ".js must NOT pointer-wrap F32 (silent mis-marshal):\n{}",
        out.js
    );
    assert!(
        !out.js.contains("DynWinRtValue.pointer(precise)"),
        ".js must NOT pointer-wrap F64 (silent mis-marshal):\n{}",
        out.js
    );
}

#[test]
fn flat_unsigned_enum_high_bit_args_cross_u32_boundary_as_unsigned() {
    let high_bit_enum = FlatAbiType::Enum {
        namespace: "Fake.Ns".into(),
        name: "UnsignedFlags".into(),
        underlying: Box::new(FlatAbiType::U32),
        members: vec![dynwinrt_codegen::types::EnumMember {
            name: "HighBit".into(),
            value: i32::MIN,
            doc: None,
        }],
    };
    let method = FlatMethodMeta {
        name: "UseFlags".into(),
        dll: "FAKE.dll".into(),
        entry_point: "UseFlags".into(),
        return_type: high_bit_enum.clone(),
        params: vec![
            FlatParamMeta {
                name: "flags".into(),
                abi: high_bit_enum.clone(),
                direction: FlatDirection::In,
            },
            FlatParamMeta {
                name: "inoutFlags".into(),
                abi: FlatAbiType::PtrTo(Box::new(high_bit_enum)),
                direction: FlatDirection::InOut,
            },
        ],
        return_is_status: false,
    };
    let apis = FlatApisMeta {
        namespace: "Fake.Ns".into(),
        class_name: "Apis".into(),
        methods: vec![method],
        referenced_enums: vec![TypeMeta::Enum {
            namespace: "Fake.Ns".into(),
            name: "UnsignedFlags".into(),
            underlying: Box::new(TypeMeta::U32),
            members: vec![dynwinrt_codegen::types::EnumMember {
                name: "HighBit".into(),
                value: i32::MIN,
                doc: None,
            }],
            is_flags: true,
            doc: None,
            deprecated: None,
        }],
    };
    let out = flat::generate_flat_apis_files(&apis);

    assert!(
        out.extra_files
            .iter()
            .any(|(name, content)| name == "UnsignedFlags.js"
                && content.contains("HighBit: -2147483648")),
        "high-bit enum constants should remain signed i32 values so === comparisons with toNumber() returns keep working: {:?}",
        out.extra_files
    );
    assert!(
        out.js.contains("DynWinRtValue.u32((flags) >>> 0)"),
        "unsigned enum input args must coerce signed high-bit constants before napi u32 conversion:\n{}",
        out.js
    );
    assert!(
        out.js
            .contains("_inoutFlagsSlot.writeUInt32LE((inoutFlags) >>> 0, 0)"),
        "unsigned enum inout args must coerce signed high-bit constants before writeUInt32LE:\n{}",
        out.js
    );
    assert!(
        out.js.contains("result: _ret.toNumber()")
            && out.js.contains("inoutFlags: (_inoutFlagsSlot.readUInt32LE(0) | 0)"),
        "unsigned enum returns/out slots should stay signed to match emitted constants:\n{}",
        out.js
    );
}

/// The `.d.ts` return type for pointer-like return kinds MUST match what
/// `.js` actually produces at runtime. Any `retKind === "Ptr"` (see
/// `flat_ret_kind_literal` — `Ptr`, `PtrTo(_)`, `PWStr`, `PStr`,
/// `Handle{..}`) is unconditionally converted through
/// `_ret.asPointerBigint()`, which returns a plain `bigint` (`0n` for
/// null). Typing the `.d.ts` `result` as `bigint | Buffer | null` /
/// `string | null` / a HANDLE alias (as `dts_type_of` does for input
/// params) would misdescribe the runtime and force callers into
/// wrong-branch narrowing (checking for `Buffer`/`null` values that
/// never appear).
#[test]
fn flat_dts_return_types_match_js_runtime() {
    // Cover every pointer-like return kind that `flat_ret_kind_literal`
    // routes to "Ptr". All should surface as `bigint` in the .d.ts.
    let apis = synth_apis(vec![
        synth_method("ReturnsRawPtr", FlatAbiType::Ptr),
        synth_method(
            "ReturnsPtrToU32",
            FlatAbiType::PtrTo(Box::new(FlatAbiType::U32)),
        ),
        synth_method("ReturnsPWStr", FlatAbiType::PWStr),
        synth_method("ReturnsPStr", FlatAbiType::PStr),
        synth_method(
            "ReturnsHandle",
            FlatAbiType::Handle {
                namespace: "Windows.Win32.Foundation".into(),
                name: "HWND".into(),
            },
        ),
        // Non-pointer sanity check: I32 must still project as `number`.
        synth_method("ReturnsI32", FlatAbiType::I32),
    ]);
    let out = flat::generate_flat_apis_files(&apis);
    // Pointer-family returns all show `result: bigint`.
    for camel in &[
        "returnsRawPtr",
        "returnsPtrToU32",
        "returnsPWStr",
        "returnsPStr",
        "returnsHandle",
    ] {
        let needle = format!("function {camel}(");
        let idx = out
            .dts
            .find(&needle)
            .unwrap_or_else(|| panic!(".d.ts missing declaration for {camel}:\n{}", out.dts));
        let sig = &out.dts[idx..];
        let end = sig.find(';').unwrap_or(sig.len());
        let sig = &sig[..end];
        assert!(
            sig.contains("readonly result: bigint"),
            ".d.ts for {camel} must type result as bigint (matches asPointerBigint at runtime), got: {sig}",
        );
        assert!(
            !sig.contains("Buffer") && !sig.contains("string"),
            ".d.ts for {camel} must NOT surface Buffer/string return (input-only shape), got: {sig}",
        );
    }
    // Non-pointer sanity check.
    assert!(
        out.dts
            .contains("function returnsI32(arg: number): { readonly result: number }"),
        ".d.ts for returnsI32 must project result as number:\n{}",
        out.dts
    );
    // And the same signals in the .js confirm the contract we're describing.
    assert!(
        out.js.contains("asPointerBigint()"),
        ".js must convert pointer returns via asPointerBigint():\n{}",
        out.js
    );
}

#[test]
fn flat_filters_referenced_enums_to_kept_methods_only() {
    let kept_enum = synth_enum_abi("Fake.Kept", "KeptStatus", "Ok");
    let skipped_a = synth_enum_abi("Fake.SkippedA", "Status", "A");
    let skipped_b = synth_enum_abi("Fake.SkippedB", "Status", "B");
    let kept = FlatMethodMeta {
        name: "Kept".into(),
        dll: "FAKE.dll".into(),
        entry_point: "Kept".into(),
        return_type: FlatAbiType::I32,
        params: vec![FlatParamMeta {
            name: "status".into(),
            abi: kept_enum,
            direction: FlatDirection::In,
        }],
        return_is_status: false,
    };
    let skipped_one = FlatMethodMeta {
        name: "SkippedOne".into(),
        dll: "FAKE.dll".into(),
        entry_point: "SkippedOne".into(),
        return_type: FlatAbiType::Unknown,
        params: vec![FlatParamMeta {
            name: "status".into(),
            abi: skipped_a,
            direction: FlatDirection::In,
        }],
        return_is_status: false,
    };
    let skipped_two = FlatMethodMeta {
        name: "SkippedTwo".into(),
        dll: "FAKE.dll".into(),
        entry_point: "SkippedTwo".into(),
        return_type: FlatAbiType::Unknown,
        params: vec![FlatParamMeta {
            name: "status".into(),
            abi: skipped_b,
            direction: FlatDirection::In,
        }],
        return_is_status: false,
    };
    let apis = FlatApisMeta {
        namespace: "Fake.Ns".into(),
        class_name: "Apis".into(),
        methods: vec![kept, skipped_one, skipped_two],
        referenced_enums: vec![
            synth_enum_meta("Fake.Kept", "KeptStatus", "Ok"),
            synth_enum_meta("Fake.SkippedA", "Status", "A"),
            synth_enum_meta("Fake.SkippedB", "Status", "B"),
        ],
    };

    let out = std::panic::catch_unwind(|| flat::generate_flat_apis_files(&apis))
        .expect("skipped-only enum simple-name collisions must not abort generation");
    let extra_names: Vec<&str> = out.extra_files.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        extra_names,
        vec!["KeptStatus.d.ts", "KeptStatus.js"],
        "only enums referenced by emitted methods should produce sibling files"
    );
    assert!(out.js.contains("export function kept"));
    assert!(!out.js.contains("skippedOne") && !out.js.contains("skippedTwo"));
}

/// The CLI must fail loud when `--lang py` (or any non-`js` language) is
/// combined with a `--class-name` that resolves to a flat-Win32 `[DllImport]`
/// module — those emitters produce only `.js` + `.d.ts` and would otherwise
/// silently write the wrong artifact types into the output directory.
#[test]
fn cli_rejects_non_js_lang_for_flat_apis() {
    if !win32_available() {
        eprintln!("Skipping: Win32 winmd not available");
        return;
    }
    let out_dir = std::env::temp_dir().join(format!(
        "dynwinrt_codegen_reject_flat_py_{}",
        std::process::id()
    ));
    if out_dir.exists() {
        let _ = fs::remove_dir_all(&out_dir);
    }
    fs::create_dir_all(&out_dir).unwrap();

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.ancestors().nth(2).expect("workspace root");
    let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args([
            "generate",
            "--winmd",
            &win32_winmd(),
            "--namespace",
            REGISTRY_NS,
            "--class-name",
            "Apis",
            "--lang",
            "py",
            "--output",
        ])
        .arg(out_dir.to_str().unwrap())
        .current_dir(workspace_root)
        .output()
        .expect("run cargo");

    assert!(
        !output.status.success(),
        "CLI must reject --lang py for a flat-Apis class (got success)"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", String::from_utf8_lossy(&output.stdout), stderr);
    assert!(
        combined.contains("--lang py")
            && (combined.contains("flat-Win32") || combined.contains("[DllImport]")),
        "error must explain the flat-Win32 language mismatch. output was:\n{}",
        combined
    );
    // And no artifacts should have been written.
    assert!(
        !out_dir.join("Apis.js").exists(),
        "no .js should be written when the CLI rejects the invocation"
    );
    let _ = fs::remove_dir_all(&out_dir);
}

/// `parse_flat_apis_from_index` deduplicates referenced enums by
/// `(namespace, name)`, not `name` alone, so an `Apis` class that
/// references two enums that happen to share a simple name across
/// distinct namespaces keeps both entries. The emitter then fails
/// loud with a clear panic instead of silently emitting a
/// wrong-shape sibling file (only one variant would survive because
/// enum-file names use the simple name).
#[test]
#[should_panic(expected = "multiple distinct enums named `Status`")]
fn flat_fails_loud_on_simple_name_enum_collision() {
    // Two distinct enums with the same simple name from different
    // namespaces. Both must reach codegen (post-dedup) because the
    // `(namespace, name)` key differs.
    let apis = FlatApisMeta {
        namespace: "Fake.Ns".into(),
        class_name: "Apis".into(),
        methods: vec![FlatMethodMeta {
            name: "Noop".into(),
            dll: "FAKE.dll".into(),
            entry_point: "Noop".into(),
            return_type: FlatAbiType::I32,
            params: vec![
                FlatParamMeta {
                    name: "a".into(),
                    abi: synth_enum_abi("Fake.NsA", "Status", "AVariant"),
                    direction: FlatDirection::In,
                },
                FlatParamMeta {
                    name: "b".into(),
                    abi: synth_enum_abi("Fake.NsB", "Status", "BVariant"),
                    direction: FlatDirection::In,
                },
            ],
            return_is_status: false,
        }],
        referenced_enums: vec![
            synth_enum_meta("Fake.NsA", "Status", "AVariant"),
            synth_enum_meta("Fake.NsB", "Status", "BVariant"),
        ],
    };
    // Should panic before returning FlatGeneratedOutput.
    let _ = flat::generate_flat_apis_files(&apis);
}
