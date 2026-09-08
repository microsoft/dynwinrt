// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::testing::CopyFixture;
use super::*;

#[path = "com_borrowed_plan_tests.rs"]
mod planner;

fn plan(kind: CopyKind) -> BorrowedCopyPlan {
    BorrowedCopyPlan::prepare(&MetadataTable::new(), BorrowedCopyContract::expected(kind)).unwrap()
}

fn effect_method(effect: ContextEffect) -> MethodHandle {
    let table = MetadataTable::new();
    let signature = MethodSignature::new(&table);
    let (iid, slot, name, signature) = match effect {
        ContextEffect::AudioInitialize => (
            AUDIO_CLIENT,
            3,
            "Initialize",
            signature
                .add_in(Type::winrt(table.i32_type()))
                .add_in(Type::winrt(table.u32_type()))
                .add_in(Type::winrt(table.i64_type()))
                .add_in(Type::winrt(table.i64_type()))
                .add_in(Type::audio_format())
                .add_nullable_in(Type::pointer()),
        ),
        ContextEffect::AudioInitializeShared => (
            AUDIO_CLIENT3,
            20,
            "InitializeSharedAudioStream",
            signature
                .add_in(Type::winrt(table.u32_type()))
                .add_in(Type::winrt(table.u32_type()))
                .add_in(Type::audio_format())
                .add_nullable_in(Type::pointer()),
        ),
        ContextEffect::AudioGetService => (
            AUDIO_CLIENT,
            14,
            "GetService",
            signature
                .add_in(Type::pointer())
                .add_out(Type::owned_com_pointer()),
        ),
    };
    super::super::register_interface(&table, "context-test", iid, InterfaceBase::IUnknown)
        .add_method_at(slot, name, signature.with_context_effect(effect))
        .unwrap()
        .method(slot)
        .unwrap()
}

fn initialize_with(
    fixture: &CopyFixture,
    context: &Identity,
    format: AudioFormatValue,
    mode: i32,
    flags: u32,
    shared3: bool,
) -> result::Result<()> {
    let effect = if shared3 {
        ContextEffect::AudioInitializeShared
    } else {
        ContextEffect::AudioInitialize
    };
    let value = |v| Value::WinRt(v);
    let args = if shared3 {
        vec![
            value(WinRTValue::U32(flags)),
            value(WinRTValue::U32(2)),
            Value::AudioFormat(format),
            value(WinRTValue::RawPtr(ptr::null_mut())),
        ]
    } else {
        vec![
            value(WinRTValue::I32(mode)),
            value(WinRTValue::U32(flags)),
            value(WinRTValue::I64(100)),
            value(WinRTValue::I64(0)),
            Value::AudioFormat(format),
            value(WinRTValue::RawPtr(ptr::null_mut())),
        ]
    };
    unsafe {
        invoke_managed(
            &effect_method(effect),
            context,
            &fixture.object("client")?,
            &args,
        )
    }
    .map(|_| ())
}

fn initialized(fixture: &CopyFixture, mode: i32, flags: u32) -> Identity {
    let context = Identity::for_object(&fixture.object("client").unwrap()).unwrap();
    initialize_with(
        fixture,
        &context,
        AudioFormatValue::pcm(2, 48_000, 16).unwrap(),
        mode,
        flags,
        false,
    )
    .unwrap();
    context
}

fn service(
    fixture: &CopyFixture,
    client: &Identity,
    mut iid: GUID,
) -> result::Result<(IUnknown, Identity)> {
    let mut result = unsafe {
        invoke_managed(
            &effect_method(ContextEffect::AudioGetService),
            client,
            &fixture.object("client")?,
            &[Value::WinRt(cell(&mut iid))],
        )
    }?;
    let (Value::WinRt(WinRTValue::RawPtr(pointer)), super::super::PointerOutputKind::Com) =
        result.values.remove(0)
    else {
        panic!("service output");
    };
    Ok((
        unsafe { IUnknown::from_raw(pointer) },
        result.output_context.unwrap(),
    ))
}

fn count(fixture: &CopyFixture, event: &str) -> usize {
    fixture
        .events()
        .iter()
        .filter(|entry| entry.starts_with(event))
        .count()
}

#[test]
fn borrowed_copy_descriptor_drift_and_effect_signature_reject_before_dispatch() {
    for kind in [
        CopyKind::AudioRender,
        CopyKind::AudioCapture,
        CopyKind::BitmapBgra8,
        CopyKind::MediaBuffer,
    ] {
        let mut descriptor = BorrowedCopyContract::expected(kind);
        descriptor.calls[0].evidence.push_str(".slot-drift");
        assert!(BorrowedCopyPlan::prepare(&MetadataTable::new(), descriptor).is_err());
        let mut descriptor = BorrowedCopyContract::expected(kind);
        descriptor.calls[0].bindings.clear();
        assert!(BorrowedCopyPlan::prepare(&MetadataTable::new(), descriptor).is_err());
        let mut descriptor = BorrowedCopyContract::expected(kind);
        descriptor.audio_origin = if descriptor.audio_origin.is_some() {
            None
        } else {
            BorrowedCopyContract::expected(CopyKind::AudioRender).audio_origin
        };
        assert!(BorrowedCopyPlan::prepare(&MetadataTable::new(), descriptor).is_err());
    }
    let table = MetadataTable::new();
    let signature =
        MethodSignature::new(&table).with_context_effect(ContextEffect::AudioInitialize);
    assert!(
        super::super::register_interface(&table, "bad", AUDIO_CLIENT, InterfaceBase::IUnknown)
            .add_method_at(3, "Initialize", signature)
            .is_err()
    );
}

#[test]
fn borrowed_copy_audio_format_proof_uses_initialized_not_mix_layout() {
    let fixture = CopyFixture::default();
    let client = initialized(&fixture, 0, 0x80000000); // AUTOCONVERTPCM
    let table = MetadataTable::new();
    let mix =
        super::super::register_interface(&table, "mix", AUDIO_CLIENT, InterfaceBase::IUnknown)
            .add_method_at(
                8,
                "GetMixFormat",
                MethodSignature::new(&table).add_out(Type::audio_format()),
            )
            .unwrap();
    let result = unsafe {
        mix.method(8)
            .unwrap()
            .invoke_values_with_output_kinds(fixture.object("client").unwrap().as_raw(), &[])
    }
    .unwrap();
    let Value::AudioFormat(mix) = &result[0].0 else {
        panic!()
    };
    assert_eq!(mix.block_align(), 1);
    let (render, context) = service(&fixture, &client, AUDIO_RENDER).unwrap();
    plan(CopyKind::AudioRender)
        .write_frames_copy(&context, &render, &[9, 8, 7, 6, 5, 4, 3, 2])
        .unwrap();
    assert_eq!(&fixture.bytes()[..8], &[9, 8, 7, 6, 5, 4, 3, 2]);
    assert!(fixture.events().contains(&"render.acquire.2".into()));
    assert!(fixture.events().contains(&"render.release.2.0".into()));
}

#[test]
fn borrowed_copy_audio_requires_both_observed_successful_effects() {
    let fixture = CopyFixture::default();
    let render = fixture.object("render").unwrap();
    let render_context = Identity::for_object(&render).unwrap();
    assert!(
        plan(CopyKind::AudioRender)
            .write_frames_copy(&render_context, &render, &[0; 4])
            .is_err()
    );
    let client = Identity::for_object(&fixture.object("client").unwrap()).unwrap();
    fixture.configure("initializeHr", 0x80004005).unwrap();
    assert!(
        initialize_with(
            &fixture,
            &client,
            AudioFormatValue::pcm(2, 48_000, 16).unwrap(),
            0,
            0,
            false
        )
        .is_err()
    );
    assert!(client.inner.initialized.borrow().is_none());
    fixture.configure("externalInitialized", 1).unwrap();
    fixture.configure("initializeHr", 0).unwrap();
    assert!(
        initialize_with(
            &fixture,
            &client,
            AudioFormatValue::pcm(2, 48_000, 16).unwrap(),
            0,
            0,
            false
        )
        .is_err()
    );
    let (render, context) = service(&fixture, &client, AUDIO_RENDER).unwrap();
    assert!(
        plan(CopyKind::AudioRender)
            .write_frames_copy(&context, &render, &[0; 4])
            .is_err()
    );
    assert_eq!(count(&fixture, "render.acquire"), 0);
}

#[test]
fn borrowed_copy_audio_failed_reinitialization_cannot_overwrite_provenance() {
    let fixture = CopyFixture::default();
    let client = initialized(&fixture, 0, 0);
    assert!(
        initialize_with(
            &fixture,
            &client,
            AudioFormatValue::pcm(1, 44_100, 8).unwrap(),
            1,
            4,
            false
        )
        .is_err()
    );
    let stored = client.inner.initialized.borrow();
    let stored = stored.as_ref().unwrap();
    assert_eq!(stored.format.block_align(), 4);
    assert_eq!((stored.share_mode, stored.flags), (0, 0));
}

#[test]
fn borrowed_copy_audio_shared_initialization_and_unsupported_format() {
    let fixture = CopyFixture::default();
    let context = Identity::for_object(&fixture.object("client").unwrap()).unwrap();
    initialize_with(
        &fixture,
        &context,
        AudioFormatValue::pcm(2, 48_000, 16).unwrap(),
        9,
        4,
        true,
    )
    .unwrap();
    assert_eq!(
        context
            .inner
            .initialized
            .borrow()
            .as_ref()
            .unwrap()
            .share_mode,
        0
    );
    let (capture, origin) = service(&fixture, &context, AUDIO_CAPTURE).unwrap();
    assert_eq!(
        plan(CopyKind::AudioCapture)
            .read_packet_copy(&origin, &capture)
            .unwrap()
            .unwrap()
            .data
            .unwrap()
            .len(),
        8
    );
    let fixture = CopyFixture::default();
    let context = Identity::for_object(&fixture.object("client").unwrap()).unwrap();
    let compressed = AudioFormatValue::wave_format_ex(2, 2, 48_000, 100, 4, 4, vec![1; 6]).unwrap();
    initialize_with(&fixture, &context, compressed, 0, 0, false).unwrap();
    let (render, origin) = service(&fixture, &context, AUDIO_RENDER).unwrap();
    assert!(
        plan(CopyKind::AudioRender)
            .write_frames_copy(&origin, &render, &[0; 4])
            .is_err()
    );
    assert_eq!(count(&fixture, "render.acquire"), 0);
}

#[test]
fn borrowed_copy_identity_aliasing_and_context_lifetime_are_canonical() {
    let mut fixture = CopyFixture::default();
    let client = initialized(&fixture, 0, 0);
    let (render, context) = service(&fixture, &client, AUDIO_RENDER).unwrap();
    let unknown = query(&render, &IUnknown::IID).unwrap();
    assert_ne!(
        unknown.as_raw(),
        render.as_raw(),
        "fixture provides distinct canonical and service views"
    );
    let alias = Identity::for_object(&unknown).unwrap();
    assert!(context.matches(&alias));
    drop(context);
    drop(client);
    fixture.release_owners();
    assert_eq!(count(&fixture, "drop.client"), 0);
    plan(CopyKind::AudioRender)
        .write_frames_copy(&alias, &unknown, &[3; 8])
        .unwrap();
    drop(alias);
    assert_eq!(count(&fixture, "drop.client"), 1);
    drop(render);
    drop(unknown);
    assert_eq!(count(&fixture, "drop.render"), 1);
}

#[test]
fn borrowed_copy_rejects_wrong_service_or_forged_context() {
    let fixture = CopyFixture::default();
    let client = initialized(&fixture, 0, 0);
    fixture.configure("serviceWrong", 1).unwrap();
    assert!(service(&fixture, &client, AUDIO_RENDER).is_err());
    assert_eq!(count(&fixture, "render.acquire"), 0);
    fixture.configure("serviceWrong", 0).unwrap();
    let (render, context) = service(&fixture, &client, AUDIO_RENDER).unwrap();
    let other = CopyFixture::default().object("render").unwrap();
    assert!(
        plan(CopyKind::AudioRender)
            .write_frames_copy(&context, &other, &[0; 4])
            .is_err()
    );
    plan(CopyKind::AudioRender)
        .write_frames_copy(&context, &render, &[0; 4])
        .unwrap();
    let other_client = CopyFixture::default();
    let other_context = initialized(&other_client, 0, 0);
    other_client.share_render_service(&fixture);
    assert!(service(&other_client, &other_context, AUDIO_RENDER).is_err());
    // The rejected mapping cannot disturb the original service provenance.
    plan(CopyKind::AudioRender)
        .write_frames_copy(&context, &render, &[0; 4])
        .unwrap();
}

#[test]
fn borrowed_copy_render_zero_silence_bounds_and_abort() {
    let fixture = CopyFixture::default();
    let client = initialized(&fixture, 0, 0);
    let (render, context) = service(&fixture, &client, AUDIO_RENDER).unwrap();
    let plan = plan(CopyKind::AudioRender);
    plan.write_frames_copy(&context, &render, &[]).unwrap();
    plan.write_silence(&context, &render, 0).unwrap();
    assert_eq!(count(&fixture, "render.acquire"), 0);
    assert_eq!(count(&fixture, "audio.capacity"), 0);
    assert!(plan.write_frames_copy(&context, &render, &[0; 3]).is_err());
    assert!(plan.write_silence(&context, &render, 9).is_err());
    fixture.configure("renderNull", 1).unwrap();
    plan.write_silence(&context, &render, 2).unwrap();
    assert!(fixture.events().contains(&"render.release.2.2".into()));
    assert!(plan.write_frames_copy(&context, &render, &[0; 8]).is_err());
    assert!(fixture.events().contains(&"render.release.0.0".into()));
    context.ensure_idle().unwrap();
    let fixture = CopyFixture::default();
    let client = initialized(&fixture, 1, EVENT_CALLBACK);
    let (render, context) = service(&fixture, &client, AUDIO_RENDER).unwrap();
    assert!(plan.write_frames_copy(&context, &render, &[0; 4]).is_err());
    assert_eq!(count(&fixture, "render.acquire"), 0);
}

#[test]
fn borrowed_copy_capture_empty_silent_timestamps_and_exclusive_packets() {
    let fixture = CopyFixture::default();
    let client = initialized(&fixture, 1, 0);
    let (capture, context) = service(&fixture, &client, AUDIO_CAPTURE).unwrap();
    let plan = plan(CopyKind::AudioCapture);
    fixture.configure("captureHr", BUFFER_EMPTY as u32).unwrap();
    assert!(plan.read_packet_copy(&context, &capture).unwrap().is_none());
    assert_eq!(count(&fixture, "capture.release"), 0);
    fixture.configure("captureHr", 0).unwrap();
    fixture.configure("captureNull", 1).unwrap();
    fixture
        .configure("captureFlags", SILENT | TIMESTAMP_ERROR)
        .unwrap();
    let packet = plan.read_packet_copy(&context, &capture).unwrap().unwrap();
    assert!(packet.data.is_none());
    assert_eq!(packet.frames, 2);
    assert!(packet.device_position.is_none() && packet.qpc_position.is_none());
    fixture.configure("captureNull", 0).unwrap();
    fixture.configure("captureFlags", 1).unwrap();
    let packet = plan.read_packet_copy(&context, &capture).unwrap().unwrap();
    assert_eq!(packet.data.unwrap(), (0..8).collect::<Vec<_>>());
    assert_eq!(
        (packet.device_position, packet.qpc_position),
        (Some(123), Some(456))
    );
    assert_eq!(count(&fixture, "capture.next"), 0);
    assert_eq!(count(&fixture, "capture.release.2"), 2);
}

#[test]
fn borrowed_copy_capture_validation_zero_abort_and_unknown_success_poison() {
    let fixture = CopyFixture::default();
    let client = initialized(&fixture, 0, 0);
    let (capture, context) = service(&fixture, &client, AUDIO_CAPTURE).unwrap();
    let plan = plan(CopyKind::AudioCapture);
    fixture.configure("captureFrames", 9).unwrap();
    assert!(plan.read_packet_copy(&context, &capture).is_err());
    fixture.configure("captureFrames", 2).unwrap();
    fixture.configure("captureNull", 1).unwrap();
    assert!(plan.read_packet_copy(&context, &capture).is_err());
    assert_eq!(count(&fixture, "capture.release.0"), 2);
    fixture.configure("captureHr", 2).unwrap();
    assert!(plan.read_packet_copy(&context, &capture).is_err());
    assert!(context.ensure_idle().is_err());
    assert_eq!(count(&fixture, "capture.release"), 2);
}

#[test]
fn borrowed_copy_reentrancy_guards_precede_acquire_and_finalization() {
    let fixture = CopyFixture::default();
    let client = initialized(&fixture, 0, 0);
    let (render, context) = service(&fixture, &client, AUDIO_RENDER).unwrap();
    let rejected = Rc::new(Cell::new(0));
    let checked = rejected.clone();
    let alias = context.clone();
    let client_alias = client.clone();
    fixture.hook(Some(Rc::new(move |event| {
        if event.starts_with("render.") {
            if alias.ensure_idle().is_err()
                && client_alias.ensure_idle().is_err()
                && callbacks_suppressed()
            {
                checked.set(checked.get() + 1);
            }
        }
    })));
    plan(CopyKind::AudioRender)
        .write_frames_copy(&context, &render, &[0; 4])
        .unwrap();
    fixture.hook(None);
    assert_eq!(rejected.get(), 2);
    assert!(!callbacks_suppressed());
    context.ensure_idle().unwrap();
}

#[test]
fn borrowed_copy_cleanup_failures_are_exactly_once_and_poison_all_aliases() {
    for (kind, service_iid, setting, prefix) in [
        (
            CopyKind::AudioRender,
            Some(AUDIO_RENDER),
            "renderReleaseHr",
            "render.release",
        ),
        (
            CopyKind::AudioCapture,
            Some(AUDIO_CAPTURE),
            "captureReleaseHr",
            "capture.release",
        ),
        (CopyKind::MediaBuffer, None, "mediaUnlockHr", "media.unlock"),
    ] {
        let fixture = CopyFixture::default();
        let (object, context, client) = if let Some(iid) = service_iid {
            let client = initialized(&fixture, 0, 0);
            let (object, context) = service(&fixture, &client, iid).unwrap();
            (object, context, Some(client))
        } else {
            let object = fixture.object("media").unwrap();
            let context = Identity::for_object(&object).unwrap();
            (object, context, None)
        };
        fixture.configure(setting, 0x80004005).unwrap();
        let plan = plan(kind);
        let invoke = || match kind {
            CopyKind::AudioRender => plan.write_frames_copy(&context, &object, &[0; 4]),
            CopyKind::AudioCapture => plan.read_packet_copy(&context, &object).map(|_| ()),
            CopyKind::MediaBuffer => plan.read_copy(&context, &object).map(|_| ()),
            CopyKind::BitmapBgra8 => unreachable!(),
        };
        assert!(invoke().is_err());
        assert!(invoke().is_err());
        assert_eq!(count(&fixture, prefix), 1);
        if let Some(client) = client {
            assert!(client.ensure_idle().is_err());
        }
        drop(context);
        assert_eq!(count(&fixture, prefix), 1);
    }
}

#[test]
fn borrowed_copy_wic_short_final_row_stride_changes_and_cleanup() {
    super::super::initialize_apartment(super::super::ApartmentType::SingleThreaded).unwrap();
    let fixture = CopyFixture::default();
    let bitmap = fixture.object("bitmap").unwrap();
    let context = Identity::for_object(&bitmap).unwrap();
    let plan = plan(CopyKind::BitmapBgra8);
    let copy = plan
        .read_locked_bgra8_copy(&context, &bitmap, [0, 0, 2, 2])
        .unwrap();
    assert_eq!(copy.data, (0..8).chain(12..20).collect::<Vec<_>>());
    assert_eq!((copy.width, copy.height), (2, 2));
    fixture.configure("wicStride", 8).unwrap();
    fixture.configure("wicCount", 16).unwrap();
    assert_eq!(
        plan.read_locked_bgra8_copy(&context, &bitmap, [0, 0, 2, 2])
            .unwrap()
            .data,
        (0..16).collect::<Vec<_>>()
    );
    fixture.configure("wicCount", 15).unwrap();
    assert!(
        plan.read_locked_bgra8_copy(&context, &bitmap, [0, 0, 2, 2])
            .is_err()
    );
    fixture.configure("wicStride", 7).unwrap();
    assert!(
        plan.read_locked_bgra8_copy(&context, &bitmap, [0, 0, 2, 2])
            .is_err()
    );
    fixture.configure("wicWrongFormat", 1).unwrap();
    assert!(
        plan.read_locked_bgra8_copy(&context, &bitmap, [0, 0, 2, 2])
            .is_err()
    );
    assert_eq!(count(&fixture, "wic.release"), 5);
    assert!(
        !fixture
            .events()
            .iter()
            .any(|event| event.contains("unlock"))
    );
    assert!(
        plan.read_locked_bgra8_copy(&context, &bitmap, [0, 0, 3, 2])
            .is_err()
    );
    assert_eq!(count(&fixture, "wic.acquire"), 5);
}

#[test]
fn borrowed_copy_wic_failure_owned_output_and_mta_restriction() {
    std::thread::spawn(|| {
        super::super::initialize_apartment(super::super::ApartmentType::SingleThreaded).unwrap();
        let fixture = CopyFixture::default();
        let bitmap = fixture.object("bitmap").unwrap();
        let context = Identity::for_object(&bitmap).unwrap();
        fixture.configure("wicLockHr", 0x80004005).unwrap();
        assert!(
            plan(CopyKind::BitmapBgra8)
                .read_locked_bgra8_copy(&context, &bitmap, [0, 0, 2, 2])
                .is_err()
        );
        assert_eq!(count(&fixture, "wic.release"), 1);
        fixture.configure("wicLockHr", 0).unwrap();
        fixture.configure("wicQueryHr", 0x80004005).unwrap();
        assert!(
            plan(CopyKind::BitmapBgra8)
                .read_locked_bgra8_copy(&context, &bitmap, [0, 0, 2, 2])
                .is_err()
        );
        assert_eq!(count(&fixture, "wic.release"), 2);
        context.ensure_idle().unwrap();
    })
    .join()
    .unwrap();
    super::super::initialize_apartment(super::super::ApartmentType::MultiThreaded).unwrap();
    let fixture = CopyFixture::default();
    let bitmap = fixture.object("bitmap").unwrap();
    let context = Identity::for_object(&bitmap).unwrap();
    assert!(
        plan(CopyKind::BitmapBgra8)
            .read_locked_bgra8_copy(&context, &bitmap, [0, 0, 2, 2])
            .is_err()
    );
    assert_eq!(count(&fixture, "wic.acquire"), 0);
}

#[test]
fn borrowed_copy_media_native_lengths_owned_results_and_setlength_failures() {
    let fixture = CopyFixture::default();
    let media = fixture.object("media").unwrap();
    let context = Identity::for_object(&media).unwrap();
    let plan = plan(CopyKind::MediaBuffer);
    let original = plan.read_copy(&context, &media).unwrap();
    plan.replace_copy(&context, &media, &[8, 9, 10]).unwrap();
    assert_eq!(plan.read_copy(&context, &media).unwrap(), [8, 9, 10]);
    assert_eq!(original, [0, 1, 2, 3]);
    assert!(plan.replace_copy(&context, &media, &[0; 17]).is_err());
    fixture.configure("mediaCurrent", 17).unwrap();
    assert!(plan.read_copy(&context, &media).is_err());
    fixture.configure("mediaCurrent", 3).unwrap();
    fixture.configure("mediaSetHr", 0x80004005).unwrap();
    assert!(plan.replace_copy(&context, &media, &[4; 4]).is_err());
    assert_eq!(
        count(&fixture, "media.acquire"),
        count(&fixture, "media.unlock")
    );
    // Writes may already have changed bytes when SetCurrentLength fails.
    assert_eq!(&fixture.bytes()[..4], &[4; 4]);
    fixture.configure("mediaSetHr", 0).unwrap();
    fixture.configure("mediaCurrent", 0).unwrap();
    fixture.configure("mediaNull", 1).unwrap();
    assert!(plan.read_copy(&context, &media).unwrap().is_empty());
    fixture.configure("mediaLockHr", 0x80004005).unwrap();
    let releases = count(&fixture, "media.unlock");
    assert!(plan.read_copy(&context, &media).is_err());
    assert_eq!(count(&fixture, "media.unlock"), releases);
}

#[test]
fn borrowed_copy_wrong_thread_rejects_without_dispatch_or_release() {
    let fixture = CopyFixture::default();
    let object = fixture.object("media").unwrap();
    let context = Identity::for_object(&object).unwrap();
    let address = (&context as *const Identity).addr();
    // Test-only cross-thread probe calls the guard, which reads only ThreadId
    // before rejecting. No Rc operation or apartment reference is touched there.
    let rejected = std::thread::spawn(move || unsafe {
        (&*(address as *const Identity)).ensure_thread().is_err()
    })
    .join()
    .unwrap();
    assert!(rejected);
    assert!(fixture.events().is_empty());
    let key = context.inner._canonical.as_raw().addr();
    let rejected_claim = std::thread::spawn(move || identity_owner(key, true).is_err())
        .join()
        .unwrap();
    assert!(
        rejected_claim,
        "independent cross-thread rewrapping cannot create a second gate"
    );
    assert!(fixture.events().is_empty());
    plan(CopyKind::MediaBuffer)
        .read_copy(&context, &object)
        .unwrap();
}

#[test]
fn borrowed_copy_uncompressed_layout_and_checked_extents() {
    assert_eq!(
        uncompressed_block_align(&AudioFormatValue::pcm(2, 48_000, 16).unwrap()),
        Some(4)
    );
    let malformed = AudioFormatValue::wave_format_ex(1, 2, 48_000, 96_000, 2, 16, vec![]).unwrap();
    assert_eq!(uncompressed_block_align(&malformed), None);
    let mut extra = vec![16, 0, 3, 0, 0, 0];
    extra.extend_from_slice(&[1, 0, 0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]);
    let extensible =
        AudioFormatValue::wave_format_ex(0xfffe, 2, 48_000, 192_000, 4, 16, extra.clone()).unwrap();
    assert_eq!(uncompressed_block_align(&extensible), Some(4));
    extra[0] = 17;
    assert_eq!(
        uncompressed_block_align(
            &AudioFormatValue::wave_format_ex(0xfffe, 2, 48_000, 192_000, 4, 16, extra).unwrap()
        ),
        None
    );
    assert!(frame_bytes(u32::MAX, 4).is_err());
    assert!(frame_bytes(2, usize::MAX).is_err());
    assert!(validate_pointer(ptr::null(), 1).is_err());
    assert!(validate_pointer(ptr::null(), 0).is_ok());
}

#[test]
fn borrowed_copy_live_stock_windows_wic_and_linear_mf() -> result::Result<()> {
    use windows::Win32::{
        Graphics::Imaging::{
            CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICImagingFactory,
        },
        Media::MediaFoundation::MFCreateMemoryBuffer,
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
    };
    super::super::initialize_apartment(super::super::ApartmentType::SingleThreaded)?;
    // SDK adapters are confined to test acquisition. All buffer transactions
    // below execute the same completed MethodHandles as generated JavaScript.
    let factory: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
    let pixels = [7u8; 16];
    let bitmap =
        unsafe { factory.CreateBitmapFromMemory(2, 2, &GUID_WICPixelFormat32bppBGRA, 8, &pixels)? };
    let bitmap = bitmap.cast::<IUnknown>()?;
    let context = Identity::for_object(&bitmap)?;
    let copy =
        plan(CopyKind::BitmapBgra8).read_locked_bgra8_copy(&context, &bitmap, [0, 0, 2, 2])?;
    assert_eq!(copy.data, pixels);
    let media = unsafe { MFCreateMemoryBuffer(16)? }.cast::<IUnknown>()?;
    let context = Identity::for_object(&media)?;
    let media_plan = plan(CopyKind::MediaBuffer);
    media_plan.replace_copy(&context, &media, &[1, 2, 3, 4])?;
    assert_eq!(media_plan.read_copy(&context, &media)?, [1, 2, 3, 4]);
    Ok(())
}

#[test]
fn borrowed_copy_unknown_initialization_and_finalizer_successes_fail_closed() {
    let fixture = CopyFixture::default();
    let context = Identity::for_object(&fixture.object("client").unwrap()).unwrap();
    fixture.configure("initializeHr", 1).unwrap();
    assert!(
        initialize_with(
            &fixture,
            &context,
            AudioFormatValue::pcm(2, 48_000, 16).unwrap(),
            0,
            0,
            false,
        )
        .is_err()
    );
    assert!(context.inner.initialized.borrow().is_none());
    assert!(context.ensure_idle().is_err());

    let fixture = CopyFixture::default();
    let context = initialized(&fixture, 0, 0);
    let (render, service_context) = service(&fixture, &context, AUDIO_RENDER).unwrap();
    fixture.configure("renderReleaseHr", 1).unwrap();
    let plan = plan(CopyKind::AudioRender);
    assert!(
        plan.write_frames_copy(&service_context, &render, &[0; 4])
            .is_err()
    );
    assert!(
        plan.write_frames_copy(&service_context, &render, &[0; 4])
            .is_err()
    );
    assert_eq!(count(&fixture, "render.release"), 1);
    assert!(context.ensure_idle().is_err());
}
