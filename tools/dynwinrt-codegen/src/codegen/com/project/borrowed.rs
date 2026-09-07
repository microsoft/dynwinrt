// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::super::{
    ir::{
        CopyArguments, CopyResult, ProjectedBorrowedCopy, ProjectedBorrowedOperation,
        ProjectedBorrowedStorage,
    },
    model::borrowed::{CopyFamily, ValidatedStorage},
};

pub(super) fn project(storage: ValidatedStorage) -> Result<ProjectedBorrowedStorage, String> {
    let copy_only = storage
        .copy
        .as_ref()
        .is_some_and(|(family, _)| *family != CopyFamily::BitmapBgra8);
    let context_effects = storage
        .effects
        .into_iter()
        .map(|(slot, evidence)| {
            Ok((
                slot,
                serde_json::to_string(&serde_json::json!({
                    "version": 1,
                    "effect": evidence.effect,
                    "metadata_sha256": crate::com_metadata::borrowed::METADATA_SHA256,
                    "fingerprint": evidence.fingerprint,
                }))
                .map_err(|error| error.to_string())?,
            ))
        })
        .collect::<Result<_, String>>()?;
    let copy = storage
        .copy
        .map(|(family, plan)| {
            let op = |name, arguments, result| ProjectedBorrowedOperation {
                name,
                runtime_method: name,
                arguments,
                result,
            };
            let operations = match family {
                CopyFamily::AudioRender => vec![
                    op("writeFramesCopy", CopyArguments::Bytes, CopyResult::Void),
                    op("writeSilence", CopyArguments::Frames, CopyResult::Void),
                ],
                CopyFamily::AudioCapture => vec![op(
                    "readPacketCopy",
                    CopyArguments::None,
                    CopyResult::Packet,
                )],
                CopyFamily::BitmapBgra8 => vec![op(
                    "readLockedBgra8Copy",
                    CopyArguments::Rectangle,
                    CopyResult::Bitmap,
                )],
                CopyFamily::MediaBuffer => vec![
                    op("readCopy", CopyArguments::None, CopyResult::Bytes),
                    op("replaceCopy", CopyArguments::Bytes, CopyResult::Void),
                ],
            };
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
