// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use dynwinrt::com::borrowed::{contracts, BorrowedCopyContract, BorrowedCopyPlan, ContextEffect};
use napi::bindgen_prelude::{BigInt, Buffer, Unknown};
use napi_derive::napi;
use serde::Deserialize;

use super::{com, DynWinRTValue, TABLE};

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
  let effect = match value.effect.as_str() {
    "audio-initialize" => ContextEffect::AudioInitialize,
    "audio-initialize-shared" => ContextEffect::AudioInitializeShared,
    "audio-get-service" => ContextEffect::AudioGetService,
    _ => return Err(invalid("Unsupported audio context effect")),
  };
  let evidence = contracts::registry()
    .evidence
    .iter()
    .find(|entry| entry.effect.as_deref() == Some(value.effect.as_str()))
    .ok_or_else(|| invalid("Unknown audio context evidence"))?;
  if value.version != 1
    || value.metadata_sha256 != contracts::METADATA_SHA256
    || value.fingerprint != evidence.fingerprint
  {
    return Err(invalid("Audio context evidence drift; regenerate bindings"));
  }
  Ok(effect)
}

fn parse_contract(value: &str) -> napi::Result<BorrowedCopyContract> {
  BorrowedCopyContract::parse(value).map_err(invalid)
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
