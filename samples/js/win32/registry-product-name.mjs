// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import {
  regOpenKeyEx,
  regQueryValueEx,
} from "./generated/win32/Windows.Win32.System.Registry/index.mjs";

const HKEY_LOCAL_MACHINE = 0x80000002n;
const KEY_READ = 0x20019;
const ERROR_SUCCESS = 0;

const opened = regOpenKeyEx(
  HKEY_LOCAL_MACHINE,
  "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
  0,
  KEY_READ,
);
if (opened.status !== ERROR_SUCCESS || !opened.key) {
  throw new Error(`RegOpenKeyEx failed with status ${opened.status}`);
}

try {
  const probe = regQueryValueEx(opened.key, "ProductName", null);
  if (probe.status !== ERROR_SUCCESS) {
    throw new Error(`RegQueryValueEx size query failed with status ${probe.status}`);
  }

  const data = Buffer.alloc(probe.dataSize);
  const result = regQueryValueEx(opened.key, "ProductName", data);
  if (result.status !== ERROR_SUCCESS) {
    throw new Error(`RegQueryValueEx failed with status ${result.status}`);
  }

  let byteLength = result.dataSize;
  if (byteLength >= 2 && data.readUInt16LE(byteLength - 2) === 0) {
    byteLength -= 2;
  }
  console.log(data.toString("utf16le", 0, byteLength));
} finally {
  opened.key.close();
}
