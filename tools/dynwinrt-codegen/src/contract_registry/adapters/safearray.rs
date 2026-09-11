// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::com_metadata::{RawSafeArrayEvidence, RawSafeArrayOwnership, RawSafeArrayVartype};
use crate::contract_registry::{self, SafeArrayContract, SafeArrayOwnership, SafeArrayVartype};
use std::sync::OnceLock;

pub(crate) fn registered_safe_array_evidence(
    namespace: &str,
    interface: &str,
    interface_iid: &str,
    method: &str,
    vtable_index: usize,
    parameter_index: usize,
) -> Option<RawSafeArrayEvidence> {
    all_safe_array_evidence()
        .iter()
        .find(|evidence| {
            evidence.declaring_namespace == namespace
                && evidence.declaring_interface == interface
                && evidence.declaring_iid.eq_ignore_ascii_case(interface_iid)
                && evidence.method_name == method
                && evidence.vtable_index == vtable_index
                && evidence.parameter_index == parameter_index
        })
        .cloned()
}

pub(crate) fn safe_array_evidence_for_declaration(
    namespace: &str,
    interface: &str,
    method: &str,
    parameter_index: usize,
) -> Option<RawSafeArrayEvidence> {
    all_safe_array_evidence()
        .iter()
        .find(|evidence| {
            evidence.declaring_namespace == namespace
                && evidence.declaring_interface == interface
                && evidence.method_name == method
                && evidence.parameter_index == parameter_index
        })
        .cloned()
}

pub(crate) fn safe_array_output_allows_null(evidence: &RawSafeArrayEvidence) -> bool {
    contract_for_evidence(evidence)
        .is_some_and(|entry| entry.contract.nullability.allows_null_output())
}

pub(crate) fn contract_for_evidence(
    evidence: &RawSafeArrayEvidence,
) -> Option<&'static SafeArrayContract> {
    contract_registry::safe_array_contracts()
        .expect("embedded SAFEARRAY contract registry must validate")
        .iter()
        .zip(all_safe_array_evidence())
        .find_map(|(entry, registered)| (registered == evidence).then_some(entry))
}

pub(crate) fn all_safe_array_evidence() -> &'static [RawSafeArrayEvidence] {
    static EVIDENCE: OnceLock<Vec<RawSafeArrayEvidence>> = OnceLock::new();
    EVIDENCE.get_or_init(|| {
        contract_registry::safe_array_contracts()
            .expect("embedded SAFEARRAY contract registry must validate")
            .iter()
            .map(|entry| {
                let selector = &entry.selector;
                RawSafeArrayEvidence {
                    declaring_namespace: &selector.interface.namespace,
                    declaring_interface: &selector.interface.name,
                    declaring_iid: &selector.declaring_iid,
                    method_name: &selector.method,
                    vtable_index: selector.absolute_slot,
                    parameter_index: entry.contract.parameter_index,
                    parameter_name: &selector.parameters[entry.contract.parameter_index].name,
                    element_vartype: match entry.contract.element.vartype {
                        SafeArrayVartype::I4 => RawSafeArrayVartype::I4,
                        SafeArrayVartype::Ui1 => RawSafeArrayVartype::Ui1,
                        SafeArrayVartype::Ui4 => RawSafeArrayVartype::Ui4,
                        SafeArrayVartype::R8 => RawSafeArrayVartype::R8,
                        SafeArrayVartype::Bstr => RawSafeArrayVartype::Bstr,
                        SafeArrayVartype::Unknown => RawSafeArrayVartype::Unknown,
                        SafeArrayVartype::Variant => RawSafeArrayVartype::Variant,
                    },
                    element_iid: entry.contract.element.interface_iid.as_deref(),
                    ownership: match entry.contract.ownership {
                        SafeArrayOwnership::BorrowedInput => RawSafeArrayOwnership::BorrowedInput,
                        SafeArrayOwnership::OwnedOutput => RawSafeArrayOwnership::OwnedOutput,
                    },
                    raw_method_shape: selector.raw_method_shape(),
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
    fn registry_keys_and_evidence_are_exact() {
        let mut keys = std::collections::BTreeSet::new();
        assert_eq!(all_safe_array_evidence().len(), 209);
        for evidence in all_safe_array_evidence() {
            assert!(keys.insert((
                evidence.declaring_namespace,
                evidence.declaring_interface,
                evidence.declaring_iid,
                evidence.method_name,
                evidence.vtable_index,
                evidence.parameter_index,
            )));
            assert!(
                evidence
                    .citation
                    .starts_with("https://learn.microsoft.com/")
            );
            assert!(!evidence.reason.is_empty());
            assert!(!evidence.raw_method_shape.is_empty());
            assert_eq!(
                evidence.element_iid.is_some(),
                evidence.element_vartype == RawSafeArrayVartype::Unknown
            );
            assert_eq!(
                registered_safe_array_evidence(
                    evidence.declaring_namespace,
                    evidence.declaring_interface,
                    evidence.declaring_iid,
                    evidence.method_name,
                    evidence.vtable_index,
                    evidence.parameter_index,
                )
                .as_ref(),
                Some(evidence)
            );
        }
        assert_eq!(
            all_safe_array_evidence()
                .iter()
                .filter(|evidence| safe_array_output_allows_null(evidence))
                .map(|evidence| (
                    evidence.declaring_interface,
                    evidence.method_name,
                    evidence.vtable_index,
                    evidence.parameter_index,
                ))
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([
                (
                    "IRawElementProviderFragment",
                    "GetEmbeddedFragmentRoots",
                    6,
                    0
                ),
                ("IRawElementProviderFragment", "GetRuntimeId", 4, 0),
                ("IDragProvider", "GetGrabbedItems", 6, 0),
                ("ITextProvider", "GetSelection", 3, 0),
            ])
        );
    }

    #[test]
    fn documented_vartype_values_match_automation_constants() {
        use windows::Win32::System::Variant::{
            VT_BSTR, VT_I4, VT_R8, VT_UI1, VT_UI4, VT_UNKNOWN, VT_VARIANT,
        };

        assert_eq!(RawSafeArrayVartype::I4.value(), VT_I4.0);
        assert_eq!(RawSafeArrayVartype::R8.value(), VT_R8.0);
        assert_eq!(RawSafeArrayVartype::Bstr.value(), VT_BSTR.0);
        assert_eq!(RawSafeArrayVartype::Variant.value(), VT_VARIANT.0);
        assert_eq!(RawSafeArrayVartype::Unknown.value(), VT_UNKNOWN.0);
        assert_eq!(RawSafeArrayVartype::Ui1.value(), VT_UI1.0);
        assert_eq!(RawSafeArrayVartype::Ui4.value(), VT_UI4.0);
    }

    #[test]
    fn documented_interface_iids_match_windows_rs() {
        use windows::Win32::UI::Accessibility::{
            IRawElementProviderFragmentRoot, IRawElementProviderSimple, ITextRangeProvider,
            IUIAutomationCondition,
        };
        use windows::core::Interface;

        assert_eq!(
            ITextRangeProvider::IID,
            windows::core::GUID::from_u128(0x5347ad7b_c355_46f8_aff5_909033582f63)
        );
        assert_eq!(
            IRawElementProviderSimple::IID,
            windows::core::GUID::from_u128(0xd6dd68d1_86fd_4332_8666_9abedea2d24c)
        );
        assert_eq!(
            IRawElementProviderFragmentRoot::IID,
            windows::core::GUID::from_u128(0x620ce2a5_ab8f_40a9_86cb_de3c75599b58)
        );
        assert_eq!(
            IUIAutomationCondition::IID,
            windows::core::GUID::from_u128(0x352ffba8_0973_437c_a61f_f64cafd81df9)
        );
    }
}
