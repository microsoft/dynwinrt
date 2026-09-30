# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Native container owners must be disposed before their COM apartment exits."""

import gc
import subprocess
import sys
import weakref

import pytest

from dynwinrt import (
    DynWinRTArray,
    DynWinRTStruct,
    DynWinRTType,
    DynWinRTValue,
    RoApartment,
    WinGUID,
    projected_lifetime_scope,
)


_ESCAPING_CONTAINER = r"""
import gc
import sys
from dynwinrt import (
    DynWinRTArray, DynWinRTStruct, DynWinRTType, RoApartment,
    projected_lifetime_scope, to_winrt_object,
)

mode = sys.argv[1]
with RoApartment(), projected_lifetime_scope():
    boxed = to_winrt_object(8080)
    element = DynWinRTType.object()
    if mode in ('from_values', 'from_object_values', 'as_array'):
        constructor = (
            DynWinRTArray.from_values if mode == 'from_values'
            else DynWinRTArray.from_object_values
        )
        original = constructor([boxed], element)
        if mode == 'as_array':
            raw = original.to_value()
            escaped = raw.as_array()
            del original
        else:
            escaped = original
    elif mode == 'array_of_struct':
        shape = DynWinRTType.struct_type('Tests.ScopedStructElement', [element])
        field = DynWinRTStruct.create(shape)
        field.set_object(0, boxed)
        original = DynWinRTArray.from_values([field.to_value()], shape)
        escaped = original.to_value().as_array()
        del original, field
    elif mode == 'get_struct':
        inner_shape = DynWinRTType.struct_type('Tests.ScopedInner', [element])
        outer_shape = DynWinRTType.struct_type('Tests.ScopedOuter', [inner_shape])
        inner = DynWinRTStruct.create(inner_shape)
        inner.set_object(0, boxed)
        outer = DynWinRTStruct.create(outer_shape)
        outer.set_struct(0, inner)
        escaped = outer.get_struct(0)
        del inner, outer
    else:
        shape = DynWinRTType.struct_type('Tests.ScopedObject', [element])
        original = DynWinRTStruct.create(shape)
        original.set_object(0, boxed)
        if mode == 'as_struct':
            raw = original.to_value()
            escaped = raw.as_struct()
            del original
        else:
            escaped = original
assert boxed.is_released()
assert escaped.is_released()
if mode in ('from_values', 'from_object_values', 'as_array', 'array_of_struct'):
    operations = (lambda: len(escaped), lambda: escaped.get(0),
                  escaped.to_values, escaped.to_value, escaped.to_i32_list)
    name = 'DynWinRTArray'
else:
    operations = (lambda: escaped.get_object(0), lambda: escaped.set_object(0, boxed),
                  escaped.to_value)
    name = 'DynWinRTStruct'
for operation in operations:
    try:
        operation()
    except RuntimeError as error:
        assert f'{name}.release()' in str(error), error
    else:
        raise AssertionError(f'{name} accepted use after its native reference was released')
print('scope-exited', mode, flush=True)
del escaped
gc.collect()
print('clean-exit', mode, flush=True)
"""

_BALANCE_CONTAINER = r"""
import gc
import sys
from dynwinrt import (
    DynWinRTArray, DynWinRTStruct, DynWinRTType, DynWinRTValue,
    DynWinRTInterfacePlan, DynWinRTImplementationMethod, DynWinRTImplementation,
    DynWinRTMethodSig, RoApartment, WinGUID, projected_lifetime_scope,
)

mode = sys.argv[1]
signature = DynWinRTMethodSig().add_out(DynWinRTType.hstring())
iid = WinGUID.parse('96369f54-8eb6-48f0-abce-c1b211e627c3')
interface = DynWinRTType.register_interface(
    'Tests.IStringableContainerLifetime', iid,
).add_method('ToString', signature)
plan = DynWinRTInterfacePlan.create(
    'Tests.IStringableContainerLifetime',
    interface,
    [DynWinRTImplementationMethod('ToString', 6, signature)],
)
with RoApartment(), projected_lifetime_scope():
    owner = DynWinRTImplementation.create(
        [plan],
        lambda _interface, _slot, _args: [DynWinRTValue.from_hstring('alive')],
        'DynWinRT.Tests.ContainerOwner',
    )
    source = owner.to_value()
    if mode == 'array':
        container = DynWinRTArray.from_object_values([source], DynWinRTType.object())
        clone = container.to_value().as_array()
    elif mode == 'array_of_struct':
        shape = DynWinRTType.struct_type('Tests.ContainerOwnerRefElement', [DynWinRTType.object()])
        field = DynWinRTStruct.create(shape)
        field.set_object(0, source)
        container = DynWinRTArray.from_values([field.to_value()], shape)
        clone = container.to_value().as_array()
        del field
    elif mode == 'get_struct':
        inner_shape = DynWinRTType.struct_type('Tests.ContainerOwnerRefInner', [DynWinRTType.object()])
        outer_shape = DynWinRTType.struct_type('Tests.ContainerOwnerRefOuter', [inner_shape])
        inner = DynWinRTStruct.create(inner_shape)
        inner.set_object(0, source)
        container = DynWinRTStruct.create(outer_shape)
        container.set_struct(0, inner)
        clone = container.get_struct(0)
        del inner
    else:
        shape = DynWinRTType.struct_type('Tests.ContainerOwnerRef', [DynWinRTType.object()])
        container = DynWinRTStruct.create(shape)
        container.set_object(0, source)
        clone = container.to_value().as_struct()
    owner.release()
    assert not owner.is_closed
assert owner.is_closed, 'a container retained a native owner past its apartment'
assert container.is_released() and clone.is_released()
del clone, container
gc.collect()
print('balanced-references', mode, flush=True)
"""

_BORROWED_CALLBACK = r"""
from dynwinrt import (
    DynWinRTArray, DynWinRTType, DynWinRTValue, DynWinRTImplementation,
    DynWinRTImplementationMethod, DynWinRTInterfacePlan, DynWinRTMethodSig,
    RoApartment, WinGUID, projected_lifetime_scope,
)

signature = DynWinRTMethodSig().add_in(
    DynWinRTType.array_type(DynWinRTType.object())
)
iid = WinGUID.parse('13fd99ec-a997-4497-aabc-247345013f26')
interface = DynWinRTType.register_interface(
    'Tests.IBorrowedContainerCallback', iid,
).add_method('AcceptArray', signature)
plan = DynWinRTInterfacePlan.create(
    'Tests.IBorrowedContainerCallback', interface,
    [DynWinRTImplementationMethod('AcceptArray', 6, signature)],
)
borrowed = []
copies = []

def dispatch(index, slot, args):
    assert (index, slot) == (0, 6) and len(args) == 1
    borrowed.append(args[0])
    copies.append(args[0].as_array())
    assert len(copies[0]) == 1
    return []

with RoApartment():
    with projected_lifetime_scope():
        owner = DynWinRTImplementation.create(
            [plan], dispatch, 'DynWinRT.Tests.BorrowedArrayOwner'
        )
        source = DynWinRTValue.activation_factory('Windows.Foundation.Uri')
        array = DynWinRTArray.from_object_values([source], DynWinRTType.object())
        raw = array.to_value()
        receiver = owner.to_value().cast(iid)
        assert interface.method(6).invoke_all(receiver, [raw]) == []

    assert source.is_released() and array.is_released() and copies[0].is_released()
    assert not borrowed[0].is_released(), 'the scope consumed a borrowed callback parameter'
    live_copy = borrowed[0].as_array()
    assert len(live_copy) == 1
    live_copy.release()
    borrowed[0].release()
    owner.release()
    assert owner.is_closed
print('borrowed-callback-retained', flush=True)
"""

_REJECT_INVALID_ARRAY = r"""
import sys
from dynwinrt import (
    DynWinRTArray, DynWinRTType, DynWinRTValue, DynWinRTImplementation,
    DynWinRTImplementationMethod, DynWinRTInterfacePlan, DynWinRTMethodSig,
    RoApartment, WinGUID, projected_lifetime_scope,
)

mode = sys.argv[1]
signature = DynWinRTMethodSig().add_out(DynWinRTType.hstring())
iid = WinGUID.parse('96369f54-8eb6-48f0-abce-c1b211e627c3')
interface = DynWinRTType.register_interface(
    'Tests.IArrayContractOwner', iid,
).add_method('ToString', signature)
plan = DynWinRTInterfacePlan.create(
    'Tests.IArrayContractOwner', interface,
    [DynWinRTImplementationMethod('ToString', 6, signature)],
)
with RoApartment(), projected_lifetime_scope():
    owner = DynWinRTImplementation.create(
        [plan], lambda *_: [DynWinRTValue.from_hstring('alive')],
        'DynWinRT.Tests.ArrayContractOwner',
    )
    source = owner.to_value()
    identity = source.identity_raw()
    if mode in ('i32_object', 'i32_object_helper', 'i32_object_late'):
        constructor = (
            DynWinRTArray.from_values if mode != 'i32_object_helper'
            else DynWinRTArray.from_object_values
        )
        elements = (
            [DynWinRTValue.from_i32(17), source]
            if mode == 'i32_object_late' else [source]
        )
        declared = DynWinRTType.i32_type()
    elif mode == 'object_scalar_late':
        constructor = DynWinRTArray.from_values
        elements = [source, DynWinRTValue.from_i32(17)]
        declared = DynWinRTType.object()
    elif mode == 'wrong_iid':
        constructor = DynWinRTArray.from_object_values
        elements = [source]
        declared = DynWinRTType.interface(
            WinGUID.parse('905a0fe0-bc53-11df-8c49-001e4fc686da')
        )  # IBuffer is not implemented by the IStringable fixture.
    else:
        constructor = DynWinRTArray.from_values
        declared = (
            DynWinRTType.i32_type()
            if mode == 'nested_scalar'
            else DynWinRTType.array_type(DynWinRTType.object())
        )
        elements = []
        if mode in ('nested_object', 'nested_scalar'):
            inner = DynWinRTArray.from_object_values(
                [source], DynWinRTType.object()
            )
            elements = [inner.to_value()]
    try:
        constructor(elements, declared)
    except OSError as error:
        expected_hresult = -2147467262 if mode == 'wrong_iid' else -2147024809
        assert error.winerror == expected_hresult, error  # E_NOINTERFACE / E_INVALIDARG
        assert ('Array element 0' in str(error)
                or 'Array element 1' in str(error)
                or 'nested WinRT arrays' in str(error)), error
    else:
        raise AssertionError(f'{mode} accepted an unsupported array element contract')

    assert not source.is_released() and source.identity_raw() == identity
    if mode in ('nested_object', 'nested_scalar'):
        elements[0].release()
        inner.release()
    source.release()
    owner.release()
    assert owner.is_closed, f'{mode} retained a native reference on rejection'
print('array-contract-rejected-before-owning', mode, flush=True)
"""

_STOCK_URI_ARRAY = r"""
from dynwinrt import (
    DynWinRTArray, DynWinRTMethodSig, DynWinRTType, DynWinRTValue,
    RoApartment, WinGUID, projected_lifetime_scope,
)

factory_iid = WinGUID.parse('44a9796f-723e-4fdf-a218-033e75b0c084')
stringable_iid = WinGUID.parse('96369f54-8eb6-48f0-abce-c1b211e627c3')
factory_type = DynWinRTType.register_interface(
    'Tests.IUriRuntimeClassFactoryArrayBoundary', factory_iid,
).add_method(
    'CreateUri',
    DynWinRTMethodSig()
        .add_in(DynWinRTType.hstring())
        .add_out(DynWinRTType.object()),
)

with RoApartment(), projected_lifetime_scope():
    def exercise(url):
        factory = DynWinRTValue.activation_factory(
            'Windows.Foundation.Uri'
        ).cast(factory_iid)
        uri = factory_type.method(6).invoke(factory, [DynWinRTValue.from_hstring(url)])
        identity = uri.identity_raw()
        try:
            DynWinRTArray.from_values([uri], DynWinRTType.i32_type())
        except OSError as error:
            assert error.winerror == -2147024809 and 'Array element 0' in str(error)
        else:
            raise AssertionError('stock Uri pointer was stored in an I32 array')

        checked = DynWinRTArray.from_values(
            [uri, DynWinRTValue.null_value()],
            DynWinRTType.interface(stringable_iid),
        )
        typed = checked.get(0)
        assert typed.identity_raw() == identity
        assert typed.as_raw() != uri.as_raw(), 'typed element did not QueryInterface'
        assert checked.get(1).is_null()
        assert uri.identity_raw() == identity and not uri.is_released()
        return uri, checked

    first_uri, first_array = exercise('https://example.com/first')
    second_uri, second_array = exercise('https://example.com/second')
assert first_uri.is_released() and first_array.is_released()
assert second_uri.is_released() and second_array.is_released()
print('stock-uri-array-one-apartment', flush=True)
"""

_UNSCOPED_APARTMENT = r"""
import gc
import sys
import weakref
from dynwinrt import (
    DynWinRTArray, DynWinRTStruct, DynWinRTType, DynWinRTValue,
    RoApartment, ro_initialize, ro_uninitialize,
)

mode = sys.argv[1]
if mode == 'manual':
    ro_initialize(1)
with RoApartment(1):
    if mode == 'nested':
        with RoApartment(1):
            source = DynWinRTValue.activation_factory('Windows.Foundation.Uri')
        assert not source.is_released()
    else:
        source = DynWinRTValue.activation_factory('Windows.Foundation.Uri')
    object_type = DynWinRTType.object()
    array = DynWinRTArray.from_object_values([source], object_type)
    extracted = array.to_value().as_array()
    shape = DynWinRTType.struct_type('Tests.UnscopedApartment', [object_type])
    record = DynWinRTStruct.create(shape)
    record.set_object(0, source)
    nested = record.to_value().as_struct()
    scalars = DynWinRTArray.from_i32_values([7])

    transient = DynWinRTArray.from_object_values([source], object_type)
    observed = weakref.ref(transient)
    del transient
    gc.collect()
    assert observed() is None, 'implicit apartment registry rooted a temporary array'

if mode == 'manual':
    assert not source.is_released() and not array.is_released()
    ro_uninitialize()
assert source.is_released()
assert array.is_released() and extracted.is_released()
assert record.is_released() and nested.is_released()
assert scalars.to_i32_list() == [7]
for operation in (source.identity_raw, lambda: array.get(0), lambda: record.get_object(0)):
    try:
        operation()
    except RuntimeError as error:
        assert 'released' in str(error)
    else:
        raise AssertionError('unscoped COM owner remained callable outside apartment')
print('unscoped-owner-safe', mode, flush=True)
if mode != 'shutdown':
    del source, array, extracted, record, nested
    gc.collect()
    print('after-del', mode, flush=True)
"""

_EXTERNAL_RO_INITIALIZE = r"""
import ctypes
from dynwinrt import DynWinRTValue, RoApartment, WinGUID

runtimeobject = ctypes.WinDLL('combase.dll')
runtimeobject.RoInitialize.argtypes = (ctypes.c_int,)
runtimeobject.RoInitialize.restype = ctypes.c_long
runtimeobject.RoUninitialize.argtypes = ()
runtimeobject.RoUninitialize.restype = None
assert runtimeobject.RoInitialize(1) >= 0
try:
    external = DynWinRTValue.activation_factory('Windows.Foundation.Uri')
    with RoApartment(1):
        owned = external.cast(
            WinGUID.parse('44a9796f-723e-4fdf-a218-033e75b0c084')
        )
        assert not external.is_released() and not owned.is_released()
    assert owned.is_released() and not external.is_released()
    assert external.identity_raw() != 0
    assert runtimeobject.RoInitialize(1) == 1
    runtimeobject.RoUninitialize()
    external.release()
finally:
    runtimeobject.RoUninitialize()
print('external-host-initialization-preserved', flush=True)
"""

_INVERTED_PROJECTION_SCOPE = r"""
from dynwinrt import (
    DynWinRTArray, DynWinRTType, DynWinRTValue, RoApartment,
    projected_lifetime_scope,
)

with projected_lifetime_scope() as scope:
    with RoApartment(1):
        source = DynWinRTValue.activation_factory('Windows.Foundation.Uri')
        array = DynWinRTArray.from_object_values(
            [source], DynWinRTType.object()
        )
        assert not source.is_released() and not array.is_released()
    assert source.is_released() and array.is_released()
assert scope.disposed
source.release()
array.release()
print('inverted-scope-deduplicated', flush=True)
"""

_UNSCOPED_CLOSE_FAILURE = r"""
import gc
import sys
from dynwinrt import RoApartment, retry_pending_apartment_close
from dynwinrt.dynwinrt import _dynwinrt_track_native

class FailingOwner:
    def __init__(self):
        self.attempts = 0
    def release(self):
        self.attempts += 1
        if self.attempts == 1:
            raise RuntimeError('owner release failed')

mode = sys.argv[1]
try:
    with RoApartment(1):
        owner = FailingOwner()
        _dynwinrt_track_native(owner)
        if mode == 'body':
            raise ValueError('original body failure')
except ValueError as error:
    assert mode == 'body' and str(error) == 'original body failure'
    assert isinstance(error.__cause__, RuntimeError)
    assert str(error.__cause__) == 'owner release failed'
except RuntimeError as error:
    assert mode == 'cleanup' and str(error) == 'owner release failed'
else:
    raise AssertionError('final apartment close hid owner cleanup failure')
gc.collect()
assert owner.attempts == 1
retry_pending_apartment_close()
assert owner.attempts == 2
print('failed-apartment-close-retried', mode, flush=True)
"""

_REENTRANT_APARTMENT_CLOSE = r"""
from dynwinrt import DynWinRTValue, RoApartment
from dynwinrt.dynwinrt import _dynwinrt_track_native

created = []
class ReentrantOwner:
    def __init__(self):
        self.releases = 0
    def release(self):
        self.releases += 1
        if self.releases == 1:
            created.append(DynWinRTValue.activation_factory('Windows.Foundation.Uri'))

with RoApartment(1):
    owner = ReentrantOwner()
    _dynwinrt_track_native(owner)
assert owner.releases == 1
assert len(created) == 1 and created[0].is_released()
print('reentrant-apartment-owners-drained', flush=True)
"""

_UNBALANCED_MANAGED_APARTMENT = r"""
from dynwinrt import retry_pending_apartment_close, ro_uninitialize

for invalid in (ro_uninitialize, retry_pending_apartment_close):
    try:
        invalid()
    except RuntimeError as error:
        assert 'requires a successful' in str(error) or 'no failed RoApartment close' in str(error)
    else:
        raise AssertionError('unbalanced managed apartment call succeeded')
print('unbalanced-managed-apartment-rejected', flush=True)
"""

_WRONG_THREAD_APARTMENT = r"""
import threading
from dynwinrt import DynWinRTValue, RoApartment

with RoApartment(1) as apartment:
    source = DynWinRTValue.activation_factory('Windows.Foundation.Uri')
    errors = []
    def close_from_foreign_thread():
        try:
            apartment.close()
        except RuntimeError as error:
            errors.append(str(error))
    worker = threading.Thread(target=close_from_foreign_thread)
    worker.start()
    worker.join()
    assert len(errors) == 1 and 'initializing thread' in errors[0], errors
    assert not source.is_released()
assert source.is_released()
print('wrong-thread-close-retryable', flush=True)
"""

_FOREIGN_GUARD_FINALIZER = r"""
import gc
import threading
from dynwinrt import DynWinRTType, DynWinRTValue, RoApartment
from dynwinrt.dynwinrt import _managed_apartment_depth

apartment = RoApartment(1)
apartment.__enter__()
native = DynWinRTValue.activation_factory('Windows.Foundation.Uri')
handoff = [apartment]
del apartment
def finalizer_thread():
    handoff.clear()
    gc.collect()
    DynWinRTType.i32_type()
    gc.collect()
worker = threading.Thread(target=finalizer_thread)
worker.start()
worker.join(10)
assert not worker.is_alive()
assert _managed_apartment_depth() == 1, 'foreign Drop uninitialized the owner thread'
assert native.identity_raw() != 0
native.release()
print('foreign-guard-drop-retained-apartment', flush=True)
"""

_EXPLICIT_CALLBACK_SHUTDOWN = r"""
import sys
import threading
from dynwinrt import (
    DynWinRTImplementation, DynWinRTImplementationMethod, DynWinRTInterfacePlan,
    DynWinRTMethodSig, DynWinRTType, DynWinRTValue, DynWinRtDelegate,
    DynWinRtElementFactory, RoApartment, WinGUID, shutdown_python_callbacks,
)

mode = sys.argv[1]
calls = []
errors = []
started = threading.Event()
proceed = threading.Event()
stringable_iid = WinGUID.parse('96369f54-8eb6-48f0-abce-c1b211e627c3')
factory_iid = WinGUID.parse('75faba47-2cf2-54ae-91e6-0581556fddaa')
delegate_iid = WinGUID.parse('13fd99ec-a997-4497-aabc-247345013f26')
object_type = DynWinRTType.object()
delegate_sig = DynWinRTMethodSig().add_in(object_type).add_in(object_type)
signature = DynWinRTMethodSig().add_out(DynWinRTType.hstring())
string_type = DynWinRTType.register_interface(
    'Tests.IStringableShutdownGate', stringable_iid
).add_method('ToString', signature)
string_plan = DynWinRTInterfacePlan.create(
    'Tests.IStringableShutdownGate', string_type,
    [DynWinRTImplementationMethod('ToString', 6, signature)],
)
factory_type = DynWinRTType.register_interface(
    'Tests.IElementFactoryShutdownGate', factory_iid
).add_method(
    'GetElement', DynWinRTMethodSig().add_in(object_type).add_out(object_type)
).add_method('RecycleElement', DynWinRTMethodSig().add_in(object_type))

def recycle(_args):
    calls.append('factory')
    if mode == 'inflight':
        started.set()
        assert proceed.wait(8), 'host never settled its callback'

def delegate_callback(_first, _second):
    calls.append('delegate')

def implementation_callback(*_args):
    calls.append('implementation')
    if mode == 'inflight-implementation':
        try:
            shutdown_python_callbacks()
        except RuntimeError as error:
            assert 'callback(s) are in flight' in str(error), error
        else:
            raise AssertionError('host gate closed during an implementation callback')
    return [DynWinRTValue.from_hstring('alive')]

def invoke_delegate(value):
    return value.invoke_delegate(
        delegate_iid, delegate_sig,
        [DynWinRTValue.null_value(), DynWinRTValue.null_value()],
    )

with RoApartment(1):
    delegate = DynWinRtDelegate.create(
        delegate_iid, [object_type, object_type], delegate_callback
    )
    delegate_alias = delegate.to_value().cast(delegate_iid)
    delegate.release()
    factory = DynWinRtElementFactory.create(
        stringable_iid, lambda _args: None, recycle
    )
    factory_alias = factory.to_value().cast(factory_iid)
    factory._release_apartment_owner()
    implementation = DynWinRTImplementation.create(
        [string_plan], implementation_callback
    )
    implementation_alias = implementation.to_value().cast(stringable_iid)
    implementation.release()

    assert invoke_delegate(delegate_alias) == []
    assert string_type.method(6).invoke(implementation_alias, []).to_string() == 'alive'
    if mode == 'inflight':
        def worker():
            try:
                with RoApartment(1):
                    factory_type.method(7).invoke(factory_alias, [factory_alias])
            except BaseException as error:
                errors.append(error)
        thread = threading.Thread(target=worker)
        thread.start()
        assert started.wait(5), 'native callback did not start'
        try:
            shutdown_python_callbacks()
        except RuntimeError as error:
            assert 'callback(s) are in flight' in str(error)
        else:
            raise AssertionError('host gate closed while a callback was in flight')
        proceed.set()
        thread.join(10)
        assert not thread.is_alive() and not errors, errors
    else:
        factory_type.method(7).invoke(factory_alias, [factory_alias])
    assert calls == ['delegate', 'implementation', 'factory']
    if mode == 'gate-preflight':
        import dynwinrt.dynwinrt as native_module
        implementation_runtime = native_module._dynwinrt_implementation_runtime
        del native_module._dynwinrt_implementation_runtime
        try:
            try:
                shutdown_python_callbacks()
            except AttributeError as error:
                assert '_dynwinrt_implementation_runtime' in str(error), error
            else:
                raise AssertionError('missing implementation runtime closed the gate')
        finally:
            native_module._dynwinrt_implementation_runtime = implementation_runtime
        assert invoke_delegate(delegate_alias) == []
        assert calls.pop() == 'delegate'
        assert string_type.method(6).invoke(implementation_alias, []).to_string() == 'alive'
        assert calls.pop() == 'implementation'
    if mode == 'ordered':
        factory.release_callbacks()
        implementation.dispose()
        for alias in (delegate_alias, factory_alias, implementation_alias):
            alias.release()
    shutdown_python_callbacks()
    shutdown_python_callbacks()

    if mode != 'ordered':
        def expect_closed(call):
            try:
                call()
            except OSError as error:
                assert error.winerror == -2147483629, error
            else:
                raise AssertionError('late native callback reached Python after shutdown gate')
        expect_closed(lambda: invoke_delegate(delegate_alias))
        expect_closed(lambda: factory_type.method(7).invoke(factory_alias, [factory_alias]))
        expect_closed(lambda: string_type.method(6).invoke(implementation_alias, []))
        factory.release_callbacks()
        implementation.dispose()
        for alias in (delegate_alias, factory_alias, implementation_alias):
            alias.release()
    assert calls == ['delegate', 'implementation', 'factory']

    for constructor in (
        lambda: DynWinRtDelegate.create(delegate_iid, [object_type, object_type], delegate_callback),
        lambda: DynWinRtElementFactory.create(stringable_iid, lambda _args: None, recycle),
        lambda: DynWinRTImplementation.create([string_plan], implementation_callback),
    ):
        try:
            constructor()
        except RuntimeError as error:
            assert 'shut down' in str(error) or 'shutting down' in str(error), error
        else:
            raise AssertionError('Python-backed callback was created after shutdown gate')
    del delegate_alias, factory_alias, implementation_alias
    del delegate, factory, implementation
if mode == 'ordered':
    print('host-ordered-shutdown-safe', flush=True)
else:
    print('explicit-native-callback-gate-safe', mode, flush=True)
"""

_SPECIAL_UNSCOPED_OWNER = r"""
import sys
from dynwinrt import (
    DynWinRTImplementation, DynWinRTImplementationMethod, DynWinRTInterfacePlan,
    DynWinRTMethodSig, DynWinRTType, DynWinRTValue, DynWinRtDelegate,
    DynWinRtElementFactory, RoApartment, WinGUID,
)

mode = sys.argv[1]
early = sys.argv[2] == 'early'
stringable = WinGUID.parse('96369f54-8eb6-48f0-abce-c1b211e627c3')
element_factory = WinGUID.parse('75faba47-2cf2-54ae-91e6-0581556fddaa')

with RoApartment(1):
    if mode == 'delegate':
        owner = DynWinRtDelegate.create(stringable, [], lambda *args: None)
        alias = owner.to_value()
        identity = alias.identity_raw()
        if early:
            owner.release()
            assert owner.is_released() and alias.identity_raw() == identity
    elif mode == 'element_factory':
        calls = []
        typ = DynWinRTType.register_interface('Tests.IApartmentElementFactory', element_factory)
        typ = typ.add_method(
            'GetElement', DynWinRTMethodSig()
                .add_in(DynWinRTType.object()).add_out(DynWinRTType.object())
        ).add_method('RecycleElement', DynWinRTMethodSig().add_in(DynWinRTType.object()))
        owner = DynWinRtElementFactory.create(
            stringable, lambda _args: None, lambda _args: calls.append('recycled')
        )
        alias = owner.to_value().cast(element_factory)
        if early:
            owner._release_apartment_owner()
            typ.method(7).invoke(alias, [alias])
            assert calls == ['recycled'], 'owner release disconnected an independent alias'
    else:
        signature = DynWinRTMethodSig().add_out(DynWinRTType.hstring())
        typ = DynWinRTType.register_interface('Tests.IApartmentImplementation', stringable)
        typ = typ.add_method('ToString', signature)
        plan = DynWinRTInterfacePlan.create(
            'Tests.IApartmentImplementation', typ,
            [DynWinRTImplementationMethod('ToString', 6, signature)],
        )
        owner = DynWinRTImplementation.create(
            [plan], lambda *_: [DynWinRTValue.from_hstring('still alive')],
        )
        alias = owner.to_value().cast(stringable)
        if early:
            owner.release()
            assert typ.method(6).invoke(alias, []).to_string() == 'still alive'

    assert not alias.is_released()
assert alias.is_released()
if mode == 'delegate':
    assert owner.is_released()
else:
    try:
        owner.to_value()
    except (RuntimeError, OSError) as error:
        assert 'released' in str(error) or 'closed' in str(error)
    else:
        raise AssertionError('special owner kept an independent COM reference after apartment')
print('special-unscoped-owner', mode, 'early' if early else 'automatic', flush=True)
"""

_CROSS_THREAD_LOCAL_OWNER = r"""
import gc
import sys
import threading
import weakref
from dynwinrt import (
    DynWinRTImplementation, DynWinRTImplementationMethod, DynWinRTInterfacePlan,
    DynWinRTMethodSig, DynWinRTType, DynWinRTValue, DynWinRtDelegate,
    DynWinRtElementFactory, RoApartment, WinGUID,
)

mode = sys.argv[1]
owner_thread = threading.get_ident()
dropped = []
errors = []
stringable = WinGUID.parse('96369f54-8eb6-48f0-abce-c1b211e627c3')

class Handler:
    def __call__(self, *_args):
        return None
    def get(self, _args):
        return None
    def recycle(self, _args):
        return None
    def dispatch(self, *_args):
        return [DynWinRTValue.from_hstring('alive')]
    def __del__(self):
        dropped.append(threading.get_ident())

with RoApartment(1):
    handler = Handler()
    retained = weakref.ref(handler)
    if mode == 'delegate':
        owner = DynWinRtDelegate.create(stringable, [], handler)
        alias = owner.to_value()
        owner.release()
    elif mode == 'element_factory':
        owner = DynWinRtElementFactory.create(
            stringable, handler.get, handler.recycle
        )
        alias = owner.to_value()
        owner._release_apartment_owner()
    else:
        signature = DynWinRTMethodSig().add_out(DynWinRTType.hstring())
        typ = DynWinRTType.register_interface(
            'Tests.ICrossThreadOwner', stringable
        ).add_method('ToString', signature)
        plan = DynWinRTInterfacePlan.create(
            'Tests.ICrossThreadOwner', typ,
            [DynWinRTImplementationMethod('ToString', 6, signature)],
        )
        owner = DynWinRTImplementation.create([plan], handler.dispatch)
        alias = owner.to_value()
        owner.release()
    del owner, handler
    assert retained() is not None
    handoff = [alias]
    del alias

    def drop_on_worker():
        try:
            handoff.clear()
            gc.collect()
            DynWinRTType.i32_type()
            gc.collect()
            assert retained() is None, 'an agile/local COM owner leaked on foreign Python Drop'
        except BaseException as error:
            errors.append(error)

    worker = threading.Thread(target=drop_on_worker)
    worker.start()
    worker.join(10)
    assert not worker.is_alive() and not errors, errors
assert len(dropped) == 1 and dropped[0] != owner_thread
print('local-owner-foreign-drop-balanced', mode, flush=True)
"""

_AGILE_CONTAINER_FOREIGN_DROP = r"""
import gc
import sys
import threading
import weakref
from dynwinrt import (
    DynWinRTArray, DynWinRTStruct, DynWinRTType, DynWinRtElementFactory,
    RoApartment, WinGUID,
)

mode = sys.argv[1]
disposed = []
errors = []

class Handler:
    def get(self, _args):
        return None
    def recycle(self, _args):
        return None
    def __del__(self):
        disposed.append(threading.get_ident())

with RoApartment(1):
    handler = Handler()
    weak = weakref.ref(handler)
    owner = DynWinRtElementFactory.create(
        WinGUID.parse('96369f54-8eb6-48f0-abce-c1b211e627c3'),
        handler.get, handler.recycle,
    )
    source = owner.to_value()
    if mode == 'array':
        container = DynWinRTArray.from_object_values(
            [source], DynWinRTType.object()
        )
    elif mode == 'nested':
        inner_type = DynWinRTType.struct_type(
            'Tests.AgileInner', [DynWinRTType.object()]
        )
        outer_type = DynWinRTType.struct_type('Tests.AgileOuter', [inner_type])
        inner = DynWinRTStruct.create(inner_type)
        inner.set_object(0, source)
        container = DynWinRTStruct.create(outer_type)
        container.set_struct(0, inner)
        del inner
    else:
        shape = DynWinRTType.struct_type(
            'Tests.AgileContainerField', [DynWinRTType.object()]
        )
        container = DynWinRTStruct.create(shape)
        container.set_object(0, source)
    owner._release_apartment_owner()
    source.release()
    del owner, source, handler
    assert weak() is not None
    handoff = [container]
    del container

    def destroy_on_worker():
        try:
            handoff.clear()
            gc.collect()
            DynWinRTType.i32_type()
            gc.collect()
            assert weak() is None, 'agile COM container leaked after foreign Drop'
        except BaseException as error:
            errors.append(error)

    worker = threading.Thread(target=destroy_on_worker)
    worker.start()
    worker.join(10)
    assert not worker.is_alive() and not errors, errors
assert len(disposed) == 1
print('agile-container-foreign-drop-balanced', mode, flush=True)
"""

_NONAGILE_CONTAINER_FOREIGN_DROP = r"""
import gc
import sys
import threading
import weakref
from dynwinrt import (
    DynWinRTArray, DynWinRTImplementation, DynWinRTImplementationMethod,
    DynWinRTInterfacePlan, DynWinRTMethodSig, DynWinRTStruct, DynWinRTType,
    DynWinRTValue, RoApartment, WinGUID,
)

mode = sys.argv[1]
errors = []

class Handler:
    def dispatch(self, *_args):
        return [DynWinRTValue.from_hstring('owner thread')]

iid = WinGUID.parse('96369f54-8eb6-48f0-abce-c1b211e627c3')
signature = DynWinRTMethodSig().add_out(DynWinRTType.hstring())
interface = DynWinRTType.register_interface('Tests.INonAgileForeignContainer', iid)
interface = interface.add_method('ToString', signature)
plan = DynWinRTInterfacePlan.create(
    'Tests.INonAgileForeignContainer', interface,
    [DynWinRTImplementationMethod('ToString', 6, signature)],
)
with RoApartment(1):
    handler = Handler()
    retained = weakref.ref(handler)
    owner = DynWinRTImplementation.create([plan], handler.dispatch)
    source = owner.to_value()
    if mode == 'array':
        container = DynWinRTArray.from_object_values(
            [source], DynWinRTType.object()
        )
    elif mode == 'nested':
        inner_type = DynWinRTType.struct_type(
            'Tests.NonAgileForeignInner', [DynWinRTType.object()]
        )
        outer_type = DynWinRTType.struct_type(
            'Tests.NonAgileForeignOuter', [inner_type]
        )
        inner = DynWinRTStruct.create(inner_type)
        inner.set_object(0, source)
        container = DynWinRTStruct.create(outer_type)
        container.set_struct(0, inner)
        del inner
    else:
        shape = DynWinRTType.struct_type(
            'Tests.NonAgileForeignField', [DynWinRTType.object()]
        )
        container = DynWinRTStruct.create(shape)
        container.set_object(0, source)
    owner.release()
    source.release()
    del source, handler, owner
    assert retained() is not None
    handoff = [container]
    del container

    def release_on_worker():
        try:
            native = handoff[0]
            try:
                if mode == 'array':
                    native.get(0)
                elif mode == 'nested':
                    native.get_struct(0)
                else:
                    native.get_object(0)
            except RuntimeError as error:
                assert 'owning COM apartment thread' in str(error)
            else:
                raise AssertionError('non-agile COM field was callable on a foreign thread')
            del native
            handoff.clear()
            gc.collect()
            DynWinRTType.i32_type()
            gc.collect()
            assert retained() is not None, 'non-agile COM ref was released off-thread'
        except BaseException as error:
            errors.append(error)

    worker = threading.Thread(target=release_on_worker)
    worker.start()
    worker.join(10)
    assert not worker.is_alive() and not errors, errors
print('nonagile-container-foreign-drop-quarantined', mode, flush=True)
"""

_FOREIGN_NONAGILE_STRUCT_MUTATION = r"""
import threading
from dynwinrt import (
    DynWinRTImplementation, DynWinRTImplementationMethod, DynWinRTInterfacePlan,
    DynWinRTMethodSig, DynWinRTStruct, DynWinRTType, DynWinRTValue,
    RoApartment, WinGUID,
)

object_type = DynWinRTType.object()
inner_type = DynWinRTType.struct_type('Tests.ForeignInnerField', [object_type])
outer_type = DynWinRTType.struct_type('Tests.ForeignOuterField', [inner_type])
stringable = WinGUID.parse('96369f54-8eb6-48f0-abce-c1b211e627c3')
signature = DynWinRTMethodSig().add_out(DynWinRTType.hstring())
interface = DynWinRTType.register_interface('Tests.IForeignStructOwner', stringable)
interface = interface.add_method('ToString', signature)
plan = DynWinRTInterfacePlan.create(
    'Tests.IForeignStructOwner', interface,
    [DynWinRTImplementationMethod('ToString', 6, signature)],
)

errors = []
with RoApartment(1):
    direct = DynWinRTStruct.create(inner_type)
    nested = DynWinRTStruct.create(outer_type)
    def worker():
        try:
            with RoApartment(1):
                owner = DynWinRTImplementation.create(
                    [plan], lambda *_: [DynWinRTValue.from_hstring('alive')]
                )
                source = owner.to_value()
                source_record = DynWinRTStruct.create(inner_type)
                source_record.set_object(0, source)
                for attempt in (
                    lambda: direct.set_object(0, source),
                    lambda: nested.set_struct(0, source_record),
                ):
                    try:
                        attempt()
                    except RuntimeError as error:
                        assert 'non-agile' in str(error), error
                    else:
                        raise AssertionError('cross-apartment non-agile COM field was stored')
        except BaseException as error:
            errors.append(error)
    thread = threading.Thread(target=worker)
    thread.start()
    thread.join(10)
    assert not thread.is_alive() and not errors, errors
    assert direct.get_object(0).is_null()
    assert nested.get_struct(0).get_object(0).is_null()
assert direct.is_released() and nested.is_released()
print('foreign-nonagile-struct-mutation-rejected', flush=True)
"""

_FAILED_AGILE_STRUCT_SETTER = r"""
import gc
import sys
import threading
import weakref
from dynwinrt import (
    DynWinRTStruct, DynWinRTType, DynWinRtElementFactory, RoApartment, WinGUID,
)

mode = sys.argv[1]
disposed = []
errors = []

class Handler:
    def get(self, _args):
        return None
    def recycle(self, _args):
        return None
    def __del__(self):
        disposed.append(threading.get_ident())

with RoApartment(1):
    handler = Handler()
    retained = weakref.ref(handler)
    owner = DynWinRtElementFactory.create(
        WinGUID.parse('96369f54-8eb6-48f0-abce-c1b211e627c3'),
        handler.get, handler.recycle,
    )
    source = owner.to_value()
    inner_type = DynWinRTType.struct_type(
        'Tests.FailedAgileInner', [DynWinRTType.object()]
    )
    if mode == 'object':
        record = DynWinRTStruct.create(inner_type)
        record.set_object(0, source)
        def identity():
            return record.get_object(0).identity_raw()
        def invalid_setter():
            record.set_object(100, source)
    else:
        inner = DynWinRTStruct.create(inner_type)
        inner.set_object(0, source)
        outer_type = DynWinRTType.struct_type(
            'Tests.FailedAgileOuter', [inner_type]
        )
        record = DynWinRTStruct.create(outer_type)
        record.set_struct(0, inner)
        def identity():
            return record.get_struct(0).get_object(0).identity_raw()
        def invalid_setter():
            record.set_struct(100, inner)

    original = identity()
    try:
        invalid_setter()
    except IndexError:
        pass
    else:
        raise AssertionError('invalid field index was accepted')
    assert identity() == original == source.identity_raw()
    if mode == 'struct':
        inner.release()
    owner._release_apartment_owner()
    source.release()
    del owner, source, handler
    assert retained() is not None

    def release_foreign():
        try:
            with RoApartment(1):
                assert identity() == original
                record.release()
                assert record.is_released()
                gc.collect()
                assert retained() is None, 'agile struct leaked its own COM reference'
        except BaseException as error:
            errors.append(error)

    worker = threading.Thread(target=release_foreign)
    worker.start()
    worker.join(10)
    assert not worker.is_alive() and not errors, errors
assert len(disposed) == 1
print('failed-agile-struct-setter-balanced', mode, flush=True)
"""

_VALID_NONAGILE_STRUCT_SETTER = r"""
import gc
import sys
import threading
import weakref
from dynwinrt import (
    DynWinRTImplementation, DynWinRTImplementationMethod, DynWinRTInterfacePlan,
    DynWinRTMethodSig, DynWinRTStruct, DynWinRTType, DynWinRTValue,
    RoApartment, WinGUID,
)

mode = sys.argv[1]
disposed = []
errors = []

class Handler:
    def dispatch(self, *_args):
        return [DynWinRTValue.from_hstring('non-agile')]
    def __del__(self):
        disposed.append(threading.get_ident())

with RoApartment(1):
    handler = Handler()
    retained = weakref.ref(handler)
    iid = WinGUID.parse('96369f54-8eb6-48f0-abce-c1b211e627c3')
    signature = DynWinRTMethodSig().add_out(DynWinRTType.hstring())
    interface = DynWinRTType.register_interface(
        'Tests.IValidNonAgileSetter', iid
    ).add_method('ToString', signature)
    plan = DynWinRTInterfacePlan.create(
        'Tests.IValidNonAgileSetter', interface,
        [DynWinRTImplementationMethod('ToString', 6, signature)],
    )
    owner = DynWinRTImplementation.create([plan], handler.dispatch)
    source = owner.to_value()
    inner_type = DynWinRTType.struct_type(
        'Tests.ValidNonAgileInner', [DynWinRTType.object()]
    )
    if mode == 'object':
        record = DynWinRTStruct.create(inner_type)
        record.set_object(0, source)
        def identity():
            return record.get_object(0).identity_raw()
    else:
        inner = DynWinRTStruct.create(inner_type)
        inner.set_object(0, source)
        outer_type = DynWinRTType.struct_type(
            'Tests.ValidNonAgileOuter', [inner_type]
        )
        record = DynWinRTStruct.create(outer_type)
        record.set_struct(0, inner)
        def identity():
            return record.get_struct(0).get_object(0).identity_raw()

    original = identity()
    assert original == source.identity_raw()
    def release_foreign():
        try:
            with RoApartment(1):
                try:
                    record.release()
                except RuntimeError as error:
                    assert 'owning COM apartment thread' in str(error), error
                else:
                    raise AssertionError('non-agile struct released on foreign thread')
                assert not record.is_released()
        except BaseException as error:
            errors.append(error)

    worker = threading.Thread(target=release_foreign)
    worker.start()
    worker.join(10)
    assert not worker.is_alive() and not errors, errors
    assert identity() == original
    record.release()
    if mode == 'struct':
        inner.release()
    source.release()
    owner.release()
    del handler, source, owner
    gc.collect()
    assert retained() is None, 'non-agile owner was not released on its apartment'
assert len(disposed) == 1
print('valid-nonagile-struct-setter-guarded', mode, flush=True)
"""

_UNSCOPED_ASYNC_OWNER = r"""
from pathlib import Path
from tempfile import TemporaryDirectory
from dynwinrt import (
    DynWinRTMethodSig, DynWinRTType, DynWinRTValue, RoApartment, WinGUID,
)
from dynwinrt.dynwinrt import _DynWinRTAsync

with TemporaryDirectory() as folder:
    path = Path(folder) / 'owner.txt'
    path.write_text('alive')
    static_iid = WinGUID.parse('5984c710-daf2-43c8-8bb4-a4d3eacfd03f')
    file_iid = WinGUID.parse('fa3f6186-4214-428c-a64c-14c9ac7315ea')
    file_type = DynWinRTType.runtime_class(
        'Windows.Storage.StorageFile', DynWinRTType.interface(file_iid)
    )
    statics = DynWinRTType.register_interface(
        'Tests.IStorageFileStaticsLifetime', static_iid
    ).add_method(
        'GetFileFromPathAsync',
        DynWinRTMethodSig()
            .add_in(DynWinRTType.hstring())
            .add_out(DynWinRTType.i_async_operation(file_type)),
    )
    with RoApartment(1):
        factory = DynWinRTValue.activation_factory(
            'Windows.Storage.StorageFile'
        ).cast(static_iid)
        raw = statics.method(6).invoke(
            factory, [DynWinRTValue.from_hstring(str(path))]
        )
        operation = _DynWinRTAsync(raw, lambda value: value)
        result = operation.wait()
        assert result.identity_raw() != 0
    assert raw.is_released() and factory.is_released() and result.is_released()
    try:
        operation.wait()
    except RuntimeError as error:
        assert 'released' in str(error)
    else:
        raise AssertionError('async owner retained COM past apartment exit')
print('unscoped-async-owner-released', flush=True)
"""

_NONAGILE_CALLBACK_COPY = r"""
from dynwinrt import (
    DynWinRTImplementation, DynWinRTImplementationMethod, DynWinRTInterfacePlan,
    DynWinRTMethodSig, DynWinRTType, DynWinRTValue, DynWinRtElementFactory,
    RoApartment, WinGUID, projected_lifetime_scope,
)

stringable = WinGUID.parse('96369f54-8eb6-48f0-abce-c1b211e627c3')
element_factory = WinGUID.parse('75faba47-2cf2-54ae-91e6-0581556fddaa')
signature = DynWinRTMethodSig().add_out(DynWinRTType.hstring())
interface = DynWinRTType.register_interface('Tests.INonAgileCallbackSource', stringable)
interface = interface.add_method('ToString', signature)
plan = DynWinRTInterfacePlan.create(
    'Tests.INonAgileCallbackSource', interface,
    [DynWinRTImplementationMethod('ToString', 6, signature)],
)
factory_type = DynWinRTType.register_interface(
    'Tests.IElementFactoryCallbackLifetime', element_factory
)
factory_type = factory_type.add_method(
    'GetElement', DynWinRTMethodSig()
        .add_in(DynWinRTType.object()).add_out(DynWinRTType.object())
).add_method('RecycleElement', DynWinRTMethodSig().add_in(DynWinRTType.object()))
retained = []
with RoApartment(1):
    with projected_lifetime_scope():
        owner = DynWinRTImplementation.create(
            [plan], lambda *_: [DynWinRTValue.from_hstring('alive')]
        )
        source = owner.to_value()
        factory = DynWinRtElementFactory.create(
            stringable, lambda _args: None, retained.append
        )
        receiver = factory.to_value().cast(element_factory)
        factory_type.method(7).invoke(receiver, [source])
        assert len(retained) == 1
    assert not retained[0].is_released(), 'callback clone inherited an explicit scope'
    assert retained[0].identity_raw() != 0
assert retained[0].is_released(), 'non-agile callback clone escaped its native apartment'
print('nonagile-callback-copy-released', flush=True)
"""

_UNSCOPED_RECEIVED_ARRAY = r"""
import sys
from dynwinrt import (
    DynWinRTArray, DynWinRTImplementation, DynWinRTImplementationMethod,
    DynWinRTInterfacePlan, DynWinRTMethodSig, DynWinRTStruct, DynWinRTType, DynWinRTValue,
    RoApartment, WinGUID,
)

mode = sys.argv[1]
iid = WinGUID.parse('38684d40-bab3-42de-998d-26e4cce87c51')
element = DynWinRTType.object()
if mode == 'nested':
    inner_type = DynWinRTType.struct_type('Tests.ReceivedInner', [element])
    element_type = DynWinRTType.struct_type('Tests.ReceivedOuter', [inner_type])
else:
    element_type = element
signature = DynWinRTMethodSig().add_out(DynWinRTType.array_type(element_type))
interface = DynWinRTType.register_interface('Tests.IReceivedOwnerArray', iid)
interface = interface.add_method('GetItems', signature)
plan = DynWinRTInterfacePlan.create(
    'Tests.IReceivedOwnerArray', interface,
    [DynWinRTImplementationMethod('GetItems', 6, signature)],
)

with RoApartment(1):
    source = DynWinRTValue.activation_factory('Windows.Foundation.Uri')
    def produce():
        if mode == 'nested':
            inner = DynWinRTStruct.create(inner_type)
            inner.set_object(0, source)
            outer = DynWinRTStruct.create(element_type)
            outer.set_struct(0, inner)
            values = [outer.to_value()]
        else:
            values = [source]
        return [DynWinRTArray.from_values(values, element_type).to_value()]
    owner = DynWinRTImplementation.create(
        [plan],
        lambda *_: produce(),
    )
    receiver = owner.to_value().cast(iid)
    raw = interface.method(6).invoke(receiver, [])
    extracted = raw.as_array()
    clone = extracted.to_value().as_array()
    assert len(extracted) == len(clone) == 1
    if mode == 'nested':
        nested = extracted.get(0).as_struct().get_struct(0)
        assert nested.get_object(0).identity_raw() == source.identity_raw()
    else:
        assert extracted.get(0).identity_raw() == source.identity_raw()
assert raw.is_released() and extracted.is_released() and clone.is_released()
assert receiver.is_released() and source.is_released()
if mode == 'nested':
    assert nested.is_released()
assert owner.is_closed
print('unscoped-received-com-array-safe', mode, flush=True)
"""

_FOREIGN_RECEIVED_COM_ARRAY = r"""
import gc
import sys
import threading
import weakref
from dynwinrt import (
    DynWinRTArray, DynWinRTImplementation, DynWinRTImplementationMethod,
    DynWinRTInterfacePlan, DynWinRTMethodSig, DynWinRTStruct, DynWinRTType,
    DynWinRTValue, DynWinRtElementFactory, RoApartment, WinGUID,
)

agility, shape = sys.argv[1:]
errors = []
stringable = WinGUID.parse('96369f54-8eb6-48f0-abce-c1b211e627c3')
received_iid = WinGUID.parse('38684d40-bab3-42de-998d-26e4cce87c51')

class Handler:
    def get(self, _args):
        return None
    def recycle(self, _args):
        return None
    def dispatch(self, *_args):
        return [DynWinRTValue.from_hstring('alive')]

with RoApartment(1):
    handler = Handler()
    retained = weakref.ref(handler)
    if agility == 'agile':
        source_owner = DynWinRtElementFactory.create(
            stringable, handler.get, handler.recycle
        )
    else:
        signature = DynWinRTMethodSig().add_out(DynWinRTType.hstring())
        string_type = DynWinRTType.register_interface(
            'Tests.INonAgileReceivedSource', stringable
        ).add_method('ToString', signature)
        string_plan = DynWinRTInterfacePlan.create(
            'Tests.INonAgileReceivedSource', string_type,
            [DynWinRTImplementationMethod('ToString', 6, signature)],
        )
        source_owner = DynWinRTImplementation.create(
            [string_plan], handler.dispatch
        )
    source = source_owner.to_value()
    object_type = DynWinRTType.object()
    if shape == 'nested':
        inner_type = DynWinRTType.struct_type(
            'Tests.ForeignReceivedInner', [object_type]
        )
        element = DynWinRTType.struct_type(
            'Tests.ForeignReceivedOuter', [inner_type]
        )
    else:
        element = object_type

    def produce():
        if shape == 'nested':
            inner = DynWinRTStruct.create(inner_type)
            inner.set_object(0, source)
            outer = DynWinRTStruct.create(element)
            outer.set_struct(0, inner)
            values = [outer.to_value()]
        else:
            values = [source]
        return [DynWinRTArray.from_values(values, element).to_value()]

    array_sig = DynWinRTMethodSig().add_out(DynWinRTType.array_type(element))
    receiver_type = DynWinRTType.register_interface(
        'Tests.IForeignReceivedArray', received_iid
    ).add_method('GetItems', array_sig)
    receive_plan = DynWinRTInterfacePlan.create(
        'Tests.IForeignReceivedArray', receiver_type,
        [DynWinRTImplementationMethod('GetItems', 6, array_sig)],
    )
    receive_owner = DynWinRTImplementation.create(
        [receive_plan], lambda *_: produce()
    )
    receiver = receive_owner.to_value().cast(received_iid)
    raw = receiver_type.method(6).invoke(receiver, [])
    extracted = raw.as_array()
    assert len(extracted) == 1
    raw.release()
    receiver.release()
    receive_owner.release()
    source.release()
    if agility == 'agile':
        source_owner._release_apartment_owner()
    else:
        source_owner.release()
    del handler, source_owner, receive_owner
    assert retained() is not None
    handoff = [extracted]
    del extracted

    def release_on_worker():
        try:
            array = handoff[0]
            if agility == 'agile':
                item = array.get(0)
                if shape == 'nested':
                    item = item.as_struct().get_struct(0).get_object(0)
                assert item.identity_raw() != 0
                del item
            else:
                try:
                    array.get(0)
                except RuntimeError as error:
                    assert 'owning COM apartment thread' in str(error)
                else:
                    raise AssertionError('non-agile CoTaskMem array read on foreign thread')
            del array
            handoff.clear()
            gc.collect()
            DynWinRTType.i32_type()
            gc.collect()
            assert (retained() is None) == (agility == 'agile')
        except BaseException as error:
            errors.append(error)

    worker = threading.Thread(target=release_on_worker)
    worker.start()
    worker.join(10)
    assert not worker.is_alive() and not errors, errors
print('foreign-received-com-array', agility, shape, flush=True)
"""


@pytest.mark.parametrize(
    "mode",
    [
        "from_values",
        "from_object_values",
        "as_array",
        "array_of_struct",
        "create_struct",
        "as_struct",
        "get_struct",
    ],
)
def test_escaping_com_container_drops_after_apartment_exit(mode):
    result = subprocess.run(
        [sys.executable, "-B", "-c", _ESCAPING_CONTAINER, mode],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        mode,
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert f"clean-exit {mode}" in result.stdout


@pytest.mark.parametrize("mode", ["array", "array_of_struct", "struct", "get_struct"])
def test_scope_balances_native_implementation_container_references(mode):
    result = subprocess.run(
        [sys.executable, "-B", "-c", _BALANCE_CONTAINER, mode],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        mode,
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert f"balanced-references {mode}" in result.stdout


@pytest.mark.parametrize(
    "mode",
    [
        "i32_object",
        "i32_object_helper",
        "i32_object_late",
        "object_scalar_late",
        "wrong_iid",
        "nested_object",
        "nested_scalar",
        "nested_empty",
    ],
)
def test_invalid_array_contract_fails_before_retaining_native_references(mode):
    result = subprocess.run(
        [sys.executable, "-B", "-c", _REJECT_INVALID_ARRAY, mode],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        mode,
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert f"array-contract-rejected-before-owning {mode}" in result.stdout


def test_stock_uri_checked_arrays_repeat_within_one_apartment():
    result = subprocess.run(
        [sys.executable, "-B", "-c", _STOCK_URI_ARRAY],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert "stock-uri-array-one-apartment" in result.stdout


@pytest.mark.parametrize("mode", ["raw", "nested", "manual", "shutdown"])
def test_unscoped_native_carriers_release_before_last_managed_apartment_exit(mode):
    result = subprocess.run(
        [sys.executable, "-B", "-c", _UNSCOPED_APARTMENT, mode],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        mode,
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert f"unscoped-owner-safe {mode}" in result.stdout


def test_final_managed_exit_does_not_consume_an_external_com_initialization():
    result = subprocess.run(
        [sys.executable, "-B", "-c", _EXTERNAL_RO_INITIALIZE],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert "external-host-initialization-preserved" in result.stdout


def test_earlier_managed_exit_deduplicates_an_outer_explicit_scope():
    result = subprocess.run(
        [sys.executable, "-B", "-c", _INVERTED_PROJECTION_SCOPE],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert "inverted-scope-deduplicated" in result.stdout


@pytest.mark.parametrize("mode", ["cleanup", "body"])
def test_failed_unnamed_apartment_close_preserves_owner_for_same_thread_retry(mode):
    result = subprocess.run(
        [sys.executable, "-B", "-c", _UNSCOPED_CLOSE_FAILURE, mode],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert f"failed-apartment-close-retried {mode}" in result.stdout


@pytest.mark.parametrize(
    ("script", "marker"),
    [
        (_REENTRANT_APARTMENT_CLOSE, "reentrant-apartment-owners-drained"),
        (_UNBALANCED_MANAGED_APARTMENT, "unbalanced-managed-apartment-rejected"),
    ],
)
def test_managed_apartment_drain_is_reentrant_and_unbalanced_calls_fail(script, marker):
    result = subprocess.run(
        [sys.executable, "-B", "-c", script],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert marker in result.stdout


def test_wrong_thread_apartment_close_preserves_native_owners():
    result = subprocess.run(
        [sys.executable, "-B", "-c", _WRONG_THREAD_APARTMENT],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert "wrong-thread-close-retryable" in result.stdout


def test_implicit_foreign_guard_drop_never_uninitializes_the_owner_thread():
    result = subprocess.run(
        [sys.executable, "-B", "-c", _FOREIGN_GUARD_FINALIZER],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert "foreign-guard-drop-retained-apartment" in result.stdout


@pytest.mark.parametrize(
    "mode", [
        "retained-aliases", "inflight", "inflight-implementation",
        "gate-preflight", "ordered",
    ]
)
def test_explicit_callback_gate_rejects_late_native_invocations(mode):
    result = subprocess.run(
        [sys.executable, "-B", "-c", _EXPLICIT_CALLBACK_SHUTDOWN, mode],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        mode,
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    marker = (
        "host-ordered-shutdown-safe"
        if mode == "ordered"
        else f"explicit-native-callback-gate-safe {mode}"
    )
    assert marker in result.stdout


@pytest.mark.parametrize("mode", ["delegate", "element_factory", "implementation"])
@pytest.mark.parametrize("early", [False, True], ids=["automatic", "early"])
def test_special_owners_release_only_their_own_native_reference(mode, early):
    result = subprocess.run(
        [
            sys.executable, "-B", "-c", _SPECIAL_UNSCOPED_OWNER,
            mode, "early" if early else "automatic",
        ],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        mode,
        early,
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert f"special-unscoped-owner {mode}" in result.stdout


@pytest.mark.parametrize("mode", ["delegate", "element_factory", "implementation"])
def test_local_com_owners_dropped_on_foreign_thread_do_not_leak_callbacks(mode):
    result = subprocess.run(
        [sys.executable, "-B", "-c", _CROSS_THREAD_LOCAL_OWNER, mode],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        mode,
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert f"local-owner-foreign-drop-balanced {mode}" in result.stdout


@pytest.mark.parametrize("mode", ["array", "struct", "nested"])
def test_agile_com_containers_dropped_on_foreign_thread_release_own_references(mode):
    result = subprocess.run(
        [sys.executable, "-B", "-c", _AGILE_CONTAINER_FOREIGN_DROP, mode],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        mode,
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert f"agile-container-foreign-drop-balanced {mode}" in result.stdout


@pytest.mark.parametrize("mode", ["array", "struct", "nested"])
def test_nonagile_com_containers_drop_without_off_thread_native_release(mode):
    result = subprocess.run(
        [sys.executable, "-B", "-c", _NONAGILE_CONTAINER_FOREIGN_DROP, mode],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        mode,
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert f"nonagile-container-foreign-drop-quarantined {mode}" in result.stdout


def test_nonagile_struct_fields_reject_cross_apartment_mutation_before_owning():
    result = subprocess.run(
        [sys.executable, "-B", "-c", _FOREIGN_NONAGILE_STRUCT_MUTATION],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert "foreign-nonagile-struct-mutation-rejected" in result.stdout


@pytest.mark.parametrize("mode", ["object", "struct"])
def test_failed_agile_struct_setters_preserve_foreign_release_and_refs(mode):
    result = subprocess.run(
        [sys.executable, "-B", "-c", _FAILED_AGILE_STRUCT_SETTER, mode],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        mode,
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert f"failed-agile-struct-setter-balanced {mode}" in result.stdout


@pytest.mark.parametrize("mode", ["object", "struct"])
def test_valid_nonagile_struct_setters_reject_foreign_release(mode):
    result = subprocess.run(
        [sys.executable, "-B", "-c", _VALID_NONAGILE_STRUCT_SETTER, mode],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        mode,
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert f"valid-nonagile-struct-setter-guarded {mode}" in result.stdout


def test_completed_async_owner_drops_its_reference_without_implicit_cancel():
    result = subprocess.run(
        [sys.executable, "-B", "-c", _UNSCOPED_ASYNC_OWNER],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert "unscoped-async-owner-released" in result.stdout


def test_nonagile_callback_clone_outlives_scope_but_not_its_apartment():
    result = subprocess.run(
        [sys.executable, "-B", "-c", _NONAGILE_CALLBACK_COPY],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert "nonagile-callback-copy-released" in result.stdout


@pytest.mark.parametrize("mode", ["object", "nested"])
def test_unscoped_received_com_array_and_clones_release_before_apartment_exit(mode):
    result = subprocess.run(
        [sys.executable, "-B", "-c", _UNSCOPED_RECEIVED_ARRAY, mode],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert f"unscoped-received-com-array-safe {mode}" in result.stdout


def test_checked_array_contracts_keep_valid_null_scalars_and_struct_owners():
    with RoApartment(), projected_lifetime_scope():
        source = DynWinRTValue.activation_factory("Windows.Foundation.Uri")
        identity = source.identity_raw()
        object_type = DynWinRTType.object()

        objects = DynWinRTArray.from_object_values(
            [source, DynWinRTValue.null_value()], object_type
        )
        assert objects.get(0).identity_raw() == identity
        assert objects.get(1).is_null()
        assert DynWinRTArray.from_values([DynWinRTValue.null_value()], object_type).get(0).is_null()

        signed = DynWinRTType.enum_type("Tests.CheckedArrayEnum", ["One"], [1])
        assert DynWinRTArray.from_values([DynWinRTValue.from_i32(1)], signed).get(0).to_int() == 1
        assert DynWinRTArray.from_values(
            [DynWinRTValue.from_u16(ord("x"))], DynWinRTType.char16()
        ).get(0).to_int() == ord("x")
        assert DynWinRTArray.from_values(
            [DynWinRTValue.from_i32(8080)], DynWinRTType.i32_type()
        ).to_i32_list() == [8080]
        assert DynWinRTArray.from_values(
            [DynWinRTValue.from_i32(-1)], DynWinRTType.hresult()
        ).to_i32_list() == [-1]

        shape = DynWinRTType.struct_type("Tests.CheckedArrayStruct", [object_type])
        record = DynWinRTStruct.create(shape)
        record.set_object(0, source)
        structured = DynWinRTArray.from_values([record.to_value()], shape)
        assert structured.get(0).as_struct().get_object(0).identity_raw() == identity

        structured.release()
        objects.release()
        record.release()
        assert not source.is_released() and source.identity_raw() == identity


@pytest.mark.parametrize("agility", ["agile", "nonagile"])
@pytest.mark.parametrize("shape", ["object", "nested"])
def test_received_cotaskmem_com_array_foreign_drop_preserves_native_contract(agility, shape):
    result = subprocess.run(
        [sys.executable, "-B", "-c", _FOREIGN_RECEIVED_COM_ARRAY, agility, shape],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        agility,
        shape,
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert f"foreign-received-com-array {agility} {shape}" in result.stdout


def test_borrowed_callback_array_survives_scope_within_its_apartment():
    result = subprocess.run(
        [sys.executable, "-B", "-c", _BORROWED_CALLBACK],
        capture_output=True,
        text=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 0, (
        hex(result.returncode & 0xFFFFFFFF),
        result.stdout,
        result.stderr,
    )
    assert "borrowed-callback-retained" in result.stdout


def test_scalar_containers_remain_usable_after_scope_exit():
    with RoApartment(), projected_lifetime_scope():
        numbers = DynWinRTArray.from_i32_values([7, 11])
        copy = numbers.to_value().as_array()
        shape = DynWinRTType.struct_type("Tests.ScopedScalar", [DynWinRTType.i32_type()])
        record = DynWinRTStruct.create(shape)
        record.set_i32(0, 8080)
        record_copy = record.to_value().as_struct()

    for container in (numbers, copy, record, record_copy):
        assert not container.is_released()
    assert numbers.to_i32_list() == copy.to_i32_list() == [7, 11]
    assert record.get_i32(0) == record_copy.get_i32(0) == 8080


def test_explicit_release_of_com_containers_is_idempotent_and_keeps_source_live():
    with RoApartment():
        boxed = DynWinRTValue.activation_factory("Windows.Foundation.Uri")
        identity = boxed.identity_raw()
        array = DynWinRTArray.from_object_values([boxed], DynWinRTType.object())
        shape = DynWinRTType.struct_type("Tests.ExplicitObject", [DynWinRTType.object()])
        record = DynWinRTStruct.create(shape)
        record.set_object(0, boxed)
        assert not array.is_released() and not record.is_released()

        for container in (array, record):
            container.release()
            container.release()
            assert container.is_released()
        assert not boxed.is_released() and boxed.identity_raw() == identity
        with pytest.raises(RuntimeError, match="DynWinRTArray.release"):
            array.to_value()
        with pytest.raises(RuntimeError, match="DynWinRTStruct.release"):
            record.get_object(0)
        boxed.release()


def test_scope_does_not_root_temporary_com_containers():
    with RoApartment(), projected_lifetime_scope() as scope:
        boxed = DynWinRTValue.activation_factory("Windows.Foundation.Uri")
        array = DynWinRTArray.from_object_values([boxed], DynWinRTType.object())
        array_id = id(array)
        array_ref = weakref.ref(array)
        assert scope.track_native(array) is array
        assert array_id in scope._native_refs
        del array
        gc.collect()
        assert array_ref() is None and array_id not in scope._native_refs

        shape = DynWinRTType.struct_type("Tests.TemporaryObject", [DynWinRTType.object()])
        record = DynWinRTStruct.create(shape)
        record.set_object(0, boxed)
        record_id = id(record)
        record_ref = weakref.ref(record)
        assert record_id in scope._native_refs
        del record
        gc.collect()
        assert record_ref() is None and record_id not in scope._native_refs

    assert boxed.is_released()


def test_released_array_rejects_every_read_and_conversion():
    array = DynWinRTArray.from_i32_values([7])
    array.release()
    array.release()
    assert array.is_released()
    for operation in (
        lambda: len(array),
        lambda: array.get(0),
        array.to_values,
        array.to_i8_list,
        array.to_u8_list,
        array.to_i16_list,
        array.to_u16_list,
        array.to_i32_list,
        array.to_u32_list,
        array.to_f32_list,
        array.to_f64_list,
        array.to_i64_list,
        array.to_u64_list,
        array.to_string_list,
        array.to_bytes,
        array.to_value,
    ):
        with pytest.raises(RuntimeError, match=r"DynWinRTArray\.release\(\)"):
            operation()


def test_released_struct_rejects_every_field_operation():
    shape = DynWinRTType.struct_type("Tests.ReleasedFields", [DynWinRTType.i32_type()])
    record = DynWinRTStruct.create(shape)
    replacement = DynWinRTStruct.create(shape)
    record.release()
    record.release()
    assert record.is_released()
    for operation in (
        lambda: record.get_i8(0),
        lambda: record.set_i8(0, 1),
        lambda: record.get_u8(0),
        lambda: record.set_u8(0, 1),
        lambda: record.get_i16(0),
        lambda: record.set_i16(0, 1),
        lambda: record.get_u16(0),
        lambda: record.set_u16(0, 1),
        lambda: record.get_i32(0),
        lambda: record.set_i32(0, 1),
        lambda: record.get_u32(0),
        lambda: record.set_u32(0, 1),
        lambda: record.get_i64(0),
        lambda: record.set_i64(0, 1),
        lambda: record.get_u64(0),
        lambda: record.set_u64(0, 1),
        lambda: record.get_f32(0),
        lambda: record.set_f32(0, 1.0),
        lambda: record.get_f64(0),
        lambda: record.set_f64(0, 1.0),
        lambda: record.get_hstring(0),
        lambda: record.set_hstring(0, "released"),
        lambda: record.get_guid(0),
        lambda: record.set_guid(0, WinGUID.parse("00000000-0000-0000-0000-000000000000")),
        lambda: record.get_object(0),
        lambda: record.set_object(0, DynWinRTValue.null_value()),
        lambda: record.get_struct(0),
        lambda: record.set_struct(0, replacement),
        record.to_value,
    ):
        with pytest.raises(RuntimeError, match=r"DynWinRTStruct\.release\(\)"):
            operation()
