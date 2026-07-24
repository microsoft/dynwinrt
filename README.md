# dynwinrt

**Call Windows APIs — WinRT, classic COM, and flat Win32 (`[DllImport]`) — from JavaScript or TypeScript without writing a native addon.**

[![@microsoft/dynwinrt](https://img.shields.io/npm/v/@microsoft/dynwinrt.svg?label=%40microsoft%2Fdynwinrt)](https://www.npmjs.com/package/@microsoft/dynwinrt)
[![@microsoft/dynwinrt-codegen](https://img.shields.io/npm/v/@microsoft/dynwinrt-codegen.svg?label=%40microsoft%2Fdynwinrt-codegen)](https://www.npmjs.com/package/@microsoft/dynwinrt-codegen)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## Why dynwinrt?

If you've ever tried to call a modern Windows API (WinAppSDK, Windows AI, notifications, file pickers, sensors, …) from an Electron, Node, or Python app, you've probably hit one of these walls:

- **Writing a C++ `node-addon-api` addon** — needs `node-gyp`, MSVC, Python, the matching Windows SDK, and a CI matrix per Electron version.
- **Writing a C# addon via `node-api-dotnet`** — needs the .NET SDK, a `csproj` build step, and a hand-maintained wrapper for every API surface you want to expose.
- **Waiting for an official projection** — Windows ships `.winmd` metadata months before any JavaScript- or Python-friendly projection appears in a published package.

`dynwinrt` removes all of that. It reads the same `.winmd` metadata your Windows SDK / WinAppSDK NuGet packages already ship and calls the underlying COM vtables **dynamically at runtime via libffi**. The codegen emits typed `.js` + `.d.ts` wrappers; the runtime invokes them through `dynwinrt`'s native binary. No MSBuild step in your app, no `node-gyp`, no per-Windows-version recompile. The same runtime and generator now cover three metadata-driven surfaces — **WinRT**, **classic COM**, and **flat Win32 (`[DllImport]`)** — so you can reach Windows APIs whether or not they ship as WinRT runtime classes.

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

Generated bindings project unambiguous public WinRT activation metadata as JavaScript
constructors, including overloads such as `new Uri(base, relative)`. Existing
static factory methods remain available. Classes that can only be returned by
the system, or that expose only protected composition, remain non-constructible.
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

let app;
Application.start(() => {
  app = Application.create(() => {
    const window = new Window();
    window.content = new Button();
    window.activate();
  });
  app.requestedTheme = 1; // Dark
});
```

`Application.start()` runs the WinUI dispatcher loop. In an unpackaged process,
set `WINAPPSDK_BOOTSTRAP_DLL_PATH` to the architecture-matched
`Microsoft.WindowsAppRuntime.Bootstrap.dll` before calling `initWinappsdk()`.
`Application.create()` resolves the bootstrapped framework resources and
configures its UI thread for Per-Monitor V2 DPI awareness. Packaged processes
can omit the bootstrap call.

## New: Win32 and classic COM support (JavaScript/TypeScript)

`dynwinrt` now reaches beyond WinRT runtime classes. From the same metadata-driven pipeline it can also project **Windows APIs that are not WinRT**:

- **Classic COM** — `IUnknown`-rooted interfaces created via `CoCreateInstance` and dispatched through the COM vtable (e.g. taskbar, shell).
- **WinRT interop bridges** — the desktop `*Interop` shims a windowed app needs to reach WinRT features from an `HWND` (Share, media controls, file-picker parenting).
- **Flat Win32 `[DllImport]`** — plain DLL exports (e.g. Registry, credentials) called through `LoadLibrary` + `GetProcAddress` + libffi.

Generate Win32/COM wrappers by pointing the codegen at the Win32 metadata (`Windows.Win32.winmd` from the [win32metadata](https://github.com/microsoft/win32metadata) package):

### Getting `Windows.Win32.winmd`

`Windows.Win32.winmd` is not on a stock Windows machine — it ships in the MIT-licensed NuGet package [`Microsoft.Windows.SDK.Win32Metadata`](https://www.nuget.org/packages/Microsoft.Windows.SDK.Win32Metadata). A `.nupkg` is just a zip with the `.winmd` at its root, so **copy-paste this whole block into PowerShell as-is** — it fetches the latest winmd and generates `ITaskbarList3` wrappers into `./generated` (no NuGet client, no version number to pick):

```powershell
$pkg = 'microsoft.windows.sdk.win32metadata'
$ver = (Invoke-RestMethod "https://api.nuget.org/v3-flatcontainer/$pkg/index.json").versions[-1]
Invoke-WebRequest "https://api.nuget.org/v3-flatcontainer/$pkg/$ver/$pkg.$ver.nupkg" -OutFile "$env:TEMP\win32meta.zip"
Expand-Archive "$env:TEMP\win32meta.zip" -DestinationPath .\win32meta -Force
npx dynwinrt-codegen generate --winmd .\win32meta\Windows.Win32.winmd --namespace Windows.Win32.UI.Shell --class-name ITaskbarList3 --output ./generated
```

Swap `--namespace` / `--class-name` for whatever API you need; reuse the same `.\win32meta\Windows.Win32.winmd` for every run.

> Win32 and classic-COM generation currently emits JavaScript/TypeScript (`.js` + `.d.ts`).

### Classic COM: `CoCreateInstance` + vtable interfaces

```js
import { ITaskbarList3 } from './generated/ITaskbarList3.js';
import { TBPFLAG } from './generated/TBPFLAG.js';

// HWND is generated as `bigint | number`. In Electron, read the handle value
// out of BrowserWindow.getNativeWindowHandle() (a Buffer) — don't pass the
// Buffer itself:  const hwnd = win.getNativeWindowHandle().readBigUInt64LE(0);
const hwnd = 0x0000000000123456n; // your real HWND

const taskbar = ITaskbarList3.create(); // CoCreateInstance under the hood
taskbar.hrInit();
taskbar.setProgressState(hwnd, TBPFLAG.TBPF_NORMAL);
taskbar.setProgressValue(hwnd, 40n, 100n); // 40%
```

The generated wrapper exposes natural typed methods while the runtime handles `CoCreateInstance`, interface registration, pointer arguments, `HRESULT` checks, and vtable slot dispatch — no hand-written IIDs, `REFIID`, `void**`, or vtable indices.

### WinRT interop bridges: from `HWND` to WinRT objects

```js
import { IDataTransferManagerInterop } from './generated/IDataTransferManagerInterop.js';

const hwnd = 0x0000000000123456n; // your real HWND as a bigint (see the Electron note above)

const interop = IDataTransferManagerInterop.create();
const dtm = interop.getForWindow(hwnd); // → DynWinRtValue bridge to DataTransferManager
interop.showShareUIForWindow(hwnd);
```

Interop interfaces are the Windows pattern for features that require an `HWND` but operate on WinRT objects. `create()` activates the WinRT factory and QIs to the interop interface; `getForWindow(hwnd)` adopts the returned COM pointer and hands it back as a `DynWinRtValue` bridge — with no `riid`/`void**` in the signature.

### Flat Win32 exports: call `[DllImport]` APIs

```js
import { regOpenKeyExW, regCloseKey } from './generated/Apis.js';
import { REG_SAM_FLAGS } from './generated/REG_SAM_FLAGS.js';

const HKEY_LOCAL_MACHINE = 0x80000002n;

const open = regOpenKeyExW(
  HKEY_LOCAL_MACHINE,
  'SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion', // note: doubled backslashes in JS
  0,
  REG_SAM_FLAGS.KEY_READ,
);
if (open.status !== 0) throw new Error(`RegOpenKeyExW failed: LSTATUS=${open.status}`);
try {
  console.log(`hKey = 0x${open.phkResult.toString(16)}`);
} finally {
  regCloseKey(open.phkResult);
}
```

`[out]` parameters become return fields (`{ status, phkResult }`), caller-allocated buffers and in/out sizes are handled for you, `LPCWSTR` ↔ `string`, and the Win32 `LSTATUS`/error is surfaced. Handles (`HKEY`, `HANDLE`) project as `bigint`.

## Repository layout

```
dynwinrt/
├── crates/dynwinrt/          # Core Rust runtime (FFI, metadata, async, delegates, collections)
├── bindings/
│   ├── js/                   # @microsoft/dynwinrt — JS / TS bindings (napi-rs)
│   └── py/                   # Python bindings (PyO3, experimental — not published)
├── tools/
│   └── dynwinrt-codegen/     # @microsoft/dynwinrt-codegen — typed-binding generator
├── tests/                    # Integration tests + sample E2E projects
└── bench-electron/           # Electron benchmark app
```

## Build from source

```bash
# Core library
cargo build -p dynwinrt
cargo test  -p dynwinrt

# JS bindings (napi-rs)
cd bindings/js && npm install && npx napi build --no-const-enum --platform --release -o dist

# Python bindings (PyO3 + maturin) — experimental, not published to PyPI
cd bindings/py && maturin develop && pytest

# Codegen tool
cargo build -p dynwinrt-codegen --release
cargo run   -p dynwinrt-codegen -- generate --namespace Windows.Foundation --class-name Uri --output ./generated
```

Python runtime, codegen, packaging, and WinUI readiness are tracked in
[`PYTHON_CHECKLIST.md`](PYTHON_CHECKLIST.md).

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
| `--dry-run` | No | Validate input, don't write files |

For each WinRT class the codegen emits a typed wrapper, factory, interface registration, async + progress support, generic collections, structs, enums, delegates, and an `index.js` / `index.d.ts` that re-exports every emitted symbol.

## Local development — fix import paths in generated files

Generated files import from `'@microsoft/dynwinrt'`. When iterating against a locally-built runtime, rewrite imports to the relative path:

```bash
find generated -name "*.js" -exec sed -i "s|from '@microsoft/dynwinrt'|from '../../dist/index.js'|g" {} +
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
