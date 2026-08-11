# @microsoft/dynwinrt-codegen

**Generate typed JavaScript + TypeScript bindings for any Windows Runtime (WinRT) API from `.winmd` metadata.**

Pair this with the [`@microsoft/dynwinrt`](https://www.npmjs.com/package/@microsoft/dynwinrt) runtime to call modern Windows APIs (WinAppSDK, Windows AI, notifications, storage, networking, …) **directly from JavaScript / TypeScript** — full IntelliSense, no native build step, no C# projection, no per-Windows-version recompile.

## Why use this?

Until now, the choices for calling a Windows API from Node.js or Electron were:

- **Write a C++ `node-addon-api` addon** — needs `node-gyp`, MSVC, Python, the right Windows SDK, and a CI matrix per Electron version.
- **Write a C# addon via `node-api-dotnet`** — needs the .NET SDK, a `csproj` build step, and a hand-maintained C# wrapper for every API surface.
- **Wait for an official projection** — Windows ships `.winmd` metadata months before a JavaScript-friendly projection appears.

`dynwinrt-codegen` reads the same `.winmd` metadata the Windows SDK already ships and emits **typed JavaScript + `.d.ts` wrappers** that call WinRT through [`@microsoft/dynwinrt`](https://www.npmjs.com/package/@microsoft/dynwinrt) at runtime. There is no native build in your Electron / Node project. You generate the bindings once, commit them (or regenerate on demand), and import them like any other module:

```ts
import { LanguageModel, LanguageModelOptions } from './bindings/winrt';
const model = await LanguageModel.createAsync();
```

You get IntelliSense in your IDE, type errors at `tsc` time, and the underlying COM call dispatched dynamically at runtime — no MSBuild involved.

The trade-off: `dynwinrt-codegen` is designed for **data-style WinRT APIs** (AI, storage, notifications, networking, globalization, …) and skips XAML / WinUI namespaces, which need composable-class aggregation patterns the codegen doesn't implement. For everything else, this is the easiest path from JavaScript to native Windows.

## CLI usage

```bash
npm install -D @microsoft/dynwinrt-codegen @microsoft/dynwinrt

# A single class (auto-detects the Windows SDK winmd)
npx dynwinrt-codegen generate \
  --namespace Windows.Foundation \
  --class-name Uri \
  --output ./generated

# An entire namespace
npx dynwinrt-codegen generate \
  --namespace Windows.Web.Http \
  --output ./generated

# A custom .winmd (e.g., a WinAppSDK NuGet package or your own SDK)
npx dynwinrt-codegen generate \
  --winmd "C:\path\to\Microsoft.WindowsAppSDK.AI.winmd" \
  --output ./generated
```

### Flags

| Flag | Description |
|---|---|
| `--winmd PATH[;PATH...]` | Path to `.winmd` file(s) (auto-detects Windows SDK if omitted) |
| `--folder PATH` | Directory containing `.winmd` files |
| `--namespace NAMESPACE` | WinRT namespace to generate (omit for all non-`Windows.*` namespaces) |
| `--class-name CLASS` | Specific class (transitively pulls in dependencies) |
| `--ref PATH` | Additional `.winmd` files for type resolution only (no code emitted) |
| `--lang LANG` | `js` (default, emits `.js` + `.d.ts`) or `py` (Python) |
| `--output DIR` | Output directory (default `./generated`) |
| `--shared-interface-members` | JS opt-in that shares inherited interface member descriptors across concrete classes |
| `--dry-run` | Validate input, don't write files |

### Shared interface members

```powershell
npx dynwinrt-codegen generate `
  --winmd-list .winapp\winmds.txt `
  --class-name Microsoft.UI.Xaml.Controls.Button,Microsoft.UI.Xaml.Controls.TextBlock `
  --output .winapp\bindings `
  --shared-interface-members
```

The generated concrete prototypes reuse method and accessor descriptors from
the standalone required-interface prototypes. Overloaded or conflicting
members remain class-local, raw interface wrappers remain available, generated
`.d.ts` files are unchanged, and generation without the flag is unchanged.
Only interfaces already emitted as canonical standalone shared wrappers
participate; one-off inline required interfaces remain class-local.

## What gets generated

For each WinRT class, the codegen emits:

- **A typed wrapper class** with properties and methods using camelCase JS conventions
- **JavaScript constructors** for unambiguous public default, factory, and composable activations
- **The original factory methods** (`.create(...)`, `.createInstance(...)`) for compatibility
- **An interface registration** (`DynWinRtType.registerInterface()`) wired to the COM vtable
- **A JavaScript-backed `IElementFactory.create()` helper** for WinUI
  ItemsRepeater realization and recycling
- **`IAsyncOperation<T>` awaitables** with `.progress(cb)` for streaming results
- **Generic collections** (`IVector<T>`, `IMap<K,V>`, `IIterable<T>`)
- **Creatable observable vectors** that expose both `IObservableVector<T>`
  events and `IVector<T>` mutation helpers
- **Structs** with `pack`/`unpack` helpers
- **Enums** (`Object.freeze`'d in JS, `enum` in `.d.ts`)
- **Delegate types** (IID + parameter signatures) for event handlers
- **An `index.js` + `index.d.ts`** re-exporting every emitted symbol from one place

## Platform

- **Windows only** (x64 / arm64) — the binary is built per architecture and selected automatically by the npm install
- The generated bindings depend on [`@microsoft/dynwinrt`](https://www.npmjs.com/package/@microsoft/dynwinrt) at runtime

## Links

- 📦 [`@microsoft/dynwinrt`](https://www.npmjs.com/package/@microsoft/dynwinrt) — the runtime the generated code targets
- 🐛 [Source on GitHub](https://github.com/microsoft/dynwinrt) — issues, contributions, internal design docs

## License

[MIT](https://github.com/microsoft/dynwinrt/blob/main/LICENSE)
