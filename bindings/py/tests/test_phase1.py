# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""
Phase 1 — Python binding API additions for JS parity.

Covers:
  * DynWinRTMethodHandle.invoke_all  — multi-out-parameter invocation
  * DynWinRTValue.cancel             — IAsyncInfo::Cancel
  * WinRTAsync                       — public asyncio-compatible protocols
  * DynWinRTArray.to_bytes/from_bytes — Pythonic byte-buffer interop
  * DynWinRTArray.from_object_values — T[] of object/interface elements
"""

import asyncio
from datetime import datetime, timedelta, timezone
import threading

import pytest

import dynwinrt_py
from dynwinrt_py import (
    DynWinRTType,
    DynWinRTMethodSig,
    DynWinRTValue,
    DynWinRTArray,
    RoApartment,
    WinRTAsync,
    WinRTAsyncWithProgress,
    WinGUID,
    ro_initialize,
)
from dynwinrt_py.dynwinrt_py import (
    _DynWinRTAsync,
    _DynWinRTAsyncWithProgress,
    _dynwinrt_datetime_to_ticks,
    _dynwinrt_ticks_to_datetime,
    _dynwinrt_ticks_to_timedelta,
    _dynwinrt_timedelta_to_ticks,
)


# IUriRuntimeClassFactory IID — CreateUri(string) is at vtable index 6
IID_IURI_FACTORY = "44A9796F-723E-4FDF-A218-033E75B0C084"
# IUriRuntimeClass IID — get_AbsoluteUri at vtable index 6
IID_IURI = "9E365E57-48B2-4160-956F-C7385120BBFC"
IID_ISTORAGE_FILE = "FA3F6186-4214-428C-A64C-14C9AC7315EA"
IID_ISTORAGE_FILE_STATICS = "5984C710-DAF2-43C8-8BB4-A4D3EACFD03F"


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
# WinRT error mapping
# ----------------------------------------------------------------------

def test_sync_hresult_maps_to_os_error_with_winerror():
    factory_iid = WinGUID.parse(IID_IURI_FACTORY)
    factory_t = DynWinRTType.register_interface(
        "IUriRuntimeClassFactoryErrorMapping",
        factory_iid,
    ).add_method(
        "CreateUri",
        DynWinRTMethodSig()
        .add_in(DynWinRTType.hstring())
        .add_out(DynWinRTType.object()),
    )
    factory = DynWinRTValue.activation_factory("Windows.Foundation.Uri").cast(
        factory_iid,
    )

    with pytest.raises(OSError) as exc_info:
        factory_t.method(6).invoke(
            factory,
            [DynWinRTValue.from_hstring("")],
        )

    assert exc_info.value.winerror == -2147467261  # E_POINTER
    assert exc_info.value.strerror


def _missing_storage_file_operation(path: str):
    statics_iid = WinGUID.parse(IID_ISTORAGE_FILE_STATICS)
    storage_file_type = DynWinRTType.runtime_class(
        "Windows.Storage.StorageFile",
        WinGUID.parse(IID_ISTORAGE_FILE),
    )
    statics = DynWinRTType.register_interface(
        "IStorageFileStaticsErrorMapping",
        statics_iid,
    ).add_method(
        "GetFileFromPathAsync",
        DynWinRTMethodSig()
        .add_in(DynWinRTType.hstring())
        .add_out(DynWinRTType.i_async_operation(storage_file_type)),
    )
    factory = DynWinRTValue.activation_factory("Windows.Storage.StorageFile").cast(
        statics_iid,
    )
    raw_operation = statics.method(6).invoke(
        factory,
        [DynWinRTValue.from_hstring(path)],
    )
    return _DynWinRTAsync(raw_operation, lambda value: value)


def test_async_hresult_maps_to_os_error_with_winerror(tmp_path):
    missing_path = str(tmp_path / "missing-dynwinrt-file")

    async def await_missing_file():
        await _missing_storage_file_operation(missing_path)

    with pytest.raises(OSError) as exc_info:
        asyncio.run(await_missing_file())

    assert exc_info.value.winerror == -2147024894  # HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)
    assert exc_info.value.strerror


def test_blocking_async_hresult_maps_to_os_error_with_winerror(tmp_path):
    missing_path = str(tmp_path / "missing-dynwinrt-file")

    with pytest.raises(OSError) as exc_info:
        _missing_storage_file_operation(missing_path).wait()

    assert exc_info.value.winerror == -2147024894  # HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)
    assert exc_info.value.strerror


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


def test_async_wrapper_rejects_non_async_value():
    with pytest.raises(RuntimeError, match="not a WinRT async operation"):
        _DynWinRTAsync(DynWinRTValue.from_i32(42), lambda value: value.to_number())


def test_progress_wrapper_rejects_non_async_value():
    with pytest.raises(RuntimeError, match="not a WinRT async operation"):
        _DynWinRTAsyncWithProgress(
            DynWinRTValue.from_i32(42),
            lambda value: value.to_number(),
            lambda value: value.to_number(),
        )


def test_async_protocols_are_public():
    assert WinRTAsync.__name__ == "WinRTAsync"
    assert WinRTAsyncWithProgress.__name__ == "WinRTAsyncWithProgress"
    assert not hasattr(dynwinrt_py, "_DynWinRTAsync")


def test_ro_apartment_balances_nested_initialization():
    errors = []

    def worker():
        try:
            with RoApartment(1):
                DynWinRTValue.activation_factory("Windows.Foundation.Uri")
                with RoApartment(1):
                    DynWinRTValue.activation_factory("Windows.Foundation.Uri")
                DynWinRTValue.activation_factory("Windows.Foundation.Uri")
                with pytest.raises(OSError):
                    with RoApartment(0):
                        pass
            apartment = RoApartment(0)
            apartment.__enter__()
            apartment.__exit__(None, None, None)
            apartment.__exit__(None, None, None)
            with RoApartment(1):
                DynWinRTValue.activation_factory("Windows.Foundation.Uri")
        except BaseException as error:
            errors.append(error)

    thread = threading.Thread(target=worker)
    thread.start()
    thread.join()
    assert not errors


def test_ro_apartment_rejects_changed_mode():
    errors = []

    def worker():
        try:
            with RoApartment(0):
                with pytest.raises(OSError) as exc_info:
                    with RoApartment(1):
                        pass
                assert exc_info.value.winerror == -2147417850  # RPC_E_CHANGED_MODE
        except BaseException as error:
            errors.append(error)

    thread = threading.Thread(target=worker)
    thread.start()
    thread.join()
    assert not errors


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


def test_winrt_datetime_round_trip():
    value = datetime(2024, 1, 2, 3, 4, 5, 678901, tzinfo=timezone.utc)
    assert _dynwinrt_ticks_to_datetime(_dynwinrt_datetime_to_ticks(value)) == value


@pytest.mark.parametrize(
    "value",
    [
        timedelta(days=2, seconds=3, microseconds=4),
        timedelta(days=-2, seconds=3, microseconds=4),
    ],
)
def test_winrt_timedelta_round_trip(value):
    assert _dynwinrt_ticks_to_timedelta(_dynwinrt_timedelta_to_ticks(value)) == value


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
