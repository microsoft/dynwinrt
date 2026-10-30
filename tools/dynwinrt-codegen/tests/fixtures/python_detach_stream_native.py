# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

from dynwinrt import RoApartment, projected_lifetime_scope
from streams_generated.windows.storage.streams import (
    DataReader,
    DataWriter,
    IBuffer,
    InMemoryRandomAccessStream,
)
from streams_generated.windows__storage__streams__i_data_reader import IDataReader
from streams_generated.windows__storage__streams__i_data_writer import IDataWriter


with RoApartment(), projected_lifetime_scope():
    with DataWriter() as writer:
        for view in (writer, writer.as_interface(IDataWriter)):
            assert view.detach_stream() is None
            assert view.detach_stream() is None
    with DataReader.from_buffer(IBuffer.from_bytes(b"reader")) as reader:
        for view in (reader, reader.as_interface(IDataReader)):
            assert view.detach_stream() is None
            assert view.detach_stream() is None
    print("unbound writer and buffer-backed reader: class/interface return None")

    for use_interface in (False, True):
        with InMemoryRandomAccessStream() as stream:
            output = stream.get_output_stream_at(0)
            with DataWriter(output) as writer:
                view = writer.as_interface(IDataWriter) if use_interface else writer
                detached = view.detach_stream()
                assert detached is not None
                assert detached._obj.identity_raw() == output._obj.identity_raw()
                assert view.detach_stream() is None
                assert writer.detach_stream() is None

            input = stream.get_input_stream_at(0)
            with DataReader(input) as reader:
                view = reader.as_interface(IDataReader) if use_interface else reader
                detached = view.detach_stream()
                assert detached is not None
                assert detached._obj.identity_raw() == input._obj.identity_raw()
                assert view.detach_stream() is None
                assert reader.detach_stream() is None
        print(f"attached class/interface={use_interface}: original stream, then None")

print("native DetachStream states passed; references released before apartment exit")
