// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { AsyncLocalStorage } from 'node:async_hooks'
import { Worker } from 'node:worker_threads'

const require = createRequire(import.meta.url)
const runtimePath = process.env.DYNWINRT_COMPLETION_RUNTIME
const generatedRoot = process.env.DYNWINRT_COMPLETION_GENERATED
const native = require(runtimePath)
const com = require(join(dirname(runtimePath), 'com.js'))
const winrt = require(join(dirname(runtimePath), 'winrt.js'))
const { activateAudioInterfaceAsync, IAudioClient, IAudioEndpointVolume, IMMDevice } = require(
  join(generatedRoot, 'com', 'index.js'),
)
const path = 'dynwinrt-test-render'
const scenario = process.argv[2]

const stats = () => native.comCompletionTestStats()
const configure = (mode) => native.comCompletionTestConfigure(mode)
async function until(predicate) {
  const deadline = Date.now() + 10_000
  while (!predicate()) {
    assert.ok(Date.now() < deadline, JSON.stringify(stats()))
    await new Promise((resolve) => setTimeout(resolve, 2))
  }
}
async function clean(expectedResults) {
  await until(() => {
    const s = stats()
    return s.operationCreated === s.operationDropped && s.signalsLive === 0
  })
  const s = stats()
  assert.equal(s.pending, 0)
  assert.equal(s.resultCreated, expectedResults)
  assert.equal(s.resultDropped, expectedResults)
  assert.equal(s.wrongThread, 0)
  assert.equal(s.marshalingErrors, 0)
  assert.equal(s.callbackErrors, 0)
  assert.equal(native.tsfnTestRegisteredHandleCount(), 0)
}

if (scenario === 'render-smoke') {
  native.roInitialize(0)
  native.initializeCom(0)
  const { MediaDevice, AudioDeviceRole } = require(join(generatedRoot, 'index.js'))
  const renderPath = MediaDevice.getDefaultAudioRenderId(AudioDeviceRole.Default)
  assert.ok(renderPath, 'The opt-in smoke requires a default speaker endpoint')
  // Resolve only the render device. No capture ID, recording, or settings.
  const volume = await activateAudioInterfaceAsync(renderPath, IAudioEndpointVolume)
  try {
    assert.ok(volume instanceof IAudioEndpointVolume)
    const channels = volume.getChannelCount()
    assert.ok(Number.isInteger(channels) && channels > 0)
  } finally {
    volume.release()
  }
} else if (scenario === 'uninitialized') {
  configure('mta')
  await assert.rejects(activateAudioInterfaceAsync(path, IAudioClient), (error) => {
    assert.equal(error.stage, 'apartment')
    assert.equal(error.hresult, 0x800401f0 | 0)
    return true
  })
  await clean(0)
  assert.equal(stats().startCalls, 0)
} else if (scenario === 'sta' || scenario === 'mta') {
  native.initializeCom(scenario === 'sta' ? 0 : 1)
  assert.equal(winrt.DynComAsync, undefined)
  assert.equal(winrt.activateAudioInterfaceAsync, undefined)
  assert.equal(com.DynComAsync, undefined)
  assert.equal(com.__activateAudioInterfaceAsync, undefined)
  const context = new AsyncLocalStorage()
  for (const mode of ['mta', 'early', 'duplicate']) {
    configure(mode)
    await context.run({ mode }, async () => {
      const promise = activateAudioInterfaceAsync(path, IAudioClient)
      assert.equal(promise.cancel, undefined)
      assert.equal(stats().resultCalls, 0, 'result access must wait for owner dispatch')
      const result = await promise
      assert.equal(context.getStore().mode, mode)
      assert.ok(result instanceof IAudioClient)
      assert.equal(result.getBufferSize(), 384)
      const projected = com.projectAs(result, IAudioClient)
      assert.equal(projected.getBufferSize(), 384)
      projected.release()
      assert.throws(
        () => result._obj.cast(native.WinGuid.parse('94ea2b94-e9cc-49e0-c0ff-ee64ca8f5b90')),
        /interface|80004002/i,
      )
      assert.throws(
        () => result._obj.cast(native.WinGuid.parse('00000003-0000-0000-c000-000000000046')),
        /interface|80004002/i,
      )
      result.release()
    })
    await clean(1)
    const s = stats()
    assert.equal(s.startCalls, 1)
    assert.equal(s.validatedArguments, 1)
    assert.equal(s.signalQiChecks, 1)
    assert.equal(s.resultCalls, 1)
    assert.equal(s.callbacks, mode === 'mta' ? 1 : 2)
    assert.equal(s.mtaCallbacks, mode === 'early' ? 0 : s.callbacks)
    assert.equal(s.callbackHresult, 0)
  }
  configure('mta')
  const endpoint = await activateAudioInterfaceAsync(path, IAudioEndpointVolume)
  assert.ok(endpoint instanceof IAudioEndpointVolume)
  assert.equal(endpoint.getChannelCount(), 2)
  endpoint.release()
  await clean(1)

  for (const [mode, stage, hresult, created] of [
    ['start-failure', 'start', 0x80070005 | 0, 0],
    ['early-start-failure', 'start', 0x80070005 | 0, 0],
    ['start-null', 'start', undefined, 0],
    ['outer-failure', 'result', 0x80004005 | 0, 0],
    ['outer-failure-written', 'result', 0x80004005 | 0, 1],
    ['inner-failure', 'activation', 0x80070005 | 0, 0],
    ['inner-failure-written', 'activation', 0x80070005 | 0, 1],
    ['null-result', 'result', undefined, 0],
    ['projection-failure', 'projection', 0x80004002 | 0, 1],
  ]) {
    configure(mode)
    await assert.rejects(activateAudioInterfaceAsync(path, IAudioClient), (error) => {
      assert.equal(error.stage, stage, mode)
      assert.equal(error.hresult, hresult, mode)
      if (hresult !== undefined) {
        assert.match(error.message, new RegExp(`0x${(hresult >>> 0).toString(16)}`, 'i'))
      } else {
        assert.match(error.message, /NULL/)
      }
      return true
    })
    await clean(created)
    assert.equal(stats().resultCalls, mode.startsWith('start') || mode === 'early-start-failure' ? 0 : 1)
  }

  configure('mta')
  for (const input of [path + '\0truncated', null, {}, 1]) {
    await assert.rejects(activateAudioInterfaceAsync(input, IAudioClient), /string|NUL/)
  }
  class Unregistered extends IAudioClient {}
  await assert.rejects(activateAudioInterfaceAsync(path, IAudioClient, {}), /custom activation parameters/)
  await assert.rejects(activateAudioInterfaceAsync(path, Unregistered), /registered/)
  await assert.rejects(activateAudioInterfaceAsync(path, { IID: IAudioClient.IID }), /registered/)
  await assert.rejects(activateAudioInterfaceAsync(path, IMMDevice), /Unsupported.*target/)
  await clean(0)
  assert.equal(stats().startCalls, 0)

  const source = readFileSync(
    join(generatedRoot, 'com', 'windows', 'win32', 'media', 'audio', 'ActivateAudioInterfaceAsync.js'),
    'utf8',
  )
  const descriptor = JSON.parse(JSON.parse(source.match(/const descriptor = (.+);/)[1]))
  for (const mutate of [
    (d) => {
      d.has_this = true
    },
    (d) => {
      d.library = 'other.dll'
    },
    (d) => {
      d.export = 'other'
    },
    (d) => {
      d.calling_convention = 'cdecl'
    },
    (d) => {
      d.start_parameters.pop()
    },
    (d) => {
      d.start_parameters[2] = 'arbitrary-pointer'
    },
    (d) => {
      d.handler.slot = 4
    },
    (d) => {
      d.handler.iid = IAudioClient.IID.toString()
    },
    (d) => {
      d.result.slot = 4
    },
    (d) => {
      d.result.outputs = ['hresult', 'required-owned-interface']
    },
    (d) => {
      d.allowed_targets.push(IMMDevice.IID.toString())
    },
    (d) => {
      d.allocator = 'some-release'
    },
  ]) {
    const changed = structuredClone(descriptor)
    mutate(changed)
    assert.throws(
      () => native.DynComAsync.activateAudioInterface(JSON.stringify(changed), path, IAudioClient.IID),
      /descriptor|contract/i,
    )
  }
  assert.equal(stats().startCalls, 0)
} else if (scenario === 'bounds') {
  native.initializeCom(1)
  configure('held')
  const promises = Array.from({ length: 256 }, () => activateAudioInterfaceAsync(path, IAudioClient))
  await assert.rejects(activateAudioInterfaceAsync(path, IAudioClient), /limit.*256/)
  assert.equal(stats().pending, 256)
  native.comCompletionTestRelease()
  const values = await Promise.all(promises)
  for (const value of values) value.release()
  await clean(256)
  assert.equal(stats().resultCalls, 256)
} else if (scenario === 'liveness') {
  native.initializeCom(0)
  configure('mta')
  // No timer/worker/explicit event-loop keeper and no retained Promise.
  void activateAudioInterfaceAsync(path, IAudioClient).then((value) => {
    value.release()
    assert.equal(stats().resultCalls, 1)
    console.log(`native-completion:${scenario}:ok`)
  })
} else if (scenario === 'stale-token') {
  native.initializeCom(0)
  configure('early')
  void activateAudioInterfaceAsync(path, IAudioClient)
  assert.equal(stats().pending, 1)
  native.comCompletionTestCloseOwner()
  assert.equal(stats().pending, 0)
  assert.equal(stats().resultCalls, 0)
  // A new owner record in the same env must not receive the abandoned token.
  const value = await activateAudioInterfaceAsync(path, IAudioClient)
  value.release()
  await clean(1)
  assert.equal(stats().startCalls, 2)
  assert.equal(stats().resultCalls, 1)
} else if (scenario.startsWith('teardown-')) {
  native.initializeCom(1)
  const worker = new Worker(new URL('./native-completion-worker.mjs', import.meta.url), {
    workerData: { runtimePath, generatedRoot, scenario },
  })
  await new Promise((resolve, reject) => {
    worker.once('message', resolve)
    worker.once('error', reject)
  })
  if (scenario === 'teardown-inflight') {
    await until(() => stats().callbackEntered)
  }
  await worker.terminate()
  const collectedBeforeCleanup = stats().resultCalls
  // Node may drain an already-queued callback on its still-valid owner
  // apartment before running cleanup hooks. Late callbacks must not collect.
  if (scenario === 'teardown-queued') assert.ok(collectedBeforeCleanup <= 1)
  else assert.equal(collectedBeforeCleanup, 0)
  native.comCompletionTestRelease()
  await until(() => stats().operationCreated === stats().operationDropped && stats().signalsLive === 0)
  const s = stats()
  assert.equal(s.callbacks, 1)
  assert.equal(s.mtaCallbacks, 1)
  assert.equal(s.callbackErrors, 0)
  assert.equal(s.callbackHresult, 0)
  assert.equal(s.resultCreated, collectedBeforeCleanup)
  assert.equal(s.resultDropped, collectedBeforeCleanup)
  assert.equal(s.resultCalls, collectedBeforeCleanup)
  assert.equal(s.wrongThread, 0)
  assert.equal(s.marshalingErrors, 0)
} else {
  throw new Error(`Unknown scenario ${scenario}`)
}

if (scenario !== 'liveness') console.log(`native-completion:${scenario}:ok`)
