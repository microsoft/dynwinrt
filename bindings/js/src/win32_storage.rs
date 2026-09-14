// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{ffi::c_void, ptr};

use napi::{
  bindgen_prelude::{Buffer, FromNapiValue, Unknown},
  sys, JsValue,
};

pub(super) use crate::js_storage::ByteView as BufferInfo;
use crate::js_storage::{self, CallStorage, RetainedUint8Array, TypedArrayView};

pub(super) const MAX_BUFFER_BYTES: usize = 64 * 1024 * 1024;

pub(super) struct PointerStorage {
  pub pointer: *mut c_void,
  pub backing: Option<CallStorage>,
}

impl PointerStorage {
  pub(super) fn borrowed(pointer: *mut c_void) -> Self {
    Self {
      pointer,
      backing: None,
    }
  }

  pub(super) fn retained(pointer: *mut c_void, backing: CallStorage) -> Self {
    Self {
      pointer,
      backing: Some(backing),
    }
  }

  pub(super) fn validate(&self) -> napi::Result<()> {
    if let Some(storage) = &self.backing {
      storage.validate()?;
    }
    Ok(())
  }
}

fn retained_bytes(value: Unknown) -> napi::Result<PointerStorage> {
  let storage = RetainedUint8Array::new(value.value().env, value.raw())?;
  Ok(PointerStorage::retained(
    storage.view().pointer.cast(),
    CallStorage::Uint8Array(storage),
  ))
}

pub(super) fn signed64(value: &Unknown) -> napi::Result<i64> {
  let mut integer = 0;
  let mut lossless = false;
  napi::check_status!(unsafe {
    sys::napi_get_value_bigint_int64(value.value().env, value.raw(), &mut integer, &mut lossless)
  })?;
  if !lossless {
    return Err(napi::Error::from_reason(
      "Win32 bigint must fit a signed 64-bit integer",
    ));
  }
  Ok(integer)
}

pub(super) fn unsigned64(value: &Unknown) -> napi::Result<u64> {
  let mut integer = 0;
  let mut lossless = false;
  napi::check_status!(unsafe {
    sys::napi_get_value_bigint_uint64(value.value().env, value.raw(), &mut integer, &mut lossless)
  })?;
  if !lossless {
    return Err(napi::Error::from_reason(
      "Win32 bigint must fit an unsigned 64-bit integer",
    ));
  }
  Ok(integer)
}

pub(super) fn buffer_info(env: sys::napi_env, raw: sys::napi_value) -> napi::Result<BufferInfo> {
  if !js_storage::is_typed_array(env, raw, None)? {
    return Err(napi::Error::from_reason(
      "Expected Buffer or Uint8Array native storage",
    ));
  }
  let info = TypedArrayView::inspect(env, raw, None)?;
  if info.kind != sys::TypedarrayType::uint8_array as i32 {
    return Err(napi::Error::from_reason(
      "Expected unsigned byte Buffer or Uint8Array storage",
    ));
  }
  info.require_unshared(
    env,
    None,
    "SharedArrayBuffer storage is unsupported for Win32",
  )?;
  info.require_attached(env, None, "Cannot use a detached Win32 Buffer/Uint8Array")?;
  info.checked_extent(
    env,
    BufferInfo {
      pointer: info.pointer,
      length: info.elements,
    },
    MAX_BUFFER_BYTES,
    "Invalid or excessive Win32 native buffer extent (64 MiB maximum)",
  )
}

pub(super) fn length(value: &Unknown) -> napi::Result<usize> {
  Ok(buffer_info(value.value().env, value.raw())?.length)
}

pub(super) fn checked_length(value: f64) -> napi::Result<usize> {
  if !value.is_finite() || value.fract() != 0.0 || !(0.0..=MAX_BUFFER_BYTES as f64).contains(&value)
  {
    return Err(napi::Error::from_reason(
      "Win32 byte length must be a non-negative integer no larger than 64 MiB",
    ));
  }
  Ok(value as usize)
}

pub(super) fn zeroed(length: usize) -> napi::Result<Vec<u8>> {
  if length > MAX_BUFFER_BYTES {
    return Err(napi::Error::from_reason(
      "Win32 buffer exceeds the 64 MiB limit",
    ));
  }
  let mut bytes = Vec::new();
  bytes
    .try_reserve_exact(length)
    .map_err(|_| napi::Error::from_reason("Unable to allocate Win32 native buffer"))?;
  bytes.resize(length, 0);
  Ok(bytes)
}

pub(super) fn copy(value: &Unknown, actual: Option<f64>) -> napi::Result<Buffer> {
  let info = buffer_info(value.value().env, value.raw())?;
  let length = actual
    .map(checked_length)
    .transpose()?
    .unwrap_or(info.length);
  if length > info.length {
    return Err(napi::Error::from_reason(
      "Successful Win32 output length exceeds native buffer capacity",
    ));
  }
  let mut bytes = zeroed(length)?;
  if length != 0 {
    unsafe { ptr::copy_nonoverlapping(info.pointer, bytes.as_mut_ptr(), length) };
  }
  Ok(Buffer::from(bytes))
}

fn is_null(value: &Unknown) -> napi::Result<bool> {
  Ok(matches!(
    value.get_type()?,
    napi::ValueType::Null | napi::ValueType::Undefined
  ))
}

pub(super) fn data_pointer(value: Unknown, nullable: bool) -> napi::Result<PointerStorage> {
  if matches!(
    value.get_type()?,
    napi::ValueType::Number | napi::ValueType::BigInt
  ) {
    return Err(napi::Error::from_reason(
      "arbitrary numeric addresses require @microsoft/dynwinrt/win32/unsafe",
    ));
  }
  if is_null(&value)? {
    return if nullable {
      Ok(PointerStorage::borrowed(ptr::null_mut()))
    } else {
      Err(napi::Error::from_reason(
        "safeDataPointer(): null requires an explicitly nullable parameter",
      ))
    };
  }
  buffer_info(value.value().env, value.raw())?;
  retained_bytes(value)
}

pub(super) fn unsafe_pointer(value: Unknown) -> napi::Result<PointerStorage> {
  if value.get_type()? == napi::ValueType::BigInt {
    let bits = usize::try_from(unsigned64(&value)?)
      .map_err(|_| napi::Error::from_reason("Win32 address exceeds the process pointer width"))?;
    return Ok(PointerStorage::borrowed(bits as *mut c_void));
  }
  if value.get_type()? == napi::ValueType::Object {
    buffer_info(value.value().env, value.raw())?;
    return retained_bytes(value);
  }
  if is_null(&value)? {
    return Ok(PointerStorage::borrowed(ptr::null_mut()));
  }
  if value.get_type()? == napi::ValueType::Number {
    let mut number = 0.0;
    unsafe { sys::napi_get_value_double(value.value().env, value.raw(), &mut number) };
    if !number.is_finite()
      || number < 0.0
      || number.fract() != 0.0
      || number > 9_007_199_254_740_991.0
      || number as u64 as usize as u64 != number as u64
    {
      return Err(napi::Error::from_reason(
        "pointer(): number must be a non-negative safe integer that fits in a pointer",
      ));
    }
    return Ok(PointerStorage::borrowed(number as usize as *mut c_void));
  }
  Err(napi::Error::from_reason(
    "pointer(): expected bigint, number, Buffer, Uint8Array, null, or undefined",
  ))
}

fn utf16(env: sys::napi_env, raw: sys::napi_value) -> napi::Result<Vec<u16>> {
  let mut length = 0;
  napi::check_status!(unsafe {
    sys::napi_get_value_string_utf16(env, raw, ptr::null_mut(), 0, &mut length)
  })?;
  if length >= MAX_BUFFER_BYTES / 2 {
    return Err(napi::Error::from_reason(
      "Win32 UTF-16 string exceeds the 64 MiB limit",
    ));
  }
  let mut units = Vec::new();
  units
    .try_reserve_exact(length + 1)
    .map_err(|_| napi::Error::from_reason("Unable to allocate Win32 UTF-16 storage"))?;
  units.resize(length + 1, 0);
  let mut written = 0;
  napi::check_status!(unsafe {
    sys::napi_get_value_string_utf16(env, raw, units.as_mut_ptr(), units.len(), &mut written)
  })?;
  if written != length {
    return Err(napi::Error::from_reason(
      "Win32 UTF-16 input length changed",
    ));
  }
  units.truncate(length);
  if units.contains(&0) || char::decode_utf16(units.iter().copied()).any(|unit| unit.is_err()) {
    return Err(napi::Error::from_reason(
      "Win32 UTF-16 input contains NUL or ill-formed UTF-16",
    ));
  }
  Ok(units)
}

fn push_units(storage: &mut Vec<u16>, units: &[u16]) -> napi::Result<()> {
  let additional = units
    .len()
    .checked_add(1)
    .ok_or_else(|| napi::Error::from_reason("Win32 string length overflow"))?;
  let length = storage
    .len()
    .checked_add(additional)
    .filter(|length| *length < MAX_BUFFER_BYTES / 2)
    .ok_or_else(|| napi::Error::from_reason("Win32 string exceeds the 64 MiB limit"))?;
  storage
    .try_reserve_exact(additional)
    .map_err(|_| napi::Error::from_reason("Unable to allocate Win32 string storage"))?;
  storage.extend_from_slice(units);
  storage.resize(length, 0);
  Ok(())
}

pub(super) fn string_pointer(
  value: Unknown,
  nullable: bool,
  wide: bool,
  multi: bool,
) -> napi::Result<PointerStorage> {
  if is_null(&value)? {
    return if nullable {
      Ok(PointerStorage::borrowed(ptr::null_mut()))
    } else {
      Err(napi::Error::from_reason(
        "Win32 string null requires an explicitly nullable parameter",
      ))
    };
  }
  let env = value.value().env;
  let raw = value.raw();
  let mut is_array = false;
  if multi {
    napi::check_status!(unsafe { sys::napi_is_array(env, raw, &mut is_array) })?;
  }
  if value.get_type()? == napi::ValueType::String || is_array {
    let mut units = Vec::new();
    if is_array {
      let mut length = 0u32;
      napi::check_status!(unsafe { sys::napi_get_array_length(env, raw, &mut length) })?;
      if length as usize > MAX_BUFFER_BYTES / 2 - 2 {
        return Err(napi::Error::from_reason(
          "Win32 multi-string array exceeds the storage bound",
        ));
      }
      for index in 0..length {
        let mut element = ptr::null_mut();
        napi::check_status!(unsafe { sys::napi_get_element(env, raw, index, &mut element) })?;
        let text = utf16(env, element)?;
        if text.is_empty() && length > 1 {
          return Err(napi::Error::from_reason(
            "Empty multi-string entries would truncate the list",
          ));
        }
        push_units(&mut units, &text)?;
      }
      if length == 0 {
        push_units(&mut units, &[])?;
      }
    } else {
      push_units(&mut units, &utf16(env, raw)?)?;
    }
    if multi {
      push_units(&mut units, &[])?;
    }
    return if wide {
      let mut units = units.into_boxed_slice();
      let pointer = units.as_mut_ptr().cast();
      Ok(PointerStorage::retained(pointer, CallStorage::Words(units)))
    } else {
      if units.iter().any(|unit| *unit > 0x7f) {
        return Err(napi::Error::from_reason(
          "ANSI strings must be ASCII; use explicitly encoded terminated Buffer storage",
        ));
      }
      let mut bytes = zeroed(units.len())?.into_boxed_slice();
      for (target, unit) in bytes.iter_mut().zip(units) {
        *target = unit as u8;
      }
      let pointer = bytes.as_mut_ptr().cast();
      Ok(PointerStorage::retained(pointer, CallStorage::Bytes(bytes)))
    };
  }
  let info = buffer_info(env, raw)?;
  validate_string_bytes(&info, wide, multi)?;
  retained_bytes(value)
}

pub(super) fn validate_string_bytes(
  info: &BufferInfo,
  wide: bool,
  multi: bool,
) -> napi::Result<()> {
  let terminator = if wide { 2 } else { 1 } * if multi { 2 } else { 1 };
  if info.length < terminator
    || (wide && (info.length % 2 != 0 || !info.is_aligned(2)))
    || unsafe { info.bytes() }[info.length - terminator..]
      .iter()
      .any(|byte| *byte != 0)
  {
    return Err(napi::Error::from_reason(if multi && wide {
      "Win32 multi-string storage must end in two NUL code units with UTF-16 alignment"
    } else if multi {
      "Win32 multi-string storage must end in two NUL bytes"
    } else {
      "Win32 string storage must be aligned and NUL-terminated"
    }));
  }
  Ok(())
}

pub(super) fn validate_string_owner(
  owner: &PointerStorage,
  wide: bool,
  multi: bool,
) -> napi::Result<()> {
  match &owner.backing {
    Some(CallStorage::Uint8Array(storage)) => storage
      .with_value("Win32 string owner lock is poisoned", |env, raw| {
        validate_string_bytes(&buffer_info(env, raw)?, wide, multi)
      }),
    Some(CallStorage::Words(units)) if wide => validate_string_bytes(
      &BufferInfo {
        pointer: units.as_ptr().cast::<u8>().cast_mut(),
        length: units.len() * 2,
      },
      wide,
      multi,
    ),
    Some(CallStorage::Bytes(bytes)) if !wide => validate_string_bytes(
      &BufferInfo {
        pointer: bytes.as_ptr().cast_mut(),
        length: bytes.len(),
      },
      wide,
      multi,
    ),
    None if owner.pointer.is_null() => Ok(()),
    _ => Err(napi::Error::from_reason(
      "Win32 string is missing its exact retained storage",
    )),
  }
}

pub(super) fn native_buffer(value: Unknown) -> napi::Result<Buffer> {
  buffer_info(value.value().env, value.raw())?;
  let mut is_buffer = false;
  napi::check_status!(unsafe {
    sys::napi_is_buffer(value.value().env, value.raw(), &mut is_buffer)
  })?;
  if !is_buffer {
    return Err(napi::Error::from_reason(
      "OVERLAPPED I/O requires a Node Buffer",
    ));
  }
  unsafe { Buffer::from_napi_value(value.value().env, value.raw()) }
}
