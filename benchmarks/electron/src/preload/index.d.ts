// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

declare global {
  interface Window {
    api: {
      ipc: (channel: string) => Promise<unknown>
      logResults: (lines: string[]) => Promise<void>
    }
  }
}

export {}
