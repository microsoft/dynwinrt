// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { runExample } from '../example.ts'

function output() {
    const messages: unknown[] = []
    const errors: unknown[] = []
    return { messages, errors, log: (value: unknown) => messages.push(value), error: (value: unknown) => errors.push(value) }
}

test('values retain their type and release in reverse order before success is printed', async () => {
    const calls: string[] = []
    const sink = output()
    const first = { value: 42, release: () => calls.push('first') }
    const second = { release: () => calls.push('second') }
    const code = await runExample('owned values', async ({ own, defer }) => {
        defer('temporary directory', () => calls.push('directory'))
        assert.equal(own(first).value, 42)
        assert.equal(own(second), second)
        await Promise.resolve()
        assert.deepEqual(sink.messages, [])
        return 'completed'
    }, sink)
    assert.equal(code, 0)
    assert.deepEqual(calls, ['second', 'first', 'directory'])
    assert.deepEqual(sink.messages, ['completed'])
    assert.deepEqual(sink.errors, [])
})

test('an operation failure cannot print success and still releases every resource', async () => {
    const calls: string[] = []
    const sink = output()
    const failure = new Error('owned async failure')
    const code = await runExample('failing operation', async ({ own }) => {
        own({ release: () => calls.push('released') })
        await Promise.resolve()
        throw failure
    }, sink)
    assert.equal(code, 1)
    assert.deepEqual(calls, ['released'])
    assert.deepEqual(sink.messages, [])
    assert.ok(sink.errors[0] instanceof AggregateError)
    assert.deepEqual(sink.errors[0].errors, [failure])
})

test('cleanup failure remains nonzero, preserves the primary error, and does not stop other cleanup', async () => {
    const sink = output()
    let released = false
    const primary = new Error('operation failed')
    const cleanup = new Error('close failed')
    const code = await runExample('failure cleanup', ({ own, defer }) => {
        own({ release: () => { released = true } })
        defer('file stream', () => { throw cleanup })
        throw primary
    }, sink)
    assert.equal(code, 1)
    assert.equal(released, true)
    assert.deepEqual(sink.messages, [])
    assert.ok(sink.errors[0] instanceof AggregateError)
    assert.equal(sink.errors[0].errors[0], primary)
    assert.equal(sink.errors[0].errors[1].cause, cleanup)
})

test('cleanup-only failures do not report a successful result', async () => {
    const sink = output()
    const code = await runExample('cleanup-only', ({ defer }) => {
        defer('owned resource', () => { throw new Error('cleanup failed') })
        return 'must not print this result'
    }, sink)
    assert.equal(code, 1)
    assert.deepEqual(sink.messages, [])
    assert.equal(sink.errors.length, 1)
})

test('entry-level failures propagate as nonzero process exits', () => {
    const module = new URL('../example.ts', import.meta.url).href
    const guard = new URL('no-native.mjs', import.meta.url).href
    const code = `import { runExample } from ${JSON.stringify(module)};
        process.exitCode = await runExample('child', () => { throw new Error('owned child failure'); });`
    const child = spawnSync(process.execPath, ['--import', guard, '--input-type=module', '--eval', code], {
        encoding: 'utf8',
        timeout: 10000,
    })
    assert.ifError(child.error)
    assert.equal(child.status, 1)
    assert.match(child.stderr, /owned child failure/)
})

for (const name of ['uri', 'async-file', 'struct-geopoint', 'array-property-value']) {
    test(`${name} uses only the WinRT domain and propagates native import failures`, () => {
        const source = readFileSync(new URL(`../${name}.ts`, import.meta.url), 'utf8')
        assert.match(source, /roInitialize\(1\)/)
        assert.match(source, /process\.exitCode = await runExample/)
        assert.doesNotMatch(source, /generated[/\\]|initWinappsdk|@microsoft\/dynwinrt\/(?:com|win32)/)
        const child = spawnSync(process.execPath, [
            '--import', new URL('no-native.mjs', import.meta.url).href,
            fileURLToPath(new URL(`../${name}.ts`, import.meta.url)),
        ], { encoding: 'utf8', timeout: 10000 })
        assert.ifError(child.error)
        assert.equal(child.status, 1)
        assert.match(child.stderr, /Native addons are forbidden/)
    })
}
