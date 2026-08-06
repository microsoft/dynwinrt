# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

from __future__ import annotations

import argparse
import asyncio
import importlib
import os
import sys
import threading
from contextvars import ContextVar
from datetime import timedelta
from pathlib import Path


def load_type(package: str, namespace: str, name: str):
    return getattr(importlib.import_module(f"{package}.{namespace}"), name)


def run(bindings_dir: Path, bootstrap_dll: Path, major: int, minor: int) -> None:
    os.environ["WINAPPSDK_BOOTSTRAP_DLL_PATH"] = str(bootstrap_dll)
    sys.path.insert(0, str(bindings_dir.parent))
    package = bindings_dir.name

    from dynwinrt_py import (
        DynWinRTType,
        DynWinRTValue,
        RoApartment,
        WinGUID,
        get_winappsdk_resource_pri_path,
        has_package_identity,
        init_winappsdk,
        projected_lifetime_scope,
    )

    Application = load_type(package, "microsoft.ui.xaml", "Application")
    Window = load_type(package, "microsoft.ui.xaml", "Window")
    XamlReader = load_type(package, "microsoft.ui.xaml.markup", "XamlReader")
    Grid = load_type(package, "microsoft.ui.xaml.controls", "Grid")
    Button = load_type(package, "microsoft.ui.xaml.controls", "Button")
    TextBlock = load_type(package, "microsoft.ui.xaml.controls", "TextBlock")
    CommandBar = load_type(package, "microsoft.ui.xaml.controls", "CommandBar")
    AppBarButton = load_type(
        package, "microsoft.ui.xaml.controls", "AppBarButton"
    )
    ItemsRepeater = load_type(
        package, "microsoft.ui.xaml.controls", "ItemsRepeater"
    )
    ListView = load_type(package, "microsoft.ui.xaml.controls", "ListView")
    StackLayout = load_type(
        package, "microsoft.ui.xaml.controls", "StackLayout"
    )
    StackPanel = load_type(
        package, "microsoft.ui.xaml.controls", "StackPanel"
    )
    IElementFactory = load_type(
        package, "microsoft.ui.xaml", "IElementFactory"
    )
    ElementFactoryGetArgs = load_type(
        package, "microsoft.ui.xaml", "ElementFactoryGetArgs"
    )
    ElementFactoryRecycleArgs = load_type(
        package, "microsoft.ui.xaml", "ElementFactoryRecycleArgs"
    )
    ButtonAutomationPeer = load_type(
        package, "microsoft.ui.xaml.automation.peers", "ButtonAutomationPeer"
    )
    DispatcherQueue = load_type(
        package, "microsoft.ui.dispatching", "DispatcherQueue"
    )
    DispatcherQueuePriority = load_type(
        package, "microsoft.ui.dispatching", "DispatcherQueuePriority"
    )
    ThreadPool = load_type(package, "windows.system.threading", "ThreadPool")
    CollectionChange = load_type(
        package, "windows.foundation.collections", "CollectionChange"
    )
    ApplicationTheme = load_type(
        package, "microsoft.ui.xaml", "ApplicationTheme"
    )
    ElementTheme = load_type(package, "microsoft.ui.xaml", "ElementTheme")

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

    with RoApartment(0), projected_lifetime_scope():
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

                    def on_ui_thread() -> int:
                        state["enqueue_thread_id"] = threading.get_ident()
                        state["enqueue_ran"] = True
                        return state["enqueue_thread_id"]

                    state["enqueue_thread_id"] = await queue.enqueue_async(
                        on_ui_thread
                    )
                    state["enqueue_accepted"] = True

                    def fail_on_ui_thread() -> None:
                        raise RuntimeError("dispatcher callback failed")

                    try:
                        await queue.enqueue_async(fail_on_ui_thread)
                    except RuntimeError as error:
                        state["enqueue_error"] = str(error)
                    else:
                        raise RuntimeError(
                            "DispatcherQueue callback error was not propagated"
                        )

                    state["priority_enqueue_thread_id"] = (
                        await queue.enqueue_with_priority_async(
                            DispatcherQueuePriority.High,
                            threading.get_ident,
                        )
                    )
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
                    controls_resources = resources.merged_dictionaries[0]
                    theme_dictionaries = controls_resources.theme_dictionaries
                    if theme_dictionaries is None:
                        raise RuntimeError(
                            "XamlControlsResources theme dictionaries are unavailable"
                        )
                    property_value_iid = WinGUID.parse(
                        "4BD682DD-7554-40E9-9A9B-82654EDE7E62"
                    )
                    theme_names = set()
                    for key in theme_dictionaries:
                        if key is None:
                            continue
                        try:
                            name = (
                                key.cast(property_value_iid)
                                .call_0(19, DynWinRTType.hstring())
                                .to_string()
                            )
                        except (OSError, RuntimeError):
                            continue
                        theme_names.add(name)
                    required_themes = {"Default", "Light", "HighContrast"}
                    if not required_themes.issubset(theme_names):
                        raise RuntimeError(
                            "Fluent theme dictionaries are incomplete: "
                            f"{sorted(theme_names)!r}"
                        )
                    state["fluent_theme_names"] = sorted(theme_names)
                    if app.requested_theme != ApplicationTheme.Dark:
                        raise RuntimeError(
                            "Application requested theme was not applied"
                        )

                    window = Window()
                    grid = Grid()
                    grid.requested_theme = ElementTheme.Light
                    button = Button()
                    label = TextBlock()
                    command_bar = CommandBar()
                    app_bar_button = AppBarButton()
                    repeater = ItemsRepeater()
                    repeater.layout = StackLayout()
                    source_list = ListView()
                    exact_stack_panel = StackPanel()

                    override_context = ContextVar(
                        "dynwinrt_winui_override_context", default="missing"
                    )
                    context_token = override_context.set("captured-at-construction")

                    class PythonStackPanel(StackPanel):
                        def measure_override(
                            self, available_size: tuple[float, float]
                        ) -> tuple[float, float]:
                            state["measure_override_count"] = (
                                int(state.get("measure_override_count", 0)) + 1
                            )
                            state["measure_override_thread_id"] = threading.get_ident()
                            state["measure_override_context"] = override_context.get()
                            if (
                                not isinstance(available_size, tuple)
                                or len(available_size) != 2
                            ):
                                raise RuntimeError(
                                    "MeasureOverride did not receive a Size tuple"
                                )
                            return available_size

                    class MarkupPythonStackPanel(StackPanel):
                        def measure_override(
                            self, available_size: tuple[float, float]
                        ) -> tuple[float, float]:
                            state["markup_measure_override_count"] = (
                                int(state.get("markup_measure_override_count", 0)) + 1
                            )
                            state["markup_measure_override_thread_id"] = (
                                threading.get_ident()
                            )
                            return available_size

                    markup_registration = StackPanel.register_xaml_runtime_class(
                        "DynWinRT.Tests.MarkupPythonStackPanel",
                        MarkupPythonStackPanel,
                    )
                    state["markup_registration"] = markup_registration
                    metadata_provider = app._obj.cast(
                        WinGUID.parse(
                            "a96251f0-2214-5d53-8746-ce99a2593cd7"
                        )
                    )
                    registered_xaml_type = metadata_provider.call_1(
                        7,
                        DynWinRTType.interface(
                            WinGUID.parse(
                                "d24219df-7ec9-57f1-a27b-6af251d9c5bc"
                            )
                        ),
                        DynWinRTValue.from_hstring(
                            "DynWinRT.Tests.MarkupPythonStackPanel"
                        ),
                    )
                    if registered_xaml_type.is_null():
                        raise RuntimeError(
                            "Application metadata provider did not expose the registered type"
                        )
                    markup_value = XamlReader.load(
                        """
                        <local:MarkupPythonStackPanel
                            xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
                            xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
                            xmlns:local="using:DynWinRT.Tests">
                            <TextBlock Text="Created by XamlReader" />
                        </local:MarkupPythonStackPanel>
                        """
                    )
                    if markup_value is None:
                        raise RuntimeError(
                            "XamlReader returned null for the registered Python control"
                        )
                    markup_stack_panel = StackPanel(markup_value)
                    if len(markup_stack_panel.children) != 1:
                        raise RuntimeError(
                            "Registered Python control did not inherit StackPanel content metadata"
                        )
                    if (
                        markup_stack_panel._obj.identity_raw()
                        != markup_value.identity_raw()
                    ):
                        raise RuntimeError(
                            "XAML-created Python control lost controlling COM identity"
                        )
                    state["markup_constructed"] = True

                    class FailingMarkupStackPanel(StackPanel):
                        def __init__(self):
                            raise RuntimeError("registered constructor failed")

                    failing_registration = StackPanel.register_xaml_runtime_class(
                        "DynWinRT.Tests.FailingMarkupStackPanel",
                        FailingMarkupStackPanel,
                    )
                    unraisable_errors = []
                    previous_unraisablehook = sys.unraisablehook
                    sys.unraisablehook = unraisable_errors.append
                    try:
                        XamlReader.load(
                            """
                            <local:FailingMarkupStackPanel
                                xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
                                xmlns:local="using:DynWinRT.Tests" />
                            """
                        )
                    except OSError:
                        state["markup_callback_error_propagated"] = True
                    else:
                        raise RuntimeError(
                            "XAML activation hid the Python constructor failure"
                        )
                    finally:
                        sys.unraisablehook = previous_unraisablehook
                        failing_registration.unregister()
                    if (
                        len(unraisable_errors) != 1
                        or "registered constructor failed"
                        not in str(unraisable_errors[0].exc_value)
                    ):
                        raise RuntimeError(
                            "Python XAML constructor error was not reported through unraisablehook"
                        )

                    composed_stack_panel = PythonStackPanel()
                    override_context.reset(context_token)
                    override_context.set("changed-after-construction")
                    composed_stack_panel.spacing = 3.0
                    composed_child = TextBlock()
                    composed_child.text = "Composed Python subclass"
                    composed_stack_panel.children.append(composed_child)
                    if (
                        composed_stack_panel.spacing != 3.0
                        or len(composed_stack_panel.children) != 1
                    ):
                        raise RuntimeError(
                            "Python StackPanel subclass composition was not functional"
                        )
                    if len(exact_stack_panel.children) != 0:
                        raise RuntimeError(
                            "Exact StackPanel construction changed unexpectedly"
                        )

                    class UnsupportedOverrideStackPanel(StackPanel):
                        def go_to_element_state_core(
                            self, state_name: str, use_transitions: bool
                        ) -> bool:
                            return False

                    try:
                        UnsupportedOverrideStackPanel()
                    except TypeError as error:
                        if "native override ABI is unsupported" not in str(error):
                            raise RuntimeError(
                                "Native override rejection was unclear"
                            ) from error
                    else:
                        raise RuntimeError(
                            "Native WinRT override subclass was not rejected"
                        )
                    state["composition_constructed"] = True

                    repeater.width = 240.0
                    repeater.height = 80.0
                    app_bar_button.label = "Observable"
                    label.text = "dynwinrt Python WinUI"
                    button.content = label

                    children = grid.children
                    if children is None:
                        raise RuntimeError("Grid.children is unavailable")
                    commands = command_bar.primary_commands
                    if commands is None:
                        raise RuntimeError(
                            "CommandBar.primary_commands is unavailable"
                        )

                    vector_changes = []

                    def vector_changed(sender: object, args: object) -> None:
                        vector_changes.append(
                            (sender, args.collection_change, args.index)
                        )

                    unsubscribe_vector = commands.subscribe_vector_changed(
                        vector_changed
                    )
                    commands.append(app_bar_button)
                    if len(commands) != 1:
                        raise RuntimeError(
                            "Observable vector mutation was not visible"
                        )
                    if len(vector_changes) != 1:
                        raise RuntimeError(
                            "Observable vector change event was not raised"
                        )
                    sender, change, index = vector_changes[0]
                    if sender is None or change != CollectionChange.ItemInserted:
                        raise RuntimeError(
                            "Observable vector event projection was incorrect"
                        )
                    if index != 0:
                        raise RuntimeError(
                            f"Observable vector reported wrong index: {index}"
                        )
                    unsubscribe_vector()
                    commands.clear()
                    if len(vector_changes) != 1:
                        raise RuntimeError(
                            "Observable vector unsubscribe was not effective"
                        )
                    state["observable_vector_validated"] = True

                    factory_elements = []
                    recycled_elements = []

                    def get_element(_args: object) -> object:
                        element = TextBlock()
                        element.text = "Factory generated"
                        factory_elements.append(element)
                        state["factory_get_thread_id"] = threading.get_ident()
                        return element

                    def recycle_element(args: object) -> None:
                        args.element = args.element
                        recycled_elements.append(args.element)
                        state["factory_recycle_thread_id"] = (
                            threading.get_ident()
                        )

                    element_factory = IElementFactory.create(
                        get_element,
                        recycle_element,
                    )
                    manual_element = element_factory.get_element(
                        ElementFactoryGetArgs()
                    )
                    if manual_element is None or len(factory_elements) != 1:
                        raise RuntimeError(
                            "Direct IElementFactory get bridge failed"
                        )
                    manual_recycle_args = ElementFactoryRecycleArgs()
                    manual_recycle_args.element = manual_element
                    element_factory.recycle_element(manual_recycle_args)
                    if (
                        len(recycled_elements) != 1
                        or recycled_elements[0] is not factory_elements[0]
                    ):
                        raise RuntimeError(
                            "Direct IElementFactory recycle identity failed"
                        )
                    factory_elements.clear()
                    recycled_elements.clear()

                    source_value = DynWinRTValue.box_reference(
                        DynWinRTValue.from_hstring("Factory source"),
                        DynWinRTType.hstring(),
                    )
                    items_source = source_list.items
                    if items_source is None:
                        raise RuntimeError("ListView.items is unavailable")
                    items_source.append(source_value)
                    item_template_property = (
                        ItemsRepeater.get_item_template_property()
                    )
                    if item_template_property is None:
                        raise RuntimeError(
                            "ItemsRepeater item template property is unavailable"
                        )
                    repeater.set_value(
                        item_template_property,
                        element_factory._obj,
                    )
                    repeater.items_source = items_source
                    assigned_template = repeater.item_template
                    if assigned_template is None:
                        raise RuntimeError(
                            "ItemsRepeater.item_template was not assigned"
                        )
                    IElementFactory.from_value(assigned_template)
                    source_view = repeater.items_source_view
                    if source_view is None or source_view.count != 1:
                        raise RuntimeError(
                            "ItemsRepeater source view did not expose one item"
                        )

                    children.append(command_bar)
                    children.append(repeater)
                    children.append(button)
                    children.append(composed_stack_panel)
                    children.append(markup_stack_panel)

                    def clicked(_sender: object, _args: object) -> None:
                        state["clicked"] = True
                        state["click_thread_id"] = threading.get_ident()
                        label.text = "Clicked"

                    state["click_token"] = button.on_click(clicked)
                    window.content = grid
                    window.activate()
                    state["factory_realized_element"] = (
                        repeater.get_or_create_element(0)
                    )

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
                        if not state.get("factory_recycle_requested"):
                            try:
                                if int(state.get("measure_override_count", 0)) < 1:
                                    raise RuntimeError(
                                        "WinUI did not invoke Python MeasureOverride"
                                    )
                                if (
                                    int(
                                        state.get(
                                            "markup_measure_override_count", 0
                                        )
                                    )
                                    < 1
                                ):
                                    raise RuntimeError(
                                        "XAML-created control did not invoke Python MeasureOverride"
                                    )
                                if (
                                    state.get(
                                        "markup_measure_override_thread_id"
                                    )
                                    != state["ui_thread_id"]
                                ):
                                    raise RuntimeError(
                                        "XAML-created Python override ran outside the WinUI apartment"
                                    )
                                if (
                                    state.get("measure_override_thread_id")
                                    != state["ui_thread_id"]
                                ):
                                    raise RuntimeError(
                                        "MeasureOverride ran outside the WinUI apartment"
                                    )
                                if (
                                    state.get("measure_override_context")
                                    != "captured-at-construction"
                                ):
                                    raise RuntimeError(
                                        "MeasureOverride lost its captured ContextVar context"
                                    )
                                state["composition_validated"] = True
                                if not factory_elements:
                                    raise RuntimeError(
                                        "IElementFactory get callback did not run"
                                    )
                                if (
                                    state.get("factory_get_thread_id")
                                    != state["ui_thread_id"]
                                ):
                                    raise RuntimeError(
                                        "IElementFactory get callback ran on wrong thread"
                                    )
                                if grid.actual_theme != ElementTheme.Light:
                                    raise RuntimeError(
                                        "Element Light theme was not applied"
                                    )
                                grid.requested_theme = ElementTheme.Dark
                                state["factory_recycle_requested"] = True
                                repeater.items_source = (
                                    DynWinRTValue.null_value()
                                )
                                timer.interval = timedelta(milliseconds=200)
                                timer.start()
                                return
                            except BaseException as error:
                                state["callback_error"] = repr(error)
                                window.close()
                                app.exit()
                                queue.enqueue_event_loop_exit()
                                raise
                        try:
                            if len(recycled_elements) != len(factory_elements):
                                raise RuntimeError(
                                    "IElementFactory did not recycle every element"
                                )
                            if grid.actual_theme != ElementTheme.Dark:
                                raise RuntimeError(
                                    "Element Dark theme was not applied"
                                )
                            if any(
                                not any(
                                    recycled is created
                                    for recycled in recycled_elements
                                )
                                for created in factory_elements
                            ):
                                raise RuntimeError(
                                    "IElementFactory did not preserve projected identity"
                                )
                            if (
                                state.get("factory_recycle_thread_id")
                                != state["ui_thread_id"]
                            ):
                                raise RuntimeError(
                                    "IElementFactory recycle callback ran on wrong thread"
                                )
                            element_factory.release_callbacks()
                            state["element_factory_validated"] = True

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
                            if state.get("enqueue_error") != "dispatcher callback failed":
                                raise RuntimeError(
                                    "DispatcherQueue callback error was not preserved"
                                )
                            if not state.get("winrt_async_completed"):
                                raise RuntimeError(
                                    "WinRT async operation did not complete on the asyncio worker"
                                )
                            if not state.get("observable_vector_validated"):
                                raise RuntimeError(
                                    "Observable vector validation did not complete"
                                )
                            if not state.get("element_factory_validated"):
                                raise RuntimeError(
                                    "Element factory validation did not complete"
                                )
                            markup_registration = state.get(
                                "markup_registration"
                            )
                            if markup_registration is not None:
                                markup_registration.unregister()
                                if markup_registration.release_instances() < 1:
                                    raise RuntimeError(
                                        "Registered XAML Python instance was not retained"
                                    )
                            if not state.get("composition_validated"):
                                raise RuntimeError(
                                    "Composable subclass validation did not complete"
                                )
                            if not state.get("markup_constructed"):
                                raise RuntimeError(
                                    "Registered XAML control was not constructed"
                                )
                            if not state.get("markup_callback_error_propagated"):
                                raise RuntimeError(
                                    "Registered XAML constructor errors were not propagated"
                                )
                            if state.get("enqueue_thread_id") != state["ui_thread_id"]:
                                raise RuntimeError("DispatcherQueue callback ran on wrong thread")
                            if (
                                state.get("priority_enqueue_thread_id")
                                != state["ui_thread_id"]
                            ):
                                raise RuntimeError(
                                    "Priority DispatcherQueue callback ran on wrong thread"
                                )
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
                        repeater=repeater,
                        element_factory=element_factory,
                        factory_elements=factory_elements,
                        recycled_elements=recycled_elements,
                        items_source=items_source,
                        source_value=source_value,
                        source_list=source_list,
                        source_view=source_view,
                    )
                    state["heartbeat_at_timer_start"] = state["heartbeat"]
                    queue_ready.set()
                    timer.start()

                created_app = Application.create(launched)
                created_app.requested_theme = ApplicationTheme.Dark
                state["app"] = created_app

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
            result["measure_override_count"] = state["measure_override_count"]
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
