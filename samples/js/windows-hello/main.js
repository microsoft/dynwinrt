// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const path = require('node:path')
const { app, BrowserWindow, ipcMain } = require('electron')
const {
  DynWinRtType,
  DynWinRtValue,
} = require('@microsoft/dynwinrt')
const { DynCom } = require('@microsoft/dynwinrt/com/unsafe')
const {
  UserConsentVerifier,
  UserConsentVerifierAvailability,
  UserConsentVerificationResult,
} = require('./generated/index.js')
const {
  IUserConsentVerifierInterop,
} = require('./generated/com/windows/win32/system/win-rt/IUserConsentVerifierInterop.js')

const resultType = DynWinRtType.enumType(
  'Windows.Security.Credentials.UI.UserConsentVerificationResult',
  Object.keys(UserConsentVerificationResult),
  Object.values(UserConsentVerificationResult),
)
const asyncResultType = DynWinRtType.iAsyncOperation(resultType)
const asyncResultIid = asyncResultType.iid().toString()

function enumName(values, value) {
  return Object.entries(values).find(([, candidate]) => candidate === value)?.[0] ?? `Unknown (${value})`
}

function createInterop() {
  const factory = DynWinRtValue.activationFactory(
    'Windows.Security.Credentials.UI.UserConsentVerifier',
  )
  try {
    return IUserConsentVerifierInterop._fromNative(factory)
  } finally {
    factory.release()
  }
}

ipcMain.handle('windows-hello:availability', async () => {
  const availability = await UserConsentVerifier.checkAvailabilityAsync()
  return {
    value: availability,
    name: enumName(UserConsentVerifierAvailability, availability),
  }
})

ipcMain.handle('windows-hello:verify', async (event) => {
  const window = BrowserWindow.fromWebContents(event.sender)
  if (!window) {
    throw new Error('The Electron window is unavailable.')
  }

  window.show()
  window.focus()

  const interop = createInterop()
  let rawOperation
  let asyncOperation
  try {
    rawOperation = interop.requestVerificationForWindowAsync(
      window.getNativeWindowHandle(),
      'Verify dynwinrt Windows Hello support',
      asyncResultIid,
    )
    asyncOperation = DynCom.projectWinRtAsync(rawOperation, asyncResultType)
    const resultValue = await asyncOperation.toPromise()
    return {
      value: resultValue.toNumber(),
      name: enumName(UserConsentVerificationResult, resultValue.toNumber()),
    }
  } finally {
    asyncOperation?.release()
    rawOperation?.release()
    interop.release()
  }
})

function createWindow() {
  const window = new BrowserWindow({
    width: 520,
    height: 350,
    resizable: false,
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
    },
  })
  window.loadFile('index.html')
}

app.whenReady().then(createWindow)

app.on('window-all-closed', () => {
  app.quit()
})
