// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { spawn } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { constants, copyFileSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { HELP, parseOptions } from './cli.ts'
import type { OcrOptions } from './cli.ts'
import { describeError } from './support.ts'

const ROOT = fileURLToPath(new URL('.', import.meta.url))

export function renderManifest(template: string, name: string, architecture: string): string {
    if (!/^dynwinrt-ocr-[0-9a-f-]{36}$/.test(name)) throw new Error('Invalid private OCR package name.')
    if (architecture !== 'arm64' && architecture !== 'x64') throw new Error(`Unsupported Node.js architecture: ${architecture}`)
    if (!template.includes('__PACKAGE_NAME__') || !template.includes('__ARCHITECTURE__')) {
        throw new Error('The OCR package manifest template is missing its identity/architecture placeholders.')
    }
    return template.replaceAll('__PACKAGE_NAME__', name).replaceAll('__ARCHITECTURE__', architecture)
}

export function launchArguments(host: string, options: OcrOptions): string[] {
    return [
        'run', host,
        '--manifest', join(host, 'Package.appxmanifest'),
        '--exe', 'dynwinrt-ocr-node.exe',
        '--with-alias',
        '--unregister-on-exit',
        '--', fileURLToPath(new URL('main.ts', import.meta.url)),
        ...(options.image === undefined ? [] : ['--image', options.image]),
        ...(options.ensureReady ? ['--ensure-ready'] : []),
    ]
}

export function runProcess(executable: string, args: string[], cwd: string): Promise<number> {
    return new Promise((resolveExit, reject) => {
        const child = spawn(executable, args, { cwd, stdio: 'inherit', shell: false })
        child.once('error', reject)
        child.once('close', (code, signal) => {
            if (signal !== null) {
                reject(new Error(`The launcher process was terminated by ${signal}.`))
            } else {
                resolveExit(code ?? 1)
            }
        })
    })
}

export async function prepareBindings(
    useExistingBindings: boolean,
    generate: () => Promise<number>,
    resolveApi: (specifier: string) => unknown,
    report: (message: string) => void,
): Promise<void> {
    if (!useExistingBindings) {
        const generated = await generate()
        if (generated !== 0) {
            throw new Error(
                `WinApp CLI binding generation failed (exit ${generated}). Run npm run restore successfully before npm start. ` +
                'For explicitly generated cached metadata, follow the separate start:generated instructions in README.',
            )
        }
    } else {
        report('Using explicitly pre-generated bindings; this does not validate WinApp CLI SDK restore.')
        for (const api of [
            'windows/storage/StorageFile',
            'windows/graphics/imaging/BitmapDecoder',
            'microsoft/windows/storage/pickers/FileOpenPicker',
            'microsoft/graphics/imaging/ImageBuffer',
            'microsoft/windows/ai/imaging/TextRecognizer',
        ]) {
            resolveApi(`#winapp/bindings/${api}`)
        }
    }
}

async function launch(options: OcrOptions, useExistingBindings = false): Promise<number> {
    if (process.platform !== 'win32') throw new Error('Windows AI OCR requires Windows.')
    const require = createRequire(import.meta.url)
    const cli = require.resolve('@microsoft/winappcli/dist/cli.js')
    const runWinapp = (args: string[], cwd = ROOT) => runProcess(process.execPath, [cli, ...args], cwd)
    await prepareBindings(useExistingBindings, () => runWinapp(['node', 'generate-bindings']), require.resolve, console.log)
    const name = `dynwinrt-ocr-${randomUUID()}`
    const host = join(ROOT, '.winapp', name)
    mkdirSync(join(ROOT, '.winapp'), { recursive: true })
    mkdirSync(host)
    let exitCode: number | undefined
    let registrationAttempted = false
    try {
        const executable = join(host, 'dynwinrt-ocr-node.exe')
        copyFileSync(process.execPath, executable, constants.COPYFILE_EXCL)
        const generated = await runWinapp([
            'manifest', 'generate', host,
            '--package-name', name,
            '--publisher-name', 'CN=dynwinrtOcrSample',
            '--executable', executable,
            '--if-exists', 'Error',
        ], host)
        if (generated !== 0) throw new Error(`WinApp CLI manifest/assets generation failed (exit ${generated}).`)
        const manifest = renderManifest(readFileSync(join(ROOT, 'Package.appxmanifest.in'), 'utf8'), name, process.arch)
        writeFileSync(join(host, 'Package.appxmanifest'), manifest)
        console.log(`Launching private package ${name} with systemAIModels; the installed node.exe is unchanged.`)
        registrationAttempted = true
        exitCode = await runWinapp(launchArguments(host, options))
        return exitCode
    } finally {
        if (!registrationAttempted || exitCode === 0) {
            rmSync(host, { recursive: true })
        } else {
            console.error(
                `Private launch files retained at "${host}". WinApp CLI was asked to unregister on exit. ` +
                'If registration/launch was interrupted, use the scoped cleanup command in README before removing this directory.',
            )
        }
    }
}

export const launchWithGeneratedBindings = (options: OcrOptions) => launch(options, true)

export async function runLauncher(
    args: string[],
    start: (options: OcrOptions) => Promise<number> = launch,
    output: Pick<Console, 'log' | 'error'> = console,
): Promise<number> {
    try {
        const options = parseOptions(args)
        if (options === 'help') {
            output.log(HELP)
            return 0
        }
        return await start(options)
    } catch (error) {
        output.error(`Windows AI OCR launch failed: ${describeError(error)}`)
        return 1
    }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    process.exitCode = await runLauncher(process.argv.slice(2))
}
