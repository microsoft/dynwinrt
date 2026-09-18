// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::com_metadata::{ComInterfaceMeta, borrowed as evidence};
use dynwinrt_com_contracts::{BorrowedCopyContract, registry};

pub(in crate::codegen::com) struct ValidatedStorage {
    pub copy: Option<BorrowedCopyContract>,
    pub effects: Vec<(usize, &'static evidence::Evidence)>,
    pub dependencies: crate::contract_registry::EvidenceDependencies,
}

pub(in crate::codegen::com) fn validate(
    meta: &ComInterfaceMeta,
    paths: &str,
) -> Result<Option<ValidatedStorage>, String> {
    if !evidence::is_audio_context(meta) && !evidence::is_bounded_copy(meta) {
        return Ok(None);
    }
    evidence::require_metadata_hash(paths)?;
    evidence::validate_interface(meta)?;
    let mut dependencies = crate::com_metadata::collect_evidence_dependencies(meta);
    let effects = meta
        .raw_methods
        .as_ref()
        .expect("validated raw evidence")
        .iter()
        .filter_map(|raw| evidence::evidence_for(raw).transpose())
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|entry| entry.effect.is_some())
        .map(|entry| (entry.slot, entry))
        .collect();
    let copy = registry()
        .select(
            &meta.interface.namespace,
            &meta.interface.name,
            &meta.interface.iid,
        )
        .cloned();
    if let Some(copy) = &copy {
        registry().admit(copy)?;
        for identity in &copy.dependencies {
            let record = registry().interface(identity)?;
            let dependency =
                crate::com_metadata::parse_com_interface(paths, &record.namespace, &record.name)
                    .ok_or_else(|| format!("Borrowed-copy dependency {identity} is absent"))?;
            evidence::validate_interface(&dependency)?;
            for entry in evidence::catalog_entries(&dependency) {
                dependencies.add_exact(entry.entry_id, entry.family_id, entry.contract_kind);
            }
        }
    }
    Ok(Some(ValidatedStorage {
        copy,
        effects,
        dependencies,
    }))
}
