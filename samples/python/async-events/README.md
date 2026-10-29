# Async notification scope

Requires Windows 10/11, CPython 3.11-3.14, a Windows SDK, and a `dynwinrt`
runtime build containing `AsyncEventScope`. No WinUI, package identity,
network, or Windows App SDK is needed.

From this directory, with `dynwinrt-codegen` on PATH and the matching runtime
installed in your Python environment:

```powershell
dynwinrt-codegen generate --class-name "Windows.Foundation.Collections.PropertySet,Windows.Storage.StorageFolder,Windows.Storage.FileIO" --lang py --output .\generated
python .\app.py
python .\app.py --fail
```

The application owns the asyncio loop. Inside its COM apartment and projection
scope, it registers an async `PropertySet.MapChanged` handler, mutates the map
on that same thread, and unsubscribes immediately. The accepted handler still
finishes its native asynchronous temporary-file I/O before the event scope
exits; later notifications are ignored. `--fail` demonstrates a handler error
reaching the enclosing `except*` only after task cleanup.

This is a concurrent TaskGroup adapter for notifications that allow deferred
work, not a WinUI pump, cross-thread COM bridge, or deferral implementation.
Keep native lifetimes outside the event scope. A custom eager task factory can
start a handler during the native callback; no completion ordering or
backpressure policy is provided. See the [runtime contract](../../../bindings/py/README.md#async-notification-handlers).
