# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Scenarios for generated bindings whose WinRT ``Object`` positions convert
through ``to_winrt_object`` / ``from_winrt_object``.

Run by ``test_object_values_generated.py``, one scenario per fresh process:
the metadata table is process-global, and other test modules register partial
layouts of the same Windows interfaces. Usage::

    python object_values_scenarios.py <generated-root> <package> <scenario>
"""

import importlib
import sys
import tempfile
import threading
from datetime import datetime, timedelta, timezone
from pathlib import Path
from uuid import UUID

import dynwinrt
from dynwinrt import (
    DynWinRTType,
    DynWinRTValue,
    WinGUID,
    from_winrt_object,
    project_as,
    ro_initialize,
    to_winrt_object,
    unbox_object,
)

CLASSES = ",".join(
    [
        "Windows.Foundation.Collections.PropertySet",
        "Windows.Foundation.Collections.ValueSet",
        "Windows.Foundation.Collections.StringMap",
        "Windows.Foundation.PropertyValue",
        "Windows.Foundation.IPropertyValue",
        "Windows.Foundation.Uri",
        "Windows.Devices.Enumeration.DeviceInformation",
        "Windows.Storage.StorageFile",
    ]
)
IREFERENCE_PIID = WinGUID.parse("61c17706-2d65-11e0-9ae8-d48564015472")
UTC = timezone.utc


class Skip(Exception):
    """The scenario's prerequisite is unavailable on this machine."""


def _import(sdk, module):
    return importlib.import_module(f"{sdk.__name__}.{module}")


def _raw_lookup(properties, key):
    """IMap<String, Object>.Lookup without the Object conversion (vtable 6)."""
    return properties._collection_obj.call_1(
        6, DynWinRTType.object(), DynWinRTValue.from_hstring(key)
    )


def _stored_type(foundation, properties, key):
    return foundation.IPropertyValue.from_value(_raw_lookup(properties, key)).type


def scenario_property_value_factories_round_trip_every_property_type(foundation, collections):
    """Write with PropertyValue.create_X, read, write back: the type is still X."""
    pv, kinds = foundation.PropertyValue, foundation.PropertyType
    point, size, rect = foundation.Point, foundation.Size, foundation.Rect
    moment = datetime(2024, 5, 6, 7, 8, 9, 123456, tzinfo=UTC)
    cases = [
        ("create_uint8", 200, "UInt8"),
        ("create_int16", -3, "Int16"),
        ("create_uint16", 65535, "UInt16"),
        ("create_int32", -7, "Int32"),
        ("create_uint32", 2**32 - 1, "UInt32"),
        ("create_int64", -(2**63), "Int64"),
        ("create_uint64", 2**64 - 1, "UInt64"),
        ("create_single", 0.5, "Single"),
        ("create_double", 0.1, "Double"),
        ("create_char16", "x", "Char16"),
        ("create_boolean", True, "Boolean"),
        ("create_string", "text", "String"),
        ("create_guid", UUID(int=5), "Guid"),
        ("create_date_time", moment, "DateTime"),
        ("create_time_span", timedelta(seconds=-5), "TimeSpan"),
        ("create_point", point(1.5, 2.5), "Point"),
        ("create_size", size(3.0, 4.0), "Size"),
        ("create_rect", rect(1.0, 2.0, 3.0, 4.0), "Rect"),
        ("create_uint8_array", b"\x01\x02", "UInt8Array"),
        ("create_int16_array", [1, -2], "Int16Array"),
        ("create_uint16_array", [1, 2], "UInt16Array"),
        ("create_int32_array", [1, -2], "Int32Array"),
        ("create_uint32_array", [1, 2], "UInt32Array"),
        ("create_int64_array", [1, -2], "Int64Array"),
        ("create_uint64_array", [1, 2], "UInt64Array"),
        ("create_single_array", [0.5, 1.5], "SingleArray"),
        ("create_double_array", [0.1, 2.0], "DoubleArray"),
        ("create_char16_array", ["a", "b"], "Char16Array"),
        ("create_boolean_array", [True, False], "BooleanArray"),
        ("create_string_array", ["a", ""], "StringArray"),
        ("create_inspectable_array", [None, "nested"], "InspectableArray"),
        ("create_guid_array", [UUID(int=1)], "GuidArray"),
        ("create_date_time_array", [moment], "DateTimeArray"),
        ("create_time_span_array", [timedelta(1)], "TimeSpanArray"),
        ("create_point_array", [point(1.0, 2.0)], "PointArray"),
        ("create_size_array", [size(1.0, 2.0)], "SizeArray"),
        ("create_rect_array", [rect(1.0, 2.0, 3.0, 4.0)], "RectArray"),
    ]
    first, second = collections.PropertySet(), collections.PropertySet()
    for factory, argument, expected in cases:
        value = getattr(pv, factory)(argument)
        assert not isinstance(value, DynWinRTValue), factory
        first[factory] = value
        assert _stored_type(foundation, first, factory) == kinds[expected], factory
        read = first[factory]
        assert type(read) is type(value) and read == value, (factory, read, value)
        second[factory] = read
        assert _stored_type(foundation, second, factory) == kinds[expected], factory
        assert second[factory] == value


def scenario_plain_python_values_write_and_read_back(foundation, collections):
    kinds = foundation.PropertyType
    values = {
        "bool": (True, "Boolean"),
        "int": (5, "Int32"),
        "big": (2**40, "Int64"),
        "huge": (2**63, "UInt64"),
        "float": (1.5, "Double"),
        "str": ("text", "String"),
        "uuid": (UUID(int=7), "Guid"),
        "when": (datetime(2024, 1, 2, tzinfo=UTC), "DateTime"),
        "span": (timedelta(minutes=3), "TimeSpan"),
        "bytes": (b"\x00\xff", "UInt8Array"),
        "ints": ([1, 2, 3], "Int32Array"),
        "floats": ([1, 2.5], "DoubleArray"),
        "strings": (["a", "b"], "StringArray"),
        "mixed": ([1, "a", None], "InspectableArray"),
        "point": (dynwinrt.Point(1, 2), "Point"),
        "tagged": (dynwinrt.UInt16(7), "UInt16"),
    }
    properties = collections.PropertySet()
    properties.update({key: value for key, (value, _) in values.items()})
    for key, (value, kind) in values.items():
        assert _stored_type(foundation, properties, key) == kinds[kind], key
        assert properties[key] == value, key
    properties["none"] = None
    assert properties["none"] is None and _raw_lookup(properties, "none").is_null()


def scenario_property_set_value_set_and_string_map_behave_like_dicts(foundation, collections):
    properties = collections.PropertySet()
    properties["a"] = 1
    properties.update({"b": "two", "c": [1.5, 2.5]})
    expected = {"a": 1, "b": "two", "c": [1.5, 2.5]}
    assert dict(properties) == expected and properties == expected
    assert sorted(properties) == ["a", "b", "c"] and len(properties) == 3
    assert properties.get("a") == 1 and properties.get("missing") is None
    assert properties.get("missing", "default") == "default"
    assert "a" in properties and "missing" not in properties
    assert sorted(properties.values(), key=repr) == sorted(expected.values(), key=repr)
    assert dict(properties.items()) == expected
    assert properties.setdefault("z", 3) == 3 and properties["z"] == 3
    assert properties.pop("b") == "two" and "b" not in properties
    del properties["a"]
    assert set(properties) == {"c", "z"}

    values = collections.ValueSet()
    values.update(properties)
    assert dict(values) == dict(properties)
    values["tagged"] = dynwinrt.UInt32(9)
    assert repr(values["tagged"]) == "dynwinrt.UInt32(9)"
    try:
        values["object"] = foundation.Uri("https://example.com/")  # ValueSet stores values only
    except OSError:
        pass
    else:
        raise AssertionError("ValueSet accepted a non-value object")

    strings = collections.StringMap()
    strings["k"] = "v"
    strings.update({"x": "y"})
    assert dict(strings) == {"k": "v", "x": "y"} and strings.get("k") == "v"


def scenario_objects_keep_identity_and_raw_values_stay_compatible(foundation, collections):
    uri = foundation.Uri("https://example.com/path")
    properties = collections.PropertySet()
    properties["uri"] = uri
    raw = properties["uri"]
    assert isinstance(raw, DynWinRTValue)
    assert raw.identity_raw() == uri._obj.identity_raw()
    assert project_as(raw, foundation.Uri).host == "example.com"
    assert from_winrt_object(raw) is raw and unbox_object(raw) is raw
    properties["raw"] = raw
    assert properties["raw"].identity_raw() == uri._obj.identity_raw()
    properties["legacy"] = to_winrt_object(dynwinrt.Int64(3))
    assert type(properties["legacy"]) is dynwinrt.Int64
    assert unbox_object(properties["legacy"]) == 3


def scenario_generated_geometry_structs_and_enums_box_like_csharp(foundation, collections):
    properties = collections.PropertySet()
    properties["point"] = foundation.Point(1.5, 2.5)
    assert _stored_type(foundation, properties, "point") == foundation.PropertyType.Point
    read = properties["point"]
    assert read == dynwinrt.Point(1.5, 2.5)
    assert read == foundation.Point(1.5, 2.5) and foundation.Point(1.5, 2.5) == read

    properties["kind"] = foundation.PropertyType.UInt32
    raw = properties["kind"]
    assert isinstance(raw, DynWinRTValue)  # IReference<Enum>, not an IPropertyValue
    enum_type = DynWinRTType.enum_type("Windows.Foundation.PropertyType")
    iid = DynWinRTType.parameterized(IREFERENCE_PIID, [enum_type]).iid()
    assert raw.cast(iid).call_0(6, enum_type).get_enum_int() == 5


def scenario_device_information_properties_unbox_to_python_values(sdk):
    enumeration = _import(sdk, "windows.devices.enumeration")
    devices = enumeration.DeviceInformation.find_all_async().wait()
    named = [device for device in devices if "System.ItemNameDisplay" in device.properties]
    if not named:
        raise Skip("no device exposes System.ItemNameDisplay")
    assert all(isinstance(device.properties["System.ItemNameDisplay"], str) for device in named)
    values = dict(named[0].properties)
    # Boxed property values arrive as Python values; only objects stay native.
    assert any(not isinstance(value, DynWinRTValue) for value in values.values())


def scenario_storage_file_properties_unbox_uint64_size_and_datetime(sdk, foundation, collections):
    storage = _import(sdk, "windows.storage")
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "object-values.txt"
        path.write_bytes(b"hello world")
        file = storage.StorageFile.get_file_from_path_async(str(path)).wait()
        values = file.properties.retrieve_properties_async(
            ["System.Size", "System.DateModified"]
        ).wait()
        size, modified = values["System.Size"], values["System.DateModified"]
        assert type(size) is dynwinrt.UInt64 and size == 11
        assert isinstance(modified, datetime) and modified.tzinfo is not None
        assert abs(datetime.now(UTC) - modified) < timedelta(hours=1)
        copy = collections.PropertySet()
        copy.update(values)
        assert _stored_type(foundation, copy, "System.Size") == foundation.PropertyType.UInt64
        assert _stored_type(foundation, copy, "System.DateModified") == (
            foundation.PropertyType.DateTime
        )
        del file, values


def scenario_event_arguments_typed_object_use_the_conversion(sdk):
    enumeration = _import(sdk, "windows.devices.enumeration")
    watcher = enumeration.DeviceInformation.create_watcher()
    completed = threading.Event()
    arguments = []

    def on_completed(sender, args):
        arguments.append(args)
        completed.set()

    unsubscribers = [
        watcher.subscribe_added(lambda sender, info: None),
        watcher.subscribe_updated(lambda sender, update: None),
        watcher.subscribe_enumeration_completed(on_completed),
    ]
    watcher.start()
    try:
        assert completed.wait(60), "EnumerationCompleted was not raised"
    finally:
        watcher.stop()
        for unsubscribe in unsubscribers:
            unsubscribe()
    assert arguments[:1] == [None]


def main(root, package, scenario):
    sys.path.insert(0, root)
    ro_initialize(1)
    sdk = importlib.import_module(package)
    foundation = _import(sdk, "windows.foundation")
    collections = _import(sdk, "windows.foundation.collections")
    function = globals()[f"scenario_{scenario}"]
    arguments = {"sdk": sdk, "foundation": foundation, "collections": collections}
    names = function.__code__.co_varnames[: function.__code__.co_argcount]
    try:
        function(*(arguments[name] for name in names))
    except Skip as reason:
        print(f"object-values-skip: {reason}", flush=True)
        return
    print(f"object-values-ok: {scenario}", flush=True)


if __name__ == "__main__":
    main(*sys.argv[1:])
