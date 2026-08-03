# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

from __future__ import annotations

import argparse
import asyncio
import importlib
import os
import sys
import threading
from datetime import timedelta
from pathlib import Path


def load_type(package: str, namespace: str, name: str):
    return getattr(importlib.import_module(f"{package}.{namespace}"), name)


def run(bindings_dir: Path, bootstrap_dll: Path, major: int, minor: int) -> None:
    os.environ["WINAPPSDK_BOOTSTRAP_DLL_PATH"] = str(bootstrap_dll)
    sys.path.insert(0, str(bindings_dir.parent))
    package = bindings_dir.name

    from dynwinrt_py import (
        RoApartment,
        get_winappsdk_resource_pri_path,
        has_package_identity,
        init_winappsdk,
    )

    Application = load_type(package, "microsoft.ui.xaml", "Application")
    Window = load_type(package, "microsoft.ui.xaml", "Window")
    Grid = load_type(package, "microsoft.ui.xaml.controls", "Grid")
    Button = load_type(package, "microsoft.ui.xaml.controls", "Button")
    TextBlock = load_type(package, "microsoft.ui.xaml.controls", "TextBlock")
    ButtonAutomationPeer = load_type(
        package, "microsoft.ui.xaml.automation.peers", "ButtonAutomationPeer"
    )
    DispatcherQueue = load_type(
        package, "microsoft.ui.dispatching", "DispatcherQueue"
    )
    ThreadPool = load_type(package, "windows.system.threading", "ThreadPool")

    context = init_winappsdk(major, minor)
    resource_pri = Path(get_winappsdk_resource_pri_path())
    if not resource_pri.is_file():
        raise RuntimeError(f"WinAppSDK resources.pri not found: {resource_pri}")

    state: dict[str, object] = {
        "clicked": False,
        "launched": False,
        "validated": False,
        "heartbeat": 0,
    }
    result: dict[str, object] = {}

    with RoApartment(0):
        try:
            result["packaged"] = has_package_identity()
            worker_stop = threading.Event()
            worker_started = threading.Event()
            queue_ready = threading.Event()

            async def worker_async() -> None:
                with RoApartment(1):
                    state["worker_thread_id"] = threading.get_ident()
                    worker_started.set()
                    while not queue_ready.is_set() and not worker_stop.is_set():
                        state["heartbeat"] = int(state["heartbeat"]) + 1
                        await asyncio.sleep(0.01)

                    if worker_stop.is_set():
                        return

                    def work_item(_operation: object) -> None:
                        state["work_item_thread_id"] = threading.get_ident()

                    await ThreadPool.run_async(work_item)
                    state["winrt_async_completed"] = True

                    queue = state.get("queue")
                    if not isinstance(queue, DispatcherQueue):
                        raise RuntimeError("Worker did not receive the DispatcherQueue")

                    def on_ui_thread() -> None:
                        state["enqueue_thread_id"] = threading.get_ident()
                        state["enqueue_ran"] = True

                    state["enqueue_accepted"] = queue.try_enqueue(on_ui_thread)
                    while not worker_stop.is_set():
                        state["heartbeat"] = int(state["heartbeat"]) + 1
                        await asyncio.sleep(0.01)

            worker_thread = threading.Thread(
                target=lambda: asyncio.run(worker_async()),
                name="winui-asyncio-worker",
            )
            worker_thread.start()
            if not worker_started.wait(1):
                raise RuntimeError("asyncio worker failed to start")
            state["heartbeat"] = 0

            def initialize(_params: object) -> None:
                def launched() -> None:
                    app = Application.get_current()
                    if app is None:
                        raise RuntimeError("Application.current is unavailable")
                    state["launched"] = True
                    state["app"] = app
                    state["ui_thread_id"] = threading.get_ident()

                    resources = app.resources
                    if resources is None or resources.merged_dictionaries is None:
                        raise RuntimeError("Application resources are unavailable")
                    state["fluent_resource_count"] = len(resources.merged_dictionaries)
                    if state["fluent_resource_count"] < 1:
                        raise RuntimeError("XamlControlsResources was not installed")

                    window = Window()
                    grid = Grid()
                    button = Button()
                    label = TextBlock()
                    label.text = "dynwinrt Python WinUI"
                    button.content = label

                    children = grid.children
                    if children is None:
                        raise RuntimeError("Grid.children is unavailable")
                    children.append(button)

                    def clicked(_sender: object, _args: object) -> None:
                        state["clicked"] = True
                        state["click_thread_id"] = threading.get_ident()
                        label.text = "Clicked"

                    state["click_token"] = button.on_click(clicked)
                    window.content = grid
                    window.activate()

                    queue = DispatcherQueue.get_for_current_thread()
                    if queue is None:
                        raise RuntimeError(
                            "DispatcherQueue is unavailable on the UI thread"
                        )
                    timer = queue.create_timer()
                    if timer is None:
                        raise RuntimeError("DispatcherQueueTimer creation failed")
                    timer.interval = timedelta(milliseconds=500)
                    timer.is_repeating = False

                    def tick(_sender: object, _args: object) -> None:
                        try:
                            ButtonAutomationPeer(button).invoke()
                            if not state["clicked"]:
                                raise RuntimeError("Button Click event was not raised")
                            if label.text != "Clicked":
                                raise RuntimeError(
                                    "Click callback did not update the TextBlock"
                                )
                            state["heartbeat_during_loop"] = (
                                int(state["heartbeat"])
                                - int(state["heartbeat_at_timer_start"])
                            )
                            if state["heartbeat_during_loop"] < 5:
                                raise RuntimeError(
                                    "asyncio worker was starved while WinUI owned "
                                    f"the UI thread: {state['heartbeat_during_loop']}"
                                )
                            if not state.get("enqueue_accepted"):
                                raise RuntimeError("DispatcherQueue rejected worker callback")
                            if not state.get("enqueue_ran"):
                                raise RuntimeError("Worker callback did not run on the UI thread")
                            if not state.get("winrt_async_completed"):
                                raise RuntimeError(
                                    "WinRT async operation did not complete on the asyncio worker"
                                )
                            if state.get("enqueue_thread_id") != state["ui_thread_id"]:
                                raise RuntimeError("DispatcherQueue callback ran on wrong thread")
                            if state.get("click_thread_id") != state["ui_thread_id"]:
                                raise RuntimeError("Click callback ran on wrong thread")
                            state["validated"] = True
                        except BaseException as error:
                            state["callback_error"] = repr(error)
                            raise
                        finally:
                            window.close()
                            app.exit()
                            queue.enqueue_event_loop_exit()

                    state["timer_token"] = timer.on_tick(tick)
                    state.update(
                        window=window,
                        grid=grid,
                        button=button,
                        label=label,
                        timer=timer,
                        queue=queue,
                    )
                    state["heartbeat_at_timer_start"] = state["heartbeat"]
                    queue_ready.set()
                    timer.start()

                state["app"] = Application.create(launched)

            Application.start(initialize)
            if not state["clicked"]:
                fallback_queue = state.get("queue")
                if not isinstance(fallback_queue, DispatcherQueue):
                    raise RuntimeError("DispatcherQueue was not initialized")
                try:
                    fallback_queue.run_event_loop()
                finally:
                    del fallback_queue

            if (
                not state["launched"]
                or not state["clicked"]
                or not state["validated"]
            ):
                raise RuntimeError(f"WinUI validation did not complete: {state}")
            result["resource_pri"] = str(resource_pri)
            result["fluent_resource_count"] = state["fluent_resource_count"]
            result["heartbeat_during_loop"] = state["heartbeat_during_loop"]
            result["ui_thread_id"] = state["ui_thread_id"]
            result["worker_thread_id"] = state["worker_thread_id"]
        finally:
            if "worker_stop" in locals():
                worker_stop.set()
            if "worker_thread" in locals():
                worker_thread.join(1)
            # Release every projected object before balancing RoInitialize,
            # including when setup or a callback fails.
            state.clear()

    del context
    print(f"python-winui-ok {result}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bindings-dir", type=Path, required=True)
    parser.add_argument("--bootstrap-dll", type=Path, required=True)
    parser.add_argument("--major", type=int, required=True)
    parser.add_argument("--minor", type=int, required=True)
    args = parser.parse_args()

    run(
        args.bindings_dir.resolve(),
        args.bootstrap_dll.resolve(),
        args.major,
        args.minor,
    )


if __name__ == "__main__":
    main()
