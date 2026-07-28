// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict';
import { DynCom } from '../../../bindings/js/dist/com.js';
import { IFileOpenDialog } from '../../e2e_generated/com/shell/IFileOpenDialog.js';

DynCom.initialize(0);

const dialog = IFileOpenDialog.create();
const options = dialog.getOptions();
dialog.setOptions(options);
assert.equal(dialog.getOptions(), options);
dialog._obj.release();

console.log('file-open-dialog ok');
