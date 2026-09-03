// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { DynCom } from "../../../../bindings/js/dist/com-unsafe.js";
import {
  createPOINTL,
  IDropTarget,
} from "../../e2e_generated/com/shell/com/windows/win32/system/ole/IDropTarget.js";
import { IFileDialogEvents } from "../../e2e_generated/com/shell/com/windows/win32/ui/shell/IFileDialogEvents.js";

DynCom.initialize(0);

const calls = [];
const handlers = {
  dragEnter(dataObject, keyState, point, effect) {
    assert.equal(this, handlers);
    assert.equal(typeof dataObject.isNull, "function");
    assert.equal(typeof keyState, "number");
    assert.equal(point.bytes.length, 8);
    return effect;
  },
  dragOver(keyState, point, effect) {
    assert.equal(keyState, 8);
    assert.equal(point.bytes.readInt32LE(0), 10);
    assert.equal(point.bytes.readInt32LE(4), 20);
    assert.equal(effect, 2);
    calls.push("over");
    return 3;
  },
  dragLeave() {
    calls.push("leave");
  },
  drop(dataObject, keyState, point, effect) {
    assert.equal(typeof dataObject.isNull, "function");
    assert.equal(typeof keyState, "number");
    assert.equal(point.bytes.length, 8);
    return effect;
  },
};

const events = IFileDialogEvents.implementation({
  onFileOk() {},
  onFolderChanging() {},
  onFolderChange() {},
  onSelectionChange() {},
  onShareViolation() {
    return 0;
  },
  onTypeChange() {},
  onOverwrite() {
    return 0;
  },
});
const target = IDropTarget.implement(handlers, events);
const eventsView = target.as(IFileDialogEvents);
try {
  const bytes = Buffer.alloc(8);
  bytes.writeInt32LE(10, 0);
  bytes.writeInt32LE(20, 4);
  const point = createPOINTL(bytes);

  assert.equal(target.dragOver(8, point, 2), 3);
  target.dragLeave();
  assert.deepEqual(calls, ["over", "leave"]);
} finally {
  eventsView.release();
  target.release();
}

console.log("drop-target multi-interface dynamic sink ok");
