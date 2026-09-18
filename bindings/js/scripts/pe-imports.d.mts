// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

export interface PeImport {
  dll: string
  symbols: (string | number)[]
}

export function readPeImports(bytes: Buffer): PeImport[]
export function assertNoEagerWin32Imports(imports: PeImport[], options?: { testHooks?: boolean }): void
export function verifyAddonImports(directory: string, options?: { testHooksAddon?: string }): string[]
