// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const assert = require('node:assert/strict');
const { DynCom } = require('../../../../bindings/js/dist/com-unsafe.js');
const barrel = require('../../e2e_generated/com/shell/com/index.js');
const deep = require('../../e2e_generated/com/shell/com/windows/win32/ui/shell/TaskbarList.js');

assert.equal(typeof barrel.TaskbarList, 'function');
assert.equal(typeof barrel.ITaskbarList3, 'function');
assert.strictEqual(barrel.TaskbarList, deep.TaskbarList);

DynCom.initialize(1);
const taskbar = new barrel.TaskbarList();
taskbar.hrInit();
assert.equal(taskbar.supports(barrel.ITaskbarList3), true);
taskbar.release();

console.log('module-commonjs ok');
