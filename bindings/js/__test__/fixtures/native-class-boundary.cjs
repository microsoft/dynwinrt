// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const assert = require('node:assert/strict')
const { Worker, parentPort } = require('node:worker_threads')
const root = require('../../dist/winrt.js')
const com = require('../../dist/com-unsafe.js')
const win32 = require('../../dist/win32-unsafe.js')
const { DynWinRtValue: Value, DynWinRtType: Type } = root
const { DynWin32: W, DynWin32Function: F } = win32

root.roInitialize(1)
let rejected = 0
function report(details = {}) {
  const result = { mode, rejected, arch: process.arch, node: process.version, ...details }
  if (parentPort) parentPort.postMessage(result)
  else console.log(JSON.stringify(result))
}

function reject(call, name = 'DynWinRTValue') {
  assert.throws(call, (error) => {
    assert.match(error.message, /Failed to recover|Failed to unwrap|not an instance of class/)
    assert.ok(error.message.includes(name), error.message)
    return true
  })
  rejected++
}

function openOwnProcess() {
  const result = F.bind({
    dll: 'kernel32.dll',
    entryPoint: 'OpenProcess',
    parameters: [
      { type: 'u32', direction: 'in' },
      { type: 'bool32', direction: 'in' },
      { type: 'u32', direction: 'in' },
    ],
    returnType: 'handle',
    returnCleanup: 'closeHandle',
    successRule: 'nonnull',
    captureLastError: true,
  }).invoke([W.u32(0x1000), W.bool32(false), W.u32(process.pid)])
  assert.equal(result.succeeded, true, `OpenProcess failed: ${result.lastError}`)
  const resource = W.toResource(result.returnValue)
  assert.ok(resource)
  assert.equal(resource.closed, false)
  return resource
}

function checkResourceUsable(resource) {
  const result = F.bind({
    dll: 'kernel32.dll',
    entryPoint: 'GetProcessId',
    parameters: [{ type: 'handle', direction: 'in', resourceCleanup: 'closeHandle' }],
    returnType: 'u32',
  }).invoke([W.resource(resource, 'closeHandle')])
  assert.equal(W.toNumber(result.returnValue), process.pid)
  assert.equal(resource.closed, false)
}

const mode = process.argv[2]
if (mode === 'resource' || mode === 'resource-control') {
  const resource = openOwnProcess()
  try {
    checkResourceUsable(resource)
    if (mode === 'resource') reject(() => Value.createVector([resource], Type.object()))
    checkResourceUsable(resource)
  } finally {
    resource.close()
  }
  assert.equal(resource.closed, true)
  report({ closes: 1 })
} else if (mode === 'controls') {
  const nil = Value.nullValue()
  const vector = Value.createVector([nil], Type.object())
  assert.equal(vector.isNull(), false)
  assert.equal(nil.isNull(), true)
  vector.release()
  assert.equal(root.DynWinRtValue, com.DynWinRtValue)
  assert.equal(root.DynWinRtValue, win32.DynWinRtValue)
  for (const foreign of [
    {},
    new root.DynWinRtMethodSig(),
    root.WinGuid.parse('00000000-0000-0000-c000-000000000046'),
  ]) {
    reject(() => Value.createVector([foreign], Type.object()))
  }
  reject(() => Value.createVector([], nil), 'DynWinRTType')
  nil.release()
  report()
} else if (mode === 'identity') {
  const foreign = new root.DynWinRtMethodSig()
  const originalPrototype = Object.getPrototypeOf(foreign)
  let userCalls = 0
  const unexpected = () => {
    userCalls++
    throw new Error('Identity validation must not call user code')
  }
  const nil = Value.nullValue()
  const constructor = Object.getOwnPropertyDescriptor(Value.prototype, 'constructor')
  Object.defineProperty(nil, 'isNull', { value: unexpected })
  Object.defineProperty(Value.prototype, 'constructor', { get: unexpected, configurable: true })
  Object.defineProperty(Value, Symbol.hasInstance, { value: () => true, configurable: true })
  try {
    for (const invalid of [
      Object.create(Value.prototype),
      { constructor: Value, isNull: unexpected },
      Object.setPrototypeOf(foreign, Value.prototype),
    ]) {
      reject(() => Value.createVector([invalid], Type.object()))
    }
    const vector = Value.createVector([nil], Type.object())
    vector.release()
    assert.equal(userCalls, 0)
  } finally {
    Object.setPrototypeOf(foreign, originalPrototype)
    Object.defineProperty(Value.prototype, 'constructor', constructor)
    delete Value[Symbol.hasInstance]
    nil.release()
  }
  report({ userCalls })
} else if (mode === 'collections') {
  const resource = openOwnProcess()
  const key = Value.hstring('key')
  const value = Value.hstring('value')
  const nil = Value.nullValue()
  const stringType = Type.hstring()
  const signature = new root.DynWinRtMethodSig()
  try {
    reject(() => Value.createVector([resource], Type.object()))
    reject(() => Value.createMap([resource], [value], stringType, stringType))
    reject(() => Value.createMap([key], [resource], stringType, stringType))
    reject(() => Value.createVector([], signature), 'DynWinRTType')
    reject(() => Value.createMap([], [], resource, stringType), 'DynWinRTType')
    reject(() => Value.createMap([], [], stringType, nil), 'DynWinRTType')
    reject(() => signature.addIn(resource), 'DynWinRTType')
    reject(() => signature.addOut(nil), 'DynWinRTType')
    reject(() => Type.structType('Tests.RejectedNativeClass', [stringType, resource]), 'DynWinRTType')
    reject(() => Type.interface(resource), 'WinGUID')
    const guid = root.WinGuid.parse('fbf417fe-7a96-4b54-9997-3da55c9cdf8d')
    const iface = Type.registerInterface('Tests.NativeClassBoundary', guid)
    reject(() => iface.addMethod('Rejected', resource), 'DynWinRTMethodSig')
    iface.addMethod('Valid', signature.addOut(stringType))
    const map = Value.createMap([key], [value], stringType, stringType)
    assert.equal(map.isNull(), false)
    map.release()
    const vector = Value.createVector([nil], Type.object())
    vector.release()
    checkResourceUsable(resource)
  } finally {
    resource.close()
    key.release()
    value.release()
    nil.release()
  }
  report({ closes: 1 })
} else if (mode === 'accessors') {
  const Variant = com.DynComVariant
  const variant = Variant.i32(9)
  const resource = openOwnProcess()
  const nil = Value.nullValue()
  try {
    const vartype = Object.getOwnPropertyDescriptor(Variant.prototype, 'vartype').get
    const kind = Object.getOwnPropertyDescriptor(Variant.prototype, 'kind').get
    assert.equal(vartype.call(variant), 3)
    assert.equal(variant.toNumber(), 9)
    for (const foreign of [resource, nil, {}, Object.create(Variant.prototype)]) {
      reject(() => vartype.call(foreign), 'DynComVariant')
      reject(() => kind.call(foreign), 'DynComVariant')
    }
    reject(() => Variant.unknown(resource))
    const nullable = Variant.unknown(null)
    assert.equal(nullable.vartype, 13)
    nullable.release()
    checkResourceUsable(resource)
  } finally {
    variant.release()
    resource.close()
    nil.release()
  }
  assert.throws(() => variant.vartype, /released/i)
  report({ closes: 1 })
} else if (mode === 'bridges') {
  const resource = openOwnProcess()
  const nil = Value.nullValue()
  const variant = com.DynComVariant.i32(12)
  const stored = com.DynCom.variant(variant)
  try {
    reject(() => root.unboxObject(resource))
    reject(() => com.DynCom.takeVariant(resource))
    reject(() => com.DynCom.takeBuffer(resource))
    reject(() => new com.DynComMethodSig().addIn(Type.i32()), 'DynComType')
    reject(() => Value.createVector([variant], Type.object()))
    assert.equal(root.unboxObject(nil), null)
    const extracted = com.DynCom.takeVariant(stored)
    assert.equal(extracted.toNumber(), 12)
    extracted.release()
    class Signature extends root.DynWinRtMethodSig {}
    const signature = new Signature().addOut(Type.hstring())
    const iid = root.WinGuid.parse('96369f54-8eb6-48f0-abce-c1b211e627c3')
    const type = Type.registerInterface('Windows.Foundation.IStringable', iid).addMethod('ToString', signature)
    reject(
      () =>
        root.DynWinRtInterfacePlan.create('Invalid', type, [{ name: 'ToString', vtableIndex: 6, signature: resource }]),
      'MethodSig',
    )
    reject(() => root.DynWinRtDelegateMethod.create(iid, resource), 'MethodSig')
    const plan = root.DynWinRtInterfacePlan.create('Windows.Foundation.IStringable', type, [
      { name: 'ToString', vtableIndex: 6, signature },
    ])
    let calls = 0
    let invalid = true
    const owner = root.DynWinRtImplementation.create([plan], () => {
      calls++
      return [invalid ? resource : Value.hstring('still usable')]
    })
    const object = owner.toValue().cast(iid)
    try {
      assert.throws(() => type.method(6).getString(object))
      assert.match(owner.takeError(), /DynWinRtValue|class/i)
      invalid = false
      assert.equal(type.method(6).getString(object), 'still usable')
      assert.equal(calls, 2)
    } finally {
      object.release()
      owner.dispose()
      owner.release()
    }
    checkResourceUsable(resource)
  } finally {
    resource.close()
    nil.release()
    stored.release()
    variant.release()
  }
  report({ closes: 1 })
} else if (mode === 'native-hooks') {
  const native = require('../../dist/index.js')
  const Probe = native.NativeClassTestProbe
  assert.equal(typeof Probe, 'function', 'An explicitly built test-hooks addon is required')
  assert.equal(typeof global.gc, 'function', '--expose-gc is required')
  const calls = [
    (value) => Probe.read(value),
    (value) => Probe.write(value),
    (value) => Probe.alias(value),
    (value) => Probe.instance(value),
    (value) => Probe.reference(value),
    (value) => native.nativeClassTestBridge(value),
  ]
  const start = native.nativeClassTestEntries()
  const finalized = native.nativeClassTestFinalized()
  ;(() => {
    const controls = [native.nativeClassTestUntagged(), native.nativeClassTestOtherBuild()]
    for (const value of controls) {
      Object.setPrototypeOf(value, Value.prototype)
      for (const call of calls) reject(() => call(value))
    }
  })()
  assert.equal(native.nativeClassTestEntries(), start, 'Rejected arguments entered a native body')
  const nil = Value.nullValue()
  for (const call of calls) call(nil)
  assert.equal(native.nativeClassTestEntries(), start + calls.length)
  const probe = new Probe()
  probe.value = 17
  assert.equal(probe.value, 17)
  const beforeAccessors = native.nativeClassTestEntries()
  const descriptor = Object.getOwnPropertyDescriptor(Probe.prototype, 'value')
  reject(() => descriptor.get.call(nil), 'NativeClassTestProbe')
  reject(() => descriptor.set.call(nil, 1), 'NativeClassTestProbe')
  assert.equal(native.nativeClassTestEntries(), beforeAccessors, 'Rejected receiver entered an accessor body')
  nil.release()
  assert.throws(() => native.nativeClassTestDuplicateTag(), /tag.*identity/i)
  ;(async () => {
    for (let attempt = 0; attempt < 20 && native.nativeClassTestFinalized() - finalized !== 3; attempt++) {
      global.gc()
      await new Promise(setImmediate)
    }
    assert.equal(native.nativeClassTestFinalized() - finalized, 3, 'Control payloads must each finalize once')
    report({ finalized: 3, entriesForInvalidInputs: 0 })
  })().catch((error) => {
    console.error(error)
    process.exitCode = 1
  })
} else if (mode === 'workers') {
  Promise.all(
    Array.from(
      { length: 4 },
      () =>
        new Promise((resolve, rejectWorker) => {
          const worker = new Worker(__filename, { argv: ['controls'] })
          let result
          worker.on('message', (message) => {
            result = message
          })
          worker.on('error', rejectWorker)
          worker.on('exit', (code) => {
            if (code !== 0 || !result) rejectWorker(new Error(`Worker exited ${code} without its report`))
            else resolve(result)
          })
        }),
    ),
  )
    .then((results) => {
      for (const result of results) {
        assert.equal(result.mode, 'controls')
        rejected += result.rejected
      }
      const nil = Value.nullValue()
      const vector = Value.createVector([nil], Type.object())
      vector.release()
      nil.release()
      report({ workers: results.length })
    })
    .catch((error) => {
      console.error(error)
      process.exitCode = 1
    })
} else {
  throw new Error(`Unknown native class boundary mode: ${mode}`)
}
