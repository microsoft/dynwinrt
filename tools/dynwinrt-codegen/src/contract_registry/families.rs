// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use serde::Deserialize;

use super::{
    BTreeSet, ContractKind, ContractSelector, EvidenceCitation, EvidenceContract,
    EvidenceSourceKind, ExactFamilyId, exact_method_entry_id, exact_parameter_entry_id,
    parse_contract_file, required_nullable, validate_citations, validate_contract_selector,
    validate_guid,
};

pub(crate) type SafeArrayContract = EvidenceContract<SafeArraySemantics>;
pub(crate) type NullInputContract = EvidenceContract<NullInputSemantics>;
pub(crate) type ParameterDirectionContract = EvidenceContract<ParameterDirectionSemantics>;
pub(crate) type BorrowedHandleContract = EvidenceContract<BorrowedHandleSemantics>;
pub(crate) type EnumeratorContract = EvidenceContract<EnumeratorSemantics>;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SafeArraySemantics {
    pub parameter_index: usize,
    pub element: SafeArrayElement,
    pub ownership: SafeArrayOwnership,
    pub cleanup: SafeArrayCleanup,
    pub nullability: SafeArrayNullability,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SafeArrayElement {
    pub vartype: SafeArrayVartype,
    #[serde(deserialize_with = "required_nullable")]
    pub interface_iid: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
pub(crate) enum SafeArrayVartype {
    #[serde(rename = "VT_I4")]
    I4,
    #[serde(rename = "VT_UI1")]
    Ui1,
    #[serde(rename = "VT_UI4")]
    Ui4,
    #[serde(rename = "VT_R8")]
    R8,
    #[serde(rename = "VT_BSTR")]
    Bstr,
    #[serde(rename = "VT_UNKNOWN")]
    Unknown,
    #[serde(rename = "VT_VARIANT")]
    Variant,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SafeArrayOwnership {
    BorrowedInput,
    OwnedOutput,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
pub(crate) enum SafeArrayCleanup {
    #[serde(rename = "none")]
    None,
    SafeArrayDestroy,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) enum SafeArrayNullability {
    RequiredInput {},
    RequiredOutputCell { pointee: SafeArrayPointee },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) enum SafeArrayPointee {
    Required {},
    NullableOnSuccess {
        reason: String,
        evidence: Vec<EvidenceCitation>,
    },
}

impl SafeArrayNullability {
    pub(crate) fn allows_null_output(&self) -> bool {
        matches!(
            self,
            Self::RequiredOutputCell {
                pointee: SafeArrayPointee::NullableOnSuccess { .. }
            }
        )
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct NullInputSemantics {
    pub parameter_index: usize,
    pub value: NativeNull,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum NativeNull {
    NativeNull,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ParameterDirectionSemantics {
    pub parameter_index: usize,
    pub direction: OutputDirection,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum OutputDirection {
    Out,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct BorrowedHandleSemantics {
    pub parameter_index: usize,
    pub handle: BorrowedHandle,
    pub ownership: BorrowedOwnership,
    pub cleanup: NoCleanup,
    pub receiving_cell: RequiredCell,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
pub(crate) enum BorrowedHandle {
    #[serde(rename = "Windows.Win32.Foundation.HWND")]
    Hwnd,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum BorrowedOwnership {
    Borrowed,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum NoCleanup {
    None,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RequiredCell {
    Required,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct EnumeratorSemantics {
    pub capacity_parameter_index: usize,
    pub values_parameter_index: usize,
    pub fetched_parameter_index: usize,
    pub element: EnumeratorElement,
    pub fetched_optional_for_single: bool,
    pub hresult: EnumeratorHresult,
    pub evidence_source: EnumeratorEvidenceSource,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct EnumeratorElement {
    pub namespace: String,
    pub name: String,
    pub kind: EnumeratorElementKind,
    #[serde(deserialize_with = "required_nullable")]
    pub iid: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum EnumeratorElementKind {
    Interface,
    Struct,
    Unknown,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
pub(crate) enum EnumeratorHresult {
    #[serde(rename = "s-ok-s-false")]
    SOkSFalse,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum EnumeratorEvidenceSource {
    ComStandard,
    ExactRegistry,
}

pub(super) trait FamilySemantics: Sized {
    const KIND: ContractKind;
    const FAMILY: ExactFamilyId;
    const SHAPE_BASED: bool;

    fn parameter_index(&self) -> Option<usize>;
    fn validate(&self, entry: &EvidenceContract<Self>) -> Result<(), String>;
}

pub(super) fn load_family<T: FamilySemantics + serde::de::DeserializeOwned>(
    json: &str,
    name: &str,
) -> Result<Vec<EvidenceContract<T>>, String> {
    let file = parse_contract_file(json, name)?;
    validate_family(&file.contracts)?;
    Ok(file.contracts)
}

pub(super) fn validate_family<T: FamilySemantics>(
    entries: &[EvidenceContract<T>],
) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    let mut selectors = BTreeSet::new();
    for entry in entries {
        if entry.kind != T::KIND || entry.family_id != T::FAMILY {
            return Err(format!(
                "Contract '{}' must use the exact {} family and kind",
                entry.entry_id,
                T::KIND.key()
            ));
        }
        validate_contract_selector(
            &entry.entry_id,
            &entry.selector,
            &entry.evidence,
            &entry.validated_metadata,
        )?;
        let selector = &entry.selector;
        if selector.source_shape.is_some() != T::SHAPE_BASED {
            return Err(format!(
                "Contract '{}' uses an unsupported source fingerprint format",
                entry.entry_id
            ));
        }
        if entry.reason.is_empty()
            || entry.evidence.len() != 1
            || entry.evidence[0].kind != EvidenceSourceKind::MicrosoftLearn
        {
            return Err(format!(
                "Contract '{}' must retain its reason and single Microsoft citation",
                entry.entry_id
            ));
        }
        let expected_id = if let Some(index) = entry.contract.parameter_index() {
            let parameter = selector.parameters.get(index).ok_or_else(|| {
                format!(
                    "Contract '{}' parameter index is outside its selector",
                    entry.entry_id
                )
            })?;
            exact_parameter_entry_id(
                entry.family_id,
                &selector.interface.namespace,
                &selector.interface.name,
                &selector.interface.iid,
                &selector.method,
                selector.absolute_slot,
                index,
                &parameter.name,
            )
        } else {
            exact_method_entry_id(
                entry.family_id,
                &selector.interface.namespace,
                &selector.interface.name,
                &selector.interface.iid,
                &selector.method,
                selector.absolute_slot,
            )
        };
        if entry.entry_id != expected_id {
            return Err(format!(
                "Contract registry entry ID '{}' does not match its exact selector; expected '{expected_id}'",
                entry.entry_id
            ));
        }
        if !ids.insert(&entry.entry_id) {
            return Err(format!(
                "Duplicate Classic COM contract registry ID '{}'",
                entry.entry_id
            ));
        }
        if !selectors.insert((
            &selector.interface.namespace,
            &selector.interface.name,
            selector.interface.iid.to_ascii_lowercase(),
            &selector.method,
            selector.absolute_slot,
            entry.contract.parameter_index(),
        )) {
            return Err(format!(
                "Conflicting contract selectors for '{}'",
                entry.entry_id
            ));
        }
        entry.contract.validate(entry)?;
    }
    Ok(())
}

fn unsupported<T>(entry: &EvidenceContract<T>) -> String {
    format!(
        "Contract '{}' uses unsupported {} semantics",
        entry.entry_id,
        entry.kind.key()
    )
}

fn ordinary_hresult(selector: &ContractSelector) -> bool {
    selector.raw_method_shape().ends_with(
        ")->Windows.Win32.Foundation.HRESULT[Struct]/ptr0/Unspecified/underlying=i32/ptr0/Unspecified:plain_hresult:not_enumerator_next",
    )
}

impl FamilySemantics for SafeArraySemantics {
    const KIND: ContractKind = ContractKind::Safearray;
    const FAMILY: ExactFamilyId = ExactFamilyId::SafeArray;
    const SHAPE_BASED: bool = true;

    fn parameter_index(&self) -> Option<usize> {
        Some(self.parameter_index)
    }

    fn validate(&self, entry: &SafeArrayContract) -> Result<(), String> {
        let parameter = &entry.selector.parameters[self.parameter_index];
        if parameter.native_type != "Windows.Win32.System.Com.SAFEARRAY"
            || parameter.optional
            || parameter.constness != "mutable"
            || !ordinary_hresult(&entry.selector)
            || self.element.interface_iid.is_some()
                != (self.element.vartype == SafeArrayVartype::Unknown)
        {
            return Err(unsupported(entry));
        }
        if let Some(iid) = &self.element.interface_iid {
            validate_guid(iid, &entry.entry_id)?;
        }
        match (&self.ownership, &self.cleanup, &self.nullability) {
            (
                SafeArrayOwnership::BorrowedInput,
                SafeArrayCleanup::None,
                SafeArrayNullability::RequiredInput {},
            ) if parameter.direction == "in" && parameter.pointer_depth == 1 => {}
            (
                SafeArrayOwnership::OwnedOutput,
                SafeArrayCleanup::SafeArrayDestroy,
                SafeArrayNullability::RequiredOutputCell { pointee },
            ) if parameter.direction == "out" && parameter.pointer_depth == 2 => {
                if let SafeArrayPointee::NullableOnSuccess { reason, evidence } = pointee {
                    if reason.is_empty() {
                        return Err(format!(
                            "Contract '{}' has no nullable-pointee reason",
                            entry.entry_id
                        ));
                    }
                    validate_citations(&entry.entry_id, evidence)?;
                }
            }
            _ => return Err(unsupported(entry)),
        }
        Ok(())
    }
}

impl FamilySemantics for NullInputSemantics {
    const KIND: ContractKind = ContractKind::NullInput;
    const FAMILY: ExactFamilyId = ExactFamilyId::ReservedNullInput;
    const SHAPE_BASED: bool = false;

    fn parameter_index(&self) -> Option<usize> {
        Some(self.parameter_index)
    }

    fn validate(&self, entry: &NullInputContract) -> Result<(), String> {
        let parameter = &entry.selector.parameters[self.parameter_index];
        if self.value != NativeNull::NativeNull
            || parameter.direction != "in"
            || parameter.pointer_depth == 0
            || parameter.native_type != "void"
        {
            return Err(unsupported(entry));
        }
        Ok(())
    }
}

impl FamilySemantics for ParameterDirectionSemantics {
    const KIND: ContractKind = ContractKind::ParameterDirection;
    const FAMILY: ExactFamilyId = ExactFamilyId::ParameterDirection;
    const SHAPE_BASED: bool = false;

    fn parameter_index(&self) -> Option<usize> {
        Some(self.parameter_index)
    }

    fn validate(&self, entry: &ParameterDirectionContract) -> Result<(), String> {
        let parameter = &entry.selector.parameters[self.parameter_index];
        if self.direction != OutputDirection::Out
            || parameter.direction != "inout"
            || parameter.pointer_depth == 0
        {
            return Err(unsupported(entry));
        }
        Ok(())
    }
}

impl FamilySemantics for BorrowedHandleSemantics {
    const KIND: ContractKind = ContractKind::BorrowedHandle;
    const FAMILY: ExactFamilyId = ExactFamilyId::BorrowedHwndOutput;
    const SHAPE_BASED: bool = true;

    fn parameter_index(&self) -> Option<usize> {
        Some(self.parameter_index)
    }

    fn validate(&self, entry: &BorrowedHandleContract) -> Result<(), String> {
        let parameter = &entry.selector.parameters[self.parameter_index];
        if self.handle != BorrowedHandle::Hwnd
            || self.ownership != BorrowedOwnership::Borrowed
            || self.cleanup != NoCleanup::None
            || self.receiving_cell != RequiredCell::Required
            || parameter.native_type != "Windows.Win32.Foundation.HWND"
            || parameter.direction != "out"
            || parameter.pointer_depth != 1
            || parameter.optional
            || parameter.const_attribute
            || parameter.constness != "mutable"
            || !ordinary_hresult(&entry.selector)
        {
            return Err(unsupported(entry));
        }
        Ok(())
    }
}

impl FamilySemantics for EnumeratorSemantics {
    const KIND: ContractKind = ContractKind::EnumeratorNext;
    const FAMILY: ExactFamilyId = ExactFamilyId::EnumeratorException;
    const SHAPE_BASED: bool = true;

    fn parameter_index(&self) -> Option<usize> {
        None
    }

    fn validate(&self, entry: &EnumeratorContract) -> Result<(), String> {
        let selector = &entry.selector;
        if selector.method != "Next"
            || selector.parameter_count != 3
            || (self.capacity_parameter_index, self.values_parameter_index, self.fetched_parameter_index) != (0, 1, 2)
            || self.hresult != EnumeratorHresult::SOkSFalse
            || !selector.raw_method_shape().ends_with(
                ")->Windows.Win32.Foundation.HRESULT[Struct]/ptr0/Unspecified/underlying=i32/ptr0/Unspecified:semantic_hresult:enumerator_next",
            )
            || self.element.namespace.is_empty()
            || self.element.name.is_empty()
            || self.element.iid.is_some() != (self.element.kind == EnumeratorElementKind::Interface)
        {
            return Err(unsupported(entry));
        }
        if let Some(iid) = &self.element.iid {
            validate_guid(iid, &entry.entry_id)?;
        }
        let [capacity, values, fetched] = selector.parameters.as_slice() else {
            return Err(unsupported(entry));
        };
        if capacity.native_type != "u32"
            || capacity.pointer_depth != 0
            || capacity.direction != "in"
            || capacity.optional
            || capacity.const_attribute
            || values.native_type != format!("{}.{}", self.element.namespace, self.element.name)
            || values.pointer_depth != 1
            || !matches!(values.direction.as_str(), "out" | "inout")
            || values.optional
            || values.const_attribute
            || values.constness != "mutable"
            || fetched.native_type != "u32"
            || fetched.pointer_depth != 1
            || !matches!(fetched.direction.as_str(), "out" | "inout")
            || fetched.optional != self.fetched_optional_for_single
            || fetched.const_attribute
            || fetched.constness != "mutable"
        {
            return Err(unsupported(entry));
        }
        Ok(())
    }
}
