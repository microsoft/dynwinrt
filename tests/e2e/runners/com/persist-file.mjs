// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict';
import { DynCom } from '../../../../bindings/js/dist/com-unsafe.js';
import {
  IID_IPersistFile,
  IPersistFile,
} from '../../e2e_generated/com/shell/com/IPersistFile.js';

const CLSID_SHELL_LINK = '00021401-0000-0000-c000-000000000046';
DynCom.initialize(1);

const persist = IPersistFile._fromNative(
  DynCom.coCreateInstance(CLSID_SHELL_LINK, IID_IPersistFile),
);
assert.equal(persist.getClassID().toLowerCase(), CLSID_SHELL_LINK);
persist.release();

console.log('persist-file ok');
