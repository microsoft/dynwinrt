// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.
#![cfg(windows)]

use dynwinrt::{InterfaceSignature, MetadataTable, MethodSignature, WinRTValue};
use windows::Devices::Geolocation::{BasicGeoposition, Geopoint, IGeopoint, IGeopointFactory};
use windows::Foundation::{IPropertyValue, IUriRuntimeClass, IUriRuntimeClassFactory};
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows_core::{GUID, HRESULT, HSTRING, Interface};

fn init_winrt() {
    // The process may already be initialized by another test. `RoInitialize`
    // returns `Ok` for S_FALSE (already initialized in the same apartment), and
    // `RPC_E_CHANGED_MODE` when it is already initialized in a different
    // apartment — both are acceptable for these headless WinRT calls. Any other
    // failure is a genuine problem and must not be silently ignored.
    const RPC_E_CHANGED_MODE: HRESULT = HRESULT(0x8001_0106u32 as i32);
    if let Err(e) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
        assert_eq!(
            e.code(),
            RPC_E_CHANGED_MODE,
            "RoInitialize failed unexpectedly (only RPC_E_CHANGED_MODE is benign): {e:?}"
        );
    }
}

fn assert_hstring(value: &WinRTValue, expected: &str) {
    assert_eq!(value.as_hstring().expect("expected HSTRING"), expected);
}

fn assert_bool(value: &WinRTValue, expected: bool) {
    match value {
        WinRTValue::Bool(actual) => assert_eq!(*actual, expected),
        other => panic!("expected Bool({expected}), got {other:?}"),
    }
}

fn uri_runtime_class_signature(reg: &std::sync::Arc<MetadataTable>) -> InterfaceSignature {
    let mut iface = InterfaceSignature::define_from_iinspectable(
        "IUriRuntimeClass",
        IUriRuntimeClass::IID,
        reg,
    );
    iface.add_method(MethodSignature::new(reg).add_out(reg.hstring())); // 6 AbsoluteUri
    iface.add_method(MethodSignature::new(reg).add_out(reg.hstring())); // 7 DisplayUri
    iface.add_method(MethodSignature::new(reg).add_out(reg.hstring())); // 8 Domain
    iface.add_method(MethodSignature::new(reg).add_out(reg.hstring())); // 9 Extension
    iface.add_method(MethodSignature::new(reg).add_out(reg.hstring())); // 10 Fragment
    iface.add_method(MethodSignature::new(reg).add_out(reg.hstring())); // 11 Host
    iface.add_method(MethodSignature::new(reg).add_out(reg.hstring())); // 12 Password
    iface.add_method(MethodSignature::new(reg).add_out(reg.hstring())); // 13 Path
    iface.add_method(MethodSignature::new(reg).add_out(reg.hstring())); // 14 Query
    iface.add_method(MethodSignature::new(reg).add_out(reg.object())); // 15 QueryParsed
    iface.add_method(MethodSignature::new(reg).add_out(reg.hstring())); // 16 RawUri
    iface.add_method(MethodSignature::new(reg).add_out(reg.hstring())); // 17 SchemeName
    iface.add_method(MethodSignature::new(reg).add_out(reg.hstring())); // 18 UserName
    iface.add_method(MethodSignature::new(reg).add_out(reg.i32_type())); // 19 Port
    iface.add_method(MethodSignature::new(reg)); // 20 Suspicious (unused)
    iface
}

fn create_uri_dynamic(
    reg: &std::sync::Arc<MetadataTable>,
    raw: &str,
) -> windows_core::Result<WinRTValue> {
    let factory = WinRTValue::from_activation_factory(&HSTRING::from("Windows.Foundation.Uri"))
        .expect("Windows.Foundation.Uri activation factory");
    let uri_factory = factory
        .cast(&IUriRuntimeClassFactory::IID)
        .expect("IUriRuntimeClassFactory");
    let mut iface = InterfaceSignature::define_from_iinspectable(
        "IUriRuntimeClassFactory",
        IUriRuntimeClassFactory::IID,
        reg,
    );
    iface.add_method(
        MethodSignature::new(reg)
            .add_in(reg.hstring())
            .add_out(reg.object()),
    );
    let uri_factory_obj = uri_factory.as_object().expect("factory object");
    let result = iface.methods[6].call_dynamic(
        uri_factory_obj.as_raw(),
        &[WinRTValue::HString(HSTRING::from(raw))],
    )?;
    Ok(result[0].clone())
}

fn property_value_statics_signature(reg: &std::sync::Arc<MetadataTable>) -> InterfaceSignature {
    let statics_iid = GUID::from_u128(0x629BDBC8_D932_4FF4_96B9_8D96C5C1E858);
    let mut iface =
        InterfaceSignature::define_from_iinspectable("IPropertyValueStatics", statics_iid, reg);
    for _ in 0..4 {
        iface.add_method(MethodSignature::new(reg)); // 6 CreateEmpty through 9 CreateUInt16
    }
    iface.add_method(
        MethodSignature::new(reg)
            .add_in(reg.i32_type())
            .add_out(reg.object()),
    ); // 10 CreateInt32
    for _ in 0..6 {
        iface.add_method(MethodSignature::new(reg)); // 11 CreateUInt32 through 16 CreateChar16
    }
    iface.add_method(
        MethodSignature::new(reg)
            .add_in(reg.bool_type())
            .add_out(reg.object()),
    ); // 17 CreateBoolean
    iface.add_method(
        MethodSignature::new(reg)
            .add_in(reg.hstring())
            .add_out(reg.object()),
    ); // 18 CreateString
    iface
}

fn property_value_signature(reg: &std::sync::Arc<MetadataTable>) -> InterfaceSignature {
    let mut iface =
        InterfaceSignature::define_from_iinspectable("IPropertyValue", IPropertyValue::IID, reg);
    iface.add_method(MethodSignature::new(reg).add_out(reg.i32_type())); // 6 get_Type
    iface.add_method(MethodSignature::new(reg).add_out(reg.bool_type())); // 7 get_IsNumericScalar
    for _ in 0..3 {
        iface.add_method(MethodSignature::new(reg)); // 8 GetUInt8 through 10 GetUInt16
    }
    iface.add_method(MethodSignature::new(reg).add_out(reg.i32_type())); // 11 GetInt32
    for _ in 0..6 {
        iface.add_method(MethodSignature::new(reg)); // 12 GetUInt32 through 17 GetChar16
    }
    iface.add_method(MethodSignature::new(reg).add_out(reg.bool_type())); // 18 GetBoolean
    iface.add_method(MethodSignature::new(reg).add_out(reg.hstring())); // 19 GetString
    iface
}

fn create_property_value(
    statics: &WinRTValue,
    iface: &InterfaceSignature,
    vtable_index: usize,
    arg: WinRTValue,
) -> windows_core::Result<WinRTValue> {
    let statics_obj = statics.as_object().expect("statics object");
    Ok(iface.methods[vtable_index].call_dynamic(statics_obj.as_raw(), &[arg])?[0].clone())
}

fn as_property_value(value: &WinRTValue) -> WinRTValue {
    value
        .cast(&IPropertyValue::IID)
        .expect("IPropertyValue interface")
}

fn check_winrt_uri_factory_dynamic_properties_are_golden() -> windows_core::Result<()> {
    let reg = MetadataTable::new();
    let uri = create_uri_dynamic(&reg, "https://www.example.com/a/b?q=2#frag")?;
    let uri_obj = uri.as_object().expect("uri object");
    let iface = uri_runtime_class_signature(&reg);

    assert_hstring(
        &iface.methods[6].call_dynamic(uri_obj.as_raw(), &[])?[0],
        "https://www.example.com/a/b?q=2#frag",
    );
    assert_hstring(
        &iface.methods[8].call_dynamic(uri_obj.as_raw(), &[])?[0],
        "example.com",
    );
    assert_hstring(
        &iface.methods[10].call_dynamic(uri_obj.as_raw(), &[])?[0],
        "#frag",
    );
    assert_hstring(
        &iface.methods[11].call_dynamic(uri_obj.as_raw(), &[])?[0],
        "www.example.com",
    );
    assert_hstring(
        &iface.methods[13].call_dynamic(uri_obj.as_raw(), &[])?[0],
        "/a/b",
    );
    assert_hstring(
        &iface.methods[14].call_dynamic(uri_obj.as_raw(), &[])?[0],
        "?q=2",
    );
    assert_hstring(
        &iface.methods[16].call_dynamic(uri_obj.as_raw(), &[])?[0],
        "https://www.example.com/a/b?q=2#frag",
    );
    assert_hstring(
        &iface.methods[17].call_dynamic(uri_obj.as_raw(), &[])?[0],
        "https",
    );
    assert_eq!(
        iface.methods[19].call_dynamic(uri_obj.as_raw(), &[])?[0]
            .as_i32()
            .unwrap(),
        443
    );

    Ok(())
}

fn check_winrt_uri_empty_path_is_golden() -> windows_core::Result<()> {
    let reg = MetadataTable::new();
    let uri = create_uri_dynamic(&reg, "https://www.example.com")?;
    let uri_obj = uri.as_object().expect("uri object");
    let iface = uri_runtime_class_signature(&reg);

    assert_hstring(
        &iface.methods[6].call_dynamic(uri_obj.as_raw(), &[])?[0],
        "https://www.example.com/",
    );
    assert_hstring(
        &iface.methods[13].call_dynamic(uri_obj.as_raw(), &[])?[0],
        "/",
    );
    assert_hstring(
        &iface.methods[14].call_dynamic(uri_obj.as_raw(), &[])?[0],
        "",
    );
    assert_hstring(
        &iface.methods[17].call_dynamic(uri_obj.as_raw(), &[])?[0],
        "https",
    );
    assert_hstring(
        &iface.methods[18].call_dynamic(uri_obj.as_raw(), &[])?[0],
        "",
    );
    assert_eq!(
        iface.methods[19].call_dynamic(uri_obj.as_raw(), &[])?[0]
            .as_i32()
            .unwrap(),
        443
    );

    Ok(())
}

fn check_property_value_dynamic_scalar_round_trips_are_golden() -> windows_core::Result<()> {
    let reg = MetadataTable::new();
    let statics =
        WinRTValue::from_activation_factory(&HSTRING::from("Windows.Foundation.PropertyValue"))
            .expect("PropertyValue activation factory")
            .cast(&GUID::from_u128(0x629BDBC8_D932_4FF4_96B9_8D96C5C1E858))
            .expect("IPropertyValueStatics");
    let statics_iface = property_value_statics_signature(&reg);
    let value_iface = property_value_signature(&reg);

    let int_value = as_property_value(&create_property_value(
        &statics,
        &statics_iface,
        10,
        WinRTValue::I32(-12345),
    )?);
    let int_obj = int_value.as_object().expect("IPropertyValue int object");
    assert_eq!(
        value_iface.methods[6].call_dynamic(int_obj.as_raw(), &[])?[0]
            .as_i32()
            .unwrap(),
        4
    );
    assert_bool(
        &value_iface.methods[7].call_dynamic(int_obj.as_raw(), &[])?[0],
        false,
    );
    assert_eq!(
        value_iface.methods[11].call_dynamic(int_obj.as_raw(), &[])?[0]
            .as_i32()
            .unwrap(),
        -12345
    );

    let bool_value = as_property_value(&create_property_value(
        &statics,
        &statics_iface,
        17,
        WinRTValue::Bool(true),
    )?);
    let bool_obj = bool_value.as_object().expect("IPropertyValue bool object");
    assert_eq!(
        value_iface.methods[6].call_dynamic(bool_obj.as_raw(), &[])?[0]
            .as_i32()
            .unwrap(),
        11
    );
    assert_bool(
        &value_iface.methods[7].call_dynamic(bool_obj.as_raw(), &[])?[0],
        false,
    );
    assert_bool(
        &value_iface.methods[18].call_dynamic(bool_obj.as_raw(), &[])?[0],
        true,
    );

    let string_value = as_property_value(&create_property_value(
        &statics,
        &statics_iface,
        18,
        WinRTValue::HString(HSTRING::from("dynwinrt regression")),
    )?);
    let string_obj = string_value
        .as_object()
        .expect("IPropertyValue string object");
    assert_eq!(
        value_iface.methods[6].call_dynamic(string_obj.as_raw(), &[])?[0]
            .as_i32()
            .unwrap(),
        12
    );
    assert_bool(
        &value_iface.methods[7].call_dynamic(string_obj.as_raw(), &[])?[0],
        false,
    );
    assert_hstring(
        &value_iface.methods[19].call_dynamic(string_obj.as_raw(), &[])?[0],
        "dynwinrt regression",
    );

    Ok(())
}

fn check_property_value_dynamic_type_mismatch_returns_golden_error() -> windows_core::Result<()> {
    let reg = MetadataTable::new();
    let statics =
        WinRTValue::from_activation_factory(&HSTRING::from("Windows.Foundation.PropertyValue"))
            .expect("PropertyValue activation factory")
            .cast(&GUID::from_u128(0x629BDBC8_D932_4FF4_96B9_8D96C5C1E858))
            .expect("IPropertyValueStatics");
    let statics_iface = property_value_statics_signature(&reg);
    let value_iface = property_value_signature(&reg);
    let int_value = as_property_value(&create_property_value(
        &statics,
        &statics_iface,
        10,
        WinRTValue::I32(7),
    )?);
    let int_obj = int_value.as_object().expect("IPropertyValue int object");

    let err = value_iface.methods[19]
        .call_dynamic(int_obj.as_raw(), &[])
        .expect_err("GetString on an Int32 PropertyValue should fail");
    assert_eq!(err.code(), HRESULT(0x80028CA0u32 as i32));

    Ok(())
}

fn check_geopoint_struct_layout_and_dynamic_position_round_trip_are_golden()
-> windows_core::Result<()> {
    let reg = MetadataTable::new();
    let f64_type = reg.f64_type();
    let geo_type = reg.struct_type(
        "Windows.Devices.Geolocation.BasicGeoposition",
        &[f64_type.clone(), f64_type.clone(), f64_type],
    );
    assert_eq!(geo_type.size_of(), 24);
    assert_eq!(geo_type.align_of(), 8);
    assert_eq!(geo_type.field_offset(0), 0);
    assert_eq!(geo_type.field_offset(1), 8);
    assert_eq!(geo_type.field_offset(2), 16);

    let mut geo_value = geo_type.default_value();
    assert_eq!(geo_value.get_field::<f64>(0), 0.0);
    assert_eq!(geo_value.get_field::<f64>(1), 0.0);
    assert_eq!(geo_value.get_field::<f64>(2), 0.0);
    geo_value.set_field(0, 47.643);
    geo_value.set_field(1, -122.131);
    geo_value.set_field(2, 100.5);

    let projected = Geopoint::Create(BasicGeoposition {
        Latitude: 47.643,
        Longitude: -122.131,
        Altitude: 100.5,
    })?;
    let projected_position = projected.Position()?;
    assert!((projected_position.Latitude - 47.643).abs() < 1e-6);
    assert!((projected_position.Longitude + 122.131).abs() < 1e-6);
    assert!((projected_position.Altitude - 100.5).abs() < 1e-6);

    let factory =
        WinRTValue::from_activation_factory(&HSTRING::from("Windows.Devices.Geolocation.Geopoint"))
            .expect("Geopoint activation factory")
            .cast(&IGeopointFactory::IID)
            .expect("IGeopointFactory");
    let mut factory_iface = InterfaceSignature::define_from_iinspectable(
        "IGeopointFactory",
        IGeopointFactory::IID,
        &reg,
    );
    factory_iface.add_method(
        MethodSignature::new(&reg)
            .add_in(geo_type.clone())
            .add_out(reg.object()),
    );
    let factory_obj = factory.as_object().expect("factory object");
    let created = factory_iface.methods[6]
        .call_dynamic(factory_obj.as_raw(), &[WinRTValue::Struct(geo_value)])?;
    let geopoint: IGeopoint = created[0].as_object().expect("Geopoint object").cast()?;

    let mut geopoint_iface =
        InterfaceSignature::define_from_iinspectable("IGeopoint", IGeopoint::IID, &reg);
    geopoint_iface.add_method(MethodSignature::new(&reg).add_out(geo_type));
    let position = geopoint_iface.methods[6].call_dynamic(geopoint.as_raw(), &[])?;
    let data = position[0].as_struct().expect("BasicGeoposition struct");
    assert!((data.get_field::<f64>(0) - 47.643).abs() < 1e-6);
    assert!((data.get_field::<f64>(1) + 122.131).abs() < 1e-6);
    assert!((data.get_field::<f64>(2) - 100.5).abs() < 1e-6);

    Ok(())
}

#[test]
fn winrt_regression_harness_golden_behaviors() -> windows_core::Result<()> {
    init_winrt();

    check_winrt_uri_factory_dynamic_properties_are_golden()?;
    check_winrt_uri_empty_path_is_golden()?;
    check_property_value_dynamic_scalar_round_trips_are_golden()?;
    check_property_value_dynamic_type_mismatch_returns_golden_error()?;
    check_geopoint_struct_layout_and_dynamic_position_round_trip_are_golden()?;

    Ok(())
}
