// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { resolve } from 'node:path'
import { defineConfig, externalizeDepsPlugin } from 'electron-vite'

export default defineConfig({
  main: {
    plugins: [externalizeDepsPlugin()],
    build: {
      commonjsOptions: {
        include: [/node_modules/, /generated/]
      },
      rollupOptions: {
        external: [/^@microsoft\/dynwinrt(?:\/com)?$/, /\.node$/]
      }
    }
  },
  preload: {
    plugins: [externalizeDepsPlugin()]
  },
  renderer: {
    build: {
      rollupOptions: {
        input: resolve(__dirname, 'src/renderer/index.html')
      }
    }
  }
})
