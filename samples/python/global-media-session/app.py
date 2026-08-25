import argparse
import asyncio
import json
from typing import Any

from dynwinrt import RoApartment, projected_lifetime_scope
from generated.windows.media.control import (
    GlobalSystemMediaTransportControlsSession,
    GlobalSystemMediaTransportControlsSessionManager,
)


async def describe_session(
    session: GlobalSystemMediaTransportControlsSession,
) -> dict[str, Any]:
    properties = await session.try_get_media_properties_async()
    playback = session.get_playback_info()
    timeline = session.get_timeline_properties()
    return {
        "app_user_model_id": session.source_app_user_model_id,
        "title": "" if properties is None else properties.title,
        "artist": "" if properties is None else properties.artist,
        "album": "" if properties is None else properties.album_title,
        "status": (
            None if playback is None else playback.playback_status.name
        ),
        "position_seconds": (
            None if timeline is None else timeline.position.total_seconds()
        ),
        "duration_seconds": (
            None if timeline is None else timeline.end_time.total_seconds()
        ),
    }


async def snapshot(
    manager: GlobalSystemMediaTransportControlsSessionManager,
) -> list[dict[str, Any]]:
    sessions = manager.get_sessions() or ()
    return [
        await describe_session(session)
        for session in sessions
        if session is not None
    ]


async def run(watch: bool) -> None:
    with RoApartment(1), projected_lifetime_scope():
        manager = (
            await GlobalSystemMediaTransportControlsSessionManager.request_async()
        )
        if manager is None:
            raise RuntimeError("GSMTC session manager is unavailable")

        print(json.dumps(await snapshot(manager), indent=2))
        if not watch:
            return

        loop = asyncio.get_running_loop()
        changed = asyncio.Event()

        def notify(_sender: object, _args: object) -> None:
            loop.call_soon_threadsafe(changed.set)

        unsubscribe_sessions = manager.subscribe_sessions_changed(notify)
        unsubscribe_current = manager.subscribe_current_session_changed(notify)
        print("Watching media sessions. Press Ctrl+C to stop.")
        try:
            while True:
                await changed.wait()
                changed.clear()
                print(json.dumps(await snapshot(manager), indent=2))
        finally:
            unsubscribe_current()
            unsubscribe_sessions()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--watch",
        action="store_true",
        help="Print a new snapshot whenever the global sessions change.",
    )
    args = parser.parse_args()
    try:
        asyncio.run(run(args.watch))
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
