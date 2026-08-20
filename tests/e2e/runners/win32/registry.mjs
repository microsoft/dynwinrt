// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from "node:assert/strict";
import {
  regCloseKey,
  regOpenKeyEx,
  regQueryValueEx,
} from "../../e2e_generated/win32/win32/Windows.Win32.System.Registry/index.mjs";

const HKEY_LOCAL_MACHINE = 0x80000002n;
const KEY_READ = 0x20019;

const opened = regOpenKeyEx(
  HKEY_LOCAL_MACHINE,
  "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
  0,
  KEY_READ,
);
assert.equal(opened.status, 0);
assert(opened.key);
assert.equal(opened.key.closed, false);

try {
  const probe = regQueryValueEx(opened.key, "ProductName", null);
  assert.equal(probe.status, 0);
  assert(probe.dataSize > 0);

  const data = Buffer.alloc(probe.dataSize);
  const read = regQueryValueEx(opened.key, "ProductName", data);
  assert.equal(read.status, 0);
  let end = read.dataSize;
  if (end >= 2 && data.readUInt16LE(end - 2) === 0) {
    end -= 2;
  }
  const productName = data.toString("utf16le", 0, end);
  assert.match(productName, /Windows/i);
  assert.throws(() => regCloseKey(opened.key.value), /DynWin32Resource|resource/i);
  assert.equal(opened.key.closed, false);
} finally {
  const closed = regCloseKey(opened.key);
  assert.equal(closed.status, 0);
}
assert.equal(opened.key.closed, true);

const missing = regOpenKeyEx(
  HKEY_LOCAL_MACHINE,
  "SOFTWARE\\DynWinRT\\DefinitelyMissing",
  0,
  KEY_READ,
);
assert.equal(missing.status, 2);
assert.equal(missing.key, null);

console.log("PASS");
