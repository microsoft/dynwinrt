# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

from typing import assert_type

from streams_generated import DataReader as RootReader, DataWriter as RootWriter
from streams_generated.windows.storage.streams import (
    DataReader,
    DataWriter,
    IBuffer,
    IInputStream,
    IOutputStream,
    InMemoryRandomAccessStream,
    InputStreamOptions,
)
from streams_generated.windows__storage__streams__data_reader import DataReaderLike
from streams_generated.windows__storage__streams__data_writer import DataWriterLike
from streams_generated.windows__storage__streams__i_data_reader import IDataReader
from streams_generated.windows__storage__streams__i_data_writer import IDataWriter


async def guarded(
    writer: DataWriter,
    reader: DataReader,
    writer_like: DataWriterLike,
    reader_like: DataReaderLike,
    writer_view: IDataWriter,
    reader_view: IDataReader,
) -> None:
    assert_type(writer, RootWriter)
    assert_type(reader, RootReader)
    for output in (
        writer.detach_stream(),
        writer_like.detach_stream(),
        writer_view.detach_stream(),
    ):
        assert_type(output, IOutputStream | None)
        if output is not None:
            assert_type(await output.flush_async(), bool)

    buffer = IBuffer.from_bytes(b"")
    for input in (
        reader.detach_stream(),
        reader_like.detach_stream(),
        reader_view.detach_stream(),
    ):
        assert_type(input, IInputStream | None)
        if input is not None:
            assert_type(await input.read_async(buffer, 0, InputStreamOptions.None_), IBuffer)

    with DataWriter() as unbound:
        output = unbound.detach_stream()
        assert_type(output, IOutputStream | None)
        if output is not None:
            await output.flush_async()
    with DataReader.from_buffer(buffer) as buffered:
        input = buffered.detach_stream()
        assert_type(input, IInputStream | None)
        if input is not None:
            await input.read_async(buffer, 0, InputStreamOptions.None_)

    assert_type(writer.detach_buffer(), IBuffer)
    assert_type(reader.detach_buffer(), IBuffer)
    assert_type(writer_view.detach_buffer(), IBuffer)
    assert_type(reader_view.detach_buffer(), IBuffer)
    assert_type(reader.read_buffer(0), IBuffer)
    assert_type(DataReader.from_buffer(buffer), DataReader)
    stream = InMemoryRandomAccessStream()
    assert_type(stream.get_output_stream_at(0), IOutputStream)
    assert_type(stream.get_input_stream_at(0), IInputStream)
