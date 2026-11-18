// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

'use strict'

// A native IAsyncAction backed by an owner-thread WinRT implementation. A
// foreign GetResults returns RPC_E_WRONG_THREAD before entering JavaScript.
module.exports = function createOwnerAction(runtime, { completed = false, fail = false } = {}) {
  const { DynWinRtType: T, DynWinRtMethodSig: Sig, DynWinRtValue: V, WinGuid } = runtime
  const action = T.iAsyncAction()
  const infoIid = WinGuid.parse('00000036-0000-0000-c000-000000000046')
  const handlerIid = WinGuid.parse('a4ed5c81-76c9-40bd-8be6-b1d90fb20ae7')
  const handlerType = T.delegate(handlerIid)
  const statusType = T.enumType('Windows.Foundation.AsyncStatus')
  const invoke = new Sig().addIn(action).addIn(statusType)
  const plan = (name, iid, methods, requires = []) =>
    runtime.DynWinRtInterfacePlan.create(
      name,
      T.interface(iid),
      methods.map(([name, signature], index) => ({ name, signature, vtableIndex: 6 + index })),
      requires,
    )
  const actionPlan = plan(
    'IAsyncAction',
    action.iid(),
    [
      ['put_Completed', new Sig().addIn(handlerType)],
      ['get_Completed', new Sig().addOut(handlerType)],
      ['GetResults', new Sig()],
    ],
    [infoIid],
  )
  const infoPlan = plan('IAsyncInfo', infoIid, [
    ['get_Id', new Sig().addOut(T.u32())],
    ['get_Status', new Sig().addOut(statusType)],
    ['get_ErrorCode', new Sig().addOut(T.hresult())],
    ['Cancel', new Sig()],
    ['Close', new Sig()],
  ])
  const sourceIid = WinGuid.parse('970dd88b-f056-4a63-a5a2-f41236e11215')
  const start = new Sig().addOut(action)
  const source = T.registerInterface('ITestActionSource', sourceIid).addMethod('Start', start)
  const sourcePlan = plan('ITestActionSource', sourceIid, [['Start', start]])
  let handler
  let resultCalls = 0
  const status = () => V.enumValue(statusType, completed ? (fail ? 3 : 1) : 0)
  const notify = () => handler.invokeDelegate(handlerIid, invoke, [operation, status()])
  const owner = runtime.DynWinRtImplementation.create([actionPlan, infoPlan, sourcePlan], (iface, slot, args) => {
    if (iface === 2) return [identity]
    if (iface === 0) {
      if (slot === 6) {
        handler = args[0]
        if (completed) notify()
      } else if (slot === 7) {
        return [handler ?? V.nullValue()]
      } else {
        resultCalls++
        if (!completed || fail) throw new Error('owner action GetResults failed')
      }
    } else {
      if (slot === 6) return [V.u32(1)]
      if (slot === 7) return [status()]
      if (slot === 8) return [V.hresult(completed && fail ? 0x80004005 | 0 : 0)]
    }
    return []
  })
  const identity = owner.toValue()
  const sourceView = identity.cast(sourceIid)
  const operation = source.method(6).invoke(sourceView, [])
  sourceView.release()
  return {
    operation,
    finish() {
      completed = true
      notify()
    },
    get resultCalls() {
      return resultCalls
    },
    dispose() {
      owner.dispose()
      handler?.release()
      identity.release()
      operation.release()
    },
  }
}
