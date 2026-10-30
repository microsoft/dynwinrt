# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Isolate buffer regressions from pytest's process-lifetime COM initialization."""

import subprocess
import sys

import pytest


@pytest.mark.parametrize("model", [0, 1])
@pytest.mark.parametrize("scope", ["single", "separate"])
@pytest.mark.parametrize("cleanup", ["release", "gc", "apartment"])
def test_from_bytes_apartment_lifetime(model, scope, cleanup):
    script = r"""
import ctypes
import gc
import sys
from dynwinrt import DynWinRTValue, RoApartment

model, scope, cleanup = int(sys.argv[1]), sys.argv[2], sys.argv[3]
get_apartment = ctypes.WinDLL('combase').CoGetApartmentType
get_apartment.argtypes = [ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int)]
get_apartment.restype = ctypes.c_long
escaped = []

def assert_uninitialized():
    apartment, qualifier = ctypes.c_int(), ctypes.c_int()
    assert get_apartment(ctypes.byref(apartment), ctypes.byref(qualifier)) == -2147221008

def copy_bytes():
    for data in (b'abc', b'', bytearray(b'\x00\x01\xff\x80\x00')):
        expected = bytes(data)
        value = DynWinRTValue.from_bytes(data)
        if isinstance(data, bytearray):
            data[:] = b'\x09' * len(data)
        copied = value.to_bytes()
        if cleanup == 'release':
            value.release()
            assert value.is_released()
        elif cleanup == 'apartment':
            escaped.append(value)
        del value
        gc.collect()
        assert copied == expected

def assert_closed():
    assert_uninitialized()
    assert all(value.is_released() for value in escaped)
    escaped.clear()

assert_uninitialized()
if scope == 'single':
    with RoApartment(model):
        for _ in range(3):
            copy_bytes()
    assert_closed()
else:
    for _ in range(3):
        with RoApartment(model):
            copy_bytes()
        assert_closed()
print('buffer-apartment-clean-exit', flush=True)
"""
    result = subprocess.run(
        [sys.executable, "-X", "faulthandler", "-c", script, str(model), scope, cleanup],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (result.returncode, result.stdout, result.stderr)
    assert "buffer-apartment-clean-exit" in result.stdout
