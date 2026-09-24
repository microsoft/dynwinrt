# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Actionable errors for common runtime misuse."""

import re

import pytest

from dynwinrt import (
    DynWinRTMethodSig,
    DynWinRTType,
    DynWinRTValue,
    DynWinRtDelegate,
    RoApartment,
    WinGUID,
    projected_lifetime_scope,
    release_projected,
)
from dynwinrt.dynwinrt import (
    _dynwinrt_cache_projected,
    _dynwinrt_projected_from_native,
    _dynwinrt_track_projected,
)

IID_IURI_FACTORY = WinGUID.parse("44A9796F-723E-4FDF-A218-033E75B0C084")
IID_IURI = WinGUID.parse("9E365E57-48B2-4160-956F-C7385120BBFC")
IID_ISTRINGABLE = WinGUID.parse("96369F54-8EB6-48F0-ABCE-C1B211E627C3")
IID_TEST_DELEGATE = WinGUID.parse("5A0F1C3E-7B24-4D69-8E1F-2C3B4A5D6E7F")
RELEASED_REASON = (
    r"has been released: its projected_lifetime_scope\(\) exited or "
    r"release_projected\(\) was called\. Use the object inside its scope, or "
    r"don't release it\.$"
)
RELEASED = rf"^This WinRT object {RELEASED_REASON}"

_URI_FACTORY = DynWinRTType.register_interface(
    "ErrorGuidanceUriFactory", IID_IURI_FACTORY
).add_method(
    "CreateUri",
    DynWinRTMethodSig().add_in(DynWinRTType.hstring()).add_out(DynWinRTType.object()),
)
_URI = DynWinRTType.register_interface("ErrorGuidanceUri", IID_IURI).add_method(
    "get_AbsoluteUri", DynWinRTMethodSig().add_out(DynWinRTType.hstring())
)
_STRINGABLE = DynWinRTType.register_interface(
    "ErrorGuidanceStringable", IID_ISTRINGABLE
).add_method("ToString", DynWinRTMethodSig().add_out(DynWinRTType.hstring()))


def released_argument(position, operation):
    return (
        rf"^This WinRT object \(argument {position} of {re.escape(operation)}\) "
        rf"{RELEASED_REASON}"
    )


class ProjectedUri:
    """The generated runtime-class wrapper shape, reduced to two members."""

    def __new__(cls, *args, **kwargs):
        if len(args) == 1 and not kwargs and isinstance(args[0], DynWinRTValue):
            return _dynwinrt_projected_from_native(cls, args[0], "_set_native")
        return super().__new__(cls)

    def _set_native(self, obj):
        self._obj = obj.cast(IID_IURI)
        self._dynwinrt_native_ready = True
        _dynwinrt_track_projected(self, "Windows.Foundation.Uri")
        _dynwinrt_cache_projected(self)

    def __init__(self, obj):
        if getattr(self, "_dynwinrt_native_ready", False):
            return
        self._set_native(obj)

    @classmethod
    def create(cls, uri):
        factory = DynWinRTValue.activation_factory("Windows.Foundation.Uri").cast(
            IID_IURI_FACTORY
        )
        try:
            return cls(
                _URI_FACTORY.method(6).invoke(factory, [DynWinRTValue.from_hstring(uri)])
            )
        finally:
            factory.release()

    @property
    def absolute_uri(self):
        return _URI.method(6).invoke(self._obj, []).to_string()

    def to_string(self):
        stringable = self._obj.cast(IID_ISTRINGABLE)
        return _STRINGABLE.method(6).invoke(stringable, []).to_string()


def _assert_released(uri):
    with pytest.raises(RuntimeError, match=RELEASED):
        uri.absolute_uri
    with pytest.raises(RuntimeError, match=RELEASED):
        uri.to_string()
    assert uri._obj.is_null()


def _released_uri_value():
    value = DynWinRTValue.activation_factory("Windows.Foundation.Uri")
    value.release()
    value.release()
    assert value.is_null()
    return value


def test_projection_used_after_scope_exit_explains_the_release():
    with RoApartment():
        with projected_lifetime_scope():
            uri = ProjectedUri.create("https://example.com/scoped")
            assert uri.absolute_uri == "https://example.com/scoped"

        _assert_released(uri)


def test_projection_used_after_release_projected_explains_the_release():
    with RoApartment():
        uri = ProjectedUri.create("https://example.com/released")
        assert uri.to_string() == "https://example.com/released"
        release_projected(uri)

        _assert_released(uri)


def test_receivers_report_released_values_null_and_value_kinds():
    method = _URI.method(6)
    receivers = (
        ("invoke()", lambda value: method.invoke(value, [])),
        ("invoke_all()", lambda value: method.invoke_all(value, [])),
        ("invoke_detached()", lambda value: method.invoke_detached(value, [])),
        ("get_string()", lambda value: method.get_string(value)),
        ("call_0()", lambda value: value.call_0(6, DynWinRTType.hstring())),
        ("call()", lambda value: value.call(6, DynWinRTType.hstring(), [], [])),
        ("as_raw()", lambda value: value.as_raw()),
        ("identity_raw()", lambda value: value.identity_raw()),
        ("cast()", lambda value: value.cast(IID_IURI)),
    )
    for value, kind in (
        (DynWinRTValue.from_i32(7), "I32"),
        (DynWinRTValue.from_hstring("text"), "HString"),
        (DynWinRTValue.null_value(), "null"),
    ):
        for operation, call in receivers:
            expected = rf"^{re.escape(operation)} requires an Object value, got {kind}$"
            with pytest.raises(RuntimeError, match=expected):
                call(value)

    with RoApartment():
        released = _released_uri_value()
        for _, call in receivers:
            with pytest.raises(RuntimeError, match=RELEASED):
                call(released)

        buffer = DynWinRTValue.from_bytes(b"released")
        buffer.release()
        with pytest.raises(RuntimeError, match=RELEASED):
            buffer.to_bytes()


def test_released_arguments_are_rejected_by_position():
    with RoApartment():
        uri = ProjectedUri.create("https://example.com/argument")
        other = ProjectedUri.create("https://example.com/argument")
        released = _released_uri_value()
        method = _URI.method(6)
        live = DynWinRTValue.from_i32(1)
        # IUriRuntimeClass.Equals(Uri) is vtable slot 21.
        equals = (21, DynWinRTType.bool_type(), [DynWinRTType.object()])

        assert uri._obj.call(*equals, [other._obj]).to_bool()
        for operation, call in (
            ("invoke()", lambda: method.invoke(uri._obj, [live, released])),
            ("invoke_all()", lambda: method.invoke_all(uri._obj, [live, released])),
            (
                "invoke_detached()",
                lambda: method.invoke_detached(uri._obj, [live, released]),
            ),
            ("call()", lambda: uri._obj.call(*equals, [live, released])),
        ):
            with pytest.raises(RuntimeError, match=released_argument(1, operation)):
                call()
        with pytest.raises(RuntimeError, match=released_argument(0, "call_1()")):
            uri._obj.call_1(21, DynWinRTType.bool_type(), released)

        release_projected(uri)
        release_projected(other)


def test_delegate_invocation_rejects_released_receivers_and_arguments():
    signature = DynWinRTMethodSig().add_in(DynWinRTType.object())
    calls = []
    delegate = DynWinRtDelegate.create(
        IID_TEST_DELEGATE,
        [DynWinRTType.object()],
        lambda argument: calls.append(argument.is_null()),
    ).to_value()
    try:
        assert delegate.invoke_delegate(
            IID_TEST_DELEGATE, signature, [DynWinRTValue.null_value()]
        ) == []
        assert calls == [True]

        with RoApartment():
            released = _released_uri_value()
            with pytest.raises(
                RuntimeError, match=released_argument(0, "delegate Invoke()")
            ):
                delegate.invoke_delegate(IID_TEST_DELEGATE, signature, [released])
        assert calls == [True]
    finally:
        delegate.release()

    with pytest.raises(RuntimeError, match=RELEASED):
        delegate.invoke_delegate(
            IID_TEST_DELEGATE, signature, [DynWinRTValue.null_value()]
        )
    with pytest.raises(
        RuntimeError, match=r"^delegate Invoke\(\) requires an Object value, got null$"
    ):
        DynWinRTValue.null_value().invoke_delegate(IID_TEST_DELEGATE, signature, [])
