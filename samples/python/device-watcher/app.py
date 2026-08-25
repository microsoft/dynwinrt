import argparse
import asyncio
import json
from collections import Counter

from dynwinrt import RoApartment, projected_lifetime_scope
from generated.windows.devices.enumeration import (
    DeviceInformation,
    DeviceInformationUpdate,
)


async def enumerate_devices(timeout: int, show_names: bool) -> None:
    with RoApartment(1), projected_lifetime_scope():
        watcher = DeviceInformation.create_watcher()
        if watcher is None:
            raise RuntimeError("DeviceInformation returned no watcher")

        devices: dict[str, dict[str, object]] = {}

        def add_device(info: DeviceInformation) -> None:
            devices[info.id] = {
                "name": info.name,
                "enabled": info.is_enabled,
                "kind": info.kind.name,
            }

        def mark_updated(device_id: str) -> None:
            current = devices.get(device_id)
            if current is not None:
                current["updated"] = True

        def remove_device(update: DeviceInformationUpdate) -> None:
            devices.pop(update.id, None)

        async with (
            watcher.added_events(max_queue_size=256) as added_events,
            watcher.updated_events() as updated_events,
            watcher.removed_events() as removed_events,
            watcher.enumeration_completed_events(
                max_queue_size=1
            ) as completed_events,
            watcher.stopped_events(max_queue_size=1) as stopped_events,
        ):

            async def consume_added() -> None:
                async for _sender, info in added_events:
                    if info is not None:
                        add_device(info)

            async def consume_updated() -> None:
                async for _sender, update in updated_events:
                    if update is not None:
                        mark_updated(update.id)

            async def consume_removed() -> None:
                async for _sender, update in removed_events:
                    if update is not None:
                        remove_device(update)

            async with asyncio.TaskGroup() as group:
                consumers = [
                    group.create_task(consume_added()),
                    group.create_task(consume_updated()),
                    group.create_task(consume_removed()),
                ]
                try:
                    watcher.start()
                    await asyncio.wait_for(
                        anext(completed_events),
                        timeout=timeout,
                    )
                    watcher.stop()
                    await asyncio.wait_for(anext(stopped_events), timeout=5)
                finally:
                    for consumer in consumers:
                        consumer.cancel()

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
