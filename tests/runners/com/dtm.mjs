// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.
//
// E2E: real Node.js proof that IDataTransferManagerInterop returns a live
// WinRT object through the HWND interop pattern:
//   IDataTransferManagerInterop::GetForWindow(HWND, REFIID, void**)
// Run: .\tests\e2e_test.ps1 -SkipBuild -Lang com

import { DynCom, DynComMethodSig, WinGuid } from '../../../bindings/js/dist/index.js';
import { IDataTransferManagerInterop } from '../../e2e_generated/com/shell/IDataTransferManagerInterop.js';
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

console.log('[e2e] step 2: IDataTransferManagerInterop.getForWindow(hwnd)');
let dtm;
try {
    dtm = IDataTransferManagerInterop.create().getForWindow(hwndBig);
} catch (e) {
    fail(`getForWindow threw: ${e && e.message ? e.message : e}`);
}

if (dtm == null) fail('getForWindow returned null');
console.log(`[e2e]   got DataTransferManager instance = ${dtm}`);

console.log('[e2e] step 3: MEANINGFUL — read live member `runtimeClassName` (via IInspectable::GetRuntimeClassName)');
const inspectable = DynCom.registerIUnknownInterface(
    'IInspectable_e2e',
    WinGuid.parse('af86e2e0-b12d-4c6a-9c5a-d7aa65101e90'),
)
    .addMethod('GetIids', new DynComMethodSig().addOut(DynCom.pointerType()).addOut(DynCom.pointerType()))
    .addMethod('GetRuntimeClassName', new DynComMethodSig().addOut(DynCom.hstringType()))
    .addMethod('GetTrustLevel', new DynComMethodSig().addOut(DynCom.i32Type()));
let name;
try {
    name = inspectable.method(4).getString(dtm);
} catch (e) {
    fail(`GetRuntimeClassName threw: ${e && e.message ? e.message : e}`);
}
console.log(`[e2e]   runtimeClassName = ${JSON.stringify(name)}`);

const expected = 'Windows.ApplicationModel.DataTransfer.DataTransferManager';
if (name !== expected) fail(`expected runtimeClassName='${expected}', got '${name}'`);

console.log('PASS');
process.exit(0);
