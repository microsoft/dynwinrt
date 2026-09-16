// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { statSync } from 'node:fs'
import { join } from 'node:path'

export function describeError(error: unknown): string {
    if (error instanceof AggregateError) {
        return `${error.message}\n${error.errors.map(describeError).join('\n')}`
    }
    if (error instanceof Error) {
        return error.cause === undefined ? error.message : `${error.message}\n${describeError(error.cause)}`
    }
    return String(error)
}

export async function step<T>(label: string, action: () => T | PromiseLike<T>): Promise<T> {
    try {
        return await action()
    } catch (cause) {
        throw new Error(label, { cause })
    }
}

export function bootstrapPath(root: string, architecture: string, override?: string): string {
    if (architecture !== 'arm64' && architecture !== 'x64') {
        throw new Error(`Unsupported Node.js architecture: ${architecture}. Use native ARM64 or x64 Node.js.`)
    }
    const filename = override ?? join(root, '.winapp', 'bin', architecture, 'Microsoft.WindowsAppRuntime.Bootstrap.dll')
    try {
        if (!statSync(filename).isFile()) throw new Error('The bootstrap path is not a file.')
    } catch (cause) {
        throw new Error(
            `Windows App SDK 1.8 bootstrap DLL was not found at "${filename}". ` +
            'Run npm run restore, or set WINAPPSDK_BOOTSTRAP_DLL_PATH to the matching architecture DLL (see README).',
            { cause },
        )
    }
    return filename
}

type Defer = (label: string, action: () => void) => void

export async function withCleanup<T>(body: (defer: Defer) => Promise<T>): Promise<T> {
    const cleanup: { label: string; action: () => void }[] = []
    let outcome: { value: T } | { error: unknown }
    try {
        outcome = { value: await body((label, action) => cleanup.push({ label, action })) }
    } catch (error) {
        outcome = { error }
    }
    const errors: unknown[] = 'error' in outcome ? [outcome.error] : []
    for (const { label, action } of cleanup.reverse()) {
        try {
            action()
        } catch (cause) {
            errors.push(new Error(`Releasing ${label} failed`, { cause }))
        }
    }
    if (errors.length > 1) throw new AggregateError(errors, 'OCR operation/cleanup failed.')
    if (errors.length === 1) throw errors[0]
    if ('error' in outcome) throw outcome.error
    return outcome.value
}
