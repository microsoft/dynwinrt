# dynwinrt-py


Develop

Run `uv run maturin develop` to build the package

Use `uv run pytest` to run the tests

After modification `uv sync --reinstall` may be needed to reinstall the package

The wheel includes `__init__.pyi` and `py.typed` for static type checking.

Generated `IReference<T>` values are projected as `T | None`; native values,
`None`, and generated `IReference_*` wrappers are accepted as inputs.

## Async WinRT operations

Generated async methods return typed, asyncio-compatible operation objects:

```python
operation = writer.store_async()
stored_bytes = await operation
```

Their public types are `WinRTAsync[T]` and
`WinRTAsyncWithProgress[T, P]`; the concrete runtime wrappers remain private.

Regenerated bindings no longer block inside async methods. Existing code that
expects an immediate result must use `await operation` or `operation.wait()`.

`asyncio` task cancellation calls `IAsyncInfo.Cancel()` on the underlying
WinRT operation. Operations with scalar progress values also expose
`operation.progress(callback)`. Fast operations can finish before registration;
in that case no future progress exists and registration is a no-op.

For scripts without an event loop, `operation.wait()` remains available as an
explicit blocking API. It rejects started operations when called from a running
asyncio loop or an STA thread, where blocking could freeze or deadlock the
caller.

WinRT HRESULT failures raise `OSError` (or a standard `OSError` subclass) with
the signed HRESULT in `error.winerror`. The exception message preserves
restricted WinRT error information when Windows provides it.

## Python-native values

Generated collection projections implement the standard `collections.abc`
protocols: vectors behave as sequences, maps as mappings, and WinRT iterables
and iterators work with `iter()` and `next()`. Mutable vectors and maps support
normal indexing, slicing, assignment, insertion, and deletion.

Method inputs accept normal Python sequences and mappings in place of compatible
WinRT collection interfaces. Byte arrays accept `bytes` and `bytearray`; GUID,
`DateTime`, and `TimeSpan` values use `uuid.UUID`, `datetime.datetime`, and
`datetime.timedelta`.

Exceptions raised by Python event or delegate callbacks are reported through
`sys.unraisablehook`. The originating WinRT invocation receives
`0xA0EE4005` (`PYWINRT_E_UNRAISABLE_PYTHON_EXCEPTION`) instead of unconditional
success. Generated delegate parameters accept normal Python callables. WinRT
chooses the callback thread, so callbacks must not assume they run on the
registration thread or an asyncio event-loop thread. Keep each token returned by
`on_*` and pass it to the matching `off_*` when the subscription is no longer
needed. For callback-style cleanup, `subscribe_*` returns an idempotent
unsubscribe function. `once_*` subscribes for at most one callback invocation.

WinRT flags enums are projected as `enum.IntFlag`. Overloaded methods share one
Python name with runtime type/arity dispatch and `typing.overload` declarations.
Activatable runtime classes use normal constructors, for example
`Uri("https://example.com")`; native return values use an internal wrapper path.

## COM apartments and cleanup

Use `RoApartment` to initialize COM for a thread and balance every successful
initialization:

```python
with RoApartment(0):  # RO_INIT_SINGLETHREADED
    use_winrt()
```

Use `RoApartment(1)` for `RO_INIT_MULTITHREADED`. Nested contexts using the same
model are supported. Requesting a conflicting model raises `OSError` with
`RPC_E_CHANGED_MODE`. The low-level `ro_initialize()` API remains available, but
each successful call, including `S_FALSE`, must be paired with one
`ro_uninitialize()` call on the same thread.

Generated runtime classes that implement `IClosable` support `with` and an
idempotent `close()` method. Prefer deterministic cleanup instead of relying on
Python garbage collection.

## Experimental WinUI bootstrap

When `Microsoft.UI.Xaml.Application` is generated with the WinUI metadata
provider and controls resources, codegen also emits
`Application.create_with_metadata_provider(...)` and `Application.create(...)`.
The latter installs `XamlControlsResources` and configures unpackaged resource
resolution when the required metadata is available.

This is a bootstrap slice, not general WinUI support. Arbitrary
composition/aggregation, custom controls, and ARM64 UI validation remain tracked in
[`PYTHON_CHECKLIST.md`](../../PYTHON_CHECKLIST.md).

Generated `Application.start()` and `DispatcherQueue.run_event_loop()` calls
remain on the caller's native thread but release the Python GIL while WinUI
pumps messages. WinRT callbacks reacquire the GIL, and worker-thread asyncio
code can use `DispatcherQueue.try_enqueue()` to return to the UI thread.

The live x64 smoke test accepts explicit WinAppSDK metadata and bootstrap
inputs:

```powershell
.\tests\python_winui_e2e.ps1 `
  -WinuiWinmd <Microsoft.UI.Xaml.winmd> `
  -RefList <winmd-reference-list.txt> `
  -BootstrapDll <Microsoft.WindowsAppRuntime.Bootstrap.dll> `
  -Major 2 -Minor 3
```