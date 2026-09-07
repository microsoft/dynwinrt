// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use dynwinrt::com::borrowed::{
  BorrowOwner, BorrowedCopyContract, BorrowedCopyPlan, CallContract, ContextEffect, CopyKind,
  Extent, Finalization, MemoryAccess, Storage,
};
use napi::bindgen_prelude::{BigInt, Buffer, Unknown};
use napi_derive::napi;
use serde::Deserialize;
use windows::core::GUID;

use super::{com, DynWinRTValue, TABLE};

const METADATA_HASH: &str = "B64EE4818A7ED9F9D135038D58C51BD08369184D4D5ED428F20E9DE55DF8121D";

fn invalid(message: impl Into<String>) -> napi::Error {
  napi::Error::from_reason(message)
}

fn native_error(error: dynwinrt::Error) -> napi::Error {
  invalid(error.message())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectDescriptor {
  version: u32,
  effect: String,
  metadata_sha256: String,
  fingerprint: String,
}

pub(super) fn parse_effect(value: &str) -> napi::Result<ContextEffect> {
  let value: EffectDescriptor = serde_json::from_str(value)
    .map_err(|e| invalid(format!("Invalid audio context descriptor: {e}")))?;
  let (effect, fingerprint) = match value.effect.as_str() {
    "audio-initialize" => (
      ContextEffect::AudioInitialize,
      "7F14AD1E6E6E178A91283B68002988D362898FCAC97823B866D61FAFF5766039",
    ),
    "audio-initialize-shared" => (
      ContextEffect::AudioInitializeShared,
      "08879857019485B6813E45F4BEB2941E4D83ACF80CA55AA4538F50B08C1B9531",
    ),
    "audio-get-service" => (
      ContextEffect::AudioGetService,
      "6362D65B61F18446F4E990C84DA998C4A7DE08517C7405981DF823854227CA7E",
    ),
    _ => return Err(invalid("Unsupported audio context effect")),
  };
  if value.version != 1
    || value.metadata_sha256 != METADATA_HASH
    || value.fingerprint != fingerprint
  {
    return Err(invalid("Audio context evidence drift; regenerate bindings"));
  }
  Ok(effect)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
  version: u32,
  metadata_sha256: String,
  access: String,
  exposure: String,
  owner: String,
  acquired_hresult: i32,
  empty_hresult: Option<i32>,
  kind: String,
  receiver: String,
  acquire: CallDescriptor,
  queries: Vec<CallDescriptor>,
  finalize: Option<CallDescriptor>,
  extent: String,
  finalization: String,
  sta_only: bool,
  audio_origin_required: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallDescriptor {
  iid: String,
  slot: usize,
  storage: Vec<String>,
}

impl CallDescriptor {
  fn contract(self) -> napi::Result<CallContract> {
    Ok(CallContract {
      iid: GUID::try_from(self.iid.as_str()).map_err(|_| invalid("Invalid borrowed-copy IID"))?,
      slot: self.slot,
      storage: self
        .storage
        .iter()
        .map(|value| match value.as_str() {
          "in-u32" => Ok(Storage::InU32),
          "in-rect" => Ok(Storage::InRect),
          "out-u32" => Ok(Storage::OutU32),
          "out-u64" => Ok(Storage::OutU64),
          "out-guid" => Ok(Storage::OutGuid),
          "borrowed-bytes" => Ok(Storage::BorrowedBytes),
          "owned-interface" => Ok(Storage::OwnedInterface),
          _ => Err(invalid("Unsupported borrowed-copy native storage")),
        })
        .collect::<napi::Result<_>>()?,
    })
  }
}

fn parse_contract(value: &str) -> napi::Result<BorrowedCopyContract> {
  let value: Descriptor = serde_json::from_str(value)
    .map_err(|e| invalid(format!("Invalid borrowed-copy descriptor: {e}")))?;
  if value.version != 1
    || value.metadata_sha256 != METADATA_HASH
    || value.exposure != "owned-copy-only"
  {
    return Err(invalid(
      "Borrowed-copy descriptor version/evidence/access drift",
    ));
  }
  let kind = match value.kind.as_str() {
    "audio-render" => CopyKind::AudioRender,
    "audio-capture" => CopyKind::AudioCapture,
    "bitmap-bgra8" => CopyKind::BitmapBgra8,
    "media-buffer" => CopyKind::MediaBuffer,
    _ => return Err(invalid("Unsupported borrowed-copy kind")),
  };
  let extent = match value.extent.as_str() {
    "initialized-frames" => Extent::InitializedFrames,
    "capture-packet-frames" => Extent::CapturePacketFrames,
    "native-bgra8-rows" => Extent::NativeBgra8Rows,
    "native-current-and-max" => Extent::NativeCurrentAndMax,
    _ => return Err(invalid("Unsupported borrowed-copy native extent")),
  };
  let finalization = match value.finalization.as_str() {
    "render-full-or-zero" => Finalization::RenderFullOrZero,
    "capture-full-or-zero" => Finalization::CaptureFullOrZero,
    "release-lock-reference" => Finalization::ReleaseLockReference,
    "unlock" => Finalization::Unlock,
    _ => return Err(invalid("Unsupported borrowed-copy finalizer")),
  };
  let access = match value.access.as_str() {
    "read" => MemoryAccess::Read,
    "write" => MemoryAccess::Write,
    "read-write" => MemoryAccess::ReadWrite,
    _ => return Err(invalid("Unsupported borrowed-copy access")),
  };
  let owner = match value.owner.as_str() {
    "receiver" => BorrowOwner::Receiver,
    "audio-service-origin" => BorrowOwner::AudioServiceOrigin,
    "acquired-interface" => BorrowOwner::AcquiredInterface,
    _ => return Err(invalid("Unsupported borrowed-copy owner")),
  };
  Ok(BorrowedCopyContract {
    kind,
    extent,
    finalization,
    receiver: GUID::try_from(value.receiver.as_str())
      .map_err(|_| invalid("Invalid borrowed-copy receiver IID"))?,
    acquire: value.acquire.contract()?,
    queries: value
      .queries
      .into_iter()
      .map(CallDescriptor::contract)
      .collect::<napi::Result<_>>()?,
    finalize: value.finalize.map(CallDescriptor::contract).transpose()?,
    sta_only: value.sta_only,
    audio_origin_required: value.audio_origin_required,
    access,
    owner,
    acquired_hresult: value.acquired_hresult,
    empty_hresult: value.empty_hresult,
  })
}

#[napi]
pub struct DynComBorrowedCopyPlan(BorrowedCopyPlan);

#[napi(object)]
pub struct DynComCapturePacketCopy {
  pub kind: String,
  pub data: Option<Buffer>,
  pub frames: u32,
  pub flags: u32,
  pub timestamps_valid: bool,
  pub device_position: Option<BigInt>,
  pub qpc_position: Option<BigInt>,
}

#[napi(object)]
pub struct DynComBitmapCopy {
  pub data: Buffer,
  pub width: u32,
  pub height: u32,
}

#[napi]
impl DynComBorrowedCopyPlan {
  #[napi]
  pub fn prepare(descriptor: String) -> napi::Result<Self> {
    BorrowedCopyPlan::prepare(&TABLE, parse_contract(&descriptor)?)
      .map(Self)
      .map_err(native_error)
  }
}

#[cfg(feature = "test-hooks")]
#[napi]
pub struct DynComBorrowedCopyTestFixture {
  fixture: dynwinrt::com::borrowed::testing::CopyFixture,
  owner: std::thread::ThreadId,
}

#[cfg(feature = "test-hooks")]
impl DynComBorrowedCopyTestFixture {
  fn check(&self) -> napi::Result<()> {
    if self.owner != std::thread::current().id() {
      return Err(invalid("Borrowed-copy fixture used on a different thread"));
    }
    Ok(())
  }
}

#[cfg(feature = "test-hooks")]
#[napi]
impl DynComBorrowedCopyTestFixture {
  #[napi(constructor)]
  pub fn new() -> Self {
    Self {
      fixture: Default::default(),
      owner: std::thread::current().id(),
    }
  }

  #[napi]
  pub fn object(&self, kind: String) -> napi::Result<DynWinRTValue> {
    self.check()?;
    let object = self.fixture.object(&kind).map_err(native_error)?;
    let mut value = DynWinRTValue::new(dynwinrt::WinRTValue::Object(object));
    value.bind_current_com_apartment()?;
    Ok(value)
  }

  #[napi]
  pub fn configure(&self, key: String, value: u32) -> napi::Result<()> {
    self.check()?;
    self.fixture.configure(&key, value).map_err(native_error)
  }

  #[napi]
  pub fn events(&self) -> napi::Result<Vec<String>> {
    self.check()?;
    Ok(self.fixture.events())
  }

  #[napi]
  pub fn bytes(&self) -> napi::Result<Buffer> {
    self.check()?;
    Ok(self.fixture.bytes().into())
  }

  #[napi]
  pub fn release_owners(&mut self) -> napi::Result<()> {
    self.check()?;
    self.fixture.release_owners();
    Ok(())
  }

  #[napi]
  pub fn install_callback_probe(&self, object: &DynWinRTValue) -> napi::Result<()> {
    self.check()?;
    object.ensure_com_apartment()?;
    let object = object
      .0
      .as_object()
      .ok_or_else(|| invalid("Probe requires a managed COM sink"))?;
    self
      .fixture
      .install_callback_probe(object)
      .map_err(native_error)
  }
}

#[napi]
impl DynComBorrowedCopyPlan {
  #[napi]
  pub fn write_frames_copy(
    &self,
    object: &DynWinRTValue,
    #[napi(ts_arg_type = "Buffer | Uint8Array")] data: Unknown,
  ) -> napi::Result<()> {
    let bytes = com::stage_copy_bytes(data)?;
    let context = object.com_context()?;
    let view = object
      .0
      .as_object()
      .ok_or_else(|| invalid("COM object is released"))?
      .clone();
    self
      .0
      .write_frames_copy(&context, &view, &bytes)
      .map_err(native_error)
  }

  #[napi]
  pub fn write_silence(&self, object: &DynWinRTValue, frames: f64) -> napi::Result<()> {
    if !frames.is_finite() || frames < 0.0 || frames > u32::MAX as f64 || frames.fract() != 0.0 {
      return Err(invalid("Silence frame count must be a u32 integer"));
    }
    let context = object.com_context()?;
    let view = object
      .0
      .as_object()
      .ok_or_else(|| invalid("COM object is released"))?
      .clone();
    self
      .0
      .write_silence(&context, &view, frames as u32)
      .map_err(native_error)
  }

  #[napi]
  pub fn read_packet_copy(
    &self,
    object: &DynWinRTValue,
  ) -> napi::Result<Option<DynComCapturePacketCopy>> {
    let context = object.com_context()?;
    let view = object
      .0
      .as_object()
      .ok_or_else(|| invalid("COM object is released"))?
      .clone();
    self
      .0
      .read_packet_copy(&context, &view)
      .map(|packet| {
        packet.map(|packet| DynComCapturePacketCopy {
          kind: if packet.data.is_some() {
            "data"
          } else {
            "silent"
          }
          .into(),
          data: packet.data.map(Buffer::from),
          frames: packet.frames,
          flags: packet.flags,
          timestamps_valid: packet.device_position.is_some(),
          device_position: packet.device_position.map(BigInt::from),
          qpc_position: packet.qpc_position.map(BigInt::from),
        })
      })
      .map_err(native_error)
  }

  #[napi]
  pub fn read_copy(&self, object: &DynWinRTValue) -> napi::Result<Buffer> {
    let context = object.com_context()?;
    let view = object
      .0
      .as_object()
      .ok_or_else(|| invalid("COM object is released"))?
      .clone();
    self
      .0
      .read_copy(&context, &view)
      .map(Buffer::from)
      .map_err(native_error)
  }

  #[napi]
  pub fn replace_copy(
    &self,
    object: &DynWinRTValue,
    #[napi(ts_arg_type = "Buffer | Uint8Array")] data: Unknown,
  ) -> napi::Result<()> {
    let bytes = com::stage_copy_bytes(data)?;
    let context = object.com_context()?;
    let view = object
      .0
      .as_object()
      .ok_or_else(|| invalid("COM object is released"))?
      .clone();
    self
      .0
      .replace_copy(&context, &view, &bytes)
      .map_err(native_error)
  }

  #[napi]
  pub fn read_locked_bgra8_copy(
    &self,
    object: &DynWinRTValue,
    rect: Vec<f64>,
  ) -> napi::Result<DynComBitmapCopy> {
    if rect.len() != 4
      || rect
        .iter()
        .any(|v| !v.is_finite() || v.fract() != 0.0 || *v < i32::MIN as f64 || *v > i32::MAX as f64)
    {
      return Err(invalid(
        "Bitmap rectangle requires exactly four i32 integers",
      ));
    }
    let rect = [
      rect[0] as i32,
      rect[1] as i32,
      rect[2] as i32,
      rect[3] as i32,
    ];
    let context = object.com_context()?;
    let view = object
      .0
      .as_object()
      .ok_or_else(|| invalid("COM object is released"))?
      .clone();
    self
      .0
      .read_locked_bgra8_copy(&context, &view, rect)
      .map(|copy| DynComBitmapCopy {
        data: copy.data.into(),
        width: copy.width,
        height: copy.height,
      })
      .map_err(native_error)
  }
}
