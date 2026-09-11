// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::sync::OnceLock;

use crate::contract_registry::{self, EnumeratorEvidenceSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnumeratorElementKind {
    Interface,
    Struct,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnumeratorDirection {
    Out,
    InOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EnumeratorContract {
    pub selector: &'static contract_registry::ContractSelector,
    pub interface_namespace: &'static str,
    pub interface_name: &'static str,
    pub interface_iid: &'static str,
    pub next_vtable_index: usize,
    pub element_namespace: &'static str,
    pub element_name: &'static str,
    pub element_kind: EnumeratorElementKind,
    pub element_iid: Option<&'static str>,
    pub values_direction: EnumeratorDirection,
    pub fetched_direction: EnumeratorDirection,
    pub fetched_optional: bool,
    pub evidence_source: EnumeratorEvidenceSource,
    pub reason: &'static str,
    pub citation: &'static str,
}

impl EnumeratorContract {
    pub(crate) fn entry_id(&self) -> String {
        crate::contract_registry::exact_method_entry_id(
            self.family_id(),
            self.interface_namespace,
            self.interface_name,
            self.interface_iid,
            "Next",
            self.next_vtable_index,
        )
    }

    pub(crate) const fn family_id(&self) -> crate::contract_registry::ExactFamilyId {
        crate::contract_registry::ExactFamilyId::EnumeratorException
    }

    pub(crate) const fn contract_kind(&self) -> crate::contract_registry::ContractKind {
        crate::contract_registry::ContractKind::EnumeratorNext
    }

    pub(crate) fn uses_generic_standard(&self) -> bool {
        self.evidence_source == EnumeratorEvidenceSource::ComStandard
    }
}

fn output_direction(parameter: &contract_registry::ParameterSelector) -> EnumeratorDirection {
    match parameter.direction.as_str() {
        "out" => EnumeratorDirection::Out,
        "inout" => EnumeratorDirection::InOut,
        _ => unreachable!("validated enumerator output direction"),
    }
}

pub(crate) fn contract_for_declaration(
    interface_namespace: &str,
    interface_name: &str,
) -> Option<&'static EnumeratorContract> {
    contracts().iter().find(|contract| {
        contract.interface_namespace == interface_namespace
            && contract.interface_name == interface_name
    })
}

pub(crate) fn exact_contract(
    interface_namespace: &str,
    interface_name: &str,
    interface_iid: &str,
    next_vtable_index: usize,
) -> Option<&'static EnumeratorContract> {
    contract_for_declaration(interface_namespace, interface_name).filter(|contract| {
        contract.interface_iid.eq_ignore_ascii_case(interface_iid)
            && contract.next_vtable_index == next_vtable_index
    })
}

pub(crate) fn contracts() -> &'static [EnumeratorContract] {
    static CONTRACTS: OnceLock<Vec<EnumeratorContract>> = OnceLock::new();
    CONTRACTS.get_or_init(|| {
        contract_registry::enumerator_contracts()
            .expect("embedded enumerator contract registry must validate")
            .iter()
            .map(|entry| {
                let selector = &entry.selector;
                let semantics = &entry.contract;
                EnumeratorContract {
                    selector,
                    interface_namespace: &selector.interface.namespace,
                    interface_name: &selector.interface.name,
                    interface_iid: &selector.declaring_iid,
                    next_vtable_index: selector.absolute_slot,
                    element_namespace: &semantics.element.namespace,
                    element_name: &semantics.element.name,
                    element_kind: match semantics.element.kind {
                        contract_registry::EnumeratorElementKind::Interface => {
                            EnumeratorElementKind::Interface
                        }
                        contract_registry::EnumeratorElementKind::Struct => {
                            EnumeratorElementKind::Struct
                        }
                        contract_registry::EnumeratorElementKind::Unknown => {
                            EnumeratorElementKind::Unknown
                        }
                    },
                    element_iid: semantics.element.iid.as_deref(),
                    values_direction: output_direction(
                        &selector.parameters[semantics.values_parameter_index],
                    ),
                    fetched_direction: output_direction(
                        &selector.parameters[semantics.fetched_parameter_index],
                    ),
                    fetched_optional: semantics.fetched_optional_for_single,
                    evidence_source: semantics.evidence_source,
                    reason: &entry.reason,
                    citation: entry.microsoft_citation(),
                }
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_lookup_rejects_identity_and_slot_drift() {
        assert!(
            exact_contract(
                "Windows.Win32.System.Com",
                "IEnumUnknown",
                "00000100-0000-0000-c000-000000000046",
                3
            )
            .is_some()
        );
        assert!(
            exact_contract(
                "Contoso",
                "IEnumUnknown",
                "00000100-0000-0000-c000-000000000046",
                3
            )
            .is_none()
        );
        assert!(
            exact_contract(
                "Windows.Win32.System.Com",
                "IEnumUnknown",
                "ffffffff-ffff-ffff-ffff-ffffffffffff",
                3
            )
            .is_none()
        );
        assert!(
            exact_contract(
                "Windows.Win32.System.Com",
                "IEnumUnknown",
                "00000100-0000-0000-c000-000000000046",
                4
            )
            .is_none()
        );
    }

    #[test]
    fn declarations_are_unique() {
        for (index, contract) in contracts().iter().enumerate() {
            assert!(
                !contracts()[..index].iter().any(|previous| {
                    previous.interface_namespace == contract.interface_namespace
                        && previous.interface_name == contract.interface_name
                }),
                "{}.{}",
                contract.interface_namespace,
                contract.interface_name
            );
            assert_eq!(
                contract.element_iid.is_some(),
                contract.element_kind == EnumeratorElementKind::Interface,
                "{}.{} element IID",
                contract.interface_namespace,
                contract.interface_name
            );
        }
    }

    #[test]
    fn generic_standard_next_entries_are_not_exact_exceptions() {
        assert_eq!(
            contracts()
                .iter()
                .filter(|contract| contract.uses_generic_standard())
                .count(),
            24
        );
        assert!(
            contracts()
                .iter()
                .filter(|contract| !contract.uses_generic_standard())
                .all(|contract| contract.citation != "https://learn.microsoft.com/windows/win32/api/unknwn/nf-unknwn-ienumunknown-next")
        );
    }
}
