// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::com_metadata::{
    ComInterfaceMeta, RawComMethod, RawComParam, RawComType, RawConstness, RawLayoutKind,
    RawNamedKind, RawNativeField, RawNativeLayout, RawNativeLayoutSet, RawNativeType, RawPacking,
    RawParamDirection, RawSafeArrayOwnership, RawStringEncoding,
};

use super::generate_com_interface_files;

pub const OFFICIAL_METADATA_VERSION: &str = "71.0.14-preview";
pub const OFFICIAL_METADATA_SHA256: &str =
    "B64EE4818A7ED9F9D135038D58C51BD08369184D4D5ED428F20E9DE55DF8121D";
pub const REGENERATION_COMMAND: &str = "cargo run -p dynwinrt-codegen -- com-capability-census --winmd <Windows.Win32.winmd> --output-dir docs/status/generated";

const CATEGORIES: [&str; 21] = [
    "Scalar",
    "Guid",
    "Enum",
    "NativeStruct",
    "NativeUnion",
    "Pointer",
    "Handle",
    "DataPointer",
    "StringPointer",
    "Bstr",
    "HString",
    "ComInterface",
    "CountedBuffer",
    "SafeArray",
    "Variant",
    "PropVariant",
    "DispatchParams",
    "ExcepInfo",
    "StatStg",
    "FunctionPointer",
    "Unknown",
];

const STANDARD_CLEANUPS: [&str; 12] = [
    "IUnknown::Release",
    "CoTaskMemFree",
    "LocalFree",
    "GlobalFree",
    "SysFreeString",
    "VariantClear",
    "PropVariantClear",
    "SafeArrayDestroy",
    "ReleaseStgMedium",
    "CloseHandle",
    "DestroyIcon",
    "DeleteObject",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CensusTarget {
    X64,
    I686,
    Arm64,
}

impl CensusTarget {
    const ALL: [Self; 3] = [Self::X64, Self::I686, Self::Arm64];

    const fn key(self) -> &'static str {
        match self {
            Self::X64 => "x64",
            Self::I686 => "i686",
            Self::Arm64 => "arm64",
        }
    }

    const fn metadata_mask(self) -> u8 {
        match self {
            Self::I686 => 1,
            Self::X64 => 2,
            Self::Arm64 => 4,
        }
    }

    const fn pointer_size(self) -> usize {
        match self {
            Self::I686 => 4,
            Self::X64 | Self::Arm64 => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RawClassification {
    RawMetadataComplete,
    RawManualContract,
    RawRuntimeBlocked,
}

impl RawClassification {
    pub const fn key(self) -> &'static str {
        match self {
            Self::RawMetadataComplete => "raw_metadata_complete",
            Self::RawManualContract => "raw_manual_contract",
            Self::RawRuntimeBlocked => "raw_runtime_blocked",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupAvailability {
    #[default]
    NoneRequired,
    StandardSupported,
    KnownExternal,
    Unknown,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct LifecycleFlags {
    pub cleanup: CleanupAvailability,
    pub requires_external_pointer_or_callback: bool,
    pub requires_external_acquisition: bool,
    pub requires_current_apartment: bool,
}

impl LifecycleFlags {
    fn merge(&mut self, other: &Self) {
        self.cleanup = self.cleanup.max(other.cleanup);
        self.requires_external_pointer_or_callback |= other.requires_external_pointer_or_callback;
        self.requires_external_acquisition |= other.requires_external_acquisition;
        self.requires_current_apartment |= other.requires_current_apartment;
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TargetCapability {
    pub classification: RawClassification,
    pub first_blocker_reason: Option<String>,
    pub blocker_reasons: Vec<String>,
    pub manual_contract_reasons: Vec<String>,
    pub lifecycle: LifecycleFlags,
}

#[derive(Debug, Clone, Serialize)]
pub struct InterfaceCapability {
    pub namespace: String,
    pub name: String,
    pub iid: String,
    pub is_iunknown_rooted: bool,
    pub method_count: usize,
    pub first_vtable_slot: Option<usize>,
    pub last_vtable_slot: Option<usize>,
    pub safe_complete: bool,
    pub safe_error: Option<String>,
    pub evidence_class: Option<SafeEvidenceClass>,
    pub standard_rule_ids: Vec<String>,
    pub exact_entry_ids: Vec<String>,
    pub exact_family_ids: Vec<String>,
    pub exact_contract_kinds: Vec<String>,
    #[serde(skip)]
    pub exact_entry_kinds: BTreeMap<String, String>,
    pub metadata_attributes: Vec<String>,
    pub targets: BTreeMap<String, TargetCapability>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeEvidenceClass {
    StandardDerived,
    ExactRegistryDependent,
}

impl SafeEvidenceClass {
    const fn key(self) -> &'static str {
        match self {
            Self::StandardDerived => "standard_derived",
            Self::ExactRegistryDependent => "exact_registry_dependent",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MethodCapability {
    pub name: String,
    pub projected_name: String,
    pub declaring_iid: String,
    pub absolute_slot: usize,
    pub signature_fingerprint: String,
    pub targets: BTreeMap<String, TargetCapability>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetadataIdentity {
    pub package: String,
    pub version: String,
    pub file: String,
    pub sha256: String,
    pub label: String,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataFileIdentity {
    pub file: String,
    pub package: String,
    pub version: String,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct MetadataSetIdentity {
    pub set_sha256: String,
    pub files: Vec<MetadataFileIdentity>,
    pub(super) definition_files: BTreeMap<String, Vec<MetadataFileIdentity>>,
}

impl MetadataSetIdentity {
    pub fn defining_file(&self, namespace: &str, name: &str) -> Option<MetadataFileIdentity> {
        let definitions = self.definition_files.get(&format!("{namespace}.{name}"))?;
        (definitions.len() == 1).then(|| definitions[0].clone())
    }
}

pub fn metadata_set_identity_for_paths(winmd_paths: &str) -> Result<MetadataSetIdentity, String> {
    let mut canonical_paths = BTreeMap::<String, PathBuf>::new();
    for path in winmd_paths
        .split(';')
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        let canonical = Path::new(path)
            .canonicalize()
            .map_err(|error| format!("Failed to resolve metadata file {path}: {error}"))?;
        let key = if cfg!(windows) {
            canonical.to_string_lossy().to_ascii_lowercase()
        } else {
            canonical.to_string_lossy().into_owned()
        };
        canonical_paths.entry(key).or_insert(canonical);
    }
    if canonical_paths.is_empty() {
        return Err("No metadata path was supplied".into());
    }

    let mut entries = canonical_paths
        .into_values()
        .map(|path| {
            let bytes = fs::read(&path)
                .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
            let sha256 = format!("{:X}", Sha256::digest(&bytes));
            let file = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("Metadata path has no file name: {}", path.display()))?
                .to_string();
            let (package, version) = if sha256 == OFFICIAL_METADATA_SHA256 {
                (
                    "Microsoft.Windows.SDK.Win32Metadata".to_string(),
                    OFFICIAL_METADATA_VERSION.to_string(),
                )
            } else {
                ("unknown".to_string(), "unknown".to_string())
            };
            Ok((
                path,
                MetadataFileIdentity {
                    file,
                    package,
                    version,
                    sha256,
                },
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    entries.sort_by(|left, right| {
        left.1
            .file
            .to_ascii_lowercase()
            .cmp(&right.1.file.to_ascii_lowercase())
            .then_with(|| left.1.file.cmp(&right.1.file))
            .then_with(|| left.1.sha256.cmp(&right.1.sha256))
    });
    entries.dedup_by(|left, right| left.1 == right.1);

    let files = entries
        .iter()
        .map(|(_, identity)| identity.clone())
        .collect::<Vec<_>>();
    let canonical = serde_json::to_vec(&files)
        .map_err(|error| format!("Failed to fingerprint metadata set: {error}"))?;
    let set_sha256 = format!("{:X}", Sha256::digest(&canonical));
    let mut definition_files = BTreeMap::<String, Vec<MetadataFileIdentity>>::new();
    for (path, identity) in entries {
        if let Ok(definitions) = super::typedef_inventory::read_typedefs(&path) {
            for definition in definitions {
                definition_files
                    .entry(format!("{}.{}", definition.namespace, definition.name))
                    .or_default()
                    .push(identity.clone());
            }
        }
    }
    for identities in definition_files.values_mut() {
        identities.sort_by(|left, right| {
            left.file
                .cmp(&right.file)
                .then_with(|| left.sha256.cmp(&right.sha256))
        });
        identities.dedup();
    }
    Ok(MetadataSetIdentity {
        set_sha256,
        files,
        definition_files,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct TargetCounts {
    pub raw_metadata_complete: usize,
    pub raw_manual_contract: usize,
    pub raw_runtime_blocked: usize,
    pub safe_incomplete_raw_metadata_complete: usize,
    pub safe_incomplete_raw_manual_contract: usize,
    pub safe_incomplete_raw_runtime_blocked: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryCounts {
    pub category: String,
    pub direct_occurrences: usize,
    pub expanded_occurrences: usize,
    pub named_types: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilitySummary {
    pub schema_version: u32,
    pub metadata: MetadataIdentity,
    pub regeneration_command: String,
    pub definitions: BTreeMap<String, String>,
    pub reason_definitions: BTreeMap<String, String>,
    pub csv_json_cell_schemas: BTreeMap<String, String>,
    pub limitations: Vec<String>,
    pub metadata_typedefs: usize,
    pub addressable_type_definitions: usize,
    pub parsed_interface_identities: usize,
    pub eligible_interfaces: usize,
    pub not_addressable: usize,
    pub safe_complete: usize,
    pub safe_incomplete: usize,
    #[serde(rename = "safeEvidence")]
    pub safe_evidence: SafeEvidenceSummary,
    pub targets: BTreeMap<String, TargetCounts>,
    pub blocker_reason_counts: BTreeMap<String, BTreeMap<String, usize>>,
    pub manual_reason_counts: BTreeMap<String, BTreeMap<String, usize>>,
    pub lifecycle_flag_counts: BTreeMap<String, BTreeMap<String, usize>>,
    pub type_categories: Vec<CategoryCounts>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeEvidenceSummary {
    pub safe_complete: usize,
    pub standard_derived: usize,
    pub exact_registry_dependent: usize,
    pub metadata_fact_occurrences: usize,
    pub com_standard_fact_occurrences: usize,
    pub registered_exact_entries: usize,
    pub metadata_matched_exact_entries: usize,
    pub safe_consumed_exact_entries: usize,
    pub exact_entry_interface_dependencies: usize,
    pub exact_family_interface_dependencies: usize,
    pub by_contract_kind: BTreeMap<String, usize>,
    pub by_entry_id: BTreeMap<String, usize>,
    pub by_family_id: BTreeMap<String, usize>,
    pub by_metadata_attribute: BTreeMap<String, usize>,
    pub by_standard_rule_id: BTreeMap<String, usize>,
    pub exact_entry_status: BTreeMap<String, ExactEntryStatus>,
    pub count_semantics: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactEntryStatus {
    pub family_id: String,
    pub contract_kind: String,
    pub selector: crate::contract_registry::ExactEntrySelector,
    pub source_fingerprint: String,
    pub citation: String,
    pub registered: bool,
    pub metadata_matched: bool,
    pub safe_consumed: bool,
    pub interface_dependencies: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct LayoutFact {
    pub architectures: u8,
    pub kind: String,
    pub packing: String,
    pub declared_size: Option<usize>,
    pub field_count: usize,
    pub is_union: bool,
    pub recursive: bool,
    pub x64_size: Option<usize>,
    pub x64_alignment: Option<usize>,
    pub i686_size: Option<usize>,
    pub i686_alignment: Option<usize>,
    pub arm64_size: Option<usize>,
    pub arm64_alignment: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NamedTypeInventory {
    pub identity_key: String,
    pub identity_kind: String,
    pub containing_path: Option<String>,
    pub namespace: String,
    pub name: String,
    pub raw_named_kind: String,
    pub category: String,
    pub direct_occurrences: usize,
    pub expanded_occurrences: usize,
    pub by_value_occurrences: usize,
    pub pointer_occurrences: usize,
    pub max_pointer_depth: usize,
    pub safe_interface_occurrences: usize,
    pub unsafe_interface_occurrences: usize,
    pub layout_facts: Vec<LayoutFact>,
    pub targets: BTreeMap<String, TargetCapability>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TypeShapeInventory {
    pub canonical_signature: String,
    pub category: String,
    pub direct_occurrences: usize,
    pub expanded_occurrences: usize,
    pub uses: BTreeMap<String, usize>,
    pub max_pointer_depth: usize,
    pub constness: Vec<String>,
    pub array_relations: Vec<String>,
    pub named_identities: Vec<String>,
    pub targets: BTreeMap<String, TargetCapability>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetadataDefinitionInventory {
    pub token: u32,
    pub namespace: String,
    pub name: String,
    pub full_name: String,
    pub entity_kind: String,
    pub enclosing_type: Option<String>,
    pub reachable_from_eligible_com_signatures: bool,
}

pub struct CapabilityReport {
    pub summary: CapabilitySummary,
    pub interfaces: Vec<InterfaceCapability>,
    pub named_types: Vec<NamedTypeInventory>,
    pub type_shapes: Vec<TypeShapeInventory>,
    pub all_metadata_definitions: Vec<MetadataDefinitionInventory>,
}

#[derive(Default)]
struct Analysis {
    blockers: BTreeSet<String>,
    manual: BTreeSet<String>,
    lifecycle: LifecycleFlags,
}

impl Analysis {
    fn merge(&mut self, other: Self) {
        self.blockers.extend(other.blockers);
        self.manual.extend(other.manual);
        self.lifecycle.merge(&other.lifecycle);
    }

    fn capability(self) -> TargetCapability {
        let classification = if !self.blockers.is_empty() {
            RawClassification::RawRuntimeBlocked
        } else if !self.manual.is_empty() {
            RawClassification::RawManualContract
        } else {
            RawClassification::RawMetadataComplete
        };
        let blocker_reasons = self.blockers.into_iter().collect::<Vec<_>>();
        TargetCapability {
            classification,
            first_blocker_reason: blocker_reasons.first().cloned(),
            blocker_reasons,
            manual_contract_reasons: self.manual.into_iter().collect(),
            lifecycle: self.lifecycle,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AbiLayout {
    size: usize,
    alignment: usize,
    contains_union: bool,
}

#[derive(Default)]
struct ShapeAccumulator {
    category: String,
    direct_occurrences: usize,
    expanded_occurrences: usize,
    uses: BTreeMap<String, usize>,
    max_pointer_depth: usize,
    constness: BTreeSet<String>,
    array_relations: BTreeSet<String>,
    named_identities: BTreeSet<String>,
    targets: BTreeMap<String, Analysis>,
}

#[derive(Default)]
struct NamedAccumulator {
    identity_key: String,
    identity_kind: String,
    containing_path: Option<String>,
    namespace: String,
    name: String,
    raw_named_kind: String,
    category: String,
    direct_occurrences: usize,
    expanded_occurrences: usize,
    by_value_occurrences: usize,
    pointer_occurrences: usize,
    max_pointer_depth: usize,
    safe_interface_occurrences: usize,
    unsafe_interface_occurrences: usize,
    layout_facts: BTreeSet<LayoutFact>,
    targets: BTreeMap<String, Analysis>,
}

#[derive(Default)]
struct CategoryAccumulator {
    direct_occurrences: usize,
    expanded_occurrences: usize,
    named_types: BTreeSet<String>,
}

pub fn generate_capability_report(
    winmd: &Path,
    output_dir: &Path,
    emit_large_json: bool,
) -> Result<CapabilityReport, String> {
    let bytes =
        fs::read(winmd).map_err(|error| format!("Failed to read {}: {error}", winmd.display()))?;
    let hash = format!("{:X}", Sha256::digest(&bytes));
    if hash != OFFICIAL_METADATA_SHA256 {
        return Err(format!(
            "Windows.Win32.winmd SHA-256 mismatch: expected {OFFICIAL_METADATA_SHA256}, found {hash}"
        ));
    }
    let winmd_text = winmd
        .to_str()
        .ok_or_else(|| "Windows.Win32.winmd path is not valid UTF-8".to_string())?;
    let interfaces =
        crate::com_metadata::parse_all_com_interfaces(winmd_text).ok_or_else(|| {
            format!(
                "Failed to load Classic COM metadata from {}",
                winmd.display()
            )
        })?;
    let parsed_interface_identities = interfaces.len();
    let typedefs = super::typedef_inventory::read_typedefs(winmd)?;
    let addressable_type_definitions = crate::meta::load_index(winmd_text)
        .ok_or_else(|| format!("Failed to load metadata index from {winmd_text}"))?
        .all()
        .count();
    let mut eligible = interfaces
        .into_iter()
        .filter(is_eligible_interface)
        .collect::<Vec<_>>();
    eligible.sort_by(|left, right| interface_identity(left).cmp(&interface_identity(right)));

    let metadata = MetadataIdentity {
        package: "Microsoft.Windows.SDK.Win32Metadata".into(),
        version: OFFICIAL_METADATA_VERSION.into(),
        file: "Windows.Win32.winmd".into(),
        sha256: hash,
        label: format!(
            "Microsoft.Windows.SDK.Win32Metadata {OFFICIAL_METADATA_VERSION}/Windows.Win32.winmd"
        ),
    };
    let report = build_report(
        &eligible,
        winmd_text,
        metadata,
        parsed_interface_identities,
        typedefs,
        addressable_type_definitions,
    )?;
    write_report_files(output_dir, &report, emit_large_json)?;
    Ok(report)
}

fn is_eligible_interface(interface: &ComInterfaceMeta) -> bool {
    (interface.is_iunknown_rooted || interface.interface.name.ends_with("Interop"))
        && !(interface.interface.namespace == "Windows.Win32.UI.Controls.RichEdit"
            && interface.interface.name == "ITextHost2")
}

fn interface_identity(interface: &ComInterfaceMeta) -> String {
    format!(
        "{}.{}",
        interface.interface.namespace, interface.interface.name
    )
}

fn build_report(
    interfaces: &[ComInterfaceMeta],
    winmd: &str,
    metadata: MetadataIdentity,
    parsed_interface_identities: usize,
    typedefs: Vec<super::typedef_inventory::TypeDefRecord>,
    addressable_type_definitions: usize,
) -> Result<CapabilityReport, String> {
    let validate_embedded_registry_usage = metadata.sha256 == OFFICIAL_METADATA_SHA256;
    let mut details = Vec::with_capacity(interfaces.len());
    let mut shapes = BTreeMap::<String, ShapeAccumulator>::new();
    let mut named = BTreeMap::<String, NamedAccumulator>::new();
    let mut categories = BTreeMap::<String, CategoryAccumulator>::new();
    let mut metadata_matched_entry_ids = BTreeSet::new();
    let mut registered_entry_records = Vec::new();

    for interface in interfaces {
        let raw_dependencies = crate::com_metadata::collect_evidence_dependencies(interface);
        metadata_matched_entry_ids.extend(raw_dependencies.exact_entry_ids);
        registered_entry_records.extend(crate::com_metadata::collect_exact_registry_entries(
            interface,
        ));
        let safe_result = generate_com_interface_files(interface, winmd);
        let safe_complete = safe_result.is_ok();
        let safe_error = safe_result
            .err()
            .map(|error| normalize_safe_error(&error, winmd));
        let evidence_dependencies = if safe_complete {
            super::project::project_com_interface(interface, winmd)
                .map_err(|error| {
                    format!(
                        "Safe-complete interface {}.{} failed evidence validation: {error}",
                        interface.interface.namespace, interface.interface.name
                    )
                })?
                .evidence_dependencies
                .clone()
        } else {
            crate::contract_registry::EvidenceDependencies::default()
        };
        evidence_dependencies.validate_exact_ids()?;
        metadata_matched_entry_ids.extend(evidence_dependencies.exact_entry_ids.iter().cloned());
        if evidence_dependencies
            .exact_family_ids
            .contains(&crate::contract_registry::ExactFamilyId::DispatchInvoke)
        {
            let method = interface
                .raw_methods
                .as_deref()
                .unwrap_or_default()
                .iter()
                .find(|method| {
                    method.declaring_namespace == "Windows.Win32.System.Com"
                        && method.declaring_interface == "IDispatch"
                        && method
                            .declaring_iid
                            .eq_ignore_ascii_case("00020400-0000-0000-c000-000000000046")
                        && method.metadata_name == "Invoke"
                        && method.vtable_index == 6
                })
                .ok_or_else(|| {
                    "Projected IDispatch Invoke evidence no longer matches metadata".to_string()
                })?;
            let family_id = crate::contract_registry::ExactFamilyId::DispatchInvoke;
            registered_entry_records.push(crate::contract_registry::ExactRegistryEntry {
                entry_id: crate::contract_registry::exact_method_entry_id(
                    family_id,
                    &method.declaring_namespace,
                    &method.declaring_interface,
                    &method.declaring_iid,
                    &method.metadata_name,
                    method.vtable_index,
                ),
                family_id,
                contract_kind: crate::contract_registry::ContractKind::CompoundDispatch,
                selector: crate::contract_registry::ExactEntrySelector {
                    namespace: method.declaring_namespace.clone(),
                    interface: method.declaring_interface.clone(),
                    iid: method.declaring_iid.clone(),
                    method: method.metadata_name.clone(),
                    slot: method.vtable_index,
                    parameter: None,
                },
                source_fingerprint: crate::com_metadata::raw_method_fingerprint(method),
                reason: "IDispatch::Invoke has a documented compound Automation call/result contract not represented by WinMD".into(),
                citation: "https://learn.microsoft.com/windows/win32/api/oaidl/nf-oaidl-idispatch-invoke".into(),
            });
        }
        let safe_cleanup = safe_complete
            .then(|| super::safe_interface_cleanup_availability(interface, winmd))
            .transpose()?;
        let mut targets = BTreeMap::new();
        for target in CensusTarget::ALL {
            let analyzed = analyze_interface(interface, target);
            let capability = if safe_complete {
                let mut lifecycle = analyzed.lifecycle;
                lifecycle.cleanup = match safe_cleanup.expect("safe cleanup was projected") {
                    super::SafeCleanupAvailability::NoneRequired => {
                        CleanupAvailability::NoneRequired
                    }
                    super::SafeCleanupAvailability::StandardSupported => {
                        CleanupAvailability::StandardSupported
                    }
                };
                TargetCapability {
                    classification: RawClassification::RawMetadataComplete,
                    first_blocker_reason: None,
                    blocker_reasons: Vec::new(),
                    manual_contract_reasons: Vec::new(),
                    lifecycle,
                }
            } else {
                analyzed
            };
            targets.insert(target.key().to_string(), capability);
        }
        collect_interface_types(
            interface,
            safe_complete,
            &mut shapes,
            &mut named,
            &mut categories,
        );
        let raw = interface.raw_methods.as_deref().unwrap_or_default();
        details.push(InterfaceCapability {
            namespace: interface.interface.namespace.clone(),
            name: interface.interface.name.clone(),
            iid: interface.interface.iid.clone(),
            is_iunknown_rooted: interface.is_iunknown_rooted,
            method_count: raw.len(),
            first_vtable_slot: raw.first().map(|method| method.vtable_index),
            last_vtable_slot: raw.last().map(|method| method.vtable_index),
            safe_complete,
            safe_error,
            evidence_class: safe_complete.then_some(
                if evidence_dependencies.exact_entry_ids.is_empty() {
                    SafeEvidenceClass::StandardDerived
                } else {
                    SafeEvidenceClass::ExactRegistryDependent
                },
            ),
            standard_rule_ids: evidence_dependencies
                .standard_rule_ids
                .into_iter()
                .collect(),
            exact_entry_ids: evidence_dependencies.exact_entry_ids.into_iter().collect(),
            exact_family_ids: evidence_dependencies
                .exact_family_ids
                .into_iter()
                .map(|family| family.id().into())
                .collect(),
            exact_contract_kinds: evidence_dependencies
                .exact_contract_kinds
                .into_iter()
                .map(|kind| kind.key().into())
                .collect(),
            exact_entry_kinds: evidence_dependencies
                .exact_entry_kinds
                .into_iter()
                .map(|(entry_id, kind)| (entry_id, kind.key().into()))
                .collect(),
            metadata_attributes: evidence_dependencies
                .metadata_attributes
                .into_iter()
                .collect(),
            targets,
        });
    }
    let exact_catalog =
        crate::contract_registry::validate_exact_entry_catalog(registered_entry_records)?;
    let registered_entry_ids = exact_catalog.keys().cloned().collect::<BTreeSet<_>>();
    let statically_declared_entry_ids =
        crate::contract_registry::statically_declared_exact_entry_ids()?;
    let stale_entries = statically_declared_entry_ids
        .difference(&metadata_matched_entry_ids)
        .cloned()
        .collect::<Vec<_>>();
    let unregistered_entries = metadata_matched_entry_ids
        .difference(&statically_declared_entry_ids)
        .cloned()
        .collect::<Vec<_>>();
    if !stale_entries.is_empty() || !unregistered_entries.is_empty() {
        return Err(format!(
            "Exact registry declaration/metadata mismatch: unmatched={stale_entries:?}, unregistered={unregistered_entries:?}"
        ));
    }
    if registered_entry_ids != metadata_matched_entry_ids {
        let unmatched = registered_entry_ids
            .difference(&metadata_matched_entry_ids)
            .cloned()
            .collect::<Vec<_>>();
        let unregistered = metadata_matched_entry_ids
            .difference(&registered_entry_ids)
            .cloned()
            .collect::<Vec<_>>();
        return Err(format!(
            "Exact registry catalog/metadata mismatch: unmatched={unmatched:?}, unregistered={unregistered:?}"
        ));
    }
    if validate_embedded_registry_usage {
        crate::contract_registry::validate_registry_usage(&metadata_matched_entry_ids)?;
    }

    let type_shapes = shapes
        .into_iter()
        .map(|(canonical_signature, accumulator)| TypeShapeInventory {
            canonical_signature,
            category: accumulator.category,
            direct_occurrences: accumulator.direct_occurrences,
            expanded_occurrences: accumulator.expanded_occurrences,
            uses: accumulator.uses,
            max_pointer_depth: accumulator.max_pointer_depth,
            constness: accumulator.constness.into_iter().collect(),
            array_relations: accumulator.array_relations.into_iter().collect(),
            named_identities: accumulator.named_identities.into_iter().collect(),
            targets: finalize_target_analyses(accumulator.targets),
        })
        .collect::<Vec<_>>();
    let mut named_types = named
        .into_values()
        .map(|accumulator| NamedTypeInventory {
            identity_key: accumulator.identity_key,
            identity_kind: accumulator.identity_kind,
            containing_path: accumulator.containing_path,
            namespace: accumulator.namespace,
            name: accumulator.name,
            raw_named_kind: accumulator.raw_named_kind,
            category: accumulator.category,
            direct_occurrences: accumulator.direct_occurrences,
            expanded_occurrences: accumulator.expanded_occurrences,
            by_value_occurrences: accumulator.by_value_occurrences,
            pointer_occurrences: accumulator.pointer_occurrences,
            max_pointer_depth: accumulator.max_pointer_depth,
            safe_interface_occurrences: accumulator.safe_interface_occurrences,
            unsafe_interface_occurrences: accumulator.unsafe_interface_occurrences,
            layout_facts: accumulator.layout_facts.into_iter().collect(),
            targets: finalize_target_analyses(accumulator.targets),
        })
        .collect::<Vec<_>>();
    let metadata_definition_names = typedefs
        .iter()
        .map(typedef_full_name)
        .collect::<BTreeSet<_>>();
    for value in &mut named_types {
        if value.identity_kind == "named_metadata_definition"
            && !metadata_definition_names.contains(&value.identity_key)
        {
            value.identity_kind = "external_metadata_reference".into();
        }
    }
    let reachable_qualified = named_types
        .iter()
        .filter(|value| value.identity_kind == "named_metadata_definition")
        .map(|value| value.identity_key.clone())
        .collect::<BTreeSet<_>>();
    let reachable_anonymous = named_types
        .iter()
        .filter(|value| value.identity_kind == "anonymous_nested_record")
        .filter_map(|value| {
            value
                .containing_path
                .as_ref()
                .map(|containing| (containing.clone(), value.name.clone()))
        })
        .collect::<BTreeSet<_>>();
    let all_metadata_definitions = typedefs
        .into_iter()
        .map(|definition| {
            let full_name = typedef_full_name(&definition);
            let reachable_from_eligible_com_signatures = reachable_qualified.contains(&full_name)
                || definition.enclosing_type.as_ref().is_some_and(|enclosing| {
                    reachable_anonymous.contains(&(enclosing.clone(), definition.name.clone()))
                });
            MetadataDefinitionInventory {
                token: definition.token,
                namespace: definition.namespace,
                name: definition.name,
                full_name,
                entity_kind: definition.entity_kind,
                enclosing_type: definition.enclosing_type,
                reachable_from_eligible_com_signatures,
            }
        })
        .collect::<Vec<_>>();

    let type_categories = CATEGORIES
        .into_iter()
        .map(|category| {
            let accumulator = categories.remove(category).unwrap_or_default();
            CategoryCounts {
                category: category.into(),
                direct_occurrences: accumulator.direct_occurrences,
                expanded_occurrences: accumulator.expanded_occurrences,
                named_types: accumulator.named_types.len(),
            }
        })
        .collect::<Vec<_>>();
    let metadata_typedefs = all_metadata_definitions.len();
    let summary = summarize(
        metadata,
        &details,
        type_categories,
        parsed_interface_identities,
        metadata_typedefs,
        addressable_type_definitions,
        statically_declared_entry_ids.len(),
        metadata_matched_entry_ids.len(),
        &exact_catalog,
        &metadata_matched_entry_ids,
    );
    Ok(CapabilityReport {
        summary,
        interfaces: details,
        named_types,
        type_shapes,
        all_metadata_definitions,
    })
}

fn typedef_full_name(definition: &super::typedef_inventory::TypeDefRecord) -> String {
    definition.enclosing_type.as_ref().map_or_else(
        || {
            if definition.namespace.is_empty() {
                definition.name.clone()
            } else {
                format!("{}.{}", definition.namespace, definition.name)
            }
        },
        |enclosing| format!("{enclosing}+{}", definition.name),
    )
}

fn normalize_safe_error(error: &str, winmd: &str) -> String {
    let escaped_winmd = winmd.replace('\\', r"\\");
    error
        .replace(&escaped_winmd, "<Windows.Win32.winmd>")
        .replace(winmd, "<Windows.Win32.winmd>")
        .replace(
            r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\<version>\Windows.winmd",
            "<Windows SDK UnionMetadata>/Windows.winmd",
        )
}

fn finalize_target_analyses(
    mut analyses: BTreeMap<String, Analysis>,
) -> BTreeMap<String, TargetCapability> {
    CensusTarget::ALL
        .into_iter()
        .map(|target| {
            let capability = analyses
                .remove(target.key())
                .unwrap_or_default()
                .capability();
            (target.key().to_string(), capability)
        })
        .collect()
}

fn analyze_interface(interface: &ComInterfaceMeta, target: CensusTarget) -> TargetCapability {
    let mut analysis = analyze_interface_identity(interface);
    let Some(methods) = interface.raw_methods.as_deref() else {
        analysis.blockers.insert("missing_raw_methods".into());
        return analysis.capability();
    };
    if methods.len() != interface.interface.methods.len() {
        analysis.blockers.insert("incomplete_raw_vtable".into());
    }
    for (index, method) in methods.iter().enumerate() {
        if method.vtable_index != interface.base_offset + index {
            analysis.blockers.insert("invalid_vtable_slot".into());
        }
        analysis.merge(analyze_method(method, target));
    }
    analysis.capability()
}

fn analyze_method(method: &RawComMethod, target: CensusTarget) -> Analysis {
    let mut analysis = analyze_type(
        &method.return_type,
        target,
        TypeUse::Return,
        &mut Vec::new(),
    );
    if category_for(&method.return_type, false) == "DataPointer"
        && effective_pointer_depth(&method.return_type) > 0
    {
        if method.exact_contract.as_ref().is_some_and(|contract| {
            contract.kind == crate::com_metadata::RawExactMethodContractKind::Malloc
        }) {
            analysis.manual.insert("allocator_interface_cleanup".into());
            analysis.lifecycle.cleanup = CleanupAvailability::KnownExternal;
        } else {
            analysis.manual.insert("missing_output_ownership".into());
            analysis.manual.insert("missing_allocator".into());
            analysis.lifecycle.cleanup = CleanupAvailability::Unknown;
        }
    }
    if category_for(&method.return_type, false) == "Handle" {
        analysis.manual.insert("missing_handle_ownership".into());
        analysis.lifecycle.cleanup = CleanupAvailability::Unknown;
    }
    if !is_nonzero_iid(&method.declaring_iid) {
        analysis.blockers.insert("missing_declaring_iid".into());
    }
    for (param_index, param) in method.params.iter().enumerate() {
        analysis.merge(analyze_parameter(method, param_index, param, target));
    }
    analysis
}

fn analyze_parameter(
    method: &RawComMethod,
    param_index: usize,
    param: &RawComParam,
    target: CensusTarget,
) -> Analysis {
    let usage = match param.direction {
        RawParamDirection::In => TypeUse::In,
        RawParamDirection::Out => TypeUse::Out,
        RawParamDirection::InOut => TypeUse::InOut,
    };
    let mut parameter = analyze_type(&param.typ, target, usage, &mut Vec::new());
    if param.typ.constness == RawConstness::Mixed {
        parameter.manual.insert("mixed_pointer_constness".into());
    }
    if let Some(array) = &param.native_array {
        if array.count_param_index.is_none() {
            parameter.manual.insert("missing_count_relation".into());
        }
    } else if matches!(param.typ.native_type, RawNativeType::Array(_)) {
        parameter.manual.insert("missing_count_relation".into());
    }
    if let Some(string_array) = &param.string_pointer_array {
        if string_array.encoding == RawStringEncoding::Unknown {
            parameter.manual.insert("unknown_string_encoding".into());
        }
    }
    if let Some(safe_array) = &param.safe_array_evidence {
        if safe_array.element_vartype == crate::com_metadata::RawSafeArrayVartype::Variant {
            parameter
                .manual
                .insert("variant_safearray_element_contract".into());
        }
        if safe_array.ownership == RawSafeArrayOwnership::OwnedOutput {
            parameter.lifecycle.cleanup = CleanupAvailability::StandardSupported;
        }
    }
    let borrowed_handle =
        crate::com_metadata::is_registered_borrowed_hwnd_output(method, param_index);
    let handle_output = matches!(
        param.direction,
        RawParamDirection::Out | RawParamDirection::InOut
    ) && category_for(&param.typ, false) == "Handle";
    let data_pointer = category_for(&param.typ, false) == "DataPointer";
    let opaque_output = matches!(
        param.direction,
        RawParamDirection::Out | RawParamDirection::InOut
    ) && data_pointer;
    let typed_interface_replacement = param.direction == RawParamDirection::InOut
        && category_for(&param.typ, false) == "ComInterface"
        && param.typ.pointer_depth == 1
        && method
            .interface_replacement_contracts
            .iter()
            .filter(|contract| contract.parameter_index == param_index)
            .count()
            != 1;
    let pointer_slot_output = opaque_output && effective_pointer_depth(&param.typ) >= 2;
    let exact_caller_buffer = param
        .native_array
        .as_ref()
        .is_some_and(|relation| relation.count_param_index.is_some())
        || method.exact_contract.as_ref().is_some_and(|contract| {
            contract.buffer_param_index == param_index
                && matches!(
                    contract.kind,
                    crate::com_metadata::RawExactMethodContractKind::FixedCapacityBytes
                        | crate::com_metadata::RawExactMethodContractKind::UnsafePrivateData
                )
        });
    if let Some(free_with) = &param.free_with {
        parameter.lifecycle.cleanup = if STANDARD_CLEANUPS
            .iter()
            .any(|cleanup| *cleanup == free_with.function)
        {
            CleanupAvailability::StandardSupported
        } else {
            CleanupAvailability::KnownExternal
        };
    } else if handle_output && !borrowed_handle {
        parameter.manual.insert("missing_handle_ownership".into());
        parameter.lifecycle.cleanup = CleanupAvailability::Unknown;
    } else if typed_interface_replacement {
        parameter
            .manual
            .insert("missing_interface_replacement_contract".into());
    } else if pointer_slot_output {
        parameter.manual.insert("missing_output_ownership".into());
        parameter.manual.insert("missing_allocator".into());
        parameter.lifecycle.cleanup = CleanupAvailability::Unknown;
    } else if opaque_output && !exact_caller_buffer {
        parameter.manual.insert("opaque_pointee_contract".into());
        parameter.manual.insert("external_pointee_storage".into());
        parameter.lifecycle.requires_external_pointer_or_callback = true;
    } else if data_pointer
        && matches!(param.direction, RawParamDirection::In)
        && !exact_caller_buffer
    {
        parameter.manual.insert("external_pointee_storage".into());
        parameter.lifecycle.requires_external_pointer_or_callback = true;
    } else if matches!(
        param.direction,
        RawParamDirection::Out | RawParamDirection::InOut
    ) && pointer_needs_manual_output_contract(&param.typ)
    {
        parameter.manual.insert("missing_output_ownership".into());
        parameter.manual.insert("missing_allocator".into());
        parameter.lifecycle.cleanup = CleanupAvailability::Unknown;
    }
    if output_has_standard_cleanup(param) {
        parameter.lifecycle.cleanup = parameter
            .lifecycle
            .cleanup
            .max(CleanupAvailability::StandardSupported);
    }
    parameter
}

pub fn parameter_manual_reasons(
    method: &RawComMethod,
    param_index: usize,
) -> Result<Vec<String>, String> {
    let param = method.params.get(param_index).ok_or_else(|| {
        format!(
            "{} parameter index {param_index} is out of range",
            method.metadata_name
        )
    })?;
    Ok(CensusTarget::ALL
        .into_iter()
        .flat_map(|target| {
            analyze_parameter(method, param_index, param, target)
                .manual
                .into_iter()
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RawPointeeLayout {
    pub size: usize,
    pub alignment: usize,
}

pub(crate) fn parameter_pointee_layouts(
    param: &RawComParam,
) -> BTreeMap<String, Option<RawPointeeLayout>> {
    let pointee = dereferenced_type(&param.typ);
    CensusTarget::ALL
        .into_iter()
        .map(|target| {
            let layout = pointee.as_ref().and_then(|pointee| {
                if pointee.pointer_depth == 0
                    && matches!(
                        &pointee.native_type,
                        RawNativeType::Named {
                            kind: RawNamedKind::Interface | RawNamedKind::RuntimeClass,
                            ..
                        }
                    )
                {
                    return Some(RawPointeeLayout {
                        size: target.pointer_size(),
                        alignment: target.pointer_size(),
                    });
                }
                let (layout, analysis) = abi_layout(pointee, target, &mut Vec::new());
                if analysis.blockers.is_empty() {
                    layout.map(|layout| RawPointeeLayout {
                        size: layout.size,
                        alignment: layout.alignment,
                    })
                } else {
                    None
                }
            });
            (target.key().into(), layout)
        })
        .collect()
}

fn dereferenced_type(typ: &RawComType) -> Option<RawComType> {
    if typ.pointer_depth > 0 {
        let mut pointee = typ.clone();
        pointee.pointer_depth -= 1;
        return Some(pointee);
    }
    typ.underlying.as_deref().and_then(dereferenced_type)
}

pub fn classify_interface_methods(
    interface: &ComInterfaceMeta,
) -> Result<Vec<MethodCapability>, String> {
    let methods = interface.raw_methods.as_ref().ok_or_else(|| {
        format!(
            "{}.{} has no complete raw method metadata",
            interface.interface.namespace, interface.interface.name
        )
    })?;
    if methods.len() != interface.interface.methods.len() {
        return Err(format!(
            "{}.{} raw method count does not match its inherited compatibility vtable",
            interface.interface.namespace, interface.interface.name
        ));
    }
    methods
        .iter()
        .enumerate()
        .map(|(index, method)| {
            if method.vtable_index != interface.base_offset + index {
                return Err(format!(
                    "{}.{} has non-contiguous raw slot {}",
                    interface.interface.namespace, method.metadata_name, method.vtable_index
                ));
            }
            let canonical = canonical_method(method);
            Ok(MethodCapability {
                name: method.metadata_name.clone(),
                projected_name: method.projected_name.clone(),
                declaring_iid: method.declaring_iid.clone(),
                absolute_slot: method.vtable_index,
                signature_fingerprint: format!("{:X}", Sha256::digest(canonical.as_bytes())),
                targets: CensusTarget::ALL
                    .into_iter()
                    .map(|target| {
                        let mut analysis = analyze_interface_identity(interface);
                        analysis.merge(analyze_method(method, target));
                        (target.key().into(), analysis.capability())
                    })
                    .collect(),
            })
        })
        .collect()
}

fn analyze_interface_identity(interface: &ComInterfaceMeta) -> Analysis {
    let mut analysis = Analysis::default();
    analysis.lifecycle.requires_external_acquisition = interface.coclass_clsid.is_none();
    analysis.lifecycle.requires_current_apartment = true;
    if !is_nonzero_iid(&interface.interface.iid) {
        analysis.blockers.insert("missing_interface_iid".into());
    }
    let supported_root = (interface.is_iunknown_rooted && interface.base_offset == 3)
        || (!interface.is_iunknown_rooted
            && interface.interface.name.ends_with("Interop")
            && interface.base_offset == 6);
    if !supported_root {
        analysis.blockers.insert("missing_interface_root".into());
    }
    if !is_eligible_interface(interface) {
        analysis.blockers.insert("not_addressable".into());
    }
    analysis
}

fn is_nonzero_iid(value: &str) -> bool {
    let value = value.trim();
    let value = match (value.strip_prefix('{'), value.strip_suffix('}')) {
        (Some(without_open), Some(_)) => &without_open[..without_open.len().saturating_sub(1)],
        (None, None) => value,
        _ => return false,
    };
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
        && bytes
            .iter()
            .any(|byte| byte.is_ascii_hexdigit() && *byte != b'0')
}

fn canonical_method(method: &RawComMethod) -> String {
    crate::com_metadata::canonical_raw_method(method)
}

#[derive(Debug, Clone, Copy)]
enum TypeUse {
    In,
    Out,
    InOut,
    Return,
    Nested,
}

impl TypeUse {
    const fn key(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
            Self::InOut => "inout",
            Self::Return => "return",
            Self::Nested => "nested",
        }
    }
}

fn analyze_type(
    typ: &RawComType,
    target: CensusTarget,
    usage: TypeUse,
    stack: &mut Vec<String>,
) -> Analysis {
    let mut analysis = Analysis::default();
    let known_named_abi = matches!(
        &typ.native_type,
        RawNativeType::Named {
            namespace,
            name,
            ..
        } if is_known_named_abi_type(namespace, name)
    );
    if typ.pointer_depth > 0 {
        match &typ.native_type {
            RawNativeType::Named {
                kind: RawNamedKind::Interface | RawNamedKind::RuntimeClass,
                iid,
                ..
            } if iid.as_deref().is_none_or(|iid| !is_nonzero_iid(iid)) => {
                analysis.blockers.insert("missing_interface_iid".into());
            }
            RawNativeType::Named {
                kind: RawNamedKind::Delegate,
                ..
            } => analysis.lifecycle.requires_external_pointer_or_callback = true,
            RawNativeType::Named {
                namespace,
                name,
                kind: RawNamedKind::Struct,
                layout,
                ..
            } if !is_pointer_sized_semantic_type(namespace, name)
                && !is_special_abi_compound(namespace, name) =>
            {
                let identity = format!("{namespace}.{name}");
                if stack.contains(&identity) {
                    classify_incomplete_pointee(&mut analysis, typ, usage, "recursive_layout");
                    return analysis;
                }
                if let Some(layout) = layout.as_deref() {
                    stack.push(identity);
                    analysis.merge(analyze_pointer_layout(typ, layout, target, usage, stack));
                    stack.pop();
                } else if typ
                    .underlying
                    .as_deref()
                    .and_then(|underlying| scalar_layout(underlying, target))
                    .is_none()
                {
                    classify_incomplete_pointee(&mut analysis, typ, usage, "missing_layout");
                }
            }
            RawNativeType::Named {
                kind: RawNamedKind::Unknown,
                ..
            } if !known_named_abi => {
                classify_incomplete_pointee(&mut analysis, typ, usage, "unknown_native_type");
            }
            RawNativeType::Unknown(_) => {
                classify_incomplete_pointee(&mut analysis, typ, usage, "unknown_native_type");
            }
            RawNativeType::Array(element) | RawNativeType::FixedArray { element, .. } => {
                analysis.merge(analyze_type(element, target, TypeUse::Nested, stack));
            }
            _ => {}
        }
        if matches!(usage, TypeUse::Nested) {
            analysis.manual.insert("nested_pointer_lifetime".into());
        }
        return analysis;
    }
    match &typ.native_type {
        RawNativeType::Void
        | RawNativeType::Bool
        | RawNativeType::I8
        | RawNativeType::U8
        | RawNativeType::I16
        | RawNativeType::U16
        | RawNativeType::I32
        | RawNativeType::U32
        | RawNativeType::I64
        | RawNativeType::U64
        | RawNativeType::F32
        | RawNativeType::F64
        | RawNativeType::Char16
        | RawNativeType::ISize
        | RawNativeType::USize
        | RawNativeType::String
        | RawNativeType::Object => {}
        RawNativeType::Array(element) => {
            analysis.manual.insert("missing_count_relation".into());
            analysis.merge(analyze_type(element, target, TypeUse::Nested, stack));
        }

        RawNativeType::FixedArray { .. } => {
            if !matches!(usage, TypeUse::Nested) {
                analysis
                    .blockers
                    .insert("unsupported_top_level_fixed_array".into());
            }
        }
        RawNativeType::Unknown(_) => {
            analysis.blockers.insert("unknown_native_type".into());
        }
        RawNativeType::Named {
            namespace,
            name,
            kind,
            iid,
            layout,
        } => {
            let identity = format!("{namespace}.{name}");
            match kind {
                RawNamedKind::Interface | RawNamedKind::RuntimeClass => {
                    if iid.as_deref().is_none_or(|iid| !is_nonzero_iid(iid)) {
                        analysis.blockers.insert("missing_interface_iid".into());
                    }
                }
                RawNamedKind::Delegate => {
                    analysis.lifecycle.requires_external_pointer_or_callback = true;
                }
                RawNamedKind::Enum => {
                    if typ
                        .underlying
                        .as_deref()
                        .and_then(|underlying| scalar_layout(underlying, target))
                        .is_none()
                    {
                        analysis.blockers.insert("missing_enum_underlying".into());
                    }
                }
                RawNamedKind::Struct => {
                    if is_pointer_sized_semantic_type(namespace, name) {
                        return analysis;
                    }
                    if is_special_abi_compound(namespace, name) {
                        return analysis;
                    }
                    if let Some(underlying) = typ.underlying.as_deref()
                        && scalar_layout(underlying, target).is_some()
                    {
                        return analysis;
                    }
                    let Some(layout) = layout.as_deref() else {
                        analysis.blockers.insert("missing_layout".into());
                        return analysis;
                    };
                    if stack.contains(&identity) || layout.recursive {
                        analysis.blockers.insert("recursive_layout".into());
                        return analysis;
                    }
                    stack.push(identity);
                    let (_, nested) = analyze_layout_set(layout, target, stack, true);
                    stack.pop();
                    analysis.merge(nested);
                }
                RawNamedKind::Unknown => {
                    if known_named_abi {
                        // Known system/native ABI identities are authoritative
                        // even when Windows metadata does not assign a named kind.
                    } else if let Some(underlying) = typ.underlying.as_deref() {
                        analysis.merge(analyze_type(underlying, target, usage, stack));
                    } else {
                        analysis.blockers.insert("unknown_native_type".into());
                    }
                }
            }
        }
    }
    analysis
}

fn is_known_named_abi_type(namespace: &str, name: &str) -> bool {
    (namespace == "System" && matches!(name, "Guid" | "IntPtr" | "UIntPtr"))
        || (namespace == "Windows.Win32.Foundation"
            && matches!(
                name,
                "BOOL"
                    | "HRESULT"
                    | "BSTR"
                    | "PWSTR"
                    | "PCWSTR"
                    | "LPWSTR"
                    | "LPCWSTR"
                    | "PSTR"
                    | "PCSTR"
                    | "LPSTR"
                    | "LPCSTR"
                    | "PVOID"
                    | "PCVOID"
                    | "LPVOID"
                    | "LPCVOID"
            ))
        || (namespace == "Windows.Win32.System.WinRT" && name == "HSTRING")
        || (namespace == "Windows.Win32.System.Variant" && matches!(name, "VARIANT" | "VARIANTARG"))
        || (namespace == "Windows.Win32.System.Com"
            && matches!(name, "SAFEARRAY" | "DISPPARAMS" | "EXCEPINFO" | "STATSTG"))
        || (namespace == "Windows.Win32.System.Com.StructuredStorage" && name == "PROPVARIANT")
}

fn analyze_pointer_layout(
    typ: &RawComType,
    layout: &RawNativeLayoutSet,
    target: CensusTarget,
    usage: TypeUse,
    stack: &mut Vec<String>,
) -> Analysis {
    let mut analysis = Analysis::default();
    let candidates = layout
        .variants
        .iter()
        .filter(|variant| variant.architectures & target.metadata_mask() != 0)
        .collect::<Vec<_>>();
    let [variant] = candidates.as_slice() else {
        classify_incomplete_pointee(
            &mut analysis,
            typ,
            usage,
            if candidates.is_empty() {
                "missing_target_layout"
            } else {
                "ambiguous_target_layout"
            },
        );
        return analysis;
    };
    let (_, layout_analysis) = analyze_layout(variant, target, stack, false);
    if !layout_analysis.blockers.is_empty() {
        for reason in layout_analysis.blockers {
            classify_incomplete_pointee(&mut analysis, typ, usage, &reason);
        }
        return analysis;
    }
    for field in &variant.fields {
        analysis.merge(analyze_type(&field.typ, target, TypeUse::Nested, stack));
    }
    analysis
}

fn classify_incomplete_pointee(
    analysis: &mut Analysis,
    typ: &RawComType,
    usage: TypeUse,
    detail: &str,
) {
    if matches!(usage, TypeUse::Out | TypeUse::InOut) && typ.pointer_depth == 1 {
        analysis
            .blockers
            .insert("incomplete_pointee_layout_for_storage".into());
        analysis.blockers.insert(format!("pointee_{detail}"));
    } else {
        analysis.manual.insert("external_pointee_storage".into());
        analysis.manual.insert(format!("pointee_{detail}"));
        analysis.lifecycle.requires_external_pointer_or_callback = true;
    }
}

fn analyze_layout_set(
    set: &RawNativeLayoutSet,
    target: CensusTarget,
    stack: &mut Vec<String>,
    top_level: bool,
) -> (Option<AbiLayout>, Analysis) {
    let mut analysis = Analysis::default();
    if set.recursive {
        analysis.blockers.insert("recursive_layout".into());
        return (None, analysis);
    }
    let candidates = set
        .variants
        .iter()
        .filter(|variant| variant.architectures & target.metadata_mask() != 0)
        .collect::<Vec<_>>();
    let [layout] = candidates.as_slice() else {
        analysis.blockers.insert(
            if candidates.is_empty() {
                "missing_target_layout"
            } else {
                "ambiguous_target_layout"
            }
            .into(),
        );
        return (None, analysis);
    };
    analyze_layout(layout, target, stack, top_level)
}

fn analyze_layout(
    raw: &RawNativeLayout,
    target: CensusTarget,
    stack: &mut Vec<String>,
    top_level: bool,
) -> (Option<AbiLayout>, Analysis) {
    let mut analysis = Analysis::default();
    if raw.packing != RawPacking::Default {
        analysis.blockers.insert("unsupported_packing".into());
    }
    if raw.fields.is_empty() {
        analysis.blockers.insert("incomplete_layout".into());
    }
    if raw.fields.iter().any(|field| field.bitfield) {
        analysis.blockers.insert("unsupported_bitfield".into());
    }
    if raw.fields.iter().any(|field| field.flexible_array) {
        analysis
            .blockers
            .insert("unsupported_flexible_array".into());
    }
    if raw.kind == RawLayoutKind::Unknown {
        analysis.blockers.insert("unsupported_layout_kind".into());
    }

    let mut field_layouts = Vec::with_capacity(raw.fields.len());
    for field in &raw.fields {
        let (layout, nested) = analyze_field_layout(field, target, stack);
        analysis.merge(nested);
        if let Some(layout) = layout {
            field_layouts.push((field, layout));
        }
    }
    if !analysis.blockers.is_empty() || field_layouts.len() != raw.fields.len() {
        return (None, analysis);
    }

    let computed = if raw.is_union {
        let maximum_alignment = field_layouts
            .iter()
            .map(|(_, field)| field.alignment)
            .max()
            .unwrap_or(1);
        let maximum_size = field_layouts
            .iter()
            .map(|(_, field)| field.size)
            .max()
            .unwrap_or(0);
        checked_align_up(maximum_size, maximum_alignment).map(|size| AbiLayout {
            size,
            alignment: maximum_alignment,
            contains_union: true,
        })
    } else {
        let mut ordered = field_layouts;
        ordered.sort_by_key(|(field, _)| field.explicit_offset.unwrap_or(usize::MAX));
        let mut end = 0usize;
        let mut maximum_alignment = 1usize;
        let mut valid = true;
        for (field, field_layout) in ordered {
            let expected = match checked_align_up(end, field_layout.alignment) {
                Some(offset) => offset,
                None => {
                    valid = false;
                    break;
                }
            };
            let offset = match raw.kind {
                RawLayoutKind::Sequential => expected,
                RawLayoutKind::Explicit => field.explicit_offset.unwrap_or(usize::MAX),
                RawLayoutKind::Unknown => usize::MAX,
            };
            if offset != expected {
                valid = false;
                break;
            }
            let Some(next) = offset.checked_add(field_layout.size) else {
                valid = false;
                break;
            };
            end = next;
            maximum_alignment = maximum_alignment.max(field_layout.alignment);
        }
        if valid {
            checked_align_up(end, maximum_alignment).map(|size| AbiLayout {
                size,
                alignment: maximum_alignment,
                contains_union: field_layouts_contains_union(&raw.fields, target, stack),
            })
        } else {
            None
        }
    };
    let Some(computed) = computed else {
        analysis.blockers.insert("non_natural_struct_layout".into());
        return (None, analysis);
    };
    if let Some(declared_size) = raw.declared_size
        && declared_size != computed.size
    {
        analysis.blockers.insert(
            if raw.is_union {
                "incomplete_union_layout"
            } else {
                "non_natural_struct_layout"
            }
            .into(),
        );
    }
    if top_level && target == CensusTarget::X64 && matches!(computed.size, 3 | 5 | 6 | 7) {
        analysis
            .blockers
            .insert("win64_irregular_aggregate_copy".into());
    }
    if top_level && target == CensusTarget::Arm64 && computed.contains_union {
        analysis.blockers.insert("arm64_union_by_value_gate".into());
    }
    if analysis.blockers.is_empty() {
        (Some(computed), analysis)
    } else {
        (None, analysis)
    }
}

fn field_layouts_contains_union(
    fields: &[RawNativeField],
    target: CensusTarget,
    stack: &mut Vec<String>,
) -> bool {
    fields.iter().any(|field| {
        let RawNativeType::Named {
            layout: Some(layout),
            ..
        } = &field.typ.native_type
        else {
            return false;
        };
        analyze_layout_set(layout, target, stack, false)
            .0
            .is_some_and(|layout| layout.contains_union)
    })
}

fn analyze_field_layout(
    field: &RawNativeField,
    target: CensusTarget,
    stack: &mut Vec<String>,
) -> (Option<AbiLayout>, Analysis) {
    let count = field.fixed_count.unwrap_or(1);
    let (element, analysis) = abi_layout(&field.typ, target, stack);
    let Some(element) = element else {
        return (None, analysis);
    };
    let Some(size) = element.size.checked_mul(count) else {
        let mut analysis = analysis;
        analysis.blockers.insert("layout_overflow".into());
        return (None, analysis);
    };
    (
        Some(AbiLayout {
            size,
            alignment: element.alignment,
            contains_union: element.contains_union,
        }),
        analysis,
    )
}

fn abi_layout(
    typ: &RawComType,
    target: CensusTarget,
    stack: &mut Vec<String>,
) -> (Option<AbiLayout>, Analysis) {
    if typ.pointer_depth > 0 {
        return (
            Some(AbiLayout {
                size: target.pointer_size(),
                alignment: target.pointer_size(),
                contains_union: false,
            }),
            Analysis::default(),
        );
    }
    if let Some((size, alignment)) = scalar_layout(typ, target) {
        return (
            Some(AbiLayout {
                size,
                alignment,
                contains_union: false,
            }),
            Analysis::default(),
        );
    }
    match &typ.native_type {
        RawNativeType::FixedArray { element, count } => {
            let (element, mut analysis) = abi_layout(element, target, stack);
            let Some(element) = element else {
                return (None, analysis);
            };
            let Some(size) = element.size.checked_mul(*count) else {
                analysis.blockers.insert("layout_overflow".into());
                return (None, analysis);
            };
            (
                Some(AbiLayout {
                    size,
                    alignment: element.alignment,
                    contains_union: element.contains_union,
                }),
                analysis,
            )
        }
        RawNativeType::Named {
            namespace,
            name,
            kind: RawNamedKind::Struct,
            layout: Some(layout),
            ..
        } => {
            let identity = format!("{namespace}.{name}");
            if stack.contains(&identity) {
                let mut analysis = Analysis::default();
                analysis.blockers.insert("recursive_layout".into());
                return (None, analysis);
            }
            stack.push(identity);
            let result = analyze_layout_set(layout, target, stack, false);
            stack.pop();
            result
        }
        RawNativeType::Named {
            kind: RawNamedKind::Enum,
            ..
        } => typ.underlying.as_deref().map_or_else(
            || {
                let mut analysis = Analysis::default();
                analysis.blockers.insert("missing_enum_underlying".into());
                (None, analysis)
            },
            |underlying| abi_layout(underlying, target, stack),
        ),
        RawNativeType::Named {
            namespace, name, ..
        } if is_pointer_sized_semantic_type(namespace, name) => (
            Some(AbiLayout {
                size: target.pointer_size(),
                alignment: target.pointer_size(),
                contains_union: false,
            }),
            Analysis::default(),
        ),
        RawNativeType::Named { .. } => typ.underlying.as_deref().map_or_else(
            || {
                let mut analysis = Analysis::default();
                analysis.blockers.insert("missing_layout".into());
                (None, analysis)
            },
            |underlying| abi_layout(underlying, target, stack),
        ),
        _ => {
            let mut analysis = Analysis::default();
            analysis.blockers.insert("unknown_native_type".into());
            (None, analysis)
        }
    }
}

fn scalar_layout(typ: &RawComType, target: CensusTarget) -> Option<(usize, usize)> {
    let layout = match typ.native_type {
        RawNativeType::Bool | RawNativeType::I8 | RawNativeType::U8 => (1, 1),
        RawNativeType::I16 | RawNativeType::U16 | RawNativeType::Char16 => (2, 2),
        RawNativeType::I32 | RawNativeType::U32 | RawNativeType::F32 => (4, 4),
        RawNativeType::I64 | RawNativeType::U64 | RawNativeType::F64 => (8, 8),
        RawNativeType::ISize | RawNativeType::USize => {
            (target.pointer_size(), target.pointer_size())
        }
        RawNativeType::String | RawNativeType::Object => {
            (target.pointer_size(), target.pointer_size())
        }
        RawNativeType::Named {
            ref namespace,
            ref name,
            ..
        } if namespace == "System" && name == "Guid" => (16, 4),
        _ => return None,
    };
    Some(layout)
}

fn checked_align_up(value: usize, alignment: usize) -> Option<usize> {
    value
        .checked_add(alignment.checked_sub(1)?)
        .map(|value| value & !(alignment - 1))
}

fn pointer_needs_manual_output_contract(typ: &RawComType) -> bool {
    if effective_pointer_depth(typ) < 2 || category_for(typ, false) == "Handle" {
        return false;
    }
    match &typ.native_type {
        RawNativeType::Named {
            kind: RawNamedKind::Interface | RawNamedKind::RuntimeClass,
            ..
        } => false,
        RawNativeType::Named {
            namespace, name, ..
        } if is_standard_owned_type(namespace, name) => false,
        _ => true,
    }
}

fn effective_pointer_depth(typ: &RawComType) -> usize {
    typ.pointer_depth.saturating_add(
        typ.underlying
            .as_deref()
            .map(effective_pointer_depth)
            .unwrap_or(0),
    )
}

fn output_has_standard_cleanup(param: &RawComParam) -> bool {
    if !matches!(
        param.direction,
        RawParamDirection::Out | RawParamDirection::InOut
    ) {
        return false;
    }
    match &param.typ.native_type {
        RawNativeType::Named {
            kind: RawNamedKind::Interface | RawNamedKind::RuntimeClass,
            ..
        } => true,
        RawNativeType::Named {
            namespace, name, ..
        } => is_standard_owned_type(namespace, name),
        _ => false,
    }
}

fn is_standard_owned_type(namespace: &str, name: &str) -> bool {
    (namespace == "Windows.Win32.Foundation" && name == "BSTR")
        || (namespace == "Windows.Win32.System.WinRT" && name == "HSTRING")
        || (namespace == "Windows.Win32.System.Variant" && matches!(name, "VARIANT" | "VARIANTARG"))
        || (namespace == "Windows.Win32.System.Com" && name == "SAFEARRAY")
        || (namespace == "Windows.Win32.System.Com.StructuredStorage" && name == "PROPVARIANT")
}

fn is_pointer_sized_semantic_type(namespace: &str, name: &str) -> bool {
    (namespace == "System" && matches!(name, "IntPtr" | "UIntPtr"))
        || (namespace == "Windows.Win32.Foundation"
            && matches!(
                name,
                "BSTR"
                    | "PWSTR"
                    | "PCWSTR"
                    | "LPWSTR"
                    | "LPCWSTR"
                    | "PSTR"
                    | "PCSTR"
                    | "LPSTR"
                    | "LPCSTR"
                    | "PVOID"
                    | "PCVOID"
                    | "LPVOID"
                    | "LPCVOID"
            ))
        || (namespace == "Windows.Win32.System.WinRT" && name == "HSTRING")
}

fn is_special_abi_compound(namespace: &str, name: &str) -> bool {
    (namespace == "Windows.Win32.System.Variant" && matches!(name, "VARIANT" | "VARIANTARG"))
        || (namespace == "Windows.Win32.System.Com"
            && matches!(name, "SAFEARRAY" | "DISPPARAMS" | "EXCEPINFO" | "STATSTG"))
        || (namespace == "Windows.Win32.System.Com.StructuredStorage" && name == "PROPVARIANT")
}

fn collect_interface_types(
    interface: &ComInterfaceMeta,
    safe_complete: bool,
    shapes: &mut BTreeMap<String, ShapeAccumulator>,
    named: &mut BTreeMap<String, NamedAccumulator>,
    categories: &mut BTreeMap<String, CategoryAccumulator>,
) {
    let Some(methods) = interface.raw_methods.as_deref() else {
        return;
    };
    for method in methods {
        let method_path = format!(
            "{}.{}::{}@{}",
            interface.interface.namespace,
            interface.interface.name,
            method.metadata_name,
            method.vtable_index
        );
        collect_type(
            &method.return_type,
            TypeUse::Return,
            None,
            "return",
            &format!("{method_path}.return"),
            true,
            safe_complete,
            shapes,
            named,
            categories,
            &mut Vec::new(),
        );
        for (index, param) in method.params.iter().enumerate() {
            let usage = match param.direction {
                RawParamDirection::In => TypeUse::In,
                RawParamDirection::Out => TypeUse::Out,
                RawParamDirection::InOut => TypeUse::InOut,
            };
            let relation = param.native_array.as_ref().map(|relation| {
                format!(
                    "count={:?};actual={:?};unit={:?};two_call={}",
                    relation.count_param_index,
                    relation.actual_length_param_index,
                    relation.unit,
                    relation.two_call
                )
            });
            collect_type(
                &param.typ,
                usage,
                relation.as_deref(),
                &parameter_ownership_key(param),
                &format!("{method_path}.param[{index}:{}]", param.name),
                true,
                safe_complete,
                shapes,
                named,
                categories,
                &mut Vec::new(),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_type(
    typ: &RawComType,
    usage: TypeUse,
    array_relation: Option<&str>,
    ownership: &str,
    path: &str,
    direct: bool,
    safe_complete: bool,
    shapes: &mut BTreeMap<String, ShapeAccumulator>,
    named: &mut BTreeMap<String, NamedAccumulator>,
    categories: &mut BTreeMap<String, CategoryAccumulator>,
    stack: &mut Vec<String>,
) {
    let category = category_for(typ, array_relation.is_some()).to_string();
    let canonical_type = canonical_type(typ);
    let canonical = format!(
        "category={category};use={};relation={};ownership={ownership};type={canonical_type}",
        usage.key(),
        array_relation.unwrap_or("none")
    );
    let shape = shapes.entry(canonical).or_default();
    shape.category = category.clone();
    shape.expanded_occurrences += 1;
    if direct {
        shape.direct_occurrences += 1;
    }
    *shape.uses.entry(usage.key().into()).or_default() += 1;
    shape.max_pointer_depth = shape.max_pointer_depth.max(effective_pointer_depth(typ));
    shape.constness.insert(format!("{:?}", typ.constness));
    if let Some(relation) = array_relation {
        shape.array_relations.insert(relation.to_string());
    }
    let category_accumulator = categories.entry(category.clone()).or_default();
    category_accumulator.expanded_occurrences += 1;
    if direct {
        category_accumulator.direct_occurrences += 1;
    }
    for target in CensusTarget::ALL {
        shape
            .targets
            .entry(target.key().into())
            .or_default()
            .merge(analyze_type(typ, target, usage, &mut Vec::new()));
    }

    let mut child_path_base = path.to_string();
    if let RawNativeType::Named {
        namespace,
        name,
        kind,
        layout,
        ..
    } = &typ.native_type
    {
        let (identity, identity_kind, containing_path) =
            inventory_identity(namespace, name, path, stack.last(), &canonical_type);
        child_path_base = format!("{path}.type[{identity}]");
        shape.named_identities.insert(identity.clone());
        category_accumulator.named_types.insert(identity.clone());
        let value = named.entry(identity.clone()).or_default();
        value.identity_key = identity.clone();
        value.identity_kind = identity_kind;
        value.containing_path = containing_path;
        value.namespace = namespace.clone();
        value.name = name.clone();
        value.raw_named_kind = format!("{kind:?}");
        value.category = named_category(typ).to_string();
        value.expanded_occurrences += 1;
        if direct {
            value.direct_occurrences += 1;
        }
        let pointer_depth = effective_pointer_depth(typ);
        value.max_pointer_depth = value.max_pointer_depth.max(pointer_depth);
        if pointer_depth == 0 {
            value.by_value_occurrences += 1;
        } else {
            value.pointer_occurrences += 1;
        }
        if safe_complete {
            value.safe_interface_occurrences += 1;
        } else {
            value.unsafe_interface_occurrences += 1;
        }
        if let Some(layout) = layout.as_deref() {
            value.layout_facts.extend(layout_facts(layout));
        }
        for target in CensusTarget::ALL {
            value
                .targets
                .entry(target.key().into())
                .or_default()
                .merge(analyze_type(typ, target, usage, &mut Vec::new()));
        }
        if stack.contains(&identity) {
            return;
        }
        stack.push(identity);
    }

    if let Some(underlying) = typ.underlying.as_deref() {
        collect_type(
            underlying,
            TypeUse::Nested,
            None,
            "underlying",
            &format!("{child_path_base}.underlying"),
            false,
            safe_complete,
            shapes,
            named,
            categories,
            stack,
        );
    }
    match &typ.native_type {
        RawNativeType::Array(element) | RawNativeType::FixedArray { element, .. } => collect_type(
            element,
            TypeUse::Nested,
            None,
            "element",
            &format!("{child_path_base}.element"),
            false,
            safe_complete,
            shapes,
            named,
            categories,
            stack,
        ),
        RawNativeType::Named {
            layout: Some(layout),
            ..
        } => {
            for (variant_index, variant) in layout.variants.iter().enumerate() {
                for (field_index, field) in variant.fields.iter().enumerate() {
                    collect_type(
                        &field.typ,
                        TypeUse::Nested,
                        None,
                        "field",
                        &format!(
                            "{child_path_base}.layout[{variant_index}].field[{field_index}:{}]",
                            field.name
                        ),
                        false,
                        safe_complete,
                        shapes,
                        named,
                        categories,
                        stack,
                    );
                }
            }
        }
        _ => {}
    }
    if matches!(typ.native_type, RawNativeType::Named { .. }) {
        stack.pop();
    }
}

fn parameter_ownership_key(param: &RawComParam) -> String {
    format!(
        "direction={:?};optional={};free_with={};safe_array={};string_ownership={};interface_output={}",
        param.direction,
        param.optional,
        param
            .free_with
            .as_ref()
            .map(|value| value.function.as_str())
            .unwrap_or("none"),
        param
            .safe_array_evidence
            .as_ref()
            .map(|value| format!("{:?}", value.ownership))
            .unwrap_or_else(|| "none".into()),
        param
            .string_pointer_array
            .as_ref()
            .map(|value| format!("{:?}", value.ownership))
            .unwrap_or_else(|| "none".into()),
        param
            .exact_interface_output
            .as_ref()
            .map(|value| format!("owned+1:{}", value.interface_iid))
            .unwrap_or_else(|| "none".into()),
    )
}

fn inventory_identity(
    namespace: &str,
    name: &str,
    path: &str,
    enclosing_identity: Option<&String>,
    canonical: &str,
) -> (String, String, Option<String>) {
    if namespace.is_empty() || name.is_empty() || name.starts_with("_Anonymous") {
        let containing = enclosing_identity.map_or(path, String::as_str);
        (
            format!("anonymous::{containing}::{name}::{canonical}"),
            "anonymous_nested_record".into(),
            Some(containing.to_string()),
        )
    } else if namespace == "System" {
        (
            format!("{namespace}.{name}"),
            "synthetic_or_system".into(),
            None,
        )
    } else {
        (
            format!("{namespace}.{name}"),
            "named_metadata_definition".into(),
            None,
        )
    }
}

fn canonical_type(typ: &RawComType) -> String {
    canonical_type_inner(typ, &mut Vec::new())
}

fn canonical_type_inner(typ: &RawComType, stack: &mut Vec<String>) -> String {
    let base = match &typ.native_type {
        RawNativeType::Void => "void".into(),
        RawNativeType::Bool => "bool".into(),
        RawNativeType::I8 => "i8".into(),
        RawNativeType::U8 => "u8".into(),
        RawNativeType::I16 => "i16".into(),
        RawNativeType::U16 => "u16".into(),
        RawNativeType::I32 => "i32".into(),
        RawNativeType::U32 => "u32".into(),
        RawNativeType::I64 => "i64".into(),
        RawNativeType::U64 => "u64".into(),
        RawNativeType::F32 => "f32".into(),
        RawNativeType::F64 => "f64".into(),
        RawNativeType::Char16 => "char16".into(),
        RawNativeType::ISize => "isize".into(),
        RawNativeType::USize => "usize".into(),
        RawNativeType::String => "hstring".into(),
        RawNativeType::Object => "object".into(),
        RawNativeType::Named {
            namespace,
            name,
            kind,
            iid,
            layout,
            ..
        } => {
            let identity = format!("{namespace}.{name}");
            let layout = if stack.contains(&identity) {
                "recursive".into()
            } else if let Some(layout) = layout.as_deref() {
                stack.push(identity);
                let value = canonical_layout_set(layout, stack);
                stack.pop();
                value
            } else {
                "none".into()
            };
            format!(
                "named({namespace}.{name};kind={kind:?};iid={};layout={layout};delegate_signature={})",
                iid.as_deref().unwrap_or("none"),
                if *kind == RawNamedKind::Delegate {
                    "unavailable"
                } else {
                    "not_applicable"
                }
            )
        }
        RawNativeType::Array(element) => {
            format!("array({})", canonical_type_inner(element, stack))
        }
        RawNativeType::FixedArray { element, count } => {
            format!("fixed[{count}]({})", canonical_type_inner(element, stack))
        }
        RawNativeType::Unknown(value) => format!("unknown({value})"),
    };
    let underlying = typ
        .underlying
        .as_deref()
        .map(|underlying| format!(";underlying={}", canonical_type_inner(underlying, stack)))
        .unwrap_or_default();
    format!(
        "{base};ptr={};const={:?}{underlying}",
        typ.pointer_depth, typ.constness
    )
}

fn canonical_layout_set(layout: &RawNativeLayoutSet, stack: &mut Vec<String>) -> String {
    let variants = layout
        .variants
        .iter()
        .map(|variant| {
            let fields = variant
                .fields
                .iter()
                .map(|field| {
                    format!(
                        "{}:type={}:offset={:?}:fixed_count={:?}:bitfield={}:flexible={}",
                        field.name,
                        canonical_type_inner(&field.typ, stack),
                        field.explicit_offset,
                        field.fixed_count,
                        field.bitfield,
                        field.flexible_array
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "arch={}:kind={:?}:packing={:?}:declared_size={:?}:union={}:fields=[{}]",
                variant.architectures,
                variant.kind,
                variant.packing,
                variant.declared_size,
                variant.is_union,
                fields
            )
        })
        .collect::<Vec<_>>()
        .join("|");
    format!("recursive={};variants=[{variants}]", layout.recursive)
}

pub fn raw_aggregate_descriptor(
    namespace: &str,
    name: &str,
    layout: &RawNativeLayoutSet,
) -> Result<(bool, String), String> {
    if namespace.is_empty() || name.is_empty() {
        return Err("Raw aggregate descriptors require a qualified metadata identity".into());
    }
    let is_union = layout
        .variants
        .first()
        .is_some_and(|variant| variant.is_union);
    let mut root = serde_json::Map::new();
    root.insert(
        "name".into(),
        serde_json::Value::String(format!("{namespace}.{name}")),
    );
    for target in CensusTarget::ALL {
        let candidates = layout
            .variants
            .iter()
            .filter(|variant| variant.architectures & target.metadata_mask() != 0)
            .collect::<Vec<_>>();
        let [variant] = candidates.as_slice() else {
            return Err(format!(
                "{} target layout is not singular for {namespace}.{name}",
                target.key()
            ));
        };
        root.insert(
            target.key().into(),
            raw_layout_descriptor_value(variant, target, &mut Vec::new())?,
        );
    }
    serde_json::to_string(&serde_json::Value::Object(root))
        .map(|value| (is_union, value))
        .map_err(|error| format!("Failed to serialize raw aggregate descriptor: {error}"))
}

fn raw_layout_descriptor_value(
    layout: &RawNativeLayout,
    target: CensusTarget,
    stack: &mut Vec<String>,
) -> Result<serde_json::Value, String> {
    let (computed, analysis) = analyze_layout(layout, target, stack, false);
    let computed = computed.ok_or_else(|| {
        format!(
            "{} layout is not raw-descriptor complete: {}",
            target.key(),
            analysis.blockers.into_iter().collect::<Vec<_>>().join(",")
        )
    })?;
    let mut fields = Vec::with_capacity(layout.fields.len());
    let mut cursor = 0usize;
    for field in &layout.fields {
        let (field_layout, field_analysis) = analyze_field_layout(field, target, stack);
        let field_layout = field_layout.ok_or_else(|| {
            format!(
                "{} field `{}` is not layout complete: {}",
                target.key(),
                field.name,
                field_analysis
                    .blockers
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(",")
            )
        })?;
        let typ = raw_descriptor_field_type(&field.typ, target, stack)?;
        let count = field.fixed_count.unwrap_or(1);
        let mut value = serde_json::Map::new();
        value.insert("name".into(), serde_json::Value::String(field.name.clone()));
        if !layout.is_union {
            let natural = checked_align_up(cursor, field_layout.alignment)
                .ok_or_else(|| "Raw aggregate offset overflow".to_string())?;
            let offset = if layout.kind == RawLayoutKind::Explicit {
                field.explicit_offset.unwrap_or(natural)
            } else {
                natural
            };
            value.insert("offset".into(), serde_json::Value::from(offset));
            cursor = offset
                .checked_add(field_layout.size)
                .ok_or_else(|| "Raw aggregate field end overflow".to_string())?;
        }
        value.insert("count".into(), serde_json::Value::from(count));
        value.insert("type".into(), typ);
        fields.push(serde_json::Value::Object(value));
    }
    let mut result = serde_json::Map::new();
    result.insert("size".into(), serde_json::Value::from(computed.size));
    result.insert(
        "alignment".into(),
        serde_json::Value::from(computed.alignment),
    );
    if layout.is_union {
        result.insert("complete".into(), serde_json::Value::Bool(true));
    }
    result.insert("fields".into(), serde_json::Value::Array(fields));
    Ok(serde_json::Value::Object(result))
}

fn raw_descriptor_field_type(
    typ: &RawComType,
    target: CensusTarget,
    stack: &mut Vec<String>,
) -> Result<serde_json::Value, String> {
    if typ.pointer_depth > 0 {
        return Ok(serde_json::json!({ "kind": "pointer" }));
    }
    let scalar = match &typ.native_type {
        RawNativeType::Bool | RawNativeType::U8 => Some("u8"),
        RawNativeType::I8 => Some("i8"),
        RawNativeType::I16 => Some("i16"),
        RawNativeType::U16 | RawNativeType::Char16 => Some("u16"),
        RawNativeType::I32 => Some("i32"),
        RawNativeType::U32 => Some("u32"),
        RawNativeType::I64 => Some("i64"),
        RawNativeType::U64 => Some("u64"),
        RawNativeType::F32 => Some("f32"),
        RawNativeType::F64 => Some("f64"),
        RawNativeType::ISize => Some("isize"),
        RawNativeType::USize => Some("usize"),
        RawNativeType::Named {
            namespace, name, ..
        } if namespace == "System" && name == "Guid" => Some("guid"),
        _ => None,
    };
    if let Some(kind) = scalar {
        return Ok(serde_json::json!({ "kind": kind }));
    }
    match &typ.native_type {
        RawNativeType::Named {
            namespace,
            name,
            kind: RawNamedKind::Enum,
            ..
        } => typ
            .underlying
            .as_deref()
            .ok_or_else(|| format!("Enum {namespace}.{name} has no underlying type"))
            .and_then(|underlying| raw_descriptor_field_type(underlying, target, stack)),
        RawNativeType::Named {
            namespace,
            name,
            kind: RawNamedKind::Struct,
            layout: Some(layout),
            ..
        } => {
            let identity = format!("{namespace}.{name}");
            if stack.contains(&identity) {
                return Err(format!("Recursive raw aggregate `{identity}`"));
            }
            stack.push(identity);
            let candidates = layout
                .variants
                .iter()
                .filter(|variant| variant.architectures & target.metadata_mask() != 0)
                .collect::<Vec<_>>();
            let [nested] = candidates.as_slice() else {
                stack.pop();
                return Err(format!(
                    "{} nested layout is not singular for {namespace}.{name}",
                    target.key()
                ));
            };
            let nested_layout = raw_layout_descriptor_value(nested, target, stack);
            stack.pop();
            Ok(serde_json::json!({
                "kind": if nested.is_union { "union" } else { "struct" },
                "name": format!("{namespace}.{name}"),
                "layout": nested_layout?,
            }))
        }
        RawNativeType::FixedArray { element, .. } => {
            raw_descriptor_field_type(element, target, stack)
        }
        _ => Err(format!(
            "Raw aggregate field type is not descriptor-complete: {}",
            canonical_type(typ)
        )),
    }
}

fn category_for(typ: &RawComType, counted: bool) -> &'static str {
    if counted {
        return "CountedBuffer";
    }
    if matches!(typ.native_type, RawNativeType::Named { .. }) {
        let base = super::model::metadata::census_raw_base_category(typ);
        return if base == "Unknown" && typ.pointer_depth > 0 {
            "DataPointer"
        } else {
            base
        };
    }
    if typ.pointer_depth > 0 {
        return if matches!(typ.native_type, RawNativeType::Void | RawNativeType::Object) {
            "DataPointer"
        } else {
            "Pointer"
        };
    }
    match typ.native_type {
        RawNativeType::Bool
        | RawNativeType::I8
        | RawNativeType::U8
        | RawNativeType::I16
        | RawNativeType::U16
        | RawNativeType::I32
        | RawNativeType::U32
        | RawNativeType::I64
        | RawNativeType::U64
        | RawNativeType::F32
        | RawNativeType::F64
        | RawNativeType::Char16
        | RawNativeType::ISize
        | RawNativeType::USize => "Scalar",
        RawNativeType::String => "HString",
        RawNativeType::Object => "ComInterface",
        RawNativeType::Array(_) => "CountedBuffer",
        RawNativeType::FixedArray { .. } => "NativeStruct",
        RawNativeType::Void | RawNativeType::Unknown(_) | RawNativeType::Named { .. } => "Unknown",
    }
}

fn named_category(typ: &RawComType) -> &'static str {
    category_for(
        &RawComType {
            pointer_depth: 0,
            ..typ.clone()
        },
        false,
    )
}

fn layout_facts(layout: &RawNativeLayoutSet) -> BTreeSet<LayoutFact> {
    if layout.variants.is_empty() {
        return BTreeSet::from([LayoutFact {
            architectures: 0,
            kind: "Unknown".into(),
            packing: "Unknown".into(),
            declared_size: None,
            field_count: 0,
            is_union: false,
            recursive: layout.recursive,
            x64_size: None,
            x64_alignment: None,
            i686_size: None,
            i686_alignment: None,
            arm64_size: None,
            arm64_alignment: None,
        }]);
    }
    layout
        .variants
        .iter()
        .map(|variant| {
            let computed = |target: CensusTarget| {
                (variant.architectures & target.metadata_mask() != 0)
                    .then(|| analyze_layout(variant, target, &mut Vec::new(), false).0)
                    .flatten()
            };
            let x64 = computed(CensusTarget::X64);
            let i686 = computed(CensusTarget::I686);
            let arm64 = computed(CensusTarget::Arm64);
            LayoutFact {
                architectures: variant.architectures,
                kind: format!("{:?}", variant.kind),
                packing: format!("{:?}", variant.packing),
                declared_size: variant.declared_size,
                field_count: variant.fields.len(),
                is_union: variant.is_union,
                recursive: layout.recursive,
                x64_size: x64.map(|layout| layout.size),
                x64_alignment: x64.map(|layout| layout.alignment),
                i686_size: i686.map(|layout| layout.size),
                i686_alignment: i686.map(|layout| layout.alignment),
                arm64_size: arm64.map(|layout| layout.size),
                arm64_alignment: arm64.map(|layout| layout.alignment),
            }
        })
        .collect()
}

fn summarize(
    metadata: MetadataIdentity,
    interfaces: &[InterfaceCapability],
    type_categories: Vec<CategoryCounts>,
    parsed_interface_identities: usize,
    metadata_typedefs: usize,
    addressable_type_definitions: usize,
    registered_exact_entries: usize,
    metadata_matched_exact_entries: usize,
    exact_catalog: &BTreeMap<String, crate::contract_registry::ExactRegistryEntry>,
    metadata_matched_entry_ids: &BTreeSet<String>,
) -> CapabilitySummary {
    let safe_complete = interfaces
        .iter()
        .filter(|interface| interface.safe_complete)
        .count();
    let mut targets = BTreeMap::new();
    let mut blocker_reason_counts = BTreeMap::new();
    let mut manual_reason_counts = BTreeMap::new();
    let mut lifecycle_flag_counts = BTreeMap::new();
    for target in CensusTarget::ALL {
        let mut counts = TargetCounts {
            raw_metadata_complete: 0,
            raw_manual_contract: 0,
            raw_runtime_blocked: 0,
            safe_incomplete_raw_metadata_complete: 0,
            safe_incomplete_raw_manual_contract: 0,
            safe_incomplete_raw_runtime_blocked: 0,
        };
        let mut blockers = BTreeMap::new();
        let mut manual = BTreeMap::new();
        let mut lifecycle = BTreeMap::from([
            ("cleanup_none_required".into(), 0usize),
            ("cleanup_standard_supported".into(), 0usize),
            ("cleanup_known_external".into(), 0usize),
            ("cleanup_unknown".into(), 0usize),
            ("requires_external_pointer_or_callback".into(), 0usize),
            ("requires_external_acquisition".into(), 0usize),
            ("requires_current_apartment".into(), 0usize),
        ]);
        for interface in interfaces {
            let capability = &interface.targets[target.key()];
            match capability.classification {
                RawClassification::RawMetadataComplete => {
                    counts.raw_metadata_complete += 1;
                    if !interface.safe_complete {
                        counts.safe_incomplete_raw_metadata_complete += 1;
                    }
                }
                RawClassification::RawManualContract => {
                    counts.raw_manual_contract += 1;
                    if !interface.safe_complete {
                        counts.safe_incomplete_raw_manual_contract += 1;
                    }
                }
                RawClassification::RawRuntimeBlocked => {
                    counts.raw_runtime_blocked += 1;
                    if !interface.safe_complete {
                        counts.safe_incomplete_raw_runtime_blocked += 1;
                    }
                }
            }
            for reason in &capability.blocker_reasons {
                *blockers.entry(reason.clone()).or_default() += 1;
            }
            for reason in &capability.manual_contract_reasons {
                *manual.entry(reason.clone()).or_default() += 1;
            }
            *lifecycle
                .entry(format!(
                    "cleanup_{}",
                    match capability.lifecycle.cleanup {
                        CleanupAvailability::NoneRequired => "none_required",
                        CleanupAvailability::StandardSupported => "standard_supported",
                        CleanupAvailability::KnownExternal => "known_external",
                        CleanupAvailability::Unknown => "unknown",
                    }
                ))
                .or_default() += 1;
            for (name, enabled) in [
                (
                    "requires_external_pointer_or_callback",
                    capability.lifecycle.requires_external_pointer_or_callback,
                ),
                (
                    "requires_external_acquisition",
                    capability.lifecycle.requires_external_acquisition,
                ),
                (
                    "requires_current_apartment",
                    capability.lifecycle.requires_current_apartment,
                ),
            ] {
                if enabled {
                    *lifecycle.entry(name.to_string()).or_default() += 1;
                }
            }
        }
        targets.insert(target.key().into(), counts);
        blocker_reason_counts.insert(target.key().into(), blockers);
        manual_reason_counts.insert(target.key().into(), manual);
        lifecycle_flag_counts.insert(target.key().into(), lifecycle);
    }

    let mut standard_derived = 0;
    let mut exact_registry_dependent = 0;
    let mut metadata_fact_occurrences = 0;
    let mut com_standard_fact_occurrences = 0;
    let mut safe_consumed_entry_ids = BTreeSet::new();
    let mut exact_entry_interface_dependencies = 0;
    let mut exact_family_interface_dependencies = 0;
    let mut by_contract_kind = BTreeMap::new();
    let mut by_entry_id = BTreeMap::new();
    let mut by_family_id = BTreeMap::new();
    let mut by_metadata_attribute = BTreeMap::new();
    let mut by_standard_rule_id = BTreeMap::new();
    for interface in interfaces
        .iter()
        .filter(|interface| interface.safe_complete)
    {
        match interface.evidence_class {
            Some(SafeEvidenceClass::StandardDerived) => standard_derived += 1,
            Some(SafeEvidenceClass::ExactRegistryDependent) => exact_registry_dependent += 1,
            None => unreachable!("safe-complete interface has an evidence class"),
        }
        metadata_fact_occurrences += interface.metadata_attributes.len();
        com_standard_fact_occurrences += interface.standard_rule_ids.len();
        exact_entry_interface_dependencies += interface.exact_entry_ids.len();
        exact_family_interface_dependencies += interface.exact_family_ids.len();
        for value in &interface.metadata_attributes {
            *by_metadata_attribute.entry(value.clone()).or_default() += 1;
        }
        for value in &interface.standard_rule_ids {
            *by_standard_rule_id.entry(value.clone()).or_default() += 1;
        }
        for value in &interface.exact_entry_ids {
            safe_consumed_entry_ids.insert(value.clone());
            *by_entry_id.entry(value.clone()).or_default() += 1;
        }
        for value in &interface.exact_family_ids {
            *by_family_id.entry(value.clone()).or_default() += 1;
        }
        for kind in interface.exact_entry_kinds.values() {
            *by_contract_kind.entry(kind.clone()).or_default() += 1;
        }
    }
    let exact_entry_status = exact_catalog
        .iter()
        .map(|(entry_id, entry)| {
            (
                entry_id.clone(),
                ExactEntryStatus {
                    family_id: entry.family_id.id().into(),
                    contract_kind: entry.contract_kind.key().into(),
                    selector: entry.selector.clone(),
                    source_fingerprint: entry.source_fingerprint.clone(),
                    citation: entry.citation.clone(),
                    registered: true,
                    metadata_matched: metadata_matched_entry_ids.contains(entry_id),
                    safe_consumed: safe_consumed_entry_ids.contains(entry_id),
                    interface_dependencies: by_entry_id.get(entry_id).copied().unwrap_or(0),
                },
            )
        })
        .collect();
    let safe_evidence = SafeEvidenceSummary {
        safe_complete,
        standard_derived,
        exact_registry_dependent,
        metadata_fact_occurrences,
        com_standard_fact_occurrences,
        registered_exact_entries,
        metadata_matched_exact_entries,
        safe_consumed_exact_entries: safe_consumed_entry_ids.len(),
        exact_entry_interface_dependencies,
        exact_family_interface_dependencies,
        by_contract_kind,
        by_entry_id,
        by_family_id,
        by_metadata_attribute,
        by_standard_rule_id,
        exact_entry_status,
        count_semantics: "registeredExactEntries counts declared selector-specific entries; metadataMatchedExactEntries counts entries matched against pinned metadata; safeConsumedExactEntries counts distinct entries used by safe plans; exactEntryInterfaceDependencies counts every safe entry/interface pair; exactFamilyInterfaceDependencies counts each family once per safe interface. None is a net-contribution or ablation count.".into(),
    };

    CapabilitySummary {
        schema_version: 3,
        metadata,
        regeneration_command: REGENERATION_COMMAND.into(),
        definitions: BTreeMap::from([
            (
                "safe_complete".into(),
                "Existing complete safe generator succeeds for the full inherited interface.".into(),
            ),
            (
                "standard_derived".into(),
                "A safe-complete interface whose validated plan consumes only WinMD facts and universal COM standard rules.".into(),
            ),
            (
                "exact_registry_dependent".into(),
                "A safe-complete interface whose validated plan consumes at least one exact registry entry.".into(),
            ),
            (
                "raw_metadata_complete".into(),
                "Every outbound ABI fact is present and expressible by the Phase 1 raw runtime.".into(),
            ),
            (
                "raw_manual_contract".into(),
                "Pointer ABI execution is expressible, but the caller must supply listed semantic/lifetime facts or externally created pointee storage. Missing layout for caller-created readable/writable T* storage is not manual; it is blocked.".into(),
            ),
            (
                "raw_runtime_blocked".into(),
                "At least one inherited or declared method uses a target ABI shape rejected by the current raw runtime.".into(),
            ),
        ]),
        reason_definitions: BTreeMap::from([
            ("allocator_interface_cleanup".into(), "The returned allocation is released through a known allocator-interface method rather than a Phase 1 standard cleanup wrapper.".into()),
            ("ambiguous_target_layout".into(), "More than one native layout variant applies to the target.".into()),
            ("arm64_union_by_value_gate".into(), "ARM64 by-value unions and structs containing unions remain runtime-gated.".into()),
            ("external_pointee_storage".into(), "The pointer ABI is known, but the caller must supply externally created pointee storage.".into()),
            ("incomplete_pointee_layout_for_storage".into(), "A writable/readable T* contract requires caller-created storage whose exact layout is unavailable or invalid.".into()),
            ("incomplete_layout".into(), "A required by-value aggregate has no fields.".into()),
            ("incomplete_raw_vtable".into(), "Raw methods do not cover the complete inherited compatibility method set.".into()),
            ("incomplete_union_layout".into(), "A union does not have a complete exact natural target layout.".into()),
            ("invalid_vtable_slot".into(), "Raw method slots are not contiguous from the interface root.".into()),
            ("layout_overflow".into(), "Checked native size or offset arithmetic overflowed.".into()),
            ("missing_allocator".into(), "A pointer replacement/allocation requires caller-supplied allocator knowledge.".into()),
            ("missing_count_relation".into(), "A native array lacks an authoritative count relation.".into()),
            ("missing_declaring_iid".into(), "A method's declaring interface IID is unavailable.".into()),
            ("missing_enum_underlying".into(), "An enum's exact scalar ABI is unavailable.".into()),
            ("missing_handle_ownership".into(), "A handle output is not covered by exact borrowed-handle evidence or a declared cleanup owner.".into()),
            ("missing_interface_iid".into(), "A required interface identity has no exact IID.".into()),
            ("missing_interface_replacement_contract".into(), "A typed interface InOut slot requires exact old/new ownership and replacement semantics.".into()),
            ("missing_interface_root".into(), "The interface has no supported exact IUnknown/IInspectable vtable root.".into()),
            ("missing_layout".into(), "A required by-value named type has no target layout facts.".into()),
            ("missing_output_ownership".into(), "A pointer output/replacement requires caller-supplied ownership semantics.".into()),
            ("missing_raw_methods".into(), "The complete inherited raw method list is unavailable.".into()),
            ("mixed_pointer_constness".into(), "Per-level pointer qualifiers collapse to mixed/unspecified metadata.".into()),
            ("nested_pointer_lifetime".into(), "A complete aggregate contains pointer fields whose nested lifetime/ownership remains caller-managed.".into()),
            ("non_natural_struct_layout".into(), "A raw by-value struct contains unexplained gaps, packing, or inflated tail padding.".into()),
            ("not_addressable".into(), "The identity is outside the addressable outbound Classic COM interface set.".into()),
            ("opaque_pointee_contract".into(), "The pointer width is known but the caller must supply the opaque pointee contract.".into()),
            ("pointee_missing_layout".into(), "The pointed-to aggregate has no complete metadata layout.".into()),
            ("pointee_recursive_layout".into(), "The pointed-to aggregate layout is recursively incomplete.".into()),
            ("pointee_unknown_native_type".into(), "The pointed-to aggregate contains an unknown native field type.".into()),
            ("pointee_unsupported_bitfield".into(), "The pointed-to aggregate contains a bitfield outside the Phase 1 layout subset.".into()),
            ("pointee_unsupported_flexible_array".into(), "The pointed-to aggregate contains a flexible array outside the Phase 1 layout subset.".into()),
            ("pointee_unsupported_layout_kind".into(), "The pointed-to aggregate has an unknown layout kind.".into()),
            ("pointee_unsupported_packing".into(), "The pointed-to aggregate uses packing not admitted by the current raw descriptor subset.".into()),
            ("recursive_layout".into(), "A required by-value aggregate layout is recursively incomplete.".into()),
            ("unknown_native_type".into(), "The metadata type has no exact supported native ABI identity.".into()),
            ("unknown_string_encoding".into(), "A string pointer array lacks exact encoding facts.".into()),
            ("unsupported_bitfield".into(), "Bitfield by-value layout is outside the Phase 1 raw subset.".into()),
            ("unsupported_flexible_array".into(), "Flexible-array by-value layout is outside the Phase 1 raw subset.".into()),
            ("unsupported_layout_kind".into(), "The required native layout kind is unknown.".into()),
            ("unsupported_packing".into(), "Packed/custom-packed by-value aggregates are outside the Phase 1 raw subset.".into()),
            ("unsupported_top_level_fixed_array".into(), "A bare fixed-array argument/return has no Phase 1 by-value type.".into()),
            ("variant_safearray_element_contract".into(), "A VARIANT SAFEARRAY element contract requires caller-supplied semantic interpretation.".into()),
            ("win64_irregular_aggregate_copy".into(), "Bundled libffi 3.5.2 has an irregular-size argument passing/copy defect for top-level 3/5/6/7-byte Win64 aggregates.".into()),
        ]),
        csv_json_cell_schemas: BTreeMap::from([
            ("interface_target".into(), "[classification,first_blocker,blockers[],manual_reasons[],cleanup_availability,external_pointer_or_callback,external_acquisition,current_apartment]".into()),
            ("interface_evidence".into(), "evidence_class plus JSON arrays of standard rule IDs, exact entry IDs, exact family IDs, exact contract kinds, and metadata attributes; counts are current plan dependencies, not net contribution".into()),
            ("inventory_target".into(), "[classification,blockers[],manual_reasons[]]".into()),
            ("layout_facts".into(), "JSON array of complete target layout fact objects".into()),
            ("uses_constness_relations_identities".into(), "JSON object/array; RFC 4180 CSV escaping is applied by the csv crate".into()),
        ]),
        limitations: vec![
            "Outbound ABI callability only; callbacks, servers, aggregation, and cross-apartment marshaling are excluded.".into(),
            "Safe-complete interfaces are accepted by construction because the existing validated semantic call plan is exported from the raw entrypoint.".into(),
            "Lifecycle and acquisition flags are orthogonal to ABI classification and do not imply an automatic wrapper.".into(),
            "Direct occurrence counts cover method parameters/returns; expanded counts additionally recurse through underlying, element, and architecture-layout field types.".into(),
            "Safe evidence counts are dependency counts. No net-contribution claim is made without a controlled contract-family ablation.".into(),
        ],
        metadata_typedefs,
        addressable_type_definitions,
        parsed_interface_identities,
        eligible_interfaces: interfaces.len(),
        not_addressable: parsed_interface_identities.saturating_sub(interfaces.len()),
        safe_complete,
        safe_incomplete: interfaces.len() - safe_complete,
        safe_evidence,
        targets,
        blocker_reason_counts,
        manual_reason_counts,
        lifecycle_flag_counts,
        type_categories,
    }
}

fn write_report_files(
    output_dir: &Path,
    report: &CapabilityReport,
    emit_large_json: bool,
) -> Result<(), String> {
    fs::create_dir_all(output_dir)
        .map_err(|error| format!("Failed to create {}: {error}", output_dir.display()))?;
    write_json(
        output_dir.join("classic-com-capability-summary.json"),
        &report.summary,
    )?;
    write_interface_support_csv(
        output_dir.join("classic-com-interface-support.csv"),
        &report.interfaces,
    )?;
    for stale in [
        "classic-com-interface-capabilities.json",
        "classic-com-named-types.json",
        "classic-com-type-shapes.json",
    ] {
        let path = output_dir.join(stale);
        if !emit_large_json && path.exists() {
            fs::remove_file(&path)
                .map_err(|error| format!("Failed to remove {}: {error}", path.display()))?;
        }
    }
    if emit_large_json {
        write_json(
            output_dir.join("classic-com-interface-capabilities.json"),
            &report.interfaces,
        )?;
        write_json(
            output_dir.join("classic-com-named-types.json"),
            &report.named_types,
        )?;
        write_json(
            output_dir.join("classic-com-type-shapes.json"),
            &report.type_shapes,
        )?;
    }
    write_interface_csv(
        output_dir.join("classic-com-interface-capabilities.csv"),
        &report.interfaces,
    )?;
    write_named_csv(
        output_dir.join("classic-com-named-types.csv"),
        &report.named_types,
    )?;
    write_shape_csv(
        output_dir.join("classic-com-type-shapes.csv"),
        &report.type_shapes,
    )?;
    write_definitions_csv(
        output_dir.join("classic-com-all-metadata-definitions.csv"),
        &report.all_metadata_definitions,
    )?;
    write_markdown(
        output_dir.join("classic-com-named-types.md"),
        &report.named_types,
        &report.summary,
    )?;
    Ok(())
}

fn write_json(path: PathBuf, value: &impl Serialize) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("Failed to serialize {}: {error}", path.display()))?;
    bytes.push(b'\n');
    fs::write(&path, bytes).map_err(|error| format!("Failed to write {}: {error}", path.display()))
}

fn json_cell(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| format!("Failed to serialize CSV cell: {error}"))
}

fn interface_target_cell(value: &TargetCapability) -> Result<String, String> {
    json_cell(&(
        value.classification.key(),
        &value.first_blocker_reason,
        &value.blocker_reasons,
        &value.manual_contract_reasons,
        value.lifecycle.cleanup,
        value.lifecycle.requires_external_pointer_or_callback,
        value.lifecycle.requires_external_acquisition,
        value.lifecycle.requires_current_apartment,
    ))
}

fn inventory_target_cell(value: &TargetCapability) -> Result<String, String> {
    json_cell(&(
        value.classification.key(),
        &value.blocker_reasons,
        &value.manual_contract_reasons,
    ))
}

fn write_interface_csv(path: PathBuf, values: &[InterfaceCapability]) -> Result<(), String> {
    let file = fs::File::create(&path)
        .map_err(|error| format!("Failed to create {}: {error}", path.display()))?;
    let mut writer = csv::Writer::from_writer(file);
    writer
        .write_record([
            "namespace",
            "name",
            "iid",
            "is_iunknown_rooted",
            "method_count",
            "first_vtable_slot",
            "last_vtable_slot",
            "safe_complete",
            "safe_error",
            "evidence_class",
            "standard_rule_ids",
            "exact_entry_ids",
            "exact_family_ids",
            "exact_contract_kinds",
            "metadata_attributes",
            "x64",
            "i686",
            "arm64",
        ])
        .map_err(|error| error.to_string())?;
    for value in values {
        writer
            .write_record([
                value.namespace.clone(),
                value.name.clone(),
                value.iid.clone(),
                value.is_iunknown_rooted.to_string(),
                value.method_count.to_string(),
                value
                    .first_vtable_slot
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                value
                    .last_vtable_slot
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                value.safe_complete.to_string(),
                value.safe_error.clone().unwrap_or_default(),
                value
                    .evidence_class
                    .map(SafeEvidenceClass::key)
                    .unwrap_or_default()
                    .into(),
                json_cell(&value.standard_rule_ids)?,
                json_cell(&value.exact_entry_ids)?,
                json_cell(&value.exact_family_ids)?,
                json_cell(&value.exact_contract_kinds)?,
                json_cell(&value.metadata_attributes)?,
                interface_target_cell(&value.targets["x64"])?,
                interface_target_cell(&value.targets["i686"])?,
                interface_target_cell(&value.targets["arm64"])?,
            ])
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

fn interface_support_reason(value: &InterfaceCapability) -> Option<&str> {
    if value.safe_complete {
        return None;
    }
    for target in ["x64", "i686", "arm64"] {
        let capability = &value.targets[target];
        if let Some(reason) = capability.first_blocker_reason.as_deref() {
            return Some(reason);
        }
        if let Some(reason) = capability.manual_contract_reasons.first() {
            return Some(reason);
        }
    }
    Some("safe_projection_incomplete")
}

fn write_interface_support_csv(
    path: PathBuf,
    values: &[InterfaceCapability],
) -> Result<(), String> {
    let file = fs::File::create(&path)
        .map_err(|error| format!("Failed to create {}: {error}", path.display()))?;
    let mut writer = csv::Writer::from_writer(file);
    writer
        .write_record([
            "namespace",
            "name",
            "iid",
            "safe_complete",
            "evidence_class",
            "first_reason_code",
        ])
        .map_err(|error| error.to_string())?;
    for value in values {
        writer
            .write_record([
                value.namespace.as_str(),
                value.name.as_str(),
                value.iid.as_str(),
                if value.safe_complete { "true" } else { "false" },
                value
                    .evidence_class
                    .map(SafeEvidenceClass::key)
                    .unwrap_or_default(),
                interface_support_reason(value).unwrap_or_default(),
            ])
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

fn write_named_csv(path: PathBuf, values: &[NamedTypeInventory]) -> Result<(), String> {
    let file = fs::File::create(&path)
        .map_err(|error| format!("Failed to create {}: {error}", path.display()))?;
    let mut writer = csv::Writer::from_writer(file);
    writer
        .write_record([
            "identity_key",
            "identity_kind",
            "containing_path",
            "namespace",
            "name",
            "raw_named_kind",
            "category",
            "direct_occurrences",
            "expanded_occurrences",
            "by_value_occurrences",
            "pointer_occurrences",
            "max_pointer_depth",
            "safe_interface_occurrences",
            "unsafe_interface_occurrences",
            "layout_facts",
            "x64",
            "i686",
            "arm64",
        ])
        .map_err(|error| error.to_string())?;
    for value in values {
        writer
            .write_record([
                value.identity_key.clone(),
                value.identity_kind.clone(),
                value.containing_path.clone().unwrap_or_default(),
                value.namespace.clone(),
                value.name.clone(),
                value.raw_named_kind.clone(),
                value.category.clone(),
                value.direct_occurrences.to_string(),
                value.expanded_occurrences.to_string(),
                value.by_value_occurrences.to_string(),
                value.pointer_occurrences.to_string(),
                value.max_pointer_depth.to_string(),
                value.safe_interface_occurrences.to_string(),
                value.unsafe_interface_occurrences.to_string(),
                json_cell(&value.layout_facts)?,
                inventory_target_cell(&value.targets["x64"])?,
                inventory_target_cell(&value.targets["i686"])?,
                inventory_target_cell(&value.targets["arm64"])?,
            ])
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

fn write_shape_csv(path: PathBuf, values: &[TypeShapeInventory]) -> Result<(), String> {
    let file = fs::File::create(&path)
        .map_err(|error| format!("Failed to create {}: {error}", path.display()))?;
    let mut writer = csv::Writer::from_writer(file);
    writer
        .write_record([
            "canonical_signature",
            "category",
            "direct_occurrences",
            "expanded_occurrences",
            "uses",
            "max_pointer_depth",
            "constness",
            "array_relations",
            "named_identities",
            "x64",
            "i686",
            "arm64",
        ])
        .map_err(|error| error.to_string())?;
    for value in values {
        writer
            .write_record([
                value.canonical_signature.clone(),
                value.category.clone(),
                value.direct_occurrences.to_string(),
                value.expanded_occurrences.to_string(),
                json_cell(&value.uses)?,
                value.max_pointer_depth.to_string(),
                json_cell(&value.constness)?,
                json_cell(&value.array_relations)?,
                json_cell(&value.named_identities)?,
                inventory_target_cell(&value.targets["x64"])?,
                inventory_target_cell(&value.targets["i686"])?,
                inventory_target_cell(&value.targets["arm64"])?,
            ])
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

fn write_definitions_csv(
    path: PathBuf,
    values: &[MetadataDefinitionInventory],
) -> Result<(), String> {
    let file = fs::File::create(&path)
        .map_err(|error| format!("Failed to create {}: {error}", path.display()))?;
    let mut writer = csv::Writer::from_writer(file);
    writer
        .write_record([
            "token",
            "namespace",
            "name",
            "full_name",
            "entity_kind",
            "enclosing_type",
            "reachable_from_eligible_com_signatures",
        ])
        .map_err(|error| error.to_string())?;
    for value in values {
        writer
            .write_record([
                format!("0x{:08X}", value.token),
                value.namespace.clone(),
                value.name.clone(),
                value.full_name.clone(),
                value.entity_kind.clone(),
                value.enclosing_type.clone().unwrap_or_default(),
                value.reachable_from_eligible_com_signatures.to_string(),
            ])
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

fn write_markdown(
    path: PathBuf,
    values: &[NamedTypeInventory],
    summary: &CapabilitySummary,
) -> Result<(), String> {
    let mut output = String::new();
    output.push_str("# Classic COM named type capability appendix\n\n");
    output.push_str("Generated; do not edit manually.\n\n");
    output.push_str(&format!(
        "- Metadata: `{}`\n- SHA-256: `{}`\n- Regenerate: `{}`\n\n",
        summary.metadata.label, summary.metadata.sha256, summary.regeneration_command
    ));
    for category in CATEGORIES {
        output.push_str(&format!("## {category}\n\n"));
        output.push_str("| Identity key | Identity kind | Containing path | Raw kind | Occurrences | Safe uses | x64 | i686 | ARM64 |\n");
        output.push_str("| --- | --- | --- | --- | ---: | ---: | --- | --- | --- |\n");
        for value in values.iter().filter(|value| value.category == category) {
            output.push_str(&format!(
                "| `{}` | {} | `{}` | {} | {} | {} | {} | {} | {} |\n",
                value.identity_key.replace('|', "\\|"),
                value.identity_kind,
                value
                    .containing_path
                    .as_deref()
                    .unwrap_or("")
                    .replace('|', "\\|"),
                value.raw_named_kind,
                value.expanded_occurrences,
                value.safe_interface_occurrences,
                value.targets["x64"].classification.key(),
                value.targets["i686"].classification.key(),
                value.targets["arm64"].classification.key(),
            ));
        }
        output.push('\n');
    }
    fs::write(&path, output).map_err(|error| format!("Failed to write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::com_metadata::{
        InterfaceMeta, MethodMeta, ParamDirection, ParamMeta, RawArrayRelation, RawComParam,
        RawCountUnit, RawElementOwnership, RawInterfaceReplacementContract,
        RawInterfaceReplacementSemantics, RawStringPointerArray,
    };

    fn raw(native_type: RawNativeType, pointer_depth: usize) -> RawComType {
        RawComType {
            native_type,
            underlying: None,
            pointer_depth,
            constness: RawConstness::Unspecified,
        }
    }

    fn raw_param(name: &str, typ: RawComType, direction: RawParamDirection) -> RawComParam {
        RawComParam {
            name: name.into(),
            typ,
            direction,
            optional: false,
            const_attribute: false,
            native_array: None,
            string_pointer_array: None,
            free_with: None,
            safe_array_evidence: None,
            exact_interface_output: None,
        }
    }

    fn raw_method(params: Vec<RawComParam>) -> RawComMethod {
        RawComMethod {
            declaring_namespace: "Tests".into(),
            declaring_interface: "ITest".into(),
            declaring_iid: "00000000-0000-0000-c000-000000000046".into(),
            metadata_name: "Call".into(),
            projected_name: "call".into(),
            vtable_index: 3,
            params,
            return_type: raw(RawNativeType::I32, 0),
            semantic_hresult: None,
            enumerator_next: None,
            exact_contract: None,
            interface_replacement_contracts: Vec::new(),
            output_ownership_contracts: Vec::new(),
            exact_null_input_contracts: Vec::new(),
            exact_parameter_direction_contracts: Vec::new(),
            exact_interface_output_call: None,
            safe_array_contract_error: None,
        }
    }

    fn raw_interface(raw_methods: Option<Vec<RawComMethod>>) -> ComInterfaceMeta {
        let method_count = raw_methods.as_ref().map_or(0, Vec::len);
        ComInterfaceMeta {
            interface: InterfaceMeta {
                name: "ITest".into(),
                namespace: "Tests".into(),
                iid: "00000000-0000-0000-c000-000000000046".into(),
                methods: (0..method_count)
                    .map(|_| MethodMeta {
                        name: "Call".into(),
                        vtable_index: 3,
                        params: vec![ParamMeta {
                            name: "value".into(),
                            typ: crate::types::TypeMeta::I32,
                            direction: ParamDirection::In,
                        }],
                        ..MethodMeta::default()
                    })
                    .collect(),
                ..InterfaceMeta::default()
            },
            base_offset: 3,
            is_iunknown_rooted: true,
            base_chain: Vec::new(),
            base_iids: Vec::new(),
            coclass_clsid: None,
            coclass_name: None,
            own_methods_start: 3,
            referenced_enums: Vec::new(),
            raw_referenced_enums: Some(Vec::new()),
            raw_methods,
        }
    }

    #[test]
    fn method_capabilities_inherit_missing_interface_identity_and_root_blockers() {
        for iid in ["", "00000000-0000-0000-0000-000000000000"] {
            let mut interface = raw_interface(Some(vec![raw_method(Vec::new())]));
            interface.interface.iid = iid.into();
            let methods = classify_interface_methods(&interface).unwrap();
            assert!(methods[0].targets.values().all(|target| {
                target.classification == RawClassification::RawRuntimeBlocked
                    && target
                        .blocker_reasons
                        .contains(&"missing_interface_iid".to_string())
            }));
        }

        let mut interface = raw_interface(Some(vec![raw_method(Vec::new())]));
        interface.is_iunknown_rooted = false;
        let methods = classify_interface_methods(&interface).unwrap();
        assert!(methods[0].targets.values().all(|target| {
            target.classification == RawClassification::RawRuntimeBlocked
                && target
                    .blocker_reasons
                    .contains(&"missing_interface_root".to_string())
                && target
                    .blocker_reasons
                    .contains(&"not_addressable".to_string())
        }));

        let mut interface = raw_interface(Some(vec![raw_method(Vec::new())]));
        interface.interface.namespace = "Windows.Win32.UI.Controls.RichEdit".into();
        interface.interface.name = "ITextHost2".into();
        let methods = classify_interface_methods(&interface).unwrap();
        assert!(methods[0].targets.values().all(|target| {
            target.classification == RawClassification::RawRuntimeBlocked
                && target
                    .blocker_reasons
                    .contains(&"not_addressable".to_string())
                && !target
                    .blocker_reasons
                    .contains(&"missing_interface_root".to_string())
        }));
    }

    #[test]
    fn typed_interface_inout_requires_exact_replacement_evidence() {
        let interface_type = |pointer_depth| {
            raw(
                RawNativeType::Named {
                    namespace: "Tests".into(),
                    name: "IValue".into(),
                    kind: RawNamedKind::Interface,
                    iid: Some("10000000-0000-0000-c000-000000000046".into()),
                    layout: None,
                },
                pointer_depth,
            )
        };
        let mut method = raw_method(vec![raw_param(
            "value",
            interface_type(1),
            RawParamDirection::InOut,
        )]);
        assert_eq!(
            parameter_manual_reasons(&method, 0).unwrap(),
            vec!["missing_interface_replacement_contract"]
        );
        assert!(
            analyze_method(&method, CensusTarget::X64)
                .manual
                .contains("missing_interface_replacement_contract")
        );

        for (depth, direction) in [
            (0, RawParamDirection::InOut),
            (2, RawParamDirection::InOut),
            (1, RawParamDirection::In),
            (1, RawParamDirection::Out),
        ] {
            let ordinary = raw_method(vec![raw_param("value", interface_type(depth), direction)]);
            assert!(
                !parameter_manual_reasons(&ordinary, 0)
                    .unwrap()
                    .contains(&"missing_interface_replacement_contract".into())
            );
        }
        let output = raw_param("value", interface_type(1), RawParamDirection::Out);
        let layouts = parameter_pointee_layouts(&output);
        assert_eq!(
            layouts,
            BTreeMap::from([
                (
                    "arm64".into(),
                    Some(RawPointeeLayout {
                        size: 8,
                        alignment: 8,
                    }),
                ),
                (
                    "i686".into(),
                    Some(RawPointeeLayout {
                        size: 4,
                        alignment: 4,
                    }),
                ),
                (
                    "x64".into(),
                    Some(RawPointeeLayout {
                        size: 8,
                        alignment: 8,
                    }),
                ),
            ])
        );

        method
            .interface_replacement_contracts
            .push(RawInterfaceReplacementContract {
                parameter_index: 0,
                semantics: RawInterfaceReplacementSemantics::PreservesOldReturnsOwnedNew,
                evidence: crate::com_metadata::RawEvidence::exact_registry(
                    "com.ownership.entry.tests.ifoo.00000000000000000000000000000000.replace.slot-3.v1",
                    crate::contract_registry::ExactFamilyId::Ownership,
                    crate::contract_registry::ContractKind::Ownership,
                    "synthetic exact replacement ownership",
                    "test://typed-interface-replacement",
                ),
            });
        assert!(
            !parameter_manual_reasons(&method, 0)
                .unwrap()
                .contains(&"missing_interface_replacement_contract".into())
        );
    }

    #[test]
    fn metadata_set_fingerprint_is_order_independent_complete_and_path_free() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join(format!("metadata-set-test-{}", std::process::id()));
        if directory.exists() {
            fs::remove_dir_all(&directory).unwrap();
        }
        fs::create_dir_all(&directory).unwrap();
        let primary = directory.join("Primary.winmd");
        let sibling = directory.join("Sibling.winmd");
        let reference = directory.join("Reference.winmd");
        fs::write(&primary, b"primary").unwrap();
        fs::write(&sibling, b"sibling").unwrap();
        fs::write(&reference, b"reference").unwrap();

        let first = metadata_set_identity_for_paths(&format!(
            "{};{};{};{}",
            primary.display(),
            sibling.display(),
            reference.display(),
            primary.display()
        ))
        .unwrap();
        let reordered = metadata_set_identity_for_paths(&format!(
            "{};{};{}",
            reference.display(),
            primary.display(),
            sibling.display()
        ))
        .unwrap();
        assert_eq!(first.set_sha256, reordered.set_sha256);
        assert_eq!(first.files, reordered.files);
        assert_eq!(first.files.len(), 3);
        assert_eq!(
            first
                .files
                .iter()
                .map(|file| file.file.as_str())
                .collect::<Vec<_>>(),
            vec!["Primary.winmd", "Reference.winmd", "Sibling.winmd"]
        );
        let serialized = serde_json::to_string(&serde_json::json!({
            "setSha256": first.set_sha256.clone(),
            "files": first.files.clone(),
        }))
        .unwrap();
        assert!(!serialized.contains(directory.to_string_lossy().as_ref()));

        fs::write(&reference, b"reference-mutated").unwrap();
        let mutated = metadata_set_identity_for_paths(&format!(
            "{};{};{}",
            primary.display(),
            sibling.display(),
            reference.display()
        ))
        .unwrap();
        assert_ne!(first.set_sha256, mutated.set_sha256);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn metadata_set_records_a_custom_named_defining_file_when_identifiable() {
        let Ok(official) = std::env::var("DYNWINRT_WIN32_WINMD") else {
            return;
        };
        if !Path::new(&official).is_file() {
            return;
        }
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join(format!(
                "metadata-defining-file-test-{}",
                std::process::id()
            ));
        if directory.exists() {
            fs::remove_dir_all(&directory).unwrap();
        }
        fs::create_dir_all(&directory).unwrap();
        let contoso = directory.join("Contoso.Contracts.winmd");
        fs::copy(official, &contoso).unwrap();
        use std::io::Write as _;
        fs::OpenOptions::new()
            .append(true)
            .open(&contoso)
            .unwrap()
            .write_all(b"Contoso metadata fixture")
            .unwrap();

        let identity = metadata_set_identity_for_paths(contoso.to_str().unwrap()).unwrap();
        let defining = identity
            .defining_file("Windows.Win32.Media.MediaFoundation", "MFASYNCRESULT")
            .unwrap();
        assert_eq!(defining.file, "Contoso.Contracts.winmd");
        assert_eq!(defining.package, "unknown");
        assert_eq!(defining.version, "unknown");
        assert_eq!(identity.files, vec![defining]);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn raw_native_variants_and_pointer_depths_classify_deterministically() {
        for native in [
            RawNativeType::Void,
            RawNativeType::Bool,
            RawNativeType::I8,
            RawNativeType::U8,
            RawNativeType::I16,
            RawNativeType::U16,
            RawNativeType::I32,
            RawNativeType::U32,
            RawNativeType::I64,
            RawNativeType::U64,
            RawNativeType::F32,
            RawNativeType::F64,
            RawNativeType::Char16,
            RawNativeType::ISize,
            RawNativeType::USize,
            RawNativeType::String,
            RawNativeType::Object,
            RawNativeType::Named {
                namespace: "Tests".into(),
                name: "Interface".into(),
                kind: RawNamedKind::Interface,
                iid: Some("00000000-0000-0000-c000-000000000046".into()),
                layout: None,
            },
            RawNativeType::Named {
                namespace: "Tests".into(),
                name: "Enum".into(),
                kind: RawNamedKind::Enum,
                iid: None,
                layout: None,
            },
            RawNativeType::Named {
                namespace: "Tests".into(),
                name: "Struct".into(),
                kind: RawNamedKind::Struct,
                iid: None,
                layout: None,
            },
            RawNativeType::Named {
                namespace: "Tests".into(),
                name: "Delegate".into(),
                kind: RawNamedKind::Delegate,
                iid: None,
                layout: None,
            },
            RawNativeType::Named {
                namespace: "Tests".into(),
                name: "RuntimeClass".into(),
                kind: RawNamedKind::RuntimeClass,
                iid: Some("00000000-0000-0000-c000-000000000046".into()),
                layout: None,
            },
            RawNativeType::Named {
                namespace: "Tests".into(),
                name: "Unknown".into(),
                kind: RawNamedKind::Unknown,
                iid: None,
                layout: None,
            },
            RawNativeType::Array(Box::new(raw(RawNativeType::U8, 0))),
            RawNativeType::FixedArray {
                element: Box::new(raw(RawNativeType::U16, 0)),
                count: 4,
            },
            RawNativeType::Unknown("test".into()),
        ] {
            for depth in 0..=3 {
                let value = raw(native.clone(), depth);
                let first = canonical_type(&value);
                let second = canonical_type(&value);
                assert_eq!(first, second);
                for target in CensusTarget::ALL {
                    let _ = analyze_type(&value, target, TypeUse::In, &mut Vec::new());
                }
            }
        }
    }

    #[test]
    fn direct_handle_returns_require_explicit_ownership() {
        let mut method = raw_method(Vec::new());
        method.return_type = raw(
            RawNativeType::Named {
                namespace: "Windows.Win32.Foundation".into(),
                name: "HANDLE".into(),
                kind: RawNamedKind::Struct,
                iid: None,
                layout: None,
            },
            0,
        );
        let analysis = analyze_method(&method, CensusTarget::X64);
        assert!(analysis.manual.contains("missing_handle_ownership"));
        assert_eq!(analysis.lifecycle.cleanup, CleanupAvailability::Unknown);
    }

    #[test]
    fn known_guid_precedes_unknown_named_kind_for_all_directions() {
        let guid = |depth| {
            raw(
                RawNativeType::Named {
                    namespace: "System".into(),
                    name: "Guid".into(),
                    kind: RawNamedKind::Unknown,
                    iid: None,
                    layout: None,
                },
                depth,
            )
        };
        for usage in [TypeUse::In, TypeUse::Out, TypeUse::InOut, TypeUse::Return] {
            for depth in [0, 1] {
                let analysis =
                    analyze_type(&guid(depth), CensusTarget::X64, usage, &mut Vec::new());
                assert!(analysis.blockers.is_empty(), "{usage:?} depth {depth}");
                assert!(analysis.manual.is_empty(), "{usage:?} depth {depth}");
            }
        }
        assert_eq!(category_for(&guid(0), false), "Guid");
        assert_eq!(scalar_layout(&guid(0), CensusTarget::X64), Some((16, 4)));
    }

    #[test]
    fn directions_arrays_function_pointers_and_manual_cleanup_are_distinct() {
        let mut input = raw_param(
            "input",
            raw(RawNativeType::Array(Box::new(raw(RawNativeType::U8, 0))), 0),
            RawParamDirection::In,
        );
        input.native_array = Some(RawArrayRelation {
            count_param_index: Some(1),
            actual_length_param_index: None,
            unit: RawCountUnit::Elements,
            two_call: false,
            projected_capacity: false,
            constness: Some(RawConstness::Const),
            evidence: Vec::new(),
        });
        input.string_pointer_array = Some(RawStringPointerArray {
            encoding: RawStringEncoding::Utf16,
            pointer_depth: 1,
            constness: RawConstness::Const,
            ownership: RawElementOwnership::Borrowed,
        });
        let output = raw_param(
            "output",
            raw(RawNativeType::Void, 2),
            RawParamDirection::Out,
        );
        let inout = raw_param(
            "inout",
            raw(RawNativeType::U32, 1),
            RawParamDirection::InOut,
        );
        let callback = raw_param(
            "callback",
            raw(
                RawNativeType::Named {
                    namespace: "Tests".into(),
                    name: "Callback".into(),
                    kind: RawNamedKind::Delegate,
                    iid: None,
                    layout: None,
                },
                0,
            ),
            RawParamDirection::In,
        );
        let alias_output = raw_param(
            "alias_output",
            RawComType {
                native_type: RawNativeType::Named {
                    namespace: "Windows.Win32.Foundation".into(),
                    name: "PWSTR".into(),
                    kind: RawNamedKind::Struct,
                    iid: None,
                    layout: None,
                },
                underlying: Some(Box::new(raw(RawNativeType::Char16, 1))),
                pointer_depth: 1,
                constness: RawConstness::Mutable,
            },
            RawParamDirection::Out,
        );
        let handle_output = raw_param(
            "handle_output",
            RawComType {
                native_type: RawNativeType::Named {
                    namespace: "Windows.Win32.Foundation".into(),
                    name: "HANDLE".into(),
                    kind: RawNamedKind::Struct,
                    iid: None,
                    layout: None,
                },
                underlying: Some(Box::new(raw(RawNativeType::Void, 1))),
                pointer_depth: 1,
                constness: RawConstness::Mutable,
            },
            RawParamDirection::Out,
        );
        let analysis = analyze_method(
            &raw_method(vec![
                input,
                output,
                inout,
                callback,
                alias_output,
                handle_output,
            ]),
            CensusTarget::X64,
        );
        assert!(analysis.manual.contains("missing_output_ownership"));
        assert!(analysis.manual.contains("missing_allocator"));
        assert!(analysis.manual.contains("missing_handle_ownership"));
        assert!(analysis.lifecycle.requires_external_pointer_or_callback);
        assert_eq!(
            effective_pointer_depth(&RawComType {
                native_type: RawNativeType::Named {
                    namespace: "Windows.Win32.Foundation".into(),
                    name: "PWSTR".into(),
                    kind: RawNamedKind::Struct,
                    iid: None,
                    layout: None,
                },
                underlying: Some(Box::new(raw(RawNativeType::Char16, 1))),
                pointer_depth: 1,
                constness: RawConstness::Mutable,
            }),
            2
        );
    }

    #[test]
    fn target_aggregate_gates_and_manual_pointer_contracts_are_closed() {
        let odd = RawNativeLayoutSet {
            recursive: false,
            variants: vec![RawNativeLayout {
                architectures: 0b111,
                kind: RawLayoutKind::Sequential,
                packing: RawPacking::Default,
                declared_size: Some(3),
                fields: vec![RawNativeField {
                    name: "bytes".into(),
                    typ: raw(RawNativeType::U8, 0),
                    explicit_offset: None,
                    fixed_count: Some(3),
                    bitfield: false,
                    flexible_array: false,
                }],
                is_union: false,
            }],
        };
        let odd_type = raw(
            RawNativeType::Named {
                namespace: "Tests".into(),
                name: "Odd".into(),
                kind: RawNamedKind::Struct,
                iid: None,
                layout: Some(Box::new(odd)),
            },
            0,
        );
        assert!(
            analyze_type(&odd_type, CensusTarget::X64, TypeUse::In, &mut Vec::new())
                .blockers
                .contains("win64_irregular_aggregate_copy")
        );
        assert!(
            analyze_type(&odd_type, CensusTarget::I686, TypeUse::In, &mut Vec::new())
                .blockers
                .is_empty()
        );

        let opaque = raw(
            RawNativeType::Named {
                namespace: "Tests".into(),
                name: "Opaque".into(),
                kind: RawNamedKind::Unknown,
                iid: None,
                layout: None,
            },
            2,
        );
        let analysis = analyze_type(&opaque, CensusTarget::X64, TypeUse::Out, &mut Vec::new());
        assert!(analysis.blockers.is_empty());
        assert!(analysis.manual.contains("external_pointee_storage"));
        assert!(analysis.manual.contains("pointee_unknown_native_type"));
    }

    #[test]
    fn pointer_pointee_layout_and_nested_ownership_are_recursive() {
        let complete_layout = RawNativeLayoutSet {
            recursive: false,
            variants: vec![RawNativeLayout {
                architectures: 0b111,
                kind: RawLayoutKind::Sequential,
                packing: RawPacking::Default,
                declared_size: Some(4),
                fields: vec![RawNativeField {
                    name: "value".into(),
                    typ: raw(RawNativeType::U32, 0),
                    explicit_offset: None,
                    fixed_count: None,
                    bitfield: false,
                    flexible_array: false,
                }],
                is_union: false,
            }],
        };
        let complete = raw(
            RawNativeType::Named {
                namespace: "Tests".into(),
                name: "Complete".into(),
                kind: RawNamedKind::Struct,
                iid: None,
                layout: Some(Box::new(complete_layout)),
            },
            1,
        );
        let analysis = analyze_type(&complete, CensusTarget::X64, TypeUse::Out, &mut Vec::new());
        assert!(analysis.blockers.is_empty());
        assert!(analysis.manual.is_empty());

        let incomplete = raw(
            RawNativeType::Named {
                namespace: "Tests".into(),
                name: "Incomplete".into(),
                kind: RawNamedKind::Struct,
                iid: None,
                layout: None,
            },
            1,
        );
        assert!(
            analyze_type(&incomplete, CensusTarget::X64, TypeUse::In, &mut Vec::new())
                .manual
                .contains("external_pointee_storage")
        );
        assert!(
            analyze_type(
                &incomplete,
                CensusTarget::X64,
                TypeUse::Out,
                &mut Vec::new()
            )
            .blockers
            .contains("incomplete_pointee_layout_for_storage")
        );

        let nested_pointer = RawNativeLayoutSet {
            recursive: false,
            variants: vec![RawNativeLayout {
                architectures: 0b111,
                kind: RawLayoutKind::Sequential,
                packing: RawPacking::Default,
                declared_size: None,
                fields: vec![RawNativeField {
                    name: "nested".into(),
                    typ: raw(RawNativeType::Void, 1),
                    explicit_offset: None,
                    fixed_count: None,
                    bitfield: false,
                    flexible_array: false,
                }],
                is_union: false,
            }],
        };
        let nested = raw(
            RawNativeType::Named {
                namespace: "Tests".into(),
                name: "NestedPointer".into(),
                kind: RawNamedKind::Struct,
                iid: None,
                layout: Some(Box::new(nested_pointer)),
            },
            1,
        );
        assert!(
            analyze_type(&nested, CensusTarget::X64, TypeUse::In, &mut Vec::new())
                .manual
                .contains("nested_pointer_lifetime")
        );
    }

    #[test]
    fn shape_keys_split_counted_pointers_and_preserve_fixed_counts() {
        let pointer = raw(RawNativeType::U8, 1);
        let mut shapes = BTreeMap::new();
        let mut named = BTreeMap::new();
        let mut categories = BTreeMap::new();
        collect_type(
            &pointer,
            TypeUse::In,
            None,
            "direction=In",
            "Tests.I.method.plain",
            true,
            false,
            &mut shapes,
            &mut named,
            &mut categories,
            &mut Vec::new(),
        );
        collect_type(
            &pointer,
            TypeUse::In,
            Some("count=1;actual=None;unit=Elements;two_call=false"),
            "direction=In",
            "Tests.I.method.counted",
            true,
            false,
            &mut shapes,
            &mut named,
            &mut categories,
            &mut Vec::new(),
        );
        assert_eq!(shapes.len(), 2);
        assert!(shapes.values().any(|shape| shape.category == "Pointer"));
        assert!(
            shapes
                .values()
                .any(|shape| shape.category == "CountedBuffer")
        );
        let fixed = raw(
            RawNativeType::FixedArray {
                element: Box::new(raw(RawNativeType::U16, 0)),
                count: 7,
            },
            0,
        );
        assert!(canonical_type(&fixed).contains("fixed[7]"));
    }

    #[test]
    fn anonymous_and_case_sensitive_identities_do_not_alias() {
        let canonical = "layout";
        let first = inventory_identity("", "_Anonymous", "use", Some(&"ParentA".into()), canonical);
        let second =
            inventory_identity("", "_Anonymous", "use", Some(&"ParentB".into()), canonical);
        assert_ne!(first.0, second.0);
        let upper = inventory_identity("Tests", "Value", "use", None, canonical);
        let lower = inventory_identity("Tests", "value", "use", None, canonical);
        assert_ne!(upper.0, lower.0);
    }

    #[test]
    fn cleanup_availability_distinguishes_standard_external_and_unknown() {
        let mut standard = raw_param("value", raw(RawNativeType::Void, 2), RawParamDirection::Out);
        standard.free_with = Some(crate::com_metadata::RawFreeWith {
            function: "CoTaskMemFree".into(),
            evidence: crate::com_metadata::RawEvidence::MetadataAttribute("FreeWithAttribute"),
        });
        assert_eq!(
            analyze_method(&raw_method(vec![standard]), CensusTarget::X64)
                .lifecycle
                .cleanup,
            CleanupAvailability::StandardSupported
        );

        let mut external = raw_param("value", raw(RawNativeType::Void, 2), RawParamDirection::Out);
        external.free_with = Some(crate::com_metadata::RawFreeWith {
            function: "CustomFree".into(),
            evidence: crate::com_metadata::RawEvidence::MetadataAttribute("FreeWithAttribute"),
        });
        assert_eq!(
            analyze_method(&raw_method(vec![external]), CensusTarget::X64)
                .lifecycle
                .cleanup,
            CleanupAvailability::KnownExternal
        );

        let unknown = raw_param("value", raw(RawNativeType::U32, 2), RawParamDirection::Out);
        assert_eq!(
            analyze_method(&raw_method(vec![unknown]), CensusTarget::X64)
                .lifecycle
                .cleanup,
            CleanupAvailability::Unknown
        );
    }

    #[test]
    fn data_pointer_buffers_slots_and_known_outputs_are_distinct() {
        let mut caller_buffer = raw_param(
            "buffer",
            raw(RawNativeType::Void, 1),
            RawParamDirection::Out,
        );
        caller_buffer.native_array = Some(RawArrayRelation {
            count_param_index: Some(1),
            actual_length_param_index: None,
            unit: RawCountUnit::Bytes,
            two_call: false,
            projected_capacity: true,
            constness: Some(RawConstness::Mutable),
            evidence: Vec::new(),
        });
        let caller = analyze_method(&raw_method(vec![caller_buffer]), CensusTarget::X64);
        assert!(!caller.manual.contains("missing_output_ownership"));
        assert_eq!(caller.lifecycle.cleanup, CleanupAvailability::NoneRequired);

        for depth in [2, 3] {
            let mut slot = raw_param(
                "slot",
                raw(RawNativeType::Void, depth),
                RawParamDirection::Out,
            );
            slot.optional = true;
            let analysis = analyze_method(&raw_method(vec![slot]), CensusTarget::X64);
            assert!(analysis.manual.contains("missing_output_ownership"));
            assert!(analysis.manual.contains("missing_allocator"));
            assert_eq!(analysis.lifecycle.cleanup, CleanupAvailability::Unknown);
        }

        let interface = raw_param(
            "object",
            raw(
                RawNativeType::Named {
                    namespace: "Tests".into(),
                    name: "IObject".into(),
                    kind: RawNamedKind::Interface,
                    iid: Some("00000000-0000-0000-c000-000000000046".into()),
                    layout: None,
                },
                2,
            ),
            RawParamDirection::Out,
        );
        let interface_analysis = analyze_method(&raw_method(vec![interface]), CensusTarget::X64);
        assert!(!interface_analysis.manual.contains("missing_allocator"));
        assert_eq!(
            interface_analysis.lifecycle.cleanup,
            CleanupAvailability::StandardSupported
        );

        let mut cotask = raw_param(
            "memory",
            raw(RawNativeType::Void, 2),
            RawParamDirection::Out,
        );
        cotask.free_with = Some(crate::com_metadata::RawFreeWith {
            function: "CoTaskMemFree".into(),
            evidence: crate::com_metadata::RawEvidence::MetadataAttribute("FreeWithAttribute"),
        });
        assert_eq!(
            analyze_method(&raw_method(vec![cotask]), CensusTarget::X64)
                .lifecycle
                .cleanup,
            CleanupAvailability::StandardSupported
        );

        let bstr = raw_param(
            "text",
            RawComType {
                native_type: RawNativeType::Named {
                    namespace: "Windows.Win32.Foundation".into(),
                    name: "BSTR".into(),
                    kind: RawNamedKind::Struct,
                    iid: None,
                    layout: None,
                },
                underlying: Some(Box::new(raw(RawNativeType::U16, 1))),
                pointer_depth: 1,
                constness: RawConstness::Mutable,
            },
            RawParamDirection::Out,
        );
        assert_eq!(
            analyze_method(&raw_method(vec![bstr]), CensusTarget::X64)
                .lifecycle
                .cleanup,
            CleanupAvailability::StandardSupported
        );
    }

    #[test]
    fn union_and_struct_target_gates_are_independent() {
        let union = RawNativeLayoutSet {
            recursive: false,
            variants: vec![RawNativeLayout {
                architectures: 0b111,
                kind: RawLayoutKind::Explicit,
                packing: RawPacking::Default,
                declared_size: Some(8),
                fields: vec![RawNativeField {
                    name: "value".into(),
                    typ: raw(RawNativeType::U64, 0),
                    explicit_offset: Some(0),
                    fixed_count: None,
                    bitfield: false,
                    flexible_array: false,
                }],
                is_union: true,
            }],
        };
        let typ = raw(
            RawNativeType::Named {
                namespace: "Tests".into(),
                name: "Union".into(),
                kind: RawNamedKind::Struct,
                iid: None,
                layout: Some(Box::new(union)),
            },
            0,
        );
        assert!(
            analyze_type(&typ, CensusTarget::X64, TypeUse::Return, &mut Vec::new())
                .blockers
                .is_empty()
        );
        assert!(
            analyze_type(&typ, CensusTarget::I686, TypeUse::Return, &mut Vec::new())
                .blockers
                .is_empty()
        );
        assert!(
            analyze_type(&typ, CensusTarget::Arm64, TypeUse::Return, &mut Vec::new())
                .blockers
                .contains("arm64_union_by_value_gate")
        );
        let pointer = RawComType {
            pointer_depth: 1,
            ..typ
        };
        assert!(
            analyze_type(&pointer, CensusTarget::Arm64, TypeUse::In, &mut Vec::new())
                .blockers
                .is_empty()
        );
    }

    #[test]
    fn missing_raw_methods_and_vtable_gaps_fail_closed() {
        let missing = analyze_interface(&raw_interface(None), CensusTarget::X64);
        assert!(
            missing
                .blocker_reasons
                .contains(&"missing_raw_methods".into())
        );

        let mut method = raw_method(Vec::new());
        method.vtable_index = 4;
        let invalid = analyze_interface(&raw_interface(Some(vec![method])), CensusTarget::X64);
        assert!(
            invalid
                .blocker_reasons
                .contains(&"invalid_vtable_slot".into())
        );
    }

    #[test]
    fn serialization_is_byte_deterministic() {
        let value = TargetCapability {
            classification: RawClassification::RawManualContract,
            first_blocker_reason: None,
            blocker_reasons: Vec::new(),
            manual_contract_reasons: vec!["missing_allocator".into()],
            lifecycle: LifecycleFlags {
                cleanup: CleanupAvailability::Unknown,
                ..LifecycleFlags::default()
            },
        };
        assert_eq!(
            serde_json::to_vec_pretty(&value).unwrap(),
            serde_json::to_vec_pretty(&value).unwrap()
        );
    }

    #[test]
    fn csv_json_cells_round_trip_losslessly() {
        let target = TargetCapability {
            classification: RawClassification::RawManualContract,
            first_blocker_reason: None,
            blocker_reasons: Vec::new(),
            manual_contract_reasons: vec![
                "comma,reason".into(),
                "quote\"reason".into(),
                "semicolon;reason".into(),
            ],
            lifecycle: LifecycleFlags {
                cleanup: CleanupAvailability::Unknown,
                requires_external_pointer_or_callback: true,
                ..LifecycleFlags::default()
            },
        };
        let value = InterfaceCapability {
            namespace: "Tests".into(),
            name: "ICsv".into(),
            iid: "iid".into(),
            is_iunknown_rooted: true,
            method_count: 1,
            first_vtable_slot: Some(3),
            last_vtable_slot: Some(3),
            safe_complete: false,
            safe_error: Some("line1\nline2,\"quoted\"".into()),
            evidence_class: None,
            standard_rule_ids: Vec::new(),
            exact_entry_ids: Vec::new(),
            exact_family_ids: Vec::new(),
            exact_contract_kinds: Vec::new(),
            exact_entry_kinds: BTreeMap::new(),
            metadata_attributes: Vec::new(),
            targets: CensusTarget::ALL
                .into_iter()
                .map(|target_name| (target_name.key().into(), target.clone()))
                .collect(),
        };
        let path = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("capability-csv-roundtrip.csv");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let support_path = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("capability-interface-support-roundtrip.csv");
        write_interface_support_csv(support_path.clone(), &[value.clone()]).unwrap();
        let mut support_reader = csv::Reader::from_path(&support_path).unwrap();
        let expected_headers = [
            "namespace",
            "name",
            "iid",
            "safe_complete",
            "evidence_class",
            "first_reason_code",
        ];
        assert_eq!(support_reader.headers().unwrap(), &expected_headers[..]);
        let support_row = support_reader.records().next().unwrap().unwrap();
        assert_eq!(
            support_row.iter().collect::<Vec<_>>(),
            ["Tests", "ICsv", "iid", "false", "", "comma,reason"]
        );
        std::fs::remove_file(support_path).unwrap();

        write_interface_csv(path.clone(), &[value]).unwrap();
        let mut reader = csv::Reader::from_path(&path).unwrap();
        let row = reader.records().next().unwrap().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row[15]).unwrap();
        assert_eq!(
            parsed[3],
            serde_json::to_value(&target.manual_contract_reasons).unwrap()
        );
        std::fs::remove_file(path).unwrap();

        let named_path = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("capability-named-csv-roundtrip.csv");
        let named = NamedTypeInventory {
            identity_key: "anonymous::Parent::field".into(),
            identity_kind: "anonymous_nested_record".into(),
            containing_path: Some("Parent".into()),
            namespace: String::new(),
            name: "_Anonymous".into(),
            raw_named_kind: "Struct".into(),
            category: "NativeStruct".into(),
            direct_occurrences: 1,
            expanded_occurrences: 2,
            by_value_occurrences: 1,
            pointer_occurrences: 1,
            max_pointer_depth: 1,
            safe_interface_occurrences: 0,
            unsafe_interface_occurrences: 1,
            layout_facts: vec![LayoutFact {
                architectures: 7,
                kind: "Explicit,\"quoted\"".into(),
                packing: "Default".into(),
                declared_size: Some(8),
                field_count: 1,
                is_union: true,
                recursive: false,
                x64_size: Some(8),
                x64_alignment: Some(8),
                i686_size: Some(8),
                i686_alignment: Some(8),
                arm64_size: Some(8),
                arm64_alignment: Some(8),
            }],
            targets: CensusTarget::ALL
                .into_iter()
                .map(|target_name| (target_name.key().into(), target.clone()))
                .collect(),
        };
        write_named_csv(named_path.clone(), &[named]).unwrap();
        let named_row = csv::Reader::from_path(&named_path)
            .unwrap()
            .records()
            .next()
            .unwrap()
            .unwrap();
        let layout: serde_json::Value = serde_json::from_str(&named_row[14]).unwrap();
        assert_eq!(layout[0]["field_count"], 1);
        std::fs::remove_file(named_path).unwrap();

        let shape_path = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("capability-shape-csv-roundtrip.csv");
        let shape = TypeShapeInventory {
            canonical_signature: "fixed[7](named(A,B;layout=[x,\"y\"]))".into(),
            category: "CountedBuffer".into(),
            direct_occurrences: 1,
            expanded_occurrences: 3,
            uses: BTreeMap::from([("in,out".into(), 1)]),
            max_pointer_depth: 2,
            constness: vec!["Const,\"mixed\"".into()],
            array_relations: vec!["count=1;actual=2".into()],
            named_identities: vec!["A,B".into()],
            targets: CensusTarget::ALL
                .into_iter()
                .map(|target_name| (target_name.key().into(), target.clone()))
                .collect(),
        };
        write_shape_csv(shape_path.clone(), &[shape]).unwrap();
        let shape_row = csv::Reader::from_path(&shape_path)
            .unwrap()
            .records()
            .next()
            .unwrap()
            .unwrap();
        let relations: serde_json::Value = serde_json::from_str(&shape_row[7]).unwrap();
        assert_eq!(relations[0], "count=1;actual=2");
        std::fs::remove_file(shape_path).unwrap();
    }

    #[test]
    fn official_metadata_capability_totals_are_stable_when_configured() {
        let Some(winmd) = std::env::var("DYNWINRT_WIN32_WINMD")
            .ok()
            .filter(|path| Path::new(path).exists())
        else {
            return;
        };
        let bytes = fs::read(&winmd).unwrap();
        let hash = format!("{:X}", Sha256::digest(&bytes));
        if hash != OFFICIAL_METADATA_SHA256 {
            return;
        }
        let mut interfaces = crate::com_metadata::parse_all_com_interfaces(&winmd)
            .unwrap()
            .into_iter()
            .filter(is_eligible_interface)
            .collect::<Vec<_>>();
        interfaces.sort_by(|left, right| interface_identity(left).cmp(&interface_identity(right)));
        let open_namespace = interfaces
            .iter()
            .find(|interface| {
                interface.interface.namespace == "Windows.Win32.System.Wmi"
                    && interface.interface.name == "IWbemServices"
            })
            .and_then(|interface| interface.raw_methods.as_ref())
            .and_then(|methods| {
                methods
                    .iter()
                    .find(|method| method.metadata_name == "OpenNamespace")
            })
            .expect("official IWbemServices::OpenNamespace");
        for parameter_name in ["ppWorkingNamespace", "ppResult"] {
            let output_index = open_namespace
                .params
                .iter()
                .position(|parameter| parameter.name == parameter_name)
                .expect("OpenNamespace output parameter");
            assert_eq!(
                open_namespace.params[output_index].direction,
                RawParamDirection::Out
            );
            assert!(
                open_namespace.params[output_index]
                    .exact_interface_output
                    .is_some()
            );
            assert!(
                !parameter_manual_reasons(open_namespace, output_index)
                    .unwrap()
                    .contains(&"missing_interface_replacement_contract".into())
            );
        }
        let report = build_report(
            &interfaces,
            &winmd,
            MetadataIdentity {
                package: "Microsoft.Windows.SDK.Win32Metadata".into(),
                version: OFFICIAL_METADATA_VERSION.into(),
                file: "Windows.Win32.winmd".into(),
                sha256: hash,
                label: "official-test".into(),
            },
            interfaces.len(),
            super::super::typedef_inventory::read_typedefs(Path::new(&winmd)).unwrap(),
            crate::meta::load_index(&winmd).unwrap().all().count(),
        )
        .unwrap();
        assert_eq!(report.summary.eligible_interfaces, 7_929);
        assert_eq!(report.summary.safe_complete, 5_692);
        assert_eq!(report.summary.safe_evidence.safe_complete, 5_692);
        assert_eq!(report.summary.safe_evidence.standard_derived, 5_336);
        assert_eq!(report.summary.safe_evidence.exact_registry_dependent, 356);
        assert_eq!(
            report.summary.safe_evidence.standard_derived
                + report.summary.safe_evidence.exact_registry_dependent,
            report.summary.safe_complete
        );
        assert_eq!(
            report.summary.safe_evidence.metadata_fact_occurrences,
            5_976
        );
        assert_eq!(
            report.summary.safe_evidence.com_standard_fact_occurrences,
            26_119
        );
        assert_eq!(report.summary.safe_evidence.registered_exact_entries, 496);
        assert_eq!(
            report.summary.safe_evidence.metadata_matched_exact_entries,
            496
        );
        assert_eq!(
            report.summary.safe_evidence.safe_consumed_exact_entries,
            405
        );
        assert_eq!(
            report
                .summary
                .safe_evidence
                .exact_entry_interface_dependencies,
            656
        );
        assert_eq!(
            report
                .summary
                .safe_evidence
                .exact_family_interface_dependencies,
            405
        );
        assert_eq!(
            report.summary.safe_evidence.by_contract_kind,
            BTreeMap::from([
                ("borrowed-handle".into(), 54),
                ("bounded-two-call".into(), 16),
                ("compound-dispatch".into(), 1),
                ("conditional-output".into(), 7),
                ("counted-buffer".into(), 16),
                ("enumerator-next".into(), 74),
                ("flag-selected-buffer".into(), 3),
                ("null-input".into(), 2),
                ("ownership".into(), 173),
                ("parameter-direction".into(), 45),
                ("safearray".into(), 263),
                ("semantic-hresult".into(), 2),
            ])
        );
        assert_eq!(
            report.summary.safe_evidence.by_family_id["com.ownership.v1"],
            117
        );
        assert_eq!(
            report.summary.safe_evidence.by_family_id["windows.borrowed-hwnd-output.v1"],
            45
        );
        assert_eq!(
            report.summary.safe_evidence.by_family_id["com.sequential-stream-buffer.v1"],
            7
        );
        assert_eq!(
            report.summary.safe_evidence.by_family_id["automation.idispatch-invoke.v1"],
            1
        );
        assert_eq!(report.summary.safe_evidence.by_entry_id.len(), 405);
        assert!(
            report
                .summary
                .safe_evidence
                .by_entry_id
                .keys()
                .all(|id| crate::contract_registry::valid_exact_entry_id(id))
        );
        assert_eq!(
            report.summary.safe_evidence.by_standard_rule_id["com.automation.bstr-output-owned-sysfreestring.v1"],
            1_231
        );
        assert_eq!(
            report.summary.safe_evidence.by_standard_rule_id["com.automation.bstr-replacement.v1"],
            99
        );
        assert_eq!(
            report.summary.safe_evidence.by_standard_rule_id["com.enumerator-next.generic.v1"],
            25
        );
        assert!(
            report
                .summary
                .safe_evidence
                .by_entry_id
                .contains_key(crate::contract_registry::WMI_OPEN_NAMESPACE_ENTRY_ID),
            "safe IWbemServices must retain its exact conditional-output dependency"
        );
        assert!(
            report
                .summary
                .safe_evidence
                .exact_entry_status
                .values()
                .all(|entry| entry.registered && entry.metadata_matched)
        );
        assert_eq!(
            report
                .summary
                .safe_evidence
                .exact_entry_status
                .values()
                .filter(|entry| entry.safe_consumed)
                .count(),
            405
        );
        let status = &report.summary.safe_evidence.exact_entry_status;
        assert_eq!(
            status
                .values()
                .filter(|entry| entry.family_id == "windows.borrowed-hwnd-output.v1")
                .count(),
            22
        );
        assert_eq!(
            status
                .values()
                .filter(|entry| entry.family_id == "com.enumerator-next-exception.v1")
                .count(),
            73
        );
        assert_eq!(
            status
                .values()
                .filter(|entry| entry.family_id == "com.sequential-stream-buffer.v1")
                .count(),
            2
        );
        assert_eq!(
            status
                .values()
                .filter(|entry| entry.family_id == "automation.idispatch-invoke.v1")
                .count(),
            1
        );
        assert!(status.values().all(|entry| {
            entry.source_fingerprint.len() == 64
                && entry
                    .source_fingerprint
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
        }));
        assert!(
            report
                .interfaces
                .iter()
                .all(|interface| { interface.safe_complete == interface.evidence_class.is_some() })
        );
        let standard = report
            .interfaces
            .iter()
            .find(|interface| interface.name == "IDMLDebugDevice")
            .unwrap();
        assert!(matches!(
            standard.evidence_class,
            Some(SafeEvidenceClass::StandardDerived)
        ));
        assert!(standard.exact_entry_ids.is_empty());
        let promoted_audio = report
            .interfaces
            .iter()
            .find(|interface| {
                interface.namespace == "Windows.Win32.Media.Audio"
                    && interface.name == "IAudioSessionControl"
            })
            .unwrap();
        assert!(promoted_audio.safe_complete);
        assert!(matches!(
            promoted_audio.evidence_class,
            Some(SafeEvidenceClass::ExactRegistryDependent)
        ));
        let data_object = report
            .interfaces
            .iter()
            .find(|interface| {
                interface.namespace == "Windows.Win32.System.Com" && interface.name == "IDataObject"
            })
            .unwrap();
        assert!(data_object.safe_complete);
        assert!(matches!(
            data_object.evidence_class,
            Some(SafeEvidenceClass::ExactRegistryDependent)
        ));
        assert!(data_object.exact_entry_ids.contains(
            &"com.ownership.entry.windows-win32-system-com.idataobject.0000010e00000000c000000000000046.setdata.slot-7.v1".into()
        ));
        assert_eq!(
            promoted_audio
                .exact_entry_ids
                .iter()
                .filter(|entry| entry.starts_with("com.ownership.entry."))
                .count(),
            2
        );
        let bstr_standard = report
            .interfaces
            .iter()
            .find(|interface| {
                interface.namespace == "Windows.Win32.Data.Xml.MsXml"
                    && interface.name == "IMXWriter"
            })
            .unwrap();
        assert!(matches!(
            bstr_standard.evidence_class,
            Some(SafeEvidenceClass::StandardDerived)
        ));
        assert!(bstr_standard.exact_entry_ids.is_empty());
        assert!(
            bstr_standard
                .standard_rule_ids
                .contains(&"com.automation.bstr-output-owned-sysfreestring.v1".into())
        );
        let dispatch = report
            .interfaces
            .iter()
            .find(|interface| {
                interface.namespace == "Windows.Win32.System.Com" && interface.name == "IDispatch"
            })
            .unwrap();
        assert!(matches!(
            dispatch.evidence_class,
            Some(SafeEvidenceClass::ExactRegistryDependent)
        ));
        assert!(
            dispatch
                .exact_family_ids
                .contains(&"automation.idispatch-invoke.v1".into())
        );
        assert_eq!(dispatch.exact_entry_ids.len(), 1);
        let fsrm = report
            .interfaces
            .iter()
            .find(|interface| interface.name == "IFsrmFileManagementJob")
            .unwrap();
        assert_eq!(
            fsrm.exact_entry_ids
                .iter()
                .filter(|id| id.starts_with("automation.safearray.entry."))
                .count(),
            7
        );
        assert_eq!(report.summary.metadata_typedefs, 37_310);
        assert_eq!(report.summary.addressable_type_definitions, 35_146);
        assert_eq!(report.all_metadata_definitions.len(), 37_310);
        for target in CensusTarget::ALL {
            let counts = &report.summary.targets[target.key()];
            assert_eq!(
                (
                    counts.safe_incomplete_raw_metadata_complete,
                    counts.safe_incomplete_raw_manual_contract,
                    counts.safe_incomplete_raw_runtime_blocked,
                ),
                (
                    412 - usize::from(target == CensusTarget::I686),
                    1_437 - 23 * usize::from(target == CensusTarget::I686),
                    388 + 24 * usize::from(target == CensusTarget::I686)
                )
            );
        }
        let d2d = report
            .interfaces
            .iter()
            .find(|interface| interface.name == "ID2D1Factory")
            .unwrap();
        assert_ne!(
            d2d.targets["x64"].classification,
            RawClassification::RawMetadataComplete
        );
        assert!(
            d2d.targets["x64"]
                .manual_contract_reasons
                .iter()
                .any(|reason| reason.starts_with("pointee_"))
        );
        let bindptr = report
            .interfaces
            .iter()
            .find(|interface| interface.name == "ITypeComp")
            .unwrap();
        assert_ne!(
            bindptr.targets["x64"].classification,
            RawClassification::RawMetadataComplete
        );
        let winml = report
            .interfaces
            .iter()
            .find(|interface| interface.name == "IWinMLEvaluationContext")
            .unwrap();
        assert_eq!(
            winml.targets["x64"].lifecycle.cleanup,
            CleanupAvailability::Unknown
        );
        assert!(report.interfaces.iter().all(|interface| {
            interface.targets.values().all(|target| {
                !target.manual_contract_reasons.iter().any(|reason| {
                    matches!(
                        reason.as_str(),
                        "missing_allocator" | "missing_output_ownership"
                    )
                }) || target.lifecycle.cleanup == CleanupAvailability::Unknown
            })
        }));
        let direct_input = report
            .interfaces
            .iter()
            .find(|interface| interface.name == "IDirectInputEffect")
            .unwrap();
        assert!(direct_input.targets.values().all(|target| {
            !target
                .blocker_reasons
                .iter()
                .any(|reason| reason.contains("unknown_native_type"))
        }));
        let guid = report
            .named_types
            .iter()
            .find(|value| value.identity_key == "System.Guid")
            .unwrap();
        assert_eq!(guid.category, "Guid");
        assert!(guid.targets.values().all(|target| {
            target.classification != RawClassification::RawRuntimeBlocked
                && !target
                    .blocker_reasons
                    .iter()
                    .any(|reason| reason.contains("unknown_native_type"))
        }));

        let host_malloc = report
            .interfaces
            .iter()
            .find(|interface| interface.name == "IHostMalloc")
            .unwrap();
        assert_eq!(
            host_malloc.targets["x64"].classification,
            RawClassification::RawManualContract
        );
        assert_ne!(
            host_malloc.targets["x64"].lifecycle.cleanup,
            CleanupAvailability::NoneRequired
        );
        let ole_window = report
            .interfaces
            .iter()
            .find(|interface| interface.name == "IOleWindow")
            .unwrap();
        assert_eq!(
            ole_window.targets["x64"].lifecycle.cleanup,
            CleanupAvailability::NoneRequired
        );
        assert!(
            !ole_window.targets["x64"]
                .manual_contract_reasons
                .contains(&"missing_handle_ownership".into())
        );
        for name in [
            "ICreateDeviceAccessAsync",
            "IErrorInfo",
            "IPersistFile",
            "IDispatch",
            "IPropertyStore",
            "IClassFactory",
        ] {
            let interface = report
                .interfaces
                .iter()
                .find(|interface| interface.name == name)
                .unwrap_or_else(|| panic!("{name}"));
            assert!(interface.safe_complete, "{name}");
            assert_eq!(
                interface.targets["x64"].lifecycle.cleanup,
                CleanupAvailability::StandardSupported,
                "{name}"
            );
        }
        let create_device_metadata = interfaces
            .iter()
            .find(|interface| interface.interface.name == "ICreateDeviceAccessAsync")
            .unwrap();
        let get_result = create_device_metadata
            .raw_methods
            .as_ref()
            .unwrap()
            .iter()
            .find(|method| method.metadata_name == "GetResult")
            .unwrap();
        assert!(get_result.params.iter().any(|param| {
            matches!(
                &param.typ.native_type,
                RawNativeType::Named {
                    namespace,
                    name,
                    ..
                } if namespace == "System" && name == "Guid"
            ) && param.typ.pointer_depth == 1
        }));
        assert!(get_result.params.iter().any(|param| {
            matches!(param.typ.native_type, RawNativeType::Void) && param.typ.pointer_depth == 2
        }));
        let taskbar = report
            .interfaces
            .iter()
            .find(|interface| interface.name == "ITaskbarList")
            .unwrap();
        assert!(taskbar.safe_complete);
        assert_eq!(
            taskbar.targets["x64"].lifecycle.cleanup,
            CleanupAvailability::NoneRequired
        );
        assert!(report.interfaces.iter().all(|interface| {
            !interface.safe_complete
                || interface.targets.values().all(|target| {
                    matches!(
                        target.lifecycle.cleanup,
                        CleanupAvailability::NoneRequired | CleanupAvailability::StandardSupported
                    )
                })
        }));

        let external = report
            .named_types
            .iter()
            .filter(|value| value.identity_kind == "external_metadata_reference")
            .map(|value| value.identity_key.as_str())
            .collect::<BTreeSet<_>>();
        for expected in [
            "Windows.Foundation.IPropertyValue",
            "Windows.Graphics.Effects.IGraphicsEffectSource",
            "Windows.UI.Composition.CompositionGraphicsDevice",
            "Windows.UI.Composition.CompositionTexture",
            "Windows.UI.Composition.Desktop.DesktopWindowTarget",
            "Windows.UI.Composition.ICompositionSurface",
        ] {
            assert!(external.contains(expected), "{expected}");
        }
        let definition_names = report
            .all_metadata_definitions
            .iter()
            .map(|value| value.full_name.as_str())
            .collect::<BTreeSet<_>>();
        assert!(report.named_types.iter().all(|value| {
            value.identity_kind != "named_metadata_definition"
                || definition_names.contains(value.identity_key.as_str())
        }));
        assert!(report.interfaces.iter().all(|interface| {
            !interface.safe_complete
                || interface
                    .targets
                    .values()
                    .all(|target| target.classification == RawClassification::RawMetadataComplete)
        }));
    }

    #[test]
    fn categories_always_include_zero_occurrence_entries() {
        let summary = summarize(
            MetadataIdentity {
                package: "test".into(),
                version: "test".into(),
                file: "test".into(),
                sha256: "test".into(),
                label: "test".into(),
            },
            &[],
            CATEGORIES
                .into_iter()
                .map(|category| CategoryCounts {
                    category: category.into(),
                    direct_occurrences: 0,
                    expanded_occurrences: 0,
                    named_types: 0,
                })
                .collect(),
            0,
            0,
            0,
            0,
            0,
            &BTreeMap::new(),
            &BTreeSet::new(),
        );
        assert_eq!(summary.type_categories.len(), CATEGORIES.len());
        assert!(summary.type_categories.iter().all(|category| {
            category.direct_occurrences == 0
                && category.expanded_occurrences == 0
                && category.named_types == 0
        }));
    }
}
