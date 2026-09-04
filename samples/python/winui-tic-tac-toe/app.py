import math
import os
from pathlib import Path

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
from generated.microsoft.ui.xaml import Application, ApplicationTheme, Window
from generated.microsoft.ui.xaml.controls import Button, StackPanel, TextBlock
from generated.microsoft.ui.xaml.markup import XamlReader
from generated.microsoft.ui.xaml.media import MicaBackdrop
from generated.windows.foundation import Size
from generated.windows.graphics import SizeInt32


class TicTacToePanel(StackPanel):
    measure_count = 0

    def measure_override(
        self, available_size: tuple[float, float]
    ) -> tuple[float, float]:
        """Handle WinUI's native layout callback in Python."""
        type(self).measure_count += 1
        width = available_size[0]
        if not math.isfinite(width) or width <= 0:
            width = 560.0
        width = min(width, 560.0)

        desired_width = 0.0
        desired_height = 0.0
        children = self.children
        if children is not None:
            for child in children:
                if child is None:
                    continue
                child.measure(Size(width, 1200.0))
                desired = child.desired_size
                desired_width = max(desired_width, desired.width)
                desired_height += desired.height

        return (max(desired_width, 480.0), max(desired_height, 640.0))


WINNING_LINES = (
    (0, 1, 2),
    (3, 4, 5),
    (6, 7, 8),
    (0, 3, 6),
    (1, 4, 7),
    (2, 5, 8),
    (0, 4, 8),
    (2, 4, 6),
)


def run() -> None:
    runtime = init_winappsdk(2, 3)
    state: dict[str, object] = {}

    try:
        with RoApartment(0), projected_lifetime_scope():
            registration = StackPanel.register_xaml_runtime_class(
                "DynWinRT.Example.TicTacToePanel",
                TicTacToePanel,
            )
            released = 0
            try:

                def initialize(_params: object) -> None:
                    def launched() -> None:
                        app = Application.get_current()
                        if app is None:
                            raise RuntimeError("Application.current is unavailable")

                        value = XamlReader.load(
                            """
                            <local:TicTacToePanel
                                xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
                                xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
                                xmlns:local="using:DynWinRT.Example"
                                Width="520"
                                Spacing="12"
                                Padding="28"
                                HorizontalAlignment="Center"
                                VerticalAlignment="Center">

                                <TextBlock
                                    Text="Tic-Tac-Toe"
                                    FontFamily="Segoe UI Variable Display"
                                    FontSize="36"
                                    FontWeight="SemiBold"
                                    HorizontalAlignment="Center" />

                                <TextBlock
                                    Text="Python custom control · native WinUI layout"
                                    Opacity="0.65"
                                    FontSize="14"
                                    HorizontalAlignment="Center" />

                                <Border
                                    Background="{ThemeResource CardBackgroundFillColorDefaultBrush}"
                                    BorderBrush="{ThemeResource CardStrokeColorDefaultBrush}"
                                    BorderThickness="1"
                                    CornerRadius="18"
                                    Padding="16"
                                    Margin="0,12,0,4">
                                    <Grid
                                        Width="420"
                                        Height="420"
                                        RowSpacing="8"
                                        ColumnSpacing="8">
                                        <Grid.RowDefinitions>
                                            <RowDefinition Height="*" />
                                            <RowDefinition Height="*" />
                                            <RowDefinition Height="*" />
                                        </Grid.RowDefinitions>
                                        <Grid.ColumnDefinitions>
                                            <ColumnDefinition Width="*" />
                                            <ColumnDefinition Width="*" />
                                            <ColumnDefinition Width="*" />
                                        </Grid.ColumnDefinitions>

                                        <Button x:Name="Cell0" Grid.Row="0" Grid.Column="0" HorizontalAlignment="Stretch" VerticalAlignment="Stretch" CornerRadius="12"><TextBlock x:Name="CellText0" FontSize="48" FontWeight="SemiBold" /></Button>
                                        <Button x:Name="Cell1" Grid.Row="0" Grid.Column="1" HorizontalAlignment="Stretch" VerticalAlignment="Stretch" CornerRadius="12"><TextBlock x:Name="CellText1" FontSize="48" FontWeight="SemiBold" /></Button>
                                        <Button x:Name="Cell2" Grid.Row="0" Grid.Column="2" HorizontalAlignment="Stretch" VerticalAlignment="Stretch" CornerRadius="12"><TextBlock x:Name="CellText2" FontSize="48" FontWeight="SemiBold" /></Button>
                                        <Button x:Name="Cell3" Grid.Row="1" Grid.Column="0" HorizontalAlignment="Stretch" VerticalAlignment="Stretch" CornerRadius="12"><TextBlock x:Name="CellText3" FontSize="48" FontWeight="SemiBold" /></Button>
                                        <Button x:Name="Cell4" Grid.Row="1" Grid.Column="1" HorizontalAlignment="Stretch" VerticalAlignment="Stretch" CornerRadius="12"><TextBlock x:Name="CellText4" FontSize="48" FontWeight="SemiBold" /></Button>
                                        <Button x:Name="Cell5" Grid.Row="1" Grid.Column="2" HorizontalAlignment="Stretch" VerticalAlignment="Stretch" CornerRadius="12"><TextBlock x:Name="CellText5" FontSize="48" FontWeight="SemiBold" /></Button>
                                        <Button x:Name="Cell6" Grid.Row="2" Grid.Column="0" HorizontalAlignment="Stretch" VerticalAlignment="Stretch" CornerRadius="12"><TextBlock x:Name="CellText6" FontSize="48" FontWeight="SemiBold" /></Button>
                                        <Button x:Name="Cell7" Grid.Row="2" Grid.Column="1" HorizontalAlignment="Stretch" VerticalAlignment="Stretch" CornerRadius="12"><TextBlock x:Name="CellText7" FontSize="48" FontWeight="SemiBold" /></Button>
                                        <Button x:Name="Cell8" Grid.Row="2" Grid.Column="2" HorizontalAlignment="Stretch" VerticalAlignment="Stretch" CornerRadius="12"><TextBlock x:Name="CellText8" FontSize="48" FontWeight="SemiBold" /></Button>
                                    </Grid>
                                </Border>

                                <TextBlock
                                    x:Name="StatusText"
                                    Text="Player X's turn"
                                    FontSize="18"
                                    FontWeight="SemiBold"
                                    HorizontalAlignment="Center"
                                    Margin="0,4,0,4" />

                                <Button
                                    x:Name="ResetButton"
                                    Content="New game"
                                    Style="{StaticResource AccentButtonStyle}"
                                    HorizontalAlignment="Center"
                                    Padding="28,8" />
                            </local:TicTacToePanel>
                            """
                        )
                        if value is None:
                            raise RuntimeError("XamlReader returned no game board")

                        panel = project_as(value, StackPanel)
                        status_value = panel.find_name("StatusText")
                        reset_value = panel.find_name("ResetButton")
                        if status_value is None or reset_value is None:
                            raise RuntimeError("Named controls were not created")

                        status = project_as(status_value, TextBlock)
                        reset_button = project_as(reset_value, Button)
                        cells: list[Button] = []
                        cell_text: list[TextBlock] = []
                        for index in range(9):
                            button_value = panel.find_name(f"Cell{index}")
                            text_value = panel.find_name(f"CellText{index}")
                            if button_value is None or text_value is None:
                                raise RuntimeError(f"Cell {index} was not created")
                            cells.append(project_as(button_value, Button))
                            cell_text.append(project_as(text_value, TextBlock))

                        board = [""] * 9
                        current_player = ["X"]
                        game_over = [False]
                        event_tokens = []

                        def reset_game(
                            _sender: object = None, _args: object = None
                        ) -> None:
                            board[:] = [""] * 9
                            current_player[0] = "X"
                            game_over[0] = False
                            status.text = "Player X's turn"
                            for button, text in zip(cells, cell_text):
                                text.text = ""
                                button.is_enabled = True

                        def play(index: int) -> None:
                            if game_over[0] or board[index]:
                                return
                            player = current_player[0]
                            board[index] = player
                            cell_text[index].text = player
                            cells[index].is_enabled = False

                            winner = next(
                                (
                                    player
                                    for a, b, c in WINNING_LINES
                                    if board[a] == board[b] == board[c] != ""
                                ),
                                None,
                            )
                            if winner is not None:
                                game_over[0] = True
                                status.text = f"Player {winner} wins!"
                                for cell, cell_value in zip(cells, board):
                                    if not cell_value:
                                        cell.is_enabled = False
                                return
                            if all(board):
                                game_over[0] = True
                                status.text = "It's a draw"
                                return

                            current_player[0] = "O" if player == "X" else "X"
                            status.text = f"Player {current_player[0]}'s turn"

                        for index, button in enumerate(cells):
                            event_tokens.append(
                                button.on_click(
                                    lambda _sender, _args, index=index: play(index)
                                )
                            )
                        event_tokens.append(reset_button.on_click(reset_game))

                        window = Window()
                        mica = MicaBackdrop()
                        window.system_backdrop = mica

                        def closed(_sender: object, _args: object) -> None:
                            current = Application.get_current()
                            if current is not None:
                                current.exit()

                        event_tokens.append(window.on_closed(closed))
                        window.title = "Tic-Tac-Toe · dynwinrt Python"
                        window.content = panel
                        window.activate()
                        app_window = window.app_window
                        if app_window is not None:
                            app_window.resize(SizeInt32(620, 760))

                        state.update(
                            window=window,
                            panel=panel,
                            cells=cells,
                            cell_text=cell_text,
                            status=status,
                            reset_button=reset_button,
                            event_tokens=event_tokens,
                            mica=mica,
                        )

                    app = Application.create(launched)
                    app.requested_theme = ApplicationTheme.Dark
                    state["app"] = app

                Application.start(initialize)
            finally:
                state.clear()
                registration.unregister()
                released = registration.release_instances()

            print(
                f"Exited cleanly; released {released} Python control(s); "
                f"measure_override ran {TicTacToePanel.measure_count} time(s)"
            )
    finally:
        del runtime


if __name__ == "__main__":
    run()
