// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const { contextBridge, ipcRenderer } = require('electron')

contextBridge.exposeInMainWorld('windowsHello', {
  checkAvailability: () => ipcRenderer.invoke('windows-hello:availability'),
  verify: () => ipcRenderer.invoke('windows-hello:verify'),
})
