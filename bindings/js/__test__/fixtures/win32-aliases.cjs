// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

'use strict'

const assert = require('node:assert/strict')
const { DynWin32: W, DynWin32Function: F } = require('..\\..\\dist\\win32-unsafe.js')
const layout = {
  size: 16,
  alignment: 4,
  fields: ['left', 'top', 'right', 'bottom'].map((name, index) => ({
    name,
    offset: index * 4,
    count: 1,
    type: { kind: 'i32' },
  })),
}
const descriptor = JSON.stringify({ name: 'Tests.RECT', kind: 'struct', x86: layout, x64: layout, arm64: layout })
const rectangle = W.createNativeStruct(descriptor, Buffer.alloc(16))
const equal = F.bind({
  dll: 'user32.dll',
  entryPoint: 'EqualRect',
  parameters: [
    { type: 'pointer', direction: 'in' },
    { type: 'pointer', direction: 'in' },
  ],
  returnType: 'bool32',
})
const pointer = W.nativeStruct(rectangle, descriptor)
assert.throws(() => equal.invoke([pointer, pointer]), /same native aggregate/)
const second = W.createNativeStruct(descriptor, Buffer.alloc(16))
assert.equal(W.toBoolean(equal.invoke([pointer, W.nativeStruct(second, descriptor)]).returnValue), true)

const event = F.bind({
  dll: 'kernel32.dll',
  entryPoint: 'CreateEventW',
  parameters: [
    { type: 'pointer', direction: 'in', nullable: true },
    { type: 'bool32', direction: 'in' },
    { type: 'bool32', direction: 'in' },
    { type: 'pointer', direction: 'in', nullable: true },
  ],
  returnType: 'handle',
  returnCleanup: 'closeHandle',
  successRule: 'nonnull',
}).invoke([W.nullPointer(), W.bool32(true), W.bool32(false), W.nullPointer()])
assert.equal(event.succeeded, true)
const resource = W.toResource(event.returnValue)
try {
  const compare = F.bind({
    dll: 'kernelbase.dll',
    entryPoint: 'CompareObjectHandles',
    parameters: [
      { type: 'handle', direction: 'in', resourceCleanup: 'closeHandle' },
      { type: 'handle', direction: 'in', resourceCleanup: 'closeHandle' },
    ],
    returnType: 'bool32',
  })
  const alias = W.toResource(W.handle(resource))
  assert.throws(() => compare.invoke([W.handle(resource), W.handle(alias)]), /same managed Win32 resource/)
  resource.close()
  assert.equal(alias.closed, true)
} finally {
  resource.close()
}
console.log('native-alias-guards-ok')
