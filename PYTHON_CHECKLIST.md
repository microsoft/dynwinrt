# Python readiness checklist

This checklist tracks the path from the current source-built Python prototype
to a distributable, typed, and reliable WinRT projection.

See [`docs/python-ui-ecosystem.md`](docs/python-ui-ecosystem.md) for research on
Python UI frameworks, static pywinrt adoption, real integrations, and the value
of a dynamic projection.

## Target states

- **Data API preview:** a clean machine can install wheels and generated
  bindings, then call supported Windows SDK APIs without Rust, Node.js, or a
  source checkout.
- **Data API release:** generated APIs follow normal Python semantics for
  typing, async, events, collections, errors, and lifecycle.
- **WinUI preview:** Python can bootstrap the Windows App SDK, enter an STA,
  construct composable WinUI classes, load Fluent resources, and close cleanly.

## Verified baseline

- [x] The PyO3 runtime builds on Windows ARM64.
- [x] Generated `Windows.Foundation.Uri` bindings import and call real WinRT
      APIs on ARM64.
- [x] Python generation emits `.py` implementations.
- [x] `--lang py` emits `.pyi`, `__init__.pyi`, and `py.typed` by default.
- [x] Generated Python E2E covers 34 Windows SDK scenarios.
- [x] Runtime primitives exist for arrays, structs, delegates, events,
      cancellation, progress callbacks, vectors, and maps.
- [x] Python codegen snapshots cover the `Uri` implementation and stubs.

## P0: generated package correctness

- [x] Replace eager cross-module runtime imports with cycle-safe lazy
      resolution.
- [x] Keep referenced types visible to static type checkers through
      `TYPE_CHECKING` imports.
- [x] Add an import regression for a real cyclic graph such as
      `Windows.Data.Xml.Dom.XmlDocument`.
- [x] Preserve `Uri` implementation and `.pyi` snapshots.
- [ ] Organize output by WinRT namespace instead of one flat short-name
      namespace.
- [ ] Detect and reject namespace/type filename collisions.
- [ ] Add stage/swap generation and remove stale Python output.
- [ ] Emit a consumable Python package manifest with an exact runtime
      dependency.

## P0: runtime and generated API agreement

- [x] Ship a `.pyi` and `py.typed` marker for the `dynwinrt_py` extension.
- [x] Return declared enum members as generated `IntEnum` instances while
      preserving unknown values as integers.
- [x] Use `invoke_all()` for arbitrary multiple-out methods.
- [x] Add Python E2E for `IVector.IndexOf` and other multiple-out methods.
- [x] Project `IReference<T>` return positions as `T | None`.
- [x] Accept `T | None` in `IReference<T>` input positions.
- [x] Make Python stubs part of E2E and run a static type checker.
- [x] Make `.pyi` generation the default for `--lang py`.

## P0: async semantics

- [x] Return a Python awaitable instead of blocking inside generated async
      methods.
- [x] Integrate WinRT completion with an `asyncio` event loop.
- [x] Propagate Python cancellation to `IAsyncInfo.Cancel`.
- [x] Expose progress without forcing generated wrappers to call `.wait()`.
- [x] Keep an explicit blocking API for scripts and non-async hosts.
- [x] Reject blocking waits from an STA when they could deadlock.

## P0: CI and distribution

- [x] Execute all tests under `bindings/py/tests` in CI.
- [x] Build and install wheels before running Python E2E.
- [ ] Test supported CPython versions on x64 and ARM64.
- [ ] Remove unverified PyPy metadata or add real PyPy coverage.
- [ ] Derive the Python package version from the release tag.
- [ ] Publish signed/provenanced wheels to an approved internal Python feed.
- [ ] Add Python wheel assets and installation notes to GitHub releases.
- [ ] Provide a Python-native or standalone codegen installation path.
- [ ] Add a clean-machine consumer test that uses only released artifacts.

## P1: Python-native behavior

- [x] Map HRESULT failures to `OSError` with a stable `.winerror`.
- [x] Include restricted WinRT error information when available.
- [x] Surface Python callback failures instead of returning unconditional
      success to WinRT.
- [x] Document callback threads and require explicit event unsubscription.
- [x] Implement Python collection protocols for iterable, vector, and map
      projections.
- [x] Accept normal Python sequences, mappings, bytes, UUIDs, datetimes, and
      timedeltas where the WinRT signature permits them.
- [x] Emit `IntFlag` for flags enums.
- [x] Add overload-aware runtime dispatch and `typing.overload` stubs.
- [x] Generate idiomatic constructors such as `Uri(...)` while retaining an
      internal wrapper path for returned native objects.
- [x] Replace deprecated PyO3 automatic `FromPyObject` behavior explicitly.
- [x] Document COM apartment ownership and provide a balanced context manager.

## P1: tooling and documentation

- [ ] Add a Python configuration surface to winapp CLI or a dedicated Python
      orchestration command.
- [ ] Generate package names and imports consistently across Windows SDK,
      Windows App SDK, and custom WinMD namespaces.
- [ ] Add runnable samples for files, notifications, imaging, async, events,
      collections, and custom WinMD consumption.
- [ ] Document generated-code version compatibility with `dynwinrt-py`.
- [ ] Add troubleshooting for metadata, apartment, bootstrap, architecture,
      and wheel compatibility failures.

## WinUI milestone

- [ ] Project public composable constructors in Python codegen.
- [ ] Hide protected/system-returned constructors.
- [ ] Add composition/aggregation support required by XAML runtime classes.
- [ ] Make Windows App SDK initialization idempotent and version-aware.
- [ ] Auto-discover or explicitly provision the bootstrap DLL.
- [ ] Expose the selected framework `resources.pri` path.
- [ ] Add Python-specific implicit XAML metadata provider generation.
- [ ] Run `Application.start`, `Application`, `Window`, and all controls on an
      STA.
- [ ] Load Fluent Light, Dark, and High Contrast resources.
- [ ] Add a real Python WinUI application E2E on x64 and ARM64.

## Later

- [ ] Preserve projected object identity where it affects Python semantics.
- [ ] Support delegates with more than two ABI parameters.
- [ ] Add zero-copy Python buffer protocol integration.
- [ ] Add performance benchmarks against pywinrt for representative APIs.
- [ ] Add diagnostics for active COM wrappers, delegates, and async operations.

## Current implementation slice

1. [x] Make generated runtime imports cycle-safe.
2. [x] Add a real cyclic-package import regression.
3. [x] Re-run Python snapshots, codegen tests, and generated Python E2E.
4. [x] Return known enum values as generated `IntEnum` instances.
5. [x] Verify `Calendar.day_of_week` is a `DayOfWeek` at runtime.
6. [x] Use `invoke_all()` for arbitrary multiple-out methods.
7. [x] Model FillArray as caller-owned `(capacity, buffer)` ABI.
8. [x] Verify Python and JavaScript `IndexOf` and `GetMany`, including zero items.
9. [x] Ship typed `dynwinrt_py` wheels and validate the native extension stubs.
10. [x] Generate Python stubs by default and type-check generated E2E APIs.
11. [x] Unbox `IReference<T>` returns as `T | None`.
12. [x] Box native Python values and `None` for `IReference<T>` inputs.
13. [x] Map synchronous and asynchronous HRESULT failures to `OSError`.
14. [x] Preserve signed HRESULTs in `.winerror` and captured WinRT error messages.
15. [x] Project WinRT collections through Python sequence, mapping, and iterator protocols.
16. [x] Accept Python-native containers, bytes, UUID, datetime, and timedelta values.
17. [x] Route delegate exceptions through `sys.unraisablehook` and fail WinRT invocation.
18. [x] Emit `IntFlag`, overload dispatch/stubs, and idiomatic runtime-class constructors.
19. [x] Balance COM initialization with `RoApartment` and add deterministic `IClosable` cleanup.
