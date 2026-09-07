// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::com_metadata::{ComInterfaceMeta, borrowed as evidence};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::codegen::com) enum CopyFamily {
    AudioRender,
    AudioCapture,
    BitmapBgra8,
    MediaBuffer,
}

#[derive(serde::Serialize)]
pub(in crate::codegen::com) struct NativeCall {
    iid: &'static str,
    slot: usize,
    storage: Vec<&'static str>,
}

#[derive(serde::Serialize)]
pub(in crate::codegen::com) struct BorrowedCopyPlan {
    version: u32,
    metadata_sha256: &'static str,
    access: &'static str,
    exposure: &'static str,
    owner: &'static str,
    acquired_hresult: i32,
    empty_hresult: Option<i32>,
    kind: &'static str,
    receiver: &'static str,
    acquire: NativeCall,
    queries: Vec<NativeCall>,
    finalize: Option<NativeCall>,
    extent: &'static str,
    finalization: &'static str,
    sta_only: bool,
    audio_origin_required: bool,
}

pub(in crate::codegen::com) struct ValidatedStorage {
    pub copy: Option<(CopyFamily, BorrowedCopyPlan)>,
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
    if evidence::is_audio_context(meta) {
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
        return Ok(Some(ValidatedStorage {
            copy: None,
            effects,
            dependencies,
        }));
    }
    let family = match (
        meta.interface.namespace.as_str(),
        meta.interface.name.as_str(),
    ) {
        (evidence::AUDIO, "IAudioRenderClient") => CopyFamily::AudioRender,
        (evidence::AUDIO, "IAudioCaptureClient") => CopyFamily::AudioCapture,
        (evidence::WIC, "IWICBitmap") => CopyFamily::BitmapBgra8,
        (evidence::MF, "IMFMediaBuffer") => CopyFamily::MediaBuffer,
        _ => return Err("Unknown borrowed-storage family".into()),
    };
    let dependency = if family == CopyFamily::BitmapBgra8 {
        Some((evidence::WIC, "IWICBitmapLock"))
    } else if matches!(family, CopyFamily::AudioRender | CopyFamily::AudioCapture) {
        Some((evidence::AUDIO, "IAudioClient"))
    } else {
        None
    };
    if let Some((namespace, name)) = dependency {
        let dependency = crate::com_metadata::parse_com_interface(paths, namespace, name)
            .ok_or_else(|| format!("Borrowed-copy dependency {namespace}.{name} is absent"))?;
        evidence::validate_interface(&dependency)?;
        for entry in evidence::catalog_entries(&dependency) {
            dependencies.add_exact(entry.entry_id, entry.family_id, entry.contract_kind);
        }
    }
    let call = |iid, slot, storage: &[&'static str]| NativeCall {
        iid,
        slot,
        storage: storage.to_vec(),
    };
    let (kind, receiver, acquire, queries, finalize, extent, finalization) = match family {
        CopyFamily::AudioRender => (
            "audio-render",
            evidence::RENDER_IID,
            call(evidence::RENDER_IID, 3, &["in-u32", "borrowed-bytes"]),
            vec![call(evidence::CLIENT_IID, 4, &["out-u32"])],
            Some(call(evidence::RENDER_IID, 4, &["in-u32", "in-u32"])),
            "initialized-frames",
            "render-full-or-zero",
        ),
        CopyFamily::AudioCapture => (
            "audio-capture",
            evidence::CAPTURE_IID,
            call(
                evidence::CAPTURE_IID,
                3,
                &["borrowed-bytes", "out-u32", "out-u32", "out-u64", "out-u64"],
            ),
            vec![call(evidence::CLIENT_IID, 4, &["out-u32"])],
            Some(call(evidence::CAPTURE_IID, 4, &["in-u32"])),
            "capture-packet-frames",
            "capture-full-or-zero",
        ),
        CopyFamily::BitmapBgra8 => (
            "bitmap-bgra8",
            evidence::BITMAP_IID,
            call(
                evidence::BITMAP_IID,
                8,
                &["in-rect", "in-u32", "owned-interface"],
            ),
            vec![
                call(evidence::BITMAP_IID, 3, &["out-u32", "out-u32"]),
                call(evidence::LOCK_IID, 3, &["out-u32", "out-u32"]),
                call(evidence::LOCK_IID, 4, &["out-u32"]),
                call(evidence::LOCK_IID, 6, &["out-guid"]),
                call(evidence::LOCK_IID, 5, &["out-u32", "borrowed-bytes"]),
            ],
            None,
            "native-bgra8-rows",
            "release-lock-reference",
        ),
        CopyFamily::MediaBuffer => (
            "media-buffer",
            evidence::MF_IID,
            call(
                evidence::MF_IID,
                3,
                &["borrowed-bytes", "out-u32", "out-u32"],
            ),
            vec![call(evidence::MF_IID, 6, &["in-u32"])],
            Some(call(evidence::MF_IID, 4, &[])),
            "native-current-and-max",
            "unlock",
        ),
    };
    Ok(Some(ValidatedStorage {
        effects: vec![],
        dependencies,
        copy: Some((
            family,
            BorrowedCopyPlan {
                version: 1,
                metadata_sha256: evidence::METADATA_SHA256,
                exposure: "owned-copy-only",
                access: match family {
                    CopyFamily::AudioRender => "write",
                    CopyFamily::AudioCapture | CopyFamily::BitmapBgra8 => "read",
                    CopyFamily::MediaBuffer => "read-write",
                },
                owner: match family {
                    CopyFamily::AudioRender | CopyFamily::AudioCapture => "audio-service-origin",
                    CopyFamily::BitmapBgra8 => "acquired-interface",
                    CopyFamily::MediaBuffer => "receiver",
                },
                acquired_hresult: 0,
                empty_hresult: (family == CopyFamily::AudioCapture).then_some(0x08890001),
                kind,
                receiver,
                acquire,
                queries,
                finalize,
                extent,
                finalization,
                sta_only: family == CopyFamily::BitmapBgra8,
                audio_origin_required: matches!(
                    family,
                    CopyFamily::AudioRender | CopyFamily::AudioCapture
                ),
            },
        )),
    }))
}
