// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict';
import { DynCom } from '../../../../bindings/js/dist/com-unsafe.js';
import {
  createWIN32_FIND_DATAW,
  IID_IShellLinkW,
  IShellLinkW,
} from '../../e2e_generated/com/shell/com/windows/win32/ui/shell/IShellLinkW.js';

const CLSID_SHELL_LINK = '00021401-0000-0000-c000-000000000046';
const SLGP_RAWPATH = 4;

DynCom.initialize(1);

const link = IShellLinkW._fromNative(
  DynCom.coCreateInstance(CLSID_SHELL_LINK, IID_IShellLinkW),
);
const expected = process.execPath;
link.setPath(expected);

const findData = createWIN32_FIND_DATAW();
assert.equal(findData.length, 592);
assert.deepEqual(findData.bytes, Buffer.alloc(592));

const [actual, returnedFindData] = link.getPath(32768, findData, SLGP_RAWPATH);
assert.equal(actual.toLowerCase(), expected.toLowerCase());
assert.equal(returnedFindData.length, 592);

link.release();
console.log('shell-link-pod ok');
