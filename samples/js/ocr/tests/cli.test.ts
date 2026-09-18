// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { HELP, parseOptions, runCli } from '../cli.ts'
import { launchArguments, prepareBindings, renderManifest, runLauncher, runProcess } from '../launch.ts'
import { bootstrapPath } from '../support.ts'

const root = fileURLToPath(new URL('..', import.meta.url))
const entry = resolve(root, 'main.ts')
const guard = new URL('no-native.mjs', import.meta.url).href

function output() {
    const messages: string[] = []
    return { messages, log: (value: string) => messages.push(value), error: (value: string) => messages.push(value) }
}

test('defaults retain the interactive picker without model preparation', () => {
    assert.deepEqual(parseOptions([]), { image: undefined, ensureReady: false })
    assert.equal(parseOptions(['--help']), 'help')
    assert.equal(parseOptions(['-h']), 'help')
})

test('image input is absolute, preserves spaces, and preparation is opt-in', (t) => {
    const directory = mkdtempSync(join(tmpdir(), 'dynwinrt-ocr-args-'))
    t.after(() => rmSync(directory, { recursive: true }))
    const filename = join(directory, 'owned image.png')
    writeFileSync(filename, 'owned fixture; argument parsing never decodes it')
    assert.deepEqual(parseOptions(['--image', filename]), { image: filename, ensureReady: false })
    assert.deepEqual(parseOptions([`--image=${filename}`, '--ensure-ready']), { image: filename, ensureReady: true })
    assert.throws(() => parseOptions(['--image', filename, '--image', filename]), /only once/)
    assert.throws(() => parseOptions(['--image', directory]), /not a regular file|Cannot read image/)
})

for (const args of [
    ['--unknown'],
    ['image.png'],
    ['--image'],
    ['--image', ''],
    ['--image', 'invalid\0name.png'],
    ['--image', 'this-owned-fixture-does-not-exist.png'],
    ['--ensure-ready=false'],
]) {
    test(`invalid arguments fail before native loading or identity registration: ${JSON.stringify(args)}`, async () => {
        const never = async () => { assert.fail('execution must not start') }
        assert.equal(await runCli(args, never, output()), 1)
        assert.equal(await runLauncher(args, never, output()), 1)
    })
}

test('help does not execute or register a package', async () => {
    const never = async () => { assert.fail('execution must not start') }
    const sink = output()
    assert.equal(await runCli(['--help'], never, sink), 0)
    assert.equal(await runLauncher(['--help'], never, sink), 0)
    assert.deepEqual(sink.messages, [HELP, HELP])
})

test('success prints recognized lines and empty inference results honestly', async () => {
    const sink = output()
    assert.equal(await runCli([], async () => ({ kind: 'recognized', lines: ['First', 'Second'] }), sink), 0)
    assert.deepEqual(sink.messages, ['=== Recognized Text ===', 'First\nSecond'])
    const empty = output()
    assert.equal(await runCli([], async () => ({ kind: 'recognized', lines: [] }), empty), 0)
    assert.match(empty.messages.join('\n'), /No text was recognized/)
})

test('picker cancellation is nonzero and never reports OCR success', async () => {
    const sink = output()
    assert.equal(await runCli([], async () => ({ kind: 'cancelled' }), sink), 2)
    assert.match(sink.messages.join('\n'), /OCR was not run/)
    assert.doesNotMatch(sink.messages.join('\n'), /=== Recognized Text ===/)
})

for (const stage of ['bootstrap', 'apartment', 'capability', 'readiness', 'decode', 'recognition', 'cleanup']) {
    test(`${stage} errors retain their cause and produce exit 1`, async () => {
        const sink = output()
        const run = async () => { throw new Error(`${stage} failed`, { cause: new Error('HRESULT 0x80070005') }) }
        assert.equal(await runCli([], run, sink), 1)
        assert.match(sink.messages.join('\n'), new RegExp(`${stage} failed`))
        assert.match(sink.messages.join('\n'), /0x80070005/)
        assert.doesNotMatch(sink.messages.join('\n'), /=== Recognized Text ===/)
    })
}

test('Ctrl+C requests cancellation and removes its process listener', async () => {
    const before = process.listenerCount('SIGINT')
    const code = await runCli([], async (_options, _report, signal) => {
        process.emit('SIGINT')
        signal.throwIfAborted()
        assert.fail('the abort must throw')
    }, output())
    assert.equal(code, 130)
    assert.equal(process.listenerCount('SIGINT'), before)
})

test('the real standalone entry returns process-level failure without loading an addon', () => {
    for (const [args, code, message] of [
        [['--help'], 0, /Windows AI OCR/],
        [['--unknown'], 1, /Unknown option/],
        [[], 1, /Native addons are forbidden/],
    ] as const) {
        const child = spawnSync(process.execPath, ['--import', guard, entry, ...args], { encoding: 'utf8', timeout: 15000 })
        assert.ifError(child.error)
        assert.equal(child.signal, null)
        assert.equal(child.status, code, `${child.stdout}\n${child.stderr}`)
        assert.match(child.stdout + child.stderr, message)
    }
})

test('bootstrap selection checks architecture, overrides, and missing files without initialization', (t) => {
    const directory = mkdtempSync(join(tmpdir(), 'dynwinrt-ocr-bootstrap-'))
    t.after(() => rmSync(directory, { recursive: true }))
    const dll = join(directory, 'owned-bootstrap.dll')
    writeFileSync(dll, 'not a DLL; this is a path-only test')
    assert.equal(bootstrapPath(directory, 'arm64', dll), dll)
    assert.equal(bootstrapPath(directory, 'x64', dll), dll)
    assert.throws(() => bootstrapPath(directory, 'ia32', dll), /Unsupported Node.js architecture/)
    assert.throws(() => bootstrapPath(directory, 'arm64'), /npm run restore/)
})

test('private manifest retains capability, console alias, and integrity protection for both architectures', () => {
    assert.equal(existsSync(join(root, 'Package.appxmanifest')), false, 'WinApp CLI must not auto-discover the template as a package')
    const template = readFileSync(join(root, 'Package.appxmanifest.in'), 'utf8')
    const name = 'dynwinrt-ocr-12345678-1234-1234-1234-123456789abc'
    for (const architecture of ['arm64', 'x64']) {
        const manifest = renderManifest(template, name, architecture)
        assert.match(manifest, /systemai:Capability Name="systemAIModels"/)
        assert.match(manifest, /manifest\/systemai\/windows10/)
        assert.match(manifest, /MaxVersionTested="10\.0\.26226\.0"/)
        assert.match(manifest, /Name="Microsoft\.WindowsAppRuntime\.1\.8"/)
        assert.match(manifest, /MinVersion="8000\.675\.1142\.0"/)
        assert.match(manifest, /Content Enforcement="on"/)
        assert.match(manifest, new RegExp(`ProcessorArchitecture="${architecture}"`))
        assert.match(manifest, new RegExp(`Alias="${name}\\.exe"`))
        assert.doesNotMatch(manifest, /__PACKAGE_NAME__|__ARCHITECTURE__/)
    }
    assert.throws(() => renderManifest(template, '<unsafe>', 'arm64'), /Invalid private/)
    assert.throws(() => renderManifest(template, name, 'ia32'), /Unsupported/)
})

test('launch arguments use the in-directory entry and preserve paths, console output, and cleanup', () => {
    const host = resolve(root, '.winapp', 'owned host')
    const image = resolve(root, 'owned image.png')
    const args = launchArguments(host, { image, ensureReady: false })
    assert.equal(existsSync(entry), true)
    assert.equal(existsSync(resolve(root, '..', 'ocr.ts')), false, 'The outer entry must not be retained')
    assert.equal(args[1], host)
    assert.ok(args.includes('--with-alias'))
    assert.ok(args.includes('--unregister-on-exit'))
    assert.deepEqual(args.slice(args.indexOf('--') + 1), [entry, '--image', image])
    assert.ok(!args.includes('--ensure-ready'))
    assert.equal(launchArguments(host, { ensureReady: true }).at(-1), '--ensure-ready')
    assert.ok(!args.includes('--force'))
})

test('the launcher preserves child failure and cancellation statuses', async () => {
    for (const code of [0, 1, 2, 130]) {
        assert.equal(await runLauncher([], async () => code, output()), code)
    }
    assert.equal(await runLauncher([], async () => { throw new Error('registration failed') }, output()), 1)
    assert.equal(await runProcess(process.execPath, ['-e', 'process.exitCode = 17'], root), 17)
})

test('a failed restore/generation never silently falls back to existing bindings', async () => {
    const never = () => { assert.fail('existing bindings must not be used implicitly') }
    await assert.rejects(prepareBindings(false, async () => 1, never, never), /Run npm run restore successfully/)
    await prepareBindings(false, async () => 0, never, never)
})

test('pre-generated bindings require an explicit path and a visible restore limitation', async () => {
    const modules: string[] = []
    const messages: string[] = []
    await prepareBindings(
        true,
        async () => { assert.fail('this explicit path does not run normal restore/generation') },
        (specifier) => modules.push(specifier),
        (message) => messages.push(message),
    )
    assert.equal(modules.length, 5)
    assert.match(messages[0], /does not validate WinApp CLI SDK restore/)
    await assert.rejects(prepareBindings(true, async () => 0, () => {
        throw new Error('generated module missing')
    }, () => {}), /generated module missing/)
})
