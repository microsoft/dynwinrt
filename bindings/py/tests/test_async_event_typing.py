# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import subprocess
import sys

import pytest


SOURCE = """
import asyncio
from collections.abc import Awaitable, Callable
from dynwinrt import AsyncEventScope

class Sender:
    size: int

class Args:
    key: str

def subscribe(callback: Callable[[Sender, Args], object]) -> Callable[[], None]:
    return lambda: None

async def changed(sender: Sender, args: Args) -> None:
    await asyncio.sleep(0)
    size: int = sender.size
    key: str = args.key

def awaitable_handler(sender: Sender, args: Args) -> Awaitable[None]:
    return changed(sender, args)

async def wrong_sender(sender: str, args: Args) -> None:
    pass

async def wrong_args(sender: Sender, args: int) -> None:
    pass

async def wrong_arity(sender: Sender) -> None:
    pass

def synchronous(sender: Sender, args: Args) -> None:
    pass

async def run() -> None:
    async with AsyncEventScope(loop=asyncio.get_running_loop()) as events:
        stop: Callable[[], None] = events.subscribe(subscribe, HANDLER)
        stop()
"""


@pytest.mark.parametrize(
    "handler", ["changed", "awaitable_handler", "wrong_sender", "wrong_args", "wrong_arity", "synchronous"],
)
def test_installed_scope_preserves_handler_parameter_and_return_types(tmp_path, handler):
    consumer = tmp_path / "consumer.py"
    consumer.write_text(SOURCE.replace("HANDLER", handler), encoding="utf-8")
    result = subprocess.run(
        [sys.executable, "-m", "mypy", "--strict", "--no-incremental",
         "--cache-dir", str(tmp_path / "cache"), str(consumer)],
        capture_output=True, text=True, timeout=60,
    )
    if handler in ("changed", "awaitable_handler"):
        assert result.returncode == 0, result.stdout + result.stderr
    else:
        assert result.returncode == 1, result.stdout + result.stderr
        assert result.stdout.count("error:") == 1, result.stdout
        assert "[arg-type]" in result.stdout, result.stdout
