// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { Worker } from 'node:worker_threads'

const runtimePath = process.env.DYNWINRT_TEST_RUNTIME
const runtime = createRequire(import.meta.url)(runtimePath)
runtime.tsfnTestReset()

const worker = new Worker(new URL('./tsfn-teardown-worker.mjs', import.meta.url), {
  workerData: { runtimePath },
})
const queued = await new Promise((resolve, reject) => {
  worker.once('message', resolve)
  worker.once('error', reject)
})
assert.equal(queued.produced, 10_000)
assert.equal(queued.accepted, 10_000)
assert.equal(queued.dropped, 0)

await worker.terminate()
await new Promise((resolve) => setTimeout(resolve, 100))

const stats = runtime.tsfnTestStats()
console.log(JSON.stringify(stats))
assert.equal(stats.produced, 10_000)
assert.equal(stats.dropped, stats.produced)
