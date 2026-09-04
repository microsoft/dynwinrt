// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { createRequire } from 'node:module'
import { parentPort, workerData } from 'node:worker_threads'
import { registerTestComSinkInterface } from './tsfn-com-sink-helpers.mjs'

const runtime = createRequire(import.meta.url)(workerData.runtimePath)
const sink = runtime.DynCom.createIUnknownSink(registerTestComSinkInterface(runtime), () => 0)
runtime.tsfnTestRetainComSink(sink)
parentPort.postMessage('retained')

Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 5_000)
