// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { isMissingMediaFoundation } from "../../../../bindings/js/__test__/helpers/optional-media-foundation.mjs";

const require = createRequire(import.meta.url);
const native = require("../../../../bindings/js/dist/index.js");
const originalRuntime = native.DynWin32;
const originalBind = native.win32Bind;
const initializers = new Set([
  "initializeWinsock",
  "initializeGdiPlus",
  "initializeMediaFoundation",
  "initializeMapiUtilities",
]);
let importing = true;

// Native static methods are immutable. Forward through a guarded class before
// the JS facades capture them; forbidden import-time calls fail, never fake success.
class ImportGuard {}
for (const name of Object.getOwnPropertyNames(originalRuntime)) {
  if (["name", "length", "prototype"].includes(name)) continue;
  const method = Object.getOwnPropertyDescriptor(originalRuntime, name).value;
  if (typeof method !== "function") continue;
  Object.defineProperty(ImportGuard, name, {
    value: (...args) => {
      if (initializers.has(name)) {
        assert(!importing, `Namespace import must not call ${name}`);
      }
      return Reflect.apply(method, originalRuntime, args);
    },
  });
}
native.DynWin32 = ImportGuard;
native.win32Bind = (...args) => {
  assert(!importing, "Namespace import must not bind Win32 exports");
  return Reflect.apply(originalBind, native, args);
};
let namespaces;
try {
  namespaces = await Promise.all([
    import("../../e2e_generated/win32/win32/windows/win32/networking/win-sock/Apis.js"),
    import("../../e2e_generated/win32/win32/windows/win32/graphics/gdi-plus/Apis.js"),
    import("../../e2e_generated/win32/win32/windows/win32/media/media-foundation/Apis.js"),
    import("../../../../bindings/js/dist/winrt.js"),
    import("../../../../bindings/js/dist/com.js"),
  ]);
} finally {
  importing = false;
  native.DynWin32 = originalRuntime;
  native.win32Bind = originalBind;
}
const [
  { initializeWinsock, wsaGetLastError, wsaSetLastError },
  { gdipGetImageDecodersSize, initializeGdiPlus },
  { initializeMediaFoundation, mfGetTimerPeriodicity },
  winrt,
  com,
] = namespaces;
assert.equal(typeof winrt.WinGuid.parse, "function");
assert.equal(typeof com.initializeCom, "function");
console.log(
  "PASS namespace and WinRT/COM imports without Win32 binding or subsystem startup",
);

const winsock = initializeWinsock();
assert.equal(winsock.subsystem, "winsock");
wsaSetLastError(winsock, 12345);
assert.equal(wsaGetLastError().result, 12345);
winsock.close();
assert.throws(() => wsaSetLastError(winsock, 0), /context is closed/);

const gdiplus = initializeGdiPlus();
assert.equal(gdiplus.subsystem, "gdiplus");
const decoders = gdipGetImageDecodersSize(gdiplus);
assert.equal(decoders.result, 0);
assert(decoders.numDecoders > 0);
assert(decoders.size > 0);
gdiplus.close();
assert.throws(() => gdipGetImageDecodersSize(gdiplus), /context is closed/);

let mediaFoundation;
try {
  mediaFoundation = initializeMediaFoundation();
} catch (error) {
  if (!isMissingMediaFoundation(error)) throw error;
  console.log(
    `SKIP optional Media Foundation calls; initialization reported: ${error.message}`,
  );
}
if (mediaFoundation) {
  try {
    assert.equal(mediaFoundation.subsystem, "mediaFoundation");
    const timer = mfGetTimerPeriodicity(mediaFoundation);
    assert.equal(timer.status, 0);
    assert(timer.periodicity > 0);
  } finally {
    mediaFoundation.close();
  }
  assert.throws(
    () => mfGetTimerPeriodicity(mediaFoundation),
    /context is closed/,
  );
}

console.log("PASS");
