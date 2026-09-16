// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { initializePicker, pickerOutcome } from '../picker-support.ts'

test('unpackaged initialization selects MTA before Windows App SDK 1.8 bootstrap', (t) => {
    const directory = mkdtempSync(join(tmpdir(), 'dynwinrt-picker-init-'))
    t.after(() => rmSync(directory, { recursive: true }))
    const filename = join(directory, 'owned-bootstrap.dll')
    writeFileSync(filename, 'path-only fixture; no DLL is loaded')
    const calls: string[] = []
    initializePicker({
        roInitialize: (mode) => { calls.push(`apartment:${mode}`) },
        hasPackageIdentity: () => { calls.push('identity'); return false },
        initWinappsdk: (major, minor) => { calls.push(`bootstrap:${major}.${minor}`) },
    }, { WINAPPSDK_BOOTSTRAP_DLL_PATH: filename })
    assert.deepEqual(calls, ['apartment:1', 'identity', 'bootstrap:1.8'])
})

test('packaged initialization uses its static framework dependency instead of bootstrap', () => {
    const calls: string[] = []
    initializePicker({
        roInitialize: (mode) => { calls.push(`apartment:${mode}`) },
        hasPackageIdentity: () => true,
        initWinappsdk: () => { assert.fail('Full packaged hosts must not call MddBootstrapInitialize2') },
    }, {})
    assert.deepEqual(calls, ['apartment:1'])
})

test('missing or invalid bootstrap input fails explicitly before calling the bootstrap API', (t) => {
    const directory = mkdtempSync(join(tmpdir(), 'dynwinrt-picker-missing-'))
    t.after(() => rmSync(directory, { recursive: true }))
    const runtime = {
        roInitialize: () => {},
        hasPackageIdentity: () => false,
        initWinappsdk: () => { assert.fail('Invalid bootstrap setup must not call native code') },
    }
    for (const path of [undefined, '', ' ', join(directory, 'missing.dll'), directory]) {
        assert.throws(() => initializePicker(runtime, { WINAPPSDK_BOOTSTRAP_DLL_PATH: path }), /bootstrap|BOOTSTRAP/)
    }
})

test('bootstrap errors preserve their original cause and include actionable setup guidance', (t) => {
    const directory = mkdtempSync(join(tmpdir(), 'dynwinrt-picker-failure-'))
    t.after(() => rmSync(directory, { recursive: true }))
    const filename = join(directory, 'owned-bootstrap.dll')
    writeFileSync(filename, 'path-only fixture')
    const failure = new Error('owned HRESULT failure')
    assert.throws(() => initializePicker({
        roInitialize: () => {},
        hasPackageIdentity: () => false,
        initWinappsdk: () => { throw failure },
    }, { WINAPPSDK_BOOTSTRAP_DLL_PATH: filename }), (error: unknown) => {
        assert.ok(error instanceof Error)
        assert.equal(error.cause, failure)
        assert.match(error.message, /Install the matching runtime/)
        return true
    })
})

test('a null picker result is exit 2 rather than a fake selected file', () => {
    const result = pickerOutcome(null)
    assert.equal(result.exitCode, 2)
    assert.match(result.message, /No file selected/)
    assert.doesNotMatch(result.message, /Selected path:/)
})

test('selected paths preserve spaces and Unicode; an empty non-null path fails', () => {
    const path = 'C:\\owned input\\\u6587\u4ef6.txt'
    assert.deepEqual(pickerOutcome(path), { exitCode: 0, message: `Selected path: ${path}` })
    assert.throws(() => pickerOutcome(''), /empty file path/)
})

test('the picker uses corrected typed contracts and the shared resource/error handling', () => {
    const source = readFileSync(new URL('../picker.ts', import.meta.url), 'utf8')
    const contracts = readFileSync(new URL('../contracts.ts', import.meta.url), 'utf8')
    assert.match(source, /from '@microsoft\/dynwinrt'/)
    assert.match(source, /await runExample\('picker'/)
    assert.match(source, /selected\.isNull\(\)/)
    assert.match(source, /methodByName\('get_Path'\)/)
    assert.match(source, /if \(process\.exitCode === 0\) process\.exitCode = resultExitCode/)
    assert.match(contracts, /structType\('Microsoft\.UI\.WindowId', \[Type\.u64\(\)\]\)/)
    assert.match(contracts, /9d00f175-c783-51bd-8c93-fb63695d3abc/)
    assert.doesNotMatch(source + contracts, /2C3D04E9|CreateWithMode|get_File['"]/i)
    assert.doesNotMatch(source, /bindings\/js\/dist|@microsoft\/dynwinrt\/(?:com|win32)/)
})

test('native import failures produce a nonzero picker process exit without loading the addon', () => {
    const child = spawnSync(process.execPath, [
        '--import', new URL('no-native.mjs', import.meta.url).href,
        fileURLToPath(new URL('../picker.ts', import.meta.url)), '--no-ui',
    ], { encoding: 'utf8', timeout: 10000 })
    assert.ifError(child.error)
    assert.equal(child.status, 1)
    assert.match(child.stderr, /Native addons are forbidden/)
})
