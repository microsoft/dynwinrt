# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Headless AsyncEventScope regressions against generated, real PropertySet APIs."""

import argparse
import asyncio
from contextvars import ContextVar
import gc
import importlib
from pathlib import Path
import sys
import tempfile
import threading
import warnings


async def check_native(g, dw, mode, *, temp_root):
    collections = importlib.import_module(f"{g.__name__}.windows.foundation.collections")
    owner = threading.current_thread()
    context = ContextVar("async-event-context")
    with tempfile.TemporaryDirectory(prefix="dynwinrt-event-test-", dir=temp_root) as directory:
        with dw.RoApartment(mode), dw.projected_lifetime_scope() as lifetime:
            folder = await g.StorageFolder.get_folder_from_path_async(directory)
            properties = g.PropertySet()
            observed = []
            retained = []
            context.set("subscription")

            async def changed(sender, args):
                assert threading.current_thread() is owner
                file = await folder.create_file_async(f"{args.key}.txt")
                assert Path(file.path).is_file()
                assert threading.current_thread() is owner
                assert context.get() == "subscription"
                assert not lifetime.disposed
                assert args.key in sender
                assert sender.size == 3
                observed.append(args.key)
                retained.append((sender, args))

            def register(callback):
                stop = properties.subscribe_map_changed(callback)
                properties["registration"] = None
                return stop

            async with dw.AsyncEventScope(loop=asyncio.get_running_loop()) as events:
                stop = events.subscribe(register, changed)
                context.set("body")
                properties["accepted"] = None
                stop()
                stop()
                properties["ignored"] = None
            assert sorted(observed) == ["accepted", "registration"]
            # Event args are not agile. Test the args, not just the agile sender.
            errors = []

            def foreign_read():
                with dw.RoApartment():
                    try:
                        _ = retained[0][1].key
                    except RuntimeError as error:
                        errors.append(str(error))

            thread = threading.Thread(target=foreign_read)
            thread.start()
            thread.join(5)
            assert not thread.is_alive() and len(errors) == 1, errors
            assert "thread" in errors[0], errors
            assert retained[0][1].key in properties

            # Native synchronous callbacks and already-owned Task results stay valid.
            synchronous = []
            stop = properties.subscribe_map_changed(
                lambda sender, args: synchronous.append(args.key)
            )
            properties["sync"] = None
            stop()
            assert synchronous == ["sync"]
            owned = []

            async def legacy_handler(sender, args):
                await asyncio.sleep(0)
                owned.append(args.key)

            async with asyncio.TaskGroup() as group:
                stop = properties.subscribe_map_changed(
                    lambda sender, args: group.create_task(legacy_handler(sender, args))
                )
                properties["owned-task"] = None
                stop()
            assert owned == ["owned-task"]

            raw = []
            delegate = dw.DynWinRtDelegate.create(
                collections.IID_MapChangedEventHandler_String_Object,
                collections.MapChangedEventHandler_String_Object_PARAM_TYPES,
                lambda sender, args: raw.append((sender, args)),
            )
            stop = properties.subscribe_map_changed(delegate)
            properties["raw"] = None
            stop()
            assert len(raw) == 1
            assert all(isinstance(value, dw.DynWinRTValue) for value in raw[0])
            delegate.release()

            called = []

            async def bare_handler(sender, args):
                called.append(args.key)

            with warnings.catch_warnings(record=True) as caught:
                warnings.simplefilter("always")
                stop = properties.subscribe_map_changed(bare_handler)
                properties["bare"] = None
                stop()
                gc.collect()
            assert called == []
            assert any("was never awaited" in str(w.message) for w in caught)

            loop = asyncio.get_running_loop()
            factory = loop.get_task_factory()
            unraisable = []
            rejected = []
            hook = sys.unraisablehook

            def fail_factory(loop, coroutine, **kwargs):
                rejected.append(coroutine)
                raise RuntimeError("native callback task creation failed")

            async with dw.AsyncEventScope(loop=loop) as events:
                events.subscribe(properties.subscribe_map_changed, bare_handler)
                sys.unraisablehook = lambda error: unraisable.append(str(error.exc_value))
                loop.set_task_factory(fail_factory)
                try:
                    try:
                        properties["task-creation"] = None
                    except OSError as error:
                        assert error.winerror & 0xFFFFFFFF == 0xA0EE4005
                finally:
                    loop.set_task_factory(factory)
                    sys.unraisablehook = hook
            assert unraisable == ["native callback task creation failed"]
            assert len(rejected) == 1 and rejected[0].cr_frame is None
            assert called == []

            for after_await in (False, True):
                started = asyncio.Event()
                cleaned = []

                async def failing(sender, args):
                    key = args.key
                    if key == "waiting":
                        try:
                            started.set()
                            await asyncio.Event().wait()
                        finally:
                            await asyncio.sleep(0)
                            assert args.key in sender
                            assert not lifetime.disposed
                            cleaned.append(key)
                    else:
                        if after_await:
                            await asyncio.sleep(0)
                        raise ValueError("native event handler failed")

                try:
                    async with dw.AsyncEventScope(loop=asyncio.get_running_loop()) as events:
                        events.subscribe(properties.subscribe_map_changed, failing)
                        properties["waiting"] = None
                        await started.wait()
                        properties["failure"] = None
                        await asyncio.Event().wait()
                except ExceptionGroup as error:
                    assert len(error.exceptions) == 1
                    assert str(error.exceptions[0]) == "native event handler failed"
                else:
                    raise AssertionError("handler failure was lost")
                assert cleaned == ["waiting"]
                properties["after-failure"] = None

            started = asyncio.Event()
            cleaned = []

            async def cancellable(sender, args):
                try:
                    started.set()
                    await asyncio.Event().wait()
                finally:
                    await asyncio.sleep(0)
                    assert args.key in sender and not lifetime.disposed
                    cleaned.append(args.key)

            async def parent():
                async with dw.AsyncEventScope(loop=asyncio.get_running_loop()) as events:
                    events.subscribe(properties.subscribe_map_changed, cancellable)
                    properties["cancel"] = None
                    await asyncio.Event().wait()

            task = asyncio.create_task(parent())
            await started.wait()
            task.cancel()
            try:
                await task
            except asyncio.CancelledError:
                pass
            else:
                raise AssertionError("external cancellation was lost")
            assert cleaned == ["cancel"]

            reentered = asyncio.Event()
            keys = []

            async def reentrant(sender, args):
                # PropertySet rejects mutation inside its native notification.
                # This yield is also needed with an eager task factory.
                await asyncio.sleep(0)
                if args.key == "outer":
                    properties["inner"] = None
                    await reentered.wait()
                else:
                    reentered.set()
                keys.append(args.key)

            async with dw.AsyncEventScope(loop=asyncio.get_running_loop()) as events:
                events.subscribe(properties.subscribe_map_changed, reentrant)
                properties["outer"] = None
                await reentered.wait()
            assert keys == ["inner", "outer"]

            # Context captured by the generated delegate must not be cleared by
            # the helper to make early native-owner disposal appear safe.
            captured = []
            inner = dw.projected_lifetime_scope()
            with inner:
                captured_folder = await g.StorageFolder.get_folder_from_path_async(directory)
                async with dw.AsyncEventScope(loop=asyncio.get_running_loop()) as events:
                    async def observe_scope(sender, args):
                        await asyncio.sleep(0)
                        captured.append(captured_folder.path)
                    events.subscribe(properties.subscribe_map_changed, observe_scope)
                    properties["lifetime"] = None
            assert len(captured) == 1, captured
            assert Path(captured[0]).samefile(directory), (captured[0], directory)
            try:
                _ = captured_folder.path
            except RuntimeError as error:
                assert "released" in str(error)
            else:
                raise AssertionError("projection scope was bypassed")

            try:
                async with dw.AsyncEventScope(loop=loop) as events:
                    with dw.projected_lifetime_scope():
                        early_folder = await g.StorageFolder.get_folder_from_path_async(directory)

                        async def invalid_lifetime(sender, args):
                            await asyncio.sleep(0)
                            _ = early_folder.path

                    events.subscribe(properties.subscribe_map_changed, invalid_lifetime)
                    properties["early-release"] = None
            except ExceptionGroup as error:
                assert len(error.exceptions) == 1
                assert "released" in str(error.exceptions[0])
            else:
                raise AssertionError("AsyncEventScope extended a released projection lifetime")
        try:
            _ = retained[0][1].key
        except RuntimeError as error:
            assert "released" in str(error) or "initialized" in str(error)
        else:
            raise AssertionError("event arguments survived final apartment release")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--generated", required=True)
    parser.add_argument("--mode", type=int, choices=(0, 1), required=True)
    options = parser.parse_args()
    generated = Path(options.generated).resolve()
    sys.path.insert(0, str(generated.parent))
    g = importlib.import_module(generated.name)
    import dynwinrt as dw

    async def run():
        loop = asyncio.get_running_loop()
        with tempfile.TemporaryDirectory(prefix="dynwinrt-event-root-") as root:
            # Exercise native path normalization without requiring NTFS 8.3 names.
            temp_root = str(Path(root) / ".." / Path(root).name)
            await check_native(g, dw, options.mode, temp_root=temp_root)
            if hasattr(asyncio, "eager_task_factory"):
                loop.set_task_factory(asyncio.eager_task_factory)
                try:
                    await check_native(g, dw, options.mode, temp_root=temp_root)
                finally:
                    loop.set_task_factory(None)

    asyncio.run(run())
    print(f"PASS async-events apartment={options.mode}")


if __name__ == "__main__":
    main()
