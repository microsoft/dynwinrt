// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict';
import { DynCom } from '../../../bindings/js/dist/com.js';
import { FILEOPERATION_FLAGS } from '../../e2e_generated/com/shell/FILEOPERATION_FLAGS.js';
import { IFileOperation } from '../../e2e_generated/com/shell/IFileOperation.js';

DynCom.initialize(1);

const operation = IFileOperation.create();
const flags =
  FILEOPERATION_FLAGS.FOF_NO_UI +
  FILEOPERATION_FLAGS.FOFX_DONTDISPLAYLOCATIONS;

assert.equal(flags, 2147485204);
operation.setOperationFlags(flags);
assert.equal(operation.getAnyOperationsAborted(), false);
operation._obj.release();

console.log('file-operation ok');
