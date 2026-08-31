// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const { roInitialize } = require('@microsoft/dynwinrt')
const {
  UserConsentVerifier,
  UserConsentVerifierAvailability,
} = require('./generated/index.js')

function enumName(values, value) {
  return Object.entries(values).find(([, candidate]) => candidate === value)?.[0] ?? `Unknown (${value})`
}

async function main() {
  roInitialize(1)
  const availability = await UserConsentVerifier.checkAvailabilityAsync()
  console.log(`Windows Hello availability: ${enumName(UserConsentVerifierAvailability, availability)}`)
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
