// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::hash::{BuildHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};

use dynwinrt_codegen::codegen::com;
use dynwinrt_codegen::codegen::javascript;
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

fn resolve_dependencies_for_lang(
    winmd: &str,
    classes: &[meta::ClassMeta],
    interfaces: &[meta::InterfaceMeta],
    enums: &[TypeMeta],
    lang: &str,
) -> meta::ResolvedDeps {
    if lang == "py" {
        meta::resolve_python_dependencies(winmd, classes, interfaces, enums)
    } else {
        meta::resolve_dependencies(winmd, classes, interfaces, enums)
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Print supported machine-readable capabilities, one per line.
    Capabilities,

    /// Measure complete safe Classic COM interface generation.
    ComCensus {
        /// Path(s) to Windows.Win32.winmd metadata, separated by ';'.
        #[arg(long, value_name = "PATH")]
        winmd: String,

        /// Emit one machine-readable JSON object.
        #[arg(long)]
        json: bool,
    },

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

        /// Dedicated codegen-owned output directory.
        /// Existing contents may be replaced or removed; do not store handwritten files here.
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

const COM_MANIFEST_FILE: &str = ".dynwinrt-com-manifest.json";

#[derive(Debug, Default, Deserialize, Serialize)]
struct ComGenerationManifest {
    version: u32,
    roots: BTreeMap<String, BTreeSet<String>>,
}

struct ComManifestUpdate {
    manifest: ComGenerationManifest,
    stale_files: BTreeSet<String>,
}

#[derive(Debug, Serialize)]
struct ComCensusResult {
    metadata: String,
    eligible_interfaces: usize,
    complete_interfaces: usize,
    incomplete_interfaces: usize,
    coverage_percent: f64,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

fn run_com_census(winmd: &str, json: bool) -> Result<(), String> {
    let interfaces = com_metadata::parse_all_com_interfaces(winmd)
        .ok_or_else(|| format!("Failed to load Classic COM metadata from {winmd}"))?;
    let eligible = interfaces
        .into_iter()
        .filter(|interface| {
            (interface.is_iunknown_rooted || interface.interface.name.ends_with("Interop"))
                && !(interface.interface.namespace == "Windows.Win32.UI.Controls.RichEdit"
                    && interface.interface.name == "ITextHost2")
        })
        .collect::<Vec<_>>();
    let complete = eligible
        .iter()
        .filter(|interface| com::generate_com_interface_files(interface, winmd).is_ok())
        .count();
    let result = ComCensusResult {
        metadata: winmd.to_string(),
        eligible_interfaces: eligible.len(),
        complete_interfaces: complete,
        incomplete_interfaces: eligible.len() - complete,
        coverage_percent: if eligible.is_empty() {
            0.0
        } else {
            complete as f64 * 100.0 / eligible.len() as f64
        },
    };
    if json {
        println!(
            "{}",
            serde_json::to_string(&result)
                .map_err(|error| format!("Failed to serialize COM census: {error}"))?
        );
    } else {
        println!(
            "Classic COM complete interfaces: {}/{} ({:.6}%)",
            result.complete_interfaces, result.eligible_interfaces, result.coverage_percent
        );
    }
    Ok(())
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
        Commands::ComCensus { winmd, json } => {
            run_com_census(&winmd, json)?;
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
                let entries = fs::read_dir(dir_path)
                    .map_err(|error| format!("Failed to read --folder '{}': {error}", dir))?;
                let mut folder_paths = entries
                    .map(|entry| {
                        entry
                            .map(|entry| entry.path())
                            .map_err(|error| format!("Failed to read --folder entry: {error}"))
                    })
                    .collect::<Result<Vec<_>, String>>()?
                    .into_iter()
                    .filter(|path| {
                        path.extension()
                            .map_or(false, |ext| ext.eq_ignore_ascii_case("winmd"))
                    })
                    .collect::<Vec<_>>();
                folder_paths.sort_by(|left, right| {
                    let left = left.to_string_lossy();
                    let right = right.to_string_lossy();
                    left.to_ascii_lowercase()
                        .cmp(&right.to_ascii_lowercase())
                        .then_with(|| left.cmp(&right))
                });
                if folder_paths.is_empty() {
                    return Err(format!("No .winmd files found in folder: {}", dir));
                }
                for path in folder_paths {
                    eprintln!("Loading winmd from folder: {}", path.display());
                    winmd_parts.push(path.to_string_lossy().to_string());
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

            if output.trim().is_empty() {
                return Err("JavaScript/Python output directory cannot be empty.".into());
            }
            let final_output_dir = Path::new(&output);
            let output_contains_cwd = output_contains_current_directory(final_output_dir)?;
            if matches!(lang.as_str(), "js" | "py") && !dry_run && output_contains_cwd {
                return Err(format!(
                    "Generated output directory '{}' contains the current working directory. \
                     Choose a dedicated child or sibling output directory so generation can be \
                     committed atomically.",
                    final_output_dir.display()
                ));
            }
            let mut output_transaction =
                if matches!(lang.as_str(), "js" | "py") && !dry_run && !output_contains_cwd {
                    Some(OutputTransaction::begin(final_output_dir)?)
                } else {
                    None
                };
            let effective_output_dir = output_transaction
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
                let mut requested_winrt_interfaces = Vec::new();
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
                        // Public IInspectable-rooted interfaces use the WinRT projection
                        // pipeline directly. `parse_class` intentionally accepts a raw
                        // TypeDef and cannot distinguish this case on its own.
                        let is_runtime_class = meta::parse_namespace(&winmd, ns)
                            .iter()
                            .any(|class| class.name == cls.as_str());
                        if !is_runtime_class {
                            if let Some(interface) = meta::parse_interfaces(&winmd, ns)
                                .into_iter()
                                .find(|interface| interface.name == cls.as_str())
                            {
                                requested_winrt_interfaces.push(interface);
                                continue;
                            }
                            return Err(format!(
                                "{}.{} is an exclusive IInspectable interface, not a public runtime \
                                 class or standalone WinRT interface.",
                                ns, cls
                            ));
                        }
                    }
                    if let Some(coclass) = com_metadata::parse_com_coclass(&winmd, ns, cls)? {
                        com_coclasses.push(coclass);
                        continue;
                    }
                    if let Some(interface) = meta::parse_interfaces(&winmd, ns)
                        .into_iter()
                        .find(|interface| interface.name == cls.as_str())
                    {
                        requested_winrt_interfaces.push(interface);
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
                        generated.push((
                            format!(
                                "{}.{}",
                                com_iface.interface.namespace, com_iface.interface.name
                            ),
                            com_iface.interface.name.clone(),
                            out,
                        ));
                    }
                    for coclass in &com_coclasses {
                        let out =
                            com::generate_com_coclass_files(coclass, &winmd).map_err(|e| {
                                format!(
                                    "Classic-COM coclass codegen for {} failed: {}",
                                    coclass.name, e
                                )
                            })?;
                        generated.push((
                            format!("{}.{}", coclass.namespace, coclass.name),
                            coclass.name.clone(),
                            out,
                        ));
                    }

                    let mut planned_files = BTreeMap::new();
                    let mut root_files = BTreeMap::new();
                    for (root, name, out) in &generated {
                        let mut files = vec![
                            (format!("{name}.js"), out.js.clone()),
                            (format!("{name}.d.ts"), out.dts.clone()),
                        ];
                        files.extend(out.extra_files.iter().cloned());
                        for (file_name, content) in files {
                            root_files
                                .entry(root.clone())
                                .or_insert_with(BTreeSet::new)
                                .insert(file_name.clone());
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
                        ensure_safe_generated_parent(
                            output_dir,
                            &com_output_dir.join(".dynwinrt-write-check"),
                        )?;
                        let manifest_update =
                            prepare_com_generation_manifest(&com_output_dir, &root_files)?;
                        for (file_name, content) in &planned_files {
                            let path = com_output_dir.join(file_name);
                            ensure_safe_generated_destination(output_dir, &path)?;
                            fs::write(&path, content)
                                .map_err(|e| format!("Failed to write {}: {}", file_name, e))?;
                        }
                        apply_com_generation_manifest(&com_output_dir, manifest_update)?;
                    }
                    for (_, name, out) in &generated {
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

                    if classes.is_empty() && requested_winrt_interfaces.is_empty() {
                        if !dry_run {
                            finalize_com_generation(output_dir)?;
                        }
                        if let Some(transaction) = output_transaction.take() {
                            transaction.commit()?;
                        }
                        return Ok(());
                    }
                }

                winui::add_implicit_classes(&winmd, &mut classes);
                let mut implicit_interfaces = requested_winrt_interfaces;
                winui::add_implicit_interfaces(&winmd, &classes, &mut implicit_interfaces);
                let existing_python_identities = if lang == "py" && !dry_run {
                    read_python_type_inventory(output_dir)?
                        .into_iter()
                        .map(|typ| typ.identity)
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let (_, _, _, shared_interfaces) = generate_for_types(
                    &winmd,
                    output_dir,
                    classes.clone(),
                    implicit_interfaces.clone(),
                    Vec::new(),
                    dry_run,
                    &lang,
                    &import_name,
                    pyi,
                    &doc_table,
                    &existing_python_identities,
                )?;

                // Write (or append to) the index file for the output directory
                if !dry_run {
                    let deps = resolve_dependencies_for_lang(
                        &winmd,
                        &classes,
                        &implicit_interfaces,
                        &[],
                        &lang,
                    );
                    let mut all_classes = [classes.as_slice(), deps.classes.as_slice()].concat();
                    let mut all_interfaces =
                        [implicit_interfaces.as_slice(), deps.interfaces.as_slice()].concat();
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
                        record_python_supplemental_types(output_dir, &shared_interfaces)?;
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

                let mut selected_classes = Vec::new();
                let mut selected_interfaces = Vec::new();
                let mut selected_enums = Vec::new();
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
                    selected_classes.extend(meta::parse_namespace(&winmd, ns));
                    selected_interfaces.extend(meta::parse_interfaces(&winmd, ns));
                    selected_enums.extend(meta::parse_enums(&winmd, ns));
                }
                winui::add_implicit_classes(&winmd, &mut selected_classes);
                winui::add_implicit_interfaces(&winmd, &selected_classes, &mut selected_interfaces);
                let (total_classes, total_interfaces, total_enums, shared_interfaces) =
                    generate_for_types(
                        &winmd,
                        output_dir,
                        selected_classes,
                        selected_interfaces,
                        selected_enums,
                        dry_run,
                        &lang,
                        &import_name,
                        pyi,
                        &doc_table,
                        &[],
                    )?;

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
                    winui::add_implicit_classes(&winmd, &mut all_classes);
                    winui::add_implicit_interfaces(&winmd, &all_classes, &mut all_interfaces);
                    let deps = resolve_dependencies_for_lang(
                        &winmd,
                        &all_classes,
                        &all_interfaces,
                        &all_enums,
                        &lang,
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
                        record_python_supplemental_types(output_dir, &shared_interfaces)?;
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
            }
            if let Some(transaction) = output_transaction.take() {
                transaction.commit()?;
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
    runtime_import_name: &str,
    pyi: bool,
    doc_table: &DocTable,
    existing_python_identities: &[python::PythonTypeIdentity],
) -> Result<(usize, usize, usize, Vec<meta::InterfaceMeta>), String> {
    let deps = resolve_dependencies_for_lang(winmd, &classes, &interfaces, &enums, lang);
    let mut all_classes = classes;
    let mut all_interfaces = interfaces;
    let mut all_enums = enums;
    all_classes.extend(deps.classes);
    all_interfaces.extend(deps.interfaces);
    all_enums.extend(deps.enums);
    let previous_javascript_inventory = if lang != "py" {
        if dry_run {
            check_javascript_layout_inventory(output_dir)?;
        } else {
            ensure_javascript_layout_inventory(output_dir)?;
        }
        read_javascript_type_inventory(output_dir)?
    } else {
        JavaScriptTypeInventory::default()
    };
    let previous_javascript_records = previous_javascript_inventory.records;

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
    let current_javascript_records = if lang != "py" {
        javascript_type_layout_records(&all_classes, &all_interfaces, &all_enums)?
    } else {
        Vec::new()
    };
    let mut retained_javascript_renames = Vec::new();
    let javascript_context = if lang != "py" {
        validate_javascript_type_layout_records(
            &previous_javascript_records,
            &current_javascript_records,
        )?;
        let identities = previous_javascript_records
            .iter()
            .chain(current_javascript_records.iter())
            .map(|record| record.identity.clone());
        let context = javascript::create_javascript_projection_context_with_records(
            identities,
            previous_javascript_records.iter().cloned(),
            runtime_import_name,
        )?;
        let projected_names = context
            .output_targets()
            .into_iter()
            .map(|target| (target.identity.clone(), target.projected_name.clone()))
            .collect::<HashMap<_, _>>();
        let current_identities = current_javascript_records
            .iter()
            .map(|record| record.identity.clone())
            .collect::<HashSet<_>>();
        retained_javascript_renames = previous_javascript_records
            .iter()
            .filter(|record| {
                projected_names
                    .get(&record.identity)
                    .is_some_and(|projected| projected != &record.projected_name)
                    && !current_identities.contains(&record.identity)
            })
            .cloned()
            .collect::<Vec<_>>();
        let current_struct_helpers =
            javascript::validate_struct_helper_identities(&all_classes, &all_interfaces)?;
        validate_generated_struct_helper_identities_against(
            &context,
            output_dir,
            &current_struct_helpers,
        )?;
        if !output_dir.join(JAVASCRIPT_TYPE_INVENTORY).is_file() {
            ensure_uninventoried_javascript_targets_absent(&context, output_dir)?;
        }
        javascript::apply_javascript_projected_names(
            &context,
            &mut all_classes,
            &mut all_interfaces,
            &mut all_enums,
        );
        validate_unique_class_output_names(&all_classes)?;
        Some(context)
    } else {
        None
    };

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
    if lang != "py" {
        for target in javascript_context
            .as_ref()
            .expect("JavaScript context")
            .output_targets()
        {
            known_types.insert(target.projected_name.clone());
            known_types.insert(format!(
                "{}.{}",
                target.identity.namespace, target.identity.name
            ));
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
            if ri.iid.is_empty() || ri.generic_piid.is_some() {
                continue;
            }
            req_iface_count
                .entry(ri.iid.clone())
                .and_modify(|(_, c)| *c += 1)
                .or_insert((ri, 1));
        }
    }
    let mut shared_iids: HashSet<String> = req_iface_count
        .iter()
        .filter(|(_, (_, count))| *count >= 2)
        .map(|(iid, _)| iid.clone())
        .collect();
    if lang != "py" {
        shared_iids.extend(
            all_interfaces
                .iter()
                .filter(|interface| interface.generic_piid.is_none())
                .map(|interface| interface.iid.clone()),
        );
        shared_iids.extend(previous_javascript_records.iter().filter_map(|record| {
            (record.identity.kind == javascript::JavaScriptTypeKind::Interface
                && record.abi_identity.starts_with("iid:"))
            .then(|| record.abi_identity.strip_prefix("iid:"))
            .flatten()
            .map(str::to_string)
        }));
    }

    let shared_interfaces: Vec<meta::InterfaceMeta> = req_iface_count
        .iter()
        .filter(|(_, (_, count))| *count >= 2)
        .map(|(_, (iface, _))| (*iface).clone())
        .collect();
    if lang == "py" {
        let mut struct_interfaces = emittable_interfaces.clone();
        struct_interfaces.extend(shared_interfaces.iter().cloned());
        python::validate_struct_symbol_uniqueness(&all_classes, &struct_interfaces)?;
    }
    for iface in &shared_interfaces {
        known_types.insert(iface.name.clone());
    }
    if lang == "py" {
        let mut struct_interfaces = emittable_interfaces.clone();
        struct_interfaces.extend(shared_interfaces.iter().cloned());
        for typ in python::package_structs(&all_classes, &struct_interfaces) {
            if let TypeMeta::Struct {
                namespace, name, ..
            } = typ
            {
                known_types.insert(name.clone());
                known_types.insert(format!("{namespace}.{name}"));
            }
        }
    }

    let python_layout = if lang == "py" {
        Some(install_python_generation_layout(
            &all_classes,
            &emittable_interfaces,
            &all_enums,
            &shared_interfaces,
            existing_python_identities,
        )?)
    } else {
        None
    };

    let (delegate_signatures, delegate_sig_refs, delegate_param_wraps) =
        if let Some(context) = javascript_context.as_ref() {
            project::build_delegate_signatures(
                context,
                &all_interfaces,
                &delegate_type_names,
                &known_types,
            )
        } else {
            Default::default()
        };

    if !dry_run {
        if lang == "py" {
            generate_py_files(
                output_dir,
                &all_classes,
                &emittable_interfaces,
                &all_enums,
                &shared_interfaces,
                &known_types,
                &delegate_type_names,
                &shared_iids,
                pyi,
            )?;
        } else {
            let mut plan = generate_js_files(
                javascript_context.as_ref().expect("JavaScript context"),
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
            let context = javascript_context.as_ref().expect("JavaScript context");
            write_retained_javascript_projected_aliases(
                context,
                output_dir,
                &retained_javascript_renames,
            )?;
            validate_generated_struct_helper_identities(context, output_dir)?;
            write_javascript_type_inventory(
                output_dir,
                &emitted_javascript_type_records(
                    context,
                    output_dir,
                    &previous_javascript_records,
                    &current_javascript_records,
                )?,
            )?;
            write_js_barrel_and_manifest(output_dir, &mut plan)?;
        }
        drop(python_layout);
    }

    Ok((
        all_classes.len(),
        all_interfaces.len(),
        all_enums.len(),
        shared_interfaces,
    ))
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
    identities.extend(
        python::package_struct_identities(classes, interfaces)
            .into_iter()
            .map(|(namespace, name)| python::PythonTypeIdentity { namespace, name }),
    );
    identities
}

fn install_python_generation_layout(
    classes: &[meta::ClassMeta],
    interfaces: &[meta::InterfaceMeta],
    enums: &[TypeMeta],
    supplemental_interfaces: &[meta::InterfaceMeta],
    existing_identities: &[python::PythonTypeIdentity],
) -> Result<python::PythonModuleLayoutGuard, String> {
    let mut identities = python_type_identities(classes, interfaces, enums);
    identities.extend(python_type_identities(&[], supplemental_interfaces, &[]));
    identities.extend_from_slice(existing_identities);
    python::install_python_module_layout(identities)
}

const JAVASCRIPT_TYPE_INVENTORY: &str = ".dynwinrt-js-types";
const JAVASCRIPT_TYPE_INVENTORY_VERSION: u32 = 1;

#[derive(Debug, Default, Deserialize, Serialize)]
struct JavaScriptTypeInventory {
    version: u32,
    records: Vec<javascript::JavaScriptTypeLayoutRecord>,
}

fn ensure_javascript_layout_inventory(output_dir: &Path) -> Result<(), String> {
    if !output_dir.is_dir() {
        return Ok(());
    }
    check_javascript_layout_inventory(output_dir)
}

fn check_javascript_layout_inventory(output_dir: &Path) -> Result<(), String> {
    if !output_dir.is_dir() {
        return Ok(());
    }
    if output_dir.join(JAVASCRIPT_TYPE_INVENTORY).is_file() {
        return Ok(());
    }
    fn contains_generated_implementation(root: &Path, current: &Path) -> bool {
        let Ok(entries) = fs::read_dir(current) else {
            return false;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if is_link_or_reparse_point(&metadata) {
                continue;
            }
            if metadata.is_dir() {
                if current == root && path.file_name().is_some_and(|name| name == "com") {
                    continue;
                }
                if contains_generated_implementation(root, &path) {
                    return true;
                }
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if matches!(
                name,
                "index.js"
                    | "index.mjs"
                    | "index.d.ts"
                    | "index.proxy.js"
                    | "index.getter.js"
                    | "lifetime.js"
                    | "lifetime.d.ts"
                    | "package.json"
            ) {
                continue;
            }
            if (name.ends_with(".js") || name.ends_with(".d.ts"))
                && fs::read_to_string(&path)
                    .map(|content| content.starts_with("// Generated by dynwinrt-codegen"))
                    .unwrap_or(false)
            {
                return true;
            }
        }
        false
    }

    if contains_generated_implementation(output_dir, output_dir) {
        return Err(format!(
            "Existing JavaScript bindings in '{}' use the legacy flat layout and cannot be \
             updated incrementally without type identities. Remove that generated directory \
             and regenerate it once to migrate to the namespace layout.",
            output_dir.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
fn javascript_type_identities(
    classes: &[meta::ClassMeta],
    interfaces: &[meta::InterfaceMeta],
    enums: &[TypeMeta],
) -> Result<Vec<javascript::JavaScriptTypeIdentity>, String> {
    Ok(javascript_type_layout_records(classes, interfaces, enums)?
        .into_iter()
        .map(|record| record.identity)
        .collect())
}

fn javascript_interface_abi_identity(interface: &meta::InterfaceMeta) -> String {
    javascript::interface_abi_identity(interface)
}

fn javascript_type_layout_records(
    classes: &[meta::ClassMeta],
    interfaces: &[meta::InterfaceMeta],
    enums: &[TypeMeta],
) -> Result<Vec<javascript::JavaScriptTypeLayoutRecord>, String> {
    let record = |identity: javascript::JavaScriptTypeIdentity, abi_identity: String| {
        javascript::JavaScriptTypeLayoutRecord::new(
            identity.clone(),
            identity.name.clone(),
            abi_identity,
        )
    };
    let interface_record = |interface: &meta::InterfaceMeta| {
        let abi_identity = javascript_interface_abi_identity(interface);
        let kind = javascript_interface_kind(interface);
        let identity = if interface.generic_piid.is_some() {
            javascript::JavaScriptTypeIdentity::with_variant(
                &interface.namespace,
                &javascript::parameterized_interface_name(
                    &interface.namespace,
                    &interface.name,
                    interface.generic_piid.as_deref().unwrap_or_default(),
                    &interface.generic_args,
                ),
                kind,
                &javascript::parameterized_reference_identity(
                    interface.generic_piid.as_deref().unwrap_or_default(),
                    &interface.generic_args,
                ),
            )
        } else {
            javascript::JavaScriptTypeIdentity::new(&interface.namespace, &interface.name, kind)
        };
        record(identity, abi_identity)
    };
    let class_identities = classes
        .iter()
        .map(|class| (class.namespace.as_str(), class.name.as_str()))
        .collect::<HashSet<_>>();
    let mut required_iid_counts = HashMap::<&str, usize>::new();
    for interface in classes
        .iter()
        .flat_map(|class| class.required_interfaces.iter())
        .filter(|interface| interface.generic_piid.is_none() && !interface.iid.is_empty())
    {
        *required_iid_counts
            .entry(interface.iid.as_str())
            .or_default() += 1;
    }
    let mut records = classes
        .iter()
        .map(|class| {
            record(
                javascript::JavaScriptTypeIdentity::new(
                    &class.namespace,
                    &class.name,
                    javascript::JavaScriptTypeKind::Class,
                ),
                "type".into(),
            )
        })
        .collect::<Vec<_>>();
    records.extend(
        classes
            .iter()
            .flat_map(|class| class.required_interfaces.iter())
            .filter(|interface| {
                !interface.iid.is_empty()
                    && interface.generic_piid.is_none()
                    && required_iid_counts
                        .get(interface.iid.as_str())
                        .is_some_and(|count| *count >= 2)
                    && !class_identities
                        .contains(&(interface.namespace.as_str(), interface.name.as_str()))
            })
            .map(interface_record)
            .collect::<Vec<_>>(),
    );
    records.extend(
        interfaces
            .iter()
            .filter(|interface| {
                !interface.iid.is_empty()
                    && !class_identities
                        .contains(&(interface.namespace.as_str(), interface.name.as_str()))
            })
            .map(interface_record)
            .collect::<Vec<_>>(),
    );
    records.extend(enums.iter().filter_map(|typ| {
        let TypeMeta::Enum {
            namespace, name, ..
        } = typ
        else {
            return None;
        };
        (!class_identities.contains(&(namespace.as_str(), name.as_str()))).then(|| {
            record(
                javascript::JavaScriptTypeIdentity::new(
                    namespace,
                    name,
                    javascript::JavaScriptTypeKind::Enum,
                ),
                "type".into(),
            )
        })
    }));
    Ok(records)
}

fn javascript_interface_kind(interface: &meta::InterfaceMeta) -> javascript::JavaScriptTypeKind {
    if interface
        .methods
        .iter()
        .any(|method| method.name == ".ctor")
        && interface
            .methods
            .iter()
            .any(|method| method.name == "Invoke")
    {
        javascript::JavaScriptTypeKind::Delegate
    } else {
        javascript::JavaScriptTypeKind::Interface
    }
}

fn read_javascript_type_inventory(output_dir: &Path) -> Result<JavaScriptTypeInventory, String> {
    if output_dir.is_dir() && !output_contains_current_directory(output_dir)? {
        ensure_generated_tree_has_no_links(output_dir)?;
    }
    let path = output_dir.join(JAVASCRIPT_TYPE_INVENTORY);
    if !path.is_file() {
        return Ok(JavaScriptTypeInventory::default());
    }
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
    let inventory = serde_json::from_str::<JavaScriptTypeInventory>(&content).map_err(|error| {
        format!(
            "Invalid JavaScript type inventory {}: {error}",
            path.display()
        )
    })?;
    if inventory.version != JAVASCRIPT_TYPE_INVENTORY_VERSION {
        return Err(format!(
            "Unsupported or invalid JavaScript type inventory in {}",
            path.display()
        ));
    }
    validate_javascript_type_layout_records(&inventory.records, &[])?;
    validate_javascript_inventory_files(output_dir, &inventory.records)?;
    Ok(inventory)
}

fn validate_javascript_inventory_files(
    output_dir: &Path,
    records: &[javascript::JavaScriptTypeLayoutRecord],
) -> Result<(), String> {
    let context = javascript::create_javascript_projection_context_with_records(
        records.iter().map(|record| record.identity.clone()),
        records.iter().cloned(),
        "@microsoft/dynwinrt",
    )?;
    let targets = context.output_targets().cloned().collect::<Vec<_>>();
    let target_layout = targets
        .iter()
        .map(|target| {
            (
                &target.identity,
                (
                    target.projected_name.as_str(),
                    &target.compatibility_aliases,
                ),
            )
        })
        .collect::<HashMap<_, _>>();
    if let Some(record) = records.iter().find(|record| {
        target_layout
            .get(&record.identity)
            .is_none_or(|(projected, aliases)| {
                *projected != record.projected_name || *aliases != &record.compatibility_aliases
            })
    }) {
        return Err(format!(
            "JavaScript type inventory has non-deterministic projected name `{}` for `{}.{}`",
            record.projected_name, record.identity.namespace, record.identity.name
        ));
    }
    let expected_canonical = targets
        .iter()
        .map(|target| target.canonical_module.clone())
        .collect::<BTreeSet<_>>();
    let shared_output = output_contains_current_directory(output_dir)?;
    let expected_namespace_roots = expected_canonical
        .iter()
        .filter_map(|module| module.split('/').next().map(str::to_string))
        .collect::<BTreeSet<_>>();
    let mut actual_canonical = BTreeSet::new();

    fn visit(
        root: &Path,
        current: &Path,
        actual_canonical: &mut BTreeSet<String>,
        shared_output: bool,
        expected_namespace_roots: &BTreeSet<String>,
    ) {
        let Ok(entries) = fs::read_dir(current) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path == root.join("com") {
                    continue;
                }
                if shared_output
                    && current == root
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_none_or(|name| !expected_namespace_roots.contains(name))
                {
                    continue;
                }
                visit(
                    root,
                    &path,
                    actual_canonical,
                    shared_output,
                    expected_namespace_roots,
                );
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.ends_with(".js")
                || matches!(
                    name,
                    "index.js" | "index.proxy.js" | "index.getter.js" | "lifetime.js"
                )
            {
                continue;
            }
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            if content.starts_with("// Generated by dynwinrt-codegen")
                && let Ok(relative) = path.strip_prefix(root)
            {
                actual_canonical.insert(
                    relative
                        .with_extension("")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    visit(
        output_dir,
        output_dir,
        &mut actual_canonical,
        shared_output,
        &expected_namespace_roots,
    );

    if actual_canonical != expected_canonical {
        let summarize =
            |values: BTreeSet<String>| values.into_iter().take(5).collect::<Vec<_>>().join(", ");
        return Err(format!(
            "JavaScript generated type inventory does not match '{}' (missing canonical: [{}]; \
             unexpected canonical: [{}])",
            output_dir.display(),
            summarize(
                expected_canonical
                    .difference(&actual_canonical)
                    .cloned()
                    .collect()
            ),
            summarize(
                actual_canonical
                    .difference(&expected_canonical)
                    .cloned()
                    .collect()
            ),
        ));
    }
    let records_by_identity = records
        .iter()
        .map(|record| (&record.identity, record))
        .collect::<HashMap<_, _>>();
    for target in targets {
        for suffix in ["js", "d.ts"] {
            let canonical = output_dir.join(format!("{}.{suffix}", target.canonical_module));
            if !canonical.is_file() {
                return Err(format!(
                    "JavaScript generated type inventory references missing files for `{}.{}`",
                    target.identity.namespace, target.identity.name
                ));
            }
        }
        let record = records_by_identity[&target.identity];
        let js_path = output_dir.join(format!("{}.js", target.canonical_module));
        let js = fs::read_to_string(&js_path)
            .map_err(|error| format!("Failed to read {}: {error}", js_path.display()))?;
        let implementation_export =
            if target.identity.kind == javascript::JavaScriptTypeKind::Delegate {
                format!("exports.IID_{} =", record.implementation_name)
            } else {
                format!("exports.{} =", record.implementation_name)
            };
        if !js.contains(&implementation_export) {
            return Err(format!(
                "JavaScript type inventory implementation `{}` is missing from '{}'",
                record.implementation_name,
                js_path.display()
            ));
        }
    }
    Ok(())
}

fn ensure_uninventoried_javascript_targets_absent(
    context: &javascript::JavaScriptProjectionContext,
    output_dir: &Path,
) -> Result<(), String> {
    for target in context.output_targets() {
        for suffix in ["js", "d.ts"] {
            let path = output_dir.join(format!("{}.{suffix}", target.canonical_module));
            if path.exists() {
                return Err(format!(
                    "Existing JavaScript canonical module '{}' has no type inventory. Remove the \
                     incomplete generated output and regenerate it.",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

fn validate_javascript_type_layout_records(
    previous: &[javascript::JavaScriptTypeLayoutRecord],
    current: &[javascript::JavaScriptTypeLayoutRecord],
) -> Result<(), String> {
    let mut previous_by_identity = HashMap::new();
    for record in previous {
        if previous_by_identity
            .insert(record.identity.clone(), record)
            .is_some()
        {
            return Err(format!(
                "JavaScript type inventory contains duplicate layout records for `{}.{}`",
                record.identity.namespace, record.identity.name
            ));
        }
    }

    let mut abi_by_identity = previous
        .iter()
        .map(|record| (record.identity.clone(), record.abi_identity.as_str()))
        .collect::<HashMap<_, _>>();
    for record in current {
        if let Some(existing) = abi_by_identity.get(&record.identity) {
            if *existing != record.abi_identity {
                return Err(format!(
                    "JavaScript output identity `{}.{}` maps to multiple WinRT ABI identities \
                     (`{existing}` and `{}`). Closed generic interfaces with identical generated \
                     names are not supported; generation stopped before either module was overwritten.",
                    record.identity.namespace, record.identity.name, record.abi_identity,
                ));
            }
        } else {
            abi_by_identity.insert(record.identity.clone(), &record.abi_identity);
        }
    }
    Ok(())
}

fn write_retained_javascript_projected_aliases(
    context: &javascript::JavaScriptProjectionContext,
    output_dir: &Path,
    renamed: &[javascript::JavaScriptTypeLayoutRecord],
) -> Result<(), String> {
    let targets = context
        .output_targets()
        .into_iter()
        .map(|target| (target.identity.clone(), target.clone()))
        .collect::<HashMap<_, _>>();
    let append = |content: &mut String, line: String| {
        if !content.contains(&line) {
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(&line);
        }
    };
    for record in renamed {
        let target = targets.get(&record.identity).ok_or_else(|| {
            format!(
                "JavaScript module layout does not contain renamed type `{}.{}`",
                record.identity.namespace, record.identity.name
            )
        })?;
        if target.projected_name == record.implementation_name {
            continue;
        }
        let js_path = output_dir.join(format!("{}.js", target.canonical_module));
        let dts_path = output_dir.join(format!("{}.d.ts", target.canonical_module));
        let mut js = fs::read_to_string(&js_path)
            .map_err(|error| format!("Failed to read {}: {error}", js_path.display()))?;
        let mut dts = fs::read_to_string(&dts_path)
            .map_err(|error| format!("Failed to read {}: {error}", dts_path.display()))?;
        let implementation = &record.implementation_name;
        let new = &target.projected_name;
        if record.identity.kind == javascript::JavaScriptTypeKind::Delegate {
            if !js.contains(&format!("exports.IID_{implementation} =")) {
                return Err(format!(
                    "Retained delegate module '{}' does not export IID_{implementation}",
                    js_path.display()
                ));
            }
            append(
                &mut js,
                format!(
                    "exports.IID_{new} = exports.IID_{implementation};\n\
                     exports.{new}_PARAM_TYPES = exports.{implementation}_PARAM_TYPES;\n"
                ),
            );
            append(
                &mut dts,
                format!(
                    "export {{ IID_{implementation} as IID_{new}, \
                     {implementation}_PARAM_TYPES as {new}_PARAM_TYPES }};\n\
                     export type {new} = {implementation};\n"
                ),
            );
        } else {
            if !js.contains(&format!("exports.{implementation} =")) {
                return Err(format!(
                    "Retained JavaScript module '{}' does not export `{implementation}`",
                    js_path.display()
                ));
            }
            append(
                &mut js,
                format!("exports.{new} = exports.{implementation};\n"),
            );
            append(
                &mut dts,
                format!("export {{ {implementation} as {new} }};\n"),
            );
            if js.contains(&format!("exports.IID_{implementation} =")) {
                append(
                    &mut js,
                    format!("exports.IID_{new} = exports.IID_{implementation};\n"),
                );
                append(
                    &mut dts,
                    format!("export {{ IID_{implementation} as IID_{new} }};\n"),
                );
            }
        }
        ensure_safe_generated_destination(output_dir, &js_path)?;
        ensure_safe_generated_destination(output_dir, &dts_path)?;
        fs::write(&js_path, js)
            .map_err(|error| format!("Failed to write {}: {error}", js_path.display()))?;
        fs::write(&dts_path, dts)
            .map_err(|error| format!("Failed to write {}: {error}", dts_path.display()))?;
    }
    Ok(())
}

fn write_javascript_type_inventory(
    output_dir: &Path,
    records: &[javascript::JavaScriptTypeLayoutRecord],
) -> Result<(), String> {
    let inventory = JavaScriptTypeInventory {
        version: JAVASCRIPT_TYPE_INVENTORY_VERSION,
        records: records.to_vec(),
    };
    let content = serde_json::to_string_pretty(&inventory)
        .map_err(|error| format!("Failed to serialize JavaScript type inventory: {error}"))?;
    let path = output_dir.join(JAVASCRIPT_TYPE_INVENTORY);
    ensure_safe_generated_destination(output_dir, &path)?;
    write_file(&path, &format!("{content}\n"))
}

fn emitted_javascript_type_records(
    context: &javascript::JavaScriptProjectionContext,
    output_dir: &Path,
    previous: &[javascript::JavaScriptTypeLayoutRecord],
    current: &[javascript::JavaScriptTypeLayoutRecord],
) -> Result<Vec<javascript::JavaScriptTypeLayoutRecord>, String> {
    let identities = previous
        .iter()
        .chain(current)
        .map(|record| (record.identity.clone(), record.abi_identity.clone()))
        .collect::<HashMap<_, _>>();
    let previous_implementations = previous
        .iter()
        .map(|record| (record.identity.clone(), record.implementation_name.clone()))
        .collect::<HashMap<_, _>>();
    let current_identities = current
        .iter()
        .map(|record| record.identity.clone())
        .collect::<HashSet<_>>();
    context
        .output_targets()
        .into_iter()
        .filter(|target| {
            output_dir
                .join(format!("{}.js", target.canonical_module))
                .is_file()
                && output_dir
                    .join(format!("{}.d.ts", target.canonical_module))
                    .is_file()
        })
        .map(|target| {
            let abi_identity = identities.get(&target.identity).cloned().ok_or_else(|| {
                format!(
                    "Missing WinRT ABI identity for JavaScript output `{}.{}`",
                    target.identity.namespace, target.identity.name
                )
            })?;
            let compatibility_aliases = target.compatibility_aliases.clone();
            let implementation_name = if current_identities.contains(&target.identity) {
                target.projected_name.clone()
            } else {
                previous_implementations
                    .get(&target.identity)
                    .cloned()
                    .ok_or_else(|| {
                        format!(
                            "Missing implementation name for retained JavaScript output `{}.{}`",
                            target.identity.namespace, target.identity.name
                        )
                    })?
            };
            Ok(javascript::JavaScriptTypeLayoutRecord::new(
                target.identity.clone(),
                target.projected_name.clone(),
                abi_identity,
            )
            .with_implementation_name(implementation_name)
            .with_compatibility_aliases(compatibility_aliases))
        })
        .collect()
}

fn generated_struct_helper_identities(content: &str) -> Vec<(String, String)> {
    content
        .lines()
        .filter_map(|line| {
            let declaration = line.trim_start().strip_prefix("const ")?;
            let (name, value) = declaration.split_once("_Type = DynWinRtType.structType('")?;
            let (identity, _) = value.split_once('\'')?;
            (!name.is_empty() && !identity.is_empty())
                .then(|| (name.to_string(), identity.to_string()))
        })
        .collect()
}

fn validate_generated_struct_helper_identities_against(
    context: &javascript::JavaScriptProjectionContext,
    output_dir: &Path,
    current: &BTreeMap<String, String>,
) -> Result<(), String> {
    let mut owners = current
        .iter()
        .map(|(name, identity)| {
            (
                name.clone(),
                (identity.clone(), "current generation".to_string()),
            )
        })
        .collect::<HashMap<_, _>>();
    for target in context.output_targets() {
        let path = output_dir.join(format!("{}.js", target.canonical_module));
        if !path.is_file() {
            continue;
        }
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        for (name, identity) in generated_struct_helper_identities(&content) {
            if let Some((existing, existing_module)) = owners.insert(
                name.clone(),
                (identity.clone(), target.canonical_module.clone()),
            ) && existing != identity
            {
                return Err(format!(
                    "JavaScript struct helper collision: `{existing}` from `{existing_module}` and \
                     `{identity}` from `{}` both export `pack{name}`. Generate these bindings into \
                     separate output directories.",
                    target.canonical_module
                ));
            }
        }
    }
    Ok(())
}

fn validate_generated_struct_helper_identities(
    context: &javascript::JavaScriptProjectionContext,
    output_dir: &Path,
) -> Result<(), String> {
    validate_generated_struct_helper_identities_against(context, output_dir, &BTreeMap::new())
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

fn load_effective_generation_plan(
    context: &javascript::JavaScriptProjectionContext,
    output_dir: &Path,
    regenerated_modules: &HashSet<String>,
) -> Result<dynwinrt_codegen::codegen::projected::GenerationPlan, String> {
    use dynwinrt_codegen::codegen::projected::{GeneratedModule, GenerationPlan};

    let mut plan = GenerationPlan::default();
    let mut retained_public_modules = BTreeSet::new();
    for target in context.output_targets() {
        if output_dir
            .join(format!("{}.js", target.canonical_module))
            .is_file()
            && output_dir
                .join(format!("{}.d.ts", target.canonical_module))
                .is_file()
        {
            plan.insert(GeneratedModule::retained(&target.canonical_module))?;
            if target.identity.kind != javascript::JavaScriptTypeKind::Delegate
                && !regenerated_modules.contains(&target.canonical_module)
            {
                retained_public_modules.insert(target.canonical_module.clone());
            }
        }
    }

    // Incremental generation may retain modules from the previous inventory.
    // Their user-facing exports are loaded only from the previous root metadata;
    // newly rendered JavaScript is never parsed.
    let index_path = output_dir.join("index.d.ts");
    let mut indexed_modules = BTreeSet::new();
    if index_path.is_file() {
        let index = fs::read_to_string(&index_path)
            .map_err(|error| format!("Failed to read {}: {error}", index_path.display()))?;
        for line in index.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            let (names, module) = parse_root_export_metadata(line).ok_or_else(|| {
                format!(
                    "Invalid retained JavaScript root export metadata in {}: `{line}`",
                    index_path.display()
                )
            })?;
            indexed_modules.insert(module.clone());
            if module == "lifetime" {
                let mut retained = GeneratedModule::retained("lifetime");
                retained.public_exports.extend(names);
                plan.insert(retained)?;
            } else if let Some(retained) = plan.modules.get_mut(&module) {
                retained.public_exports.extend(names);
            } else {
                return Err(format!(
                    "Retained JavaScript root export metadata references missing canonical module `{module}`"
                ));
            }
        }
    } else if !retained_public_modules.is_empty() {
        return Err(format!(
            "Retained JavaScript modules require root export metadata in {}",
            index_path.display()
        ));
    }
    if let Some(module) = retained_public_modules.difference(&indexed_modules).next() {
        return Err(format!(
            "Retained JavaScript module `{module}` is missing root export metadata in {}",
            index_path.display()
        ));
    }
    for target in context.output_targets() {
        if regenerated_modules.contains(&target.canonical_module) {
            continue;
        }
        let Some(module) = plan.modules.get_mut(&target.canonical_module) else {
            continue;
        };
        if target.collides {
            module.public_exports.remove(&target.identity.name);
        }
        let retained_projected_alias = target
            .compatibility_aliases
            .iter()
            .any(|alias| module.public_exports.contains(alias));
        for alias in &target.compatibility_aliases {
            module.public_exports.remove(alias);
        }
        if target.identity.kind != javascript::JavaScriptTypeKind::Delegate {
            if retained_projected_alias {
                module.public_exports.insert(target.projected_name.clone());
            }
            if !module.public_exports.contains(&target.projected_name) {
                return Err(format!(
                    "Retained JavaScript module `{}` does not export its projected type `{}` in {}",
                    target.canonical_module,
                    target.projected_name,
                    index_path.display()
                ));
            }
            module.primary_export = Some(target.projected_name.clone());
        }
        module.compatibility_aliases = target.compatibility_aliases.clone();
    }
    Ok(plan)
}

fn parse_root_export_metadata(line: &str) -> Option<(Vec<String>, String)> {
    let line = line.trim().trim_end_matches(';');
    let rest = line.strip_prefix("export {")?;
    let (names, source) = rest.split_once("} from './")?;
    let module = source.strip_suffix(".js'")?.to_string();
    let names = names
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    (!names.is_empty()).then_some((names, module))
}

fn write_generation_plan_modules(
    output_dir: &Path,
    plan: &dynwinrt_codegen::codegen::projected::GenerationPlan,
) -> Result<(), String> {
    for module in plan.modules.values() {
        let (Some(js), Some(dts)) = (&module.javascript, &module.declarations) else {
            continue;
        };
        let js_path = output_dir.join(format!("{}.js", module.canonical_module));
        let dts_path = output_dir.join(format!("{}.d.ts", module.canonical_module));
        write_generated_javascript_file(output_dir, &js_path, js)?;
        write_generated_javascript_file(output_dir, &dts_path, dts)?;
        println!("Generated {}", js_path.display());
    }
    Ok(())
}

fn emit_javascript_projected_file(
    context: &javascript::JavaScriptProjectionContext,
    mut projected: dynwinrt_codegen::codegen::projected::ProjectedFile,
) -> Result<dynwinrt_codegen::codegen::projected::GeneratedModule, String> {
    use dynwinrt_codegen::codegen::projected::{GeneratedModule, PlannedImport};

    let public_exports = projected.public_exports();
    let internal_exports = projected.internal_exports();
    let dependencies = projected
        .imports
        .iter()
        .map(|import| {
            if import.is_runtime_package {
                return None;
            }
            import
                .from
                .strip_prefix("./")
                .and_then(|source| source.strip_suffix(".js"))
                .map(|name| {
                    context.output_target(name).map_or_else(
                        || name.to_string(),
                        |target| target.canonical_module.clone(),
                    )
                })
        })
        .collect::<Vec<_>>();
    let target = context
        .configure_projected_file(&mut projected)
        .ok_or_else(|| {
            format!(
                "JavaScript module layout does not contain projected type `{}`",
                projected.name
            )
        })?;
    let mut js = render_js::render(&projected);
    let mut dts = render_dts::render(&projected);
    let lifetime = javascript::root_relative_module(&target.canonical_module, "lifetime");
    js = js.replace(
        "require('./lifetime.js')",
        &format!("require('{lifetime}.js')"),
    );
    if target.collides {
        if target.identity.kind == javascript::JavaScriptTypeKind::Delegate {
            js.push_str(&format!(
                "exports.IID_{native} = exports.IID_{projected};\n\
                 exports.{native}_PARAM_TYPES = exports.{projected}_PARAM_TYPES;\n",
                native = target.identity.name,
                projected = target.projected_name,
            ));
            dts.push_str(&format!(
                "\nexport {{ IID_{projected} as IID_{native}, \
                 {projected}_PARAM_TYPES as {native}_PARAM_TYPES }};\n\
                 export type {native} = {projected};\n",
                projected = target.projected_name,
                native = target.identity.name,
            ));
        } else {
            if js.contains(&format!("exports.{} =", target.projected_name)) {
                js.push_str(&format!(
                    "exports.{native} = {projected};\n",
                    native = target.identity.name,
                    projected = target.projected_name,
                ));
            }
            let projected_iid = format!("exports.IID_{} =", target.projected_name);
            if js.contains(&projected_iid) {
                js.push_str(&format!(
                    "exports.IID_{native} = exports.IID_{projected};\n",
                    native = target.identity.name,
                    projected = target.projected_name,
                ));
            }
            dts.push_str(&format!(
                "\nexport {{ {projected} as {native} }};\n",
                projected = target.projected_name,
                native = target.identity.name,
            ));
            if dts.contains(&format!("IID_{}", target.projected_name)) {
                dts.push_str(&format!(
                    "export {{ IID_{projected} as IID_{native} }};\n",
                    projected = target.projected_name,
                    native = target.identity.name,
                ));
            }
        }
    }
    for alias in &target.compatibility_aliases {
        if target.identity.kind == javascript::JavaScriptTypeKind::Delegate {
            js.push_str(&format!(
                "exports.IID_{alias} = exports.IID_{projected};\n\
                 exports.{alias}_PARAM_TYPES = exports.{projected}_PARAM_TYPES;\n",
                projected = target.projected_name,
            ));
            dts.push_str(&format!(
                "\nexport {{ IID_{projected} as IID_{alias}, \
                 {projected}_PARAM_TYPES as {alias}_PARAM_TYPES }};\n\
                 export type {alias} = {projected};\n",
                projected = target.projected_name,
            ));
        } else {
            if js.contains(&format!("exports.{} =", target.projected_name)) {
                js.push_str(&format!(
                    "exports.{alias} = {projected};\n",
                    projected = target.projected_name,
                ));
            }
            if js.contains(&format!("exports.IID_{} =", target.projected_name)) {
                js.push_str(&format!(
                    "exports.IID_{alias} = exports.IID_{projected};\n",
                    projected = target.projected_name,
                ));
            }
            dts.push_str(&format!(
                "\nexport {{ {projected} as {alias} }};\n",
                projected = target.projected_name,
            ));
            if dts.contains(&format!("IID_{}", target.projected_name)) {
                dts.push_str(&format!(
                    "export {{ IID_{projected} as IID_{alias} }};\n",
                    projected = target.projected_name,
                ));
            }
        }
    }
    let imports = projected
        .imports
        .iter()
        .zip(dependencies)
        .map(|(import, canonical_dependency)| PlannedImport {
            source: import.from.clone(),
            symbols: import.symbols.iter().cloned().collect(),
            runtime_only: import.runtime_only,
            dts_only: import.dts_only,
            is_runtime_package: import.is_runtime_package,
            canonical_dependency,
        })
        .collect();
    Ok(GeneratedModule {
        canonical_module: target.canonical_module.clone(),
        package_subpath: target.canonical_module,
        javascript: Some(js),
        declarations: Some(dts),
        imports,
        public_exports,
        primary_export: (target.identity.kind != javascript::JavaScriptTypeKind::Delegate)
            .then(|| target.projected_name.clone()),
        internal_exports,
        compatibility_aliases: target.compatibility_aliases,
    })
}

fn ensure_safe_generated_parent(output_dir: &Path, destination: &Path) -> Result<(), String> {
    if !output_dir.exists() {
        fs::create_dir_all(output_dir).map_err(|error| {
            format!(
                "Failed to create JavaScript output directory {}: {error}",
                output_dir.display()
            )
        })?;
    }
    let relative = destination.strip_prefix(output_dir).map_err(|_| {
        format!(
            "Generated JavaScript path '{}' is outside output directory '{}'",
            destination.display(),
            output_dir.display()
        )
    })?;
    let relative_parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let mut current = output_dir.to_path_buf();
    for component in relative_parent.components() {
        let std::path::Component::Normal(segment) = component else {
            return Err(format!(
                "Generated JavaScript path '{}' contains an unsafe component",
                destination.display()
            ));
        };
        current.push(segment);
        if current.exists() {
            let metadata = fs::symlink_metadata(&current)
                .map_err(|error| format!("Failed to inspect {}: {error}", current.display()))?;
            if is_link_or_reparse_point(&metadata) {
                return Err(format!(
                    "Refusing to write generated JavaScript through linked directory '{}'",
                    current.display()
                ));
            }
            if !metadata.is_dir() {
                return Err(format!(
                    "Generated JavaScript parent '{}' is not a directory",
                    current.display()
                ));
            }
        } else {
            fs::create_dir(&current)
                .map_err(|error| format!("Failed to create {}: {error}", current.display()))?;
        }
    }
    let resolved_root = fs::canonicalize(output_dir)
        .map_err(|error| format!("Failed to resolve {}: {error}", output_dir.display()))?;
    let resolved_parent = fs::canonicalize(&current)
        .map_err(|error| format!("Failed to resolve {}: {error}", current.display()))?;
    if !resolved_parent.starts_with(&resolved_root) {
        return Err(format!(
            "Resolved JavaScript parent '{}' escapes output directory '{}'",
            resolved_parent.display(),
            resolved_root.display()
        ));
    }
    Ok(())
}

fn is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    let mut is_link = metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        is_link |= metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    is_link
}

fn ensure_generated_tree_has_no_links(output_dir: &Path) -> Result<(), String> {
    fn visit(current: &Path) -> Result<(), String> {
        let entries = fs::read_dir(current)
            .map_err(|error| format!("Failed to read {}: {error}", current.display()))?;
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("Failed to read directory entry: {error}"))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("Failed to inspect {}: {error}", path.display()))?;
            if is_link_or_reparse_point(&metadata) {
                return Err(format!(
                    "Generated JavaScript output contains linked path '{}'",
                    path.display()
                ));
            }
            if metadata.is_dir() {
                visit(&path)?;
            }
        }
        Ok(())
    }
    visit(output_dir)
}

fn ensure_safe_generated_destination(output_dir: &Path, destination: &Path) -> Result<(), String> {
    ensure_safe_generated_parent(output_dir, destination)?;
    match fs::symlink_metadata(destination) {
        Ok(metadata) if is_link_or_reparse_point(&metadata) => Err(format!(
            "Refusing to write generated JavaScript through linked file '{}'",
            destination.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Failed to inspect generated JavaScript destination {}: {error}",
            destination.display()
        )),
    }
}

fn write_generated_javascript_file(
    output_dir: &Path,
    path: &Path,
    content: &str,
) -> Result<(), String> {
    ensure_safe_generated_destination(output_dir, path)?;
    if path.is_file()
        && !fs::read_to_string(path)
            .map(|existing| existing.starts_with("// Generated by dynwinrt-codegen"))
            .unwrap_or(false)
    {
        return Err(format!(
            "Refusing to overwrite non-generated JavaScript binding '{}'",
            path.display()
        ));
    }
    write_file(path, content)
}

fn generate_js_files(
    context: &javascript::JavaScriptProjectionContext,
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
) -> Result<dynwinrt_codegen::codegen::projected::GenerationPlan, String> {
    // Exclusive interfaces paired with a runtime class are implementation
    // details. The projected layout already qualifies genuine cross-namespace
    // collisions, so this name check only removes those class-owned entries.
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

    let all_interface_modules = all_interfaces
        .iter()
        .filter(|iface| !class_names.contains(iface.name.as_str()) && is_emittable_interface(iface))
        .filter_map(|iface| {
            context
                .output_target(&iface.name)
                .map(|target| target.canonical_module.clone())
        })
        .collect::<HashSet<_>>();
    let mut regenerated_modules = all_classes
        .iter()
        .filter_map(|class| {
            context
                .output_target(&class.name)
                .map(|target| target.canonical_module.clone())
        })
        .chain(all_interface_modules.iter().cloned())
        .collect::<HashSet<_>>();
    regenerated_modules.extend(shared_interfaces.iter().filter_map(|iface| {
        (!class_names.contains(iface.name.as_str()) && is_emittable_interface(iface))
            .then(|| context.output_target(&iface.name))
            .flatten()
            .map(|target| target.canonical_module.clone())
    }));
    regenerated_modules.extend(all_enums.iter().filter_map(|en| {
        let TypeMeta::Enum { name, .. } = en else {
            return None;
        };
        (!name.contains('<') && !class_names.contains(name.as_str()))
            .then(|| context.output_target(name))
            .flatten()
            .map(|target| target.canonical_module.clone())
    }));
    let mut plan = load_effective_generation_plan(context, output_dir, &regenerated_modules)?;
    for iface in shared_interfaces {
        if class_names.contains(iface.name.as_str()) {
            continue;
        }
        if !is_emittable_interface(iface) {
            continue;
        }
        if context
            .output_target(&iface.name)
            .is_some_and(|target| all_interface_modules.contains(&target.canonical_module))
        {
            continue;
        }
        let projected = project::project_interface(
            context,
            iface,
            known_types,
            delegate_type_names,
            delegate_sigs,
            delegate_sig_refs,
            delegate_param_wraps,
        );
        plan.insert(emit_javascript_projected_file(context, projected)?)?;
    }
    for iface in all_interfaces {
        if class_names.contains(iface.name.as_str()) {
            continue;
        }
        if !is_emittable_interface(iface) {
            continue;
        }
        let projected = project::project_interface(
            context,
            iface,
            known_types,
            delegate_type_names,
            delegate_sigs,
            delegate_sig_refs,
            delegate_param_wraps,
        );
        plan.insert(emit_javascript_projected_file(context, projected)?)?;
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
                plan.insert(emit_javascript_projected_file(context, projected)?)?;
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
            context,
            class,
            known_types,
            delegate_type_names,
            shared_iids,
            delegate_sigs,
            delegate_sig_refs,
            delegate_param_wraps,
        );
        plan.insert(emit_javascript_projected_file(context, projected)?)?;
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
        let mut stub_js = format!(
            "// Generated by dynwinrt-codegen \u{2014} do not edit\n\
             // Placeholder for a class whose default interface has no IID in\n\
             // the loaded winmd graph. Any attempt to use it will throw.\n\
             const __unavailable = () => {{ throw new Error(\"'{name}' has no default interface in the loaded winmd graph and cannot be constructed. Add its owning package to `additionalWinmds` / `additionalRefs`.\"); }};\n\
             class {name} {{ constructor() {{ __unavailable(); }} }}\n\
             exports.{name} = {name};\n",
            name = class.name,
        );
        let mut stub_dts = format!(
            "// Generated by dynwinrt-codegen \u{2014} do not edit\n\
             // Placeholder: throwing at construction. Typed as a class so
             // other .d.ts files can still use `{name}` as a parameter /
             // return type.\n\
             export declare class {name} {{ private constructor(); }}\n",
            name = class.name,
        );
        let target = context.output_target(&class.name).ok_or_else(|| {
            format!(
                "JavaScript module layout does not contain projected type `{}`",
                class.name
            )
        })?;
        if target.collides {
            stub_js.push_str(&format!(
                "exports.{native} = {projected};\n",
                native = target.identity.name,
                projected = target.projected_name,
            ));
            stub_dts.push_str(&format!(
                "export {{ {projected} as {native} }};\n",
                projected = target.projected_name,
                native = target.identity.name,
            ));
        }
        for alias in &target.compatibility_aliases {
            stub_js.push_str(&format!(
                "exports.{alias} = {projected};\n",
                projected = target.projected_name,
            ));
            stub_dts.push_str(&format!(
                "export {{ {projected} as {alias} }};\n",
                projected = target.projected_name,
            ));
        }
        plan.insert(dynwinrt_codegen::codegen::projected::GeneratedModule {
            canonical_module: target.canonical_module.clone(),
            package_subpath: target.canonical_module.clone(),
            javascript: Some(stub_js),
            declarations: Some(stub_dts),
            imports: Vec::new(),
            public_exports: [target.projected_name.clone()].into_iter().collect(),
            primary_export: Some(target.projected_name.clone()),
            internal_exports: BTreeSet::new(),
            compatibility_aliases: target.compatibility_aliases.clone(),
        })?;
        emitted_class_names.insert(class.name.clone());
    }

    plan.validate_dependencies()?;
    write_generation_plan_modules(output_dir, &plan)?;
    Ok(plan)
}

/// Write the four root barrel entries plus `package.json` alongside canonical
/// namespaced implementations.
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
/// Root barrels point directly at canonical modules. Package subpaths expose
/// only canonical namespace paths.
fn write_js_barrel_and_manifest(
    output_dir: &Path,
    plan: &mut dynwinrt_codegen::codegen::projected::GenerationPlan,
) -> Result<(), String> {
    use dynwinrt_codegen::codegen::projected::GeneratedModule;

    let js_path = output_dir.join("index.js");
    let mjs_path = output_dir.join("index.mjs");
    let proxy_path = output_dir.join("index.proxy.js");
    let dts_path = output_dir.join("index.d.ts");
    write_lifetime_module(output_dir)?;
    let lifetime = plan
        .modules
        .entry("lifetime".into())
        .or_insert_with(|| GeneratedModule::retained("lifetime"));
    lifetime.public_exports.extend(
        [
            "createProjectedLifetimeScope",
            "projectAs",
            "releaseProjected",
        ]
        .into_iter()
        .map(str::to_string),
    );

    // Clean up any stale `.index.ts` cache from older codegen versions.
    let stale = output_dir.join(".index.ts");
    if stale.exists() {
        ensure_safe_generated_destination(output_dir, &stale)?;
        fs::remove_file(&stale)
            .map_err(|error| format!("Failed to remove {}: {error}", stale.display()))?;
    }

    // Remove the previous opt-in getter barrel name if it exists from older
    // generated output. `index.js` is now the getter barrel and
    // `index.proxy.js` is the explicit compatibility path.
    let stale_getter = output_dir.join("index.getter.js");
    if stale_getter.exists() {
        ensure_safe_generated_destination(output_dir, &stale_getter)?;
        fs::remove_file(&stale_getter)
            .map_err(|error| format!("Failed to remove {}: {error}", stale_getter.display()))?;
    }

    plan.validate_dependencies()?;
    let index_content = plan.render_root_index();

    let js_content = typescript::esm_index_to_cjs_getter(&index_content);
    ensure_safe_generated_destination(output_dir, &js_path)?;
    fs::write(&js_path, &js_content)
        .map_err(|e| format!("Failed to write {}: {}", js_path.display(), e))?;

    let mjs_content = typescript::esm_index_to_esm(&index_content);
    ensure_safe_generated_destination(output_dir, &mjs_path)?;
    fs::write(&mjs_path, &mjs_content)
        .map_err(|e| format!("Failed to write {}: {}", mjs_path.display(), e))?;

    let proxy_content = typescript::esm_index_to_cjs_lazy(&index_content);
    ensure_safe_generated_destination(output_dir, &proxy_path)?;
    fs::write(&proxy_path, &proxy_content)
        .map_err(|e| format!("Failed to write {}: {}", proxy_path.display(), e))?;

    ensure_safe_generated_destination(output_dir, &dts_path)?;
    fs::write(&dts_path, &index_content)
        .map_err(|e| format!("Failed to write {}: {}", dts_path.display(), e))?;

    write_bindings_manifest_with_plan(output_dir, Some(plan))?;

    println!("Generated {}", js_path.display());
    Ok(())
}

fn prepare_com_generation_manifest(
    com_output_dir: &Path,
    updated_roots: &BTreeMap<String, BTreeSet<String>>,
) -> Result<ComManifestUpdate, String> {
    let path = com_output_dir.join(COM_MANIFEST_FILE);
    let mut manifest = if path.exists() {
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        serde_json::from_str::<ComGenerationManifest>(&content).map_err(|error| {
            format!(
                "Invalid COM generation manifest {}: {error}",
                path.display()
            )
        })?
    } else {
        ComGenerationManifest {
            version: 1,
            roots: BTreeMap::new(),
        }
    };
    if manifest.version != 1 {
        return Err(format!(
            "Unsupported COM generation manifest version {} in {}",
            manifest.version,
            path.display()
        ));
    }

    let previous_files = updated_roots
        .keys()
        .filter_map(|root| manifest.roots.get(root))
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>();
    for (root, files) in updated_roots {
        manifest.roots.insert(root.clone(), files.clone());
    }
    let retained_files = manifest
        .roots
        .values()
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>();
    let stale_files = previous_files
        .difference(&retained_files)
        .cloned()
        .collect::<BTreeSet<_>>();
    for stale in &stale_files {
        let relative = Path::new(stale);
        if relative.components().count() != 1
            || !(stale.ends_with(".js") || stale.ends_with(".d.ts"))
        {
            return Err(format!(
                "Refusing unsafe path `{stale}` in COM generation manifest {}",
                path.display()
            ));
        }
    }
    Ok(ComManifestUpdate {
        manifest,
        stale_files,
    })
}

fn apply_com_generation_manifest(
    com_output_dir: &Path,
    update: ComManifestUpdate,
) -> Result<(), String> {
    let output_dir = com_output_dir.parent().unwrap_or(com_output_dir);
    let path = com_output_dir.join(COM_MANIFEST_FILE);
    for stale in &update.stale_files {
        let stale_path = com_output_dir.join(stale);
        ensure_safe_generated_destination(output_dir, &stale_path)?;
        if stale_path.exists() {
            fs::remove_file(&stale_path)
                .map_err(|error| format!("Failed to remove {}: {error}", stale_path.display()))?;
        }
    }
    let content = serde_json::to_string_pretty(&update.manifest)
        .map_err(|error| format!("Failed to serialize COM generation manifest: {error}"))?;
    ensure_safe_generated_destination(output_dir, &path)?;
    fs::write(&path, format!("{content}\n"))
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))
}

fn write_com_js_barrel(com_output_dir: &Path) -> Result<(), String> {
    let output_dir = com_output_dir.parent().unwrap_or(com_output_dir);
    ensure_safe_generated_parent(output_dir, &com_output_dir.join(".dynwinrt-write-check"))?;
    let mut modules: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut contents: BTreeMap<String, String> = BTreeMap::new();
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
            contents.insert(module.to_string(), content);
        }
    }
    deduplicate_com_barrel_exports(&mut modules, &contents)?;

    let mut index = String::from("// Generated by dynwinrt-codegen - do not edit\n");
    for (module, exports) in &modules {
        index.push_str(&format!(
            "export {{ {} }} from './{module}.js';\n",
            exports.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    let cjs_index = typescript::esm_index_to_cjs_getter(&index);
    let esm_index = typescript::esm_index_to_esm(&index);
    let index_js = com_output_dir.join("index.js");
    let index_mjs = com_output_dir.join("index.mjs");
    let index_dts = com_output_dir.join("index.d.ts");
    let package_json = com_output_dir.join("package.json");
    for path in [&index_js, &index_mjs, &index_dts, &package_json] {
        ensure_safe_generated_destination(output_dir, path)?;
    }
    fs::write(&index_js, &cjs_index)
        .map_err(|error| format!("Failed to write COM index.js: {error}"))?;
    fs::write(&index_mjs, &esm_index)
        .map_err(|error| format!("Failed to write COM index.mjs: {error}"))?;
    fs::write(&index_dts, &index)
        .map_err(|error| format!("Failed to write COM index.d.ts: {error}"))?;

    let package = "{\n  \"type\": \"commonjs\",\n  \"sideEffects\": false\n}\n";
    fs::write(&package_json, package)
        .map_err(|error| format!("Failed to write COM package boundary: {error}"))?;
    Ok(())
}

fn deduplicate_com_barrel_exports(
    modules: &mut BTreeMap<String, BTreeSet<String>>,
    contents: &BTreeMap<String, String>,
) -> Result<(), String> {
    let mut owners = BTreeMap::<String, (String, Option<String>)>::new();
    for (module, exports) in modules.iter_mut() {
        let content = &contents[module];
        let names = exports.iter().cloned().collect::<Vec<_>>();
        for name in names {
            let signature = native_pod_factory_signature(content, &name);
            if let Some((owner, owner_signature)) = owners.get(&name) {
                if signature.is_some() && signature.as_ref() == owner_signature.as_ref() {
                    exports.remove(&name);
                } else {
                    return Err(format!(
                        "COM barrel export `{name}` is ambiguous between `{owner}.js` and `{module}.js`"
                    ));
                }
            } else {
                owners.insert(name, (module.clone(), signature));
            }
        }
    }
    Ok(())
}

fn native_pod_factory_signature(content: &str, export_name: &str) -> Option<String> {
    let pod_name = export_name.strip_prefix("create")?;
    if pod_name.is_empty() {
        return None;
    }
    let pod_name = pod_name.strip_suffix("Array").unwrap_or(pod_name);
    [
        format!("const _nativeLayout_{pod_name} = "),
        format!("const _nativeUnionLayout_{pod_name} = "),
    ]
    .into_iter()
    .find_map(|prefix| {
        content
            .lines()
            .find(|line| line.starts_with(&prefix))
            .map(str::to_string)
    })
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
    ensure_safe_generated_destination(output_dir, &com_js_path)?;
    ensure_safe_generated_destination(output_dir, &com_dts_path)?;
    let com_js = fs::read_to_string(&com_js_path)
        .map_err(|error| format!("Failed to read {}: {error}", com_js_path.display()))?;
    let com_dts = fs::read_to_string(&com_dts_path)
        .map_err(|error| format!("Failed to read {}: {error}", com_dts_path.display()))?;
    let root_js = com_js.replace(", './", ", './com/");
    let root_dts = com_dts.replace("from './", "from './com/");
    let index_js = output_dir.join("index.js");
    let index_dts = output_dir.join("index.d.ts");
    ensure_safe_generated_destination(output_dir, &index_js)?;
    ensure_safe_generated_destination(output_dir, &index_dts)?;
    fs::write(&index_js, &root_js)
        .map_err(|error| format!("Failed to write COM compatibility index.js: {error}"))?;
    fs::write(&index_dts, root_dts)
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
    ensure_safe_generated_parent(
        output_dir,
        &com_output_dir.join(".dynwinrt-migration-check"),
    )?;
    for module in modules {
        for suffix in [".js", ".d.ts"] {
            move_legacy_com_file(
                output_dir,
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
            ensure_safe_generated_destination(output_dir, &path)?;
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

fn move_legacy_com_file(
    output_dir: &Path,
    source: &Path,
    destination: &Path,
) -> Result<(), String> {
    if !source.exists() {
        return Ok(());
    }
    ensure_safe_generated_destination(output_dir, source)?;
    ensure_safe_generated_destination(output_dir, destination)?;
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
    write_bindings_manifest_with_plan(output_dir, None)
}

fn write_bindings_manifest_with_plan(
    output_dir: &Path,
    plan: Option<&dynwinrt_codegen::codegen::projected::GenerationPlan>,
) -> Result<(), String> {
    let has_winrt_root = has_winrt_root(output_dir);
    let winrt_subpath_names = if let Some(plan) = plan {
        plan.package_subpaths()
    } else if has_winrt_root {
        collect_subpath_names_from_dir(output_dir)?
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
    ensure_safe_generated_destination(output_dir, &path)?;
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
/**\n\
 * Borrows a raw projected value or wrapper and exposes it as a generated type.\n\
 * The input remains valid; the returned projection owns its interface view and\n\
 * must be released independently or retained by a projected lifetime scope.\n\
 */\n\
export declare function projectAs<T extends object>(value: unknown, type: ProjectedType<T>): T;\n\
export declare function releaseProjected(value: object): void;\n\
export interface ProjectedLifetimeScope {\n\
  readonly disposed: boolean;\n\
  dispose(): void;\n\
}\n\
export declare function createProjectedLifetimeScope(): ProjectedLifetimeScope;\n";
    let js_path = output_dir.join("lifetime.js");
    let dts_path = output_dir.join("lifetime.d.ts");
    ensure_safe_generated_destination(output_dir, &js_path)?;
    ensure_safe_generated_destination(output_dir, &dts_path)?;
    fs::write(&js_path, js).map_err(|e| format!("Failed to write lifetime.js: {e}"))?;
    fs::write(&dts_path, dts).map_err(|e| format!("Failed to write lifetime.d.ts: {e}"))?;
    Ok(())
}

/// Enumerate generated `.js` modules recursively and return package subpaths
/// without extensions. Excludes root/nested barrels and the domain-specific
/// Classic COM tree, which is added separately.
fn collect_subpath_names_from_dir(output_dir: &Path) -> Result<BTreeSet<String>, String> {
    const BARREL_STEMS: &[&str] = &["index", "index.getter", "index.proxy"];
    if output_dir.join(JAVASCRIPT_TYPE_INVENTORY).is_file() {
        let records = read_javascript_type_inventory(output_dir)?.records;
        let context = javascript::create_javascript_projection_context_with_records(
            records.iter().map(|record| record.identity.clone()),
            records.iter().cloned(),
            "@microsoft/dynwinrt",
        )?;
        let mut names = context
            .output_targets()
            .into_iter()
            .map(|target| target.canonical_module.clone())
            .collect::<BTreeSet<_>>();
        if output_dir.join("lifetime.js").is_file() {
            names.insert("lifetime".into());
        }
        return Ok(names);
    }

    // Legacy flat output has no identity inventory. Only inspect generated
    // root files; never recurse into an application tree or dependencies when
    // `--output .` is used.
    let mut names = BTreeSet::new();
    let entries = fs::read_dir(output_dir)
        .map_err(|error| format!("Failed to read {}: {error}", output_dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("Failed to read directory entry: {error}"))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Failed to inspect {}: {error}", path.display()))?;
        if is_link_or_reparse_point(&metadata) || !metadata.is_file() {
            continue;
        }
        let Some(fname) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(stem) = fname.strip_suffix(".js") else {
            continue;
        };
        if BARREL_STEMS.iter().any(|barrel| stem == *barrel) {
            continue;
        }
        let generated = fs::read_to_string(&path)
            .map(|content| content.starts_with("// Generated by dynwinrt-codegen"))
            .unwrap_or(false);
        if generated || stem == "lifetime" {
            names.insert(stem.into());
        }
    }
    Ok(names)
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

    write_file(
        &output_dir.join("_runtime.py"),
        &python::generate_runtime_support_module(),
    )?;
    if pyi {
        write_file(
            &output_dir.join("_runtime.pyi"),
            &python_stub::generate_runtime_support_stub(),
        )?;
        write_file(
            &output_dir.join("_typing.pyi"),
            &python_stub::generate_typing_support_module(),
        )?;
    }

    let mut generated_modules = HashSet::new();
    let mut struct_interfaces = all_interfaces.to_vec();
    struct_interfaces.extend_from_slice(shared_interfaces);
    let structs = python::package_structs(all_classes, &struct_interfaces);

    // A runtime class owns its public identity when metadata also exposes an
    // interface or enum with the same namespace/name. Generate in precedence
    // order and render each final module exactly once.
    for class in all_classes {
        let module = python::python_module_name(&class.namespace, &class.name);
        if !generated_modules.insert(module.clone()) {
            continue;
        }
        let code = python::generate_class(class, known_types, delegate_type_names, shared_iids);
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
            write_file(&output_dir.join(format!("{module}.pyi")), &stub)?;
        }
    }
    for en in all_enums {
        if let TypeMeta::Enum {
            namespace, name, ..
        } = en
        {
            let module = python::python_module_name(namespace, name);
            if !generated_modules.insert(module.clone()) {
                continue;
            }
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
    for iface in all_interfaces.iter().chain(shared_interfaces) {
        let module = python::python_module_name(&iface.namespace, &iface.name);
        if !generated_modules.insert(module.clone()) {
            continue;
        }
        let code = python::generate_interface(iface, known_types, delegate_type_names);
        let filepath = output_dir.join(format!("{module}.py"));
        write_file(&filepath, &code)?;
        println!("Generated {}", filepath.display());
        if pyi {
            let stub =
                python_stub::generate_interface_stub(iface, known_types, delegate_type_names);
            write_file(&output_dir.join(format!("{module}.pyi")), &stub)?;
        }
    }
    for typ in &structs {
        let TypeMeta::Struct {
            namespace, name, ..
        } = typ
        else {
            continue;
        };
        let module = python::python_module_name(namespace, name);
        if !generated_modules.insert(module.clone()) {
            return Err(format!(
                "Python struct module `{namespace}.{name}` collides with another generated type"
            ));
        }
        if let Some(code) = python::generate_struct(typ) {
            let filepath = output_dir.join(format!("{module}.py"));
            write_file(&filepath, &code)?;
            println!("Generated {}", filepath.display());
        }
        if pyi {
            if let Some(stub) = python_stub::generate_struct_stub(typ) {
                write_file(&output_dir.join(format!("{module}.pyi")), &stub)?;
            }
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
    structs: Vec<TypeMeta>,
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

    let current_structs = python::package_structs(classes, interfaces);
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
    let root_structs = current_structs
        .iter()
        .filter(|typ| matches!(typ, TypeMeta::Struct { name, .. } if counts[name] == 1))
        .cloned()
        .collect::<Vec<_>>();

    let mut root_index =
        python::generate_public_index(&root_classes, &root_interfaces, &root_enums);
    root_index.push_str(
        python::generate_public_struct_index(&root_structs)
            .strip_prefix(GENERATED_PYTHON_HEADER)
            .unwrap_or_default(),
    );
    write_python_lazy_root_index(
        &output_dir.join("__init__.py"),
        &root_index,
        append,
        &suppressed_root_names,
    )?;
    if pyi {
        let mut root_stub =
            python_stub::generate_public_index_stub(&root_classes, &root_interfaces, &root_enums);
        root_stub.push_str(
            python_stub::generate_public_struct_index_stub(&root_structs)
                .strip_prefix(GENERATED_PYTHON_HEADER)
                .unwrap_or_default(),
        );
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
    for typ in current_structs {
        if let TypeMeta::Struct { namespace, .. } = &typ {
            groups
                .entry(namespace.clone())
                .or_default()
                .structs
                .push(typ);
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
    for typ in &group.structs {
        let TypeMeta::Struct { name, .. } = typ else {
            continue;
        };
        if !seen.insert(name.clone()) {
            continue;
        }
        let runtime = python::generate_struct_index(std::slice::from_ref(typ));
        if runtime.lines().any(|line| line.starts_with("from .")) {
            write_python_facade(
                &package_dir,
                &segments,
                name,
                &runtime,
                "py",
                &mut runtime_exports,
            )?;
        }
        if pyi {
            let stub = python_stub::generate_struct_index_stub(std::slice::from_ref(typ));
            if stub.lines().any(|line| line.starts_with("from .")) {
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
    }

    let runtime_index = format!("{}{}", GENERATED_PYTHON_HEADER, runtime_exports.join("\n"));
    let suppressed_root_names = HashSet::new();
    write_python_lazy_root_index(
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
    let exports = exports.split('#').next().unwrap_or(exports).trim();
    let exported_type = exports.split(',').any(|export| {
        export
            .trim()
            .split_once(" as ")
            .map_or_else(|| export.trim(), |(_, alias)| alias.trim())
            == type_name
    });
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
    let mut facade =
        format!("{GENERATED_PYTHON_HEADER}from {relative_root}{source} import {exports}\n");
    if extension == "py" && exported_type {
        facade.push_str(&format!("\n{type_name}.__module__ = __name__\n"));
    }
    let public_module = python::python_public_module_name(type_name);
    write_file(
        &package_dir.join(format!("{public_module}.{extension}")),
        &facade,
    )?;
    if exported_type {
        let package_export = if extension == "pyi" {
            format!("from .{public_module} import {type_name} as {type_name}")
        } else {
            format!("from .{public_module} import {type_name}")
        };
        package_exports.push(package_export);
    }
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

fn write_python_lazy_root_index(
    path: &Path,
    generated: &str,
    append: bool,
    suppressed_names: &HashSet<String>,
) -> Result<(), String> {
    let existing = if append && path.exists() {
        fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))?
    } else {
        GENERATED_PYTHON_HEADER.to_string()
    };
    let content = merge_python_lazy_root_indexes(&existing, generated, suppressed_names);
    write_file(path, &content)
}

fn merge_python_lazy_root_indexes(
    existing: &str,
    generated: &str,
    suppressed_names: &HashSet<String>,
) -> String {
    let mut exports = BTreeMap::<String, (String, String)>::new();
    collect_python_root_exports(existing, suppressed_names, &mut exports);
    collect_python_root_exports(generated, suppressed_names, &mut exports);

    let mut out = String::from(GENERATED_PYTHON_HEADER);
    out.push_str("from importlib import import_module as _import_module\n\n");
    out.push_str("__all__ = (\n");
    for name in exports.keys() {
        out.push_str(&format!("    \"{name}\",\n"));
    }
    out.push_str(")\n\n");
    out.push_str("_EXPORTS = {\n");
    for (name, (module, symbol)) in &exports {
        out.push_str(&format!("    \"{name}\": (\"{module}\", \"{symbol}\"),\n"));
    }
    out.push_str(
        "}\n\n\
         \n\
         def __getattr__(name):\n\
         \x20   try:\n\
         \x20       module_name, symbol_name = _EXPORTS[name]\n\
         \x20   except KeyError:\n\
         \x20       raise AttributeError(\n\
         \x20           f\"module {__name__!r} has no attribute {name!r}\"\n\
         \x20       ) from None\n\
         \x20   value = getattr(_import_module(module_name, __name__), symbol_name)\n\
         \x20   globals()[name] = value\n\
         \x20   return value\n\
         \n\
         \n\
         def __dir__():\n\
         \x20   return sorted(set(globals()) | set(__all__))\n",
    );
    out
}

fn collect_python_root_exports(
    content: &str,
    suppressed_names: &HashSet<String>,
    exports: &mut BTreeMap<String, (String, String)>,
) {
    for line in content.lines() {
        if let Some((module, symbols)) = parse_python_import_line(line) {
            for (source_symbol, exported_symbol) in symbols {
                if suppressed_names.contains(&source_symbol)
                    || suppressed_names.contains(&exported_symbol)
                {
                    continue;
                }
                exports
                    .entry(exported_symbol)
                    .or_insert_with(|| (module.clone(), source_symbol));
            }
        } else if let Some((exported_symbol, module, source_symbol)) =
            parse_python_lazy_export(line)
        {
            if suppressed_names.contains(&source_symbol)
                || suppressed_names.contains(&exported_symbol)
            {
                continue;
            }
            exports
                .entry(exported_symbol)
                .or_insert((module, source_symbol));
        }
    }
}

fn parse_python_import_line(line: &str) -> Option<(String, Vec<(String, String)>)> {
    let line = line.split_once("  #").map_or(line, |(line, _)| line);
    let (source, exports) = line.split_once(" import ")?;
    let module = source.strip_prefix("from ")?.to_string();
    if !module.starts_with('.') {
        return None;
    }
    let exports = exports
        .split(',')
        .map(str::trim)
        .filter(|export| !export.is_empty())
        .map(|export| {
            export.split_once(" as ").map_or_else(
                || (export.to_string(), export.to_string()),
                |(source, alias)| (source.trim().to_string(), alias.trim().to_string()),
            )
        })
        .collect();
    Some((module, exports))
}

fn parse_python_lazy_export(line: &str) -> Option<(String, String, String)> {
    let line = line.trim();
    let line = line.strip_prefix('"')?;
    let (exported_symbol, line) = line.split_once("\": (\"")?;
    let (module, source_symbol) = line.split_once("\", \"")?;
    let source_symbol = source_symbol.strip_suffix("\"),")?;
    Some((
        exported_symbol.to_string(),
        module.to_string(),
        source_symbol.to_string(),
    ))
}

fn merge_python_indexes(
    existing: &str,
    generated: &str,
    suppressed_names: &HashSet<String>,
) -> String {
    let mut imports = BTreeSet::new();
    let mut exported_symbols = HashSet::new();
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
                    let source_symbol = export
                        .split_once(" as ")
                        .map_or(*export, |(name, _)| name.trim());
                    let exported_symbol = export
                        .split_once(" as ")
                        .map_or(*export, |(_, alias)| alias.trim());
                    !suppressed_names.contains(source_symbol)
                        && exported_symbols.insert(exported_symbol.to_string())
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

fn record_python_supplemental_types(
    output_dir: &Path,
    interfaces: &[meta::InterfaceMeta],
) -> Result<(), String> {
    if interfaces.is_empty() {
        return Ok(());
    }
    let mut types = read_python_type_inventory(output_dir)?;
    types.extend(python_generated_types(&[], interfaces, &[]));
    let mut seen = HashSet::new();
    types.retain(|typ| seen.insert(typ.clone()));
    write_python_type_inventory(output_dir, &types)
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

fn output_contains_current_directory(output_dir: &Path) -> Result<bool, String> {
    if output_dir.as_os_str().is_empty() {
        return Err("Generated output directory cannot be empty.".into());
    }
    let current = std::env::current_dir()
        .and_then(fs::canonicalize)
        .map_err(|error| format!("Failed to resolve current directory: {error}"))?;
    let absolute = if output_dir.is_absolute() {
        output_dir.to_path_buf()
    } else {
        current.join(output_dir)
    };
    let output = if absolute.exists() {
        fs::canonicalize(&absolute)
            .map_err(|error| format!("Failed to resolve {}: {error}", absolute.display()))?
    } else {
        let mut ancestor = absolute.as_path();
        let mut missing = Vec::new();
        while !ancestor.exists() {
            let leaf = ancestor.file_name().ok_or_else(|| {
                format!(
                    "Invalid generated output directory '{}'",
                    output_dir.display()
                )
            })?;
            missing.push(leaf.to_os_string());
            ancestor = ancestor.parent().ok_or_else(|| {
                format!(
                    "Invalid generated output directory '{}'",
                    output_dir.display()
                )
            })?;
        }
        let mut resolved = fs::canonicalize(ancestor)
            .map_err(|error| format!("Failed to resolve {}: {error}", ancestor.display()))?;
        for component in missing.into_iter().rev() {
            resolved.push(component);
        }
        resolved
    };
    Ok(current.starts_with(output))
}

struct OutputTransaction {
    final_dir: PathBuf,
    stage_dir: PathBuf,
    backup_dir: PathBuf,
    nonce: String,
    had_existing_output: bool,
    lock_file: Option<fs::File>,
    committed: bool,
}

const OUTPUT_TRANSACTION_OWNER: &str = ".dynwinrt-transaction-owner";

fn transaction_nonce(final_dir: &Path) -> String {
    static NEXT_NONCE: AtomicU64 = AtomicU64::new(0);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_NONCE.fetch_add(1, Ordering::Relaxed);
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    final_dir.hash(&mut hasher);
    timestamp.hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    sequence.hash(&mut hasher);
    format!("{timestamp:032x}{:016x}", hasher.finish())
}

fn transaction_owner_path(directory: &Path) -> PathBuf {
    directory.join(OUTPUT_TRANSACTION_OWNER)
}

fn write_transaction_owner(directory: &Path, nonce: &str) -> Result<(), String> {
    let marker = transaction_owner_path(directory);
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&marker)
        .map_err(|error| format!("Failed to create {}: {error}", marker.display()))?;
    use std::io::Write;
    file.write_all(nonce.as_bytes())
        .map_err(|error| format!("Failed to write {}: {error}", marker.display()))
}

fn validate_transaction_owner(directory: &Path, nonce: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(directory)
        .map_err(|error| format!("Failed to inspect {}: {error}", directory.display()))?;
    if is_link_or_reparse_point(&metadata) || !metadata.is_dir() {
        return Err(format!(
            "Invalid generated output transaction directory '{}'",
            directory.display()
        ));
    }
    let marker = transaction_owner_path(directory);
    let metadata = fs::symlink_metadata(&marker)
        .map_err(|error| format!("Missing ownership marker '{}': {error}", marker.display()))?;
    if is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(format!(
            "Invalid transaction ownership marker '{}'",
            marker.display()
        ));
    }
    let actual = fs::read_to_string(&marker)
        .map_err(|error| format!("Failed to read {}: {error}", marker.display()))?;
    if actual != nonce {
        return Err(format!(
            "Transaction ownership marker '{}' does not match the active transaction",
            marker.display()
        ));
    }
    Ok(())
}

fn remove_transaction_owner(directory: &Path, nonce: &str) -> Result<(), String> {
    validate_transaction_owner(directory, nonce)?;
    let marker = transaction_owner_path(directory);
    fs::remove_file(&marker)
        .map_err(|error| format!("Failed to remove {}: {error}", marker.display()))
}

fn remove_owned_transaction_dir(directory: &Path, nonce: &str) -> Result<(), String> {
    if !directory.exists() {
        return Ok(());
    }
    validate_transaction_owner(directory, nonce)?;
    fs::remove_dir_all(directory)
        .map_err(|error| format!("Failed to remove {}: {error}", directory.display()))
}

fn ensure_no_orphaned_transaction_artifacts(parent: &Path, leaf: &str) -> Result<(), String> {
    let stage_prefix = format!(".{leaf}.dynwinrt-stage-").to_ascii_lowercase();
    let backup_prefix = format!(".{leaf}.dynwinrt-backup-").to_ascii_lowercase();
    for entry in fs::read_dir(parent)
        .map_err(|error| format!("Failed to inspect {}: {error}", parent.display()))?
    {
        let entry =
            entry.map_err(|error| format!("Failed to inspect transaction entry: {error}"))?;
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.starts_with(&stage_prefix) || name.starts_with(&backup_prefix) {
            return Err(format!(
                "Incomplete generated output transaction artifact '{}' must be recovered or removed manually",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

impl OutputTransaction {
    fn begin(requested_final_dir: &Path) -> Result<Self, String> {
        let absolute = if requested_final_dir.is_absolute() {
            requested_final_dir.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| format!("Failed to resolve current directory: {e}"))?
                .join(requested_final_dir)
        };
        let final_dir = if absolute.exists() {
            let metadata = fs::symlink_metadata(&absolute)
                .map_err(|e| format!("Failed to inspect {}: {}", absolute.display(), e))?;
            if is_link_or_reparse_point(&metadata) {
                return Err(format!(
                    "Generated output path '{}' cannot be a linked directory",
                    requested_final_dir.display()
                ));
            }
            fs::canonicalize(&absolute)
                .map_err(|e| format!("Failed to resolve {}: {}", absolute.display(), e))?
        } else {
            let leaf = absolute.file_name().ok_or_else(|| {
                format!(
                    "Invalid generated output directory '{}'",
                    requested_final_dir.display()
                )
            })?;
            let parent = absolute.parent().unwrap_or_else(|| Path::new("."));
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
            fs::canonicalize(parent)
                .map_err(|e| format!("Failed to resolve {}: {}", parent.display(), e))?
                .join(leaf)
        };
        let parent = final_dir.parent().unwrap_or_else(|| Path::new("."));
        let leaf = final_dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                format!(
                    "Invalid generated output directory '{}'",
                    requested_final_dir.display()
                )
            })?;
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;

        let lock_path = parent.join(format!(".{}.dynwinrt-lock", leaf));
        if lock_path.exists() {
            let metadata = fs::symlink_metadata(&lock_path)
                .map_err(|error| format!("Failed to inspect {}: {error}", lock_path.display()))?;
            if is_link_or_reparse_point(&metadata) || !metadata.is_file() {
                return Err(format!(
                    "Invalid generated output lock '{}'",
                    lock_path.display()
                ));
            }
        }
        let lock_file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|error| {
                format!(
                    "Failed to open generated output lock '{}': {error}",
                    lock_path.display()
                )
            })?;
        lock_file.try_lock().map_err(|error| {
            format!(
                "Another generation is already using output directory '{}': {error}",
                final_dir.display()
            )
        })?;

        ensure_no_orphaned_transaction_artifacts(parent, leaf)?;
        let nonce = transaction_nonce(&final_dir);
        let stage_dir = parent.join(format!(".{leaf}.dynwinrt-stage-{nonce}"));
        let backup_dir = parent.join(format!(".{leaf}.dynwinrt-backup-{nonce}"));
        if stage_dir.exists() || backup_dir.exists() {
            return Err("Generated output transaction nonce collision".into());
        }
        fs::create_dir(&stage_dir)
            .map_err(|error| format!("Failed to create {}: {error}", stage_dir.display()))?;
        if let Err(error) = write_transaction_owner(&stage_dir, &nonce) {
            let _ = fs::remove_dir(&stage_dir);
            return Err(error);
        }
        let had_existing_output = final_dir.exists();
        if had_existing_output {
            if !final_dir.is_dir() {
                let _ = remove_owned_transaction_dir(&stage_dir, &nonce);
                return Err(format!(
                    "Generated output path '{}' is not a directory",
                    final_dir.display()
                ));
            }
            if transaction_owner_path(&final_dir).exists() {
                let _ = remove_owned_transaction_dir(&stage_dir, &nonce);
                return Err(format!(
                    "Generated output '{}' contains a reserved transaction ownership marker",
                    final_dir.display()
                ));
            }
            if let Err(error) = copy_directory(&final_dir, &stage_dir) {
                let _ = remove_owned_transaction_dir(&stage_dir, &nonce);
                return Err(error);
            }
        }

        Ok(Self {
            final_dir,
            stage_dir,
            backup_dir,
            nonce,
            had_existing_output,
            lock_file: Some(lock_file),
            committed: false,
        })
    }

    fn stage_dir(&self) -> &Path {
        &self.stage_dir
    }

    fn commit(mut self) -> Result<(), String> {
        validate_transaction_owner(&self.stage_dir, &self.nonce)?;
        let had_existing_output = self.had_existing_output;
        let cwd_relative = std::env::current_dir()
            .ok()
            .and_then(|cwd| fs::canonicalize(cwd).ok())
            .and_then(|cwd| {
                cwd.strip_prefix(&self.final_dir)
                    .ok()
                    .map(Path::to_path_buf)
            });
        if cwd_relative.is_some() {
            let parent = self.final_dir.parent().unwrap_or_else(|| Path::new("."));
            std::env::set_current_dir(parent).map_err(|e| {
                format!(
                    "Failed to leave generated output directory '{}' before replacement: {}",
                    self.final_dir.display(),
                    e
                )
            })?;
        }
        let cwd_output_dir = self.final_dir.clone();
        let restore_cwd = |relative: &Option<PathBuf>| -> Result<(), String> {
            if let Some(relative) = relative {
                let destination = cwd_output_dir.join(relative);
                std::env::set_current_dir(&destination).map_err(|e| {
                    format!(
                        "Failed to restore current directory '{}': {}",
                        destination.display(),
                        e
                    )
                })?;
            }
            Ok(())
        };
        if had_existing_output {
            write_transaction_owner(&self.final_dir, &self.nonce)?;
            if let Err(error) = fs::rename(&self.final_dir, &self.backup_dir) {
                let _ = remove_transaction_owner(&self.final_dir, &self.nonce);
                let restore_error = restore_cwd(&cwd_relative).err();
                return Err(format!(
                    "Failed to stage existing output '{}' for replacement: {}",
                    self.final_dir.display(),
                    error
                ) + &restore_error
                    .map(|error| format!(". {error}"))
                    .unwrap_or_default());
            }
        }

        if let Err(error) = fs::rename(&self.stage_dir, &self.final_dir) {
            if had_existing_output {
                if let Err(rollback_error) = fs::rename(&self.backup_dir, &self.final_dir) {
                    return Err(format!(
                        "Failed to replace generated output directory '{}': {}. Rollback also failed: \
                         {}. The original output remains at '{}'",
                        self.final_dir.display(),
                        error,
                        rollback_error,
                        self.backup_dir.display()
                    ));
                }
                let _ = remove_transaction_owner(&self.final_dir, &self.nonce);
            }
            let restore_error = restore_cwd(&cwd_relative).err();
            return Err(format!(
                "Failed to replace generated output directory '{}': {}",
                self.final_dir.display(),
                error
            ) + &restore_error
                .map(|error| format!(". {error}"))
                .unwrap_or_default());
        }

        restore_cwd(&cwd_relative)?;
        if had_existing_output {
            validate_transaction_owner(&self.final_dir, &self.nonce)?;
            remove_owned_transaction_dir(&self.backup_dir, &self.nonce)?;
        }
        remove_transaction_owner(&self.final_dir, &self.nonce)?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for OutputTransaction {
    fn drop(&mut self) {
        if !self.committed {
            if self.backup_dir.exists() {
                if self.final_dir.exists() {
                    if validate_transaction_owner(&self.final_dir, &self.nonce).is_ok()
                        && validate_transaction_owner(&self.backup_dir, &self.nonce).is_ok()
                    {
                        let _ = remove_owned_transaction_dir(&self.backup_dir, &self.nonce);
                        let _ = remove_transaction_owner(&self.final_dir, &self.nonce);
                    }
                } else if validate_transaction_owner(&self.backup_dir, &self.nonce).is_ok()
                    && fs::rename(&self.backup_dir, &self.final_dir).is_ok()
                {
                    let _ = remove_transaction_owner(&self.final_dir, &self.nonce);
                }
            }
            if self.stage_dir.exists() {
                let _ = remove_owned_transaction_dir(&self.stage_dir, &self.nonce);
            }
        }
        drop(self.lock_file.take());
    }
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
        let metadata = fs::symlink_metadata(&source_path)
            .map_err(|e| format!("Failed to inspect {}: {}", source_path.display(), e))?;
        if is_link_or_reparse_point(&metadata) {
            return Err(format!(
                "Unsupported linked filesystem entry in generated output: {}",
                source_path.display()
            ));
        }
        if metadata.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else if metadata.is_file() {
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
                "Unsupported filesystem entry in generated output: {}",
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

    fn collect_test_file_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
        fn visit(root: &Path, current: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(current).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(root, &path, files);
                } else {
                    let relative = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    files.insert(relative, fs::read(path).unwrap());
                }
            }
        }

        let mut files = BTreeMap::new();
        visit(root, root, &mut files);
        files
    }

    fn synthetic_javascript_class(namespace: &str, iid: &str) -> meta::ClassMeta {
        meta::ClassMeta {
            name: "Widget".into(),
            namespace: namespace.into(),
            full_name: format!("{namespace}.Widget"),
            default_interface: Some(meta::InterfaceMeta {
                name: "IWidget".into(),
                namespace: namespace.into(),
                iid: iid.into(),
                methods: vec![meta::MethodMeta {
                    name: "GetValue".into(),
                    return_type: Some(TypeMeta::I32),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn generate_test_javascript_stage(
        output: &Path,
        native_classes: &[meta::ClassMeta],
    ) -> Result<(), String> {
        generate_test_javascript_stage_with_previous(output, native_classes, None)
    }

    fn generate_test_javascript_stage_with_previous(
        output: &Path,
        native_classes: &[meta::ClassMeta],
        previous_records: Option<&[javascript::JavaScriptTypeLayoutRecord]>,
    ) -> Result<(), String> {
        fs::create_dir_all(output)
            .map_err(|error| format!("Failed to create {}: {error}", output.display()))?;
        let previous_records = previous_records.map_or_else(
            || read_javascript_type_inventory(output).map(|inventory| inventory.records),
            |records| Ok(records.to_vec()),
        )?;
        let current_records = javascript_type_layout_records(native_classes, &[], &[])?;
        validate_javascript_type_layout_records(&previous_records, &current_records)?;
        let context = javascript::create_javascript_projection_context_with_records(
            previous_records
                .iter()
                .chain(&current_records)
                .map(|record| record.identity.clone()),
            previous_records.iter().cloned(),
            "@microsoft/dynwinrt",
        )?;
        let projected_names = context
            .output_targets()
            .map(|target| (target.identity.clone(), target.projected_name.clone()))
            .collect::<HashMap<_, _>>();
        let current_identities = current_records
            .iter()
            .map(|record| record.identity.clone())
            .collect::<HashSet<_>>();
        let retained_renames = previous_records
            .iter()
            .filter(|record| {
                projected_names
                    .get(&record.identity)
                    .is_some_and(|projected| projected != &record.projected_name)
                    && !current_identities.contains(&record.identity)
            })
            .cloned()
            .collect::<Vec<_>>();

        let mut classes = native_classes.to_vec();
        javascript::apply_javascript_projected_names(&context, &mut classes, &mut [], &mut []);
        let mut known_types = context
            .output_targets()
            .map(|target| target.projected_name.clone())
            .collect::<HashSet<_>>();
        for class in &classes {
            known_types.insert(class.name.clone());
            known_types.insert(class.full_name.clone());
        }
        let mut plan = generate_js_files(
            &context,
            output,
            &classes,
            &[],
            &[],
            &[],
            &known_types,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        )?;
        write_retained_javascript_projected_aliases(&context, output, &retained_renames)?;
        validate_generated_struct_helper_identities(&context, output)?;
        let emitted =
            emitted_javascript_type_records(&context, output, &previous_records, &current_records)?;
        write_javascript_type_inventory(output, &emitted)?;
        write_js_barrel_and_manifest(output, &mut plan)
    }

    #[test]
    fn com_barrel_deduplicates_only_identical_pod_factories() {
        let descriptor =
            "const _nativeLayout_RECT = '{\"name\":\"Windows.Win32.Foundation.RECT\"}';";
        let mut modules = BTreeMap::from([
            (
                "IFirst".into(),
                BTreeSet::from(["IFirst".into(), "createRECT".into()]),
            ),
            (
                "ISecond".into(),
                BTreeSet::from(["ISecond".into(), "createRECT".into()]),
            ),
        ]);
        let contents = BTreeMap::from([
            (
                "IFirst".into(),
                format!(
                    "{descriptor}\nexports.IFirst = IFirst;\nexports.createRECT = createRECT;\n"
                ),
            ),
            (
                "ISecond".into(),
                format!(
                    "{descriptor}\nexports.ISecond = ISecond;\nexports.createRECT = createRECT;\n"
                ),
            ),
        ]);

        deduplicate_com_barrel_exports(&mut modules, &contents).unwrap();
        assert!(modules["IFirst"].contains("createRECT"));
        assert!(!modules["ISecond"].contains("createRECT"));

        let mut conflicting_modules = modules.clone();
        conflicting_modules
            .get_mut("ISecond")
            .unwrap()
            .insert("createRECT".into());
        let mut conflicting_contents = contents;
        conflicting_contents.insert(
            "ISecond".into(),
            "const _nativeLayout_RECT = 'different';\nexports.createRECT = createRECT;\n".into(),
        );
        assert!(
            deduplicate_com_barrel_exports(&mut conflicting_modules, &conflicting_contents)
                .is_err()
        );

        let union_descriptor = "const _nativeUnionLayout_VALUE = '{\"name\":\"Contoso.VALUE\"}';";
        let mut union_modules = BTreeMap::from([
            ("IFirst".into(), BTreeSet::from(["createVALUE".into()])),
            ("ISecond".into(), BTreeSet::from(["createVALUE".into()])),
        ]);
        let union_contents = BTreeMap::from([
            (
                "IFirst".into(),
                format!("{union_descriptor}\nexports.createVALUE = createVALUE;\n"),
            ),
            (
                "ISecond".into(),
                format!("{union_descriptor}\nexports.createVALUE = createVALUE;\n"),
            ),
        ]);
        deduplicate_com_barrel_exports(&mut union_modules, &union_contents).unwrap();
        assert!(union_modules["IFirst"].contains("createVALUE"));
        assert!(!union_modules["ISecond"].contains("createVALUE"));
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

    #[cfg(windows)]
    #[test]
    fn generated_javascript_rejects_linked_namespace_directories() {
        use std::os::windows::fs::{symlink_dir, symlink_file};

        let output = test_directory("javascript-linked-namespace");
        let outside = test_directory("javascript-linked-namespace-outside");
        fs::create_dir_all(&output).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let link = output.join("contoso");
        if let Err(error) = symlink_dir(&outside, &link) {
            eprintln!("Skipping linked-directory test: {error}");
            fs::remove_dir_all(output).unwrap();
            fs::remove_dir_all(outside).unwrap();
            return;
        }

        let error = ensure_safe_generated_parent(&output, &link.join("Widget.js"))
            .expect_err("linked namespace components must be rejected");

        assert!(error.contains("linked directory"), "{error}");
        assert!(!outside.join("Widget.js").exists());
        fs::remove_dir(link).unwrap();

        let external_file = outside.join("Widget.js");
        fs::write(
            &external_file,
            "// Generated by dynwinrt-codegen — do not edit\n",
        )
        .unwrap();
        let linked_file = output.join("Widget.js");
        if symlink_file(&external_file, &linked_file).is_ok() {
            let error = write_generated_javascript_file(
                &output,
                &linked_file,
                "// Generated by dynwinrt-codegen — do not edit\nexports.Widget = Widget;\n",
            )
            .expect_err("linked destination files must be rejected");
            assert!(error.contains("linked file"), "{error}");
            assert_eq!(
                fs::read_to_string(&external_file).unwrap(),
                "// Generated by dynwinrt-codegen — do not edit\n"
            );
            fs::remove_file(linked_file).unwrap();
        }
        let external_lifetime = outside.join("lifetime.js");
        fs::write(&external_lifetime, "outside lifetime").unwrap();
        let linked_lifetime = output.join("lifetime.js");
        if symlink_file(&external_lifetime, &linked_lifetime).is_ok() {
            let error =
                write_lifetime_module(&output).expect_err("linked root artifacts must be rejected");
            assert!(error.contains("linked file"), "{error}");
            assert_eq!(
                fs::read_to_string(&external_lifetime).unwrap(),
                "outside lifetime"
            );
            fs::remove_file(linked_lifetime).unwrap();
        }
        fs::remove_dir_all(output).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn javascript_namespace_layout_emits_canonical_modules_and_qualified_root_exports() {
        let class = |namespace: &str, iid: &str| meta::ClassMeta {
            name: "Widget".into(),
            namespace: namespace.into(),
            full_name: format!("{namespace}.Widget"),
            default_interface: Some(meta::InterfaceMeta {
                name: "IWidget".into(),
                namespace: namespace.into(),
                iid: iid.into(),
                methods: vec![meta::MethodMeta {
                    name: "GetPoint".into(),
                    return_type: Some(TypeMeta::Struct {
                        namespace: "Windows.Foundation".into(),
                        name: "Point".into(),
                        fields: vec![
                            dynwinrt_codegen::types::FieldMeta {
                                name: "X".into(),
                                typ: TypeMeta::F32,
                            },
                            dynwinrt_codegen::types::FieldMeta {
                                name: "Y".into(),
                                typ: TypeMeta::F32,
                            },
                        ],
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut classes = vec![
            class("Contoso.Alpha", "11111111-1111-1111-1111-111111111111"),
            class("Fabrikam.Beta", "22222222-2222-2222-2222-222222222222"),
        ];
        let records = javascript_type_layout_records(&classes, &[], &[]).unwrap();
        let identities = records.iter().map(|record| record.identity.clone());
        let context = javascript::create_javascript_projection_context(identities).unwrap();
        javascript::apply_javascript_projected_names(&context, &mut classes, &mut [], &mut []);
        let known_types = classes
            .iter()
            .map(|class| class.name.clone())
            .collect::<HashSet<_>>();
        let output = test_directory("javascript-namespace-layout");
        fs::create_dir_all(&output).unwrap();

        let mut plan = generate_js_files(
            &context,
            &output,
            &classes,
            &[],
            &[],
            &[],
            &known_types,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();
        write_javascript_type_inventory(
            &output,
            &emitted_javascript_type_records(&context, &output, &[], &records).unwrap(),
        )
        .unwrap();
        write_js_barrel_and_manifest(&output, &mut plan).unwrap();

        assert!(output.join("contoso/alpha/Widget.js").is_file());
        assert!(output.join("fabrikam/beta/Widget.js").is_file());
        assert!(!output.join("ContosoAlphaWidget.js").exists());
        assert!(!output.join("FabrikamBetaWidget.js").exists());
        assert!(!output.join("Widget.js").exists());
        let index = fs::read_to_string(output.join("index.d.ts")).unwrap();
        assert!(index.contains("ContosoAlphaWidget"));
        assert!(index.contains("FabrikamBetaWidget"));
        assert!(index.contains("packPoint"));
        let package = fs::read_to_string(output.join("package.json")).unwrap();
        assert!(!package.contains("\"./ContosoAlphaWidget\""));
        assert!(package.contains("\"./contoso/alpha/Widget\""));

        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn all_interface_projection_supersedes_shared_interface_duplicate() {
        let interface = |method_name: &str| meta::InterfaceMeta {
            name: "IWidget".into(),
            namespace: "Contoso".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
            methods: vec![meta::MethodMeta {
                name: method_name.into(),
                return_type: Some(TypeMeta::I32),
                ..Default::default()
            }],
            ..Default::default()
        };
        let all_interface = interface("NewValue");
        let shared_interface = interface("OldValue");
        let records =
            javascript_type_layout_records(&[], std::slice::from_ref(&all_interface), &[]).unwrap();
        let context = javascript::create_javascript_projection_context(
            records.iter().map(|record| record.identity.clone()),
        )
        .unwrap();
        let output = test_directory("javascript-shared-interface-dedup");
        fs::create_dir_all(&output).unwrap();

        let plan = generate_js_files(
            &context,
            &output,
            &[],
            std::slice::from_ref(&all_interface),
            &[],
            std::slice::from_ref(&shared_interface),
            &HashSet::from(["IWidget".into()]),
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();
        let generated = plan.modules["contoso/IWidget"]
            .javascript
            .as_deref()
            .unwrap();
        assert!(generated.contains("newValue()"), "{generated}");
        assert!(!generated.contains("oldValue()"), "{generated}");

        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn legacy_javascript_output_requires_one_time_clean_regeneration() {
        let output = test_directory("javascript-legacy-layout");
        fs::create_dir_all(&output).unwrap();
        fs::write(
            output.join("Widget.js"),
            "// Generated by dynwinrt-codegen — do not edit\nexports.Widget = Widget;\n",
        )
        .unwrap();

        let error = ensure_javascript_layout_inventory(&output)
            .expect_err("legacy generated files without identities must fail closed");

        assert!(error.contains("legacy flat layout"), "{error}");
        assert!(error.contains("regenerate it once"), "{error}");
        assert!(output.join("Widget.js").is_file());
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn retained_javascript_modules_require_complete_root_export_metadata() {
        let identity = javascript::JavaScriptTypeIdentity::new(
            "Contoso",
            "Widget",
            javascript::JavaScriptTypeKind::Class,
        );
        let context = javascript::create_javascript_projection_context([identity]).unwrap();
        let output = test_directory("javascript-retained-root-metadata");
        let js = output.join("contoso/Widget.js");
        let dts = output.join("contoso/Widget.d.ts");
        fs::create_dir_all(js.parent().unwrap()).unwrap();
        fs::write(
            &js,
            "// Generated by dynwinrt-codegen — do not edit\nexports.Widget = Widget;\n",
        )
        .unwrap();
        fs::write(
            &dts,
            "// Generated by dynwinrt-codegen — do not edit\nexport declare class Widget {}\n",
        )
        .unwrap();

        let error = match load_effective_generation_plan(&context, &output, &HashSet::new()) {
            Ok(_) => panic!("retained public modules without root metadata must fail closed"),
            Err(error) => error,
        };
        assert!(error.contains("require root export metadata"), "{error}");

        fs::write(
            output.join("index.d.ts"),
            "// Generated by dynwinrt-codegen — do not edit\n\
             export { Widget, packPoint, unpackPoint } from './contoso/Widget.js';\n",
        )
        .unwrap();
        let plan = load_effective_generation_plan(&context, &output, &HashSet::new()).unwrap();
        let retained = &plan.modules["contoso/Widget"];
        assert_eq!(
            retained.public_exports,
            ["Widget", "packPoint", "unpackPoint"]
                .into_iter()
                .map(str::to_string)
                .collect()
        );

        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn generated_struct_helper_scan_rejects_cross_module_identity_collisions() {
        let identities = [
            javascript::JavaScriptTypeIdentity::new(
                "Contoso.Alpha",
                "First",
                javascript::JavaScriptTypeKind::Class,
            ),
            javascript::JavaScriptTypeIdentity::new(
                "Contoso.Beta",
                "Second",
                javascript::JavaScriptTypeKind::Class,
            ),
        ];
        let context = javascript::create_javascript_projection_context(identities).unwrap();
        let output = test_directory("javascript-cross-module-struct-helper");
        for (module, struct_identity) in [
            ("contoso/alpha/First", "Alpha.Payload"),
            ("contoso/beta/Second", "Beta.Payload"),
        ] {
            let path = output.join(format!("{module}.js"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(
                path,
                format!(
                    "// Generated by dynwinrt-codegen — do not edit\n\
                     const Payload_Type = DynWinRtType.structType('{struct_identity}', []);\n"
                ),
            )
            .unwrap();
        }

        let error = validate_generated_struct_helper_identities(&context, &output)
            .expect_err("cross-module helper identity collision must fail closed");

        assert!(error.contains("Alpha.Payload"), "{error}");
        assert!(error.contains("Beta.Payload"), "{error}");
        assert!(error.contains("packPayload"), "{error}");
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn retained_projected_type_adds_new_module_alias_without_regeneration() {
        let windows = javascript::JavaScriptTypeIdentity::new(
            "Windows.Foundation",
            "Widget",
            javascript::JavaScriptTypeKind::Class,
        );
        let contoso = javascript::JavaScriptTypeIdentity::new(
            "Contoso",
            "Widget",
            javascript::JavaScriptTypeKind::Class,
        );
        let old_context =
            javascript::create_javascript_projection_context([windows.clone(), contoso.clone()])
                .unwrap();
        let previous = old_context
            .output_targets()
            .into_iter()
            .find(|target| target.identity == windows)
            .map(|target| {
                javascript::JavaScriptTypeLayoutRecord::new(
                    target.identity.clone(),
                    target.projected_name.clone(),
                    "type",
                )
            })
            .unwrap();
        let context = javascript::create_javascript_projection_context_with_records(
            [
                windows.clone(),
                contoso,
                javascript::JavaScriptTypeIdentity::new(
                    "Microsoft.UI.Foundation",
                    "Widget",
                    javascript::JavaScriptTypeKind::Class,
                ),
            ],
            [previous.clone()],
            "@microsoft/dynwinrt",
        )
        .unwrap();
        let output = test_directory("javascript-retained-projected-alias");
        let js = output.join("windows/foundation/Widget.js");
        let dts = output.join("windows/foundation/Widget.d.ts");
        fs::create_dir_all(js.parent().unwrap()).unwrap();
        fs::write(
            &js,
            "// Generated by dynwinrt-codegen — do not edit\nexports.FoundationWidget = FoundationWidget;\n",
        )
        .unwrap();
        fs::write(
            &dts,
            "// Generated by dynwinrt-codegen — do not edit\nexport class FoundationWidget {}\n",
        )
        .unwrap();
        fs::write(
            output.join("index.d.ts"),
            "// Generated by dynwinrt-codegen — do not edit\n\
             export { FoundationWidget } from './windows/foundation/Widget.js';\n",
        )
        .unwrap();

        let plan = load_effective_generation_plan(&context, &output, &HashSet::new()).unwrap();
        write_retained_javascript_projected_aliases(&context, &output, &[previous]).unwrap();

        assert!(
            fs::read_to_string(&js)
                .unwrap()
                .contains("exports.WindowsFoundationWidgetClass = exports.FoundationWidget;")
        );
        assert!(
            fs::read_to_string(&dts)
                .unwrap()
                .contains("export { FoundationWidget as WindowsFoundationWidgetClass };")
        );
        assert_eq!(
            plan.modules["windows/foundation/Widget"].public_exports,
            ["WindowsFoundationWidgetClass"]
                .into_iter()
                .map(str::to_string)
                .collect()
        );
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn incremental_javascript_collision_matches_clean_full_generation() {
        let windows = synthetic_javascript_class(
            "Windows.Foundation",
            "11111111-1111-1111-1111-111111111111",
        );
        let contoso = synthetic_javascript_class("Contoso", "22222222-2222-2222-2222-222222222222");
        let microsoft = synthetic_javascript_class(
            "Microsoft.UI.Foundation",
            "33333333-3333-3333-3333-333333333333",
        );
        let phased = test_directory("javascript-phased-collision");
        let clean = test_directory("javascript-clean-collision");

        generate_test_javascript_stage(&phased, &[windows.clone(), contoso.clone()]).unwrap();
        let phase_one_inventory = read_javascript_type_inventory(&phased).unwrap();
        assert_eq!(phase_one_inventory.records.len(), 2);
        let phase_one_windows = phase_one_inventory
            .records
            .iter()
            .find(|record| record.identity.namespace == "Windows.Foundation")
            .unwrap();
        assert_ne!(phase_one_windows.projected_name, "Widget");
        assert!(phase_one_windows.compatibility_aliases.is_empty());
        let phase_one_projected_name = phase_one_windows.projected_name.clone();
        let phase_one_root = fs::read_to_string(phased.join("index.d.ts")).unwrap();
        assert_eq!(
            phase_one_root.matches(&phase_one_projected_name).count(),
            1,
            "{phase_one_root}"
        );
        let phase_one_package = fs::read_to_string(phased.join("package.json")).unwrap();
        assert!(phase_one_package.contains("\"./windows/foundation/Widget\""));
        assert!(phase_one_package.contains("\"./contoso/Widget\""));
        assert!(!phase_one_package.contains("\"./microsoft/ui/foundation/Widget\""));

        let phase_one_index = fs::read(phased.join("index.d.ts")).unwrap();
        fs::remove_file(phased.join("index.d.ts")).unwrap();
        let context = javascript::create_javascript_projection_context(
            phase_one_inventory
                .records
                .iter()
                .map(|record| record.identity.clone()),
        )
        .unwrap();
        let error = match load_effective_generation_plan(&context, &phased, &HashSet::new()) {
            Ok(_) => panic!("retained modules without root metadata must fail closed"),
            Err(error) => error,
        };
        assert!(error.contains("require root export metadata"), "{error}");
        fs::write(phased.join("index.d.ts"), phase_one_index).unwrap();

        let full = [windows, contoso, microsoft];
        generate_test_javascript_stage(&phased, &full).unwrap();
        generate_test_javascript_stage_with_previous(
            &clean,
            &full,
            Some(&phase_one_inventory.records),
        )
        .unwrap();

        let final_inventory = read_javascript_type_inventory(&phased).unwrap();
        let windows_record = final_inventory
            .records
            .iter()
            .find(|record| record.identity.namespace == "Windows.Foundation")
            .unwrap();
        let contoso_record = final_inventory
            .records
            .iter()
            .find(|record| record.identity.namespace == "Contoso")
            .unwrap();
        let microsoft_record = final_inventory
            .records
            .iter()
            .find(|record| record.identity.namespace == "Microsoft.UI.Foundation")
            .unwrap();
        assert_ne!(windows_record.projected_name, "Widget");
        assert_ne!(contoso_record.projected_name, "Widget");
        assert_ne!(
            windows_record.projected_name,
            phase_one_windows.projected_name
        );
        assert!(
            windows_record
                .compatibility_aliases
                .contains(&phase_one_projected_name),
            "{windows_record:?}"
        );

        let windows_js = fs::read_to_string(phased.join("windows/foundation/Widget.js")).unwrap();
        assert_eq!(
            windows_js
                .matches(&format!(
                    "exports.Widget = {};",
                    windows_record.projected_name
                ))
                .count(),
            1,
            "{windows_js}"
        );
        assert_eq!(
            windows_js
                .matches(&format!(
                    "exports.{phase_one_projected_name} = {};",
                    windows_record.projected_name
                ))
                .count(),
            1,
            "{windows_js}"
        );
        let root = fs::read_to_string(phased.join("index.d.ts")).unwrap();
        let root_exports = root
            .lines()
            .filter_map(parse_root_export_metadata)
            .collect::<Vec<_>>();
        for projected in [
            &windows_record.projected_name,
            &contoso_record.projected_name,
            &microsoft_record.projected_name,
        ] {
            assert_eq!(
                root_exports
                    .iter()
                    .flat_map(|(names, _)| names)
                    .filter(|name| *name == projected)
                    .count(),
                1,
                "{root}"
            );
        }
        assert!(!root.contains("export { Widget"), "{root}");
        assert!(
            root.contains(&format!(
                "export {{ {} }} from './microsoft/ui/foundation/Widget.js';",
                microsoft_record.projected_name
            )),
            "{root}"
        );
        assert!(
            !root_exports.iter().any(|(names, module)| {
                module == "windows/foundation/Widget" && names.contains(&phase_one_projected_name)
            }),
            "compatibility alias leaked from its retained module: {root}"
        );

        let package = fs::read_to_string(phased.join("package.json")).unwrap();
        for canonical in [
            "windows/foundation/Widget",
            "contoso/Widget",
            "microsoft/ui/foundation/Widget",
        ] {
            assert_eq!(
                package.matches(&format!("\"./{canonical}\"")).count(),
                1,
                "{package}"
            );
        }
        assert!(!package.contains(&format!("\"./{}\"", windows_record.projected_name)));
        assert!(!package.contains(&format!("\"./{}\"", contoso_record.projected_name)));
        assert!(!package.contains(&format!("\"./{}\"", microsoft_record.projected_name)));

        let phased_files = collect_test_file_tree(&phased);
        let clean_files = collect_test_file_tree(&clean);
        assert_eq!(
            phased_files.keys().collect::<Vec<_>>(),
            clean_files.keys().collect::<Vec<_>>()
        );
        for (path, phased_content) in &phased_files {
            assert_eq!(
                phased_content, &clean_files[path],
                "phased and clean output differ at {path}"
            );
        }

        generate_test_javascript_stage(&phased, &full).unwrap();
        assert_eq!(collect_test_file_tree(&phased), phased_files);

        fs::remove_dir_all(phased).unwrap();
        fs::remove_dir_all(clean).unwrap();
    }

    #[test]
    fn repeated_retained_renames_alias_the_original_implementation() {
        let contoso = javascript::JavaScriptTypeIdentity::new(
            "Contoso",
            "Widget",
            javascript::JavaScriptTypeKind::Class,
        );
        let previous =
            javascript::JavaScriptTypeLayoutRecord::new(contoso.clone(), "ContosoWidget", "type")
                .with_implementation_name("Widget");
        let context = javascript::create_javascript_projection_context_with_records(
            [
                contoso,
                javascript::JavaScriptTypeIdentity::new(
                    "Aardvark",
                    "Widget",
                    javascript::JavaScriptTypeKind::Class,
                ),
                javascript::JavaScriptTypeIdentity::new(
                    "Con.Toso",
                    "Widget",
                    javascript::JavaScriptTypeKind::Class,
                ),
            ],
            [previous.clone()],
            "@microsoft/dynwinrt",
        )
        .unwrap();
        let output = test_directory("javascript-repeated-retained-alias");
        let js = output.join("contoso/Widget.js");
        let dts = output.join("contoso/Widget.d.ts");
        fs::create_dir_all(js.parent().unwrap()).unwrap();
        fs::write(
            &js,
            "// Generated by dynwinrt-codegen — do not edit\n\
             class Widget {}\n\
             exports.Widget = Widget;\n\
             exports.ContosoWidget = Widget;\n",
        )
        .unwrap();
        fs::write(
            &dts,
            "// Generated by dynwinrt-codegen — do not edit\n\
             export declare class Widget {}\n\
             export { Widget as ContosoWidget };\n",
        )
        .unwrap();

        write_retained_javascript_projected_aliases(&context, &output, &[previous]).unwrap();

        let js = fs::read_to_string(js).unwrap();
        let dts = fs::read_to_string(dts).unwrap();
        assert!(
            js.contains("exports.ContosoWidgetClass = exports.Widget;"),
            "{js}"
        );
        assert!(
            dts.contains("export { Widget as ContosoWidgetClass };"),
            "{dts}"
        );
        assert!(
            !dts.contains("export { ContosoWidget as ContosoWidgetClass };"),
            "{dts}"
        );
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn retained_name_cycle_does_not_emit_a_self_alias() {
        let identity = javascript::JavaScriptTypeIdentity::new(
            "Contoso",
            "Widget",
            javascript::JavaScriptTypeKind::Class,
        );
        let previous =
            javascript::JavaScriptTypeLayoutRecord::new(identity.clone(), "ContosoWidget", "type")
                .with_implementation_name("Widget");
        let context = javascript::create_javascript_projection_context_with_records(
            [identity],
            [previous.clone()],
            "@microsoft/dynwinrt",
        )
        .unwrap();
        let output = test_directory("javascript-retained-name-cycle");
        let js = output.join("contoso/Widget.js");
        let dts = output.join("contoso/Widget.d.ts");
        fs::create_dir_all(js.parent().unwrap()).unwrap();
        fs::write(
            &js,
            "// Generated by dynwinrt-codegen — do not edit\nexports.Widget = Widget;\n",
        )
        .unwrap();
        fs::write(
            &dts,
            "// Generated by dynwinrt-codegen — do not edit\nexport class Widget {}\n",
        )
        .unwrap();

        write_retained_javascript_projected_aliases(&context, &output, &[previous]).unwrap();

        assert!(
            !fs::read_to_string(js)
                .unwrap()
                .contains("exports.Widget = exports.Widget")
        );
        assert!(
            !fs::read_to_string(dts)
                .unwrap()
                .contains("export { Widget as Widget }")
        );
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn com_subtree_does_not_trigger_legacy_winrt_detection() {
        let output = test_directory("javascript-ignore-com-subtree");
        fs::create_dir_all(output.join("com")).unwrap();
        fs::write(
            output.join("com").join("IWidget.js"),
            "// Generated by dynwinrt-codegen - do not edit\nexports.IWidget = IWidget;\n",
        )
        .unwrap();

        check_javascript_layout_inventory(&output)
            .expect("Classic COM modules are isolated from WinRT inventory detection");

        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn uninventoried_canonical_targets_fail_closed() {
        let output = test_directory("javascript-uninventoried-canonical");
        let canonical = output.join("contoso").join("Widget.js");
        fs::create_dir_all(canonical.parent().unwrap()).unwrap();
        fs::write(
            &canonical,
            "// Generated by dynwinrt-codegen — do not edit\nexports.Widget = Widget;\n",
        )
        .unwrap();
        let identity = javascript::JavaScriptTypeIdentity::new(
            "Contoso",
            "Widget",
            javascript::JavaScriptTypeKind::Class,
        );
        let context = javascript::create_javascript_projection_context([identity]).unwrap();

        let error = ensure_uninventoried_javascript_targets_absent(&context, &output)
            .expect_err("canonical output without inventory must fail closed");

        assert!(error.contains("has no type inventory"), "{error}");
        assert!(canonical.is_file());
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn colliding_empty_class_stubs_keep_native_canonical_aliases() {
        let mut classes = vec![
            meta::ClassMeta {
                name: "Widget".into(),
                namespace: "Contoso.Alpha".into(),
                full_name: "Contoso.Alpha.Widget".into(),
                ..Default::default()
            },
            meta::ClassMeta {
                name: "Widget".into(),
                namespace: "Fabrikam.Beta".into(),
                full_name: "Fabrikam.Beta.Widget".into(),
                ..Default::default()
            },
        ];
        let identities = javascript_type_identities(&classes, &[], &[]).unwrap();
        let context = javascript::create_javascript_projection_context(identities).unwrap();
        javascript::apply_javascript_projected_names(&context, &mut classes, &mut [], &mut []);
        let known_types = classes
            .iter()
            .map(|class| class.name.clone())
            .collect::<HashSet<_>>();
        let output = test_directory("javascript-colliding-stubs");
        fs::create_dir_all(&output).unwrap();

        generate_js_files(
            &context,
            &output,
            &classes,
            &[],
            &[],
            &[],
            &known_types,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();

        let js = fs::read_to_string(output.join("contoso/alpha/Widget.js")).unwrap();
        assert!(js.contains("exports.Widget = ContosoAlphaWidget;"), "{js}");
        let dts = fs::read_to_string(output.join("contoso/alpha/Widget.d.ts")).unwrap();
        assert!(
            dts.contains("export { ContosoAlphaWidget as Widget };"),
            "{dts}"
        );
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn javascript_inventory_uses_initial_schema_version() {
        let output = test_directory("javascript-inventory-version");
        fs::create_dir_all(&output).unwrap();
        write_javascript_type_inventory(&output, &[]).unwrap();

        let content = fs::read_to_string(output.join(JAVASCRIPT_TYPE_INVENTORY)).unwrap();
        let inventory: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(inventory["version"], 1);
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn truncated_javascript_inventory_is_rejected() {
        let output = test_directory("javascript-truncated-inventory");
        let canonical = output.join("contoso").join("Widget.js");
        fs::create_dir_all(canonical.parent().unwrap()).unwrap();
        fs::write(
            &canonical,
            "// Generated by dynwinrt-codegen — do not edit\nexports.Widget = Widget;\n",
        )
        .unwrap();
        write_javascript_type_inventory(&output, &[]).unwrap();

        let error = read_javascript_type_inventory(&output)
            .expect_err("inventory missing an existing canonical type must fail closed");

        assert!(error.contains("does not match"), "{error}");
        assert!(canonical.is_file());
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn javascript_namespace_layout_handles_colliding_delegates_without_flat_files() {
        let delegate = |namespace: &str, iid: &str| meta::InterfaceMeta {
            name: "Handler".into(),
            namespace: namespace.into(),
            iid: iid.into(),
            methods: vec![
                meta::MethodMeta {
                    name: ".ctor".into(),
                    ..Default::default()
                },
                meta::MethodMeta {
                    name: "Invoke".into(),
                    return_type: Some(TypeMeta::Object),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut interfaces = vec![
            delegate("Contoso.Alpha", "11111111-1111-1111-1111-111111111111"),
            delegate("Fabrikam.Beta", "22222222-2222-2222-2222-222222222222"),
        ];
        let identities = javascript_type_identities(&[], &interfaces, &[]).unwrap();
        let context = javascript::create_javascript_projection_context(identities).unwrap();
        javascript::apply_javascript_projected_names(&context, &mut [], &mut interfaces, &mut []);
        let delegate_names = interfaces
            .iter()
            .map(|interface| interface.name.clone())
            .collect::<HashSet<_>>();
        let (signatures, references, wraps) = project::build_delegate_signatures(
            &context,
            &interfaces,
            &delegate_names,
            &delegate_names,
        );
        let output = test_directory("javascript-delegate-layout");
        fs::create_dir_all(&output).unwrap();

        generate_js_files(
            &context,
            &output,
            &[],
            &interfaces,
            &[],
            &[],
            &delegate_names,
            &delegate_names,
            &HashSet::new(),
            &signatures,
            &references,
            &wraps,
        )
        .unwrap();

        for name in ["ContosoAlphaHandler", "FabrikamBetaHandler"] {
            assert!(!output.join(format!("{name}.js")).exists());
        }
        let contoso = fs::read_to_string(output.join("contoso/alpha/Handler.d.ts")).unwrap();
        assert!(contoso.contains("export type ContosoAlphaHandler"));
        assert!(contoso.contains("export type Handler = ContosoAlphaHandler"));

        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn colliding_interfaces_preserve_native_iid_exports() {
        let interface = |namespace: &str, iid: &str| meta::InterfaceMeta {
            name: "IWidget".into(),
            namespace: namespace.into(),
            iid: iid.into(),
            methods: vec![meta::MethodMeta {
                name: "GetValue".into(),
                return_type: Some(TypeMeta::I32),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut interfaces = vec![
            interface("Contoso.Alpha", "11111111-1111-1111-1111-111111111111"),
            interface("Fabrikam.Beta", "22222222-2222-2222-2222-222222222222"),
        ];
        let identities = javascript_type_identities(&[], &interfaces, &[]).unwrap();
        let context = javascript::create_javascript_projection_context(identities).unwrap();
        javascript::apply_javascript_projected_names(&context, &mut [], &mut interfaces, &mut []);
        let known_types = interfaces
            .iter()
            .map(|interface| interface.name.clone())
            .collect::<HashSet<_>>();
        let output = test_directory("javascript-interface-iid-aliases");
        fs::create_dir_all(&output).unwrap();

        generate_js_files(
            &context,
            &output,
            &[],
            &interfaces,
            &[],
            &[],
            &known_types,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();

        let js = fs::read_to_string(output.join("contoso/alpha/IWidget.js")).unwrap();
        assert!(js.contains("exports.IID_ContosoAlphaIWidget ="), "{js}");
        assert!(
            js.contains("exports.IID_IWidget = exports.IID_ContosoAlphaIWidget"),
            "{js}"
        );
        let dts = fs::read_to_string(output.join("contoso/alpha/IWidget.d.ts")).unwrap();
        assert!(
            dts.contains("IID_ContosoAlphaIWidget as IID_IWidget"),
            "{dts}"
        );
        assert_eq!(
            dts.matches("export { ContosoAlphaIWidget as IWidget };")
                .count(),
            1,
            "{dts}"
        );

        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn collision_renamed_iclosable_uses_projected_import() {
        let closable = meta::InterfaceMeta {
            name: "IClosable".into(),
            namespace: "Windows.Foundation".into(),
            iid: "30d5a829-7fa4-4026-83bb-d75bae4ea99e".into(),
            methods: vec![meta::MethodMeta {
                name: "Close".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut classes = vec![meta::ClassMeta {
            name: "Resource".into(),
            namespace: "Contoso".into(),
            full_name: "Contoso.Resource".into(),
            default_interface: Some(meta::InterfaceMeta {
                name: "IResource".into(),
                namespace: "Contoso".into(),
                iid: "11111111-1111-1111-1111-111111111111".into(),
                ..Default::default()
            }),
            required_interfaces: vec![closable.clone()],
            ..Default::default()
        }];
        let mut interfaces = vec![
            closable,
            meta::InterfaceMeta {
                name: "IClosable".into(),
                namespace: "Contoso".into(),
                iid: "22222222-2222-2222-2222-222222222222".into(),
                ..Default::default()
            },
        ];
        let identities = javascript_type_identities(&classes, &interfaces, &[]).unwrap();
        let context = javascript::create_javascript_projection_context(identities).unwrap();
        javascript::apply_javascript_projected_names(
            &context,
            &mut classes,
            &mut interfaces,
            &mut [],
        );
        let projected_closable = classes[0].required_interfaces[0].name.clone();
        assert_ne!(projected_closable, "IClosable");
        let known = HashSet::from([
            classes[0].name.clone(),
            projected_closable.clone(),
            interfaces[1].name.clone(),
        ]);
        let projected = project::project_class(
            &context,
            &classes[0],
            &known,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        );
        let js = render_js::render(&projected);

        assert!(
            js.contains(&format!("__get_{projected_closable}()")),
            "{js}"
        );
        assert!(!js.contains("__get_IClosable()"), "{js}");
    }

    #[test]
    fn collision_renamed_element_factory_keeps_special_projection() {
        let mut interfaces = vec![
            meta::InterfaceMeta {
                name: "IElementFactory".into(),
                namespace: "Microsoft.UI.Xaml".into(),
                iid: "11111111-1111-1111-1111-111111111111".into(),
                ..Default::default()
            },
            meta::InterfaceMeta {
                name: "IElementFactory".into(),
                namespace: "Contoso".into(),
                iid: "22222222-2222-2222-2222-222222222222".into(),
                ..Default::default()
            },
        ];
        let identities = javascript_type_identities(&[], &interfaces, &[]).unwrap();
        let context = javascript::create_javascript_projection_context(identities).unwrap();
        javascript::apply_javascript_projected_names(&context, &mut [], &mut interfaces, &mut []);
        let known = interfaces
            .iter()
            .map(|interface| interface.name.clone())
            .collect::<HashSet<_>>();
        let projected = project::project_interface(
            &context,
            &interfaces[0],
            &known,
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        );
        let dts = render_dts::render(&projected);

        assert_ne!(interfaces[0].name, "IElementFactory");
        assert!(dts.contains("static create(getElement:"), "{dts}");
    }

    #[test]
    fn closed_generics_use_complete_argument_identity() {
        let interface = |argument_namespace: &str, iid: &str| meta::InterfaceMeta {
            name: "IVector_Widget".into(),
            namespace: "Windows.Foundation.Collections".into(),
            iid: iid.into(),
            generic_piid: Some(meta::PIID_IVECTOR.into()),
            generic_args: vec![TypeMeta::RuntimeClass {
                namespace: argument_namespace.into(),
                name: "Widget".into(),
                default_interface: None,
            }],
            ..Default::default()
        };
        let first = interface("Contoso.Alpha", meta::PIID_IVECTOR);
        let second = interface("Fabrikam.Beta", meta::PIID_IVECTOR);
        let first_records =
            javascript_type_layout_records(&[], std::slice::from_ref(&first), &[]).unwrap();
        let second_records =
            javascript_type_layout_records(&[], std::slice::from_ref(&second), &[]).unwrap();
        let first_name = javascript::parameterized_interface_name(
            &first.namespace,
            &first.name,
            first.generic_piid.as_deref().unwrap(),
            &first.generic_args,
        );
        let second_name = javascript::parameterized_interface_name(
            &second.namespace,
            &second.name,
            second.generic_piid.as_deref().unwrap(),
            &second.generic_args,
        );

        assert_eq!(first_records[0].identity.name, first_name);
        assert_eq!(second_records[0].identity.name, second_name);
        assert_ne!(first_name, second_name);
        validate_javascript_type_layout_records(&first_records, &second_records)
            .expect("incremental generic identities must coexist");

        let mut interfaces = vec![first, second];
        let same_run_records = javascript_type_layout_records(&[], &interfaces, &[]).unwrap();
        validate_javascript_type_layout_records(&[], &same_run_records)
            .expect("same-run generic identities must coexist");
        let context = javascript::create_javascript_projection_context_with_records(
            same_run_records
                .iter()
                .map(|record| record.identity.clone()),
            same_run_records.iter().cloned(),
            "@microsoft/dynwinrt",
        )
        .unwrap();
        let modules = context
            .output_targets()
            .into_iter()
            .map(|target| target.canonical_module.clone())
            .collect::<BTreeSet<_>>();
        assert!(modules.contains(&format!("windows/foundation/collections/{first_name}")));
        assert!(modules.contains(&format!("windows/foundation/collections/{second_name}")));

        javascript::apply_javascript_projected_names(&context, &mut [], &mut interfaces, &mut []);
        assert_eq!(interfaces[0].name, first_name);
        assert_eq!(interfaces[1].name, second_name);
        let known_types = interfaces
            .iter()
            .map(|interface| interface.name.clone())
            .collect::<HashSet<_>>();
        let output = test_directory("javascript-complete-generic-identities");
        fs::create_dir_all(&output).unwrap();
        generate_js_files(
            &context,
            &output,
            &[],
            &interfaces,
            &[],
            &[],
            &known_types,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();
        assert!(
            output
                .join(format!("windows/foundation/collections/{first_name}.js"))
                .is_file()
        );
        assert!(
            output
                .join(format!("windows/foundation/collections/{second_name}.js"))
                .is_file()
        );
        fs::remove_dir_all(output).unwrap();
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
    fn output_transaction_commits_complete_tree() {
        let output = test_directory("transaction-commit");
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("existing.py"), "old").unwrap();

        let transaction = OutputTransaction::begin(&output).unwrap();
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
    fn output_transaction_preserves_unrelated_legacy_backup_directory() {
        let output = test_directory("transaction-unrelated-backup");
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("existing.js"), "original").unwrap();
        let parent = output.parent().unwrap();
        let leaf = output.file_name().unwrap().to_string_lossy();
        let backup = parent.join(format!(".{leaf}.dynwinrt-backup"));
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("unrelated.txt"), "keep").unwrap();

        let transaction = OutputTransaction::begin(&output).unwrap();
        fs::write(transaction.stage_dir().join("existing.js"), "updated").unwrap();
        transaction.commit().unwrap();

        assert_eq!(
            fs::read_to_string(output.join("existing.js")).unwrap(),
            "updated"
        );
        assert_eq!(
            fs::read_to_string(backup.join("unrelated.txt")).unwrap(),
            "keep"
        );
        fs::remove_dir_all(output).unwrap();
        fs::remove_dir_all(backup).unwrap();
    }

    #[test]
    fn output_transaction_refuses_orphaned_nonce_artifacts() {
        let output = test_directory("transaction-orphan");
        fs::create_dir_all(&output).unwrap();
        let parent = output.parent().unwrap();
        let leaf = output.file_name().unwrap().to_string_lossy();
        let orphan = parent.join(format!(
            ".{}.dynwinrt-backup-orphan",
            leaf.to_ascii_uppercase()
        ));
        fs::create_dir_all(&orphan).unwrap();
        fs::write(orphan.join("unrelated.txt"), "keep").unwrap();

        let error = OutputTransaction::begin(&output)
            .err()
            .expect("orphaned nonce artifacts must fail closed");

        assert!(
            error.contains("Incomplete generated output transaction"),
            "{error}"
        );
        assert_eq!(
            fs::read_to_string(orphan.join("unrelated.txt")).unwrap(),
            "keep"
        );
        fs::remove_dir_all(output).unwrap();
        fs::remove_dir_all(orphan).unwrap();
    }

    #[test]
    fn output_transaction_excludes_concurrent_generation() {
        let output = test_directory("transaction-lock");
        fs::create_dir_all(&output).unwrap();
        let first = OutputTransaction::begin(&output).unwrap();

        let error = OutputTransaction::begin(&output)
            .err()
            .expect("a second transaction must not share the output");

        assert!(
            error.contains("Another generation is already using"),
            "{error}"
        );
        drop(first);
        let next = OutputTransaction::begin(&output)
            .expect("the lock must be released when the transaction drops");
        drop(next);
        fs::remove_dir_all(output).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn output_transaction_rejects_linked_root_and_cleans_failed_stage() {
        use std::os::windows::fs::{symlink_dir, symlink_file};

        let output = test_directory("transaction-linked-root");
        let target = test_directory("transaction-linked-root-target");
        fs::create_dir_all(&target).unwrap();
        if let Err(error) = symlink_dir(&target, &output) {
            eprintln!("Skipping transaction link test: {error}");
            fs::remove_dir_all(target).unwrap();
            return;
        }
        let error = OutputTransaction::begin(&output)
            .err()
            .expect("a linked transaction root must be rejected");
        assert!(error.contains("linked directory"), "{error}");
        fs::remove_dir(&output).unwrap();

        fs::create_dir_all(&output).unwrap();
        let external = target.join("external.js");
        fs::write(&external, "outside").unwrap();
        symlink_file(&external, output.join("linked.js")).unwrap();
        let error = OutputTransaction::begin(&output)
            .err()
            .expect("a linked child must fail staging");
        assert!(error.contains("linked filesystem entry"), "{error}");
        let parent = output.parent().unwrap();
        let leaf = output.file_name().unwrap().to_string_lossy();
        assert!(
            fs::read_dir(parent).unwrap().flatten().all(|entry| {
                !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!(".{leaf}.dynwinrt-stage-"))
            }),
            "failed begin must not leak a staging directory"
        );

        fs::remove_file(output.join("linked.js")).unwrap();
        fs::remove_dir_all(output).unwrap();
        fs::remove_dir_all(target).unwrap();
    }

    #[test]
    fn python_generation_emits_shared_runtime_and_typing_support() {
        let output = test_directory("python-shared-support");
        fs::create_dir_all(&output).unwrap();

        generate_py_files(
            &output,
            &[],
            &[],
            &[],
            &[],
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            true,
        )
        .unwrap();

        assert!(output.join("_runtime.py").is_file());
        assert!(output.join("_runtime.pyi").is_file());
        assert!(output.join("_typing.pyi").is_file());
        assert!(output.join("py.typed").is_file());
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn python_generation_uses_one_canonical_struct_module() {
        let output = test_directory("python-canonical-struct");
        fs::create_dir_all(&output).unwrap();
        let point = TypeMeta::Struct {
            namespace: "Windows.Foundation".into(),
            name: "Point".into(),
            fields: vec![
                dynwinrt_codegen::types::FieldMeta {
                    name: "X".into(),
                    typ: TypeMeta::F32,
                },
                dynwinrt_codegen::types::FieldMeta {
                    name: "Y".into(),
                    typ: TypeMeta::F32,
                },
            ],
        };
        let interface = meta::InterfaceMeta {
            name: "IWidget".into(),
            namespace: "Contoso".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
            methods: vec![meta::MethodMeta {
                name: "SetPoint".into(),
                raw_name: "SetPoint".into(),
                params: vec![meta::ParamMeta {
                    name: "value".into(),
                    typ: point,
                    direction: meta::ParamDirection::In,
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let class = meta::ClassMeta {
            name: "Widget".into(),
            namespace: "Contoso".into(),
            full_name: "Contoso.Widget".into(),
            default_interface: Some(interface),
            ..Default::default()
        };
        let known_types = HashSet::from(["Point".into(), "Widget".into()]);

        {
            let _layout =
                install_python_generation_layout(std::slice::from_ref(&class), &[], &[], &[], &[])
                    .unwrap();
            generate_py_files(
                &output,
                std::slice::from_ref(&class),
                &[],
                &[],
                &[],
                &known_types,
                &HashSet::new(),
                &HashSet::new(),
                true,
            )
            .unwrap();
        }

        let class_py = fs::read_to_string(output.join("contoso__widget.py")).unwrap();
        let class_pyi = fs::read_to_string(output.join("contoso__widget.pyi")).unwrap();
        let struct_py = fs::read_to_string(output.join("windows__foundation__point.py")).unwrap();
        assert!(class_py.contains("from .windows__foundation__point import Point"));
        assert!(class_pyi.contains("from .windows__foundation__point import Point"));
        assert!(!class_py.contains("\nclass Point:"));
        assert!(!class_pyi.contains("\nclass Point:"));
        assert!(struct_py.contains("\nclass Point:"));

        write_python_package_indexes(&output, std::slice::from_ref(&class), &[], &[], true, false)
            .unwrap();
        let point_facade = fs::read_to_string(output.join("windows/foundation/point.py")).unwrap();
        assert!(point_facade.contains("Point.__module__ = __name__"));
        assert!(point_facade.contains("Point_TYPE"));
        assert!(point_facade.contains("pack_point"));
        assert!(point_facade.contains("unpack_point"));
        let point_stub = fs::read_to_string(output.join("windows/foundation/point.pyi")).unwrap();
        assert!(point_stub.contains("Point_TYPE as Point_TYPE"));
        assert!(point_stub.contains("pack_point as pack_point"));
        assert!(point_stub.contains("unpack_point as unpack_point"));
        let foundation_index =
            fs::read_to_string(output.join("windows/foundation/__init__.py")).unwrap();
        assert!(foundation_index.contains("def __getattr__(name):"));
        assert!(!foundation_index.contains("from .point import"));
        assert!(foundation_index.contains("\"Point\": (\".point\", \"Point\")"));
        assert!(!foundation_index.contains("Point_TYPE"));
        let root_index = fs::read_to_string(output.join("__init__.py")).unwrap();
        assert!(root_index.contains("\"Point\": (\".windows.foundation.point\", \"Point\")"));
        assert!(root_index.contains("\"Widget\": (\".contoso.widget\", \"Widget\")"));
        assert!(!root_index.contains("windows__foundation__point"));
        assert!(
            !fs::read_to_string(output.join("contoso/widget.py"))
                .unwrap()
                .contains("Point")
        );
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn python_generation_rejects_consumer_struct_symbol_collisions() {
        let point = |namespace: &str| TypeMeta::Struct {
            namespace: namespace.into(),
            name: "Point".into(),
            fields: vec![dynwinrt_codegen::types::FieldMeta {
                name: "Value".into(),
                typ: TypeMeta::I32,
            }],
        };
        let class = meta::ClassMeta {
            name: "Widget".into(),
            namespace: "Contoso".into(),
            full_name: "Contoso.Widget".into(),
            default_interface: Some(meta::InterfaceMeta {
                name: "IWidget".into(),
                namespace: "Contoso".into(),
                iid: "11111111-1111-1111-1111-111111111111".into(),
                methods: vec![meta::MethodMeta {
                    name: "Transform".into(),
                    raw_name: "Transform".into(),
                    params: vec![
                        meta::ParamMeta {
                            name: "source".into(),
                            typ: point("Contoso.Geometry"),
                            direction: meta::ParamDirection::In,
                        },
                        meta::ParamMeta {
                            name: "target".into(),
                            typ: point("Fabrikam.Geometry"),
                            direction: meta::ParamDirection::In,
                        },
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        let error = generate_for_types(
            "",
            &test_directory("python-struct-symbol-collision"),
            vec![class],
            Vec::new(),
            Vec::new(),
            true,
            "py",
            "@microsoft/dynwinrt",
            true,
            &DocTable::default(),
            &[],
        )
        .unwrap_err();
        assert!(error.contains("Contoso.Widget"));
        assert!(error.contains("Contoso.Geometry.Point"));
        assert!(error.contains("Fabrikam.Geometry.Point"));
        assert!(error.contains("_pack_point"));
    }

    #[test]
    fn incremental_python_generation_reuses_existing_module_layout() {
        let existing = [python::PythonTypeIdentity {
            namespace: "Windows.Foundation.Collections".into(),
            name: "IIterable_IKeyValuePair_Object_Object".into(),
        }];
        let _layout = install_python_generation_layout(&[], &[], &[], &[], &existing).unwrap();

        assert_eq!(
            python::to_snake_case_filename("IIterable_IKeyValuePair_Object_Object"),
            "windows__foundation__collections__i_iterable_i_key_value_pair_object_object"
        );
    }

    #[test]
    fn python_generation_layout_and_inventory_include_shared_interfaces() {
        let output = test_directory("python-shared-interface-layout");
        fs::create_dir_all(&output).unwrap();
        let shared = [meta::InterfaceMeta {
            namespace: "Windows.Foundation.Collections".into(),
            name: "IIterable_IKeyValuePair_Object_Object".into(),
            ..Default::default()
        }];

        {
            let _layout = install_python_generation_layout(&[], &[], &[], &shared, &[]).unwrap();
            assert_eq!(
                python::to_snake_case_filename("IIterable_IKeyValuePair_Object_Object"),
                "windows__foundation__collections__i_iterable_i_key_value_pair_object_object"
            );
        }

        write_python_type_inventory(&output, &[]).unwrap();
        record_python_supplemental_types(&output, &shared).unwrap();
        let inventory = read_python_type_inventory(&output).unwrap();
        assert!(inventory.iter().any(|typ| typ.identity
            == python::PythonTypeIdentity {
                namespace: shared[0].namespace.clone(),
                name: shared[0].name.clone(),
            }));
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn dropped_output_transaction_preserves_existing_output() {
        let output = test_directory("transaction-drop");
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("existing.py"), "old").unwrap();

        {
            let transaction = OutputTransaction::begin(&output).unwrap();
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
    fn type_facades_keep_abi_symbols_while_public_indexes_export_types_only() {
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
        let root_runtime = fs::read_to_string(output.join("__init__.py")).unwrap();
        assert!(root_runtime.contains("def __getattr__(name):"));
        assert!(
            root_runtime.contains("\"IWidget\": (\".contoso.foundation.i_widget\", \"IWidget\")")
        );
        assert!(!root_runtime.contains("contoso__foundation__i_widget"));
        let root_stub = fs::read_to_string(output.join("__init__.pyi")).unwrap();
        assert!(root_stub.contains("from .contoso.foundation.i_widget import IWidget as IWidget"));
        assert!(!root_stub.contains("IID_IWidget"));
        let namespace_stub =
            fs::read_to_string(output.join("contoso/foundation/__init__.pyi")).unwrap();
        assert!(namespace_stub.contains("from .i_widget import IWidget as IWidget"));
        assert!(!namespace_stub.contains("IID_IWidget"));
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
    fn root_merge_deduplicates_shared_struct_exports() {
        let existing = format!(
            "{GENERATED_PYTHON_HEADER}from .contoso__first import First, EventRegistrationToken, pack_event_registration_token\n"
        );
        let generated = format!(
            "{GENERATED_PYTHON_HEADER}from .contoso__second import Second, EventRegistrationToken, pack_event_registration_token, TextSegment\n"
        );

        let merged = merge_python_indexes(&existing, &generated, &HashSet::new());

        assert!(merged.contains("from .contoso__first import First, EventRegistrationToken, pack_event_registration_token"));
        assert!(merged.contains("from .contoso__second import Second, TextSegment"));
        assert_eq!(merged.matches("EventRegistrationToken").count(), 1);
        assert_eq!(merged.matches("pack_event_registration_token").count(), 1);
    }

    #[test]
    fn lazy_root_merge_migrates_eager_indexes_and_appends_exports() {
        let existing =
            format!("{GENERATED_PYTHON_HEADER}from .contoso__first import First, Shared\n");
        let generated =
            format!("{GENERATED_PYTHON_HEADER}from .contoso__second import Second, Shared\n");

        let merged = merge_python_lazy_root_indexes(&existing, &generated, &HashSet::new());

        assert!(merged.contains("from importlib import import_module as _import_module"));
        assert!(merged.contains("\"First\": (\".contoso__first\", \"First\")"));
        assert!(merged.contains("\"Second\": (\".contoso__second\", \"Second\")"));
        assert!(merged.contains("\"Shared\": (\".contoso__first\", \"Shared\")"));
        assert_eq!(merged.matches("\"Shared\": (").count(), 1);
        assert!(!merged.contains("from .contoso__first import"));
        assert!(merged.contains("def __getattr__(name):"));
        assert!(merged.contains("def __dir__():"));

        let appended = merge_python_lazy_root_indexes(
            &merged,
            &format!("{GENERATED_PYTHON_HEADER}from .contoso__third import Third\n"),
            &HashSet::from(["Second".to_string()]),
        );
        assert!(appended.contains("\"First\": (\".contoso__first\", \"First\")"));
        assert!(appended.contains("\"Third\": (\".contoso__third\", \"Third\")"));
        assert!(!appended.contains("\"Second\": ("));
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
