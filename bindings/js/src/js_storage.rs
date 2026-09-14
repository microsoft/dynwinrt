// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! JavaScript backing stores and stable call-local bytes. Native interpretation,
//! ownership contracts and apartment policy belong to the calling domain.

use std::{ffi::c_void, marker::PhantomData, ptr, rc::Rc, sync::Mutex};

use napi::{
  bindgen_prelude::{Buffer, FromNapiValue, ToNapiValue, Uint8Array, Unknown},
  sys, JsValue,
};

#[derive(Clone, Copy)]
pub(crate) struct ByteView {
  pub pointer: *mut u8,
  pub length: usize,
}

pub(crate) fn address_is_aligned(address: usize, alignment: usize) -> bool {
  alignment != 0 && address.is_multiple_of(alignment)
}

impl ByteView {
  pub(crate) unsafe fn bytes(&self) -> &[u8] {
    if self.length == 0 {
      &[]
    } else {
      unsafe { std::slice::from_raw_parts(self.pointer, self.length) }
    }
  }

  pub(crate) fn normalized(self) -> Self {
    Self {
      pointer: if self.length == 0 {
        ptr::null_mut()
      } else {
        self.pointer
      },
      ..self
    }
  }

  pub(crate) fn same_storage(self, other: Self) -> bool {
    self.length == other.length && (self.length == 0 || self.pointer == other.pointer)
  }

  pub(crate) fn is_aligned(self, alignment: usize) -> bool {
    address_is_aligned(self.pointer as usize, alignment)
  }

  fn within(self, backing: Self, offset: usize, maximum: usize) -> bool {
    self.length <= maximum
      && offset
        .checked_add(self.length)
        .is_some_and(|end| end <= backing.length)
      && (self.length == 0
        || (!self.pointer.is_null()
          && !backing.pointer.is_null()
          && (backing.pointer as usize).checked_add(offset) == Some(self.pointer as usize)))
  }
}

fn check_status(status: sys::napi_status, message: Option<&str>) -> napi::Result<()> {
  match message {
    Some(message) => napi::check_status!(status, "{message}"),
    None => napi::check_status!(status),
  }
}

pub(crate) fn is_typed_array(
  env: sys::napi_env,
  raw: sys::napi_value,
  message: Option<&str>,
) -> napi::Result<bool> {
  let mut result = false;
  check_status(
    unsafe { sys::napi_is_typedarray(env, raw, &mut result) },
    message,
  )?;
  Ok(result)
}

pub(crate) struct TypedArrayView {
  pub kind: i32,
  pub elements: usize,
  pub pointer: *mut u8,
  backing: sys::napi_value,
  offset: usize,
}

impl TypedArrayView {
  pub(crate) fn inspect(
    env: sys::napi_env,
    raw: sys::napi_value,
    message: Option<&str>,
  ) -> napi::Result<Self> {
    let mut kind = 0;
    let mut elements = 0;
    let mut pointer = ptr::null_mut();
    let mut backing = ptr::null_mut();
    let mut offset = 0;
    check_status(
      unsafe {
        sys::napi_get_typedarray_info(
          env,
          raw,
          &mut kind,
          &mut elements,
          &mut pointer,
          &mut backing,
          &mut offset,
        )
      },
      message,
    )?;
    Ok(Self {
      kind,
      elements,
      pointer: pointer.cast(),
      backing,
      offset,
    })
  }

  pub(crate) fn require_unshared(
    &self,
    env: sys::napi_env,
    message: Option<&str>,
    reason: &str,
  ) -> napi::Result<()> {
    let mut is_array_buffer = false;
    check_status(
      unsafe { sys::napi_is_arraybuffer(env, self.backing, &mut is_array_buffer) },
      message,
    )?;
    if !is_array_buffer {
      return Err(napi::Error::from_reason(reason));
    }
    Ok(())
  }

  pub(crate) fn require_attached(
    &self,
    env: sys::napi_env,
    message: Option<&str>,
    reason: &str,
  ) -> napi::Result<()> {
    let mut detached = false;
    check_status(
      unsafe { sys::napi_is_detached_arraybuffer(env, self.backing, &mut detached) },
      message,
    )?;
    if detached {
      return Err(napi::Error::from_reason(reason));
    }
    Ok(())
  }

  pub(crate) fn bytes(&self, element_size: usize, overflow: &str) -> napi::Result<ByteView> {
    Ok(ByteView {
      pointer: self.pointer,
      length: self
        .elements
        .checked_mul(element_size)
        .ok_or_else(|| napi::Error::from_reason(overflow))?,
    })
  }

  pub(crate) fn checked_extent(
    &self,
    env: sys::napi_env,
    bytes: ByteView,
    maximum: usize,
    reason: &str,
  ) -> napi::Result<ByteView> {
    let mut pointer: *mut c_void = ptr::null_mut();
    let mut length = 0;
    napi::check_status!(unsafe {
      sys::napi_get_arraybuffer_info(env, self.backing, &mut pointer, &mut length)
    })?;
    if !bytes.within(
      ByteView {
        pointer: pointer.cast(),
        length,
      },
      self.offset,
      maximum,
    ) {
      return Err(napi::Error::from_reason(reason));
    }
    Ok(bytes)
  }
}

pub(crate) fn typed_array_element_size(kind: i32) -> Option<usize> {
  use sys::TypedarrayType;
  match kind {
    value
      if value == TypedarrayType::int8_array
        || value == TypedarrayType::uint8_array
        || value == TypedarrayType::uint8_clamped_array =>
    {
      Some(1)
    }
    value if value == TypedarrayType::int16_array || value == TypedarrayType::uint16_array => {
      Some(2)
    }
    value
      if value == TypedarrayType::int32_array
        || value == TypedarrayType::uint32_array
        || value == TypedarrayType::float32_array =>
    {
      Some(4)
    }
    value
      if value == TypedarrayType::float64_array
        || value == TypedarrayType::bigint64_array
        || value == TypedarrayType::biguint64_array =>
    {
      Some(8)
    }
    _ => None,
  }
}

pub(crate) fn uint8_array_info(
  env: sys::napi_env,
  raw: sys::napi_value,
  shared_error: &str,
) -> napi::Result<Option<ByteView>> {
  if !is_typed_array(env, raw, Some("Failed to inspect TypedArray value"))? {
    return Ok(None);
  }
  let info = TypedArrayView::inspect(
    env,
    raw,
    Some("Failed to inspect TypedArray backing storage"),
  )?;
  info.require_unshared(
    env,
    Some("Failed to inspect TypedArray backing storage"),
    shared_error,
  )?;
  if info.kind != sys::TypedarrayType::uint8_array {
    return Ok(None);
  }
  info.require_attached(
    env,
    Some("Failed to inspect TypedArray backing storage"),
    "Cannot use a detached Buffer/Uint8Array",
  )?;
  Ok(Some(ByteView {
    pointer: info.pointer,
    length: info.elements,
  }))
}

pub(crate) struct TypedBufferMessages {
  pub inspect_value: &'static str,
  pub expected: &'static str,
  pub inspect_backing: &'static str,
  pub shared: &'static str,
  pub detached: &'static str,
  pub element_type: &'static str,
  pub overflow: &'static str,
  pub retain: &'static str,
  pub revalidate: &'static str,
  pub changed: &'static str,
}

pub(crate) struct TypedBufferInfo {
  pub bytes: ByteView,
  pub source_element_size: usize,
  pub raw_bytes: bool,
  kind: i32,
}

pub(crate) fn typed_buffer_info(
  env: sys::napi_env,
  raw: sys::napi_value,
  messages: &TypedBufferMessages,
) -> napi::Result<TypedBufferInfo> {
  if !is_typed_array(env, raw, Some(messages.inspect_value))? {
    return Err(napi::Error::from_reason(messages.expected));
  }
  let info = TypedArrayView::inspect(env, raw, Some(messages.inspect_backing))?;
  info.require_unshared(
    env,
    Some("Failed to inspect TypedArray backing storage"),
    messages.shared,
  )?;
  info.require_attached(env, Some(messages.inspect_backing), messages.detached)?;
  let source_element_size = typed_array_element_size(info.kind)
    .ok_or_else(|| napi::Error::from_reason(messages.element_type))?;
  let bytes = info
    .bytes(source_element_size, messages.overflow)?
    .normalized();
  let mut is_buffer = false;
  napi::check_status!(
    unsafe { sys::napi_is_buffer(env, raw, &mut is_buffer) },
    "Failed to identify Node Buffer storage"
  )?;
  Ok(TypedBufferInfo {
    bytes,
    source_element_size,
    raw_bytes: is_buffer,
    kind: info.kind,
  })
}

pub(crate) fn stage_copy_bytes(
  value: Unknown,
  messages: &TypedBufferMessages,
) -> napi::Result<Vec<u8>> {
  let info = typed_buffer_info(value.value().env, value.value().value, messages)?;
  if info.kind != sys::TypedarrayType::uint8_array {
    return Err(napi::Error::from_reason(
      "Owned-copy input must be Buffer or Uint8Array",
    ));
  }
  if info.bytes.length > 64 * 1024 * 1024 {
    return Err(napi::Error::from_reason(
      "Owned-copy input exceeds the 64 MiB safety cap",
    ));
  }
  let mut bytes = Vec::new();
  bytes
    .try_reserve_exact(info.bytes.length)
    .map_err(|_| napi::Error::from_reason("Unable to stage owned copy"))?;
  if info.bytes.length != 0 {
    if info.bytes.pointer.is_null() {
      return Err(napi::Error::from_reason("Owned-copy input storage is null"));
    }
    bytes.extend_from_slice(unsafe { info.bytes.bytes() });
  }
  Ok(bytes)
}

pub(crate) struct RetainedUint8Array {
  value: Mutex<Uint8Array>,
  env: sys::napi_env,
  original: ByteView,
}

impl RetainedUint8Array {
  pub(crate) fn new(env: sys::napi_env, raw: sys::napi_value) -> napi::Result<Self> {
    let array = unsafe { Uint8Array::from_napi_value(env, raw) }?;
    let original = ByteView {
      pointer: array.as_ref().as_ptr().cast_mut(),
      length: array.len(),
    }
    .normalized();
    Ok(Self {
      value: Mutex::new(array),
      env,
      original,
    })
  }

  pub(crate) fn view(&self) -> ByteView {
    self.original
  }

  pub(crate) fn with_value<T>(
    &self,
    lock_error: &str,
    action: impl FnOnce(sys::napi_env, sys::napi_value) -> napi::Result<T>,
  ) -> napi::Result<T> {
    let mut value = self
      .value
      .lock()
      .map_err(|_| napi::Error::from_reason(lock_error))?;
    let raw = unsafe { <&mut Uint8Array as ToNapiValue>::to_napi_value(self.env, &mut *value) }?;
    action(self.env, raw)
  }

  fn validate(&self) -> napi::Result<()> {
    self.with_value("TypedArray pointer owner lock is poisoned", |env, raw| {
      let info = TypedArrayView::inspect(
        env,
        raw,
        Some("Failed to revalidate TypedArray backing storage"),
      )?;
      info.require_attached(
        env,
        Some("Failed to inspect TypedArray backing storage"),
        "Cannot use a pointer whose TypedArray backing ArrayBuffer is detached",
      )?;
      let current = ByteView {
        pointer: info.pointer,
        length: info.elements,
      };
      if !self.original.same_storage(current) {
        return Err(napi::Error::from_reason(
          "Cannot use a pointer whose TypedArray backing storage changed",
        ));
      }
      Ok(())
    })
  }
}

pub(crate) struct RetainedTypedBuffer {
  env: sys::napi_env,
  reference: sys::napi_ref,
  original: TypedBufferInfo,
  messages: &'static TypedBufferMessages,
}

impl RetainedTypedBuffer {
  pub(crate) fn new(
    env: sys::napi_env,
    raw: sys::napi_value,
    messages: &'static TypedBufferMessages,
  ) -> napi::Result<Self> {
    let original = typed_buffer_info(env, raw, messages)?;
    let mut reference = ptr::null_mut();
    napi::check_status!(
      unsafe { sys::napi_create_reference(env, raw, 1, &mut reference) },
      "{}",
      messages.retain
    )?;
    Ok(Self {
      env,
      reference,
      original,
      messages,
    })
  }

  pub(crate) fn info(&self) -> &TypedBufferInfo {
    &self.original
  }

  fn validate(&self) -> napi::Result<()> {
    let mut raw = ptr::null_mut();
    napi::check_status!(
      unsafe { sys::napi_get_reference_value(self.env, self.reference, &mut raw) },
      "{}",
      self.messages.revalidate
    )?;
    let current = typed_buffer_info(self.env, raw, self.messages)?;
    if !self.original.bytes.same_storage(current.bytes) || self.original.kind != current.kind {
      return Err(napi::Error::from_reason(self.messages.changed));
    }
    Ok(())
  }
}

impl Drop for RetainedTypedBuffer {
  fn drop(&mut self) {
    if !self.reference.is_null() {
      let _ = unsafe { sys::napi_delete_reference(self.env, self.reference) };
      self.reference = ptr::null_mut();
    }
  }
}

pub(crate) enum CallStorage {
  Uint8Array(RetainedUint8Array),
  TypedBuffer(RetainedTypedBuffer),
  Words(Box<[u16]>),
  Bytes(Box<[u8]>),
}

impl CallStorage {
  pub(crate) fn validate(&self) -> napi::Result<()> {
    match self {
      Self::Uint8Array(value) => value.validate(),
      Self::TypedBuffer(value) => value.validate(),
      Self::Words(_) | Self::Bytes(_) => Ok(()),
    }
  }
}

pub(crate) struct RetainedBuffer {
  buffer: Buffer,
  original: ByteView,
  env: usize,
  _owner_thread: PhantomData<Rc<()>>,
}

impl RetainedBuffer {
  pub(crate) fn new(buffer: Buffer, env: sys::napi_env) -> Self {
    Self {
      original: ByteView {
        pointer: buffer.as_ptr().cast_mut(),
        length: buffer.len(),
      },
      buffer,
      env: env as usize,
      _owner_thread: PhantomData,
    }
  }

  pub(crate) fn env(&self) -> sys::napi_env {
    self.env as sys::napi_env
  }

  pub(crate) fn original(&self) -> ByteView {
    self.original
  }

  pub(crate) fn into_value(self) -> napi::Result<sys::napi_value> {
    unsafe { Buffer::to_napi_value(self.env(), self.buffer) }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn view(address: usize, length: usize) -> ByteView {
    ByteView {
      pointer: ptr::with_exposed_provenance_mut(address),
      length,
    }
  }

  #[test]
  fn byte_extents_require_the_exact_backing_offset_and_bounded_range() {
    let backing = view(0x1000, 32);
    assert!(view(0x1004, 8).within(backing, 4, 8));
    assert!(!view(0x1005, 8).within(backing, 4, 8));
    assert!(!view(0x1004, 9).within(backing, 4, 8));
    assert!(!view(0x1018, 16).within(backing, 24, 32));
    assert!(!view(0, 1).within(backing, 0, 32));
    assert!(!view(1, 8).within(view(0, 32), 1, 32));
    assert!(!view(1, 8).within(view(usize::MAX, 32), 2, 32));
    assert!(!view(0x1000, 8).within(backing, usize::MAX, 32));
    assert!(view(0, 0).within(backing, 32, 0));
    assert!(!view(0, 0).within(backing, 33, 0));
  }

  #[test]
  fn backing_identity_and_alignment_preserve_empty_view_rules() {
    assert!(view(0x1000, 8).same_storage(view(0x1000, 8)));
    assert!(!view(0x1000, 8).same_storage(view(0x1000, 4)));
    assert!(!view(0x1000, 8).same_storage(view(0x1004, 8)));
    assert!(view(0x1000, 0).same_storage(view(0, 0)));
    assert!(view(0x1000, 0).normalized().pointer.is_null());
    assert!(view(0x1000, 8).is_aligned(8));
    assert!(!view(0x1001, 8).is_aligned(2));
    assert!(!view(0x1000, 8).is_aligned(0));
  }

  #[test]
  fn typed_array_widths_are_storage_facts() {
    for (kinds, width) in [
      (&[0, 1, 2][..], 1),
      (&[3, 4][..], 2),
      (&[5, 6, 7][..], 4),
      (&[8, 9, 10][..], 8),
    ] {
      for &kind in kinds {
        assert_eq!(typed_array_element_size(kind), Some(width));
      }
    }
    assert_eq!(typed_array_element_size(i32::MAX), None);
  }
}
