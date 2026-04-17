// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { app, BrowserWindow, ipcMain } from 'electron'
import { join } from 'path'
import { createRequire } from 'module'
import { is } from '@electron-toolkit/utils'

// --- Native addons (loaded in main process only) ---
import {
  roInitialize,
  DynWinRtType,
  DynWinRtValue,
  DynWinRtMethodSig,
  DynWinRtStruct,
  WinGuid
} from '@microsoft/dynwinrt'

// Static C++/WinRT benchmark addon
const require = createRequire(import.meta.url)
const CppBench = require(
  join(__dirname, '../../../bindings/js/static-bench-cpp/build/Release/static_bench.node')
) as {
  uriCreate: (url: string) => unknown
  uriGetHost: (url: string) => string
  uriHostFromObj: (uri: unknown) => string
  uriPortFromObj: (uri: unknown) => number
  uriSuspiciousFromObj: (uri: unknown) => boolean
  uriQueryParsedFromObj: (uri: unknown) => unknown
  uriCombine: (uri: unknown, relative: string) => unknown
  pvCreateI32: (v: number) => unknown
  pvCreateF64: (v: number) => unknown
  pvCreateBool: (v: boolean) => unknown
  pvCreateString: (v: string) => unknown
  geopointCreate: (lat: number, lon: number, alt: number) => unknown
}

// Initialize COM (MTA)
roInitialize(1)
console.log('[main] COM initialized (MTA)')

// ======================================================================
// WinRT interface setup (not measured)
// ======================================================================

const factoryIid = WinGuid.parse('44A9796F-723E-4FDF-A218-033E75B0C084')
const iUriFactory = DynWinRtType.registerInterface('IUriRuntimeClassFactory', factoryIid)
  .addMethod(
    'CreateUri',
    new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.object())
  )
  .addMethod(
    'CreateWithRelativeUri',
    new DynWinRtMethodSig()
      .addIn(DynWinRtType.hstring())
      .addIn(DynWinRtType.hstring())
      .addOut(DynWinRtType.object())
  )

const uriIid = WinGuid.parse('9E365E57-48B2-4160-956F-C7385120BBFC')
const iUri = DynWinRtType.registerInterface('IUriRuntimeClass', uriIid)
  .addMethod('get_AbsoluteUri', new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
  .addMethod('get_DisplayUri', new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
  .addMethod('get_RawUri', new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
  .addMethod('get_SchemeName', new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
  .addMethod('get_UserName', new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
  .addMethod('get_Password', new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
  .addMethod('get_Host', new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
  .addMethod('get_Domain', new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
  .addMethod('get_Port', new DynWinRtMethodSig().addOut(DynWinRtType.i32()))
  .addMethod('get_Path', new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
  .addMethod('get_Query', new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
  .addMethod('get_QueryParsed', new DynWinRtMethodSig().addOut(DynWinRtType.object()))
  .addMethod('get_Fragment', new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
  .addMethod('get_Extension', new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
  .addMethod('get_Suspicious', new DynWinRtMethodSig().addOut(DynWinRtType.boolType()))
  .addMethod(
    'Equals',
    new DynWinRtMethodSig().addIn(DynWinRtType.object()).addOut(DynWinRtType.boolType())
  )
  .addMethod(
    'CombineUri',
    new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.object())
  )

const pvStaticsIid = WinGuid.parse('629BDBC8-D932-4FF4-96B9-8D96C5C1E858')
const iPvStatics = DynWinRtType.registerInterface('IPropertyValueStatics', pvStaticsIid)
  .addMethod('CreateEmpty', new DynWinRtMethodSig().addOut(DynWinRtType.object()))
  .addMethod(
    'CreateUInt8',
    new DynWinRtMethodSig().addIn(DynWinRtType.u8()).addOut(DynWinRtType.object())
  )
  .addMethod(
    'CreateInt16',
    new DynWinRtMethodSig().addIn(DynWinRtType.i16()).addOut(DynWinRtType.object())
  )
  .addMethod(
    'CreateUInt16',
    new DynWinRtMethodSig().addIn(DynWinRtType.u16()).addOut(DynWinRtType.object())
  )
  .addMethod(
    'CreateInt32',
    new DynWinRtMethodSig().addIn(DynWinRtType.i32()).addOut(DynWinRtType.object())
  )
  .addMethod(
    'CreateUInt32',
    new DynWinRtMethodSig().addIn(DynWinRtType.u32()).addOut(DynWinRtType.object())
  )
  .addMethod(
    'CreateInt64',
    new DynWinRtMethodSig().addIn(DynWinRtType.i64()).addOut(DynWinRtType.object())
  )
  .addMethod(
    'CreateUInt64',
    new DynWinRtMethodSig().addIn(DynWinRtType.u64()).addOut(DynWinRtType.object())
  )
  .addMethod(
    'CreateSingle',
    new DynWinRtMethodSig().addIn(DynWinRtType.f32()).addOut(DynWinRtType.object())
  )
  .addMethod(
    'CreateDouble',
    new DynWinRtMethodSig().addIn(DynWinRtType.f64()).addOut(DynWinRtType.object())
  )
  .addMethod(
    'CreateChar16',
    new DynWinRtMethodSig().addIn(DynWinRtType.u16()).addOut(DynWinRtType.object())
  )
  .addMethod(
    'CreateBoolean',
    new DynWinRtMethodSig().addIn(DynWinRtType.boolType()).addOut(DynWinRtType.object())
  )
  .addMethod(
    'CreateString',
    new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.object())
  )

// --- Geopoint (struct input) ---
const geoFactoryIid = WinGuid.parse('DB6B8D33-76BD-4E30-8AF7-A844DC37B7A0')
const geoStructType = DynWinRtType.structType('Windows.Devices.Geolocation.BasicGeoposition', [
  DynWinRtType.f64(),
  DynWinRtType.f64(),
  DynWinRtType.f64()
])
const iGeoFactory = DynWinRtType.registerInterface('IGeopointFactory', geoFactoryIid).addMethod(
  'Create',
  new DynWinRtMethodSig().addIn(geoStructType).addOut(DynWinRtType.object())
)

// Pre-create factories and objects
const uriFactory = DynWinRtValue.activationFactory('Windows.Foundation.Uri').cast(factoryIid)
const pvStatics = DynWinRtValue.activationFactory('Windows.Foundation.PropertyValue').cast(
  pvStaticsIid
)

const testUrl = 'https://example.com:8080/path?q=1'
const mCreateUri = iUriFactory.methodByName('CreateUri')
const mGetHost = iUri.methodByName('get_Host')
const mGetPort = iUri.methodByName('get_Port')
const mGetSuspicious = iUri.methodByName('get_Suspicious')
const mGetQueryParsed = iUri.methodByName('get_QueryParsed')
const mCombineUri = iUri.methodByName('CombineUri')
const mPvCreateI32 = iPvStatics.methodByName('CreateInt32')
const mPvCreateF64 = iPvStatics.methodByName('CreateDouble')
const mPvCreateBool = iPvStatics.methodByName('CreateBoolean')
const mPvCreateString = iPvStatics.methodByName('CreateString')
const geoFactory = DynWinRtValue.activationFactory('Windows.Devices.Geolocation.Geopoint').cast(
  geoFactoryIid
)
const mGeoCreate = iGeoFactory.methodByName('Create')

// Pre-create objects for getter benchmarks
const dynUri = mCreateUri.invoke(uriFactory, [DynWinRtValue.hstring(testUrl)]).cast(uriIid)
const staticUri = CppBench.uriCreate(testUrl)

console.log('[main] WinRT interfaces registered, factories created')


// ======================================================================
// IPC handlers — one WinRT call per IPC round-trip
// ======================================================================

// Baseline
ipcMain.handle('ipc-noop', () => {})

// Getters (0 in → 1 out)
ipcMain.handle('static-get-host', () => CppBench.uriHostFromObj(staticUri))
ipcMain.handle('dynamic-get-host', () => mGetHost.invoke(dynUri, []).toString())

ipcMain.handle('static-get-port', () => CppBench.uriPortFromObj(staticUri))
ipcMain.handle('dynamic-get-port', () => mGetPort.invoke(dynUri, []).toNumber())

ipcMain.handle('static-get-suspicious', () => CppBench.uriSuspiciousFromObj(staticUri))
ipcMain.handle('dynamic-get-suspicious', () => mGetSuspicious.invoke(dynUri, []).toBool())

ipcMain.handle('static-get-query-parsed', () => { CppBench.uriQueryParsedFromObj(staticUri) })
ipcMain.handle('dynamic-get-query-parsed', () => { mGetQueryParsed.invoke(dynUri, []) })

// Factory (1 in → 1 out)
ipcMain.handle('static-create-uri', () => { CppBench.uriCreate('https://example.com') })
ipcMain.handle('dynamic-create-uri', () => {
  mCreateUri.invoke(uriFactory, [DynWinRtValue.hstring('https://example.com')])
})

ipcMain.handle('static-pv-i32', () => { CppBench.pvCreateI32(42) })
ipcMain.handle('dynamic-pv-i32', () => {
  mPvCreateI32.invoke(pvStatics, [DynWinRtValue.i32(42)])
})

ipcMain.handle('static-pv-f64', () => { CppBench.pvCreateF64(3.14) })
ipcMain.handle('dynamic-pv-f64', () => {
  mPvCreateF64.invoke(pvStatics, [DynWinRtValue.f64(3.14)])
})

ipcMain.handle('static-pv-bool', () => { CppBench.pvCreateBool(true) })
ipcMain.handle('dynamic-pv-bool', () => {
  mPvCreateBool.invoke(pvStatics, [DynWinRtValue.boolValue(true)])
})

ipcMain.handle('static-pv-string', () => { CppBench.pvCreateString('hello') })
ipcMain.handle('dynamic-pv-string', () => {
  mPvCreateString.invoke(pvStatics, [DynWinRtValue.hstring('hello')])
})

// Struct input (struct 3×f64 → 1 out)
ipcMain.handle('static-geopoint', () => {
  CppBench.geopointCreate(47.6, -122.1, 100.0)
})
ipcMain.handle('dynamic-geopoint', () => {
  const s = DynWinRtStruct.create(geoStructType)
  s.setF64(0, 47.6)
  s.setF64(1, -122.1)
  s.setF64(2, 100.0)
  mGeoCreate.invoke(geoFactory, [s.toValue()])
})

// Method on existing object (1 in → 1 out)
ipcMain.handle('static-combine-uri', () => { CppBench.uriCombine(staticUri, '/other') })
ipcMain.handle('dynamic-combine-uri', () => {
  mCombineUri.invoke(dynUri, [DynWinRtValue.hstring('/other')])
})

// Log results from renderer
ipcMain.handle('log-results', (_event, lines: string[]) => {
  for (const line of lines) console.log(line)
})

// ======================================================================
// Electron app
// ======================================================================

function createWindow(): BrowserWindow {
  const win = new BrowserWindow({
    width: 920,
    height: 750,
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      sandbox: false,
      contextIsolation: true,
      nodeIntegration: false
    }
  })

  if (is.dev && process.env['ELECTRON_RENDERER_URL']) {
    win.loadURL(process.env['ELECTRON_RENDERER_URL'])
  } else {
    win.loadFile(join(__dirname, '../renderer/index.html'))
  }

  return win
}

app.whenReady().then(() => {
  createWindow()
  console.log('[main] Window created')
})

app.on('window-all-closed', () => {
  app.quit()
})
