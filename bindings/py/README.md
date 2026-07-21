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
`operation.progress(callback)`.

For scripts without an event loop, `operation.wait()` remains available as an
explicit blocking API. It rejects started operations when called from a running
asyncio loop or an STA thread, where blocking could freeze or deadlock the
caller.