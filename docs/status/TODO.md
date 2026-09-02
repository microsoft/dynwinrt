# dynwinrt — TODO / Roadmap

Living document. Newest items on top of each section. Items removed once shipped for more than one release cycle.

## P0 — Release blockers

_None currently. Reserved for issues that make v0.1 unshippable (crash on happy path, data loss, security). Existing correctness / robustness work is tracked under P1._

## P1 — Correctness / quality (near-term)

- [ ] **Panic-free WinRT collection entrypoints**. Several internal
  `extern "system"` implementations still assume Windows supplied valid
  pointers:
  - `com_to_usize` / `com_usize_addref_out` and array clone/read paths use
    `IUnknown::from_raw_borrowed(...).unwrap()` after a non-null check;
  - generated `IInspectable` stubs still write through output pointers without
    validating them; and
  - vector/map/iterator methods still have output writes whose `E_POINTER`
    coverage must be audited method by method.

  Fix pattern: at every COM ABI entry, validate each out-pointer against null and return `E_POINTER`; convert `.unwrap()` on incoming COM pointers to `Result` + `E_UNEXPECTED`.

- [x] **JS 64-bit integer round trips**. Scalar, array, and struct-field
      boundaries accept range-checked `bigint` or safe integers. Array outputs
      use `bigint[]`, preserving the complete signed and unsigned 64-bit ranges.

- [x] **JS binding: TSFN failure propagation**. Delegate and progress callbacks
      map a failed TSFN queue operation to `E_FAIL` instead of reporting
      `S_OK` for a callback that was not accepted. The binding owns its TSFN
      payload lifecycle, releases rejected and teardown-drained values, uses a
      finite queue, and serializes calls with per-environment cleanup.

- [x] **JS binding: panic-shaped public APIs**. Invalid value conversions,
      raw-pointer access, primitive array conversions, array indexes, and
      struct field accesses now return `napi::Result<T>` errors. Struct fields
      validate type, numeric range, and nested struct identity before entering
      core accessors.

- [ ] **Codegen: `extract_iid` silently zero-fills malformed GuidAttribute**.
      `extract_u8`/`extract_u16`/`extract_u32` return `0` for a mismatched
      metadata value, producing a plausible-but-wrong IID. Treat a malformed
      attribute as a hard error or empty IID.

- [x] **Codegen: project concrete generic ancestor interfaces**. Inherited generic
      interfaces are resolved with their concrete arguments, deduplicated by full
      instantiated identity, and covered with `ColorPaletteResources` inheriting
      `IMap<Object, Object>` from `ResourceDictionary`.

- [x] **Named Python XAML custom types**. Process-local registrations are chained
  into the composed `IXamlMetadataProvider`; `XamlReader` activates generated
  Python control subclasses by arbitrary qualified names and preserves native
  overrides/identity. OS activation remains deliberately outside this boundary.

- [ ] **Codegen: default-interface lookup returns first hit**.
      `find_default_interface_type` returns the first `DefaultAttribute` it
      resolves. Validate that malformed metadata cannot expose multiple
      conflicting defaults, or fail loudly.

- [ ] **Codegen: snapshot coverage too narrow**. Python snapshots now cover
  `Windows.Foundation.Uri` and the method-rich
  `Windows.Storage.Streams.DataWriter`, but event-heavy types, parameterized
  interfaces, and inherited-interface flattening still need dedicated
  snapshots.

- [ ] **Rust: map key semantics under-specified**. `crates/dynwinrt/src/map.rs:79-120` — pointer identity is the default, with an ad-hoc string extraction path for `IPropertyValue`. Define one contract (identity vs value equality) and enforce it explicitly.

- [x] **JS: same-thread delegate N-API status handling**. Global lookup,
      callback invocation, exception inspection/clearing, fatal exception
      forwarding, and handle-scope closure are checked and mapped to a failing
      HRESULT when dispatch cannot complete. Each delegate also carries a
      `napi_async_context`, preserving `async_hooks` and `AsyncLocalStorage`.

## P2 — Feature completeness

- [ ] **Struct auto-marshaling**. Users still need `DynWinRtStruct.create()` + `setF64(...)` per field. Codegen generates `_packXxx` helpers for known structs already; the gap is user-defined / ad-hoc structs. Consider generic `pack(schema, obj)`.

- [x] **Python `IReference<T>` as struct field**. Generated structs read native
      `T | None`, accept native values and legacy wrappers, and box through
      `DynWinRTStruct.set_object`; covered by SDK `HttpProgress` and synthetic
      `IReference<Point>`.
- [x] **JS `IReference<T>` as struct field**. Generated structs read native
      `T | null`, accept native values, `null`, and legacy wrappers, and box
      through `DynWinRtStruct.setObject`; covered by SDK `HttpProgress`,
      synthetic `IReference<Point>`, and a runtime object-field round trip.

- [ ] **Guid array / bool array fast paths**. Currently per-element via `.toValues().map(...)`. Add `toGuidVec()` / `toBoolVec()` if any real workload hits these.

## P3 — Developer experience & performance

- [ ] **Auto-detect the WinAppSDK Bootstrap DLL for unpackaged apps**.
      Initialization now returns a typed error when neither
      `bootstrap_dll_path` nor `WINAPPSDK_BOOTSTRAP_DLL_PATH` is supplied, but
      it does not search restored `~/.winapp/packages`,
      `~/.nuget/packages/microsoft.windowsappsdk.*`, or installed framework
      locations.

- [ ] **JavaScript error message enrichment**. Preserve restricted WinRT error
      information alongside HRESULTs. Python already exposes the signed HRESULT
      through `.winerror` and includes restricted error text when Windows
      provides it.

- [ ] **Value-type inputs to `invoke()`**. `invoke()` currently requires `DynWinRtValue` wrappers per argument (`+~0.6-1.6 µs / arg`). Accept raw JS values (`number`/`string`/`bool`) and dispatch via `in_param_types()` on `MethodHandle`.

- [ ] **Method handle without an arena read lock**. `MethodHandle` stores an
      arena index; each call briefly reads `AppendOnlyBoxArena` to obtain its
      stable method pointer. Store a stable method handle directly if benchmark
      results justify removing that lock.

- [ ] **Stack-allocated return path**. `Ok(vec![out])` heap-allocates per call. `SmallVec<[WinRTValue; 2]>` for the common single-out shape.

- [ ] **JS binding: raw `Env` lifetime discipline**. The same-thread delegate
      path stores the raw `napi_env` under the assumption that the registering
      thread stays alive. Document and assert thread affinity, or minimize the
      raw environment lifetime.

- [ ] **`package.json` engines vs README floor**. `bindings/js/package.json`
      advertises Node 12+ ranges, while the README requires Node 18+. Align both
      to a tested release floor. (Currently CI runs Node 24.)

- [ ] **Python binding follow-ups**. Remaining work is tracked in
      `PYTHON_CHECKLIST.md`: WinApp CLI integration, consolidated
      troubleshooting, native ARM64 WinUI E2E, delegates with more than two ABI
      parameters, zero-copy buffers, and diagnostics.

- [ ] **Consolidate troubleshooting docs**. Apartment, bootstrap, package
      identity, architecture, and capability guidance exists across the root,
      Python, Node dev-mode, and sample READMEs; add one indexed troubleshooting
      guide instead of duplicating it further.

## Completed

Kept for reference; git history is the source of truth. Grouped by area.

### JS binding
- [x] All 13 process-crashing `.unwrap()` on public API paths → `napi::Result` with contextual errors
- [x] `call()` / `callVoid()` removed — `invoke()` is the sole invoke path
- [x] Removed unused `call_0`, `callSingleOut0`, `callSingleOut1`
- [x] `DynWinRtValue` constructors (bool, i8-u64, f32, f64, guid, null) + extractors (toBool, toI64, toF64, toGuid, isNull)
- [x] `DynWinRtType` factories (guid, char16, hresult, delegate, fillArray, iid) + `DynWinRtType.iid()` for parameterized IIDs
- [x] `toNumber()` expanded to Bool, I8, U8, I16, U16, I32, U32, HResult
- [x] `WinGuid.toString()` for cache keys
- [x] Auto value wrapping — `filter.append('.png')` works directly on generated `IVector_String`
- [x] `DynWinRtStruct::setObject` validates field kind and value type and returns
      a contextual N-API error instead of silently ignoring invalid values

### Runtime (crates/dynwinrt)
- [x] `SingleThreadedVector` / `SingleThreadedMap` migrated `RefCell` → `Mutex`; now `Send + Sync + IAgileObject`
- [x] `lock_or!` macro returns HRESULT on poisoning instead of panicking across FFI
- [x] Nested struct recursive Clone/Drop (HString, COM pointers in nested structs)
- [x] `ArrayData::get()` returns `WinRTValue::Null` for null COM elements instead of `IUnknown::from_raw(null)` (UB fix)
- [x] `ArrayData::get_i32()` returns checked `Result<i32>` errors for invalid indices/types across Values and CoTaskMem arrays
- [x] FillArray / ReceiveArray error paths use `ArrayData::drop` for per-element release; `ArrayOutSlot` + `FillArraySlot` have Drop impls
- [x] FillArray `actual_count` clamped to `capacity` (OOB read prevention)
- [x] F32 delegate ABI: separate f32/f64 trampolines for 1- and 2-param delegates
- [x] Vector value-type ABI: `write_item_out` writes only `elem_size` bytes for small value types
- [x] COM vtable panic safety: `lock_or!` + null-checked `from_raw_borrowed` returning HRESULTs
- [x] `WinRTValue::Enum { value, type_handle }` as independent runtime type
- [x] Parameterized type `default_winrt_value` no longer panics
- [x] `AppendOnlyBoxArena` documents and tests its stable-pointer invariant;
      method calls release the arena read guard before native dispatch
- [x] JS HSTRING struct fields use typed core accessors instead of layout casts

### Metadata / codegen
- [x] Struct helpers deduplicated (`generate_struct_helpers`: shared TS interface + pack/unpack)
- [x] Exclusive interfaces flattened via `all_interfaces()` (default + required)
- [x] Parameterized IID matches QI for `IAsyncOperationWithProgress` and async-of-struct-with-enum (enum-in-struct emits `enum(Ns.Name;i4)` in both runtime IID sig and codegen)
- [x] Missing-type warnings + `assert!(!iid.is_empty(), ...)` at generation time
- [x] `StructEntry.name` now `String` (deprecates `define_struct` in favor of `define_named_struct`)
- [x] `strip_generic_arity()` removed from winrt-meta
- [x] Parameterized interfaces (e.g. `IVector<String>`, `IReference<UInt32>`) generated as concrete types from winmd (removed unused `_collections.ts`)
- [x] Auto-detect `Windows.winmd` from `C:\Program Files (x86)\Windows Kits\10\UnionMetadata\`
- [x] Collection methods: IVector / IVectorView / IMap / IMapView / IKeyValuePair / IIterable / IIterator with full methods
- [x] Generated JSDoc includes summaries, parameter descriptions, return
      descriptions, and deprecation text from sibling XML documentation

### Features
- [x] Delegate / event support: COM vtable + napi ThreadsafeFunction in `delegate.rs`; `DynWinRtDelegate.create(iid, paramTypes, callback)`; same-thread synchronous invocation path
- [x] Python `.pyi` type stubs and `py.typed` marker by default with `--lang py`

### Distribution / CI
- [x] Python runtime wheel matrix for CPython 3.11–3.14 on x64/native ARM64,
  standalone `py3-none-win_<arch>` codegen wheels, isolated artifact consumers,
  shared GitHub release assets, and PyPI publication through Microsoft ESRP
- [x] npm prebuilds for `win32-x64-msvc` + `win32-arm64-msvc`
- [x] `.github/workflows/build.yml` validates the Rust runtime, JS/Python
  bindings, codegen, Classic COM coverage, and x64/ARM64 targets
- [x] `winapp init --add-js-bindings` toolchain integration
- [x] `package.json` repository URL corrected to `github.com/microsoft/dynwinrt`
- [x] Cargo.toml files have `authors`, `license`, `description`, `repository`; `bindings/py/pyproject.toml` has authors/license/urls
- [x] Electron benchmark app under `benchmarks/electron/` — full IPC round-trip static vs dynamic

### Cleanup
- [x] `[resolve]` debug `eprintln!` removed from `meta.rs`
- [x] `CLAUDE.md` refreshed (removed `tools/winrt-meta/` path, `--lang ts`; documented IR pipeline + `--pyi`)
- [x] Clippy pass (partial) — remaining redundant-closure / style warnings tracked separately
