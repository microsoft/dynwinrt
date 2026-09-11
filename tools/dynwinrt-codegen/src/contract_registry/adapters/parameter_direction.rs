// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExactOutParameterEvidence {
    pub selector: &'static crate::contract_registry::ContractSelector,
    pub declaring_namespace: &'static str,
    pub declaring_interface: &'static str,
    pub declaring_iid: &'static str,
    pub method_name: &'static str,
    pub vtable_index: usize,
    pub parameter_count: usize,
    pub parameter_index: usize,
    pub parameter_name: &'static str,
    pub source_fingerprint: &'static str,
    pub reason: &'static str,
    pub citation: &'static str,
}

impl ExactOutParameterEvidence {
    pub(crate) fn entry_id(&self) -> String {
        crate::contract_registry::exact_parameter_entry_id(
            self.family_id(),
            self.declaring_namespace,
            self.declaring_interface,
            self.declaring_iid,
            self.method_name,
            self.vtable_index,
            self.parameter_index,
            self.parameter_name,
        )
    }

    pub(crate) const fn family_id(&self) -> crate::contract_registry::ExactFamilyId {
        crate::contract_registry::ExactFamilyId::ParameterDirection
    }

    pub(crate) const fn contract_kind(&self) -> crate::contract_registry::ContractKind {
        crate::contract_registry::ContractKind::ParameterDirection
    }
}

pub(crate) fn entries() -> &'static [ExactOutParameterEvidence] {
    static EVIDENCE: OnceLock<Vec<ExactOutParameterEvidence>> = OnceLock::new();
    EVIDENCE.get_or_init(|| {
        crate::contract_registry::parameter_direction_contracts()
            .expect("embedded parameter-direction contract registry must validate")
            .iter()
            .map(|entry| {
                let selector = &entry.selector;
                ExactOutParameterEvidence {
                    selector,
                    declaring_namespace: &selector.interface.namespace,
                    declaring_interface: &selector.interface.name,
                    declaring_iid: &selector.declaring_iid,
                    method_name: &selector.method,
                    vtable_index: selector.absolute_slot,
                    parameter_count: selector.parameter_count,
                    parameter_index: entry.contract.parameter_index,
                    parameter_name: &selector.parameters[entry.contract.parameter_index].name,
                    source_fingerprint: &selector.source_fingerprint,
                    reason: &entry.reason,
                    citation: entry.microsoft_citation(),
                }
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn entries_are_unique_selector_specific_and_cited() {
        let mut selectors = BTreeSet::new();
        assert_eq!(entries().len(), 3);
        for entry in entries() {
            assert!(selectors.insert((
                entry.declaring_namespace,
                entry.declaring_interface,
                entry.declaring_iid,
                entry.method_name,
                entry.vtable_index,
                entry.parameter_index,
            )));
            assert!(entry.parameter_index < entry.parameter_count);
            assert_eq!(entry.source_fingerprint.len(), 64);
            assert!(entry.citation.starts_with("https://learn.microsoft.com/"));
            assert!(crate::contract_registry::valid_exact_entry_id(
                &entry.entry_id()
            ));
        }
    }
}
