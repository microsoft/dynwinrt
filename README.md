# dynwinrt

**Call Windows Runtime (WinRT) APIs from JavaScript, TypeScript, or Python — without writing a native extension.**

[![@microsoft/dynwinrt](https://img.shields.io/npm/v/@microsoft/dynwinrt.svg?label=%40microsoft%2Fdynwinrt)](https://www.npmjs.com/package/@microsoft/dynwinrt)
[![@microsoft/dynwinrt-codegen](https://img.shields.io/npm/v/@microsoft/dynwinrt-codegen.svg?label=%40microsoft%2Fdynwinrt-codegen)](https://www.npmjs.com/package/@microsoft/dynwinrt-codegen)
[![dynwinrt on PyPI](https://img.shields.io/pypi/v/dynwinrt.svg?label=PyPI%20dynwinrt)](https://pypi.org/project/dynwinrt/)
[![dynwinrt-codegen on PyPI](https://img.shields.io/pypi/v/dynwinrt-codegen.svg?label=PyPI%20dynwinrt-codegen)](https://pypi.org/project/dynwinrt-codegen/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## Why dynwinrt?

If you've ever tried to call a modern Windows API (WinAppSDK, Windows AI, notifications, file pickers, sensors, …) from an Electron, Node, or Python app, you've probably hit one of these walls:

- **Writing a native extension for each API surface** — needs C++, Rust, or C#, the matching Windows SDK, and language-specific build tooling.
- **Bridging through another runtime** — adds deployment dependencies and a hand-maintained wrapper for every API you expose.
- **Waiting for an official projection** — Windows ships `.winmd` metadata months before any JavaScript- or Python-friendly projection appears in a published package.

`dynwinrt` reads the same `.winmd` metadata shipped by the Windows SDK and WinAppSDK, then calls the underlying COM vtables **dynamically at runtime via libffi**. The codegen emits typed `.js` + `.d.ts` or `.py` + `.pyi` wrappers; the matching native runtime invokes them. Consuming applications do not need MSBuild, `node-gyp`, Cargo, or a native compiler.

> **Scope** — `dynwinrt` primarily targets **data-style WinRT APIs**. WinUI
> `Application + Window` hosting is also supported on a caller-managed STA UI
> thread. Classic COM has a separate preview surface described below.

## Quick start

### JavaScript / TypeScript

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

### Python

```powershell
python -m pip install --pre dynwinrt dynwinrt-codegen

# Generate and install one typed projection package.
dynwinrt-codegen generate `
  --namespace Windows.Foundation `
  --class-name Uri `
  --lang py `
  --output .\generated_uri
python -m pip install .\generated_uri
```

```python
from dynwinrt import RoApartment, projected_lifetime_scope
from generated_uri.windows.foundation import Uri

with RoApartment(1), projected_lifetime_scope():
    uri = Uri("https://example.com/path?q=1")
    print(uri.host)  # "example.com"
```

Initialize one apartment per thread that uses WinRT. Use `RoApartment(1)` for a
normal MTA thread and `RoApartment(0)` for an STA UI thread. The projection
lifetime scope releases generated wrappers before the apartment closes.

## Generated API

Both projections expose public WinRT activation metadata as normal
constructors, preserve static factory methods, and generate properties,
overloads, async operations with progress, collections, structs, enums,
delegates, and events. JavaScript uses camelCase names; Python uses snake_case
names, native Python values, asyncio-compatible awaitables, and type stubs.

- [JavaScript/TypeScript codegen package guide](tools/dynwinrt-codegen/npm/README.md)
- [Python codegen package guide](tools/dynwinrt-codegen/python/README.md)
- [Python runtime guide](bindings/py/README.md)

### WinUI `Application + Window`

When `Microsoft.UI.Xaml.Application` is selected, codegen emits helpers for
WinUI metadata and `XamlControlsResources`. The application remains responsible
for package identity, framework bootstrap, UI-thread ownership, and lifecycle.
For JavaScript:

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

Python uses `Application.start()` inside `RoApartment(0)` and
`projected_lifetime_scope()`. See the
[Python WinUI hello-world sample](samples/python/winui-hello-world/) and
[Python-defined WinUI control sample](samples/python/winui-tic-tac-toe/).

In an unpackaged process,
set `WINAPPSDK_BOOTSTRAP_DLL_PATH` to the architecture-matched
`Microsoft.WindowsAppRuntime.Bootstrap.dll` before calling JavaScript
`initWinappsdk()` or Python `init_winappsdk()`.
`Application.create()` resolves the bootstrapped framework resources and
configures its UI thread for Per-Monitor V2 DPI awareness. Packaged processes
can omit the bootstrap call.

## Classic COM (Preview)

Classic COM support is functional and tested, but remains a **preview under
active development**. It targets a conservatively validated subset of
`IUnknown`- and `IInspectable`-rooted interfaces from `Windows.Win32.winmd`; it
is not a general Automation or native Win32 projection, and it does not project
flat DLL exports.

The current CI baseline against
`Microsoft.Windows.SDK.Win32Metadata` 71.0.14-preview is **5,567 of 7,929
eligible interfaces (70.21%)** with complete safe code generation. Supported
contracts include generated coclass activation and QueryInterface views,
managed interface ownership, native POD layouts, typed counted buffers,
BSTR/HSTRING, validated VARIANT, SAFEARRAY and PROPVARIANT subsets, and
synchronous JavaScript implementations of fully supported callback interfaces.
Seventeen stock-Windows Node E2E runners exercise representative Shell,
Automation, stream, callback, HWND, and WinRT interop scenarios.

Safety takes priority over coverage. If metadata does not fully describe an
interface's ABI, layout, ownership, allocator, or cleanup contract, generation
fails before emitting a partial wrapper. Material gaps still include several
common graphics, audio, WMI, clipboard/drag-and-drop, derived Automation, union,
BYREF/InOut, and output-ownership shapes.

Classic COM generation currently emits JavaScript and TypeScript only. It uses
the separate `@microsoft/dynwinrt/com` public surface; generated wrappers call
`@microsoft/dynwinrt/com/unsafe` internally after codegen validates the ABI.
Generated COM modules use the same lowercase/kebab namespace layout as WinRT
under `com/`, while the COM barrel keeps globally unique short exports. The
`@microsoft/dynwinrt` package root remains WinRT-only.

- [Classic COM JavaScript usage guide](docs/guides/windows/classic-com-usage.md)
- [Supported ABI, coverage, limitations, and ownership model](docs/architecture/classic-com-support.md)

## Repository layout

```
dynwinrt/
├── .github/
│   └── workflows/            # CI, coverage, and Python wheel assembly
├── .pipelines/               # Official 1ES npm, PyPI, and GitHub release pipeline
├── crates/dynwinrt/          # Shared WinRT + Classic COM ABI/libffi runtime
├── bindings/
│   ├── js/                   # @microsoft/dynwinrt npm runtime (N-API)
│   └── py/                   # dynwinrt PyPI runtime (PyO3)
├── tools/
│   └── dynwinrt-codegen/     # npm + PyPI WinRT/Classic COM codegen CLI
├── tests/
│   └── e2e/                  # JavaScript, Python, and Classic COM E2E suites
├── benchmarks/
│   ├── electron/             # Electron IPC benchmark app
│   └── js/                   # Dynamic and static JS/native benchmarks
├── samples/
│   ├── js/                   # JavaScript/TypeScript samples
│   └── python/               # Python samples
├── docs/                     # Architecture, benchmark, guide, and status docs
└── eng/
    ├── coverage/             # Mixed Rust/JavaScript/Python coverage tooling
    └── release/python/       # Python release preparation and verification
```

## Build from source

```bash
# Core library
cargo build -p dynwinrt
cargo test  -p dynwinrt

# JS bindings (napi-rs)
cd bindings/js && npm install && npm run build

# Python bindings (PyO3 + maturin)
cd bindings/py && python -m maturin develop && python -m pytest

# Codegen tool
cargo build -p dynwinrt-codegen --release
cargo run   -p dynwinrt-codegen -- generate --namespace Windows.Foundation --class-name Uri --output ./generated
```

Python [`dynwinrt`](https://pypi.org/project/dynwinrt/) runtime wheels target
CPython 3.11–3.14 on Windows x64 and ARM64. The standalone
[`dynwinrt-codegen`](https://pypi.org/project/dynwinrt-codegen/) wheel runs on
Python 3.8–3.14 and needs no Rust installation at consumption time.

Python runtime, codegen, packaging, and WinUI readiness are tracked in
[`docs/status/PYTHON_CHECKLIST.md`](docs/status/PYTHON_CHECKLIST.md).

Python samples cover
[files, OCR, cryptography, devices, AppLifecycle, text-to-speech, app
notifications, WinUI, and custom WinMD generation](samples/python/README.md).

Electron samples include:

- [Windows Hello](samples/js/windows-hello/README.md) — WinRT async APIs and
  HWND-bound Classic COM interop.
- [Share UI](samples/js/electron-share-ui/README.md) — typed WinRT and
  `IDataTransferManagerInterop`.
- [System Media Controls](samples/js/electron-smtc/README.md) — interactive,
  end-to-end SMTC publication and GSMTC loopback control with playlist,
  artwork, live timeline, session discovery, and automated validation.

For deployment, see
[Package a dynwinrt Node.js application as MSIX](docs/guides/windows/msix-packaging.md).

## Codegen CLI reference

| Argument | Required | Description |
|---|---|---|
| `--winmd PATH[;PATH...]` | No | Path to `.winmd` file(s) (auto-detects Windows SDK if omitted) |
| `--winmd-list FILE` | No | Newline-separated `.winmd` paths to emit |
| `--folder PATH` | No | Directory containing `.winmd` files |
| `--namespace NAMESPACE` | No | WinRT namespace to generate (omit for all non-`Windows.*` namespaces) |
| `--class-name NAME[,NAME...]` | No | Specific classes or public interfaces; dependencies are resolved transitively |
| `--ref PATH[;PATH...]` | No | Additional `.winmd` files for type resolution only (no code emitted) |
| `--ref-list FILE` | No | Newline-separated reference metadata paths |
| `--lang LANG` | No | `js` (default, emits `.js` + `.d.ts`) or `py` (emits `.py` + `.pyi` and `py.typed`) |
| `--import-name NAME` | No | JavaScript runtime import name (default `@microsoft/dynwinrt`) |
| `--pyi` | No | Explicitly request the default Python type stubs |
| `--no-pyi` | No | With `--lang py`, emit implementation files without type stubs |
| `--output DIR` | No | Output directory (default `./generated`) |
| `--dry-run` | No | Validate input, don't write files |

For the complete language-specific behavior and examples, see the
[JavaScript/TypeScript](tools/dynwinrt-codegen/npm/README.md) and
[Python](tools/dynwinrt-codegen/python/README.md) codegen package guides.

## JavaScript local development — fix generated imports

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
| Python bindings won't build | Requires CPython 3.11–3.14 and `maturin` (`python -m pip install maturin`) |
| Codegen snapshot tests fail after an intentional change | Set `DYNWINRT_UPDATE_SNAPSHOTS=1` for JavaScript snapshots or `DYNWINRT_UPDATE_PY_SNAPSHOTS=1` for Python snapshots, then rerun the affected test |

## Contributing

This project welcomes contributions and suggestions. Most contributions require you to agree to a Contributor License Agreement (CLA) declaring that you have the right to, and actually do, grant us the rights to use your contribution. For details, visit <https://cla.opensource.microsoft.com>.

When you submit a pull request, a CLA bot will automatically determine whether you need to provide a CLA and decorate the PR appropriately (e.g., status check, comment). Simply follow the instructions provided by the bot. You will only need to do this once across all repos using our CLA.

This project has adopted the [Microsoft Open Source Code of Conduct](https://opensource.microsoft.com/codeofconduct/). For more information see the [Code of Conduct FAQ](https://opensource.microsoft.com/codeofconduct/faq/) or contact <opencode@microsoft.com> with any additional questions or comments.

## Trademarks

This project may contain trademarks or logos for projects, products, or services. Authorized use of Microsoft trademarks or logos is subject to and must follow [Microsoft's Trademark & Brand Guidelines](https://www.microsoft.com/en-us/legal/intellectualproperty/trademarks/usage/general). Use of Microsoft trademarks or logos in modified versions of this project must not cause confusion or imply Microsoft sponsorship. Any use of third-party trademarks or logos are subject to those third-party's policies.

## License

This project is licensed under the [MIT License](LICENSE).
