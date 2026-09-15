// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

'use strict'

const assert = require('node:assert/strict')
const { readFileSync } = require('node:fs')
const { join, resolve } = require('node:path')
const { runInNewContext } = require('node:vm')

const dist = process.argv[2] ? resolve(process.argv[2]) : resolve(__dirname, '..', '..', 'dist')
const tags = new WeakMap()
const owners = new WeakMap()
const diagnostics = new WeakMap()
function tagged(value, kind) {
  tags.set(value, kind)
  return value
}
function requireKind(value, kind, name) {
  if (tags.get(value) !== kind) throw new Error(`Expected a native ${name} carrier`)
}
const native = {
  DynWin32: class {},
  DynWin32Unsafe: class {},
  win32CarrierKind: (value) => tags.get(value) || 0,
  win32Bind: () => tagged({}, 4),
  win32CallErrorCleanupFailures(value) {
    requireKind(value, 8, 'DynWin32CallError')
    return diagnostics.get(value)
  },
  win32CallErrorRetryCleanup(value) {
    requireKind(value, 8, 'DynWin32CallError')
    for (const record of diagnostics.get(value)) owners.get(record.resource).closed = true
  },
  win32ResourceClosed: (value) => owners.get(value).closed,
  win32ResourceClose(value) {
    owners.get(value).closed = true
  },
}
const facadeModule = { exports: {} }
runInNewContext(readFileSync(join(dist, 'win32-internal.js'), 'utf8'), {
  module: facadeModule,
  require(name) {
    assert.equal(name, './index.js')
    return native
  },
  Error,
  TypeError,
  AggregateError,
})
const { DynWin32CallError, DynWin32Function, DynWin32Resource } = facadeModule.exports
const fn = DynWin32Function.bind({})
const invokeModes = [() => fn.invoke([]), () => fn.invokeWithSubsystem({}, 'winsock', [])]
function capture(call) {
  try {
    call()
  } catch (error) {
    return error
  }
  throw new Error('Expected a thrown value')
}
function throwFromNative(error) {
  native.win32Invoke = native.win32InvokeWithSubsystem = () => {
    throw error
  }
}
function failure() {
  const error = tagged(new Error('original native cleanup error'), 8)
  const resource = tagged({}, 3)
  owners.set(resource, { closed: false })
  diagnostics.set(error, [
    {
      target: { kind: 'parameter', index: 2 },
      error: { code: -2147024890, message: 'original cleanup diagnostic' },
      resource,
    },
  ])
  return error
}

assert.ok(DynWin32CallError.prototype instanceof Error)
assert.throws(() => new DynWin32CallError(), /native dispatch/)
for (const invoke of invokeModes) {
  const error = failure()
  throwFromNative(error)
  assert.equal(capture(invoke), error)
  assert.ok(error instanceof DynWin32CallError)
  assert.equal(error.message, 'original native cleanup error')
  const records = error.cleanupFailures
  assert.equal(error.cleanupFailures, records)
  assert.ok(Object.isFrozen(records))
  assert.ok(Object.isFrozen(records[0]))
  assert.ok(Object.isFrozen(records[0].target))
  assert.ok(Object.isFrozen(records[0].error))
  assert.ok(records[0].resource instanceof DynWin32Resource)
  assert.equal(Reflect.set(error, 'cleanupFailures', []), false)
  assert.equal(Reflect.set(records[0], 'resource', {}), false)
  assert.equal(Reflect.set(records[0].error, 'message', 'changed'), false)
  assert.equal(records[0].resource.closed, false)
  error.retryCleanup()
  assert.equal(records[0].resource.closed, true)
  assert.equal(error.cleanupFailures, records)

  const frozenError = failure()
  Object.preventExtensions(frozenError)
  throwFromNative(frozenError)
  const aggregate = capture(invoke)
  assert.ok(aggregate instanceof AggregateError)
  assert.equal(aggregate.cause, frozenError)
  assert.equal(aggregate.errors[0], frozenError)
  assert.equal(aggregate.errors[1].name, 'TypeError')
  const recovery = aggregate.cleanupFailures[0].resource
  aggregate.retryCleanup()
  assert.equal(recovery.closed, true)
}

const recordError = failure()
Object.preventExtensions(diagnostics.get(recordError)[0].resource)
throwFromNative(recordError)
assert.equal(capture(invokeModes[0]), recordError)
const recordProjection = capture(() => recordError.cleanupFailures)
assert.ok(recordProjection instanceof AggregateError)
assert.equal(recordProjection.cause, recordError)
assert.equal(recordProjection.errors[0], recordError)
recordProjection.retryCleanup()
assert.equal(owners.get(diagnostics.get(recordError)[0].resource).closed, true)

const nativeProjection = failure()
throwFromNative(nativeProjection)
assert.equal(capture(invokeModes[0]), nativeProjection)
const readRecords = native.win32CallErrorCleanupFailures
const diagnosticError = new Error('N-API result projection failed')
native.win32CallErrorCleanupFailures = () => {
  throw diagnosticError
}
const retained = capture(() => nativeProjection.cleanupFailures)
assert.equal(retained.cause, nativeProjection)
assert.equal(retained.errors[1], diagnosticError)
retained.retryCleanup()
native.win32CallErrorCleanupFailures = readRecords
assert.equal(retained.cleanupFailures[0].resource.closed, true)

let coercions = 0
const hostile = {
  toString() {
    coercions++
    throw new Error('unexpected toString')
  },
  get cause() {
    coercions++
    throw new Error('unexpected cause')
  },
}
for (const value of [
  new Error('ordinary'),
  hostile,
  Object.create(DynWin32CallError.prototype),
  'string',
  1,
  1n,
  Symbol('value'),
  null,
  undefined,
  false,
  () => {},
]) {
  throwFromNative(value)
  for (const invoke of invokeModes) assert.equal(capture(invoke), value)
}
assert.equal(coercions, 0)
const cleanupGetter = Object.getOwnPropertyDescriptor(DynWin32CallError.prototype, 'cleanupFailures').get
for (const forged of [hostile, Object.create(DynWin32CallError.prototype), tagged({}, 1)]) {
  assert.throws(() => cleanupGetter.call(forged), /native DynWin32CallError/)
  assert.throws(() => DynWin32CallError.prototype.retryCleanup.call(forged), /native DynWin32CallError/)
}

const declaration = readFileSync(join(dist, 'index.d.ts'), 'utf8')
assert.match(declaration, /export declare class DynWin32CallError extends Error \{/)
assert.match(declaration, /readonly cleanupFailures: readonly DynWin32CleanupFailure\[\]/)
assert.match(declaration, /readonly kind: 'aggregate-field'; readonly parameter: number; readonly field: number/)
for (const facade of ['win32', 'win32-unsafe']) {
  const text = readFileSync(join(dist, `${facade}.d.ts`), 'utf8')
  assert.match(text, /\bDynWin32CallError\b/)
  assert.match(text, /export type \{ DynWin32CleanupFailure, DynWin32ResultTarget \}/)
  assert.doesNotMatch(text, /\bwin32TestCleanupFailure\b/)
}
for (const facade of ['winrt', 'com', 'com-unsafe', 'com-unsafe-raw']) {
  assert.doesNotMatch(
    readFileSync(join(dist, `${facade}.d.ts`), 'utf8'),
    /DynWin32|win32CallError|win32TestCleanupFailure/,
  )
  assert.doesNotMatch(
    readFileSync(join(dist, `${facade}.js`), 'utf8'),
    /DynWin32|win32CallError|win32TestCleanupFailure/,
  )
}
console.log('PASS Win32 cleanup error facade')
