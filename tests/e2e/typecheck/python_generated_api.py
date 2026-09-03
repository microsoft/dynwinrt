# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import asyncio
from collections.abc import Coroutine, Generator, Sequence
from typing import Any, Awaitable, List, Tuple

from dynwinrt import (
    DynWinRTArray,
    WinRTAsync,
    WinRTAsyncWithProgress,
    WinRTCoroutine,
    WinRTCoroutineWithProgress,
    DynWinRTMethodSig,
    DynWinRTType,
    DynWinRTValue,
    WinGUID,
)
from python_bindings.windows.application_model.contacts import ContactDate
from python_bindings.windows.foundation import (
    IReference_UInt32,
    IWwwFormUrlDecoderEntry,
    Uri,
)
from python_bindings.windows.globalization import Calendar
from python_bindings.windows.storage.streams import (
    Buffer as WinRTBuffer,
    DataWriter,
    IBuffer,
    IOutputStream,
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
    combined: Uri | None = uri.combine_uri("child")
    _: Tuple[str, Uri, Uri | None] = (host, relative, combined)


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
    languages: Sequence[str] | None = calendar.languages
    assert languages is not None
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


def check_ibuffer_bytes() -> None:
    interface_buffer: IBuffer = IBuffer.from_bytes(bytearray(b"\x00\xff"))
    runtime_buffer: WinRTBuffer = WinRTBuffer.from_bytes(b"\x01\x02")
    interface_bytes: bytes = interface_buffer.to_bytes()
    runtime_bytes: bytes = runtime_buffer.to_bytes()
    _: Tuple[bytes, bytes] = (interface_bytes, runtime_bytes)
