# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import inspect as _implementation_inspect


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
