// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import childProcess from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { syncBuiltinESMExports } from 'node:module'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { runCodegen } from './run-codegen.mjs'

test('prebuilt invocation executes the selected file, arguments and environment without Cargo', () => {
  const env = { ...process.env, DYNWINRT_CODEGEN: process.execPath, CODEGEN_SENTINEL: 'forwarded' }
  const result = runCodegen(['-e', 'console.log(process.env.CODEGEN_SENTINEL); process.exit(23)'], {
    encoding: 'utf8',
    env,
    windowsHide: true,
  })
  assert.equal(result.status, 23)
  assert.equal(result.stdout.trim(), 'forwarded')
})

test('an invalid executable reports its launch error without trying Cargo', (t) => {
  const calls = []
  const error = new Error('not a valid executable')
  t.mock.method(childProcess, 'spawnSync', (...args) => {
    calls.push(args)
    return { error, status: null, stdout: null, stderr: null }
  })
  syncBuiltinESMExports()
  try {
    assert.throws(
      () => runCodegen(['generate'], { encoding: 'utf8', env: { DYNWINRT_CODEGEN: process.execPath } }),
      (actual) => actual === error,
    )
    assert.equal(calls.length, 1)
    assert.equal(calls[0][0], process.execPath)
  } finally {
    t.mock.restoreAll()
    syncBuiltinESMExports()
  }
})

test('invalid explicit paths never fall back; omission retains the developer Cargo command', (t) => {
  const directory = mkdtempSync(join(tmpdir(), 'dynwinrt-codegen-selection-'))
  const calls = []
  t.mock.method(childProcess, 'spawnSync', (...args) => {
    calls.push(args)
    return { status: 31, stdout: '', stderr: 'cargo boundary' }
  })
  syncBuiltinESMExports()
  try {
    for (const path of ['', ' ', directory, join(directory, 'missing.exe')]) {
      assert.throws(() => runCodegen(['generate'], { encoding: 'utf8', env: { DYNWINRT_CODEGEN: path } }))
    }
    assert.deepEqual(calls, [])
    const options = { encoding: 'utf8', env: {}, cwd: directory }
    assert.equal(runCodegen(['generate', '--output', 'a path with spaces'], options).status, 31)
    assert.deepEqual(calls, [
      [
        'cargo',
        ['run', '--quiet', '-p', 'dynwinrt-codegen', '--', 'generate', '--output', 'a path with spaces'],
        options,
      ],
    ])
  } finally {
    t.mock.restoreAll()
    syncBuiltinESMExports()
    rmSync(directory, { recursive: true, force: true })
  }
})
