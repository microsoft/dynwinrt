// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

'use strict'

const assert = require('node:assert/strict')
const native = require('../../dist/index.js')

if (typeof native.win32TestCleanupFailure !== 'function') {
  throw new Error('Win32 cleanup recovery requires the test-hooks addon')
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})

async function main() {
  const { DynWin32, DynWin32CallError, DynWin32Function, DynWin32Resource } = require('../../dist/win32-unsafe.js')
  assert.equal(typeof global.gc, 'function')
  const setInformation = DynWin32Function.bind({
    dll: 'kernel32.dll',
    entryPoint: 'SetHandleInformation',
    parameters: [
      { type: 'handle', direction: 'in', resourceCleanup: 'closeHandle' },
      { type: 'u32', direction: 'in' },
      { type: 'u32', direction: 'in' },
    ],
    returnType: 'bool32',
    successRule: 'nonzero',
    captureLastError: true,
  })
  const getInformation = DynWin32Function.bind({
    dll: 'kernel32.dll',
    entryPoint: 'GetHandleInformation',
    parameters: [
      { type: 'handle', direction: 'in', resourceCleanup: 'closeHandle' },
      { type: 'u32', direction: 'out' },
    ],
    returnType: 'bool32',
    successRule: 'nonzero',
    captureLastError: true,
  })
  const fn = DynWin32Function.bind({
    dll: 'kernel32.dll',
    entryPoint: 'GetLastError',
    parameters: [],
    returnType: 'u32',
  })
  const unprotect = (resource) => {
    const result = setInformation.invoke([DynWin32.handle(resource), DynWin32.u32(2), DynWin32.u32(0)])
    assert.equal(result.succeeded, true, `SetHandleInformation: ${result.lastError}`)
  }
  const inspect = (resource) => getInformation.invoke([DynWin32.handle(resource)])
  const capture = (call) => {
    try {
      call()
    } catch (error) {
      return error
    }
    throw new Error('Expected a native cleanup failure')
  }
  const resourceOf = (error) => native.win32CallErrorCleanupFailures(error)[0].resource
  const retire = (error, resource) => {
    if (!native.win32ResourceClosed(resource)) {
      unprotect(resource)
      native.win32CallErrorRetryCleanup(error)
    }
  }
  const throughFacade = (error, subsystem) => {
    const name = subsystem ? 'win32InvokeWithSubsystem' : 'win32Invoke'
    const original = native[name]
    // The real test-only ABI supplies the failure; only facade routing is replaced.
    native[name] = () => {
      throw error
    }
    try {
      return capture(() => (subsystem ? fn.invokeWithSubsystem(subsystem, 'winsock', []) : fn.invoke([])))
    } finally {
      native[name] = original
    }
  }

  const originalToString = Object.getOwnPropertyDescriptor(Error.prototype, 'toString')
  const originalCause = Object.getOwnPropertyDescriptor(Error.prototype, 'cause')
  let coercions = 0
  let raw
  try {
    Object.defineProperty(Error.prototype, 'toString', {
      configurable: true,
      value() {
        coercions++
        throw new Error('Unexpected Error.toString during transport')
      },
    })
    Object.defineProperty(Error.prototype, 'cause', {
      configurable: true,
      get() {
        coercions++
        throw new Error('Unexpected Error.cause during transport')
      },
    })
    raw = capture(() => native.win32TestCleanupFailure())
  } finally {
    Object.defineProperty(Error.prototype, 'toString', originalToString)
    if (originalCause) Object.defineProperty(Error.prototype, 'cause', originalCause)
    else delete Error.prototype.cause
  }
  const retained = resourceOf(raw)
  try {
    assert.equal(coercions, 0)
    assert.ok(raw instanceof Error)
    assert.equal(native.win32CarrierKind(raw), 8)
    const nativeMessage = raw.message
    const error = throughFacade(raw)
    assert.equal(error, raw)
    assert.ok(error instanceof Error)
    assert.ok(error instanceof DynWin32CallError)
    assert.equal(error.name, 'DynWin32CallError')
    assert.equal(error.message, nativeMessage)
    const records = error.cleanupFailures
    assert.equal(records.length, 1)
    assert.equal(error.cleanupFailures, records)
    const record = records[0]
    const resource = record.resource
    assert.deepEqual(record.target, { kind: 'return' })
    assert.equal(record.error.code >>> 0, 0x80070006)
    assert.equal(error.message, `DynWin32Function test!cleanupFailure: 0x80070006: ${record.error.message}`)
    assert.ok(resource instanceof DynWin32Resource)
    assert.equal(typeof resource.value, 'bigint')
    assert.ok(resource.value > 0n)
    assert.equal(resource.closed, false)
    for (const value of [records, record, record.target, record.error]) assert.ok(Object.isFrozen(value))
    assert.equal(Reflect.set(error, 'cleanupFailures', []), false)
    assert.equal(Reflect.set(record, 'resource', {}), false)
    assert.equal(Reflect.set(record.target, 'kind', 'parameter'), false)
    assert.equal(Reflect.set(record.error, 'message', 'replacement'), false)

    const alias = DynWin32.toResource(DynWin32.handle(resource))
    assert.ok(alias instanceof DynWin32Resource)
    assert.equal(alias.value, resource.value)
    assert.throws(() => error.retryCleanup())
    assert.throws(() => resource.close())
    assert.equal(error.cleanupFailures, records)
    assert.equal(error.cleanupFailures[0].resource, resource)
    assert.equal(resource.closed, false)
    assert.equal(alias.closed, false)
    const receiverMessage = Object.getOwnPropertyDescriptor(error, 'message').get
    assert.throws(() => receiverMessage.call(DynWin32.u32(1)), /native DynWin32CallError/)
    Object.setPrototypeOf(error, DynWin32Resource.prototype)
    assert.throws(() => DynWin32Resource.prototype.close.call(error), /native DynWin32Resource/)
    assert.throws(() => DynWin32.resource(error, 'closeHandle'), /native DynWin32Resource/)
    Object.setPrototypeOf(error, DynWin32CallError.prototype)

    const handle = resource.value
    unprotect(resource)
    error.retryCleanup()
    assert.equal(resource.closed, true)
    assert.equal(alias.closed, true)
    assert.equal(native.win32ResourceClosed(retained), true)
    assert.equal(error.cleanupFailures, records)
    assert.equal(error.cleanupFailures[0].error, record.error)
    assert.equal(inspect(handle).succeeded, false)
    assert.doesNotThrow(() => error.retryCleanup())
    assert.doesNotThrow(() => resource.close())
    assert.doesNotThrow(() => alias.close())
  } finally {
    retire(raw, retained)
  }

  const context = DynWin32.initializeWinsock()
  const subsystemError = capture(() => native.win32TestCleanupFailure())
  const subsystemResource = resourceOf(subsystemError)
  try {
    assert.equal(throughFacade(subsystemError, context), subsystemError)
    assert.ok(subsystemError instanceof DynWin32CallError)
    const resource = subsystemError.cleanupFailures[0].resource
    const alias = DynWin32.toResource(DynWin32.handle(resource))
    unprotect(resource)
    alias.close()
    assert.equal(resource.closed, true)
    assert.doesNotThrow(() => subsystemError.retryCleanup())
  } finally {
    retire(subsystemError, subsystemResource)
    context.close()
  }

  const unprojectable = capture(() => native.win32TestCleanupFailure())
  const unprojectableResource = resourceOf(unprojectable)
  try {
    Object.preventExtensions(unprojectable)
    const aggregate = throughFacade(unprojectable)
    assert.ok(aggregate instanceof AggregateError)
    assert.equal(aggregate.cause, unprojectable)
    assert.equal(aggregate.errors[0], unprojectable)
    assert.ok(aggregate.errors[1] instanceof TypeError)
    const resource = aggregate.cleanupFailures[0].resource
    assert.ok(resource instanceof DynWin32Resource)
    assert.throws(() => aggregate.retryCleanup())
    assert.equal(resource.closed, false)
    unprotect(resource)
    aggregate.retryCleanup()
    assert.equal(resource.closed, true)
    assert.doesNotThrow(() => aggregate.retryCleanup())
  } finally {
    retire(unprojectable, unprojectableResource)
  }

  const recordError = capture(() => native.win32TestCleanupFailure())
  const recordResource = resourceOf(recordError)
  const readRecords = native.win32CallErrorCleanupFailures
  try {
    assert.equal(throughFacade(recordError), recordError)
    native.win32CallErrorCleanupFailures = (error) => {
      const records = readRecords(error)
      Object.preventExtensions(records[0].resource)
      return records
    }
    const aggregate = capture(() => recordError.cleanupFailures)
    assert.ok(aggregate instanceof AggregateError)
    assert.equal(aggregate.cause, recordError)
    assert.equal(aggregate.errors[0], recordError)
    assert.ok(aggregate.errors[1] instanceof TypeError)
    unprotect(recordResource)
    aggregate.retryCleanup()
    assert.equal(native.win32ResourceClosed(recordResource), true)
    native.win32CallErrorCleanupFailures = readRecords
    assert.equal(aggregate.cleanupFailures[0].resource.closed, true)
  } finally {
    native.win32CallErrorCleanupFailures = readRecords
    retire(recordError, recordResource)
  }

  async function collectUntil(condition, description) {
    const deadline = Date.now() + 5000
    do {
      await new Promise((resolve) => setImmediate(resolve))
      global.gc()
      await new Promise((resolve) => setImmediate(resolve))
      if (condition()) return
    } while (Date.now() < deadline)
    throw new Error(`GC did not retire ${description}`)
  }

  const errorOnly = capture(() => native.win32TestCleanupFailure())
  const errorOnlyState = (() => {
    const resource = resourceOf(errorOnly)
    return { handle: native.win32ResourceValue(resource), weak: new WeakRef(resource) }
  })()
  try {
    await collectUntil(() => errorOnlyState.weak.deref() === undefined, 'the unretained resource projection')
    assert.equal(inspect(errorOnlyState.handle).succeeded, true)
    unprotect(errorOnlyState.handle)
    global.gc()
    assert.equal(inspect(errorOnlyState.handle).succeeded, true, 'The native CallError alone retains ownership')
    native.win32CallErrorRetryCleanup(errorOnly)
    assert.equal(inspect(errorOnlyState.handle).succeeded, false)
  } finally {
    retire(errorOnly, resourceOf(errorOnly))
  }

  const aliasOnly = (() => {
    const error = throughFacade(capture(() => native.win32TestCleanupFailure()))
    return { weak: new WeakRef(error), resource: error.cleanupFailures[0].resource }
  })()
  try {
    await collectUntil(() => aliasOnly.weak.deref() === undefined, 'the error while an alias remains')
    assert.equal(aliasOnly.resource.closed, false)
    const handle = aliasOnly.resource.value
    unprotect(aliasOnly.resource)
    aliasOnly.resource.close()
    assert.equal(aliasOnly.resource.closed, true)
    assert.equal(inspect(handle).succeeded, false)
  } finally {
    if (!aliasOnly.resource.closed) {
      unprotect(aliasOnly.resource)
      aliasOnly.resource.close()
    }
  }

  const finalizerOnly = (() => {
    const error = capture(() => native.win32TestCleanupFailure())
    const resource = resourceOf(error)
    const handle = native.win32ResourceValue(resource)
    unprotect(resource)
    return { handle, error: new WeakRef(error), resource: new WeakRef(resource) }
  })()
  try {
    await collectUntil(
      () =>
        finalizerOnly.error.deref() === undefined &&
        finalizerOnly.resource.deref() === undefined &&
        !inspect(finalizerOnly.handle).succeeded,
      'the finalizer-backed cleanup owner',
    )
    assert.equal(inspect(finalizerOnly.handle).lastError, 6)
  } finally {
    const error = finalizerOnly.error.deref()
    if (error) native.win32CallErrorRetryCleanup(error)
    const resource = finalizerOnly.resource.deref()
    if (resource) native.win32ResourceClose(resource)
  }
  console.log('PASS Win32 cleanup error native recovery and lifetime')
}
