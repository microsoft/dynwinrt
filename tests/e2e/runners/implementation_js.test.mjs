// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict';
import test from 'node:test';
import { caseIds, decodeChildResult } from './implementation_js.mjs';

const pass = 'DYNWINRT_IMPLEMENTATION_RESULT={"id":"case","pass":true,"error":null}\n';

test('native case result is preserved only for a successful process', () => {
    assert.equal(decodeChildResult('case', { stdout: pass, status: 0 }).pass, true);
    const failed = decodeChildResult('case', { stdout: pass, status: 3221225477 });
    assert.equal(failed.pass, false);
    assert.match(failed.error, /cleanup/);
});

test('crash, timeout, missing result, and malformed result are failures', () => {
    for (const child of [
        { stdout: '', status: 3221225477 },
        { stdout: '', status: null, error: new Error('ETIMEDOUT') },
        { stdout: 'no marker', status: 0 },
        { stdout: 'DYNWINRT_IMPLEMENTATION_RESULT=invalid', status: 0 },
        { stdout: 'DYNWINRT_IMPLEMENTATION_RESULT={"id":"wrong","pass":true}', status: 0 },
        { stdout: 'DYNWINRT_IMPLEMENTATION_RESULT={"id":"case","pass":"true"}', status: 0 },
    ]) {
        assert.equal(decodeChildResult('case', child).pass, false);
    }
});

test('the whole requested native scenario matrix stays enabled', () => {
    assert.equal(caseIds.length, 16);
    assert.equal(caseIds[0], 'background_task');
    for (const name of ['memory_buffer_event', 'array_contracts', 'fill_array', 'named_outputs']) {
        assert.ok(caseIds.includes(name));
    }
    assert.ok(caseIds.includes('public_view_success_ref_balance'));
    assert.ok(caseIds.includes('public_view_failed_cast_ref_balance'));
    assert.ok(caseIds.includes('nullable_reference_results'));
});
