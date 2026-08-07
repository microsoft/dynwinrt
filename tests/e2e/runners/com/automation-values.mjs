// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from "node:assert/strict";
import {
  DynComPropVariant,
  DynComSafeArray,
  DynComVariant,
  WinGuid,
  initializeCom,
} from "../../../../bindings/js/dist/com.js";

initializeCom(1);

const variant = DynComVariant.bstr("embedded\0automation");
const array = DynComSafeArray.variant(
  [DynComVariant.i32(7), variant],
  [{ lowerBound: -1, length: 2 }],
);
const property = DynComPropVariant.guid(
  WinGuid.parse("00000000-0000-0000-c000-000000000046"),
);

try {
  assert.equal(variant.kind, "bstr");
  assert.equal(variant.toStringValue(), "embedded\0automation");
  assert.equal(array.elementType, "variant");
  assert.deepEqual(array.bounds, [{ lowerBound: -1, length: 2 }]);
  const values = array.toVariants();
  try {
    assert.equal(values[0].toNumber(), 7);
    assert.equal(values[1].toStringValue(), "embedded\0automation");
  } finally {
    for (const value of values) value.release();
  }
  assert.equal(property.kind, "guid");
} finally {
  property.release();
  array.release();
  variant.release();
}

console.log("automation-values ok");
