// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'

const fixture = fileURLToPath(new URL('./fixtures/native-class-boundary.cjs', import.meta.url))
for (const mode of [
  'resource-control',
  'resource',
  'controls',
  'identity',
  'collections',
  'accessors',
  'bridges',
  'workers',
]) {
  test(`native class boundary: ${mode}`, (t) => {
    const child = spawnSync(process.execPath, [fixture, mode], {
      encoding: 'utf8',
      timeout: 30_000,
      windowsHide: true,
    })
    assert.equal(child.status, 0, `${child.error ?? ''}\n${child.stdout}\n${child.stderr}`)
    assert.equal(child.signal, null)
    const report = JSON.parse(child.stdout.trim())
    t.diagnostic(child.stdout.trim())
    assert.equal(report.mode, mode)
    if (mode !== 'resource-control') assert.ok(report.rejected > 0)
    if (mode.startsWith('resource')) assert.equal(report.closes, 1)
    if (mode === 'workers') assert.equal(report.workers, 4)
  })
}
