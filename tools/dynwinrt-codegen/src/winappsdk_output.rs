// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Optional, transaction-owned Bootstrap resources. No native input is executed.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use object::{Object, ObjectSection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const MANIFEST: &str = ".dynwinrt-winappsdk.json";
const CONFIG: &str = "_winappsdk.json";
const WHEEL_HOOK: &str = "_dynwinrt_build/wheel.py";
const DLL_NAME: &str = "Microsoft.WindowsAppRuntime.Bootstrap.dll";
const NOTICE: &str = "_native/winappsdk/NOTICE.txt";
const LICENSE: &str = "_native/winappsdk/LICENSE.txt";
const JS_MARKER: &str = " // dynwinrt: Windows App SDK configuration";
const MAX_DLL_SIZE: u64 = 64 * 1024 * 1024;
const NPM_VERSION_ENV: &str = "DYNWINRT_CODEGEN_NPM_VERSION";

const NOTICE_TEXT: &str = "\
Microsoft Windows App SDK Bootstrap redistributable

The DLLs in this directory were copied unchanged from inputs selected by this
application package's producer. They are NOT licensed under dynwinrt's MIT
license. The Microsoft Software License Terms supplied with the original
Windows App SDK NuGet package govern them.

Redistribution is permitted only in applications satisfying those terms,
including significant primary application functionality and protective
distributor/end-user terms. This bundle does not grant standalone SDK
distribution rights. The application producer must preserve and provide the
applicable SDK license and notices when redistributing their application.
If LICENSE.txt is present, it is an unmodified copy found beside the supplied
inputs or in their standard NuGet package. Otherwise obtain the terms from
the same SDK package used to restore the inputs before distribution.

The Bootstrap DLL is not the complete Windows App Runtime. Deploy the matching
Windows App Runtime separately for framework-dependent applications.
https://learn.microsoft.com/windows/apps/windows-app-sdk/deploy-unpackaged-apps
";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Configuration {
    schema_version: u32,
    major: u16,
    minor: u16,
    dlls: BTreeMap<String, String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    generator_version: String,
    language: String,
    runtime_import: String,
    configuration: Configuration,
    files: BTreeMap<String, String>,
}

pub(crate) struct Bundle {
    manifest: Manifest,
    contents: BTreeMap<String, Vec<u8>>,
}

pub(crate) struct Inputs {
    major: u16,
    minor: u16,
    dll_paths: Vec<PathBuf>,
}

fn regeneration_error(reason: impl std::fmt::Display) -> String {
    format!(
        "{reason}. Clean the dedicated output directory and regenerate the complete bindings \
         package with matching Windows App SDK inputs. Existing output has not been changed."
    )
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn producer_version(language: &str) -> Result<String, String> {
    if language != "js" {
        return Ok(env!("CARGO_PKG_VERSION").into());
    }
    // npm releases stamp their package.json after compiling the native executable.
    let version = match std::env::var(NPM_VERSION_ENV) {
        Ok(version) => version,
        Err(std::env::VarError::NotPresent) => return Ok(env!("CARGO_PKG_VERSION").into()),
        Err(error) => return Err(format!("Invalid npm codegen version: {error}")),
    };
    semver::Version::parse(&version)
        .map_err(|error| format!("Invalid {NPM_VERSION_ENV} {version:?}: {error}"))?;
    Ok(version)
}

fn dll_path(architecture: &str) -> String {
    format!("_native/winappsdk/{architecture}/{DLL_NAME}")
}

pub(crate) fn parse_inputs(
    version: Option<&str>,
    paths: &[String],
) -> Result<Option<Inputs>, String> {
    let version = match (version, paths.is_empty()) {
        (None, true) => return Ok(None),
        (Some(version), false) => version,
        _ => {
            return Err(
                "--winappsdk-version and --bootstrap-dll-path must be supplied together".into(),
            );
        }
    };
    let components = version.split('.').collect::<Vec<_>>();
    if components.len() != 2
        || components.iter().any(|part| {
            part.is_empty()
                || !part.bytes().all(|byte| byte.is_ascii_digit())
                || (part.len() > 1 && part.starts_with('0'))
                || part.parse::<u16>().is_err()
        })
    {
        return Err(
            "--winappsdk-version requires a stable product MAJOR.MINOR (each 0..65535), \
             not a NuGet servicing version or a preview/experimental channel"
                .into(),
        );
    }
    if paths
        .iter()
        .any(|path| path.trim().is_empty() || path.contains('\0'))
    {
        return Err(
            "Each --bootstrap-dll-path must be a nonempty concrete path without NUL".into(),
        );
    }
    Ok(Some(Inputs {
        major: components[0].parse().unwrap(),
        minor: components[1].parse().unwrap(),
        dll_paths: paths.iter().map(PathBuf::from).collect(),
    }))
}

fn read_input(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Cannot inspect Bootstrap input {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > limit {
        return Err(format!(
            "Bootstrap input {} must be a nonempty file no larger than {limit} bytes",
            path.display()
        ));
    }
    let bytes = fs::read(path)
        .map_err(|error| format!("Cannot read Bootstrap input {}: {error}", path.display()))?;
    if bytes.is_empty() || bytes.len() as u64 > limit {
        return Err(format!("Bootstrap input changed size: {}", path.display()));
    }
    Ok(bytes)
}

fn validate_pe(bytes: &[u8]) -> Result<&'static str, String> {
    let file =
        object::File::parse(bytes).map_err(|error| format!("Invalid Bootstrap PE: {error}"))?;
    if file.format() != object::BinaryFormat::Pe {
        return Err("Bootstrap input must be a PE DLL, not an object file or executable".into());
    }
    let characteristics = match file.flags() {
        object::FileFlags::Coff { characteristics } => characteristics,
        _ => return Err("Bootstrap input has no PE characteristics".into()),
    };
    if characteristics & object::pe::IMAGE_FILE_DLL == 0 {
        return Err("Bootstrap PE input is not marked as a DLL".into());
    }
    let architecture = match file.architecture() {
        object::Architecture::X86_64 => "x64",
        object::Architecture::Aarch64 => "arm64",
        other => return Err(format!("Unsupported Bootstrap PE architecture: {other:?}")),
    };
    // Reject ARM64EC/ARM64X as well as malformed or truncated COFF headers.
    let offset = bytes
        .get(0x3c..0x40)
        .map(|value| u32::from_le_bytes(value.try_into().unwrap()) as usize)
        .ok_or("Truncated Bootstrap DOS header")?;
    let machine_start = offset.checked_add(4).ok_or("Invalid Bootstrap PE offset")?;
    let machine_end = offset.checked_add(6).ok_or("Invalid Bootstrap PE offset")?;
    let machine = bytes
        .get(machine_start..machine_end)
        .map(|value| u16::from_le_bytes(value.try_into().unwrap()))
        .ok_or("Truncated Bootstrap PE header")?;
    let expected = if architecture == "x64" {
        0x8664
    } else {
        0xaa64
    };
    if machine != expected {
        return Err(format!("Unsupported Bootstrap PE machine 0x{machine:04x}"));
    }
    for section in file.sections() {
        section
            .data()
            .map_err(|error| format!("Invalid Bootstrap PE section bounds: {error}"))?;
    }
    let exports = file
        .exports()
        .map_err(|error| format!("Invalid Bootstrap PE export table: {error}"))?;
    if !exports
        .iter()
        .any(|export| export.name() == b"MddBootstrapInitialize2")
    {
        return Err("Bootstrap DLL does not export MddBootstrapInitialize2".into());
    }
    Ok(architecture)
}

fn source_license(paths: &[PathBuf]) -> Result<Option<Vec<u8>>, String> {
    let mut candidates = BTreeSet::new();
    for path in paths {
        let parent = path
            .parent()
            .ok_or("Bootstrap input has no parent directory")?;
        candidates.insert(parent.join("license.txt"));
        if let Some(common) = parent.parent() {
            candidates.insert(common.join("license.txt"));
        }
        // A restored NuGet payload has <package>/<version>/runtimes/win-*/native/.
        if parent.file_name().is_some_and(|name| name == "native")
            && let Some(runtimes) = parent.parent().and_then(Path::parent)
            && runtimes.file_name().is_some_and(|name| name == "runtimes")
            && let Some(package) = runtimes.parent()
        {
            candidates.insert(package.join("license.txt"));
        }
    }
    let mut license: Option<(PathBuf, Vec<u8>)> = None;
    for candidate in candidates {
        match fs::metadata(&candidate) {
            Ok(_) => {
                let bytes = read_input(&candidate, 1024 * 1024)?;
                if let Some((source, existing)) = &license
                    && existing != &bytes
                {
                    return Err(format!(
                        "Bootstrap inputs have conflicting license.txt files: {} and {}",
                        source.display(),
                        candidate.display()
                    ));
                }
                license = Some((candidate, bytes));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("Cannot inspect {}: {error}", candidate.display()));
            }
        }
    }
    Ok(license.map(|(_, bytes)| bytes))
}

fn from_inputs(inputs: Inputs, language: &str, runtime_import: &str) -> Result<Bundle, String> {
    let mut contents = BTreeMap::new();
    let mut dlls = BTreeMap::new();
    let mut sources = BTreeMap::<&str, &Path>::new();
    for path in &inputs.dll_paths {
        let bytes = read_input(path, MAX_DLL_SIZE)?;
        let architecture =
            validate_pe(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
        let relative = dll_path(architecture);
        if let Some(existing) = contents.get(&relative) {
            if existing != &bytes {
                return Err(format!(
                    "Conflicting Bootstrap DLL inputs for {architecture}: {} and {} have different contents",
                    sources[architecture].display(),
                    path.display()
                ));
            }
        } else {
            sources.insert(architecture, path);
            dlls.insert(architecture.into(), digest(&bytes));
            contents.insert(relative, bytes);
        }
    }
    // Duplicate payloads may still supply a missing or conflicting source license.
    if let Some(license) = source_license(&inputs.dll_paths)? {
        contents.insert(LICENSE.into(), license);
    } else {
        eprintln!(
            "warning: Bootstrap DLLs retain the Microsoft Windows App SDK license, not dynwinrt's \
             MIT license. No source license.txt was found beside the inputs; include the matching \
             SDK terms with your application before redistribution. See {NOTICE} in the output."
        );
    }
    let configuration = Configuration {
        schema_version: 1,
        major: inputs.major,
        minor: inputs.minor,
        dlls,
    };
    Ok(Bundle {
        manifest: Manifest {
            schema_version: 1,
            generator_version: producer_version(language)?,
            language: language.into(),
            runtime_import: runtime_import.into(),
            configuration,
            files: BTreeMap::new(),
        },
        contents,
    })
}

fn read_manifest(output: &Path) -> Result<Option<Manifest>, String> {
    let path = output.join(MANIFEST);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Cannot inspect {}: {error}", path.display())),
    };
    if super::is_link_or_reparse_point(&metadata) || !metadata.is_file() || metadata.len() > 65536 {
        return Err(regeneration_error(
            "Invalid Windows App SDK bundle manifest",
        ));
    }
    let content =
        fs::read(&path).map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    let manifest: Manifest = serde_json::from_slice(&content).map_err(|error| {
        regeneration_error(format!("Invalid Windows App SDK manifest: {error}"))
    })?;
    let config = &manifest.configuration;
    if manifest.schema_version != 1
        || config.schema_version != 1
        || manifest.generator_version != producer_version(&manifest.language)?
        || !matches!(manifest.language.as_str(), "py" | "js")
        || config.dlls.is_empty()
        || config
            .dlls
            .iter()
            .any(|(arch, hash)| !matches!(arch.as_str(), "x64" | "arm64") || !valid_hash(hash))
    {
        return Err(regeneration_error(
            "Unsupported Windows App SDK bundle manifest",
        ));
    }
    let allowed = allowed_files(&manifest.language, config);
    if manifest
        .files
        .iter()
        .any(|(path, hash)| !allowed.contains(path) || !valid_hash(hash))
        || ![CONFIG, NOTICE]
            .iter()
            .all(|path| manifest.files.contains_key(*path))
        || config
            .dlls
            .iter()
            .any(|(arch, hash)| manifest.files.get(&dll_path(arch)) != Some(hash))
        || (manifest.language == "py"
            && !["_winappsdk.py", WHEEL_HOOK, "MANIFEST.in"]
                .iter()
                .all(|path| manifest.files.contains_key(*path)))
        || (manifest.language == "js" && !manifest.files.contains_key("_winappsdk.cjs"))
    {
        return Err(regeneration_error(
            "Invalid Windows App SDK bundle file inventory",
        ));
    }
    Ok(Some(manifest))
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn allowed_files(language: &str, config: &Configuration) -> BTreeSet<String> {
    [CONFIG, NOTICE, LICENSE]
        .into_iter()
        .map(String::from)
        .chain(config.dlls.keys().map(|arch| dll_path(arch)))
        .chain(
            if language == "py" {
                vec!["_winappsdk.py", "_winappsdk.pyi", WHEEL_HOOK, "MANIFEST.in"]
            } else {
                vec!["_winappsdk.cjs"]
            }
            .into_iter()
            .map(String::from),
        )
        .collect()
}

fn inspect_owned_path(output: &Path, relative: &str) -> Result<(), String> {
    let mut current = output.to_path_buf();
    for component in Path::new(relative).components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(regeneration_error("Invalid Bootstrap resource path"));
        }
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if super::is_link_or_reparse_point(&metadata) => {
                return Err(regeneration_error(format!(
                    "Windows App SDK resource is linked: {}",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Cannot inspect {}: {error}", current.display())),
        }
    }
    Ok(())
}

pub(crate) fn prepare(
    output: &Path,
    language: &str,
    runtime_import: &str,
    inputs: Option<Inputs>,
) -> Result<Option<Bundle>, String> {
    let existing = read_manifest(output)?;
    if existing.is_none()
        && inputs.is_none()
        && [
            CONFIG,
            "_winappsdk.py",
            "_winappsdk.cjs",
            "_native/winappsdk",
        ]
        .iter()
        .any(|name| super::path_entry_exists(&output.join(name)))
    {
        return Err(regeneration_error(
            "Windows App SDK bundle artifacts have no ownership manifest",
        ));
    }
    let mut retained = BTreeMap::new();
    if let Some(manifest) = &existing {
        if manifest.language != language || manifest.runtime_import != runtime_import {
            return Err(regeneration_error(
                "Conflicting Windows App SDK language/runtime import",
            ));
        }
        for (relative, hash) in &manifest.files {
            inspect_owned_path(output, relative)?;
            let path = output.join(relative);
            let bytes = read_input(&path, MAX_DLL_SIZE)?;
            if digest(&bytes) != *hash {
                return Err(regeneration_error(format!(
                    "Windows App SDK generated resource was modified: {}",
                    path.display()
                )));
            }
            if relative == LICENSE || relative.ends_with(".dll") {
                retained.insert(relative.clone(), bytes);
            }
        }
    }
    let mut bundle = if let Some(inputs) = inputs {
        let requested = from_inputs(inputs, language, runtime_import)?;
        if let Some(existing) = &existing
            && (existing.configuration != requested.manifest.configuration
                || retained.get(LICENSE) != requested.contents.get(LICENSE))
        {
            return Err(regeneration_error(
                "Conflicting Windows App SDK version or payload",
            ));
        }
        requested
    } else if let Some(manifest) = existing {
        Bundle {
            manifest,
            contents: retained,
        }
    } else {
        return Ok(None);
    };
    // Keep every currently-owned helper in the reservation set, including optional stubs.
    let reservations = allowed_files(language, &bundle.manifest.configuration);
    let prior = read_manifest(output)?;
    for relative in reservations {
        inspect_owned_path(output, &relative)?;
        if super::path_entry_exists(&output.join(&relative))
            && !prior
                .as_ref()
                .is_some_and(|manifest| manifest.files.contains_key(&relative))
        {
            return Err(format!(
                "Refusing to overwrite non-generated Windows App SDK resource `{relative}`"
            ));
        }
    }
    bundle.manifest.files = prior.map(|manifest| manifest.files).unwrap_or_default();
    bundle
        .contents
        .insert(NOTICE.into(), NOTICE_TEXT.as_bytes().to_vec());
    Ok(Some(bundle))
}

pub(crate) fn owned_files(output: &Path) -> Result<Vec<PathBuf>, String> {
    Ok(read_manifest(output)?
        .map(|manifest| {
            manifest
                .files
                .keys()
                .map(PathBuf::from)
                .chain(std::iter::once(PathBuf::from(MANIFEST)))
                .collect()
        })
        .unwrap_or_default())
}

pub(crate) fn owns(output: &Path, relative: &Path) -> Result<bool, String> {
    Ok(owned_files(output)?.iter().any(|path| path == relative))
}

impl Bundle {
    pub(crate) fn write(mut self, output: &Path, pyi: bool) -> Result<(), String> {
        self.contents.insert(
            CONFIG.into(),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&self.manifest.configuration).map_err(
                    |error| format!("Cannot serialize Bootstrap configuration: {error}")
                )?
            )
            .into_bytes(),
        );
        if self.manifest.language == "py" {
            self.contents
                .insert("_winappsdk.py".into(), python_provider().into_bytes());
            self.contents
                .insert(WHEEL_HOOK.into(), self.python_wheel_hook().into_bytes());
            self.contents.insert(
                "MANIFEST.in".into(),
                format!("{}include {WHEEL_HOOK}\n", super::GENERATED_PYTHON_HEADER).into_bytes(),
            );
            if pyi {
                self.contents.insert(
                    "_winappsdk.pyi".into(),
                    format!(
                        "{}def get_config() -> tuple[str, dict[str, object]]: ...\n",
                        super::GENERATED_PYTHON_HEADER
                    )
                    .into_bytes(),
                );
            }
        } else {
            let import =
                serde_json::to_string(&format!("{}/winappsdk", self.manifest.runtime_import))
                    .map_err(|error| format!("Invalid runtime import: {error}"))?;
            self.contents.insert(
                "_winappsdk.cjs".into(),
                format!(
                    "// Generated by dynwinrt-codegen - do not edit\n\
                     'use strict';\n\
                     require({import}).__registerWinAppSdkConfig(__filename, require('./{CONFIG}'));\n"
                )
                .into_bytes(),
            );
        }
        for relative in allowed_files(&self.manifest.language, &self.manifest.configuration) {
            inspect_owned_path(output, &relative)?;
            let path = output.join(&relative);
            match fs::symlink_metadata(&path) {
                Ok(_) => {
                    let bytes = read_input(&path, MAX_DLL_SIZE)?;
                    if self.manifest.files.get(&relative) != Some(&digest(&bytes)) {
                        return Err(format!(
                            "Refusing to overwrite Windows App SDK destination `{relative}` \
                             created or modified by another generator"
                        ));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("Cannot inspect {}: {error}", path.display())),
            }
        }
        for relative in self.manifest.files.keys() {
            if !self.contents.contains_key(relative) {
                let path = output.join(relative);
                match fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(format!("Cannot remove {}: {error}", path.display())),
                }
            }
        }
        for (relative, bytes) in &self.contents {
            let path = output.join(relative);
            super::ensure_safe_generated_destination(output, &path)?;
            fs::write(&path, bytes)
                .map_err(|error| format!("Cannot write {}: {error}", path.display()))?;
        }
        self.manifest.files = self
            .contents
            .iter()
            .map(|(path, bytes)| (path.clone(), digest(bytes)))
            .collect();
        let path = output.join(MANIFEST);
        super::ensure_safe_generated_destination(output, &path)?;
        super::write_file(
            &path,
            &format!(
                "{}\n",
                serde_json::to_string_pretty(&self.manifest)
                    .map_err(|error| format!("Cannot serialize Bootstrap manifest: {error}"))?
            ),
        )?;
        if self.manifest.language == "js" {
            wire_javascript(output, output)?;
            decorate_javascript_package(output, &self.manifest)?;
        }
        Ok(())
    }

    fn python_wheel_hook(&self) -> String {
        let platforms = self
            .manifest
            .configuration
            .dlls
            .keys()
            .map(|arch| {
                if arch == "x64" {
                    "win_amd64"
                } else {
                    "win_arm64"
                }
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(".");
        format!(
            "{}\
             from wheel.bdist_wheel import bdist_wheel\n\n\
             class BootstrapWheel(bdist_wheel):\n\
             \x20   def finalize_options(self):\n\
             \x20       super().finalize_options()\n\
             \x20       self.root_is_pure = False\n\n\
             \x20   def get_tag(self):\n\
             \x20       return ('py3', 'none', '{platforms}')\n",
            super::GENERATED_PYTHON_HEADER
        )
    }
}

fn python_provider() -> String {
    format!(
        "{}\
         \"\"\"Generated SDK configuration; importing this module does not initialize anything.\"\"\"\n\
         import json\n\
         from pathlib import Path\n\n\
         def get_config() -> tuple[str, dict[str, object]]:\n\
         \x20   root = Path(__file__).resolve().parent\n\
         \x20   return str(root), json.loads((root / '{CONFIG}').read_text(encoding='utf-8'))\n",
        super::GENERATED_PYTHON_HEADER
    )
}

pub(crate) fn javascript_registration(relative: &Path, text: &str) -> String {
    let extension = relative
        .extension()
        .and_then(|extension| extension.to_str());
    if !matches!(extension, Some("js" | "mjs"))
        || !text.starts_with("// Generated by dynwinrt-codegen")
    {
        return text.into();
    }
    let depth = relative
        .parent()
        .map_or(0, |parent| parent.components().count());
    let import = if depth == 0 {
        "./".into()
    } else {
        "../".repeat(depth)
    };
    let hook = if extension == Some("mjs") {
        format!("import '{import}_winappsdk.cjs';{JS_MARKER}")
    } else {
        format!("require('{import}_winappsdk.cjs');{JS_MARKER}")
    };
    let mut lines = text
        .lines()
        .filter(|line| !line.ends_with(JS_MARKER))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let offset = if lines.get(1).is_some_and(|line| {
        matches!(
            line.trim(),
            "'use strict';" | "\"use strict\";" | "'use strict'"
        )
    }) {
        2
    } else {
        1
    };
    lines.insert(offset, hook);
    format!("{}\n", lines.join("\n"))
}

pub(crate) fn wire_existing_com_files(output: &Path) -> Result<(), String> {
    let com_root = output.join("com");
    let Some(manifest) = super::read_com_generation_manifest(&com_root)? else {
        return Ok(());
    };
    let files = manifest.roots.values().flatten().collect::<BTreeSet<_>>();
    for file in files {
        com_path_is_safe(file)?;
        let relative = Path::new("com").join(file);
        if relative
            .extension()
            .is_none_or(|extension| extension != "js")
        {
            continue;
        }
        let path = output.join(&relative);
        super::ensure_safe_generated_destination(output, &path)?;
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
        let registered = javascript_registration(&relative, &text);
        if registered != text {
            super::write_generated_javascript_file(output, &path, &registered)?;
        }
    }
    Ok(())
}

fn com_path_is_safe(relative: &str) -> Result<(), String> {
    dynwinrt_codegen::codegen::com::windows_relative_path_key(relative)?;
    if !super::is_safe_relative_path(Path::new(relative)) {
        return Err(format!("Invalid retained COM path `{relative}`"));
    }
    Ok(())
}

fn wire_javascript(root: &Path, current: &Path) -> Result<(), String> {
    for entry in fs::read_dir(current)
        .map_err(|error| format!("Cannot inspect {}: {error}", current.display()))?
    {
        let entry = entry.map_err(|error| format!("Cannot inspect generated module: {error}"))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Cannot inspect {}: {error}", path.display()))?;
        if super::is_link_or_reparse_point(&metadata) {
            continue;
        }
        if metadata.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with('.') && !matches!(name.as_ref(), "node_modules" | "_native") {
                wire_javascript(root, &path)?;
            }
            continue;
        }
        let extension = path.extension().and_then(|extension| extension.to_str());
        if !matches!(extension, Some("js" | "mjs")) {
            continue;
        }
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
        if !text.starts_with("// Generated by dynwinrt-codegen") {
            continue;
        }
        let registered = javascript_registration(path.strip_prefix(root).unwrap(), &text);
        if registered != text {
            super::write_file(&path, &registered)?;
        }
    }
    Ok(())
}

fn decorate_javascript_package(output: &Path, manifest: &Manifest) -> Result<(), String> {
    let path = output.join("package.json");
    let bytes =
        fs::read(&path).map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    let mut package: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Invalid generated package.json: {error}"))?;
    package["version"] = manifest.generator_version.clone().into();
    package["sideEffects"] = true.into();
    package["os"] = serde_json::json!(["win32"]);
    package["cpu"] = serde_json::json!(manifest.configuration.dlls.keys().collect::<Vec<_>>());
    package["peerDependencies"][&manifest.runtime_import] =
        manifest.generator_version.clone().into();
    // Explicitly include support and binary data; exclude dependencies/build caches by construction.
    package["files"] = serde_json::json!([
        "**/*.js",
        "**/*.mjs",
        "**/*.cjs",
        "**/*.d.ts",
        CONFIG,
        "_native/winappsdk"
    ]);
    super::write_file(
        &path,
        &format!(
            "{}\n",
            serde_json::to_string_pretty(&TypesFirst(&package))
                .map_err(|error| format!("Cannot serialize package.json: {error}"))?
        ),
    )
}

// TypeScript conditional exports must test "types" before runtime conditions.
struct TypesFirst<'a>(&'a serde_json::Value);

impl Serialize for TypesFirst<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        match self.0 {
            serde_json::Value::Object(object) => {
                let mut map = serializer.serialize_map(Some(object.len()))?;
                if let Some(types) = object.get("types") {
                    map.serialize_entry("types", &TypesFirst(types))?;
                }
                for (key, value) in object {
                    if key != "types" {
                        map.serialize_entry(key, &TypesFirst(value))?;
                    }
                }
                map.end()
            }
            serde_json::Value::Array(array) => {
                let mut sequence = serializer.serialize_seq(Some(array.len()))?;
                for value in array {
                    sequence.serialize_element(&TypesFirst(value))?;
                }
                sequence.end()
            }
            value => value.serialize(serializer),
        }
    }
}
