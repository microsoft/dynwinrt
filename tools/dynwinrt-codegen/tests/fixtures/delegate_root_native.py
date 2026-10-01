# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import asyncio
import importlib
import json
import platform
import sys
import sysconfig
from pathlib import Path

from dynwinrt import DynWinRTValue, RoApartment, projected_lifetime_scope

generated = Path(sys.argv[1]).resolve()
sys.path.insert(0, str(generated.parent))
package = importlib.import_module(generated.name)
callbacks = 0
failure = None


def work_item(operation: DynWinRTValue) -> None:
    global callbacks
    try:
        assert isinstance(operation, DynWinRTValue)
        assert not operation.is_null()
        callbacks += 1
    finally:
        operation.release()


async def run() -> None:
    await asyncio.wait_for(package.ThreadPool.run_async(work_item), timeout=10)


with RoApartment(), projected_lifetime_scope():
    try:
        asyncio.run(run())
        assert callbacks == 1
    except Exception as error:
        failure = error

if len(sys.argv) > 2 and sys.argv[2] == "expect-selection-failure":
    assert callbacks == 0
    assert isinstance(failure, TypeError), repr(failure)
    assert "DynWinRTValue" in str(failure), str(failure)
else:
    assert failure is None, repr(failure)

print(json.dumps({
    "language": "py",
    "arch": sysconfig.get_platform(),
    "host": platform.machine(),
    "callbacks": callbacks,
    "error": None if failure is None else str(failure),
}))
