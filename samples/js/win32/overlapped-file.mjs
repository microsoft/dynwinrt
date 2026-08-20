// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from "node:assert/strict";
import { rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  createFileW,
  readFileAsync,
  writeFileAsync,
} from "./generated/win32/Windows.Win32.Storage.FileSystem/index.mjs";

const GENERIC_READ_WRITE = 0xc0000000;
const CREATE_ALWAYS = 2;
const FILE_ATTRIBUTE_NORMAL = 0x80;
const FILE_FLAG_OVERLAPPED = 0x40000000;

const path = join(tmpdir(), `dynwinrt-overlapped-${process.pid}-${Date.now()}.tmp`);
const opened = createFileW(
  path,
  GENERIC_READ_WRITE,
  0,
  null,
  CREATE_ALWAYS,
  FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
  null,
);
if (!opened.result) {
  throw new Error(`CreateFileW failed with error ${opened.lastError}`);
}

const file = opened.result;
try {
  const controller = new AbortController();
  const expected = Buffer.from("Hello from OVERLAPPED Win32 I/O");

  const written = await writeFileAsync(file, expected, 0n, controller.signal);
  assert.equal(written, expected.length);

  const actual = Buffer.alloc(expected.length);
  const read = await readFileAsync(file, actual, 0n, controller.signal);
  assert.equal(read, expected.length);
  assert.deepEqual(actual, expected);

  console.log(actual.toString("utf8"));
} finally {
  file.close();
  await rm(path, { force: true });
}
