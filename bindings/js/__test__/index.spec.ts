// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'

import {
  useDynwinrtAdd,
  getComputerName,
  getWindowsDirectory,
  ComUri,
  getUriVtable,
  callMethod,
  httpClientGetSync,
  asyncProgressHstringToPromiseString,
  initWinappsdk,
  roInitialize,
  DynWinRtValue,
  WinIIds,
  DynWinRtType,
} from '../dist/index.js'

test('sync function from native code', (t) => {
  t.is(useDynwinrtAdd(40, 2), 42)
  t.is(useDynwinrtAdd(5, 3), 8)
})

test('getComputerName', (t) => {
  const name = getComputerName()
  console.log('Computer name:', name)
  t.truthy(name)
  t.is(typeof name, 'string')
})

test('getWindowsDirectory', (t) => {
  const dir = getWindowsDirectory()
  console.log('Windows directory:', dir)
  t.truthy(dir)
  t.is(typeof dir, 'string')
})

test('WinRT URI and VTable', (t) => {
  const uri = ComUri.createTestUri('https://www.example.com/path?query=1#fragment')
  t.truthy(uri)

  const vtable = getUriVtable()
  t.truthy(vtable)

  const runtimeClassName = callMethod(vtable, 4, uri)
  console.log('RuntimeClassName', runtimeClassName)
  t.is(runtimeClassName, 'Windows.Foundation.Uri')

  const domain = callMethod(vtable, 13, uri)
  console.log('Domain (index 13)', domain)
  // Index 13 appears to be Path in this environment
  t.is(domain, '/path')

  const absoluteUri = callMethod(vtable, 17, uri)
  console.log('AbsoluteUri (index 17)', absoluteUri)
  t.is(absoluteUri, 'https')
})

test('WinRTType build', (t) => {
  t.truthy(DynWinRtType.i32())
  t.truthy(DynWinRtType.hstring())
  t.truthy(DynWinRtType.object())
  t.truthy(DynWinRtType.iAsyncOperation(WinIIds.iAsyncOperationPickFileResultIid()))
})

test('HttpClient Sync', async (t) => {
  const asyncOperation = httpClientGetSync('https://www.microsoft.com')
  t.truthy(asyncOperation)

  const value = await asyncProgressHstringToPromiseString(asyncOperation)
  t.truthy(value)
  t.is(typeof value, 'string')
})

// test('run picker test in Rust', async (t) => {
//   await runTestPicker()
//   t.pass()
// })

test('file open picker', async (t) => {
  initWinappsdk(1, 8)
  roInitialize(1)
  const factory = DynWinRtValue.activationFactory('Microsoft.Windows.Storage.Pickers.FileOpenPicker')
  const FileOpenPicker = factory.cast(WinIIds.iFileOpenPickerFactoryIid())
  const picker = FileOpenPicker.callSingleOut1(
    6,
    DynWinRtType.object(),
    DynWinRtValue.i64(0))
  const picked_file = await picker.callSingleOut0(
    13,
    DynWinRtType.iAsyncOperation(WinIIds.iAsyncOperationPickFileResultIid()))
    .toPromise()
  const path = picked_file.callSingleOut0(6, DynWinRtType.hstring());
  console.log("Selected Path", path.toString())
  t.pass()
})
