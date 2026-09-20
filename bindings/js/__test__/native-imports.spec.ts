// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'
import { spawnSync, type SpawnSyncReturns } from 'node:child_process'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { verifyDispatcherQueueImports, verifyUiHelperImports } from '../scripts/pe-imports.mjs'

const packageDir = fileURLToPath(new URL('..', import.meta.url))
const distDir = join(packageDir, 'dist')

function importDiagnostics(
  child: Pick<SpawnSyncReturns<string>, 'status' | 'signal' | 'error' | 'stdout' | 'stderr'>,
): string {
  return `status=${child.status}, signal=${child.signal}\n${child.error ?? ''}\n${child.stdout}\n${child.stderr}`
}

test('fresh import timeout diagnostics retain phase output alongside the spawn error', (t) => {
  const diagnostics = importDiagnostics({
    status: null,
    signal: 'SIGTERM',
    error: Object.assign(new Error('spawnSync node ETIMEDOUT'), { code: 'ETIMEDOUT' }),
    stdout: 'child stdout',
    stderr: '[native-import] before-report:start 0.0ms\n[native-import] before-report:end 42.0ms\n',
  })
  t.true(diagnostics.includes('status=null, signal=SIGTERM'))
  t.true(diagnostics.includes('ETIMEDOUT'))
  t.true(diagnostics.includes('child stdout'))
  t.true(diagnostics.includes('before-report:start 0.0ms'))
  t.true(diagnostics.includes('before-report:end 42.0ms'))
})

test('native addons do not statically import CoreMessaging or CreateDispatcherQueueController', (t) => {
  t.true(verifyDispatcherQueueImports(distDir).length > 0)
})

test('production native addons have no ordinary or delay GDI/USER32 helper imports', (t) => {
  t.true(verifyUiHelperImports(distDir).length > 0)
})

for (const entrypoint of ['@microsoft/dynwinrt', '@microsoft/dynwinrt/com']) {
  test(`fresh ${entrypoint} import does not load CoreMessaging for DispatcherQueue`, (t) => {
    const child = spawnSync(
      process.execPath,
      [
        '--eval',
        `
          const assert = require('node:assert/strict');
          const { writeSync } = require('node:fs');
          const { basename } = require('node:path');
          const { performance } = require('node:perf_hooks');
          const started = performance.now();
          const phase = name => writeSync(
            2, '[native-import] ' + name + ' ' + (performance.now() - started).toFixed(1) + 'ms\\n'
          );
          // Only sharedObjects is needed; avoid network enumeration and DNS during reports.
          // Feature detection preserves the check on Node versions before 20.13/22.
          if ('excludeNetwork' in process.report) process.report.excludeNetwork = true;
          phase('before-report:start');
          const modules = () => process.report.getReport().sharedObjects.map(p => basename(p).toLowerCase());
          const before = modules();
          phase('before-report:end');
          phase('require:start ' + ${JSON.stringify(entrypoint)});
          const runtime = require(${JSON.stringify(entrypoint)});
          phase('require:end');
          assert.equal(typeof runtime.DynWinRtValue, 'function');
          phase('after-report:start');
          const after = modules();
          phase('after-report:end');
          assert(after.some(p => /^dynwinrt\\.win32-.*\\.node$/.test(p)), 'native addon was not loaded');
          assert(
            before.includes('coremessaging.dll') || !after.includes('coremessaging.dll'),
            'ordinary import introduced CoreMessaging.dll'
          );
          console.log('lazy-coremessaging-import-ok');
        `,
      ],
      { cwd: packageDir, encoding: 'utf8', timeout: 10_000, windowsHide: true },
    )
    t.is(child.status, 0, importDiagnostics(child))
    t.regex(child.stdout, /lazy-coremessaging-import-ok/)
  })
}
