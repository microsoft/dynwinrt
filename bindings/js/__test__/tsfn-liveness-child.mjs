// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { createRequire } from 'node:module'

const runtime = createRequire(import.meta.url)(process.env.DYNWINRT_TEST_RUNTIME)
const mode = process.argv[2]

if (mode === 'strong') {
  runtime.tsfnTestHoldStrong(() => {})
  const release = setTimeout(() => {
    runtime.tsfnTestReleaseHeld()
    console.log('strong-release-fired')
  }, 300)
  release.unref()
} else if (mode === 'weak') {
  runtime.tsfnTestHoldWeak(() => {})
  process.once('beforeExit', () => {
    runtime.tsfnTestReleaseHeld()
    console.log('weak-exited-without-timer')
  })
  const shouldNotFire = setTimeout(() => {
    console.error('weak TSFN kept the event loop alive')
    process.exit(1)
  }, 1_000)
  shouldNotFire.unref()
} else {
  throw new Error(`Unknown TSFN liveness mode: ${mode}`)
}
