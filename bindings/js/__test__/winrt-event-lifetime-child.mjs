// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import {
  DynWinRtDelegate,
  DynWinRtImplementation,
  DynWinRtInterfacePlan,
  DynWinRtMethodSig,
  DynWinRtStruct,
  DynWinRtType,
  DynWinRtValue,
  WinGuid,
  roInitialize,
} from '../dist/winrt.js'

roInitialize(1)
const mode = process.argv[2]
const referenceIid = WinGuid.parse('fbc4dd29-245b-11e4-af98-689423260cf8')
const closableIid = WinGuid.parse('30d5a829-7fa4-4026-83bb-d75bae4ea99e')
const delegateType = DynWinRtType.parameterized(
  WinGuid.parse('9de1c534-6ae1-11e0-84e1-18a905bcc53f'),
  [DynWinRtType.interface(referenceIid), DynWinRtType.object()],
)
const delegateIid = delegateType.iid()
const delegateArgs = [DynWinRtType.interface(referenceIid), DynWinRtType.object()]
const invokeSignature = new DynWinRtMethodSig().addIn(delegateArgs[0]).addIn(delegateArgs[1])
const tokenType = DynWinRtType.structType('Windows.Foundation.EventRegistrationToken', [DynWinRtType.i64()])
const methods = [
  { name: 'get_Capacity', vtableIndex: 6, signature: new DynWinRtMethodSig().addOut(DynWinRtType.u32()) },
  { name: 'add_Closed', vtableIndex: 7, signature: new DynWinRtMethodSig().addIn(delegateType).addOut(tokenType) },
  { name: 'remove_Closed', vtableIndex: 8, signature: new DynWinRtMethodSig().addIn(tokenType) },
]
let referenceType = DynWinRtType.registerInterface('Windows.Foundation.IMemoryBufferReference', referenceIid)
for (const method of methods) referenceType = referenceType.addMethod(method.name, method.signature)
const closeSignature = new DynWinRtMethodSig()
const closableType = DynWinRtType.registerInterface('Windows.Foundation.IClosable', closableIid)
  .addMethod('Close', closeSignature)
const referencePlan = DynWinRtInterfacePlan.create(
  'Windows.Foundation.IMemoryBufferReference', referenceType, methods, [closableIid],
)
const closePlan = DynWinRtInterfacePlan.create('Windows.Foundation.IClosable', closableType, [
  { name: 'Close', vtableIndex: 6, signature: closeSignature },
])

let retained
let sender
let delivered = 0
let removed = 0
const owner = DynWinRtImplementation.create([referencePlan, closePlan], (index, slot, args) => {
  if (index === 1) {
    retained?.invokeDelegate(delegateIid, invokeSignature, [sender, DynWinRtValue.nullValue()])
    return []
  }
  if (slot === 6) return [DynWinRtValue.u32(4096)]
  if (slot === 7) {
    if (mode === 'add-failure') throw new Error('expected subscription rejection')
    retained = args[0]
    const token = DynWinRtStruct.create(tokenType)
    token.setI64(0, 1n)
    return [token.toValue()]
  }
  assert.equal(args[0].asStruct().getI64(0), 1n)
  retained.release()
  retained = undefined
  removed++
  return []
})
const canonical = owner.toValue()
sender = canonical.cast(referenceIid)
const closer = canonical.cast(closableIid)
canonical.release()

function onClosed(callback) {
  // Mirror generated events: sibling closures share the context containing
  // wrapped, handler, and the returned unsubscribe closure.
  const wrapped = (nativeSender, args) => callback(nativeSender, args)
  const handler = DynWinRtDelegate.create(delegateIid, delegateArgs, wrapped)
  let value
  let token
  try {
    value = handler.toValue()
    token = referenceType.method(7).invoke(sender, [value])
  } finally {
    value?.release()
    handler.release()
  }
  return () => {
    void handler
    referenceType.method(8).invoke(sender, [token])
  }
}

try {
  if (mode === 'add-failure') {
    assert.throws(() => onClosed(() => {}), /expected subscription rejection/)
    assert.equal(retained, undefined)
  } else {
    assert.ok(mode === 'remove' || mode === 'once')
    const unsubscribe = onClosed((nativeSender, args) => {
      assert.equal(referenceType.method(6).invoke(nativeSender, []).toNumber(), 4096)
      assert.equal(args.isNull(), true)
      delivered++
      if (mode === 'once') unsubscribe()
    })
    // Keeping the unsubscribe function reachable must not keep an unsubscribed
    // native delegate alive. No GC or forced process exit is used in this test.
    globalThis.retainedUnsubscribe = unsubscribe
    owner.release()
    closableType.method(6).invokeAll(closer, [])
    assert.equal(delivered, 1)
    if (mode === 'remove') unsubscribe()
    assert.equal(removed, 1)
    assert.equal(retained, undefined)
    closableType.method(6).invokeAll(closer, [])
    assert.equal(delivered, 1)
  }
} finally {
  retained?.release()
  sender.release()
  closer.release()
  owner.dispose()
}
console.log(`generated-event-lifetime-ok:${mode}`)
