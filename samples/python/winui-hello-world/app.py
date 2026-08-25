import argparse
import os
from pathlib import Path
from typing import Callable

ROOT = Path(__file__).resolve().parent
os.environ["WINAPPSDK_BOOTSTRAP_DLL_PATH"] = str(
    ROOT / ".runtime" / "Microsoft.WindowsAppRuntime.Bootstrap.dll"
)

from dynwinrt import (
    RoApartment,
    init_winappsdk,
    project_as,
    projected_lifetime_scope,
)
from generated.microsoft.ui.xaml import Application, Window
from generated.microsoft.ui.xaml.controls import Button, StackPanel, TextBlock
from generated.microsoft.ui.xaml.markup import XamlReader


XAML = """
<StackPanel
    xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
    xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
    Width="420"
    Spacing="16"
    Padding="32"
    HorizontalAlignment="Center"
    VerticalAlignment="Center">
    <TextBlock
        x:Name="Message"
        Text="Hello from dynwinrt"
        FontSize="28"
        HorizontalAlignment="Center" />
    <Button
        x:Name="HelloButton"
        Content="Click me"
        HorizontalAlignment="Center"
        Padding="24,8" />
</StackPanel>
"""


def run(smoke: bool) -> None:
    runtime = init_winappsdk(2, 3)
    state: dict[str, object] = {}
    subscriptions: list[Callable[[], None]] = []

    try:
        with RoApartment(0), projected_lifetime_scope():

            def initialize(_params: object) -> None:
                def launched() -> None:
                    app = Application.get_current()
                    if app is None:
                        raise RuntimeError("Application.current is unavailable")

                    value = XamlReader.load(XAML)
                    if value is None:
                        raise RuntimeError("XamlReader returned no content")
                    panel = project_as(value, StackPanel)

                    message_value = panel.find_name("Message")
                    button_value = panel.find_name("HelloButton")
                    if message_value is None or button_value is None:
                        raise RuntimeError("Named XAML controls were not created")
                    message = project_as(message_value, TextBlock)
                    button = project_as(button_value, Button)

                    click_count = 0

                    def clicked(_sender: object, _args: object) -> None:
                        nonlocal click_count
                        click_count += 1
                        message.text = f"Hello from dynwinrt ({click_count})"

                    def closed(_sender: object, _args: object) -> None:
                        current = Application.get_current()
                        if current is not None:
                            current.exit()

                    subscriptions.append(button.subscribe_click(clicked))
                    window = Window()
                    subscriptions.append(window.subscribe_closed(closed))
                    window.title = "dynwinrt Python Hello World"
                    window.content = panel
                    window.activate()

                    state.update(
                        app=app,
                        window=window,
                        panel=panel,
                        message=message,
                        button=button,
                    )

                    if smoke:
                        clicked(button, object())
                        print("python-winui-hello-ok", message.text)
                        window.close()

                state["app"] = Application.create(launched)

            Application.start(initialize)
            for unsubscribe in reversed(subscriptions):
                unsubscribe()
            state.clear()
    finally:
        del runtime


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--smoke",
        action="store_true",
        help="Create the UI, update it once, and exit immediately.",
    )
    args = parser.parse_args()
    run(args.smoke)


if __name__ == "__main__":
    main()
