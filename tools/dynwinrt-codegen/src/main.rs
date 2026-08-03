// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use dynwinrt_codegen::codegen::com;
use dynwinrt_codegen::codegen::package;
use dynwinrt_codegen::codegen::python;
use dynwinrt_codegen::codegen::typescript;
use dynwinrt_codegen::codegen::winrt::extensions::winui;
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

        /// Generate bindings for specific class(es), comma-separated.
        /// Names may be qualified, or unqualified when --namespace is supplied.
        /// E.g. --class-name Uri or --class-name Windows.Foundation.Uri
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

fn parse_class_requests(
    class_names: &str,
    default_namespace: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    class_names
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            if let Some((namespace, class_name)) = name.rsplit_once('.') {
                if namespace.is_empty() || class_name.is_empty() {
                    return Err(format!("Invalid qualified class name: {name}"));
                }
                return Ok((namespace.to_string(), class_name.to_string()));
            }

            let namespace = default_namespace.ok_or_else(|| {
                format!(
                    "--namespace is required for unqualified class name `{name}`; \
                     alternatively pass its fully qualified metadata name"
                )
            })?;
            Ok((namespace.to_string(), name.to_string()))
        })
        .collect()
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

            let final_output_dir = Path::new(&output);
            let mut python_output = if lang == "py" && !dry_run {
                Some(PythonOutputTransaction::begin(final_output_dir)?)
            } else {
                None
            };
            let effective_output_dir = python_output
                .as_ref()
                .map(|transaction| transaction.stage_dir().to_path_buf())
                .unwrap_or_else(|| final_output_dir.to_path_buf());
            let output_dir = effective_output_dir.as_path();
            if lang == "js" {
                project::set_import_name(&import_name);
            }
            if !dry_run {
                fs::create_dir_all(output_dir).map_err(|e| {
                    format!("Failed to create output directory '{}': {}", output, e)
                })?;
                if lang == "js" {
                    migrate_legacy_com_only_package(output_dir)?;
                }
            }
            if lang == "py" && !dry_run && !pyi && class_name.is_some() {
                remove_all_generated_python_stubs(output_dir)?;
            }

            if let Some(ref cls_arg) = class_name {
                let class_requests = parse_class_requests(cls_arg, namespace.as_deref())?;

                // First: partition into WinRT classes and classic-COM interfaces.
                let mut classes = Vec::new();
                let mut com_interfaces: Vec<com_metadata::ComInterfaceMeta> = Vec::new();
                let mut com_coclasses: Vec<com_metadata::ComCoclassMeta> = Vec::new();
                for (ns, cls) in &class_requests {
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
                    if let Some(coclass) = com_metadata::parse_com_coclass(&winmd, ns, cls)? {
                        com_coclasses.push(coclass);
                        continue;
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

                // Fail loud: classic-COM codegen only emits `.js` + `.d.ts`
                // today. If the user asked for a different language
                // (e.g. `--lang py`) but any of the requested `--class-name`
                // inputs resolved to a classic-COM interface, silently writing
                // JS files into a Python output directory would produce the
                // wrong artifact types with no diagnostic. Reject the
                // combination up front.
                if lang != "js" && (!com_interfaces.is_empty() || !com_coclasses.is_empty()) {
                    let mut offenders: Vec<String> = Vec::new();
                    for ci in &com_interfaces {
                        offenders.push(format!(
                            "{}.{} (classic-COM interface)",
                            ci.interface.namespace, ci.interface.name
                        ));
                    }
                    for coclass in &com_coclasses {
                        offenders.push(format!(
                            "{}.{} (classic-COM coclass)",
                            coclass.namespace, coclass.name
                        ));
                    }
                    return Err(format!(
                        "`--lang {}` is not supported for classic-COM interfaces \
                         (they emit only `.js` + `.d.ts` today). \
                         Offending inputs: {}. Re-run with `--lang js`, or split the \
                         invocation so the WinRT classes are generated with `--lang {}` and \
                         the COM classes with `--lang js`.",
                        lang,
                        offenders.join(", "),
                        lang
                    ));
                }

                // Classic COM occupies its own ESM subpackage so its symbols
                // cannot collide with or leak into the WinRT root barrel.
                if !com_interfaces.is_empty() || !com_coclasses.is_empty() {
                    let com_output_dir = output_dir.join("com");

                    // Project every requested COM interface into memory FIRST,
                    // before writing anything. A later interface's projection
                    // failure (e.g. an unsupported native struct) must not
                    // leave an earlier interface's files newly written on
                    // disk — this batch is all-or-nothing. Pre-existing files
                    // from a prior, separate invocation (incremental
                    // generation) are untouched either way, since we never
                    // delete anything here.
                    let mut generated =
                        Vec::with_capacity(com_interfaces.len() + com_coclasses.len());
                    for com_iface in &com_interfaces {
                        let out =
                            com::generate_com_interface_files(com_iface, &winmd).map_err(|e| {
                                format!(
                                    "Classic-COM codegen for {} failed: {}",
                                    com_iface.interface.name, e
                                )
                            })?;
                        generated.push((com_iface.interface.name.clone(), out));
                    }
                    for coclass in &com_coclasses {
                        let out =
                            com::generate_com_coclass_files(coclass, &winmd).map_err(|e| {
                                format!(
                                    "Classic-COM coclass codegen for {} failed: {}",
                                    coclass.name, e
                                )
                            })?;
                        generated.push((coclass.name.clone(), out));
                    }

                    let mut planned_files = BTreeMap::new();
                    for (name, out) in &generated {
                        let mut files = vec![
                            (format!("{name}.js"), out.js.clone()),
                            (format!("{name}.d.ts"), out.dts.clone()),
                        ];
                        files.extend(out.extra_files.iter().cloned());
                        for (file_name, content) in files {
                            if let Some(existing) = planned_files.get(&file_name)
                                && existing != &content
                            {
                                return Err(format!(
                                    "Classic-COM generation produced conflicting `{file_name}` outputs; \
                                     request a single primary interface for each coclass"
                                ));
                            }
                            planned_files.insert(file_name, content);
                        }
                    }

                    if !dry_run {
                        fs::create_dir_all(&com_output_dir).map_err(|e| {
                            format!(
                                "Failed to create COM output directory '{}': {}",
                                com_output_dir.display(),
                                e
                            )
                        })?;
                        for (file_name, content) in &planned_files {
                            fs::write(com_output_dir.join(file_name), content)
                                .map_err(|e| format!("Failed to write {}: {}", file_name, e))?;
                        }
                    }
                    for (name, out) in &generated {
                        if dry_run {
                            println!("[dry-run] Would generate {}", name);
                        } else {
                            println!(
                                "Generated {} ({} .js/.d.ts + {} extras)",
                                name,
                                2,
                                out.extra_files.len()
                            );
                        }
                    }
                    if !dry_run {
                        write_com_js_barrel(&com_output_dir)?;
                    }

                    if classes.is_empty() {
                        if !dry_run {
                            finalize_com_generation(output_dir)?;
                        }
                        return Ok(());
                    }
                }

                winui::add_implicit_classes(&winmd, &mut classes);
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
                    let (append_fn, generate_fn): (AppendFn, GenerateFn) = if lang == "py" {
                        (python::append_to_index, python::generate_index)
                    } else {
                        // For JS lang, we use an in-memory `.ts` index then split into .js + .d.ts.
                        // We pick a sentinel filename `index.js` to detect presence; `.d.ts` is written alongside.
                        (typescript::append_to_index, typescript::generate_index)
                    };
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
                    let class_names: HashSet<String> =
                        all_classes.iter().map(|c| c.name.clone()).collect();
                    let class_identities: HashSet<(String, String)> = all_classes
                        .iter()
                        .map(|class| (class.namespace.clone(), class.name.clone()))
                        .collect();
                    all_interfaces.retain(|interface| {
                        !interface.iid.is_empty()
                            && if lang == "py" {
                                !class_identities.contains(&(
                                    interface.namespace.clone(),
                                    interface.name.clone(),
                                ))
                            } else {
                                !class_names.contains(&interface.name)
                            }
                    });
                    all_enums.retain(|e| match e {
                        TypeMeta::Enum {
                            namespace, name, ..
                        } => {
                            if lang == "py" {
                                !class_identities.contains(&(namespace.clone(), name.clone()))
                            } else {
                                !class_names.contains(name)
                            }
                        }
                        _ => true,
                    });
                    if lang == "py" {
                        write_python_package_indexes(
                            output_dir,
                            &all_classes,
                            &all_interfaces,
                            &all_enums,
                            pyi,
                            true,
                        )?;
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
                }
            } else {
                if lang == "py" && !dry_run {
                    clean_python_generated_output(output_dir)?;
                }
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
                    winui::add_implicit_classes(&winmd, &mut classes);
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
                    && (total_classes + total_interfaces + total_enums) > 0
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
                    let class_names: HashSet<String> =
                        all_classes.iter().map(|c| c.name.clone()).collect();
                    let class_identities: HashSet<(String, String)> = all_classes
                        .iter()
                        .map(|class| (class.namespace.clone(), class.name.clone()))
                        .collect();
                    all_interfaces.retain(|interface| {
                        !interface.iid.is_empty()
                            && if lang == "py" {
                                !class_identities.contains(&(
                                    interface.namespace.clone(),
                                    interface.name.clone(),
                                ))
                            } else {
                                !class_names.contains(&interface.name)
                            }
                    });
                    all_enums.retain(|e| match e {
                        TypeMeta::Enum {
                            namespace, name, ..
                        } => {
                            if lang == "py" {
                                !class_identities.contains(&(namespace.clone(), name.clone()))
                            } else {
                                !class_names.contains(name)
                            }
                        }
                        _ => true,
                    });

                    if lang == "py" {
                        write_python_package_indexes(
                            output_dir,
                            &all_classes,
                            &all_interfaces,
                            &all_enums,
                            pyi,
                            false,
                        )?;
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

            if lang == "py" && !dry_run {
                write_python_package_manifest(output_dir, final_output_dir)?;
                write_python_generated_inventory(output_dir, pyi)?;
                python_output
                    .take()
                    .expect("Python output transaction must exist")
                    .commit()?;
            }
        }
    }
    Ok(())
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
    if lang != "py" {
        validate_unique_class_output_names(&all_classes)?;
    }

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
    let class_identities_all: HashSet<(String, String)> = all_classes
        .iter()
        .map(|class| (class.namespace.clone(), class.name.clone()))
        .collect();
    let is_emittable_iface = |i: &meta::InterfaceMeta| -> bool {
        !i.iid.is_empty()
            && if lang == "py" {
                !class_identities_all.contains(&(i.namespace.clone(), i.name.clone()))
            } else {
                !class_names_all.contains(&i.name)
            }
    };
    let emittable_interfaces: Vec<meta::InterfaceMeta> = all_interfaces
        .iter()
        .filter(|i| is_emittable_iface(i))
        .cloned()
        .collect();
    let python_layout = if lang == "py" {
        Some(python::install_python_module_layout(
            python_type_identities(&all_classes, &emittable_interfaces, &all_enums),
        )?)
    } else {
        None
    };

    let mut known_types: HashSet<String> = HashSet::new();
    for c in &all_classes {
        known_types.insert(c.name.clone());
        known_types.insert(c.full_name.clone());
    }
    for i in &emittable_interfaces {
        known_types.insert(i.name.clone());
        known_types.insert(format!("{}.{}", i.namespace, i.name));
    }
    for e in &all_enums {
        if let TypeMeta::Enum {
            namespace, name, ..
        } = e
        {
            let is_class = if lang == "py" {
                class_identities_all.contains(&(namespace.clone(), name.clone()))
            } else {
                class_names_all.contains(name)
            };
            if !is_class {
                known_types.insert(name.clone());
                known_types.insert(format!("{namespace}.{name}"));
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
        drop(python_layout);
    }

    Ok((all_classes.len(), all_interfaces.len(), all_enums.len()))
}

fn python_type_identities(
    classes: &[meta::ClassMeta],
    interfaces: &[meta::InterfaceMeta],
    enums: &[TypeMeta],
) -> Vec<python::PythonTypeIdentity> {
    let mut identities = Vec::new();
    identities.extend(classes.iter().map(|class| python::PythonTypeIdentity {
        namespace: class.namespace.clone(),
        name: class.name.clone(),
    }));
    identities.extend(
        interfaces
            .iter()
            .map(|interface| python::PythonTypeIdentity {
                namespace: interface.namespace.clone(),
                name: interface.name.clone(),
            }),
    );
    identities.extend(enums.iter().filter_map(|typ| {
        let TypeMeta::Enum {
            namespace, name, ..
        } = typ
        else {
            return None;
        };
        Some(python::PythonTypeIdentity {
            namespace: namespace.clone(),
            name: name.clone(),
        })
    }));
    identities
}

fn validate_unique_class_output_names(classes: &[meta::ClassMeta]) -> Result<(), String> {
    let mut full_name_by_short_name: HashMap<&str, &str> = HashMap::new();
    for class in classes {
        match full_name_by_short_name.get(class.name.as_str()) {
            Some(existing) if *existing != class.full_name => {
                return Err(format!(
                    "Cannot generate `{}` and `{}` in one output directory because both use \
                     the short class name `{}`. Generate them separately or select only one type.",
                    existing, class.full_name, class.name
                ));
            }
            Some(_) => {}
            None => {
                full_name_by_short_name.insert(&class.name, &class.full_name);
            }
        }
    }
    Ok(())
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

    write_bindings_manifest(output_dir)?;

    println!("Generated {}", js_path.display());
    Ok(())
}

fn write_com_js_barrel(com_output_dir: &Path) -> Result<(), String> {
    let mut modules: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let entries = fs::read_dir(com_output_dir).map_err(|error| {
        format!(
            "Failed to read COM output directory {}: {error}",
            com_output_dir.display()
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
        let exports = collect_com_cjs_exports(&content);
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
    let cjs_index = typescript::esm_index_to_cjs_getter(&index);
    let esm_index = typescript::esm_index_to_esm(&index);
    fs::write(com_output_dir.join("index.js"), &cjs_index)
        .map_err(|error| format!("Failed to write COM index.js: {error}"))?;
    fs::write(com_output_dir.join("index.mjs"), &esm_index)
        .map_err(|error| format!("Failed to write COM index.mjs: {error}"))?;
    fs::write(com_output_dir.join("index.d.ts"), &index)
        .map_err(|error| format!("Failed to write COM index.d.ts: {error}"))?;

    let package = "{\n  \"type\": \"commonjs\",\n  \"sideEffects\": false\n}\n";
    fs::write(com_output_dir.join("package.json"), package)
        .map_err(|error| format!("Failed to write COM package boundary: {error}"))?;
    Ok(())
}

fn finalize_com_generation(output_dir: &Path) -> Result<(), String> {
    if !has_winrt_root(output_dir) {
        write_com_root_compatibility_barrels(output_dir)?;
    }
    write_bindings_manifest(output_dir)
}

fn write_com_root_compatibility_barrels(output_dir: &Path) -> Result<(), String> {
    let com_dir = output_dir.join("com");
    let com_js_path = com_dir.join("index.js");
    let com_dts_path = com_dir.join("index.d.ts");
    let com_js = fs::read_to_string(&com_js_path)
        .map_err(|error| format!("Failed to read {}: {error}", com_js_path.display()))?;
    let com_dts = fs::read_to_string(&com_dts_path)
        .map_err(|error| format!("Failed to read {}: {error}", com_dts_path.display()))?;
    let root_js = com_js.replace(", './", ", './com/");
    let root_dts = com_dts.replace("from './", "from './com/");
    fs::write(output_dir.join("index.js"), &root_js)
        .map_err(|error| format!("Failed to write COM compatibility index.js: {error}"))?;
    fs::write(output_dir.join("index.d.ts"), root_dts)
        .map_err(|error| format!("Failed to write COM compatibility index.d.ts: {error}"))?;
    Ok(())
}

fn migrate_legacy_com_only_package(output_dir: &Path) -> Result<(), String> {
    if has_winrt_root(output_dir) {
        return Ok(());
    }

    let package_path = output_dir.join("package.json");
    let index_path = output_dir.join("index.js");
    let index_dts_path = output_dir.join("index.d.ts");
    if !package_path.is_file() || !index_path.is_file() || !index_dts_path.is_file() {
        return Ok(());
    }
    let package = fs::read_to_string(&package_path)
        .map_err(|error| format!("Failed to read {}: {error}", package_path.display()))?;
    let index = fs::read_to_string(&index_dts_path)
        .map_err(|error| format!("Failed to read {}: {error}", index_dts_path.display()))?;
    if !package.contains("\"name\": \"@winapp/bindings\"")
        || !(package.contains("\"type\": \"module\"") || package.contains("\"type\": \"commonjs\""))
        || !index.starts_with("// Generated by dynwinrt-codegen")
    {
        return Ok(());
    }

    let modules = collect_com_index_modules(&index);
    if modules.is_empty() {
        return Ok(());
    }

    let com_output_dir = output_dir.join("com");
    fs::create_dir_all(&com_output_dir).map_err(|error| {
        format!(
            "Failed to create COM output directory '{}': {error}",
            com_output_dir.display()
        )
    })?;
    for module in modules {
        for suffix in [".js", ".d.ts"] {
            move_legacy_com_file(
                &output_dir.join(format!("{module}{suffix}")),
                &com_output_dir.join(format!("{module}{suffix}")),
            )?;
        }
    }

    for path in [
        output_dir.join("index.js"),
        output_dir.join("index.mjs"),
        output_dir.join("index.d.ts"),
        package_path,
    ] {
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|error| format!("Failed to remove {}: {error}", path.display()))?;
        }
    }
    write_com_js_barrel(&com_output_dir)?;
    finalize_com_generation(output_dir)
}

fn collect_com_index_modules(index: &str) -> BTreeSet<String> {
    index
        .lines()
        .filter_map(|line| {
            let (_, module) = line.split_once(" from './")?;
            let module = module.strip_suffix(".js';")?;
            (!module.is_empty() && !module.contains(['/', '\\'])).then(|| module.to_string())
        })
        .collect()
}

fn move_legacy_com_file(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.exists() {
        return Ok(());
    }
    if destination.exists() {
        let source_content = fs::read(source)
            .map_err(|error| format!("Failed to read {}: {error}", source.display()))?;
        let destination_content = fs::read(destination)
            .map_err(|error| format!("Failed to read {}: {error}", destination.display()))?;
        if source_content != destination_content {
            return Err(format!(
                "Cannot migrate legacy COM file {} because {} already exists with different content",
                source.display(),
                destination.display()
            ));
        }
        fs::remove_file(source)
            .map_err(|error| format!("Failed to remove {}: {error}", source.display()))?;
        return Ok(());
    }

    fs::rename(source, destination).map_err(|error| {
        format!(
            "Failed to migrate legacy COM file {} to {}: {error}",
            source.display(),
            destination.display()
        )
    })
}

fn has_winrt_root(output_dir: &Path) -> bool {
    ["index.mjs", "index.proxy.js", "lifetime.js"]
        .iter()
        .any(|name| output_dir.join(name).is_file())
}

fn write_bindings_manifest(output_dir: &Path) -> Result<(), String> {
    let has_winrt_root = has_winrt_root(output_dir);
    let winrt_subpath_names = if has_winrt_root {
        collect_subpath_names_from_dir(output_dir)
    } else {
        BTreeSet::new()
    };
    let com_subpath_names = collect_com_subpath_names(&output_dir.join("com"))?;
    let content = package::render_bindings_package_json(&package::BindingsPackageManifestInput {
        has_winrt_root,
        winrt_subpath_names: &winrt_subpath_names,
        com_subpath_names: &com_subpath_names,
    });
    let path = output_dir.join("package.json");
    fs::write(&path, content)
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))
}

fn collect_com_subpath_names(com_output_dir: &Path) -> Result<BTreeSet<String>, String> {
    let index_path = com_output_dir.join("index.d.ts");
    if !index_path.is_file() {
        return Ok(BTreeSet::new());
    }
    let index = fs::read_to_string(&index_path)
        .map_err(|error| format!("Failed to read {}: {error}", index_path.display()))?;
    Ok(collect_com_index_modules(&index))
}

fn collect_com_cjs_exports(content: &str) -> BTreeSet<String> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let rest = line.strip_prefix("exports.")?;
            let name = rest
                .chars()
                .take_while(|character| {
                    character.is_ascii_alphanumeric() || *character == '_' || *character == '$'
                })
                .collect::<String>();
            (!name.is_empty() && rest[name.len()..].trim_start().starts_with('=')).then_some(name)
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
    use dynwinrt_codegen::codegen::python_stub;

    for iface in shared_interfaces {
        let code = python::generate_interface(iface, known_types, delegate_type_names);
        let module = python::python_module_name(&iface.namespace, &iface.name);
        let filepath = output_dir.join(format!("{module}.py"));
        write_file(&filepath, &code)?;
        println!("Generated shared {}", filepath.display());
        if pyi {
            let stub =
                python_stub::generate_interface_stub(iface, known_types, delegate_type_names);
            let p = output_dir.join(format!("{module}.pyi"));
            write_file(&p, &stub)?;
        }
    }
    for iface in all_interfaces {
        let code = python::generate_interface(iface, known_types, delegate_type_names);
        let module = python::python_module_name(&iface.namespace, &iface.name);
        let filepath = output_dir.join(format!("{module}.py"));
        write_file(&filepath, &code)?;
        println!("Generated {}", filepath.display());
        if pyi {
            let stub =
                python_stub::generate_interface_stub(iface, known_types, delegate_type_names);
            let p = output_dir.join(format!("{module}.pyi"));
            write_file(&p, &stub)?;
        }
    }
    for en in all_enums {
        if let TypeMeta::Enum {
            namespace, name, ..
        } = en
        {
            let module = python::python_module_name(namespace, name);
            if let Some(code) = python::generate_enum(en) {
                let filepath = output_dir.join(format!("{module}.py"));
                write_file(&filepath, &code)?;
                println!("Generated {}", filepath.display());
            }
            if pyi {
                if let Some(stub) = python_stub::generate_enum_stub(en) {
                    let p = output_dir.join(format!("{module}.pyi"));
                    write_file(&p, &stub)?;
                }
            }
        }
    }
    for class in all_classes {
        let code = python::generate_class(class, known_types, delegate_type_names, shared_iids);
        let module = python::python_module_name(&class.namespace, &class.name);
        let filepath = output_dir.join(format!("{module}.py"));
        write_file(&filepath, &code)?;
        println!("Generated {}", filepath.display());
        if pyi {
            let stub = python_stub::generate_class_stub(
                class,
                known_types,
                delegate_type_names,
                shared_iids,
            );
            let p = output_dir.join(format!("{module}.pyi"));
            write_file(&p, &stub)?;
        }
    }
    if pyi {
        let marker = output_dir.join("py.typed");
        write_file(&marker, "")?;
    }
    Ok(())
}

#[derive(Default)]
struct PythonNamespaceGroup {
    classes: Vec<meta::ClassMeta>,
    interfaces: Vec<meta::InterfaceMeta>,
    enums: Vec<TypeMeta>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PythonGeneratedType {
    kind: String,
    identity: python::PythonTypeIdentity,
}

fn write_python_package_indexes(
    output_dir: &Path,
    classes: &[meta::ClassMeta],
    interfaces: &[meta::InterfaceMeta],
    enums: &[TypeMeta],
    pyi: bool,
    append: bool,
) -> Result<(), String> {
    use dynwinrt_codegen::codegen::python_stub;

    let current_types = python_generated_types(classes, interfaces, enums);
    let mut all_types = if append {
        read_python_type_inventory(output_dir)?
    } else {
        Vec::new()
    };
    all_types.extend(current_types);
    let mut seen_types = HashSet::new();
    all_types.retain(|typ| seen_types.insert(typ.clone()));

    let module_identities = all_types
        .iter()
        .filter(|typ| typ.kind != "struct")
        .map(|typ| typ.identity.clone())
        .collect::<Vec<_>>();
    let _layout = python::install_python_module_layout(module_identities.clone())?;
    validate_python_public_identities(&module_identities)?;

    let mut interface_counts = HashMap::<String, usize>::new();
    for typ in &all_types {
        if typ.kind == "interface" {
            *interface_counts
                .entry(typ.identity.name.clone())
                .or_default() += 1;
        }
    }
    if let Some((name, _)) = interface_counts.iter().find(|(_, count)| **count > 1) {
        return Err(format!(
            "Python generation cannot safely project multiple interfaces named `{name}`; \
             their delegate and generic runtime symbols are ambiguous"
        ));
    }

    let mut counts = HashMap::<String, usize>::new();
    for typ in &all_types {
        *counts.entry(typ.identity.name.clone()).or_default() += 1;
    }
    let suppressed_root_names = counts
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(name, _)| name.clone())
        .collect::<HashSet<_>>();
    let root_classes = classes
        .iter()
        .filter(|class| counts[&class.name] == 1)
        .cloned()
        .collect::<Vec<_>>();
    let root_interfaces = interfaces
        .iter()
        .filter(|interface| counts[&interface.name] == 1)
        .cloned()
        .collect::<Vec<_>>();
    let root_enums = enums
        .iter()
        .filter(|typ| matches!(typ, TypeMeta::Enum { name, .. } if counts[name] == 1))
        .cloned()
        .collect::<Vec<_>>();

    let root_index = python::generate_index(&root_classes, &root_interfaces, &root_enums);
    write_python_index(
        &output_dir.join("__init__.py"),
        &root_index,
        append,
        &suppressed_root_names,
    )?;
    if pyi {
        let root_stub =
            python_stub::generate_index_stub(&root_classes, &root_interfaces, &root_enums);
        write_python_index(
            &output_dir.join("__init__.pyi"),
            &root_stub,
            append,
            &suppressed_root_names,
        )?;
        write_file(&output_dir.join("py.typed"), "")?;
    }

    let mut groups = BTreeMap::<String, PythonNamespaceGroup>::new();
    for class in classes {
        groups
            .entry(class.namespace.clone())
            .or_default()
            .classes
            .push(class.clone());
    }
    for interface in interfaces {
        groups
            .entry(interface.namespace.clone())
            .or_default()
            .interfaces
            .push(interface.clone());
    }
    for typ in enums {
        if let TypeMeta::Enum { namespace, .. } = typ {
            groups
                .entry(namespace.clone())
                .or_default()
                .enums
                .push(typ.clone());
        }
    }

    for (namespace, group) in groups {
        write_python_namespace_group(output_dir, &namespace, &group, pyi, append)?;
    }
    write_python_type_inventory(output_dir, &all_types)?;
    Ok(())
}

fn write_python_namespace_group(
    output_dir: &Path,
    namespace: &str,
    group: &PythonNamespaceGroup,
    pyi: bool,
    append: bool,
) -> Result<(), String> {
    use dynwinrt_codegen::codegen::python_stub;

    let segments = python::python_namespace_segments(namespace);
    if segments.is_empty() {
        return Ok(());
    }
    let mut package_dir = output_dir.to_path_buf();
    for segment in &segments {
        package_dir.push(segment);
        fs::create_dir_all(&package_dir)
            .map_err(|e| format!("Failed to create {}: {}", package_dir.display(), e))?;
        let runtime_init = package_dir.join("__init__.py");
        if !runtime_init.exists() {
            write_file(&runtime_init, GENERATED_PYTHON_HEADER)?;
        }
        if pyi {
            let stub_init = package_dir.join("__init__.pyi");
            if !stub_init.exists() {
                write_file(&stub_init, GENERATED_PYTHON_HEADER)?;
            }
        }
    }

    let mut runtime_exports = Vec::new();
    let mut stub_exports = Vec::new();
    let mut seen = HashSet::new();

    for class in &group.classes {
        if !seen.insert(class.name.clone()) {
            continue;
        }
        let runtime = python::generate_index(std::slice::from_ref(class), &[], &[]);
        write_python_facade(
            &package_dir,
            &segments,
            &class.name,
            &runtime,
            "py",
            &mut runtime_exports,
        )?;
        if pyi {
            let stub = python_stub::generate_index_stub(std::slice::from_ref(class), &[], &[]);
            write_python_facade(
                &package_dir,
                &segments,
                &class.name,
                &stub,
                "pyi",
                &mut stub_exports,
            )?;
        }
    }
    for interface in &group.interfaces {
        if !seen.insert(interface.name.clone()) {
            continue;
        }
        let runtime = python::generate_index(&[], std::slice::from_ref(interface), &[]);
        write_python_facade(
            &package_dir,
            &segments,
            &interface.name,
            &runtime,
            "py",
            &mut runtime_exports,
        )?;
        if pyi {
            let stub = python_stub::generate_index_stub(&[], std::slice::from_ref(interface), &[]);
            write_python_facade(
                &package_dir,
                &segments,
                &interface.name,
                &stub,
                "pyi",
                &mut stub_exports,
            )?;
        }
    }
    for typ in &group.enums {
        let TypeMeta::Enum { name, .. } = typ else {
            continue;
        };
        if !seen.insert(name.clone()) {
            continue;
        }
        let runtime = python::generate_index(&[], &[], std::slice::from_ref(typ));
        write_python_facade(
            &package_dir,
            &segments,
            name,
            &runtime,
            "py",
            &mut runtime_exports,
        )?;
        if pyi {
            let stub = python_stub::generate_index_stub(&[], &[], std::slice::from_ref(typ));
            write_python_facade(
                &package_dir,
                &segments,
                name,
                &stub,
                "pyi",
                &mut stub_exports,
            )?;
        }
    }

    let runtime_index = format!("{}{}", GENERATED_PYTHON_HEADER, runtime_exports.join("\n"));
    let suppressed_root_names = HashSet::new();
    write_python_index(
        &package_dir.join("__init__.py"),
        &runtime_index,
        append,
        &suppressed_root_names,
    )?;
    if pyi {
        let stub_index = format!("{}{}", GENERATED_PYTHON_HEADER, stub_exports.join("\n"));
        write_python_index(
            &package_dir.join("__init__.pyi"),
            &stub_index,
            append,
            &suppressed_root_names,
        )?;
    }
    Ok(())
}

fn write_python_facade(
    package_dir: &Path,
    namespace_segments: &[String],
    type_name: &str,
    implementation_index: &str,
    extension: &str,
    package_exports: &mut Vec<String>,
) -> Result<(), String> {
    let import_line = implementation_index
        .lines()
        .find(|line| line.starts_with("from ."))
        .ok_or_else(|| format!("Generated index for `{type_name}` has no import"))?;
    let (source, exports) = import_line
        .strip_prefix("from .")
        .and_then(|line| line.split_once(" import "))
        .ok_or_else(|| format!("Generated index import for `{type_name}` is invalid"))?;
    let exports = if extension == "pyi" {
        exports
            .split(',')
            .map(|export| {
                let export = export.trim();
                if export.contains(" as ") {
                    export.to_string()
                } else {
                    format!("{export} as {export}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        exports.to_string()
    };
    let relative_root = ".".repeat(namespace_segments.len() + 1);
    let facade =
        format!("{GENERATED_PYTHON_HEADER}from {relative_root}{source} import {exports}\n");
    let public_module = python::python_public_module_name(type_name);
    write_file(
        &package_dir.join(format!("{public_module}.{extension}")),
        &facade,
    )?;
    package_exports.push(format!("from .{public_module} import {exports}"));
    Ok(())
}

fn write_python_index(
    path: &Path,
    generated: &str,
    append: bool,
    suppressed_names: &HashSet<String>,
) -> Result<(), String> {
    let content = if append && path.exists() {
        let existing = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        merge_python_indexes(&existing, generated, suppressed_names)
    } else {
        merge_python_indexes(GENERATED_PYTHON_HEADER, generated, suppressed_names)
    };
    write_file(path, &content)
}

fn merge_python_indexes(
    existing: &str,
    generated: &str,
    suppressed_names: &HashSet<String>,
) -> String {
    let mut imports = BTreeSet::new();
    for line in existing.lines().chain(generated.lines()) {
        if line.starts_with("from .") {
            let (line, comment) = line
                .split_once("  #")
                .map_or((line, None), |(line, comment)| (line, Some(comment)));
            let Some((source, exports)) = line.split_once(" import ") else {
                continue;
            };
            let exports = exports
                .split(',')
                .map(str::trim)
                .filter(|export| {
                    let symbol = export
                        .split_once(" as ")
                        .map_or(*export, |(name, _)| name.trim());
                    !suppressed_names.contains(symbol)
                })
                .collect::<Vec<_>>();
            if !exports.is_empty() {
                let mut import = format!("{source} import {}", exports.join(", "));
                if let Some(comment) = comment {
                    import.push_str("  #");
                    import.push_str(comment);
                }
                imports.insert(import);
            }
        }
    }
    let mut merged = GENERATED_PYTHON_HEADER.to_string();
    for import in imports {
        merged.push_str(&import);
        merged.push('\n');
    }
    merged
}

const GENERATED_PYTHON_HEADER: &str = "# Generated by dynwinrt-codegen — do not edit\n";

fn python_generated_types(
    classes: &[meta::ClassMeta],
    interfaces: &[meta::InterfaceMeta],
    enums: &[TypeMeta],
) -> Vec<PythonGeneratedType> {
    let mut types = Vec::new();
    types.extend(classes.iter().map(|class| PythonGeneratedType {
        kind: "class".into(),
        identity: python::PythonTypeIdentity {
            namespace: class.namespace.clone(),
            name: class.name.clone(),
        },
    }));
    types.extend(interfaces.iter().map(|interface| PythonGeneratedType {
        kind: "interface".into(),
        identity: python::PythonTypeIdentity {
            namespace: interface.namespace.clone(),
            name: interface.name.clone(),
        },
    }));
    types.extend(enums.iter().filter_map(|typ| {
        let TypeMeta::Enum {
            namespace, name, ..
        } = typ
        else {
            return None;
        };
        Some(PythonGeneratedType {
            kind: "enum".into(),
            identity: python::PythonTypeIdentity {
                namespace: namespace.clone(),
                name: name.clone(),
            },
        })
    }));
    types.extend(
        python::package_struct_identities(classes, interfaces)
            .into_iter()
            .map(|(namespace, name)| PythonGeneratedType {
                kind: "struct".into(),
                identity: python::PythonTypeIdentity { namespace, name },
            }),
    );
    types
}

fn read_python_type_inventory(output_dir: &Path) -> Result<Vec<PythonGeneratedType>, String> {
    let path = output_dir.join(PYTHON_TYPE_INVENTORY);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut parts = line.splitn(3, '|');
            let kind = parts.next().unwrap_or_default();
            let namespace = parts.next().unwrap_or_default();
            let name = parts.next().unwrap_or_default();
            if !matches!(kind, "class" | "interface" | "enum" | "struct")
                || namespace.is_empty()
                || name.is_empty()
            {
                return Err(format!("Invalid generated type inventory entry `{line}`"));
            }
            Ok(PythonGeneratedType {
                kind: kind.into(),
                identity: python::PythonTypeIdentity {
                    namespace: namespace.into(),
                    name: name.into(),
                },
            })
        })
        .collect()
}

fn write_python_type_inventory(
    output_dir: &Path,
    types: &[PythonGeneratedType],
) -> Result<(), String> {
    let mut lines = types
        .iter()
        .map(|typ| {
            format!(
                "{}|{}|{}",
                typ.kind, typ.identity.namespace, typ.identity.name
            )
        })
        .collect::<Vec<_>>();
    lines.sort();
    lines.dedup();
    write_file(
        &output_dir.join(PYTHON_TYPE_INVENTORY),
        &format!("{}\n", lines.join("\n")),
    )
}

#[cfg(test)]
fn validate_python_public_paths(
    classes: &[meta::ClassMeta],
    interfaces: &[meta::InterfaceMeta],
    enums: &[TypeMeta],
) -> Result<(), String> {
    let identities = python_type_identities(classes, interfaces, enums);
    validate_python_public_identities(&identities)
}

fn validate_python_public_identities(
    identities: &[python::PythonTypeIdentity],
) -> Result<(), String> {
    let mut namespace_owners = HashMap::<String, String>::new();
    let mut namespace_paths = HashSet::new();
    for identity in identities {
        let segments = python::python_namespace_segments(&identity.namespace);
        let normalized = segments.join("/");
        if let Some(existing) =
            namespace_owners.insert(normalized.clone(), identity.namespace.clone())
        {
            if existing != identity.namespace {
                return Err(format!(
                    "Python namespace collision: `{existing}` and `{}` both normalize to `{normalized}`",
                    identity.namespace
                ));
            }
        }
        for depth in 1..=segments.len() {
            namespace_paths.insert(segments[..depth].join("/"));
        }
    }

    let mut module_owners = HashMap::<String, python::PythonTypeIdentity>::new();
    for identity in identities {
        let mut segments = python::python_namespace_segments(&identity.namespace);
        segments.push(python::python_public_module_name(&identity.name));
        let module_path = segments.join("/");
        if namespace_paths.contains(&module_path) {
            return Err(format!(
                "Python package/module collision: `{}.{}` normalizes to package path `{module_path}`",
                identity.namespace, identity.name
            ));
        }
        if let Some(existing) = module_owners.insert(module_path.clone(), identity.clone()) {
            if existing != *identity {
                return Err(format!(
                    "Python module collision: `{}.{}` and `{}.{}` both normalize to `{module_path}.py`",
                    existing.namespace, existing.name, identity.namespace, identity.name
                ));
            }
        }
    }
    Ok(())
}

struct PythonOutputTransaction {
    final_dir: PathBuf,
    stage_dir: PathBuf,
    backup_dir: PathBuf,
    committed: bool,
}

impl PythonOutputTransaction {
    fn begin(final_dir: &Path) -> Result<Self, String> {
        let parent = final_dir.parent().unwrap_or_else(|| Path::new("."));
        let leaf = final_dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("Invalid Python output directory '{}'", final_dir.display()))?;
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;

        let suffix = std::process::id();
        let stage_dir = parent.join(format!(".{}.dynwinrt-stage-{}", leaf, suffix));
        let backup_dir = parent.join(format!(".{}.dynwinrt-backup-{}", leaf, suffix));
        remove_transaction_dir(&stage_dir)?;
        remove_transaction_dir(&backup_dir)?;

        if final_dir.exists() {
            if !final_dir.is_dir() {
                return Err(format!(
                    "Python output path '{}' is not a directory",
                    final_dir.display()
                ));
            }
            copy_directory(final_dir, &stage_dir)?;
        } else {
            fs::create_dir_all(&stage_dir)
                .map_err(|e| format!("Failed to create {}: {}", stage_dir.display(), e))?;
        }

        Ok(Self {
            final_dir: final_dir.to_path_buf(),
            stage_dir,
            backup_dir,
            committed: false,
        })
    }

    fn stage_dir(&self) -> &Path {
        &self.stage_dir
    }

    fn commit(mut self) -> Result<(), String> {
        let had_existing_output = self.final_dir.exists();
        if had_existing_output {
            fs::rename(&self.final_dir, &self.backup_dir).map_err(|e| {
                format!(
                    "Failed to stage existing output '{}' for replacement: {}",
                    self.final_dir.display(),
                    e
                )
            })?;
        }

        if let Err(error) = fs::rename(&self.stage_dir, &self.final_dir) {
            if had_existing_output {
                if let Err(rollback_error) = fs::rename(&self.backup_dir, &self.final_dir) {
                    return Err(format!(
                        "Failed to replace Python output directory '{}': {}. Rollback also failed: \
                         {}. The original output remains at '{}'",
                        self.final_dir.display(),
                        error,
                        rollback_error,
                        self.backup_dir.display()
                    ));
                }
            }
            return Err(format!(
                "Failed to replace Python output directory '{}': {}",
                self.final_dir.display(),
                error
            ));
        }

        self.committed = true;
        if had_existing_output {
            fs::remove_dir_all(&self.backup_dir).map_err(|e| {
                format!(
                    "Replaced Python output but failed to remove backup '{}': {}",
                    self.backup_dir.display(),
                    e
                )
            })?;
        }
        Ok(())
    }
}

impl Drop for PythonOutputTransaction {
    fn drop(&mut self) {
        if !self.committed && self.stage_dir.exists() {
            let _ = fs::remove_dir_all(&self.stage_dir);
        }
    }
}

fn remove_transaction_dir(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_dir_all(path)
            .map_err(|e| format!("Failed to remove stale {}: {}", path.display(), e))?;
    }
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|e| format!("Failed to create {}: {}", destination.display(), e))?;
    for entry in
        fs::read_dir(source).map_err(|e| format!("Failed to read {}: {}", source.display(), e))?
    {
        let entry =
            entry.map_err(|e| format!("Failed to read entry in {}: {}", source.display(), e))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|e| format!("Failed to inspect {}: {}", source_path.display(), e))?;
        if file_type.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).map_err(|e| {
                format!(
                    "Failed to copy {} to {}: {}",
                    source_path.display(),
                    destination_path.display(),
                    e
                )
            })?;
        } else {
            return Err(format!(
                "Unsupported filesystem entry in Python output: {}",
                source_path.display()
            ));
        }
    }
    Ok(())
}

fn write_python_package_manifest(output_dir: &Path, final_output_dir: &Path) -> Result<(), String> {
    let manifest_path = output_dir.join("pyproject.toml");
    if manifest_path.exists()
        && !python_inventory_contains(output_dir, Path::new("pyproject.toml"))?
    {
        return Err(format!(
            "Refusing to overwrite existing non-generated manifest '{}'",
            final_output_dir.join("pyproject.toml").display()
        ));
    }
    let leaf = final_output_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            format!(
                "Cannot derive Python package name from '{}'",
                final_output_dir.display()
            )
        })?;
    let import_name = normalize_python_package_name(leaf);
    let distribution_name = import_name.replace('_', "-");
    let version = env!("CARGO_PKG_VERSION");
    let namespace_packages = collect_python_namespace_packages(output_dir)?;
    let manifest = package::render_python_pyproject(&package::PythonPackageManifestInput {
        distribution_name: &distribution_name,
        import_name: &import_name,
        package_version: version,
        runtime_version: version,
        namespace_packages: &namespace_packages,
    });
    write_file(&manifest_path, &manifest)
}

const PYTHON_GENERATED_INVENTORY: &str = ".dynwinrt-generated-files";
const PYTHON_TYPE_INVENTORY: &str = ".dynwinrt-generated-types";

fn python_inventory_contains(output_dir: &Path, relative_path: &Path) -> Result<bool, String> {
    let inventory_path = output_dir.join(PYTHON_GENERATED_INVENTORY);
    if !inventory_path.is_file() {
        return Ok(false);
    }
    Ok(fs::read_to_string(&inventory_path)
        .map_err(|e| format!("Failed to read {}: {}", inventory_path.display(), e))?
        .lines()
        .map(Path::new)
        .any(|path| path == relative_path))
}

fn remove_all_generated_python_stubs(output_dir: &Path) -> Result<(), String> {
    let inventory_path = output_dir.join(PYTHON_GENERATED_INVENTORY);
    if inventory_path.is_file() {
        for relative in fs::read_to_string(&inventory_path)
            .map_err(|e| format!("Failed to read {}: {}", inventory_path.display(), e))?
            .lines()
            .map(PathBuf::from)
        {
            if !is_safe_relative_path(&relative) {
                return Err(format!(
                    "Invalid path `{}` in {}",
                    relative.display(),
                    inventory_path.display()
                ));
            }
            let is_stub = relative
                .extension()
                .is_some_and(|extension| extension == "pyi")
                || relative.file_name().is_some_and(|name| name == "py.typed");
            let path = output_dir.join(relative);
            if is_stub && path.is_file() {
                fs::remove_file(&path)
                    .map_err(|e| format!("Failed to remove {}: {}", path.display(), e))?;
            }
        }
        return Ok(());
    }

    fn visit(current: &Path) -> Result<(), String> {
        for entry in fs::read_dir(current)
            .map_err(|e| format!("Failed to inspect {}: {}", current.display(), e))?
        {
            let entry = entry
                .map_err(|e| format!("Failed to inspect entry in {}: {}", current.display(), e))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|e| format!("Failed to inspect {}: {}", path.display(), e))?;
            if file_type.is_dir() {
                visit(&path)?;
            } else if file_type.is_file()
                && path.extension().is_some_and(|extension| extension == "pyi")
            {
                let generated = fs::read_to_string(&path)
                    .map_err(|e| format!("Failed to inspect {}: {}", path.display(), e))?
                    .starts_with("# Generated by dynwinrt-codegen");
                if generated {
                    fs::remove_file(&path)
                        .map_err(|e| format!("Failed to remove {}: {}", path.display(), e))?;
                }
            }
        }
        Ok(())
    }
    visit(output_dir)
}

fn clean_python_generated_output(output_dir: &Path) -> Result<(), String> {
    let inventory_path = output_dir.join(PYTHON_GENERATED_INVENTORY);
    let files = if inventory_path.is_file() {
        fs::read_to_string(&inventory_path)
            .map_err(|e| format!("Failed to read {}: {}", inventory_path.display(), e))?
            .lines()
            .map(PathBuf::from)
            .collect::<Vec<_>>()
    } else {
        collect_generated_python_files(output_dir, false, false)?
    };

    let mut parent_dirs = HashSet::new();
    for relative in files {
        if !is_safe_relative_path(&relative) {
            return Err(format!(
                "Invalid path `{}` in {}",
                relative.display(),
                inventory_path.display()
            ));
        }
        let path = output_dir.join(&relative);
        if path.is_file() {
            fs::remove_file(&path)
                .map_err(|e| format!("Failed to remove stale {}: {}", path.display(), e))?;
        }
        if let Some(parent) = relative.parent() {
            if !parent.as_os_str().is_empty() {
                parent_dirs.insert(parent.to_path_buf());
            }
        }
    }
    if inventory_path.is_file() {
        fs::remove_file(&inventory_path)
            .map_err(|e| format!("Failed to remove {}: {}", inventory_path.display(), e))?;
    }
    let type_inventory = output_dir.join(PYTHON_TYPE_INVENTORY);
    if type_inventory.is_file() {
        fs::remove_file(&type_inventory)
            .map_err(|e| format!("Failed to remove {}: {}", type_inventory.display(), e))?;
    }

    let mut parent_dirs = parent_dirs.into_iter().collect::<Vec<_>>();
    parent_dirs.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for relative in parent_dirs {
        let path = output_dir.join(relative);
        if path.is_dir() {
            match fs::remove_dir(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                Err(error) => {
                    return Err(format!(
                        "Failed to remove stale package directory {}: {}",
                        path.display(),
                        error
                    ));
                }
            }
        }
    }
    Ok(())
}

fn write_python_generated_inventory(output_dir: &Path, pyi: bool) -> Result<(), String> {
    let files = collect_generated_python_files(output_dir, true, pyi)?;
    let content = files
        .iter()
        .map(|path| path.to_string_lossy())
        .collect::<Vec<_>>()
        .join("\n");
    write_file(
        &output_dir.join(PYTHON_GENERATED_INVENTORY),
        &format!("{content}\n"),
    )
}

fn collect_generated_python_files(
    output_dir: &Path,
    include_root_manifest: bool,
    include_root_marker: bool,
) -> Result<Vec<PathBuf>, String> {
    fn visit(
        root: &Path,
        current: &Path,
        include_root_manifest: bool,
        include_root_marker: bool,
        files: &mut Vec<PathBuf>,
    ) -> Result<(), String> {
        for entry in fs::read_dir(current)
            .map_err(|e| format!("Failed to inspect {}: {}", current.display(), e))?
        {
            let entry = entry
                .map_err(|e| format!("Failed to inspect entry in {}: {}", current.display(), e))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|e| format!("Failed to inspect {}: {}", path.display(), e))?;
            if file_type.is_dir() {
                visit(
                    root,
                    &path,
                    include_root_manifest,
                    include_root_marker,
                    files,
                )?;
                continue;
            }
            if !file_type.is_file() || entry.file_name() == PYTHON_GENERATED_INVENTORY {
                continue;
            }

            let generated = match entry.file_name().to_str() {
                Some("py.typed") => include_root_marker && current == root,
                Some("pyproject.toml") => include_root_manifest && current == root,
                Some(name) if name.ends_with(".py") || name.ends_with(".pyi") => {
                    fs::read_to_string(&path)
                        .map_err(|e| format!("Failed to inspect {}: {}", path.display(), e))?
                        .starts_with("# Generated by dynwinrt-codegen")
                }
                _ => false,
            };
            if generated {
                files.push(path.strip_prefix(root).unwrap().to_path_buf());
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    visit(
        output_dir,
        output_dir,
        include_root_manifest,
        include_root_marker,
        &mut files,
    )?;
    files.sort();
    Ok(files)
}

fn is_safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn collect_python_namespace_packages(output_dir: &Path) -> Result<Vec<String>, String> {
    fn visit(root: &Path, current: &Path, packages: &mut Vec<String>) -> Result<(), String> {
        for entry in fs::read_dir(current)
            .map_err(|e| format!("Failed to inspect {}: {}", current.display(), e))?
        {
            let entry = entry
                .map_err(|e| format!("Failed to inspect entry in {}: {}", current.display(), e))?;
            if !entry
                .file_type()
                .map_err(|e| format!("Failed to inspect {}: {}", entry.path().display(), e))?
                .is_dir()
            {
                continue;
            }
            let path = entry.path();
            if path.join("__init__.py").is_file() {
                let relative = path.strip_prefix(root).map_err(|e| {
                    format!("Failed to normalize package path {}: {}", path.display(), e)
                })?;
                packages.push(
                    relative
                        .components()
                        .map(|component| component.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("."),
                );
            }
            visit(root, &path, packages)?;
        }
        Ok(())
    }

    let mut packages = Vec::new();
    visit(output_dir, output_dir, &mut packages)?;
    packages.sort();
    Ok(packages)
}

fn normalize_python_package_name(name: &str) -> String {
    let mut normalized = String::with_capacity(name.len());
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            normalized.push(character.to_ascii_lowercase());
        } else {
            normalized.push('_');
        }
    }
    if normalized.is_empty() || normalized.starts_with(|character: char| character.is_ascii_digit())
    {
        normalized.insert(0, '_');
    }
    normalized
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn test_directory(name: &str) -> PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "dynwinrt-codegen-{name}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn distinct_classes_with_the_same_short_name_are_rejected() {
        let classes = vec![
            meta::ClassMeta {
                name: "ResourceManager".into(),
                namespace: "Contoso".into(),
                full_name: "Contoso.ResourceManager".into(),
                ..Default::default()
            },
            meta::ClassMeta {
                name: "ResourceManager".into(),
                namespace: "Microsoft.Windows.ApplicationModel.Resources".into(),
                full_name: "Microsoft.Windows.ApplicationModel.Resources.ResourceManager".into(),
                ..Default::default()
            },
        ];

        let error = validate_unique_class_output_names(&classes)
            .expect_err("same-name classes must not overwrite each other");
        assert!(error.contains("Contoso.ResourceManager"));
        assert!(error.contains("Microsoft.Windows.ApplicationModel.Resources.ResourceManager"));
        assert!(error.contains("short class name `ResourceManager`"));
    }

    #[test]
    fn duplicate_metadata_for_the_same_class_is_allowed() {
        let class = meta::ClassMeta {
            name: "Application".into(),
            namespace: "Microsoft.UI.Xaml".into(),
            full_name: "Microsoft.UI.Xaml.Application".into(),
            ..Default::default()
        };

        validate_unique_class_output_names(&[class.clone(), class])
            .expect("identical metadata does not create an ambiguous output");
    }

    #[test]
    fn python_duplicate_short_names_use_namespace_facades() {
        let output = test_directory("namespace-facades");
        fs::create_dir_all(&output).unwrap();
        let classes = vec![
            meta::ClassMeta {
                name: "ResourceManager".into(),
                namespace: "Contoso.Resources".into(),
                full_name: "Contoso.Resources.ResourceManager".into(),
                ..Default::default()
            },
            meta::ClassMeta {
                name: "ResourceManager".into(),
                namespace: "Fabrikam.Resources".into(),
                full_name: "Fabrikam.Resources.ResourceManager".into(),
                ..Default::default()
            },
        ];

        write_python_package_indexes(&output, &classes, &[], &[], true, false).unwrap();

        let root = fs::read_to_string(output.join("__init__.py")).unwrap();
        assert!(!root.contains("ResourceManager"));
        let contoso = fs::read_to_string(
            output
                .join("contoso")
                .join("resources")
                .join("resource_manager.py"),
        )
        .unwrap();
        assert!(
            contoso.contains("from ...contoso__resources__resource_manager import ResourceManager")
        );
        let fabrikam = fs::read_to_string(
            output
                .join("fabrikam")
                .join("resources")
                .join("resource_manager.py"),
        )
        .unwrap();
        assert!(
            fabrikam
                .contains("from ...fabrikam__resources__resource_manager import ResourceManager")
        );

        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn incremental_python_generation_removes_ambiguous_root_export() {
        let output = test_directory("incremental-root");
        fs::create_dir_all(&output).unwrap();
        let contoso = meta::ClassMeta {
            name: "ResourceManager".into(),
            namespace: "Contoso.Resources".into(),
            full_name: "Contoso.Resources.ResourceManager".into(),
            ..Default::default()
        };
        let fabrikam = meta::ClassMeta {
            name: "ResourceManager".into(),
            namespace: "Fabrikam.Resources".into(),
            full_name: "Fabrikam.Resources.ResourceManager".into(),
            ..Default::default()
        };

        write_python_package_indexes(
            &output,
            std::slice::from_ref(&contoso),
            &[],
            &[],
            true,
            true,
        )
        .unwrap();
        assert!(
            fs::read_to_string(output.join("__init__.py"))
                .unwrap()
                .contains("ResourceManager")
        );

        write_python_package_indexes(
            &output,
            std::slice::from_ref(&fabrikam),
            &[],
            &[],
            true,
            true,
        )
        .unwrap();
        assert!(
            !fs::read_to_string(output.join("__init__.py"))
                .unwrap()
                .contains("ResourceManager")
        );
        assert!(
            output
                .join("contoso")
                .join("resources")
                .join("resource_manager.py")
                .is_file()
        );
        assert!(
            output
                .join("fabrikam")
                .join("resources")
                .join("resource_manager.py")
                .is_file()
        );

        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn python_package_module_collisions_are_rejected() {
        let classes = vec![
            meta::ClassMeta {
                name: "Controls".into(),
                namespace: "Contoso".into(),
                full_name: "Contoso.Controls".into(),
                ..Default::default()
            },
            meta::ClassMeta {
                name: "Button".into(),
                namespace: "Contoso.Controls".into(),
                full_name: "Contoso.Controls.Button".into(),
                ..Default::default()
            },
        ];

        let error = validate_python_public_paths(&classes, &[], &[])
            .expect_err("module/package collisions must fail closed");
        assert!(error.contains("package/module collision"));
        assert!(error.contains("contoso/controls"));
    }

    #[test]
    fn python_output_transaction_commits_complete_tree() {
        let output = test_directory("transaction-commit");
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("existing.py"), "old").unwrap();

        let transaction = PythonOutputTransaction::begin(&output).unwrap();
        fs::write(transaction.stage_dir().join("existing.py"), "new").unwrap();
        fs::write(transaction.stage_dir().join("added.py"), "added").unwrap();
        transaction.commit().unwrap();

        assert_eq!(
            fs::read_to_string(output.join("existing.py")).unwrap(),
            "new"
        );
        assert_eq!(
            fs::read_to_string(output.join("added.py")).unwrap(),
            "added"
        );
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn dropped_python_output_transaction_preserves_existing_output() {
        let output = test_directory("transaction-drop");
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("existing.py"), "old").unwrap();

        {
            let transaction = PythonOutputTransaction::begin(&output).unwrap();
            fs::write(transaction.stage_dir().join("existing.py"), "new").unwrap();
        }

        assert_eq!(
            fs::read_to_string(output.join("existing.py")).unwrap(),
            "old"
        );
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn python_package_names_are_normalized() {
        assert_eq!(normalize_python_package_name("My Bindings"), "my_bindings");
        assert_eq!(normalize_python_package_name("123"), "_123");
    }

    #[test]
    fn stale_cleanup_removes_only_inventory_files() {
        let output = test_directory("stale-cleanup");
        fs::create_dir_all(output.join("old_namespace")).unwrap();
        fs::write(
            output.join("old.py"),
            format!("{GENERATED_PYTHON_HEADER}OLD = True\n"),
        )
        .unwrap();
        fs::write(
            output.join("old_namespace").join("__init__.py"),
            GENERATED_PYTHON_HEADER,
        )
        .unwrap();
        fs::write(output.join("manual.py"), "MANUAL = True\n").unwrap();
        fs::write(
            output.join(PYTHON_GENERATED_INVENTORY),
            "old.py\nold_namespace\\__init__.py\n",
        )
        .unwrap();

        clean_python_generated_output(&output).unwrap();

        assert!(!output.join("old.py").exists());
        assert!(!output.join("old_namespace").exists());
        assert!(output.join("manual.py").exists());
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn manual_python_manifest_is_not_overwritten() {
        let output = test_directory("manual-manifest");
        fs::create_dir_all(&output).unwrap();
        fs::write(
            output.join("pyproject.toml"),
            "[project]\nname = \"manual\"\n",
        )
        .unwrap();

        let error = write_python_package_manifest(&output, &output)
            .expect_err("manual manifest must be preserved");

        assert!(error.contains("Refusing to overwrite"));
        assert!(
            fs::read_to_string(output.join("pyproject.toml"))
                .unwrap()
                .contains("name = \"manual\"")
        );
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn no_pyi_cleanup_preserves_manual_stubs() {
        let output = test_directory("stub-cleanup");
        fs::create_dir_all(&output).unwrap();
        fs::write(
            output.join("generated.pyi"),
            format!("{GENERATED_PYTHON_HEADER}class Generated: ...\n"),
        )
        .unwrap();
        fs::write(output.join("manual.pyi"), "class Manual: ...\n").unwrap();

        remove_all_generated_python_stubs(&output).unwrap();

        assert!(!output.join("generated.pyi").exists());
        assert!(output.join("manual.pyi").exists());
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn namespace_stub_facades_explicitly_reexport_all_symbols() {
        let output = test_directory("stub-reexports");
        fs::create_dir_all(&output).unwrap();
        let interface = meta::InterfaceMeta {
            name: "IWidget".into(),
            namespace: "Contoso.Foundation".into(),
            iid: "00000000-0000-0000-c000-000000000046".into(),
            ..Default::default()
        };

        write_python_package_indexes(
            &output,
            &[],
            std::slice::from_ref(&interface),
            &[],
            true,
            false,
        )
        .unwrap();

        let facade = fs::read_to_string(
            output
                .join("contoso")
                .join("foundation")
                .join("i_widget.pyi"),
        )
        .unwrap();
        assert!(facade.contains("IID_IWidget as IID_IWidget"));
        assert!(facade.contains("IWidget as IWidget"));
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn root_suppression_preserves_unique_exports_from_same_module() {
        let existing = format!(
            "{GENERATED_PYTHON_HEADER}from .contoso__resource_manager import ResourceManager, Point, pack_point  # noqa: F401\n"
        );
        let suppressed = HashSet::from(["ResourceManager".to_string()]);

        let merged = merge_python_indexes(&existing, GENERATED_PYTHON_HEADER, &suppressed);

        assert!(!merged.contains("ResourceManager"));
        assert!(merged.contains("Point, pack_point  # noqa: F401"));
    }

    #[test]
    fn generated_inventory_does_not_claim_nested_manual_metadata() {
        let output = test_directory("manual-metadata");
        let nested = output.join("manual_package");
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            nested.join("pyproject.toml"),
            "[project]\nname = \"manual\"\n",
        )
        .unwrap();
        fs::write(nested.join("py.typed"), "").unwrap();
        fs::write(
            output.join("generated.py"),
            format!("{GENERATED_PYTHON_HEADER}VALUE = True\n"),
        )
        .unwrap();

        write_python_generated_inventory(&output, false).unwrap();
        let inventory = fs::read_to_string(output.join(PYTHON_GENERATED_INVENTORY)).unwrap();

        assert!(inventory.contains("generated.py"));
        assert!(!inventory.contains("pyproject.toml"));
        assert!(!inventory.contains("py.typed"));
        fs::remove_dir_all(output).unwrap();
    }
}
