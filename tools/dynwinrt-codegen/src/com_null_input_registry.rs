// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExactNullInputEvidence {
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

impl ExactNullInputEvidence {
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
        crate::contract_registry::ExactFamilyId::ReservedNullInput
    }

    pub(crate) const fn contract_kind(&self) -> crate::contract_registry::ContractKind {
        crate::contract_registry::ContractKind::NullInput
    }
}

const EXACT_NULL_INPUTS: &[ExactNullInputEvidence] = &[
    ExactNullInputEvidence {
        declaring_namespace: "Windows.Win32.System.Com.StructuredStorage",
        declaring_interface: "IStorage",
        declaring_iid: "0000000b-0000-0000-c000-000000000046",
        method_name: "OpenStream",
        vtable_index: 4,
        parameter_count: 5,
        parameter_index: 1,
        parameter_name: "reserved1",
        source_fingerprint: "A72D743B32CCE55D17F522F6695095CF08BBCDCF4FA54E29E3275B24B30716F0",
        reason: "IStorage::OpenStream requires reserved1 to be native null",
        citation: "https://learn.microsoft.com/windows/win32/api/objidl/nf-objidl-istorage-openstream",
    },
    ExactNullInputEvidence {
        declaring_namespace: "Windows.Win32.System.Com.StructuredStorage",
        declaring_interface: "IStorage",
        declaring_iid: "0000000b-0000-0000-c000-000000000046",
        method_name: "EnumElements",
        vtable_index: 11,
        parameter_count: 4,
        parameter_index: 1,
        parameter_name: "reserved2",
        source_fingerprint: "77D49C81B42D6B6961239D06E42A8728EC4CDDDFFF0A0174DD3FC248D17071C6",
        reason: "IStorage::EnumElements requires reserved2 to be native null",
        citation: "https://learn.microsoft.com/windows/win32/api/objidl/nf-objidl-istorage-enumelements",
    },
];

pub(crate) const fn entries() -> &'static [ExactNullInputEvidence] {
    EXACT_NULL_INPUTS
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn entries_are_unique_selector_specific_and_cited() {
        let mut selectors = BTreeSet::new();
        assert_eq!(entries().len(), 2);
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
