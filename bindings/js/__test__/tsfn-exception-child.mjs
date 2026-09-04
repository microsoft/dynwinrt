// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { createRequire } from 'node:module'

const runtime = createRequire(import.meta.url)(process.env.DYNWINRT_TEST_RUNTIME)
const timeout = setTimeout(() => {
  console.error('TSFN callback exception was not observed')
  process.exit(1)
}, 2_000)

process.once('uncaughtException', (error) => {
  clearTimeout(timeout)
  console.log(`tsfn-uncaught:${error.message}`)
  process.exit(0)
})

runtime.tsfnTestStartUnbounded(() => {
  throw new Error('TSFN callback failure')
}, 1, 0)
