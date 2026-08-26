# Python readiness checklist

This checklist tracks the path from the current source-built Python prototype
to a distributable, typed, and reliable WinRT projection.

See [`docs/guides/python/python-ui-ecosystem.md`](../guides/python/python-ui-ecosystem.md) for research on
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
- [x] Model WinRT runtime-class and interface compatibility structurally in
      input annotations without changing runtime inheritance.
- [x] Add an import regression for a real cyclic graph such as
      `Windows.Data.Xml.Dom.XmlDocument`.
- [x] Preserve `Uri` implementation and `.pyi` snapshots.
- [x] Organize output by WinRT namespace instead of one flat short-name
      namespace.
- [x] Detect and reject namespace/type filename collisions.
- [x] Add stage/swap generation and remove stale Python output.
- [x] Emit a consumable Python package manifest with an exact runtime
      dependency.
- [x] Emit one canonical module and Python class per WinRT struct instead of
      duplicating incompatible struct classes in every consumer module.
- [x] Make root and namespace exports lazy, keep ABI helpers in type-specific
      facades, and expose only Python types from public package indexes.

## P0: runtime and generated API agreement

- [x] Ship a `.pyi` and `py.typed` marker for the `dynwinrt` extension.
- [x] Return declared enum members as generated `IntEnum` instances while
      preserving unknown values as integers.
- [x] Use `invoke_all()` for arbitrary multiple-out methods.
- [x] Add Python E2E for `IVector.IndexOf` and other multiple-out methods.
- [x] Project `IReference<T>` return positions as `T | None`.
- [x] Accept `T | None` in `IReference<T>` input positions.
- [x] Project null reference returns and reference-array elements as `T | None`
      consistently in runtime code and stubs.
- [x] Project `IReference<T>` struct fields as native `T | None` on reads while
      accepting native values, `None`, and legacy wrappers on writes.
- [x] Preserve the full `UInt32` / `UInt64` range when converting to Python
      integers.
- [x] Make Python stubs part of E2E and run a static type checker.
- [x] Make `.pyi` generation the default for `--lang py`.
- [x] Pass full-package `mypy --strict`, including collection overrides and
      cross-module struct arguments.

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
- [ ] Complete the first live CPython 3.11–3.14 x64/ARM64 release-matrix run.
      The native ARM64 jobs are configured on the existing self-hosted WinUI
      runner and fail closed; local x64 validation does not count as ARM64
      validation.
- [x] Remove unverified PyPy metadata.
- [x] Derive Python artifact versions from ordinary `v<version>` release tags,
      inject them only into ephemeral CI workspaces, and attach the complete
      wheel set to the shared JavaScript GitHub release.
- [ ] Publish signed/provenanced wheels to an approved internal Python feed.
- [x] Add a protected-environment, OIDC trusted-publishing path that requires
      manual opt-in from an existing release tag.
- [x] Add complete tested Python wheel assets and installation notes to GitHub
      releases.
- [x] Provide a Python-native codegen CLI wheel using maturin binary bindings.
- [x] Add clean, isolated wheel consumers that import/call stock WinRT and run
      codegen with Cargo/Rust unavailable.

## P1: Python-native behavior

- [x] Map HRESULT failures to `OSError` with a stable `.winerror`.
- [x] Include restricted WinRT error information when available.
- [x] Surface Python callback failures instead of returning unconditional
      success to WinRT.
- [x] Document callback threads and require explicit event unsubscription.
- [x] Preserve token-based `on_*` / `off_*` compatibility and provide
      idempotent `subscribe_*` and reentrancy-safe `once_*` helpers.
- [x] Convert `TypedEventHandler` / `EventHandler` callback arguments to typed
      projected Python values.
- [x] Implement Python collection protocols for iterable, vector, and map
      projections.
- [x] Accept normal Python sequences, mappings, bytes, UUIDs, datetimes, and
      timedeltas where the WinRT signature permits them.
- [x] Emit `IntFlag` for flags enums.
- [x] Add overload-aware runtime dispatch and `typing.overload` stubs.
- [x] Generate idiomatic constructors such as `Uri(...)` while retaining an
      internal wrapper path for returned native objects.
- [x] Expose `project_as(value, Type)` for typed, non-consuming projection of
      raw `Object`/`IInspectable` values, matching JavaScript `projectAs`.
- [x] Declare non-suppressing runtime and generated context managers as
      `Literal[False]`.
- [x] Replace deprecated PyO3 automatic `FromPyObject` behavior explicitly.
- [x] Document COM apartment ownership and provide a balanced context manager.

## P1: tooling and documentation

- [ ] Add a Python configuration surface to winapp CLI or a dedicated Python
      orchestration command.
- [x] Generate package names and imports consistently across Windows SDK,
      Windows App SDK, and custom WinMD namespaces.
- [x] Add runnable samples for files, notifications, imaging, async, events,
      collections, and custom WinMD consumption.
- [ ] Document generated-code version compatibility with `dynwinrt`.
- [ ] Add troubleshooting for metadata, apartment, bootstrap, architecture,
      and wheel compatibility failures.
- [x] Add progress-callback tests for worker-thread delivery and operations
      that complete concurrently with callback registration.

## WinUI milestone

- [x] Project public composable `__init__` overloads without requiring the
      ABI-only `outer` parameter. The low-level factory method remains available
      for parity and still exposes its raw ABI shape.
- [x] Keep protected composable constructors out of generated Python
      `__init__` overloads.
- [x] Make WinMD activation/composition metadata the constructor boundary:
      system-returned and protected-composition classes retain native wrapping
      but reject public construction and expose no constructible `.pyi` overload.
- [x] Add a reusable outer/inner aggregation host for public composable Python
      subclasses, preserving controlling COM identity and inner ownership.
- [x] Add metadata-driven local overridable-interface vtables for public
      composable subclasses, including native-invoked FrameworkElement
      measure/arrange/template callbacks and fail-closed ABI validation.
- [x] Register arbitrary Python runtime-class names and compose them into the
      XAML metadata-provider chain so custom types can be instantiated by name
      from XAML markup.
- [ ] Make Windows App SDK initialization idempotent and version-aware.
- [ ] Auto-discover or explicitly provision the bootstrap DLL.
- [x] Expose the selected framework `resources.pri` path.
- [x] Add Python-specific implicit XAML metadata provider generation.
- [x] Generate the specialized `Application.create_with_metadata_provider()`
      and Fluent-resource bootstrap helpers with matching `.pyi` declarations.
- [x] Run `Application.start`, `Application`, `Window`, `Grid`, `Button`, and
      `TextBlock` on an STA in a live x64 WinUI E2E.
- [x] Validate worker-thread and `asyncio` scheduling while the WinUI
      DispatcherQueue owns the UI thread, including a WinRT async completion
      and `try_enqueue` back to the UI thread.
- [x] Add projected lifetime scopes that release UI/COM wrappers before
      apartment teardown.
- [x] Add awaitable DispatcherQueue result/error/priority scheduling helpers.
- [x] Project `IObservableVector<T>` as a mutable Python sequence with typed
      `VectorChanged` events.
- [x] Add a GIL-safe Python `IElementFactory` bridge and live ItemsRepeater
      get/recycle identity coverage.
- [x] Load and validate Fluent Light, Dark, and High Contrast resources.
- [x] Add a real Python WinUI application E2E on x64.
- [ ] Add Python WinUI application E2E coverage on ARM64.

## Later

- [x] Preserve projected object identity where it affects Python semantics.
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
9. [x] Ship typed `dynwinrt` wheels and validate the native extension stubs.
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
20. [x] Make reference returns null-safe without lying in generated annotations.
21. [x] Preserve `UInt32` / `UInt64` values across the Python boundary.
22. [x] Project and box `IReference<T>` struct fields, including struct-valued `T`.
22. [x] Add typed event arguments while preserving token subscriptions and adding
        idempotent unsubscribe helpers.
23. [x] Project public composable constructors and reject protected composition.
24. [x] Add an experimental, typed WinUI `Application` bootstrap path backed
        by a shared `codegen/winrt/extensions/winui` spec consumed by both
        JavaScript and Python projection.
25. [x] Run a real unpackaged x64 Python WinUI app with Fluent resources,
        Window/Grid/Button/TextBlock, automation-driven Click, and clean
        apartment shutdown.
26. [x] Release the GIL around blocking WinUI host calls and verify asyncio
        worker progress, WinRT async completion, and DispatcherQueue UI dispatch.
