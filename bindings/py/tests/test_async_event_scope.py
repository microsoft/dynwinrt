# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import asyncio
from contextvars import ContextVar
import gc
import threading
import weakref

import pytest

from dynwinrt import AsyncEventScope


class Notification:
    def __init__(self, *, during_registration=False, failures=0):
        self.callback = None
        self.during_registration = during_registration
        self.failures = failures
        self.removals = 0
        self.removing = lambda: None

    def subscribe(self, callback):
        self.callback = callback
        if self.during_registration:
            self.emit(0)

        def remove():
            self.removals += 1
            self.removing()
            if self.failures:
                self.failures -= 1
                raise OSError("native removal failed")
            self.callback = None

        return remove

    def emit(self, value):
        if self.callback is not None:
            return self.callback(value)


def leaves(error):
    if isinstance(error, BaseExceptionGroup):
        return [leaf for child in error.exceptions for leaf in leaves(child)]
    return [error]


@pytest.fixture(params=["default", "eager"])
def task_factory(request):
    if request.param == "eager" and not hasattr(asyncio, "eager_task_factory"):
        pytest.skip("eager task factories require Python 3.12")
    return getattr(asyncio, "eager_task_factory", None) if request.param == "eager" else None


def run_with_factory(coroutine, factory):
    async def run():
        loop = asyncio.get_running_loop()
        previous = loop.get_task_factory()
        loop.set_task_factory(factory)
        try:
            return await coroutine
        finally:
            loop.set_task_factory(previous)

    return asyncio.run(run())


def test_normal_exit_unsubscribes_then_drains_accepted_tasks(task_factory):
    async def run():
        event = Notification(during_registration=True)
        finished = []

        async def changed(value):
            await asyncio.sleep(0)
            assert event.callback is None
            finished.append(value)

        async with AsyncEventScope(loop=asyncio.get_running_loop()) as scope:
            stop = scope.subscribe(event.subscribe, changed)
            event.emit(1)
            stop()
            stop()
            event.emit(2)
            assert finished == []
        assert sorted(finished) == [0, 1]
        assert event.removals == 1
        stop()

    run_with_factory(run(), task_factory)


@pytest.mark.parametrize("after_await", [False, True])
def test_handler_failure_cancels_siblings_and_waits_for_finally(task_factory, after_await):
    async def run():
        event = Notification()
        started = asyncio.Event()
        finished = []

        async def changed(value):
            if value == 0:
                try:
                    started.set()
                    await asyncio.Event().wait()
                finally:
                    await asyncio.sleep(0)
                    finished.append("finally")
            else:
                if after_await:
                    await asyncio.sleep(0)
                raise ValueError("handler failed")

        with pytest.raises(ExceptionGroup) as failure:
            async with AsyncEventScope(loop=asyncio.get_running_loop()) as scope:
                scope.subscribe(event.subscribe, changed)
                event.emit(0)
                await started.wait()
                event.emit(1)
                await asyncio.Event().wait()
        assert [str(error) for error in leaves(failure.value)] == ["handler failed"]
        assert finished == ["finally"]
        assert event.removals == 1

    run_with_factory(run(), task_factory)


@pytest.mark.parametrize("cancel_during_exit", [False, True])
def test_external_cancel_waits_for_finally(task_factory, cancel_during_exit):
    async def run():
        event = Notification()
        started = asyncio.Event()
        finished = []

        async def changed(value):
            try:
                started.set()
                await asyncio.Event().wait()
            finally:
                await asyncio.sleep(0)
                finished.append(value)

        async def parent():
            async with AsyncEventScope(loop=asyncio.get_running_loop()) as scope:
                scope.subscribe(event.subscribe, changed)
                event.emit(1)
                if not cancel_during_exit:
                    await asyncio.Event().wait()

        task = asyncio.create_task(parent())
        await started.wait()
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task
        assert finished == [1]
        assert event.removals == 1

    run_with_factory(run(), task_factory)


def test_concurrent_handlers_can_reenter_notification_source(task_factory):
    async def run():
        event = Notification()
        reentered = asyncio.Event()
        done = []

        async def changed(value):
            if value == 0:
                event.emit(1)
                await reentered.wait()
            else:
                reentered.set()
            done.append(value)

        async with AsyncEventScope(loop=asyncio.get_running_loop()) as scope:
            scope.subscribe(event.subscribe, changed)
            event.emit(0)
            await reentered.wait()
        assert done == [1, 0]

    run_with_factory(run(), task_factory)


def test_contextvars_are_inherited_without_clearing_projection_context(task_factory):
    marker = ContextVar("marker")

    async def run():
        event = Notification()
        observed = []

        async def changed(value):
            await asyncio.sleep(0)
            observed.append(marker.get())

        async with AsyncEventScope(loop=asyncio.get_running_loop()) as scope:
            scope.subscribe(event.subscribe, changed)
            marker.set("delivery")
            event.emit(1)
            marker.set("parent changed")
        assert observed == ["delivery"]

    run_with_factory(run(), task_factory)


def test_awaitable_callable_and_keyword_parameters(task_factory):
    async def run():
        callbacks = []
        results = []

        def subscribe(callback):
            callbacks.append(callback)
            return callbacks.clear

        class Handler:
            def __call__(self, sender, *, value):
                async def work():
                    await asyncio.sleep(0)
                    results.append((sender, value))
                return work()

        async with AsyncEventScope(loop=asyncio.get_running_loop()) as scope:
            scope.subscribe(subscribe, Handler())
            callbacks[0]("sender", value=42)
        assert results == [("sender", 42)]

    run_with_factory(run(), task_factory)


def test_sync_handler_result_failure_belongs_to_taskgroup():
    async def run():
        event = Notification()
        with pytest.raises(ExceptionGroup) as failure:
            async with AsyncEventScope(loop=asyncio.get_running_loop()) as scope:
                scope.subscribe(event.subscribe, lambda value: None)
                event.emit(0)
        assert isinstance(leaves(failure.value)[0], TypeError)
        assert event.removals == 1

    asyncio.run(run())


def test_failed_manual_unsubscribe_stays_stopped_and_is_retryable(task_factory):
    async def run():
        event = Notification(failures=1)
        received = []

        async def changed(value):
            received.append(value)

        async with AsyncEventScope(loop=asyncio.get_running_loop()) as scope:
            stop = scope.subscribe(event.subscribe, changed)
            event.emit(1)
            event.removing = lambda: (stop(), event.emit(2))
            with pytest.raises(OSError, match="native removal"):
                stop()
            event.emit(3)
            stop()
            stop()
        assert received == [1]
        assert event.removals == 2

    run_with_factory(run(), task_factory)


@pytest.mark.parametrize("body_error", [None, ValueError("body"), asyncio.CancelledError()])
def test_exit_attempts_all_removals_and_preserves_errors_for_retry(body_error):
    async def run():
        events = [Notification(failures=1), Notification(), Notification(failures=1)]
        stops = []

        async def changed(value):
            await asyncio.sleep(0)

        with pytest.raises(ExceptionGroup) as failure:
            async with AsyncEventScope(loop=asyncio.get_running_loop()) as scope:
                stops.extend(scope.subscribe(event.subscribe, changed) for event in events)
                if body_error:
                    raise body_error
        errors = leaves(failure.value)
        assert sum(isinstance(error, OSError) for error in errors) == 2
        if isinstance(body_error, asyncio.CancelledError):
            assert not any(isinstance(error, asyncio.CancelledError) for error in errors)
        elif body_error:
            assert body_error in errors
        assert [event.removals for event in events] == [1, 1, 1]
        for event, stop in zip(events, stops):
            event.emit(1)
            stop()
            stop()
            assert event.callback is None

    asyncio.run(run())


def test_cleanup_error_cancels_tasks_instead_of_hanging():
    async def run():
        event = Notification(failures=1)
        started = asyncio.Event()
        finished = []

        async def changed(value):
            try:
                started.set()
                await asyncio.Event().wait()
            finally:
                await asyncio.sleep(0)
                finished.append(value)

        with pytest.raises(ExceptionGroup):
            async with AsyncEventScope(loop=asyncio.get_running_loop()) as scope:
                stop = scope.subscribe(event.subscribe, changed)
                event.emit(1)
                await started.wait()
        assert finished == [1]
        stop()

    asyncio.run(run())


@pytest.mark.parametrize("failure", ["handler", "external", "both", "cancel_during_cleanup"])
@pytest.mark.parametrize("cleanup_fails", [False, True])
def test_cleanup_and_cancellation_match_taskgroup(task_factory, failure, cleanup_fails):
    async def exercise(use_scope):
        event = Notification(failures=int(cleanup_fails))
        started = asyncio.Event()
        fail_handler = asyncio.Event()
        finalized = []

        async def changed(value):
            try:
                started.set()
                await fail_handler.wait()
                raise ValueError("handler failed")
            finally:
                finalized.append(value)

        async def owner():
            task = asyncio.current_task()
            if failure == "cancel_during_cleanup":
                event.removing = task.cancel
            result = {}
            try:
                if use_scope:
                    async with AsyncEventScope(loop=asyncio.get_running_loop()) as scope:
                        stop = scope.subscribe(event.subscribe, changed)
                        event.emit(1)
                        await asyncio.Event().wait()
                else:
                    async with asyncio.TaskGroup() as group:
                        stop = event.subscribe(lambda value: group.create_task(changed(value)))
                        try:
                            event.emit(1)
                            await asyncio.Event().wait()
                        finally:
                            stop()
            except BaseException as error:
                result["exception"] = type(error)
                result["leaves"] = sorted(
                    (type(leaf).__name__, str(leaf)) for leaf in leaves(error)
                )
                result["ordinary_exception"] = isinstance(error, Exception)
            else:
                raise AssertionError("expected handler failure or cancellation")
            result["cancelling"] = task.cancelling()
            try:
                await asyncio.sleep(0)
            except asyncio.CancelledError:
                result["next_await_cancelled"] = True
            else:
                result["next_await_cancelled"] = False
            assert finalized == [1]
            assert event.removals == 1
            event.removing = lambda: None
            if cleanup_fails:
                stop()
                assert event.removals == 2
            assert event.callback is None
            return result

        task = asyncio.create_task(owner())
        await started.wait()
        if failure != "external":
            fail_handler.set()
        if failure in ("external", "both"):
            task.cancel()
        return await task

    async def run():
        expected = await exercise(False)
        actual = await exercise(True)
        assert actual == expected
        if cleanup_fails:
            assert actual["ordinary_exception"]
            assert ("OSError", "native removal failed") in actual["leaves"]
            assert all(name != "CancelledError" for name, _ in actual["leaves"])
        elif failure == "external":
            assert actual["exception"] is asyncio.CancelledError

    run_with_factory(run(), task_factory)


def test_registration_failure_disables_its_callback_and_drains_accepted_work():
    async def run():
        received = []
        callbacks = []

        async def changed(value):
            received.append(value)

        def subscribe(callback):
            callbacks.append(callback)
            callback(1)
            raise OSError("registration failed")

        async with AsyncEventScope(loop=asyncio.get_running_loop()) as scope:
            with pytest.raises(OSError, match="registration failed"):
                scope.subscribe(subscribe, changed)
            callbacks[0](2)
        assert received == [1]

    asyncio.run(run())


def test_invalid_scope_lifecycle_and_wrong_exit_task():
    async def run():
        scope = AsyncEventScope(loop=asyncio.get_running_loop())
        event = Notification()

        async def changed(value):
            pass

        with pytest.raises(RuntimeError, match="not accepting"):
            scope.subscribe(event.subscribe, changed)
        with pytest.raises(RuntimeError, match="not active"):
            await scope.__aexit__(None, None, None)
        async with scope:
            with pytest.raises(RuntimeError, match="re-entered"):
                await scope.__aenter__()
            with pytest.raises(RuntimeError, match="task that entered"):
                await asyncio.create_task(scope.__aexit__(None, None, None))
            with pytest.raises(TypeError, match="must be callable"):
                scope.subscribe(event.subscribe, None)
            with pytest.raises(TypeError, match="unsubscribe callable"):
                scope.subscribe(lambda callback: None, changed)
        with pytest.raises(RuntimeError, match="re-entered"):
            await scope.__aenter__()
        with pytest.raises(RuntimeError, match="not accepting"):
            scope.subscribe(event.subscribe, changed)

    asyncio.run(run())


def test_wrong_loop_and_no_running_loop_fail_before_registration():
    loop = asyncio.new_event_loop()
    scope = AsyncEventScope(loop=loop)
    with pytest.raises(RuntimeError, match="no running event loop"):
        scope.subscribe(lambda callback: lambda: None, lambda: None)

    async def run():
        with pytest.raises(RuntimeError, match="running asyncio loop"):
            await scope.__aenter__()

    asyncio.run(run())
    loop.close()
    asyncio.run(run())


def test_wrong_thread_entry_subscription_callback_unsubscribe_and_exit():
    async def run():
        event = Notification()

        async def changed(value):
            raise AssertionError("wrong-thread delivery was accepted")

        async with AsyncEventScope(loop=asyncio.get_running_loop()) as scope:
            stop = scope.subscribe(event.subscribe, changed)
            errors = []

            def foreign():
                for action in (
                    lambda: asyncio.run(scope.__aenter__()),
                    lambda: scope.subscribe(event.subscribe, changed),
                    lambda: event.emit(1),
                    stop,
                    lambda: asyncio.run(scope.__aexit__(None, None, None)),
                ):
                    try:
                        action()
                    except RuntimeError as error:
                        errors.append(str(error))

            thread = threading.Thread(target=foreign)
            thread.start()
            thread.join(5)
            assert not thread.is_alive()
            assert len(errors) == 5
            assert all("owner OS thread" in message for message in errors)
            assert event.removals == 0

    asyncio.run(run())


def test_task_creation_failure_closes_coroutine_without_calling_handler():
    async def run():
        loop = asyncio.get_running_loop()
        event = Notification()
        captured = []
        calls = []

        def fail_factory(loop, coroutine, **kwargs):
            captured.append(coroutine)
            raise RuntimeError("factory failed")

        async def changed(value):
            calls.append(value)

        async with AsyncEventScope(loop=loop) as scope:
            scope.subscribe(event.subscribe, changed)
            loop.set_task_factory(fail_factory)
            try:
                with pytest.raises(RuntimeError, match="factory failed"):
                    event.emit(1)
            finally:
                loop.set_task_factory(None)
        assert len(captured) == 1 and captured[0].cr_frame is None
        assert calls == []

    asyncio.run(run())


def test_stopped_loop_does_not_run_handlers_until_application_resumes_it():
    loop = asyncio.new_event_loop()
    event = Notification()
    received = []
    entered = loop.create_future()
    finish = loop.create_future()

    async def changed(value):
        received.append(value)

    async def run():
        async with AsyncEventScope(loop=loop) as scope:
            scope.subscribe(event.subscribe, changed)
            entered.set_result(None)
            await finish

    try:
        task = loop.create_task(run())
        loop.run_until_complete(entered)
        with pytest.raises(RuntimeError, match="no running event loop"):
            event.emit(0)
        assert received == []
        loop.call_soon(event.emit, 1)
        loop.call_soon(finish.set_result, None)
        loop.run_until_complete(task)
        assert received == [1]
    finally:
        loop.close()


def test_handler_execution_rejects_loop_moved_to_another_os_thread():
    loop = asyncio.new_event_loop()
    event = Notification()
    received = []

    async def changed(value):
        received.append(value)

    async def run():
        async with AsyncEventScope(loop=loop) as scope:
            scope.subscribe(event.subscribe, changed)
            event.emit(1)
            await asyncio.Event().wait()

    def foreign():
        loop.call_soon(loop.stop)
        loop.run_forever()

    try:
        # Stop immediately after scheduling the handler, before it can run.
        task = loop.create_task(run())
        loop.call_soon(loop.stop)
        loop.run_forever()
        thread = threading.Thread(target=foreign)
        thread.start()
        thread.join(5)
        assert not thread.is_alive()
        assert received == []
        with pytest.raises(ExceptionGroup) as failure:
            loop.run_until_complete(task)
        assert "owner OS thread" in str(leaves(failure.value)[0])
        assert event.callback is None
    finally:
        loop.close()


def test_completed_scope_does_not_retain_handler_arguments():
    class Value:
        pass

    async def run():
        event = Notification()
        value = Value()
        reference = weakref.ref(value)

        async def changed(arg):
            await asyncio.sleep(0)

        async with AsyncEventScope(loop=asyncio.get_running_loop()) as scope:
            scope.subscribe(event.subscribe, changed)
            event.emit(value)
            del value
            assert reference() is not None
        gc.collect()
        assert reference() is None

    asyncio.run(run())
