// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { Worker } from 'node:worker_threads'

const runtimePath = process.env.DYNWINRT_COMPLETION_RUNTIME
const worker = new Worker(new URL('./native-completion-worker.mjs', import.meta.url), {
  workerData: {
    runtimePath,
    generatedRoot: process.env.DYNWINRT_COMPLETION_GENERATED,
    scenario: 'teardown-unload',
  },
})
await new Promise((resolve, reject) => {
  worker.once('message', resolve)
  worker.once('error', reject)
})
await worker.terminate()

// No environment holds the addon between worker teardown and this require.
// The native callback code and its OS-owned reference must still be valid.
const native = createRequire(import.meta.url)(runtimePath)
native.initializeCom(1)
assert.equal(native.comCompletionTestStats().operationCreated, 1)
assert.equal(native.comCompletionTestStats().operationDropped, 0)
assert.equal(native.comCompletionTestStats().resultCalls, 0)
native.comCompletionTestRelease()
const deadline = Date.now() + 10_000
while (native.comCompletionTestStats().signalsLive !== 0) {
  assert.ok(Date.now() < deadline)
  await new Promise((resolve) => setTimeout(resolve, 2))
}
const stats = native.comCompletionTestStats()
assert.equal(stats.operationDropped, 1)
assert.equal(stats.callbacks, 1)
assert.equal(stats.mtaCallbacks, 1)
assert.equal(stats.resultCalls, 0)
assert.equal(stats.callbackErrors, 0)
assert.equal(stats.marshalingErrors, 0)
assert.equal(stats.wrongThread, 0)
console.log('native-completion:teardown-unload:ok')
