// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const assert = require('node:assert/strict')
const { randomUUID } = require('node:crypto')
const { once } = require('node:events')
const { createServer } = require('node:net')
const { isMainThread, parentPort, threadId, Worker, workerData } = require('node:worker_threads')

const DELIVERY_COUNT = 32
const SENTINEL = 0x5a

async function bounded(promise, description) {
  let timeout
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timeout = setTimeout(() => reject(new Error(`${description} timed out`)), 10000)
      }),
    ])
  } finally {
    clearTimeout(timeout)
  }
}

function waitForNativeCompletion(file) {
  const wait = new Int32Array(new SharedArrayBuffer(4))
  const deadline = Date.now() + 5000
  while (file.active) {
    assert.ok(Date.now() < deadline, 'native I/O did not become terminal while the owner was blocked')
    Atomics.wait(wait, 0, 0, 1)
  }
}

async function runWorker() {
  const { DynWin32, DynWin32Function } = require('../../dist/win32-unsafe.js')
  const opened = DynWin32Function.bind(workerData.spec).invoke([
    DynWin32.wideString(workerData.pipe),
    DynWin32.u32(0xc0000000),
    DynWin32.u32(7),
    DynWin32.nullPointer(),
    DynWin32.u32(3),
    DynWin32.u32(0x40000000),
    DynWin32.handle(0n, true),
  ])
  assert.equal(opened.succeeded, true, `CreateFileW: ${opened.lastError}`)
  const file = DynWin32.toResource(opened.returnValue)
  parentPort.postMessage('opened')
  const [command] = await once(parentPort, 'message')
  assert.equal(command.ownerThread, threadId)
  const { phase, byte } = workerData
  let deliveries = 0

  if (phase === 'prepared-gc') {
    assert.equal(typeof global.gc, 'function')
    ;(() => {
      const operation = DynWin32.beginReadFile(file, Buffer.alloc(4, SENTINEL))
      assert.ok(operation)
      assert.equal(file.busy, true)
      assert.equal(file.active, false)
    })()
    const deadline = Date.now() + 5000
    while (file.busy) {
      assert.ok(Date.now() < deadline, 'prepared operation GC retained its native lease')
      global.gc()
      await new Promise((resolve) => setImmediate(resolve))
    }
  } else if (phase === 'throwing') {
    const buffer = Buffer.alloc(1, SENTINEL)
    const expected = new Error('IOCP owner-thread callback exception')
    const uncaught = once(process, 'uncaughtException')
    DynWin32.beginReadFile(file, buffer).start((error, transferred) => {
      deliveries++
      assert.equal(threadId, command.ownerThread)
      assert.equal(error, null)
      assert.equal(transferred, 1)
      assert.equal(buffer[0], byte)
      throw expected
    })
    waitForNativeCompletion(file)
    assert.equal(buffer[0], SENTINEL)
    assert.equal(file.busy, true)
    const [actual] = await uncaught
    assert.equal(actual, expected)
    assert.equal(deliveries, 1)
    assert.equal(file.busy, false)

    const recovered = Buffer.alloc(1, SENTINEL)
    const transferred = await new Promise((resolve, reject) => {
      DynWin32.beginReadFile(file, recovered).start((error, output) => {
        deliveries++
        assert.equal(threadId, command.ownerThread)
        if (error) reject(error)
        else resolve(output)
      })
    })
    assert.equal(transferred, 1)
    assert.equal(recovered[0], byte)
  } else {
    const count = phase === 'queued' || phase === 'delivered' ? DELIVERY_COUNT : 1
    const buffers = Array.from({ length: count }, () => Buffer.alloc(1, SENTINEL))
    const calls = Array(count).fill(0)
    const operations = buffers.map((buffer) => DynWin32.beginReadFile(file, buffer))
    // Prepared work has no TSFN. Keep it reachable until environment teardown.
    if (phase === 'prepared') global.preparedOperations = operations
    const results =
      phase === 'prepared'
        ? []
        : operations.map(
            (operation, index) =>
              new Promise((resolve, reject) => {
                operation.start((error, output) => {
                  deliveries++
                  assert.equal(threadId, command.ownerThread)
                  assert.equal(++calls[index], 1)
                  if (error) reject(error)
                  else {
                    assert.equal(buffers[index][0], byte)
                    resolve(output)
                  }
                })
              }),
          )
    if (phase !== 'pending') waitForNativeCompletion(file)
    assert.equal(deliveries, 0)
    assert.equal(file.busy, true)
    assert.ok(buffers.every((buffer) => buffer[0] === SENTINEL))
    if (phase !== 'delivered') {
      parentPort.postMessage({
        busy: file.busy,
        active: file.active,
        deliveries,
        unchanged: buffers.every((buffer) => buffer[0] === SENTINEL),
      })
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0)
      throw new Error('blocked owner unexpectedly resumed before teardown')
    }
    assert.deepEqual(await Promise.all(results), Array(count).fill(1))
    assert.deepEqual(calls, Array(count).fill(1))
    assert.ok(buffers.every((buffer) => buffer[0] === byte))
  }
  assert.equal(file.busy, false)
  assert.equal(file.active, false)
  file.close()
  parentPort.postMessage({ busy: false, active: false, deliveries, ownerThread: threadId })
  parentPort.close()
}

async function scenario(phase, byte = 0x71) {
  const pipe = `\\\\.\\pipe\\dynwinrt-io-lifecycle-${randomUUID()}`
  let peer
  const server = createServer((socket) => {
    peer = socket
  })
  server.listen(pipe)
  await once(server, 'listening')
  const connected = once(server, 'connection')
  const worker = new Worker(__filename, {
    workerData: { phase, byte, pipe, spec: JSON.parse(process.argv[2]) },
  })
  const ownerThread = worker.threadId
  const exited = once(worker, 'exit')
  try {
    assert.deepEqual(await bounded(once(worker, 'message'), `${phase} open`), ['opened'])
    await bounded(connected, `${phase} connection`)
    const closed = once(peer, 'close')
    if (phase === 'queued' || phase === 'delivered' || phase === 'throwing') {
      const count = phase === 'throwing' ? 2 : DELIVERY_COUNT
      await new Promise((resolve, reject) =>
        peer.write(Buffer.alloc(count, byte), (error) => (error ? reject(error) : resolve())),
      )
    }
    const settled = once(worker, 'message')
    worker.postMessage({ ownerThread })
    const [state] = await bounded(settled, `${phase} state`)
    if (phase === 'prepared' || phase === 'pending' || phase === 'queued') {
      assert.deepEqual(state, { busy: true, active: phase === 'pending', deliveries: 0, unchanged: true })
      await bounded(worker.terminate(), `${phase} terminate`)
    } else {
      assert.deepEqual(state, {
        busy: false,
        active: false,
        deliveries: phase === 'prepared-gc' ? 0 : phase === 'throwing' ? 2 : DELIVERY_COUNT,
        ownerThread,
      })
      assert.deepEqual(await bounded(exited, `${phase} exit`), [0])
    }
    await bounded(closed, `${phase} native pipe retirement`)
    console.log(`IOCP lifecycle ${phase}: ok`)
  } finally {
    await worker.terminate()
    peer?.destroy()
    await new Promise((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())))
  }
}

async function main() {
  // Only workers load the addon. The pinned IOCP workers and shared relay must
  // survive each environment unload, including pending and queued teardown.
  for (const phase of ['prepared-gc', 'prepared', 'pending', 'queued', 'throwing']) await scenario(phase)
  await Promise.all([scenario('delivered', 0x31), scenario('delivered', 0x72)])
  console.log('PASS IOCP owner-thread lifecycle')
}

if (isMainThread) {
  main().catch((error) => {
    console.error(error)
    process.exitCode = 1
  })
} else {
  runWorker().catch((error) => {
    console.error(error)
    process.exitCode = 1
    parentPort.close()
  })
}
