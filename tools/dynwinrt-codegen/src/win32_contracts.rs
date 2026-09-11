// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Built-in Win32 evidence, not application-supplied projection overrides.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::win32_metadata::{Export, sha256};

const MANIFEST: &str = include_str!("../contracts/win32/manifest.json");
const SCHEMA: &str = include_str!("../contracts/win32/schema.json");
const SCALARS: &str = include_str!("../contracts/win32/scalar-returns.json");
const STATUSES: &str = include_str!("../contracts/win32/status-returns.json");
pub(crate) const METADATA_SHA256: &str =
    "B64EE4818A7ED9F9D135038D58C51BD08369184D4D5ED428F20E9DE55DF8121D";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MetadataPin {
    package: String,
    version: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FilePin {
    file: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    metadata: MetadataPin,
    schema: FilePin,
    files: Vec<FilePin>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Selector {
    pub namespace: String,
    pub container: String,
    pub name: String,
    pub dll: String,
    pub entry_point: String,
    pub calling_convention: String,
    pub architectures: u32,
    pub source_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum EvidenceKind {
    MicrosoftLearn,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    kind: EvidenceKind,
    url: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Cleanup {
    RegCloseKey,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PredefinedOwnership {
    Borrowed,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FailureOutput {
    Unspecified,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum ParameterContract {
    U32Input {},
    Utf16Input {
        nullable: bool,
    },
    BorrowedHkey {
        reject_performance_data: bool,
    },
    OwnedHkeyOutput {
        cleanup: Cleanup,
        borrowed_from: usize,
        predefined: PredefinedOwnership,
        failure: FailureOutput,
    },
    ConsumedHkey {
        cleanup: Cleanup,
    },
    ReservedNull {},
    U32Output {},
    OptionalByteOutput {
        count_parameter: usize,
    },
    ByteCapacityRequiredSize {
        buffer_parameter: usize,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum Contract {
    DirectScalar { parameters: Vec<ParameterContract> },
    StatusZero { parameters: Vec<ParameterContract> },
}

impl Contract {
    pub fn parameters(&self) -> &[ParameterContract] {
        match self {
            Self::DirectScalar { parameters } | Self::StatusZero { parameters } => parameters,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub id: String,
    pub selector: Selector,
    pub contract: Contract,
    evidence: Vec<Evidence>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Group {
    schema_version: u32,
    entries: Vec<Entry>,
}

pub(crate) struct Registry {
    pub entries: Vec<Entry>,
}

impl Registry {
    pub fn builtin() -> Result<&'static Self, String> {
        static REGISTRY: OnceLock<Result<Registry, String>> = OnceLock::new();
        REGISTRY
            .get_or_init(|| Self::load(MANIFEST, SCHEMA, &[SCALARS, STATUSES]))
            .as_ref()
            .map_err(Clone::clone)
    }

    fn load(manifest: &str, schema: &str, groups: &[&str]) -> Result<Self, String> {
        let manifest: Manifest = serde_json::from_str(manifest).map_err(|e| e.to_string())?;
        if manifest.schema_version != 1
            || manifest.metadata.package != "Microsoft.Windows.SDK.Win32Metadata"
            || manifest.metadata.version != "71.0.14-preview"
            || manifest.metadata.sha256 != METADATA_SHA256
            || manifest.schema.file != "schema.json"
            || manifest.schema.sha256 != sha256(schema.as_bytes())
            || manifest.files.len() != 2
            || groups.len() != 2
        {
            return Err("win32.registry: schema or provenance integrity mismatch".into());
        }
        let mut entries = Vec::new();
        for ((pin, data), file) in manifest
            .files
            .iter()
            .zip(groups)
            .zip(["scalar-returns.json", "status-returns.json"])
        {
            if pin.file != file || pin.sha256 != sha256(data.as_bytes()) {
                return Err("win32.registry: data integrity mismatch".into());
            }
            let group: Group =
                serde_json::from_str(data).map_err(|e| format!("win32.registry: {e}"))?;
            if group.schema_version != 1
                || group.entries.iter().any(|e| match &e.contract {
                    Contract::DirectScalar { .. } => file != "scalar-returns.json",
                    Contract::StatusZero { .. } => file != "status-returns.json",
                })
            {
                return Err("win32.registry: invalid semantic group".into());
            }
            entries.extend(group.entries);
        }
        Self::validate_entries(entries)
    }

    fn validate_entries(entries: Vec<Entry>) -> Result<Self, String> {
        let mut ids = BTreeSet::new();
        let mut selectors = BTreeSet::new();
        for entry in &entries {
            let selector = &entry.selector;
            if entry.id.is_empty()
                || !entry
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b".-".contains(&c))
                || !ids.insert(&entry.id)
                || !selectors.insert((&selector.namespace, &selector.container, &selector.name))
            {
                return Err("win32.registry: invalid or duplicate ID/selector".into());
            }
            if !selector.namespace.starts_with("Windows.Win32.")
                || selector.container != "Apis"
                || !identifier(&selector.name)
                || !identifier(&selector.entry_point)
                || selector.calling_convention != "system"
                || selector.architectures != 7
                || !valid_hash(&selector.source_fingerprint)
                || !selector.dll.to_ascii_lowercase().ends_with(".dll")
                || selector.dll.contains("..")
                || !selector
                    .dll
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
                || entry.evidence.is_empty()
                || entry.evidence.iter().any(|e| {
                    let EvidenceKind::MicrosoftLearn = e.kind;
                    !e.url
                        .starts_with("https://learn.microsoft.com/en-us/windows/win32/api/")
                        || e.url.bytes().any(|c| c.is_ascii_whitespace())
                })
            {
                return Err("win32.registry: incomplete exact evidence".into());
            }
        }
        Ok(Self { entries })
    }

    pub fn find(&self, raw: &Export) -> Option<&Entry> {
        self.entries.iter().find(|entry| {
            entry.selector.namespace == raw.namespace
                && entry.selector.container == raw.container
                && entry.selector.name == raw.name
        })
    }
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'A'..=b'F').contains(&c))
}

pub(crate) fn identifier(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .enumerate()
            .all(|(i, c)| c.is_ascii_alphabetic() || c == b'_' || (i > 0 && c.is_ascii_digit()))
}

impl Entry {
    pub fn validate_selector(&self, raw: &Export) -> Result<(), String> {
        let selector = &self.selector;
        if raw.metadata_sha256 != METADATA_SHA256 {
            return Err("win32.metadata-hash-mismatch".into());
        }
        if selector.namespace != raw.namespace
            || selector.container != raw.container
            || selector.name != raw.name
            || selector.dll != raw.dll
            || selector.entry_point != raw.entry_point
            || selector.calling_convention != raw.calling_convention
            || selector.architectures != raw.architectures
            || selector.source_fingerprint != raw.fingerprint()
        {
            return Err("win32.signature-drift".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win32_registry_integrity_and_strict_json() {
        let registry = Registry::builtin().unwrap();
        assert_eq!(registry.entries.len(), 5);
        assert!(Registry::load(&format!("{MANIFEST} "), SCHEMA, &[SCALARS, STATUSES]).is_ok());
        let mut manifest: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
        manifest["unexpected"] = true.into();
        assert!(Registry::load(&manifest.to_string(), SCHEMA, &[SCALARS, STATUSES]).is_err());
        assert!(Registry::load(MANIFEST, &format!("{SCHEMA} "), &[SCALARS, STATUSES]).is_err());
        assert!(Registry::load(MANIFEST, SCHEMA, &[&format!("{SCALARS} "), STATUSES]).is_err());
        for malformed in [
            r#"{"kind":"callback"}"#,
            r#"{"kind":"u32-input","js":"process.exit()"}"#,
            r#"{"kind":"consumed-hkey","cleanup":"CloseHandle"}"#,
            r#"{"kind":"utf16-input","nullable":true,"runtimeMethod":"rawPointer"}"#,
        ] {
            assert!(
                serde_json::from_str::<ParameterContract>(malformed).is_err(),
                "{malformed}"
            );
        }
        let mut group: serde_json::Value = serde_json::from_str(SCALARS).unwrap();
        group["entries"][0]["extra"] = true.into();
        assert!(serde_json::from_value::<Group>(group).is_err());
        let mut duplicate = registry.entries.clone();
        duplicate.push(duplicate[0].clone());
        assert!(Registry::validate_entries(duplicate.clone()).is_err());
        duplicate.last_mut().unwrap().id += ".different";
        assert!(Registry::validate_entries(duplicate).is_err());
        let mut duplicate = registry.entries.clone();
        duplicate[1].id = duplicate[0].id.clone();
        assert!(Registry::validate_entries(duplicate).is_err());
    }

    #[test]
    fn win32_registry_every_selector_field_is_pinned() {
        let Ok(paths) = std::env::var("DYNWINRT_WIN32_WINMD") else {
            return;
        };
        let registry = Registry::builtin().unwrap();
        for namespace in [
            "Windows.Win32.System.SystemInformation",
            "Windows.Win32.System.Registry",
        ] {
            for raw in crate::win32_metadata::read_exports(&paths, namespace).unwrap() {
                let Some(entry) = registry.find(&raw) else {
                    continue;
                };
                entry.validate_selector(&raw).unwrap();
                let mutate: &[fn(&mut Export)] = &[
                    |e| e.namespace.push('x'),
                    |e| e.container.push('x'),
                    |e| e.name.push('x'),
                    |e| e.dll.push('x'),
                    |e| e.entry_point.push('x'),
                    |e| e.calling_convention.push('x'),
                    |e| e.architectures = 1,
                    |e| e.signature.push('0'),
                    |e| e.impl_flags.push('x'),
                    |e| e.method_flags.push('x'),
                    |e| e.import_flags.push('x'),
                    |e| e.return_type.pointers.push(false),
                    |e| e.metadata_sha256.push('0'),
                ];
                for mutation in mutate {
                    let mut changed = raw.clone();
                    mutation(&mut changed);
                    assert!(entry.validate_selector(&changed).is_err(), "{}", raw.name);
                }
                if !raw.parameters.is_empty() {
                    for mutation in [
                        |p: &mut crate::win32_metadata::Parameter| p.input = !p.input,
                        |p: &mut crate::win32_metadata::Parameter| p.output = !p.output,
                        |p: &mut crate::win32_metadata::Parameter| p.optional = !p.optional,
                        |p: &mut crate::win32_metadata::Parameter| {
                            p.const_attribute = !p.const_attribute
                        },
                        |p: &mut crate::win32_metadata::Parameter| p.typ.pointers.push(true),
                        |p: &mut crate::win32_metadata::Parameter| p.typ.underlying = None,
                        |p: &mut crate::win32_metadata::Parameter| {
                            p.byte_count_parameter = Some(99)
                        },
                    ] {
                        let mut changed = raw.clone();
                        mutation(&mut changed.parameters[0]);
                        assert!(entry.validate_selector(&changed).is_err());
                    }
                }
            }
        }
    }
}
