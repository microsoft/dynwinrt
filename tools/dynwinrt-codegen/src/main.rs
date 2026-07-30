// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use clap::{Parser, Subcommand};

use dynwinrt_codegen::codegen::com;
use dynwinrt_codegen::codegen::flat;
use dynwinrt_codegen::codegen::python;
use dynwinrt_codegen::codegen::render_package_json;
use dynwinrt_codegen::codegen::typescript;
use dynwinrt_codegen::codegen::{project, render_dts, render_js};
use dynwinrt_codegen::com_metadata;
use dynwinrt_codegen::meta;
use dynwinrt_codegen::types::TypeMeta;
use dynwinrt_codegen::xml_doc::DocTable;

#[derive(Parser)]
#[command(name = "dynwinrt-codegen")]
#[command(about = "Generate typed language bindings from WinRT metadata (.winmd) files")]
#[command(
    long_about = "dynwinrt-codegen reads .winmd metadata and generates typed bindings\n\
    that use @microsoft/dynwinrt at runtime to call Windows Runtime APIs dynamically.\n\n\
    It auto-detects Windows SDK metadata and discovers sibling .winmd files\n\
    in the same directory, so you typically only need to point at one file."
)]
#[command(after_help = "\x1b[1mExamples:\x1b[0m\n\
    # Generate all namespaces from a WinAppSDK metadata folder\n\
    dynwinrt-codegen generate --folder C:\\Users\\you\\.winapp\\packages\\Microsoft.WindowsAppSDK.AI.1.8.39\\metadata\n\n\
    # Generate a single namespace (siblings auto-discovered)\n\
    dynwinrt-codegen generate --winmd path\\to\\Microsoft.Windows.AI.Imaging.winmd --namespace Microsoft.Windows.AI.Imaging\n\n\
    # Generate a single class\n\
    dynwinrt-codegen generate --namespace Windows.Foundation --class-name Uri\n\n\
    # Custom output directory\n\
    dynwinrt-codegen generate --folder path\\to\\metadata --output ./src/generated")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print supported machine-readable capabilities, one per line.
    Capabilities,

    /// Generate bindings from .winmd files
    #[command(
        long_about = "Parse .winmd metadata and generate typed binding files.\n\n\
        By default (`--lang js`) the tool emits plain ESM JavaScript (`.js`) plus\n\
        matching ambient TypeScript declarations (`.d.ts`) — no TypeScript compiler\n\
        or SWC step is involved.\n\n\
        The tool automatically:\n\
        - Detects Windows.winmd from the Windows SDK install path\n\
        - Discovers sibling .winmd files in the same directory as --winmd\n\
        - Resolves transitive type dependencies across namespaces\n\
        - Filters out Windows.* system namespaces when --namespace is omitted"
    )]
    Generate {
        /// Path(s) to .winmd metadata files, separated by ';'.
        /// Sibling .winmd files in the same directory are auto-discovered.
        /// If omitted, auto-detects Windows.winmd from Windows SDK.
        #[arg(long, value_name = "PATH")]
        winmd: Option<String>,

        /// File containing newline-separated .winmd paths to emit (one path per line).
        /// Equivalent to --winmd but read from a file, avoiding command-line length
        /// limits when many winmds are involved. Blank lines and '#' comments are ignored.
        #[arg(long = "winmd-list", value_name = "FILE")]
        winmd_list: Option<String>,

        /// Directory containing .winmd files.
        /// All .winmd files in this directory will be loaded.
        /// When --namespace is omitted, generates all non-Windows namespaces.
        #[arg(long, value_name = "DIR")]
        folder: Option<String>,

        /// Generate only this namespace (e.g. "Microsoft.Windows.AI.Imaging").
        /// If omitted, generates all non-Windows namespaces found in the winmd files.
        #[arg(long, value_name = "NS")]
        namespace: Option<String>,

        /// Generate bindings for specific class(es), comma-separated (requires --namespace).
        /// E.g. --class-name Uri or --class-name StorageFile,StorageFolder
        #[arg(long, name = "class", value_name = "NAME")]
        class_name: Option<String>,

        /// Additional .winmd files for type resolution only (no code generated).
        /// Paths separated by ';'. Sibling .winmd files are NOT auto-discovered.
        #[arg(long = "ref", value_name = "PATH")]
        ref_winmd: Option<String>,

        /// File containing newline-separated .winmd paths for type resolution only
        /// (no code generated). Equivalent to --ref but read from a file. Merged with
        /// --ref when both are given. Blank lines and '#' comments are ignored.
        #[arg(long = "ref-list", value_name = "FILE")]
        ref_list: Option<String>,

        /// Target language. `js` emits .js + .d.ts (recommended for Node consumers).
        /// `py` emits .py + .pyi and a py.typed marker.
        #[arg(long, default_value = "js", value_parser = ["js", "py"])]
        lang: String,

        /// Output directory for generated files
        #[arg(long, default_value = "./generated", value_name = "DIR")]
        output: String,

        /// Custom import name for the dynwinrt runtime package in generated JS/TS files.
        /// Defaults to "@microsoft/dynwinrt".
        #[arg(long, default_value = "@microsoft/dynwinrt", value_name = "NAME")]
        import_name: String,

        /// Validate metadata and resolve dependencies without writing files
        #[arg(long)]
        dry_run: bool,

        /// Emit Python type stubs (the default for --lang py; retained for compatibility).
        #[arg(long, conflicts_with = "no_pyi")]
        pyi: bool,

        /// Skip .pyi type stubs and the py.typed marker (requires --lang py).
        #[arg(long, conflicts_with = "pyi")]
        no_pyi: bool,
    },
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Capabilities => {
            print_capabilities();
        }
        Commands::Generate {
            winmd,
            winmd_list,
            folder,
            namespace,
            class_name,
            ref_winmd,
            ref_list,
            lang,
            output,
            import_name,
            dry_run,
            pyi,
            no_pyi,
        } => {
            if lang != "py" && (pyi || no_pyi) {
                return Err("--pyi and --no-pyi require --lang py".into());
            }
            let pyi = lang == "py" && !no_pyi;
            // Collect winmd paths from --folder and/or --winmd
            let mut winmd_parts: Vec<String> = Vec::new();

            if let Some(ref dir) = folder {
                let dir_path = Path::new(dir);
                if !dir_path.is_dir() {
                    return Err(format!("--folder path is not a directory: {}", dir));
                }
                if let Ok(entries) = fs::read_dir(dir_path) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path
                            .extension()
                            .map_or(false, |ext| ext.eq_ignore_ascii_case("winmd"))
                        {
                            eprintln!("Loading winmd from folder: {}", path.display());
                            winmd_parts.push(path.to_string_lossy().to_string());
                        }
                    }
                }
                if winmd_parts.is_empty() {
                    return Err(format!("No .winmd files found in folder: {}", dir));
                }
            }

            if let Some(ref w) = winmd {
                winmd_parts.extend(w.split(';').filter(|s| !s.is_empty()).map(String::from));
            }

            if let Some(ref lf) = winmd_list {
                winmd_parts.extend(read_path_list_file(lf)?);
            }

            // Collect ref winmd paths from --ref and --ref-list.
            let mut ref_paths: Vec<String> = Vec::new();
            if let Some(ref r) = ref_winmd {
                ref_paths.extend(r.split(';').filter(|s| !s.is_empty()).map(String::from));
            }
            if let Some(ref lf) = ref_list {
                ref_paths.extend(read_path_list_file(lf)?);
            }

            validate_winmd_paths(&winmd_parts, "winmd")?;
            validate_winmd_paths(&ref_paths, "ref")?;

            let winmd_namespaces = list_namespaces_for_paths(&winmd_parts);
            let ref_namespaces_vec = list_namespaces_for_paths(&ref_paths);

            // Auto-detect Windows SDK metadata only as a fallback. Integrators can
            // pass explicit Windows.* metadata (for example SDK.CPP split winmds)
            // via --ref/--ref-list to keep generation tied to a restored SDK version.
            let has_explicit_windows_metadata = has_windows_namespace(&winmd_namespaces)
                || has_windows_namespace(&ref_namespaces_vec);
            if !has_explicit_windows_metadata {
                if let Some(sdk_winmd) = find_windows_sdk_winmd() {
                    eprintln!("Auto-detected Windows SDK metadata: {}", sdk_winmd);
                    eprintln!(
                        "For reproducible generation, pass Windows SDK metadata via --ref or --ref-list."
                    );
                    winmd_parts.push(sdk_winmd);
                } else if folder.is_none() && winmd.is_none() && winmd_list.is_none() {
                    return Err(
                        "Could not auto-detect Windows.winmd. Please provide --winmd or --folder."
                            .into(),
                    );
                }
            }

            // Ref namespaces are excluded from generation and ref paths are appended
            // to the loaded metadata set for type resolution.
            let ref_namespaces: HashSet<String> = if !ref_paths.is_empty() {
                winmd_parts.extend(ref_paths.iter().cloned());
                ref_namespaces_vec.into_iter().collect()
            } else {
                HashSet::new()
            };

            let winmd_joined = winmd_parts.join(";");

            // Auto-discover sibling .winmd files in the same directories
            let winmd = meta::expand_winmd_paths(&winmd_joined);

            // Build XML doc table from sibling .xml files of each winmd.
            let expanded_parts: Vec<String> = winmd
                .split(';')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
            let mut doc_table = DocTable::load_from_winmd_paths(&expanded_parts);
            // Load built-in docs as fallback (sibling .xml takes priority).
            doc_table.load_builtin_docs();

            let output_dir = Path::new(&output);
            if lang == "js" {
                project::set_import_name(&import_name);
            }
            if !dry_run {
                fs::create_dir_all(output_dir).map_err(|e| {
                    format!("Failed to create output directory '{}': {}", output, e)
                })?;
            }

            if let Some(ref cls_arg) = class_name {
                // Class mode: supports comma-separated list (e.g. "StorageFile,StorageFolder")
                let ns = namespace
                    .as_deref()
                    .ok_or("--namespace is required when --class-name is specified")?;
                let class_names: Vec<&str> = cls_arg
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();

                // First: partition into WinRT classes, classic-COM interfaces,
                // and flat-Win32 [DllImport] modules.
                let mut classes = Vec::new();
                let mut com_interfaces: Vec<com_metadata::ComInterfaceMeta> = Vec::new();
                let mut flat_apis: Vec<meta::FlatApisMeta> = Vec::new();
                for cls in &class_names {
                    if let Some(apis) = meta::parse_flat_apis(&winmd, ns, cls) {
                        flat_apis.push(apis);
                        continue;
                    }
                    if let Some(com_iface) = com_metadata::parse_com_interface(&winmd, ns, cls) {
                        // Route through classic-COM path when:
                        //   1) The interface is IUnknown-rooted (base +3), OR
                        //   2) It is a `*Interop` bridge (name ends with "Interop") — even
                        //      if IInspectable-rooted (base +6), because the emitter
                        //      handles that via `registerInterface`.
                        if com_iface.is_iunknown_rooted || cls.ends_with("Interop") {
                            com_interfaces.push(com_iface);
                            continue;
                        }
                        // The type exists as an interface but is IInspectable-rooted and
                        // not `*Interop` — it's a plain WinRT interface. Those still need
                        // to go through the WinRT projection pipeline via `parse_class`,
                        // which will find it if it's the projected surface of a runtime
                        // class. If not, give a targeted error rather than the misleading
                        // "Class not found".
                        if meta::parse_class(&winmd, ns, cls).is_none() {
                            return Err(format!(
                                "{}.{} is an IInspectable-rooted WinRT interface, not a runtime class \
                                 or classic-COM interface. `--class-name` expects a WinRT runtime class, \
                                 an IUnknown-rooted classic COM interface, or a `*Interop` bridge. \
                                 If you meant to project a WinRT interface directly, use the full \
                                 namespace-projection mode (no `--class-name`).",
                                ns, cls
                            ));
                        }
                    }
                    match meta::parse_class(&winmd, ns, cls) {
                        Some(mut c) => {
                            doc_table.apply_to_class(&mut c);
                            classes.push(c);
                        }
                        None => {
                            return Err(format!("Class {}.{} not found in {}", ns, cls, winmd));
                        }
                    }
                }

                // Flat-Win32 and classic-COM codegen only emit JavaScript and
                // declarations today.
                if lang != "js" && (!flat_apis.is_empty() || !com_interfaces.is_empty()) {
                    let mut offenders: Vec<String> = Vec::new();
                    for apis in &flat_apis {
                        offenders.push(format!(
                            "{}.{} (flat-Win32 [DllImport])",
                            apis.namespace, apis.class_name
                        ));
                    }
                    for ci in &com_interfaces {
                        offenders.push(format!(
                            "{}.{} (classic-COM interface)",
                            ci.interface.namespace, ci.interface.name
                        ));
                    }
                    return Err(format!(
                        "`--lang {}` is not supported for flat-Win32 [DllImport] modules or \
                         classic-COM interfaces (they emit only `.js` + `.d.ts` today). \
                         Offending inputs: {}. Re-run with `--lang js`, or split the \
                         invocation so the WinRT classes are generated with `--lang {}` and \
                         the flat/COM classes with `--lang js`.",
                        lang,
                        offenders.join(", "),
                        lang
                    ));
                }

                if !flat_apis.is_empty() {
                    for apis in &flat_apis {
                        let out = flat::generate_flat_apis_files(apis);
                        let js_name = format!("{}.js", apis.class_name);
                        let dts_name = format!("{}.d.ts", apis.class_name);
                        if !dry_run {
                            fs::write(output_dir.join(&js_name), &out.js)
                                .map_err(|e| format!("Failed to write {}: {}", js_name, e))?;
                            fs::write(output_dir.join(&dts_name), &out.dts)
                                .map_err(|e| format!("Failed to write {}: {}", dts_name, e))?;
                            for (name, content) in &out.extra_files {
                                fs::write(output_dir.join(name), content)
                                    .map_err(|e| format!("Failed to write {}: {}", name, e))?;
                            }
                            println!(
                                "Generated flat-Win32 {}.{} ({} methods, {} extra files)",
                                apis.namespace,
                                apis.class_name,
                                apis.methods.len(),
                                out.extra_files.len()
                            );
                        } else {
                            println!(
                                "[dry-run] Would generate flat-Win32 {}.{}",
                                apis.namespace, apis.class_name
                            );
                        }
                    }
                    if classes.is_empty() && com_interfaces.is_empty() {
                        return Ok(());
                    }
                }

                if !com_interfaces.is_empty() && !classes.is_empty() {
                    return Err(
                        "Classic-COM and WinRT class generation cannot share one output package yet. \
                         Run separate `generate` commands with separate output directories."
                            .into(),
                    );
                }

                // Emit classic-COM interfaces. Mixed WinRT/COM packages were
                // rejected above; COM-only output is finalized below.
                if !com_interfaces.is_empty() {
                    for com_iface in &com_interfaces {
                        let out =
                            com::generate_com_interface_files(com_iface, &winmd).map_err(|e| {
                                format!(
                                    "Classic-COM codegen for {} failed: {}",
                                    com_iface.interface.name, e
                                )
                            })?;
                        let js_name = format!("{}.js", com_iface.interface.name);
                        let dts_name = format!("{}.d.ts", com_iface.interface.name);
                        if !dry_run {
                            fs::write(output_dir.join(&js_name), &out.js)
                                .map_err(|e| format!("Failed to write {}: {}", js_name, e))?;
                            fs::write(output_dir.join(&dts_name), &out.dts)
                                .map_err(|e| format!("Failed to write {}: {}", dts_name, e))?;
                            for (name, content) in &out.extra_files {
                                fs::write(output_dir.join(name), content)
                                    .map_err(|e| format!("Failed to write {}: {}", name, e))?;
                            }
                            println!(
                                "Generated {} ({} .js/.d.ts + {} extras)",
                                com_iface.interface.name,
                                2,
                                out.extra_files.len()
                            );
                        } else {
                            println!("[dry-run] Would generate {}", com_iface.interface.name);
                        }
                    }
                    // If we only had classic-COM interfaces requested, return early —
                    // no WinRT index/barrel work to do.
                    if classes.is_empty() {
                        if !dry_run {
                            write_com_js_barrel_and_manifest(output_dir)?;
                        }
                        return Ok(());
                    }
                }

                add_implicit_js_types(&winmd, &lang, &mut classes);
                generate_for_types(
                    &winmd,
                    output_dir,
                    classes.clone(),
                    Vec::new(),
                    Vec::new(),
                    dry_run,
                    &lang,
                    pyi,
                    &doc_table,
                )?;

                // Write (or append to) the index file for the output directory
                if !dry_run {
                    type AppendFn =
                        fn(&str, &[meta::ClassMeta], &[meta::InterfaceMeta], &[TypeMeta]) -> String;
                    type GenerateFn =
                        fn(&[meta::ClassMeta], &[meta::InterfaceMeta], &[TypeMeta]) -> String;
                    let (index_name, append_fn, generate_fn): (&str, AppendFn, GenerateFn) =
                        if lang == "py" {
                            (
                                "__init__.py",
                                python::append_to_index,
                                python::generate_index,
                            )
                        } else {
                            // For JS lang, we use an in-memory `.ts` index then split into .js + .d.ts.
                            // We pick a sentinel filename `index.js` to detect presence; `.d.ts` is written alongside.
                            (
                                "index.js",
                                typescript::append_to_index,
                                typescript::generate_index,
                            )
                        };
                    let index_path = output_dir.join(index_name);
                    let deps = meta::resolve_dependencies(&winmd, &classes, &[], &[]);
                    let mut all_classes = [classes.as_slice(), deps.classes.as_slice()].concat();
                    let mut all_interfaces: Vec<_> = deps.interfaces.clone();
                    let mut all_enums: Vec<_> = deps.enums.clone();
                    for c in all_classes.iter_mut() {
                        doc_table.apply_to_class(c);
                    }
                    for i in all_interfaces.iter_mut() {
                        doc_table.apply_to_interface(i);
                    }
                    for e in all_enums.iter_mut() {
                        doc_table.apply_to_enum(e);
                    }
                    // Keep the barrel/index in sync with `generate_js_files` —
                    // interfaces with no IID or that collide with a class name
                    // are not emitted, so must not appear in the barrel either.
                    let __cls_names: HashSet<String> =
                        all_classes.iter().map(|c| c.name.clone()).collect();
                    all_interfaces.retain(|i| !i.iid.is_empty() && !__cls_names.contains(&i.name));
                    all_enums.retain(|e| match e {
                        TypeMeta::Enum { name, .. } => !__cls_names.contains(name),
                        _ => true,
                    });
                    if lang == "py" {
                        if index_path.exists() {
                            let existing = fs::read_to_string(&index_path).map_err(|e| {
                                format!("Failed to read {}: {}", index_path.display(), e)
                            })?;
                            let updated =
                                append_fn(&existing, &all_classes, &all_interfaces, &all_enums);
                            fs::write(&index_path, &updated).map_err(|e| {
                                format!("Failed to write {}: {}", index_path.display(), e)
                            })?;
                            println!("Updated {}", index_path.display());
                        } else {
                            let new_index = generate_fn(&all_classes, &all_interfaces, &all_enums);
                            fs::write(&index_path, &new_index).map_err(|e| {
                                format!("Failed to write {}: {}", index_path.display(), e)
                            })?;
                            println!("Generated {}", index_path.display());
                        }
                    } else {
                        // JS: index.js + index.d.ts are pure re-exports and identical, so we
                        // round-trip incremental appends by reading back index.d.ts (which
                        // still uses ESM syntax and drives the append-diff logic).
                        let dts_path = output_dir.join("index.d.ts");
                        let index_content = if dts_path.exists() {
                            let existing = fs::read_to_string(&dts_path).map_err(|e| {
                                format!("Failed to read {}: {}", dts_path.display(), e)
                            })?;
                            append_fn(&existing, &all_classes, &all_interfaces, &all_enums)
                        } else {
                            generate_fn(&all_classes, &all_interfaces, &all_enums)
                        };
                        write_js_barrel_and_manifest(output_dir, &index_content)?;
                    }
                    if pyi {
                        let stub_code = dynwinrt_codegen::codegen::python_stub::generate_index_stub(
                            &all_classes,
                            &all_interfaces,
                            &all_enums,
                        );
                        let stub_path = output_dir.join("__init__.pyi");
                        fs::write(&stub_path, &stub_code).map_err(|e| {
                            format!("Failed to write {}: {}", stub_path.display(), e)
                        })?;
                        println!("Generated {}", stub_path.display());
                        let marker = output_dir.join("py.typed");
                        fs::write(&marker, "")
                            .map_err(|e| format!("Failed to write {}: {}", marker.display(), e))?;
                    }
                }
            } else {
                // Determine which namespaces to generate
                let namespaces = match namespace {
                    Some(ref ns) => vec![ns.clone()],
                    None => {
                        let all_ns = meta::list_namespaces(&winmd);
                        let filtered: Vec<String> = all_ns
                            .into_iter()
                            .filter(|ns| {
                                !ns.starts_with("Windows.") && !ref_namespaces.contains(ns)
                            })
                            .collect();
                        if filtered.is_empty() {
                            return Err(
                                "No non-Windows namespaces found. Use --namespace to specify one."
                                    .into(),
                            );
                        }
                        eprintln!("Discovered {} namespace(s) to generate:", filtered.len());
                        for ns in &filtered {
                            eprintln!("  {}", ns);
                        }
                        filtered
                    }
                };

                let mut total_classes = 0usize;
                let mut total_interfaces = 0usize;
                let mut total_enums = 0usize;

                for ns in &namespaces {
                    if let Some(interface) =
                        com_metadata::first_classic_com_interface_in_namespace(&winmd, ns)
                    {
                        return Err(format!(
                            "classic-COM namespace projection is not supported because `{ns}` \
                             contains `{interface}`. Use `--class-name {interface}` (or a \
                             comma-separated class list) so each interface is validated by the \
                             Classic-COM ABI pipeline."
                        ));
                    }
                    let mut classes = meta::parse_namespace(&winmd, ns);
                    let mut interfaces = meta::parse_interfaces(&winmd, ns);
                    let mut enums = meta::parse_enums(&winmd, ns);
                    add_implicit_js_types(&winmd, &lang, &mut classes);
                    for c in classes.iter_mut() {
                        doc_table.apply_to_class(c);
                    }
                    for i in interfaces.iter_mut() {
                        doc_table.apply_to_interface(i);
                    }
                    for e in enums.iter_mut() {
                        doc_table.apply_to_enum(e);
                    }

                    let (nc, ni, ne) = generate_for_types(
                        &winmd, output_dir, classes, interfaces, enums, dry_run, &lang, pyi,
                        &doc_table,
                    )?;
                    total_classes += nc;
                    total_interfaces += ni;
                    total_enums += ne;
                }

                // Generate index file combining everything
                if !dry_run
                    && namespaces.len() >= 1
                    && (total_classes + total_interfaces + total_enums) > 1
                {
                    let mut all_classes = Vec::new();
                    let mut all_interfaces = Vec::new();
                    let mut all_enums = Vec::new();
                    for ns in &namespaces {
                        all_classes.extend(meta::parse_namespace(&winmd, ns));
                        all_interfaces.extend(meta::parse_interfaces(&winmd, ns));
                        all_enums.extend(meta::parse_enums(&winmd, ns));
                    }
                    let deps = meta::resolve_dependencies(
                        &winmd,
                        &all_classes,
                        &all_interfaces,
                        &all_enums,
                    );
                    all_classes.extend(deps.classes);
                    all_interfaces.extend(deps.interfaces);
                    all_enums.extend(deps.enums);
                    for c in all_classes.iter_mut() {
                        doc_table.apply_to_class(c);
                    }
                    for i in all_interfaces.iter_mut() {
                        doc_table.apply_to_interface(i);
                    }
                    for e in all_enums.iter_mut() {
                        doc_table.apply_to_enum(e);
                    }
                    let __cls_names: HashSet<String> =
                        all_classes.iter().map(|c| c.name.clone()).collect();
                    all_interfaces.retain(|i| !i.iid.is_empty() && !__cls_names.contains(&i.name));
                    all_enums.retain(|e| match e {
                        TypeMeta::Enum { name, .. } => !__cls_names.contains(name),
                        _ => true,
                    });

                    if lang == "py" {
                        let index_code =
                            python::generate_index(&all_classes, &all_interfaces, &all_enums);
                        let index_path = output_dir.join("__init__.py");
                        fs::write(&index_path, &index_code).map_err(|e| {
                            format!("Failed to write {}: {}", index_path.display(), e)
                        })?;
                        println!("Generated {}", index_path.display());
                        if pyi {
                            let stub_code =
                                dynwinrt_codegen::codegen::python_stub::generate_index_stub(
                                    &all_classes,
                                    &all_interfaces,
                                    &all_enums,
                                );
                            let stub_path = output_dir.join("__init__.pyi");
                            fs::write(&stub_path, &stub_code).map_err(|e| {
                                format!("Failed to write {}: {}", stub_path.display(), e)
                            })?;
                            println!("Generated {}", stub_path.display());
                            let marker = output_dir.join("py.typed");
                            fs::write(&marker, "").map_err(|e| {
                                format!("Failed to write {}: {}", marker.display(), e)
                            })?;
                        }
                    } else {
                        let index_code =
                            typescript::generate_index(&all_classes, &all_interfaces, &all_enums);
                        write_js_barrel_and_manifest(output_dir, &index_code)?;
                    }
                }

                if dry_run {
                    println!(
                        "Done. {} class(es) + {} interface(s) + {} enum(s) validated (dry run)",
                        total_classes, total_interfaces, total_enums,
                    );
                } else {
                    println!(
                        "Done. {} class(es) + {} interface(s) + {} enum(s) generated in {}",
                        total_classes,
                        total_interfaces,
                        total_enums,
                        output_dir.display()
                    );
                }
            }
        }
    }
    Ok(())
}

fn add_implicit_js_types(winmd: &str, lang: &str, classes: &mut Vec<meta::ClassMeta>) {
    if lang != "js"
        || !classes
            .iter()
            .any(|class| class.full_name == "Microsoft.UI.Xaml.Application")
    {
        return;
    }

    for (namespace, name) in [
        (
            "Microsoft.UI.Xaml.XamlTypeInfo",
            "XamlControlsXamlMetaDataProvider",
        ),
        ("Microsoft.UI.Xaml.Controls", "XamlControlsResources"),
    ] {
        let full_name = format!("{}.{}", namespace, name);
        if classes.iter().any(|class| class.full_name == full_name) {
            continue;
        }
        if let Some(class) = meta::parse_class(winmd, namespace, name) {
            classes.push(class);
        }
    }
}

/// Generate files for a set of types plus their transitive dependencies.
/// When `dry_run` is true, all parsing/resolution runs but no files are written.
fn generate_for_types(
    winmd: &str,
    output_dir: &Path,
    classes: Vec<meta::ClassMeta>,
    interfaces: Vec<meta::InterfaceMeta>,
    enums: Vec<TypeMeta>,
    dry_run: bool,
    lang: &str,
    pyi: bool,
    doc_table: &DocTable,
) -> Result<(usize, usize, usize), String> {
    let deps = meta::resolve_dependencies(winmd, &classes, &interfaces, &enums);
    let mut all_classes = classes;
    let mut all_interfaces = interfaces;
    let mut all_enums = enums;
    all_classes.extend(deps.classes);
    all_interfaces.extend(deps.interfaces);
    all_enums.extend(deps.enums);

    // Newly-merged dependency types haven't been doc-annotated yet. Apply doc table
    // uniformly so dependency classes/interfaces/enums carry the same XML docs as
    // the primary types.
    for c in all_classes.iter_mut() {
        doc_table.apply_to_class(c);
    }
    for i in all_interfaces.iter_mut() {
        doc_table.apply_to_interface(i);
    }
    for e in all_enums.iter_mut() {
        doc_table.apply_to_enum(e);
    }

    // Compute the set of interfaces `generate_js_files` will actually emit
    // (matches the class-name-collision + no-IID filter there). Everything
    // downstream — `known_types`, the barrel index, generated imports — must
    // agree, or classes will try to `import` sibling files that never landed.
    let class_names_all: HashSet<String> = all_classes.iter().map(|c| c.name.clone()).collect();
    let is_emittable_iface = |i: &meta::InterfaceMeta| -> bool {
        !i.iid.is_empty() && !class_names_all.contains(&i.name)
    };
    let emittable_interfaces: Vec<meta::InterfaceMeta> = all_interfaces
        .iter()
        .filter(|i| is_emittable_iface(i))
        .cloned()
        .collect();

    let mut known_types: HashSet<String> = HashSet::new();
    for c in &all_classes {
        known_types.insert(c.name.clone());
    }
    for i in &emittable_interfaces {
        known_types.insert(i.name.clone());
    }
    for e in &all_enums {
        if let TypeMeta::Enum { name, .. } = e {
            if !class_names_all.contains(name) {
                known_types.insert(name.clone());
            }
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

    let mut req_iface_count: HashMap<String, (&meta::InterfaceMeta, usize)> = HashMap::new();
    for class in &all_classes {
        for ri in &class.required_interfaces {
            if ri.iid.is_empty() {
                continue;
            }
            req_iface_count
                .entry(ri.iid.clone())
                .and_modify(|(_, c)| *c += 1)
                .or_insert((ri, 1));
        }
    }
    let shared_iids: HashSet<String> = req_iface_count
        .iter()
        .filter(|(_, (_, count))| *count >= 2)
        .map(|(iid, _)| iid.clone())
        .collect();

    let shared_interfaces: Vec<meta::InterfaceMeta> = req_iface_count
        .iter()
        .filter(|(_, (_, count))| *count >= 2)
        .map(|(_, (iface, _))| (*iface).clone())
        .collect();
    for iface in &shared_interfaces {
        known_types.insert(iface.name.clone());
    }

    let (delegate_signatures, delegate_sig_refs, delegate_param_wraps) =
        project::build_delegate_signatures(&all_interfaces, &delegate_type_names, &known_types);

    if !dry_run {
        if lang == "py" {
            generate_py_files(
                output_dir,
                &all_classes,
                &all_interfaces,
                &all_enums,
                &shared_interfaces,
                &known_types,
                &delegate_type_names,
                &shared_iids,
                pyi,
            )?;
        } else {
            generate_js_files(
                output_dir,
                &all_classes,
                &all_interfaces,
                &all_enums,
                &shared_interfaces,
                &known_types,
                &delegate_type_names,
                &shared_iids,
                &delegate_signatures,
                &delegate_sig_refs,
                &delegate_param_wraps,
            )?;
        }
    }

    Ok((all_classes.len(), all_interfaces.len(), all_enums.len()))
}

fn generate_js_files(
    output_dir: &Path,
    all_classes: &[meta::ClassMeta],
    all_interfaces: &[meta::InterfaceMeta],
    all_enums: &[TypeMeta],
    shared_interfaces: &[meta::InterfaceMeta],
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
    shared_iids: &HashSet<String>,
    delegate_sigs: &HashMap<String, String>,
    delegate_sig_refs: &HashMap<String, Vec<String>>,
    delegate_param_wraps: &HashMap<String, Vec<String>>,
) -> Result<(), String> {
    let emit = |name: &str, js_code: &str, dts_code: &str| -> Result<(), String> {
        let js_path = output_dir.join(format!("{}.js", name));
        let dts_path = output_dir.join(format!("{}.d.ts", name));
        write_file(&js_path, js_code)?;
        write_file(&dts_path, dts_code)?;
        println!("Generated {}", js_path.display());
        Ok(())
    };

    // Interfaces whose short name collides with a class in this batch would
    // overwrite the class's UIElement.js / Button.js etc. Skip them; the
    // parameterized instantiations that produce these entries are not
    // themselves useful runtime bindings.
    let class_names: HashSet<&str> = all_classes.iter().map(|c| c.name.as_str()).collect();

    // Interface entries with no IID and no generic PIID are synthesized stubs
    // (typically parameterized instantiations named after a class type
    // parameter, e.g. `UIElement` from `IIterable<UIElement>`). They must not
    // be emitted — the file would have a broken `registerInterface` call with
    // no matching IID declaration and would overwrite any real class file of
    // the same name.
    fn is_emittable_interface(iface: &meta::InterfaceMeta) -> bool {
        !iface.iid.is_empty()
    }

    for iface in shared_interfaces {
        if class_names.contains(iface.name.as_str()) {
            continue;
        }
        if !is_emittable_interface(iface) {
            continue;
        }
        let projected = project::project_interface(
            iface,
            known_types,
            delegate_type_names,
            delegate_sigs,
            delegate_sig_refs,
            delegate_param_wraps,
        );
        let js = render_js::render(&projected);
        let dts = render_dts::render(&projected);
        emit(&iface.name, &js, &dts)?;
    }
    for iface in all_interfaces {
        if class_names.contains(iface.name.as_str()) {
            continue;
        }
        if !is_emittable_interface(iface) {
            continue;
        }
        let projected = project::project_interface(
            iface,
            known_types,
            delegate_type_names,
            delegate_sigs,
            delegate_sig_refs,
            delegate_param_wraps,
        );
        let js = render_js::render(&projected);
        let dts = render_dts::render(&projected);
        emit(&iface.name, &js, &dts)?;
    }
    for en in all_enums {
        if let TypeMeta::Enum { name, .. } = en {
            if name.contains('<') {
                continue;
            } // skip CLR projection types
            if class_names.contains(name.as_str()) {
                continue;
            }
            if let Some(projected) = project::project_enum(en) {
                let js = render_js::render(&projected);
                let dts = render_dts::render(&projected);
                emit(name, &js, &dts)?;
            }
        }
    }
    // First pass: emit "real" classes. A class is emit-worthy if any of these
    // hold:
    //   * it has a default interface with a resolvable IID (normal
    //     activatable / composable class), OR
    //   * it has factory/statics interfaces (statics-only utility classes such
    //     as `LimitedAccessFeatures`, `AICapabilities`, `PowerManager`), OR
    //   * it has required or overridable methods callable on an inbound obj.
    //
    // Only synthetic parameterized "instantiations" (e.g. `IIterable<UIElement>`
    // reprojected as a class stub) with nothing at all fall through to the
    // stub-emission pass below.
    let mut emitted_class_names: HashSet<String> = HashSet::new();
    let class_is_usable = |class: &meta::ClassMeta| -> bool {
        let has_default_iid = class
            .default_interface
            .as_ref()
            .map_or(false, |di| !di.iid.is_empty());
        let has_statics_or_factory =
            !class.static_interfaces.is_empty() || !class.factory_interfaces.is_empty();
        let has_required = !class.required_interfaces.is_empty();
        has_default_iid || has_statics_or_factory || has_required
    };
    for class in all_classes {
        if !class_is_usable(class) {
            continue;
        }
        let projected = project::project_class(
            class,
            known_types,
            delegate_type_names,
            shared_iids,
            delegate_sigs,
            delegate_sig_refs,
            delegate_param_wraps,
        );
        let js = render_js::render(&projected);
        let dts = render_dts::render(&projected);
        emit(&class.name, &js, &dts)?;
        emitted_class_names.insert(class.name.clone());
    }
    // Second pass: emit stubs for genuinely empty class shells (parameterized
    // synthetics from the projection layer that carry no methods). Stubs keep
    // ESM barrel imports linkable even when the underlying type isn't
    // constructible at runtime.
    for class in all_classes {
        if class_is_usable(class) {
            continue;
        }
        if emitted_class_names.contains(&class.name) {
            continue;
        }
        let stub_js = format!(
            "// Generated by dynwinrt-codegen \u{2014} do not edit\n\
             // Placeholder for a class whose default interface has no IID in\n\
             // the loaded winmd graph. Any attempt to use it will throw.\n\
             const __unavailable = () => {{ throw new Error(\"'{name}' has no default interface in the loaded winmd graph and cannot be constructed. Add its owning package to `additionalWinmds` / `additionalRefs`.\"); }};\n\
             class {name} {{ constructor() {{ __unavailable(); }} }}\n\
             exports.{name} = {name};\n",
            name = class.name,
        );
        let stub_dts = format!(
            "// Generated by dynwinrt-codegen \u{2014} do not edit\n\
             // Placeholder: throwing at construction. Typed as a class so
             // other .d.ts files can still use `{name}` as a parameter /
             // return type.\n\
             export declare class {name} {{ private constructor(); }}\n",
            name = class.name,
        );
        emit(&class.name, &stub_js, &stub_dts)?;
        emitted_class_names.insert(class.name.clone());
    }

    // Post-process: strip imports that reference non-existent sibling files.
    // This handles cases where a class pulls in a type reference (e.g. WinUI XAML
    // classes referencing Microsoft.UI.Composition types whose winmd was removed
    // in later Windows App SDK versions) but the target file was never emitted.
    // Rather than leave the entire binding set broken at load time, we drop the
    // import — any methods that depended on that type will surface as runtime
    // ReferenceErrors when actually called, but the module loads.
    strip_broken_imports(output_dir)?;

    Ok(())
}

/// Write the four barrel entries plus `package.json` alongside the per-type
/// files already emitted into `output_dir`.
///
/// The four barrels are:
///   * `index.js`         — CJS `Object.defineProperty` getter barrel — real
///                          values, lazy, default for `require('@winapp/bindings')`.
///   * `index.mjs`        — standard ESM re-export barrel — real values,
///                          tree-shakable for bundlers.
///   * `index.proxy.js`   — legacy CJS Proxy barrel — lexer-visible
///                          `exports.X = ...` assignments for opt-in
///                          compatibility via `@winapp/bindings/proxy`.
///   * `index.d.ts`       — TypeScript declarations shared across every path.
///
/// After the barrels are on disk we run `strip_broken_imports` so any lazy
/// getters referencing sibling modules that were filtered out during emission
/// are removed cleanly. Then we scan the directory for real `.js` files (each
/// one corresponds to a subpath consumer can deep-import) and emit a
/// `package.json` with the conditional-exports map.
fn write_js_barrel_and_manifest(output_dir: &Path, index_content: &str) -> Result<(), String> {
    let js_path = output_dir.join("index.js");
    let mjs_path = output_dir.join("index.mjs");
    let proxy_path = output_dir.join("index.proxy.js");
    let dts_path = output_dir.join("index.d.ts");
    let pkg_json_path = output_dir.join("package.json");
    let _ = index_content;
    write_lifetime_module(output_dir)?;

    // Clean up any stale `.index.ts` cache from older codegen versions.
    let stale = output_dir.join(".index.ts");
    if stale.exists() {
        let _ = fs::remove_file(&stale);
    }

    // Remove the previous opt-in getter barrel name if it exists from older
    // generated output. `index.js` is now the getter barrel and
    // `index.proxy.js` is the explicit compatibility path.
    let stale_getter = output_dir.join("index.getter.js");
    if stale_getter.exists() {
        let _ = fs::remove_file(&stale_getter);
    }

    // Sweep index.js and any other files that still reference sibling modules
    // that were skipped by class/interface filters during emission.
    strip_broken_imports(output_dir)?;

    // Build the barrel from what actually landed on disk rather than from raw
    // metadata. This avoids root ESM/CJS barrels referencing files or helper
    // exports that were filtered out (for example ref-only WinUI controls such
    // as CompositionTarget).
    let index_content = render_index_from_existing_js_files(output_dir)?;

    let js_content = typescript::esm_index_to_cjs_getter(&index_content);
    fs::write(&js_path, &js_content)
        .map_err(|e| format!("Failed to write {}: {}", js_path.display(), e))?;

    let mjs_content = typescript::esm_index_to_esm(&index_content);
    fs::write(&mjs_path, &mjs_content)
        .map_err(|e| format!("Failed to write {}: {}", mjs_path.display(), e))?;

    let proxy_content = typescript::esm_index_to_cjs_lazy(&index_content);
    fs::write(&proxy_path, &proxy_content)
        .map_err(|e| format!("Failed to write {}: {}", proxy_path.display(), e))?;

    fs::write(&dts_path, &index_content)
        .map_err(|e| format!("Failed to write {}: {}", dts_path.display(), e))?;

    // Now scan the directory for the concrete `.js` files that landed on
    // disk — that's the authoritative subpath list for the manifest. We
    // exclude the barrel entries themselves; everything else is a
    // consumer-facing subpath.
    let subpath_names = collect_subpath_names_from_dir(output_dir);
    let pkg_json_content =
        render_package_json::render_package_json(&render_package_json::PackageManifestInput {
            subpath_names: &subpath_names,
        });
    fs::write(&pkg_json_path, &pkg_json_content)
        .map_err(|e| format!("Failed to write {}: {}", pkg_json_path.display(), e))?;

    println!("Generated {}", js_path.display());
    Ok(())
}

fn write_com_js_barrel_and_manifest(output_dir: &Path) -> Result<(), String> {
    let mut modules: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let entries = fs::read_dir(output_dir).map_err(|error| {
        format!(
            "Failed to read COM output directory {}: {error}",
            output_dir.display()
        )
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(module) = file_name.strip_suffix(".js") else {
            continue;
        };
        if module == "index" {
            continue;
        }
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        let exports = collect_com_esm_exports(&content);
        if !exports.is_empty() {
            modules.insert(module.to_string(), exports);
        }
    }

    let mut index = String::from("// Generated by dynwinrt-codegen - do not edit\n");
    for (module, exports) in &modules {
        index.push_str(&format!(
            "export {{ {} }} from './{module}.js';\n",
            exports.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    fs::write(output_dir.join("index.js"), &index)
        .map_err(|error| format!("Failed to write COM index.js: {error}"))?;
    fs::write(output_dir.join("index.d.ts"), &index)
        .map_err(|error| format!("Failed to write COM index.d.ts: {error}"))?;

    let mut package = String::from(
        "{\n  \"name\": \"@winapp/bindings\",\n  \"type\": \"module\",\n  \
         \"sideEffects\": false,\n  \"main\": \"./index.js\",\n  \
         \"types\": \"./index.d.ts\",\n  \"exports\": {\n    \".\": {\n      \
         \"types\": \"./index.d.ts\",\n      \"import\": \"./index.js\",\n      \
         \"default\": \"./index.js\"\n    }",
    );
    for module in modules.keys() {
        package.push_str(&format!(
            ",\n    \"./{module}\": {{\n      \"types\": \"./{module}.d.ts\",\n      \
             \"import\": \"./{module}.js\",\n      \"default\": \"./{module}.js\"\n    }}"
        ));
    }
    package.push_str("\n  }\n}\n");
    fs::write(output_dir.join("package.json"), package)
        .map_err(|error| format!("Failed to write COM package.json: {error}"))?;
    Ok(())
}

fn collect_com_esm_exports(content: &str) -> BTreeSet<String> {
    const PREFIXES: &[&str] = &["export const ", "export class ", "export function "];
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let rest = PREFIXES
                .iter()
                .find_map(|prefix| line.strip_prefix(prefix))?;
            let name = rest
                .chars()
                .take_while(|character| {
                    character.is_ascii_alphanumeric() || *character == '_' || *character == '$'
                })
                .collect::<String>();
            (!name.is_empty()).then_some(name)
        })
        .collect()
}

fn write_lifetime_module(output_dir: &Path) -> Result<(), String> {
    let js = "'use strict';\n\
let activeScope = null;\n\
const trackedScopes = new WeakMap();\n\
function trackProjectedValue(value, typeName) {\n\
  activeScope?.track(value, typeName);\n\
  return value;\n\
}\n\
function isObjectLike(value) {\n\
  return value !== null && (typeof value === 'object' || typeof value === 'function');\n\
}\n\
function removeTracking(scope, value) {\n\
  const scopes = trackedScopes.get(value);\n\
  if (!scopes) return;\n\
  scopes.delete(scope);\n\
  if (scopes.size === 0) trackedScopes.delete(value);\n\
}\n\
function untrackProjectedValue(value) {\n\
  const scopes = trackedScopes.get(value);\n\
  if (!scopes) return;\n\
  for (const scope of [...scopes]) scope.untrack(value);\n\
}\n\
function castProjectedValueOwned(value, iid, typeName) {\n\
  let projected;\n\
  try { projected = value.cast(iid); }\n\
  catch (error) {\n\
    try { value.release(); } catch {}\n\
    throw error;\n\
  }\n\
  if (projected !== value) {\n\
    try { value.release(); }\n\
    catch (error) {\n\
      try { projected.release(); } catch {}\n\
      throw error;\n\
    }\n\
  }\n\
  return trackProjectedValue(projected, typeName);\n\
}\n\
function castProjectedValueBorrowed(value, iid, typeName) {\n\
  return trackProjectedValue(value.cast(iid), typeName);\n\
}\n\
const castProjectedValue = castProjectedValueOwned;\n\
function projectAs(value, type) {\n\
  const source = isObjectLike(value) && '_obj' in value ? value._obj : value;\n\
  if (!isObjectLike(source)) throw new TypeError('projectAs requires a projected value or wrapper.');\n\
  if (!isObjectLike(type) || typeof type._fromNativeBorrowed !== 'function') {\n\
    throw new TypeError('projectAs requires a generated runtime class type.');\n\
  }\n\
  const projected = type._fromNativeBorrowed(source);\n\
  if (!isObjectLike(projected) || !('_obj' in projected)) {\n\
    throw new TypeError('The generated runtime class returned an invalid projection.');\n\
  }\n\
  return projected;\n\
}\n\
function releaseProjected(value) {\n\
  if (!isObjectLike(value) || !('_obj' in value)) {\n\
    throw new TypeError('releaseProjected requires a generated projected wrapper.');\n\
  }\n\
  const projected = value._obj;\n\
  if (!isObjectLike(projected) || typeof projected.release !== 'function') {\n\
    throw new TypeError('The projected wrapper does not contain a releasable native value.');\n\
  }\n\
  projected.release();\n\
  untrackProjectedValue(projected);\n\
}\n\
function createProjectedLifetimeScope() {\n\
  const previousScope = activeScope;\n\
  const registry = new Map();\n\
  let disposed = false;\n\
  const scope = {\n\
    get disposed() { return disposed; },\n\
    track(value, typeName) {\n\
      if (disposed) throw new Error('Cannot track values in a disposed projection scope.');\n\
      if (registry.has(value)) return;\n\
      registry.set(value, typeName);\n\
      let scopes = trackedScopes.get(value);\n\
      if (!scopes) trackedScopes.set(value, scopes = new Set());\n\
      scopes.add(scope);\n\
    },\n\
    untrack(value) {\n\
      registry.delete(value);\n\
      removeTracking(scope, value);\n\
    },\n\
    dispose() {\n\
      if (disposed) return;\n\
      if (activeScope !== scope) throw new Error('Projection lifetime scopes must be disposed in LIFO order.');\n\
      let firstError;\n\
      for (const [value] of [...registry].reverse()) {\n\
        try { value.release(); scope.untrack(value); }\n\
        catch (error) { firstError ??= error; }\n\
      }\n\
      if (firstError !== undefined) throw firstError;\n\
      disposed = true;\n\
      activeScope = previousScope;\n\
    },\n\
  };\n\
  activeScope = scope;\n\
  return scope;\n\
}\n\
exports.trackProjectedValue = trackProjectedValue;\n\
exports.castProjectedValue = castProjectedValue;\n\
exports.castProjectedValueOwned = castProjectedValueOwned;\n\
exports.castProjectedValueBorrowed = castProjectedValueBorrowed;\n\
exports.projectAs = projectAs;\n\
exports.releaseProjected = releaseProjected;\n\
exports.createProjectedLifetimeScope = createProjectedLifetimeScope;\n";
    let dts = "export declare function trackProjectedValue<T extends object>(value: T, typeName: string): T;\n\
export declare function castProjectedValue<T extends object>(value: T, iid: unknown, typeName: string): T;\n\
export declare function castProjectedValueOwned<T extends object>(value: T, iid: unknown, typeName: string): T;\n\
export declare function castProjectedValueBorrowed<T extends object>(value: T, iid: unknown, typeName: string): T;\n\
export interface ProjectedType<T extends object> {\n\
  readonly prototype: T;\n\
}\n\
export declare function projectAs<T extends object>(value: unknown, type: ProjectedType<T>): T;\n\
export declare function releaseProjected(value: object): void;\n\
export interface ProjectedLifetimeScope {\n\
  readonly disposed: boolean;\n\
  dispose(): void;\n\
}\n\
export declare function createProjectedLifetimeScope(): ProjectedLifetimeScope;\n";
    fs::write(output_dir.join("lifetime.js"), js)
        .map_err(|e| format!("Failed to write lifetime.js: {e}"))?;
    fs::write(output_dir.join("lifetime.d.ts"), dts)
        .map_err(|e| format!("Failed to write lifetime.d.ts: {e}"))?;
    Ok(())
}

fn render_index_from_existing_js_files(output_dir: &Path) -> Result<String, String> {
    let mut out = String::from("// Generated by dynwinrt-codegen \u{2014} do not edit\n");
    // Two-pass so dedup is deterministic regardless of fs::read_dir order:
    // pass 1 collects (stem, exports) and sorts by stem; pass 2 dedupes so
    // the alphabetically-first module wins each shared symbol (interface
    // files like `IStringable.js` win over classes that re-export the IID).
    let mut candidates: Vec<(String, Vec<String>)> = Vec::new();
    let entries = fs::read_dir(output_dir).map_err(|e| {
        format!(
            "Failed to read output directory {}: {}",
            output_dir.display(),
            e
        )
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(stem) = fname.strip_suffix(".js") else {
            continue;
        };
        if matches!(stem, "index" | "index.proxy" | "index.getter") {
            continue;
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let mut names = collect_public_exports_from_js(&content);
        names.sort();
        names.dedup();
        if names.is_empty() {
            continue;
        }
        candidates.push((stem.to_string(), names));
    }
    // Prefer the canonical owner (module stem == export name) so shared
    // symbols like `IMemoryBuffer` come from `IMemoryBuffer.js`, not from an
    // alphabetically earlier consumer like `BitmapBuffer.js`.
    let canonical: BTreeSet<String> = candidates
        .iter()
        .filter(|(stem, names)| names.iter().any(|n| n == stem))
        .map(|(stem, _)| stem.clone())
        .collect();
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    let mut seen_exports: BTreeSet<String> = BTreeSet::new();
    let mut modules: Vec<(String, Vec<String>)> = Vec::new();
    for (stem, names) in candidates {
        let filtered: Vec<String> = names
            .into_iter()
            .filter(|name| {
                if canonical.contains(name) && name != &stem {
                    return false;
                }
                seen_exports.insert(name.clone())
            })
            .collect();
        if filtered.is_empty() {
            continue;
        }
        modules.push((stem, filtered));
    }
    for (module, names) in modules {
        out.push_str(&format!(
            "export {{ {} }} from './{}.js';\n",
            names.join(", "),
            module
        ));
    }
    Ok(out)
}

fn collect_public_exports_from_js(content: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in content.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix("exports.") else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
            .collect();
        if name.is_empty() {
            continue;
        }
        // Keep IIDs, parameter type arrays, and struct type descriptors scoped
        // to their per-type modules. Root barrels should expose user-facing
        // classes, enums, pack/unpack helpers, and interfaces only.
        if name == "trackProjectedValue"
            || name == "castProjectedValue"
            || name == "castProjectedValueOwned"
            || name == "castProjectedValueBorrowed"
            || name.starts_with("__")
            || name.starts_with("IID_")
            || name.ends_with("_PARAM_TYPES")
            || name.ends_with("_Type")
        {
            continue;
        }
        names.push(name);
    }
    names
}

/// Enumerate the per-type `.js` files in `output_dir` and return their
/// basenames (without extension) sorted alphabetically. Excludes barrel files
/// (`index.js`, `index.mjs`, `index.proxy.js`, and legacy `index.getter.js`).
/// Non-existent or unreadable
/// directories return an empty set — the caller decides what to do.
fn collect_subpath_names_from_dir(output_dir: &Path) -> BTreeSet<String> {
    const BARREL_STEMS: &[&str] = &["index", "index.getter", "index.proxy"];
    let mut names: BTreeSet<String> = BTreeSet::new();
    let Ok(entries) = fs::read_dir(output_dir) else {
        return names;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        // Match `Foo.js` but NOT `Foo.d.ts` or `Foo.mjs`.
        let Some(stem) = fname.strip_suffix(".js") else {
            continue;
        };
        if BARREL_STEMS.iter().any(|b| stem == *b) {
            continue;
        }
        names.insert(stem.to_string());
    }
    names
}

fn strip_broken_imports(output_dir: &Path) -> Result<(), String> {
    use dynwinrt_codegen::codegen::project::get_import_name;
    use std::collections::HashSet as StdHashSet;

    let mut existing: StdHashSet<String> = StdHashSet::new();
    if let Ok(read_dir) = fs::read_dir(output_dir) {
        for entry in read_dir.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(stem) = name.strip_suffix(".js") {
                    existing.insert(stem.to_string());
                }
            }
        }
    }

    // If `--import-name ./runtime.js` was used, that stem is not one of the
    // emitted modules but must not be stripped.
    let runtime_name = get_import_name();
    if let Some(runtime_stem) = runtime_name
        .strip_prefix("./")
        .and_then(|s| s.strip_suffix(".js").or(Some(s)))
    {
        existing.insert(runtime_stem.to_string());
    }
    existing.insert("lifetime".to_string());

    // Three patterns to strip when the target sibling doesn't exist:
    //
    // 1. Legacy ESM import (in case render_esm output leaks through):
    //      import { X } from './Foo.js';
    //
    // 2. CJS lazy loader triplet emitted by convert_to_cjs_with_lazy (class files):
    //      let __m_Foo;
    //      const __load_Foo = () => (__m_Foo ??= require('./Foo.js'));
    //      const X = __lazy(__load_Foo, 'X');   // one per imported symbol
    //
    // 3. Index-level lazy exports emitted by esm_index_to_cjs_lazy:
    //      exports.X = undefined;
    //      Object.defineProperty(exports, 'X', { ... get() { return require('./Foo.js').X; } });
    //      Both lines reference the same missing target and must go together.
    let esm_import_re = regex::Regex::new(r#"(?m)^import \{[^}]*\} from '\./([^']+)\.js';\r?\n"#)
        .map_err(|e| format!("regex error: {}", e))?;

    // Class-file lazy loader block. New shape:
    //   let __m_Foo;
    //   const __load_Foo = () => (__m_Foo ??= require('./Foo.js'));
    //   const __get_X = () => __load_Foo().X;   // one per imported symbol
    let cjs_lazy_re = regex::Regex::new(
        r"(?ms)^let __m_[A-Za-z0-9_]+;\r?\nconst __load_[A-Za-z0-9_]+ = \(\) => \(__m_[A-Za-z0-9_]+ \?\?= require\('\./([^']+)\.js'\)\);\r?\n(?:const __get_[A-Za-z0-9_]+ = \(\) => __load_[A-Za-z0-9_]+\(\)\.[A-Za-z0-9_]+;\r?\n)*",
    )
    .map_err(|e| format!("regex error: {}", e))?;

    // Class-file eager destructured require:
    //   const { IID_X, X_PARAM_TYPES } = require('./Foo.js');
    // Emitted alongside the lazy block for symbols the native runtime needs
    // as concrete values (IIDs, DynWinRtType arrays, struct type descriptors).
    let cjs_eager_re =
        regex::Regex::new(r"(?m)^const \{[^}]+\} = require\('\./([^']+)\.js'\);\r?\n")
            .map_err(|e| format!("regex error: {}", e))?;

    // Index-file lazy export line emitted by `esm_index_to_cjs_lazy`:
    //   { let _m; exports.NAME = __lazy(() => (_m ??= require('./Foo.js')).NAME); }
    // captures the module basename in group 1.
    let index_dp_re = regex::Regex::new(
        r"(?m)^\{ let _m; exports\.[A-Za-z0-9_]+ = __lazy\(\(\) => \(_m \?\?= require\('\./([^']+)\.js'\)\)\.[A-Za-z0-9_]+\); \}\r?\n",
    )
    .map_err(|e| format!("regex error: {}", e))?;

    let read_dir = match fs::read_dir(output_dir) {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !name.ends_with(".js") {
            continue;
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let mut changed = false;

        // 1. CJS lazy loaders for missing targets (class files).
        let filtered = cjs_lazy_re.replace_all(&content, |caps: &regex::Captures| {
            let target = &caps[1];
            if existing.contains(target) {
                caps[0].to_string()
            } else {
                changed = true;
                String::new()
            }
        });

        // 1b. CJS eager destructured requires for missing sibling modules.
        let filtered = cjs_eager_re.replace_all(&filtered, |caps: &regex::Captures| {
            let target = &caps[1];
            if existing.contains(target) {
                caps[0].to_string()
            } else {
                changed = true;
                String::new()
            }
        });

        // 2. Index lazy-export lines. The new form has a single `{ let _m; ... }`
        // block per export; captures the module basename.
        let filtered = index_dp_re.replace_all(&filtered, |caps: &regex::Captures| {
            let target = &caps[1];
            if existing.contains(target) {
                caps[0].to_string()
            } else {
                changed = true;
                String::new()
            }
        });

        // 3. Legacy ESM imports (defence-in-depth if pipeline ever emits ESM).
        let filtered = esm_import_re.replace_all(&filtered, |caps: &regex::Captures| {
            let target = &caps[1];
            if existing.contains(target) {
                caps[0].to_string()
            } else {
                changed = true;
                String::new()
            }
        });
        if changed {
            fs::write(&path, filtered.as_bytes())
                .map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;
        }
    }
    Ok(())
}

fn generate_py_files(
    output_dir: &Path,
    all_classes: &[meta::ClassMeta],
    all_interfaces: &[meta::InterfaceMeta],
    all_enums: &[TypeMeta],
    shared_interfaces: &[meta::InterfaceMeta],
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
    shared_iids: &HashSet<String>,
    pyi: bool,
) -> Result<(), String> {
    use dynwinrt_codegen::codegen::python::to_snake_case_filename;
    use dynwinrt_codegen::codegen::python_stub;

    for iface in shared_interfaces {
        let code = python::generate_interface(iface, known_types, delegate_type_names);
        let filepath = output_dir.join(format!("{}.py", to_snake_case_filename(&iface.name)));
        write_file(&filepath, &code)?;
        println!("Generated shared {}", filepath.display());
        if pyi {
            let stub =
                python_stub::generate_interface_stub(iface, known_types, delegate_type_names);
            let p = output_dir.join(format!("{}.pyi", to_snake_case_filename(&iface.name)));
            write_file(&p, &stub)?;
        }
    }
    for iface in all_interfaces {
        let code = python::generate_interface(iface, known_types, delegate_type_names);
        let filepath = output_dir.join(format!("{}.py", to_snake_case_filename(&iface.name)));
        write_file(&filepath, &code)?;
        println!("Generated {}", filepath.display());
        if pyi {
            let stub =
                python_stub::generate_interface_stub(iface, known_types, delegate_type_names);
            let p = output_dir.join(format!("{}.pyi", to_snake_case_filename(&iface.name)));
            write_file(&p, &stub)?;
        }
    }
    for en in all_enums {
        if let TypeMeta::Enum { name, .. } = en {
            if let Some(code) = python::generate_enum(en) {
                let filepath = output_dir.join(format!("{}.py", to_snake_case_filename(name)));
                write_file(&filepath, &code)?;
                println!("Generated {}", filepath.display());
            }
            if pyi {
                if let Some(stub) = python_stub::generate_enum_stub(en) {
                    let p = output_dir.join(format!("{}.pyi", to_snake_case_filename(name)));
                    write_file(&p, &stub)?;
                }
            }
        }
    }
    for class in all_classes {
        let code = python::generate_class(class, known_types, delegate_type_names, shared_iids);
        let filepath = output_dir.join(format!("{}.py", to_snake_case_filename(&class.name)));
        write_file(&filepath, &code)?;
        println!("Generated {}", filepath.display());
        if pyi {
            let stub = python_stub::generate_class_stub(
                class,
                known_types,
                delegate_type_names,
                shared_iids,
            );
            let p = output_dir.join(format!("{}.pyi", to_snake_case_filename(&class.name)));
            write_file(&p, &stub)?;
        }
    }
    if pyi {
        let marker = output_dir.join("py.typed");
        write_file(&marker, "")?;
    }
    Ok(())
}

/// Write content to a file with a descriptive error message on failure.
fn write_file(path: &Path, content: &str) -> Result<(), String> {
    fs::write(path, content).map_err(|e| format!("Failed to write {}: {}", path.display(), e))
}

fn print_capabilities() {
    for capability in [
        "generate",
        "lang.js",
        "lang.py",
        "input.winmd",
        "input.ref",
        "input.winmd-list",
        "input.ref-list",
        "selector.namespace-class",
    ] {
        println!("{}", capability);
    }
}

/// Read a list file of newline-separated .winmd paths. Trims each line and skips
/// blank lines and '#' comments. Used by --winmd-list and --ref-list to avoid
/// command-line length limits when many winmds are passed.
fn read_path_list_file(path: &str) -> Result<Vec<String>, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read list file '{}': {}", path, e))?;
    Ok(content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect())
}

fn validate_winmd_paths(paths: &[String], label: &str) -> Result<(), String> {
    for path in paths {
        let metadata = fs::metadata(path)
            .map_err(|e| format!("{} path is not accessible: {} ({})", label, path, e))?;
        if !metadata.is_file() {
            return Err(format!("{} path is not a file: {}", label, path));
        }
    }
    Ok(())
}

fn list_namespaces_for_paths(paths: &[String]) -> Vec<String> {
    if paths.is_empty() {
        Vec::new()
    } else {
        meta::list_namespaces(&paths.join(";"))
    }
}

fn has_windows_namespace(namespaces: &[String]) -> bool {
    namespaces
        .iter()
        .any(|ns| ns == "Windows" || ns.starts_with("Windows."))
}

/// Find Windows SDK Windows.winmd by scanning the standard install location.
fn find_windows_sdk_winmd() -> Option<String> {
    let base = Path::new(r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata");
    if !base.exists() {
        return None;
    }
    let mut versions: Vec<_> = fs::read_dir(base)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with("10."))
        .collect();
    versions.sort();
    for version in versions.iter().rev() {
        let winmd_path = base.join(version).join("Windows.winmd");
        if winmd_path.exists() {
            return Some(winmd_path.to_string_lossy().to_string());
        }
    }
    None
}
