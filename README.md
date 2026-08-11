# dynwinrt

**Call any Windows Runtime (WinRT) API from JavaScript or TypeScript — without writing a native addon.**

[![@microsoft/dynwinrt](https://img.shields.io/npm/v/@microsoft/dynwinrt.svg?label=%40microsoft%2Fdynwinrt)](https://www.npmjs.com/package/@microsoft/dynwinrt)
[![@microsoft/dynwinrt-codegen](https://img.shields.io/npm/v/@microsoft/dynwinrt-codegen.svg?label=%40microsoft%2Fdynwinrt-codegen)](https://www.npmjs.com/package/@microsoft/dynwinrt-codegen)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## Why dynwinrt?

If you've ever tried to call a modern Windows API (WinAppSDK, Windows AI, notifications, file pickers, sensors, …) from an Electron, Node, or Python app, you've probably hit one of these walls:

- **Writing a C++ `node-addon-api` addon** — needs `node-gyp`, MSVC, Python, the matching Windows SDK, and a CI matrix per Electron version.
- **Writing a C# addon via `node-api-dotnet`** — needs the .NET SDK, a `csproj` build step, and a hand-maintained wrapper for every API surface you want to expose.
- **Waiting for an official projection** — Windows ships `.winmd` metadata months before any JavaScript- or Python-friendly projection appears in a published package.

`dynwinrt` removes all of that. It reads the same `.winmd` metadata your Windows SDK / WinAppSDK NuGet packages already ship and calls the underlying COM vtables **dynamically at runtime via libffi**. The codegen emits typed `.js` + `.d.ts` wrappers; the runtime invokes them through `dynwinrt`'s native binary. No MSBuild step in your app, no `node-gyp`, no per-Windows-version recompile.

```ts
import { LanguageModel } from './bindings/winrt';

const model = await LanguageModel.createAsync();
const result = await model.generateResponseAsync('Tell me a joke');
console.log(result.text);
```

That's the whole story: install, generate, import, call.

> **Scope** — `dynwinrt` primarily targets **data-style WinRT APIs**. WinUI `Application + Window` hosting is also supported on a caller-managed STA UI thread; the application remains responsible for package identity and lifecycle.

## Quick start

```bash
npm install @microsoft/dynwinrt
npm install -D @microsoft/dynwinrt-codegen

# Generate a binding for one class (auto-detects the Windows SDK winmd)
npx dynwinrt-codegen generate \
  --namespace Windows.Foundation \
  --class-name Uri \
  --output ./generated
```

```js
const { roInitialize } = require('@microsoft/dynwinrt');
const { Uri } = require('./generated');

roInitialize(1);                                       // MTA
const uri = new Uri('https://example.com/path?q=1');
console.log(uri.host);                                 // "example.com"
```

Classic COM bindings import their runtime API from the separate
`@microsoft/dynwinrt/com/unsafe` subpath internally. Applications use
`@microsoft/dynwinrt/com` for initialization and managed COM value types;
manual ABI declarations require an explicit `/com/unsafe` import. All are
part of the same npm package, while the package root remains WinRT-only. See
[Classic COM support](docs/architecture/classic-com-support.md) for the supported ABI,
common-interface test matrix, unsupported native types, and ownership rules.
See [Classic COM JavaScript usage](docs/guides/windows/classic-com-usage.md) for codegen,
GUID/IID/CLSID, lifecycle, Automation, and explicit unsafe ABI examples.

Generated bindings project unambiguous public WinRT activation metadata as JavaScript
constructors, including overloads such as `new Uri(base, relative)`. Existing
static factory methods remain available. Classes that can only be returned by
the system, or that expose only protected composition, remain non-constructible;
Python stubs do not advertise their internal native-value wrapping path.
Bindings also include async + progress support, generic collections
(`IVector<T>`, `IMap<K,V>`), structs, enums, and delegates — see
`tools/dynwinrt-codegen/npm/README.md` for the full feature list.

### WinUI `Application + Window`

When `Microsoft.UI.Xaml.Application` is selected, JavaScript codegen also emits `XamlControlsXamlMetaDataProvider` and `XamlControlsResources`. Use the generated helper to compose the application outer, register WinUI metadata, and install the default Fluent resources before creating controls:

```js
const { initWinappsdk, roInitialize } = require('@microsoft/dynwinrt');
const { Application, Button, Window } = require('./generated');

initWinappsdk(2, 2);
roInitialize(0); // STA

async function main() {
  let app;
  await Application.startScheduled(() => {
    app = Application.create(() => {
      const window = new Window();
      window.content = new Button();
      window.activate();
    });
    app.requestedTheme = 1; // Dark
  });
}

main().catch(console.error);
```

`Application.startScheduled()` enters the WinUI dispatcher loop after the
current JavaScript callback unwinds and resolves when the application exits.
This keeps WinUI async completions and JavaScript Promise checkpoints working
while XAML owns the thread. `Application.start()` remains available when the
exact blocking WinRT call is required, but it pauses the Node event loop.

In an unpackaged process,
set `WINAPPSDK_BOOTSTRAP_DLL_PATH` to the architecture-matched
`Microsoft.WindowsAppRuntime.Bootstrap.dll` before calling `initWinappsdk()`.
`Application.create()` resolves the bootstrapped framework resources and
configures its UI thread for Per-Monitor V2 DPI awareness. Packaged processes
can omit the bootstrap call.

## Repository layout

```
dynwinrt/
├── crates/dynwinrt/          # Core Rust runtime (FFI, metadata, async, delegates, collections)
├── bindings/
│   ├── js/                   # @microsoft/dynwinrt — JS / TS bindings (napi-rs)
│   └── py/                   # Python bindings (PyO3; release workflow ready)
├── tools/
│   └── dynwinrt-codegen/     # @microsoft/dynwinrt-codegen — typed-binding generator
├── tests/
│   └── e2e/                  # Cross-language E2E specs, runners, and scripts
├── benchmarks/
│   ├── electron/             # Electron IPC benchmark app
│   └── js/                   # Dynamic and static JS/native benchmarks
├── samples/
│   ├── js/                   # JavaScript/TypeScript samples
│   └── python/               # Python samples
├── docs/                     # Architecture, benchmark, guide, and status docs
└── eng/release/python/       # Python release verification helpers
```

## Build from source

```bash
# Core library
cargo build -p dynwinrt
cargo test  -p dynwinrt

# JS bindings (napi-rs)
cd bindings/js && npm install && npm run build

# Python bindings (PyO3 + maturin) — experimental, not published to PyPI
cd bindings/py && maturin develop && pytest

# Codegen tool
cargo build -p dynwinrt-codegen --release
cargo run   -p dynwinrt-codegen -- generate --namespace Windows.Foundation --class-name Uri --output ./generated
```

Python runtime wheels target CPython 3.11–3.14 on Windows x64 and ARM64. The
standalone `dynwinrt-codegen` Python wheel targets CPython 3.8–3.14 and needs no
Rust installation at consumption time. Release and trusted-publishing
instructions are in [`bindings/py/README.md`](bindings/py/README.md).

Python runtime, codegen, packaging, and WinUI readiness are tracked in
[`docs/status/PYTHON_CHECKLIST.md`](docs/status/PYTHON_CHECKLIST.md).

For deployment, see
[Package a dynwinrt Node.js application as MSIX](docs/guides/windows/msix-packaging.md).

## Codegen CLI reference

| Argument | Required | Description |
|---|---|---|
| `--winmd PATH[;PATH...]` | No | Path to `.winmd` file(s) (auto-detects Windows SDK if omitted) |
| `--folder PATH` | No | Directory containing `.winmd` files |
| `--namespace NAMESPACE` | No | WinRT namespace to generate (omit for all non-`Windows.*` namespaces) |
| `--class-name CLASS` | No | Specific class (transitively pulls in dependencies) |
| `--ref PATH` | No | Additional `.winmd` files for type resolution only (no code emitted) |
| `--lang LANG` | No | `js` (default, emits `.js` + `.d.ts`) or `py` (emits `.py` + `.pyi` and `py.typed`) |
| `--no-pyi` | No | With `--lang py`, emit implementation files without type stubs |
| `--output DIR` | No | Output directory (default `./generated`) |
| `--shared-interface-members` | No | JS opt-in: reuse shared required-interface prototype descriptors instead of duplicating inherited member bodies in every concrete class |
| `--dry-run` | No | Validate input, don't write files |

For each WinRT class the codegen emits a typed wrapper, factory, interface registration, async + progress support, generic collections, structs, enums, delegates, and an `index.js` / `index.d.ts` that re-exports every emitted symbol.

### Shared interface members

Large JavaScript projections can opt into shared inherited-interface
implementations without changing concrete class declarations or member names:

```powershell
dynwinrt-codegen generate `
  --winmd-list .winapp\winmds.txt `
  --class-name Microsoft.UI.Xaml.Controls.Button,Microsoft.UI.Xaml.Controls.TextBlock `
  --output .winapp\bindings `
  --shared-interface-members
```

The generated concrete prototypes receive the same method and accessor
descriptors from standalone shared interface prototypes. Overloaded or
conflicting members remain class-local, raw interface wrapper classes remain
available, and the option does not change generated `.d.ts` files. Only
required interfaces already canonicalized as standalone shared wrappers
participate; one-off inline required interfaces remain class-local. Generation
without this flag is unchanged. If a standalone interface filename is
ambiguous between distinct interface identities, none of those identities
participate in sharing and their inherited members remain class-local.

Focused validation:

```powershell
cargo test -p dynwinrt-codegen --test shared_interface_members_test
```

## Local development — fix import paths in generated files

Generated files import from `'@microsoft/dynwinrt'`. When iterating against a locally-built runtime, rewrite imports to the relative path:

```bash
find generated -name "*.js" -exec sed -i "s|from '@microsoft/dynwinrt'|from '../../dist/winrt.js'|g" {} +
```

## Troubleshooting

| Problem | Solution |
|---------|----------|
| `cargo build` fails with libffi errors | Ensure you have a C compiler (MSVC) and the Windows SDK installed |
| `cargo test -p dynwinrt` fails | Windows SDK must be installed at the default path with `Windows.winmd` |
| JS bindings won't build | Run `npm install` first; requires Node.js 18+ |
| Python bindings won't build | Requires Python 3.8+ and `maturin` (`pip install maturin`) |
| Codegen snapshot tests fail | Line-ending differences — run `cargo test -p dynwinrt-codegen -- --include-ignored` to regenerate |

## Contributing

This project welcomes contributions and suggestions. Most contributions require you to agree to a Contributor License Agreement (CLA) declaring that you have the right to, and actually do, grant us the rights to use your contribution. For details, visit <https://cla.opensource.microsoft.com>.

When you submit a pull request, a CLA bot will automatically determine whether you need to provide a CLA and decorate the PR appropriately (e.g., status check, comment). Simply follow the instructions provided by the bot. You will only need to do this once across all repos using our CLA.

This project has adopted the [Microsoft Open Source Code of Conduct](https://opensource.microsoft.com/codeofconduct/). For more information see the [Code of Conduct FAQ](https://opensource.microsoft.com/codeofconduct/faq/) or contact <opencode@microsoft.com> with any additional questions or comments.

## Trademarks

This project may contain trademarks or logos for projects, products, or services. Authorized use of Microsoft trademarks or logos is subject to and must follow [Microsoft's Trademark & Brand Guidelines](https://www.microsoft.com/en-us/legal/intellectualproperty/trademarks/usage/general). Use of Microsoft trademarks or logos in modified versions of this project must not cause confusion or imply Microsoft sponsorship. Any use of third-party trademarks or logos are subject to those third-party's policies.

## License

This project is licensed under the [MIT License](LICENSE).
