# Python AppLifecycle single instance

This Windows App SDK sample launches two Python processes and demonstrates:

- registering a primary instance with `AppInstance`;
- retrieving the second process's activation arguments;
- redirecting activation asynchronously to the primary process; and
- receiving and unsubscribing from the typed `Activated` event.

## Prerequisites

- `Microsoft.Windows.AppLifecycle.winmd`;
- a newline-separated reference-WinMD list for the matching Windows App SDK;
- the matching architecture's bootstrap DLL;
- `dynwinrt` installed in the selected Python interpreter; and
- `dynwinrt-codegen` on `PATH`, or passed with `-Codegen`.

## Run

```powershell
.\generate.ps1 `
  -AppLifecycleWinmd C:\fixtures\Microsoft.Windows.AppLifecycle.winmd `
  -RefList C:\fixtures\winmd-reference-list.txt `
  -BootstrapDll C:\fixtures\x64\Microsoft.WindowsAppRuntime.Bootstrap.dll `
  -Codegen ..\..\..\target\release\dynwinrt-codegen.exe

.\run.ps1 -Python C:\path\to\python.exe
```

The loopback succeeds only after the secondary process redirects its launch
activation and the primary receives it. It prints `python-app-lifecycle-ok`.
