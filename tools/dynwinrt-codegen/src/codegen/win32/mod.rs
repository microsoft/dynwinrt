// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Small, contract-driven flat Win32 projection, independent of COM and WinRT.

mod ir;
mod project;
mod render;

use std::collections::BTreeMap;

use crate::{win32_contracts::Registry, win32_metadata};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmittedExport {
    pub name: String,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct GeneratedNamespace {
    pub namespace: String,
    pub js: String,
    pub dts: String,
    pub exports: Vec<String>,
    pub omissions: Vec<OmittedExport>,
}

pub fn has_flat_functions(
    winmd_paths: &str,
    namespace: &str,
    class_name: &str,
) -> Result<bool, String> {
    win32_metadata::has_exports(winmd_paths, namespace, class_name)
}

pub fn generate_namespace(
    winmd_paths: &str,
    namespace: &str,
    runtime_import: &str,
) -> Result<GeneratedNamespace, String> {
    let registry = Registry::builtin()?;
    let raw = win32_metadata::read_exports(winmd_paths, namespace)?;
    let mut counts = BTreeMap::new();
    for export in &raw {
        *counts.entry(export.name.as_str()).or_insert(0) += 1;
    }
    let mut functions = Vec::new();
    let mut omissions = Vec::new();
    for export in &raw {
        let projected = if counts[export.name.as_str()] != 1 {
            Err("win32.ambiguous-metadata-export".into())
        } else if let Some(entry) = registry.find(export) {
            project::function(export, entry)
        } else {
            Err("win32.missing-contract".into())
        };
        match projected {
            Ok(function) => functions.push(function),
            Err(reason) => omissions.push(OmittedExport {
                name: export.name.clone(),
                reason,
            }),
        }
    }
    omissions.dedup();
    if functions.is_empty() {
        let reasons = omissions
            .iter()
            .map(|o| format!("{}: {}", o.name, o.reason))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(format!(
            "win32.no-complete-exports: {namespace} (no complete supported flat exports); {reasons}"
        ));
    }
    let file = project::file(runtime_import, functions)?;
    Ok(GeneratedNamespace {
        namespace: namespace.into(),
        js: render::javascript(&file),
        dts: render::declarations(&file),
        exports: file.functions.iter().map(|f| f.name.clone()).collect(),
        omissions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win32_generation_is_bounded_and_deterministic() {
        let Ok(paths) = std::env::var("DYNWINRT_WIN32_WINMD") else {
            return;
        };
        for (namespace, names) in [
            (
                "Windows.Win32.System.SystemInformation",
                vec!["getTickCount", "getTickCount64"],
            ),
            (
                "Windows.Win32.System.Registry",
                vec!["regCloseKey", "regOpenKeyExW", "regQueryValueExW"],
            ),
        ] {
            assert!(has_flat_functions(&paths, namespace, "Apis").unwrap());
            assert!(!has_flat_functions(&paths, namespace, "NotApis").unwrap());
            let output =
                generate_namespace(&paths, namespace, "@microsoft/dynwinrt/win32").unwrap();
            assert_eq!(output.exports, names);
            assert!(!output.omissions.is_empty());
            assert!(
                output
                    .omissions
                    .iter()
                    .all(|o| o.reason == "win32.missing-contract")
            );
            assert_eq!(
                output.js,
                generate_namespace(&paths, namespace, "@microsoft/dynwinrt/win32")
                    .unwrap()
                    .js
            );
            assert!(output.js.contains("@microsoft/dynwinrt/win32/unsafe"));
            assert!(!output.js.contains("DynWinRt"));
        }
        let registry =
            generate_namespace(&paths, "Windows.Win32.System.Registry", ".\\win32.js").unwrap();
        assert!(registry.js.contains("win32-unsafe.js"));
        assert!(
            registry
                .dts
                .contains("regCloseKey(hKey: Win32Resource): number")
        );
        assert!(registry.dts.contains("lpSubKey: string | null"));
        assert!(registry.dts.contains("lpData: Buffer | Uint8Array | null"));
        assert!(registry.dts.contains("lpcbData: number | null"));
        assert!(
            generate_namespace(
                &paths,
                "Windows.Win32.System.Threading",
                "@microsoft/dynwinrt/win32"
            )
            .unwrap_err()
            .contains("win32.no-complete-exports")
        );
        assert!(
            generate_namespace(
                &paths,
                "Windows.Win32.System.Registry",
                "@microsoft/dynwinrt/com"
            )
            .is_err()
        );
        assert!(
            generate_namespace(
                &format!("{paths};{paths}"),
                "Windows.Win32.System.Registry",
                "@microsoft/dynwinrt/win32"
            )
            .unwrap_err()
            .contains("win32.ambiguous-metadata-export")
        );
    }
}
