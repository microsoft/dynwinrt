// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const assert = require('node:assert/strict');
const vm = require('node:vm');

let native;
let invocations = 0;
let invokedSlot;
const signature = {
  addIn() {
    return this;
  },
  addOut() {
    return this;
  },
};
const iface = {
  addMethod() {
    return this;
  },
  method(slot) {
    return {
      invoke() {
        invocations++;
        invokedSlot = slot;
        return native;
      },
    };
  },
};
const runtime = {
  WinGuid: { parse: (value) => value },
  DynWinRtType: {
    registerInterface() {
      return iface;
    },
    u32() {},
    object() {},
    iAsyncActionWithProgress() {},
    iAsyncOperationWithProgress() {},
  },
  DynWinRtMethodSig: function () {
    return signature;
  },
  DynWinRtValue: { u32: (value) => value },
};
const generated = {};
vm.runInNewContext(generatedSource, {
  exports: generated,
  Promise,
  AbortSignal,
  require(name) {
    if (name === '@microsoft/dynwinrt') return runtime;
    assert.equal(name, './lifetime.js');
    return {};
  },
});
const probe = Object.create(generated.ProgressProbe.prototype);
probe._obj = {
  cast() {
    return this;
  },
};

function nativeOperation() {
  let resolve, reject, callback;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  const state = {
    registrations: 0,
    results: 0,
    progress: 0,
    cancels: 0,
    error: new Error('native cancellation'),
    toPromise() {
      assert.equal(++this.registrations, 1, 'native completion registered twice');
      return promise;
    },
    onProgress(cb) {
      callback = cb;
    },
    report(value) {
      callback({
        toNumber() {
          state.progress++;
          return value;
        },
      });
    },
    complete() {
      resolve({
        toNumber() {
          state.results++;
          return 42;
        },
      });
    },
    fail(error) {
      reject(error);
    },
    cancel() {
      this.cancels++;
      reject(this.error);
    },
  };
  return state;
}

function trackedSignal() {
  const controller = new AbortController();
  const signal = controller.signal;
  let added = 0,
    removed = 0;
  const add = signal.addEventListener.bind(signal);
  const remove = signal.removeEventListener.bind(signal);
  signal.addEventListener = (...args) => {
    added++;
    add(...args);
  };
  signal.removeEventListener = (...args) => {
    removed++;
    remove(...args);
  };
  return { controller, signal, counts: () => [added, removed] };
}

async function main() {
  const cases = [
    ['runOperation', [], 6, false],
    ['runAction', [], 7, true],
    ['runOverloadedOperation', [], 8, false],
    ['runOverloadedOperation', [7], 9, false],
    ['runOverloadedAction', [], 10, true],
    ['runOverloadedAction', [7], 11, true],
    ['runCrossOperation', [], 12, false],
    ['runCrossOperation', [7], 6, false],
    ['runCrossAction', [], 13, true],
    ['runCrossAction', [7], 7, true],
  ];
  for (const [name, args, slot, action] of cases) {
    const start = (signal) => probe[name](...args, signal);
    native = nativeOperation();
    const observed = trackedSignal();
    const before = invocations;
    const op = start(observed.signal);
    assert.equal(invocations, before + 1);
    assert.equal(invokedSlot, slot);
    assert.equal(native.registrations, 1, 'creation must start completion exactly once');
    const progress = [];
    const chained = op.progress((value) => progress.push(value));
    assert.equal(chained, op);
    const promise = chained.toPromise();
    assert.equal(promise, op);
    assert.equal(op.toPromise(), promise);
    const detachedToPromise = op.toPromise;
    assert.equal(detachedToPromise(), promise);
    const together = Promise.all([op, promise, op.then((value) => value), op.toPromise()]);
    native.report(9);
    native.complete();
    assert.deepEqual(await together, Array(4).fill(action ? undefined : 42));
    assert.equal(await op, action ? undefined : 42);
    assert.equal(await op.toPromise(), action ? undefined : 42);
    assert.equal(native.registrations, 1);
    assert.equal(native.results, action ? 0 : 1, 'project the final result only once');
    assert.equal(native.progress, 1);
    assert.deepEqual(progress, [9]);
    assert.deepEqual(observed.counts(), [1, 1], `${name}: remove the abort listener once`);
    op.cancel();
    assert.equal(native.cancels, 1);
    assert.equal(await promise, action ? undefined : 42);

    for (const failure of ['native', 'cancel', 'progress-cancel', 'abort']) {
      native = nativeOperation();
      const observed = trackedSignal();
      const reason = new Error(failure);
      const op = start(observed.signal);
      const consumers = Promise.allSettled([op, op.toPromise(), op.toPromise(), op.then((value) => value)]);
      if (failure === 'native') native.fail(reason);
      if (failure === 'cancel') op.cancel();
      if (failure === 'progress-cancel') {
        op.progress(() => op.cancel());
        native.report(1);
      }
      if (failure === 'abort') observed.controller.abort(reason);
      const expected = failure.includes('cancel') ? native.error : reason;
      for (const result of await consumers) {
        assert.equal(result.status, 'rejected');
        assert.equal(result.reason, expected);
      }
      assert.equal(native.registrations, 1);
      assert.equal(native.results, 0);
      assert.equal(native.cancels, failure === 'native' ? 0 : 1);
      assert.deepEqual(observed.counts(), [1, 1]);
    }

    const aborted = trackedSignal();
    const reason = new Error('already aborted');
    aborted.controller.abort(reason);
    const beforeAbort = invocations;
    const opAborted = start(aborted.signal);
    assert.equal(opAborted.toPromise(), opAborted);
    assert.equal(
      opAborted.progress(() => assert.fail('unexpected progress')),
      opAborted,
    );
    opAborted.cancel();
    for (const result of await Promise.allSettled([opAborted, opAborted.toPromise(), opAborted.toPromise()])) {
      assert.equal(result.status, 'rejected');
      assert.equal(result.reason, reason);
    }
    assert.equal(invocations, beforeAbort, 'pre-aborted signals must not invoke native code');
    assert.deepEqual(aborted.counts(), [0, 0]);
  }
}

const timeout = setTimeout(() => {
  throw new Error('progress Promise regression timed out');
}, 10_000);
main()
  .catch((error) => {
    console.error(error);
    process.exitCode = 1;
  })
  .finally(() => clearTimeout(timeout));
