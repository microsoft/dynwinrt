// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict';
import { DynCom } from '../../../../bindings/js/dist/com-unsafe.js';
import { projectAs } from '../../../../bindings/js/dist/com.js';
import {
  IID_IPersistFile,
  IPersistFile,
} from '../../e2e_generated/com/shell/com/windows/win32/system/com/IPersistFile.js';

const CLSID_SHELL_LINK = '00021401-0000-0000-c000-000000000046';
DynCom.initialize(1);

const native = DynCom.coCreateInstance(CLSID_SHELL_LINK, IID_IPersistFile);
let persist;
let second;
try {
  persist = projectAs(native, IPersistFile);
  second = projectAs(persist, IPersistFile);
  native.release();
  assert.equal(persist.getClassID().toLowerCase(), CLSID_SHELL_LINK);
  persist.release();
  assert.equal(second.getClassID().toLowerCase(), CLSID_SHELL_LINK);
  assert.throws(() => projectAs(0n, IPersistFile), /managed COM/);
  assert.throws(() => projectAs(Buffer.alloc(8), IPersistFile), /managed COM/);
  assert.throws(() => projectAs(second, { IID: IPersistFile.IID }), /registered generated safe COM/);
} finally {
  persist?.release();
  second?.release();
  native.release();
}

console.log('persist-file ok');
