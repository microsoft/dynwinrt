# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Collection-only comparison, including guarded native reference identity."""

from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from enum import IntEnum
import gc
import threading
from types import SimpleNamespace
from uuid import UUID
import weakref

import pytest

from dynwinrt import (
    DynWinRTArray,
    DynWinRTImplementation,
    DynWinRTImplementationMethod,
    DynWinRTInterfacePlan,
    DynWinRTMethodSig,
    DynWinRTType,
    DynWinRTValue,
    RoApartment,
    WinGUID,
)
from dynwinrt.dynwinrt import (
    _WinRTMappingMixin,
    _WinRTMutableMappingMixin,
    _WinRTMutableSequenceMixin,
    _WinRTSequenceMixin,
    _dynwinrt_collection_equal,
)
from dynwinrt.values import Point, Rect, Size


class SequenceProjection(_WinRTSequenceMixin):
    def __init__(self, items):
        self._items = list(items)

    @property
    def size(self):
        return len(self._items)

    def get_at(self, index):
        return self._items[index]


class MutableSequenceProjection(SequenceProjection, _WinRTMutableSequenceMixin):
    def remove_at(self, index):
        del self._items[index]


class MappingProjection(_WinRTMappingMixin):
    def __init__(self, items):
        self._items = dict(items)

    @property
    def size(self):
        return len(self._items)

    def has_key(self, key):
        return key in self._items

    def lookup(self, key):
        return self._items[key]

    def _iter_pairs(self):
        return (SimpleNamespace(key=key) for key in self._items)


class MutableMappingProjection(MappingProjection, _WinRTMutableMappingMixin):
    def insert(self, key, value):
        self._items[key] = value

    def remove(self, key):
        del self._items[key]


class Index:
    def __init__(self, value):
        self.value = value

    def __index__(self):
        return self.value


class Number(IntEnum):
    NEGATIVE = -1
    HIGH_BIT = 0x80000000


@dataclass
class Fields:
    number: int
    text: str


@pytest.mark.parametrize("projection", [SequenceProjection, MutableSequenceProjection])
@pytest.mark.parametrize(
    "value",
    [
        None, True, 17, Number.NEGATIVE, Number.HIGH_BIT, "text",
        UUID("01234567-89ab-cdef-0123-456789abcdef"),
        datetime(2026, 1, 1, tzinfo=timezone.utc), timedelta(seconds=3),
        Point(1, 2), Size(3, 4), Rect(1, 2, 3, 4), Fields(7, "field"),
    ],
)
def test_projected_values_keep_python_equality(projection, value):
    sequence = projection([value, value])
    assert value in sequence
    assert sequence.index(value) == 0
    assert sequence.count(value) == 2
    for mapping_type in (MappingProjection, MutableMappingProjection):
        mapping = mapping_type({"value": value})
        assert mapping == {"value": value} and {"value": value} == mapping
        assert value in mapping.values() and ("value", value) in mapping.items()
        assert mapping != [("value", value)]
        with pytest.raises(TypeError):
            hash(mapping)


@pytest.mark.parametrize("start", [-20, -3, -1, 0, 1, 4, 20, 2**80])
@pytest.mark.parametrize("stop", [None, -20, -1, 0, 2, 4, 20, 2**80])
@pytest.mark.parametrize("projection", [SequenceProjection, MutableSequenceProjection])
def test_index_bounds_and_first_match_agree_with_python_list(projection, start, stop):
    values = [1, 2, 1, 3]
    sequence = projection(values)
    try:
        expected = values.index(1, start) if stop is None else values.index(1, start, stop)
    except ValueError:
        with pytest.raises(ValueError):
            sequence.index(1, Index(start), None if stop is None else Index(stop))
    else:
        assert sequence.index(1, Index(start), None if stop is None else Index(stop)) == expected


def test_invalid_bounds_and_duplicate_removal():
    sequence = MutableSequenceProjection([1, 2, 1])
    for arguments in ((1, 0.5), (1, 0, 2.5), (1, None)):
        with pytest.raises(TypeError):
            sequence.index(*arguments)
    sequence.remove(1)
    assert list(sequence) == [2, 1] and sequence.index(1) == 1
    with pytest.raises(ValueError):
        sequence.remove(4)
    assert list(sequence) == [2, 1]


def test_nan_identity_and_reverse_equality_are_ordinary_python_comparisons():
    nan = float("nan")
    different_nan = float("nan")
    sequence = MutableSequenceProjection([nan])
    assert nan in sequence and sequence.count(nan) == 1
    assert different_nan not in sequence and sequence.count(different_nan) == 0
    assert MappingProjection({"nan": nan}) == {"nan": nan}
    assert MappingProjection({"nan": nan}) != {"nan": different_nan}

    class Reverse:
        def __eq__(self, other):
            return other == 17 or isinstance(other, MappingProjection)

    assert Reverse() in SequenceProjection([17])
    assert MappingProjection({"number": 17}) == {"number": Reverse()}
    assert MappingProjection({}) == Reverse()

    class NullKey:
        def __eq__(self, other):
            return other is None

        def __hash__(self):
            return hash(None)

    assert MappingProjection({None: 17}) == {NullKey(): 17}
    assert MappingProjection({nan: 17}) == {nan: 17}
    assert MappingProjection({nan: 17}) != {different_nan: 17}


def test_comparison_and_native_read_errors_are_not_hidden():
    class BadEquality:
        def __eq__(self, other):
            raise RuntimeError("comparison failed")

    class BadRead(SequenceProjection):
        def get_at(self, index):
            raise OSError("read failed")

    bad = BadEquality()
    for operation in (
        lambda: 1 in SequenceProjection([bad]),
        lambda: SequenceProjection([bad]).index(1),
        lambda: MutableSequenceProjection([bad]).remove(1),
        lambda: MappingProjection({"bad": bad}) == {"bad": 1},
        lambda: 1 in BadRead([1]),
        lambda: BadRead([1]).index(1),
    ):
        with pytest.raises((RuntimeError, OSError), match="failed"):
            operation()


def native_reference():
    """Full SDK IStringable and IClosable plans with distinct interface pointers."""
    plans = []
    for name, iid, method, signature in (
        (
            "Windows.Foundation.IStringable", "96369f54-8eb6-48f0-abce-c1b211e627c3",
            "ToString", DynWinRTMethodSig().add_out(DynWinRTType.hstring()),
        ),
        (
            "Windows.Foundation.IClosable", "30d5a829-7fa4-4026-83bb-d75bae4ea99e",
            "Close", DynWinRTMethodSig(),
        ),
    ):
        typ = DynWinRTType.register_interface(name, WinGUID.parse(iid)).add_method(
            method, signature
        )
        plans.append(DynWinRTInterfacePlan.create(
            name, typ, [DynWinRTImplementationMethod(method, 6, signature)]
        ))

    class Handler:
        calls = 0

        def dispatch(self, interface, slot, args):
            self.calls += 1
            return [DynWinRTValue.from_hstring("live")] if interface == 0 else []

    handler = Handler()
    owner = DynWinRTImplementation.create(plans, handler.dispatch)
    raw = owner.to_value()
    alias = raw.cast(WinGUID.parse("30d5a829-7fa4-4026-83bb-d75bae4ea99e"))
    return handler, owner, raw, alias


def test_canonical_identity_does_not_call_content_methods_or_consume_source():
    with RoApartment():
        handler, owner, raw, alias = native_reference()
        retained = weakref.ref(handler)
        assert raw.as_raw() != alias.as_raw()
        assert raw.identity_raw() == alias.identity_raw()
        projected = SimpleNamespace(_obj=raw)
        assert _dynwinrt_collection_equal(projected, alias)
        assert not _dynwinrt_collection_equal(raw, DynWinRTValue.null_value())
        assert _dynwinrt_collection_equal(None, DynWinRTValue.null_value())
        for projection in (SequenceProjection, MutableSequenceProjection):
            sequence = projection([alias, None, alias])
            assert projected in sequence and sequence.index(raw, 1) == 2
            assert sequence.count(projected) == 2
        for projection in (MappingProjection, MutableMappingProjection):
            mapping = projection({alias: raw})
            assert mapping == {raw: alias}
            assert projected in mapping.values() and (alias, projected) in mapping.items()
            for aliased_keys in (
                {raw: raw, alias: raw},
                {raw: None, alias: raw},
                {raw: raw, alias: None},
            ):
                assert len(mapping) == 1 and len(aliased_keys) == 2
                assert mapping != aliased_keys and aliased_keys != mapping
            same_size = projection({alias: raw, None: None})
            assert len(same_size) == len(aliased_keys)
            assert same_size != {raw: raw, alias: None}
        assert not raw.is_released() and not alias.is_released()
        assert handler.calls == 0
        alias.release()
        owner.release()
        assert raw.identity_raw() != 0
        raw.release()
        del handler, owner
        gc.collect()
        assert retained() is None


def test_released_native_values_never_compare_as_live_null_or_same_object():
    with RoApartment():
        handler, owner, raw, alias = native_reference()
        raw.release()
        for operation in (
            lambda: _dynwinrt_collection_equal(raw, raw),
            lambda: _dynwinrt_collection_equal(raw, None),
            lambda: raw in SequenceProjection([alias]),
            lambda: SequenceProjection([alias]).index(raw),
            lambda: MutableSequenceProjection([raw]).remove(alias),
            lambda: MappingProjection({"value": raw}) == {"value": alias},
            lambda: raw in MappingProjection({"value": alias}).values(),
            lambda: ("value", raw) in MappingProjection({"value": alias}).items(),
        ):
            with pytest.raises(RuntimeError, match="has been released"):
                operation()
        assert alias.identity_raw() != 0 and handler.calls == 0
        alias.release()
        owner.release()


def test_nonagile_identity_normalization_rejects_foreign_native_access():
    with RoApartment():
        handler, owner, raw, alias = native_reference()
        # Received carriers use the native agility policy, unlike implementation owners.
        received = DynWinRTArray.from_object_values([raw], DynWinRTType.object())
        guarded = received.get(0)
        guarded_alias = guarded.cast(
            WinGUID.parse("30d5a829-7fa4-4026-83bb-d75bae4ea99e")
        )
        errors = []

        def worker():
            try:
                with RoApartment():
                    for operation in (
                        lambda: _dynwinrt_collection_equal(guarded, guarded),
                        lambda: guarded in SequenceProjection([guarded_alias]),
                        lambda: MappingProjection({"value": guarded}) == {"value": guarded_alias},
                    ):
                        with pytest.raises(RuntimeError, match="owning COM apartment thread"):
                            operation()
            except BaseException as error:
                errors.append(error)

        thread = threading.Thread(target=worker)
        thread.start()
        thread.join(10)
        assert not thread.is_alive() and not errors, errors
        assert handler.calls == 0 and raw.identity_raw() == alias.identity_raw()
        guarded_alias.release()
        guarded.release()
        received.release()
        alias.release()
        raw.release()
        owner.release()


def test_agile_identity_normalization_preserves_foreign_access_and_source():
    with RoApartment():
        native = DynWinRTValue.activation_factory("Windows.Foundation.Uri")
        assert native._try_query_interface(
            WinGUID.parse("94ea2b94-e9cc-49e0-c0ff-ee64ca8f5b90")
        )
        alias = native.cast(WinGUID.parse("00000000-0000-0000-c000-000000000046"))
        errors = []

        def worker():
            try:
                with RoApartment():
                    assert _dynwinrt_collection_equal(native, alias)
                    assert native in SequenceProjection([alias])
                    assert MappingProjection({"value": native}) == {"value": alias}
            except BaseException as error:
                errors.append(error)

        thread = threading.Thread(target=worker)
        thread.start()
        thread.join(10)
        assert not thread.is_alive() and not errors, errors
        assert not native.is_released() and native.identity_raw() == alias.identity_raw()
