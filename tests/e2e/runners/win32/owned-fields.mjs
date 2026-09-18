// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { setImmediate as nextTurn } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import {
  createProcessInformation,
  createStartupInfoW,
  createProcessW,
  takeProcessInformationProcess,
  takeProcessInformationThread,
} from "../../e2e_generated/win32/win32/windows/win32/system/threading/Apis.js";
import {
  DynWin32,
  DynWin32CallResult,
  DynWin32Function,
} from "../../../../bindings/js/dist/win32-unsafe.js";

const stage = process.argv[2];
if (!stage) {
  for (const failure of ["control", "invoke", "returnValue", "outputs"]) {
    const result = spawnSync(
      process.execPath,
      ["--expose-gc", fileURLToPath(import.meta.url), failure],
      { encoding: "utf8", timeout: 30000 },
    );
    assert.equal(result.error, undefined, `${failure}: ${result.error}\n${result.stderr}`);
    assert.equal(result.status, 0, `${failure}:\n${result.stdout}\n${result.stderr}`);
    assert.match(result.stdout, /PASS owned fields/);
  }
  console.log("PASS owned fields: actual CreateProcessW delivery failures leave no handles");
} else {
  assert.equal(typeof global.gc, "function");
  const counter = DynWin32Function.bind({
    dll: "kernel32.dll",
    entryPoint: "GetProcessHandleCount",
    parameters: [
      { type: "handle", direction: "in" },
      { type: "u32", direction: "out" },
    ],
    returnType: "bool32",
    successRule: "nonzero",
  });
  const countHandles = () => {
    const count = counter.invoke([DynWin32.handle(-1n)]);
    assert(count.succeeded);
    return DynWin32.toNumber(count.outputs[0]);
  };
  const collect = async () => {
    for (let i = 0; i < 8; ++i) {
      await nextTurn();
      global.gc();
    }
    await nextTurn();
  };

  function run(failure) {
    const startup = createStartupInfoW();
    const information = createProcessInformation();
    const reference = new WeakRef(information);
    const prototype = failure === "invoke"
      ? DynWin32Function.prototype
      : DynWin32CallResult.prototype;
    const member = failure === "invoke" ? "invoke" : failure;
    const descriptor = failure === "control"
      ? undefined
      : Object.getOwnPropertyDescriptor(prototype, member);
    if (descriptor) {
      Object.defineProperty(prototype, member, failure === "invoke"
        ? {
            ...descriptor,
            value(...args) {
              Reflect.apply(descriptor.value, this, args);
              throw new Error("injected delivery failure");
            },
          }
        : {
            ...descriptor,
            get() { throw new Error("injected delivery failure"); },
          });
    }
    try {
      const invoke = () => createProcessW(
        null,
        `"${process.env.ComSpec}" /d /c exit 0`,
        null, null, false, 0x08000000, null,
        startup, information,
      );
      if (failure === "control") {
        assert.equal(invoke().result, true);
        takeProcessInformationProcess(information).close();
        takeProcessInformationThread(information).close();
      } else {
        assert.throws(invoke, /injected delivery failure/);
      }
    } finally {
      if (descriptor) Object.defineProperty(prototype, member, descriptor);
    }
    return reference;
  }

  run("control");
  await collect();
  const before = countHandles();
  const references = Array.from({ length: 4 }, () => run(stage));
  await collect();
  for (const reference of references) assert.equal(reference.deref(), undefined);
  const after = countHandles();
  assert.equal(after, before, `${stage}: leaked ${after - before} process/thread handles`);
  console.log(`PASS owned fields ${stage}: 0 leaked handles`);
}
