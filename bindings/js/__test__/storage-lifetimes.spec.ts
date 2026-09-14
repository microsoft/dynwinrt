// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'
import { execFileSync } from 'node:child_process'
import { join } from 'node:path'
import { DynWinRtMethodSig, DynWinRtType, DynWinRtValue, WinGuid, roInitialize } from '../dist/winrt.js'
import { DynCom } from '../dist/com-unsafe.js'
import { DynWin32, DynWin32Unsafe } from '../dist/win32-unsafe.js'

test('shared JS storage preserves byte-view offsets and WinRT copy isolation', (t) => {
  roInitialize(1)
  const bytes = new Uint8Array([0, 1, 2, 3, 4, 5, 6, 7])
  const view = bytes.subarray(2, 6)
  const base = DynCom.safeDataPointer(bytes)
  const pointer = DynCom.safeDataPointer(view)
  const counted = DynCom.buffer(view)
  const win32 = DynWin32.dataPointer(view)
  const winrt = DynWinRtValue.fromBuffer(view)
  t.teardown(() => {
    base.release()
    pointer.release()
    counted.release()
    winrt.release()
  })

  const address = DynCom.asPointerBigint(base) + 2n
  t.is(DynCom.asPointerBigint(pointer), address)
  t.is(DynWin32Unsafe.pointerAddress(win32), address)
  t.is(DynCom.bufferCount(counted), 4n)
  t.deepEqual(winrt.toBuffer(), Buffer.from([2, 3, 4, 5]))
  view.fill(9)
  t.deepEqual(DynWin32.copyBuffer(view), Buffer.from([9, 9, 9, 9]))
  t.deepEqual(winrt.toBuffer(), Buffer.from([2, 3, 4, 5]))
})

test('shared JS storage preserves domain-specific detachment errors and validation order', (t) => {
  roInitialize(1)
  const view = new Uint8Array(16)
  const pointer = DynCom.safeDataPointer(view)
  const counted = DynCom.buffer(view)
  const win32 = DynWin32.dataPointer(view)
  const copied = DynWinRtValue.fromBuffer(view)
  const factoryIid = WinGuid.parse('44a9796f-723e-4fdf-a218-033e75b0c084')
  const factoryType = DynWinRtType.registerInterface('StorageTest.IUriRuntimeClassFactory', factoryIid).addMethod(
    'CreateUri',
    new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.object()),
  )
  const activation = DynWinRtValue.activationFactory('Windows.Foundation.Uri')
  const factory = activation.cast(factoryIid)
  t.teardown(() => {
    pointer.release()
    counted.release()
    copied.release()
    factory.release()
    activation.release()
  })
  structuredClone(view.buffer, { transfer: [view.buffer] })

  const pointerError = 'Cannot use a pointer whose TypedArray backing ArrayBuffer is detached'
  t.throws(() => DynCom.asPointerBigint(pointer), { message: pointerError })
  t.throws(() => DynWin32Unsafe.pointerAddress(win32), { message: pointerError })
  t.throws(() => DynCom.asPointerBigint(counted), {
    message: 'Cannot use a COM buffer whose backing ArrayBuffer is detached',
  })
  t.throws(() => DynWin32.bufferLength(view), { message: 'Cannot use a detached Win32 Buffer/Uint8Array' })
  // Storage admission precedes WinRT signature validation; this must never dispatch.
  for (const invoke of ['invoke', 'invokeAll'] as const) {
    t.throws(() => factoryType.method(6)[invoke](factory, [pointer]), { message: pointerError })
  }
  t.deepEqual(copied.toBuffer(), Buffer.alloc(16))
})

test('shared JS storage rejects resized views without rejecting unchanged fixed views', (t) => {
  const backing = new ArrayBuffer(16, { maxByteLength: 64 })
  const tracking = new Uint8Array(backing)
  const fixed = new Uint8Array(backing, 4, 8)
  const pointer = DynCom.safeDataPointer(tracking)
  const counted = DynCom.buffer(tracking)
  const win32 = DynWin32.dataPointer(tracking)
  const fixedPointer = DynCom.safeDataPointer(fixed)
  const fixedCounted = DynCom.buffer(fixed)
  const originalAddress = DynCom.asPointerBigint(fixedPointer)
  t.teardown(() => {
    for (const value of [pointer, counted, fixedPointer, fixedCounted]) value.release()
  })

  backing.resize(32)
  t.throws(() => DynCom.asPointerBigint(pointer), { message: /backing storage changed/ })
  t.throws(() => DynCom.asPointerBigint(counted), { message: /backing storage changed/ })
  t.throws(() => DynWin32Unsafe.pointerAddress(win32), { message: /backing storage changed/ })
  const current = DynCom.safeDataPointer(fixed)
  t.teardown(() => current.release())
  if (DynCom.asPointerBigint(current) === originalAddress) {
    t.is(DynCom.asPointerBigint(fixedPointer), originalAddress)
    t.is(DynCom.asPointerBigint(fixedCounted), 0n)
  } else {
    t.throws(() => DynCom.asPointerBigint(fixedPointer), { message: /backing storage changed/ })
    t.throws(() => DynCom.asPointerBigint(fixedCounted), { message: /backing storage changed/ })
  }
  backing.resize(4)
  t.throws(() => DynCom.asPointerBigint(fixedPointer), { message: /backing storage changed/ })
  t.throws(() => DynCom.asPointerBigint(fixedCounted), { message: /backing storage changed/ })
})

test('shared JS storage keeps the existing view-kind and shared-backing boundaries', (t) => {
  const wide = new Uint16Array(4)
  const counted = DynCom.buffer(wide)
  t.teardown(() => counted.release())
  t.is(DynCom.bufferCount(counted), 4n)
  for (const invalid of [wide, new DataView(new ArrayBuffer(8))]) {
    t.throws(() => Reflect.apply(DynWin32.dataPointer, DynWin32, [invalid]))
    t.throws(() => Reflect.apply(DynCom.safeDataPointer, DynCom, [invalid]))
  }
  const shared = new Uint8Array(new SharedArrayBuffer(8))
  t.throws(() => DynCom.buffer(shared), {
    message: 'SharedArrayBuffer-backed views cannot be passed to native COM calls',
  })
  t.throws(() => DynWin32.dataPointer(shared), { message: 'SharedArrayBuffer storage is unsupported for Win32' })
})

test('shared JS storage retains both JS owner strategies across collection', (t) => {
  const output = execFileSync(process.execPath, ['--expose-gc', '-e', `
    const assert = require('node:assert/strict');
    const { DynCom } = require(${JSON.stringify(join(process.cwd(), 'dist', 'com-unsafe.js'))});
    const { DynWin32, DynWin32Unsafe } = require(${JSON.stringify(join(process.cwd(), 'dist', 'win32-unsafe.js'))});
    function retain() {
      const bytes = new Uint8Array([1, 2, 3, 4]);
      const wide = new Uint16Array([5, 6]);
      const win32 = new Uint8Array([7, 8]);
      const pointer = DynCom.safeDataPointer(bytes);
      return {
        pointer, address: DynCom.asPointerBigint(pointer), counted: DynCom.buffer(wide),
        win32: DynWin32.dataPointer(win32),
        weak: [new WeakRef(bytes), new WeakRef(wide), new WeakRef(win32)],
      };
    }
    (async () => {
      const retained = retain();
      for (let index = 0; index < 4; index++) {
        await new Promise(setImmediate);
        global.gc();
      }
      assert(retained.weak.every(value => value.deref() !== undefined));
      assert.equal(DynCom.asPointerBigint(retained.pointer), retained.address);
      assert.equal(DynCom.asPointerBigint(retained.counted), 0n);
      assert.notEqual(DynWin32Unsafe.pointerAddress(retained.win32), 0n);
      retained.pointer.release();
      retained.counted.release();
      process.stdout.write('retained');
    })().catch(error => { console.error(error); process.exitCode = 1; });
  `], { encoding: 'utf8', timeout: 30_000 })
  t.is(output, 'retained')
})
