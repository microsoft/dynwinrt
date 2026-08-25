import argparse
import asyncio
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
os.environ["WINAPPSDK_BOOTSTRAP_DLL_PATH"] = str(
    ROOT / ".runtime" / "Microsoft.WindowsAppRuntime.Bootstrap.dll"
)

from dynwinrt import RoApartment, init_winappsdk, projected_lifetime_scope
from generated.microsoft.windows.app_notifications import (
    AppNotification,
    AppNotificationActivatedEventArgs,
    AppNotificationManager,
)
from generated.microsoft.windows.app_notifications.builder import (
    AppNotificationBuilder,
)


TAG = "dynwinrt-python-sample"


def build_notification() -> AppNotification:
    builder = AppNotificationBuilder()
    builder.add_text("Hello from dynwinrt")
    builder.add_text("This notification was created by generated Python bindings.")
    builder.add_argument("source", "dynwinrt-python-sample")
    builder.set_tag(TAG)
    notification = builder.build_notification()
    if notification is None:
        raise RuntimeError("AppNotificationBuilder returned no notification")
    return notification


async def run(smoke: bool, timeout: int) -> None:
    runtime = init_winappsdk(2, 3)
    try:
        with RoApartment(1), projected_lifetime_scope():
            supported = AppNotificationManager.is_supported()
            notification = build_notification()
            if smoke:
                print(
                    "python-app-notification-ok",
                    {
                        "supported": supported,
                        "payload_length": len(notification.payload),
                    },
                )
                return
            if not supported:
                raise RuntimeError("App notifications are not supported")

            manager = AppNotificationManager.get_default()
            if manager is None:
                raise RuntimeError("AppNotificationManager is unavailable")

            loop = asyncio.get_running_loop()
            invoked = asyncio.Event()

            def on_invoked(
                _sender: AppNotificationManager | None,
                args: AppNotificationActivatedEventArgs | None,
            ) -> None:
                argument = "" if args is None else args.argument
                print("Notification activated:", argument)
                loop.call_soon_threadsafe(invoked.set)

            unsubscribe = manager.subscribe_notification_invoked(on_invoked)
            registered = False
            try:
                manager.register()
                registered = True
                manager.show(notification)
                print(f"Notification shown. Click it within {timeout} seconds.")
                try:
                    await asyncio.wait_for(invoked.wait(), timeout=timeout)
                except TimeoutError:
                    print("No activation was received before the timeout.")
            finally:
                unsubscribe()
                if registered:
                    await manager.remove_by_tag_async(TAG)
                    manager.unregister()
    finally:
        del runtime


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--smoke",
        action="store_true",
        help="Build the notification without registering or displaying it.",
    )
    parser.add_argument("--timeout", type=int, default=30)
    args = parser.parse_args()
    asyncio.run(run(args.smoke, args.timeout))


if __name__ == "__main__":
    main()
