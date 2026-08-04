# dynwinrt — TODO / Roadmap

Living document. Newest items on top of each section. Items removed once shipped for more than one release cycle.

## P0 — Release blockers

_None currently. Reserved for issues that make v0.1 unshippable (crash on happy path, data loss, security). Existing correctness / robustness work is tracked under P1._

## P1 — Correctness / quality (near-term)

- [ ] **Complete the first native Python ARM64 release-matrix run**. The
  fail-closed wheel workflow targets the existing
  `[self-hosted, Windows, ARM64, winui]` runner for CPython 3.11–3.14, but this
  worktree can validate only x64. Do not mark ARM64 wheels releasable until all
  native consumer jobs have imported them and called `Windows.Foundation.Uri`.

- [ ] **Panic-free COM entrypoints**. Several `extern "system"` COM callbacks still `unwrap()` on raw pointer inputs, which is UB across the FFI boundary if a caller passes a bad pointer:
  - `crates/dynwinrt/src/delegate.rs:309` — `IUnknown::from_raw_borrowed(&raw).unwrap()` inside `marshal_abi_ptr` (fires whenever WinRT passes a null pointer arg to a delegate; not just malicious callers)
  - `crates/dynwinrt/src/com_helpers.rs:42, 53` — same pattern in `com_to_usize` / `com_usize_addref_out`
  - `crates/dynwinrt/src/array.rs:288, 461` — array element COM read paths
  - `crates/dynwinrt/src/com_helpers.rs:74-103` — generated `IInspectable` stubs write through `*count`, `*iids`, `*name`, `*level` without null-checking the out-pointer
  - `crates/dynwinrt/src/vector.rs:229, 382, 384` — `get_at`, `get_many`, `get_size` write to `result`/`items_out`/`actual` unconditionally
  - `crates/dynwinrt/src/map.rs:222, 231, 267, 396` — `lookup`, `size`, `insert`, `split` do the same

  Fix pattern: at every COM ABI entry, validate each out-pointer against null and return `E_POINTER`; convert `.unwrap()` on incoming COM pointers to `Result` + `E_UNEXPECTED`.

- [ ] **JS `u64` round-trips through signed integers**. `to_u64_vec()` returns `Vec<i64>` and `from_u64_values()` takes `Vec<i64>` (`bindings/js/src/lib.rs:809-810, 892-895`). Also `DynWinRTValue::u64(value: i64)` at line 470 casts negatives to giant unsigned values silently. Values > `i64::MAX` are silent data corruption. Switch to `BigInt` or `u64` on the JS boundary.

- [ ] **JS binding: TSFN failure returns success**. `bindings/js/src/lib.rs:1463` discards the return of `tsfn.call(...)` and always returns `HRESULT(0)`. When the JS event queue is closed or the env is tearing down, WinRT sees success but the callback is silently dropped. Map failures to `E_FAIL` / a cancellation code.

- [ ] **JS binding: panic-shaped public APIs**. Two public methods still `panic!` on ordinary type mismatches, which propagates as a Node abort rather than a JS `throw`:
  - `bindings/js/src/lib.rs:645` — `DynWinRTValue::to_number` for unsupported kinds
  - `bindings/js/src/lib.rs:692` — `DynWinRTValue::as_raw` for non-object values

  Convert both to `napi::Result<T>`.

- [ ] **Codegen: `extract_iid` silently zero-fills malformed GuidAttribute**. `tools/dynwinrt-codegen/src/meta.rs:1030-1051` — if any GuidAttribute field is the wrong integer width, helpers return `0`, producing a plausible-but-wrong IID that will corrupt interface registration without any error. Treat non-matching shapes as a hard error / empty IID.

- [x] **Codegen: project concrete generic ancestor interfaces**. Inherited generic
      interfaces are resolved with their concrete arguments, deduplicated by full
      instantiated identity, and covered with `ColorPaletteResources` inheriting
      `IMap<Object, Object>` from `ResourceDictionary`.

- [x] **Named Python XAML custom types**. Process-local registrations are chained
  into the composed `IXamlMetadataProvider`; `XamlReader` activates generated
  Python control subclasses by arbitrary qualified names and preserves native
  overrides/identity. OS activation remains deliberately outside this boundary.

- [ ] **Codegen: default-interface lookup returns first hit**. `tools/dynwinrt-codegen/src/meta.rs:1075-1090` — `find_default_interface_iid` returns on the first `DefaultAttribute` it resolves, which may not be the actual default in edge cases with malformed metadata. Validate against parsed default interface metadata or fail loudly.

- [ ] **Codegen: `--class-name` docs vs `--class` CLI**. The CLI derives the flag from the field name (`class_name` → `--class-name`), and docs use `--class-name`. Confirm both are wired consistently and that any lingering `--class` example is updated. (One instance in `main.rs` after_help was fixed in this review round.)

- [ ] **Codegen: snapshot coverage too narrow**. Only `Windows.Foundation.Uri` is snapshotted. Add snapshots for (a) an event-heavy type exercising `on*` / `off*` emission, (b) a parameterized interface / generic instantiation, (c) a class exercising inherited-interface flattening. Otherwise the recent IR refactors have no regression net.

- [ ] **Rust: array typed getter can panic on bad index**. `crates/dynwinrt/src/array.rs:317` — `ArrayBuffer::Values(v) => v[index].as_i32().unwrap()` uses unchecked indexing while the CoTaskMem branch bounds-checks. Unify.

- [ ] **Rust: `AppendOnlyBoxArena::stable_ptr` panics on out-of-range**. `crates/dynwinrt/src/metadata_table/append_only_arena.rs:45-49` — trusted internal use, but the invariant is undocumented and callers can drift. Add a documented safety contract and a debug assertion (or return `Option`).

- [ ] **Rust: map key semantics under-specified**. `crates/dynwinrt/src/map.rs:79-120` — pointer identity is the default, with an ad-hoc string extraction path for `IPropertyValue`. Define one contract (identity vs value equality) and enforce it explicitly.

- [ ] **JS: N-API result codes ignored on same-thread delegate path**. `bindings/js/src/lib.rs:1416-1448` — `napi_get_undefined`, `napi_is_exception_pending`, `napi_get_and_clear_last_exception`, `napi_close_handle_scope` return statuses are dropped. Any failure can leave a pending exception across the ABI while still returning `S_OK`. Check every status.

- [ ] **JS: `DynWinRtStruct::set_object` silently no-ops**. `bindings/js/src/lib.rs:1103-1125` — unsupported input kinds hit `_ => {}`. Return `napi::Result<()>` with a clear error.

## P2 — Feature completeness

- [ ] **Struct auto-marshaling**. Users still need `DynWinRtStruct.create()` + `setF64(...)` per field. Codegen generates `_packXxx` helpers for known structs already; the gap is user-defined / ad-hoc structs. Consider generic `pack(schema, obj)`.

- [ ] **Nullable / `IReference<T>` handling**. Null COM pointers surface as `WinRTValue::Null`; codegen wrappers should surface `T | null` in `.d.ts` for these return positions.

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

- [ ] **Auto-detect WinAppSDK Bootstrap DLL for unpackaged apps**. `initialize_winappsdk(major, minor)` currently `.expect(...)`s the `WINAPPSDK_BOOTSTRAP_DLL_PATH` env var (`crates/dynwinrt/src/winapp.rs:43`). Only relevant when a user is running unpackaged (packaged/MSIX apps don't call this — the framework package dep loads WinAppSDK automatically). For the unpackaged path, search `~/.winapp/packages/`, `~/.nuget/packages/microsoft.windowsappsdk.*/`, and standard Program Files install paths, in that order, falling back to the env var. Also swap the `expect(...)` for a typed error.

- [ ] **Error message enrichment**. Wrap HRESULT errors with `IRestrictedErrorInfo` message strings on the way out; today users see raw HRESULT codes.

- [ ] **Value-type inputs to `invoke()`**. `invoke()` currently requires `DynWinRtValue` wrappers per argument (`+~0.6-1.6 µs / arg`). Accept raw JS values (`number`/`string`/`bool`) and dispatch via `in_param_types()` on `MethodHandle`.

- [ ] **Method handle without `RwLock`**. `invoke_method` takes an `RwLock` read on every call (~15-20 ns). Store `Arc<Method>` directly in `MethodHandle` and bypass the arena lock on the hot path.

- [ ] **Stack-allocated return path**. `Ok(vec![out])` heap-allocates per call. `SmallVec<[WinRTValue; 2]>` for the common single-out shape.

- [ ] **JS binding: raw `Env` lifetime discipline**. `bindings/js/src/lib.rs:1400-1412` — the same-thread delegate path stores the raw `napi_env` under the assumption that the registering thread stays alive. Document + assert thread affinity, or minimize raw-env lifetime.

- [ ] **JS: `HSTRING` field extraction via layout cast**. `bindings/js/src/lib.rs:1041-1049` — reads HSTRING out of `ValueTypeData` by reinterpreting bytes as `*const HSTRING`. Expose a typed accessor in `dynwinrt` (`ValueTypeData::field_hstring(index)`) and switch the JS side to it.

- [ ] **`package.json` engines vs README floor**. `bindings/js/package.json:34-36` advertises Node 12+ ranges, README says Node ≥16. Align to whichever floor CI actually tests. (Currently CI runs Node 24.)

- [ ] **Python binding follow-ups**. The runtime and codegen now cover async,
  collections, structs, typed delegates/events, nullable references, and an
  experimental WinUI Application bootstrap. Remaining work is tracked in
  `PYTHON_CHECKLIST.md`, especially generated package layout, CPython/architecture
  coverage, native XAML custom-control registration/overrides, object identity,
  and delegates with more than two ABI parameters.

- [ ] **Troubleshooting docs in READMEs**. Common failure modes not covered end-to-end: `WINAPPSDK_BOOTSTRAP_DLL_PATH` not set, mismatched apartment, missing capability. Root `README.md` has a small table; grow it based on the last three GitHub issues that repeated.

- [ ] **JSDoc / TSDoc on generated `.d.ts`**. Parameter descriptions and return descriptions are missing on many generated method signatures. `xml_doc.rs` already loads sibling `.xml` — thread that through render_dts.

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

### Runtime (crates/dynwinrt)
- [x] `SingleThreadedVector` / `SingleThreadedMap` migrated `RefCell` → `Mutex`; now `Send + Sync + IAgileObject`
- [x] `lock_or!` macro returns HRESULT on poisoning instead of panicking across FFI
- [x] Nested struct recursive Clone/Drop (HString, COM pointers in nested structs)
- [x] `ArrayData::get()` returns `WinRTValue::Null` for null COM elements instead of `IUnknown::from_raw(null)` (UB fix)
- [x] FillArray / ReceiveArray error paths use `ArrayData::drop` for per-element release; `ArrayOutSlot` + `FillArraySlot` have Drop impls
- [x] FillArray `actual_count` clamped to `capacity` (OOB read prevention)
- [x] F32 delegate ABI: separate f32/f64 trampolines for 1- and 2-param delegates
- [x] Vector value-type ABI: `write_item_out` writes only `elem_size` bytes for small value types
- [x] COM vtable panic safety: `lock_or!` + null-checked `from_raw_borrowed` returning HRESULTs
- [x] `WinRTValue::Enum { value, type_handle }` as independent runtime type
- [x] Parameterized type `default_winrt_value` no longer panics

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

### Features
- [x] Delegate / event support: COM vtable + napi ThreadsafeFunction in `delegate.rs`; `DynWinRtDelegate.create(iid, paramTypes, callback)`; same-thread synchronous invocation path
- [x] Python `.pyi` type stubs and `py.typed` marker by default with `--lang py`

### Distribution / CI
- [x] Python runtime wheel matrix for CPython 3.11–3.14 on x64/native ARM64,
  standalone `py3-none-win_<arch>` codegen wheels, isolated artifact consumers,
  GitHub release assets, and manual OIDC trusted-publishing gates
- [x] npm prebuilds for `win32-x64-msvc` + `win32-arm64-msvc`
- [x] `.github/workflows/build.yml` builds winrt-meta and dynwinrt-js on x64 + arm64, plus publishing and sample generation
- [x] `winapp init --add-js-bindings` toolchain integration
- [x] `package.json` repository URL corrected to `github.com/microsoft/dynwinrt`
- [x] Cargo.toml files have `authors`, `license`, `description`, `repository`; `bindings/py/pyproject.toml` has authors/license/urls
- [x] Electron benchmark app under `bench-electron/` — full IPC round-trip static vs dynamic

### Cleanup
- [x] `[resolve]` debug `eprintln!` removed from `meta.rs`
- [x] `CLAUDE.md` refreshed (removed `tools/winrt-meta/` path, `--lang ts`; documented IR pipeline + `--pyi`)
- [x] Clippy pass (partial) — remaining redundant-closure / style warnings tracked separately
