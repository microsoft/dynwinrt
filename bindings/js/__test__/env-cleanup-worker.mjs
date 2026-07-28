// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { createRequire } from 'node:module'
import { parentPort, workerData } from 'node:worker_threads'

const require = createRequire(import.meta.url)
const { DynWinRtMethodSig, DynWinRtType, DynWinRtValue, WinGuid, roInitialize } = require(workerData.runtime)

roInitialize(1)

const staticsIid = WinGuid.parse('5984c710-daf2-43c8-8bb4-a4d3eacfd03f')
const storageFileIid = WinGuid.parse('fa3f6186-4214-428c-a64c-14c9ac7315ea')
const storageFileType = DynWinRtType.runtimeClass('Windows.Storage.StorageFile', storageFileIid)
const staticsType = DynWinRtType.registerInterface('IStorageFileStaticsCleanupAsyncTest', staticsIid).addMethod(
  'GetFileFromPathAsync',
  new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.iAsyncOperation(storageFileType)),
)
const statics = DynWinRtValue.activationFactory('Windows.Storage.StorageFile').cast(staticsIid)
for (let index = 0; index < 64; index += 1) {
  staticsType
    .methodByName('GetFileFromPathAsync')
    .invoke(statics, [DynWinRtValue.hstring(process.execPath)])
    .toPromise()
}

parentPort.postMessage('registered')
Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 5000)
