// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

'use strict'

const assert = require('node:assert/strict')
const { createHash } = require('node:crypto')
const { mkdtempSync, mkdirSync, writeFileSync, rmSync } = require('node:fs')
const { tmpdir } = require('node:os')
const { join } = require('node:path')
const { test } = require('node:test')

const environment = 'WINAPPSDK_BOOTSTRAP_DLL_PATH'
const dllName = 'Microsoft.WindowsAppRuntime.Bootstrap.dll'

function fixture(t, label = 'bundle') {
  const root = mkdtempSync(join(tmpdir(), `dynwinrt-${label}-`))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const config = { schema_version: 1, major: 2, minor: 5, dlls: {} }
  for (const arch of ['x64', 'arm64']) {
    const dir = join(root, '_native', 'winappsdk', arch)
    mkdirSync(dir, { recursive: true })
    const content = Buffer.from(`test resource ${arch}`)
    writeFileSync(join(dir, dllName), content)
    config.dlls[arch] = createHash('sha256').update(content).digest('hex')
  }
  return { root, source: join(root, '_winappsdk.cjs'), config }
}

function runtime(t) {
  const module = require.resolve('../runtime/winappsdk.cjs')
  delete require.cache[module]
  const helpers = require(module)
  const previous = process.env[environment]
  delete process.env[environment]
  t.after(() => {
    if (previous === undefined) delete process.env[environment]
    else process.env[environment] = previous
  })
  const calls = []
  const init = helpers.createInitializer((...args) => {
    calls.push(args)
  })
  return { ...helpers, init, calls }
}

test('missing configuration and partial versions fail without initializing', (t) => {
  const { init, calls } = runtime(t)
  assert.throws(() => init(), /No generated Windows App SDK configuration/)
  assert.throws(() => init(2), /both major and minor/)
  assert.throws(() => init(undefined, 5), /both major and minor/)
  assert.deepEqual(calls, [])
})

test('explicit versions retain native/environment behavior despite registered conflicts', (t) => {
  const { init, calls, registerConfig } = runtime(t)
  const first = fixture(t)
  const second = fixture(t)
  registerConfig(first.source, first.config)
  registerConfig(second.source, { ...second.config, minor: 6 })
  process.env[environment] = 'unchanged relative legacy.dll'
  assert.equal(init(2, 5), undefined)
  assert.deepEqual(calls, [[2, 5]])
  assert.equal(process.env[environment], 'unchanged relative legacy.dll')
})

test('registration is inert and consistent providers merge independently of import order', (t) => {
  const { init, calls, registerConfig } = runtime(t)
  const first = fixture(t)
  const second = fixture(t)
  registerConfig(second.source, second.config)
  registerConfig(first.source, first.config)
  registerConfig(first.source, first.config)
  assert.equal(calls.length, 0)
  init()
  const selected = [first, second].sort((a, b) => a.source.localeCompare(b.source))[0]
  assert.deepEqual(calls, [[2, 5, join(selected.root, '_native', 'winappsdk', process.arch, dllName)]])
  assert.equal(process.env[environment], undefined)
})

test('conflicting versions and payloads name both provider sources', (t) => {
  const { init, calls, registerConfig } = runtime(t)
  const first = fixture(t)
  const second = fixture(t)
  registerConfig(first.source, first.config)
  registerConfig(second.source, { ...second.config, dlls: { ...second.config.dlls, x64: '0'.repeat(64) } })
  assert.throws(init, (error) => {
    assert.match(error.message, /Conflicting generated/)
    assert.ok(error.message.includes(first.source) && error.message.includes(second.source))
    return true
  })
  assert.equal(calls.length, 0)
})

test('explicit path wins over environment and environment wins over generated resources', (t) => {
  const { init, calls, registerConfig } = runtime(t)
  const { root, source, config } = fixture(t)
  registerConfig(source, config)
  const fromEnv = join(root, 'env.dll')
  const explicit = join(root, 'explicit.dll')
  writeFileSync(fromEnv, '')
  writeFileSync(explicit, '')
  process.env[environment] = fromEnv
  init()
  init(2, 5, explicit)
  assert.deepEqual(calls, [
    [2, 5, fromEnv],
    [2, 5, explicit],
  ])
  assert.equal(process.env[environment], fromEnv)
  for (const invalid of ['', ' ', 'bad\0path', join(root, 'missing.dll')]) {
    assert.throws(() => init(2, 5, invalid))
  }
  process.env[environment] = ''
  assert.throws(init, /nonempty path/)
  assert.equal(calls.length, 2)
})

test('modified resources fail before initialization', (t) => {
  const { init, calls, registerConfig } = runtime(t)
  const { root, source, config } = fixture(t)
  registerConfig(source, config)
  writeFileSync(join(root, '_native', 'winappsdk', process.arch, dllName), 'changed')
  assert.throws(init, /digest mismatch/)
  assert.equal(calls.length, 0)
})

test('missing generated resources report reinstallation', (t) => {
  const { init, calls, registerConfig } = runtime(t)
  const { root, source, config } = fixture(t)
  registerConfig(source, config)
  rmSync(join(root, '_native', 'winappsdk', process.arch, dllName))
  assert.throws(init, /Reinstall the complete generated package/)
  assert.equal(calls.length, 0)
})

test('invalid schema fields and changed registration fail closed', (t) => {
  const { registerConfig } = runtime(t)
  const { source, config } = fixture(t)
  for (const invalid of [
    { ...config, schema_version: 2 },
    { ...config, major: -1 },
    { ...config, minor: 65536 },
    { ...config, minor: 1.5 },
    { ...config, extra: true },
    { ...config, dlls: {} },
    { ...config, dlls: { x86: '0'.repeat(64) } },
  ]) {
    assert.throws(() => registerConfig(source, invalid), /Invalid Windows App SDK configuration/)
  }
  registerConfig(source, config)
  assert.throws(() => registerConfig(source, { ...config, minor: 6 }), /configuration changed/)
})

test('native failures preserve the original exception and do not poison retries', (t) => {
  const { createInitializer, registerConfig } = runtime(t)
  const { source, config } = fixture(t)
  registerConfig(source, config)
  const error = new Error('native failure')
  let calls = 0
  const init = createInitializer(() => {
    if (++calls === 1) throw error
  })
  assert.throws(init, (caught) => caught === error)
  assert.equal(init(), undefined)
  assert.equal(calls, 2)
})
