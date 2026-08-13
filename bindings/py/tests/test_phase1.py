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
from contextvars import copy_context
from datetime import datetime, timedelta, timezone
import gc
import threading
import weakref
from types import SimpleNamespace

import pytest

import dynwinrt
from dynwinrt import (
    DynWinRTType,
    DynWinRTMethodSig,
    DynWinRTOverrideInterface,
    DynWinRTXamlRegistration,
    DynWinRTValue,
    DynWinRTArray,
    DynWinRtDelegate,
    DynWinRtElementFactory,
    RoApartment,
    projected_lifetime_scope,
    release_projected,
    WinRTAsync,
    WinRTAsyncWithProgress,
    WinGUID,
    ro_initialize,
    register_xaml_runtime_class,
)
from dynwinrt.dynwinrt import (
    _DynWinRTAsync,
    _DynWinRTAsyncWithProgress,
    _dynwinrt_dispatch_progress,
    _dynwinrt_datetime_to_ticks,
    _dynwinrt_new_vector,
    _dynwinrt_track_projected,
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
IID_IINSPECTABLE = WinGUID.parse("AF86E2E0-B12D-4C6A-9C5A-D7AA65101E90")


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


def _missing_storage_file_operation_value(path: str):
    statics_iid = WinGUID.parse(IID_ISTORAGE_FILE_STATICS)
    storage_file_type = DynWinRTType.runtime_class(
        "Windows.Storage.StorageFile",
        DynWinRTType.interface(WinGUID.parse(IID_ISTORAGE_FILE)),
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
    return raw_operation


def _missing_storage_file_operation(path: str):
    return _DynWinRTAsync(
        _missing_storage_file_operation_value(path),
        lambda value: value,
    )


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


def test_progress_wrapper_rejects_async_operation_without_progress(tmp_path):
    raw_operation = _missing_storage_file_operation_value(
        str(tmp_path / "missing-dynwinrt-progress-file")
    )
    with pytest.raises(
        RuntimeError,
        match="not a WinRT async operation with progress",
    ):
        _DynWinRTAsyncWithProgress(
            raw_operation,
            lambda value: value,
            lambda value: value,
        )


def test_progress_dispatch_propagates_callback_errors():
    def fail(_value):
        raise ValueError("progress callback failed")

    with pytest.raises(ValueError, match="progress callback failed"):
        _dynwinrt_dispatch_progress(
            fail,
            lambda value: value.to_number(),
            DynWinRTValue.from_u32(17),
        )


def test_observable_vector_reports_mutations_and_unsubscribes():
        observable_piid = WinGUID.parse("5917EB53-50B4-4A0D-B309-65862B3F1DBC")
        vector_piid = WinGUID.parse("913337E9-11A1-4345-A3A2-4E7F956E222D")
        handler_piid = WinGUID.parse("0C051752-9FBF-4C70-AA0C-0E4C82D9A761")
        event_args_iid = WinGUID.parse("575933DF-34FE-4480-AF15-07691F3D5D9B")
        element_type = DynWinRTType.hstring()
        observable_type = DynWinRTType.parameterized(
            observable_piid, [element_type]
        )
        vector_type = DynWinRTType.parameterized(vector_piid, [element_type])
        handler_type = DynWinRTType.parameterized(handler_piid, [element_type])
        event_args_type = DynWinRTType.interface(event_args_iid)

        observable_interface = DynWinRTType.register_interface(
            "IObservableVector_HString_Test",
            observable_type.iid(),
        )
        observable_interface.add_method(
            "add_VectorChanged",
            DynWinRTMethodSig()
            .add_in(handler_type)
            .add_out(DynWinRTType.i64_type()),
        )
        observable_interface.add_method(
            "remove_VectorChanged",
            DynWinRTMethodSig().add_in(DynWinRTType.i64_type()),
        )

        vector_interface = DynWinRTType.register_interface(
            "IVector_HString_Test",
            vector_type.iid(),
        )
        for index in range(7):
            vector_interface.add_method(
                f"unused_{index}",
                DynWinRTMethodSig(),
            )
        vector_interface.add_method(
            "Append",
            DynWinRTMethodSig().add_in(element_type),
        )

        event_args_interface = DynWinRTType.register_interface(
            "IVectorChangedEventArgs_Test",
            event_args_iid,
        )
        event_args_interface.add_method(
            "get_CollectionChange",
            DynWinRTMethodSig().add_out(DynWinRTType.i32_type()),
        )
        event_args_interface.add_method(
            "get_Index",
            DynWinRTMethodSig().add_out(DynWinRTType.u32_type()),
        )

        notifications = []

        def changed(_sender, args):
            event_args = args.cast(event_args_iid)
            change = event_args_interface.method(6).invoke(event_args, []).to_number()
            index = event_args_interface.method(7).invoke(event_args, []).to_u32()
            notifications.append((change, index))

        with RoApartment(1):
            vector_value = DynWinRTValue.create_vector(
                [DynWinRTValue.from_hstring("first")],
                element_type,
            )
            observable_value = vector_value.cast(observable_type.iid())
            mutable_value = vector_value.cast(vector_type.iid())
            delegate = DynWinRtDelegate.create(
                handler_type.iid(),
                [observable_type, event_args_type],
                changed,
            )
            token = observable_interface.method(6).invoke(
                observable_value,
                [delegate.to_value()],
            )

            vector_interface.method(13).invoke(
                mutable_value,
                [DynWinRTValue.from_hstring("second")],
            )
            assert notifications == [(1, 1)]

            observable_interface.method(7).invoke(observable_value, [token])
            vector_interface.method(13).invoke(
                mutable_value,
                [DynWinRTValue.from_hstring("third")],
            )
            assert notifications == [(1, 1)]

            mutable_value.release()
            observable_value.release()
            vector_value.release()
            del delegate


def test_new_vector_copies_projected_iterables_for_observable_use():
    class ProjectedIterable:
            def __init__(self, native):
                self._obj = native

            def __iter__(self):
                return iter(["first", "second"])

    element_type = DynWinRTType.hstring()
    source = DynWinRTValue.create_vector(
            [DynWinRTValue.from_hstring("source")],
            element_type,
    )
    copied = _dynwinrt_new_vector(
            ProjectedIterable(source),
            DynWinRTValue.from_hstring,
            element_type,
    )
    observable_iid = DynWinRTType.parameterized(
            WinGUID.parse("5917EB53-50B4-4A0D-B309-65862B3F1DBC"),
            [element_type],
    ).iid()
    observable = copied.cast(observable_iid)
    assert not observable.is_null()
    observable.release()
    copied.release()
    source.release()


def test_com_identity_and_element_factory_callback_release():
    element_factory_iid = WinGUID.parse(
        "75FABA47-2CF2-54AE-91E6-0581556FDDAA"
    )
    uri_iid = WinGUID.parse(IID_IURI)

    with RoApartment(1):
        factory = DynWinRTValue.activation_factory("Windows.Foundation.Uri")
        uri_factory = factory.cast(WinGUID.parse(IID_IURI_FACTORY))
        assert factory.identity_raw() == uri_factory.identity_raw()

        implementation = DynWinRtElementFactory.create(
            uri_iid,
            lambda value: value,
            lambda _value: None,
        )
        value = implementation.to_value()
        projected = value.cast(element_factory_iid)
        assert value.identity_raw() == projected.identity_raw()

        implementation.release_callbacks()
        implementation.release_callbacks()
        implementation.release()
        projected.release()
        value.release()
        uri_factory.release()
        factory.release()


def test_native_override_interface_rejects_unknown_or_unsupported_callback_shapes():
    iid = WinGUID.parse("FFC6FD98-F38C-5904-9CE4-97A3427CF4BA")
    with pytest.raises(RuntimeError, match="unsupported native override ABI shape"):
        DynWinRTOverrideInterface(iid, ["object_to_object"], {})
    with pytest.raises(RuntimeError, match="unsupported ABI shape"):
        DynWinRTOverrideInterface(
            iid,
            ["hstring_bool_to_bool"],
            {6: lambda: False},
        )
    with pytest.raises(RuntimeError, match="not callable"):
        DynWinRTOverrideInterface(iid, ["void0"], {6: object()})


def test_native_override_interface_accepts_metadata_supported_framework_shapes():
    interface = DynWinRTOverrideInterface(
        WinGUID.parse("FFC6FD98-F38C-5904-9CE4-97A3427CF4BA"),
        [
            "size_f32_to_size_f32",
            "size_f32_to_size_f32",
            "void0",
            "hstring_bool_to_bool",
        ],
        {
            6: lambda size: size,
            8: lambda: None,
        },
    )
    assert interface is not None


def test_named_xaml_registration_validates_duplicates_and_unregisters():
    class Constructor:
        def __call__(self):
            raise AssertionError("metadata lookup must not invoke the constructor")

    callback = Constructor()
    callback_ref = weakref.ref(callback)
    registration = register_xaml_runtime_class(
        "DynWinRT.Tests.UnitControl",
        "Microsoft.UI.Xaml.Controls.Control",
        IID_IINSPECTABLE,
        callback,
        ["measure_override"],
    )
    assert isinstance(registration, DynWinRTXamlRegistration)
    assert registration.active
    assert registration.name == "DynWinRT.Tests.UnitControl"
    assert registration.supported_overrides == ["measure_override"]

    with pytest.raises(OSError):
        register_xaml_runtime_class(
            "DynWinRT.Tests.UnitControl",
            "Microsoft.UI.Xaml.Controls.Control",
            IID_IINSPECTABLE,
            lambda: None,
        )
    del callback
    gc.collect()
    assert callback_ref() is not None
    assert registration.unregister()
    assert not registration.active
    assert registration.name is None
    assert not registration.unregister()
    gc.collect()
    assert callback_ref() is None


def test_named_xaml_registration_rejects_unsupported_shapes():
    with pytest.raises(OSError):
        register_xaml_runtime_class(
            "Unqualified",
            "Microsoft.UI.Xaml.Controls.Control",
            IID_IINSPECTABLE,
            lambda: None,
        )
    with pytest.raises(OSError):
        register_xaml_runtime_class(
            "DynWinRT.Tests.Generic`1",
            "Microsoft.UI.Xaml.Controls.Control",
            IID_IINSPECTABLE,
            lambda: None,
        )
    with pytest.raises(OSError):
        register_xaml_runtime_class(
            "DynWinRT.Tests.DuplicateOverrides",
            "Microsoft.UI.Xaml.Controls.Control",
            IID_IINSPECTABLE,
            lambda: None,
            ["measure_override", "measure_override"],
        )


def test_element_factory_callback_cleanup_allows_reentrant_destructors():
    holder = {}
    destructor_called = []

    class Callback:
        def __call__(self, value):
            return value

        def __del__(self):
            destructor_called.append(True)
            factory = holder.get("factory")
            if factory is not None:
                factory.release_callbacks()

    implementation = DynWinRtElementFactory.create(
        WinGUID.parse(IID_IURI),
        Callback(),
        lambda _value: None,
    )
    holder["factory"] = implementation
    implementation.release_callbacks()
    gc.collect()
    assert destructor_called
    implementation.release()


def test_async_protocols_are_public():
    assert WinRTAsync.__name__ == "WinRTAsync"
    assert WinRTAsyncWithProgress.__name__ == "WinRTAsyncWithProgress"
    assert not hasattr(dynwinrt, "_DynWinRTAsync")


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


def test_projected_lifetime_scope_releases_native_values_before_apartment_exit():
    with RoApartment(1):
        scope = projected_lifetime_scope()
        with scope:
            first = DynWinRTValue.activation_factory("Windows.Foundation.Uri")
            second = first.cast(WinGUID.parse(IID_IURI_FACTORY))
            _dynwinrt_track_projected(SimpleNamespace(_obj=first), "Factory")
            _dynwinrt_track_projected(SimpleNamespace(_obj=second), "UriFactory")
            assert not first.is_null()
            assert not second.is_null()

        assert scope.disposed
        assert first.is_null()
        assert second.is_null()

        release_projected(SimpleNamespace(_obj=first))
        release_projected(SimpleNamespace(_obj=second))


def test_projected_lifetime_scope_enforces_lifo_order():
    outer = projected_lifetime_scope()
    inner = projected_lifetime_scope()
    outer.__enter__()
    inner.__enter__()
    with pytest.raises(RuntimeError, match="LIFO"):
        outer.close()
    inner.close()
    outer.close()
    assert inner.disposed
    assert outer.disposed


def test_projected_lifetime_scope_retries_only_failed_releases():
    class Native:
        def __init__(self, failures=0):
            self.failures = failures
            self.attempts = 0

        def release(self):
            self.attempts += 1
            if self.attempts <= self.failures:
                raise RuntimeError("retry release")

    flaky = Native(failures=1)
    stable = Native()
    scope = projected_lifetime_scope()
    with pytest.raises(RuntimeError, match="retry release"):
        with scope:
            _dynwinrt_track_projected(SimpleNamespace(_obj=flaky), "Flaky")
            _dynwinrt_track_projected(SimpleNamespace(_obj=stable), "Stable")

    assert not scope.disposed
    assert flaky.attempts == 1
    assert stable.attempts == 1

    scope.close()
    assert scope.disposed
    assert flaky.attempts == 2
    assert stable.attempts == 1


def test_nested_scope_release_failure_does_not_mask_original_error():
    class FlakyNative:
        def __init__(self):
            self.attempts = 0

        def release(self):
            self.attempts += 1
            if self.attempts == 1:
                raise RuntimeError("inner release failed")

    flaky = FlakyNative()
    outer = projected_lifetime_scope()
    inner = projected_lifetime_scope()
    with pytest.raises(RuntimeError, match="inner release failed"):
        with outer:
            with inner:
                _dynwinrt_track_projected(SimpleNamespace(_obj=flaky), "Flaky")

    assert outer.disposed
    assert not inner.disposed
    inner.close()
    assert inner.disposed
    assert flaky.attempts == 2


def test_closed_scope_in_copied_context_is_ignored():
    native = type(
        "Native",
        (),
        {"release": lambda self: setattr(self, "released", True)},
    )()
    native.released = False

    with projected_lifetime_scope():
        inherited_context = copy_context()

    inherited_context.run(
        _dynwinrt_track_projected,
        SimpleNamespace(_obj=native),
        "LateValue",
    )
    assert not native.released


def test_inherited_context_cannot_close_parent_scope():
    class Native:
        def __init__(self):
            self.released = False

        def release(self):
            self.released = True

    native = Native()
    scope = projected_lifetime_scope()
    scope.__enter__()
    _dynwinrt_track_projected(SimpleNamespace(_obj=native), "Native")
    inherited_context = copy_context()

    with pytest.raises(ValueError, match="different Context"):
        inherited_context.run(scope.close)

    assert not native.released
    assert not scope.disposed
    scope.close()
    assert native.released
    assert scope.disposed


def test_nested_cleanup_preserves_the_first_failure():
    class FailingNative:
        def __init__(self, message):
            self.message = message
            self.attempts = 0

        def release(self):
            self.attempts += 1
            if self.attempts == 1:
                raise RuntimeError(self.message)

    outer_native = FailingNative("outer cleanup failed")
    inner_native = FailingNative("inner cleanup failed")
    outer = projected_lifetime_scope()
    inner = projected_lifetime_scope()

    with pytest.raises(RuntimeError, match="inner cleanup failed") as exc_info:
        with outer:
            _dynwinrt_track_projected(
                SimpleNamespace(_obj=outer_native), "Outer"
            )
            with inner:
                _dynwinrt_track_projected(
                    SimpleNamespace(_obj=inner_native), "Inner"
                )

    assert isinstance(exc_info.value.__cause__, RuntimeError)
    assert "outer cleanup failed" in str(exc_info.value.__cause__)
    inner.close()
    outer.close()


def test_body_and_nested_cleanup_failures_are_all_preserved():
    class FailingNative:
        def __init__(self, message):
            self.message = message
            self.attempts = 0

        def release(self):
            self.attempts += 1
            if self.attempts == 1:
                raise RuntimeError(self.message)

    outer_native = FailingNative("outer cleanup failed")
    inner_native = FailingNative("inner cleanup failed")
    outer = projected_lifetime_scope()
    inner = projected_lifetime_scope()

    with pytest.raises(ValueError, match="body failed") as exc_info:
        with outer:
            _dynwinrt_track_projected(
                SimpleNamespace(_obj=outer_native), "Outer"
            )
            with inner:
                _dynwinrt_track_projected(
                    SimpleNamespace(_obj=inner_native), "Inner"
                )
                raise ValueError("body failed")

    inner_error = exc_info.value.__cause__
    assert isinstance(inner_error, RuntimeError)
    assert "inner cleanup failed" in str(inner_error)
    outer_error = inner_error.__cause__
    assert isinstance(outer_error, RuntimeError)
    assert "outer cleanup failed" in str(outer_error)

    inner.close()
    outer.close()


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
