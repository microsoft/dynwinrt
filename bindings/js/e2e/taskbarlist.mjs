// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.
//
// Phase 2 E2E: real Node.js proof that the generated natural ITaskbarList3
// wrapper drives live Windows classic COM (ITaskbarList3) via CoCreateInstance.
//
// Run: node bindings/js/e2e/taskbarlist.mjs

import { DynWinRtValue, WinGuid } from '../dist/index.js';
import { ITaskbarList3 } from './ITaskbarList3.js';
import { TBPFLAG } from './TBPFLAG.js';

function fail(msg) {
    console.error(`[e2e] FAIL: ${msg}`);
    process.exit(1);
}

function toBigInt(v) {
    if (typeof v === 'bigint') return v;
    if (typeof v === 'number') return BigInt(v);
    if (v && typeof v.asPointerBigint === 'function') return v.asPointerBigint();
    fail(`cannot convert value to bigint: ${typeof v}`);
    return 0n; // unreachable
}

console.log('[e2e] step 1: acquiring an HWND via flatInvoke(kernel32!GetConsoleWindow)');

let hwndValue = DynWinRtValue.flatInvoke('kernel32.dll', 'GetConsoleWindow', 'Ptr', []);
let hwndBig = toBigInt(hwndValue);
console.log(`[e2e]   GetConsoleWindow() → 0x${hwndBig.toString(16)}`);

if (hwndBig === 0n) {
    console.log('[e2e]   console HWND is null (no console), falling back to GetDesktopWindow()');
    hwndValue = DynWinRtValue.flatInvoke('user32.dll', 'GetDesktopWindow', 'Ptr', []);
    hwndBig = toBigInt(hwndValue);
    console.log(`[e2e]   GetDesktopWindow() → 0x${hwndBig.toString(16)}`);
}

if (hwndBig === 0n) {
    fail('could not obtain a non-null HWND from GetConsoleWindow or GetDesktopWindow');
}

console.log('[e2e] step 2: CoCreateInstance(CLSID_TaskbarList, IID_ITaskbarList3)');

let t;
try {
    t = ITaskbarList3.create();
} catch (e) {
    fail(`ITaskbarList3.create() threw: ${e && e.message ? e.message : e}`);
}
console.log(`[e2e]   ITaskbarList3 = ${t}`);

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
// `DynWinRtValue.pointer(fFullscreen)` and typed `fFullscreen: BOOL = bigint | Buffer`,
// so passing a plain `false` threw at napi. After the fix, BOOL projects as an
// i32 with a `boolean` surface, and this natural-JS call round-trips.
console.log('[e2e] step 7: MarkFullscreenWindow(hwnd, false) — proves BOOL→i32 codegen fix');
try {
    t.markFullscreenWindow(hwndBig, false);
} catch (e) {
    fail(`MarkFullscreenWindow(hwnd, false) threw: ${e && e.message ? e.message : e}`);
}

console.log('PASS');
process.exit(0);
