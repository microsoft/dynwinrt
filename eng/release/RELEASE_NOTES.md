# dynwinrt 0.1.0-preview.22

This preview adds standalone WinRT interface implementations, expands safe
Classic COM access, and improves Python usability and Windows examples.
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
- **Expanded safe Classic COM for JavaScript/TypeScript.** More exact contracts
  enable `IWbemServices` conditional outputs and owned thumbnail handles.
  `DynComAudioFormat` validates packed, variable-length `WAVEFORMATEX` headers
  and `cbSize` extensions; safe `IAudioClient`/`IAudioClient2`/`IAudioClient3`
  projections manage CoTaskMem-owned format outputs. Dedicated
  `FORMATETC`/`STGMEDIUM` support covers target-device-independent
  `TYMED_HGLOBAL` transfers.
- **Typed COM acquisition and bounded copies.** `projectAs(...)`, restricted
  in-process `IMMDevice.activate(InterfaceClass)`, and Promise-based
  `activateAudioInterfaceAsync(...)` simplify managed interface acquisition.
  Owned-copy APIs cover audio render/capture, STA-only BGRA8 WIC reads, and
  linear Media Foundation buffers without exposing borrowed native memory.
  Audio/MF copy facades are not complete interface projections. Generated
  unsafe companions and raw ABI access remain explicit escape hatches, not
  general safe COM support.
  See the [Classic COM usage guide](https://github.com/microsoft/dynwinrt/blob/main/docs/guides/windows/classic-com-usage.md).
- **Python asyncio task integration.** Generated operations work with
  `asyncio.create_task()` and `TaskGroup.create_task()`, while retaining direct
  `await`, progress, cancellation, and explicit blocking through `wait()`.
- **Copied bytes and explicit unboxing.** `IBuffer` projections provide safe
  copied byte conversion, and `unboxObject()`/`unbox_object()` explicitly convert
  supported boxed `IPropertyValue` results without changing generic object
  projection.
- **WinUI examples and repeatable setup.** New
  [JavaScript](https://github.com/microsoft/dynwinrt/blob/main/samples/js/winui-tic-tac-toe-code-only/README.md)
  samples and refreshed
  [Python examples](https://github.com/microsoft/dynwinrt/blob/main/samples/python/winui-hello-world/README.md)
  demonstrate Windows App SDK initialization and restoring pinned SDK inputs
  with WinApp CLI.
- **Local AI chat in Electron.** The
  [Aion Instruct sample](https://github.com/microsoft/dynwinrt/blob/main/samples/js/electron-aion-chat/README.md)
  demonstrates streaming responses, cancellation, and multi-turn conversations
  through generated WinRT bindings. It requires Windows 11 on an ARM64
  Snapdragon Copilot+ PC, the Aion preview framework, and its QNN provider.

WinRT implementation callbacks are synchronous, non-agile, and owner-thread-only;
arbitrary generic implementation roots remain unsupported. The `IBackgroundTask`
samples demonstrate controlled native calls, not OS background activation.
These factories do not provide task registration/triggers, CLSID activation,
package deployment, or COM local-server hosting.

## Improvements and fixes

Python projected-lifetime scopes now enforce thread affinity: same-thread
asyncio tasks can share a scope, while worker threads must create their own
and native callbacks do not inherit a foreign scope. Async progress handlers
are deactivated on completion, cancellation, or release. Generated JavaScript
WinRT subscriptions release temporary delegate references; retaining an
unsubscribe function no longer keeps a removed handler alive.

## Action required

- Use Node.js 18 or later, keep runtime and codegen versions matched, and
  regenerate affected bindings.
- Fully regenerate Classic COM bindings; do not mix old and new modules.
  Outputs use canonical `com/` and `com/unsafe/` paths, not legacy flat paths.
  Import COM through the explicit `@microsoft/dynwinrt/com` entrypoint.
  Runtime and generated package roots remain WinRT-only, with lower-level
  runtime access isolated under
  `@microsoft/dynwinrt/com/unsafe` and `@microsoft/dynwinrt/com/unsafe/raw`.
- Ambiguous COM overloads now use explicit
  `<camelName>AtSlot<absoluteVtableSlot>` names; use the generated declarations
  instead of an ambiguous unsuffixed call.
- Python `Char16[]` results now match `list[str]`. Regeneration can change
  affected long module paths and ambiguous helper/root imports; use generated
  namespace exports rather than constructing names.
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
- `@microsoft/dynwinrt-codegen`: typed WinRT and supported Classic COM
  generation for JavaScript/TypeScript, emitting `.js` and `.d.ts` files.
- `dynwinrt`: CPython 3.11-3.14 runtime wheels for Windows x64 and ARM64.
- `dynwinrt-codegen`: standalone Windows x64 and ARM64 wheels that include a
  prebuilt generator and require no Rust installation.

Consuming the packaged binaries and generated bindings requires no native
compiler. Python code generation remains WinRT-only.

**Detailed changes:** [v0.1.0-preview.21...v0.1.0-preview.22](https://github.com/microsoft/dynwinrt/compare/v0.1.0-preview.21...v0.1.0-preview.22)
