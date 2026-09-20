## Prerelease v0.1.0-preview.22

This preview adds standalone WinRT interface implementations, improves Python
asyncio and Classic COM support, and introduces experimental flat Win32 bindings.
It also includes projection, collection, ownership, and native-boundary fixes
since preview.21.

## Packages and requirements

- `@microsoft/dynwinrt` - JavaScript/TypeScript runtime.
- `@microsoft/dynwinrt-codegen` - JavaScript/TypeScript code generator.
- `dynwinrt` - Native Python runtime.
- `dynwinrt-codegen` - Standalone Python code generator.

Runtime packages target Windows x64 and ARM64. JavaScript requires Node.js 18
or later; the Python runtime and generated bindings require CPython 3.11-3.14.
Available APIs also depend on the installed Windows version, SDK components,
package identity, and hardware.

```powershell
npm install @microsoft/dynwinrt
npm install -D @microsoft/dynwinrt-codegen
python -m pip install --pre dynwinrt dynwinrt-codegen
```

## Highlights

### WinRT interface implementations

Implement supported WinRT interfaces in JavaScript or Python with
`.implement(...)` and `.implementation(...)`. Synchronous handlers support typed
interface views, multi-interface composition, and explicit release/disposal.
See the supported contracts and lifetime rules in the
[interface implementation guide](https://github.com/microsoft/dynwinrt/blob/main/docs/guides/windows/winrt-interface-implementations.md).

### Python asyncio and value conversion

- Use generated operations with `asyncio.create_task()` and `TaskGroup`, retaining
  cancellation and progress. Task scheduling is one-shot; direct awaits remain repeatable.
- Copy between `IBuffer` and JavaScript `Buffer`/`Uint8Array` or Python `bytes`/`bytearray`.
- Explicitly unbox supported `IPropertyValue` scalars and arrays with
  `unboxObject()` / `unbox_object()`; generated `Object` results remain unchanged.

### Expanded Classic COM projections

- Expand safe JS/TS projections for HGLOBAL transfer, variable-length audio
  formats, typed activation, and native completion to Promise.
- Add bounded copy helpers for supported audio, WIC, and linear Media Foundation operations.
- Generate canonical namespace output under `generated/com/`, with separate
  unsafe companions and `@microsoft/dynwinrt/com/unsafe/raw`.

Safe support remains contract-specific; unsafe entrypoints retain caller-owned
ABI, ownership, and lifetime obligations.

### Experimental flat Win32 bindings

Generate JS/TS bindings for a validated subset of DLL exports through
`@microsoft/dynwinrt/win32`, including resource cleanup, `LastError`, and
cancellable IOCP-backed `ReadFile`/`WriteFile` Promises. Support is partial and
does not yet provide a backward-compatibility guarantee. See the
[Win32 support guide](https://github.com/microsoft/dynwinrt/blob/main/docs/architecture/flat-win32-contracts.md).

## Correctness and reliability

- Reject mismatched native-wrapper types in JavaScript calls, including nested
  collection inputs and callback results.
- Fix WinRT collection bulk-operation element strides and reject unsupported
  collection layouts or ownership shapes before native use.
- Fix generated JavaScript collection factories for primitive values while
  preserving supported managed-null inputs.
- Make duplicate map-constructor keys follow `Insert`: the last value wins,
  while the first key and its iteration position are retained.
- Register WinRT interfaces by IID rather than short name, avoiding collisions
  between independently generated packages.
- Fix ownership cycles in runtime metadata registration.
- Fix TypeScript array declarations whose element type is a union.
- Fix Python annotation and helper-name collisions, including standalone nested
  struct defaults, and distinguish closed generic interfaces by complete
  semantic identity.
- Enforce Python projection-scope thread affinity and improve WinUI teardown
  through explicit process-level module ownership.
- Resolve CoreMessaging lazily when creating a DispatcherQueue.

## Upgrade notes

Upgrade runtime and codegen together, then regenerate affected bindings.

- **Python:** fully regenerate and rebuild/reinstall all packages exchanging
  closed generic interfaces together. Do not mix old and new stubs.
- **Classic COM (JS/TS):** regenerate all selected roots into a fresh output directory
  and update imports for the namespace layout.
- **Node.js / Electron:** use runtime entrypoints from the same installed package;
  do not mix native objects from different builds.

## Samples and notes

New and updated samples cover WinUI, OCR, asynchronous file I/O, Aion Electron
chat, and flat Win32 scenarios. See each sample's README for required Windows
components, package identity, and hardware.

Classic COM, flat Win32, and WinUI hosting remain experimental.

For installation and usage, see the
[README](https://github.com/microsoft/dynwinrt#readme).
