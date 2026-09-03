// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict';
import {
  DynCom,
  DynComPropVariant,
} from '../../../../bindings/js/dist/com-unsafe.js';
import {
  createPROPERTYKEY,
  IID_IPropertyStore,
  IPropertyStore,
} from '../../e2e_generated/com/shell/com/windows/win32/ui/shell/properties-system/IPropertyStore.js';

const CLSID_SHELL_LINK = '00021401-0000-0000-c000-000000000046';
const VT_LPWSTR = 31;
const expected = 'Microsoft.DynWinRT.E2E.PropertyStore';
const key = createPROPERTYKEY(
  Buffer.from('55284c9f799f394ba8d0e1d42de1d5f305000000', 'hex'),
);

DynCom.initialize(1);

const native = DynCom.coCreateInstance(CLSID_SHELL_LINK, IID_IPropertyStore);
const store = IPropertyStore._fromNative(native);
native.release();

try {
  const input = DynComPropVariant.string(expected);
  try {
    store.setValue(key, input);
  } finally {
    input.release();
  }

  const output = store.getValue(key);
  try {
    assert.equal(output.vartype, VT_LPWSTR);
    assert.equal(output.kind, 'string');
    assert.equal(output.toStringValue(), expected);
  } finally {
    output.release();
  }

  store.commit();
} finally {
  store.release();
}

console.log('property-store ok');
