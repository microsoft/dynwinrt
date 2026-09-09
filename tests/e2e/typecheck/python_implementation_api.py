# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Strict mypy consumer fixture; never executed as an E2E scenario."""

from collections.abc import Callable
from typing import Never, TypedDict
from uuid import UUID

from dynwinrt import DynWinRTImplementationHandle, DynWinRTValue, release_projected
from python_bindings.windows.application_model.background import (
    IBackgroundTask, IBackgroundTaskInstance,
)
from python_bindings.windows.foundation import (
    EventRegistrationToken, IClosable, IMemoryBufferReference, IPropertyValue, IStringable, Point,
)
from python_bindings.windows.globalization.number_formatting import INumberParser
from python_bindings.windows.storage.streams import IBuffer, IDataReader, IDataWriter
from python_bindings.windows.ui.xaml.interop import (
    IBindableIterable, IBindableIterator, IBindableVectorView,
)


def unused(*_args: object, **_kwargs: object) -> Never:
    raise NotImplementedError("Typecheck fixture only")


class InstanceHandlers:
    def get_instance_id(self) -> UUID:
        return UUID("243aab7d-9411-49af-b66d-0123456789ab")

    get_task = unused

    def get_progress(self) -> int:
        return 0

    def set_progress(self, value: int) -> None:
        pass

    get_trigger_details = unused
    add_canceled = unused
    remove_canceled = unused
    get_suspended_count = unused
    get_deferral = unused


class TaskHandlers:
    def run(self, task_instance: IBackgroundTaskInstance | None) -> None:
        if task_instance is not None:
            identifier: UUID = task_instance.instance_id
            task_instance.progress = len(str(identifier))

    def to_string(self) -> str:
        return "typed owner"

    def close(self) -> None:
        pass


def check_owner() -> None:
    handlers = TaskHandlers()
    instance: DynWinRTImplementationHandle[IBackgroundTaskInstance] = IBackgroundTaskInstance.implement(InstanceHandlers())
    owner: DynWinRTImplementationHandle[IBackgroundTask] = IBackgroundTask.implement(
        handlers, IStringable.implementation(handlers), IClosable.implementation(handlers)
    )
    task: IBackgroundTask = IBackgroundTask.from_implementation(owner)
    with IBackgroundTask.implement(handlers, interfaces=[(IStringable, handlers), (IClosable, handlers)]) as impl:
        primary: IBackgroundTask = impl.value
        primary.run(instance.value)
        impl.run(instance.value)  # type: ignore[attr-defined]
        impl.value = task  # type: ignore[misc]
    IBackgroundTask.implement(handlers, interfaces=[(IStringable, WrongText())])  # type: ignore[misc]
    task.run(IBackgroundTaskInstance.from_implementation(instance))
    text: IStringable = IStringable.from_implementation(owner)
    close: IClosable = IClosable.from_implementation(owner)
    value: str = text.to_string()
    close.close()
    release_projected(text)
    release_projected(close)
    raw: DynWinRTValue = owner.to_value()
    closed: bool = owner.is_closed
    error: str | None = owner.take_error()
    owner.release()
    owner.dispose()
    _: tuple[str, DynWinRTValue, bool, str | None] = (value, raw, closed, error)

    # Expected-error fixtures also fail if declarations become permissive Any.
    IBackgroundTaskInstance.implementation(handlers)  # type: ignore[arg-type]
    IStringable.from_implementation(raw)  # type: ignore[arg-type]


class WrongText:
    def to_string(self) -> int:
        return 17


class AsyncText:
    async def to_string(self) -> str:
        return "late"


def check_invalid_handlers() -> None:
    IStringable.implementation(WrongText())  # type: ignore[arg-type]
    IStringable.implementation(AsyncText())  # type: ignore[arg-type]


class NumberParserHandlers:
    def parse_int(self, text: str) -> int | None:
        return -(1 << 63)

    def parse_u_int(self, text: str) -> int | None:
        return (1 << 64) - 1

    def parse_double(self, text: str) -> float | None:
        return None


def check_nullable_reference_outputs() -> None:
    owner = INumberParser.implement(NumberParserHandlers())
    parser = INumberParser.from_implementation(owner)
    signed: int | None = parser.parse_int("signed")
    unsigned: int | None = parser.parse_u_int("unsigned")
    real: float | None = parser.parse_double("invalid")
    release_projected(parser)
    _: tuple[int | None, int | None, float | None] = (signed, unsigned, real)


ClosedDelegate = Callable[[IMemoryBufferReference | None, DynWinRTValue | None], None]


class ReferenceHandlers:
    def __init__(self, sender: IMemoryBufferReference) -> None:
        self.sender = sender

    def get_capacity(self) -> int:
        return 4096

    def add_closed(self, handler: ClosedDelegate | None) -> EventRegistrationToken:
        if handler is not None:
            handler(self.sender, None)
        return EventRegistrationToken(value=1)

    def remove_closed(self, token: EventRegistrationToken) -> None:
        value: int = token.value
        assert value == 1


def check_event(sender: IMemoryBufferReference) -> None:
    owner = IMemoryBufferReference.implement(
        ReferenceHandlers(sender), IClosable.implementation(TaskHandlers())
    )
    view: IMemoryBufferReference = IMemoryBufferReference.from_implementation(owner)

    def callback(source: IMemoryBufferReference | None, args: DynWinRTValue | None) -> None:
        if source is not None:
            capacity: int = source.capacity
            assert capacity >= 0

    unsubscribe: Callable[[], None] = view.subscribe_closed(callback)
    unsubscribe()


class ReaderHandlers:
    get_unconsumed_buffer_length = unused
    get_unicode_encoding = unused
    set_unicode_encoding = unused
    get_byte_order = unused
    set_byte_order = unused
    get_input_stream_options = unused
    set_input_stream_options = unused
    read_byte = unused
    read_buffer = unused
    read_boolean = unused
    read_guid = unused
    read_int16 = unused
    read_int32 = unused
    read_int64 = unused
    read_uint16 = unused
    read_uint32 = unused
    read_uint64 = unused
    read_single = unused
    read_double = unused
    read_string = unused
    read_date_time = unused
    read_time_span = unused
    load_async = unused
    detach_buffer = unused
    detach_stream = unused

    def read_bytes(self, capacity: int) -> bytes:
        return bytes(capacity)


class WriterHandlers:
    get_unstored_buffer_length = unused
    get_unicode_encoding = set_unicode_encoding = unused
    get_byte_order = set_byte_order = unused
    write_byte = write_buffer_range = unused
    write_boolean = write_guid = unused
    write_int16 = write_int32 = write_int64 = unused
    write_uint16 = write_uint32 = write_uint64 = unused
    write_single = write_double = unused
    write_date_time = write_time_span = unused
    write_string = measure_string = unused
    store_async = flush_async = detach_buffer = detach_stream = unused

    def write_bytes(self, value: list[int]) -> None:
        assert bytes(value) == b"\0\xff"

    def write_buffer(self, buffer: IBuffer | None) -> None:
        if buffer is not None:
            assert isinstance(buffer.to_bytes(), bytes)


class PropertyHandlers:
    get_type = get_is_numeric_scalar = unused
    get_uint8 = get_int16 = get_uint16 = get_int32 = get_uint32 = unused
    get_int64 = get_uint64 = get_single = get_double = unused
    get_char16 = get_boolean = get_string = get_guid = unused
    get_date_time = get_time_span = get_point = get_size = get_rect = unused
    get_int16_array = get_uint16_array = get_int32_array = get_uint32_array = unused
    get_int64_array = get_uint64_array = get_single_array = get_double_array = unused
    get_char16_array = get_boolean_array = get_guid_array = unused
    get_date_time_array = get_time_span_array = get_size_array = get_rect_array = unused

    def get_uint8_array(self) -> bytes:
        return b"\0\xff"

    def get_string_array(self) -> list[str]:
        return ["owned string"]

    def get_inspectable_array(self) -> list[DynWinRTValue | None]:
        return [None]

    def get_point_array(self) -> list[Point]:
        return [Point(x=1.25, y=-3.5)]


def check_arrays() -> None:
    reader_owner = IDataReader.implement(
        ReaderHandlers(), IClosable.implementation(TaskHandlers())
    )
    reader: IDataReader = IDataReader.from_implementation(reader_owner)
    filled: bytes = reader.read_bytes(bytearray(4))
    writer_owner = IDataWriter.implement(WriterHandlers(), IClosable.implementation(TaskHandlers()))
    writer: IDataWriter = IDataWriter.from_implementation(writer_owner)
    writer.write_bytes(filled)
    writer.write_buffer(IBuffer.from_bytes(filled))
    property_owner = IPropertyValue.implement(PropertyHandlers())
    value: IPropertyValue = IPropertyValue.from_implementation(property_owner)
    copied: bytes = value.get_uint8_array()
    strings: list[str] = value.get_string_array()
    points: list[Point] = value.get_point_array()
    assert copied and strings and points


class IndexOfResult(TypedDict):
    index: int
    result: bool


class VectorHandlers:
    def get_at(self, index: int) -> DynWinRTValue | None:
        return None

    def get_size(self) -> int:
        return 1

    def index_of(self, value: DynWinRTValue | None) -> IndexOfResult:
        return {"result": value is None, "index": 0}

    def first(self) -> IBindableIterator | None:
        return None


class TupleResultHandlers:
    get_at = unused
    get_size = unused

    def index_of(self, value: DynWinRTValue | None) -> tuple[int, bool]:
        return (0, True)


def check_named_outputs() -> None:
    handlers = VectorHandlers()
    owner = IBindableVectorView.implement(handlers, IBindableIterable.implementation(handlers))
    view: IBindableVectorView = IBindableVectorView.from_implementation(owner)
    result: tuple[int, bool] = view.index_of(DynWinRTValue.null_value())
    assert result == (0, True)
    IBindableVectorView.implementation(TupleResultHandlers())  # type: ignore[arg-type]
