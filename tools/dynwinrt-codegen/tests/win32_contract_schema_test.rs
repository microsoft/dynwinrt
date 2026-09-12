// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{fs, path::Path};

use serde_json::Value;
use sha2::{Digest, Sha256};

fn lf_hash(text: &str) -> String {
    format!(
        "{:X}",
        Sha256::digest(text.replace("\r\n", "\n").as_bytes())
    )
}

#[test]
fn win32_call_contract_schema_matches_shared_protocol() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(r"contracts\win32");
    let generated = format!(
        "{}\n",
        serde_json::to_string_pretty(&dynwinrt_win32_contracts::schema()).unwrap()
    );
    let manifest_path = directory.join("manifest.json");
    let mut manifest: Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    if std::env::var_os("DYNWINRT_UPDATE_WIN32_SCHEMA").is_some() {
        fs::write(directory.join("call-contract.schema.json"), &generated).unwrap();
        manifest["schema"]["sha256"] =
            lf_hash(&fs::read_to_string(directory.join("schema.json")).unwrap()).into();
        for pin in manifest["files"].as_array_mut().unwrap() {
            let text = fs::read_to_string(directory.join(pin["file"].as_str().unwrap())).unwrap();
            pin["sha256"] = lf_hash(&text).into();
        }
        fs::write(
            &manifest_path,
            format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
        )
        .unwrap();
    }
    assert_eq!(
        fs::read_to_string(directory.join("call-contract.schema.json"))
            .unwrap()
            .replace("\r\n", "\n"),
        generated,
        "Set DYNWINRT_UPDATE_WIN32_SCHEMA=1 and rerun this test to update the shared schema and LF manifest hashes"
    );
    let production: Value =
        serde_json::from_str(&fs::read_to_string(directory.join("schema.json")).unwrap()).unwrap();
    assert_eq!(
        production["$defs"]["callContract"]["$ref"],
        "call-contract.schema.json"
    );
    assert_eq!(
        production["$defs"]["resultContract"]["$ref"],
        "call-contract.schema.json#/$defs/ResultContract"
    );
    for duplicate in [
        "outputRule",
        "condition",
        "inputPredicate",
        "outputAction",
        "resourceEffect",
    ] {
        assert!(
            production["$defs"].get(duplicate).is_none(),
            "handwritten protocol definition: {duplicate}"
        );
    }
    for pin in std::iter::once(&manifest["schema"]).chain(manifest["files"].as_array().unwrap()) {
        let text = fs::read_to_string(directory.join(pin["file"].as_str().unwrap())).unwrap();
        assert_eq!(pin["sha256"], lf_hash(&text), "{}", pin["file"]);
    }
}
