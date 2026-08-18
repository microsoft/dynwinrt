// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import {
  createSYSTEMTIME,
  getSystemTime,
  getTickCount64,
} from "./generated/win32/Windows.Win32.System.SystemInformation/index.mjs";

const uptimeMilliseconds = getTickCount64();

const systemTime = createSYSTEMTIME();
getSystemTime(systemTime);
const bytes = systemTime.bytes;
const utc = new Date(Date.UTC(
  bytes.readUInt16LE(0),
  bytes.readUInt16LE(2) - 1,
  bytes.readUInt16LE(6),
  bytes.readUInt16LE(8),
  bytes.readUInt16LE(10),
  bytes.readUInt16LE(12),
  bytes.readUInt16LE(14),
));

console.log(`Windows uptime: ${uptimeMilliseconds} ms`);
console.log(`System UTC time: ${utc.toISOString()}`);
