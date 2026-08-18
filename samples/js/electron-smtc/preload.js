// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("systemMediaControls", {
  audioEnabled: !process.argv.includes("--validate"),
  initialize: (options) => ipcRenderer.invoke("smtc:initialize", options),
  update: (options) => ipcRenderer.invoke("smtc:update", options),
  control: (action, value) => ipcRenderer.invoke("smtc:control", action, value),
  snapshot: () => ipcRenderer.invoke("smtc:snapshot"),
  sessions: () => ipcRenderer.invoke("smtc:sessions"),
  validate: () => ipcRenderer.invoke("smtc:validate"),
  onEvent: (callback) => {
    const listener = (_event, value) => callback(value);
    ipcRenderer.on("smtc:event", listener);
    return () => ipcRenderer.removeListener("smtc:event", listener);
  },
});
