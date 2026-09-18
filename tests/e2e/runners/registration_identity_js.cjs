// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const assert = require('node:assert/strict');
const path = require('node:path');

const [generated, runtimePath, order] = process.argv.slice(2);
const runtime = require(path.resolve(runtimePath));
runtime.roInitialize(1);
const results = [];

for (const name of order.split(',')) {
    const g = require(path.join(path.resolve(generated), `${name}_js`));
    let added = 0;
    let removed = 0;
    let retained;
    const owner = g.INotifyPropertyChanged.implement({
        addPropertyChanged(callback) {
            assert.equal(typeof callback, 'function');
            retained = callback;
            added++;
            return { value: 1n };
        },
        removePropertyChanged(token) {
            assert.equal(token.value, 1n);
            g.releaseProjected(retained);
            retained = undefined;
            removed++;
        },
    });
    try {
        const unsubscribe = owner.value.onPropertyChanged(() => {});
        unsubscribe();
        assert.equal(added, 1);
        assert.equal(removed, 1);
        assert.equal(owner.takeError(), null);
        results.push({ name, added, removed });
    } finally {
        if (retained) g.releaseProjected(retained);
        owner.dispose();
    }
}
console.log(JSON.stringify({ runtime: require.resolve(path.resolve(runtimePath)), results }));
