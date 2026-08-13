import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
os.environ["WINAPPSDK_BOOTSTRAP_DLL_PATH"] = str(
    ROOT / ".runtime" / "Microsoft.WindowsAppRuntime.Bootstrap.dll"
)

from dynwinrt import (
    DynWinRTMethodSig,
    DynWinRTType,
    DynWinRTValue,
    RoApartment,
    WinGUID,
    init_winappsdk,
    projected_lifetime_scope,
)
from generated.microsoft.ui.windowing import SizeInt32
from generated.microsoft.ui.xaml import (
    Application,
    ApplicationTheme,
    GridUnitType,
    HorizontalAlignment,
    VerticalAlignment,
    Window,
)
from generated.microsoft.ui.xaml.automation import AutomationProperties
from generated.microsoft.ui.xaml.controls import (
    Button,
    ColumnDefinition,
    CornerRadius,
    FontWeight,
    Grid,
    GridLength,
    RowDefinition,
    StackPanel,
    TextBlock,
    Thickness,
)
from generated.microsoft.ui.xaml.media import SystemBackdrop


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

IID_IMICA_BACKDROP = WinGUID.parse("c156a404-3dac-593a-b1f3-7a33c289dc83")
IID_IMICA_BACKDROP_FACTORY = WinGUID.parse(
    "774379ce-74bd-59d4-849d-d99c4184d838"
)
IMICA_BACKDROP_FACTORY = DynWinRTType.register_interface(
    "IMicaBackdropFactory",
    IID_IMICA_BACKDROP_FACTORY,
).add_method(
    "CreateInstance",
    DynWinRTMethodSig()
    .add_in(DynWinRTType.object())
    .add_out(DynWinRTType.object())
    .add_out(
        DynWinRTType.runtime_class(
            "Microsoft.UI.Xaml.Media.MicaBackdrop",
            DynWinRTType.interface(IID_IMICA_BACKDROP),
        )
    ),
)


def make_text(
    text: str,
    size: float,
    weight: int = 400,
    opacity: float = 1.0,
) -> TextBlock:
    block = TextBlock()
    block.text = text
    block.font_size = size
    block.font_weight = FontWeight(weight)
    block.opacity = opacity
    block.horizontal_alignment = HorizontalAlignment.Center
    return block


def create_mica_backdrop() -> SystemBackdrop:
    factory = DynWinRTValue.activation_factory(
        "Microsoft.UI.Xaml.Media.MicaBackdrop"
    ).cast(IID_IMICA_BACKDROP_FACTORY)
    results = IMICA_BACKDROP_FACTORY.method(6).invoke_all(
        factory,
        [DynWinRTValue.null_value()],
    )
    if len(results) != 2 or results[1].is_null():
        raise RuntimeError("MicaBackdrop composable activation returned no instance")
    return SystemBackdrop(results[1])


def run() -> None:
    runtime = init_winappsdk(2, 3)
    state: dict[str, object] = {}

    try:
        with RoApartment(0), projected_lifetime_scope():

            def initialize(_params: object) -> None:
                def launched() -> None:
                    app = Application.get_current()
                    if app is None:
                        raise RuntimeError("Application.current is unavailable")

                    resources = app.resources
                    merged = None if resources is None else resources.merged_dictionaries
                    if merged is None or len(merged) == 0:
                        raise RuntimeError("WinUI Fluent control resources are unavailable")

                    root = StackPanel()
                    root.width = 520.0
                    root.spacing = 14.0
                    root.padding = Thickness(32.0, 28.0, 32.0, 28.0)
                    root.horizontal_alignment = HorizontalAlignment.Center
                    root.vertical_alignment = VerticalAlignment.Center

                    title = make_text("Tic-Tac-Toe", 36.0, 600)
                    subtitle = make_text(
                        "Python · code-only WinUI 3 · Mica",
                        14.0,
                        opacity=0.65,
                    )
                    AutomationProperties.set_automation_id(title, "GameTitle")
                    AutomationProperties.set_automation_id(subtitle, "GameSubtitle")

                    board_grid = Grid()
                    board_grid.width = 420.0
                    board_grid.height = 420.0
                    board_grid.row_spacing = 8.0
                    board_grid.column_spacing = 8.0
                    board_grid.margin = Thickness(0.0, 12.0, 0.0, 4.0)
                    board_grid.horizontal_alignment = HorizontalAlignment.Center
                    AutomationProperties.set_automation_id(board_grid, "GameBoard")

                    row_definitions = board_grid.row_definitions
                    column_definitions = board_grid.column_definitions
                    children = board_grid.children
                    if (
                        row_definitions is None
                        or column_definitions is None
                        or children is None
                    ):
                        raise RuntimeError("Grid collections are unavailable")

                    star = GridLength(1.0, GridUnitType.Star)
                    for _ in range(3):
                        row = RowDefinition()
                        row.height = star
                        row_definitions.append(row)

                        column = ColumnDefinition()
                        column.width = star
                        column_definitions.append(column)

                    cells: list[Button] = []
                    cell_text: list[TextBlock] = []
                    for index in range(9):
                        button = Button()
                        button.horizontal_alignment = HorizontalAlignment.Stretch
                        button.vertical_alignment = VerticalAlignment.Stretch
                        button.corner_radius = CornerRadius(12.0, 12.0, 12.0, 12.0)

                        mark = make_text("", 48.0, 600)
                        button.content = mark
                        Grid.set_row(button, index // 3)
                        Grid.set_column(button, index % 3)
                        AutomationProperties.set_automation_id(button, f"Cell{index}")
                        AutomationProperties.set_name(button, f"Cell {index + 1}, empty")
                        children.append(button)
                        cells.append(button)
                        cell_text.append(mark)

                    status = make_text("Player X's turn", 18.0, 600)
                    status.margin = Thickness(0.0, 4.0, 0.0, 4.0)
                    AutomationProperties.set_automation_id(status, "GameStatus")

                    reset_button = Button()
                    reset_label = make_text("New game", 14.0, 600)
                    reset_button.content = reset_label
                    reset_button.padding = Thickness(28.0, 8.0, 28.0, 8.0)
                    reset_button.horizontal_alignment = HorizontalAlignment.Center
                    AutomationProperties.set_automation_id(reset_button, "ResetButton")
                    AutomationProperties.set_name(reset_button, "New game")

                    root_children = root.children
                    if root_children is None:
                        raise RuntimeError("StackPanel children are unavailable")
                    for child in (title, subtitle, board_grid, status, reset_button):
                        root_children.append(child)

                    board = [""] * 9
                    current_player = ["X"]
                    game_over = [False]
                    event_tokens = []

                    def reset_game(
                        _sender: object = None,
                        _args: object = None,
                    ) -> None:
                        board[:] = [""] * 9
                        current_player[0] = "X"
                        game_over[0] = False
                        status.text = "Player X's turn"
                        for index, (button, mark) in enumerate(
                            zip(cells, cell_text)
                        ):
                            mark.text = ""
                            button.is_enabled = True
                            AutomationProperties.set_name(
                                button, f"Cell {index + 1}, empty"
                            )

                    def play(index: int) -> None:
                        if game_over[0] or board[index]:
                            return

                        player = current_player[0]
                        board[index] = player
                        cell_text[index].text = player
                        cells[index].is_enabled = False
                        AutomationProperties.set_name(
                            cells[index], f"Cell {index + 1}, {player}"
                        )

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
                            for cell in cells:
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
                    mica = create_mica_backdrop()
                    window.system_backdrop = mica
                    if window.system_backdrop is None:
                        raise RuntimeError("MicaBackdrop was not assigned")

                    def closed(_sender: object, _args: object) -> None:
                        current = Application.get_current()
                        if current is not None:
                            current.exit()

                    event_tokens.append(window.on_closed(closed))
                    window.title = "Tic-Tac-Toe · dynwinrt Python · code-only"
                    window.content = root
                    window.activate()

                    app_window = window.app_window
                    if app_window is not None:
                        app_window.resize(SizeInt32(620, 760))

                    state.update(
                        app=app,
                        window=window,
                        root=root,
                        cells=cells,
                        cell_text=cell_text,
                        status=status,
                        reset_button=reset_button,
                        event_tokens=event_tokens,
                        mica=mica,
                    )
                    print(
                        f"Fluent resources: {len(merged)}; MicaBackdrop active",
                        flush=True,
                    )

                app = Application.create(launched)
                app.requested_theme = ApplicationTheme.Dark
                state["app"] = app

            Application.start(initialize)
            state.clear()
    finally:
        del runtime

    print("Exited cleanly", flush=True)


if __name__ == "__main__":
    run()
