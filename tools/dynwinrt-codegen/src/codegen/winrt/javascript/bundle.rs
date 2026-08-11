// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Reproducible CommonJS bundles for configured generated binding entry modules.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingBundleSpec {
    pub name: String,
    pub modules: Vec<String>,
}

pub struct GeneratedBindingBundle {
    pub js: String,
    pub dts: String,
    pub exports: BTreeSet<String>,
    pub module_count: usize,
    pub bundled_source_bytes: usize,
    pub modules: BTreeSet<String>,
}

pub fn parse_binding_bundle_spec(value: &str) -> Result<BindingBundleSpec, String> {
    let (name, modules) = value
        .split_once('=')
        .ok_or_else(|| format!("Invalid --bundle `{value}`; expected NAME=MODULE[,MODULE...]"))?;
    let name = name.trim();
    validate_binding_file_stem(name, "bundle name")?;
    if matches!(name, "index" | "index.proxy" | "index.getter" | "lifetime") {
        return Err(format!("Reserved bundle name `{name}`"));
    }

    let mut modules = modules
        .split(',')
        .map(str::trim)
        .filter(|module| !module.is_empty())
        .map(|module| module.strip_suffix(".js").unwrap_or(module).to_string())
        .collect::<Vec<_>>();
    if modules.is_empty() {
        return Err(format!(
            "Bundle `{name}` must configure at least one module"
        ));
    }
    for module in &modules {
        validate_binding_file_stem(module, "bundle module")?;
    }
    modules.sort();
    modules.dedup();
    Ok(BindingBundleSpec {
        name: name.to_string(),
        modules,
    })
}

pub fn generate_binding_bundle(
    output_dir: &Path,
    spec: &BindingBundleSpec,
) -> Result<GeneratedBindingBundle, String> {
    let modules = collect_binding_bundle_modules(output_dir, spec)?;
    generate_binding_bundle_with_modules(output_dir, spec, &modules)
}

pub fn collect_binding_bundle_modules(
    output_dir: &Path,
    spec: &BindingBundleSpec,
) -> Result<BTreeSet<String>, String> {
    validate_binding_file_stem(&spec.name, "bundle name")?;
    for module in &spec.modules {
        validate_binding_file_stem(module, "bundle module")?;
    }

    let mut modules = BTreeSet::new();
    let mut pending = VecDeque::from(spec.modules.clone());
    while let Some(module) = pending.pop_front() {
        if modules.contains(&module) {
            continue;
        }
        let path = binding_output_file_path(output_dir, &module, "js", "bundle module")?;
        let source = fs::read_to_string(&path).map_err(|error| {
            format!(
                "Bundle `{}` references missing generated module {}: {error}",
                spec.name,
                path.display()
            )
        })?;
        for dependency in collect_relative_requires(&source) {
            let dependency_path =
                binding_output_file_path(output_dir, &dependency, "js", "bundle dependency")?;
            if !dependency_path.is_file() {
                return Err(format!(
                    "Bundle `{}` dependency closure is incomplete: generated module `{module}` \
                     requires missing generated sibling `{dependency}` at {}",
                    spec.name,
                    dependency_path.display(),
                ));
            }
            if !modules.contains(&dependency) && !pending.contains(&dependency) {
                pending.push_back(dependency);
            }
        }
        modules.insert(module);
    }
    Ok(modules)
}

pub fn generate_binding_bundle_with_modules(
    output_dir: &Path,
    spec: &BindingBundleSpec,
    included_modules: &BTreeSet<String>,
) -> Result<GeneratedBindingBundle, String> {
    validate_binding_file_stem(&spec.name, "bundle name")?;
    for module in &spec.modules {
        validate_binding_file_stem(module, "bundle module")?;
        if !included_modules.contains(module) {
            return Err(format!(
                "Bundle `{}` does not own configured root `{module}`",
                spec.name
            ));
        }
    }

    let mut modules = BTreeMap::<String, String>::new();
    for module in included_modules {
        let path = binding_output_file_path(output_dir, module, "js", "bundle module")?;
        let source = fs::read_to_string(&path).map_err(|error| {
            format!(
                "Bundle `{}` references missing generated module {}: {error}",
                spec.name,
                path.display()
            )
        })?;
        modules.insert(module.clone(), source);
    }

    let mut export_owners = BTreeMap::<String, String>::new();
    for (module, source) in &modules {
        for export in collect_cjs_exports(source) {
            if export == *module {
                export_owners.insert(export, module.clone());
            }
        }
    }
    for module in &spec.modules {
        let source = modules
            .get(module)
            .ok_or_else(|| format!("Bundle `{}` did not load `{module}`", spec.name))?;
        for export in collect_cjs_exports(source) {
            export_owners
                .entry(export)
                .or_insert_with(|| module.clone());
        }
    }
    for (module, source) in &modules {
        for export in collect_cjs_exports(source) {
            export_owners
                .entry(export)
                .or_insert_with(|| module.clone());
        }
    }

    let bundled_source_bytes = modules.values().map(String::len).sum();
    let mut js = String::new();
    js.push_str("// Generated by dynwinrt-codegen — do not edit\n");
    js.push_str(&format!(
        "// Bundle `{}`: {} configured roots, {} embedded modules\n",
        spec.name,
        spec.modules.len(),
        modules.len()
    ));
    js.push_str("const __nativeRequire = require;\n");
    js.push_str("const __modules = Object.create(null);\n");
    for (module, source) in &modules {
        js.push_str(&format!(
            "__modules['./{module}.js'] = (module, exports, require) => {{\n{source}\n}};\n"
        ));
    }
    js.push_str(
        "const __cache = Object.create(null);\n\
const __normalize = (request, parent) => {\n\
    if (!request.startsWith('.')) return null;\n\
    const parts = parent.split('/');\n\
    parts.pop();\n\
    for (const part of request.split('/')) {\n\
        if (part === '' || part === '.') continue;\n\
        if (part === '..') parts.pop();\n\
        else parts.push(part);\n\
    }\n\
    return parts.join('/');\n\
};\n\
const __load = (id) => {\n\
    const cached = __cache[id];\n\
    if (cached !== undefined) return cached.exports;\n\
    const factory = __modules[id];\n\
    if (factory === undefined) return __nativeRequire(id);\n\
    const module = { exports: {} };\n\
    __cache[id] = module;\n\
    factory(module, module.exports, (request) => {\n\
        const resolved = __normalize(request, id);\n\
        return resolved !== null && __modules[resolved] !== undefined\n\
            ? __load(resolved)\n\
            : __nativeRequire(request);\n\
    });\n\
    return module.exports;\n\
};\n",
    );
    js.push_str(
        "Object.defineProperty(exports, '__dynwinrtLoadBundledModule', { value: __load });\n",
    );
    for (export, owner) in &export_owners {
        js.push_str(&format!(
            "exports.{export} = undefined;\nObject.defineProperty(exports, '{export}', {{ enumerable: true, configurable: true, get: () => __load('./{owner}.js').{export} }});\n",
        ));
    }

    let mut dts = String::from("// Generated by dynwinrt-codegen — do not edit\n");
    for module in modules.keys() {
        let names = export_owners
            .iter()
            .filter_map(|(name, owner)| (owner == module).then_some(name.as_str()))
            .collect::<Vec<_>>();
        if !names.is_empty() {
            dts.push_str(&format!(
                "export {{ {} }} from './{module}.js';\n",
                names.join(", ")
            ));
        }
    }

    Ok(GeneratedBindingBundle {
        js,
        dts,
        exports: export_owners.into_keys().collect(),
        module_count: modules.len(),
        bundled_source_bytes,
        modules: modules.into_keys().collect(),
    })
}

pub fn binding_bundle_redirect(bundle_name: &str, module_name: &str) -> String {
    format!(
        "// Generated by dynwinrt-codegen — do not edit\nmodule.exports = require('./{bundle_name}.js').__dynwinrtLoadBundledModule('./{module_name}.js');\n",
    )
}

pub fn validate_binding_file_stem(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(format!(
            "Invalid {label} `{value}`; use only ASCII letters, digits, `_`, or `-`"
        ));
    }
    Ok(())
}

pub fn binding_output_file_path(
    output_dir: &Path,
    stem: &str,
    extension: &str,
    label: &str,
) -> Result<PathBuf, String> {
    validate_binding_file_stem(stem, label)?;
    let canonical_output = fs::canonicalize(output_dir).map_err(|error| {
        format!(
            "Failed to resolve binding output directory {}: {error}",
            output_dir.display()
        )
    })?;
    let file_name = format!("{stem}.{extension}");
    let path = output_dir.join(&file_name);
    let resolved = match fs::symlink_metadata(&path) {
        Ok(_) => fs::canonicalize(&path)
            .map_err(|error| format!("Failed to resolve {}: {error}", path.display()))?,
        Err(error) if error.kind() == ErrorKind::NotFound => canonical_output.join(&file_name),
        Err(error) => {
            return Err(format!("Failed to inspect {}: {error}", path.display()));
        }
    };
    if resolved.parent() != Some(canonical_output.as_path()) {
        return Err(format!(
            "Refusing {label} `{stem}` because {} resolves outside binding output {}",
            path.display(),
            canonical_output.display()
        ));
    }
    Ok(path)
}

fn collect_relative_requires(source: &str) -> BTreeSet<String> {
    collect_require_sources(source)
        .into_iter()
        .filter_map(|request| {
            let module = request.strip_prefix("./")?.strip_suffix(".js")?;
            (!module.is_empty() && !module.contains(['/', '\\'])).then(|| module.to_string())
        })
        .collect()
}

fn collect_require_sources(source: &str) -> Vec<String> {
    let mut requests = Vec::new();
    let mut rest = source;
    while let Some(index) = rest.find("require(") {
        rest = &rest[index + "require(".len()..];
        let trimmed = rest.trim_start();
        let Some(quote) = trimmed
            .chars()
            .next()
            .filter(|quote| matches!(quote, '\'' | '"'))
        else {
            continue;
        };
        let quoted = &trimmed[quote.len_utf8()..];
        let Some(end) = quoted.find(quote) else {
            break;
        };
        requests.push(quoted[..end].to_string());
        rest = &quoted[end + quote.len_utf8()..];
    }
    requests
}

fn collect_cjs_exports(source: &str) -> BTreeSet<String> {
    source
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix("exports.")?;
            let name = rest
                .chars()
                .take_while(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '$')
                })
                .collect::<String>();
            (!name.is_empty() && rest[name.len()..].trim_start().starts_with('=')).then_some(name)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_spec_is_sorted_and_deduplicated() {
        let spec = parse_binding_bundle_spec("first-screen=B,A,B.js").unwrap();
        assert_eq!(spec.name, "first-screen");
        assert_eq!(spec.modules, ["A", "B"]);
    }

    #[test]
    fn require_scanner_keeps_only_flat_generated_siblings() {
        let source = "\
const a = require('./A.js');\n\
const runtime = require('@microsoft/dynwinrt');\n\
const nested = require('./nested/B.js');\n";
        assert_eq!(
            collect_relative_requires(source),
            BTreeSet::from(["A".to_string()])
        );
    }
}
