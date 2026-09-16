// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Setup and identity-aware launch: README.md.
import('./cli.ts')
    .then(async ({ runCli }) => {
        process.exitCode = await runCli(process.argv.slice(2))
    })
    .catch((error: unknown) => {
        console.error('Unable to start Windows AI OCR:', error)
        process.exitCode = 1
    })
