// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import asyncHooks from 'node:async_hooks'
import { createRequire } from 'node:module'
import createOwnerAction from './fixtures/owner-async-action.cjs'

const require = createRequire(import.meta.url)
const runtime = require(process.env.DYNWINRT_TEST_RUNTIME ?? '../dist/winrt.js')
const {
  DynWinRtMethodSig,
  DynWinRtType,
  DynWinRtValue,
  WinGuid,
  registerWinuiDispatcherQueue,
  roInitialize,
  unregisterWinuiDispatcherQueue,
} = runtime

roInitialize(0)

const staticsIid = WinGuid.parse('5984c710-daf2-43c8-8bb4-a4d3eacfd03f')
const storageFileIid = WinGuid.parse('fa3f6186-4214-428c-a64c-14c9ac7315ea')
const storageFileType = DynWinRtType.runtimeClass(
  'Windows.Storage.StorageFile',
  DynWinRtType.interface(storageFileIid),
)
const staticsType = DynWinRtType.registerInterface('IStorageFileStaticsShutdownAsyncTest', staticsIid).addMethod(
  'GetFileFromPathAsync',
  new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.iAsyncOperation(storageFileType)),
)
const statics = DynWinRtValue.activationFactory('Windows.Storage.StorageFile').cast(staticsIid)
const operations = Array.from({ length: 16 }, () =>
  staticsType.methodByName('GetFileFromPathAsync').invoke(statics, [DynWinRtValue.hstring(process.execPath)]),
)
Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 250)

registerWinuiDispatcherQueue()
const queuedAction = createOwnerAction(runtime, { completed: true })
const lateAction = createOwnerAction(runtime, { fail: true })
const queuedPromise = queuedAction.operation.toPromise()
const lateRejection = assert.rejects(lateAction.operation.toPromise(), /0x80004005/i)
const settlementCounts = operations.map(() => 0)
const order = []
const asyncPromiseIds = new Set()
let insideUnregister = false
let directSettlementCallbacks = 0
const hook = asyncHooks.createHook({
  init(asyncId, type) {
    if (type === 'dynwinrt.asyncPromise') {
      asyncPromiseIds.add(asyncId)
    }
  },
  before(asyncId) {
    if (insideUnregister && asyncPromiseIds.has(asyncId)) {
      directSettlementCallbacks += 1
    }
  },
})
hook.enable()
const promises = operations.map((operation, index) =>
  operation.toPromise().then((file) => {
    settlementCounts[index] += 1
    order.push(index)
    file.cast(storageFileIid)
  }),
)
Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 2000)
assert.deepEqual(
  settlementCounts,
  operations.map(() => 0),
)

insideUnregister = true
assert.equal(queuedAction.resultCalls, 0)
unregisterWinuiDispatcherQueue()
insideUnregister = false
assert.equal(queuedAction.resultCalls, 1)
lateAction.finish()
order.push('unregister-returned')
assert.equal(directSettlementCallbacks, operations.length)
assert.deepEqual(
  settlementCounts,
  operations.map(() => 0),
)
await Promise.all(promises)
assert.equal((await queuedPromise).toNumber(), 0)
await lateRejection
assert.equal(lateAction.resultCalls, 1)
queuedAction.dispose()
lateAction.dispose()
await new Promise((resolve) => setImmediate(resolve))
hook.disable()
assert.deepEqual(
  settlementCounts,
  operations.map(() => 1),
)
assert.equal(order[0], 'unregister-returned')

console.log('dispatcher-shutdown-ok')
