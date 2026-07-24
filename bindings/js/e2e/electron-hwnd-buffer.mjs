// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.
//
// Regression for classic-COM handle-value typedefs: Electron exposes HWNDs as
// Buffers (BrowserWindow.getNativeWindowHandle()). Callers must read the handle
// bits out of that Buffer and pass the numeric handle value, not the Buffer
// itself, because DynCom.pointer(Buffer) passes the Buffer's own address.

import { ITaskbarList3 } from './ITaskbarList3.js';
import { TBPFLAG } from './TBPFLAG.js';
import { acquireHwndBigInt } from './hwnd.mjs';

function fail(msg) {
    console.error(`[e2e] FAIL: ${msg}`);
    process.exit(1);
}

console.log('[e2e] step 1: acquiring a process-owned HWND');
const hwnd = acquireHwndBigInt();
console.log(`[e2e]   HWND → 0x${hwnd.toString(16)}`);

console.log('[e2e] step 2: simulating Electron getNativeWindowHandle() Buffer');
const electronHandleBuffer = Buffer.alloc(8);
electronHandleBuffer.writeBigUInt64LE(hwnd, 0);
const hwndFromBuffer = electronHandleBuffer.readBigUInt64LE(0);
if (hwndFromBuffer !== hwnd) {
    fail(`round-trip through Buffer changed HWND: ${hwndFromBuffer} !== ${hwnd}`);
}

console.log('[e2e] step 3: creating ITaskbarList3');
let taskbar;
try {
    taskbar = ITaskbarList3.create();
    taskbar.hrInit();
} catch (e) {
    fail(`ITaskbarList3 activation/HrInit threw: ${e && e.message ? e.message : e}`);
}

console.log('[e2e] step 4: passing Buffer-read bigint HWND to real classic-COM calls');
try {
    taskbar.setProgressState(hwndFromBuffer, TBPFLAG.TBPF_NORMAL);
    taskbar.markFullscreenWindow(hwndFromBuffer, false);
    taskbar.setProgressState(hwndFromBuffer, TBPFLAG.TBPF_NOPROGRESS);
} catch (e) {
    fail(`Electron Buffer -> bigint HWND pattern threw: ${e && e.message ? e.message : e}`);
}

// Do not assert that passing `electronHandleBuffer` directly fails: some shell
// APIs tolerate invalid HWNDs and return S_OK/no-op. The regression this locks
// down is that the documented readBigUInt64LE(0) pattern is valid end-to-end.
console.log('PASS');
process.exit(0);
