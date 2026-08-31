// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { join } from 'node:path'
import { app, BrowserWindow, ipcMain } from 'electron'
import { is } from '@electron-toolkit/utils'
import { initializeCom } from '@microsoft/dynwinrt/com'
import { roInitialize } from '@microsoft/dynwinrt'
import { DataTransferManager } from '../../generated/windows/application-model/data-transfer/DataTransferManager.js'
import { projectAs, releaseProjected } from '../../generated/lifetime.js'
import { IDataTransferManagerInterop } from '../../generated/com/IDataTransferManagerInterop.js'

interface ShareContent {
  title: string
  text: string
}

interface ShareController {
  show(content: ShareContent): void
  dispose(): void
}

const controllers = new Map<number, ShareController>()

function releaseIfPresent(value: object | null | undefined): void {
  if (value != null) {
    releaseProjected(value)
  }
}

function populateShareRequest(
  sender: DataTransferManager | null,
  args: Parameters<Parameters<DataTransferManager['onDataRequested']>[0]>[1],
  content: ShareContent
): void {
  let request
  let data
  let properties

  try {
    request = args.request
    data = request.data
    properties = data.properties
    properties.title = content.title
    data.setText(content.text)
  } finally {
    releaseIfPresent(properties)
    releaseIfPresent(data)
    releaseIfPresent(request)
    releaseIfPresent(args)
    releaseIfPresent(sender)
  }
}

function createShareController(window: BrowserWindow): ShareController {
  const hwnd = window.getNativeWindowHandle()
  const interop = IDataTransferManagerInterop.create()
  let manager: DataTransferManager | undefined
  let unsubscribe: (() => void) | undefined

  try {
    const rawManager = interop.getForWindow(hwnd)
    try {
      manager = projectAs(rawManager, DataTransferManager)
    } finally {
      rawManager.release()
    }

    const projectedManager = manager
    let content: ShareContent = {
      title: 'Share from Electron',
      text: 'Hello from dynwinrt'
    }
    unsubscribe = projectedManager.onDataRequested((sender, args) => {
      populateShareRequest(sender, args, content)
    })

    return {
      show(nextContent): void {
        content = nextContent
        interop.showShareUIForWindow(hwnd)
      },
      dispose(): void {
        unsubscribe?.()
        releaseProjected(projectedManager)
        interop.release()
      }
    }
  } catch (error) {
    unsubscribe?.()
    releaseIfPresent(manager)
    interop.release()
    throw error
  }
}

function createWindow(): void {
  const window = new BrowserWindow({
    width: 520,
    height: 340,
    minWidth: 440,
    minHeight: 300,
    backgroundColor: '#f3f3f3',
    backgroundMaterial: 'mica',
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      contextIsolation: true,
      nodeIntegration: false
    }
  })

  if (is.dev && process.env.ELECTRON_RENDERER_URL) {
    void window.loadURL(process.env.ELECTRON_RENDERER_URL)
  } else {
    void window.loadFile(join(__dirname, '../renderer/index.html'))
  }

  const controller = createShareController(window)
  controllers.set(window.id, controller)
  window.once('closed', () => {
    controllers.delete(window.id)
    controller.dispose()
  })
}

ipcMain.handle('share:show', (event, content: ShareContent) => {
  const window = BrowserWindow.fromWebContents(event.sender)
  const controller = window && controllers.get(window.id)
  if (!controller) {
    throw new Error('The Share UI controller is unavailable.')
  }

  controller.show({
    title: String(content.title || 'Share from Electron'),
    text: String(content.text || '')
  })
})

app.whenReady().then(() => {
  initializeCom(0)
  roInitialize(0)
  createWindow()
})

app.on('window-all-closed', () => {
  app.quit()
})
