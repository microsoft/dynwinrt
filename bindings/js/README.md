# @microsoft/dynwinrt

**Call Windows Runtime (WinRT) APIs from JavaScript — without writing a native addon.**

`dynwinrt` is a runtime library that lets your Node.js or Electron code call modern Windows APIs (WinAppSDK, Windows AI, notifications, file pickers, sensors, storage, networking, …) **directly from JavaScript / TypeScript**, with full IntelliSense, no MSBuild step, no C++ or C# project, and no per-Windows-version recompile.

## Why use this?

If you've ever tried to call a Windows API from an Electron or Node app, you've probably run into one of these:

- **Writing a C++ `node-addon-api` addon.** Needs `node-gyp`, MSVC, Python, the right Windows SDK, and a CI matrix per Electron version.
- **Writing a C# addon via `node-api-dotnet`.** Needs the .NET SDK, a separate `csproj` build step, and a manually-maintained C# wrapper for every API surface you want to expose.
- **Waiting for a typed projection.** Some Windows APIs ship `.winmd` metadata months before any JavaScript-friendly projection appears in a published package.

`dynwinrt-codegen` reads the `.winmd` metadata that ships with the Windows SDK
and WinAppSDK ahead of time. The generated JavaScript registers the required
interface signatures, and `dynwinrt` resolves and invokes the COM vtables
dynamically at runtime. There is no native build step in the consuming Electron
or Node project. The same generated bindings can work across compatible
metadata revisions without rebuilding a native addon.

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

### Generated module layout

Generated WinRT implementations use canonical namespace paths. Prefer the root
barrel for concise imports:

```js
const { Uri, Button } = require('./generated');
```

Use the canonical path when a deep import is needed:

```js
const { Uri } = require('./generated/windows/foundation/Uri.js');
const { Button } = require(
  './generated/microsoft/ui/xaml/controls/Button.js'
);
```

Legacy flat paths such as `./generated/Uri.js` are not generated. When metadata
contains duplicate short names, each canonical module keeps its native symbol
name while the root barrel uses a namespace-qualified name, such as
`AIFoundationEmbeddingVector` or `SemanticSearchEmbeddingVector`.

Classic COM is a preview under active development. It uses a separate subpath
from the same package, keeping the WinRT root API unchanged:

```js
const { initializeCom } = require('@microsoft/dynwinrt/com');
initializeCom(1); // MTA
```

COM interface values returned by activation, `QueryInterface`, or typed
interface out-parameters own one reference and release it when their
`DynWinRtValue` is released or collected. Manual interface registration,
native signatures, and raw pointer ownership are isolated under
`@microsoft/dynwinrt/com/unsafe`. `adoptOwnedComPointer()` there consumes one
explicit caller-supplied `+1` reference; Buffer backing addresses are not
accepted. Win32 handles are not COM references and require their own
type-specific cleanup function.

See the repository's
[Classic COM JavaScript usage guide](../../docs/guides/windows/classic-com-usage.md) for
codegen, current coverage and limitations, GUID/IID/CLSID, lifetime, Automation,
and `/com/unsafe` examples.

Unambiguous public WinRT activation metadata is projected as JavaScript constructors.
Parameterized and composable activations support idiomatic forms such as
`new Uri(base, relative)` and `new StackPanel()`. The generated static factory
methods remain available for compatibility.

Generated `IReference<T>` values use `T | null` in JavaScript. Native values,
`null`, and generated `IReference_*` wrappers are accepted as inputs.
The same projection applies when `IReference<T>` appears inside a WinRT struct;
packing boxes the field automatically and unpacking returns the native value.

Generated packages export `createProjectedLifetimeScope()`. WinUI/XAML hosts
can create a scope after Application and Window setup, then dispose it before
the native window and XAML core are destroyed. Active scopes retain projected
native values strongly for deterministic release; `releaseProjected()` removes
an individually released wrapper from its scope. Projects that never create a
scope do not retain projected values. Direct runtime users can release an
individual `DynWinRtValue` with `value.release()`.
`projectAs(value, Type)` is the public conversion for APIs whose metadata
returns `Object`/`IInspectable` even though the application knows the concrete
runtime class. It accepts either a raw projected value or an existing wrapper,
borrows the input, and creates a separately releasable projection:

```js
import {
  projectAs,
  StackPanel,
  XamlReader,
} from "./generated/index.mjs";

const raw = XamlReader.load(xaml);
if (raw === null) throw new Error("XamlReader returned no value");
const panel = projectAs(raw, StackPanel);
```

Use `wrapper.as(InterfaceClass)` when converting an existing runtime-class
wrapper to another interface view. Use `projectAs(raw, RuntimeClass)` when
converting a raw value to a generated runtime class. A failed QueryInterface is
reported as an error. Internal generated `_fromNative()` paths consume native
return values; application code should use `projectAs()` instead.

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
- **Node.js** ≥ 18 (Electron, plain Node, VS Code extensions, …)

## Links

- 📦 [`@microsoft/dynwinrt-codegen`](https://www.npmjs.com/package/@microsoft/dynwinrt-codegen) — the typed-binding generator
- 🐛 [Source on GitHub](https://github.com/microsoft/dynwinrt) — issues, contributions, internal design docs

## License

[MIT](https://github.com/microsoft/dynwinrt/blob/main/LICENSE)
