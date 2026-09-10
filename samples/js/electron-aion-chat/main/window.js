// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const path = require("node:path");
const { BrowserWindow, nativeTheme, screen, shell } = require("electron");

function createAppWindow({ demoMode, validationMode }) {
  const { width: workWidth, height: workHeight } =
    screen.getPrimaryDisplay().workAreaSize;
  const width = Math.min(1180, workWidth - 32);
  const height = Math.min(840, workHeight - 32);

  const window = new BrowserWindow({
    width,
    height,
    minWidth: Math.min(760, width),
    minHeight: Math.min(680, height),
    autoHideMenuBar: true,
    titleBarStyle: "hidden",
    titleBarOverlay: true,
    backgroundColor: nativeTheme.shouldUseDarkColors ? "#0d1117" : "#f6f8fa",
    show: false,
    webPreferences: {
      preload: path.join(__dirname, "..", "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  });
  window.center();

  const updateTitleBar = () => {
    const dark = nativeTheme.shouldUseDarkColors;
    window.setBackgroundColor(dark ? "#0d1117" : "#f6f8fa");
    window.setTitleBarOverlay({
      color: dark ? "#151b23" : "#ffffff",
      symbolColor: dark ? "#f0f6fc" : "#1f2328",
      height: 48,
    });
  };

  updateTitleBar();
  nativeTheme.on("updated", updateTitleBar);
  window.once("closed", () =>
    nativeTheme.removeListener("updated", updateTitleBar),
  );

  window.webContents.setWindowOpenHandler(({ url }) => {
    void shell.openExternal(url);
    return { action: "deny" };
  });
  window.webContents.on("will-navigate", (event, url) => {
    if (!url.startsWith("file:")) {
      event.preventDefault();
      void shell.openExternal(url);
    }
  });

  if (!validationMode) {
    window.webContents.once("did-finish-load", () => window.show());
  }
  void window.loadFile(
    path.join(__dirname, "..", "out", "renderer", "index.html"),
    {
      query: demoMode ? { demo: "1" } : {},
    },
  );
  return window;
}

module.exports = { createAppWindow };
