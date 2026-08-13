// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { createRequire } from 'node:module'
import { parentPort, workerData } from 'node:worker_threads'

const runtime = createRequire(import.meta.url)(workerData.runtimePath)
const delegate = runtime.DynWinRtDelegate.create(
  runtime.WinGuid.parse('8b4e9f50-8a4c-4f10-9cc0-df934c32015f'),
  [],
  () => {},
)
runtime.tsfnTestRetainDelegate(delegate)
parentPort.postMessage('retained')

Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 5_000)
