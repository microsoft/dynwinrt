// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'
import { spawn, spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import {
  DynWinRtArray,
  DynWinRtMethodSig,
  DynWinRtType,
  DynWinRtValue,
  WinGuid,
  getComputerName,
  getWindowsDirectory,
  hasPackageIdentity,
  roInitialize,
} from '../dist/winrt.js'
import * as winrtRuntime from '../dist/winrt.js'
import { DynCom, DynComMethodSig } from '../dist/com.js'
import { DynWin32 } from '../dist/win32.js'

test('Classic COM is isolated from the WinRT root entrypoint', (t) => {
  t.false(Object.prototype.hasOwnProperty.call(winrtRuntime, 'DynCom'))
  t.truthy(DynCom)

  const assertion =
    "const assert = require('node:assert/strict');" +
    "const winrt = require('@microsoft/dynwinrt');" +
    "const com = require('@microsoft/dynwinrt/com');" +
    "assert.equal(Object.prototype.hasOwnProperty.call(winrt, 'DynCom'), false);" +
    "assert.equal(typeof winrt.DynWinRtType, 'function');" +
    "assert.equal(typeof com.DynCom, 'function');" +
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
    "assert.equal(Object.prototype.hasOwnProperty.call(winrt, 'DynCom'), false);" +
    "assert.equal(typeof winrt.DynWinRtType, 'function');" +
    "assert.equal(typeof com.DynCom, 'function');" +
    "console.log('runtime-entrypoints-ok')"
  const esm = spawnSync(process.execPath, ['--input-type=module', '--eval', esmAssertion], {
    cwd: resolve(process.cwd()),
    encoding: 'utf8',
    windowsHide: true,
  })
  t.is(esm.status, 0, esm.stderr)
  t.regex(esm.stdout, /runtime-entrypoints-ok/)
})

test('flat Win32 is isolated from the WinRT root entrypoint', (t) => {
  t.false(Object.prototype.hasOwnProperty.call(winrtRuntime, 'DynWin32'))
  t.truthy(DynWin32)

  const assertion =
    "const assert = require('node:assert/strict');" +
    "const winrt = require('@microsoft/dynwinrt');" +
    "const win32 = require('@microsoft/dynwinrt/win32');" +
    "assert.equal(Object.prototype.hasOwnProperty.call(winrt, 'DynWin32'), false);" +
    "assert.equal(typeof win32.DynWin32, 'function');" +
    "console.log('win32-entrypoint-ok')"
  const cjs = spawnSync(process.execPath, ['--eval', assertion], {
    cwd: resolve(process.cwd()),
    encoding: 'utf8',
    windowsHide: true,
  })
  t.is(cjs.status, 0, cjs.stderr)
  t.regex(cjs.stdout, /win32-entrypoint-ok/)
})

test('DynWin32 validates scalar widths and retained pointer storage', (t) => {
  t.notThrows(() => DynWin32.i64(-(2n ** 63n)))
  t.notThrows(() => DynWin32.i64(Number.MAX_SAFE_INTEGER))
  t.throws(() => DynWin32.i64(2n ** 63n), { message: /signed 64-bit/ })
  t.throws(() => DynWin32.i64(1.5), { message: /safe integer/ })
  t.throws(() => DynWin32.i8(128), { message: /range/ })
  t.throws(() => DynWin32.i8(4_294_967_297), { message: /range/ })
  t.throws(() => DynWin32.i32(4_294_967_296), { message: /range/ })
  t.throws(() => DynWin32.u32(-1), { message: /range/ })
  t.throws(() => DynWin32.u16(1.5), { message: /integer/ })
  t.is(DynWin32.toPointerBigint(DynWin32.handle(-1n)), (2n ** 64n) - 1n)
  t.is(DynWin32.toPointerBigint(DynWin32.handle(-2)), (2n ** 64n) - 2n)

  const bytes = new Uint8Array(8)
  const pointer = DynWin32.pointer(bytes)
  structuredClone(bytes.buffer, { transfer: [bytes.buffer] })
  const error = t.throws(() => DynWin32.toPointerBigint(pointer))
  t.regex(error.message, /backing ArrayBuffer is detached/)
})

test('DynWinRtValue accepts lossless UInt64 bigint inputs', (t) => {
  const max = (2n ** 64n) - 1n
  t.is(DynCom.toU64Bigint(DynWinRtValue.u64(max)), max)
  t.throws(() => DynWinRtValue.u64(-1n), { message: /unsigned 64-bit/ })
  t.throws(() => DynWinRtValue.u64(Number.MAX_SAFE_INTEGER + 1), {
    message: /safe integer/,
  })
})

test('DynCom rejects pointers after their TypedArray backing store is detached', (t) => {
  const bytes = new Uint8Array(16)
  const pointer = DynCom.pointer(bytes)

  structuredClone(bytes.buffer, { transfer: [bytes.buffer] })

  t.is(bytes.byteLength, 0)
  const error = t.throws(() => DynCom.asPointerBigint(pointer))
  t.regex(error.message, /backing ArrayBuffer is detached/)
})

test('DynCom does not adopt borrowed raw pointer bits as owned COM references', (t) => {
  const borrowed = DynCom.pointer(0n)
  const error = t.throws(() => DynCom.adoptComPointer(borrowed))
  t.regex(error.message, /only owned native outputs may be consumed/)
})

test('DynCom exposes HSTRING and semantic HRESULT primitives', (t) => {
  t.is(DynCom.hstring('dynwinrt').toString(), 'dynwinrt')
  t.truthy(DynCom.hstringType())
  t.truthy(new DynComMethodSig().preserveHresult())
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
  t.is(DynWinRtValue.hstring('hello').toString(), 'hello')
  t.true(DynWinRtValue.nullValue().isNull())
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

test('release WinRT object values deterministically', (t) => {
  const value = DynWinRtValue.activationFactory('Windows.Foundation.Uri')
  value.release()
  t.true(value.isNull())
  t.notThrows(() => value.release())
})

test('round-trip WinRT value arrays', (t) => {
  const array = DynWinRtArray.fromI32Values([1, 2, 3])
  t.is(array.len(), 3)
  t.deepEqual(
    array.toValues().map((value) => value.toNumber()),
    [1, 2, 3],
  )
})

test('create empty vectors for large struct element types', (t) => {
  const rectType = DynWinRtType.structType('Windows.Graphics.RectInt32', [
    DynWinRtType.i32(),
    DynWinRtType.i32(),
    DynWinRtType.i32(),
    DynWinRtType.i32(),
  ])
  const vector = DynWinRtValue.createVector([], rectType)
  const vectorIid = DynWinRtType.parameterized(
    WinGuid.parse('913337e9-11a1-4345-a3a2-4e7f956e222d'),
    [rectType],
  ).iid()

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
