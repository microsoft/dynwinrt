// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/**
 * Generated WinRT implementation E2E. Every scenario runs in a fresh process:
 * a native crash/hang is a failure, not a reason to lose the rest of the matrix.
 * Only generated factories create implementation objects. Native consumers are
 * generated outbound methods or independently registered stock method handles;
 * no test calls an interface handler directly.
 */
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const filename = fileURLToPath(import.meta.url);
const resultPrefix = 'DYNWINRT_IMPLEMENTATION_RESULT=';
const E_FAIL = 0x80004005;
const RO_E_CLOSED = 0x80000013;
const instanceId = '243aab7d-9411-49af-b66d-0123456789ab';

function notImplemented() {
    throw new Error('Unused standalone E2E fixture method');
}

function expectHresult(action, expected) {
    let error;
    try { action(); } catch (caught) { error = caught; }
    assert.ok(error, `Expected native HRESULT 0x${expected.toString(16)}`);
    const codes = [error.hresult, error.winerror, error.code].filter(value => typeof value === 'number');
    const text = String(error);
    assert.ok(
        codes.some(code => (code >>> 0) === expected) ||
        text.toLowerCase().includes(expected.toString(16)) ||
        text.includes(String(expected | 0)),
        `Expected HRESULT 0x${expected.toString(16)}, got ${error.stack ?? error}`,
    );
    return error;
}

function takeError(owner, pattern) {
    const error = owner.takeError();
    assert.ok(error != null, 'Native callback failure must be available through takeError()');
    assert.match(String(error.message ?? error), pattern);
    assert.equal(owner.takeError(), null, 'takeError() must drain the stored error');
}

function nativeConsumer(runtime, owner, name, iidText, methodName, signature) {
    // Both stock interfaces used here have one method at IInspectable slot 6.
    // Always QI first: toValue() returns the canonical IInspectable pointer,
    // which must NEVER be called as though it were an arbitrary interface view.
    const iid = runtime.WinGuid.parse(iidText);
    const method = runtime.DynWinRtType.registerInterface(`E2E.${name}Consumer`, iid)
        .addMethod(methodName, signature).method(6);
    const canonical = owner.toValue();
    let value;
    try { value = canonical.cast(iid); } finally { canonical.release(); }
    return { invoke: () => method.invoke(value, []), release: () => value.release() };
}

function nativeStringable(runtime, owner) {
    return nativeConsumer(runtime, owner, 'IStringable', '96369f54-8eb6-48f0-abce-c1b211e627c3', 'ToString',
        new runtime.DynWinRtMethodSig().addOut(runtime.DynWinRtType.hstring()));
}

function taskInstanceHandlers(state, g) {
    // IBackgroundTaskInstance has exactly nine slots, 6 through 14. In
    // particular, Canceled is not a made-up success with a fabricated token.
    return {
        getInstanceId() { state.getIds++; return instanceId; },
        getTask: notImplemented,
        getProgress() { state.gets++; return state.progress; },
        setProgress(value) { state.sets++; state.progress = value; },
        getTriggerDetails: notImplemented,
        addCanceled(handler) {
            // This controlled source rejects registration and retains no
            // delegate. Release the visible owned callback view deterministically.
            g.releaseProjected(handler);
            notImplemented();
        },
        removeCanceled: notImplemented,
        getSuspendedCount: notImplemented,
        getDeferral: notImplemented,
    };
}

function readerHandlers(readBytes) {
    // A complete IDataReader plan, not a truncated vtable. Only ReadBytes is
    // used by this fixture; every other metadata slot deliberately fails.
    return {
        getUnconsumedBufferLength: notImplemented,
        getUnicodeEncoding: notImplemented, setUnicodeEncoding: notImplemented,
        getByteOrder: notImplemented, setByteOrder: notImplemented,
        getInputStreamOptions: notImplemented, setInputStreamOptions: notImplemented,
        readByte: notImplemented, readBytes, readBuffer: notImplemented,
        readBoolean: notImplemented, readGuid: notImplemented,
        readInt16: notImplemented, readInt32: notImplemented, readInt64: notImplemented,
        readUInt16: notImplemented, readUInt32: notImplemented, readUInt64: notImplemented,
        readSingle: notImplemented, readDouble: notImplemented, readString: notImplemented,
        readDateTime: notImplemented, readTimeSpan: notImplemented,
        loadAsync: notImplemented, detachBuffer: notImplemented, detachStream: notImplemented,
    };
}

const cases = {
    management_handle({ g, runtime, own }) {
        let calls = 0;
        const impl = own(g.IStringable.implement(
            { toString() { calls++; return 'typed primary'; } },
            { interfaces: [[g.IClosable, { close() { calls++; } }]] },
        ));
        assert.strictEqual(impl.value, impl.value);
        assert.ok(impl.value instanceof g.IStringable);
        assert.equal(impl.value.toString(), 'typed primary');
        const extra = g.IStringable.fromImplementation(impl);
        assert.notStrictEqual(extra, impl.value);
        const closer = g.IClosable.fromImplementation(impl);
        closer.close();
        const primary = impl.value;
        impl.release();
        assert.throws(() => impl.value, /released/);
        assert.throws(() => primary.toString(), /Object|object|released/);
        assert.equal(extra.toString(), 'typed primary');
        impl.dispose();
        expectHresult(() => extra.toString(), RO_E_CLOSED);
        assert.equal(calls, 3);
        g.releaseProjected(extra);
        g.releaseProjected(closer);
        const lazy = own(g.IStringable.implement({ toString: () => 'never projected' }));
        lazy.release();
        assert.throws(() => lazy.value, /released/);
        assert.equal(lazy.isClosed, true);
        for (const interfaces of [[[g.IStringable, {}]], [[null, {}]], [[g.IClosable]], [[g.IStringable, { toString: () => '' }]]]) {
            assert.throws(() => g.IStringable.implement({ toString: () => '' }, { interfaces }), /handler|entry|entries|Duplicate|duplicate/);
        }
        let failedOwner;
        class Broken extends g.IStringable {
            static fromImplementation(owner) {
                failedOwner = owner;
                throw new Error('primary view failure');
            }
        }
        const broken = own(Broken.implement({ toString: () => 'unreachable' }));
        assert.throws(() => broken.value, /primary view failure/);
        assert.equal(failedOwner.isClosed, true);
        assert.throws(() => broken.value, /closed|released/);
        const wrong = own(g.IStringable.implement({ toString: () => 17 }));
        expectHresult(() => wrong.value.toString(), E_FAIL);
        takeError(wrong, /invalid implementation result/);
        let reentrant;
        class ReleasedDuringProjection extends g.IStringable {
            static fromImplementation(owner) {
                const value = super.fromImplementation(owner);
                reentrant.release();
                return value;
            }
        }
        reentrant = own(ReleasedDuringProjection.implement({ toString: () => '' }));
        assert.throws(() => reentrant.value, /released during/);
        assert.equal(reentrant.isClosed, true);
        const disposing = own(g.IStringable.implement({
            toString() { disposing.dispose(); return 'entered callback completed'; },
        }));
        assert.equal(disposing.value.toString(), 'entered callback completed');
        assert.equal(disposing.isClosed, true);
        assert.throws(() => disposing.value, /closed|released/);
    },

    property_views({ g, own }) {
        let length = 0;
        const handlers = {
            getCapacity: () => 12, getLength: () => length,
            setLength(value) { if (value > 12) throw new Error('capacity exceeded'); length = value; },
            getAbsoluteCanonicalUri: () => 'https://example.test/\u96ea',
            getDisplayIri: () => 'https://example.test/\u96ea',
            getName: () => 'query', getValue: () => 'value\0\u96ea',
        };
        const impl = own(g.IBuffer.implement(handlers, {
            interfaces: [[g.IUriRuntimeClassWithAbsoluteCanonicalUri, handlers], [g.IWwwFormUrlDecoderEntry, handlers]],
        }));
        const uri = g.IUriRuntimeClassWithAbsoluteCanonicalUri.fromImplementation(impl);
        const entry = g.IWwwFormUrlDecoderEntry.fromImplementation(impl);
        assert.equal(impl.value.capacity, 12);
        assert.equal(impl.value.length, 0);
        impl.value.length = 7;
        assert.equal(impl.value.length, 7);
        assert.equal(uri.absoluteCanonicalUri, 'https://example.test/\u96ea');
        assert.equal(uri.displayIri, 'https://example.test/\u96ea');
        assert.equal(entry.name, 'query');
        assert.equal(entry.value, 'value\0\u96ea');
        expectHresult(() => { impl.value.length = 13; }, E_FAIL);
        takeError(impl, /capacity exceeded/);
        assert.equal(impl.value.length, 7);
        impl.value.length = 0;
        assert.equal(impl.value.length, 0);
        g.releaseProjected(uri);
        g.releaseProjected(entry);
    },

    background_task({ g, runtime, own }) {
        const state = { progress: 17, gets: 0, sets: 0, getIds: 0 };
        const instanceOwner = own(g.IBackgroundTaskInstance.implement(taskInstanceHandlers(state, g)));
        const instance = instanceOwner.value;
        let runs = 0;
        const taskOwner = own(g.IBackgroundTask.implement({
            run(taskInstance) {
                assert.ok(taskInstance instanceof g.IBackgroundTaskInstance);
                assert.equal(taskInstance.instanceId.toLowerCase(), instanceId);
                taskInstance.progress = taskInstance.progress + 5;
                runs++;
            },
        }));
        const task = taskOwner.value;
        task.run(instance);
        assert.deepEqual(state, { progress: 22, gets: 1, sets: 1, getIds: 1 });
        assert.equal(runs, 1);
        const retainedInstance = g.IBackgroundTaskInstance.fromImplementation(instanceOwner);
        const retainedTask = g.IBackgroundTask.fromImplementation(taskOwner);
        instanceOwner.release();
        taskOwner.release();
        retainedTask.run(retainedInstance);
        assert.deepEqual(state, { progress: 27, gets: 2, sets: 2, getIds: 2 });
        assert.equal(runs, 2);
        const token = runtime.DynWinRtStruct.create(runtime.DynWinRtType.structType(
            'Windows.Foundation.EventRegistrationToken', [runtime.DynWinRtType.i64()],
        ));
        token.setI64(0, 1n);
        let canceled = 0;
        for (const action of [
            () => retainedInstance.task,
            () => retainedInstance.triggerDetails,
            () => retainedInstance.suspendedCount,
            () => retainedInstance.getDeferral(),
            () => retainedInstance.onCanceled(() => { canceled++; }),
            () => retainedInstance.offCanceled(token.toValue()),
        ]) {
            expectHresult(action, E_FAIL);
            takeError(instanceOwner, /unused standalone/i);
        }
        assert.equal(canceled, 0, 'A rejected subscription must not invoke its delegate');
    },

    multi_interface_lifetime({ g, own }) {
        let closes = 0;
        const owner = own(g.IStringable.implement(
            { toString: () => 'retained \0 WinRT \u96ea' },
            g.IClosable.implementation({ close() { closes++; } }),
        ));
        const first = g.IStringable.fromImplementation(owner);
        const second = g.IStringable.fromImplementation(owner);
        const closable = g.IClosable.fromImplementation(owner);
        assert.notEqual(first, second, 'Typed views must not reuse one mutable native holder');
        const retained = owner.toValue();
        try {
            // The public lifetime helper releases only this view's QI reference.
            g.releaseProjected(first);
            assert.equal(second.toString(), 'retained \0 WinRT \u96ea');
            owner.release();
            owner.release();
            assert.equal(owner.isClosed, false, 'release() is not dispose()');
            assert.equal(second.toString(), 'retained \0 WinRT \u96ea');
            closable.close();
            assert.equal(closes, 1);
            const nativeRetainedView = g.IStringable.from(retained);
            retained.release();
            assert.equal(nativeRetainedView.toString(), 'retained \0 WinRT \u96ea');
            g.releaseProjected(nativeRetainedView);
            assert.equal(owner.takeError(), null);
        } finally { retained.release(); }
    },

    dispose_disconnects({ g, runtime, own }) {
        let calls = 0;
        const owner = own(g.IStringable.implement(
            { toString() { calls++; return 'before disposal'; } },
            g.IClosable.implementation({ close() { calls++; } }),
        ));
        const text = nativeStringable(runtime, owner);
        const close = nativeConsumer(runtime, owner, 'IClosable', '30d5a829-7fa4-4026-83bb-d75bae4ea99e', 'Close',
            new runtime.DynWinRtMethodSig());
        try {
            assert.equal(text.invoke().toString(), 'before disposal');
            close.invoke();
            owner.release();
            assert.equal(text.invoke().toString(), 'before disposal');
            assert.equal(owner.isClosed, false);
            owner.dispose();
            owner.dispose();
            assert.equal(owner.isClosed, true);
            expectHresult(text.invoke, RO_E_CLOSED);
            expectHresult(close.invoke, RO_E_CLOSED);
            assert.equal(calls, 3, 'Disposed native views must not enter any handler');
        } finally { close.release(); text.release(); }
    },

    reentrant_dispose({ g, runtime, own }) {
        let calls = 0;
        const owner = own(g.IStringable.implement({
            toString() { calls++; owner.dispose(); return 'active callback finished'; },
        }));
        const view = nativeStringable(runtime, owner);
        try {
            assert.equal(view.invoke().toString(), 'active callback finished');
            assert.equal(owner.isClosed, true);
            expectHresult(view.invoke, RO_E_CLOSED);
            assert.equal(calls, 1);
        } finally { view.release(); }
    },

    callback_error({ g, runtime, own }) {
        let fail = true;
        const owner = own(g.IStringable.implement({
            toString() {
                if (fail) throw new Error('implementation-e2e-js-sentinel');
                return 'recovered';
            },
        }));
        const view = nativeStringable(runtime, owner);
        try {
            expectHresult(view.invoke, E_FAIL);
            takeError(owner, /implementation-e2e-js-sentinel/);
            fail = false;
            assert.equal(view.invoke().toString(), 'recovered');
            assert.equal(owner.isClosed, false);
            assert.equal(owner.takeError(), null);
        } finally { view.release(); }
    },

    async_handler_rejected({ g }) {
        let entered = false;
        assert.throws(
            () => g.IStringable.implementation({ async toString() { entered = true; return 'bad'; } }),
            /async|synchronous/i,
        );
        assert.throws(
            () => g.IClosable.implementation({ async close() { entered = true; } }),
            /async|synchronous/i,
        );
        assert.equal(entered, false);
    },

    async_result_rejected({ g, runtime, own }) {
        const owner = own(g.IStringable.implement({ toString: () => Promise.resolve('not a WinRT result') }));
        const view = nativeStringable(runtime, owner);
        try {
            expectHresult(view.invoke, E_FAIL);
            takeError(owner, /promise|async|synchronous/i);
        } finally { view.release(); }
    },

    required_interfaces({ g, own }) {
        const handlers = { getCapacity: () => 73, addClosed: notImplemented, removeClosed: notImplemented };
        assert.throws(
            () => g.IMemoryBufferReference.implement(handlers),
            /required|IClosable|30d5a829/i,
        );
        const owner = own(g.IMemoryBufferReference.implement(
            handlers, g.IClosable.implementation({ close() {} }),
        ));
        assert.equal(g.IMemoryBufferReference.fromImplementation(owner).capacity, 73);
        g.IClosable.fromImplementation(owner).close();
        assert.throws(
            () => g.IStringable.implement({ toString: () => '' }, g.IStringable.implementation({ toString: () => '' })),
            /duplicate|already|IID/i,
        );
        const onlyStringable = own(g.IStringable.implement({ toString: () => 'not IClosable' }));
        expectHresult(() => g.IClosable.fromImplementation(onlyStringable), 0x80004002);
    },

    memory_buffer_event({ g, own }) {
        const callbacks = new Map();
        let added = 0;
        let removed = 0;
        let delivered = 0;
        let eventSender;
        const owner = own(g.IMemoryBufferReference.implement({
            getCapacity: () => 4096,
            addClosed(handler) {
                assert.ok(handler != null, 'Closed receives a native delegate');
                assert.equal(typeof handler, 'function', 'Received delegates are typed callable native views');
                callbacks.set(++added, handler);
                return { value: BigInt(added) };
            },
            removeClosed(token) {
                assert.equal(typeof token.value, 'bigint');
                const received = callbacks.get(Number(token.value));
                assert.ok(received);
                assert.equal(callbacks.delete(Number(token.value)), true);
                g.releaseProjected(received);
                removed++;
            },
        }, g.IClosable.implementation({
            close() {
                for (const callback of [...callbacks.values()]) {
                    // This is a generated wrapper over native delegate slot 3,
                    // not the original JavaScript event callback.
                    callback(eventSender, null);
                }
            },
        })));
        eventSender = g.IMemoryBufferReference.fromImplementation(owner);
        const close = g.IClosable.fromImplementation(owner);
        const unsubscribe = eventSender.onClosed((sender, args) => {
            assert.ok(sender instanceof g.IMemoryBufferReference);
            assert.equal(sender.capacity, 4096); // Reentrant native property call.
            assert.equal(args, null);
            delivered++;
        });
        assert.equal(added, 1);
        owner.release();
        close.close();
        assert.equal(delivered, 1, 'Close must invoke the delegate through its native vtable');
        unsubscribe();
        assert.equal(removed, 1);
        close.close();
        assert.equal(delivered, 1);
        owner.dispose();
        expectHresult(() => close.close(), RO_E_CLOSED);
    },

    array_contracts({ g, runtime, own }) {
        // Public, non-exclusive SDK interfaces: do not expose an ExclusiveTo
        // factory interface just to obtain a convenient array fixture.
        const state = { bytes: Buffer.alloc(0), text: '', guid: instanceId, signed: 0n, unsigned: 0n, real: 0 };
        const writerHandlers = Object.fromEntries([
            'getUnstoredBufferLength', 'getUnicodeEncoding', 'setUnicodeEncoding',
            'getByteOrder', 'setByteOrder', 'writeByte', 'writeBufferRange',
            'writeBoolean', 'writeInt16', 'writeInt32', 'writeUInt16', 'writeUInt32',
            'writeSingle', 'writeDateTime', 'writeTimeSpan', 'storeAsync', 'flushAsync',
            'detachStream',
        ].map(name => [name, notImplemented]));
        Object.assign(writerHandlers, {
            writeBytes(value) {
                assert.ok(Array.isArray(value), 'PassArray input is a projected number[]');
                state.bytes = Buffer.from(value);
            },
            writeBuffer(value) {
                assert.ok(value instanceof g.IBuffer);
                state.bytes = value.toBuffer();
            },
            writeString(value) { state.text = value; return Buffer.byteLength(value); },
            measureString: value => Buffer.byteLength(value),
            writeGuid: value => { state.guid = value; },
            writeInt64: value => { state.signed = value; },
            writeUInt64: value => { state.unsigned = value; },
            writeDouble: value => { state.real = value; },
            detachBuffer: () => g.IBuffer.fromBuffer(state.bytes),
        });
        const writerOwner = own(g.IDataWriter.implement(writerHandlers, g.IClosable.implementation({ close() {} })));
        const writer = g.IDataWriter.fromImplementation(writerOwner);
        const propertyHandlers = Object.fromEntries([
            'getType', 'getIsNumericScalar', 'getUInt8', 'getInt16', 'getUInt16', 'getInt32',
            'getUInt32', 'getSingle', 'getChar16', 'getBoolean', 'getDateTime', 'getTimeSpan',
            'getPoint', 'getSize', 'getRect', 'getInt16Array', 'getUInt16Array', 'getInt32Array',
            'getUInt32Array', 'getSingleArray', 'getChar16Array', 'getBooleanArray',
            'getDateTimeArray', 'getTimeSpanArray', 'getSizeArray', 'getRectArray',
        ].map(name => [name, notImplemented]));
        const objectOwner = own(g.IStringable.implement({ toString: () => 'array object' }));
        const object = objectOwner.toValue();
        let invalidElement = false;
        Object.assign(propertyHandlers, {
            getUInt8Array: () => state.bytes,
            getString: () => state.text,
            getStringArray: () => invalidElement ? [state.text, 42] : [state.text, '', 'snow \u96ea'],
            getGuid: () => state.guid,
            getGuidArray: () => [state.guid, instanceId],
            getInt64: () => state.signed,
            getInt64Array: () => [state.signed, -(1n << 63n)],
            getUInt64: () => state.unsigned,
            getUInt64Array: () => [state.unsigned, (1n << 64n) - 1n],
            getDouble: () => state.real,
            getDoubleArray: () => [state.real, -2.5],
            getPointArray: () => [{ x: 1.25, y: -3.5 }, { x: 0, y: 2 }],
            getInspectableArray: () => [object, null],
        });
        const owner = own(g.IPropertyValue.implement(propertyHandlers));
        const view = g.IPropertyValue.fromImplementation(owner);
        writerOwner.release();
        owner.release();
        for (const bytes of [Buffer.alloc(0), Buffer.from([0, 1, 127, 128, 255])]) {
            writer.writeBytes(bytes);
            assert.deepEqual(view.getUInt8Array(), bytes);
            const native = writer.detachBuffer();
            assert.ok(native instanceof g.IBuffer);
            assert.equal(native.length, bytes.length);
            assert.deepEqual(native.toBuffer(), bytes);
            writer.writeBuffer(native);
            assert.deepEqual(view.getUInt8Array(), bytes);
            g.releaseProjected(native);
        }
        const text = 'generated \0 WinRT \u96ea \ud83d\ude80';
        assert.equal(writer.writeString(text), writer.measureString(text));
        assert.equal(view.getString(), text);
        assert.deepEqual(view.getStringArray(), [text, '', 'snow \u96ea']);
        writer.writeGuid(instanceId);
        assert.equal(view.getGuid().toLowerCase(), instanceId);
        assert.deepEqual(view.getGuidArray().map(value => value.toLowerCase()), [instanceId, instanceId]);
        writer.writeInt64((1n << 63n) - 1n);
        writer.writeUInt64((1n << 64n) - 1n);
        writer.writeDouble(1.25);
        assert.equal(view.getInt64(), (1n << 63n) - 1n);
        assert.equal(view.getUInt64(), (1n << 64n) - 1n);
        assert.equal(view.getDouble(), 1.25);
        assert.deepEqual(view.getInt64Array(), [(1n << 63n) - 1n, -(1n << 63n)]);
        assert.deepEqual(view.getUInt64Array(), [(1n << 64n) - 1n, (1n << 64n) - 1n]);
        assert.deepEqual(view.getDoubleArray(), [1.25, -2.5]);
        assert.deepEqual(view.getPointArray(), [{ x: 1.25, y: -3.5 }, { x: 0, y: 2 }]);
        const objects = view.getInspectableArray();
        assert.equal(objects.length, 2);
        const textView = g.IStringable.from(objects[0]);
        assert.equal(textView.toString(), 'array object');
        assert.equal(objects[1], null);
        g.releaseProjected(textView);
        objects.forEach(value => value?.release());
        invalidElement = true;
        expectHresult(() => view.getStringArray(), E_FAIL);
        takeError(owner, /getStringArray|invalid implementation result/i);
        invalidElement = false;
        assert.deepEqual(view.getStringArray(), [text, '', 'snow \u96ea']);
        assert.equal(owner.takeError(), null);
        assert.equal(writerOwner.takeError(), null);
        object.release();
        g.releaseProjected(view);
        g.releaseProjected(writer);
    },

    value_shapes({ g, own }) {
        const values = {
            UInt8: 255, Int16: -32768, UInt16: 65535, Int32: -(2 ** 31), UInt32: 2 ** 32 - 1,
            Int64: -(1n << 63n), UInt64: (1n << 64n) - 1n,
            Single: 1.25, Double: -2.5, Char16: 0x96ea, Boolean: true,
            String: 'value\0\u96ea', Guid: instanceId,
            DateTime: { universalTime: 132537600000000000n }, TimeSpan: { duration: -1250n },
            Point: { x: 1.25, y: -3.5 }, Size: { width: 3.5, height: 2.25 },
            Rect: { x: 1.25, y: 2.5, width: 3.75, height: 4 },
        };
        const called = new Set();
        const getter = (name, value) => () => { called.add(name); return value; };
        const handlers = {
            getType: getter('type', g.PropertyType.Int32),
            getIsNumericScalar: getter('numeric', true),
            getInspectableArray: getter('inspectable', [null]),
        };
        for (const [name, value] of Object.entries(values)) {
            handlers[`get${name}`] = getter(name, value);
            handlers[`get${name}Array`] = getter(`${name}[]`, [value, value]);
        }
        const impl = own(g.IPropertyValue.implement(handlers));
        assert.equal(impl.value.type, g.PropertyType.Int32);
        assert.equal(impl.value.isNumericScalar, true);
        assert.deepEqual(impl.value.getInspectableArray(), [null]);
        for (const [name, expected] of Object.entries(values)) {
            const actual = impl.value[`get${name}`]();
            const array = impl.value[`get${name}Array`]();
            assert.deepEqual(actual, expected);
            assert.deepEqual(array, name === 'UInt8' ? Buffer.from([expected, expected]) : [expected, expected]);
        }
        assert.equal(called.size, Object.keys(values).length * 2 + 3);
    },

    fill_array({ g, own }) {
        const capacities = [];
        const owner = own(g.IDataReader.implement(readerHandlers(capacity => {
            assert.equal(typeof capacity, 'number');
            capacities.push(capacity);
            return Uint8Array.from({ length: capacity }, (_, index) => (index * 17 + 3) & 255);
        }), g.IClosable.implementation({ close() {} })));
        const view = g.IDataReader.fromImplementation(owner);
        for (const capacity of [0, 1, 7, 257]) {
            const actual = view.readBytes(new Uint8Array(capacity));
            assert.deepEqual(Array.from(actual), Array.from({ length: capacity }, (_, index) => (index * 17 + 3) & 255));
        }
        assert.deepEqual(capacities, [0, 1, 7, 257], 'FillArray logical input must be U32 capacity, not an array');
        expectHresult(() => view.readByte(), E_FAIL);
        takeError(owner, /unused standalone/i);
    },

    fill_array_wrong_length({ g, own }) {
        let wrong = true;
        const owner = own(g.IDataReader.implement(readerHandlers(capacity => new Uint8Array(capacity - (wrong ? 1 : 0))),
            g.IClosable.implementation({ close() {} })));
        const view = g.IDataReader.fromImplementation(owner);
        assert.throws(() => view.readBytes(new Uint8Array(4)), /length|capacity|80070057|invalid/i);
        takeError(owner, /capacity|length/i);
        wrong = false;
        assert.equal(view.readBytes(new Uint8Array(4)).length, 4);
        assert.equal(owner.takeError(), null);
    },

    named_outputs({ g, own }) {
        let named = true;
        const owner = own(g.IBindableVectorView.implement({
            getAt(index) { assert.equal(index, 0); return null; },
            getSize: () => 1,
            // Deliberately reverse object field insertion order. Metadata out
            // params come first at the ABI, then the method return, by NAME.
            indexOf(value) {
                return named ? { result: value === null, index: value === null ? 0 : 0xffffffff } : [0, true];
            },
        }, g.IBindableIterable.implementation({
            first() {
                let current = true;
                const iteratorOwner = own(g.IBindableIterator.implement({
                    getCurrent: () => null,
                    getHasCurrent: () => current,
                    moveNext() { current = false; return false; },
                }));
                return g.IBindableIterator.fromImplementation(iteratorOwner);
            },
        })));
        const view = g.IBindableVectorView.fromImplementation(owner);
        assert.equal(view.size, 1);
        assert.equal(view.getAt(0), null);
        // Existing outbound projections preserve their tuple shape. Inbound
        // implementation handlers use the new named multi-output shape.
        assert.deepEqual(view.indexOf(null), [0, true]);
        const other = own(g.IStringable.implement({ toString: () => 'not in the vector' }));
        const raw = other.toValue();
        try { assert.deepEqual(view.indexOf(raw), [0xffffffff, false]); }
        finally { raw.release(); }
        const iterator = g.IBindableIterable.fromImplementation(owner).first();
        assert.equal(iterator.hasCurrent, true);
        assert.equal(iterator.current, null);
        assert.equal(iterator.moveNext(), false);
        assert.equal(iterator.hasCurrent, false);
        assert.equal(owner.takeError(), null);
        named = false;
        expectHresult(() => view.indexOf(null), E_FAIL);
        takeError(owner, /named|field/i);
        named = true;
        assert.deepEqual(view.indexOf(null), [0, true]);
    },

    nullable_reference_results({ g, own }) {
        const owner = own(g.INumberParser.implement({
            parseInt(text) {
                assert.equal(typeof text, 'string');
                if (!/^-?\d+$/.test(text)) return null;
                const value = BigInt(text);
                return value >= -(1n << 63n) && value < (1n << 63n) ? value : null;
            },
            parseUInt(text) {
                assert.equal(typeof text, 'string');
                if (!/^\d+$/.test(text)) return null;
                const value = BigInt(text);
                return value < (1n << 64n) ? value : null;
            },
            parseDouble(text) {
                assert.equal(typeof text, 'string');
                const value = Number(text);
                return Number.isFinite(value) ? value : null;
            },
        }));
        const view = g.INumberParser.fromImplementation(owner);
        owner.release();
        try {
            for (const value of [-(1n << 63n), 0n, (1n << 63n) - 1n]) {
                assert.equal(view.parseInt(String(value)), value);
            }
            for (const value of [0n, (1n << 64n) - 1n]) {
                assert.equal(view.parseUInt(String(value)), value);
            }
            assert.equal(view.parseInt('9223372036854775808'), null);
            assert.equal(view.parseUInt('-1'), null);
            assert.equal(view.parseUInt('18446744073709551616'), null);
            assert.equal(view.parseInt('invalid'), null);
            assert.equal(view.parseDouble('2.5'), 2.5);
            assert.equal(view.parseDouble('-0.125'), -0.125);
            assert.equal(view.parseDouble('invalid'), null);
            assert.equal(owner.takeError(), null);
        } finally { g.releaseProjected(view); }
    },

    public_view_success_ref_balance({ g, runtime, own }) {
        const owner = own(g.IStringable.implement({ toString: () => 'balanced public view' }));
        const view = g.IStringable.fromImplementation(owner);
        try {
            assert.equal(view.toString(), 'balanced public view');
        } finally { g.releaseProjected(view); }
        assert.equal(owner.isClosed, false, 'Releasing the view must leave the owner usable');
        const consumer = nativeStringable(runtime, owner);
        try { assert.equal(consumer.invoke().toString(), 'balanced public view'); }
        finally { consumer.release(); }
        owner.release();
        // No GC or dispose before this assertion: there are no intentional
        // native aliases left, so a canonical temporary cannot be hidden.
        assert.equal(owner.isClosed, true, 'Successful fromImplementation leaked a native reference');
    },

    public_view_failed_cast_ref_balance({ g, runtime, own }) {
        const owner = own(g.IStringable.implement({ toString: () => 'still usable after failed QI' }));
        expectHresult(() => g.IClosable.fromImplementation(owner), 0x80004002);
        assert.equal(owner.isClosed, false, 'A failed conversion must not dispose the owner');
        const consumer = nativeStringable(runtime, owner);
        try { assert.equal(consumer.invoke().toString(), 'still usable after failed QI'); }
        finally { consumer.release(); }
        owner.release();
        assert.equal(owner.isClosed, true, 'Failed fromImplementation leaked its canonical temporary');
    },
};

export const caseIds = Object.freeze(Object.keys(cases));

export function decodeChildResult(id, child) {
    const line = (child.stdout ?? '').split(/\r?\n/).findLast(value => value.startsWith(resultPrefix));
    let result = {
        id, pass: false,
        error: `Native child did not report a result (exit=${child.status}, signal=${child.signal}, error=${child.error ?? 'none'})\n${child.stdout ?? ''}`,
    };
    if (line) {
        try {
            const payload = JSON.parse(line.slice(resultPrefix.length));
            if (payload.id !== id || typeof payload.pass !== 'boolean') throw new Error('Invalid case result envelope');
            result = payload;
        } catch (error) {
            result.error = `Invalid native child result: ${error}`;
        }
    }
    if (child.status !== 0 && result.pass) {
        result = { ...result, pass: false, error: `Native child failed during cleanup (exit=${child.status}, ${child.error ?? ''})` };
    }
    return { ...result, exit_code: child.status, stderr: child.stderr ?? '' };
}

function options() {
    const result = {};
    for (let index = 2; index < process.argv.length; index += 2) {
        const key = process.argv[index];
        if (!key.startsWith('--') || !process.argv[index + 1]) throw new Error(`Invalid argument: ${key}`);
        result[key.slice(2)] = process.argv[index + 1];
    }
    for (const key of ['generated', 'runtime']) {
        if (!result[key]) throw new Error(`--${key} is required`);
        result[key] = path.resolve(result[key]);
    }
    if (result.case && !Object.hasOwn(cases, result.case)) throw new Error(`Unknown case: ${result.case}`);
    return result;
}

function runOne(id, args) {
    const started = performance.now();
    const owners = [];
    let error = null;
    try {
        const runtime = require(args.runtime);
        assert.equal(typeof runtime.DynWinRtImplementation, 'function', 'Build the JS runtime with WinRT implementation support');
        runtime.roInitialize(1);
        const g = require(path.join(args.generated, 'index.js'));
        cases[id]({ g, runtime, own(value) { owners.push(value); return value; } });
    } catch (caught) {
        error = caught.stack ?? String(caught);
    } finally {
        for (const owner of owners.reverse()) {
            try { owner.dispose(); } catch (caught) { error ??= caught.stack ?? String(caught); }
        }
    }
    return { id, pass: error === null, error, duration_ms: Math.round(performance.now() - started) };
}

function main() {
    const args = options();
    if (args.case) {
        const result = runOne(args.case, args);
        console.log(resultPrefix + JSON.stringify(result));
        if (!result.pass) process.exitCode = 1;
        return;
    }
    const results = [];
    const printedErrors = new Set();
    console.log(`Node ${process.version}, ${process.arch}, ${process.execPath}`);
    const selected = args.cases ? args.cases.split(',') : caseIds;
    if (selected.some(id => !Object.hasOwn(cases, id))) throw new Error('Unknown implementation scenario in --cases');
    for (const id of selected) {
        const child = spawnSync(process.execPath, [
            filename, '--case', id, '--generated', args.generated, '--runtime', args.runtime,
        ], { encoding: 'utf8', timeout: 90_000, maxBuffer: 8 * 1024 * 1024, windowsHide: true });
        const result = decodeChildResult(id, child);
        results.push(result);
        console.log(`  ${result.pass ? 'PASS' : 'FAIL'} ${id}`);
        if (!result.pass) {
            const diagnostic = `${result.error}\n${result.stderr}`;
            const key = `${String(result.error).split('\n')[0]}\n${result.stderr}`;
            if (!printedErrors.has(key)) {
                console.log(diagnostic);
                printedErrors.add(key);
            }
        }
    }
    const report = {
        language: 'ts-implementations', architecture: process.arch, runtime: process.version,
        executable: process.execPath, binding: args.runtime, generated: args.generated,
        passed: results.filter(result => result.pass).length, total: results.length, results,
    };
    console.log(`Node implementation E2E: ${report.passed}/${report.total} passed (${report.architecture})`);
    if (args.output) {
        mkdirSync(path.dirname(path.resolve(args.output)), { recursive: true });
        writeFileSync(args.output, JSON.stringify(report, null, 2) + '\n');
    }
    if (report.passed !== report.total) process.exitCode = 1;
}

if (process.argv[1] && path.resolve(process.argv[1]) === filename) main();
