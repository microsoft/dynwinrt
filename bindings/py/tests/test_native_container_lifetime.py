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
