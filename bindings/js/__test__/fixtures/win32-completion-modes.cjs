// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const assert = require('node:assert/strict')
const { randomUUID } = require('node:crypto')
const { once } = require('node:events')
const { createServer } = require('node:net')
const { setTimeout: delay } = require('node:timers/promises')
const { DynWin32, DynWin32Function } = require('../../dist/win32-unsafe.js')

const createFile = DynWin32Function.bind(JSON.parse(process.argv[2]))
const modeSpec = {
  dll: 'kernel32.dll',
  entryPoint: 'SetFileCompletionNotificationModes',
  parameters: [
    { type: 'handle', direction: 'in', resourceCleanup: 'closeHandle' },
    { type: 'u8', direction: 'in' },
  ],
  returnType: 'bool32',
  successRule: 'nonzero',
  captureLastError: true,
}
const trackedModes = DynWin32Function.bind({
  ...modeSpec,
  callContractDescriptor: JSON.stringify({
    resourceEffects: [{ kind: 'add-file-completion-modes', handleParameter: 0, flagsParameter: 1 }],
  }),
})
const externalModes = DynWin32Function.bind(modeSpec)
const peek = DynWin32Function.bind({
  dll: 'kernel32.dll',
  entryPoint: 'PeekNamedPipe',
  parameters: [
    { type: 'handle', direction: 'in', resourceCleanup: 'closeHandle' },
    { type: 'pointer', direction: 'in', nullable: true },
    { type: 'u32', direction: 'in' },
    { type: 'pointer', direction: 'in', nullable: true },
    { type: 'u32', direction: 'out' },
    { type: 'pointer', direction: 'in', nullable: true },
  ],
  returnType: 'bool32',
  successRule: 'nonzero',
})
const start = (operation) =>
  new Promise((resolve, reject) =>
    operation.start((error, transferred) => (error ? reject(error) : resolve(transferred))),
  )
const resource = (file) => DynWin32.resource(file, 'closeHandle')
const setModes = (file, flags, external = false) => {
  const result = (external ? externalModes : trackedModes).invoke([resource(file), DynWin32.u8(flags)])
  assert.equal(result.succeeded, true, `SetFileCompletionNotificationModes: ${result.lastError}`)
}
const waitForNativeCompletion = (file) => {
  const wait = new Int32Array(new SharedArrayBuffer(4))
  const deadline = Date.now() + 5000
  while (file.active) {
    assert.ok(Date.now() < deadline, 'native completion timed out while JS delivery was blocked')
    Atomics.wait(wait, 0, 0, 1)
  }
}

async function scenario(flags, external) {
  console.log(`completion modes ${flags}, external=${external}`)
  const pipe = `\\\\.\\pipe\\dynwinrt-completion-modes-${randomUUID()}`
  let peer
  const server = createServer((socket) => {
    peer = socket
  })
  server.listen(pipe)
  await once(server, 'listening')
  let file
  try {
    const connected = once(server, 'connection')
    const opened = createFile.invoke([
      DynWin32.wideString(pipe),
      DynWin32.u32(0xc0000000),
      DynWin32.u32(7),
      DynWin32.nullPointer(),
      DynWin32.u32(3),
      DynWin32.u32(0x40000000),
      DynWin32.handle(0n, true),
    ])
    assert.equal(opened.succeeded, true)
    file = DynWin32.toResource(opened.returnValue)
    await connected
    setModes(file, flags, external)
    setModes(file, 0, external)

    await new Promise((resolve, reject) => peer.write('ready', (error) => (error ? reject(error) : resolve())))
    const available = peek.invoke([
      resource(file),
      DynWin32.nullPointer(),
      DynWin32.u32(0),
      DynWin32.nullPointer(),
      DynWin32.nullPointer(),
    ])
    assert.equal(available.succeeded, true)
    assert.ok(DynWin32.toNumber(available.outputs[0]) >= 5)
    const bytes = Buffer.alloc(5)
    const immediate = DynWin32.beginReadFile(file, bytes)
    assert.throws(() => setModes(file, 1), /completion modes.*asynchronous I\/O/)
    let deliveries = 0
    const immediateResult = new Promise((resolve, reject) =>
      immediate.start((error, transferred) => {
        deliveries++
        if (error) reject(error)
        else resolve(transferred)
      }),
    )
    waitForNativeCompletion(file)
    assert.equal(deliveries, 0)
    assert.equal(file.busy, true)
    assert.deepEqual(bytes, Buffer.alloc(5))
    assert.throws(() => file.close(), /asynchronous I\/O/)
    assert.throws(() => setModes(file, 1), /completion modes.*asynchronous I\/O/)
    immediate.cancel()
    assert.equal(await immediateResult, 5)
    assert.equal(deliveries, 1)
    assert.equal(bytes.toString(), 'ready')
    assert.equal(file.busy, false)
    assert.equal(file.active, false)
    immediate.cancel()

    const cancelled = DynWin32.beginReadFile(file, Buffer.alloc(4))
    const pending = start(cancelled)
    assert.equal(file.active, true)
    assert.throws(() => setModes(file, 2), /completion modes.*asynchronous I\/O/)
    cancelled.cancel()
    await assert.rejects(pending, /995/)
    assert.equal(file.busy, false)
    assert.equal(file.active, false)

    setModes(file, flags | 1, external)
    const recovered = Buffer.alloc(4)
    const read = start(DynWin32.beginReadFile(file, recovered))
    peer.write('next')
    assert.equal(await read, 4)
    assert.equal(recovered.toString(), 'next')
    assert.equal(file.busy, false)

    await new Promise((resolve, reject) => peer.write('late', (error) => (error ? reject(error) : resolve())))
    const backing = new ArrayBuffer(4)
    const detachedResult = start(DynWin32.beginReadFile(file, Buffer.from(backing)))
    waitForNativeCompletion(file)
    structuredClone(backing, { transfer: [backing] })
    assert.equal(file.busy, true)
    await assert.rejects(detachedResult, /detached|changed/)
    assert.equal(file.busy, false)

    if (!external && (flags === 0 || flags === 3)) {
      await new Promise((resolve, reject) =>
        peer.write(Buffer.alloc(1024, 0x51), (error) => (error ? reject(error) : resolve())),
      )
      const buffers = Array.from({ length: 1024 }, () => Buffer.alloc(1))
      const results = buffers.map((buffer) => start(DynWin32.beginReadFile(file, buffer)))
      waitForNativeCompletion(file)
      assert.equal(file.busy, true)
      assert.ok(buffers.every((buffer) => buffer[0] === 0))
      assert.throws(() => DynWin32.beginReadFile(file, Buffer.alloc(0)), /operation limit/)
      assert.deepEqual(await Promise.all(results), Array(1024).fill(1))
      assert.ok(buffers.every((buffer) => buffer[0] === 0x51))
      assert.equal(file.busy, false)
    }

    const received = once(peer, 'data')
    const original = Buffer.from('back')
    const write = DynWin32.beginWriteFile(file, original)
    original.fill(0)
    assert.equal(await start(write), 4)
    assert.equal((await received)[0].toString(), 'back')
    assert.equal(file.busy, false)
    file.close()
    assert.equal(file.closed, true)
    await delay(25)
  } finally {
    if (file && !file.busy) file.close()
    peer?.destroy()
    await new Promise((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())))
  }
}

async function main() {
  for (const flags of [0, 1, 2, 3]) await scenario(flags, false)
  await scenario(3, true)
  console.log('PASS completion modes')
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
