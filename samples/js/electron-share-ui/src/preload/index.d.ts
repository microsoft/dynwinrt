// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

declare global {
  interface Window {
    shareSample: {
      show(content: { title: string; text: string }): Promise<void>
    }
  }
}

export {}

