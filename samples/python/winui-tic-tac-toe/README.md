# Python WinUI Tic-Tac-Toe

A polished Mica/Fluent 3x3 Tic-Tac-Toe window built from generated WinUI
projections. It demonstrates unpackaged WinAppSDK startup, typed projection of
raw XAML objects, generated composable `MicaBackdrop` construction, events,
window resizing, a named Python `TicTacToePanel` XAML registration, a native
`measure_override`, and deterministic registration cleanup.

![Tic-Tac-Toe running with the Mica system backdrop](../assets/winui-tic-tac-toe.png)

## Prerequisites

- Windows 11 on x64 with CPython 3.11–3.14.
- WinApp CLI 0.6.2 or newer.
- `dynwinrt` installed in the Python interpreter used to run the sample.
- Matching `dynwinrt` and `dynwinrt-codegen` versions.

Install the published preview packages:

```powershell
python -m pip install --pre dynwinrt dynwinrt-codegen
```

For source-checkout development, build `dynwinrt-codegen` and install the
runtime with `python -m maturin develop --release` from `bindings\py` instead.

Restore the pinned Windows App SDK, generate the Python projection, and run:

```powershell
winapp restore
.\generate.ps1
.\run.ps1 -Python C:\path\to\python.exe
```

`winapp restore` prepares the pinned SDK metadata and bootstrap binaries under
`.winapp\`. `generate.ps1` uses that metadata to generate Python bindings and
copies the selected architecture's bootstrap DLL to `.runtime\`. Before
calling `init_winappsdk(2, 3)`, `app.py` sets
`WINAPPSDK_BOOTSTRAP_DLL_PATH` to that local copy.

Pass `-Codegen ..\..\..\target\release\dynwinrt-codegen.exe` when testing a
source build.

`generated\`, `.runtime\`, and Python caches are local build artifacts and are
not tracked.
