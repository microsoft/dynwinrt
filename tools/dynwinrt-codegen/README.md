# dynwinrt-codegen

Read Windows metadata (`.winmd`) files and generate typed bindings that use `@microsoft/dynwinrt` at runtime.

## Install

```bash
npm install -D @microsoft/dynwinrt-codegen
```

Python users can install the standalone native command:

```powershell
python -m pip install dynwinrt-codegen
dynwinrt-codegen generate --namespace Windows.Foundation --class-name Uri `
  --lang py --output generated_uri
```

The Python distribution contains only the executable. Its wheels are
`py3-none-win_amd64` and `py3-none-win_arm64`, work with CPython 3.8–3.14, and
do not install or invoke Cargo/Rust. Generated Python packages require CPython
3.11–3.14 and pin `dynwinrt` to the exact codegen version.

## Usage

```bash
npx dynwinrt-codegen generate [OPTIONS]
```

### Arguments

| Argument | Required | Description |
|---|---|---|
| `--winmd` | No | Path to `.winmd` file(s), separated by `;`. Auto-detects Windows SDK if omitted |
| `--folder` | No | Directory containing `.winmd` files |
| `--namespace` | No | Generate only this namespace. If omitted, generates all non-Windows namespaces |
| `--class-name` | No | Class name(s) to generate, comma-separated (requires `--namespace`). E.g. `StorageFile` or `StorageFile,StorageFolder` |
| `--ref` | No | Additional `.winmd` files for type resolution only (no code generated). Paths separated by `;` |
| `--lang` | No | Target language: `js` (default, emits `.js` + `.d.ts`) or `py` (emits `.py`, optionally `.pyi`) |
| `--output` | No | Codegen-owned output directory (default: `./generated`). Existing contents may be replaced or removed; do not store handwritten files here. |
| `--dry-run` | No | Validate metadata and resolve dependencies without writing files |

With `--lang js` (default), the tool emits plain ESM JavaScript (`.js`) plus matching ambient TypeScript declarations (`.d.ts`). No TypeScript compiler is needed — the output works for both JS and TS consumers. JSDoc comments are preserved so VS Code IntelliSense shows API descriptions.

Repeated commands may append bindings to the same output directory when they
use the same restored metadata version. After changing WinMD files, SDK
versions, or reference inputs, delete the codegen-owned output directory and
run all generation commands again.

> **Note:** Legacy flags `--lang ts`, `--lang cjs`, `--source-map`, `--declaration`, and `--no-declaration` are accepted by the npm CLI wrapper for backwards compatibility but are silently mapped to `--lang js` behavior.

### Examples

Generate JavaScript (ESM) bindings from a WinAppSDK metadata folder:

```bash
npx dynwinrt-codegen generate \
  --folder path/to/metadata \
  --output ./generated-js \
  --lang js
```

Generate bindings for a specific class (emits `.js` + `.d.ts`):

```bash
npx dynwinrt-codegen generate \
  --namespace Windows.Storage \
  --class-name StorageFile \
  --output ./generated
```

Generate multiple classes in one pass (shares the winmd index):

```bash
npx dynwinrt-codegen generate \
  --namespace Windows.Storage \
  --class-name StorageFile,StorageFolder \
  --output ./generated
```

Generate all namespaces from multiple `.winmd` files:

```bash
npx dynwinrt-codegen generate \
  --winmd "path/to/Windows.winmd;path/to/Microsoft.WindowsAppSDK.winmd" \
  --output ./generated
```

Validate metadata without writing files:

```bash
npx dynwinrt-codegen generate \
  --folder path/to/metadata \
  --dry-run
```

## Output

For each WinRT class, the tool generates:

- **Interface registration** -- `DynWinRtType.registerInterface()` with all methods and type signatures
- **Wrapper class** -- typed class with properties and methods
- **Constructors** -- unambiguous public WinRT activations projected as idiomatic JavaScript constructors
- **Factory methods** -- original static activation methods retained for compatibility
- **Enums** -- enum declarations
- **Collection types** -- `IVector<T>`, `IVectorView<T>`, `IMap<K,V>`, etc.
- **Index file** -- re-exporting all generated types

Dependencies are resolved automatically -- specifying `--class StorageFile` will also generate referenced types like `Uri`, enums, and interfaces.

## Build from Source

```bash
cargo build -p dynwinrt-codegen --release
```

The compiled executable needs to be copied into the npm package before publishing:

```bash
# x64
cargo build -p dynwinrt-codegen --release
cp target/release/dynwinrt-codegen.exe tools/dynwinrt-codegen/npm/bin/x64/

# arm64
cargo build -p dynwinrt-codegen --release --target aarch64-pc-windows-msvc
cp target/aarch64-pc-windows-msvc/release/dynwinrt-codegen.exe tools/dynwinrt-codegen/npm/bin/arm64/
```

Then publish:

```bash
cd tools/dynwinrt-codegen/npm
npm publish
```

In CI, this is handled automatically by the build workflow.

## Testing

```bash
cargo test -p dynwinrt-codegen
```

Tests include:
- Unit tests for type mapping, dependency resolution, and code generation helpers
- Snapshot test for `Windows.Foundation.Uri` (regenerate with `cargo run -p dynwinrt-codegen -- generate --namespace Windows.Foundation --class-name Uri --lang js --output tests/snapshots/uri`)
