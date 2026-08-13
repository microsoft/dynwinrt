# Python WinUI Tic-Tac-Toe

A polished Mica/Fluent 3x3 Tic-Tac-Toe window built from generated WinUI
projections. It demonstrates unpackaged WinAppSDK startup, XAML loading, events,
window resizing, a named Python `TicTacToePanel` XAML registration, a native
`measure_override`, and deterministic registration cleanup.

## Prerequisites

- Windows 11 on x64 with Python and Rust/Cargo available.
- An unpackaged **x64** WinAppSDK 2.3 fixture: `Microsoft.UI.Xaml.winmd`, a
  newline-separated reference-WinMD list, and the matching x64
  `Microsoft.WindowsAppRuntime.Bootstrap.dll`. The matching x64 WinAppSDK
  framework/runtime packages and resources must also be installed or available
  to the unpackaged runtime.
- `dynwinrt` installed in the Python interpreter used to run the sample.
- A compatible `dynwinrt-codegen` executable, either on `PATH` or passed with
  `-Codegen`.

No published package is assumed. From the repository root, build the codegen
executable and install the runtime into the active Python environment:

```powershell
cargo build -p dynwinrt-codegen --release
Push-Location bindings\py
python -m pip install maturin
python -m maturin develop --release
Pop-Location
```

From this sample directory, generate the local package and copy the bootstrap
DLL (replace the fixture paths):

```powershell
.\generate.ps1 `
  -WinuiWinmd C:\fixtures\winappsdk\metadata\Microsoft.UI.Xaml.winmd `
  -RefList C:\fixtures\winappsdk\winmd-reference-list.txt `
  -BootstrapDll C:\fixtures\winappsdk\x64\Microsoft.WindowsAppRuntime.Bootstrap.dll `
  -Codegen ..\..\..\target\release\dynwinrt-codegen.exe
```

If the CLI is installed on `PATH`, omit `-Codegen`. Run with the same Python
environment that contains `dynwinrt`:

```powershell
.\run.ps1 -Python C:\path\to\python.exe
```

`generated\`, `.runtime\`, and Python caches are local build artifacts and are
not tracked.
