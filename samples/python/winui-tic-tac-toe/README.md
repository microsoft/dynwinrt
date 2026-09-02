# Python WinUI Tic-Tac-Toe

A polished Mica/Fluent 3x3 Tic-Tac-Toe window built from generated WinUI
projections. It demonstrates unpackaged WinAppSDK startup, XAML loading, events,
window resizing, a named Python `TicTacToePanel` XAML registration, a native
`measure_override`, and deterministic registration cleanup.

## Prerequisites

- Windows 11 on x64 with CPython 3.11–3.14.
- An unpackaged **x64** WinAppSDK 2.3 fixture: `Microsoft.UI.Xaml.winmd`, a
  newline-separated reference-WinMD list, and the matching x64
  `Microsoft.WindowsAppRuntime.Bootstrap.dll`. The matching x64 WinAppSDK
  framework/runtime packages and resources must also be installed or available
  to the unpackaged runtime.
- `dynwinrt` installed in the Python interpreter used to run the sample.
- Matching `dynwinrt` and `dynwinrt-codegen` versions.

Install the published preview packages:

```powershell
python -m pip install --pre dynwinrt dynwinrt-codegen
```

For source-checkout development, build `dynwinrt-codegen` and install the
runtime with `python -m maturin develop --release` from `bindings\py` instead.

From this sample directory, generate the local package and copy the bootstrap
DLL (replace the fixture paths):

```powershell
.\generate.ps1 `
  -WinuiWinmd C:\fixtures\winappsdk\metadata\Microsoft.UI.Xaml.winmd `
  -RefList C:\fixtures\winappsdk\winmd-reference-list.txt `
  -BootstrapDll C:\fixtures\winappsdk\x64\Microsoft.WindowsAppRuntime.Bootstrap.dll
```

Pass `-Codegen ..\..\..\target\release\dynwinrt-codegen.exe` when testing a
source build. Run with the same Python environment that contains `dynwinrt`:

```powershell
.\run.ps1 -Python C:\path\to\python.exe
```

`generated\`, `.runtime\`, and Python caches are local build artifacts and are
not tracked.
