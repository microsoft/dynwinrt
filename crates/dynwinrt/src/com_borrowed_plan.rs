// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Executes only the typed, admitted copy recipes. No interface-family dispatch.

use super::*;
use contracts::{
    AudioOrigin, Binding, Cleanup, FrameInput, InputKind, Operation, OperationId, Registry,
    ResultMapping, Step, Target, Transfer, U32Source,
};

/// Completed immutable MethodHandles and a validated, closed execution recipe.
/// No native borrow or caller-provided extent is retained in this plan.
pub struct BorrowedCopyPlan {
    contract: BorrowedCopyContract,
    receiver: GUID,
    calls: HashMap<String, PreparedCall>,
}

struct PreparedCall {
    contract: CallContract,
    iid: GUID,
    method: MethodHandle,
}

enum Datum {
    U32(u32),
    U64(u64),
    Guid(GUID),
    Bytes(*mut u8),
    Owner(*mut c_void),
    Rect([i32; 4]),
    Layout(usize),
}

#[derive(Default)]
struct Values {
    data: HashMap<String, Datum>,
    extents: HashMap<String, CopyExtent>,
}

impl Values {
    fn get(&self, name: &str) -> result::Result<&Datum> {
        self.data
            .get(name)
            .ok_or_else(|| error("Uninitialized copy-plan value"))
    }
    fn u32(&self, name: &str) -> result::Result<u32> {
        match self.get(name)? {
            Datum::U32(v) => Ok(*v),
            _ => Err(error("Copy-plan u32 type mismatch")),
        }
    }
    fn u64(&self, name: &str) -> result::Result<u64> {
        match self.get(name)? {
            Datum::U64(v) => Ok(*v),
            _ => Err(error("Copy-plan timestamp type mismatch")),
        }
    }
    fn layout(&self, name: &str) -> result::Result<usize> {
        match self.get(name)? {
            Datum::Layout(v) => Ok(*v),
            _ => Err(error("Missing observed initialized layout")),
        }
    }
    fn rect(&self, name: &str) -> result::Result<[i32; 4]> {
        match self.get(name)? {
            Datum::Rect(v) => Ok(*v),
            _ => Err(error("Copy-plan rectangle type mismatch")),
        }
    }
    fn pointer(&self, name: &str) -> result::Result<*mut u8> {
        match self.get(name)? {
            Datum::Bytes(v) => Ok(*v),
            _ => Err(error("Copy-plan byte pointer type mismatch")),
        }
    }
    fn source(&self, source: &U32Source) -> result::Result<u32> {
        match source {
            U32Source::Constant(v) | U32Source::SilentFlag(v) => Ok(*v),
            U32Source::Bytes(r) => self.u32(&r.0),
            U32Source::Frames(r) => self.u32(&r.0),
            U32Source::Flags(r) => self.u32(&r.0),
            U32Source::Pixels(r) => self.u32(&r.0),
        }
    }
}

// Native output cells are never values until the exact success condition has
// been classified. Owned outputs alone carry RAII, including native failure.
enum NativeCell {
    U32(u32),
    U64(u64),
    Guid(GUID),
    Bytes(*mut u8),
    Owner(OwnedInterfaceCell),
}

impl NativeCell {
    fn address(&mut self) -> WinRTValue {
        match self {
            Self::U32(v) => cell(v),
            Self::U64(v) => cell(v),
            Self::Guid(v) => cell(v),
            Self::Bytes(v) => cell(v),
            Self::Owner(v) => cell(&mut v.0),
        }
    }
}

struct Outputs(Vec<(String, Option<NativeCell>)>);

impl Outputs {
    fn for_call(call: &CallContract) -> Self {
        Self(
            call.bindings
                .iter()
                .filter_map(|binding| {
                    let value = match binding {
                        Binding::InU32 { .. } | Binding::InRect { .. } => return None,
                        Binding::OutU32 { .. } => NativeCell::U32(0),
                        Binding::OutU64 { .. } => NativeCell::U64(0),
                        Binding::OutGuid { .. } => NativeCell::Guid(GUID::zeroed()),
                        Binding::BorrowedBytes { .. } => NativeCell::Bytes(ptr::null_mut()),
                        Binding::OwnedInterface { .. } => {
                            NativeCell::Owner(OwnedInterfaceCell::default())
                        }
                    };
                    Some((binding.output_name().unwrap().into(), Some(value)))
                })
                .collect(),
        )
    }

    fn address(&mut self, name: &str) -> result::Result<WinRTValue> {
        self.0
            .iter_mut()
            .find(|(id, _)| id == name)
            .and_then(|(_, value)| value.as_mut())
            .map(NativeCell::address)
            .ok_or_else(|| error("Missing native output binding"))
    }

    fn take_owner(&mut self, name: &str) -> result::Result<OwnedInterfaceCell> {
        let value = self
            .0
            .iter_mut()
            .find(|(id, _)| id == name)
            .and_then(|(_, value)| value.take())
            .ok_or_else(|| error("Missing acquired owner binding"))?;
        match value {
            NativeCell::Owner(owner) => Ok(owner),
            _ => Err(error("Acquired cleanup source is not an owned interface")),
        }
    }

    fn publish(self, values: &mut Values) -> result::Result<()> {
        for (name, value) in self.0 {
            let value = match value {
                None => continue, // The transaction already took its unique owner.
                Some(NativeCell::U32(v)) => Datum::U32(v),
                Some(NativeCell::U64(v)) => Datum::U64(v),
                Some(NativeCell::Guid(v)) => Datum::Guid(v),
                Some(NativeCell::Bytes(v)) => Datum::Bytes(v),
                Some(NativeCell::Owner(_)) => return Err(error("Unarmed acquired owner")),
            };
            values.data.insert(name, value);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Input<'a> {
    None,
    Bytes(&'a [u8]),
    Frames(u32),
    Rect([i32; 4]),
}

impl Input<'_> {
    fn kind(self) -> InputKind {
        match self {
            Self::None => InputKind::None,
            Self::Bytes(_) => InputKind::Bytes,
            Self::Frames(_) => InputKind::Frames,
            Self::Rect(_) => InputKind::Rectangle,
        }
    }
}

struct Run<'a> {
    input: Input<'a>,
    receiver: IUnknown,
    origin: Option<AudioOwner>,
    values: Values,
}

struct CopyExtent {
    length: usize,
    span: usize,
    row: usize,
    stride: usize,
    rows: usize,
}

impl CopyExtent {
    fn linear(bytes: usize) -> result::Result<Self> {
        checked_extent(bytes)?;
        Ok(Self {
            length: bytes,
            span: bytes,
            row: bytes,
            stride: bytes,
            rows: 1,
        })
    }

    fn read(&self, pointer: *const u8) -> result::Result<Vec<u8>> {
        if self.rows == 1 {
            return copy_bytes(pointer, self.length);
        }
        validate_pointer(pointer, self.span)?;
        let mut data = allocate(self.length)?;
        for (index, target) in data.chunks_exact_mut(self.row).enumerate() {
            unsafe {
                ptr::copy_nonoverlapping(
                    pointer.add(index * self.stride),
                    target.as_mut_ptr(),
                    self.row,
                )
            };
        }
        Ok(data)
    }

    fn write(&self, pointer: *mut u8, data: &[u8]) -> result::Result<()> {
        if self.rows != 1 || data.len() != self.length {
            return Err(error("Copy input does not match its proven native extent"));
        }
        validate_pointer(pointer, self.span)?;
        if !data.is_empty() {
            unsafe { ptr::copy_nonoverlapping(data.as_ptr(), pointer, data.len()) };
        }
        Ok(())
    }
}

enum CopyOutput {
    Void,
    Bytes(Vec<u8>),
    Packet(Option<CapturePacket>),
    Bitmap(BitmapCopy),
}

impl BorrowedCopyPlan {
    pub fn prepare(
        table: &Arc<MetadataTable>,
        contract: BorrowedCopyContract,
    ) -> result::Result<Self> {
        let registry = contracts::registry();
        registry.admit(&contract).map_err(error)?;
        Self::compile(table, contract, registry)
    }

    // No test-registry injection is compiled into production or test-hooks builds.
    #[cfg(test)]
    pub(super) fn prepare_test(
        table: &Arc<MetadataTable>,
        contract: BorrowedCopyContract,
        registry: &Registry,
    ) -> result::Result<Self> {
        registry.admit(&contract).map_err(error)?;
        Self::compile(table, contract, registry)
    }

    fn compile(
        table: &Arc<MetadataTable>,
        contract: BorrowedCopyContract,
        registry: &Registry,
    ) -> result::Result<Self> {
        let receiver = parse_iid(&registry.interface(&contract.receiver).map_err(error)?.iid)?;
        let mut calls = HashMap::new();
        for definition in &contract.calls {
            let evidence = registry.evidence(&definition.evidence).map_err(error)?;
            let iid = parse_iid(&evidence.iid)?;
            let mut signature = MethodSignature::new(table).preserve_hresult();
            for binding in &definition.bindings {
                signature = signature.add_in(match binding {
                    Binding::InU32 { .. } => Type::winrt(table.u32_type()),
                    Binding::InRect { .. }
                    | Binding::OutU32 { .. }
                    | Binding::OutU64 { .. }
                    | Binding::OutGuid { .. }
                    | Binding::BorrowedBytes { .. }
                    | Binding::OwnedInterface { .. } => Type::pointer(),
                });
            }
            let interface = super::super::register_interface(
                table,
                "BorrowedCopyPlan",
                iid,
                InterfaceBase::IUnknown,
            )
            .add_method_at(evidence.slot, &evidence.method, signature)?;
            calls.insert(
                definition.id.clone(),
                PreparedCall {
                    contract: definition.clone(),
                    iid,
                    method: interface
                        .method(evidence.slot)
                        .expect("completed copy method"),
                },
            );
        }
        Ok(Self {
            contract,
            receiver,
            calls,
        })
    }

    fn begin<'a>(
        &self,
        context: &Identity,
        object: &IUnknown,
        input: Input<'a>,
    ) -> result::Result<(Transaction, Run<'a>)> {
        context.ensure_idle()?;
        let mut values = Values::default();
        match input {
            Input::Bytes(bytes) => {
                checked_extent(bytes.len())?;
                values
                    .data
                    .insert("input.bytes".into(), Datum::U32(bytes.len() as u32));
            }
            Input::Frames(frames) => {
                values
                    .data
                    .insert("input.frames".into(), Datum::U32(frames));
            }
            Input::Rect(rect) => {
                rectangle_end(rect)?;
                values.data.insert("input.rect".into(), Datum::Rect(rect));
            }
            Input::None => {}
        }
        if self.contract.sta_only {
            let mut apartment = Default::default();
            let mut qualifier = Default::default();
            unsafe { CoGetApartmentType(&mut apartment, &mut qualifier)? };
            if apartment != APTTYPE_STA && apartment != APTTYPE_MAINSTA {
                return Err(error(
                    "WIC borrowed-copy GetDataPointer requires an STA apartment",
                ));
            }
        }
        let origin = self
            .contract
            .audio_origin
            .as_ref()
            .map(|policy| context.audio_owner(policy.role))
            .transpose()?;
        let transaction =
            Transaction::enter(context, origin.as_ref().map(|(identity, _)| identity), true)?;
        if !context.matches(&Identity::for_object(object)?) {
            return Err(error(
                "Borrowed-copy context does not belong to this COM identity",
            ));
        }
        let receiver = query(object, &self.receiver)?;
        if let (Some(policy), Some(origin)) = (&self.contract.audio_origin, &origin) {
            values.data.insert(
                policy.layout.0.clone(),
                Datum::Layout(audio_layout(origin, policy)?),
            );
        }
        Ok((
            transaction,
            Run {
                input,
                receiver,
                origin,
                values,
            },
        ))
    }

    fn prepared(&self, id: &str) -> result::Result<&PreparedCall> {
        self.calls
            .get(id)
            .ok_or_else(|| error("Unknown completed copy call"))
    }

    fn target(&self, prepared: &PreparedCall, run: &Run<'_>) -> result::Result<IUnknown> {
        let object = match &prepared.contract.target {
            Target::Receiver => &run.receiver,
            Target::AudioOrigin => {
                &run.origin
                    .as_ref()
                    .ok_or_else(|| error("Missing audio origin"))?
                    .1
            }
            Target::Acquired(owner) => {
                let Datum::Owner(raw) = run.values.get(&owner.0)? else {
                    return Err(error("Acquired target is not a live owner"));
                };
                unsafe { IUnknown::from_raw_borrowed(raw) }
                    .ok_or_else(|| error("Acquisition returned a required null owner"))?
            }
        };
        query(object, &prepared.iid)
    }

    fn invoke(&self, id: &str, run: &mut Run<'_>) -> result::Result<(HRESULT, Outputs)> {
        let prepared = self.prepared(id)?;
        let view = self.target(prepared, run)?;
        // Build the complete vector before taking addresses; its allocation is
        // stable through the synchronous libffi call, independent of binding order.
        let mut outputs = Outputs::for_call(&prepared.contract);
        let mut args = Vec::with_capacity(prepared.contract.bindings.len());
        for binding in &prepared.contract.bindings {
            args.push(match binding {
                Binding::InU32 { value } => WinRTValue::U32(run.values.source(value)?),
                Binding::InRect { value } => {
                    let Some(Datum::Rect(rect)) = run.values.data.get_mut(&value.0) else {
                        return Err(error("Uninitialized rectangle source"));
                    };
                    cell(rect)
                }
                _ => outputs.address(binding.output_name().unwrap())?,
            });
        }
        let hr = call(&prepared.method, &view, &args)?;
        Ok((hr, outputs))
    }

    fn exact_call(&self, id: &str, run: &mut Run<'_>) -> result::Result<()> {
        let (hr, outputs) = self.invoke(id, run)?;
        if hr.0 != 0 {
            return Err(error(format!(
                "Borrowed-copy call returned unexpected HRESULT 0x{:08X}",
                hr.0 as u32
            )));
        }
        outputs.publish(&mut run.values)
    }

    fn steps(&self, steps: &[Step], run: &mut Run<'_>) -> result::Result<bool> {
        for step in steps {
            match step {
                Step::Call { call } => self.exact_call(call, run)?,
                Step::InputFrames {
                    frames,
                    layout,
                    source,
                } => {
                    let block = run.values.layout(&layout.0)?;
                    let count = match (source, run.input) {
                        (FrameInput::Bytes, Input::Bytes(data)) => {
                            if data.len() % block != 0 {
                                return Err(error(
                                    "Audio data length must contain whole initialized-format frames",
                                ));
                            }
                            u32::try_from(data.len() / block)
                                .map_err(|_| error("Audio frame count overflow"))?
                        }
                        (FrameInput::Frames, Input::Frames(frames)) => frames,
                        _ => return Err(error("Invalid frame input source")),
                    };
                    frame_bytes(count, block)?;
                    run.values.data.insert(frames.0.clone(), Datum::U32(count));
                }
                Step::ReturnIfZero { frames } => {
                    if run.values.u32(&frames.0)? == 0 {
                        return Ok(false);
                    }
                }
                Step::FrameExtent {
                    extent,
                    frames,
                    capacity,
                    layout,
                    nonzero,
                } => {
                    let frames = run.values.u32(&frames.0)?;
                    if (*nonzero && frames == 0) || frames > run.values.u32(&capacity.0)? {
                        return Err(error("Audio frame count is outside native buffer capacity"));
                    }
                    let bytes = frame_bytes(frames, run.values.layout(&layout.0)?)?;
                    run.values
                        .extents
                        .insert(extent.0.clone(), CopyExtent::linear(bytes)?);
                }
                Step::NativeExtent {
                    extent,
                    current,
                    capacity,
                    length,
                } => {
                    let current = run.values.u32(&current.0)?;
                    let maximum = run.values.u32(&capacity.0)?;
                    if current > maximum {
                        return Err(error("Media buffer current length exceeds native maximum"));
                    }
                    let length = run.values.u32(&length.0)?;
                    if length > maximum {
                        return Err(error("Replacement exceeds native media-buffer capacity"));
                    }
                    run.values
                        .extents
                        .insert(extent.0.clone(), CopyExtent::linear(length as usize)?);
                }
                Step::RectBounds {
                    rect,
                    width,
                    height,
                } => {
                    let (right, bottom) = rectangle_end(run.values.rect(&rect.0)?)?;
                    if right > run.values.u32(&width.0)? || bottom > run.values.u32(&height.0)? {
                        return Err(error("Bitmap rectangle exceeds native dimensions"));
                    }
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
                    let expected = parse_iid(format_guid)?;
                    if !matches!(run.values.get(&format.0)?, Datum::Guid(value) if *value == expected)
                    {
                        return Err(error("Bitmap copy supports only native 32bpp BGRA"));
                    }
                    let rect = run.values.rect(&rect.0)?;
                    let width = run.values.u32(&width.0)?;
                    let height = run.values.u32(&height.0)?;
                    if width != rect[2] as u32 || height != rect[3] as u32 {
                        return Err(error(
                            "Bitmap lock dimensions differ from the requested rectangle",
                        ));
                    }
                    let stride = run.values.u32(&stride.0)? as usize;
                    let row = (width as usize)
                        .checked_mul(*pixel_bytes as usize)
                        .ok_or_else(|| error("Bitmap row overflow"))?;
                    let rows = height as usize;
                    let span = rows
                        .checked_sub(1)
                        .and_then(|n| n.checked_mul(stride))
                        .and_then(|n| n.checked_add(row))
                        .ok_or_else(|| error("Bitmap span overflow"))?;
                    if row == 0 || row > stride || span > run.values.u32(&count.0)? as usize {
                        return Err(error(
                            "Bitmap final-row span exceeds native byte count/stride",
                        ));
                    }
                    checked_extent(span)?;
                    let length = row
                        .checked_mul(rows)
                        .ok_or_else(|| error("Bitmap copy size overflow"))?;
                    checked_extent(length)?;
                    run.values.extents.insert(
                        extent.0.clone(),
                        CopyExtent {
                            length,
                            span,
                            row,
                            stride,
                            rows,
                        },
                    );
                }
            }
        }
        Ok(true)
    }

    fn transfer(&self, transfer: &Transfer, run: &Run<'_>) -> result::Result<Option<Vec<u8>>> {
        let extent = match transfer {
            Transfer::Read { extent, .. }
            | Transfer::Write { extent, .. }
            | Transfer::NoPayload { extent } => extent,
        };
        let extent = run
            .values
            .extents
            .get(&extent.0)
            .ok_or_else(|| error("Uninitialized native extent"))?;
        match transfer {
            Transfer::Read {
                pointer, silent, ..
            } => {
                if let Some(condition) = silent
                    && run.values.u32(&condition.flags.0)? & condition.mask != 0
                {
                    return Ok(None);
                }
                extent.read(run.values.pointer(&pointer.0)?).map(Some)
            }
            Transfer::Write { pointer, .. } => {
                let Input::Bytes(data) = run.input else {
                    return Err(error("Missing staged input bytes"));
                };
                extent.write(run.values.pointer(&pointer.0)?, data)?;
                Ok(None)
            }
            Transfer::NoPayload { .. } => Ok(None),
        }
    }

    fn output(
        &self,
        mapping: &ResultMapping,
        values: &Values,
        data: Option<Vec<u8>>,
    ) -> result::Result<CopyOutput> {
        let required_data =
            |data: Option<Vec<u8>>| data.ok_or_else(|| error("Missing owned-copy result"));
        Ok(match mapping {
            ResultMapping::Void => CopyOutput::Void,
            ResultMapping::Bytes => CopyOutput::Bytes(required_data(data)?),
            ResultMapping::Packet {
                frames,
                flags,
                device,
                qpc,
                timestamp_error_mask,
            } => {
                let flags = values.u32(&flags.0)?;
                let timestamps = flags & timestamp_error_mask == 0;
                CopyOutput::Packet(Some(CapturePacket {
                    data,
                    frames: values.u32(&frames.0)?,
                    flags,
                    device_position: if timestamps {
                        Some(values.u64(&device.0)?)
                    } else {
                        None
                    },
                    qpc_position: if timestamps {
                        Some(values.u64(&qpc.0)?)
                    } else {
                        None
                    },
                }))
            }
            ResultMapping::Bitmap { width, height } => CopyOutput::Bitmap(BitmapCopy {
                data: required_data(data)?,
                width: values.u32(&width.0)?,
                height: values.u32(&height.0)?,
            }),
        })
    }

    fn normal_args(
        &self,
        operation: &Operation,
        values: &Values,
    ) -> result::Result<Vec<WinRTValue>> {
        let Cleanup::Call { call, .. } = &operation.cleanup else {
            return Ok(vec![]);
        };
        self.prepared(call)?
            .contract
            .bindings
            .iter()
            .map(|binding| match binding {
                Binding::InU32 { value } => values.source(value).map(WinRTValue::U32),
                _ => Err(error("Invalid finalization input binding")),
            })
            .collect()
    }

    fn execute(
        &self,
        api: OperationId,
        context: &Identity,
        object: &IUnknown,
        input: Input<'_>,
    ) -> result::Result<CopyOutput> {
        let operation = self
            .contract
            .operations
            .iter()
            .find(|op| op.api == api && op.arguments == input.kind())
            .ok_or_else(|| error("Operation is not supported by this borrowed-copy plan"))?;
        let (mut transaction, mut run) = self.begin(context, object, input)?;
        if !self.steps(&operation.before, &mut run)? {
            return transaction.complete(Ok(CopyOutput::Void), &[]);
        }
        let paired = if let Cleanup::Call { call, abort } = &operation.cleanup {
            let prepared = self.prepared(call)?;
            Some(Finalizer::Call {
                method: prepared.method.clone(),
                view: self.target(prepared, &run)?,
                abort: abort
                    .iter()
                    .map(|v| run.values.source(v).map(WinRTValue::U32))
                    .collect::<result::Result<_>>()?,
            })
        } else {
            None
        };
        let (hr, mut outputs) = self.invoke(&operation.acquire, &mut run)?;
        if hr.0 != self.contract.acquired_hresult {
            if self.contract.empty_hresult == Some(hr.0) {
                // Neither publish/read native outputs nor release a packet.
                return transaction.complete(Ok(CopyOutput::Packet(None)), &[]);
            }
            transaction.poison();
            return Err(error(format!(
                "Unexpected acquisition HRESULT 0x{:08X}; context poisoned",
                hr.0 as u32
            )));
        }
        match &operation.cleanup {
            Cleanup::Call { .. } => transaction.acquired(paired.expect("prepared paired cleanup")),
            Cleanup::ReleaseOwner { owner } => {
                let acquired = outputs.take_owner(&owner.0)?;
                let raw = acquired.0;
                transaction.acquired(Finalizer::Release(acquired));
                run.values.data.insert(owner.0.clone(), Datum::Owner(raw));
            }
        }
        // Cleanup is now armed. All output validation, copies, allocations and
        // native length commits are inside this single transaction.
        let result = (|| {
            outputs.publish(&mut run.values)?;
            self.steps(&operation.after, &mut run)?;
            let data = self.transfer(&operation.transfer, &run)?;
            for call in &operation.commit {
                self.exact_call(call, &mut run)?;
            }
            let output = self.output(&operation.result, &run.values, data)?;
            Ok((output, self.normal_args(operation, &run.values)?))
        })();
        match result {
            Ok((output, commit)) => transaction.complete(Ok(output), &commit),
            Err(failure) => transaction.complete(Err(failure), &[]),
        }
    }

    pub fn write_frames_copy(
        &self,
        context: &Identity,
        object: &IUnknown,
        data: &[u8],
    ) -> result::Result<()> {
        self.execute(
            OperationId::WriteFramesCopy,
            context,
            object,
            Input::Bytes(data),
        )
        .and_then(void)
    }
    pub fn write_silence(
        &self,
        context: &Identity,
        object: &IUnknown,
        frames: u32,
    ) -> result::Result<()> {
        self.execute(
            OperationId::WriteSilence,
            context,
            object,
            Input::Frames(frames),
        )
        .and_then(void)
    }
    pub fn replace_copy(
        &self,
        context: &Identity,
        object: &IUnknown,
        data: &[u8],
    ) -> result::Result<()> {
        self.execute(
            OperationId::ReplaceCopy,
            context,
            object,
            Input::Bytes(data),
        )
        .and_then(void)
    }
    pub fn read_copy(&self, context: &Identity, object: &IUnknown) -> result::Result<Vec<u8>> {
        match self.execute(OperationId::ReadCopy, context, object, Input::None)? {
            CopyOutput::Bytes(data) => Ok(data),
            _ => Err(error("Owned-byte result shape mismatch")),
        }
    }
    pub fn read_packet_copy(
        &self,
        context: &Identity,
        object: &IUnknown,
    ) -> result::Result<Option<CapturePacket>> {
        match self.execute(OperationId::ReadPacketCopy, context, object, Input::None)? {
            CopyOutput::Packet(packet) => Ok(packet),
            _ => Err(error("Packet result shape mismatch")),
        }
    }
    pub fn read_locked_bgra8_copy(
        &self,
        context: &Identity,
        object: &IUnknown,
        rect: [i32; 4],
    ) -> result::Result<BitmapCopy> {
        match self.execute(
            OperationId::ReadLockedBgra8Copy,
            context,
            object,
            Input::Rect(rect),
        )? {
            CopyOutput::Bitmap(bitmap) => Ok(bitmap),
            _ => Err(error("Bitmap result shape mismatch")),
        }
    }
}

fn void(output: CopyOutput) -> result::Result<()> {
    match output {
        CopyOutput::Void => Ok(()),
        _ => Err(error("Void result shape mismatch")),
    }
}

fn parse_iid(iid: &str) -> result::Result<GUID> {
    GUID::try_from(iid).map_err(|_| error("Invalid reviewed copy IID/GUID"))
}

fn rectangle_end(rect: [i32; 4]) -> result::Result<(u32, u32)> {
    let [x, y, width, height] = rect;
    if x < 0 || y < 0 || width <= 0 || height <= 0 {
        return Err(error(
            "Bitmap rectangle must have nonnegative origin and positive dimensions",
        ));
    }
    let right = x
        .checked_add(width)
        .ok_or_else(|| error("Bitmap rectangle overflow"))?;
    let bottom = y
        .checked_add(height)
        .ok_or_else(|| error("Bitmap rectangle overflow"))?;
    Ok((right as u32, bottom as u32))
}

fn audio_layout(origin: &AudioOwner, policy: &AudioOrigin) -> result::Result<usize> {
    let layout = {
        let stored = origin.0.inner.initialized.borrow();
        stored.as_ref().map(|initialized| {
            (
                initialized.block_align,
                initialized.share_mode,
                initialized.flags,
                initialized.format.block_align() as usize,
            )
        })
    };
    let (block, share_mode, flags, native_block) =
        layout.ok_or_else(|| error("Audio client has no observed successful initialization"))?;
    let block = block.ok_or_else(|| {
        error("Initialized audio format is not a validated uncompressed frame layout")
    })?;
    if !(0..=1).contains(&share_mode) {
        return Err(error("Unsupported initialized audio share mode"));
    }
    if policy.reject_exclusive_event && share_mode == 1 && flags & EVENT_CALLBACK != 0 {
        return Err(error(
            "Exclusive event-driven render copy is unsupported: zero-frame abort is invalid",
        ));
    }
    debug_assert_eq!(block, native_block);
    Ok(block)
}
