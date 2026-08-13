// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'
import { AsyncLocalStorage } from 'node:async_hooks'
import { spawn } from 'node:child_process'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

interface TsfnStats {
  produced: number
  dropped: number
  accepted: number
  queueFull: number
  closing: number
  otherFailure: number
}

interface TsfnDelegateInvokeStats {
  succeeded: number
  failed: number
}

interface TsfnTestRuntime {
  tsfnTestReset(): void
  tsfnTestStats(): TsfnStats
  tsfnTestStartUnbounded(callback: (id: number) => void, count: number, delayMs: number): void
  tsfnTestStartBounded(callback: (id: number) => void, count: number, delayMs: number): void
  tsfnTestWaitProduced(expected: number, timeoutMs: number): boolean
  tsfnTestHoldStrong(callback: (id: number) => void): void
  tsfnTestHoldWeak(callback: (id: number) => void): void
  tsfnTestReleaseHeld(): void
  tsfnTestRegisteredHandleCount(): number
  tsfnTestRetainDelegate(delegate: unknown): void
  tsfnTestInvokeRetainedDelegate(): number
  tsfnTestInvokeRetainedDelegateOnThread(): number
  tsfnTestInvokeRetainedDelegateOnThreadMany(count: number): TsfnDelegateInvokeStats
  tsfnTestReleaseRetainedDelegate(): void
}

const runtime = createRequire(import.meta.url)('../dist/index.js') as Partial<TsfnTestRuntime>
const hasTestHooks = typeof runtime.tsfnTestStartUnbounded === 'function'

async function waitForDrops(expected: number): Promise<TsfnStats> {
  const deadline = Date.now() + 2_000
  let stats = runtime.tsfnTestStats!()
  while (stats.dropped < expected && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 5))
    stats = runtime.tsfnTestStats!()
  }
  return stats
}

if (!hasTestHooks) {
  test.skip('TSFN native test hooks require the test-hooks Cargo feature', () => {})
} else {
  test.serial('default TSFN queue accepts work before any JavaScript callback executes', async (t) => {
    const count = 10_000
    let callbacks = 0
    runtime.tsfnTestReset!()
    runtime.tsfnTestStartUnbounded!(() => {
      callbacks += 1
    }, count, 0)

    t.true(runtime.tsfnTestWaitProduced!(count, 2_000))
    const queued = runtime.tsfnTestStats!()
    t.is(queued.produced, count)
    t.is(queued.accepted, count)
    t.is(queued.queueFull, 0)
    t.is(callbacks, 0)
    t.is(queued.dropped, 0)

    const drained = await waitForDrops(count)
    t.is(drained.dropped, count)
    t.is(callbacks, count)
  })

  test.serial('bounded TSFN releases every payload rejected with QueueFull', async (t) => {
    const count = 1_000
    runtime.tsfnTestReset!()
    runtime.tsfnTestStartBounded!(() => {}, count, 0)

    t.true(runtime.tsfnTestWaitProduced!(count, 2_000))
    const drained = await waitForDrops(count)
    t.true(drained.queueFull > 0)
    t.is(drained.dropped, drained.produced)
  })

  test.serial('environment teardown releases every payload already queued in a TSFN', async (t) => {
    const child = spawn(
      process.execPath,
      [fileURLToPath(new URL('./tsfn-teardown-child.mjs', import.meta.url))],
      {
        env: {
          ...process.env,
          DYNWINRT_TEST_RUNTIME: fileURLToPath(new URL('../dist/index.js', import.meta.url)),
        },
        stdio: ['ignore', 'pipe', 'pipe'],
        windowsHide: true,
      },
    )
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => {
      stdout += String(chunk)
    })
    child.stderr.on('data', (chunk) => {
      stderr += String(chunk)
    })
    const code = await new Promise<number | null>((resolve) => {
      child.once('close', resolve)
    })

    t.regex(stdout, /"produced":10000/)
    t.is(code, 0, `${stdout}\n${stderr}`)
  })

  test.serial('a thrown TSFN callback reaches uncaughtException', async (t) => {
    const child = spawn(
      process.execPath,
      [fileURLToPath(new URL('./tsfn-exception-child.mjs', import.meta.url))],
      {
        env: {
          ...process.env,
          DYNWINRT_TEST_RUNTIME: fileURLToPath(new URL('../dist/index.js', import.meta.url)),
        },
        stdio: ['ignore', 'pipe', 'pipe'],
        windowsHide: true,
      },
    )
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => {
      stdout += String(chunk)
    })
    child.stderr.on('data', (chunk) => {
      stderr += String(chunk)
    })
    const code = await new Promise<number | null>((resolve) => {
      child.once('close', resolve)
    })

    t.is(code, 0, stderr)
    t.regex(stdout, /tsfn-uncaught:TSFN callback failure/)
  })

  for (const [mode, expectedMarker] of [
    ['strong', 'strong-release-fired'],
    ['weak', 'weak-exited-without-timer'],
  ] as const) {
    test.serial(`${mode} TSFN has the expected event-loop liveness`, async (t) => {
      const started = Date.now()
      const child = spawn(
        process.execPath,
        [fileURLToPath(new URL('./tsfn-liveness-child.mjs', import.meta.url)), mode],
        {
          env: {
            ...process.env,
            DYNWINRT_TEST_RUNTIME: fileURLToPath(new URL('../dist/index.js', import.meta.url)),
          },
          stdio: ['ignore', 'pipe', 'pipe'],
          windowsHide: true,
        },
      )
      let stdout = ''
      let stderr = ''
      child.stdout.on('data', (chunk) => {
        stdout += String(chunk)
      })
      child.stderr.on('data', (chunk) => {
        stderr += String(chunk)
      })
      const code = await new Promise<number | null>((resolve) => {
        child.once('close', resolve)
      })
      const elapsed = Date.now() - started

      t.is(code, 0, stderr)
      t.regex(stdout, new RegExp(expectedMarker))
      if (mode === 'strong') {
        t.true(elapsed >= 200, `strong TSFN exited after only ${elapsed} ms`)
      } else {
        t.true(elapsed < 1_000, `weak TSFN took ${elapsed} ms to exit`)
      }
    })
  }

  test.serial('short-lived TSFNs unregister from the per-environment registry', async (t) => {
    runtime.tsfnTestReset!()
    for (let index = 0; index < 100; index += 1) {
      runtime.tsfnTestStartUnbounded!(() => {}, 1, 0)
    }
    t.true(runtime.tsfnTestWaitProduced!(100, 2_000))
    await waitForDrops(100)
    const deadline = Date.now() + 2_000
    while (runtime.tsfnTestRegisteredHandleCount!() !== 0 && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 5))
    }
    t.is(runtime.tsfnTestRegisteredHandleCount!(), 0)
  })

  test.serial('a delegate retained past Worker teardown fails late invocation without crashing', async (t) => {
    const child = spawn(
      process.execPath,
      [fileURLToPath(new URL('./tsfn-worker-delegate-child.mjs', import.meta.url))],
      {
        env: {
          ...process.env,
          DYNWINRT_TEST_RUNTIME: fileURLToPath(new URL('../dist/index.js', import.meta.url)),
        },
        stdio: ['ignore', 'pipe', 'pipe'],
        windowsHide: true,
      },
    )
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => {
      stdout += String(chunk)
    })
    child.stderr.on('data', (chunk) => {
      stderr += String(chunk)
    })
    const code = await new Promise<number | null>((resolve) => {
      child.once('close', resolve)
    })

    t.is(code, 0, `${stdout}\n${stderr}`)
    t.regex(stdout, /late-delegate-hr:-2147467259/)
  })

  test.serial('same-thread delegate dispatch preserves AsyncLocalStorage context', async (t) => {
    const native = runtime as TsfnTestRuntime & {
      DynWinRtDelegate: {
        create(iid: unknown, paramTypes: unknown[], callback: () => void): unknown
      }
      WinGuid: {
        parse(value: string): unknown
      }
    }
    const storage = new AsyncLocalStorage<string>()
    let observed: string | undefined
    const delegate = storage.run('tsfn-context', () =>
      native.DynWinRtDelegate.create(
        native.WinGuid.parse('45396ba0-cd24-42d4-9685-6863e032d69d'),
        [],
        () => {
          observed = storage.getStore()
        },
      ),
    )
    native.tsfnTestRetainDelegate(delegate)

    await new Promise<void>((resolve, reject) => {
      setImmediate(() => {
        try {
          t.is(storage.getStore(), undefined)
          t.is(native.tsfnTestInvokeRetainedDelegate(), 0)
          native.tsfnTestReleaseRetainedDelegate()
          resolve()
        } catch (error) {
          reject(error)
        }
      })
    })

    t.is(observed, 'tsfn-context')
  })

  test.serial('cross-thread delegate S_OK means queued, not executed', async (t) => {
    const native = runtime as TsfnTestRuntime & {
      DynWinRtDelegate: {
        create(iid: unknown, paramTypes: unknown[], callback: () => void): unknown
      }
      WinGuid: {
        parse(value: string): unknown
      }
    }
    let fired = false
    const delegate = native.DynWinRtDelegate.create(
      native.WinGuid.parse('bf33f101-7383-449f-9ad1-d76e960c9aac'),
      [],
      () => {
        fired = true
      },
    )
    native.tsfnTestRetainDelegate(delegate)

    t.is(native.tsfnTestInvokeRetainedDelegateOnThread(), 0)
    t.false(fired)
    await new Promise((resolve) => setImmediate(resolve))
    t.true(fired)
    native.tsfnTestReleaseRetainedDelegate()
  })

  test.serial('production delegate TSFN applies a finite queue limit', async (t) => {
    const native = runtime as TsfnTestRuntime & {
      DynWinRtDelegate: {
        create(iid: unknown, paramTypes: unknown[], callback: () => void): unknown
      }
      WinGuid: {
        parse(value: string): unknown
      }
    }
    let callbacks = 0
    const delegate = native.DynWinRtDelegate.create(
      native.WinGuid.parse('e2923908-eac9-44f4-b05a-891a84728a18'),
      [],
      () => {
        callbacks += 1
      },
    )
    native.tsfnTestRetainDelegate(delegate)

    const results = native.tsfnTestInvokeRetainedDelegateOnThreadMany(1_030)
    t.is(results.succeeded, 1_024)
    t.is(results.failed, 6)
    t.is(callbacks, 0)
    while (callbacks < results.succeeded) {
      await new Promise((resolve) => setImmediate(resolve))
    }
    t.is(callbacks, results.succeeded)
    native.tsfnTestReleaseRetainedDelegate()
  })
}
