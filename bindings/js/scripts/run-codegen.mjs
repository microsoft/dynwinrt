// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { spawnSync } from 'node:child_process'
import { statSync } from 'node:fs'
import { resolve } from 'node:path'

/**
 * An explicit artifact path must never fall back to a local Cargo build.
 * @param {string[]} args
 * @param {import('node:child_process').SpawnSyncOptionsWithStringEncoding} options
 */
export function runCodegen(args, options) {
  const codegen = (options.env ?? process.env).DYNWINRT_CODEGEN
  if (codegen !== undefined) {
    if (!codegen.trim()) throw new Error('DYNWINRT_CODEGEN must name a prebuilt dynwinrt-codegen executable')
    const executable = resolve(options.cwd?.toString() ?? process.cwd(), codegen)
    if (!statSync(executable).isFile()) throw new Error(`Prebuilt dynwinrt-codegen is not a file: ${executable}`)
    const result = spawnSync(executable, args, options)
    if (result.error) throw result.error
    return result
  }
  return spawnSync('cargo', ['run', '--quiet', '-p', 'dynwinrt-codegen', '--', ...args], options)
}
