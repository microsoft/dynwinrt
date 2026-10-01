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

with RoApartment(), projected_lifetime_scope():
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

Interface protocols describe native **instances**, including their canonical
identity and methods, not wrapper factories. For example, `StorageFile` can be
passed to an `IStorageFile` parameter without defining `from_value`.
`IStorageFile.from_value(raw)` and `value.as_interface(IStorageFile)` remain
available for explicit, IID-checked projection. Inherited `from_value` factories
retain the receiving interface subclass: `TaggedBuffer.from_value(raw)` and
`value.as_interface(TaggedBuffer)` return `TaggedBuffer`, not `IBuffer`.
Independent static factories such as `IBuffer.from_bytes` keep their declared
base-interface result.

WinRT `Object` inputs accept a `DynWinRTValue` or a projected native wrapper
whose `_obj` is a `DynWinRTValue`, including interface views and runtime-class
`Like` views. They do not implicitly box arbitrary Python objects, strings, or
`None`; use an explicitly boxed or null `DynWinRTValue` instead. `Object`
outputs remain `DynWinRTValue | None`. Context managers retain the entered
instance's type (`Self`), including derived wrappers passed through a base
`Like` protocol, and still close the object without suppressing exceptions.

Collection subscripts use the input contract for keys and values: for example,
`properties["uri"] = uri` accepts a generated `Uri`, while reading the item still
returns `DynWinRTValue | None`. Sequence item assignment, slice assignment, and
`insert` likewise accept projected inputs without changing their read types;
integer indices take one item and slices take an iterable of items. Existing
nullable `collections.abc` contracts remain unchanged. To pass a native null
reference, use `DynWinRTValue.null_value()`, not implicit `None` boxing.

The output directory belongs to codegen; do not store handwritten files in it.
After changing metadata files, SDK versions, or reference inputs, regenerate the
complete output. Regenerate with the updated generator to pick up these
consumer typing contracts, and use the matching runtime package and its stubs.
No runtime API change or consumer cast is required.

### Closed-generic stub migration

Python stubs now identify closed generic interfaces by their complete semantic
identity instead of a local projection name. This prevents different type
arguments from becoming interchangeable when their short names collide.

Stubs generated before this fix and regenerated stubs are not type-compatible
for the same closed interface in either direction. This is a typing migration:
the generated `.py` runtime code and native interface IIDs are unchanged.

Regenerate the **complete output of every generated Python package exchanging
these interfaces** with the fixed generator, including runtime classes that
implement or require them, then rebuild/reinstall the affected packages.
Use a fresh codegen-owned output directory with all original type selections
and metadata/reference inputs. An incremental append that retains old `.pyi`
declarations is not sufficient. Do not copy individual marker declarations or
add legacy fallbacks: they can restore the incorrect cross-interface acceptance.
The new markers agree across independently regenerated packages for the same
closed identity, even when their local projection names differ.

## Platform and limitations

- The standalone generator has `py3-none-win_amd64` and
  `py3-none-win_arm64` wheels for Python 3.8–3.14.
- Generated bindings and the `dynwinrt` runtime require CPython 3.11–3.14 on
  Windows x64 or ARM64.
- Python generation currently supports WinRT metadata. Classic COM and flat
  Win32 DLL-export generation from `Windows.Win32.winmd` are available only
  for JavaScript and TypeScript.
- Some APIs require their Windows component, package identity, or framework
  bootstrap to be present at runtime.

Python module components longer than 120 characters are shortened with a stable
readable prefix and hash suffix while public type names remain unchanged.

### Windows path length

A short module name does not guarantee a short **absolute** path. In deep
checkouts, codegen can successfully write files that ordinary Python imports,
mypy, or pip cannot open. CPython supports long paths on appropriately
configured Windows; see the [Python Windows guide](https://docs.python.org/3.11/using/windows.html#removing-the-max-path-limitation)
and [Windows long-path requirements](https://learn.microsoft.com/en-us/windows/win32/fileio/maximum-file-path-limitation).
Extended `\\?\` paths used by the generator do not automatically make downstream
tools long-path compatible; the Windows `LongPathsEnabled` setting alone does
not prove every consumer supports them.

On Windows, the generator emits one stderr warning before publishing output
when its longest final `.py`/`.pyi` path reaches 260 UTF-16 code units (the
legacy limit allows 259, excluding the terminating NUL). The warning names
that file, measures its path without the extended prefix, and states how much
shorter the output root needs to be. Relative `--output` paths are resolved
from the current directory; temporary transactional staging paths are not
counted. `--dry-run` checks the requested projection's planned source paths,
aggregating all selected namespaces into one warning; `--no-pyi` excludes
stubs. This is a **compatibility risk diagnostic**:
generation still succeeds, and module names, imports, layout, and bytes are
unchanged.

Prefer a shorter checkout such as `C:\src\dynwinrt`, or generate directly into
a short root and run consumers there:

```powershell
dynwinrt-codegen generate --namespace Windows.Foundation --class-name Uri `
  --lang py --output C:\g\generated_uri
Set-Location C:\g
python -c "from generated_uri.windows.foundation import Uri; print(Uri)"
python -m mypy --strict .\consumer.py
python -m pip install .\generated_uri
```

Here `consumer.py` is your application or typing fixture beside the generated
package. Leave additional headroom for pip's build staging and the destination
venv/site-packages path. Those paths cannot all be predicted from `--output`;
no warning is not a guarantee that a later build or deep-venv install will work.
Keep the venv short as well, for example `C:\g\venv`.

For an existing deep source tree, an optional, user-owned short junction can
expose the **same generated bytes** without renaming modules or copying code:

```powershell
New-Item -ItemType Directory -Path C:\g -Force | Out-Null
New-Item -ItemType Junction -Path C:\g\project -Target C:\src\deep\project
Set-Location C:\g\project
dynwinrt-codegen generate --namespace Windows.Foundation --class-name Uri `
  --lang py --output .\generated
python -c "from generated.windows.foundation import Uri; print(Uri)"
python -m mypy --strict .\consumer.py
python -m pip install .\generated
Set-Location C:\g
Remove-Item -LiteralPath C:\g\project
```

Link the parent checkout, not the generated directory itself: transactional
generation rejects a linked output directory. Use the **same shortened path**
for generation, imports, mypy, and package builds; returning to the deep path
reintroduces its limits. Remove only the junction you created, without
recursively deleting its target.

Alternatively, map an unused drive for the session with
`subst.exe R: C:\src\deep\project`, use `R:\` consistently for these commands,
and remove only that mapping with `subst.exe R: /D` after leaving the drive.
These short-path remedies have been validated with Python imports, strict
mypy, and generated-package installation on Windows with long paths disabled;
they do not automatically fix arbitrary deep build or install destinations.

## Links

- [`dynwinrt` runtime on PyPI](https://pypi.org/project/dynwinrt/)
- [Python runtime documentation](https://github.com/microsoft/dynwinrt/blob/main/bindings/py/README.md)
- [Source and issue tracker](https://github.com/microsoft/dynwinrt)

## License

[MIT](https://github.com/microsoft/dynwinrt/blob/main/LICENSE)
