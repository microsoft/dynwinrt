// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'
import { spawn } from 'node:child_process'
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
} from '../dist/index.js'

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
  const storageFileType = DynWinRtType.runtimeClass('Windows.Storage.StorageFile', storageFileIid)
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
    const runtimeModule = fileURLToPath(new URL('../dist/index.js', import.meta.url))
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
