// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Test-only reviewed records and ABI-correct alternative interfaces. Original
//! views remain truthful for observed Initialize/GetService provenance; all copy
//! calls use new IIDs, new slots, and reordered native bindings.

use super::super::contracts::{
    self, AbiCell, AcquisitionCleanup, Binding, Cleanup, CopyKind, Registry, Step, Target,
    U32Source, Unit,
};
use super::*;
use std::sync::LazyLock;

const CLIENT: GUID = GUID::from_u128(0xde860701_67f0_4c94_9665_514d8b6a9811);
const RENDER: GUID = GUID::from_u128(0xde860702_67f0_4c94_9665_514d8b6a9811);
const CAPTURE: GUID = GUID::from_u128(0xde860703_67f0_4c94_9665_514d8b6a9811);
const BITMAP: GUID = GUID::from_u128(0xde860704_67f0_4c94_9665_514d8b6a9811);
const LOCK: GUID = GUID::from_u128(0xde860705_67f0_4c94_9665_514d8b6a9811);
const MEDIA: GUID = GUID::from_u128(0xde860706_67f0_4c94_9665_514d8b6a9811);
const MARKER: u32 = 0xD07;

unsafe extern "system" fn reordered_qi(
    this: *mut c_void,
    iid: *const GUID,
    output: *mut *mut c_void,
) -> HRESULT {
    if iid.is_null() || output.is_null() {
        return POINTER;
    }
    let owner = unsafe { owner(this) };
    let extra = match owner.kind {
        Kind::Client => CLIENT,
        Kind::Render => RENDER,
        Kind::Capture => CAPTURE,
        Kind::Bitmap => BITMAP,
        Kind::Lock => LOCK,
        Kind::Media => MEDIA,
    };
    if unsafe { *iid } != extra {
        return unsafe { qi(this, iid, output) };
    }
    unsafe {
        *output = (&owner.views[1] as *const View).cast_mut().cast();
        add_ref(this);
    }
    OK
}

fn unknown() -> IUnknown_Vtbl {
    IUnknown_Vtbl {
        QueryInterface: reordered_qi,
        AddRef: add_ref,
        Release: release,
    }
}

#[repr(C)]
struct Client {
    base: ClientVtable,
    capacity: unsafe extern "system" fn(*mut c_void, u32, *mut u32) -> HRESULT,
}
unsafe extern "system" fn capacity(this: *mut c_void, marker: u32, output: *mut u32) -> HRESULT {
    if marker != MARKER {
        return INVALID;
    }
    unsafe { size(this, output) }
}
static CLIENT_TABLE: LazyLock<Client> = LazyLock::new(|| Client {
    base: ClientVtable {
        base: unknown(),
        ..CLIENT_VTABLE
    },
    capacity,
});

#[repr(C)]
struct Render {
    base: RenderVtable,
    acquire: unsafe extern "system" fn(*mut c_void, *mut *mut u8, u32) -> HRESULT,
    finish: unsafe extern "system" fn(*mut c_void, u32, u32) -> HRESULT,
}
unsafe extern "system" fn render_acquire(
    this: *mut c_void,
    data: *mut *mut u8,
    frames: u32,
) -> HRESULT {
    unsafe { render_get(this, frames, data) }
}
unsafe extern "system" fn render_finish(this: *mut c_void, flags: u32, frames: u32) -> HRESULT {
    unsafe { render_release(this, frames, flags) }
}
static RENDER_TABLE: LazyLock<Render> = LazyLock::new(|| Render {
    base: RenderVtable {
        base: unknown(),
        ..RENDER_VTABLE
    },
    acquire: render_acquire,
    finish: render_finish,
});

#[repr(C)]
struct Capture {
    base: CaptureVtable,
    acquire: unsafe extern "system" fn(
        *mut c_void,
        *mut u64,
        *mut u32,
        *mut *mut u8,
        *mut u64,
        *mut u32,
    ) -> HRESULT,
    finish: unsafe extern "system" fn(*mut c_void, u32, u32) -> HRESULT,
}
unsafe extern "system" fn packet_acquire(
    this: *mut c_void,
    qpc: *mut u64,
    flags: *mut u32,
    data: *mut *mut u8,
    device: *mut u64,
    frames: *mut u32,
) -> HRESULT {
    unsafe { capture_get(this, data, frames, flags, device, qpc) }
}
unsafe extern "system" fn packet_finish(this: *mut c_void, marker: u32, frames: u32) -> HRESULT {
    if marker != MARKER {
        return INVALID;
    }
    unsafe { capture_release(this, frames) }
}
static CAPTURE_TABLE: LazyLock<Capture> = LazyLock::new(|| Capture {
    base: CaptureVtable {
        base: unknown(),
        ..CAPTURE_VTABLE
    },
    acquire: packet_acquire,
    finish: packet_finish,
});

#[repr(C)]
struct Bitmap {
    base: BitmapVtable,
    acquire: unsafe extern "system" fn(*mut c_void, *mut *mut c_void, u32, *const i32) -> HRESULT,
    dimensions: unsafe extern "system" fn(*mut c_void, *mut u32, *mut u32) -> HRESULT,
}
unsafe extern "system" fn image_acquire(
    this: *mut c_void,
    output: *mut *mut c_void,
    flags: u32,
    rect: *const i32,
) -> HRESULT {
    if rect.is_null() || unsafe { std::slice::from_raw_parts(rect, 4) } != [0, 0, 2, 3] {
        return INVALID;
    }
    let fixture_rect = [0, 0, 2, 2];
    let hr = unsafe { bitmap_lock(this, fixture_rect.as_ptr(), flags, output) };
    if !output.is_null() && !unsafe { *output }.is_null() {
        unsafe { install(*output, (&*LOCK_TABLE as *const Lock).cast()) };
    }
    hr
}
unsafe extern "system" fn dimensions(
    this: *mut c_void,
    height: *mut u32,
    width: *mut u32,
) -> HRESULT {
    let hr = unsafe { bitmap_size(this, width, height) };
    if hr.0 == 0 {
        unsafe { *height = 3 };
    }
    hr
}
static BITMAP_TABLE: LazyLock<Bitmap> = LazyLock::new(|| Bitmap {
    base: BitmapVtable {
        base: unknown(),
        ..BITMAP_VTABLE
    },
    acquire: image_acquire,
    dimensions,
});

#[repr(C)]
struct Lock {
    base: LockVtable,
    dimensions: unsafe extern "system" fn(*mut c_void, *mut u32, *mut u32) -> HRESULT,
    stride: unsafe extern "system" fn(*mut c_void, u32, *mut u32) -> HRESULT,
    data: unsafe extern "system" fn(*mut c_void, *mut *mut u8, *mut u32) -> HRESULT,
    format: unsafe extern "system" fn(*mut c_void, *mut GUID) -> HRESULT,
}
unsafe extern "system" fn stride(this: *mut c_void, marker: u32, output: *mut u32) -> HRESULT {
    if marker != MARKER {
        return INVALID;
    }
    unsafe { lock_stride(this, output) }
}
unsafe extern "system" fn image_data(
    this: *mut c_void,
    data: *mut *mut u8,
    count: *mut u32,
) -> HRESULT {
    unsafe { lock_data(this, count, data) }
}
static LOCK_TABLE: LazyLock<Lock> = LazyLock::new(|| Lock {
    base: LockVtable {
        base: unknown(),
        ..LOCK_VTABLE
    },
    dimensions,
    stride,
    data: image_data,
    format: bitmap_format,
});

#[repr(C)]
struct Linear {
    base: MediaVtable,
    acquire: unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut u8, *mut u32) -> HRESULT,
    set_length: unsafe extern "system" fn(*mut c_void, u32, u32) -> HRESULT,
    finish: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
}
unsafe extern "system" fn linear_acquire(
    this: *mut c_void,
    current: *mut u32,
    data: *mut *mut u8,
    max: *mut u32,
) -> HRESULT {
    unsafe { media_lock(this, data, max, current) }
}
unsafe extern "system" fn set_length(this: *mut c_void, marker: u32, length: u32) -> HRESULT {
    if marker != MARKER {
        return INVALID;
    }
    unsafe { media_set(this, length) }
}
unsafe extern "system" fn linear_finish(this: *mut c_void, marker: u32) -> HRESULT {
    if marker != MARKER {
        return INVALID;
    }
    unsafe { media_unlock(this) }
}
static LINEAR_TABLE: LazyLock<Linear> = LazyLock::new(|| Linear {
    base: MediaVtable {
        base: unknown(),
        ..MEDIA_VTABLE
    },
    acquire: linear_acquire,
    set_length,
    finish: linear_finish,
});

unsafe fn install(this: *mut c_void, vtable: *const c_void) {
    let owner = unsafe { &mut *(*(this.cast::<View>())).owner };
    for view in &mut owner.views {
        view.vtable = vtable;
    }
}

pub fn fixture() -> CopyFixture {
    let fixture = CopyFixture::default();
    fixture.configure("wicCount", 32).unwrap();
    for (name, table) in [
        ("client", (&*CLIENT_TABLE as *const Client).cast()),
        ("render", (&*RENDER_TABLE as *const Render).cast()),
        ("capture", (&*CAPTURE_TABLE as *const Capture).cast()),
        ("bitmap", (&*BITMAP_TABLE as *const Bitmap).cast()),
        ("media", (&*LINEAR_TABLE as *const Linear).cast()),
    ] {
        unsafe { install(fixture.object(name).unwrap().as_raw(), table) };
    }
    fixture
}

pub fn registry() -> Registry {
    let mut registry = contracts::registry().clone();
    let mut identities = std::collections::HashMap::new();
    for (original, name, iid, last) in [
        ("IAudioClient", "ObservedClient", CLIENT, 21),
        ("IAudioRenderClient", "FrameWriter", RENDER, 6),
        ("IAudioCaptureClient", "FrameReader", CAPTURE, 7),
        ("IWICBitmap", "Image", BITMAP, 12),
        ("IWICBitmapLock", "ImageOwner", LOCK, 10),
        ("IMFMediaBuffer", "Linear", MEDIA, 10),
    ] {
        let mut record = registry
            .interfaces
            .iter()
            .find(|r| r.name == original)
            .unwrap()
            .clone();
        let old = record.identity();
        record.namespace = "Tests.Borrowed".into();
        record.name = name.into();
        record.iid = format!("{iid:?}").to_lowercase();
        record.bases = vec!["IUnknown".into()];
        record.base_iids.clear();
        record.own_start = 3;
        record.last = last;
        identities.insert(old, (record.identity(), record.iid.clone()));
        registry.interfaces.push(record);
    }
    for mut copy in contracts::registry().copies.clone() {
        copy.id = format!("test-{}", copy.id);
        copy.receiver = identities[&copy.receiver].0.clone();
        for dependency in &mut copy.dependencies {
            *dependency = identities[dependency].0.clone();
        }
        if let Some(origin) = &mut copy.audio_origin {
            origin.interface = identities[&origin.interface].0.clone();
        }
        // Deliberately misleading display labels cannot select an algorithm.
        copy.kind = CopyKind::MediaBuffer;
        let mut finalizer_orders = std::collections::HashMap::new();
        for call in &mut copy.calls {
            let old = registry.evidence(&call.evidence).unwrap();
            let (slot, order, marker): (usize, &[usize], bool) = match call.evidence.as_str() {
                "IAudioClient.GetBufferSize" => (21, &[0], true),
                "IAudioRenderClient.GetBuffer" => (5, &[1, 0], false),
                "IAudioRenderClient.ReleaseBuffer" => (6, &[1, 0], false),
                "IAudioCaptureClient.GetBuffer" => (6, &[4, 2, 0, 3, 1], false),
                "IAudioCaptureClient.ReleaseBuffer" => (7, &[0], true),
                "IWICBitmapSource.GetSize" => (12, &[1, 0], false),
                "IWICBitmap.Lock" => (11, &[2, 1, 0], false),
                "IWICBitmapLock.GetSize" => (7, &[1, 0], false),
                "IWICBitmapLock.GetStride" => (8, &[0], true),
                "IWICBitmapLock.GetDataPointer" => (9, &[1, 0], false),
                "IWICBitmapLock.GetPixelFormat" => (10, &[0], false),
                "IMFMediaBuffer.Lock" => (8, &[2, 0, 1], false),
                "IMFMediaBuffer.SetCurrentLength" => (9, &[0], true),
                "IMFMediaBuffer.Unlock" => (10, &[], true),
                _ => panic!("No test ABI"),
            };
            let mut evidence = old.clone();
            let target = match &call.target {
                Target::Receiver => &copy.receiver,
                Target::AudioOrigin => &copy.audio_origin.as_ref().unwrap().interface,
                Target::Acquired(_) => &copy.dependencies[0],
            };
            let target = registry.interface(target).unwrap();
            evidence.id = format!("test-{}", evidence.id);
            evidence.namespace = target.namespace.clone();
            evidence.interface = target.name.clone();
            evidence.iid = target.iid.clone();
            evidence.slot = slot;
            evidence.fingerprint = "A".repeat(64);
            evidence.citation = "test-only ABI-correct reordered native vtable".into();
            if let Some(AcquisitionCleanup::Call { evidence }) = &mut evidence.acquisition_cleanup {
                *evidence = format!("test-{evidence}");
            }
            call.evidence = evidence.id.clone();
            let old_abi = old.abi.as_ref().unwrap();
            let mut abi: Vec<_> = order.iter().map(|i| old_abi[*i].clone()).collect();
            call.bindings = order.iter().map(|i| call.bindings[*i].clone()).collect();
            if marker {
                abi.insert(0, AbiCell::InU32(Unit::Scalar));
                call.bindings.insert(
                    0,
                    Binding::InU32 {
                        value: U32Source::Constant(MARKER),
                    },
                );
            }
            for binding in &mut call.bindings {
                if let Binding::OwnedInterface { interface, .. } = binding {
                    *interface = identities[interface].0.clone();
                }
            }
            evidence.abi = Some(abi);
            if !registry.evidence.iter().any(|e| e.id == evidence.id) {
                registry.evidence.push(evidence);
            }
            finalizer_orders.insert(call.id.clone(), (order.to_vec(), marker));
            call.id = format!("recipe-{}", call.id);
        }
        for op in &mut copy.operations {
            op.acquire = format!("recipe-{}", op.acquire);
            for step in op.before.iter_mut().chain(&mut op.after) {
                if let Step::Call { call } = step {
                    *call = format!("recipe-{call}");
                }
            }
            for call in &mut op.commit {
                *call = format!("recipe-{call}");
            }
            if let Cleanup::Call { call, abort } = &mut op.cleanup {
                let (order, marker) = &finalizer_orders[call];
                *abort = order.iter().map(|i| abort[*i].clone()).collect();
                if *marker {
                    abort.insert(0, U32Source::Constant(MARKER));
                }
                *call = format!("recipe-{call}");
            }
        }
        copy.calls.reverse();
        registry.copies.push(copy);
    }
    registry
}

#[test]
fn borrowed_copy_reordered_fake_slots_are_abi_correct_on_both_widths() {
    use std::mem::{offset_of, size_of};
    let p = size_of::<usize>();
    assert_eq!(offset_of!(Client, capacity), 21 * p);
    assert_eq!(offset_of!(Render, acquire), 5 * p);
    assert_eq!(offset_of!(Render, finish), 6 * p);
    assert_eq!(offset_of!(Capture, acquire), 6 * p);
    assert_eq!(offset_of!(Capture, finish), 7 * p);
    assert_eq!(offset_of!(Bitmap, acquire), 11 * p);
    assert_eq!(offset_of!(Bitmap, dimensions), 12 * p);
    assert_eq!(offset_of!(Lock, dimensions), 7 * p);
    assert_eq!(offset_of!(Lock, stride), 8 * p);
    assert_eq!(offset_of!(Lock, data), 9 * p);
    assert_eq!(offset_of!(Lock, format), 10 * p);
    assert_eq!(offset_of!(Linear, acquire), 8 * p);
    assert_eq!(offset_of!(Linear, set_length), 9 * p);
    assert_eq!(offset_of!(Linear, finish), 10 * p);
}
