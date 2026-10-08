# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Run unsafe-if-regressed receiver checks outside the pytest process."""

import subprocess
import sys


def test_raw_async_cannot_receive_unchecked_convenience_calls(tmp_path):
    script = r"""
import sys
from dynwinrt import (
    DynWinRTMethodSig, DynWinRTType, DynWinRTValue, RoApartment, WinGUID,
)

storage_iid = WinGUID.parse('fa3f6186-4214-428c-a64c-14c9ac7315ea')
statics_iid = WinGUID.parse('5984c710-daf2-43c8-8bb4-a4d3eacfd03f')
async_info_iid = WinGUID.parse('00000036-0000-0000-c000-000000000046')
uri_factory_iid = WinGUID.parse('44a9796f-723e-4fdf-a218-033e75b0c084')

with RoApartment():
    storage_type = DynWinRTType.runtime_class(
        'Windows.Storage.StorageFile',
        DynWinRTType.interface(storage_iid),
    )
    statics = DynWinRTType.register_interface(
        'IStorageFileStaticsAsyncReceiverGuard', statics_iid,
    ).add_method(
        'GetFileFromPathAsync',
        DynWinRTMethodSig()
            .add_in(DynWinRTType.hstring())
            .add_out(DynWinRTType.i_async_operation(storage_type)),
    )
    factory = DynWinRTValue.activation_factory(
        'Windows.Storage.StorageFile'
    ).cast(statics_iid)
    operation = statics.method(6).invoke(
        factory, [DynWinRTValue.from_hstring(sys.argv[1])]
    )

    released_arg = DynWinRTValue.from_hstring('not sent to native code')
    released_arg.release()
    for operation_name, invoke in (
        ('call_1()', lambda: operation.call_1(
            6, DynWinRTType.object(), released_arg,
        )),
        ('call_0()', lambda: operation.call_0(6, DynWinRTType.u32_type())),
    ):
        try:
            invoke()
        except RuntimeError as error:
            assert str(error) == (
                f'{operation_name} requires an Object value, got Async'
            ), error
        else:
            raise AssertionError(f'{operation_name} accepted an Async receiver')

    live_arg = DynWinRTValue.from_hstring('still not sent to native code')
    try:
        operation.call_1(6, DynWinRTType.object(), live_arg)
    except RuntimeError as error:
        assert str(error) == 'call_1() requires an Object value, got Async'
    else:
        raise AssertionError('call_1() accepted an Async receiver with a live argument')

    # An explicit IID cast remains a normal low-level Object receiver.
    info = operation.cast(async_info_iid)
    assert info.call_0(6, DynWinRTType.u32_type()).to_u32() >= 0

    uri_factory = DynWinRTValue.activation_factory(
        'Windows.Foundation.Uri'
    ).cast(uri_factory_iid)
    uri = uri_factory.call_1(
        6, DynWinRTType.object(),
        DynWinRTValue.from_hstring('https://example.com/checked'),
    )
    assert not uri.is_null()
    uri.release()
    uri_factory.release()
    info.release()
    operation.cancel()
    operation.release()
    factory.release()
print('async-receiver-rejected-before-dispatch', flush=True)
"""
    result = subprocess.run(
        [sys.executable, "-B", "-c", script, str(tmp_path / "missing-file")],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (result.returncode, result.stdout, result.stderr)
    assert "async-receiver-rejected-before-dispatch" in result.stdout
