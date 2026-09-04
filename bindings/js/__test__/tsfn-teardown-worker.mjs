// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { createRequire } from 'node:module'
import { parentPort, workerData } from 'node:worker_threads'

const runtime = createRequire(import.meta.url)(workerData.runtimePath)
const count = 10_000
runtime.tsfnTestStartUnbounded(() => {}, count, 0)
if (!runtime.tsfnTestWaitProduced(count, 2_000)) {
  throw new Error('TSFN producer did not enqueue every payload before the timeout')
}
parentPort.postMessage(runtime.tsfnTestStats())

Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 5_000)
