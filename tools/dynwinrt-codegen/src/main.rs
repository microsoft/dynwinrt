// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use clap::{Parser, Subcommand};

use dynwinrt_codegen::codegen::python;
use dynwinrt_codegen::codegen::typescript;
use dynwinrt_codegen::codegen::{project, render_js, render_dts};
use dynwinrt_codegen::meta;
use dynwinrt_codegen::types::TypeMeta;
use dynwinrt_codegen::xml_doc::DocTable;

#[derive(Parser)]
#[command(name = "dynwinrt-codegen")]
#[command(about = "Generate typed language bindings from WinRT metadata (.winmd) files")]
#[command(long_about = "dynwinrt-codegen reads .winmd metadata and generates typed bindings\n\
    that use @microsoft/dynwinrt at runtime to call Windows Runtime APIs dynamically.\n\n\
    It auto-detects Windows SDK metadata and discovers sibling .winmd files\n\
    in the same directory, so you typically only need to point at one file.")]
#[command(after_help = "\x1b[1mExamples:\x1b[0m\n\
    # Generate all namespaces from a WinAppSDK metadata folder\n\
    dynwinrt-codegen generate --folder C:\\Users\\you\\.winapp\\packages\\Microsoft.WindowsAppSDK.AI.1.8.39\\metadata\n\n\
    # Generate a single namespace (siblings auto-discovered)\n\
    dynwinrt-codegen generate --winmd path\\to\\Microsoft.Windows.AI.Imaging.winmd --namespace Microsoft.Windows.AI.Imaging\n\n\
    # Generate a single class\n\
    dynwinrt-codegen generate --namespace Windows.Foundation --class Uri\n\n\
    # Custom output directory\n\
    dynwinrt-codegen generate --folder path\\to\\metadata --output ./src/generated")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate bindings from .winmd files
    #[command(long_about = "Parse .winmd metadata and generate typed binding files.\n\n\
        By default (`--lang js`) the tool emits plain ESM JavaScript (`.js`) plus\n\
        matching ambient TypeScript declarations (`.d.ts`) — no TypeScript compiler\n\
        or SWC step is involved.\n\n\
        The tool automatically:\n\
        - Detects Windows.winmd from the Windows SDK install path\n\
        - Discovers sibling .winmd files in the same directory as --winmd\n\
        - Resolves transitive type dependencies across namespaces\n\
        - Filters out Windows.* system namespaces when --namespace is omitted")]
    Generate {
        /// Path(s) to .winmd metadata files, separated by ';'.
        /// Sibling .winmd files in the same directory are auto-discovered.
        /// If omitted, auto-detects Windows.winmd from Windows SDK.
        #[arg(long, value_name = "PATH")]
        winmd: Option<String>,

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
        /// E.g. --class Uri or --class StorageFile,StorageFolder
        #[arg(long, name = "class", value_name = "NAME")]
        class_name: Option<String>,

        /// Additional .winmd files for type resolution only (no code generated).
        /// Paths separated by ';'. Sibling .winmd files are NOT auto-discovered.
        #[arg(long = "ref", value_name = "PATH")]
        ref_winmd: Option<String>,

        /// Target language. `js` emits .js + .d.ts (recommended for Node consumers).
        /// `py` emits .py (and optionally .pyi via --pyi).
        #[arg(long, default_value = "js", value_parser = ["js", "py"])]
        lang: String,

        /// Output directory for generated files
        #[arg(long, default_value = "./generated", value_name = "DIR")]
        output: String,

        /// Validate metadata and resolve dependencies without writing files
        #[arg(long)]
        dry_run: bool,

        /// Also emit .pyi type stub files and py.typed marker (requires --lang py)
        #[arg(long)]
        pyi: bool,
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
        Commands::Generate {
            winmd,
            folder,
            namespace,
            class_name,
            ref_winmd,
            lang,
            output,
            dry_run,
            pyi,
        } => {
            if pyi && lang != "py" {
                return Err("--pyi requires --lang py".into());
            }
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
                        if path.extension().map_or(false, |ext| ext.eq_ignore_ascii_case("winmd")) {
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

            // Auto-detect Windows SDK if not already included
            let has_windows_winmd = winmd_parts.iter().any(|p| p.contains("Windows.winmd"));
            if !has_windows_winmd {
                if let Some(sdk_winmd) = find_windows_sdk_winmd() {
                    eprintln!("Auto-detected Windows SDK: {}", sdk_winmd);
                    winmd_parts.push(sdk_winmd);
                } else if folder.is_none() && winmd.is_none() {
                    return Err("Could not auto-detect Windows.winmd. Please provide --winmd or --folder.".into());
                }
            }

            // Collect ref winmd namespaces (for exclusion) and append to winmd_parts
            let ref_namespaces: HashSet<String> = if let Some(ref r) = ref_winmd {
                let ref_paths: Vec<&str> = r.split(';').filter(|s| !s.is_empty()).collect();
                let ref_joined = ref_paths.join(";");
                let ref_ns = meta::list_namespaces(&ref_joined);
                // Add ref paths to winmd_parts (loaded for type resolution)
                winmd_parts.extend(ref_paths.iter().map(|s| s.to_string()));
                ref_ns.into_iter().collect()
            } else {
                HashSet::new()
            };

            let winmd_joined = winmd_parts.join(";");

            // Auto-discover sibling .winmd files in the same directories
            let winmd = meta::expand_winmd_paths(&winmd_joined);

            // Build XML doc table from sibling .xml files of each winmd.
            let expanded_parts: Vec<String> = winmd.split(';').filter(|s| !s.is_empty()).map(String::from).collect();
            let mut doc_table = DocTable::load_from_winmd_paths(&expanded_parts);
            // Load built-in docs as fallback (sibling .xml takes priority).
            doc_table.load_builtin_docs();

            let output_dir = Path::new(&output);
            if !dry_run {
                fs::create_dir_all(output_dir)
                    .map_err(|e| format!("Failed to create output directory '{}': {}", output, e))?;
            }

            if let Some(ref cls_arg) = class_name {
                // Class mode: supports comma-separated list (e.g. "StorageFile,StorageFolder")
                let ns = namespace
                    .as_deref()
                    .ok_or("--namespace is required when --class is specified")?;
                let class_names: Vec<&str> = cls_arg.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
                let mut classes = Vec::new();
                for cls in &class_names {
                    match meta::parse_class(&winmd, ns, cls) {
                        Some(mut c) => {
                            doc_table.apply_to_class(&mut c);
                            classes.push(c);
                        }
                        None => return Err(format!("Class {}.{} not found in {}", ns, cls, winmd)),
                    }
                }
                generate_for_types(&winmd, output_dir, classes.clone(), Vec::new(), Vec::new(), dry_run, &lang, pyi, &doc_table)?;

                // Write (or append to) the index file for the output directory
                if !dry_run {
                    type AppendFn = fn(&str, &[meta::ClassMeta], &[meta::InterfaceMeta], &[TypeMeta]) -> String;
                    type GenerateFn = fn(&[meta::ClassMeta], &[meta::InterfaceMeta], &[TypeMeta]) -> String;
                    let (index_name, append_fn, generate_fn): (&str, AppendFn, GenerateFn) = if lang == "py" {
                        ("__init__.py", python::append_to_index, python::generate_index)
                    } else {
                        // For JS lang, we use an in-memory `.ts` index then split into .js + .d.ts.
                        // We pick a sentinel filename `index.js` to detect presence; `.d.ts` is written alongside.
                        ("index.js", typescript::append_to_index, typescript::generate_index)
                    };
                    let index_path = output_dir.join(index_name);
                    let deps = meta::resolve_dependencies(&winmd, &classes, &[], &[]);
                    let mut all_classes = [classes.as_slice(), deps.classes.as_slice()].concat();
                    let mut all_interfaces: Vec<_> = deps.interfaces.clone();
                    let mut all_enums: Vec<_> = deps.enums.clone();
                    for c in all_classes.iter_mut() { doc_table.apply_to_class(c); }
                    for i in all_interfaces.iter_mut() { doc_table.apply_to_interface(i); }
                    for e in all_enums.iter_mut() { doc_table.apply_to_enum(e); }
                    if lang == "py" {
                        if index_path.exists() {
                            let existing = fs::read_to_string(&index_path)
                                .map_err(|e| format!("Failed to read {}: {}", index_path.display(), e))?;
                            let updated = append_fn(&existing, &all_classes, &all_interfaces, &all_enums);
                            fs::write(&index_path, &updated)
                                .map_err(|e| format!("Failed to write {}: {}", index_path.display(), e))?;
                            println!("Updated {}", index_path.display());
                        } else {
                            let new_index = generate_fn(&all_classes, &all_interfaces, &all_enums);
                            fs::write(&index_path, &new_index)
                                .map_err(|e| format!("Failed to write {}: {}", index_path.display(), e))?;
                            println!("Generated {}", index_path.display());
                        }
                    } else {
                        // JS: index.js + index.d.ts are pure re-exports and identical, so we
                        // round-trip incremental appends by reading back index.js itself
                        // (no hidden cache file in the output directory).
                        let js_path = output_dir.join("index.js");
                        let dts_path = output_dir.join("index.d.ts");
                        let index_content = if js_path.exists() {
                            let existing = fs::read_to_string(&js_path)
                                .map_err(|e| format!("Failed to read {}: {}", js_path.display(), e))?;
                            append_fn(&existing, &all_classes, &all_interfaces, &all_enums)
                        } else {
                            generate_fn(&all_classes, &all_interfaces, &all_enums)
                        };
                        fs::write(&js_path, &index_content)
                            .map_err(|e| format!("Failed to write {}: {}", js_path.display(), e))?;
                        fs::write(&dts_path, &index_content)
                            .map_err(|e| format!("Failed to write {}: {}", dts_path.display(), e))?;
                        // Clean up any stale `.index.ts` cache from older codegen versions
                        let stale = output_dir.join(".index.ts");
                        if stale.exists() { let _ = fs::remove_file(&stale); }
                        println!("Generated {}", js_path.display());
                    }
                    if pyi {
                        let stub_code = dynwinrt_codegen::codegen::python_stub::generate_index_stub(
                            &all_classes, &all_interfaces, &all_enums);
                        let stub_path = output_dir.join("__init__.pyi");
                        fs::write(&stub_path, &stub_code)
                            .map_err(|e| format!("Failed to write {}: {}", stub_path.display(), e))?;
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
                            .filter(|ns| !ns.starts_with("Windows.") && !ref_namespaces.contains(ns))
                            .collect();
                        if filtered.is_empty() {
                            return Err("No non-Windows namespaces found. Use --namespace to specify one.".into());
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
                    let mut classes = meta::parse_namespace(&winmd, ns);
                    let mut interfaces = meta::parse_interfaces(&winmd, ns);
                    let mut enums = meta::parse_enums(&winmd, ns);
                    for c in classes.iter_mut() { doc_table.apply_to_class(c); }
                    for i in interfaces.iter_mut() { doc_table.apply_to_interface(i); }
                    for e in enums.iter_mut() { doc_table.apply_to_enum(e); }

                    let (nc, ni, ne) = generate_for_types(
                        &winmd, output_dir, classes, interfaces, enums, dry_run, &lang, pyi, &doc_table,
                    )?;
                    total_classes += nc;
                    total_interfaces += ni;
                    total_enums += ne;
                }

                // Generate index file combining everything
                if !dry_run && namespaces.len() >= 1 && (total_classes + total_interfaces + total_enums) > 1 {
                    let mut all_classes = Vec::new();
                    let mut all_interfaces = Vec::new();
                    let mut all_enums = Vec::new();
                    for ns in &namespaces {
                        all_classes.extend(meta::parse_namespace(&winmd, ns));
                        all_interfaces.extend(meta::parse_interfaces(&winmd, ns));
                        all_enums.extend(meta::parse_enums(&winmd, ns));
                    }
                    let deps = meta::resolve_dependencies(&winmd, &all_classes, &all_interfaces, &all_enums);
                    all_classes.extend(deps.classes);
                    all_interfaces.extend(deps.interfaces);
                    all_enums.extend(deps.enums);
                    for c in all_classes.iter_mut() { doc_table.apply_to_class(c); }
                    for i in all_interfaces.iter_mut() { doc_table.apply_to_interface(i); }
                    for e in all_enums.iter_mut() { doc_table.apply_to_enum(e); }

                    if lang == "py" {
                        let index_code = python::generate_index(&all_classes, &all_interfaces, &all_enums);
                        let index_path = output_dir.join("__init__.py");
                        fs::write(&index_path, &index_code)
                            .map_err(|e| format!("Failed to write {}: {}", index_path.display(), e))?;
                        println!("Generated {}", index_path.display());
                        if pyi {
                            let stub_code = dynwinrt_codegen::codegen::python_stub::generate_index_stub(
                                &all_classes, &all_interfaces, &all_enums);
                            let stub_path = output_dir.join("__init__.pyi");
                            fs::write(&stub_path, &stub_code)
                                .map_err(|e| format!("Failed to write {}: {}", stub_path.display(), e))?;
                            println!("Generated {}", stub_path.display());
                            let marker = output_dir.join("py.typed");
                            fs::write(&marker, "")
                                .map_err(|e| format!("Failed to write {}: {}", marker.display(), e))?;
                        }
                    } else {
                        let index_code = typescript::generate_index(&all_classes, &all_interfaces, &all_enums);
                        // Index files are pure re-exports — identical in JS and DTS
                        let js_path = output_dir.join("index.js");
                        let dts_path = output_dir.join("index.d.ts");
                        fs::write(&js_path, &index_code)
                            .map_err(|e| format!("Failed to write {}: {}", js_path.display(), e))?;
                        fs::write(&dts_path, &index_code)
                            .map_err(|e| format!("Failed to write {}: {}", dts_path.display(), e))?;
                        // Clean up any stale `.index.ts` cache from older codegen versions
                        let stale = output_dir.join(".index.ts");
                        if stale.exists() { let _ = fs::remove_file(&stale); }
                        println!("Generated {}", js_path.display());
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
                        total_classes, total_interfaces, total_enums, output_dir.display()
                    );
                }
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

    // Newly-merged dependency types haven't been doc-annotated yet. Apply doc table
    // uniformly so dependency classes/interfaces/enums carry the same XML docs as
    // the primary types.
    for c in all_classes.iter_mut() { doc_table.apply_to_class(c); }
    for i in all_interfaces.iter_mut() { doc_table.apply_to_interface(i); }
    for e in all_enums.iter_mut() { doc_table.apply_to_enum(e); }

    let mut known_types: HashSet<String> = HashSet::new();
    for c in &all_classes { known_types.insert(c.name.clone()); }
    for i in &all_interfaces { known_types.insert(i.name.clone()); }
    for e in &all_enums {
        if let TypeMeta::Enum { name, .. } = e { known_types.insert(name.clone()); }
    }

    let delegate_type_names: HashSet<String> = all_interfaces.iter()
        .filter(|i| i.methods.iter().any(|m| m.name == ".ctor") && i.methods.iter().any(|m| m.name == "Invoke"))
        .map(|i| i.name.clone())
        .collect();

    let mut req_iface_count: HashMap<String, (&meta::InterfaceMeta, usize)> = HashMap::new();
    for class in &all_classes {
        for ri in &class.required_interfaces {
            if ri.iid.is_empty() { continue; }
            req_iface_count.entry(ri.iid.clone())
                .and_modify(|(_, c)| *c += 1)
                .or_insert((ri, 1));
        }
    }
    let shared_iids: HashSet<String> = req_iface_count.iter()
        .filter(|(_, (_, count))| *count >= 2)
        .map(|(iid, _)| iid.clone())
        .collect();

    let shared_interfaces: Vec<meta::InterfaceMeta> = req_iface_count.iter()
        .filter(|(_, (_, count))| *count >= 2)
        .map(|(_, (iface, _))| (*iface).clone())
        .collect();
    for iface in &shared_interfaces {
        known_types.insert(iface.name.clone());
    }

    let (delegate_signatures, delegate_sig_refs, delegate_param_wraps) = project::build_delegate_signatures(&all_interfaces, &delegate_type_names, &known_types);

    if !dry_run {
        if lang == "py" {
            generate_py_files(output_dir, &all_classes, &all_interfaces, &all_enums, &shared_interfaces, &known_types, &delegate_type_names, &shared_iids, pyi)?;
        } else {
            generate_js_files(output_dir, &all_classes, &all_interfaces, &all_enums, &shared_interfaces, &known_types, &delegate_type_names, &shared_iids, &delegate_signatures, &delegate_sig_refs, &delegate_param_wraps)?;
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

    for iface in shared_interfaces {
        let projected = project::project_interface(iface, known_types, delegate_type_names, delegate_sigs, delegate_sig_refs, delegate_param_wraps);
        let js = render_js::render(&projected);
        let dts = render_dts::render(&projected);
        emit(&iface.name, &js, &dts)?;
    }
    for iface in all_interfaces {
        let projected = project::project_interface(iface, known_types, delegate_type_names, delegate_sigs, delegate_sig_refs, delegate_param_wraps);
        let js = render_js::render(&projected);
        let dts = render_dts::render(&projected);
        emit(&iface.name, &js, &dts)?;
    }
    for en in all_enums {
        if let TypeMeta::Enum { name, .. } = en {
            if let Some(projected) = project::project_enum(en) {
                let js = render_js::render(&projected);
                let dts = render_dts::render(&projected);
                emit(name, &js, &dts)?;
            }
        }
    }
    for class in all_classes {
        let projected = project::project_class(class, known_types, delegate_type_names, shared_iids, delegate_sigs, delegate_sig_refs, delegate_param_wraps);
        let js = render_js::render(&projected);
        let dts = render_dts::render(&projected);
        emit(&class.name, &js, &dts)?;
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
    use dynwinrt_codegen::codegen::common::to_snake_case_filename;
    use dynwinrt_codegen::codegen::python_stub;

    for iface in shared_interfaces {
        let code = python::generate_interface(iface, known_types, delegate_type_names);
        let filepath = output_dir.join(format!("{}.py", to_snake_case_filename(&iface.name)));
        write_file(&filepath, &code)?;
        println!("Generated shared {}", filepath.display());
        if pyi {
            let stub = python_stub::generate_interface_stub(iface, known_types, delegate_type_names);
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
            let stub = python_stub::generate_interface_stub(iface, known_types, delegate_type_names);
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
            let stub = python_stub::generate_class_stub(class, known_types, delegate_type_names, shared_iids);
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
    fs::write(path, content)
        .map_err(|e| format!("Failed to write {}: {}", path.display(), e))
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
