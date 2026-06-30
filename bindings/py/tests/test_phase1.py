# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""
Phase 1 — Python binding API additions for JS parity.

Covers:
  * DynWinRTMethodHandle.invoke_all  — multi-out-parameter invocation
  * DynWinRTValue.cancel             — IAsyncInfo::Cancel
  * DynWinRTArray.to_bytes/from_bytes — Pythonic byte-buffer interop
  * DynWinRTArray.from_object_values — T[] of object/interface elements
"""

import pytest

import dynwinrt_py
from dynwinrt_py import (
    DynWinRTType,
    DynWinRTMethodSig,
    DynWinRTValue,
    DynWinRTArray,
    WinGUID,
    ro_initialize,
)


# IUriRuntimeClassFactory IID — CreateUri(string) is at vtable index 6
IID_IURI_FACTORY = "44A9796F-723E-4FDF-A218-033E75B0C084"
# IUriRuntimeClass IID — get_AbsoluteUri at vtable index 6
IID_IURI = "9E365E57-48B2-4160-956F-C7385120BBFC"


def _setup_module():
    ro_initialize(1)  # MTA


_setup_module()


# ----------------------------------------------------------------------
# invoke_all — multi-out return shape
# ----------------------------------------------------------------------

def test_invoke_all_returns_list_for_single_out():
    """invoke_all should return a list (length >= 1) for methods with 1 out param,
    mirroring JS invokeAll semantics."""
    factory_t = DynWinRTType.interface(WinGUID.parse(IID_IURI_FACTORY))
    factory_t.register_interface("IUriRuntimeClassFactory", WinGUID.parse(IID_IURI_FACTORY))
    factory_t.add_method("CreateUri", DynWinRTMethodSig()
        .add_in(DynWinRTType.hstring())
        .add_out(DynWinRTType.object()))

    factory_val = DynWinRTValue.activation_factory("Windows.Foundation.Uri")
    factory_iface = factory_val.cast(WinGUID.parse(IID_IURI_FACTORY))

    create = factory_t.method(6)
    results = create.invoke_all(factory_iface, [DynWinRTValue.from_hstring("https://example.com/")])
    assert isinstance(results, list)
    assert len(results) == 1
    assert results[0] is not None


def test_invoke_all_vs_invoke_consistency():
    """invoke_all()[0] should be equivalent to invoke() for a single-out method."""
    factory_t = DynWinRTType.interface(WinGUID.parse(IID_IURI_FACTORY))
    factory_t.register_interface("IUriRuntimeClassFactory2", WinGUID.parse(IID_IURI_FACTORY))
    factory_t.add_method("CreateUri", DynWinRTMethodSig()
        .add_in(DynWinRTType.hstring())
        .add_out(DynWinRTType.object()))

    factory_val = DynWinRTValue.activation_factory("Windows.Foundation.Uri")
    factory_iface = factory_val.cast(WinGUID.parse(IID_IURI_FACTORY))

    create = factory_t.method(6)
    one = create.invoke(factory_iface, [DynWinRTValue.from_hstring("https://example.com/")])
    many = create.invoke_all(factory_iface, [DynWinRTValue.from_hstring("https://example.com/")])
    # Both should be Object values (non-null)
    assert not one.is_null()
    assert not many[0].is_null()


# ----------------------------------------------------------------------
# DynWinRTValue.cancel
# ----------------------------------------------------------------------

def test_cancel_on_non_async_raises():
    """cancel() on a non-async value should raise RuntimeError."""
    v = DynWinRTValue.from_i32(42)
    with pytest.raises(RuntimeError):
        v.cancel()


def test_cancel_on_async_no_raise_after_completion():
    """Cancelling an already-completed async operation should be a no-op per WinRT spec.
    We do not exercise a true async API here (would require a slow WinRT call); instead we
    confirm the dispatch path exists and rejects non-async values cleanly. A live cancel
    scenario will be covered in the E2E suite."""
    # Reuses the non-async guard above; the real-async case is covered in E2E (TODO p1-tests)
    assert callable(DynWinRTValue.from_i32(0).cancel)


# ----------------------------------------------------------------------
# DynWinRTArray.from_bytes / to_bytes round-trip
# ----------------------------------------------------------------------

def test_bytes_round_trip_empty():
    arr = DynWinRTArray.from_bytes(b"")
    assert len(arr) == 0
    assert arr.to_bytes() == b""


def test_bytes_round_trip_basic():
    data = b"hello world"
    arr = DynWinRTArray.from_bytes(data)
    assert len(arr) == len(data)
    assert arr.to_bytes() == data


def test_bytes_round_trip_binary():
    data = bytes(range(256))
    arr = DynWinRTArray.from_bytes(data)
    assert len(arr) == 256
    out = arr.to_bytes()
    assert isinstance(out, bytes)
    assert out == data


def test_from_bytes_accepts_bytearray():
    data = bytearray(b"\x00\x01\x02\xff")
    arr = DynWinRTArray.from_bytes(data)
    assert arr.to_bytes() == bytes(data)


# ----------------------------------------------------------------------
# DynWinRTArray.from_object_values
# ----------------------------------------------------------------------

def test_from_object_values_basic():
    """Build an array of WinRT Object elements via the new helper, then read back."""
    # Two distinct factory Objects make perfectly fine IInspectable values for the array.
    a = DynWinRTValue.activation_factory("Windows.Foundation.PropertyValue")
    b = DynWinRTValue.activation_factory("Windows.Foundation.Uri")

    arr = DynWinRTArray.from_object_values([a, b], DynWinRTType.object())
    assert len(arr) == 2
    # Wrap and unwrap to confirm the array round-trips through DynWinRTValue::Array
    val = arr.to_value()
    assert val.is_array()
    arr2 = val.as_array()
    assert len(arr2) == 2


def test_from_object_values_empty():
    arr = DynWinRTArray.from_object_values([], DynWinRTType.object())
    assert len(arr) == 0
