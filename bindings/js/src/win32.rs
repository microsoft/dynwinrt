// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use napi::bindgen_prelude::{BigInt, Either, FromNapiValue, Unknown};
use napi::JsValue;
use napi_derive::napi;

use super::{DynWinRTValue, com};

#[napi]
pub struct DynWin32Value(DynWinRTValue);

#[napi]
pub struct DynWin32CallResult {
  value: Option<DynWin32Value>,
  last_error: Option<u32>,
}

#[napi]
impl DynWin32CallResult {
  #[napi(getter)]
  pub fn value(&mut self) -> napi::Result<DynWin32Value> {
    self
      .value
      .take()
      .ok_or_else(|| napi::Error::from_reason("Flat Win32 result value was already consumed"))
  }

  #[napi(getter)]
  pub fn last_error(&self) -> Option<u32> {
    self.last_error
  }
}

#[napi]
pub struct DynWin32;

#[napi]
impl DynWin32 {
  #[napi]
  pub fn pointer(
    #[napi(ts_arg_type = "bigint | number | Buffer | Uint8Array | null | undefined")]
    value: Unknown,
  ) -> napi::Result<DynWin32Value> {
    com::pointer(value).map(DynWin32Value)
  }

  #[napi]
  pub fn handle(
    #[napi(ts_arg_type = "bigint | number")] value: Unknown,
  ) -> napi::Result<DynWin32Value> {
    let bits = handle_bits(value)?;
    Ok(DynWin32Value(DynWinRTValue::with_borrowed_pointer(
      dynwinrt::WinRTValue::RawPtr(bits as usize as *mut std::ffi::c_void),
    )))
  }

  #[napi]
  pub fn i8(value: f64) -> napi::Result<DynWin32Value> {
    let value = checked_integer(value, i8::MIN as f64, i8::MAX as f64, "i8")?;
    Ok(DynWin32Value(DynWinRTValue::new(dynwinrt::WinRTValue::I8(
      value as i8,
    ))))
  }

  #[napi]
  pub fn u8(value: f64) -> napi::Result<DynWin32Value> {
    let value = checked_integer(value, u8::MIN as f64, u8::MAX as f64, "u8")?;
    Ok(DynWin32Value(DynWinRTValue::new(dynwinrt::WinRTValue::U8(
      value as u8,
    ))))
  }

  #[napi]
  pub fn i16(value: f64) -> napi::Result<DynWin32Value> {
    let value = checked_integer(value, i16::MIN as f64, i16::MAX as f64, "i16")?;
    Ok(DynWin32Value(DynWinRTValue::new(
      dynwinrt::WinRTValue::I16(value as i16),
    )))
  }

  #[napi]
  pub fn u16(value: f64) -> napi::Result<DynWin32Value> {
    let value = checked_integer(value, u16::MIN as f64, u16::MAX as f64, "u16")?;
    Ok(DynWin32Value(DynWinRTValue::new(
      dynwinrt::WinRTValue::U16(value as u16),
    )))
  }

  #[napi]
  pub fn i32(value: f64) -> napi::Result<DynWin32Value> {
    let value = checked_integer(value, i32::MIN as f64, i32::MAX as f64, "i32")?;
    Ok(DynWin32Value(DynWinRTValue::new(
      dynwinrt::WinRTValue::I32(value as i32),
    )))
  }

  #[napi]
  pub fn u32(value: f64) -> napi::Result<DynWin32Value> {
    let value = checked_integer(value, u32::MIN as f64, u32::MAX as f64, "u32")?;
    Ok(DynWin32Value(DynWinRTValue::new(
      dynwinrt::WinRTValue::U32(value as u32),
    )))
  }

  #[napi]
  pub fn i64(
    #[napi(ts_arg_type = "number | bigint")] value: Either<BigInt, f64>,
  ) -> napi::Result<DynWin32Value> {
    let value = match value {
      Either::A(value) => {
        let (value, lossless) = value.get_i64();
        if !lossless {
          return Err(napi::Error::from_reason(
            "DynWin32.i64(): value must fit in a signed 64-bit integer",
          ));
        }
        value
      }
      Either::B(value) => {
        if !value.is_finite() || value.fract() != 0.0 || value.abs() > 9_007_199_254_740_991.0 {
          return Err(napi::Error::from_reason(
            "DynWin32.i64(): number must be a safe integer",
          ));
        }
        value as i64
      }
    };
    Ok(DynWin32Value(DynWinRTValue::new(
      dynwinrt::WinRTValue::I64(value),
    )))
  }

  #[napi]
  pub fn u64(
    #[napi(ts_arg_type = "number | bigint")] value: Either<BigInt, f64>,
  ) -> napi::Result<DynWin32Value> {
    let value = match value {
      Either::A(value) => {
        let (negative, value, lossless) = value.get_u64();
        if negative || !lossless {
          return Err(napi::Error::from_reason(
            "DynWin32.u64(): value must fit in an unsigned 64-bit integer",
          ));
        }
        value
      }
      Either::B(value) => {
        if !value.is_finite()
          || value < 0.0
          || value.fract() != 0.0
          || value > 9_007_199_254_740_991.0
        {
          return Err(napi::Error::from_reason(
            "DynWin32.u64(): number must be a non-negative safe integer",
          ));
        }
        value as u64
      }
    };
    Ok(DynWin32Value(DynWinRTValue::new(
      dynwinrt::WinRTValue::U64(value),
    )))
  }

  #[napi]
  pub fn f32(value: f64) -> DynWin32Value {
    DynWin32Value(DynWinRTValue::new(dynwinrt::WinRTValue::F32(value as f32)))
  }

  #[napi]
  pub fn f64(value: f64) -> DynWin32Value {
    DynWin32Value(DynWinRTValue::new(dynwinrt::WinRTValue::F64(value)))
  }

  #[napi]
  pub fn invoke(
    dll: String,
    entry: String,
    ret_kind: String,
    args: Vec<&DynWin32Value>,
    capture_last_error: bool,
  ) -> napi::Result<DynWin32CallResult> {
    let ret = parse_return_kind(&ret_kind)?;
    for arg in &args {
      com::validate_pointer_owner(&arg.0)?;
    }
    let args = args.iter().map(|arg| arg.0.0.clone()).collect::<Vec<_>>();
    let result = unsafe {
      dynwinrt::win32::flat_invoke_with_options(&dll, &entry, ret, &args, capture_last_error)
    }
    .map_err(|error| {
      napi::Error::from_reason(format!(
        "DynWin32.invoke({dll}!{entry}): {}",
        error.message()
      ))
    })?;
    Ok(DynWin32CallResult {
      value: Some(DynWin32Value(DynWinRTValue::new(result.value))),
      last_error: result.last_error,
    })
  }

  #[napi]
  pub fn to_number(value: &DynWin32Value) -> napi::Result<f64> {
    match &value.0.0 {
      dynwinrt::WinRTValue::Bool(value) => Ok(u8::from(*value) as f64),
      dynwinrt::WinRTValue::I8(value) => Ok(*value as f64),
      dynwinrt::WinRTValue::U8(value) => Ok(*value as f64),
      dynwinrt::WinRTValue::I16(value) => Ok(*value as f64),
      dynwinrt::WinRTValue::U16(value) => Ok(*value as f64),
      dynwinrt::WinRTValue::I32(value) => Ok(*value as f64),
      dynwinrt::WinRTValue::U32(value) => Ok(*value as f64),
      dynwinrt::WinRTValue::HResult(value) => Ok(value.0 as f64),
      _ => Err(napi::Error::from_reason("Value is not a 32-bit scalar")),
    }
  }

  #[napi]
  pub fn to_pointer_bigint(value: &DynWin32Value) -> napi::Result<BigInt> {
    com::as_pointer_bigint(&value.0)
  }

  #[napi]
  pub fn to_i64_bigint(value: &DynWin32Value) -> napi::Result<BigInt> {
    match &value.0.0 {
      dynwinrt::WinRTValue::I64(value) => Ok(BigInt::from(*value)),
      _ => Err(napi::Error::from_reason("Value is not an i64")),
    }
  }

  #[napi]
  pub fn to_u64_bigint(value: &DynWin32Value) -> napi::Result<BigInt> {
    match &value.0.0 {
      dynwinrt::WinRTValue::U64(value) => Ok(BigInt::from(*value)),
      _ => Err(napi::Error::from_reason("Value is not a u64")),
    }
  }

  #[napi]
  pub fn to_f64(value: &DynWin32Value) -> napi::Result<f64> {
    match &value.0.0 {
      dynwinrt::WinRTValue::F32(value) => Ok(*value as f64),
      dynwinrt::WinRTValue::F64(value) => Ok(*value),
      _ => Err(napi::Error::from_reason(
        "Value is not a floating-point scalar",
      )),
    }
  }
}

fn checked_integer(value: f64, min: f64, max: f64, kind: &str) -> napi::Result<f64> {
  if !value.is_finite() || value.fract() != 0.0 || value < min || value > max {
    return Err(napi::Error::from_reason(format!(
      "DynWin32.{kind}(): value must be an integer in the range {min}..={max}"
    )));
  }
  Ok(value)
}

fn handle_bits(value: Unknown) -> napi::Result<u64> {
  use napi::sys;

  let env = value.value().env;
  let raw = value.value().value;
  let mut value_type = sys::ValueType::napi_undefined;
  unsafe { sys::napi_typeof(env, raw, &mut value_type) };
  let bits = if value_type == sys::ValueType::napi_bigint {
    let value = unsafe { BigInt::from_napi_value(env, raw) }?;
    let (signed, signed_lossless) = value.get_i64();
    if signed_lossless {
      signed as u64
    } else {
      let (negative, unsigned, unsigned_lossless) = value.get_u64();
      if negative || !unsigned_lossless {
        return Err(napi::Error::from_reason(
          "DynWin32.handle(): bigint must fit in a signed or unsigned pointer-width value",
        ));
      }
      unsigned
    }
  } else if value_type == sys::ValueType::napi_number {
    let mut number = 0.0;
    unsafe { sys::napi_get_value_double(env, raw, &mut number) };
    if !number.is_finite() || number.fract() != 0.0 || number.abs() > 9_007_199_254_740_991.0 {
      return Err(napi::Error::from_reason(
        "DynWin32.handle(): number must be a safe integer",
      ));
    }
    (number as i64) as u64
  } else {
    return Err(napi::Error::from_reason(
      "DynWin32.handle(): expected bigint or number",
    ));
  };

  if bits as usize as u64 != bits {
    return Err(napi::Error::from_reason(
      "DynWin32.handle(): value does not fit this target pointer width",
    ));
  }
  Ok(bits)
}

fn parse_return_kind(value: &str) -> napi::Result<dynwinrt::win32::FlatReturnKind> {
  use dynwinrt::win32::FlatReturnKind;

  match value.to_ascii_lowercase().as_str() {
    "void" => Ok(FlatReturnKind::Void),
    "i8" => Ok(FlatReturnKind::I8),
    "u8" => Ok(FlatReturnKind::U8),
    "i16" => Ok(FlatReturnKind::I16),
    "u16" => Ok(FlatReturnKind::U16),
    "i32" => Ok(FlatReturnKind::I32),
    "u32" => Ok(FlatReturnKind::U32),
    "i64" => Ok(FlatReturnKind::I64),
    "u64" => Ok(FlatReturnKind::U64),
    "f32" => Ok(FlatReturnKind::F32),
    "f64" => Ok(FlatReturnKind::F64),
    "ptr" | "pointer" => Ok(FlatReturnKind::Ptr),
    _ => Err(napi::Error::from_reason(format!(
      "Unsupported flat Win32 return kind: {value}"
    ))),
  }
}
