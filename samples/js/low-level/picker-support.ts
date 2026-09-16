// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { statSync } from 'node:fs'

type InitializationApi = Pick<typeof import('@microsoft/dynwinrt'), 'roInitialize' | 'hasPackageIdentity' | 'initWinappsdk'>

export function initializePicker(runtime: InitializationApi, environment: NodeJS.ProcessEnv = process.env): void {
    runtime.roInitialize(1)
    // Full packaged hosts supply Microsoft.WindowsAppRuntime.1.8 through their manifest.
    if (runtime.hasPackageIdentity()) return
    const filename = environment.WINAPPSDK_BOOTSTRAP_DLL_PATH
    if (!filename?.trim()) {
        throw new Error(
            'Set WINAPPSDK_BOOTSTRAP_DLL_PATH to the Windows App SDK 1.8 bootstrap DLL matching Node.js architecture. See the picker setup in README.',
        )
    }
    try {
        if (!statSync(filename).isFile()) throw new Error('The bootstrap path is not a file.')
    } catch (cause) {
        throw new Error(`Windows App SDK bootstrap DLL was not found at "${filename}". See the picker setup in README.`, { cause })
    }
    try {
        runtime.initWinappsdk(1, 8)
    } catch (cause) {
        throw new Error('Windows App SDK 1.8 initialization failed. Install the matching runtime and use the correct architecture bootstrap DLL.', { cause })
    }
}

export function pickerOutcome(path: string | null): { exitCode: 0 | 2; message: string } {
    if (path === null) {
        return { exitCode: 2, message: 'No file selected (picker cancelled or unavailable).' }
    }
    if (path.length === 0) throw new Error('The picker returned an empty file path.')
    return { exitCode: 0, message: `Selected path: ${path}` }
}
