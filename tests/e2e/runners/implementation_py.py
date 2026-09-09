# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Generated implementations exercised only via real native vtable invocations.

Each case is a subprocess, so native crashes, cleanup failures, and hangs are
reported without silently dropping the rest of the requested coverage.
"""

from __future__ import annotations

import argparse
from contextlib import ExitStack, contextmanager
from datetime import datetime, timedelta, timezone
import gc
import importlib
import importlib.util
import inspect
import json
from pathlib import Path
import platform
import re
import struct
import subprocess
import sys
import time
import traceback
from uuid import UUID
import weakref


RESULT_PREFIX = "DYNWINRT_IMPLEMENTATION_RESULT="
RO_E_CLOSED = 0x80000013
PYTHON_CALLBACK_ERROR = 0xA0EE4005  # Existing PyWinRT-compatible callback HRESULT.
INSTANCE_ID = UUID("243aab7d-9411-49af-b66d-0123456789ab")


def not_implemented(*_args):
    raise NotImplementedError("Unused standalone E2E fixture method")


def expect_hresult(action, expected):
    try:
        action()
    except OSError as error:
        actual = error.winerror & 0xFFFFFFFF
        assert actual == expected, (
            f"Expected native HRESULT 0x{expected:08x}, got 0x{actual:08x}: {error}"
        )
        return error
    raise AssertionError(f"Expected native HRESULT 0x{expected:08x}")


def expect_error(action, pattern):
    try:
        action()
    except Exception as error:
        assert re.search(pattern, str(error), re.IGNORECASE), str(error)
        return error
    raise AssertionError(f"Expected an error matching {pattern!r}")


def take_error(owner, pattern):
    error = owner.take_error()
    assert error is not None, "Callback failure must be available through take_error()"
    assert re.search(pattern, str(error), re.IGNORECASE), str(error)
    assert owner.take_error() is None, "take_error() must drain the stored error"


@contextmanager
def native_consumer(dw, owner, name, iid_text, method_name, signature):
    # These stock interfaces each have exactly one method at slot 6. A
    # canonical IInspectable value is NOT that view: QI before any vtable call.
    iid = dw.WinGUID.parse(iid_text)
    interface = dw.DynWinRTType.register_interface(f"E2E.{name}Consumer", iid)
    method = interface.add_method(method_name, signature).method(6)
    canonical = owner.to_value()
    try:
        value = canonical.cast(iid)
    finally:
        canonical.release()
    try:
        yield lambda: method.invoke(value, [])
    finally:
        value.release()


def native_stringable(dw, owner):
    return native_consumer(
        dw, owner, "IStringable", "96369f54-8eb6-48f0-abce-c1b211e627c3", "ToString",
        dw.DynWinRTMethodSig().add_out(dw.DynWinRTType.hstring()),
    )


class TaskInstanceHandlers:
    # All nine exact IBackgroundTaskInstance methods, in slots 6 through 14.
    def __init__(self):
        self.progress = 17
        self.gets = self.sets = self.get_ids = 0

    def get_instance_id(self):
        self.get_ids += 1
        return INSTANCE_ID

    get_task = not_implemented

    def get_progress(self):
        self.gets += 1
        return self.progress

    def set_progress(self, value):
        self.sets += 1
        self.progress = value

    get_trigger_details = not_implemented
    add_canceled = not_implemented
    remove_canceled = not_implemented
    get_suspended_count = not_implemented
    get_deferral = not_implemented


class ReaderHandlers:
    """Complete IDataReader vtable, with only its FillArray method implemented."""

    get_unconsumed_buffer_length = not_implemented
    get_unicode_encoding = not_implemented
    set_unicode_encoding = not_implemented
    get_byte_order = not_implemented
    set_byte_order = not_implemented
    get_input_stream_options = not_implemented
    set_input_stream_options = not_implemented
    read_byte = not_implemented
    read_buffer = not_implemented
    read_boolean = not_implemented
    read_guid = not_implemented
    read_int16 = not_implemented
    read_int32 = not_implemented
    read_int64 = not_implemented
    read_uint16 = not_implemented
    read_uint32 = not_implemented
    read_uint64 = not_implemented
    read_single = not_implemented
    read_double = not_implemented
    read_string = not_implemented
    read_date_time = not_implemented
    read_time_span = not_implemented
    load_async = not_implemented
    detach_buffer = not_implemented
    detach_stream = not_implemented

    def __init__(self, fill):
        self.fill = fill

    def read_bytes(self, capacity):
        return self.fill(capacity)


class CloseHandlers:
    def close(self):
        pass


def management_handle(g, dw, own):
    class Text:
        calls = 0

        def to_string(self):
            self.calls += 1
            return "typed primary"

    handler = Text()
    with g.IStringable.implement(handler, interfaces=[(g.IClosable, CloseHandlers())]) as impl:
        assert impl.value is impl.value
        assert isinstance(impl.value, g.IStringable)
        assert impl.value.to_string() == "typed primary"
        extra = g.IStringable.from_implementation(impl)
        assert extra is not impl.value
        primary = impl.value
        closer = g.IClosable.from_implementation(impl)
        closer.close()
        impl.release()
        expect_error(lambda: impl.value, "released")
        expect_error(primary.to_string, "object|released")
        assert extra.to_string() == "typed primary"
    expect_hresult(extra.to_string, RO_E_CLOSED)
    assert handler.calls == 2
    dw.release_projected(extra)
    dw.release_projected(closer)
    lazy = own(g.IStringable.implement(Text()))
    lazy.release()
    expect_error(lambda: lazy.value, "released")
    assert lazy.is_closed
    for entries in ([(g.IStringable, object())], [(None, handler)], [(g.IClosable,)], [(g.IStringable, handler)]):
        expect_error(lambda: g.IStringable.implement(handler, interfaces=entries), "handler|entry|entries|duplicate")

    class Broken(g.IStringable):
        failed_owner = None

        @classmethod
        def from_implementation(cls, owner):
            cls.failed_owner = owner
            raise ValueError("primary view failure")

    broken = own(Broken.implement(Text()))
    expect_error(lambda: broken.value, "primary view failure")
    assert Broken.failed_owner.is_closed
    expect_error(lambda: broken.value, "closed|released")
    class BrokenInitializer(g.IStringable):
        def _set_native(self, native, *, cache=True):
            raise ValueError("native view initializer failed")
    failed_init = own(BrokenInitializer.implement(Text()))
    expect_error(lambda: failed_init.value, "native view initializer failed")
    assert failed_init.is_closed
    expect_error(lambda: failed_init.value, "closed|released")
    class WrongResult:
        def to_string(self): return 17
    wrong = own(g.IStringable.implement(WrongResult()))
    expect_hresult(wrong.value.to_string, PYTHON_CALLBACK_ERROR)
    take_error(wrong, "invalid implementation result")

    class ReleasedDuringProjection(g.IStringable):
        @classmethod
        def from_implementation(cls, owner):
            value = super().from_implementation(owner)
            reentrant.release()
            return value

    reentrant = own(ReleasedDuringProjection.implement(Text()))
    expect_error(lambda: reentrant.value, "released during")
    assert reentrant.is_closed
    class DisposeInCallback:
        def to_string(self):
            disposing.dispose()
            return "entered callback completed"
    disposing = own(g.IStringable.implement(DisposeInCallback()))
    assert disposing.value.to_string() == "entered callback completed"
    assert disposing.is_closed
    expect_error(lambda: disposing.value, "closed|released")
    gc_impl = g.IStringable.implement(Text())
    primary = gc_impl.value
    raw = gc_impl.to_value()
    reference = weakref.ref(gc_impl)
    del primary, gc_impl
    gc.collect()
    assert reference() is None
    retained = g.IStringable.from_value(raw)
    raw.release()
    assert retained.to_string() == "typed primary"
    dw.release_projected(retained)

def property_views(g, dw, own):
    class Properties:
        length = 0
        def get_capacity(self): return 12
        def get_length(self): return self.length
        def set_length(self, value):
            if value > 12: raise ValueError("capacity exceeded")
            self.length = value
        def get_absolute_canonical_uri(self): return "https://example.test/\u96ea"
        def get_display_iri(self): return "https://example.test/\u96ea"
        def get_name(self): return "query"
        def get_value(self): return "value\0\u96ea"

    handlers = Properties()
    with g.IBuffer.implement(handlers, interfaces=[
        (g.IUriRuntimeClassWithAbsoluteCanonicalUri, handlers),
        (g.IWwwFormUrlDecoderEntry, handlers),
    ]) as impl:
        uri = g.IUriRuntimeClassWithAbsoluteCanonicalUri.from_implementation(impl)
        entry = g.IWwwFormUrlDecoderEntry.from_implementation(impl)
        try:
            assert impl.value.capacity == 12 and impl.value.length == 0
            impl.value.length = 7
            assert impl.value.length == handlers.length == 7
            assert uri.absolute_canonical_uri == "https://example.test/\u96ea"
            assert uri.display_iri == "https://example.test/\u96ea"
            assert entry.name == "query" and entry.value == "value\0\u96ea"
            # Legacy cached constructors must remain independent of the new
            # handle-managed primary and independent-view APIs.
            raw = impl.to_value()
            try:
                cached = g.IBuffer.from_value(raw)
                assert cached is g.IBuffer.from_value(raw)
                assert cached is not impl.value
                assert cached.length == 7
                uri_alias = cached.as_interface(g.IUriRuntimeClassWithAbsoluteCanonicalUri)
                entry_alias = uri_alias.as_interface(g.IWwwFormUrlDecoderEntry)
                assert uri_alias.display_iri == uri.display_iri
                assert entry_alias.value == entry.value
                for alias in (cached, uri_alias, entry_alias):
                    dw.release_projected(alias)
            finally:
                raw.release()
            expect_hresult(lambda: setattr(impl.value, "length", 13), PYTHON_CALLBACK_ERROR)
            take_error(impl, "capacity exceeded")
            assert impl.value.length == 7
            impl.value.length = 0
            assert impl.value.length == 0
        finally:
            dw.release_projected(uri)
            dw.release_projected(entry)


def background_task(g, dw, own):
    state = TaskInstanceHandlers()
    instance_owner = own(g.IBackgroundTaskInstance.implement(state))
    instance = instance_owner.value

    class Task:
        runs = 0

        def run(self, task_instance):
            assert isinstance(task_instance, g.IBackgroundTaskInstance)
            assert task_instance.instance_id == INSTANCE_ID
            task_instance.progress = task_instance.progress + 5
            self.runs += 1

    handlers = Task()
    owner = own(g.IBackgroundTask.implement(handlers))
    task = owner.value
    task.run(instance)
    assert (state.progress, state.gets, state.sets, state.get_ids) == (22, 1, 1, 1)
    assert handlers.runs == 1
    retained_instance = g.IBackgroundTaskInstance.from_implementation(instance_owner)
    retained_task = g.IBackgroundTask.from_implementation(owner)
    instance_owner.release()
    owner.release()
    retained_task.run(retained_instance)
    assert (state.progress, state.gets, state.sets, state.get_ids) == (27, 2, 2, 2)
    assert handlers.runs == 2
    token = dw.DynWinRTStruct.create(dw.DynWinRTType.struct_type(
        "Windows.Foundation.EventRegistrationToken", [dw.DynWinRTType.i64_type()]
    ))
    token.set_i64(0, 1)
    for action in (
        lambda: retained_instance.task,
        lambda: retained_instance.trigger_details,
        lambda: retained_instance.suspended_count,
        lambda: retained_instance.get_deferral(),
        lambda: retained_instance.on_canceled(lambda *_: None),
        lambda: retained_instance.off_canceled(token.to_value()),
    ):
        error = expect_hresult(action, PYTHON_CALLBACK_ERROR)
        assert "NotImplementedError" in str(error) and "Unused standalone" in str(error)
        take_error(instance_owner, "unused standalone")


def multi_interface_lifetime(g, dw, own):
    class Handlers:
        closes = 0

        def to_string(self):
            return "retained \0 WinRT \u96ea"

        def close(self):
            self.closes += 1

    handlers = Handlers()
    owner = own(g.IStringable.implement(handlers, g.IClosable.implementation(handlers)))
    first = g.IStringable.from_implementation(owner)
    second = g.IStringable.from_implementation(owner)
    closable = g.IClosable.from_implementation(owner)
    assert first is not second, "Typed views must not share one mutable native holder"
    retained = owner.to_value()
    try:
        dw.release_projected(first)
        assert second.to_string() == "retained \0 WinRT \u96ea"
        owner.release()
        owner.release()
        assert not owner.is_closed, "release() must not disconnect native views"
        assert second.to_string() == "retained \0 WinRT \u96ea"
        closable.close()
        assert handlers.closes == 1
        native_retained_view = g.IStringable.from_value(retained)
        retained.release()
        assert native_retained_view.to_string() == "retained \0 WinRT \u96ea"
        assert owner.take_error() is None
    finally:
        retained.release()

    scoped_owner = own(g.IStringable.implement(handlers))
    with dw.projected_lifetime_scope():
        native = scoped_owner.to_value()
        try:
            cached = g.IStringable.from_value(native)
            first = g.IStringable.from_implementation(scoped_owner)
            second = g.IStringable.from_implementation(scoped_owner)
            assert first is not second and first is not cached
            assert g.IStringable.from_value(native) is cached, "Existing from_value cache behavior changed"
        finally:
            native.release()
        dw.release_projected(first)
        assert second.to_string() == "retained \0 WinRT \u96ea"
    expect_error(second.to_string, "object|released|null")
    expect_error(cached.to_string, "object|released|null")
    assert not scoped_owner.is_closed
    recovered = g.IStringable.from_implementation(scoped_owner)
    assert recovered.to_string() == "retained \0 WinRT \u96ea"
    dw.release_projected(recovered)
    scoped_owner.release()
    assert scoped_owner.is_closed


def gc_referent_ids(owner):
    # IDs only: keeping the objects returned by get_referents() would itself
    # retain the callback/handler graph and invalidate a collection assertion.
    native_owner = getattr(owner, "_owner", owner)
    return frozenset(id(value) for value in gc.get_referents(native_owner))


def generated_owner_cycle(g):
    class Handlers:
        def to_string(self):
            return "public helper GC"

    handlers = Handlers()
    owner = g.IStringable.implement(handlers)
    handlers.owner = owner
    assert gc.is_tracked(owner)
    return owner, weakref.ref(owner), weakref.ref(handlers)


def collect_owner_cycle(dw, owner_ref, handlers_ref):
    for _ in range(3):
        gc.collect()
        # Flush any PyO3 decrefs deferred by native destruction. This does not
        # create a native instance or retain an implementation reference.
        dw.DynWinRTType.i32_type()
        if owner_ref() is None and handlers_ref() is None:
            return
    raise AssertionError(
        "Owner-only cycle was not collected; a public helper may have leaked "
        "its canonical temporary or a QueryInterface reference"
    )


def public_view_success_gc(g, dw, own):
    # Do not use own(): its bound cleanup method would be a strong Python root.
    owner, owner_ref, handlers_ref = generated_owner_cycle(g)
    first = second = None
    try:
        baseline = gc_referent_ids(owner)
        assert baseline, "Owner-only callbacks must participate in Python GC"
        first = g.IStringable.from_implementation(owner)
        second = g.IStringable.from_implementation(owner)
        assert first is not second
        assert gc_referent_ids(owner) != baseline, (
            "A returned view must own an external native QI reference"
        )
        dw.release_projected(first)
        first = None
        assert not owner.is_closed
        assert second.to_string() == "public helper GC"
        with native_stringable(dw, owner) as invoke:
            assert invoke().to_string() == "public helper GC"

        # Only the view and the native callback -> handler -> owner cycle now
        # keep the object alive. GC must not disconnect a retained native view.
        del owner
        gc.collect()
        assert owner_ref() is not None and handlers_ref() is not None
        assert second.to_string() == "public helper GC"
        dw.release_projected(second)
        second = None

        remaining = owner_ref()
        if remaining is not None:
            assert gc_referent_ids(remaining) == baseline, (
                "Successful from_implementation left an extra native root"
            )
        del remaining
        collect_owner_cycle(dw, owner_ref, handlers_ref)
    finally:
        for view in (first, second):
            if view is not None:
                dw.release_projected(view)
        remaining = owner_ref()
        if remaining is not None:
            remaining.dispose()


def public_view_failed_cast_gc(g, dw, own):
    owner, owner_ref, handlers_ref = generated_owner_cycle(g)
    try:
        baseline = gc_referent_ids(owner)
        assert baseline, "Owner-only callbacks must participate in Python GC"
        for _ in range(8):
            caught = expect_hresult(
                lambda: g.IClosable.from_implementation(owner), 0x80004002
            )
            # Keep the exception AND its traceback alive during this check.
            # Without finally: value.release(), the failed helper's traceback
            # can retain value=owner.to_value(), hiding the callback from GC.
            assert gc_referent_ids(owner) == baseline, (
                "Failed from_implementation did not release its canonical "
                "temporary before propagating E_NOINTERFACE"
            )
            del caught
            assert not owner.is_closed
        with native_stringable(dw, owner) as invoke:
            assert invoke().to_string() == "public helper GC"
        assert gc_referent_ids(owner) == baseline
        del owner
        collect_owner_cycle(dw, owner_ref, handlers_ref)
    finally:
        remaining = owner_ref()
        if remaining is not None:
            remaining.dispose()


def dispose_disconnects(g, dw, own):
    class Handlers:
        calls = 0

        def to_string(self):
            self.calls += 1
            return "before disposal"

        def close(self):
            self.calls += 1

    handlers = Handlers()
    owner = own(g.IStringable.implement(handlers, g.IClosable.implementation(handlers)))
    with native_stringable(dw, owner) as text, native_consumer(
        dw, owner, "IClosable", "30d5a829-7fa4-4026-83bb-d75bae4ea99e", "Close", dw.DynWinRTMethodSig()
    ) as close:
        assert text().to_string() == "before disposal"
        close()
        owner.release()
        assert text().to_string() == "before disposal"
        assert not owner.is_closed
        owner.dispose()
        owner.dispose()
        assert owner.is_closed
        expect_hresult(text, RO_E_CLOSED)
        expect_hresult(close, RO_E_CLOSED)
        assert handlers.calls == 3


def reentrant_dispose(g, dw, own):
    class Handlers:
        calls = 0

        def to_string(self):
            self.calls += 1
            owner.dispose()
            return "active callback finished"

    handlers = Handlers()
    owner = own(g.IStringable.implement(handlers))
    with native_stringable(dw, owner) as invoke:
        assert invoke().to_string() == "active callback finished"
        assert owner.is_closed
        expect_hresult(invoke, RO_E_CLOSED)
        assert handlers.calls == 1


def callback_error(g, dw, own):
    class Handlers:
        fail = True

        def to_string(self):
            if self.fail:
                raise ValueError("implementation-e2e-python-sentinel")
            return "recovered"

    handlers = Handlers()
    owner = own(g.IStringable.implement(handlers))
    with native_stringable(dw, owner) as invoke:
        expect_hresult(invoke, PYTHON_CALLBACK_ERROR)
        take_error(owner, "implementation-e2e-python-sentinel")
        handlers.fail = False
        assert invoke().to_string() == "recovered"
        assert not owner.is_closed
        assert owner.take_error() is None


def async_handler_rejected(g, dw, own):
    class Handlers:
        entered = False

        async def to_string(self):
            self.entered = True
            return "bad"

    handlers = Handlers()
    expect_error(lambda: g.IStringable.implementation(handlers), "async|synchronous")
    assert not handlers.entered

    class AsyncClose:
        entered = False

        async def close(self):
            self.entered = True

    close = AsyncClose()
    expect_error(lambda: g.IClosable.implementation(close), "async|synchronous")
    assert not close.entered


def async_result_rejected(g, dw, own):
    async def result():
        return "not a WinRT result"

    class Handlers:
        pending = None

        def to_string(self):
            self.pending = result()
            return self.pending

    handlers = Handlers()
    owner = own(g.IStringable.implement(handlers))
    try:
        with native_stringable(dw, owner) as invoke:
            expect_hresult(invoke, PYTHON_CALLBACK_ERROR)
            take_error(owner, "coroutine|awaitable|async|synchronous")
            assert inspect.getcoroutinestate(handlers.pending) == inspect.CORO_CLOSED, (
                "Rejected coroutine results must be closed, not left to warn at GC"
            )
    finally:
        if handlers.pending is not None:
            handlers.pending.close()


def required_interfaces(g, dw, own):
    class Reference:
        def get_capacity(self):
            return 73

        add_closed = not_implemented
        remove_closed = not_implemented

    expect_error(
        lambda: g.IMemoryBufferReference.implement(Reference()),
        "required|IClosable|30d5a829",
    )
    owner = own(g.IMemoryBufferReference.implement(
        Reference(), g.IClosable.implementation(CloseHandlers())
    ))
    assert g.IMemoryBufferReference.from_implementation(owner).capacity == 73
    g.IClosable.from_implementation(owner).close()

    class Text:
        def to_string(self):
            return "not IClosable"

    expect_error(
        lambda: g.IStringable.implement(Text(), g.IStringable.implementation(Text())),
        "duplicate|already|IID",
    )
    only_stringable = own(g.IStringable.implement(Text()))
    expect_hresult(lambda: g.IClosable.from_implementation(only_stringable), 0x80004002)


def memory_buffer_event(g, dw, own):
    callbacks = {}
    delivered = []

    class Reference:
        added = removed = 0

        def get_capacity(self):
            return 4096

        def add_closed(self, handler):
            assert callable(handler), "Received delegates are typed callable native views"
            self.added += 1
            callbacks[self.added] = handler
            return g.EventRegistrationToken(value=self.added)

        def remove_closed(self, token):
            assert isinstance(token, g.EventRegistrationToken)
            del callbacks[token.value]
            self.removed += 1

        def close(self):
            for callback in list(callbacks.values()):
                # Generated delegate wrapper -> native Invoke slot -> Python
                # callback. The original event callback is never called here.
                callback(event_sender, None)

    handlers = Reference()
    owner = own(g.IMemoryBufferReference.implement(
        handlers, g.IClosable.implementation(handlers)
    ))
    event_sender = g.IMemoryBufferReference.from_implementation(owner)
    close = g.IClosable.from_implementation(owner)

    def on_closed(sender, args):
        assert isinstance(sender, g.IMemoryBufferReference)
        assert sender.capacity == 4096  # Reentrant outbound native getter.
        assert args is None
        delivered.append(sender.capacity)

    unsubscribe = event_sender.subscribe_closed(on_closed)
    assert handlers.added == 1
    owner.release()
    close.close()
    assert delivered == [4096]
    unsubscribe()
    assert handlers.removed == 1
    close.close()
    assert delivered == [4096]
    owner.dispose()
    expect_hresult(close.close, RO_E_CLOSED)


def array_contracts(g, dw, own):
    # Keep ExclusiveTo factory interfaces private. These complete public SDK
    # interfaces cover native pass/receive arrays and typed value ownership.
    state = {"bytes": b"", "text": "", "guid": INSTANCE_ID, "signed": 0, "unsigned": 0, "real": 0.0}

    class Writer:
        get_unstored_buffer_length = not_implemented
        get_unicode_encoding = not_implemented
        set_unicode_encoding = not_implemented
        get_byte_order = not_implemented
        set_byte_order = not_implemented
        write_byte = not_implemented
        write_buffer_range = not_implemented
        write_boolean = not_implemented
        write_int16 = not_implemented
        write_int32 = not_implemented
        write_uint16 = not_implemented
        write_uint32 = not_implemented
        write_single = not_implemented
        write_date_time = not_implemented
        write_time_span = not_implemented
        store_async = not_implemented
        flush_async = not_implemented
        detach_stream = not_implemented

        def write_bytes(self, value):
            assert isinstance(value, list), "PassArray input is a projected list[int]"
            state["bytes"] = bytes(value)

        def write_buffer(self, value):
            assert isinstance(value, g.IBuffer)
            state["bytes"] = value.to_bytes()

        def write_string(self, value):
            state["text"] = value
            return len(value.encode("utf-8"))

        def measure_string(self, value):
            return len(value.encode("utf-8"))

        def write_guid(self, value):
            state["guid"] = value

        def write_int64(self, value):
            state["signed"] = value

        def write_uint64(self, value):
            state["unsigned"] = value

        def write_double(self, value):
            state["real"] = value

        def detach_buffer(self):
            return g.IBuffer.from_bytes(state["bytes"])

    class Text:
        def to_string(self):
            return "array object"

    object_owner = own(g.IStringable.implement(Text()))
    raw_object = object_owner.to_value()

    class Properties:
        invalid_element = False
        get_type = not_implemented
        get_is_numeric_scalar = not_implemented
        get_uint8 = not_implemented
        get_int16 = not_implemented
        get_uint16 = not_implemented
        get_int32 = not_implemented
        get_uint32 = not_implemented
        get_single = not_implemented
        get_char16 = not_implemented
        get_boolean = not_implemented
        get_date_time = not_implemented
        get_time_span = not_implemented
        get_point = not_implemented
        get_size = not_implemented
        get_rect = not_implemented
        get_int16_array = not_implemented
        get_uint16_array = not_implemented
        get_int32_array = not_implemented
        get_uint32_array = not_implemented
        get_single_array = not_implemented
        get_char16_array = not_implemented
        get_boolean_array = not_implemented
        get_date_time_array = not_implemented
        get_time_span_array = not_implemented
        get_size_array = not_implemented
        get_rect_array = not_implemented

        def get_uint8_array(self):
            return state["bytes"]

        def get_string(self):
            return state["text"]

        def get_string_array(self):
            return [state["text"], 42] if self.invalid_element else [state["text"], "", "snow \u96ea"]

        def get_guid(self):
            return state["guid"]

        def get_guid_array(self):
            return [state["guid"], INSTANCE_ID]

        def get_int64(self):
            return state["signed"]

        def get_int64_array(self):
            return [state["signed"], -(1 << 63)]

        def get_uint64(self):
            return state["unsigned"]

        def get_uint64_array(self):
            return [state["unsigned"], (1 << 64) - 1]

        def get_double(self):
            return state["real"]

        def get_double_array(self):
            return [state["real"], -2.5]

        def get_point_array(self):
            return [g.Point(x=1.25, y=-3.5), g.Point(x=0.0, y=2.0)]

        def get_inspectable_array(self):
            return [raw_object, None]

    writer_owner = own(g.IDataWriter.implement(Writer(), g.IClosable.implementation(CloseHandlers())))
    writer = g.IDataWriter.from_implementation(writer_owner)
    properties = Properties()
    owner = own(g.IPropertyValue.implement(properties))
    view = g.IPropertyValue.from_implementation(owner)
    writer_owner.release()
    owner.release()
    for data in (b"", bytes([0, 1, 127, 128, 255])):
        writer.write_bytes(data)
        assert view.get_uint8_array() == data
        native = writer.detach_buffer()
        assert isinstance(native, g.IBuffer)
        assert native.length == len(data)
        assert native.to_bytes() == data
        writer.write_buffer(native)
        assert view.get_uint8_array() == data
        dw.release_projected(native)
    text = "generated \0 WinRT \u96ea \U0001f680"
    assert writer.write_string(text) == writer.measure_string(text)
    assert view.get_string() == text
    assert view.get_string_array() == [text, "", "snow \u96ea"]
    writer.write_guid(INSTANCE_ID)
    assert view.get_guid() == INSTANCE_ID
    assert view.get_guid_array() == [INSTANCE_ID, INSTANCE_ID]
    writer.write_int64((1 << 63) - 1)
    writer.write_uint64((1 << 64) - 1)
    writer.write_double(1.25)
    assert view.get_int64() == (1 << 63) - 1
    assert view.get_uint64() == (1 << 64) - 1
    assert view.get_double() == 1.25
    assert view.get_int64_array() == [(1 << 63) - 1, -(1 << 63)]
    assert view.get_uint64_array() == [(1 << 64) - 1, (1 << 64) - 1]
    assert view.get_double_array() == [1.25, -2.5]
    points = view.get_point_array()
    assert [(point.x, point.y) for point in points] == [(1.25, -3.5), (0.0, 2.0)]
    objects = view.get_inspectable_array()
    assert len(objects) == 2 and objects[1] is None
    text_view = g.IStringable.from_value(objects[0])
    assert text_view.to_string() == "array object"
    dw.release_projected(text_view)
    objects[0].release()
    properties.invalid_element = True
    expect_hresult(view.get_string_array, PYTHON_CALLBACK_ERROR)
    take_error(owner, "get_string_array|invalid implementation result")
    properties.invalid_element = False
    assert view.get_string_array() == [text, "", "snow \u96ea"]
    assert owner.take_error() is None
    assert writer_owner.take_error() is None
    raw_object.release()
    dw.release_projected(view)
    dw.release_projected(writer)

def value_shapes(g, dw, own):
    """Every IPropertyValue scalar/array getter crosses a generated native thunk."""
    values = {
        "uint8": 255, "int16": -32768, "uint16": 65535,
        "int32": -(1 << 31), "uint32": (1 << 32) - 1,
        "int64": -(1 << 63), "uint64": (1 << 64) - 1,
        "single": 1.25, "double": -2.5, "char16": "\u96ea",
        "boolean": True, "string": "value\0\u96ea", "guid": INSTANCE_ID,
        "date_time": datetime(2026, 9, 9, tzinfo=timezone.utc),
        "time_span": timedelta(microseconds=-125),
        "point": g.Point(x=1.25, y=-3.5),
        "size": g.Size(width=3.5, height=2.25),
        "rect": g.Rect(x=1.25, y=2.5, width=3.75, height=4.0),
    }
    called = set()
    def getter(name, value):
        def invoke(self):
            called.add(name)
            return value
        return invoke
    methods = {
        "get_type": getter("type", g.PropertyType.Int32),
        "get_is_numeric_scalar": getter("numeric", True),
        "get_inspectable_array": getter("inspectable", [None]),
    }
    for name, value in values.items():
        methods["get_" + name] = getter(name, value)
        methods["get_" + name + "_array"] = getter(name + "[]", [value, value])
    handlers = type("ValueHandlers", (), methods)()
    with g.IPropertyValue.implement(handlers) as impl:
        assert impl.value.type == g.PropertyType.Int32
        assert impl.value.is_numeric_scalar
        assert impl.value.get_inspectable_array() == [None]
        for name, expected in values.items():
            actual = getattr(impl.value, "get_" + name)()
            array = getattr(impl.value, "get_" + name + "_array")()
            if name in ("point", "size", "rect"):
                fields = {"point": ("x", "y"), "size": ("width", "height"), "rect": ("x", "y", "width", "height")}[name]
                assert [getattr(actual, field) for field in fields] == [getattr(expected, field) for field in fields]
                assert [[getattr(item, field) for field in fields] for item in array] == [[getattr(expected, field) for field in fields]] * 2
            else:
                assert actual == expected
                assert array == (bytes([expected, expected]) if name == "uint8" else [expected, expected]), (name, array, expected)
    assert len(called) == 2 * len(values) + 3


def fill_array(g, dw, own):
    capacities = []

    def fill(capacity):
        assert isinstance(capacity, int), "FillArray input is U32 capacity, not an array"
        capacities.append(capacity)
        return bytes((index * 17 + 3) & 255 for index in range(capacity))

    owner = own(g.IDataReader.implement(
        ReaderHandlers(fill), g.IClosable.implementation(CloseHandlers())
    ))
    view = g.IDataReader.from_implementation(owner)
    for capacity in (0, 1, 7, 257):
        actual = view.read_bytes(bytearray(capacity))
        assert actual == bytes((index * 17 + 3) & 255 for index in range(capacity))
    assert capacities == [0, 1, 7, 257]
    error = expect_hresult(view.read_byte, PYTHON_CALLBACK_ERROR)
    assert "NotImplementedError" in str(error) and "Unused standalone" in str(error)
    take_error(owner, "unused standalone")


def fill_array_wrong_length(g, dw, own):
    wrong = True

    def fill(capacity):
        return bytes(capacity - int(wrong))

    owner = own(g.IDataReader.implement(
        ReaderHandlers(fill), g.IClosable.implementation(CloseHandlers())
    ))
    view = g.IDataReader.from_implementation(owner)
    expect_error(lambda: view.read_bytes(bytearray(4)), "length|capacity|invalid|80070057")
    take_error(owner, "capacity|length")
    wrong = False
    assert len(view.read_bytes(bytearray(4))) == 4
    assert owner.take_error() is None


def named_outputs(g, dw, own):
    class VectorView:
        named = True

        def get_at(self, index):
            assert index == 0
            return None

        def get_size(self):
            return 1

        def index_of(self, value):
            # Reverse dictionary insertion order: lowering must use metadata
            # names, with explicit out index first and the return appended.
            if self.named:
                return {"result": value is None, "index": 0 if value is None else 0xFFFFFFFF}
            return (0, True)

        def first(self):
            class Iterator:
                current = True

                def get_current(self):
                    return None

                def get_has_current(self):
                    return self.current

                def move_next(self):
                    self.current = False
                    return False

            iterator = own(g.IBindableIterator.implement(Iterator()))
            return g.IBindableIterator.from_implementation(iterator)

    handlers = VectorView()
    owner = own(g.IBindableVectorView.implement(
        handlers, g.IBindableIterable.implementation(handlers)
    ))
    view = g.IBindableVectorView.from_implementation(owner)
    assert view.size == 1
    assert view.get_at(0) is None
    # Outbound API retains its existing tuple shape. Only the new inbound
    # implementation contract requires named outputs.
    assert view.index_of(dw.DynWinRTValue.null_value()) == (0, True)

    class Text:
        def to_string(self):
            return "not in the vector"

    other = own(g.IStringable.implement(Text()))
    raw = other.to_value()
    try:
        assert view.index_of(raw) == (0xFFFFFFFF, False)
    finally:
        raw.release()
    iterator = g.IBindableIterable.from_implementation(owner).first()
    assert iterator.has_current
    assert iterator.current is None
    assert not iterator.move_next()
    assert not iterator.has_current
    assert owner.take_error() is None
    handlers.named = False
    expect_hresult(lambda: view.index_of(dw.DynWinRTValue.null_value()), PYTHON_CALLBACK_ERROR)
    take_error(owner, "dict|field")
    handlers.named = True
    assert view.index_of(dw.DynWinRTValue.null_value()) == (0, True)


def nullable_reference_results(g, dw, own):
    class Handlers:
        def parse_int(self, text):
            assert isinstance(text, str)
            if re.fullmatch(r"-?\d+", text) is None:
                return None
            value = int(text)
            return value if -(1 << 63) <= value < (1 << 63) else None

        def parse_u_int(self, text):
            assert isinstance(text, str)
            if re.fullmatch(r"\d+", text) is None:
                return None
            value = int(text)
            return value if value < (1 << 64) else None

        def parse_double(self, text):
            assert isinstance(text, str)
            try:
                return float(text)
            except ValueError:
                return None

    owner = own(g.INumberParser.implement(Handlers()))
    view = g.INumberParser.from_implementation(owner)
    owner.release()
    try:
        for value in (-(1 << 63), 0, (1 << 63) - 1):
            assert view.parse_int(str(value)) == value
        for value in (0, (1 << 64) - 1):
            assert view.parse_u_int(str(value)) == value
        assert view.parse_int("9223372036854775808") is None
        assert view.parse_u_int("-1") is None
        assert view.parse_u_int("18446744073709551616") is None
        assert view.parse_int("invalid") is None
        assert view.parse_double("2.5") == 2.5
        assert view.parse_double("-0.125") == -0.125
        assert view.parse_double("invalid") is None
        assert owner.take_error() is None
    finally:
        dw.release_projected(view)


CASES = {
    case.__name__: case for case in (
        management_handle, property_views, background_task, multi_interface_lifetime, dispose_disconnects,
        reentrant_dispose, callback_error, async_handler_rejected,
        async_result_rejected, required_interfaces, memory_buffer_event,
        array_contracts, value_shapes, fill_array, fill_array_wrong_length, named_outputs,
        nullable_reference_results,
        public_view_success_gc, public_view_failed_cast_gc,
    )
}

# Python alone exposes the native owner-only cycle to its GC visitor. These
# cases must not have a projected lifetime scope or a retained cleanup callback.
PUBLIC_VIEW_GC_CASES = frozenset({
    "public_view_success_gc", "public_view_failed_cast_gc",
})


def decode_child_result(case_id, stdout, stderr, exit_code):
    lines = [line for line in stdout.splitlines() if line.startswith(RESULT_PREFIX)]
    result = {
        "id": case_id, "pass": False,
        "error": f"Native child did not report a result (exit={exit_code})\n{stdout}",
    }
    if lines:
        try:
            payload = json.loads(lines[-1][len(RESULT_PREFIX):])
            if payload["id"] != case_id or not isinstance(payload["pass"], bool):
                raise ValueError("Invalid case result envelope")
            result = payload
        except (ValueError, KeyError, TypeError) as error:
            result["error"] = f"Invalid native child result: {error}"
    if exit_code != 0 and result["pass"]:
        result.update({"pass": False, "error": f"Native child failed during cleanup (exit={exit_code})"})
    return {**result, "exit_code": exit_code, "stderr": stderr}


def import_search_path(path: Path) -> str:
    absolute = str(path.resolve())
    if sys.platform != "win32" or absolute.startswith("\\\\?\\"):
        return absolute
    if absolute.startswith("\\\\"):
        return "\\\\?\\UNC\\" + absolute[2:]
    return "\\\\?\\" + absolute


def run_one(case_id, generated):
    started = time.monotonic()
    error = None
    try:
        import dynwinrt as dw

        assert hasattr(dw, "DynWinRTImplementation"), (
            "Build/install the Python runtime with WinRT implementation support"
        )
        # Generated generic facade names can otherwise exceed Windows
        # MAX_PATH in this worktree even though codegen emitted valid files.
        sys.path.insert(0, import_search_path(generated.parent))
        g = importlib.import_module(generated.name)
        with dw.RoApartment(1), ExitStack() as cleanup:
            if case_id not in PUBLIC_VIEW_GC_CASES:
                cleanup.enter_context(dw.projected_lifetime_scope())

            def own(owner):
                assert case_id not in PUBLIC_VIEW_GC_CASES, (
                    "GC cases must not register a strong owner cleanup root"
                )
                cleanup.callback(owner.dispose)
                return owner

            CASES[case_id](g, dw, own)
        gc.collect()
    except Exception:
        error = traceback.format_exc()
    return {
        "id": case_id, "pass": error is None, "error": error,
        "duration_ms": round((time.monotonic() - started) * 1000),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--generated", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--case", choices=CASES)
    parser.add_argument("--cases", help="Comma-separated native scenarios for an existing generated package")
    args = parser.parse_args()
    generated = args.generated.resolve()
    if args.case:
        result = run_one(args.case, generated)
        print(RESULT_PREFIX + json.dumps(result), flush=True)
        return 0 if result["pass"] else 1

    # platform.machine() alone describes the OS under Windows emulation. The
    # executable build tag identifies the actual interpreter architecture.
    architecture = f"{platform.machine()} / {struct.calcsize('P') * 8}-bit / {platform.python_compiler()}"
    print(f"Python {platform.python_version()}, {architecture}, {sys.executable}")
    results = []
    printed_errors = set()
    selected = args.cases.split(",") if args.cases else list(CASES)
    if any(case_id not in CASES for case_id in selected):
        raise ValueError("Unknown implementation scenario in --cases")
    for case_id in selected:
        command = [
            sys.executable, str(Path(__file__).resolve()),
            "--case", case_id, "--generated", str(generated),
        ]
        try:
            child = subprocess.run(
                command, text=True, capture_output=True, encoding="utf-8",
                errors="replace", timeout=90, check=False,
            )
            result = decode_child_result(case_id, child.stdout, child.stderr, child.returncode)
        except subprocess.TimeoutExpired as error:
            result = {
                "id": case_id, "pass": False, "error": f"Native child exceeded 90s: {error}",
                "exit_code": None, "stderr": str(error.stderr or ""),
            }
        results.append(result)
        print(f"  {'PASS' if result['pass'] else 'FAIL'} {case_id}", flush=True)
        if not result["pass"]:
            diagnostic = f"{result['error']}\n{result['stderr']}"
            lines = str(result["error"]).strip().splitlines()
            key = (lines[-1] if lines else "", result["stderr"])
            if key not in printed_errors:
                print(diagnostic, flush=True)
                printed_errors.add(key)
    report = {
        "language": "py-implementations", "architecture": architecture,
        "runtime": sys.version, "executable": sys.executable,
        "binding": getattr(importlib.util.find_spec("dynwinrt"), "origin", None),
        "generated": str(generated), "passed": sum(result["pass"] for result in results),
        "total": len(results), "results": results,
    }
    print(f"Python implementation E2E: {report['passed']}/{report['total']} passed ({architecture})")
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return 0 if report["passed"] == report["total"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
