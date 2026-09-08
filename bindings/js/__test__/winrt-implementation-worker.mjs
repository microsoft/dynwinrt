// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { createRequire } from 'node:module'
import { parentPort, workerData } from 'node:worker_threads'

const runtime = createRequire(import.meta.url)(workerData.runtimePath)
runtime.roInitialize(1)
const iid = runtime.WinGuid.parse('96369f54-8eb6-48f0-abce-c1b211e627c3')
const plan = runtime.DynWinRtInterfacePlan.create(
  'Windows.Foundation.IStringable',
  runtime.DynWinRtType.interface(iid),
  [
    {
      name: 'ToString',
      vtableIndex: 6,
      signature: new runtime.DynWinRtMethodSig().addOut(runtime.DynWinRtType.hstring()),
    },
  ],
)
const owner = runtime.DynWinRtImplementation.create([plan], () => [runtime.DynWinRtValue.hstring('worker')])
const value = owner.toValue()
runtime.winrtImplementationTestRetain(value)
value.release()
owner.release()
parentPort.postMessage('retained')
if (workerData.terminate) Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 10_000)
