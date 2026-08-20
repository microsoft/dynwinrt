// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { Worker } from 'node:worker_threads'

const runtimePath = process.env.DYNWINRT_TEST_RUNTIME
const runtime = createRequire(import.meta.url)(runtimePath)
const worker = new Worker(new URL('./tsfn-worker-com-sink-worker.mjs', import.meta.url), {
  workerData: { runtimePath },
})

await new Promise((resolve, reject) => {
  worker.once('message', resolve)
  worker.once('error', reject)
})
await worker.terminate()

const hr = runtime.tsfnTestInvokeRetainedComSink()
console.log(`late-com-sink-hr:${hr}`)
assert.equal(hr, 0x8001010e | 0)
runtime.tsfnTestReleaseRetainedComSink()
