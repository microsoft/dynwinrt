// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BorrowedHwndOutputEvidence {
    pub selector: &'static crate::contract_registry::ContractSelector,
    pub declaring_namespace: &'static str,
    pub declaring_interface: &'static str,
    pub declaring_iid: &'static str,
    pub method_name: &'static str,
    pub vtable_index: usize,
    pub parameter_count: usize,
    pub parameter_index: usize,
    pub parameter_name: &'static str,
    pub optional: bool,
    pub reason: &'static str,
    pub citation: &'static str,
}

impl BorrowedHwndOutputEvidence {
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
        crate::contract_registry::ExactFamilyId::BorrowedHwndOutput
    }

    pub(crate) fn entries() -> &'static [BorrowedHwndOutputEvidence] {
        static EVIDENCE: OnceLock<Vec<BorrowedHwndOutputEvidence>> = OnceLock::new();
        EVIDENCE.get_or_init(|| {
            crate::contract_registry::borrowed_handle_contracts()
                .expect("embedded borrowed-handle contract registry must validate")
                .iter()
                .map(|entry| {
                    let selector = &entry.selector;
                    BorrowedHwndOutputEvidence {
                        selector,
                        declaring_namespace: &selector.interface.namespace,
                        declaring_interface: &selector.interface.name,
                        declaring_iid: &selector.declaring_iid,
                        method_name: &selector.method,
                        vtable_index: selector.absolute_slot,
                        parameter_count: selector.parameter_count,
                        parameter_index: entry.contract.parameter_index,
                        parameter_name: &selector.parameters[entry.contract.parameter_index].name,
                        optional: selector.parameters[entry.contract.parameter_index].optional,
                        reason: &entry.reason,
                        citation: entry.microsoft_citation(),
                    }
                })
                .collect()
        })
    }

    pub(crate) const fn contract_kind(&self) -> crate::contract_registry::ContractKind {
        crate::contract_registry::ContractKind::BorrowedHandle
    }
}

pub(crate) fn registered_borrowed_hwnd_output(
    namespace: &str,
    interface: &str,
    iid: &str,
    method: &str,
    slot: usize,
    parameter_index: usize,
) -> Option<&'static BorrowedHwndOutputEvidence> {
    BorrowedHwndOutputEvidence::entries()
        .iter()
        .find(|evidence| {
            evidence.declaring_namespace == namespace
                && evidence.declaring_interface == interface
                && evidence.declaring_iid.eq_ignore_ascii_case(iid)
                && evidence.method_name == method
                && evidence.vtable_index == slot
                && evidence.parameter_index == parameter_index
        })
}

pub(crate) fn borrowed_hwnd_evidence_for_declaration(
    namespace: &str,
    interface: &str,
    method: &str,
    slot: usize,
) -> Option<&'static BorrowedHwndOutputEvidence> {
    BorrowedHwndOutputEvidence::entries()
        .iter()
        .find(|evidence| {
            evidence.declaring_namespace == namespace
                && evidence.declaring_interface == interface
                && (evidence.method_name == method || evidence.vtable_index == slot)
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn registry_entries_have_unique_exact_identity_and_microsoft_citations() {
        let mut identities = BTreeSet::new();
        assert_eq!(BorrowedHwndOutputEvidence::entries().len(), 22);
        for evidence in BorrowedHwndOutputEvidence::entries() {
            assert!(identities.insert((
                evidence.declaring_namespace,
                evidence.declaring_interface,
                evidence.declaring_iid.to_ascii_lowercase(),
                evidence.method_name,
                evidence.vtable_index,
                evidence.parameter_index,
            )));
            assert_eq!(evidence.declaring_iid.len(), 36);
            assert!(evidence.parameter_index < evidence.parameter_count);
            assert!(!evidence.optional);
            assert!(
                evidence
                    .citation
                    .starts_with("https://learn.microsoft.com/")
            );
            assert!(!evidence.reason.is_empty());
        }
    }

    #[test]
    fn exact_lookup_rejects_identity_drift() {
        let evidence = BorrowedHwndOutputEvidence::entries()
            .iter()
            .find(|evidence| evidence.declaring_interface == "IOleWindow")
            .unwrap();
        assert!(
            registered_borrowed_hwnd_output(
                evidence.declaring_namespace,
                evidence.declaring_interface,
                evidence.declaring_iid,
                evidence.method_name,
                evidence.vtable_index,
                evidence.parameter_index,
            )
            .is_some()
        );
        assert!(
            registered_borrowed_hwnd_output(
                evidence.declaring_namespace,
                evidence.declaring_interface,
                "00000000-0000-0000-c000-000000000046",
                evidence.method_name,
                evidence.vtable_index,
                evidence.parameter_index,
            )
            .is_none()
        );
        assert!(
            borrowed_hwnd_evidence_for_declaration(
                evidence.declaring_namespace,
                evidence.declaring_interface,
                evidence.method_name,
                evidence.vtable_index + 1,
            )
            .is_some()
        );
    }
}
