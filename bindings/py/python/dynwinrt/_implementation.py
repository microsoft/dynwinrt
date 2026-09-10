# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

# Loaded into the native module's globals to preserve public type identity.

import inspect as _implementation_inspect
from typing import Generic as _ImplementationGeneric, TypeVar as _ImplementationTypeVar

_ImplementationValue = _ImplementationTypeVar("_ImplementationValue", covariant=True)


def _dynwinrt_require_sync_implementation_callable(callback):
    if not callable(callback):
        raise TypeError("WinRT implementation callback must be callable")
    call = getattr(callback, "__call__", None)
    if any(
        _implementation_inspect.iscoroutinefunction(candidate)
        or _implementation_inspect.isasyncgenfunction(candidate)
        for candidate in (callback, call)
    ):
        raise TypeError(
            "WinRT implementation callbacks must be synchronous; "
            "async callables are not supported"
        )


def _dynwinrt_validate_implementation_result(result):
    if (
        _implementation_inspect.isawaitable(result)
        or _implementation_inspect.isasyncgen(result)
    ):
        if _implementation_inspect.iscoroutine(result):
            result.close()
        raise TypeError(
            "WinRT implementation callbacks must return synchronously; "
            "coroutine and awaitable results are not supported"
        )
    if not isinstance(result, list) or any(
        not isinstance(value, DynWinRTValue) for value in result
    ):
        raise TypeError(
            "WinRT implementation callbacks must return a list of DynWinRTValue "
            "outputs in signature order (use [] for a void method)"
        )
    return result


def _dynwinrt_checked_implementation_callback(callback):
    _dynwinrt_require_sync_implementation_callable(callback)

    def invoke(interface_index, vtable_index, args):
        return _dynwinrt_validate_implementation_result(
            callback(interface_index, vtable_index, args)
        )

    return invoke


class DynWinRTImplementationDescriptor:
    """One generated interface plan and its synchronous projected dispatcher."""

    __slots__ = ("_plan", "_dispatch", "__weakref__")

    def __init__(self, plan, dispatch):
        if not isinstance(plan, DynWinRTInterfacePlan):
            raise TypeError("plan must be a DynWinRTInterfacePlan")
        _dynwinrt_require_sync_implementation_callable(dispatch)
        self._plan = plan
        self._dispatch = dispatch

    @property
    def plan(self):
        return self._plan

    def dispatch(self, vtable_index, args):
        return _dynwinrt_validate_implementation_result(
            self._dispatch(vtable_index, args)
        )


class DynWinRTImplementationHandle(_ImplementationGeneric[_ImplementationValue]):
    """Manage a lazy, stable typed primary view without mixing lifecycle methods into it."""

    def __init__(self, owner, projector):
        if not isinstance(owner, DynWinRTImplementation) or not callable(projector):
            raise TypeError("A WinRT implementation handle requires an owner and typed projector")
        self._owner = owner
        self._projector = projector
        self._value = None
        self._released = False
        self._projecting = False

    @staticmethod
    def _create(interface, handlers, additional, interfaces):
        descriptors = [interface.implementation(handlers), *additional]
        for entry in interfaces:
            if (
                not isinstance(entry, (tuple, list)) or len(entry) != 2
                or not getattr(entry[0], "_dynwinrt_interface_type", False)
                or not callable(getattr(entry[0], "implementation", None))
                or not callable(getattr(entry[0], "from_implementation", None))
            ):
                raise TypeError("Each interface entry must be (generated_interface, handler)")
            descriptors.append(entry[0].implementation(entry[1]))
        if any(not isinstance(item, DynWinRTImplementationDescriptor) for item in descriptors):
            raise TypeError("Invalid WinRT implementation descriptor")
        dispatchers = tuple(item.dispatch for item in descriptors)

        def callback(index, slot, args):
            if not isinstance(index, int) or index < 0 or index >= len(dispatchers):
                raise ValueError("Unknown implementation interface index")
            return dispatchers[index](slot, args)

        owner = DynWinRTImplementation.create([item.plan for item in descriptors], callback)
        return DynWinRTImplementationHandle(owner, interface.from_implementation)

    @property
    def value(self):
        if self._released or self._owner.is_closed:
            raise RuntimeError("WinRT implementation handle is closed or released")
        if self._value is None:
            if self._projecting:
                raise RuntimeError("WinRT implementation primary view creation is reentrant")
            self._projecting = True
            try:
                value = self._projector(self._owner)
                if self._released or self._owner.is_closed:
                    release_projected(value)
                    raise RuntimeError("WinRT implementation was released during primary view creation")
                self._value = value
            except BaseException:
                self.dispose()
                raise
            finally:
                self._projecting = False
        return self._value

    def to_value(self):
        if self._released:
            raise RuntimeError("WinRT implementation handle is released")
        return self._owner.to_value()

    def release(self):
        if self._released:
            return
        value = self._value
        self._released = True
        self._value = None
        try:
            if value is not None:
                release_projected(value)
        finally:
            self._owner.release()

    def disconnect(self):
        self._owner.disconnect()

    def dispose(self):
        self.disconnect()
        self.release()

    @property
    def is_closed(self):
        return self._owner.is_closed

    def take_error(self):
        return self._owner.take_error()

    def __enter__(self):
        if self._released or self.is_closed:
            raise RuntimeError("WinRT implementation handle is closed or released")
        return self

    def __exit__(self, _exc_type, _exc_value, _traceback):
        self.dispose()
        return False
