// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

export function registerTestComSinkInterface(runtime, withOutput = false) {
  const iid = runtime.WinGuid.parse('7ac2eaa2-97a4-43f0-9b0f-421c2363ef11')
  const interfaceType = runtime.DynCom.interfaceType(runtime.WinGuid.parse('00000000-0000-0000-c000-000000000046'))
  let signature = new runtime.DynComMethodSig().addIn(interfaceType)
  if (withOutput) {
    signature = signature.addIn(interfaceType).addOut(runtime.DynCom.i32Type())
  }
  return runtime.DynCom.registerIUnknownInterface('Tests.IComSink', iid).addMethodAt(3, 'Invoke', signature)
}

export function registerTestComI32SinkInterface(runtime) {
  const iid = runtime.WinGuid.parse('7ac2eaa2-97a4-43f0-9b0f-421c2363ef11')
  const signature = new runtime.DynComMethodSig().addIn(runtime.DynCom.i32Type())
  return runtime.DynCom.registerIUnknownInterface('Tests.IComI32Sink', iid).addMethodAt(3, 'Invoke', signature)
}
