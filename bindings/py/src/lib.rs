// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use pyo3::prelude::*;

mod async_runtime;
mod errors;
mod runtime;
mod values;

#[pymodule]
mod dynwinrt_py {
    use pyo3::prelude::*;

    #[pymodule_init]
    fn init(m: &Bound<'_, PyModule>) -> PyResult<()> {
        super::async_runtime::init_async_runtime();
        m.py().run(
            c"
from collections.abc import (
    Iterable as _Iterable,
    Iterator as _Iterator,
    Mapping as _Mapping,
    MutableMapping as _MutableMapping,
    MutableSequence as _MutableSequence,
    Sequence as _Sequence,
)
from datetime import datetime as _datetime, timedelta as _timedelta, timezone as _timezone
from operator import index as _index
from typing import Protocol as _Protocol, TypeVar as _TypeVar
from typing import Awaitable as _Awaitable, Callable as _Callable
from uuid import UUID as _UUID

_T = _TypeVar('_T', covariant=True)
_P = _TypeVar('_P', covariant=True)
_WINRT_EPOCH = _datetime(1601, 1, 1, tzinfo=_timezone.utc)

class WinRTAsync(_Awaitable[_T], _Protocol[_T]):
    def wait(self) -> _T: ...
    def cancel(self) -> None: ...

class WinRTAsyncWithProgress(WinRTAsync[_T], _Protocol[_T, _P]):
    def progress(self, callback: _Callable[[_P], object]) -> None: ...

def _dynwinrt_guid(value):
    if isinstance(value, WinGUID):
        return value
    if not isinstance(value, _UUID):
        raise TypeError('requires uuid.UUID object')
    return WinGUID.parse(str(value))

def _dynwinrt_uuid(value):
    return _UUID(value.to_string())

def _dynwinrt_datetime_to_ticks(value):
    if not isinstance(value, _datetime):
        raise TypeError('requires datetime.datetime object')
    value = value.astimezone(_timezone.utc)
    delta = value - _WINRT_EPOCH
    return ((delta.days * 86400 + delta.seconds) * 1_000_000 + delta.microseconds) * 10

def _dynwinrt_ticks_to_microseconds(value):
    return value // 10 if value >= 0 else -((-value) // 10)

def _dynwinrt_ticks_to_datetime(value):
    return _WINRT_EPOCH + _timedelta(microseconds=_dynwinrt_ticks_to_microseconds(value))

def _dynwinrt_timedelta_to_ticks(value):
    if not isinstance(value, _timedelta):
        raise TypeError('requires datetime.timedelta object')
    return ((value.days * 86400 + value.seconds) * 1_000_000 + value.microseconds) * 10

def _dynwinrt_ticks_to_timedelta(value):
    return _timedelta(microseconds=_dynwinrt_ticks_to_microseconds(value))

def _dynwinrt_array(value, wrap, element_type, bytes_like=False):
    raw = getattr(value, '_obj', value)
    if isinstance(raw, DynWinRTValue):
        return raw
    if isinstance(raw, DynWinRTArray):
        return raw.to_value()
    if bytes_like and isinstance(raw, (bytes, bytearray)):
        return DynWinRTArray.from_bytes(raw).to_value()
    return DynWinRTArray.from_values([wrap(item) for item in raw], element_type).to_value()

def _dynwinrt_vector(value, wrap, element_type):
    raw = getattr(value, '_obj', value)
    if isinstance(raw, DynWinRTValue):
        return raw
    return DynWinRTValue.create_vector([wrap(item) for item in raw], element_type)

def _dynwinrt_map(value, wrap_key, wrap_value, key_type, value_type):
    raw = getattr(value, '_obj', value)
    if isinstance(raw, DynWinRTValue):
        return raw
    items = list(raw.items())
    return DynWinRTValue.create_map(
        [wrap_key(key) for key, _ in items],
        [wrap_value(item) for _, item in items],
        key_type,
        value_type,
    )

def _dynwinrt_bind_overload(parameter_names, args, kwargs):
    if len(args) > len(parameter_names):
        return None
    bound = list(args)
    for name in parameter_names[len(args):]:
        if name not in kwargs:
            return None
        bound.append(kwargs[name])
    if len(kwargs) != len(parameter_names) - len(args):
        return None
    if any(name in kwargs for name in parameter_names[:len(args)]):
        return None
    return tuple(bound)

def _dynwinrt_normalize_index(value, length):
    value = _index(value)
    if value < 0:
        value += length
    if value < 0 or value >= length:
        raise IndexError('collection index out of range')
    return value

class _WinRTSequenceMixin(_Sequence):
    def __len__(self):
        return self.size

    def __getitem__(self, index):
        if isinstance(index, slice):
            return [self.get_at(i) for i in range(*index.indices(len(self)))]
        return self.get_at(_dynwinrt_normalize_index(index, len(self)))

class _WinRTMutableSequenceMixin(_MutableSequence):
    def __len__(self):
        return self.size

    def __getitem__(self, index):
        if isinstance(index, slice):
            return [self.get_at(i) for i in range(*index.indices(len(self)))]
        return self.get_at(_dynwinrt_normalize_index(index, len(self)))

    def __setitem__(self, index, value):
        if isinstance(index, slice):
            items = list(self)
            items[index] = value
            self.replace_all(items)
            return
        self.set_at(_dynwinrt_normalize_index(index, len(self)), value)

    def __delitem__(self, index):
        if isinstance(index, slice):
            items = list(self)
            del items[index]
            self.replace_all(items)
            return
        self.remove_at(_dynwinrt_normalize_index(index, len(self)))

    def insert(self, index, value):
        index = _index(index)
        length = len(self)
        if index < 0:
            index = max(0, index + length)
        else:
            index = min(index, length)
        self.insert_at(index, value)

class _WinRTIterableMixin(_Iterable):
    def __iter__(self):
        return iter(self.first())

class _WinRTIteratorMixin(_Iterator):
    def __iter__(self):
        return self

    def __next__(self):
        if not self.has_current:
            raise StopIteration
        value = self.current
        self.move_next()
        return value

class _WinRTMappingMixin(_Mapping):
    def __len__(self):
        return self.size

    def __iter__(self):
        for pair in self._iter_pairs():
            yield pair.key

    def __getitem__(self, key):
        if not self.has_key(key):
            raise KeyError(key)
        return self.lookup(key)

class _WinRTMutableMappingMixin(_MutableMapping):
    def __len__(self):
        return self.size

    def __iter__(self):
        for pair in self._iter_pairs():
            yield pair.key

    def __getitem__(self, key):
        if not self.has_key(key):
            raise KeyError(key)
        return self.lookup(key)

    def __setitem__(self, key, value):
        self.insert(key, value)

    def __delitem__(self, key):
        if not self.has_key(key):
            raise KeyError(key)
        self.remove(key)

async def _dynwinrt_convert_future(future, converter):
    try:
        completed = await future
        return converter(completed._get_async_results())
    except BaseException:
        if not future.done():
            future.cancel()
        raise

def _dynwinrt_link_cancellation(task, future):
    def cancel_inner(completed):
        if completed.cancelled() and not future.done():
            future.cancel()
    task.add_done_callback(cancel_inner)

def _dynwinrt_dispatch_progress(callback, converter, value):
    callback(converter(value))
",
            Some(&m.dict()),
            None,
        )?;

        // Classes
        m.add_class::<super::runtime::WinAppSDKContext>()?;
        m.add_class::<super::runtime::RoApartment>()?;
        m.add_class::<super::runtime::WinGUID>()?;
        m.add_class::<super::runtime::DynWinRTType>()?;
        m.add_class::<super::runtime::DynWinRTMethodSig>()?;
        m.add_class::<super::runtime::DynWinRTMethodHandle>()?;
        m.add_class::<super::runtime::DynWinRTValue>()?;
        m.add_class::<super::runtime::DynWinRTArray>()?;
        m.add_class::<super::runtime::DynWinRTStruct>()?;
        m.add_class::<super::runtime::DynWinRtDelegate>()?;
        m.add_class::<super::async_runtime::DynWinRTAsync>()?;
        m.add_class::<super::async_runtime::DynWinRTAsyncWithProgress>()?;

        // Functions
        m.add_function(wrap_pyfunction!(super::runtime::init_winappsdk, m)?)?;
        m.add_function(wrap_pyfunction!(super::runtime::ro_initialize, m)?)?;
        m.add_function(wrap_pyfunction!(super::runtime::ro_uninitialize, m)?)?;
        m.add_function(wrap_pyfunction!(super::runtime::has_package_identity, m)?)?;
        m.add_function(wrap_pyfunction!(
            super::runtime::get_winappsdk_resource_pri_path,
            m
        )?)?;
        m.add_function(wrap_pyfunction!(super::runtime::get_computer_name, m)?)?;

        m.py().run(
            c"
__all__ = [name for name in __all__ if not name.startswith('_')]
for _name in ('WinRTAsync', 'WinRTAsyncWithProgress'):
    if _name not in __all__:
        __all__.append(_name)
",
            Some(&m.dict()),
            None,
        )?;

        Ok(())
    }
}
