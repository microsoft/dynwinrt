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

- Generated Python packages use a standard `src` layout so editable installs work with static type checkers. ([#228](https://github.com/microsoft/dynwinrt/pull/228))

### Windows App SDK Bootstrap packaging

- Generated Python and Node.js packages can include Bootstrap DLLs and SDK configuration for initialization without hard-coded versions or DLL paths. ([#225](https://github.com/microsoft/dynwinrt/pull/225))

### Opt-in automatic Bootstrap

- Python `dynwinrt.auto_init` and Node.js `@microsoft/dynwinrt/auto-init` skip Bootstrap for processes with package identity and otherwise use generated SDK configuration.

### Python AsyncEventScope notifications

- `AsyncEventScope` manages async handlers for same-thread notifications and cleans up their subscriptions and tasks. ([#229](https://github.com/microsoft/dynwinrt/pull/229))

### Python interface close() and with support

- Closable stream interfaces such as `IRandomAccessStream` now support `close()` and `with` for resource cleanup. ([#233](https://github.com/microsoft/dynwinrt/pull/233))

### Python XAML string keys

- WinUI and UWP `ResourceDictionary` bindings now accept Python string keys, with matching type hints. ([#221](https://github.com/microsoft/dynwinrt/pull/221))

### Python constructor error diagnostics

- Python constructor matching errors now show supplied argument types and accepted signatures. ([#230](https://github.com/microsoft/dynwinrt/pull/230))

### Nullable Python DetachStream return types

- Type hints for `DataWriter.detach_stream()` and `DataReader.detach_stream()` now correctly allow `None`. ([#231](https://github.com/microsoft/dynwinrt/pull/231))

### Python from_bytes() crash fix after apartment teardown

- Fix a `from_bytes()` crash when COM is uninitialized and initialized again before creating another buffer. ([#232](https://github.com/microsoft/dynwinrt/pull/232))

## Upgrade notes

Upgrade runtime and codegen together, then regenerate and reinstall all bindings.

- **Python layout:** Use a fresh or cleaned codegen-owned output directory; install the outer project, not `src`. Import names stay unchanged.
- **Direct source imports:** Point them at `<output>\src`; Bootstrap discovery still requires installing the package.
- **Nullable streams:** Check `detach_stream()` results for `None`.
- **Async cleanup:** Keep COM apartments and projected lifetimes alive until `AsyncEventScope` exits.

See the [Python layout migration contract](https://github.com/microsoft/dynwinrt/blob/main/tools/dynwinrt-codegen/python/README.md#breaking-layout-upgrade-from-preview23-and-earlier) for details.

## Known boundaries

- **Bootstrap deployment:** Bundled DLLs do not install Windows App Runtime; apps using Bootstrap must handle Runtime deployment, COM/Bootstrap initialization, and SDK redistribution requirements.
- **Async notifications:** `AsyncEventScope` requires a running asyncio loop on its owning thread; it does not provide cross-thread marshalling, WinUI pump integration, or synchronous-return/deferral support.
- **Experimental APIs:** Classic COM, flat Win32, and WinUI hosting remain experimental; Python targets WinRT only, and native ARM64 WinUI validation remains limited.
- **Collection producers:** Architecture-specific restrictions remain; see the [Python](https://github.com/microsoft/dynwinrt/blob/main/bindings/py/README.md#creating-winrt-collections) and [JavaScript](https://github.com/microsoft/dynwinrt/blob/main/bindings/js/README.md#creating-winrt-collections) support tables.
- **Windows paths:** Keep generated-project and environment paths short; [Windows path-length limits](https://github.com/microsoft/dynwinrt/blob/main/tools/dynwinrt-codegen/python/README.md#windows-path-length) still apply.
- **Embedded hosts:** Stop new callbacks, drain in-flight work, then call `shutdown_python_callbacks()` before finalizing Python.

Details: [Python runtime guide](https://github.com/microsoft/dynwinrt/blob/main/bindings/py/README.md), [JavaScript runtime guide](https://github.com/microsoft/dynwinrt/blob/main/bindings/js/README.md), [async-events sample](https://github.com/microsoft/dynwinrt/tree/main/samples/python/async-events), and [project README](https://github.com/microsoft/dynwinrt#readme).
