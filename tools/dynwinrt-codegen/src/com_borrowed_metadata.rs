// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Verifies configured WinMD against the shared reviewed COM registry.
//! Registry records, not interface-name branches, select copy capabilities.

use super::{ComInterfaceMeta, RawComMethod, raw_method_fingerprint};
use crate::contract_registry::{
    ContractKind, ExactEntrySelector, ExactFamilyId, ExactRegistryEntry,
};
pub use dynwinrt_com_contracts::{Evidence, METADATA_SHA256};
use dynwinrt_com_contracts::{InterfaceRecord, registry};
use sha2::{Digest, Sha256};

#[cfg(test)]
pub const AUDIO: &str = "Windows.Win32.Media.Audio";
#[cfg(test)]
pub const WIC: &str = "Windows.Win32.Graphics.Imaging";
#[cfg(test)]
pub const MF: &str = "Windows.Win32.Media.MediaFoundation";

pub fn all_evidence() -> &'static [Evidence] {
    &registry().evidence
}

pub(crate) fn catalog_entry(entry: &Evidence) -> ExactRegistryEntry {
    let family = if entry.effect.is_some() {
        ExactFamilyId::AudioContext
    } else {
        ExactFamilyId::BorrowedCopy
    };
    let selector = ExactEntrySelector {
        namespace: entry.namespace.clone(),
        interface: entry.interface.clone(),
        iid: entry.iid.clone(),
        method: entry.method.clone(),
        slot: entry.slot,
        parameter: None,
    };
    ExactRegistryEntry {
        entry_id: selector.entry_id(family), selector, family_id: family,
        contract_kind: if entry.effect.is_some() { ContractKind::ContextualEffect } else { ContractKind::BorrowedStorage },
        source_fingerprint: entry.fingerprint.clone(),
        reason: if entry.effect.is_some() {
            "Only the actual successful metadata-described native call may create immutable initialization or service-origin provenance"
        } else {
            "Bounded owned-copy storage/extent/finalization plan; native borrowed bytes and unsupported methods are not public"
        }.into(),
        citation: entry.citation.clone(),
    }
}

pub fn evidence_for(raw: &RawComMethod) -> Result<Option<&'static Evidence>, String> {
    let selected = all_evidence().iter().find(|entry| {
        entry.namespace == raw.declaring_namespace
            && entry.interface == raw.declaring_interface
            && entry.method == raw.metadata_name
    });
    if let Some(entry) = selected
        && (entry.iid != raw.declaring_iid
            || entry.slot != raw.vtable_index
            || entry.fingerprint != raw_method_fingerprint(raw))
    {
        return Err(format!(
            "Borrowed-copy/context evidence drift: {}.{}::{}",
            entry.namespace, entry.interface, entry.method
        ));
    }
    Ok(selected)
}

fn candidate(meta: &ComInterfaceMeta) -> Option<&'static InterfaceRecord> {
    registry().interfaces.iter().find(|entry| {
        entry.namespace == meta.interface.namespace && entry.name == meta.interface.name
    })
}

pub fn is_audio_context(meta: &ComInterfaceMeta) -> bool {
    candidate(meta).is_some_and(|entry| entry.context)
}

pub fn is_bounded_copy(meta: &ComInterfaceMeta) -> bool {
    // Detect a drifted IID as a candidate too, so validation rejects instead of
    // falling back to a projection without its required copy/context evidence.
    candidate(meta).is_some_and(|entry| {
        registry()
            .copies
            .iter()
            .any(|copy| copy.receiver == entry.identity())
    })
}

pub fn is_copy_only(meta: &ComInterfaceMeta) -> bool {
    registry()
        .select(
            &meta.interface.namespace,
            &meta.interface.name,
            &meta.interface.iid,
        )
        .is_some_and(|copy| copy.copy_only)
}

pub(crate) fn catalog_entries(meta: &ComInterfaceMeta) -> Vec<ExactRegistryEntry> {
    if !candidate(meta).is_some_and(|entry| entry.catalog) {
        return vec![];
    }
    meta.raw_methods
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|raw| evidence_for(raw).ok().flatten())
        .map(catalog_entry)
        .collect()
}

pub fn require_metadata_hash(paths: &str) -> Result<(), String> {
    let matching = paths
        .split(';')
        .filter(|path| !path.trim().is_empty())
        .any(|path| {
            std::fs::read(path)
                .ok()
                .is_some_and(|bytes| format!("{:X}", Sha256::digest(bytes)) == METADATA_SHA256)
        });
    if matching {
        Ok(())
    } else {
        Err("Borrowed-copy plans require the pinned Win32Metadata 71.0.14-preview SHA256".into())
    }
}

pub fn validate_interface(meta: &ComInterfaceMeta) -> Result<(), String> {
    let entry = candidate(meta).ok_or("Not an exact borrowed-copy/context interface")?;
    let methods = meta
        .raw_methods
        .as_deref()
        .ok_or("Borrowed-copy/context raw evidence is absent")?;
    if meta.interface.iid != entry.iid
        || !meta.is_iunknown_rooted
        || meta.base_offset != 3
        || meta.own_methods_start != entry.own_start
        || meta.base_chain != entry.bases
        || meta.base_iids != entry.base_iids
        || meta.interface.generic_piid.is_some()
        || !meta.interface.generic_args.is_empty()
        || methods.len() != entry.last - 2
        || meta.interface.methods.len() != methods.len()
        || methods
            .iter()
            .enumerate()
            .any(|(index, raw)| raw.vtable_index != index + 3)
    {
        return Err(format!(
            "Borrowed-copy/context identity or complete vtable drift: {}",
            meta.interface.name
        ));
    }
    for raw in methods {
        let evidence = evidence_for(raw)?;
        if (!entry.context || entry.required_evidence.contains(&raw.vtable_index))
            && evidence.is_none()
        {
            return Err(format!(
                "Absent borrowed-storage/context evidence at slot {}",
                raw.vtable_index
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_copy_metadata_evidence_and_fail_closed_drift() {
        let Ok(winmd) = std::env::var("DYNWINRT_WIN32_WINMD") else {
            return;
        };
        require_metadata_hash(&winmd).unwrap();
        for (namespace, name) in [
            (AUDIO, "IAudioClient3"),
            (AUDIO, "IAudioRenderClient"),
            (AUDIO, "IAudioCaptureClient"),
            (WIC, "IWICBitmap"),
            (WIC, "IWICBitmapLock"),
            (MF, "IMFMediaBuffer"),
        ] {
            let meta = super::super::parse_com_interface(&winmd, namespace, name).unwrap();
            validate_interface(&meta).unwrap();
            let mut drift = meta.clone();
            drift.raw_methods = None;
            assert!(validate_interface(&drift).is_err());
            let mut drift = meta.clone();
            drift.interface.iid = "1cb9ad4c-dbfa-4c32-b178-c2f568a703b2".into();
            assert!(validate_interface(&drift).is_err());
            for index in 0..meta.raw_methods.as_ref().unwrap().len() {
                if evidence_for(&meta.raw_methods.as_ref().unwrap()[index])
                    .unwrap()
                    .is_none()
                {
                    continue;
                }
                let mut drift = meta.clone();
                drift.raw_methods.as_mut().unwrap()[index]
                    .params
                    .iter_mut()
                    .for_each(|param| param.optional = !param.optional);
                if !drift.raw_methods.as_ref().unwrap()[index].params.is_empty() {
                    assert!(validate_interface(&drift).is_err(), "{name} slot {index}");
                }
                let mut drift = meta.clone();
                drift.raw_methods.as_mut().unwrap()[index].vtable_index += 1;
                assert!(validate_interface(&drift).is_err());
            }
        }
        assert!(require_metadata_hash("").is_err());
    }
}
