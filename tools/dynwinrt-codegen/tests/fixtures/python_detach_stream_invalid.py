# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

from streams_generated import DataReader as RootReader, DataWriter as RootWriter
from streams_generated.windows.storage.streams import (
    DataReader,
    DataWriter,
    IBuffer,
    InputStreamOptions,
)
from streams_generated.windows__storage__streams__data_reader import DataReaderLike
from streams_generated.windows__storage__streams__data_writer import DataWriterLike
from streams_generated.windows__storage__streams__i_data_reader import IDataReader
from streams_generated.windows__storage__streams__i_data_writer import IDataWriter


def unguarded(
    writer: RootWriter,
    reader: RootReader,
    writer_like: DataWriterLike,
    reader_like: DataReaderLike,
    writer_view: IDataWriter,
    reader_view: IDataReader,
) -> None:
    buffer = IBuffer.from_bytes(b"")
    with DataWriter() as unbound:
        unbound.detach_stream().flush_async()  # unsafe
    with DataReader.from_buffer(buffer) as buffered:
        buffered.detach_stream().read_async(buffer, 0, InputStreamOptions.None_)  # unsafe
    writer.detach_stream().flush_async()  # unsafe
    reader.detach_stream().read_async(buffer, 0, InputStreamOptions.None_)  # unsafe
    writer_like.detach_stream().flush_async()  # unsafe
    reader_like.detach_stream().read_async(buffer, 0, InputStreamOptions.None_)  # unsafe
    writer_view.detach_stream().flush_async()  # unsafe
    reader_view.detach_stream().read_async(buffer, 0, InputStreamOptions.None_)  # unsafe
