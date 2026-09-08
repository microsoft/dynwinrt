// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Deterministic, hardware-free native fixtures. Both interface views have
//! complete ABI-correct vtables, one refcount and truthful canonical IUnknown QI.

use super::{
    AUDIO_CAPTURE, AUDIO_CLIENT, AUDIO_CLIENT2, AUDIO_CLIENT3, AUDIO_RENDER, AudioFormatValue,
    MF_BUFFER, WIC_BITMAP, WIC_LOCK, error, result,
};
use std::{
    cell::{Cell, RefCell, UnsafeCell},
    ffi::c_void,
    ptr,
    rc::Rc,
};
use windows_core::{GUID, HRESULT, IUnknown, IUnknown_Vtbl, Interface as _};

#[cfg(test)]
#[path = "com_borrowed_reordered_support.rs"]
pub(super) mod reordered;

const FAIL: HRESULT = HRESULT(0x80004005u32 as i32);
const INVALID: HRESULT = HRESULT(0x80070057u32 as i32);
const POINTER: HRESULT = HRESULT(0x80004003u32 as i32);
const NOTIMPL: HRESULT = HRESULT(0x80004001u32 as i32);
const OK: HRESULT = HRESULT(0);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Client,
    Render,
    Capture,
    Bitmap,
    Lock,
    Media,
}

#[repr(C)]
struct View {
    vtable: *const c_void,
    owner: *mut Owner,
}

struct Owner {
    views: [View; 2],
    references: Cell<u32>,
    kind: Kind,
    control: Rc<Control>,
    held: RefCell<Vec<IUnknown>>,
}

struct Control {
    initialize_hr: Cell<i32>,
    initialized: Cell<bool>,
    format: RefCell<Option<AudioFormatValue>>,
    capacity: Cell<u32>,
    render_active: Cell<bool>,
    render_requested: Cell<u32>,
    render_release_hr: Cell<i32>,
    render_null: Cell<bool>,
    render_acquire_hr: Cell<i32>,
    capture_active: Cell<bool>,
    capture_hr: Cell<i32>,
    capture_frames: Cell<u32>,
    capture_flags: Cell<u32>,
    capture_null: Cell<bool>,
    capture_release_hr: Cell<i32>,
    service_wrong: Cell<bool>,
    media_active: Cell<bool>,
    media_max: Cell<u32>,
    media_current: Cell<u32>,
    media_lock_hr: Cell<i32>,
    media_unlock_hr: Cell<i32>,
    media_set_hr: Cell<i32>,
    media_null: Cell<bool>,
    wic_active: Cell<bool>,
    wic_stride: Cell<u32>,
    wic_count: Cell<u32>,
    wic_lock_hr: Cell<i32>,
    wic_null: Cell<bool>,
    wic_wrong_format: Cell<bool>,
    wic_query_hr: Cell<i32>,
    bytes: UnsafeCell<[u8; 256]>,
    events: RefCell<Vec<String>>,
    hook: RefCell<Option<Rc<dyn Fn(&str)>>>,
}

impl Default for Control {
    fn default() -> Self {
        Self {
            initialize_hr: Cell::new(0),
            initialized: Cell::new(false),
            format: RefCell::new(None),
            capacity: Cell::new(8),
            render_active: Cell::new(false),
            render_requested: Cell::new(0),
            render_release_hr: Cell::new(0),
            render_null: Cell::new(false),
            render_acquire_hr: Cell::new(0),
            capture_active: Cell::new(false),
            capture_hr: Cell::new(0),
            capture_frames: Cell::new(2),
            capture_flags: Cell::new(0),
            capture_null: Cell::new(false),
            capture_release_hr: Cell::new(0),
            service_wrong: Cell::new(false),
            media_active: Cell::new(false),
            media_max: Cell::new(16),
            media_current: Cell::new(4),
            media_lock_hr: Cell::new(0),
            media_unlock_hr: Cell::new(0),
            media_set_hr: Cell::new(0),
            media_null: Cell::new(false),
            wic_active: Cell::new(false),
            wic_stride: Cell::new(12),
            wic_count: Cell::new(20),
            wic_lock_hr: Cell::new(0),
            wic_null: Cell::new(false),
            wic_wrong_format: Cell::new(false),
            wic_query_hr: Cell::new(0),
            bytes: UnsafeCell::new(std::array::from_fn(|i| i as u8)),
            events: RefCell::new(vec![]),
            hook: RefCell::new(None),
        }
    }
}

impl Control {
    fn event(&self, event: impl Into<String>) {
        let event = event.into();
        self.events.borrow_mut().push(event.clone());
        let hook = self.hook.borrow().clone();
        if let Some(hook) = hook {
            hook(&event);
        }
    }
    fn block(&self) -> usize {
        self.format
            .borrow()
            .as_ref()
            .map_or(4, |format| format.block_align() as usize)
    }
    fn bytes(&self) -> *mut u8 {
        self.bytes.get().cast()
    }
}

unsafe fn owner<'a>(this: *mut c_void) -> &'a Owner {
    unsafe { &*(*(this.cast::<View>())).owner }
}

unsafe extern "system" fn qi(
    this: *mut c_void,
    iid: *const GUID,
    output: *mut *mut c_void,
) -> HRESULT {
    if iid.is_null() || output.is_null() {
        return POINTER;
    }
    unsafe {
        *output = ptr::null_mut();
        let owner = owner(this);
        let supported = *iid == IUnknown::IID
            || match owner.kind {
                Kind::Client => {
                    *iid == AUDIO_CLIENT || *iid == AUDIO_CLIENT2 || *iid == AUDIO_CLIENT3
                }
                Kind::Render => *iid == AUDIO_RENDER,
                Kind::Capture => *iid == AUDIO_CAPTURE,
                Kind::Bitmap => {
                    *iid == WIC_BITMAP
                        || *iid == GUID::from_u128(0x00000120_a8f2_4877_ba0a_fd2b6645fb94)
                }
                Kind::Lock => *iid == WIC_LOCK,
                Kind::Media => *iid == MF_BUFFER,
            };
        if !supported {
            return HRESULT(0x80004002u32 as i32);
        }
        let index = usize::from(*iid != IUnknown::IID);
        *output = (&owner.views[index] as *const View).cast_mut().cast();
        add_ref(this);
        OK
    }
}

unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    let owner = unsafe { owner(this) };
    let count = owner.references.get() + 1;
    owner.references.set(count);
    count
}

unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    unsafe {
        let owner = owner(this);
        let count = owner.references.get() - 1;
        owner.references.set(count);
        if count == 0 {
            if owner.kind == Kind::Lock {
                owner.control.wic_active.set(false);
                owner.control.event("wic.release");
            }
            owner.control.event(format!(
                "drop.{}",
                match owner.kind {
                    Kind::Client => "client",
                    Kind::Render => "render",
                    Kind::Capture => "capture",
                    Kind::Bitmap => "bitmap",
                    Kind::Lock => "lock",
                    Kind::Media => "media",
                }
            ));
            drop(Box::from_raw((owner as *const Owner).cast_mut()));
        }
        count
    }
}

const UNKNOWN: IUnknown_Vtbl = IUnknown_Vtbl {
    QueryInterface: qi,
    AddRef: add_ref,
    Release: release,
};

fn make(kind: Kind, control: &Rc<Control>, held: Vec<IUnknown>) -> IUnknown {
    let vtable = match kind {
        Kind::Client => (&CLIENT_VTABLE as *const ClientVtable).cast(),
        Kind::Render => (&RENDER_VTABLE as *const RenderVtable).cast(),
        Kind::Capture => (&CAPTURE_VTABLE as *const CaptureVtable).cast(),
        Kind::Bitmap => (&BITMAP_VTABLE as *const BitmapVtable).cast(),
        Kind::Lock => (&LOCK_VTABLE as *const LockVtable).cast(),
        Kind::Media => (&MEDIA_VTABLE as *const MediaVtable).cast(),
    };
    let mut object = Box::new(Owner {
        views: [
            View {
                vtable,
                owner: ptr::null_mut(),
            },
            View {
                vtable,
                owner: ptr::null_mut(),
            },
        ],
        references: Cell::new(1),
        kind,
        control: control.clone(),
        held: RefCell::new(held),
    });
    let pointer = &mut *object as *mut Owner;
    object.views[0].owner = pointer;
    object.views[1].owner = pointer;
    let output = (&mut object.views[1] as *mut View).cast();
    let _ = Box::into_raw(object);
    unsafe { IUnknown::from_raw(output) }
}

#[repr(C)]
struct ClientVtable {
    base: IUnknown_Vtbl,
    initialize: unsafe extern "system" fn(
        *mut c_void,
        i32,
        u32,
        i64,
        i64,
        *const u8,
        *const GUID,
    ) -> HRESULT,
    size: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    latency: unsafe extern "system" fn(*mut c_void, *mut i64) -> HRESULT,
    padding: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    format_supported:
        unsafe extern "system" fn(*mut c_void, i32, *const u8, *mut *mut u8) -> HRESULT,
    mix_format: unsafe extern "system" fn(*mut c_void, *mut *mut u8) -> HRESULT,
    periods: unsafe extern "system" fn(*mut c_void, *mut i64, *mut i64) -> HRESULT,
    start: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    stop: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    reset: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    set_event: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    service: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    offload: unsafe extern "system" fn(*mut c_void, i32, *mut i32) -> HRESULT,
    properties: unsafe extern "system" fn(*mut c_void, *const c_void) -> HRESULT,
    limits: unsafe extern "system" fn(*mut c_void, *const u8, i32, *mut i64, *mut i64) -> HRESULT,
    engine_periods: unsafe extern "system" fn(
        *mut c_void,
        *const u8,
        *mut u32,
        *mut u32,
        *mut u32,
        *mut u32,
    ) -> HRESULT,
    current_period: unsafe extern "system" fn(*mut c_void, *mut *mut u8, *mut u32) -> HRESULT,
    initialize_shared:
        unsafe extern "system" fn(*mut c_void, u32, u32, *const u8, *const GUID) -> HRESULT,
}

unsafe fn initialize_native(this: *mut c_void, format: *const u8) -> HRESULT {
    if format.is_null() {
        return POINTER;
    }
    let control = &unsafe { owner(this) }.control;
    control.event("audio.initialize");
    let hr = control.initialize_hr.get();
    if hr != 0 {
        return HRESULT(hr);
    }
    if control.initialized.get() {
        return HRESULT(0x88890002u32 as i32);
    }
    let extra = unsafe { u16::from_le_bytes([*format.add(16), *format.add(17)]) } as usize;
    let bytes = unsafe { std::slice::from_raw_parts(format, 18 + extra) }.to_vec();
    let Ok(format) = AudioFormatValue::from_wave_format_ex(bytes) else {
        return INVALID;
    };
    // This fixture never returns storage larger than its physical array.
    if usize::from(format.block_align())
        .checked_mul(control.capacity.get() as usize)
        .is_none_or(|size| size > 256)
    {
        return INVALID;
    }
    *control.format.borrow_mut() = Some(format);
    control.initialized.set(true);
    OK
}

unsafe extern "system" fn initialize(
    this: *mut c_void,
    _: i32,
    _: u32,
    _: i64,
    _: i64,
    format: *const u8,
    _: *const GUID,
) -> HRESULT {
    unsafe { initialize_native(this, format) }
}
unsafe extern "system" fn initialize_shared(
    this: *mut c_void,
    _: u32,
    _: u32,
    format: *const u8,
    _: *const GUID,
) -> HRESULT {
    unsafe { initialize_native(this, format) }
}
unsafe extern "system" fn size(this: *mut c_void, output: *mut u32) -> HRESULT {
    if output.is_null() {
        return POINTER;
    }
    let control = &unsafe { owner(this) }.control;
    control.event("audio.capacity");
    unsafe {
        *output = control.capacity.get();
    }
    OK
}
unsafe extern "system" fn u32_zero(_: *mut c_void, output: *mut u32) -> HRESULT {
    if output.is_null() {
        return POINTER;
    }
    unsafe {
        *output = 0;
    }
    OK
}
unsafe extern "system" fn latency(_: *mut c_void, output: *mut i64) -> HRESULT {
    if output.is_null() {
        return POINTER;
    }
    unsafe {
        *output = 0;
    }
    OK
}
unsafe extern "system" fn format_supported(
    _: *mut c_void,
    _: i32,
    _: *const u8,
    output: *mut *mut u8,
) -> HRESULT {
    if !output.is_null() {
        unsafe {
            *output = ptr::null_mut();
        }
    }
    OK
}
unsafe extern "system" fn mix_format(this: *mut c_void, output: *mut *mut u8) -> HRESULT {
    if output.is_null() {
        return POINTER;
    }
    let control = &unsafe { owner(this) }.control;
    control.event("audio.mix");
    let format = AudioFormatValue::pcm(1, 48_000, 8).unwrap();
    let pointer =
        unsafe { windows::Win32::System::Com::CoTaskMemAlloc(format.bytes().len()) }.cast::<u8>();
    if pointer.is_null() {
        return HRESULT(0x8007000eu32 as i32);
    }
    unsafe {
        ptr::copy_nonoverlapping(format.bytes().as_ptr(), pointer, format.bytes().len());
        *output = pointer;
    }
    OK
}
unsafe extern "system" fn periods(_: *mut c_void, a: *mut i64, b: *mut i64) -> HRESULT {
    unsafe {
        if !a.is_null() {
            *a = 0;
        }
        if !b.is_null() {
            *b = 0;
        }
    }
    OK
}
unsafe extern "system" fn simple(_: *mut c_void) -> HRESULT {
    OK
}
unsafe extern "system" fn set_event(_: *mut c_void, _: *mut c_void) -> HRESULT {
    OK
}
unsafe extern "system" fn service(
    this: *mut c_void,
    iid: *const GUID,
    output: *mut *mut c_void,
) -> HRESULT {
    if iid.is_null() || output.is_null() {
        return POINTER;
    }
    unsafe {
        *output = ptr::null_mut();
        let owner = owner(this);
        owner.control.event("audio.service");
        if owner.control.service_wrong.get() {
            add_ref(this);
            *output = this;
            return OK;
        }
        let target = {
            let held = owner.held.borrow();
            if *iid == AUDIO_RENDER {
                held.first().cloned()
            } else if *iid == AUDIO_CAPTURE {
                held.get(1).cloned()
            } else {
                None
            }
        };
        if let Some(target) = target {
            target.query(iid, output)
        } else {
            qi(this, iid, output)
        }
    }
}
unsafe extern "system" fn offload(_: *mut c_void, _: i32, out: *mut i32) -> HRESULT {
    if out.is_null() {
        return POINTER;
    }
    unsafe {
        *out = 0;
    }
    OK
}
unsafe extern "system" fn properties(_: *mut c_void, _: *const c_void) -> HRESULT {
    OK
}
unsafe extern "system" fn limits(
    this: *mut c_void,
    _: *const u8,
    _: i32,
    a: *mut i64,
    b: *mut i64,
) -> HRESULT {
    unsafe { periods(this, a, b) }
}
unsafe extern "system" fn engine_periods(
    _: *mut c_void,
    _: *const u8,
    a: *mut u32,
    b: *mut u32,
    c: *mut u32,
    d: *mut u32,
) -> HRESULT {
    unsafe {
        for p in [a, b, c, d] {
            if p.is_null() {
                return POINTER;
            }
            *p = 1;
        }
    }
    OK
}
unsafe extern "system" fn current_period(
    this: *mut c_void,
    format: *mut *mut u8,
    period: *mut u32,
) -> HRESULT {
    if period.is_null() {
        return POINTER;
    }
    unsafe {
        *period = 1;
        mix_format(this, format)
    }
}
const CLIENT_VTABLE: ClientVtable = ClientVtable {
    base: UNKNOWN,
    initialize,
    size,
    latency,
    padding: u32_zero,
    format_supported,
    mix_format,
    periods,
    start: simple,
    stop: simple,
    reset: simple,
    set_event,
    service,
    offload,
    properties,
    limits,
    engine_periods,
    current_period,
    initialize_shared,
};

#[repr(C)]
struct RenderVtable {
    base: IUnknown_Vtbl,
    get: unsafe extern "system" fn(*mut c_void, u32, *mut *mut u8) -> HRESULT,
    release: unsafe extern "system" fn(*mut c_void, u32, u32) -> HRESULT,
}
unsafe extern "system" fn render_get(
    this: *mut c_void,
    frames: u32,
    output: *mut *mut u8,
) -> HRESULT {
    if output.is_null() {
        return POINTER;
    }
    unsafe {
        *output = ptr::null_mut();
    }
    let control = &unsafe { owner(this) }.control;
    control.event(format!("render.acquire.{frames}"));
    if frames == 0
        || frames > control.capacity.get()
        || (frames as usize)
            .checked_mul(control.block())
            .is_none_or(|n| n > 256)
    {
        return INVALID;
    }
    if control.render_active.get() {
        return FAIL;
    }
    if control.render_acquire_hr.get() != 0 {
        return HRESULT(control.render_acquire_hr.get());
    }
    control.render_active.set(true);
    control.render_requested.set(frames);
    if !control.render_null.get() {
        unsafe {
            *output = control.bytes();
        }
    }
    OK
}
unsafe extern "system" fn render_release(this: *mut c_void, frames: u32, flags: u32) -> HRESULT {
    let control = &unsafe { owner(this) }.control;
    control.event(format!("render.release.{frames}.{flags}"));
    if !control.render_active.get()
        || (frames != 0 && frames != control.render_requested.get())
        || !matches!(flags, 0 | 2)
        || (frames == 0 && flags != 0)
    {
        return INVALID;
    }
    if control.render_release_hr.get() != 0 {
        return HRESULT(control.render_release_hr.get());
    }
    control.render_active.set(false);
    OK
}
const RENDER_VTABLE: RenderVtable = RenderVtable {
    base: UNKNOWN,
    get: render_get,
    release: render_release,
};

#[repr(C)]
struct CaptureVtable {
    base: IUnknown_Vtbl,
    get: unsafe extern "system" fn(
        *mut c_void,
        *mut *mut u8,
        *mut u32,
        *mut u32,
        *mut u64,
        *mut u64,
    ) -> HRESULT,
    release: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    next: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
}
unsafe extern "system" fn capture_get(
    this: *mut c_void,
    data: *mut *mut u8,
    frames: *mut u32,
    flags: *mut u32,
    device: *mut u64,
    qpc: *mut u64,
) -> HRESULT {
    if data.is_null() || frames.is_null() || flags.is_null() {
        return POINTER;
    }
    let control = &unsafe { owner(this) }.control;
    control.event("capture.acquire");
    let hr = control.capture_hr.get();
    if hr != 0 {
        // Deliberately meaningless output cells on BUFFER_EMPTY. The caller must
        // classify HRESULT without reading them, and must not release a packet.
        unsafe {
            *data = ptr::null_mut();
            *frames = u32::MAX;
            *flags = u32::MAX;
        }
        return HRESULT(hr);
    }
    if control.capture_active.get() {
        return FAIL;
    }
    if (control.capture_frames.get() as usize)
        .checked_mul(control.block())
        .is_none_or(|n| n > 256)
    {
        return INVALID;
    }
    control.capture_active.set(true);
    unsafe {
        *data = if control.capture_null.get() {
            ptr::null_mut()
        } else {
            control.bytes()
        };
        *frames = control.capture_frames.get();
        *flags = control.capture_flags.get();
        if !device.is_null() {
            *device = 123;
        }
        if !qpc.is_null() {
            *qpc = 456;
        }
    }
    OK
}
unsafe extern "system" fn capture_release(this: *mut c_void, frames: u32) -> HRESULT {
    let control = &unsafe { owner(this) }.control;
    control.event(format!("capture.release.{frames}"));
    if !control.capture_active.get() || (frames != 0 && frames != control.capture_frames.get()) {
        return INVALID;
    }
    if control.capture_release_hr.get() != 0 {
        return HRESULT(control.capture_release_hr.get());
    }
    control.capture_active.set(false);
    OK
}
unsafe extern "system" fn capture_next(this: *mut c_void, output: *mut u32) -> HRESULT {
    if output.is_null() {
        return POINTER;
    }
    let control = &unsafe { owner(this) }.control;
    control.event("capture.next");
    unsafe {
        *output = control.capture_frames.get();
    }
    OK
}
const CAPTURE_VTABLE: CaptureVtable = CaptureVtable {
    base: UNKNOWN,
    get: capture_get,
    release: capture_release,
    next: capture_next,
};

#[repr(C)]
struct BitmapVtable {
    base: IUnknown_Vtbl,
    size: unsafe extern "system" fn(*mut c_void, *mut u32, *mut u32) -> HRESULT,
    format: unsafe extern "system" fn(*mut c_void, *mut GUID) -> HRESULT,
    resolution: unsafe extern "system" fn(*mut c_void, *mut f64, *mut f64) -> HRESULT,
    palette: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pixels: unsafe extern "system" fn(*mut c_void, *const i32, u32, u32, *mut u8) -> HRESULT,
    lock: unsafe extern "system" fn(*mut c_void, *const i32, u32, *mut *mut c_void) -> HRESULT,
    set_palette: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    set_resolution: unsafe extern "system" fn(*mut c_void, f64, f64) -> HRESULT,
}
unsafe extern "system" fn bitmap_size(
    _: *mut c_void,
    width: *mut u32,
    height: *mut u32,
) -> HRESULT {
    if width.is_null() || height.is_null() {
        return POINTER;
    }
    unsafe {
        *width = 2;
        *height = 2;
    }
    OK
}
unsafe extern "system" fn bitmap_format(this: *mut c_void, output: *mut GUID) -> HRESULT {
    if output.is_null() {
        return POINTER;
    }
    let control = &unsafe { owner(this) }.control;
    unsafe {
        *output = if control.wic_wrong_format.get() {
            GUID::zeroed()
        } else {
            super::BGRA8
        };
    }
    OK
}
unsafe extern "system" fn bitmap_resolution(_: *mut c_void, x: *mut f64, y: *mut f64) -> HRESULT {
    if x.is_null() || y.is_null() {
        return POINTER;
    }
    unsafe {
        *x = 96.0;
        *y = 96.0;
    }
    OK
}
unsafe extern "system" fn bitmap_pixels(
    _: *mut c_void,
    _: *const i32,
    _: u32,
    _: u32,
    _: *mut u8,
) -> HRESULT {
    NOTIMPL
}
unsafe extern "system" fn bitmap_set_resolution(_: *mut c_void, _: f64, _: f64) -> HRESULT {
    OK
}
unsafe extern "system" fn bitmap_lock(
    this: *mut c_void,
    rect: *const i32,
    flags: u32,
    output: *mut *mut c_void,
) -> HRESULT {
    if rect.is_null() || output.is_null() {
        return POINTER;
    }
    unsafe {
        *output = ptr::null_mut();
    }
    let control = &unsafe { owner(this) }.control;
    control.event(format!("wic.acquire.{flags}"));
    let rect = unsafe { std::slice::from_raw_parts(rect, 4) };
    if flags != 1 || rect != [0, 0, 2, 2] || control.wic_active.get() {
        return INVALID;
    }
    if !control.wic_null.get() {
        control.wic_active.set(true);
        let parent = unsafe { IUnknown::from_raw_borrowed(&this) }
            .unwrap()
            .clone();
        let lock = make(Kind::Lock, control, vec![parent]);
        unsafe {
            *output = lock.into_raw();
        }
    }
    HRESULT(control.wic_lock_hr.get())
}
const BITMAP_VTABLE: BitmapVtable = BitmapVtable {
    base: UNKNOWN,
    size: bitmap_size,
    format: bitmap_format,
    resolution: bitmap_resolution,
    palette: set_event,
    pixels: bitmap_pixels,
    lock: bitmap_lock,
    set_palette: set_event,
    set_resolution: bitmap_set_resolution,
};

#[repr(C)]
struct LockVtable {
    base: IUnknown_Vtbl,
    size: unsafe extern "system" fn(*mut c_void, *mut u32, *mut u32) -> HRESULT,
    stride: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    data: unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut u8) -> HRESULT,
    format: unsafe extern "system" fn(*mut c_void, *mut GUID) -> HRESULT,
}
unsafe extern "system" fn lock_stride(this: *mut c_void, stride: *mut u32) -> HRESULT {
    if stride.is_null() {
        return POINTER;
    }
    unsafe {
        *stride = owner(this).control.wic_stride.get();
    }
    OK
}
unsafe extern "system" fn lock_data(
    this: *mut c_void,
    count: *mut u32,
    output: *mut *mut u8,
) -> HRESULT {
    if count.is_null() || output.is_null() {
        return POINTER;
    }
    let control = &unsafe { owner(this) }.control;
    control.event("wic.data");
    if control.wic_query_hr.get() != 0 {
        return HRESULT(control.wic_query_hr.get());
    }
    if control.wic_count.get() > 256 {
        return INVALID;
    }
    unsafe {
        *count = control.wic_count.get();
        *output = control.bytes();
    }
    OK
}
const LOCK_VTABLE: LockVtable = LockVtable {
    base: UNKNOWN,
    size: bitmap_size,
    stride: lock_stride,
    data: lock_data,
    format: bitmap_format,
};

#[repr(C)]
struct MediaVtable {
    base: IUnknown_Vtbl,
    lock: unsafe extern "system" fn(*mut c_void, *mut *mut u8, *mut u32, *mut u32) -> HRESULT,
    unlock: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    current: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    set_current: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    maximum: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
}
unsafe extern "system" fn media_lock(
    this: *mut c_void,
    output: *mut *mut u8,
    maximum: *mut u32,
    current: *mut u32,
) -> HRESULT {
    // Both optional length cells are deliberately required by this fixture so
    // that a test cannot accidentally pass while omitting either native bound.
    if output.is_null() || maximum.is_null() || current.is_null() {
        return POINTER;
    }
    let control = &unsafe { owner(this) }.control;
    control.event("media.acquire");
    if control.media_max.get() > 256 || control.media_active.get() {
        return INVALID;
    }
    if control.media_lock_hr.get() != 0 {
        return HRESULT(control.media_lock_hr.get());
    }
    control.media_active.set(true);
    unsafe {
        *output = if control.media_null.get() {
            ptr::null_mut()
        } else {
            control.bytes()
        };
        *maximum = control.media_max.get();
        *current = control.media_current.get();
    }
    OK
}
unsafe extern "system" fn media_unlock(this: *mut c_void) -> HRESULT {
    let control = &unsafe { owner(this) }.control;
    control.event("media.unlock");
    if !control.media_active.get() {
        return INVALID;
    }
    if control.media_unlock_hr.get() != 0 {
        return HRESULT(control.media_unlock_hr.get());
    }
    control.media_active.set(false);
    OK
}
unsafe extern "system" fn media_current(this: *mut c_void, output: *mut u32) -> HRESULT {
    if output.is_null() {
        return POINTER;
    }
    unsafe {
        *output = owner(this).control.media_current.get();
    }
    OK
}
unsafe extern "system" fn media_set(this: *mut c_void, length: u32) -> HRESULT {
    let control = &unsafe { owner(this) }.control;
    control.event(format!("media.length.{length}"));
    if length > control.media_max.get() || length > 256 {
        return INVALID;
    }
    if control.media_set_hr.get() != 0 {
        return HRESULT(control.media_set_hr.get());
    }
    control.media_current.set(length);
    OK
}
unsafe extern "system" fn media_max(this: *mut c_void, output: *mut u32) -> HRESULT {
    if output.is_null() {
        return POINTER;
    }
    unsafe {
        *output = owner(this).control.media_max.get();
    }
    OK
}
const MEDIA_VTABLE: MediaVtable = MediaVtable {
    base: UNKNOWN,
    lock: media_lock,
    unlock: media_unlock,
    current: media_current,
    set_current: media_set,
    maximum: media_max,
};

pub struct CopyFixture {
    objects: Vec<IUnknown>,
    control: Rc<Control>,
}

impl Default for CopyFixture {
    fn default() -> Self {
        let control = Rc::new(Control::default());
        let render = make(Kind::Render, &control, vec![]);
        let capture = make(Kind::Capture, &control, vec![]);
        let client = make(
            Kind::Client,
            &control,
            vec![render.clone(), capture.clone()],
        );
        let bitmap = make(Kind::Bitmap, &control, vec![]);
        let media = make(Kind::Media, &control, vec![]);
        Self {
            objects: vec![client, render, capture, bitmap, media],
            control,
        }
    }
}

impl CopyFixture {
    pub fn object(&self, kind: &str) -> result::Result<IUnknown> {
        if kind == "mediaUnknown" {
            return super::query(&self.object("media")?, &IUnknown::IID);
        }
        let index = match kind {
            "client" => 0,
            "render" => 1,
            "capture" => 2,
            "bitmap" => 3,
            "media" => 4,
            _ => return Err(error("Unknown borrowed-copy fixture object")),
        };
        self.objects
            .get(index)
            .cloned()
            .ok_or_else(|| error("Fixture owners have been released"))
    }
    pub fn release_owners(&mut self) {
        self.objects.clear();
    }
    pub fn configure(&self, key: &str, value: u32) -> result::Result<()> {
        let c = &self.control;
        match key {
            "initializeHr" => c.initialize_hr.set(value as i32),
            "externalInitialized" => c.initialized.set(value != 0),
            "capacity" => c.capacity.set(value),
            "renderNull" => c.render_null.set(value != 0),
            "renderAcquireHr" => c.render_acquire_hr.set(value as i32),
            "renderReleaseHr" => c.render_release_hr.set(value as i32),
            "captureHr" => c.capture_hr.set(value as i32),
            "captureFrames" => c.capture_frames.set(value),
            "captureFlags" => c.capture_flags.set(value),
            "captureNull" => c.capture_null.set(value != 0),
            "captureReleaseHr" => c.capture_release_hr.set(value as i32),
            "serviceWrong" => c.service_wrong.set(value != 0),
            "mediaMax" => c.media_max.set(value),
            "mediaCurrent" => c.media_current.set(value),
            "mediaLockHr" => c.media_lock_hr.set(value as i32),
            "mediaUnlockHr" => c.media_unlock_hr.set(value as i32),
            "mediaSetHr" => c.media_set_hr.set(value as i32),
            "mediaNull" => c.media_null.set(value != 0),
            "wicStride" => c.wic_stride.set(value),
            "wicCount" => c.wic_count.set(value),
            "wicLockHr" => c.wic_lock_hr.set(value as i32),
            "wicNull" => c.wic_null.set(value != 0),
            "wicWrongFormat" => c.wic_wrong_format.set(value != 0),
            "wicQueryHr" => c.wic_query_hr.set(value as i32),
            _ => return Err(error("Unknown borrowed-copy fixture setting")),
        }
        Ok(())
    }
    pub fn events(&self) -> Vec<String> {
        self.control.events.borrow().clone()
    }
    pub fn bytes(&self) -> Vec<u8> {
        unsafe { (&*self.control.bytes.get()).to_vec() }
    }
    #[cfg(test)]
    pub fn share_render_service(&self, other: &Self) {
        let target = other.object("render").unwrap();
        let client = self.object("client").unwrap();
        let owner = unsafe { owner(client.as_raw()) };
        let old = std::mem::replace(&mut owner.held.borrow_mut()[0], target);
        drop(old);
    }
    pub fn install_callback_probe(&self, sink: IUnknown) -> result::Result<()> {
        const IID: GUID = GUID::from_u128(0x5bbdf51f_3e8d_40c5_ad0f_d8b341172d12);
        let sink = super::query(&sink, &IID)?;
        let table = crate::MetadataTable::new();
        let method = super::super::register_interface(
            &table,
            "borrowed-test-callback",
            IID,
            super::InterfaceBase::IUnknown,
        )
        .add_method_at(
            3,
            "Probe",
            super::MethodSignature::new(&table).preserve_hresult(),
        )?
        .method(3)
        .unwrap();
        let weak = Rc::downgrade(&self.control);
        *self.control.hook.borrow_mut() = Some(Rc::new(move |event| {
            if event.starts_with("render.acquire.") {
                let rejected = unsafe { method.invoke(sink.as_raw(), &[]) }.is_err();
                if let Some(control) = weak.upgrade() {
                    control
                        .events
                        .borrow_mut()
                        .push(format!("callback.rejected.{rejected}"));
                }
            }
        }));
        Ok(())
    }
    #[cfg(test)]
    pub fn hook(&self, hook: Option<Rc<dyn Fn(&str)>>) {
        *self.control.hook.borrow_mut() = hook;
    }
}

#[test]
fn borrowed_copy_fixture_vtables_have_exact_sdk_slots_on_every_pointer_width() {
    use std::mem::{offset_of, size_of};
    let pointer = size_of::<*const c_void>();
    assert_eq!(size_of::<ClientVtable>(), 21 * pointer);
    assert_eq!(offset_of!(ClientVtable, initialize), 3 * pointer);
    assert_eq!(offset_of!(ClientVtable, service), 14 * pointer);
    assert_eq!(offset_of!(ClientVtable, initialize_shared), 20 * pointer);
    assert_eq!(size_of::<RenderVtable>(), 5 * pointer);
    assert_eq!(size_of::<CaptureVtable>(), 6 * pointer);
    assert_eq!(size_of::<BitmapVtable>(), 11 * pointer);
    assert_eq!(offset_of!(BitmapVtable, lock), 8 * pointer);
    assert_eq!(size_of::<LockVtable>(), 7 * pointer);
    assert_eq!(offset_of!(LockVtable, data), 5 * pointer);
    assert_eq!(size_of::<MediaVtable>(), 8 * pointer);
    assert_eq!(offset_of!(MediaVtable, set_current), 6 * pointer);
}
