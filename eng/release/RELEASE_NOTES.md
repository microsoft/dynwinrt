# dynwinrt 0.1.0-preview.22

This preview adds standalone WinRT interface implementations and experimental
flat Win32 bindings, expands safe Classic COM access, and improves Python
usability and Windows examples.
The npm version is `0.1.0-preview.22`; its Python PEP 440 version is `0.1.0rc22`.

## Highlights

- **Implement WinRT interfaces in Node.js/TypeScript and Python.**
  `IBackgroundTask.implement(handlers)` and other supported interface factories
  create standalone `IInspectable` objects without a composable WinUI base.
  The typed management handle exposes a stable `impl.value` and `impl.dispose()`;
  Python also supports `with IBackgroundTask.implement(handler) as impl:`.
  Generated handler declarations cover supported methods, properties, events,
  structs, arrays, and multiple outputs. See the
  [interface implementation guide and samples](https://github.com/microsoft/dynwinrt/blob/main/docs/guides/windows/winrt-interface-implementations.md).
- **Experimental flat Win32 bindings for JavaScript/TypeScript.** Generate typed
  native-function wrappers from `Windows.Win32.winmd` through the separate
  `@microsoft/dynwinrt/win32` entrypoint. Supported contracts provide managed
  resources with explicit `close()` and cancellable `readFileAsync()` /
  `writeFileAsync()` Promise helpers. New
  [Win32 samples](https://github.com/microsoft/dynwinrt/blob/main/samples/js/win32/README.md)
  cover system information, Registry queries, and overlapped file I/O.
- **Expanded safe Classic COM for JavaScript/TypeScript.** More exact contracts
  enable `IWbemServices` conditional outputs and owned thumbnail handles.
  `DynComAudioFormat` validates packed, variable-length `WAVEFORMATEX` headers
  and `cbSize` extensions; safe `IAudioClient`/`IAudioClient2`/`IAudioClient3`
  projections manage CoTaskMem-owned format outputs. Dedicated
  `FORMATETC`/`STGMEDIUM` support covers target-device-independent
  `TYMED_HGLOBAL` transfers.
- **Typed COM acquisition and bounded copies.** `projectAs(...)`, restricted
  in-process `IMMDevice.activate(InterfaceClass)`, and Promise-based
  `activateAudioInterfaceAsync(...)` for `IAudioClient`/`IAudioEndpointVolume`
  simplify managed interface acquisition.
  Owned-copy APIs cover audio render/capture, STA-only BGRA8 WIC reads, and
  linear Media Foundation buffers without exposing borrowed native memory.
  Audio/MF copy facades are not complete interface projections. Generated
  unsafe companions and raw ABI access remain explicit escape hatches, not
  general safe COM support.
  See the [Classic COM usage guide](https://github.com/microsoft/dynwinrt/blob/main/docs/guides/windows/classic-com-usage.md).
- **Python asyncio and value conversion.** Generated operations work with
  `asyncio.create_task()` and `TaskGroup.create_task()`, while retaining direct
  `await`, progress, cancellation, and explicit blocking through `wait()`.
  In both languages, `IBuffer` projections support copied byte conversion, and
  `unboxObject()`/`unbox_object()` explicitly convert supported boxed
  `IPropertyValue` results without changing generic object projection.
- **Windows examples and repeatable setup.**
  [JavaScript WinUI](https://github.com/microsoft/dynwinrt/blob/main/samples/js/winui-tic-tac-toe-code-only/README.md)
  and [Python WinUI](https://github.com/microsoft/dynwinrt/blob/main/samples/python/winui-hello-world/README.md)
  examples demonstrate Windows App SDK initialization and pinned WinApp CLI
  setup. The repaired
  [Windows AI OCR sample](https://github.com/microsoft/dynwinrt/blob/main/samples/js/ocr/README.md)
  uses an identity-aware launcher and opt-in model preparation; it requires
  supported Windows 11/NPU hardware, Windows App SDK 1.8, and Node.js 24+.
- **Local AI chat in Electron.** The
  [Aion Instruct sample](https://github.com/microsoft/dynwinrt/blob/main/samples/js/electron-aion-chat/README.md)
  demonstrates streaming responses, cancellation, and multi-turn conversations
  through generated WinRT bindings. It requires Windows 11 on an ARM64
  Snapdragon Copilot+ PC, the Aion preview framework, and its QNN provider.

Repository samples have additional SDK and source-build requirements; follow
their linked setup guides rather than the packaged-runtime minimums below.

WinRT implementation callbacks are synchronous, non-agile, and owner-thread-only;
Python implementations require the main interpreter. Arbitrary generic
implementation roots remain unsupported. The `IBackgroundTask` samples
demonstrate controlled native calls, not OS background activation. These
factories do not provide task registration/triggers, CLSID activation, package
deployment, or COM local-server hosting.

Flat Win32 is a reviewed subset, not a complete Windows API projection;
arbitrary managed native callbacks and variadic functions remain unsupported.
See the [Win32 contract boundaries](https://github.com/microsoft/dynwinrt/blob/main/docs/architecture/flat-win32-contracts.md).

## Improvements and fixes

- **Lifetime cleanup:** Python projected-lifetime scopes enforce thread
  affinity: same-thread asyncio tasks can share a scope, but worker threads
  need their own and native callbacks do not inherit a foreign scope. Python
  asyncio task wrappers clear progress callbacks on completion, cancellation,
  and release.
  JavaScript subscriptions release temporary delegates, and retained
  unsubscribe functions no longer keep removed handlers alive.
- **Collection correctness:** WinRT collection `GetMany` and `ReplaceAll`
  operations use native element sizes rather than pointer-sized strides for
  small value types, while preserving string and interface ownership.
- **Generated bindings:** Python `.py` and `.pyi` annotations consistently
  resolve same-named types, structs, handlers, and delegates. Unsafe COM
  companions correctly escape JavaScript binding identifiers and use the
  runtime's x86 aggregate descriptor key, fixing affected module imports.
- **UI lifecycle:** Python WinUI activation retains its implementation modules
  until process exit to prevent premature unloading during COM teardown.
  Object, event, and apartment cleanup remain required; in-process WinUI
  unloading is unsupported. The system DispatcherQueue helper loads
  CoreMessaging only when a new current-thread queue is needed.
- **Recoverable Win32 cleanup failures:** Failed result cleanup retains owned
  resources in `DynWin32CallError.cleanupFailures`; `retryCleanup()` retries
  their cleanup without repeating the native call.

Contributor CI now runs independent builds and Rust checks in parallel, reuses
verified build artifacts for E2E, and retains required validation gates.

## Action required

- Use Node.js 18 or later, keep runtime and codegen versions matched, and
  regenerate affected bindings.
- Fully regenerate Classic COM bindings; do not mix old and new modules.
  Outputs use canonical `com/` and `com/unsafe/` paths, not legacy flat paths.
  Import COM through the explicit `@microsoft/dynwinrt/com` entrypoint.
  Runtime and generated package roots remain WinRT-only, with lower-level
  runtime access isolated under
  `@microsoft/dynwinrt/com/unsafe` and `@microsoft/dynwinrt/com/unsafe/raw`.
- Flat Win32 is experimental with no backward-compatibility guarantee yet.
  Keep its generated `win32/` modules separate from WinRT and COM, regenerate
  with the matching runtime/codegen pair, and validate the APIs you use.
- Previously rejected ambiguous COM overload groups now use explicit
  `<camelName>AtSlot<absoluteVtableSlot>` names. Existing distinguishable
  overloads retain their names and dispatch; follow the generated declarations.
- Regenerated Python bindings return `Char16[]` as `list[str]`, matching their
  declarations. Regeneration can change affected long module paths and
  ambiguous helper/root imports; use generated namespace exports rather than
  constructing names.
- The first typed incremental Python generation over an older package may
  need its original WinMD/`--ref` inputs. Supply them or fully regenerate;
  missing metadata fails without replacing the previous output. See the
  [generator upgrade guidance](https://github.com/microsoft/dynwinrt/blob/main/tools/dynwinrt-codegen/README.md).

## Install

```powershell
npm install @microsoft/dynwinrt@0.1.0-preview.22
npm install -D @microsoft/dynwinrt-codegen@0.1.0-preview.22
python -m pip install --pre "dynwinrt==0.1.0rc22" "dynwinrt-codegen==0.1.0rc22"
```

## Packages/platforms

- `@microsoft/dynwinrt`: JavaScript/TypeScript runtime with Windows x64 and
  ARM64 native addons for Node.js 18 or later.
- `@microsoft/dynwinrt-codegen`: typed WinRT, supported Classic COM, and
  experimental flat Win32 generation for JavaScript/TypeScript, emitting
  `.js` and `.d.ts` files.
- `dynwinrt`: CPython 3.11-3.14 runtime wheels for Windows x64 and ARM64.
- `dynwinrt-codegen`: standalone Windows x64 and ARM64 wheels that include a
  prebuilt generator and require no Rust installation.

Consuming the packaged binaries and generated bindings requires no native
compiler. Python code generation remains WinRT-only.

**Detailed changes:** [v0.1.0-preview.21...v0.1.0-preview.22](https://github.com/microsoft/dynwinrt/compare/v0.1.0-preview.21...v0.1.0-preview.22)
