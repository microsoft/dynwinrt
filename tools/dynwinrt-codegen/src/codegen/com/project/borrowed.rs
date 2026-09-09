// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::super::{
    ir::{
        CopyArguments, CopyResult, ProjectedBorrowedCopy, ProjectedBorrowedOperation,
        ProjectedBorrowedStorage,
    },
    model::borrowed::ValidatedStorage,
};
use dynwinrt_com_contracts::{InputKind, ResultMapping};

pub(super) fn project(storage: ValidatedStorage) -> Result<ProjectedBorrowedStorage, String> {
    let copy_only = storage.copy.as_ref().is_some_and(|copy| copy.copy_only);
    let context_effects = storage
        .effects
        .into_iter()
        .map(|(slot, evidence)| {
            Ok((
                slot,
                serde_json::to_string(&serde_json::json!({
                    "version": 1,
                    "effect": evidence.effect.as_deref(),
                    "metadata_sha256": crate::com_metadata::borrowed::METADATA_SHA256,
                    "fingerprint": evidence.fingerprint,
                }))
                .map_err(|error| error.to_string())?,
            ))
        })
        .collect::<Result<_, String>>()?;
    let copy = storage
        .copy
        .map(|plan| {
            let operations = plan
                .operations
                .iter()
                .map(|op| ProjectedBorrowedOperation {
                    name: op.api.name(),
                    runtime_method: op.api.name(),
                    arguments: match op.arguments {
                        InputKind::None => CopyArguments::None,
                        InputKind::Bytes => CopyArguments::Bytes,
                        InputKind::Frames => CopyArguments::Frames,
                        InputKind::Rectangle => CopyArguments::Rectangle,
                    },
                    result: match op.result {
                        ResultMapping::Void => CopyResult::Void,
                        ResultMapping::Bytes => CopyResult::Bytes,
                        ResultMapping::Packet { .. } => CopyResult::Packet,
                        ResultMapping::Bitmap { .. } => CopyResult::Bitmap,
                    },
                })
                .collect();
            Ok::<_, String>(ProjectedBorrowedCopy {
                descriptor: serde_json::to_string(&plan).map_err(|error| error.to_string())?,
                operations,
            })
        })
        .transpose()?;
    Ok(ProjectedBorrowedStorage {
        copy,
        context_effects,
        copy_only,
        evidence_dependencies: storage.dependencies,
    })
}

#[cfg(test)]
mod tests {
    use crate::codegen::com::{
        generate_com_interface_files, generate_complete_com_interface_files,
    };
    use crate::com_metadata::{borrowed as evidence, parse_com_interface};

    fn compare_public_baseline(name: &str, declarations: &str) {
        let Ok(root) = std::env::var("DYNWINRT_BORROWED_DECLARATION_BASELINE") else {
            return;
        };
        fn find(root: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
            for file in std::fs::read_dir(root).unwrap() {
                let file = file.unwrap();
                let path = file.path();
                if file.file_type().unwrap().is_dir() {
                    if let Some(found) = find(&path, name) {
                        return Some(found);
                    }
                } else if file.file_name() == format!("{name}.d.ts").as_str() {
                    return Some(path);
                }
            }
            None
        }
        let path = find(std::path::Path::new(&root), name).expect("baseline declaration");
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            declarations,
            "{name} public declarations must remain byte-for-byte unchanged"
        );
    }

    #[test]
    fn borrowed_copy_projection_is_bounded_and_renderer_is_ir_driven() {
        let Ok(paths) = std::env::var("DYNWINRT_WIN32_WINMD") else {
            return;
        };
        for (namespace, name, public_methods) in [
            (
                evidence::AUDIO,
                "IAudioRenderClient",
                vec!["writeFramesCopy", "writeSilence"],
            ),
            (
                evidence::AUDIO,
                "IAudioCaptureClient",
                vec!["readPacketCopy"],
            ),
            (evidence::WIC, "IWICBitmap", vec!["readLockedBgra8Copy"]),
            (
                evidence::MF,
                "IMFMediaBuffer",
                vec!["readCopy", "replaceCopy"],
            ),
        ] {
            let metadata = parse_com_interface(&paths, namespace, name).unwrap();
            let output = generate_com_interface_files(&metadata, &paths).unwrap();
            compare_public_baseline(name, &output.dts);
            for method in public_methods {
                assert!(output.js.contains(&format!(" {method}(")));
                assert!(output.dts.contains(&format!(" {method}(")));
            }
            if evidence::is_copy_only(&metadata) {
                assert!(
                    !output.js.contains(".addMethodAt("),
                    "copy-only wrapper must not publish unsafe native methods"
                );
                assert!(!output.dts.contains("lock(") && !output.dts.contains("getBuffer("));
                assert!(generate_complete_com_interface_files(&metadata, &paths).is_err());
            } else {
                assert!(
                    output.dts.contains("lock("),
                    "preserve IWICBitmap's existing safe owned-lock acquisition"
                );
                assert!(generate_complete_com_interface_files(&metadata, &paths).is_ok());
            }
            let mut projected = super::super::project_com_interface(&metadata, &paths).unwrap();
            let copy = projected
                .borrowed_storage
                .as_mut()
                .unwrap()
                .copy
                .as_mut()
                .unwrap();
            copy.operations[0].name = "renamedTransaction";
            let rendered =
                crate::codegen::com::javascript::render::render_com_interface(&projected).unwrap();
            assert!(rendered.js.contains(" renamedTransaction("));
            assert!(rendered.dts.contains(" renamedTransaction("));
            assert!(!rendered.js.contains("asPointerBigint"));
        }
        for name in ["IAudioClient", "IAudioClient2", "IAudioClient3"] {
            let metadata = parse_com_interface(&paths, evidence::AUDIO, name).unwrap();
            let projected = super::super::project_com_interface(&metadata, &paths).unwrap();
            assert!(projected.borrowed_storage.as_ref().unwrap().copy.is_none());
            assert_eq!(
                projected
                    .borrowed_storage
                    .as_ref()
                    .unwrap()
                    .context_effects
                    .len(),
                if name == "IAudioClient3" { 3 } else { 2 }
            );
            let output = generate_complete_com_interface_files(&metadata, &paths).unwrap();
            compare_public_baseline(name, &output.dts);
            assert!(output.js.contains(".withContextEffect("));
            assert!(
                output.js.contains("audio-initialize") && output.js.contains("audio-get-service")
            );
            assert!(output.dts.contains("getMixFormat("));
        }
        for name in ["IMF2DBuffer", "IMF2DBuffer2"] {
            let metadata = parse_com_interface(&paths, evidence::MF, name).unwrap();
            assert!(generate_com_interface_files(&metadata, &paths).is_err());
        }
    }
}
