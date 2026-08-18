# Flat Win32 support

Flat Win32 APIs are DLL exports described by `[DllImport]` methods in
`Windows.Win32.winmd`. They are a third frontend, separate from WinRT
activation and Classic COM vtables.

```text
Windows.Win32.winmd facts
  -> flat-local semantic ABI contracts
  -> validated projected function IR
  -> immutable Win32CallPlan
  -> shared behavior-neutral libffi executor
  -> System32 export
```

The npm surfaces remain isolated:

- `@microsoft/dynwinrt` is WinRT-only;
- `@microsoft/dynwinrt/com` is managed Classic COM support;
- `@microsoft/dynwinrt/com/unsafe` is raw Classic COM ABI access;
- `@microsoft/dynwinrt/win32` is the safe flat Win32 runtime; and
- `@microsoft/dynwinrt/win32/unsafe` permits numeric native addresses.

## Runtime architecture

`Win32CallPlan` fixes the DLL, export, ordered parameter ABI, parameter
direction, nullability, return ABI, success rule, LastError behavior, and
resource cleanup before the first invocation. Its libffi CIF is prepared once
and then treated as immutable.

The executor never derives an ABI type from a JavaScript value. Invocation
first validates every value against the plan, creates stable output slots, calls
the resolved export, captures LastError immediately when requested, and adopts
owned resources only when the function's success rule succeeds.

Modules are loaded only from System32 with `LOAD_LIBRARY_SEARCH_SYSTEM32` and
remain loaded for the process lifetime. Bare `.dll` and `.drv` names are
accepted; paths are rejected. `mapi32.dll` is excluded from the safe projection
until MAPI/MAPI utility initialization and shutdown are modeled explicitly.

The runtime supports x64 and ARM64 with explicit `system` and `cdecl` plans.
A 32-bit build compiles, but plan binding fails explicitly until generation
also carries target-specific availability and all x86 convention variants.

`ReadFile` and `WriteFile` use a separate OVERLAPPED Promise path rather than
the synchronous call plan. It owns an event and private native buffer, leases a
managed file handle for the operation, handles immediate and
`ERROR_IO_PENDING` completion, supports `AbortSignal`/`CancelIoEx`, and copies
read results back on the JavaScript thread. Completion waits run on a shared,
fixed eight-thread native waiter rather than the libuv worker pool; excess
operations are rejected explicitly instead of allocating unbounded OS threads.
Before copying a read result, the JS thread reacquires and revalidates the Node
Buffer backing store so a transferred/detached ArrayBuffer cannot leave a stale
destination pointer. EOF resolves with zero bytes.

## Metadata and codegen layers

```text
win32_metadata.rs
  -> codegen/win32/model.rs
  -> codegen/win32/project.rs
  -> codegen/win32/render.rs
```

Raw metadata retains:

- native name and scalar/enum/handle/pointer category;
- explicit pointer depth and constness;
- `In`, `Out`, and `InOut`;
- nullability;
- reserved-zero and single-/double-NUL termination attributes;
- element-count and byte-count relationships;
- sequential/union layout, packing, fixed arrays, forced alignment, and nested fields;
- architecture and calling convention;
- status-return and LastError metadata; and
- exact handle cleanup metadata.

The semantic model is closed. Unknown layouts, pointer ownership, callback
thunks, writable buffers without size relationships, unsupported cleanup, and
pointer returns without lifetime evidence are omitted with a diagnostic.
Renderers consume projected IR only and have no pointer or Buffer fallback.

Validated native aggregate pointers use aligned, branded call storage. Plain
structs without nested unions may also pass and return by value through libffi.
Typed scalar/GUID pointers and fixed-layout POD buffers use call-local or
caller-owned storage with exact size and alignment checks. COM interface inputs
require an exact IID QueryInterface and remain borrowed for the synchronous
call.
Byte-counted buffers with exactly one data indirection may remain opaque even
when their element is a variable native record. This supports safe two-call
size queries such as `GetAdaptersAddresses` without pretending JavaScript can
interpret the record's internal pointers. Exact adapters supply missing
character-buffer relationships for `LCMapStringA`, `FoldString*`,
`GetLocaleInfo*`, and `QueryFullProcessImageName*`. The wide `LCMapString`
forms remain closed because sort-key flags change the count unit from UTF-16
characters to bytes.

The pinned metadata reader loses parent row identity for some anonymous nested
types. Those layouts are never matched globally by simple name. Exact adapters
currently restore the `SYSTEM_INFO` and `INPUT` anonymous unions; the former
still fails closed because its outer structure contains pointers, while the
latter enables safe caller-owned `SendInput` buffers.
Pointer-bearing aggregates remain closed by default. Exact support currently
includes:

- `SECURITY_ATTRIBUTES`: an input-only builder retains its optional security
  descriptor Buffer for the synchronous call;
- `STARTUPINFOA/W`: zero-initialized builders set the required `cb` field and
  keep optional pointer/handle fields null; and
- `PROCESS_INFORMATION`: successful calls expose PID/TID getters and
  success-gated `CloseHandle` resource adoption for process/thread handles.

Pointer owners are tracked per field, replaced atomically, and revalidated
under the aggregate call lock. Raw bytes are unavailable for pointer-bearing
storage. Unclaimed owned output fields are closed on aggregate drop or before
reuse. Other pointer-bearing structs remain unsupported until every pointee
has an exact retained/borrowed/owned projection. Non-default packed or
forced-aligned aggregates are not passed by value.

## Safe JavaScript projection

Confirmed handle values accept `bigint`, safe-integer `number`, or a managed
`DynWin32Resource`. Dereferenced data addresses accept retained
`Buffer`/`Uint8Array` storage only. UTF-16 strings accept JavaScript strings or
validated terminated storage; ANSI strings require ASCII or caller-encoded
terminated bytes. Consuming handle APIs accept only a managed resource with the
exact cleanup kind; raw handle values cannot bypass close-state or asynchronous
lease checks. Lease state is checked while holding the same resource mutex used
for lease creation and consuming calls. Double-NUL string-list inputs accept
string arrays or validated encoded storage.

Native aggregate descriptors and layouts have explicit size limits and use
fallible allocation paths. Reserved inputs are hidden and supplied as exact
zero/null ABI values.

Scalar returns are direct:

```js
const ticks = getTickCount64(); // bigint
```

Win32 status APIs retain status objects because codes such as
`ERROR_MORE_DATA` are normal control flow:

```js
const opened = regOpenKeyEx(HKEY_LOCAL_MACHINE, path, 0, KEY_READ);
if (opened.status !== 0) {
  // handle status
}
opened.key?.close();
```

Unicode `W` exports also receive an unsuffixed alias when it cannot collide
with another generated name. ANSI `A` exports remain explicit.

## Ownership

Owned outputs are represented by `DynWin32Resource`. Explicit `close()` is
idempotent, and dropping the wrapper invokes the exact cleanup:

- `HKEY` -> `RegCloseKey`;
- `HANDLE` -> `CloseHandle`;
- `HLOCAL` -> `LocalFree`;
- `HGLOBAL` -> `GlobalFree`;
- owned `HMODULE` -> `FreeLibrary`;
- `SC_HANDLE` -> `CloseServiceHandle`;
- task allocator pointers -> `CoTaskMemFree`; and
- credential allocations -> `CredFree`.

A typedef's cleanup attribute does not by itself prove that a direct function
return transfers ownership. Direct handle returns remain borrowed unless a
per-function ownership and success-sentinel contract is registered. Current
exact direct-return evidence includes common kernel handles, `LocalAlloc`,
`GlobalAlloc`, `LoadLibrary*`, and service-control-manager handles.

## Reproducibility

Generated files are tracked by
`win32/.dynwinrt-win32-manifest.json`. Regenerating one metadata root removes
only stale files previously owned by that root.

The machine-readable census is:

```powershell
dynwinrt-codegen win32-census `
  --winmd C:\path\to\Windows.Win32.winmd `
  --json
```

For `Microsoft.Windows.SDK.Win32Metadata 71.0.14-preview`, the baseline is
8,943 complete safe functions out of 18,321 DllImport rows
(48.8128377271983%). Omission reasons are grouped into stable categories.

`windows-metadata` remains behind the flat-local adapter. Parameter rows are
associated by ECMA-335 `Param.Sequence`, and calling convention remains a raw
fact through semantic projection and immutable plan construction.
The pinned metadata reader's lossy convention helper is not used: the adapter
decodes the ECMA `ImplMap` convention mask exactly and rejects stdcall,
thiscall, and fastcall until their target-specific runtime plans exist.
`windows`-generated declarations are used only as differential ABI oracles;
they do not replace the runtime semantic model.
