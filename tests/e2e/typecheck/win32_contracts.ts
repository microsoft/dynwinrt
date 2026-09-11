// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { Win32Handle, Win32Resource } from "../../../bindings/js/dist/win32.js";
import {
  getTickCount,
  getTickCount64,
} from "../e2e_generated/win32/win32/windows/win32/system/system-information/Apis.js";
import {
  regCloseKey,
  regOpenKeyExW,
  regQueryValueExW,
} from "../e2e_generated/win32/win32/windows/win32/system/registry/Apis.js";

const ticks: number = getTickCount();
const ticks64: bigint = getTickCount64();
void [ticks, ticks64];
const machine = Win32Handle.hkey(0x80000002n);
const opened = regOpenKeyExW(machine, "SOFTWARE", 0, 1);
const status: number = opened.status;
void status;
if (opened.phkResult instanceof Win32Resource) {
  const result = regQueryValueExW(opened.phkResult, null, new Uint8Array(16));
  const data: Buffer | null = result.lpData;
  const count: number | null = result.lpcbData;
  const type: number | null = result.lpType;
  void [data, count, type];
  const closed: number = regCloseKey(opened.phkResult);
  void closed;
}

// @ts-expect-error borrowed handles cannot be consumed
regCloseKey(machine);
// @ts-expect-error numeric pointer bits are not typed HKEY values
regOpenKeyExW(0x80000002n, null, 0, 1);
// @ts-expect-error raw addresses cannot supply byte storage
regQueryValueExW(machine, null, 123n);
