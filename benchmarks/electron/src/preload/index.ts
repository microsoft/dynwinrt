// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { contextBridge, ipcRenderer } from 'electron'

const api = {
  ipc: (channel: string): Promise<unknown> => ipcRenderer.invoke(channel),
  logResults: (lines: string[]): Promise<void> => ipcRenderer.invoke('log-results', lines)
}

contextBridge.exposeInMainWorld('api', api)
