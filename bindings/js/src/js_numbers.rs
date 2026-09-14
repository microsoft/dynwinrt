// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use napi::bindgen_prelude::{BigInt, Either};

pub(crate) fn unsigned_size_number(value: f64, reason: &str) -> napi::Result<usize> {
  if !value.is_finite()
    || value < 0.0
    || value.fract() != 0.0
    || value > 9_007_199_254_740_991.0
    || value as u64 as usize as u64 != value as u64
  {
    return Err(napi::Error::from_reason(reason));
  }
  Ok(value as usize)
}

pub(crate) fn js_u64(value: Either<BigInt, f64>, context: &str) -> napi::Result<u64> {
  match value {
    Either::A(value) => {
      let (negative, value, lossless) = value.get_u64();
      if negative || !lossless {
        return Err(napi::Error::from_reason(format!(
          "{context}: bigint value must fit in an unsigned 64-bit integer",
        )));
      }
      Ok(value)
    }
    Either::B(value) => {
      if !value.is_finite()
        || value.fract() != 0.0
        || !(0.0..=9_007_199_254_740_991.0).contains(&value)
      {
        return Err(napi::Error::from_reason(format!(
          "{context}: number value must be a non-negative safe integer; use bigint for larger values",
        )));
      }
      Ok(value as u64)
    }
  }
}

pub(crate) fn js_i64(value: Either<BigInt, f64>, context: &str) -> napi::Result<i64> {
  match value {
    Either::A(value) => {
      let (value, lossless) = value.get_i64();
      if !lossless {
        return Err(napi::Error::from_reason(format!(
          "{context}: bigint value must fit in a signed 64-bit integer",
        )));
      }
      Ok(value)
    }
    Either::B(value) => {
      if !value.is_finite() || value.fract() != 0.0 || value.abs() > 9_007_199_254_740_991.0 {
        return Err(napi::Error::from_reason(format!(
          "{context}: number value must be a safe integer; use bigint for larger values",
        )));
      }
      Ok(value as i64)
    }
  }
}

pub(crate) fn js_i32(value: f64, context: &str) -> napi::Result<i32> {
  if !value.is_finite()
    || value.fract() != 0.0
    || value < f64::from(i32::MIN)
    || value > f64::from(i32::MAX)
  {
    return Err(napi::Error::from_reason(format!(
      "{context}: value must be an integer in the i32 range",
    )));
  }
  Ok(value as i32)
}

pub(crate) fn js_u32(value: f64, context: &str) -> napi::Result<u32> {
  if !value.is_finite() || value.fract() != 0.0 || !(0.0..=f64::from(u32::MAX)).contains(&value) {
    return Err(napi::Error::from_reason(format!(
      "{context}: value must be an integer in the u32 range",
    )));
  }
  Ok(value as u32)
}

pub(crate) fn js_safe_i64(value: i64, context: &str) -> napi::Result<i64> {
  const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
  if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) {
    return Err(napi::Error::from_reason(format!(
      "{context}: value is outside the JavaScript safe-integer range; use the bigint conversion instead",
    )));
  }

  Ok(value)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn unsigned_sizes_preserve_js_and_target_integer_bounds() {
    for value in [f64::NAN, f64::INFINITY, -1.0, 0.5, 9_007_199_254_740_992.0] {
      assert_eq!(
        unsigned_size_number(value, "size error")
          .unwrap_err()
          .reason,
        "size error"
      );
    }
    assert_eq!(unsigned_size_number(-0.0, "size error").unwrap(), 0);
    assert_eq!(
      unsigned_size_number(u32::MAX as f64, "size error").unwrap(),
      u32::MAX as usize
    );
    assert_eq!(
      unsigned_size_number(u32::MAX as f64 + 1.0, "size error").is_ok(),
      cfg!(target_pointer_width = "64"),
    );
  }
}
