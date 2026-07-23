// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict';
import { DynWinRtValue, roInitialize } from '../dist/index.js';

roInitialize();

const maxU64 = 0xffff_ffff_ffff_ffffn;
const highBitU64 = 0x8000_0000_0000_0000n;

assert.equal(DynWinRtValue.u64(maxU64).toU64BigInt(), maxU64);
assert.equal(DynWinRtValue.u64(highBitU64).toU64BigInt(), highBitU64);

const factory = DynWinRtValue.activationFactory('Windows.Foundation.Uri');
assert.throws(
    () => DynWinRtValue.pointer(factory),
    /DynWinRtValue inputs are not accepted/,
);

console.log('PASS');
