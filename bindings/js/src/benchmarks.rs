// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use napi_derive::napi;
use windows::core::HSTRING;

native_class! {
  pub struct RustStaticBench;
}

fn map_win_err(e: windows::core::Error) -> napi::Error {
  napi::Error::from_reason(e.message())
}

native_class! {
  /// Pre-created Uri for static benchmark (stores typed interface, no QI on access).
  pub struct StaticUri(windows::Foundation::Uri);
}
unsafe impl Send for StaticUri {}
unsafe impl Sync for StaticUri {}

native_class! {
  /// Pre-created opaque COM object for static benchmark (factory results).
  pub struct StaticObj(#[allow(dead_code)] windows::core::IInspectable);
}
unsafe impl Send for StaticObj {}
unsafe impl Sync for StaticObj {}

#[napi]
impl RustStaticBench {
  // --- Uri ---

  #[napi]
  pub fn uri_create(url: String) -> napi::Result<StaticUri> {
    let uri = windows::Foundation::Uri::CreateUri(&HSTRING::from(url)).map_err(map_win_err)?;
    Ok(StaticUri(uri))
  }

  #[napi]
  pub fn uri_get_host(url: String) -> napi::Result<String> {
    let uri = windows::Foundation::Uri::CreateUri(&HSTRING::from(url)).map_err(map_win_err)?;
    Ok(uri.Host().map_err(map_win_err)?.to_string())
  }

  #[napi]
  pub fn uri_host_from_obj(obj: &StaticUri) -> napi::Result<String> {
    Ok(obj.0.Host().map_err(map_win_err)?.to_string())
  }

  #[napi]
  pub fn uri_port_from_obj(obj: &StaticUri) -> napi::Result<i32> {
    Ok(obj.0.Port().map_err(map_win_err)?)
  }

  #[napi]
  pub fn uri_suspicious_from_obj(obj: &StaticUri) -> napi::Result<bool> {
    Ok(obj.0.Suspicious().map_err(map_win_err)?)
  }

  #[napi]
  pub fn uri_query_parsed_from_obj(obj: &StaticUri) -> napi::Result<StaticObj> {
    Ok(StaticObj(obj.0.QueryParsed().map_err(map_win_err)?.into()))
  }

  #[napi]
  pub fn uri_combine(obj: &StaticUri, relative: String) -> napi::Result<StaticUri> {
    let result = obj
      .0
      .CombineUri(&HSTRING::from(relative))
      .map_err(map_win_err)?;
    Ok(StaticUri(result))
  }

  #[napi]
  pub fn uri_create_with_relative(base: String, relative: String) -> napi::Result<StaticUri> {
    let uri = windows::Foundation::Uri::CreateWithRelativeUri(
      &HSTRING::from(base),
      &HSTRING::from(relative),
    )
    .map_err(map_win_err)?;
    Ok(StaticUri(uri))
  }

  // --- PropertyValue ---

  #[napi]
  pub fn pv_create_i32(value: i32) -> napi::Result<StaticObj> {
    Ok(StaticObj(
      windows::Foundation::PropertyValue::CreateInt32(value)
        .map_err(map_win_err)?
        .into(),
    ))
  }

  #[napi]
  pub fn pv_create_f64(value: f64) -> napi::Result<StaticObj> {
    Ok(StaticObj(
      windows::Foundation::PropertyValue::CreateDouble(value)
        .map_err(map_win_err)?
        .into(),
    ))
  }

  #[napi]
  pub fn pv_create_bool(value: bool) -> napi::Result<StaticObj> {
    Ok(StaticObj(
      windows::Foundation::PropertyValue::CreateBoolean(value)
        .map_err(map_win_err)?
        .into(),
    ))
  }

  #[napi]
  pub fn pv_create_string(value: String) -> napi::Result<StaticObj> {
    Ok(StaticObj(
      windows::Foundation::PropertyValue::CreateString(&HSTRING::from(value))
        .map_err(map_win_err)?
        .into(),
    ))
  }

  // --- Geopoint ---

  #[napi]
  pub fn geopoint_create(lat: f64, lon: f64, alt: f64) -> napi::Result<StaticObj> {
    use windows::Devices::Geolocation::{BasicGeoposition, Geopoint};
    let pos = BasicGeoposition {
      Latitude: lat,
      Longitude: lon,
      Altitude: alt,
    };
    Ok(StaticObj(
      Geopoint::Create(pos).map_err(map_win_err)?.into(),
    ))
  }
}
