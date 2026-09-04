import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { initWinappsdk, roInitialize } from "@microsoft/dynwinrt";
import {
  Application,
  ApplicationTheme,
  AutomationProperties,
  Button,
  ColumnDefinition,
  Grid,
  GridUnitType,
  HorizontalAlignment,
  MicaBackdrop,
  RowDefinition,
  StackPanel,
  TextBlock,
  VerticalAlignment,
  Window,
  createProjectedLifetimeScope,
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

function makeText(text, size, weight = 400, opacity = 1) {
  const block = new TextBlock();
  block.text = text;
  block.fontSize = size;
  block.fontWeight = { weight };
  block.opacity = opacity;
  block.horizontalAlignment = HorizontalAlignment.Center;
  return block;
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

        const merged = current.resources?.mergedDictionaries;
        if (merged === null || merged === undefined || merged.size === 0) {
          throw new Error("WinUI Fluent control resources are unavailable");
        }

        const root = new StackPanel();
        root.width = 520;
        root.spacing = 14;
        root.padding = { left: 32, top: 28, right: 32, bottom: 28 };
        root.horizontalAlignment = HorizontalAlignment.Center;
        root.verticalAlignment = VerticalAlignment.Center;

        const title = makeText("Tic-Tac-Toe", 36, 600);
        const subtitle = makeText(
          "JavaScript - code-only WinUI 3 - Mica",
          14,
          400,
          0.65,
        );
        AutomationProperties.setAutomationId(title, "GameTitle");
        AutomationProperties.setAutomationId(subtitle, "GameSubtitle");

        const boardGrid = new Grid();
        boardGrid.width = 420;
        boardGrid.height = 420;
        boardGrid.rowSpacing = 8;
        boardGrid.columnSpacing = 8;
        boardGrid.margin = { left: 0, top: 12, right: 0, bottom: 4 };
        boardGrid.horizontalAlignment = HorizontalAlignment.Center;
        AutomationProperties.setAutomationId(boardGrid, "GameBoard");

        const star = { value: 1, gridUnitType: GridUnitType.Star };
        for (let index = 0; index < 3; index += 1) {
          const row = new RowDefinition();
          row.height = star;
          boardGrid.rowDefinitions.append(row);

          const column = new ColumnDefinition();
          column.width = star;
          boardGrid.columnDefinitions.append(column);
        }

        const cells = [];
        const cellText = [];
        for (let index = 0; index < 9; index += 1) {
          const button = new Button();
          button.horizontalAlignment = HorizontalAlignment.Stretch;
          button.verticalAlignment = VerticalAlignment.Stretch;
          button.cornerRadius = {
            topLeft: 12,
            topRight: 12,
            bottomRight: 12,
            bottomLeft: 12,
          };

          const mark = makeText("", 48, 600);
          button.content = mark;
          Grid.setRow(button, Math.floor(index / 3));
          Grid.setColumn(button, index % 3);
          AutomationProperties.setAutomationId(button, `Cell${index}`);
          AutomationProperties.setName(button, `Cell ${index + 1}, empty`);
          boardGrid.children.append(button);
          cells.push(button);
          cellText.push(mark);
        }

        const status = makeText("Player X's turn", 18, 600);
        status.margin = { left: 0, top: 4, right: 0, bottom: 4 };
        AutomationProperties.setAutomationId(status, "GameStatus");

        const resetButton = new Button();
        resetButton.content = makeText("New game", 14, 600);
        resetButton.padding = { left: 28, top: 8, right: 28, bottom: 8 };
        resetButton.horizontalAlignment = HorizontalAlignment.Center;
        AutomationProperties.setAutomationId(resetButton, "ResetButton");
        AutomationProperties.setName(resetButton, "New game");

        for (const child of [title, subtitle, boardGrid, status, resetButton]) {
          root.children.append(child);
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
            AutomationProperties.setName(button, `Cell ${index + 1}, empty`);
          });
        }

        function play(index) {
          if (gameOver || board[index]) return;

          const player = currentPlayer;
          board[index] = player;
          cellText[index].text = player;
          cells[index].isEnabled = false;
          AutomationProperties.setName(
            cells[index],
            `Cell ${index + 1}, ${player}`,
          );

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
        if (window.systemBackdrop === null) {
          throw new Error("MicaBackdrop was not assigned");
        }

        subscriptions.push(
          root.onceLoaded(() => {
            const scale = root.xamlRoot?.rasterizationScale ?? 1;
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
        window.title = "Tic-Tac-Toe - dynwinrt JavaScript - code-only";
        window.content = root;
        window.activate();

        Object.assign(state, {
          app: current,
          window,
          root,
          cells,
          cellText,
          status,
          resetButton,
          mica,
        });
        console.log(`Fluent resources: ${merged.size}; MicaBackdrop active`);
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
