# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Opt-in value views of ``Object``-valued WinRT maps: ``object_value_view``.

The maps are real WinRT collections (PropertySet, ValueSet, StringMap,
MediaPropertySet and ``create_map`` maps) behind wrappers built like the
generated ones: the runtime mapping mixins over the collection interface,
registered with the method names and vtable order codegen uses.
"""

import gc
from collections.abc import Mapping, MutableMapping
from contextlib import contextmanager
from datetime import datetime, timedelta, timezone
from enum import IntEnum
from types import SimpleNamespace
from uuid import UUID

import pytest

import dynwinrt
from dynwinrt import (
    DynWinRTImplementation,
    DynWinRTImplementationMethod,
    DynWinRTInterfacePlan,
    DynWinRTMethodSig,
    DynWinRTStruct,
    DynWinRTType,
    DynWinRTValue,
    RoApartment,
    WinGUID,
    release_projected,
    to_winrt_object,
    unbox_object,
)
from dynwinrt import values
from dynwinrt.dynwinrt import _WinRTMappingMixin, _WinRTMutableMappingMixin
from dynwinrt.values import (
    MutableObjectValueView,
    ObjectValueView,
    PropertyType,
    object_value_view,
)

T = DynWinRTType
OBJECT = T.object()
IMAP = WinGUID.parse("3c2925fe-8519-45c1-aa79-197b6718c1c1")
IMAP_VIEW = WinGUID.parse("e480ce40-a338-4ada-adcf-272272e48cb9")
IITERABLE = WinGUID.parse("faa585ea-6214-4217-afda-7f46de5869b3")
IKEY_VALUE_PAIR = WinGUID.parse("02b51929-c1c4-4a7e-8940-0312b5c18500")
IPROPERTY_VALUE = WinGUID.parse("4bd682dd-7554-40e9-9a9b-82654ede7e62")
IPROPERTY_VALUE_STATICS = WinGUID.parse("629bdbc8-d932-4ff4-96b9-8d96c5c1e858")
IURI_FACTORY = WinGUID.parse("44a9796f-723e-4fdf-a218-033e75b0c084")
E_NOTIMPL = -2147467263
UTC = timezone.utc
RELEASED_REASON = (
    "has been released (its projected_lifetime_scope() exited, or "
    "release_projected() / DynWinRTValue.release() was called) and can no longer "
    "be used."
)


@pytest.fixture(scope="module", autouse=True)
def apartment():
    with RoApartment(1):
        yield
        # Release the WinRT objects that exception tracebacks keep in
        # reference cycles while the apartment is still initialized.
        gc.collect()


# ----------------------------------------------------------------------
# Generated-style map wrappers
# ----------------------------------------------------------------------


class Keys:
    """How a generated wrapper marshals one WinRT key type."""

    def __init__(self, typ, to_native, from_native):
        self.type = typ
        self.to_native = to_native
        self.from_native = from_native


STRING_KEYS = Keys(T.hstring(), DynWinRTValue.from_hstring, lambda value: value.to_string())
GUID_KEYS = Keys(
    T.guid_type(),
    lambda key: DynWinRTValue.from_guid(WinGUID.parse(str(key))),
    lambda value: UUID(value.to_guid().to_string()),
)
INT32_KEYS = Keys(T.i32_type(), DynWinRTValue.from_i32, lambda value: value.to_number())
IMAP_STRING_OBJECT = T.parameterized(IMAP, [STRING_KEYS.type, OBJECT]).iid()
IMAP_VIEW_STRING_OBJECT = T.parameterized(IMAP_VIEW, [STRING_KEYS.type, OBJECT]).iid()
IMAP_GUID_OBJECT = T.parameterized(IMAP, [GUID_KEYS.type, OBJECT]).iid()
IMAP_VIEW_GUID_OBJECT = T.parameterized(IMAP_VIEW, [GUID_KEYS.type, OBJECT]).iid()
IMAP_STRING_STRING = T.parameterized(IMAP, [STRING_KEYS.type, T.hstring()]).iid()
IMAP_VIEW_STRING_STRING = T.parameterized(IMAP_VIEW, [STRING_KEYS.type, T.hstring()]).iid()
IMAP_INT32_OBJECT = T.parameterized(IMAP, [INT32_KEYS.type, OBJECT]).iid()
JSON_VALUE = T.interface(WinGUID.parse("a3219ecb-f0b3-4dcd-beee-19d48cd3ed1e"))
IMAP_STRING_JSON = T.parameterized(IMAP, [STRING_KEYS.type, JSON_VALUE]).iid()


def register(generic, keys, value_type, methods):
    """Register ``generic<K, V>`` with codegen's method names, in vtable order."""
    iid = T.parameterized(generic, [keys.type, value_type]).iid()
    interface = T.register_interface(f"GeneratedStyle{iid.to_string()}", iid)
    for name, inputs, output in methods:
        signature = DynWinRTMethodSig()
        for typ in inputs:
            signature = signature.add_in(typ)
        if output is not None:
            signature = signature.add_out(output)
        interface = interface.add_method(name, signature)
    return iid, interface


class GeneratedStyle:
    GENERIC = IMAP_VIEW

    def __init__(self, native, keys=STRING_KEYS, value_type=OBJECT):
        self._keys = keys
        self._value_type = value_type
        iid, self._interface = register(
            self.GENERIC, keys, value_type, self.methods(keys.type, value_type)
        )
        self._obj = native.cast(iid)

    @staticmethod
    def methods(key, value):
        return [
            ("Lookup", [key], value),
            ("get_Size", [], T.u32_type()),
            ("HasKey", [key], T.bool_type()),
        ]

    @property
    def _map_obj(self):
        return self._obj

    @property
    def size(self):
        return self._interface.method(7).invoke(self._map_obj, []).to_u32()

    def lookup(self, key):
        value = self._interface.method(6).invoke(
            self._map_obj, [self._keys.to_native(key)]
        )
        return None if value.is_null() else value

    def has_key(self, key):
        return self._interface.method(8).invoke(
            self._map_obj, [self._keys.to_native(key)]
        ).to_bool()

    def _iter_pairs(self):
        pair = T.parameterized(IKEY_VALUE_PAIR, [self._keys.type, self._value_type])
        iterable = self._map_obj.cast(T.parameterized(IITERABLE, [pair]).iid())
        iterator = iterable.call_0(6, OBJECT)  # First
        while iterator.call_0(7, T.bool_type()).to_bool():  # get_HasCurrent
            current = iterator.call_0(6, OBJECT)  # get_Current
            key = current.call_0(6, self._keys.type)  # get_Key
            yield SimpleNamespace(key=self._keys.from_native(key))
            iterator.call_0(8, T.bool_type())  # MoveNext


class GeneratedStyleMapView(GeneratedStyle, _WinRTMappingMixin):
    """Built like a generated ``IMapView<K, V>`` wrapper."""

    _dynwinrt_interface_type = True
    _dynwinrt_interface_iid = IMAP_VIEW_STRING_OBJECT
    _dynwinrt_map_dispatch = (IMAP_VIEW_STRING_OBJECT, "_obj")


class GeneratedStyleMap(GeneratedStyle, _WinRTMutableMappingMixin):
    """Built like a generated ``IMap<K, V>`` wrapper."""

    GENERIC = IMAP
    VIEW_TYPE = GeneratedStyleMapView
    _dynwinrt_interface_type = True
    _dynwinrt_interface_iid = IMAP_STRING_OBJECT
    _dynwinrt_map_dispatch = (IMAP_STRING_OBJECT, "_obj")

    @staticmethod
    def methods(key, value):
        return GeneratedStyle.methods(key, value) + [
            ("GetView", [], T.parameterized(IMAP_VIEW, [key, value])),
            ("Insert", [key, value], T.bool_type()),
            ("Remove", [key], None),
            ("Clear", [], None),
        ]

    def get_view(self):
        native = self._interface.method(9).invoke(self._map_obj, [])
        return self.VIEW_TYPE(native)

    def insert(self, key, value):
        native = getattr(value, "_obj", value)
        return self._interface.method(10).invoke(
            self._map_obj, [self._keys.to_native(key), native]
        ).to_bool()

    def remove(self, key):
        self._interface.method(11).invoke(self._map_obj, [self._keys.to_native(key)])

    def clear(self):
        self._interface.method(12).invoke(self._map_obj, [])


class GeneratedStyleRuntimeMap(GeneratedStyleMap):
    """A runtime-class wrapper whose collection interface is non-default."""

    _dynwinrt_interface_type = False
    _dynwinrt_runtime_class_type = True
    _dynwinrt_map_dispatch = (IMAP_STRING_OBJECT, "_collection_obj")

    def __init__(self, native, keys=STRING_KEYS, value_type=OBJECT):
        super().__init__(native, keys, value_type)
        self._collection_obj = self._obj
        # Stand in for a different default interface on the same identity.
        self._obj = native.cast(
            T.parameterized(IMAP, [keys.type, T.hstring()]).iid()
        )

    @property
    def _map_obj(self):
        return self._collection_obj


class GeneratedStyleDefaultRuntimeMap(GeneratedStyleMap):
    """A runtime-class wrapper whose collection interface is its default."""

    _dynwinrt_interface_type = False
    _dynwinrt_runtime_class_type = True
    _dynwinrt_map_dispatch = (IMAP_STRING_OBJECT, "_obj")


class GeneratedStyleStringMapView(GeneratedStyleMapView):
    _dynwinrt_interface_type = True
    _dynwinrt_interface_iid = IMAP_VIEW_STRING_STRING
    _dynwinrt_map_dispatch = (IMAP_VIEW_STRING_STRING, "_obj")

    def __init__(self, native):
        super().__init__(native, value_type=T.hstring())


class GeneratedStyleStringMap(GeneratedStyleMap):
    VIEW_TYPE = GeneratedStyleStringMapView
    _dynwinrt_interface_type = True
    _dynwinrt_interface_iid = IMAP_STRING_STRING
    _dynwinrt_map_dispatch = (IMAP_STRING_STRING, "_obj")

    def __init__(self, native):
        super().__init__(native, value_type=T.hstring())


class GeneratedStyleGuidObjectMapView(GeneratedStyleMapView):
    _dynwinrt_interface_type = True
    _dynwinrt_interface_iid = IMAP_VIEW_GUID_OBJECT
    _dynwinrt_map_dispatch = (IMAP_VIEW_GUID_OBJECT, "_obj")

    def __init__(self, native):
        super().__init__(native, keys=GUID_KEYS)


class GeneratedStyleGuidObjectMap(GeneratedStyleMap):
    VIEW_TYPE = GeneratedStyleGuidObjectMapView
    _dynwinrt_interface_type = True
    _dynwinrt_interface_iid = IMAP_GUID_OBJECT
    _dynwinrt_map_dispatch = (IMAP_GUID_OBJECT, "_obj")

    def __init__(self, native):
        super().__init__(native, keys=GUID_KEYS)


class GeneratedStyleInt32ObjectMap(GeneratedStyleMap):
    _dynwinrt_interface_type = True
    _dynwinrt_interface_iid = IMAP_INT32_OBJECT
    _dynwinrt_map_dispatch = (IMAP_INT32_OBJECT, "_obj")

    def __init__(self, native):
        super().__init__(native, keys=INT32_KEYS)


class GeneratedStyleJsonMap(GeneratedStyleMap):
    _dynwinrt_interface_type = True
    _dynwinrt_interface_iid = IMAP_STRING_JSON
    _dynwinrt_map_dispatch = (IMAP_STRING_JSON, "_obj")

    def __init__(self, native):
        super().__init__(native, value_type=JSON_VALUE)


class GeneratedStyleInterface:
    """Built like a generated interface wrapper without a mapping protocol (IPropertySet)."""

    def __init__(self, native):
        self._obj = native


def activate(class_name):
    return DynWinRTValue.activation_factory(class_name).activate()


def property_set():
    return GeneratedStyleMap(activate("Windows.Foundation.Collections.PropertySet"))


def property_type(raw):
    """IPropertyValue.Type through a raw vtable call."""
    view = raw.cast(IPROPERTY_VALUE)
    try:
        return PropertyType(view.call_0(6, T.i32_type()).to_number())
    finally:
        view.release()


def uri(text="https://example.com/"):
    factory = DynWinRTValue.activation_factory("Windows.Foundation.Uri").cast(IURI_FACTORY)
    return factory.call(6, OBJECT, [T.hstring()], [DynWinRTValue.from_hstring(text)])


def date_time_box(ticks):
    """A DateTime box created by the real PropertyValue factory."""
    date_time = T.struct_type("Windows.Foundation.DateTime", [T.i64_type()])
    value = DynWinRTStruct.create(date_time)
    value.set_i64(0, ticks)
    factory = DynWinRTValue.activation_factory("Windows.Foundation.PropertyValue")
    statics = factory.cast(IPROPERTY_VALUE_STATICS)
    try:
        return statics.call(21, OBJECT, [date_time], [value.to_value()])
    finally:
        statics.release()
        factory.release()


@contextmanager
def payloadless_box(type_value=PropertyType.OtherType):
    """A Python-implemented IPropertyValue whose Type() reports ``type_value``."""
    methods = [
        DynWinRTImplementationMethod("get_Type", 6, DynWinRTMethodSig().add_out(T.i32_type()))
    ]
    plan = DynWinRTInterfacePlan.create(
        "Windows.Foundation.IPropertyValue", T.interface(IPROPERTY_VALUE), methods
    )

    def dispatch(_interface, slot, _args):
        assert slot == 6, "only Type() may be called for a payload-less box"
        return [DynWinRTValue.from_i32(int(type_value))]

    owner = DynWinRTImplementation.create([plan], dispatch)
    raw = owner.to_value()
    try:
        yield owner, raw
    finally:
        raw.release()
        owner.release()


def map_implementation_plan(name, value_type):
    """A complete ``IMap<String, value_type>`` implementation plan."""
    methods = GeneratedStyleMap.methods(T.hstring(), value_type)
    iid = T.parameterized(IMAP, [T.hstring(), value_type]).iid()
    interface = T.register_interface(name, iid)
    definitions = []
    for slot, (method_name, inputs, output) in enumerate(methods, 6):
        signature = DynWinRTMethodSig()
        for typ in inputs:
            signature = signature.add_in(typ)
        if output is not None:
            signature = signature.add_out(output)
        interface = interface.add_method(method_name, signature)
        definitions.append(
            DynWinRTImplementationMethod(method_name, slot, signature)
        )
    return DynWinRTInterfacePlan.create(name, interface, definitions)


@contextmanager
def object_and_string_maps():
    """One COM identity with distinct ``IMap<String, Object/String>`` stores."""
    plans = [
        map_implementation_plan("Tests.IMap_String_Object", OBJECT),
        map_implementation_plan("Tests.IMap_String_String", T.hstring()),
    ]
    stores = [
        {"shared": to_winrt_object(7)},
        {"shared": "wrong map"},
    ]
    calls = []

    def dispatch(interface, slot, args):
        calls.append((interface, slot))
        store = stores[interface]
        key = args[0].to_string() if args else None
        if slot == 6:  # Lookup
            value = store[key]
            return [
                value
                if interface == 0
                else DynWinRTValue.from_hstring(value)
            ]
        if slot == 7:  # get_Size
            return [DynWinRTValue.from_u32(len(store))]
        if slot == 8:  # HasKey
            return [DynWinRTValue.from_bool(key in store)]
        if slot == 9:  # GetView (unused by this regression)
            return [DynWinRTValue.null_value()]
        if slot == 10:  # Insert
            replaced = key in store
            store[key] = args[1] if interface == 0 else args[1].to_string()
            return [DynWinRTValue.from_bool(replaced)]
        if slot == 11:  # Remove
            del store[key]
            return []
        assert slot == 12  # Clear
        store.clear()
        return []

    owner = DynWinRTImplementation.create(plans, dispatch)
    raw = owner.to_value()
    try:
        yield raw, stores, calls
    finally:
        raw.release()
        owner.release()


# ----------------------------------------------------------------------
# Views and their protocol
# ----------------------------------------------------------------------


def test_object_value_view_picks_the_view_for_the_map_protocol():
    properties = property_set()
    view = object_value_view(properties)
    assert type(view) is MutableObjectValueView
    assert isinstance(view, MutableMapping) and isinstance(view, ObjectValueView)
    assert view.raw is properties and view.preserve_type is False
    assert repr(view) == f"dynwinrt.values.MutableObjectValueView({properties!r})"

    read_only = object_value_view(properties.get_view(), preserve_type=True)
    assert type(read_only) is ObjectValueView
    assert isinstance(read_only, Mapping) and not isinstance(read_only, MutableMapping)
    assert read_only.preserve_type is True
    assert repr(read_only).endswith(", preserve_type=True)")
    with pytest.raises(TypeError, match="does not support item assignment"):
        read_only["count"] = 5
    assert not hasattr(read_only, "pop") and not hasattr(read_only, "update")

    # A read-only view of a mutable map; a mutable view needs a mutable map.
    assert type(ObjectValueView(properties)) is ObjectValueView
    with pytest.raises(TypeError, match="requires a mutable map; GeneratedStyleMapView is"):
        MutableObjectValueView(properties.get_view())
    with pytest.raises(AttributeError):
        view.raw = properties


def test_multi_map_identity_uses_only_the_wrappers_projected_map():
    class MisleadingStringMap(GeneratedStyleStringMap):
        _dynwinrt_interface_type = True
        _dynwinrt_interface_iid = IMAP_STRING_OBJECT
        _dynwinrt_map_dispatch = (IMAP_STRING_STRING, "_obj")

    with object_and_string_maps() as (raw, stores, calls):
        object_map = GeneratedStyleMap(raw)
        string_map = GeneratedStyleStringMap(raw)

        view = object_value_view(object_map)
        assert view.raw is object_map
        assert view["shared"] == 7
        view["added"] = values.UInt32(9)
        assert unbox_object(stores[0]["added"]) == 9
        assert stores[1] == {"shared": "wrong map"}
        assert calls == [(0, 8), (0, 6), (0, 10)]

        before = list(calls)
        with pytest.raises(
            TypeError,
            match=r"projects a different WinRT map interface; pass "
            r"value\.as_interface\(IMap_String_Object\)",
        ):
            object_value_view(string_map)
        # QI does not enter either map implementation, and rejection happens
        # before any mapping operation can dispatch through the wrong vtable.
        assert calls == before

        with pytest.raises(TypeError, match="projects a different WinRT map interface"):
            object_value_view(MisleadingStringMap(raw))
        assert calls == before

        runtime_map = GeneratedStyleRuntimeMap(raw)
        runtime_view = object_value_view(runtime_map)
        assert runtime_view.raw is runtime_map
        assert runtime_view["shared"] == 7
        runtime_view["runtime"] = "collection pointer"
        assert unbox_object(stores[0]["runtime"]) == "collection pointer"
        assert stores[1] == {"shared": "wrong map"}
        assert calls[-3:] == [(0, 8), (0, 6), (0, 10)]

        default_runtime_map = GeneratedStyleDefaultRuntimeMap(raw)
        default_runtime_view = object_value_view(default_runtime_map)
        assert default_runtime_view.raw is default_runtime_map
        assert default_runtime_view["shared"] == 7
        default_runtime_view["default"] = "default pointer"
        assert unbox_object(stores[0]["default"]) == "default pointer"
        assert stores[1] == {"shared": "wrong map"}
        assert calls[-3:] == [(0, 8), (0, 6), (0, 10)]

        assert (
            raw.identity_raw()
            == object_map._obj.identity_raw()
            == string_map._obj.identity_raw()
            == runtime_map._obj.identity_raw()
            == runtime_map._collection_obj.identity_raw()
        )


def test_same_iid_query_does_not_require_physical_pointer_identity():
    class TearOff:
        released = False

        def as_raw(self):
            return 2

        def release(self):
            self.released = True

    class Dispatch:
        def __init__(self):
            self.queried = TearOff()

        def as_raw(self):
            return 1

        def cast(self, iid):
            assert iid.to_string().lower() == IMAP_STRING_OBJECT.to_string().lower()
            return self.queried

    dispatch = Dispatch()
    assert (
        values._projected_object_map(dispatch, IMAP_STRING_OBJECT, "IMap")
        == "IMap_String_Object"
    )
    assert dispatch.queried.released


def test_the_view_types_are_public_and_generic():
    names = ("WinRTObjectValue", "ObjectValueView", "MutableObjectValueView", "object_value_view")
    for name in names:
        assert name in values.__all__ and name not in dynwinrt.__all__
    assert ObjectValueView[str] is not None and MutableObjectValueView[UUID] is not None
    assert issubclass(MutableObjectValueView, ObjectValueView)


def test_legacy_direct_map_interface_retains_its_exact_iid_contract():
    class LegacyMap(GeneratedStyleMap):
        _dynwinrt_interface_type = True
        _dynwinrt_interface_iid = IMAP_STRING_OBJECT

    mapping = LegacyMap(activate("Windows.Foundation.Collections.PropertySet"))
    view = object_value_view(mapping)
    view["count"] = 5
    assert view.raw is mapping and view["count"] == 5


@pytest.mark.parametrize("class_name", ["PropertySet", "ValueSet"])
def test_writes_box_by_the_default_rules_and_reads_unbox(class_name):
    properties = GeneratedStyleMap(activate(f"Windows.Foundation.Collections.{class_name}"))
    view = object_value_view(properties)
    moment = datetime(2024, 5, 6, 7, 8, 9, 123456, tzinfo=timezone(timedelta(hours=2)))
    cases = [
        # (value, PropertyType, value read back)
        (5, PropertyType.Int32, 5),
        ("text", PropertyType.String, "text"),
        (moment, PropertyType.DateTime, moment),
        (values.UInt32(8080), PropertyType.UInt32, 8080),
        (values.UInt16Array([1, 2]), PropertyType.UInt16Array, [1, 2]),
        (b"\x00\xff", PropertyType.UInt8Array, b"\x00\xff"),
        (values.Point(1.5, 2.5), PropertyType.Point, values.Point(1.5, 2.5)),
    ]
    for index, (value, kind, expected) in enumerate(cases):
        view[f"key{index}"] = value
        assert property_type(properties[f"key{index}"]) == kind
        read = view[f"key{index}"]
        assert read == expected
        assert not isinstance(read, (values.WinRTScalar, values.WinRTArray))
    assert view["key2"].tzinfo is UTC

    view["null"] = None
    assert properties["null"] is None and view["null"] is None and "null" in view
    assert len(view) == len(cases) + 1


def test_the_view_is_live_in_both_directions():
    properties = property_set()
    view = object_value_view(properties)
    other = object_value_view(properties)
    properties["native"] = to_winrt_object(values.Int64(7))
    assert view["native"] == 7 and len(view) == 1
    view["converted"] = 1.5
    assert unbox_object(properties["converted"]) == 1.5
    assert other["converted"] == 1.5 and set(other) == {"native", "converted"}
    del properties["native"]
    with pytest.raises(KeyError):
        view["native"]
    assert list(view) == ["converted"]


def test_dict_and_equality_use_converted_values():
    properties = property_set()
    view = object_value_view(properties)
    view.update({"count": 5, "name": "text", "sizes": values.UInt32Array([1, 2])})
    expected = {"count": 5, "name": "text", "sizes": [1, 2]}
    assert dict(view) == expected
    assert view == expected and not view != expected
    assert view != {"count": 5}
    assert object_value_view(properties.get_view()) == expected
    assert view != [("count", 5)]


def test_mutable_mapping_methods():
    view = object_value_view(property_set())
    view.update({"a": 1}, b="x")
    view.update([("c", values.Int16(3))])
    assert dict(view) == {"a": 1, "b": "x", "c": 3}
    assert property_type(view.raw["c"]) == PropertyType.Int16
    guid = UUID("01234567-89ab-cdef-0123-456789abcdef")
    view.update(pair=(1, 2), guid=WinGUID.parse(str(guid)))
    assert property_type(view.raw["pair"]) == PropertyType.Int32Array
    assert view.pop("pair") == [1, 2] and view.pop("guid") == guid
    view.update()

    assert view.setdefault("a", 9) == 1
    stored = view.setdefault("d", values.UInt8(4))
    assert stored == 4 and type(stored) is int
    assert property_type(view.raw["d"]) == PropertyType.UInt8

    assert view.get("a") == 1 and view.get("missing") is None and view.get("missing", 0) == 0
    assert view.pop("a") == 1 and "a" not in view
    assert view.pop("missing", None) is None
    with pytest.raises(KeyError):
        view.pop("missing")
    key, value = view.popitem()
    assert key not in view and value in ("x", 3, 4)

    keys, items = view.keys(), view.items()
    view["e"] = [True, False]
    assert "e" in keys and ("e", [True, False]) in items
    assert sorted(view.values(), key=repr) == sorted(
        (unbox_object(view.raw[name]) for name in view), key=repr
    )

    del view["e"]
    with pytest.raises(KeyError):
        del view["e"]
    view.clear()
    assert len(view) == 0 and not view and list(view.raw) == []


def test_setdefault_returns_the_value_as_a_read_returns_it():
    properties = property_set()
    view = object_value_view(properties)
    raw = uri()

    class Projected:
        def __init__(self, obj):
            self._obj = obj

    # A missing key stores to_winrt_object(default) and reads it back.
    assert view.setdefault("pair", (1, 2)) == [1, 2]
    assert property_type(properties["pair"]) == PropertyType.Int32Array
    stored = view.setdefault("uri", Projected(raw))
    assert isinstance(stored, DynWinRTValue) and stored.identity_raw() == raw.identity_raw()
    assert view.setdefault("nothing") is None and "nothing" in view
    assert properties["nothing"] is None
    # A present key keeps its value.
    assert view.setdefault("pair", "ignored") == [1, 2]
    assert view.setdefault("nothing", 5) is None
    exact = object_value_view(properties, preserve_type=True)
    assert type(exact.setdefault("port", values.UInt32(80))) is values.UInt32
    assert type(exact.setdefault("pair")) is values.Int32Array
    with pytest.raises(TypeError, match="empty list or tuple"):
        view.setdefault("empty", [])
    assert "empty" not in view


def test_contains_len_and_iteration_do_not_unbox(monkeypatch):
    view = object_value_view(property_set())
    view["count"] = 5
    calls = []

    def counting(value, *, preserve_type=False):
        calls.append(value)
        return unbox_object(value, preserve_type=preserve_type)

    monkeypatch.setattr(values, "unbox_object", counting)
    assert "count" in view and "missing" not in view
    assert len(view) == 1 and list(view) == ["count"] and "count" in view.keys()
    assert calls == []
    assert view["count"] == 5 and len(calls) == 1


def test_raw_is_the_wrapper_and_keeps_the_box_identity():
    properties = property_set()
    box = to_winrt_object(values.UInt32(7))
    properties["port"] = box
    view = object_value_view(properties)
    exact = object_value_view(properties, preserve_type=True)
    assert view.raw is properties and exact.raw is properties
    assert view.raw["port"].identity_raw() == box.identity_raw()
    assert view["port"] == 7 and type(view["port"]) is int

    # Reading and writing back keeps the PropertyType, not the box's identity.
    assert type(exact["port"]) is values.UInt32
    exact["port"] = exact["port"]
    assert property_type(view.raw["port"]) == PropertyType.UInt32
    assert view.raw["port"].identity_raw() != box.identity_raw()
    # Native access through raw keeps it.
    view.raw["copy"] = view.raw["port"]
    assert view.raw["copy"].identity_raw() == view.raw["port"].identity_raw()


def test_objects_that_are_not_boxes_keep_their_identity():
    properties = property_set()
    view = object_value_view(properties)
    raw = uri()
    view["uri"] = raw
    read = view["uri"]
    assert isinstance(read, DynWinRTValue) and read.identity_raw() == raw.identity_raw()
    assert unbox_object(read) is read

    class Projected:
        def __init__(self, obj):
            self._obj = obj

    view["projected"] = Projected(raw)
    assert view["projected"].identity_raw() == raw.identity_raw()

    # A nested map is an object too, and gets a view of its own.
    nested = property_set()
    view["nested"] = nested
    inner = object_value_view(GeneratedStyleMap(view["nested"]))
    inner["count"] = 5
    assert object_value_view(nested)["count"] == 5


def test_boxes_without_a_python_form_come_back_raw():
    properties = property_set()
    view = object_value_view(properties)
    view["count"] = 5
    with payloadless_box() as (_owner, other):
        properties["other"] = other
        properties["nested"] = to_winrt_object(values.InspectableArray([1, other]))
        properties["far"] = date_time_box(2**62)
        for key in ("other", "nested", "far"):
            read = view[key]
            assert isinstance(read, DynWinRTValue)
            assert read.identity_raw() == properties[key].identity_raw()
        # Explicit unboxing still raises for the same values.
        with pytest.raises(OSError) as caught:
            unbox_object(properties["other"])
        assert type(caught.value) is OSError
        assert caught.value.winerror == E_NOTIMPL
        assert (
            caught.value._dynwinrt_unsupported_property_type
            == PropertyType.OtherType
        )
        with pytest.raises(OSError, match="Unsupported WinRT IPropertyValue type"):
            unbox_object(properties["nested"])
        with pytest.raises(OverflowError, match="outside the range of datetime.datetime"):
            unbox_object(properties["far"])
        # One odd entry does not break iteration.
        converted = dict(object_value_view(properties, preserve_type=True))
        assert converted["count"] == 5 and set(converted) == {"count", "other", "nested", "far"}


def test_supported_getter_e_notimpl_propagates_the_same_error(monkeypatch):
    properties = property_set()
    properties["count"] = to_winrt_object(5)
    failure = OSError(
        0,
        "supported IPropertyValue getter failed",
        None,
        E_NOTIMPL,
    )
    assert failure.winerror == E_NOTIMPL
    assert not hasattr(failure, "_dynwinrt_unsupported_property_type")

    def failing_getter(_raw, *, preserve_type=False):
        assert preserve_type is False
        raise failure

    monkeypatch.setattr(values, "unbox_object", failing_getter)
    with pytest.raises(OSError) as caught:
        object_value_view(properties)["count"]
    # The view must not classify by HRESULT, exception text or public type.
    assert caught.value is failure


def test_other_read_errors_propagate():
    properties = property_set()
    with payloadless_box() as (owner, box):
        properties["box"] = box
        owner.disconnect()
        with pytest.raises(OSError, match="disconnected"):
            object_value_view(properties)["box"]
        owner.take_error()


class Color(IntEnum):
    Red = 1


@pytest.mark.parametrize(
    "value",
    [
        pytest.param(2**31, id="int-above-Int32"),
        pytest.param(-(2**31) - 1, id="int-below-Int32"),
        pytest.param([], id="empty-list"),
        pytest.param((), id="empty-tuple"),
        pytest.param([1, "a"], id="mixed-list"),
        pytest.param([1, None], id="list-with-None"),
        pytest.param(Color.Red, id="enum-member"),
        pytest.param(datetime(2024, 1, 1), id="naive-datetime"),
        pytest.param(object(), id="object"),
        pytest.param({"a": 1}, id="dict"),
        pytest.param(DynWinRTValue.from_i32(5), id="non-object-DynWinRTValue"),
        pytest.param(values.StringArray([1]), id="invalid-typed-array"),
    ],
)
def test_write_errors_propagate_from_to_winrt_object(value):
    with pytest.raises((TypeError, ValueError, OverflowError)) as expected:
        to_winrt_object(value)
    properties = property_set()
    view = object_value_view(properties)
    view["key"] = "kept"
    with pytest.raises(type(expected.value)) as caught:
        view["key"] = value
    assert str(caught.value) == str(expected.value)
    with pytest.raises(type(expected.value)):
        view.update(other=value)
    assert dict(view) == {"key": "kept"}


@pytest.mark.parametrize(
    "value",
    [
        values.UInt8(255),
        values.Int16(-5),
        values.UInt16(9),
        values.UInt32(2**32 - 1),
        values.Int64(-(2**63)),
        values.UInt64(2**64 - 1),
        values.Single(0.1),
        values.Char16("x"),
        values.UInt16Array([1, 2]),
        values.DoubleArray([]),
        values.InspectableArray([None, values.UInt32(1), "a"]),
    ],
    ids=repr,
)
def test_preserve_type_reads_tags_and_writes_back_the_same_type(value):
    properties = property_set()
    view = object_value_view(properties, preserve_type=True)
    view["value"] = value
    kind = property_type(properties["value"])
    read = view["value"]
    assert type(read) is type(value) and read == value
    view["again"] = read
    assert property_type(properties["again"]) == kind
    assert view["again"] == value and type(view["again"]) is type(value)
    assert type(object_value_view(properties)["value"]) is not type(value)


def test_read_only_views_of_map_views():
    properties = property_set()
    object_value_view(properties).update(count=5, name="text")
    view = object_value_view(properties.get_view())
    assert view["count"] == 5 and view.get("name") == "text" and view.get("missing") is None
    assert len(view) == 2 and "count" in view and set(view) == {"count", "name"}
    assert dict(view.items()) == {"count": 5, "name": "text"}
    with pytest.raises(KeyError):
        view["missing"]


def test_guid_keyed_maps():
    media = GeneratedStyleGuidObjectMap(
        activate("Windows.Media.MediaProperties.MediaPropertySet")
    )
    view = object_value_view(media)
    key = UUID(int=7)
    view[key] = values.UInt32(3)
    assert type(view) is MutableObjectValueView
    assert dict(view) == {key: 3} and property_type(media[key]) == PropertyType.UInt32


def test_create_map_maps():
    native = DynWinRTValue.create_map(
        [DynWinRTValue.from_hstring("count")], [to_winrt_object(5)], T.hstring(), OBJECT
    )
    view = object_value_view(GeneratedStyleMap(native))
    view["name"] = "text"
    assert dict(view) == {"count": 5, "name": "text"}


# ----------------------------------------------------------------------
# Rejected maps and values
# ----------------------------------------------------------------------


def test_maps_without_object_values_are_rejected():
    message = "is not a WinRT map with Object values"
    string_map = GeneratedStyleStringMap(
        activate("Windows.Foundation.Collections.StringMap")
    )
    with pytest.raises(TypeError, match=f"GeneratedStyleStringMap {message}.*String or Guid keys"):
        object_value_view(string_map)
    with pytest.raises(TypeError, match=message):
        ObjectValueView(string_map.get_view())
    json_object = GeneratedStyleJsonMap(activate("Windows.Data.Json.JsonObject"))
    with pytest.raises(TypeError, match=message):
        object_value_view(json_object)
    int32_keys = DynWinRTValue.create_map(
        [DynWinRTValue.from_i32(1)], [to_winrt_object(5)], T.i32_type(), OBJECT
    )
    with pytest.raises(TypeError, match=message):
        object_value_view(GeneratedStyleInt32ObjectMap(int32_keys))


def test_values_that_are_not_generated_map_wrappers_are_rejected():
    properties = property_set()
    requires = "requires a generated WinRT map wrapper"

    class CustomMap(GeneratedStyleMap):
        pass

    class UndeclaredRuntimeMap(GeneratedStyleMap):
        _dynwinrt_runtime_class_type = True

    class InvalidInterfaceMarker(GeneratedStyleMap):
        _dynwinrt_interface_type = True
        _dynwinrt_interface_iid = IMAP_STRING_OBJECT
        _dynwinrt_map_dispatch = ("not a WinGUID", "_obj")

    class InvalidInterfaceDispatch(GeneratedStyleMap):
        _dynwinrt_interface_type = True
        _dynwinrt_interface_iid = IMAP_STRING_OBJECT
        _dynwinrt_map_dispatch = (IMAP_STRING_OBJECT, "_collection_obj")

        def __init__(self, native):
            super().__init__(native)
            self._collection_obj = self._obj

    with pytest.raises(TypeError, match=f"{requires}.*not .*CustomMap"):
        object_value_view(CustomMap(properties._obj))
    with pytest.raises(TypeError, match="does not declare a valid generated map projection"):
        object_value_view(UndeclaredRuntimeMap(properties._obj))
    with pytest.raises(TypeError, match="does not declare a valid generated map projection"):
        object_value_view(InvalidInterfaceMarker(properties._obj))
    with pytest.raises(TypeError, match="does not declare a valid generated map projection"):
        object_value_view(InvalidInterfaceDispatch(properties._obj))
    with pytest.raises(TypeError, match=f"{requires}.*not dict$"):
        object_value_view({"count": to_winrt_object(5)})
    with pytest.raises(TypeError, match=r"not DynWinRTValue; .*IMap_String_Object\.from_value"):
        object_value_view(properties._obj)
    with pytest.raises(TypeError, match=f"{requires}.*not NoneType"):
        object_value_view(None)
    with pytest.raises(TypeError, match=f"{requires}.*not GeneratedStyleInterface"):
        object_value_view(GeneratedStyleInterface(uri()))
    # IPropertySet wrappers are not Python mappings in generated bindings.
    with pytest.raises(
        TypeError,
        match=r"GeneratedStyleInterface is not a Python mapping in generated bindings; "
        r"pass value\.as_interface\(IMap_String_Object\) to object_value_view\(\)",
    ):
        object_value_view(GeneratedStyleInterface(properties._obj))
    with pytest.raises(TypeError, match="preserve_type must be a bool, not int"):
        object_value_view(properties, preserve_type=1)


@pytest.mark.parametrize("class_name", ["PropertySet", "ValueSet"])
def test_released_maps_fail_through_native_and_mapping_operations(class_name):
    properties = GeneratedStyleMap(activate(f"Windows.Foundation.Collections.{class_name}"))
    view = object_value_view(properties)
    view["count"] = 5
    view["empty"] = None
    read_only = object_value_view(properties.get_view())
    assert view.raw is properties
    assert view["empty"] is None and properties["empty"] is None
    assert read_only["empty"] is None and dict(read_only) == {"count": 5, "empty": None}
    assert view.setdefault("empty", 6) is None
    assert view.pop("absent", 7) == 7

    release_projected(properties)
    assert view.raw is properties and read_only["count"] == 5
    operations = [
        ("view creation", lambda: object_value_view(properties)),
        ("raw read", lambda: view.raw["count"]),
        ("raw null read", lambda: view.raw["empty"]),
        ("raw write", lambda: view.raw.__setitem__("added", to_winrt_object(6))),
        ("read", lambda: view["count"]),
        ("null read", lambda: view["empty"]),
        ("write", lambda: view.__setitem__("added", 6)),
        ("delete", lambda: view.__delitem__("count")),
        ("contains", lambda: "count" in view),
        ("length", lambda: len(view)),
        ("iteration", lambda: next(iter(view))),
        ("get", lambda: view.get("absent", 7)),
        ("keys", lambda: list(view.keys())),
        ("items", lambda: list(view.items())),
        ("values", lambda: list(view.values())),
        ("dict", lambda: dict(view)),
        ("equality", lambda: view == {"count": 5, "empty": None}),
        ("update", lambda: view.update(added=7)),
        ("setdefault", lambda: view.setdefault("added", 7)),
        ("pop", lambda: view.pop("absent", 7)),
        ("popitem", lambda: view.popitem()),
        ("clear", lambda: view.clear()),
    ]
    for label, operation in operations:
        with pytest.raises(RuntimeError) as caught:
            operation()
        assert str(caught.value) == f"This WinRT object {RELEASED_REASON}", label

    native_read_only = read_only.raw
    release_projected(native_read_only)
    assert read_only.raw is native_read_only
    for label, operation in [
        ("read-only creation", lambda: object_value_view(native_read_only)),
        ("read-only raw", lambda: read_only.raw["count"]),
        ("read-only null", lambda: read_only["empty"]),
        ("read-only get", lambda: read_only.get("absent", 7)),
        ("read-only items", lambda: list(read_only.items())),
    ]:
        with pytest.raises(RuntimeError) as caught:
            operation()
        assert str(caught.value) == f"This WinRT object {RELEASED_REASON}", label


def test_view_writes_reject_released_values_but_keep_live_null():
    view = object_value_view(property_set())
    view["empty"] = None
    released = to_winrt_object(5)
    released.release()
    assert view.setdefault("empty", released) is None

    operations = [
        ("item assignment", lambda: view.__setitem__("bad", released), "argument 0"),
        ("update", lambda: view.update(bad=released), "argument 0"),
        ("setdefault", lambda: view.setdefault("bad", released), "argument 0"),
        (
            "typed InspectableArray",
            lambda: view.__setitem__("bad", values.InspectableArray([None, released])),
            "element 1",
        ),
    ]
    for label, operation, slot in operations:
        with pytest.raises(RuntimeError) as caught:
            operation()
        assert (
            str(caught.value)
            == f"This WinRT object ({slot} of to_winrt_object()) {RELEASED_REASON}"
        ), label
        assert dict(view) == {"empty": None} and view.raw["empty"] is None
