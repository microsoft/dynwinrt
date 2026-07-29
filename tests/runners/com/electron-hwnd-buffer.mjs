// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.
//
// Regression for Electron's BrowserWindow.getNativeWindowHandle() shape:
// generated HWND inputs accept an exact pointer-width Buffer and decode its
// contents as the handle value. Other Buffer-backed pointers retain address
// semantics.

import { ITaskbarList3 } from '../../e2e_generated/com/shell/ITaskbarList3.js';
import { TBPFLAG } from '../../e2e_generated/com/shell/TBPFLAG.js';
import { acquireHwndBigInt } from './hwnd.mjs';

function fail(msg) {
    console.error(`[e2e] FAIL: ${msg}`);
    process.exit(1);
}

console.log('[e2e] step 1: acquiring a process-owned HWND');
const hwnd = acquireHwndBigInt();
console.log(`[e2e]   HWND → 0x${hwnd.toString(16)}`);

console.log('[e2e] step 2: simulating Electron getNativeWindowHandle() Buffer');
const pointerWidth = process.arch === 'ia32' ? 4 : 8;
const electronHandleBuffer = Buffer.alloc(pointerWidth);
if (pointerWidth === 8) {
    electronHandleBuffer.writeBigUInt64LE(hwnd, 0);
} else {
    electronHandleBuffer.writeUInt32LE(Number(hwnd), 0);
}

console.log('[e2e] step 3: creating ITaskbarList3');
let taskbar;
try {
    taskbar = ITaskbarList3.create();
    taskbar.hrInit();
} catch (e) {
    fail(`ITaskbarList3 activation/HrInit threw: ${e && e.message ? e.message : e}`);
}

console.log('[e2e] step 4: passing Electron-style HWND Buffer directly');
try {
    taskbar.setProgressState(electronHandleBuffer, TBPFLAG.TBPF_NORMAL);
    taskbar.markFullscreenWindow(electronHandleBuffer, false);
    taskbar.setProgressState(electronHandleBuffer, TBPFLAG.TBPF_NOPROGRESS);
} catch (e) {
    fail(`Electron HWND Buffer pattern threw: ${e && e.message ? e.message : e}`);
}

console.log('PASS');
process.exit(0);
