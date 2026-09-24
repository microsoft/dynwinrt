"""Measure the cost of the Object conversion layer.

Runs against pre- and post-boxing builds: values are written through
PropertyValue factories, which both builds accept, and the same reads are
timed. Plain-value writes are only timed when automatic boxing is available.
"""

import statistics
import sys
import timeit
from collections.abc import Callable
from functools import partial
from datetime import datetime, timezone
from typing import TYPE_CHECKING

import dynwinrt
from dynwinrt import DynWinRTValue, RoApartment
from generated.windows.foundation import PropertyValue, Uri
from generated.windows.foundation.collections import PropertySet

if TYPE_CHECKING:
    from dynwinrt import WinRTObjectValue

ENTRIES = 1000
REPEAT = 15


def measure(label: str, statement: Callable[[], object], number: int) -> None:
    samples = timeit.repeat(statement, repeat=REPEAT, number=number)
    per_call = [sample / number * 1e6 for sample in samples]
    print(f"  {label:<52} median {statistics.median(per_call):9.2f} us"
          f"   min {min(per_call):9.2f} us")


def mixed_value(index: int) -> "WinRTObjectValue | None":
    kind = index % 4
    if kind == 0:
        return PropertyValue.create_int32(index)
    if kind == 1:
        return PropertyValue.create_string(str(index))
    if kind == 2:
        return PropertyValue.create_uint32(index)
    return PropertyValue.create_double(index)


def main() -> None:
    with RoApartment(1):
        large = PropertySet()
        mixed = PropertySet()
        for index in range(ENTRIES):
            large[f"key{index:04}"] = PropertyValue.create_int32(index)
            mixed[f"key{index:04}"] = mixed_value(index)
        single = PropertySet()
        single["int32"] = PropertyValue.create_int32(42)
        single["string"] = PropertyValue.create_string("forty-two")
        single["uint32"] = PropertyValue.create_uint32(42)
        single["datetime"] = PropertyValue.create_date_time(
            datetime(2024, 1, 2, tzinfo=timezone.utc)
        )
        single["object"] = Uri("https://example.com/")
        automatic = not isinstance(single["int32"], DynWinRTValue)
        print(f"python {sys.version.split()[0]}; automatic boxing: {automatic}")

        print("single Object read (IMap.Lookup through the generated wrapper):")
        for key in ("int32", "string", "uint32", "datetime", "object"):
            measure(f"properties.lookup({key!r})", partial(single.lookup, key), 20000)
        measure("properties['int32'] (HasKey + Lookup)", lambda: single["int32"], 20000)
        # The pre-boxing way to get a Python value: an explicit unbox per read.
        unbox = dynwinrt.unbox_object
        measure("unbox_object(properties.lookup('int32'))",
                lambda: unbox(single.lookup("int32")), 20000)

        print(f"iterate and read a {ENTRIES}-entry PropertySet:")
        measure("dict(properties) [Int32 values]", lambda: dict(large), 5)
        measure("[v for _, v in properties.items()] [Int32]",
                lambda: [value for _, value in large.items()], 5)
        measure("dict(properties) [mixed Int32/String/UInt32/Double]", lambda: dict(mixed), 5)
        measure("{k: unbox_object(v) for k, v in properties.items()}",
                lambda: {key: unbox(value) for key, value in large.items()}, 5)

        print("single Object write:")
        measure("properties['k'] = PropertyValue.create_int32(5)",
                lambda: single.__setitem__("k", PropertyValue.create_int32(5)), 20000)
        if automatic:
            measure("properties['k'] = 5", lambda: single.__setitem__("k", 5), 20000)
            measure("properties['k'] = dynwinrt.UInt32(5)",
                    lambda: single.__setitem__("k", dynwinrt.UInt32(5)), 20000)
        del large, mixed, single


if __name__ == "__main__":
    main()
