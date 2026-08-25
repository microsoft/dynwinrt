// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! E2E snapshot test: generate TypeScript for Windows.Foundation.Uri and compare
//! against committed snapshots.
//!
//! To update snapshots after an intentional output change, run:
//!   cargo run -p dynwinrt-codegen -- generate --namespace Windows.Foundation --class-name Uri --output tests/snapshots/uri

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use dynwinrt_codegen::codegen::common::to_snake_case_filename;
use dynwinrt_codegen::codegen::python_stub;
use dynwinrt_codegen::codegen::{project, render_dts, render_js};
use dynwinrt_codegen::meta;
use dynwinrt_codegen::types::TypeMeta;

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

/// Generate TypeScript for Uri and compare every file against the snapshot.
#[test]
fn snapshot_uri_class() {
    let winmd = WINDOWS_WINMD;
    let classes = match meta::parse_class(winmd, "Windows.Foundation", "Uri") {
        Some(c) => vec![c],
        None => {
            eprintln!("Skipping snapshot test: Windows.winmd not found");
            return;
        }
    };

    let deps = meta::resolve_dependencies(winmd, &classes, &[], &[]);
    let mut all_classes = classes;
    all_classes.extend(deps.classes);
    let all_interfaces = deps.interfaces;
    let all_enums = deps.enums;

    let mut known_types: HashSet<String> = HashSet::new();
    for c in &all_classes {
        known_types.insert(c.name.clone());
    }
    for i in &all_interfaces {
        known_types.insert(i.name.clone());
    }
    for e in &all_enums {
        if let TypeMeta::Enum { name, .. } = e {
            known_types.insert(name.clone());
        }
    }

    let delegate_type_names: HashSet<String> = all_interfaces
        .iter()
        .filter(|i| {
            i.methods.iter().any(|m| m.name == ".ctor")
                && i.methods.iter().any(|m| m.name == "Invoke")
        })
        .map(|i| i.name.clone())
        .collect();

    let shared_iids: HashSet<String> = HashSet::new();
    let (delegate_sigs, delegate_sig_refs, delegate_param_wraps) =
        project::build_delegate_signatures(&all_interfaces, &delegate_type_names, &known_types);

    // Generate all files into a map (.js + .d.ts pair per type)
    let mut generated: HashMap<String, String> = HashMap::new();
    for iface in &all_interfaces {
        let projected = project::project_interface(
            iface,
            &known_types,
            &delegate_type_names,
            &delegate_sigs,
            &delegate_sig_refs,
            &delegate_param_wraps,
        );
        let js = render_js::render(&projected);
        let dts = render_dts::render(&projected);
        generated.insert(format!("{}.js", iface.name), js);
        generated.insert(format!("{}.d.ts", iface.name), dts);
    }
    for class in &all_classes {
        let projected = project::project_class(
            class,
            &known_types,
            &delegate_type_names,
            &shared_iids,
            &delegate_sigs,
            &delegate_sig_refs,
            &delegate_param_wraps,
        );
        let js = render_js::render(&projected);
        let dts = render_dts::render(&projected);
        generated.insert(format!("{}.js", class.name), js);
        generated.insert(format!("{}.d.ts", class.name), dts);
    }
    let uri_js = generated.get("Uri.js").expect("generated Uri.js");
    assert!(uri_js.contains(
        "const IID_ARG_Windows_Foundation_Uri = WinGuid.parse('9e365e57-48b2-4160-956f-c7385120bbfc');"
    ));
    assert!(uri_js.contains("_unwrap(pUri).cast(IID_ARG_Windows_Foundation_Uri)"));
    assert!(
        !uri_js.contains("_unwrap(pUri).cast(DynWinRtType.runtimeClass('Windows.Foundation.Uri'")
    );

    // Compare against snapshots
    let snapshot_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/uri");
    assert!(
        snapshot_dir.exists(),
        "Snapshot directory not found: {}",
        snapshot_dir.display()
    );

    let mut mismatches = Vec::new();
    for (filename, actual) in &generated {
        let snapshot_path = snapshot_dir.join(filename);
        if !snapshot_path.exists() {
            mismatches.push(format!("  missing snapshot: {}", filename));
            continue;
        }
        let expected = fs::read_to_string(&snapshot_path).unwrap_or_else(|e| {
            panic!("Failed to read snapshot {}: {}", snapshot_path.display(), e)
        });
        if *actual != expected {
            mismatches.push(format!("  differs: {}", filename));
        }
    }

    // Check for extra snapshot files not in generated output
    if let Ok(entries) = fs::read_dir(&snapshot_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if (name.ends_with(".js") || name.ends_with(".d.ts")) && !generated.contains_key(&name)
            {
                mismatches.push(format!("  extra snapshot not generated: {}", name));
            }
        }
    }

    if !mismatches.is_empty() {
        panic!(
            "Snapshot mismatch for Windows.Foundation.Uri!\n{}\n\n\
             To update snapshots, run:\n  \
             cargo run -p dynwinrt-codegen -- generate --namespace Windows.Foundation --class-name Uri --output tests/snapshots/uri",
            mismatches.join("\n")
        );
    }
}

/// Generate .pyi stubs for Uri and compare against committed snapshots.
#[test]
fn snapshot_uri_pyi_class() {
    let winmd = WINDOWS_WINMD;
    let classes = match meta::parse_class(winmd, "Windows.Foundation", "Uri") {
        Some(c) => vec![c],
        None => {
            eprintln!("Skipping snapshot test: Windows.winmd not found");
            return;
        }
    };

    let deps = meta::resolve_python_dependencies(winmd, &classes, &[], &[]);
    let mut all_classes = classes;
    all_classes.extend(deps.classes);
    let all_interfaces = deps.interfaces;
    let all_enums = deps.enums;

    let mut known_types: HashSet<String> = HashSet::new();
    for c in &all_classes {
        known_types.insert(c.name.clone());
    }
    for i in &all_interfaces {
        known_types.insert(i.name.clone());
    }
    for e in &all_enums {
        if let TypeMeta::Enum { name, .. } = e {
            known_types.insert(name.clone());
        }
    }
    let delegate_type_names: HashSet<String> = all_interfaces
        .iter()
        .filter(|i| {
            i.methods.iter().any(|m| m.name == ".ctor")
                && i.methods.iter().any(|m| m.name == "Invoke")
        })
        .map(|i| i.name.clone())
        .collect();
    let shared_iids: HashSet<String> = HashSet::new();

    let mut generated: HashMap<String, String> = HashMap::new();
    for iface in &all_interfaces {
        let code = python_stub::generate_interface_stub(iface, &known_types, &delegate_type_names);
        generated.insert(format!("{}.pyi", to_snake_case_filename(&iface.name)), code);
    }
    for class in &all_classes {
        let code = python_stub::generate_class_stub(
            class,
            &known_types,
            &delegate_type_names,
            &shared_iids,
        );
        generated.insert(format!("{}.pyi", to_snake_case_filename(&class.name)), code);
    }
    let index = python_stub::generate_index_stub(&all_classes, &all_interfaces, &all_enums);
    generated.insert("__init__.pyi".to_string(), index);

    let snapshot_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/uri_pyi");
    assert!(
        snapshot_dir.exists(),
        "Snapshot directory not found: {}",
        snapshot_dir.display()
    );
    if std::env::var_os("DYNWINRT_UPDATE_PY_SNAPSHOTS").is_some() {
        for (filename, actual) in &generated {
            fs::write(snapshot_dir.join(filename), actual).expect("write Python stub snapshot");
        }
    }

    let mut mismatches = Vec::new();
    for (filename, actual) in &generated {
        let snapshot_path = snapshot_dir.join(filename);
        if !snapshot_path.exists() {
            mismatches.push(format!("  missing snapshot: {}", filename));
            continue;
        }
        let expected = fs::read_to_string(&snapshot_path).unwrap();
        if *actual != expected {
            mismatches.push(format!("  differs: {}", filename));
        }
    }
    if let Ok(entries) = fs::read_dir(&snapshot_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".pyi") && !generated.contains_key(&name) {
                mismatches.push(format!("  extra snapshot not generated: {}", name));
            }
        }
    }
    if !mismatches.is_empty() {
        panic!("Snapshot mismatch for Uri .pyi!\n{}", mismatches.join("\n"));
    }
}

/// Generate .py files for Uri and compare against committed snapshots.
/// Guards Phase 3 refactor (common.rs split + Lang trait) from drifting output.
#[test]
fn snapshot_uri_py_class() {
    use dynwinrt_codegen::codegen::python;
    let winmd = WINDOWS_WINMD;
    let classes = match meta::parse_class(winmd, "Windows.Foundation", "Uri") {
        Some(c) => vec![c],
        None => {
            eprintln!("Skipping snapshot test: Windows.winmd not found");
            return;
        }
    };
    let deps = meta::resolve_python_dependencies(winmd, &classes, &[], &[]);
    let mut all_classes = classes;
    all_classes.extend(deps.classes);
    let all_interfaces = deps.interfaces;
    let all_enums = deps.enums;

    let mut known_types: HashSet<String> = HashSet::new();
    for c in &all_classes {
        known_types.insert(c.name.clone());
    }
    for i in &all_interfaces {
        known_types.insert(i.name.clone());
    }
    for e in &all_enums {
        if let TypeMeta::Enum { name, .. } = e {
            known_types.insert(name.clone());
        }
    }
    let delegate_type_names: HashSet<String> = all_interfaces
        .iter()
        .filter(|i| {
            i.methods.iter().any(|m| m.name == ".ctor")
                && i.methods.iter().any(|m| m.name == "Invoke")
        })
        .map(|i| i.name.clone())
        .collect();
    let shared_iids: HashSet<String> = HashSet::new();

    let mut generated: HashMap<String, String> = HashMap::new();
    for iface in &all_interfaces {
        let code = python::generate_interface(iface, &known_types, &delegate_type_names);
        generated.insert(format!("{}.py", to_snake_case_filename(&iface.name)), code);
    }
    for class in &all_classes {
        let code = python::generate_class(class, &known_types, &delegate_type_names, &shared_iids);
        generated.insert(format!("{}.py", to_snake_case_filename(&class.name)), code);
    }
    let index = python::generate_index(&all_classes, &all_interfaces, &all_enums);
    generated.insert("__init__.py".to_string(), index);

    let snapshot_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/uri_py");
    assert!(
        snapshot_dir.exists(),
        "Snapshot directory not found: {}",
        snapshot_dir.display()
    );
    if std::env::var_os("DYNWINRT_UPDATE_PY_SNAPSHOTS").is_some() {
        for (filename, actual) in &generated {
            fs::write(snapshot_dir.join(filename), actual).expect("write Python snapshot");
        }
    }

    let mut mismatches = Vec::new();
    for (filename, actual) in &generated {
        let snapshot_path = snapshot_dir.join(filename);
        if !snapshot_path.exists() {
            mismatches.push(format!("  missing snapshot: {}", filename));
            continue;
        }
        let expected = fs::read_to_string(&snapshot_path).unwrap();
        if *actual != expected {
            mismatches.push(format!("  differs: {}", filename));
        }
    }
    if let Ok(entries) = fs::read_dir(&snapshot_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".py") && !generated.contains_key(&name) {
                mismatches.push(format!("  extra snapshot not generated: {}", name));
            }
        }
    }
    if !mismatches.is_empty() {
        panic!(
            "Snapshot mismatch for Uri .py!\n{}\n\nTo update: cargo run -p dynwinrt-codegen -- generate --namespace Windows.Foundation --class-name Uri --lang py --output tools/dynwinrt-codegen/tests/snapshots/uri_py",
            mismatches.join("\n")
        );
    }
}

/// Snapshot a method-rich Python class outside Windows.Foundation.
#[test]
fn snapshot_data_writer_py_class() {
    use dynwinrt_codegen::codegen::python;

    let classes = match meta::parse_class(WINDOWS_WINMD, "Windows.Storage.Streams", "DataWriter") {
        Some(class) => vec![class],
        None => {
            eprintln!("Skipping snapshot test: Windows.winmd not found");
            return;
        }
    };
    let deps = meta::resolve_python_dependencies(WINDOWS_WINMD, &classes, &[], &[]);
    let mut all_classes = classes;
    all_classes.extend(deps.classes);
    let interfaces = deps.interfaces;
    let enums = deps.enums;

    let mut known_types = HashSet::new();
    known_types.extend(all_classes.iter().map(|class| class.name.clone()));
    known_types.extend(interfaces.iter().map(|interface| interface.name.clone()));
    known_types.extend(enums.iter().filter_map(|typ| match typ {
        TypeMeta::Enum { name, .. } => Some(name.clone()),
        _ => None,
    }));
    let delegate_type_names = interfaces
        .iter()
        .filter(|interface| {
            interface
                .methods
                .iter()
                .any(|method| method.name == ".ctor")
                && interface
                    .methods
                    .iter()
                    .any(|method| method.name == "Invoke")
        })
        .map(|interface| interface.name.clone())
        .collect::<HashSet<_>>();
    let class = all_classes
        .iter()
        .find(|class| class.name == "DataWriter")
        .expect("DataWriter class");
    let actual = python::generate_class(class, &known_types, &delegate_type_names, &HashSet::new());

    let snapshot_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/data_writer_py");
    let snapshot_path = snapshot_dir.join("data_writer.py");
    if std::env::var_os("DYNWINRT_UPDATE_PY_SNAPSHOTS").is_some() {
        fs::create_dir_all(&snapshot_dir).expect("create DataWriter snapshot directory");
        fs::write(&snapshot_path, &actual).expect("write DataWriter Python snapshot");
    }
    let expected = fs::read_to_string(&snapshot_path).unwrap_or_else(|error| {
        panic!(
            "Failed to read snapshot {}: {error}. Set DYNWINRT_UPDATE_PY_SNAPSHOTS=1 to create it.",
            snapshot_path.display()
        )
    });
    assert_eq!(actual, expected, "DataWriter Python snapshot changed");
}

/// Verify generated TypeScript for async (and async-with-progress) methods
/// includes AbortSignal scaffolding: the `signal?: AbortSignal` parameter,
/// `_op.cancel()` on abort, and `signal.reason` rethrow.
///
/// Targets `Windows.Storage.Streams.DataWriter` (`StoreAsync`/`FlushAsync` =
/// IAsyncOperation<UInt32>/<Boolean>) and `Windows.Web.Http.HttpClient`
/// (`GetStringAsync` etc. = IAsyncOperationWithProgress).
#[test]
fn ts_async_methods_emit_abort_signal_scaffolding() {
    let winmd = WINDOWS_WINMD;

    // ---- DataWriter: plain IAsyncOperation<T> ------------------------------
    let dw_classes = match meta::parse_class(winmd, "Windows.Storage.Streams", "DataWriter") {
        Some(c) => vec![c],
        None => {
            eprintln!("Skipping: Windows.winmd not found");
            return;
        }
    };
    let dw_deps = meta::resolve_dependencies(winmd, &dw_classes, &[], &[]);
    let mut dw_all_classes = dw_classes;
    dw_all_classes.extend(dw_deps.classes);
    let dw_ifaces = dw_deps.interfaces;
    let dw_enums = dw_deps.enums;

    let mut known: HashSet<String> = HashSet::new();
    for c in &dw_all_classes {
        known.insert(c.name.clone());
    }
    for i in &dw_ifaces {
        known.insert(i.name.clone());
    }
    for e in &dw_enums {
        if let TypeMeta::Enum { name, .. } = e {
            known.insert(name.clone());
        }
    }
    let delegates: HashSet<String> = dw_ifaces
        .iter()
        .filter(|i| {
            i.methods.iter().any(|m| m.name == ".ctor")
                && i.methods.iter().any(|m| m.name == "Invoke")
        })
        .map(|i| i.name.clone())
        .collect();
    let shared: HashSet<String> = HashSet::new();
    let (dw_delegate_sigs, dw_delegate_sig_refs, dw_delegate_param_wraps) =
        project::build_delegate_signatures(&dw_ifaces, &delegates, &known);

    let dw_class = dw_all_classes
        .iter()
        .find(|c| c.name == "DataWriter")
        .expect("DataWriter class");
    let dw_projected = project::project_class(
        dw_class,
        &known,
        &delegates,
        &shared,
        &dw_delegate_sigs,
        &dw_delegate_sig_refs,
        &dw_delegate_param_wraps,
    );
    let dw_code = render_js::render(&dw_projected);

    // The generated method must accept signal and wire it to cancel().
    assert!(
        dw_code.contains("storeAsync(signal)"),
        "Expected `storeAsync(signal)` in DataWriter.js, got:\n{}",
        dw_code
    );
    assert!(
        dw_code.contains("if (signal?.aborted) throw signal.reason;"),
        "Expected fast-path `if (signal?.aborted) throw signal.reason;` in DataWriter.js"
    );
    assert!(
        dw_code.contains("_op.cancel()"),
        "Expected `_op.cancel()` invocation in DataWriter.js"
    );
    assert!(
        dw_code.contains("addEventListener('abort'"),
        "Expected `addEventListener('abort'` listener registration in DataWriter.js"
    );
    assert!(
        dw_code.contains("removeEventListener('abort'"),
        "Expected `removeEventListener('abort'` cleanup in DataWriter.js"
    );

    // ---- HttpClient: IAsyncOperationWithProgress<T,P> ----------------------
    let hc_classes = match meta::parse_class(winmd, "Windows.Web.Http", "HttpClient") {
        Some(c) => vec![c],
        None => {
            eprintln!("Skipping HttpClient portion: not found");
            return;
        }
    };
    let hc_deps = meta::resolve_dependencies(winmd, &hc_classes, &[], &[]);
    let mut hc_all_classes = hc_classes;
    hc_all_classes.extend(hc_deps.classes);
    let hc_ifaces = hc_deps.interfaces;
    let hc_enums = hc_deps.enums;

    let mut hc_known: HashSet<String> = HashSet::new();
    for c in &hc_all_classes {
        hc_known.insert(c.name.clone());
    }
    for i in &hc_ifaces {
        hc_known.insert(i.name.clone());
    }
    for e in &hc_enums {
        if let TypeMeta::Enum { name, .. } = e {
            hc_known.insert(name.clone());
        }
    }
    let hc_delegates: HashSet<String> = hc_ifaces
        .iter()
        .filter(|i| {
            i.methods.iter().any(|m| m.name == ".ctor")
                && i.methods.iter().any(|m| m.name == "Invoke")
        })
        .map(|i| i.name.clone())
        .collect();
    let (hc_delegate_sigs, hc_delegate_sig_refs, hc_delegate_param_wraps) =
        project::build_delegate_signatures(&hc_ifaces, &hc_delegates, &hc_known);

    let hc_class = hc_all_classes
        .iter()
        .find(|c| c.name == "HttpClient")
        .expect("HttpClient class");
    let hc_projected = project::project_class(
        hc_class,
        &hc_known,
        &hc_delegates,
        &shared,
        &hc_delegate_sigs,
        &hc_delegate_sig_refs,
        &hc_delegate_param_wraps,
    );
    let hc_code = render_js::render(&hc_projected);

    // WithProgress methods must accept signal AND expose cancel() on the wrapper.
    assert!(
        hc_code.contains("getStringAsync(uri, signal)"),
        "Expected `getStringAsync(uri, signal)` in HttpClient.js"
    );
    assert!(
        hc_code.contains("cancel()"),
        "Expected `cancel()` in WithProgress return type in HttpClient.js"
    );
    assert!(hc_code.contains("cancel() { try { _op.cancel(); } catch (_ce) { /* cancel after completion is a no-op per WinRT spec */ } }"),
        "Expected `cancel()` impl on returned WithProgress wrapper in HttpClient.js");
    assert!(
        hc_code.contains("if (signal?.aborted) throw signal.reason;"),
        "Expected `signal.reason` rethrow in HttpClient.js WithProgress wrapper"
    );

    // WithProgress must also fast-fail BEFORE invoking the op when signal is
    // already aborted, returning a shaped rejected wrapper. Without this guard
    // the underlying op runs and may resolve successfully (cooperative cancel),
    // breaking the AbortSignal contract.
    assert!(
        hc_code.contains("if (signal?.aborted) {"),
        "Expected pre-invoke fast-fail `if (signal?.aborted) {{` block in HttpClient.js WithProgress methods"
    );
    assert!(
        hc_code.contains("Promise.reject(signal.reason)"),
        "Expected `Promise.reject(signal.reason)` in HttpClient.js WithProgress fast-fail"
    );
    assert!(
        hc_code.contains("progress(_cb) { return this; }"),
        "Expected shaped fast-fail wrapper to expose `progress(_cb)` in HttpClient.js"
    );

    // ---- ESM extension rule: every relative import/export ends with .js ----
    for line in hc_code.lines().chain(dw_code.lines()) {
        let trimmed = line.trim_start();
        if !(trimmed.starts_with("import ") || trimmed.starts_with("export ")) {
            continue;
        }
        if !line.contains(" from './") {
            continue;
        }
        assert!(
            line.contains(".js';"),
            "Relative import/export missing `.js` extension:\n  {}",
            line
        );
    }

    // ---- Unwrap helper rule
    for code in [&hc_code, &dw_code] {
        assert!(
            !code.contains(" as any)._obj ?? "),
            "Found legacy `(x as any)._obj ?? x` pattern (should be `_unwrap(x)`)"
        );
    }
    assert!(
        hc_code.contains("_unwrap("),
        "Expected `_unwrap(` calls in HttpClient.js"
    );
    assert!(
        hc_code.contains("const _unwrap = (x) => x?._obj ?? x;"),
        "Expected `_unwrap` helper declaration in HttpClient.js"
    );

    // ---- Collection helpers: IVector/IVectorView/IIterator/IIterable get
    // JS-idiomatic methods so `for..of`, spread, Array.from, and `.at()` work.
    // HttpClient transitively pulls in IVector_String, IIterable_Certificate,
    // and IIterator_Certificate via TLS/cookie deps.
    if let Some(iv) = hc_ifaces.iter().find(|i| i.name == "IVector_String") {
        let projected = project::project_interface(
            iv,
            &hc_known,
            &hc_delegates,
            &hc_delegate_sigs,
            &hc_delegate_sig_refs,
            &hc_delegate_param_wraps,
        );
        let code = render_js::render(&projected);
        assert!(
            code.contains("*[Symbol.iterator]()"),
            "Expected `*[Symbol.iterator]()` in IVector_String.js"
        );
        assert!(
            code.contains("get length() { return this.size; }"),
            "Expected `length` alias in IVector_String.js"
        );
        assert!(
            code.contains("at(index)"),
            "Expected `at(index)` in IVector_String.js"
        );
        assert!(
            code.contains("toArray()"),
            "Expected `toArray()` in IVector_String.js"
        );
    } else {
        panic!("IVector_String not in HttpClient deps — sample changed?");
    }

    if let Some(iv) = hc_ifaces
        .iter()
        .find(|i| i.name.starts_with("IVectorView_"))
    {
        let projected = project::project_interface(
            iv,
            &hc_known,
            &hc_delegates,
            &hc_delegate_sigs,
            &hc_delegate_sig_refs,
            &hc_delegate_param_wraps,
        );
        let code = render_js::render(&projected);
        assert!(
            code.contains("*[Symbol.iterator]()"),
            "Expected `*[Symbol.iterator]()` in {}.js",
            iv.name
        );
        assert!(
            code.contains("at(index)"),
            "Expected `at(index)` in {}.js",
            iv.name
        );
    }

    if let Some(it) = hc_ifaces.iter().find(|i| i.name.starts_with("IIterator_")) {
        let projected = project::project_interface(
            it,
            &hc_known,
            &hc_delegates,
            &hc_delegate_sigs,
            &hc_delegate_sig_refs,
            &hc_delegate_param_wraps,
        );
        let code = render_js::render(&projected);
        assert!(
            code.contains("next()"),
            "Expected JS iterator `next()` in {}.js",
            it.name
        );
        assert!(
            code.contains("[Symbol.iterator]()"),
            "Expected `[Symbol.iterator]()` in {}.js (returns this)",
            it.name
        );
    }

    if let Some(it) = hc_ifaces.iter().find(|i| i.name.starts_with("IIterable_")) {
        let projected = project::project_interface(
            it,
            &hc_known,
            &hc_delegates,
            &hc_delegate_sigs,
            &hc_delegate_sig_refs,
            &hc_delegate_param_wraps,
        );
        let code = render_js::render(&projected);
        assert!(
            code.contains("[Symbol.iterator]()"),
            "Expected `[Symbol.iterator]()` delegating to first() in {}.js",
            it.name
        );
        assert!(
            code.contains("this.first()"),
            "Expected `this.first()` delegate in {}.js",
            it.name
        );
    }

    // ---- IStringable auto-generation: classes implementing IStringable get
    // toString / Symbol.toPrimitive / Symbol.toStringTag automatically so
    // template literals and console.log behave like a string. Verify on Uri,
    // a well-known IStringable implementor.
    let uri_classes = match meta::parse_class(winmd, "Windows.Foundation", "Uri") {
        Some(c) => vec![c],
        None => {
            eprintln!("Skipping Uri portion: not found");
            return;
        }
    };
    let uri_deps = meta::resolve_dependencies(winmd, &uri_classes, &[], &[]);
    let mut uri_all_classes = uri_classes;
    uri_all_classes.extend(uri_deps.classes);
    let mut uri_known: HashSet<String> = HashSet::new();
    for c in &uri_all_classes {
        uri_known.insert(c.name.clone());
    }
    for i in &uri_deps.interfaces {
        uri_known.insert(i.name.clone());
    }
    let uri_class = uri_all_classes
        .iter()
        .find(|c| c.name == "Uri")
        .expect("Uri class");
    let uri_projected = project::project_class(
        uri_class,
        &uri_known,
        &HashSet::new(),
        &shared,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let uri_code = render_js::render(&uri_projected);
    assert!(
        uri_code.contains("toString() {\n        return IStringable.from(this._obj).toString();"),
        "Expected auto-generated toString() in Uri.js (IStringable path)"
    );
    assert!(
        uri_code.contains("[Symbol.toPrimitive](_hint)"),
        "Expected [Symbol.toPrimitive] in Uri.js"
    );
    assert!(
        uri_code.contains("get [Symbol.toStringTag]() { return 'Uri'; }"),
        "Expected [Symbol.toStringTag] = 'Uri' in Uri.js"
    );
}

#[test]
fn data_package_view_projects_text_async_overloads_and_hstring_results() {
    let classes = match meta::parse_class(
        WINDOWS_WINMD,
        "Windows.ApplicationModel.DataTransfer",
        "DataPackageView",
    ) {
        Some(class) => vec![class],
        None => {
            eprintln!("Skipping: Windows.winmd not found");
            return;
        }
    };
    let deps = meta::resolve_dependencies(WINDOWS_WINMD, &classes, &[], &[]);
    let mut all_classes = classes;
    all_classes.extend(deps.classes);
    let interfaces = deps.interfaces;
    let enums = deps.enums;
    let mut known = HashSet::new();
    for class in &all_classes {
        known.insert(class.name.clone());
    }
    for interface in &interfaces {
        known.insert(interface.name.clone());
    }
    for enum_type in &enums {
        if let TypeMeta::Enum { name, .. } = enum_type {
            known.insert(name.clone());
        }
    }
    let delegates: HashSet<String> = interfaces
        .iter()
        .filter(|interface| {
            interface
                .methods
                .iter()
                .any(|method| method.name == ".ctor")
                && interface
                    .methods
                    .iter()
                    .any(|method| method.name == "Invoke")
        })
        .map(|interface| interface.name.clone())
        .collect();
    let shared = HashSet::new();
    let (delegate_sigs, delegate_sig_refs, delegate_param_wraps) =
        project::build_delegate_signatures(&interfaces, &delegates, &known);
    let class = all_classes
        .iter()
        .find(|class| class.name == "DataPackageView")
        .expect("DataPackageView class");
    let projected = project::project_class(
        class,
        &known,
        &delegates,
        &shared,
        &delegate_sigs,
        &delegate_sig_refs,
        &delegate_param_wraps,
    );
    let code = render_js::render(&projected);
    let declarations = render_dts::render(&projected);

    assert!(code.contains(
        ".addMethod(\"GetTextAsync\", new DynWinRtMethodSig().addOut(DynWinRtType.iAsyncOperation(DynWinRtType.hstring())))"
    ));
    assert!(code.contains(
        ".addMethod(\"GetCustomTextAsync\", new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.iAsyncOperation(DynWinRtType.hstring())))"
    ));
    assert!(code.contains("async _getTextAsync_1(signal)"));
    assert!(code.contains("async _getTextAsync_2(formatId, signal)"));
    assert!(code.matches("return _v.toString();").count() >= 2);
    assert!(code.contains(
        "if (args.length >= 1 && args[0] !== undefined && !(args[0] instanceof AbortSignal))"
    ));
    assert!(declarations.contains("getTextAsync(signal?: AbortSignal): Promise<string>;"));
    assert!(
        declarations
            .contains("getTextAsync(formatId: string, signal?: AbortSignal): Promise<string>;")
    );
}
