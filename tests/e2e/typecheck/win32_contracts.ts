// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { DynWin32Resource } from "../../../bindings/js/dist/win32.js";
import {
  getTickCount,
  getTickCount64,
  createSYSTEMTIME,
  getSystemTime,
} from "../e2e_generated/win32/win32/windows/win32/system/system-information/Apis.js";
import {
  regCloseKey,
  regOpenKeyExW,
  regQueryValueExW,
} from "../e2e_generated/win32/win32/windows/win32/system/registry/Apis.js";
import {
  readFileAsync,
  writeFileAsync,
} from "../e2e_generated/win32/win32/windows/win32/storage/file-system/Apis.js";

const ticks: number = getTickCount();
const ticks64: bigint = getTickCount64();
void [ticks, ticks64];
const time = createSYSTEMTIME();
getSystemTime(time);
const machine = 0x80000002n;
const opened = regOpenKeyExW(machine, "SOFTWARE", 0, 1);
const status: number = opened.status;
void status;
if (opened.key instanceof DynWin32Resource) {
  const key: DynWin32Resource = opened.key;
  const result = regQueryValueExW(key, null, new Uint8Array(16));
  const count: number | null = result.dataSize;
  void count;
  const closed: number = regCloseKey(key).status;
  void closed;
}

function asyncIo(file: DynWin32Resource, bytes: Buffer, signal: AbortSignal) {
  const read: Promise<number> = readFileAsync(file, bytes, 0n, signal);
  const write: Promise<number> = writeFileAsync(file, bytes, 0n, signal);
  return [read, write];
}
void asyncIo;

// @ts-expect-error raw handles cannot be consumed as managed resources
regCloseKey(machine);
// @ts-expect-error byte storage is not a numeric HKEY value
regOpenKeyExW(Buffer.alloc(8), null, 0, 1);
// @ts-expect-error raw addresses cannot supply byte storage
regQueryValueExW(machine, null, 123n);
