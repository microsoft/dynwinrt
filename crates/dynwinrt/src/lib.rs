// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use windows::core::*;

mod abi;
mod call;
pub mod com;
mod composition;
mod interfaces;
mod native_call;
mod native_callback;
mod result;
mod roapi;
mod signature;
mod value;
mod winapp;
mod xaml_application;

mod array;
#[macro_use]
mod com_helpers;
mod dasync;
pub mod delegate;
mod dispatcher_queue;
pub mod element_factory;
pub mod map;
mod meta;
pub mod metadata_table;
mod reference;
pub mod vector;

pub use crate::array::ArrayData;
pub use crate::composition::{
    LocalOverrideAbi, LocalOverrideInterface, LocalOverrideSizeCallback, LocalOverrideVoidCallback,
    compose_winrt, compose_winrt_with_overrides,
};
pub use crate::dasync::{
    AsyncCompletedCallback, ProgressCallback, ProgressResultCallback, WinRTAsyncFuture,
    create_progress_handler, create_progress_handler_with_result, get_async_results,
    set_async_completed_handler, try_create_progress_handler,
    try_create_progress_handler_with_result,
};
pub use crate::dispatcher_queue::{SystemDispatcherQueue, SystemDispatcherQueueHandle};
pub use crate::element_factory::{
    ElementFactoryGetCallback, ElementFactoryRecycleCallback, create_element_factory,
    create_element_factory_value,
};
pub use crate::metadata_table::{MetadataTable, MethodHandle, TypeHandle, TypeKind, ValueTypeData};
pub use crate::reference::box_ireference;
pub use crate::result::{Error, Result};
pub use crate::roapi::ro_get_activation_factory_2;
pub use crate::signature::{InterfaceSignature, MethodSignature};
pub use crate::value::{ArrayOfIUnknownData, AsyncInfo, WinRTValue};
pub use crate::winapp::{WinAppSdkContext, initialize_winappsdk};
pub use crate::xaml_application::{
    XamlRuntimeClassActivator, XamlRuntimeClassRegistration, create_xaml_application,
    register_xaml_runtime_class,
};
pub use interfaces::uri_vtable;

pub async fn get_async_string(
    op_string: windows_future::IAsyncOperation<HSTRING>,
) -> windows_core::Result<String> {
    let s = op_string.await?;
    Ok(s.to_string())
}

pub async fn http_get_string(url: &str) -> windows_core::Result<String> {
    use windows::Foundation::Uri;
    use windows::Web::Http::HttpClient;
    let uri = Uri::CreateUri(&HSTRING::from(url))?;
    let http_client = HttpClient::new()?;
    let op = http_client.GetStringAsync(&uri)?;
    let s = op.await?;
    Ok(s.to_string())
}

#[cfg(test)]
mod tests {
    use windows::Data::Xml::Dom::*;
    use windows::Foundation::Uri;

    use crate::value::WinRTValue;

    use super::*;

    #[test]
    fn from_raw_borrow_test() -> Result<()> {
        let uri = Uri::CreateUri(h!("https://www.example.com/path?query=1#fragment")).unwrap();
        let raw = uri.as_raw();
        let uri2 = unsafe { Uri::from_raw_borrowed(&raw) }.unwrap();
        let _ = uri2.clone();

        assert_eq!(uri.SchemeName().unwrap(), uri2.SchemeName().unwrap());
        Ok(())
    }

    #[test]
    fn http_call() -> Result<()> {
        futures::executor::block_on(async {
            use std::future::IntoFuture;

            let uri = Uri::CreateUri(h!("https://www.microsoft.com"))?;
            uri.SchemeName()?;
            let _ = uri.Host();
            let _ = uri.Port();
            let op = windows::Web::Http::HttpClient::new()?.GetStringAsync(&uri)?;
            let _s = op.into_future().await?;

            Ok(())
        })
    }

    #[test]
    fn simple_windows_api_should_work() -> Result<()> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(h!("<html>hello world</html>"))?;

        let root = doc.DocumentElement()?;
        assert!(root.NodeName()? == "html");
        assert!(root.InnerText()? == "hello world");

        Ok(())
    }

    #[test]
    fn test_winrt_uri() -> Result<()> {
        use windows::Foundation::Uri;
        println!("URI guid = {:?} ...", Uri::IID);
        println!("URI name = {:?} ...", Uri::NAME);
        let uri = Uri::CreateUri(h!("https://www.example.com/path?query=1#fragment"))?;
        assert_eq!(uri.SchemeName()?, "https");
        assert_eq!(uri.Host()?, "www.example.com");
        assert_eq!(uri.Path()?, "/path");
        assert_eq!(uri.Query()?, "?query=1");
        assert_eq!(uri.Fragment()?, "#fragment");

        Ok(())
    }

    #[test]
    fn test_winrt_uri_interop_using_signature() -> Result<()> {
        use windows::Foundation::Uri;

        let uri = Uri::CreateUri(h!("https://www.example.com/path?query=1#fragment"))?;

        let reg = metadata_table::MetadataTable::new();
        let vtable = uri_vtable(&reg);

        let get_runtime_classname = &vtable.methods[4];
        assert_eq!(
            get_runtime_classname.call_dynamic(uri.as_raw(), &[])?[0]
                .as_hstring()
                .unwrap(),
            "Windows.Foundation.Uri"
        );

        let get_scheme = &vtable.methods[17];
        let scheme = get_scheme.call_dynamic(uri.as_raw(), &[])?;
        assert_eq!(scheme[0].as_hstring().unwrap(), "https");
        let get_path = &vtable.methods[13];
        let path = get_path.call_dynamic(uri.as_raw(), &[])?;
        assert_eq!(path[0].as_hstring().unwrap(), "/path");
        let get_port = &vtable.methods[19];
        let port = get_port.call_dynamic(uri.as_raw(), &[])?;
        assert_eq!(port[0].as_i32().unwrap(), 443);

        Ok(())
    }

    #[test]
    fn test_uri_call_dynamic() -> Result<()> {
        let uri = Uri::CreateUri(h!("https://www.example.com/path?query=1#fragment")).unwrap();
        let reg = metadata_table::MetadataTable::new();
        let vtable = interfaces::uri_vtable(&reg);
        let res = vtable.methods[17].call_dynamic(uri.as_raw(), &[])?;
        assert_eq!(res[0].as_hstring().unwrap(), "https");
        Ok(())
    }

    #[test]
    fn test_struct_in_param_geopoint_create() -> Result<()> {
        use windows::Devices::Geolocation::Geopoint;
        use windows::Win32::System::WinRT::{
            IActivationFactory, RO_INIT_MULTITHREADED, RoGetActivationFactory, RoInitialize,
        };

        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        // 1. Define BasicGeoposition { Latitude: f64, Longitude: f64, Altitude: f64 }
        let reg = metadata_table::MetadataTable::new();
        let f64_h = reg.f64_type();
        let geo_type = reg.struct_type(
            "Windows.Devices.Geolocation.BasicGeoposition",
            &[f64_h.clone(), f64_h.clone(), f64_h],
        );

        // 2. Create and populate struct value
        let mut geo_val = geo_type.default_value();
        geo_val.set_field(0, 47.643f64);
        geo_val.set_field(1, -122.131f64);
        geo_val.set_field(2, 100.0f64);

        // 3. Get IGeopointFactory
        let afactory = unsafe {
            RoGetActivationFactory::<IActivationFactory>(h!("Windows.Devices.Geolocation.Geopoint"))
        }?;
        let geopoint_factory =
            afactory.cast::<windows::Devices::Geolocation::IGeopointFactory>()?;

        // 4. Define IGeopointFactory with Create method at vtable index 6
        // ABI: fn(this, BasicGeoposition, *out) -> HRESULT
        let mut factory_sig = InterfaceSignature::define_from_iinspectable(
            "IGeopointFactory",
            windows::Devices::Geolocation::IGeopointFactory::IID,
            &reg,
        );
        factory_sig.add_method(
            MethodSignature::new(&reg)
                .add_in(geo_type.clone())
                .add_out(reg.object()),
        );

        // 5. Call Create via MethodSignature
        let create_method = &factory_sig.methods[6];
        let results = create_method
            .call_dynamic(geopoint_factory.as_raw(), &[WinRTValue::Struct(geo_val)])?;

        // 6. Verify
        let geopoint_obj = results[0].as_object().unwrap();
        let geopoint: Geopoint = geopoint_obj.cast()?;
        let pos = geopoint.Position()?;
        assert!((pos.Latitude - 47.643).abs() < 1e-6);
        assert!((pos.Longitude - (-122.131)).abs() < 1e-6);
        assert!((pos.Altitude - 100.0).abs() < 1e-6);

        Ok(())
    }

    #[test]
    fn test_struct_out_param_geopoint_position() -> Result<()> {
        use windows::Devices::Geolocation::{BasicGeoposition, Geopoint, IGeopoint};
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        // 1. Create a Geopoint using static projection (known-good)
        let position = BasicGeoposition {
            Latitude: 47.643,
            Longitude: -122.131,
            Altitude: 100.0,
        };
        let geopoint = Geopoint::Create(position)?;

        // 2. Define BasicGeoposition struct type
        let reg = metadata_table::MetadataTable::new();
        let f64_h = reg.f64_type();
        let geo_type = reg.struct_type(
            "Windows.Devices.Geolocation.BasicGeoposition",
            &[f64_h.clone(), f64_h.clone(), f64_h],
        );

        // 3. Define IGeopoint with get_Position at vtable index 6
        // ABI: fn(this, *out_BasicGeoposition) -> HRESULT
        let mut igeopoint_sig =
            InterfaceSignature::define_from_iinspectable("IGeopoint", IGeopoint::IID, &reg);
        igeopoint_sig.add_method(MethodSignature::new(&reg).add_out(geo_type));

        // 4. Call get_Position via MethodSignature
        let igeopoint: IGeopoint = geopoint.cast()?;
        let get_position = &igeopoint_sig.methods[6];
        let results = get_position.call_dynamic(igeopoint.as_raw(), &[])?;

        // 5. Verify
        let data = results[0].as_struct().expect("Expected WinRTValue::Struct");
        let lat: f64 = data.get_field(0);
        let lon: f64 = data.get_field(1);
        let alt: f64 = data.get_field(2);
        assert!((lat - 47.643).abs() < 1e-6);
        assert!((lon - (-122.131)).abs() < 1e-6);
        assert!((alt - 100.0).abs() < 1e-6);

        Ok(())
    }

    #[test]
    fn test_pass_array_create_int32() -> Result<()> {
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        // IPropertyValueStatics: {629BDBC8-D932-4FF4-96B9-8D96C5C1E858}
        let statics_iid = windows_core::GUID::from_u128(0x629BDBC8_D932_4FF4_96B9_8D96C5C1E858);

        // Get factory and QI to IPropertyValueStatics
        let factory =
            WinRTValue::from_activation_factory(h!("Windows.Foundation.PropertyValue")).unwrap();
        let statics = factory.cast(&statics_iid).unwrap();

        // Build IPropertyValueStatics interface signature
        // vtable[6..28] = scalar Create methods (23 methods)
        // vtable[29] = CreateInt32Array(UINT32 length, INT32* data, IInspectable** result)
        let reg = metadata_table::MetadataTable::new();
        let mut iface = InterfaceSignature::define_from_iinspectable(
            "IPropertyValueStatics",
            statics_iid,
            &reg,
        );
        for _ in 0..23 {
            iface.add_method(MethodSignature::new(&reg)); // placeholders for vtable[6..28]
        }
        iface.add_method(
            MethodSignature::new(&reg)
                .add_in(reg.array(&reg.i32_type()))
                .add_out(reg.object()),
        );

        // Create array data
        let array_reg = metadata_table::MetadataTable::new();
        let array_arg = WinRTValue::Array(value::ArrayData::from_values(
            array_reg.i32_type(),
            &[
                WinRTValue::I32(10),
                WinRTValue::I32(20),
                WinRTValue::I32(30),
            ],
        ));

        // Call CreateInt32Array at vtable index 29
        let results =
            iface.methods[29].call_dynamic(statics.as_object().unwrap().as_raw(), &[array_arg])?;

        assert_eq!(results.len(), 1);
        let result_obj = results[0].as_object().unwrap();

        // Verify by reading back via static projection
        let inspectable: windows_core::IInspectable = result_obj.cast()?;
        let pv: windows::Foundation::IPropertyValue = inspectable.cast()?;
        let mut readback = windows::core::Array::<i32>::new();
        pv.GetInt32Array(&mut readback)?;
        assert_eq!(&readback[..], &[10, 20, 30]);

        Ok(())
    }

    #[test]
    fn test_receive_array_get_int32() -> Result<()> {
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        // Create a PropertyValue with known data via static projection
        let test_data = vec![100i32, 200, 300, 400, 500];
        let prop = windows::Foundation::PropertyValue::CreateInt32Array(&test_data)?;

        // IPropertyValue: {4BD682DD-7554-40E9-9A9B-82654EDE7E62}
        let ipv_iid = windows_core::GUID::from_u128(0x4BD682DD_7554_40E9_9A9B_82654EDE7E62);

        // QI to IPropertyValue
        let prop_unk: IUnknown = prop.cast()?;
        let mut ipv_ptr = std::ptr::null_mut();
        unsafe { prop_unk.query(&ipv_iid, &mut ipv_ptr) }.ok()?;
        let ipv = unsafe { IUnknown::from_raw(ipv_ptr) };

        // Build IPropertyValue interface signature
        // vtable[6..25] = scalar Get methods (20 methods)
        // vtable[26..28] = GetUInt8Array, GetInt16Array, GetUInt16Array
        // vtable[29] = GetInt32Array(UINT32* length, INT32** data)
        let reg = metadata_table::MetadataTable::new();
        let mut iface =
            InterfaceSignature::define_from_iinspectable("IPropertyValue", ipv_iid, &reg);
        for _ in 0..23 {
            iface.add_method(MethodSignature::new(&reg)); // placeholders for vtable[6..28]
        }
        iface.add_method(MethodSignature::new(&reg).add_out(reg.array(&reg.i32_type())));

        // Call GetInt32Array at vtable index 29
        let results = iface.methods[29].call_dynamic(ipv.as_raw(), &[])?;

        // Verify
        let array = results[0].as_array().expect("Expected WinRTValue::Array");
        assert_eq!(array.len(), 5);
        assert_eq!(array.get_i32(0).unwrap(), 100);
        assert_eq!(array.get_i32(1).unwrap(), 200);
        assert_eq!(array.get_i32(2).unwrap(), 300);
        assert_eq!(array.get_i32(3).unwrap(), 400);
        assert_eq!(array.get_i32(4).unwrap(), 500);

        Ok(())
    }
}
