import argparse
import asyncio
import os
import sys
import tempfile
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parent
os.environ["WINAPPSDK_BOOTSTRAP_DLL_PATH"] = str(
    ROOT / ".runtime" / "Microsoft.WindowsAppRuntime.Bootstrap.dll"
)

from dynwinrt import RoApartment, init_winappsdk, projected_lifetime_scope
from generated.microsoft.windows.app_lifecycle import (
    AppActivationArguments,
    AppInstance,
)


async def run_primary(key: str, ready: Path, timeout: int) -> None:
    runtime = init_winappsdk(2, 3)
    try:
        with RoApartment(1), projected_lifetime_scope():
            instance = AppInstance.find_or_register_for_key(key)
            if instance is None or not instance.is_current:
                raise RuntimeError("Could not register the primary instance")

            loop = asyncio.get_running_loop()
            activated = asyncio.Event()
            activation_kind: list[str] = []

            def on_activated(
                _sender: object,
                args: AppActivationArguments | None,
            ) -> None:
                activation_kind.append(
                    "None" if args is None else args.kind.name
                )
                loop.call_soon_threadsafe(activated.set)

            unsubscribe = instance.subscribe_activated(on_activated)
            ready.write_text(str(os.getpid()), encoding="utf-8")
            try:
                await asyncio.wait_for(activated.wait(), timeout=timeout)
                print(
                    "primary-received-activation",
                    activation_kind[-1],
                    flush=True,
                )
            finally:
                unsubscribe()
                instance.unregister_key()
    finally:
        del runtime


async def redirect_to_primary(key: str) -> None:
    runtime = init_winappsdk(2, 3)
    try:
        with RoApartment(1), projected_lifetime_scope():
            current = AppInstance.get_current()
            target = AppInstance.find_or_register_for_key(key)
            if current is None or target is None:
                raise RuntimeError("AppInstance is unavailable")
            if target.is_current:
                target.unregister_key()
                raise RuntimeError("No primary instance was registered")
            arguments = current.get_activated_event_args()
            if arguments is None:
                raise RuntimeError("Current process has no activation arguments")
            await target.redirect_activation_to_async(arguments)
            print("secondary-redirected-activation", flush=True)
    finally:
        del runtime


async def run_loopback(timeout: int) -> None:
    key = f"dynwinrt-python-{uuid.uuid4()}"
    with tempfile.TemporaryDirectory(prefix="dynwinrt-app-lifecycle-") as temp:
        ready = Path(temp) / "ready.txt"
        primary = await asyncio.create_subprocess_exec(
            sys.executable,
            str(Path(__file__).resolve()),
            "--primary",
            "--key",
            key,
            "--ready",
            str(ready),
            "--timeout",
            str(timeout),
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        try:
            for _ in range(timeout * 10):
                if ready.exists():
                    break
                if primary.returncode is not None:
                    break
                await asyncio.sleep(0.1)
            if not ready.exists():
                stdout, stderr = await primary.communicate()
                raise RuntimeError(
                    "Primary instance did not become ready:\n"
                    + stdout.decode(errors="replace")
                    + stderr.decode(errors="replace")
                )

            secondary = await asyncio.create_subprocess_exec(
                sys.executable,
                str(Path(__file__).resolve()),
                "--redirect",
                "--key",
                key,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
            secondary_stdout, secondary_stderr = await secondary.communicate()
            if secondary.returncode:
                raise RuntimeError(
                    "Secondary instance failed:\n"
                    + secondary_stdout.decode(errors="replace")
                    + secondary_stderr.decode(errors="replace")
                )

            primary_stdout, primary_stderr = await asyncio.wait_for(
                primary.communicate(),
                timeout=timeout,
            )
            if primary.returncode:
                raise RuntimeError(
                    "Primary instance failed:\n"
                    + primary_stdout.decode(errors="replace")
                    + primary_stderr.decode(errors="replace")
                )
            print(secondary_stdout.decode().strip())
            print(primary_stdout.decode().strip())
            print("python-app-lifecycle-ok")
        finally:
            if primary.returncode is None:
                primary.terminate()
                await primary.wait()


def main() -> None:
    parser = argparse.ArgumentParser()
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--loopback", action="store_true")
    mode.add_argument("--primary", action="store_true")
    mode.add_argument("--redirect", action="store_true")
    parser.add_argument("--key")
    parser.add_argument("--ready", type=Path)
    parser.add_argument("--timeout", type=int, default=15)
    args = parser.parse_args()

    if args.loopback:
        asyncio.run(run_loopback(args.timeout))
    elif args.primary:
        if not args.key or args.ready is None:
            parser.error("--primary requires --key and --ready")
        asyncio.run(run_primary(args.key, args.ready, args.timeout))
    else:
        if not args.key:
            parser.error("--redirect requires --key")
        asyncio.run(redirect_to_primary(args.key))


if __name__ == "__main__":
    main()
