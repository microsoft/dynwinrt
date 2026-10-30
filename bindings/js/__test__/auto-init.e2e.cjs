// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

'use strict'

const assert = require('node:assert/strict')
const { spawnSync } = require('node:child_process')
const { mkdtempSync, writeFileSync, rmSync } = require('node:fs')
const { tmpdir } = require('node:os')
const { join, resolve } = require('node:path')
const { before, after, test } = require('node:test')

const source = resolve(__dirname, '..')
const consumer = mkdtempSync(join(tmpdir(), 'dynwinrt-auto-init-'))
const packageRoot = join(consumer, 'node_modules', '@microsoft', 'dynwinrt')

function run(args, cwd = consumer, env = process.env) {
  const child = spawnSync(process.execPath, args, { cwd, env, encoding: 'utf8', timeout: 120_000 })
  assert.equal(child.status, 0, `${child.error ?? ''}\n${child.stdout}\n${child.stderr}`)
  return child.stdout
}

before(() => {
  assert.ok(process.env.npm_execpath, 'Run via npm run test:winappsdk to test the packed package')
  const packed = JSON.parse(
    run([process.env.npm_execpath, 'pack', '--json', '--ignore-scripts', '--pack-destination', consumer], source),
  )
  const files = packed[0].files.map((file) => file.path)
  assert.ok(files.includes('dist/auto-init.js'))
  assert.ok(files.includes('dist/auto-init.d.ts'))
  writeFileSync(join(consumer, 'package.json'), '{"private":true}')
  run([
    process.env.npm_execpath,
    'install',
    '--ignore-scripts',
    '--no-audit',
    '--no-fund',
    '--package-lock=false',
    '--offline',
    join(consumer, packed[0].filename),
  ])
})

after(() => rmSync(consumer, { recursive: true, force: true }))

async function exercise(scenario, mode) {
  const assert = require('node:assert/strict')
  const { createHash } = require('node:crypto')
  const { mkdirSync, writeFileSync } = require('node:fs')
  const { join, basename } = require('node:path')
  const environment = 'WINAPPSDK_BOOTSTRAP_DLL_PATH'
  delete process.env[environment]
  const native = require('./node_modules/@microsoft/dynwinrt/dist/index.js')
  const calls = []
  const failure = new Error('original native failure')
  const realNative = ['passive', 'bad-dll'].includes(scenario)
  if (!realNative) {
    native.initWinappsdk = (...args) => {
      calls.push(args)
      if (scenario === 'native-error') throw failure
    }
  }
  const runtime = require('@microsoft/dynwinrt')
  const { DynWin32, DynWin32Function } = require('@microsoft/dynwinrt/win32/unsafe')
  const apartmentType = DynWin32Function.bind({
    dll: 'ole32.dll',
    entryPoint: 'CoGetApartmentType',
    parameters: [
      { type: 'i32', direction: 'out' },
      { type: 'i32', direction: 'out' },
    ],
    returnType: 'i32',
  })
  const apartmentStatus = () => DynWin32.toNumber(apartmentType.invoke([]).returnValue)
  assert.equal(apartmentStatus(), -2147221008) // CO_E_NOTINITIALIZED
  const esmRuntime = await import('@microsoft/dynwinrt')
  assert.equal(runtime.initWinappsdk, esmRuntime.initWinappsdk)
  const { __registerWinAppSdkConfig: register } = await import('@microsoft/dynwinrt/winappsdk')
  assert.deepEqual(calls, [])
  if ('excludeNetwork' in process.report) process.report.excludeNetwork = true
  const bootstrapLoaded = () =>
    process.report
      .getReport()
      .sharedObjects.some((path) => basename(path).toLowerCase() === 'microsoft.windowsappruntime.bootstrap.dll')
  assert.equal(bootstrapLoaded(), false)
  assert.equal(typeof runtime.hasPackageIdentity(), 'boolean')
  assert.equal(apartmentStatus(), -2147221008)
  if (scenario === 'passive') return

  const identity = scenario.startsWith('identity')
  let probes = 0
  runtime.hasPackageIdentity = () => {
    probes++
    if (scenario === 'probe-error') throw failure
    return identity
  }
  const dll = join(process.cwd(), '_native', 'winappsdk', process.arch, 'Microsoft.WindowsAppRuntime.Bootstrap.dll')
  const bytes = Buffer.from('not a native library')
  mkdirSync(join(dll, '..'), { recursive: true })
  writeFileSync(dll, bytes)
  const config = {
    schema_version: 1,
    major: 2,
    minor: 5,
    dlls: { [process.arch]: createHash('sha256').update(bytes).digest('hex') },
  }
  if (!['missing', 'identity', 'probe-error'].includes(scenario)) {
    register(join(process.cwd(), '_winappsdk.cjs'), config)
  }
  if (['conflict', 'identity-conflict'].includes(scenario)) {
    register(join(process.cwd(), 'second.cjs'), { ...config, minor: 6 })
  }
  if (identity || scenario === 'path') process.env[environment] = join(process.cwd(), 'missing.dll')
  assert.deepEqual(calls, []) // Generated registration remains passive.

  const load = async () =>
    mode === 'cjs' ? require('@microsoft/dynwinrt/auto-init') : import('@microsoft/dynwinrt/auto-init')
  let nativeFailure
  if (scenario === 'bad-dll') {
    assert.throws(
      () => native.initWinappsdk(2, 5, dll),
      (error) => {
        nativeFailure = error
        return true
      },
    )
  }
  if (['missing', 'conflict', 'path', 'bad-dll', 'native-error', 'probe-error'].includes(scenario)) {
    const expected = {
      missing: /No generated Windows App SDK configuration/,
      conflict: /Conflicting generated Windows App SDK configurations/,
      path: /ENOENT/,
      'bad-dll': (error) => error.message === nativeFailure.message,
      'native-error': (error) => error === failure,
      'probe-error': (error) => error === failure,
    }[scenario]
    await assert.rejects(load, expected)
    assert.equal(calls.length, scenario === 'native-error' ? 1 : 0)
    assert.equal(bootstrapLoaded(), false)
    return
  }
  await load()
  await load()
  const commonjs = require('@microsoft/dynwinrt/auto-init')
  const esm = await import('@microsoft/dynwinrt/auto-init')
  assert.equal(esm.default, commonjs)
  assert.deepEqual(Object.keys(commonjs), [])
  assert.equal(probes, 1)
  assert.equal(bootstrapLoaded(), false)
  if (identity) {
    assert.deepEqual(calls, [])
    assert.throws(() => runtime.initWinappsdk(), /No generated|Conflicting|ENOENT/)
  } else {
    assert.deepEqual(calls, [[2, 5, dll]])
    runtime.initWinappsdk()
    assert.equal(calls.length, 2)
  }
  runtime.initWinappsdk(2, 5)
  runtime.initWinappsdk(2, 5, dll)
  assert.deepEqual(calls.slice(-2), [
    [2, 5],
    [2, 5, dll],
  ])
  assert.equal(probes, 1) // Explicit initialization never queries identity.
}

for (const mode of ['cjs', 'esm']) {
  for (const scenario of [
    'passive',
    'identity',
    'identity-conflict',
    'initialize',
    'missing',
    'conflict',
    'path',
    'bad-dll',
    'native-error',
    'probe-error',
  ]) {
    test(`installed ${mode} auto-init: ${scenario}`, () => {
      run([
        '--eval',
        `(${exercise.toString()})(${JSON.stringify(scenario)}, ${JSON.stringify(mode)})
        .catch(error => { console.error(error); process.exitCode = 1 })`,
      ])
    })
  }
}

test('packed auto-init has no public API and resolves in CJS and ESM TypeScript', () => {
  const manifest = require(join(packageRoot, 'package.json'))
  assert.equal(manifest.sideEffects, true)
  assert.equal(manifest.exports['./auto-init'].require, manifest.exports['./auto-init'].import)
  for (const extension of ['cts', 'mts']) {
    writeFileSync(
      join(consumer, `consumer.${extension}`),
      `
import '@microsoft/dynwinrt/auto-init'
type AutoInit = typeof import('@microsoft/dynwinrt/auto-init')
const noExports: keyof AutoInit extends never ? true : never = true
void noExports
`,
    )
  }
  run([
    join(source, 'node_modules', 'typescript', 'bin', 'tsc'),
    '--noEmit',
    '--strict',
    '--module',
    'NodeNext',
    '--moduleResolution',
    'NodeNext',
    '--noUncheckedSideEffectImports',
    'consumer.cts',
    'consumer.mts',
  ])
})
