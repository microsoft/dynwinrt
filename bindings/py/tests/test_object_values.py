# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""The WinRT ``Object`` value model and its single conversion layer.

``to_winrt_object`` / ``from_winrt_object`` are the only conversions generated
code uses for ``Object`` (IInspectable) positions. These tests pin their
contract without generated bindings:

- R1: a boxed value read and written back keeps its PropertyType and value.
- R2: a supported Python value written and read back compares equal.
- I1: objects that are not boxed values pass through as the same object.
- I2: boxed values have value semantics (every write creates a new box).
"""

import copy
import json
import math
import pickle
from datetime import datetime, timedelta, timezone
from enum import IntEnum, IntFlag
from uuid import UUID

import pytest

import dynwinrt
from dynwinrt import (
    DynWinRTStruct,
    DynWinRTType,
    DynWinRTValue,
    WinGUID,
    from_winrt_object,
    ro_initialize,
    to_winrt_object,
    unbox_object,
)

ro_initialize(1)

IID_IPROPERTY_VALUE = WinGUID.parse("4bd682dd-7554-40e9-9a9b-82654ede7e62")
IREFERENCE_PIID = WinGUID.parse("61c17706-2d65-11e0-9ae8-d48564015472")
PROPERTY_TYPES = {
    "UInt8": 1, "Int16": 2, "UInt16": 3, "Int32": 4, "UInt32": 5, "Int64": 6,
    "UInt64": 7, "Single": 8, "Double": 9, "Char16": 10, "Boolean": 11, "String": 12,
    "DateTime": 14, "TimeSpan": 15, "Guid": 16, "Point": 17, "Size": 18, "Rect": 19,
    "UInt8Array": 1025, "Int16Array": 1026, "UInt16Array": 1027, "Int32Array": 1028,
    "UInt32Array": 1029, "Int64Array": 1030, "UInt64Array": 1031, "SingleArray": 1032,
    "DoubleArray": 1033, "Char16Array": 1034, "BooleanArray": 1035, "StringArray": 1036,
    "InspectableArray": 1037, "DateTimeArray": 1038, "TimeSpanArray": 1039,
    "GuidArray": 1040, "PointArray": 1041, "SizeArray": 1042, "RectArray": 1043,
}
UTC = timezone.utc


def property_type(raw):
    """Read IPropertyValue.Type through a raw vtable call (no registrations)."""
    assert isinstance(raw, DynWinRTValue)
    return raw.cast(IID_IPROPERTY_VALUE).call_0(6, DynWinRTType.i32_type()).to_number()


def type_name(raw):
    number = property_type(raw)
    return next(name for name, value in PROPERTY_TYPES.items() if value == number)


def uri():
    factory = DynWinRTValue.activation_factory("Windows.Foundation.Uri")
    # IUriRuntimeClassFactory.CreateUri is vtable slot 6.
    iid = WinGUID.parse("44a9796f-723e-4fdf-a218-033e75b0c084")
    signature = (
        dynwinrt.DynWinRTMethodSig()
        .add_in(DynWinRTType.hstring())
        .add_out(DynWinRTType.object())
    )
    handle = DynWinRTType.register_interface("IUriRuntimeClassFactory_ObjectValues", iid)
    handle = handle.add_method("CreateUri", signature)
    return handle.method(6).invoke(factory.cast(iid), [DynWinRTValue.from_hstring("https://example.com/")])


class Mode(IntEnum):
    Off = 0
    On = 1


Mode._dynwinrt_enum_type = ("Contoso.ObjectValues.Mode", "Int32")


class Options(IntFlag):
    Low = 1
    High = 0x8000_0000


Options._dynwinrt_enum_type = ("Contoso.ObjectValues.Options", "UInt32")


class UnmarkedMode(IntEnum):
    A = 7


class GeneratedPoint:
    """Shape of the generated windows.foundation.Point struct."""

    __slots__ = ("x", "y")
    _dynwinrt_property_type = "Point"

    def __init__(self, x=0.0, y=0.0):
        self.x = x
        self.y = y

    def __eq__(self, other):
        if type(other) is not type(self):
            return NotImplemented
        return (self.x, self.y) == (other.x, other.y)


class Wrapper:
    """Shape of a generated projected wrapper."""

    def __init__(self, obj):
        self._obj = obj


# ----------------------------------------------------------------------
# The tagged value model
# ----------------------------------------------------------------------


@pytest.mark.parametrize(
    ("tag", "low", "high"),
    [
        (dynwinrt.UInt8, 0, 2**8 - 1),
        (dynwinrt.Int16, -(2**15), 2**15 - 1),
        (dynwinrt.UInt16, 0, 2**16 - 1),
        (dynwinrt.Int32, -(2**31), 2**31 - 1),
        (dynwinrt.UInt32, 0, 2**32 - 1),
        (dynwinrt.Int64, -(2**63), 2**63 - 1),
        (dynwinrt.UInt64, 0, 2**64 - 1),
    ],
)
def test_integer_tags_validate_their_range_and_behave_like_int(tag, low, high):
    assert tag(low) == low and tag(high) == high
    for invalid in (low - 1, high + 1):
        with pytest.raises(OverflowError, match=tag.__name__):
            tag(invalid)
    with pytest.raises(TypeError):
        tag(1.5)
    value = tag(5)
    assert isinstance(value, int) and isinstance(value, dynwinrt.WinRTScalar)
    assert value == 5 and hash(value) == hash(5) and {value: "x"}[5] == "x"
    assert repr(value) == f"dynwinrt.{tag.__name__}(5)"
    assert str(value) == "5" and f"{value}" == "5" and f"{value:03}" == "005"
    assert json.dumps({"n": value}) == '{"n": 5}'
    assert type(value + 1) is int and value + 1 == 6
    assert type(-value) is int and type(value * 2) is int
    assert value.property_type == tag.__name__
    assert type(tag.__module__) is str and tag.__module__ == "dynwinrt"
    assert pickle.loads(pickle.dumps(value)) == 5
    assert type(pickle.loads(pickle.dumps(value))) is tag
    assert type(copy.deepcopy(value)) is tag


def test_float_and_char16_tags():
    single = dynwinrt.Single(0.1)
    assert single != 0.1 and single == 0.10000000149011612
    assert repr(single) == "dynwinrt.Single(0.10000000149011612)"
    assert json.dumps(single) == "0.10000000149011612"
    assert type(single * 2) is float
    assert math.isinf(dynwinrt.Single(float("inf")))
    assert math.isnan(dynwinrt.Single(float("nan")))
    with pytest.raises(OverflowError, match="Single"):
        dynwinrt.Single(1e39)
    with pytest.raises(TypeError):
        dynwinrt.Single("1.5")
    double = dynwinrt.Double(3)
    assert double == 3.0 and type(double + 1) is float
    assert repr(double) == "dynwinrt.Double(3.0)"

    char = dynwinrt.Char16("\ud800")
    assert isinstance(char, str) and char == "\ud800" and len(char) == 1
    assert repr(char) == "dynwinrt.Char16('\\ud800')"
    assert type(char + "x") is str
    with pytest.raises(ValueError):
        dynwinrt.Char16("ab")
    with pytest.raises(ValueError):
        dynwinrt.Char16("\U0001f600")
    with pytest.raises(TypeError):
        dynwinrt.Char16(97)
    assert type(pickle.loads(pickle.dumps(char))) is dynwinrt.Char16


def test_typed_arrays_are_lists_that_remember_their_type():
    values = dynwinrt.UInt32Array([1, 2])
    assert isinstance(values, list) and isinstance(values, dynwinrt.WinRTArray)
    assert values == [1, 2] and values.property_type == "UInt32Array"
    assert repr(values) == "dynwinrt.UInt32Array([1, 2])"
    assert type(values.copy()) is dynwinrt.UInt32Array
    assert type(copy.copy(values)) is dynwinrt.UInt32Array
    assert type(pickle.loads(pickle.dumps(values))) is dynwinrt.UInt32Array
    assert json.dumps(values) == "[1, 2]"
    assert type(values[:1]) is list


def test_geometry_values_are_immutable_float32_values():
    point = dynwinrt.Point(0.1, 2)
    assert point.x == dynwinrt.Single(0.1) and point.y == 2.0
    assert repr(point) == "dynwinrt.Point(x=0.10000000149011612, y=2.0)"
    with pytest.raises(AttributeError):
        point.x = 3
    assert point == dynwinrt.Point(x=0.1, y=2.0)
    assert hash(point) == hash(dynwinrt.Point(0.1, 2.0))
    assert point != dynwinrt.Size(0.1, 2.0)
    assert point != (0.1, 2.0)
    # Equal to the generated struct when both box to the same float32 fields.
    generated = GeneratedPoint(0.1, 2.0)
    assert point == generated and generated == point
    assert point != GeneratedPoint(0.2, 2.0)
    assert pickle.loads(pickle.dumps(point)) == point
    match dynwinrt.Rect(1, 2, 3, 4):
        case dynwinrt.Rect(x, y, width, height):
            assert (x, y, width, height) == (1.0, 2.0, 3.0, 4.0)
    assert dynwinrt.Size(width=3).width == 3.0 and dynwinrt.Size().height == 0.0
    with pytest.raises(TypeError):
        dynwinrt.Point(1, 2, 3)
    with pytest.raises(OverflowError):
        dynwinrt.Point(1e39, 0)


# ----------------------------------------------------------------------
# R1/R2: round trips for every PropertyType
# ----------------------------------------------------------------------

EPOCH = datetime(1601, 1, 1, tzinfo=UTC)
WRITE_READ = [
    # (written Python value, expected PropertyType, expected read value, exact read type)
    (True, "Boolean", True, bool),
    (5, "Int32", 5, int),
    (2**40, "Int64", 2**40, dynwinrt.Int64),
    (2**63, "UInt64", 2**63, dynwinrt.UInt64),
    (1.5, "Double", 1.5, float),
    ("text", "String", "text", str),
    (UUID(int=42), "Guid", UUID(int=42), UUID),
    (datetime(2024, 5, 6, 7, 8, 9, 123456, tzinfo=UTC), "DateTime",
     datetime(2024, 5, 6, 7, 8, 9, 123456, tzinfo=UTC), datetime),
    (datetime(2024, 5, 6, 9, tzinfo=timezone(timedelta(hours=2))), "DateTime",
     datetime(2024, 5, 6, 7, tzinfo=UTC), datetime),
    (timedelta(days=-1, microseconds=5), "TimeSpan", timedelta(days=-1, microseconds=5), timedelta),
    (b"\x00\xff", "UInt8Array", b"\x00\xff", bytes),
    (bytearray(b"ab"), "UInt8Array", b"ab", bytes),
    (memoryview(b"xyz"), "UInt8Array", b"xyz", bytes),
    (dynwinrt.UInt8(255), "UInt8", 255, dynwinrt.UInt8),
    (dynwinrt.Int16(-5), "Int16", -5, dynwinrt.Int16),
    (dynwinrt.UInt16(65535), "UInt16", 65535, dynwinrt.UInt16),
    (dynwinrt.Int32(7), "Int32", 7, int),
    (dynwinrt.UInt32(2**32 - 1), "UInt32", 2**32 - 1, dynwinrt.UInt32),
    (dynwinrt.Int64(-(2**63)), "Int64", -(2**63), dynwinrt.Int64),
    (dynwinrt.UInt64(2**64 - 1), "UInt64", 2**64 - 1, dynwinrt.UInt64),
    (dynwinrt.Single(0.5), "Single", 0.5, dynwinrt.Single),
    (dynwinrt.Double(3), "Double", 3.0, float),
    (dynwinrt.Char16("\ud800"), "Char16", "\ud800", dynwinrt.Char16),
    (dynwinrt.Point(1.5, 2.5), "Point", dynwinrt.Point(1.5, 2.5), dynwinrt.Point),
    (GeneratedPoint(1.5, 2.5), "Point", dynwinrt.Point(1.5, 2.5), dynwinrt.Point),
    (dynwinrt.Size(3, 4), "Size", dynwinrt.Size(3, 4), dynwinrt.Size),
    (dynwinrt.Rect(1, 2, 3, 4), "Rect", dynwinrt.Rect(1, 2, 3, 4), dynwinrt.Rect),
    ([1, 2], "Int32Array", [1, 2], dynwinrt.Int32Array),
    ((1, 2), "Int32Array", [1, 2], dynwinrt.Int32Array),
    ([1, 2**40], "Int64Array", [1, 2**40], dynwinrt.Int64Array),
    ([1, 2**63], "UInt64Array", [1, 2**63], dynwinrt.UInt64Array),
    ([1, 2.5], "DoubleArray", [1.0, 2.5], dynwinrt.DoubleArray),
    (["a", ""], "StringArray", ["a", ""], dynwinrt.StringArray),
    ([True, False], "BooleanArray", [True, False], dynwinrt.BooleanArray),
    ([UUID(int=1)], "GuidArray", [UUID(int=1)], dynwinrt.GuidArray),
    ([EPOCH], "DateTimeArray", [EPOCH], dynwinrt.DateTimeArray),
    ([timedelta(1)], "TimeSpanArray", [timedelta(1)], dynwinrt.TimeSpanArray),
    ([dynwinrt.Point(1, 2)], "PointArray", [dynwinrt.Point(1, 2)], dynwinrt.PointArray),
    ([dynwinrt.Size(1, 2)], "SizeArray", [dynwinrt.Size(1, 2)], dynwinrt.SizeArray),
    ([dynwinrt.Rect(1, 2, 3, 4)], "RectArray", [dynwinrt.Rect(1, 2, 3, 4)], dynwinrt.RectArray),
    ([dynwinrt.Char16("a")], "Char16Array", ["a"], dynwinrt.Char16Array),
    ([dynwinrt.UInt8(1), dynwinrt.UInt8(2)], "UInt8Array", b"\x01\x02", bytes),
    ([dynwinrt.UInt32(1), 2], "UInt32Array", [1, 2], dynwinrt.UInt32Array),
    ([dynwinrt.Single(1), 2, 3.5], "SingleArray", [1.0, 2.0, 3.5], dynwinrt.SingleArray),
    ([1, "a", None], "InspectableArray", [1, "a", None], dynwinrt.InspectableArray),
    ([True, 1], "InspectableArray", [True, 1], dynwinrt.InspectableArray),
    ([[1, 2], ["x"]], "InspectableArray", [[1, 2], ["x"]], dynwinrt.InspectableArray),
    (dynwinrt.StringArray([]), "StringArray", [], dynwinrt.StringArray),
    (dynwinrt.InspectableArray([]), "InspectableArray", [], dynwinrt.InspectableArray),
    (dynwinrt.InspectableArray([1, 2]), "InspectableArray", [1, 2], dynwinrt.InspectableArray),
    (dynwinrt.UInt16Array([1, 2]), "UInt16Array", [1, 2], dynwinrt.UInt16Array),
]


@pytest.mark.parametrize(
    ("written", "expected_type", "expected", "read_type"),
    WRITE_READ,
    ids=[f"{index}-{case[1]}" for index, case in enumerate(WRITE_READ)],
)
def test_written_values_read_back_equal_with_their_exact_type(
    written, expected_type, expected, read_type
):
    boxed = to_winrt_object(written)
    assert type_name(boxed) == expected_type
    read = from_winrt_object(boxed)
    assert type(read) is read_type, repr(read)
    assert read == expected
    # R1: the value read writes back as the same PropertyType and value.
    rewritten = to_winrt_object(read)
    assert rewritten is not boxed
    assert type_name(rewritten) == expected_type
    assert from_winrt_object(rewritten) == expected


def test_array_elements_keep_their_tags_through_list_operations():
    read = from_winrt_object(to_winrt_object(dynwinrt.UInt32Array([1, 2, 3])))
    assert read == [1, 2, 3]
    assert all(type(value) is dynwinrt.UInt32 for value in read)
    # Slicing and concatenation drop the list subclass, not the element tags.
    for derived in (read[:2], read + [4], [value for value in read]):
        assert type(derived) is list
        assert type_name(to_winrt_object(derived)) == "UInt32Array"
    nested = from_winrt_object(to_winrt_object([dynwinrt.Int16(1), "x", [dynwinrt.UInt8(2)]]))
    assert type(nested) is dynwinrt.InspectableArray
    assert type(nested[0]) is dynwinrt.Int16 and nested[2] == b"\x02"


def test_datetime_and_timespan_keep_microseconds_and_truncate_ticks():
    value = datetime(1999, 12, 31, 23, 59, 59, 999999, tzinfo=UTC)
    assert from_winrt_object(to_winrt_object(value)) == value
    date_type = DynWinRTType.struct_type("Windows.Foundation.DateTime", [DynWinRTType.i64_type()])
    ticks = DynWinRTStruct.create(date_type)
    ticks.set_i64(0, 7)  # 700 ns after the WinRT epoch
    assert from_winrt_object(to_winrt_object(ticks.to_value())) == EPOCH


# ----------------------------------------------------------------------
# Write rules and their pinned order
# ----------------------------------------------------------------------


def test_write_rule_order_for_int_subclasses_and_markers():
    # bool before int, tags before int, enum markers before int.
    assert type_name(to_winrt_object(True)) == "Boolean"
    assert type_name(to_winrt_object(dynwinrt.UInt32(1))) == "UInt32"
    assert type_name(to_winrt_object(UnmarkedMode.A)) == "Int32"

    enum_type = DynWinRTType.enum_type("Contoso.ObjectValues.Mode", ["Off", "On"], [0, 1])
    boxed = to_winrt_object(Mode.On)
    reference_iid = DynWinRTType.parameterized(IREFERENCE_PIID, [enum_type]).iid()
    assert boxed.cast(reference_iid).call_0(6, enum_type).get_enum_int() == 1
    # IReference<Enum> is not an IPropertyValue: it reads back as the same object
    # and writes back unchanged (I1), like any other WinRT object.
    assert from_winrt_object(boxed) is boxed
    assert to_winrt_object(boxed) is boxed

    flags = DynWinRTType.enum_type(
        "Contoso.ObjectValues.Options", ["Low", "High"], [1, 0x8000_0000], DynWinRTType.u32_type()
    )
    flag_iid = DynWinRTType.parameterized(IREFERENCE_PIID, [flags]).iid()
    boxed = to_winrt_object(Options.High | Options.Low)
    assert boxed.cast(flag_iid).call_0(6, flags).get_enum_int() == 0x8000_0001

    # Geometry markers: dynwinrt values and generated structs box as Point.
    assert type_name(to_winrt_object(GeneratedPoint(1, 2))) == "Point"
    # Typed arrays before list/tuple inference.
    assert type_name(to_winrt_object(dynwinrt.DoubleArray([1, 2]))) == "DoubleArray"


def test_native_values_and_wrappers_pass_through_by_identity():
    raw = uri()
    assert to_winrt_object(raw) is raw
    assert to_winrt_object(Wrapper(raw)) is raw
    assert from_winrt_object(raw) is raw
    null = DynWinRTValue.null_value()
    assert to_winrt_object(null) is null
    assert from_winrt_object(null) is None
    assert to_winrt_object(None).is_null()
    # Native values holding a non-object payload box by their exact kind.
    assert type_name(to_winrt_object(DynWinRTValue.from_u32(5))) == "UInt32"
    assert type_name(to_winrt_object(DynWinRTValue.from_hstring("s"))) == "String"
    point_type = DynWinRTType.struct_type(
        "Windows.Foundation.Point", [DynWinRTType.f32_type(), DynWinRTType.f32_type()]
    )
    point = DynWinRTStruct.create(point_type)
    point.set_f32(0, 1.5)
    assert from_winrt_object(to_winrt_object(point.to_value())) == dynwinrt.Point(1.5, 0)
    with pytest.raises(TypeError, match="no IPropertyValue representation"):
        to_winrt_object(DynWinRTValue.from_i8(1))


def test_unsupported_inputs_raise_helpful_errors():
    for value in (object(), {1: 2}, {1, 2}, range(3), 1 + 2j):
        with pytest.raises(TypeError, match="expected None, a DynWinRTValue"):
            to_winrt_object(value)
    with pytest.raises(TypeError, match="empty list or tuple"):
        to_winrt_object([])
    with pytest.raises(TypeError, match="empty list or tuple"):
        to_winrt_object(())
    with pytest.raises(ValueError, match="timezone-aware"):
        to_winrt_object(datetime(2024, 1, 1))
    with pytest.raises(OverflowError, match="Int32, Int64 or UInt64"):
        to_winrt_object(2**64)
    with pytest.raises(OverflowError):
        to_winrt_object([-1, 2**64 - 1])
    with pytest.raises(OverflowError, match="UInt32Array element 1"):
        to_winrt_object(dynwinrt.UInt32Array([1, -1]))
    with pytest.raises(TypeError, match="StringArray element 0"):
        to_winrt_object(dynwinrt.StringArray([1]))
    with pytest.raises(TypeError, match="abstract"):
        to_winrt_object(dynwinrt.WinRTArray([1]))
    with pytest.raises(TypeError, match="expected None"):
        to_winrt_object([1, object()])


# ----------------------------------------------------------------------
# Read rules, identity and idempotency
# ----------------------------------------------------------------------


def test_from_winrt_object_is_idempotent_and_unbox_object_is_the_same_function():
    assert unbox_object is from_winrt_object
    for plain in (None, 5, "x", dynwinrt.UInt32(5), [1, 2], dynwinrt.Point(1, 2), object()):
        assert from_winrt_object(plain) is plain
    once = from_winrt_object(to_winrt_object(dynwinrt.UInt64(9)))
    assert from_winrt_object(once) is once


def test_unrepresentable_values_stay_native_instead_of_raising():
    date_type = DynWinRTType.struct_type("Windows.Foundation.DateTime", [DynWinRTType.i64_type()])
    ticks = DynWinRTStruct.create(date_type)
    ticks.set_i64(0, 2**62)  # far beyond Python's datetime.max
    boxed = to_winrt_object(ticks.to_value())
    assert type_name(boxed) == "DateTime"
    assert from_winrt_object(boxed) is boxed
    array = to_winrt_object([ticks.to_value()])
    assert type_name(array) == "InspectableArray"
    unboxed = from_winrt_object(array)
    assert type(unboxed) is dynwinrt.InspectableArray
    assert isinstance(unboxed[0], DynWinRTValue)


def test_inspectable_arrays_convert_elements_and_keep_object_identity():
    raw = uri()
    unboxed = from_winrt_object(to_winrt_object([None, raw, 3, [dynwinrt.UInt16(4)]]))
    assert unboxed[0] is None
    assert unboxed[1].identity_raw() == raw.identity_raw()
    assert unboxed[2] == 3 and unboxed[3] == [4] and type(unboxed[3][0]) is dynwinrt.UInt16
    rewritten = from_winrt_object(to_winrt_object(unboxed))
    assert rewritten[1].identity_raw() == raw.identity_raw()


def test_nesting_is_bounded_in_both_directions():
    cyclic = [1, "x"]
    cyclic.append(cyclic)
    with pytest.raises(RecursionError, match="64 levels"):
        to_winrt_object(cyclic)
    deep = 1
    for _ in range(64):
        deep = [deep, "x"]
    boxed = to_winrt_object(deep)
    read = from_winrt_object(boxed)
    for _ in range(64):
        assert type(read) is dynwinrt.InspectableArray
        read = read[0]
    assert read == 1
    with pytest.raises(RecursionError):
        to_winrt_object([deep, "x"])
    # A box nested one level deeper (built from an existing box) still reads;
    # values beyond the limit stay native instead of raising.
    deeper = from_winrt_object(to_winrt_object([boxed, "x"]))
    for _ in range(65):
        deeper = deeper[0]
    assert isinstance(deeper, DynWinRTValue)
    assert from_winrt_object(deeper) == 1


def test_boxes_have_value_semantics():
    first = to_winrt_object(42)
    second = to_winrt_object(from_winrt_object(first))
    assert first.identity_raw() != second.identity_raw()
    assert from_winrt_object(first) == from_winrt_object(second) == 42


def test_aliases_describe_the_runtime_model():
    for value in (True, 1, 1.5, "s", UUID(int=1), EPOCH, timedelta(1), b"",
                  dynwinrt.Point(), dynwinrt.Size(), dynwinrt.Rect(), [1], uri()):
        assert isinstance(value, dynwinrt.WinRTObjectValue)
    for value in ((1,), bytearray(), memoryview(b"")):
        assert isinstance(value, dynwinrt.WinRTObjectInput)
        assert not isinstance(value, dynwinrt.WinRTObjectValue)
    exported = set(dynwinrt.__all__)
    assert {"to_winrt_object", "from_winrt_object", "unbox_object", "UInt32",
            "InspectableArray", "Point", "WinRTObjectValue", "WinRTObjectInput"} <= exported
