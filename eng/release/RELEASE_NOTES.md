## Prerelease v0.1.0-preview.24

This preview improves Python package installation, async notification handling, resource cleanup, and generated API contracts. It also adds packaged Windows App SDK Bootstrap configuration for Python and Node.js. These changes are relative to preview.23.

## Packages and installation

- `@microsoft/dynwinrt` - JavaScript/TypeScript runtime.
- `@microsoft/dynwinrt-codegen` - JavaScript/TypeScript code generator.
- `dynwinrt` - Native Python runtime.
- `dynwinrt-codegen` - Standalone Python code generator.

- **Runtime platforms:** Windows x64 and ARM64.
- **JavaScript version:** Node.js 18 or later.
- **Python version:** CPython 3.11-3.14 for the runtime and generated bindings.
- **API availability:** Depends on the Windows version, installed SDK components, package identity, and hardware.
- **Package versions:** Use the same runtime and codegen version for each language.
- **Upgrade cleanup:** Clean old generated bindings before using the new version.

Installation commands for this preview, once published:

```powershell
npm install @microsoft/dynwinrt@0.1.0-preview.24
npm install -D @microsoft/dynwinrt-codegen@0.1.0-preview.24
python -m pip install --pre "dynwinrt==0.1.0rc24" "dynwinrt-codegen==0.1.0rc24"
```

## Highlights

### Python src package layout

- Generate standard `src/<import_name>` packages so editable installations expose modules and type stubs to editors and static type checkers without custom import hooks. The outer `--output` directory remains the installable project; installed import names do not change. ([#228](https://github.com/microsoft/dynwinrt/pull/228))

### Windows App SDK Bootstrap packaging

- Include Bootstrap DLLs, SDK configuration, and redistribution notices in generated Python and Node.js packages using `--winappsdk-version` and repeatable `--winappsdk-bootstrap-path` options. Inputs may be DLL files or directories; directories are searched recursively. The version is the stable SDK product `MAJOR.MINOR`, not a NuGet servicing version. ([#225](https://github.com/microsoft/dynwinrt/pull/225))
- Call `init_winappsdk()` without version or DLL path arguments after installing the generated Python package, or `initWinappsdk()` after importing the generated Node.js package. Resources are package-relative, independent of the working directory. Existing explicit-version initialization remains available.

### Python AsyncEventScope notifications

- Add `AsyncEventScope` for async handlers attached through existing `subscribe_*` methods to same-thread notifications. Normal exit removes subscriptions and waits for accepted tasks without cancelling them; handler/body failures cancel sibling tasks and wait for cleanup using `asyncio.TaskGroup` semantics. Unsubscribing does not cancel already accepted tasks. ([#229](https://github.com/microsoft/dynwinrt/pull/229))

### Python interface close() and with support

- Add `close()` and `with` helpers to interfaces with an `IClosable` requirement, including `IInputStream`, `IOutputStream`, and `IRandomAccessStream`, without replacing native members. Context entry preserves the wrapper type; exit closes the resource without suppressing application exceptions. Existing concrete-class cleanup behavior is unchanged. ([#233](https://github.com/microsoft/dynwinrt/pull/233))

### Python XAML string keys

- WinUI and UWP `ResourceDictionary` bindings accept Python strings for key operations, for example `resources.lookup("AccentButtonStyle")`, with matching type hints. Object results and enumerated keys remain raw; this is not general implicit Object boxing. ([#221](https://github.com/microsoft/dynwinrt/pull/221))

### Python constructor error diagnostics

- Failed constructor matching reports supplied argument types and keyword names alongside candidate signatures using the generated Python names and types. Argument values are not included, and native activation/conversion failures keep their existing behavior. ([#230](https://github.com/microsoft/dynwinrt/pull/230))

### Nullable Python DetachStream return types

- `DataWriter.detach_stream()` and `DataReader.detach_stream()` are accurately typed as `IOutputStream | None` and `IInputStream | None`, including their declaring-interface projections. An unbound writer, a buffer-backed reader, or a repeated detach can return `None`; native behavior is unchanged. ([#231](https://github.com/microsoft/dynwinrt/pull/231))

### Python from_bytes() crash fix after apartment teardown

- Fix a crash when `from_bytes()` is used again after complete COM apartment teardown. Buffer creation no longer retains a stale activation factory across apartments; copied-byte semantics are unchanged. ([#232](https://github.com/microsoft/dynwinrt/pull/232))

## Upgrade notes

Upgrade runtime and codegen together. Regenerate the complete bindings selection, including `.py` and `.pyi` files together, and rebuild/reinstall generated packages. Do not mix old bindings or stubs with the new runtime.

- **Python layout migration:** Clean the dedicated output directory or generate into a fresh one. Old flat-layout output is rejected even if its producer version matches; do not move selected files or edit/remove the producer stamp to bypass this check.
- **Installation and imports:** Continue installing the outer project, for example `python -m pip install .\generated` or `python -m pip install -e .\generated`. Keep imports such as `from generated.windows.foundation import Uri`; do not add `src` to the import name. Select the installation's environment in your editor/type checker.
- **Direct source imports:** Prefer a normal or editable installation over old `sys.path`/`PYTHONPATH` adjustments. Deliberate uninstalled imports now need `<output>\src`, not `<output>` or its parent. Source-only imports do not register the distribution metadata needed for Python Bootstrap discovery.
- **Nullable streams:** Save each `detach_stream()` result and check `is not None` before calling stream methods.
- **Cleanup ordering:** Keep COM apartments and captured projected lifetimes alive until `AsyncEventScope` has finished task cleanup. Closing a stream with `with` or `close()` closes its native resource; it does not replace projection release or apartment teardown.

See the [Python layout migration contract](https://github.com/microsoft/dynwinrt/blob/main/tools/dynwinrt-codegen/python/README.md#breaking-layout-upgrade-from-preview23-and-earlier) for full regeneration, editable-install, and path guidance.

## Known boundaries

- **Bootstrap deployment:** Bundling Bootstrap DLLs does not install or include the complete Windows App Runtime. Applications using Bootstrap must deploy a matching Runtime, comply with the SDK's redistribution terms, initialize COM, and explicitly initialize Bootstrap. Importing configuration does not initialize either COM or Bootstrap.
- **Async notifications:** `AsyncEventScope` requires its creating OS thread and an application-driven, running asyncio loop. It does not marshal cross-thread events, run or integrate a WinUI message pump, or implement synchronous return, `Handled`, `Cancel`, or deferral semantics.
- **Experimental APIs:** Classic COM, flat Win32, and WinUI hosting remain experimental. Python generation still targets WinRT, not Classic COM or flat Win32. WinUI applications retain responsibility for UI threads and lifecycle; native ARM64 WinUI coverage remains incomplete.
- **Collection producers:** Existing architecture-specific ABI restrictions remain; see the [Python](https://github.com/microsoft/dynwinrt/blob/main/bindings/py/README.md#creating-winrt-collections) and [JavaScript](https://github.com/microsoft/dynwinrt/blob/main/bindings/js/README.md#creating-winrt-collections) support tables.
- **Windows paths:** The new source layout does not remove downstream Windows path-length limits. Keep generated-project and environment paths short; see the [path guidance](https://github.com/microsoft/dynwinrt/blob/main/tools/dynwinrt-codegen/python/README.md#windows-path-length).
- **Embedded hosts:** Stop new callbacks and settle in-flight work before calling `shutdown_python_callbacks()` on the live interpreter, then finalize Python. Skipping this protocol leaves only best-effort protection.

For usage and lifetime details, see the [Python runtime guide](https://github.com/microsoft/dynwinrt/blob/main/bindings/py/README.md), [JavaScript runtime guide](https://github.com/microsoft/dynwinrt/blob/main/bindings/js/README.md), [async-events sample](https://github.com/microsoft/dynwinrt/tree/main/samples/python/async-events), and [project README](https://github.com/microsoft/dynwinrt#readme).
