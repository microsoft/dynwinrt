// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { DynWinRtMethodSig, DynWinRtType, DynWinRtValue, WinGuid, roInitialize } from '../dist/index.js'

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
    .addIn(DynWinRtType.object())
    .addOut(DynWinRtType.iAsyncOperationWithProgress(DynWinRtType.u32(), DynWinRtType.u32())),
)

const bufferFactory = DynWinRtType.registerInterface(
  'IBufferFactory',
  WinGuid.parse('71AF914D-C10F-484B-BC50-14BC623B3A27'),
).addMethod('Create', new DynWinRtMethodSig().addIn(DynWinRtType.u32()).addOut(DynWinRtType.object()))

const bufferType = DynWinRtType.registerInterface('IBuffer', WinGuid.parse('905A0FE0-BC53-11DF-8C49-001E4FC686DA'))
  .addMethod('get_Capacity', new DynWinRtMethodSig().addOut(DynWinRtType.u32()))
  .addMethod('get_Length', new DynWinRtMethodSig().addOut(DynWinRtType.u32()))
  .addMethod('put_Length', new DynWinRtMethodSig().addIn(DynWinRtType.u32()))

const stream = activationFactory
  .method(6)
  .invoke(
    DynWinRtValue.activationFactory('Windows.Storage.Streams.InMemoryRandomAccessStream').cast(
      WinGuid.parse('00000035-0000-0000-C000-000000000046'),
    ),
    [],
  )
const streamOutput = stream.cast(WinGuid.parse('905A0FE6-BC53-11DF-8C49-001E4FC686DA'))

const buffer = bufferFactory
  .method(6)
  .invoke(
    DynWinRtValue.activationFactory('Windows.Storage.Streams.Buffer').cast(
      WinGuid.parse('71AF914D-C10F-484B-BC50-14BC623B3A27'),
    ),
    [DynWinRtValue.u32(1024)],
  )
bufferType
  .method(8)
  .invoke(buffer.cast(WinGuid.parse('905A0FE0-BC53-11DF-8C49-001E4FC686DA')), [DynWinRtValue.u32(1024)])

const operation = outputStream.method(6).invoke(streamOutput, [buffer])
operation.onProgress(() => {})

const result = await operation.toPromise()
if (result.toNumber() !== 1024) {
  throw new Error(`Expected 1024 bytes written, got ${result.toNumber()}`)
}

console.log('progress-exit-ok')
