# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Explicit value types for boxed WinRT ``Object`` (``IInspectable``) values.

Generated bindings keep ``Object`` values native. Convert them explicitly:
``dynwinrt.to_winrt_object(value, property_type=None)`` boxes a Python value
as a system ``Windows.Foundation.PropertyValue``, and
``dynwinrt.unbox_object(raw, preserve_type=True)`` reads a box back with the
types below wherever a plain Python value would box as a different
``PropertyType``.

- ``PropertyType`` mirrors ``Windows.Foundation.PropertyType``.
- Tags (``UInt8`` ... ``Char16``) are ``int``, ``float`` and ``str``
  subclasses that box as one exact scalar ``PropertyType``. They compare,
  hash, format, pickle and serialize like the plain value; only ``repr`` shows
  the tag, and arithmetic returns plain numbers.
- Typed arrays (``Int16Array`` ... ``RectArray``) are ``list`` subclasses that
  box as one exact array ``PropertyType``. ``bytes`` is the ``UInt8Array``
  form.
- ``Point``, ``Size`` and ``Rect`` are immutable ``Windows.Foundation``
  geometry values whose fields are stored as float32, as WinRT stores them.
"""

from __future__ import annotations

import operator
import struct
from datetime import datetime, timedelta
from enum import IntEnum
from typing import Any, ClassVar, Self, SupportsFloat, SupportsIndex, TypeVar
from uuid import UUID

__all__ = [
    "PropertyType",
    "WinRTScalar",
    "UInt8",
    "Int16",
    "UInt16",
    "Int32",
    "UInt32",
    "Int64",
    "UInt64",
    "Single",
    "Double",
    "Char16",
    "Point",
    "Size",
    "Rect",
    "WinRTArray",
    "Int16Array",
    "UInt16Array",
    "Int32Array",
    "UInt32Array",
    "Int64Array",
    "UInt64Array",
    "SingleArray",
    "DoubleArray",
    "Char16Array",
    "BooleanArray",
    "StringArray",
    "InspectableArray",
    "DateTimeArray",
    "TimeSpanArray",
    "GuidArray",
    "PointArray",
    "SizeArray",
    "RectArray",
]


class PropertyType(IntEnum):
    """``Windows.Foundation.PropertyType``: the kind of value a WinRT box holds."""

    Empty = 0
    UInt8 = 1
    Int16 = 2
    UInt16 = 3
    Int32 = 4
    UInt32 = 5
    Int64 = 6
    UInt64 = 7
    Single = 8
    Double = 9
    Char16 = 10
    Boolean = 11
    String = 12
    Inspectable = 13
    DateTime = 14
    TimeSpan = 15
    Guid = 16
    Point = 17
    Size = 18
    Rect = 19
    OtherType = 20
    UInt8Array = 1025
    Int16Array = 1026
    UInt16Array = 1027
    Int32Array = 1028
    UInt32Array = 1029
    Int64Array = 1030
    UInt64Array = 1031
    SingleArray = 1032
    DoubleArray = 1033
    Char16Array = 1034
    BooleanArray = 1035
    StringArray = 1036
    InspectableArray = 1037
    DateTimeArray = 1038
    TimeSpanArray = 1039
    GuidArray = 1040
    PointArray = 1041
    SizeArray = 1042
    RectArray = 1043
    OtherTypeArray = 1044


def _real(value: SupportsFloat | SupportsIndex, name: str) -> float:
    if not isinstance(value, (str, bytes, bytearray)):
        try:
            return float(value)
        except TypeError:
            pass
    raise TypeError(f"WinRT {name} requires a real number, not {type(value).__name__}")


def _single(value: SupportsFloat | SupportsIndex, name: str) -> float:
    number = _real(value, name)
    try:
        rounded: float = struct.unpack("<f", struct.pack("<f", number))[0]
    except OverflowError:
        raise OverflowError(f"{number!r} is out of range for WinRT {name}") from None
    return rounded


# ----------------------------------------------------------------------
# Tags
# ----------------------------------------------------------------------


class WinRTScalar:
    """Base class of the tags: a scalar that boxes as one exact ``PropertyType``."""

    __slots__ = ()
    property_type: ClassVar[PropertyType]


class _WinRTInteger(WinRTScalar, int):
    __slots__ = ()
    _range: ClassVar[tuple[int, int]] = (0, -1)

    def __new__(cls, value: SupportsIndex = 0, /) -> Self:
        try:
            number = operator.index(value)
        except TypeError:
            raise TypeError(
                f"WinRT {cls.__name__} requires an int, not {type(value).__name__}"
            ) from None
        low, high = cls._range
        if not low <= number <= high:
            raise OverflowError(
                f"{number} is out of range for WinRT {cls.__name__} ({low}..{high})"
            )
        return int.__new__(cls, number)

    def __repr__(self) -> str:
        return f"dynwinrt.values.{type(self).__name__}({int.__repr__(self)})"

    def __str__(self) -> str:
        return int.__repr__(self)


class UInt8(_WinRTInteger):
    """An ``int`` that boxes as ``PropertyType.UInt8``."""

    __slots__ = ()
    property_type = PropertyType.UInt8
    _range = (0, 0xFF)


class Int16(_WinRTInteger):
    """An ``int`` that boxes as ``PropertyType.Int16``."""

    __slots__ = ()
    property_type = PropertyType.Int16
    _range = (-0x8000, 0x7FFF)


class UInt16(_WinRTInteger):
    """An ``int`` that boxes as ``PropertyType.UInt16``."""

    __slots__ = ()
    property_type = PropertyType.UInt16
    _range = (0, 0xFFFF)


class Int32(_WinRTInteger):
    """An ``int`` that boxes as ``PropertyType.Int32``, the plain ``int`` default."""

    __slots__ = ()
    property_type = PropertyType.Int32
    _range = (-0x8000_0000, 0x7FFF_FFFF)


class UInt32(_WinRTInteger):
    """An ``int`` that boxes as ``PropertyType.UInt32``."""

    __slots__ = ()
    property_type = PropertyType.UInt32
    _range = (0, 0xFFFF_FFFF)


class Int64(_WinRTInteger):
    """An ``int`` that boxes as ``PropertyType.Int64``."""

    __slots__ = ()
    property_type = PropertyType.Int64
    _range = (-0x8000_0000_0000_0000, 0x7FFF_FFFF_FFFF_FFFF)


class UInt64(_WinRTInteger):
    """An ``int`` that boxes as ``PropertyType.UInt64``."""

    __slots__ = ()
    property_type = PropertyType.UInt64
    _range = (0, 0xFFFF_FFFF_FFFF_FFFF)


class Single(WinRTScalar, float):
    """A ``float`` that boxes as ``PropertyType.Single``; rounded to float32 on construction."""

    __slots__ = ()
    property_type = PropertyType.Single

    def __new__(cls, value: SupportsFloat | SupportsIndex = 0.0, /) -> Self:
        return float.__new__(cls, _single(value, "Single"))

    def __repr__(self) -> str:
        return f"dynwinrt.values.Single({float.__repr__(self)})"

    def __str__(self) -> str:
        return float.__repr__(self)


class Double(WinRTScalar, float):
    """A ``float`` that boxes as ``PropertyType.Double``, the plain ``float`` default."""

    __slots__ = ()
    property_type = PropertyType.Double

    def __new__(cls, value: SupportsFloat | SupportsIndex = 0.0, /) -> Self:
        return float.__new__(cls, _real(value, "Double"))

    def __repr__(self) -> str:
        return f"dynwinrt.values.Double({float.__repr__(self)})"

    def __str__(self) -> str:
        return float.__repr__(self)


class Char16(WinRTScalar, str):
    """One UTF-16 code unit that boxes as ``PropertyType.Char16``."""

    __slots__ = ()
    property_type = PropertyType.Char16

    def __new__(cls, value: str, /) -> Self:
        if not isinstance(value, str):
            raise TypeError(f"WinRT Char16 requires a str, not {type(value).__name__}")
        if len(value) != 1 or ord(value) > 0xFFFF:
            raise ValueError(
                f"WinRT Char16 requires exactly one UTF-16 code unit, not {value!r}"
            )
        return str.__new__(cls, value)

    def __repr__(self) -> str:
        return f"dynwinrt.values.Char16({str.__repr__(self)})"


# ----------------------------------------------------------------------
# Geometry
# ----------------------------------------------------------------------


class _WinRTGeometry:
    __slots__ = ()
    property_type: ClassVar[PropertyType]
    _fields: ClassVar[tuple[str, ...]] = ()

    def _values(self) -> tuple[float, ...]:
        return tuple(getattr(self, f"_{name}") for name in self._fields)

    def __setattr__(self, name: str, value: object) -> None:
        raise AttributeError(f"dynwinrt.values.{type(self).__name__} is immutable")

    def __delattr__(self, name: str) -> None:
        raise AttributeError(f"dynwinrt.values.{type(self).__name__} is immutable")

    def __eq__(self, other: object) -> bool:
        if type(other) is not type(self) or not isinstance(other, _WinRTGeometry):
            return NotImplemented
        return self._values() == other._values()

    def __hash__(self) -> int:
        return hash((type(self).__name__, self._values()))

    def __repr__(self) -> str:
        fields = ", ".join(
            f"{name}={value!r}" for name, value in zip(self._fields, self._values())
        )
        return f"dynwinrt.values.{type(self).__name__}({fields})"

    def __reduce__(self) -> tuple[type[Self], tuple[float, ...]]:
        return (type(self), self._values())


class Point(_WinRTGeometry):
    """An immutable ``Windows.Foundation.Point`` that boxes as ``PropertyType.Point``."""

    __slots__ = ("_x", "_y")
    __match_args__ = ("x", "y")
    property_type = PropertyType.Point
    _fields = ("x", "y")
    _x: float
    _y: float

    def __init__(self, x: float = 0.0, y: float = 0.0) -> None:
        object.__setattr__(self, "_x", _single(x, "Point.x"))
        object.__setattr__(self, "_y", _single(y, "Point.y"))

    @property
    def x(self) -> float:
        return self._x

    @property
    def y(self) -> float:
        return self._y


class Size(_WinRTGeometry):
    """An immutable ``Windows.Foundation.Size`` that boxes as ``PropertyType.Size``."""

    __slots__ = ("_width", "_height")
    __match_args__ = ("width", "height")
    property_type = PropertyType.Size
    _fields = ("width", "height")
    _width: float
    _height: float

    def __init__(self, width: float = 0.0, height: float = 0.0) -> None:
        object.__setattr__(self, "_width", _single(width, "Size.width"))
        object.__setattr__(self, "_height", _single(height, "Size.height"))

    @property
    def width(self) -> float:
        return self._width

    @property
    def height(self) -> float:
        return self._height


class Rect(_WinRTGeometry):
    """An immutable ``Windows.Foundation.Rect`` that boxes as ``PropertyType.Rect``."""

    __slots__ = ("_x", "_y", "_width", "_height")
    __match_args__ = ("x", "y", "width", "height")
    property_type = PropertyType.Rect
    _fields = ("x", "y", "width", "height")
    _x: float
    _y: float
    _width: float
    _height: float

    def __init__(
        self, x: float = 0.0, y: float = 0.0, width: float = 0.0, height: float = 0.0
    ) -> None:
        object.__setattr__(self, "_x", _single(x, "Rect.x"))
        object.__setattr__(self, "_y", _single(y, "Rect.y"))
        object.__setattr__(self, "_width", _single(width, "Rect.width"))
        object.__setattr__(self, "_height", _single(height, "Rect.height"))

    @property
    def x(self) -> float:
        return self._x

    @property
    def y(self) -> float:
        return self._y

    @property
    def width(self) -> float:
        return self._width

    @property
    def height(self) -> float:
        return self._height


# ----------------------------------------------------------------------
# Typed arrays
# ----------------------------------------------------------------------

_T = TypeVar("_T")


class WinRTArray(list[_T]):
    """Base class of the typed arrays: a list that boxes as one exact array ``PropertyType``.

    Elements are plain values and are validated when the array is boxed.
    Operations that build a new list, such as slicing, ``+`` and
    comprehensions, return plain lists.
    """

    __slots__ = ()
    property_type: ClassVar[PropertyType]

    def __repr__(self) -> str:
        return f"dynwinrt.values.{type(self).__name__}({list.__repr__(self)})"

    def copy(self) -> Self:
        return type(self)(self)


class Int16Array(WinRTArray[int]):
    """A list that boxes as ``PropertyType.Int16Array``."""

    __slots__ = ()
    property_type = PropertyType.Int16Array


class UInt16Array(WinRTArray[int]):
    """A list that boxes as ``PropertyType.UInt16Array``."""

    __slots__ = ()
    property_type = PropertyType.UInt16Array


class Int32Array(WinRTArray[int]):
    """A list that boxes as ``PropertyType.Int32Array``."""

    __slots__ = ()
    property_type = PropertyType.Int32Array


class UInt32Array(WinRTArray[int]):
    """A list that boxes as ``PropertyType.UInt32Array``."""

    __slots__ = ()
    property_type = PropertyType.UInt32Array


class Int64Array(WinRTArray[int]):
    """A list that boxes as ``PropertyType.Int64Array``."""

    __slots__ = ()
    property_type = PropertyType.Int64Array


class UInt64Array(WinRTArray[int]):
    """A list that boxes as ``PropertyType.UInt64Array``."""

    __slots__ = ()
    property_type = PropertyType.UInt64Array


class SingleArray(WinRTArray[float]):
    """A list that boxes as ``PropertyType.SingleArray``."""

    __slots__ = ()
    property_type = PropertyType.SingleArray


class DoubleArray(WinRTArray[float]):
    """A list that boxes as ``PropertyType.DoubleArray``."""

    __slots__ = ()
    property_type = PropertyType.DoubleArray


class Char16Array(WinRTArray[str]):
    """A list of one-code-unit strings that boxes as ``PropertyType.Char16Array``."""

    __slots__ = ()
    property_type = PropertyType.Char16Array


class BooleanArray(WinRTArray[bool]):
    """A list that boxes as ``PropertyType.BooleanArray``."""

    __slots__ = ()
    property_type = PropertyType.BooleanArray


class StringArray(WinRTArray[str]):
    """A list that boxes as ``PropertyType.StringArray``."""

    __slots__ = ()
    property_type = PropertyType.StringArray


class InspectableArray(WinRTArray[Any]):
    """A list that boxes as ``PropertyType.InspectableArray``.

    Each element is converted like a ``to_winrt_object`` argument without
    ``property_type``: ``None`` is a null element and WinRT objects are stored
    as they are.
    """

    __slots__ = ()
    property_type = PropertyType.InspectableArray


class DateTimeArray(WinRTArray[datetime]):
    """A list of timezone-aware datetimes that boxes as ``PropertyType.DateTimeArray``."""

    __slots__ = ()
    property_type = PropertyType.DateTimeArray


class TimeSpanArray(WinRTArray[timedelta]):
    """A list that boxes as ``PropertyType.TimeSpanArray``."""

    __slots__ = ()
    property_type = PropertyType.TimeSpanArray


class GuidArray(WinRTArray[UUID]):
    """A list that boxes as ``PropertyType.GuidArray``."""

    __slots__ = ()
    property_type = PropertyType.GuidArray


class PointArray(WinRTArray[Point]):
    """A list that boxes as ``PropertyType.PointArray``."""

    __slots__ = ()
    property_type = PropertyType.PointArray


class SizeArray(WinRTArray[Size]):
    """A list that boxes as ``PropertyType.SizeArray``."""

    __slots__ = ()
    property_type = PropertyType.SizeArray


class RectArray(WinRTArray[Rect]):
    """A list that boxes as ``PropertyType.RectArray``."""

    __slots__ = ()
    property_type = PropertyType.RectArray
