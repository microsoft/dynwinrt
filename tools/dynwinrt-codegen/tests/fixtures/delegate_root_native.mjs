// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from "node:assert/strict";
import { createRequire } from "node:module";
import path from "node:path";
import { pathToFileURL } from "node:url";

const require = createRequire(import.meta.url);
const [runtimePath, generatedPath, moduleKind, expectedFailure] =
  process.argv.slice(2);
const runtime = require(runtimePath);
runtime.roInitialize(1);

const generated =
  moduleKind === "esm"
    ? await import(pathToFileURL(path.join(generatedPath, "index.mjs")).href)
    : require(generatedPath);
const scope = generated.createProjectedLifetimeScope();
let callbacks = 0;
let callbackError;
let failure;
const timeout = setTimeout(() => {
  console.error("Timed out waiting for the generated ThreadPool callback");
  process.exit(2);
}, 10_000);

try {
  await generated.ThreadPool.runAsync((operation) => {
    try {
      assert.ok(operation instanceof runtime.DynWinRtValue);
      assert.equal(operation.isNull(), false);
      callbacks += 1;
    } catch (error) {
      callbackError = error;
    } finally {
      operation.release();
    }
  });
  assert.ifError(callbackError);
  assert.equal(callbacks, 1);
} catch (error) {
  failure = error;
} finally {
  clearTimeout(timeout);
  scope.dispose();
}

// Generated callback wrappers are GC-owned; release them before the child exits.
await new Promise((resolve) => setImmediate(resolve));
global.gc();
await new Promise((resolve) => setImmediate(resolve));

if (expectedFailure === "expect-selection-failure") {
  assert.equal(callbacks, 0);
  assert.match(String(failure), /Failed to recover `?WinGUID`?/);
} else {
  assert.ifError(failure);
}
console.log(
  JSON.stringify({
    language: "js",
    arch: process.arch,
    module: moduleKind,
    callbacks,
    error: failure ? String(failure) : null,
  }),
);
