# Using generated flat Win32 bindings

Generate a flat `Apis` container from the restored Win32 metadata package:

```powershell
dynwinrt-codegen generate `
  --winmd C:\path\to\Windows.Win32.winmd `
  --namespace Windows.Win32.System.Registry `
  --output .\generated
```

The namespace is emitted under its own domain:

```text
generated/
  package.json
  win32/
    Windows.Win32.System.Registry/
      Apis.js
      Apis.d.ts
      index.js
      index.d.ts
```

Import the generated namespace subpath:

```js
import {
  regOpenKeyEx,
  regQueryValueEx,
} from "@winapp/bindings/win32/Windows.Win32.System.Registry";

const HKEY_LOCAL_MACHINE = 0x80000002n;
const KEY_READ = 0x20019;

const opened = regOpenKeyEx(
  HKEY_LOCAL_MACHINE,
  "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
  0,
  KEY_READ,
);

if (opened.status !== 0 || !opened.key) {
  throw new Error(`RegOpenKeyEx failed: ${opened.status}`);
}

try {
  const probe = regQueryValueEx(opened.key, "ProductName", null);
  const data = Buffer.alloc(probe.dataSize);
  const result = regQueryValueEx(opened.key, "ProductName", data);
  if (result.status !== 0) {
    throw new Error(`RegQueryValueEx failed: ${result.status}`);
  }
} finally {
  opened.key.close();
}
```

Generated safe wrappers do not accept numeric data addresses. Use
`Buffer`/`Uint8Array` for native storage and `bigint` only for confirmed handle
values. Manual ABI declarations and arbitrary native addresses require the
explicit `@microsoft/dynwinrt/win32/unsafe` entrypoint.

APIs that consume and close a handle require `DynWin32Resource`; passing
`resource.value` or another numeric handle is rejected. Double-NUL string-list
parameters accept `string[]` or explicitly encoded, double-terminated storage.

Validated native structs receive generated factories and branded storage:

```js
const systemTime = createSYSTEMTIME();
getSystemTime(systemTime);
const year = systemTime.bytes.readUInt16LE(0);
```

By-value structs use the same branded value. Typed buffers enforce metadata
size/count relationships and native alignment before dispatch.

Byte-counted APIs expose a nullable caller buffer and return the updated size,
so standard Win32 two-call queries remain explicit:

```js
let query = getAdaptersAddresses(0, 0, null);
const data = Buffer.alloc(query.sizePointer);
query = getAdaptersAddresses(0, 0, data);
if (query.result !== 0) {
  throw new Error(`GetAdaptersAddresses failed: ${query.result}`);
}
```

The buffer remains opaque when its native records contain internal pointers.
This preserves memory safety without inventing a JavaScript object model for
unvalidated pointees.

Exact pointer-bearing structs use generated builders rather than raw bytes:

```js
const attributes = createSecurityAttributes({
  securityDescriptor: null,
  inheritHandle: false,
});
const pipe = createPipe(attributes, 0);
pipe.hReadPipe?.close();
pipe.hWritePipe?.close();
```

`PROCESS_INFORMATION` outputs are success-gated and adopted explicitly:

```js
const startup = createStartupInfoW();
const processInfo = createProcessInformation();
const created = createProcessW(
  null, commandLine, null, null, false, 0, null, startup, processInfo,
);
if (created.result) {
  const process = takeProcessInformationProcess(processInfo);
  const thread = takeProcessInformationThread(processInfo);
  process?.close();
  thread?.close();
}
```

Generated `ReadFile`/`WriteFile` projections use dedicated OVERLAPPED Promises:

```js
const written = await writeFileAsync(file, data, 0n, abortController.signal);
const read = await readFileAsync(file, destination, 0n);
```

The runtime holds the file resource and private native storage until completion
or cancellation, waits on a fixed-capacity native waiter outside the libuv
worker pool, and revalidates read Buffers before copying data back. More than
eight concurrent operations are rejected explicitly. The file must have been
opened with `FILE_FLAG_OVERLAPPED`.

Runnable examples are available under
[`samples/js/win32`](../../../samples/js/win32/README.md). They cover direct
64-bit returns, branded native structs, caller-owned Registry buffers, and
deterministic handle cleanup without using the unsafe entrypoint.
