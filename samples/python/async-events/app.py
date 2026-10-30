# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import argparse
import asyncio
import tempfile

from dynwinrt import AsyncEventScope, RoApartment, projected_lifetime_scope
from generated.windows.foundation.collections import (
    IMapChangedEventArgs_String,
    IObservableMap_String_Object,
    PropertySet,
)
from generated.windows.storage import FileIO, StorageFolder


async def run(*, fail: bool = False) -> None:
    with tempfile.TemporaryDirectory(prefix="dynwinrt-events-") as directory:
        with RoApartment(), projected_lifetime_scope():
            folder = await StorageFolder.get_folder_from_path_async(directory)
            properties = PropertySet()
            completed: list[str] = []
            finalized: list[str] = []

            async def on_changed(
                sender: IObservableMap_String_Object, args: IMapChangedEventArgs_String,
            ) -> None:
                key = args.key
                try:
                    file = await folder.create_file_async(f"{key}.txt")
                    await FileIO.write_text_async(file, key)
                    assert await FileIO.read_text_async(file) == key
                    # Both projected callback arguments are still usable after await.
                    assert args.key in sender
                    if fail:
                        raise ValueError("notification handler failed")
                    completed.append(key)
                finally:
                    finalized.append(key)

            try:
                async with AsyncEventScope(loop=asyncio.get_running_loop()) as events:
                    unsubscribe = events.subscribe(properties.subscribe_map_changed, on_changed)
                    properties["accepted"] = None
                    unsubscribe()
                    properties["not-delivered"] = None
            except* ValueError as errors:
                if not fail:
                    raise
                print("Handler failure propagated:", errors)
            assert finalized == ["accepted"]
            assert completed == ([] if fail else ["accepted"])
            print("async-events-ok", {"completed": completed, "finalized": finalized})


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--fail", action="store_true", help="Demonstrate handler error propagation")
    asyncio.run(run(fail=parser.parse_args().fail))
