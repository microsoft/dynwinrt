# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import dynwinrt_py
from dynwinrt_py import (
    DynWinRTType,
    DynWinRTMethodSig,
    DynWinRTValue,
    WinGUID,
    DynWinRTArray,
    DynWinRTStruct,
    ro_initialize,
)


def test_ro_initialize():
    """RoInitialize should succeed (or already initialized)."""
    ro_initialize(1)


def test_primitive_types():
    """All primitive type factories should return DynWinRTType instances."""
    assert DynWinRTType.i32_type() is not None
    assert DynWinRTType.i64_type() is not None
    assert DynWinRTType.hstring() is not None
    assert DynWinRTType.object() is not None
    assert DynWinRTType.f32_type() is not None
    assert DynWinRTType.f64_type() is not None
    assert DynWinRTType.u8_type() is not None
    assert DynWinRTType.u16_type() is not None
    assert DynWinRTType.u32_type() is not None
    assert DynWinRTType.u64_type() is not None
    assert DynWinRTType.i8_type() is not None
    assert DynWinRTType.i16_type() is not None
    assert DynWinRTType.bool_type() is not None
    assert DynWinRTType.guid_type() is not None
    assert DynWinRTType.char16() is not None
    assert DynWinRTType.hresult() is not None


def test_box_ireference_values():
    value_type = DynWinRTType.u32_type()
    reference_type = DynWinRTType.parameterized(
        WinGUID.parse("61c17706-2d65-11e0-9ae8-d48564015472"),
        [value_type],
    )
    reference = DynWinRTType.register_interface(
        "IReference_UInt32_Test", reference_type.iid()
    ).add_method("get_Value", DynWinRTMethodSig().add_out(value_type))
    boxed = DynWinRTValue.box_reference(DynWinRTValue.from_u32(17), value_type)

    assert reference.method(6).invoke(boxed, []).to_number() == 17
    assert DynWinRTValue.box_reference(
        DynWinRTValue.null_value(), value_type
    ).is_null()


def test_create_map_round_trips_values_and_validates_lengths():
    key_type = DynWinRTType.hstring()
    value_type = DynWinRTType.i32_type()
    mapping = DynWinRTValue.create_map(
        [DynWinRTValue.from_hstring("answer")],
        [DynWinRTValue.from_i32(42)],
        key_type,
        value_type,
    )
    map_type = DynWinRTType.parameterized(
        WinGUID.parse("3c2925fe-8519-45c1-aa79-197b6718c1c1"),
        [key_type, value_type],
    )
    map_interface = (
        DynWinRTType.register_interface("IMap_String_Int32_Test", map_type.iid())
        .add_method(
            "Lookup",
            DynWinRTMethodSig().add_in(key_type).add_out(value_type),
        )
        .add_method(
            "get_Size",
            DynWinRTMethodSig().add_out(DynWinRTType.u32_type()),
        )
    )
    mapping = mapping.cast(map_type.iid())

    assert map_interface.method(7).invoke(mapping, []).to_u32() == 1
    assert map_interface.method(6).invoke(
        mapping,
        [DynWinRTValue.from_hstring("answer")],
    ).to_int() == 42

    import pytest

    with pytest.raises(RuntimeError, match="same length"):
        DynWinRTValue.create_map(
            [DynWinRTValue.from_hstring("answer")],
            [],
            key_type,
            value_type,
        )


def test_guid_parse():
    """WinGUID.parse should parse valid GUIDs."""
    guid = WinGUID.parse("9e365e57-48b2-4160-956f-c7385120bbfc")
    assert guid is not None
    assert "WinGUID" in repr(guid)


def test_guid_to_string():
    guid = WinGUID.parse("9e365e57-48b2-4160-956f-c7385120bbfc")
    s = guid.to_string()
    assert "9E365E57" in s.upper() or "9e365e57" in s.lower()


def test_value_from_hstring():
    v = DynWinRTValue.from_hstring("hello")
    assert str(v) == "hello"
    assert v.to_string() == "hello"


def test_value_from_i32():
    v = DynWinRTValue.from_i32(42)
    assert v.to_int() == 42
    assert v.to_string() == "42"


def test_value_from_i64():
    v = DynWinRTValue.from_i64(123456789)
    assert v.to_int() == 123456789


def test_value_from_f64():
    v = DynWinRTValue.from_f64(3.14)
    assert abs(v.to_float() - 3.14) < 1e-10


def test_value_from_bool():
    v = DynWinRTValue.from_bool(True)
    assert v.to_int() == 1
    assert v.to_bool() is True


def test_value_from_all_scalars():
    """Test all scalar value constructors."""
    assert DynWinRTValue.from_i8(42).to_int() == 42
    assert DynWinRTValue.from_u8(200).to_int() == 200
    assert DynWinRTValue.from_i16(-100).to_int() == -100
    assert DynWinRTValue.from_u16(5000).to_int() == 5000
    assert DynWinRTValue.from_u32(123456).to_int() == 123456
    assert DynWinRTValue.from_u64(99999).to_int() == 99999
    assert abs(DynWinRTValue.from_f32(1.5).to_float() - 1.5) < 0.01


def test_null_value():
    v = DynWinRTValue.null_value()
    assert v.is_null()


def test_guid_value():
    guid = WinGUID.parse("9e365e57-48b2-4160-956f-c7385120bbfc")
    v = DynWinRTValue.from_guid(guid)
    roundtrip = v.to_guid()
    assert roundtrip is not None


def test_enum_type_and_value():
    """Create an enum type, get enum values."""
    etype = DynWinRTType.enum_type("TestEnum", ["A", "B", "C"], [0, 1, 2])
    assert DynWinRTType.get_enum_value("TestEnum", "B") == 1

    ev = DynWinRTValue.enum_value(etype, 1)
    assert ev.get_enum_int() == 1
    assert ev.get_enum_name() == "B"


def test_iid():
    """iid() should return the IID for an interface type."""
    iid = WinGUID.parse("00000002-0000-0000-0000-000000000002")
    iface = DynWinRTType.register_interface("TestIIDInterface", iid)
    result_iid = iface.iid()
    assert result_iid is not None


def test_delegate_type():
    iid = WinGUID.parse("00000003-0000-0000-0000-000000000003")
    d = DynWinRTType.delegate(iid)
    assert d is not None


def test_method_sig_builder():
    """MethodSig builder chain should work."""
    sig = DynWinRTMethodSig()
    sig2 = sig.add_in(DynWinRTType.hstring())
    sig3 = sig2.add_out(DynWinRTType.object())
    assert sig3 is not None


def test_register_interface_and_add_method():
    """Register an interface and add a method."""
    # Use a unique IID (not IUriRuntimeClass) to avoid polluting global method tables
    iid = WinGUID.parse("00000001-0000-0000-0000-000000000001")
    iface = DynWinRTType.register_interface("TestInterface", iid)
    sig = DynWinRTMethodSig().add_out(DynWinRTType.hstring())
    iface2 = iface.add_method("GetName", sig)
    handle = iface2.method(6)
    assert handle is not None


def test_uri_dynamic_invocation():
    """Full round-trip: create Uri via activation factory, read properties."""
    ro_initialize(1)

    # IUriRuntimeClassFactory IID
    factory_iid = WinGUID.parse("44a9796f-723e-4fdf-a218-033e75b0c084")
    # IUriRuntimeClass IID
    uri_iid = WinGUID.parse("9e365e57-48b2-4160-956f-c7385120bbfc")

    # Register IUriRuntimeClassFactory interface
    factory_type = DynWinRTType.register_interface("IUriRuntimeClassFactory", factory_iid)
    create_uri_sig = DynWinRTMethodSig().add_in(DynWinRTType.hstring()).add_out(DynWinRTType.object())
    factory_type = factory_type.add_method("CreateUri", create_uri_sig)

    # Register IUriRuntimeClass interface
    uri_type = DynWinRTType.register_interface("IUriRuntimeClass", uri_iid)
    uri_type = uri_type.add_method("get_AbsoluteUri", DynWinRTMethodSig().add_out(DynWinRTType.hstring()))
    uri_type = uri_type.add_method("get_DisplayUri", DynWinRTMethodSig().add_out(DynWinRTType.hstring()))
    uri_type = uri_type.add_method("get_Domain", DynWinRTMethodSig().add_out(DynWinRTType.hstring()))
    uri_type = uri_type.add_method("get_Extension", DynWinRTMethodSig().add_out(DynWinRTType.hstring()))
    uri_type = uri_type.add_method("get_Fragment", DynWinRTMethodSig().add_out(DynWinRTType.hstring()))
    uri_type = uri_type.add_method("get_Host", DynWinRTMethodSig().add_out(DynWinRTType.hstring()))
    uri_type = uri_type.add_method("get_Password", DynWinRTMethodSig().add_out(DynWinRTType.hstring()))
    uri_type = uri_type.add_method("get_Path", DynWinRTMethodSig().add_out(DynWinRTType.hstring()))
    uri_type = uri_type.add_method("get_Query", DynWinRTMethodSig().add_out(DynWinRTType.hstring()))
    uri_type = uri_type.add_method("get_QueryParsed", DynWinRTMethodSig().add_out(DynWinRTType.object()))
    uri_type = uri_type.add_method("get_RawUri", DynWinRTMethodSig().add_out(DynWinRTType.hstring()))
    uri_type = uri_type.add_method("get_SchemeName", DynWinRTMethodSig().add_out(DynWinRTType.hstring()))
    uri_type = uri_type.add_method("get_UserName", DynWinRTMethodSig().add_out(DynWinRTType.hstring()))
    uri_type = uri_type.add_method("get_Port", DynWinRTMethodSig().add_out(DynWinRTType.i32_type()))
    uri_type = uri_type.add_method("get_Suspicious", DynWinRTMethodSig().add_out(DynWinRTType.bool_type()))

    # Get activation factory
    factory = DynWinRTValue.activation_factory("Windows.Foundation.Uri")
    factory_obj = factory.cast(factory_iid)

    # Create a Uri
    create_method = factory_type.method_by_name("CreateUri")
    uri_obj = create_method.invoke(factory_obj, [DynWinRTValue.from_hstring("https://example.com/path?q=1")])

    # Cast to IUriRuntimeClass
    uri_casted = uri_obj.cast(uri_iid)

    # Test fast-path getters
    get_host = uri_type.method_by_name("get_Host")
    assert get_host.get_string(uri_casted) == "example.com"

    get_port = uri_type.method_by_name("get_Port")
    assert get_port.get_i32(uri_casted) == 443

    get_suspicious = uri_type.method_by_name("get_Suspicious")
    assert get_suspicious.get_bool(uri_casted) is False

    get_scheme = uri_type.method_by_name("get_SchemeName")
    assert get_scheme.get_string(uri_casted) == "https"

    # Test invoke path
    get_abs = uri_type.method_by_name("get_AbsoluteUri")
    abs_uri = get_abs.invoke(uri_casted, [])
    assert abs_uri.to_string() == "https://example.com/path?q=1"

    # Test invoke_hstring (CreateUri is hstring -> object)
    uri2 = create_method.invoke_hstring(factory_obj, "https://test.com")
    assert uri2 is not None

    objects = DynWinRTArray.from_object_values(
        [uri_obj, uri2],
        DynWinRTType.object(),
    )
    assert objects.get(0).identity_raw() == uri_obj.identity_raw()
    assert objects.get(1).identity_raw() == uri2.identity_raw()


def test_invoke_detached_uses_the_normal_marshalling_path():
    ro_initialize(1)
    factory_iid = WinGUID.parse("44a9796f-723e-4fdf-a218-033e75b0c084")
    factory_type = DynWinRTType.register_interface(
        "IUriRuntimeClassFactoryDetachedTest", factory_iid
    ).add_method(
        "CreateUri",
        DynWinRTMethodSig()
        .add_in(DynWinRTType.hstring())
        .add_out(DynWinRTType.object()),
    )
    factory = DynWinRTValue.activation_factory("Windows.Foundation.Uri").cast(
        factory_iid
    )

    result = factory_type.method(6).invoke_detached(
        factory, [DynWinRTValue.from_hstring("https://example.com/detached")]
    )

    assert not result.is_null()


def test_uri_get_query_parsed():
    """Test get_obj fast-path via QueryParsed."""
    ro_initialize(1)

    factory_iid = WinGUID.parse("44a9796f-723e-4fdf-a218-033e75b0c084")
    uri_iid = WinGUID.parse("9e365e57-48b2-4160-956f-c7385120bbfc")

    factory_type = DynWinRTType.register_interface("IUriRTFactory2", factory_iid)
    factory_type = factory_type.add_method("CreateUri", DynWinRTMethodSig().add_in(DynWinRTType.hstring()).add_out(DynWinRTType.object()))

    uri_type = DynWinRTType.register_interface("IUriRT2", uri_iid)
    # Skip methods up to QueryParsed (index 6..15)
    for name in ["get_AbsoluteUri", "get_DisplayUri", "get_Domain", "get_Extension",
                  "get_Fragment", "get_Host", "get_Password", "get_Path", "get_Query"]:
        uri_type = uri_type.add_method(name, DynWinRTMethodSig().add_out(DynWinRTType.hstring()))
    uri_type = uri_type.add_method("get_QueryParsed", DynWinRTMethodSig().add_out(DynWinRTType.object()))

    factory = DynWinRTValue.activation_factory("Windows.Foundation.Uri")
    factory_obj = factory.cast(factory_iid)
    create = factory_type.method_by_name("CreateUri")
    uri_obj = create.invoke(factory_obj, [DynWinRTValue.from_hstring("https://example.com?a=1")])
    uri_casted = uri_obj.cast(uri_iid)

    get_qp = uri_type.method_by_name("get_QueryParsed")
    qp = get_qp.get_obj(uri_casted)
    assert qp is not None
    assert not qp.is_null()


def test_array_from_i32():
    arr = DynWinRTArray.from_i32_values([1, 2, 3, 4, 5])
    assert len(arr) == 5
    assert arr.get(0).to_int() == 1
    assert arr.get(4).to_int() == 5
    assert arr.to_i32_list() == [1, 2, 3, 4, 5]


def test_array_from_f64():
    arr = DynWinRTArray.from_f64_values([1.5, 2.5, 3.5])
    assert len(arr) == 3
    assert arr.to_f64_list() == [1.5, 2.5, 3.5]


def test_array_from_u8():
    arr = DynWinRTArray.from_u8_values([0, 127, 255])
    assert len(arr) == 3
    assert arr.to_u8_list() == bytes([0, 127, 255])


def test_array_all_types():
    """Test all array constructor/conversion pairs."""
    assert DynWinRTArray.from_i8_values([1, -1]).to_i8_list() == [1, -1]
    assert DynWinRTArray.from_i16_values([100, -100]).to_i16_list() == [100, -100]
    assert DynWinRTArray.from_u16_values([1000, 2000]).to_u16_list() == [1000, 2000]
    assert DynWinRTArray.from_u32_values([10, 20]).to_u32_list() == [10, 20]
    assert DynWinRTArray.from_i64_values([99, -99]).to_i64_list() == [99, -99]
    assert DynWinRTArray.from_u64_values([42, 84]).to_u64_list() == [42, 84]
    assert DynWinRTArray.from_f32_values([1.0, 2.0]).to_f32_list() == [1.0, 2.0]
    assert DynWinRTArray.from_string_values(["a", "b"]).to_string_list() == ["a", "b"]


def test_array_to_value():
    """Array can be wrapped as DynWinRTValue."""
    arr = DynWinRTArray.from_i32_values([10, 20])
    val = arr.to_value()
    assert val.is_array()
    roundtrip = val.as_array()
    assert len(roundtrip) == 2


def test_struct_create_and_field_access():
    """Create a struct and get/set fields."""
    typ = DynWinRTType.struct_type("TestStruct1", [DynWinRTType.i32_type(), DynWinRTType.f64_type()])
    s = DynWinRTStruct.create(typ)
    assert s.get_i32(0) == 0
    s.set_i32(0, 42)
    assert s.get_i32(0) == 42
    s.set_f64(1, 3.14)
    assert abs(s.get_f64(1) - 3.14) < 1e-10


def test_struct_to_value():
    typ = DynWinRTType.struct_type("TestStruct2", [DynWinRTType.u32_type()])
    s = DynWinRTStruct.create(typ)
    s.set_u32(0, 99)
    val = s.to_value()
    assert val.is_struct()


def test_struct_array_round_trip():
    typ = DynWinRTType.struct_type(
        "TestStructArray",
        [DynWinRTType.i32_type(), DynWinRTType.f64_type()],
    )
    first = DynWinRTStruct.create(typ)
    first.set_i32(0, 17)
    first.set_f64(1, 1.5)
    second = DynWinRTStruct.create(typ)
    second.set_i32(0, 23)
    second.set_f64(1, 2.5)

    array = DynWinRTArray.from_values(
        [first.to_value(), second.to_value()],
        typ,
    )

    assert len(array) == 2
    assert array.get(0).as_struct().get_i32(0) == 17
    assert array.get(0).as_struct().get_f64(1) == 1.5
    assert array.get(1).as_struct().get_i32(0) == 23
    assert array.get(1).as_struct().get_f64(1) == 2.5


def test_struct_hstring_field_round_trip():
    typ = DynWinRTType.struct_type(
        "TestStructHString",
        [DynWinRTType.i32_type(), DynWinRTType.hstring()],
    )
    value = DynWinRTStruct.create(typ)
    value.set_i32(0, 17)
    value.set_hstring(1, "dynwinrt")

    assert value.get_i32(0) == 17
    assert value.get_hstring(1) == "dynwinrt"
    assert value.to_value().as_struct().get_hstring(1) == "dynwinrt"


def test_struct_all_field_types():
    """Test all blittable struct field types."""
    typ = DynWinRTType.struct_type("TestStruct3", [
        DynWinRTType.i8_type(), DynWinRTType.u8_type(),
        DynWinRTType.i16_type(), DynWinRTType.u16_type(),
        DynWinRTType.i64_type(), DynWinRTType.u64_type(),
    ])
    s = DynWinRTStruct.create(typ)
    s.set_i8(0, -5)
    assert s.get_i8(0) == -5
    s.set_u8(1, 200)
    assert s.get_u8(1) == 200
    s.set_i16(2, -1000)
    assert s.get_i16(2) == -1000
    s.set_u16(3, 50000)
    assert s.get_u16(3) == 50000
    s.set_i64(4, -999999)
    assert s.get_i64(4) == -999999
    s.set_u64(5, 12345678)
    assert s.get_u64(5) == 12345678
