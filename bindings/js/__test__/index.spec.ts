// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'
import { spawn, spawnSync } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import {
  DynWinRtArray,
  DynWinRtMethodSig,
  DynWinRtStruct,
  DynWinRtType,
  DynWinRtValue,
  WinGuid,
  getComputerName,
  getWindowsDirectory,
  hasPackageIdentity,
  roInitialize,
} from '../dist/winrt.js'
import * as winrtRuntime from '../dist/winrt.js'
import * as comRuntime from '../dist/com.js'
import {
  DynCom,
  DynComDispatchParams,
  DynComMethodSig,
  DynComPropVariant,
  DynComSafeArray,
  DynComUnsafe,
  DynComVariant,
} from '../dist/com-unsafe.js'
import { DynWin32 } from '../dist/win32.js'
import * as win32Runtime from '../dist/win32.js'
import {
  DynWin32Function,
  DynWin32Unsafe,
} from '../dist/win32-unsafe.js'

const requireFromTest = createRequire(import.meta.url)
const nativeRuntime = requireFromTest('../dist/index.js') as Record<string, unknown>
const winrtCjsRuntime = requireFromTest('../dist/winrt.js') as Record<string, unknown>
const comCjsRuntime = requireFromTest('../dist/com.js') as Record<string, unknown>
const unsafeComRuntime = requireFromTest('../dist/com-unsafe.js') as Record<string, unknown>
const win32CjsRuntime = requireFromTest('../dist/win32.js') as Record<string, unknown>
const unsafeWin32Runtime = requireFromTest('../dist/win32-unsafe.js') as Record<string, unknown>

const moduleKeys = (value: object) =>
  Object.keys(value)
    .filter((key) => key !== 'default' && key !== 'module.exports')
    .sort()

test('Classic COM is isolated from the WinRT root entrypoint', (t) => {
  t.false(Object.prototype.hasOwnProperty.call(winrtRuntime, 'DynCom'))
  t.false(Object.prototype.hasOwnProperty.call(comRuntime, 'DynCom'))
  t.false(Object.prototype.hasOwnProperty.call(comRuntime, 'DynComUnsafe'))
  t.false(Object.prototype.hasOwnProperty.call(comRuntime, 'DynComMethodSig'))
  t.false(Object.prototype.hasOwnProperty.call(comRuntime, 'DynComInterface'))
  t.false(Object.prototype.hasOwnProperty.call(comRuntime, 'DynComType'))
  t.false(Object.prototype.hasOwnProperty.call(comRuntime, 'DynWinRTValue'))
  t.false(Object.prototype.hasOwnProperty.call(comRuntime, 'WinGUID'))
  t.is(typeof comRuntime.initializeCom, 'function')
  t.truthy(DynCom)
  t.truthy(DynComUnsafe)

  const assertion =
    "const assert = require('node:assert/strict');" +
    "const winrt = require('@microsoft/dynwinrt');" +
    "const com = require('@microsoft/dynwinrt/com');" +
    "const unsafeCom = require('@microsoft/dynwinrt/com/unsafe');" +
    "assert.equal(Object.prototype.hasOwnProperty.call(winrt, 'DynCom'), false);" +
    "assert.equal(Object.prototype.hasOwnProperty.call(com, 'DynComUnsafe'), false);" +
    "assert.equal(Object.prototype.hasOwnProperty.call(com, 'DynComMethodSig'), false);" +
    "assert.equal(Object.prototype.hasOwnProperty.call(com, 'DynComInterface'), false);" +
    "assert.equal(Object.prototype.hasOwnProperty.call(com, 'DynComType'), false);" +
    "assert.equal(Object.prototype.hasOwnProperty.call(com, 'DynWinRTValue'), false);" +
    "assert.equal(Object.prototype.hasOwnProperty.call(com, 'WinGUID'), false);" +
    "assert.equal(typeof winrt.DynWinRtType, 'function');" +
    "assert.equal(typeof com.DynComVariant, 'function');" +
    "assert.equal(typeof com.initializeCom, 'function');" +
    "assert.equal(typeof unsafeCom.DynComUnsafe, 'function');" +
    "console.log('runtime-entrypoints-ok')"
  const cjs = spawnSync(process.execPath, ['--eval', assertion], {
    cwd: resolve(process.cwd()),
    encoding: 'utf8',
    windowsHide: true,
  })
  t.is(cjs.status, 0, cjs.stderr)
  t.regex(cjs.stdout, /runtime-entrypoints-ok/)

  const esmAssertion =
    "import assert from 'node:assert/strict';" +
    "import * as winrt from '@microsoft/dynwinrt';" +
    "import * as com from '@microsoft/dynwinrt/com';" +
    "import * as unsafeCom from '@microsoft/dynwinrt/com/unsafe';" +
    "assert.equal(Object.prototype.hasOwnProperty.call(winrt, 'DynCom'), false);" +
    "assert.equal(Object.prototype.hasOwnProperty.call(com, 'DynComUnsafe'), false);" +
    "assert.equal(Object.prototype.hasOwnProperty.call(com, 'DynComMethodSig'), false);" +
    "assert.equal(Object.prototype.hasOwnProperty.call(com, 'DynComInterface'), false);" +
    "assert.equal(Object.prototype.hasOwnProperty.call(com, 'DynComType'), false);" +
    "assert.equal(Object.prototype.hasOwnProperty.call(com, 'DynWinRTValue'), false);" +
    "assert.equal(Object.prototype.hasOwnProperty.call(com, 'WinGUID'), false);" +
    "assert.equal(typeof winrt.DynWinRtType, 'function');" +
    "assert.equal(typeof com.DynComVariant, 'function');" +
    "assert.equal(typeof com.initializeCom, 'function');" +
    "assert.equal(typeof unsafeCom.DynComUnsafe, 'function');" +
    "console.log('runtime-entrypoints-ok')"
  const esm = spawnSync(process.execPath, ['--input-type=module', '--eval', esmAssertion], {
    cwd: resolve(process.cwd()),
    encoding: 'utf8',
    windowsHide: true,
  })

  t.is(esm.status, 0, esm.stderr)
  t.regex(esm.stdout, /runtime-entrypoints-ok/)
})

test('package facades exactly partition native exports', (t) => {
  const nativeKeys = moduleKeys(nativeRuntime)
  const expectedWinrt = nativeKeys.filter(
    (name) => !name.startsWith('DynCom') && !name.startsWith('DynWin32') && name !== 'initializeCom',
  )
  const safeComNames = new Set([
    'DynComDispatchParams',
    'DynComAllocation',
    'DynComExcepInfo',
    'DynComNativeStruct',
    'DynComNativeStructArray',
    'DynComNativeUnion',
    'DynComPropVariant',
    'DynComSafeArray',
    'DynComStatStg',
    'DynComVariant',
    'DynWinRtValue',
    'WinGuid',
    'initializeCom',
  ])
  const unsafeComNames = new Set([
    ...safeComNames,
    'DynCom',
    'DynComDispatchInvokeResult',
    'DynComInterface',
    'DynComMethodHandle',
    'DynComMethodSig',
    'DynComType',
    'DynComUnsafe',
    'DynComUnsafeInterface',
  ])
  const safeWin32Names = new Set([
    'DynWin32',
    'DynWin32NativeStruct',
    'DynWin32OverlappedOperation',
    'DynWin32Resource',
    'DynWin32Value',
    'DynWinRtValue',
  ])
  const unsafeWin32Names = new Set([
    ...safeWin32Names,
    'DynWin32CallResult',
    'DynWin32Function',
    'DynWin32Unsafe',
  ])

  t.deepEqual(moduleKeys(winrtCjsRuntime), expectedWinrt)
  t.deepEqual(
    moduleKeys(comCjsRuntime),
    nativeKeys.filter((name) => safeComNames.has(name)),
  )
  t.deepEqual(
    moduleKeys(unsafeComRuntime),
    nativeKeys.filter((name) => unsafeComNames.has(name)),
  )
  t.deepEqual(
    moduleKeys(win32CjsRuntime),
    nativeKeys.filter((name) => safeWin32Names.has(name)),
  )
  t.deepEqual(
    moduleKeys(unsafeWin32Runtime),
    nativeKeys.filter((name) => unsafeWin32Names.has(name)),
  )

  t.is(winrtCjsRuntime.WinGuid, comCjsRuntime.WinGuid)
  t.is(comCjsRuntime.initializeCom, unsafeComRuntime.initializeCom)
  t.false(Object.prototype.hasOwnProperty.call(comCjsRuntime, 'DynWinRTValue'))
  t.false(Object.prototype.hasOwnProperty.call(comCjsRuntime, 'WinGUID'))
  t.false(Object.prototype.hasOwnProperty.call(unsafeComRuntime, 'DynWinRTValue'))
  t.false(Object.prototype.hasOwnProperty.call(unsafeComRuntime, 'WinGUID'))
  for (const name of moduleKeys(comCjsRuntime)) {
    t.true(moduleKeys(unsafeComRuntime).includes(name), `${name} must remain available from /com/unsafe`)
  }
  for (const name of moduleKeys(win32CjsRuntime)) {
    t.true(moduleKeys(unsafeWin32Runtime).includes(name), `${name} must remain available from /win32/unsafe`)
  }
})

test('COM allocation declaration is opaque and non-constructible', (t) => {
  const declaration = readFileSync(
    fileURLToPath(new URL('../dist/com.d.ts', import.meta.url)),
    'utf8',
  )
  t.regex(declaration, /export interface DynComAllocation/)
  t.notRegex(declaration, /export \{[^}]*DynComAllocation[^}]*\} from/)
})

test('WinRT root facade exposes usable native primitives', (t) => {
  t.truthy(winrtRuntime.DynWinRtType.i32())
  t.is(winrtRuntime.DynWinRtValue.i32(1).toNumber(), 1)
  t.is(winrtRuntime.DynWinRtValue.hstring('root-smoke').toString(), 'root-smoke')
  t.regex(
    winrtRuntime.WinGuid.parse('00000000-0000-0000-c000-000000000046').toString(),
    /00000000-0000-0000-c000-000000000046/i,
  )
  t.is(typeof winrtRuntime.hasPackageIdentity(), 'boolean')
  t.is(typeof winrtRuntime.roInitialize, 'function')
})

test('Classic COM raw ABI access requires the explicit unsafe entrypoint', (t) => {
  const iid = WinGuid.parse('00000000-0000-0000-c000-000000000046')
  t.is(typeof DynCom.projectWinRtAsync, 'function')
  const raw = DynComUnsafe.registerIUnknownInterface('Unsafe.IUnknown', iid).addMethodAt(
    3,
    'Raw',
    new DynComMethodSig().addOut(DynComUnsafe.unclassifiedPointerType()),
  )

  t.truthy(raw.method(3))
  t.is(typeof (raw as unknown as { addMethod?: unknown }).addMethod, 'undefined')
  t.truthy(DynComUnsafe.ownedComOutputType())
  t.truthy(DynComUnsafe.coTaskMemOutputType())
  t.truthy(DynComUnsafe.bstrOutputType())
  t.truthy(DynComUnsafe.borrowedHandleOutputType())
  t.true(DynComUnsafe.adoptOwnedComPointer(0n, iid).isNull())
  t.true(DynComUnsafe.borrowComPointer(0n, iid).isNull())
  t.throws(
    () =>
      (DynComUnsafe.adoptOwnedComPointer as unknown as (value: Buffer, iid: WinGuid) => unknown)(Buffer.alloc(8), iid),
    { message: /numeric pointer bits/ },
  )
})

test('DynCom rejects invalid WinRT async result signatures', (t) => {
  const invalidAsyncType = DynWinRtType.iAsyncOperation(
    DynWinRtType.arrayType(DynWinRtType.i32()),
  )

  const error = t.throws(() =>
    DynCom.projectWinRtAsync(DynWinRtValue.nullValue(), invalidAsyncType),
  )
  t.regex(error.message, /valid WinRT signature/)
})

test('DynCom verifies the projected WinRT async interface IID', (t) => {
  roInitialize(1)
  const factory = DynWinRtValue.activationFactory('Windows.Foundation.Uri')
  try {
    const error = t.throws(() =>
      DynCom.projectWinRtAsync(
        factory,
        DynWinRtType.iAsyncOperation(DynWinRtType.i32()),
      ),
    )
    t.regex(error.message, /0x80004002|interface/i)
  } finally {
    factory.release()
  }
})

test('flat Win32 is isolated and raw addresses require the unsafe entrypoint', (t) => {
  t.false(Object.prototype.hasOwnProperty.call(winrtRuntime, 'DynWin32'))
  t.false(Object.prototype.hasOwnProperty.call(comRuntime, 'DynWin32'))
  t.false(Object.prototype.hasOwnProperty.call(win32Runtime, 'DynWin32Unsafe'))
  t.false(Object.prototype.hasOwnProperty.call(win32Runtime, 'DynWin32Function'))
  t.is(typeof DynWin32.dataPointer, 'function')
  t.is(typeof win32Runtime.DynWin32Value, 'function')
  t.truthy(DynWin32Unsafe)

  t.throws(() => (DynWin32.dataPointer as unknown as (value: bigint) => unknown)(0x1234n), {
    message: /arbitrary numeric addresses/,
  })
  const pointer = DynWin32.dataPointer(Buffer.alloc(4))
  t.truthy(pointer)
  t.throws(() => DynWin32.toBigint(pointer), { message: /not a 64-bit integer/ })
  t.truthy(DynWin32Unsafe.pointerAddress(pointer))
  t.truthy(DynWin32Unsafe.pointer(0x1234n))
  t.truthy(DynWin32.handle(null, true))
  t.throws(() => DynWin32.handle(null), { message: /explicitly nullable/ })
  t.throws(() => DynWin32.dataPointer(Buffer.alloc(0)), {
    message: /non-empty backing storage/,
  })
  t.truthy(DynWin32.dataPointer(Buffer.alloc(0), true))
})

test('flat Win32 immutable plans validate ABI before native dispatch', (t) => {
  const mulDiv = DynWin32Function.bind({
    dll: 'kernel32.dll',
    entryPoint: 'MulDiv',
    parameters: [
      { type: 'i32', direction: 'in' },
      { type: 'i32', direction: 'in' },
      { type: 'i32', direction: 'in' },
    ],
    returnType: 'i32',
    successRule: 'always',
    captureLastError: false,
  })

  const call = mulDiv.invoke([DynWin32.i32(100), DynWin32.i32(3), DynWin32.i32(2)])
  t.is(DynWin32.toNumber(call.returnValue!), 150)
  t.deepEqual(call.outputs, [])
  t.throws(() => mulDiv.invoke([DynWin32.u32(100), DynWin32.i32(3), DynWin32.i32(2)]), {
    message: /does not match I32/,
  })

  const consuming = DynWin32Function.bind({
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
  t.throws(() => consuming.invoke([DynWin32.handle(0x80000002n)]), {
    message: /managed resource object/,
  })
})

test('flat Win32 native aggregate storage is aligned, branded, and mutable', (t) => {
  const descriptor = JSON.stringify({
    name: 'Tests.POINT',
    kind: 'struct',
    x86: { size: 8, alignment: 4, fields: [] },
    x64: { size: 8, alignment: 4, fields: [] },
    arm64: { size: 8, alignment: 4, fields: [] },
  })
  const point = DynWin32.createNativeStruct(
    descriptor,
    Buffer.from([1, 0, 0, 0, 2, 0, 0, 0]),
  )
  t.is(point.length, 8)
  t.deepEqual([...point.bytes], [1, 0, 0, 0, 2, 0, 0, 0])
  t.truthy(DynWin32.nativeStruct(point, descriptor))
  t.throws(
    () => DynWin32.nativeStruct(point, descriptor.replace('POINT', 'SIZE')),
    { message: /type mismatch/ },
  )

  const hugeDescriptor = JSON.stringify({
    name: 'Tests.HUGE',
    kind: 'struct',
    x86: { size: 16 * 1024 * 1024 + 8, alignment: 8, fields: [] },
    x64: { size: 16 * 1024 * 1024 + 8, alignment: 8, fields: [] },
    arm64: { size: 16 * 1024 * 1024 + 8, alignment: 8, fields: [] },
  })
  t.throws(() => DynWin32.createNativeStruct(hugeDescriptor), {
    message: /safety limit/,
  })
  t.throws(() => DynWin32.createNativeStruct(`${descriptor}${' '.repeat(1024 * 1024)}`), {
    message: /descriptor exceeds/,
  })
})

test('flat Win32 multi-strings require double-NUL storage', (t) => {
  t.truthy(DynWin32.wideMultiString(['en-US', 'fr-FR']))
  t.truthy(DynWin32.ansiMultiString(['alpha', 'beta']))
  t.throws(() => DynWin32.wideMultiString(Buffer.from([65, 0, 0, 0])), {
    message: /two NUL code units/,
  })
  t.throws(() => DynWin32.ansiMultiString(Buffer.from([65, 0])), {
    message: /two NUL bytes/,
  })
  t.truthy(DynWin32.wideMultiString(Buffer.from([65, 0, 0, 0, 0, 0])))
  t.truthy(DynWin32.ansiMultiString(Buffer.from([65, 0, 0])))
})

test('flat Win32 pointer-bearing aggregates retain safe field owners', (t) => {
  const descriptor = JSON.stringify({
    name: 'Windows.Win32.Security.SECURITY_ATTRIBUTES',
    kind: 'struct',
    x86: {
      size: 12,
      alignment: 4,
      fields: [
        { name: 'nLength', offset: 0, count: 1, type: { kind: 'u32' } },
        { name: 'lpSecurityDescriptor', offset: 4, count: 1, type: { kind: 'pointer' } },
        { name: 'bInheritHandle', offset: 8, count: 1, type: { kind: 'i32' } },
      ],
    },
    x64: {
      size: 24,
      alignment: 8,
      fields: [
        { name: 'nLength', offset: 0, count: 1, type: { kind: 'u32' } },
        { name: 'lpSecurityDescriptor', offset: 8, count: 1, type: { kind: 'pointer' } },
        { name: 'bInheritHandle', offset: 16, count: 1, type: { kind: 'i32' } },
      ],
    },
    arm64: {
      size: 24,
      alignment: 8,
      fields: [
        { name: 'nLength', offset: 0, count: 1, type: { kind: 'u32' } },
        { name: 'lpSecurityDescriptor', offset: 8, count: 1, type: { kind: 'pointer' } },
        { name: 'bInheritHandle', offset: 16, count: 1, type: { kind: 'i32' } },
      ],
    },
  })
  t.throws(() => DynWin32.createNativeStruct(descriptor, Buffer.alloc(24)), {
    message: /cannot be initialized from raw bytes/,
  })

  const attributes = DynWin32.createNativeStruct(descriptor)
  DynWin32.setNativeStructU32(attributes, descriptor, 'nLength', attributes.length)
  DynWin32.setNativeStructBool32(attributes, descriptor, 'bInheritHandle', true)
  const descriptorBytes = new Uint8Array(20)
  DynWin32.setNativeStructPointer(
    attributes,
    descriptor,
    'lpSecurityDescriptor',
    DynWin32.dataPointer(descriptorBytes),
  )
  t.throws(() => attributes.bytes, { message: /unavailable/ })
  t.throws(
    () =>
      DynWin32.setNativeStructPointer(
        attributes,
        descriptor,
        'lpSecurityDescriptor',
        DynWin32Unsafe.pointer(0x1234n),
      ),
    { message: /retained Buffer or string storage/ },
  )

  const aggregate = DynWin32.nativeStruct(attributes, descriptor)
  structuredClone(descriptorBytes.buffer, { transfer: [descriptorBytes.buffer] })
  const noArgs = DynWin32Function.bind({
      dll: 'kernel32.dll',
      entryPoint: 'GetLastError',
      parameters: [],
      returnType: 'u32',
  })
  t.throws(() => noArgs.invoke([aggregate]), { message: /detached/ })
})

test('DynCom rejects pointers after their TypedArray backing store is detached', (t) => {
  const bytes = new Uint8Array(16)
  const pointer = DynCom.pointer(bytes)

  structuredClone(bytes.buffer, { transfer: [bytes.buffer] })

  t.is(bytes.byteLength, 0)
  const error = t.throws(() => DynCom.asPointerBigint(pointer))
  t.regex(error.message, /backing ArrayBuffer is detached/)
})

test('Generated COM pointer helpers reject arbitrary numeric addresses', (t) => {
  t.truthy(DynCom.safeDataPointer(Buffer.alloc(8)))
  t.truthy(DynCom.safeDataPointer(new Uint8Array(8)))
  t.throws(
    () =>
      (DynCom.safeDataPointer as unknown as (value: bigint) => unknown)(0x1234n),
    { message: /arbitrary numeric addresses/ },
  )

  t.truthy(DynCom.safeWideStringPointer('wide'))
  t.truthy(DynCom.safeAnsiStringPointer('ansi'))
  t.throws(
    () =>
      (DynCom.safeWideStringPointer as unknown as (value: bigint) => unknown)(0x1234n),
    { message: /arbitrary numeric addresses/ },
  )
  t.throws(
    () =>
      (DynCom.safeAnsiStringPointer as unknown as (value: number) => unknown)(0x1234),
    { message: /arbitrary numeric addresses/ },
  )
})

test('DynCom rejects detached counted buffers and accepts typed backing widths', (t) => {
  const detached = new Uint8Array(16)
  structuredClone(detached.buffer, { transfer: [detached.buffer] })
  const error = t.throws(() => DynCom.buffer(detached))
  t.regex(error.message, /backing ArrayBuffer is detached/)

  for (const value of [
    Buffer.alloc(8),
    new Uint8Array(8),
    new Uint16Array(4),
    new Uint32Array(2),
    new Float32Array(2),
    new Float64Array(1),
    new BigInt64Array(1),
  ]) {
    t.truthy(DynCom.buffer(value))
  }
})

test('DynCom rejects SharedArrayBuffer-backed native storage', (t) => {
  const shared = new Uint8Array(new SharedArrayBuffer(16))
  const bufferError = t.throws(() => DynCom.buffer(shared))
  t.regex(bufferError.message, /SharedArrayBuffer/)
  const pointerError = t.throws(() => DynCom.pointer(shared))
  t.regex(pointerError.message, /SharedArrayBuffer/)
})

test('DynCom does not adopt borrowed raw pointer bits as owned COM references', (t) => {
  const borrowed = DynCom.pointer(0n)
  const error = t.throws(() => DynCom.adoptComPointer(borrowed))
  t.regex(error.message, /Borrowed provenance; expected ComOutput/)
})

test('DynCom exposes HSTRING and semantic HRESULT primitives', (t) => {
  t.is(DynCom.hstring('dynwinrt').toString(), 'dynwinrt')
  t.truthy(DynCom.hstringType())
  t.truthy(new DynComMethodSig().preserveHresult())
})

test('DynCom BSTR values preserve exact strings and use dedicated signatures', (t) => {
  const values = ['', 'embedded\u0000nul', 'supplementary \u{1f642}', 'x'.repeat(65537)]
  for (const value of values) {
    const original = value
    const bstr = DynCom.bstr(value)
    t.false(bstr.isNull())
    t.is(bstr.toString(), value)
    t.is(value, original)
  }
  t.is(DynCom.nullBstr().toString(), '')
  t.truthy(DynCom.bstrType())
  t.truthy(DynCom.nullableBstrType())
  t.truthy(new DynComMethodSig().addIn(DynCom.bstrType()).addInOut(DynCom.bstrType()))
  t.throws(() => DynCom.bstr(1 as unknown as string), { message: /string/i })
})

test('DynCom owning counted arrays use natural managed inputs', (t) => {
  roInitialize()
  const bstrs = DynCom.bstrArray(['embedded\u0000nul', ''])
  const variants = DynCom.variantArray([DynComVariant.i32(17), DynComVariant.bstr('embedded\u0000nul')])
  const iid = WinGuid.parse('00000035-0000-0000-c000-000000000046')
  const interfaces = DynCom.interfaceArray(iid, [DynWinRtValue.activationFactory('Windows.Foundation.Uri')])

  t.is(DynCom.bufferCount(bstrs), 2n)
  t.is(DynCom.bufferCount(variants), 2n)
  t.is(DynCom.bufferCount(interfaces), 1n)
  t.false(bstrs.isNull())
  t.false(variants.isNull())
  t.false(interfaces.isNull())
  t.truthy(DynCom.callerOutputArray(DynCom.bstrType(), 2n))
  t.truthy(DynCom.enumeratorOutputArray(DynCom.variantType(), 2n))
  t.truthy(DynCom.coTaskMemWideStringType())
  const oneShot = DynCom.callerOutputArray(DynCom.bstrType(), 1n)
  t.throws(() => DynCom.takeBstrArray(oneShot), { message: /does not contain owned string/ })
  t.throws(() => DynCom.takeBstrArray(oneShot), { message: /not an owned BSTR array/ })
  t.throws(() => DynCom.interfaceArray(iid, [DynCom.pointer(0n) as unknown as DynWinRtValue]), {
    message: /managed objects/,
  })
})

test('DynCom Automation wrappers preserve tags, values, bounds, and transfer', (t) => {
  roInitialize()
  const unionDescriptor = JSON.stringify({
    name: 'Tests.NativeUnion',
    x86: {
      size: 8,
      alignment: 8,
      fields: [
        { name: 'integer', count: 1, type: { kind: 'u64' } },
        { name: 'pointer', count: 1, type: { kind: 'pointer' } },
      ],
    },
    x64: {
      size: 8,
      alignment: 8,
      fields: [
        { name: 'integer', count: 1, type: { kind: 'u64' } },
        { name: 'pointer', count: 1, type: { kind: 'pointer' } },
      ],
    },
    arm64: {
      size: 8,
      alignment: 8,
      fields: [
        { name: 'integer', count: 1, type: { kind: 'u64' } },
        { name: 'pointer', count: 1, type: { kind: 'pointer' } },
      ],
    },
  })
  const union = DynCom.createNativeUnion(unionDescriptor, 'integer')
  t.is(union.activeField, 'integer')
  t.deepEqual(union.bytes, Buffer.alloc(8))
  t.throws(() => DynCom.createNativeUnion(unionDescriptor, 'missing'), {
    message: /no active field/,
  })

  const variants = [
    DynComVariant.empty(),
    DynComVariant.null(),
    DynComVariant.i8(-1),
    DynComVariant.u8(1),
    DynComVariant.i16(-2),
    DynComVariant.u16(2),
    DynComVariant.i32(-3),
    DynComVariant.u32(3),
    DynComVariant.int(-4),
    DynComVariant.uint(4),
    DynComVariant.i64(-5n),
    DynComVariant.u64(5n),
    DynComVariant.f32(1.5),
    DynComVariant.f64(2.5),
    DynComVariant.bool(true),
    DynComVariant.bstr('automation'),
  ]
  t.deepEqual(
    variants.map((value) => value.kind),
    [
      'empty',
      'null',
      'i8',
      'u8',
      'i16',
      'u16',
      'i32',
      'u32',
      'int',
      'uint',
      'i64',
      'u64',
      'f32',
      'f64',
      'bool',
      'bstr',
    ],
  )
  t.is(variants[14].toBool(), true)
  t.is(variants[15].toStringValue(), 'automation')
  t.throws(() => DynComVariant.i8(128), { message: /out of range/ })

  const unknown = DynComVariant.unknown(DynWinRtValue.activationFactory('Windows.Foundation.Uri'))
  t.is(unknown.kind, 'unknown')
  t.truthy(unknown.toInterface())
  const dispatch = DynComVariant.dispatch()
  t.is(dispatch.kind, 'dispatch')
  t.is(dispatch.toInterface(), null)

  const bounds = [
    { lowerBound: -2, length: 2 },
    { lowerBound: 5, length: 2 },
  ]
  const arrays = [
    DynComSafeArray.i8([-1]),
    DynComSafeArray.u8([1]),
    DynComSafeArray.i16([-2]),
    DynComSafeArray.u16([2]),
    DynComSafeArray.i32([1, 2, 3, 4], bounds),
    DynComSafeArray.u32([3]),
    DynComSafeArray.i64([-4n]),
    DynComSafeArray.u64([4n]),
    DynComSafeArray.f32([1.25]),
    DynComSafeArray.f64([2.5]),
    DynComSafeArray.bool([true, false]),
    DynComSafeArray.bstr(['a', 'b']),
    DynComSafeArray.unknown([]),
    DynComSafeArray.dispatch([]),
    DynComSafeArray.variant([DynComVariant.i32(7), DynComVariant.bstr('v')]),
  ]
  t.deepEqual(
    arrays.map((value) => value.elementType),
    [
      'i8',
      'u8',
      'i16',
      'u16',
      'i32',
      'u32',
      'i64',
      'u64',
      'f32',
      'f64',
      'bool',
      'bstr',
      'unknown',
      'dispatch',
      'variant',
    ],
  )
  t.deepEqual(arrays[4].bounds, bounds)
  t.deepEqual(arrays[4].toNumbers(), [1, 2, 3, 4])
  t.deepEqual(arrays[10].toBools(), [true, false])
  t.deepEqual(arrays[11].toStrings(), ['a', 'b'])
  t.deepEqual(arrays[12].toInterfaces(), [])
  t.deepEqual(arrays[13].toInterfaces(), [])
  t.deepEqual(
    arrays[14].toVariants().map((value) => value.kind),
    ['i32', 'bstr'],
  )
  t.throws(() => DynComSafeArray.i32([1], [{ lowerBound: 0, length: 2 }]), {
    message: /require 2 element/,
  })
  t.throws(
    () =>
      DynComSafeArray.i32(
        [1],
        Array.from({ length: 9 }, () => ({ lowerBound: 0, length: 1 })),
      ),
    { message: /ranks must be between 1 and 8/ },
  )

  const activationFactoryIid = WinGuid.parse('00000035-0000-0000-c000-000000000046')
  const activationFactory = DynWinRtValue.activationFactory('Windows.Foundation.Uri')
  const interfaceArray = DynComSafeArray.interface(
    activationFactoryIid,
    [activationFactory],
    [{ lowerBound: -4, length: 1 }],
  )
  t.is(interfaceArray.elementType, 'unknown')
  t.is(interfaceArray.interfaceIid?.toString().toLowerCase(), activationFactoryIid.toString().toLowerCase())
  t.deepEqual(interfaceArray.bounds, [{ lowerBound: -4, length: 1 }])
  t.is(interfaceArray.toInterfaces().length, 1)
  t.notThrows(() => DynCom.safeArrayType('unknown', activationFactoryIid))
  t.notThrows(() => DynCom.safeArrayType('unknown', activationFactoryIid, true))
  t.throws(() => DynCom.safeArrayType('i32', activationFactoryIid), {
    message: /requires VT_UNKNOWN/,
  })
  t.throws(
    () => DynComSafeArray.interface(WinGuid.parse('ffffffff-ffff-ffff-ffff-ffffffffffff'), [activationFactory]),
    { message: /80004002|interface is not supported|No such interface supported/i },
  )
  const nullSafeArray = DynWinRtValue.nullValue()
  t.is(DynCom.takeNullableSafeArray(nullSafeArray), null)
  const nullableSafeArray = DynCom.safeArray(interfaceArray)
  const takenNullableSafeArray = DynCom.takeNullableSafeArray(nullableSafeArray)
  t.is(takenNullableSafeArray?.interfaceIid?.toString().toLowerCase(), activationFactoryIid.toString().toLowerCase())

  const arrayVariant = DynComVariant.safeArray(arrays[4])
  t.deepEqual(arrayVariant.toSafeArray().toNumbers(), [1, 2, 3, 4])

  const props = [
    DynComPropVariant.empty(),
    DynComPropVariant.null(),
    DynComPropVariant.i8(-1),
    DynComPropVariant.u8(1),
    DynComPropVariant.i16(-2),
    DynComPropVariant.u16(2),
    DynComPropVariant.i32(-3),
    DynComPropVariant.u32(3),
    DynComPropVariant.int(-4),
    DynComPropVariant.uint(4),
    DynComPropVariant.i64(-5n),
    DynComPropVariant.u64(5n),
    DynComPropVariant.f32(1.5),
    DynComPropVariant.f64(2.5),
    DynComPropVariant.bool(true),
    DynComPropVariant.string('property'),
    DynComPropVariant.guid(WinGuid.parse('00112233-4455-6677-8899-aabbccddeeff')),
    DynComPropVariant.filetime(123n),
    DynComPropVariant.blob(Buffer.from([1, 2, 3])),
  ]
  t.deepEqual(
    props.slice(0, 19).map((value) => value.kind),
    [
      'empty',
      'null',
      'i8',
      'u8',
      'i16',
      'u16',
      'i32',
      'u32',
      'int',
      'uint',
      'i64',
      'u64',
      'f32',
      'f64',
      'bool',
      'string',
      'guid',
      'filetime',
      'blob',
    ],
  )
  t.is(props[15].toStringValue(), 'property')
  t.is(props[17].toBigint(), 123n)
  t.deepEqual(props[18].toBlob(), Buffer.from([1, 2, 3]))
  for (const [value, expected] of [
    [DynComPropVariant.i8Vector([-1]), [-1]],
    [DynComPropVariant.u8Vector([1]), [1]],
    [DynComPropVariant.i16Vector([-2]), [-2]],
    [DynComPropVariant.u16Vector([2]), [2]],
    [DynComPropVariant.i32Vector([-3]), [-3]],
    [DynComPropVariant.u32Vector([3]), [3]],
    [DynComPropVariant.f32Vector([1.5]), [1.5]],
    [DynComPropVariant.f64Vector([2.5]), [2.5]],
  ] as const) {
    t.deepEqual(value.toNumbers(), expected)
  }
  t.deepEqual(DynComPropVariant.i64Vector([-4n]).toBigints(), [-4n])
  t.deepEqual(DynComPropVariant.u64Vector([4n]).toBigints(), [4n])
  t.deepEqual(DynComPropVariant.boolVector([true, false]).toBools(), [true, false])
  t.deepEqual(DynComPropVariant.stringVector(['a', 'b']).toStrings(), ['a', 'b'])
  t.is(DynComPropVariant.guidVector([WinGuid.parse('00112233-4455-6677-8899-aabbccddeeff')]).toGuidStrings().length, 1)
  t.deepEqual(DynComPropVariant.filetimeVector([1n, 2n]).toBigints(), [1n, 2n])
  t.deepEqual(DynComPropVariant.boolVector([]).toBools(), [])
  t.deepEqual(DynComPropVariant.stringVector([]).toStrings(), [])
  t.deepEqual(DynComPropVariant.filetimeVector([]).toBigints(), [])

  const stored = DynCom.variant(DynComVariant.bstr('transfer'))
  t.is(DynCom.takeVariant(stored).toStringValue(), 'transfer')
  t.throws(() => DynCom.takeVariant(stored), { message: /not a COM VARIANT/ })

  const released = DynComVariant.i32(1)
  released.release()
  t.throws(() => released.kind, { message: /released/ })

  const dispatchParams = new DynComDispatchParams([DynComVariant.i32(10), DynComVariant.bstr('named')], [42])
  t.is(dispatchParams.argumentCount, 2)
  t.deepEqual(dispatchParams.namedDispids, [42])
  const clonedDispatchParams = dispatchParams.clone()
  dispatchParams.release()
  t.throws(() => dispatchParams.argumentCount, { message: /released/ })
  t.is(clonedDispatchParams.argumentCount, 2)
  t.throws(() => new DynComDispatchParams([DynComVariant.i32(1)], [1, 2]), {
    message: /exceeds argument count/,
  })
  t.throws(() => new DynComDispatchParams([DynComVariant.i32(1)], [1.5]), {
    message: /integral number/,
  })

  const invokeSig = new DynComMethodSig()
    .addIn(DynCom.i32Type())
    .addIn(DynCom.pointerType())
    .addIn(DynCom.u32Type())
    .addIn(DynCom.u16Type())
    .addIn(DynCom.dispatchParamsType())
    .addOptionalOut(DynCom.variantType())
    .addOptionalOut(DynCom.excepInfoType())
    .addOptionalOut(DynCom.u32Type())
    .captureDispatchInvokeHresult()
  const dispatchIid = WinGuid.parse('00020400-0000-0000-c000-000000000046')
  t.notThrows(() =>
    DynCom.registerIUnknownInterface('Windows.Win32.System.Com.IDispatch', dispatchIid).addMethodAt(
      6,
      'Invoke',
      invokeSig,
    ),
  )
  t.throws(
    () =>
      DynCom.registerIUnknownInterface(
        'Tests.INotDispatch',
        WinGuid.parse('10000000-0000-0000-0000-000000000010'),
      ).addMethodAt(6, 'Invoke', invokeSig),
    { message: /restricted to the exact IDispatch::Invoke/ },
  )
})

test('DynCom distinguishes handle-value bytes from data-pointer storage', (t) => {
  const width = process.arch === 'ia32' ? 4 : 8
  const expected = 0x12345678n
  const handle = Buffer.alloc(width)
  if (width === 8) {
    handle.writeBigUInt64LE(expected)
  } else {
    handle.writeUInt32LE(Number(expected))
  }

  t.is(DynCom.handleValue(handle), expected)
  t.is(DynCom.handleValue(expected), expected)
  t.throws(() => DynCom.handleValue(Buffer.alloc(width - 1)), {
    message: /must contain exactly/,
  })
  t.throws(() => DynCom.handleValue(Buffer.alloc(width + 1)), {
    message: /must contain exactly/,
  })
  const wrongTypedArray = new Uint16Array(width / 2) as unknown as Uint8Array
  t.throws(() => DynCom.handleValue(wrongTypedArray), {
    message: /expected bigint, number, Buffer, or Uint8Array/,
  })
  t.throws(() => DynCom.pointer(wrongTypedArray), {
    message: /expected bigint, number, Buffer, Uint8Array/,
  })

  const sid = Buffer.alloc(width)
  sid.set(width === 8 ? [1, 2, 0, 0, 0, 0, 0, 5] : [1, 2, 0, 5])
  const sidPointer = DynCom.pointer(sid)
  t.not(DynCom.asPointerBigint(sidPointer), DynCom.handleValue(sid))
})

test('DynCom rejects a detached handle-value buffer', (t) => {
  const bytes = new Uint8Array(process.arch === 'ia32' ? 4 : 8)
  structuredClone(bytes.buffer, { transfer: [bytes.buffer] })

  const error = t.throws(() => DynCom.handleValue(bytes))
  t.regex(error.message, /detached Buffer/)
})

test('getComputerName', (t) => {
  const name = getComputerName()
  t.truthy(name)
  t.is(typeof name, 'string')
})

test('getWindowsDirectory', (t) => {
  const dir = getWindowsDirectory()
  t.truthy(dir)
  t.is(typeof dir, 'string')
})

test('build WinRT types', (t) => {
  t.truthy(DynWinRtType.i32())
  t.truthy(DynWinRtType.hstring())
  t.truthy(DynWinRtType.object())
  t.truthy(DynWinRtType.iAsyncOperation(DynWinRtType.object()))
})

test('invoke WinRT through dynamic metadata', (t) => {
  roInitialize(1)
  const factoryIid = WinGuid.parse('44a9796f-723e-4fdf-a218-033e75b0c084')
  const uriIid = WinGuid.parse('9e365e57-48b2-4160-956f-c7385120bbfc')
  const uriFactoryType = DynWinRtType.registerInterface('IUriRuntimeClassFactory', factoryIid).addMethod(
    'CreateUri',
    new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.object()),
  )
  const uriType = DynWinRtType.registerInterface('IUriRuntimeClass', uriIid).addMethod(
    'get_AbsoluteUri',
    new DynWinRtMethodSig().addOut(DynWinRtType.hstring()),
  )
  const uriFactory = DynWinRtValue.activationFactory('Windows.Foundation.Uri').cast(factoryIid)
  const expected = 'https://www.example.com/path?q=1#frag'
  const uri = uriFactoryType
    .methodByName('CreateUri')
    .invoke(uriFactory, [DynWinRtValue.hstring(expected)])
    .cast(uriIid)

  t.is(uriType.methodByName('get_AbsoluteUri').invoke(uri, []).toString(), expected)
})

test('resolve WinRT async operations through the Node event loop', async (t) => {
  roInitialize(1)
  const staticsIid = WinGuid.parse('5984c710-daf2-43c8-8bb4-a4d3eacfd03f')
  const storageFileIid = WinGuid.parse('fa3f6186-4214-428c-a64c-14c9ac7315ea')
  const storageFileType = DynWinRtType.runtimeClass(
    'Windows.Storage.StorageFile',
    DynWinRtType.interface(storageFileIid),
  )
  const staticsType = DynWinRtType.registerInterface('IStorageFileStaticsAsyncTest', staticsIid).addMethod(
    'GetFileFromPathAsync',
    new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.iAsyncOperation(storageFileType)),
  )
  const statics = DynWinRtValue.activationFactory('Windows.Storage.StorageFile').cast(staticsIid)
  const operation = staticsType
    .methodByName('GetFileFromPathAsync')
    .invoke(statics, [DynWinRtValue.hstring(process.execPath)])

  const file = await operation.toPromise()

  t.false(file.isNull())
  t.notThrows(() => file.cast(storageFileIid))
})

test('completed operations preserve Node ordering and share one dispatcher', async (t) => {
  const child = spawn(process.execPath, [fileURLToPath(new URL('./async-promise-child.mjs', import.meta.url))], {
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  })

  let stdout = ''
  let stderr = ''
  child.stdout.setEncoding('utf8')
  child.stderr.setEncoding('utf8')
  child.stdout.on('data', (chunk) => {
    stdout += chunk
  })
  child.stderr.on('data', (chunk) => {
    stderr += chunk
  })

  const code = await new Promise<number | null>((resolve, reject) => {
    child.once('error', reject)
    child.once('close', resolve)
  })

  t.is(code, 0, stderr)
  t.regex(stdout, /async-promise-ok/)
})

test('DispatcherQueue shutdown drains accepted work exactly once', async (t) => {
  const child = spawn(process.execPath, [fileURLToPath(new URL('./dispatcher-shutdown-child.mjs', import.meta.url))], {
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  })

  let stdout = ''
  let stderr = ''
  child.stdout.setEncoding('utf8')
  child.stderr.setEncoding('utf8')
  child.stdout.on('data', (chunk) => {
    stdout += chunk
  })
  child.stderr.on('data', (chunk) => {
    stderr += chunk
  })

  const code = await new Promise<number | null>((resolve, reject) => {
    child.once('error', reject)
    child.once('close', resolve)
  })

  t.is(code, 0, stderr)
  t.regex(stdout, /dispatcher-shutdown-ok/)
})

test('environment cleanup prevents late Promise callbacks', async (t) => {
  const child = spawn(process.execPath, [fileURLToPath(new URL('./env-cleanup-child.mjs', import.meta.url))], {
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  })

  let stdout = ''
  let stderr = ''
  child.stdout.setEncoding('utf8')
  child.stderr.setEncoding('utf8')
  child.stdout.on('data', (chunk) => {
    stdout += chunk
  })
  child.stderr.on('data', (chunk) => {
    stderr += chunk
  })

  const code = await new Promise<number | null>((resolve, reject) => {
    child.once('error', reject)
    child.once('close', resolve)
  })

  t.is(code, 0, stderr)
  t.regex(stdout, /env-cleanup-ok/)
})

test('resolve WinRT async operations on an STA without a WinUI message pump', async (t) => {
  const child = spawn(process.execPath, [fileURLToPath(new URL('./sta-async-child.mjs', import.meta.url))], {
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  })

  let stdout = ''
  let stderr = ''
  child.stdout.setEncoding('utf8')
  child.stderr.setEncoding('utf8')
  child.stdout.on('data', (chunk) => {
    stdout += chunk
  })
  child.stderr.on('data', (chunk) => {
    stderr += chunk
  })

  let timedOut = false
  const timeout = setTimeout(() => {
    timedOut = true
    child.kill()
  }, 10_000)
  t.teardown(() => {
    clearTimeout(timeout)
    if (child.exitCode === null && child.signalCode === null) {
      child.kill()
    }
  })

  const code = await new Promise<number | null>((resolve, reject) => {
    child.once('error', reject)
    child.once('close', resolve)
  })
  clearTimeout(timeout)

  t.false(timedOut, 'STA child hung waiting for the shared Node dispatcher')
  t.is(code, 0, stderr)
  t.regex(stdout, /sta-async-ok/)
})

const testDirectory = fileURLToPath(new URL('.', import.meta.url))
const siblingJsxRoot = resolve(testDirectory, '..', '..', '..', '..', 'dynwinrt-jsx')
const winuiApplicationModule =
  process.env.DYNWINRT_TEST_WINUI_APPLICATION ??
  resolve(siblingJsxRoot, 'examples', 'dashboard', '.winapp', 'bindings', 'Application.js')
const winuiBootstrapDll =
  process.env.DYNWINRT_TEST_WINAPPSDK_BOOTSTRAP ??
  resolve(siblingJsxRoot, '.winapp', 'bin', 'x64', 'Microsoft.WindowsAppRuntime.Bootstrap.dll')
const missingWinuiFixtures = [
  !existsSync(winuiApplicationModule) ? winuiApplicationModule : undefined,
  !existsSync(winuiBootstrapDll) ? winuiBootstrapDll : undefined,
].filter((value): value is string => value !== undefined)

if (missingWinuiFixtures.length > 0) {
  console.warn(
    `[dynwinrt] skipping native WinUI Promise checkpoint test; missing fixture(s): ${missingWinuiFixtures.join(', ')}`,
  )
  test.skip('WinUI scheduled start drains Promise reactions inside Application.Start', () => {})
} else {
  test('WinUI scheduled start drains Promise reactions inside Application.Start', async (t) => {
    const runtimeModule = fileURLToPath(new URL('../dist/winrt.js', import.meta.url))
    const child = spawn(
      process.execPath,
      [
        fileURLToPath(new URL('./dispatcher-queue-winui-child.cjs', import.meta.url)),
        winuiApplicationModule,
        winuiBootstrapDll,
        runtimeModule,
        'scheduled',
      ],
      {
        stdio: ['ignore', 'pipe', 'pipe'],
        windowsHide: true,
      },
    )
    let stdout = ''
    let stderr = ''
    child.stdout.setEncoding('utf8')
    child.stderr.setEncoding('utf8')
    child.stdout.on('data', (chunk) => {
      stdout += chunk
    })
    child.stderr.on('data', (chunk) => {
      stderr += chunk
    })
    let timedOut = false
    const timeout = setTimeout(() => {
      timedOut = true
      child.kill()
    }, 20_000)
    t.teardown(() => {
      clearTimeout(timeout)
      if (child.exitCode === null && child.signalCode === null) {
        child.kill()
      }
    })

    const code = await new Promise<number | null>((resolveClose, reject) => {
      child.once('error', reject)
      child.once('close', resolveClose)
    })
    clearTimeout(timeout)

    t.false(timedOut, 'native WinUI Promise checkpoint test timed out')
    t.is(code, 0, stderr)
    t.regex(stdout, /dispatcher-queue-winui-ok/)
  })
}

test('round-trip WinRT values', (t) => {
  t.is(DynWinRtValue.i32(42).toNumber(), 42)
  t.is(DynWinRtValue.u32(0xffffffff).toNumber(), 0xffffffff)
  t.is(DynWinRtValue.i64(-42n).toI64Bigint(), -42n)
  t.is(DynWinRtValue.u64(2n ** 63n).toU64Bigint(), 2n ** 63n)
  t.is(DynWinRtValue.u64(42).toI64(), 42)
  t.throws(() => DynWinRtValue.i64(9_007_199_254_740_992n).toI64(), {
    message: /safe-integer range.*bigint conversion/,
  })
  t.throws(() => DynWinRtValue.u64(9_007_199_254_740_992n).toI64(), {
    message: /safe-integer range.*bigint conversion/,
  })
  t.throws(() => DynWinRtValue.u64(2n ** 63n).toI64(), {
    message: /greater than i64::MAX/,
  })
  t.throws(() => DynWinRtValue.u64(-1), {
    message: /non-negative safe integer/,
  })
  t.throws(() => DynWinRtValue.i64(1.5), { message: /safe integer/ })
  t.throws(() => DynWinRtValue.u64(Number.POSITIVE_INFINITY), {
    message: /safe integer/,
  })
  t.throws(() => DynWinRtValue.u64(Number.MAX_SAFE_INTEGER + 1), {
    message: /use bigint/,
  })
  t.is(DynWinRtValue.hstring('hello').toString(), 'hello')
  t.true(DynWinRtValue.nullValue().isNull())
})

test('invalid WinRT value conversions throw JavaScript errors', (t) => {
  const value = DynWinRtValue.hstring('not numeric')

  t.throws(() => value.toNumber(), { message: /Cannot convert.*to number/ })
  t.throws(() => value.toBool(), { message: /Cannot convert.*to number/ })
  t.throws(() => value.toI64(), { message: /Cannot convert.*to number/ })
  t.throws(() => value.toF64(), { message: /Cannot convert.*to number/ })
  t.throws(() => value.asRaw(), { message: /non-object/ })
  t.throws(() => DynCom.toNumber(value), { message: /Cannot convert.*to number/ })
})

test('u64 arrays and struct fields preserve the full unsigned range', (t) => {
  const values = [0n, 2n ** 63n, 2n ** 64n - 1n]
  const array = DynWinRtArray.fromU64Values(values)
  t.deepEqual(array.toU64Vec(), values)
  t.deepEqual(DynWinRtArray.fromU64Values([42]).toU64Vec(), [42n])
  t.throws(() => DynWinRtArray.fromU64Values([-1]), {
    message: /non-negative safe integer/,
  })
  t.throws(() => DynWinRtArray.fromU64Values([2n ** 64n]), {
    message: /fit in an unsigned 64-bit integer/,
  })

  const structType = DynWinRtType.structType('DynWinRT.Tests.IntegerBoundary', [
    DynWinRtType.u64(),
    DynWinRtType.i64(),
  ])
  const value = DynWinRtStruct.create(structType)
  value.setU64(0, 2n ** 64n - 1n)
  t.is(value.getU64(0), 2n ** 64n - 1n)
  t.throws(() => value.setU64(0, -1n), {
    message: /fit in an unsigned 64-bit integer/,
  })
  value.setI64(1, -(2n ** 63n))
  t.is(value.getI64(1), -(2n ** 63n))
  t.throws(() => value.setI64(1, 2n ** 63n), {
    message: /fit in a signed 64-bit integer/,
  })

  const signedValues = [-(2n ** 63n), 0n, 2n ** 63n - 1n]
  t.deepEqual(DynWinRtArray.fromI64Values(signedValues).toI64Vec(), signedValues)
  t.throws(() => DynWinRtArray.fromI64Values([2n ** 63n]), {
    message: /fit in a signed 64-bit integer/,
  })
})

test('struct field access rejects invalid indexes, types, ranges, and identities', (t) => {
  t.throws(() => DynWinRtStruct.create(DynWinRtType.i32()), {
    message: /requires a struct type/,
  })

  const scalarType = DynWinRtType.structType('DynWinRT.Tests.ValidatedScalarFields', [
    DynWinRtType.i32(),
    DynWinRtType.u8(),
  ])
  const scalar = DynWinRtStruct.create(scalarType)
  t.throws(() => scalar.getI32(2), { message: /out of bounds/ })
  t.throws(() => scalar.getI32(2 ** 32), { message: /u32 range/ })
  t.throws(() => scalar.getI32(1.9), { message: /integer in the u32 range/ })
  t.throws(() => scalar.getU64(0), { message: /expected u64/ })
  t.throws(() => scalar.setU8(1, 256), { message: /outside the u8 range/ })
  t.throws(() => scalar.setU8(1, 2 ** 32), { message: /u32 range/ })
  t.throws(() => scalar.setU8(1, 1.9), { message: /integer in the u32 range/ })
  t.throws(() => scalar.setI32(0, 2 ** 32), { message: /i32 range/ })

  const innerAType = DynWinRtType.structType('DynWinRT.Tests.InnerA', [DynWinRtType.i32()])
  const innerBType = DynWinRtType.structType('DynWinRT.Tests.InnerB', [DynWinRtType.i32()])
  const outerType = DynWinRtType.structType('DynWinRT.Tests.Outer', [innerAType])
  const outer = DynWinRtStruct.create(outerType)
  const innerB = DynWinRtStruct.create(innerBType)
  t.throws(() => outer.setStruct(0, innerB), {
    message: /requires Struct.*found Struct/,
  })
})

test('box IReference values', (t) => {
  const valueType = DynWinRtType.u32()
  const referenceType = DynWinRtType.parameterized(WinGuid.parse('61c17706-2d65-11e0-9ae8-d48564015472'), [valueType])
  const reference = DynWinRtType.registerInterface('IReference_UInt32_Test', referenceType.iid()).addMethod(
    'get_Value',
    new DynWinRtMethodSig().addOut(valueType),
  )
  const boxed = DynWinRtValue.boxReference(DynWinRtValue.u32(17), valueType)

  t.is(reference.method(6).invoke(boxed, []).toNumber(), 17)
  t.true(DynWinRtValue.boxReference(DynWinRtValue.nullValue(), valueType).isNull())
})

test('round-trip IReference struct fields', (t) => {
  roInitialize(1)
  const valueType = DynWinRtType.u64()
  const referenceType = DynWinRtType.parameterized(WinGuid.parse('61c17706-2d65-11e0-9ae8-d48564015472'), [valueType])
  const reference = DynWinRtType.registerInterface('IReference_UInt64_Struct_Test', referenceType.iid()).addMethod(
    'get_Value',
    new DynWinRtMethodSig().addOut(valueType),
  )
  const holderType = DynWinRtType.structType('DynWinRT.Tests.OptionalUInt64Holder', [referenceType])
  const holder = DynWinRtStruct.create(holderType)

  holder.setObject(0, DynWinRtValue.boxReference(DynWinRtValue.u64(17n), valueType))
  t.is(reference.method(6).invoke(holder.getObject(0), []).toU64Bigint(), 17n)

  const incompatible = DynWinRtValue.activationFactory('Windows.Foundation.Uri')
  t.throws(() => holder.setObject(0, incompatible), {
    message: /80004002|No such interface/i,
  })
  incompatible.release()
  t.is(reference.method(6).invoke(holder.getObject(0), []).toU64Bigint(), 17n)

  holder.setObject(0, DynWinRtValue.nullValue())
  t.true(holder.getObject(0).isNull())
  t.throws(() => holder.setObject(0, DynWinRtValue.u64(17n)), {
    message: /WinRT object or null/,
  })
})

test('release WinRT object values deterministically', (t) => {
  const value = DynWinRtValue.activationFactory('Windows.Foundation.Uri')
  value.release()
  t.true(value.isNull())
  t.notThrows(() => value.release())
})

test('round-trip WinRT value arrays', (t) => {
  const array = DynWinRtArray.fromI32Values([1, 2, 3])
  t.is(array.len(), 3)
  t.deepEqual(array.toI32Vec(), [1, 2, 3])
  t.deepEqual(
    array.toValues().map((value) => value.toNumber()),
    [1, 2, 3],
  )
  t.deepEqual([...DynWinRtArray.fromU8Values([1, 2, 3]).toBuffer()], [1, 2, 3])
  t.throws(() => array.toU64Vec(), { message: /element type I32/ })
  t.throws(() => array.get(3), { message: /out of bounds/ })
  t.throws(() => array.get(2 ** 32), { message: /u32 range/ })
  t.throws(() => array.get(1.9), { message: /integer in the u32 range/ })
})

test('create empty vectors for large struct element types', (t) => {
  const rectType = DynWinRtType.structType('Windows.Graphics.RectInt32', [
    DynWinRtType.i32(),
    DynWinRtType.i32(),
    DynWinRtType.i32(),
    DynWinRtType.i32(),
  ])
  const vector = DynWinRtValue.createVector([], rectType)
  const vectorIid = DynWinRtType.parameterized(WinGuid.parse('913337e9-11a1-4345-a3a2-4e7f956e222d'), [rectType]).iid()

  t.notThrows(() => vector.cast(vectorIid))
})

test('parse GUIDs', (t) => {
  const iid = '00000000-0000-0000-c000-000000000046'
  t.is(WinGuid.parse(iid).toString().toLowerCase(), iid)
  t.throws(() => WinGuid.parse('not-a-guid'))
})

test('report package identity state', (t) => {
  t.is(typeof hasPackageIdentity(), 'boolean')
})

test('progress callbacks do not keep Node alive after completion', async (t) => {
  const child = spawn(process.execPath, [fileURLToPath(new URL('./progress-exit-child.mjs', import.meta.url))], {
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  })

  let stdout = ''
  let stderr = ''
  child.stdout.setEncoding('utf8')
  child.stderr.setEncoding('utf8')
  child.stdout.on('data', (chunk) => {
    stdout += chunk
  })
  child.stderr.on('data', (chunk) => {
    stderr += chunk
  })

  let timedOut = false
  const timeout = setTimeout(() => {
    timedOut = true
    child.kill()
  }, 10_000)
  t.teardown(() => {
    clearTimeout(timeout)
    if (child.exitCode === null && child.signalCode === null) {
      child.kill()
    }
  })

  const code = await new Promise<number | null>((resolve, reject) => {
    child.once('error', reject)
    child.once('close', resolve)
  })
  clearTimeout(timeout)

  t.false(timedOut, 'child process remained alive after the progress operation completed')
  t.is(code, 0, stderr)
  t.regex(stdout, /progress-exit-ok/)
})
