// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { readFileSync, realpathSync, statSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

if (process.argv.length !== 3) {
  throw new Error('Usage: node verify-runtime-artifact.mjs <downloaded-artifact-directory>')
}

const artifact = realpathSync(process.argv[2])
const source = dirname(fileURLToPath(import.meta.url))
const manifest = JSON.parse(readFileSync(join(source, '..', 'package.json'), 'utf8'))
const require = createRequire(join(artifact, 'artifact-consumer.cjs'))
const modules = new Map()

function artifactFile(target) {
  assert.ok(target.startsWith('./dist/'), `Unexpected runtime export target: ${target}`)
  const path = join(artifact, target.slice('./dist/'.length))
  assert.ok(statSync(path).isFile(), `Missing artifact file: ${target}`)
  return path
}

for (const [name, target] of Object.entries(manifest.exports)) {
  if (name === './package.json') continue
  artifactFile(target.types)
  const commonjs = require(artifactFile(target.require))
  const esm = await import(pathToFileURL(artifactFile(target.import)).href)
  for (const key of Object.keys(commonjs)) {
    assert.equal(esm[key], commonjs[key], `${name}: inconsistent ESM export ${key}`)
  }
  modules.set(name, commonjs)
}

assert.equal(typeof modules.get('.').WinGuid.parse, 'function')
assert.equal(typeof modules.get('./com').initializeCom, 'function')
const { DynWin32, DynWin32Function } = modules.get('./win32/unsafe')
const tickCount = DynWin32Function.bind({
  dll: 'kernel32.dll',
  entryPoint: 'GetTickCount',
  parameters: [],
  returnType: 'u32',
}).invoke([])
assert.ok(Number.isInteger(DynWin32.toNumber(tickCount.returnValue)))
assert.equal(modules.get('./win32').DynWin32.initializeMapiUtilities, undefined)
console.log(`Verified ${modules.size} downloaded runtime entrypoints through CommonJS, ESM and native dispatch`)
