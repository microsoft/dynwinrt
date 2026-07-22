// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.
//
// E2E: real Node.js proof that the generated natural DataTransferManager
// wrapper drives live WinRT via the *Interop* HWND pattern:
//   IDataTransferManagerInterop::GetForWindow(HWND, REFIID, void**)
// The test uses ONLY the high-level generated wrapper — no low-level
// `registerInterfaceUnknown` / `coCreateInstance` / QI plumbing in the test.
//
// Run: node bindings/js/e2e/dtm.mjs

import { DynWinRtValue } from '../dist/index.js';
import { DataTransferManager } from './DataTransferManager.js';
import { acquireHwndBigInt } from './hwnd.mjs';

function fail(msg) {
    console.error(`[e2e] FAIL: ${msg}`);
    process.exit(1);
}

console.log('[e2e] step 1: acquiring a process-owned HWND via napi createTestHwnd()');
// The classic-vertical does not bundle flat-Win32, so we obtain a
// process-owned HWND via a small napi helper (`createTestHwnd`) instead of
// `flatInvoke(user32!CreateWindowExW, ...)`. This keeps the E2E
// self-contained with respect to the classic vertical's surface area.
const hwndBig = acquireHwndBigInt();
console.log(`[e2e]   HWND → 0x${hwndBig.toString(16)}`);
if (hwndBig === 0n) fail('acquireHwndBigInt returned NULL');

console.log('[e2e] step 2: DataTransferManager.getForWindow(hwnd)  [HIGH-LEVEL WRAPPER]');
let dtm;
try {
    dtm = DataTransferManager.getForWindow(hwndBig);
} catch (e) {
    fail(`DataTransferManager.getForWindow threw: ${e && e.message ? e.message : e}`);
}

if (dtm == null) fail('DataTransferManager.getForWindow returned null');
console.log(`[e2e]   got DataTransferManager instance = ${dtm}`);

console.log('[e2e] step 3: MEANINGFUL — read live member `runtimeClassName` (via IInspectable::GetRuntimeClassName)');
let name;
try {
    name = dtm.runtimeClassName;
} catch (e) {
    fail(`dtm.runtimeClassName threw: ${e && e.message ? e.message : e}`);
}
console.log(`[e2e]   runtimeClassName = ${JSON.stringify(name)}`);

const expected = 'Windows.ApplicationModel.DataTransfer.DataTransferManager';
if (name !== expected) fail(`expected runtimeClassName='${expected}', got '${name}'`);

console.log('PASS');
process.exit(0);
