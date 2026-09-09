// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;

fn reject(kind: CopyKind, edit: impl FnOnce(&mut BorrowedCopyContract), reason: &str) {
    let mut contract = BorrowedCopyContract::expected(kind);
    edit(&mut contract);
    let error = registry().validate(&contract).unwrap_err();
    assert!(error.contains(reason), "{error}, expected {reason}");
    assert!(registry().admit(&contract).is_err());
}

#[test]
fn packaged_borrowed_copy_recipes_are_typed_and_admitted() {
    assert_eq!(registry().copies.len(), 4);
    assert_eq!(registry().evidence.len(), 26);
    for contract in &registry().copies {
        registry().admit(contract).unwrap();
        assert_eq!(
            BorrowedCopyContract::parse(&serde_json::to_string(contract).unwrap()).unwrap(),
            *contract,
        );
    }
}

#[test]
fn borrowed_copy_rejects_wrong_native_cells_and_reference_units() {
    reject(
        CopyKind::MediaBuffer,
        |c| {
            c.calls[0].bindings[1] = Binding::OutU64 {
                name: "maximum".into(),
            };
        },
        "ABI cells",
    );
    reject(
        CopyKind::MediaBuffer,
        |c| {
            let Binding::OutU32 { unit, .. } = &mut c.calls[0].bindings[1] else {
                panic!()
            };
            *unit = Unit::Frames;
        },
        "Wrong output/reference type",
    );
    reject(
        CopyKind::MediaBuffer,
        |c| {
            let Binding::OutU32 { name, .. } = &mut c.calls[0].bindings[1] else {
                panic!()
            };
            *name = "data".into();
        },
        "Duplicate value/output",
    );
}

#[test]
fn borrowed_copy_rejects_dangling_forward_and_uninitialized_sources() {
    reject(
        CopyKind::AudioRender,
        |c| {
            c.calls[1].bindings[0] = Binding::InU32 {
                value: U32Source::Frames(FrameCountRef("not-produced".into())),
            };
        },
        "Dangling, forward or uninitialized",
    );
    reject(
        CopyKind::MediaBuffer,
        |c| {
            // A source from another operation is not initialized here.
            let Step::NativeExtent { length, .. } = &mut c.operations[0].after[0] else {
                panic!()
            };
            length.0 = "input.bytes".into();
        },
        "Dangling, forward or uninitialized",
    );
    reject(
        CopyKind::BitmapBgra8,
        |c| {
            c.operations[0].before.insert(
                0,
                Step::Call {
                    call: "lock-size".into(),
                },
            );
        },
        "Unavailable acquired owner",
    );
    reject(
        CopyKind::AudioRender,
        |c| {
            c.operations[0].before.swap(2, 3);
        },
        "Dangling, forward or uninitialized",
    );
}

#[test]
fn borrowed_copy_rejects_invalid_bounds_results_and_cleanup_dependencies() {
    reject(
        CopyKind::MediaBuffer,
        |c| {
            let Step::NativeExtent { length, .. } = &mut c.operations[1].after[0] else {
                panic!()
            };
            length.0 = "maximum".into();
        },
        "Copy length",
    );
    reject(
        CopyKind::AudioCapture,
        |c| {
            let Cleanup::Call { abort, .. } = &mut c.operations[0].cleanup else {
                panic!()
            };
            abort[0] = U32Source::Frames(FrameCountRef("frames".into()));
        },
        "Dangling, forward or uninitialized",
    );
    reject(
        CopyKind::AudioCapture,
        |c| {
            c.calls[2].bindings[0] = Binding::InU32 {
                value: U32Source::Constant(0),
            };
        },
        "Missing full/zero",
    );
    reject(
        CopyKind::MediaBuffer,
        |c| c.operations[1].commit.clear(),
        "requires its native length commit",
    );
    reject(
        CopyKind::BitmapBgra8,
        |c| {
            c.operations[0].cleanup = Cleanup::ReleaseOwner {
                owner: OwnerRef("other-lock".into()),
            };
        },
        "unique acquired cleanup owner",
    );
    reject(
        CopyKind::BitmapBgra8,
        |c| {
            let ResultMapping::Bitmap { width, .. } = &mut c.operations[0].result else {
                panic!()
            };
            width.0 = "bitmap-width".into();
        },
        "Bitmap result",
    );
    reject(
        CopyKind::MediaBuffer,
        |c| c.access = MemoryAccess::Read,
        "access policy",
    );
}

#[test]
fn borrowed_copy_unknown_evidence_and_old_descriptors_fail_closed() {
    reject(
        CopyKind::MediaBuffer,
        |c| c.calls[0].evidence = "unreviewed.Lock".into(),
        "Unknown registry evidence",
    );
    let mut c = BorrowedCopyContract::expected(CopyKind::MediaBuffer);
    c.version = 1;
    let error = BorrowedCopyContract::parse(&serde_json::to_string(&c).unwrap()).unwrap_err();
    assert!(error.contains("fully regenerate"));
    c.version = 2;
    c.id = "matching-looking-json".into();
    assert!(
        registry()
            .admit(&c)
            .unwrap_err()
            .contains("fully regenerate")
    );
}

#[test]
fn borrowed_copy_duplicate_json_fields_and_call_bindings_are_rejected() {
    let c = BorrowedCopyContract::expected(CopyKind::MediaBuffer);
    let json = serde_json::to_string(&c).unwrap();
    let duplicate = json.replacen("\"version\":2", "\"version\":2,\"version\":2", 1);
    assert!(BorrowedCopyContract::parse(&duplicate).is_err());
    reject(
        CopyKind::MediaBuffer,
        |c| c.calls.push(c.calls[0].clone()),
        "Duplicate call binding",
    );
}

#[test]
fn borrowed_copy_silence_requires_a_typed_normal_commit_flag() {
    reject(
        CopyKind::AudioRender,
        |c| {
            let release = c
                .calls
                .iter_mut()
                .find(|call| call.id == "release-silent")
                .unwrap();
            release.bindings[1] = Binding::InU32 {
                value: U32Source::Constant(0),
            };
        },
        "typed silent-flag",
    );
    reject(
        CopyKind::AudioRender,
        |c| {
            let Cleanup::Call { abort, .. } = &mut c.operations[1].cleanup else {
                panic!()
            };
            abort[1] = U32Source::SilentFlag(2);
        },
        "Abort cannot commit",
    );
}

#[test]
fn borrowed_copy_cannot_alias_an_acquisition_as_an_after_query() {
    reject(
        CopyKind::AudioRender,
        |c| {
            let mut alias = c
                .calls
                .iter()
                .find(|call| call.id == "acquire")
                .unwrap()
                .clone();
            alias.id = "another-acquisition".into();
            c.calls.push(alias);
            c.operations[0].after.push(Step::Call {
                call: "another-acquisition".into(),
            });
        },
        "Acquisition evidence",
    );
}

#[test]
fn borrowed_copy_cleanup_must_close_its_reviewed_acquisition() {
    reject(
        CopyKind::MediaBuffer,
        |c| {
            let Cleanup::Call { call: id, .. } = &c.operations[0].cleanup else {
                panic!()
            };
            let unlock = c.calls.iter_mut().find(|call| call.id == *id).unwrap();
            unlock.evidence = "IMFMediaBuffer.SetCurrentLength".into();
            unlock.bindings = vec![Binding::InU32 {
                value: U32Source::Constant(0),
            }];
            for operation in &mut c.operations {
                let Cleanup::Call { abort, .. } = &mut operation.cleanup else {
                    panic!()
                };
                *abort = vec![U32Source::Constant(0)];
            }
        },
        "Cleanup does not close the reviewed acquisition",
    );
}

#[test]
fn borrowed_copy_native_units_are_positional_on_all_call_paths() {
    for id in ["release", "release-silent"] {
        reject(
            CopyKind::AudioRender,
            |c| {
                c.calls
                    .iter_mut()
                    .find(|call| call.id == id)
                    .unwrap()
                    .bindings
                    .swap(0, 1);
            },
            "Wrong output/reference type for native ABI cell",
        );
    }
    reject(
        CopyKind::AudioCapture,
        |c| {
            c.calls
                .iter_mut()
                .find(|call| call.id == "acquire")
                .unwrap()
                .bindings
                .swap(1, 2);
        },
        "Wrong output/reference type for native ABI cell",
    );
    reject(
        CopyKind::AudioRender,
        |c| {
            let Cleanup::Call { abort, .. } = &mut c.operations[0].cleanup else {
                panic!()
            };
            abort[1] = U32Source::Frames(FrameCountRef("capacity".into()));
        },
        "Wrong output/reference type for native abort ABI cell",
    );
}
