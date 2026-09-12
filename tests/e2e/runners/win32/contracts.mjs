// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import * as runtime from "../../../../bindings/js/dist/win32.js";
import * as winrt from "../../../../bindings/js/dist/winrt.js";

const option = process.argv.indexOf("--generated");
assert(option >= 0 && process.argv[option + 1], "--generated is required");
const generated = resolve(process.argv[option + 1]);
const requireGenerated = createRequire(join(generated, "package.json"));
const systemPath = "win32/windows/win32/system/system-information";
const registryPath = "win32/windows/win32/system/registry";
const system = requireGenerated(`@winapp/bindings/${systemPath}`);
const registry = requireGenerated(`@winapp/bindings/${registryPath}`);
const esm = await import(pathToFileURL(join(generated, systemPath, "index.mjs")).href);

assert.equal(typeof system.getTickCount(), "number");
assert.equal(typeof system.getTickCount64(), "bigint");
assert.equal(typeof system.createSYSTEMTIME, "function");
assert.equal(esm.getTickCount64, system.getTickCount64);
assert.equal(registry.regOpenKeyEx, registry.regOpenKeyExW);
assert.equal(registry.regQueryValueEx, registry.regQueryValueExW);
assert.throws(() => requireGenerated("@winapp/bindings"), {
  code: "ERR_PACKAGE_PATH_NOT_EXPORTED",
});
assert.equal(typeof runtime.DynWin32, "function");
assert.equal(Object.hasOwn(runtime, "DynWin32Unsafe"), false);
assert.equal(Object.hasOwn(winrt, "DynWin32"), false);

const opened = registry.regOpenKeyEx(
  0x80000002n,
  "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
  0,
  1,
);
assert.equal(opened.status, 0);
assert(opened.key instanceof runtime.DynWin32Resource);
try {
  const probe = registry.regQueryValueEx(opened.key, "ProductName", null);
  assert.equal(probe.status, 0);
  const data = Buffer.alloc(probe.dataSize);
  const value = registry.regQueryValueEx(opened.key, "ProductName", data);
  assert.equal(value.status, 0);
  assert(value.dataSize > 0);
  assert.match(data.toString("utf16le", 0, value.dataSize), /Windows/);
} finally {
  opened.key.close();
}
console.log("PASS Win32 contracts: package isolation, namespace imports, aliases and baseline calls");
