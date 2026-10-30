# Python device watcher

This sample enumerates devices with `DeviceInformation.create_watcher()` and
demonstrates typed `Added`, `Updated`, `Removed`, `EnumerationCompleted`, and
`Stopped` event subscriptions.

Callbacks read WinRT objects on the callback apartment, then safely schedule
only Python-native values back onto the `asyncio` loop. The watcher is stopped
and every subscription is removed before the apartment exits.

```powershell
.\generate.ps1 -Codegen ..\..\..\target\release\dynwinrt-codegen.exe
.\run.ps1 -Python C:\path\to\python.exe
```

The exact devices vary by machine. A successful run prints
`python-device-watcher-ok` and counts by device kind. Pass `-ShowNames` to
`run.ps1` to also print up to ten discovered device names.

After generation, install the generated project into the same interpreter used
to run the sample: `python -m pip install -e .\generated` (use that interpreter's
full path when needed). The import remains `generated`; sources now live in
`generated\src\generated`, not directly in `generated`. When upgrading from
preview.23 or earlier, regenerate the complete output and reinstall it.
