// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { launchWithGeneratedBindings, runLauncher } from './launch.ts'

process.exitCode = await runLauncher(process.argv.slice(2), launchWithGeneratedBindings)
