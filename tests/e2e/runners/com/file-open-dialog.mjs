// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict';
import { DynCom } from '../../../../bindings/js/dist/com-unsafe.js';
import { FDE_OVERWRITE_RESPONSE } from '../../e2e_generated/com/shell/com/FDE_OVERWRITE_RESPONSE.js';
import { FDE_SHAREVIOLATION_RESPONSE } from '../../e2e_generated/com/shell/com/FDE_SHAREVIOLATION_RESPONSE.js';
import { FileOpenDialog } from '../../e2e_generated/com/shell/com/FileOpenDialog.js';
import { IFileDialogEvents } from '../../e2e_generated/com/shell/com/IFileDialogEvents.js';

DynCom.initialize(0);

const dialog = new FileOpenDialog();
const calls = [];
assert.throws(
  () => IFileDialogEvents.implement({}),
  /onFileOk must be a function/,
);
const eventsImplementation = {
  onFileOk(value) {
    assert.equal(this, eventsImplementation);
    assert.equal(value.isNull(), false);
    calls.push('fileOk');
    return 1;
  },
  onFolderChanging() {},
  onFolderChange() {},
  onSelectionChange() {},
  onShareViolation() {
    return FDE_SHAREVIOLATION_RESPONSE.FDESVR_DEFAULT;
  },
  onTypeChange() {},
  onOverwrite() {
    return FDE_OVERWRITE_RESPONSE.FDEOR_DEFAULT;
  },
};
const events = IFileDialogEvents.implement(eventsImplementation);

try {
  const options = dialog.getOptions();
  dialog.setOptions(options);
  assert.equal(dialog.getOptions(), options);

  events.onFileOk(dialog._obj);
  assert.deepEqual(calls, ['fileOk']);
  events.onSelectionChange(dialog._obj);

  const cookie = dialog.advise(events.nativeValue);
  dialog.unadvise(cookie);
} finally {
  events.release();
  dialog.release();
}

console.log('file-open-dialog and events sink ok');
