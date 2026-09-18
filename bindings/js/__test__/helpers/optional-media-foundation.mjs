// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/** Only the explicit System32 loader's missing-component errors permit a skip. */
export function isMissingMediaFoundation(/** @type {unknown} */ error) {
  if (!(error instanceof Error)) return false
  return (
    /^0x8007007E: System DLL `mfplat\.dll` could not be loaded from System32: .+/.test(error.message) ||
    /^0x8007007F: Export `(?:MFStartup|MFShutdown)` was not found in `mfplat\.dll`(?: |$)/.test(error.message)
  )
}
