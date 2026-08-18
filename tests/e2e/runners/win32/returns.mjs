// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from "node:assert/strict";
import {
  getModuleHandleW,
  getProcAddress,
  loadLibraryW,
} from "../../e2e_generated/win32/win32/Windows.Win32.System.LibraryLoader/Apis.js";
import { ldapGetLastError } from "../../e2e_generated/win32/win32/Windows.Win32.Networking.Ldap/Apis.js";
import { getAdaptersAddresses } from "../../e2e_generated/win32/win32/Windows.Win32.NetworkManagement.IpHelper/Apis.js";
import {
  createPOINT,
  createRECT,
  ptInRect,
} from "../../e2e_generated/win32/win32/Windows.Win32.Graphics.Gdi/Apis.js";
import {
  createProcessInformation,
  createStartupInfoW,
  createProcessW,
  getProcessInformationProcessId,
  getProcessInformationThreadId,
  openProcess,
  takeProcessInformationProcess,
  takeProcessInformationThread,
} from "../../e2e_generated/win32/win32/Windows.Win32.System.Threading/Apis.js";
import {
  createSecurityAttributes,
  createPipe,
} from "../../e2e_generated/win32/win32/Windows.Win32.System.Pipes/Apis.js";
import {
  createFileW,
  readFileAsync,
  writeFileAsync,
} from "../../e2e_generated/win32/win32/Windows.Win32.Storage.FileSystem/Apis.js";
import { coIsHandlerConnected } from "../../e2e_generated/win32/win32/Windows.Win32.System.Com/Apis.js";
import {
  DynWinRtValue,
  roInitialize,
} from "../../../../bindings/js/dist/winrt.js";
import { rm } from "node:fs/promises";
import { pbkdf2 } from "node:crypto";
import { once } from "node:events";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import {
  createSYSTEMTIME,
  getSystemTime,
  getTickCount64,
} from "../../e2e_generated/win32/win32/Windows.Win32.System.SystemInformation/Apis.js";

async function withTimeout(promise, timeoutMs, message, onTimeout = () => {}) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => {
      try {
        onTimeout();
      } finally {
        reject(new Error(message));
      }
    }, timeoutMs);
  });
  try {
    return await Promise.race([promise, timeout]);
  } finally {
    clearTimeout(timer);
  }
}

async function withAbortTimeout(start, timeoutMs, label) {
  const controller = new AbortController();
  let timedOut = false;
  const timer = setTimeout(() => {
    timedOut = true;
    controller.abort();
  }, timeoutMs);
  try {
    return await start(controller.signal);
  } catch (error) {
    if (timedOut && error.name === "AbortError") {
      throw new Error(`${label} timed out`, { cause: error });
    }
    throw error;
  } finally {
    clearTimeout(timer);
  }
}

const first = getTickCount64();
await new Promise((resolve) => setTimeout(resolve, 10));
const second = getTickCount64();
assert.equal(typeof first, "bigint");
assert(second >= first);

const missing = getModuleHandleW("dynwinrt-module-that-is-not-loaded.dll");
assert.equal(missing.result, 0n);
assert.equal(missing.lastError, 126);

const kernel = getModuleHandleW("kernel32.dll").result;
assert.notEqual(kernel, 0n);
const proc = getProcAddress(kernel, Buffer.from("GetTickCount64\0", "ascii"));
assert.equal(typeof proc.result, "bigint");
assert.notEqual(proc.result, 0n);

const loadedKernelCall = loadLibraryW("kernel32.dll");
const loadedKernel = loadedKernelCall.result;
assert(loadedKernel);
assert.equal(loadedKernel.closed, false);
assert.notEqual(
  getProcAddress(loadedKernel, Buffer.from("GetTickCount64\0", "ascii")).result,
  0n,
);
loadedKernel.close();
assert.equal(loadedKernel.closed, true);

assert.equal(typeof ldapGetLastError(), "number");
console.log("[win32-e2e] scalar, module, and cdecl calls passed");

let adapters = getAdaptersAddresses(0, 0, null);
assert.equal(adapters.result, 111);
assert(adapters.sizePointer > 0);
let adapterBuffer = Buffer.alloc(adapters.sizePointer);
adapters = getAdaptersAddresses(0, 0, adapterBuffer);
if (adapters.result === 111) {
  adapterBuffer = Buffer.alloc(adapters.sizePointer);
  adapters = getAdaptersAddresses(0, 0, adapterBuffer);
}
assert.equal(adapters.result, 0);
console.log("[win32-e2e] adapter buffer query passed");

const systemTime = createSYSTEMTIME();
getSystemTime(systemTime);
const systemTimeBytes = systemTime.bytes;
const year = systemTimeBytes.readUInt16LE(0);
const month = systemTimeBytes.readUInt16LE(2);
assert(year >= 2020);
assert(month >= 1 && month <= 12);
console.log("[win32-e2e] system time aggregate passed");

const rectBytes = Buffer.alloc(16);
rectBytes.writeInt32LE(0, 0);
rectBytes.writeInt32LE(0, 4);
rectBytes.writeInt32LE(10, 8);
rectBytes.writeInt32LE(10, 12);
const pointBytes = Buffer.alloc(8);
pointBytes.writeInt32LE(5, 0);
pointBytes.writeInt32LE(5, 4);
assert.equal(ptInRect(createRECT(rectBytes), createPOINT(pointBytes)), true);
console.log("[win32-e2e] by-value POINT call passed");

const processHandle = openProcess(0x1000, false, process.pid);
assert(processHandle.result);
processHandle.result.close();
assert(processHandle.result.closed);
console.log("[win32-e2e] OpenProcess ownership passed");

const attributes = createSecurityAttributes({
  securityDescriptor: null,
  inheritHandle: false,
});
console.log("[win32-e2e] SECURITY_ATTRIBUTES builder passed");
const pipe = createPipe(attributes, 0);
console.log("[win32-e2e] CreatePipe call returned");
assert.equal(pipe.result, true);
assert(pipe.hReadPipe);
assert(pipe.hWritePipe);
pipe.hReadPipe.close();
pipe.hWritePipe.close();
console.log("[win32-e2e] aggregate and resource calls passed");

const startupInfo = createStartupInfoW();
const processInformation = createProcessInformation();
const command = `"${process.env.ComSpec}" /d /c exit 0`;
const created = createProcessW(
  null,
  command,
  null,
  null,
  false,
  0x08000000,
  null,
  startupInfo,
  processInformation,
);
assert.equal(
  created.result,
  true,
  `CreateProcessW failed: ${created.lastError}`,
);
assert(getProcessInformationProcessId(processInformation) > 0);
assert(getProcessInformationThreadId(processInformation) > 0);
const childProcess = takeProcessInformationProcess(processInformation);
const childThread = takeProcessInformationThread(processInformation);
assert(childProcess);
assert(childThread);
childProcess.close();
childThread.close();
assert.equal(takeProcessInformationProcess(processInformation), null);
assert.equal(takeProcessInformationThread(processInformation), null);

const failedProcessInformation = createProcessInformation();
const failedProcess = createProcessW(
  null,
  "dynwinrt-definitely-missing-executable.exe",
  null,
  null,
  false,
  0x08000000,
  null,
  startupInfo,
  failedProcessInformation,
);
assert.equal(failedProcess.result, false);
assert.throws(
  () => getProcessInformationProcessId(failedProcessInformation),
  /native call failed/,
);
assert.throws(
  () => takeProcessInformationProcess(failedProcessInformation),
  /native call failed/,
);
console.log("[win32-e2e] process creation and output ownership passed");

const asyncPath = join(
  tmpdir(),
  `dynwinrt-overlapped-${process.pid}-${Date.now()}.tmp`,
);
const asyncFileCall = createFileW(
  asyncPath,
  0xc0000000,
  0,
  null,
  2,
  0x40000080,
  null,
);
assert(asyncFileCall.result, `CreateFileW failed: ${asyncFileCall.lastError}`);
const asyncFile = asyncFileCall.result;
console.log("[win32-e2e] starting file OVERLAPPED I/O");
try {
  const payload = Buffer.from("dynwinrt-overlapped");
  const aborted = new AbortController();
  aborted.abort();
  await assert.rejects(
    readFileAsync(asyncFile, Buffer.alloc(1), 0n, aborted.signal),
    (error) => error.name === "AbortError",
  );
  let invalidSignal;
  assert.doesNotThrow(() => {
    invalidSignal = readFileAsync(asyncFile, Buffer.alloc(1), 0n, {});
  });
  await assert.rejects(invalidSignal, /signal must be an AbortSignal/);
  assert.equal(asyncFile.busy, false);
  assert.equal(
    await withAbortTimeout(
      (signal) => writeFileAsync(asyncFile, payload, 0n, signal),
      10000,
      "file WriteFile",
    ),
    payload.length,
  );
  const received = Buffer.alloc(payload.length);
  assert.equal(
    await withAbortTimeout(
      (signal) => readFileAsync(asyncFile, received, 0n, signal),
      10000,
      "file ReadFile",
    ),
    payload.length,
  );
  assert.deepEqual(received, payload);
  assert.equal(
    await withAbortTimeout(
      (signal) =>
        readFileAsync(
          asyncFile,
          Buffer.alloc(1),
          BigInt(payload.length),
          signal,
        ),
      10000,
      "file EOF ReadFile",
    ),
    0,
  );
} finally {
  asyncFile.close();
  await rm(asyncPath, { force: true });
}
console.log("[win32-e2e] file OVERLAPPED I/O passed");

const pipePath = `\\\\.\\pipe\\dynwinrt-overlapped-cancel-${process.pid}-${Date.now()}`;
const pipeServer = createServer();
pipeServer.listen(pipePath);
await withTimeout(
  once(pipeServer, "listening"),
  10000,
  "named-pipe server did not start",
  () => pipeServer.close(),
);
const connected = once(pipeServer, "connection");
const pipeClientCall = createFileW(
  pipePath,
  0x80000000,
  0,
  null,
  3,
  0x40000000,
  null,
);
assert(
  pipeClientCall.result,
  `CreateFileW named pipe failed: ${pipeClientCall.lastError}`,
);
const pipeClient = pipeClientCall.result;
const [pipeServerSocket] = await withTimeout(
  connected,
  10000,
  "named-pipe client did not connect",
  () => pipeServer.close(),
);
try {
  const controller = new AbortController();
  const pendingRead = readFileAsync(
    pipeClient,
    Buffer.alloc(1),
    0n,
    controller.signal,
  );
  assert.equal(pipeClient.busy, true);
  assert.throws(() => pipeClient.close(), /asynchronous I\/O is pending/);
  const activeDeadline = Date.now() + 5000;
  while (!pipeClient.active && Date.now() < activeDeadline) {
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  if (!pipeClient.active) {
    controller.abort();
    await pendingRead.catch(() => {});
    assert.fail("named-pipe ReadFile did not enter ERROR_IO_PENDING");
  }
  controller.abort();
  await assert.rejects(
    pendingRead,
    (error) =>
      error.name === "AbortError" &&
      /Win32 error 995/i.test(error.cause?.message ?? ""),
  );
  assert.equal(pipeClient.active, false);
  assert.equal(pipeClient.busy, false);
  console.log("[win32-e2e] pending cancellation passed");

  const poolController = new AbortController();
  const concurrentReads = Array.from({ length: 16 }, () =>
    readFileAsync(pipeClient, Buffer.alloc(1), 0n, poolController.signal),
  );
  const poolDeadline = Date.now() + 5000;
  while (!pipeClient.active && Date.now() < poolDeadline) {
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  assert.equal(
    pipeClient.active,
    true,
    "concurrent reads did not enter pending I/O",
  );
  const queueLimitError = await withTimeout(
    Promise.any(
      concurrentReads.map((read) =>
        read.then(
          () =>
            Promise.reject(new Error("pending read completed unexpectedly")),
          (error) => {
            if (/waiter capacity is full/i.test(error.message)) {
              return error;
            }
            throw error;
          },
        ),
      ),
    ),
    10000,
    "bounded OVERLAPPED waiter did not reject excess work",
    () => poolController.abort(),
  );
  assert.match(queueLimitError.message, /waiter capacity is full/i);
  await Promise.race([
    promisify(pbkdf2)("dynwinrt", "win32", 1, 16, "sha256"),
    new Promise((_, reject) =>
      setTimeout(
        () =>
          reject(new Error("OVERLAPPED reads exhausted the libuv worker pool")),
        2000,
      ),
    ),
  ]);
  poolController.abort();
  const cancelledReads = await withTimeout(
    Promise.allSettled(concurrentReads),
    10000,
    "bounded OVERLAPPED reads did not settle after cancellation",
    () => pipeServerSocket.destroy(),
  );
  assert(
    cancelledReads.every(
      (result) =>
        result.status === "rejected" &&
        (result.reason.name === "AbortError" ||
          /waiter capacity is full/i.test(result.reason.message)),
    ),
  );
  assert.equal(pipeClient.busy, false);
  console.log("[win32-e2e] bounded waiter and libuv availability passed");

  const detachable = new ArrayBuffer(1);
  const detachedBuffer = Buffer.from(detachable);
  const detachedController = new AbortController();
  const detachedRead = readFileAsync(
    pipeClient,
    detachedBuffer,
    0n,
    detachedController.signal,
  );
  const detachedDeadline = Date.now() + 5000;
  while (!pipeClient.active && Date.now() < detachedDeadline) {
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  assert.equal(
    pipeClient.active,
    true,
    "detachment read did not enter pending I/O",
  );
  structuredClone(detachable, { transfer: [detachable] });
  assert.equal(detachedBuffer.length, 0);
  pipeServerSocket.write(Buffer.from([42]));
  await withTimeout(
    assert.rejects(detachedRead, /backing ArrayBuffer was detached or changed/),
    10000,
    "detached read did not settle",
    () => detachedController.abort(),
  );
  assert.equal(pipeClient.busy, false);
  console.log("[win32-e2e] detached read rejection passed");
} finally {
  if (!pipeClient.busy) {
    pipeClient.close();
  }
  pipeServerSocket.destroy();
  await new Promise((resolve) => pipeServer.close(resolve));
}
let closedRead;
assert.doesNotThrow(() => {
  closedRead = readFileAsync(pipeClient, Buffer.alloc(1), 0n);
});
await assert.rejects(closedRead, /closed Win32 resource/);

roInitialize(1);
const activationFactory = DynWinRtValue.activationFactory(
  "Windows.Foundation.Uri",
);
assert.equal(typeof coIsHandlerConnected(activationFactory), "boolean");
activationFactory.release();
console.log("[win32-e2e] WinRT compatibility assertion passed");

console.log("PASS");
