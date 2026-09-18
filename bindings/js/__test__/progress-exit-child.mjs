// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { setImmediate } from 'node:timers/promises'
import { DynWinRtMethodSig, DynWinRtType, DynWinRtValue, WinGuid, roInitialize } from '../dist/winrt.js'

roInitialize(1)

const activationFactory = DynWinRtType.registerInterface(
  'IActivationFactory',
  WinGuid.parse('00000035-0000-0000-C000-000000000046'),
).addMethod('ActivateInstance', new DynWinRtMethodSig().addOut(DynWinRtType.object()))

const outputStream = DynWinRtType.registerInterface(
  'IOutputStream',
  WinGuid.parse('905A0FE6-BC53-11DF-8C49-001E4FC686DA'),
).addMethod(
  'WriteAsync',
  new DynWinRtMethodSig()
    .addIn(DynWinRtType.interface(WinGuid.parse('905A0FE0-BC53-11DF-8C49-001E4FC686DA')))
    .addOut(DynWinRtType.iAsyncOperationWithProgress(DynWinRtType.u32(), DynWinRtType.u32())),
)

const owned = []
function own(value) {
  owned.push(value)
  return value
}

try {
  const factory = own(DynWinRtValue.activationFactory('Windows.Storage.Streams.InMemoryRandomAccessStream'))
  const factoryView = own(factory.cast(WinGuid.parse('00000035-0000-0000-C000-000000000046')))
  const stream = own(activationFactory.method(6).invoke(factoryView, []))
  const streamOutput = own(stream.cast(WinGuid.parse('905A0FE6-BC53-11DF-8C49-001E4FC686DA')))

  for (const [length, observeProgress] of [
    [1024, false],
    [2048, true],
  ]) {
    const buffer = own(DynWinRtValue.fromBuffer(Buffer.alloc(length, 0x5a)))
    const operation = own(outputStream.method(6).invoke(streamOutput, [buffer]))
    const progress = []
    if (observeProgress) {
      operation.onProgress((value) => progress.push(value.toNumber()))
    }

    const result = own(await operation.toPromise())
    assert.equal(result.toNumber(), length)
    await setImmediate()

    // A fast write may finish before registration; any delivered progress is a UInt32 byte count.
    for (const value of progress) {
      assert.ok(Number.isInteger(value) && value >= 0 && value <= length)
    }
  }
} finally {
  for (const value of owned.reverse()) value.release()
}

console.log('progress-exit-ok')
