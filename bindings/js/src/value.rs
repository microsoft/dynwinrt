// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::{
  async_promise, com_input, js_numbers::*, js_storage, managed_tsfn, winrt_delegate_method,
  DynWinRTArray, DynWinRTStruct, DynWinRTType, WinGUID, TABLE,
};
use napi::{
  bindgen_prelude::{BigInt, Either, PromiseRaw, Unknown},
  Env,
};
use napi_derive::napi;
use windows::core::{IUnknown, Interface, HSTRING};

#[path = "com_value.rs"]
pub(crate) mod com_value;

pub(crate) fn ensure_progress_type_supported(
  progress_type: &dynwinrt::TypeHandle,
) -> napi::Result<()> {
  match progress_type.kind() {
    dynwinrt::TypeKind::Guid
    | dynwinrt::TypeKind::ArrayOfIUnknown
    | dynwinrt::TypeKind::Generic { .. }
    | dynwinrt::TypeKind::OutValue(_)
    | dynwinrt::TypeKind::Array(_) => Err(napi::Error::from_reason(format!(
      "onProgress: progress callbacks do not support {:?} values",
      progress_type.kind()
    ))),
    dynwinrt::TypeKind::Bool
    | dynwinrt::TypeKind::I8
    | dynwinrt::TypeKind::U8
    | dynwinrt::TypeKind::I16
    | dynwinrt::TypeKind::U16
    | dynwinrt::TypeKind::Char16
    | dynwinrt::TypeKind::I32
    | dynwinrt::TypeKind::U32
    | dynwinrt::TypeKind::I64
    | dynwinrt::TypeKind::U64
    | dynwinrt::TypeKind::F32
    | dynwinrt::TypeKind::F64
    | dynwinrt::TypeKind::HString
    | dynwinrt::TypeKind::Object
    | dynwinrt::TypeKind::HResult
    | dynwinrt::TypeKind::Interface(_)
    | dynwinrt::TypeKind::Delegate(_)
    | dynwinrt::TypeKind::IAsyncAction
    | dynwinrt::TypeKind::IAsyncActionWithProgress(_)
    | dynwinrt::TypeKind::IAsyncOperation(_)
    | dynwinrt::TypeKind::IAsyncOperationWithProgress(_)
    | dynwinrt::TypeKind::RuntimeClass(_)
    | dynwinrt::TypeKind::Parameterized(_)
    | dynwinrt::TypeKind::Enum(_)
    | dynwinrt::TypeKind::Struct(_) => Ok(()),
  }
}

native_class! {
  pub struct DynWinRTValue {
    winrt: dynwinrt::WinRTValue,
    storage: Option<js_storage::CallStorage>,
    com: com_value::ComValueState,
  }
}
// Unbound WinRT bookkeeping is thread-safe. Once COM ownership is established,
// owner checks protect apartment-local Rc state; foreign destruction leaks only
// that bound state and its native owners instead of releasing them off-thread.
unsafe impl Send for DynWinRTValue {}
unsafe impl Sync for DynWinRTValue {}

#[napi]
impl DynWinRTValue {
  #[napi]
  pub fn release(&mut self) -> napi::Result<()> {
    self.release_value()
  }

  #[napi]
  pub fn activation_factory(name: String) -> napi::Result<DynWinRTValue> {
    let factory = dynwinrt::ro_get_activation_factory_2(&HSTRING::from(&name)).map_err(|e| {
      napi::Error::from_reason(format!("ActivationFactory '{}': {}", name, e.message()))
    })?;
    Ok(DynWinRTValue::new(factory))
  }

  /// Create an owned WinRT IBuffer by copying a Node.js Buffer or Uint8Array.
  #[napi]
  pub fn from_buffer(
    #[napi(ts_arg_type = "Buffer | Uint8Array")] data: napi::bindgen_prelude::Uint8Array,
  ) -> napi::Result<DynWinRTValue> {
    dynwinrt::copy_to_ibuffer(&data)
      .map(DynWinRTValue::new)
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  /// Create a composed WinUI Application that forwards IXamlMetadataProvider
  /// calls to the supplied provider.
  #[napi]
  pub fn create_xaml_application(
    metadata_provider: &DynWinRTValue,
    launched_callback: Option<&DynWinRTValue>,
  ) -> napi::Result<DynWinRTValue> {
    let provider = metadata_provider.winrt().as_object().ok_or_else(|| {
      napi::Error::from_reason("createXamlApplication: metadataProvider must be an Object")
    })?;
    let callback = launched_callback
      .map(|value| {
        value.winrt().as_object().ok_or_else(|| {
          napi::Error::from_reason("createXamlApplication: launchedCallback must be an Object")
        })
      })
      .transpose()?;
    dynwinrt::create_xaml_application(&provider, callback.as_ref())
      .map(DynWinRTValue::new)
      .map_err(|e| {
        napi::Error::from_reason(format!("createXamlApplication failed: {}", e.message()))
      })
  }

  #[napi]
  pub fn bool_value(value: bool) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::Bool(value))
  }
  #[napi]
  pub fn i8_value(value: i32) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::I8(value as i8))
  }
  #[napi]
  pub fn u8_value(value: u32) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::U8(value as u8))
  }
  #[napi]
  pub fn i16(value: i32) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::I16(value as i16))
  }
  #[napi]
  pub fn u16(value: u32) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::U16(value as u16))
  }
  #[napi]
  pub fn i32(value: i32) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::I32(value))
  }
  /// Create a semantic HRESULT value from a signed 32-bit status code.
  #[napi]
  pub fn hresult(value: f64) -> napi::Result<DynWinRTValue> {
    Ok(DynWinRTValue::new(dynwinrt::WinRTValue::HResult(
      windows::core::HRESULT(js_i32(value, "hresult")?),
    )))
  }
  /// Alias for hresult(), preserving the exact native HRESULT value type.
  #[napi(js_name = "fromHResult")]
  pub fn from_hresult(value: f64) -> napi::Result<DynWinRTValue> {
    Self::hresult(value)
  }
  #[napi]
  pub fn u32(value: f64) -> napi::Result<DynWinRTValue> {
    Ok(DynWinRTValue::new(dynwinrt::WinRTValue::U32(js_u32(
      value, "u32",
    )?)))
  }
  #[napi]
  pub fn i64(value: Either<BigInt, f64>) -> napi::Result<DynWinRTValue> {
    let value = js_i64(value, "i64")?;
    Ok(DynWinRTValue::new(dynwinrt::WinRTValue::I64(value)))
  }
  #[napi]
  pub fn u64(value: Either<BigInt, f64>) -> napi::Result<DynWinRTValue> {
    let value = js_u64(value, "u64")?;
    Ok(DynWinRTValue::new(dynwinrt::WinRTValue::U64(value)))
  }
  #[napi]
  pub fn f32(value: f64) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::F32(value as f32))
  }
  #[napi]
  pub fn f64(value: f64) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::F64(value))
  }
  /// Create an enum value within the enum's declared i32 or u32 range.
  #[napi]
  pub fn enum_value(enum_type: &DynWinRTType, value: f64) -> napi::Result<DynWinRTValue> {
    let value = if enum_type.0.underlying_kind() == dynwinrt::TypeKind::U32 {
      i64::from(js_u32(value, "enumValue")?)
    } else {
      i64::from(js_i32(value, "enumValue")?)
    };
    enum_type
      .0
      .enum_value(value)
      .map(DynWinRTValue::new)
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  #[napi]
  pub fn box_reference(
    value: &DynWinRTValue,
    value_type: &DynWinRTType,
  ) -> napi::Result<DynWinRTValue> {
    dynwinrt::box_ireference(value.winrt().clone(), value_type.0.clone())
      .map(DynWinRTValue::new)
      .map_err(|e| napi::Error::from_reason(e.message()))
  }

  /// Get the signed or unsigned numeric value of an enum. Returns None if not an enum.
  #[napi]
  pub fn get_enum_int(&self) -> Option<f64> {
    self.winrt.as_enum_number().map(|value| value as f64)
  }

  /// Get the member name of an enum value. Returns None if not an enum or no matching member.
  #[napi]
  pub fn get_enum_name(&self) -> Option<String> {
    match &self.winrt {
      dynwinrt::WinRTValue::Enum { value, type_handle } => type_handle.enum_member_name(*value),
      _ => None,
    }
  }

  #[napi]
  pub fn hstring(value: String) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::HString(HSTRING::from(value)))
  }
  #[napi]
  pub fn guid(value: &WinGUID) -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::Guid(value.0))
  }
  #[napi]
  pub fn null_value() -> DynWinRTValue {
    DynWinRTValue::new(dynwinrt::WinRTValue::Null)
  }

  /// Create an IVector<T> from items. The element_type is used for IID computation.
  /// Items are passed as DynWinRTValue objects (Object or Struct-wrapped values).
  #[napi]
  pub fn create_vector(
    items: Vec<&DynWinRTValue>,
    element_type: &DynWinRTType,
  ) -> napi::Result<DynWinRTValue> {
    let iids = TABLE.vector_iids(&element_type.0);
    let wrt_items: Vec<dynwinrt::WinRTValue> = items.iter().map(|i| i.winrt().clone()).collect();
    let vector = dynwinrt::vector::create_vector_from_values(&wrt_items, &element_type.0, iids)
      .map_err(|error| napi::Error::from_reason(error.message()))?;
    Ok(DynWinRTValue::new(dynwinrt::WinRTValue::Object(vector)))
  }

  /// Create an IMap<K,V> from parallel key/value arrays.
  /// Keys and values must be Object values (e.g. PropertyValue-boxed strings/ints).
  #[napi]
  pub fn create_map(
    keys: Vec<&DynWinRTValue>,
    values: Vec<&DynWinRTValue>,
    key_type: &DynWinRTType,
    value_type: &DynWinRTType,
  ) -> napi::Result<DynWinRTValue> {
    if keys.len() != values.len() {
      return Err(napi::Error::from_reason(
        "createMap: keys and values must have the same length",
      ));
    }
    let iids = TABLE.map_iids(&key_type.0, &value_type.0);
    let entries: Vec<(dynwinrt::WinRTValue, dynwinrt::WinRTValue)> = keys
      .iter()
      .zip(values.iter())
      .map(|(key, value)| (key.winrt().clone(), value.winrt().clone()))
      .collect();
    let map = dynwinrt::map::create_map_from_values(&entries, &key_type.0, &value_type.0, iids)
      .map_err(|error| napi::Error::from_reason(error.message()))?;
    Ok(DynWinRTValue::new(dynwinrt::WinRTValue::Object(map)))
  }

  #[napi]
  pub fn to_promise<'env>(&self, env: Env) -> napi::Result<PromiseRaw<'env, DynWinRTValue>> {
    let operation = match &self.winrt {
      dynwinrt::WinRTValue::Async(_) => self.winrt.clone(),
      _ => return Err(napi::Error::from_reason("toPromise: not an async value")),
    };
    async_promise::to_promise(env, operation)
  }

  /// Cancel the underlying WinRT async operation (calls `IAsyncInfo::Cancel`).
  /// Safe to call multiple times or on already-completed operations.
  ///
  /// Throws if this value is not an async operation.
  #[napi]
  pub fn cancel(&self) -> napi::Result<()> {
    let async_info = match &self.winrt {
      dynwinrt::WinRTValue::Async(a) => a,
      _ => return Err(napi::Error::from_reason("cancel: not an async value")),
    };
    async_info
      .cancel()
      .map_err(|e| napi::Error::from_reason(format!("Cancel failed: {}", e.message())))
  }

  #[napi]
  pub fn on_progress(
    &self,
    #[napi(ts_arg_type = "(progress: DynWinRtValue) => void")]
    callback: napi::bindgen_prelude::Function<'static, DynWinRTValue, ()>,
  ) -> napi::Result<()> {
    let async_info = match &self.winrt {
      dynwinrt::WinRTValue::Async(a) => a,
      _ => return Err(napi::Error::from_reason("onProgress: not an async value")),
    };

    let progress_type = async_info
      .progress_type()
      .ok_or_else(|| napi::Error::from_reason("onProgress: not a WithProgress async type"))?;
    ensure_progress_type_supported(&progress_type)?;

    let handler_iid = async_info
      .progress_handler_iid()
      .ok_or_else(|| napi::Error::from_reason("onProgress: cannot compute progress handler IID"))?;

    use napi::bindgen_prelude::ToNapiValue;
    use napi::JsValue;

    // Progress callbacks must not keep an otherwise idle Node process alive.
    let raw_env = callback.value().env;
    let raw_callback = napi::JsValue::raw(&callback);
    let tsfn = managed_tsfn::ManagedTsfn::create(
      raw_env,
      raw_callback,
      1024,
      true,
      |value: DynWinRTValue, env| {
        unsafe { DynWinRTValue::to_napi_value(env, value) }.map(|value| vec![value])
      },
      None,
    )?;
    let progress_cb: dynwinrt::ProgressResultCallback =
      Box::new(move |val: dynwinrt::WinRTValue| {
        let status = tsfn.call(DynWinRTValue::new(val));
        if status == napi::Status::Ok {
          windows::core::HRESULT(0)
        } else {
          if status != napi::Status::QueueFull {
            eprintln!("[dynwinrt] progress callback queue failed: {status}");
          }
          windows::core::HRESULT(0x80004005u32 as i32)
        }
      });
    let handler =
      dynwinrt::try_create_progress_handler_with_result(handler_iid, progress_type, progress_cb)
        .map_err(|error| {
          napi::Error::from_reason(format!(
            "onProgress: failed to create progress handler: {}",
            error.message()
          ))
        })?;

    async_info
      .set_progress_handler(&handler)
      .map_err(|e| napi::Error::from_reason(format!("SetProgress failed: {}", e.message())))?;

    Ok(())
  }

  #[napi]
  pub fn to_string(&self) -> String {
    if let Some(automation) = self.automation() {
      if let Ok(dynwinrt::com::Value::Bstr(value)) = automation.to_com_value() {
        return value.as_deref().unwrap_or_default().to_string();
      }
    }
    match &self.winrt {
      dynwinrt::WinRTValue::HString(s) => s.to_string(),
      dynwinrt::WinRTValue::I32(i) => i.to_string(),
      dynwinrt::WinRTValue::I64(i) => i.to_string(),
      dynwinrt::WinRTValue::Guid(g) => format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        g.data1,
        g.data2,
        g.data3,
        g.data4[0],
        g.data4[1],
        g.data4[2],
        g.data4[3],
        g.data4[4],
        g.data4[5],
        g.data4[6],
        g.data4[7]
      ),
      dynwinrt::WinRTValue::Object(o) => format!("Object: {:?}", o),
      _ => "Unsupported type".to_string(),
    }
  }

  #[napi]
  pub fn cast(&self, iid: &WinGUID) -> napi::Result<DynWinRTValue> {
    self.ensure_existing_com_apartment()?;
    let result = self
      .winrt
      .cast(&iid.0)
      .map_err(|e| napi::Error::from_reason(format!("QueryInterface failed: {}", e.message())))?;
    let mut result = DynWinRTValue::new(result);
    result.copy_com_bookkeeping_from(self);
    Ok(result)
  }

  /// Invoke a metadata-described WinRT delegate at IUnknown slot 3.
  /// Returns every output in signature order ([] for void), retaining the target
  /// and arguments across native calls. FillArray inputs require typed Array
  /// storage, just like outbound invokeAll; they are not U32 capacities.
  #[napi(strict)]
  pub fn invoke_delegate(
    &self,
    #[napi(ts_arg_type = "WinGuid")] iid: Unknown<'_>,
    #[napi(ts_arg_type = "DynWinRtMethodSig")] signature: Unknown<'_>,
    #[napi(ts_arg_type = "DynWinRtValue[]")] args: Vec<Unknown<'_>>,
  ) -> napi::Result<Vec<DynWinRTValue>> {
    self.ensure_existing_com_apartment()?;
    let target = self.winrt.clone();
    winrt_delegate_method::invoke_delegate_value(target, iid, signature, args)
  }

  #[napi]
  pub fn to_number(&self) -> napi::Result<f64> {
    Ok(match &self.winrt {
      dynwinrt::WinRTValue::Bool(b) => {
        if *b {
          1.0
        } else {
          0.0
        }
      }
      dynwinrt::WinRTValue::I8(i) => f64::from(*i),
      dynwinrt::WinRTValue::U8(i) => f64::from(*i),
      dynwinrt::WinRTValue::I16(i) => f64::from(*i),
      dynwinrt::WinRTValue::U16(i) => f64::from(*i),
      dynwinrt::WinRTValue::I32(i) => f64::from(*i),
      dynwinrt::WinRTValue::U32(i) => f64::from(*i),
      dynwinrt::WinRTValue::HResult(hr) => f64::from(hr.0),
      dynwinrt::WinRTValue::Enum { value, type_handle } => {
        if type_handle.underlying_kind() == dynwinrt::TypeKind::U32 {
          f64::from(*value as u32)
        } else {
          f64::from(*value)
        }
      }
      _ => {
        return Err(napi::Error::from_reason(format!(
          "Cannot convert {:?} to number",
          self.winrt.get_type_kind(),
        )));
      }
    })
  }

  #[napi]
  pub fn to_bool(&self) -> napi::Result<bool> {
    match &self.winrt {
      dynwinrt::WinRTValue::Bool(b) => Ok(*b),
      _ => self.to_number().map(|value| value != 0.0),
    }
  }

  #[napi]
  pub fn to_i64(&self) -> napi::Result<i64> {
    let value = match &self.winrt {
      dynwinrt::WinRTValue::I64(i) => *i,
      dynwinrt::WinRTValue::U64(i) => i64::try_from(*i).map_err(|_| {
        napi::Error::from_reason("Cannot convert u64 value greater than i64::MAX to i64")
      })?,
      _ => self.to_number().map(|value| value as i64)?,
    };
    js_safe_i64(value, "toI64")
  }

  #[napi]
  pub fn to_i64_bigint(&self) -> napi::Result<BigInt> {
    match &self.winrt {
      dynwinrt::WinRTValue::I64(value) => Ok(BigInt::from(*value)),
      _ => Err(napi::Error::from_reason(
        "toI64Bigint requires a signed 64-bit value",
      )),
    }
  }

  #[napi]
  pub fn to_u64_bigint(&self) -> napi::Result<BigInt> {
    match &self.winrt {
      dynwinrt::WinRTValue::U64(value) => Ok(BigInt::from(*value)),
      _ => Err(napi::Error::from_reason(
        "toU64Bigint requires an unsigned 64-bit value",
      )),
    }
  }

  #[napi]
  pub fn to_f64(&self) -> napi::Result<f64> {
    match &self.winrt {
      dynwinrt::WinRTValue::F64(f) => Ok(*f),
      dynwinrt::WinRTValue::F32(f) => Ok(*f as f64),
      _ => self.to_number(),
    }
  }

  #[napi]
  pub fn to_guid(&self) -> napi::Result<WinGUID> {
    match &self.winrt {
      dynwinrt::WinRTValue::Guid(g) => Ok(WinGUID(*g)),
      _ => Err(napi::Error::from_reason("Value is not a GUID")),
    }
  }

  /// Copy the initialized bytes from a WinRT IBuffer into a Node.js Buffer.
  #[napi]
  pub fn to_buffer(&self) -> napi::Result<napi::bindgen_prelude::Buffer> {
    dynwinrt::copy_from_ibuffer(&self.winrt)
      .map(Into::into)
      .map_err(|error| napi::Error::from_reason(error.message()))
  }

  #[napi]
  pub fn is_null(&self) -> bool {
    !self.has_com_payload() && self.winrt.is_null_object()
  }

  #[napi]
  pub fn as_raw(&self) -> napi::Result<i64> {
    match &self.winrt {
      dynwinrt::WinRTValue::Object(o) => Ok(o.as_raw() as i64),
      _ => Err(napi::Error::from_reason(
        "Cannot get raw pointer from non-object",
      )),
    }
  }

  #[napi]
  pub fn identity_raw(&self) -> napi::Result<i64> {
    match &self.winrt {
      dynwinrt::WinRTValue::Object(object) => object
        .cast::<IUnknown>()
        .map(|identity| identity.as_raw() as i64)
        .map_err(|error| napi::Error::from_reason(error.message())),
      _ => Err(napi::Error::from_reason(
        "Cannot get COM identity from a non-object value",
      )),
    }
  }

  // -- Array / Struct extraction --

  #[napi]
  pub fn is_array(&self) -> bool {
    self.winrt.as_array().is_some()
  }

  #[napi]
  pub fn as_array(&self) -> napi::Result<DynWinRTArray> {
    self.ensure_existing_com_apartment()?;
    match &self.winrt {
      dynwinrt::WinRTValue::Array(data) => Ok(DynWinRTArray(
        data.clone(),
        self
          .input_bindings()
          .map(com_input::InputBindings::copy_bookkeeping),
      )),
      _ => Err(napi::Error::from_reason("Value is not an Array")),
    }
  }

  #[napi]
  pub fn is_struct(&self) -> bool {
    self.winrt.as_struct().is_some()
  }

  #[napi]
  pub fn as_struct(&self) -> napi::Result<DynWinRTStruct> {
    self.ensure_existing_com_apartment()?;
    match &self.winrt {
      dynwinrt::WinRTValue::Struct(data) => Ok(DynWinRTStruct::from_data(
        data.clone(),
        self.input_bindings(),
      )),
      _ => Err(napi::Error::from_reason("Value is not a Struct")),
    }
  }
}
