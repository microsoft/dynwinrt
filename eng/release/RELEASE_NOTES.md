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
`.implement(...)` and `.implementation(...)`. Synchronous, owner-thread handlers
support methods, properties, events, supported structs/arrays, and named multiple
outputs, with typed interface views, multi-interface composition, and explicit
release/disposal. This does not register an OS class or create a COM server.
See the supported contracts and lifetime rules in the
[interface implementation guide](https://github.com/microsoft/dynwinrt/blob/main/docs/guides/windows/winrt-interface-implementations.md).

### Python asyncio and value conversion

- Use generated operations with `asyncio.create_task()` and `TaskGroup`, retaining
  cancellation and progress. Task scheduling is one-shot; direct awaits remain repeatable.
- Copy between `IBuffer` and JavaScript `Buffer`/`Uint8Array` or Python `bytes`/`bytearray`.
- Explicitly unbox supported `IPropertyValue` scalars and arrays with
  `unboxObject()` / `unbox_object()`; generated `Object` results remain unchanged.

### Expanded Classic COM projections

- Expand contract-validated JS/TS projections, including HGLOBAL transfer and
  variable-length audio formats.
- Add non-consuming `projectAs(...)` interface views, bounded typed activation,
  and native completion-to-Promise support for `ActivateAudioInterfaceAsync`.
- Add bounded copy helpers for supported audio, WIC, and linear Media Foundation operations.
- Generate canonical namespace output under `generated/com/`, with separate
  unsafe companions and `@microsoft/dynwinrt/com/unsafe/raw`.
- Give previously ambiguous ordinary overloads explicit slot-qualified names
  and centralize metadata, ownership, and cleanup contract evidence.

Safe support remains contract-specific; unsafe entrypoints retain caller-owned
ABI, ownership, and lifetime obligations.

### Experimental flat Win32 bindings

Generate JS/TS bindings for a validated subset of DLL exports through
`@microsoft/dynwinrt/win32`, with CJS/ESM namespace modules, status and `LastError`
handling, managed resource cleanup, and explicit Winsock, GDI+, and Media
Foundation contexts. File I/O includes cancellable IOCP-backed
`ReadFile`/`WriteFile` Promises with bounded buffers. Support is partial and does
not yet provide a backward-compatibility guarantee. See the
[Win32 support guide](https://github.com/microsoft/dynwinrt/blob/main/docs/architecture/flat-win32-contracts.md).

## Correctness and reliability

- Preserve signed and unsigned WinRT enum backing types in signatures, generic
  IIDs, and value conversion, including high-bit constants, arrays, struct
  fields, boxed references, and callbacks.
- Normalize signed 32-bit patterns from JavaScript bitwise operations for
  generated WinRT and Classic COM UInt32 flags inputs, without relaxing
  ordinary UInt32 validation.
- Reject mismatched native-wrapper types in JavaScript calls, including nested
  collection inputs and callback results.
- Fix WinRT collection bulk-operation element strides and reject unsupported
  collection layouts or ownership shapes before native use.
- Accept explicit `null` for generated JavaScript WinRT collection inputs;
  omitted or `undefined` arguments do not silently become null.
- Fix JavaScript collection factories and automatic `IReference<T>` element
  boxing, including native array parameters. Convert JavaScript `Map` inputs
  to independently owned `IMapView` snapshots through the native `GetView` operation.
- Make duplicate map-constructor keys follow `Insert`: the last value wins,
  while the first key and its iteration position are retained. Compare admitted
  struct elements and keys by their fields rather than padding, using numerical
  floating-point equality.
- Register WinRT interfaces by IID rather than short name, avoiding collisions
  between independently generated packages.
- Fix ownership cycles in runtime metadata registration.
- Preserve Win32 resource ownership when explicit cleanup fails, allowing cleanup
  to be retried.
- Fix TypeScript union-element array declarations and align nullable collection
  output types with runtime values.
- Fix generated COM identifier escaping and architecture-specific aggregate keys.
- Fix Python annotation and helper-name collisions, including standalone nested
  struct defaults, and distinguish closed generic interfaces by complete
  semantic identity. Improve instance/interface input typing and preserve
  subclass types in projected factories.
- Return Python `Char16[]` values as characters, matching their `list[str]`
  declarations. Code that relied on integer elements can use `ord()` explicitly.
- Enforce Python projection-scope thread affinity and improve WinUI teardown
  through explicit process-level module ownership. Activated WinUI implementation
  modules remain loaded until process exit; in-process hot replacement is not supported.
- Resolve CoreMessaging for DispatcherQueue creation and GDI/USER32 helpers
  lazily, avoiding their unconditional direct production DLL dependencies.

## Upgrade notes

Upgrade runtime and codegen together, then regenerate affected bindings.
Regenerate all packages sharing unsigned WinRT enum declarations together;
do not mix old signed declarations with corrected unsigned declarations.

- **Python:** fully regenerate and rebuild/reinstall all packages exchanging
  closed generic interfaces together. Do not mix old and new stubs. Prefer public
  namespace exports over hard-coded internal or long generated module paths.
  Worker threads need their own `RoApartment` and `projected_lifetime_scope()`.
- **JavaScript/TypeScript collections:** guard nullable collection outputs.
  A present `null` differs from the `undefined` returned for a missing map key
  or out-of-range `at()` index. Map `get()` now propagates conversion and native
  errors; its `HasKey`/`Lookup` sequence is not atomic against native mutation.
- **Low-level JavaScript UInt32 inputs:** factories, array constructors, and fast
  setters reject negative, fractional, non-finite, and out-of-range values
  instead of silently wrapping them. Generated UInt32 flags projections handle
  ordinary JavaScript bitwise combinations.
- **Classic COM (JS/TS):** regenerate all selected roots into a fresh output directory
  and update imports for the namespace layout.
- **Node.js / Electron:** use runtime entrypoints from the same installed package;
  do not mix native objects from different builds.

## Samples and notes

New and updated samples cover standalone interface implementations, XAML and
code-only WinUI apps, OCR, asynchronous file I/O, Aion Electron chat, and flat
Win32 system/registry/file operations. Setup guidance adopts WinApp CLI and
clarifies metadata, packaging, Windows component, and hardware prerequisites.
See each sample's README before running it.

Build and release workflows gain parallel validation, expanded projection and
native-boundary regression coverage, and file-based release notes. Native
dependency inspection no longer requires Visual Studio. Guides, JavaScript
binding internals, and benchmark dependencies are also updated.

Classic COM, flat Win32, and WinUI hosting remain experimental.

For installation and usage, see the
[README](https://github.com/microsoft/dynwinrt#readme).
