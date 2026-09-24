# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

# Public data model for WinRT ``Object`` (IInspectable) values.
#
# Loaded into the native module's globals (like _implementation.py) so the Rust
# conversion layer, ``to_winrt_object`` / ``from_winrt_object``, and Python share
# one set of types. Public types report ``__module__ == 'dynwinrt'`` so ``repr``,
# ``pickle`` and ``copy`` use the public import path.
#
# Names mirror ``Windows.Foundation.PropertyType``. A tag records the exact
# WinRT type of a value read from an ``Object`` position when the plain Python
# write rule would pick a different type (plain ``int`` boxes as Int32, plain
# ``float`` as Double, one-character ``str`` as String). Tags behave like the
# plain value (``==``, ``hash``, ``str``, ``format``, ``json``); only ``repr``
# shows the tag, and arithmetic returns plain numbers.

import operator as _values_operator
import struct as _values_struct
from datetime import datetime as _values_datetime, timedelta as _values_timedelta
from uuid import UUID as _values_UUID

if 'DynWinRTValue' not in globals():
    # A standalone import would create a second, unrelated set of value types.
    raise ImportError('dynwinrt._values is loaded by the dynwinrt native module; import dynwinrt')


def _values_to_f32(value, label):
    try:
        return _values_struct.unpack('<f', _values_struct.pack('<f', value))[0]
    except OverflowError:
        raise OverflowError(f'{value!r} is out of range for WinRT {label}') from None


def _values_real(value, label):
    if isinstance(value, (str, bytes, bytearray)) or not hasattr(value, '__float__') and not hasattr(value, '__index__'):
        raise TypeError(f'WinRT {label} requires a real number, not {type(value).__name__}')
    return float(value)


class WinRTScalar:
    """Base of the PropertyType-tagged scalars (``UInt32``, ``Single``, ...)."""

    __slots__ = ()
    property_type = ''


class _WinRTInteger(WinRTScalar, int):
    __slots__ = ()
    _range = (0, -1)

    def __new__(cls, value=0, /):
        number = _values_operator.index(value)
        low, high = cls._range
        if not low <= number <= high:
            raise OverflowError(
                f'{number} is out of range for WinRT {cls.property_type} ({low}..{high})'
            )
        return int.__new__(cls, number)

    def __repr__(self):
        return f'dynwinrt.{type(self).__name__}({int.__repr__(self)})'

    __str__ = int.__repr__


class UInt8(_WinRTInteger):
    """An integer boxed as ``PropertyType.UInt8``."""

    __slots__ = ()
    property_type = 'UInt8'
    _range = (0, 0xFF)


class Int16(_WinRTInteger):
    """An integer boxed as ``PropertyType.Int16``."""

    __slots__ = ()
    property_type = 'Int16'
    _range = (-0x8000, 0x7FFF)


class UInt16(_WinRTInteger):
    """An integer boxed as ``PropertyType.UInt16``."""

    __slots__ = ()
    property_type = 'UInt16'
    _range = (0, 0xFFFF)


class Int32(_WinRTInteger):
    """An integer boxed as ``PropertyType.Int32`` (the plain ``int`` default)."""

    __slots__ = ()
    property_type = 'Int32'
    _range = (-0x8000_0000, 0x7FFF_FFFF)


class UInt32(_WinRTInteger):
    """An integer boxed as ``PropertyType.UInt32``."""

    __slots__ = ()
    property_type = 'UInt32'
    _range = (0, 0xFFFF_FFFF)


class Int64(_WinRTInteger):
    """An integer boxed as ``PropertyType.Int64``."""

    __slots__ = ()
    property_type = 'Int64'
    _range = (-0x8000_0000_0000_0000, 0x7FFF_FFFF_FFFF_FFFF)


class UInt64(_WinRTInteger):
    """An integer boxed as ``PropertyType.UInt64``."""

    __slots__ = ()
    property_type = 'UInt64'
    _range = (0, 0xFFFF_FFFF_FFFF_FFFF)


class Single(WinRTScalar, float):
    """A float boxed as ``PropertyType.Single``; rounded to float32 on construction."""

    __slots__ = ()
    property_type = 'Single'

    def __new__(cls, value=0.0, /):
        return float.__new__(cls, _values_to_f32(_values_real(value, 'Single'), 'Single'))

    def __repr__(self):
        return f'dynwinrt.Single({float.__repr__(self)})'

    __str__ = float.__repr__


class Double(WinRTScalar, float):
    """A float boxed as ``PropertyType.Double`` (the plain ``float`` default)."""

    __slots__ = ()
    property_type = 'Double'

    def __new__(cls, value=0.0, /):
        return float.__new__(cls, _values_real(value, 'Double'))

    def __repr__(self):
        return f'dynwinrt.Double({float.__repr__(self)})'

    __str__ = float.__repr__


class Char16(WinRTScalar, str):
    """One UTF-16 code unit boxed as ``PropertyType.Char16``."""

    __slots__ = ()
    property_type = 'Char16'

    def __new__(cls, value, /):
        if not isinstance(value, str):
            raise TypeError(f'WinRT Char16 requires a str, not {type(value).__name__}')
        if len(value) != 1 or ord(value) > 0xFFFF:
            raise ValueError(f'WinRT Char16 requires exactly one UTF-16 code unit, not {value!r}')
        return str.__new__(cls, value)

    def __repr__(self):
        return f'dynwinrt.Char16({str.__repr__(self)})'


class WinRTArray(list):
    """A list that boxes as one specific WinRT array ``PropertyType``.

    Arrays read from ``Object`` positions are returned as these subclasses, so
    empty and homogeneous-``InspectableArray`` values keep their type when they
    are written back. Operations that build new lists (slicing, ``+``) return
    plain lists; their tagged elements still infer the same array type.
    """

    __slots__ = ()
    property_type = ''

    def __repr__(self):
        return f'dynwinrt.{type(self).__name__}({list.__repr__(self)})'

    def copy(self):
        return type(self)(self)


class Int16Array(WinRTArray):
    __slots__ = ()
    property_type = 'Int16Array'


class UInt16Array(WinRTArray):
    __slots__ = ()
    property_type = 'UInt16Array'


class Int32Array(WinRTArray):
    __slots__ = ()
    property_type = 'Int32Array'


class UInt32Array(WinRTArray):
    __slots__ = ()
    property_type = 'UInt32Array'


class Int64Array(WinRTArray):
    __slots__ = ()
    property_type = 'Int64Array'


class UInt64Array(WinRTArray):
    __slots__ = ()
    property_type = 'UInt64Array'


class SingleArray(WinRTArray):
    __slots__ = ()
    property_type = 'SingleArray'


class DoubleArray(WinRTArray):
    __slots__ = ()
    property_type = 'DoubleArray'


class Char16Array(WinRTArray):
    __slots__ = ()
    property_type = 'Char16Array'


class BooleanArray(WinRTArray):
    __slots__ = ()
    property_type = 'BooleanArray'


class StringArray(WinRTArray):
    __slots__ = ()
    property_type = 'StringArray'


class InspectableArray(WinRTArray):
    __slots__ = ()
    property_type = 'InspectableArray'


class DateTimeArray(WinRTArray):
    __slots__ = ()
    property_type = 'DateTimeArray'


class TimeSpanArray(WinRTArray):
    __slots__ = ()
    property_type = 'TimeSpanArray'


class GuidArray(WinRTArray):
    __slots__ = ()
    property_type = 'GuidArray'


class PointArray(WinRTArray):
    __slots__ = ()
    property_type = 'PointArray'


class SizeArray(WinRTArray):
    __slots__ = ()
    property_type = 'SizeArray'


class RectArray(WinRTArray):
    __slots__ = ()
    property_type = 'RectArray'


class _WinRTGeometry:
    """Immutable float32 fields of a boxed ``Windows.Foundation`` geometry struct.

    Equal to any value with the same ``_dynwinrt_property_type`` (including the
    generated ``windows.foundation`` struct of that name) whose fields box to
    the same float32 values.
    """

    __slots__ = ()
    property_type = ''
    _dynwinrt_property_type = ''
    _fields: tuple[str, ...] = ()

    def __init__(self, *args, **kwargs):
        names = type(self)._fields
        if len(args) > len(names):
            raise TypeError(f'{type(self).__name__} takes at most {len(names)} positional arguments')
        values = dict(zip(names, args))
        for name, value in kwargs.items():
            if name not in names or name in values:
                raise TypeError(f'{type(self).__name__} got an unexpected or repeated argument {name!r}')
            values[name] = value
        for name in names:
            label = f'{type(self).property_type}.{name}'
            object.__setattr__(
                self, f'_{name}', _values_to_f32(_values_real(values.get(name, 0.0), label), label)
            )

    def __setattr__(self, name, value):
        raise AttributeError(f'dynwinrt.{type(self).__name__} is immutable')

    def __delattr__(self, name):
        raise AttributeError(f'dynwinrt.{type(self).__name__} is immutable')

    def _values(self):
        return tuple(getattr(self, f'_{name}') for name in type(self)._fields)

    def __eq__(self, other):
        kind = type(self)._dynwinrt_property_type
        if getattr(type(other), '_dynwinrt_property_type', None) != kind:
            return NotImplemented
        if isinstance(other, _WinRTGeometry):
            return self._values() == other._values()
        try:
            other_values = tuple(
                _values_to_f32(_values_real(getattr(other, name), kind), kind)
                for name in type(self)._fields
            )
        except (AttributeError, OverflowError, TypeError):
            return False
        return self._values() == other_values

    def __hash__(self):
        return hash((type(self)._dynwinrt_property_type, self._values()))

    def __repr__(self):
        fields = ', '.join(
            f'{name}={value!r}' for name, value in zip(type(self)._fields, self._values())
        )
        return f'dynwinrt.{type(self).__name__}({fields})'

    def __reduce__(self):
        return (type(self), self._values())


class Point(_WinRTGeometry):
    """A boxed ``Windows.Foundation.Point`` (``PropertyType.Point``)."""

    __slots__ = ('_x', '_y')
    __match_args__ = ('x', 'y')
    property_type = 'Point'
    _dynwinrt_property_type = 'Point'
    _fields = ('x', 'y')
    x = property(lambda self: self._x)
    y = property(lambda self: self._y)


class Size(_WinRTGeometry):
    """A boxed ``Windows.Foundation.Size`` (``PropertyType.Size``)."""

    __slots__ = ('_width', '_height')
    __match_args__ = ('width', 'height')
    property_type = 'Size'
    _dynwinrt_property_type = 'Size'
    _fields = ('width', 'height')
    width = property(lambda self: self._width)
    height = property(lambda self: self._height)


class Rect(_WinRTGeometry):
    """A boxed ``Windows.Foundation.Rect`` (``PropertyType.Rect``)."""

    __slots__ = ('_x', '_y', '_width', '_height')
    __match_args__ = ('x', 'y', 'width', 'height')
    property_type = 'Rect'
    _dynwinrt_property_type = 'Rect'
    _fields = ('x', 'y', 'width', 'height')
    x = property(lambda self: self._x)
    y = property(lambda self: self._y)
    width = property(lambda self: self._width)
    height = property(lambda self: self._height)


# The native module namespace this file executes in; it supplies DynWinRTValue
# and the export list.
_values_native = globals()

# Values ``from_winrt_object`` can return (``None`` is added per position).
WinRTObjectValue = (
    bool | int | float | str | _values_UUID | _values_datetime | _values_timedelta
    | bytes | Point | Size | Rect | list | _values_native['DynWinRTValue']
)
# Values ``to_winrt_object`` accepts besides ``None`` (plus projected wrappers,
# generated enums and generated Windows.Foundation Point/Size/Rect structs).
WinRTObjectInput = WinRTObjectValue | tuple | bytearray | memoryview

_DYNWINRT_VALUE_EXPORTS = (
    'WinRTScalar', 'UInt8', 'Int16', 'UInt16', 'Int32', 'UInt32', 'Int64', 'UInt64',
    'Single', 'Double', 'Char16',
    'WinRTArray', 'Int16Array', 'UInt16Array', 'Int32Array', 'UInt32Array', 'Int64Array',
    'UInt64Array', 'SingleArray', 'DoubleArray', 'Char16Array', 'BooleanArray',
    'StringArray', 'InspectableArray', 'DateTimeArray', 'TimeSpanArray', 'GuidArray',
    'PointArray', 'SizeArray', 'RectArray',
    'Point', 'Size', 'Rect',
    'WinRTObjectValue', 'WinRTObjectInput',
)

for _values_name in _DYNWINRT_VALUE_EXPORTS:
    _values_export = _values_native[_values_name]
    if isinstance(_values_export, type):
        _values_export.__module__ = 'dynwinrt'
    if _values_name not in _values_native['__all__']:
        _values_native['__all__'].append(_values_name)
for _values_export in (_WinRTInteger, _WinRTGeometry):
    _values_export.__module__ = 'dynwinrt'
del _values_name, _values_export, _values_native
