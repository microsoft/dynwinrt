// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { readFileSync } from 'node:fs'
import { extname } from 'node:path'
import {
  assertNoDispatcherQueueImports,
  assertNoEagerWin32Imports,
  assertNoUiHelperImports,
  readPeImports,
} from './pe-imports.mjs'

const files = process.argv.slice(2)
if (!files.length) throw new Error('Usage: node check-ui-helper-imports.mjs <production.node|production.pyd> [...]')
for (const file of files) {
  if (!['.node', '.pyd'].includes(extname(file).toLowerCase())) {
    throw new Error(`Expected a production .node or .pyd binary: ${file}`)
  }
  const imports = readPeImports(readFileSync(file), { includeDelayImports: true })
  assertNoUiHelperImports(imports)
  assertNoDispatcherQueueImports(imports)
  assertNoEagerWin32Imports(imports)
  console.log(`Verified production ordinary and delay native imports: ${file}`)
}
