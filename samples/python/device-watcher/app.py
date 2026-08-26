import argparse
import asyncio
import json
from collections import Counter
from collections.abc import Callable

from dynwinrt import DynWinRTValue, RoApartment, projected_lifetime_scope
from generated.windows.devices.enumeration import (
    DeviceInformation,
    DeviceInformationUpdate,
    DeviceWatcher,
)


async def enumerate_devices(timeout: int, show_names: bool) -> None:
    with RoApartment(1), projected_lifetime_scope():
        watcher = DeviceInformation.create_watcher()
        if watcher is None:
            raise RuntimeError("DeviceInformation returned no watcher")

        loop = asyncio.get_running_loop()
        enumeration_completed = asyncio.Event()
        stopped = asyncio.Event()
        devices: dict[str, dict[str, object]] = {}

        def add_device(
            device_id: str,
            name: str,
            enabled: bool,
            kind: str,
        ) -> None:
            devices[device_id] = {
                "name": name,
                "enabled": enabled,
                "kind": kind,
            }

        def mark_updated(device_id: str) -> None:
            current = devices.get(device_id)
            if current is not None:
                current["updated"] = True

        def remove_device(device_id: str) -> None:
            devices.pop(device_id, None)

        def on_added(
            _sender: DeviceWatcher | None,
            info: DeviceInformation | None,
        ) -> None:
            if info is not None:
                loop.call_soon_threadsafe(
                    add_device,
                    info.id,
                    info.name,
                    info.is_enabled,
                    info.kind.name,
                )

        def on_updated(
            _sender: DeviceWatcher | None,
            update: DeviceInformationUpdate | None,
        ) -> None:
            if update is not None:
                loop.call_soon_threadsafe(mark_updated, update.id)

        def on_removed(
            _sender: DeviceWatcher | None,
            update: DeviceInformationUpdate | None,
        ) -> None:
            if update is not None:
                loop.call_soon_threadsafe(remove_device, update.id)

        def on_enumeration_completed(
            _sender: DeviceWatcher | None,
            _args: DynWinRTValue | None,
        ) -> None:
            loop.call_soon_threadsafe(enumeration_completed.set)

        def on_stopped(
            _sender: DeviceWatcher | None,
            _args: DynWinRTValue | None,
        ) -> None:
            loop.call_soon_threadsafe(stopped.set)

        unsubscribers: list[Callable[[], None]] = [
            watcher.subscribe_added(on_added),
            watcher.subscribe_updated(on_updated),
            watcher.subscribe_removed(on_removed),
            watcher.subscribe_enumeration_completed(
                on_enumeration_completed
            ),
            watcher.subscribe_stopped(on_stopped),
        ]
        try:
            watcher.start()
            await asyncio.wait_for(
                enumeration_completed.wait(),
                timeout=timeout,
            )
            watcher.stop()
            await asyncio.wait_for(stopped.wait(), timeout=5)
        finally:
            for unsubscribe in reversed(unsubscribers):
                unsubscribe()

        values = sorted(
            devices.values(),
            key=lambda item: str(item["name"]).casefold(),
        )
        summary: dict[str, object] = {
            "count": len(values),
            "kinds": dict(Counter(str(value["kind"]) for value in values)),
        }
        if show_names:
            summary["devices"] = values[:10]
        print("python-device-watcher-ok", json.dumps(summary, indent=2))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeout", type=int, default=10)
    parser.add_argument("--show-names", action="store_true")
    args = parser.parse_args()
    asyncio.run(enumerate_devices(args.timeout, args.show_names))


if __name__ == "__main__":
    main()
