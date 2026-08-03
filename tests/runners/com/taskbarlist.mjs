// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.
//
// Phase 2 E2E: real Node.js proof that the generated natural ITaskbarList3
// wrapper drives live Windows classic COM (ITaskbarList3) via CoCreateInstance.
//
// Run: .\tests\e2e_test.ps1 -SkipBuild -Lang com

import {
    ITaskbarList3,
    TaskbarList,
    TBPFLAG,
} from '../../e2e_generated/com/shell/com/index.mjs';
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

console.log('[e2e] step 2: CoCreateInstance(CLSID_TaskbarList, IID_ITaskbarList3)');

let taskbar;
try {
    taskbar = new TaskbarList();
} catch (e) {
    fail(`new TaskbarList() threw: ${e && e.message ? e.message : e}`);
}
console.log(`[e2e]   TaskbarList = ${taskbar}`);

if (!taskbar.supports(ITaskbarList3)) {
    fail('TaskbarList.supports(ITaskbarList3) returned false');
}
const optionalV3 = taskbar.tryAs(ITaskbarList3);
if (optionalV3 === null) {
    fail('TaskbarList.tryAs(ITaskbarList3) returned null');
}
optionalV3.release();
const t = taskbar.as(ITaskbarList3);

console.log('[e2e] step 3: HrInit() (vtable slot 3)');
try {
    t.hrInit();
} catch (e) {
    fail(`HrInit() threw: ${e && e.message ? e.message : e}`);
}

// ITaskbarList3 accepts arbitrary HWNDs — SetProgressState / SetProgressValue on
// non-owned windows do not fail; they simply have no visible effect if the
// window is not a top-level shell window. What we need is: the call returns
// without an HRESULT error being thrown.

console.log('[e2e] step 4: SetProgressState(hwnd, TBPF_NORMAL) (vtable slot 10)');
try {
    t.setProgressState(hwndBig, TBPFLAG.TBPF_NORMAL);
} catch (e) {
    fail(`SetProgressState(TBPF_NORMAL) threw: ${e && e.message ? e.message : e}`);
}

console.log('[e2e] step 5: SetProgressValue(hwnd, 30n, 100n) (vtable slot 9, u64 args)');
try {
    t.setProgressValue(hwndBig, 30n, 100n);
} catch (e) {
    fail(`SetProgressValue(30, 100) threw: ${e && e.message ? e.message : e}`);
}

console.log('[e2e] step 6: SetProgressState(hwnd, TBPF_NOPROGRESS)');
try {
    t.setProgressState(hwndBig, TBPFLAG.TBPF_NOPROGRESS);
} catch (e) {
    fail(`SetProgressState(TBPF_NOPROGRESS) threw: ${e && e.message ? e.message : e}`);
}

// Prove the BOOL → i32 codegen fix: markFullscreenWindow historically emitted
// `DynCom.pointer(fFullscreen)` and typed `fFullscreen: BOOL = bigint | Buffer`,
// so passing a plain `false` threw at napi. After the fix, BOOL projects as an
// i32 with a `boolean` surface, and this natural-JS call round-trips.
console.log('[e2e] step 7: MarkFullscreenWindow(hwnd, false) — proves BOOL→i32 codegen fix');
try {
    t.markFullscreenWindow(hwndBig, false);
} catch (e) {
    fail(`MarkFullscreenWindow(hwnd, false) threw: ${e && e.message ? e.message : e}`);
}

console.log('[e2e] step 8: SetThumbnailTooltip(hwnd, string) — proves PWSTR string projection');
try {
    t.setThumbnailTooltip(hwndBig, 'dynwinrt COM E2E');
} catch (e) {
    fail(`SetThumbnailTooltip(hwnd, string) threw: ${e && e.message ? e.message : e}`);
}

t.release();
taskbar.release();
console.log('PASS');
process.exit(0);
