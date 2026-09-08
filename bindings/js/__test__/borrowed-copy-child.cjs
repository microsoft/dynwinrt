// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

'use strict'
const assert = require('node:assert/strict')
const { readFileSync } = require('node:fs')
const { join } = require('node:path')
const native = require('../dist/index.js')
const com = require('../dist/com.js')
const unsafe = require('../dist/com-unsafe.js')
const generatedRoot = process.env.DYNWINRT_BORROWED_GENERATED
const generated = require(join(generatedRoot, 'com', 'index.js'))
const { IAudioClient, IAudioClient3, IAudioRenderClient, IAudioCaptureClient, IWICBitmap, IMFMediaBuffer, IMFSample } =
  generated
const { DynComAudioFormat, projectAs } = com
const scenario = process.argv[2]

native.initializeCom(scenario === 'mta' ? 1 : 0)
assert.equal(require('../dist/winrt.js').DynComBorrowedCopyPlan, undefined)
assert.equal(com.DynComBorrowedCopyPlan, undefined)

function fixture() {
  return new native.DynComBorrowedCopyTestFixture()
}
function wrap(f, kind, type) {
  const value = f.object(kind)
  try {
    return projectAs(value, type)
  } finally {
    value.release()
  }
}
function initialize(f, { mode = 0, flags = 0, three = false, format } = {}) {
  const client = wrap(f, 'client', three ? IAudioClient3 : IAudioClient)
  format ??= DynComAudioFormat.pcm(2, 48000, 16)
  if (three) client.initializeSharedAudioStream(flags, 2, format, null)
  else client.initialize(mode, flags, 10000n, 0n, format, null)
  return client
}
function service(client, type) {
  const value = client.getService(type.IID.toString())
  try {
    return projectAs(value, type)
  } finally {
    value.release()
  }
}
function count(f, prefix) {
  return f.events().filter((event) => event.startsWith(prefix)).length
}

if (scenario === 'audio') {
  const f = fixture()
  const client = initialize(f, { flags: 0x80000000 })
  assert.equal(client.getMixFormat().blockAlign, 1)
  const render = service(client, IAudioRenderClient)
  const queried = projectAs(render, IAudioRenderClient)
  queried.release()
  const alias = wrap(f, 'render', IAudioRenderClient)
  assert.equal(render.getBuffer, undefined)
  assert.equal(render.releaseBuffer, undefined)
  assert.equal(render.attachFormat, undefined)
  client.release()
  render.release()
  f.releaseOwners()
  assert.equal(count(f, 'drop.client'), 0)
  const data = Buffer.from([1, 2, 3, 4, 5, 6, 7, 8])
  alias.writeFramesCopy(data)
  assert.deepEqual(f.bytes().subarray(0, 8), data)
  assert(f.events().includes('render.acquire.2'))
  assert(f.events().includes('render.release.2.0'))
  alias.writeFramesCopy(Buffer.alloc(0))
  alias.writeSilence(0)
  assert.equal(count(f, 'render.acquire'), 1)
  assert.throws(() => alias.writeFramesCopy(Buffer.alloc(3)), /whole.*frames/i)
  assert.throws(() => alias.writeSilence(9), /native.*capacity/i)
  assert.throws(() => alias.writeSilence(1.5), /integer/i)
  assert.throws(() => alias.writeFramesCopy(new Uint8Array(new SharedArrayBuffer(4))), /SharedArrayBuffer/i)
  const detached = new Uint8Array(4)
  structuredClone(detached.buffer, { transfer: [detached.buffer] })
  assert.throws(() => alias.writeFramesCopy(detached), /detached/i)
  f.configure('renderNull', 1)
  alias.writeSilence(2)
  assert(f.events().includes('render.release.2.2'))
  assert.throws(() => alias.writeFramesCopy(Buffer.alloc(4)), /null/i)
  assert(f.events().includes('render.release.0.0'))
  f.configure('renderNull', 0)
  f.configure('renderReleaseHr', 0x80004005)
  assert.throws(() => alias.writeFramesCopy(Buffer.alloc(4)))
  const finalizations = count(f, 'render.release')
  assert.throws(() => alias.writeFramesCopy(Buffer.alloc(4)), /poisoned/i)
  assert.equal(count(f, 'render.release'), finalizations)
  alias.release()
  assert.equal(count(f, 'drop.client'), 1)
} else if (scenario === 'provenance') {
  const f = fixture()
  const external = wrap(f, 'render', IAudioRenderClient)
  assert.throws(() => external.writeFramesCopy(Buffer.alloc(4)), /observed/i)
  const client = wrap(f, 'client', IAudioClient)
  f.configure('initializeHr', 0x80004005)
  assert.throws(() => client.initialize(0, 0, 1000n, 0n, DynComAudioFormat.pcm(2, 48000, 16), null))
  f.configure('initializeHr', 0)
  f.configure('externalInitialized', 1)
  assert.throws(() => client.initialize(0, 0, 1000n, 0n, DynComAudioFormat.pcm(2, 48000, 16), null))
  const noProof = service(client, IAudioRenderClient)
  assert.throws(() => noProof.writeFramesCopy(Buffer.alloc(4)), /observed.*initialization/i)
  assert.equal(count(f, 'render.acquire'), 0)
  noProof.release()
  external.release()
  client.release()
  const supported = fixture()
  const shared = initialize(supported, { three: true })
  const alias = projectAs(shared, IAudioClient)
  assert.throws(() => alias.initialize(1, 0, 1000n, 0n, DynComAudioFormat.pcm(1, 48000, 8), null))
  const render = service(alias, IAudioRenderClient)
  render.writeFramesCopy(Buffer.alloc(8))
  assert(supported.events().includes('render.acquire.2'))
  supported.configure('serviceWrong', 1)
  assert.throws(() => service(shared, IAudioCaptureClient))
  render.release()
  alias.release()
  shared.release()
  const exclusive = fixture()
  const eventClient = initialize(exclusive, { mode: 1, flags: 0x40000 })
  const eventRender = service(eventClient, IAudioRenderClient)
  assert.throws(() => eventRender.writeFramesCopy(Buffer.alloc(4)), /Exclusive event-driven/i)
  assert.equal(count(exclusive, 'render.acquire'), 0)
  eventRender.release()
  eventClient.release()
} else if (scenario === 'capture') {
  const f = fixture()
  const client = initialize(f, { mode: 1 })
  const capture = service(client, IAudioCaptureClient)
  f.configure('captureHr', 0x08890001)
  assert.equal(capture.readPacketCopy(), null)
  assert.equal(count(f, 'capture.release'), 0)
  f.configure('captureHr', 0)
  f.configure('captureFlags', 6)
  f.configure('captureNull', 1)
  const silent = capture.readPacketCopy()
  assert.equal(silent.kind, 'silent')
  assert.equal(silent.data, undefined)
  assert.equal(silent.timestampsValid, false)
  assert.equal(silent.devicePosition, undefined)
  assert.equal(silent.frames, 2)
  f.configure('captureFlags', 1)
  f.configure('captureNull', 0)
  const packet = capture.readPacketCopy()
  assert.equal(packet.kind, 'data')
  assert.deepEqual(packet.data, Buffer.from([0, 1, 2, 3, 4, 5, 6, 7]))
  assert.equal(packet.devicePosition, 123n)
  assert.equal(packet.qpcPosition, 456n)
  packet.data[0] = 99
  assert.equal(f.bytes()[0], 0, 'packet data must not alias native storage')
  packet.data[0] = 0
  f.configure('captureFrames', 9)
  assert.throws(() => capture.readPacketCopy(), /capacity/i)
  assert(f.events().includes('capture.release.0'))
  f.configure('captureHr', 2)
  assert.throws(() => capture.readPacketCopy(), /Unexpected acquisition HRESULT/i)
  assert.throws(() => capture.readPacketCopy(), /poisoned/i)
  assert.equal(count(f, 'capture.next'), 0)
  assert.deepEqual(packet.data, Buffer.from([0, 1, 2, 3, 4, 5, 6, 7]))
  capture.release()
  client.release()
} else if (scenario === 'media') {
  const f = fixture()
  const media = wrap(f, 'media', IMFMediaBuffer)
  const alias = wrap(f, 'mediaUnknown', IMFMediaBuffer)
  const input = f.object('media')
  const inputAlias = f.object('mediaUnknown')
  const sample = wrap(f, 'sample', IMFSample)
  sample.addBuffer(inputAlias)
  assert.equal(sample.getBufferCount(), 1)
  const inputArray = native.DynCom.interfaceArray(IMFMediaBuffer.IID, [inputAlias])
  const inputVariant = native.DynComVariant.unknown(inputAlias)
  const iid = native.WinGuid.parse('109302ce-8b8e-4e90-ad9c-5b988abc94b1')
  const receiverType = native.DynCom.registerIUnknownInterface('ArgumentGuardReceiver', iid).addMethodAt(
    3,
    'Accept',
    new native.DynComMethodSig().addIn(native.DynCom.interfaceType(IMFMediaBuffer.IID)),
  )
  let calls = 0
  const receiver = native.DynCom.createIUnknownSink(receiverType, (_slot, argument) => {
    calls++
    argument.release()
    return 0
  })
  native.DynCom.bindComObject(receiver)
  const accept = receiverType.method(3)
  accept.invokeAll(receiver, [inputAlias])
  assert.equal(calls, 1)
  const copy = media.readCopy()
  assert.deepEqual(copy, Buffer.from([0, 1, 2, 3]))
  media.replaceCopy(Buffer.from([9, 8, 7]))
  assert.deepEqual(alias.readCopy(), Buffer.from([9, 8, 7]))
  assert.deepEqual(copy, Buffer.from([0, 1, 2, 3]))
  assert.throws(() => media.replaceCopy(Buffer.alloc(17)), /capacity/i)
  f.configure('mediaCurrent', 17)
  assert.throws(() => media.readCopy(), /current.*maximum/i)
  f.configure('mediaCurrent', 3)
  f.configure('mediaSetHr', 0x80004005)
  assert.throws(() => media.replaceCopy(Buffer.alloc(4)))
  assert.equal(count(f, 'media.acquire'), count(f, 'media.unlock'))
  f.configure('mediaSetHr', 0)
  f.configure('mediaUnlockHr', 0x80004005)
  assert.throws(() => media.readCopy())
  const unlocks = count(f, 'media.unlock')
  assert.throws(() => sample.addBuffer(input), /poisoned/i)
  assert.throws(() => sample.addBuffer(inputAlias), /poisoned/i)
  assert.equal(sample.getBufferCount(), 1, 'poisoned input must not be retained by IMFSample')
  sample.removeAllBuffers()
  sample.release()
  for (const value of [input, inputAlias, inputArray]) {
    assert.throws(() => accept.invokeAll(receiver, [value]), /poisoned/i)
  }
  assert.throws(() => native.DynCom.variant(inputVariant), /poisoned/i)
  assert.throws(() => native.DynCom.interfaceArray(IMFMediaBuffer.IID, [inputAlias]), /poisoned/i)
  assert.equal(calls, 1, 'poisoned arguments must not enter the other receiver')
  media.release()
  assert.throws(() => alias.readCopy(), /poisoned/i)
  assert.equal(count(f, 'media.unlock'), unlocks)
  alias.release()
  input.release()
  inputAlias.release()
  inputVariant.release()
  f.releaseOwners()
  assert.throws(() => accept.invokeAll(receiver, [inputArray]), /poisoned/i)
  inputArray.release()
  assert.equal(count(f, 'drop.media'), 1)
  receiver.release()
} else if (scenario === 'wic' || scenario === 'mta') {
  const f = fixture()
  const bitmap = wrap(f, 'bitmap', IWICBitmap)
  const rect = { x: 0, y: 0, width: 2, height: 2 }
  if (scenario === 'mta') {
    assert.throws(() => bitmap.readLockedBgra8Copy(rect), /STA apartment/i)
    assert.equal(count(f, 'wic.acquire'), 0)
  } else {
    const copy = bitmap.readLockedBgra8Copy(rect)
    assert.deepEqual(copy.data, Buffer.from([0, 1, 2, 3, 4, 5, 6, 7, 12, 13, 14, 15, 16, 17, 18, 19]))
    copy.data[0] = 99
    assert.equal(f.bytes()[0], 0, 'read-only bitmap copy must not write native storage')
    f.configure('wicStride', 8)
    f.configure('wicCount', 16)
    assert.deepEqual(bitmap.readLockedBgra8Copy(rect).data, Buffer.from(Array.from({ length: 16 }, (_, i) => i)))
    f.configure('wicCount', 15)
    assert.throws(() => bitmap.readLockedBgra8Copy(rect), /span/i)
    f.configure('wicWrongFormat', 1)
    assert.throws(() => bitmap.readLockedBgra8Copy(rect), /32bpp BGRA/i)
    f.configure('wicWrongFormat', 0)
    f.configure('wicLockHr', 0x80004005)
    assert.throws(() => bitmap.readLockedBgra8Copy(rect))
    assert.equal(count(f, 'wic.acquire'), count(f, 'wic.release'))
    assert.throws(() => bitmap.readLockedBgra8Copy({ ...rect, width: 3 }), /native dimensions/i)
  }
  bitmap.release()
} else if (scenario === 'callback') {
  const f = fixture()
  const client = initialize(f)
  const render = service(client, IAudioRenderClient)
  let callbacks = 0
  const iid = native.WinGuid.parse('5bbdf51f-3e8d-40c5-ad0f-d8b341172d12')
  const iface = native.DynCom.registerIUnknownInterface('BorrowedProbe', iid).addMethodAt(
    3,
    'Probe',
    new native.DynComMethodSig(),
  )
  const sink = native.DynCom.createIUnknownSink(iface, () => {
    callbacks++
    return 0
  })
  native.DynCom.bindComObject(sink)
  f.installCallbackProbe(sink)
  render.writeFramesCopy(Buffer.alloc(4))
  assert.equal(callbacks, 0)
  assert(f.events().includes('callback.rejected.true'))
  render.release()
  client.release()
  sink.release()
} else if (scenario === 'descriptor') {
  const source = readFileSync(
    join(generatedRoot, 'com', 'windows', 'win32', 'media', 'media-foundation', 'IMFMediaBuffer.js'),
    'utf8',
  )
  const match = source.match(/DynComBorrowedCopyPlan\.prepare\(("(?:\\.|[^"\\])*")\)/)
  assert(match)
  const descriptor = JSON.parse(JSON.parse(match[1]))
  assert.equal(descriptor.exposure, 'owned-copy-only')
  assert.equal(descriptor.access, 'read-write')
  assert.equal(descriptor.owner, 'receiver')
  assert.equal(descriptor.version, 2)
  const acquisition = descriptor.calls.find((call) => call.id === descriptor.operations[0].acquire)
  assert.deepEqual(
    acquisition.bindings.map((binding) => binding.storage),
    ['borrowed-bytes', 'out-u32', 'out-u32'],
  )
  assert.equal(acquisition.evidence, 'IMFMediaBuffer.Lock')
  assert.equal(descriptor.operations[0].after[0].current, 'current')
  assert.equal(descriptor.operations[1].after[0].length, 'input.bytes')
  assert.equal(descriptor.operations[1].commit[0], 'set-length')
  assert.throws(
    () => unsafe.DynComBorrowedCopyPlan.prepare(JSON.stringify({ ...descriptor, version: 1 })),
    /fully regenerate/i,
  )
  for (const edit of [
    (d) => {
      d.version++
    },
    (d) => {
      d.calls[0].slot = 42
    },
    (d) => {
      d.audio_origin = { role: 'render' }
    },
    (d) => {
      d.operations[0].cleanup = null
    },
    (d) => {
      d.metadata_sha256 = 'unknown'
    },
    (d) => {
      d.exposure = 'native-view'
    },
    (d) => {
      d.owner = 'acquired-interface'
    },
    (d) => {
      d.acquired_hresult = 1
    },
    (d) => {
      d.empty_hresult = 0x08890001
    },
    (d) => {
      d.calls[0].evidence = 'unknown.Lock'
    },
    (d) => {
      d.calls[0].bindings[1].unit = 'frames'
    },
    (d) => {
      d.calls[0].bindings[1].name = d.calls[0].bindings[2].name
    },
    (d) => {
      d.operations[0].after[0].length = 'uninitialized'
    },
    (d) => {
      d.operations[1].commit = []
    },
    (d) => {
      d.id = 'matching-looking-json'
    },
  ]) {
    const changed = structuredClone(descriptor)
    edit(changed)
    assert.throws(
      () => unsafe.DynComBorrowedCopyPlan.prepare(JSON.stringify(changed)),
      /descriptor|evidence|Unsupported/i,
    )
  }
} else {
  throw new Error(`Unknown scenario ${scenario}`)
}
console.log(`borrowed-copy:${scenario}:ok`)
