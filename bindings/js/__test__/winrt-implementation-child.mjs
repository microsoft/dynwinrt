// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { once } from 'node:events'
import { createRequire } from 'node:module'
import { setImmediate as tick } from 'node:timers/promises'
import { Worker } from 'node:worker_threads'

const runtimePath = process.env.DYNWINRT_TEST_RUNTIME
const runtime = createRequire(import.meta.url)(runtimePath)
const {
  DynWinRtImplementation: Implementation,
  DynWinRtInterfacePlan: Plan,
  DynWinRtType: Type,
  DynWinRtMethodSig: Signature,
  DynWinRtValue: Value,
  WinGuid,
} = runtime
runtime.roInitialize(1)

const mode = process.argv[2]
const iid = WinGuid.parse('96369f54-8eb6-48f0-abce-c1b211e627c3')
const signature = new Signature().addOut(Type.hstring())
const plan = Plan.create('Windows.Foundation.IStringable', Type.interface(iid), [
  { name: 'ToString', vtableIndex: 6, signature },
])
const callbackFor = (sentinel) => () => [Value.hstring(sentinel.text)]
const stats = () => runtime.winrtImplementationTestStats()

async function collect(reference) {
  for (let i = 0; i < 100; i++) {
    await tick()
    global.gc()
    await tick()
    if (reference.deref() === undefined) return true
  }
  return false
}

if (mode === 'retention') {
  function retain() {
    const sentinel = { text: 'native-retained callback' }
    const owner = Implementation.create([plan], callbackFor(sentinel))
    const value = owner.toValue()
    runtime.winrtImplementationTestRetain(value)
    value.release()
    return { owner: new WeakRef(owner), sentinel: new WeakRef(sentinel) }
  }
  const references = retain()
  assert.equal(await collect(references.owner), true, 'the JS owner should be collectable')
  assert.notEqual(references.sentinel.deref(), undefined, 'a native reference retains the callback')
  assert.equal(runtime.winrtImplementationTestInvokeRetained().text, 'native-retained callback')
  assert.equal(runtime.winrtImplementationTestInvokeRetained(true).hresult, 0x8001010e | 0)
  runtime.winrtImplementationTestReleaseRetained(true)
  assert.equal(await collect(references.sentinel), true, 'foreign final release frees callback roots on the JS thread')
  assert.equal(stats().liveControls, 0)
  assert.equal(stats().finalizedCallbacks, 1)
} else if (mode === 'cycle') {
  function createCycle() {
    const sentinel = { text: 'cycle' }
    const owner = Implementation.create([plan], () => {
      assert.equal(owner.isClosed, false)
      return [Value.hstring(sentinel.text)]
    })
    return { owner: new WeakRef(owner), sentinel: new WeakRef(sentinel) }
  }
  const references = createCycle()
  for (let i = 0; i < 5; i++) {
    await tick()
    global.gc()
  }
  assert.notEqual(references.owner.deref(), undefined, 'a callback that captures its owner is a cross-runtime cycle')
  assert.equal(stats().liveControls, 1)
  references.owner.deref().dispose()
  assert.equal(await collect(references.owner), true, 'dispose breaks the owner/callback cycle')
  assert.equal(await collect(references.sentinel), true)
  assert.equal(stats().liveControls, 0)
} else if (mode === 'reentrant-release') {
  const owner = Implementation.create([plan], () => {
    runtime.winrtImplementationTestReleaseRetained()
    return [Value.hstring('completed after release')]
  })
  const value = owner.toValue()
  runtime.winrtImplementationTestRetain(value)
  value.release()
  owner.release()
  assert.equal(runtime.winrtImplementationTestInvokeRetained().text, 'completed after release')
  assert.equal(owner.isClosed, true)
  for (let i = 0; i < 5; i++) await tick()
  assert.equal(stats().liveControls, 0)
} else if (mode === 'delegate-release' || mode === 'delegate-value-release') {
  const delegateIid = WinGuid.parse('749139bd-84a7-4abd-b17d-871a77c5c39f')
  const signature = new Signature().addIn(Type.i32())
  const method = runtime.DynWinRtDelegateMethod.create(delegateIid, signature)
  const callbackForValue = (box, sentinel) => (argument) => {
    assert.equal(argument.toNumber(), 42)
    sentinel.called = true
    box.value.release()
  }
  function createDelegate() {
    const box = {}
    const sentinel = { called: false }
    const delegate = runtime.DynWinRtDelegate.create(delegateIid, [Type.i32()], callbackForValue(box, sentinel))
    box.value = delegate.toValue()
    return { owner: new WeakRef(delegate), sentinel: new WeakRef(sentinel), box }
  }
  const references = createDelegate()
  assert.equal(await collect(references.owner), true)
  assert.equal(runtime.tsfnTestRegisteredHandleCount(), 1)
  const outputs =
    mode === 'delegate-release'
      ? method.invoke(references.box.value, [Value.i32(42)])
      : references.box.value.invokeDelegate(delegateIid, signature, [Value.i32(42)])
  assert.deepEqual(outputs, [])
  assert.equal(references.box.value.isNull(), true)
  assert.equal(await collect(references.sentinel), true, 'the invocation pin releases the final native reference')
  assert.equal(runtime.tsfnTestRegisteredHandleCount(), 0)
} else if (mode === 'shutdown') {
  const before = stats()
  for (const terminate of [false, true]) {
    const worker = new Worker(new URL('./winrt-implementation-worker.mjs', import.meta.url), {
      workerData: { runtimePath, terminate },
    })
    const exited = once(worker, 'exit')
    const [message] = await once(worker, 'message')
    assert.equal(message, 'retained')
    if (terminate) await worker.terminate()
    await exited
    const result = runtime.winrtImplementationTestInvokeRetained()
    assert.notEqual(result.hresult, 0, 'a stale environment must never be invoked')
    assert.equal(result.text ?? null, null)
    runtime.winrtImplementationTestReleaseRetained(true)
  }
  assert.equal(stats().environmentDisconnects, before.environmentDisconnects + 2)
  assert.equal(stats().finalizedCallbacks, before.finalizedCallbacks + 2)
} else if (mode === 'natural-exit') {
  globalThis.retainedImplementation = Implementation.create([plan], callbackFor({ text: 'exit' }))
} else {
  throw new Error(`Unknown child mode: ${mode}`)
}
console.log(`winrt-${mode}-ok`)
