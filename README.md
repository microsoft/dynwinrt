# dynwinrt

Dynamic WinRT API invocation — call any Windows Runtime method at runtime without native code generation.

## Overview

`dynwinrt` is a Rust library that uses runtime metadata (.winmd files) and FFI (libffi) to call arbitrary WinRT methods dynamically. It provides a foundation for JavaScript and Python bindings that don't require MSVC compilation or version-specific generated code.

### Scope

`dynwinrt` is designed for **non-UI WinRT APIs** — data, storage, networking, globalization, cryptography, sensors, connectivity, AI, and similar headless services from the Windows SDK and WinAppSDK. It is **not intended for XAML / WinUI scenarios**. Supporting XAML / WinUI would require host-framework integration and composable-class aggregation patterns that `dynwinrt` does not currently implement. In practice, generating bindings for `Microsoft.UI.Xaml.*` / `Windows.UI.Xaml.*` namespaces produces invalid wrappers for composable constructors, so those namespaces should be treated as out of scope for this project. See `TODO.md` for details.

## Repository Structure

```
dynwinrt/
├── crates/dynwinrt/       # Core Rust library
├── bindings/
│   ├── js/                # JavaScript/TypeScript bindings (napi-rs)
│   └── py/                # Python bindings (PyO3)
└── tools/
    └── winrt-meta/        # Source for dynwinrt-codegen (TypeScript & Python)
```

## Build

```bash
# Build the core library
cargo build -p dynwinrt

# Run tests
cargo test -p dynwinrt

# Build JS bindings
cd bindings/js && npm install && npx napi build --no-const-enum --platform --release -o dist

# Build Python bindings
cd bindings/py && maturin develop
```

## Code Generation with dynwinrt-codegen

`dynwinrt-codegen` reads Windows metadata (.winmd) files and generates typed bindings for `@microsoft/dynwinrt` (TypeScript) or `dynwinrt-py` (Python).

### Quick Start

```bash
# Install from npm (once published)
npm install -D @microsoft/dynwinrt-codegen

# Generate TypeScript bindings for a class
npx dynwinrt-codegen generate --namespace Windows.Foundation --class-name Uri --lang ts --output ./generated

# Generate Python bindings for a class
npx dynwinrt-codegen generate --namespace Windows.Foundation --class-name Uri --lang py --output ./generated

# Generate an entire namespace
npx dynwinrt-codegen generate --namespace Windows.Web.Http --lang ts --output ./generated
```

### Building from Source

```bash
cd tools/dynwinrt-codegen
cargo build -p dynwinrt-codegen --release
cargo run -p dynwinrt-codegen --release -- generate --namespace Windows.Foundation --class-name Uri --lang ts --output ./generated
```

**Arguments:**

| Argument | Required | Description |
|---|---|---|
| `--winmd` | No | Path to .winmd file(s), separated by `;` (auto-detects Windows SDK) |
| `--folder` | No | Directory containing .winmd files |
| `--namespace` | No | WinRT namespace to generate (omit to generate all non-Windows namespaces) |
| `--class-name` | No | Specific class (generates dependencies too) |
| `--ref` | No | Additional .winmd files for type resolution only (no code generated) |
| `--lang` | No | Target language: `ts` (default) or `py` |
| `--output` | No | Output directory (default: `./generated`) |
| `--dry-run` | No | Validate without writing files |

### Fix Import Paths (local development)

Generated TypeScript files import from `'@microsoft/dynwinrt'`. For local development, fix to relative path:

```bash
find generated -name "*.ts" -exec sed -i "s|from '@microsoft/dynwinrt'|from '../../dist/index.js'|g" {} +
```

### Use Generated Bindings

**TypeScript:**

```typescript
import { roInitialize } from '@microsoft/dynwinrt'
import { Uri } from './generated/Uri'

roInitialize(1) // Initialize WinRT (MTA)

const uri = Uri.createUri('https://example.com/path?q=1')
console.log(uri.host)       // "example.com"
console.log(uri.port)       // 443
console.log(uri.schemeName) // "https"
```

**Python:**

```python
import dynwinrt_py as dw
from generated.uri import Uri

dw.ro_initialize(1)  # Initialize WinRT (MTA)

uri = Uri.create_uri('https://example.com/path?q=1')
print(uri.host)         # "example.com"
print(uri.port)         # 443
print(uri.scheme_name)  # "https"
```

### What Gets Generated

For each WinRT class, dynwinrt-codegen generates:

- **Interface registration** — `DynWinRtType.registerInterface()` with all methods and type signatures
- **Wrapper class** — Typed class with properties and methods (TypeScript or Python)
- **Factory methods** — Static methods for object creation (via activation factory)
- **Collection types** — `IVector<T>`, `IMap<K,V>` wrappers
- **Structs** — Value types with pack/unpack helpers
- **Enums** — TypeScript `enum` or Python `IntEnum` declarations
- **Delegates** — IID and parameter type exports for event handling
- **Index file** — `index.ts` or `__init__.py` re-exporting all types

### Running Tests

```bash
# Core library tests
cargo test -p dynwinrt

# JS binding tests
cd bindings/js && npx tsx __test__/index.spec.ts

# Python binding tests
cd bindings/py && pytest
```

## Use WinAppSDK Bootstrap

The path to the WinAppSDK Bootstrap DLL is retrieved from the `WINAPPSDK_BOOTSTRAP_DLL_PATH` environment variable. Only needed for unpackaged apps using WinAppSDK APIs.

```typescript
import { initWinappsdk } from '@microsoft/dynwinrt'
initWinappsdk(1, 8) // Initialize WinAppSDK 1.8
```

## Troubleshooting

| Problem | Solution |
|---------|----------|
| `cargo build` fails with libffi errors | Ensure you have a C compiler (MSVC) and Windows SDK installed |
| WinAppSDK APIs fail at runtime | Set `WINAPPSDK_BOOTSTRAP_DLL_PATH` environment variable |
| `cargo test -p dynwinrt` fails | Ensure Windows SDK is installed at default path with `Windows.winmd` |
| JS bindings won't build | Run `npm install` first; requires Node.js 18+ |
| Python bindings won't build | Requires Python 3.8+ and `maturin` (`pip install maturin`) |
| dynwinrt-codegen snapshot tests fail | Line-ending differences — run `cargo test -p dynwinrt-codegen -- --include-ignored` to regenerate |

## Contributing

This project welcomes contributions and suggestions. Most contributions require you to agree to a
Contributor License Agreement (CLA) declaring that you have the right to, and actually do, grant us
the rights to use your contribution. For details, visit https://cla.opensource.microsoft.com.

When you submit a pull request, a CLA bot will automatically determine whether you need to provide
a CLA and decorate the PR appropriately (e.g., status check, comment). Simply follow the instructions
provided by the bot. You will only need to do this once across all repos using our CLA.

This project has adopted the [Microsoft Open Source Code of Conduct](https://opensource.microsoft.com/codeofconduct/).
For more information see the [Code of Conduct FAQ](https://opensource.microsoft.com/codeofconduct/faq/) or
contact [opencode@microsoft.com](mailto:opencode@microsoft.com) with any additional questions or comments.

## Trademarks

This project may contain trademarks or logos for projects, products, or services. Authorized use of Microsoft
trademarks or logos is subject to and must follow
[Microsoft's Trademark & Brand Guidelines](https://www.microsoft.com/en-us/legal/intellectualproperty/trademarks/usage/general).
Use of Microsoft trademarks or logos in modified versions of this project must not cause confusion or imply Microsoft sponsorship.
Any use of third-party trademarks or logos are subject to those third-party's policies.

## License

This project is licensed under the [MIT License](LICENSE).

