import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { initWinappsdk, roInitialize } from "@microsoft/dynwinrt";
import {
  Application,
  ApplicationTheme,
  Button,
  MicaBackdrop,
  StackPanel,
  TextBlock,
  Window,
  XamlReader,
  createProjectedLifetimeScope,
  projectAs,
} from "#winapp/bindings";

const ROOT = path.dirname(fileURLToPath(import.meta.url));
const architecture = { arm64: "arm64", x64: "x64" }[process.arch];
if (!architecture) {
  throw new Error(`Unsupported Node.js architecture: ${process.arch}`);
}
const bootstrapDll =
  process.env.WINAPPSDK_BOOTSTRAP_DLL_PATH ??
  path.join(
    ROOT,
    ".winapp",
    "bin",
    architecture,
    "Microsoft.WindowsAppRuntime.Bootstrap.dll",
  );
if (!fs.existsSync(bootstrapDll)) {
  throw new Error(
    `Windows App SDK bootstrap DLL was not found at ${bootstrapDll}. Run npm run restore first.`,
  );
}
process.env.WINAPPSDK_BOOTSTRAP_DLL_PATH = bootstrapDll;

const XAML = `
<StackPanel
    xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
    xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
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
        Text="JavaScript - XAML WinUI 3 - Mica"
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
</StackPanel>
`;

const WINNING_LINES = [
  [0, 1, 2],
  [3, 4, 5],
  [6, 7, 8],
  [0, 3, 6],
  [1, 4, 7],
  [2, 5, 8],
  [0, 4, 8],
  [2, 4, 6],
];

/**
 * @template {object} T
 * @param {unknown} value
 * @param {{ prototype: T }} type
 * @param {string} description
 * @returns {T}
 */
function projectRaw(value, type, description) {
  if (value === null || value === undefined) {
    throw new Error(`${description} was not created`);
  }
  const ownedValue = /** @type {{ release(): void }} */ (value);
  try {
    return /** @type {T} */ (
      projectAs(value, /** @type {{ prototype: object }} */ (type))
    );
  } finally {
    ownedValue.release();
  }
}

async function main() {
  initWinappsdk(2, 3);
  roInitialize(0);

  const scope = createProjectedLifetimeScope();
  const subscriptions = [];
  const state = {};
  let cleanedUp = false;

  function cleanup() {
    if (cleanedUp) return;
    cleanedUp = true;
    for (const unsubscribe of subscriptions.reverse()) {
      unsubscribe();
    }
    for (const key of Object.keys(state)) {
      delete state[key];
    }
    scope.dispose();
  }

  try {
    await Application.startScheduled(() => {
      const app = Application.create(() => {
        const current = Application.current;
        if (current === null) {
          throw new Error("Application.current is unavailable");
        }

        const panel = projectRaw(
          XamlReader.load(XAML),
          StackPanel,
          "XAML root",
        );
        const status = projectRaw(
          panel.findName("StatusText"),
          TextBlock,
          "StatusText",
        );
        const resetButton = projectRaw(
          panel.findName("ResetButton"),
          Button,
          "ResetButton",
        );
        const cells = [];
        const cellText = [];
        for (let index = 0; index < 9; index += 1) {
          cells.push(
            projectRaw(panel.findName(`Cell${index}`), Button, `Cell${index}`),
          );
          cellText.push(
            projectRaw(
              panel.findName(`CellText${index}`),
              TextBlock,
              `CellText${index}`,
            ),
          );
        }

        const board = Array(9).fill("");
        let currentPlayer = "X";
        let gameOver = false;

        function resetGame() {
          board.fill("");
          currentPlayer = "X";
          gameOver = false;
          status.text = "Player X's turn";
          cells.forEach((button, index) => {
            cellText[index].text = "";
            button.isEnabled = true;
          });
        }

        function play(index) {
          if (gameOver || board[index]) return;

          const player = currentPlayer;
          board[index] = player;
          cellText[index].text = player;
          cells[index].isEnabled = false;

          const winner = WINNING_LINES.some(
            ([a, b, c]) =>
              board[a] && board[a] === board[b] && board[b] === board[c],
          );
          if (winner) {
            gameOver = true;
            status.text = `Player ${player} wins!`;
            cells.forEach((cell) => {
              cell.isEnabled = false;
            });
            return;
          }

          if (board.every(Boolean)) {
            gameOver = true;
            status.text = "It's a draw";
            return;
          }

          currentPlayer = player === "X" ? "O" : "X";
          status.text = `Player ${currentPlayer}'s turn`;
        }

        cells.forEach((button, index) => {
          subscriptions.push(button.onClick(() => play(index)));
        });
        subscriptions.push(resetButton.onClick(resetGame));

        const window = new Window();
        const mica = new MicaBackdrop();
        window.systemBackdrop = mica;
        subscriptions.push(
          panel.onceLoaded(() => {
            const scale = panel.xamlRoot?.rasterizationScale ?? 1;
            window.appWindow?.resizeClient({
              width: Math.ceil(620 * scale),
              height: Math.ceil(760 * scale),
            });
          }),
        );
        subscriptions.push(
          window.onClosed(() => {
            const application = Application.current;
            application?.exit();
            cleanup();
          }),
        );
        window.title = "Tic-Tac-Toe - dynwinrt JavaScript";
        window.content = panel;
        window.activate();

        Object.assign(state, {
          app: current,
          window,
          panel,
          cells,
          cellText,
          status,
          resetButton,
          mica,
        });
        console.log("XAML loaded; MicaBackdrop active");
      });

      app.requestedTheme = ApplicationTheme.Dark;
      state.app = app;
    });
  } finally {
    cleanup();
  }

  console.log("Exited cleanly");
}

main().then(
  () => process.exit(0),
  (error) => {
    console.error(error);
    process.exit(1);
  },
);
