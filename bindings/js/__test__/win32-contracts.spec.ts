// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'
import { spawnSync } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { once } from 'node:events'
import { writeFileSync, unlinkSync } from 'node:fs'
import { createRequire } from 'node:module'
import { createServer, type Socket } from 'node:net'
import { join } from 'node:path'
import { Worker } from 'node:worker_threads'
import { fileURLToPath } from 'node:url'
import { DynCom, DynWinRtValue, initializeCom } from '../dist/com-unsafe.js'
import { DynWinRtType, WinGuid } from '../dist/winrt.js'
import {
  DynWin32,
  DynWin32Function,
  DynWin32NativeStruct,
  DynWin32Resource,
  DynWin32Unsafe,
  type DynWin32OverlappedOperation,
} from '../dist/win32-unsafe.js'
import * as safe from '../dist/win32.js'

const require = createRequire(import.meta.url)
const machine = () => DynWin32.handle(0x80000002n)
const openKey = () => {
  const open = DynWin32Function.bind({
    dll: 'advapi32.dll',
    entryPoint: 'RegOpenKeyExW',
    parameters: [
      { type: 'handle', direction: 'in' },
      { type: 'pointer', direction: 'in' },
      { type: 'u32', direction: 'in' },
      { type: 'u32', direction: 'in' },
      { type: 'handle', direction: 'out', cleanup: 'regCloseKey' },
    ],
    returnType: 'i32',
    successRule: 'zero',
  })
  const result = open.invoke([
    machine(),
    DynWin32.wideString('SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion'),
    DynWin32.u32(0),
    DynWin32.u32(1),
  ])
  if (!result.succeeded) throw new Error(`RegOpenKeyExW: ${DynWin32.toNumber(result.returnValue!)}`)
  return DynWin32.toResource(result.outputs[0])!
}
const consumeKey = () =>
  DynWin32Function.bind({
    dll: 'advapi32.dll',
    entryPoint: 'RegCloseKey',
    parameters: [
      {
        type: 'handle',
        direction: 'in',
        consumesResource: true,
        resourceCleanup: 'regCloseKey',
      },
    ],
    returnType: 'i32',
    successRule: 'zero',
  })
const pointDescriptor = () => {
  const layout = {
    size: 8,
    alignment: 4,
    fields: [
      { name: 'x', offset: 0, count: 1, type: { kind: 'i32' } },
      { name: 'y', offset: 4, count: 1, type: { kind: 'i32' } },
    ],
  }
  return JSON.stringify({ name: 'Tests.POINT', kind: 'struct', x86: layout, x64: layout, arm64: layout })
}

test('Win32 facades retain the full isolated runtime without manual safe descriptors', (t) => {
  t.deepEqual(
    Object.keys(safe)
      .filter((key) => !['default', 'module.exports'].includes(key))
      .sort(),
    [
      'DynWin32',
      'DynWin32NativeStruct',
      'DynWin32OverlappedOperation',
      'DynWin32Resource',
      'DynWin32SubsystemContext',
      'DynWin32Value',
      'DynWinRtValue',
    ].sort(),
  )
  for (const facade of ['winrt', 'com', 'com-unsafe', 'com-unsafe-raw']) {
    t.false(Object.keys(require(`../dist/${facade}.js`)).some((name) => /win32/i.test(name)))
  }
  t.false('createNativeStruct' in safe.DynWin32)
  t.false('initializeMapiUtilities' in safe.DynWin32)
  t.is(typeof DynWin32.initializeMapiUtilities, 'function')
  for (const [entryPoint, returnType, expected] of [
    ['GetTickCount', 'u32', 'number'],
    ['GetTickCount64', 'u64', 'bigint'],
  ] as const) {
    const result = DynWin32Function.bind({ dll: 'kernel32.dll', entryPoint, parameters: [], returnType }).invoke([])
    t.is(
      typeof (returnType === 'u64' ? DynWin32.toBigint(result.returnValue!) : DynWin32.toNumber(result.returnValue!)),
      expected,
    )
    t.deepEqual(result.outputs, [])
  }
})

test('Win32 native tags reject forged handles, structs, values and receivers', (t) => {
  const resource = openKey()
  const descriptor = pointDescriptor()
  const point = DynWin32.createNativeStruct(descriptor)
  try {
    t.true(resource instanceof DynWin32Resource)
    for (const forged of [
      0x80000002n,
      { value: resource.value },
      Object.create(DynWin32Resource.prototype),
      Object.setPrototypeOf(DynWin32.i32(1), DynWin32Resource.prototype),
      Object.setPrototypeOf(Buffer.alloc(8), DynWin32Resource.prototype),
      Object.setPrototypeOf(DynWin32.createNativeStruct(descriptor), DynWin32Resource.prototype),
    ]) {
      t.throws(() => DynWin32.resource(forged as never, 'regCloseKey'))
      t.throws(() => DynWin32Resource.prototype.close.call(forged))
    }
    for (const forged of [
      {},
      Object.create(DynWin32NativeStruct.prototype),
      Object.setPrototypeOf(DynWin32.i32(1), DynWin32NativeStruct.prototype),
    ]) {
      t.throws(() => DynWin32.nativeStruct(forged as never, descriptor), { message: /native DynWin32NativeStruct/ })
      const getter = Object.getOwnPropertyDescriptor(DynWin32NativeStruct.prototype, 'bytes')!.get!
      t.throws(() => getter.call(forged))
    }
    t.throws(() => DynWin32.toNumber(point as never), { message: /native DynWin32Value/ })
    t.throws(() => DynWin32Function.prototype.invoke.call(point, []), { message: /native DynWin32Function/ })
    const alias = DynWin32.toResource(DynWin32.handle(resource))!
    t.true(consumeKey().invoke([DynWin32.resource(resource, 'regCloseKey')]).succeeded)
    t.true(resource.closed)
    t.true(alias.closed)
    t.notThrows(() => alias.close())
    t.throws(() => DynWin32.resource(resource, 'regCloseKey'), { message: /closed/ })
  } finally {
    resource.close()
  }
})

test('Win32 buffer lengths come from native backing storage and never truncate success', (t) => {
  let getterCalls = 0
  const backing = Buffer.from([1, 2, 3, 4, 5, 6])
  const value = backing.subarray(1, 5)
  for (const property of ['length', 'byteLength']) {
    Object.defineProperty(value, property, {
      get() {
        getterCalls++
        return 0xffffffff
      },
    })
  }
  t.is(DynWin32.bufferLength(value), 4)
  t.is(DynWin32.byteLength(value), 4)
  t.is(DynWin32.bufferByteLength(value), 4)
  t.is(safe.DynWin32.byteLength(value), 4)
  t.is(safe.DynWin32.bufferByteLength(value), 4)
  t.deepEqual(DynWin32.copyBuffer(value), Buffer.from([2, 3, 4, 5]))
  t.deepEqual(DynWin32.copyBuffer(value, 2), Buffer.from([2, 3]))
  t.throws(() => DynWin32.copyBuffer(value, 5), { message: /exceeds native buffer capacity/ })
  t.truthy(DynWin32.dataPointer(value))
  t.is(getterCalls, 0)
  for (const length of [-1, 0.5, NaN, Infinity, 64 * 1024 * 1024 + 1]) {
    t.throws(() => DynWin32.allocateBuffer(length), { message: /64 MiB/ })
  }
  t.deepEqual(DynWin32.allocateBuffer(4), Buffer.alloc(4))
  for (const alignment of [0, 3, 0.5, 2 ** 32 + 1]) {
    t.throws(() => DynWin32.alignedDataPointer(Buffer.alloc(8), alignment))
  }
  const detached = new Uint8Array(8)
  structuredClone(detached.buffer, { transfer: [detached.buffer] })
  for (const invalid of [detached, new Uint8Array(new SharedArrayBuffer(8)), new Uint16Array(8), { length: 8 }]) {
    t.throws(() => DynWin32.bufferLength(invalid as never))
    t.throws(() => DynWin32.byteLength(invalid as never))
    t.throws(() => DynWin32.bufferByteLength(invalid as never))
    t.throws(() => DynWin32.dataPointer(invalid as never))
    t.throws(() => DynWin32.createNativeStruct(pointDescriptor(), invalid as never))
  }
})

test('Win32 counted registry buffers preserve status, exact sizes and immediate LastError', (t) => {
  const key = openKey()
  const query = DynWin32Function.bind({
    dll: 'advapi32.dll',
    entryPoint: 'RegQueryValueExW',
    parameters: [
      { type: 'handle', direction: 'in', resourceCleanup: 'regCloseKey' },
      { type: 'pointer', direction: 'in' },
      { type: 'pointer', direction: 'in', nullable: true },
      { type: 'u32', direction: 'out' },
      { type: 'pointer', direction: 'in', nullable: true },
      { type: 'u32', direction: 'inout' },
    ],
    returnType: 'i32',
    successRule: 'zero',
  })
  const read = (buffer: Buffer | null) =>
    query.invoke([
      DynWin32.handle(key),
      DynWin32.wideString('ProductName'),
      DynWin32.nullPointer(),
      buffer === null ? DynWin32.nullPointer() : DynWin32.dataPointer(buffer),
      DynWin32.u32(buffer === null ? 0 : DynWin32.bufferLength(buffer)),
    ])
  try {
    const probe = read(null)
    t.true(probe.succeeded)
    const size = DynWin32.toNumber(probe.outputs[1])
    t.true(size > 2)
    const small = read(Buffer.alloc(1))
    t.false(small.succeeded)
    t.is(DynWin32.toNumber(small.returnValue!), 234)
    t.is(DynWin32.toNumber(small.outputs[1]), size)
    const buffer = DynWin32.allocateBuffer(size)
    Object.defineProperty(buffer, 'byteLength', { value: 0xffffffff })
    const result = read(buffer)
    t.true(result.succeeded)
    const actual = DynWin32.toNumber(result.outputs[1])
    t.is(actual, size)
    t.true(DynWin32.copyBuffer(buffer, actual).toString('utf16le').includes('Windows'))
    t.throws(() => DynWin32.copyBuffer(buffer, actual + 1), { message: /exceeds native buffer capacity/ })
  } finally {
    key.close()
  }
  const missingModule = DynWin32Function.bind({
    dll: 'kernel32.dll',
    entryPoint: 'GetModuleHandleW',
    parameters: [{ type: 'pointer', direction: 'in' }],
    returnType: 'handle',
    successRule: 'nonnull',
    captureLastError: true,
  }).invoke([DynWin32.wideString(`dynwinrt-missing-${randomUUID()}.dll`)])
  t.false(missingModule.succeeded)
  t.is(missingModule.lastError, 126)
  t.is(DynWin32.toBigint(missingModule.returnValue!), 0n)
})

test.serial('Win32 performance-data queries preserve success without failure-size inference', (t) => {
  const performanceKey = DynWin32.handle(-2147483644n)
  const normalKey = openKey()
  const closePerformance = DynWin32Function.bind({
    dll: 'advapi32.dll',
    entryPoint: 'RegCloseKey',
    parameters: [{ type: 'handle', direction: 'in' }],
    returnType: 'i32',
    successRule: 'zero',
  })
  try {
    for (const suffix of ['A', 'W']) {
      const query = DynWin32Function.bind({
        dll: 'advapi32.dll',
        entryPoint: `RegQueryValueEx${suffix}`,
        parameters: [
          { type: 'handle', direction: 'in', resourceCleanup: 'regCloseKey' },
          { type: 'pointer', direction: 'in' },
          { type: 'pointer', direction: 'in', nullable: true },
          { type: 'u32', direction: 'out' },
          { type: 'pointer', direction: 'in', nullable: true },
          { type: 'u32', direction: 'inout' },
        ],
        returnType: 'i32',
        successRule: 'zero',
      })
      const read = (performance: boolean, buffer: Buffer | null) =>
        query.invoke([
          performance ? performanceKey : DynWin32.handle(normalKey),
          suffix === 'W'
            ? DynWin32.wideString(performance ? '2' : 'ProductName')
            : DynWin32.ansiString(performance ? '2' : 'ProductName'),
          DynWin32.nullPointer(),
          buffer === null ? DynWin32.nullPointer() : DynWin32.dataPointer(buffer),
          DynWin32.u32(buffer === null ? 0 : DynWin32.byteLength(buffer)),
        ])

      // ERROR_MORE_DATA does not authorize reading the performance-data size.
      for (const buffer of [null, DynWin32.allocateBuffer(1)]) {
        const result = read(true, buffer)
        t.is(DynWin32.toNumber(result.returnValue!), 234)
        t.false(result.succeeded)
      }

      let succeeded = false
      for (let capacity = 64 * 1024; capacity <= 64 * 1024 * 1024; capacity *= 2) {
        const buffer = DynWin32.allocateBuffer(capacity)
        const result = read(true, buffer)
        const status = DynWin32.toNumber(result.returnValue!)
        if (status === 234) continue
        t.is(status, 0)
        t.true(result.succeeded)
        const size = DynWin32.toNumber(result.outputs[1])
        t.true(size >= 8 && size <= DynWin32.byteLength(buffer))
        t.deepEqual(DynWin32.copyBuffer(buffer, 8), Buffer.from('PERF', 'utf16le'))
        succeeded = true
        break
      }
      t.true(succeeded, `${suffix}: performance data must remain queryable by growing capacity`)

      const normalProbe = read(false, null)
      t.true(normalProbe.succeeded)
      const required = DynWin32.toNumber(normalProbe.outputs[1])
      t.true(required > 1)
      const normalSmall = read(false, DynWin32.allocateBuffer(1))
      t.is(DynWin32.toNumber(normalSmall.returnValue!), 234)
      t.is(DynWin32.toNumber(normalSmall.outputs[1]), required)
      const normalSuccess = read(false, DynWin32.allocateBuffer(required))
      t.true(normalSuccess.succeeded)
      t.is(DynWin32.toNumber(normalSuccess.outputs[1]), required)
    }
  } finally {
    normalKey.close()
    t.is(DynWin32.toNumber(closePerformance.invoke([performanceKey]).returnValue!), 0)
  }
})

test('Win32 integer, string and aggregate guards fail before native invocation', (t) => {
  const scalarSpec = { dll: 'kernel32.dll', entryPoint: 'GetTickCount', parameters: [], returnType: 'u32' }
  const oversized: never[] = []
  oversized.length = 0xffffffff
  t.throws(() => DynWin32Function.bind(scalarSpec).invoke(oversized), { message: /1024/ })
  t.throws(() => DynWin32Function.bind({ ...scalarSpec, unknown: true } as never), { message: /[Uu]nknown/ })
  t.throws(() => DynWin32Unsafe.bind({ ...scalarSpec, parameters: oversized }), { message: /1024/ })
  t.throws(
    () =>
      DynWin32Function.bind({
        ...scalarSpec,
        parameters: [{ type: 'u32', direction: 'in', countParamIndex: 9 }],
      } as never),
    { message: /[Uu]nknown/ },
  )
  t.throws(
    () =>
      DynWin32Function.bind({
        ...scalarSpec,
        returnType: 'handle',
        returnCleanup: 'notCloseHandle',
      }),
    { message: /Unsupported flat Win32 cleanup/ },
  )
  for (const [convert, low, high] of [
    [DynWin32.i8, -128, 127],
    [DynWin32.u8, 0, 255],
    [DynWin32.i16, -32768, 32767],
    [DynWin32.u16, 0, 65535],
    [DynWin32.i32, -2147483648, 2147483647],
    [DynWin32.u32, 0, 4294967295],
  ] as const) {
    t.is(DynWin32.toNumber(convert(low)), low)
    t.is(DynWin32.toNumber(convert(high)), high)
    for (const invalid of [low - 1, high + 1, 0.5, NaN, Infinity]) t.throws(() => convert(invalid))
  }
  t.is(DynWin32.toBigint(DynWin32.i64(-(1n << 63n))), -(1n << 63n))
  t.is(DynWin32.toBigint(DynWin32.u64((1n << 64n) - 1n)), (1n << 64n) - 1n)
  t.throws(() => DynWin32.i64(1n << 63n))
  t.throws(() => DynWin32.u64(-1n))
  t.throws(() => DynWin32.u64(1n << 64n))
  t.is(DynWin32.toNumber(DynWin32.f32(Math.PI)), Math.fround(Math.PI))
  t.is(DynWin32.toNumber(DynWin32.f64(Math.PI)), Math.PI)
  t.throws(() => DynWin32.wideString('nul\0suffix'), { message: /NUL/ })
  t.throws(() => DynWin32.wideString('\ud800'), { message: /UTF-16/ })
  t.throws(() => DynWin32.ansiString('é'), { message: /ASCII/ })
  t.throws(() => DynWin32.wideMultiString(['first', '', 'hidden']), { message: /truncate/ })
  const mutableString = Buffer.from('ok\0')
  const retainedString = DynWin32.ansiString(mutableString)
  mutableString[2] = 1
  t.throws(() => DynWin32Unsafe.pointerAddress(retainedString), { message: /NUL-terminated/ })
  const original = JSON.parse(pointDescriptor())
  for (const mutate of [
    (layout: any) => {
      layout.fields[0].offset = 2 ** 40
    },
    (layout: any) => {
      layout.fields[0].count = 2 ** 40
    },
    (layout: any) => {
      layout.fields[0].type.kind = 'unknown'
    },
    (layout: any) => {
      layout.fields[1].offset = 0
    },
  ]) {
    const invalid = structuredClone(original)
    for (const architecture of ['x86', 'x64', 'arm64']) mutate(invalid[architecture])
    t.throws(() => DynWin32.createNativeStruct(JSON.stringify(invalid)))
    t.throws(() =>
      DynWin32Function.bind({
        dll: 'kernel32.dll',
        entryPoint: 'GetTickCount',
        parameters: [
          {
            type: 'pointer',
            direction: 'in',
            aggregateDescriptor: JSON.stringify(invalid),
          },
        ],
        returnType: 'u32',
      }),
    )
  }
  t.throws(
    () =>
      DynWin32Function.bind({
        dll: 'kernel32.dll',
        entryPoint: 'GetTickCount',
        parameters: [],
        returnType: 'f64',
        successRule: 'nonzero',
      }),
    { message: /floating-point/ },
  )
})

test('Win32 by-value aggregate and explicit subsystem plans call real exports', (t) => {
  const descriptor = pointDescriptor()
  const pointBytes = Buffer.alloc(8)
  pointBytes.writeInt32LE(5, 0)
  pointBytes.writeInt32LE(7, 4)
  const point = DynWin32.createNativeStruct(descriptor, pointBytes)
  const rect = Buffer.alloc(16)
  rect.writeInt32LE(10, 8)
  rect.writeInt32LE(10, 12)
  const contains = DynWin32Function.bind({
    dll: 'user32.dll',
    entryPoint: 'PtInRect',
    parameters: [
      { type: 'pointer', direction: 'in' },
      { type: 'pointer', direction: 'in', aggregateDescriptor: descriptor },
    ],
    returnType: 'bool32',
  })
  t.true(
    DynWin32.toBoolean(
      contains.invoke([DynWin32.dataPointer(rect), DynWin32.nativeStructValue(point, descriptor)]).returnValue!,
    ),
  )
  const context = DynWin32.initializeWinsock()
  const getError = DynWin32Function.bind({
    dll: 'ws2_32.dll',
    entryPoint: 'WSAGetLastError',
    parameters: [],
    returnType: 'i32',
  })
  t.is(typeof DynWin32.toNumber(getError.invokeWithSubsystem(context, 'winsock', []).returnValue!), 'number')
  context.close()
  t.throws(() => getError.invokeWithSubsystem(context, 'winsock', []), { message: /closed/ })
})

test('Win32 COM inputs borrow an exact IID and retain the independent reference', (t) => {
  initializeCom(0)
  const object = DynCom.createErrorInfo()
  try {
    t.throws(() => DynWin32.comObject(object, '00000000-0000-0000-0000-000000000000'), {
      message: /required Win32 interface/,
    })
    const borrowed = DynWin32.comObject(object, '22f03340-547d-101b-8e65-08002b2bd119')
    const spoofed = Object.setPrototypeOf(DynWin32.createNativeStruct(pointDescriptor()), DynWinRtValue.prototype)
    t.throws(() => DynWin32.comObject(spoofed, '00000000-0000-0000-c000-000000000046'), {
      message: /not managed COM references/,
    })
    const original = Object.getOwnPropertyDescriptor(DynWinRtValue.prototype, 'isNull')!
    try {
      Object.defineProperty(DynWinRtValue.prototype, 'isNull', { ...original, value: () => false })
      for (const foreign of [DynWinRtType.i32(), WinGuid.parse('00000000-0000-0000-0000-000000000000')]) {
        Object.setPrototypeOf(foreign, DynWinRtValue.prototype)
        t.throws(() => DynWin32.comObject(foreign as never, '00000000-0000-0000-c000-000000000046'), {
          message: /native managed COM carrier/,
        })
      }
    } finally {
      Object.defineProperty(DynWinRtValue.prototype, 'isNull', original)
    }
    object.release()
    t.true(DynWin32Unsafe.pointerAddress(borrowed) > 0n)
  } finally {
    object.release()
  }
})

test('Win32 descriptors reject hidden contract fields and propagate getter failures', (t) => {
  const base = { dll: 'kernel32.dll', entryPoint: 'GetLastError', parameters: [], returnType: 'u32' }
  const hidden = { ...base }
  Object.defineProperty(hidden, 'callingConventon', { value: 'cdecl', enumerable: false })
  t.throws(() => DynWin32Function.bind(hidden), { message: /[Uu]nknown/ })
  const symbol = { ...base, [Symbol('unmodeled contract')]: true }
  t.throws(() => DynWin32Function.bind(symbol), { message: /[Uu]nknown|specification/ })
  for (const change of [
    { parameters: {} },
    { parameters: [null] },
    { returnType: true },
    { callingConvention: 'fastcall' },
    { captureLastError: 'true' },
    { dll: 'x'.repeat(1025) },
    { dll: 'kernel32.dll\0' },
    { parameters: [{ type: 'u32', direction: 'sideways' }] },
    { parameters: [{ type: 'u32', direction: 'out', cleanup: 'closeHandle' }] },
  ]) {
    t.throws(() => DynWin32Function.bind({ ...base, ...change } as never))
  }
  for (const property of ['dll', 'parameters', 'captureLastError']) {
    const failure = new Error(`native descriptor getter: ${property}`)
    const descriptor = { ...base }
    Object.defineProperty(descriptor, property, {
      get() {
        throw failure
      },
      enumerable: true,
    })
    t.throws(() => DynWin32Function.bind(descriptor), { is: failure })
  }
  const parameterFailure = new Error('native parameter getter')
  t.throws(
    () =>
      DynWin32Function.bind({
        ...base,
        parameters: [
          {
            get type(): string {
              throw parameterFailure
            },
            direction: 'in',
          },
        ],
      }),
    { is: parameterFailure },
  )
  const argumentFailure = new Error('native argument getter')
  const args = [DynWin32.u32(0)]
  Object.defineProperty(args, 0, {
    get() {
      throw argumentFailure
    },
  })
  t.throws(() => DynWin32Function.bind(base).invoke(args), { is: argumentFailure })
  const nonEnumerable = Object.create(null)
  for (const [name, value] of Object.entries(base)) Object.defineProperty(nonEnumerable, name, { value })
  t.is(typeof DynWin32.toNumber(DynWin32Function.bind(nonEnumerable).invoke([]).returnValue!), 'number')
})

test('Win32 prepared plans do not follow later descriptor mutations', (t) => {
  const spec = {
    dll: 'kernel32.dll',
    entryPoint: 'MulDiv',
    parameters: Array.from({ length: 3 }, () => ({ type: 'i32', direction: 'in' })),
    returnType: 'i32',
    successRule: 'always',
  }
  const plan = DynWin32Function.bind(spec)
  spec.entryPoint = 'GetLastError'
  spec.parameters[0].type = 'pointer'
  spec.parameters.length = 0
  spec.returnType = 'pointer'
  const result = plan.invoke([DynWin32.i32(8), DynWin32.i32(9), DynWin32.i32(2)])
  t.is(plan.entryPoint, 'MulDiv')
  t.is(DynWin32.toNumber(result.returnValue!), 36)
  t.is(result.returnValue, null)
  t.deepEqual(result.outputs, [])
  t.throws(() => result.outputs, { message: /already consumed/ })
})

test('Win32 string encodings and pointer slots retain validated native storage', (t) => {
  for (const wide of [false, true]) {
    const length = DynWin32Function.bind({
      dll: 'kernel32.dll',
      entryPoint: wide ? 'lstrlenW' : 'lstrlenA',
      parameters: [{ type: 'pointer', direction: 'in' }],
      returnType: 'i32',
    })
    const construct = wide ? DynWin32.wideString : DynWin32.ansiString
    t.is(DynWin32.toNumber(length.invoke([construct('')]).returnValue!), 0)
    t.is(DynWin32.toNumber(length.invoke([construct('hello')]).returnValue!), 5)
    const bytes = Buffer.from('abc\0', wide ? 'utf16le' : 'ascii')
    const pointer = construct(bytes)
    bytes[0] = 90
    t.is(DynWin32.toNumber(length.invoke([pointer]).returnValue!), 3)
    bytes.fill(1, bytes.length - (wide ? 2 : 1))
    t.throws(() => DynWin32Unsafe.pointerAddress(pointer), { message: /NUL-terminated/ })
    const detached = new Uint8Array(wide ? [65, 0, 0, 0] : [65, 0])
    const held = construct(detached)
    structuredClone(detached.buffer, { transfer: [detached.buffer] })
    t.throws(() => DynWin32Unsafe.pointerAddress(held), { message: /detached/ })
    const multi = wide ? DynWin32.wideMultiString : DynWin32.ansiMultiString
    t.truthy(multi([]))
    t.truthy(multi(['one', 'two']))
    t.throws(() => multi(['one\0two']), { message: /NUL/ })
    t.throws(() => multi(['one', 2] as never))
    t.truthy(multi(null, true))
    t.throws(() => multi(null), { message: /nullable/ })
    const encoded = Buffer.from(wide ? [65, 0, 0, 0, 0, 0] : [65, 0, 0])
    const list = multi(encoded)
    encoded[encoded.length - 1] = 1
    t.throws(() => DynWin32Unsafe.pointerAddress(list), { message: /two NUL/ })
  }
  t.truthy(DynWin32.ansiString(Buffer.from([0xe9, 0])))
  const wideLength = DynWin32Function.bind({
    dll: 'kernel32.dll',
    entryPoint: 'lstrlenW',
    parameters: [{ type: 'pointer', direction: 'in' }],
    returnType: 'i32',
  })
  t.is(DynWin32.toNumber(wideLength.invoke([DynWin32.wideString('😀A')]).returnValue!), 3)
  const width = process.arch === 'ia32' ? 4 : 8
  const copy = DynWin32Function.bind({
    dll: 'msvcrt.dll',
    entryPoint: 'memcpy',
    callingConvention: 'cdecl',
    parameters: [
      { type: 'pointer', direction: 'in' },
      { type: 'pointer', direction: 'in' },
      { type: width === 8 ? 'u64' : 'u32', direction: 'in' },
    ],
    returnType: 'pointer',
  })
  const slot = DynWin32.wideStringPointerPointer('owned slot')
  const pointerBytes = DynWin32.allocateBuffer(width)
  copy.invoke([
    DynWin32.dataPointer(pointerBytes),
    slot,
    width === 8 ? DynWin32.u64(BigInt(width)) : DynWin32.u32(width),
  ])
  const pointer = width === 8 ? pointerBytes.readBigUInt64LE() : BigInt(pointerBytes.readUInt32LE())
  t.not(pointer, DynWin32Unsafe.pointerAddress(slot))
  t.is(DynWin32.toNumber(wideLength.invoke([DynWin32Unsafe.pointer(pointer)]).returnValue!), 10)
  t.throws(() => DynWin32.wideStringPointerPointer(null), { message: /nullable/ })
  t.is(DynWin32Unsafe.pointerAddress(DynWin32.wideStringPointerPointer(null, true)), 0n)
})

test('Win32 buffers keep byte-view offsets, alignment and exact native argument counts', (t) => {
  const backing = Buffer.alloc(32)
  const base = DynWin32Unsafe.pointerAddress(DynWin32.dataPointer(backing))
  const offset = Number(-base & 7n)
  const aligned = backing.subarray(offset, offset + 16)
  t.is(DynWin32Unsafe.pointerAddress(DynWin32.alignedDataPointer(aligned, 8)), base + BigInt(offset))
  t.throws(() => DynWin32.alignedDataPointer(backing.subarray(offset + 1, offset + 9), 4), {
    message: /not aligned/,
  })
  t.deepEqual(DynWin32.copyBuffer(aligned, 0), Buffer.alloc(0))
  for (const length of [-1, 0.5, Infinity, NaN, 17]) {
    t.throws(() => DynWin32.copyBuffer(aligned, length))
  }
  const scalar = DynWin32Function.bind({
    dll: 'kernel32.dll',
    entryPoint: 'GetTickCount',
    parameters: [],
    returnType: 'u32',
  })
  t.throws(() => scalar.invoke([DynWin32.u32(0)]), { message: /expects 0/ })
  t.throws(() => scalar.invoke({ length: 0 } as never), { message: /array/ })
})

test('Win32 aliased resources and aggregates cannot deadlock dispatch', (t) => {
  const child = spawnSync(process.execPath, [fileURLToPath(new URL('./fixtures/win32-aliases.cjs', import.meta.url))], {
    timeout: 15000,
    encoding: 'utf8',
    windowsHide: true,
  })
  t.is(child.error, undefined)
  t.is(child.status, 0, child.stderr)
  t.regex(child.stdout, /native-alias-guards-ok/)
})

test('Win32 aggregate field access checks state, identity and copy isolation', (t) => {
  const layout = {
    size: 8,
    alignment: 4,
    fields: [
      { name: 'value', offset: 0, count: 1, type: { kind: 'u32' } },
      { name: 'flag', offset: 4, count: 1, type: { kind: 'i32' } },
    ],
  }
  const descriptor = JSON.stringify({ name: 'Tests.Mutable', kind: 'struct', x86: layout, x64: layout, arm64: layout })
  const value = DynWin32.createNativeStruct(descriptor)
  DynWin32.setNativeStructU32(value, descriptor, 'value', 42)
  DynWin32.setNativeStructBool32(value, descriptor, 'flag', true)
  const copy = value.bytes
  copy.fill(0xff)
  t.is(value.bytes.readUInt32LE(0), 42)
  t.is(value.bytes.readInt32LE(4), 1)
  t.throws(() => DynWin32.getNativeStructU32(value, descriptor, 'value'), { message: /before a successful/ })
  DynWin32.markNativeStructCallResult(value, descriptor, false)
  t.throws(() => DynWin32.getNativeStructU32(value, descriptor, 'value'), { message: /call failed/ })
  DynWin32.markNativeStructCallResult(value, descriptor, true)
  t.is(DynWin32.getNativeStructU32(value, descriptor, 'value'), 42)
  t.throws(() => DynWin32.setNativeStructU32(value, descriptor, 'flag', 1), { message: /not u32/ })
  t.throws(() => DynWin32.setNativeStructU32(value, descriptor, 'missing', 1), { message: /unknown native field/ })
  t.throws(() => DynWin32.getNativeStructU32(value, descriptor.replace('Mutable', 'Other'), 'value'), {
    message: /type mismatch/,
  })
  DynWin32.prepareNativeStructCall(value, descriptor)
  t.throws(() => DynWin32.getNativeStructU32(value, descriptor, 'value'), { message: /call failed/ })
  t.throws(() => DynWin32.toResource(DynWin32.handle(1n)), { message: /not an owned resource/ })
  t.is(DynWin32.toResource(DynWin32.handle(0n)), null)
})

const createEvent = () => {
  const result = DynWin32Function.bind({
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
    captureLastError: true,
  }).invoke([DynWin32.nullPointer(), DynWin32.bool32(true), DynWin32.bool32(false), DynWin32.nullPointer()])
  if (!result.succeeded) throw new Error(`CreateEventW: ${result.lastError}`)
  return DynWin32.toResource(result.returnValue!)!
}

test('Win32 argument getters cannot close a captured resource and still dispatch', (t) => {
  const event = createEvent()
  const wait = DynWin32Function.bind({
    dll: 'kernel32.dll',
    entryPoint: 'WaitForSingleObject',
    parameters: [
      { type: 'handle', direction: 'in', resourceCleanup: 'closeHandle' },
      { type: 'u32', direction: 'in' },
    ],
    returnType: 'u32',
  })
  try {
    const args = [DynWin32.handle(event), DynWin32.u32(0)]
    Object.defineProperty(args, 1, {
      get() {
        event.close()
        return DynWin32.u32(0)
      },
    })
    t.throws(() => wait.invoke(args), { message: /closed/ })
    t.true(event.closed)
  } finally {
    event.close()
  }
})

const start = (operation: DynWin32OverlappedOperation) =>
  new Promise<number>((resolve, reject) => {
    operation.start((error, transferred) => (error ? reject(error) : resolve(transferred!)))
  })
const fileSpec = {
  dll: 'kernel32.dll',
  entryPoint: 'CreateFileW',
  parameters: [
    { type: 'pointer', direction: 'in' },
    { type: 'u32', direction: 'in' },
    { type: 'u32', direction: 'in' },
    { type: 'pointer', direction: 'in', nullable: true },
    { type: 'u32', direction: 'in' },
    { type: 'u32', direction: 'in' },
    { type: 'handle', direction: 'in', nullable: true },
  ],
  returnType: 'handle',
  returnCleanup: 'closeHandle',
  successRule: 'validHandle',
  captureLastError: true,
}
const openFile = (path: string, access = 0xc0000000) => {
  const result = DynWin32Function.bind(fileSpec).invoke([
    DynWin32.wideString(path),
    DynWin32.u32(access),
    DynWin32.u32(7),
    DynWin32.nullPointer(),
    DynWin32.u32(3),
    DynWin32.u32(0x40000000),
    DynWin32.handle(0n, true),
  ])
  if (!result.succeeded) throw new Error(`CreateFileW: ${result.lastError}`)
  return DynWin32.toResource(result.returnValue!)!
}

test.serial('Win32 IOCP AbortSignal cancels active native pipe I/O through CancelIoEx', async (t) => {
  t.timeout(20000)
  const pipe = `\\\\.\\pipe\\dynwinrt-iocp-${randomUUID()}`
  let peer: Socket | undefined
  const server = createServer((socket) => {
    peer = socket
  })
  server.listen(pipe)
  await once(server, 'listening')
  let file: DynWin32Resource | undefined
  let operation: DynWin32OverlappedOperation | undefined
  let completion: Promise<number> | undefined
  try {
    const connected = once(server, 'connection')
    file = openFile(pipe)
    await connected
    const cancelledOutput = Buffer.alloc(16, 0x5a)
    operation = DynWin32.beginReadFile(file, cancelledOutput)
    const abort = new AbortController()
    abort.signal.addEventListener('abort', () => operation!.cancel(), { once: true })
    completion = start(operation)
    await new Promise<void>((resolve) => setImmediate(resolve))
    t.true(file.active)
    t.true(file.busy)
    abort.abort()
    await t.throwsAsync(completion, { message: /Win32 error 995/ })
    t.false(file.active)
    t.false(file.busy)
    t.deepEqual(cancelledOutput, Buffer.alloc(16, 0x5a))
    operation.cancel()
    const recovered = Buffer.alloc(4)
    operation = DynWin32.beginReadFile(file, recovered)
    completion = start(operation)
    peer!.write(Buffer.from('next'))
    t.is(await completion, 4)
    t.is(recovered.toString(), 'next')
    operation.cancel()
    t.false(file.busy)
  } finally {
    operation?.cancel()
    await completion?.catch(() => {})
    file?.close()
    peer?.destroy()
    await new Promise<void>((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())))
  }
})

test.serial('Win32 IOCP retires pending native work when its Node environment closes', async (t) => {
  t.timeout(20000)
  const pipe = `\\\\.\\pipe\\dynwinrt-iocp-teardown-${randomUUID()}`
  let peer: Socket | undefined
  const server = createServer((socket) => {
    peer = socket
  })
  server.listen(pipe)
  await once(server, 'listening')
  const connected = once(server, 'connection')
  const worker = new Worker(
    `
    const { parentPort, workerData } = require('node:worker_threads');
    const { DynWin32, DynWin32Function } = require(workerData.runtime);
    const result = DynWin32Function.bind(workerData.spec).invoke([
      DynWin32.wideString(workerData.pipe), DynWin32.u32(0xc0000000), DynWin32.u32(7),
      DynWin32.nullPointer(), DynWin32.u32(3), DynWin32.u32(0x40000000), DynWin32.handle(0n, true)
    ]);
    if (!result.succeeded) throw new Error('CreateFileW failed: ' + result.lastError);
    const file = DynWin32.toResource(result.returnValue);
    const operation = DynWin32.beginReadFile(file, Buffer.alloc(16));
    operation.start(() => {});
    parentPort.postMessage({ active: file.active });
  `,
    { eval: true, workerData: { runtime: join(process.cwd(), 'dist', 'win32-unsafe.js'), spec: fileSpec, pipe } },
  )
  let timeout: ReturnType<typeof setTimeout> | undefined
  try {
    const [message] = await once(worker, 'message')
    await connected
    t.true(message.active)
    const closed = once(peer!, 'close')
    await worker.terminate()
    await Promise.race([
      closed,
      new Promise<never>((_, reject) => {
        timeout = setTimeout(() => reject(new Error('IOCP teardown did not release the native pipe')), 10000)
      }),
    ])
    t.pass()
  } finally {
    clearTimeout(timeout)
    await worker.terminate()
    peer?.destroy()
    await new Promise<void>((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())))
  }
})

test.serial('Win32 IOCP owns buffers through completion, cancellation, EOF and capacity checks', async (t) => {
  t.timeout(20000)
  const path = join(process.cwd(), '__test__', `win32-iocp-${randomUUID()}.bin`)
  writeFileSync(path, Buffer.from('native-iocp'))
  t.teardown(() => unlinkSync(path))
  const file = openFile(path)
  try {
    const output = Buffer.alloc(11)
    const read = DynWin32.beginReadFile(file, output, 0n)
    t.true(file.busy)
    t.throws(() => file.close(), { message: /asynchronous I\/O/ })
    const close = DynWin32Function.bind({
      dll: 'kernel32.dll',
      entryPoint: 'CloseHandle',
      parameters: [{ type: 'handle', direction: 'in', consumesResource: true, resourceCleanup: 'closeHandle' }],
      returnType: 'bool32',
      successRule: 'nonzero',
    })

    t.throws(() => close.invoke([DynWin32.resource(file, 'closeHandle')]), { message: /asynchronous I\/O/ })
    t.is(await start(read), 11)
    t.is(output.toString(), 'native-iocp')
    t.false(file.busy)
    read.cancel()
    t.throws(() => read.start(() => {}), { message: /already started/ })
    const short = Buffer.alloc(20, 0xa5)
    t.is(await start(DynWin32.beginReadFile(file, short, 7n)), 4)
    t.is(short.subarray(0, 4).toString(), 'iocp')
    t.true(short.subarray(4).every((byte) => byte === 0xa5))
    for (const offset of [-1n, 1n << 64n]) {
      t.throws(() => DynWin32.beginReadFile(file, Buffer.alloc(1), offset), { message: /unsigned 64-bit/ })
      t.throws(() => DynWin32.beginWriteFile(file, Buffer.alloc(1), offset), { message: /unsigned 64-bit/ })
      t.false(file.busy)
    }
    t.is(await start(DynWin32.beginReadFile(file, Buffer.alloc(4), 100n)), 0)
    t.is(await start(DynWin32.beginWriteFile(file, Buffer.from('IOCP'), 0n)), 4)
    const written = Buffer.alloc(4)
    t.is(await start(DynWin32.beginReadFile(file, written, 0n)), 4)
    t.is(written.toString(), 'IOCP')
    const cancelled = DynWin32.beginReadFile(file, Buffer.alloc(1))
    cancelled.cancel()
    await t.throwsAsync(start(cancelled), { message: /aborted/ })
    t.false(file.busy)
    const detachedBacking = new ArrayBuffer(4)
    const detachedBefore = Buffer.from(detachedBacking)
    structuredClone(detachedBacking, { transfer: [detachedBacking] })
    t.throws(() => DynWin32.beginReadFile(file, detachedBefore), { message: /detached/ })
    t.false(file.busy)
    const backing = new ArrayBuffer(16)
    const detached = DynWin32.beginReadFile(file, Buffer.from(backing), 0n)
    structuredClone(backing, { transfer: [backing] })
    await t.throwsAsync(start(detached), { message: /detached|changed/ })
    t.false(file.busy)
    t.throws(() => DynWin32.beginReadFile(file, Buffer.from(new SharedArrayBuffer(4))), {
      message: /SharedArrayBuffer/,
    })
    const pending = Array.from({ length: 1024 }, () => DynWin32.beginReadFile(file, Buffer.alloc(0)))
    try {
      t.throws(() => DynWin32.beginReadFile(file, Buffer.alloc(0)), { message: /operation limit/ })
    } finally {
      for (const operation of pending) {
        operation.cancel()
        t.throws(() => operation.start(() => {}), { message: /aborted/ })
      }
    }
    t.false(file.busy)
    const resumed = Buffer.alloc(1)
    t.is(await start(DynWin32.beginReadFile(file, resumed, 0n)), 1)
    t.is(resumed[0], 'I'.charCodeAt(0))
    file.close()
    t.true(file.closed)
  } finally {
    file.close()
  }
})

test.serial('Win32 IOCP submission errors release reservations and leave resources usable', async (t) => {
  t.timeout(20000)
  const event = createEvent()
  try {
    const invalidFile = DynWin32.beginReadFile(event, Buffer.alloc(1))
    t.true(event.busy)
    await t.throwsAsync(start(invalidFile), { message: /associate.*IOCP/ })
    t.false(event.busy)
    t.false(event.active)
    t.false(event.closed)
    t.throws(() => invalidFile.start(() => {}), { message: /already started/ })
  } finally {
    event.close()
  }
  const path = join(process.cwd(), '__test__', `win32-readonly-${randomUUID()}.bin`)
  writeFileSync(path, Buffer.from('read-only'))
  let file: DynWin32Resource | undefined
  try {
    file = openFile(path, 0x80000000)
    await t.throwsAsync(start(DynWin32.beginWriteFile(file, Buffer.from('bad'))), {
      message: /WriteFile failed|Win32 error/,
    })
    t.false(file.busy)
    t.false(file.active)
    t.false(file.closed)
    const output = Buffer.alloc(9)
    const operation = DynWin32.beginReadFile(file, output)
    t.throws(() => operation.start(null as never))
    t.false(file.busy)
    t.false(file.active)
    t.throws(() => operation.start(() => {}), { message: /already started/ })
    t.is(await start(DynWin32.beginReadFile(file, output)), 9)
    t.is(output.toString(), 'read-only')
    t.false(file.busy)
  } finally {
    file?.close()
    unlinkSync(path)
  }
})
