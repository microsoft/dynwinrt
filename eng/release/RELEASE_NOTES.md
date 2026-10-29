## Prerelease v0.1.0-preview.24

## Highlights

- **Python XAML string keys:** Generated WinUI and UWP `ResourceDictionary` bindings now accept Python strings directly, for example `resources.lookup("AccentButtonStyle")`, with matching type hints. ([#221](https://github.com/microsoft/dynwinrt/pull/221))
- **Windows App SDK Bootstrap packaging:** Generated Python and Node.js packages can now include Bootstrap DLLs and SDK configuration using `--winappsdk-version` and repeatable `--winappsdk-bootstrap-path` options. Inputs may be DLL files or directories. After installing the generated Python package, or importing the generated Node.js package, call `init_winappsdk()` or `initWinappsdk()` without SDK version or DLL path arguments. ([#225](https://github.com/microsoft/dynwinrt/pull/225))

## Upgrade notes

Upgrade runtime and codegen together, then regenerate and reinstall your bindings. The Windows App Runtime must still be installed; applications remain responsible for COM initialization.
