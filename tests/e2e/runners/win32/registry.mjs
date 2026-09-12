// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from "node:assert/strict";
import {
  regCloseKey,
  regGetValueA,
  regGetValueW,
  regOpenKeyA,
  regOpenKeyW,
  regOpenKeyExA,
  regOpenKeyExW,
  regOpenKeyEx,
  regQueryValueExA,
  regQueryValueEx,
} from "../../e2e_generated/win32/win32/windows/win32/system/registry/index.mjs";
import {
  DynWin32,
  DynWin32Function,
  DynWin32Resource,
} from "../../../../bindings/js/dist/win32-unsafe.js";

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

for (const open of [regOpenKeyA, regOpenKeyW]) {
  for (const subKey of [null, ""]) {
    const owner = regOpenKeyEx(HKEY_LOCAL_MACHINE, "SOFTWARE", 0, KEY_READ).key;
    assert(owner instanceof DynWin32Resource);
    try {
      const result = open(owner, subKey);
      assert.equal(result.status, 0);
      assert(result.key instanceof DynWin32Resource);
      assert.equal(result.key.value, owner.value);
      result.key.close();
      assert.equal(owner.closed, true);
      assert.throws(() => regQueryValueEx(owner, "Missing", null), /closed/);
      result.key.close();
    } finally {
      owner.close();
    }
    const borrowed = open(HKEY_LOCAL_MACHINE, subKey);
    assert.equal(borrowed.status, 0);
    assert.equal(borrowed.key, HKEY_LOCAL_MACHINE);
  }
}

for (const open of [regOpenKeyExA, regOpenKeyExW]) {
  const predefined = -2147483646n;
  const borrowed = open(predefined, "", 0, KEY_READ);
  assert.equal(borrowed.status, 0);
  assert.equal(borrowed.key, DynWin32.toBigint(DynWin32.handle(predefined)));
  const zeroExtended = open(HKEY_LOCAL_MACHINE, "", 0, KEY_READ);
  assert.equal(zeroExtended.status, 0);
  assert(zeroExtended.key instanceof DynWin32Resource);
  zeroExtended.key.close();
  assert.equal(zeroExtended.key.closed, true);
  const owner = regOpenKeyEx(
    HKEY_LOCAL_MACHINE,
    "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
    0,
    KEY_READ,
  ).key;
  assert(owner instanceof DynWin32Resource);
  try {
    const independent = open(owner, null, 0, KEY_READ).key;
    assert(independent instanceof DynWin32Resource);
    independent.close();
    assert.equal(owner.closed, false);
    assert.equal(regQueryValueEx(owner, "ProductName", null).status, 0);
  } finally {
    owner.close();
  }
}

const performance = -2147483644n;
const normal = regOpenKeyEx(
  HKEY_LOCAL_MACHINE,
  "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
  0,
  KEY_READ,
).key;
assert(normal instanceof DynWin32Resource);
try {
  for (const read of [
    (key, name, data) => regQueryValueExA(key, name, data),
    (key, name, data) => regQueryValueEx(key, name, data),
    (key, name, data) => regGetValueA(key, null, name, 0xffff, data),
    (key, name, data) => regGetValueW(key, null, name, 0xffff, data),
  ]) {
    for (const capacity of [1, 64]) {
      const result = read(performance, "2", Buffer.alloc(capacity));
      assert.equal(result.status, 234);
      assert.equal(result.dataSize, null);
    }
    let succeeded = false;
    for (let capacity = 64 * 1024; capacity <= 64 * 1024 * 1024; capacity *= 2) {
      const data = Buffer.alloc(capacity);
      const result = read(performance, "2", data);
      if (result.status === 234) {
        assert.equal(result.dataSize, null);
        continue;
      }
      assert.equal(result.status, 0);
      assert(result.dataSize > 0 && result.dataSize <= capacity);
      assert.equal(data.toString("utf16le", 0, 8), "PERF");
      succeeded = true;
      break;
    }
    assert(succeeded);
    const probe = read(normal, "ProductName", null);
    assert.equal(probe.status, 0);
    assert(probe.dataSize > 1);
    const small = read(normal, "ProductName", Buffer.alloc(1));
    assert.equal(small.status, 234);
    assert(small.dataSize > 1);
    const result = read(normal, "ProductName", Buffer.alloc(probe.dataSize));
    assert.equal(result.status, 0);
    assert(result.dataSize > 0 && result.dataSize <= probe.dataSize);
  }
} finally {
  normal.close();
  const closePerformance = DynWin32Function.bind({
    dll: "advapi32.dll",
    entryPoint: "RegCloseKey",
    parameters: [{ type: "handle", direction: "in" }],
    returnType: "i32",
    successRule: "zero",
  }).invoke([DynWin32.handle(performance)]);
  assert.equal(DynWin32.toNumber(closePerformance.returnValue), 0);
}

console.log("PASS");
