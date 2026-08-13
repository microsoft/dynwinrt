// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { Worker } from 'node:worker_threads'

const runtimePath = process.env.DYNWINRT_TEST_RUNTIME
const runtime = createRequire(import.meta.url)(runtimePath)
const worker = new Worker(new URL('./tsfn-worker-delegate-worker.mjs', import.meta.url), {
  workerData: { runtimePath },
})

await new Promise((resolve, reject) => {
  worker.once('message', resolve)
  worker.once('error', reject)
})
runtime.tsfnTestArmCallPause()
runtime.tsfnTestStartRetainedDelegateStress(1)
assert.equal(runtime.tsfnTestWaitCallPaused(2_000), true)
const termination = worker.terminate()
assert.equal(runtime.tsfnTestWaitCleanupWaiting(2_000), true)
assert.equal(runtime.tsfnTestCleanupAcquired(), false)
runtime.tsfnTestReleaseCallPause()
await termination
assert.equal(runtime.tsfnTestCleanupAcquired(), true)
const stress = runtime.tsfnTestWaitRetainedDelegateStress(5_000)
assert.equal(stress.succeeded + stress.failed, 1)
await new Promise((resolve) => setTimeout(resolve, 50))

const hr = runtime.tsfnTestInvokeRetainedDelegate()
console.log(`late-delegate-hr:${hr}`)
assert.equal(hr, 0x80004005 | 0)
runtime.tsfnTestReleaseRetainedDelegate()
