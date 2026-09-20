// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'

test('native class conversions reject old/foreign tags before bodies and finalize controls once', (t) => {
  const fixture = fileURLToPath(new URL('./fixtures/native-class-boundary.cjs', import.meta.url))
  const child = spawnSync(process.execPath, ['--expose-gc', fixture, 'native-hooks'], {
    encoding: 'utf8',
    timeout: 30_000,
    windowsHide: true,
  })
  assert.equal(child.status, 0, `${child.error ?? ''}\n${child.stdout}\n${child.stderr}`)
  const report = JSON.parse(child.stdout.trim())
  t.diagnostic(child.stdout.trim())
  assert.equal(report.rejected, 14)
  assert.equal(report.entriesForInvalidInputs, 0)
  assert.equal(report.finalized, 3)
})
