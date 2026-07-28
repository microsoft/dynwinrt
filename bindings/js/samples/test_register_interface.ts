// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/**
 * Test: registerInterface + addMethod + method/methodByName + invoke
 * Uses Windows.Foundation.Uri (no WinAppSDK needed)
 */
import {
  DynWinRtValue,
  DynWinRtType,
  DynWinRtMethodSig,
  WinGuid,
  roInitialize,
} from '../dist/winrt.js'

roInitialize(1)

// Register IUriRuntimeClassFactory
const factoryIid = WinGuid.parse('44A9796F-723E-4FDF-A218-033E75B0C084')
const iUriFactory = DynWinRtType.registerInterface("IUriRuntimeClassFactory", factoryIid)
    .addMethod("CreateUri", new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.object()))

// Register IUriRuntimeClass
const uriIid = WinGuid.parse('9E365E57-48B2-4160-956F-C7385120BBFC')
const iUri = DynWinRtType.registerInterface("IUriRuntimeClass", uriIid)
    .addMethod("get_AbsoluteUri", new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))  // vtable 6
    .addMethod("get_DisplayUri",  new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))  // vtable 7
    .addMethod("get_Domain",      new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))  // vtable 8
    .addMethod("get_Extension",   new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))  // vtable 9
    .addMethod("get_Fragment",    new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))  // vtable 10
    .addMethod("get_Host",        new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))  // vtable 11

// Create Uri
const factory = DynWinRtValue.activationFactory('Windows.Foundation.Uri')
const uriFactory = factory.cast(factoryIid)

// invoke by name
const uri = iUriFactory.methodByName("CreateUri").invoke(uriFactory, [
    DynWinRtValue.hstring("https://www.example.com/path?q=1#frag")
])

const uriObj = uri.cast(uriIid)

// invoke by vtable index
const absUri = iUri.method(6).invoke(uriObj, [])
console.log("AbsoluteUri:", absUri.toString())
console.assert(absUri.toString() === "https://www.example.com/path?q=1#frag")

// invoke by name
const host = iUri.methodByName("get_Host").invoke(uriObj, [])
console.log("Host:", host.toString())
console.assert(host.toString() === "www.example.com")

const domain = iUri.methodByName("get_Domain").invoke(uriObj, [])
console.log("Domain:", domain.toString())

console.log("\n=== registerInterface + invoke test PASSED ===")
