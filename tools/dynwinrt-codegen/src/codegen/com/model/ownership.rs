// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::abi::QualifiedName;
use super::diagnostics::{ModelError, UnsupportedReason};
use super::ids::CleanupId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) struct HandleCleanup {
    function: QualifiedName,
}

impl HandleCleanup {
    pub(super) const fn new(function: QualifiedName) -> Self {
        Self { function }
    }

    pub(in crate::codegen::com) const fn function(&self) -> &QualifiedName {
        &self.function
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) enum ComOwnership {
    Borrowed,
    ComOwned,
    CoTaskMemOwned,
    BstrOwned,
    BstrReplaced,
    HStringOwned,
    VariantOwned,
    SafeArrayOwned,
    PropVariantOwned,
    ExcepInfoOwned,
    StatStgOwned,
    FormatEtcOwned,
    StgMediumOwned,
    LocalOwned,
    HandleOwned(HandleCleanup),
    CustomOwned(CleanupId),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) enum Cleanup {
    None,
    ComRelease,
    CoTaskMemFree,
    SysFreeString,
    WindowsDeleteString,
    VariantClear,
    SafeArrayDestroy,
    PropVariantClear,
    ExcepInfoClear,
    StatStgClear,
    FormatEtcClear,
    ReleaseStgMedium,
    LocalFree,
    Handle(HandleCleanup),
    Custom(CleanupId),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CustomCleanup {
    function: QualifiedName,
}

impl CustomCleanup {
    pub(super) const fn new(function: QualifiedName) -> Self {
        Self { function }
    }

    pub(super) const fn function(&self) -> &QualifiedName {
        &self.function
    }
}

#[derive(Debug, Default)]
pub(super) struct CleanupTable {
    definitions: Vec<CustomCleanup>,
}

impl CleanupTable {
    pub(super) fn insert(&mut self, definition: CustomCleanup) -> Result<CleanupId, ModelError> {
        let id = CleanupId::from_index(self.definitions.len())
            .ok_or(ModelError::CapacityExceeded("custom cleanup"))?;
        self.definitions.push(definition);
        Ok(id)
    }

    pub(super) fn get(&self, id: CleanupId) -> Result<&CustomCleanup, ModelError> {
        self.definitions
            .get(id.index())
            .ok_or(ModelError::UnknownId {
                kind: "custom cleanup",
                index: id.index(),
            })
    }
}

pub(super) fn validate_ownership_cleanup(
    ownership: &ComOwnership,
    cleanup: &Cleanup,
) -> Result<(), ModelError> {
    let valid = match (ownership, cleanup) {
        (ComOwnership::Borrowed, Cleanup::None)
        | (ComOwnership::ComOwned, Cleanup::ComRelease)
        | (ComOwnership::CoTaskMemOwned, Cleanup::CoTaskMemFree)
        | (ComOwnership::BstrOwned, Cleanup::SysFreeString)
        | (ComOwnership::BstrReplaced, Cleanup::SysFreeString)
        | (ComOwnership::HStringOwned, Cleanup::WindowsDeleteString)
        | (ComOwnership::VariantOwned, Cleanup::VariantClear)
        | (ComOwnership::SafeArrayOwned, Cleanup::SafeArrayDestroy)
        | (ComOwnership::PropVariantOwned, Cleanup::PropVariantClear)
        | (ComOwnership::ExcepInfoOwned, Cleanup::ExcepInfoClear)
        | (ComOwnership::StatStgOwned, Cleanup::StatStgClear)
        | (ComOwnership::FormatEtcOwned, Cleanup::FormatEtcClear)
        | (ComOwnership::StgMediumOwned, Cleanup::ReleaseStgMedium)
        | (ComOwnership::LocalOwned, Cleanup::LocalFree) => true,
        (ComOwnership::HandleOwned(expected), Cleanup::Handle(actual)) => expected == actual,
        (ComOwnership::CustomOwned(expected), Cleanup::Custom(actual)) => expected == actual,
        _ => false,
    };
    if valid {
        Ok(())
    } else if matches!(ownership, ComOwnership::Unknown) {
        Err(ModelError::Unsupported(UnsupportedReason::UnknownOwnership))
    } else if matches!(cleanup, Cleanup::Unknown) {
        Err(ModelError::Unsupported(UnsupportedReason::UnknownCleanup))
    } else {
        Err(ModelError::InvalidOwnership(format!(
            "{ownership:?} cannot use {cleanup:?}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocator_pairs_are_exact() {
        validate_ownership_cleanup(&ComOwnership::BstrOwned, &Cleanup::SysFreeString).unwrap();
        validate_ownership_cleanup(&ComOwnership::BstrReplaced, &Cleanup::SysFreeString).unwrap();
        assert!(
            validate_ownership_cleanup(&ComOwnership::BstrOwned, &Cleanup::CoTaskMemFree).is_err()
        );
        assert!(validate_ownership_cleanup(&ComOwnership::BstrReplaced, &Cleanup::None).is_err());
        assert!(
            validate_ownership_cleanup(&ComOwnership::CoTaskMemOwned, &Cleanup::SysFreeString)
                .is_err()
        );
    }

    #[test]
    fn handle_cleanup_requires_exact_identity() {
        let close_handle = HandleCleanup::new(
            QualifiedName::new("Windows.Win32.Foundation", "CloseHandle").unwrap(),
        );
        let reg_close = HandleCleanup::new(
            QualifiedName::new("Windows.Win32.System.Registry", "RegCloseKey").unwrap(),
        );

        validate_ownership_cleanup(
            &ComOwnership::HandleOwned(close_handle.clone()),
            &Cleanup::Handle(close_handle),
        )
        .unwrap();
        assert!(
            validate_ownership_cleanup(
                &ComOwnership::HandleOwned(reg_close),
                &Cleanup::Handle(HandleCleanup::new(
                    QualifiedName::new("Windows.Win32.Foundation", "CloseHandle").unwrap()
                )),
            )
            .is_err()
        );
    }

    #[test]
    fn unknown_ownership_fails_closed() {
        assert_eq!(
            validate_ownership_cleanup(&ComOwnership::Unknown, &Cleanup::Unknown),
            Err(ModelError::Unsupported(UnsupportedReason::UnknownOwnership))
        );
    }
}
