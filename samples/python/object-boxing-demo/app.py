"""Automatic boxing and unboxing of WinRT Object values (experimental).

Every metadata position typed Object (IInspectable) converts through
dynwinrt.to_winrt_object / dynwinrt.from_winrt_object, so property bags,
property stores and XAML-style Object properties read and write plain Python
values while keeping the exact WinRT type on a round trip.
"""

import tempfile
from datetime import datetime, timezone
from pathlib import Path

import dynwinrt
from dynwinrt import RoApartment, UInt32, WinRTObjectInput, projected_lifetime_scope
from generated.windows.devices.enumeration import DeviceInformation
from generated.windows.foundation import IPropertyValue, Point, PropertyValue, Uri
from generated.windows.foundation.collections import PropertySet
from generated.windows.storage import StorageFile


def show(label: str, value: object) -> None:
    print(f"  {label:<26} {value!r}")


def boxed_type(value: WinRTObjectInput | None) -> str:
    """The WinRT PropertyType a Python value boxes as."""
    return IPropertyValue.from_value(dynwinrt.to_winrt_object(value)).type.name


def property_bag() -> None:
    print("PropertySet with plain Python values:")
    settings = PropertySet()
    settings["retries"] = 3
    settings["ratio"] = 0.75
    settings["name"] = "demo"
    settings["enabled"] = True
    settings["tags"] = ["a", "b"]
    settings["when"] = datetime(2024, 1, 2, tzinfo=timezone.utc)
    settings["origin"] = Point(1.5, 2.5)
    settings["port"] = UInt32(8080)
    settings["missing"] = None
    for key in sorted(settings):
        show(key, settings[key])
    retries = settings["retries"]
    assert isinstance(retries, int) and retries + 1 == 4
    assert settings["tags"] == ["a", "b"] and settings["origin"] == Point(1.5, 2.5)


def lossless_round_trip() -> None:
    print("Read-then-write keeps the exact WinRT type:")
    source, copy = PropertySet(), PropertySet()
    source["byte"] = PropertyValue.create_uint8(200)
    source["wide"] = PropertyValue.create_int64(5)
    source["char"] = PropertyValue.create_char16("x")
    source["single"] = PropertyValue.create_single(0.5)
    source["shorts"] = PropertyValue.create_int16_array([1, 2])
    for key, value in source.items():
        copy[key] = value
        print(f"  {key:<26} {copy[key]!r} boxes as {boxed_type(copy[key])}")
    assert boxed_type(copy["wide"]) == "Int64"


def file_properties() -> None:
    print("StorageFile property store:")
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "demo.txt"
        path.write_bytes(b"hello world")
        file = StorageFile.get_file_from_path_async(str(path)).wait()
        store = None if file is None else file.properties
        if store is None:
            raise RuntimeError("StorageFile returned no property store")
        values = store.retrieve_properties_async(["System.Size", "System.DateModified"]).wait()
        if values is None:
            raise RuntimeError("RetrievePropertiesAsync returned no values")
        size = values["System.Size"]
        show("System.Size", size)
        show("System.DateModified", values["System.DateModified"])
        show("size + 1", size + 1 if isinstance(size, int) else None)
        del file, store, values


def device_names() -> None:
    print("DeviceInformation property store:")
    for device in DeviceInformation.find_all_async().wait() or ():
        properties = None if device is None else device.properties
        name = None if properties is None else properties.get("System.ItemNameDisplay")
        if isinstance(name, str):
            show("System.ItemNameDisplay", name)
            return
    print("  no device exposes System.ItemNameDisplay")


def objects_keep_identity() -> None:
    print("Objects that are not boxed values stay native:")
    bag = PropertySet()
    bag["uri"] = Uri("https://example.com/demo")
    raw = bag["uri"]
    show("bag['uri']", raw)
    show("project_as(raw, Uri).host", dynwinrt.project_as(raw, Uri).host)


def main() -> None:
    with RoApartment(1), projected_lifetime_scope():
        property_bag()
        lossless_round_trip()
        file_properties()
        device_names()
        objects_keep_identity()
    print("python-object-boxing-demo-ok")


if __name__ == "__main__":
    main()
