// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from "node:assert/strict";
import {
  initializeWinsock,
  wsaGetLastError,
  wsaSetLastError,
} from "../../e2e_generated/win32/win32/Windows.Win32.Networking.WinSock/Apis.js";
import {
  gdipGetImageDecodersSize,
  initializeGdiPlus,
} from "../../e2e_generated/win32/win32/Windows.Win32.Graphics.GdiPlus/Apis.js";
import {
  initializeMediaFoundation,
  mfGetTimerPeriodicity,
} from "../../e2e_generated/win32/win32/Windows.Win32.Media.MediaFoundation/Apis.js";

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

const mediaFoundation = initializeMediaFoundation();
assert.equal(mediaFoundation.subsystem, "mediaFoundation");
const timer = mfGetTimerPeriodicity(mediaFoundation);
assert.equal(timer.status, 0);
assert(timer.periodicity > 0);
mediaFoundation.close();
assert.throws(
  () => mfGetTimerPeriodicity(mediaFoundation),
  /context is closed/,
);

console.log("PASS");
