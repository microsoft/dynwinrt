// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { contextBridge, ipcRenderer } from 'electron'

const shareSample = {
  show: (content: { title: string; text: string }): Promise<void> =>
    ipcRenderer.invoke('share:show', content)
}

contextBridge.exposeInMainWorld('shareSample', shareSample)

