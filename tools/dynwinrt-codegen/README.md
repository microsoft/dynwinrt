# dynwinrt-codegen

`dynwinrt-codegen` reads Windows metadata (`.winmd`) and generates typed
bindings for [dynwinrt](https://github.com/microsoft/dynwinrt):

- WinRT JavaScript (`.js`) with TypeScript declarations (`.d.ts`)
- WinRT Python (`.py`) with type stubs (`.pyi`) and a `py.typed` marker
- Supported Classic COM APIs from `Windows.Win32.winmd` as JavaScript and
  TypeScript

The command is available for Windows x64 and ARM64. Generated JavaScript uses
`@microsoft/dynwinrt`; generated Python uses `dynwinrt`.

## Install

### Python

```powershell
python -m pip install --pre dynwinrt-codegen
dynwinrt-codegen generate --namespace Windows.Foundation --class-name Uri `
  --lang py --output .\generated
```

The Python distribution is a standalone native command. Its
`py3-none-win_amd64` and `py3-none-win_arm64` wheels require Python 3.8–3.14
but do not require Cargo or Rust. Generated Python package manifests require
CPython 3.11–3.14 and pin `dynwinrt` to the generator's exact version.

### npm

```powershell
npm install @microsoft/dynwinrt
npm install --save-dev @microsoft/dynwinrt-codegen
npx dynwinrt-codegen generate --namespace Windows.Foundation --class-name Uri `
  --output .\generated
```

## Generate

```text
dynwinrt-codegen generate [OPTIONS]
```

Use `npx dynwinrt-codegen` instead of `dynwinrt-codegen` when running the npm
package.

| Option | Description |
|---|---|
| `--winmd PATH[;PATH...]` | Metadata file paths. Sibling `.winmd` files are discovered automatically. The Windows SDK is auto-detected when no input supplies `Windows.*` metadata. |
| `--winmd-list FILE` | Newline-separated metadata paths to emit; blank lines and `#` comments are ignored. |
| `--folder DIR` | Load every `.winmd` file directly inside a directory. |
| `--namespace NS` | Generate one namespace. Without it, generate all non-`Windows.*` namespaces in the input. |
| `--class-name NAME[,NAME...]` | Generate specific classes or public interfaces. Use fully qualified names, or unqualified names together with `--namespace`. |
| `--ref PATH[;PATH...]` | Metadata used only for type resolution. Sibling discovery is disabled for references. |
| `--ref-list FILE` | Newline-separated reference metadata paths; blank lines and `#` comments are ignored. |
| `--lang js\|py` | `js` emits CommonJS `.js`, an ESM facade, and `.d.ts` files (default); `py` emits `.py`, `.pyi`, and `py.typed`. |
| `--output DIR` | Dedicated codegen-owned output directory (default `./generated`). Existing contents may be replaced or removed. |
| `--import-name NAME` | Runtime package imported by generated JavaScript (default `@microsoft/dynwinrt`). |
| `--dry-run` | Validate metadata, dependencies, ABI contracts, and layout without writing files. |
| `--pyi` | With `--lang py`, explicitly request the default type stubs; retained for compatibility. |
| `--no-pyi` | With `--lang py`, omit `.pyi` files and `py.typed`. |

### Examples

Generate two Windows SDK classes as JavaScript and TypeScript:

```powershell
dynwinrt-codegen generate `
  --namespace Windows.Storage `
  --class-name StorageFile,StorageFolder `
  --output .\generated
```

Generate Python from a restored metadata folder:

```powershell
dynwinrt-codegen generate `
  --folder C:\path\to\metadata `
  --lang py `
  --output .\generated-python
```

Load emitted metadata and reference metadata from list files:

```powershell
dynwinrt-codegen generate `
  --winmd-list .\winmd-inputs.txt `
  --ref-list .\winmd-references.txt `
  --output .\generated
```

Validate a generation request without changing the output directory:

```powershell
dynwinrt-codegen generate `
  --folder C:\path\to\metadata `
  --dry-run
```

### Flat Win32 contract slice

The initial flat DLL-export projection is JavaScript-only and supports
`GetTickCount`, `GetTickCount64`, `RegOpenKeyExW`, `RegQueryValueExW`, and
`RegCloseKey` from the pinned `Microsoft.Windows.SDK.Win32Metadata`
`71.0.14-preview` package:

```powershell
dynwinrt-codegen generate `
  --winmd $env:DYNWINRT_WIN32_WINMD `
  --class-name "Windows.Win32.System.SystemInformation.Apis,Windows.Win32.System.Registry.Apis" `
  --output .\generated
```

`--namespace Windows.Win32.System.Registry` is equivalent to selecting its
`Apis` container. Other exports are explicitly omitted; this is not complete
namespace support. Generated modules live under
`win32/windows/win32/system/registry/` and
`win32/windows/win32/system/system-information/`, outside the WinRT root.
The runtime package must include the matching `win32` entrypoint.
See the [contract architecture and migration scope](../../docs/architecture/flat-win32-contracts.md).

### Other commands

`dynwinrt-codegen capabilities` prints the command's supported features, one
machine-readable value per line.

`dynwinrt-codegen com-census --winmd <PATH> [--json]` measures how many eligible
interfaces in `Windows.Win32.winmd` have complete safe Classic COM generation.

## Generated output

The generator resolves transitive dependencies and emits namespace index files.
WinRT output includes typed classes and interfaces, public activation
constructors, static factory methods, enums, structs, delegates, async
operations, and generic collections.

JavaScript implementation modules are CommonJS, with `index.mjs` facades for
ESM consumers, and need no TypeScript compilation step. Python output uses
snake_case names and includes type information by default. Documentation from
sibling XML files is included when available.

Classic COM generation is available only with `--lang js`. It is isolated in a
`com` subpackage and fails closed when metadata does not provide enough ABI,
layout, ownership, or cleanup information. See the
[Classic COM usage guide](https://github.com/microsoft/dynwinrt/blob/main/docs/guides/windows/classic-com-usage.md).

The output directory belongs to codegen; do not store handwritten files in it.
After changing metadata files, SDK versions, or reference inputs, regenerate the
complete output. Python module components longer than 120 characters are
shortened with a stable readable prefix and hash suffix while public type names
remain unchanged.

Implementation helper names are allocated alongside metadata types and follow
their owning interfaces through incremental renames. Python's package-level
root exports drop an interface and its helpers when its short name becomes
ambiguous, while namespace imports remain available. Python's
heterogeneous interface-pair union is generated from validated inventory
records, not inferred from `.pyi` text. The first typed incremental generation
over an older inventory needs the original WinMD/`--ref` inputs for interfaces
whose implementation records are missing. If those inputs are unavailable,
generation fails atomically and leaves the previous output usable: supply the
listed metadata and retry, or fully regenerate the package. New inventories
retain these records, and `--no-pyi` does not require this typing migration.
After `--no-pyi`, include the earlier types when regenerating with stubs enabled
(or fully regenerate the package). A typed append fails rather than publishing
imports of missing retained declarations or dropping interfaces from the union.

The npm wrapper accepts the legacy `--source-map`, `--declaration`, and
`--no-declaration` flags as no-ops. The Rust command accepts only `js` and `py`
for `--lang`.

## Build and test from source

From the repository root:

```powershell
cargo build -p dynwinrt-codegen --release
cargo test -p dynwinrt-codegen
```

Official npm and PyPI packages are built and published by the repository release
pipelines.

## License

[MIT](https://github.com/microsoft/dynwinrt/blob/main/LICENSE)
