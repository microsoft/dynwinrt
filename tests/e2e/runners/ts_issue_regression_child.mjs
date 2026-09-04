// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import * as path from 'node:path'
import { createRequire } from 'node:module'
import { pathToFileURL } from 'node:url'

const [name, generatedDir, runtimePath] = process.argv.slice(2)

if (!name || !generatedDir || !runtimePath) {
  throw new Error('Usage: ts_issue_regression_child.mjs <name> <generated> <runtime>')
}

const require = createRequire(import.meta.url)
const generatedRoot = require(path.resolve(generatedDir, 'index.js'))

const importGenerated = async (className) => {
  const cls = generatedRoot[className]
  if (!cls) {
    throw new Error(`${className} was not exported by the generated root`)
  }
  return cls
}

const runtime = await import(pathToFileURL(path.resolve(runtimePath)).href)
runtime.roInitialize(1)

if (name === 'device_information_async_collection') {
  const DeviceInformation = await importGenerated('DeviceInformation')
  const devices = await DeviceInformation.findAllAsync()
  if (devices == null || typeof devices.size !== 'number') {
    throw new Error('DeviceInformation.findAllAsync() did not return a collection')
  }
  console.log(`device-information-ok size=${devices.size}`)
} else if (name === 'bitmap_encoder_async_create') {
  const BitmapEncoder = await importGenerated('BitmapEncoder')
  const InMemoryRandomAccessStream = await importGenerated('InMemoryRandomAccessStream')
  const stream = new InMemoryRandomAccessStream()
  const encoder = await BitmapEncoder.createAsync(BitmapEncoder.jpegEncoderId, stream)
  if (encoder == null) {
    throw new Error('BitmapEncoder.createAsync() returned null')
  }
  console.log('bitmap-encoder-ok')
} else {
  throw new Error(`Unknown issue regression: ${name}`)
}
