// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict';
import { DynCom } from '../../../../bindings/js/dist/com-unsafe.js';
import { FileOpenDialog } from '../../e2e_generated/com/shell/com/FileOpenDialog.js';

DynCom.initialize(0);

const dialog = new FileOpenDialog();
const options = dialog.getOptions();
dialog.setOptions(options);
assert.equal(dialog.getOptions(), options);
dialog.release();

console.log('file-open-dialog ok');
