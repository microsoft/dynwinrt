// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Closed COM copy recipes and their reviewed evidence. No native dependencies.
//! Deserializing or validating a recipe is not native-call authorization:
//! production admission additionally requires equality with a packaged record.

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[cfg(test)]
mod tests;
mod validate;

pub const DESCRIPTOR_VERSION: u32 = 2;
pub const METADATA_SHA256: &str =
    "B64EE4818A7ED9F9D135038D58C51BD08369184D4D5ED428F20E9DE55DF8121D";
pub const REGENERATE: &str = "Unsupported borrowed-copy descriptor/evidence; fully regenerate bindings with the current generator (no incremental migration)";

macro_rules! reference {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);
    )+};
}

reference!(
    ByteCountRef,
    FrameCountRef,
    PixelCountRef,
    StrideRef,
    FlagsRef,
    TimestampRef,
    GuidRef,
    BytesRef,
    OwnerRef,
    RectRef,
    ExtentRef,
    LayoutRef,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CopyKind {
    AudioRender,
    AudioCapture,
    BitmapBgra8,
    MediaBuffer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Storage {
    InU32,
    InRect,
    OutU32,
    OutU64,
    OutGuid,
    BorrowedBytes,
    OwnedInterface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Unit {
    Scalar,
    Bytes,
    Frames,
    Pixels,
    Stride,
    Flags,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum AbiCell {
    InU32(Unit),
    InRect,
    OutU32(Unit),
    OutU64,
    OutGuid,
    BorrowedBytes,
    OwnedInterface,
}

impl AbiCell {
    pub fn storage(&self) -> Storage {
        match self {
            Self::InU32(_) => Storage::InU32,
            Self::InRect => Storage::InRect,
            Self::OutU32(_) => Storage::OutU32,
            Self::OutU64 => Storage::OutU64,
            Self::OutGuid => Storage::OutGuid,
            Self::BorrowedBytes => Storage::BorrowedBytes,
            Self::OwnedInterface => Storage::OwnedInterface,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum U32Source {
    Constant(u32),
    SilentFlag(u32),
    Bytes(ByteCountRef),
    Frames(FrameCountRef),
    Flags(FlagsRef),
    Pixels(PixelCountRef),
}

impl U32Source {
    fn unit(&self) -> Option<Unit> {
        match self {
            Self::Constant(_) => None,
            Self::Bytes(_) => Some(Unit::Bytes),
            Self::Frames(_) => Some(Unit::Frames),
            Self::SilentFlag(_) | Self::Flags(_) => Some(Unit::Flags),
            Self::Pixels(_) => Some(Unit::Pixels),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "storage", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Binding {
    InU32 { value: U32Source },
    InRect { value: RectRef },
    OutU32 { name: String, unit: Unit },
    OutU64 { name: String },
    OutGuid { name: String },
    BorrowedBytes { name: String },
    OwnedInterface { name: String, interface: String },
}

impl Binding {
    pub fn storage(&self) -> Storage {
        match self {
            Self::InU32 { .. } => Storage::InU32,
            Self::InRect { .. } => Storage::InRect,
            Self::OutU32 { .. } => Storage::OutU32,
            Self::OutU64 { .. } => Storage::OutU64,
            Self::OutGuid { .. } => Storage::OutGuid,
            Self::BorrowedBytes { .. } => Storage::BorrowedBytes,
            Self::OwnedInterface { .. } => Storage::OwnedInterface,
        }
    }

    pub fn output_name(&self) -> Option<&str> {
        match self {
            Self::InU32 { .. } | Self::InRect { .. } => None,
            Self::OutU32 { name, .. }
            | Self::OutU64 { name }
            | Self::OutGuid { name }
            | Self::BorrowedBytes { name }
            | Self::OwnedInterface { name, .. } => Some(name),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum Target {
    Receiver,
    AudioOrigin,
    Acquired(OwnerRef),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallContract {
    pub id: String,
    pub evidence: String,
    pub target: Target,
    pub bindings: Vec<Binding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MemoryAccess {
    Read,
    Write,
    ReadWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BorrowOwner {
    Receiver,
    AudioServiceOrigin,
    AcquiredInterface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioRole {
    Render,
    Capture,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioOrigin {
    pub role: AudioRole,
    pub interface: String,
    pub layout: LayoutRef,
    pub reject_exclusive_event: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationId {
    WriteFramesCopy,
    WriteSilence,
    ReadPacketCopy,
    ReadLockedBgra8Copy,
    ReadCopy,
    ReplaceCopy,
}

impl OperationId {
    pub fn name(self) -> &'static str {
        match self {
            Self::WriteFramesCopy => "writeFramesCopy",
            Self::WriteSilence => "writeSilence",
            Self::ReadPacketCopy => "readPacketCopy",
            Self::ReadLockedBgra8Copy => "readLockedBgra8Copy",
            Self::ReadCopy => "readCopy",
            Self::ReplaceCopy => "replaceCopy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InputKind {
    None,
    Bytes,
    Frames,
    Rectangle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrameInput {
    Bytes,
    Frames,
}

/// No general arithmetic, arbitrary branches, pointer inputs or callbacks.
/// Each extent constructor proves one reviewed native memory relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Step {
    Call {
        call: String,
    },
    InputFrames {
        frames: FrameCountRef,
        layout: LayoutRef,
        source: FrameInput,
    },
    ReturnIfZero {
        frames: FrameCountRef,
    },
    FrameExtent {
        extent: ExtentRef,
        frames: FrameCountRef,
        capacity: FrameCountRef,
        layout: LayoutRef,
        nonzero: bool,
    },
    NativeExtent {
        extent: ExtentRef,
        current: ByteCountRef,
        capacity: ByteCountRef,
        length: ByteCountRef,
    },
    RectBounds {
        rect: RectRef,
        width: PixelCountRef,
        height: PixelCountRef,
    },
    RowExtent {
        extent: ExtentRef,
        rect: RectRef,
        width: PixelCountRef,
        height: PixelCountRef,
        stride: StrideRef,
        count: ByteCountRef,
        format: GuidRef,
        format_guid: String,
        pixel_bytes: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlagCondition {
    pub flags: FlagsRef,
    pub mask: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "direction", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Transfer {
    Read {
        pointer: BytesRef,
        extent: ExtentRef,
        silent: Option<FlagCondition>,
    },
    Write {
        pointer: BytesRef,
        extent: ExtentRef,
    },
    NoPayload {
        extent: ExtentRef,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Cleanup {
    Call { call: String, abort: Vec<U32Source> },
    ReleaseOwner { owner: OwnerRef },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ResultMapping {
    Void,
    Bytes,
    Packet {
        frames: FrameCountRef,
        flags: FlagsRef,
        device: TimestampRef,
        qpc: TimestampRef,
        timestamp_error_mask: u32,
    },
    Bitmap {
        width: PixelCountRef,
        height: PixelCountRef,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub api: OperationId,
    pub arguments: InputKind,
    pub before: Vec<Step>,
    pub acquire: String,
    pub after: Vec<Step>,
    pub transfer: Transfer,
    pub commit: Vec<String>,
    pub cleanup: Cleanup,
    pub result: ResultMapping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BorrowedCopyContract {
    pub version: u32,
    pub metadata_sha256: String,
    pub exposure: String,
    pub id: String,
    // A compatibility/display label, never an executor or model dispatch key.
    pub kind: CopyKind,
    pub receiver: String,
    pub copy_only: bool,
    pub dependencies: Vec<String>,
    pub access: MemoryAccess,
    pub owner: BorrowOwner,
    pub sta_only: bool,
    pub audio_origin: Option<AudioOrigin>,
    pub acquired_hresult: i32,
    pub empty_hresult: Option<i32>,
    pub calls: Vec<CallContract>,
    pub operations: Vec<Operation>,
}

impl BorrowedCopyContract {
    pub fn expected(kind: CopyKind) -> Self {
        registry()
            .copies
            .iter()
            .find(|copy| copy.kind == kind)
            .expect("packaged copy")
            .clone()
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        let header: serde_json::Value =
            serde_json::from_str(value).map_err(|e| format!("{REGENERATE}: {e}"))?;
        if header.get("version").and_then(serde_json::Value::as_u64)
            != Some(u64::from(DESCRIPTOR_VERSION))
        {
            return Err(REGENERATE.into());
        }
        let contract = serde_json::from_str(value).map_err(|e| format!("{REGENERATE}: {e}"))?;
        registry().admit(&contract)?;
        Ok(contract)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterfaceRecord {
    pub namespace: String,
    pub name: String,
    pub iid: String,
    pub bases: Vec<String>,
    pub base_iids: Vec<String>,
    pub own_start: usize,
    pub last: usize,
    pub context: bool,
    pub catalog: bool,
    pub required_evidence: Vec<usize>,
}

impl InterfaceRecord {
    pub fn identity(&self) -> String {
        format!("{}.{}", self.namespace, self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AcquisitionCleanup {
    Call { evidence: String },
    ReleaseOwner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub id: String,
    pub namespace: String,
    pub interface: String,
    pub iid: String,
    pub method: String,
    pub slot: usize,
    pub fingerprint: String,
    pub effect: Option<String>,
    pub citation: String,
    pub abi: Option<Vec<AbiCell>>,
    pub acquisition_cleanup: Option<AcquisitionCleanup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub interfaces: Vec<InterfaceRecord>,
    pub evidence: Vec<Evidence>,
    pub copies: Vec<BorrowedCopyContract>,
}

impl Registry {
    pub fn interface(&self, identity: &str) -> Result<&InterfaceRecord, String> {
        self.interfaces
            .iter()
            .find(|record| record.identity() == identity)
            .ok_or_else(|| format!("Unknown registry interface {identity}"))
    }

    pub fn select(&self, namespace: &str, name: &str, iid: &str) -> Option<&BorrowedCopyContract> {
        self.copies.iter().find(|copy| {
            self.interface(&copy.receiver).is_ok_and(|record| {
                record.namespace == namespace && record.name == name && record.iid == iid
            })
        })
    }

    pub fn evidence(&self, id: &str) -> Result<&Evidence, String> {
        self.evidence
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| format!("Unknown registry evidence {id}"))
    }

    pub fn admit(&self, contract: &BorrowedCopyContract) -> Result<(), String> {
        if !self
            .copies
            .iter()
            .any(|record| record.id == contract.id && record == contract)
        {
            return Err(REGENERATE.into());
        }
        self.validate(contract)
            .map_err(|e| format!("{REGENERATE}: {e}"))
    }
}

pub fn registry() -> &'static Registry {
    static REGISTRY: LazyLock<Registry> = LazyLock::new(|| {
        serde_json::from_str(include_str!("registry.json")).expect("packaged COM copy registry")
    });
    &REGISTRY
}
