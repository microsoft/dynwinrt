// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use contracts::{Binding, Cleanup, OperationId, Registry, Step, U32Source, Unit};
use testing::reordered;

fn record(registry: &Registry, name: &str) -> BorrowedCopyContract {
    let identity = registry
        .interface(&format!("Tests.Borrowed.{name}"))
        .unwrap();
    registry
        .select(&identity.namespace, &identity.name, &identity.iid)
        .unwrap()
        .clone()
}

fn prepare(registry: &Registry, name: &str) -> BorrowedCopyPlan {
    let copy = record(registry, name);
    assert_eq!(
        copy.kind,
        CopyKind::MediaBuffer,
        "display label must not route execution"
    );
    assert!(
        BorrowedCopyPlan::prepare(&MetadataTable::new(), copy.clone()).is_err(),
        "Production admission must never authorize a test record"
    );
    BorrowedCopyPlan::prepare_test(&MetadataTable::new(), copy, registry).unwrap()
}

#[test]
fn borrowed_copy_new_records_execute_linear_frame_and_row_contracts() {
    super::super::super::initialize_apartment(super::super::super::ApartmentType::SingleThreaded)
        .unwrap();
    let registry = reordered::registry();
    let fixture = reordered::fixture();
    let media = fixture.object("media").unwrap();
    let context = Identity::for_object(&media).unwrap();
    let linear = prepare(&registry, "Linear");
    assert_eq!(linear.read_copy(&context, &media).unwrap(), [0, 1, 2, 3]);
    linear.replace_copy(&context, &media, &[9, 8, 7]).unwrap();
    assert_eq!(linear.read_copy(&context, &media).unwrap(), [9, 8, 7]);
    assert_eq!(count(&fixture, "media.unlock"), 3);
    assert_eq!(count(&fixture, "media.length.3"), 1);

    let client = initialized(&fixture, 0, 0);
    let (render, context) = service(&fixture, &client, AUDIO_RENDER).unwrap();
    let writer = prepare(&registry, "FrameWriter");
    let data = [9, 8, 7, 6, 5, 4, 3, 2];
    writer.write_frames_copy(&context, &render, &data).unwrap();
    assert_eq!(&fixture.bytes()[..8], &data);
    fixture.configure("renderNull", 1).unwrap();
    writer.write_silence(&context, &render, 2).unwrap();
    assert!(fixture.events().contains(&"render.release.2.0".into()));
    assert!(fixture.events().contains(&"render.release.2.2".into()));
    assert!(writer.write_frames_copy(&context, &render, &data).is_err());
    assert!(fixture.events().contains(&"render.release.0.0".into()));

    let (capture, context) = service(&fixture, &client, AUDIO_CAPTURE).unwrap();
    let reader = prepare(&registry, "FrameReader");
    let packet = reader
        .read_packet_copy(&context, &capture)
        .unwrap()
        .unwrap();
    assert_eq!(packet.data.unwrap(), data);
    assert_eq!(
        (
            packet.frames,
            packet.flags,
            packet.device_position,
            packet.qpc_position
        ),
        (2, 0, Some(123), Some(456))
    );
    fixture.configure("captureHr", BUFFER_EMPTY as u32).unwrap();
    assert!(
        reader
            .read_packet_copy(&context, &capture)
            .unwrap()
            .is_none()
    );
    assert_eq!(count(&fixture, "capture.release"), 1);
    fixture.configure("captureHr", 0).unwrap();
    fixture
        .configure("captureFlags", SILENT | TIMESTAMP_ERROR)
        .unwrap();
    fixture.configure("captureNull", 1).unwrap();
    let packet = reader
        .read_packet_copy(&context, &capture)
        .unwrap()
        .unwrap();
    assert!(
        packet.data.is_none() && packet.device_position.is_none() && packet.qpc_position.is_none()
    );
    assert_eq!(count(&fixture, "capture.release.2"), 2);

    let bitmap = fixture.object("bitmap").unwrap();
    let context = Identity::for_object(&bitmap).unwrap();
    let image = prepare(&registry, "Image");
    let copy = image
        .read_locked_bgra8_copy(&context, &bitmap, [0, 0, 2, 3])
        .unwrap();
    assert_eq!((copy.width, copy.height), (2, 3));
    assert_eq!(
        copy.data,
        fixture.bytes()[..8]
            .iter()
            .copied()
            .chain(12..20)
            .chain(24..32)
            .collect::<Vec<_>>()
    );
    assert_eq!(count(&fixture, "wic.release"), 1);
    fixture.configure("wicCount", 31).unwrap();
    assert!(
        image
            .read_locked_bgra8_copy(&context, &bitmap, [0, 0, 2, 3])
            .is_err()
    );
    assert_eq!(count(&fixture, "wic.release"), 2);
}

#[test]
fn borrowed_copy_reordered_cleanup_errors_poison_once() {
    let registry = reordered::registry();
    let fixture = reordered::fixture();
    let media = fixture.object("media").unwrap();
    let context = Identity::for_object(&media).unwrap();
    let linear = prepare(&registry, "Linear");
    fixture.configure("mediaSetHr", 0x80004005).unwrap();
    assert!(linear.replace_copy(&context, &media, &[4; 4]).is_err());
    assert_eq!(count(&fixture, "media.unlock"), 1);
    context.ensure_idle().unwrap();
    fixture.configure("mediaUnlockHr", 0x80004005).unwrap();
    assert!(linear.read_copy(&context, &media).is_err());
    assert!(linear.read_copy(&context, &media).is_err());
    assert_eq!(count(&fixture, "media.unlock"), 2);
    assert!(context.ensure_idle().is_err());
}

fn reject(name: &str, edit: impl FnOnce(&mut BorrowedCopyContract), reason: &str) {
    let fixture = reordered::fixture();
    let mut registry = reordered::registry();
    let mut copy = record(&registry, name);
    edit(&mut copy);
    *registry
        .copies
        .iter_mut()
        .find(|c| c.id == copy.id)
        .unwrap() = copy.clone();
    let Err(failure) = BorrowedCopyPlan::prepare_test(&MetadataTable::new(), copy, &registry)
    else {
        panic!("Invalid plan was prepared");
    };
    assert!(failure.message().contains(reason), "{}", failure.message());
    assert!(
        fixture.events().is_empty(),
        "No native entry is allowed during rejected admission"
    );
}

#[test]
fn borrowed_copy_test_registry_validation_rejects_invalid_compositions_before_native_entry() {
    reject(
        "Linear",
        |copy| {
            let acquire = copy
                .calls
                .iter_mut()
                .find(|c| c.id == "recipe-acquire")
                .unwrap();
            acquire.bindings[0] = Binding::OutU64 {
                name: "current".into(),
            };
        },
        "ABI cells",
    );
    reject(
        "Linear",
        |copy| {
            let acquire = copy
                .calls
                .iter_mut()
                .find(|c| c.id == "recipe-acquire")
                .unwrap();
            acquire.bindings[0] = Binding::OutU32 {
                name: "current".into(),
                unit: Unit::Frames,
            };
        },
        "Wrong output/reference type",
    );
    reject(
        "Linear",
        |copy| {
            let acquire = copy
                .calls
                .iter_mut()
                .find(|c| c.id == "recipe-acquire")
                .unwrap();
            acquire.bindings[2] = Binding::OutU32 {
                name: "current".into(),
                unit: Unit::Bytes,
            };
        },
        "Duplicate value/output",
    );
    reject(
        "FrameWriter",
        |copy| copy.operations[0].before.swap(2, 3),
        "Dangling, forward or uninitialized",
    );
    reject(
        "FrameReader",
        |copy| {
            let Cleanup::Call { abort, .. } = &mut copy.operations[0].cleanup else {
                panic!()
            };
            abort[1] = U32Source::Frames(contracts::FrameCountRef("frames".into()));
        },
        "Dangling, forward or uninitialized",
    );
    reject(
        "Linear",
        |copy| {
            let Step::NativeExtent { length, .. } = &mut copy.operations[0].after[0] else {
                panic!()
            };
            length.0 = "input.bytes".into();
        },
        "Dangling, forward or uninitialized",
    );
    reject(
        "Image",
        |copy| {
            copy.operations[0].before.insert(
                0,
                Step::Call {
                    call: "recipe-data".into(),
                },
            );
        },
        "Unavailable acquired owner",
    );
    reject(
        "Linear",
        |copy| {
            let replace = copy
                .operations
                .iter_mut()
                .find(|op| op.api == OperationId::ReplaceCopy)
                .unwrap();
            replace.commit.clear();
        },
        "requires its native length commit",
    );
    reject(
        "FrameReader",
        |copy| {
            let release = copy
                .calls
                .iter_mut()
                .find(|c| c.id == "recipe-release")
                .unwrap();
            release.bindings[1] = Binding::InU32 {
                value: U32Source::Constant(0),
            };
        },
        "Missing full/zero",
    );
    reject(
        "Image",
        |copy| copy.calls[0].evidence = "unknown-review-evidence".into(),
        "Unknown registry evidence",
    );
    reject(
        "FrameWriter",
        |copy| {
            copy.calls
                .iter_mut()
                .find(|call| call.id == "recipe-release")
                .unwrap()
                .bindings
                .swap(0, 1);
        },
        "Wrong output/reference type for native ABI cell",
    );
    reject(
        "Linear",
        |copy| {
            let Cleanup::Call { call: id, .. } = &copy.operations[0].cleanup else {
                panic!()
            };
            let release = copy.calls.iter_mut().find(|call| call.id == *id).unwrap();
            release.evidence = "test-IMFMediaBuffer.SetCurrentLength".into();
            release.bindings.push(Binding::InU32 {
                value: U32Source::Constant(0),
            });
            for op in &mut copy.operations {
                let Cleanup::Call { abort, .. } = &mut op.cleanup else {
                    panic!()
                };
                abort.push(U32Source::Constant(0));
            }
        },
        "Cleanup does not close the reviewed acquisition",
    );
}
