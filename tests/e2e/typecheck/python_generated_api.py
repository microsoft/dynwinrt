# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import asyncio
from collections.abc import Callable, Coroutine, Generator, Sequence
from datetime import timedelta
from typing import Any, Awaitable, List, Tuple, assert_type

from dynwinrt import (
    DynWinRTArray,
    DynWinRtDelegate,
    WinRTAsync,
    WinRTAsyncWithProgress,
    WinRTCoroutine,
    WinRTCoroutineWithProgress,
    DynWinRTMethodSig,
    DynWinRTType,
    DynWinRTValue,
    WinGUID,
)
from python_bindings.windows.gaming.input import Gamepad
from python_bindings.windows.application_model.contacts import ContactDate
from python_bindings.windows.data.xml.dom import XmlDocument, XmlLoadSettings
from python_bindings.windows.foundation import (
    IReference_UInt32,
    IWwwFormUrlDecoderEntry,
    Uri,
)
from python_bindings.windows.foundation.collections import (
    CollectionChange,
    IMapChangedEventArgs_String,
    IObservableMap_String_Object,
    IObservableMap_String_String,
    PropertySet,
    StringMap,
    ValueSet,
)
from python_bindings.windows.globalization import Calendar
from python_bindings.windows.globalization.number_formatting import DecimalFormatter
from python_bindings.windows.storage import (
    IStorageItem,
    NameCollisionOption,
    StorageFile,
    StorageFolder,
)
from python_bindings.windows.storage.streams import (
    Buffer as WinRTBuffer,
    DataWriter,
    IBuffer,
    InMemoryRandomAccessStream,
    IOutputStream,
    RandomAccessStream,
)
from python_bindings.windows.system.threading import (
    ThreadPool,
    ThreadPoolTimer,
    WorkItemOptions,
    WorkItemPriority,
)


class StructuralAsyncAdapter:
    def __await__(self) -> Generator[None, None, int]:
        if False:
            yield None
        return self.wait()

    def wait(self) -> int:
        return 1

    def cancel(self) -> None:
        pass

    def release(self) -> None:
        pass


def check_structural_async_compatibility() -> None:
    adapter: WinRTAsync[int] = StructuralAsyncAdapter()
    awaitable: Awaitable[int] = adapter
    _: Tuple[WinRTAsync[int], Awaitable[int]] = (adapter, awaitable)


def check_runtime_stubs() -> None:
    iid: WinGUID = WinGUID.parse("00000000-0000-0000-c000-000000000046")
    interface: DynWinRTType = DynWinRTType.register_interface("IUnknown", iid)
    signature: DynWinRTMethodSig = DynWinRTMethodSig().add_out(
        DynWinRTType.hstring()
    )
    method = interface.add_method("GetName", signature).method(6)
    value: DynWinRTValue = DynWinRTValue.null_value()
    outputs: List[DynWinRTValue] = method.invoke_all(value, [])
    _: List[DynWinRTValue] = outputs


def check_uri() -> None:
    uri: Uri = Uri("https://example.com")
    relative: Uri = Uri("https://example.com/root/", "child")
    host: str = uri.host
    combined: Uri = uri.combine_uri("child")
    absolute: str = combined.absolute_uri
    _: Tuple[str, Uri, Uri, str] = (host, relative, combined, absolute)


def check_nullable_value(
    contact_date: ContactDate, legacy_day: IReference_UInt32
) -> None:
    day: int | None = contact_date.day
    month: int | None = contact_date.month
    year: int | None = contact_date.year
    _: Tuple[int | None, int | None, int | None] = (day, month, year)
    contact_date.day = 17
    contact_date.day = None
    contact_date.day = legacy_day


def check_string_vector(calendar: Calendar) -> None:
    languages: Sequence[str] = calendar.languages
    first: str = languages[0]
    located: int = languages.index(first)
    many: List[str] = list(languages[:4])
    _: Tuple[int, List[str]] = (located, many)


def check_object_vector(
    entries: Sequence[IWwwFormUrlDecoderEntry],
    entry: IWwwFormUrlDecoderEntry,
    buffer: DynWinRTArray,
) -> None:
    first: IWwwFormUrlDecoderEntry = entries[0]
    located: int = entries.index(entry)
    many: List[IWwwFormUrlDecoderEntry] = list(entries[:4])
    _: Tuple[
        IWwwFormUrlDecoderEntry,
        int,
        List[IWwwFormUrlDecoderEntry],
    ] = (first, located, many)


def check_async_types(
    writer: DataWriter,
    output: IOutputStream,
    buffer: IBuffer,
) -> None:
    store: WinRTCoroutine[int] = writer.store_async()
    legacy_store: WinRTAsync[int] = store
    awaitable: Awaitable[int] = store
    coroutine: Coroutine[Any, Any, int] = store
    task: asyncio.Task[int] = asyncio.create_task(store)
    write: WinRTCoroutineWithProgress[int, int] = output.write_async(buffer)
    legacy_write: WinRTAsyncWithProgress[int, int] = write
    write.progress(lambda value: value)
    progress_awaitable: Awaitable[int] = write
    progress_coroutine: Coroutine[Any, Any, int] = write
    progress_task: asyncio.Task[int] = asyncio.TaskGroup().create_task(write)
    _: Tuple[
        WinRTAsync[int],
        Awaitable[int],
        Coroutine[Any, Any, int],
        asyncio.Task[int],
        WinRTAsyncWithProgress[int, int],
        Awaitable[int],
        Coroutine[Any, Any, int],
        asyncio.Task[int],
    ] = (
        legacy_store,
        awaitable,
        coroutine,
        task,
        legacy_write,
        progress_awaitable,
        progress_coroutine,
        progress_task,
    )


def check_documented_overload_names(
    formatter: DecimalFormatter,
    calendar: Calendar,
    document: XmlDocument,
    settings: XmlLoadSettings,
) -> None:
    formatted: List[str] = [
        formatter.format(5),
        formatter.format(2.5),
        formatter.format_int(5),
        formatter.format_u_int(5),
        calendar.month_as_string(),
        calendar.month_as_string(3),
        calendar.month_as_full_string(),
    ]
    document.load_xml("<root />")
    document.load_xml("<root />", settings)
    document.load_xml_with_settings("<root />", settings)
    _: List[str] = formatted


async def check_documented_async_overload_names(
    file: StorageFile,
    folder: StorageFolder,
    source: InMemoryRandomAccessStream,
    target: InMemoryRandomAccessStream,
) -> None:
    option = NameCollisionOption.ReplaceExisting
    copies: List[StorageFile] = [
        await file.copy_async(folder),
        await file.copy_async(folder, "copy.txt"),
        await file.copy_async(folder, "copy.txt", option),
        await file.copy_overload(folder, "copy.txt", option),
    ]
    copied: List[int] = [
        await RandomAccessStream.copy_async(source, target),
        await RandomAccessStream.copy_async(source, target, 4),
        await RandomAccessStream.copy_size_async(source, target, 4),
    ]
    writer: DataWriter = DataWriter(source)
    _: Tuple[List[StorageFile], List[int], DataWriter] = (copies, copied, writer)


def check_ibuffer_bytes() -> None:
    interface_buffer: IBuffer = IBuffer.from_bytes(bytearray(b"\x00\xff"))
    runtime_buffer: WinRTBuffer = WinRTBuffer.from_bytes(b"\x01\x02")
    interface_bytes: bytes = interface_buffer.to_bytes()
    runtime_bytes: bytes = runtime_buffer.to_bytes()
    _: Tuple[bytes, bytes] = (interface_bytes, runtime_bytes)


async def check_output_nullability(folder: StorageFolder, values: ValueSet) -> None:
    created: StorageFile = await folder.create_file_async("notes.txt")
    names: List[str] = [
        item.name for item in await folder.get_files_async() if item is not None
    ]
    assert_type(folder.try_get_item_async("notes.txt"), WinRTCoroutine[IStorageItem | None])
    assert_type(values["key"], DynWinRTValue | None)
    _: Tuple[StorageFile, List[str]] = (created, names)


def check_map_changed_handlers(properties: PropertySet, strings: StringMap) -> None:
    def on_properties(
        sender: IObservableMap_String_Object, args: IMapChangedEventArgs_String
    ) -> None:
        size: int = len(sender)
        value: DynWinRTValue | None = sender[args.key]
        change: CollectionChange = args.collection_change
        _: Tuple[int, DynWinRTValue | None, CollectionChange] = (size, value, change)

    unsubscribe: Callable[[], None] = properties.subscribe_map_changed(on_properties)
    properties.once_map_changed(
        lambda sender, args: assert_type(sender, IObservableMap_String_Object)
    )
    token: DynWinRTValue = properties.on_map_changed(
        lambda sender, args: assert_type(args, IMapChangedEventArgs_String)
    )
    properties.off_map_changed(token)
    strings.subscribe_map_changed(
        lambda sender, args: assert_type(
            (sender, sender[args.key], args.collection_change),
            Tuple[IObservableMap_String_String, str, CollectionChange],
        )
    )
    unsubscribe()


def check_delegate_callback_parameters() -> None:
    inferred_work: WinRTCoroutine[None] = ThreadPool.run_async(
        lambda operation: assert_type(operation, DynWinRTValue)
    )

    def on_work_item(operation: DynWinRTValue) -> None:
        assert_type(operation, DynWinRTValue)

    work: WinRTCoroutine[None] = ThreadPool.run_async(on_work_item)
    priority_work: WinRTCoroutine[None] = ThreadPool.run_with_priority_async(
        lambda operation: assert_type(operation, DynWinRTValue),
        WorkItemPriority.Normal,
    )
    options_work: WinRTCoroutine[None] = ThreadPool.run_with_priority_and_options_async(
        lambda operation: assert_type(operation, DynWinRTValue),
        WorkItemPriority.Normal,
        WorkItemOptions.TimeSliced,
    )
    timer: ThreadPoolTimer | None = ThreadPoolTimer.create_timer(
        lambda elapsed: assert_type(elapsed.delay, timedelta),
        timedelta(milliseconds=1),
    )
    _: Tuple[
        WinRTCoroutine[None],
        WinRTCoroutine[None],
        WinRTCoroutine[None],
        WinRTCoroutine[None],
        ThreadPoolTimer | None,
    ] = (
        inferred_work,
        work,
        priority_work,
        options_work,
        timer,
    )


def check_native_delegate_inputs(
    native: DynWinRtDelegate,
    raw: DynWinRTValue,
    properties: PropertySet,
) -> None:
    token = properties.on_map_changed(native)
    properties.off_map_changed(token)
    properties.subscribe_map_changed(raw)()
    ThreadPool.run_async(native)
    ThreadPool.run_async(raw)
    ThreadPool.run_with_priority_async(native, WorkItemPriority.Normal)
    ThreadPool.run_with_priority_and_options_async(
        native, WorkItemPriority.Normal, WorkItemOptions.TimeSliced
    )
    static_token = Gamepad.add_gamepad_added(native)
    Gamepad.remove_gamepad_added(static_token)
