# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`dynwinrt` is a Rust-based runtime library that enables dynamic invocation of Windows Runtime (WinRT) APIs. Unlike static projections (PyWinRT, C++/WinRT), this library uses runtime metadata (.winmd files) and FFI (libffi) to call arbitrary WinRT methods without native code generation. It provides JavaScript (napi-rs) and Python (PyO3) bindings, plus a code generation tool (`dynwinrt-codegen`) that produces typed wrappers from .winmd files.

Classic COM and flat Win32 have separate JavaScript/TypeScript projections and
runtime subpaths. Flat Win32 DLL exports use an independent native ABI and
contract model; see [the Win32 architecture](docs/architecture/flat-win32-contracts.md).
Run `.\tests\e2e\e2e_test.ps1 -SkipBuild -Lang win32` for its E2E suite with
`DYNWINRT_WIN32_WINMD` pointing to the pinned Win32 metadata.

## Repository Structure

```
dynwinrt/
├── crates/dynwinrt/          # Core Rust library (FFI, types, metadata, async, delegates, collections)
├── bindings/
│   ├── js/                   # JavaScript/TypeScript bindings (napi-rs)
│   └── py/                   # Python bindings (PyO3)
├── tools/
│   └── dynwinrt-codegen/     # Code generator (JS + .d.ts and Python from .winmd)
├── tests/e2e/                # Cross-language E2E specs, runners, and scripts
├── benchmarks/               # Electron and JS/native benchmarks
├── samples/                  # JavaScript and Python samples
├── docs/                     # Architecture, benchmark, guide, and status docs
└── eng/release/python/       # Python release verification helpers
```

## Build Commands

```bash
# Build everything
cargo build

# Run core library tests
cargo test -p dynwinrt

# Run dynwinrt-codegen tests (includes snapshot tests)
cargo test -p dynwinrt-codegen

# Build JS bindings
cd bindings/js && npm install && npm run build

# Build Python bindings
cd bindings/py && maturin develop

# Run Python tests
cd bindings/py && python -m pytest tests/ -v

# Build dynwinrt-codegen in release mode
cargo build -p dynwinrt-codegen --release

# Generate JS bindings (.js + .d.ts) — default --lang is "js"
cargo run -p dynwinrt-codegen -- generate --namespace Windows.Foundation --class-name Uri --output ./generated

# Generate Python bindings (.pyi stubs are emitted by default)
cargo run -p dynwinrt-codegen -- generate --namespace Windows.Foundation --class-name Uri --lang py --output ./generated
```

## Environment Setup

**Critical**: Set the `WINAPPSDK_BOOTSTRAP_DLL_PATH` environment variable to the path of the WinAppSDK Bootstrap DLL before running tests that use WinAppSDK APIs (e.g., FileOpenPicker). Without this, WinAppSDK initialization will fail.

All tests assume Windows 10/11 with SDK installed at `C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd`.

## Architecture

### Core Type System (`crates/dynwinrt/`)

The core uses a metadata-driven approach with these key abstractions:

1. **MetadataTable** (`metadata_table/mod.rs`): Central registry that loads .winmd files, stores interface registrations, and resolves types. Created once as a `LazyLock<Arc<MetadataTable>>` and shared across all bindings.
2. **TypeHandle** (`metadata_table/type_handle.rs`): Smart reference to a type in the arena. Supports all WinRT types: primitives (I8-U64, F32, F64, Bool), HString, GUID, Object, Interface, RuntimeClass, Delegate, Struct, Enum, Array, and parameterized generics.
3. **TypeKind** (`metadata_table/type_kind.rs`): Enum of all supported WinRT type categories.
4. **MethodHandle** (`metadata_table/method_handle.rs`): Bound method reference with `invoke(raw_ptr, &[WinRTValue])` for calling COM vtable methods.
5. **ValueTypeData** (`metadata_table/value_data.rs`): Blittable struct storage with field-level get/set access.
6. **WinRTValue** (`value.rs`): Runtime value container — the universal currency between Rust, JS, and Python.

### Dynamic Method Invocation (`call.rs`)

The call mechanism works as follows:
1. `MetadataTable::register_interface()` records an interface's IID and method signatures
2. `MetadataTable::method(vtable_index)` returns a `MethodHandle`
3. `MethodHandle::invoke(raw_ptr, args)` extracts the function pointer from the COM vtable, marshals arguments via libffi, calls the method, and converts output parameters back to `WinRTValue`

### Key Modules

- **call.rs**: Core FFI invocation logic — vtable pointer extraction, libffi argument marshaling, dynamic method calling
- **metadata_table/**: Type registry, arena allocation, IID computation, method handles, type handles
  - **arena.rs**: Arena-based type storage with `TypeKind` → `TypeHandle` mapping
  - **iid.rs**: Parameterized interface IID computation (SHA-1 based)
  - **method_handle.rs**: Bound method invocation with fast-path getters
- **delegate.rs**: Dynamic WinRT delegate (callback) COM objects — supports up to 2 ABI params, covers TypedEventHandler, EventHandler, etc.
- **dasync.rs**: Async operation support — `WinRTAsyncFuture` implementing Rust's `Future` trait for IAsyncAction/IAsyncOperation with progress handler support
- **vector.rs**: Dynamic IVector\<T\> / IVectorView\<T\> / IIterable\<T\> COM implementation
- **map.rs**: Dynamic IMap\<K,V\> / IMapView\<K,V\> / IKeyValuePair\<K,V\> COM implementation
- **array.rs**: WinRT array (pass/fill/receive) marshaling via `ArrayData`
- **value.rs**: `WinRTValue` enum with all supported types + `AsyncInfo` for async operations
- **signature.rs**: Legacy `InterfaceSignature`/`MethodSignature` (still used by some tests)
- **roapi.rs**: `RoGetActivationFactory` wrapper
- **winapp.rs**: WinAppSDK Bootstrap initialization

### JS Binding (`bindings/js/`)

napi-rs binding exposing: `DynWinRtType`, `DynWinRtMethodSig`, `DynWinRtMethodHandle`, `DynWinRtValue`, `DynWinRtArray`, `DynWinRtStruct`, `DynWinRtDelegate`, `WinGuid`. Async operations return Promises via `toPromise()`. Events use `DynWinRtDelegate.create()` with ThreadsafeFunction for cross-thread callback safety.

### Python Binding (`bindings/py/`)

PyO3 binding exposing: `DynWinRTType`, `DynWinRTMethodSig`, `DynWinRTMethodHandle`, `DynWinRTValue`, `DynWinRTArray`, `DynWinRTStruct`, `DynWinRtDelegate`, `WinGUID`. Generated async operations support `asyncio`; the low-level `wait()` fallback blocks with the GIL released. Events use `DynWinRtDelegate.create()` with `Python::attach()` for GIL-safe callback invocation.

### Code Generation Tool (`dynwinrt-codegen`, source in `tools/dynwinrt-codegen/`)

Reads .winmd metadata and generates typed wrapper code:
- `--lang js` (default): emits CommonJS `.js`, ESM index facades, and `.d.ts` declarations using the `DynWinRtType`/`DynWinRtValue` API; no tsc step is required
- `--lang py`: Python classes using the `DynWinRTType`/`DynWinRTValue` API, with `.pyi` stubs and a `py.typed` marker by default; `--no-pyi` opts out
- Handles: classes, interfaces, enums, structs (pack/unpack), delegates (IID + param types), async operations (with `AbortSignal`/cancellation), generic collections, events
- Auto-detects Windows SDK winmd, auto-discovers sibling `.winmd` files in the same directory, resolves transitive dependencies

Key codegen modules:
- **codegen/winrt/shared/**: Shared WinRT dependency, import, and documentation helpers
- **codegen/winrt/javascript/project/** + **ir.rs**: Build the JavaScript/TypeScript projection IR from metadata
- **codegen/winrt/javascript/render/**: Render CommonJS `.js` and `.d.ts` declarations
- **codegen/winrt/javascript/generator.rs**: Index-file generation and ESM/CommonJS facades
- **codegen/winrt/python/generator/** + **stubs.rs**: Python `.py` and `.pyi` generation
- **meta.rs**: WinMD parsing — classes, interfaces, enums, methods, parameters, vtable indices
- **types.rs**: `TypeMeta` enum describing WinRT types extracted from metadata
- **xml_doc.rs**: Loads sibling `.xml` files (C# /doc format) to inject JSDoc/docstrings

## Testing Strategy

Tests use real Windows APIs without mocking:

- **Core Rust tests** (`cargo test -p dynwinrt`): Uri, HttpClient (async), XmlDocument, metadata reading, vector/map collections
- **dynwinrt-codegen tests** (`cargo test -p dynwinrt-codegen`): Snapshot tests for Uri TypeScript output, unit tests for type mapping/codegen
- **Python binding tests** (`bindings/py/tests/`):
  - `test_basic.py`: Binding features — primitives, GUID, arrays, structs, enum, URI E2E
  - `test_e2e_winrt.py`: Real WinRT APIs — XmlDocument, Geopoint, PropertyValue, Buffer, Uri
  - `test_runtime.py`: Value conversion utilities

## Common Patterns

### Using MetadataTable (current API)

```rust
use dynwinrt::{MetadataTable, WinRTValue};

let table = MetadataTable::new();

// Register an interface
let iface = table.register_interface("IUriRuntimeClass", iid)
    .add_method(table.method_sig().add_out(table.hstring()));  // get_AbsoluteUri

// Get a method handle and invoke
let method = iface.method(6);  // vtable index 6
let result = method.invoke(obj_ptr, &[])?;
```

### Creating a delegate

```rust
let delegate = dynwinrt::delegate::create_delegate_value(
    handler_iid,
    vec![param1_type, param2_type],
    Box::new(|args| { /* callback logic */ HRESULT(0) }),
);
```

### Async operations

```rust
// Core: WinRTValue::Async implements Future
let result = async_value.await?;
```

```js
// JS: returns Promise
const result = await value.toPromise();
```

```python
result = await generated_async_operation
result = value.wait()  # Low-level blocking fallback; releases the GIL.
```

## Known Limitations

- Delegate callbacks support up to 2 ABI parameters (covers ~95% of WinRT delegates)
- WinUI/XAML hosting and DispatcherQueue support remain experimental and require matching Windows App SDK metadata and runtime inputs.
- Python generated async operations integrate with `asyncio`; synchronous interface-implementation callbacks must not return coroutines.

## Environment Setup — updated invariant

Windows App SDK bootstrap requires an explicit `bootstrap_dll_path` or `WINAPPSDK_BOOTSTRAP_DLL_PATH`. Missing paths, invalid versions, and DLL/export failures return errors; they do not panic. Samples restore or select matching SDK inputs explicitly. Bootstrap-path auto-discovery is not implemented.

## Implementation Notes

### Why libffi?

libffi provides portable FFI that can call functions with arbitrary signatures at runtime. This is essential because WinRT method signatures are only known after reading WinMD metadata.

### COM Object Lifetimes

The library uses `windows-core::IUnknown` smart pointers which automatically handle AddRef/Release. Raw pointers extracted via `as_raw()` are only used for the duration of a single call.

### Parameterized IID Computation

Generic interfaces (IVector\<T\>, IMap\<K,V\>, IAsyncOperation\<T\>) have IIDs computed at runtime using the WinRT parameterized interface algorithm (SHA-1 hash of the PIID + type argument signatures). This is implemented in `metadata_table/iid.rs`.
