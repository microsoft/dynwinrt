# Python device watcher

This sample enumerates devices with `DeviceInformation.create_watcher()` and
demonstrates typed `Added`, `Updated`, `Removed`, `EnumerationCompleted`, and
`Stopped` event subscriptions.

Callbacks safely schedule their work back onto the `asyncio` loop. The watcher
is stopped and every subscription is removed before the apartment exits.

```powershell
.\generate.ps1 -Codegen ..\..\..\target\release\dynwinrt-codegen.exe
.\run.ps1 -Python C:\path\to\python.exe
```

The exact devices vary by machine. A successful run prints
`python-device-watcher-ok` and counts by device kind. Pass `-ShowNames` to
`run.ps1` to also print up to ten discovered device names.
