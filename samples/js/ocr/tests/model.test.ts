// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { AIFeatureReadyState as State } from '#winapp/bindings/microsoft/windows/ai/AIFeatureReadyState'
import { AIFeatureReadyResultState as Result } from '#winapp/bindings/microsoft/windows/ai/AIFeatureReadyResultState'
import { ensureModelReady, recognizedLines } from '../model.ts'
import type { ReadinessApi } from '../model.ts'
import { describeError, withCleanup } from '../support.ts'

const quiet = () => {}
const noPreparation = async () => { assert.fail('model preparation must not be requested') }

test('Ready never requests preparation, including with the opt-in flag', async () => {
    for (const optIn of [false, true]) {
        await ensureModelReady({ getReadyState: () => State.Ready, ensureReadyAsync: noPreparation }, optIn, quiet)
    }
})

test('NotReady is actionable and does not download by default', async () => {
    await assert.rejects(
        ensureModelReady({ getReadyState: () => State.NotReady, ensureReadyAsync: noPreparation }, false, quiet),
        /No download was requested.*--ensure-ready/,
    )
})

for (const state of [State.DisabledByUser, State.NotSupportedOnCurrentSystem]) {
    test(`state ${state} fails even with opt-in and never changes policy`, async () => {
        await assert.rejects(
            ensureModelReady({ getReadyState: () => state, ensureReadyAsync: noPreparation }, true, quiet),
            /disabled by the user|not supported/,
        )
    })
}

test('explicit preparation awaits the typed result, propagates cancellation, and rechecks Ready', async () => {
    const signal = new AbortController().signal
    let ready = false
    let checks = 0
    const messages: string[] = []
    const api: ReadinessApi = {
        getReadyState() { checks++; return ready ? State.Ready : State.NotReady },
        async ensureReadyAsync(actual) {
            assert.equal(actual, signal)
            await Promise.resolve()
            ready = true
            return { status: Result.Success, error: 0, extendedError: 0, errorDisplayText: '' }
        },
    }
    await ensureModelReady(api, true, (message) => messages.push(message), signal)
    assert.equal(checks, 2)
    assert.match(messages[0], /explicitly requested.*download/)
    assert.match(messages[1], /recognition has not run yet/)
})

test('model preparation failures preserve typed status and extended error', async () => {
    for (const status of [Result.Failure, Result.InProgress]) {
        await assert.rejects(ensureModelReady({
            getReadyState: () => State.NotReady,
            ensureReadyAsync: async () => ({
                status, error: -2147024891, extendedError: -2147467259, errorDisplayText: 'owned failure detail',
            }),
        }, true, quiet), /0x80070005.*0x80004005.*owned failure detail/)
    }
})

test('a preparation success flag is not a substitute for Ready', async () => {
    await assert.rejects(ensureModelReady({
        getReadyState: () => State.NotReady,
        ensureReadyAsync: async () => ({ status: Result.Success, error: 0, extendedError: 0, errorDisplayText: '' }),
    }, true, quiet), /still not Ready/)
})

test('readiness call failures keep the capability hint and original error', async () => {
    await assert.rejects(ensureModelReady({
        getReadyState: () => { throw new Error('E_ACCESSDENIED') },
        ensureReadyAsync: noPreparation,
    }, false, quiet), (error: unknown) => {
        assert.match(describeError(error), /systemAIModels/)
        assert.match(describeError(error), /E_ACCESSDENIED/)
        return true
    })
})

test('OCR text comes only from typed RecognizedText.Lines and line.Text, in order', () => {
    let reads = 0
    const result = {
        get lines() {
            reads++
            return [{ text: 'First line' }, { text: '' }, { text: 'Second line' }]
        },
        toString() { assert.fail('IStringable/stringification is not OCR output') },
    }
    assert.deepEqual(recognizedLines(result), ['First line', '', 'Second line'])
    assert.equal(reads, 1)
    assert.deepEqual(recognizedLines({ lines: [] }), [])
    assert.throws(() => recognizedLines(null), /no RecognizedText/)
})

test('cleanup is LIFO, complete on failure, and preserves operation and cleanup errors', async () => {
    const calls: string[] = []
    await assert.rejects(withCleanup(async (defer) => {
        defer('scope', () => calls.push('scope'))
        defer('bitmap', () => { calls.push('bitmap'); throw new Error('owned close failure') })
        defer('image', () => calls.push('image'))
        throw new Error('owned inference failure')
    }), (error: unknown) => {
        assert.ok(error instanceof AggregateError)
        assert.match(describeError(error), /owned inference failure/)
        assert.match(describeError(error), /owned close failure/)
        return true
    })
    assert.deepEqual(calls, ['image', 'bitmap', 'scope'])
})

test('cleanup failures cannot turn into successful OCR output', async () => {
    await assert.rejects(withCleanup(async (defer) => {
        defer('scope', () => { throw new Error('owned release failure') })
        return ['text that must not be reported as success']
    }), /Releasing scope failed/)
    assert.equal(await withCleanup(async () => 42), 42)
})
