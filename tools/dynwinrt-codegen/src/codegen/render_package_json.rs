// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Emit `package.json` for the bindings output directory so it acts as a
//! self-contained `@winapp/bindings` package with CJS + ESM + per-type
//! deep-import subpaths.

use std::collections::BTreeSet;

pub struct PackageManifestInput<'a> {
    pub subpath_names: &'a BTreeSet<String>,
}

pub fn render_package_json(input: &PackageManifestInput<'_>) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"name\": \"@winapp/bindings\",\n");
    out.push_str("  \"type\": \"commonjs\",\n");
    out.push_str("  \"sideEffects\": false,\n");
    out.push_str("  \"main\": \"./index.js\",\n");
    out.push_str("  \"types\": \"./index.d.ts\",\n");
    out.push_str("  \"exports\": {\n");

    out.push_str("    \".\": {\n");
    out.push_str("      \"types\": \"./index.d.ts\",\n");
    out.push_str("      \"import\": \"./index.mjs\",\n");
    out.push_str("      \"require\": \"./index.js\"\n");
    out.push_str("    },\n");

    // Opt-in Proxy barrel for tools that need cjs-module-lexer-visible
    // `exports.X = ...` assignments.
    out.push_str("    \"./proxy\": {\n");
    out.push_str("      \"types\": \"./index.d.ts\",\n");
    out.push_str("      \"require\": \"./index.proxy.js\"\n");
    out.push_str("    }");

    for name in input.subpath_names {
        out.push_str(",\n");
        out.push_str(&format!("    \"./{}\": {{\n", name));
        out.push_str(&format!("      \"types\": \"./{}.d.ts\",\n", name));
        out.push_str(&format!("      \"default\": \"./{}.js\"\n", name));
        out.push_str("    }");
    }

    out.push_str("\n  }\n");
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bindings_produces_root_only() {
        let names: BTreeSet<String> = BTreeSet::new();
        let out = render_package_json(&PackageManifestInput { subpath_names: &names });
        assert!(out.contains("\"name\": \"@winapp/bindings\""), "should set package name");
        assert!(out.contains("\"type\": \"commonjs\""), "should set commonjs");
        assert!(out.contains("\"sideEffects\": false"), "should set sideEffects: false");
        assert!(out.contains("\"./index.mjs\""), "should point ESM at .mjs");
        assert!(out.contains("\"./index.js\""), "should point CJS at .js");
        assert!(out.contains("\"./index.proxy.js\""), "should expose proxy barrel");
        // No trailing comma after the last `.`/`./proxy` entry when subpaths are empty.
        assert!(!out.contains("    },\n  }"), "must not emit trailing comma");
    }

    #[test]
    fn subpath_exports_are_alphabetical() {
        let mut names: BTreeSet<String> = BTreeSet::new();
        names.insert("Uri".into());
        names.insert("AppWindow".into());
        names.insert("LanguageModel".into());
        let out = render_package_json(&PackageManifestInput { subpath_names: &names });
        let a = out.find("\"./AppWindow\"").expect("AppWindow present");
        let l = out.find("\"./LanguageModel\"").expect("LanguageModel present");
        let u = out.find("\"./Uri\"").expect("Uri present");
        assert!(a < l && l < u, "subpaths must be alphabetically ordered");
        // Every subpath must point at both its .js and .d.ts.
        for name in ["AppWindow", "LanguageModel", "Uri"] {
            assert!(
                out.contains(&format!("\"./{}.js\"", name)),
                "subpath {} must reference its .js",
                name
            );
            assert!(
                out.contains(&format!("\"./{}.d.ts\"", name)),
                "subpath {} must reference its .d.ts",
                name
            );
        }
    }
}
