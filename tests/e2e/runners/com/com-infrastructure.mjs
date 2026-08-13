// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict';
import { DynCom } from '../../../../bindings/js/dist/com-unsafe.js';
import {
  IClassFactory,
  ICreateErrorInfo,
  IErrorInfo,
  IID_IClassFactory,
  IID_IErrorInfo,
  IMalloc,
} from '../../e2e_generated/com/infrastructure/com/index.mjs';
import {
  IID_ITaskbarList3,
  ITaskbarList3,
} from '../../e2e_generated/com/shell/com/index.mjs';

const CLSID_TASKBAR_LIST = '56fdf344-fd6d-11d0-958a-006097c9a090';
const ERROR_GUID = '12345678-1234-5678-90ab-1234567890ab';

DynCom.initialize(1);

const allocatorValue = DynCom.coGetMalloc();
const allocator = IMalloc._fromNative(allocatorValue);
allocatorValue.release();
let allocation = allocator.alloc(32n);
assert.ok(allocation);
assert.ok(allocator.getSize(allocation) >= 32n);
assert.notEqual(allocator.didAlloc(allocation), 0);
const originalAllocation = allocation;
allocation = allocator.realloc(originalAllocation, 64);
assert.ok(allocation);
assert.equal(originalAllocation.released, true);
assert.ok(allocator.getSize(allocation) >= 64n);
allocator.free(allocation);
assert.equal(allocation.released, true);
allocator.heapMinimize();
allocator.release();

const factoryValue = DynCom.coGetClassObject(
  CLSID_TASKBAR_LIST,
  IID_IClassFactory,
);
const factory = IClassFactory._fromNative(factoryValue);
factoryValue.release();
factory.lockServer(true);
try {
  const taskbarValue = factory.createInstance(
    null,
    IID_ITaskbarList3.toString(),
  );
  const taskbar = ITaskbarList3._fromNative(taskbarValue);
  taskbarValue.release();
  taskbar.hrInit();
  taskbar.release();
} finally {
  factory.lockServer(false);
  factory.release();
}

const createValue = DynCom.createErrorInfo();
const create = ICreateErrorInfo._fromNative(createValue);
createValue.release();
create.setGUID(ERROR_GUID);
create.setSource('dynwinrt');
create.setDescription('generated COM error info');
create.setHelpFile('dynwinrt-help.chm');
create.setHelpContext(42);
DynCom.setErrorInfo(create._obj);
create.release();

const errorValue = DynCom.getErrorInfo();
assert.ok(errorValue);
const error = IErrorInfo._fromNative(errorValue);
errorValue.release();
assert.equal(error.getGUID().toLowerCase(), ERROR_GUID);
assert.equal(error.getSource(), 'dynwinrt');
assert.equal(error.getDescription(), 'generated COM error info');
assert.equal(error.getHelpFile(), 'dynwinrt-help.chm');
assert.equal(error.getHelpContext(), 42);
error.release();
assert.equal(DynCom.getErrorInfo(), null);

console.log('com-infrastructure ok');
