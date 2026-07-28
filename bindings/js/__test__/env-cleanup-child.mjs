// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { Worker } from 'node:worker_threads'

const runtime = process.env.DYNWINRT_TEST_RUNTIME ?? fileURLToPath(new URL('../dist/index.js', import.meta.url))
createRequire(import.meta.url)(runtime)
const worker = new Worker(new URL('./env-cleanup-worker.mjs', import.meta.url), {
  workerData: { runtime },
})
const registered = await new Promise((resolve, reject) => {
  worker.once('message', resolve)
  worker.once('error', reject)
})
assert.equal(registered, 'registered')
await worker.terminate()
await new Promise((resolve) => setTimeout(resolve, 500))

console.log('env-cleanup-ok')
