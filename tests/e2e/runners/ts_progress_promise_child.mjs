// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { createRequire } from 'node:module'
import { resolve } from 'node:path'
import { setImmediate } from 'node:timers/promises'

const [name, generatedDir, runtimePath] = process.argv.slice(2)
if (!name || !generatedDir || !runtimePath) {
  throw new Error('Usage: ts_progress_promise_child.mjs <name> <generated> <runtime>')
}
const require = createRequire(import.meta.url)
const g = require(resolve(generatedDir, 'index.js'))
const runtime = require(resolve(runtimePath))
runtime.roInitialize(1)

async function streamPromises() {
  const owned = []
  const own = (value) => {
    owned.push(value)
    return value
  }
  const stream = own(new g.InMemoryRandomAccessStream())
  const bytes = Buffer.alloc(4 * 1024 * 1024, 0x5a)
  const buffer = own(g.Buffer.fromBuffer(bytes))
  try {
    const progress = []
    const op = stream.writeAsync(buffer)
    const promise = op.progress((value) => progress.push(value)).toPromise()
    assert.deepEqual(
      await Promise.all([op, promise, op.toPromise(), op.then((value) => value)]),
      Array(4).fill(bytes.length),
    )
    assert.equal(promise, op)
    assert.equal(op.toPromise(), promise)
    op.cancel()
    assert.equal(await op.toPromise(), bytes.length)
    await setImmediate()
    // Stock memory writes can finish before progress registration.
    assert.ok(progress.every((value) => Number.isInteger(value) && value >= 0 && value <= bytes.length))

    stream.seek(0n)
    const destination = own(g.Buffer.fromBuffer(Buffer.alloc(bytes.length)))
    const read = stream.readAsync(destination, bytes.length, g.InputStreamOptions.None)
    const readPromise = read.toPromise()
    const [result, sameResult] = await Promise.all([read, readPromise, read.toPromise()])
    own(result)
    assert.equal(result, sameResult, 'convert the native buffer into one projected wrapper')
    assert.equal(readPromise, read)
    assert.equal(await read.toPromise(), result)
    assert.deepEqual(result.toBuffer(), bytes)

    stream.seek(0n)
    assert.equal(await stream.writeAsync(buffer), bytes.length, 'ordinary await remains supported')
    stream.close()
    assert.throws(() => stream.writeAsync(buffer), /0x80000013|closed/i)
    const aborted = new AbortController()
    const reason = new Error('already aborted')
    aborted.abort(reason)
    const preAborted = stream.writeAsync(buffer, aborted.signal)
    const rejected = preAborted.progress(() => assert.fail('unexpected progress')).toPromise()
    assert.equal(rejected, preAborted)
    await assert.rejects(rejected, (error) => error === reason)
    await assert.rejects(preAborted.toPromise(), (error) => error === reason)
    preAborted.cancel()
  } finally {
    for (const value of owned.reverse()) g.releaseProjected(value)
  }
  console.log(`progress-stream-ok arch=${process.arch} bytes=${bytes.length}`)
}

async function httpCancellation() {
  const waiting = new Map()
  const payload = 'dynwinrt-progress-'.repeat(16_384)
  const server = createServer((request, response) => {
    if (request.url === '/error') {
      response.writeHead(404, { 'Content-Length': 0, Connection: 'close' })
      response.end()
    } else if (request.url === '/progress') {
      response.writeHead(200, { 'Content-Length': Buffer.byteLength(payload), Connection: 'close' })
      let offset = 0
      const send = () => {
        if (response.destroyed) return
        if (offset === payload.length) {
          response.end()
          return
        }
        const end = Math.min(offset + 8192, payload.length)
        response.write(payload.slice(offset, end))
        offset = end
        setTimeout(send, 5)
      }
      send()
    } else {
      const accepted = waiting.get(request.url)
      assert.ok(accepted, `unexpected localhost request ${request.url}`)
      accepted()
      // Hold the response open so cancellation cannot race a successful completion.
    }
  })
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
  const client = new g.HttpClient()
  const uris = []
  const uri = (path) => {
    const value = new g.Uri(`http://127.0.0.1:${server.address().port}${path}`)
    uris.push(value)
    return value
  }
  try {
    const progress = []
    const op = client.getStringAsync(uri('/progress')).progress((value) => progress.push(value))
    const promise = op.toPromise()
    assert.deepEqual(await Promise.all([op, promise, op.toPromise()]), Array(3).fill(payload))
    assert.equal(promise, op)
    await setImmediate()
    assert.ok(
      progress.some(
        (value) => value.bytesReceived > 0n && value.totalBytesToReceive === BigInt(Buffer.byteLength(payload)),
      ),
    )
    assert.ok(progress.every((value) => typeof value.stage === 'number' && typeof value.retries === 'number'))
    op.cancel()
    assert.equal(await op.toPromise(), payload)

    for (const mode of ['cancel', 'abort']) {
      const path = `/${mode}`
      const accepted = new Promise((resolve) => waiting.set(path, resolve))
      const controller = new AbortController()
      const reason = new Error('aborted during native request')
      const op = client.getStringAsync(uri(path), controller.signal)
      const consumers = Promise.allSettled([op, op.toPromise(), op.toPromise()])
      await accepted
      if (mode === 'cancel') op.cancel()
      else controller.abort(reason)
      const results = await consumers
      assert.ok(results.every((result) => result.status === 'rejected'))
      assert.equal(results[0].reason, results[1].reason)
      assert.equal(results[1].reason, results[2].reason)
      if (mode === 'abort') assert.equal(results[0].reason, reason)
      // GetResults on a canceled stock operation can also report E_ILLEGAL_METHOD_CALL.
      else assert.match(results[0].reason.message, /0x80004004|0x800704c7|0x8000000e/i)
    }

    const failed = client.getStringAsync(uri('/error'))
    const errors = await Promise.allSettled([failed, failed.toPromise(), failed.toPromise()])
    assert.ok(errors.every((result) => result.status === 'rejected'))
    assert.equal(errors[0].reason, errors[1].reason)
    assert.equal(errors[1].reason, errors[2].reason)
    assert.match(errors[0].reason.message, /0x80190194|404/i)
  } finally {
    client.close()
    g.releaseProjected(client)
    for (const value of uris.reverse()) g.releaseProjected(value)
    server.closeAllConnections()
    await new Promise((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())))
  }
  console.log(`progress-cancel-error-ok arch=${process.arch}`)
}

if (name === 'async_progress_promise') await streamPromises()
else if (name === 'async_progress_cancel_error') await httpCancellation()
else throw new Error(`Unknown progress Promise regression: ${name}`)
