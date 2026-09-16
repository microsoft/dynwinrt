// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { statSync } from 'node:fs'
import { resolve } from 'node:path'
import { parseArgs } from 'node:util'
import { describeError, step } from './support.ts'

export interface OcrOptions {
    image?: string
    ensureReady: boolean
}

export type OcrResult = { kind: 'recognized'; lines: string[] } | { kind: 'cancelled' }
export type OcrRunner = (
    options: OcrOptions,
    report: (message: string) => void,
    signal: AbortSignal,
) => Promise<OcrResult>

export const HELP = `Windows AI OCR (Microsoft.Windows.AI.Imaging.TextRecognizer)

From samples\\js\\ocr:
  npm start                            Open the interactive image picker with package identity
  npm start -- --image "C:\\path\\image.png"
  npm start -- --image "C:\\path\\image.png" --ensure-ready
  node main.ts --help

--image PATH     Read this image instead of opening the picker.
--ensure-ready   Explicitly allow Windows to prepare/download the OCR model if not ready.
                 Without this option the sample never requests model installation.
--help, -h       Show this help without loading native bindings or registering identity.

Requires Windows App SDK 1.8, supported NPU hardware, and systemAIModels capability.
See samples\\js\\ocr\\README.md for restore, identity, and offline generation.
Exit codes: 0 recognized (possibly no text) or help; 1 error; 2 no file selected; 130 Ctrl+C.`

export function imagePath(input: string): string {
    if (input.trim().length === 0 || input.includes('\0')) {
        throw new Error('--image requires a nonempty file path.')
    }
    const absolute = resolve(input)
    try {
        if (!statSync(absolute).isFile()) {
            throw new Error('The path is not a regular file.')
        }
    } catch (cause) {
        throw new Error(`Cannot read image "${absolute}". Pass an existing image file with --image.`, { cause })
    }
    return absolute
}

export function parseOptions(args: string[]): OcrOptions | 'help' {
    const { values, tokens } = parseArgs({
        args,
        options: {
            image: { type: 'string' },
            'ensure-ready': { type: 'boolean', default: false },
            help: { type: 'boolean', short: 'h', default: false },
        },
        strict: true,
        allowPositionals: false,
        tokens: true,
    })
    if (tokens.filter((token) => token.kind === 'option' && token.name === 'image').length > 1) {
        throw new Error('Pass --image only once.')
    }
    if (values.help) return 'help'
    return {
        image: values.image === undefined ? undefined : imagePath(values.image),
        ensureReady: values['ensure-ready'],
    }
}

const runNative: OcrRunner = async (options, report, signal) => {
    const { recognize } = await step(
        'Loading generated bindings/runtime. Run npm install, prepare-local.ps1, and npm run restore (see README)',
        () => import('./runtime.ts'),
    )
    return recognize(options, report, signal)
}

export async function runCli(
    args: string[],
    run: OcrRunner = runNative,
    output: Pick<Console, 'log' | 'error'> = console,
): Promise<number> {
    const controller = new AbortController()
    const interrupt = () => controller.abort(new Error('Cancelled by Ctrl+C.'))
    try {
        const options = parseOptions(args)
        if (options === 'help') {
            output.log(HELP)
            return 0
        }
        process.once('SIGINT', interrupt)
        const result = await run(options, (message) => output.log(message), controller.signal)
        if (controller.signal.aborted) {
            output.error('Cancelled by Ctrl+C; OCR did not complete.')
            return 130
        }
        if (result.kind === 'cancelled') {
            output.log('No file selected (picker cancelled or unavailable); OCR was not run.')
            return 2
        }
        output.log('=== Recognized Text ===')
        output.log(result.lines.length === 0 ? 'No text was recognized.' : result.lines.join('\n'))
        return 0
    } catch (error) {
        output.error(`Windows AI OCR failed: ${describeError(error)}`)
        return controller.signal.aborted ? 130 : 1
    } finally {
        process.removeListener('SIGINT', interrupt)
    }
}
