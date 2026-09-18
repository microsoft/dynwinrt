# dynwinrt

**Typed Windows Runtime bindings for JavaScript and Python.**

[![@microsoft/dynwinrt](https://img.shields.io/npm/v/@microsoft/dynwinrt.svg?label=%40microsoft%2Fdynwinrt)](https://www.npmjs.com/package/@microsoft/dynwinrt)
[![@microsoft/dynwinrt-codegen](https://img.shields.io/npm/v/@microsoft/dynwinrt-codegen.svg?label=%40microsoft%2Fdynwinrt-codegen)](https://www.npmjs.com/package/@microsoft/dynwinrt-codegen)
[![dynwinrt on PyPI](https://img.shields.io/pypi/v/dynwinrt.svg?label=PyPI%20dynwinrt)](https://pypi.org/project/dynwinrt/)
[![dynwinrt-codegen on PyPI](https://img.shields.io/pypi/v/dynwinrt-codegen.svg?label=PyPI%20dynwinrt-codegen)](https://pypi.org/project/dynwinrt-codegen/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Use Windows Runtime (WinRT) APIs from Node.js, Electron, and Python without
writing a native extension for each API. `dynwinrt-codegen` generates typed
bindings from `.winmd` metadata, and `dynwinrt` provides the prebuilt native
runtime that invokes those APIs.

Codegen reads metadata from the Windows SDK, Windows App SDK, or custom WinRT
components ahead of time. The generated JavaScript or Python wrappers register
interface signatures with the shared Rust runtime. The runtime marshals values
and invokes native methods dynamically, using direct-call fast paths where
available and libffi for the general case. This avoids compiling a native
extension for each API; published prebuilt packages let you generate and use
bindings without Rust, MSBuild, or `node-gyp`.

## Features

WinRT bindings include constructors, properties, collections, structs, enums,
delegates, and events. JavaScript gets camelCase APIs, TypeScript declarations,
and Promise-based async operations; Python gets snake_case APIs, type stubs,
and asyncio-compatible operations. Both support async progress and cancellation.

| API surface | Languages | Scope |
| --- | --- | --- |
| **WinRT (preview)** | JS/TS, Python | Typed bindings for Windows SDK, Windows App SDK, and custom WinRT APIs. |
| **Classic COM (experimental)** | JS/TS | A validated subset of interfaces from `Windows.Win32.winmd`, through `@microsoft/dynwinrt/com`. |
| **Flat Win32 APIs (experimental)** | JS/TS | Partial support for native functions and associated types from `Windows.Win32.winmd`, through `@microsoft/dynwinrt/win32`. |

WinUI 3 application and window hosting is **experimental** and requires
application-managed UI threads and lifecycle.

You can also implement supported WinRT interfaces in
JavaScript or Python and pass them to native consumers.

COM and Win32 are not complete projections of every Windows API: safe generation
requires validated ABI, layout, and ownership contracts. Win32 support is intended
for evaluation and prototyping, with no backward-compatibility guarantee yet.
See the [COM support guide](docs/architecture/classic-com-support.md) and
[Win32 support guide](docs/architecture/flat-win32-contracts.md) for current
coverage and limitations.

## Getting started

### Requirements

- **Windows 10 or 11, x64 or ARM64.** Available APIs depend on your Windows version.
- **Node.js 18+** for JavaScript/TypeScript, or **CPython 3.11-3.14** for Python.
- **Windows SDK metadata** for the examples below. Codegen auto-detects an
  installed Windows SDK; use `--winmd` to supply metadata explicitly.

The examples use a built-in Windows API and do not require Windows App SDK,
package identity, or an AI model. Additional runtime packages, permissions, or
hardware may be needed for other APIs.

Run the commands below in PowerShell from your project directory. Keep generated
output in its own directory: codegen manages that directory's contents.

### JavaScript / TypeScript

Install the runtime and generator, then generate a binding for
`Windows.Foundation.Uri`:

```powershell
npm install @microsoft/dynwinrt
npm install -D @microsoft/dynwinrt-codegen

npx dynwinrt-codegen generate `
  --namespace Windows.Foundation `
  --class-name Uri `
  --output .\generated
```

Save this as `example.cjs`:

```js
const { roInitialize } = require('@microsoft/dynwinrt');
const { Uri } = require('./generated');
const { releaseProjected } = require('./generated/lifetime');

roInitialize(1); // Initialize WinRT on this thread (MTA).
const uri = new Uri('https://example.com/path?q=1');
try {
  console.log(uri.host); // "example.com"
} finally {
  releaseProjected(uri);
}
```

```powershell
node .\example.cjs
```

The generated `.js` files run directly; accompanying `.d.ts` files provide
IntelliSense and TypeScript type checking.

### Python

Install the generator in your Python environment, generate a package, and
install it. The generated package installs its matching `dynwinrt` runtime:

```powershell
python -m pip install --pre dynwinrt-codegen

dynwinrt-codegen generate `
  --namespace Windows.Foundation `
  --class-name Uri `
  --lang py `
  --output .\generated_uri

python -m pip install .\generated_uri
```

Save this as `example.py`:

```python
from dynwinrt import RoApartment, projected_lifetime_scope
from generated_uri.windows.foundation import Uri

with RoApartment(1), projected_lifetime_scope():
    uri = Uri("https://example.com/path?q=1")
    print(uri.host)  # "example.com"
```

```powershell
python .\example.py
```

`RoApartment(1)` initializes WinRT on the current thread. The lifetime scope
releases generated wrappers before the apartment closes.

## Examples

| Build something with | Example |
| --- | --- |
| WinUI 3 controls and events (experimental) | [JavaScript Tic-Tac-Toe](samples/js/winui-tic-tac-toe/README.md), [Python Hello World](samples/python/winui-hello-world/README.md) |
| Image OCR | [Windows AI in Node.js](samples/js/ocr/README.md), [Windows OCR in Python](samples/python/ocr-image/README.md) |
| Local AI inference | [Aion Instruct chat in Electron](samples/js/electron-aion-chat/README.md) |
| WinRT and Classic COM interop | [Windows Hello in Electron](samples/js/windows-hello/README.md) |
| Async file operations | [Python file I/O](samples/python/async-file-io/README.md) |
| Flat Win32 APIs (experimental) | [System information, Registry, and async file I/O](samples/js/win32/README.md) |

Each sample documents its Windows version, SDK, package identity, and hardware
requirements. Browse all [JavaScript/Electron samples](samples/js/README.md) or
[Python samples](samples/python/README.md) for setup instructions and more examples.

### Examples with WinApp CLI

WinApp CLI integrates with dynwinrt to restore SDK dependencies and generate
typed bindings. It also simplifies SDK package management, package identity
setup for local development, and MSIX packaging and signing. See its
[Electron JavaScript guides](https://github.com/microsoft/winappCli/blob/main/docs/guides/electron/index.md#2-call-windows-apis-from-javascript)
for notifications, file pickers, Phi Silica, and WinML provider integration.

## Documentation

| Task | Guide |
| --- | --- |
| Generate typed bindings | [CLI reference](tools/dynwinrt-codegen/README.md), [JavaScript/TypeScript](tools/dynwinrt-codegen/npm/README.md), [Python](tools/dynwinrt-codegen/python/README.md) |
| Use the runtime and manage object lifetimes | [JavaScript](bindings/js/README.md), [Python](bindings/py/README.md) |
| Implement WinRT interfaces in JavaScript or Python | [Interface implementations](docs/guides/windows/winrt-interface-implementations.md) |
| Use Classic COM APIs | [Usage guide](docs/guides/windows/classic-com-usage.md) |
| Use flat Win32 APIs | [Capabilities and contract boundaries](docs/architecture/flat-win32-contracts.md) |
| Configure package identity and deployment | [Node.js development](docs/guides/node/dev-mode.md), [MSIX packaging](docs/guides/windows/msix-packaging.md) |

## Development

The [core runtime](crates/dynwinrt/) is written in Rust, with
[Node-API bindings](bindings/js/) and [PyO3 bindings](bindings/py/).
The [code generator](tools/dynwinrt-codegen/) is shared by both languages.

To build the core and generator from source, install Rust, the MSVC C++ build
tools, and the Windows SDK, then run from the repository root:

```powershell
cargo build -p dynwinrt -p dynwinrt-codegen
cargo test -p dynwinrt -p dynwinrt-codegen
```

See [Development Setup](CONTRIBUTING.md#development-setup) for language-binding
builds, test suites, and contribution guidelines.

## Contributing

See the [Build CI guide](docs/guides/development/ci.md) for parallel validation,
artifact reuse, documentation-only PR checks, and running E2E with a prebuilt generator.

This project welcomes contributions and suggestions. Most contributions require you to agree to a Contributor License Agreement (CLA) declaring that you have the right to, and actually do, grant us the rights to use your contribution. For details, visit <https://cla.opensource.microsoft.com>.

When you submit a pull request, a CLA bot will automatically determine whether you need to provide a CLA and decorate the PR appropriately (e.g., status check, comment). Simply follow the instructions provided by the bot. You will only need to do this once across all repos using our CLA.

This project has adopted the [Microsoft Open Source Code of Conduct](https://opensource.microsoft.com/codeofconduct/). For more information see the [Code of Conduct FAQ](https://opensource.microsoft.com/codeofconduct/faq/) or contact <opencode@microsoft.com> with any additional questions or comments.

## Trademarks

This project may contain trademarks or logos for projects, products, or services. Authorized use of Microsoft trademarks or logos is subject to and must follow [Microsoft's Trademark & Brand Guidelines](https://www.microsoft.com/en-us/legal/intellectualproperty/trademarks/usage/general). Use of Microsoft trademarks or logos in modified versions of this project must not cause confusion or imply Microsoft sponsorship. Any use of third-party trademarks or logos are subject to those third-party's policies.

## License

This project is licensed under the [MIT License](LICENSE).
