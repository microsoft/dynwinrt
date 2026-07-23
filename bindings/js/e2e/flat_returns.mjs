// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.
//
// E2E coverage for flat-Win32 return kinds that require exact ABI handling:
// pointer/function-pointer, void, u64, and optional float returns.

import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const __dirname_flat_returns = dirname(fileURLToPath(import.meta.url));

function fixture(path) {
    return resolve(__dirname_flat_returns, path);
}

function requireFixture(path, namespace) {
    const full = fixture(path);
    if (existsSync(full)) {
        return full;
    }
    console.error(`[e2e] FAIL: required fixture not found: ${full}`);
    console.error('[e2e] Regenerate it with:');
    console.error('  target\\release\\dynwinrt-codegen.exe generate \\');
    console.error('    --winmd C:\\s\\win32metadata\\Windows.Win32.winmd \\');
    console.error(`    --namespace ${namespace} \\`);
    console.error('    --class-name Apis \\');
    console.error(`    --output ${dirname(full)} \\`);
    console.error('    --import-name ../../../dist/index.js');
    process.exit(1);
}

const libraryLoaderPath = requireFixture(
    'generated/flat_returns_libraryloader/Apis.js',
    'Windows.Win32.System.LibraryLoader',
);
const systemInformationPath = requireFixture(
    'generated/flat_returns_systeminformation/Apis.js',
    'Windows.Win32.System.SystemInformation',
);

const {
    getModuleHandleW,
    getProcAddress,
} = await import(pathToFileURL(libraryLoaderPath).href);
const {
    getNativeSystemInfo,
    getTickCount64,
} = await import(pathToFileURL(systemInformationPath).href);

function pass(msg) {
    console.log(`[e2e] PASS: ${msg}`);
}

function isPowerOfTwo(value) {
    return value > 0 && (value & (value - 1)) === 0;
}

// F4: FARPROC/function-pointer returns must be BigInt pointer values, not
// truncated I32/EAX numbers.
const k32 = getModuleHandleW('KERNEL32.dll').result;
assert.equal(typeof k32, 'bigint');
assert.notEqual(k32, 0n, 'KERNEL32.dll should already be loaded');

const proc = getProcAddress(k32, 'GetProcAddress').result;
assert.equal(typeof proc, 'bigint');
assert.notEqual(proc, 0n, 'GetProcAddress export should resolve');
assert(proc > 0xffffffffn, 'x64 function pointer should not be EAX-truncated');
pass(`GetProcAddress returned full pointer ${proc}`);

// U64 return: GetTickCount64 must surface as BigInt and be monotonic.
const firstTick = getTickCount64().result;
await new Promise((resolveDelay) => setTimeout(resolveDelay, 20));
const secondTick = getTickCount64().result;
assert.equal(typeof firstTick, 'bigint');
assert(firstTick > 0n);
assert(secondTick >= firstTick);
pass(`GetTickCount64 returned monotonic BigInts ${firstTick} -> ${secondTick}`);

// Void return + caller-owned opaque struct pointer: the wrapper should return
// undefined while mutating the caller's SYSTEM_INFO buffer.
const systemInfo = Buffer.alloc(48);
const voidRet = getNativeSystemInfo(systemInfo);
assert.equal(voidRet, undefined);
const pageSize = systemInfo.readUInt32LE(4);
const processors = systemInfo.readUInt32LE(32);
assert(pageSize >= 4096 && isPowerOfTwo(pageSize), `unexpected page size ${pageSize}`);
assert(processors > 0, `unexpected processor count ${processors}`);
pass(`GetNativeSystemInfo returned undefined and filled pageSize=${pageSize}, processors=${processors}`);

// Optional F32 return + F32 arg: Direct2D is present on normal Windows 10/11,
// but keep this resilient because the Rust flat_call unit is the authoritative
// float ABI proof.
const direct2DPath = fixture('generated/flat_returns_direct2d/Apis.js');
if (!existsSync(direct2DPath)) {
    console.log('[e2e] SKIP: Direct2D fixture not generated');
} else {
    const direct2D = await import(pathToFileURL(direct2DPath).href);
    if (typeof direct2D.d2D1Tan !== 'function') {
        console.log('[e2e] SKIP: Direct2D D2D1Tan export unavailable in generated fixture');
    } else {
        const zero = direct2D.d2D1Tan(0).result;
        const one = direct2D.d2D1Tan(Math.PI / 4).result;
        assert.equal(typeof zero, 'number');
        assert.equal(typeof one, 'number');
        assert(Math.abs(zero) < 1e-6, `D2D1Tan(0) = ${zero}`);
        assert(Math.abs(one - 1) < 1e-5, `D2D1Tan(pi/4) = ${one}`);
        pass(`D2D1Tan float return/arg works (${zero}, ${one})`);
    }
}

console.log('PASS');
