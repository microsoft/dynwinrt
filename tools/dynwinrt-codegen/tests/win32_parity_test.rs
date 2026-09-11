// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use dynwinrt_codegen::{codegen::win32, win32_metadata};
use serde::Deserialize;
use sha2::{Digest, Sha256};

type Function = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Baseline {
    schema_version: u32,
    baseline: Source,
    eligible_functions: usize,
    complete_functions: usize,
    containers: BTreeMap<String, Container>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Source {
    commit: String,
    metadata_sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Container {
    namespace: String,
    container: String,
    functions: Vec<Function>,
    async_helpers: BTreeSet<String>,
    public_exports: BTreeSet<String>,
    enum_hash: String,
    builder_hash: String,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:X}", Sha256::digest(bytes))
}

#[test]
fn full_pr102_exports_and_projection_semantics_are_preserved() {
    let Ok(path) = std::env::var("DYNWINRT_WIN32_WINMD") else {
        assert_ne!(
            std::env::var("DYNWINRT_REQUIRE_WIN32_METADATA").as_deref(),
            Ok("1"),
            "the full Win32 parity gate requires DYNWINRT_WIN32_WINMD"
        );
        eprintln!("Skipping full Win32 parity gate: no configured metadata");
        return;
    };
    assert!(Path::new(&path).is_file(), "configured metadata must exist");
    let baseline: Baseline =
        serde_json::from_str(include_str!("fixtures/win32-pr102.json")).unwrap();
    assert_eq!(baseline.schema_version, 1);
    assert_eq!(
        baseline.baseline.commit,
        "13e47588fc54b14cf11364af329aa882ddefaddc"
    );
    assert_eq!(
        hash(&fs::read(&path).unwrap()),
        baseline.baseline.metadata_sha256
    );
    let all = win32_metadata::parse_all_functions(&path).expect("parse complete metadata");
    assert_eq!(all.len(), baseline.eligible_functions);
    let exports = regex::Regex::new(r"\bexports\.([A-Za-z_$][A-Za-z0-9_$]*)").unwrap();
    let mut problems = Vec::new();
    let mut complete = 0;
    for (identity, expected) in baseline.containers {
        let Some(raw) = win32_metadata::parse_apis(&path, &expected.namespace, &expected.container)
        else {
            problems.push(format!("{identity}: missing container"));
            continue;
        };
        let projection = win32::project_apis(&raw);
        complete += projection.complete_count();
        let functions = projection
            .projected
            .functions
            .iter()
            .map(|function| (function.metadata_name.as_str(), function))
            .collect::<BTreeMap<_, _>>();
        for (name, dll, entry, js_name, alias, subsystem, semantic_hash) in expected.functions {
            let Some(actual) = functions.get(name.as_str()) else {
                problems.push(format!("{identity}::{name}: missing supported function"));
                continue;
            };
            if !actual.runtime.dll.eq_ignore_ascii_case(&dll)
                || actual.runtime.entry_point != entry
                || actual.js_name != js_name
                || actual.unicode_alias != alias
                || format!("{:?}", actual.subsystem) != subsystem
            {
                problems.push(format!(
                    "{identity}::{name}: native/public identity or subsystem changed"
                ));
            }
            let semantic = format!(
                "{:?}\n{:?}\n{:?}\n{:?}",
                actual.parameters, actual.inputs, actual.runtime, actual.return_shape
            );
            if hash(semantic.as_bytes()) != semantic_hash {
                problems.push(format!(
                    "{identity}::{name}: ABI or projected contract changed"
                ));
            }
        }
        let asynchronous = projection
            .projected
            .async_functions
            .iter()
            .map(|function| function.js_name.clone())
            .collect::<BTreeSet<_>>();
        for missing in expected.async_helpers.difference(&asynchronous) {
            problems.push(format!(
                "{identity}::{missing}: missing asynchronous helper"
            ));
        }
        if hash(format!("{:?}", projection.projected.enums).as_bytes()) != expected.enum_hash {
            problems.push(format!("{identity}: enum definitions changed"));
        }
        if hash(format!("{:?}", projection.projected.native_builders).as_bytes())
            != expected.builder_hash
        {
            problems.push(format!("{identity}: native builder contracts changed"));
        }
        let (generated, _) = win32::generate_apis_files(&raw, "@microsoft/dynwinrt/win32");
        let actual_exports = exports
            .captures_iter(&generated.js)
            .map(|capture| capture[1].to_string())
            .collect::<BTreeSet<_>>();
        for missing in expected.public_exports.difference(&actual_exports) {
            problems.push(format!(
                "{identity}::{missing}: missing public export or alias"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "{} baseline parity differences:\n{}",
        problems.len(),
        problems
            .iter()
            .take(80)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(complete >= baseline.complete_functions);
}
