// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { Win32Handle, Win32Resource } from "../../../../bindings/js/dist/win32.js";

const option = process.argv.indexOf("--generated");
assert(option >= 0 && process.argv[option + 1], "--generated is required");
const generated = resolve(process.argv[option + 1]);
const requireGenerated = createRequire(join(generated, "package.json"));
const systemPath = "win32/windows/win32/system/system-information";
const registryPath = "win32/windows/win32/system/registry";
const system = requireGenerated(`@winapp/bindings/${systemPath}`);
const registry = requireGenerated(`@winapp/bindings/${registryPath}`);
const esm = await import(pathToFileURL(join(generated, systemPath, "index.mjs")).href);

assert.deepEqual(Object.keys(system).sort(), ["getTickCount", "getTickCount64"]);
assert.equal(typeof system.getTickCount(), "number");
assert.equal(typeof system.getTickCount64(), "bigint");
assert(system.getTickCount64() > 0n);
assert.equal(esm.getTickCount, system.getTickCount);
assert.throws(() => system.getTickCount(1), /Expected 0 arguments/);
assert.throws(() => requireGenerated("@winapp/bindings"), {
  code: "ERR_PACKAGE_PATH_NOT_EXPORTED",
});

const machine = Win32Handle.hkey(0x80000002n);
const opened = registry.regOpenKeyExW(
  machine,
  "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
  0,
  1,
);
assert.deepEqual(Object.keys(opened).sort(), ["phkResult", "status"]);
assert.equal(opened.status, 0);
const key = opened.phkResult;
assert(key instanceof Win32Resource);

try {
  const probe = registry.regQueryValueExW(key, "ProductName", null);
  assert.deepEqual(Object.keys(probe).sort(), ["lpData", "lpType", "lpcbData", "status"]);
  assert.equal(probe.status, 0);
  assert.equal(probe.lpType, 1);
  assert.equal(probe.lpData, null);
  assert(probe.lpcbData > 2);

  const small = Buffer.alloc(1);
  Object.defineProperty(small, "length", { value: 0xffffffff });
  Object.defineProperty(small, "byteLength", { value: 0xffffffff });
  assert.deepEqual(registry.regQueryValueExW(key, "ProductName", small), {
    status: 234,
    lpType: null,
    lpData: null,
    lpcbData: probe.lpcbData,
  });

  const capacity = new Uint8Array(probe.lpcbData).fill(0xab);
  const value = registry.regQueryValueExW(key, "ProductName", capacity);
  assert.equal(value.status, 0);
  assert.equal(value.lpType, 1);
  assert.equal(value.lpcbData, probe.lpcbData);
  assert(Buffer.isBuffer(value.lpData));
  assert.equal(value.lpData.length, value.lpcbData);
  assert.match(value.lpData.toString("utf16le"), /Windows/);
  assert(capacity.every((byte) => byte === 0xab));

  const missing = registry.regQueryValueExW(key, "dynwinrt-missing-contract-value", null);
  assert.notEqual(missing.status, 0);
  assert.equal(missing.lpType, null);
  assert.equal(missing.lpData, null);
  assert.equal(missing.lpcbData, null);
  assert.throws(() => registry.regCloseKey(machine), /managed HKEY|native Win32 carrier/);
  assert.equal(key.closed, false);
  assert.equal(registry.regCloseKey(key), 0);
  assert.equal(key.closed, true);
  assert.equal(registry.regCloseKey(key), 0);
  assert.throws(() => registry.regQueryValueExW(key, "ProductName", null), /closed/);
} finally {
  assert.equal(key.close(), 0);
}

const failed = registry.regOpenKeyExW(
  machine,
  `SOFTWARE\\dynwinrt-missing-contract-${process.pid}`,
  0,
  1,
);
assert.notEqual(failed.status, 0);
assert.equal(failed.phkResult, null);
console.log("PASS Win32 contracts: namespace imports, scalar calls, owned handles, byte counts");
