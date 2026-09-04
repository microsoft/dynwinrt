// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExactOutParameterEvidence {
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

const EXACT_OUT_PARAMETERS: &[ExactOutParameterEvidence] = &[
    ExactOutParameterEvidence {
        declaring_namespace: "Windows.Win32.Media.MediaFoundation",
        declaring_interface: "IMFAttributes",
        declaring_iid: "2cd2d921-c447-44a7-a13c-4adabfc247e3",
        method_name: "GetString",
        vtable_index: 12,
        parameter_count: 4,
        parameter_index: 3,
        parameter_name: "pcchLength",
        source_fingerprint: "CF4D9EAF95123257840355F21F0623E137623EF95D205B63D5B1BB8758328DB0",
        reason: "IMFAttributes::GetString documents pcchLength as an optional pure output length",
        citation: "https://learn.microsoft.com/windows/win32/api/mfobjects/nf-mfobjects-imfattributes-getstring",
    },
    ExactOutParameterEvidence {
        declaring_namespace: "Windows.Win32.Media.MediaFoundation",
        declaring_interface: "IMFAttributes",
        declaring_iid: "2cd2d921-c447-44a7-a13c-4adabfc247e3",
        method_name: "GetItem",
        vtable_index: 3,
        parameter_count: 2,
        parameter_index: 1,
        parameter_name: "pValue",
        source_fingerprint: "0D8781FF90DF92EA09042A8386C01B7E8DEE1B2E546B3E616D2498896330BDF0",
        reason: "IMFAttributes::GetItem documents pValue as an optional pure output copy initialized by the method",
        citation: "https://learn.microsoft.com/windows/win32/api/mfobjects/nf-mfobjects-imfattributes-getitem",
    },
    ExactOutParameterEvidence {
        declaring_namespace: "Windows.Win32.Media.MediaFoundation",
        declaring_interface: "IMFAttributes",
        declaring_iid: "2cd2d921-c447-44a7-a13c-4adabfc247e3",
        method_name: "GetItemByIndex",
        vtable_index: 31,
        parameter_count: 3,
        parameter_index: 2,
        parameter_name: "pValue",
        source_fingerprint: "1D349D3E79CC839831A7F6E87CC2656C2D153A136CB397815850D2417B880DAE",
        reason: "IMFAttributes::GetItemByIndex documents pValue as an optional pure output copy initialized by the method",
        citation: "https://learn.microsoft.com/windows/win32/api/mfobjects/nf-mfobjects-imfattributes-getitembyindex",
    },
];

pub(crate) const fn entries() -> &'static [ExactOutParameterEvidence] {
    EXACT_OUT_PARAMETERS
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
