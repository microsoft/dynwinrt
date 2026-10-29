# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Structured concurrency for same-thread, asynchronous notification handlers."""

from __future__ import annotations

import asyncio as _asyncio
import threading as _threading
from collections.abc import Awaitable as _Awaitable, Callable as _Callable
from types import TracebackType as _TracebackType
from typing import ParamSpec as _ParamSpec, final as _final

_Args = _ParamSpec("_Args")


@_final
class AsyncEventScope:
    """Own notification subscriptions and their concurrent TaskGroup tasks.

    The application must drive ``loop`` on the creating OS thread and keep its
    COM apartment and projected lifetimes alive until this context has exited.
    Only ordinary notifications that may be handled later are supported.
    """

    def __init__(self, *, loop: _asyncio.AbstractEventLoop) -> None:
        self._loop = loop
        self._thread = _threading.current_thread()
        self._group = _asyncio.TaskGroup()
        self._state = "new"
        self._owner_task: _asyncio.Task[object] | None = None
        self._subscriptions: list[_Callable[[], None]] = []

    def _check_owner(self) -> None:
        if _threading.current_thread() is not self._thread:
            raise RuntimeError("AsyncEventScope requires its owner OS thread")
        if _asyncio.get_running_loop() is not self._loop:
            raise RuntimeError("AsyncEventScope requires its running asyncio loop")

    async def __aenter__(self) -> AsyncEventScope:
        self._check_owner()
        if self._state != "new":
            raise RuntimeError("AsyncEventScope cannot be re-entered or reused")
        await self._group.__aenter__()
        self._owner_task = _asyncio.current_task()
        self._state = "active"
        return self

    def subscribe(
        self,
        subscribe: _Callable[[_Callable[_Args, object]], _Callable[[], None]],
        handler: _Callable[_Args, _Awaitable[object]],
    ) -> _Callable[[], None]:
        """Register a handler; return a retryable, idempotent unsubscribe.

        Unsubscribing stops new deliveries before attempting native removal.
        Already accepted tasks remain owned by the scope, even if removal fails.
        The handler is called inside a group task and its result is awaited.
        """
        self._check_owner()
        if self._state != "active":
            raise RuntimeError("AsyncEventScope is not accepting subscriptions")
        if not callable(subscribe) or not callable(handler):
            raise TypeError("subscribe and handler must be callable")
        accepting = True
        removing = False
        remove: _Callable[[], None] | None = None

        async def invoke(*args: _Args.args, **kwargs: _Args.kwargs) -> None:
            self._check_owner()
            await handler(*args, **kwargs)

        def callback(*args: _Args.args, **kwargs: _Args.kwargs) -> None:
            self._check_owner()
            if not accepting or self._state != "active":
                return
            coroutine = invoke(*args, **kwargs)
            try:
                self._group.create_task(coroutine)
            except BaseException:
                # TaskGroup only closes rejected coroutines on Python 3.13+.
                coroutine.close()
                raise

        def unsubscribe() -> None:
            nonlocal accepting, removing, remove
            self._check_owner()
            accepting = False
            if remove is None or removing:
                return
            removing = True
            try:
                remove()
                remove = None
                self._subscriptions.remove(unsubscribe)
            finally:
                removing = False

        try:
            remove = subscribe(callback)
            if not callable(remove):
                raise TypeError("subscribe must return an unsubscribe callable")
        except BaseException:
            accepting = False
            raise
        self._subscriptions.append(unsubscribe)
        return unsubscribe

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: _TracebackType | None,
    ) -> None:
        self._check_owner()
        if self._state != "active":
            raise RuntimeError("AsyncEventScope is not active")
        if _asyncio.current_task() is not self._owner_task:
            raise RuntimeError("AsyncEventScope must exit in the task that entered it")
        self._state = "closing"
        errors: list[BaseException] = []
        for unsubscribe in reversed(self._subscriptions.copy()):
            try:
                unsubscribe()
            except BaseException as error:
                errors.append(error)
        if errors:
            if exc_value is not None:
                errors.insert(0, exc_value)
            exc_value = BaseExceptionGroup("AsyncEventScope unsubscribe failed", errors)
            exc_type = type(exc_value)
            traceback = exc_value.__traceback__
        try:
            await self._group.__aexit__(exc_type, exc_value, traceback)
        finally:
            self._state = "closed"
            self._owner_task = None
