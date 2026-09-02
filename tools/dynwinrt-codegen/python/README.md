# dynwinrt-codegen

**Generate typed Python bindings for Windows Runtime (WinRT) APIs from `.winmd`
metadata.**

`dynwinrt-codegen` reads the metadata shipped by the Windows SDK, WinAppSDK, and
other Windows components. It emits Python modules that use
[`dynwinrt`](https://pypi.org/project/dynwinrt/) to invoke those APIs at runtime.
The generated API uses Python naming, values, collections, type annotations, and
asyncio-compatible operations.

## Why use this?

Calling a WinRT API without an existing Python projection normally requires a
native extension or handwritten metadata, COM ABI, and marshaling code.
`dynwinrt-codegen` derives that information from `.winmd` files and generates:

- Python classes with snake_case properties and methods
- type-checked overloads and `.pyi` type stubs
- asyncio-compatible WinRT operations
- Python-native collections, GUIDs, dates, times, and byte arrays
- enums, structs, delegates, and event helpers
- a package manifest pinned to the matching `dynwinrt` runtime version

The generator is a standalone Windows executable. Installing or running it does
not require Cargo or Rust.

## Install and generate

```powershell
python -m pip install --pre dynwinrt-codegen

# Generate one Windows SDK class.
dynwinrt-codegen generate `
  --namespace Windows.Foundation `
  --class-name Uri `
  --lang py `
  --output .\generated_uri

# Install the generated package and its exact dynwinrt runtime dependency.
python -m pip install .\generated_uri
```

The generated package can then be imported normally:

```python
from dynwinrt import RoApartment, projected_lifetime_scope
from generated_uri.windows.foundation import Uri

with RoApartment(1), projected_lifetime_scope():
    uri = Uri("https://example.com/path")
    print(uri.host)
```

## CLI options

| Option | Description |
|---|---|
| `--winmd PATH[;PATH...]` | Metadata file paths. Sibling `.winmd` files are discovered automatically. The Windows SDK is auto-detected when no input supplies `Windows.*` metadata. |
| `--winmd-list FILE` | Newline-separated metadata paths to emit; blank lines and `#` comments are ignored. |
| `--folder DIR` | Load every `.winmd` file directly inside a directory. |
| `--namespace NS` | Generate one namespace. Without it, generate all non-`Windows.*` namespaces in the input. |
| `--class-name NAME[,NAME...]` | Generate specific classes or public interfaces. Use fully qualified names, or unqualified names together with `--namespace`. |
| `--ref PATH[;PATH...]` | Metadata used only for type resolution. Sibling discovery is disabled for references. |
| `--ref-list FILE` | Newline-separated reference metadata paths; blank lines and `#` comments are ignored. |
| `--output DIR` | Dedicated codegen-owned output directory (default `./generated`). Existing contents may be replaced or removed. |
| `--dry-run` | Validate metadata and dependencies without writing files. |
| `--pyi` | Explicitly request the default Python type stubs; retained for compatibility. |
| `--no-pyi` | Omit `.pyi` files and the `py.typed` marker. |

Use `--lang py` for every Python generation command. Run
`dynwinrt-codegen generate --help` for the complete command reference.

### More examples

Generate two classes from the Windows SDK:

```powershell
dynwinrt-codegen generate `
  --namespace Windows.Storage `
  --class-name StorageFile,StorageFolder `
  --lang py `
  --output .\storage_bindings
```

Generate all non-system namespaces from a restored metadata folder:

```powershell
dynwinrt-codegen generate `
  --folder C:\path\to\metadata `
  --lang py `
  --output .\component_bindings
```

Use explicit reference metadata for a reproducible generation:

```powershell
dynwinrt-codegen generate `
  --winmd-list .\winmd-inputs.txt `
  --ref-list .\winmd-references.txt `
  --lang py `
  --output .\component_bindings
```

Validate a request without changing its output directory:

```powershell
dynwinrt-codegen generate `
  --folder C:\path\to\metadata `
  --lang py `
  --output .\component_bindings `
  --dry-run
```

## Generated output

The output is an installable Python package. Its generated `pyproject.toml` pins
`dynwinrt` to the generator's exact version and requires CPython 3.11–3.14.
`.pyi` files and a `py.typed` marker are emitted by default.

Transitive metadata dependencies are resolved automatically. Namespace packages
and imports mirror the metadata hierarchy, while public members use Python
snake_case naming. XML documentation found beside the input metadata is
included when available.

Generated async methods return typed awaitable objects. WinRT collections
implement standard `collections.abc` protocols, flags use `enum.IntFlag`, and
compatible method inputs accept native Python sequences, mappings, `bytes`,
`bytearray`, `uuid.UUID`, `datetime.datetime`, and `datetime.timedelta`.

The output directory belongs to codegen; do not store handwritten files in it.
After changing metadata files, SDK versions, or reference inputs, regenerate the
complete output.

## Platform and limitations

- The standalone generator has `py3-none-win_amd64` and
  `py3-none-win_arm64` wheels for Python 3.8–3.14.
- Generated bindings and the `dynwinrt` runtime require CPython 3.11–3.14 on
  Windows x64 or ARM64.
- Python generation currently supports WinRT metadata. Classic COM generation
  from `Windows.Win32.winmd` is currently available only for JavaScript and
  TypeScript.
- Some APIs require their Windows component, package identity, or framework
  bootstrap to be present at runtime.

Python module components longer than 120 characters are shortened with a stable
readable prefix and hash suffix while public type names remain unchanged.

## Links

- [`dynwinrt` runtime on PyPI](https://pypi.org/project/dynwinrt/)
- [Python runtime documentation](https://github.com/microsoft/dynwinrt/blob/main/bindings/py/README.md)
- [Source and issue tracker](https://github.com/microsoft/dynwinrt)

## License

[MIT](https://github.com/microsoft/dynwinrt/blob/main/LICENSE)
