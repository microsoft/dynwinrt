// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { createRequire } from 'node:module'

const requireFromBinding = createRequire(
  new URL('../../../bindings/js/package.json', import.meta.url),
)
const { Bench } = requireFromBinding('tinybench') as typeof import('tinybench')
const { plus100 } = requireFromBinding('./index.js') as {
  plus100(value: number): number
}

function add(a: number) {
  return a + 100
}

const bench = new Bench()

bench.add('Native a + 100', () => {
  plus100(10)
})

bench.add('JavaScript a + 100', () => {
  add(10)
})

await bench.run()

console.table(bench.table())
