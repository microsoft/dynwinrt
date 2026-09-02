# Python WinUI Tic-Tac-Toe (code-only)

A polished, playable 3x3 Tic-Tac-Toe sample built entirely in Python with
generated WinUI 3 projections. `Application`, `Window`, every layout object,
grid definition, control, event handler, and game-state transition is created
programmatically. The sample uses the default Fluent control resources and a
Mica system backdrop; it does not load or register XAML.

`MicaBackdrop` currently reaches an ambiguous composition-metadata closure when
generated directly. The sample therefore activates that runtime class through
its low-level `IMicaBackdropFactory.CreateInstance` ABI and safely wraps the
returned object as the projected `SystemBackdrop` expected by
`Window.system_backdrop`.

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

Generate the local projection package and copy the bootstrap DLL:

```powershell
.\generate.ps1 `
  -WinuiWinmd C:\fixtures\winappsdk\metadata\Microsoft.UI.Xaml.winmd `
  -RefList C:\fixtures\winappsdk\winmd-reference-list.txt `
  -BootstrapDll C:\fixtures\winappsdk\x64\Microsoft.WindowsAppRuntime.Bootstrap.dll
```

Pass `-Codegen ..\..\..\target\release\dynwinrt-codegen.exe` when testing a
source build. Run with the Python environment that contains `dynwinrt`:

```powershell
.\run.ps1 -Python C:\path\to\python.exe
```

Players alternate selecting cells. Choose **New game** after a win or draw to
reset the board. Closing the window exits the WinUI application and releases
projected objects deterministically.

`generated\`, `.runtime\`, and Python caches are local artifacts and are not
tracked.
