// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.
//
// Real Node.js E2E: reads a real Windows registry value through the
// high-level `Registry` wrapper and asserts its actual content.
//
// Run: node bindings/js/e2e/registry.mjs
//
// Success criteria:
//   * HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProductName is a
//     non-empty string that contains "Windows".
//   * Reading a non-existent value raises the natural "not found" error
//     from the high-level API (not raw flatInvoke errors).

import assert from 'node:assert/strict';
import {
    Registry,
    RegistryError,
    RegistryValueNotFoundError,
} from './registry.js';

function pass(msg) {
    console.log(`[e2e] PASS: ${msg}`);
}

function fail(msg) {
    console.error(`[e2e] FAIL: ${msg}`);
    process.exit(1);
}

// --------------------------------------------------------------------
// 1. Happy path: read a real registry value through the high-level API
// --------------------------------------------------------------------

console.log('[e2e] step 1: Registry.getString(HKLM, ...\\CurrentVersion, ProductName)');
let productName;
try {
    productName = Registry.getString(
        'HKEY_LOCAL_MACHINE',
        'SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion',
        'ProductName',
    );
} catch (e) {
    fail(`Registry.getString threw unexpectedly: ${e && e.message ? e.message : e}`);
}

console.log(`[e2e]   ProductName = ${JSON.stringify(productName)}`);

try {
    assert.equal(typeof productName, 'string', 'ProductName should be a string');
    assert.ok(productName.length > 0, 'ProductName should be non-empty');
    assert.match(
        productName,
        /Windows/i,
        `ProductName should contain 'Windows' (got ${JSON.stringify(productName)})`,
    );
} catch (e) {
    fail(`content assertion: ${e.message}`);
}
pass('ProductName is a non-empty string containing "Windows"');

// --------------------------------------------------------------------
// 2. Read another real value: BuildLabEx (sanity cross-check on the same
//    key). Not all builds have every value; we only assert non-empty when
//    present. This proves the wrapper handles a second value.
// --------------------------------------------------------------------

console.log('[e2e] step 2: Registry.tryGetString(...BuildLabEx)');
const buildLabEx = Registry.tryGetString(
    'HKLM',
    'SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion',
    'BuildLabEx',
);
console.log(`[e2e]   BuildLabEx = ${JSON.stringify(buildLabEx)}`);
if (buildLabEx !== undefined) {
    assert.equal(typeof buildLabEx, 'string');
    assert.ok(buildLabEx.length > 0);
    pass('BuildLabEx returned a non-empty string');
} else {
    pass('BuildLabEx not present; tryGetString returned undefined');
}

// --------------------------------------------------------------------
// 3. Corner case: missing VALUE raises RegistryValueNotFoundError
//    through the high-level wrapper (not raw flatInvoke output).
// --------------------------------------------------------------------

console.log('[e2e] step 3: missing value must throw RegistryValueNotFoundError');
let missingValueError;
try {
    Registry.getString(
        'HKLM',
        'SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion',
        'ThisValueShouldNeverExist_DynWinrtE2E',
    );
    fail('expected RegistryValueNotFoundError for missing value');
} catch (e) {
    missingValueError = e;
}
assert.ok(
    missingValueError instanceof RegistryValueNotFoundError,
    `expected RegistryValueNotFoundError, got ${missingValueError && missingValueError.constructor.name}`,
);
assert.equal(missingValueError.code, 2, 'LSTATUS should be ERROR_FILE_NOT_FOUND (2)');
pass(`missing value threw ${missingValueError.constructor.name} (${missingValueError.message})`);

// --------------------------------------------------------------------
// 4. Corner case: missing SUBKEY also raises RegistryValueNotFoundError.
// --------------------------------------------------------------------

console.log('[e2e] step 4: missing subkey must throw RegistryValueNotFoundError');
let missingKeyError;
try {
    Registry.getString(
        'HKLM',
        'SOFTWARE\\DynWinrt\\NoSuchKey\\Nope',
        'Anything',
    );
    fail('expected RegistryValueNotFoundError for missing subkey');
} catch (e) {
    missingKeyError = e;
}
assert.ok(
    missingKeyError instanceof RegistryValueNotFoundError,
    `expected RegistryValueNotFoundError, got ${missingKeyError && missingKeyError.constructor.name}`,
);
assert.ok(
    missingKeyError instanceof RegistryError,
    'RegistryValueNotFoundError should extend RegistryError',
);
pass(`missing subkey threw ${missingKeyError.constructor.name} (${missingKeyError.message})`);

// --------------------------------------------------------------------
// 5. tryGetString returns undefined for missing subkey (no throw).
// --------------------------------------------------------------------

console.log('[e2e] step 5: tryGetString returns undefined for missing subkey');
const missing = Registry.tryGetString(
    'HKLM',
    'SOFTWARE\\DynWinrt\\NoSuchKey\\Nope',
    'Anything',
);
assert.equal(missing, undefined);
pass('tryGetString returned undefined for missing subkey');

console.log('');
console.log(`ProductName = ${productName}`);
console.log('PASS');
process.exit(0);
