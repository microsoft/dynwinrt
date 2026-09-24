# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Explicit WinRT ``Object`` conversion: ``unbox_object`` and ``to_winrt_object``.

Generated code keeps ``Object`` values native; these helpers convert only when
the application asks. Boxes are created here through the real
``Windows.Foundation.PropertyValue`` factory (raw vtable calls, no interface
registrations) and through ``to_winrt_object`` itself.
"""

import copy
import json
import math
import pickle
from datetime import date, datetime, timedelta, timezone
from enum import Enum, IntEnum, IntFlag, StrEnum
from uuid import UUID

import pytest

import dynwinrt
from dynwinrt import (
    DynWinRTArray,
    DynWinRTImplementation,
    DynWinRTImplementationMethod,
    DynWinRTInterfacePlan,
    DynWinRTMethodSig,
    DynWinRTStruct,
    DynWinRTType,
    DynWinRTValue,
    RoApartment,
    WinGUID,
    to_winrt_object,
    unbox_object,
)
from dynwinrt import values
from dynwinrt.values import PropertyType

IPROPERTY_VALUE = WinGUID.parse("4bd682dd-7554-40e9-9a9b-82654ede7e62")
IPROPERTY_VALUE_STATICS = WinGUID.parse("629bdbc8-d932-4ff4-96b9-8d96c5c1e858")
IURI_FACTORY = WinGUID.parse("44a9796f-723e-4fdf-a218-033e75b0c084")
E_NOTIMPL = -2147467263
UTC = timezone.utc
EPOCH = datetime(1601, 1, 1, tzinfo=UTC)


@pytest.fixture(scope="module", autouse=True)
def apartment():
    with RoApartment(1):
        yield


def property_type(raw):
    """IPropertyValue.Type through a raw vtable call."""
    view = raw.cast(IPROPERTY_VALUE)
    try:
        return PropertyType(view.call_0(6, DynWinRTType.i32_type()).to_number())
    finally:
        view.release()


def uri(text="https://example.com/"):
    factory = DynWinRTValue.activation_factory("Windows.Foundation.Uri").cast(IURI_FACTORY)
    return factory.call(
        6, DynWinRTType.object(), [DynWinRTType.hstring()], [DynWinRTValue.from_hstring(text)]
    )


# ----------------------------------------------------------------------
# Boxes created by the real PropertyValue factory
# ----------------------------------------------------------------------

I8 = DynWinRTType.i64_type()
F4 = DynWinRTType.f32_type()
DATE_TIME = DynWinRTType.struct_type("Windows.Foundation.DateTime", [I8])
TIME_SPAN = DynWinRTType.struct_type("Windows.Foundation.TimeSpan", [I8])
POINT = DynWinRTType.struct_type("Windows.Foundation.Point", [F4, F4])
SIZE = DynWinRTType.struct_type("Windows.Foundation.Size", [F4, F4])
RECT = DynWinRTType.struct_type("Windows.Foundation.Rect", [F4, F4, F4, F4])
GUID_TEXT = "01234567-89ab-cdef-0123-456789abcdef"
TICKS = 133_000_000_001_234_560  # 2022-06-18, a whole number of microseconds


def struct(typ, *fields, setter="set_f32"):
    value = DynWinRTStruct.create(typ)
    for index, field in enumerate(fields):
        getattr(value, setter)(index, field)
    return value.to_value()


def ticks(typ, value):
    return struct(typ, value, setter="set_i64")


def created(slot, in_type, argument):
    """Call IPropertyValueStatics.Create* (vtable ``slot``) directly."""
    factory = DynWinRTValue.activation_factory("Windows.Foundation.PropertyValue")
    statics = factory.cast(IPROPERTY_VALUE_STATICS)
    try:
        return statics.call(slot, DynWinRTType.object(), [in_type], [argument])
    finally:
        statics.release()
        factory.release()


def array(values_, element_type):
    return DynWinRTArray.from_values(values_, element_type).to_value()


def factory_cases():
    t = DynWinRTType
    at = DynWinRTType.array_type
    moment = EPOCH + timedelta(microseconds=TICKS // 10)
    return [
        # (PropertyType, slot, input type, input or input factory, plain value, preserved type)
        (PropertyType.UInt8, 7, t.u8_type(), DynWinRTValue.from_u8(255), 255, values.UInt8),
        (PropertyType.Int16, 8, t.i16_type(), DynWinRTValue.from_i16(-32768), -32768, values.Int16),
        (PropertyType.UInt16, 9, t.u16_type(), DynWinRTValue.from_u16(65535), 65535, values.UInt16),
        (PropertyType.Int32, 10, t.i32_type(), DynWinRTValue.from_i32(-(2**31)), -(2**31), int),
        (PropertyType.UInt32, 11, t.u32_type(), DynWinRTValue.from_u32(2**32 - 1), 2**32 - 1, values.UInt32),
        (PropertyType.Int64, 12, t.i64_type(), DynWinRTValue.from_i64(-(2**63)), -(2**63), values.Int64),
        (PropertyType.UInt64, 13, t.u64_type(), DynWinRTValue.from_u64(2**64 - 1), 2**64 - 1, values.UInt64),
        (PropertyType.Single, 14, t.f32_type(), DynWinRTValue.from_f32(0.1), values.Single(0.1), values.Single),
        (PropertyType.Double, 15, t.f64_type(), DynWinRTValue.from_f64(-0.1), -0.1, float),
        (PropertyType.Char16, 16, t.char16(), DynWinRTValue.from_u16(0xD800), "\ud800", values.Char16),
        (PropertyType.Boolean, 17, t.bool_type(), DynWinRTValue.from_bool(True), True, bool),
        (PropertyType.String, 18, t.hstring(), DynWinRTValue.from_hstring("BLE Device"), "BLE Device", str),
        (PropertyType.Guid, 20, t.guid_type(), DynWinRTValue.from_guid(WinGUID.parse(GUID_TEXT)), UUID(GUID_TEXT), UUID),
        (PropertyType.DateTime, 21, DATE_TIME, ticks(DATE_TIME, TICKS), moment, datetime),
        (PropertyType.TimeSpan, 22, TIME_SPAN, ticks(TIME_SPAN, -12_345_670), timedelta(microseconds=-1_234_567), timedelta),
        (PropertyType.Point, 23, POINT, struct(POINT, 1.5, -2.25), values.Point(1.5, -2.25), values.Point),
        (PropertyType.Size, 24, SIZE, struct(SIZE, 3.0, 4.5), values.Size(3.0, 4.5), values.Size),
        (PropertyType.Rect, 25, RECT, struct(RECT, 1, 2, 3, 4), values.Rect(1, 2, 3, 4), values.Rect),
        (PropertyType.UInt8Array, 26, at(t.u8_type()), DynWinRTArray.from_u8_values([0, 1, 255]).to_value(), b"\x00\x01\xff", bytes),
        (PropertyType.Int16Array, 27, at(t.i16_type()), DynWinRTArray.from_i16_values([-32768, 7]).to_value(), [-32768, 7], values.Int16Array),
        (PropertyType.UInt16Array, 28, at(t.u16_type()), DynWinRTArray.from_u16_values([0, 65535]).to_value(), [0, 65535], values.UInt16Array),
        (PropertyType.Int32Array, 29, at(t.i32_type()), DynWinRTArray.from_i32_values([-1, 0, 42]).to_value(), [-1, 0, 42], values.Int32Array),
        (PropertyType.UInt32Array, 30, at(t.u32_type()), DynWinRTArray.from_u32_values([0, 2**32 - 1]).to_value(), [0, 2**32 - 1], values.UInt32Array),
        (PropertyType.Int64Array, 31, at(t.i64_type()), DynWinRTArray.from_i64_values([-(2**63), 2**63 - 1]).to_value(), [-(2**63), 2**63 - 1], values.Int64Array),
        (PropertyType.UInt64Array, 32, at(t.u64_type()), DynWinRTArray.from_u64_values([0, 2**64 - 1]).to_value(), [0, 2**64 - 1], values.UInt64Array),
        (PropertyType.SingleArray, 33, at(t.f32_type()), DynWinRTArray.from_f32_values([0.5, -1.25]).to_value(), [0.5, -1.25], values.SingleArray),
        (PropertyType.DoubleArray, 34, at(t.f64_type()), DynWinRTArray.from_f64_values([0.1, 2.0]).to_value(), [0.1, 2.0], values.DoubleArray),
        (PropertyType.Char16Array, 35, at(t.char16()), DynWinRTArray.from_u16_values([0xD800, 0x61]).to_value(), ["\ud800", "a"], values.Char16Array),
        (PropertyType.BooleanArray, 36, at(t.bool_type()), array([DynWinRTValue.from_bool(True), DynWinRTValue.from_bool(False)], t.bool_type()), [True, False], values.BooleanArray),
        (PropertyType.StringArray, 37, at(t.hstring()), DynWinRTArray.from_string_values(["one", ""]).to_value(), ["one", ""], values.StringArray),
        (PropertyType.InspectableArray, 38, at(t.object()), lambda: array([DynWinRTValue.null_value(), created(10, t.i32_type(), DynWinRTValue.from_i32(7))], t.object()), [None, 7], values.InspectableArray),
        (PropertyType.GuidArray, 39, at(t.guid_type()), array([DynWinRTValue.from_guid(WinGUID.parse(GUID_TEXT))], t.guid_type()), [UUID(GUID_TEXT)], values.GuidArray),
        (PropertyType.DateTimeArray, 40, at(DATE_TIME), array([ticks(DATE_TIME, TICKS), ticks(DATE_TIME, 0)], DATE_TIME), [moment, EPOCH], values.DateTimeArray),
        (PropertyType.TimeSpanArray, 41, at(TIME_SPAN), array([ticks(TIME_SPAN, 10)], TIME_SPAN), [timedelta(microseconds=1)], values.TimeSpanArray),
        (PropertyType.PointArray, 42, at(POINT), array([struct(POINT, 1, 2)], POINT), [values.Point(1, 2)], values.PointArray),
        (PropertyType.SizeArray, 43, at(SIZE), array([struct(SIZE, 1, 2), struct(SIZE, 0, 0)], SIZE), [values.Size(1, 2), values.Size()], values.SizeArray),
        (PropertyType.RectArray, 44, at(RECT), array([struct(RECT, 1, 2, 3, 4)], RECT), [values.Rect(1, 2, 3, 4)], values.RectArray),
    ]


FACTORY_CASES = factory_cases()


def test_the_factory_matrix_covers_every_boxable_property_type():
    boxable = set(PropertyType) - {
        PropertyType.Empty,
        PropertyType.Inspectable,
        PropertyType.OtherType,
        PropertyType.OtherTypeArray,
    }
    assert {case[0] for case in FACTORY_CASES} == boxable
    assert len(boxable) == 37


@pytest.mark.parametrize(
    ("kind", "slot", "in_type", "argument", "plain", "preserved_type"),
    FACTORY_CASES,
    ids=[case[0].name for case in FACTORY_CASES],
)
def test_factory_boxes_round_trip_with_their_exact_property_type(
    kind, slot, in_type, argument, plain, preserved_type
):
    boxed = created(slot, in_type, argument() if callable(argument) else argument)
    assert property_type(boxed) == kind

    exact = unbox_object(boxed, preserve_type=True)
    assert type(exact) is preserved_type
    assert exact == plain

    reboxed = to_winrt_object(exact)
    assert property_type(reboxed) == kind
    assert unbox_object(reboxed, preserve_type=True) == exact
    assert type(unbox_object(reboxed, preserve_type=True)) is preserved_type
    # Re-boxing keeps the PropertyType and value, not the box's COM identity.
    assert reboxed.identity_raw() != boxed.identity_raw()


LEGACY_TYPES = {
    PropertyType.UInt8: int,
    PropertyType.Int16: int,
    PropertyType.UInt16: int,
    PropertyType.Int32: int,
    PropertyType.UInt32: int,
    PropertyType.Int64: int,
    PropertyType.UInt64: int,
    PropertyType.Single: float,
    PropertyType.Double: float,
    PropertyType.Char16: str,
    PropertyType.Boolean: bool,
    PropertyType.String: str,
    PropertyType.Guid: UUID,
    PropertyType.UInt8Array: bytes,
}


@pytest.mark.parametrize(
    ("kind", "slot", "in_type", "argument", "plain", "preserved_type"),
    FACTORY_CASES,
    ids=[case[0].name for case in FACTORY_CASES],
)
def test_default_unboxing_returns_plain_python_values(
    kind, slot, in_type, argument, plain, preserved_type
):
    unboxed = unbox_object(created(slot, in_type, argument() if callable(argument) else argument))
    assert unboxed == plain
    if kind in LEGACY_TYPES:
        # Unchanged from the previous release: exact built-in types.
        assert type(unboxed) is LEGACY_TYPES[kind]
    elif kind.name.endswith("Array"):
        assert type(unboxed) is list
        assert not any(isinstance(item, values.WinRTScalar) for item in unboxed)
    else:
        assert type(unboxed) is preserved_type
        assert not isinstance(unboxed, values.WinRTScalar)


def test_already_supported_arrays_keep_their_element_types():
    int64s = unbox_object(created(31, DynWinRTType.array_type(I8), DynWinRTArray.from_i64_values([1]).to_value()))
    chars = unbox_object(created(35, DynWinRTType.array_type(DynWinRTType.char16()), DynWinRTArray.from_u16_values([0x61]).to_value()))
    assert type(int64s) is list and type(int64s[0]) is int
    assert type(chars) is list and type(chars[0]) is str


def test_datetime_and_timespan_truncate_to_microseconds():
    assert unbox_object(created(21, DATE_TIME, ticks(DATE_TIME, 7))) == EPOCH
    assert unbox_object(created(21, DATE_TIME, ticks(DATE_TIME, -7))) == EPOCH
    assert unbox_object(created(22, TIME_SPAN, ticks(TIME_SPAN, -7))) == timedelta(0)
    moment = datetime(2024, 5, 6, 7, 8, 9, 123456, tzinfo=timezone(timedelta(hours=2)))
    unboxed = unbox_object(to_winrt_object(moment))
    assert unboxed == moment and unboxed.tzinfo is UTC


def test_datetime_outside_the_python_range_raises():
    boxed = created(21, DATE_TIME, ticks(DATE_TIME, 2**62))
    with pytest.raises(OverflowError, match="outside the range of datetime.datetime"):
        unbox_object(boxed)


def test_inspectable_arrays_unbox_their_elements_by_the_same_rules():
    raw = uri()
    nested = to_winrt_object(values.InspectableArray([values.UInt16(4), "x"]))
    boxed = to_winrt_object(values.InspectableArray([None, raw, 3, nested]))
    plain = unbox_object(boxed)
    assert type(plain) is list
    assert plain[0] is None
    assert isinstance(plain[1], DynWinRTValue) and plain[1].identity_raw() == raw.identity_raw()
    assert plain[2] == 3 and plain[3] == [4, "x"] and type(plain[3][0]) is int

    exact = unbox_object(boxed, preserve_type=True)
    assert type(exact) is values.InspectableArray
    assert type(exact[3]) is values.InspectableArray and type(exact[3][0]) is values.UInt16
    reboxed = unbox_object(to_winrt_object(exact), preserve_type=True)
    assert reboxed[1].identity_raw() == raw.identity_raw()
    assert [reboxed[0], reboxed[2], reboxed[3]] == [exact[0], exact[2], exact[3]]
    assert type(reboxed[3][0]) is values.UInt16


def test_inspectable_array_nesting_is_bounded():
    deep = values.InspectableArray([1])
    for _ in range(63):
        deep = values.InspectableArray([deep])
    boxed = to_winrt_object(deep)  # 64 levels
    read = unbox_object(boxed)
    for _ in range(64):
        read = read[0]
    assert read == 1
    deeper = to_winrt_object(values.InspectableArray([boxed]))  # 65 levels
    with pytest.raises(RecursionError, match="64 levels"):
        unbox_object(deeper)
    with pytest.raises(RecursionError, match="64 levels"):
        to_winrt_object(values.InspectableArray([deep]))
    cyclic = values.InspectableArray([1])
    cyclic.append(cyclic)
    with pytest.raises(RecursionError):
        to_winrt_object(cyclic)


# ----------------------------------------------------------------------
# Identity and unsupported boxes
# ----------------------------------------------------------------------


def test_non_boxes_and_null_keep_their_identity():
    raw = uri()
    identity = raw.identity_raw()
    assert unbox_object(raw) is raw
    assert unbox_object(raw, preserve_type=True) is raw
    assert raw.identity_raw() == identity
    assert to_winrt_object(raw) is raw

    class Wrapper:
        def __init__(self, obj):
            self._obj = obj

    assert to_winrt_object(Wrapper(raw)) is raw
    null = DynWinRTValue.null_value()
    assert unbox_object(null) is None and unbox_object(None) is None
    assert to_winrt_object(null) is null
    assert to_winrt_object(None).is_null()
    number = DynWinRTValue.from_i32(5)
    assert unbox_object(number) is number
    with pytest.raises(TypeError, match="holding a non-object I32 value"):
        to_winrt_object(number)
    with pytest.raises(TypeError, match="requires None or a DynWinRTValue"):
        unbox_object(5)


def _custom_property_value(type_value):
    """A Python-implemented IPropertyValue whose Type() reports ``type_value``."""
    t = DynWinRTType
    at = DynWinRTType.array_type
    scalar_types = [
        t.bool_type(), t.u8_type(), t.i16_type(), t.u16_type(), t.i32_type(), t.u32_type(),
        t.i64_type(), t.u64_type(), t.f32_type(), t.f64_type(), t.char16(), t.bool_type(),
        t.hstring(), t.guid_type(), DATE_TIME, TIME_SPAN, POINT, SIZE, RECT,
    ]
    array_types = [
        t.u8_type(), t.i16_type(), t.u16_type(), t.i32_type(), t.u32_type(), t.i64_type(),
        t.u64_type(), t.f32_type(), t.f64_type(), t.char16(), t.bool_type(), t.hstring(),
        t.object(), t.guid_type(), DATE_TIME, TIME_SPAN, POINT, SIZE, RECT,
    ]
    signatures = [DynWinRTMethodSig().add_out(t.i32_type())]
    signatures += [DynWinRTMethodSig().add_out(typ) for typ in scalar_types]
    signatures += [DynWinRTMethodSig().add_out(at(typ)) for typ in array_types]
    methods = [
        DynWinRTImplementationMethod(f"Method{slot}", slot, signature)
        for slot, signature in enumerate(signatures, 6)
    ]
    plan = DynWinRTInterfacePlan.create(
        "Windows.Foundation.IPropertyValue", DynWinRTType.interface(IPROPERTY_VALUE), methods
    )

    def dispatch(_interface, slot, _args):
        assert slot == 6, "only Type() may be called for a payload-less box"
        return [DynWinRTValue.from_i32(type_value)]

    return DynWinRTImplementation.create([plan], dispatch)


@pytest.mark.parametrize(
    ("type_value", "message"),
    [
        (PropertyType.Empty, "Unsupported WinRT IPropertyValue type: 0 (Empty)"),
        (PropertyType.Inspectable, "Unsupported WinRT IPropertyValue type: 13 (Inspectable)"),
        (PropertyType.OtherType, "Unsupported WinRT IPropertyValue type: 20 (OtherType)"),
        (PropertyType.OtherTypeArray, "Unsupported WinRT IPropertyValue type: 1044 (OtherTypeArray)"),
        (0x7FFF, "Unsupported WinRT IPropertyValue type: 32767"),
    ],
)
def test_payloadless_property_types_still_raise(type_value, message):
    with _custom_property_value(int(type_value)) as owner:
        raw = owner.to_value()
        try:
            for preserve_type in (False, True):
                with pytest.raises(OSError) as caught:
                    unbox_object(raw, preserve_type=preserve_type)
                assert caught.value.winerror == E_NOTIMPL
                assert str(caught.value.strerror) == message
            # The same rules apply to InspectableArray elements.
            with pytest.raises(OSError, match="Unsupported WinRT IPropertyValue type"):
                unbox_object(to_winrt_object(values.InspectableArray([1, raw])))
        finally:
            raw.release()


# ----------------------------------------------------------------------
# to_winrt_object without property_type
# ----------------------------------------------------------------------


class Color(IntEnum):
    Red = 1


class Flags(IntFlag):
    High = 0x8000_0000


class Plain(Enum):
    A = 1


class Letter(StrEnum):
    A = "a"


class GeneratedPoint:
    """Shape of a generated windows.foundation.Point struct."""

    __slots__ = ("x", "y")

    def __init__(self, x=0.0, y=0.0):
        self.x = x
        self.y = y


INFERRED = [
    # (value, PropertyType, unboxed with preserve_type=True)
    (True, PropertyType.Boolean, True),
    (5, PropertyType.Int32, 5),
    (-(2**31), PropertyType.Int32, -(2**31)),
    (1.5, PropertyType.Double, 1.5),
    ("text", PropertyType.String, "text"),
    (datetime(2024, 1, 2, 3, 4, 5, 678901, tzinfo=UTC), PropertyType.DateTime, datetime(2024, 1, 2, 3, 4, 5, 678901, tzinfo=UTC)),
    (timedelta(seconds=-3.5), PropertyType.TimeSpan, timedelta(seconds=-3.5)),
    (UUID(int=7), PropertyType.Guid, UUID(int=7)),
    (WinGUID.parse(GUID_TEXT), PropertyType.Guid, UUID(GUID_TEXT)),
    (b"\x00\xff", PropertyType.UInt8Array, b"\x00\xff"),
    (bytearray(b"ab"), PropertyType.UInt8Array, b"ab"),
    (memoryview(b"xyz"), PropertyType.UInt8Array, b"xyz"),
    (values.UInt8(255), PropertyType.UInt8, values.UInt8(255)),
    (values.Int16(-5), PropertyType.Int16, values.Int16(-5)),
    (values.UInt16(9), PropertyType.UInt16, values.UInt16(9)),
    (values.Int32(3), PropertyType.Int32, 3),
    (values.UInt32(2**32 - 1), PropertyType.UInt32, values.UInt32(2**32 - 1)),
    (values.Int64(-(2**63)), PropertyType.Int64, values.Int64(-(2**63))),
    (values.UInt64(2**64 - 1), PropertyType.UInt64, values.UInt64(2**64 - 1)),
    (values.Single(0.1), PropertyType.Single, values.Single(0.1)),
    (values.Double(2), PropertyType.Double, 2.0),
    (values.Char16("\ud800"), PropertyType.Char16, values.Char16("\ud800")),
    (values.Point(1.5, 2.5), PropertyType.Point, values.Point(1.5, 2.5)),
    (values.Size(3, 4), PropertyType.Size, values.Size(3, 4)),
    (values.Rect(1, 2, 3, 4), PropertyType.Rect, values.Rect(1, 2, 3, 4)),
    ([1, 2], PropertyType.Int32Array, [1, 2]),
    ((1, 2), PropertyType.Int32Array, [1, 2]),
    ([values.Int32(1), 2], PropertyType.Int32Array, [1, 2]),
    (["a", ""], PropertyType.StringArray, ["a", ""]),
    ([True, False], PropertyType.BooleanArray, [True, False]),
    ([1.5, values.Double(2)], PropertyType.DoubleArray, [1.5, 2.0]),
    ([values.UInt32(1), values.UInt32(2)], PropertyType.UInt32Array, [1, 2]),
    ([values.UInt8(1), values.UInt8(2)], PropertyType.UInt8Array, b"\x01\x02"),
    ([values.Char16("a")], PropertyType.Char16Array, ["a"]),
    ([EPOCH], PropertyType.DateTimeArray, [EPOCH]),
    ([timedelta(1)], PropertyType.TimeSpanArray, [timedelta(1)]),
    ([UUID(int=1), WinGUID.parse(GUID_TEXT)], PropertyType.GuidArray, [UUID(int=1), UUID(GUID_TEXT)]),
    ([values.Point(1, 2)], PropertyType.PointArray, [values.Point(1, 2)]),
    ([values.Size(1, 2)], PropertyType.SizeArray, [values.Size(1, 2)]),
    ([values.Rect(1, 2, 3, 4)], PropertyType.RectArray, [values.Rect(1, 2, 3, 4)]),
    (values.StringArray([]), PropertyType.StringArray, []),
    (values.UInt16Array([1, 2]), PropertyType.UInt16Array, [1, 2]),
    (values.InspectableArray([]), PropertyType.InspectableArray, []),
    (values.InspectableArray([None, 1, "a"]), PropertyType.InspectableArray, [None, 1, "a"]),
]


@pytest.mark.parametrize(
    ("value", "kind", "expected"),
    INFERRED,
    ids=[f"{index}-{case[1].name}" for index, case in enumerate(INFERRED)],
)
def test_unambiguous_values_box_as_their_exact_property_type(value, kind, expected):
    boxed = to_winrt_object(value)
    assert property_type(boxed) == kind
    read = unbox_object(boxed, preserve_type=True)
    assert read == expected
    assert type(read) is type(expected) or isinstance(read, values.WinRTArray)
    assert property_type(to_winrt_object(read)) == kind


@pytest.mark.parametrize(
    ("value", "error", "message"),
    [
        (2**31, OverflowError, r"does not fit in Int32.*property_type=.*dynwinrt\.values\.Int64"),
        (-(2**31) - 1, OverflowError, "does not fit in Int32"),
        (2**100, OverflowError, "does not fit in Int32"),
        ([1, 2**40], OverflowError, r"element 1: .*Int32Array.*Int64Array"),
        ([], TypeError, "empty list or tuple"),
        ((), TypeError, "empty list or tuple"),
        ([1, "a"], TypeError, r"mixing int \(element 0\) and str \(element 1\)"),
        ([1, 2.5], TypeError, "mixing int"),
        ([True, 1], TypeError, "mixing bool"),
        ([values.UInt32(1), 2], TypeError, "mixing UInt32"),
        ([1, None], TypeError, r"containing None \(element 1\)"),
        ([[1]], TypeError, "containing list"),
        ([b"x"], TypeError, "containing bytes"),
        ([Color.Red], TypeError, "containing Color"),
        (datetime(2024, 1, 1), ValueError, "timezone-aware"),
        ([datetime(2024, 1, 1)], ValueError, "DateTimeArray element 0: .*timezone-aware"),
        (Color.Red, TypeError, r"enum member <Color.Red: 1>.*IReference.*property_type"),
        (Flags.High, TypeError, "enum member"),
        (Letter.A, TypeError, "enum member"),
        (Plain.A, TypeError, "enum member"),
        (date(2024, 1, 1), TypeError, "cannot convert date to a WinRT Object"),
        (object(), TypeError, "cannot convert object"),
        ({1: 2}, TypeError, "cannot convert dict"),
        (1 + 2j, TypeError, "cannot convert complex"),
        (GeneratedPoint(1, 2), TypeError, "cannot convert GeneratedPoint.*property_type="),
        (timedelta.max, OverflowError, "out of range for WinRT TimeSpan"),
        (values.UInt32Array([1, -1]), OverflowError, r"UInt32Array element 1: -1 is out of range"),
        (values.StringArray([1]), TypeError, "StringArray element 0: WinRT String requires a str"),
        (values.InspectableArray([2**40]), OverflowError, "InspectableArray element 0: .*does not fit in Int32"),
        (values.InspectableArray([Color.Red]), TypeError, "InspectableArray element 0: .*enum member"),
        (values.WinRTArray([1]), TypeError, "not a dynwinrt.values typed array"),
        (DynWinRTValue.from_u32(5), TypeError, "holding a non-object U32 value"),
    ],
)
def test_ambiguous_or_unsupported_values_raise(value, error, message):
    with pytest.raises(error, match=message):
        to_winrt_object(value)


# ----------------------------------------------------------------------
# to_winrt_object with property_type
# ----------------------------------------------------------------------

EXPLICIT = [
    # (value, property_type, unboxed with preserve_type=True)
    (5, PropertyType.UInt8, values.UInt8(5)),
    (5, PropertyType.Int64, values.Int64(5)),
    (2**63, PropertyType.UInt64, values.UInt64(2**63)),
    (values.UInt8(7), PropertyType.Int16, values.Int16(7)),
    (5, PropertyType.Double, 5.0),
    (0.1, PropertyType.Single, values.Single(0.1)),
    ("a", PropertyType.Char16, values.Char16("a")),
    (values.Char16("b"), PropertyType.String, "b"),
    (False, PropertyType.Boolean, False),
    (UUID(int=9), PropertyType.Guid, UUID(int=9)),
    (Color.Red, PropertyType.Int32, 1),
    (Flags.High, PropertyType.UInt32, values.UInt32(0x8000_0000)),
    (GeneratedPoint(1, 2), PropertyType.Point, values.Point(1, 2)),
    (values.Point(1, 2), PropertyType.Point, values.Point(1, 2)),
    ([1, 2], PropertyType.UInt8Array, b"\x01\x02"),
    (b"\x01", PropertyType.UInt8Array, b"\x01"),
    ([1, 2], PropertyType.Int64Array, [1, 2]),
    ([], PropertyType.StringArray, []),
    ((0.5,), PropertyType.SingleArray, [0.5]),
    ([Color.Red, Flags.High], PropertyType.UInt32Array, [1, 0x8000_0000]),
    (values.Int32Array([1]), PropertyType.Int64Array, [1]),
    ([GeneratedPoint(1, 2)], PropertyType.PointArray, [values.Point(1, 2)]),
    ([1, "a", None, [True]], PropertyType.InspectableArray, [1, "a", None, [True]]),
    ([], PropertyType.InspectableArray, []),
]


@pytest.mark.parametrize(
    ("value", "kind", "expected"),
    EXPLICIT,
    ids=[f"{index}-{case[1].name}" for index, case in enumerate(EXPLICIT)],
)
def test_property_type_boxes_exactly_that_type(value, kind, expected):
    boxed = to_winrt_object(value, kind)
    assert property_type(boxed) == kind
    assert to_winrt_object(value, property_type=kind) is not boxed
    read = unbox_object(boxed, preserve_type=True)
    assert read == expected
    assert type(read) is type(expected) or isinstance(read, values.WinRTArray)


def test_property_type_accepts_equal_ints_and_other_int_enums():
    class GeneratedPropertyType(IntEnum):
        UInt16 = 3

    assert property_type(to_winrt_object(5, 5)) == PropertyType.UInt32
    assert property_type(to_winrt_object(5, GeneratedPropertyType.UInt16)) == PropertyType.UInt16
    assert property_type(to_winrt_object(5, property_type=None)) == PropertyType.Int32


@pytest.mark.parametrize(
    ("value", "kind", "error", "message"),
    [
        (300, PropertyType.UInt8, OverflowError, r"300 is out of range for WinRT UInt8 \(0..255\)"),
        (-1, PropertyType.UInt64, OverflowError, "out of range for WinRT UInt64"),
        (2**64, PropertyType.UInt64, OverflowError, "out of range for WinRT UInt64"),
        (True, PropertyType.Int32, TypeError, "WinRT Int32 requires an int, not bool"),
        (1.5, PropertyType.Int32, TypeError, "WinRT Int32 requires an int, not float"),
        ("1", PropertyType.Int32, TypeError, "requires an int"),
        (Flags.High, PropertyType.Int32, OverflowError, "out of range for WinRT Int32"),
        (Plain.A, PropertyType.Int32, TypeError, "int-valued enum member"),
        (Color.Red, PropertyType.Double, TypeError, "enum members box only with an integer"),
        (Letter.A, PropertyType.String, TypeError, "enum members box only with an integer"),
        (True, PropertyType.Double, TypeError, "requires a real number"),
        (1e39, PropertyType.Single, OverflowError, r"1e\+39 is out of range for WinRT Single"),
        ("ab", PropertyType.Char16, ValueError, "exactly one UTF-16 code unit"),
        ("\U0001f600", PropertyType.Char16, ValueError, "exactly one UTF-16 code unit"),
        (97, PropertyType.Char16, TypeError, "requires a str"),
        (1, PropertyType.Boolean, TypeError, "requires a bool"),
        (5, PropertyType.String, TypeError, "requires a str"),
        (GUID_TEXT, PropertyType.Guid, TypeError, "requires a uuid.UUID or WinGUID"),
        (datetime(2024, 1, 1), PropertyType.DateTime, ValueError, "timezone-aware"),
        (5, PropertyType.TimeSpan, TypeError, "requires a datetime.timedelta"),
        (values.Rect(), PropertyType.Point, TypeError, "not dynwinrt.values.Rect"),
        (GeneratedPoint(), PropertyType.Size, TypeError, "numeric width/height attributes"),
        (GeneratedPoint("x", 1), PropertyType.Point, TypeError, "WinRT Point.x requires a real number"),
        (b"ab", PropertyType.Int32Array, TypeError, "Int32Array requires a list or tuple"),
        ("abc", PropertyType.Char16Array, TypeError, "Char16Array requires a list or tuple"),
        ({1}, PropertyType.UInt8Array, TypeError, "list or tuple, bytes, bytearray or memoryview"),
        ([1, -1], PropertyType.UInt32Array, OverflowError, "UInt32Array element 1: -1 is out of range"),
        ([1, None], PropertyType.Int32Array, TypeError, "Int32Array element 1: WinRT Int32 requires an int"),
        ([[1, "a"]], PropertyType.InspectableArray, TypeError, "InspectableArray element 0: .*mixing"),
        (None, PropertyType.Int32, TypeError, "pass None without property_type"),
        (DynWinRTValue.null_value(), PropertyType.Int32, TypeError, "already a WinRT object"),
        (1, PropertyType.Empty, ValueError, "PropertyType.Empty is WinRT null"),
        (1, PropertyType.Inspectable, ValueError, "PropertyType.Inspectable is not a boxed value"),
        (1, PropertyType.OtherType, ValueError, "OtherType has no language-neutral payload"),
        (1, PropertyType.OtherTypeArray, ValueError, "OtherTypeArray has no language-neutral payload"),
        (1, 99, ValueError, "99 is not a Windows.Foundation.PropertyType value"),
        (1, 2**40, ValueError, "is not a Windows.Foundation.PropertyType value"),
        (1, True, TypeError, "property_type must be a dynwinrt.values.PropertyType member or an int"),
        (1, "UInt32", TypeError, "property_type must be"),
    ],
)
def test_property_type_validates_range_and_convertibility(value, kind, error, message):
    with pytest.raises(error, match=message):
        to_winrt_object(value, kind)


def test_existing_objects_cannot_be_combined_with_property_type():
    raw = to_winrt_object(5)
    with pytest.raises(TypeError, match="already a WinRT object"):
        to_winrt_object(raw, PropertyType.Int32)


# ----------------------------------------------------------------------
# The dynwinrt.values data model
# ----------------------------------------------------------------------


def test_the_data_model_lives_in_its_own_namespace():
    assert dynwinrt.values is values
    for name in values.__all__:
        assert name not in dynwinrt.__all__
        if name != "WinRTObjectValue":  # a typing.Union alias
            assert getattr(values, name).__module__ == "dynwinrt.values"
    assert "to_winrt_object" in dynwinrt.__all__ and "unbox_object" in dynwinrt.__all__
    namespace = {}
    exec("from dynwinrt import *", namespace)
    assert not {"Point", "Size", "Rect", "PropertyType", "UInt32", "values"} & set(namespace)


def test_property_type_mirrors_windows_foundation_property_type():
    assert [(member.name, member.value) for member in PropertyType][:21] == [
        ("Empty", 0), ("UInt8", 1), ("Int16", 2), ("UInt16", 3), ("Int32", 4),
        ("UInt32", 5), ("Int64", 6), ("UInt64", 7), ("Single", 8), ("Double", 9),
        ("Char16", 10), ("Boolean", 11), ("String", 12), ("Inspectable", 13),
        ("DateTime", 14), ("TimeSpan", 15), ("Guid", 16), ("Point", 17), ("Size", 18),
        ("Rect", 19), ("OtherType", 20),
    ]
    for member in list(PropertyType)[1:21]:
        assert PropertyType[f"{member.name}Array"] == member.value + 1024


@pytest.mark.parametrize(
    ("tag", "low", "high"),
    [
        (values.UInt8, 0, 2**8 - 1),
        (values.Int16, -(2**15), 2**15 - 1),
        (values.UInt16, 0, 2**16 - 1),
        (values.Int32, -(2**31), 2**31 - 1),
        (values.UInt32, 0, 2**32 - 1),
        (values.Int64, -(2**63), 2**63 - 1),
        (values.UInt64, 0, 2**64 - 1),
    ],
)
def test_integer_tags_validate_their_range_and_behave_like_int(tag, low, high):
    assert tag(low) == low and tag(high) == high
    for invalid in (low - 1, high + 1):
        with pytest.raises(OverflowError, match=tag.__name__):
            tag(invalid)
    with pytest.raises(TypeError, match=f"WinRT {tag.__name__} requires an int"):
        tag(1.5)
    value = tag(5)
    assert isinstance(value, int) and isinstance(value, values.WinRTScalar)
    assert value.property_type == PropertyType[tag.__name__]
    assert value == 5 and hash(value) == hash(5) and {value: "x"}[5] == "x"
    assert repr(value) == f"dynwinrt.values.{tag.__name__}(5)"
    assert str(value) == "5" and f"{value}" == "5" and f"{value:03}" == "005"
    assert json.dumps({"n": value}) == '{"n": 5}'
    assert type(value + 1) is int and value + 1 == 6
    assert type(-value) is int and type(value * 2) is int
    for protocol in range(pickle.HIGHEST_PROTOCOL + 1):
        restored = pickle.loads(pickle.dumps(value, protocol))
        assert type(restored) is tag and restored == 5
    assert type(copy.deepcopy(value)) is tag
    assert tag(Color.Red) == 1


def test_float_and_char16_tags():
    single = values.Single(0.1)
    assert single != 0.1 and single == 0.10000000149011612
    assert repr(single) == "dynwinrt.values.Single(0.10000000149011612)"
    assert str(single) == "0.10000000149011612"
    assert json.dumps(single) == "0.10000000149011612"
    assert type(single * 2) is float
    assert math.isinf(values.Single(float("inf"))) and math.isnan(values.Single(float("nan")))
    with pytest.raises(OverflowError, match="Single"):
        values.Single(1e39)
    with pytest.raises(TypeError, match="real number"):
        values.Single("1.5")
    double = values.Double(3)
    assert double == 3.0 and type(double + 1) is float
    assert repr(double) == "dynwinrt.values.Double(3.0)"
    assert type(pickle.loads(pickle.dumps(single))) is values.Single

    char = values.Char16("\ud800")
    assert isinstance(char, str) and char == "\ud800" and len(char) == 1
    assert repr(char) == "dynwinrt.values.Char16('\\ud800')"
    assert type(char + "x") is str and str(char) == "\ud800"
    with pytest.raises(ValueError):
        values.Char16("ab")
    with pytest.raises(ValueError):
        values.Char16("\U0001f600")
    with pytest.raises(TypeError):
        values.Char16(97)
    assert type(pickle.loads(pickle.dumps(values.Char16("a")))) is values.Char16


def test_typed_arrays_are_lists_that_remember_their_type():
    items = values.UInt32Array([1, 2])
    assert isinstance(items, list) and isinstance(items, values.WinRTArray)
    assert items == [1, 2] and items.property_type == PropertyType.UInt32Array
    assert repr(items) == "dynwinrt.values.UInt32Array([1, 2])"
    assert type(items.copy()) is values.UInt32Array
    assert type(copy.copy(items)) is values.UInt32Array
    assert type(pickle.loads(pickle.dumps(items))) is values.UInt32Array
    assert json.dumps(items) == "[1, 2]"
    assert type(items[:1]) is list and type(items + [3]) is list
    assert not hasattr(values, "UInt8Array")  # bytes is the UInt8Array form


def test_geometry_values_are_immutable_float32_values():
    point = values.Point(0.1, 2)
    assert point.x == values.Single(0.1) and point.y == 2.0
    assert repr(point) == "dynwinrt.values.Point(x=0.10000000149011612, y=2.0)"
    with pytest.raises(AttributeError, match="immutable"):
        point.x = 3
    assert point == values.Point(x=0.1, y=2.0)
    assert hash(point) == hash(values.Point(0.1, 2.0))
    assert point != values.Size(0.1, 2.0) and point != (0.1, 2.0)
    assert point != GeneratedPoint(0.1, 2.0)
    assert point.property_type == PropertyType.Point
    assert pickle.loads(pickle.dumps(point)) == point and copy.copy(point) == point
    match values.Rect(1, 2, 3, 4):
        case values.Rect(x, y, width, height):
            assert (x, y, width, height) == (1.0, 2.0, 3.0, 4.0)
    assert values.Size(width=3).width == 3.0 and values.Size().height == 0.0
    with pytest.raises(TypeError):
        values.Point(1, 2, 3)
    with pytest.raises(OverflowError):
        values.Point(1e39, 0)
    with pytest.raises(TypeError, match="real number"):
        values.Point("1", 0)
