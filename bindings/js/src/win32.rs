// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::BTreeMap;
use std::rc::Rc;
#[cfg(test)]
use std::sync::atomic::Ordering;
use std::sync::Arc;

use napi::bindgen_prelude::{BigInt, Buffer, Either, FromNapiValue, Unknown};
use napi::JsValue;
use napi_derive::napi;
#[cfg(test)]
use windows::Win32::Foundation::HANDLE;

use super::{com, win32_subsystem, DynWin32SubsystemContext, DynWinRTValue, WinGUID};

#[path = "win32_boundary.rs"]
pub(super) mod boundary;
#[path = "win32_call_error.rs"]
mod call_error;
#[path = "win32_io.rs"]
mod io;
pub use io::DynWin32OverlappedOperation;
#[path = "win32_spec.rs"]
mod specification;
#[path = "win32_storage.rs"]
mod storage;

const MAX_NATIVE_AGGREGATE_DESCRIPTOR_LENGTH: usize = 1024 * 1024;

#[napi(object)]
pub struct DynWin32ParameterSpec {
  #[napi(js_name = "type")]
  pub typ: String,
  pub direction: String,
  pub nullable: Option<bool>,
  pub cleanup: Option<String>,
  pub consumes_resource: Option<bool>,
  pub resource_cleanup: Option<String>,
  pub aggregate_descriptor: Option<String>,
  pub pointee_descriptor: Option<String>,
}

#[napi(object)]
pub struct DynWin32FunctionSpec {
  pub dll: String,
  pub entry_point: String,
  pub parameters: Vec<DynWin32ParameterSpec>,
  pub return_type: Option<String>,
  pub return_cleanup: Option<String>,
  pub success_rule: Option<String>,
  pub capture_last_error: Option<bool>,
  pub calling_convention: Option<String>,
  pub return_aggregate_descriptor: Option<String>,
  pub call_contract_descriptor: Option<String>,
}

pub struct DynWin32Value {
  value: dynwinrt::win32::Value,
  pointer_owner: Option<Win32PointerOwner>,
}
boundary::carrier!(DynWin32Value, 1, "DynWin32Value");

enum Win32PointerOwner {
  Native(Rc<RetainedNativePointer>),
  Aggregate(Rc<NativeAggregateStorage>),
  PointerSlot {
    inner: Rc<RetainedNativePointer>,
    slot: Box<usize>,
  },
}

enum RetainedNativePointer {
  Storage {
    value: storage::PointerStorage,
    string: Option<(bool, bool)>,
  },
  Com(Box<DynWinRTValue>),
}

impl RetainedNativePointer {
  fn com_value(&self) -> Option<&DynWinRTValue> {
    match self {
      Self::Com(value) => Some(value),
      Self::Storage { .. } => None,
    }
  }

  fn has_storage(&self) -> bool {
    matches!(self, Self::Storage { value, .. } if value.backing.is_some())
  }

  fn validate_storage(&self) -> napi::Result<()> {
    match self {
      Self::Storage { value, .. } => value.validate(),
      Self::Com(value) => value.validate_pointer_owner(),
    }
  }

  fn validate(&self) -> napi::Result<()> {
    if let Self::Com(value) = self {
      value.ensure_existing_com_apartment()?;
      value.check_com_input_state()?;
    }
    self.validate_storage()?;
    if let Self::Storage {
      value,
      string: Some((wide, multi)),
    } = self
    {
      storage::validate_string_owner(value, *wide, *multi)?;
    }
    Ok(())
  }
}

impl DynWin32Value {
  fn new(value: dynwinrt::win32::Value) -> Self {
    Self {
      value,
      pointer_owner: None,
    }
  }

  fn with_pointer_owner(
    value: dynwinrt::win32::Value,
    pointer_owner: RetainedNativePointer,
  ) -> Self {
    Self {
      value,
      pointer_owner: Some(Win32PointerOwner::Native(Rc::new(pointer_owner))),
    }
  }

  fn validate(&self) -> napi::Result<()> {
    if let Some(Win32PointerOwner::Native(owner)) = &self.pointer_owner {
      owner.validate()?;
    }
    if let Some(Win32PointerOwner::Aggregate(owner)) = &self.pointer_owner {
      let _ = owner.byte_length;
    }
    if let Some(Win32PointerOwner::PointerSlot { inner, slot }) = &self.pointer_owner {
      inner.validate()?;
      let _ = **slot;
    }
    Ok(())
  }
}

struct NativeAggregateStorage {
  state: std::sync::Mutex<NativeAggregateState>,
  backing: Arc<dynwinrt::win32::NativeAggregateBuffer>,
  byte_length: usize,
  contains_pointers: bool,
}

struct NativeAggregateState {
  owners: BTreeMap<usize, Rc<RetainedNativePointer>>,
}

impl NativeAggregateStorage {
  fn new(
    identity: String,
    byte_length: usize,
    alignment: usize,
    bytes: Option<&[u8]>,
    contains_pointers: bool,
    fields: Vec<dynwinrt::win32::AggregateResultField>,
  ) -> napi::Result<Self> {
    if contains_pointers && bytes.is_some() {
      return Err(napi::Error::from_reason(
        "pointer-bearing native aggregates cannot be initialized from raw bytes",
      ));
    }
    if byte_length > dynwinrt::win32::MAX_NATIVE_AGGREGATE_SIZE {
      return Err(napi::Error::from_reason(format!(
        "native aggregate exceeds the {} byte safety limit",
        dynwinrt::win32::MAX_NATIVE_AGGREGATE_SIZE
      )));
    }
    let layout =
      dynwinrt::win32::NativeAggregatePointerLayout::new(identity, byte_length, alignment, fields)
        .map_err(|error| napi::Error::from_reason(error.message()))?;
    let backing = dynwinrt::win32::NativeAggregateBuffer::new(layout, bytes)
      .map_err(|error| napi::Error::from_reason(error.message()))?;
    Ok(Self {
      state: std::sync::Mutex::new(NativeAggregateState {
        owners: BTreeMap::new(),
      }),
      backing,
      byte_length,
      contains_pointers,
    })
  }

  fn pointer(&self) -> *mut std::ffi::c_void {
    self.backing.pointer()
  }

  fn bytes(&self) -> napi::Result<Vec<u8>> {
    if self.contains_pointers {
      return Err(napi::Error::from_reason(
        "raw bytes are unavailable for pointer-bearing native aggregates",
      ));
    }
    self
      .backing
      .bytes()
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  fn write_field(
    &self,
    offset: usize,
    bytes: &[u8],
    owner: Option<Rc<RetainedNativePointer>>,
  ) -> napi::Result<()> {
    let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
    self
      .backing
      .write(offset, bytes)
      .map_err(|error| napi::Error::from_reason(error.message()))?;
    state.owners.remove(&offset);
    if let Some(owner) = owner {
      state.owners.insert(offset, owner);
    }
    Ok(())
  }

  fn read_field<const N: usize>(&self, offset: usize) -> napi::Result<[u8; N]> {
    self
      .backing
      .read(offset)
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  unsafe fn mark_call_result(&self, succeeded: bool) -> napi::Result<()> {
    unsafe { self.backing.mark_legacy(succeeded) }
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  fn prepare_call(&self) -> napi::Result<()> {
    self
      .backing
      .prepare()
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  fn require_success(&self) -> napi::Result<()> {
    self
      .backing
      .require_success()
      .map_err(|error| napi::Error::from_reason(error.message()))
  }
}

pub struct DynWin32NativeStruct {
  descriptor: String,
  storage: Rc<NativeAggregateStorage>,
}
boundary::carrier!(DynWin32NativeStruct, 2, "DynWin32NativeStruct");

impl DynWin32NativeStruct {
  pub fn bytes(&self) -> napi::Result<Buffer> {
    Ok(Buffer::from(self.storage.bytes()?))
  }

  pub fn length(&self) -> u32 {
    self.storage.byte_length as u32
  }
}

pub struct DynWin32Resource(Arc<dynwinrt::win32::OwnedResource>);
boundary::carrier!(DynWin32Resource, 3, "DynWin32Resource");

impl DynWin32Resource {
  pub fn value(&self) -> BigInt {
    BigInt::from(self.0.raw() as u64)
  }

  pub fn closed(&self) -> bool {
    self.0.is_closed()
  }

  pub fn busy(&self) -> bool {
    self.0.has_async_leases()
  }

  pub fn active(&self) -> bool {
    self.0.has_active_async_io()
  }

  pub fn close(&self) -> napi::Result<()> {
    self
      .0
      .close()
      .map_err(|error| napi::Error::from_reason(error.to_string()))
  }
}

pub struct DynWin32Function(Arc<dynwinrt::win32::CallPlan>);
boundary::carrier!(DynWin32Function, 4, "DynWin32Function");

impl DynWin32Function {
  pub fn bind(spec: DynWin32FunctionSpec) -> napi::Result<Self> {
    bind_function(spec)
  }

  pub fn dll(&self) -> String {
    self.0.dll().to_string()
  }

  pub fn entry_point(&self) -> String {
    self.0.entry_point().to_string()
  }

  pub fn invoke(
    &self,
    env: napi::Env,
    args: Vec<&DynWin32Value>,
  ) -> napi::Result<DynWin32CallResult> {
    self.invoke_impl(env, args)
  }

  pub fn invoke_with_subsystem(
    &self,
    env: napi::Env,
    context: &DynWin32SubsystemContext,
    subsystem: String,
    args: Vec<&DynWin32Value>,
  ) -> napi::Result<DynWin32CallResult> {
    let _subsystem_guard = win32_subsystem::call_guard(context, &subsystem)?;
    self.invoke_impl(env, args)
  }
}

impl DynWin32Function {
  fn invoke_impl(
    &self,
    env: napi::Env,
    args: Vec<&DynWin32Value>,
  ) -> napi::Result<DynWin32CallResult> {
    let owners = args
      .iter()
      .filter_map(|value| match &value.pointer_owner {
        Some(Win32PointerOwner::Native(owner)) => Some(owner.as_ref()),
        Some(Win32PointerOwner::PointerSlot { inner, .. }) => Some(inner.as_ref()),
        _ => None,
      })
      .collect::<Vec<_>>();
    let com_inputs = owners
      .iter()
      .filter_map(|owner| owner.com_value())
      .collect::<Vec<_>>();
    com::with_win32_input_leases(&com_inputs, |validate_inputs| {
      for owner in &owners {
        owner.validate_storage()?;
      }
      self.invoke_validated(env, &args, validate_inputs)
    })
  }

  fn invoke_validated(
    &self,
    env: napi::Env,
    args: &[&DynWin32Value],
    validate_inputs: &dyn Fn() -> napi::Result<()>,
  ) -> napi::Result<DynWin32CallResult> {
    for value in args {
      value.validate()?;
    }
    let mut aggregates = args
      .iter()
      .filter_map(|value| match &value.pointer_owner {
        Some(Win32PointerOwner::Aggregate(owner)) => Some(owner),
        _ => None,
      })
      .collect::<Vec<_>>();
    aggregates.sort_by_key(|owner| Rc::as_ptr(owner) as usize);
    if aggregates
      .windows(2)
      .any(|pair| Rc::ptr_eq(pair[0], pair[1]))
    {
      return Err(napi::Error::from_reason(
        "the same native aggregate cannot occupy multiple parameters in one call",
      ));
    }
    let _aggregate_guards = aggregates
      .into_iter()
      .map(|owner| {
        owner
          .state
          .lock()
          .unwrap_or_else(|error| error.into_inner())
      })
      .collect::<Vec<_>>();
    for state in &_aggregate_guards {
      for owner in state.owners.values() {
        owner.validate()?;
      }
    }
    let _by_value_guards = args
      .iter()
      .filter_map(|value| {
        if matches!(value.value, dynwinrt::win32::Value::Aggregate { .. }) {
          if let Some(Win32PointerOwner::Aggregate(owner)) = &value.pointer_owner {
            return Some(owner.backing.lease());
          }
        }
        None
      })
      .collect::<Vec<_>>();
    let values = args
      .iter()
      .map(|value| value.value.clone())
      .collect::<Vec<_>>();
    validate_inputs()?;
    let failure = if self.0.has_owned_result_cleanup() {
      Some(call_error::PreparedCallError::new(
        env,
        format!(
          "DynWin32Function {}!{}: ",
          self.0.dll(),
          self.0.entry_point()
        ),
      )?)
    } else {
      None
    };
    let result = match unsafe { self.0.invoke(&values) } {
      Ok(result) => result,
      Err(error) => {
        return Err(match failure {
          Some(failure) => failure.raise(error),
          None => napi::Error::from_reason(format!(
            "DynWin32Function {}!{}: {}",
            self.0.dll(),
            self.0.entry_point(),
            error.message()
          )),
        });
      }
    };
    Ok(DynWin32CallResult {
      return_value: result.return_value.map(DynWin32Value::new),
      outputs: Some(result.outputs.into_iter().map(DynWin32Value::new).collect()),
      last_error: result.last_error,
      succeeded: result.succeeded,
    })
  }
}

pub struct DynWin32CallResult {
  return_value: Option<DynWin32Value>,
  outputs: Option<Vec<DynWin32Value>>,
  last_error: Option<u32>,
  succeeded: bool,
}
boundary::carrier!(DynWin32CallResult, 5, "DynWin32CallResult");

impl DynWin32CallResult {
  pub fn return_value(&mut self) -> napi::Result<Option<DynWin32Value>> {
    Ok(self.return_value.take())
  }

  pub fn outputs(&mut self) -> napi::Result<Vec<DynWin32Value>> {
    self
      .outputs
      .take()
      .ok_or_else(|| napi::Error::from_reason("Win32 outputs were already consumed"))
  }

  pub fn last_error(&self) -> Option<u32> {
    self.last_error
  }

  pub fn succeeded(&self) -> bool {
    self.succeeded
  }
}

native_class! {
  pub struct DynWin32;
}

#[napi]
impl DynWin32 {
  #[napi]
  pub fn initialize_winsock() -> napi::Result<DynWin32SubsystemContext> {
    win32_subsystem::initialize("winsock")
  }

  #[napi]
  pub fn initialize_gdi_plus() -> napi::Result<DynWin32SubsystemContext> {
    win32_subsystem::initialize("gdiplus")
  }

  #[napi]
  pub fn initialize_media_foundation() -> napi::Result<DynWin32SubsystemContext> {
    win32_subsystem::initialize("mediaFoundation")
  }

  /// Requires an installed, configured MAPI provider of the process architecture.
  /// The Windows system stub alone is insufficient and may display native UI.
  #[napi]
  pub fn initialize_mapi_utilities() -> napi::Result<DynWin32SubsystemContext> {
    win32_subsystem::initialize("mapiUtilities")
  }

  #[napi]
  pub fn require_subsystem(
    context: &DynWin32SubsystemContext,
    #[napi(ts_arg_type = "string")] subsystem: specification::Name,
  ) -> napi::Result<()> {
    win32_subsystem::require(context, &subsystem)
  }

  #[napi]
  pub fn bool8(value: bool) -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::U8(u8::from(value)))
  }

  #[napi]
  pub fn bool32(value: bool) -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::Bool(value))
  }

  #[napi]
  pub fn i8(value: f64) -> napi::Result<DynWin32Value> {
    check_integer(value, i8::MIN as f64, i8::MAX as f64, "i8")?;
    Ok(DynWin32Value::new(dynwinrt::win32::Value::I8(value as i8)))
  }

  #[napi]
  pub fn u8(value: f64) -> napi::Result<DynWin32Value> {
    check_integer(value, 0.0, u8::MAX as f64, "u8")?;
    Ok(DynWin32Value::new(dynwinrt::win32::Value::U8(value as u8)))
  }

  #[napi]
  pub fn i16(value: f64) -> napi::Result<DynWin32Value> {
    check_integer(value, i16::MIN as f64, i16::MAX as f64, "i16")?;
    Ok(DynWin32Value::new(dynwinrt::win32::Value::I16(
      value as i16,
    )))
  }

  #[napi]
  pub fn u16(value: f64) -> napi::Result<DynWin32Value> {
    check_integer(value, 0.0, u16::MAX as f64, "u16")?;
    Ok(DynWin32Value::new(dynwinrt::win32::Value::U16(
      value as u16,
    )))
  }

  #[napi]
  pub fn i32(value: f64) -> napi::Result<DynWin32Value> {
    check_integer(value, i32::MIN as f64, i32::MAX as f64, "i32")?;
    Ok(DynWin32Value::new(dynwinrt::win32::Value::I32(
      value as i32,
    )))
  }

  #[napi]
  pub fn u32(value: f64) -> napi::Result<DynWin32Value> {
    check_integer(value, 0.0, u32::MAX as f64, "u32")?;
    Ok(DynWin32Value::new(dynwinrt::win32::Value::U32(
      value as u32,
    )))
  }

  #[napi]
  pub fn i64(#[napi(ts_arg_type = "bigint")] value: Unknown) -> napi::Result<DynWin32Value> {
    Ok(DynWin32Value::new(dynwinrt::win32::Value::I64(
      storage::signed64(&value)?,
    )))
  }

  #[napi]
  pub fn u64(#[napi(ts_arg_type = "bigint")] value: Unknown) -> napi::Result<DynWin32Value> {
    Ok(DynWin32Value::new(dynwinrt::win32::Value::U64(
      storage::unsigned64(&value)?,
    )))
  }

  #[napi]
  pub fn f32(value: f64) -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::F32(value as f32))
  }

  #[napi]
  pub fn f64(value: f64) -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::F64(value))
  }

  #[napi]
  pub fn handle(
    #[napi(ts_arg_type = "bigint | number | DynWin32Resource | null | undefined")] value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    handle_value(value, nullable.unwrap_or(false))
  }

  #[napi]
  pub fn resource(
    value: &DynWin32Resource,
    #[napi(ts_arg_type = "string")] cleanup: specification::Name,
  ) -> napi::Result<DynWin32Value> {
    let cleanup = parse_cleanup(&cleanup)?;
    if cleanup == dynwinrt::win32::Cleanup::None || value.0.cleanup() != cleanup {
      return Err(napi::Error::from_reason(
        "Managed Win32 resource cleanup does not match the consuming API",
      ));
    }
    if value.0.is_closed() {
      return Err(napi::Error::from_reason(
        "Cannot consume a closed flat Win32 resource",
      ));
    }
    Ok(DynWin32Value::new(dynwinrt::win32::Value::Resource(
      Arc::clone(&value.0),
    )))
  }

  #[napi]
  pub fn com_object(
    #[napi(ts_arg_type = "DynWinRtValue | null | undefined")] value: Unknown,
    #[napi(ts_arg_type = "string")] iid: specification::Name,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    use windows::core::Interface;

    let env = value.value().env;
    let raw = value.value().value;
    let mut value_type = napi::sys::ValueType::napi_undefined;
    unsafe { napi::sys::napi_typeof(env, raw, &mut value_type) };
    if matches!(
      value_type,
      napi::sys::ValueType::napi_null | napi::sys::ValueType::napi_undefined
    ) {
      return if nullable.unwrap_or(false) {
        Ok(DynWin32Value::new(dynwinrt::win32::Value::Null))
      } else {
        Err(napi::Error::from_reason(
          "DynWin32.comObject(): null requires an explicitly nullable interface",
        ))
      };
    }
    if boundary::has_carrier(env, raw)? {
      return Err(napi::Error::from_reason(
        "Win32 handles and structs are not managed COM references",
      ));
    }
    let owner = boundary::with_managed_com_value(&value, |value| {
      let iid = WinGUID(
        windows::core::GUID::try_from(iid.as_str())
          .map_err(|_| napi::Error::from_reason("Invalid COM interface IID"))?,
      );
      com::with_win32_input_leases(&[value], |validate_inputs| {
        let owner = com::try_cast(value, &iid)?.ok_or_else(|| {
          napi::Error::from_reason("Managed object does not implement the required Win32 interface")
        })?;
        validate_inputs()?;
        Ok(owner)
      })
    })?;
    let pointer = owner
      .winrt()
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("Managed value is not a COM object"))?
      .as_raw();
    Ok(DynWin32Value {
      value: dynwinrt::win32::Value::Pointer(pointer),
      pointer_owner: Some(Win32PointerOwner::Native(Rc::new(
        RetainedNativePointer::Com(Box::new(owner)),
      ))),
    })
  }

  #[napi]
  pub fn data_pointer(
    #[napi(ts_arg_type = "Buffer | Uint8Array | null | undefined")] value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    let nullable = nullable.unwrap_or(false);
    let value = pointer_value(storage::data_pointer(value, nullable)?)?;
    reject_required_null_pointer(&value, nullable)?;
    Ok(value)
  }

  #[napi]
  pub fn buffer_length(
    #[napi(ts_arg_type = "Buffer | Uint8Array")] value: Unknown,
  ) -> napi::Result<u32> {
    Ok(storage::length(&value)? as u32)
  }

  #[napi]
  pub fn byte_length(
    #[napi(ts_arg_type = "Buffer | Uint8Array")] value: Unknown,
  ) -> napi::Result<u32> {
    Self::buffer_length(value)
  }

  #[napi]
  pub fn buffer_byte_length(
    #[napi(ts_arg_type = "Buffer | Uint8Array")] value: Unknown,
  ) -> napi::Result<u32> {
    Self::buffer_length(value)
  }

  #[napi]
  pub fn allocate_buffer(length: f64) -> napi::Result<Buffer> {
    Ok(Buffer::from(storage::zeroed(storage::checked_length(
      length,
    )?)?))
  }

  #[napi]
  pub fn copy_buffer(
    #[napi(ts_arg_type = "Buffer | Uint8Array")] value: Unknown,
    actual_length: Option<f64>,
  ) -> napi::Result<Buffer> {
    storage::copy(&value, actual_length)
  }

  #[napi]
  pub fn aligned_data_pointer(
    #[napi(ts_arg_type = "Buffer | Uint8Array | null | undefined")] value: Unknown,
    alignment: f64,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    check_integer(alignment, 1.0, 8.0, "alignedDataPointer alignment")?;
    let alignment = alignment as u32;
    if !alignment.is_power_of_two() {
      return Err(napi::Error::from_reason(
        "DynWin32.alignedDataPointer(): alignment must be 1, 2, 4, or 8",
      ));
    }
    let nullable = nullable.unwrap_or(false);
    let value = pointer_value(storage::data_pointer(value, nullable)?)?;
    reject_required_null_pointer(&value, nullable)?;
    if let dynwinrt::win32::Value::Pointer(pointer) = &value.value {
      if !pointer.is_null()
        && !crate::js_storage::address_is_aligned(*pointer as usize, alignment as usize)
      {
        return Err(napi::Error::from_reason(format!(
          "native buffer address is not aligned to {alignment} bytes"
        )));
      }
    }
    Ok(value)
  }

  #[napi]
  pub fn wide_string(
    #[napi(ts_arg_type = "string | Buffer | Uint8Array | null | undefined")] value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    string_value(
      storage::string_pointer(value, nullable.unwrap_or(false), true, false)?,
      true,
      false,
    )
  }

  #[napi]
  pub fn ansi_string(
    #[napi(ts_arg_type = "string | Buffer | Uint8Array | null | undefined")] value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    string_value(
      storage::string_pointer(value, nullable.unwrap_or(false), false, false)?,
      false,
      false,
    )
  }

  #[napi]
  pub fn wide_multi_string(
    #[napi(ts_arg_type = "string | readonly string[] | Buffer | Uint8Array | null | undefined")]
    value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    string_value(
      storage::string_pointer(value, nullable.unwrap_or(false), true, true)?,
      true,
      true,
    )
  }

  #[napi]
  pub fn ansi_multi_string(
    #[napi(ts_arg_type = "string | readonly string[] | Buffer | Uint8Array | null | undefined")]
    value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    string_value(
      storage::string_pointer(value, nullable.unwrap_or(false), false, true)?,
      false,
      true,
    )
  }

  #[napi]
  pub fn wide_string_pointer_pointer(
    #[napi(ts_arg_type = "string | Buffer | Uint8Array | null | undefined")] value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    string_pointer_pointer(value, nullable.unwrap_or(false), true)
  }

  #[napi]
  pub fn ansi_string_pointer_pointer(
    #[napi(ts_arg_type = "string | Buffer | Uint8Array | null | undefined")] value: Unknown,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    string_pointer_pointer(value, nullable.unwrap_or(false), false)
  }

  #[napi]
  pub fn null_pointer() -> DynWin32Value {
    DynWin32Value::new(dynwinrt::win32::Value::Null)
  }

  #[napi]
  pub fn begin_read_file(
    file: &DynWin32Resource,
    #[napi(ts_arg_type = "Buffer")] buffer: Unknown,
    #[napi(ts_arg_type = "bigint | null")] offset: Option<Unknown>,
  ) -> napi::Result<DynWin32OverlappedOperation> {
    io::prepare(dynwinrt::win32::io::IoKind::Read, file, buffer, offset)
  }

  #[napi]
  pub fn begin_write_file(
    file: &DynWin32Resource,
    #[napi(ts_arg_type = "Buffer")] buffer: Unknown,
    #[napi(ts_arg_type = "bigint | null")] offset: Option<Unknown>,
  ) -> napi::Result<DynWin32OverlappedOperation> {
    io::prepare(dynwinrt::win32::io::IoKind::Write, file, buffer, offset)
  }

  #[napi]
  pub fn create_native_struct(
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
    #[napi(ts_arg_type = "Buffer | Uint8Array | null")] bytes: Option<Unknown>,
  ) -> napi::Result<DynWin32NativeStruct> {
    let (_, size, alignment, contains_pointers, fields) = native_aggregate_layout(&descriptor)?;
    if alignment > 8 {
      return Err(napi::Error::from_reason(
        "flat Win32 native aggregate alignment above 8 is unsupported",
      ));
    }
    Ok(DynWin32NativeStruct {
      storage: Rc::new(NativeAggregateStorage::new(
        descriptor.as_str().to_string(),
        size,
        alignment,
        bytes
          .as_ref()
          .map(|bytes| storage::buffer_info(bytes.value().env, bytes.raw()))
          .transpose()?
          .as_ref()
          .map(|info| unsafe { info.bytes() }),
        contains_pointers,
        fields,
      )?),
      descriptor: descriptor.into_string(),
    })
  }

  #[napi]
  pub fn set_native_struct_u32(
    value: &DynWin32NativeStruct,
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
    #[napi(ts_arg_type = "string")] field: specification::Name,
    input: f64,
  ) -> napi::Result<()> {
    check_integer(input, 0.0, u32::MAX as f64, "native u32 field")?;
    validate_native_struct(value, &descriptor)?;
    let (offset, kind, _) = native_aggregate_field(&descriptor, &field)?;
    if kind != "u32" {
      return Err(napi::Error::from_reason(format!(
        "native field `{field}` is not u32"
      )));
    }
    value
      .storage
      .write_field(offset, &(input as u32).to_le_bytes(), None)
  }

  #[napi]
  pub fn set_native_struct_bool32(
    value: &DynWin32NativeStruct,
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
    #[napi(ts_arg_type = "string")] field: specification::Name,
    input: bool,
  ) -> napi::Result<()> {
    validate_native_struct(value, &descriptor)?;
    let (offset, kind, _) = native_aggregate_field(&descriptor, &field)?;
    if kind != "i32" {
      return Err(napi::Error::from_reason(format!(
        "native field `{field}` is not BOOL"
      )));
    }
    value
      .storage
      .write_field(offset, &i32::from(input).to_le_bytes(), None)
  }

  #[napi]
  pub fn set_native_struct_pointer(
    value: &DynWin32NativeStruct,
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
    #[napi(ts_arg_type = "string")] field: specification::Name,
    pointer: &DynWin32Value,
  ) -> napi::Result<()> {
    validate_native_struct(value, &descriptor)?;
    pointer.validate()?;
    let (offset, kind, _) = native_aggregate_field(&descriptor, &field)?;
    if kind != "pointer" {
      return Err(napi::Error::from_reason(format!(
        "native field `{field}` is not a data pointer"
      )));
    }
    let (bits, owner) = match (&pointer.value, &pointer.pointer_owner) {
      (dynwinrt::win32::Value::Null, _) => (0usize, None),
      (dynwinrt::win32::Value::Pointer(pointer), _) if pointer.is_null() => (0usize, None),
      (dynwinrt::win32::Value::Pointer(pointer), Some(Win32PointerOwner::Native(owner)))
        if owner.has_storage() =>
      {
        (*pointer as usize, Some(Rc::clone(owner)))
      }
      (dynwinrt::win32::Value::Pointer(_), _) => {
        return Err(napi::Error::from_reason(
          "native struct pointer fields require retained Buffer or string storage",
        ));
      }
      _ => {
        return Err(napi::Error::from_reason(
          "native struct pointer field value is not a data pointer",
        ));
      }
    };
    value
      .storage
      .write_field(offset, &bits.to_le_bytes(), owner)
  }

  #[napi]
  pub fn get_native_struct_u32(
    value: &DynWin32NativeStruct,
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
    #[napi(ts_arg_type = "string")] field: specification::Name,
  ) -> napi::Result<u32> {
    validate_native_struct(value, &descriptor)?;
    let (offset, kind, _) = native_aggregate_field(&descriptor, &field)?;
    if kind != "u32" {
      return Err(napi::Error::from_reason(format!(
        "native field `{field}` is not u32"
      )));
    }
    if let Some(index) = value.storage.backing.field_index(&field) {
      match value
        .storage
        .backing
        .field_value(index)
        .map_err(|error| napi::Error::from_reason(error.message()))?
      {
        dynwinrt::win32::Value::U32(_) => Ok(u32::from_le_bytes(value.storage.read_field(offset)?)),
        _ => Err(napi::Error::from_reason(
          "Native aggregate result field is not u32",
        )),
      }
    } else {
      value.storage.require_success()?;
      Ok(u32::from_le_bytes(value.storage.read_field(offset)?))
    }
  }

  #[napi]
  pub fn take_native_struct_resource(
    value: &DynWin32NativeStruct,
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
    #[napi(ts_arg_type = "string")] field: specification::Name,
    #[napi(ts_arg_type = "string")] cleanup: specification::Name,
  ) -> napi::Result<Option<DynWin32Resource>> {
    validate_native_struct(value, &descriptor)?;
    let (_, kind, field_cleanup) = native_aggregate_field(&descriptor, &field)?;
    if kind != "handle" || field_cleanup.as_deref() != Some(cleanup.as_str()) {
      return Err(napi::Error::from_reason(format!(
        "native field `{field}` does not have cleanup `{cleanup}`"
      )));
    }
    let cleanup = parse_cleanup(&cleanup)?;
    if cleanup == dynwinrt::win32::Cleanup::None {
      return Err(napi::Error::from_reason(
        "Native resource fields require exact ownership cleanup",
      ));
    }
    let index = value.storage.backing.field_index(&field).ok_or_else(|| {
      napi::Error::from_reason("Native handle field has no result ownership contract")
    })?;
    match value
      .storage
      .backing
      .take_field(index)
      .map_err(|error| napi::Error::from_reason(error.message()))?
    {
      dynwinrt::win32::Value::Resource(resource) if resource.cleanup() == cleanup => {
        Ok(Some(DynWin32Resource(resource)))
      }
      dynwinrt::win32::Value::Handle(0) => Ok(None),
      _ => Err(napi::Error::from_reason(
        "Native aggregate result is not an owned handle",
      )),
    }
  }

  /// # Safety
  ///
  /// Successful legacy fields must be defined native results transferring
  /// exclusive ownership with their declared cleanup; borrowed handles or
  /// caller-written pointer bytes alone do not satisfy this contract. The caller
  /// must uphold `dynwinrt::win32::NativeAggregateBuffer::mark_legacy`'s safety
  /// requirements, including completed native writes and no other owners.
  #[napi]
  pub unsafe fn mark_native_struct_call_result(
    value: &DynWin32NativeStruct,
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
    succeeded: bool,
  ) -> napi::Result<()> {
    validate_native_struct(value, &descriptor)?;
    unsafe { value.storage.mark_call_result(succeeded) }
  }

  #[napi]
  pub fn prepare_native_struct_call(
    value: &DynWin32NativeStruct,
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
  ) -> napi::Result<()> {
    validate_native_struct(value, &descriptor)?;
    value.storage.prepare_call()
  }

  #[napi]
  pub fn native_struct(
    #[napi(ts_arg_type = "DynWin32NativeStruct | null | undefined")] value: Unknown,
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
    nullable: Option<bool>,
  ) -> napi::Result<DynWin32Value> {
    let env = value.value().env;
    let raw = value.value().value;
    let mut value_type = napi::sys::ValueType::napi_undefined;
    unsafe { napi::sys::napi_typeof(env, raw, &mut value_type) };
    if matches!(
      value_type,
      napi::sys::ValueType::napi_null | napi::sys::ValueType::napi_undefined
    ) {
      return if nullable.unwrap_or(false) {
        Ok(DynWin32Value::new(dynwinrt::win32::Value::Null))
      } else {
        Err(napi::Error::from_reason(
          "DynWin32.nativeStruct(): null requires an explicitly nullable aggregate pointer",
        ))
      };
    }
    let value = unsafe { <&DynWin32NativeStruct>::from_napi_value(env, raw) }?;
    if value.descriptor != descriptor.as_str() {
      return Err(napi::Error::from_reason(
        "DynWin32.nativeStruct(): native aggregate type mismatch",
      ));
    }
    Ok(DynWin32Value {
      value: dynwinrt::win32::Value::AggregatePointer(Arc::clone(&value.storage.backing)),
      pointer_owner: Some(Win32PointerOwner::Aggregate(Rc::clone(&value.storage))),
    })
  }

  #[napi]
  pub fn native_struct_value(
    value: &DynWin32NativeStruct,
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
  ) -> napi::Result<DynWin32Value> {
    if value.descriptor != descriptor.as_str() {
      return Err(napi::Error::from_reason(
        "DynWin32.nativeStructValue(): native aggregate type mismatch",
      ));
    }
    let layout = native_aggregate_call_layout(&descriptor)?;
    Ok(DynWin32Value {
      value: dynwinrt::win32::Value::Aggregate {
        layout,
        pointer: value.storage.pointer(),
      },
      pointer_owner: Some(Win32PointerOwner::Aggregate(Rc::clone(&value.storage))),
    })
  }

  #[napi]
  pub fn to_native_struct(
    value: &DynWin32Value,
    #[napi(ts_arg_type = "string")] descriptor: specification::Descriptor,
  ) -> napi::Result<DynWin32NativeStruct> {
    let dynwinrt::win32::Value::OwnedAggregate { layout, bytes } = &value.value else {
      return Err(napi::Error::from_reason(
        "Win32 value is not an owned native aggregate",
      ));
    };
    if layout.identity() != descriptor.as_str() {
      return Err(napi::Error::from_reason(
        "native aggregate return identity mismatch",
      ));
    }
    let (_, _, alignment, _, fields) = native_aggregate_layout(&descriptor)?;
    Ok(DynWin32NativeStruct {
      storage: Rc::new(NativeAggregateStorage::new(
        descriptor.as_str().to_string(),
        bytes.len(),
        alignment,
        Some(bytes),
        false,
        fields,
      )?),
      descriptor: descriptor.into_string(),
    })
  }

  #[napi]
  pub fn to_number(value: &DynWin32Value) -> napi::Result<f64> {
    Ok(match &value.value {
      dynwinrt::win32::Value::Bool(value) => u8::from(*value) as f64,
      dynwinrt::win32::Value::I8(value) => *value as f64,
      dynwinrt::win32::Value::U8(value) => *value as f64,
      dynwinrt::win32::Value::I16(value) => *value as f64,
      dynwinrt::win32::Value::U16(value) => *value as f64,
      dynwinrt::win32::Value::I32(value) => *value as f64,
      dynwinrt::win32::Value::U32(value) => *value as f64,
      dynwinrt::win32::Value::F32(value) => *value as f64,
      dynwinrt::win32::Value::F64(value) => *value,
      _ => {
        return Err(napi::Error::from_reason(
          "Win32 value is not a JavaScript number",
        ));
      }
    })
  }

  #[napi]
  pub fn to_bigint(value: &DynWin32Value) -> napi::Result<BigInt> {
    match &value.value {
      dynwinrt::win32::Value::I64(value) => Ok(BigInt::from(*value)),
      dynwinrt::win32::Value::U64(value) => Ok(BigInt::from(*value)),
      dynwinrt::win32::Value::FunctionPointer(value) => Ok(BigInt::from(*value as u64)),
      dynwinrt::win32::Value::Handle(value) => Ok(BigInt::from(*value as u64)),
      dynwinrt::win32::Value::Resource(value) => Ok(BigInt::from(value.raw() as u64)),
      dynwinrt::win32::Value::Null => Ok(BigInt::from(0u64)),
      _ => Err(napi::Error::from_reason(
        "Win32 value is not a 64-bit integer, pointer, or handle",
      )),
    }
  }

  #[napi]
  pub fn to_boolean(value: &DynWin32Value) -> napi::Result<bool> {
    match value.value {
      dynwinrt::win32::Value::Bool(value) => Ok(value),
      dynwinrt::win32::Value::U8(value) => Ok(value != 0),
      _ => Err(napi::Error::from_reason("Win32 value is not BOOL")),
    }
  }

  #[napi]
  pub fn to_resource(value: &DynWin32Value) -> napi::Result<Option<DynWin32Resource>> {
    if matches!(
      &value.value,
      dynwinrt::win32::Value::Handle(0)
        | dynwinrt::win32::Value::Null
        | dynwinrt::win32::Value::Discarded
    ) {
      return Ok(None);
    }
    value
      .value
      .resource()
      .cloned()
      .map(DynWin32Resource)
      .map(Some)
      .ok_or_else(|| napi::Error::from_reason("Win32 value is not an owned resource"))
  }

  #[napi]
  pub fn is_unavailable(value: &DynWin32Value) -> bool {
    matches!(
      value.value,
      dynwinrt::win32::Value::Unavailable | dynwinrt::win32::Value::Discarded
    )
  }

  #[napi]
  pub fn to_resource_or_handle(
    value: &DynWin32Value,
  ) -> napi::Result<Option<Either<DynWin32Resource, BigInt>>> {
    match &value.value {
      dynwinrt::win32::Value::Resource(resource) => {
        Ok(Some(Either::A(DynWin32Resource(Arc::clone(resource)))))
      }
      dynwinrt::win32::Value::Handle(0)
      | dynwinrt::win32::Value::Null
      | dynwinrt::win32::Value::Discarded => Ok(None),
      dynwinrt::win32::Value::Handle(bits) => Ok(Some(Either::B(BigInt::from(*bits as u64)))),
      _ => Err(napi::Error::from_reason(
        "Win32 value is not a resource or borrowed handle",
      )),
    }
  }
}

fn reject_required_null_pointer(value: &DynWin32Value, nullable: bool) -> napi::Result<()> {
  if !nullable
    && matches!(
      &value.value,
      dynwinrt::win32::Value::Pointer(pointer) if pointer.is_null()
    )
  {
    return Err(napi::Error::from_reason(
      "non-nullable native pointer requires non-empty backing storage",
    ));
  }
  Ok(())
}

fn validate_native_struct(value: &DynWin32NativeStruct, descriptor: &str) -> napi::Result<()> {
  if value.descriptor != descriptor {
    return Err(napi::Error::from_reason("native aggregate type mismatch"));
  }
  Ok(())
}

fn native_aggregate_field(
  descriptor: &str,
  field: &str,
) -> napi::Result<(usize, String, Option<String>)> {
  let root = parse_native_aggregate_descriptor(descriptor)?;
  #[cfg(target_arch = "x86")]
  let architecture = "x86";
  #[cfg(target_arch = "x86_64")]
  let architecture = "x64";
  #[cfg(target_arch = "aarch64")]
  let architecture = "arm64";
  #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
  return Err(napi::Error::from_reason(
    "flat Win32 native aggregates support only x86, x64, and ARM64",
  ));
  let fields = root
    .get(architecture)
    .and_then(|layout| layout.get("fields"))
    .and_then(serde_json::Value::as_array)
    .ok_or_else(|| napi::Error::from_reason("native aggregate descriptor has no fields"))?;
  let field = fields
    .iter()
    .find(|candidate| candidate.get("name").and_then(serde_json::Value::as_str) == Some(field))
    .ok_or_else(|| napi::Error::from_reason(format!("unknown native field `{field}`")))?;
  let offset = field
    .get("offset")
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .ok_or_else(|| napi::Error::from_reason("native field has invalid offset"))?;
  let kind = field
    .get("type")
    .and_then(|typ| typ.get("kind"))
    .and_then(serde_json::Value::as_str)
    .ok_or_else(|| napi::Error::from_reason("native field has invalid type"))?;
  let cleanup = field
    .get("type")
    .and_then(|typ| typ.get("cleanup"))
    .and_then(serde_json::Value::as_str)
    .map(str::to_string);
  Ok((offset, kind.to_string(), cleanup))
}

fn native_layout_extent(layout: &serde_json::Value) -> napi::Result<(usize, usize)> {
  let size = layout
    .get("size")
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .filter(|value| *value > 0 && *value <= dynwinrt::win32::MAX_NATIVE_AGGREGATE_SIZE)
    .ok_or_else(|| napi::Error::from_reason("Native aggregate size exceeds its safety limit"))?;
  let alignment = layout
    .get("alignment")
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .filter(|value| value.is_power_of_two() && *value <= 8 && size % *value == 0)
    .ok_or_else(|| napi::Error::from_reason("Native aggregate alignment is invalid"))?;
  Ok((size, alignment))
}

fn validate_native_layout(layout: &serde_json::Value, union: bool) -> napi::Result<()> {
  let (size, _) = native_layout_extent(layout)?;
  let fields = layout
    .get("fields")
    .and_then(serde_json::Value::as_array)
    .ok_or_else(|| napi::Error::from_reason("Native aggregate is missing fields"))?;
  let mut ranges = Vec::new();
  ranges
    .try_reserve_exact(fields.len())
    .map_err(|_| napi::Error::from_reason("Unable to allocate native layout validation"))?;
  let mut names = std::collections::HashSet::new();
  names
    .try_reserve(fields.len())
    .map_err(|_| napi::Error::from_reason("Unable to allocate native field identities"))?;
  for field in fields {
    let name = field
      .get("name")
      .and_then(serde_json::Value::as_str)
      .filter(|name| !name.is_empty())
      .ok_or_else(|| napi::Error::from_reason("Native field requires an exact name"))?;
    if !names.insert(name) {
      return Err(napi::Error::from_reason("Duplicate native field name"));
    }
    let offset = field
      .get("offset")
      .and_then(serde_json::Value::as_u64)
      .and_then(|value| usize::try_from(value).ok())
      .ok_or_else(|| napi::Error::from_reason("Native field offset is invalid"))?;
    let count = field
      .get("count")
      .and_then(serde_json::Value::as_u64)
      .and_then(|value| usize::try_from(value).ok())
      .filter(|value| *value > 0)
      .ok_or_else(|| napi::Error::from_reason("Native field count is invalid"))?;
    let typ = field
      .get("type")
      .ok_or_else(|| napi::Error::from_reason("Native field type is missing"))?;
    let kind = typ
      .get("kind")
      .and_then(serde_json::Value::as_str)
      .ok_or_else(|| napi::Error::from_reason("Native field kind is missing"))?;
    let field_size = match kind {
      "i8" | "u8" => 1,
      "i16" | "u16" => 2,
      "i32" | "u32" | "f32" => 4,
      "i64" | "u64" | "f64" => 8,
      "isize" | "usize" => std::mem::size_of::<usize>(),
      "guid" => 16,
      "pointer" | "handle" => {
        if count != 1 || union {
          return Err(napi::Error::from_reason(
            "Native pointer/resource arrays and unions require ownership contracts",
          ));
        }
        if kind == "handle" {
          parse_cleanup(
            typ
              .get("cleanup")
              .and_then(serde_json::Value::as_str)
              .ok_or_else(|| {
                napi::Error::from_reason("Native handle field has no cleanup contract")
              })?,
          )?;
        }
        std::mem::size_of::<usize>()
      }
      "struct" | "union" => {
        let nested = typ
          .get("layout")
          .ok_or_else(|| napi::Error::from_reason("Nested native field layout is missing"))?;
        validate_native_layout(nested, kind == "union")?;
        if native_layout_contains_pointers(nested)? {
          return Err(napi::Error::from_reason(
            "Nested pointer ownership is not modeled",
          ));
        }
        native_layout_extent(nested)?.0
      }
      _ => {
        return Err(napi::Error::from_reason(format!(
          "Unsupported native field kind `{kind}`"
        )))
      }
    };
    let end = field_size
      .checked_mul(count)
      .and_then(|length| offset.checked_add(length))
      .filter(|end| *end <= size)
      .ok_or_else(|| napi::Error::from_reason("Native field exceeds aggregate size"))?;
    if union && offset != 0 {
      return Err(napi::Error::from_reason(
        "Native union fields must overlap at offset zero",
      ));
    }
    ranges.push((offset, end));
  }
  if !union {
    ranges.sort_unstable();
    if ranges.windows(2).any(|fields| fields[0].1 > fields[1].0) {
      return Err(napi::Error::from_reason("Native struct fields overlap"));
    }
  }
  Ok(())
}

fn reserve_ffi_elements(
  elements: &mut Vec<libffi::middle::Type>,
  count: usize,
) -> napi::Result<()> {
  const MAX_ELEMENTS: usize =
    dynwinrt::win32::MAX_NATIVE_AGGREGATE_SIZE / std::mem::size_of::<libffi::middle::Type>();
  if elements
    .len()
    .checked_add(count)
    .is_none_or(|length| length > MAX_ELEMENTS)
  {
    return Err(napi::Error::from_reason(
      "Native aggregate libffi type graph exceeds its safety limit",
    ));
  }
  elements
    .try_reserve_exact(count)
    .map_err(|_| napi::Error::from_reason("Unable to allocate native aggregate libffi fields"))
}

fn native_aggregate_layout(
  descriptor: &str,
) -> napi::Result<(
  String,
  usize,
  usize,
  bool,
  Vec<dynwinrt::win32::AggregateResultField>,
)> {
  let root = parse_native_aggregate_descriptor(descriptor)?;
  let name = root
    .get("name")
    .and_then(serde_json::Value::as_str)
    .ok_or_else(|| napi::Error::from_reason("native aggregate descriptor is missing `name`"))?;
  #[cfg(target_arch = "x86")]
  let architecture = "x86";
  #[cfg(target_arch = "x86_64")]
  let architecture = "x64";
  #[cfg(target_arch = "aarch64")]
  let architecture = "arm64";
  #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
  return Err(napi::Error::from_reason(
    "flat Win32 native aggregates support only x86, x64, and ARM64",
  ));
  let layout = root.get(architecture).ok_or_else(|| {
    napi::Error::from_reason(format!(
      "native aggregate descriptor is missing `{architecture}`"
    ))
  })?;
  let union = match root.get("kind").and_then(serde_json::Value::as_str) {
    Some("struct") => false,
    Some("union") => true,
    _ => {
      return Err(napi::Error::from_reason(
        "Native aggregate requires struct or union kind",
      ))
    }
  };
  validate_native_layout(layout, union)?;
  let size = layout
    .get("size")
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .filter(|value| *value > 0)
    .ok_or_else(|| napi::Error::from_reason("native aggregate has invalid `size`"))?;
  if size > dynwinrt::win32::MAX_NATIVE_AGGREGATE_SIZE {
    return Err(napi::Error::from_reason(format!(
      "native aggregate exceeds the {} byte safety limit",
      dynwinrt::win32::MAX_NATIVE_AGGREGATE_SIZE
    )));
  }
  let alignment = layout
    .get("alignment")
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .filter(|value| value.is_power_of_two() && size % *value == 0)
    .ok_or_else(|| napi::Error::from_reason("native aggregate has invalid `alignment`"))?;
  Ok((
    name.to_string(),
    size,
    alignment,
    native_layout_contains_pointers(layout)?,
    native_layout_result_fields(&root, layout)?,
  ))
}

fn native_aggregate_pointer_layout(
  descriptor: &str,
) -> napi::Result<Arc<dynwinrt::win32::NativeAggregatePointerLayout>> {
  let (_, size, alignment, _, fields) = native_aggregate_layout(descriptor)?;
  dynwinrt::win32::NativeAggregatePointerLayout::new(
    descriptor.to_string(),
    size,
    alignment,
    fields,
  )
  .map_err(|error| napi::Error::from_reason(error.message()))
}

fn native_layout_result_fields(
  root: &serde_json::Value,
  layout: &serde_json::Value,
) -> napi::Result<Vec<dynwinrt::win32::AggregateResultField>> {
  let fields = layout
    .get("fields")
    .and_then(serde_json::Value::as_array)
    .ok_or_else(|| napi::Error::from_reason("Native aggregate fields are missing"))?;
  let owned = fields
    .iter()
    .filter_map(|field| {
      let typ = field.get("type")?;
      (typ.get("kind").and_then(serde_json::Value::as_str) == Some("handle")
        && typ
          .get("cleanup")
          .and_then(serde_json::Value::as_str)
          .is_some_and(|cleanup| cleanup != "none"))
      .then(|| field.get("name").and_then(serde_json::Value::as_str))
      .flatten()
    })
    .collect::<Vec<_>>();
  let names = match root.get("outputFields") {
    None => owned.clone(),
    Some(value) => value
      .as_array()
      .ok_or_else(|| napi::Error::from_reason("outputFields must be an array"))?
      .iter()
      .map(|value| {
        value
          .as_str()
          .ok_or_else(|| napi::Error::from_reason("outputFields must contain field names"))
      })
      .collect::<napi::Result<Vec<_>>>()?,
  };
  if names.len() > 1024 || owned.iter().any(|name| !names.contains(name)) {
    return Err(napi::Error::from_reason(
      "Native result fields omit owned handles or exceed the field limit",
    ));
  }
  if !names.is_empty() && root.get("kind").and_then(serde_json::Value::as_str) != Some("struct") {
    return Err(napi::Error::from_reason(
      "Union result ownership requires an explicit active-arm contract",
    ));
  }
  let mut seen = std::collections::BTreeSet::new();
  names
    .into_iter()
    .map(|name| {
      if !seen.insert(name) {
        return Err(napi::Error::from_reason(
          "Duplicate native aggregate output field",
        ));
      }
      let field = fields
        .iter()
        .find(|field| field.get("name").and_then(serde_json::Value::as_str) == Some(name))
        .ok_or_else(|| napi::Error::from_reason("Unknown native aggregate output field"))?;
      if field.get("count").and_then(serde_json::Value::as_u64) != Some(1) {
        return Err(napi::Error::from_reason(
          "Aggregate result arrays require an element ownership contract",
        ));
      }
      let typ = field
        .get("type")
        .ok_or_else(|| napi::Error::from_reason("Native output field type is missing"))?;
      let kind = typ
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| napi::Error::from_reason("Native output field kind is missing"))?;
      let abi = match kind {
        "isize" => {
          if cfg!(target_pointer_width = "64") {
            dynwinrt::win32::Type::I64
          } else {
            dynwinrt::win32::Type::I32
          }
        }
        "usize" => {
          if cfg!(target_pointer_width = "64") {
            dynwinrt::win32::Type::U64
          } else {
            dynwinrt::win32::Type::U32
          }
        }
        _ => parse_type(kind)?,
      };
      let cleanup = typ
        .get("cleanup")
        .and_then(serde_json::Value::as_str)
        .map(parse_cleanup)
        .transpose()?
        .unwrap_or(dynwinrt::win32::Cleanup::None);
      let offset = field
        .get("offset")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| napi::Error::from_reason("Native output field offset is invalid"))?;
      Ok(dynwinrt::win32::AggregateResultField {
        name: name.to_owned(),
        offset,
        typ: abi,
        cleanup,
      })
    })
    .collect()
}

fn native_layout_contains_pointers(layout: &serde_json::Value) -> napi::Result<bool> {
  let mut contains = false;
  for field in layout
    .get("fields")
    .and_then(serde_json::Value::as_array)
    .ok_or_else(|| napi::Error::from_reason("Native aggregate is missing fields"))?
  {
    let typ = field
      .get("type")
      .ok_or_else(|| napi::Error::from_reason("Native field is missing type"))?;
    let kind = typ
      .get("kind")
      .and_then(serde_json::Value::as_str)
      .ok_or_else(|| napi::Error::from_reason("Native field is missing kind"))?;
    contains |= match kind {
      "pointer" | "handle" => true,
      "struct" | "union" => native_layout_contains_pointers(
        typ
          .get("layout")
          .ok_or_else(|| napi::Error::from_reason("Nested native layout is missing"))?,
      )?,
      "i8" | "u8" | "i16" | "u16" | "i32" | "u32" | "i64" | "u64" | "isize" | "usize" | "f32"
      | "f64" | "guid" => false,
      _ => {
        return Err(napi::Error::from_reason(format!(
          "Unsupported native field kind `{kind}`"
        )))
      }
    };
  }
  Ok(contains)
}

fn native_aggregate_call_layout(
  descriptor: &str,
) -> napi::Result<Arc<dynwinrt::win32::NativeAggregateLayout>> {
  let root = parse_native_aggregate_descriptor(descriptor)?;
  if root.get("kind").and_then(serde_json::Value::as_str) != Some("struct") {
    return Err(napi::Error::from_reason(
      "Only plain native structs can be passed by value",
    ));
  }
  #[cfg(target_arch = "x86")]
  let architecture = "x86";
  #[cfg(target_arch = "x86_64")]
  let architecture = "x64";
  #[cfg(target_arch = "aarch64")]
  let architecture = "arm64";
  #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
  return Err(napi::Error::from_reason(
    "flat Win32 native aggregates support only x86, x64, and ARM64",
  ));
  let layout = root.get(architecture).ok_or_else(|| {
    napi::Error::from_reason(format!(
      "native aggregate descriptor is missing `{architecture}`"
    ))
  })?;
  validate_native_layout(layout, false)?;
  if native_layout_contains_pointers(layout)? {
    return Err(napi::Error::from_reason(
      "Pointer-bearing native aggregates cannot be passed by value",
    ));
  }
  let size = layout
    .get("size")
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .ok_or_else(|| napi::Error::from_reason("native aggregate has invalid `size`"))?;
  let alignment = layout
    .get("alignment")
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .ok_or_else(|| napi::Error::from_reason("native aggregate has invalid `alignment`"))?;
  let ffi_type = native_aggregate_ffi_type(layout)?;
  dynwinrt::win32::NativeAggregateLayout::new(descriptor, size, alignment, ffi_type)
    .map_err(|error| napi::Error::from_reason(error.message()))
}

fn parse_native_aggregate_descriptor(descriptor: &str) -> napi::Result<serde_json::Value> {
  if descriptor.len() > MAX_NATIVE_AGGREGATE_DESCRIPTOR_LENGTH {
    return Err(napi::Error::from_reason(format!(
      "flat Win32 native aggregate descriptor exceeds the {MAX_NATIVE_AGGREGATE_DESCRIPTOR_LENGTH} byte safety limit"
    )));
  }
  serde_json::from_str(descriptor).map_err(|error| {
    napi::Error::from_reason(format!(
      "Invalid flat Win32 native aggregate descriptor: {error}"
    ))
  })
}

fn native_aggregate_ffi_type(layout: &serde_json::Value) -> napi::Result<libffi::middle::Type> {
  validate_native_layout(layout, false)?;
  let size = layout
    .get("size")
    .and_then(serde_json::Value::as_u64)
    .and_then(|value| usize::try_from(value).ok())
    .ok_or_else(|| napi::Error::from_reason("native aggregate has invalid `size`"))?;
  let mut fields = layout
    .get("fields")
    .and_then(serde_json::Value::as_array)
    .ok_or_else(|| napi::Error::from_reason("native aggregate is missing `fields`"))?
    .iter()
    .collect::<Vec<_>>();
  fields.sort_by_key(|field| {
    field
      .get("offset")
      .and_then(serde_json::Value::as_u64)
      .unwrap_or(u64::MAX)
  });
  let mut elements = Vec::new();
  let mut cursor = 0usize;
  for field in fields {
    let offset = field
      .get("offset")
      .and_then(serde_json::Value::as_u64)
      .and_then(|value| usize::try_from(value).ok())
      .ok_or_else(|| napi::Error::from_reason("native field has invalid `offset`"))?;
    if offset < cursor {
      return Err(napi::Error::from_reason(
        "overlapping native fields cannot be passed by value",
      ));
    }
    if offset > size {
      return Err(napi::Error::from_reason(
        "Native field offset exceeds aggregate size",
      ));
    }
    reserve_ffi_elements(&mut elements, offset - cursor)?;
    elements.extend(std::iter::repeat_with(libffi::middle::Type::u8).take(offset - cursor));
    let count = field
      .get("count")
      .and_then(serde_json::Value::as_u64)
      .and_then(|value| usize::try_from(value).ok())
      .filter(|value| *value > 0)
      .ok_or_else(|| napi::Error::from_reason("native field has invalid `count`"))?;
    let typ = field
      .get("type")
      .ok_or_else(|| napi::Error::from_reason("native field is missing `type`"))?;
    let (field_type, field_size) = native_ffi_field_type(typ)?;
    let end = offset
      .checked_add(
        field_size
          .checked_mul(count)
          .ok_or_else(|| napi::Error::from_reason("native aggregate field size overflow"))?,
      )
      .ok_or_else(|| napi::Error::from_reason("native aggregate field end overflow"))?;
    if end > size {
      return Err(napi::Error::from_reason(
        "Native field exceeds aggregate size",
      ));
    }
    reserve_ffi_elements(&mut elements, count)?;
    for _ in 0..count {
      elements.push(field_type.clone());
    }
    cursor = end;
  }
  if cursor > size {
    return Err(napi::Error::from_reason(
      "native aggregate fields exceed declared size",
    ));
  }
  if elements.is_empty() {
    return Err(napi::Error::from_reason(
      "By-value native aggregates require exact field ABI types",
    ));
  }
  reserve_ffi_elements(&mut elements, size - cursor)?;
  elements.extend(std::iter::repeat_with(libffi::middle::Type::u8).take(size - cursor));
  Ok(libffi::middle::Type::structure(elements))
}

fn native_ffi_field_type(typ: &serde_json::Value) -> napi::Result<(libffi::middle::Type, usize)> {
  let kind = typ
    .get("kind")
    .and_then(serde_json::Value::as_str)
    .ok_or_else(|| napi::Error::from_reason("native field type is missing `kind`"))?;
  Ok(match kind {
    "i8" => (libffi::middle::Type::i8(), 1),
    "u8" => (libffi::middle::Type::u8(), 1),
    "i16" => (libffi::middle::Type::i16(), 2),
    "u16" => (libffi::middle::Type::u16(), 2),
    "i32" => (libffi::middle::Type::i32(), 4),
    "u32" => (libffi::middle::Type::u32(), 4),
    "i64" => (libffi::middle::Type::i64(), 8),
    "u64" => (libffi::middle::Type::u64(), 8),
    "isize" if std::mem::size_of::<usize>() == 4 => (libffi::middle::Type::i32(), 4),
    "usize" if std::mem::size_of::<usize>() == 4 => (libffi::middle::Type::u32(), 4),
    "isize" => (libffi::middle::Type::i64(), 8),
    "usize" => (libffi::middle::Type::u64(), 8),
    "f32" => (libffi::middle::Type::f32(), 4),
    "f64" => (libffi::middle::Type::f64(), 8),
    "pointer" => (
      libffi::middle::Type::pointer(),
      std::mem::size_of::<usize>(),
    ),
    "guid" => {
      let mut fields = vec![
        libffi::middle::Type::u32(),
        libffi::middle::Type::u16(),
        libffi::middle::Type::u16(),
      ];
      fields.extend(std::iter::repeat_with(libffi::middle::Type::u8).take(8));
      (libffi::middle::Type::structure(fields), 16)
    }
    "struct" => {
      let layout = typ
        .get("layout")
        .ok_or_else(|| napi::Error::from_reason("nested struct is missing `layout`"))?;
      let size = layout
        .get("size")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| napi::Error::from_reason("nested struct has invalid `size`"))?;
      (native_aggregate_ffi_type(layout)?, size)
    }
    "union" => {
      return Err(napi::Error::from_reason(
        "nested unions cannot be passed by value safely",
      ));
    }
    _ => {
      return Err(napi::Error::from_reason(format!(
        "unsupported native aggregate field kind `{kind}`"
      )));
    }
  })
}

native_class! {
  pub struct DynWin32Unsafe;
}

#[napi]
impl DynWin32Unsafe {
  #[napi]
  pub fn bind(
    #[napi(ts_arg_type = "DynWin32FunctionSpec")] spec: Unknown,
  ) -> napi::Result<DynWin32Function> {
    bind_function(specification::function_spec(spec)?)
  }

  #[napi]
  pub fn pointer(
    #[napi(ts_arg_type = "bigint | number | Buffer | Uint8Array | null | undefined")]
    value: Unknown,
  ) -> napi::Result<DynWin32Value> {
    pointer_value(storage::unsafe_pointer(value)?)
  }

  #[napi]
  pub fn pointer_address(value: &DynWin32Value) -> napi::Result<BigInt> {
    value.validate()?;
    match &value.value {
      dynwinrt::win32::Value::Pointer(pointer) => Ok(BigInt::from(*pointer as usize as u64)),
      dynwinrt::win32::Value::AggregatePointer(buffer) => {
        Ok(BigInt::from(buffer.pointer() as usize as u64))
      }
      dynwinrt::win32::Value::Null => Ok(BigInt::from(0u64)),
      _ => Err(napi::Error::from_reason(
        "DynWin32Unsafe.pointerAddress(): value is not a data pointer",
      )),
    }
  }
}

fn bind_function(spec: DynWin32FunctionSpec) -> napi::Result<DynWin32Function> {
  let mut parameter_aggregates = Vec::with_capacity(spec.parameters.len());
  let mut parameter_pointees = Vec::with_capacity(spec.parameters.len());
  let parameters = spec
    .parameters
    .into_iter()
    .map(|parameter| {
      let aggregate = parameter
        .aggregate_descriptor
        .as_deref()
        .map(native_aggregate_call_layout)
        .transpose()?;
      let pointee = parameter
        .pointee_descriptor
        .as_deref()
        .map(native_aggregate_pointer_layout)
        .transpose()?;
      let typ = if aggregate.is_some() {
        dynwinrt::win32::Type::Pointer
      } else {
        parse_type(&parameter.typ)?
      };
      parameter_aggregates.push(aggregate);
      parameter_pointees.push(pointee);
      Ok(dynwinrt::win32::Parameter {
        typ,
        direction: parse_direction(&parameter.direction)?,
        nullable: parameter.nullable.unwrap_or(false),
        cleanup: parse_cleanup(parameter.cleanup.as_deref().unwrap_or("none"))?,
        consumes_resource: parameter.consumes_resource.unwrap_or(false),
        resource_cleanup: parse_cleanup(parameter.resource_cleanup.as_deref().unwrap_or("none"))?,
      })
    })
    .collect::<napi::Result<Vec<_>>>()?;
  let return_aggregate = spec
    .return_aggregate_descriptor
    .as_deref()
    .map(native_aggregate_call_layout)
    .transpose()?;
  let return_type = spec
    .return_type
    .as_deref()
    .filter(|value| !value.eq_ignore_ascii_case("void"))
    .map(parse_type)
    .transpose()?
    .filter(|_| return_aggregate.is_none());
  let plan = unsafe {
    dynwinrt::win32::CallPlan::new_with_described_pointees(
      dynwinrt::win32::CallPlanSpec {
        dll: spec.dll,
        entry_point: spec.entry_point,
        parameters,
        return_type,
        return_cleanup: parse_cleanup(spec.return_cleanup.as_deref().unwrap_or("none"))?,
        success_rule: parse_success_rule(spec.success_rule.as_deref().unwrap_or("always"))?,
        capture_last_error: spec.capture_last_error.unwrap_or(false),
        calling_convention: parse_calling_convention(
          spec.calling_convention.as_deref().unwrap_or("system"),
        )?,
        parameter_aggregates,
        return_aggregate,
      },
      spec.call_contract_descriptor.as_deref(),
      parameter_pointees,
    )
  }
  .map_err(|error| napi::Error::from_reason(error.message()))?;
  Ok(DynWin32Function(plan))
}

fn parse_type(value: &str) -> napi::Result<dynwinrt::win32::Type> {
  use dynwinrt::win32::Type;
  match value.to_ascii_lowercase().as_str() {
    "bool32" | "bool" => Ok(Type::Bool32),
    "i8" => Ok(Type::I8),
    "u8" => Ok(Type::U8),
    "i16" => Ok(Type::I16),
    "u16" | "char16" => Ok(Type::U16),
    "i32" => Ok(Type::I32),
    "u32" => Ok(Type::U32),
    "i64" => Ok(Type::I64),
    "u64" => Ok(Type::U64),
    "f32" => Ok(Type::F32),
    "f64" => Ok(Type::F64),
    "pointer" | "ptr" => Ok(Type::Pointer),
    "functionpointer" | "function_pointer" => Ok(Type::FunctionPointer),
    "handle" => Ok(Type::Handle),
    _ => Err(napi::Error::from_reason(format!(
      "Unsupported flat Win32 ABI type `{value}`"
    ))),
  }
}

fn parse_direction(value: &str) -> napi::Result<dynwinrt::win32::Direction> {
  use dynwinrt::win32::Direction;
  match value.to_ascii_lowercase().as_str() {
    "in" => Ok(Direction::In),
    "out" => Ok(Direction::Out),
    "inout" | "in_out" | "in,out" => Ok(Direction::InOut),
    _ => Err(napi::Error::from_reason(format!(
      "Unsupported flat Win32 parameter direction `{value}`"
    ))),
  }
}

fn parse_cleanup(value: &str) -> napi::Result<dynwinrt::win32::Cleanup> {
  use dynwinrt::win32::Cleanup;
  match value.to_ascii_lowercase().as_str() {
    "none" => Ok(Cleanup::None),
    "closehandle" => Ok(Cleanup::CloseHandle),
    "regclosekey" => Ok(Cleanup::RegCloseKey),
    "localfree" => Ok(Cleanup::LocalFree),
    "globalfree" => Ok(Cleanup::GlobalFree),
    "freelibrary" => Ok(Cleanup::FreeLibrary),
    "closeservicehandle" => Ok(Cleanup::CloseServiceHandle),
    "cotaskmemfree" => Ok(Cleanup::CoTaskMemFree),
    "credfree" => Ok(Cleanup::CredFree),
    _ => Err(napi::Error::from_reason(format!(
      "Unsupported flat Win32 cleanup `{value}`"
    ))),
  }
}

fn parse_success_rule(value: &str) -> napi::Result<dynwinrt::win32::SuccessRule> {
  use dynwinrt::win32::SuccessRule;
  match value.to_ascii_lowercase().as_str() {
    "always" => Ok(SuccessRule::Always),
    "zero" | "returnzero" => Ok(SuccessRule::ReturnZero),
    "nonzero" | "returnnonzero" => Ok(SuccessRule::ReturnNonZero),
    "nonnull" | "returnnonnull" => Ok(SuccessRule::ReturnNonNull),
    "hresult" | "hresultsucceeded" => Ok(SuccessRule::HResultSucceeded),
    "signednonnegative" | "nonnegative" => Ok(SuccessRule::SignedNonNegative),
    "validhandle" | "returnvalidhandle" => Ok(SuccessRule::ReturnValidHandle),
    _ => Err(napi::Error::from_reason(format!(
      "Unsupported flat Win32 success rule `{value}`"
    ))),
  }
}

fn parse_calling_convention(value: &str) -> napi::Result<dynwinrt::win32::CallingConvention> {
  use dynwinrt::win32::CallingConvention;
  match value.to_ascii_lowercase().as_str() {
    "system" | "winapi" => Ok(CallingConvention::System),
    "cdecl" | "c" => Ok(CallingConvention::Cdecl),
    _ => Err(napi::Error::from_reason(format!(
      "Unsupported flat Win32 calling convention `{value}`"
    ))),
  }
}

fn pointer_value(owner: storage::PointerStorage) -> napi::Result<DynWin32Value> {
  Ok(DynWin32Value::with_pointer_owner(
    dynwinrt::win32::Value::Pointer(owner.pointer),
    RetainedNativePointer::Storage {
      value: owner,
      string: None,
    },
  ))
}

fn string_value(
  owner: storage::PointerStorage,
  wide: bool,
  multi: bool,
) -> napi::Result<DynWin32Value> {
  Ok(DynWin32Value::with_pointer_owner(
    dynwinrt::win32::Value::Pointer(owner.pointer),
    RetainedNativePointer::Storage {
      value: owner,
      string: Some((wide, multi)),
    },
  ))
}

fn string_pointer_pointer(
  value: Unknown,
  nullable: bool,
  wide: bool,
) -> napi::Result<DynWin32Value> {
  let env = value.value().env;
  let raw = value.value().value;
  let mut value_type = napi::sys::ValueType::napi_undefined;
  unsafe { napi::sys::napi_typeof(env, raw, &mut value_type) };
  if matches!(
    value_type,
    napi::sys::ValueType::napi_null | napi::sys::ValueType::napi_undefined
  ) {
    return if nullable {
      Ok(DynWin32Value::new(dynwinrt::win32::Value::Null))
    } else {
      Err(napi::Error::from_reason(
        "string pointer slot null requires an explicitly nullable parameter",
      ))
    };
  }
  let inner = storage::string_pointer(value, false, wide, false)?;
  let pointer = inner.pointer as usize;
  let mut slot = Box::new(pointer);
  let slot_pointer = (&mut *slot as *mut usize).cast();
  Ok(DynWin32Value {
    value: dynwinrt::win32::Value::Pointer(slot_pointer),
    pointer_owner: Some(Win32PointerOwner::PointerSlot {
      inner: Rc::new(RetainedNativePointer::Storage {
        value: inner,
        string: Some((wide, false)),
      }),
      slot,
    }),
  })
}

fn check_integer(value: f64, minimum: f64, maximum: f64, name: &str) -> napi::Result<()> {
  if !value.is_finite() || value.fract() != 0.0 || !(minimum..=maximum).contains(&value) {
    return Err(napi::Error::from_reason(format!(
      "DynWin32.{name}(): value is outside the exact integer range"
    )));
  }
  Ok(())
}

fn handle_value(value: Unknown, nullable: bool) -> napi::Result<DynWin32Value> {
  use napi::sys;

  let env = value.value().env;
  let raw = value.value().value;
  if let Ok(resource) = unsafe { <&DynWin32Resource>::from_napi_value(env, raw) } {
    if resource.0.is_closed() {
      return Err(napi::Error::from_reason(
        "Cannot use a closed flat Win32 resource",
      ));
    }

    return Ok(DynWin32Value::new(dynwinrt::win32::Value::Resource(
      Arc::clone(&resource.0),
    )));
  }

  let mut value_type = sys::ValueType::napi_undefined;
  unsafe { sys::napi_typeof(env, raw, &mut value_type) };
  if matches!(
    value_type,
    sys::ValueType::napi_null | sys::ValueType::napi_undefined
  ) {
    return if nullable {
      Ok(DynWin32Value::new(dynwinrt::win32::Value::Null))
    } else {
      Err(napi::Error::from_reason(
        "DynWin32.handle(): null requires an explicitly nullable handle parameter",
      ))
    };
  }
  let bits = if value_type == sys::ValueType::napi_bigint {
    if let Ok(signed) = storage::signed64(&value) {
      if signed as isize as i64 != signed {
        return Err(napi::Error::from_reason(
          "DynWin32.handle(): value does not fit this process pointer width",
        ));
      }
      signed as isize as usize as u64
    } else {
      storage::unsigned64(&value)?
    }
  } else if value_type == sys::ValueType::napi_number {
    let mut number = 0.0;
    unsafe { sys::napi_get_value_double(env, raw, &mut number) };
    if !number.is_finite() || number.fract() != 0.0 || number.abs() > 9_007_199_254_740_991.0 {
      return Err(napi::Error::from_reason(
        "DynWin32.handle(): number must be a safe integer",
      ));
    }
    if number < 0.0 {
      let signed = number as i64;
      if signed as isize as i64 != signed {
        return Err(napi::Error::from_reason(
          "DynWin32.handle(): value does not fit this process pointer width",
        ));
      }
      signed as isize as usize as u64
    } else {
      number as u64
    }
  } else {
    return Err(napi::Error::from_reason(
      "DynWin32.handle(): expected bigint, number, or DynWin32Resource",
    ));
  };
  if bits as usize as u64 != bits {
    return Err(napi::Error::from_reason(
      "DynWin32.handle(): value does not fit this process pointer width",
    ));
  }
  Ok(DynWin32Value::new(dynwinrt::win32::Value::Handle(
    bits as usize,
  )))
}

#[cfg(test)]
#[path = "win32_outcome_tests.rs"]
mod outcome_tests;

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn discarded_results_have_no_delivered_resource_without_relaxing_invalid_types() {
    let discarded = DynWin32Value::new(dynwinrt::win32::Value::Discarded);
    assert!(DynWin32::is_unavailable(&discarded));
    assert!(DynWin32::to_resource(&discarded).unwrap().is_none());
    assert!(DynWin32::to_resource_or_handle(&discarded)
      .unwrap()
      .is_none());

    let unavailable = DynWin32Value::new(dynwinrt::win32::Value::Unavailable);
    assert!(DynWin32::is_unavailable(&unavailable));
    assert!(DynWin32::to_resource(&unavailable).is_err());
    assert!(DynWin32::to_resource_or_handle(&unavailable).is_err());
    let invalid = DynWin32Value::new(dynwinrt::win32::Value::I32(0));
    assert!(!DynWin32::is_unavailable(&invalid));
    assert!(DynWin32::to_resource(&invalid).is_err());
    assert!(DynWin32::to_resource_or_handle(&invalid).is_err());
  }
}
