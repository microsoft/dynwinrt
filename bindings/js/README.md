# @microsoft/dynwinrt

**Call any Windows Runtime (WinRT) API from JavaScript — without writing a native addon.**

`dynwinrt` is a runtime library that lets your Node.js or Electron code call modern Windows APIs (WinAppSDK, Windows AI, notifications, file pickers, sensors, storage, networking, …) **directly from JavaScript / TypeScript**, with full IntelliSense, no MSBuild step, no C++ or C# project, and no per-Windows-version recompile.

## Why use this?

If you've ever tried to call a Windows API from an Electron or Node app, you've probably run into one of these:

- **Writing a C++ `node-addon-api` addon.** Needs `node-gyp`, MSVC, Python, the right Windows SDK, and a CI matrix per Electron version.
- **Writing a C# addon via `node-api-dotnet`.** Needs the .NET SDK, a separate `csproj` build step, and a manually-maintained C# wrapper for every API surface you want to expose.
- **Waiting for a typed projection.** Some Windows APIs ship `.winmd` metadata months before any JavaScript-friendly projection appears in a published package.

`dynwinrt` removes all of that. It reads the `.winmd` metadata that ships with the Windows SDK (and WinAppSDK NuGet packages) at **runtime**, resolves the COM vtables, and invokes WinRT methods dynamically. There is no native build step in your Electron project. There is no version pinning — the same generated bindings work across Windows SDK / WinAppSDK revisions as long as the metadata is forward-compatible. You just install `@microsoft/dynwinrt` from npm and call the API.

The runtime primarily targets **data-style WinRT APIs** (AI, storage, notifications, networking, globalization, …). It also supports WinUI `Application + Window` hosting through the generated `Application.create()` helper when the caller supplies an STA UI thread, an initialized Windows App SDK runtime, and application lifecycle. Unpackaged callers can initialize the runtime with `initWinappsdk()`; the helper resolves the framework resources from that package graph. It also enables Per-Monitor V2 DPI awareness on the UI thread.

## Quick start

`@microsoft/dynwinrt` is the **runtime**. You generate the typed bindings ahead of time with [`@microsoft/dynwinrt-codegen`](https://www.npmjs.com/package/@microsoft/dynwinrt-codegen), then import them at runtime:

```bash
npm install @microsoft/dynwinrt
npm install -D @microsoft/dynwinrt-codegen

# Generate a binding for Windows.Foundation.Uri
npx dynwinrt-codegen generate \
  --namespace Windows.Foundation \
  --class-name Uri \
  --output ./generated
```

```js
const { roInitialize } = require('@microsoft/dynwinrt');
const { Uri } = require('./generated');

roInitialize(1);                                      // MTA
const uri = new Uri('https://example.com/path?q=1');
console.log(uri.host);                                // "example.com"
console.log(uri.port);                                // 443
```

Unambiguous public WinRT activation metadata is projected as JavaScript constructors.
Parameterized and composable activations support idiomatic forms such as
`new Uri(base, relative)` and `new StackPanel()`. The generated static factory
methods remain available for compatibility.

Generated `IReference<T>` values use `T | null` in JavaScript. Native values,
`null`, and generated `IReference_*` wrappers are accepted as inputs.

Generated packages export `createProjectedLifetimeScope()`. WinUI/XAML hosts
can create a scope after Application and Window setup, then dispose it before
the native window and XAML core are destroyed. Active scopes retain projected
native values strongly for deterministic release; `releaseProjected()` removes
an individually released wrapper from its scope. Projects that never create a
scope do not retain projected values. Direct runtime users can release an
individual `DynWinRtValue` with `value.release()`.
`projectAs(value, Type)` borrows its input and creates a separately releasable
interface projection; internal generated `_fromNative()` paths consume native
return values.

Generated WinUI `IElementFactory` bindings expose `IElementFactory.create()`.
It creates a synchronous, UI-thread factory backed by JavaScript
`getElement`/`recycleElement` callbacks. Call `releaseCallbacks()` on the
returned factory after clearing its ItemsRepeater source so JavaScript item
state can be released before the native repeater is destroyed.

`DynWinRtValue.createVector()` objects implement `IObservableVector<T>` in
addition to `IIterable<T>`, `IVector<T>`, and `IVectorView<T>`. Mutations emit
the standard `VectorChanged` collection-change notifications. Generated
`IObservableVector<T>` projections expose `asVector()` for mutating collection
properties, and codegen emits the paired `IVector<T>` binding automatically.

## Platform support

- **Windows 10 / 11** — x64 and arm64 native binaries shipped via `napi-rs` prebuilds
- **Node.js** ≥ 16 (Electron, plain Node, vscode extensions, …)

## Links

- 📦 [`@microsoft/dynwinrt-codegen`](https://www.npmjs.com/package/@microsoft/dynwinrt-codegen) — the typed-binding generator
- 🐛 [Source on GitHub](https://github.com/microsoft/dynwinrt) — issues, contributions, internal design docs

## License

[MIT](https://github.com/microsoft/dynwinrt/blob/main/LICENSE)
