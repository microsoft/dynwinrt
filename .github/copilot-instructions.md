# Copilot Agent Instructions

This file provides instructions for the GitHub Copilot coding agent when working on this repository.

## Project Overview

`dynwinrt` is a Rust library for dynamically invoking Windows Runtime (WinRT) APIs. It includes:
- **Core library** (`crates/dynwinrt/`) — Rust runtime using libffi for dynamic WinRT method invocation
- **JS binding** (`bindings/js/`) — napi-rs binding for Node.js/Electron
- **Python binding** (`bindings/py/`) — PyO3 binding for Python
- **Code generator** (`tools/dynwinrt-codegen/`) — Source for dynwinrt-codegen, which generates typed JavaScript (`.js`) + TypeScript declarations (`.d.ts`), or Python (`.py` + optional `.pyi`) wrappers from .winmd metadata

## Build & Test Commands

```bash
# Build everything
cargo build

# Core library tests (52 tests, 1 ignored — requires WinAppSDK)
cargo test -p dynwinrt

# dynwinrt-codegen tests (49 unit tests + 1 snapshot test)
cargo test -p dynwinrt-codegen

# Python binding (requires Python 3.8+ and maturin)
cd bindings/py
python -m venv .venv && .venv/Scripts/Activate.ps1
pip install pytest maturin
maturin build
pip install target/wheels/*.whl --force-reinstall
python -m pytest tests/ -v

# JS binding (requires Node.js 18+)
cd bindings/js
npm install
npx napi build --no-const-enum --platform --release -o dist

# Code generation (JS + .d.ts is the default; --lang py for Python)
cargo run -p dynwinrt-codegen -- generate --namespace Windows.Foundation --class-name Uri --output ./generated
cargo run -p dynwinrt-codegen -- generate --namespace Windows.Foundation --class-name Uri --lang py --output ./generated

# E2E test (full pipeline: winmd → generate → call real WinRT APIs)
.\tests\e2e_test.ps1 -SkipBuild -Lang py
```

## E2E Testing

The E2E test framework validates the full pipeline: reading .winmd metadata → generating code → calling real Windows APIs.

### How it works

1. **Test specs** are defined in `tests/e2e_specs.json` (schema: `tests/e2e_specs.schema.json`) — each entry describes:
   - `instantiate`: how to create an instance (`activate`, `static_factory`, or `none`)
   - `checks`: array of assertions (`property_equals`, `property_exists`, `method_equals`, `method_result_contains`, `static_equals`, `static_not_null`)

2. **Runners** (`tests/runners/py_runner.py`, `tests/runners/ts_runner.ts`) read the specs and execute them, outputting `results.json`.

3. **Orchestrator** (`tests/e2e_test.ps1`) handles build, code generation, and runner invocation.

4. **Adding new test cases**: Add entries to `e2e_specs.json`:
```json
{
  "namespace": "Windows.Foundation",
  "class": "Uri",
  "langs": ["py", "ts"],
  "instantiate": { "kind": "static_factory", "method": "create_uri", "args": ["https://example.com"] },
  "checks": [
    { "kind": "property_equals", "member": "host", "expected": "example.com" },
    { "kind": "method_result_contains", "member": "combine_uri", "args": ["sub/page"], "contains": "sub/page" },
    { "kind": "static_equals", "member": "escape_component", "args": ["hello world"], "expected": "hello%20world" }
  ]
}
```

### Testable WinRT APIs (no extra dependencies needed)

These APIs are available on any Windows 10/11 machine without WinAppSDK:
- `Windows.Foundation`: Uri, PropertyValue, WwwFormUrlDecoder, MemoryBuffer, Deferral
- `Windows.Data.Xml.Dom`: XmlDocument (has circular import issue in Python codegen)
- `Windows.Globalization`: Calendar, Language, GeographicRegion, CurrencyIdentifiers
- `Windows.Devices.Geolocation`: Geopoint (factory with struct parameter)
- `Windows.Storage.Streams`: Buffer
- `Windows.Security.Cryptography`: CryptographicBuffer

## Architecture Notes

### Type System Flow
```
.winmd metadata → MetadataTable (arena) → TypeHandle → MethodHandle → libffi call → WinRTValue
```

### Key Design Decisions
- **MetadataTable** is a global singleton (`LazyLock<Arc<MetadataTable>>`) shared across all bindings
- **Interface registration** is by IID (GUID) — each IID maps to one vtable with methods
- **Method invocation** returns a single `WinRTValue` (not a list) in Python binding
- **Generated code** uses relative imports (`from .module import Class`) — must be in a Python package

### Code Generator (dynwinrt-codegen)
- `src/codegen/project.rs` + `src/codegen/projected.rs` — Build the language-neutral `ProjectedFile` IR from parsed metadata
- `src/codegen/render_js.rs` + `src/codegen/render_dts.rs` — Render IR to `.js` and `.d.ts`
- `src/codegen/python.rs` + `src/codegen/py_method.rs` + `src/codegen/python_stub.rs` — Python `.py` and `.pyi` generation
- `src/codegen/common.rs` — Shared helpers (type mapping, argument wrapping, return conversion)
- `src/codegen/typescript.rs` + `src/codegen/method.rs` — Index-file generation and ESM/CJS shim helpers (name is historical; no longer emits stand-alone `.ts` output)
- `--lang js` (default) generates `.js` + ambient `.d.ts` with the `DynWinRtType`/`DynWinRtValue` API (camelCase)
- `--lang py` generates `.py` (and, with `--pyi`, `.pyi` stubs plus a `py.typed` marker) using the `DynWinRTType`/`DynWinRTValue` API (snake_case)

### Python Binding API Names
- Types: `DynWinRTType.i32_type()`, `DynWinRTType.hstring()`, `DynWinRTType.bool_type()`, etc.
- Values: `DynWinRTValue.from_i32()`, `DynWinRTValue.from_hstring()`, `DynWinRTValue.from_bool()`
- GUID: `WinGUID.parse('...')`
- Method call: `method_handle.invoke(obj, [args])` → returns single `DynWinRTValue`

### Common Issues
- `test_initialize` is `#[ignore]` — requires `WINAPPSDK_BOOTSTRAP_DLL_PATH` env var
- Python `@property` must come before `@prop.setter` — codegen reorders methods for this
- Windows SDK winmd is at `C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd`


