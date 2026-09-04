// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.
//
// Classic-COM/interop E2E helper to obtain a process-owned Win32 HWND
// without relying on flat-Win32 codegen.
//
// Interop APIs like `IDataTransferManagerInterop::GetForWindow` and
// `ISystemMediaTransportControlsInterop::GetForWindow` require an HWND
// that is OWNED BY THE CALLING PROCESS (they return E_ACCESSDENIED for
// desktop / shell / cross-process HWNDs). This helper delegates to the
// napi `createTestHwnd()` export, which creates a hidden `WS_POPUP`
// window in the Node process using the pre-registered `STATIC` class.

import { DynCom } from '../../../../bindings/js/dist/com-unsafe.js';
import { roInitialize } from '../../../../bindings/js/dist/winrt.js';

roInitialize(1);

/**
 * Return a valid Win32 HWND owned by the current process, as a bigint.
 * Throws if window creation fails.
 */
export function acquireHwndBigInt() {
    const hwnd = DynCom.createTestHwnd();
    // napi BigInt → JS bigint.
    const n = typeof hwnd === 'bigint' ? hwnd : BigInt(hwnd);
    if (n === 0n) {
        throw new Error('acquireHwndBigInt: createTestHwnd returned 0');
    }
    return n;
}
