# Python device watcher

This sample enumerates devices with `DeviceInformation.create_watcher()` and
demonstrates typed `Added`, `Updated`, `Removed`, `EnumerationCompleted`, and
`Stopped` async event iterators.

Each iterator uses a bounded queue and safely transfers callbacks onto the
owning `asyncio` loop. Its `async with` scope removes the subscription on normal
exit, cancellation, or failure before the apartment exits.

```powershell
.\generate.ps1 -Codegen ..\..\..\target\release\dynwinrt-codegen.exe
.\run.ps1 -Python C:\path\to\python.exe
```

The exact devices vary by machine. A successful run prints
`python-device-watcher-ok` and counts by device kind. Pass `-ShowNames` to
`run.ps1` to also print up to ten discovered device names.
