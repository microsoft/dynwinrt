# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import sys
import sysconfig

import dynwinrt
import pytest
from dynwinrt import (
    DynWinRTType,
    DynWinRTMethodSig,
    DynWinRTValue,
    WinGUID,
    DynWinRTArray,
    DynWinRTStruct,
    DynWinRtDelegate,
    ro_initialize,
)


def test_ro_initialize():
    """RoInitialize should succeed (or already initialized)."""
    ro_initialize(1)


def test_delegate_creation_failures_are_python_exceptions():
    object_type = DynWinRTType.object()
    with pytest.raises(OSError, match="supports up to 2 parameters"):
        DynWinRtDelegate.create(
            WinGUID.parse("699a62d5-a8a5-431c-9c00-a75c70b30524"),
            [object_type, object_type, object_type],
            lambda *_args: None,
        )

    empty_struct = DynWinRTType.struct_type(
        "DynWinRT.Tests.EmptyDelegateArgument",
        [],
    )
    with pytest.raises(OSError, match="struct callbacks do not support"):
        DynWinRtDelegate.create(
            WinGUID.parse("f1662550-78fd-4bf1-9022-0a2c85168380"),
            [object_type, empty_struct],
            lambda *_args: None,
        )


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


def test_create_map_duplicate_keys_keep_the_last_value():
    key_type = DynWinRTType.hstring()
    value_type = DynWinRTType.i32_type()
    reader = _collection_map_reader(key_type, value_type)
    keys = [DynWinRTValue.from_hstring(key) for key in ["A", "B", "A"]]
    values = [DynWinRTValue.from_i32(value) for value in [1, 2, 3]]
    mapping = DynWinRTValue.create_map(keys, values, key_type, value_type)
    try:
        assert _invoke_collection_reader(reader, mapping, 7, []).to_u32() == 2
        for key, expected in [("A", 3), ("B", 2)]:
            assert _invoke_collection_reader(
                reader, mapping, 6, [DynWinRTValue.from_hstring(key)]
            ).to_int() == expected
        assert keys[0].to_string() == "A"
        assert values[0].to_int() == 1
        with pytest.raises(OSError):
            DynWinRTValue.create_map(
                [keys[0], keys[0]],
                [values[0], DynWinRTValue.from_hstring("invalid later duplicate")],
                key_type,
                value_type,
            )
    finally:
        mapping.release()


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


def test_narrow_array_constructors_enforce_boundaries():
    assert DynWinRTArray.from_i8_values([-128, 127]).to_i8_list() == [-128, 127]
    assert DynWinRTArray.from_u8_values([0, 255]).to_u8_list() == bytes([0, 255])
    assert DynWinRTArray.from_i16_values([-32768, 32767]).to_i16_list() == [
        -32768,
        32767,
    ]
    assert DynWinRTArray.from_u16_values([0, 0xFFFF]).to_u16_list() == [0, 0xFFFF]

    with pytest.raises(OverflowError):
        DynWinRTArray.from_i8_values([-129])
    with pytest.raises(OverflowError):
        DynWinRTArray.from_i8_values([128])
    with pytest.raises(OverflowError):
        DynWinRTArray.from_u8_values([-1])
    with pytest.raises(OverflowError):
        DynWinRTArray.from_u8_values([256])
    with pytest.raises(OverflowError):
        DynWinRTArray.from_i16_values([-32769])
    with pytest.raises(OverflowError):
        DynWinRTArray.from_i16_values([32768])
    with pytest.raises(OverflowError):
        DynWinRTArray.from_u16_values([-1])
    with pytest.raises(OverflowError):
        DynWinRTArray.from_u16_values([0x1_0000])


def test_array_get_invalid_index_raises_index_error():
    arr = DynWinRTArray.from_i32_values([1, 2, 3])

    with pytest.raises(IndexError):
        arr.get(-1)
    with pytest.raises(IndexError):
        arr.get(3)


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


def test_struct_enum_field_uses_underlying_integer_accessor():
    enum_type = DynWinRTType.enum_type("TestStructEnum", ["A", "B"], [0, 1])
    typ = DynWinRTType.struct_type("TestStructWithEnum", [enum_type])
    value = DynWinRTStruct.create(typ)

    value.set_i32(0, 1)
    assert value.get_i32(0) == 1


def test_struct_bool_and_hresult_use_abi_integer_accessors():
    typ = DynWinRTType.struct_type(
        "TestStructAbiIntegerAliases",
        [DynWinRTType.bool_type(), DynWinRTType.hresult()],
    )
    value = DynWinRTStruct.create(typ)

    value.set_u8(0, 1)
    value.set_i32(1, -1)
    assert value.get_u8(0) == 1
    assert value.get_i32(1) == -1


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


def test_struct_narrow_setters_enforce_boundaries_and_char16_range():
    typ = DynWinRTType.struct_type(
        "TestStructNarrowBounds",
        [
            DynWinRTType.i8_type(),
            DynWinRTType.u8_type(),
            DynWinRTType.i16_type(),
            DynWinRTType.u16_type(),
            DynWinRTType.char16(),
        ],
    )
    value = DynWinRTStruct.create(typ)

    value.set_i8(0, -128)
    assert value.get_i8(0) == -128
    value.set_i8(0, 127)
    assert value.get_i8(0) == 127
    value.set_u8(1, 255)
    assert value.get_u8(1) == 255
    value.set_i16(2, -32768)
    assert value.get_i16(2) == -32768
    value.set_i16(2, 32767)
    assert value.get_i16(2) == 32767
    value.set_u16(3, 0xFFFF)
    assert value.get_u16(3) == 0xFFFF
    value.set_u16(4, 0xFFFF)
    assert value.get_u16(4) == 0xFFFF

    with pytest.raises(OverflowError):
        value.set_i8(0, -129)
    with pytest.raises(OverflowError):
        value.set_i8(0, 128)
    with pytest.raises(OverflowError):
        value.set_u8(1, -1)
    with pytest.raises(OverflowError):
        value.set_u8(1, 256)
    with pytest.raises(OverflowError):
        value.set_i16(2, -32769)
    with pytest.raises(OverflowError):
        value.set_i16(2, 32768)
    with pytest.raises(OverflowError):
        value.set_u16(3, -1)
    with pytest.raises(OverflowError):
        value.set_u16(3, 0x1_0000)
    with pytest.raises(OverflowError):
        value.set_u16(4, 0x1_0000)


def test_struct_indexed_accessors_raise_index_error_for_invalid_indices():
    typ = DynWinRTType.struct_type("TestStructIndexErrors", [DynWinRTType.i32_type()])
    value = DynWinRTStruct.create(typ)
    inner = DynWinRTStruct.create(
        DynWinRTType.struct_type("TestStructIndexErrorsInner", [DynWinRTType.i32_type()])
    )

    with pytest.raises(IndexError):
        value.get_i32(-1)
    with pytest.raises(IndexError):
        value.get_guid(1)
    with pytest.raises(IndexError):
        value.set_hstring(1, "bad")
    with pytest.raises(IndexError):
        value.get_struct(1)
    with pytest.raises(IndexError):
        value.set_struct(1, inner)
    with pytest.raises(IndexError):
        value.get_object(1)
    with pytest.raises(IndexError):
        value.set_object(1, DynWinRTValue.null_value())


def test_struct_indexed_accessors_raise_runtime_error_for_wrong_field_shape():
    typ = DynWinRTType.struct_type("TestStructWrongFieldShape", [DynWinRTType.i32_type()])
    value = DynWinRTStruct.create(typ)
    inner = DynWinRTStruct.create(
        DynWinRTType.struct_type("TestStructWrongFieldShapeInner", [DynWinRTType.i32_type()])
    )
    guid = WinGUID.parse("9e365e57-48b2-4160-956f-c7385120bbfc")

    with pytest.raises(RuntimeError):
        value.get_i8(0)
    with pytest.raises(RuntimeError):
        value.set_i8(0, 1)
    with pytest.raises(RuntimeError):
        value.get_hstring(0)
    with pytest.raises(RuntimeError):
        value.set_hstring(0, "bad")
    with pytest.raises(RuntimeError):
        value.get_guid(0)
    with pytest.raises(RuntimeError):
        value.set_guid(0, guid)
    with pytest.raises(RuntimeError):
        value.get_struct(0)
    with pytest.raises(RuntimeError):
        value.set_struct(0, inner)
    with pytest.raises(RuntimeError):
        value.get_object(0)
    with pytest.raises(RuntimeError):
        value.set_object(0, DynWinRTValue.null_value())
    with pytest.raises(RuntimeError):
        DynWinRTStruct.create(DynWinRTType.i32_type()).get_i32(0)


def _collection_producer_cases(element_type, items):
    indexes = [DynWinRTValue.from_i32(index) for index in range(len(items))]
    return [
        lambda: DynWinRTValue.create_vector(items, element_type),
        lambda: DynWinRTValue.create_map(
            items, indexes, element_type, DynWinRTType.i32_type()
        ),
        lambda: DynWinRTValue.create_map(
            indexes, items, DynWinRTType.i32_type(), element_type
        ),
    ]


def _collection_vector_reader(element_type):
    iid = DynWinRTType.parameterized(
        WinGUID.parse("913337e9-11a1-4345-a3a2-4e7f956e222d"), [element_type]
    ).iid()
    return (
        DynWinRTType.register_interface(
            f"CollectionBoundary.Vector.{iid.to_string()}", iid
        )
        .add_method(
            "GetAt",
            DynWinRTMethodSig().add_in(DynWinRTType.u32_type()).add_out(element_type),
        )
        .add_method("get_Size", DynWinRTMethodSig().add_out(DynWinRTType.u32_type()))
    )


def _collection_map_reader(key_type, value_type):
    iid = DynWinRTType.parameterized(
        WinGUID.parse("3c2925fe-8519-45c1-aa79-197b6718c1c1"),
        [key_type, value_type],
    ).iid()
    return (
        DynWinRTType.register_interface(
            f"CollectionBoundary.Map.{iid.to_string()}", iid
        )
        .add_method("Lookup", DynWinRTMethodSig().add_in(key_type).add_out(value_type))
        .add_method("get_Size", DynWinRTMethodSig().add_out(DynWinRTType.u32_type()))
    )


def _invoke_collection_reader(reader, collection, slot, args):
    typed = collection.cast(reader.iid())
    try:
        return reader.method(slot).invoke(typed, args)
    finally:
        typed.release()


def _collection_producer_round_trips(element_type, item, lookup_key=None):
    if lookup_key is None:
        lookup_key = item
    vector, key_map, value_map = [
        create() for create in _collection_producer_cases(element_type, [item])
    ]
    try:
        return [
            _invoke_collection_reader(
                _collection_vector_reader(element_type), vector, 6,
                [DynWinRTValue.from_u32(0)],
            ),
            _invoke_collection_reader(
                _collection_map_reader(DynWinRTType.i32_type(), element_type),
                value_map, 6, [DynWinRTValue.from_i32(0)],
            ),
            _invoke_collection_reader(
                _collection_map_reader(element_type, DynWinRTType.i32_type()),
                key_map, 6, [lookup_key],
            ),
        ]
    finally:
        vector.release()
        key_map.release()
        value_map.release()


def test_collection_producers_reject_recursively_owned_structs_even_when_empty():
    reference_type = DynWinRTType.parameterized(
        WinGUID.parse("61c17706-2d65-11e0-9ae8-d48564015472"),
        [DynWinRTType.u32_type()],
    )
    for name, field_type in [
        ("String", DynWinRTType.hstring()),
        ("Reference", reference_type),
    ]:
        inner = DynWinRTType.struct_type(
            f"CollectionBoundary.Owned{name}", [field_type]
        )
        outer = DynWinRTType.struct_type(
            f"CollectionBoundary.NestedOwned{name}", [inner]
        )
        for typ in [inner, outer]:
            for items in [[], [DynWinRTStruct.create(typ).to_value()]]:
                for create in _collection_producer_cases(typ, items):
                    with pytest.raises(RuntimeError):
                        create()


def test_collection_producers_require_exact_struct_and_enum_identities():
    expected = DynWinRTType.struct_type(
        "CollectionBoundary.Expected", [DynWinRTType.i32_type()]
    )
    wrong_structs = [
        DynWinRTType.struct_type(
            "CollectionBoundary.SameShape", [DynWinRTType.i32_type()]
        ),
        DynWinRTType.struct_type(
            "CollectionBoundary.Smaller", [DynWinRTType.u8_type()]
        ),
        DynWinRTType.struct_type(
            "CollectionBoundary.CrossType", [DynWinRTType.u32_type()]
        ),
    ]
    enum_type = DynWinRTType.enum_type("CollectionBoundary.Enum", ["One"], [1])
    other_enum = DynWinRTType.enum_type("CollectionBoundary.OtherEnum", ["One"], [1])
    cases = [
        *[(expected, DynWinRTStruct.create(typ).to_value()) for typ in wrong_structs],
        (expected, DynWinRTValue.from_i32(1)),
        (DynWinRTType.i32_type(), DynWinRTStruct.create(expected).to_value()),
        (enum_type, DynWinRTValue.enum_value(other_enum, 1)),
        (DynWinRTType.i32_type(), DynWinRTValue.from_u32(1)),
        (DynWinRTType.bool_type(), DynWinRTValue.from_u8(1)),
    ]
    for typ, value in cases:
        for create in _collection_producer_cases(typ, [value]):
            with pytest.raises(OSError):
                create()


def test_collection_producers_preserve_exact_scalars_and_checked_projection_aliases():
    enum_type = DynWinRTType.enum_type("CollectionBoundary.ScalarEnum", ["One"], [1])
    cases = [
        (DynWinRTType.bool_type(), DynWinRTValue.from_bool(True), 1),
        (DynWinRTType.i8_type(), DynWinRTValue.from_i8(-128), -128),
        (DynWinRTType.u8_type(), DynWinRTValue.from_u8(255), 255),
        (DynWinRTType.i16_type(), DynWinRTValue.from_i16(-32768), -32768),
        (DynWinRTType.u16_type(), DynWinRTValue.from_u16(65535), 65535),
        (DynWinRTType.i32_type(), DynWinRTValue.from_i32(-123), -123),
        (DynWinRTType.u32_type(), DynWinRTValue.from_u32(0xFFFFFFFF), 0xFFFFFFFF),
        (DynWinRTType.char16(), DynWinRTValue.from_u16(0x03BB), 0x03BB),
        (enum_type, DynWinRTValue.enum_value(enum_type, 1), 1),
        (enum_type, DynWinRTValue.from_i32(1), 1),
        # HRESULT and I32 share collection IIDs; Lookup uses the common I32 projection.
        (DynWinRTType.hresult(), DynWinRTValue.from_hresult(-1), -1,
         DynWinRTValue.from_i32(-1)),
        (DynWinRTType.hresult(), DynWinRTValue.from_i32(-1), -1),
        (DynWinRTType.i8_type(), DynWinRTValue.from_i32(-128), -128),
        (DynWinRTType.i8_type(), DynWinRTValue.from_i32(127), 127),
        (DynWinRTType.u8_type(), DynWinRTValue.from_i32(0), 0),
        (DynWinRTType.u8_type(), DynWinRTValue.from_i32(255), 255),
        (DynWinRTType.char16(), DynWinRTValue.from_i32(65535), 65535),
    ]
    for typ, value, expected, *lookup_keys in cases:
        from_vector, from_map, key_lookup = _collection_producer_round_trips(
            typ, value, lookup_keys[0] if lookup_keys else value
        )
        assert from_vector.to_number() == expected
        assert from_map.to_number() == expected
        assert key_lookup.to_number() == 0

    for typ, values in [
        (DynWinRTType.i8_type(), [-129, 128]),
        (DynWinRTType.u8_type(), [-1, 256, 257]),
        (DynWinRTType.char16(), [-1, 65536]),
    ]:
        for value in values:
            for create in _collection_producer_cases(typ, [DynWinRTValue.from_i32(value)]):
                with pytest.raises(OSError):
                    create()


def test_collection_producers_retain_direct_strings_and_nullable_references_with_expected_iid():
    ro_initialize(1)
    from_vector, from_map, key_lookup = _collection_producer_round_trips(
        DynWinRTType.hstring(), DynWinRTValue.from_hstring("owned \u03bb")
    )
    assert from_vector.to_string() == "owned \u03bb"
    assert from_map.to_string() == "owned \u03bb"
    assert key_lookup.to_number() == 0

    reference_type = DynWinRTType.parameterized(
        WinGUID.parse("61c17706-2d65-11e0-9ae8-d48564015472"),
        [DynWinRTType.u32_type()],
    )
    reference = DynWinRTType.register_interface(
        "CollectionBoundary.Reference", reference_type.iid()
    ).add_method("get_Value", DynWinRTMethodSig().add_out(DynWinRTType.u32_type()))
    for typ in [
        reference_type,
        DynWinRTType.interface(reference_type.iid()),
        DynWinRTType.object(),
    ]:
        source = DynWinRTValue.box_reference(
            DynWinRTValue.from_u32(17), DynWinRTType.u32_type()
        )
        items = [source, DynWinRTValue.null_value()]
        vector = DynWinRTValue.create_vector(items, typ)
        mapping = DynWinRTValue.create_map(
            [DynWinRTValue.from_i32(0), DynWinRTValue.from_i32(1)],
            items, DynWinRTType.i32_type(), typ,
        )
        source.release()
        try:
            vector_reader = _collection_vector_reader(typ)
            map_reader = _collection_map_reader(DynWinRTType.i32_type(), typ)
            for value in [
                _invoke_collection_reader(
                    vector_reader, vector, 6, [DynWinRTValue.from_u32(0)]
                ),
                _invoke_collection_reader(
                    map_reader, mapping, 6, [DynWinRTValue.from_i32(0)]
                ),
            ]:
                typed = value.cast(reference_type.iid())
                assert reference.method(6).invoke(typed, []).to_number() == 17
                typed.release()
                value.release()
            assert _invoke_collection_reader(
                vector_reader, vector, 6, [DynWinRTValue.from_u32(1)]
            ).is_null()
            assert _invoke_collection_reader(
                map_reader, mapping, 6, [DynWinRTValue.from_i32(1)]
            ).is_null()
        finally:
            vector.release()
            mapping.release()

    incompatible = DynWinRTValue.activation_factory("Windows.Foundation.Uri")
    try:
        for value in [incompatible, DynWinRTValue.from_i32(17)]:
            for create in _collection_producer_cases(reference_type, [value]):
                with pytest.raises(OSError):
                    create()
    finally:
        incompatible.release()


def test_pod_vectors_work_without_broadening_architecture_specific_map_boundary():
    is_x86 = sys.maxsize == 2**31 - 1
    is_arm64 = sysconfig.get_platform() == "win-arm64"
    cases = [
        ("CollectionBoundary.Byte", [DynWinRTType.u8_type()], 1, False,
         "set_u8", "get_u8", 0, 201),
        ("CollectionBoundary.Short", [DynWinRTType.i16_type()], 2, False,
         "set_i16", "get_i16", 0, -1234),
        ("CollectionBoundary.Int", [DynWinRTType.i32_type()], 4, False,
         "set_i32", "get_i32", 0, 123456),
        ("Windows.Graphics.PointInt32", [DynWinRTType.i32_type()] * 2, 8, False,
         "set_i32", "get_i32", 1, -42),
        *[(f"Windows.Foundation.{name}", [DynWinRTType.f32_type()] * 2, 8, True,
           "set_f32", "get_f32", 1, 2.5) for name in ["Point", "Size"]],
    ]
    for name, fields, size, hfa, setter, getter, index, expected in cases:
        typ = DynWinRTType.struct_type(name, fields)
        value = DynWinRTStruct.create(typ)
        getattr(value, setter)(index, expected)
        supported = not ((is_x86 and size > 4) or (is_arm64 and hfa))
        for producer_index, create in enumerate(_collection_producer_cases(typ, [])):
            if producer_index == 0 or supported:
                assert not create().is_null()
            else:
                with pytest.raises(RuntimeError):
                    create()
        if supported:
            from_vector, from_map, key_lookup = _collection_producer_round_trips(
                typ, value.to_value()
            )
            assert getattr(from_vector.as_struct(), getter)(index) == expected
            assert getattr(from_map.as_struct(), getter)(index) == expected
            assert key_lookup.to_number() == 0
        else:
            vector = DynWinRTValue.create_vector([value.to_value()], typ)
            output = _invoke_collection_reader(
                _collection_vector_reader(typ), vector, 6, [DynWinRTValue.from_u32(0)]
            )
            assert getattr(output.as_struct(), getter)(index) == expected
            output.release()
            vector.release()
            for create in _collection_producer_cases(typ, [value.to_value()])[1:]:
                with pytest.raises(RuntimeError):
                    create()


def test_collection_producers_support_large_pod_vectors_but_not_maps():
    cases = [
        ("Windows.Graphics.RectInt32", [DynWinRTType.i32_type()] * 4, False, 16),
        ("Windows.Foundation.Rect", [DynWinRTType.f32_type()] * 4, True, 16),
        ("Windows.Devices.Geolocation.BasicGeoposition",
         [DynWinRTType.f64_type()] * 3, True, 24),
        ("Windows.UI.Input.ManipulationDelta", [
            DynWinRTType.struct_type(
                "Windows.Foundation.Point", [DynWinRTType.f32_type()] * 2
            ),
            DynWinRTType.f32_type(), DynWinRTType.f32_type(), DynWinRTType.f32_type(),
        ], False, 20),
    ]
    for name, fields, hfa, size in cases:
        typ = DynWinRTType.struct_type(name, fields)
        for items in [[], [DynWinRTStruct.create(typ).to_value()]]:
            vector = DynWinRTValue.create_vector(items, typ)
            assert _invoke_collection_reader(
                _collection_vector_reader(typ), vector, 7, []
            ).to_number() == len(items)
            if items:
                output = _invoke_collection_reader(
                    _collection_vector_reader(typ), vector, 6, [DynWinRTValue.from_u32(0)]
                )
                assert not output.is_null()
                output.release()
            vector.release()
            for create in _collection_producer_cases(typ, items)[1:]:
                with pytest.raises(RuntimeError):
                    create()


def test_collection_producers_reject_floating_scalars_and_guids_even_when_empty():
    cases = [
        (DynWinRTType.f32_type(), DynWinRTValue.from_f32(1.5)),
        (DynWinRTType.f64_type(), DynWinRTValue.from_f64(1.5)),
        (DynWinRTType.guid_type(), DynWinRTValue.from_guid(
            WinGUID.parse("9e365e57-48b2-4160-956f-c7385120bbfc")
        )),
    ]
    for typ, value in cases:
        for items in [[], [value]]:
            for create in _collection_producer_cases(typ, items):
                with pytest.raises(RuntimeError):
                    create()


def test_collection_producers_reject_64_bit_scalars_on_i686_even_when_empty():
    for typ, value in [
        (DynWinRTType.i64_type(), DynWinRTValue.from_i64(-123)),
        (DynWinRTType.u64_type(), DynWinRTValue.from_u64(123)),
    ]:
        if sys.maxsize == 2**31 - 1:
            for items in [[], [value]]:
                for create in _collection_producer_cases(typ, items):
                    with pytest.raises(RuntimeError):
                        create()
        else:
            from_vector, from_map, key_lookup = _collection_producer_round_trips(typ, value)
            assert from_vector.to_int() == value.to_int()
            assert from_map.to_int() == value.to_int()
            assert key_lookup.to_number() == 0
