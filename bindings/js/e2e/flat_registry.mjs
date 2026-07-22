// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.
//
// E2E test for the GENERATED flat-Win32 Registry wrapper.
//
// Unlike bindings/js/e2e/registry.js (hand-written), this test imports the
// output of `dynwinrt-codegen generate --namespace Windows.Win32.System.Registry
// --class-name Apis --output ./generated/flat_registry` and reads a real
// registry value through it: HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion
// ProductName. On a normal Windows install this reads something like
// "Windows 10 Pro" or "Windows 11 Enterprise".
//
// Composes a `Registry.getString(hive, subKey, valueName)` helper on top of
// the generated `regOpenKeyExW` / `regQueryValueExW` / `regCloseKey` — the
// wrapper itself is codegen output; the composition (retry-on-more-data,
// REG_SZ decode) is a thin ergonomic layer.

import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

// The generated flat-Win32 Registry wrapper under
// ./generated/flat_registry/ is a codegen fixture and is intentionally
// gitignored. On a clean checkout it must be regenerated before this test can
// run — otherwise a static import below would fail with an opaque
// module-not-found error. Fail early with a helpful message that spells out
// the exact regeneration command.
const __dirname_flat = dirname(fileURLToPath(import.meta.url));
const FLAT_FIXTURE = resolve(
    __dirname_flat,
    'generated/flat_registry/Apis.js'
);
if (!existsSync(FLAT_FIXTURE)) {
    console.error(`[e2e] FAIL: flat_registry fixture not found: ${FLAT_FIXTURE}`);
    console.error(`[e2e] This fixture is gitignored — regenerate it with:`);
    console.error(`  cargo run -p dynwinrt-codegen -- generate \\`);
    console.error(`    --winmd C:\\s\\win32metadata\\Windows.Win32.winmd \\`);
    console.error(`    --namespace Windows.Win32.System.Registry \\`);
    console.error(`    --class-name Apis \\`);
    console.error(`    --output bindings/js/e2e/generated/flat_registry \\`);
    console.error(`    --import-name ../../../dist/index.js`);
    process.exit(1);
}

const {
    regOpenKeyExW,
    regQueryValueExW,
    regCloseKey,
} = await import('./generated/flat_registry/Apis.js');

// Predefined HKEY hive constants. These are stable Win32 pseudo-handles that
// live in the same address slot on x86/x64 and are safe to pass as bigints.
const HKEY_LOCAL_MACHINE = 0x80000002n;

// KEY_READ = STANDARD_RIGHTS_READ (0x00020000) | KEY_QUERY_VALUE (0x0001)
//          | KEY_ENUMERATE_SUB_KEYS (0x0008) | KEY_NOTIFY (0x0010)
const KEY_READ = 0x20019;

const ERROR_SUCCESS = 0;
const ERROR_MORE_DATA = 234;

// REG_VALUE_TYPE constants we care about.
const REG_SZ = 1;
const REG_EXPAND_SZ = 2;

function decodeWideNulTerminated(buf, byteLength) {
    // REG_SZ / REG_EXPAND_SZ values are stored as UTF-16LE with (usually) a
    // NUL terminator inside the reported byte length. Strip the trailing NUL
    // if present so the surface string doesn't end in U+0000.
    let end = byteLength;
    if (end >= 2 && buf.readUInt16LE(end - 2) === 0) {
        end -= 2;
    }
    return buf.toString('utf16le', 0, end);
}

function getString(hive, subKey, valueName) {
    // 1. Open the subkey via the generated wrapper. `regOpenKeyExW` returns
    //    { status, phkResult } — natural JS shape, no raw flatInvoke leaking.
    const openRes = regOpenKeyExW(hive, subKey, 0, KEY_READ);
    if (openRes.status !== ERROR_SUCCESS) {
        throw new Error(
            `RegOpenKeyExW('${subKey}') failed with LSTATUS=${openRes.status}`,
        );
    }
    const hKey = openRes.phkResult;
    try {
        // 2. Probe the required buffer size. Passing data=null and
        //    lpcbData=0 causes RegQueryValueExW to fill lpcbData with the
        //    needed byte count and return either ERROR_SUCCESS or
        //    ERROR_MORE_DATA depending on the OS / value size.
        let probe = regQueryValueExW(hKey, valueName, null, null, 0);
        if (probe.status !== ERROR_SUCCESS && probe.status !== ERROR_MORE_DATA) {
            throw new Error(
                `RegQueryValueExW('${valueName}') sizing failed with LSTATUS=${probe.status}`,
            );
        }
        const needed = probe.lpcbData;
        if (needed === 0) {
            return '';
        }

        // 3. Allocate a caller-owned Buffer and re-query. The generated
        //    wrapper accepts the Buffer as the opaque `data` param and the
        //    initial size as `lpcbData`; the returned object carries back
        //    both the type discriminator and the number of bytes actually
        //    written.
        const buf = Buffer.alloc(needed);
        const res = regQueryValueExW(hKey, valueName, null, buf, needed);
        if (res.status !== ERROR_SUCCESS) {
            throw new Error(
                `RegQueryValueExW('${valueName}') read failed with LSTATUS=${res.status}`,
            );
        }
        if (res.type !== REG_SZ && res.type !== REG_EXPAND_SZ) {
            throw new Error(
                `Value '${valueName}' has type ${res.type}; expected REG_SZ or REG_EXPAND_SZ`,
            );
        }
        return decodeWideNulTerminated(buf, res.lpcbData);
    } finally {
        // 4. Always release the key handle — codegen exposes this as a
        //    natural single-arg call returning `{ status }`.
        const closeRes = regCloseKey(hKey);
        if (closeRes.status !== ERROR_SUCCESS) {
            // Not fatal, but surface it so leaks are visible in CI logs.
            console.warn(
                `RegCloseKey failed with LSTATUS=${closeRes.status}`,
            );
        }
    }
}

function main() {
    const subKey = 'SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion';
    const valueName = 'ProductName';
    const productName = getString(HKEY_LOCAL_MACHINE, subKey, valueName);

    if (typeof productName !== 'string' || productName.length === 0) {
        console.error(`FAIL: expected non-empty string, got ${JSON.stringify(productName)}`);
        process.exit(1);
    }
    if (!productName.includes('Windows')) {
        console.error(
            `FAIL: expected ProductName to contain 'Windows', got ${JSON.stringify(productName)}`,
        );
        process.exit(1);
    }

    console.log(`ProductName = ${JSON.stringify(productName)}`);
    console.log('PASS');
}

main();
