// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Reproducible CommonJS bundles for configured generated binding entry modules.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const RESERVED_BUNDLE_NAMES: &[&str] = &["com", "index", "lifetime", "proxy"];
const RESERVED_BUNDLE_MODULE_NAMES: &[&str] = &["index", "lifetime"];

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
    let (name, module_list) = value
        .split_once('=')
        .ok_or_else(|| format!("Invalid --bundle `{value}`; expected NAME=MODULE[,MODULE...]"))?;
    let name = name.trim();
    validate_binding_file_stem(name, "bundle name")?;
    if RESERVED_BUNDLE_NAMES.contains(&portable_binding_name_key(name).as_str()) {
        return Err(format!("Reserved bundle name `{name}`"));
    }

    let mut modules = Vec::new();
    let mut module_spellings = BTreeMap::<String, String>::new();
    for module in module_list
        .split(',')
        .map(str::trim)
        .filter(|module| !module.is_empty())
        .map(|module| module.strip_suffix(".js").unwrap_or(module))
    {
        validate_binding_file_stem(module, "bundle module")?;
        let key = portable_binding_name_key(module);
        if RESERVED_BUNDLE_MODULE_NAMES.contains(&key.as_str()) {
            return Err(format!("Reserved bundle module `{module}`"));
        }
        if let Some(existing) = module_spellings.get(&key) {
            if existing != module {
                return Err(format!(
                    "Bundle `{name}` configures module names `{existing}` and `{module}` that \
                     collide case-insensitively"
                ));
            }
            continue;
        }
        module_spellings.insert(key, module.to_string());
        modules.push(module.to_string());
    }
    if modules.is_empty() {
        return Err(format!(
            "Bundle `{name}` must configure at least one module"
        ));
    }
    modules.sort();
    Ok(BindingBundleSpec {
        name: name.to_string(),
        modules,
    })
}

pub fn portable_binding_name_key(value: &str) -> String {
    value.to_ascii_lowercase()
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

    let available_modules = collect_portable_js_module_stems(output_dir)?;
    let mut modules = BTreeSet::new();
    let mut module_spellings = BTreeMap::<String, String>::new();
    let mut pending = VecDeque::from(spec.modules.clone());
    while let Some(module) = pending.pop_front() {
        let key = portable_binding_name_key(&module);
        if let Some(existing) = available_modules.get(&key)
            && existing != &module
        {
            return Err(format!(
                "Bundle `{}` references module `{module}` with non-portable casing; \
                 the generated file is `{existing}.js`",
                spec.name
            ));
        }
        if let Some(existing) = module_spellings.get(&key) {
            if existing != &module {
                return Err(format!(
                    "Bundle `{}` dependency closure contains module names `{existing}` and \
                     `{module}` that collide case-insensitively",
                    spec.name
                ));
            }
        } else {
            module_spellings.insert(key, module.clone());
        }
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

fn collect_portable_js_module_stems(output_dir: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut stems = BTreeMap::<String, String>::new();
    let entries = fs::read_dir(output_dir).map_err(|error| {
        format!(
            "Failed to inspect binding output directory {}: {error}",
            output_dir.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "Failed to inspect an entry in binding output directory {}: {error}",
                output_dir.display()
            )
        })?;
        if !entry
            .file_type()
            .map_err(|error| format!("Failed to inspect {}: {error}", entry.path().display()))?
            .is_file()
        {
            continue;
        }
        let Some(file_name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(stem) = file_name.strip_suffix(".js") else {
            continue;
        };
        let key = portable_binding_name_key(stem);
        if let Some(existing) = stems.get(&key) {
            if existing != stem {
                return Err(format!(
                    "Generated module names `{existing}` and `{stem}` collide \
                     case-insensitively in {}",
                    output_dir.display()
                ));
            }
        } else {
            stems.insert(key, stem.to_string());
        }
    }
    Ok(stems)
}

fn select_export_owners(
    exports_by_module: &BTreeMap<String, BTreeSet<String>>,
    configured_roots: &[String],
    canonical_owners: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut owners = BTreeMap::<String, String>::new();
    for (export, module) in canonical_owners {
        if exports_by_module
            .get(module)
            .is_some_and(|exports| exports.contains(export))
        {
            owners.insert(export.clone(), module.clone());
        }
    }
    for (module, exports) in exports_by_module {
        if exports.contains(module) {
            owners
                .entry(module.clone())
                .or_insert_with(|| module.clone());
        }
    }
    for module in configured_roots {
        if let Some(exports) = exports_by_module.get(module) {
            for export in exports {
                owners
                    .entry(export.clone())
                    .or_insert_with(|| module.clone());
            }
        }
    }
    for (module, exports) in exports_by_module {
        for export in exports {
            owners
                .entry(export.clone())
                .or_insert_with(|| module.clone());
        }
    }
    owners
}

fn collect_canonical_export_owners(
    exports_by_module: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeMap<String, String> {
    let mut owners = BTreeMap::new();
    for (module, exports) in exports_by_module {
        if exports.contains(module) {
            owners.insert(module.clone(), module.clone());
        }
    }
    for (module, exports) in exports_by_module {
        for export in exports {
            owners
                .entry(export.clone())
                .or_insert_with(|| module.clone());
        }
    }
    owners
}

fn collect_output_export_owners(
    output_dir: &Path,
    extension: &str,
    collect_exports: fn(&str) -> BTreeSet<String>,
) -> Result<BTreeMap<String, String>, String> {
    let mut exports_by_module = BTreeMap::new();
    let suffix = format!(".{extension}");
    let entries = fs::read_dir(output_dir).map_err(|error| {
        format!(
            "Failed to inspect binding output directory {}: {error}",
            output_dir.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "Failed to inspect an entry in binding output directory {}: {error}",
                output_dir.display()
            )
        })?;
        if !entry
            .file_type()
            .map_err(|error| format!("Failed to inspect {}: {error}", entry.path().display()))?
            .is_file()
        {
            continue;
        }
        let Some(file_name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(module) = file_name.strip_suffix(&suffix) else {
            continue;
        };
        if matches!(module, "index" | "index.proxy" | "index.getter") {
            continue;
        }
        let source = fs::read_to_string(entry.path())
            .map_err(|error| format!("Failed to read {}: {error}", entry.path().display()))?;
        if extension == "js"
            && source.contains("Object.defineProperty(exports, '__dynwinrtLoadBundledModule'")
        {
            continue;
        }
        exports_by_module.insert(module.to_string(), collect_exports(&source));
    }
    Ok(collect_canonical_export_owners(&exports_by_module))
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

    let runtime_exports_by_module = modules
        .iter()
        .map(|(module, source)| (module.clone(), collect_cjs_exports(source)))
        .collect::<BTreeMap<_, _>>();
    let canonical_runtime_owners =
        collect_output_export_owners(output_dir, "js", collect_cjs_exports)?;
    let export_owners = select_export_owners(
        &runtime_exports_by_module,
        &spec.modules,
        &canonical_runtime_owners,
    );

    let declaration_exports_by_module = modules
        .keys()
        .map(|module| {
            let path = binding_output_file_path(output_dir, module, "d.ts", "bundle declaration")?;
            let exports = match fs::read_to_string(&path) {
                Ok(source) => collect_dts_exports(&source),
                Err(error) if error.kind() == ErrorKind::NotFound => BTreeSet::new(),
                Err(error) => {
                    return Err(format!("Failed to read {}: {error}", path.display()));
                }
            };
            Ok((module.clone(), exports))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let canonical_declaration_owners =
        collect_output_export_owners(output_dir, "d.ts", collect_dts_exports)?;
    let declaration_owners = select_export_owners(
        &declaration_exports_by_module,
        &spec.modules,
        &canonical_declaration_owners,
    );

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
    // Keep the lazy getters behind a helper. Node 22's CommonJS lexer drops a
    // named export when it sees a direct Object.defineProperty getter for the
    // same name, even if a static exports.Name assignment is also present.
    js.push_str(
        "const __defineBundleExport = (name, get) => Object.defineProperty(exports, name, { enumerable: true, configurable: true, get });\n",
    );
    for (export, owner) in &export_owners {
        js.push_str(&format!(
            "exports.{export} = undefined;\n__defineBundleExport('{export}', () => __load('./{owner}.js').{export});\n",
        ));
    }

    let mut dts = String::from("// Generated by dynwinrt-codegen — do not edit\n");
    for module in modules.keys() {
        let runtime_names = export_owners
            .iter()
            .filter_map(|(name, owner)| (owner == module).then_some(name.as_str()))
            .collect::<Vec<_>>();
        if !runtime_names.is_empty() {
            dts.push_str(&format!(
                "export {{ {} }} from './{module}.js';\n",
                runtime_names.join(", ")
            ));
        }
        let type_only_names = declaration_owners
            .iter()
            .filter_map(|(name, owner)| {
                (owner == module && !export_owners.contains_key(name)).then_some(name.as_str())
            })
            .collect::<Vec<_>>();
        if !type_only_names.is_empty() {
            dts.push_str(&format!(
                "export type {{ {} }} from './{module}.js';\n",
                type_only_names.join(", ")
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

#[derive(Debug, PartialEq, Eq)]
enum TypeScriptToken {
    Identifier(String),
    Symbol(char),
}

fn tokenize_typescript(source: &str) -> Vec<TypeScriptToken> {
    let mut tokens = Vec::new();
    let mut chars = source.char_indices().peekable();
    while let Some((_, character)) = chars.next() {
        if character.is_whitespace() {
            continue;
        }
        if character == '/' {
            match chars.peek().map(|(_, next)| *next) {
                Some('/') => {
                    chars.next();
                    for (_, next) in chars.by_ref() {
                        if next == '\n' {
                            break;
                        }
                    }
                    continue;
                }
                Some('*') => {
                    chars.next();
                    let mut previous = '\0';
                    for (_, next) in chars.by_ref() {
                        if previous == '*' && next == '/' {
                            break;
                        }
                        previous = next;
                    }
                    continue;
                }
                _ => {}
            }
        }
        if matches!(character, '\'' | '"' | '`') {
            let quote = character;
            let mut escaped = false;
            for (_, next) in chars.by_ref() {
                if escaped {
                    escaped = false;
                } else if next == '\\' {
                    escaped = true;
                } else if next == quote {
                    break;
                }
            }
            continue;
        }
        if character.is_ascii_alphabetic() || matches!(character, '_' | '$') {
            let mut identifier = String::from(character);
            while let Some((_, next)) = chars.peek() {
                if next.is_ascii_alphanumeric() || matches!(next, '_' | '$') {
                    identifier.push(*next);
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(TypeScriptToken::Identifier(identifier));
        } else {
            tokens.push(TypeScriptToken::Symbol(character));
        }
    }
    tokens
}

fn identifier_at(tokens: &[TypeScriptToken], index: usize) -> Option<&str> {
    match tokens.get(index)? {
        TypeScriptToken::Identifier(identifier) => Some(identifier),
        TypeScriptToken::Symbol(_) => None,
    }
}

fn collect_export_list(
    tokens: &[TypeScriptToken],
    mut index: usize,
    exports: &mut BTreeSet<String>,
) -> usize {
    while index < tokens.len() {
        match tokens.get(index) {
            Some(TypeScriptToken::Symbol('}')) => return index + 1,
            Some(TypeScriptToken::Identifier(identifier)) => {
                let mut exported = identifier.as_str();
                if identifier == "type" {
                    index += 1;
                    let Some(name) = identifier_at(tokens, index) else {
                        continue;
                    };
                    exported = name;
                }
                if identifier_at(tokens, index + 1) == Some("as") {
                    if let Some(alias) = identifier_at(tokens, index + 2) {
                        exported = alias;
                        index += 2;
                    }
                }
                exports.insert(exported.to_string());
            }
            _ => {}
        }
        index += 1;
    }
    index
}

fn collect_dts_exports(source: &str) -> BTreeSet<String> {
    let tokens = tokenize_typescript(source);
    let mut exports = BTreeSet::new();
    let mut index = 0;
    let mut brace_depth = 0usize;
    while index < tokens.len() {
        match tokens.get(index) {
            Some(TypeScriptToken::Symbol('{')) => {
                brace_depth += 1;
                index += 1;
            }
            Some(TypeScriptToken::Symbol('}')) => {
                brace_depth = brace_depth.saturating_sub(1);
                index += 1;
            }
            Some(TypeScriptToken::Identifier(keyword))
                if brace_depth == 0 && keyword == "export" =>
            {
                index += 1;
                while matches!(identifier_at(&tokens, index), Some("declare" | "abstract")) {
                    index += 1;
                }
                if identifier_at(&tokens, index) == Some("default") {
                    index += 1;
                    while matches!(identifier_at(&tokens, index), Some("declare" | "abstract")) {
                        index += 1;
                    }
                }
                if matches!(tokens.get(index), Some(TypeScriptToken::Symbol('{'))) {
                    index = collect_export_list(&tokens, index + 1, &mut exports);
                    continue;
                }
                let Some(kind) = identifier_at(&tokens, index) else {
                    continue;
                };
                index += 1;
                if kind == "type" && matches!(tokens.get(index), Some(TypeScriptToken::Symbol('{')))
                {
                    index = collect_export_list(&tokens, index + 1, &mut exports);
                    continue;
                }
                if matches!(
                    kind,
                    "class"
                        | "const"
                        | "enum"
                        | "function"
                        | "interface"
                        | "let"
                        | "module"
                        | "namespace"
                        | "type"
                        | "var"
                ) && let Some(name) = identifier_at(&tokens, index)
                {
                    exports.insert(name.to_string());
                }
            }
            _ => {
                index += 1;
            }
        }
    }
    exports
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
    fn bundle_spec_rejects_portable_name_collisions() {
        let error = parse_binding_bundle_spec("first=A,a").unwrap_err();
        assert!(error.contains("collide case-insensitively"));
        for name in ["INDEX", "Lifetime", "PrOxY", "COM"] {
            assert!(
                parse_binding_bundle_spec(&format!("{name}=A"))
                    .unwrap_err()
                    .contains("Reserved bundle name")
            );
        }
        for module in ["INDEX", "Lifetime"] {
            assert!(
                parse_binding_bundle_spec(&format!("first={module}"))
                    .unwrap_err()
                    .contains("Reserved bundle module")
            );
        }
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

    #[test]
    fn declaration_export_scanner_keeps_value_and_type_exports() {
        let source = "\
// export interface Ignored {}\n\
export interface Point { x: number; }\n\
export type Rect = { width: number };\n\
export declare class Widget {}\n\
export declare function createWidget(): Widget;\n\
export { External, type ExternalShape as Shape } from './External.js';\n";
        assert_eq!(
            collect_dts_exports(source),
            BTreeSet::from([
                "External".to_string(),
                "Point".to_string(),
                "Rect".to_string(),
                "Shape".to_string(),
                "Widget".to_string(),
                "createWidget".to_string(),
            ])
        );
    }
}
