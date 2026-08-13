// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Emit `package.json` for a generated bindings package containing WinRT,
//! Classic COM, or both domains.

use std::collections::BTreeSet;

pub struct PackageManifestInput<'a> {
    pub subpath_names: &'a BTreeSet<String>,
}

pub struct BindingsPackageManifestInput<'a> {
    pub has_winrt_root: bool,
    pub winrt_subpath_names: &'a BTreeSet<String>,
    pub com_subpath_names: &'a BTreeSet<String>,
}

pub struct PythonPackageManifestInput<'a> {
    pub distribution_name: &'a str,
    pub import_name: &'a str,
    pub package_version: &'a str,
    pub runtime_version: &'a str,
    pub namespace_packages: &'a [String],
}

/// Preserve the existing WinRT-only renderer API and byte-for-byte output.
pub fn render_package_json(input: &PackageManifestInput<'_>) -> String {
    let com_subpath_names = BTreeSet::new();
    render_bindings_package_json(&BindingsPackageManifestInput {
        has_winrt_root: true,
        winrt_subpath_names: input.subpath_names,
        com_subpath_names: &com_subpath_names,
    })
}

pub fn render_bindings_package_json(input: &BindingsPackageManifestInput<'_>) -> String {
    if input.has_winrt_root {
        render_winrt_package(input)
    } else {
        render_com_only_package(input.com_subpath_names)
    }
}

pub fn render_python_pyproject(input: &PythonPackageManifestInput<'_>) -> String {
    let mut packages = vec![format!("\"{}\"", input.import_name)];
    packages.extend(
        input
            .namespace_packages
            .iter()
            .map(|namespace| format!("\"{}.{}\"", input.import_name, namespace)),
    );
    format!(
        "[build-system]\n\
         requires = [\"setuptools>=68\"]\n\
         build-backend = \"setuptools.build_meta\"\n\
         \n\
         [project]\n\
         name = \"{distribution_name}\"\n\
         version = \"{package_version}\"\n\
         requires-python = \">=3.11,<3.15\"\n\
         dependencies = [\"dynwinrt=={runtime_version}\"]\n\
         \n\
         [tool.setuptools]\n\
         packages = [{packages}]\n\
         package-dir = {{ \"{import_name}\" = \".\" }}\n\
         \n\
         [tool.setuptools.package-data]\n\
         \"{import_name}\" = [\"py.typed\", \"*.pyi\", \"**/*.pyi\"]\n",
        distribution_name = input.distribution_name,
        import_name = input.import_name,
        packages = packages.join(", "),
        package_version = input.package_version,
        runtime_version = input.runtime_version,
    )
}

fn render_winrt_package(input: &BindingsPackageManifestInput<'_>) -> String {
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

    out.push_str("    \"./proxy\": {\n");
    out.push_str("      \"types\": \"./index.d.ts\",\n");
    out.push_str("      \"require\": \"./index.proxy.js\"\n");
    out.push_str("    }");

    for name in input.winrt_subpath_names {
        out.push_str(",\n");
        out.push_str(&format!("    \"./{}\": {{\n", name));
        out.push_str(&format!("      \"types\": \"./{}.d.ts\",\n", name));
        out.push_str(&format!("      \"default\": \"./{}.js\"\n", name));
        out.push_str("    }");
    }

    append_com_exports(&mut out, input.com_subpath_names);
    out.push_str("\n  }\n");
    out.push_str("}\n");
    out
}

fn render_com_only_package(com_subpath_names: &BTreeSet<String>) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"name\": \"@winapp/bindings\",\n");
    out.push_str("  \"type\": \"commonjs\",\n");
    out.push_str("  \"sideEffects\": false,\n");
    out.push_str("  \"main\": \"./index.js\",\n");
    out.push_str("  \"types\": \"./index.d.ts\",\n");
    out.push_str("  \"exports\": {\n");
    out.push_str("    \".\": {\n");
    out.push_str("      \"types\": \"./com/index.d.ts\",\n");
    out.push_str("      \"import\": \"./com/index.mjs\",\n");
    out.push_str("      \"require\": \"./com/index.js\",\n");
    out.push_str("      \"default\": \"./com/index.js\"\n");
    out.push_str("    }");

    // Preserve the original COM-only deep-import paths while storing all COM
    // implementation files under the domain-specific `com/` directory.
    for name in com_subpath_names {
        out.push_str(",\n");
        out.push_str(&format!("    \"./{}\": {{\n", name));
        out.push_str(&format!("      \"types\": \"./com/{}.d.ts\",\n", name));
        out.push_str(&format!("      \"import\": \"./com/{}.js\",\n", name));
        out.push_str(&format!("      \"require\": \"./com/{}.js\",\n", name));
        out.push_str(&format!("      \"default\": \"./com/{}.js\"\n", name));
        out.push_str("    }");
    }

    append_com_exports(&mut out, com_subpath_names);
    out.push_str("\n  }\n");
    out.push_str("}\n");
    out
}

fn append_com_exports(out: &mut String, com_subpath_names: &BTreeSet<String>) {
    if com_subpath_names.is_empty() {
        return;
    }

    out.push_str(",\n");
    out.push_str("    \"./com\": {\n");
    out.push_str("      \"types\": \"./com/index.d.ts\",\n");
    out.push_str("      \"import\": \"./com/index.mjs\",\n");
    out.push_str("      \"require\": \"./com/index.js\"\n");
    out.push_str("    },\n");
    out.push_str("    \"./com/*\": {\n");
    out.push_str("      \"types\": \"./com/*.d.ts\",\n");
    out.push_str("      \"import\": \"./com/*.js\",\n");
    out.push_str("      \"require\": \"./com/*.js\"\n");
    out.push_str("    }");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bindings_produces_root_only() {
        let names: BTreeSet<String> = BTreeSet::new();
        let out = render_package_json(&PackageManifestInput {
            subpath_names: &names,
        });
        assert!(
            out.contains("\"name\": \"@winapp/bindings\""),
            "should set package name"
        );
        assert!(
            out.contains("\"type\": \"commonjs\""),
            "should set commonjs"
        );
        assert!(
            out.contains("\"sideEffects\": false"),
            "should set sideEffects: false"
        );
        assert!(out.contains("\"./index.mjs\""), "should point ESM at .mjs");
        assert!(out.contains("\"./index.js\""), "should point CJS at .js");
        assert!(
            out.contains("\"./index.proxy.js\""),
            "should expose proxy barrel"
        );
        assert!(!out.contains("    },\n  }"), "must not emit trailing comma");
    }

    #[test]
    fn subpath_exports_are_alphabetical() {
        let mut names: BTreeSet<String> = BTreeSet::new();
        names.insert("Uri".into());
        names.insert("AppWindow".into());
        names.insert("LanguageModel".into());
        let out = render_package_json(&PackageManifestInput {
            subpath_names: &names,
        });
        let a = out.find("\"./AppWindow\"").expect("AppWindow present");
        let l = out
            .find("\"./LanguageModel\"")
            .expect("LanguageModel present");
        let u = out.find("\"./Uri\"").expect("Uri present");
        assert!(a < l && l < u, "subpaths must be alphabetically ordered");
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

    #[test]
    fn mixed_package_keeps_duplicate_names_in_separate_domains() {
        let winrt = BTreeSet::from(["Uri".to_string()]);
        let com = BTreeSet::from(["Uri".to_string()]);
        let out = render_bindings_package_json(&BindingsPackageManifestInput {
            has_winrt_root: true,
            winrt_subpath_names: &winrt,
            com_subpath_names: &com,
        });

        assert!(out.contains("\"./Uri\""));
        assert!(out.contains("\"./com\""));
        assert!(out.contains("\"./com/*\""));
        assert!(out.contains("\"types\": \"./com/*.d.ts\""));
        assert!(out.contains("\"import\": \"./com/*.js\""));
        assert!(out.contains("\"require\": \"./com/index.js\""));
        assert!(out.contains("\"require\": \"./com/*.js\""));
    }

    #[test]
    fn com_only_package_preserves_legacy_root_subpaths() {
        let com = BTreeSet::from(["ITaskbarList3".to_string()]);
        let winrt = BTreeSet::new();
        let out = render_bindings_package_json(&BindingsPackageManifestInput {
            has_winrt_root: false,
            winrt_subpath_names: &winrt,
            com_subpath_names: &com,
        });

        assert!(out.contains("\"type\": \"commonjs\""));
        assert!(out.contains("\"./ITaskbarList3\""));
        assert!(out.contains("\"types\": \"./com/ITaskbarList3.d.ts\""));
        assert!(out.contains("\"import\": \"./com/ITaskbarList3.js\""));
        assert!(out.contains("\"require\": \"./com/ITaskbarList3.js\""));
        assert!(out.contains("\"import\": \"./com/index.mjs\""));
        assert!(out.contains("\"require\": \"./com/index.js\""));
        assert!(out.contains("\"./com/*\""));
    }

    #[test]
    fn python_manifest_pins_runtime_and_typed_package_data() {
        let out = render_python_pyproject(&PythonPackageManifestInput {
            distribution_name: "contoso-winrt",
            import_name: "contoso_winrt",
            package_version: "1.2.3",
            runtime_version: "0.4.5",
            namespace_packages: &["windows".into(), "windows.foundation".into()],
        });

        assert!(out.contains("name = \"contoso-winrt\""));
        assert!(out.contains("requires-python = \">=3.11,<3.15\""));
        assert!(out.contains(
            "packages = [\"contoso_winrt\", \"contoso_winrt.windows\", \
             \"contoso_winrt.windows.foundation\"]"
        ));
        assert!(out.contains("dependencies = [\"dynwinrt==0.4.5\"]"));
        assert!(out.contains("\"contoso_winrt\" = [\"py.typed\", \"*.pyi\", \"**/*.pyi\"]"));
    }
}
