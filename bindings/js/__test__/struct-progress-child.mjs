// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { createServer } from 'node:http'

import { DynWinRtMethodSig, DynWinRtType, DynWinRtValue, WinGuid, roInitialize } from '../dist/winrt.js'

roInitialize(1)

const uriFactoryIid = WinGuid.parse('44a9796f-723e-4fdf-a218-033e75b0c084')
const uriFactory = DynWinRtType.registerInterface('IUriRuntimeClassFactoryStructProgressTest', uriFactoryIid).addMethod(
  'CreateUri',
  new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.object()),
)

const activationFactoryIid = WinGuid.parse('00000035-0000-0000-c000-000000000046')
const activationFactory = DynWinRtType.registerInterface(
  'IActivationFactoryStructProgressTest',
  activationFactoryIid,
).addMethod('ActivateInstance', new DynWinRtMethodSig().addOut(DynWinRtType.object()))

const progressStage = DynWinRtType.enumType('Windows.Web.Http.HttpProgressStage')
const referenceU64 = DynWinRtType.parameterized(WinGuid.parse('61c17706-2d65-11e0-9ae8-d48564015472'), [
  DynWinRtType.u64(),
])
const httpProgress = DynWinRtType.structType('Windows.Web.Http.HttpProgress', [
  progressStage,
  DynWinRtType.u64(),
  referenceU64,
  DynWinRtType.u64(),
  referenceU64,
  DynWinRtType.u32(),
])
const reference = DynWinRtType.registerInterface('IReference_UInt64_StructProgressTest', referenceU64.iid()).addMethod(
  'get_Value',
  new DynWinRtMethodSig().addOut(DynWinRtType.u64()),
)

const clientIid = WinGuid.parse('7fda1151-3574-4880-a8ba-e6b1e0061f3d')
const clientType = DynWinRtType.registerInterface('IHttpClientStructProgressTest', clientIid)
for (const name of ['DeleteAsync', 'GetAsync', 'GetWithOptionAsync', 'GetBufferAsync', 'GetInputStreamAsync']) {
  clientType.addMethod(name, new DynWinRtMethodSig())
}
clientType.addMethod(
  'GetStringAsync',
  new DynWinRtMethodSig()
    .addIn(DynWinRtType.object())
    .addOut(DynWinRtType.iAsyncOperationWithProgress(DynWinRtType.hstring(), httpProgress)),
)

const payload = 'dynwinrt-struct-progress-'.repeat(16_384)
const server = createServer((_request, response) => {
  response.writeHead(200, {
    'Content-Length': Buffer.byteLength(payload),
    'Content-Type': 'text/plain; charset=utf-8',
    Connection: 'close',
  })
  let offset = 0
  const writeNext = () => {
    if (offset >= payload.length) {
      response.end()
      return
    }
    const end = Math.min(offset + 8_192, payload.length)
    response.write(payload.slice(offset, end))
    offset = end
    setTimeout(writeNext, 5)
  }
  writeNext()
})

await new Promise((resolve, reject) => {
  server.once('error', reject)
  server.listen(0, '127.0.0.1', resolve)
})

try {
  const address = server.address()
  assert.ok(address && typeof address !== 'string')
  const uri = uriFactory
    .method(6)
    .invoke(DynWinRtValue.activationFactory('Windows.Foundation.Uri').cast(uriFactoryIid), [
      DynWinRtValue.hstring(`http://127.0.0.1:${address.port}/progress`),
    ])
  const client = activationFactory
    .method(6)
    .invoke(DynWinRtValue.activationFactory('Windows.Web.Http.HttpClient').cast(activationFactoryIid), [])
  const operation = clientType.method(11).invoke(client.cast(clientIid), [uri])
  const snapshots = []

  operation.onProgress((value) => {
    const progress = value.asStruct()
    const totalValue = progress.getObject(4)
    const totalBytesToReceive = totalValue.isNull() ? null : reference.method(6).invoke(totalValue, []).toU64Bigint()
    snapshots.push({
      stage: progress.getI32(0),
      bytesReceived: progress.getU64(3),
      totalBytesToReceive,
      retries: progress.getU32(5),
    })
  })

  const result = await operation.toPromise()
  await new Promise((resolve) => setImmediate(resolve))

  assert.equal(result.toString(), payload)
  assert.ok(
    snapshots.some(
      (progress) => progress.bytesReceived > 0n && progress.totalBytesToReceive === BigInt(Buffer.byteLength(payload)),
    ),
    `expected retained receive totals, got ${JSON.stringify(
      snapshots.map((progress) => ({
        ...progress,
        bytesReceived: progress.bytesReceived.toString(),
        totalBytesToReceive: progress.totalBytesToReceive?.toString() ?? null,
      })),
    )}`,
  )
  assert.ok(snapshots.every((progress) => typeof progress.stage === 'number' && typeof progress.retries === 'number'))
  console.log('struct-progress-ok')
} finally {
  await new Promise((resolve) => server.close(resolve))
}
