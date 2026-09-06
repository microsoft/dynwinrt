// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MANIFEST_JSON: &str = include_str!("../contracts/classic-com/manifest.json");
const SCHEMA_JSON: &str = include_str!("../contracts/classic-com/schema.json");
const CONDITIONAL_OUTPUTS_JSON: &str =
    include_str!("../contracts/classic-com/conditional-outputs.json");
const OWNERSHIP_OUTPUTS_JSON: &str =
    include_str!("../contracts/classic-com/ownership-outputs.json");
const PINNED_METADATA_PACKAGE: &str = "Microsoft.Windows.SDK.Win32Metadata";
const PINNED_METADATA_VERSION: &str = "71.0.14-preview";
const PINNED_METADATA_SHA256: &str =
    "B64EE4818A7ED9F9D135038D58C51BD08369184D4D5ED428F20E9DE55DF8121D";
#[cfg(test)]
pub(crate) const WMI_OPEN_NAMESPACE_ENTRY_ID: &str = "wmi.conditional-output.entry.windows-win32-system-wmi.iwbemservices.9556dc99828c11cfa37e00aa003240c7.opennamespace.slot-3.v1";

#[derive(Debug, Clone, Copy, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContractKind {
    Ownership,
    ConditionalOutput,
    CountedBuffer,
    BoundedTwoCall,
    BorrowedHandle,
    EnumeratorNext,
    Safearray,
    SemanticHresult,
    CompoundDispatch,
    Hazard,
    NullInput,
    ParameterDirection,
    FlagSelectedBuffer,
}

impl ContractKind {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Ownership => "ownership",
            Self::ConditionalOutput => "conditional-output",
            Self::CountedBuffer => "counted-buffer",
            Self::BoundedTwoCall => "bounded-two-call",
            Self::BorrowedHandle => "borrowed-handle",
            Self::EnumeratorNext => "enumerator-next",
            Self::Safearray => "safearray",
            Self::SemanticHresult => "semantic-hresult",
            Self::CompoundDispatch => "compound-dispatch",
            Self::Hazard => "hazard",
            Self::NullInput => "null-input",
            Self::ParameterDirection => "parameter-direction",
            Self::FlagSelectedBuffer => "flag-selected-buffer",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum EvidenceSourceKind {
    MicrosoftLearn,
    SdkHeader,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub(crate) struct EvidenceDependencies {
    pub metadata_attributes: BTreeSet<String>,
    pub standard_rule_ids: BTreeSet<String>,
    pub exact_entry_ids: BTreeSet<String>,
    pub exact_family_ids: BTreeSet<ExactFamilyId>,
    pub exact_contract_kinds: BTreeSet<ContractKind>,
    pub exact_entry_families: BTreeMap<String, ExactFamilyId>,
    pub exact_entry_kinds: BTreeMap<String, ContractKind>,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum ExactFamilyId {
    SafeArray,
    BorrowedHwndOutput,
    EnumeratorException,
    BoundedTwoCall,
    CountedBuffer,
    Ownership,
    SemanticHresult,
    PrivateDataHazard,
    ConditionalOutput,
    SequentialStreamBuffer,
    DispatchInvoke,
    ReservedNullInput,
    ParameterDirection,
    ShellCommandString,
}

impl ExactFamilyId {
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::SafeArray => "automation.safearray.v1",
            Self::BorrowedHwndOutput => "windows.borrowed-hwnd-output.v1",
            Self::EnumeratorException => "com.enumerator-next-exception.v1",
            Self::BoundedTwoCall => "buffers.bounded-two-call.v1",
            Self::CountedBuffer => "buffers.counted-buffer.v1",
            Self::Ownership => "com.ownership.v1",
            Self::SemanticHresult => "com.semantic-hresult.v1",
            Self::PrivateDataHazard => "graphics.private-data-hazard.v1",
            Self::ConditionalOutput => "wmi.conditional-output.v1",
            Self::SequentialStreamBuffer => "com.sequential-stream-buffer.v1",
            Self::DispatchInvoke => "automation.idispatch-invoke.v1",
            Self::ReservedNullInput => "com.reserved-null-input.v1",
            Self::ParameterDirection => "com.parameter-direction.v1",
            Self::ShellCommandString => "shell.flag-selected-string.v1",
        }
    }

    pub(crate) fn from_id(value: &str) -> Option<Self> {
        [
            Self::SafeArray,
            Self::BorrowedHwndOutput,
            Self::EnumeratorException,
            Self::BoundedTwoCall,
            Self::CountedBuffer,
            Self::Ownership,
            Self::SemanticHresult,
            Self::PrivateDataHazard,
            Self::ConditionalOutput,
            Self::SequentialStreamBuffer,
            Self::DispatchInvoke,
            Self::ReservedNullInput,
            Self::ParameterDirection,
            Self::ShellCommandString,
        ]
        .into_iter()
        .find(|family| family.id() == value)
    }
}

impl<'de> Deserialize<'de> for ExactFamilyId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_id(&value)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown exact family ID '{value}'")))
    }
}

pub(crate) fn exact_method_entry_id(
    family: ExactFamilyId,
    namespace: &str,
    interface: &str,
    iid: &str,
    method: &str,
    slot: usize,
) -> String {
    format!(
        "{}.entry.{}.{}.{}.{}.slot-{slot}.v1",
        family.id().trim_end_matches(".v1"),
        id_component(namespace),
        id_component(interface),
        iid.to_ascii_lowercase().replace('-', ""),
        id_component(method),
    )
}

pub(crate) fn exact_parameter_entry_id(
    family: ExactFamilyId,
    namespace: &str,
    interface: &str,
    iid: &str,
    method: &str,
    slot: usize,
    parameter_index: usize,
    parameter_name: &str,
) -> String {
    format!(
        "{}.param-{parameter_index}-{}.v1",
        exact_method_entry_id(family, namespace, interface, iid, method, slot)
            .trim_end_matches(".v1"),
        id_component(parameter_name),
    )
}

fn id_component(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() {
                (byte as char).to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ComStandardRule {
    IUnknownIdentityRefcount,
    InterfaceInputBorrow,
    TypedInterfaceOutputPlusOne,
    QueryInterfaceOutputPlusOne,
    ActivationOutputPlusOne,
    HresultFailure,
    MatchingStandardCleanup,
    BorrowedHandleNoCleanup,
    BstrOutputOwnershipCleanup,
    BstrReplacement,
    GenericEnumeratorNext,
}

impl ComStandardRule {
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::IUnknownIdentityRefcount => "com.iunknown.identity-refcount.v1",
            Self::InterfaceInputBorrow => "com.interface.input-borrow.v1",
            Self::TypedInterfaceOutputPlusOne => "com.interface.typed-output-plus-one.v1",
            Self::QueryInterfaceOutputPlusOne => "com.query-interface.output-plus-one.v1",
            Self::ActivationOutputPlusOne => "com.activation.output-plus-one.v1",
            Self::HresultFailure => "com.hresult.failure.v1",
            Self::MatchingStandardCleanup => "com.standard-cleanup.matching-allocator.v1",
            Self::BorrowedHandleNoCleanup => "com.handle.borrowed-no-cleanup.v1",
            Self::BstrOutputOwnershipCleanup => "com.automation.bstr-output-owned-sysfreestring.v1",
            Self::BstrReplacement => "com.automation.bstr-replacement.v1",
            Self::GenericEnumeratorNext => "com.enumerator-next.generic.v1",
        }
    }
}

impl EvidenceDependencies {
    pub(crate) fn consume_raw_evidence(&mut self, evidence: &crate::com_metadata::RawEvidence) {
        match evidence {
            crate::com_metadata::RawEvidence::MetadataAttribute(attribute) => {
                self.metadata_attributes.insert((*attribute).into());
            }
            crate::com_metadata::RawEvidence::ComStandard(rule) => {
                self.add_standard(*rule);
            }
            crate::com_metadata::RawEvidence::ExactRegistry {
                entry_id,
                family_id,
                contract_kind,
                ..
            } => {
                self.add_exact(entry_id.clone(), *family_id, *contract_kind);
            }
        }
    }

    pub(crate) fn add_exact(
        &mut self,
        entry_id: String,
        family_id: ExactFamilyId,
        kind: ContractKind,
    ) {
        self.exact_entry_ids.insert(entry_id.clone());
        self.exact_family_ids.insert(family_id);
        self.exact_contract_kinds.insert(kind);
        self.exact_entry_families
            .insert(entry_id.clone(), family_id);
        self.exact_entry_kinds.insert(entry_id, kind);
    }

    pub(crate) fn add_standard(&mut self, rule: ComStandardRule) {
        self.standard_rule_ids.insert(rule.id().into());
    }

    pub(crate) fn validate_exact_ids(&self) -> Result<(), String> {
        for id in &self.exact_entry_ids {
            if !valid_exact_entry_id(id) {
                return Err(format!("Exact evidence entry ID '{id}' is malformed"));
            }
            if !self.exact_entry_families.contains_key(id)
                || !self.exact_entry_kinds.contains_key(id)
            {
                return Err(format!(
                    "Exact evidence entry ID '{id}' has incomplete family/kind provenance"
                ));
            }
        }
        Ok(())
    }
}

pub(crate) fn valid_exact_entry_id(id: &str) -> bool {
    id.ends_with(".v1")
        && id.contains(".entry.")
        && id.contains(".slot-")
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        })
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactEntrySelector {
    pub namespace: String,
    pub interface: String,
    pub iid: String,
    pub method: String,
    pub slot: usize,
    pub parameter: Option<(usize, String)>,
}

impl ExactEntrySelector {
    pub(crate) fn entry_id(&self, family_id: ExactFamilyId) -> String {
        if let Some((index, name)) = &self.parameter {
            exact_parameter_entry_id(
                family_id,
                &self.namespace,
                &self.interface,
                &self.iid,
                &self.method,
                self.slot,
                *index,
                name,
            )
        } else {
            exact_method_entry_id(
                family_id,
                &self.namespace,
                &self.interface,
                &self.iid,
                &self.method,
                self.slot,
            )
        }
    }

    fn key(&self) -> String {
        format!(
            "{}.{}:{}:{}:{}:{}",
            self.namespace,
            self.interface,
            self.iid.to_ascii_lowercase(),
            self.method,
            self.slot,
            self.parameter
                .as_ref()
                .map(|(index, name)| format!("{index}:{name}"))
                .unwrap_or_else(|| "method".into())
        )
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct ExactRegistryEntry {
    pub entry_id: String,
    pub family_id: ExactFamilyId,
    pub contract_kind: ContractKind,
    pub selector: ExactEntrySelector,
    pub source_fingerprint: String,
    pub reason: String,
    pub citation: String,
}

pub(crate) fn validate_exact_entry_catalog(
    entries: impl IntoIterator<Item = ExactRegistryEntry>,
) -> Result<BTreeMap<String, ExactRegistryEntry>, String> {
    let mut catalog = BTreeMap::<String, ExactRegistryEntry>::new();
    let mut selectors = BTreeMap::new();
    for entry in entries {
        if !valid_exact_entry_id(&entry.entry_id) {
            return Err(format!(
                "Exact registry entry ID '{}' is malformed",
                entry.entry_id
            ));
        }
        let expected = entry.selector.entry_id(entry.family_id);
        if entry.entry_id != expected {
            return Err(format!(
                "Exact registry entry ID '{}' does not match selector-derived ID '{expected}'",
                entry.entry_id
            ));
        }
        validate_sha256(
            &entry.source_fingerprint,
            &format!("source fingerprint for '{}'", entry.entry_id),
        )?;
        if entry.reason.is_empty() || entry.citation.is_empty() {
            return Err(format!(
                "Exact registry entry '{}' has incomplete evidence",
                entry.entry_id
            ));
        }
        let selector_key = format!(
            "{}:{}:{}",
            entry.family_id.id(),
            entry.contract_kind.key(),
            entry.selector.key()
        );
        if let Some(existing) = selectors.insert(selector_key.clone(), entry.entry_id.clone())
            && existing != entry.entry_id
        {
            return Err(format!(
                "Exact registry selector collision between '{existing}' and '{}' for {selector_key}",
                entry.entry_id
            ));
        }
        if let Some(existing) = catalog.get(&entry.entry_id) {
            if existing.family_id != entry.family_id
                || existing.contract_kind != entry.contract_kind
                || existing.selector != entry.selector
                || existing.source_fingerprint != entry.source_fingerprint
                || existing.citation != entry.citation
            {
                return Err(format!(
                    "Duplicate exact registry entry ID '{}' has conflicting definitions",
                    entry.entry_id
                ));
            }
        } else {
            catalog.insert(entry.entry_id.clone(), entry);
        }
    }
    Ok(catalog)
}

pub(crate) fn statically_declared_exact_entry_ids() -> Result<BTreeSet<String>, String> {
    let mut ids = BTreeSet::new();
    ids.extend(
        crate::com_safe_array_registry::all_safe_array_evidence()
            .iter()
            .map(crate::com_metadata::RawSafeArrayEvidence::entry_id),
    );
    ids.extend(
        crate::com_borrowed_handle_registry::BorrowedHwndOutputEvidence::entries()
            .iter()
            .map(crate::com_borrowed_handle_registry::BorrowedHwndOutputEvidence::entry_id),
    );
    ids.extend(
        crate::com_enumerator_registry::contracts()
            .iter()
            .filter(|contract| !contract.uses_generic_standard())
            .map(crate::com_enumerator_registry::EnumeratorContract::entry_id),
    );
    ids.extend(
        crate::com_null_input_registry::entries()
            .iter()
            .map(crate::com_null_input_registry::ExactNullInputEvidence::entry_id),
    );
    ids.extend(
        crate::com_parameter_direction_registry::entries()
            .iter()
            .map(crate::com_parameter_direction_registry::ExactOutParameterEvidence::entry_id),
    );
    let registry = REGISTRY
        .get_or_init(load_registry)
        .as_ref()
        .map_err(Clone::clone)?;
    ids.extend(
        registry
            .conditional_outputs
            .iter()
            .map(|entry| entry.entry_id.clone()),
    );
    ids.extend(
        registry
            .ownership_outputs
            .iter()
            .map(|entry| entry.entry_id.clone()),
    );
    ids.extend(
        ADDITIONAL_EXACT_ENTRY_IDS
            .iter()
            .map(|id| (*id).to_string()),
    );
    Ok(ids)
}

const ADDITIONAL_EXACT_ENTRY_IDS: &[&str] = &[
    "automation.idispatch-invoke.entry.windows-win32-system-com.idispatch.0002040000000000c000000000000046.invoke.slot-6.v1",
    "buffers.bounded-two-call.entry.windows-win32-media-mediafoundation.imfattributes.2cd2d921c44744a7a13c4adabfc247e3.getblob.slot-15.v1",
    "buffers.bounded-two-call.entry.windows-win32-storage-imapi.idiscrecorder.85ac9776ca884cf2894e09598c078a41.getrecorderguid.slot-4.param-0-pbyuniqueid.v1",
    "buffers.counted-buffer.entry.windows-win32-storage-imapi.idiscrecorder.85ac9776ca884cf2894e09598c078a41.init.slot-3.param-0-pbyuniqueid.v1",
    "buffers.counted-buffer.entry.windows-win32-storage-packaging-opc.iopcsignaturecustomobject.5d77a19e62c144e7becd45da5ae51a56.getxml.slot-3.param-0-xmlmarkup.v1",
    "buffers.counted-buffer.entry.windows-win32-system-com.itypeinfo.0002040100000000c000000000000046.getnames.slot-7.param-1-rgbstrnames.v1",
    "com.ownership.entry.windows-win32-storage-packaging-opc.iopcsignaturecustomobject.5d77a19e62c144e7becd45da5ae51a56.getxml.slot-3.param-0-xmlmarkup.v1",
    "com.ownership.entry.windows-win32-system-com.idataobject.0000010e00000000c000000000000046.setdata.slot-7.v1",
    "com.ownership.entry.windows-win32-system-com.imalloc.0000000200000000c000000000000046.alloc.slot-3.v1",
    "com.ownership.entry.windows-win32-system-com.imalloc.0000000200000000c000000000000046.didalloc.slot-7.v1",
    "com.ownership.entry.windows-win32-system-com.imalloc.0000000200000000c000000000000046.free.slot-5.v1",
    "com.ownership.entry.windows-win32-system-com.imalloc.0000000200000000c000000000000046.getsize.slot-6.v1",
    "com.ownership.entry.windows-win32-system-com.imalloc.0000000200000000c000000000000046.heapminimize.slot-8.v1",
    "com.ownership.entry.windows-win32-system-com.imalloc.0000000200000000c000000000000046.realloc.slot-4.v1",
    "com.ownership.entry.windows-win32-system-com.ipersistfile.0000010b00000000c000000000000046.getcurfile.slot-8.param-0-ppszfilename.v1",
    "com.ownership.entry.windows-win32-system-com.istream.0000000c00000000c000000000000046.stat.slot-12.v1",
    "com.ownership.entry.windows-win32-system-com-structuredstorage.istorage.0000000b00000000c000000000000046.stat.slot-17.v1",
    "com.ownership.entry.windows-win32-system-ole.iolecache.0000011e00000000c000000000000046.setdata.slot-7.v1",
    "shell.flag-selected-string.entry.windows-win32-ui-shell.icontextmenu.000214e400000000c000000000000046.getcommandstring.slot-5.v1",
    "com.ownership.entry.windows-win32-ui-shell.ifiledialog.42f85136db7e439c85f1e4075d135fc8.getfilename.slot-16.param-0-pszname.v1",
    "com.ownership.entry.windows-win32-ui-shell.ishellitem.43826d1ee71842eebc55a1e261c37bfe.getdisplayname.slot-5.param-1-ppszname.v1",
    "com.ownership.entry.windows-win32-ui-shell.ishelllinka.000214ee00000000c000000000000046.getidlist.slot-4.param-0-ppidl.v1",
    "com.ownership.entry.windows-win32-ui-shell.ishelllinkw.000214f900000000c000000000000046.getidlist.slot-4.param-0-ppidl.v1",
    "com.semantic-hresult.entry.windows-win32-system-com.ipersistfile.0000010b00000000c000000000000046.getcurfile.slot-8.v1",
    "com.semantic-hresult.entry.windows-win32-system-com.idataobject.0000010e00000000c000000000000046.getcanonicalformatetc.slot-6.v1",
    "com.sequential-stream-buffer.entry.windows-win32-system-com.isequentialstream.0c733a302a1c11ceade500aa0044773d.read.slot-3.param-0-pv.v1",
    "com.sequential-stream-buffer.entry.windows-win32-system-com.isequentialstream.0c733a302a1c11ceade500aa0044773d.write.slot-4.param-0-pv.v1",
    "graphics.private-data-hazard.entry.windows-win32-ai-machinelearning-directml.idmlobject.c8263aac9e0c4a2d9b8e007521a3317c.getprivatedata.slot-3.v1",
    "graphics.private-data-hazard.entry.windows-win32-graphics-direct3d10.id3d10device.9b7e4c0f342c4106a19f4f2704f689f0.getprivatedata.slot-66.v1",
    "graphics.private-data-hazard.entry.windows-win32-graphics-direct3d10.id3d10devicechild.9b7e4c00342c4106a19f4f2704f689f0.getprivatedata.slot-4.v1",
    "graphics.private-data-hazard.entry.windows-win32-graphics-direct3d11.id3d11device.db6f6ddbac774e888253819df9bbf140.getprivatedata.slot-34.v1",
    "graphics.private-data-hazard.entry.windows-win32-graphics-direct3d11.id3d11devicechild.1841e5c816b0489bbcc844cfb0d5deae.getprivatedata.slot-4.v1",
    "graphics.private-data-hazard.entry.windows-win32-graphics-direct3d12.id3d12object.c4fec28f79664e959f94f431cb56c3b8.getprivatedata.slot-3.v1",
    "graphics.private-data-hazard.entry.windows-win32-graphics-dxgi.idxgiobject.aec22fb876f346399be028eb43a67a2e.getprivatedata.slot-5.v1",
];

impl EvidenceSourceKind {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::MicrosoftLearn => "microsoft-learn",
            Self::SdkHeader => "sdk-header",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RegistryManifest {
    schema_version: u32,
    files: Vec<RegistryManifestFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RegistryManifestFile {
    path: String,
    kind: ContractKind,
    family_ids: Vec<ExactFamilyId>,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ConditionalOutputFile {
    schema_version: u32,
    contracts: Vec<ConditionalOutputContract>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct OwnershipOutputFile {
    schema_version: u32,
    contracts: Vec<OwnershipOutputContract>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ConditionalOutputContract {
    pub entry_id: String,
    pub family_id: ExactFamilyId,
    pub kind: ContractKind,
    pub reason: String,
    pub selector: ContractSelector,
    pub contract: ConditionalOutputSemantics,
    pub evidence: Vec<EvidenceCitation>,
    pub validated_metadata: Vec<ValidatedMetadata>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct OwnershipOutputContract {
    pub entry_id: String,
    pub family_id: ExactFamilyId,
    pub kind: ContractKind,
    pub reason: String,
    pub selector: ContractSelector,
    pub contract: OutputOwnershipSemantics,
    pub evidence: Vec<EvidenceCitation>,
    pub validated_metadata: Vec<ValidatedMetadata>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ContractSelector {
    pub interface: InterfaceSelector,
    pub declaring_iid: String,
    pub method: String,
    pub absolute_slot: usize,
    pub parameter_count: usize,
    pub parameters: Vec<ParameterSelector>,
    pub source_fingerprint: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct InterfaceSelector {
    pub namespace: String,
    pub name: String,
    pub iid: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ParameterSelector {
    pub index: usize,
    pub name: String,
    pub native_type: String,
    pub pointer_depth: usize,
    pub direction: String,
    pub optional: bool,
    pub constness: String,
    pub const_attribute: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ConditionalOutputSemantics {
    pub public_input_parameter_indices: Vec<usize>,
    pub context_parameter_index: usize,
    pub context_must_be_native_null: bool,
    pub flags_parameter_index: usize,
    pub flags_option_name: String,
    pub synchronous: OutputMode,
    pub semisynchronous: OutputMode,
    pub outputs: Vec<InterfaceOutput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct OutputMode {
    pub flags: i32,
    pub output_parameter_index: Option<usize>,
    pub option_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct InterfaceOutput {
    pub parameter_index: usize,
    pub interface_iid: String,
    pub argument_optional: bool,
    pub nullable_on_success: bool,
    pub ownership: OutputOwnership,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct OutputOwnershipSemantics {
    pub parameter_index: usize,
    pub ownership: OutputAllocationOwnership,
    pub cleanup: OutputAllocationCleanup,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
pub(crate) enum OutputAllocationOwnership {
    #[serde(rename = "cotaskmem-owned")]
    CoTaskMemOwned,
    #[serde(rename = "handle-owned")]
    HandleOwned,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
pub(crate) enum OutputAllocationCleanup {
    CoTaskMemFree,
    DeleteObject,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum OutputOwnership {
    OwnedComPlusOne,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct EvidenceCitation {
    pub kind: EvidenceSourceKind,
    pub url: Option<String>,
    pub file: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ValidatedMetadata {
    pub package: String,
    pub version: String,
    pub sha256: String,
}

#[derive(Debug)]
struct Registry {
    conditional_outputs: Vec<ConditionalOutputContract>,
    ownership_outputs: Vec<OwnershipOutputContract>,
}

static REGISTRY: OnceLock<Result<Registry, String>> = OnceLock::new();

#[cfg(test)]
pub(crate) fn conditional_output_contract(
    id: &str,
) -> Result<&'static ConditionalOutputContract, String> {
    let registry = REGISTRY
        .get_or_init(load_registry)
        .as_ref()
        .map_err(Clone::clone)?;
    registry
        .conditional_outputs
        .iter()
        .find(|entry| entry.entry_id == id)
        .ok_or_else(|| format!("Classic COM contract registry entry '{id}' was not found"))
}

pub(crate) fn conditional_output_contracts() -> Result<&'static [ConditionalOutputContract], String>
{
    let registry = REGISTRY
        .get_or_init(load_registry)
        .as_ref()
        .map_err(Clone::clone)?;
    Ok(&registry.conditional_outputs)
}

pub(crate) fn ownership_output_contracts() -> Result<&'static [OwnershipOutputContract], String> {
    let registry = REGISTRY
        .get_or_init(load_registry)
        .as_ref()
        .map_err(Clone::clone)?;
    Ok(&registry.ownership_outputs)
}

pub(crate) fn validate_registry_usage(consumed_ids: &BTreeSet<String>) -> Result<(), String> {
    let registry = REGISTRY
        .get_or_init(load_registry)
        .as_ref()
        .map_err(Clone::clone)?;
    for entry in &registry.conditional_outputs {
        if !consumed_ids.contains(&entry.entry_id) {
            return Err(format!(
                "Classic COM contract registry entry '{}' is unused by the loaded metadata",
                entry.entry_id
            ));
        }
    }
    for entry in &registry.ownership_outputs {
        if !consumed_ids.contains(&entry.entry_id) {
            return Err(format!(
                "Classic COM contract registry entry '{}' is unused by the loaded metadata",
                entry.entry_id
            ));
        }
    }
    Ok(())
}

fn load_registry() -> Result<Registry, String> {
    serde_json::from_str::<serde_json::Value>(SCHEMA_JSON)
        .map_err(|error| format!("Invalid Classic COM contract JSON schema: {error}"))?;
    let manifest: RegistryManifest = serde_json::from_str(MANIFEST_JSON)
        .map_err(|error| format!("Invalid Classic COM contract manifest: {error}"))?;
    if manifest.schema_version != 2 {
        return Err(format!(
            "Unsupported Classic COM contract manifest schema {}",
            manifest.schema_version
        ));
    }
    if manifest.files.len() != 2
        || manifest.files[0].path != "conditional-outputs.json"
        || manifest.files[0].kind != ContractKind::ConditionalOutput
        || manifest.files[0].family_ids != [ExactFamilyId::ConditionalOutput]
        || manifest.files[1].path != "ownership-outputs.json"
        || manifest.files[1].kind != ContractKind::Ownership
        || manifest.files[1].family_ids != [ExactFamilyId::Ownership]
    {
        return Err("Classic COM contract manifest does not list the compiled data files".into());
    }
    validate_sha256(
        &manifest.files[0].sha256,
        "conditional-outputs.json manifest file hash",
    )?;
    validate_sha256(
        &manifest.files[1].sha256,
        "ownership-outputs.json manifest file hash",
    )?;
    let conditional_hash = format!("{:X}", Sha256::digest(CONDITIONAL_OUTPUTS_JSON.as_bytes()));
    if conditional_hash != manifest.files[0].sha256 {
        return Err(format!(
            "Classic COM contract file hash mismatch for conditional-outputs.json: expected {}, found {conditional_hash}",
            manifest.files[0].sha256
        ));
    }
    let ownership_hash = format!("{:X}", Sha256::digest(OWNERSHIP_OUTPUTS_JSON.as_bytes()));
    if ownership_hash != manifest.files[1].sha256 {
        return Err(format!(
            "Classic COM contract file hash mismatch for ownership-outputs.json: expected {}, found {ownership_hash}",
            manifest.files[1].sha256
        ));
    }
    let file: ConditionalOutputFile = serde_json::from_str(CONDITIONAL_OUTPUTS_JSON)
        .map_err(|error| format!("Invalid conditional-output contracts: {error}"))?;
    if file.schema_version != 2 {
        return Err(format!(
            "Unsupported conditional-output contract schema {}",
            file.schema_version
        ));
    }
    validate_conditional_outputs(&file.contracts)?;
    let ownership_file: OwnershipOutputFile = serde_json::from_str(OWNERSHIP_OUTPUTS_JSON)
        .map_err(|error| format!("Invalid ownership-output contracts: {error}"))?;
    if ownership_file.schema_version != 2 {
        return Err(format!(
            "Unsupported ownership-output contract schema {}",
            ownership_file.schema_version
        ));
    }
    validate_ownership_outputs(&ownership_file.contracts)?;
    Ok(Registry {
        conditional_outputs: file.contracts,
        ownership_outputs: ownership_file.contracts,
    })
}

fn validate_ownership_outputs(entries: &[OwnershipOutputContract]) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    let mut selectors = BTreeMap::new();
    for entry in entries {
        if entry.kind != ContractKind::Ownership || entry.family_id != ExactFamilyId::Ownership {
            return Err(format!(
                "Contract '{}' must use the exact ownership family and kind",
                entry.entry_id
            ));
        }
        if !valid_exact_entry_id(&entry.entry_id) {
            return Err(format!(
                "Contract registry entry ID is invalid: '{}'",
                entry.entry_id
            ));
        }
        let output = entry.contract.parameter_index;
        let parameter = entry.selector.parameters.get(output).ok_or_else(|| {
            format!(
                "Contract '{}' output parameter index {output} is outside its selector",
                entry.entry_id
            )
        })?;
        let expected_entry_id = exact_parameter_entry_id(
            entry.family_id,
            &entry.selector.interface.namespace,
            &entry.selector.interface.name,
            &entry.selector.interface.iid,
            &entry.selector.method,
            entry.selector.absolute_slot,
            output,
            &parameter.name,
        );
        if entry.entry_id != expected_entry_id {
            return Err(format!(
                "Contract registry entry ID '{}' does not match its exact selector; expected '{expected_entry_id}'",
                entry.entry_id
            ));
        }
        if !ids.insert(entry.entry_id.clone()) {
            return Err(format!(
                "Duplicate Classic COM contract registry ID '{}'",
                entry.entry_id
            ));
        }
        validate_contract_selector(
            &entry.entry_id,
            &entry.selector,
            &entry.evidence,
            &entry.validated_metadata,
        )?;
        let valid_ownership = matches!(
            (entry.contract.ownership, entry.contract.cleanup),
            (
                OutputAllocationOwnership::CoTaskMemOwned,
                OutputAllocationCleanup::CoTaskMemFree
            ) | (
                OutputAllocationOwnership::HandleOwned,
                OutputAllocationCleanup::DeleteObject
            )
        );
        if parameter.direction != "out" || parameter.pointer_depth == 0 || !valid_ownership {
            return Err(format!(
                "Contract '{}' is not a supported output ownership/cleanup pair",
                entry.entry_id
            ));
        }
        let selector_key = format!(
            "{}.{}:{}:{}:{}:{}",
            entry.selector.interface.namespace,
            entry.selector.interface.name,
            entry.selector.interface.iid.to_ascii_lowercase(),
            entry.selector.method,
            entry.selector.absolute_slot,
            output
        );
        if let Some(existing) = selectors.insert(selector_key.clone(), entry.entry_id.clone()) {
            return Err(format!(
                "Conflicting contract selectors '{existing}' and '{}' for {selector_key}",
                entry.entry_id
            ));
        }
        if entry.reason.is_empty() {
            return Err(format!(
                "Contract '{}' has no ownership reason",
                entry.entry_id
            ));
        }
    }
    Ok(())
}

fn validate_contract_selector(
    entry_id: &str,
    selector: &ContractSelector,
    evidence: &[EvidenceCitation],
    validated_metadata: &[ValidatedMetadata],
) -> Result<(), String> {
    validate_sha256(
        &selector.source_fingerprint,
        &format!("source fingerprint for '{entry_id}'"),
    )?;
    if selector.interface.iid != selector.declaring_iid
        || selector.parameters.len() != selector.parameter_count
        || selector
            .parameters
            .iter()
            .enumerate()
            .any(|(index, parameter)| parameter.index != index)
    {
        return Err(format!(
            "Contract '{entry_id}' parameter selector is incomplete, unordered, or has mismatched IIDs"
        ));
    }
    if evidence.is_empty()
        || evidence.iter().any(|citation| match citation.kind {
            EvidenceSourceKind::MicrosoftLearn => {
                citation.url.as_deref().is_none_or(str::is_empty) || citation.file.is_some()
            }
            EvidenceSourceKind::SdkHeader => {
                citation.file.as_deref().is_none_or(str::is_empty) || citation.url.is_some()
            }
        })
    {
        return Err(format!("Contract '{entry_id}' has no usable citation"));
    }
    if validated_metadata.is_empty() {
        return Err(format!(
            "Contract '{entry_id}' has no validated metadata identity"
        ));
    }
    for metadata in validated_metadata {
        if metadata.package.is_empty() || metadata.version.is_empty() {
            return Err(format!(
                "Contract '{entry_id}' has an incomplete metadata identity"
            ));
        }
        validate_sha256(
            &metadata.sha256,
            &format!("validated metadata hash for '{entry_id}'"),
        )?;
    }
    if !validated_metadata.iter().any(|metadata| {
        metadata.package == PINNED_METADATA_PACKAGE
            && metadata.version == PINNED_METADATA_VERSION
            && metadata.sha256 == PINNED_METADATA_SHA256
    }) {
        return Err(format!(
            "Contract '{entry_id}' is not validated against the pinned metadata package/version/hash"
        ));
    }
    Ok(())
}

fn validate_conditional_outputs(entries: &[ConditionalOutputContract]) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    let mut selectors = BTreeMap::new();
    for entry in entries {
        if entry.kind != ContractKind::ConditionalOutput {
            return Err(format!(
                "Contract '{}' has kind '{}', expected conditional-output",
                entry.entry_id,
                entry.kind.key()
            ));
        }
        if !valid_exact_entry_id(&entry.entry_id) {
            return Err(format!(
                "Contract registry entry ID is invalid: '{}'",
                entry.entry_id
            ));
        }
        let expected_entry_id = exact_method_entry_id(
            entry.family_id,
            &entry.selector.interface.namespace,
            &entry.selector.interface.name,
            &entry.selector.interface.iid,
            &entry.selector.method,
            entry.selector.absolute_slot,
        );
        if entry.entry_id != expected_entry_id {
            return Err(format!(
                "Contract registry entry ID '{}' does not match its exact selector; expected '{expected_entry_id}'",
                entry.entry_id
            ));
        }
        if !ids.insert(entry.entry_id.clone()) {
            return Err(format!(
                "Duplicate Classic COM contract registry ID '{}'",
                entry.entry_id
            ));
        }
        validate_sha256(
            &entry.selector.source_fingerprint,
            &format!("source fingerprint for '{}'", entry.entry_id),
        )?;
        if entry.selector.parameters.len() != entry.selector.parameter_count
            || entry
                .selector
                .parameters
                .iter()
                .enumerate()
                .any(|(index, parameter)| parameter.index != index)
        {
            return Err(format!(
                "Contract '{}' parameter selector is incomplete or unordered",
                entry.entry_id
            ));
        }
        let selector_key = format!(
            "{}.{}:{}:{}:{}",
            entry.selector.interface.namespace,
            entry.selector.interface.name,
            entry.selector.interface.iid.to_ascii_lowercase(),
            entry.selector.method,
            entry.selector.absolute_slot
        );
        if let Some(existing) = selectors.insert(selector_key.clone(), entry.entry_id.clone()) {
            return Err(format!(
                "Conflicting contract selectors '{existing}' and '{}' for {selector_key}",
                entry.entry_id
            ));
        }
        if entry.evidence.is_empty()
            || entry.evidence.iter().any(|citation| match citation.kind {
                EvidenceSourceKind::MicrosoftLearn => {
                    citation.url.as_deref().is_none_or(str::is_empty) || citation.file.is_some()
                }
                EvidenceSourceKind::SdkHeader => {
                    citation.file.as_deref().is_none_or(str::is_empty) || citation.url.is_some()
                }
            })
        {
            return Err(format!(
                "Contract '{}' has no usable citation",
                entry.entry_id
            ));
        }
        if entry.validated_metadata.is_empty() {
            return Err(format!(
                "Contract '{}' has no validated metadata identity",
                entry.entry_id
            ));
        }
        for metadata in &entry.validated_metadata {
            if metadata.package.is_empty() || metadata.version.is_empty() {
                return Err(format!(
                    "Contract '{}' has an incomplete metadata identity",
                    entry.entry_id
                ));
            }
            validate_sha256(
                &metadata.sha256,
                &format!("validated metadata hash for '{}'", entry.entry_id),
            )?;
        }
        if !entry.validated_metadata.iter().any(|metadata| {
            metadata.package == PINNED_METADATA_PACKAGE
                && metadata.version == PINNED_METADATA_VERSION
                && metadata.sha256 == PINNED_METADATA_SHA256
        }) {
            return Err(format!(
                "Contract '{}' is not validated against the pinned metadata package/version/hash",
                entry.entry_id
            ));
        }
        let semantics = &entry.contract;
        let output_indices = semantics
            .outputs
            .iter()
            .map(|output| output.parameter_index)
            .collect::<BTreeSet<_>>();
        if !semantics.context_must_be_native_null
            || semantics.synchronous.flags != 0
            || semantics.semisynchronous.flags != 16
            || !(1..=2).contains(&semantics.outputs.len())
            || output_indices.len() != semantics.outputs.len()
            || semantics.synchronous.output_parameter_index.is_some()
                != semantics.synchronous.option_name.is_some()
            || semantics.semisynchronous.output_parameter_index.is_some()
                != semantics.semisynchronous.option_name.is_some()
            || semantics
                .synchronous
                .output_parameter_index
                .into_iter()
                .chain(semantics.semisynchronous.output_parameter_index)
                .collect::<BTreeSet<_>>()
                != output_indices
            || semantics.context_parameter_index >= entry.selector.parameter_count
            || semantics.flags_parameter_index >= entry.selector.parameter_count
            || semantics
                .public_input_parameter_indices
                .iter()
                .any(|index| *index >= entry.selector.parameter_count)
            || semantics.outputs.iter().any(|output| {
                output.parameter_index >= entry.selector.parameter_count
                    || output.ownership != OutputOwnership::OwnedComPlusOne
                    || output.interface_iid.is_empty()
                    || !output.argument_optional
                    || output.nullable_on_success
            })
        {
            return Err(format!(
                "Contract '{}' uses unsupported conditional-output semantics",
                entry.entry_id
            ));
        }
    }
    Ok(())
}

fn validate_sha256(value: &str, context: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Err(format!("{context} is not a 64-character SHA-256 value"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_registry_is_strict_unique_and_hash_validated() {
        serde_json::from_str::<serde_json::Value>(SCHEMA_JSON).unwrap();
        let registry = load_registry().unwrap();
        let ids = statically_declared_exact_entry_ids().unwrap();
        assert_eq!(ids.len(), 498);
        assert_eq!(registry.conditional_outputs.len(), 7);
        assert_eq!(registry.ownership_outputs.len(), 148);
        assert_eq!(
            registry.conditional_outputs[0].entry_id,
            "wmi.conditional-output.entry.windows-win32-system-wmi.iwbemservices.9556dc99828c11cfa37e00aa003240c7.opennamespace.slot-3.v1"
        );
        assert_eq!(
            registry.conditional_outputs[0].family_id,
            ExactFamilyId::ConditionalOutput
        );
        assert!(registry.ownership_outputs.iter().all(|entry| {
            entry.family_id == ExactFamilyId::Ownership
                && entry.kind == ContractKind::Ownership
                && matches!(
                    (entry.contract.ownership, entry.contract.cleanup),
                    (
                        OutputAllocationOwnership::CoTaskMemOwned,
                        OutputAllocationCleanup::CoTaskMemFree
                    ) | (
                        OutputAllocationOwnership::HandleOwned,
                        OutputAllocationCleanup::DeleteObject
                    )
                )
        }));
        assert_eq!(
            registry
                .ownership_outputs
                .iter()
                .filter(|entry| {
                    entry.contract.ownership == OutputAllocationOwnership::HandleOwned
                })
                .count(),
            1
        );
    }

    #[test]
    fn strict_contract_types_reject_unknown_fields_and_kinds() {
        let unknown_field = r#"{"schemaVersion":2,"contracts":[],"extra":true}"#;
        assert!(serde_json::from_str::<ConditionalOutputFile>(unknown_field).is_err());
        let unknown_kind = r#"{"entryId":"x.v1","familyId":"wmi.conditional-output.v1","kind":"renderer-code","selector":{},"contract":{},"evidence":[],"validatedMetadata":[]}"#;
        assert!(serde_json::from_str::<ConditionalOutputContract>(unknown_kind).is_err());
    }

    #[test]
    fn exact_catalog_rejects_malformed_duplicate_and_selector_drift() {
        let family_id = ExactFamilyId::Ownership;
        let selector = ExactEntrySelector {
            namespace: "Tests".into(),
            interface: "IExact".into(),
            iid: "11111111-2222-3333-4444-555555555555".into(),
            method: "GetValue".into(),
            slot: 3,
            parameter: Some((0, "value".into())),
        };
        let entry = ExactRegistryEntry {
            entry_id: selector.entry_id(family_id),
            family_id,
            contract_kind: ContractKind::Ownership,
            selector,
            source_fingerprint: "A".repeat(64),
            reason: "exact test contract".into(),
            citation: "https://learn.microsoft.com/test".into(),
        };
        assert_eq!(
            validate_exact_entry_catalog([entry.clone()]).unwrap().len(),
            1
        );

        let mut malformed = entry.clone();
        malformed.entry_id = "family-wide.v1".into();
        assert!(
            validate_exact_entry_catalog([malformed])
                .unwrap_err()
                .contains("malformed")
        );

        let mut selector_drift = entry.clone();
        selector_drift.selector.method = "Other".into();
        assert!(
            validate_exact_entry_catalog([selector_drift])
                .unwrap_err()
                .contains("selector-derived")
        );

        let mut conflicting = entry.clone();
        conflicting.citation = "https://learn.microsoft.com/conflict".into();
        assert!(
            validate_exact_entry_catalog([entry, conflicting])
                .unwrap_err()
                .contains("conflicting definitions")
        );
    }

    #[test]
    fn validation_rejects_duplicate_ids_selectors_citations_and_fingerprints() {
        let registry = load_registry().unwrap();
        let entry = registry.conditional_outputs[0].clone();
        assert!(
            validate_conditional_outputs(&[entry.clone(), entry.clone()])
                .unwrap_err()
                .contains("Duplicate")
        );

        let mut malformed = entry.clone();
        malformed.selector.source_fingerprint = "not-a-hash".into();
        assert!(
            validate_conditional_outputs(&[malformed])
                .unwrap_err()
                .contains("SHA-256")
        );
        let mut uncited = entry;
        uncited.evidence.clear();
        assert!(
            validate_conditional_outputs(&[uncited])
                .unwrap_err()
                .contains("citation")
        );

        let mut conflict = registry.conditional_outputs[0].clone();
        conflict.entry_id =
            "wmi.conditional-output.entry.windows-win32-system-wmi.iwbemservices.9556dc99828c11cfa37e00aa003240c7.conflict.slot-3.v1".into();
        assert!(
            validate_conditional_outputs(&[registry.conditional_outputs[0].clone(), conflict])
                .unwrap_err()
                .contains("does not match its exact selector")
        );

        let mut metadata_hash = registry.conditional_outputs[0].clone();
        metadata_hash.validated_metadata[0].sha256 = "bad".into();
        assert!(
            validate_conditional_outputs(&[metadata_hash])
                .unwrap_err()
                .contains("SHA-256")
        );
        let mut wrong_metadata = registry.conditional_outputs[0].clone();
        wrong_metadata.validated_metadata[0].sha256 = "0".repeat(64);
        assert!(
            validate_conditional_outputs(&[wrong_metadata])
                .unwrap_err()
                .contains("pinned metadata")
        );

        let ownership = registry.ownership_outputs[0].clone();
        assert!(
            validate_ownership_outputs(&[ownership.clone(), ownership.clone()])
                .unwrap_err()
                .contains("Duplicate")
        );
        let mut ownership_drift = ownership.clone();
        ownership_drift.selector.parameters[ownership.contract.parameter_index].direction =
            "in".into();
        assert!(
            validate_ownership_outputs(&[ownership_drift])
                .unwrap_err()
                .contains("supported output ownership/cleanup pair")
        );
        let mut ownership_fingerprint = ownership;
        ownership_fingerprint.selector.source_fingerprint = "bad".into();
        assert!(
            validate_ownership_outputs(&[ownership_fingerprint])
                .unwrap_err()
                .contains("SHA-256")
        );
    }

    #[test]
    fn registry_usage_validation_rejects_unused_entries() {
        assert!(
            validate_registry_usage(&BTreeSet::new())
                .unwrap_err()
                .contains("unused")
        );
        let registry = load_registry().unwrap();
        let mut used = registry
            .conditional_outputs
            .iter()
            .map(|entry| entry.entry_id.clone())
            .chain(
                registry
                    .ownership_outputs
                    .iter()
                    .map(|entry| entry.entry_id.clone()),
            )
            .collect::<BTreeSet<_>>();
        assert!(validate_registry_usage(&used).is_ok());
        used.remove(&registry.ownership_outputs[0].entry_id);
        assert!(
            validate_registry_usage(&used)
                .unwrap_err()
                .contains("unused")
        );
    }

    #[test]
    fn contract_data_cannot_inject_cleanup_or_renderer_fields() {
        let mut value: serde_json::Value = serde_json::from_str(CONDITIONAL_OUTPUTS_JSON).unwrap();
        value["contracts"][0]["contract"]["cleanup"] = "ArbitraryFree".into();
        assert!(
            serde_json::from_value::<ConditionalOutputFile>(value)
                .unwrap_err()
                .to_string()
                .contains("unknown field")
        );

        let mut ownership: serde_json::Value =
            serde_json::from_str(OWNERSHIP_OUTPUTS_JSON).unwrap();
        ownership["contracts"][0]["contract"]["cleanup"] = "ArbitraryFree".into();
        assert!(
            serde_json::from_value::<OwnershipOutputFile>(ownership)
                .unwrap_err()
                .to_string()
                .contains("unknown variant")
        );
    }
}
