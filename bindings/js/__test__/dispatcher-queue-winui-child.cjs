// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const Module = require('node:module')
const path = require('node:path')
const { spawn } = require('node:child_process')

const applicationModule = process.argv[2]
const bootstrapDll = process.argv[3]
const runtimeModule = process.argv[4] ?? path.resolve(__dirname, '../dist/index.js')
const startMode = process.argv[5] ?? 'direct'
if (!applicationModule || !bootstrapDll) {
  throw new Error(
    'Usage: node dispatcher-queue-winui-child.cjs <Application.js> <Microsoft.WindowsAppRuntime.Bootstrap.dll> [runtime index.js] [direct|scheduled]',
  )
}
if (!fs.existsSync(applicationModule)) {
  throw new Error(`Generated Application binding was not found: ${applicationModule}`)
}
if (!fs.existsSync(bootstrapDll)) {
  throw new Error(`Windows App SDK bootstrap DLL was not found: ${bootstrapDll}`)
}

function startDelayedServer() {
  const script = `
    const http = require('node:http')
    const server = http.createServer((_request, response) => {
      setTimeout(() => {
        response.end('dispatcher-queue-ok')
        server.close()
      }, 3000)
    })
    server.listen(0, '127.0.0.1', () => process.stdout.write(String(server.address().port) + '\\n'))
  `
  const child = spawn(process.execPath, ['-e', script], {
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  })
  return new Promise((resolve, reject) => {
    let stdout = ''
    let stderr = ''
    child.stdout.setEncoding('utf8')
    child.stderr.setEncoding('utf8')
    child.stdout.on('data', (chunk) => {
      stdout += chunk
      const newline = stdout.indexOf('\n')
      if (newline !== -1) {
        resolve({ child, port: Number(stdout.slice(0, newline)) })
      }
    })
    child.stderr.on('data', (chunk) => {
      stderr += chunk
    })
    child.once('error', reject)
    child.once('exit', (code) => {
      if (!stdout.includes('\n') && code !== null) {
        reject(new Error(`Delayed HTTP server exited with code ${code}: ${stderr}`))
      }
    })
  })
}

function createUri(runtime, value) {
  const factoryIid = runtime.WinGuid.parse('44a9796f-723e-4fdf-a218-033e75b0c084')
  const factoryType = runtime.DynWinRtType.registerInterface(
    'IUriRuntimeClassFactoryWinuiAsyncTest',
    factoryIid,
  ).addMethod(
    'CreateUri',
    new runtime.DynWinRtMethodSig().addIn(runtime.DynWinRtType.hstring()).addOut(runtime.DynWinRtType.object()),
  )
  return factoryType
    .methodByName('CreateUri')
    .invoke(runtime.DynWinRtValue.activationFactory('Windows.Foundation.Uri').cast(factoryIid), [
      runtime.DynWinRtValue.hstring(value),
    ])
}

function createDelayedHttpOperation(runtime, uri) {
  const activationFactoryIid = runtime.WinGuid.parse('00000035-0000-0000-c000-000000000046')
  const activationFactoryType = runtime.DynWinRtType.registerInterface(
    'IActivationFactoryWinuiAsyncTest',
    activationFactoryIid,
  ).addMethod('ActivateInstance', new runtime.DynWinRtMethodSig().addOut(runtime.DynWinRtType.object()))
  const client = activationFactoryType
    .methodByName('ActivateInstance')
    .invoke(runtime.DynWinRtValue.activationFactory('Windows.Web.Http.HttpClient'), [])

  const clientIid = runtime.WinGuid.parse('7fda1151-3574-4880-a8ba-e6b1e0061f3d')
  const clientType = runtime.DynWinRtType.registerInterface('IHttpClientWinuiAsyncTest', clientIid)
  for (const name of ['DeleteAsync', 'GetAsync', 'GetWithOptionAsync', 'GetBufferAsync', 'GetInputStreamAsync']) {
    clientType.addMethod(name, new runtime.DynWinRtMethodSig())
  }
  const progressStage = runtime.DynWinRtType.enumType('Windows.Web.Http.HttpProgressStage')
  const referenceU64 = runtime.DynWinRtType.parameterized(
    runtime.WinGuid.parse('61c17706-2d65-11e0-9ae8-d48564015472'),
    [runtime.DynWinRtType.u64()],
  )
  const httpProgress = runtime.DynWinRtType.structType('Windows.Web.Http.HttpProgress', [
    progressStage,
    runtime.DynWinRtType.u64(),
    referenceU64,
    runtime.DynWinRtType.u64(),
    referenceU64,
    runtime.DynWinRtType.u32(),
  ])
  clientType.addMethod(
    'GetStringAsync',
    new runtime.DynWinRtMethodSig()
      .addIn(runtime.DynWinRtType.object())
      .addOut(runtime.DynWinRtType.iAsyncOperationWithProgress(runtime.DynWinRtType.hstring(), httpProgress)),
  )
  return clientType.methodByName('GetStringAsync').invoke(client.cast(clientIid), [uri])
}

function startExitTimer(runtime, delayMs, callback) {
  const queueIid = runtime.WinGuid.parse('603e88e4-a338-4ffe-a457-a5cfb9ceb899')
  const timerIid = runtime.WinGuid.parse('5feabb1d-a31c-4727-b1ac-37454649d56a')
  const queueType = runtime.DynWinRtType.runtimeClass('Windows.System.DispatcherQueue', queueIid)
  const timerType = runtime.DynWinRtType.runtimeClass('Windows.System.DispatcherQueueTimer', timerIid)
  const staticsIid = runtime.WinGuid.parse('a96d83d7-9371-4517-9245-d0824ac12c74')
  const staticsType = runtime.DynWinRtType.registerInterface(
    'IDispatcherQueueStaticsWinuiAsyncTest',
    staticsIid,
  ).addMethod('GetForCurrentThread', new runtime.DynWinRtMethodSig().addOut(queueType))
  const queue = staticsType
    .methodByName('GetForCurrentThread')
    .invoke(runtime.DynWinRtValue.activationFactory('Windows.System.DispatcherQueue').cast(staticsIid), [])
  const queueInterface = runtime.DynWinRtType.registerInterface('IDispatcherQueueWinuiAsyncTest', queueIid).addMethod(
    'CreateTimer',
    new runtime.DynWinRtMethodSig().addOut(timerType),
  )
  const timer = queueInterface.methodByName('CreateTimer').invoke(queue.cast(queueIid), [])

  const timeSpanType = runtime.DynWinRtType.structType('Windows.Foundation.TimeSpan', [runtime.DynWinRtType.i64()])
  const interval = runtime.DynWinRtStruct.create(timeSpanType)
  interval.setI64(0, BigInt(delayMs) * 10_000n)
  const typedEventHandlerType = runtime.DynWinRtType.parameterized(
    runtime.WinGuid.parse('9de1c534-6ae1-11e0-84e1-18a905bcc53f'),
    [timerType, runtime.DynWinRtType.object()],
  )
  const tickHandler = runtime.DynWinRtDelegate.create(
    typedEventHandlerType.iid(),
    [timerType, runtime.DynWinRtType.object()],
    callback,
  )
  const timerInterface = runtime.DynWinRtType.registerInterface('IDispatcherQueueTimerWinuiAsyncTest', timerIid)
    .addMethod('get_Interval', new runtime.DynWinRtMethodSig().addOut(timeSpanType))
    .addMethod('put_Interval', new runtime.DynWinRtMethodSig().addIn(timeSpanType))
    .addMethod('get_IsRunning', new runtime.DynWinRtMethodSig().addOut(runtime.DynWinRtType.boolType()))
    .addMethod('get_IsRepeating', new runtime.DynWinRtMethodSig().addOut(runtime.DynWinRtType.boolType()))
    .addMethod('put_IsRepeating', new runtime.DynWinRtMethodSig().addIn(runtime.DynWinRtType.boolType()))
    .addMethod('Start', new runtime.DynWinRtMethodSig())
    .addMethod('Stop', new runtime.DynWinRtMethodSig())
    .addMethod(
      'add_Tick',
      new runtime.DynWinRtMethodSig().addIn(typedEventHandlerType).addOut(runtime.DynWinRtType.i64()),
    )
  timerInterface.methodByName('put_Interval').invoke(timer.cast(timerIid), [interval.toValue()])
  timerInterface.methodByName('put_IsRepeating').invoke(timer.cast(timerIid), [runtime.DynWinRtValue.boolValue(false)])
  timerInterface.methodByName('add_Tick').invoke(timer.cast(timerIid), [tickHandler.toValue()])
  timerInterface.methodByName('Start').invoke(timer.cast(timerIid), [])
  return { tickHandler, timer }
}

function scheduleApplicationStart(runtime, callback) {
  const callbackIid = runtime.WinGuid.parse('d8eef1c9-1234-56f1-9963-45dd9c80a661')
  const callbackParam = runtime.DynWinRtType.runtimeClass(
    'Microsoft.UI.Xaml.ApplicationInitializationCallbackParams',
    runtime.WinGuid.parse('1b1906ea-5b7b-5876-81ab-7c2281ac3d20'),
  )
  const callbackDelegate = runtime.DynWinRtDelegate.create(callbackIid, [callbackParam], callback).toValue()
  const staticsIid = runtime.WinGuid.parse('4e0d09f5-4358-512c-a987-503b52848e95')
  const applicationType = runtime.DynWinRtType.runtimeClass(
    'Microsoft.UI.Xaml.Application',
    runtime.WinGuid.parse('06a8f4e7-1146-55af-820d-ebd55643b021'),
  )
  const staticsType = runtime.DynWinRtType.registerInterface('IApplicationStaticsScheduledTest', staticsIid)
    .addMethod('get_Current', new runtime.DynWinRtMethodSig().addOut(applicationType))
    .addMethod('Start', new runtime.DynWinRtMethodSig().addIn(runtime.DynWinRtType.interface(callbackIid)))
  const statics = runtime.DynWinRtValue.activationFactory('Microsoft.UI.Xaml.Application').cast(staticsIid)
  return staticsType.method(7).invokeScheduled(statics, [callbackDelegate])
}

async function main() {
  const server = await startDelayedServer()
  process.env.WINAPPSDK_BOOTSTRAP_DLL_PATH = bootstrapDll
  const runtime = require(path.resolve(runtimeModule))
  runtime.initWinappsdk(2, 2)
  runtime.roInitialize(0)

  const originalLoad = Module._load
  Module._load = function loadWithLocalDynwinrt(request, parent, isMain) {
    if (request === '@microsoft/dynwinrt') {
      return runtime
    }
    return originalLoad.call(this, request, parent, isMain)
  }

  const { Application } = require(path.resolve(applicationModule))
  const delayedOperation = createDelayedHttpOperation(runtime, createUri(runtime, `http://127.0.0.1:${server.port}/`))

  let insideStart = false
  const startedAt = performance.now()
  const order = []
  let reactionElapsed
  let reactionInsideStart = false
  let nextTickElapsed
  let timerElapsed
  let dispatcherSettlementCount = 0
  let exitTimer
  let app
  let applicationStartPromise
  const dispatcherPromise = delayedOperation.toPromise().then((result) => {
    reactionElapsed = performance.now() - startedAt
    reactionInsideStart = insideStart
    order.push('promise')
    dispatcherSettlementCount += 1
    assert.equal(result.toString(), 'dispatcher-queue-ok')
  })

  insideStart = true
  const initializeApplication = () => {
    if (startMode === 'scheduled') {
      process.nextTick(() => {
        nextTickElapsed = performance.now() - startedAt
        order.push('nextTick')
      })
    }
    app = Application.create(() => {
      exitTimer = startExitTimer(runtime, 5000, () => {
        timerElapsed = performance.now() - startedAt
        order.push('timer')
        assert.equal(dispatcherSettlementCount, 1)
        assert.equal(reactionInsideStart, true)
        assert.ok(reactionElapsed < 4500, `Promise reaction was delayed ${reactionElapsed}ms`)
        assert.ok(reactionElapsed < timerElapsed - 500)
        if (startMode === 'scheduled') {
          assert.ok(nextTickElapsed < 1000, `nextTick was delayed ${nextTickElapsed}ms`)
          assert.deepEqual(order, ['nextTick', 'promise', 'timer'])
        } else {
          assert.deepEqual(order, ['promise', 'timer'])
        }
        Application.current.exit()
      })
    })
  }
  if (startMode === 'scheduled') {
    applicationStartPromise = scheduleApplicationStart(runtime, initializeApplication)
  } else {
    Application.start(initializeApplication)
    insideStart = false
  }
  await dispatcherPromise
  await new Promise((resolve) => setImmediate(resolve))
  await applicationStartPromise
  insideStart = false
  assert.ok(app)
  assert.ok(exitTimer)
  assert.equal(dispatcherSettlementCount, 1)
  if (server.child.exitCode === null && server.child.signalCode === null) {
    server.child.kill()
  }
  console.log(
    `dispatcher-queue-winui-ok ${JSON.stringify({
      dispatcherSettlementCount,
      nextTickElapsed,
      reactionElapsed,
      timerElapsed,
    })}`,
  )
  process.exit(0)
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
