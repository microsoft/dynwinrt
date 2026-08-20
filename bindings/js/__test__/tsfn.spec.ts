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

interface TsfnComSinkInvokeResult {
  hresult: number
  output: number
}

interface TsfnComAllocatedBufferResult {
  hresult: number
  count: number
  byteSum: number
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
  tsfnTestRetainComSink(value: unknown): void
  tsfnTestInvokeRetainedComSink(): number
  tsfnTestInvokeRetainedComSinkOnThread(): number
  tsfnTestInvokeRetainedComSinkOut(): TsfnComSinkInvokeResult
  tsfnTestInvokeRetainedComSinkI32(value: number): number
  tsfnTestInvokeRetainedComSinkI32OnThread(value: number): number
  tsfnTestInvokeRetainedComSinkDirectI32(value: number): number
  tsfnTestInvokeRetainedComSinkDirectI32OnThread(value: number): number
  tsfnTestInvokeRetainedComSinkVoidI32(value: number): void
  tsfnTestInvokeRetainedComSinkGuid(): number
  tsfnTestInvokeRetainedComSinkBstr(): number
  tsfnTestInvokeRetainedComSinkWideString(): number
  tsfnTestInvokeRetainedComSinkAnsiString(): number
  tsfnTestInvokeRetainedComSinkAllocatedBuffer(): TsfnComAllocatedBufferResult
  tsfnTestInvokeRetainedComSinkCallerBuffer(): TsfnComAllocatedBufferResult
  tsfnTestInvokeRetainedComObjectI32(iid: unknown, value: number): number
  tsfnTestReleaseRetainedComSink(): void
}

interface TestComMethodSig {
  addIn(type: unknown): TestComMethodSig
  addOut(type: unknown): TestComMethodSig
  returns(type: unknown): TestComMethodSig
  returnsVoid(): TestComMethodSig
  addCoTaskMemOutputBuffer(elementType: unknown, countParam: number, countIsBytes: boolean): TestComMethodSig
  addCallerOutputBuffer(
    elementType: unknown,
    capacityParam: number,
    actualLengthParam: number | undefined,
    countIsBytes: boolean,
    twoCall: boolean,
  ): TestComMethodSig
}

interface TestComInterface {
  addMethodAt(vtableIndex: number, name: string, signature: TestComMethodSig): TestComInterface
}

interface ComSinkTestRuntime extends TsfnTestRuntime {
  DynCom: {
    registerIUnknownInterface(name: string, iid: unknown): TestComInterface
    interfaceType(iid: unknown): unknown
    i32Type(): unknown
    u8Type(): unknown
    u32Type(): unknown
    bstrType(): unknown
    pointerType(): unknown
    buffer(value: ArrayBufferView): unknown
    copyCallbackGuid(value: unknown): string
    copyCallbackBstr(value: unknown): string | null
    copyCallbackWideString(value: unknown): string | null
    copyCallbackAnsiString(value: unknown): string | null
    createIUnknownSink(
      interfaceType: TestComInterface,
      callback: (vtableIndex: number, ...args: unknown[]) => unknown,
    ): { release(): void }
    createComObject(
      interfaces: TestComInterface[],
      callback: (interfaceIid: string, vtableIndex: number, ...args: unknown[]) => unknown,
    ): { release(): void }
  }
  DynComMethodSig: new () => TestComMethodSig
  WinGuid: {
    parse(value: string): unknown
  }
}

function registerTestComSinkInterface(native: ComSinkTestRuntime, withOutput: boolean): TestComInterface {
  const iid = native.WinGuid.parse('7ac2eaa2-97a4-43f0-9b0f-421c2363ef11')
  const interfaceType = native.DynCom.interfaceType(native.WinGuid.parse('00000000-0000-0000-c000-000000000046'))
  let signature = new native.DynComMethodSig().addIn(interfaceType)
  if (withOutput) {
    signature = signature.addIn(interfaceType).addOut(native.DynCom.i32Type())
  }

  return native.DynCom.registerIUnknownInterface('Tests.IComSink', iid).addMethodAt(3, 'Invoke', signature)
}

function registerTestComI32SinkInterface(native: ComSinkTestRuntime): TestComInterface {
  const iid = native.WinGuid.parse('7ac2eaa2-97a4-43f0-9b0f-421c2363ef11')
  const signature = new native.DynComMethodSig().addIn(native.DynCom.i32Type())
  return native.DynCom.registerIUnknownInterface('Tests.IComI32Sink', iid).addMethodAt(3, 'Invoke', signature)
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
    runtime.tsfnTestStartUnbounded!(
      () => {
        callbacks += 1
      },
      count,
      0,
    )

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
    const child = spawn(process.execPath, [fileURLToPath(new URL('./tsfn-teardown-child.mjs', import.meta.url))], {
      env: {
        ...process.env,
        DYNWINRT_TEST_RUNTIME: fileURLToPath(new URL('../dist/index.js', import.meta.url)),
      },
      stdio: ['ignore', 'pipe', 'pipe'],
      windowsHide: true,
    })
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
    const child = spawn(process.execPath, [fileURLToPath(new URL('./tsfn-exception-child.mjs', import.meta.url))], {
      env: {
        ...process.env,
        DYNWINRT_TEST_RUNTIME: fileURLToPath(new URL('../dist/index.js', import.meta.url)),
      },
      stdio: ['ignore', 'pipe', 'pipe'],
      windowsHide: true,
    })
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

  test.serial('a COM sink retained past Worker teardown rejects late invocation without crashing', async (t) => {
    const child = spawn(
      process.execPath,
      [fileURLToPath(new URL('./tsfn-worker-com-sink-child.mjs', import.meta.url))],
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
    t.regex(stdout, /late-com-sink-hr:-2147417842/)
  })

  test.serial('a thrown COM sink callback fails the call and releases callback resources', async (t) => {
    const child = spawn(
      process.execPath,
      [fileURLToPath(new URL('./tsfn-com-sink-exception-child.mjs', import.meta.url))],
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
    t.regex(stdout, /com-sink-uncaught:COM sink callback failure;hr:-2147467259/)
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
      native.DynWinRtDelegate.create(native.WinGuid.parse('45396ba0-cd24-42d4-9685-6863e032d69d'), [], () => {
        observed = storage.getStore()
      }),
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

  test.serial('COM sinks execute synchronously on their owner thread and reject other threads', (t) => {
    const native = runtime as ComSinkTestRuntime
    let calls = 0
    const sink = native.DynCom.createIUnknownSink(registerTestComSinkInterface(native, false), (vtableIndex, value) => {
      t.is(vtableIndex, 3)
      t.truthy(value)
      calls += 1
      return 0
    })
    runtime.tsfnTestRetainComSink!(sink)

    t.is(runtime.tsfnTestInvokeRetainedComSink!(), 0)
    t.is(calls, 1)
    t.is(runtime.tsfnTestInvokeRetainedComSinkOnThread!(), -2147417842)
    t.is(calls, 1)

    runtime.tsfnTestReleaseRetainedComSink!()
    sink.release()
  })

  test.serial('COM sinks synchronously return HRESULT and required i32 output', (t) => {
    const native = runtime as ComSinkTestRuntime
    const sink = native.DynCom.createIUnknownSink(
      registerTestComSinkInterface(native, true),
      (vtableIndex, first, second) => {
        t.is(vtableIndex, 3)
        t.truthy(first)
        t.truthy(second)
        return [1, native.DynCom.i32(42)]
      },
    )
    runtime.tsfnTestRetainComSink!(sink)

    t.deepEqual(runtime.tsfnTestInvokeRetainedComSinkOut!(), {
      hresult: 1,
      output: 42,
    })

    runtime.tsfnTestReleaseRetainedComSink!()
    sink.release()
  })

  test.serial('COM sinks use libffi for runtime scalar callback signatures', (t) => {
    const native = runtime as ComSinkTestRuntime
    let calls = 0
    const sink = native.DynCom.createIUnknownSink(registerTestComI32SinkInterface(native), (vtableIndex, value) => {
      t.is(vtableIndex, 3)
      calls += 1
      const scalar = value as { toNumber(): number }
      return scalar.toNumber() + 1
    })
    runtime.tsfnTestRetainComSink!(sink)

    t.is(runtime.tsfnTestInvokeRetainedComSinkI32!(41), 42)
    t.is(runtime.tsfnTestInvokeRetainedComSinkI32OnThread!(41), -2147417842)
    t.is(calls, 1)

    runtime.tsfnTestReleaseRetainedComSink!()
    sink.release()
  })

  test.serial('COM sinks use libffi for direct and void native returns', (t) => {
    const native = runtime as ComSinkTestRuntime
    const iid = native.WinGuid.parse('16787a9f-b53c-41ba-87e7-f368950a79df')
    const directInterface = native.DynCom.registerIUnknownInterface('Tests.IDirectSink', iid).addMethodAt(
      3,
      'Invoke',
      new native.DynComMethodSig().addIn(native.DynCom.i32Type()).returns(native.DynCom.i32Type()),
    )
    const direct = native.DynCom.createIUnknownSink(directInterface, (_slot, value) =>
      native.DynCom.i32((value as { toNumber(): number }).toNumber() + 1),
    )
    runtime.tsfnTestRetainComSink!(direct)
    t.is(runtime.tsfnTestInvokeRetainedComSinkDirectI32!(41), 42)
    t.is(runtime.tsfnTestInvokeRetainedComSinkDirectI32OnThread!(41), 0)
    runtime.tsfnTestReleaseRetainedComSink!()
    direct.release()

    let observed = 0
    const voidInterface = native.DynCom.registerIUnknownInterface('Tests.IVoidSink', iid).addMethodAt(
      3,
      'Invoke',
      new native.DynComMethodSig().addIn(native.DynCom.i32Type()).returnsVoid(),
    )
    const voidSink = native.DynCom.createIUnknownSink(voidInterface, (_slot, value) => {
      observed = (value as { toNumber(): number }).toNumber()
    })
    runtime.tsfnTestRetainComSink!(voidSink)
    runtime.tsfnTestInvokeRetainedComSinkVoidI32!(27)
    t.is(observed, 27)
    runtime.tsfnTestReleaseRetainedComSink!()
    voidSink.release()
  })

  test.serial('COM sinks copy REFGUID inputs and allocate CoTaskMem output buffers', (t) => {
    const native = runtime as ComSinkTestRuntime
    const iid = native.WinGuid.parse('f25cce61-827f-4b11-b13f-8e276b0e67a9')
    let observedGuid = ''
    const guidInterface = native.DynCom.registerIUnknownInterface('Tests.IGuidSink', iid).addMethodAt(
      3,
      'Invoke',
      new native.DynComMethodSig().addIn(native.DynCom.pointerType()),
    )
    const guidSink = native.DynCom.createIUnknownSink(guidInterface, (_slot, value) => {
      observedGuid = native.DynCom.copyCallbackGuid(value)
      return 0
    })
    runtime.tsfnTestRetainComSink!(guidSink)
    t.is(runtime.tsfnTestInvokeRetainedComSinkGuid!(), 0)
    t.is(observedGuid.toLowerCase(), '990c600e-60c7-4d28-af4c-bf148a92b11a')
    runtime.tsfnTestReleaseRetainedComSink!()
    guidSink.release()

    let observedBstr: string | null = null
    const bstrInterface = native.DynCom.registerIUnknownInterface('Tests.IBstrSink', iid).addMethodAt(
      3,
      'Invoke',
      new native.DynComMethodSig().addIn(native.DynCom.bstrType()),
    )
    const bstrSink = native.DynCom.createIUnknownSink(bstrInterface, (_slot, value) => {
      observedBstr = native.DynCom.copyCallbackBstr(value)
      return 0
    })
    runtime.tsfnTestRetainComSink!(bstrSink)
    t.is(runtime.tsfnTestInvokeRetainedComSinkBstr!(), 0)
    t.is(observedBstr, 'embedded\0callback')
    runtime.tsfnTestReleaseRetainedComSink!()
    bstrSink.release()

    const stringInterface = (name: string) =>
      native.DynCom.registerIUnknownInterface(name, iid).addMethodAt(
        3,
        'Invoke',
        new native.DynComMethodSig().addIn(native.DynCom.pointerType()),
      )
    let observedString: string | null = null
    const wideSink = native.DynCom.createIUnknownSink(stringInterface('Tests.IWideStringSink'), (_slot, value) => {
      observedString = native.DynCom.copyCallbackWideString(value)
      return 0
    })
    runtime.tsfnTestRetainComSink!(wideSink)
    t.is(runtime.tsfnTestInvokeRetainedComSinkWideString!(), 0)
    t.is(observedString, 'wide callback')
    runtime.tsfnTestReleaseRetainedComSink!()
    wideSink.release()

    const ansiSink = native.DynCom.createIUnknownSink(stringInterface('Tests.IAnsiStringSink'), (_slot, value) => {
      observedString = native.DynCom.copyCallbackAnsiString(value)
      return 0
    })
    runtime.tsfnTestRetainComSink!(ansiSink)
    t.is(runtime.tsfnTestInvokeRetainedComSinkAnsiString!(), 0)
    t.is(observedString, 'ansi callback')
    runtime.tsfnTestReleaseRetainedComSink!()
    ansiSink.release()

    const bufferInterface = native.DynCom.registerIUnknownInterface('Tests.IBufferSink', iid).addMethodAt(
      3,
      'Invoke',
      new native.DynComMethodSig()
        .addCoTaskMemOutputBuffer(native.DynCom.u8Type(), 1, false)
        .addOut(native.DynCom.u32Type()),
    )
    const bufferSink = native.DynCom.createIUnknownSink(bufferInterface, () => [
      0,
      native.DynCom.buffer(Uint8Array.from([4, 5, 6])),
    ])
    runtime.tsfnTestRetainComSink!(bufferSink)
    t.deepEqual(runtime.tsfnTestInvokeRetainedComSinkAllocatedBuffer!(), {
      hresult: 0,
      count: 3,
      byteSum: 15,
    })
    runtime.tsfnTestReleaseRetainedComSink!()
    bufferSink.release()

    const callerBufferInterface = native.DynCom.registerIUnknownInterface('Tests.ICallerBufferSink', iid).addMethodAt(
      3,
      'Invoke',
      new native.DynComMethodSig()
        .addCallerOutputBuffer(native.DynCom.u8Type(), 1, 2, false, false)
        .addIn(native.DynCom.u32Type())
        .addOut(native.DynCom.u32Type()),
    )
    const callerBufferSink = native.DynCom.createIUnknownSink(callerBufferInterface, (_slot, capacity) => {
      t.is((capacity as { toNumber(): number }).toNumber(), 5)
      return [0, native.DynCom.buffer(Uint8Array.from([9, 8, 7]))]
    })
    runtime.tsfnTestRetainComSink!(callerBufferSink)
    t.deepEqual(runtime.tsfnTestInvokeRetainedComSinkCallerBuffer!(), {
      hresult: 0,
      count: 3,
      byteSum: 24,
    })
    runtime.tsfnTestReleaseRetainedComSink!()
    callerBufferSink.release()
  })

  test.serial('COM objects dispatch multiple interface views through one identity', (t) => {
    const native = runtime as ComSinkTestRuntime
    const firstIid = native.WinGuid.parse('a4c5b87d-f5cc-420b-93bd-a01b9415de83')
    const secondIid = native.WinGuid.parse('8a28f0f7-8d77-46aa-a9d1-95d01f6b3179')
    const register = (name: string, iid: unknown) =>
      native.DynCom.registerIUnknownInterface(name, iid).addMethodAt(
        3,
        'Invoke',
        new native.DynComMethodSig().addIn(native.DynCom.i32Type()),
      )
    const seen = new Map<string, number>()
    const object = native.DynCom.createComObject(
      [register('Tests.IFirst', firstIid), register('Tests.ISecond', secondIid)],
      (iid, vtableIndex, value) => {
        t.is(vtableIndex, 3)
        seen.set(iid, (value as { toNumber(): number }).toNumber())
        return 0
      },
    )
    runtime.tsfnTestRetainComSink!(object)

    t.is(runtime.tsfnTestInvokeRetainedComObjectI32!(firstIid, 11), 0)
    t.is(runtime.tsfnTestInvokeRetainedComObjectI32!(secondIid, 22), 0)
    t.deepEqual(
      [...seen.values()].sort((left, right) => left - right),
      [11, 22],
    )

    runtime.tsfnTestReleaseRetainedComSink!()
    object.release()
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
