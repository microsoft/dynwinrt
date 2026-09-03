// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict';
import { DynCom, DynComMethodSig, WinGuid } from '../../../../bindings/js/dist/com-unsafe.js';
import {
  IID_IStream,
  IStream,
} from '../../e2e_generated/com/stream/com/windows/win32/system/com/IStream.js';
import {
  IID_IWICImagingFactory,
  IWICImagingFactory,
} from '../../e2e_generated/com/wic/com/windows/win32/graphics/imaging/IWICImagingFactory.js';

const CLSID_WIC_IMAGING_FACTORY = 'cacaf262-9370-4615-a13b-9f5539da4c0a';
const IID_IWIC_STREAM = WinGuid.parse('135ff860-22b7-4ddf-b0f6-218f4f299a43');

DynCom.initialize(1);

const factory = IWICImagingFactory._fromNative(
  DynCom.coCreateInstance(CLSID_WIC_IMAGING_FACTORY, IID_IWICImagingFactory),
);
const stream = factory.createStream();
// IWICStream inherits the complete IStream vtable; InitializeFromMemory is slot 16.
const wicStream = DynCom.registerIUnknownInterface(
  'Windows.Win32.Graphics.Imaging.IWICStream',
  IID_IWIC_STREAM,
).addMethodAt(
  16,
  'InitializeFromMemory',
  new DynComMethodSig().addIn(DynCom.pointerType()).addIn(DynCom.u32Type()),
);

const expected = Buffer.from('phase8 counted buffer', 'utf8');
wicStream
  .method(16)
  .invoke(stream, [DynCom.pointer(expected), DynCom.u32(expected.length)]);

const projectedStream = IStream._fromNative(stream.cast(IID_IStream));
const stat = projectedStream.stat(1);
assert.equal(stat.name, null);
assert.equal(stat.storageType, 2);
assert.equal(stat.size, BigInt(expected.length));
stat.release();

assert.equal(projectedStream.seek(0n, 0), 0n);
const [hresult, actual] = projectedStream.read(Buffer.alloc(expected.length));
assert.equal(hresult, 0);
assert.deepEqual(actual, expected);

assert.throws(() => projectedStream.clone(), /0x80004001/);

projectedStream.release();
stream.release();
factory.release();

console.log('istream-buffer ok');
