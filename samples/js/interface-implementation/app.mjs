// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict';
import { roInitialize } from '@microsoft/dynwinrt';
import {
    IBackgroundTask,
    IBackgroundTaskInstance,
    IClosable,
    IStringable,
    releaseProjected,
} from './generated/index.js';

roInitialize(1);

function unavailable() {
    throw new Error('This local task-instance fixture has no OS registration, trigger, or deferral');
}

let progress = 0;
let runs = 0;
let closes = 0;
const instanceImpl = IBackgroundTaskInstance.implement({
    getInstanceId: () => '4ab7eea7-29a1-4f48-a6da-22894e210ec0',
    getTask: unavailable,
    getProgress: () => progress,
    setProgress: value => { progress = value; },
    getTriggerDetails: unavailable,
    addCanceled: unavailable,
    removeCanceled: unavailable,
    getSuspendedCount: () => 0,
    getDeferral: unavailable,
});
const handlers = {
    run(instance) {
        if (instance === null) throw new TypeError('A task instance is required by this handler');
        instance.progress = instance.progress + 1;
        runs += 1;
    },
    toString: () => `JavaScript task: ${runs} native Run calls`,
    close: () => { closes += 1; },
};
const impl = IBackgroundTask.implement(handlers, {
    interfaces: [[IStringable, handlers], [IClosable, handlers]],
});
const text = IStringable.fromImplementation(impl);
const duplicateText = IStringable.fromImplementation(impl);
const closable = IClosable.fromImplementation(impl);

try {
    assert.notStrictEqual(text, duplicateText);
    releaseProjected(duplicateText);
    // Both the Run call and its progress property calls cross native vtables.
    assert.strictEqual(impl.value, impl.value);
    impl.value.run(instanceImpl.value);
    assert.equal(instanceImpl.value.progress, 1);
    console.log(text.toString());

    const independentTask = IBackgroundTask.fromImplementation(impl);
    impl.release();
    assert.throws(() => impl.value, /released/);
    independentTask.run(instanceImpl.value);
    releaseProjected(independentTask);
    assert.equal(instanceImpl.value.progress, 2);
    closable.close();
    assert.equal(closes, 1);
    releaseProjected(closable);
    assert.equal(impl.isClosed, false);
    console.log(text.toString());

    impl.dispose();
    assert.throws(() => text.toString(), /closed|80000013/i);
    const diagnostic = impl.takeError();
    assert.match(diagnostic, /0x80000013/);
    console.log(diagnostic);
} finally {
    impl.dispose();
    instanceImpl.dispose();
    for (const view of [closable, duplicateText, text]) releaseProjected(view);
}
