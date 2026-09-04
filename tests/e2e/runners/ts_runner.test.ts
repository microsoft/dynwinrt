// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { strict as assert } from "node:assert";
import { test } from "node:test";

import { instantiateClass } from "./ts_runner.ts";

test("instantiateClass invokes constructors", () => {
  class Example {
    constructor(readonly value: string) {}
  }

  const result = instantiateClass(
    "Example",
    { kind: "constructor", args: ["value"] },
    Example,
  );

  assert.ok(result instanceof Example);
  assert.equal(result.value, "value");
});

test("instantiateClass rejects unknown kinds", () => {
  const invalidInstantiate = JSON.parse('{"kind":"unsupported"}');

  assert.throws(
    () => instantiateClass("Example", invalidInstantiate, class Example {}),
    /Unknown instantiate kind: unsupported/,
  );
});
