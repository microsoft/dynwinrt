// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const { app, BrowserWindow } = require("electron");
const { attachValidation } = require("./main/validation.js");
const { createAppWindow } = require("./main/window.js");

const demoMode = process.argv.includes("--demo");
const validationMode = process.argv.includes("--validate");

if (!app.isPackaged) {
  app.commandLine.appendSwitch("no-sandbox");
}

app.setAppUserModelId("Microsoft.dynwinrt.AionChatSample");

function createWindow() {
  const window = createAppWindow({ demoMode, validationMode });
  if (validationMode) attachValidation(app, window);
}

app.whenReady().then(() => {
  createWindow();
  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on("window-all-closed", () => app.quit());
