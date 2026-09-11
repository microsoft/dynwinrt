// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const safe = require('../dist/win32.js')
const { Win32CallPlan, Win32Handle, Win32Resource } = require('../dist/win32-unsafe.js')

const specification = (entryPoint: string, parameters: unknown[], returns = 'status', dll = 'ADVAPI32.dll') => ({
  version: 1,
  dll,
  entryPoint,
  callingConvention: 'system',
  architectures: 7,
  returns,
  parameters,
})
const bind = (spec: unknown) => Win32CallPlan.bind(JSON.stringify(spec))
const borrow = { kind: 'borrow-hkey', rejectPerformanceData: true }
const utf16 = { kind: 'utf16', nullable: true }
const open = () =>
  bind(
    specification('RegOpenKeyExW', [
      borrow,
      utf16,
      { kind: 'u32' },
      { kind: 'u32' },
      { kind: 'own-hkey', borrowedFrom: 0 },
    ]),
  )
const query = () =>
  bind(
    specification('RegQueryValueExW', [
      borrow,
      utf16,
      { kind: 'reserved-null' },
      { kind: 'out-u32' },
      { kind: 'bytes', countParameter: 5 },
      { kind: 'byte-count', bufferParameter: 4 },
    ]),
  )
const close = () => bind(specification('RegCloseKey', [{ kind: 'consume-hkey' }]))
const machine = () => Win32Handle.hkey(0x80000002n)
const openKey = () => {
  const result = open().invoke([machine(), 'SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion', 0, 1])
  if (result.value !== 0) throw new Error(`RegOpenKeyExW failed: ${result.value}`)
  return result.outputs[0]
}

test('Win32 facades preserve WinRT and COM isolation', (t) => {
  t.deepEqual(Object.keys(safe).sort(), ['Win32Handle', 'Win32Resource'])
  for (const facade of ['winrt', 'com', 'com-unsafe', 'com-unsafe-raw']) {
    t.false(Object.keys(require(`../dist/${facade}.js`)).some((name) => /win32/i.test(name)))
  }
  t.throws(() => new Win32Resource())
  t.throws(() => new Win32CallPlan())
  t.throws(() => Win32Handle.hkey(Buffer.alloc(8)))
  t.throws(() => Win32Handle.hkey(1n << 100n))
  for (const [entry, returns, type] of [
    ['GetTickCount', 'u32', 'number'],
    ['GetTickCount64', 'u64', 'bigint'],
  ]) {
    const result = bind(specification(entry, [], returns, 'KERNEL32.dll')).invoke([])
    t.is(typeof result.value, type)
    t.deepEqual(result.outputs, [])
  }
})

test('Win32 plans reject unknown fields, unsafe shapes and mismatched values', (t) => {
  for (const spec of [
    { ...specification('GetTickCount', [], 'u32', 'KERNEL32.dll'), unknown: true },
    specification('GetTickCount', [], 'u32', '..\\KERNEL32.dll'),
    specification('CloseHandle', [{ kind: 'consume-hkey' }], 'status', 'KERNEL32.dll'),
    specification('RegCloseKey', [borrow]),
    specification('RegQueryValueExW', [{ kind: 'bytes', countParameter: 99 }]),
    specification('RegOpenKeyExW', [{ kind: 'u32', js: 'injected' }]),
  ])
    t.throws(() => bind(spec))
  t.throws(() => open().invoke([Buffer.alloc(8), null, 0, 1]))
  t.throws(() => open().invoke([machine(), undefined, 0, 1]))
  t.throws(() => open().invoke([machine(), 'nul\0suffix', 0, 1]))
  t.throws(() => open().invoke([machine(), '\ud800', 0, 1]), { message: /UTF-16/ })
  for (const value of [-1, 0.5, 2 ** 32, NaN, Infinity, '1']) {
    t.throws(() => open().invoke([machine(), null, value, 1]))
  }
  t.throws(() => query().invoke([Win32Handle.hkey(0x80000004n), null, null]), {
    message: /HKEY_PERFORMANCE_DATA/,
  })
})

test('Win32 resource ownership cannot be forged or consumed twice', (t) => {
  const resource = openKey()
  t.true(resource instanceof Win32Resource)
  t.false(resource.closed)
  t.false('value' in resource)
  for (const forged of [
    0x80000002n,
    { value: 0x80000002n },
    machine(),
    Object.create(Win32Resource.prototype),
    Object.setPrototypeOf(machine(), Win32Resource.prototype),
    Object.setPrototypeOf(Buffer.alloc(8), Win32Resource.prototype),
  ]) {
    t.throws(() => close().invoke([forged]))
    t.throws(() => Win32Resource.prototype.close.call(forged))
  }
  t.is(close().invoke([resource]).value, 0)
  t.true(resource.closed)
  t.is(close().invoke([resource]).value, 0)
  t.is(resource.close(), 0)
  t.throws(() => query().invoke([resource, 'ProductName', null]), { message: /closed/ })
  const refreshed = open().invoke([machine(), null, 0, 1])
  t.is(refreshed.value, 0)
  t.true(refreshed.outputs[0] instanceof Win32Handle)
  t.throws(() => close().invoke([refreshed.outputs[0]]))
})

test('Win32 counted outputs use native backing lengths and preserve required size', (t) => {
  const resource = openKey()
  const plan = query()
  try {
    const probe = plan.invoke([resource, 'ProductName', null])
    t.is(probe.value, 0)
    t.is(probe.outputs[1], null)
    const size = probe.outputs[2]
    t.true(size > 2)
    const small = Buffer.alloc(1)
    Object.defineProperty(small, 'length', { value: 0xffffffff })
    Object.defineProperty(small, 'byteLength', { value: 0xffffffff })
    const more = plan.invoke([resource, 'ProductName', small])
    t.is(more.value, 234)
    t.is(more.outputs[0], null)
    t.is(more.outputs[1], null)
    t.is(more.outputs[2], size)
    const backing = Buffer.alloc(size + 8, 0xab)
    const view = backing.subarray(4, 4 + size)
    Object.defineProperty(view, 'length', { value: 1 })
    Object.defineProperty(view, 'byteLength', { value: 1 })
    const read = plan.invoke([resource, 'ProductName', view])
    t.is(read.value, 0)
    t.is(read.outputs[1].byteLength, size)
    t.true(read.outputs[1].toString('utf16le').includes('Windows'))
    t.true(backing.every((byte: number) => byte === 0xab))
    const missing = plan.invoke([resource, 'dynwinrt-missing-contract-value', Buffer.alloc(8)])
    t.not(missing.value, 0)
    t.deepEqual(missing.outputs, [null, null, null])
    const shared = new Uint8Array(new SharedArrayBuffer(size))
    t.throws(() => plan.invoke([resource, 'ProductName', shared]), { message: /SharedArrayBuffer/ })
    const detached = new Uint8Array(size)
    structuredClone(detached.buffer, { transfer: [detached.buffer] })
    t.throws(() => plan.invoke([resource, 'ProductName', detached]), { message: /detached/ })
    t.throws(() => plan.invoke([resource, 'ProductName', { length: size, byteLength: size }]))
    t.throws(() => plan.invoke([resource, 'ProductName', new Uint16Array(size)]))
    t.throws(() => plan.invoke([resource, 'ProductName', undefined]))
  } finally {
    t.is(resource.close(), 0)
  }
})
