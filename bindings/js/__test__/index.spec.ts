// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'

import {
  DynWinRtArray,
  DynWinRtMethodSig,
  DynWinRtType,
  DynWinRtValue,
  WinGuid,
  getComputerName,
  getWindowsDirectory,
  hasPackageIdentity,
  roInitialize,
} from '../dist/index.js'

test('getComputerName', (t) => {
  const name = getComputerName()
  t.truthy(name)
  t.is(typeof name, 'string')
})

test('getWindowsDirectory', (t) => {
  const dir = getWindowsDirectory()
  t.truthy(dir)
  t.is(typeof dir, 'string')
})

test('build WinRT types', (t) => {
  t.truthy(DynWinRtType.i32())
  t.truthy(DynWinRtType.hstring())
  t.truthy(DynWinRtType.object())
  t.truthy(DynWinRtType.iAsyncOperation(DynWinRtType.object()))
})

test('invoke WinRT through dynamic metadata', (t) => {
  roInitialize(1)
  const factoryIid = WinGuid.parse('44a9796f-723e-4fdf-a218-033e75b0c084')
  const uriIid = WinGuid.parse('9e365e57-48b2-4160-956f-c7385120bbfc')
  const uriFactoryType = DynWinRtType.registerInterface('IUriRuntimeClassFactory', factoryIid).addMethod(
    'CreateUri',
    new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.object()),
  )
  const uriType = DynWinRtType.registerInterface('IUriRuntimeClass', uriIid).addMethod(
    'get_AbsoluteUri',
    new DynWinRtMethodSig().addOut(DynWinRtType.hstring()),
  )
  const uriFactory = DynWinRtValue.activationFactory('Windows.Foundation.Uri').cast(factoryIid)
  const expected = 'https://www.example.com/path?q=1#frag'
  const uri = uriFactoryType
    .methodByName('CreateUri')
    .invoke(uriFactory, [DynWinRtValue.hstring(expected)])
    .cast(uriIid)

  t.is(uriType.methodByName('get_AbsoluteUri').invoke(uri, []).toString(), expected)
})

test('round-trip WinRT values', (t) => {
  t.is(DynWinRtValue.i32(42).toNumber(), 42)
  t.is(DynWinRtValue.hstring('hello').toString(), 'hello')
  t.true(DynWinRtValue.nullValue().isNull())
})

test('release WinRT object values deterministically', (t) => {
  const value = DynWinRtValue.activationFactory('Windows.Foundation.Uri')
  value.release()
  t.true(value.isNull())
  t.notThrows(() => value.release())
})

test('round-trip WinRT value arrays', (t) => {
  const array = DynWinRtArray.fromI32Values([1, 2, 3])
  t.is(array.len(), 3)
  t.deepEqual(
    array.toValues().map((value) => value.toNumber()),
    [1, 2, 3],
  )
})

test('parse GUIDs', (t) => {
  const iid = '00000000-0000-0000-c000-000000000046'
  t.is(WinGuid.parse(iid).toString().toLowerCase(), iid)
  t.throws(() => WinGuid.parse('not-a-guid'))
})

test('report package identity state', (t) => {
  t.is(typeof hasPackageIdentity(), 'boolean')
})
