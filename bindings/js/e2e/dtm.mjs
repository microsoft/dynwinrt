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

function wideCString(s) {
    // UTF-16LE + NUL terminator. Node's Buffer.from(s, 'utf16le') already emits LE.
    const body = Buffer.from(s, 'utf16le');
    const out = Buffer.alloc(body.length + 2);
    body.copy(out, 0);
    return out;
}

console.log('[e2e] step 1: creating a private HWND via CreateWindowExW("STATIC", ...)');
// GetForWindow requires an HWND OWNED by this process (else E_ACCESSDENIED).
// The desktop / foreground windows aren't ours, so we synthesise our own via
// the pre-registered system class "STATIC" — no need for RegisterClass.
const classNameBuf = wideCString('STATIC');
const titleBuf     = wideCString('dtm-e2e-test');
const p = DynWinRtValue.pointer;
const i = DynWinRtValue.i32;
const u = DynWinRtValue.u32;
const NULL_PTR = p(0n);

const hwndValue = DynWinRtValue.flatInvoke(
    'user32.dll',
    'CreateWindowExW',
    'Ptr',
    [
        u(0),                        // dwExStyle
        p(classNameBuf),             // lpClassName = "STATIC"
        p(titleBuf),                 // lpWindowName
        u(0),                        // dwStyle = WS_OVERLAPPED (0)
        i(0), i(0), i(1), i(1),       // X, Y, nWidth, nHeight
        NULL_PTR,                    // hWndParent
        NULL_PTR,                    // hMenu
        NULL_PTR,                    // hInstance
        NULL_PTR,                    // lpParam
    ],
);
const hwndBig = toBigInt(hwndValue);
console.log(`[e2e]   CreateWindowExW → 0x${hwndBig.toString(16)}`);
if (hwndBig === 0n) fail('CreateWindowExW returned NULL');

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
