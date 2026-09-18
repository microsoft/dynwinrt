// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import type { SpawnSyncOptionsWithStringEncoding, SpawnSyncReturns } from 'node:child_process'

export declare function runCodegen(
  args: string[],
  options: SpawnSyncOptionsWithStringEncoding,
): SpawnSyncReturns<string>
