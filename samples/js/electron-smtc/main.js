// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const path = require("node:path");
const { app, BrowserWindow, ipcMain } = require("electron");
const { roInitialize } = require("@microsoft/dynwinrt");
const { MediaControlsLoopback } = require("./media-controls.js");

const validationMode = process.argv.includes("--validate");
const demoMode = process.argv.includes("--demo");
let loopback;

app.commandLine.appendSwitch("autoplay-policy", "no-user-gesture-required");
app.setAppUserModelId("Microsoft.dynwinrt.SmtcSample");

function createWindow() {
  const window = new BrowserWindow({
    width: 1180,
    height: 860,
    minWidth: 920,
    minHeight: 680,
    autoHideMenuBar: true,
    backgroundColor: "#00000000",
    backgroundMaterial: "mica",
    show: !validationMode,
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
    },
  });
  window.loadFile("index.html");

  loopback = new MediaControlsLoopback(window, (event) => {
    if (!window.isDestroyed()) window.webContents.send("smtc:event", event);
  });
  window.once("closed", () => {
    try {
      loopback?.dispose();
    } catch (error) {
      console.error("Failed to dispose the media controls loopback.", error);
    } finally {
      loopback = undefined;
    }
  });
  return window;
}

function requireLoopback() {
  if (!loopback) throw new Error("The Electron window is not ready.");
  return loopback;
}

ipcMain.handle("smtc:initialize", (_event, options) =>
  requireLoopback().initialize(options),
);
ipcMain.handle("smtc:update", (_event, options) =>
  requireLoopback().update(options),
);
ipcMain.handle("smtc:control", (_event, action, value) =>
  requireLoopback().control(action, value),
);
ipcMain.handle("smtc:snapshot", () => requireLoopback().snapshot());
ipcMain.handle("smtc:sessions", () => requireLoopback().listSessions());
ipcMain.handle("smtc:validate", () => requireLoopback().validate());

app.whenReady().then(async () => {
  roInitialize(1);
  const window = createWindow();

  if (validationMode || demoMode) {
    try {
      const result = await loopback.validate();
      const failures = Object.entries(result.checks).filter(
        ([, passed]) => !passed,
      );
      if (failures.length > 0) {
        throw new Error(
          `Validation failed: ${JSON.stringify(result, null, 2)}`,
        );
      }
      if (validationMode) {
        console.log(JSON.stringify(result, null, 2));
        app.exit(0);
      } else {
        await loopback.initialize();
        await loopback.control("play");
        await new Promise((resolve) => setTimeout(resolve, 150));
        result.snapshot = await loopback.snapshot();
        result.sessions = await loopback.listSessions();
        console.log(JSON.stringify(result, null, 2));
        await window.webContents.executeJavaScript(
          `window.showValidationResult(${JSON.stringify(result)})`,
        );
      }
    } catch (error) {
      console.error(error);
      if (validationMode) {
        app.exit(1);
      } else {
        await window.webContents.executeJavaScript(
          `window.showValidationResult(${JSON.stringify({
            error: "Automatic validation failed. See the terminal for details.",
          })})`,
        );
      }
    }
  }
});

app.on("window-all-closed", () => app.quit());
