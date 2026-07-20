# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

from typing import List, Tuple

from dynwinrt_py import (
    DynWinRTArray,
    DynWinRTMethodSig,
    DynWinRTType,
    DynWinRTValue,
    WinGUID,
)
from python_bindings.calendar import Calendar
from python_bindings.i_vector_view_string import IVectorView_String
from python_bindings.i_www_form_url_decoder_entry import IWwwFormUrlDecoderEntry
from python_bindings.uri import Uri
from python_bindings.www_form_url_decoder import (
    IVectorView_IWwwFormUrlDecoderEntry,
)


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
    uri: Uri = Uri.create_uri("https://example.com")
    host: str = uri.host
    combined: Uri = uri.combine_uri("child")
    _: Tuple[str, Uri] = (host, combined)


def check_string_vector(calendar: Calendar) -> None:
    languages: IVectorView_String = calendar.languages
    first: str = languages.get_at(0)
    located: Tuple[int, bool] = languages.index_of(first)
    buffer = DynWinRTArray.from_string_values([""] * 4)
    many: List[str] = languages.get_many(0, buffer)
    _: Tuple[Tuple[int, bool], List[str]] = (located, many)


def check_object_vector(
    entries: IVectorView_IWwwFormUrlDecoderEntry,
    entry: IWwwFormUrlDecoderEntry,
    buffer: DynWinRTArray,
) -> None:
    first: IWwwFormUrlDecoderEntry = entries.get_at(0)
    located: Tuple[int, bool] = entries.index_of(entry)
    many: List[IWwwFormUrlDecoderEntry] = entries.get_many(0, buffer)
    _: Tuple[
        IWwwFormUrlDecoderEntry,
        Tuple[int, bool],
        List[IWwwFormUrlDecoderEntry],
    ] = (first, located, many)
