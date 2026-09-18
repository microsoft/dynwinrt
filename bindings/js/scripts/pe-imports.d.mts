// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

export interface PeImport {
  dll: string
  symbols: (string | number)[]
}

export function readPeImports(bytes: Buffer, options?: { includeDelayImports?: boolean }): PeImport[]
export function assertNoEagerWin32Imports(imports: PeImport[], options?: { testHooks?: boolean }): void
export function assertNoDispatcherQueueImports(imports: PeImport[]): void
export function verifyAddonImports(directory: string, options?: { testHooksAddon?: string }): string[]
export function verifyDispatcherQueueImports(directory: string): string[]
