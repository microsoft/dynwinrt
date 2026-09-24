"""Static typing of WinRT Object positions.

Check with ``mypy --strict typing_check.py`` and ``pyright typing_check.py``
after running generate.ps1.
"""

from datetime import datetime, timezone
from typing import assert_type

import dynwinrt
from dynwinrt import DynWinRTValue, UInt32, WinRTObjectInput, WinRTObjectValue
from generated.windows.foundation import Point, Uri
from generated.windows.foundation.collections import PropertySet


def write(properties: PropertySet, uri: Uri, raw: DynWinRTValue) -> None:
    properties["count"] = 5
    properties["ratio"] = 0.5
    properties["name"] = "demo"
    properties["tags"] = ["a", "b"]
    properties["pair"] = (1, 2)
    properties["when"] = datetime.now(timezone.utc)
    properties["port"] = UInt32(8080)
    properties["origin"] = Point(1.0, 2.0)
    properties["uri"] = uri
    properties["raw"] = raw
    properties["missing"] = None
    properties.insert("inserted", b"bytes")
    properties.update({"a": 1, "b": "two"})


def read(properties: PropertySet) -> int:
    value = properties["count"]
    assert_type(value, WinRTObjectValue | None)
    if isinstance(value, bool):
        return int(value)
    if isinstance(value, int):
        return value + 1
    if isinstance(value, DynWinRTValue):
        return 0
    return -1


def convert(value: WinRTObjectInput) -> WinRTObjectValue | None:
    boxed = dynwinrt.to_winrt_object(value)
    assert_type(boxed, DynWinRTValue)
    return dynwinrt.from_winrt_object(boxed)
