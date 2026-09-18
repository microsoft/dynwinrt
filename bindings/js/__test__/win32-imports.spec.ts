// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { readPeImports, verifyAddonImports } from '../scripts/pe-imports.mjs'
import { isMissingMediaFoundation } from './helpers/optional-media-foundation.mjs'

const require = createRequire(import.meta.url)
const dist = fileURLToPath(new URL('../dist/', import.meta.url))

test('Win32 addon PE ordinary imports exclude optional subsystem DLLs', (t) => {
  const hostAddon = `dynwinrt.win32-${process.arch}-msvc.node`
  const imports = readPeImports(readFileSync(join(dist, hostAddon)))
  const hasMediaFixture = imports.some(
    ({ dll, symbols }) => dll.toLowerCase() === 'mfplat.dll' && symbols.includes('MFCreateSample'),
  )
  const testHooksAddon =
    hasMediaFixture && typeof require(join(dist, hostAddon)).DynComBorrowedCopyTestFixture === 'function'
      ? hostAddon
      : undefined
  const checked = verifyAddonImports(dist, { testHooksAddon })
  t.true(checked.includes(hostAddon))
  t.true(imports.length > 0, 'The actual addon has ordinary imports; this is not a source or loaded-module scan')
  if (testHooksAddon) t.log('test-hooks: only mfplat.dll!MFCreateSample is permitted for the COM media fixture')
})

test('Win32 Media Foundation skips require actionable missing system DLL or lifecycle export errors', (t) => {
  const missingDll =
    '0x8007007E: System DLL `mfplat.dll` could not be loaded from System32: The specified module could not be found.'
  t.true(isMissingMediaFoundation(new Error(missingDll)))
  for (const symbol of ['MFStartup', 'MFShutdown']) {
    t.true(isMissingMediaFoundation(new Error(`0x8007007F: Export \`${symbol}\` was not found in \`mfplat.dll\``)))
  }
  for (const error of [
    missingDll,
    { message: missingDll },
    new Error('The specified module could not be found.'),
    new Error('MFStartup failed: 0x8007007E'),
    new Error('MFStartup failed: 0x80004005'),
    new Error(missingDll.replace('0x8007007E', '0x80070005')),
    new Error(missingDll.replace('0x8007007E', '0x800700C1')),
    new Error(missingDll.replace('mfplat.dll', 'other.dll')),
    new Error(missingDll.replace('System32', 'the current directory')),
    new Error('0x8007007F: Export `MFGetTimerPeriodicity` was not found in `mfplat.dll`'),
    new Error('0x8007007F: Export `mfstartup` was not found in `mfplat.dll`'),
    new Error('0x8007007F: Export `MFStartup` was not found in `other.dll`'),
    new Error('0x80070005: Export `MFStartup` was not found in `mfplat.dll`'),
  ]) {
    t.false(isMissingMediaFoundation(error))
  }
})

test('Win32 Media Foundation initialization is explicit and isolated from WinRT and COM imports', async (t) => {
  const winrt = await import('../dist/winrt.js')
  const com = await import('../dist/com.js')
  const { DynWin32 } = await import('../dist/win32.js')
  t.is(typeof winrt.WinGuid.parse, 'function')
  t.is(typeof com.initializeCom, 'function')
  let context
  try {
    context = DynWin32.initializeMediaFoundation()
  } catch (error) {
    if (!isMissingMediaFoundation(error)) throw error
    t.log(`SKIP optional Media Foundation context checks; initialization reported: ${error.message}`)
    t.pass('Initialization rejected the positively identified missing system component')
    return
  }
  try {
    t.is(context.subsystem, 'mediaFoundation')
    t.false(context.closed)
    t.notThrows(() => DynWin32.requireSubsystem(context, 'mediaFoundation'))
  } finally {
    context.close()
  }
  t.true(context.closed)
  t.throws(() => DynWin32.requireSubsystem(context, 'mediaFoundation'), { message: /context is closed/ })
  t.notThrows(() => context.close())
})
