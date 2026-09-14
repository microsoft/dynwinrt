// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'
import { spawnSync } from 'node:child_process'
import { existsSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const packageDir = fileURLToPath(new URL('..', import.meta.url))
const distDir = join(packageDir, 'dist')

function findDumpbin(): string {
  const vswhere = join(
    process.env['ProgramFiles(x86)'] ?? 'C:\\Program Files (x86)',
    'Microsoft Visual Studio',
    'Installer',
    'vswhere.exe',
  )
  const installation = spawnSync(vswhere, ['-latest', '-products', '*', '-property', 'installationPath'], {
    encoding: 'utf8',
    timeout: 10_000,
    windowsHide: true,
  })
  if (installation.status !== 0 || !installation.stdout.trim()) {
    throw new Error(`MSVC dumpbin is required to check native imports: ${installation.error ?? installation.stderr}`)
  }
  const toolsDir = join(installation.stdout.trim(), 'VC', 'Tools', 'MSVC')
  const versions = readdirSync(toolsDir).sort((a, b) => b.localeCompare(a, undefined, { numeric: true }))
  const hosts = process.arch === 'arm64' ? ['Hostarm64\\arm64', 'Hostx64\\x64'] : ['Hostx64\\x64']
  for (const version of versions) {
    for (const host of hosts) {
      const candidate = join(toolsDir, version, 'bin', host, 'dumpbin.exe')
      if (existsSync(candidate)) return candidate
    }
  }
  throw new Error(`MSVC dumpbin was not found under ${toolsDir}`)
}

test('native addons do not statically import CoreMessaging or CreateDispatcherQueueController', (t) => {
  const artifacts = readdirSync(distDir).filter((name) => /^dynwinrt\.win32-.*\.node$/.test(name))
  t.true(artifacts.length > 0, 'Build the production JS addon before checking imports')
  const dumpbin = findDumpbin()
  for (const artifact of artifacts) {
    const imports = spawnSync(dumpbin, ['/nologo', '/imports', join(distDir, artifact)], {
      encoding: 'utf8',
      timeout: 30_000,
      windowsHide: true,
    })
    t.is(imports.status, 0, `${artifact}: ${imports.error ?? imports.stderr}`)
    t.notRegex(imports.stdout, /coremessaging\.dll|CreateDispatcherQueueController/i, artifact)
  }
})

for (const entrypoint of ['@microsoft/dynwinrt', '@microsoft/dynwinrt/com']) {
  test(`fresh ${entrypoint} import does not load CoreMessaging for DispatcherQueue`, (t) => {
    const child = spawnSync(
      process.execPath,
      [
        '--eval',
        `
          const assert = require('node:assert/strict');
          const { basename } = require('node:path');
          const modules = () => process.report.getReport().sharedObjects.map(p => basename(p).toLowerCase());
          const before = modules();
          const runtime = require(${JSON.stringify(entrypoint)});
          assert.equal(typeof runtime.DynWinRtValue, 'function');
          const after = modules();
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
    t.is(child.status, 0, `${child.error ?? child.stderr}`)
    t.regex(child.stdout, /lazy-coremessaging-import-ok/)
  })
}
