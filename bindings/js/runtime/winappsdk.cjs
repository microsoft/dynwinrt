// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

'use strict'

const { createHash } = require('node:crypto')
const { readFileSync, statSync } = require('node:fs')
const { dirname, isAbsolute, join, resolve } = require('node:path')

const configurations = new Map()
const dllName = 'Microsoft.WindowsAppRuntime.Bootstrap.dll'

function configuration(source, value) {
  const invalid = (reason) => new TypeError(`Invalid Windows App SDK configuration from ${source}: ${reason}`)
  if (typeof source !== 'string' || !isAbsolute(source)) {
    throw invalid('source must be the absolute generated configuration module path')
  }
  if (
    !value ||
    typeof value !== 'object' ||
    Array.isArray(value) ||
    Object.keys(value).sort().join(',') !== 'dlls,major,minor,schema_version'
  ) {
    throw invalid('unexpected configuration fields')
  }
  if (value.schema_version !== 1) throw invalid('unsupported schema_version')
  for (const name of ['major', 'minor']) {
    if (!Number.isInteger(value[name]) || value[name] < 0 || value[name] > 65535) {
      throw invalid(`${name} must be an integer between 0 and 65535`)
    }
  }
  if (
    !value.dlls ||
    typeof value.dlls !== 'object' ||
    !Object.keys(value.dlls).length ||
    Object.keys(value.dlls).some((arch) => !['x64', 'arm64'].includes(arch))
  ) {
    throw invalid('x64 and/or arm64 DLL digests are required')
  }
  const dlls = {}
  for (const arch of Object.keys(value.dlls).sort()) {
    if (typeof value.dlls[arch] !== 'string' || !/^[0-9a-f]{64}$/.test(value.dlls[arch])) {
      throw invalid(`invalid ${arch} SHA-256 digest`)
    }
    dlls[arch] = value.dlls[arch]
  }
  return Object.freeze({
    source: resolve(source),
    major: value.major,
    minor: value.minor,
    dlls: Object.freeze(dlls),
    identity: JSON.stringify([value.major, value.minor, dlls]),
  })
}

function registerConfig(source, value) {
  const config = configuration(source, value)
  const previous = configurations.get(config.source)
  if (previous && previous.identity !== config.identity) {
    throw new Error(`Windows App SDK configuration changed in ${config.source}; restart with one SDK bundle`)
  }
  configurations.set(config.source, config)
}

function discover() {
  const configs = [...configurations.values()].sort((a, b) => a.source.localeCompare(b.source))
  if (!configs.length) {
    throw new Error(
      'No generated Windows App SDK configuration has been imported. Import the generated bindings package ' +
        'before initWinappsdk(), or pass major, minor and bootstrapDllPath explicitly.',
    )
  }
  const first = configs[0]
  if (configs.some((item) => item.identity !== first.identity)) {
    throw new Error(
      'Conflicting generated Windows App SDK configurations: ' +
        configs.map((item) => `${item.source}: ${item.major}.${item.minor}`).join('; ') +
        '. Use one consistent SDK bundle or pass explicit initialization arguments.',
    )
  }
  return first
}

function explicitPath(value) {
  if (typeof value !== 'string' || !value.trim() || value.includes('\0')) {
    throw new TypeError('Windows App SDK bootstrapDllPath must be a nonempty path without NUL')
  }
  const path = resolve(value)
  if (!statSync(path).isFile()) {
    throw new Error(`Windows App SDK bootstrap DLL is not a file: ${path}`)
  }
  return path
}

function resourcePath(config) {
  if (process.platform !== 'win32' || !['x64', 'arm64'].includes(process.arch)) {
    throw new Error(`Unsupported Node platform ${process.platform}/${process.arch}; use Windows x64 or ARM64 Node.js`)
  }
  if (!config.dlls[process.arch]) {
    throw new Error(
      `Windows App SDK configuration from ${config.source} has no ${process.arch} DLL; install matching bindings`,
    )
  }
  const path = join(dirname(config.source), '_native', 'winappsdk', process.arch, dllName)
  let bytes
  try {
    bytes = readFileSync(path)
  } catch (error) {
    if (error.code !== 'ENOENT') throw error
    throw new Error(
      `Generated Windows App SDK bootstrap DLL is missing: ${path}. Reinstall the complete generated package.`,
      { cause: error },
    )
  }
  const digest = createHash('sha256').update(bytes).digest('hex')
  if (digest !== config.dlls[process.arch]) {
    throw new Error(`Windows App SDK bootstrap DLL digest mismatch: ${path}. Reinstall the complete generated package.`)
  }
  return path
}

function createInitializer(initialize) {
  return function initWinappsdk(major, minor, bootstrapDllPath) {
    if (arguments.length !== 0) {
      if (major === undefined || minor === undefined) {
        throw new TypeError('initWinappsdk() requires both major and minor, or neither')
      }
      if (bootstrapDllPath === undefined) return initialize(major, minor)
      return initialize(major, minor, explicitPath(bootstrapDllPath))
    }
    const config = discover()
    const override = process.env.WINAPPSDK_BOOTSTRAP_DLL_PATH
    const path = override !== undefined ? explicitPath(override) : resourcePath(config)
    return initialize(config.major, config.minor, path)
  }
}

module.exports.registerConfig = registerConfig
module.exports.createInitializer = createInitializer
