// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Fixed metadata input for Windows SDK corpus assertions, not runtime discovery.

use std::path::PathBuf;

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    package: String,
    version: String,
    relative_path: String,
    sha256: String,
}

pub(crate) fn windows_winmd() -> String {
    let corpus: Corpus = serde_json::from_str(include_str!("../tests/windows_sdk_corpus.json"))
        .expect("valid Windows SDK corpus descriptor");
    let path = std::env::var_os("DYNWINRT_WINDOWS_WINMD")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd",
            )
        });
    let instructions = format!(
        "Windows corpus tests require {} {} ({}). \
         Run eng\\ci\\restore-windows-metadata.ps1 -OutputDirectory <scratch-directory> \
         and set DYNWINRT_WINDOWS_WINMD to its returned path; \
         a rolling SDK installation is not a pinned corpus.",
        corpus.package, corpus.version, corpus.relative_path,
    );
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("Cannot read {}: {error}. {instructions}", path.display()));
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        corpus.sha256,
        "Unexpected Windows metadata content at {}. {instructions}",
        path.display()
    );
    path.to_str()
        .expect("Windows metadata path must be Unicode")
        .into()
}
