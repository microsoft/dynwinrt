// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use std::collections::{HashMap, HashSet};

type Check<T = ()> = Result<T, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    U32(Unit),
    Timestamp,
    Guid,
    Bytes,
    Owner(String),
    Rect,
    Layout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Before,
    Acquire,
    After,
    Commit,
    Cleanup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Producer {
    call: String,
    target: Target,
    phase: Phase,
}

#[derive(Clone)]
struct Value {
    kind: Kind,
    producer: Option<Producer>,
}

#[derive(Clone)]
enum ExtentProof {
    Frames {
        frames: FrameCountRef,
        from_input: bool,
    },
    Linear {
        current: ByteCountRef,
        length: ByteCountRef,
        producer: Producer,
    },
    Rows {
        width: PixelCountRef,
        height: PixelCountRef,
        producer: Producer,
    },
}

struct Validator<'a> {
    registry: &'a Registry,
    contract: &'a BorrowedCopyContract,
    operation: &'a Operation,
    values: HashMap<String, Value>,
    extents: HashMap<String, ExtentProof>,
    called: HashSet<String>,
    bounded_rects: HashSet<String>,
}

fn require(condition: bool, reason: &str) -> Check {
    if condition {
        Ok(())
    } else {
        Err(reason.into())
    }
}

fn guid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, ch)| {
            if [8, 13, 18, 23].contains(&i) {
                ch == b'-'
            } else {
                ch.is_ascii_hexdigit()
            }
        })
}

impl Registry {
    /// Checks the closed typed graph, not permission to execute an untrusted graph.
    pub fn validate(&self, contract: &BorrowedCopyContract) -> Check {
        require(
            contract.version == DESCRIPTOR_VERSION
                && contract.metadata_sha256 == METADATA_SHA256
                && contract.exposure == "owned-copy-only",
            REGENERATE,
        )?;
        let unique = |names: Vec<String>| {
            let count = names.len();
            names.into_iter().collect::<HashSet<_>>().len() == count
        };
        require(
            unique(
                self.interfaces
                    .iter()
                    .map(InterfaceRecord::identity)
                    .collect(),
            ),
            "Duplicate registry interface identity",
        )?;
        require(
            unique(self.evidence.iter().map(|e| e.id.clone()).collect()),
            "Duplicate registry evidence",
        )?;
        require(
            unique(contract.calls.iter().map(|c| c.id.clone()).collect()),
            "Duplicate call binding",
        )?;
        require(
            !contract.calls.is_empty()
                && contract.calls.len() <= 32
                && !contract.operations.is_empty()
                && contract.operations.len() <= 6,
            "Unsupported recipe size",
        )?;
        let receiver = self.interface(&contract.receiver)?;
        require(guid(&receiver.iid), "Invalid receiver IID")?;
        for dependency in &contract.dependencies {
            self.interface(dependency)?;
        }
        require(
            (contract.owner == BorrowOwner::AudioServiceOrigin) == contract.audio_origin.is_some(),
            "Audio owner/layout capability mismatch",
        )?;
        if let Some(origin) = &contract.audio_origin {
            require(
                self.interface(&origin.interface)?.context
                    && contract.dependencies.contains(&origin.interface),
                "Audio origin is not a reviewed context dependency",
            )?;
            require(
                origin.role != AudioRole::Render || origin.reject_exclusive_event,
                "Full/zero render cleanup requires exclusive-event rejection",
            )?;
        }
        require(
            contract.acquired_hresult == 0 && contract.empty_hresult.is_none_or(|hr| hr > 0),
            "Unsupported acquisition HRESULT conditions",
        )?;
        let mut operations = HashSet::new();
        let mut used = HashSet::new();
        for operation in &contract.operations {
            require(
                operations.insert(operation.api.name()),
                "Duplicate public operation",
            )?;
            let mut validator = Validator {
                registry: self,
                contract,
                operation,
                values: HashMap::new(),
                extents: HashMap::new(),
                called: HashSet::new(),
                bounded_rects: HashSet::new(),
            };
            validator.validate()?;
            used.extend(validator.called);
        }
        require(
            used.len() == contract.calls.len(),
            "Unreachable native call in recipe",
        )
    }
}

impl Validator<'_> {
    fn define(&mut self, name: &str, kind: Kind, producer: Option<Producer>) -> Check {
        require(
            !name.is_empty() && name.len() <= 96,
            "Invalid value binding name",
        )?;
        require(
            !self.values.contains_key(name) && !self.extents.contains_key(name),
            "Duplicate value/output binding",
        )?;
        self.values.insert(name.into(), Value { kind, producer });
        Ok(())
    }

    fn value(&self, name: &str, kind: &Kind) -> Check<&Value> {
        let value = self
            .values
            .get(name)
            .ok_or_else(|| format!("Dangling, forward or uninitialized reference {name}"))?;
        require(
            &value.kind == kind,
            &format!("Wrong output/reference type for {name}"),
        )?;
        Ok(value)
    }

    fn native(&self, name: &str, kind: &Kind) -> Check<&Producer> {
        self.value(name, kind)?
            .producer
            .as_ref()
            .ok_or_else(|| format!("Native extent evidence required for {name}"))
    }

    fn source(&self, source: &U32Source) -> Check {
        let (name, unit) = match source {
            U32Source::Constant(_) | U32Source::SilentFlag(_) => return Ok(()),
            U32Source::Bytes(r) => (&r.0, Unit::Bytes),
            U32Source::Frames(r) => (&r.0, Unit::Frames),
            U32Source::Flags(r) => (&r.0, Unit::Flags),
            U32Source::Pixels(r) => (&r.0, Unit::Pixels),
        };
        self.value(name, &Kind::U32(unit)).map(|_| ())
    }

    fn call(&self, id: &str) -> Check<&CallContract> {
        self.contract
            .calls
            .iter()
            .find(|call| call.id == id)
            .ok_or_else(|| format!("Unknown native call {id}"))
    }

    fn target(&self, target: &Target) -> Check<&InterfaceRecord> {
        let identity = match target {
            Target::Receiver => &self.contract.receiver,
            Target::AudioOrigin => {
                &self
                    .contract
                    .audio_origin
                    .as_ref()
                    .ok_or("Missing audio-origin capability")?
                    .interface
            }
            Target::Acquired(owner) => {
                let value = self
                    .values
                    .get(&owner.0)
                    .ok_or("Unavailable acquired owner")?;
                let Kind::Owner(identity) = &value.kind else {
                    return Err("Acquired target is not an owned-interface output".into());
                };
                require(
                    value
                        .producer
                        .as_ref()
                        .is_some_and(|p| p.phase == Phase::Acquire),
                    "Owner did not originate at acquisition",
                )?;
                identity
            }
        };
        self.registry.interface(identity)
    }

    fn abi(&self, call: &CallContract) -> Check<&[AbiCell]> {
        let abi = self
            .registry
            .evidence(&call.evidence)?
            .abi
            .as_deref()
            .ok_or("Evidence has no completed copy ABI")?;
        require(
            abi.len() == call.bindings.len()
                && abi
                    .iter()
                    .zip(&call.bindings)
                    .all(|(cell, binding)| cell.storage() == binding.storage()),
            "Native argument/output storage does not match registry ABI cells",
        )?;
        for (cell, binding) in abi.iter().zip(&call.bindings) {
            let matches = match cell {
                AbiCell::InU32(unit) => {
                    matches!(binding, Binding::InU32 { value } if value.unit().is_none_or(|source| source == *unit))
                }
                AbiCell::OutU32(unit) => {
                    matches!(binding, Binding::OutU32 { unit: output, .. } if unit == output)
                }
                AbiCell::InRect
                | AbiCell::OutU64
                | AbiCell::OutGuid
                | AbiCell::BorrowedBytes
                | AbiCell::OwnedInterface => true,
            };
            require(matches, "Wrong output/reference type for native ABI cell")?;
        }
        Ok(abi)
    }

    fn native_call(&mut self, id: &str, phase: Phase) -> Check {
        require(
            self.called.insert(id.into()),
            "Native call repeated across lifecycle phases",
        )?;
        let call = self.call(id)?.clone();
        let evidence = self.registry.evidence(&call.evidence)?;
        require(
            (phase == Phase::Acquire) == evidence.acquisition_cleanup.is_some(),
            "Acquisition evidence must be used in its acquisition phase",
        )?;
        if matches!(phase, Phase::Before | Phase::After) {
            let mut lifecycle = vec![self.operation.acquire.as_str()];
            lifecycle.extend(self.operation.commit.iter().map(String::as_str));
            if let Cleanup::Call { call, .. } = &self.operation.cleanup {
                lifecycle.push(call);
            }
            for id in lifecycle {
                let reserved = self.registry.evidence(&self.call(id)?.evidence)?;
                require(
                    evidence.iid != reserved.iid || evidence.slot != reserved.slot,
                    "A lifecycle method cannot be reused as a query under another binding",
                )?;
            }
        }
        require(
            guid(&evidence.iid)
                && evidence.slot >= 3
                && evidence.slot < 256
                && evidence.fingerprint.len() == 64
                && evidence.fingerprint.bytes().all(|c| c.is_ascii_hexdigit())
                && !evidence.citation.is_empty(),
            "Incomplete native registry evidence",
        )?;
        let target = self.target(&call.target)?;
        require(
            target.iid == evidence.iid || target.base_iids.contains(&evidence.iid),
            "Native call IID is incompatible with its owner/receiver",
        )?;
        self.abi(&call)?;
        // Resolve every input against the old environment. An earlier output
        // argument in this same native call is not an initialized input source.
        for binding in &call.bindings {
            match binding {
                Binding::InU32 { value } => {
                    self.source(value)?;
                    if let U32Source::SilentFlag(flag) = value {
                        require(
                            phase == Phase::Cleanup
                                && *flag != 0
                                && matches!(self.operation.transfer, Transfer::NoPayload { .. }),
                            "Silent flags are eligible only for no-payload normal finalization",
                        )?;
                    }
                }
                Binding::InRect { value } => {
                    self.value(&value.0, &Kind::Rect)?;
                    require(
                        phase == Phase::Acquire && self.bounded_rects.contains(&value.0),
                        "Rectangle acquisition lacks prior native bounds",
                    )?;
                }
                _ => {}
            }
        }
        let outputs = call
            .bindings
            .iter()
            .filter(|b| b.output_name().is_some())
            .count();
        require(
            match phase {
                Phase::Before | Phase::After => outputs != 0,
                Phase::Acquire => outputs != 0 && call.target == Target::Receiver,
                Phase::Commit | Phase::Cleanup => {
                    outputs == 0
                        && call
                            .bindings
                            .iter()
                            .all(|b| matches!(b, Binding::InU32 { .. }))
                }
            },
            "Native call is incompatible with its lifecycle phase",
        )?;
        for binding in &call.bindings {
            let kind = match binding {
                Binding::InU32 { .. } | Binding::InRect { .. } => continue,
                Binding::OutU32 { unit, .. } => Kind::U32(*unit),
                Binding::OutU64 { .. } => Kind::Timestamp,
                Binding::OutGuid { .. } => Kind::Guid,
                Binding::BorrowedBytes { .. } => {
                    require(
                        phase == Phase::Acquire || phase == Phase::After,
                        "Borrowed bytes have no active acquisition",
                    )?;
                    match &self.operation.cleanup {
                        Cleanup::Call { .. } => require(
                            call.target == Target::Receiver,
                            "Borrowed bytes are not covered by paired receiver cleanup",
                        )?,
                        Cleanup::ReleaseOwner { owner } => require(
                            call.target == Target::Acquired(owner.clone()) && phase == Phase::After,
                            "Borrowed bytes outlive or do not belong to the acquired owner",
                        )?,
                    }
                    Kind::Bytes
                }
                Binding::OwnedInterface { interface, name } => {
                    require(
                        phase == Phase::Acquire
                            && self.operation.cleanup
                                == (Cleanup::ReleaseOwner {
                                    owner: OwnerRef(name.clone()),
                                })
                            && self.contract.dependencies.contains(interface),
                        "Owned output is not the unique acquired cleanup owner",
                    )?;
                    self.registry.interface(interface)?;
                    Kind::Owner(interface.clone())
                }
            };
            self.define(
                binding.output_name().unwrap(),
                kind,
                Some(Producer {
                    call: id.into(),
                    target: call.target.clone(),
                    phase,
                }),
            )?;
        }
        Ok(())
    }

    fn extent(&mut self, name: &ExtentRef, proof: ExtentProof) -> Check {
        require(
            !self.values.contains_key(&name.0) && !self.extents.contains_key(&name.0),
            "Duplicate extent binding",
        )?;
        self.extents.insert(name.0.clone(), proof);
        Ok(())
    }

    fn steps(&mut self, steps: &[Step], phase: Phase) -> Check {
        for step in steps {
            match step {
                Step::Call { call } => self.native_call(call, phase)?,
                Step::InputFrames {
                    frames,
                    layout,
                    source,
                } => {
                    require(
                        phase == Phase::Before,
                        "Input layout must be proven before acquisition",
                    )?;
                    self.value(&layout.0, &Kind::Layout)?;
                    require(
                        matches!(
                            (source, self.operation.arguments),
                            (FrameInput::Bytes, InputKind::Bytes)
                                | (FrameInput::Frames, InputKind::Frames)
                        ),
                        "Frame input source does not match operation input",
                    )?;
                    self.define(&frames.0, Kind::U32(Unit::Frames), None)?;
                }
                Step::ReturnIfZero { frames } => {
                    require(
                        phase == Phase::Before && self.operation.result == ResultMapping::Void,
                        "Early return cannot skip acquired cleanup or result producers",
                    )?;
                    require(
                        self.value(&frames.0, &Kind::U32(Unit::Frames))?
                            .producer
                            .is_none(),
                        "Zero-request shortcut requires input frames",
                    )?;
                }
                Step::FrameExtent {
                    extent,
                    frames,
                    capacity,
                    layout,
                    nonzero,
                } => {
                    self.value(&layout.0, &Kind::Layout)?;
                    let from_input = self
                        .value(&frames.0, &Kind::U32(Unit::Frames))?
                        .producer
                        .is_none();
                    let capacity_source = self.native(&capacity.0, &Kind::U32(Unit::Frames))?;
                    require(
                        capacity_source.target == Target::AudioOrigin
                            && capacity_source.phase == Phase::Before
                            && *nonzero,
                        "Frame bound requires native owner capacity and a nonzero full packet",
                    )?;
                    if !from_input {
                        require(
                            self.native(&frames.0, &Kind::U32(Unit::Frames))?.phase
                                == Phase::Acquire,
                            "Packet frames must originate in acquisition",
                        )?;
                    }
                    self.extent(
                        extent,
                        ExtentProof::Frames {
                            frames: frames.clone(),
                            from_input,
                        },
                    )?;
                }
                Step::NativeExtent {
                    extent,
                    current,
                    capacity,
                    length,
                } => {
                    require(
                        phase == Phase::After,
                        "Native byte bounds require acquisition",
                    )?;
                    let current_source = self.native(&current.0, &Kind::U32(Unit::Bytes))?.clone();
                    require(
                        current_source == *self.native(&capacity.0, &Kind::U32(Unit::Bytes))?
                            && current_source.phase == Phase::Acquire,
                        "Current/capacity must be initialized outputs of the same acquisition",
                    )?;
                    self.value(&length.0, &Kind::U32(Unit::Bytes))?;
                    self.extent(
                        extent,
                        ExtentProof::Linear {
                            current: current.clone(),
                            length: length.clone(),
                            producer: current_source,
                        },
                    )?;
                }
                Step::RectBounds {
                    rect,
                    width,
                    height,
                } => {
                    require(
                        phase == Phase::Before,
                        "Rectangle bounds must precede acquisition",
                    )?;
                    self.value(&rect.0, &Kind::Rect)?;
                    let w = self.native(&width.0, &Kind::U32(Unit::Pixels))?;
                    require(
                        w.target == Target::Receiver
                            && w.phase == Phase::Before
                            && w == self.native(&height.0, &Kind::U32(Unit::Pixels))?,
                        "Rectangle bound must use native receiver dimensions",
                    )?;
                    self.bounded_rects.insert(rect.0.clone());
                }
                Step::RowExtent {
                    extent,
                    rect,
                    width,
                    height,
                    stride,
                    count,
                    format,
                    format_guid,
                    pixel_bytes,
                } => {
                    require(
                        phase == Phase::After
                            && self.bounded_rects.contains(&rect.0)
                            && *pixel_bytes > 0
                            && *pixel_bytes <= 16
                            && guid(format_guid),
                        "Unsupported row-layout or rectangle proof",
                    )?;
                    let source = self.native(&count.0, &Kind::U32(Unit::Bytes))?.clone();
                    require(
                        matches!(source.target, Target::Acquired(_))
                            && source.phase == Phase::After,
                        "Row count requires a live acquired owner",
                    )?;
                    for (name, kind) in [
                        (&width.0, Kind::U32(Unit::Pixels)),
                        (&height.0, Kind::U32(Unit::Pixels)),
                        (&stride.0, Kind::U32(Unit::Stride)),
                        (&format.0, Kind::Guid),
                    ] {
                        let p = self.native(name, &kind)?;
                        require(
                            p.target == source.target && p.phase == Phase::After,
                            "Row layout sources have different owner lifetimes",
                        )?;
                    }
                    self.extent(
                        extent,
                        ExtentProof::Rows {
                            width: width.clone(),
                            height: height.clone(),
                            producer: source,
                        },
                    )?;
                }
            }
        }
        Ok(())
    }

    fn validate(&mut self) -> Check {
        let op = self.operation;
        require(
            op.before.len() + op.after.len() <= 24,
            "Unsupported recipe length",
        )?;
        match op.arguments {
            InputKind::Bytes => self.define("input.bytes", Kind::U32(Unit::Bytes), None)?,
            InputKind::Frames => self.define("input.frames", Kind::U32(Unit::Frames), None)?,
            InputKind::Rectangle => self.define("input.rect", Kind::Rect, None)?,
            InputKind::None => {}
        }
        if let Some(origin) = &self.contract.audio_origin {
            self.define(&origin.layout.0, Kind::Layout, None)?;
        }
        self.steps(&op.before, Phase::Before)?;
        let acquisition = self.registry.evidence(&self.call(&op.acquire)?.evidence)?;
        // Abort must be constructible before native acquisition, even if
        // acquisition's pointer/count outputs turn out invalid or are unreadable.
        match &op.cleanup {
            Cleanup::Call { call, abort } => {
                let finalize = self.call(call)?;
                require(
                    matches!(
                        &acquisition.acquisition_cleanup,
                        Some(AcquisitionCleanup::Call { evidence }) if evidence == &finalize.evidence
                    ),
                    "Cleanup does not close the reviewed acquisition",
                )?;
                self.target(&finalize.target)?;
                require(
                    finalize.target == Target::Receiver
                        && finalize.bindings.len() == abort.len()
                        && finalize
                            .bindings
                            .iter()
                            .all(|b| matches!(b, Binding::InU32 { .. }))
                        && self.contract.owner != BorrowOwner::AcquiredInterface,
                    "Invalid paired finalization target/storage/owner",
                )?;
                for (arg, cell) in abort.iter().zip(self.abi(finalize)?) {
                    require(
                        !matches!(arg, U32Source::SilentFlag(_)),
                        "Abort cannot commit a silent payload",
                    )?;
                    self.source(arg)?;
                    require(
                        matches!(cell, AbiCell::InU32(unit) if arg.unit().is_none_or(|source| source == *unit)),
                        "Wrong output/reference type for native abort ABI cell",
                    )?;
                }
            }
            Cleanup::ReleaseOwner { .. } => require(
                acquisition.acquisition_cleanup == Some(AcquisitionCleanup::ReleaseOwner)
                    && self.contract.owner == BorrowOwner::AcquiredInterface
                    && self.contract.empty_hresult.is_none(),
                "Owned acquisition cannot use a paired or empty-packet lifecycle",
            )?,
        }
        self.native_call(&op.acquire, Phase::Acquire)?;
        if let Cleanup::ReleaseOwner { owner } = &op.cleanup {
            self.target(&Target::Acquired(owner.clone()))?;
        }
        self.steps(&op.after, Phase::After)?;
        require(
            self.extents.len() == 1,
            "Exactly one proven copy extent is required",
        )?;
        let (extent, pointer, silent, write) = match &op.transfer {
            Transfer::Read {
                extent,
                pointer,
                silent,
            } => (extent, Some(pointer), silent.as_ref(), false),
            Transfer::Write { extent, pointer } => (extent, Some(pointer), None, true),
            Transfer::NoPayload { extent } => (extent, None, None, true),
        };
        require(
            if write {
                self.contract.access != MemoryAccess::Read
            } else {
                self.contract.access != MemoryAccess::Write
            },
            "Copy violates native access policy",
        )?;
        let proof = self
            .extents
            .get(&extent.0)
            .ok_or("Uninitialized copy extent reference")?
            .clone();
        let pointer_source = pointer
            .map(|p| self.native(&p.0, &Kind::Bytes).cloned())
            .transpose()?;
        match &proof {
            ExtentProof::Linear {
                current,
                length,
                producer,
            } => {
                require(
                    pointer_source.as_ref() == Some(producer),
                    "Pointer and byte bound have different producers",
                )?;
                require(
                    if write {
                        length.0 == "input.bytes" && op.arguments == InputKind::Bytes
                    } else {
                        length == current
                    },
                    "Copy length is not native current length or staged input bytes",
                )?;
                require(
                    silent.is_none(),
                    "Linear byte copies cannot skip pointer validation",
                )?;
            }
            ExtentProof::Frames { frames, from_input } => {
                require(
                    *from_input == write,
                    "Frame transfer direction/source mismatch",
                )?;
                require(
                    self.contract.audio_origin.as_ref().is_some_and(|origin| {
                        origin.role
                            == if write {
                                AudioRole::Render
                            } else {
                                AudioRole::Capture
                            }
                    }),
                    "Frame access requires the corresponding observed audio service role",
                )?;
                if write {
                    if let Some(pointer) = &pointer_source {
                        require(
                            pointer.phase == Phase::Acquire && pointer.call == op.acquire,
                            "Frame writes require the pointer returned for the acquired request",
                        )?;
                    }
                    require(
                        self.call(&op.acquire)?
                            .bindings
                            .iter()
                            .filter(|b| {
                                matches!(b,
                        Binding::InU32 { value: U32Source::Frames(r) } if r == frames)
                            })
                            .count()
                            == 1,
                        "Acquisition must request the exact proven input frames",
                    )?;
                } else {
                    require(
                        pointer_source.as_ref()
                            == Some(self.native(&frames.0, &Kind::U32(Unit::Frames))?),
                        "Packet pointer and frames have different acquisition producers",
                    )?;
                }
                let Cleanup::Call { call, abort } = &op.cleanup else {
                    return Err("Frame copies require full/zero paired cleanup".into());
                };
                let finalize = self.call(call)?;
                let mut full = 0;
                let mut silent = 0;
                for ((binding, abort), cell) in
                    finalize.bindings.iter().zip(abort).zip(self.abi(finalize)?)
                {
                    if matches!(cell, AbiCell::InU32(Unit::Frames)) {
                        let Binding::InU32 {
                            value: U32Source::Frames(r),
                        } = binding
                        else {
                            return Err("Missing full/zero frame finalization dependency".into());
                        };
                        require(
                            r == frames && abort == &U32Source::Constant(0),
                            "Frame finalization must use full proven frames or zero on abort",
                        )?;
                        full += 1;
                    }
                    if let Binding::InU32 {
                        value: U32Source::SilentFlag(flag),
                    } = binding
                    {
                        require(
                            *flag != 0 && abort == &U32Source::Constant(0),
                            "Silence commit needs its reviewed flag and zero flags on abort",
                        )?;
                        silent += 1;
                    }
                }
                require(full == 1, "Missing full/zero frame finalization dependency")?;
                require(
                    silent == usize::from(matches!(op.transfer, Transfer::NoPayload { .. })),
                    "No-payload transfer requires exactly one typed silent-flag finalization binding",
                )?;
            }
            ExtentProof::Rows { producer, .. } => {
                require(
                    !write
                        && silent.is_none()
                        && pointer_source.as_ref() == Some(producer)
                        && self.contract.sta_only,
                    "Row copy access, owner, count or apartment mismatch",
                )?;
            }
        }
        if let Some(silent) = silent {
            require(silent.mask != 0, "Empty silent-packet mask")?;
            self.native(&silent.flags.0, &Kind::U32(Unit::Flags))?;
        }
        if write && matches!(proof, ExtentProof::Linear { .. }) {
            require(
                op.commit.len() == 1,
                "Byte replacement requires its native length commit",
            )?;
            let commit = self.call(&op.commit[0])?;
            require(
                commit.target == Target::Receiver
                    && commit.bindings.iter().any(|b| {
                        matches!(b,
                Binding::InU32 { value: U32Source::Bytes(r) } if r.0 == "input.bytes")
                    }),
                "Native length commit must consume the bounded staged input length",
            )?;
        } else {
            require(
                op.commit.is_empty(),
                "Unsupported commit after this transfer",
            )?;
        }
        for call in &op.commit {
            self.native_call(call, Phase::Commit)?;
        }
        if let Cleanup::Call { call, abort } = &op.cleanup {
            if !matches!(proof, ExtentProof::Frames { .. }) {
                require(
                    self.call(call)?.bindings.iter().zip(abort).all(
                        |(arg, abort)| matches!(arg, Binding::InU32 { value } if value == abort),
                    ),
                    "Non-frame cleanup must have identical normal/abort arguments",
                )?;
            }
            self.native_call(call, Phase::Cleanup)?;
        }
        match &op.result {
            ResultMapping::Void => require(write, "Read result cannot discard required bytes")?,
            ResultMapping::Bytes => {
                require(!write && silent.is_none(), "Invalid owned-byte result")?
            }
            ResultMapping::Packet {
                frames,
                flags,
                device,
                qpc,
                timestamp_error_mask,
            } => {
                require(
                    !write
                        && matches!(&proof, ExtentProof::Frames { frames: f, .. } if f == frames)
                        && silent.is_some_and(|s| s.flags == *flags)
                        && *timestamp_error_mask != 0,
                    "Packet result does not match its frame/flag transfer",
                )?;
                for (name, kind) in [
                    (&flags.0, Kind::U32(Unit::Flags)),
                    (&device.0, Kind::Timestamp),
                    (&qpc.0, Kind::Timestamp),
                ] {
                    require(
                        Some(self.native(name, &kind)?) == pointer_source.as_ref(),
                        "Packet result sources do not share the acquisition",
                    )?;
                }
            }
            ResultMapping::Bitmap { width, height } => require(
                matches!(&proof, ExtentProof::Rows { width: w, height: h, .. } if width == w && height == h),
                "Bitmap result does not match bounded native rows",
            )?,
        }
        require(
            self.contract.empty_hresult.is_none()
                || matches!(op.result, ResultMapping::Packet { .. }),
            "Empty acquisition needs a nullable packet result",
        )?;
        let shape = match op.api {
            OperationId::WriteFramesCopy => {
                op.arguments == InputKind::Bytes
                    && matches!(proof, ExtentProof::Frames { .. })
                    && matches!(op.transfer, Transfer::Write { .. })
            }
            OperationId::WriteSilence => {
                op.arguments == InputKind::Frames
                    && matches!(proof, ExtentProof::Frames { .. })
                    && matches!(op.transfer, Transfer::NoPayload { .. })
            }
            OperationId::ReadPacketCopy => {
                op.arguments == InputKind::None && matches!(op.result, ResultMapping::Packet { .. })
            }
            OperationId::ReadLockedBgra8Copy => {
                op.arguments == InputKind::Rectangle
                    && matches!(op.result, ResultMapping::Bitmap { .. })
            }
            OperationId::ReadCopy => {
                op.arguments == InputKind::None && op.result == ResultMapping::Bytes
            }
            OperationId::ReplaceCopy => {
                op.arguments == InputKind::Bytes
                    && matches!(proof, ExtentProof::Linear { .. })
                    && matches!(op.transfer, Transfer::Write { .. })
            }
        };
        require(
            shape,
            "Operation input/result shape does not match its public API",
        )
    }
}
