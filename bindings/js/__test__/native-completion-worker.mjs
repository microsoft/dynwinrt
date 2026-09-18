// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { createRequire } from 'node:module'
import { join } from 'node:path'
import { parentPort, workerData } from 'node:worker_threads'

const require = createRequire(import.meta.url)
const { runtimePath, generatedRoot, scenario } = workerData
const native = require(runtimePath)
const { activateAudioInterfaceAsync, IAudioClient } = require(join(generatedRoot, 'com', 'index.js'))
native.initializeCom(0)
native.comCompletionTestConfigure(
  scenario === 'teardown-held' || scenario === 'teardown-unload'
    ? 'held'
    : scenario === 'teardown-inflight'
      ? 'pause-input'
      : 'mta',
)
void activateAudioInterfaceAsync('dynwinrt-test-render', IAudioClient)
if (scenario === 'teardown-queued') {
  const deadline = Date.now() + 10_000
  while (native.comCompletionTestStats().callbacks === 0) {
    if (Date.now() > deadline) throw new Error('Native completion did not arrive')
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 1)
  }
}
parentPort.postMessage('ready')
// Keep the owner from processing the queued completion until termination.
Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0)
