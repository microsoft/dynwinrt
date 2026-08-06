// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict';
import { DynCom } from '../../../bindings/js/dist/com-unsafe.js';
import { FILEOPERATION_FLAGS } from '../../e2e_generated/com/shell/com/FILEOPERATION_FLAGS.js';
import { FileOperation } from '../../e2e_generated/com/shell/com/FileOperation.js';

DynCom.initialize(1);

const operation = new FileOperation();
const flags =
  FILEOPERATION_FLAGS.FOF_NO_UI +
  FILEOPERATION_FLAGS.FOFX_DONTDISPLAYLOCATIONS;

assert.equal(flags, 2147485204);
operation.setOperationFlags(flags);
assert.equal(operation.getAnyOperationsAborted(), false);
operation.release();

console.log('file-operation ok');
