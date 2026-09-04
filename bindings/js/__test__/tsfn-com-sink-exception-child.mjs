// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { registerTestComSinkInterface } from './tsfn-com-sink-helpers.mjs'

const runtime = createRequire(import.meta.url)(process.env.DYNWINRT_TEST_RUNTIME)
let observed
process.once('uncaughtException', (error) => {
  observed = error
})

const sink = runtime.DynCom.createIUnknownSink(registerTestComSinkInterface(runtime), () => {
  throw new Error('COM sink callback failure')
})
runtime.tsfnTestRetainComSink(sink)
assert.equal(runtime.tsfnTestRegisteredHandleCount(), 1)

const hr = runtime.tsfnTestInvokeRetainedComSink()
assert.equal(hr, 0x80004005 | 0)
runtime.tsfnTestReleaseRetainedComSink()
sink.release()

const deadline = Date.now() + 2_000
while ((observed === undefined || runtime.tsfnTestRegisteredHandleCount() !== 0) && Date.now() < deadline) {
  await new Promise((resolve) => setTimeout(resolve, 5))
}

assert.equal(observed?.message, 'COM sink callback failure')
assert.equal(runtime.tsfnTestRegisteredHandleCount(), 0)
console.log(`com-sink-uncaught:${observed.message};hr:${hr}`)
