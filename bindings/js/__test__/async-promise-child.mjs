// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import asyncHooks from 'node:async_hooks'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { DynWinRtMethodSig, DynWinRtType, DynWinRtValue, WinGuid, roInitialize } = require(
  process.env.DYNWINRT_TEST_RUNTIME ?? '../dist/index.js',
)

roInitialize(1)

const staticsIid = WinGuid.parse('5984c710-daf2-43c8-8bb4-a4d3eacfd03f')
const storageFileIid = WinGuid.parse('fa3f6186-4214-428c-a64c-14c9ac7315ea')
const storageFileType = DynWinRtType.runtimeClass('Windows.Storage.StorageFile', storageFileIid)
const staticsType = DynWinRtType.registerInterface('IStorageFileStaticsCompletedAsyncTest', staticsIid).addMethod(
  'GetFileFromPathAsync',
  new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.iAsyncOperation(storageFileType)),
)
const statics = DynWinRtValue.activationFactory('Windows.Storage.StorageFile').cast(staticsIid)
const getFile = () =>
  staticsType.methodByName('GetFileFromPathAsync').invoke(statics, [DynWinRtValue.hstring(process.execPath)])

const operations = Array.from({ length: 4 }, getFile)
Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 250)

const asyncTypes = []
const hook = asyncHooks.createHook({
  init(_asyncId, type) {
    asyncTypes.push(type)
  },
})
hook.enable()

const order = ['returned']
const settlementCounts = operations.map(() => 0)
const promises = operations.map((operation, index) =>
  operation.toPromise().then((file) => {
    settlementCounts[index] += 1
    order.push(`promise-${index}`)
    file.cast(storageFileIid)
  }),
)
process.nextTick(() => order.push('nextTick'))

assert.deepEqual(settlementCounts, [0, 0, 0, 0])
await Promise.resolve()
assert.deepEqual(settlementCounts, [0, 0, 0, 0])
await Promise.all(promises)
await new Promise((resolve) => setImmediate(resolve))
hook.disable()

assert.deepEqual(settlementCounts, [1, 1, 1, 1])
assert.equal(order[0], 'returned')
assert.equal(order[1], 'nextTick')
assert.equal(asyncTypes.filter((type) => type === 'dynwinrt.asyncPromise.dispatcher').length, 1)
assert.equal(asyncTypes.filter((type) => type === 'dynwinrt.asyncPromise').length, operations.length)

console.log('async-promise-ok')
