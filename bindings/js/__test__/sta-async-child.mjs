// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { DynWinRtMethodSig, DynWinRtType, DynWinRtValue, WinGuid, roInitialize } = require(
  process.env.DYNWINRT_TEST_RUNTIME ?? '../dist/index.js',
)

roInitialize(0)

const staticsIid = WinGuid.parse('5984c710-daf2-43c8-8bb4-a4d3eacfd03f')
const storageFileIid = WinGuid.parse('fa3f6186-4214-428c-a64c-14c9ac7315ea')
const storageFileType = DynWinRtType.runtimeClass(
  'Windows.Storage.StorageFile',
  DynWinRtType.interface(storageFileIid),
)
const staticsType = DynWinRtType.registerInterface('IStorageFileStaticsStaAsyncTest', staticsIid).addMethod(
  'GetFileFromPathAsync',
  new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.iAsyncOperation(storageFileType)),
)
const statics = DynWinRtValue.activationFactory('Windows.Storage.StorageFile').cast(staticsIid)
const operation = staticsType
  .methodByName('GetFileFromPathAsync')
  .invoke(statics, [DynWinRtValue.hstring(process.execPath)])

const file = await operation.toPromise()
file.cast(storageFileIid)
console.log('sta-async-ok')
