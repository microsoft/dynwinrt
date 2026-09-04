# Python AppLifecycle single instance

This Windows App SDK sample launches two Python processes and demonstrates:

- registering a primary instance with `AppInstance`;
- retrieving the second process's activation arguments;
- redirecting activation asynchronously to the primary process; and
- receiving and unsubscribing from the typed `Activated` event.

## Prerequisites

- WinApp CLI 0.6.2 or newer;
- `dynwinrt` installed in the selected Python interpreter; and
- `dynwinrt-codegen` on `PATH`, or passed with `-Codegen`.

## Run

```powershell
winapp restore
.\generate.ps1 -Codegen ..\..\..\target\release\dynwinrt-codegen.exe

.\run.ps1 -Python C:\path\to\python.exe -Major 2 -Minor 3
```

`winapp restore` prepares the pinned SDK metadata and bootstrap binaries under
`.winapp\`. `generate.ps1` uses that metadata to generate Python bindings and
copies the selected architecture's bootstrap DLL to `.runtime\`.

`Major` and `Minor` default to `2` and `3`. They must exactly match the Windows
App SDK product version represented by the metadata, bootstrap DLL, and
installed runtime.

The loopback succeeds only after the secondary process redirects its launch
activation and the primary receives it. It prints `python-app-lifecycle-ok`.
