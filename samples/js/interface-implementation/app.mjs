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
const instanceOwner = IBackgroundTaskInstance.implement({
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
const owner = IBackgroundTask.implement(
    handlers,
    IStringable.implementation(handlers),
    IClosable.implementation(handlers),
);
const instance = IBackgroundTaskInstance.fromImplementation(instanceOwner);
const task = IBackgroundTask.fromImplementation(owner);
const text = IStringable.fromImplementation(owner);
const duplicateText = IStringable.fromImplementation(owner);
const closable = IClosable.fromImplementation(owner);

try {
    assert.notStrictEqual(text, duplicateText);
    releaseProjected(duplicateText);
    // Both the Run call and its progress property calls cross native vtables.
    task.run(instance);
    assert.equal(instance.progress, 1);
    console.log(text.toString());

    owner.release();
    task.run(instance);
    assert.equal(instance.progress, 2);
    closable.close();
    assert.equal(closes, 1);
    releaseProjected(closable);
    assert.equal(owner.isClosed, false);
    console.log(text.toString());

    owner.dispose();
    assert.throws(() => text.toString(), /closed|80000013/i);
    const diagnostic = owner.takeError();
    assert.match(diagnostic, /0x80000013/);
    console.log(diagnostic);
} finally {
    owner.dispose();
    instanceOwner.dispose();
    for (const view of [closable, duplicateText, text, task, instance]) releaseProjected(view);
}
