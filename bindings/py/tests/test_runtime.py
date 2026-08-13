# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import dynwinrt
import pytest
from dynwinrt import DynWinRTType, DynWinRTValue, WinGUID, ro_initialize


def test_to_number():
    """to_number should work for all integer-like types."""
    assert DynWinRTValue.from_i32(42).to_number() == 42
    assert DynWinRTValue.from_bool(True).to_number() == 1
    assert DynWinRTValue.from_i8(-5).to_number() == -5
    assert DynWinRTValue.from_u8(200).to_number() == 200
    assert DynWinRTValue.from_i16(-1000).to_number() == -1000
    assert DynWinRTValue.from_u16(5000).to_number() == 5000
    assert DynWinRTValue.from_u32(0xFFFFFFFF).to_number() == 0xFFFFFFFF


def test_to_i64():
    assert DynWinRTValue.from_i64(9999999999).to_i64() == 9999999999
    assert DynWinRTValue.from_u64(12345).to_i64() == 12345
    assert DynWinRTValue.from_i32(42).to_i64() == 42


def test_unsigned_conversions_preserve_full_range():
    assert DynWinRTValue.from_u32(0xFFFFFFFF).to_u32() == 0xFFFFFFFF
    assert DynWinRTValue.from_u64(0xFFFFFFFFFFFFFFFF).to_u64() == 0xFFFFFFFFFFFFFFFF
    assert DynWinRTValue.from_u64(0xFFFFFFFFFFFFFFFF).to_int() == 0xFFFFFFFFFFFFFFFF
    with pytest.raises(RuntimeError, match="does not fit"):
        DynWinRTValue.from_u64(0xFFFFFFFFFFFFFFFF).to_i64()


def test_to_f64():
    assert DynWinRTValue.from_f64(3.14).to_f64() == 3.14
    assert abs(DynWinRTValue.from_f32(1.5).to_f64() - 1.5) < 0.01
    assert DynWinRTValue.from_i32(10).to_f64() == 10.0


def test_to_bool():
    assert DynWinRTValue.from_bool(True).to_bool() is True
    assert DynWinRTValue.from_bool(False).to_bool() is False
    assert DynWinRTValue.from_i32(0).to_bool() is False
    assert DynWinRTValue.from_i32(1).to_bool() is True


def test_enum_value_in_to_number():
    etype = DynWinRTType.enum_type("RuntimeEnum", ["X", "Y"], [10, 20])
    ev = DynWinRTValue.enum_value(etype, 20)
    assert ev.to_number() == 20
    assert ev.to_string() == "Y"
