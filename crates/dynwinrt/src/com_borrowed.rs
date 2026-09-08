// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! COM-local owned-copy transactions. Native storage never leaves a transaction.
//! Interface-specific calls use completed MethodHandles/libffi; standard
//! IUnknown ownership and COM apartment queries use shared infrastructure.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::c_void,
    ptr,
    rc::{Rc, Weak},
    sync::{Arc, LazyLock, Mutex, Weak as SyncWeak},
    thread::{self, ThreadId},
};

use windows::Win32::System::Com::{APTTYPE_MAINSTA, APTTYPE_STA, CoGetApartmentType};
use windows_core::{GUID, HRESULT, IUnknown, Interface as _};

use super::{AudioFormatValue, InterfaceBase, MethodHandle, MethodSignature, Type, Value};
use crate::{MetadataTable, WinRTValue, result};

use contracts::AudioRole;
pub use dynwinrt_com_contracts::{
    self as contracts, BorrowOwner, BorrowedCopyContract, CallContract, CopyKind, MemoryAccess,
    Storage,
};

#[path = "com_borrowed_plan.rs"]
mod plan;
pub use plan::BorrowedCopyPlan;

#[cfg(any(test, feature = "test-hooks"))]
#[path = "com_borrowed_test_support.rs"]
pub mod testing;
#[cfg(test)]
#[path = "com_borrowed_tests.rs"]
mod tests;

pub const AUDIO_CLIENT: GUID = GUID::from_u128(0x1cb9ad4c_dbfa_4c32_b178_c2f568a703b2);
pub const AUDIO_CLIENT2: GUID = GUID::from_u128(0x726778cd_f60a_4eda_82de_e47610cd78aa);
pub const AUDIO_CLIENT3: GUID = GUID::from_u128(0x7ed4ee07_8e67_4cd4_8c1a_2b7a5987ad42);
pub const AUDIO_RENDER: GUID = GUID::from_u128(0xf294acfc_3146_4483_a7bf_addca7c260e2);
pub const AUDIO_CAPTURE: GUID = GUID::from_u128(0xc8adbd64_e71e_48a0_a4de_185c395cd317);
pub const WIC_BITMAP: GUID = GUID::from_u128(0x00000121_a8f2_4877_ba0a_fd2b6645fb94);
pub const WIC_LOCK: GUID = GUID::from_u128(0x00000123_a8f2_4877_ba0a_fd2b6645fb94);
pub const MF_BUFFER: GUID = GUID::from_u128(0x045fa593_8799_42b8_bc8d_8968c6453507);
#[cfg(any(test, feature = "test-hooks"))]
const BGRA8: GUID = GUID::from_u128(0x6fddc324_4e03_4bfe_b185_3d77768dc90f);
#[cfg(test)]
const BUFFER_EMPTY: i32 = 0x08890001;
#[cfg(test)]
const SILENT: u32 = 2;
#[cfg(test)]
const TIMESTAMP_ERROR: u32 = 4;
const EVENT_CALLBACK: u32 = 0x00040000;
const MAX_COPY_BYTES: usize = 64 * 1024 * 1024;

fn error(message: impl AsRef<str>) -> result::Error {
    super::invalid_argument(message.as_ref())
}

thread_local! {
    // An address is a key only while IdentityInner owns its canonical IUnknown.
    // Weak entries neither keep native objects alive nor form client/service cycles.
    static IDENTITIES: RefCell<HashMap<usize, Weak<IdentityInner>>> = RefCell::new(HashMap::new());
    static COPY_DEPTH: Cell<usize> = const { Cell::new(0) };
}

// Only pointer-free ownership tokens cross threads. All native references and
// mutable lifecycle/provenance state remain in the owner-thread Rc sidecar.
static IDENTITY_OWNERS: LazyLock<Mutex<HashMap<usize, SyncWeak<ThreadId>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn identity_owner(key: usize, claim: bool) -> result::Result<Option<Arc<ThreadId>>> {
    let mut owners = IDENTITY_OWNERS.lock().map_err(|poison| {
        drop(poison);
        error("COM identity owner registry is poisoned")
    })?;
    if let Some(owner) = owners.get(&key).and_then(SyncWeak::upgrade) {
        if *owner != thread::current().id() {
            drop(owners);
            return Err(error(
                "COM borrowed-copy identity belongs to a different apartment thread",
            ));
        }
        return Ok(Some(owner));
    }
    if !claim {
        return Ok(None);
    }
    owners.retain(|_, owner| owner.strong_count() != 0);
    let owner = Arc::new(thread::current().id());
    owners.insert(key, Arc::downgrade(&owner));
    Ok(Some(owner))
}

pub fn callbacks_suppressed() -> bool {
    COPY_DEPTH.with(|depth| depth.get() != 0)
}

pub fn has_live_contexts() -> result::Result<bool> {
    let owners = IDENTITY_OWNERS.lock().map_err(|poison| {
        drop(poison);
        error("COM identity owner registry is poisoned")
    })?;
    Ok(owners.values().any(|owner| owner.strong_count() != 0))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Acquiring,
    Active,
    Finalizing,
    Poisoned,
}

struct InitializedAudio {
    format: AudioFormatValue,
    share_mode: i32,
    flags: u32,
    block_align: Option<usize>,
}

struct ServiceOrigin {
    client: Identity,
    // The canonical unknown is identity, not necessarily an IAudioClient view.
    client_view: Rc<IUnknown>,
}

struct IdentityInner {
    state: Cell<State>,
    initialized: RefCell<Option<InitializedAudio>>,
    origin: RefCell<Option<ServiceOrigin>>,
    services: Cell<u8>,
    same_identity_client_view: RefCell<Option<Rc<IUnknown>>>,
    _owner_token: Arc<ThreadId>,
    // Drop the pointer-free registry token before this last field. An active
    // address-key token can therefore never outlive the canonical native owner.
    _canonical: IUnknown,
}

/// Private sidecar retained by managed COM carriers, never by a global strong map.
/// Not Send/Sync: no apartment reference or Rc operation is allowed off its owner.
///
/// ```compile_fail
/// fn require_send<T: Send>() {}
/// require_send::<dynwinrt::com::borrowed::Identity>();
/// ```
/// ```compile_fail
/// fn require_sync<T: Sync>() {}
/// require_sync::<dynwinrt::com::borrowed::Identity>();
/// ```
#[derive(Clone)]
pub struct Identity {
    owner: ThreadId,
    inner: Rc<IdentityInner>,
}

type AudioOwner = (Identity, Rc<IUnknown>);

impl Identity {
    pub fn lookup(object: &IUnknown) -> result::Result<Option<Self>> {
        if !has_live_contexts()? {
            return Ok(None);
        }
        let canonical = query(object, &IUnknown::IID)?;
        let _owner = identity_owner(canonical.as_raw().addr(), false)?;
        let inner = IDENTITIES.with(|entries| {
            entries
                .borrow()
                .get(&canonical.as_raw().addr())
                .and_then(Weak::upgrade)
        });
        Ok(inner.map(|inner| Self {
            owner: thread::current().id(),
            inner,
        }))
    }

    pub fn for_object(object: &IUnknown) -> result::Result<Self> {
        let canonical = query(object, &IUnknown::IID)?;
        let key = canonical.as_raw().addr();
        let existing =
            IDENTITIES.with(|entries| entries.borrow().get(&key).and_then(Weak::upgrade));
        let inner = if let Some(existing) = existing {
            existing
        } else {
            let owner_token = identity_owner(key, true)?.expect("claimed identity owner");
            let inner = Rc::new(IdentityInner {
                _canonical: canonical,
                state: Cell::new(State::Idle),
                initialized: RefCell::new(None),
                origin: RefCell::new(None),
                services: Cell::new(0),
                same_identity_client_view: RefCell::new(None),
                _owner_token: owner_token,
            });
            IDENTITIES.with(|entries| {
                let mut entries = entries.borrow_mut();
                entries.retain(|_, entry| entry.strong_count() != 0);
                entries.insert(key, Rc::downgrade(&inner));
            });
            inner
        };
        Ok(Self {
            owner: thread::current().id(),
            inner,
        })
    }

    pub fn ensure_thread(&self) -> result::Result<()> {
        if self.owner != thread::current().id() {
            return Err(error(
                "Borrowed-copy context used from a different apartment thread",
            ));
        }
        Ok(())
    }

    pub fn ensure_idle(&self) -> result::Result<()> {
        self.ensure_thread()?;
        match self.inner.state.get() {
            State::Idle => {}
            State::Poisoned => return Err(error("Borrowed-copy context is poisoned")),
            _ => {
                return Err(error(
                    "Borrowed-copy context is busy (reentrant operation rejected)",
                ));
            }
        }
        let origin = self
            .inner
            .origin
            .borrow()
            .as_ref()
            .map(|origin| origin.client.clone());
        if let Some(origin) = origin {
            origin.ensure_idle()?;
        }
        Ok(())
    }

    fn matches(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    fn audio_owner(&self, service: AudioRole) -> result::Result<AudioOwner> {
        self.ensure_thread()?;
        let role = match service {
            AudioRole::Render => 1,
            AudioRole::Capture => 2,
        };
        if self.inner.services.get() & role == 0 {
            return Err(error(
                "Audio copy requires an observed successful GetService for this interface",
            ));
        }
        let origin = self
            .inner
            .origin
            .borrow()
            .as_ref()
            .map(|origin| (origin.client.clone(), Rc::clone(&origin.client_view)));
        if let Some(origin) = origin {
            return Ok(origin);
        }
        let view = self.inner.same_identity_client_view.borrow().clone();
        if let Some(view) = view {
            return Ok((self.clone(), view));
        }
        Err(error(
            "Audio copy requires an observed successful Initialize and GetService",
        ))
    }

    fn bind_service(&self, client: &Self, client_view: IUnknown, iid: GUID) -> result::Result<()> {
        self.ensure_thread()?;
        client.ensure_thread()?;
        if client.inner.origin.borrow().is_some() {
            return Err(error(
                "An audio service cannot become another service's client origin",
            ));
        }
        if self.matches(client) {
            {
                let mut view = self.inner.same_identity_client_view.borrow_mut();
                if view.is_none() {
                    *view = Some(Rc::new(client_view));
                }
            }
            self.inner
                .services
                .set(self.inner.services.get() | if iid == AUDIO_RENDER { 1 } else { 2 });
            return Ok(());
        }
        // Never create A -> B -> A, or silently relabel an initialized client.
        if self.inner.initialized.borrow().is_some() {
            return Err(error("Conflicting audio service identity mapping"));
        }
        let mut origin = self.inner.origin.borrow_mut();
        let conflicting = origin
            .as_ref()
            .is_some_and(|existing| !existing.client.matches(client));
        if origin.is_none() {
            *origin = Some(ServiceOrigin {
                client: client.clone(),
                client_view: Rc::new(client_view),
            });
        }
        drop(origin);
        if conflicting {
            return Err(error("Conflicting audio service origin"));
        }
        self.inner
            .services
            .set(self.inner.services.get() | if iid == AUDIO_RENDER { 1 } else { 2 });
        Ok(())
    }
}

fn query(object: &IUnknown, iid: &GUID) -> result::Result<IUnknown> {
    let mut output = ptr::null_mut();
    unsafe { object.query(iid, &mut output).ok()? };
    if output.is_null() {
        return Err(error("QueryInterface returned a required null interface"));
    }
    Ok(unsafe { IUnknown::from_raw(output) })
}

struct Transaction {
    identities: Vec<Identity>,
    suppress_callbacks: bool,
    finalizer: Option<Finalizer>,
    finished: bool,
}

impl Transaction {
    fn enter(
        identity: &Identity,
        owner: Option<&Identity>,
        suppress: bool,
    ) -> result::Result<Self> {
        identity.ensure_idle()?;
        let mut identities = vec![identity.clone()];
        if let Some(owner) = owner {
            owner.ensure_idle()?;
            if !owner.matches(identity) {
                identities.push(owner.clone());
            }
        }
        for identity in &identities {
            identity.inner.state.set(State::Acquiring);
        }
        if suppress {
            COPY_DEPTH.with(|depth| depth.set(depth.get() + 1));
        }
        Ok(Self {
            identities,
            suppress_callbacks: suppress,
            finalizer: None,
            finished: false,
        })
    }

    fn acquired(&mut self, finalizer: Finalizer) {
        // Arm before inspecting any native length/pointer or allocating copy storage.
        self.finalizer = Some(finalizer);
        self.set_state(State::Active);
    }

    fn set_state(&self, state: State) {
        for identity in &self.identities {
            identity.inner.state.set(state);
        }
    }

    fn poison(&self) {
        self.set_state(State::Poisoned);
    }

    fn complete<T>(
        mut self,
        result: result::Result<T>,
        commit: &[WinRTValue],
    ) -> result::Result<T> {
        let cleanup = self.finalize(if result.is_ok() { Some(commit) } else { None });
        self.finished = true;
        match (result, cleanup) {
            (_, Err(cleanup)) => Err(cleanup),
            (result, Ok(())) => result,
        }
    }

    fn finalize(&mut self, commit: Option<&[WinRTValue]>) -> result::Result<()> {
        for identity in &self.identities {
            identity.ensure_thread()?;
        }
        // Taking first prevents a failed finalizer from ever being retried by Drop.
        let finalizer = self.finalizer.take();
        let poisoned = self
            .identities
            .iter()
            .any(|id| id.inner.state.get() == State::Poisoned);
        self.set_state(State::Finalizing);
        let result = match finalizer {
            Some(Finalizer::Call {
                method,
                view,
                abort,
            }) => exact_success(&method, &view, commit.unwrap_or(&abort)),
            Some(Finalizer::Release(owner)) => {
                drop(owner);
                Ok(())
            }
            None => Ok(()),
        };
        self.set_state(if result.is_err() || poisoned {
            State::Poisoned
        } else {
            State::Idle
        });
        result
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.finalize(None);
        }
        if self.suppress_callbacks {
            COPY_DEPTH.with(|depth| depth.set(depth.get() - 1));
        }
    }
}

enum Finalizer {
    Call {
        method: MethodHandle,
        view: IUnknown,
        abort: Vec<WinRTValue>,
    },
    Release(OwnedInterfaceCell),
}

#[derive(Default)]
struct OwnedInterfaceCell(*mut c_void);

impl Drop for OwnedInterfaceCell {
    fn drop(&mut self) {
        if !self.0.is_null() {
            drop(unsafe { IUnknown::from_raw(self.0) });
        }
    }
}

pub struct CapturePacket {
    pub data: Option<Vec<u8>>,
    pub frames: u32,
    pub flags: u32,
    pub device_position: Option<u64>,
    pub qpc_position: Option<u64>,
}

pub struct BitmapCopy {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

fn cell<T>(value: &mut T) -> WinRTValue {
    WinRTValue::RawPtr((value as *mut T).cast())
}

fn call(method: &MethodHandle, view: &IUnknown, args: &[WinRTValue]) -> result::Result<HRESULT> {
    let result = unsafe { method.invoke(view.as_raw(), args) }?;
    match result.as_slice() {
        [WinRTValue::I32(value)] => Ok(HRESULT(*value)),
        [WinRTValue::HResult(value)] => Ok(*value),
        _ => Err(error("Borrowed-copy HRESULT call-plan result mismatch")),
    }
}

fn exact_success(
    method: &MethodHandle,
    view: &IUnknown,
    args: &[WinRTValue],
) -> result::Result<()> {
    let hr = call(method, view, args)?;
    if hr.0 == 0 {
        Ok(())
    } else {
        Err(error(format!(
            "Borrowed-copy call returned unexpected HRESULT 0x{:08X}",
            hr.0 as u32
        )))
    }
}

fn checked_extent(bytes: usize) -> result::Result<()> {
    if bytes > MAX_COPY_BYTES || bytes > isize::MAX as usize {
        return Err(error("Borrowed-copy extent exceeds the 64 MiB safety cap"));
    }
    Ok(())
}

fn frame_bytes(frames: u32, block: usize) -> result::Result<usize> {
    let bytes = (frames as usize)
        .checked_mul(block)
        .ok_or_else(|| error("Audio byte extent overflow"))?;
    checked_extent(bytes)?;
    Ok(bytes)
}

fn validate_pointer(pointer: *const u8, bytes: usize) -> result::Result<()> {
    checked_extent(bytes)?;
    if bytes != 0 && (pointer.is_null() || pointer.addr().checked_add(bytes).is_none()) {
        return Err(error(
            "Native borrowed buffer is null or its address extent overflows",
        ));
    }
    Ok(())
}

fn allocate(length: usize) -> result::Result<Vec<u8>> {
    checked_extent(length)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| error("Unable to allocate owned copy"))?;
    output.resize(length, 0);
    Ok(output)
}

fn copy_bytes(pointer: *const u8, bytes: usize) -> result::Result<Vec<u8>> {
    validate_pointer(pointer, bytes)?;
    let mut data = allocate(bytes)?;
    if bytes != 0 {
        unsafe { ptr::copy_nonoverlapping(pointer, data.as_mut_ptr(), bytes) };
    }
    Ok(data)
}

fn uncompressed_block_align(format: &AudioFormatValue) -> Option<usize> {
    let bits = format.bits_per_sample();
    let mut tag = format.format_tag();
    if tag == 0xfffe {
        let extra = format.extra_data();
        if extra.len() != 22 {
            return None;
        }
        let valid = u16::from_le_bytes(extra[0..2].try_into().ok()?);
        let mask = u32::from_le_bytes(extra[2..6].try_into().ok()?);
        if valid == 0
            || valid > bits
            || (mask != 0 && mask.count_ones() != u32::from(format.channels()))
        {
            return None;
        }
        let subformat = unsafe { ptr::read_unaligned(extra[6..22].as_ptr().cast::<GUID>()) };
        tag = if subformat == GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71) {
            1
        } else if subformat == GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71)
            && valid == bits
        {
            3
        } else {
            return None;
        };
    } else if !format.extra_data().is_empty() {
        return None;
    }
    if format.channels() == 0
        || format.samples_per_second() == 0
        || !(tag == 1 && matches!(bits, 8 | 16 | 24 | 32) || tag == 3 && matches!(bits, 32 | 64))
    {
        return None;
    }
    let block = u32::from(format.channels()).checked_mul(u32::from(bits / 8))?;
    if block == 0
        || block != u32::from(format.block_align())
        || format.samples_per_second().checked_mul(block)? != format.average_bytes_per_second()
    {
        return None;
    }
    Some(block as usize)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextEffect {
    AudioInitialize,
    AudioInitializeShared,
    AudioGetService,
}

pub struct ManagedCallOutput {
    pub values: Vec<(Value, super::PointerOutputKind)>,
    pub output_context: Option<Identity>,
}

/// Executes the actual registered call; there is intentionally no "record success"
/// or attach-format API. The receiver and converted inputs remain pinned throughout.
///
/// # Safety
/// The receiver and pointer-shaped inputs must satisfy the registered native ABI.
pub unsafe fn invoke_managed(
    method: &MethodHandle,
    context: &Identity,
    view: &IUnknown,
    args: &[Value],
) -> result::Result<ManagedCallOutput> {
    context.ensure_idle()?;
    let transaction = method
        .0
        .context_effect
        .map(|_| Transaction::enter(context, None, true))
        .transpose()?;
    if !context.matches(&Identity::for_object(view)?) {
        return Err(error("Context effect receiver identity mismatch"));
    }
    let Some(effect) = method.0.context_effect else {
        return Ok(ManagedCallOutput {
            values: unsafe { method.invoke_values_with_output_kinds(view.as_raw(), args)? },
            output_context: None,
        });
    };
    let transaction = transaction.expect("context effect transaction");
    let format_input = match effect {
        ContextEffect::AudioInitialize => Some((4, 0, 1)),
        ContextEffect::AudioInitializeShared => Some((2, usize::MAX, 0)),
        ContextEffect::AudioGetService => None,
    };
    let initialized = if let Some((format_index, mode_index, flags_index)) = format_input {
        if context.inner.origin.borrow().is_some() {
            return Err(error(
                "An audio service identity cannot be independently initialized",
            ));
        }
        let Some(Value::AudioFormat(format)) = args.get(format_index) else {
            return Err(error(
                "Audio initialization requires owned WAVEFORMATEX storage",
            ));
        };
        let mode = if mode_index == usize::MAX {
            0
        } else {
            match args.get(mode_index) {
                Some(Value::WinRt(WinRTValue::I32(value))) => *value,
                _ => return Err(error("Audio share mode ABI mismatch")),
            }
        };
        let flags = match args.get(flags_index) {
            Some(Value::WinRt(WinRTValue::U32(value))) => *value,
            _ => return Err(error("Audio stream flags ABI mismatch")),
        };
        Some(InitializedAudio {
            format: format.clone(),
            share_mode: mode,
            flags,
            block_align: uncompressed_block_align(format),
        })
    } else {
        None
    };
    let iid = if effect == ContextEffect::AudioGetService {
        let Some(Value::WinRt(WinRTValue::RawPtr(pointer))) = args.first() else {
            return Err(error("GetService requires REFIID storage"));
        };
        if pointer.is_null() {
            return Err(error("GetService REFIID is required"));
        }
        Some(unsafe { ptr::read_unaligned(pointer.cast::<GUID>()) })
    } else {
        None
    };
    let mut values = method
        .0
        .context_hresult_plan
        .as_ref()
        .expect("validated context call")
        .invoke_values_with_output_kinds(view.as_raw(), args)?;
    let hresult = values.remove(0).0;
    let hr = match hresult {
        Value::WinRt(WinRTValue::I32(value)) => value,
        Value::WinRt(WinRTValue::HResult(value)) => value.0,
        _ => return Err(error("Context-effect HRESULT plan mismatch")),
    };
    let mut service = if effect == ContextEffect::AudioGetService {
        let raw = match values.first_mut() {
            Some((Value::WinRt(WinRTValue::RawPtr(raw)), super::PointerOutputKind::Com)) => {
                std::mem::replace(raw, ptr::null_mut())
            }
            _ => return Err(error("GetService requires an owned interface output plan")),
        };
        Some(OwnedInterfaceCell(raw))
    } else {
        None
    };
    if hr != 0 {
        transaction.poison();
        return Err(error(
            "Unexpected successful audio context HRESULT; provenance not recorded",
        ));
    }
    if let Some(initialized) = initialized {
        if context.inner.initialized.borrow().is_some() {
            transaction.poison();
            return Err(error(
                "Audio initialization succeeded twice; immutable provenance cannot be overwritten",
            ));
        }
        *context.inner.initialized.borrow_mut() = Some(initialized);
    }
    let output_context = if let (Some(iid), Some(service)) = (iid, service.as_ref()) {
        let object = unsafe { IUnknown::from_raw_borrowed(&service.0) }
            .ok_or_else(|| error("GetService returned a required null interface"))?;
        if iid == AUDIO_RENDER || iid == AUDIO_CAPTURE {
            // A truthful QI pins the exact service view; never reinterpret the client.
            let verified = query(object, &iid)?;
            let identity = Identity::for_object(&verified)?;
            identity.bind_service(context, query(view, &AUDIO_CLIENT)?, iid)?;
            Some(identity)
        } else {
            None
        }
    } else {
        None
    };
    if let Some(service) = service.as_mut() {
        let raw = std::mem::replace(&mut service.0, ptr::null_mut());
        values[0].0 = Value::WinRt(WinRTValue::RawPtr(raw));
    }
    transaction.complete(
        Ok(ManagedCallOutput {
            values,
            output_context,
        }),
        &[],
    )
}

impl MethodSignature {
    pub fn with_context_effect(mut self, effect: ContextEffect) -> Self {
        self.context_effect = Some(effect);
        self
    }
}

pub(super) fn validate_effect_signature(
    signature: &MethodSignature,
    effect: ContextEffect,
    iid: GUID,
    name: &str,
    slot: usize,
) -> result::Result<()> {
    use super::{ComParameterDirection as Direction, ComReturnPlan, PointerOutputKind};
    use crate::{TypeKind, native_call::ParameterType};
    let params = &signature.parameters;
    let scalar = |index: usize, kind: TypeKind| {
        params.get(index).is_some_and(|param| {
            param.direction == Direction::In
                && matches!(&param.typ.abi, ParameterType::WinRT(typ) if typ.kind() == kind)
        })
    };
    let pointer = |index: usize, direction: Direction| {
        params.get(index).is_some_and(|param| {
            param.direction == direction && matches!(param.typ.abi, ParameterType::Pointer)
        })
    };
    let audio = |index: usize| {
        params.get(index).is_some_and(|param| {
            param.direction == Direction::In
                && matches!(
                    param.typ.abi,
                    ParameterType::AudioFormat {
                        nullable_input: false
                    }
                )
                && !param.nullable
        })
    };
    let client = iid == AUDIO_CLIENT || iid == AUDIO_CLIENT2 || iid == AUDIO_CLIENT3;
    let valid = matches!(signature.return_plan, ComReturnPlan::HResult)
        && params.iter().all(|param| param.buffer.is_none())
        && match effect {
            ContextEffect::AudioInitialize => {
                client
                    && name == "Initialize"
                    && slot == 3
                    && params.len() == 6
                    && scalar(0, TypeKind::I32)
                    && scalar(1, TypeKind::U32)
                    && scalar(2, TypeKind::I64)
                    && scalar(3, TypeKind::I64)
                    && audio(4)
                    && pointer(5, Direction::In)
            }
            ContextEffect::AudioInitializeShared => {
                iid == AUDIO_CLIENT3
                    && name == "InitializeSharedAudioStream"
                    && slot == 20
                    && params.len() == 4
                    && scalar(0, TypeKind::U32)
                    && scalar(1, TypeKind::U32)
                    && audio(2)
                    && pointer(3, Direction::In)
            }
            ContextEffect::AudioGetService => {
                client
                    && name == "GetService"
                    && slot == 14
                    && params.len() == 2
                    && pointer(0, Direction::In)
                    && pointer(1, Direction::Out)
                    && !params[0].nullable
                    && params[1].typ.pointer_output == PointerOutputKind::Com
            }
        };
    if valid {
        Ok(())
    } else {
        Err(error(
            "Audio context effect does not match the exact IID/slot/ABI contract",
        ))
    }
}
